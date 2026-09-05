//! La table `catalog_cache` : l'introspection mise en cache, par connexion.
//!
//! L'introspection coûte des minutes sur un schéma à 20 000 objets
//! (ARCHITECTURE §6). Ce cache est ce qui rend l'arbre de catalogue consultable
//! **hors ligne**, et ce qui rend le contexte d'un agent constructible sans
//! aller-retour serveur.
//!
//! # Un blob JSON, pas un modèle relationnel
//!
//! Le contenu est stocké tel quel, en JSON, dans une seule colonne. La forme du
//! modèle de catalogue appartient à `oxyn-catalog` ; la figer ici en tables
//! obligerait à migrer l'état local de tous les utilisateurs à chaque évolution
//! de ce modèle, et `oxyn-store` ne dépend pas de `oxyn-catalog` (le sens des
//! dépendances du workspace l'interdit). Le JSON reste par ailleurs lisible sans
//! Oxyn (I-11).
//!
//! # La fraîcheur est rendue, jamais décidée ici
//!
//! [`CatalogCache::age`] rend l'âge de l'instantané ; c'est l'appelant qui sait
//! ce qui est trop vieux — quelques secondes après un DDL émis depuis Oxyn,
//! plusieurs jours pour un catalogue consulté hors ligne. Un seuil câblé ici
//! serait faux dans les deux cas.

use chrono::{DateTime, Utc};
use oxyn_core::ConnectionId;
use rusqlite::{OptionalExtension, Row, params};
use std::time::Duration;

use crate::encoding::parse_id;
use crate::error::Result;
use crate::store::Store;

/// Un instantané d'introspection, tel qu'il est mis en cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSnapshot {
    /// La connexion introspectée.
    pub connection: ConnectionId,
    /// Le catalogue, dans la forme que lui donne `oxyn-catalog`.
    pub payload: serde_json::Value,
    /// Quand l'introspection a eu lieu.
    pub refreshed_at: DateTime<Utc>,
    /// La version du serveur au moment de l'introspection, quand elle est
    /// connue. Un changement de version invalide le cache aussi sûrement qu'un
    /// DDL.
    pub server_version: Option<String>,
}

impl CatalogSnapshot {
    /// Construit un instantané daté de maintenant.
    #[must_use]
    pub fn new(connection: ConnectionId, payload: serde_json::Value) -> Self {
        Self {
            connection,
            payload,
            refreshed_at: Utc::now(),
            server_version: None,
        }
    }

    /// Note la version du serveur.
    #[must_use]
    pub fn with_server_version(mut self, version: impl Into<String>) -> Self {
        self.server_version = Some(version.into());
        self
    }
}

/// Accès typé à la table `catalog_cache`.
#[derive(Debug)]
pub struct CatalogCache<'a> {
    store: &'a Store,
}

impl<'a> CatalogCache<'a> {
    /// Rattache l'accesseur à son `Store`.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Écrit — ou remplace — l'instantané d'une connexion.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] si la connexion n'existe pas — la clé
    /// étrangère le refuse — ou si l'écriture échoue ;
    /// [`crate::StoreError::Json`] si le contenu n'est pas sérialisable.
    pub fn put(&self, snapshot: &CatalogSnapshot) -> Result<()> {
        let payload = serde_json::to_string(&snapshot.payload)?;
        self.store.with_connection(|conn| {
            conn.execute(
                "INSERT INTO catalog_cache (connection_id, payload, refreshed_at, server_version)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(connection_id) DO UPDATE SET
                     payload        = excluded.payload,
                     refreshed_at   = excluded.refreshed_at,
                     server_version = excluded.server_version",
                params![
                    snapshot.connection.to_string(),
                    payload,
                    snapshot.refreshed_at,
                    snapshot.server_version,
                ],
            )?;
            Ok(())
        })
    }

    /// Relit l'instantané d'une connexion.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`], [`crate::StoreError::Corrupted`] ou
    /// [`crate::StoreError::Json`].
    pub fn get(&self, connection: ConnectionId) -> Result<Option<CatalogSnapshot>> {
        self.store.with_connection(|conn| {
            conn.query_row(
                "SELECT connection_id, payload, refreshed_at, server_version
                 FROM catalog_cache WHERE connection_id = ?1",
                params![connection.to_string()],
                |row| Ok(depuis_ligne(row)),
            )
            .optional()?
            .transpose()
        })
    }

    /// Âge de l'instantané, mesuré depuis `now`.
    ///
    /// Rend `None` si aucun instantané n'existe **ou** si l'horodatage est
    /// postérieur à `now` — horloge remise à l'heure, fichier copié depuis une
    /// autre machine. Un âge négatif n'existe pas ; rendre `None` laisse
    /// l'appelant traiter le cache comme absent plutôt que comme frais.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] si la lecture échoue.
    pub fn age(&self, connection: ConnectionId, now: DateTime<Utc>) -> Result<Option<Duration>> {
        let refreshed_at: Option<DateTime<Utc>> = self.store.with_connection(|conn| {
            Ok(conn
                .query_row(
                    "SELECT refreshed_at FROM catalog_cache WHERE connection_id = ?1",
                    params![connection.to_string()],
                    |row| row.get(0),
                )
                .optional()?)
        })?;

        Ok(refreshed_at.and_then(|ts| now.signed_duration_since(ts).to_std().ok()))
    }

    /// Oublie l'instantané d'une connexion. Rend `true` s'il y en avait un.
    ///
    /// À appeler après tout DDL émis depuis Oxyn : le cache est alors
    /// périmé sans qu'aucun délai ne le dise (ARCHITECTURE §6).
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] si la suppression échoue.
    pub fn invalidate(&self, connection: ConnectionId) -> Result<bool> {
        self.store.with_connection(|conn| {
            let touchees = conn.execute(
                "DELETE FROM catalog_cache WHERE connection_id = ?1",
                params![connection.to_string()],
            )?;
            Ok(touchees > 0)
        })
    }

    /// Vide le cache de toutes les connexions et rend le nombre d'instantanés
    /// supprimés.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] si la suppression échoue.
    pub fn invalidate_all(&self) -> Result<usize> {
        self.store
            .with_connection(|conn| Ok(conn.execute("DELETE FROM catalog_cache", [])?))
    }
}

/// Reconstruit un [`CatalogSnapshot`] à partir d'une ligne.
fn depuis_ligne(row: &Row<'_>) -> Result<CatalogSnapshot> {
    let connection: String = row.get("connection_id")?;
    let payload: String = row.get("payload")?;

    Ok(CatalogSnapshot {
        connection: parse_id(&connection, "catalog_cache.connection_id")?,
        payload: serde_json::from_str(&payload)?,
        refreshed_at: row.get("refreshed_at")?,
        server_version: row.get("server_version")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::{ConnectionConfig, DriverId};
    use serde_json::json;

    fn store_avec_connexion() -> (Store, ConnectionId) {
        let store = Store::open_in_memory().expect("ouverture");
        let workspace = store.workspaces().create("atelier").expect("workspace");
        let config = ConnectionConfig::new("base client", DriverId::postgres());
        store
            .connections()
            .save(workspace.id, &config)
            .expect("connexion");
        (store, config.id)
    }

    #[test]
    fn aller_retour_d_un_instantane() {
        let (store, connexion) = store_avec_connexion();
        let instantane = CatalogSnapshot::new(
            connexion,
            json!({ "schemas": [{ "name": "public", "tables": ["clients", "commandes"] }] }),
        )
        .with_server_version("PostgreSQL 17.2");

        store.catalog_cache().put(&instantane).expect("écriture");
        let relu = store
            .catalog_cache()
            .get(connexion)
            .expect("lecture")
            .expect("présent");

        assert_eq!(relu.connection, connexion);
        assert_eq!(relu.payload, instantane.payload);
        assert_eq!(relu.server_version.as_deref(), Some("PostgreSQL 17.2"));
    }

    #[test]
    fn un_second_put_remplace_sans_doubler() {
        let (store, connexion) = store_avec_connexion();
        store
            .catalog_cache()
            .put(&CatalogSnapshot::new(connexion, json!({ "v": 1 })))
            .expect("écriture");
        store
            .catalog_cache()
            .put(&CatalogSnapshot::new(connexion, json!({ "v": 2 })))
            .expect("remplacement");

        let relu = store
            .catalog_cache()
            .get(connexion)
            .expect("lecture")
            .expect("présent");
        assert_eq!(relu.payload, json!({ "v": 2 }));
    }

    #[test]
    fn l_age_se_mesure_et_ne_devient_jamais_negatif() {
        let (store, connexion) = store_avec_connexion();
        let mut instantane = CatalogSnapshot::new(connexion, json!({}));
        instantane.refreshed_at = Utc::now() - chrono::Duration::seconds(120);
        store.catalog_cache().put(&instantane).expect("écriture");

        let age = store
            .catalog_cache()
            .age(connexion, Utc::now())
            .expect("âge")
            .expect("présent");
        assert!(age >= Duration::from_secs(119), "{age:?}");

        // Horloge remise en arrière : pas d'âge plutôt qu'un âge absurde.
        let dans_le_passe = Utc::now() - chrono::Duration::days(1);
        assert!(
            store
                .catalog_cache()
                .age(connexion, dans_le_passe)
                .expect("âge")
                .is_none()
        );
    }

    #[test]
    fn un_cache_absent_n_a_pas_d_age() {
        let store = Store::open_in_memory().expect("ouverture");
        assert!(
            store
                .catalog_cache()
                .age(ConnectionId::new(), Utc::now())
                .expect("âge")
                .is_none()
        );
    }

    #[test]
    fn invalider_efface_l_instantane() {
        let (store, connexion) = store_avec_connexion();
        store
            .catalog_cache()
            .put(&CatalogSnapshot::new(connexion, json!({})))
            .expect("écriture");

        assert!(store.catalog_cache().invalidate(connexion).expect("purge"));
        assert!(
            store
                .catalog_cache()
                .get(connexion)
                .expect("lecture")
                .is_none()
        );
        assert!(
            !store.catalog_cache().invalidate(connexion).expect("purge"),
            "invalider deux fois n'efface rien la seconde"
        );
    }

    #[test]
    fn supprimer_la_connexion_emporte_son_cache() {
        let (store, connexion) = store_avec_connexion();
        store
            .catalog_cache()
            .put(&CatalogSnapshot::new(connexion, json!({})))
            .expect("écriture");

        assert!(store.connections().delete(connexion).expect("suppression"));
        assert!(
            store
                .catalog_cache()
                .get(connexion)
                .expect("lecture")
                .is_none()
        );
    }

    #[test]
    fn un_cache_sans_connexion_est_refuse() {
        let store = Store::open_in_memory().expect("ouverture");
        let orphelin = CatalogSnapshot::new(ConnectionId::new(), json!({}));
        assert!(
            store.catalog_cache().put(&orphelin).is_err(),
            "la clé étrangère doit refuser une connexion inexistante"
        );
    }
}
