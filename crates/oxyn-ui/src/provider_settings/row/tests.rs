//! Ce qu'une ligne doit dire, et ce qu'elle ne doit jamais laisser croire.

use super::*;
use crate::provider_settings::DeclaredProvider;
use crate::provider_settings::ProviderReach;
use oxyn_core::AiProviderKind;

fn fournisseur() -> DeclaredProvider {
    DeclaredProvider {
        label: "Ollama du portable".into(),
        kind: AiProviderKind::OpenAiCompatible,
        base_url: "http://127.0.0.1:11434/v1".into(),
        model: "llama3".into(),
        key: KeyState::Absent,
        reach: ProviderReach::Local,
        measured_at: "2026-09-14 10:00".into(),
    }
}

#[test]
fn une_ligne_de_fournisseur_montre_son_point_dacces_et_son_modele() {
    let ligne = row_display(Declaration::Provider(&fournisseur()));
    assert_eq!(ligne.label, "Ollama du portable");
    assert_eq!(ligne.primary, "http://127.0.0.1:11434/v1");
    assert_eq!(ligne.secondary, "llama3");
    assert!(
        !ligne.reach_warns,
        "un point d'accès classé local ne s'annonce pas en avertissement"
    );
}

/// La mention de clé d'un agent ne doit pas se lire comme un réglage manquant.
///
/// « Absente » dirait qu'une clé pourrait être configurée et ne l'est pas.
/// C'est faux, et c'est le contraire du sujet : un agent externe porte sa propre
/// authentification, ce qui est **l'intérêt** du mode
/// ([ADR-0026](../../../../docs/adr/0026-agents-externes-acp.md)).
#[test]
fn un_agent_ne_dit_pas_quil_lui_manque_une_cle() {
    let ligne = row_display(Declaration::Agent {
        label: "Claude Code",
        command: "claude",
        args: 1,
    });

    assert_eq!(ligne.key_note, "no key — the agent carries its own");
    assert_ne!(
        ligne.key_note,
        KeyState::Absent.label(),
        "« absente » se lirait comme un réglage qui manque"
    );
    assert_ne!(ligne.key_note, configured_key_note());
}

/// La destination d'un agent est **inconnaissable**, et l'avertissement est
/// permanent.
///
/// Pour un fournisseur, la portée se mesure et peut être périmée ; pour un
/// agent, il n'y a rien à mesurer. La mention ne doit donc pas laisser espérer
/// qu'une prochaine mesure la lèvera.
#[test]
fn la_destination_dun_agent_est_annoncee_comme_inconnaissable() {
    let ligne = row_display(Declaration::Agent {
        label: "Gemini CLI",
        command: "gemini",
        args: 0,
    });

    assert!(ligne.reach_warns, "l'avertissement est permanent");
    assert!(
        ligne.reach_note.contains("unknowable"),
        "« inconnue » laisserait croire qu'une mesure viendra : {}",
        ligne.reach_note
    );
    assert!(
        !ligne.reach_note.contains("measured"),
        "rien n'a été mesuré, et rien ne le sera : {}",
        ligne.reach_note
    );
}

/// Le compte d'arguments s'accorde, et zéro se dit.
///
/// « 0 arguments » se lit comme une information manquante ; « no arguments » dit
/// que la commande se suffit à elle-même.
#[test]
fn le_compte_darguments_saccorde_et_zero_se_dit() {
    for (args, attendu) in [(0, "no arguments"), (1, "1 argument"), (3, "3 arguments")] {
        let ligne = row_display(Declaration::Agent {
            label: "Agent",
            command: "claude",
            args,
        });
        assert_eq!(ligne.secondary, attendu);
    }
}

/// Les deux sortes remplissent **tous** les champs de la ligne.
///
/// C'est ce qui permet à la vue de n'écrire qu'un `render_row` : un champ vide
/// pour une sorte obligerait à un `when` dans le rendu, et le parcours
/// recommencerait à diverger.
#[test]
fn les_deux_sortes_remplissent_la_meme_ligne() {
    let lignes = [
        row_display(Declaration::Provider(&fournisseur())),
        row_display(Declaration::Agent {
            label: "Claude Code",
            command: "claude",
            args: 1,
        }),
    ];
    for ligne in lignes {
        for (nom, valeur) in [
            ("label", &ligne.label),
            ("family", &ligne.family),
            ("primary", &ligne.primary),
            ("secondary", &ligne.secondary),
            ("key_note", &ligne.key_note),
            ("reach_note", &ligne.reach_note),
        ] {
            assert!(!valeur.trim().is_empty(), "{nom} est vide pour {ligne:?}");
        }
    }
}

/// Un argument contenant une espace reste **un** argument.
///
/// C'est le cas que le découpage sur l'espace aurait cassé, et il n'a rien
/// d'exotique : `--flag=valeur avec espace`, un chemin de fichier.
#[test]
fn une_ligne_est_un_argument_espaces_comprises() {
    let args = parse_args("--acp\n--flag=a b c\n  --trimmed  \n\n");
    assert_eq!(args, ["--acp", "--flag=a b c", "--trimmed"]);
}

#[test]
fn une_saisie_vide_ne_donne_aucun_argument() {
    for vide in ["", "   ", "\n\n  \n"] {
        assert!(parse_args(vide).is_empty(), "« {vide} »");
    }
}

/// La validation du formulaire est **celle du domaine**, pas une copie.
#[test]
fn le_formulaire_refuse_ce_que_le_domaine_refuse() {
    let bon = AgentDraft {
        label: "Claude Code".to_owned(),
        command: "claude".to_owned(),
        args: "--acp".to_owned(),
    };
    assert!(agent_draft_error(&bon).is_none());

    for (cas, draft) in [
        (
            "commande vide",
            AgentDraft {
                command: String::new(),
                ..bon.clone()
            },
        ),
        (
            "nom vide",
            AgentDraft {
                label: "   ".to_owned(),
                ..bon.clone()
            },
        ),
        (
            "caractère de contrôle dans la commande",
            AgentDraft {
                command: "claude\u{7}".to_owned(),
                ..bon.clone()
            },
        ),
    ] {
        assert!(
            agent_draft_error(&draft).is_some(),
            "{cas} doit être refusé pendant la saisie"
        );
    }
}
