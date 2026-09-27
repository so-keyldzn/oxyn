//! Domain identifiers.
//!
//! Two families, and the difference is deliberate:
//!
//! * **instance** identifiers (connection, session, query, document…) are
//!   UUID v7. Version 7 is time-ordered: sorting `ResultId`s gives back the
//!   creation order without storing a timestamp alongside;
//! * the **driver** identifier is a stable string (`"postgres"`, `"sqlite"`).
//!   It appears in workspace files and in error messages; it must stay
//!   readable and identical from one version to the next.
//!
//! None of these identifiers is a secret, but a connection identifier is not
//! shown in a message meant for the user (I-03): it is the connection's *name*
//! that is shown.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Failure to parse an identifier.
///
/// The faulty text is **never** repeated in the message: a malformed connection
/// identifier is still a connection identifier, and it has no business in a log
/// or a dialog box (I-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid `{kind}`: {detail}")]
pub struct IdParseError {
    kind: &'static str,
    detail: &'static str,
}

impl IdParseError {
    /// Builds a parse error.
    #[must_use]
    pub const fn new(kind: &'static str, detail: &'static str) -> Self {
        Self { kind, detail }
    }

    /// Name of the expected identifier type.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        self.kind
    }

    /// Reason for the rejection, without repeating the faulty value.
    #[must_use]
    pub const fn detail(&self) -> &'static str {
        self.detail
    }
}

/// Declares a newtype identifier backed by a UUID v7.
///
/// This macro is not exported: it only avoids copying the same twenty lines
/// ten times in this module.
macro_rules! define_uuid_ids {
    ($( $(#[$meta:meta])* $name:ident ),* $(,)?) => {
        $(
            $(#[$meta])*
            #[derive(
                Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
                Serialize, Deserialize
            )]
            #[serde(transparent)]
            pub struct $name(Uuid);

            impl $name {
                /// Creates a fresh, time-ordered identifier (UUID v7).
                #[must_use]
                pub fn new() -> Self {
                    Self(Uuid::now_v7())
                }

                /// Adopts an existing UUID, read back from a workspace file
                /// for instance. No version is imposed: workspaces written
                /// before v7 was adopted stay readable.
                #[must_use]
                pub const fn from_uuid(uuid: Uuid) -> Self {
                    Self(uuid)
                }

                /// Borrows the underlying UUID.
                #[must_use]
                pub const fn as_uuid(&self) -> &Uuid {
                    &self.0
                }

                /// Returns the underlying UUID.
                #[must_use]
                pub const fn into_uuid(self) -> Uuid {
                    self.0
                }
            }

            impl Default for $name {
                /// Equivalent to [`Self::new`]: a **fresh** identifier.
                ///
                /// It is not a neutral value — two successive `default()` are
                /// not equal.
                fn default() -> Self {
                    Self::new()
                }
            }

            impl fmt::Display for $name {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    fmt::Display::fmt(&self.0, f)
                }
            }

            impl FromStr for $name {
                type Err = IdParseError;

                fn from_str(s: &str) -> Result<Self, Self::Err> {
                    Uuid::parse_str(s)
                        .map(Self)
                        .map_err(|_| IdParseError::new(
                            stringify!($name),
                            "not a UUID",
                        ))
                }
            }

            impl From<Uuid> for $name {
                fn from(uuid: Uuid) -> Self {
                    Self(uuid)
                }
            }

            impl From<$name> for Uuid {
                fn from(id: $name) -> Self {
                    id.0
                }
            }
        )*
    };
}

define_uuid_ids! {
    /// A configured connection, open or not.
    ///
    /// Not shown in the interface: we show
    /// [`ConnectionConfig::name`](crate::connection::ConnectionConfig::name).
    ConnectionId,

    /// A session open on a connection. Capabilities are evaluated at this
    /// level, not at the driver's (ADR-0003).
    SessionId,

    /// A statement execution in progress. It is the handle cancellation
    /// targets, and it must stay valid until the end of the result stream.
    StatementHandle,

    /// A result set, that is, a buffer of Arrow `RecordBatch`es.
    ResultId,

    /// A workspace document: saved query, note, draft.
    DocumentId,

    /// A workspace, that is, the unit of persistence of the user's state.
    WorkspaceId,

    /// An AI agent (the role: SQL, Schema, Performance…), stable from one
    /// session to the next.
    AgentId,

    /// A conversation with an agent. What an agent did is audited through
    /// this `AgentId` + `AgentSessionId` pair.
    AgentSessionId,

    /// A command submitted to the bus. Serves as a correlation key between the
    /// approval request, the decision and the audit log.
    CommandId,

    /// A launch of the application. Serves to tell a clean shutdown from a
    /// crash: the row it identifies carries its closing and its heartbeat
    /// ([ADR-0021](../../docs/adr/0021-marqueur-d-arret.md)).
    AppSessionId,

    /// A conversation thread with the assistant, as persisted and found again
    /// from one launch to the next.
    ///
    /// Distinct from [`AgentSessionId`], and the distinction matters: an agent
    /// session is **one** turn execution, and a thread chains several — a retry
    /// or a resumption opens a new one without changing thread. Confusing the
    /// two would restart the history at every retry.
    ConversationId,

    /// A workspace window, chosen by the desktop application and kept from one
    /// launch to the next with its layout
    /// ([ADR-0043](../../docs/adr/0043-multi-fenetre.md)).
    WindowId,
}

/// Stable identifier of a driver, per **protocol** and not per product.
///
/// `"postgres"` covers Redshift, TimescaleDB and pgvector; `"mysql"` covers
/// MariaDB (ADR-0003). Creating one identifier per product would be the first
/// step towards one crate per product.
///
/// The value is normalized: lowercase ASCII, digits, `-` and `_`, first
/// character a letter, 32 characters at most. This constraint is not cosmetic —
/// a driver identifier ends up in cache file names and in keychain keys.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DriverId(Arc<str>);

impl DriverId {
    /// PostgreSQL driver (and everything that speaks its protocol).
    pub const POSTGRES: &'static str = "postgres";
    /// MySQL driver (and MariaDB).
    pub const MYSQL: &'static str = "mysql";
    /// SQLite driver, in-process.
    pub const SQLITE: &'static str = "sqlite";

    /// Builds a driver identifier after validation.
    ///
    /// # Errors
    /// Returns [`IdParseError`] if the string is empty, too long, does not
    /// start with a lowercase letter, or contains a character outside
    /// `[a-z0-9_-]`.
    pub fn new(name: impl AsRef<str>) -> Result<Self, IdParseError> {
        let name = name.as_ref();
        if name.is_empty() {
            return Err(IdParseError::new("DriverId", "the string is empty"));
        }
        if name.len() > 32 {
            return Err(IdParseError::new("DriverId", "longer than 32 characters"));
        }
        if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
            return Err(IdParseError::new(
                "DriverId",
                "must start with an ASCII lowercase letter",
            ));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(IdParseError::new(
                "DriverId",
                "allowed characters: a-z, 0-9, `-`, `_`",
            ));
        }
        Ok(Self(Arc::from(name)))
    }

    /// Builds an identifier whose validity is guaranteed by the calling code.
    /// Reserved for this module's constants.
    fn known(name: &'static str) -> Self {
        debug_assert!(Self::new(name).is_ok(), "invalid driver constant");
        Self(Arc::from(name))
    }

    /// PostgreSQL driver identifier.
    #[must_use]
    pub fn postgres() -> Self {
        Self::known(Self::POSTGRES)
    }

    /// MySQL driver identifier.
    #[must_use]
    pub fn mysql() -> Self {
        Self::known(Self::MYSQL)
    }

    /// SQLite driver identifier.
    #[must_use]
    pub fn sqlite() -> Self {
        Self::known(Self::SQLITE)
    }

    /// Borrowed view of the identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DriverId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DriverId({:?})", self.as_str())
    }
}

impl fmt::Display for DriverId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for DriverId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for DriverId {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<String> for DriverId {
    type Error = IdParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<DriverId> for String {
    fn from(id: DriverId) -> Self {
        id.as_str().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_identifiers_are_distinct() {
        assert_ne!(ConnectionId::new(), ConnectionId::new());
        assert_ne!(SessionId::default(), SessionId::default());
    }

    #[test]
    fn v7_is_time_ordered() {
        let mut precedent = ResultId::new();
        for _ in 0..64 {
            let suivant = ResultId::new();
            assert!(precedent <= suivant, "UUID v7 must grow with time");
            precedent = suivant;
        }
    }

    #[test]
    fn text_round_trip() {
        let id = CommandId::new();
        let relu: CommandId = id.to_string().parse().expect("a rendered UUID reads back");
        assert_eq!(id, relu);
    }

    #[test]
    fn transparent_json_round_trip() {
        let id = DocumentId::new();
        let json = serde_json::to_string(&id).expect("serialization");
        assert_eq!(json, format!("\"{id}\""));
        let relu: DocumentId = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(id, relu);
    }

    #[test]
    fn a_failed_parse_does_not_copy_the_value() {
        let erreur = "production-connection-of-the-bank"
            .parse::<ConnectionId>()
            .expect_err("this is not a UUID");
        let message = erreur.to_string();
        assert!(!message.contains("bank"), "the faulty value leaked");
        assert_eq!(erreur.kind(), "ConnectionId");
    }

    #[test]
    fn driver_id_accepts_protocol_names() {
        for nom in [
            "postgres",
            "mysql",
            "sqlite",
            "elasticsearch",
            "mongo2",
            "sql-server",
        ] {
            assert!(DriverId::new(nom).is_ok(), "{nom} should be accepted");
        }
    }

    #[test]
    fn driver_id_refuses_what_is_not_normalized() {
        for nom in [
            "",
            "Postgres",
            "2fast",
            "post gres",
            "post/gres",
            "post.gres",
        ] {
            assert!(DriverId::new(nom).is_err(), "{nom:?} should be refused");
        }
        assert!(DriverId::new("a".repeat(33)).is_err());
    }

    #[test]
    fn driver_id_constants() {
        assert_eq!(DriverId::postgres().as_str(), "postgres");
        assert_eq!(DriverId::sqlite().to_string(), "sqlite");
        assert_eq!(DriverId::mysql(), DriverId::new("mysql").expect("valide"));
    }

    #[test]
    fn driver_id_json_validates_on_read_back() {
        let id = DriverId::postgres();
        let json = serde_json::to_string(&id).expect("serialization");
        assert_eq!(json, "\"postgres\"");
        assert!(
            serde_json::from_str::<DriverId>("\"POSTGRES\"").is_err(),
            "validation must also apply to deserialization"
        );
    }
}
