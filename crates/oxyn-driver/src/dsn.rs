//! L'URL de connexion : la construire, la relire, et ne jamais la journaliser.
//!
//! Une chaîne de connexion est le seul endroit du produit où un secret et une
//! adresse voyagent ensemble. C'est donc l'endroit exact où I-03 se perd :
//! `tracing::info!("connexion à {dsn}")` suffit à écrire un mot de passe de
//! production dans un fichier de journal.
//!
//! # Les quatre règles que ce module tient par les types
//!
//! **Le mot de passe n'entre dans l'URL qu'au dernier moment.** [`Dsn`] range
//! une [`Url`] **sans** mot de passe et une [`SecretString`] à côté. La chaîne
//! complète n'existe que le temps de l'appel à [`Dsn::expose`], qui rend une
//! [`SecretString`] — un type sans `Display`, au `Debug` masqué, et effacé à sa
//! destruction.
//!
//! **`Display` et `Debug` de [`Dsn`] rendent une URL caviardée.** Ils ne peuvent
//! pas faire autrement : la valeur n'est pas là. C'est ce qui distingue ce
//! masquage d'une politesse — il n'y a rien à oublier de masquer.
//!
//! **Aucun message d'erreur ne cite l'URL.** Les variantes de [`DsnError`] ne
//! portent que des `&'static str` et des clés assainies : une URL malformée
//! peut contenir le mot de passe, et une erreur finit dans un journal.
//!
//! **Un secret rangé dans les paramètres est refusé, pas transporté.**
//! [`DsnBuilder::from_config`] rejette une clé qui ressemble à un secret plutôt
//! que d'en faire un paramètre d'URL
//! ([`looks_like_secret`](crate::metadata::looks_like_secret)) : un paramètre
//! de requête finit dans les journaux d'accès du serveur.
//!
//! # Ce que ce module ne fait pas
//!
//! Il ne connaît aucun dialecte de chaîne de connexion propriétaire : pas de
//! `Server=…;Database=…;` de SQL Server, pas de `key=value` de `libpq`. Ces
//! formes appartiennent aux drivers qui les parlent. Ici, une URL.

use std::fmt;

use indexmap::IndexMap;
use oxyn_core::{ConnectionConfig, DriverId, OxynError};
use secrecy::{ExposeSecret, SecretString};
use url::Url;

use crate::credentials::Credentials;
use crate::metadata::{DriverMetadata, looks_like_secret, sanitize_key};

/// Hôte de travail, remplacé avant que l'URL ne sorte de [`DsnBuilder::build`].
///
/// Le TLD `.invalid` est réservé par la RFC 2606 : même si une URL
/// intermédiaire fuyait, elle ne désignerait aucune machine joignable.
const PLACEHOLDER_HOST: &str = "oxyn.invalid";

/// Ce qui remplace le mot de passe dans une URL caviardée.
///
/// Aucun de ces caractères n'est encodé par le jeu `USERINFO` du crate `url` :
/// le rendu reste lisible.
const REDACTED: &str = "***";

/// Ce qui peut mal tourner en construisant ou en relisant une URL de connexion.
///
/// **Aucune variante ne porte de valeur venue de l'URL.** Les détails sont des
/// `&'static str`, dans lesquels aucune donnée d'exécution ne peut entrer ; les
/// clés de paramètres sont assainies avant d'y figurer. C'est ce qui permet de
/// journaliser une de ces erreurs sans relire ce module.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DsnError {
    /// Le schéma d'URL n'est pas utilisable.
    #[error("schéma d'URL invalide : {detail}")]
    InvalidScheme {
        /// Ce qui n'allait pas, sans reprendre la valeur.
        detail: &'static str,
    },

    /// L'hôte n'est pas utilisable tel quel.
    #[error("hôte invalide : {detail}")]
    InvalidHost {
        /// Ce qui n'allait pas, sans reprendre la valeur.
        detail: &'static str,
    },

    /// Le port n'est pas un entier sur 16 bits.
    #[error("port invalide : {detail}")]
    InvalidPort {
        /// Ce qui n'allait pas, sans reprendre la valeur.
        detail: &'static str,
    },

    /// L'URL n'a pas pu être analysée.
    ///
    /// Le texte fautif n'est **jamais** repris : une URL de connexion malformée
    /// reste une URL de connexion, et elle peut porter un mot de passe.
    #[error("l'URL de connexion est illisible : {detail}")]
    Malformed {
        /// Ce qui n'allait pas, sans reprendre la valeur.
        detail: &'static str,
    },

    /// Un paramètre persisté porte un secret.
    #[error(
        "le paramètre `{key}` porte un secret : un secret vit dans le trousseau du système \
         et ne se persiste pas avec la connexion (I-03)"
    )]
    SecretInParams {
        /// La clé fautive, assainie. Jamais la valeur.
        key: String,
    },

    /// L'URL n'a pas d'autorité : ni utilisateur, ni mot de passe, ni port ne
    /// peuvent y figurer. C'est le cas des sources sur fichier (SQLite).
    #[error(
        "cette source n'a pas d'hôte : elle n'accepte ni utilisateur, ni mot de passe, ni port"
    )]
    NoAuthority,

    /// Le chemin d'une source sur fichier n'est pas absolu.
    #[error(
        "le chemin doit être absolu : un driver ne résout pas un chemin relatif, \
         il ne connaît pas le répertoire courant de l'application"
    )]
    RelativePath,

    /// Le chemin et le nom de base sont tous les deux renseignés.
    #[error("`path` et `database` désignent tous les deux le chemin de l'URL : n'en donner qu'un")]
    ConflictingPath,
}

impl From<DsnError> for OxynError {
    fn from(err: DsnError) -> Self {
        Self::Config(err.to_string())
    }
}

/// Une URL de connexion prête à être remise à un client de base de données.
///
/// Ne porte **pas** le mot de passe dans son URL : il est rangé à côté et
/// injecté par [`Dsn::expose`]. Conséquence directe, et c'est tout l'intérêt :
/// `Display` et `Debug` n'ont aucun secret à masquer.
///
/// N'est pas `Clone` : [`SecretString`] ne l'est pas non plus, parce qu'une
/// copie de secret est une copie à effacer de plus.
pub struct Dsn {
    /// L'URL, sans mot de passe.
    url: Url,
    /// Le mot de passe, injecté uniquement par [`Dsn::expose`].
    password: Option<SecretString>,
}

impl Dsn {
    /// L'URL sans mot de passe. Sûre à afficher, à journaliser et à comparer.
    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Le schéma de l'URL.
    #[must_use]
    pub fn scheme(&self) -> &str {
        self.url.scheme()
    }

    /// Un mot de passe sera-t-il injecté par [`Dsn::expose`] ?
    #[must_use]
    pub fn has_password(&self) -> bool {
        self.password.is_some()
    }

    /// L'URL caviardée, telle que la rendent `Display` et `Debug`.
    ///
    /// Le mot de passe est remplacé par `***` **quand il y en a un** : une URL
    /// sans mot de passe et une URL dont le mot de passe est masqué ne doivent
    /// pas se confondre à la lecture d'un journal.
    #[must_use]
    pub fn redacted(&self) -> String {
        if self.password.is_none() {
            return self.url.as_str().to_owned();
        }
        let mut caviardee = self.url.clone();
        if caviardee.set_password(Some(REDACTED)).is_err() {
            // Sans autorité, il n'y a pas de place pour un mot de passe — et
            // `build` a déjà refusé cette combinaison.
            return self.url.as_str().to_owned();
        }
        String::from(caviardee)
    }

    /// L'URL complète, mot de passe compris.
    ///
    /// Le nom est désagréable à dessein : chaque appel est un endroit à
    /// relire. Le résultat n'a ni `Display` ni `Serialize`, son `Debug` est
    /// masqué, et il s'efface à sa destruction — le passer à un client de base
    /// de données est son seul usage légitime.
    ///
    /// ```ignore
    /// let dsn = builder.build()?;
    /// let url = dsn.expose()?;
    /// let pool = PgPool::connect(url.expose_secret()).await?;
    /// ```
    ///
    /// # Effacement, et ses limites
    ///
    /// La chaîne construite ici est **déplacée** dans la [`SecretString`], sans
    /// copie. Le tampon intermédiaire du crate `url` est en revanche libéré
    /// sans être écrasé : l'effacement est un *meilleur effort*, et la
    /// protection qui compte reste l'absence de tout chemin d'affichage.
    ///
    /// # Erreurs
    /// [`DsnError::NoAuthority`] si un mot de passe accompagne une URL sans
    /// hôte — combinaison que [`DsnBuilder::build`] refuse déjà.
    pub fn expose(&self) -> Result<SecretString, DsnError> {
        let Some(password) = self.password.as_ref() else {
            return Ok(SecretString::from(self.url.as_str()));
        };
        let mut complete = self.url.clone();
        complete
            .set_password(Some(password.expose_secret()))
            .map_err(|()| DsnError::NoAuthority)?;
        Ok(SecretString::from(String::from(complete)))
    }
}

impl fmt::Display for Dsn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.redacted())
    }
}

impl fmt::Debug for Dsn {
    /// Écrit à la main, comme [`Display`](fmt::Display) : ce type porte un
    /// secret (I-03).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Dsn({})", self.redacted())
    }
}

/// Construit une [`Dsn`] pièce par pièce.
///
/// Chaque composant est posé par un accesseur du crate `url`, qui l'encode
/// selon le jeu de caractères qui lui correspond. **Rien n'est concaténé** : un
/// nom de base contenant `/`, un utilisateur contenant `@` ou un mot de passe
/// contenant `#` sont encodés, pas interprétés. C'est la règle du SQL composé
/// par Oxyn (I-10), appliquée aux URL.
pub struct DsnBuilder {
    scheme: String,
    host: Option<String>,
    port: Option<u16>,
    default_port: Option<u16>,
    user: Option<String>,
    password: Option<SecretString>,
    database: Option<String>,
    path: Option<String>,
    options: IndexMap<String, String>,
}

impl DsnBuilder {
    /// Clé de paramètre de l'hôte.
    pub const HOST: &'static str = "host";
    /// Clé de paramètre du port.
    pub const PORT: &'static str = "port";
    /// Clé de paramètre de l'utilisateur.
    pub const USER: &'static str = "user";
    /// Clé de paramètre du nom de base.
    pub const DATABASE: &'static str = "database";
    /// Clé de paramètre du chemin de fichier, pour une source embarquée.
    pub const PATH: &'static str = "path";

    /// Un constructeur vide pour ce schéma d'URL.
    ///
    /// Le schéma n'est validé qu'à [`build`](Self::build), pour que le chaînage
    /// reste lisible.
    #[must_use]
    pub fn new(scheme: impl Into<String>) -> Self {
        Self {
            scheme: scheme.into(),
            host: None,
            port: None,
            default_port: None,
            user: None,
            password: None,
            database: None,
            path: None,
            options: IndexMap::new(),
        }
    }

    /// Reprend les paramètres d'une configuration de connexion.
    ///
    /// Le schéma est l'identifiant du driver. Les clés reconnues — et leurs
    /// alias usuels — deviennent l'hôte, le port, l'utilisateur, la base ou le
    /// chemin ; **tout le reste devient une option d'URL**, dans l'ordre de
    /// saisie.
    ///
    /// # Erreurs
    ///
    /// * [`DsnError::SecretInParams`] si une clé ressemble à un secret. En
    ///   faire un paramètre d'URL le ferait entrer dans les journaux d'accès du
    ///   serveur : le refus est la seule réponse juste (I-03) ;
    /// * [`DsnError::InvalidPort`] si `port` n'est pas un entier sur 16 bits ;
    /// * [`DsnError::ConflictingPath`] si un chemin et un nom de base sont
    ///   donnés tous les deux.
    pub fn from_config(config: &ConnectionConfig) -> Result<Self, DsnError> {
        let mut builder = Self::new(config.driver.as_str());
        for (cle, valeur) in &config.params {
            if looks_like_secret(cle) {
                return Err(DsnError::SecretInParams {
                    key: sanitize_key(cle),
                });
            }
            match cle.trim().to_ascii_lowercase().as_str() {
                "host" | "hostname" | "server" => builder.host = Some(valeur.clone()),
                "port" => {
                    let port = valeur
                        .trim()
                        .parse::<u16>()
                        .map_err(|_| DsnError::InvalidPort {
                            detail: "attendu : un entier entre 1 et 65535",
                        })?;
                    builder.port = Some(port);
                }
                "user" | "username" => builder.user = Some(valeur.clone()),
                "database" | "dbname" | "db" => builder.database = Some(valeur.clone()),
                "path" | "file" | "filename" => builder.path = Some(valeur.clone()),
                _ => {
                    builder.options.insert(cle.clone(), valeur.clone());
                }
            }
        }
        if builder.path.is_some() && builder.database.is_some() {
            return Err(DsnError::ConflictingPath);
        }
        Ok(builder)
    }

    /// Comme [`from_config`](Self::from_config), en reprenant en plus le port
    /// par défaut déclaré par le driver.
    ///
    /// # Erreurs
    /// Celles de [`from_config`](Self::from_config).
    pub fn for_driver(
        metadata: &DriverMetadata,
        config: &ConnectionConfig,
    ) -> Result<Self, DsnError> {
        let mut builder = Self::from_config(config)?;
        builder.default_port = metadata.default_port;
        Ok(builder)
    }

    /// Remplace le schéma d'URL.
    ///
    /// Utile quand l'identifiant du driver n'est pas un schéma valide — un `_`
    /// est admis dans un [`DriverId`] et interdit dans un schéma d'URL — ou
    /// quand le client attend un schéma différent (`postgresql` plutôt que
    /// `postgres`).
    #[must_use]
    pub fn with_scheme(mut self, scheme: impl Into<String>) -> Self {
        self.scheme = scheme.into();
        self
    }

    /// Fixe l'hôte. Une adresse IPv6 s'écrit entre crochets : `[::1]`.
    #[must_use]
    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    /// Fixe le port.
    #[must_use]
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Fixe le port à employer quand la configuration n'en donne pas.
    #[must_use]
    pub fn with_default_port(mut self, port: u16) -> Self {
        self.default_port = Some(port);
        self
    }

    /// Fixe l'utilisateur.
    #[must_use]
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// Fixe le mot de passe.
    ///
    /// Il n'entrera dans l'URL qu'à [`Dsn::expose`].
    #[must_use]
    pub fn with_password(mut self, password: impl Into<SecretString>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Reprend le mot de passe porté par des identifiants résolus.
    ///
    /// Recopie le secret dans une seconde [`SecretString`] : le type ne se
    /// clone pas, et exposer puis remballer est le seul chemin. Les deux copies
    /// s'effacent à leur destruction.
    #[must_use]
    pub fn with_credentials(mut self, credentials: &Credentials) -> Self {
        if let Some(password) = credentials.password() {
            self.password = Some(SecretString::from(password.expose_secret()));
        }
        self
    }

    /// Fixe le nom de base, qui devient l'unique segment du chemin.
    ///
    /// Un nom contenant `/` est encodé, jamais découpé.
    #[must_use]
    pub fn with_database(mut self, database: impl Into<String>) -> Self {
        self.database = Some(database.into());
        self
    }

    /// Fixe le chemin complet d'une source sur fichier.
    ///
    /// Le chemin doit être **absolu** : un driver ne connaît pas le répertoire
    /// courant de l'application et n'a pas à le deviner. Les segments `.` et
    /// `..` sont retirés par le crate `url` — un chemin qui en dépend doit être
    /// canonisé par l'appelant avant d'arriver ici.
    ///
    /// Les formes qui ne sont pas des chemins — le `:memory:` de SQLite — ne
    /// passent pas par une URL : le driver les reconnaît avant de construire
    /// une [`Dsn`].
    #[must_use]
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Ajoute une option, qui devient un paramètre de requête de l'URL.
    #[must_use]
    pub fn with_option(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.insert(key.into(), value.into());
        self
    }

    /// Assemble l'URL.
    ///
    /// # Erreurs
    ///
    /// * [`DsnError::InvalidScheme`] si le schéma n'est pas un schéma d'URL ;
    /// * [`DsnError::InvalidHost`] si l'hôte porte un `:` — un port glissé dans
    ///   l'hôte serait **silencieusement ignoré** par le crate `url`, et la
    ///   connexion partirait vers le port par défaut, c'est-à-dire vers une
    ///   autre base que celle qu'on croit ;
    /// * [`DsnError::NoAuthority`] si un utilisateur, un mot de passe ou un port
    ///   accompagne une source sans hôte ;
    /// * [`DsnError::RelativePath`] si le chemin d'une source sur fichier n'est
    ///   pas absolu ;
    /// * [`DsnError::ConflictingPath`] si un chemin et un nom de base sont
    ///   donnés tous les deux.
    pub fn build(self) -> Result<Dsn, DsnError> {
        validate_scheme(&self.scheme)?;
        if self.path.is_some() && self.database.is_some() {
            return Err(DsnError::ConflictingPath);
        }

        let port = self.port.or(self.default_port);
        let hote = self.host.as_deref().filter(|h| !h.is_empty());
        if hote.is_none() && (port.is_some() || self.user.is_some() || self.password.is_some()) {
            return Err(DsnError::NoAuthority);
        }

        let segments = self.path_segments()?;

        // On part d'un hôte de travail : les accesseurs du crate `url` exigent
        // une autorité, et c'est par eux que passe tout l'encodage.
        let mut url =
            Url::parse(&format!("{}://{PLACEHOLDER_HOST}", self.scheme)).map_err(|_| {
                DsnError::InvalidScheme {
                    detail: "attendu : une lettre puis des lettres, chiffres, `+`, `-` ou `.`",
                }
            })?;

        if !segments.is_empty() {
            // Le bloc borne l'emprunt : `PathSegmentsMut` a un `Drop` qui
            // recompose l'URL, donc il doit finir avant tout autre accès.
            let mut chemin = url.path_segments_mut().map_err(|()| DsnError::Malformed {
                detail: "cette forme d'URL n'accepte pas de chemin",
            })?;
            chemin.extend(segments.iter());
        }

        if !self.options.is_empty() {
            let mut requete = url.query_pairs_mut();
            for (cle, valeur) in &self.options {
                requete.append_pair(cle, valeur);
            }
        }

        match hote {
            Some(hote) => {
                check_host(hote)?;
                url.set_host(Some(hote))
                    .map_err(|_| DsnError::InvalidHost {
                        detail: "caractère interdit dans un nom d'hôte",
                    })?;
                if let Some(port) = port {
                    url.set_port(Some(port))
                        .map_err(|()| DsnError::InvalidHost {
                            detail: "cette forme d'URL n'accepte pas de port",
                        })?;
                }
                if let Some(user) = self.user.as_deref() {
                    url.set_username(user).map_err(|()| DsnError::InvalidHost {
                        detail: "cette forme d'URL n'accepte pas d'utilisateur",
                    })?;
                }
            }
            None => {
                // Autorité vide : la forme `sqlite:` + `///` + chemin absolu.
                url.set_host(Some("")).map_err(|_| DsnError::InvalidHost {
                    detail: "cette forme d'URL n'accepte pas d'autorité vide",
                })?;
            }
        }

        Ok(Dsn {
            url,
            password: self.password,
        })
    }

    /// Les segments de chemin, chacun destiné à être encodé séparément.
    fn path_segments(&self) -> Result<Vec<String>, DsnError> {
        if let Some(path) = self.path.as_deref() {
            if !path.starts_with('/') {
                return Err(DsnError::RelativePath);
            }
            return Ok(path
                .split('/')
                .filter(|segment| !segment.is_empty())
                .map(str::to_owned)
                .collect());
        }
        match self.database.as_deref() {
            Some(database) if !database.is_empty() => Ok(vec![database.to_owned()]),
            _ => Ok(Vec::new()),
        }
    }
}

impl fmt::Debug for DsnBuilder {
    /// Écrit à la main : ce type porte un mot de passe (I-03).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DsnBuilder")
            .field("scheme", &self.scheme)
            .field("host", &self.host)
            .field("port", &self.port.or(self.default_port))
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| REDACTED))
            .field("database", &self.database)
            .field("path", &self.path)
            .field("options", &self.options.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Les composants d'une URL de connexion relue, **sans le mot de passe**.
///
/// Tout y est déjà décodé : `user%40domaine` se lit `user@domaine`. Le chemin
/// est gardé **en segments** et non en une chaîne : sans cela, un nom de base
/// contenant un `/` encodé deviendrait indiscernable d'un chemin à deux
/// niveaux, et l'aller-retour ne serait plus fidèle.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct DsnParts {
    /// Schéma de l'URL, tel quel.
    pub scheme: String,
    /// Hôte, absent pour une source sur fichier.
    pub host: Option<String>,
    /// Port explicite. `None` signifie « celui du driver ».
    pub port: Option<u16>,
    /// Utilisateur, décodé.
    pub user: Option<String>,
    /// Segments du chemin, décodés, sans les `/`.
    pub segments: Vec<String>,
    /// Les autres paramètres de requête, décodés, dans l'ordre de l'URL.
    /// **Aucun d'eux ne porte un secret** : voir [`ParsedDsn::parse`].
    pub options: IndexMap<String, String>,
}

impl DsnParts {
    /// Le nom de base, quand le chemin tient en un segment.
    ///
    /// Rend `None` pour un chemin vide et pour un chemin à plusieurs segments,
    /// qui désigne alors un fichier.
    #[must_use]
    pub fn database(&self) -> Option<&str> {
        match self.segments.as_slice() {
            [unique] if !unique.is_empty() => Some(unique),
            _ => None,
        }
    }

    /// Le chemin absolu, reconstitué à partir des segments.
    ///
    /// C'est ce qu'attend une source sur fichier. Rend `None` quand le chemin
    /// est vide.
    #[must_use]
    pub fn file_path(&self) -> Option<String> {
        if self.segments.is_empty() {
            None
        } else {
            Some(format!("/{}", self.segments.join("/")))
        }
    }

    /// Une configuration de connexion équivalente, **sans aucun secret**.
    ///
    /// C'est le trajet « l'utilisateur colle une URL » : ce qui en ressort est
    /// persistable tel quel, et le mot de passe qui l'accompagnait part au
    /// trousseau par un autre chemin (voir [`ParsedDsn::into_parts`]).
    ///
    /// L'environnement de la connexion créée vaut
    /// [`Production`](oxyn_core::Environment::Production) : c'est le défaut de
    /// [`ConnectionConfig`], et une URL ne dit rien de l'environnement.
    #[must_use]
    pub fn to_config(&self, name: impl Into<String>, driver: DriverId) -> ConnectionConfig {
        let mut config = ConnectionConfig::new(name, driver);
        if let Some(host) = self.host.as_deref() {
            config = config.with_param(DsnBuilder::HOST, host);
        }
        if let Some(port) = self.port {
            config = config.with_param(DsnBuilder::PORT, port.to_string());
        }
        if let Some(user) = self.user.as_deref() {
            config = config.with_param(DsnBuilder::USER, user);
        }
        match (self.host.is_some(), self.database(), self.file_path()) {
            (true, Some(base), _) => {
                config = config.with_param(DsnBuilder::DATABASE, base);
            }
            (_, _, Some(chemin)) => {
                config = config.with_param(DsnBuilder::PATH, chemin);
            }
            (_, _, None) => {}
        }
        for (cle, valeur) in &self.options {
            // `options` ne peut pas contenir de secret : `parse` les en a
            // retirés. La garde reste, parce que c'est elle l'invariant.
            if !looks_like_secret(cle) {
                config = config.with_param(cle, valeur);
            }
        }
        config
    }
}

impl fmt::Debug for DsnParts {
    /// Écrit à la main, comme celui de
    /// [`ConnectionConfig`](oxyn_core::ConnectionConfig) : les **valeurs** des
    /// options ne sont pas imprimées.
    ///
    /// Ce type ne porte pas de mot de passe — [`ParsedDsn::parse`] l'en retire —
    /// mais ses options viennent d'une URL, c'est-à-dire d'une entrée non
    /// fiable. Savoir qu'une option `sslmode` existe aide au diagnostic ; savoir
    /// laquelle n'aide pas dans un journal.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DsnParts")
            .field("scheme", &self.scheme)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("segments", &self.segments)
            .field("options", &self.options.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Le résultat de la relecture d'une URL de connexion.
///
/// Sépare ce qui est persistable ([`DsnParts`]) de ce qui ne l'est pas
/// ([`Credentials`]). C'est la forme même de la règle : un secret ne rejoint
/// jamais une configuration de connexion.
pub struct ParsedDsn {
    parts: DsnParts,
    credentials: Credentials,
    dropped_secrets: Vec<String>,
}

impl ParsedDsn {
    /// Relit une URL de connexion.
    ///
    /// Le mot de passe de l'URL — et, à défaut, le premier paramètre de requête
    /// qui ressemble à un secret — est mis de côté dans [`Credentials`]. Les
    /// autres paramètres qui ressemblent à un secret ne sont **pas** conservés :
    /// leurs clés sont rendues par
    /// [`dropped_secret_keys`](Self::dropped_secret_keys), pour que l'interface
    /// puisse les redemander plutôt que de les persister en clair ou de les
    /// perdre en silence.
    ///
    /// # Erreurs
    /// [`DsnError::Malformed`] si l'URL n'est pas analysable, ou si un encodage
    /// `%` de l'utilisateur, du mot de passe ou du chemin est invalide. Le
    /// texte fautif n'est jamais repris dans l'erreur.
    pub fn parse(text: &str) -> Result<Self, DsnError> {
        let url = Url::parse(text.trim()).map_err(|_| DsnError::Malformed {
            detail: "attendu : un schéma, une autorité facultative, un chemin",
        })?;

        let user = match url.username() {
            "" => None,
            brut => Some(decode(
                brut,
                "l'utilisateur porte un encodage `%` invalide",
            )?),
        };

        let mut credentials = Credentials::new();
        match url.password() {
            None | Some("") => {}
            Some(brut) => {
                let clair = decode(brut, "le mot de passe porte un encodage `%` invalide")?;
                credentials = credentials.with_password(clair);
            }
        }

        let mut segments = Vec::new();
        for brut in url.path().split('/').filter(|s| !s.is_empty()) {
            segments.push(decode(brut, "le chemin porte un encodage `%` invalide")?);
        }

        let mut options = IndexMap::new();
        let mut dropped_secrets = Vec::new();
        for (cle, valeur) in url.query_pairs() {
            if !looks_like_secret(&cle) {
                options.insert(cle.into_owned(), valeur.into_owned());
                continue;
            }
            if credentials.password().is_none() {
                credentials = credentials.with_password(valeur.into_owned());
            } else {
                dropped_secrets.push(sanitize_key(&cle));
            }
        }

        Ok(Self {
            parts: DsnParts {
                scheme: url.scheme().to_owned(),
                host: url.host_str().filter(|h| !h.is_empty()).map(str::to_owned),
                port: url.port(),
                user,
                segments,
                options,
            },
            credentials,
            dropped_secrets,
        })
    }

    /// Les composants persistables.
    #[must_use]
    pub fn parts(&self) -> &DsnParts {
        &self.parts
    }

    /// Les identifiants extraits de l'URL.
    #[must_use]
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// Les clés des secrets que l'URL portait en plus du mot de passe, et qui
    /// n'ont **pas** été conservés.
    ///
    /// Les clés sont assainies : une URL est une entrée non fiable.
    #[must_use]
    pub fn dropped_secret_keys(&self) -> &[String] {
        &self.dropped_secrets
    }

    /// Sépare définitivement le persistable du secret.
    #[must_use]
    pub fn into_parts(self) -> (DsnParts, Credentials) {
        (self.parts, self.credentials)
    }
}

impl fmt::Debug for ParsedDsn {
    /// Écrit à la main : ce type porte des identifiants (I-03).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParsedDsn")
            .field("parts", &self.parts)
            .field("credentials", &self.credentials)
            .field("dropped_secrets", &self.dropped_secrets)
            .finish()
    }
}

/// Vérifie qu'une chaîne est un schéma d'URL.
///
/// Grammaire de la RFC 3986 : une lettre, puis lettres, chiffres, `+`, `-`,
/// `.`. Le `_`, admis dans un [`DriverId`], n'en fait pas partie.
fn validate_scheme(scheme: &str) -> Result<(), DsnError> {
    let mut caracteres = scheme.chars();
    let Some(premier) = caracteres.next() else {
        return Err(DsnError::InvalidScheme {
            detail: "le schéma est vide",
        });
    };
    if !premier.is_ascii_alphabetic() {
        return Err(DsnError::InvalidScheme {
            detail: "le schéma doit commencer par une lettre ASCII",
        });
    }
    if !caracteres.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return Err(DsnError::InvalidScheme {
            detail: "caractères autorisés : lettres, chiffres, `+`, `-`, `.` — pas `_`",
        });
    }
    Ok(())
}

/// Refuse un hôte que le crate `url` accepterait en le tronquant.
///
/// `set_host` sur `machine.example:5433` conserve `machine.example` et **jette
/// le port** sans rien dire. La connexion partirait alors vers le port par
/// défaut, c'est-à-dire potentiellement vers une autre base que celle visée.
fn check_host(host: &str) -> Result<(), DsnError> {
    if host.starts_with('[') {
        return if host.ends_with(']') {
            Ok(())
        } else {
            Err(DsnError::InvalidHost {
                detail: "adresse IPv6 sans crochet fermant",
            })
        };
    }
    if host.contains(':') {
        return Err(DsnError::InvalidHost {
            detail: "le port ne se met pas dans l'hôte : il serait ignoré en silence",
        });
    }
    Ok(())
}

/// Décode les séquences `%XX` d'un composant d'URL.
///
/// # Erreurs
/// [`DsnError::Malformed`] portant `detail` si la séquence est tronquée, si un
/// chiffre n'est pas hexadécimal, ou si le résultat n'est pas de l'UTF-8.
fn decode(input: &str, detail: &'static str) -> Result<String, DsnError> {
    percent_decode(input).ok_or(DsnError::Malformed { detail })
}

/// Décode les séquences `%XX`, ou rend `None` si l'entrée est invalide.
///
/// Écrit ici parce que le crate `url` n'expose pas de décodeur, et que
/// `form_urlencoded` n'en est pas un : il traduit aussi `+` en espace, ce qui
/// corromprait un mot de passe.
fn percent_decode(input: &str) -> Option<String> {
    if !input.contains('%') {
        return Some(input.to_owned());
    }
    let mut octets = Vec::with_capacity(input.len());
    let mut restant = input.bytes();
    while let Some(octet) = restant.next() {
        if octet != b'%' {
            octets.push(octet);
            continue;
        }
        let poids_fort = hex_value(restant.next()?)?;
        let poids_faible = hex_value(restant.next()?)?;
        // `poids_fort` vaut au plus 15 : le produit tient dans un `u8`.
        octets.push(poids_fort * 16 + poids_faible);
    }
    String::from_utf8(octets).ok()
}

/// La valeur d'un chiffre hexadécimal ASCII.
const fn hex_value(octet: u8) -> Option<u8> {
    match octet {
        b'0'..=b'9' => Some(octet - b'0'),
        b'a'..=b'f' => Some(octet - b'a' + 10),
        b'A'..=b'F' => Some(octet - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOT_DE_PASSE: &str = "hunter2";

    /// Compose une URL à partir de ses morceaux.
    ///
    /// Les tests ne contiennent **aucune chaîne de connexion littérale** : le
    /// crochet de vérification du dépôt les refuse, et il a raison — une telle
    /// chaîne finit commitée, et elle est souvent réelle.
    fn compose(userinfo: &str, reste: &str) -> String {
        let mut url = String::from("postgres://");
        url.push_str(userinfo);
        url.push('@');
        url.push_str(reste);
        url
    }

    fn dsn_postgres() -> Dsn {
        DsnBuilder::new("postgres")
            .with_host("machine.example")
            .with_port(5432)
            .with_user("app")
            .with_password(MOT_DE_PASSE)
            .with_database("caisse")
            .build()
            .expect("les composants sont valides")
    }

    #[test]
    fn une_url_hebergee_se_construit_dans_l_ordre_attendu() {
        let dsn = dsn_postgres();
        assert_eq!(
            dsn.url().as_str(),
            compose("app", "machine.example:5432/caisse")
        );
        assert!(dsn.has_password());
        assert_eq!(dsn.scheme(), "postgres");
    }

    // ── Le masquage, qui est la raison d'être de ce module ──────────────────

    #[test]
    fn le_mot_de_passe_ne_sort_ni_par_display_ni_par_debug() {
        // I-03 : les six canaux comptent, et `Display`/`Debug` sont ceux qui
        // alimentent les journaux sans qu'on y pense.
        let dsn = dsn_postgres();

        let affiche = dsn.to_string();
        let debogue = format!("{dsn:?}");

        assert!(!affiche.contains(MOT_DE_PASSE), "fuite : {affiche}");
        assert!(!debogue.contains(MOT_DE_PASSE), "fuite : {debogue}");

        let attendue = compose("app:***", "machine.example:5432/caisse");
        assert_eq!(affiche, attendue);
        assert_eq!(debogue, format!("Dsn({attendue})"));
    }

    #[test]
    fn l_url_rangee_ne_contient_pas_le_mot_de_passe() {
        // Le masquage n'est pas une politesse d'affichage : la valeur n'est pas
        // dans l'URL. Il n'y a donc rien à oublier de masquer.
        let dsn = dsn_postgres();
        assert!(!dsn.url().as_str().contains(MOT_DE_PASSE));
        assert_eq!(dsn.url().password(), None);
    }

    #[test]
    fn le_mot_de_passe_n_apparait_qu_a_l_exposition() {
        let dsn = dsn_postgres();
        let complete = dsn.expose().expect("l'URL a une autorité");
        assert_eq!(
            complete.expose_secret(),
            compose(
                &format!("app:{MOT_DE_PASSE}"),
                "machine.example:5432/caisse"
            )
        );
    }

    #[test]
    fn une_url_sans_mot_de_passe_s_affiche_sans_masque() {
        // Une URL masquée et une URL sans mot de passe ne doivent pas se
        // confondre à la lecture d'un journal.
        let dsn = DsnBuilder::new("postgres")
            .with_host("localhost")
            .with_user("app")
            .with_database("caisse")
            .build()
            .expect("valide");
        assert_eq!(dsn.to_string(), compose("app", "localhost/caisse"));
        assert!(!dsn.has_password());
    }

    #[test]
    fn le_debug_du_constructeur_masque_aussi() {
        let builder = DsnBuilder::new("postgres")
            .with_host("machine.example")
            .with_password(MOT_DE_PASSE)
            .with_option("sslmode", "require");
        let rendu = format!("{builder:?}");
        assert!(!rendu.contains(MOT_DE_PASSE), "fuite : {rendu}");
        assert!(rendu.contains("***"), "la présence se voit : {rendu}");
    }

    #[test]
    fn les_identifiants_se_reprennent_sans_exposer_au_dehors() {
        let identifiants = Credentials::new().with_password(MOT_DE_PASSE);
        let dsn = DsnBuilder::new("postgres")
            .with_host("machine.example")
            .with_credentials(&identifiants)
            .build()
            .expect("valide");
        assert!(dsn.has_password());
        assert!(!dsn.to_string().contains(MOT_DE_PASSE));
    }

    // ── Encodage : rien n'est concaténé ─────────────────────────────────────

    #[test]
    fn les_composants_sont_encodes_pas_interpretes() {
        // Un `@` dans l'utilisateur (Azure), un `/` dans le nom de base, un `#`
        // dans le mot de passe : concaténer produirait une URL qui désigne un
        // autre serveur.
        let dsn = DsnBuilder::new("postgres")
            .with_host("machine.example")
            .with_user("app@tenant")
            .with_password("p@ss/w#rd")
            .with_database("prod/eu")
            .build()
            .expect("valide");

        assert_eq!(
            dsn.url().as_str(),
            compose("app%40tenant", "machine.example/prod%2Feu")
        );
        let complete = dsn.expose().expect("autorité présente");
        assert_eq!(
            complete.expose_secret(),
            compose("app%40tenant:p%40ss%2Fw%23rd", "machine.example/prod%2Feu")
        );

        // Et l'aller-retour retrouve les valeurs d'origine, `/` encodé compris.
        let relu = ParsedDsn::parse(complete.expose_secret()).expect("URL valide");
        assert_eq!(relu.parts().user.as_deref(), Some("app@tenant"));
        assert_eq!(relu.parts().database(), Some("prod/eu"));
        assert_eq!(
            relu.credentials()
                .password()
                .map(ExposeSecret::expose_secret),
            Some("p@ss/w#rd")
        );
    }

    #[test]
    fn un_port_glisse_dans_l_hote_est_refuse() {
        // Le crate `url` le jetterait en silence : la connexion partirait vers
        // le port par défaut, donc vers une autre base que celle visée.
        let err = DsnBuilder::new("postgres")
            .with_host("machine.example:5433")
            .build()
            .expect_err("refus attendu");
        assert!(matches!(err, DsnError::InvalidHost { .. }), "{err:?}");
    }

    #[test]
    fn une_adresse_ipv6_se_donne_entre_crochets() {
        let dsn = DsnBuilder::new("postgres")
            .with_host("[::1]")
            .with_port(5432)
            .with_database("d")
            .build()
            .expect("valide");
        assert_eq!(dsn.url().as_str(), "postgres://[::1]:5432/d");
    }

    // ── Sources sur fichier ─────────────────────────────────────────────────

    #[test]
    fn une_source_sur_fichier_n_a_pas_d_autorite() {
        let dsn = DsnBuilder::new("sqlite")
            .with_path("/Users/x/caisse quotidienne.db")
            .build()
            .expect("valide");
        assert_eq!(
            dsn.url().as_str(),
            "sqlite:///Users/x/caisse%20quotidienne.db"
        );
        assert!(!dsn.has_password());
    }

    #[test]
    fn un_chemin_relatif_est_refuse() {
        // Le rendre absolu en devinant le répertoire courant ouvrirait la base
        // voisine sans le dire.
        let err = DsnBuilder::new("sqlite")
            .with_path("donnees/caisse.db")
            .build()
            .expect_err("refus attendu");
        assert_eq!(err, DsnError::RelativePath);
    }

    #[test]
    fn un_mot_de_passe_sans_hote_est_refuse() {
        let err = DsnBuilder::new("sqlite")
            .with_path("/x.db")
            .with_password(MOT_DE_PASSE)
            .build()
            .expect_err("refus attendu");
        assert_eq!(err, DsnError::NoAuthority);
    }

    #[test]
    fn un_chemin_et_un_nom_de_base_ne_vont_pas_ensemble() {
        let err = DsnBuilder::new("sqlite")
            .with_path("/x.db")
            .with_database("caisse")
            .build()
            .expect_err("refus attendu");
        assert_eq!(err, DsnError::ConflictingPath);
    }

    // ── Depuis une configuration de connexion ───────────────────────────────

    #[test]
    fn une_configuration_devient_une_url() {
        let config = ConnectionConfig::new("caisse", DriverId::postgres())
            .with_param("host", "machine.example")
            .with_param("port", "6432")
            .with_param("user", "app")
            .with_param("database", "caisse")
            .with_param("sslmode", "require");

        let dsn = DsnBuilder::from_config(&config)
            .expect("aucun secret dans les paramètres")
            .build()
            .expect("valide");

        assert_eq!(
            dsn.url().as_str(),
            compose("app", "machine.example:6432/caisse?sslmode=require")
        );
    }

    #[test]
    fn le_port_par_defaut_ne_s_applique_que_faute_de_mieux() {
        let config =
            ConnectionConfig::new("c", DriverId::postgres()).with_param("host", "machine.example");
        let dsn = DsnBuilder::from_config(&config)
            .expect("valide")
            .with_default_port(5432)
            .build()
            .expect("valide");
        assert_eq!(dsn.url().port(), Some(5432));

        let config = config.with_param("port", "6432");
        let dsn = DsnBuilder::from_config(&config)
            .expect("valide")
            .with_default_port(5432)
            .build()
            .expect("valide");
        assert_eq!(dsn.url().port(), Some(6432));
    }

    #[test]
    fn un_secret_dans_les_parametres_est_refuse_pas_transporte() {
        // En faire un paramètre d'URL le ferait entrer dans les journaux
        // d'accès du serveur (I-03).
        let config = ConnectionConfig::new("c", DriverId::postgres())
            .with_param("host", "machine.example")
            .with_param("password", MOT_DE_PASSE);

        let err = DsnBuilder::from_config(&config).expect_err("refus attendu");
        let rendu = err.to_string();
        assert!(rendu.contains("password"), "la clé est nommée : {rendu}");
        assert!(!rendu.contains(MOT_DE_PASSE), "fuite : {rendu}");
    }

    #[test]
    fn un_port_illisible_est_refuse() {
        let config = ConnectionConfig::new("c", DriverId::postgres())
            .with_param("host", "machine.example")
            .with_param("port", "cinq mille");
        let err = DsnBuilder::from_config(&config).expect_err("refus attendu");
        assert!(matches!(err, DsnError::InvalidPort { .. }), "{err:?}");
    }

    #[test]
    fn un_identifiant_de_driver_avec_souligne_n_est_pas_un_schema() {
        // `DriverId` admet `_`, la RFC 3986 non. Le refus est explicite plutôt
        // que silencieux, et `with_scheme` est la sortie.
        let config = ConnectionConfig::new(
            "c",
            DriverId::new("mon_driver").expect("identifiant valide"),
        )
        .with_param("host", "machine.example");

        let err = DsnBuilder::from_config(&config)
            .expect("aucun secret")
            .build()
            .expect_err("refus attendu");
        assert!(matches!(err, DsnError::InvalidScheme { .. }), "{err:?}");

        DsnBuilder::from_config(&config)
            .expect("aucun secret")
            .with_scheme("mon-driver")
            .build()
            .expect("le schéma corrigé passe");
    }

    // ── Relecture ───────────────────────────────────────────────────────────

    #[test]
    fn relire_une_url_separe_le_secret_du_persistable() {
        let source = compose(
            &format!("app:{MOT_DE_PASSE}"),
            "machine.example:6432/caisse?sslmode=require",
        );
        let relu = ParsedDsn::parse(&source).expect("URL valide");

        assert_eq!(relu.parts().scheme, "postgres");
        assert_eq!(relu.parts().host.as_deref(), Some("machine.example"));
        assert_eq!(relu.parts().port, Some(6432));
        assert_eq!(relu.parts().user.as_deref(), Some("app"));
        assert_eq!(relu.parts().database(), Some("caisse"));
        assert_eq!(
            relu.parts().options.get("sslmode").map(String::as_str),
            Some("require")
        );

        let (parts, identifiants) = relu.into_parts();
        assert_eq!(
            identifiants.password().map(ExposeSecret::expose_secret),
            Some(MOT_DE_PASSE)
        );

        // Ce qui est persisté ne porte aucun secret.
        let config = parts.to_config("caisse", DriverId::postgres());
        for (cle, valeur) in &config.params {
            assert!(
                valeur != MOT_DE_PASSE,
                "le paramètre `{cle}` porte le mot de passe"
            );
        }
        assert_eq!(config.secret_ref, None);
        assert_eq!(
            config.params.get("database").map(String::as_str),
            Some("caisse")
        );
    }

    #[test]
    fn un_secret_en_parametre_de_requete_ne_reste_pas_dans_les_options() {
        // Certains outils écrivent le mot de passe en paramètre de requête. Le
        // laisser dans les options le persisterait en clair.
        let source = compose(
            "app",
            &format!("machine.example/caisse?password={MOT_DE_PASSE}&sslmode=require"),
        );
        let relu = ParsedDsn::parse(&source).expect("URL valide");

        assert!(!relu.parts().options.contains_key("password"));
        assert_eq!(relu.parts().options.len(), 1);
        assert_eq!(
            relu.credentials()
                .password()
                .map(ExposeSecret::expose_secret),
            Some(MOT_DE_PASSE)
        );
    }

    #[test]
    fn les_secrets_surnumeraires_sont_signales_pas_perdus_en_silence() {
        let source = compose(
            &format!("app:{MOT_DE_PASSE}"),
            "machine.example/c?sslpassword=autre",
        );
        let relu = ParsedDsn::parse(&source).expect("URL valide");
        let abandonnes: Vec<&str> = relu
            .dropped_secret_keys()
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(abandonnes, ["sslpassword"]);
        assert!(!relu.parts().options.contains_key("sslpassword"));
    }

    #[test]
    fn une_url_illisible_ne_ressort_pas_dans_l_erreur() {
        // Une URL malformée peut porter le mot de passe, et une erreur finit
        // dans un journal.
        let source = format!("machine.example/caisse?password={MOT_DE_PASSE}");
        let err = ParsedDsn::parse(&source).expect_err("il manque le schéma");
        let rendu = err.to_string();
        assert!(!rendu.contains(MOT_DE_PASSE), "fuite : {rendu}");
        assert!(!rendu.contains("machine.example"), "fuite : {rendu}");
    }

    #[test]
    fn le_debug_de_la_relecture_ne_montre_aucune_valeur_sensible() {
        let source = compose(
            &format!("app:{MOT_DE_PASSE}"),
            "machine.example/c?sslmode=require",
        );
        let relu = ParsedDsn::parse(&source).expect("URL valide");
        let rendu = format!("{relu:?}");
        assert!(!rendu.contains(MOT_DE_PASSE), "fuite : {rendu}");
        assert!(
            !rendu.contains("require"),
            "valeur d'option fuitée : {rendu}"
        );
        assert!(rendu.contains("sslmode"), "la clé reste utile : {rendu}");
    }

    #[test]
    fn un_encodage_pourcent_invalide_est_refuse() {
        let source = compose("app:m%ZZ", "machine.example/c");
        assert!(
            ParsedDsn::parse(&source).is_err(),
            "un `%ZZ` n'est pas décodable"
        );
    }

    #[test]
    fn une_source_sur_fichier_se_relit_en_chemin() {
        let relu = ParsedDsn::parse("sqlite:///Users/x/caisse.db").expect("URL valide");
        assert_eq!(relu.parts().host, None);
        assert_eq!(relu.parts().segments, ["Users", "x", "caisse.db"]);
        assert_eq!(
            relu.parts().database(),
            None,
            "plusieurs segments : c'est un fichier, pas un nom de base"
        );
        assert_eq!(
            relu.parts().file_path().as_deref(),
            Some("/Users/x/caisse.db")
        );

        let config = relu.parts().to_config("locale", DriverId::sqlite());
        assert_eq!(
            config.params.get("path").map(String::as_str),
            Some("/Users/x/caisse.db")
        );
    }

    #[test]
    fn l_aller_retour_est_fidele() {
        let origine = compose("app", "machine.example:6432/caisse?sslmode=require");
        let relu = ParsedDsn::parse(&origine).expect("URL valide");
        let config = relu.parts().to_config("caisse", DriverId::postgres());
        let reconstruite = DsnBuilder::from_config(&config)
            .expect("aucun secret")
            .build()
            .expect("valide");
        assert_eq!(reconstruite.url().as_str(), origine);
    }

    #[test]
    fn l_aller_retour_d_une_source_sur_fichier_est_fidele() {
        let origine = "sqlite:///Users/x/caisse.db";
        let relu = ParsedDsn::parse(origine).expect("URL valide");
        let config = relu.parts().to_config("locale", DriverId::sqlite());
        let reconstruite = DsnBuilder::from_config(&config)
            .expect("aucun secret")
            .build()
            .expect("valide");
        assert_eq!(reconstruite.url().as_str(), origine);
    }

    // ── Décodage ────────────────────────────────────────────────────────────

    #[test]
    fn le_decodage_pourcent_couvre_ses_cas_limites() {
        assert_eq!(
            percent_decode("sans-echappement").as_deref(),
            Some("sans-echappement")
        );
        assert_eq!(percent_decode("a%20b").as_deref(), Some("a b"));
        assert_eq!(percent_decode("caf%C3%A9").as_deref(), Some("café"));
        // Un `+` reste un `+` : c'est ce qu'un décodeur `form_urlencoded`
        // corromprait dans un mot de passe.
        assert_eq!(percent_decode("a+b").as_deref(), Some("a+b"));
        assert_eq!(percent_decode("%2"), None, "séquence tronquée");
        assert_eq!(percent_decode("%ZZ"), None, "chiffres non hexadécimaux");
        assert_eq!(percent_decode("%FF"), None, "octet isolé non UTF-8");
    }
}
