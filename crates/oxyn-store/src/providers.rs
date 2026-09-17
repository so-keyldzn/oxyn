//! La table `ai_providers` : les fournisseurs de modèles déclarés.
//!
//! **Par machine, pas par workspace** — la table n'a pas de `workspace_id`, et
//! c'est la décision d'[ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md) :
//! un Ollama qui écoute sur la machine sert tous les workspaces. Ce qui reste
//! par connexion, c'est le niveau de confidentialité.
//!
//! **Aucune clé n'est écrite ici.** `secret_ref` désigne une entrée du
//! trousseau du système, exactement comme pour les connexions (I-03). C'est
//! aussi ici qu'une URL de base porteuse d'identifiants est refusée : c'est le
//! dernier point avant le disque.
//!
//! **Aucun classement local/distant n'est persisté.** `Reach` n'a pas de
//! colonne et n'en aura pas : une valeur en base serait une réponse DNS d'hier
//! appliquée à un envoi d'aujourd'hui. Rien dans ce module ne résout un nom ;
//! rien n'ouvre de connexion réseau.
//!
//! Toutes les méthodes peuvent bloquer : elles ne s'appellent jamais depuis le
//! thread d'interface ([I-05](../../../CLAUDE.md#i-05)).

use chrono::Utc;
use oxyn_core::{AiProviderConfig, AiProviderKind, ProviderId};
use rusqlite::{Row, params};

use crate::error::{Result, StoreError};
use crate::store::Store;

/// Accès typé à la table `ai_providers`.
#[derive(Debug)]
pub struct Providers<'a> {
    store: &'a Store,
}

impl<'a> Providers<'a> {
    /// Rattache l'accesseur à son `Store`.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Les fournisseurs déclarés, par nom.
    ///
    /// Une liste **vide** est l'installation par défaut d'Oxyn et non un état
    /// dégradé : c'est elle qui décide si le workspace IA existe (ADR-0006).
    ///
    /// L'ordre est celui du nom donné par l'utilisateur, départagé par
    /// l'identifiant : une liste de réglages qui se réordonne d'une ouverture à
    /// l'autre se lit comme un défaut.
    ///
    /// Une ligne que ce binaire ne sait pas relire — famille de protocole d'une
    /// version ultérieure, URL devenue illisible sous un éditeur SQLite — est
    /// **écartée** avec un `warn`, pas propagée en erreur : un fournisseur
    /// qu'on ne saurait pas instancier ne doit pas être proposé, et une seule
    /// ligne étrange ne doit pas rendre l'écran de configuration inutilisable.
    ///
    /// # Erreurs
    /// [`StoreError::Sqlite`] si la lecture échoue.
    pub fn list(&self) -> Result<Vec<AiProviderConfig>> {
        self.store.with_connection(|conn| {
            let mut requete = conn.prepare(
                "SELECT id, kind, label, base_url, model, secret_ref, created_at, updated_at
                 FROM ai_providers ORDER BY label, id",
            )?;
            let lignes = requete.query_and_then([], depuis_ligne)?;
            let mut fournisseurs = Vec::new();
            for ligne in lignes {
                match ligne? {
                    Some(config) => fournisseurs.push(config),
                    None => continue,
                }
            }
            Ok(fournisseurs)
        })
    }

    /// Déclare un fournisseur, ou remplace la déclaration portant son
    /// identifiant.
    ///
    /// `created_at` n'est jamais écrasé : la date de déclaration ne change pas
    /// parce qu'on a corrigé un modèle par défaut.
    ///
    /// La configuration est **revalidée** ici, même si l'exécuteur l'a déjà
    /// fait : c'est le dernier endroit où une URL portant un couple
    /// `utilisateur:motdepasse` peut être arrêtée avant le disque, et une
    /// vérification qui ne vit qu'en amont est une vérification qu'un second
    /// appelant contournera.
    ///
    /// # Erreurs
    /// [`StoreError::Corrupted`] si la déclaration est invalide — le message
    /// nomme la raison, jamais la valeur ; [`StoreError::Sqlite`] si
    /// l'écriture échoue.
    pub fn save(&self, config: &AiProviderConfig) -> Result<()> {
        config.validate().map_err(|erreur| StoreError::Corrupted {
            field: "ai_providers",
            detail: erreur.to_string(),
        })?;
        let maintenant = Utc::now();
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
                    maintenant,
                ],
            )?;
            Ok(())
        })
    }

    /// Retire une déclaration. Rend `true` si une ligne a disparu.
    ///
    /// N'efface **rien** d'autre : les documents écrits par un agent gardent
    /// leur provenance, qui dit d'où venait un texte et non quel fournisseur
    /// est encore déclaré. Le secret référencé, lui, vit dans le trousseau du
    /// système et se révoque là-bas.
    ///
    /// # Erreurs
    /// [`StoreError::Sqlite`] si la suppression échoue.
    pub fn remove(&self, id: &ProviderId) -> Result<bool> {
        self.store.with_connection(|conn| {
            let effacees = conn.execute(
                "DELETE FROM ai_providers WHERE id = ?1",
                params![id.as_str()],
            )?;
            Ok(effacees > 0)
        })
    }
}

/// Reconstruit une déclaration à partir d'une ligne.
///
/// Rend `Ok(None)` pour une ligne que le domaine ne sait pas relire : la
/// distinction avec `Err` est ce qui permet de propager une vraie panne SQLite
/// tout en écartant une ligne écrite par une version ultérieure. Aucun message
/// ne recopie la valeur fautive (I-03).
fn depuis_ligne(row: &Row<'_>) -> Result<Option<AiProviderConfig>> {
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
            ProviderId::new(id).expect("identifiant de test valide"),
            AiProviderKind::OpenAiCompatible,
            label,
            "http://localhost:11434/v1",
            "llama3.2",
        )
    }

    #[test]
    fn sans_declaration_la_liste_est_vide() {
        // ADR-0006 : c'est l'installation par défaut, et c'est ce que
        // l'interface interroge pour décider si le workspace IA existe.
        let store = Store::open_in_memory().expect("ouverture");
        assert!(store.providers().list().expect("liste").is_empty());
    }

    #[test]
    fn un_fournisseur_survit_a_la_suppression_de_tous_les_workspaces() {
        // La table n'a pas de `workspace_id` : c'est la décision d'ADR-0023.
        // Ce test rougit si quelqu'un lui ajoute une portée par workspace.
        let store = Store::open_in_memory().expect("ouverture");
        let atelier = store.workspaces().create("atelier").expect("workspace");
        store
            .providers()
            .save(&declaration("ollama", "Ollama"))
            .expect("déclaration");

        assert!(store.workspaces().delete(atelier.id).expect("suppression"));

        let restants = store.providers().list().expect("liste");
        assert_eq!(restants.len(), 1, "un fournisseur sert toutes les fenêtres");
        assert_eq!(restants[0].label, "Ollama");
    }

    #[test]
    fn enregistrer_deux_fois_le_meme_identifiant_remplace_sans_dater_a_neuf() {
        let store = Store::open_in_memory().expect("ouverture");
        let origine = declaration("ollama", "Ollama");
        store.providers().save(&origine).expect("déclaration");

        let mut corrigee = declaration("ollama", "Ollama du portable");
        corrigee.model = "qwen2.5-coder".to_owned();
        // Même si l'appelant se trompe de date de création.
        corrigee.created_at = Utc::now();
        store.providers().save(&corrigee).expect("correction");

        let liste = store.providers().list().expect("liste");
        assert_eq!(
            liste.len(),
            1,
            "l'écriture est un remplacement, pas un ajout"
        );
        assert_eq!(liste[0].label, "Ollama du portable");
        assert_eq!(liste[0].model, "qwen2.5-coder");
        assert_eq!(
            liste[0].created_at.timestamp_millis(),
            origine.created_at.timestamp_millis(),
            "corriger un modèle ne redate pas la déclaration"
        );
        assert!(liste[0].updated_at >= liste[0].created_at);
    }

    #[test]
    fn la_liste_est_ordonnee_par_nom() {
        // Une liste de réglages qui se réordonne d'une ouverture à l'autre se
        // lit comme un défaut.
        let store = Store::open_in_memory().expect("ouverture");
        for (id, label) in [
            ("openrouter", "Passerelle"),
            ("ollama", "Ollama"),
            ("lm-studio", "LM Studio"),
        ] {
            store
                .providers()
                .save(&declaration(id, label))
                .expect("déclaration");
        }
        let labels: Vec<String> = store
            .providers()
            .list()
            .expect("liste")
            .into_iter()
            .map(|config| config.label)
            .collect();
        assert_eq!(labels, ["LM Studio", "Ollama", "Passerelle"]);
    }

    #[test]
    fn aucune_url_porteuse_d_identifiants_n_atteint_le_disque() {
        // I-03 : le dernier point d'arrêt avant le fichier. Le refus est une
        // erreur, pas un nettoyage silencieux.
        let store = Store::open_in_memory().expect("ouverture");
        let mut declaration = declaration("openai", "OpenAI");
        declaration.base_url = "https://cle:motdepasse@api.example.com/v1".to_owned();

        let erreur = store
            .providers()
            .save(&declaration)
            .expect_err("une URL avec identifiants ne s'écrit pas");
        assert!(!erreur.to_string().contains("motdepasse"), "{erreur}");
        assert!(store.providers().list().expect("liste").is_empty());
    }

    #[test]
    fn seule_une_reference_de_secret_est_persistee() {
        let store = Store::open_in_memory().expect("ouverture");
        store
            .providers()
            .save(&declaration("ollama", "Ollama").with_secret_ref("keychain://oxyn/ollama"))
            .expect("déclaration");

        let colonnes: Vec<String> = store
            .with_connection(|conn| {
                let mut requete =
                    conn.prepare("SELECT name FROM pragma_table_info('ai_providers')")?;
                let noms = requete.query_map([], |row| row.get(0))?;
                Ok(noms.collect::<rusqlite::Result<Vec<String>>>()?)
            })
            .expect("schéma de la table");
        assert!(
            !colonnes
                .iter()
                .any(|nom| nom == "api_key" || nom == "reach"),
            "ni clé ni classement en base : {colonnes:?}"
        );

        let relu = store.providers().list().expect("liste").remove(0);
        assert_eq!(relu.secret_ref.as_deref(), Some("keychain://oxyn/ollama"));
    }

    #[test]
    fn retirer_une_declaration_est_idempotent() {
        let store = Store::open_in_memory().expect("ouverture");
        let id = ProviderId::ollama();
        store
            .providers()
            .save(&declaration("ollama", "Ollama"))
            .expect("déclaration");

        assert!(store.providers().remove(&id).expect("retrait"));
        assert!(
            !store.providers().remove(&id).expect("second retrait"),
            "retirer ce qui n'existe plus n'est pas une erreur"
        );
        assert!(store.providers().list().expect("liste").is_empty());
    }

    #[test]
    fn une_famille_inconnue_est_ecartee_sans_faire_tomber_la_liste() {
        // Le cas réel : un état local écrit par une version ultérieure d'Oxyn.
        // Un fournisseur qu'on ne saurait pas instancier ne doit pas être
        // proposé, et l'écran de configuration doit rester utilisable.
        let store = Store::open_in_memory().expect("ouverture");
        store
            .providers()
            .save(&declaration("ollama", "Ollama"))
            .expect("déclaration");
        store
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO ai_providers
                     VALUES ('futur','mistral','Venu du futur','https://api.example.com','m',
                             NULL, ?1, ?1)",
                    params![Utc::now()],
                )?;
                Ok(())
            })
            .expect("ligne d'une version ultérieure");

        let liste = store.providers().list().expect("liste");
        assert_eq!(liste.len(), 1);
        assert_eq!(liste[0].id, ProviderId::ollama());
    }
}
