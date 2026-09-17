//! Ce qui remonte vers l'interface pendant l'exécution d'une commande.
//!
//! Ces événements sont le pendant des cinq états d'une vue
//! ([`UX-SPEC`](../../../docs/UX-SPEC.md)) : ils permettent d'afficher une
//! progression réelle et un moyen d'annuler, plutôt qu'un gel suivi d'un
//! résultat.
//!
//! Deux règles de fond :
//!
//! * **rien d'optimiste.** Aucun événement n'annonce un succès avant que le
//!   serveur ne l'ait confirmé : [`Completed`](Event::Completed) arrive après la
//!   fin du flux, pas à la soumission ;
//! * **aucune valeur de la base ici.** Les lignes voyagent en `RecordBatch`
//!   (ADR-0002) ; un événement ne porte que des compteurs et des identifiants.

use serde::{Deserialize, Serialize};

use crate::error::OxynError;
use crate::ids::{CommandId, ResultId};
use crate::policy::Preview;
use crate::query::StatementIntent;
use crate::stats::ExecStats;

/// Un événement d'exécution destiné à l'interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
#[non_exhaustive]
pub enum Event {
    /// Le schéma du résultat est connu : les colonnes peuvent être dessinées
    /// avant qu'une seule ligne n'arrive.
    SchemaReady {
        /// Le résultat concerné.
        result: ResultId,
    },

    /// Un lot est disponible dans le tampon de résultats.
    BatchReady {
        /// Le résultat concerné.
        result: ResultId,
        /// Lignes contenues dans ce lot.
        rows: usize,
    },

    /// Progression, pour les exécutions sans schéma ni lot immédiat.
    Progress {
        /// Lignes traitées jusqu'ici.
        rows: u64,
    },

    /// L'exécution est terminée avec succès.
    Completed {
        /// Le résultat produit.
        result: ResultId,
        /// Ce qu'elle a coûté.
        stats: ExecStats,
        /// Ce que l'instruction faisait, **tel que le classificateur l'a lu**
        /// après reclassification — pas ce que l'appelant avait déclaré.
        ///
        /// C'est ce qui permet à une vue de savoir qu'un rafraîchissement a du
        /// sens : une écriture ou un DDL périme ce qui est affiché, une lecture
        /// non ([ADR-0022](../../docs/adr/0022-rafraichissement-automatique.md)).
        /// L'événement ne dit pas **quel objet** a changé : le classificateur ne
        /// nomme pas les tables, et prétendre le contraire produirait des
        /// invalidations fausses dans les deux sens.
        intent: StatementIntent,
    },

    /// L'exécution a échoué.
    Failed {
        /// Le message, tel qu'il sera montré : celui du serveur, code compris,
        /// et non une paraphrase rassurante.
        error: String,
        /// L'opération est-elle rejouable telle quelle ?
        ///
        /// L'interface doit pouvoir répondre « est-ce retentable » sans analyser
        /// le message (UX-SPEC, DRIVER-CONTRACT §4).
        retryable: bool,
    },

    /// Le `PolicyGate` demande un accord avant d'exécuter.
    ApprovalRequested {
        /// La commande en attente.
        command: CommandId,
        /// Ce sur quoi l'utilisateur doit se prononcer.
        reason: String,
        /// De quoi juger sans aller lire ailleurs.
        preview: Option<Preview>,
    },

    /// L'exécution a été interrompue.
    Cancelled,

    /// Le catalogue a changé : l'arborescence doit être relue.
    CatalogUpdated,
}

impl Event {
    /// Construit un événement d'échec à partir d'une erreur, en reportant sa
    /// classe plutôt qu'en la laissant déduire.
    #[must_use]
    pub fn failed(error: &OxynError) -> Self {
        Self::Failed {
            error: error.to_string(),
            retryable: error.is_retryable(),
        }
    }

    /// L'événement clôt-il l'exécution ?
    ///
    /// Après un événement terminal, plus rien n'arrive pour cette exécution :
    /// c'est le signal qui autorise l'interface à quitter l'état « en cours ».
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Failed { .. } | Self::Cancelled
        )
    }

    /// Le résultat concerné, quand il y en a un.
    #[must_use]
    pub const fn result(&self) -> Option<ResultId> {
        match self {
            Self::SchemaReady { result }
            | Self::BatchReady { result, .. }
            | Self::Completed { result, .. } => Some(*result),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn les_evenements_terminaux_sont_les_trois_attendus() {
        let resultat = ResultId::new();
        assert!(
            Event::Completed {
                result: resultat,
                stats: ExecStats::default(),
                intent: StatementIntent::Read,
            }
            .is_terminal()
        );
        assert!(
            Event::Failed {
                error: "boum".into(),
                retryable: false,
            }
            .is_terminal()
        );
        assert!(Event::Cancelled.is_terminal());

        assert!(!Event::SchemaReady { result: resultat }.is_terminal());
        assert!(!Event::Progress { rows: 10 }.is_terminal());
        assert!(!Event::CatalogUpdated.is_terminal());
    }

    #[test]
    fn un_echec_reporte_la_classe_de_l_erreur() {
        let transitoire = Event::failed(&OxynError::Connection("réseau coupé".into()));
        let Event::Failed { retryable, error } = transitoire else {
            panic!("mauvaise variante");
        };
        assert!(retryable, "une coupure réseau est retentable");
        assert!(
            error.contains("réseau coupé"),
            "le message du serveur est montré"
        );

        let ambigu = Event::failed(&OxynError::Timeout {
            after: Duration::from_secs(30),
        });
        let Event::Failed { retryable, .. } = ambigu else {
            panic!("mauvaise variante");
        };
        assert!(
            !retryable,
            "une expiration est ambiguë : l'interface ne doit pas proposer de rejouer"
        );
    }

    #[test]
    fn le_resultat_concerne_est_retrouvable() {
        let resultat = ResultId::new();
        assert_eq!(
            Event::BatchReady {
                result: resultat,
                rows: 1_024,
            }
            .result(),
            Some(resultat)
        );
        assert_eq!(Event::Cancelled.result(), None);
    }

    #[test]
    fn une_demande_d_approbation_porte_de_quoi_juger() {
        let evt = Event::ApprovalRequested {
            command: CommandId::new(),
            reason: "TRUNCATE : vidage complet de la table".into(),
            preview: Some(Preview::new("TRUNCATE audit", "caisse")),
        };
        let Event::ApprovalRequested { preview, .. } = &evt else {
            panic!("mauvaise variante");
        };
        let preview = preview.as_ref().expect("prévisualisation attendue");
        assert_eq!(preview.connection, "caisse");
        assert!(
            !evt.is_terminal(),
            "l'exécution n'est pas close, elle attend"
        );
    }

    #[test]
    fn aller_retour_json() {
        let evt = Event::Completed {
            // Une écriture, pour que l'aller-retour porte sur autre chose que
            // la valeur par défaut de l'intention.
            intent: StatementIntent::Write,
            result: ResultId::new(),
            stats: ExecStats {
                rows: 3,
                bytes: 96,
                server_time: Some(Duration::from_millis(2)),
                total_time: Duration::from_millis(9),
                batches: 1,
                truncated: false,
            },
        };
        let json = serde_json::to_string(&evt).expect("sérialisation");
        let relu: Event = serde_json::from_str(&json).expect("désérialisation");
        assert_eq!(evt, relu);
    }
}
