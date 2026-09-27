//! What a conversation's tree must hold.
//!
//! Four guarantees, each silent if lost: the structure can neither loop nor
//! change conversation, an exchange that received a sample never keeps an
//! answer — **neither in the database, nor in the file's bytes** —, an
//! unreadable outcome never reads back as an assertion, and migration 10's
//! rows stay readable.

use super::*;
use crate::conversations::{Conversation, TurnRecord, TurnRole};
use crate::{Store, StoreError};
use oxyn_core::{ConnectionConfig, ConnectionId, DriverId, WorkspaceId};

/// A migrated store, a workspace, a connection.
fn decor() -> (Store, WorkspaceId, ConnectionId) {
    let store = Store::open_in_memory().expect("open");
    let workspace = store.workspaces().create("atelier").expect("workspace").id;
    let connexion = ConnectionConfig::new("base client", DriverId::postgres());
    store
        .connections()
        .save(workspace, &connexion)
        .expect("connection");
    (store, workspace, connexion.id)
}

/// The common destination.
fn fournisseur() -> Destination {
    Destination::provider(
        ProviderId::new("anthropic-1a2b3c4d").expect("identifier"),
        "Anthropic",
        "un-modele",
    )
}

/// A saved thread.
fn fil(store: &Store, workspace: WorkspaceId, connexion: ConnectionId) -> ConversationId {
    let conversation = Conversation::new(workspace, fournisseur(), "Doublons")
        .on_connection(connexion, "base client");
    let id = conversation.id;
    store.conversations().save(&conversation).expect("thread");
    id
}

/// Adds an exchange and returns its node.
fn echange(store: &Store, id: ConversationId, parent: Option<u32>, question: &str) -> u32 {
    store
        .conversations()
        .append_exchange(
            id,
            &ExchangeRecord::new(parent, PrivacyTier::Metadata, question, fournisseur()),
        )
        .expect("write")
        .expect("the thread exists")
}

/// Runs SQL outside the API, as `sqlite3` would.
fn sql(store: &Store, requete: &str) -> crate::error::Result<usize> {
    store.with_connection(|conn| Ok(conn.execute(requete, [])?))
}

// --- Structure --------------------------------------------------------------

/// A regeneration creates a sibling, a follow-up a child, and the branch read
/// back is the path from the root to the leaf.
#[test]
fn the_tree_carries_versions_and_the_branch_reads_them_back() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);

    let racine = echange(&store, id, None, "première question");
    let suite = echange(&store, id, Some(racine), "et ensuite ?");
    let autre_suite = echange(&store, id, Some(racine), "et ensuite ? (reformulée)");

    let branche = store
        .conversations()
        .branch_page(id, autre_suite, 0, MAX_EXCHANGE_PAGE)
        .expect("branch");
    let noeuds: Vec<u32> = branche.exchanges.iter().map(|e| e.node).collect();
    assert_eq!(noeuds, vec![racine, autre_suite], "root first");
    assert_eq!(branche.next, None);
    assert_eq!(branche.exchanges[0].question, "première question");
    assert_eq!(branche.exchanges[1].parent, Some(racine));

    let versions = store.conversations().versions(id, suite).expect("versions");
    let noeuds: Vec<u32> = versions.iter().map(|v| v.node).collect();
    assert_eq!(noeuds, vec![suite, autre_suite], "itself and its siblings");
    assert_eq!(
        store
            .conversations()
            .versions(id, racine)
            .expect("versions")
            .len(),
        1,
        "a root only has the other roots as siblings"
    );
}

/// A parent that does not belong to the conversation is refused, by the API as
/// by the file.
#[test]
fn a_parent_from_another_conversation_is_refused() {
    let (store, workspace, connexion) = decor();
    let premier = fil(&store, workspace, connexion);
    let second = fil(&store, workspace, connexion);
    // The first thread goes further than the second: that is what yields a
    // node number the second does not carry. Without it, `parent = 0` would
    // legitimately designate the second's node 0, and the foreign key would
    // have nothing to refuse.
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
        .expect_err("the parent is not in this conversation");
    assert!(
        matches!(erreur, StoreError::Corrupted { field, .. } if field.ends_with(".parent")),
        "{erreur:?}"
    );

    // And the composite foreign key refuses the same thing written in SQL.
    // The inserted node is newer than its parent, so `CHECK(parent < node)`
    // lets it through: only the foreign key can refuse it here.
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
    assert!(refus.is_err(), "the composite foreign key must refuse");
}

/// No cycle can be expressed: a parent is always older, and a node's place
/// never changes.
#[test]
fn no_cycle_can_be_expressed() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    let racine = echange(&store, id, None, "racine");
    let enfant = echange(&store, id, Some(racine), "enfant");
    let petit = echange(&store, id, Some(enfant), "petit-enfant");

    for (cas, requete) in [
        // The first two cases are the ones only the trigger can refuse: the
        // targeted parent is older and exists, so the `CHECK` and the foreign
        // key both let them through.
        (
            "move a node up under a valid parent",
            format!(
                "UPDATE ai_conversation_nodes SET parent = {racine}
                  WHERE conversation_id = '{id}' AND node = {petit}"
            ),
        ),
        (
            "renumber a node",
            format!(
                "UPDATE ai_conversation_nodes SET node = 42
                  WHERE conversation_id = '{id}' AND node = {petit}"
            ),
        ),
        (
            "a node its own parent",
            format!(
                "INSERT INTO ai_conversation_nodes
                     (conversation_id, node, parent, created_at, privacy_tier, question,
                      destination_kind, destination_label)
                 VALUES ('{id}', 9, 9, '2026-09-17 00:00:00+00:00', 'metadata', 'q',
                         'provider', 'A')"
            ),
        ),
        (
            "a parent newer than its child",
            format!(
                "INSERT INTO ai_conversation_nodes
                     (conversation_id, node, parent, created_at, privacy_tier, question,
                      destination_kind, destination_label)
                 VALUES ('{id}', 9, 10, '2026-09-17 00:00:00+00:00', 'metadata', 'q',
                         'provider', 'A')"
            ),
        ),
        (
            "reparent the root under its child",
            format!(
                "UPDATE ai_conversation_nodes SET parent = {enfant}
                  WHERE conversation_id = '{id}' AND node = {racine}"
            ),
        ),
        (
            "move a node into another conversation",
            format!(
                "UPDATE ai_conversation_nodes SET conversation_id = 'ailleurs'
                  WHERE conversation_id = '{id}' AND node = {enfant}"
            ),
        ),
    ] {
        assert!(sql(&store, &requete).is_err(), "{cas} must be refused");
    }
}

/// The number of exchanges is bounded, in the code and in the file.
#[test]
fn the_number_of_exchanges_is_bounded() {
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
        .expect("a full thread");

    let erreur = store
        .conversations()
        .append_exchange(
            id,
            &ExchangeRecord::new(None, PrivacyTier::Metadata, "de trop", fournisseur()),
        )
        .expect_err("the thread is full");
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
    assert!(refus.is_err(), "the file bounds the number of exchanges");
}

// --- The withheld sample -----------------------------------------------------

/// An exchange that received a sample only keeps its question, its counters
/// and its outcome — refused by the API **and** by the file.
#[test]
fn a_withheld_exchange_refuses_any_answer() {
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
            .expect("marking")
    );

    let erreur = store
        .conversations()
        .append(
            id,
            &TurnRecord::new(TurnRole::Assistant, PrivacyTier::Sampled, "la réponse")
                .in_exchange(node),
        )
        .expect_err("the API must refuse");
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
    assert!(refus.is_err(), "the file must refuse too");

    // The question, the counters and the outcome remain.
    store
        .conversations()
        .finish_exchange(id, node, ExchangeOutcome::Answered(AnswerEnding::Answered))
        .expect("issue");
    let branche = store
        .conversations()
        .branch_page(id, node, 0, MAX_EXCHANGE_PAGE)
        .expect("branch");
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

/// The marker cannot be removed, and the counters are bounded.
#[test]
fn the_marker_cannot_be_removed() {
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
        .expect("marking");

    // The counters go with the marker: otherwise, the consistency `CHECK`
    // between the two would refuse, and the trigger would never be tested.
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
        "a withheld sample stays withheld"
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
            .expect("missing node"),
        "a nonexistent exchange returns `false`"
    );
}

/// **The file's bytes** no longer contain a withheld exchange's answer.
///
/// It is the only form that proves the promise: erasing a row is not enough,
/// SQLite leaves its text in its free pages, and the write-ahead log keeps a
/// copy. `secure_delete` and truncating the log close both.
#[test]
fn the_file_no_longer_contains_the_withheld_answer() {
    const TEMOIN: &str = "reponse-temoin-9f3a7c21-alice@example.test";
    let racine = tempfile::tempdir().expect("temporary directory");
    let chemin = racine.path().join("oxyn.sqlite3");
    let store = Store::open_at(&chemin).expect("open");
    let workspace = store.workspaces().create("atelier").expect("workspace").id;
    let config = ConnectionConfig::new("base client", DriverId::postgres());
    store
        .connections()
        .save(workspace, &config)
        .expect("connection");
    let id = fil(&store, workspace, config.id);
    let node = echange(&store, id, None, "montre-moi cinq lignes");

    // The answer is written **before** the sample is withheld: that is the
    // order that happens when the approval lands in the middle of a turn.
    store
        .conversations()
        .append(
            id,
            &TurnRecord::new(TurnRole::Assistant, PrivacyTier::Sampled, TEMOIN).in_exchange(node),
        )
        .expect("written answer");
    assert!(
        contient(&octets_du_fichier(&chemin), TEMOIN.as_bytes()),
        "the test proves nothing if the answer did not reach the disk"
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
        .expect("marking");

    assert!(
        !contient(&octets_du_fichier(&chemin), TEMOIN.as_bytes()),
        "the answer's text is still in the file's bytes"
    );
    assert!(
        store
            .conversations()
            .transcript_page(id, None, 16)
            .expect("read back")
            .turns
            .is_empty(),
        "and the row is gone"
    );
}

/// Does the witness appear anywhere in these bytes?
///
/// `Vec::contains` compares **one** byte: a subslice is what is searched here,
/// hence a sliding window. An empty witness would always answer true and prove
/// nothing.
fn contient(octets: &[u8], temoin: &[u8]) -> bool {
    assert!(!temoin.is_empty(), "an empty witness proves nothing");
    octets
        .windows(temoin.len())
        .any(|fenetre| fenetre == temoin)
}

/// The bytes of the file and of its write-ahead log.
fn octets_du_fichier(chemin: &std::path::Path) -> Vec<u8> {
    let mut octets = std::fs::read(chemin).expect("database");
    for suffixe in ["-wal", "-shm"] {
        let voisin = chemin.with_extension(format!("sqlite3{suffixe}"));
        if let Ok(mut extra) = std::fs::read(&voisin) {
            octets.append(&mut extra);
        }
    }
    octets
}

// --- Outcome ----------------------------------------------------------------

/// The three outcome forms survive the round trip.
#[test]
fn outcomes_survive_the_round_trip() {
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
            .expect("branch");
        assert_eq!(branche.exchanges[0].outcome, Some(issue), "{issue:?}");
    }
    let sans_issue = echange(&store, id, None, "en cours");
    let branche = store
        .conversations()
        .branch_page(id, sans_issue, 0, 1)
        .expect("branch");
    assert_eq!(branche.exchanges[0].outcome, None, "nothing is asserted");
}

/// Writing an unknown is refused; reading one back is tolerated, and never
/// towards an assertion.
#[test]
fn an_unknown_reads_back_but_is_not_written() {
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
            "{refus:?} is not written"
        );
    }

    for (cas, colonnes, attendu) in [
        (
            "an unknown outcome",
            "outcome = 'vaporise', outcome_detail = NULL, retryable = NULL",
            ExchangeOutcome::Unknown,
        ),
        (
            "an unknown ending",
            "outcome = 'answered', outcome_detail = 'vaporise', retryable = NULL",
            ExchangeOutcome::Answered(AnswerEnding::Unknown),
        ),
        (
            "an unknown category",
            "outcome = 'failed', outcome_detail = 'vaporise', retryable = NULL",
            ExchangeOutcome::Failed {
                kind: FailureKind::Unknown,
                retryable: false,
            },
        ),
        (
            "an unreadable retry",
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
        .expect("row written outside Oxyn");
        let branche = store
            .conversations()
            .branch_page(id, node, 0, 1)
            .expect("branch");
        assert_eq!(branche.exchanges[0].outcome, Some(attendu), "{cas}");
    }
}

// --- Selected version ---------------------------------------------------------

/// The selected leaf must belong to the conversation.
#[test]
fn the_selected_leaf_belongs_to_the_conversation() {
    let (store, workspace, connexion) = decor();
    let premier = fil(&store, workspace, connexion);
    let second = fil(&store, workspace, connexion);
    let ici = echange(&store, second, None, "chez le second");
    // Two exchanges in the first: nodes are numbered per conversation, so a
    // number from the first designates nothing in the second only if it
    // exceeds it. Without the second exchange, `ailleurs` would equal `ici`
    // and the test would pass without checking anything.
    let _ = echange(&store, premier, None, "chez le premier");
    let ailleurs = echange(&store, premier, None, "et sa suite");
    assert_ne!(
        ailleurs, ici,
        "the test must target a number absent from the second"
    );

    assert!(store.conversations().select(second, Some(ici)).expect("ok"));
    assert_eq!(
        store
            .conversations()
            .get(second)
            .expect("read back")
            .expect("thread")
            .selected,
        Some(ici)
    );

    let erreur = store
        .conversations()
        .select(second, Some(ailleurs))
        .expect_err("leaf from another conversation");
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
        "the file refuses the same thing"
    );

    assert!(store.conversations().select(second, None).expect("erased"));
    assert_eq!(
        store
            .conversations()
            .get(second)
            .expect("read back")
            .expect("thread")
            .selected,
        None
    );
}

// --- Pagination and bounds ----------------------------------------------------

/// The branch is read in bounded pages, without gap or duplicate.
#[test]
fn the_branch_is_read_in_bounded_pages() {
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
    assert_eq!(lus, attendus, "the whole branch, in order, once");

    assert_eq!(
        store
            .conversations()
            .branch_page(id, feuille, 0, 0)
            .expect("page")
            .exchanges
            .len(),
        1,
        "a zero `limit` counts as one"
    );
    assert!(
        store
            .conversations()
            .branch_page(id, 250, 0, 4)
            .expect("unknown leaf")
            .exchanges
            .is_empty()
    );
}

/// An `Exchange`'s size is the one the page bound assumes.
///
/// If it grows, [`MAX_EXCHANGE_PAGE`]'s worst case is wrong and must be
/// recomputed — that is the whole point of this test.
#[test]
fn an_exchange_size_is_what_the_bound_assumes() {
    assert_eq!(
        std::mem::size_of::<Exchange>(),
        EXCHANGE_LAYOUT_BYTES,
        "the layout changed: recompute MAX_EXCHANGE_PAGE's worst case"
    );
    assert!(std::mem::size_of::<Version>() <= 64);
}

// --- Migration --------------------------------------------------------------

/// Migration 10's rows read back without loss: no node, no selected leaf, and
/// the linear transcript intact.
#[test]
fn migration_10_rows_read_back_without_loss() {
    let racine = tempfile::tempdir().expect("temporary directory");
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
        .expect("thread from an earlier version");
    }

    let store = Store::open_at(&chemin).expect("opening must not fail");
    let page = store
        .conversations()
        .transcript_page(id, None, 16)
        .expect("read back");
    assert_eq!(page.turns.len(), 2, "no row lost");
    assert_eq!(page.turns[1].record.text, "une réponse");
    assert!(
        page.turns.iter().all(|tour| tour.record.node.is_none()),
        "a row from before the tree belongs to no exchange"
    );
    let fil = store
        .conversations()
        .get(id)
        .expect("read back")
        .expect("thread");
    assert_eq!(fil.selected, None);
    assert!(
        store
            .conversations()
            .branch_page(id, 0, 0, 4)
            .expect("branch")
            .exchanges
            .is_empty(),
        "a conversation without nodes has no branch"
    );
}

// --- Mentions ---------------------------------------------------------------

fn mention_table(nom: &str) -> ExchangeMention {
    ExchangeMention {
        kind: "table".to_owned(),
        label: nom.to_owned(),
        catalog: None,
        namespace: Some("public".to_owned()),
        relation: Some(nom.to_owned()),
        field: None,
        document: None,
    }
}

/// What a question named reads back with it, in order, and the file keeps it
/// as JSON readable without Oxyn (I-11).
#[test]
fn a_question_mentions_read_back_as_open_json() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    // A hostile name stays data: escaped by the JSON, never SQL.
    let hostile = mention_table("\"users\"; DROP TABLE audit; --");
    let record = ExchangeRecord::new(None, PrivacyTier::Metadata, "@orders", fournisseur())
        .with_mentions(vec![mention_table("orders"), hostile.clone()]);
    let noeud = store
        .conversations()
        .append_exchange(id, &record)
        .expect("write")
        .expect("the thread exists");

    let relu = store
        .conversations()
        .branch_page(id, noeud, 0, MAX_EXCHANGE_PAGE)
        .expect("branch");
    let mentions = &relu.exchanges[0].mentions;
    assert_eq!(mentions, &vec![mention_table("orders"), hostile]);

    let brut: String = store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT mentions FROM ai_conversation_nodes WHERE node = ?1",
                [i64::from(noeud)],
                |row| row.get(0),
            )?)
        })
        .expect("column");
    let json: serde_json::Value = serde_json::from_str(&brut).expect("JSON");
    assert_eq!(json[0]["relation"], "orders");
    assert_eq!(json[0]["namespace"], "public");
}

/// A question without a mention, or written before migration 14, reads back
/// without a chip; the file refuses what is not a JSON array, and what
/// exceeds the bound.
#[test]
fn without_mention_nothing_and_the_file_refuses_the_rest() {
    let (store, workspace, connexion) = decor();
    let id = fil(&store, workspace, connexion);
    let noeud = echange(&store, id, None, "sans mention");
    let relu = store
        .conversations()
        .branch_page(id, noeud, 0, MAX_EXCHANGE_PAGE)
        .expect("branch");
    assert!(relu.exchanges[0].mentions.is_empty());

    for invalide in ["'pas du json'", "'{\"kind\":\"table\"}'"] {
        assert!(
            sql(
                &store,
                &format!("UPDATE ai_conversation_nodes SET mentions = {invalide}")
            )
            .is_err(),
            "{invalide} is not written"
        );
    }

    let trop: Vec<ExchangeMention> = (0..=crate::conversations::MAX_EXCHANGE_MENTIONS)
        .map(|i| mention_table(&format!("t{i}")))
        .collect();
    let refus = store.conversations().append_exchange(
        id,
        &ExchangeRecord::new(None, PrivacyTier::Metadata, "trop", fournisseur())
            .with_mentions(trop),
    );
    assert!(
        matches!(refus, Err(StoreError::TooLarge { .. })),
        "{refus:?}"
    );
}
