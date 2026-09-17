//! La clé d'API, et ce qu'on fait pour qu'elle ne sorte nulle part.
//!
//! Une clé de fournisseur est un secret au sens d'[`I-03`](../../../CLAUDE.md) :
//! elle n'a rien à faire dans un journal, une erreur affichée, un rapport de
//! plantage ni un fichier de workspace. Le corollaire vérifiable de l'invariant
//! est qu'**aucun type portant un secret ne dérive `Debug`** — c'est le
//! `tracing::debug!("{provider:?}")` ajouté six mois plus tard qui fuit.
//!
//! # Pourquoi pas `secrecy::SecretString`
//!
//! `secrecy` et `zeroize` ne sont pas au contrat de dépendances de cette crate
//! (voir son `Cargo.toml`, qui est figé). [`ApiKey`] est l'équivalent minimal :
//! `Debug` écrit à la main, pas de `Display`, pas de `Serialize`, et un `Drop`
//! qui écrase le tampon. Cet effacement est un **meilleur effort** : sans
//! `zeroize`, rien n'empêche formellement le compilateur de considérer
//! l'écriture comme morte. La protection qui compte vraiment ici est l'absence
//! de tout chemin d'affichage.

use std::fmt;

/// Clé d'API d'un fournisseur de modèles.
///
/// Ne s'affiche jamais : `Debug` est masqué, `Display` n'existe pas, et le type
/// n'est ni `Serialize` ni `Deserialize` — une clé se lit dans le trousseau ou
/// dans l'environnement, elle ne se persiste pas depuis ici.
///
/// L'égalité n'est volontairement pas implémentée : comparer deux clés n'a
/// aucun usage légitime dans cette crate, et une comparaison naïve invite la
/// mauvaise idée d'authentifier quelqu'un avec.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    /// Adopte une clé fournie par l'appelant.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Lit une clé depuis une variable d'environnement.
    ///
    /// Rend `None` si la variable est absente, vide, ou non représentable en
    /// UTF-8. C'est le chemin des fournisseurs configurés hors interface
    /// (`OPENAI_API_KEY`, `OPENROUTER_API_KEY`…).
    #[must_use]
    pub fn from_env(variable: &str) -> Option<Self> {
        let brute = std::env::var(variable).ok()?;
        let cle = Self::new(brute);
        if cle.is_blank() { None } else { Some(cle) }
    }

    /// Expose la clé, pour la poser dans un en-tête HTTP et rien d'autre.
    ///
    /// Le nom est délibérément désagréable : chaque appel est un endroit à
    /// relire. Ne jamais mettre le résultat dans un message d'erreur, un
    /// `format!` de journal, ni une URL — une clé en paramètre de requête finit
    /// dans les journaux d'accès du fournisseur.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// La clé est-elle vide ou uniquement composée d'espaces ?
    ///
    /// Une clé blanche est une erreur de configuration, pas une absence de clé :
    /// un fournisseur qui la reçoit répondra `401` plutôt que d'être clair.
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.0.trim().is_empty()
    }

    /// Longueur en octets, seule information qu'on accepte de divulguer.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Équivalent de [`is_blank`](Self::is_blank) au sens strict de la longueur.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for ApiKey {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ApiKey {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl fmt::Debug for ApiKey {
    /// Ne montre que la longueur. Jamais un préfixe, jamais un suffixe : quatre
    /// caractères d'une clé suffisent à la reconnaître dans une fuite.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ApiKey(<masquée, {} octets>)", self.0.len())
    }
}

impl Drop for ApiKey {
    fn drop(&mut self) {
        // `String::into_bytes` réutilise l'allocation : on écrase donc bien le
        // tampon qui portait la clé, et non une copie.
        let mut octets = std::mem::take(&mut self.0).into_bytes();
        octets.fill(0);
    }
}

/// Ce qui remplace la clé dans un texte expurgé.
///
/// En anglais : cette mention finit dans un message d'erreur affiché, et c'est
/// la langue du code source (CLAUDE.md § Langue).
pub(crate) const REDACTED: &str = "<redacted API key>";

/// Remplace toute occurrence littérale de la clé par une mention neutre.
///
/// Certains fournisseurs recopient la clé reçue dans leur message d'erreur.
/// Ce filtre est la dernière barrière avant qu'un corps de réponse ne devienne
/// un message d'erreur d'Oxyn, donc un journal (I-03).
///
/// Une clé vide ou blanche n'est pas cherchée : elle apparaîtrait partout.
#[must_use]
pub(crate) fn redact_key(texte: &str, cle: Option<&ApiKey>) -> String {
    match cle {
        Some(cle) if !cle.is_blank() => texte.replace(cle.expose(), REDACTED),
        _ => texte.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_debug_ne_laisse_rien_filtrer() {
        let cle = ApiKey::new("sk-proj-0123456789abcdef");
        let rendu = format!("{cle:?}");
        assert!(!rendu.contains("sk-proj"), "{rendu}");
        assert!(!rendu.contains("0123"), "{rendu}");
        assert!(rendu.contains("masquée"), "{rendu}");
    }

    #[test]
    fn le_debug_d_une_option_ne_laisse_rien_filtrer_non_plus() {
        // Le cas réel : `#[derive(Debug)]` sur une structure qui porte
        // `Option<ApiKey>` délègue au `Debug` de `ApiKey`.
        let porte = Some(ApiKey::new("sk-secret"));
        let rendu = format!("{porte:?}");
        assert!(!rendu.contains("secret"), "{rendu}");
    }

    #[test]
    fn une_cle_blanche_est_reconnue() {
        assert!(ApiKey::new("").is_blank());
        assert!(ApiKey::new("   \t\n").is_blank());
        assert!(!ApiKey::new("sk-x").is_blank());
    }

    #[test]
    fn la_redaction_efface_la_cle_recopiee_par_le_fournisseur() {
        let cle = ApiKey::new("sk-abcdef");
        let corps = r#"{"error":{"message":"Incorrect API key provided: sk-abcdef"}}"#;
        let filtre = redact_key(corps, Some(&cle));
        assert!(!filtre.contains("sk-abcdef"), "{filtre}");
        assert!(filtre.contains(REDACTED), "{filtre}");
    }

    #[test]
    fn la_redaction_sans_cle_laisse_le_texte_intact() {
        let corps = "model not found";
        assert_eq!(redact_key(corps, None), corps);
    }

    #[test]
    fn une_cle_blanche_ne_sert_pas_de_motif_de_redaction() {
        // Sinon `replace("", …)` insérerait la mention entre chaque caractère.
        let cle = ApiKey::new("   ");
        assert_eq!(redact_key("abc", Some(&cle)), "abc");
    }
}
