//! Ce qu'un agent externe ne doit jamais obtenir d'Oxyn.

use super::*;
use crate::privacy::PrivacyTier;
use oxyn_core::{ExternalAgentConfig, ProviderId};

fn agent(commande: &str, args: &[&str]) -> ExternalAgentConfig {
    ExternalAgentConfig::new(
        ProviderId::new("agent").expect("un identifiant valide"),
        "Agent",
        commande,
    )
    .with_args(args.iter().copied())
}

/// La commande et ses arguments ne sont jamais réassemblés en une chaîne.
///
/// C'est ce qui met le lancement hors d'atteinte d'un découpage façon shell : un
/// nom de programme contenant une espace, un guillemet ou un point-virgule
/// arrive au système d'exploitation tel quel, et n'y prend aucun sens
/// supplémentaire. L'exemple du protocole, lui, part d'une chaîne unique qu'il
/// découpe — c'est cette grammaire-là qu'on n'emprunte pas.
#[test]
fn la_commande_et_ses_arguments_ne_sont_jamais_recolles() {
    let hostile = agent(
        "/opt/mes agents/claude code",
        &["--acp", "; rm -rf /", "--flag=a b c"],
    );
    check_launchable(&hostile).expect("la déclaration est valide");

    // La propriété tient parce que la déclaration porte deux champs distincts
    // jusqu'à l'appel système : il n'existe aucun point du trajet où une chaîne
    // unique serait redécoupée.
    assert_eq!(
        hostile.command, "/opt/mes agents/claude code",
        "l'espace du chemin ne coupe pas la commande"
    );
    assert_eq!(
        hostile.args,
        ["--acp", "; rm -rf /", "--flag=a b c"],
        "chaque argument reste un argument, point-virgule compris"
    );
}

/// Une déclaration invalide ne produit pas de lancement.
///
/// La validation est refaite ici, et pas seulement à la saisie : une déclaration
/// peut venir d'un fichier d'état écrit par un tiers ou par une version future.
#[test]
fn une_declaration_invalide_ne_lance_rien() {
    assert!(check_launchable(&agent("", &[])).is_err(), "commande vide");
    assert!(
        check_launchable(&agent("claude\n--evil", &[])).is_err(),
        "un saut de ligne dans la commande n'a aucun usage légitime"
    );
    assert!(
        check_launchable(&agent("claude", &["--ok\u{7}"])).is_err(),
        "un caractère de contrôle dans un argument non plus"
    );
}

/// Rien de ce qui touche la machine n'est accordé.
///
/// C'est la garantie centrale du module. Elle est écrite genre par genre plutôt
/// qu'en bloc : ajouter une variante au protocole ne doit pas la relâcher en
/// silence, et un `match` de test qui énumère oblige à revenir ici.
#[test]
fn aucun_acces_au_systeme_nest_accorde() {
    for genre in [
        ToolKind::Read,
        ToolKind::Search,
        ToolKind::Edit,
        ToolKind::Delete,
        ToolKind::Move,
        ToolKind::Execute,
        ToolKind::Fetch,
        ToolKind::Other,
    ] {
        assert!(
            permission_for(genre).is_refused(),
            "{genre:?} ne doit jamais être accordé : Oxyn est un atelier de bases de données"
        );
    }
}

/// Ce qui ne quitte pas l'agent est accordé, sans quoi rien ne fonctionne.
#[test]
fn ce_qui_reste_dans_lagent_est_accorde() {
    assert_eq!(permission_for(ToolKind::Think), PermissionVerdict::Granted);
    assert_eq!(
        permission_for(ToolKind::SwitchMode),
        PermissionVerdict::Granted
    );
}

/// Chaque refus porte une raison, et elle ne cite aucune valeur.
///
/// La raison part **à l'agent** : sans elle, il reformule sa demande
/// indéfiniment. Et elle s'affiche, donc elle ne doit pas recopier un chemin de
/// fichier ni un identifiant venu de la demande
/// ([I-03](../../../CLAUDE.md#i-03)).
#[test]
fn chaque_refus_dit_pourquoi_sans_citer_la_demande() {
    for genre in [
        ToolKind::Read,
        ToolKind::Edit,
        ToolKind::Delete,
        ToolKind::Move,
        ToolKind::Execute,
        ToolKind::Fetch,
        ToolKind::Search,
        ToolKind::Other,
    ] {
        let PermissionVerdict::Refused(raison) = permission_for(genre) else {
            panic!("{genre:?} devrait être refusé");
        };
        assert!(
            raison.len() > 20 && raison.ends_with('.'),
            "{genre:?} : une raison se lit, {raison:?}"
        );
        // La raison est une constante : elle ne peut pas porter de valeur venue
        // de la demande. Ce test le tient en interdisant les marques de
        // formatage qu'un jour quelqu'un serait tenté d'y glisser.
        assert!(
            !raison.contains('{') && !raison.contains('}'),
            "{genre:?} : une raison ne se compose pas, {raison:?}"
        );
    }
}

/// Une option de chaque genre, telle qu'un agent les propose.
fn options() -> Vec<PermissionOption> {
    [
        (PermissionOptionKind::AllowOnce, "allow-once"),
        (PermissionOptionKind::AllowAlways, "allow-always"),
        (PermissionOptionKind::RejectOnce, "reject-once"),
        (PermissionOptionKind::RejectAlways, "reject-always"),
    ]
    .into_iter()
    .map(|(kind, id)| {
        PermissionOption::new(
            agent_client_protocol::schema::v1::PermissionOptionId::new(id),
            id,
            kind,
        )
    })
    .collect()
}

/// Oxyn n'autorise jamais « toujours », et refuse « toujours » quand il peut.
///
/// L'asymétrie est le sujet : mémoriser un accord large est une décision que
/// l'utilisateur n'a pas prise ; mémoriser un refus n'en est pas une, puisque ce
/// qu'Oxyn refuse il le refusera à chaque fois.
#[test]
fn loxyn_est_prudent_a_lautorisation_et_decisif_au_refus() {
    let toutes = options();

    let accorde = option_for(&PermissionVerdict::Granted, &toutes).expect("une option d'accord");
    assert_eq!(
        accorde.kind,
        PermissionOptionKind::AllowOnce,
        "jamais « autoriser toujours » : l'utilisateur ne l'a pas décidé"
    );

    let refuse =
        option_for(&PermissionVerdict::Refused("non"), &toutes).expect("une option de refus");
    assert_eq!(
        refuse.kind,
        PermissionOptionKind::RejectAlways,
        "ce qu'Oxyn refuse, il le refusera toujours : le redemander fait tourner la conversation en rond"
    );
}

/// Sans l'option qu'il faut, on n'en prend pas une autre.
///
/// Le mode de panne évité : sélectionner une option d'autorisation parce
/// qu'aucune option de refus n'est offerte. `None` fait répondre `Cancelled`,
/// qui interrompt — c'est la seule issue honnête.
#[test]
fn aucune_option_convenable_ne_se_remplace_par_son_contraire() {
    let seulement_accord: Vec<PermissionOption> = options()
        .into_iter()
        .filter(|option| {
            matches!(
                option.kind,
                PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
            )
        })
        .collect();
    assert!(
        option_for(&PermissionVerdict::Refused("non"), &seulement_accord).is_none(),
        "un refus ne se satisfait jamais d'une option d'autorisation"
    );

    let seulement_refus: Vec<PermissionOption> = options()
        .into_iter()
        .filter(|option| {
            matches!(
                option.kind,
                PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways
            )
        })
        .collect();
    assert!(
        option_for(&PermissionVerdict::Granted, &seulement_refus).is_none(),
        "un accord ne se satisfait jamais d'une option de refus"
    );

    assert!(option_for(&PermissionVerdict::Granted, &[]).is_none());
}

/// À défaut de « refuser toujours », « refuser une fois » convient.
#[test]
fn un_refus_se_contente_dun_refus_ponctuel_si_cest_tout_ce_quil_y_a() {
    let ponctuel: Vec<PermissionOption> = options()
        .into_iter()
        .filter(|option| option.kind != PermissionOptionKind::RejectAlways)
        .collect();
    let choisi = option_for(&PermissionVerdict::Refused("non"), &ponctuel).expect("un refus");
    assert_eq!(choisi.kind, PermissionOptionKind::RejectOnce);
}

/// Le niveau est vérifié **avant** que le processus ne soit lancé.
///
/// L'ordre est la garantie, pas seulement le refus. Démarrer l'agent puis
/// refuser de lui parler serait déjà trop tard : le seul fait de le lancer peut
/// suffire à lui faire contacter son service.
///
/// La commande pointe volontairement vers un programme **inexistant**. Sans la
/// garde, l'exécution atteint `connect_with`, tente le lancement et rend
/// « the external agent did not complete its turn » — vérifié par sabotage. Que
/// l'erreur porte sur le **niveau** prouve donc qu'aucun processus n'a démarré.
///
/// Attention au faux sabotage : intervertir la garde et `check_launchable` ne
/// change rien, parce que `check_launchable` ne lance rien — il ne fait que valider
/// une configuration. Le seul lanceur est `connect_with`, et c'est par rapport à
/// lui que l'ordre compte.
#[test]
fn le_niveau_local_refuse_avant_meme_de_lancer_le_processus() {
    let inexistant = agent("/oxyn/ce-programme-nexiste-pas", &["--acp"]);

    // L'invite est composée sous un niveau qui l'admet, puis présentée à
    // `run_turn` sous `Local` : c'est la **défense en profondeur** qu'on éprouve
    // ici. Depuis ADR-0027, `AgentPrompt::from_user` refuserait déjà sous
    // `Local` — mais `run_turn` ne doit pas s'en remettre à son appelant.
    let invite =
        super::prompt::AgentPrompt::from_user(PrivacyTier::Metadata, "quelles tables existent ?")
            .expect("ce niveau admet un agent externe");

    let refus = futures::executor::block_on(super::turn::run_turn(
        &inexistant,
        PrivacyTier::Local,
        &invite,
        &oxyn_core::CancelToken::new(),
        std::sync::Arc::new(()),
    ))
    .expect_err("une connexion locale ne peut pas parler à un agent externe");

    let message = refus.to_string();
    assert!(
        message.contains("local-only"),
        "le refus doit porter sur le niveau, pas sur le lancement : {message}"
    );
    assert!(
        !message.contains("nexiste-pas"),
        "un refus de niveau ne cite pas la commande : {message}"
    );
}

/// Un tour déjà annulé ne lance aucun processus.
///
/// Le jeton n'était pas transmis du tout : `start_agent_turn` en créait un,
/// le rendait au panneau, et ne le passait jamais à `run_turn`. Le bouton
/// d'arrêt changeait donc l'affichage pendant que le sous-processus continuait
/// de parler à un service dont Oxyn ne sait pas où il est — le mode où
/// l'annulation compte le plus, précisément parce que la destination n'est pas
/// vérifiable ([ADR-0026](../../../../docs/adr/0026-agents-externes-acp.md)).
///
/// Ce test porte sur le cas observable sans lancer quoi que ce soit : un jeton
/// déjà armé. Il est volontairement adossé à une commande **inexistante** —
/// si la garde d'annulation disparaissait, l'exécution atteindrait
/// `connect_with` et rendrait une erreur, pas `Cancelled`.
#[test]
fn un_tour_deja_annule_ne_lance_rien() {
    let inexistant = agent("/oxyn/ce-programme-nexiste-pas", &["--acp"]);
    let jeton = oxyn_core::CancelToken::new();
    jeton.cancel();

    let invite =
        super::prompt::AgentPrompt::from_user(PrivacyTier::Metadata, "quelles tables existent ?")
            .expect("invite valide");

    let fin = futures::executor::block_on(super::turn::run_turn(
        &inexistant,
        PrivacyTier::Metadata,
        &invite,
        &jeton,
        std::sync::Arc::new(()),
    ))
    .expect("une annulation est une fin de tour, pas une erreur");

    assert_eq!(
        fin,
        super::turn::TurnEnd::Cancelled,
        "un tour annulé se dit annulé, et surtout ne lance pas le programme"
    );
}

/// Une déclaration invalide est refusée avant le lancement elle aussi.
#[test]
fn une_declaration_invalide_est_refusee_avant_le_lancement() {
    let vide = agent("", &[]);
    let invite = super::prompt::AgentPrompt::from_user(PrivacyTier::Metadata, "bonjour")
        .expect("invite valide");

    let erreur = futures::executor::block_on(super::turn::run_turn(
        &vide,
        PrivacyTier::Metadata,
        &invite,
        &oxyn_core::CancelToken::new(),
        std::sync::Arc::new(()),
    ))
    .expect_err("une commande vide ne se lance pas");
    assert!(erreur.to_string().contains("command"), "{erreur}");
}
