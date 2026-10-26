use std::sync::Arc;

use arrow::array::Int32Array;
use arrow::datatypes::{DataType, Field, Schema};
use futures::executor::block_on;
use oxyn_catalog::model::{Relation, RelationKind, RelationRef, ServerInfo};
use oxyn_catalog::path::CatalogPath;
use oxyn_data::{BatchSink, ResultBuffer};

use super::*;
use crate::metadata::DriverFamily;

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)]))
}

fn batch(start: i32, rows: i32) -> RecordBatch {
    let ids: Vec<i32> = (start..start.saturating_add(rows)).collect();
    RecordBatch::try_new(schema(), vec![Arc::new(Int32Array::from(ids))])
        .expect("the column matches the schema built just above")
}

#[derive(Debug)]
struct FakeCatalog;

#[async_trait]
impl CatalogProvider for FakeCatalog {
    async fn server_info(&self, _cancel: &CancelToken) -> Result<ServerInfo> {
        Ok(ServerInfo::new(
            "Fake",
            "1.0",
            Capabilities::SQL | Capabilities::RELATIONAL,
        ))
    }

    async fn list_relations(
        &self,
        _namespace: &CatalogPath,
        _cancel: &CancelToken,
    ) -> Result<Vec<RelationRef>> {
        Ok(Vec::new())
    }

    async fn describe_relation(
        &self,
        relation: &CatalogPath,
        _cancel: &CancelToken,
    ) -> Result<Relation> {
        Ok(Relation::new(
            relation.relation().unwrap_or(""),
            RelationKind::Table,
        ))
    }
}

#[derive(Debug)]
struct FakeCursor {
    handle: StatementHandle,
    remaining: Vec<RecordBatch>,
    stats: ExecStats,
}

impl FakeCursor {
    fn new(batches: Vec<RecordBatch>) -> Self {
        Self {
            handle: StatementHandle::new(),
            remaining: batches,
            stats: ExecStats::default(),
        }
    }
}

#[async_trait]
impl Cursor for FakeCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if self.remaining.is_empty() {
            return Ok(None);
        }
        let batch = self.remaining.remove(0);
        self.stats.record_batch(
            u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
            u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
        );
        Ok(Some(batch))
    }

    fn stats(&self) -> ExecStats {
        self.stats
    }
}

#[derive(Debug)]
struct FakeSession {
    capabilities: Capabilities,
    catalog: FakeCatalog,
}

impl FakeSession {
    fn new(capabilities: Capabilities) -> Self {
        Self {
            capabilities,
            catalog: FakeCatalog,
        }
    }
}

#[async_trait]
impl Session for FakeSession {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    async fn execute(
        &self,
        request: ExecRequest,
        _cancel: &CancelToken,
    ) -> Result<Box<dyn Cursor>> {
        self.capabilities.require_language(request.language)?;
        Ok(Box::new(FakeCursor::new(vec![batch(0, 3), batch(3, 2)])))
    }

    async fn cancel(&self, _statement: StatementHandle) -> Result<()> {
        Ok(())
    }

    fn catalog(&self) -> &dyn CatalogProvider {
        &self.catalog
    }

    async fn ping(&self) -> Result<Duration> {
        Ok(Duration::from_millis(1))
    }

    async fn close(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
struct FakeDriver {
    metadata: DriverMetadata,
}

impl FakeDriver {
    fn new() -> Self {
        Self {
            metadata: DriverMetadata::new(
                DriverId::sqlite(),
                "Fake SQLite",
                DriverFamily::Relational,
            ),
        }
    }
}

#[async_trait]
impl Driver for FakeDriver {
    fn id(&self) -> DriverId {
        self.metadata.id.clone()
    }

    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::SQL | Capabilities::RELATIONAL
    }

    async fn connect(
        &self,
        _config: &ConnectionConfig,
        _credentials: &Credentials,
        _cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        Ok(Box::new(FakeSession::new(
            Capabilities::SQL | Capabilities::RELATIONAL | Capabilities::STREAMING,
        )))
    }
}

#[test]
fn the_three_traits_stay_object_safe() {
    // Hard constraint of ARCHITECTURE §4.1. A generic method would break
    // it, and the compiler's message would not say why.
    let driver: Box<dyn Driver> = Box::new(FakeDriver::new());
    let config = ConnectionConfig::new("workshop", DriverId::sqlite());
    let credentials = Credentials::new();
    let token = CancelToken::new();

    let session: Box<dyn Session> =
        block_on(driver.connect(&config, &credentials, &token)).expect("fake connection");
    assert!(session.capabilities().contains(Capabilities::SQL));

    let cursor: Box<dyn Cursor> = block_on(session.execute(
        ExecRequest::new(oxyn_core::QueryLanguage::SQL, "SELECT 1"),
        &token,
    ))
    .expect("fake execution");
    assert_eq!(cursor.schema().fields().len(), 1);

    block_on(session.close()).expect("closing");
}

#[test]
fn an_undeclared_language_is_refused_not_translated() {
    let session = FakeSession::new(Capabilities::SQL);
    let token = CancelToken::new();
    let request = ExecRequest::new(oxyn_core::QueryLanguage::Cypher, "MATCH (n) RETURN n");

    // See `registry.rs`: `expect_err` would require `Debug` on `dyn Cursor`.
    let err = match block_on(session.execute(request, &token)) {
        Ok(_) => panic!("refusal expected: Cypher is not declared"),
        Err(err) => err,
    };
    assert!(matches!(err, OxynError::NotSupported { .. }), "{err:?}");
    assert!(err.is_user_error(), "it is not an incident");
}

#[test]
fn a_session_without_transactions_says_so_instead_of_pretending() {
    // DRIVER-CONTRACT §5: not knowing how is an acceptable answer; letting
    // the user believe a ROLLBACK undid the write is not.
    let session = FakeSession::new(Capabilities::SQL);
    let token = CancelToken::new();

    for outcome in [
        block_on(session.begin(&token)),
        block_on(session.commit(&token)),
        block_on(session.rollback(&token)),
    ] {
        let err = outcome.expect_err("refusal expected");
        assert!(matches!(err, OxynError::NotSupported { .. }), "{err:?}");
        assert!(err.to_string().contains("TRANSACTIONS"), "{err}");
    }
}

#[test]
fn declaring_transactions_without_implementing_them_is_a_bug_not_a_success() {
    // The worst case would be succeeding without opening anything.
    let session = FakeSession::new(Capabilities::SQL | Capabilities::TRANSACTIONS);
    let token = CancelToken::new();

    let err = block_on(session.rollback(&token)).expect_err("refusal expected");
    assert!(matches!(err, OxynError::Internal(_)), "{err:?}");
    assert!(
        !err.is_user_error(),
        "it is a driver bug, not a usage error"
    );
}

#[test]
fn by_default_a_session_does_not_claim_to_know_its_transaction() {
    // ADR-0039: a driver that implemented nothing does not say "no
    // transaction", even if it declares the capability.
    let token = CancelToken::new();
    for capabilities in [
        Capabilities::SQL,
        Capabilities::SQL | Capabilities::TRANSACTIONS,
    ] {
        let session = FakeSession::new(capabilities);
        assert_eq!(
            block_on(session.transaction_state(&token)),
            TransactionState::Unknown
        );
    }
}

#[test]
fn a_cursor_feeds_a_result_buffer_directly() {
    // It is the junction between `oxyn-driver` and `oxyn-data`: if this
    // adaptation breaks, nothing connects a driver to the screen anymore.
    let cursor: Box<dyn Cursor> = Box::new(FakeCursor::new(vec![batch(0, 3), batch(3, 2)]));
    let handle = cursor.handle();

    let buffer = Arc::new(ResultBuffer::new(BatchSource::schema(&cursor), 1 << 20));
    let sink = BatchSink::new(Arc::clone(&buffer));
    let mut source = cursor;

    let outcome = block_on(sink.drain(&mut source, &CancelToken::new())).expect("draining");

    assert!(
        outcome.is_complete(),
        "the fake source runs out: {outcome:?}"
    );
    assert_eq!(buffer.row_count(), 5);
    assert_eq!(buffer.batch_count(), 2);

    // The handle stays that of the execution: it is what
    // `Session::cancel` targets.
    assert_eq!(BatchSource::stats(&source).rows, 5);
    assert_eq!(source.handle(), handle);
}

#[test]
fn an_exhausted_cursor_returns_none_rather_than_an_error() {
    let mut cursor = FakeCursor::new(Vec::new());
    let batch = block_on(cursor.next_batch()).expect("no error");
    assert!(batch.is_none());
    assert!(cursor.stats().is_empty());
}
