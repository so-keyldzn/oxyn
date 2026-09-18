//! Ce que l'arbre d'une conversation doit tenir.
//!
//! Quatre garanties, et chacune est silencieuse si elle se perd : la structure
//! ne peut pas boucler ni changer de conversation, un échange qui a reçu un
//! échantillon ne garde jamais de réponse — **ni en base, ni dans les octets du
//! fichier** —, une issue illisible ne se relit jamais comme une affirmation, et
//! les lignes de la migration 10 restent lisibles.

use super::*;
use crate::conversations::{Conversation, TurnRecord, TurnRole};
use crate::{Store, StoreError};
use oxyn_core::{ConnectionConfig, ConnectionId, DriverId, WorkspaceId};

/// Un magasin migré, un workspace, une connexion.
fn decor() -> (Store, WorkspaceId, ConnectionId) {
    let store = Store::open_in_memory().expect("ouverture");
    let workspace = store.workspaces().create("atelier").expect("workspace").id;
    let connexion = ConnectionConfig::new("base client", DriverId::postgres());
    store
        .connections()
        .save(workspace, &connexion)
        .expect("connexion");
    (store, workspace, connexion.id)
}

/// La destination courante.
fn fournisseur() -> Destination {
    Destination::provider(
        ProviderId::new("anthropic-1a2b3c4d").expect("identifiant"),
        "Anthropic",
        "un-modele",
    )
}

/// Un fil enregistré.
fn fil(store: &Store, workspace: WorkspaceId, connexion: ConnectionId) -> ConversationId {
    let conversation = Conversation::new(workspace, fournisseur(), "Doublons")
        .on_connection(connexion, "base client");
    let id = conversation.id;
    store.conversations().save(&conversation).expect("fil");
    id
}

/// Ajoute un échange et rend son nœud.
fn echange(store: &Store, id: ConversationId, parent: Option<u32>, question: &str) -> u32 {
    store
        .conversations()
        .append_exchange(
            id,
            &ExchangeRecord::new(parent, PrivacyTier::Metadata, question, fournisseur()),
        )
        .expect("écriture")
        .expect("le fil existe")
}

/// Exécute du SQL hors de l'API, comme le ferait `sqlite3`.
fn sql(store: &Store, requete: &str) -> crate::error::Result<usize> {
    store.with_connection(|conn| Ok(conn.execute(requete, [])?))
}

// --- Structure --------------------------------------------------------------

/// Une régénération crée une sœur, une relance un enfant, et la branche relue
/// est le chemin de la racine à la feuille.
#[test]
fn l_arbre_porte_les_versions_et_la_branche_les_relit() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);

    let racine = echange(&store, id, None, "première question");
    let suite = echange(&store, id, Some(racine), "et ensuite ?");
    let autre_suite = echange(&store, id, Some(racine), "et ensuite ? (reformulée)");

    let branche = store
        .conversations()
        .branch_page(id, autre_suite, 0, MAX_EXCHANGE_PAGE)
        .expect("branche");
    let noeuds: Vec<u32> = branche.exchanges.iter().map(|e| e.node).collect();
    assert_eq!(noeuds, vec![racine, autre_suite], "racine d'abord");
    assert_eq!(branche.next, None);
    assert_eq!(branche.exchanges[0].question, "première question");
    assert_eq!(branche.exchanges[1].parent, Some(racine));

    let versions = store.conversations().versions(id, suite).expect("versions");
    let noeuds: Vec<u32> = versions.iter().map(|v| v.node).collect();
    assert_eq!(noeuds, vec![suite, autre_suite], "soi et ses sœurs");
    assert_eq!(
        store
            .conversations()
            .versions(id, racine)
            .expect("versions")
            .len(),
        1,
        "une racine n'a de sœurs que les autres racines"
    );
}

/// Un parent qui n'appartient pas à la conversation est refusé, par l'API comme
/// par le fichier.
#[test]
fn un_parent_d_une_autre_conversation_est_refuse() {
    let (store, workspace, connexion) = decor();
    let premier = fil(&store, workspace, connexion);
    let second = fil(&store, workspace, connexion);
    // Le premier fil va plus loin que le second : c'est ce qui donne un numéro
    // de nœud que le second ne porte pas. Sans cela, `parent = 0` désignerait
    // légitimement le nœud 0 du second, et la clé étrangère n'aurait rien à
    // refuser.
    let mut ailleurs = echange(&store, premier, None, "chez le premier");
    for _ in 0..2 {
        ailleurs = echange(&store, premier, Some(ailleurs), "encore chez le premier");
    }
    echange(&store, second, None, "chez le second");

    let erreur = store
        .conversations()
        .append_exchange(
            second,
            &ExchangeRecord::new(
                Some(ailleurs),
                PrivacyTier::Metadata,
                "question",
                fournisseur(),
            ),
        )
        .expect_err("le parent n'est pas dans cette conversation");
    assert!(
        matches!(erreur, StoreError::Corrupted { field, .. } if field.ends_with(".parent")),
        "{erreur:?}"
    );

    // Et la clé étrangère composite refuse la même chose écrite en SQL. Le
    // nœud inséré est plus récent que son parent, donc le `CHECK(parent < node)`
    // le laisse passer : seule la clé étrangère peut le refuser ici.
    let refus = store.with_connection(|conn| {
        Ok(conn.execute(
            "INSERT INTO ai_conversation_nodes
                 (conversation_id, node, parent, created_at, privacy_tier, question,
                  destination_kind, destination_label)
             VALUES (?1, ?2, ?3, ?4, 'metadata', 'q', 'provider', 'Anthropic')",
            rusqlite::params![
                second.to_string(),
                i64::from(ailleurs) + 1,
                i64::from(ailleurs),
                Utc::now()
            ],
        )?)
    });
    assert!(refus.is_err(), "la clé étrangère composite doit refuser");
}

/// Aucun cycle n'est exprimable : un parent est toujours plus ancien, et la
/// place d'un nœud ne change jamais.
#[test]
fn aucun_cycle_n_est_exprimable() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    let racine = echange(&store, id, None, "racine");
    let enfant = echange(&store, id, Some(racine), "enfant");
    let petit = echange(&store, id, Some(enfant), "petit-enfant");

    for (cas, requete) in [
        // Les deux premiers cas sont ceux que seul le déclencheur peut
        // refuser : le parent visé est plus ancien et il existe, donc le
        // `CHECK` et la clé étrangère les laissent passer tous les deux.
        (
            "remonter un nœud sous un parent valide",
            format!(
                "UPDATE ai_conversation_nodes SET parent = {racine}
                  WHERE conversation_id = '{id}' AND node = {petit}"
            ),
        ),
        (
            "renuméroter un nœud",
            format!(
                "UPDATE ai_conversation_nodes SET node = 42
                  WHERE conversation_id = '{id}' AND node = {petit}"
            ),
        ),
        (
            "un nœud son propre parent",
            format!(
                "INSERT INTO ai_conversation_nodes
                     (conversation_id, node, parent, created_at, privacy_tier, question,
                      destination_kind, destination_label)
                 VALUES ('{id}', 9, 9, '2026-09-17 00:00:00+00:00', 'metadata', 'q',
                         'provider', 'A')"
            ),
        ),
        (
            "un parent plus récent que son enfant",
            format!(
                "INSERT INTO ai_conversation_nodes
                     (conversation_id, node, parent, created_at, privacy_tier, question,
                      destination_kind, destination_label)
                 VALUES ('{id}', 9, 10, '2026-09-17 00:00:00+00:00', 'metadata', 'q',
                         'provider', 'A')"
            ),
        ),
        (
            "reparenter la racine sous son enfant",
            format!(
                "UPDATE ai_conversation_nodes SET parent = {enfant}
                  WHERE conversation_id = '{id}' AND node = {racine}"
            ),
        ),
        (
            "déplacer un nœud dans une autre conversation",
            format!(
                "UPDATE ai_conversation_nodes SET conversation_id = 'ailleurs'
                  WHERE conversation_id = '{id}' AND node = {enfant}"
            ),
        ),
    ] {
        assert!(sql(&store, &requete).is_err(), "{cas} doit être refusé");
    }
}

/// Le nombre d'échanges est borné, dans le code et dans le fichier.
#[test]
fn le_nombre_d_echanges_est_borne() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    store
        .with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            for node in 0..MAX_EXCHANGES_PER_CONVERSATION {
                tx.execute(
                    "INSERT INTO ai_conversation_nodes
                         (conversation_id, node, created_at, privacy_tier, question,
                          destination_kind, destination_label)
                     VALUES (?1, ?2, ?3, 'metadata', 'q', 'provider', 'A')",
                    rusqlite::params![id.to_string(), i64::from(node), Utc::now()],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .expect("un fil plein");

    let erreur = store
        .conversations()
        .append_exchange(
            id,
            &ExchangeRecord::new(None, PrivacyTier::Metadata, "de trop", fournisseur()),
        )
        .expect_err("le fil est plein");
    assert!(
        matches!(erreur, StoreError::TooLarge { field, .. } if field.ends_with(".node")),
        "{erreur:?}"
    );
    let refus = store.with_connection(|conn| {
        Ok(conn.execute(
            "INSERT INTO ai_conversation_nodes
                 (conversation_id, node, created_at, privacy_tier, question,
                  destination_kind, destination_label)
             VALUES (?1, ?2, ?3, 'metadata', 'q', 'provider', 'A')",
            rusqlite::params![
                id.to_string(),
                i64::from(MAX_EXCHANGES_PER_CONVERSATION),
                Utc::now()
            ],
        )?)
    });
    assert!(refus.is_err(), "le fichier borne le nombre d'échanges");
}

// --- L'échantillon retenu ---------------------------------------------------

/// Un échange qui a reçu un échantillon ne garde que sa question, ses compteurs
/// et son issue — refus par l'API **et** par le fichier.
#[test]
fn un_echange_retenu_refuse_toute_reponse() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    let node = echange(&store, id, None, "montre-moi cinq lignes");

    assert!(
        store
            .conversations()
            .withhold_sample(
                id,
                node,
                WithheldSample {
                    rows: 5,
                    columns: 2
                }
            )
            .expect("marquage")
    );

    let erreur = store
        .conversations()
        .append(
            id,
            &TurnRecord::new(TurnRole::Assistant, PrivacyTier::Sampled, "la réponse")
                .in_exchange(node),
        )
        .expect_err("l'API doit refuser");
    assert!(
        matches!(erreur, StoreError::SampleWithheld { node: refuse } if refuse == node),
        "{erreur:?}"
    );

    let refus = store.with_connection(|conn| {
        Ok(conn.execute(
            "INSERT INTO ai_conversation_turns
                 (conversation_id, ordinal, ts, role, privacy_tier, text, node)
             VALUES (?1, 0, ?2, 'assistant', 'sampled', 'la réponse', ?3)",
            rusqlite::params![id.to_string(), Utc::now(), i64::from(node)],
        )?)
    });
    assert!(refus.is_err(), "le fichier doit refuser aussi");

    // La question, les compteurs et l'issue restent.
    store
        .conversations()
        .finish_exchange(id, node, ExchangeOutcome::Answered(AnswerEnding::Answered))
        .expect("issue");
    let branche = store
        .conversations()
        .branch_page(id, node, 0, MAX_EXCHANGE_PAGE)
        .expect("branche");
    let retenu = &branche.exchanges[0];
    assert_eq!(retenu.question, "montre-moi cinq lignes");
    assert_eq!(
        retenu.sample,
        Some(WithheldSample {
            rows: 5,
            columns: 2
        })
    );
    assert_eq!(
        retenu.outcome,
        Some(ExchangeOutcome::Answered(AnswerEnding::Answered))
    );
}

/// Le marqueur ne se retire pas, et les compteurs sont bornés.
#[test]
fn le_marqueur_ne_se_retire_pas() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    let node = echange(&store, id, None, "question");
    store
        .conversations()
        .withhold_sample(
            id,
            node,
            WithheldSample {
                rows: 5,
                columns: 2,
            },
        )
        .expect("marquage");

    // Les compteurs partent avec le marqueur : sans cela, c'est le `CHECK` de
    // cohérence entre les deux qui refuserait, et le déclencheur ne serait
    // jamais éprouvé.
    assert!(
        sql(
            &store,
            &format!(
                "UPDATE ai_conversation_nodes
                    SET sample_withheld = 0, sample_rows = NULL, sample_columns = NULL
                  WHERE conversation_id = '{id}' AND node = {node}"
            )
        )
        .is_err(),
        "un échantillon retenu le reste"
    );
    for hors_bornes in [
        WithheldSample {
            rows: MAX_SAMPLE_ROWS + 1,
            columns: 1,
        },
        WithheldSample {
            rows: 1,
            columns: MAX_SAMPLE_COLUMNS + 1,
        },
    ] {
        assert!(
            store
                .conversations()
                .withhold_sample(id, node, hors_bornes)
                .is_err()
        );
    }
    assert!(
        !store
            .conversations()
            .withhold_sample(
                id,
                250,
                WithheldSample {
                    rows: 1,
                    columns: 1
                }
            )
            .expect("nœud absent"),
        "un échange qui n'existe pas rend `false`"
    );
}

/// **Les octets du fichier** ne contiennent plus la réponse d'un échange retenu.
///
/// C'est la seule forme qui prouve la promesse : effacer une ligne ne suffit
/// pas, SQLite laisse son texte dans ses pages libres, et le journal d'écriture
/// en garde une copie. `secure_delete` et la troncature du journal ferment les
/// deux.
#[test]
fn le_fichier_ne_contient_plus_la_reponse_retenue() {
    const TEMOIN: &str = "reponse-temoin-9f3a7c21-alice@example.test";
    let racine = tempfile::tempdir().expect("répertoire temporaire");
    let chemin = racine.path().join("oxyn.sqlite3");
    let store = Store::open_at(&chemin).expect("ouverture");
    let workspace = store.workspaces().create("atelier").expect("workspace").id;
    let config = ConnectionConfig::new("base client", DriverId::postgres());
    store
        .connections()
        .save(workspace, &config)
        .expect("connexion");
    let id = fil(&store, workspace, config.id);
    let node = echange(&store, id, None, "montre-moi cinq lignes");

    // La réponse est écrite **avant** que l'échantillon ne soit retenu : c'est
    // l'ordre qui arrive quand l'approbation tombe au milieu d'un tour.
    store
        .conversations()
        .append(
            id,
            &TurnRecord::new(TurnRole::Assistant, PrivacyTier::Sampled, TEMOIN).in_exchange(node),
        )
        .expect("réponse écrite");
    assert!(
        contient(&octets_du_fichier(&chemin), TEMOIN.as_bytes()),
        "le test ne prouve rien si la réponse n'a pas atteint le disque"
    );

    store
        .conversations()
        .withhold_sample(
            id,
            node,
            WithheldSample {
                rows: 5,
                columns: 2,
            },
        )
        .expect("marquage");

    assert!(
        !contient(&octets_du_fichier(&chemin), TEMOIN.as_bytes()),
        "le texte de la réponse est encore dans les octets du fichier"
    );
    assert!(
        store
            .conversations()
            .transcript_page(id, None, 16)
            .expect("relecture")
            .turns
            .is_empty(),
        "et la ligne est partie"
    );
}

/// Le témoin apparaît-il quelque part dans ces octets ?
///
/// `Vec::contains` compare **un** octet : c'est une sous-tranche qu'on cherche
/// ici, donc une fenêtre glissante. Un témoin vide répondrait toujours vrai et
/// ne prouverait rien.
fn contient(octets: &[u8], temoin: &[u8]) -> bool {
    assert!(!temoin.is_empty(), "un témoin vide ne prouve rien");
    octets
        .windows(temoin.len())
        .any(|fenetre| fenetre == temoin)
}

/// Les octets du fichier et de son journal d'écriture.
fn octets_du_fichier(chemin: &std::path::Path) -> Vec<u8> {
    let mut octets = std::fs::read(chemin).expect("base");
    for suffixe in ["-wal", "-shm"] {
        let voisin = chemin.with_extension(format!("sqlite3{suffixe}"));
        if let Ok(mut extra) = std::fs::read(&voisin) {
            octets.append(&mut extra);
        }
    }
    octets
}

// --- Issue ------------------------------------------------------------------

/// Les trois formes d'issue survivent à l'aller-retour.
#[test]
fn les_issues_survivent_a_l_aller_retour() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    for issue in [
        ExchangeOutcome::Answered(AnswerEnding::Answered),
        ExchangeOutcome::Answered(AnswerEnding::Truncated),
        ExchangeOutcome::Answered(AnswerEnding::TurnLimit),
        ExchangeOutcome::Answered(AnswerEnding::AgentLimit),
        ExchangeOutcome::Answered(AnswerEnding::Paused),
        ExchangeOutcome::Answered(AnswerEnding::Refused),
        ExchangeOutcome::Failed {
            kind: FailureKind::Provider,
            retryable: true,
        },
        ExchangeOutcome::Failed {
            kind: FailureKind::AgentSignIn,
            retryable: false,
        },
        ExchangeOutcome::Cancelled,
    ] {
        let node = echange(&store, id, None, "question");
        assert!(
            store
                .conversations()
                .finish_exchange(id, node, issue)
                .expect("issue")
        );
        let branche = store
            .conversations()
            .branch_page(id, node, 0, 1)
            .expect("branche");
        assert_eq!(branche.exchanges[0].outcome, Some(issue), "{issue:?}");
    }
    let sans_issue = echange(&store, id, None, "en cours");
    let branche = store
        .conversations()
        .branch_page(id, sans_issue, 0, 1)
        .expect("branche");
    assert_eq!(branche.exchanges[0].outcome, None, "rien n'est affirmé");
}

/// Écrire une ignorance est refusé ; la relire est tolérée, et jamais vers une
/// affirmation.
#[test]
fn l_ignorance_se_relit_mais_ne_s_ecrit_pas() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    let node = echange(&store, id, None, "question");

    for refus in [
        ExchangeOutcome::Unknown,
        ExchangeOutcome::Answered(AnswerEnding::Unknown),
        ExchangeOutcome::Failed {
            kind: FailureKind::Unknown,
            retryable: true,
        },
    ] {
        assert!(
            store
                .conversations()
                .finish_exchange(id, node, refus)
                .is_err(),
            "{refus:?} ne s'écrit pas"
        );
    }

    for (cas, colonnes, attendu) in [
        (
            "une issue inconnue",
            "outcome = 'vaporise', outcome_detail = NULL, retryable = NULL",
            ExchangeOutcome::Unknown,
        ),
        (
            "une fin inconnue",
            "outcome = 'answered', outcome_detail = 'vaporise', retryable = NULL",
            ExchangeOutcome::Answered(AnswerEnding::Unknown),
        ),
        (
            "une catégorie inconnue",
            "outcome = 'failed', outcome_detail = 'vaporise', retryable = NULL",
            ExchangeOutcome::Failed {
                kind: FailureKind::Unknown,
                retryable: false,
            },
        ),
        (
            "une reprise illisible",
            "outcome = 'failed', outcome_detail = 'provider', retryable = NULL",
            ExchangeOutcome::Failed {
                kind: FailureKind::Provider,
                retryable: false,
            },
        ),
    ] {
        sql(
            &store,
            &format!(
                "UPDATE ai_conversation_nodes SET {colonnes}
                  WHERE conversation_id = '{id}' AND node = {node}"
            ),
        )
        .expect("ligne écrite hors d'Oxyn");
        let branche = store
            .conversations()
            .branch_page(id, node, 0, 1)
            .expect("branche");
        assert_eq!(branche.exchanges[0].outcome, Some(attendu), "{cas}");
    }
}

// --- Version sélectionnée ---------------------------------------------------

/// La feuille sélectionnée doit appartenir à la conversation.
#[test]
fn la_feuille_selectionnee_appartient_a_la_conversation() {
    let (store, workspace, connexion) = decor();
    let premier = fil(&store, workspace, connexion);
    let second = fil(&store, workspace, connexion);
    let ici = echange(&store, second, None, "chez le second");
    // Deux échanges chez le premier : les nœuds sont numérotés par conversation,
    // donc un numéro du premier ne désigne rien chez le second que s'il le
    // dépasse. Sans le second échange, `ailleurs` vaudrait `ici` et le test
    // passerait sans rien vérifier.
    let _ = echange(&store, premier, None, "chez le premier");
    let ailleurs = echange(&store, premier, None, "et sa suite");
    assert_ne!(
        ailleurs, ici,
        "le test doit viser un numéro absent du second"
    );

    assert!(store.conversations().select(second, Some(ici)).expect("ok"));
    assert_eq!(
        store
            .conversations()
            .get(second)
            .expect("relecture")
            .expect("fil")
            .selected,
        Some(ici)
    );

    let erreur = store
        .conversations()
        .select(second, Some(ailleurs))
        .expect_err("feuille d'une autre conversation");
    assert!(
        matches!(erreur, StoreError::Corrupted { field, .. } if field.ends_with(".selected_node")),
        "{erreur:?}"
    );
    assert!(
        sql(
            &store,
            &format!(
                "UPDATE ai_conversations SET selected_node = {ailleurs} WHERE id = '{second}'"
            )
        )
        .is_err(),
        "le fichier refuse la même chose"
    );

    assert!(store.conversations().select(second, None).expect("effacée"));
    assert_eq!(
        store
            .conversations()
            .get(second)
            .expect("relecture")
            .expect("fil")
            .selected,
        None
    );
}

// --- Pagination et bornes ---------------------------------------------------

/// La branche se lit par pages bornées, sans trou ni doublon.
#[test]
fn la_branche_se_lit_par_pages_bornees() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    let mut feuille = echange(&store, id, None, "q0");
    let mut attendus = vec![feuille];
    for index in 1..20 {
        feuille = echange(&store, id, Some(feuille), &format!("q{index}"));
        attendus.push(feuille);
    }

    let page = store
        .conversations()
        .branch_page(id, feuille, 0, u16::MAX)
        .expect("page");
    assert_eq!(page.exchanges.len(), usize::from(MAX_EXCHANGE_PAGE));
    assert_eq!(page.next, Some(u32::from(MAX_EXCHANGE_PAGE)));

    let mut lus = Vec::new();
    let mut depart = Some(0);
    while let Some(start) = depart {
        let page = store
            .conversations()
            .branch_page(id, feuille, start, 3)
            .expect("page");
        lus.extend(page.exchanges.iter().map(|e| e.node));
        depart = page.next;
    }
    assert_eq!(
        lus, attendus,
        "toute la branche, dans l'ordre, une seule fois"
    );

    assert_eq!(
        store
            .conversations()
            .branch_page(id, feuille, 0, 0)
            .expect("page")
            .exchanges
            .len(),
        1,
        "`limit` nul vaut un"
    );
    assert!(
        store
            .conversations()
            .branch_page(id, 250, 0, 4)
            .expect("feuille inconnue")
            .exchanges
            .is_empty()
    );
}

/// La taille d'un `Exchange` est celle que la borne de page suppose.
///
/// Si elle grossit, le pire cas de [`MAX_EXCHANGE_PAGE`] est faux et doit être
/// recalculé — c'est tout l'objet de ce test.
#[test]
fn la_taille_d_un_echange_est_celle_que_la_borne_suppose() {
    assert_eq!(
        std::mem::size_of::<Exchange>(),
        EXCHANGE_LAYOUT_BYTES,
        "la disposition a changé : recalculer le pire cas de MAX_EXCHANGE_PAGE"
    );
    assert!(std::mem::size_of::<Version>() <= 64);
}

// --- Migration --------------------------------------------------------------

/// Les lignes de la migration 10 se relisent sans perte : pas de nœud, pas de
/// feuille sélectionnée, et la transcription linéaire intacte.
#[test]
fn les_lignes_de_la_migration_10_se_relisent_sans_perte() {
    let racine = tempfile::tempdir().expect("répertoire temporaire");
    let chemin = racine.path().join("oxyn.sqlite3");
    let id = ConversationId::new();
    {
        let conn = crate::schema::file_at_version(&chemin, 10);
        let atelier = WorkspaceId::new();
        let quand = Utc::now().format("%F %T%.f%:z");
        conn.execute_batch(&format!(
            "INSERT INTO workspaces VALUES ('{atelier}', 'atelier', '{quand}', '{quand}');
             INSERT INTO ai_conversations
                 (id, workspace_id, destination_kind, destination_label, title,
                  created_at, updated_at)
             VALUES ('{id}', '{atelier}', 'provider', 'Anthropic', 'Fil', '{quand}', '{quand}');
             INSERT INTO ai_conversation_turns
                 (conversation_id, ordinal, ts, role, privacy_tier, text)
             VALUES ('{id}', 0, '{quand}', 'user', 'metadata', 'une question'),
                    ('{id}', 1, '{quand}', 'assistant', 'metadata', 'une réponse');"
        ))
        .expect("fil d'une version antérieure");
    }

    let store = Store::open_at(&chemin).expect("l'ouverture ne doit pas échouer");
    let page = store
        .conversations()
        .transcript_page(id, None, 16)
        .expect("relecture");
    assert_eq!(page.turns.len(), 2, "aucune ligne perdue");
    assert_eq!(page.turns[1].record.text, "une réponse");
    assert!(
        page.turns.iter().all(|tour| tour.record.node.is_none()),
        "une ligne d'avant l'arbre n'appartient à aucun échange"
    );
    let fil = store
        .conversations()
        .get(id)
        .expect("relecture")
        .expect("fil");
    assert_eq!(fil.selected, None);
    assert!(
        store
            .conversations()
            .branch_page(id, 0, 0, 4)
            .expect("branche")
            .exchanges
            .is_empty(),
        "une conversation sans nœud n'a pas de branche"
    );
}
