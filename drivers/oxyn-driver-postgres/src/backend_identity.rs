//! A replacement pool connection must not impersonate a lost temporary context.

use std::sync::atomic::{AtomicBool, Ordering};

use oxyn_core::{OxynError, Result};

#[derive(Debug, Default)]
pub(crate) struct BackendIdentity {
    opened: AtomicBool,
    lost: AtomicBool,
    local_state: AtomicBool,
}

impl BackendIdentity {
    pub fn opened(&self) {
        if self.opened.swap(true, Ordering::SeqCst) && self.local_state.load(Ordering::SeqCst) {
            self.lost.store(true, Ordering::SeqCst);
        }
    }

    pub fn preserve(&self) {
        self.local_state.store(true, Ordering::SeqCst);
    }

    pub fn check(&self) -> Result<()> {
        if self.lost.load(Ordering::SeqCst) {
            Err(OxynError::Config("this PostgreSQL session lost its physical connection and temporary state; reconnect before continuing".into()))
        } else {
            Ok(())
        }
    }
}
