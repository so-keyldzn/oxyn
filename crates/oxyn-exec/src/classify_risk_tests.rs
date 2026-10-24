//! A session tells how destructive a non-SQL request is (ADR-0066).
//!
//! For SQL, `oxyn-query` flags `DELETE FROM users` as an unbounded delete, and
//! the gate asks a human for an approval even on staging. MongoDB's
//! `{"delete":"users","deletes":[{"q":{},"limit":0}]}` empties the same
//! collection: without the session's answer it is a plain write, and runs on
//! staging with no approval. These tests prove the answer takes the path the
//! SQL risk takes — approval for a human, refusal for an agent on production,
//! never an execution an agent proposed — and that it is asked for nowhere
//! else: not for SQL, not for a read, not for a request whose intent nobody
//! proved.

use std::sync::atomic::Ordering;

use super::classify_intent_tests::{Bench, Fake, actors, agent, bench_on, execution};
use super::*;
use oxyn_core::{MutationRisk, QueryLanguage, SqlDialect, StatementIntent};

/// MongoDB's command form of a delete whose filter matches every document.
const DELETE_ALL: &str = r#"{"delete":"users","deletes":[{"q":{},"limit":0}]}"#;

fn proven_write_with(risk: Option<MutationRisk>) -> Fake {
    let mut fake = Fake::answering(Some(StatementIntent::Write));
    fake.risk = risk;
    fake
}

/// The intent and risk the journal recorded for the last decision.
fn journaled(bench: &Bench) -> (StatementIntent, MutationRisk) {
    let record = bench
        .executor
        .store
        .journal()
        .recent(1)
        .expect("reading the journal")
        .pop()
        .expect("one entry")
        .record;
    (record.intent, record.risk)
}

async fn dispatched(bench: &Bench, actor: Actor, language: QueryLanguage, text: &str) -> Outcome {
    bench
        .executor
        .dispatch(actor, execution(bench, language, text), &CancelToken::new())
        .await
        .expect("an approval request or a denial is not a failure")
}

#[tokio::test]
async fn a_non_sql_unbounded_delete_asks_the_approval_sql_asks_on_staging() {
    let bench = bench_on(
        proven_write_with(Some(MutationRisk::UnboundedDelete)),
        Environment::Staging,
    );

    let mongo = dispatched(&bench, Actor::Human, QueryLanguage::MongoQuery, DELETE_ALL).await;
    let Outcome::NeedsApproval {
        command, reason, ..
    } = mongo
    else {
        panic!("a human approves a collection-wide delete, even on staging: {mongo:?}");
    };
    assert_eq!(
        journaled(&bench),
        (StatementIntent::Write, MutationRisk::UnboundedDelete)
    );

    let sql = dispatched(
        &bench,
        Actor::Human,
        QueryLanguage::Sql(SqlDialect::Postgres),
        "DELETE FROM users",
    )
    .await;
    let Outcome::NeedsApproval {
        reason: sql_reason, ..
    } = sql
    else {
        panic!("the SQL delete asks for an approval too: {sql:?}");
    };
    assert_eq!(reason, sql_reason, "both deletes give the same reason");
    assert_eq!(
        journaled(&bench),
        (StatementIntent::Write, MutationRisk::UnboundedDelete)
    );
    assert!(
        bench.fake.executed.lock().is_empty(),
        "nothing runs before the approval"
    );

    // The approval releases the request exactly as SQL's would: with the
    // risk it was approved under.
    let _ = bench
        .executor
        .approve("tester", command, &CancelToken::new())
        .await;
    let executed = bench.fake.executed.lock();
    let request = executed.first().expect("the approved delete ran");
    assert_eq!(request.language, QueryLanguage::MongoQuery);
    assert_eq!(request.intent, StatementIntent::Write);
    assert_eq!(request.risk, MutationRisk::UnboundedDelete);
    assert_eq!(
        bench.fake.risk_consulted.load(Ordering::SeqCst),
        1,
        "the SQL delete never asked the session"
    );
}

#[tokio::test]
async fn an_agent_risky_write_is_refused_on_production_and_never_runs_elsewhere() {
    let production = bench_on(
        proven_write_with(Some(MutationRisk::DropObject)),
        Environment::Production,
    );
    let outcome = dispatched(
        &production,
        agent(),
        QueryLanguage::MongoQuery,
        r#"{"drop":"users"}"#,
    )
    .await;
    assert!(
        outcome.is_denied(),
        "an agent is refused a write on production: {outcome:?}"
    );
    let human = dispatched(
        &production,
        Actor::Human,
        QueryLanguage::MongoQuery,
        r#"{"drop":"users"}"#,
    )
    .await;
    assert!(human.needs_approval(), "{human:?}");
    assert_eq!(
        journaled(&production),
        (StatementIntent::Write, MutationRisk::DropObject)
    );
    assert!(production.fake.executed.lock().is_empty());

    let staging = bench_on(
        proven_write_with(Some(MutationRisk::DropObject)),
        Environment::Staging,
    );
    let outcome = dispatched(
        &staging,
        agent(),
        QueryLanguage::MongoQuery,
        r#"{"drop":"users"}"#,
    )
    .await;
    let Outcome::NeedsApproval { reason, .. } = outcome else {
        panic!("an agent's drop waits for a human: {outcome:?}");
    };
    assert_eq!(Some(reason.as_str()), MutationRisk::DropObject.reason());
    assert!(
        staging.fake.executed.lock().is_empty(),
        "what an agent proposes is never executed on its own word"
    );
}

#[tokio::test]
async fn a_proven_write_without_a_risk_runs_on_staging_as_before() {
    let bench = bench_on(proven_write_with(None), Environment::Staging);
    let outcome = bench
        .executor
        .dispatch(
            Actor::Human,
            execution(
                &bench,
                QueryLanguage::MongoQuery,
                r#"{"delete":"users","deletes":[{"q":{"_id":1},"limit":1}]}"#,
            ),
            &CancelToken::new(),
        )
        .await;

    assert!(
        !matches!(
            outcome,
            Ok(Outcome::NeedsApproval { .. } | Outcome::Denied { .. })
        ),
        "a bounded write on staging needs no approval: {outcome:?}"
    );
    let executed = bench.fake.executed.lock();
    let request = executed.first().expect("the write reached the session");
    assert_eq!(request.risk, MutationRisk::None);
    assert_eq!(bench.fake.risk_consulted.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_read_or_an_unproven_intent_is_never_given_a_risk() {
    for answer in [
        Some(StatementIntent::Read),
        Some(StatementIntent::Unknown),
        None,
    ] {
        let mut fake = Fake::answering(answer);
        fake.risk = Some(MutationRisk::DropObject);
        let bench = bench_on(fake, Environment::Staging);
        for actor in actors() {
            let _ = bench
                .executor
                .dispatch(
                    actor,
                    execution(&bench, QueryLanguage::RedisCommand, "GET user:1"),
                    &CancelToken::new(),
                )
                .await;
            assert_eq!(
                journaled(&bench).1,
                MutationRisk::None,
                "{answer:?} carries no risk"
            );
        }
        assert_eq!(
            bench.fake.risk_consulted.load(Ordering::SeqCst),
            0,
            "{answer:?} never asks for a risk"
        );
    }
}

#[tokio::test]
async fn a_sql_request_never_asks_the_session_for_its_risk() {
    // The fake would call any write a drop: only `oxyn-query` reads SQL.
    let bench = bench_on(
        proven_write_with(Some(MutationRisk::DropObject)),
        Environment::Staging,
    );
    let outcome = bench
        .executor
        .dispatch(
            Actor::Human,
            execution(
                &bench,
                QueryLanguage::Sql(SqlDialect::Postgres),
                "DELETE FROM users WHERE id = 1",
            ),
            &CancelToken::new(),
        )
        .await;

    assert!(
        !matches!(
            outcome,
            Ok(Outcome::NeedsApproval { .. } | Outcome::Denied { .. })
        ),
        "a bounded SQL delete on staging needs no approval: {outcome:?}"
    );
    assert_eq!(
        journaled(&bench),
        (StatementIntent::Write, MutationRisk::None)
    );
    assert_eq!(bench.fake.risk_consulted.load(Ordering::SeqCst), 0);
    assert_eq!(bench.fake.consulted.load(Ordering::SeqCst), 0);
}
