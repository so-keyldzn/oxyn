//! Ce qu'Oxyn accorde à un agent externe, et ce qu'il lui refuse d'office.
//!
//! Autorité : [ADR-0026](../../../docs/adr/0026-agents-externes-acp.md).
//!
//! # La correction qui a fondé ce module
//!
//! Une première rédaction d'ADR-0026 affirmait que `session/request_permission`
//! **est** le point d'entrée du `PolicyGate`. La lecture du protocole a montré
//! que c'était trop fort, et le corriger a décidé de la forme d'ici.
//!
//! La demande d'autorisation porte un `tool_call` — un outil **de l'agent** :
//! lire un fichier, en éditer un, lancer une commande. Ce ne sont pas des
//! `Command` d'Oxyn, et il n'existe aucune traduction : « l'agent veut éditer
//! `/etc/hosts` » ne devient pas une commande de base de données. Le
//! `PolicyGate` garde donc son domaine — ce qu'un agent demande **à Oxyn** —,
//! et ce module décide de l'autre moitié : ce qu'un agent demande à faire **sur
//! la machine**, pendant qu'Oxyn est son client.
//!
//! # Le parti retenu : Oxyn n'est pas un hôte d'agent de code
//!
//! Oxyn est un atelier de bases de données. Rien dans son périmètre ne justifie
//! qu'il accorde à un sous-processus le droit d'écrire des fichiers, d'en
//! supprimer ou de lancer des commandes — et il n'a pas d'interface pour
//! montrer *quel* fichier ni *quelle* commande, donc pas de quoi demander à
//! l'utilisateur de décider en connaissance de cause.
//!
//! Un éditeur de code comme Zed accorde ces droits parce que son périmètre les
//! rend sensés et parce qu'il sait les montrer. Copier ce choix sans l'un ni
//! l'autre serait ouvrir un accès au système derrière une fenêtre de base de
//! données.

use agent_client_protocol::schema::v1::{PermissionOption, PermissionOptionKind, ToolKind};
use oxyn_core::{ExternalAgentConfig, Result};

/// Vérifie qu'une déclaration d'agent est lançable.
///
/// # Pourquoi la commande et ses arguments restent séparés
///
/// L'exemple du protocole part d'une **chaîne unique** — `"python my_agent.py"`
/// — qu'il découpe. Un découpage de ligne de commande est une grammaire, et une
/// grammaire est une surface : un nom de programme contenant une espace, un
/// guillemet ou un point-virgule y prend un sens qu'on n'a pas voulu.
///
/// La déclaration d'Oxyn sépare la commande de ses arguments **à la saisie**, et
/// le lancement les transmet séparément à l'appel système. Rien n'est jamais
/// réassemblé en une chaîne, donc rien n'est jamais redécoupé : il n'y a pas de
/// shell dans le trajet.
///
/// # Erreurs
///
/// [`OxynError::Config`](oxyn_core::OxynError::Config) si la déclaration ne
/// passe pas sa propre validation — nom ou commande vide, caractère de contrôle,
/// listes hors borne. Valider **ici** plutôt qu'à la saisie seule : une
/// déclaration peut venir d'un fichier d'état écrit ailleurs.
pub fn check_launchable(agent: &ExternalAgentConfig) -> Result<()> {
    agent.validate()
}

/// Is this log record one the host must drop, whatever level it was asked for?
///
/// `agent-client-protocol` 2.1.0 logs every outgoing message **whole** at
/// `debug` (`src/jsonrpc/outgoing_actor.rs:15` and `:36`) and every line in and
/// out at `trace` (`src/jsonrpc/transport_actor.rs`). That is the bearer token
/// of the tool endpoint in `session/new`, and the user's questions in
/// `session/prompt` — both behind `OXYN_LOG=debug`, which is exactly what a user
/// is asked to set for a bug report ([I-03](../../../CLAUDE.md#i-03)).
///
/// So the crate is capped at `error` by **target**, not by level: an upgrade
/// that moves the same `?message` to `info` stays capped.
///
/// `error` and not `warn`: its `warn` lines carry `?error` built from what the
/// agent sent (`src/util/typed.rs:893` and `:925`,
/// `src/jsonrpc/incoming_actor.rs:276`, `:542` and `:588`), and a serde « invalid
/// type » error quotes the offending string — a question, or rows read under
/// `sampled`. Oxyn writes its own `warn` where it receives a protocol error,
/// with words it controls: the method and the error code, never the text. The host applies this as a
/// filter of its own, after the one `OXYN_LOG` configures, so no value of the
/// variable lifts it.
#[must_use]
pub fn is_protocol_chatter(target: &str, level: &tracing::Level) -> bool {
    const PROTOCOL: &str = "agent_client_protocol";
    let ours = target == PROTOCOL
        || target
            .strip_prefix(PROTOCOL)
            .is_some_and(|rest| rest.starts_with("::"));
    // `tracing` orders levels by verbosity: `WARN > ERROR`.
    ours && *level > tracing::Level::ERROR
}

/// Ce qu'Oxyn répond à une demande d'autorisation d'agent externe.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PermissionVerdict {
    /// Accordé sans demander : l'action ne quitte pas l'agent.
    Granted,
    /// Refusé, avec la raison à afficher **et** à renvoyer à l'agent.
    ///
    /// La raison est rendue telle quelle à l'agent pour qu'il cesse d'insister
    /// plutôt que de reformuler sa demande indéfiniment.
    Refused(&'static str),
}

impl PermissionVerdict {
    /// Le verdict est-il un refus ?
    #[must_use]
    pub const fn is_refused(&self) -> bool {
        matches!(self, Self::Refused(_))
    }
}

/// Ce qu'Oxyn accorde, par genre d'outil.
///
/// **Refus par défaut.** Seuls les genres qui ne touchent ni au système de
/// fichiers, ni au réseau, ni à un processus sont accordés : ils se déroulent
/// entièrement dans l'agent, et les refuser empêcherait toute conversation sans
/// rien protéger.
///
/// Le reste est refusé **d'office**, sans demander à l'utilisateur — non par
/// prudence excessive, mais parce qu'Oxyn n'a pas d'écran pour montrer quel
/// fichier ou quelle commande est en jeu. Une confirmation qui ne dit pas ce
/// qu'elle autorise est pire qu'un refus : elle déplace la responsabilité sans
/// donner de quoi l'exercer, et [I-02](../../../CLAUDE.md#i-02) dit déjà qu'une
/// confirmation finit par être cliquée.
///
/// `Read` est refusé comme les autres, et c'est délibéré : lire un fichier
/// arbitraire de la machine est une lecture **hors** du périmètre de la
/// connexion, donc hors de ce que le niveau de confidentialité gouverne.
#[must_use]
pub const fn permission_for(kind: ToolKind) -> PermissionVerdict {
    match kind {
        // Raisonnement interne et changement de mode : rien ne sort de l'agent.
        ToolKind::Think | ToolKind::SwitchMode => PermissionVerdict::Granted,
        ToolKind::Read | ToolKind::Search => PermissionVerdict::Refused(
            "Oxyn does not grant agents access to the file system. \
             Ask about the connected database instead.",
        ),
        ToolKind::Edit | ToolKind::Delete | ToolKind::Move => PermissionVerdict::Refused(
            "Oxyn never grants an agent write access to the file system.",
        ),
        ToolKind::Execute => PermissionVerdict::Refused(
            "Oxyn does not run commands on behalf of an agent. \
             Database work goes through Oxyn's own tools, which are reviewed.",
        ),
        ToolKind::Fetch => PermissionVerdict::Refused(
            "Oxyn does not fetch external resources on behalf of an agent.",
        ),
        // `Other` est le défaut de désérialisation du protocole : un genre
        // qu'on ne connaît pas est un genre qu'on ne sait pas juger.
        ToolKind::Other => PermissionVerdict::Refused(
            "Oxyn cannot tell what this tool would do, so it does not allow it.",
        ),
        // `ToolKind` est `#[non_exhaustive]` côté protocole : une version
        // ultérieure peut en ajouter. Refuser est le seul défaut sûr — accorder
        // un genre inconnu serait accorder ce que le protocole inventera.
        _ => PermissionVerdict::Refused(
            "This Oxyn build does not know that tool kind, so it does not allow it.",
        ),
    }
}

/// Choisit l'option de réponse qui exprime le verdict.
///
/// # L'asymétrie est délibérée
///
/// Le protocole offre quatre genres d'option : autoriser une fois, autoriser
/// toujours, refuser une fois, refuser toujours.
///
/// **À l'autorisation, Oxyn ne choisit jamais « toujours ».** Mémoriser un
/// accord large est une décision que l'utilisateur n'a pas prise, et que rien à
/// l'écran ne lui montrerait. `AllowOnce` seulement.
///
/// **Au refus, Oxyn choisit « toujours » quand il peut.** Ce qu'il refuse, il le
/// refusera à chaque fois — la raison rendue par [`permission_for`] est une
/// propriété du produit, pas une humeur. Laisser l'agent redemander à chaque
/// tour lui ferait perdre le sien, et l'utilisateur verrait une conversation qui
/// tourne en rond sans comprendre pourquoi.
///
/// Rend `None` quand aucune option ne convient — l'appelant répond alors
/// `Cancelled`, seule issue honnête : prétendre autoriser en sélectionnant une
/// option de refus, ou l'inverse, serait pire que d'interrompre.
///
/// Le `match` est **exhaustif sans bras attrape-tout**, et délibérément :
/// [`PermissionVerdict`] est défini ici, donc ajouter un verdict casse cette
/// fonction à la compilation plutôt que de le faire tomber en silence dans un
/// défaut. C'est le seul endroit du module où ce choix est possible — pour
/// [`ToolKind`], qui vient du protocole, l'attrape-tout est au contraire
/// obligatoire.
#[must_use]
pub fn option_for<'a>(
    verdict: &PermissionVerdict,
    options: &'a [PermissionOption],
) -> Option<&'a PermissionOption> {
    match verdict {
        PermissionVerdict::Granted => options
            .iter()
            .find(|option| option.kind == PermissionOptionKind::AllowOnce),
        PermissionVerdict::Refused(_) => options
            .iter()
            .find(|option| option.kind == PermissionOptionKind::RejectAlways)
            .or_else(|| {
                options
                    .iter()
                    .find(|option| option.kind == PermissionOptionKind::RejectOnce)
            }),
    }
}

pub mod confine;
pub mod locate;
pub mod mcp;
pub mod presets;
pub mod prompt;
pub mod session;
pub mod settings;
pub mod spawn;
pub mod turn;

#[cfg(test)]
mod tests;
