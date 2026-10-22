//! An open session: execute, cancel, set the database, transactions.
//!
//! # Prepared first (ADR-0050 §2-3)
//!
//! Every statement of the user is prepared: `Conn::prep`, `exec_iter` without
//! parameters, `Conn::close`. The server's prepare **proves** the text is one
//! statement — the lexer runs without multi-statements, so anything after a
//! `;` is a 1064 — and values come back in the binary protocol. Only a 1295
//! (`ER_UNSUPPORTED_PS`: `START TRANSACTION`, `CREATE PROCEDURE`…) opens the
//! text protocol, and only for a request without bound parameters that
//! `oxyn-query` splits into exactly one statement. Every other prepare error
//! is the answer shown to the user.
//!
//! # A typed `USE` is refused, whoever types it
//!
//! `USE` is the one statement of that list that never reaches the server: the
//! session owns its database ([ADR-0019](../../../docs/adr/0019-contexte-de-session.md))
//! and reapplies the declared one after a reconnect, so a database selected
//! behind its back would be undone without a word. The refusal comes before the
//! prepare — MariaDB prepares `USE`, and a `COM_STMT_PREPARE` already reaches
//! the server — and points to the context selector, which is
//! [`Session::set_context`].
//!
//! # A typed `SET` of the time zone or a character set is refused too
//!
//! The setup of every connection pins `+00:00` and `utf8mb4`, and the decoders
//! tag a `TIMESTAMP` UTC and read text as `utf8mb4` on that basis. A typed
//! `SET time_zone` or `SET NAMES` would make them lie, and a reconnect would flip
//! the values back: it is refused, before the prepare, with a message that
//! points to `CONVERT_TZ`. What the lexical check covers, and the procedure and
//! dynamic-SQL cases it cannot, are in
//! [DRIVER-CONTRACT §7](../../../docs/DRIVER-CONTRACT.md) and in the private
//! `pinned` module.
//!
//! # Read-only is imposed by the server
//!
//! `SET SESSION TRANSACTION READ ONLY` before the statement, `READ WRITE`
//! after (ADR-0050 §9): the server refuses DDL and DML with 1792, in both
//! protocols. It does not apply to a transaction already running, so a
//! read-only execution inside an open transaction is refused rather than run
//! unbounded ([DRIVER-CONTRACT §5](../../../docs/DRIVER-CONTRACT.md)).

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use mysql_async::prelude::Queryable as _;
use oxyn_catalog::CatalogProvider;
use oxyn_core::{
    CancelToken, Capabilities, ExecRequest, OxynError, Result, SqlDialect, StatementHandle,
    StatementIntent, TransactionState,
};
use oxyn_driver::{Cursor, Session, SessionContext};

use crate::cancel::StatementRegistry;
use crate::catalog::MysqlCatalog;
use crate::connection::{Lease, Shared, race_cancel, use_statement};
use crate::cursor::{self, Source, StreamRequest};
use crate::error::{Bound, is_unsupported_prepare, map_stream_error};
use crate::params::bind_params;
use crate::pinned::changes_pinned_setting;
use crate::types::panics_in_binary_protocol;
use crate::variant::MysqlVariant;

/// Bounds the following transactions to reading. A literal.
pub(crate) const SQL_READ_ONLY: &str = "SET SESSION TRANSACTION READ ONLY";
/// Gives the session back its default access mode. A literal.
pub(crate) const SQL_READ_WRITE: &str = "SET SESSION TRANSACTION READ WRITE";
/// Opens a transaction for [`Session::begin`].
const SQL_BEGIN: &str = "START TRANSACTION";
/// Commits for [`Session::commit`].
const SQL_COMMIT: &str = "COMMIT";
/// Rolls back for [`Session::rollback`].
const SQL_ROLLBACK: &str = "ROLLBACK";

/// Why a read-only execution is refused inside an open transaction.
const READ_ONLY_IN_TRANSACTION: &str = "a read-only bound inside an open transaction \
    (MySQL applies `SET SESSION TRANSACTION READ ONLY` to the next transaction only): \
    commit or roll back first";

/// Why a typed `USE` is refused.
///
/// Fixed text: it never echoes the database the user typed
/// ([I-03](../../../CLAUDE.md#i-03)), and it names the way out.
const CONTEXT_REFUSED: &str = "a typed `USE`: the session keeps its database itself and would \
    reapply the declared one after a reconnect, so change it with the context selector";

/// Why a typed `SET` of the time zone or of a character set is refused.
///
/// Fixed text, for the same reason as [`CONTEXT_REFUSED`]: the value the user
/// typed is never echoed.
const PINNED_SETTING_REFUSED: &str = "a typed `SET` of the time zone or of the character sets: \
    the session pins `+00:00` and `utf8mb4` so that every TIMESTAMP and every text is read as \
    such, and applies them again after a reconnect; convert in the query instead \
    (`CONVERT_TZ`, `CONVERT(… USING …)`). A dump's `/*!… SET …*/` lines only save and \
    restore these settings: run the dump without them";

/// An open MySQL or MariaDB session.
#[derive(Debug)]
pub struct MysqlSession {
    shared: Arc<Shared>,
    statements: Arc<StatementRegistry>,
    variant: MysqlVariant,
    capabilities: Capabilities,
    catalog: MysqlCatalog,
    /// What the server confirmed, or `None` as long as nothing was declared.
    context: Mutex<Option<SessionContext>>,
}

impl MysqlSession {
    pub(crate) fn new(shared: Arc<Shared>, variant: MysqlVariant) -> Self {
        let capabilities = variant.capabilities();
        let catalog = MysqlCatalog::new(Arc::clone(&shared), variant.clone(), capabilities);
        Self {
            shared,
            statements: Arc::new(StatementRegistry::default()),
            variant,
            capabilities,
            catalog,
            context: Mutex::new(None),
        }
    }

    /// What the session learned from its server.
    #[must_use]
    pub const fn variant(&self) -> &MysqlVariant {
        &self.variant
    }

    /// Running executions. Diagnostics and tests.
    #[doc(hidden)]
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.statements.len()
    }

    /// Runs one of the driver's transaction statements.
    async fn transaction(
        &self,
        cancel: &CancelToken,
        sql: &'static str,
        intent: StatementIntent,
    ) -> Result<()> {
        self.capabilities.require(Capabilities::TRANSACTIONS)?;
        let mut lease = self.shared.lease(cancel).await?;
        self.shared
            .run(&mut lease, cancel, intent, |conn| {
                Box::pin(conn.query_drop(sql))
            })
            .await
    }

    /// Makes sure a read-only bound can be honored on this connection.
    async fn guard_read_only(&self, lease: &mut Lease, cancel: &CancelToken) -> Result<()> {
        if lease.transaction() != TransactionState::Idle {
            race_cancel(cancel, async {
                lease.refresh().await;
                Ok(())
            })
            .await?;
        }
        match lease.transaction() {
            TransactionState::Idle => Ok(()),
            _ => Err(OxynError::NotSupported {
                capability: READ_ONLY_IN_TRANSACTION.to_owned(),
            }),
        }
    }

    /// Puts the session back in read-write mode after a refusal that left no
    /// cursor to do it.
    async fn restore_read_write(&self, lease: &mut Lease) {
        let restored = match lease.conn() {
            Some(conn) => conn.query_drop(SQL_READ_WRITE).await.is_ok(),
            None => true,
        };
        if !restored {
            lease.discard();
        }
    }
}

/// Refuses a text `mysql_async` would rewrite before preparing it.
///
/// The library reads `:name` outside strings and comments as a named
/// parameter and replaces it with `?` — in the text of every prepared
/// statement, `prep` included. The server would then prepare a different
/// statement than the one written, a label such as `lbl:begin` among them.
/// Oxyn sends the user's SQL as is, or not at all.
fn refuse_rewritten(text: &str) -> Result<()> {
    let rewritten = match mysql_common::named_params::ParsedNamedParams::parse(text.as_bytes()) {
        Ok(parsed) => !parsed.params().is_empty(),
        Err(_) => true,
    };
    if rewritten {
        return Err(OxynError::NotSupported {
            capability: "`:name` outside a string or a comment, which the MySQL client library \
                         rewrites into a placeholder: add a space after the colon, or quote the \
                         name"
                .to_owned(),
        });
    }
    Ok(())
}

/// Refuses a typed `USE`, for every actor, before anything is sent.
///
/// The user's own `USE` would change the database behind the session's back
/// (ADR-0019): `Session::context` would go on naming the previous one, and
/// the first reconnect would silently select it again. The context is changed
/// by [`Session::set_context`], which composes its own `USE` through
/// [`use_statement`] and never comes through here. The agents' refusal is the
/// `PolicyGate`'s (I-01, I-07); this one is the driver's own, and applies to a
/// human as well.
///
/// It reads every fragment of the text, the body of an executable comment
/// (`/*!50000 USE x */`, `/*M!100000 USE x */`) included since the server runs
/// it, and does not wait for the server to object: MySQL answers 1064 to
/// `SELECT 1; USE x` at prepare, but MariaDB
/// prepares a lone `USE` (ADR-0050), and a refusal placed after the prepare
/// would come once the statement had already reached the server. Hence the
/// position of the check: before the prepare, before the lease. Letting the
/// `USE` run and reading `SELECT DATABASE()` back was not chosen: the declared
/// context is the session's, not what the driver discovers behind the user's
/// back.
fn refuse_context_change(text: &str) -> Result<()> {
    if oxyn_query::changes_context(text, SqlDialect::MySql) {
        return Err(OxynError::NotSupported {
            capability: CONTEXT_REFUSED.to_owned(),
        });
    }
    Ok(())
}

/// Refuses a typed `SET` of a setting the decoders rest on, before anything is
/// sent.
///
/// A `TIMESTAMP` is tagged UTC and text is read as `utf8mb4` because the setup
/// of every connection says so: a `SET time_zone` or `SET NAMES latin1` typed
/// by the user would make the tag and the decoding lie, and the next reconnect
/// would flip them back. What the check reads and what it cannot is in
/// [`crate::pinned`]. Placed with [`refuse_context_change`], for the same
/// reason: nothing has been sent, so there is no read-only state to put back.
fn refuse_pinned_setting(text: &str) -> Result<()> {
    if changes_pinned_setting(text) {
        return Err(OxynError::NotSupported {
            capability: PINNED_SETTING_REFUSED.to_owned(),
        });
    }
    Ok(())
}

/// Is the statement a `CALL`, whose extra result sets are the procedure's?
fn is_call(text: &str) -> bool {
    oxyn_query::words(text, SqlDialect::MySql)
        .first()
        .is_some_and(|word| word.text.eq_ignore_ascii_case("call"))
}

#[async_trait]
impl Session for MysqlSession {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Prepares, then starts the stream.
    ///
    /// Returns once the server answered the execution, whose columns are the
    /// schema: a prepare's columns may be missing or differ from the rows'
    /// (the stream task's `cursor::spawn` explains when).
    ///
    /// A typed `USE` is refused before the connection is touched: the context
    /// is changed with [`Session::set_context`], never from the user's SQL.
    /// So is a typed `SET` of the time zone or of a character set, which the
    /// decoders rest on.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] for a language other than MySQL's SQL, a
    /// parameter MySQL cannot receive, a text the library would rewrite, a
    /// typed `USE` or a typed `SET` of a pinned setting (any statement of the
    /// text), a read-only bound inside an
    /// open transaction, or a statement MySQL cannot prepare that carries
    /// parameters or several statements; [`OxynError::Cancelled`]; the
    /// server's refusal, classified.
    async fn execute(&self, request: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.capabilities.require_language(request.language)?;
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let ExecRequest {
            text,
            params,
            intent,
            limits,
            ..
        } = request;
        refuse_rewritten(&text)?;
        // Before the bound parameters and the lease: nothing has been sent, so
        // there is no read-only state to put back either.
        refuse_context_change(&text)?;
        refuse_pinned_setting(&text)?;
        let read_only = limits.read_only;
        let bound = Bound::of(&params);
        let unbound = params.is_empty();
        let params = bind_params(&params)?;

        let mut lease = self.shared.lease(cancel).await?;
        if limits.read_only {
            self.guard_read_only(&mut lease, cancel).await?;
            self.shared
                .run(&mut lease, cancel, StatementIntent::Read, |conn| {
                    Box::pin(conn.query_drop(SQL_READ_ONLY))
                })
                .await?;
        }

        let prepared = self.shared.prepare(&mut lease, cancel, text.clone()).await;
        let source = match prepared {
            Err(error) => {
                if limits.read_only {
                    self.restore_read_write(&mut lease).await;
                }
                return Err(error);
            }
            Ok(Ok(statement)) => {
                if let Some(column) = statement
                    .columns()
                    .iter()
                    .find(|column| panics_in_binary_protocol(column.column_type()))
                {
                    let name = column.name_str().into_owned();
                    lease.close_statement(statement).await;
                    if limits.read_only {
                        self.restore_read_write(&mut lease).await;
                    }
                    return Err(OxynError::Query(format!(
                        "column `{name}` has a type internal to the server that the client \
                         library cannot decode; cast it in the query"
                    )));
                }
                Source::Prepared {
                    call: is_call(&text),
                    statement,
                    params,
                }
            }
            Ok(Err(error)) if is_unsupported_prepare(&error) => {
                let statements = oxyn_query::split(&text, SqlDialect::MySql).len();
                if !unbound || statements != 1 {
                    if limits.read_only {
                        self.restore_read_write(&mut lease).await;
                    }
                    return Err(OxynError::NotSupported {
                        capability: if unbound {
                            format!(
                                "a statement MySQL cannot prepare, in a text Oxyn splits into \
                                 {statements} statements: send them one at a time"
                            )
                        } else {
                            "bound parameters with a statement MySQL cannot prepare".to_owned()
                        },
                    });
                }
                Source::Text { sql: text }
            }
            // `Bound::Internal`: the prepare carries the text only, never a
            // value, so the server's message quotes nothing of the caller's.
            Ok(Err(error)) => {
                if limits.read_only {
                    self.restore_read_write(&mut lease).await;
                }
                return Err(map_stream_error(
                    &self.shared.driver,
                    intent,
                    limits.read_only,
                    Bound::Internal,
                    &error,
                ));
            }
        };

        let handle = StatementHandle::new();
        // A child: cancelling the caller cancels this execution, not the
        // other way round.
        let execution = cancel.child();
        let verdict = self.statements.register(handle, execution.clone());
        let cursor = cursor::spawn(
            StreamRequest {
                driver: self.shared.driver.clone(),
                lease,
                source,
                bound,
                limits,
                intent,
                restore_read_write: read_only,
                killer: Arc::clone(&self.shared.killer),
                statements: Arc::clone(&self.statements),
                verdict,
                handle,
            },
            execution,
        )
        .await;
        Ok(Box::new(cursor))
    }

    /// Asks the server to stop an execution, through the task that holds the
    /// connection. Cancelling a finished statement is not an error.
    ///
    /// # Errors
    /// Those of the kill, reported by the task.
    async fn cancel(&self, statement: StatementHandle) -> Result<()> {
        self.statements.cancel(&self.shared.driver, statement).await
    }

    /// Selects the database unqualified names resolve in.
    ///
    /// MySQL has no catalog level: a context naming one is refused. The name is
    /// quoted by the driver ([I-10](../../../CLAUDE.md#i-10)); a database that
    /// does not exist is the server's refusal. Going back to the server
    /// default selects the form's database, and is refused when the form named
    /// none: MySQL cannot leave a database once selected.
    ///
    /// # Errors
    /// [`OxynError::Config`] for a catalog level, [`OxynError::NotSupported`]
    /// for leaving a database, [`OxynError::Cancelled`], or the server's error.
    async fn set_context(&self, context: &SessionContext, cancel: &CancelToken) -> Result<()> {
        if context.catalog().is_some() {
            return Err(OxynError::Config(
                "MySQL has no catalog level: a context names a database only".to_owned(),
            ));
        }
        let target = match context.namespace() {
            Some(database) => Some(database.to_owned()),
            None => self.shared.spec.database().map(str::to_owned),
        };
        match target {
            Some(database) => {
                let statement = use_statement(&database);
                let mut lease = self.shared.lease(cancel).await?;
                self.shared
                    .run(&mut lease, cancel, StatementIntent::Read, |conn| {
                        Box::pin(conn.query_drop(statement))
                    })
                    .await?;
                self.shared.set_database(Some(database));
            }
            None if self.shared.database().is_none() => {}
            None => {
                return Err(OxynError::NotSupported {
                    capability: "leaving the selected database: MySQL can only switch to \
                                 another one"
                        .to_owned(),
                });
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

    /// Measures a round trip with `COM_PING`, once the connection is free.
    ///
    /// # Errors
    /// Any transport error: a session whose ping fails is considered lost.
    async fn ping(&self) -> Result<Duration> {
        let cancel = CancelToken::new();
        let mut lease = self.shared.lease(&cancel).await?;
        let start = Instant::now();
        self.shared
            .run(&mut lease, &cancel, StatementIntent::Read, |conn| {
                Box::pin(conn.ping())
            })
            .await?;
        Ok(start.elapsed())
    }

    /// Closes the connection, once the operations before have ended.
    ///
    /// # Errors
    /// The transport error met while saying goodbye; the connection is
    /// released in every case.
    async fn close(self: Box<Self>) -> Result<()> {
        let mut lease = self.shared.lock().await;
        lease.close().await
    }

    async fn begin(&self, cancel: &CancelToken) -> Result<()> {
        // Opening modifies nothing: an interruption leaves nothing behind.
        self.transaction(cancel, SQL_BEGIN, StatementIntent::Read)
            .await
    }

    async fn commit(&self, cancel: &CancelToken) -> Result<()> {
        // An interrupted COMMIT leaves the effect unknown: ambiguous (I-13).
        self.transaction(cancel, SQL_COMMIT, StatementIntent::Write)
            .await
    }

    async fn rollback(&self, cancel: &CancelToken) -> Result<()> {
        self.transaction(cancel, SQL_ROLLBACK, StatementIntent::Write)
            .await
    }

    /// Reads the flag the server set on its last answer (ADR-0050 §8).
    ///
    /// Waits for the operations already submitted — the connection is taken
    /// in turn —, sends nothing, and says `Unknown` when the token fires, when
    /// the last statement failed without the status being read back, or when
    /// a transaction was lost with a closed connection.
    async fn transaction_state(&self, cancel: &CancelToken) -> TransactionState {
        tokio::select! {
            biased;
            () = cancel.cancelled() => TransactionState::Unknown,
            lease = self.shared.lock() => lease.transaction(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_the_library_would_rewrite_is_refused() {
        for text in [
            "SELECT * FROM t WHERE id = :id",
            "CREATE PROCEDURE p() lbl:begin SELECT 1; end",
            "SELECT ? , :named",
        ] {
            let error = refuse_rewritten(text).expect_err(text);
            assert!(matches!(error, OxynError::NotSupported { .. }), "{error:?}");
        }
        for text in [
            "SELECT ':id', \":id\", `a:b`",
            "SELECT @x := 1",
            "SELECT 1 -- :id\n",
            "SELECT /* :id */ 1",
            "SELECT * FROM t WHERE id = ?",
            "lbl: BEGIN END",
        ] {
            refuse_rewritten(text).expect(text);
        }
    }

    /// `execute` calls this before the bound parameters and the lease, which is
    /// the proof that nothing is sent: a session cannot be built without a live
    /// connection, so the check is tested on its own, and its position by
    /// reading `execute`.
    #[test]
    fn a_typed_use_is_refused_before_anything_is_sent() {
        for text in [
            "USE shop",
            "use shop;",
            "  USE\n  shop  ",
            "/* x */ USE shop",
            "-- pick it\nUSE shop",
            "SELECT 1; USE x",
            "USE shop; SELECT 1",
            // The quoting cannot hide the verb, nor smuggle a second statement
            // past the check.
            "use `a``; DROP`",
            "USE `oxyn ctx``db`",
            "USE \"a\"; DROP DATABASE b",
            // The server runs the body of an executable comment, and MariaDB
            // prepares a lone `USE`: the lexical rule has to see it.
            "/*!50000 USE shop */",
            "/*M!100000 USE shop */",
            "SELECT 1 /*!50000 ; USE shop */",
            "/*! USE `a``; DROP` */",
        ] {
            let error = refuse_context_change(text).expect_err(text);
            let OxynError::NotSupported { capability } = &error else {
                panic!("`{text}` must be a refusal of capability: {error:?}");
            };
            assert!(
                capability.contains("context selector"),
                "`{text}`: the message points to the selector: {capability}"
            );
            // I-03, I-10: the name the user typed is never echoed back.
            for name in ["shop", "DROP", "oxyn ctx", "`a``"] {
                assert!(
                    !capability.contains(name),
                    "`{text}`: the message echoes `{name}`: {capability}"
                );
            }
        }
        for text in [
            "SELECT 'use'",
            "SELECT `use` FROM t",
            "SELECT 1 /* USE x */",
            "-- USE x\nSELECT 1",
            "UPDATE t SET use_count = use_count + 1",
            "SELECT * FROM used",
            "CALL p()",
            "CREATE PROCEDURE p() BEGIN SELECT 1; END",
            "SET @use = 1",
            "",
            // Dumps carry executable comments all the time: only a `USE`
            // in one is refused.
            "/*!40101 SET NAMES utf8mb4 */",
            "SELECT /*!50000 DISTINCT */ 1",
            "SELECT * FROM t /*!50000 USE INDEX (i) */",
        ] {
            refuse_context_change(text).expect(text);
        }
    }

    #[test]
    fn a_typed_set_of_a_pinned_setting_is_refused_with_a_fixed_message() {
        for text in [
            "SET time_zone = 'Europe/Paris'",
            "SET NAMES latin1",
            "SELECT 1; SET character_set_results = latin1",
            "SET @@session.`time_zone` = 'Europe/Paris'",
            "/*!50000 SET NAMES latin1 */",
        ] {
            let error = refuse_pinned_setting(text).expect_err(text);
            let OxynError::NotSupported { capability } = &error else {
                panic!("`{text}` must be a refusal of capability: {error:?}");
            };
            assert!(capability.contains("CONVERT_TZ"), "{capability}");
            // I-03: what the user typed is never echoed back.
            for typed in ["Paris", "latin1"] {
                assert!(!capability.contains(typed), "`{text}`: {capability}");
            }
        }
        refuse_pinned_setting("SELECT @@session.time_zone").expect("a read is not refused");
        refuse_pinned_setting("SET @is_utc = @@session.time_zone = '+00:00'")
            .expect("a comparison on the right-hand side is a read");
    }

    #[test]
    fn a_call_is_recognized_whatever_its_case() {
        assert!(is_call("CALL p()"));
        assert!(is_call("  call p(1)"));
        assert!(is_call("/* x */ Call p"));
        assert!(!is_call("SELECT 'CALL'"));
    }

    #[test]
    fn composed_statements_are_literals() {
        for sql in [
            SQL_READ_ONLY,
            SQL_READ_WRITE,
            SQL_BEGIN,
            SQL_COMMIT,
            SQL_ROLLBACK,
        ] {
            assert!(!sql.contains('{'), "{sql}");
        }
    }
}
