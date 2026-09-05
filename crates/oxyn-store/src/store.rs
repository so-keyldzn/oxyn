//! L'ouverture de l'état local et l'accès sérialisé à SQLite.
//!
//! # Une seule connexion, derrière un verrou
//!
//! [`Store`] encapsule **une** `rusqlite::Connection` dans un
//! [`parking_lot::Mutex`]. Ce n'est pas un pis-aller de pool : l'état local
//! d'Oxyn est écrit par petites touches — une ligne d'historique, une ligne de
//! journal, un instantané de catalogue — et un pool ne ferait qu'ajouter des
//! écrivains concurrents à une base que SQLite sérialise de toute façon à
//! l'écriture. Le verrou rend cette sérialisation visible dans les types plutôt
//! que dans un `SQLITE_BUSY` intermittent.
//!
//! **Conséquence pour l'appelant : aucune méthode de cette crate ne doit être
//! appelée depuis le thread UI** (I-05). Elles sont synchrones et prennent un
//! verrou ; c'est à `oxyn-exec` de les porter sur le pool bloquant.
//!
//! # Les réglages d'ouverture, et ce qu'ils achètent
//!
//! | PRAGMA | Valeur | Pourquoi |
//! |---|---|---|
//! | `journal_mode` | `WAL` | un lecteur ne bloque plus un écrivain : la grille peut relire l'historique pendant qu'une commande s'inscrit au journal |
//! | `synchronous` | `NORMAL` | le compagnon usuel de WAL : durable au crash de processus, une transaction récente peut se perdre à la coupure de courant |
//! | `foreign_keys` | activé | SQLite les ignore **par défaut** ; sans ce réglage les cascades du schéma ne s'appliquent pas |
//! | `busy_timeout` | 5 s | une seconde instance d'Oxyn attend plutôt que d'échouer |
//!
//! `foreign_keys` est le piège classique : le schéma déclare des `REFERENCES`
//! qui ne font strictement rien tant que ce pragma n'est pas posé, **par
//! connexion**.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use parking_lot::Mutex;
use rusqlite::Connection;

use crate::catalog::CatalogCache;
use crate::connections::Connections;
use crate::documents::Documents;
use crate::error::{Result, StoreError};
use crate::history::History;
use crate::journal::Journal;
use crate::schema;
use crate::workspaces::Workspaces;

/// Nom du fichier de base sous le répertoire de données de l'OS.
pub const DATABASE_FILE_NAME: &str = "oxyn.sqlite3";

/// Attente maximale sur une base occupée par un autre processus.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Composants du chemin de données, au sens de `directories`.
const APP_QUALIFIER: &str = "dev";
/// Organisation, au sens de `directories`.
const APP_ORGANIZATION: &str = "keyldzn";
/// Application, au sens de `directories`.
const APP_NAME: &str = "oxyn";

/// L'état local persistant : workspaces, connexions, historique, journal
/// d'audit, cache de catalogue, documents.
///
/// Le type est `Send + Sync` : il se partage par `Arc` entre le bus
/// d'exécution et les tâches de fond.
///
/// # Exemple
///
/// ```
/// use oxyn_store::Store;
///
/// let store = Store::open_in_memory()?;
/// let atelier = store.workspaces().create("atelier")?;
/// assert_eq!(store.workspaces().list()?.len(), 1);
/// assert_eq!(atelier.name, "atelier");
/// # Ok::<(), oxyn_store::StoreError>(())
/// ```
pub struct Store {
    /// `None` pour une base en mémoire.
    path: Option<PathBuf>,
    conn: Mutex<Connection>,
}

impl Store {
    /// Ouvre l'état local à son emplacement standard sous le répertoire de
    /// données de l'utilisateur, en créant l'arborescence si besoin.
    ///
    /// # Erreurs
    /// * [`StoreError::DataDirUnavailable`] si le système n'expose pas de
    ///   répertoire de données ;
    /// * [`StoreError::Io`] si l'arborescence ne peut pas être créée ;
    /// * les erreurs de [`Store::open_at`].
    pub fn open_default() -> Result<Self> {
        Self::open_at(Self::default_path()?)
    }

    /// Ouvre — ou crée — l'état local à un chemin donné.
    ///
    /// Le répertoire parent est créé si nécessaire. Les migrations manquantes
    /// sont appliquées avant que la méthode ne rende la main : un `Store`
    /// existant a toujours un schéma à jour.
    ///
    /// # Erreurs
    /// [`StoreError::Io`], [`StoreError::Sqlite`],
    /// [`StoreError::SchemaTooRecent`] ou [`StoreError::Migration`].
    pub fn open_at(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)?;
        Self::from_connection(conn, Some(path))
    }

    /// Ouvre un état local en mémoire, migré et vide.
    ///
    /// Destiné aux tests — les siens comme ceux des crates qui en dépendent.
    /// Rien n'est écrit sur le disque, et `journal_mode` reste `memory` : WAL
    /// n'a pas de sens sans fichier.
    ///
    /// # Erreurs
    /// [`StoreError::Sqlite`] ou [`StoreError::Migration`].
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(conn, None)
    }

    /// Chemin standard du fichier d'état local pour cet utilisateur.
    ///
    /// # Erreurs
    /// [`StoreError::DataDirUnavailable`] si le système n'expose pas de
    /// répertoire de données — cas d'un environnement sans variable `HOME`.
    pub fn default_path() -> Result<PathBuf> {
        let dirs = directories::ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, APP_NAME)
            .ok_or(StoreError::DataDirUnavailable)?;
        Ok(dirs.data_dir().join(DATABASE_FILE_NAME))
    }

    /// Chemin du fichier, ou `None` pour une base en mémoire.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Version de schéma inscrite dans le fichier.
    ///
    /// # Erreurs
    /// [`StoreError::Sqlite`] si la table de suivi est illisible.
    pub fn schema_version(&self) -> Result<u32> {
        self.with_connection(schema::current_version)
    }

    /// Les workspaces.
    #[must_use]
    pub fn workspaces(&self) -> Workspaces<'_> {
        Workspaces::new(self)
    }

    /// Les connexions configurées.
    #[must_use]
    pub fn connections(&self) -> Connections<'_> {
        Connections::new(self)
    }

    /// L'historique des exécutions.
    #[must_use]
    pub fn history(&self) -> History<'_> {
        History::new(self)
    }

    /// La piste d'audit, en ajout seul.
    #[must_use]
    pub fn journal(&self) -> Journal<'_> {
        Journal::new(self)
    }

    /// Les documents du workspace.
    #[must_use]
    pub fn documents(&self) -> Documents<'_> {
        Documents::new(self)
    }

    /// Le cache d'introspection, par connexion.
    #[must_use]
    pub fn catalog_cache(&self) -> CatalogCache<'_> {
        CatalogCache::new(self)
    }

    /// Configure puis migre une connexion fraîche.
    fn from_connection(mut conn: Connection, path: Option<PathBuf>) -> Result<Self> {
        Self::configure(&conn)?;
        schema::migrate(&mut conn)?;
        Ok(Self {
            path,
            conn: Mutex::new(conn),
        })
    }

    /// Pose les quatre réglages d'ouverture décrits en tête de module.
    fn configure(conn: &Connection) -> Result<()> {
        conn.busy_timeout(BUSY_TIMEOUT)?;
        // Sur une base en mémoire, SQLite répond `memory` et ignore la demande :
        // c'est attendu, et ce n'est pas une erreur.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // Booléen — donc l'entier 1 — plutôt que la chaîne `ON` : le pragma
        // accepte les deux, l'entier ne dépend pas de la citation.
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(())
    }

    /// Exécute une lecture ou une écriture sous le verrou.
    ///
    /// Toutes les opérations de la crate tiennent en **une** instruction SQL :
    /// les cascades du schéma font le reste, et une seule instruction est
    /// atomique pour SQLite. Il n'y a donc pas d'assistant de transaction ici.
    /// Le jour où une opération en demandera plusieurs — l'import d'un fichier
    /// de workspace, par exemple — il faudra en écrire un, et non enchaîner les
    /// appels à cette méthode : le verrou est relâché entre deux.
    pub(crate) fn with_connection<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let guard = self.conn.lock();
        f(&guard)
    }
}

impl fmt::Debug for Store {
    /// Ne rend que le chemin : le contenu de l'état local — instructions,
    /// journal, catalogue — n'a rien à faire dans un `{store:?}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store")
            .field(
                "path",
                &self
                    .path
                    .as_deref()
                    .map_or("<mémoire>", |p| p.to_str().unwrap_or("<chemin non-UTF-8>")),
            )
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_base_en_memoire_est_migree_a_l_ouverture() {
        let store = Store::open_in_memory().expect("ouverture en mémoire");
        assert_eq!(
            store.schema_version().expect("version lisible"),
            schema::latest_version()
        );
        assert!(store.path().is_none());
    }

    #[test]
    fn le_store_se_partage_entre_threads() {
        // La documentation du type l'affirme ; le compilateur le vérifie.
        // `rusqlite::Connection` est `Send` mais pas `Sync` : c'est le `Mutex`
        // qui rend `Store` partageable, et le retirer casserait ce test.
        fn exige_send_sync<T: Send + Sync>() {}
        exige_send_sync::<Store>();
    }

    #[test]
    fn les_cles_etrangeres_sont_actives() {
        // SQLite les ignore par défaut : sans ce réglage, les cascades du
        // schéma seraient décoratives.
        let store = Store::open_in_memory().expect("ouverture");
        let actif: i64 = store
            .with_connection(
                |conn| Ok(conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?),
            )
            .expect("lecture du pragma");
        assert_eq!(actif, 1);
    }

    #[test]
    fn un_fichier_est_cree_avec_son_repertoire_et_se_rouvre() {
        let racine = tempfile::tempdir().expect("répertoire temporaire");
        let chemin = racine
            .path()
            .join("profond")
            .join("etat")
            .join("oxyn.sqlite3");

        {
            let store = Store::open_at(&chemin).expect("première ouverture");
            store
                .workspaces()
                .create("atelier")
                .expect("création de workspace");
            assert_eq!(store.path(), Some(chemin.as_path()));
        }

        let store = Store::open_at(&chemin).expect("réouverture");
        assert_eq!(
            store.schema_version().expect("version"),
            schema::latest_version(),
            "réouvrir ne doit pas rejouer les migrations"
        );
        assert_eq!(
            store.workspaces().list().expect("liste").len(),
            1,
            "l'état écrit doit survivre à la fermeture"
        );
    }

    #[test]
    fn un_fichier_est_en_wal() {
        let racine = tempfile::tempdir().expect("répertoire temporaire");
        let store = Store::open_at(racine.path().join("oxyn.sqlite3")).expect("ouverture");
        let mode: String = store
            .with_connection(
                |conn| Ok(conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?),
            )
            .expect("lecture du pragma");
        assert_eq!(mode, "wal");
    }

    #[test]
    fn le_debug_ne_montre_pas_le_contenu() {
        let store = Store::open_in_memory().expect("ouverture");
        let rendu = format!("{store:?}");
        assert!(rendu.contains("Store"));
        assert!(
            !rendu.contains("audit_journal"),
            "le Debug ne doit rien dire du contenu : {rendu}"
        );
    }

    #[test]
    fn une_migration_qui_echoue_ne_laisse_pas_le_schema_a_moitie_pose() {
        // La migration s'applique dans une transaction ; SQLite sait annuler du
        // DDL, donc un lot refusé ne laisse aucune table derrière lui.
        // La collision porte sur `documents`, créée en dernier : les cinq
        // tables précédentes sont donc bien posées avant l'échec, et c'est leur
        // disparition qui prouve l'annulation.
        let mut conn = Connection::open_in_memory().expect("base en mémoire");
        conn.execute_batch("CREATE TABLE documents (bloquante INTEGER);")
            .expect("table qui entrera en collision");

        let erreur = schema::migrate(&mut conn).expect_err("`documents` existe déjà");
        assert!(matches!(erreur, StoreError::Migration { version: 1, .. }));
        assert_eq!(
            schema::current_version(&conn).expect("version"),
            0,
            "aucune migration ne doit être inscrite"
        );

        let audit: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'audit_journal'",
                [],
                |row| row.get(0),
            )
            .expect("interrogation du schéma");
        assert_eq!(audit, 0, "le lot devait être annulé en entier");
    }

    #[test]
    fn une_base_neuve_n_a_aucun_workspace() {
        let store = Store::open_in_memory().expect("ouverture");
        assert!(
            store.workspaces().list().expect("liste").is_empty(),
            "migrer crée le schéma, pas des données"
        );
        assert_eq!(store.journal().count().expect("comptage"), 0);
    }
}
