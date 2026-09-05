//! Les secrets d'une connexion, le temps de les remettre à un driver.
//!
//! [`ConnectionConfig`](oxyn_core::ConnectionConfig) ne porte **aucun secret** :
//! elle n'y range qu'une *référence*, résolue par `oxyn-secrets` auprès du
//! trousseau du système. [`Credentials`] est ce que devient cette référence
//! après résolution, et son seul usage est d'être passée à
//! [`Driver::connect`](crate::traits::Driver::connect).
//!
//! # Pourquoi un type de plus
//!
//! `oxyn_secrets::CredentialBundle` fait le même travail, mais `oxyn-secrets`
//! n'est pas au contrat de dépendances de cette crate — et l'y mettre
//! renverserait le sens des dépendances : le trousseau du système est un détail
//! de l'hôte, pas du contrat de driver. Un driver reçoit sa configuration ; il
//! ne va jamais la chercher ([`DRIVER-CONTRACT`](../../../docs/DRIVER-CONTRACT.md)).
//! La conversion tient en quelques lignes, du côté de l'appelant qui connaît les
//! deux.

use std::fmt;

use indexmap::IndexMap;
use secrecy::SecretString;

/// Les secrets d'une connexion, résolus depuis le trousseau du système.
///
/// # Ce que le type garantit
///
/// Pas de `Debug` dérivé, pas de `Display`, pas de `Serialize`, pas de `Clone` :
/// les seuls chemins de sortie sont [`password`](Self::password),
/// [`token`](Self::token) et [`extra`](Self::extra), qui rendent une
/// [`SecretString`] — elle-même sans `Display`, au `Debug` masqué, et effacée à
/// sa destruction.
///
/// L'absence de `Clone` n'est pas un oubli : [`SecretString`] ne l'est pas non
/// plus, parce qu'une copie de secret est une copie à effacer de plus.
#[derive(Default)]
pub struct Credentials {
    password: Option<SecretString>,
    token: Option<SecretString>,
    extras: IndexMap<String, SecretString>,
}

impl Credentials {
    /// Aucun identifiant. C'est le cas de SQLite et de DuckDB.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attache un mot de passe.
    #[must_use]
    pub fn with_password(mut self, password: impl Into<SecretString>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Attache un jeton — clé d'API, jeton de service, `AUTH` Redis.
    #[must_use]
    pub fn with_token(mut self, token: impl Into<SecretString>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Attache un secret propre au driver : phrase de passe d'une clé cliente,
    /// clé privée SSH…
    ///
    /// La clé est un **nom de champ**, pas un secret : elle apparaît dans le
    /// `Debug` du type.
    #[must_use]
    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<SecretString>) -> Self {
        self.extras.insert(key.into(), value.into());
        self
    }

    /// Le mot de passe, s'il y en a un.
    #[must_use]
    pub fn password(&self) -> Option<&SecretString> {
        self.password.as_ref()
    }

    /// Le jeton, s'il y en a un.
    #[must_use]
    pub fn token(&self) -> Option<&SecretString> {
        self.token.as_ref()
    }

    /// Un secret propre au driver.
    #[must_use]
    pub fn extra(&self, key: &str) -> Option<&SecretString> {
        self.extras.get(key)
    }

    /// Aucun secret n'est porté.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.password.is_none() && self.token.is_none() && self.extras.is_empty()
    }

    /// Les **noms** des secrets présents, jamais leurs valeurs.
    ///
    /// C'est tout ce qu'un diagnostic a le droit de dire d'un porteur de
    /// secrets, et c'est exactement ce que rend son `Debug`.
    #[must_use]
    pub fn filled_fields(&self) -> Vec<&str> {
        let mut noms = Vec::new();
        if self.password.is_some() {
            noms.push("password");
        }
        if self.token.is_some() {
            noms.push("token");
        }
        noms.extend(self.extras.keys().map(String::as_str));
        noms
    }
}

impl fmt::Debug for Credentials {
    /// Écrit à la main : un `Debug` dérivé sur un porteur de secrets est le mode
    /// de fuite le plus fréquent, parce qu'il est invisible à la relecture
    /// (I-03). C'est le `tracing::debug!("{creds:?}")` ajouté six mois plus tard
    /// qui fuit.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Credentials(<masqué : {:?}>)", self.filled_fields())
    }
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    const MOT_DE_PASSE: &str = "hunter2";

    #[test]
    fn des_identifiants_vides_se_declarent_comme_tels() {
        let identifiants = Credentials::new();
        assert!(identifiants.is_empty());
        assert!(identifiants.password().is_none());
        assert!(identifiants.token().is_none());
        assert!(identifiants.extra("ssh_passphrase").is_none());
        assert!(identifiants.filled_fields().is_empty());
    }

    #[test]
    fn chaque_secret_se_relit_par_son_accesseur() {
        let identifiants = Credentials::new()
            .with_password(MOT_DE_PASSE)
            .with_token("jeton-de-service")
            .with_extra("ssh_passphrase", "phrase-de-passe");

        assert_eq!(
            identifiants.password().map(ExposeSecret::expose_secret),
            Some(MOT_DE_PASSE)
        );
        assert_eq!(
            identifiants.token().map(ExposeSecret::expose_secret),
            Some("jeton-de-service")
        );
        assert_eq!(
            identifiants
                .extra("ssh_passphrase")
                .map(ExposeSecret::expose_secret),
            Some("phrase-de-passe")
        );
        assert!(!identifiants.is_empty());
    }

    #[test]
    fn le_debug_ne_nomme_que_les_champs_remplis() {
        // I-03, corollaire vérifiable : aucun `Debug` ne montre un secret.
        let identifiants = Credentials::new()
            .with_password(MOT_DE_PASSE)
            .with_token("jeton-de-service")
            .with_extra("ssh_passphrase", "phrase-de-passe");

        let rendu = format!("{identifiants:?}");

        assert!(!rendu.contains(MOT_DE_PASSE), "fuite : {rendu}");
        assert!(!rendu.contains("jeton-de-service"), "fuite : {rendu}");
        assert!(!rendu.contains("phrase-de-passe"), "fuite : {rendu}");

        // Ce qui reste doit rester utile au diagnostic.
        assert!(rendu.contains("password"), "{rendu}");
        assert!(rendu.contains("token"), "{rendu}");
        assert!(rendu.contains("ssh_passphrase"), "{rendu}");
    }

    #[test]
    fn le_debug_d_identifiants_vides_ne_ment_pas() {
        let rendu = format!("{:?}", Credentials::new());
        assert_eq!(rendu, "Credentials(<masqué : []>)");
    }

    #[test]
    fn un_secret_reecrit_remplace_le_precedent() {
        let identifiants = Credentials::new()
            .with_password("ancien")
            .with_password(MOT_DE_PASSE);
        assert_eq!(
            identifiants.password().map(ExposeSecret::expose_secret),
            Some(MOT_DE_PASSE)
        );
        assert_eq!(identifiants.filled_fields(), ["password"]);
    }
}
