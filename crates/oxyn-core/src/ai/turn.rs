//! Le vocabulaire d'un tour de conversation qu'Oxyn **persiste**.
//!
//! # Pourquoi ces trois types vivent ici et non dans `oxyn-llm`
//!
//! [`ReasoningBlock`] et [`StopReason`] sont écrits en base par `oxyn-store`,
//! **tels quels** : un bloc de raisonnement se renvoie au fournisseur à
//! l'identique au tour suivant, et la raison d'arrêt garde le mot du
//! fournisseur. Les recopier en aval en ferait deux définitions ; et parce
//! qu'ils sont `#[non_exhaustive]`, la conversion devrait passer par un bras
//! `_ =>` — c'est-à-dire perdre en silence la variante ajoutée plus tard, pour
//! un échec qui ne sortirait que chez l'utilisateur, sous la forme « le
//! fournisseur refuse mon tour ».
//!
//! Laisser `oxyn-store` dépendre d'`oxyn-llm` pour les lire faisait entrer un
//! client HTTP dans l'arbre d'`oxyn-exec`, qui ne fait jamais de réseau
//! lui-même : le sens des dépendances
//! d'[ARCHITECTURE](../../../../docs/ARCHITECTURE.md) était cassé. C'est du
//! **vocabulaire commun**, pas du protocole, et il a donc sa place ici — c'est le
//! précédent exact de [`ProviderId`](super::ProviderId). `oxyn-llm` les
//! ré-exporte : ils n'ont qu'une définition dans le dépôt.
//!
//! [`Role`] suit pour une raison plus modeste, et elle est dite : il n'est pas
//! persisté, mais `oxyn-store` le convertit vers son propre rôle ouvert. Le
//! laisser en arrière aurait gardé la dépendance pour une seule conversion.
//!
//! # Ce qui n'est **pas** ici
//!
//! La traduction depuis une chaîne de protocole (`stop`, `end_turn`,
//! `pause_turn`…). Elle appartient à chaque fournisseur, et elle reste dans
//! `oxyn-llm` : le cœur ne connaît aucun protocole.
//!
//! # La forme sérialisée est un format de persistance
//!
//! `oxyn-store` fait `serde_json::to_string` directement sur ces types. Leurs
//! attributs `serde` **sont** donc le format des conversations sur disque :
//! renommer une variante, changer un `tag` ou un `rename_all` rend illisibles,
//! sans erreur, les tours déjà écrits. Les tests de ce module figent la forme
//! exacte ; un échec y est une migration à écrire, pas un test à ajuster.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Qui parle dans un tour de conversation.
///
/// L'énumération est **fermée** : ces quatre rôles sont le vocabulaire commun
/// des trois familles de protocoles supportées, et chaque traduction vers un
/// protocole doit les couvrir toutes. Ajouter un rôle doit casser la
/// compilation de chaque fournisseur, pas être silencieusement ignoré.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Consigne de cadrage, posée par Oxyn et jamais par l'utilisateur.
    System,
    /// Tour de l'utilisateur — ou du contexte qu'Oxyn assemble pour lui.
    User,
    /// Tour du modèle.
    Assistant,
    /// Résultat de l'exécution d'un outil, renvoyé au modèle.
    Tool,
}

impl Role {
    /// Nom stable, celui qui part sur le fil pour les protocoles compatibles
    /// OpenAI.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Raison pour laquelle un flux s'est arrêté.
///
/// **Représentation `serde` externe par défaut, sans renommage** : c'est le
/// format de la colonne `stop_reason` des conversations persistées. Voir la
/// note du module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum StopReason {
    /// Le modèle a terminé son tour.
    EndTurn,
    /// Le plafond de jetons a été atteint : la réponse est **tronquée**.
    MaxTokens,
    /// Le modèle demande l'exécution d'outils avant de continuer.
    ToolCalls,
    /// Le fournisseur a interrompu pour cause de filtrage de contenu.
    ContentFilter,
    /// Une séquence d'arrêt demandée par l'appelant a été rencontrée.
    ///
    /// Distincte de [`MaxTokens`](Self::MaxTokens) : la réponse s'arrête là où
    /// l'appelant l'a voulu, elle n'est pas tronquée par accident.
    StopSequence,
    /// Le modèle a refusé de répondre.
    ///
    /// Distincte de [`ContentFilter`](Self::ContentFilter), qui est une
    /// intervention du fournisseur **sur** une réponse produite : ici il n'y a
    /// pas de réponse. Ce que l'interface doit dire n'est donc pas le même.
    Refusal,
    /// Le tour est en **pause** et attend d'être repris.
    ///
    /// Le fournisseur a atteint une limite interne — un nombre de tours
    /// d'outils côté serveur, typiquement — et rend la main sans que la réponse
    /// soit finie. Rien n'a échoué, et rien n'est complet.
    Paused,
    /// La réponse a rempli la fenêtre de contexte du modèle.
    ///
    /// La réponse est tronquée, et augmenter le plafond de jetons n'y changera
    /// rien : c'est l'entrée qu'il faut réduire.
    ContextWindowExceeded,
    /// L'appelant a annulé via le `CancelToken`.
    Cancelled,
    /// Le flux s'est arrêté **avant la fin du tour**, sans que le fournisseur
    /// l'annonce : connexion coupée, tampon dépassé, trames illisibles en
    /// série.
    ///
    /// C'est le cas **ambigu** d'[I-13](../../../../CLAUDE.md#i-13), et le seul
    /// de cette énumération : le serveur a peut-être terminé la génération — et
    /// l'a donc facturée — alors que rien n'est arrivé jusqu'ici. Rejouer le
    /// tour paie une deuxième fois une réponse déjà produite.
    /// Voir [`is_ambiguous`](Self::is_ambiguous).
    Interrupted,
    /// Le fournisseur a signalé une erreur **dans** le flux, après avoir
    /// commencé à répondre.
    ///
    /// Distinct d'[`Interrupted`](Self::Interrupted) : ici le fournisseur a
    /// annoncé son échec, il n'y a donc pas de doute sur ce qui s'est passé de
    /// son côté. La réponse reste incomplète.
    ProviderError,
    /// Le flux s'est terminé sans que le fournisseur en donne la raison.
    Unspecified,
    /// Raison propre au fournisseur, conservée telle quelle plutôt que
    /// rabattue sur une variante voisine.
    Other(String),
}

impl StopReason {
    /// La réponse est-elle incomplète du fait de l'arrêt ?
    ///
    /// À montrer dans l'interface : une réponse coupée au plafond de jetons qui
    /// ne le dit pas ressemble à une réponse fausse.
    ///
    /// [`StopSequence`](Self::StopSequence) répond `false` : la réponse
    /// s'arrête là où l'appelant l'a demandé. [`Paused`](Self::Paused) répond
    /// `true` — rien n'a échoué, mais il manque la suite.
    ///
    /// [`Other`](Self::Other) répond `false` **délibérément** : c'est une
    /// raison que le fournisseur a nommée et que cette version ne connaît pas,
    /// donc une fin de tour ordinaire jusqu'à preuve du contraire. Une coupure
    /// n'y tombe pas : elle a sa variante,
    /// [`Interrupted`](Self::Interrupted). Faire l'inverse — une coupure
    /// rangée dans `Other` — présenterait une réponse tranchée en plein milieu
    /// comme une réponse complète.
    #[must_use]
    pub const fn is_truncated(&self) -> bool {
        matches!(
            self,
            Self::MaxTokens
                | Self::ContentFilter
                | Self::Refusal
                | Self::Paused
                | Self::ContextWindowExceeded
                | Self::Cancelled
                | Self::Interrupted
                | Self::ProviderError
        )
    }

    /// Ignore-t-on ce que le fournisseur a réellement produit ?
    ///
    /// C'est la question d'[I-13](../../../../CLAUDE.md#i-13) posée à une
    /// génération : un tour dont le flux a été coupé a peut-être été terminé —
    /// et facturé — côté serveur. **Rejouer un tour ambigu paie deux fois.**
    ///
    /// Un seul cas répond `true`, et c'est voulu : partout ailleurs, ou bien le
    /// fournisseur a annoncé la fin, ou bien c'est l'appelant qui a décidé
    /// d'arrêter. Aucune crate du dépôt ne rejoue un tour d'elle-même ; cette
    /// méthode existe pour que l'appelant qui y songerait ait la donnée, plutôt
    /// que de la déduire d'une chaîne de caractères.
    #[must_use]
    pub const fn is_ambiguous(&self) -> bool {
        matches!(self, Self::Interrupted)
    }
}

/// Un bloc de raisonnement produit par le modèle.
///
/// **Ce type est un transport, pas un contenu à lire.** Il existe pour être
/// remis au fournisseur au tour suivant, à l'identique : c'est ce que les
/// protocoles exigent quand un tour de raisonnement précède un appel d'outil.
/// Le reconstruire à partir de ses morceaux, le réordonner ou en supprimer un
/// fait refuser la requête suivante.
///
/// Sa forme `serde` (`tag = "kind"`, `snake_case`) est le format de la colonne
/// `reasoning` des conversations persistées. Voir la note du module.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReasoningBlock {
    /// Raisonnement rédigé, tel que le fournisseur accepte de le montrer.
    ///
    /// Le texte peut être **vide** : plusieurs protocoles renvoient le bloc sans
    /// son texte quand l'appelant n'a pas demandé à le voir. Le bloc reste
    /// nécessaire au tour suivant, et c'est pourquoi on le garde quand même.
    Summarized {
        /// Ce que le modèle accepte de montrer. Vide est licite.
        text: String,
        /// Charge opaque qui authentifie le bloc auprès du fournisseur.
        ///
        /// Ne s'interprète pas, ne se compare pas, ne s'affiche pas : elle
        /// n'a de sens que pour l'émetteur.
        signature: Option<String>,
    },
    /// Raisonnement chiffré par le fournisseur.
    ///
    /// Il n'a **pas** de texte, et il ne s'affiche jamais. Il se renvoie tel
    /// quel, sans quoi le tour suivant perd le fil du raisonnement.
    Redacted {
        /// Charge opaque. Voir la note de variante : elle ne s'affiche pas.
        data: String,
    },
}

impl ReasoningBlock {
    /// Construit un bloc rédigé.
    #[must_use]
    pub fn summarized(text: impl Into<String>, signature: Option<String>) -> Self {
        Self::Summarized {
            text: text.into(),
            signature,
        }
    }

    /// Construit un bloc chiffré.
    #[must_use]
    pub fn redacted(data: impl Into<String>) -> Self {
        Self::Redacted { data: data.into() }
    }

    /// Le texte à montrer, s'il y en a un.
    ///
    /// Rend `None` pour un bloc chiffré **et** pour un bloc rédigé dont le
    /// texte est vide : dans les deux cas il n'y a rien à afficher, et une
    /// interface qui rendrait une chaîne vide dessinerait un cadre vide.
    #[must_use]
    pub fn display_text(&self) -> Option<&str> {
        match self {
            Self::Summarized { text, .. } if !text.is_empty() => Some(text),
            _ => None,
        }
    }

    /// Le bloc est-il chiffré ?
    #[must_use]
    pub const fn is_redacted(&self) -> bool {
        matches!(self, Self::Redacted { .. })
    }
}

impl fmt::Debug for ReasoningBlock {
    /// Montre le texte, jamais la charge opaque.
    ///
    /// Une signature pèse des centaines d'octets et n'aide personne à
    /// diagnostiquer un flux ; la recopier dans un journal ne ferait que le
    /// rendre illisible. Le texte, lui, est la sortie du modèle.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Summarized { text, signature } => f
                .debug_struct("Summarized")
                .field("text", text)
                .field("signature", &signature.as_ref().map(|s| Opaque(s.len())))
                .finish(),
            Self::Redacted { data } => f
                .debug_struct("Redacted")
                .field("data", &Opaque(data.len()))
                .finish(),
        }
    }
}

/// Marqueur de charge opaque, rendu `<opaque, N octets>`.
struct Opaque(usize);

impl fmt::Debug for Opaque {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<opaque, {} octets>", self.0)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // ── Format de persistance, figé ────────────────────────────────────────
    //
    // Ces tests ne vérifient pas un comportement : ils figent un format sur
    // disque. Un échec ici n'est pas un test à ajuster, c'est une migration des
    // conversations persistées à écrire.

    #[test]
    fn la_forme_persistee_d_une_raison_d_arret_est_figee() {
        for (raison, attendu) in [
            (StopReason::EndTurn, json!("EndTurn")),
            (StopReason::MaxTokens, json!("MaxTokens")),
            (StopReason::ToolCalls, json!("ToolCalls")),
            (StopReason::ContentFilter, json!("ContentFilter")),
            (StopReason::StopSequence, json!("StopSequence")),
            (StopReason::Refusal, json!("Refusal")),
            (StopReason::Paused, json!("Paused")),
            (
                StopReason::ContextWindowExceeded,
                json!("ContextWindowExceeded"),
            ),
            (StopReason::Cancelled, json!("Cancelled")),
            (StopReason::Interrupted, json!("Interrupted")),
            (StopReason::ProviderError, json!("ProviderError")),
            (StopReason::Unspecified, json!("Unspecified")),
            (
                StopReason::Other("guardrail_intervened".to_owned()),
                json!({"Other": "guardrail_intervened"}),
            ),
        ] {
            assert_eq!(
                serde_json::to_value(&raison).expect("sérialisation"),
                attendu,
                "{raison:?}"
            );
            let relue: StopReason = serde_json::from_value(attendu).expect("désérialisation");
            assert_eq!(relue, raison);
        }
    }

    #[test]
    fn la_forme_persistee_d_un_bloc_de_raisonnement_est_figee() {
        for (bloc, attendu) in [
            (
                ReasoningBlock::summarized("je compte", Some("SIG".to_owned())),
                json!({"kind": "summarized", "text": "je compte", "signature": "SIG"}),
            ),
            (
                ReasoningBlock::summarized("", None),
                json!({"kind": "summarized", "text": "", "signature": null}),
            ),
            (
                ReasoningBlock::redacted("CHIFFRE"),
                json!({"kind": "redacted", "data": "CHIFFRE"}),
            ),
        ] {
            assert_eq!(
                serde_json::to_value(&bloc).expect("sérialisation"),
                attendu,
                "{bloc:?}"
            );
            let relu: ReasoningBlock = serde_json::from_value(attendu).expect("désérialisation");
            assert_eq!(relu, bloc);
        }
    }

    #[test]
    fn la_forme_serialisee_d_un_role_est_figee() {
        for (role, attendu) in [
            (Role::System, "system"),
            (Role::User, "user"),
            (Role::Assistant, "assistant"),
            (Role::Tool, "tool"),
        ] {
            assert_eq!(
                serde_json::to_value(role).expect("sérialisation"),
                json!(attendu)
            );
            assert_eq!(
                role.as_str(),
                attendu,
                "le nom de fil et la forme serde coïncident"
            );
        }
    }

    // ── Raisons d'arrêt ────────────────────────────────────────────────────

    #[test]
    fn une_reponse_coupee_se_signale() {
        assert!(StopReason::MaxTokens.is_truncated());
        assert!(StopReason::Cancelled.is_truncated());
        assert!(!StopReason::EndTurn.is_truncated());
        assert!(!StopReason::ToolCalls.is_truncated());
    }

    #[test]
    fn une_reponse_refusee_ou_en_pause_se_signale_comme_incomplete() {
        assert!(StopReason::Refusal.is_truncated());
        assert!(StopReason::Paused.is_truncated());
        assert!(StopReason::ContextWindowExceeded.is_truncated());
        assert!(
            !StopReason::StopSequence.is_truncated(),
            "l'arrêt demandé par l'appelant n'est pas une troncature"
        );
    }

    #[test]
    fn une_coupure_de_flux_est_la_seule_raison_ambigue() {
        // I-13 : rejouer un tour dont on ignore le sort côté serveur paie deux
        // fois une réponse peut-être déjà produite.
        assert!(StopReason::Interrupted.is_ambiguous());
        assert!(StopReason::Interrupted.is_truncated());

        for certaine in [
            StopReason::EndTurn,
            StopReason::MaxTokens,
            StopReason::StopSequence,
            StopReason::ToolCalls,
            StopReason::ContentFilter,
            StopReason::Refusal,
            StopReason::Paused,
            StopReason::ContextWindowExceeded,
            StopReason::Cancelled,
            StopReason::ProviderError,
            StopReason::Unspecified,
        ] {
            assert!(
                !certaine.is_ambiguous(),
                "{certaine:?} : le sort du tour est connu, ou c'est l'appelant qui a décidé"
            );
        }
    }

    #[test]
    fn une_erreur_annoncee_par_le_fournisseur_tronque_sans_etre_ambigue() {
        assert!(StopReason::ProviderError.is_truncated());
        assert!(
            !StopReason::ProviderError.is_ambiguous(),
            "le fournisseur a annoncé son échec : il n'y a pas de doute sur ce qu'il a fait"
        );
    }

    #[test]
    fn une_raison_inconnue_du_fournisseur_reste_une_fin_ordinaire() {
        // `Other` porte une raison **nommée** par le fournisseur : la traiter
        // comme une troncature ferait passer chaque évolution de protocole pour
        // une réponse coupée.
        let future = StopReason::Other("guardrail_intervened".to_owned());
        assert!(!future.is_truncated());
        assert!(!future.is_ambiguous());
    }

    // ── Blocs de raisonnement ──────────────────────────────────────────────

    #[test]
    fn un_bloc_chiffre_n_a_rien_a_montrer() {
        let bloc = ReasoningBlock::redacted("EncRypTeD");
        assert!(bloc.is_redacted());
        assert_eq!(bloc.display_text(), None);
    }

    #[test]
    fn un_bloc_rendu_vide_reste_transportable_mais_ne_s_affiche_pas() {
        // Le cas courant quand l'appelant n'a pas demandé à voir le
        // raisonnement : le bloc arrive sans texte et doit quand même être
        // renvoyé au tour suivant.
        let bloc = ReasoningBlock::summarized("", Some("sig".to_owned()));
        assert_eq!(bloc.display_text(), None);
        assert!(!bloc.is_redacted());
    }

    #[test]
    fn le_debug_ne_recopie_pas_la_charge_opaque() {
        let rendu = format!(
            "{:?}",
            ReasoningBlock::summarized("j'additionne", Some("SIGNATURE_TRES_LONGUE".to_owned()))
        );
        assert!(!rendu.contains("SIGNATURE_TRES_LONGUE"), "{rendu}");
        assert!(rendu.contains("j'additionne"), "{rendu}");

        let chiffre = format!("{:?}", ReasoningBlock::redacted("CHARGE_CHIFFREE"));
        assert!(!chiffre.contains("CHARGE_CHIFFREE"), "{chiffre}");
        assert!(chiffre.contains("opaque"), "{chiffre}");
    }
}
