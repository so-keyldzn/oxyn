//! An open connection: execute, cancel, probe, close.
//!
//! # What governs this file
//!
//! **Cancellation goes out from a second connection.** `pg_cancel_backend` is
//! an ordinary SQL function: calling it requires a free connection. Yet it is
//! exactly when all connections are taken by long queries that one wants to
//! cancel. `BackendCanceller` therefore opens a **fresh** connection, outside
//! the pool, rather than wait for a slot to free up
//! ([DRIVER-CONTRACT §2](../../../docs/DRIVER-CONTRACT.md)).
//!
//! **Cancellation is sent by whoever holds the connection.** A session relies on
//! a pool, so each execution runs on a different server process, and a single
//! process serves one query after another. [`Session::cancel`] therefore does
//! not target a pid: it wakes the execution's stream task, which knows its pid
//! and **keeps its connection** until the cancellation has gone out. Sent from
//! here, it could land on the next query, started on the same process during
//! the cancellation's handshake. The details are in the `cancel` module.
//!
//! **The user's SQL is sent as is; Oxyn's concatenates nothing.** The text of
//! an [`ExecRequest`] is prepared without being parsed or rewritten: that is
//! the feature of a professional tool. The queries the driver **composes** —
//! introspection, cancellation — are literals with bound parameters
//! ([I-10](../../../CLAUDE.md#i-10)).
//!
//! **Read-only is imposed by the server.** When
//! [`ExecLimits::read_only`](oxyn_core::ExecLimits::read_only) is true, the
//! execution is wrapped in `BEGIN READ ONLY`: the server refuses the write, not
//! a client-side filter. That is what
//! [`Capabilities::READ_ONLY_SESSION`](oxyn_core::Capabilities::READ_ONLY_SESSION)
//! declares.

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use oxyn_catalog::CatalogProvider;
use oxyn_core::{
    CancelToken, Capabilities, DriverId, ExecRequest, OxynError, Result, ScalarValue, SqlDialect,
    StatementHandle, StatementIntent,
};
use oxyn_driver::{Cursor, Session};
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgArguments, PgConnection, PgPool, Postgres};
use sqlx::{Arguments as _, AssertSqlSafe, Connection as _, Executor as _};
use sqlx::{Column as _, Row as _, SqlSafeStr as _, Statement as _};

use crate::cancel::{BackendCanceller, StatementRegistry};
use crate::catalog::PostgresCatalog;
use crate::cursor::{self, Source, StreamRequest};
use crate::error::{
    Bound, is_unresolvable_type, map_connect_error, map_exec_error, map_stream_error,
};
use crate::lease::Lease;
use crate::options::ConnectSpec;
use crate::transaction_text::{controls_transaction, opens_transaction};

/// What the refusal of a transaction control statement says.
///
/// Names the missing capability, then what the user must know not to be
/// mistaken about their data: without transactions, each statement commits on
/// its own.
const TRANSACTIONS_REFUSED: &str = "TRANSACTIONS (transactions are not supported in the console \
                                    yet: each statement commits on its own)";
use crate::types::{lacks_binary_output, schema_for};
use crate::variant::PostgresVariant;
use oxyn_catalog::path::{QuoteStyle, quote_identifier};
use oxyn_driver::SessionContext;

/// Opens a read-only transaction for the duration of an execution.
///
/// A literal: nothing in it is composed.
pub(crate) const SQL_BEGIN_READ_ONLY: &str = "BEGIN READ ONLY";
/// Closes the transaction opened by [`SQL_BEGIN_READ_ONLY`].
pub(crate) const SQL_ROLLBACK: &str = "ROLLBACK";
/// Gives `search_path` back its default value before the connection goes back
/// to the pool.
///
/// A literal: a console's context must not be carried by the connection to
/// introspection or to another console.
pub(crate) const SQL_RESET_SEARCH_PATH: &str = "SET search_path TO DEFAULT";
/// Resets the session state to the default after a **writable** execution,
/// which has no `ROLLBACK` to undo what the user may have set.
///
/// A literal, two statements in a single round trip (simple protocol):
///
/// * `standard_conforming_strings`: `oxyn-query`'s splitter assumes `on`. A
///   connection returned to the pool with `off` would make the server read a
///   `DELETE` where the splitter saw a string, hence a write classified as a
///   read, without the confirmation that names the connection
///   ([I-02](../../../CLAUDE.md#i-02)). See [`ConnectSpec`](crate::ConnectSpec)
///   for the value set at opening;
/// * `search_path`: a `SET` typed in a console would otherwise travel to the
///   next borrower. It is not reliable for the user anyway, whose next query
///   may go out on another connection; a console's schema goes through its
///   session context.
pub(crate) const SQL_RESET_AFTER_WRITE: &str =
    "SET standard_conforming_strings TO on; SET search_path TO DEFAULT";
/// The pid of the server process executing on this connection.
const SQL_BACKEND_PID: &str = "SELECT pg_catalog.pg_backend_pid()";

/// Does the requested schema exist, and does this role see it?
///
/// `SET search_path` silently accepts a missing schema; this query is what
/// makes the difference between an applied context and a displayed one. The
/// name travels **bound**, never concatenated (I-10).
const SQL_NAMESPACE_EXISTS: &str = "SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname = $1 \
     AND pg_catalog.has_schema_privilege(oid, 'USAGE')";

/// An open PostgreSQL session.
#[derive(Debug)]
pub struct PostgresSession {
    driver: DriverId,
    pool: PgPool,
    canceller: Arc<BackendCanceller>,
    statements: Arc<StatementRegistry>,
    variant: PostgresVariant,
    capabilities: Capabilities,
    catalog: PostgresCatalog,
    /// The connection's database, to refuse a context that designates another
    /// one: a PostgreSQL session does not change database.
    database: String,
    /// What the server confirmed, or `None` as long as nothing was declared.
    ///
    /// Under a synchronous lock: it is never held across an `await`, only read
    /// the time it takes to compose a statement.
    context: Mutex<Option<SessionContext>>,
}

impl PostgresSession {
    /// Assembles a session from an already open pool and an already detected
    /// variant.
    ///
    /// Called by [`PostgresDriver::connect`](crate::driver::PostgresDriver);
    /// detection is done there because it needs a connection, and it is the
    /// only moment when failure can still translate into a connection refusal
    /// rather than into a wrong capability.
    #[must_use]
    pub(crate) fn new(
        driver: DriverId,
        pool: PgPool,
        spec: ConnectSpec,
        variant: PostgresVariant,
        database: String,
    ) -> Self {
        let capabilities = variant.capabilities();
        // The canceller is shared with the catalog: a long introspection must
        // be cuttable on the server just like a query.
        let canceller = Arc::new(BackendCanceller::new(spec, driver.clone()));
        let catalog = PostgresCatalog::new(
            driver.clone(),
            pool.clone(),
            database.clone(),
            variant.clone(),
            capabilities,
            Arc::clone(&canceller),
        );
        Self {
            canceller,
            statements: Arc::new(StatementRegistry::default()),
            driver,
            pool,
            variant,
            capabilities,
            catalog,
            database,
            context: Mutex::new(None),
        }
    }

    /// The statement that sets the context on an execution's connection.
    ///
    /// `None` when there is nothing to set and nothing was ever set: the
    /// connection is then in the state the server opened it in. As soon as a
    /// context has been declared, even to go back to the default, the statement
    /// is issued — a pool connection can carry the state of a previous
    /// execution, and `search_path` is a **per-connection** state.
    fn context_statement(&self) -> Option<String> {
        let guard = self
            .context
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let context = guard?;
        Some(match context.namespace() {
            // Quoted by the driver: a schema name comes from the catalog, hence
            // from the server, and concatenating it would execute what it
            // contains (I-10).
            Some(namespace) => format!(
                "SET search_path TO {}",
                quote_identifier(namespace, QuoteStyle::for_dialect(SqlDialect::Postgres))
            ),
            None => "SET search_path TO DEFAULT".to_owned(),
        })
    }

    /// What the session learned from its server.
    #[must_use]
    pub const fn variant(&self) -> &PostgresVariant {
        &self.variant
    }

    /// Running executions, that is, cancellable ones. Diagnostics and tests.
    #[doc(hidden)]
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.statements.len()
    }

    /// The canceller, for tests that hold back an in-flight cancellation.
    #[cfg(test)]
    pub(crate) fn canceller(&self) -> &BackendCanceller {
        &self.canceller
    }

    /// Declares a capability the driver does not implement, for tests that
    /// exercise what happens **behind** a refusal: the safety net that closes a
    /// connection left in a transaction.
    #[cfg(test)]
    pub(crate) fn declare_for_test(&mut self, capabilities: Capabilities) {
        self.capabilities.insert(capabilities);
    }

    /// Borrows a connection from the pool, without ever becoming
    /// uncancellable.
    async fn acquire(&self, cancel: &CancelToken) -> Result<PoolConnection<Postgres>> {
        race_cancel(cancel, async {
            self.pool
                .acquire()
                .await
                .map_err(|error| map_connect_error(&error))
        })
        .await
    }
}

#[async_trait]
impl Session for PostgresSession {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Declares where unqualified names resolve, after checking.
    ///
    /// PostgreSQL **silently accepts** a `SET search_path` to a schema that does
    /// not exist: without the prior check, Oxyn would display a context the
    /// server does not apply. The name is compared through a bound value, not
    /// concatenated.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the context designates another database — a
    /// PostgreSQL session does not change database — or a missing schema;
    /// [`OxynError::Cancelled`] if `cancel` fires.
    async fn set_context(&self, context: &SessionContext, cancel: &CancelToken) -> Result<()> {
        if let Some(catalog) = context.catalog()
            && catalog != self.database
        {
            return Err(OxynError::Config(
                "a PostgreSQL session cannot change database; \
                 open a connection to that database instead"
                    .to_owned(),
            ));
        }
        if let Some(namespace) = context.namespace() {
            let mut connection = self.acquire(cancel).await?;
            let found: Option<i32> = race_cancel(cancel, async {
                sqlx::query_scalar(SQL_NAMESPACE_EXISTS)
                    .bind(namespace)
                    .fetch_optional(&mut *connection)
                    .await
                    .map_err(|error| map_exec_error(&self.driver, StatementIntent::Read, error))
            })
            .await?;
            if found.is_none() {
                // The requested name comes from the displayed catalog: quoting
                // it here reveals nothing the user does not already see, and
                // without it the message would not say what to fix.
                return Err(OxynError::Config(format!(
                    "schema `{namespace}` does not exist or is not visible to this role"
                )));
            }
        }
        *self.context.lock().unwrap_or_else(PoisonError::into_inner) = Some(context.clone());
        Ok(())
    }

    fn context(&self) -> Option<SessionContext> {
        self.context
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Prepares, then starts the stream.
    ///
    /// Returns **as soon as the schema is known**: it comes from the prepared
    /// statement, not from the first row. That is what lets the grid draw its
    /// columns while the data is still arriving.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] if the language is not SQL, or if a bound
    /// parameter carries a type the driver cannot encode;
    /// [`OxynError::Cancelled`] if `cancel` fires before departure;
    /// [`OxynError::Query`] if the server rejects the statement; any transport
    /// error, classified.
    async fn execute(&self, request: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.capabilities.require_language(request.language)?;
        // Before any borrow: nothing goes out to the server. See
        // `transaction_text::controls_transaction` for the lie this refusal
        // prevents — a `ROLLBACK` that "succeeds" without cancelling anything.
        if !self.capabilities.contains(Capabilities::TRANSACTIONS)
            && controls_transaction(&request.text)
        {
            return Err(OxynError::NotSupported {
                capability: TRANSACTIONS_REFUSED.to_owned(),
            });
        }
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }

        // Taken before the request is taken apart: `is_mutating` also counts
        // the reported risk, which the cursor does not receive.
        let confirm_end = !request.is_mutating();
        let ExecRequest {
            text,
            params,
            intent,
            limits,
            ..
        } = request;

        // Recorded **before** the values go into `PgArguments`, then into
        // `query_with`: from there on, nobody on the path knows there were bound
        // values any more, and the server's message may quote one (I-03).
        let bound = Bound::of(&params);

        // The parameters are encoded before any round trip: a non-encodable
        // type must be refused without having occupied a connection.
        let arguments = bind_params(&params)?;

        let mut connection = Lease::new(self.acquire(cancel).await?);

        let pid = race_cancel(cancel, async {
            backend_pid(&mut connection)
                .await
                .map_err(|error| map_exec_error(&self.driver, StatementIntent::Read, error))
        })
        .await?;

        // From the `SET` or the `BEGIN` on, the connection carries a state that
        // is not the pool's, and the cursor resets it to the default, and only
        // the cursor. The borrow is therefore marked **before** sending: any
        // early departure — an error, a `?`, and above all an abandoned future,
        // which goes through neither — closes the connection instead of
        // returning it. Otherwise a console's context would travel to
        // introspection or to another console
        // ([ADR-0019](../../../docs/adr/0019-contexte-de-session.md)). Reopening
        // a connection costs less than a `DELETE` resolved in another schema.

        // Before the transaction: set inside it, a `SET` would be undone by the
        // `ROLLBACK` that closes a read-only execution, and the next statement
        // on the same connection would resolve elsewhere.
        let restore_context = self.context_statement().is_some();
        if let Some(statement) = self.context_statement() {
            let sql = AssertSqlSafe(statement).into_sql_str();
            connection.taint();
            race_cancel(cancel, async {
                sqlx::raw_sql(sql)
                    .execute(&mut *connection)
                    .await
                    .map_err(|error| map_exec_error(&self.driver, StatementIntent::Read, error))
            })
            .await?;
        }

        if limits.read_only {
            connection.taint();
            race_cancel(cancel, async {
                sqlx::raw_sql(SQL_BEGIN_READ_ONLY)
                    .execute(&mut *connection)
                    .await
                    .map_err(|error| map_exec_error(&self.driver, StatementIntent::Read, error))
            })
            .await?;
        }

        // The user's SQL goes out **as is**: that is the feature of a
        // professional tool, and the distinction with the SQL Oxyn composes is
        // what I-10 guards. Nothing is concatenated here: the text is only
        // read, to know whether its connection can be returned.
        let opens_transaction = opens_transaction(&text);
        // Kept for the simple protocol (ADR-0048): `prepare` consumes the text,
        // and `SqlStr` is not `Clone`. One copy per statement, not per row.
        let fallback_text = text.clone();
        let sql = AssertSqlSafe(text).into_sql_str();
        let prepared =
            race_cancel(cancel, async { Ok((&mut *connection).prepare(sql).await) }).await?;

        // ADR-0048: a statement `sqlx` cannot prepare, or whose result has a
        // type the server cannot send in binary, runs in the simple protocol.
        // Nothing has run yet — `prepare` only sends Parse and Describe — and
        // the server's Parse accepted the text, which it refuses when it holds
        // more than one command: the simple protocol, which would run several,
        // receives one.
        let source = match prepared {
            Ok(statement)
                if !statement
                    .columns()
                    .iter()
                    .any(|c| lacks_binary_output(c.type_info())) =>
            {
                Source::Prepared {
                    statement,
                    arguments,
                }
            }
            Ok(_) => simple_source(params.is_empty(), fallback_text, NO_BINARY_OUTPUT)?,
            Err(error) if is_unresolvable_type(&error) => {
                simple_source(params.is_empty(), fallback_text, UNRESOLVABLE_TYPE)?
            }
            // A failed preparation closes the connection if it is already
            // dirty: the transaction opened above would keep locks and block
            // the `VACUUM` of the whole database, and a console's `search_path`
            // would leave with it. A typo in a table name in a writable console
            // is enough.
            //
            // `Bound::Internal`, whatever the request's values: `prepare` only
            // issues Parse and Describe, which carry the SQL text and the
            // parameters' OIDs — never their content, which only goes out at
            // Bind, in `cursor.rs`. The server can therefore quote nothing
            // here, and withholding its message would cost the most useful
            // diagnostic of a parameterized query: the name of the object that
            // does not exist.
            Err(error) => {
                return Err(map_stream_error(
                    &self.driver,
                    intent,
                    limits.read_only,
                    Bound::Internal,
                    error,
                ));
            }
        };
        // The user's statement is about to run, and it can itself change the
        // session state (`SET standard_conforming_strings = off`): the cursor
        // will decide whether the connection may go back.
        connection.taint();

        // The simple protocol gives the column list with the first row: its
        // cursor learns the schema there.
        let (schema, decodings) = match &source {
            Source::Prepared { statement, .. } => schema_for(statement.columns()),
            Source::Simple { .. } => (Arc::new(arrow::datatypes::Schema::empty()), Vec::new()),
        };
        let handle = StatementHandle::new();
        // A **child** of the caller's token: cancelling the caller cancels this
        // execution, but cancelling this one does not cancel the other tabs.
        let execution = cancel.child();
        let verdict = self.statements.register(handle, execution.clone());
        let cursor = cursor::spawn(
            StreamRequest {
                driver: self.driver.clone(),
                connection,
                source,
                bound,
                restore_context,
                opens_transaction,
                schema,
                decodings,
                limits,
                confirm_end,
                intent,
                backend_pid: pid,
                canceller: Arc::clone(&self.canceller),
                statements: Arc::clone(&self.statements),
                verdict,
                handle,
            },
            execution,
        )
        .await;
        Ok(Box::new(cursor))
    }

    /// Asks the server to interrupt an execution.
    ///
    /// The cancellation goes out from the execution's stream task, which holds
    /// its connection while sending: no other query can start on that server
    /// process before it has gone out. The call returns once the task has
    /// ruled.
    ///
    /// Cancelling an already finished statement is not an error, and sends
    /// nothing to the server.
    ///
    /// # Errors
    /// Those of `BackendCanceller::cancel_backend`, reported by the task.
    async fn cancel(&self, statement: StatementHandle) -> Result<()> {
        self.statements.cancel(&self.driver, statement).await
    }

    async fn preview_request(
        &self,
        path: &oxyn_catalog::CatalogPath,
        limit: u32,
        shape: &oxyn_core::PreviewShape,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        self.catalog
            .preview_request(path, limit, shape, cancel)
            .await
    }

    fn catalog(&self) -> &dyn CatalogProvider {
        &self.catalog
    }

    /// Measures a full round trip to the server.
    ///
    /// # Errors
    /// Any transport error. A session whose `ping` fails is considered lost.
    async fn ping(&self) -> Result<Duration> {
        let start = Instant::now();
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|error| map_connect_error(&error))?;
        connection
            .ping()
            .await
            .map_err(|error| map_connect_error(&error))?;
        Ok(start.elapsed())
    }

    /// Closes the pool.
    ///
    /// The cursors still alive each have their connection, marked to be closed:
    /// they stop on their own when dropped.
    ///
    /// # Errors
    /// None: `sqlx` closes on a best-effort basis and reports nothing. The
    /// signature stays fallible because the trait shares it with drivers that,
    /// for their part, have something to say.
    async fn close(self: Box<Self>) -> Result<()> {
        self.pool.close().await;
        Ok(())
    }
}

/// Why a statement goes through the simple protocol: `sqlx` cannot prepare it.
const UNRESOLVABLE_TYPE: &str = "a result type the client library cannot describe \
    (a multirange, or an internal type such as `pg_node_tree`)";
/// Why a statement goes through the simple protocol: the server cannot send one
/// of its result columns in binary.
const NO_BINARY_OUTPUT: &str = "a result type PostgreSQL cannot send in binary \
    (`aclitem`, `gtsvector`)";

/// The simple-protocol source, or the refusal when the request has bound
/// parameters: that protocol cannot carry them, and dropping them would run
/// another statement than the one asked for.
fn simple_source(unbound: bool, text: String, cause: &str) -> Result<Source> {
    if unbound {
        return Ok(Source::Simple {
            sql: AssertSqlSafe(text).into_sql_str(),
        });
    }
    Err(OxynError::NotSupported {
        capability: format!(
            "bound parameters with {cause}: cast that column to `text` in the query (ADR-0048)"
        ),
    })
}

/// Races a future against cancellation.
///
/// The token wins ties (`biased`): an already requested cancellation must not
/// wait for a slow operation to be willing to finish.
pub(crate) async fn race_cancel<T, F>(cancel: &CancelToken, operation: F) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(OxynError::Cancelled),
        issue = operation => issue,
    }
}

/// Encodes a query's bound parameters.
///
/// The "write" direction of the type mapping table. What is not in it is
/// **refused**, never converted by guesswork: binding an exact decimal as a
/// float would corrupt amounts, and binding a heterogeneous array would have no
/// element type to declare.
///
/// # Errors
/// [`OxynError::NotSupported`] for a type the driver cannot encode,
/// [`OxynError::Internal`] if `sqlx` refuses the encoding — which would be a
/// bug.
pub(crate) fn bind_params(params: &[ScalarValue]) -> Result<PgArguments> {
    let mut arguments = PgArguments::default();
    arguments.reserve(params.len(), 0);

    for (rank, value) in params.iter().enumerate() {
        let issue = match value {
            // A `NULL` is encoded by a length of −1, regardless of the type: the
            // type declared here never reaches the server, since the statement
            // is prepared separately and the server inferred its parameters'
            // types.
            ScalarValue::Null => arguments.add(Option::<&str>::None),
            ScalarValue::Bool(v) => arguments.add(*v),
            ScalarValue::Int64(v) => arguments.add(*v),
            ScalarValue::Float64(v) => arguments.add(*v),
            ScalarValue::Text(v) => arguments.add(v.as_str()),
            ScalarValue::Bytes(v) => arguments.add(v.as_slice()),
            ScalarValue::Uuid(v) => arguments.add(*v),
            ScalarValue::Date(v) => arguments.add(*v),
            ScalarValue::Time(v) => arguments.add(*v),
            ScalarValue::Timestamp(v) => arguments.add(*v),
            ScalarValue::TimestampNaive(v) => arguments.add(*v),
            ScalarValue::Json(v) => arguments.add(sqlx::types::Json(v)),
            ScalarValue::Interval {
                months,
                days,
                nanos,
            } => {
                // PostgreSQL stores intervals to the microsecond. Truncating the
                // remaining nanoseconds would silently change the value;
                // refusing says so.
                if nanos % 1_000 != 0 {
                    return Err(unsupported_param(
                        rank,
                        "an interval finer than a microsecond",
                    ));
                }
                let microseconds = nanos / 1_000;
                arguments.add(sqlx::postgres::types::PgInterval {
                    months: *months,
                    days: *days,
                    microseconds,
                })
            }
            // An exact decimal has no encoder in `sqlx` without `bigdecimal` or
            // `rust_decimal`, neither of which is in the dependency contract.
            // Binding it as text would work by chance on some columns and fail
            // on others.
            ScalarValue::Decimal(_) => {
                return Err(unsupported_param(rank, "an exact decimal value"));
            }
            // An empty array has no element type, and a heterogeneous array
            // does not have a single one.
            ScalarValue::Array(_) => {
                return Err(unsupported_param(rank, "an array"));
            }
        };
        // The `sqlx` encoder's message is dropped: it is composed from the
        // value, and this error is displayed then persisted (I-03). The rank
        // and the type are enough to locate the defect, which is a driver bug
        // and not user data.
        issue.map_err(|_| {
            OxynError::Internal(format!(
                "parameter ${} of type `{}` could not be encoded",
                rank.saturating_add(1),
                value.type_name()
            ))
        })?;
    }
    Ok(arguments)
}

/// The error of a parameter the driver cannot encode.
///
/// Names the parameter's **rank**, never its value
/// ([I-03](../../../CLAUDE.md#i-03)).
fn unsupported_param(rank: usize, what: &str) -> OxynError {
    OxynError::NotSupported {
        capability: format!(
            "binding {what} as parameter ${} — cast it in the query instead",
            rank.saturating_add(1)
        ),
    }
}

/// The pid of a connection's server process.
///
/// Extracted so that the cancellation connection and the execution connection
/// share exactly the same query.
pub(crate) async fn backend_pid(
    connection: &mut PgConnection,
) -> std::result::Result<i32, sqlx::Error> {
    let row = sqlx::query(SQL_BACKEND_PID).fetch_one(connection).await?;
    row.try_get::<i32, _>(0)
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, NaiveTime};
    use oxyn_core::{ConnectionConfig, QueryLanguage};

    use crate::cancel::SQL_CANCEL_BACKEND;
    use oxyn_driver::Credentials;
    use sqlx::postgres::PgPoolOptions;

    use super::*;
    use crate::driver::postgres_metadata;

    /// A session whose pool has never opened anything.
    ///
    /// `connect_lazy_with` establishes no connection: what is tested here is
    /// the statement the session **composes**, and composing it needs no
    /// server. The behavior against a real server is exercised by the
    /// `#[ignore]` tests of [`crate::integration`].
    fn offline_session() -> PostgresSession {
        let config = ConnectionConfig::new("trial", DriverId::postgres())
            .with_param("host", "127.0.0.1")
            .with_param("database", "shop")
            .with_param("user", "read_request");
        let spec = ConnectSpec::from_config(&postgres_metadata(), &config, &Credentials::new())
            .expect("complete configuration");
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy_with(spec.options().clone());
        PostgresSession::new(
            DriverId::postgres(),
            pool,
            spec,
            PostgresVariant::detect("PostgreSQL 17.11", "17.11", Vec::new()),
            "shop".to_owned(),
        )
    }

    /// Forces the retained context without going through the server.
    ///
    /// `set_context` checks the schema exists, hence needs a connection; this
    /// test is only about composing the statement.
    fn apply_context(session: &PostgresSession, context: Option<SessionContext>) {
        *session
            .context
            .lock()
            .expect("no other thread holds this lock") = context;
    }

    #[tokio::test]
    async fn without_declared_context_no_statement_is_set() {
        // The connection stays in the state the server opened it in: setting a
        // `SET` here would be exactly the invisible session state the contract
        // refuses.
        let session = offline_session();
        assert_eq!(session.context_statement(), None);
    }

    #[tokio::test]
    async fn a_declared_namespace_is_quoted_never_concatenated() {
        // I-10: a schema name comes from the catalog, hence from the server. The
        // doubled quote is what separates a `SET` from an arbitrary execution.
        let session = offline_session();

        apply_context(
            &session,
            Some(SessionContext::new(None, Some("analytics".to_owned()))),
        );
        assert_eq!(
            session.context_statement().as_deref(),
            Some(r#"SET search_path TO "analytics""#)
        );

        apply_context(
            &session,
            Some(SessionContext::new(
                None,
                Some(r#"oxyn_ctx"weird"#.to_owned()),
            )),
        );
        assert_eq!(
            session.context_statement().as_deref(),
            Some(r#"SET search_path TO "oxyn_ctx""weird""#)
        );
    }

    #[tokio::test]
    async fn transaction_control_is_refused_without_sending_anything() {
        // Without `TRANSACTIONS`, an accepted `ROLLBACK` would "succeed" on the
        // server without cancelling anything. The refusal goes out before any
        // borrow: the lazy pool never opens a single connection.
        let session = offline_session();
        assert!(!session.capabilities().contains(Capabilities::TRANSACTIONS));
        for text in [
            "BEGIN",
            "START TRANSACTION",
            "COMMIT",
            "END",
            "ROLLBACK",
            "ABORT",
            "SAVEPOINT s",
            "RELEASE SAVEPOINT s",
            "PREPARE TRANSACTION 'x'",
            "/* annuler */ rollback",
        ] {
            let exec_request = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), text);
            let error = match session.execute(exec_request, &CancelToken::new()).await {
                Ok(_) => panic!("`{text}` must be refused"),
                Err(error) => error,
            };
            assert!(
                matches!(error, OxynError::NotSupported { .. }),
                "`{text}`: {error:?}"
            );
            assert!(
                error
                    .to_string()
                    .contains("transactions are not supported in the console yet: each statement commits on its own"),
                "{error}"
            );
        }
        assert_eq!(
            session.pool.size(),
            0,
            "no connection must have been opened"
        );
    }

    #[tokio::test]
    async fn going_back_to_the_default_sets_a_statement_rather_than_nothing() {
        // Setting nothing would leave the pool connection on a previous
        // execution's `search_path`: one query out of two would resolve
        // elsewhere.
        let session = offline_session();
        apply_context(&session, Some(SessionContext::server_default()));
        assert_eq!(
            session.context_statement().as_deref(),
            Some("SET search_path TO DEFAULT")
        );
        // Setting the default and **undoing** it on return to the pool are two
        // distinct paths, carried by two distinct texts. Letting them diverge
        // would break nothing at compile time.
        assert_eq!(
            session.context_statement().as_deref(),
            Some(SQL_RESET_SEARCH_PATH)
        );
    }

    #[test]
    fn the_usual_scalar_types_bind() {
        let params = vec![
            ScalarValue::Null,
            ScalarValue::Bool(true),
            ScalarValue::Int64(42),
            ScalarValue::Float64(1.5),
            ScalarValue::Text("shop".to_owned()),
            ScalarValue::Bytes(vec![1, 2, 3]),
            ScalarValue::Uuid(uuid::Uuid::nil()),
            ScalarValue::Date(NaiveDate::from_ymd_opt(2026, 9, 5).expect("valid test date")),
            ScalarValue::Time(NaiveTime::from_hms_opt(14, 30, 0).expect("valid test time")),
        ];
        let arguments = bind_params(&params).expect("all these types encode");
        assert_eq!(arguments.len(), params.len());
    }

    #[test]
    fn a_microsecond_interval_binds() {
        let params = vec![ScalarValue::Interval {
            months: 1,
            days: 2,
            nanos: 3_000_000,
        }];
        assert!(bind_params(&params).is_ok());
    }

    #[test]
    fn an_interval_finer_than_a_microsecond_is_refused_not_truncated() {
        // Truncating would change the value without anything reporting it.
        let params = vec![ScalarValue::Interval {
            months: 0,
            days: 0,
            nanos: 1,
        }];
        let error = bind_params(&params).expect_err("refusal expected");
        assert!(matches!(error, OxynError::NotSupported { .. }), "{error:?}");
        assert!(error.is_user_error());
    }

    #[test]
    fn an_exact_decimal_is_refused_rather_than_bound_as_a_float() {
        // It is the loss the type table forbids in both directions.
        let params = vec![ScalarValue::Decimal("12345678901234567890.12".to_owned())];
        let error = bind_params(&params).expect_err("refusal expected");
        assert!(matches!(error, OxynError::NotSupported { .. }), "{error:?}");
        assert!(error.to_string().contains("$1"), "{error}");
    }

    #[test]
    fn a_refusal_message_never_repeats_the_value() {
        // I-03: a bound parameter is exactly what is not logged.
        let secret = "4111111111111111";
        let params = vec![ScalarValue::Decimal(secret.to_owned())];
        let error = bind_params(&params).expect_err("refusal expected");
        assert!(!error.to_string().contains(secret), "leak: {error}");
    }

    #[test]
    fn an_array_is_refused_for_lack_of_element_type() {
        let params = vec![ScalarValue::Array(vec![ScalarValue::Int64(1)])];
        let error = bind_params(&params).expect_err("refusal expected");
        assert!(matches!(error, OxynError::NotSupported { .. }), "{error:?}");
    }

    #[test]
    fn the_rank_of_the_faulty_parameter_is_the_one_the_user_reads() {
        // PostgreSQL numbers its placeholders from $1.
        let params = vec![
            ScalarValue::Int64(1),
            ScalarValue::Decimal("1.5".to_owned()),
        ];
        let error = bind_params(&params).expect_err("refusal expected");
        assert!(error.to_string().contains("$2"), "{error}");
    }

    #[test]
    fn sql_composed_by_the_driver_concatenates_nothing() {
        // I-10: these six literals are all the driver composes here. An
        // identifier or a value slipping in would be a visible `{}`.
        for compose in [
            SQL_BEGIN_READ_ONLY,
            SQL_ROLLBACK,
            SQL_RESET_SEARCH_PATH,
            SQL_RESET_AFTER_WRITE,
            SQL_BACKEND_PID,
            SQL_CANCEL_BACKEND,
        ] {
            assert!(!compose.contains('{'), "{compose}");
            assert!(!compose.contains("' ||"), "{compose}");
        }
        assert!(
            SQL_CANCEL_BACKEND.contains("$1"),
            "the pid must be a bound parameter"
        );
    }
}
