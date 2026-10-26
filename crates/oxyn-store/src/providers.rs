//! The `ai_providers` table: the declared model providers.
//!
//! **Per machine, not per workspace** — the table has no `workspace_id`, and
//! that is the decision of [ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md):
//! an Ollama listening on the machine serves every workspace. What stays per
//! connection is the privacy tier.
//!
//! **No key is written here.** `secret_ref` designates an entry of the system
//! keychain, exactly as for connections (I-03). This is also where a base URL
//! carrying credentials, a query string or a fragment is refused: it is the
//! last point before the disk. A row written before that refusal existed is
//! cleaned the first time the table is listed.
//!
//! **No local/remote classification is persisted.** `Reach` has no column and
//! will not have one: a stored value would be yesterday's DNS answer applied to
//! today's send. Nothing in this module resolves a name; nothing opens a
//! network connection.
//!
//! Every method may block: they are never called from the UI thread
//! ([I-05](../../../CLAUDE.md#i-05)).

use chrono::Utc;
use oxyn_core::{AiProviderConfig, AiProviderKind, ProviderId};
use rusqlite::{Connection, Row, params};

use crate::error::{Result, StoreError};
use crate::store::Store;

/// Typed access to the `ai_providers` table.
#[derive(Debug)]
pub struct Providers<'a> {
    store: &'a Store,
}

impl<'a> Providers<'a> {
    /// Binds the accessor to its `Store`.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// The declared providers, by name.
    ///
    /// An **empty** list is Oxyn's default installation and not a degraded
    /// state: it decides whether the AI workspace exists (ADR-0006).
    ///
    /// The order is that of the name given by the user, tie-broken by the
    /// identifier: a settings list that reorders itself from one opening to
    /// the next reads as a defect.
    ///
    /// A row this binary cannot read back — a protocol family from a later
    /// version, a URL made unreadable in an SQLite editor — is **skipped**
    /// with a `warn`, not propagated as an error: a provider that could not be
    /// instantiated must not be offered, and a single odd row must not make
    /// the configuration screen unusable.
    ///
    /// **This read may write.** A base URL stored before a query string or a
    /// fragment was refused (a key pasted as `?key=…`, written in clear) is
    /// rewritten **without** them before anything is read, and the
    /// write-ahead log is truncated so that the old page image leaves the
    /// file too. Only `base_url` changes: the key reference, the declaration
    /// date and the modification date stay as they were, because cleaning is
    /// not an edit by the user and the query was never sent anywhere. The
    /// contract is the one of the whole module — it may block, so it is
    /// never called from the UI thread (I-05).
    ///
    /// # Errors
    /// [`StoreError::Sqlite`] if the cleaning or the read fails.
    pub fn list(&self) -> Result<Vec<AiProviderConfig>> {
        self.store.with_connection(|conn| {
            // Before the read, not after: `from_row` skips a row that fails
            // validation, which a stored query now does. Reading first would
            // make the provider vanish from the screen and leave its key on
            // the disk, instead of repairing it.
            drop_stored_queries(conn)?;
            let mut query = conn.prepare(
                "SELECT id, kind, label, base_url, model, secret_ref, created_at, updated_at
                 FROM ai_providers ORDER BY label, id",
            )?;
            let rows = query.query_and_then([], from_row)?;
            let mut providers = Vec::new();
            for row in rows {
                match row? {
                    Some(config) => providers.push(config),
                    None => continue,
                }
            }
            Ok(providers)
        })
    }

    /// Declares a provider, or replaces the declaration carrying its
    /// identifier.
    ///
    /// `created_at` is never overwritten: the declaration date does not change
    /// because a default model was corrected.
    ///
    /// The configuration is **revalidated** here, even if the executor
    /// already did it: it is the last place where a URL carrying a
    /// `user:password` pair, a query string or a fragment can be stopped
    /// before the disk, and a check that only lives upstream is a check a
    /// second caller will bypass.
    ///
    /// # Errors
    /// [`StoreError::Corrupted`] if the declaration is invalid — the message
    /// names the reason, never the value; [`StoreError::Sqlite`] if the write
    /// fails.
    pub fn save(&self, config: &AiProviderConfig) -> Result<()> {
        config.validate().map_err(|error| StoreError::Corrupted {
            field: "ai_providers",
            detail: error.to_string(),
        })?;
        let now = Utc::now();
        self.store.with_connection(|conn| {
            conn.execute(
                "INSERT INTO ai_providers
                     (id, kind, label, base_url, model, secret_ref, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET
                     kind       = excluded.kind,
                     label      = excluded.label,
                     base_url   = excluded.base_url,
                     model      = excluded.model,
                     secret_ref = excluded.secret_ref,
                     updated_at = excluded.updated_at",
                params![
                    config.id.as_str(),
                    config.kind.as_str(),
                    config.label,
                    config.base_url,
                    config.model,
                    config.secret_ref,
                    config.created_at,
                    now,
                ],
            )?;
            Ok(())
        })
    }

    /// Removes a declaration. Returns `true` if a row disappeared.
    ///
    /// Erases **nothing** else: documents written by an agent keep their
    /// provenance, which says where a text came from and not which provider is
    /// still declared. The referenced secret lives in the system keychain and
    /// is revoked there.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`] if the deletion fails.
    pub fn remove(&self, id: &ProviderId) -> Result<bool> {
        self.store.with_connection(|conn| {
            let erased = conn.execute(
                "DELETE FROM ai_providers WHERE id = ?1",
                params![id.as_str()],
            )?;
            Ok(erased > 0)
        })
    }
}

/// Rewrites every stored base URL without its query string and its fragment.
///
/// The cut is made **on the string**, at the first `?` or `#`, and not by
/// parsing then serializing the URL: a serialization lowercases the host,
/// adds a `/` to an empty path and drops a default port, so it would rewrite
/// the endpoint the user typed and could change what `same_endpoint_as` says
/// about it. Under WHATWG parsing a raw `?` or `#` always opens the query or
/// the fragment (it ends the authority and cannot belong to the path), so the
/// cut removes exactly them. `find` returns a character boundary: `get`
/// cannot fail, and the `let … else` is there so that no slice index ever
/// sits on a stored value (I-09).
///
/// `created_at`, `updated_at` and `secret_ref` are left alone. Every request
/// URL is built with a relative `Url::join`, which discards the base's query
/// and fragment, so the destination of a request — hence the endpoint the
/// keychain entry belongs to — is the same before and after.
///
/// All the rows go in one transaction: either every stored value is cleaned or
/// none is. A row that is still unreadable afterwards (`not a url?key=…`
/// becomes `not a url`) stays skipped by `from_row`, but its value is gone.
///
/// Never calls another `Store` method: the lock is not reentrant.
fn drop_stored_queries(conn: &Connection) -> Result<()> {
    let transaction = conn.unchecked_transaction()?;
    let stale: Vec<(String, String)> = transaction
        .prepare(
            "SELECT id, base_url FROM ai_providers
              WHERE instr(base_url, '?') > 0 OR instr(base_url, '#') > 0",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;

    let mut rewritten = Vec::new();
    for (id, base_url) in &stale {
        let Some(endpoint) = base_url
            .find(['?', '#'])
            .and_then(|end| base_url.get(..end))
        else {
            continue;
        };
        transaction.execute(
            "UPDATE ai_providers SET base_url = ?1 WHERE id = ?2",
            params![endpoint, id],
        )?;
        rewritten.push(id);
    }
    if rewritten.is_empty() {
        return Ok(());
    }
    transaction.commit()?;
    for id in rewritten {
        // The identifier only: the URL is exactly what must not be logged.
        tracing::info!(
            provider = %id,
            "ai_providers: query or fragment dropped from a stored base URL"
        );
    }
    truncate_wal(conn);
    Ok(())
}

/// Truncates the write-ahead log, so the page image that held the old value
/// leaves the file.
///
/// `secure_delete` zeroes the freed content in the main file, but the log
/// keeps the earlier page image until a checkpoint copies it over and
/// truncates the file. Best effort: the rewrite is already committed, and
/// another connection holding the file (a second Oxyn, a backup tool) only
/// postpones the truncation to the next checkpoint, so a refusal is logged
/// and never fails the listing. The pragma answers one row — busy, log
/// frames, checkpointed frames — also on an in-memory store.
fn truncate_wal(conn: &Connection) {
    match conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
        row.get::<_, i64>(0)
    }) {
        Ok(0) => {}
        Ok(_) => tracing::warn!(
            "ai_providers: write-ahead log not truncated after cleaning a base URL: \
             another connection holds the file"
        ),
        Err(error) => tracing::warn!(
            %error,
            "ai_providers: write-ahead log not truncated after cleaning a base URL"
        ),
    }
}

/// Rebuilds a declaration from a row.
///
/// Returns `Ok(None)` for a row the domain cannot read back: the distinction
/// with `Err` is what makes it possible to propagate a real SQLite failure
/// while skipping a row written by a later version. No message copies the
/// faulty value (I-03).
fn from_row(row: &Row<'_>) -> Result<Option<AiProviderConfig>> {
    let id: String = row.get("id")?;
    let kind: String = row.get("kind")?;
    let label: String = row.get("label")?;
    let base_url: String = row.get("base_url")?;
    let model: String = row.get("model")?;
    let secret_ref: Option<String> = row.get("secret_ref")?;

    let (Ok(id), Ok(kind)) = (id.parse::<ProviderId>(), kind.parse::<AiProviderKind>()) else {
        tracing::warn!("ai_providers: declaration ignored, unknown identifier or protocol family");
        return Ok(None);
    };

    let config = AiProviderConfig {
        id,
        kind,
        label,
        base_url,
        model,
        secret_ref,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    };
    if config.validate().is_err() {
        tracing::warn!(
            provider = %config.id,
            "ai_providers: declaration ignored, it no longer passes domain validation"
        );
        return Ok(None);
    }
    Ok(Some(config))
}

#[cfg(test)]
mod tests;
