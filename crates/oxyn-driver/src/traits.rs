//! The three traits every driver implements.
//!
//! They are those of [`ARCHITECTURE` §4.1](../../../docs/ARCHITECTURE.md):
//! [`Driver`] opens, [`Session`] executes, [`Cursor`] returns the batches. A
//! mistake here is paid fourteen times — the number of real implementations
//! the vision assumes (ADR-0003).
//!
//! # Three hard constraints, not preferences
//!
//! **Object-safe.** The three traits are used behind `Box<dyn ...>`. No
//! generic method, no `impl Trait` in return position, no `where Self: Sized`
//! on a method called through the object. A test of this module checks it,
//! because breaking it is easy and the compiler's message is less so.
//!
//! **Cancellable end to end.** A `&CancelToken` goes through every method that
//! can last. The token **signals**; it is up to the driver to turn it into a
//! `pg_cancel_backend`, a `KILL QUERY` or a `sqlite3_interrupt`, and to declare
//! [`Capabilities::SERVER_SIDE_CANCEL`] if it is able to. Dropping the future
//! releases neither the connection nor the lock
//! ([`DRIVER-CONTRACT` §2](../../../docs/DRIVER-CONTRACT.md)).
//!
//! **Streamed, never materialized.** [`Cursor::next_batch`] returns **one**
//! batch. A driver that builds the whole result before returning makes the RSS
//! climb up to the OOM killer, on a simple click in the tree
//! ([I-06](../../../CLAUDE.md#i-06)).
//!
//! # WASM boundary
//!
//! These traits already honor the constraints of
//! [PLUGIN-CONTRACT](../../../docs/PLUGIN-CONTRACT.md): no generic parameter,
//! no synchronous callback into the host, every error expressed as a value.
//! Fixing them in phase 4 would cost a rework of the fourteen drivers.

use std::time::Duration;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use futures::future::BoxFuture;
use oxyn_catalog::{CatalogPath, CatalogProvider};
use oxyn_core::{
    CancelToken, Capabilities, ConnectionConfig, DriverId, ExecRequest, ExecStats, OxynError,
    PreviewShape, Result, StatementHandle, TransactionState,
};
use oxyn_data::BatchSource;

use crate::context::SessionContext;
use crate::credentials::Credentials;
use crate::metadata::DriverMetadata;

/// A database protocol, not a product.
///
/// `postgres` covers Redshift, TimescaleDB and pgvector; `mysql` covers
/// MariaDB (ADR-0003). A driver has no shared state: it is [`Session`] that
/// carries an open connection.
///
/// # What a driver is not allowed to do
///
/// Read an environment variable, write a file, open a window, log a bound
/// value, or retry on its own. The retry policy belongs to the caller, the only
/// one to know whether the operation can be replayed
/// ([`DRIVER-CONTRACT`](../../../docs/DRIVER-CONTRACT.md)).
#[async_trait]
pub trait Driver: Send + Sync + 'static {
    /// The protocol identifier. Must equal [`DriverMetadata::id`] —
    /// [`DriverRegistry::register`](crate::registry::DriverRegistry::register)
    /// checks it, because a divergence would make the driver unreachable once
    /// registered.
    fn id(&self) -> DriverId;

    /// What the driver says about itself, connection form included.
    ///
    /// The value is **borrowed**: it is built once, when the driver is
    /// created, not each time the list is displayed.
    fn metadata(&self) -> &DriverMetadata;

    /// The capabilities the driver can offer **at best**.
    ///
    /// It is an indicative ceiling, not a promise: what counts is
    /// [`Session::capabilities`], evaluated after connecting. The same
    /// PostgreSQL driver talks to a version 12 database without `MERGE` and to
    /// a version 17 database that has it (ADR-0003).
    fn capabilities(&self) -> Capabilities;

    /// Opens a session.
    ///
    /// `config` carries **no secret**: credentials arrive through
    /// `credentials`, resolved from the system keychain by the caller. A driver
    /// never looks for a password itself, neither in the environment nor in a
    /// file.
    ///
    /// # Deliberate divergence from ARCHITECTURE §4.1
    ///
    /// The document only shows `(cfg, ct)`. That signature authenticates
    /// nobody: `ConnectionConfig` carries only a secret *reference*, which only
    /// `oxyn-secrets` can resolve — and making drivers depend on the system
    /// keychain would reverse the direction of dependencies. The parameter is
    /// therefore explicit. To be carried over into ARCHITECTURE §4.1.
    ///
    /// # Errors
    /// [`OxynError::Connection`] if the server is unreachable,
    /// [`OxynError::Authentication`] if it refuses the credentials,
    /// [`OxynError::Cancelled`] if `cancel` fires during the handshake,
    /// [`OxynError::Config`] if the configuration is unusable.
    async fn connect(
        &self,
        config: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>>;
}

/// An open connection.
///
/// The methods take `&self`: a session is shareable, and synchronizing a client
/// that is not belongs to the driver. It is `Send + Sync` because it lives on
/// the Tokio runtime while the interface thread reads the result buffer
/// (ARCHITECTURE §9).
#[async_trait]
pub trait Session: Send + Sync {
    /// What **this** session can do.
    ///
    /// Evaluated after connecting, from the server version, its extensions and
    /// the account's privileges — not deduced from the driver (ADR-0003). A
    /// missing flag means "I cannot do it", never "I will pretend".
    fn capabilities(&self) -> Capabilities;

    /// Executes a request and returns a cursor over its batches.
    ///
    /// Returns **as soon as the schema is known**, without waiting for the
    /// first row: that is what lets the grid draw its columns while the data
    /// arrives.
    ///
    /// The driver mints a [`StatementHandle`] for this execution and exposes it
    /// through [`Cursor::handle`]: it is what [`Session::cancel`] targets.
    ///
    /// The implementation **refuses** a language it does not declare
    /// ([`Capabilities::require_language`]); it does not translate.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] if the language or a required capability is
    /// missing, [`OxynError::Query`] if the server rejects the statement,
    /// [`OxynError::Cancelled`] if `cancel` fires.
    async fn execute(&self, request: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>>;

    /// Compose a read-only preview, with cancellable metadata I/O if needed.
    ///
    /// Implementations validate `limit` in `1..=1000`, quote every path segment
    /// according to their dialect, and set `read_only` and `max_rows` limits.
    /// This may read column types, but never executes the preview or changes
    /// session state. Unsupported drivers fail explicitly.
    ///
    /// `shape` carries the order, the predicate and the page asked for
    /// ([ADR-0020](../../../docs/adr/0020-apercu-trie-filtre-parcouru.md)). Its
    /// two halves are not alike.
    ///
    /// The **sort** is structured: a column is an identifier the implementation
    /// quotes, because Oxyn composes that fragment and answers for what it
    /// contains ([I-10](../../../CLAUDE.md#i-10)). A column the relation does
    /// not declare is refused, not forwarded in the hope the server rejects it.
    /// The **projection** ([`PreviewShape::projection`]) is structured the same
    /// way: each name is checked against the relation and quoted, and the
    /// server returns those columns only.
    ///
    /// The **predicate** is SQL the user wrote, and it travels through
    /// untouched — neither parsed nor rewritten — exactly like the text of a
    /// console. It still ends up inside a statement Oxyn composes, so the
    /// implementation must make sure it cannot silently swallow what follows
    /// it: an unterminated comment at its end would otherwise eat the very
    /// `LIMIT` that bounds the read.
    ///
    /// What an implementation must not do is ignore part of `shape`. A
    /// predicate silently dropped returns rows the user believes they excluded,
    /// and nothing on screen says otherwise: refuse what the engine cannot
    /// express, with [`OxynError::NotSupported`] naming the missing capability.
    ///
    /// A non-zero [`PreviewShape::offset`] is only meaningful under a total
    /// order — see [`PreviewShape::needs_total_order`]. Implementations
    /// complete the requested sort with a unique key, the primary key when the
    /// catalog declares one, and refuse the page otherwise: without it, two
    /// consecutive pages show the same row twice and skip another, silently.
    /// The completion applies from the first page: ordering page 0 by one
    /// column and page 1 by two would make a row reappear exactly at the
    /// boundary.
    async fn preview_request(
        &self,
        _path: &CatalogPath,
        _limit: u32,
        _shape: &PreviewShape,
        _cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        Err(OxynError::NotSupported {
            capability: "relation preview".to_owned(),
        })
    }

    /// Declares where this session resolves the names a statement does not
    /// qualify, and waits for the server's confirmation.
    ///
    /// Only called if [`Capabilities::SESSION_CONTEXT`] is declared. The driver
    /// **quotes each segment itself**: a schema name is data coming from the
    /// catalog, and concatenating it would execute what it contains
    /// ([I-10](../../../CLAUDE.md#i-10)).
    ///
    /// The implementation touches nothing else. A context change that would
    /// empty along the way a `search_path` composed by the user, or that would
    /// open a transaction, would do more than its name announces — and that is
    /// precisely the invisible session state the contract refuses.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] if the engine has no session context,
    /// [`OxynError::Cancelled`] if `cancel` fires, and the server's error if
    /// the requested location does not exist.
    async fn set_context(&self, _context: &SessionContext, _cancel: &CancelToken) -> Result<()> {
        Err(OxynError::NotSupported {
            capability: "session context".to_owned(),
        })
    }

    /// What the server **confirmed**, never what was requested.
    ///
    /// `None` as long as no context has been declared: the interface then shows
    /// that the session works in what the server chose when opening, which is
    /// not the same thing as a chosen location.
    ///
    /// Returns a value and not a reference: an implementation keeps its
    /// context behind a lock, because `set_context` takes `&self`.
    fn context(&self) -> Option<SessionContext> {
        None
    }

    /// Asks the **server** to interrupt an execution.
    ///
    /// Only called if [`Capabilities::SERVER_SIDE_CANCEL`] is declared: an
    /// implementation that merely drops the future leaves the query running,
    /// the connection taken and the lock held. By the tenth closed tab, the
    /// database refuses connections and the user concludes Oxyn broke their
    /// production.
    ///
    /// Cancelling a statement that already ended is **not** an error.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] if the session cannot cancel server-side,
    /// or any transport error.
    async fn cancel(&self, statement: StatementHandle) -> Result<()>;

    /// The introspection of this session.
    ///
    /// Borrowed, never built on demand: the tree calls it on every node
    /// opened.
    fn catalog(&self) -> &dyn CatalogProvider;

    /// Checks that the connection is alive, and returns the measured round
    /// trip.
    ///
    /// # Errors
    /// Any transport error. A session whose `ping` fails is considered lost.
    async fn ping(&self) -> Result<Duration>;

    /// Closes the session cleanly.
    ///
    /// Consumes the session: a closed session is not reused. A closing error is
    /// reported but not recovered from — local resources are released in every
    /// case.
    ///
    /// # Errors
    /// Any transport error met while closing.
    async fn close(self: Box<Self>) -> Result<()>;

    /// The transaction state, once every operation already submitted on this
    /// session has ended.
    ///
    /// **Ordered after what precedes it**: an execution, `begin`, `commit`,
    /// `rollback` or `set_context` already submitted ends first — succeeded,
    /// failed or interrupted — and the state read is the one it left. That is
    /// what makes the answer right after a Stop: SQLite rolls the transaction
    /// back on its own when interrupted, and a value stored earlier would show
    /// the transaction still open
    /// ([ADR-0039](../../../docs/adr/0039-etat-de-transaction-d-une-session.md)).
    ///
    /// **No server round trip**: it waits for the operations to end, it does
    /// not query the server. If `cancel` fires first, or the session can no
    /// longer answer, it returns [`TransactionState::Unknown`] — never an
    /// error, never [`TransactionState::Idle`] by default.
    ///
    /// **Called once the cursor is released.** A driver may keep its
    /// connection busy for as long as a cursor lives; a call made meanwhile
    /// would wait without bound.
    ///
    /// **Never deduced from the submitted text**: only the engine knows about
    /// an automatic rollback after an error.
    ///
    /// A session that declares [`Capabilities::TRANSACTIONS`] overrides it.
    /// The default says it does not know, which is an honest answer
    /// ([`DRIVER-CONTRACT` §5](../../../docs/DRIVER-CONTRACT.md)).
    async fn transaction_state(&self, cancel: &CancelToken) -> TransactionState {
        let _ = cancel;
        TransactionState::Unknown
    }

    /// Opens a transaction.
    ///
    /// # The default implementation, and what it protects
    ///
    /// It does nothing useful — **on purpose**. It starts by requiring
    /// [`Capabilities::TRANSACTIONS`]: a session that does not declare it
    /// returns [`OxynError::NotSupported`], which is an honest answer. A
    /// session that declares it **and** has not overridden this method returns
    /// [`OxynError::Internal`], because it is a driver bug.
    ///
    /// What must above all not happen is succeeding without opening anything:
    /// the user would believe a `ROLLBACK` undid their write. Not knowing how
    /// is an acceptable answer; letting them believe is not
    /// ([`DRIVER-CONTRACT` §5](../../../docs/DRIVER-CONTRACT.md)).
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] if the session does not have
    /// [`Capabilities::TRANSACTIONS`], [`OxynError::Internal`] if it has it
    /// without implementing the method, or any server error.
    async fn begin(&self, cancel: &CancelToken) -> Result<()> {
        let _ = cancel;
        self.capabilities().require(Capabilities::TRANSACTIONS)?;
        Err(unimplemented_transaction("begin"))
    }

    /// Commits the current transaction.
    ///
    /// Same guard as [`begin`](Self::begin).
    ///
    /// # Errors
    /// Those of [`begin`](Self::begin), plus the server's rejection if no
    /// transaction is open.
    async fn commit(&self, cancel: &CancelToken) -> Result<()> {
        let _ = cancel;
        self.capabilities().require(Capabilities::TRANSACTIONS)?;
        Err(unimplemented_transaction("commit"))
    }

    /// Rolls back the current transaction.
    ///
    /// Same guard as [`begin`](Self::begin). It is the method whose silent
    /// failure costs the most: a `ROLLBACK` that undoes nothing leaves the
    /// write applied.
    ///
    /// # Errors
    /// Those of [`begin`](Self::begin), plus the server's rejection if no
    /// transaction is open.
    async fn rollback(&self, cancel: &CancelToken) -> Result<()> {
        let _ = cancel;
        self.capabilities().require(Capabilities::TRANSACTIONS)?;
        Err(unimplemented_transaction("rollback"))
    }
}

/// A stream of Arrow batches.
///
/// `Send` but not `Sync`: a cursor is moved onto the task that drains it,
/// never shared. Backpressure and spilling to disk are the business of
/// `oxyn-data`; the cursor only returns the next batch when asked.
#[async_trait]
pub trait Cursor: Send {
    /// The handle of the execution that feeds this cursor.
    ///
    /// It is what [`Session::cancel`] targets. **Addition to ARCHITECTURE
    /// §4.1**, where nothing says where the [`StatementHandle`] comes from:
    /// without it, server-side cancellation has no target. To be carried over
    /// into the document.
    fn handle(&self) -> StatementHandle;

    /// The schema of the batches.
    ///
    /// Known **before** the first batch, and stable for the whole stream. A
    /// schemaless source infers it by sampling and marks each field as
    /// inferred; it never presents it as a declaration of the server
    /// ([`DRIVER-CONTRACT` §3](../../../docs/DRIVER-CONTRACT.md)).
    fn schema(&self) -> SchemaRef;

    /// The next batch, or `None` when the stream is exhausted.
    ///
    /// The batch is sized **in bytes, not in rows**: a thousand rows each
    /// carrying a one-megabyte BLOB make a gigabyte, and a `batch_size` counted
    /// in rows works on demo tables before triggering the OOM on real ones.
    ///
    /// The future must be droppable. After a drop, the cursor is **unusable**:
    /// it may have consumed bytes of the stream, leaving the decoder out of
    /// sync. It is destroyed, not resumed.
    ///
    /// # Errors
    /// Any server or decoding error, **classified**: [`OxynError::driver`] with
    /// the right [`ErrorClass`](oxyn_core::ErrorClass). A client-side timeout
    /// during a write is [`Ambiguous`](oxyn_core::ErrorClass::Ambiguous), never
    /// transient (I-13).
    async fn next_batch(&mut self) -> Result<Option<RecordBatch>>;

    /// What the cursor knows about the execution.
    ///
    /// Queried at the end of the stream to close the buffer. Server time, when
    /// the server returns it, is what tells a slow database from a slow
    /// network.
    fn stats(&self) -> ExecStats;
}

/// A [`Cursor`] is a batch source for `oxyn-data`.
///
/// The adaptation is the junction between the two crates: `oxyn-data` does
/// not depend on `oxyn-driver` — it is the other way round — so this is where
/// the bridge goes. [`BatchSink`](oxyn_data::BatchSink) can thus drain a driver
/// cursor with backpressure, without any driver having to know the buffer.
///
/// The implementation targets `Box<dyn Cursor>` and not `dyn Cursor`: that is
/// the form in which the cursor travels, and `BatchSource` implicitly requires
/// `Sized` for its implementors.
impl BatchSource for Box<dyn Cursor> {
    fn schema(&self) -> SchemaRef {
        (**self).schema()
    }

    fn next_batch(&mut self) -> BoxFuture<'_, std::result::Result<Option<RecordBatch>, OxynError>> {
        (**self).next_batch()
    }

    fn stats(&self) -> ExecStats {
        (**self).stats()
    }
}

/// The error of a session that declares transactions without implementing
/// them.
///
/// It is a driver bug, not a usage error: the message says so, so that it is
/// not shown to the user as a limitation of the server.
fn unimplemented_transaction(operation: &'static str) -> OxynError {
    OxynError::Internal(format!(
        "the session declares TRANSACTIONS without implementing `{operation}`: \
         a ROLLBACK that undoes nothing leaves the write applied"
    ))
}

#[cfg(test)]
mod tests {
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
}
