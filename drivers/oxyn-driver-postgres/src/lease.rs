//! Borrowing a connection: it returns to the pool only once restored.
//!
//! # The defect this module closes
//!
//! An execution sets state **per connection**: `SET search_path` for a
//! console's context ([ADR-0019](../../../docs/adr/0019-contexte-de-session.md)),
//! `BEGIN READ ONLY` for read-only, and the user may type
//! `SET standard_conforming_strings = off` themselves. The cursor undoes that
//! state at the end of the stream. But an **abandoned future** — Esc during the
//! `SET` round trip, an agent that cuts its MCP connection — follows no error
//! path: `PoolConnection::drop` then returns the connection to the pool as is
//! (sqlx-core 0.9.0: a `ping` that resynchronizes the protocol, nothing more).
//! The next borrower inherits another console's `search_path`, and its
//! `DELETE FROM orders` targets a schema other than the one the user believes.
//!
//! # The rule
//!
//! [`Lease`] is **marked dirty before** any statement that can change the
//! connection's state, and **becomes clean again only after** a reset to the
//! default confirmed by the server. Dropped dirty — by a `?`, an abandoned
//! future, an interrupted task —, it closes the connection instead of
//! returning it.
//!
//! Why a guard and not `PoolConnection::close_on_drop` set in advance: in
//! sqlx-core 0.9.0 that flag cannot be cleared (`close_on_drop` only sets it to
//! `true`, with no inverse accessor). Setting it in advance would close **every**
//! connection that has carried a context, including those the cursor has reset
//! to the default. The guard sets it only at the last moment, in its `Drop`.

use std::ops::{Deref, DerefMut};

use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgConnection, Postgres};

/// A connection borrowed from the pool, which knows whether it may go back.
#[derive(Debug)]
pub(crate) struct Lease {
    connection: PoolConnection<Postgres>,
    /// The connection's state is no longer guaranteed to be the pool's.
    dirty: bool,
}

impl Lease {
    /// A fresh borrow: the connection is in the state the pool returned it in.
    pub(crate) const fn new(connection: PoolConnection<Postgres>) -> Self {
        Self {
            connection,
            dirty: false,
        }
    }

    /// To call **before** sending a statement that can change session state:
    /// from here on, an abandonment closes the connection.
    pub(crate) const fn taint(&mut self) {
        self.dirty = true;
    }

    /// To call **after** the server has confirmed the reset to the default.
    pub(crate) const fn restored(&mut self) {
        self.dirty = false;
    }

    /// The connection will not go back to the pool, whatever happens next.
    ///
    /// For a connection whose stream was abandoned midway: it may hold unread
    /// bytes, and no reset to the default can be trusted on it.
    pub(crate) fn discard(&mut self) {
        self.connection.close_on_drop();
    }
}

impl Deref for Lease {
    type Target = PgConnection;

    fn deref(&self) -> &PgConnection {
        &self.connection
    }
}

impl DerefMut for Lease {
    fn deref_mut(&mut self) -> &mut PgConnection {
        &mut self.connection
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if self.dirty {
            // Set here, just before `PoolConnection` is dropped in turn: sqlx
            // closes it instead of returning it.
            self.connection.close_on_drop();
        }
    }
}
