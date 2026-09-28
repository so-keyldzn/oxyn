//! PostgreSQL binary values rendered as the text the server itself would print.
//!
//! For built-in types whose binary layout is documented and stable, the driver
//! reproduces the type's `*_out` function: the user sees `192.168.0.1/24`, not
//! the eight bytes that encode it. Every layout and every output rule is
//! checked against PostgreSQL's sources in
//! [RESEARCH-NOTES](../../../docs/RESEARCH-NOTES.md#postgresql-binary-wire-formats--checked-on-2026-09-28);
//! none is written from memory ([I-12](../../../CLAUDE.md#i-12)).
//!
//! Every function takes the bytes of one value and appends its text to `out`.
//! The bytes come from the network: nothing here panics
//! ([I-09](../../../CLAUDE.md#i-09)), and an error is a `&'static str` that
//! never repeats the value ([I-03](../../../CLAUDE.md#i-03)).

pub(crate) mod array;
pub(crate) mod datetime;
pub(crate) mod float;
pub(crate) mod geometry;
pub(crate) mod network;
pub(crate) mod range;
pub(crate) mod record;
pub(crate) mod system;
pub(crate) mod textsearch;
pub(crate) mod value;

/// What a renderer returns: the text is in `out`, or a detail without the value.
pub(crate) type Rendered = Result<(), &'static str>;
