//! Owned native database transfers: confined tools, files and process trees.
//!
//! Policy, audit and job persistence belong to `oxyn-exec`. This boundary
//! accepts only host-issued grants and structured options, never a shell or
//! an argument vector from a webview. No credentials are resolved here.
pub mod error;
pub mod postgres;
pub mod source;
pub use error::{Result, TransferError};
pub use source::TransferSource;
mod files;
mod process;
pub mod runner;
pub use runner::{PreparedTransfer, SourceBinding, TransferResult, TransferRunner};
