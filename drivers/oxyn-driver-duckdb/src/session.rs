//! An open DuckDB session.

use std::time::Duration;

use async_trait::async_trait;
use duckdb::Connection;
use oxyn_catalog::CatalogProvider;
use oxyn_core::{
    CancelToken, Capabilities, ExecRequest, OxynError, Result, StatementHandle, TransactionState,
};
use oxyn_driver::{Cursor, Session};

use crate::catalog::DuckDbCatalog;
use crate::options::BatchLimits;

/// An open connection. Execution support is added on a dedicated carrier thread.
#[derive(Debug)]
pub(crate) struct DuckDbSession {
    connection: Connection,
    capabilities: Capabilities,
    catalog: DuckDbCatalog,
    limits: BatchLimits,
}

impl DuckDbSession {
    pub(crate) fn new(
        connection: Connection,
        capabilities: Capabilities,
        limits: BatchLimits,
    ) -> Self {
        Self {
            connection,
            capabilities,
            catalog: DuckDbCatalog::new(capabilities),
            limits,
        }
    }
}

#[async_trait]
impl Session for DuckDbSession {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    async fn execute(&self, request: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.capabilities.require_language(request.language)?;
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        Err(OxynError::NotSupported {
            capability: "DuckDB execution while the streaming carrier is incomplete".to_owned(),
        })
    }

    async fn cancel(&self, _statement: StatementHandle) -> Result<()> {
        Err(OxynError::NotSupported {
            capability: "SERVER_SIDE_CANCEL".to_owned(),
        })
    }

    fn catalog(&self) -> &dyn CatalogProvider {
        &self.catalog
    }

    async fn ping(&self) -> Result<Duration> {
        let _ = &self.connection;
        Err(OxynError::NotSupported {
            capability: "DuckDB execution while the streaming carrier is incomplete".to_owned(),
        })
    }

    async fn close(self: Box<Self>) -> Result<()> {
        Ok(())
    }

    async fn transaction_state(&self, _cancel: &CancelToken) -> TransactionState {
        TransactionState::Unknown
    }
}
