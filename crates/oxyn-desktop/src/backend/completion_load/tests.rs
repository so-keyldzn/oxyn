//! The editor's pass on a real SQLite connection whose tree was never
//! expanded: what it reads, as whom, and what the snapshot shows afterwards.

use oxyn_core::Environment;
use oxyn_store::{ActorKind, PolicyOutcome};

use super::*;
use crate::backend::ai::catalog_fill::tests::{Opened, shop};
use crate::ipc::completion::{CompletionCatalog, CompletionFields, CompletionResolution};

fn bare(text: &str) -> CompletionNamePart {
    CompletionNamePart {
        text: text.to_owned(),
        quoted: false,
    }
}

fn quoted(text: &str) -> CompletionNamePart {
    CompletionNamePart {
        text: text.to_owned(),
        quoted: true,
    }
}

impl Opened {
    fn load(&self, names: Vec<Vec<CompletionNamePart>>) -> CompletionLoad {
        let _guard = self.runtime.enter();
        self.runtime
            .block_on(self.backend.load_completion(self.connection, names))
            .expect("a connected catalog")
    }

    fn snapshot(&self) -> CompletionCatalog {
        self.backend
            .completion_catalog(self.connection)
            .expect("a connected catalog")
    }
}

fn fields_of<'a>(snapshot: &'a CompletionCatalog, relation: &str) -> &'a CompletionFields {
    let main = snapshot
        .namespaces
        .iter()
        .find(|space| space.name.as_deref() == Some("main"))
        .expect("`main` is in the snapshot");
    &main
        .relations
        .iter()
        .find(|one| one.name == relation)
        .unwrap_or_else(|| panic!("{relation} is not in the snapshot"))
        .fields
}

#[test]
fn a_pass_reads_the_tables_and_only_the_columns_the_statement_names() {
    let shop = shop(Environment::Local);
    let before = shop.snapshot();
    assert!(
        before
            .namespaces
            .iter()
            .all(|space| space.relations.is_empty()),
        "nothing expanded: {before:?}"
    );
    assert_eq!(before.resolution, CompletionResolution::Unknown);

    let load = shop.load(vec![vec![bare("orders")]]);
    assert_eq!((load.failed, load.stopped), (0, None), "{load:?}");
    assert!(
        load.attempted >= 3,
        "the server, the path, the tables, one description: {load:?}"
    );

    let after = shop.snapshot();
    assert_eq!(
        after.resolution,
        CompletionResolution::Known {
            namespaces: vec!["main".into()],
            stale: false
        },
        "read from the server, not assumed"
    );
    match fields_of(&after, "orders") {
        CompletionFields::Loaded { items } => {
            let names: Vec<_> = items.iter().map(|field| field.name.as_str()).collect();
            assert_eq!(names, ["id", "customer_id", "total"]);
        }
        other => panic!("the named table is described: {other:?}"),
    }
    for untouched in ["customers", "audit"] {
        assert_eq!(
            fields_of(&after, untouched),
            &CompletionFields::NotLoaded,
            "{untouched} was not named: its columns stay unread"
        );
    }
}

#[test]
fn a_second_pass_over_the_same_statement_reads_nothing() {
    let shop = shop(Environment::Local);
    let names = || vec![vec![bare("orders")], vec![bare("customers")]];
    assert!(shop.load(names()).attempted > 0);
    let journaled = shop.reads().len();

    let again = shop.load(names());
    assert_eq!(again.attempted, 0, "{again:?}");
    assert_eq!(shop.reads().len(), journaled);
}

#[test]
fn a_name_the_database_does_not_have_reads_no_column() {
    let shop = shop(Environment::Local);
    let load = shop.load(vec![vec![bare("nothing")]]);
    assert_eq!(load.failed, 0, "{load:?}");
    let snapshot = shop.snapshot();
    for relation in ["orders", "customers", "audit"] {
        assert_eq!(fields_of(&snapshot, relation), &CompletionFields::NotLoaded);
    }
}

#[test]
fn every_read_is_a_catalog_command_of_the_human_allowed_on_production() {
    let shop = shop(Environment::Production);
    let load = shop.load(vec![vec![bare("orders")]]);
    assert_eq!(load.failed, 0, "{load:?}");

    let reads = shop.reads();
    assert_eq!(reads.len(), load.attempted as usize, "one command per read");
    assert!(
        reads
            .iter()
            .any(|read| read.command_kind == "RefreshCatalogScope"),
        "the path is a scope of its own"
    );
    for read in &reads {
        assert_eq!(read.actor_kind, ActorKind::Human);
        assert_eq!(read.decision, PolicyOutcome::Allowed, "{read:?}");
        assert_eq!(read.connection, Some(shop.connection));
        assert!(read.statement.is_none(), "no SQL of Oxyn's: {read:?}");
    }
}

#[test]
fn no_row_value_reaches_the_snapshot() {
    let shop = shop(Environment::Local);
    shop.load(vec![vec![bare("customers")], vec![bare("audit")]]);
    let json = serde_json::to_string(&shop.snapshot()).expect("serializes");
    for value in ["user1@example.com", "SECRET-1"] {
        assert!(!json.contains(value), "{value} left the database: {json}");
    }
}

#[test]
fn a_ddl_makes_the_next_pass_read_again_and_find_the_new_table() {
    let shop = shop(Environment::Local);
    shop.load(vec![vec![bare("orders")]]);

    shop.sql("CREATE TABLE refunds (id INTEGER PRIMARY KEY, reason TEXT)");
    let load = shop.load(vec![vec![bare("refunds")]]);
    assert_eq!(load.failed, 0, "{load:?}");
    match fields_of(&shop.snapshot(), "refunds") {
        CompletionFields::Loaded { items } => {
            assert_eq!(items.len(), 2, "id and reason");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_hostile_table_name_is_described_and_runs_nothing() {
    let shop = shop(Environment::Production);
    let hostile = "users\"; DROP TABLE audit; --";
    shop.sql(&format!(
        "CREATE TABLE \"{}\" (id INTEGER PRIMARY KEY)",
        hostile.replace('"', "\"\"")
    ));
    let load = shop.load(vec![vec![quoted(hostile)]]);
    assert_eq!(load.failed, 0, "{load:?}");
    assert!(matches!(
        fields_of(&shop.snapshot(), hostile),
        CompletionFields::Loaded { .. }
    ));
    // `audit` is still there, and every command the pass journaled is a read.
    let snapshot = shop.snapshot();
    assert!(
        snapshot
            .namespaces
            .iter()
            .flat_map(|space| &space.relations)
            .any(|relation| relation.name == "audit")
    );
    assert!(
        shop.reads()
            .iter()
            .all(|read| read.command_kind.starts_with("RefreshCatalog"))
    );
}

#[test]
fn a_pass_on_a_connection_that_does_not_exist_says_so() {
    let shop = shop(Environment::Local);
    let _guard = shop.runtime.enter();
    let result = shop.runtime.block_on(
        shop.backend
            .load_completion(ConnectionId::new(), vec![vec![bare("orders")]]),
    );
    assert!(result.is_err());
}

#[test]
fn a_newer_pass_cancels_the_one_it_replaces_and_the_older_one_leaves_no_trace_of_its_end() {
    let passes = Passes::default();
    let connection = ConnectionId::new();
    let other = ConnectionId::new();

    let (first, first_cancel) = passes.begin(connection);
    let (_, other_cancel) = passes.begin(other);
    let (second, second_cancel) = passes.begin(connection);
    assert!(first_cancel.is_cancelled(), "replaced");
    assert!(!second_cancel.is_cancelled());
    assert!(!other_cancel.is_cancelled(), "another connection's pass");

    // The older pass ends after the newer one began: it must not remove it.
    passes.end(connection, first);
    let (_, third_cancel) = passes.begin(connection);
    assert!(
        second_cancel.is_cancelled(),
        "the newer pass was still registered"
    );
    passes.end(connection, second);
    assert!(
        !third_cancel.is_cancelled(),
        "ending a replaced pass leaves the current one alone"
    );
}

#[test]
fn the_names_a_pass_acts_on_are_bounded() {
    let long = "x".repeat(PRODUCT_BOUNDS.name_bytes + 1);
    let kept = bounded(vec![
        vec![bare("a")],
        vec![bare("a"), bare("b")],
        vec![bare("a"), bare("b"), bare("c")],
        vec![],
        vec![bare(&long)],
        vec![bare("a"), bare(&long)],
    ]);
    assert_eq!(kept.len(), 2, "{kept:?}");

    let many: Vec<_> = (0..MAX_NAMES * 2)
        .map(|n| vec![bare(&format!("t{n}"))])
        .collect();
    assert_eq!(bounded(many).len(), MAX_NAMES);
}
