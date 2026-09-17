//! Construire les paramètres de connexion sans jamais les rendre affichables.
//!
//! # Le piège que ce module existe pour fermer
//!
//! [`PgConnectOptions`] de `sqlx` 0.9 dérive `Debug` **et porte le mot de passe
//! en clair**. Un `tracing::debug!("{options:?}")` ajouté six mois plus tard
//! écrit donc un mot de passe de production dans un fichier de journal — c'est
//! exactement le mode de fuite que décrit [I-03](../../../CLAUDE.md#i-03), et il
//! est invisible à la relecture.
//!
//! [`ConnectSpec`] emballe ces options et **n'a pas de `Debug` dérivé** : le
//! sien ne montre que l'hôte, le port, la base, l'utilisateur et le mode TLS. Le
//! type ne se sérialise pas, ne s'affiche pas, et son seul chemin de sortie est
//! `ConnectSpec::options`, qui rend une référence à passer à `sqlx`.
//!
//! Corollaire : **aucune structure de cette crate ne range un
//! [`PgConnectOptions`] nu.** Un test le vérifie.
//!
//! # Le second piège : l'environnement
//!
//! Un driver reçoit sa configuration, il ne va pas la chercher
//! ([DRIVER-CONTRACT](../../../docs/DRIVER-CONTRACT.md)). Or **tous** les
//! constructeurs de [`PgConnectOptions`] lisent `PGHOST`, `PGPORT`, `PGUSER`,
//! `PGDATABASE`, `PGPASSWORD`, `PGSSLMODE`, `PGAPPNAME` et `PGOPTIONS` ; il n'en
//! existe aucun qui ne le fasse pas.
//!
//! Ce module écrase donc systématiquement tout ce qu'il peut écraser — y compris
//! le mot de passe, remplacé par une chaîne vide quand la connexion n'en déclare
//! pas, ce qui neutralise `PGPASSWORD`. Une authentification `trust` ou `peer`
//! n'en souffre pas : le serveur ne réclame alors aucun mot de passe.
//!
//! Ce qui **reste** hors de portée est consigné dans [`LEAKY_ENV`] : `sqlx`
//! n'offre pas d'accesseur pour effacer un certificat ou des options serveur
//! héritées de l'environnement.
//
// TODO(phase 1) : demander en amont un constructeur `PgConnectOptions` sans
// lecture d'environnement, ou porter le nettoyage dans `oxyn-app` avant le
// démarrage du runtime. Débloque : la suppression de `LEAKY_ENV`.

use std::fmt;
use std::time::Duration;

use oxyn_core::{ConnectionConfig, OxynError, Result};
use oxyn_driver::{Credentials, DriverMetadata};
use secrecy::ExposeSecret as _;
use sqlx::postgres::{PgConnectOptions, PgSslMode};

/// Les variables d'environnement que `sqlx` lit et que ce module ne peut pas
/// neutraliser, faute d'accesseur pour les effacer.
///
/// Elles ne portent pas de secret — ce sont des chemins de certificats et des
/// options serveur —, mais elles peuvent changer le comportement d'une connexion
/// à l'insu de sa configuration. Documentées ici plutôt que tues.
pub const LEAKY_ENV: [&str; 4] = ["PGSSLROOTCERT", "PGSSLCERT", "PGSSLKEY", "PGOPTIONS"];

/// Nom applicatif annoncé au serveur quand la connexion n'en fixe pas.
///
/// Apparaît dans `pg_stat_activity` : c'est ce qui permet à un DBA de savoir
/// qu'une requête vient d'Oxyn plutôt que d'un traitement par lots.
pub const DEFAULT_APPLICATION_NAME: &str = "oxyn";

/// Port par défaut du protocole.
pub const DEFAULT_PORT: u16 = 5432;

/// Les paramètres d'une connexion PostgreSQL, secret compris, non affichables.
#[derive(Clone)]
pub struct ConnectSpec {
    options: PgConnectOptions,
    /// Ce qui est montrable, figé à la construction pour que `Debug` n'ait rien
    /// à extraire des options — donc rien à oublier de masquer.
    apercu: Apercu,
}

/// La part montrable d'une connexion.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Apercu {
    host: String,
    port: u16,
    database: String,
    user: String,
    sslmode: &'static str,
}

impl ConnectSpec {
    /// Construit les paramètres depuis une configuration et des identifiants
    /// résolus.
    ///
    /// La configuration ne porte **aucun** secret : [`DriverMetadata::validate`]
    /// a déjà refusé tout paramètre qui y ressemblerait. Le mot de passe arrive
    /// par `credentials`, résolu depuis le trousseau du système par l'appelant.
    ///
    /// Les clés reconnues sont `host`, `port`, `database`, `user`, `sslmode` et
    /// `application_name`, avec leurs alias usuels. **Toute autre clé devient une
    /// option de session du serveur** (`-c clé=valeur`) : c'est le sens qu'elle a
    /// dans une URL `libpq`, et l'utilisateur l'a écrite explicitement.
    ///
    /// # Erreurs
    /// [`OxynError::Config`] si la configuration ne désigne pas ce driver, si un
    /// paramètre obligatoire manque, si le port n'est pas un entier sur 16 bits,
    /// ou si `sslmode` ne nomme pas un mode connu.
    pub fn from_config(
        metadata: &DriverMetadata,
        config: &ConnectionConfig,
        credentials: &Credentials,
    ) -> Result<Self> {
        metadata.validate(config)?;

        let mut host = "localhost".to_owned();
        let mut port = metadata.default_port.unwrap_or(DEFAULT_PORT);
        let mut database = String::new();
        let mut user = String::new();
        let mut sslmode = PgSslMode::Prefer;
        let mut sslmode_nom = "prefer";
        let mut application_name = DEFAULT_APPLICATION_NAME.to_owned();
        let mut extras: Vec<(String, String)> = Vec::new();

        for (cle, valeur) in &config.params {
            match cle.trim().to_ascii_lowercase().as_str() {
                "host" | "hostname" | "server" => host = valeur.trim().to_owned(),
                "port" => {
                    port = valeur.trim().parse::<u16>().map_err(|_| {
                        OxynError::Config(
                            "parameter `port` expects an integer between 1 and 65535".to_owned(),
                        )
                    })?;
                }
                "database" | "dbname" | "db" => database = valeur.trim().to_owned(),
                "user" | "username" => user = valeur.trim().to_owned(),
                "sslmode" | "ssl_mode" => {
                    let (mode, nom) = parse_ssl_mode(valeur.trim())?;
                    sslmode = mode;
                    sslmode_nom = nom;
                }
                "application_name" => {
                    let demande = valeur.trim();
                    if !demande.is_empty() {
                        application_name = demande.to_owned();
                    }
                }
                _ => extras.push((cle.clone(), valeur.clone())),
            }
        }

        if host.is_empty() {
            return Err(OxynError::Config("parameter `host` is required".to_owned()));
        }
        if database.is_empty() {
            return Err(OxynError::Config(
                "parameter `database` is required".to_owned(),
            ));
        }
        if user.is_empty() {
            return Err(OxynError::Config("parameter `user` is required".to_owned()));
        }

        // `new_without_pgpass` plutôt que `new` : le second lit en plus le
        // fichier `~/.pgpass`, ce qu'un driver n'a pas à faire. Tout ce que
        // celui-ci a pu lire dans l'environnement est écrasé juste après.
        let mut options = PgConnectOptions::new_without_pgpass()
            .host(&host)
            .port(port)
            .database(&database)
            .username(&user)
            .ssl_mode(sslmode)
            .application_name(&application_name);

        // Toujours poser le mot de passe, même vide : c'est ce qui empêche un
        // `PGPASSWORD` traînant dans l'environnement de s'appliquer à une
        // connexion qui n'en déclare pas.
        options = match credentials.password() {
            Some(secret) => options.password(secret.expose_secret()),
            None => options.password(""),
        };

        if !extras.is_empty() {
            options = options.options(extras.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        }
        // Un réglage de session que le driver **impose**, et déclare ici.
        //
        // Le découpeur et le classifieur d'`oxyn-query` lisent `'a\'` comme une
        // chaîne complète, ce qui n'est vrai qu'avec
        // `standard_conforming_strings = on`. Un serveur réglé à `off` —
        // `ALTER DATABASE … SET`, héritage d'une application ancienne — lirait
        // la suite comme du code : un `DELETE` classé lecture partirait sans la
        // confirmation qui nomme la connexion ([I-02](../../../CLAUDE.md#i-02)).
        //
        // Posé comme paramètre de démarrage, **après** les options de
        // l'utilisateur : un `-c` plus tardif l'emporte, et un paramètre de
        // connexion prime sur `ALTER DATABASE` et `ALTER ROLE`. Il ne change le
        // sens d'aucune requête écrite pour un serveur à jour — c'est la valeur
        // par défaut de PostgreSQL. Un serveur qui refuserait le paramètre
        // refuse la connexion, ce qui se voit ; le curseur le repose après
        // toute exécution qui aurait pu le changer (`SQL_RESET_AFTER_WRITE`).
        options = options.options([("standard_conforming_strings", "on")]);

        Ok(Self {
            options,
            apercu: Apercu {
                host,
                port,
                database,
                user,
                sslmode: sslmode_nom,
            },
        })
    }

    /// Les paramètres à remettre à `sqlx`.
    ///
    /// Le nom est neutre, mais l'usage ne l'est pas : ce qui sort d'ici porte le
    /// mot de passe. Le seul appel légitime est l'ouverture d'une connexion.
    #[must_use]
    pub(crate) fn options(&self) -> &PgConnectOptions {
        &self.options
    }

    /// L'hôte, tel que la configuration l'a donné.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.apercu.host
    }

    /// Le port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.apercu.port
    }

    /// La base visée.
    #[must_use]
    pub fn database(&self) -> &str {
        &self.apercu.database
    }

    /// L'utilisateur.
    #[must_use]
    pub fn user(&self) -> &str {
        &self.apercu.user
    }

    /// Le mode TLS demandé, sous son nom `libpq`.
    #[must_use]
    pub const fn ssl_mode(&self) -> &'static str {
        self.apercu.sslmode
    }
}

impl fmt::Debug for ConnectSpec {
    /// Écrit à la main : ce type porte un secret, et un `Debug` dérivé est le
    /// mode de fuite le plus fréquent parce qu'il est invisible à la relecture
    /// ([I-03](../../../CLAUDE.md#i-03)).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectSpec")
            .field("host", &self.apercu.host)
            .field("port", &self.apercu.port)
            .field("database", &self.apercu.database)
            .field("user", &self.apercu.user)
            .field("sslmode", &self.apercu.sslmode)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Traduit un `sslmode` `libpq` vers son équivalent `sqlx`.
///
/// Les six noms sont ceux de `libpq`, parce que c'est ce que l'utilisateur a
/// dans ses fichiers de connexion existants.
fn parse_ssl_mode(valeur: &str) -> Result<(PgSslMode, &'static str)> {
    let couple = match valeur.to_ascii_lowercase().as_str() {
        "disable" => (PgSslMode::Disable, "disable"),
        "allow" => (PgSslMode::Allow, "allow"),
        "prefer" | "" => (PgSslMode::Prefer, "prefer"),
        "require" => (PgSslMode::Require, "require"),
        "verify-ca" | "verify_ca" => (PgSslMode::VerifyCa, "verify-ca"),
        "verify-full" | "verify_full" => (PgSslMode::VerifyFull, "verify-full"),
        _ => {
            return Err(OxynError::Config(
                "parameter `sslmode` expects: disable, allow, prefer, require, \
                 verify-ca or verify-full"
                    .to_owned(),
            ));
        }
    };
    Ok(couple)
}

/// Délai d'acquisition d'une connexion dans le bassin.
///
/// Court à dessein : au-delà, l'utilisateur préfère une erreur claire à une
/// interface qui semble figée ([PERFORMANCE](../../../docs/PERFORMANCE.md)).
pub const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(15);

/// Nombre de connexions simultanées d'une session.
///
/// Une session Oxyn n'est pas un serveur d'applications : quelques onglets, un
/// arbre de catalogue, et l'annulation — qui, elle, n'emprunte **pas** le bassin
/// (voir [`crate::session`]). Quatre suffit, et laisse la base tranquille.
pub const MAX_CONNECTIONS: u32 = 4;

#[cfg(test)]
mod tests {
    use oxyn_core::DriverId;

    use super::*;
    use crate::driver::postgres_metadata;

    const MOT_DE_PASSE: &str = "hunter2";

    fn config() -> ConnectionConfig {
        ConnectionConfig::new("caisse", DriverId::postgres())
            .with_param("host", "db.interne")
            .with_param("port", "6543")
            .with_param("database", "caisse")
            .with_param("user", "lecture")
    }

    #[test]
    fn le_debug_ne_montre_jamais_le_mot_de_passe() {
        // I-03, corollaire vérifiable. Ce test est la raison d'être du module.
        let spec = ConnectSpec::from_config(
            &postgres_metadata(),
            &config(),
            &Credentials::new().with_password(MOT_DE_PASSE),
        )
        .expect("configuration complète");

        let rendu = format!("{spec:?}");
        assert!(!rendu.contains(MOT_DE_PASSE), "fuite : {rendu}");
        assert!(rendu.contains("<redacted>"), "{rendu}");

        // Ce qui reste doit rester utile au diagnostic.
        assert!(rendu.contains("db.interne"), "{rendu}");
        assert!(rendu.contains("6543"), "{rendu}");
        assert!(rendu.contains("lecture"), "{rendu}");
    }

    #[test]
    fn les_paramtres_reconnus_arrivent_jusqu_a_sqlx() {
        let spec = ConnectSpec::from_config(&postgres_metadata(), &config(), &Credentials::new())
            .expect("configuration complète");
        assert_eq!(spec.host(), "db.interne");
        assert_eq!(spec.port(), 6543);
        assert_eq!(spec.database(), "caisse");
        assert_eq!(spec.user(), "lecture");
        assert_eq!(spec.ssl_mode(), "prefer");
    }

    #[test]
    fn le_port_par_defaut_est_celui_du_protocole() {
        let sans_port = ConnectionConfig::new("caisse", DriverId::postgres())
            .with_param("host", "localhost")
            .with_param("database", "caisse")
            .with_param("user", "lecture");
        let spec = ConnectSpec::from_config(&postgres_metadata(), &sans_port, &Credentials::new())
            .expect("configuration complète");
        assert_eq!(spec.port(), DEFAULT_PORT);
    }

    #[test]
    fn un_port_illisible_est_refuse_avec_un_message_qui_dit_quoi_faire() {
        let mauvais = config().with_param("port", "cinq-mille");
        let erreur = ConnectSpec::from_config(&postgres_metadata(), &mauvais, &Credentials::new())
            .expect_err("refus attendu");
        assert!(matches!(erreur, OxynError::Config(_)), "{erreur:?}");
        assert!(erreur.to_string().contains("65535"), "{erreur}");
    }

    #[test]
    fn un_sslmode_inconnu_est_refuse_plutot_que_degrade_en_silence() {
        // Le degrader vers `prefer` ouvrirait une connexion en clair là où
        // l'utilisateur a demandé du chiffrement.
        let mauvais = config().with_param("sslmode", "peut-etre");
        let erreur = ConnectSpec::from_config(&postgres_metadata(), &mauvais, &Credentials::new())
            .expect_err("refus attendu");
        assert!(matches!(erreur, OxynError::Config(_)), "{erreur:?}");
        assert!(erreur.to_string().contains("verify-full"), "{erreur}");
    }

    #[test]
    fn les_six_modes_tls_de_libpq_sont_acceptes() {
        for nom in [
            "disable",
            "allow",
            "prefer",
            "require",
            "verify-ca",
            "verify-full",
        ] {
            let avec = config().with_param("sslmode", nom);
            let spec = ConnectSpec::from_config(&postgres_metadata(), &avec, &Credentials::new())
                .expect("mode connu");
            assert_eq!(spec.ssl_mode(), nom);
        }
    }

    #[test]
    fn un_secret_range_dans_les_parametres_est_refuse() {
        // La configuration se persiste : un secret n'y a pas sa place (I-03).
        // C'est `DriverMetadata::validate` qui refuse, et ce module l'appelle.
        let fuite = config().with_param("password", MOT_DE_PASSE);
        let erreur = ConnectSpec::from_config(&postgres_metadata(), &fuite, &Credentials::new())
            .expect_err("refus attendu");
        assert!(matches!(erreur, OxynError::Config(_)), "{erreur:?}");
        assert!(!erreur.to_string().contains(MOT_DE_PASSE), "{erreur}");
    }

    #[test]
    fn un_champ_obligatoire_manquant_se_nomme() {
        let sans_base = ConnectionConfig::new("caisse", DriverId::postgres())
            .with_param("host", "localhost")
            .with_param("user", "lecture");
        let erreur =
            ConnectSpec::from_config(&postgres_metadata(), &sans_base, &Credentials::new())
                .expect_err("refus attendu");
        assert!(erreur.to_string().contains("database"), "{erreur}");
    }

    #[test]
    fn une_configuration_visant_un_autre_driver_est_refusee() {
        let ailleurs = ConnectionConfig::new("fichier", DriverId::sqlite())
            .with_param("host", "localhost")
            .with_param("database", "caisse")
            .with_param("user", "lecture");
        let erreur = ConnectSpec::from_config(&postgres_metadata(), &ailleurs, &Credentials::new())
            .expect_err("refus attendu");
        assert!(matches!(erreur, OxynError::Config(_)), "{erreur:?}");
    }
}
