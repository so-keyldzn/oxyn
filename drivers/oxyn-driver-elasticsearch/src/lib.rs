//! Elasticsearch and OpenSearch through their shared JSON-over-HTTP protocol.
//!
//! One crate covers both products: the server identifies its distribution at
//! connection time, and every product-specific difference is a session
//! capability (ADR-0003).

mod api;
mod catalog;
mod classify;
mod cursor;
mod error;
mod request;
mod session;

#[cfg(test)]
mod integration;

pub mod driver;

pub use driver::{ElasticsearchDriver, elasticsearch_metadata};
