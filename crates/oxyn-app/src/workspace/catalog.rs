//! Correlated catalog commands, with explicit retry and server cancellation.

use super::*;
use oxyn_core::CatalogRefreshScope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CatalogState {
    Initial,
    Loading,
    Ready,
    Empty,
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
        if !self.catalog_supported() || self.catalog_active.is_some() {
            return;
        }
        let id = CommandId::new();
        let cancel = CancelToken::new();
        let command = catalog_command(self.connection, &scope, self.capabilities);
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
                    Err(OxynError::Cancelled) => CatalogState::Initial,
                    Err(error) => CatalogState::Error(error.to_string()),
                    Ok(Outcome::Denied { reason, .. }) => CatalogState::Error(reason),
                    _ => CatalogState::Error("Unexpected catalog response".into()),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn cancel_catalog(&mut self, cx: &mut Context<'_, Self>) {
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
