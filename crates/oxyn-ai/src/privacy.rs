//! Le niveau de confidentialité d'une connexion, appliqué à un point d'accès.
//!
//! Autorité : [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md). Le tableau
//! des niveaux y vit et n'est pas recopié ici.
//!
//! # Le type vit dans `oxyn-core`, et c'est le sujet
//!
//! [`PrivacyTier`] est défini dans
//! [`oxyn_core::connection`] et ré-exporté ici. Il n'y a
//! **qu'une** définition dans le dépôt, et elle se trouve à côté de la
//! [`ConnectionConfig`](oxyn_core::ConnectionConfig) qui la porte : un niveau
//! rangé dans la crate d'IA serait un niveau attaché au workspace IA, donc un
//! réglage global sous un autre nom — exactement ce qu'ADR-0006 refuse.
//!
//! Ce module ne porte donc que ce qui a besoin d'`oxyn-llm` : la confrontation
//! du niveau avec le classement d'un point d'accès.
//!
//! # `Local` est une garantie, pas une préférence
//!
//! [`PrivacyTier::allows_remote_provider`] rend `false` pour `Local`, et
//! [`AgentRuntime::run`](crate::runtime::AgentRuntime::run) refuse avant
//! d'assembler quoi que ce soit : il n'existe pas de chemin qui envoie une
//! invite hors de la machine sous ce niveau. La vérification a lieu sur la
//! **session**, parce que c'est le seul endroit où le niveau de la connexion
//! est connu.
//!
//! # Le piège du mandataire sur `localhost`
//!
//! Le classement local/distant se fait sur l'hôte réel **après résolution**
//! ([`Reach`]), jamais sur la présence de `localhost` dans une URL : un point
//! d'accès compatible OpenAI en écoute sur la boucle locale peut être un
//! mandataire qui réémet vers le nuage. Il se re-vérifie à chaque changement de
//! configuration, parce que le nom qui résolvait vers `127.0.0.1` hier peut
//! résoudre ailleurs aujourd'hui.

use oxyn_llm::Reach;

pub use oxyn_core::PrivacyTier;

/// Ce point d'accès est-il utilisable sous ce niveau ?
///
/// [`Reach::Unresolved`] est traité comme distant : un point d'accès qu'on n'a
/// pas su classer n'obtient pas le bénéfice du doute
/// ([`Reach::leaves_machine`]).
///
/// Fonction libre plutôt que méthode : le classement d'un point d'accès vit
/// dans `oxyn-llm`, dont `oxyn-core` ne dépend pas — et ne doit pas dépendre.
#[must_use]
pub const fn allows_endpoint(tier: PrivacyTier, reach: Reach) -> bool {
    !reach.leaves_machine() || tier.allows_remote_provider()
}

/// La portée d'un agent externe : **inconnaissable**, donc [`Reach::Unresolved`].
///
/// Ce n'est pas la même chose qu'un point d'accès mal résolu. Un fournisseur
/// déclaré a une URL qu'on peut résoudre, et dont la résolution peut être
/// périmée ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
/// Un agent externe est un **processus opaque** : il peut parler à un modèle
/// local, à un service distant, ou changer entre deux tours, et rien dans le
/// protocole ne permet de le lui demander
/// ([ADR-0026](../../../docs/adr/0026-agents-externes-acp.md)).
///
/// La fonction prend la déclaration pour que l'appelant ne puisse pas se
/// tromper de valeur, et rend une constante parce qu'il n'y a rien à calculer :
/// c'est l'absence d'information qui est modélisée, pas une mesure ratée.
#[must_use]
pub const fn agent_reach(_agent: &oxyn_core::ExternalAgentConfig) -> Reach {
    Reach::Unresolved
}

/// Cet agent externe est-il utilisable sous ce niveau ?
///
/// Conséquence directe d'[`agent_reach`] : **non sous `Local`**, oui sous
/// `Metadata` et `Sampled`. Un utilisateur dont l'agent tourne réellement
/// contre un modèle local trouvera la restriction excessive, et il aura raison
/// sur le fond — mais la lever demanderait de le croire sur parole, et `Local`
/// promet « rien ne sort de la machine ». Une promesse assortie d'une case à
/// cocher n'est plus une promesse.
#[must_use]
pub const fn allows_external_agent(
    tier: PrivacyTier,
    agent: &oxyn_core::ExternalAgentConfig,
) -> bool {
    allows_endpoint(tier, agent_reach(agent))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un agent externe ne sert jamais une connexion `Local`.
    ///
    /// C'est la conséquence qui compte d'ADR-0026, et elle ne tient à aucune
    /// mécanique nouvelle : un agent est `Unresolved`, et « dans le doute, on
    /// protège » était déjà la règle.
    #[test]
    fn un_agent_externe_ne_sert_jamais_une_connexion_locale() {
        use oxyn_core::{ExternalAgentConfig, ProviderId};

        let agent = ExternalAgentConfig::new(
            ProviderId::new("agent-local").expect("un identifiant valide"),
            "Claude Code",
            "claude",
        )
        .with_args(["--acp"]);

        assert_eq!(
            agent_reach(&agent),
            Reach::Unresolved,
            "un processus opaque n'a pas de portée connaissable"
        );
        assert!(
            !allows_external_agent(PrivacyTier::Local, &agent),
            "`Local` promet que rien ne sort : un agent dont on ne voit pas la sortie ne peut pas le tenir"
        );
        // Les deux autres niveaux l'acceptent, sans quoi le mode n'existerait
        // pour personne.
        assert!(allows_external_agent(PrivacyTier::Metadata, &agent));
        assert!(allows_external_agent(PrivacyTier::Sampled, &agent));
    }

    #[test]
    fn un_point_d_acces_non_resolu_est_traite_comme_distant() {
        // Le piège d'AI-PROVIDERS : un mandataire en écoute sur localhost. Le
        // classement vient de `Reach`, jamais de la forme de l'URL.
        assert!(allows_endpoint(PrivacyTier::Local, Reach::Local));
        assert!(!allows_endpoint(PrivacyTier::Local, Reach::Remote));
        assert!(
            !allows_endpoint(PrivacyTier::Local, Reach::Unresolved),
            "dans le doute, on protège"
        );
        assert!(allows_endpoint(PrivacyTier::Metadata, Reach::Unresolved));
    }

    #[test]
    fn le_niveau_rendu_ici_est_bien_celui_du_domaine() {
        // Une seule définition dans le dépôt : si quelqu'un en réintroduisait
        // une locale, cette égalité de types ne compilerait plus.
        let du_domaine: oxyn_core::PrivacyTier = oxyn_core::PrivacyTier::Metadata;
        let reexporte: PrivacyTier = du_domaine;
        assert_eq!(reexporte, PrivacyTier::default());
    }
}
