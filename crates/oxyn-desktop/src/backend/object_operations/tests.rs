//! The review of ADR-0042: the composer on its own, then the whole trip on a
//! SQLite connection whose host dialog is scripted.

use std::sync::Arc;
use std::time::Duration;

use oxyn_catalog::CatalogPath;
use oxyn_catalog::model::RelationKind;
use oxyn_core::{
    Actor, CancelToken, Capabilities, Command, CommandId, ConnectionConfig, ConnectionId, DriverId,
    Environment, SessionId, SqlDialect,
};
use oxyn_exec::Outcome;

use super::{HOSTILE_NAME, NO_DDL, NO_TRUNCATE, check_statement, compose, unavailable};
use crate::backend::Backend;
use crate::backend::confirm::{Answer, ScriptedConfirm, Timing};
use crate::ipc::object_operations::{
    Dependents, ObjectOperation, ObjectOperationOutcome, OperationKind,
};
use crate::ipc::{CatalogAddress, CatalogNode, CommandOutcome};

const HOSTILE: &str = r#"users"; DROP TABLE audit; --"#;

fn table(namespace: &str, name: &str) -> CatalogPath {
    CatalogPath::for_relation(None, Some(namespace), name).expect("a legal path")
}

fn drop(cascade: bool) -> ObjectOperation {
    ObjectOperation::Drop { cascade }
}

fn rename(column: Option<&str>, new_name: &str) -> ObjectOperation {
    ObjectOperation::Rename {
        column: column.map(str::to_owned),
        new_name: new_name.to_owned(),
    }
}

// ── The composer ────────────────────────────────────────────────────────────

#[test]
fn every_identifier_is_quoted_for_the_dialect_and_a_hostile_name_stays_one_name() {
    let path = table("public", HOSTILE);
    let sql = compose(
        &path,
        RelationKind::Table,
        &drop(false),
        SqlDialect::Postgres,
        true,
    )
    .expect("composed");
    assert_eq!(
        sql,
        r#"DROP TABLE "public"."users""; DROP TABLE audit; --""#
    );
    assert_eq!(
        oxyn_query::split(&sql, SqlDialect::Postgres).len(),
        1,
        "the hostile name did not end the statement"
    );

    let renamed = compose(
        &table("public", "Orders"),
        RelationKind::Table,
        &rename(Some("Total"), HOSTILE),
        SqlDialect::Postgres,
        true,
    )
    .expect("composed");
    assert_eq!(
        renamed,
        r#"ALTER TABLE "public"."Orders" RENAME COLUMN "Total" TO "users""; DROP TABLE audit; --""#
    );
    assert_eq!(oxyn_query::split(&renamed, SqlDialect::Postgres).len(), 1);
}

#[test]
fn each_operation_composes_one_statement_without_if_exists_or_comment() {
    let path = table("public", "orders");
    let cases = [
        (
            RelationKind::Table,
            drop(false),
            r#"DROP TABLE "public"."orders""#,
        ),
        (
            RelationKind::View,
            drop(false),
            r#"DROP VIEW "public"."orders""#,
        ),
        (
            RelationKind::MaterializedView,
            drop(false),
            r#"DROP MATERIALIZED VIEW "public"."orders""#,
        ),
        (
            RelationKind::Table,
            ObjectOperation::Truncate { cascade: false },
            r#"TRUNCATE TABLE "public"."orders""#,
        ),
        (
            RelationKind::Table,
            rename(None, "Orders"),
            r#"ALTER TABLE "public"."orders" RENAME TO "Orders""#,
        ),
    ];
    for (kind, operation, expected) in cases {
        let sql = compose(&path, kind, &operation, SqlDialect::Postgres, true).expect("composed");
        assert_eq!(sql, expected);
        assert!(!sql.contains("IF EXISTS") && !sql.contains("--"), "{sql}");
    }
    // SQLite names an attached database, not a schema of the session's
    // catalog: the path's level is kept, quoted.
    let sqlite = CatalogPath::for_relation(Some("main"), None, "t").expect("path");
    assert_eq!(
        compose(
            &sqlite,
            RelationKind::Table,
            &drop(false),
            SqlDialect::Sqlite,
            false
        )
        .expect("composed"),
        r#"DROP TABLE "main"."t""#
    );
}

#[test]
fn cascade_is_composed_only_where_the_engine_restricts_dependents() {
    let path = table("public", "orders");
    assert_eq!(
        compose(
            &path,
            RelationKind::Table,
            &drop(true),
            SqlDialect::Postgres,
            true
        )
        .expect("composed"),
        r#"DROP TABLE "public"."orders" CASCADE"#
    );
    assert_eq!(
        compose(
            &path,
            RelationKind::Table,
            &ObjectOperation::Truncate { cascade: true },
            SqlDialect::Postgres,
            true
        )
        .expect("composed"),
        r#"TRUNCATE TABLE "public"."orders" CASCADE"#
    );
    assert!(
        compose(
            &path,
            RelationKind::Table,
            &drop(true),
            SqlDialect::Sqlite,
            false
        )
        .is_err(),
        "no CASCADE where the engine would not restrict anything"
    );
}

#[test]
fn a_name_holding_control_characters_is_never_composed() {
    for hostile in [
        "line\nbreak",
        "carriage\rreturn",
        "tab\there",
        "separator\u{2028}here",
        "paragraph\u{2029}here",
        "override\u{202E}here",
        "isolate\u{2066}here",
        "nul\0here",
    ] {
        let mut cases = vec![
            (table("public", "orders"), rename(None, hostile)),
            (table("public", "orders"), rename(Some(hostile), "total")),
        ];
        // `CatalogPath` already refuses a control character in a level; the
        // separators and direction controls get through it, and stop here.
        if let (Ok(relation), Ok(namespace)) = (
            CatalogPath::for_relation(None, Some("public"), hostile),
            CatalogPath::for_relation(None, Some(hostile), "orders"),
        ) {
            assert!(!hostile.chars().any(char::is_control), "{hostile:?}");
            cases.push((relation, drop(false)));
            cases.push((namespace, drop(false)));
        }
        for (path, operation) in cases {
            assert_eq!(
                compose(
                    &path,
                    RelationKind::Table,
                    &operation,
                    SqlDialect::Postgres,
                    true
                ),
                Err(HOSTILE_NAME.to_owned()),
                "{hostile:?}"
            );
        }
    }
}

#[test]
fn a_rename_needs_a_new_name_that_differs() {
    let path = table("public", "orders");
    for new_name in ["", "orders"] {
        assert!(
            compose(
                &path,
                RelationKind::Table,
                &rename(None, new_name),
                SqlDialect::Postgres,
                true
            )
            .is_err()
        );
    }
}

#[test]
fn availability_follows_declared_capabilities_never_the_product() {
    let postgres = Capabilities::DDL
        | Capabilities::TRUNCATE
        | Capabilities::TRANSACTIONAL_DDL
        | Capabilities::RESTRICT_DEPENDENTS;
    let sqlite = Capabilities::DDL | Capabilities::TRANSACTIONAL_DDL;
    let read_only = Capabilities::TRANSACTIONAL_DDL;
    for operation in [
        OperationKind::Drop,
        OperationKind::Truncate,
        OperationKind::Rename,
    ] {
        assert_eq!(unavailable(operation, RelationKind::Table, postgres), None);
        assert_eq!(
            unavailable(operation, RelationKind::Table, read_only),
            Some(NO_DDL)
        );
    }
    assert_eq!(
        unavailable(OperationKind::Drop, RelationKind::Table, sqlite),
        None
    );
    assert_eq!(
        unavailable(OperationKind::Rename, RelationKind::Table, sqlite),
        None
    );
    assert_eq!(
        unavailable(OperationKind::Truncate, RelationKind::Table, sqlite),
        Some(NO_TRUNCATE)
    );
    assert_eq!(
        unavailable(OperationKind::Drop, RelationKind::View, postgres),
        None
    );
    assert!(unavailable(OperationKind::Truncate, RelationKind::View, postgres).is_some());
    assert!(unavailable(OperationKind::Rename, RelationKind::View, postgres).is_some());
    assert!(unavailable(OperationKind::Drop, RelationKind::Function, postgres).is_some());
}

#[test]
fn a_submission_must_be_one_statement_of_the_announced_kind() {
    let pg = SqlDialect::Postgres;
    assert!(check_statement(r#"DROP TABLE "t""#, pg, OperationKind::Drop).is_ok());
    assert!(check_statement(r#"TRUNCATE TABLE "t""#, pg, OperationKind::Truncate).is_ok());
    assert!(
        check_statement(
            r#"ALTER TABLE "t" RENAME TO "u""#,
            pg,
            OperationKind::Rename
        )
        .is_ok()
    );

    assert!(check_statement(r#"DROP TABLE "t""#, pg, OperationKind::Rename).is_err());
    assert!(check_statement(r#"TRUNCATE TABLE "t""#, pg, OperationKind::Drop).is_err());
    assert!(check_statement(r#"DELETE FROM "t""#, pg, OperationKind::Truncate).is_err());
    assert!(
        check_statement(
            r#"ALTER TABLE "t" RENAME TO "u"; DROP TABLE audit"#,
            pg,
            OperationKind::Rename
        )
        .is_err()
    );
    assert!(check_statement("", pg, OperationKind::Drop).is_err());
}

// ── The whole trip, on SQLite ───────────────────────────────────────────────

const NAME: &str = "Ledger";

/// Answers at once, and waits five minutes for an answer.
const PROMPT: Timing = Timing {
    min_delay: Duration::ZERO,
    ..Timing::HOST
};

struct Fixture {
    runtime: tokio::runtime::Runtime,
    backend: Backend,
    host: Arc<ScriptedConfirm>,
    connection: ConnectionId,
    session: SessionId,
}

impl Fixture {
    /// A SQLite connection marked `environment`, holding a table `t` of two
    /// rows and a view over it, with its catalog read. Nothing the setup
    /// needs approved goes through the host.
    fn new(environment: Environment) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let host = ScriptedConfirm::new(Answer::Refuse);
        let backend = {
            let _guard = runtime.enter();
            Backend::open_scripted(Arc::clone(&host), PROMPT).expect("temporary backend")
        };
        let config = ConnectionConfig::new(NAME, DriverId::sqlite())
            .with_environment(environment)
            .with_param("path", ":memory:");
        let mut fixture = Self {
            connection: config.id,
            session: SessionId::new(),
            runtime,
            backend,
            host,
        };
        fixture.around_the_port(Command::CreateConnection {
            config: Box::new(config.clone()),
        });
        fixture.backend.inner.policy.register(&config);
        let open = fixture
            .runtime
            .block_on(fixture.backend.reconnect(CommandId::new(), config.id))
            .expect("the connection opens");
        fixture.session = open.session.parse().expect("session id");
        for sql in [
            "CREATE TABLE t (id INTEGER, amount INTEGER)",
            "INSERT INTO t VALUES (1, 10), (2, 20)",
            "CREATE VIEW v AS SELECT id FROM t",
        ] {
            if let CommandOutcome::NeedsApproval { command, .. } = fixture.execute(sql) {
                fixture.around_the_port_approve(command.parse().expect("command id"));
            }
        }
        fixture.read_catalog();
        assert!(
            fixture.host.shown().is_empty(),
            "the setup showed no dialog"
        );
        fixture
    }

    fn around_the_port(&self, command: Command) {
        let outcome = self
            .runtime
            .block_on(self.backend.inner.executor.dispatch(
                Actor::Human,
                command,
                &CancelToken::new(),
            ))
            .expect("policy answers");
        if let Outcome::NeedsApproval { command, .. } = outcome {
            self.around_the_port_approve(command);
        }
    }

    fn around_the_port_approve(&self, command: CommandId) {
        self.runtime
            .block_on(
                self.backend
                    .inner
                    .executor
                    .approve("human", command, &CancelToken::new()),
            )
            .expect("approved");
    }

    fn execute(&self, sql: &str) -> CommandOutcome {
        let _guard = self.runtime.enter();
        self.runtime
            .block_on(self.backend.execute(
                CommandId::new(),
                self.connection,
                self.session,
                sql.to_owned(),
            ))
            .expect("the statement answers")
    }

    /// The root, then every level under it, as the tree would expand them.
    /// Every level, read again whatever the cache holds: after a DDL the
    /// executor only invalidates, and the tree reads again on its signal.
    fn read_catalog(&self) {
        let mut levels: Vec<Option<CatalogAddress>> = vec![None];
        let mut read: Vec<Option<CatalogAddress>> = Vec::new();
        while let Some(level) = levels.pop() {
            if read.contains(&level) {
                continue;
            }
            read.push(level.clone());
            let _guard = self.runtime.enter();
            self.runtime
                .block_on(self.backend.refresh_catalog(
                    CommandId::new(),
                    self.connection,
                    self.session,
                    level,
                ))
                .expect("the level reads");
            levels.extend(
                self.nodes()
                    .into_iter()
                    .filter(|node| node.address.relation.is_none())
                    .map(|node| Some(node.address)),
            );
        }
    }

    fn nodes(&self) -> Vec<CatalogNode> {
        fn flatten(nodes: Vec<CatalogNode>, into: &mut Vec<CatalogNode>) {
            for mut node in nodes {
                let children = std::mem::take(&mut node.children);
                into.push(node);
                flatten(children, into);
            }
        }
        let mut all = Vec::new();
        flatten(
            self.backend
                .catalog_tree(self.connection)
                .expect("the tree reads"),
            &mut all,
        );
        all
    }

    fn address(&self, name: &str) -> CatalogAddress {
        self.nodes()
            .into_iter()
            .find(|node| node.address.relation.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("{name} is in the catalog"))
            .address
    }

    fn review(&self, name: &str, operation: &ObjectOperation) -> Result<String, String> {
        self.backend
            .review_object_operation(
                self.connection,
                self.session,
                &self.address(name),
                operation,
            )
            .map(|review| review.sql)
            .map_err(|error| error.message)
    }

    fn submit(&self, operation: OperationKind, sql: &str) -> ObjectOperationOutcome {
        let _guard = self.runtime.enter();
        self.runtime
            .block_on(self.backend.run_object_operation(
                CommandId::new(),
                self.connection,
                operation,
                sql.to_owned(),
            ))
            .expect("submitted")
    }

    fn exists(&self, table: &str) -> bool {
        let _guard = self.runtime.enter();
        self.runtime
            .block_on(self.backend.execute(
                CommandId::new(),
                self.connection,
                self.session,
                format!("SELECT 1 FROM {table} LIMIT 0"),
            ))
            .is_ok()
    }

    fn sessions(&self) -> usize {
        self.backend
            .inner
            .executor
            .sessions()
            .for_connection(self.connection)
            .len()
    }
}

#[test]
fn a_local_drop_is_reviewed_then_held_for_the_approval_dialog_on_its_own_session() {
    let fixture = Fixture::new(Environment::Local);
    let review = fixture
        .backend
        .review_object_operation(
            fixture.connection,
            fixture.session,
            &fixture.address("t"),
            &drop(false),
        )
        .expect("reviewed");
    assert_eq!(review.sql, r#"DROP TABLE "main"."t""#);
    assert_eq!(review.connection_name, NAME);
    assert_eq!(review.environment, Environment::Local);
    assert_eq!(review.object_name, "t");
    assert!(review.transactional_ddl, "SQLite declares it");
    assert!(!review.restrict_dependents, "SQLite does not");
    assert!(matches!(review.dependents, Dependents::IncomingKeys(_)));

    let before = fixture.sessions();
    let command = match fixture.submit(OperationKind::Drop, &review.sql) {
        ObjectOperationOutcome::NeedsApproval { command, .. } => {
            command.parse::<CommandId>().expect("command id")
        }
        other => panic!("a DROP is held back, got {other:?}"),
    };
    assert!(
        fixture.host.shown().is_empty(),
        "off production the webview's approval stands: no host dialog"
    );
    assert_eq!(
        fixture.sessions(),
        before + 1,
        "the review's session waits with its command"
    );
    assert!(fixture.exists("t"), "nothing ran before the decision");

    let _guard = fixture.runtime.enter();
    let decided = fixture
        .runtime
        .block_on(fixture.backend.decide(command, true))
        .expect("decided");
    assert!(
        matches!(decided, CommandOutcome::Executed { .. }),
        "{decided:?}"
    );
    assert!(!fixture.exists("t"), "the approved DROP ran");
    assert_eq!(fixture.sessions(), before, "and its session is closed");
}

#[test]
fn a_rejected_review_closes_its_session_and_runs_nothing() {
    let fixture = Fixture::new(Environment::Development);
    let sql = fixture.review("t", &drop(false)).expect("reviewed");
    let before = fixture.sessions();
    let ObjectOperationOutcome::NeedsApproval { command, .. } =
        fixture.submit(OperationKind::Drop, &sql)
    else {
        panic!("a DROP is held back");
    };
    let _guard = fixture.runtime.enter();
    fixture
        .runtime
        .block_on(
            fixture
                .backend
                .decide(command.parse().expect("command id"), false),
        )
        .expect("rejected");
    assert!(fixture.exists("t"));
    assert_eq!(fixture.sessions(), before);
}

#[test]
fn a_local_rename_runs_at_once_and_closes_its_session() {
    let fixture = Fixture::new(Environment::Local);
    let sql = fixture
        .review("t", &rename(None, "renamed"))
        .expect("reviewed");
    assert_eq!(sql, r#"ALTER TABLE "main"."t" RENAME TO "renamed""#);
    let before = fixture.sessions();
    let outcome = fixture.submit(OperationKind::Rename, &sql);
    assert!(
        matches!(outcome, ObjectOperationOutcome::Applied),
        "{outcome:?}"
    );
    assert!(fixture.exists("renamed") && !fixture.exists("t"));
    assert_eq!(fixture.sessions(), before, "no session outlives the run");

    fixture.read_catalog();
    // A column is renamed from the structure the object view has read.
    let _guard = fixture.runtime.enter();
    fixture
        .runtime
        .block_on(fixture.backend.refresh_relation_facet(
            CommandId::new(),
            fixture.connection,
            fixture.session,
            fixture.address("renamed"),
            crate::ipc::metadata::RelationFacet::Detail,
        ))
        .expect("the structure reads");
    let column = fixture
        .review("renamed", &rename(Some("amount"), "total"))
        .expect("reviewed");
    assert_eq!(
        column,
        r#"ALTER TABLE "main"."renamed" RENAME COLUMN "amount" TO "total""#
    );
    assert!(
        fixture
            .review("renamed", &rename(Some("missing"), "total"))
            .is_err(),
        "a column the catalog does not hold is not renamed"
    );
}

#[test]
fn sqlite_offers_no_truncate_and_no_cascade() {
    let fixture = Fixture::new(Environment::Local);
    assert_eq!(
        fixture.review("t", &ObjectOperation::Truncate { cascade: false }),
        Err(NO_TRUNCATE.to_owned())
    );
    assert!(fixture.review("t", &drop(true)).is_err());
    assert_eq!(
        fixture.review("v", &drop(false)),
        Ok(r#"DROP VIEW "main"."v""#.to_owned())
    );
    assert!(fixture.review("v", &rename(None, "w")).is_err());
    // Submitted anyway, by a script: refused on the review's own session.
    let outcome = fixture.submit(OperationKind::Truncate, r#"TRUNCATE TABLE "main"."t""#);
    assert!(
        matches!(&outcome, ObjectOperationOutcome::Denied { reason } if reason == NO_TRUNCATE),
        "{outcome:?}"
    );
}

#[test]
fn a_statement_other_than_the_announced_one_is_refused_before_any_session() {
    let fixture = Fixture::new(Environment::Local);
    let before = fixture.sessions();
    let _guard = fixture.runtime.enter();
    for (operation, sql) in [
        (OperationKind::Rename, r#"DROP TABLE "main"."t""#),
        (OperationKind::Drop, r#"DROP TABLE "main"."t"; DROP VIEW v"#),
    ] {
        assert!(
            fixture
                .runtime
                .block_on(fixture.backend.run_object_operation(
                    CommandId::new(),
                    fixture.connection,
                    operation,
                    sql.to_owned(),
                ))
                .is_err(),
            "{sql}"
        );
    }
    assert_eq!(fixture.sessions(), before);
    assert!(fixture.exists("t"));
}

#[test]
fn on_production_the_host_dialog_decides_and_the_webview_approves_nothing() {
    let fixture = Fixture::new(Environment::Production);
    let sql = fixture.review("t", &drop(false)).expect("reviewed");
    let before = fixture.sessions();

    // Refused in the host's dialog: nothing runs, nothing waits.
    let refused = fixture.submit(OperationKind::Drop, &sql);
    assert!(
        matches!(refused, ObjectOperationOutcome::Denied { .. }),
        "{refused:?}"
    );
    assert_eq!(fixture.host.shown().len(), 1, "the host was asked");
    assert!(
        fixture.host.shown()[0].body.contains(&sql),
        "the dialog quotes the statement"
    );
    assert!(fixture.exists("t"));
    assert!(fixture.backend.inner.executor.approvals().is_empty());
    assert_eq!(fixture.sessions(), before);

    // Confirmed there: it runs, with no approval handed to the webview.
    fixture.host.answer(Answer::Confirm);
    let applied = fixture.submit(OperationKind::Drop, &sql);
    assert!(
        matches!(applied, ObjectOperationOutcome::Applied),
        "{applied:?}"
    );
    assert_eq!(fixture.host.shown().len(), 2);
    assert!(!fixture.exists("t"));
    assert_eq!(fixture.sessions(), before);
}

#[test]
fn a_production_rename_goes_through_the_host_dialog_too() {
    let fixture = Fixture::new(Environment::Production);
    fixture.host.answer(Answer::Confirm);
    let sql = fixture.review("t", &rename(None, "u")).expect("reviewed");
    let outcome = fixture.submit(OperationKind::Rename, &sql);
    assert!(
        matches!(outcome, ObjectOperationOutcome::Applied),
        "{outcome:?}"
    );
    assert_eq!(fixture.host.shown().len(), 1, "a DDL on production asks");
    assert!(fixture.exists("u"));
}

#[test]
fn a_stale_review_session_is_closed_once_its_command_no_longer_waits() {
    let fixture = Fixture::new(Environment::Local);
    let sql = fixture.review("t", &drop(false)).expect("reviewed");
    let before = fixture.sessions();
    let ObjectOperationOutcome::NeedsApproval { command, .. } =
        fixture.submit(OperationKind::Drop, &sql)
    else {
        panic!("a DROP is held back");
    };
    let command: CommandId = command.parse().expect("command id");
    let _guard = fixture.runtime.enter();
    fixture
        .runtime
        .block_on(fixture.backend.release_stale_reviews());
    assert_eq!(fixture.sessions(), before + 1, "still waiting: kept");

    // Resolved elsewhere than `decide` — expired, or rejected by a window
    // that closed: the next sweep closes it.
    fixture.backend.inner.executor.reject(command);
    fixture
        .runtime
        .block_on(fixture.backend.release_stale_reviews());
    assert_eq!(fixture.sessions(), before);
    assert!(fixture.exists("t"));
}
