//! Namespace state that a dedicated SQLite preview can reproduce faithfully.

use std::fmt::Write as _;

use oxyn_catalog::path::{QuoteStyle, quote_identifier};
use oxyn_core::{CancelToken, OxynError, Result};
use rusqlite::Connection;

use crate::error::{self, Effect};
use crate::worker::WorkerHandle;

/// No Debug: attached file paths are connection parameters.
pub(crate) struct Context {
    attachments: Vec<(String, String)>,
    temporary: bool,
}

impl Context {
    pub fn read(connection: &Connection) -> Result<Self> {
        let map = |error| error::engine(error, Effect::ReadOnly);
        let temporary = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM temp.sqlite_schema WHERE type IN ('table', 'view'))",
                [],
                |row| row.get(0),
            )
            .map_err(map)?;
        let mut statement = connection.prepare("PRAGMA database_list").map_err(map)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })
            .map_err(map)?;
        let mut attachments = Vec::new();
        for row in rows {
            let (name, file) = row.map_err(map)?;
            if name != "main" && name != "temp" {
                attachments.push((name, file));
            }
        }
        Ok(Self {
            attachments,
            temporary,
        })
    }

    pub fn reproducible(&self) -> bool {
        !self.temporary && self.attachments.iter().all(|(_, file)| !file.is_empty())
    }

    pub async fn apply(self, worker: &WorkerHandle, cancel: &CancelToken) -> Result<()> {
        if !self.reproducible() {
            return Err(OxynError::NotSupported {
                capability:
                    "retained previews of session-local temporary or attached in-memory data".into(),
            });
        }
        worker
            .call(cancel, move |connection| {
                for (name, file) in self.attachments {
                    let sql = format!(
                        "ATTACH DATABASE ?1 AS {}",
                        quote_identifier(&name, QuoteStyle::Double)
                    );
                    // Explicit read-only URI: a removed attachment must not be
                    // replaced by an empty database while the old handle lives.
                    connection
                        .execute(&sql, [read_only_uri(&file)])
                        .map_err(|_| {
                            OxynError::Connection(
                                "an attached database could not be reopened for the preview".into(),
                            )
                        })?;
                }
                Ok(())
            })
            .await
    }
}

fn read_only_uri(path: &str) -> String {
    let mut uri = String::from("file:");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
            uri.push(char::from(byte));
        } else {
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    uri.push_str("?mode=ro");
    uri
}

#[cfg(test)]
mod tests {
    use super::read_only_uri;

    #[test]
    fn a_path_cannot_inject_uri_options() {
        assert_eq!(
            read_only_uri("/tmp/a?mode=rw#%.db"),
            "file:/tmp/a%3Fmode%3Drw%23%25.db?mode=ro"
        );
    }
}
