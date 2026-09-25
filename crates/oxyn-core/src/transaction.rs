//! Whether a session has a transaction block open.
//!
//! [ADR-0039](../../../docs/adr/0039-etat-de-transaction-d-une-session.md): a
//! capability says what a session **can** do; this says what it **is** doing,
//! as the engine reported it once its last operation ended — never as deduced
//! from the text submitted. SQLite rolls a transaction back on its own after
//! an interruption or some errors, and only the engine knows it did.

use serde::{Deserialize, Serialize};

/// The transaction state a session reported.
///
/// Carries no value from the database: the derived `Debug` leaks nothing
/// ([I-03](../../../CLAUDE.md#i-03)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TransactionState {
    /// No transaction block: each statement commits on its own.
    Idle,
    /// A transaction block is open on this session.
    Open,
    /// The session does not know, or does not say.
    ///
    /// Never shown as [`Idle`](Self::Idle): the interface shows what the
    /// session observed, not what one assumes (ADR-0019).
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_etats_voyagent_en_snake_case() {
        for (etat, attendu) in [
            (TransactionState::Idle, "\"idle\""),
            (TransactionState::Open, "\"open\""),
            (TransactionState::Unknown, "\"unknown\""),
        ] {
            let json = serde_json::to_string(&etat).expect("sérialisation");
            assert_eq!(json, attendu);
            let relu: TransactionState = serde_json::from_str(&json).expect("désérialisation");
            assert_eq!(relu, etat);
        }
    }
}
