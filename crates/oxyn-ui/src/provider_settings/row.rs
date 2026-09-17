//! Ce qu'une ligne de l'écran de déclaration **affiche**, calculé hors du rendu.
//!
//! # Pourquoi cette extraction existe
//!
//! L'écran doit lister deux sortes de déclarations : un fournisseur de modèles
//! ([ADR-0023](../../../../docs/adr/0023-fournisseurs-declares-et-provenance.md))
//! et un agent externe
//! ([ADR-0026](../../../../docs/adr/0026-agents-externes-acp.md)). Elles n'ont
//! **aucun champ en commun** hors du nom : un agent n'a ni point d'accès, ni
//! modèle, ni clé, ni portée mesurable.
//!
//! Dupliquer la liste, la ligne, le focus et la confirmation pour la seconde
//! sorte aurait recopié un parcours entier, ce que
//! [CLAUDE.md](../../../../CLAUDE.md#organisation-du-code) interdit. Ce module
//! prend l'autre voie : **ce qui varie devient une donnée**, et la vue dessine
//! la même ligne pour les deux.
//!
//! C'est aussi ce que demande la règle d'interface du dépôt — « le calcul sort,
//! la vue dessine » : ce fichier se teste sans fenêtre, contrairement au rendu.

use crate::provider_settings::{DeclaredProvider, KeyState, kind_label, reach_summary};

/// Une déclaration, quelle que soit sa sorte.
///
/// Emprunté : la liste vit dans l'écran, et une ligne ne survit pas à la trame
/// qui la dessine.
#[derive(Debug, Clone, Copy)]
pub enum Declaration<'a> {
    /// Un fournisseur de modèles, avec son point d'accès et sa clé.
    Provider(&'a DeclaredProvider),
    /// Un agent externe : une commande, et aucun secret.
    Agent {
        /// Le nom donné par l'utilisateur.
        label: &'a str,
        /// Le programme déclaré.
        command: &'a str,
        /// Combien d'arguments l'accompagnent.
        args: usize,
    },
}

/// Ce que la vue dessine, une fois le calcul fait.
///
/// Cinq champs, tous du texte : la ligne est la **même** pour les deux sortes,
/// et c'est ce qui permet de n'écrire qu'un `render_row`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowDisplay {
    /// Le nom donné par l'utilisateur. C'est **lui** qui est montré.
    pub label: String,
    /// La sorte, en un mot : famille de protocole, ou « external agent ».
    pub family: String,
    /// La première ligne de détail — point d'accès, ou commande.
    pub primary: String,
    /// La seconde — modèle, ou nombre d'arguments.
    pub secondary: String,
    /// L'état de la clé, jamais sa valeur.
    pub key_note: String,
    /// Ce qu'on sait de la destination des données.
    pub reach_note: String,
    /// La destination doit-elle être signalée en avertissement ?
    pub reach_warns: bool,
}

/// Calcule ce qu'une ligne affiche.
///
/// # Les deux mentions qui comptent pour un agent
///
/// `key_note` dit « no key » et ce n'est pas un manque : un agent externe porte
/// sa propre authentification, et c'est l'intérêt du mode. Le dire plutôt que
/// laisser un blanc évite la lecture « clé non configurée », qui serait fausse.
///
/// `reach_note` dit que la destination est **inconnaissable**, et non
/// « inconnue ». La nuance n'est pas de style : pour un fournisseur, la portée
/// se mesure et peut être périmée ; pour un agent, il n'y a rien à mesurer. Le
/// signalement en avertissement est donc **permanent** — il ne disparaîtra pas à
/// la mesure suivante, puisqu'il n'y en aura pas.
#[must_use]
pub fn row_display(declaration: Declaration<'_>) -> RowDisplay {
    match declaration {
        Declaration::Provider(fournisseur) => RowDisplay {
            label: fournisseur.label.to_string(),
            family: kind_label(fournisseur.kind)
                .unwrap_or("unknown family")
                .to_owned(),
            primary: fournisseur.base_url.to_string(),
            secondary: fournisseur.model.to_string(),
            key_note: fournisseur.key.label().to_owned(),
            reach_note: reach_summary(fournisseur.reach, &fournisseur.measured_at),
            reach_warns: fournisseur.reach.leaves_machine(),
        },
        Declaration::Agent {
            label,
            command,
            args,
        } => RowDisplay {
            label: label.to_owned(),
            family: "external agent".to_owned(),
            primary: command.to_owned(),
            secondary: match args {
                0 => "no arguments".to_owned(),
                1 => "1 argument".to_owned(),
                nombre => format!("{nombre} arguments"),
            },
            // Pas `KeyState::Absent` : « absente » se lirait comme un réglage
            // qui manque. Ici, il n'y a pas de clé à configurer.
            key_note: "no key — the agent carries its own".to_owned(),
            reach_note: "destination unknowable — Oxyn cannot see where this agent sends data"
                .to_owned(),
            reach_warns: true,
        },
    }
}

/// Le mot employé quand une clé **est** configurée, pour comparaison.
///
/// Rendu ici plutôt qu'importé par les tests : il documente ce à quoi la mention
/// d'un agent ne doit **pas** ressembler.
#[must_use]
pub fn configured_key_note() -> &'static str {
    KeyState::Configured.label()
}

/// La saisie d'un agent, telle que le formulaire la porte.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentDraft {
    /// Le nom donné à la déclaration.
    pub label: String,
    /// Le programme à lancer.
    pub command: String,
    /// Les arguments, **un par ligne**.
    pub args: String,
}

/// Découpe les arguments saisis, **une ligne par argument**.
///
/// # Pourquoi une ligne et non une espace
///
/// Séparer sur l'espace obligerait à inventer des guillemets pour l'argument qui
/// en contient une — donc une grammaire de ligne de commande, donc la surface
/// que tout ce lot évite : la déclaration transmet la commande et ses arguments
/// **séparément**, sans jamais rien recoller
/// ([ADR-0026](../../../../docs/adr/0026-agents-externes-acp.md)).
///
/// Une ligne par argument n'a pas de cas ambigu : ce que l'utilisateur tape est
/// ce que le processus reçoit, espaces comprises. Les lignes vides sont
/// ignorées — elles viennent d'un copier-coller, jamais d'une intention.
#[must_use]
pub fn parse_args(saisie: &str) -> Vec<String> {
    saisie
        .lines()
        .map(str::trim)
        .filter(|ligne| !ligne.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Ce qui empêche cette saisie d'être déclarée, s'il y a lieu.
///
/// **Délègue à `ExternalAgentConfig::validate`** plutôt que de redire ses
/// règles : deux validations divergent, et c'est celle du domaine qui décide au
/// moment d'écrire. Ici on ne fait que la poser plus tôt, pour que le message
/// arrive pendant la saisie et non après.
#[must_use]
pub fn agent_draft_error(draft: &AgentDraft) -> Option<String> {
    let id = oxyn_core::ProviderId::new("agent-draft").ok()?;
    let config = oxyn_core::ExternalAgentConfig::new(id, &draft.label, &draft.command)
        .with_args(parse_args(&draft.args));
    config.validate().err().map(|erreur| erreur.to_string())
}

#[cfg(test)]
mod tests;
