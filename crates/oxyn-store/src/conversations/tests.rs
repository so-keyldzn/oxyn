//! Ce que la persistance des conversations doit tenir.
//!
//! Les tests de cette table portent sur quatre choses qu'une régression rendrait
//! silencieuse : la fidélité de la relecture — raisonnement chiffré compris —,
//! l'ouverture d'un fichier qu'une version antérieure a écrit, la tolérance à
//! une ligne que personne ne sait plus lire, et l'élagage qui ne coupe jamais un
//! transcript en deux.

use super::*;
use crate::Store;
use chrono::TimeDelta;
use oxyn_core::ai::{ReasoningBlock, Role, StopReason};
use oxyn_core::{
    AgentSessionId, AiProviderConfig, AiProviderKind, ConnectionConfig, DriverId, Environment,
    ExecRequest, PrivacyTier, QueryLanguage, ScalarValue, StatementIntent, WorkspaceId,
};

/// Un magasin migré, un workspace, une connexion : le décor commun.
fn decor() -> (Store, WorkspaceId, ConnectionId) {
    let store = Store::open_in_memory().expect("ouverture");
    let workspace = store.workspaces().create("atelier").expect("workspace").id;
    let connexion = ConnectionConfig::new("base client", DriverId::postgres())
        .with_environment(Environment::Production);
    store
        .connections()
        .save(workspace, &connexion)
        .expect("connexion");
    (store, workspace, connexion.id)
}

/// Relit un fil entier, page par page, comme un appelant doit le faire.
///
/// Les pages sont volontairement petites : c'est la couture entre deux pages
/// qu'on veut éprouver à chaque test, pas seulement dans le test qui lui est
/// consacré.
fn tout_le_fil(store: &Store, id: ConversationId) -> Vec<Turn> {
    let mut tours = Vec::new();
    let mut apres = None;
    loop {
        let page = store
            .conversations()
            .transcript_page(id, apres, 2)
            .expect("relecture d'une page");
        tours.extend(page.turns);
        match page.next {
            Some(suivant) => apres = Some(suivant),
            None => return tours,
        }
    }
}

/// Une destination de fournisseur, la forme courante.
fn fournisseur() -> Destination {
    Destination::provider(
        ProviderId::new("anthropic-1a2b3c4d").expect("identifiant"),
        "Anthropic",
        "un-modele",
    )
}

/// Un fil ouvert et enregistré.
fn fil(store: &Store, workspace: WorkspaceId, connexion: ConnectionId) -> ConversationId {
    let conversation = Conversation::new(workspace, fournisseur(), "Doublons de commandes")
        .on_connection(connexion, "base client");
    let id = conversation.id;
    store
        .conversations()
        .save(&conversation)
        .expect("enregistrement du fil");
    id
}

/// Un tour d'assistant chargé : raisonnement rédigé, raisonnement chiffré,
/// appel d'outil, consommation déclarée, raison d'arrêt.
fn tour_charge() -> TurnRecord {
    TurnRecord::new(
        TurnRole::Assistant,
        PrivacyTier::Metadata,
        "Voici la requête qui trouve les doublons.",
    )
    .with_reasoning(vec![
        ReasoningBlock::summarized("Je compare les colonnes indexées", Some("SIG-1".into())),
        ReasoningBlock::redacted("CHARGE-CHIFFREE-DU-FOURNISSEUR"),
    ])
    .with_tool_calls(vec![
        ToolCallRecord::new(
            "call_1",
            "execute",
            "Exécuter une lecture sur « base client »",
            ToolCallStatus::Completed,
        )
        .with_statement("SELECT email, COUNT(*) FROM clients GROUP BY email HAVING COUNT(*) > 1"),
    ])
    .with_usage(TurnUsage {
        prompt: Some(1_200),
        completion: Some(340),
        cache_write: Some(800),
        cache_read: None,
        reasoning: Some(120),
    })
    .stopped(StopReason::EndTurn)
}

// --- Fidélité de l'aller-retour ------------------------------------------

/// Un fil relu est le fil écrit, blocs de raisonnement et consommation compris.
///
/// Le point qui compte n'est pas que la relecture rende « à peu près » la même
/// chose : c'est qu'un bloc de raisonnement revienne **à l'identique**, signature
/// comprise. Un fournisseur qui signe ses blocs refuse le tour suivant si l'un
/// d'eux a été reconstruit — et ce refus n'apparaîtrait ni ici, ni en revue.
#[test]
fn un_fil_relu_est_le_fil_ecrit() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    let session = AgentSessionId::new();

    let question = TurnRecord::new(
        TurnRole::User,
        PrivacyTier::Metadata,
        "Trouve-moi les doublons de clients",
    )
    .in_agent_session(session);
    let reponse = tour_charge().in_agent_session(session);

    assert_eq!(
        store
            .conversations()
            .append(id, &question)
            .expect("écriture"),
        Some(0)
    );
    assert_eq!(
        store
            .conversations()
            .append(id, &reponse)
            .expect("écriture"),
        Some(1)
    );

    let relu = tout_le_fil(&store, id);
    assert_eq!(relu.len(), 2);
    assert_eq!(relu[0].ordinal, 0);
    assert_eq!(relu[0].record.role, TurnRole::User);
    assert_eq!(relu[0].record.agent_session, Some(session));

    let tour = &relu[1].record;
    assert_eq!(tour.role, TurnRole::Assistant);
    assert_eq!(tour.tier, PrivacyTier::Metadata);
    assert_eq!(
        tour.reasoning, reponse.reasoning,
        "un bloc reconstruit fait refuser le tour suivant"
    );
    assert!(
        tour.reasoning.iter().any(ReasoningBlock::is_redacted),
        "le bloc chiffré doit survivre : sans lui le fournisseur perd le fil"
    );
    assert_eq!(tour.tool_calls, reponse.tool_calls);
    assert_eq!(tour.usage, reponse.usage);
    assert_eq!(tour.stop, Some(StopReason::EndTurn));

    // « Non déclaré » et « zéro » restent deux faits différents.
    assert_eq!(tour.usage.cache_read, None);
    assert_eq!(tour.usage.cache_write, Some(800));
}

/// Une raison d'arrêt propre au fournisseur se conserve telle quelle.
///
/// La rabattre sur une variante voisine ferait passer une réponse incomplète
/// pour une réponse finie, et c'est la relecture, un mois plus tard, qui le
/// dirait faux.
#[test]
fn une_raison_d_arret_inconnue_garde_le_mot_du_fournisseur() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    let tour = TurnRecord::new(TurnRole::Assistant, PrivacyTier::Local, "")
        .stopped(StopReason::Other("quota_exhausted".into()));
    store.conversations().append(id, &tour).expect("écriture");

    let relu = tout_le_fil(&store, id);
    assert_eq!(
        relu[0].record.stop,
        Some(StopReason::Other("quota_exhausted".into()))
    );
}

// --- Anciennes fins anormales -----------------------------------------------
//
// Avant `Interrupted` et `ProviderError`, une fin anormale était rangée dans
// `Other("…")` et se relisait comme une fin ordinaire : une réponse tronquée
// qui se donne pour complète. Six formes ont existé
// (`scratchpad/rapport-anthropic-llm-legacy.md`).
//
// Chaque forme est écrite **en dur ici**, et non relue depuis la table du
// module : un test qui parcourrait la table resterait vert le jour où l'on en
// retire une entrée, c'est-à-dire précisément le jour où il doit rougir. Les
// lignes sont posées en SQL direct, sous la forme exacte du disque — `append`
// n'écrit plus ces formes, passer par lui ne prouverait rien.

/// Écrit un tour portant `stop_reason` tel quel, puis rend la raison relue.
fn raison_relue(stop_reason_json: &str) -> Option<StopReason> {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO ai_conversation_turns
                     (conversation_id, ordinal, ts, role, privacy_tier, text, stop_reason)
                 VALUES (?1, 0, ?2, 'assistant', 'metadata', 'Voici le début de la rép', ?3)",
                rusqlite::params![id.to_string(), Utc::now(), stop_reason_json],
            )?;
            Ok(())
        })
        .expect("ligne écrite par une version précédente");
    tout_le_fil(&store, id)
        .pop()
        .and_then(|tour| tour.record.stop)
}

/// Vérifie qu'une ancienne forme se relit comme `attendu`, et jamais comme une
/// réponse complète.
fn verifier_forme_ancienne(stop_reason_json: &str, attendu: StopReason) {
    let relue = raison_relue(stop_reason_json);
    assert_eq!(relue.as_ref(), Some(&attendu), "forme {stop_reason_json}");
    assert!(
        attendu.is_truncated(),
        "une fin anormale ne doit jamais se relire comme une réponse complète"
    );
}

#[test]
fn ancienne_forme_flux_interrompu_se_relit_interrompue() {
    verifier_forme_ancienne(r#"{"Other":"flux interrompu"}"#, StopReason::Interrupted);
}

#[test]
fn ancienne_forme_flux_illisible_se_relit_interrompue() {
    verifier_forme_ancienne(r#"{"Other":"flux illisible"}"#, StopReason::Interrupted);
}

#[test]
fn ancienne_forme_interrupted_stream_se_relit_interrompue() {
    verifier_forme_ancienne(r#"{"Other":"interrupted stream"}"#, StopReason::Interrupted);
}

#[test]
fn ancienne_forme_unreadable_stream_se_relit_interrompue() {
    verifier_forme_ancienne(r#"{"Other":"unreadable stream"}"#, StopReason::Interrupted);
}

/// Une erreur annoncée par le fournisseur est tronquée mais **pas** ambiguë :
/// la relire `Interrupted` bloquerait une reprise légitime.
#[test]
fn ancienne_forme_erreur_du_fournisseur_se_relit_erreur_fournisseur() {
    verifier_forme_ancienne(
        r#"{"Other":"erreur du fournisseur"}"#,
        StopReason::ProviderError,
    );
}

#[test]
fn ancienne_forme_provider_error_se_relit_erreur_fournisseur() {
    verifier_forme_ancienne(r#"{"Other":"provider error"}"#, StopReason::ProviderError);
}

/// La frontière : seules les six formes exactes sont reconnues.
///
/// Un autre mot de fournisseur reste le sien, et une forme voisine — casse,
/// espace, préfixe — n'est pas une ancienne forme : élargir la reconnaissance
/// transformerait en coupure un arrêt que personne n'a jamais signalé comme tel.
#[test]
fn aucune_autre_valeur_d_other_n_est_reconnue() {
    for mot in [
        "quota_exhausted",
        "Interrupted Stream",
        "interrupted stream ",
        "flux interrompu par le client",
        "provider",
        "",
    ] {
        let json = serde_json::to_string(&StopReason::Other(mot.into())).expect("encodage");
        assert_eq!(
            raison_relue(&json),
            Some(StopReason::Other(mot.into())),
            "`{mot}` ne doit pas être reconnu"
        );
    }
}

/// Le niveau est noté sur le **tour**, pas sur le fil.
///
/// C'est ce qui permet à un audit de dire sous quel régime chaque tour a eu
/// lieu quand l'utilisateur change le niveau de la connexion en cours de route
/// (I-04). Un niveau rangé une fois en tête dirait faux de la moitié du fil.
#[test]
fn le_niveau_suit_le_tour_et_non_le_fil() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);

    store
        .conversations()
        .append(
            id,
            &TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "première question"),
        )
        .expect("écriture");
    store
        .conversations()
        .append(
            id,
            &TurnRecord::new(TurnRole::User, PrivacyTier::Sampled, "seconde question"),
        )
        .expect("écriture");

    let relu = tout_le_fil(&store, id);
    assert_eq!(relu[0].record.tier, PrivacyTier::Metadata);
    assert_eq!(
        relu[1].record.tier,
        PrivacyTier::Sampled,
        "le second tour a eu lieu sous un autre régime, et la relecture doit le dire"
    );
}

/// Le fil survit à la suppression de la connexion, sous son nom d'alors.
#[test]
fn un_fil_survit_a_la_suppression_de_sa_connexion() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    store
        .conversations()
        .append(
            id,
            &TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "une question"),
        )
        .expect("écriture");

    store.connections().delete(connexion).expect("suppression");

    let relu = store
        .conversations()
        .get(id)
        .expect("relecture")
        .expect("le fil existe encore");
    assert_eq!(relu.connection_name.as_deref(), Some("base client"));
    assert_eq!(tout_le_fil(&store, id).len(), 1);
}

/// Un tour écrit dans un fil disparu n'est pas une panne, c'est un `None`.
#[test]
fn ecrire_dans_un_fil_disparu_rend_none() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    assert!(store.conversations().delete(id).expect("suppression"));

    let tour = TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "trop tard");
    assert_eq!(
        store.conversations().append(id, &tour).expect("écriture"),
        None
    );
}

/// Supprimer un fil emporte ses tours : la clé étrangère le tient, pas le code.
#[test]
fn supprimer_un_fil_emporte_ses_tours() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    for index in 0..4 {
        store
            .conversations()
            .append(
                id,
                &TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, format!("q{index}")),
            )
            .expect("écriture");
    }
    store.conversations().delete(id).expect("suppression");

    let restants: i64 = store
        .with_connection(|conn| {
            Ok(
                conn.query_row("SELECT COUNT(*) FROM ai_conversation_turns", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .expect("comptage");
    assert_eq!(restants, 0, "aucun tour orphelin ne doit subsister");
}

/// La liste d'une connexion est ordonnée par activité, et compte ses tours.
#[test]
fn la_liste_est_ordonnee_par_activite() {
    let (store, workspace, connexion) = decor();
    let ancien = fil(&store, workspace, connexion);
    let recent = fil(&store, workspace, connexion);

    store
        .conversations()
        .append(
            ancien,
            &TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "vieux"),
        )
        .expect("écriture");
    let mut plus_tard = TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "neuf");
    plus_tard.ts = Utc::now() + TimeDelta::minutes(5);
    store
        .conversations()
        .append(recent, &plus_tard)
        .expect("écriture");

    let liste = store.conversations().list(connexion, 10).expect("liste");
    assert_eq!(liste.len(), 2);
    assert_eq!(liste[0].id, recent);
    assert_eq!(liste[0].turns, 1);
    assert_eq!(liste[1].id, ancien);
}

/// Renommer ne réordonne pas l'historique sous le curseur de l'utilisateur.
#[test]
fn renommer_n_est_pas_une_activite() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    let avant = store
        .conversations()
        .get(id)
        .expect("relecture")
        .expect("fil")
        .updated_at;

    assert!(
        store
            .conversations()
            .rename(id, "Nouveau titre")
            .expect("renommage")
    );

    let apres = store
        .conversations()
        .get(id)
        .expect("relecture")
        .expect("fil");
    assert_eq!(apres.title, "Nouveau titre");
    assert_eq!(apres.updated_at, avant);
}

// --- Bornes d'écriture ----------------------------------------------------

/// Ce qui ne rentre pas est refusé, jamais tronqué.
#[test]
fn un_tour_trop_gros_est_refuse_et_non_tronque() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    let tour = TurnRecord::new(
        TurnRole::Assistant,
        PrivacyTier::Metadata,
        "x".repeat(MAX_TURN_TEXT_BYTES + 1),
    );

    let erreur = store
        .conversations()
        .append(id, &tour)
        .expect_err("la borne doit refuser");
    assert!(
        matches!(erreur, StoreError::TooLarge { field, .. } if field.ends_with(".text")),
        "{erreur:?}"
    );
    assert!(
        tout_le_fil(&store, id).is_empty(),
        "un refus ne doit rien laisser derrière lui"
    );
}

/// La borne du fichier vaut aussi pour un tiers armé de `sqlite3`.
#[test]
fn les_bornes_tiennent_dans_le_fichier() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    let refus = store.with_connection(|conn| {
        Ok(conn.execute(
            "INSERT INTO ai_conversation_turns
                 (conversation_id, ordinal, ts, role, privacy_tier, text)
             VALUES (?1, 0, ?2, 'assistant', 'metadata', ?3)",
            rusqlite::params![
                id.to_string(),
                Utc::now(),
                "x".repeat(MAX_TURN_TEXT_BYTES + 1)
            ],
        )?)
    });
    assert!(
        refus.is_err(),
        "le budget doit être opposable hors d'Oxyn, pas seulement dans le code"
    );
}

/// Écrit `count` tours d'utilisateur en SQL direct, sans passer par `append`.
fn tours_bruts(store: &Store, id: ConversationId, count: u32) {
    store
        .with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            for ordinal in 0..count {
                tx.execute(
                    "INSERT INTO ai_conversation_turns
                         (conversation_id, ordinal, ts, role, privacy_tier, text)
                     VALUES (?1, ?2, ?3, 'user', 'metadata', ?4)",
                    rusqlite::params![id.to_string(), ordinal, Utc::now(), format!("q{ordinal}")],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .expect("tours écrits");
}

/// Le fichier refuse un tour au-delà de la borne et une raison d'arrêt trop
/// longue, y compris pour un `sqlite3` — à l'insertion comme à la mise à jour.
///
/// Et les deux nombres des déclencheurs sont ceux du code : une borne qui
/// divergerait entre les deux ferait refuser au fichier ce que le code croit
/// permis, ou l'inverse, sans que rien ne le signale.
#[test]
fn le_fichier_borne_les_tours_et_la_raison_d_arret() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    let inserer = |ordinal: u32, stop: Option<String>| {
        store.with_connection(|conn| {
            Ok(conn.execute(
                "INSERT INTO ai_conversation_turns
                     (conversation_id, ordinal, ts, role, privacy_tier, text, stop_reason)
                 VALUES (?1, ?2, ?3, 'assistant', 'metadata', '', ?4)",
                rusqlite::params![id.to_string(), ordinal, Utc::now(), stop],
            )?)
        })
    };

    assert!(
        inserer(MAX_TURNS_PER_CONVERSATION, None).is_err(),
        "un tour au-delà de la borne"
    );
    let trop_long = format!("\"{}\"", "x".repeat(MAX_STOP_REASON_BYTES));
    assert!(
        inserer(0, Some(trop_long.clone())).is_err(),
        "une raison d'arrêt trop longue"
    );
    inserer(MAX_TURNS_PER_CONVERSATION - 1, Some("\"EndTurn\"".into()))
        .expect("la dernière position admise");
    let mise_a_jour = store.with_connection(|conn| {
        Ok(conn.execute(
            "UPDATE ai_conversation_turns SET stop_reason = ?1",
            rusqlite::params![trop_long],
        )?)
    });
    assert!(mise_a_jour.is_err(), "la mise à jour est bornée aussi");

    let declencheur: String = store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT sql FROM sqlite_schema WHERE name = 'ai_conversation_turns_bounds_insert'",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("déclencheur");
    assert!(declencheur.contains(&format!(">= {MAX_TURNS_PER_CONVERSATION}")));
    assert!(declencheur.contains(&format!("> {MAX_STOP_REASON_BYTES}")));
}

/// `append` refuse le tour de trop avec une erreur qui nomme la borne.
#[test]
fn un_fil_plein_refuse_le_tour_suivant() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    tours_bruts(&store, id, MAX_TURNS_PER_CONVERSATION);

    let erreur = store
        .conversations()
        .append(
            id,
            &TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "de trop"),
        )
        .expect_err("le fil est plein");
    assert!(
        matches!(erreur, StoreError::TooLarge { field, .. } if field.ends_with(".ordinal")),
        "{erreur:?}"
    );
}

/// Un mot de fournisseur trop long est raccourci, et le tour n'est pas perdu.
///
/// Refuser le tour ferait disparaître de l'historique la réponse du modèle
/// parce qu'un fournisseur a choisi un long mot pour dire pourquoi il s'est
/// arrêté. Le mot est une étiquette, pas un transcript.
#[test]
fn une_raison_d_arret_trop_longue_est_raccourcie_et_le_tour_garde() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    // Des guillemets, pour que l'échappement JSON allonge l'encodage au-delà
    // du texte brut : c'est le cas où couper au nombre d'octets ne suffit pas.
    let mot = "\"é".repeat(10_000);
    let tour = TurnRecord::new(TurnRole::Assistant, PrivacyTier::Metadata, "La réponse")
        .stopped(StopReason::Other(mot));
    store
        .conversations()
        .append(id, &tour)
        .expect("le tour est écrit");

    let relu = tout_le_fil(&store, id);
    assert_eq!(relu[0].record.text, "La réponse");
    let Some(StopReason::Other(garde)) = &relu[0].record.stop else {
        panic!(
            "la raison reste le mot du fournisseur : {:?}",
            relu[0].record.stop
        );
    };
    assert!(garde.ends_with('…'), "le raccourci se voit");
    assert!(
        serde_json::to_string(&StopReason::Other(garde.clone()))
            .expect("encodage")
            .len()
            <= MAX_STOP_REASON_BYTES
    );
}

/// La lecture paginée rend tout le fil, dans l'ordre, sans page vide ni
/// doublon — et ses bornes de `limit` tiennent.
#[test]
fn la_lecture_paginee_rend_tout_le_fil_et_rien_de_plus() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    tours_bruts(&store, id, 20);

    let premiere = store
        .conversations()
        .transcript_page(id, None, u16::MAX)
        .expect("page");
    assert_eq!(
        premiere.turns.len(),
        usize::from(MAX_TURN_PAGE),
        "`limit` est plafonné"
    );
    assert_eq!(premiere.next, Some(u32::from(MAX_TURN_PAGE) - 1));

    let seconde = store
        .conversations()
        .transcript_page(id, premiere.next, u16::MAX)
        .expect("page");
    assert_eq!(seconde.turns.len(), 4);
    assert_eq!(seconde.next, None, "pas de page vide après la dernière");

    let unitaire = store
        .conversations()
        .transcript_page(id, None, 0)
        .expect("page");
    assert_eq!(unitaire.turns.len(), 1, "`limit` nul vaut un");

    let ordinaux: Vec<u32> = tout_le_fil(&store, id)
        .iter()
        .map(|tour| tour.ordinal)
        .collect();
    assert_eq!(ordinaux, (0..20).collect::<Vec<_>>());

    // Un fil dont le compte est un multiple exact de la page.
    let pair = fil(&store, workspace, connexion);
    tours_bruts(&store, pair, 4);
    let page = store
        .conversations()
        .transcript_page(pair, Some(1), 2)
        .expect("page");
    assert_eq!(page.turns.len(), 2);
    assert_eq!(page.next, None);
}

// --- Relire est plus permissif qu'écrire ----------------------------------

/// Ce que le fichier contenait avant la migration 11 se relit encore.
///
/// La migration 11 pose des déclencheurs et non des `CHECK` précisément pour
/// ce cas : une raison d'arrêt démesurée et des tours au-delà de la borne,
/// écrits quand rien ne les refusait, ne doivent ni faire échouer l'ouverture
/// ni être réécrits. La raison trop longue n'est pas chargée — elle se relit
/// `Unspecified` —, et les tours en surnombre se lisent par pages bornées.
#[test]
fn un_fichier_anterieur_aux_bornes_s_ouvre_et_se_relit() {
    let racine = tempfile::tempdir().expect("répertoire temporaire");
    let chemin = racine.path().join("oxyn.sqlite3");
    let id = ConversationId::new();
    {
        // Un fichier en version 10 : ni bornes, ni arbre.
        let conn = crate::schema::file_at_version(&chemin, 10);
        let atelier = oxyn_core::WorkspaceId::new();
        // La forme exacte qu'écrit rusqlite pour un `DateTime<Utc>`.
        let quand = Utc::now().format("%F %T%.f%:z");
        conn.execute_batch(&format!(
            "INSERT INTO workspaces VALUES ('{atelier}', 'atelier', '{quand}', '{quand}');
             INSERT INTO ai_conversations
                 (id, workspace_id, destination_kind, destination_label, title,
                  created_at, updated_at)
             VALUES ('{id}', '{atelier}', 'provider', 'Anthropic', 'Fil', '{quand}', '{quand}');"
        ))
        .expect("fil d'une version antérieure");
        conn.execute(
            "INSERT INTO ai_conversation_turns
                 (conversation_id, ordinal, ts, role, privacy_tier, text, stop_reason)
             VALUES (?1, 0, ?2, 'assistant', 'metadata', 'réponse', ?3),
                    (?1, 700, ?2, 'user', 'metadata', 'au-delà de la borne', NULL)",
            rusqlite::params![
                id.to_string(),
                Utc::now(),
                format!("{{\"Other\":\"{}\"}}", "x".repeat(8 * 1024 * 1024))
            ],
        )
        .expect("tours d'une version antérieure");
    }

    let store = Store::open_at(&chemin).expect("l'ouverture ne doit pas échouer");
    let relu = tout_le_fil(&store, id);
    assert_eq!(relu.len(), 2, "aucune ligne perdue ni réécrite");
    assert_eq!(relu[0].record.text, "réponse");
    assert_eq!(
        relu[0].record.stop,
        Some(StopReason::Unspecified),
        "la valeur démesurée n'est pas chargée"
    );
    assert_eq!(relu[1].ordinal, 700);

    // Et les déclencheurs sont revenus pour ce qui s'écrira ensuite.
    let declencheurs: i64 = store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM sqlite_schema
                  WHERE type = 'trigger' AND name LIKE 'ai_conversation_turns_bounds_%'",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("comptage");
    assert_eq!(declencheurs, 2);
}

/// Une ligne que personne ne sait plus lire ne fait pas échouer l'ouverture.
///
/// Chaque retombée est vérifiée dans le sens contraignant : un rôle inconnu
/// n'est ni l'utilisateur ni l'assistant, un niveau illisible vaut `Local`, une
/// issue d'outil illisible ne passe pas pour un succès. Un transcript qu'on ne
/// peut plus ouvrir parce qu'une ligne est étrange ne protège personne.
#[test]
fn une_ligne_corrompue_n_empeche_pas_l_ouverture() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    store
        .conversations()
        .append(
            id,
            &TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "une question saine"),
        )
        .expect("écriture");

    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO ai_conversation_turns
                     (conversation_id, ordinal, ts, role, privacy_tier, text, reasoning,
                      tool_calls, stop_reason, prompt_tokens)
                 VALUES (?1, 1, ?2, 'oracle', 'confidentiel', 'du texte', '{ pas du json',
                         '[{\"call_id\":\"c\",\"tool\":\"t\",\"summary\":\"s\",\"status\":\"vaporise\"}]',
                         'pas du json non plus', -7)",
                rusqlite::params![id.to_string(), Utc::now()],
            )?;
            Ok(())
        })
        .expect("ligne écrite hors d'Oxyn");

    let relu = tout_le_fil(&store, id);
    assert_eq!(relu.len(), 2, "la ligne saine et la ligne étrange");

    let etrange = &relu[1].record;
    assert_eq!(
        etrange.role,
        TurnRole::Unknown,
        "un rôle inconnu n'est ni l'utilisateur ni l'assistant"
    );
    assert_eq!(
        etrange.tier,
        PrivacyTier::Local,
        "un niveau illisible retombe sur le plus contraignant"
    );
    assert_eq!(etrange.text, "du texte", "le texte lisible reste lisible");
    assert!(
        etrange.reasoning.is_empty(),
        "un raisonnement illisible est écarté, le tour survit"
    );
    assert_eq!(
        etrange.tool_calls[0].status,
        ToolCallStatus::Unknown,
        "une issue illisible ne passe pas pour un succès"
    );
    assert_eq!(
        etrange.stop,
        Some(StopReason::Other("pas du json non plus".into())),
        "le mot du fournisseur vaut mieux qu'une absence"
    );
    assert_eq!(
        etrange.usage.prompt, None,
        "un compte hors bornes se lit « non déclaré », pas comme un nombre faux"
    );
}

/// Un identifiant de destination illisible n'efface pas le fil de la liste.
#[test]
fn une_destination_illisible_laisse_le_fil_lisible() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE ai_conversations SET destination_id = 'PAS UN IDENTIFIANT',
                                             destination_kind = 'oracle' WHERE id = ?1",
                rusqlite::params![id.to_string()],
            )?;
            Ok(())
        })
        .expect("écriture hors d'Oxyn");

    let relu = store
        .conversations()
        .get(id)
        .expect("relecture")
        .expect("fil");
    assert_eq!(relu.destination.id, None);
    assert_eq!(relu.destination.kind, DestinationKind::Unknown);
    assert_eq!(relu.destination.label, "Anthropic");
}

// --- Élagage ---------------------------------------------------------------

/// Écrit `count` fils, du plus ancien au plus récent, avec un tour chacun.
fn fils_dates(
    store: &Store,
    workspace: WorkspaceId,
    connexion: ConnectionId,
    count: u32,
) -> Vec<ConversationId> {
    let base = Utc::now() - TimeDelta::days(i64::from(count) + 1);
    (0..count)
        .map(|index| {
            let quand = base + TimeDelta::days(i64::from(index));
            let mut conversation =
                Conversation::new(workspace, fournisseur(), format!("fil {index}"))
                    .on_connection(connexion, "base client");
            conversation.created_at = quand;
            conversation.updated_at = quand;
            let id = conversation.id;
            store.conversations().save(&conversation).expect("fil");
            let mut tour = TurnRecord::new(
                TurnRole::User,
                PrivacyTier::Metadata,
                format!("question {index}"),
            );
            tour.ts = quand;
            store.conversations().append(id, &tour).expect("tour");
            id
        })
        .collect()
}

/// L'élagage respecte la borne de nombre et garde les plus récents.
#[test]
fn l_elagage_garde_les_plus_recents() {
    let (store, workspace, connexion) = decor();
    let fils = fils_dates(&store, workspace, connexion, 10);
    let politique = RetentionPolicy {
        max_conversations: 4,
        max_age_days: None,
        max_bytes: u64::MAX,
    };

    let rapport = store
        .conversations()
        .prune(workspace, politique)
        .expect("élagage");
    assert_eq!(rapport.conversations, 6);
    assert_eq!(rapport.turns, 6);

    let restants = store.conversations().list(connexion, 100).expect("liste");
    assert_eq!(restants.len(), 4);
    for garde in &fils[6..] {
        assert!(
            restants.iter().any(|fil| fil.id == *garde),
            "les quatre plus récents doivent rester"
        );
    }

    // Deux fois de suite ne change rien.
    assert!(
        store
            .conversations()
            .prune(workspace, politique)
            .expect("second élagage")
            .is_empty()
    );
}

/// L'élagage ne coupe **jamais** une conversation en deux.
///
/// C'est la propriété qui compte : un transcript amputé du milieu se relit
/// comme un transcript complet, et personne ne peut dire que quelque chose a
/// été retiré plutôt que jamais dit.
#[test]
fn l_elagage_ne_coupe_jamais_un_fil_en_deux() {
    let (store, workspace, connexion) = decor();
    let fils = fils_dates(&store, workspace, connexion, 6);
    // Le plus ancien reçoit beaucoup de tours : c'est lui que la borne vise.
    for index in 0..20 {
        let mut tour = TurnRecord::new(
            TurnRole::Assistant,
            PrivacyTier::Metadata,
            format!("réponse {index}"),
        );
        tour.ts = Utc::now() - TimeDelta::days(7);
        store
            .conversations()
            .append(fils[0], &tour)
            .expect("écriture");
    }

    store
        .conversations()
        .prune(
            workspace,
            RetentionPolicy {
                max_conversations: 3,
                max_age_days: None,
                max_bytes: u64::MAX,
            },
        )
        .expect("élagage");

    // Aucun tour ne subsiste sans son en-tête, et aucun en-tête n'a perdu la
    // moitié de ses tours : chaque fil restant a exactement ce qu'il avait.
    let orphelins: i64 = store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM ai_conversation_turns t
                  WHERE NOT EXISTS (SELECT 1 FROM ai_conversations c WHERE c.id = t.conversation_id)",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("comptage");
    assert_eq!(orphelins, 0);
    assert!(
        store
            .conversations()
            .get(fils[0])
            .expect("relecture")
            .is_none(),
        "le fil visé doit être parti en entier"
    );
}

/// La borne d'âge élague sur l'inactivité, pas sur la création.
#[test]
fn l_elagage_par_age_regarde_la_derniere_activite() {
    let (store, workspace, connexion) = decor();
    let ancien = fil(&store, workspace, connexion);
    let mut vieux_tour = TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "il y a longtemps");
    vieux_tour.ts = Utc::now() - TimeDelta::days(200);
    store
        .conversations()
        .append(ancien, &vieux_tour)
        .expect("écriture");

    let vivant = fil(&store, workspace, connexion);
    store
        .conversations()
        .append(
            vivant,
            &TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "aujourd'hui"),
        )
        .expect("écriture");

    let rapport = store
        .conversations()
        .prune(
            workspace,
            RetentionPolicy {
                max_conversations: u32::MAX,
                max_age_days: Some(90),
                max_bytes: u64::MAX,
            },
        )
        .expect("élagage");
    assert_eq!(rapport.conversations, 1);
    assert!(
        store
            .conversations()
            .get(ancien)
            .expect("relecture")
            .is_none()
    );
    assert!(
        store
            .conversations()
            .get(vivant)
            .expect("relecture")
            .is_some()
    );
}

/// La borne d'octets n'efface jamais le fil que l'utilisateur regarde.
#[test]
fn la_borne_d_octets_epargne_le_fil_courant() {
    let (store, workspace, connexion) = decor();
    let fils = fils_dates(&store, workspace, connexion, 3);

    let rapport = store
        .conversations()
        .prune(
            workspace,
            RetentionPolicy {
                max_conversations: u32::MAX,
                max_age_days: None,
                max_bytes: 1,
            },
        )
        .expect("élagage");
    assert_eq!(rapport.conversations, 2);
    assert!(
        store
            .conversations()
            .get(fils[2])
            .expect("relecture")
            .is_some(),
        "supprimer le fil courant pour tenir un budget serait une perte visible"
    );
}

/// Supprimer un workspace emporte ses conversations.
#[test]
fn supprimer_un_workspace_emporte_ses_conversations() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    store.workspaces().delete(workspace).expect("suppression");
    assert!(store.conversations().get(id).expect("relecture").is_none());
}

// --- Mesure ---------------------------------------------------------------

/// Ce que coûte une conversation moyenne, mesuré et non supposé.
///
/// Le chiffre sert à fixer [`RetentionPolicy`] : sans lui, les bornes seraient
/// des valeurs plausibles, c'est-à-dire des valeurs fausses que rien ne
/// signale. Le test échoue si le coût unitaire sort de la fourchette annoncée —
/// ce qui arrive le jour où une colonne s'ajoute, et c'est précisément le jour
/// où la politique doit être rejugée.
#[test]
fn une_conversation_moyenne_coute_ce_que_la_politique_suppose() {
    // Sur fichier et non en mémoire : ce qu'on veut savoir, c'est ce que la
    // conversation coûte **au disque de l'utilisateur**, index et surcoût de
    // ligne compris — et une base en mémoire ne le dit pas.
    let racine = tempfile::tempdir().expect("répertoire temporaire");
    let store = Store::open_at(racine.path().join("oxyn.sqlite3")).expect("ouverture");
    let workspace = store.workspaces().create("atelier").expect("workspace").id;
    let connexion = ConnectionConfig::new("base client", DriverId::postgres());
    store
        .connections()
        .save(workspace, &connexion)
        .expect("connexion");
    let connexion = connexion.id;
    let avant = octets_du_fichier(&store);

    // Douze échanges : la longueur d'une séance de travail sur un schéma.
    const ECHANGES: usize = 12;
    let id = fil(&store, workspace, connexion);
    for index in 0..ECHANGES {
        store
            .conversations()
            .append(
                id,
                &TurnRecord::new(
                    TurnRole::User,
                    PrivacyTier::Metadata,
                    format!("Question {index} sur le schéma des commandes et leurs jointures."),
                ),
            )
            .expect("question");
        let mut reponse = tour_charge();
        // Une réponse réelle : un paragraphe, une requête, un raisonnement
        // rédigé et une charge chiffrée de taille réaliste.
        reponse.text = "r".repeat(2_048);
        reponse.reasoning = vec![
            ReasoningBlock::summarized("t".repeat(1_024), Some("s".repeat(512))),
            ReasoningBlock::redacted("c".repeat(1_536)),
        ];
        store.conversations().append(id, &reponse).expect("réponse");
    }

    let transcript = store.conversations().bytes_held(workspace).expect("mesure");
    let fichier = octets_du_fichier(&store).saturating_sub(avant);
    println!(
        "conversation de {ECHANGES} échanges : {transcript} octets de transcript, \
         {fichier} octets de fichier"
    );

    assert!(
        (48 * 1024..=128 * 1024).contains(&transcript),
        "coût unitaire hors de la fourchette qui fonde RetentionPolicy : {transcript} octets"
    );
    // Le fichier coûte davantage que le transcript — surcoût de ligne, pages,
    // index. Le facteur est ce qui traduit `max_bytes` en place occupée ; s'il
    // s'envole, la borne ne veut plus dire ce que sa documentation dit.
    assert!(
        fichier <= transcript * 3,
        "le fichier coûte {fichier} octets pour {transcript} de transcript"
    );

    // Les deux bornes par défaut doivent parler de la même chose : le budget
    // d'octets doit tenir au moins la moitié du nombre de conversations admis,
    // sinon l'une des deux ne sert jamais.
    let defaut = RetentionPolicy::default();
    assert!(
        defaut.max_bytes / transcript >= u64::from(defaut.max_conversations) / 2,
        "les deux bornes par défaut ne parlent pas de la même chose"
    );
}

/// La taille réellement occupée par le fichier de l'état local.
fn octets_du_fichier(store: &Store) -> u64 {
    store
        .with_connection(|conn| {
            let pages: i64 = conn.query_row("PRAGMA page_count", [], |row| row.get(0))?;
            let taille: i64 = conn.query_row("PRAGMA page_size", [], |row| row.get(0))?;
            Ok(crate::encoding::count_from_i64(
                pages.saturating_mul(taille),
            ))
        })
        .expect("mesure du fichier")
}

// --- Aucun secret ---------------------------------------------------------

/// Aucun chemin public n'offre d'endroit où ranger une valeur liée.
///
/// Le test passe une valeur témoin par les deux portes qu'un appelant pourrait
/// croire commodes — les valeurs liées d'une `ExecRequest` et les arguments
/// d'un appel d'outil — puis balaie **toutes** les colonnes de la table. Ce
/// qu'il prouve n'est pas qu'on a pensé à ne pas les écrire : c'est qu'il n'y a
/// pas de champ pour le faire ([I-03](../../../../CLAUDE.md#i-03)).
#[test]
fn aucune_valeur_liee_ni_cle_n_atteint_la_table() {
    const TEMOIN: &str = "oxyn-temoin-ne-doit-jamais-atteindre-la-table-des-conversations";

    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);

    // Une requête d'agent, telle qu'elle traverse le bus : l'instruction est
    // légitime et reste, ses valeurs liées n'ont aucun endroit où aller.
    let requete = ExecRequest::new(QueryLanguage::SQL, "SELECT * FROM clients WHERE email = $1")
        .with_intent(StatementIntent::Read)
        .with_params(vec![ScalarValue::from(TEMOIN)]);

    // Un fournisseur déclaré, dont la clé n'est qu'une référence de trousseau.
    let fournisseur_declare = AiProviderConfig::new(
        ProviderId::new("temoin-fournisseur").expect("identifiant"),
        AiProviderKind::OpenAiCompatible,
        "Fournisseur témoin",
        "http://127.0.0.1:11434/v1",
        "un-modele",
    )
    .with_secret_ref(format!("keychain://oxyn/{TEMOIN}"));
    store
        .providers()
        .save(&fournisseur_declare)
        .expect("fournisseur");

    let tour = TurnRecord::new(
        TurnRole::Assistant,
        PrivacyTier::Metadata,
        "J'ai cherché ce client.",
    )
    .with_tool_calls(vec![
        ToolCallRecord::new(
            "call_1",
            "execute",
            "Exécuter une lecture sur « base client »",
            ToolCallStatus::Completed,
        )
        .with_statement(requete.text.clone()),
    ]);
    store.conversations().append(id, &tour).expect("écriture");

    let fuites = balayer_les_conversations(&store, TEMOIN);
    assert!(
        fuites.is_empty(),
        "la valeur témoin a atteint la table : {fuites:#?}"
    );

    // Le test ne prouverait rien si rien n'avait été écrit.
    let relu = tout_le_fil(&store, id);
    assert_eq!(
        relu[0].record.tool_calls[0].statement.as_deref(),
        Some("SELECT * FROM clients WHERE email = $1"),
        "le SQL reste : c'est la question, pas une valeur"
    );
    assert!(
        !requete.params.is_empty(),
        "la requête devait bien porter une valeur liée"
    );
}

/// Toutes les valeurs textuelles des deux tables, colonne par colonne.
///
/// Passe par `pragma_table_info` plutôt que par une liste : une colonne ajoutée
/// demain est balayée sans que personne n'ait à y penser.
fn balayer_les_conversations(store: &Store, temoin: &str) -> Vec<(String, String)> {
    store
        .with_connection(|conn| {
            let mut trouvailles = Vec::new();
            for table in ["ai_conversations", "ai_conversation_turns"] {
                let colonnes: Vec<String> = conn
                    .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?
                    .query_map([], |row| row.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                for colonne in colonnes {
                    let mut requete = conn.prepare(&format!(
                        "SELECT CAST(\"{colonne}\" AS TEXT) FROM \"{table}\" \
                         WHERE \"{colonne}\" IS NOT NULL"
                    ))?;
                    let valeurs = requete.query_map([], |row| row.get::<_, String>(0))?;
                    for valeur in valeurs.flatten() {
                        if valeur.contains(temoin) {
                            trouvailles.push((colonne.clone(), valeur));
                        }
                    }
                }
            }
            Ok(trouvailles)
        })
        .expect("balayage")
}

/// Les `Debug` comptent le texte de l'utilisateur, ils ne le rendent jamais.
///
/// Les trois formes portent du texte saisi — un tour, un en-tête, une ligne de
/// liste — et un titre est le plus souvent la première question raccourcie. En
/// masquer deux sur trois donnerait une protection que le troisième annule, et
/// c'est le `tracing::debug!` ajouté dans six mois qui s'en chargerait.
#[test]
fn les_debug_ne_rendent_pas_le_texte_de_l_utilisateur() {
    // Un appel d'outil : l'instruction d'un agent recopie ce qu'il a lu.
    let appel = ToolCallRecord::new(
        "call_1",
        "execute",
        "ALTER ROLE app PASSWORD 'hunter2-temoin'",
        ToolCallStatus::Completed,
    )
    .with_statement("ALTER ROLE app PASSWORD 'hunter2-temoin'");
    let rendu = format!("{appel:?}");
    assert!(!rendu.contains("hunter2"), "{rendu}");
    assert!(rendu.contains("statement_bytes"), "{rendu}");
    assert!(rendu.contains("execute"), "{rendu}");

    let tour = TurnRecord::new(
        TurnRole::User,
        PrivacyTier::Metadata,
        "mot-de-passe-colle-par-erreur",
    );
    let rendu = format!("{tour:?}");
    assert!(!rendu.contains("mot-de-passe"), "{rendu}");
    assert!(rendu.contains("text_bytes"), "{rendu}");

    let (store, workspace, connexion) = decor();
    let conversation = Conversation::new(workspace, fournisseur(), "question-collee-par-erreur")
        .on_connection(connexion, "base client");
    store.conversations().save(&conversation).expect("fil");
    let rendu = format!("{conversation:?}");
    assert!(!rendu.contains("question-collee"), "{rendu}");
    assert!(rendu.contains("title_bytes"), "{rendu}");

    let liste = store.conversations().list(connexion, 10).expect("liste");
    let rendu = format!("{:?}", liste[0]);
    assert!(!rendu.contains("question-collee"), "{rendu}");
    assert!(rendu.contains("title_bytes"), "{rendu}");
}

/// Un rôle du fil venu d'`oxyn-llm` se convertit sans perte.
#[test]
fn les_roles_du_fil_couvrent_ceux_du_protocole() {
    for (role, attendu) in [
        (Role::System, TurnRole::System),
        (Role::User, TurnRole::User),
        (Role::Assistant, TurnRole::Assistant),
        (Role::Tool, TurnRole::Tool),
    ] {
        assert_eq!(TurnRole::from(role), attendu);
        assert_eq!(TurnRole::from_text(attendu.as_str()), attendu);
    }
    assert_eq!(TurnRole::from_text("oracle"), TurnRole::Unknown);
}
