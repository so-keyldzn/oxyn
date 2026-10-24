//! Elasticsearch and OpenSearch through their shared JSON-over-HTTP protocol.
//!
//! One crate covers both products: the server identifies its distribution at
//! connection time, and every product-specific difference is a session
//! capability (ADR-0003, ADR-0062).

mod aggs;
mod catalog;
mod cells;
mod classify;
mod columnar;
mod complete;
mod cursor;
mod decode;
mod error;
mod hits;
mod http;
mod mapping;
mod options;
mod preview;
mod request;
mod session;
mod stop;

#[cfg(test)]
mod integration;

pub mod driver;

pub use driver::{ElasticsearchDriver, elasticsearch_metadata};
