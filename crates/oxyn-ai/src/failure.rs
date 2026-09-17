//! Ce qu'on dit au modèle quand une commande a échoué — et ce qu'on lui tait.
//!
//! # Le défaut que ce module ferme
//!
//! Un message d'erreur de serveur **est** du contenu de la base. PostgreSQL
//! répond `duplicate key value violates unique constraint "clients_email_key"
//! DETAIL: Key (email)=(dupont@example.com) already exists.` : la valeur de
//! ligne est dans le texte. Réinjecter ce message dans la conversation le fait
//! partir chez le fournisseur au tour suivant, sous un niveau qui l'interdit
//! ([I-04](../../../CLAUDE.md#i-04)).
//!
//! La protection posée à la frontière des drivers ne couvre pas ce cas :
//! elle retient le message quand l'instruction portait des **valeurs liées**,
//! or une instruction composée par un agent n'en porte aucune (`tools.rs`).
//! Son message part donc entier.
//!
//! # Le filtre est à la construction, pas au rendu
//!
//! [`FailureReport`] ne **stocke** que ce que le niveau laisse sortir : sous
//! `Local` et `Metadata`, le message du serveur n'entre jamais dans la
//! structure. Un `Display` oublié, un `Debug` dérivé, un champ ajouté six mois
//! plus tard à un journal ne peuvent donc pas le laisser fuir — il n'est plus
//! là. Filtrer au rendu aurait supposé que tous les rendus futurs y pensent.
//!
//! # Pourquoi tout le message, et pas seulement la valeur
//!
//! Sous `Metadata`, les noms d'objets sortent : on pourrait vouloir garder la
//! partie « nom de contrainte » et retirer la partie « valeur ». On ne sait pas
//! le faire. Le serveur compose un texte libre, un déclencheur y concatène ce
//! qu'il veut, et chercher la valeur dans le texte aurait l'apparence d'une
//! protection sans en être une. Le critère retenu est donc le même que partout
//! ailleurs dans la crate :
//! [`PrivacyTier::allows_row_values`](oxyn_core::PrivacyTier::allows_row_values).
//!
//! Sous `Local`, aucun fournisseur distant n'est accepté : rien ne quitterait
//! la machine de toute façon. Le message est **quand même** retenu, pour que la
//! règle n'ait pas d'exception — un niveau plus strict que `Metadata` qui en
//! laisserait sortir davantage serait la sorte d'inversion que personne ne
//! relit deux fois.
//!
//! # Ce qui survit au filtrage
//!
//! La **classe** de l'erreur, sa **retentabilité** — qui en découle, donc ne
//! peut pas la contredire ([I-13](../../../CLAUDE.md#i-13)) — et le **code**
//! identifiant quand le message en porte un de forme reconnaissable. Un code
//! ne cite rien : c'est ce que les drivers gardent eux-mêmes quand ils
//! retiennent un message.

use std::fmt;

use oxyn_core::{ErrorClass, PrivacyTier};

/// Marqueur d'un SQLSTATE dans un message. Les drivers PostgreSQL du dépôt
/// l'écrivent sous cette forme quand ils retiennent le message du serveur.
const SQLSTATE_MARKER: &str = "SQLSTATE ";

/// Longueur d'un SQLSTATE : cinq caractères, sans exception.
const SQLSTATE_LEN: usize = 5;

/// Marqueur d'un code de résultat SQLite. `rusqlite` rend « Error code 19:
/// constraint failed » : le libellé est dérivé du **nombre**.
const SQLITE_MARKER: &str = "Error code ";

/// Nombre maximal de chiffres retenus pour un code de résultat SQLite. Les
/// codes étendus tiennent sur quatre chiffres ; au-delà, ce n'est plus un code.
const SQLITE_CODE_MAX_DIGITS: usize = 4;

/// L'échec d'une commande, réduit à ce que le niveau de la connexion laisse
/// sortir.
///
/// **Ne se construit pas hors de cette crate** : ses champs sont privés et son
/// seul constructeur est `redact`, privé au module, qui exige un
/// [`PrivacyTier`]. C'est ce qui fait du filtre une contrainte de type et non
/// une convention qu'un appelant futur oublierait — la même propriété que celle
/// qui tient déjà le point de passage du contexte
/// ([`AgentContext`](crate::context::AgentContext)).
///
/// Le `Debug` est dérivé sans danger : la structure ne contient le message du
/// serveur que sous un niveau qui l'autorise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureReport {
    /// Le niveau qui a été appliqué. Conservé pour que le modèle sache qu'un
    /// message existe et pourquoi il ne l'a pas : sans cela, il comblerait le
    /// vide en inventant la cause de l'échec.
    tier: PrivacyTier,
    /// La famille de l'erreur, telle que le driver l'a classée.
    class: ErrorClass,
    /// Le code identifiant, quand le message en portait un.
    code: Option<String>,
    /// Le message du serveur. `Some` **seulement** sous un niveau qui autorise
    /// les valeurs de lignes.
    detail: Option<String>,
}

impl FailureReport {
    /// Réduit un échec à ce que ce niveau laisse sortir.
    ///
    /// `pub(crate)` : le seul appelant légitime est la conversion d'un
    /// [`DispatchOutcome`](crate::runtime::DispatchOutcome), qui tient le
    /// niveau de la session en cours. Un puits de commandes, lui, n'a aucune
    /// raison de connaître le niveau — et donc aucun moyen de se tromper de
    /// niveau.
    pub(crate) fn redact(tier: PrivacyTier, class: ErrorClass, message: &str) -> Self {
        Self {
            tier,
            class,
            code: safe_code(message),
            detail: tier.allows_row_values().then(|| message.to_owned()),
        }
    }

    /// La famille de l'erreur. Elle survit au filtrage : c'est une donnée, pas
    /// une déduction faite sur un message.
    #[must_use]
    pub const fn class(&self) -> ErrorClass {
        self.class
    }

    /// L'opération est-elle rejouable telle quelle ?
    ///
    /// Dérivé de la classe, donc jamais en contradiction avec elle : une erreur
    /// ambiguë ne se retente pas, filtrée ou non ([I-13](../../../CLAUDE.md#i-13)).
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        self.class.is_retryable()
    }

    /// Le code identifiant retenu, quand il y en avait un.
    #[must_use]
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// Le message du serveur, quand le niveau l'autorise.
    ///
    /// `None` n'est pas « il n'y avait pas de message » : c'est « le niveau ne
    /// le laisse pas sortir ».
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Le niveau appliqué à cet échec.
    #[must_use]
    pub const fn tier(&self) -> PrivacyTier {
        self.tier
    }
}

impl fmt::Display for FailureReport {
    /// Le corps destiné au modèle, **en anglais** : c'est une invite.
    ///
    /// Un seul rendu, ici : [`ToolOutcome::render`](crate::ToolOutcome::render)
    /// s'appuie dessus plutôt que de composer un second texte qui divergerait.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "class: {}", class_label(self.class))?;
        writeln!(f, "retryable: {}", self.is_retryable())?;
        if let Some(code) = &self.code {
            writeln!(f, "code: {code}")?;
        }
        match &self.detail {
            Some(message) => write!(f, "error: {message}"),
            // Dire que le message existe et qu'il n'est pas disponible : un
            // modèle à qui il manque une information la comble en l'inventant.
            None => write!(
                f,
                "error: withheld — this connection's privacy tier (`{}`) keeps server messages \
                 on this machine, because they can quote row values. Do not guess what it said; \
                 ask the user to read the full error in Oxyn.",
                self.tier
            ),
        }
    }
}

/// Le nom d'une famille d'erreur, pour un modèle.
///
/// Distinct de [`ErrorClass::as_str`], qui est exhaustif par construction :
/// ici le bras `_` rend `unknown`, parce que ce texte traverse la frontière IA
/// et qu'une famille inconnue ne doit pas y être annoncée comme retentable.
///
/// Le bras `_` n'est pas un relâchement : [`ErrorClass`] est
/// `#[non_exhaustive]`, et une famille que cette crate ne connaît pas encore ne
/// doit pas être annoncée retentable ou définitive par défaut — `unknown` est la
/// seule réponse honnête.
const fn class_label(class: ErrorClass) -> &'static str {
    match class {
        ErrorClass::Transient => "transient",
        ErrorClass::Permanent => "permanent",
        ErrorClass::Ambiguous => "ambiguous",
        _ => "unknown",
    }
}

/// Le code identifiant porté par un message, quand il est reconnaissable.
///
/// Deux formes seulement, toutes deux **marquées** dans le texte : un SQLSTATE
/// PostgreSQL et un code de résultat SQLite. Chercher un motif non marqué —
/// « cinq caractères majuscules quelque part » — rapporterait des noms de
/// tables et des fragments de valeurs.
///
/// # La réserve à connaître
///
/// Un serveur hostile peut écrire `SQLSTATE ABCDE` dans son message et faire
/// ainsi sortir cinq caractères de son choix par échec. C'est le prix de garder
/// le seul identifiant qu'un professionnel utilise réellement pour diagnostiquer
/// — et le canal est borné : cinq caractères d'un alphabet de trente-six, une
/// fois par appel d'outil, dans un flux que l'utilisateur voit.
fn safe_code(message: &str) -> Option<String> {
    if let Some(code) = sqlstate(message) {
        return Some(format!("SQLSTATE {code}"));
    }
    sqlite_code(message).map(|code| format!("SQLite error code {code}"))
}

/// Le SQLSTATE qui suit le marqueur, s'il a exactement la forme attendue.
fn sqlstate(message: &str) -> Option<String> {
    let (_, rest) = message.split_once(SQLSTATE_MARKER)?;
    let code: String = rest
        .chars()
        .take(SQLSTATE_LEN)
        .filter(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
        .collect();
    // `chars().count()` et non `len()` : le filtre a pu écarter des caractères,
    // et un code amputé n'est pas un code.
    if code.chars().count() != SQLSTATE_LEN {
        return None;
    }
    // Ce qui suit doit fermer le jeton : `SQLSTATE 42P01X` n'est pas un
    // SQLSTATE, c'est le début d'autre chose.
    match rest.chars().nth(SQLSTATE_LEN) {
        Some(suivant) if suivant.is_ascii_alphanumeric() => None,
        _ => Some(code),
    }
}

/// Le code de résultat SQLite qui suit le marqueur, s'il est numérique.
fn sqlite_code(message: &str) -> Option<String> {
    let (_, rest) = message.split_once(SQLITE_MARKER)?;
    let code: String = rest
        .chars()
        .take_while(char::is_ascii_digit)
        .take(SQLITE_CODE_MAX_DIGITS)
        .collect();
    (!code.is_empty()).then_some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le message que PostgreSQL rend sur une violation de contrainte unique :
    /// il recopie la valeur de la ligne.
    const MESSAGE_HOSTILE: &str = "duplicate key value violates unique constraint \
                                   \"clients_email_key\" DETAIL: Key (email)=\
                                   (dupont@example.com) already exists. \
                                   (SQLSTATE 23505) iban=FR7630006000011234567890189";

    #[test]
    fn sous_local_et_metadata_le_message_du_serveur_n_est_pas_meme_stocke() {
        for niveau in [PrivacyTier::Local, PrivacyTier::Metadata] {
            let rapport = FailureReport::redact(niveau, ErrorClass::Permanent, MESSAGE_HOSTILE);
            assert_eq!(rapport.detail(), None, "{niveau}");

            let affiche = rapport.to_string();
            let debogue = format!("{rapport:?}");
            for rendu in [&affiche, &debogue] {
                assert!(!rendu.contains("dupont@example.com"), "{niveau} : {rendu}");
                assert!(!rendu.contains("FR76"), "{niveau} : {rendu}");
                assert!(!rendu.contains("clients_email_key"), "{niveau} : {rendu}");
            }
            // Ce qui reste doit rester utile : sinon le modèle réessaie à
            // l'aveugle la même instruction.
            assert!(affiche.contains("SQLSTATE 23505"), "{affiche}");
            assert!(affiche.contains("class: permanent"), "{affiche}");
            assert!(affiche.contains("retryable: false"), "{affiche}");
        }
    }

    #[test]
    fn sous_sampled_le_message_arrive_entier() {
        // Le test négatif qui donne son sens au précédent : sans lui, tout
        // pourrait être masqué en permanence sans que rien ne le signale.
        let rapport =
            FailureReport::redact(PrivacyTier::Sampled, ErrorClass::Permanent, MESSAGE_HOSTILE);
        assert_eq!(rapport.detail(), Some(MESSAGE_HOSTILE));
        assert!(rapport.to_string().contains("dupont@example.com"));
    }

    #[test]
    fn la_classe_et_la_retentabilite_survivent_au_filtrage() {
        // I-13 : une erreur ambiguë ne se retente jamais, filtrée ou non. La
        // retentabilité est dérivée de la classe, donc elles ne peuvent pas
        // diverger.
        let transitoire = FailureReport::redact(
            PrivacyTier::Metadata,
            ErrorClass::Transient,
            "server closed the connection unexpectedly",
        );
        assert_eq!(transitoire.class(), ErrorClass::Transient);
        assert!(transitoire.is_retryable());
        assert!(transitoire.to_string().contains("retryable: true"));

        let ambigue = FailureReport::redact(
            PrivacyTier::Metadata,
            ErrorClass::Ambiguous,
            "timed out after 30s",
        );
        assert_eq!(ambigue.class(), ErrorClass::Ambiguous);
        assert!(
            !ambigue.is_retryable(),
            "le serveur a peut-être appliqué l'écriture"
        );
        assert!(ambigue.to_string().contains("class: ambiguous"));
    }

    #[test]
    fn un_niveau_qui_masque_le_dit_au_modele() {
        let rapport = FailureReport::redact(
            PrivacyTier::Metadata,
            ErrorClass::Permanent,
            "boom (email=x)",
        );
        let rendu = rapport.to_string();
        assert!(rendu.contains("withheld"), "{rendu}");
        assert!(rendu.contains("`metadata`"), "{rendu}");
        assert!(
            rendu.contains("Do not guess"),
            "un modèle privé d'information l'invente : {rendu}"
        );
    }

    #[test]
    fn seuls_les_codes_marques_survivent() {
        assert_eq!(
            safe_code("… (SQLSTATE 42P01)").as_deref(),
            Some("SQLSTATE 42P01")
        );
        assert_eq!(
            safe_code("Error code 19: constraint failed: UNIQUE constraint failed").as_deref(),
            Some("SQLite error code 19")
        );
        // Un nom de table de cinq majuscules n'est pas un code : sans marqueur,
        // rien n'est retenu.
        assert_eq!(safe_code("relation \"USERS\" does not exist"), None);
        assert_eq!(safe_code("Key (email)=(dupont@example.com)"), None);
    }

    #[test]
    fn un_code_mal_forme_n_est_pas_repris() {
        // Le marqueur ne suffit pas : ce qui le suit doit avoir la forme d'un
        // code, sinon un serveur hostile ferait passer du texte pour un code.
        assert_eq!(safe_code("SQLSTATE 42p0"), None, "trop court");
        assert_eq!(safe_code("SQLSTATE 42P01X"), None, "jeton non fermé");
        assert_eq!(safe_code("SQLSTATE dupont"), None, "minuscules");
        assert_eq!(safe_code("Error code : none"), None, "pas de chiffre");
        assert_eq!(
            safe_code("Error code 12345678: x").as_deref(),
            Some("SQLite error code 1234"),
            "borné à quatre chiffres"
        );
    }

    #[test]
    fn un_message_vide_ne_produit_pas_de_code() {
        let rapport = FailureReport::redact(PrivacyTier::Metadata, ErrorClass::Permanent, "");
        assert_eq!(rapport.code(), None);
        assert!(!rapport.to_string().contains("code:"));
    }
}
