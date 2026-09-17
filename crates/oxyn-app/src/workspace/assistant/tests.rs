//! Ce que le panneau garantit, et qui ne casse rien en se défaisant.
//!
//! Chacun de ces tests ferme une régression **silencieuse** : rien ne cesse de
//! compiler, aucun autre test ne rougit, et l'interface se met simplement à
//! mentir. C'est le critère de `.claude/rules/tests.md`.

use super::transcript::{Ending, PanelState, command_target, panel_state};
use super::*;
use crate::workspace::tests::{connected_workspace, submit, wait_until};
use gpui::TestAppContext;
use oxyn_core::{AiProviderConfig, AiProviderKind, Command, ConnectionId, ErrorClass, ProviderId};

/// Une déclaration de fournisseur, sans clé et sans réseau.
fn declaration(label: &str, base_url: &str) -> AiProviderConfig {
    AiProviderConfig::new(
        ProviderId::new("essai").expect("identifiant de fournisseur valide"),
        AiProviderKind::OpenAiCompatible,
        label,
        base_url,
        "un-modele",
    )
}

fn classe(base_url: &str, local: bool) -> ClassifiedProvider {
    ClassifiedProvider {
        config: declaration("déclaration", base_url),
        reach: if local {
            oxyn_llm::Reach::Local
        } else {
            oxyn_llm::Reach::Remote
        },
        measured_at: chrono::Utc::now(),
    }
}

/// Une déclaration d'agent externe, sans secret et sans processus.
fn agent_declare() -> oxyn_core::ExternalAgentConfig {
    oxyn_core::ExternalAgentConfig::new(
        ProviderId::new("claude-code").expect("identifiant d'agent valide"),
        "Claude Code",
        "claude",
    )
    .with_args(["--acp"])
}

#[test]
fn sans_fournisseur_declare_il_n_y_a_pas_d_entree() {
    // La garantie d'ADR-0006 : zéro fournisseur, pas de bouton, pas de badge,
    // aucune mention. Un `Disabled` ici serait la publicité qu'UX-SPEC refuse,
    // et il passerait toutes les autres vérifications du dépôt.
    assert_eq!(
        ask_ai_entry(&[], &[], false, PrivacyTier::Metadata, Capabilities::SQL),
        AskAi::Absent
    );
    assert_eq!(
        ask_ai_entry(&[], &[], false, PrivacyTier::Local, Capabilities::SQL),
        AskAi::Absent
    );
}

#[test]
fn le_niveau_local_desactive_sans_masquer_et_dit_pourquoi() {
    let distants = [classe("https://api.example.com", false)];
    let AskAi::Disabled(raison) =
        ask_ai_entry(&distants, &[], false, PrivacyTier::Local, Capabilities::SQL)
    else {
        panic!("l'entrée reste visible : la faire disparaître se lirait comme un défaut");
    };
    assert_eq!(raison, NO_USABLE_DECLARATION);

    // Un fournisseur local dans la liste suffit à rendre le niveau utilisable.
    let mixte = [
        classe("https://api.example.com", false),
        classe("http://127.0.0.1:11434", true),
    ];
    assert_eq!(
        ask_ai_entry(&mixte, &[], false, PrivacyTier::Local, Capabilities::SQL),
        AskAi::Enabled
    );
    // Et le même fournisseur distant est parfaitement utilisable ailleurs.
    assert_eq!(
        ask_ai_entry(
            &distants,
            &[],
            false,
            PrivacyTier::Metadata,
            Capabilities::SQL
        ),
        AskAi::Enabled
    );
}

/// Un agent externe déclaré **seul** ouvre l'entrée.
///
/// Le défaut que ce test ferme rendait tout le mode inatteignable : l'entrée
/// ne regardait que les fournisseurs, donc un utilisateur qui n'avait déclaré
/// qu'un agent voyait `Absent` — « Oxyn marche sans IA » — alors qu'il venait
/// précisément de déclarer de quoi s'en servir
/// ([ADR-0026](../../../../docs/adr/0026-agents-externes-acp.md)).
#[test]
fn un_agent_declare_seul_suffit_a_ouvrir_l_entree() {
    let agents = [agent_declare()];
    assert_eq!(
        ask_ai_entry(
            &[],
            &agents,
            false,
            PrivacyTier::Metadata,
            Capabilities::SQL
        ),
        AskAi::Enabled
    );
    // Mais pas sous `Local` : rien ne dit où cet agent envoie les données, et
    // c'est un refus, pas une réserve.
    let AskAi::Disabled(raison) =
        ask_ai_entry(&[], &agents, false, PrivacyTier::Local, Capabilities::SQL)
    else {
        panic!("sous Local, l'entrée reste visible et inerte");
    };
    assert!(
        raison.contains("external agent"),
        "la raison doit nommer les deux sortes, sinon elle envoie déclarer \
         un agent que ce niveau refuse tout autant : {raison}"
    );
}

/// Le repli vers un agent est **atteignable**, et c'est tout l'enjeu.
///
/// La garde de `ask_assistant` et `provider_for` décidaient séparément, avec
/// des conditions qui se recouvraient : passé la garde, `provider_for` rendait
/// toujours `Some`, et la branche agent était morte. Ce test décrit l'état
/// exact que le défaut rendait impossible — entrée ouverte, aucun fournisseur
/// utilisable, un agent qui l'est.
#[test]
fn sans_fournisseur_utilisable_un_agent_prend_le_relais() {
    let agents = [agent_declare()];
    assert_eq!(
        ask_ai_entry(
            &[],
            &agents,
            false,
            PrivacyTier::Metadata,
            Capabilities::SQL
        ),
        AskAi::Enabled,
        "l'entrée est ouverte"
    );
    assert!(
        provider_for(&[], PrivacyTier::Metadata).is_none(),
        "et pourtant aucun fournisseur n'est utilisable"
    );
    assert!(
        usable_agent(&agents, PrivacyTier::Metadata).is_some(),
        "c'est donc l'agent qui répond, et ce chemin doit exister"
    );
}

/// Aucun agent n'est choisi sous un niveau qui les ferme tous.
///
/// Prendre le premier de la liste puis se faire refuser par `run_turn`
/// afficherait un échec là où une autre déclaration aurait convenu.
#[test]
fn sous_local_aucun_agent_n_est_choisi() {
    let agents = [agent_declare()];
    assert!(usable_agent(&agents, PrivacyTier::Local).is_none());
    assert!(usable_agent(&agents, PrivacyTier::Metadata).is_some());
    assert!(usable_agent(&[], PrivacyTier::Metadata).is_none());
}

#[test]
fn sous_local_aucune_declaration_distante_n_est_choisie() {
    // Le runtime revérifie et refuserait, mais proposer puis refuser vaut moins
    // bien que ne pas proposer : ce test garde le choix honnête ici aussi.
    let providers = [
        classe("https://api.example.com", false),
        classe("http://127.0.0.1:11434", true),
    ];
    let choisi = provider_for(&providers, PrivacyTier::Local).expect("un fournisseur local existe");
    assert!(choisi.is_local());
    assert!(
        provider_for(
            &[classe("https://api.example.com", false)],
            PrivacyTier::Local
        )
        .is_none()
    );
}

#[test]
fn une_commande_est_montree_avant_son_resultat() {
    // La règle d'UX-SPEC : « un agent qui travaille en silence pendant huit
    // tours est indistinguable d'un agent bloqué ». Inverser les deux poussées
    // ne casserait aucune compilation.
    let connexion = ConnectionId::new();
    let mut entrees = Vec::new();
    apply_event(
        &mut entrees,
        AiEvent::CommandSubmitted {
            tool: "execute_query".to_owned(),
            command: "Execute",
            connection: Some(connexion),
            mutating: false,
        },
        connexion,
        "commerce-prod",
    );
    apply_event(
        &mut entrees,
        AiEvent::CommandReported {
            tool: "execute_query".to_owned(),
            outcome: DispatchOutcome::Completed {
                summary: "12 rows, 1 batches".to_owned(),
            },
            withheld: false,
        },
        connexion,
        "commerce-prod",
    );

    // La forme entière, dans l'ordre : une position relative laisserait passer
    // une commande qu'on n'annonce qu'au retour de son rapport, ce qui est
    // précisément la « simplification » qu'UX-SPEC refuse.
    assert!(
        matches!(
            entrees.as_slice(),
            [
                Entry::Command {
                    target,
                    mutating: false,
                    ..
                },
                Entry::Report { .. }
            ] if target == "commerce-prod"
        ),
        "la commande se lit avant son résultat, et nomme sa connexion : {entrees:?}"
    );
}

#[test]
fn aucun_identifiant_de_connexion_n_atteint_la_conversation() {
    // I-03 : ni dans la ligne visible, ni dans un `Debug` de la conversation.
    let courante = ConnectionId::new();
    let ailleurs = ConnectionId::new();
    assert_eq!(
        command_target(Some(courante), courante, "commerce-prod"),
        "commerce-prod"
    );
    let autre = command_target(Some(ailleurs), courante, "commerce-prod");
    assert!(!autre.contains(&ailleurs.to_string()), "{autre}");
    assert!(!autre.contains(&courante.to_string()), "{autre}");
    assert_eq!(
        command_target(None, courante, "commerce-prod"),
        "no connection"
    );
}

#[test]
fn l_ecart_entre_ce_qui_est_lu_et_ce_qui_est_envoye_survit_jusqu_au_panneau() {
    // Cacher l'écart ferait passer une réponse mal informée pour une réponse
    // fausse (UX-SPEC). Le message du serveur, lui, arrive entier.
    let connexion = ConnectionId::new();
    let mut entrees = Vec::new();
    apply_event(
        &mut entrees,
        AiEvent::CommandReported {
            tool: "execute_query".to_owned(),
            outcome: DispatchOutcome::Failed {
                class: ErrorClass::Permanent,
                message: "duplicate key value violates unique constraint \"clients_email_key\""
                    .to_owned(),
            },
            withheld: true,
        },
        connexion,
        "commerce-prod",
    );
    let Some(Entry::Report {
        outcome, withheld, ..
    }) = entrees.first()
    else {
        panic!("le rapport doit être dans la conversation");
    };
    assert!(
        withheld,
        "le panneau doit pouvoir dire que le modèle a eu moins"
    );
    let DispatchOutcome::Failed { message, .. } = outcome else {
        panic!("un échec reste un échec");
    };
    assert!(
        message.contains("clients_email_key"),
        "l'utilisateur lit le message entier de son serveur : {message}"
    );
}

#[test]
fn les_fragments_se_recollent_et_un_appel_d_outil_ouvre_un_paragraphe() {
    let connexion = ConnectionId::new();
    let mut entrees = Vec::new();
    for fragment in ["SELECT ", "count(*)"] {
        apply_event(
            &mut entrees,
            AiEvent::TextDelta(fragment.to_owned()),
            connexion,
            "base",
        );
    }
    apply_event(
        &mut entrees,
        AiEvent::CallRejected {
            tool: "drop_everything".to_owned(),
            error: "unknown tool".to_owned(),
        },
        connexion,
        "base",
    );
    apply_event(
        &mut entrees,
        AiEvent::TextDelta(" FROM clients".to_owned()),
        connexion,
        "base",
    );
    let reponses: Vec<&str> = entrees
        .iter()
        .filter_map(|entree| match entree {
            Entry::Answer(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(reponses, vec!["SELECT count(*)", " FROM clients"]);
}

#[test]
fn une_reponse_coupee_et_un_plafond_de_tours_ne_se_confondent_pas() {
    let connexion = ConnectionId::new();
    for (issue, attendue) in [
        (
            AgentOutcome::Answered {
                text: String::new(),
                turns: 2,
                truncated: true,
                stop: oxyn_core::ai::StopReason::MaxTokens,
            },
            Ending::Answered {
                turns: 2,
                truncated: true,
            },
        ),
        (
            AgentOutcome::TurnLimit { turns: 8 },
            Ending::TurnLimit { turns: 8 },
        ),
        (
            AgentOutcome::Cancelled { turns: 1 },
            Ending::Cancelled { turns: 1 },
        ),
    ] {
        let mut entrees = Vec::new();
        apply_event(&mut entrees, AiEvent::Finished(issue), connexion, "base");
        assert_eq!(entrees, vec![Entry::Ended(attendue)]);
    }
}

#[test]
fn le_vide_ne_se_lit_pas_comme_une_erreur() {
    // L'état qu'on oublie. Une réponse vide et une panne du fournisseur ne
    // demandent pas la même chose à l'utilisateur.
    assert_eq!(panel_state(&[], false), PanelState::Initial);
    assert_eq!(
        panel_state(&[Entry::Question("bonjour".to_owned())], true),
        PanelState::Running,
        "une conversation en cours prime : c'est l'état qui porte l'annulation"
    );
    assert_eq!(
        panel_state(
            &[
                Entry::Question("bonjour".to_owned()),
                Entry::Ended(Ending::Answered {
                    turns: 1,
                    truncated: false
                })
            ],
            false
        ),
        PanelState::Empty
    );
    assert_eq!(
        panel_state(
            &[Entry::Failed("le fournisseur n'a pas répondu".to_owned())],
            false
        ),
        PanelState::Failed
    );
}

#[test]
fn une_proposition_se_lit_dans_un_bloc_et_nulle_part_ailleurs() {
    // Le piège fermé ici : une phrase qui mentionne `DELETE` prise pour une
    // proposition, et offerte à l'ouverture dans une console.
    let reponse = "Voici la requête :\n\n```sql\nSELECT count(*)\nFROM clients;\n```\n\n\
                   Elle ne fait pas de DELETE FROM clients.";
    assert_eq!(
        sql_proposals(reponse),
        vec!["SELECT count(*)\nFROM clients;".to_owned()]
    );
    assert!(sql_proposals("Aucun bloc ici, juste SELECT 1.").is_empty());
    // Un bloc annoncé dans un autre langage n'est pas une requête.
    assert!(sql_proposals("```python\nprint(1)\n```").is_empty());
    // Un bloc vide n'est pas une proposition.
    assert!(sql_proposals("```sql\n\n```").is_empty());
}

#[gpui::test]
fn l_entree_ask_ai_n_existe_pas_tant_qu_aucun_fournisseur_n_est_declare(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend.clone(), open, cx));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("ask-ai").is_none(),
        "sans déclaration, l'entrée n'est pas dessinée du tout"
    );
    assert!(
        cx.debug_bounds("privacy-tier").is_none(),
        "et aucun niveau ne s'affiche non plus"
    );

    // Une déclaration enregistrée fait apparaître les deux, sans redémarrage.
    submit(
        &backend,
        Command::SaveAiProvider {
            config: Box::new(declaration("local", "http://127.0.0.1:11434")),
        },
    )
    .expect("la déclaration doit être enregistrée");
    workspace.update(cx, |view, cx| {
        view.reload_ai_providers(cx);
        cx.notify();
    });
    wait_until(
        &workspace,
        cx,
        "la déclaration enregistrée doit revenir jusqu'à la vue",
        |view, _| {
            view.assistant
                .providers
                .as_deref()
                .is_some_and(|liste| !liste.is_empty())
        },
    );
    assert!(
        cx.debug_bounds("ask-ai").is_some(),
        "la première déclaration fait apparaître l'entrée (ADR-0023)"
    );
    assert!(cx.debug_bounds("privacy-tier").is_some());
}

#[gpui::test]
fn une_lecture_perimee_ne_fait_pas_disparaitre_une_declaration(cx: &mut TestAppContext) {
    // Deux lectures se chevauchent : celle que le workspace lance à sa
    // construction, et celle que l'écran de réglages demande après un
    // enregistrement. Elles répondent sur un runtime qui ne promet aucun ordre.
    // Sans jeton, la plus ancienne peut atterrir en dernier et remettre la liste
    // comme avant — c'est-à-dire un fournisseur enregistré et une entrée qui
    // n'apparaît jamais. Rien ne casse, rien ne rougit : l'entrée manque.
    let (backend, open) = connected_workspace();
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend.clone(), open, cx));
    submit(
        &backend,
        Command::SaveAiProvider {
            config: Box::new(declaration("local", "http://127.0.0.1:11434")),
        },
    )
    .expect("la déclaration doit être enregistrée");
    workspace.update(cx, |view, cx| view.reload_ai_providers(cx));
    wait_until(
        &workspace,
        cx,
        "la lecture la plus récente doit atteindre la vue",
        |view, _| {
            view.assistant
                .providers
                .as_deref()
                .is_some_and(|liste| !liste.is_empty())
        },
    );

    // La réponse d'une lecture précédente arrive après coup, vide.
    workspace.update(cx, |view, cx| {
        let perimee = view.assistant.read.saturating_sub(1);
        view.receive_ai_providers(perimee, Ok(Ok(Vec::new())), cx);
    });
    cx.run_until_parked();
    workspace.read_with(cx, |view, _| {
        assert!(
            view.assistant
                .providers
                .as_deref()
                .is_some_and(|liste| !liste.is_empty()),
            "une réponse périmée ne dit rien de ce qui est déclaré maintenant"
        );
    });
    assert!(cx.debug_bounds("ask-ai").is_some());
}

#[gpui::test]
fn le_panneau_montre_la_commande_avant_son_resultat_et_offre_l_annulation(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let connexion = open.connection;
    // Une déclaration, sinon le panneau se referme de lui-même : sans
    // fournisseur, il n'existe pas.
    submit(
        &backend,
        Command::SaveAiProvider {
            config: Box::new(declaration("local", "http://127.0.0.1:11434")),
        },
    )
    .expect("la déclaration doit être enregistrée");
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    wait_until(
        &workspace,
        cx,
        "la déclaration doit atteindre la vue",
        |view, _| view.assistant.providers.is_some(),
    );

    // Une conversation en cours, sans fournisseur ni réseau : ce qui est
    // vérifié ici, c'est l'affichage, pas le transport.
    let jeton = CancelToken::new();
    workspace.update(cx, |view, cx| {
        view.panel = WorkspacePanel::Assistant;
        view.assistant.active = Some(jeton.clone());
        view.on_ai_event(
            AiEvent::CommandSubmitted {
                tool: "execute_query".to_owned(),
                command: "Execute",
                connection: Some(connexion),
                mutating: true,
            },
            cx,
        );
        cx.notify();
    });
    cx.run_until_parked();

    // Le rapport arrive après, et le panneau le dessine après : c'est la même
    // garantie qu'au niveau de la conversation, vérifiée là où l'utilisateur la
    // lit. Les deux ordonnées sont décidées par Oxyn — une colonne flex — et non
    // par la métrique de texte du harnais.
    workspace.update(cx, |view, cx| {
        view.on_ai_event(
            AiEvent::CommandReported {
                tool: "execute_query".to_owned(),
                outcome: DispatchOutcome::Completed {
                    summary: "3 rows, 1 batches".to_owned(),
                },
                withheld: false,
            },
            cx,
        );
        cx.notify();
    });
    cx.run_until_parked();
    let soumise = cx
        .debug_bounds("assistant-entry-0")
        .expect("la commande soumise est dessinée");
    let rapport = cx
        .debug_bounds("assistant-entry-1")
        .expect("le rapport est dessiné");
    assert!(
        soumise.origin.y < rapport.origin.y,
        "la commande se lit au-dessus de son résultat"
    );

    let annuler = cx
        .debug_bounds("assistant-cancel")
        .expect("l'annulation reste offerte pendant toute la conversation");

    // Le clic atteint bien la cible : des bornes rendues ne le prouvent pas.
    cx.simulate_click(annuler.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(
        jeton.is_cancelled(),
        "le bouton doit atteindre le jeton, pas seulement changer d'apparence"
    );
    workspace.read_with(cx, |view, _| {
        assert!(
            view.assistant.cancelling,
            "la demande est prise en compte tout de suite ; son effet est celui du fournisseur"
        );
        assert!(
            view.assistant.active.is_some(),
            "la conversation ne disparaît pas de l'écran au clic"
        );
    });
}

#[gpui::test]
fn echap_dans_le_champ_atteint_le_jeton_de_la_conversation(cx: &mut TestAppContext) {
    // `TextField` consomme `escape` : sans l'abonnement à `FieldEvent::Escape`,
    // le seul moyen d'annuler disparaîtrait pour qui a le curseur dans le champ
    // — c'est-à-dire exactement là où il est pendant une conversation.
    let (backend, open) = connected_workspace();
    submit(
        &backend,
        Command::SaveAiProvider {
            config: Box::new(declaration("local", "http://127.0.0.1:11434")),
        },
    )
    .expect("la déclaration doit être enregistrée");
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    wait_until(
        &workspace,
        cx,
        "la déclaration doit atteindre la vue",
        |view, _| view.assistant.providers.is_some(),
    );
    let jeton = CancelToken::new();
    workspace.update(cx, |view, cx| {
        view.panel = WorkspacePanel::Assistant;
        view.assistant.active = Some(jeton.clone());
        cx.notify();
    });
    cx.run_until_parked();
    let champ = workspace.read_with(cx, |view, cx| {
        view.assistant.question.clone().read(cx).focus_handle(cx)
    });
    cx.update(|window, _| window.focus(&champ));
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(
        jeton.is_cancelled(),
        "l'assertion porte sur le jeton, pas sur l'affichage"
    );
}

#[gpui::test]
fn une_proposition_arrive_dans_une_console_sans_s_executer(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.run_until_parked();
    let consoles = workspace.read_with(cx, |view, _| view.consoles.len());

    workspace.update(cx, |view, cx| {
        // Ce que le parcours réel a déjà posé quand ce geste devient
        // atteignable : sans marque, `open_proposal` refuse d'écrire, et c'est
        // `une_proposition_sans_marque_nouvre_rien` qui couvre ce cas-là.
        view.on_ai_event(
            AiEvent::Started(Box::new(oxyn_core::Provenance::new(
                oxyn_core::AgentId::new(),
                oxyn_core::AgentSessionId::new(),
                AiProviderKind::Anthropic,
                "claude-sonnet-5",
            ))),
            cx,
        );
        view.open_proposal("SELECT count(*) FROM clients".to_owned(), cx);
    });
    // La console ouvre sa propre session sur le runtime Tokio, que l'horloge
    // virtuelle de GPUI ne fait pas avancer.
    wait_until(
        &workspace,
        cx,
        "la proposition doit atteindre une console",
        |view, _| view.consoles.len() > consoles,
    );

    workspace.read_with(cx, |view, cx| {
        assert_eq!(
            view.consoles.len(),
            consoles + 1,
            "la proposition ouvre sa propre console plutôt que d'écraser un brouillon"
        );
        let arrivee = view
            .consoles
            .last()
            .expect("la console vient d'être ajoutée")
            .read(cx);
        assert_eq!(
            arrivee.editor.read(cx).text(),
            "SELECT count(*) FROM clients"
        );
        assert!(
            arrivee.active.is_none(),
            "rien ne part sur le bus du seul fait qu'un texte est arrivé (I-07)"
        );
        assert!(
            view.assistant.active.is_none(),
            "et aucune conversation n'a été ouverte par ce geste"
        );
    });
}

/// Les boutons du panneau s'actionnent au clavier, pas seulement à la souris.
///
/// Le piège que ce test ferme est documentaire autant que technique :
/// `oxyn_ui::control` **n'active rien** au clavier — il pose `tab_stop`, donc
/// l'atteignabilité, et rien de plus. Un commentaire de `controls.rs` a affirmé
/// le contraire pendant un temps, et s'y fier laisse des boutons qu'on peut
/// atteindre au Tab sans pouvoir les actionner : la définition exacte d'un
/// contrôle inutilisable au clavier, point bloquant de
/// `.claude/checklists/revue-ui.md`.
///
/// Le test porte sur `Stop` parce que son effet est observable **hors de
/// l'affichage** — le jeton d'annulation. Une assertion sur des bornes rendues
/// ne prouverait rien ici.
///
/// Le clic initial sert à poser le focus sur le bouton, comme une tabulation
/// l'y mènerait ; c'est le **second** jeton, annulé sans qu'aucun clic ne le
/// vise, qui porte la garantie.
#[gpui::test]
fn les_boutons_du_panneau_s_actionnent_au_clavier(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    submit(
        &backend,
        Command::SaveAiProvider {
            config: Box::new(declaration("local", "http://127.0.0.1:11434")),
        },
    )
    .expect("la déclaration doit être enregistrée");
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    wait_until(
        &workspace,
        cx,
        "la déclaration doit atteindre la vue",
        |view, _| view.assistant.providers.is_some(),
    );

    let a_la_souris = CancelToken::new();
    workspace.update(cx, |view, cx| {
        view.panel = WorkspacePanel::Assistant;
        view.assistant.active = Some(a_la_souris.clone());
        cx.notify();
    });
    cx.run_until_parked();

    let annuler = cx
        .debug_bounds("assistant-cancel")
        .expect("l'annulation est offerte pendant la conversation");
    cx.simulate_click(annuler.center(), gpui::Modifiers::none());
    cx.run_until_parked();

    // Le bouton tient maintenant le focus. On repart d'une conversation neuve
    // pour que le jeton observé ne puisse pas avoir été annulé par le clic.
    let au_clavier = CancelToken::new();
    workspace.update(cx, |view, cx| {
        view.assistant.cancelling = false;
        view.assistant.active = Some(au_clavier.clone());
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        !au_clavier.is_cancelled(),
        "le second jeton part intact, sinon le test ne mesure rien"
    );

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    assert!(
        au_clavier.is_cancelled(),
        "un bouton atteignable au clavier doit s'actionner à la touche Entrée"
    );
}

/// Le parcours entier : déclarer un fournisseur fait apparaître l'entrée.
///
/// C'est la garantie que tout le lot existe pour tenir, et elle traverse les
/// quatre couches : l'écran émet, le workspace traduit en `Command`, le store
/// écrit, la lecture reclassée revient, l'entrée apparaît **sans
/// redémarrage** (ADR-0023). Aucun maillon ne peut être vérifié seul : chacun
/// pris isolément passerait avec un câblage rompu.
///
/// Le retrait est vérifié dans la foulée, parce que c'est le sens qui casse en
/// silence : une liste qui ne se vide pas laisse une entrée qui ne peut plus
/// rien envoyer.
#[gpui::test]
fn declarer_puis_retirer_un_fournisseur_fait_apparaitre_et_disparaitre_l_entree(
    cx: &mut TestAppContext,
) {
    let (backend, open) = connected_workspace();
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend.clone(), open, cx));
    wait_until(
        &workspace,
        cx,
        "la première lecture doit répondre, même vide",
        |view, _| view.assistant.providers.is_some(),
    );

    // Rien n'est déclaré : pas d'entrée, et pas de bouton dessiné.
    workspace.read_with(cx, |view, _| {
        assert_eq!(
            view.assistant
                .entry(view.display.privacy_tier, view.capabilities),
            AskAi::Absent,
            "sans fournisseur, l'entrée n'existe pas"
        );
    });
    assert!(
        cx.debug_bounds("ask-ai").is_none(),
        "et rien n'est dessiné : ni bouton, ni appel à l'action"
    );

    // L'écran de configuration décide, exactement comme un utilisateur qui
    // remplit le formulaire et valide.
    workspace.update(cx, |view, cx| {
        view.provider_settings.update(cx, |_, cx| {
            cx.emit(
                oxyn_ui::provider_settings::ProviderSettingsEvent::SaveRequested(Box::new(
                    oxyn_ui::provider_settings::ProviderDraft {
                        kind: AiProviderKind::OpenAiCompatible,
                        label: "Ollama".to_owned(),
                        base_url: "http://127.0.0.1:11434".to_owned(),
                        model: "llama3.2".to_owned(),
                        // Pas de clé : un point d'accès local n'en demande pas, et
                        // le test ne doit toucher aucun trousseau.
                        key: None,
                    },
                )),
            );
        });
    });

    wait_until(
        &workspace,
        cx,
        "la déclaration doit traverser le bus et revenir classée",
        |view, _| {
            view.assistant
                .providers
                .as_ref()
                .is_some_and(|providers| !providers.is_empty())
        },
    );
    workspace.read_with(cx, |view, _| {
        assert_ne!(
            view.assistant
                .entry(view.display.privacy_tier, view.capabilities),
            AskAi::Absent,
            "un fournisseur déclaré fait apparaître l'entrée, sans redémarrage"
        );
    });

    // Puis le retrait, par le même chemin.
    workspace.update(cx, |view, cx| {
        view.provider_settings.update(cx, |_, cx| {
            cx.emit(oxyn_ui::provider_settings::ProviderSettingsEvent::RemovalConfirmed(0));
        });
    });
    wait_until(
        &workspace,
        cx,
        "le retrait doit vider la liste",
        |view, _| view.assistant.providers.as_ref().is_some_and(Vec::is_empty),
    );
    workspace.read_with(cx, |view, _| {
        assert_eq!(
            view.assistant
                .entry(view.display.privacy_tier, view.capabilities),
            AskAi::Absent,
            "la dernière déclaration retirée fait disparaître l'entrée"
        );
    });
}

/// La marque naît sur le chemin agent → console, et nulle part ailleurs.
///
/// C'est le maillon que les deux relectures ont trouvé manquant : tout existait
/// — la colonne, le `coalesce`, `Provenance` — mais aucun chemin applicatif ne
/// posait jamais autre chose que `None`. Le dépôt affirmait alors formellement
/// une contre-vérité sur exactement la ligne qu'ADR-0023 existe pour marquer :
/// « provenance absente » veut dire « écrit par l'utilisateur ».
#[gpui::test]
fn une_proposition_arrive_dans_une_console_avec_sa_marque(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.run_until_parked();

    let origine = oxyn_core::Provenance::new(
        oxyn_core::AgentId::new(),
        oxyn_core::AgentSessionId::new(),
        AiProviderKind::Anthropic,
        "claude-sonnet-5",
    );
    let consoles = workspace.read_with(cx, |view, _| view.consoles.len());

    workspace.update(cx, |view, cx| {
        view.panel = WorkspacePanel::Assistant;
        // Ce que `AiEvent::Started` pose au montage de la conversation.
        view.on_ai_event(AiEvent::Started(Box::new(origine.clone())), cx);
        view.open_proposal("SELECT count(*) FROM clients".to_owned(), cx);
    });
    wait_until(
        &workspace,
        cx,
        "la proposition doit atteindre une console",
        |view, _| view.consoles.len() > consoles,
    );

    workspace.read_with(cx, |view, cx| {
        let arrivee = view
            .consoles
            .last()
            .expect("la console vient d'être ajoutée")
            .read(cx);
        assert_eq!(
            arrivee.provenance.as_ref(),
            Some(&origine),
            "la console retient d'où vient le texte, pour l'écrire avec lui"
        );
        assert!(
            arrivee.active.is_none(),
            "et rien ne part sur le bus du seul fait qu'un texte est arrivé (I-07)"
        );
    });
}

/// Sans marque, la proposition n'ouvre rien — et le dit.
///
/// Le cas se produit pour un agent externe
/// ([ADR-0026](../../../../docs/adr/0026-agents-externes-acp.md)) : `Provenance`
/// exige un `AiProviderKind` et un modèle, qu'un agent n'a pas. Le défaut que ce
/// test ferme est de ceux qui ne rougissent nulle part : écrire avec `None`
/// aurait donné une console valide, un document valide, et une ligne
/// d'historique affirmant que **l'utilisateur** a écrit ce texte.
///
/// La notice compte autant que le refus : un geste qui ne fait rien en silence
/// se lit comme un bouton cassé, et l'utilisateur recommence.
#[gpui::test]
fn une_proposition_sans_marque_nouvre_rien(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.run_until_parked();
    let consoles = workspace.read_with(cx, |view, _| view.consoles.len());

    workspace.update(cx, |view, cx| {
        assert!(
            view.assistant.provenance.is_none(),
            "le cas testé est bien celui d'un tour qui n'a posé aucune marque"
        );
        view.open_proposal("SELECT count(*) FROM clients".to_owned(), cx);
    });
    cx.run_until_parked();

    workspace.read_with(cx, |view, _| {
        assert_eq!(
            view.consoles.len(),
            consoles,
            "aucune console ne s'ouvre : un texte non marqué passerait pour écrit par l'utilisateur"
        );
        let derniere = view.assistant.entries.last();
        assert!(
            matches!(derniere, Some(Entry::Notice(_))),
            "le refus se dit dans le fil, sinon le bouton paraît cassé : {derniere:?}"
        );
    });
}

/// Une copie d'historique n'hérite d'aucune marque.
///
/// Le sens qui casse en silence : marquer par excès ferait passer pour écrit
/// par un agent un texte que l'utilisateur avait écrit lui-même. Une copie en
/// reçoit une ou pas ; elle n'en hérite jamais.
#[gpui::test]
fn une_copie_ordinaire_reste_sans_marque(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.run_until_parked();

    let consoles = workspace.read_with(cx, |view, _| view.consoles.len());
    workspace.update(cx, |view, cx| {
        view.open_library_query(
            crate::workspace::library::OpenQuery::Copy {
                text: "SELECT 1".to_owned(),
                title: "History copy.sql".to_owned(),
                origin: "History".to_owned(),
                provenance: None,
            },
            cx,
        );
    });
    wait_until(
        &workspace,
        cx,
        "la copie doit atteindre une console",
        |view, _| view.consoles.len() > consoles,
    );

    workspace.read_with(cx, |view, cx| {
        assert!(
            view.consoles
                .last()
                .expect("la console vient d'être ajoutée")
                .read(cx)
                .provenance
                .is_none(),
            "aucune marque ne s'invente : ce texte n'a pas été écrit par un agent"
        );
    });
}

/// Ne pas savoir n'est pas savoir qu'il n'y a rien.
///
/// Le défaut que ce test ferme a été trouvé par la relecture des divergences :
/// sur un échec de lecture du store, la liste devenait vide, `Ask AI`
/// disparaissait — et le commentaire du code affirmait que « le panneau dit
/// pourquoi », alors que l'entrée manquante rendait ce panneau inatteignable.
///
/// Une erreur SQLite locale devenait donc indiscernable d'une absence de
/// configuration, et l'utilisateur repartait déclarer un fournisseur déjà
/// enregistré.
#[test]
fn une_lecture_qui_echoue_ne_se_lit_pas_comme_une_absence_de_fournisseur() {
    // Aucun fournisseur déclaré : l'entrée n'existe pas, et c'est exact.
    assert_eq!(
        ask_ai_entry(&[], &[], false, PrivacyTier::Metadata, Capabilities::SQL),
        AskAi::Absent
    );

    // Lecture en échec : la même liste vide, mais l'entrée reste — inerte, avec
    // sa raison. C'est la distinction que le code perdait.
    let AskAi::Disabled(raison) =
        ask_ai_entry(&[], &[], true, PrivacyTier::Metadata, Capabilities::SQL)
    else {
        panic!("une lecture en échec laisse l'entrée visible et inerte");
    };
    assert!(
        raison.contains("could not be read"),
        "la raison doit nommer la lecture, pas l'absence : {raison}"
    );
}
