//! La table `connections` : **des métadonnées, jamais un secret**.
//!
//! Ce module est le dernier point du produit où l'on peut empêcher un mot de
//! passe d'atteindre le disque en clair. Ce qui y est persisté, c'est ce que
//! porte [`ConnectionConfig`] : un nom, un driver, un marquage d'environnement,
//! des paramètres non secrets, et une **référence** de secret résolue ailleurs
//! par `oxyn-secrets` auprès du trousseau du système (SECURITY, I-03).
//!
//! # Le garde-fou, et pourquoi il refuse au lieu d'avertir
//!
//! [`Connections::save`] **refuse** une configuration dont un paramètre porte un
//! nom de secret — `password`, `sslpassword`, `api_key`, `client_secret`,
//! `access_token`… Un commentaire dans le schéma disant « jamais de secret ici »
//! n'empêche rien : c'est un driver écrit six mois plus tard qui rangera le mot
//! de passe dans `params` parce que c'était le champ commode, et personne ne le
//! verra jusqu'au jour où l'utilisateur ouvrira son fichier d'état.
//!
//! Le refus porte sur la **clé**, jamais sur la valeur : décider en regardant la
//! valeur supposerait de la lire, de la journaliser en cas d'erreur, et de
//! laisser passer un mot de passe qui ressemble à un nom d'hôte. Un paramètre
//! nommé `password` mais vide est refusé lui aussi — le nom est le signal.
//!
//! Les noms de fichiers de certificat (`sslkey`, `sslcert`, `sslrootcert`) ne
//! sont **pas** refusés : ce sont des chemins, et interdire `sslkey` rendrait
//! impossible l'authentification par certificat client de PostgreSQL.
//!
//! À la **relecture**, une clé suspecte n'est pas une erreur — la ligne existe
//! déjà — mais elle émet un `warn` nommant la connexion et la clé, jamais la
//! valeur.

use chrono::{DateTime, Utc};
use oxyn_core::{ConnectionConfig, ConnectionId, DriverId, WorkspaceId};
use rusqlite::{OptionalExtension, Row, params};

use crate::encoding::{environment_from_text, parse_id, privacy_tier_from_column};
use crate::error::{Result, StoreError};
use crate::store::Store;

/// Fragments qui, présents dans un nom de paramètre normalisé, le désignent
/// comme secret.
///
/// La liste vise les noms réellement rencontrés dans les chaînes de connexion
/// des SGBD et des SDK cloud. Elle est délibérément courte : chaque entrée
/// interdit un nom de paramètre légitime potentiel, et une liste trop large
/// pousse à contourner le garde-fou.
const SECRET_KEY_MARKERS: &[&str] = &[
    "password",
    "passwd",
    "pwd",
    "passphrase",
    "secret",
    "token",
    "credential",
    "apikey",
    "api_key",
    "private_key",
    "privatekey",
];

/// Accès typé à la table `connections`.
#[derive(Debug)]
pub struct Connections<'a> {
    store: &'a Store,
}

impl<'a> Connections<'a> {
    /// Rattache l'accesseur à son `Store`.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Insère ou met à jour une connexion dans un workspace.
    ///
    /// `created_at` est préservé lors d'une mise à jour.
    ///
    /// # Erreurs
    /// * [`StoreError::SecretInParams`] si un paramètre porte un nom de secret.
    ///   **Rien n'est écrit** dans ce cas ;
    /// * [`StoreError::Sqlite`] si le workspace n'existe pas — la clé étrangère
    ///   le refuse — ou si l'écriture échoue ;
    /// * [`StoreError::Json`] si les paramètres ne sont pas sérialisables.
    pub fn save(&self, workspace: WorkspaceId, config: &ConnectionConfig) -> Result<()> {
        refuse_secrets(config)?;

        let params_json = serde_json::to_string(&config.params)?;
        let maintenant = Utc::now();

        self.store.with_connection(|conn| {
            conn.execute(
                "INSERT INTO connections
                     (id, workspace_id, name, driver, environment,
                      params, secret_ref, read_only, created_at, updated_at,
                      privacy_tier)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET
                     workspace_id = excluded.workspace_id,
                     name         = excluded.name,
                     driver       = excluded.driver,
                     environment  = excluded.environment,
                     params       = excluded.params,
                     secret_ref   = excluded.secret_ref,
                     read_only    = excluded.read_only,
                     privacy_tier = excluded.privacy_tier,
                     updated_at   = excluded.updated_at",
                params![
                    config.id.to_string(),
                    workspace.to_string(),
                    config.name,
                    config.driver.as_str(),
                    config.environment.as_str(),
                    params_json,
                    config.secret_ref,
                    config.read_only,
                    maintenant,
                    maintenant,
                    config.privacy_tier.as_str(),
                ],
            )?;
            Ok(())
        })
    }

    /// Relit une connexion par son identifiant.
    ///
    /// # Erreurs
    /// [`StoreError::Sqlite`], [`StoreError::Corrupted`] ou
    /// [`StoreError::Json`].
    pub fn get(&self, id: ConnectionId) -> Result<Option<ConnectionConfig>> {
        self.store.with_connection(|conn| {
            conn.query_row(
                "SELECT id, name, driver, environment, params, secret_ref, read_only,
                        privacy_tier
                 FROM connections WHERE id = ?1",
                params![id.to_string()],
                |row| Ok(depuis_ligne(row)),
            )
            .optional()?
            .transpose()
        })
    }

    /// Liste les connexions d'un workspace, par nom.
    ///
    /// # Erreurs
    /// [`StoreError::Sqlite`], [`StoreError::Corrupted`] ou
    /// [`StoreError::Json`].
    pub fn list(&self, workspace: WorkspaceId) -> Result<Vec<ConnectionConfig>> {
        self.store.with_connection(|conn| {
            let mut requete = conn.prepare(
                "SELECT id, name, driver, environment, params, secret_ref, read_only,
                        privacy_tier
                 FROM connections WHERE workspace_id = ?1 ORDER BY name, id",
            )?;
            let lignes = requete.query_and_then(params![workspace.to_string()], depuis_ligne)?;
            lignes.collect()
        })
    }

    /// Le workspace auquel appartient une connexion.
    ///
    /// # Erreurs
    /// [`StoreError::Sqlite`] ou [`StoreError::Corrupted`].
    pub fn workspace_of(&self, id: ConnectionId) -> Result<Option<WorkspaceId>> {
        self.store.with_connection(|conn| {
            let brut: Option<String> = conn
                .query_row(
                    "SELECT workspace_id FROM connections WHERE id = ?1",
                    params![id.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            brut.as_deref()
                .map(|raw| parse_id(raw, "connections.workspace_id"))
                .transpose()
        })
    }

    /// Date de la dernière écriture de cette connexion.
    ///
    /// # Erreurs
    /// [`StoreError::Sqlite`] si la lecture échoue.
    pub fn updated_at(&self, id: ConnectionId) -> Result<Option<DateTime<Utc>>> {
        self.store.with_connection(|conn| {
            Ok(conn
                .query_row(
                    "SELECT updated_at FROM connections WHERE id = ?1",
                    params![id.to_string()],
                    |row| row.get(0),
                )
                .optional()?)
        })
    }

    /// Supprime une connexion, et en cascade son cache de catalogue.
    ///
    /// Les documents qui la visaient restent, leur connexion passant à `NULL` ;
    /// l'historique et le journal d'audit ne sont **pas** touchés.
    ///
    /// Rend `true` si une ligne a été supprimée.
    ///
    /// # Erreurs
    /// [`StoreError::Sqlite`] si la suppression échoue.
    pub fn delete(&self, id: ConnectionId) -> Result<bool> {
        self.store.with_connection(|conn| {
            let touchees = conn.execute(
                "DELETE FROM connections WHERE id = ?1",
                params![id.to_string()],
            )?;
            Ok(touchees > 0)
        })
    }
}

/// Normalise un nom de paramètre avant comparaison : minuscules, séparateurs
/// ramenés au tiret bas. `Access-Token` et `ACCESS TOKEN` valent `access_token`.
fn normalise_key(key: &str) -> String {
    key.chars()
        .map(|c| match c {
            '-' | ' ' | '.' => '_',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

/// Ce nom de paramètre désigne-t-il un secret ?
fn is_secret_key(key: &str) -> bool {
    let normalise = normalise_key(key);
    SECRET_KEY_MARKERS
        .iter()
        .any(|marqueur| normalise.contains(*marqueur))
}

/// Refuse une configuration qui rangerait un secret dans ses paramètres.
fn refuse_secrets(config: &ConnectionConfig) -> Result<()> {
    for cle in config.params.keys() {
        if is_secret_key(cle) {
            return Err(StoreError::SecretInParams { key: cle.clone() });
        }
    }
    Ok(())
}

/// Reconstruit une [`ConnectionConfig`] à partir d'une ligne.
///
/// Un environnement illisible retombe sur `production`
/// ([`environment_from_text`]) : c'est la valeur la plus contraignante, et
/// c'est ce que SECURITY exige d'un marquage absent ou douteux.
fn depuis_ligne(row: &Row<'_>) -> Result<ConnectionConfig> {
    let id: String = row.get("id")?;
    let driver: String = row.get("driver")?;
    let environment: String = row.get("environment")?;
    let params_json: String = row.get("params")?;
    let name: String = row.get("name")?;

    let driver = DriverId::new(&driver).map_err(|err| StoreError::Corrupted {
        field: "connections.driver",
        detail: err.detail().to_owned(),
    })?;

    let mut config = ConnectionConfig::new(name, driver);
    config.id = parse_id(&id, "connections.id")?;
    config.environment = environment_from_text(&environment);
    // Le type de `params` est celui du champ : le déduire évite de nommer
    // `IndexMap`, qui n'est pas une dépendance de cette crate. L'ordre de
    // saisie est conservé par la sérialisation comme par la relecture.
    config.params = serde_json::from_str(&params_json)?;
    config.secret_ref = row.get("secret_ref")?;
    config.read_only = row.get("read_only")?;
    let niveau: Option<String> = row.get("privacy_tier")?;
    config.privacy_tier = privacy_tier_from_column(niveau.as_deref());

    signale_les_cles_suspectes(&config);
    Ok(config)
}

/// Avertit si une ligne déjà écrite porte une clé de secret.
///
/// N'échoue pas : la ligne existe, la refuser rendrait la connexion
/// inutilisable sans rien effacer. Le message nomme la connexion et la clé,
/// jamais la valeur.
fn signale_les_cles_suspectes(config: &ConnectionConfig) {
    for cle in config.params.keys() {
        if is_secret_key(cle) {
            tracing::warn!(
                connection = %config.name,
                param = %cle,
                "stored connection parameter looks like a secret; secrets belong in the OS keychain"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::Environment;

    fn store_avec_workspace() -> (Store, WorkspaceId) {
        let store = Store::open_in_memory().expect("ouverture");
        let workspace = store.workspaces().create("atelier").expect("workspace");
        (store, workspace.id)
    }

    #[test]
    fn aller_retour_d_une_connexion() {
        let (store, workspace) = store_avec_workspace();
        let config = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Staging)
            .with_param("host", "db.interne")
            .with_param("port", "5432")
            .with_param("dbname", "facturation")
            .with_secret_ref("keychain://oxyn/base-client")
            .read_only();

        store
            .connections()
            .save(workspace, &config)
            .expect("écriture");
        let relu = store
            .connections()
            .get(config.id)
            .expect("lecture")
            .expect("la connexion existe");

        assert_eq!(relu.id, config.id);
        assert_eq!(relu.name, "base client");
        assert_eq!(relu.driver, DriverId::postgres());
        assert_eq!(relu.environment, Environment::Staging);
        assert_eq!(
            relu.secret_ref.as_deref(),
            Some("keychain://oxyn/base-client")
        );
        assert!(relu.read_only);
        assert_eq!(relu.params.len(), 3);
    }

    #[test]
    fn l_ordre_des_parametres_est_conserve() {
        // Un fichier de configuration qui se réordonne tout seul produit des
        // différences illisibles ; `ConnectionConfig` s'appuie sur cet ordre.
        let (store, workspace) = store_avec_workspace();
        let config = ConnectionConfig::new("ordre", DriverId::sqlite())
            .with_param("zeta", "1")
            .with_param("alpha", "2")
            .with_param("mu", "3");

        store
            .connections()
            .save(workspace, &config)
            .expect("écriture");
        let relu = store
            .connections()
            .get(config.id)
            .expect("lecture")
            .expect("présente");

        let cles: Vec<&str> = relu.params.keys().map(String::as_str).collect();
        assert_eq!(cles, ["zeta", "alpha", "mu"]);
    }

    #[test]
    fn un_parametre_qui_porte_un_nom_de_secret_est_refuse() {
        let (store, workspace) = store_avec_workspace();

        for cle in [
            "password",
            "PASSWORD",
            "sslpassword",
            "passwd",
            "pwd",
            "passphrase",
            "api_key",
            "API-KEY",
            "apikey",
            "client_secret",
            "access_token",
            "aws_credentials",
            "private_key",
        ] {
            let config = ConnectionConfig::new("essai", DriverId::postgres())
                .with_param("host", "localhost")
                .with_param(cle, "peu importe");

            let erreur = store
                .connections()
                .save(workspace, &config)
                .expect_err("un paramètre au nom de secret doit être refusé");
            assert!(
                matches!(erreur, StoreError::SecretInParams { .. }),
                "`{cle}` : {erreur}"
            );
            assert!(
                !erreur.to_string().contains("peu importe"),
                "la valeur ne doit jamais apparaître"
            );
            assert!(
                store
                    .connections()
                    .get(config.id)
                    .expect("lecture")
                    .is_none(),
                "`{cle}` : rien ne doit avoir été écrit"
            );
        }
    }

    #[test]
    fn un_parametre_vide_mais_nomme_comme_un_secret_est_refuse_aussi() {
        // C'est le nom qui est le signal : regarder la valeur supposerait de la
        // lire, et un mot de passe qui ressemble à un nom d'hôte passerait.
        let (store, workspace) = store_avec_workspace();
        let config =
            ConnectionConfig::new("essai", DriverId::postgres()).with_param("password", "");
        assert!(store.connections().save(workspace, &config).is_err());
    }

    #[test]
    fn les_chemins_de_certificat_restent_autorises() {
        // Refuser `sslkey` rendrait impossible l'authentification par
        // certificat client de PostgreSQL : ce sont des chemins, pas des clés.
        let (store, workspace) = store_avec_workspace();
        let config = ConnectionConfig::new("tls", DriverId::postgres())
            .with_param("sslmode", "verify-full")
            .with_param("sslkey", "/etc/ssl/client.key")
            .with_param("sslcert", "/etc/ssl/client.crt")
            .with_param("sslrootcert", "/etc/ssl/ca.crt")
            .with_param("host", "db.interne")
            .with_param("application_name", "oxyn");

        store
            .connections()
            .save(workspace, &config)
            .expect("aucun de ces paramètres n'est un secret");
    }

    #[test]
    fn une_connexion_sans_environnement_se_relit_en_production() {
        let (store, workspace) = store_avec_workspace();
        // Aucun `with_environment` : le défaut de `ConnectionConfig` s'applique.
        let config = ConnectionConfig::new("ajoutée à la hâte", DriverId::postgres());
        assert!(config.is_production());

        store
            .connections()
            .save(workspace, &config)
            .expect("écriture");
        let relu = store
            .connections()
            .get(config.id)
            .expect("lecture")
            .expect("présente");
        assert!(relu.is_production());
    }

    #[test]
    fn un_environnement_illisible_se_relit_en_production() {
        let (store, workspace) = store_avec_workspace();
        let config = ConnectionConfig::new("trafiquée", DriverId::postgres())
            .with_environment(Environment::Local);
        store
            .connections()
            .save(workspace, &config)
            .expect("écriture");

        // Quelqu'un ouvre le fichier avec `sqlite3` et écrit n'importe quoi.
        store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE connections SET environment = 'presque-du-dev' WHERE id = ?1",
                    params![config.id.to_string()],
                )?;
                Ok(())
            })
            .expect("altération");

        let relu = store
            .connections()
            .get(config.id)
            .expect("lecture")
            .expect("présente");
        assert!(
            relu.is_production(),
            "SECURITY : un marquage illisible vaut production"
        );
    }

    #[test]
    fn une_connexion_sans_workspace_est_refusee() {
        let store = Store::open_in_memory().expect("ouverture");
        let config = ConnectionConfig::new("orpheline", DriverId::sqlite());
        assert!(
            store
                .connections()
                .save(WorkspaceId::new(), &config)
                .is_err(),
            "la clé étrangère doit refuser un workspace inexistant"
        );
    }

    #[test]
    fn la_liste_est_bornee_au_workspace() {
        let store = Store::open_in_memory().expect("ouverture");
        let a = store.workspaces().create("a").expect("workspace");
        let b = store.workspaces().create("b").expect("workspace");

        store
            .connections()
            .save(a.id, &ConnectionConfig::new("dans a", DriverId::sqlite()))
            .expect("écriture");
        store
            .connections()
            .save(b.id, &ConnectionConfig::new("dans b", DriverId::sqlite()))
            .expect("écriture");

        let dans_a = store.connections().list(a.id).expect("liste");
        assert_eq!(dans_a.len(), 1);
        assert_eq!(dans_a[0].name, "dans a");
    }

    #[test]
    fn supprimer_un_workspace_emporte_ses_connexions() {
        let (store, workspace) = store_avec_workspace();
        let config = ConnectionConfig::new("éphémère", DriverId::sqlite());
        store
            .connections()
            .save(workspace, &config)
            .expect("écriture");

        assert!(store.workspaces().delete(workspace).expect("suppression"));
        assert!(
            store
                .connections()
                .get(config.id)
                .expect("lecture")
                .is_none(),
            "la cascade doit s'appliquer : PRAGMA foreign_keys est actif"
        );
    }

    #[test]
    fn une_mise_a_jour_ne_cree_pas_de_doublon() {
        let (store, workspace) = store_avec_workspace();
        let mut config = ConnectionConfig::new("avant", DriverId::sqlite());
        store
            .connections()
            .save(workspace, &config)
            .expect("écriture");

        config.name = "après".to_owned();
        store
            .connections()
            .save(workspace, &config)
            .expect("mise à jour");

        let liste = store.connections().list(workspace).expect("liste");
        assert_eq!(liste.len(), 1);
        assert_eq!(liste[0].name, "après");
    }

    #[test]
    fn le_debug_d_une_connexion_relue_masque_toujours_les_valeurs() {
        // La garantie vient d'`oxyn-core`, mais elle doit tenir après un
        // aller-retour par le disque : c'est le trajet réel.
        let (store, workspace) = store_avec_workspace();
        let config = ConnectionConfig::new("prod", DriverId::postgres())
            .with_param("host", "db-secret.interne")
            .with_secret_ref("keychain://oxyn/prod");
        store
            .connections()
            .save(workspace, &config)
            .expect("écriture");

        let relu = store
            .connections()
            .get(config.id)
            .expect("lecture")
            .expect("présente");
        let rendu = format!("{relu:?}");
        assert!(!rendu.contains("db-secret.interne"), "{rendu}");
        assert!(!rendu.contains("keychain://oxyn/prod"), "{rendu}");
    }

    /// ADR-0006 : le niveau appartient à la connexion, donc il lui survit.
    ///
    /// Le défaut que ce test ferme était silencieux et permissif : sans
    /// colonne, toute connexion relue repartait à `Metadata`. Un utilisateur
    /// réglant `Local` sur une base client, fermant Oxyn et la rouvrant voyait
    /// le DDL et les noms de colonnes repartir chez un fournisseur distant,
    /// sans message nulle part.
    #[test]
    fn le_niveau_de_confidentialite_survit_a_la_fermeture() {
        let (store, workspace) = store_avec_workspace();
        let mut config =
            ConnectionConfig::new("base client", DriverId::new("postgres").expect("driver"));
        config.privacy_tier = oxyn_core::PrivacyTier::Local;
        store
            .connections()
            .save(workspace, &config)
            .expect("écriture");

        let relu = store
            .connections()
            .get(config.id)
            .expect("lecture")
            .expect("présente");
        assert_eq!(
            relu.privacy_tier,
            oxyn_core::PrivacyTier::Local,
            "le niveau appartient à la connexion, pas à la session"
        );
    }

    /// Une absence et une valeur illisible ne veulent pas dire la même chose.
    ///
    /// Absente, la colonne dit « ce binaire est plus ancien que ce réglage » :
    /// l'utilisateur n'en a jamais choisi, et le défaut d'ADR-0006 s'applique.
    /// Illisible, elle dit « un réglage existait et son sens s'est perdu » : le
    /// plus contraignant s'applique, parce qu'une base dont on ne sait plus ce
    /// qu'elle autorisait n'obtient pas le bénéfice du doute.
    #[test]
    fn une_valeur_illisible_nest_pas_une_absence() {
        let (store, workspace) = store_avec_workspace();
        let config = ConnectionConfig::new("héritée", DriverId::new("sqlite").expect("driver"));
        store
            .connections()
            .save(workspace, &config)
            .expect("écriture");

        // Le cas de la ligne antérieure à la migration.
        store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE connections SET privacy_tier = NULL WHERE id = ?1",
                    params![config.id.to_string()],
                )?;
                Ok(())
            })
            .expect("mise à zéro");
        assert_eq!(
            store
                .connections()
                .get(config.id)
                .expect("lecture")
                .expect("présente")
                .privacy_tier,
            oxyn_core::PrivacyTier::Metadata,
            "aucune valeur écrite : le défaut d'ADR-0006 s'applique"
        );

        // Le cas de la valeur qu'on ne sait plus lire.
        store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE connections SET privacy_tier = 'confidentiel' WHERE id = ?1",
                    params![config.id.to_string()],
                )?;
                Ok(())
            })
            .expect("valeur inconnue");
        assert_eq!(
            store
                .connections()
                .get(config.id)
                .expect("lecture")
                .expect("présente")
                .privacy_tier,
            oxyn_core::PrivacyTier::Local,
            "un réglage dont le sens s'est perdu retombe sur le plus contraignant"
        );
    }
}
