//! La référence à un secret, et le contrat que tout trousseau respecte.
//!
//! Le partage des rôles est celui de
//! [`SECURITY`](../../../docs/SECURITY.md) et de
//! [`ARCHITECTURE` §8](../../../docs/ARCHITECTURE.md) :
//!
//! * ce qui est **persisté** dans un fichier de workspace est une
//!   [`SecretRef`] — une chaîne stable, publique, dérivée de l'identifiant de
//!   connexion ;
//! * ce qui est **stocké** dans le trousseau du système est la valeur, sous
//!   cette référence.
//!
//! La panne évitée est concrète : un fichier de workspace contenant un mot de
//! passe de production, commité par l'utilisateur dans le dépôt de son équipe
//! parce que le fichier avait l'air d'être une simple configuration.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use oxyn_core::{ConnectionConfig, ConnectionId};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::bundle::CredentialBundle;
use crate::error::{Result, SecretError};

/// Désignation stable d'un secret dans le trousseau du système.
///
/// Forme canonique : `oxyn:<genre>:<nom>`, par exemple
/// `oxyn:conn:018f0000-0000-7000-8000-000000000000`. Le préfixe `oxyn`
/// cantonne les entrées écrites par Oxyn ; le genre dit à quoi le secret se
/// rattache ; le nom identifie l'objet.
///
/// # Une référence n'est pas un secret
///
/// Son `Debug` est **complet**, et c'est délibéré : la référence est écrite en
/// clair dans les fichiers de workspace, elle est faite pour être vue. Masquer
/// ce qui n'est pas secret dilue le signal des masques qui comptent — ceux de
/// [`CredentialBundle`] et de
/// [`oxyn_core::ConnectionConfig`].
///
/// # Pourquoi la validation est stricte
///
/// Une référence relue depuis un fichier de workspace devient un **nom de
/// compte dans le trousseau du système**. Un fichier de workspace est une
/// entrée non fiable ([`SECURITY`](../../../docs/SECURITY.md), surface
/// d'entrée n° 3) : il peut avoir été écrit par un tiers. Laisser passer un
/// octet de contrôle, un espace ou un `:` supplémentaire, c'est laisser une
/// chaîne étrangère décider de quelle entrée du trousseau Oxyn va lire.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SecretRef(Arc<str>);

impl SecretRef {
    /// Préfixe de toutes les entrées écrites par Oxyn.
    pub const SCHEME: &'static str = "oxyn";
    /// Genre des secrets rattachés à une connexion de base de données.
    pub const KIND_CONNECTION: &'static str = "conn";
    /// Genre des secrets rattachés à un fournisseur de modèles (phase 2).
    pub const KIND_PROVIDER: &'static str = "llm";

    /// Longueur maximale de la référence complète.
    const MAX_LEN: usize = 160;
    /// Longueur maximale du segment de genre.
    const MAX_KIND_LEN: usize = 16;
    /// Longueur maximale du segment de nom.
    const MAX_NAME_LEN: usize = 128;

    /// Référence du secret d'une connexion, dérivée de son identifiant.
    ///
    /// La dérivation est totale et déterministe : deux appels sur le même
    /// [`ConnectionId`] rendent la même référence, y compris après
    /// redémarrage. C'est ce qui permet de retrouver le mot de passe d'une
    /// connexion dont le fichier n'a jamais porté que l'identifiant.
    #[must_use]
    pub fn for_connection(id: ConnectionId) -> Self {
        Self(Arc::from(format!(
            "{}:{}:{id}",
            Self::SCHEME,
            Self::KIND_CONNECTION
        )))
    }

    /// Référence du secret d'un fournisseur de modèles.
    ///
    /// # Erreurs
    /// Renvoie [`SecretError::InvalidReference`] si le nom du fournisseur ne
    /// respecte pas les règles de nommage d'un segment.
    pub fn for_provider(provider: impl AsRef<str>) -> Result<Self> {
        let provider = provider.as_ref();
        validate_name(provider)?;
        Ok(Self(Arc::from(format!(
            "{}:{}:{provider}",
            Self::SCHEME,
            Self::KIND_PROVIDER
        ))))
    }

    /// Référence à interroger pour une connexion donnée.
    ///
    /// Si la configuration porte déjà une
    /// [`secret_ref`](ConnectionConfig::secret_ref), c'est elle qui fait foi —
    /// après validation, parce qu'elle vient d'un fichier. Sinon, la référence
    /// est dérivée de l'identifiant de la connexion.
    ///
    /// # Erreurs
    /// Renvoie [`SecretError::InvalidReference`] si la référence écrite dans le
    /// fichier est malformée. On ne se rabat **pas** silencieusement sur la
    /// référence dérivée dans ce cas : lire un autre secret que celui demandé
    /// serait pire que ne rien lire.
    pub fn for_connection_config(config: &ConnectionConfig) -> Result<Self> {
        match config.secret_ref.as_deref() {
            Some(existing) => Self::parse(existing),
            None => Ok(Self::for_connection(config.id)),
        }
    }

    /// Analyse une référence relue depuis un fichier de workspace.
    ///
    /// # Erreurs
    /// Renvoie [`SecretError::InvalidReference`] si la chaîne n'est pas de la
    /// forme `oxyn:<genre>:<nom>` avec des segments normalisés. Le message ne
    /// recopie jamais la valeur fautive.
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() > Self::MAX_LEN {
            return Err(invalid("reference too long"));
        }
        let mut segments = text.split(':');
        let (Some(scheme), Some(kind), Some(name), None) = (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
        ) else {
            return Err(invalid("expected form: oxyn:<kind>:<name>"));
        };
        if scheme != Self::SCHEME {
            return Err(invalid("the prefix must be `oxyn`"));
        }
        validate_kind(kind)?;
        validate_name(name)?;
        Ok(Self(Arc::from(text)))
    }

    /// Vue empruntée de la référence complète.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Segment de genre (`conn`, `llm`…).
    #[must_use]
    pub fn kind(&self) -> &str {
        self.segment(1)
    }

    /// Segment de nom : l'identifiant de connexion, le nom de fournisseur…
    #[must_use]
    pub fn name(&self) -> &str {
        self.segment(2)
    }

    /// La référence désigne-t-elle le secret d'une connexion ?
    #[must_use]
    pub fn is_connection(&self) -> bool {
        self.kind() == Self::KIND_CONNECTION
    }

    /// Segment d'indice `index`, la forme étant garantie par la construction.
    fn segment(&self, index: usize) -> &str {
        // Toute `SecretRef` a traversé `parse` ou une des fabriques : elle a
        // donc exactement trois segments. Rendre `""` plutôt que paniquer si
        // cet invariant venait à être rompu.
        self.0.split(':').nth(index).unwrap_or_default()
    }
}

/// Construit une erreur de référence.
fn invalid(detail: &'static str) -> SecretError {
    SecretError::InvalidReference { detail }
}

/// Valide un segment de genre : minuscules ASCII, chiffres, `-` et `_`.
fn validate_kind(kind: &str) -> Result<()> {
    if kind.is_empty() {
        return Err(invalid("the kind is empty"));
    }
    if kind.len() > SecretRef::MAX_KIND_LEN {
        return Err(invalid("kind too long"));
    }
    if !kind.starts_with(|c: char| c.is_ascii_lowercase()) {
        return Err(invalid("the kind must start with a lowercase letter"));
    }
    if !kind
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(invalid("allowed characters in the kind: a-z, 0-9, -, _"));
    }
    Ok(())
}

/// Valide un segment de nom.
///
/// Le jeu autorisé est volontairement étroit : ce segment devient un nom de
/// compte dans le trousseau du système, et il faut qu'il soit impossible d'y
/// glisser un espace, un octet de contrôle ou un séparateur.
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(invalid("the name is empty"));
    }
    if name.len() > SecretRef::MAX_NAME_LEN {
        return Err(invalid("name too long"));
    }
    // Le premier caractère est alphanumérique, ce qui exclut `.` et `..` : une
    // référence finit un jour dans un nom de fichier de cache, et un nom qui
    // désigne un répertoire parent y serait une remontée d'arborescence.
    if !name.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return Err(invalid("the name must start with a letter or a digit"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(invalid(
            "allowed characters in the name: A-Z, a-z, 0-9, -, _, .",
        ));
    }
    Ok(())
}

impl fmt::Debug for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretRef({:?})", self.as_str())
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for SecretRef {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for SecretRef {
    type Err = SecretError;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

impl TryFrom<String> for SecretRef {
    type Error = SecretError;

    fn try_from(value: String) -> Result<Self> {
        Self::parse(&value)
    }
}

impl From<SecretRef> for String {
    fn from(reference: SecretRef) -> Self {
        reference.as_str().to_owned()
    }
}

/// Ce qu'Oxyn attend d'un trousseau.
///
/// L'API est **synchrone** : un accès au trousseau du système est un appel
/// local de l'ordre de la milliseconde, et l'envelopper dans `async` ferait
/// payer un `spawn_blocking` à chaque lecture pour ne rien résoudre. Les
/// implémentations restent néanmoins `Send + Sync` : elles sont appelées depuis
/// le runtime Tokio, jamais depuis le thread d'interface (I-05).
///
/// # Le `Debug` fait partie du contrat
///
/// Le trait exige `Debug` pour qu'une structure qui tient un
/// `Arc<dyn SecretStore>` puisse en dériver un — et il impose du même coup à
/// chaque implémentation d'écrire un `Debug` qui ne montre **aucune valeur
/// stockée**. Une implémentation qui imprimerait ses entrées viole I-03.
///
/// # Idempotence
///
/// [`delete`](Self::delete) réussit lorsque la référence est inconnue :
/// supprimer ce qui n'existe pas est l'état recherché, pas une panne. C'est ce
/// qui permet de supprimer une connexion sans savoir si elle avait un mot de
/// passe.
pub trait SecretStore: fmt::Debug + Send + Sync {
    /// Écrit — ou remplace — le secret désigné.
    ///
    /// # Erreurs
    /// Voir [`SecretError`] : trousseau indisponible, accès refusé, valeur trop
    /// grande pour la plateforme.
    fn put(&self, reference: &SecretRef, secret: SecretString) -> Result<()>;

    /// Relit le secret désigné, ou `None` si la référence est inconnue.
    ///
    /// Une référence inconnue n'est pas une erreur : une connexion peut n'avoir
    /// aucun secret (SQLite sur fichier, authentification par socket Unix,
    /// `~/.pgpass`).
    ///
    /// # Erreurs
    /// Voir [`SecretError`]. En particulier
    /// [`Malformed`](SecretError::Malformed) si le trousseau rend autre chose
    /// que ce qui y avait été écrit.
    fn get(&self, reference: &SecretRef) -> Result<Option<SecretString>>;

    /// Supprime le secret désigné. Réussit si la référence est déjà inconnue.
    ///
    /// # Erreurs
    /// Voir [`SecretError`].
    fn delete(&self, reference: &SecretRef) -> Result<()>;

    /// Écrit un jeu d'identifiants complet, encodé en JSON.
    ///
    /// C'est **le** chemin d'écriture pour tout ce qui n'est pas un simple mot
    /// de passe : l'enveloppe JSON vit ici et nulle part ailleurs, pour qu'une
    /// crate appelante n'invente pas son propre format.
    ///
    /// # Erreurs
    /// Voir [`SecretError`].
    fn put_bundle(&self, reference: &SecretRef, bundle: &CredentialBundle) -> Result<()> {
        self.put(reference, bundle.to_secret_json()?)
    }

    /// Relit un jeu d'identifiants complet.
    ///
    /// # Erreurs
    /// [`Malformed`](SecretError::Malformed) si l'entrée du trousseau existe
    /// mais n'est pas un bundle — typiquement un mot de passe nu écrit par une
    /// version antérieure, ou par un autre outil sous la même référence.
    fn get_bundle(&self, reference: &SecretRef) -> Result<Option<CredentialBundle>> {
        match self.get(reference)? {
            Some(json) => CredentialBundle::from_secret_json(&json).map(Some),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_store::MemorySecretStore;
    use oxyn_core::DriverId;

    #[test]
    fn la_reference_d_une_connexion_est_stable() {
        let id = ConnectionId::new();
        assert_eq!(SecretRef::for_connection(id), SecretRef::for_connection(id));
        assert_ne!(
            SecretRef::for_connection(id),
            SecretRef::for_connection(ConnectionId::new())
        );

        let reference = SecretRef::for_connection(id);
        assert!(reference.as_str().starts_with("oxyn:conn:"));
        assert_eq!(reference.kind(), SecretRef::KIND_CONNECTION);
        assert_eq!(reference.name(), id.to_string());
        assert!(reference.is_connection());
    }

    #[test]
    fn une_connexion_sans_reference_ecrite_derive_la_sienne() {
        let cfg = ConnectionConfig::new("base client", DriverId::postgres());
        let reference = SecretRef::for_connection_config(&cfg).expect("dérivation");
        assert_eq!(reference, SecretRef::for_connection(cfg.id));
    }

    #[test]
    fn une_reference_ecrite_fait_foi() {
        let cfg = ConnectionConfig::new("base client", DriverId::postgres())
            .with_secret_ref("oxyn:conn:heritee-de-la-v0");
        let reference = SecretRef::for_connection_config(&cfg).expect("analyse");
        assert_eq!(reference.name(), "heritee-de-la-v0");
        assert_ne!(reference, SecretRef::for_connection(cfg.id));
    }

    #[test]
    fn une_reference_ecrite_malformee_est_un_refus_pas_un_repli() {
        // Se rabattre sur la référence dérivée lirait *un autre* secret que
        // celui que le fichier désigne. Ne rien lire est moins grave.
        let cfg = ConnectionConfig::new("base client", DriverId::postgres())
            .with_secret_ref("oxyn/connexion/prod-eu");
        assert!(SecretRef::for_connection_config(&cfg).is_err());
    }

    #[test]
    fn le_format_canonique_fait_l_aller_retour() {
        for texte in [
            "oxyn:conn:018f0000-0000-7000-8000-000000000000",
            "oxyn:llm:anthropic",
            "oxyn:llm:azure.openai",
            "oxyn:tunnel_ssh:bastion-eu",
        ] {
            let reference = SecretRef::parse(texte).expect(texte);
            assert_eq!(reference.as_str(), texte);
            assert_eq!(reference.to_string(), texte);
            let relu: SecretRef = texte.parse().expect("FromStr");
            assert_eq!(reference, relu);
        }
    }

    #[test]
    fn une_reference_venue_d_un_fichier_ne_peut_pas_designer_ce_qu_elle_veut() {
        // Chacune de ces valeurs est plausible dans un fichier de workspace
        // écrit par un tiers. Toutes doivent être refusées avant d'atteindre le
        // trousseau du système.
        for texte in [
            "",
            "oxyn",
            "oxyn:conn",
            "oxyn:conn:",
            "oxyn::nom",
            "autre:conn:nom",
            "oxyn:conn:nom:supplement",
            "oxyn:conn:nom avec espace",
            "oxyn:conn:nom\nligne2",
            "oxyn:conn:nom\u{0}",
            "oxyn:conn:../../autre",
            "oxyn:conn:..",
            "oxyn:conn:.",
            "oxyn:conn:-nom",
            "oxyn:CONN:nom",
            "oxyn:1conn:nom",
            "oxyn:conn:nom/chemin",
            "oxyn:conn:nom;rm -rf",
        ] {
            assert!(
                SecretRef::parse(texte).is_err(),
                "{texte:?} aurait dû être refusé"
            );
        }
        assert!(SecretRef::parse(&format!("oxyn:conn:{}", "a".repeat(129))).is_err());
    }

    #[test]
    fn un_refus_ne_recopie_pas_la_valeur_fautive() {
        let err =
            SecretRef::parse("oxyn:conn:base-de-la-banque centrale").expect_err("espace interdit");
        let message = err.to_string();
        assert!(!message.contains("banque"), "valeur fuitée : {message}");
    }

    #[test]
    fn le_nom_de_fournisseur_est_valide_a_la_construction() {
        assert!(SecretRef::for_provider("anthropic").is_ok());
        assert!(SecretRef::for_provider("").is_err());
        assert!(SecretRef::for_provider("open ai").is_err());
        assert!(SecretRef::for_provider("openai:prod").is_err());
    }

    #[test]
    fn la_reference_traverse_le_json_avec_sa_validation() {
        let reference = SecretRef::for_provider("ollama").expect("valide");
        let json = serde_json::to_string(&reference).expect("sérialisation");
        assert_eq!(json, "\"oxyn:llm:ollama\"");
        let relu: SecretRef = serde_json::from_str(&json).expect("désérialisation");
        assert_eq!(relu, reference);
        assert!(
            serde_json::from_str::<SecretRef>("\"oxyn:llm:oll ama\"").is_err(),
            "la validation doit s'appliquer aussi à la désérialisation"
        );
    }

    #[test]
    fn le_debug_d_une_reference_est_complet() {
        // Une référence est publique : elle est écrite en clair dans les
        // fichiers de workspace. La masquer n'apporterait rien et brouillerait
        // le sens des masques qui comptent.
        let reference = SecretRef::for_provider("anthropic").expect("valide");
        assert!(format!("{reference:?}").contains("oxyn:llm:anthropic"));
    }

    #[test]
    fn les_methodes_de_bundle_passent_par_l_enveloppe_json() {
        let store = MemorySecretStore::new();
        let reference = SecretRef::for_connection(ConnectionId::new());

        assert!(store.get_bundle(&reference).expect("lecture").is_none());

        let bundle = CredentialBundle::new()
            .with_password("hunter2")
            .with_token("sk-témoin");
        store.put_bundle(&reference, &bundle).expect("écriture");

        let relu = store
            .get_bundle(&reference)
            .expect("lecture")
            .expect("le bundle vient d'être écrit");
        assert_eq!(relu.password(), Some("hunter2"));
        assert_eq!(relu.token(), Some("sk-témoin"));
    }

    #[test]
    fn un_mot_de_passe_nu_ne_se_relit_pas_comme_un_bundle() {
        // Cas réel : une entrée écrite par une version antérieure, ou par un
        // autre outil, sous la même référence.
        let store = MemorySecretStore::new();
        let reference = SecretRef::for_connection(ConnectionId::new());
        store
            .put(&reference, SecretString::from("hunter2"))
            .expect("écriture");

        let err = store
            .get_bundle(&reference)
            .expect_err("ce n'est pas du JSON");
        assert!(matches!(err, SecretError::Malformed { .. }));
        assert!(
            !err.to_string().contains("hunter2"),
            "le contenu a fuité dans l'erreur : {err}"
        );
    }
}
