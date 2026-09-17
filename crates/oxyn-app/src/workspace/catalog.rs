//! Correlated catalog commands, with explicit retry and server cancellation.

use super::*;
use oxyn_core::CatalogRefreshScope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CatalogState {
    Initial,
    Loading,
    Ready,
    Empty,
    Cancelled,
    Error(String),
}

impl CatalogState {
    pub(super) fn message(&self) -> (&'static str, String) {
        match self {
            Self::Initial => (
                "Catalog",
                "Refresh to load the objects in this session.".into(),
            ),
            Self::Loading => (
                "Loading catalog…",
                "Cancel to stop the catalog request.".into(),
            ),
            Self::Cancelled => (
                "Catalog load cancelled",
                "Previously loaded metadata is preserved. Refresh explicitly to load again.".into(),
            ),
            Self::Ready => ("Catalog", String::new()),
            Self::Empty => (
                "Empty catalog",
                "The server returned no visible objects.".into(),
            ),
            Self::Error(message) => (
                "Catalog error",
                format!("{message}\nRefresh retries this request only when you choose it."),
            ),
        }
    }
}

/// Scope names remain structured data; only the driver may compose identifiers.
fn catalog_command(
    connection: oxyn_core::ConnectionId,
    scope: &CatalogScope,
    capabilities: Capabilities,
) -> Command {
    let scope = match scope {
        CatalogScope::Server => return Command::RefreshCatalog { connection },
        CatalogScope::Catalog(path) if capabilities.contains(Capabilities::SCHEMAS) => {
            CatalogRefreshScope::Namespaces {
                catalog: path.catalog().map(str::to_owned),
            }
        }
        CatalogScope::Catalog(path) | CatalogScope::Namespace(path) => {
            CatalogRefreshScope::Relations {
                catalog: path.catalog().map(str::to_owned),
                namespace: path.namespace().map(str::to_owned),
            }
        }
        CatalogScope::Definition(path) => CatalogRefreshScope::Definition {
            catalog: path.catalog().map(str::to_owned),
            namespace: path.namespace().map(str::to_owned),
            relation: path.relation().unwrap_or_default().to_owned(),
        },
        CatalogScope::IncomingForeignKeys(path) => CatalogRefreshScope::IncomingForeignKeys {
            catalog: path.catalog().map(str::to_owned),
            namespace: path.namespace().map(str::to_owned),
            relation: path.relation().unwrap_or_default().to_owned(),
        },
        CatalogScope::Constraints(path) => CatalogRefreshScope::Constraints {
            catalog: path.catalog().map(str::to_owned),
            namespace: path.namespace().map(str::to_owned),
            relation: path.relation().unwrap_or_default().to_owned(),
        },
        CatalogScope::Relation(path) => CatalogRefreshScope::Relation {
            catalog: path.catalog().map(str::to_owned),
            namespace: path.namespace().map(str::to_owned),
            relation: path.relation().unwrap_or_default().to_owned(),
        },
    };
    Command::RefreshCatalogScope { connection, scope }
}

impl Workspace {
    pub(super) fn catalog_supported(&self) -> bool {
        self.capabilities.intersects(
            Capabilities::SCHEMAS
                | Capabilities::TABLES
                | Capabilities::VIEWS
                | Capabilities::ROUTINES,
        )
    }

    pub(super) fn refresh_catalog(&mut self, scope: CatalogScope, cx: &mut Context<'_, Self>) {
        if !self.catalog_supported() {
            return;
        }
        if self.catalog_active.is_some() {
            if self.catalog_scope != scope {
                self.catalog_pending = Some(scope);
            }
            return;
        }
        let id = CommandId::new();
        let cancel = CancelToken::new();
        let command = catalog_command(self.connection, &scope, self.capabilities);
        // This fetch leaves now, so it sees every change already applied and
        // settles whatever automatic re-fetch was owed before it.
        self.catalog_refresh_owed = false;
        self.catalog_scope = scope;
        self.catalog_state = CatalogState::Loading;
        self.catalog_active = Some((id, cancel.clone()));
        let response = self.backend.dispatch(id, command, cancel);
        cx.spawn(async move |this, cx| {
            let result = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal("The executor stopped answering".into()))
            });
            let _ = this.update(cx, |this, cx| {
                if this.catalog_active.as_ref().map(|run| run.0) != Some(id) {
                    return;
                }
                this.catalog_active = None;
                if let Some(tree) = &this.catalog {
                    tree.update(cx, |tree, cx| {
                        tree.finish_loading(cx);
                        tree.on_catalog_updated(cx);
                    });
                }
                this.catalog_state = match result {
                    Ok(Outcome::CatalogRefreshed { connection, .. })
                        if connection == this.connection =>
                    {
                        this.catalog_scope = CatalogScope::Server;
                        if this.catalog_cache.try_read().is_some_and(|cache| {
                            cache.catalogs().next().is_none()
                                && cache.namespaces(None).next().is_none()
                                && cache.iter_relations().next().is_none()
                        }) {
                            CatalogState::Empty
                        } else {
                            CatalogState::Ready
                        }
                    }
                    Err(OxynError::Cancelled) => CatalogState::Cancelled,
                    Err(error) => CatalogState::Error(error.to_string()),
                    Ok(Outcome::Denied { reason, .. }) => CatalogState::Error(reason),
                    _ => CatalogState::Error("Unexpected catalog response".into()),
                };
                // A catalog that has just been read is the only witness able to
                // settle a location restored from the last session.
                this.settle_restored_location();
                // A refresh is the only moment the schemas on offer can change.
                // Re-reading the cache per frame would put the frame budget at
                // the mercy of a lock ([I-05](../../../CLAUDE.md#i-05)).
                this.sync_context_choices(cx);
                // Same reason for the sort menu: the columns it offers change
                // only when the catalog does, and reading the cache per frame
                // would put the frame budget at the mercy of a lock.
                this.refresh_preview_sort_choices(cx);
                if this.catalog_state == CatalogState::Ready
                    && this.panel == WorkspacePanel::Object
                    && this.object_tab == ObjectTab::Data
                    && this.preview_path.is_none()
                {
                    this.load_preview(cx);
                }
                // An expansion the user asked for comes first, and it is also a
                // fetch: it clears the automatic debt rather than adding to it.
                // Otherwise, a change that landed while this fetch was on the
                // wire is answered by exactly one more fetch, however many
                // changes arrived
                // ([ADR-0022](../../../docs/adr/0022-rafraichissement-automatique.md)).
                if let Some(scope) = this.catalog_pending.take() {
                    this.refresh_catalog(scope, cx);
                } else if std::mem::take(&mut this.catalog_refresh_owed) {
                    this.refresh_catalog(this.catalog_scope.clone(), cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Settles a restored location against a catalog that has just been read.
    ///
    /// The location is kept either way. A table dropped while Oxyn was closed
    /// is the server's news, not a reason to discard where the user was: the
    /// workspace says it once, and the breadcrumb keeps naming the object so
    /// the sentence has a subject.
    ///
    /// Silence is not absence: as long as the container holding the object has
    /// never been listed, nothing is claimed. Otherwise expanding one schema
    /// would announce the disappearance of every object in the others.
    pub(super) fn settle_restored_location(&mut self) {
        if !self.location_unconfirmed {
            return;
        }
        let Some(path) = self.selected_path.clone() else {
            self.location_unconfirmed = false;
            return;
        };
        // Never blocks the frame on the introspection lock ([I-05]); an unread
        // cache simply settles nothing this time.
        let Some(cache) = self.catalog_cache.try_read() else {
            return;
        };
        if cache.relation_summary(&path).is_some() {
            self.location_unconfirmed = false;
            return;
        }
        let container = path
            .parent()
            .map_or(CatalogScope::Server, |parent| CatalogScope::of(&parent));
        if !cache.freshness(&container).is_known() {
            return;
        }
        drop(cache);
        self.location_unconfirmed = false;
        self.console_notice = Some(format!(
            "{path} is no longer in this catalog. It is where you left off in the previous session; nothing was loaded."
        ));
    }

    pub(super) fn cancel_catalog(&mut self, cx: &mut Context<'_, Self>) {
        self.catalog_pending = None;
        // Cancelling means « stop reading »: an automatic fetch starting right
        // after would contradict the message the user is left with.
        self.catalog_refresh_owed = false;
        if let Some((_, cancel)) = &self.catalog_active {
            cancel.cancel();
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_relation_stays_data_and_never_becomes_sql() {
        let name = "odd\"; name.with.dots";
        let path = CatalogPath::for_relation(Some("db"), Some("public"), name).expect("valid path");
        let command = catalog_command(
            oxyn_core::ConnectionId::new(),
            &CatalogScope::Relation(path),
            Capabilities::SQL,
        );
        let Command::RefreshCatalogScope {
            scope: CatalogRefreshScope::Relation { relation, .. },
            ..
        } = command
        else {
            panic!("catalog command required")
        };
        assert_eq!(relation, name);
    }

    #[test]
    fn databases_without_schemas_request_relations_directly() {
        let scope = CatalogScope::Catalog(CatalogPath::for_catalog("db").expect("valid path"));
        let connection = oxyn_core::ConnectionId::new();
        assert!(matches!(
            catalog_command(connection, &scope, Capabilities::RELATIONAL),
            Command::RefreshCatalogScope {
                scope: CatalogRefreshScope::Relations {
                    namespace: None,
                    ..
                },
                ..
            }
        ));
        assert!(matches!(
            catalog_command(connection, &scope, Capabilities::SCHEMAS),
            Command::RefreshCatalogScope {
                scope: CatalogRefreshScope::Namespaces { .. },
                ..
            }
        ));
    }

    #[test]
    fn empty_catalog_does_not_claim_failure_or_loading() {
        assert_ne!(
            CatalogState::Empty.message(),
            CatalogState::Initial.message()
        );
        assert_ne!(
            CatalogState::Empty.message(),
            CatalogState::Error("server error".into()).message()
        );
    }
}
