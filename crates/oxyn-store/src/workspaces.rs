//! La table `workspaces` : l'unité de persistance de l'état utilisateur.
//!
//! Un workspace groupe des connexions et des documents. Le supprimer emporte
//! les deux, **et rien d'autre** : le journal d'audit ne référence aucun
//! workspace, précisément pour qu'effacer le sien n'efface pas la trace de ce
//! qu'on y a fait ([`crate::journal`]).

use chrono::{DateTime, Utc};
use oxyn_core::WorkspaceId;
use rusqlite::{OptionalExtension, Row, params};

use crate::encoding::parse_id;
use crate::error::Result;
use crate::store::Store;

/// Un workspace, tel qu'il est persisté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// Identifiant interne, stable d'une ouverture à l'autre.
    pub id: WorkspaceId,
    /// Nom donné par l'utilisateur. C'est lui qui est affiché.
    pub name: String,
    /// Date de création.
    pub created_at: DateTime<Utc>,
    /// Date de la dernière modification enregistrée.
    pub updated_at: DateTime<Utc>,
}

impl Workspace {
    /// Construit un workspace neuf, non encore persisté.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        let maintenant = Utc::now();
        Self {
            id: WorkspaceId::new(),
            name: name.into(),
            created_at: maintenant,
            updated_at: maintenant,
        }
    }
}

/// Accès typé à la table `workspaces`.
#[derive(Debug)]
pub struct Workspaces<'a> {
    store: &'a Store,
}

impl<'a> Workspaces<'a> {
    /// Rattache l'accesseur à son `Store`.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Crée et persiste un workspace.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] si l'écriture échoue.
    pub fn create(&self, name: impl Into<String>) -> Result<Workspace> {
        let workspace = Workspace::new(name);
        self.save(&workspace)?;
        Ok(workspace)
    }

    /// Insère ou met à jour un workspace.
    ///
    /// `created_at` n'est jamais écrasé par une mise à jour : la date de
    /// création d'un workspace ne change pas parce qu'on l'a renommé.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] si l'écriture échoue.
    pub fn save(&self, workspace: &Workspace) -> Result<()> {
        self.store.with_connection(|conn| {
            conn.execute(
                "INSERT INTO workspaces (id, name, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                     name       = excluded.name,
                     updated_at = excluded.updated_at",
                params![
                    workspace.id.to_string(),
                    workspace.name,
                    workspace.created_at,
                    workspace.updated_at,
                ],
            )?;
            Ok(())
        })
    }

    /// Relit un workspace par son identifiant.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] ou [`crate::StoreError::Corrupted`].
    pub fn get(&self, id: WorkspaceId) -> Result<Option<Workspace>> {
        self.store.with_connection(|conn| {
            conn.query_row(
                "SELECT id, name, created_at, updated_at FROM workspaces WHERE id = ?1",
                params![id.to_string()],
                |row| Ok(depuis_ligne(row)),
            )
            .optional()?
            .transpose()
        })
    }

    /// Liste les workspaces, par nom.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] ou [`crate::StoreError::Corrupted`].
    pub fn list(&self) -> Result<Vec<Workspace>> {
        self.store.with_connection(|conn| {
            let mut requete = conn.prepare(
                "SELECT id, name, created_at, updated_at FROM workspaces ORDER BY name, id",
            )?;
            let lignes = requete.query_and_then([], depuis_ligne)?;
            lignes.collect()
        })
    }

    /// Supprime un workspace et, en cascade, ses connexions, leurs caches de
    /// catalogue et ses documents.
    ///
    /// **Le journal d'audit n'est pas touché** : ses lignes ne portent aucune
    /// clé étrangère vers un workspace. Supprimer un workspace n'efface pas ce
    /// qu'un agent y a fait.
    ///
    /// Rend `true` si une ligne a été supprimée.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] si la suppression échoue.
    pub fn delete(&self, id: WorkspaceId) -> Result<bool> {
        self.store.with_connection(|conn| {
            let touchees = conn.execute(
                "DELETE FROM workspaces WHERE id = ?1",
                params![id.to_string()],
            )?;
            Ok(touchees > 0)
        })
    }
}

/// Reconstruit un [`Workspace`] à partir d'une ligne.
fn depuis_ligne(row: &Row<'_>) -> Result<Workspace> {
    let id: String = row.get("id")?;
    Ok(Workspace {
        id: parse_id(&id, "workspaces.id")?,
        name: row.get("name")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aller_retour_d_un_workspace() {
        let store = Store::open_in_memory().expect("ouverture");
        let cree = store.workspaces().create("atelier").expect("création");

        let relu = store
            .workspaces()
            .get(cree.id)
            .expect("lecture")
            .expect("le workspace existe");
        assert_eq!(relu.id, cree.id);
        assert_eq!(relu.name, "atelier");
        // La sérialisation des horodatages passe par SQLite : la comparaison
        // vaut à la milliseconde près, pas à la nanoseconde.
        assert_eq!(
            relu.created_at.timestamp_millis(),
            cree.created_at.timestamp_millis()
        );
    }

    #[test]
    fn renommer_ne_change_pas_la_date_de_creation() {
        let store = Store::open_in_memory().expect("ouverture");
        let mut workspace = store.workspaces().create("avant").expect("création");
        let creation = workspace.created_at;

        workspace.name = "après".to_owned();
        workspace.updated_at = Utc::now();
        workspace.created_at = Utc::now(); // même si l'appelant se trompe
        store.workspaces().save(&workspace).expect("mise à jour");

        let relu = store
            .workspaces()
            .get(workspace.id)
            .expect("lecture")
            .expect("présent");
        assert_eq!(relu.name, "après");
        assert_eq!(
            relu.created_at.timestamp_millis(),
            creation.timestamp_millis()
        );
    }

    #[test]
    fn un_workspace_absent_rend_none() {
        let store = Store::open_in_memory().expect("ouverture");
        assert!(
            store
                .workspaces()
                .get(WorkspaceId::new())
                .expect("lecture")
                .is_none()
        );
        assert!(
            !store
                .workspaces()
                .delete(WorkspaceId::new())
                .expect("suppression")
        );
    }

    #[test]
    fn la_liste_est_ordonnee_par_nom() {
        let store = Store::open_in_memory().expect("ouverture");
        for nom in ["gamma", "alpha", "beta"] {
            store.workspaces().create(nom).expect("création");
        }
        let noms: Vec<String> = store
            .workspaces()
            .list()
            .expect("liste")
            .into_iter()
            .map(|w| w.name)
            .collect();
        assert_eq!(noms, ["alpha", "beta", "gamma"]);
    }
}
