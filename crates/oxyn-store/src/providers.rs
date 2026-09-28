//! The `ai_providers` table: the declared model providers.
//!
//! **Per machine, not per workspace** — the table has no `workspace_id`, and
//! that is the decision of [ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md):
//! an Ollama listening on the machine serves every workspace. What stays per
//! connection is the privacy tier.
//!
//! **No key is written here.** `secret_ref` designates an entry of the system
//! keychain, exactly as for connections (I-03). This is also where a base URL
//! carrying credentials is refused: it is the last point before the disk.
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
use rusqlite::{Row, params};

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
    /// # Errors
    /// [`StoreError::Sqlite`] if the read fails.
    pub fn list(&self) -> Result<Vec<AiProviderConfig>> {
        self.store.with_connection(|conn| {
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
    /// `user:password` pair can be stopped before the disk, and a check that
    /// only lives upstream is a check a second caller will bypass.
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
mod tests {
    use super::*;

    fn declaration(id: &str, label: &str) -> AiProviderConfig {
        AiProviderConfig::new(
            ProviderId::new(id).expect("valid test identifier"),
            AiProviderKind::OpenAiCompatible,
            label,
            "http://localhost:11434/v1",
            "llama3.2",
        )
    }

    #[test]
    fn without_declaration_the_list_is_empty() {
        // ADR-0006: this is the default installation, and it is what the
        // interface queries to decide whether the AI workspace exists.
        let store = Store::open_in_memory().expect("open");
        assert!(store.providers().list().expect("list").is_empty());
    }

    #[test]
    fn a_provider_survives_the_deletion_of_every_workspace() {
        // The table has no `workspace_id`: that is ADR-0023's decision. This
        // test turns red if someone gives it a per-workspace scope.
        let store = Store::open_in_memory().expect("open");
        let workshop = store.workspaces().create("workshop").expect("workspace");
        store
            .providers()
            .save(&declaration("ollama", "Ollama"))
            .expect("declaration");

        assert!(store.workspaces().delete(workshop.id).expect("deletion"));

        let remaining = store.providers().list().expect("list");
        assert_eq!(remaining.len(), 1, "a provider serves every window");
        assert_eq!(remaining[0].label, "Ollama");
    }

    #[test]
    fn saving_the_same_identifier_twice_replaces_without_redating() {
        let store = Store::open_in_memory().expect("open");
        let origin = declaration("ollama", "Ollama");
        store.providers().save(&origin).expect("declaration");

        let mut corrected = declaration("ollama", "Laptop Ollama");
        corrected.model = "qwen2.5-coder".to_owned();
        // Even if the caller gets the creation date wrong.
        corrected.created_at = Utc::now();
        store.providers().save(&corrected).expect("correction");

        let list = store.providers().list().expect("list");
        assert_eq!(list.len(), 1, "the write is a replacement, not an insert");
        assert_eq!(list[0].label, "Laptop Ollama");
        assert_eq!(list[0].model, "qwen2.5-coder");
        assert_eq!(
            list[0].created_at.timestamp_millis(),
            origin.created_at.timestamp_millis(),
            "correcting a model does not redate the declaration"
        );
        assert!(list[0].updated_at >= list[0].created_at);
    }

    #[test]
    fn the_list_is_ordered_by_name() {
        // A settings list that reorders itself from one opening to the next
        // reads as a defect.
        let store = Store::open_in_memory().expect("open");
        for (id, label) in [
            ("openrouter", "Passerelle"),
            ("ollama", "Ollama"),
            ("lm-studio", "LM Studio"),
        ] {
            store
                .providers()
                .save(&declaration(id, label))
                .expect("declaration");
        }
        let labels: Vec<String> = store
            .providers()
            .list()
            .expect("list")
            .into_iter()
            .map(|config| config.label)
            .collect();
        assert_eq!(labels, ["LM Studio", "Ollama", "Passerelle"]);
    }

    #[test]
    fn no_url_carrying_credentials_reaches_the_disk() {
        // I-03: the last stopping point before the file. The refusal is an
        // error, not a silent cleanup.
        let store = Store::open_in_memory().expect("open");
        let mut declaration = declaration("openai", "OpenAI");
        declaration.base_url = "https://cle:motdepasse@api.example.com/v1".to_owned();

        let error = store
            .providers()
            .save(&declaration)
            .expect_err("a URL with credentials is not written");
        assert!(!error.to_string().contains("motdepasse"), "{error}");
        assert!(store.providers().list().expect("list").is_empty());
    }

    #[test]
    fn only_a_secret_reference_is_persisted() {
        let store = Store::open_in_memory().expect("open");
        store
            .providers()
            .save(&declaration("ollama", "Ollama").with_secret_ref("keychain://oxyn/ollama"))
            .expect("declaration");

        let columns: Vec<String> = store
            .with_connection(|conn| {
                let mut query =
                    conn.prepare("SELECT name FROM pragma_table_info('ai_providers')")?;
                let names = query.query_map([], |row| row.get(0))?;
                Ok(names.collect::<rusqlite::Result<Vec<String>>>()?)
            })
            .expect("table schema");
        assert!(
            !columns
                .iter()
                .any(|name| name == "api_key" || name == "reach"),
            "neither key nor classification in the database: {columns:?}"
        );

        let read_back = store.providers().list().expect("list").remove(0);
        assert_eq!(
            read_back.secret_ref.as_deref(),
            Some("keychain://oxyn/ollama")
        );
    }

    #[test]
    fn removing_a_declaration_is_idempotent() {
        let store = Store::open_in_memory().expect("open");
        let id = ProviderId::ollama();
        store
            .providers()
            .save(&declaration("ollama", "Ollama"))
            .expect("declaration");

        assert!(store.providers().remove(&id).expect("removal"));
        assert!(
            !store.providers().remove(&id).expect("second removal"),
            "removing what no longer exists is not an error"
        );
        assert!(store.providers().list().expect("list").is_empty());
    }

    #[test]
    fn an_unknown_family_is_skipped_without_bringing_down_the_list() {
        // The real case: a local state written by a later version of Oxyn. A
        // provider that could not be instantiated must not be offered, and the
        // configuration screen must stay usable.
        let store = Store::open_in_memory().expect("open");
        store
            .providers()
            .save(&declaration("ollama", "Ollama"))
            .expect("declaration");
        store
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO ai_providers
                     VALUES ('future','mistral','From the future','https://api.example.com','m',
                             NULL, ?1, ?1)",
                    params![Utc::now()],
                )?;
                Ok(())
            })
            .expect("row from a later version");

        let list = store.providers().list().expect("list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, ProviderId::ollama());
    }
}
