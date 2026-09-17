//! Le câblage du runtime d'agents sur l'ordonnanceur — et rien d'autre.
//!
//! `oxyn-ai` définit le trait [`CommandSink`], `oxyn-exec` fournit
//! [`ExecutorSink`], et aucune des deux crates ne connaît l'autre : le contrat
//! de dépendances l'interdit, et `oxyn-exec/src/sink.rs` explique pourquoi. La
//! jonction revient donc ici, à la seule crate qui dépend des deux.
//!
//! Ce module est **délibérément sans logique**. Toute règle qu'on y ajouterait
//! serait une règle que l'interface n'applique pas, donc un second chemin
//! d'exécution à auditer séparément — exactement ce qu'[I-01](../../../CLAUDE.md#i-01)
//! et [ADR-0004](../../../docs/adr/0004-command-bus.md) refusent. Ce qui
//! ressemble à une décision de produit dans un agent se décide dans le
//! `PolicyGate`, pas dans une traduction de types.
//!
//! # Ce qui ne passe pas par ici
//!
//! Le niveau de confidentialité. Un [`DispatchOutcome`] porte les faits — le
//! message du serveur entier, classe comprise — et c'est le runtime IA qui le
//! filtre, parce que le niveau appartient à la connexion et que lui seul le
//! connaît ([I-04](../../../CLAUDE.md#i-04)). Un adaptateur qui filtrerait de
//! son côté serait un second endroit où l'oublier.

mod session;
mod settings;

pub(crate) use session::{AiEvent, ChannelObserver};
pub(crate) use settings::{config_of, declared};

use std::sync::Arc;

use async_trait::async_trait;
use oxyn_ai::{CommandSink, DispatchOutcome};
use oxyn_core::{Actor, CancelToken, Command};
use oxyn_exec::{DispatchReport, Executor, ExecutorSink};

/// Le puits par lequel une conversation atteint l'ordonnanceur.
///
/// Construit par [`for_agent`](Self::for_agent) : un puits attaché refuse toute
/// commande portant un autre acteur, `Actor::Human` compris. C'est ce qui
/// empêche un agent de se présenter comme l'utilisateur, et donc d'échapper à
/// la moitié « agent » de la matrice de politique.
pub struct AgentSink(ExecutorSink);

impl AgentSink {
    /// Attache un puits à un agent et à sa conversation.
    #[must_use]
    pub fn for_agent(
        executor: Arc<Executor>,
        agent: oxyn_core::AgentId,
        session: oxyn_core::AgentSessionId,
    ) -> Self {
        Self(ExecutorSink::for_agent(executor, agent, session))
    }
}

impl std::fmt::Debug for AgentSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentSink").finish_non_exhaustive()
    }
}

#[async_trait]
impl CommandSink for AgentSink {
    async fn dispatch(
        &self,
        actor: Actor,
        command: Command,
        cancel: &CancelToken,
    ) -> DispatchOutcome {
        translate(self.0.dispatch(actor, command, cancel).await)
    }
}

/// Traduit un rapport d'exécution en ce que le runtime IA attend.
///
/// Une correspondance de variantes, sans jugement. Un seul point mérite d'être
/// dit : le `summary` du rapport est repris **tel quel**, y compris quand des
/// statistiques l'accompagnent. Il est calculé au plus près des faits et sait
/// une chose que les statistiques ignorent — qu'un résultat a été tronqué par
/// le tampon et pas seulement par la limite de lignes. Le reconstruire depuis
/// `ExecStats` perdrait cette mention, et un modèle qui croit tenir tout le
/// résultat en tire des conclusions fausses sur des données réelles.
fn translate(report: DispatchReport) -> DispatchOutcome {
    match report {
        DispatchReport::Completed { summary, .. } => DispatchOutcome::Completed { summary },
        DispatchReport::AwaitingApproval { reason, .. } => {
            DispatchOutcome::AwaitingApproval { reason }
        }
        DispatchReport::Denied { reason, .. } => DispatchOutcome::Denied { reason },
        DispatchReport::Failed { class, error, .. } => DispatchOutcome::Failed {
            class,
            message: error,
        },
        // `DispatchReport` est `#[non_exhaustive]` : une variante ajoutée dans
        // `oxyn-exec` atterrit ici sans que ce module ait été relu. Le refus
        // est le seul rendu qui ne mente pas — un succès laisserait le modèle
        // supposer un effet, et un échec retentable l'inviterait à rejouer une
        // commande dont on ne sait pas si elle a produit quelque chose (I-13).
        autre => {
            // La commande, jamais le rapport. `DispatchReport::Failed` porte le
            // message du serveur, valeurs de ligne comprises, et une variante
            // future en portera autant : un `?` ici ferait du journal — canal
            // n° 1 des six — le chemin de fuite qu'I-03 nomme.
            tracing::error!(
                command = %autre.command(),
                "rapport d'exécution non traduit : l'adaptateur d'agents est en retard sur oxyn-exec"
            );
            DispatchOutcome::Denied {
                reason: "this Oxyn build cannot interpret the result of that command; \
                         do not assume it ran, and do not retry it"
                    .to_owned(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::{CommandId, ErrorClass, ExecStats};

    #[test]
    fn la_troncature_du_tampon_survit_a_la_traduction() {
        // Le rapport le sait, `ExecStats` non : `stats.truncated` reste faux
        // quand c'est le tampon qui a coupé. Reconstruire le résumé depuis les
        // statistiques ferait disparaître l'avertissement.
        let rapport = DispatchReport::Completed {
            command: CommandId::new(),
            summary: "5 rows, 1 batches (truncated; this is not the whole result)".to_owned(),
            stats: Some(ExecStats {
                rows: 5,
                batches: 1,
                truncated: false,
                ..ExecStats::default()
            }),
        };

        let DispatchOutcome::Completed { summary } = translate(rapport) else {
            panic!("une exécution réussie reste une exécution réussie");
        };
        assert!(
            summary.contains("truncated"),
            "le modèle doit savoir que ce n'est pas tout le résultat : {summary}"
        );
    }

    #[test]
    fn une_approbation_en_attente_ne_devient_pas_un_succes() {
        // Le piège que ce test ferme : un modèle qui suppose que son écriture a
        // eu lieu et enchaîne sur cette hypothèse.
        let rapport = DispatchReport::AwaitingApproval {
            command: CommandId::new(),
            reason: "write on a production connection".to_owned(),
        };
        assert_eq!(
            translate(rapport),
            DispatchOutcome::AwaitingApproval {
                reason: "write on a production connection".to_owned(),
            }
        );
    }

    #[test]
    fn un_refus_reste_un_refus_definitif() {
        let rapport = DispatchReport::Denied {
            command: CommandId::new(),
            reason: "agents may not write on production".to_owned(),
        };
        assert!(matches!(translate(rapport), DispatchOutcome::Denied { .. }));
    }

    #[test]
    fn la_classe_de_l_erreur_est_transmise_sans_etre_rededuite() {
        // Une expiration est ambiguë : le serveur a peut-être appliqué (I-13).
        // La classe voyage comme donnée ; la redéduire du message casserait en
        // silence le jour où le message change.
        let rapport = DispatchReport::Failed {
            command: CommandId::new(),
            class: ErrorClass::Ambiguous,
            error: "timed out after 30s".to_owned(),
        };
        assert_eq!(
            translate(rapport),
            DispatchOutcome::Failed {
                class: ErrorClass::Ambiguous,
                message: "timed out after 30s".to_owned(),
            }
        );
    }

    #[test]
    fn le_message_du_serveur_arrive_entier_au_runtime() {
        // Ce module ne filtre pas : le niveau appartient à la connexion, et
        // c'est `ToolOutcome::from_dispatch` qui l'applique (I-04). Un filtre
        // ici serait un second endroit où l'oublier.
        let cite = "Key (email)=(dupont@example.com) already exists";
        let rapport = DispatchReport::Failed {
            command: CommandId::new(),
            class: ErrorClass::Permanent,
            error: cite.to_owned(),
        };
        let DispatchOutcome::Failed { message, .. } = translate(rapport) else {
            panic!("un échec reste un échec");
        };
        assert_eq!(message, cite);
    }
}
