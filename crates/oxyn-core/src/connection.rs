//! Configuration d'une connexion : marquage de son environnement, et niveau de
//! confidentialité de ce qui peut rejoindre une invite IA.
//!
//! Trois règles gouvernent ce module. Les deux premières viennent de
//! [`SECURITY`](../../../docs/SECURITY.md) :
//!
//! 1. **Aucun secret ici.** Mot de passe, chaîne de connexion complète, clé SSH,
//!    certificat client : rien de tout cela n'entre dans [`ConnectionConfig`].
//!    Ce qui est persisté, c'est une *référence* au secret
//!    ([`secret_ref`](ConnectionConfig::secret_ref)), résolue par `oxyn-secrets`
//!    auprès du trousseau du système. La panne évitée est concrète : un fichier
//!    de workspace contenant un mot de passe de production, commité par
//!    l'utilisateur dans le dépôt de son équipe.
//! 2. **Le défaut est [`Environment::Production`]**, la valeur la plus
//!    contraignante. Le défaut inverse est ce qui laisse partir un `UPDATE` sans
//!    `WHERE` sur la base client parce que l'utilisateur a ajouté la connexion à
//!    la hâte sans remplir le champ.
//!
//! La troisième vient de [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md) :
//!
//! 3. **Le [`PrivacyTier`] est porté par la connexion**, pas par la session, le
//!    fournisseur ou l'application, et son défaut est
//!    [`Metadata`](PrivacyTier::Metadata). Le type vit ici — et non dans
//!    `oxyn-ai` — précisément parce que c'est cette structure qui le porte : un
//!    niveau rangé ailleurs finit par être un réglage global, ce qu'ADR-0006
//!    refuse.
//!
//! Le `Debug` de [`ConnectionConfig`] est écrit à la main : les valeurs des
//! paramètres sont masquées. Un `Debug` dérivé est le mode de fuite le plus
//! fréquent, parce qu'il est invisible à la relecture — c'est le
//! `tracing::debug!("{cfg:?}")` ajouté six mois plus tard qui fuit (I-03).

use std::fmt;
use std::str::FromStr;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::ids::{ConnectionId, DriverId, IdParseError};

/// Environnement d'une connexion.
///
/// Les quatre valeurs sont exactement celles de
/// [`SECURITY`](../../../docs/SECURITY.md) ; l'énumération est fermée pour la
/// même raison que [`StatementIntent`](crate::query::StatementIntent) : un
/// environnement de plus doit forcer la relecture de chaque décision qui en
/// dépend.
///
/// L'ordre des variantes est celui de la contrainte croissante, et il est
/// **signifiant** : `Ord` sert à prendre le plus contraignant de deux
/// environnements (voir `DefaultPolicy`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    /// Base locale, jetable.
    Local,
    /// Environnement de développement partagé.
    Development,
    /// Préproduction : ressemble à la production, mais les données ne comptent
    /// pas de la même façon.
    Staging,
    /// Production. **C'est le défaut** quand l'environnement n'est pas
    /// renseigné.
    #[default]
    Production,
}

impl Environment {
    /// S'agit-il de la production ?
    #[must_use]
    pub const fn is_production(&self) -> bool {
        matches!(self, Self::Production)
    }

    /// Nom stable, celui qui est écrit dans les fichiers de workspace.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Development => "development",
            Self::Staging => "staging",
            Self::Production => "production",
        }
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Environment {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local" => Ok(Self::Local),
            "development" | "dev" => Ok(Self::Development),
            "staging" | "stage" => Ok(Self::Staging),
            "production" | "prod" => Ok(Self::Production),
            _ => Err(IdParseError::new(
                "Environment",
                "expected: local, development, staging or production",
            )),
        }
    }
}

/// Ce qui a le droit de quitter la machine pour une connexion donnée.
///
/// Autorité : [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md). Le tableau
/// des niveaux y vit et n'est pas recopié ici.
///
/// # Pourquoi par connexion, et jamais globalement
///
/// La panne visée par [AI-PROVIDERS](../../../docs/AI-PROVIDERS.md) :
/// l'utilisateur règle le niveau sur `Sampled` pour sa base de bac à sable,
/// l'oublie, puis ouvre trois jours plus tard la base client de son employeur.
/// Si le niveau était global, des lignes réelles partiraient chez un
/// fournisseur tiers. Techniquement rien n'a échoué ; contractuellement, c'est
/// irréversible.
///
/// Conséquence de conception : ce type ne porte **aucun** constructeur qui le
/// dérive d'un fournisseur, d'une session ou d'un réglage d'application. Il
/// vient du champ [`ConnectionConfig::privacy_tier`] et de rien d'autre (I-04).
///
/// # `Metadata` par défaut n'est pas « rien ne sort »
///
/// Le DDL, les noms, les types, les index et les cardinalités **sortent** dès
/// qu'un fournisseur distant est configuré. Une table `patients` avec une
/// colonne `hiv_status` révèle l'essentiel sans qu'une seule ligne ne sorte.
/// C'est un compromis délibéré, et l'interface doit le montrer en permanence.
///
/// # L'énumération est fermée
///
/// Contrairement à la convention du dépôt sur les énumérations publiques : la
/// triade d'ADR-0006 est un contrat, et un quatrième niveau serait une décision
/// d'ADR, pas une variante ajoutée au fil de l'eau. Un `_ =>` dans l'interface
/// qui avalerait un niveau inconnu choisirait silencieusement le mauvais
/// comportement.
///
/// L'ordre est celui de la **divulgation croissante** : `Local < Metadata <
/// Sampled`. C'est ce qui rend [`most_restrictive`](Self::most_restrictive)
/// écrivable, et donc composable quand deux niveaux s'appliquent au même envoi.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyTier {
    /// Rien ne quitte la machine. Modèle local uniquement.
    Local,
    /// DDL, noms, types, index, cardinalités, plans d'exécution.
    /// **Aucune valeur de ligne.** C'est le défaut.
    #[default]
    Metadata,
    /// Idem, plus un échantillon de lignes explicitement approuvé, colonne par
    /// colonne.
    Sampled,
}

impl PrivacyTier {
    /// Des valeurs de lignes peuvent-elles rejoindre une invite ?
    ///
    /// Seul [`Sampled`](Self::Sampled) répond `true`, et même alors les valeurs
    /// doivent avoir été approuvées colonne par colonne en amont : ce prédicat
    /// est une condition nécessaire, pas suffisante.
    ///
    /// C'est **le** prédicat qui gouverne tout contenu susceptible de citer une
    /// ligne — un échantillon, mais aussi un message d'erreur de serveur, qui
    /// recopie la valeur qui viole une contrainte.
    #[must_use]
    pub const fn allows_row_values(&self) -> bool {
        matches!(self, Self::Sampled)
    }

    /// Un fournisseur dont les données quittent la machine est-il utilisable ?
    ///
    /// `false` pour [`Local`](Self::Local). Ce n'est pas une valeur par défaut
    /// qu'un réglage renverse : c'est la promesse du niveau.
    ///
    /// Le classement local/distant d'un point d'accès ne se fait **jamais** sur
    /// la forme de son URL — un point d'accès compatible OpenAI en écoute sur
    /// la boucle locale peut être un mandataire vers le nuage. Il se fait sur
    /// l'hôte réel après résolution, ce qui est une opération bloquante : elle
    /// vit dans `oxyn-llm`, avec le reste de ce qui parle au réseau.
    #[must_use]
    pub const fn allows_remote_provider(&self) -> bool {
        !matches!(self, Self::Local)
    }

    /// Le plus contraignant des deux niveaux.
    ///
    /// Sert partout où deux niveaux se rencontrent — une conversation qui
    /// touche deux connexions, un contexte assemblé avant que l'utilisateur ne
    /// change de connexion. Le résultat ne divulgue jamais plus que le plus
    /// prudent des deux.
    #[must_use]
    pub fn most_restrictive(self, other: Self) -> Self {
        self.min(other)
    }

    /// Nom stable, pour l'affichage, la persistance et l'audit.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Metadata => "metadata",
            Self::Sampled => "sampled",
        }
    }

    /// Ce qui sort de la machine sous ce niveau, en une phrase montrable.
    ///
    /// En anglais : cette phrase rejoint une invite, c'est donc du texte de
    /// code. L'interface doit afficher le niveau effectif **en permanence** et
    /// non dans un panneau de réglages : un utilisateur qui ne peut pas dire
    /// d'un coup d'œil où part sa requête ne donne pas un consentement éclairé
    /// (AI-PROVIDERS).
    #[must_use]
    pub const fn describe(&self) -> &'static str {
        match self {
            Self::Local => "nothing leaves this machine; local model only",
            Self::Metadata => "schema only: names, types, indexes, cardinalities — no row values",
            Self::Sampled => "schema, plus row samples you approved column by column",
        }
    }
}

impl fmt::Display for PrivacyTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PrivacyTier {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local" => Ok(Self::Local),
            "metadata" => Ok(Self::Metadata),
            "sampled" => Ok(Self::Sampled),
            _ => Err(IdParseError::new(
                "PrivacyTier",
                "expected: local, metadata or sampled",
            )),
        }
    }
}

/// Configuration d'une connexion, telle qu'elle est persistée dans le
/// workspace.
///
/// Les paramètres sont un [`IndexMap`] et non une `HashMap` : leur ordre est
/// celui que l'utilisateur a saisi, et il est conservé à la réécriture du
/// fichier. Un fichier de configuration qui se réordonne tout seul produit des
/// différences illisibles dans un dépôt.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    /// Identifiant interne. N'apparaît pas dans l'interface.
    pub id: ConnectionId,
    /// Nom donné par l'utilisateur. C'est **lui** qui est montré, y compris
    /// dans les demandes de confirmation.
    pub name: String,
    /// Driver, par protocole (ADR-0003).
    pub driver: DriverId,
    /// Environnement. Absent du fichier, il vaut
    /// [`Environment::Production`].
    #[serde(default)]
    pub environment: Environment,
    /// Ce qui a le droit de rejoindre une invite IA pour cette connexion.
    ///
    /// Absent du fichier, il vaut [`PrivacyTier::Metadata`] : le défaut
    /// d'ADR-0006, et jamais [`Sampled`](PrivacyTier::Sampled). Le sens de la
    /// prudence est celui d'[`environment`](Self::environment) — un champ
    /// manquant ne dégrade pas la protection.
    #[serde(default)]
    pub privacy_tier: PrivacyTier,
    /// Paramètres non secrets : hôte, port, base, schéma, mode TLS…
    ///
    /// Un mot de passe n'a rien à faire ici. Voir
    /// [`secret_ref`](Self::secret_ref).
    #[serde(default)]
    pub params: IndexMap<String, String>,
    /// Référence au secret dans le trousseau du système. Jamais le secret
    /// lui-même.
    #[serde(default)]
    pub secret_ref: Option<String>,
    /// La connexion est déclarée en lecture seule par l'utilisateur.
    ///
    /// Le `PolicyGate` en fait un **refus** de toute commande mutante, acteur
    /// humain compris : marquer une connexion en lecture seule est une
    /// déclaration d'intention, pas une préférence d'affichage.
    #[serde(default)]
    pub read_only: bool,
}

impl ConnectionConfig {
    /// Crée une configuration avec les défauts prudents : identifiant frais,
    /// environnement [`Production`](Environment::Production), niveau
    /// [`Metadata`](PrivacyTier::Metadata), aucun paramètre, aucun secret.
    #[must_use]
    pub fn new(name: impl Into<String>, driver: DriverId) -> Self {
        Self {
            id: ConnectionId::new(),
            name: name.into(),
            driver,
            environment: Environment::default(),
            privacy_tier: PrivacyTier::default(),
            params: IndexMap::new(),
            secret_ref: None,
            read_only: false,
        }
    }

    /// Fixe l'environnement.
    #[must_use]
    pub fn with_environment(mut self, environment: Environment) -> Self {
        self.environment = environment;
        self
    }

    /// Fixe le niveau de confidentialité de cette connexion.
    ///
    /// C'est un acte de l'utilisateur sur **une** connexion : il n'existe
    /// volontairement pas de chemin qui l'applique à plusieurs d'un coup
    /// (ADR-0006).
    #[must_use]
    pub fn with_privacy_tier(mut self, tier: PrivacyTier) -> Self {
        self.privacy_tier = tier;
        self
    }

    /// Ajoute un paramètre non secret.
    #[must_use]
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }

    /// Rattache une référence de secret.
    #[must_use]
    pub fn with_secret_ref(mut self, secret_ref: impl Into<String>) -> Self {
        self.secret_ref = Some(secret_ref.into());
        self
    }

    /// Marque la connexion en lecture seule.
    #[must_use]
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// La connexion vise-t-elle la production ?
    #[must_use]
    pub const fn is_production(&self) -> bool {
        self.environment.is_production()
    }
}

impl fmt::Debug for ConnectionConfig {
    /// Rendu volontairement partiel : les **valeurs** des paramètres et la
    /// référence de secret ne sont pas imprimées. Seules les clés le sont —
    /// savoir qu'un paramètre `host` existe est utile ; savoir lequel ne l'est
    /// pas dans un journal.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct ClesSeules<'a>(&'a IndexMap<String, String>);
        impl fmt::Debug for ClesSeules<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_map()
                    .entries(self.0.keys().map(|k| (k, "<redacted>")))
                    .finish()
            }
        }

        f.debug_struct("ConnectionConfig")
            .field("name", &self.name)
            .field("driver", &self.driver)
            .field("environment", &self.environment)
            // Le niveau est montrable, et il doit l'être : un incident se
            // diagnostique en sachant sous quel niveau la connexion tournait.
            .field("privacy_tier", &self.privacy_tier)
            .field("params", &ClesSeules(&self.params))
            .field(
                "secret_ref",
                &self.secret_ref.as_ref().map(|_| "<redacted reference>"),
            )
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_defaut_est_la_production() {
        // SECURITY : le défaut est la valeur la plus contraignante, pas la plus
        // permissive.
        assert_eq!(Environment::default(), Environment::Production);
        assert!(Environment::default().is_production());
        assert!(ConnectionConfig::new("sans environnement", DriverId::postgres()).is_production());
    }

    #[test]
    fn un_environnement_absent_du_fichier_vaut_production() {
        let json = r#"{
            "id": "018f0000-0000-7000-8000-000000000000",
            "name": "base client",
            "driver": "postgres"
        }"#;
        let cfg: ConnectionConfig = serde_json::from_str(json).expect("désérialisation");
        assert_eq!(
            cfg.environment,
            Environment::Production,
            "un champ manquant ne doit jamais dégrader la protection"
        );
        assert!(!cfg.read_only);
        assert!(cfg.params.is_empty());
    }

    #[test]
    fn une_connexion_sans_niveau_explicite_vaut_metadata() {
        // ADR-0006 : le défaut est sûr. Une connexion dont le niveau n'est pas
        // renseigné ne vaut **jamais** `Sampled` — c'est le sens de prudence
        // d'I-02 appliqué à la frontière IA.
        let cfg = ConnectionConfig::new("base client", DriverId::postgres());
        assert_eq!(cfg.privacy_tier, PrivacyTier::Metadata);
        assert!(!cfg.privacy_tier.allows_row_values());
    }

    #[test]
    fn un_niveau_absent_du_fichier_vaut_metadata() {
        // Le champ manquant est le cas réel : un workspace écrit avant que le
        // champ n'existe. Il ne doit pas se relire en `Sampled`.
        let json = r#"{
            "id": "018f0000-0000-7000-8000-000000000000",
            "name": "base client",
            "driver": "postgres"
        }"#;
        let cfg: ConnectionConfig = serde_json::from_str(json).expect("désérialisation");
        assert_eq!(cfg.privacy_tier, PrivacyTier::Metadata);
    }

    #[test]
    fn le_niveau_se_persiste_avec_la_connexion() {
        // Le niveau appartient à la connexion : l'aller-retour doit être exact,
        // sinon un workspace relu dégraderait — ou élargirait — la protection.
        for niveau in [
            PrivacyTier::Local,
            PrivacyTier::Metadata,
            PrivacyTier::Sampled,
        ] {
            let cfg =
                ConnectionConfig::new("bac à sable", DriverId::sqlite()).with_privacy_tier(niveau);
            let json = serde_json::to_string(&cfg).expect("sérialisation");
            let relu: ConnectionConfig = serde_json::from_str(&json).expect("désérialisation");
            assert_eq!(relu.privacy_tier, niveau, "{json}");
            assert_eq!(niveau.as_str().parse::<PrivacyTier>(), Ok(niveau));
        }
        assert!("confidentiel".parse::<PrivacyTier>().is_err());
    }

    #[test]
    fn l_ordre_des_niveaux_va_du_moins_au_plus_divulgant() {
        assert!(PrivacyTier::Local < PrivacyTier::Metadata);
        assert!(PrivacyTier::Metadata < PrivacyTier::Sampled);
        assert_eq!(
            PrivacyTier::Sampled.most_restrictive(PrivacyTier::Metadata),
            PrivacyTier::Metadata
        );
        assert_eq!(
            PrivacyTier::Metadata.most_restrictive(PrivacyTier::Local),
            PrivacyTier::Local
        );

        assert!(!PrivacyTier::Local.allows_row_values());
        assert!(!PrivacyTier::Metadata.allows_row_values());
        assert!(PrivacyTier::Sampled.allows_row_values());

        assert!(!PrivacyTier::Local.allows_remote_provider());
        assert!(PrivacyTier::Metadata.allows_remote_provider());
        assert!(PrivacyTier::Sampled.allows_remote_provider());
    }

    #[test]
    fn l_ordre_des_environnements_va_du_moins_au_plus_contraignant() {
        assert!(Environment::Local < Environment::Development);
        assert!(Environment::Development < Environment::Staging);
        assert!(Environment::Staging < Environment::Production);
        assert_eq!(
            Environment::Local.max(Environment::Production),
            Environment::Production
        );
    }

    #[test]
    fn analyse_et_rendu_des_environnements() {
        for env in [
            Environment::Local,
            Environment::Development,
            Environment::Staging,
            Environment::Production,
        ] {
            let relu: Environment = env.as_str().parse().expect("aller-retour");
            assert_eq!(env, relu);
        }
        assert_eq!(
            "prod".parse::<Environment>().expect("abréviation acceptée"),
            Environment::Production
        );
        assert!("recette".parse::<Environment>().is_err());
    }

    #[test]
    fn le_debug_ne_laisse_fuir_aucune_valeur_de_parametre() {
        let cfg = ConnectionConfig::new("prod-eu", DriverId::postgres())
            .with_param("host", "db.interne.example")
            .with_param("sslmode", "require")
            .with_secret_ref("oxyn/connexion/prod-eu");

        let rendu = format!("{cfg:?}");

        assert!(
            !rendu.contains("db.interne.example"),
            "hôte fuité : {rendu}"
        );
        assert!(!rendu.contains("require"), "valeur fuitée : {rendu}");
        assert!(
            !rendu.contains("oxyn/connexion/prod-eu"),
            "référence de secret fuitée : {rendu}"
        );
        assert!(
            !rendu.contains(&cfg.id.to_string()),
            "identifiant fuité : {rendu}"
        );

        // Ce qui reste doit rester utile au diagnostic.
        assert!(rendu.contains("prod-eu"), "le nom est montrable : {rendu}");
        assert!(rendu.contains("host"), "les clés sont montrables : {rendu}");
        assert!(rendu.contains("postgres"));
        // `Production` et non `production` : le `Debug` dérivé rend le nom de la
        // variante Rust. La minuscule est la forme serde
        // (`rename_all = "lowercase"`), qui ne vaut que pour ce qui est persisté.
        assert!(
            rendu.contains("Production"),
            "l'environnement doit rester lisible : {rendu}"
        );
    }

    #[test]
    fn ce_qui_est_persiste_ne_contient_qu_une_reference_de_secret() {
        // La seule voie pour un secret est `secret_ref`, et elle désigne une
        // entrée du trousseau — jamais la valeur. C'est ce qui évite qu'un
        // fichier de workspace commité dans un dépôt d'équipe emporte un mot de
        // passe de production.
        let cfg = ConnectionConfig::new("prod-eu", DriverId::postgres())
            .with_param("host", "db.interne.example")
            .with_secret_ref("oxyn/connexion/prod-eu");

        let json = serde_json::to_string(&cfg).expect("sérialisation");

        assert!(!json.contains("password"), "champ suspect : {json}");
        assert!(
            json.contains("oxyn/connexion/prod-eu"),
            "la référence est persistée"
        );

        let relu: ConnectionConfig = serde_json::from_str(&json).expect("désérialisation");
        assert_eq!(relu, cfg, "l'aller-retour doit être fidèle");
        assert_eq!(relu.secret_ref.as_deref(), Some("oxyn/connexion/prod-eu"));
    }

    #[test]
    fn l_ordre_des_parametres_est_conserve() {
        let cfg = ConnectionConfig::new("x", DriverId::postgres())
            .with_param("host", "h")
            .with_param("port", "5432")
            .with_param("dbname", "d");
        let cles: Vec<_> = cfg.params.keys().map(String::as_str).collect();
        assert_eq!(cles, ["host", "port", "dbname"]);
    }

    #[test]
    fn une_connexion_en_lecture_seule_se_declare() {
        let cfg = ConnectionConfig::new("réplica", DriverId::postgres()).read_only();
        assert!(cfg.read_only);
    }
}
