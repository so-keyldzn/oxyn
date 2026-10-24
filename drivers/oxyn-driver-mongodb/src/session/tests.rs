//! What the session tells the gate before anything is sent: the intent and
//! the risk of a console command, from the same reading.

use mongodb::options::{ClientOptions, ServerAddress};
use oxyn_core::SqlDialect;

use super::*;

/// A session whose client never connects: classifying does no I/O, and the
/// address listens to nothing.
fn session() -> MongoSession {
    let options = ClientOptions::builder()
        .hosts(vec![ServerAddress::Tcp {
            host: "127.0.0.1".to_owned(),
            port: Some(1),
        }])
        .build();
    let client = Client::with_options(options.clone()).expect("options without I/O");
    let stopper = Client::with_options(options).expect("options without I/O");
    MongoSession::new(
        Arc::new(Shared {
            client,
            stopper,
            quarantine: Arc::default(),
            driver: DriverId::mongodb(),
            default_database: None,
            capabilities: Capabilities::empty(),
        }),
        "8.0.34".to_owned(),
    )
}

fn request(text: &str) -> ExecRequest {
    ExecRequest::new(LANGUAGE, text)
}

#[tokio::test]
async fn the_destructive_forms_carry_their_risk_beside_a_proven_write() {
    let session = session();
    for (text, intent, risk) in [
        (
            r#"{"delete": "u", "deletes": [{"q": {}, "limit": 0}]}"#,
            StatementIntent::Write,
            MutationRisk::UnboundedDelete,
        ),
        (
            r#"{"update": "u", "updates": [{"q": {}, "u": {"$set": {"a": 1}}, "multi": true}]}"#,
            StatementIntent::Write,
            MutationRisk::UnboundedUpdate,
        ),
        (
            r#"{"bulkWrite": 1, "ops": [{"delete": 0, "filter": {}, "multi": true}], "nsInfo": []}"#,
            StatementIntent::Write,
            MutationRisk::UnboundedDelete,
        ),
        (
            r#"{"drop": "u"}"#,
            StatementIntent::Ddl,
            MutationRisk::DropObject,
        ),
        (
            r#"{"dropDatabase": 1}"#,
            StatementIntent::Ddl,
            MutationRisk::DropObject,
        ),
        (
            r#"{"dropIndexes": "u", "index": "*"}"#,
            StatementIntent::Ddl,
            MutationRisk::DropObject,
        ),
        (
            r#"{"dropSearchIndex": "u", "name": "default"}"#,
            StatementIntent::Ddl,
            MutationRisk::DropObject,
        ),
    ] {
        let request = request(text);
        assert_eq!(session.classify_intent(&request), Some(intent), "{text}");
        assert_eq!(session.classify_risk(&request), Some(risk), "{text}");
    }
}

#[tokio::test]
async fn a_bounded_write_carries_no_risk() {
    let session = session();
    for text in [
        r#"{"delete": "u", "deletes": [{"q": {"_id": 1}, "limit": 0}]}"#,
        r#"{"delete": "u", "deletes": [{"q": {}, "limit": 1}]}"#,
        r#"{"update": "u", "updates": [{"q": {}, "u": {"$set": {"a": 1}}}]}"#,
        r#"{"insert": "u", "documents": [{"a": 1}]}"#,
        r#"{"createIndexes": "u", "indexes": [{"key": {"a": 1}, "name": "a"}]}"#,
        r#"{"dropUser": "reporting"}"#,
    ] {
        let request = request(text);
        assert!(
            matches!(
                session.classify_intent(&request),
                Some(StatementIntent::Write | StatementIntent::Ddl | StatementIntent::Grant)
            ),
            "{text}"
        );
        assert_eq!(session.classify_risk(&request), None, "{text}");
    }
}

#[tokio::test]
async fn what_proves_no_write_answers_no_risk() {
    let session = session();
    // A read, a transaction verb the session refuses, a command the table
    // does not know, server-side JavaScript in a delete of everything: the
    // intent is a read or nothing, and the risk follows it.
    for (text, intent) in [
        (
            r#"{"find": "u", "filter": {}}"#,
            Some(StatementIntent::Read),
        ),
        (r#"{"commitTransaction": 1}"#, None),
        (r#"{"shutdown": 1}"#, None),
        (
            r#"{"delete": "u", "deletes": [{"q": {"$where": "true"}, "limit": 0}]}"#,
            None,
        ),
    ] {
        let request = request(text);
        assert_eq!(session.classify_intent(&request), intent, "{text}");
        assert_eq!(session.classify_risk(&request), None, "{text}");
    }
    // Another language is not this session's to read.
    let sql = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), "DROP TABLE u");
    assert_eq!(session.classify_risk(&sql), None);
}
