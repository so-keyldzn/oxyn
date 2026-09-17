//! Ce que la porte d'une invite d'agent garantit.

use super::*;

/// Le niveau ferme la porte **avant** qu'une invite n'existe.
///
/// C'est ce qui distingue cette porte d'une vérification : sous `Local`, il n'y
/// a pas d'invite mal formée à refuser plus loin — il n'y a pas d'invite du
/// tout ([I-04](../../../../../CLAUDE.md#i-04)).
#[test]
fn sous_local_aucune_invite_ne_se_compose() {
    let erreur = AgentPrompt::from_user(PrivacyTier::Local, "quelles tables existent ?")
        .expect_err("une connexion locale ne parle pas à un agent externe");

    let message = erreur.to_string();
    assert!(message.contains("local-only"), "{message}");
    assert!(
        !message.contains("quelles tables"),
        "un refus ne recopie pas la question : {message}"
    );
}

#[test]
fn les_niveaux_qui_admettent_un_agent_composent_l_invite() {
    for tier in [PrivacyTier::Metadata, PrivacyTier::Sampled] {
        let invite = AgentPrompt::from_user(tier, "  quelles tables existent ?  ")
            .expect("ce niveau admet un agent externe");
        assert_eq!(
            invite.as_str(),
            "quelles tables existent ?",
            "la saisie est transmise telle quelle, espaces de bord retirés"
        );
    }
}

/// Une question vide ne lance pas de processus.
#[test]
fn une_question_vide_est_refusee() {
    for vide in ["", "   ", "\n\t "] {
        assert!(
            AgentPrompt::from_user(PrivacyTier::Metadata, vide).is_err(),
            "« {vide} » ne compose aucune invite"
        );
    }
}

/// **Le test qui tient la garantie du type.**
///
/// Il ne s'exécute pas : il décrit ce que le compilateur doit refuser. Si
/// quelqu'un ajoute un `From<String>`, un `new(&str)` ou rend le champ public,
/// `AgentPrompt` cesse d'être une porte et redevient une chaîne — et rien
/// d'autre dans le dépôt ne le signalerait.
///
/// ```compile_fail
/// use oxyn_ai::external::prompt::AgentPrompt;
/// // Aucun constructeur ne prend une chaîne seule : le niveau est obligatoire.
/// let _ = AgentPrompt::from("une invite fabriquée sans niveau".to_owned());
/// ```
///
/// ```compile_fail
/// use oxyn_ai::external::prompt::AgentPrompt;
/// // Le champ n'est pas public : on ne contourne pas la porte en le posant.
/// let _ = AgentPrompt { text: "sans passer par la porte".to_owned() };
/// ```
const _: () = ();
