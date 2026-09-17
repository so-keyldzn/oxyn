//! La table `documents` : onglets et requêtes sauvegardés.
//!
//! Un document est ce que l'utilisateur voit comme un onglet : un titre, un
//! langage, un texte, et — facultativement — la connexion contre laquelle il
//! s'exécute. C'est le contenu le plus précieux du fichier d'état : le reste se
//! reconstruit, le travail de l'utilisateur non.
//!
//! Supprimer la connexion associée ne supprime **pas** le document : sa
//! référence passe à `NULL` (`ON DELETE SET NULL`). Perdre une requête écrite à
//! la main parce qu'on a nettoyé une liste de connexions serait une perte de
//! données déguisée en rangement.

use chrono::{DateTime, Utc};
use oxyn_core::{ConnectionId, DocumentId, Provenance, QueryLanguage, WorkspaceId};
use rusqlite::{OptionalExtension, Row, params};

use crate::encoding::{parse_id, parse_id_opt, tag_from_json, tag_to_json};
use crate::error::{Result, StoreError};
mod lifecycle;
pub use lifecycle::DocumentRevision;
mod listing;
use crate::store::Store;
pub use listing::{DocumentPage, DocumentSummary};

/// Un document du workspace.
#[derive(Clone, PartialEq, Eq)]
pub struct Document {
    /// Identifiant interne.
    pub id: DocumentId,
    /// Le workspace propriétaire.
    pub workspace: WorkspaceId,
    /// Titre affiché sur l'onglet.
    pub title: String,
    /// Langage du contenu, dialecte compris.
    pub language: QueryLanguage,
    /// Le texte, tel que l'utilisateur l'a écrit.
    pub content: String,
    /// La connexion contre laquelle il s'exécute, quand il y en a une.
    pub connection: Option<ConnectionId>,
    /// Date de création.
    pub created_at: DateTime<Utc>,
    /// Date de la dernière écriture.
    pub updated_at: DateTime<Utc>,
    /// Latest working-copy revision.
    pub revision: u64,
    /// Latest named-state decision, including a close/discard barrier.
    pub saved_revision: u64,
    /// A named copy exists independently of the draft.
    pub is_saved: bool,
    /// The document was left open locally; this never reconnects it.
    pub is_open: bool,
    /// Explicitly saved text, absent for an unnamed draft.
    pub saved_content: Option<String>,
    /// Explicitly saved title.
    pub saved_title: Option<String>,
    /// D'où vient ce texte. `None` veut dire « écrit par l'utilisateur », et
    /// c'est le cas de tous les documents antérieurs à la migration 7.
    ///
    /// Une provenance **ne s'efface pas** quand le contenu change : elle date
    /// l'origine du texte, pas la dernière frappe
    /// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
    pub provenance: Option<Provenance>,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("id", &self.id)
            .field("workspace", &self.workspace)
            .field("revision", &self.revision)
            .field("text_bytes", &self.content.len())
            .field("is_saved", &self.is_saved)
            .field("is_open", &self.is_open)
            // Montrable, et il faut qu'elle le soit : aucun de ses champs n'est
            // un secret, et c'est ce qu'on lit le jour où un document porte une
            // origine inattendue.
            .field("provenance", &self.provenance)
            .finish_non_exhaustive()
    }
}

impl Document {
    /// Construit un document neuf, non encore persisté.
    #[must_use]
    pub fn new(workspace: WorkspaceId, title: impl Into<String>, language: QueryLanguage) -> Self {
        let maintenant = Utc::now();
        Self {
            id: DocumentId::new(),
            workspace,
            title: title.into(),
            language,
            content: String::new(),
            connection: None,
            created_at: maintenant,
            updated_at: maintenant,
            revision: 0,
            saved_revision: 0,
            is_saved: true,
            is_open: false,
            saved_content: None,
            saved_title: None,
            provenance: None,
        }
    }

    /// Déclare que ce texte vient d'un agent.
    ///
    /// À n'appeler que sur le document **que l'agent a écrit** : la provenance
    /// n'est pas une étiquette de session, c'est l'origine d'un texte.
    #[must_use]
    pub fn with_provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = Some(provenance);
        self
    }

    /// Fixe le contenu.
    #[must_use]
    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = content.into();
        self
    }

    /// Rattache une connexion.
    #[must_use]
    pub fn on_connection(mut self, connection: ConnectionId) -> Self {
        self.connection = Some(connection);
        self
    }
}

/// Accès typé à la table `documents`.
#[derive(Debug)]
pub struct Documents<'a> {
    store: &'a Store,
}

impl<'a> Documents<'a> {
    /// Rattache l'accesseur à son `Store`.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Insère ou met à jour un document, et met `updated_at` à maintenant.
    ///
    /// `created_at` n'est jamais écrasé par une mise à jour.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] si le workspace ou la connexion n'existent
    /// pas — les clés étrangères le refusent — ou si l'écriture échoue ;
    /// [`crate::StoreError::Json`] si le langage n'est pas sérialisable.
    pub fn save(&self, document: &Document) -> Result<()> {
        let language = tag_to_json(&document.language)?;
        let provenance = provenance_to_column(document.provenance.as_ref())?;
        let maintenant = Utc::now();

        self.store.with_connection(|conn| {
            let changed = conn.execute(
                "INSERT INTO documents
                     (id, workspace_id, title, language, content, connection_id,
                      created_at, updated_at, saved_content, saved_title, provenance)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?5, ?3, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                     workspace_id  = excluded.workspace_id,
                     title         = excluded.title,
                     language      = excluded.language,
                     content       = excluded.content,
                     connection_id = excluded.connection_id,
                     updated_at    = excluded.updated_at,
                     saved_content = excluded.saved_content,
                     saved_title = excluded.saved_title,
                     -- Une provenance ne s'efface pas quand le contenu change :
                     -- elle date l'origine du texte, pas la dernière frappe
                     -- (ADR-0023). Un appelant qui n'en porte pas — parce qu'il
                     -- relit un document et le réécrit — ne peut donc pas
                     -- effacer celle qui existe.
                     provenance    = coalesce(excluded.provenance, documents.provenance)
                 WHERE documents.is_deleted=0 AND documents.revision=0 AND documents.workspace_id=excluded.workspace_id",
                params![
                    document.id.to_string(),
                    document.workspace.to_string(),
                    document.title,
                    language,
                    document.content,
                    document.connection.map(|id| id.to_string()),
                    document.created_at,
                    maintenant,
                    provenance,
                ],
            )?;
            if changed != 1 {
                return Err(StoreError::Corrupted {
                    field: "documents",
                    detail: "document is versioned, deleted, or belongs to another workspace".into(),
                });
            }
            Ok(())
        })
    }

    /// Relit un document.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] ou [`crate::StoreError::Corrupted`].
    pub fn get(&self, id: DocumentId) -> Result<Option<Document>> {
        self.get_cancellable(id, &oxyn_core::CancelToken::new())
    }

    /// Opens a bounded working copy with cancellation on the local connection.
    pub fn get_cancellable(
        &self,
        id: DocumentId,
        cancel: &oxyn_core::CancelToken,
    ) -> Result<Option<Document>> {
        self.store.with_connection_cancellable(cancel, |conn| {
            conn.query_row(
                &format!("{SELECT_COLONNES} WHERE id = ?1 AND is_deleted=0"),
                params![id.to_string()],
                |row| Ok(depuis_ligne(row)),
            )
            .optional()?
            .transpose()
        })
    }

    /// Liste les documents d'un workspace, le plus récemment modifié d'abord.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] ou [`crate::StoreError::Corrupted`].
    pub fn list(&self, workspace: WorkspaceId) -> Result<Vec<Document>> {
        self.store.with_connection(|conn| {
            let mut requete = conn.prepare(&format!(
                "{SELECT_COLONNES} WHERE workspace_id = ?1 AND is_deleted=0 ORDER BY updated_at DESC, id"
            ))?;
            requete
                .query_and_then(params![workspace.to_string()], depuis_ligne)?
                .collect()
        })
    }

    /// Supprime un document. Rend `true` si une ligne a été supprimée.
    ///
    /// # Erreurs
    /// [`crate::StoreError::Sqlite`] si la suppression échoue.
    pub fn delete(&self, id: DocumentId) -> Result<bool> {
        self.store.with_connection(|conn| {
            let touchees = conn.execute(
                "UPDATE documents SET is_deleted=1,is_open=0,is_saved=0,content='',
                 saved_content=NULL,saved_title=NULL,revision=revision+1,saved_revision=revision+1
                 WHERE id=?1 AND is_deleted=0 AND revision<9223372036854775807",
                params![id.to_string()],
            )?;
            Ok(touchees > 0)
        })
    }
}

/// Rend la provenance telle qu'elle s'écrit dans la colonne.
///
/// Le budget de 512 octets est vérifié **avant** l'écriture : le `CHECK` de
/// SQLite le refuserait de toute façon, mais son message parlerait d'une
/// contrainte plutôt que d'un nom de modèle trop long.
fn provenance_to_column(provenance: Option<&Provenance>) -> Result<Option<String>> {
    provenance
        .map(Provenance::to_json)
        .transpose()
        .map_err(|erreur| StoreError::Corrupted {
            field: "documents.provenance",
            detail: erreur.to_string(),
        })
}

/// La liste de colonnes, partagée par toutes les lectures.
const SELECT_COLONNES: &str = "SELECT id, workspace_id,
     CASE WHEN length(CAST(title AS BLOB))<=4096 THEN title ELSE NULL END AS title, language,
     CASE WHEN length(CAST(content AS BLOB))<=1048576 THEN content ELSE NULL END AS content,
     connection_id, created_at, updated_at, revision, saved_revision, is_saved, is_open,
     CASE WHEN length(CAST(saved_content AS BLOB))<=1048576 THEN saved_content ELSE NULL END AS saved_content,
     CASE WHEN length(CAST(saved_title AS BLOB))<=4096 THEN saved_title ELSE NULL END AS saved_title,
     provenance FROM documents";

/// Reconstruit un [`Document`] à partir d'une ligne.
fn depuis_ligne(row: &Row<'_>) -> Result<Document> {
    let id: String = row.get("id")?;
    let workspace: String = row.get("workspace_id")?;
    let language: String = row.get("language")?;

    let is_saved: bool = row.get("is_saved")?;
    let saved_content: Option<String> = row.get("saved_content")?;
    let saved_title: Option<String> = row.get("saved_title")?;
    if is_saved && (saved_content.is_none() || saved_title.is_none()) {
        return Err(StoreError::Corrupted {
            field: "documents.saved_content",
            detail: "saved copy is missing or exceeds the supported text or title limit".into(),
        });
    }
    Ok(Document {
        id: parse_id(&id, "documents.id")?,
        workspace: parse_id(&workspace, "documents.workspace_id")?,
        title: row
            .get::<_, Option<String>>("title")?
            .ok_or_else(|| StoreError::Corrupted {
                field: "documents.title",
                detail: "legacy title exceeds the 4 KiB read limit".into(),
            })?,
        language: tag_from_json(&language, "documents.language")?,
        content: row
            .get::<_, Option<String>>("content")?
            .ok_or_else(|| StoreError::Corrupted {
                field: "documents.content",
                detail: "query exceeds the 1 MiB editor limit".into(),
            })?,
        connection: parse_id_opt(row.get("connection_id")?, "documents.connection_id")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        revision: u64::try_from(row.get::<_, i64>("revision")?).map_err(|_| {
            StoreError::Corrupted {
                field: "documents.revision",
                detail: "negative revision".into(),
            }
        })?,
        saved_revision: u64::try_from(row.get::<_, i64>("saved_revision")?).map_err(|_| {
            StoreError::Corrupted {
                field: "documents.saved_revision",
                detail: "negative revision".into(),
            }
        })?,
        is_saved,
        is_open: row.get("is_open")?,
        saved_content,
        saved_title,
        // Une colonne `NULL` est le cas nominal : le document a été écrit par
        // l'utilisateur.
        //
        // Une valeur **illisible** est écartée avec un cri, et le document
        // s'ouvre quand même. C'est le contraire du sort des colonnes voisines,
        // et la différence est délibérée : `title` ou `content` illisibles
        // rendraient le document faux, alors qu'une provenance perdue le rend
        // seulement moins renseigné. Or ajouter une famille de fournisseur ne
        // demande **aucune migration** : un Oxyn plus récent peut écrire une
        // provenance que celui-ci ne sait pas lire, sur un schéma qu'il accepte,
        // et refuser ici rendrait le document inouvrable pour une métadonnée
        // qu'ADR-0023 qualifie lui-même de trace et non de scellé. C'est le
        // piège que `deny_unknown_fields` avait déjà tendu aux préférences
        // ([ADR-0013](../../../docs/adr/0013-preferences-workspace.md)).
        //
        // Le prix est assumé : la marque disparaît d'un document réécrit par ce
        // binaire-là. Le journal le dit ; le document reste ouvrable.
        provenance: row
            .get::<_, Option<String>>("provenance")?
            .and_then(|brut| match Provenance::from_json(&brut) {
                Ok(provenance) => Some(provenance),
                Err(erreur) => {
                    tracing::warn!(
                        error = %erreur,
                        "unreadable provenance dropped; the document opens without its mark"
                    );
                    None
                }
            }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::{
        AgentId, AgentSessionId, AiProviderKind, ConnectionConfig, DriverId, SqlDialect,
    };

    fn store_avec_workspace() -> (Store, WorkspaceId) {
        let store = Store::open_in_memory().expect("ouverture");
        let workspace = store.workspaces().create("atelier").expect("workspace");
        (store, workspace.id)
    }

    #[test]
    fn aller_retour_d_un_document() {
        let (store, workspace) = store_avec_workspace();
        let document = Document::new(
            workspace,
            "chiffre d'affaires",
            QueryLanguage::Sql(SqlDialect::DuckDb),
        )
        .with_content("SELECT sum(montant) FROM ventes");

        store.documents().save(&document).expect("écriture");
        let relu = store
            .documents()
            .get(document.id)
            .expect("lecture")
            .expect("présent");

        assert_eq!(relu.id, document.id);
        assert_eq!(relu.workspace, workspace);
        assert_eq!(relu.title, "chiffre d'affaires");
        assert_eq!(relu.language, QueryLanguage::Sql(SqlDialect::DuckDb));
        assert_eq!(relu.content, "SELECT sum(montant) FROM ventes");
        assert!(relu.connection.is_none());
    }

    #[test]
    fn reecrire_conserve_la_date_de_creation() {
        let (store, workspace) = store_avec_workspace();
        let mut document = Document::new(workspace, "brouillon", QueryLanguage::SQL);
        store.documents().save(&document).expect("écriture");
        let creation = store
            .documents()
            .get(document.id)
            .expect("lecture")
            .expect("présent")
            .created_at;

        document.content = "SELECT 1".to_owned();
        document.created_at = Utc::now(); // même si l'appelant se trompe
        store.documents().save(&document).expect("réécriture");

        let relu = store
            .documents()
            .get(document.id)
            .expect("lecture")
            .expect("présent");
        assert_eq!(relu.content, "SELECT 1");
        assert_eq!(
            relu.created_at.timestamp_millis(),
            creation.timestamp_millis()
        );
        assert!(relu.updated_at >= relu.created_at);
    }

    #[test]
    fn supprimer_la_connexion_ne_supprime_pas_le_document() {
        // Perdre une requête écrite à la main parce qu'on a nettoyé une liste
        // de connexions serait une perte de données déguisée en rangement.
        let (store, workspace) = store_avec_workspace();
        let config = ConnectionConfig::new("base client", DriverId::postgres());
        store
            .connections()
            .save(workspace, &config)
            .expect("connexion");

        let document = Document::new(workspace, "requête du lundi", QueryLanguage::SQL)
            .with_content("SELECT * FROM clients")
            .on_connection(config.id);
        store.documents().save(&document).expect("écriture");

        assert!(store.connections().delete(config.id).expect("suppression"));

        let relu = store
            .documents()
            .get(document.id)
            .expect("lecture")
            .expect("le document doit survivre");
        assert_eq!(relu.content, "SELECT * FROM clients");
        assert!(relu.connection.is_none(), "la référence passe à NULL");
    }

    #[test]
    fn supprimer_le_workspace_emporte_ses_documents() {
        let (store, workspace) = store_avec_workspace();
        let document = Document::new(workspace, "éphémère", QueryLanguage::SQL);
        store.documents().save(&document).expect("écriture");

        assert!(store.workspaces().delete(workspace).expect("suppression"));
        assert!(
            store
                .documents()
                .get(document.id)
                .expect("lecture")
                .is_none()
        );
    }

    #[test]
    fn un_document_sans_workspace_est_refuse() {
        let store = Store::open_in_memory().expect("ouverture");
        let orphelin = Document::new(WorkspaceId::new(), "orphelin", QueryLanguage::SQL);
        assert!(
            store.documents().save(&orphelin).is_err(),
            "la clé étrangère doit refuser un workspace inexistant"
        );
    }

    /// ADR-0023 : la provenance date l'origine du texte, pas la dernière
    /// frappe. Le test rougit si une réécriture l'efface.
    #[test]
    fn une_provenance_survit_a_la_reecriture_du_contenu() {
        let (store, workspace) = store_avec_workspace();
        let origine = Provenance::new(
            AgentId::new(),
            AgentSessionId::new(),
            AiProviderKind::OpenAiCompatible,
            "llama3.2",
        );
        let mut document = Document::new(workspace, "proposé par l'agent", QueryLanguage::SQL)
            .with_content("SELECT 1")
            .with_provenance(origine.clone());
        store.documents().save(&document).expect("écriture");

        // L'utilisateur reprend le texte à son compte, puis sauvegarde par le
        // chemin versionné de la bibliothèque.
        document.content = "SELECT 2 -- réécrit à la main".to_owned();
        store.documents().save(&document).expect("réécriture");
        store
            .documents()
            .update_query(
                workspace,
                &oxyn_core::QueryDocumentUpdate {
                    document: document.id,
                    title: "proposé par l'agent".to_owned(),
                    text: "SELECT 3".to_owned(),
                    language: QueryLanguage::SQL,
                    connection: None,
                    revision: 1,
                    expected_revision: None,
                    save_named: true,
                    is_open: true,
                    // Le chemin versionné n'en porte pas : c'est précisément
                    // ce que ce test éprouve.
                    provenance: None,
                },
                &oxyn_core::CancelToken::new(),
            )
            .expect("sauvegarde versionnée");

        let relu = store
            .documents()
            .get(document.id)
            .expect("lecture")
            .expect("présent");
        assert_eq!(relu.content, "SELECT 3");
        assert_eq!(
            relu.provenance.as_ref(),
            Some(&origine),
            "la provenance date l'origine du texte, pas la dernière frappe"
        );

        // Et un appelant qui relit puis réécrit sans porter la provenance ne
        // l'efface pas non plus : c'est le cas d'un chemin qui l'ignore.
        let mut sans = relu;
        sans.provenance = None;
        sans.revision = 0;
        store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE documents SET revision=0, saved_revision=0 WHERE id=?1",
                    params![sans.id.to_string()],
                )?;
                Ok(())
            })
            .expect("retour à une copie non versionnée");
        store.documents().save(&sans).expect("réécriture aveugle");
        assert_eq!(
            store
                .documents()
                .get(sans.id)
                .expect("lecture")
                .expect("présent")
                .provenance,
            Some(origine)
        );
    }

    #[test]
    fn un_document_ecrit_par_l_utilisateur_n_a_pas_de_provenance() {
        // `NULL` veut dire « écrit par l'utilisateur », et c'est vrai.
        let (store, workspace) = store_avec_workspace();
        let document = Document::new(workspace, "à la main", QueryLanguage::SQL)
            .with_content("SELECT * FROM ventes");
        store.documents().save(&document).expect("écriture");

        let relu = store
            .documents()
            .get(document.id)
            .expect("lecture")
            .expect("présent");
        assert_eq!(relu.provenance, None);
    }

    #[test]
    fn une_provenance_illisible_nempeche_pas_douvrir_le_document_ni_ne_seffface() {
        // Ce test disait l'inverse : une provenance illisible faisait échouer la
        // lecture. La relecture de sécurité a montré le prix de ce choix —
        // ajouter une famille de fournisseur ne demande **aucune migration**,
        // donc un Oxyn plus récent peut écrire, sur un schéma que celui-ci
        // accepte, une provenance qu'il ne sait pas lire. Le document devenait
        // inouvrable pour une métadonnée qu'ADR-0023 qualifie de trace et non
        // de scellé — le piège que `deny_unknown_fields` avait déjà tendu aux
        // préférences (ADR-0013).
        //
        // Ce qui compte est tenu autrement : **rien n'est détruit**. La valeur
        // reste en base, protégée par le `coalesce` de l'écriture, et un binaire
        // qui saura la lire la retrouvera.
        let (store, workspace) = store_avec_workspace();
        let document = Document::new(
            workspace,
            "écrit par un Oxyn plus récent",
            QueryLanguage::SQL,
        );
        store.documents().save(&document).expect("écriture");
        store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE documents SET provenance='{\"agent\":\"pas-un-uuid\"}' WHERE id=?1",
                    params![document.id.to_string()],
                )?;
                Ok(())
            })
            .expect("colonne écrite par une version que ce binaire ne connaît pas");

        let relu = store
            .documents()
            .get(document.id)
            .expect("le document s'ouvre malgré une marque illisible")
            .expect("présent");
        assert!(
            relu.provenance.is_none(),
            "ce binaire ne sait pas la lire, il ne prétend donc rien"
        );

        // Et il ne l'écrase pas non plus en réécrivant le document.
        store.documents().save(&relu).expect("réécriture");
        let brut: Option<String> = store
            .with_connection(|conn| {
                Ok(conn.query_row(
                    "SELECT provenance FROM documents WHERE id=?1",
                    params![document.id.to_string()],
                    |row| row.get(0),
                )?)
            })
            .expect("relecture brute");
        assert_eq!(
            brut.as_deref(),
            Some("{\"agent\":\"pas-un-uuid\"}"),
            "la valeur reste en base : une version qui saura la lire la retrouvera"
        );
    }

    #[test]
    fn la_liste_est_bornee_au_workspace() {
        let store = Store::open_in_memory().expect("ouverture");
        let a = store.workspaces().create("a").expect("workspace");
        let b = store.workspaces().create("b").expect("workspace");

        store
            .documents()
            .save(&Document::new(a.id, "dans a", QueryLanguage::SQL))
            .expect("écriture");
        store
            .documents()
            .save(&Document::new(b.id, "dans b", QueryLanguage::SQL))
            .expect("écriture");

        let dans_a = store.documents().list(a.id).expect("liste");
        assert_eq!(dans_a.len(), 1);
        assert_eq!(dans_a[0].title, "dans a");
    }
}

#[cfg(test)]
mod library_tests;
