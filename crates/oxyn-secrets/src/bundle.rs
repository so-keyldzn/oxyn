//! Le jeu d'identifiants d'une connexion.
//!
//! Un trousseau de système d'exploitation ne stocke qu'**une** valeur par
//! entrée. Or une connexion peut demander un mot de passe *et* un certificat
//! client *et* la phrase de passe d'une clé SSH de tunnel. [`CredentialBundle`]
//! rassemble ces valeurs et les encode en JSON avant stockage : une entrée de
//! trousseau, un secret logique.
//!
//! Trois propriétés sont tenues par le type, pas par la discipline de
//! l'appelant :
//!
//! 1. **Aucun `Debug` dérivé.** Le `Debug` est écrit à la main et rend
//!    `<redacted>`. C'est le corollaire vérifiable de I-03 : c'est le
//!    `tracing::debug!("{bundle:?}")` ajouté six mois plus tard qui fuit.
//! 2. **Effacement à la destruction.** [`Zeroize`] est implémenté champ par
//!    champ, et [`Drop`] l'appelle. Un mot de passe libéré sans effacement reste
//!    lisible dans le tas jusqu'à réutilisation de la page.
//! 3. **Pas de `Clone`.** Copier un jeu d'identifiants multiplie les tampons à
//!    effacer sans qu'aucun appelant en ait besoin. Ce qui doit voyager, c'est
//!    la [`SecretRef`](crate::SecretRef).

use std::collections::BTreeMap;
use std::fmt;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{Result, SecretError};

/// Ce qu'il faut fournir à un serveur pour ouvrir une session.
///
/// Tous les champs sont facultatifs : une connexion SQLite sur fichier n'en
/// renseigne aucun, une connexion PostgreSQL par mot de passe un seul, une
/// connexion mutuellement authentifiée trois.
///
/// # Sérialisation
///
/// Le JSON produit n'écrit que les champs renseignés, et la relecture tolère
/// les champs inconnus : un workspace écrit par une version ultérieure d'Oxyn
/// reste lisible, quitte à ignorer ce qu'il apporte.
#[derive(Default, Serialize, Deserialize)]
pub struct CredentialBundle {
    /// Mot de passe du compte de base de données.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    password: Option<String>,

    /// Jeton d'authentification : jeton d'API d'un fournisseur de modèles,
    /// jeton porteur, jeton de session d'un fournisseur cloud.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,

    /// Certificat client TLS, au format PEM.
    ///
    /// Le certificat n'est pas secret en lui-même, mais il ne sert à rien sans
    /// sa clé et il voyage avec elle : le séparer ne ferait qu'ajouter une
    /// entrée de trousseau à gérer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tls_client_cert: Option<String>,

    /// Clé privée du certificat client TLS, au format PEM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tls_client_key: Option<String>,

    /// Clé privée SSH d'un tunnel, au format PEM ou OpenSSH.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ssh_private_key: Option<String>,

    /// Phrase de passe protégeant la clé SSH.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ssh_passphrase: Option<String>,

    /// Secrets propres à un driver, que le modèle commun ne prévoit pas :
    /// jeton de session AWS, compte de service BigQuery, clé privée Snowflake.
    ///
    /// La clé est un nom court choisi par le driver et documenté par lui ; elle
    /// n'est pas un secret. La valeur en est un, et elle est effacée comme les
    /// autres.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    extra: BTreeMap<String, String>,
}

impl CredentialBundle {
    /// Un jeu vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Renseigne le mot de passe.
    #[must_use]
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Renseigne le jeton d'authentification.
    #[must_use]
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Renseigne le certificat client TLS (PEM).
    #[must_use]
    pub fn with_tls_client_cert(mut self, pem: impl Into<String>) -> Self {
        self.tls_client_cert = Some(pem.into());
        self
    }

    /// Renseigne la clé privée du certificat client TLS (PEM).
    #[must_use]
    pub fn with_tls_client_key(mut self, pem: impl Into<String>) -> Self {
        self.tls_client_key = Some(pem.into());
        self
    }

    /// Renseigne la clé privée SSH du tunnel.
    #[must_use]
    pub fn with_ssh_private_key(mut self, pem: impl Into<String>) -> Self {
        self.ssh_private_key = Some(pem.into());
        self
    }

    /// Renseigne la phrase de passe de la clé SSH.
    #[must_use]
    pub fn with_ssh_passphrase(mut self, passphrase: impl Into<String>) -> Self {
        self.ssh_passphrase = Some(passphrase.into());
        self
    }

    /// Ajoute un secret propre à un driver.
    #[must_use]
    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }

    /// Expose le mot de passe.
    ///
    /// Comme tous les accesseurs de ce type, la valeur rendue est le secret en
    /// clair : elle se transmet au driver et s'oublie. Elle ne se journalise
    /// pas, ne se met pas en cache, ne rejoint pas une invite IA (I-03, I-04).
    #[must_use]
    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    /// Expose le jeton d'authentification. Mêmes précautions que
    /// [`password`](Self::password).
    #[must_use]
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Expose le certificat client TLS. Mêmes précautions que
    /// [`password`](Self::password).
    #[must_use]
    pub fn tls_client_cert(&self) -> Option<&str> {
        self.tls_client_cert.as_deref()
    }

    /// Expose la clé privée TLS. Mêmes précautions que
    /// [`password`](Self::password).
    #[must_use]
    pub fn tls_client_key(&self) -> Option<&str> {
        self.tls_client_key.as_deref()
    }

    /// Expose la clé privée SSH. Mêmes précautions que
    /// [`password`](Self::password).
    #[must_use]
    pub fn ssh_private_key(&self) -> Option<&str> {
        self.ssh_private_key.as_deref()
    }

    /// Expose la phrase de passe SSH. Mêmes précautions que
    /// [`password`](Self::password).
    #[must_use]
    pub fn ssh_passphrase(&self) -> Option<&str> {
        self.ssh_passphrase.as_deref()
    }

    /// Expose un secret propre à un driver. Mêmes précautions que
    /// [`password`](Self::password).
    #[must_use]
    pub fn extra(&self, key: &str) -> Option<&str> {
        self.extra.get(key).map(String::as_str)
    }

    /// Le jeu ne contient-il aucun secret ?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.password.is_none()
            && self.token.is_none()
            && self.tls_client_cert.is_none()
            && self.tls_client_key.is_none()
            && self.ssh_private_key.is_none()
            && self.ssh_passphrase.is_none()
            && self.extra.is_empty()
    }

    /// Noms des champs renseignés, pour le diagnostic.
    ///
    /// C'est l'échappatoire explicite au `Debug` masqué : savoir *qu'un* mot de
    /// passe est présent aide à comprendre un échec d'authentification, et ne
    /// dit rien de sa valeur. Les clés de [`extra`](Self::with_extra) sont
    /// rendues telles quelles — ce sont des noms choisis par un driver, pas des
    /// secrets.
    #[must_use]
    pub fn filled_fields(&self) -> Vec<&str> {
        let mut noms = Vec::new();
        for (present, nom) in [
            (self.password.is_some(), "password"),
            (self.token.is_some(), "token"),
            (self.tls_client_cert.is_some(), "tls_client_cert"),
            (self.tls_client_key.is_some(), "tls_client_key"),
            (self.ssh_private_key.is_some(), "ssh_private_key"),
            (self.ssh_passphrase.is_some(), "ssh_passphrase"),
        ] {
            if present {
                noms.push(nom);
            }
        }
        noms.extend(self.extra.keys().map(String::as_str));
        noms
    }

    /// Encode le jeu en JSON, enveloppé dans un [`SecretString`].
    ///
    /// # Erreurs
    /// [`SecretError::Malformed`] si l'encodage échoue — ce qui, sur une
    /// structure de chaînes, signalerait un bug de `serde_json` plutôt qu'une
    /// donnée fautive.
    pub fn to_secret_json(&self) -> Result<SecretString> {
        let mut json = serde_json::to_string(self).map_err(|_| SecretError::Malformed {
            detail: SecretError::NOT_ENCODABLE,
        })?;

        // `SecretString::from(&str)` alloue exactement la longueur nécessaire,
        // donc la conversion en `Box<str>` qu'il fait ensuite ne réalloue pas
        // et ne laisse pas de copie derrière elle. Le tampon de `serde_json`,
        // lui, a une capacité quelconque : on l'efface explicitement, sinon le
        // JSON en clair survivrait dans le tas après libération.
        let secret = SecretString::from(json.as_str());
        json.zeroize();
        Ok(secret)
    }

    /// Décode un jeu depuis le JSON relu dans le trousseau.
    ///
    /// # Erreurs
    /// [`SecretError::Malformed`] si le contenu n'est pas un bundle. L'erreur
    /// de `serde_json` est **abandonnée**, sans être ni rendue ni journalisée :
    /// son message cite volontiers un fragment de son entrée, qui est ici le
    /// secret lui-même.
    pub fn from_secret_json(secret: &SecretString) -> Result<Self> {
        serde_json::from_str(secret.expose_secret()).map_err(|_| SecretError::Malformed {
            detail: SecretError::NOT_A_BUNDLE,
        })
    }
}

impl fmt::Debug for CredentialBundle {
    /// Rendu total : `CredentialBundle(<redacted>)`.
    ///
    /// Pas même la liste des champs renseignés — un `Debug` est appelé par des
    /// chemins qu'on ne relit pas (un `#[derive(Debug)]` d'une structure
    /// englobante, une macro `tracing`), et sa sortie atterrit dans des canaux
    /// qu'on ne choisit pas. Ce qu'on veut montrer volontairement passe par
    /// [`filled_fields`](Self::filled_fields).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CredentialBundle(<redacted>)")
    }
}

impl Zeroize for CredentialBundle {
    fn zeroize(&mut self) {
        self.password.zeroize();
        self.token.zeroize();
        self.tls_client_cert.zeroize();
        self.tls_client_key.zeroize();
        self.ssh_private_key.zeroize();
        self.ssh_passphrase.zeroize();
        // `BTreeMap` n'a pas d'implémentation de `Zeroize` : on efface chaque
        // valeur sur place avant de vider la structure, sinon `clear()` se
        // contenterait de libérer des tampons encore lisibles.
        for valeur in self.extra.values_mut() {
            valeur.zeroize();
        }
        self.extra.clear();
    }
}

impl Drop for CredentialBundle {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl ZeroizeOnDrop for CredentialBundle {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_debug_ne_montre_rien() {
        let bundle = CredentialBundle::new()
            .with_password("hunter2")
            .with_token("sk-ant-secret")
            .with_ssh_passphrase("phrase de passe");

        let rendu = format!("{bundle:?}");
        assert_eq!(rendu, "CredentialBundle(<redacted>)");
        for secret in ["hunter2", "sk-ant-secret", "phrase de passe"] {
            assert!(!rendu.contains(secret), "secret fuité : {rendu}");
        }
    }

    #[test]
    fn le_debug_d_une_structure_englobante_ne_montre_rien_non_plus() {
        // C'est le vrai chemin de fuite : personne n'écrit `{bundle:?}` ; on
        // dérive `Debug` sur une structure qui en contient un.
        #[derive(Debug)]
        struct Englobante {
            #[allow(dead_code)]
            nom: &'static str,
            #[allow(dead_code)]
            identifiants: CredentialBundle,
        }

        let englobante = Englobante {
            nom: "prod-eu",
            identifiants: CredentialBundle::new().with_password("hunter2"),
        };
        let rendu = format!("{englobante:?}");
        assert!(!rendu.contains("hunter2"), "secret fuité : {rendu}");
        assert!(rendu.contains("prod-eu"));
    }

    #[test]
    fn les_champs_renseignes_se_nomment_sans_se_montrer() {
        let bundle = CredentialBundle::new()
            .with_password("hunter2")
            .with_extra("aws_session_token", "AQoDYXdz");
        assert_eq!(bundle.filled_fields(), ["password", "aws_session_token"]);
        assert!(!bundle.is_empty());
        assert!(CredentialBundle::new().is_empty());
    }

    #[test]
    fn l_aller_retour_json_est_fidele() {
        let bundle = CredentialBundle::new()
            .with_password("hunter2")
            .with_token("sk-ant-secret")
            .with_tls_client_cert("-----BEGIN CERTIFICATE-----")
            .with_tls_client_key("-----BEGIN PRIVATE KEY-----")
            .with_ssh_private_key("-----BEGIN OPENSSH PRIVATE KEY-----")
            .with_ssh_passphrase("phrase")
            .with_extra("aws_session_token", "AQoDYXdz");

        let json = bundle.to_secret_json().expect("encodage");
        let relu = CredentialBundle::from_secret_json(&json).expect("décodage");

        assert_eq!(relu.password(), Some("hunter2"));
        assert_eq!(relu.token(), Some("sk-ant-secret"));
        assert_eq!(relu.tls_client_cert(), Some("-----BEGIN CERTIFICATE-----"));
        assert_eq!(relu.tls_client_key(), Some("-----BEGIN PRIVATE KEY-----"));
        assert_eq!(
            relu.ssh_private_key(),
            Some("-----BEGIN OPENSSH PRIVATE KEY-----")
        );
        assert_eq!(relu.ssh_passphrase(), Some("phrase"));
        assert_eq!(relu.extra("aws_session_token"), Some("AQoDYXdz"));
        assert_eq!(relu.extra("inconnu"), None);
    }

    #[test]
    fn seuls_les_champs_renseignes_sont_ecrits() {
        let bundle = CredentialBundle::new().with_password("hunter2");
        let json = bundle.to_secret_json().expect("encodage");
        assert_eq!(json.expose_secret(), r#"{"password":"hunter2"}"#);

        let vide = CredentialBundle::new().to_secret_json().expect("encodage");
        assert_eq!(vide.expose_secret(), "{}");
    }

    #[test]
    fn un_bundle_ecrit_par_une_version_ulterieure_reste_lisible() {
        // I-11 : ce qu'Oxyn écrit reste lisible, et l'inverse aussi — un champ
        // qu'on ne connaît pas encore ne doit pas rendre le trousseau
        // inexploitable.
        let json = SecretString::from(r#"{"password":"hunter2","kerberos_keytab":"…"}"#);
        let bundle = CredentialBundle::from_secret_json(&json).expect("champ inconnu toléré");
        assert_eq!(bundle.password(), Some("hunter2"));
    }

    #[test]
    fn ce_qui_n_est_pas_un_bundle_est_refuse_sans_etre_recopie() {
        for contenu in ["hunter2", "[]", "{\"password\": 42}", ""] {
            let secret = SecretString::from(contenu);
            let err = CredentialBundle::from_secret_json(&secret)
                .expect_err("ce n'est pas un bundle valide");
            assert!(matches!(err, SecretError::Malformed { .. }));
            assert!(
                !err.to_string().contains("hunter2"),
                "contenu fuité : {err}"
            );
        }
    }

    #[test]
    fn l_effacement_vide_tous_les_champs() {
        // On ne peut pas observer le tas depuis un test portable ; ce qui est
        // vérifiable, c'est que `zeroize` remet la structure à l'état vide —
        // donc qu'aucun champ n'a été oublié dans l'implémentation manuelle.
        let mut bundle = CredentialBundle::new()
            .with_password("hunter2")
            .with_token("sk-ant-secret")
            .with_tls_client_cert("cert")
            .with_tls_client_key("clé")
            .with_ssh_private_key("clé ssh")
            .with_ssh_passphrase("phrase")
            .with_extra("aws_session_token", "AQoDYXdz");

        bundle.zeroize();

        assert!(bundle.is_empty(), "un champ a échappé à l'effacement");
        assert!(bundle.filled_fields().is_empty());
    }
}
