use oxyn_core::{ConnectionId, ObjectSection, QueryLanguage};

use super::*;
use crate::documents::Document;

fn atelier() -> (Store, WorkspaceId) {
    let store = Store::open_in_memory().expect("store en mémoire");
    let workspace = store.workspaces().create("atelier").expect("workspace").id;
    (store, workspace)
}

/// Un document enregistré et ouvert, comme une console qu'on a tapée.
fn console(store: &Store, workspace: WorkspaceId) -> DocumentId {
    let document = Document::new(workspace, "console", QueryLanguage::SQL);
    store.documents().save(&document).expect("document");
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE documents SET is_open = 1 WHERE id = ?1",
                params![document.id.to_string()],
            )?;
            Ok(())
        })
        .expect("ouverture");
    document.id
}

fn layout(consoles: Vec<DocumentId>) -> WindowLayout {
    WindowLayout {
        window: WindowId::new(),
        ordinal: 0,
        geometry: WindowGeometry {
            x: Some(40.0),
            y: Some(60.0),
            width: 1280.0,
            height: 820.0,
            maximized: false,
        },
        object_location: None,
        active_document: consoles.first().copied(),
        consoles,
    }
}

/// Termine un lancement, comme un `⌘Q` ordinaire.
fn quit(store: &Store, session: AppSessionId) {
    store.sessions().close(session).expect("fermeture");
}

fn sql<T: rusqlite::types::FromSql>(store: &Store, query: &str) -> T {
    store
        .with_connection(|connection| Ok(connection.query_row(query, [], |row| row.get(0))?))
        .expect("lecture")
}

#[test]
fn une_fenetre_ecrite_revient_au_lancement_suivant() {
    let (store, workspace) = atelier();
    let (first, _) = store.sessions().begin(workspace).expect("lancement");
    let mut written = layout(vec![console(&store, workspace), console(&store, workspace)]);
    written.object_location = Some(ObjectLocation {
        connection: ConnectionId::new(),
        path: "public.users".into(),
        section: ObjectSection::Data,
    });
    store
        .windows()
        .save(workspace, first, &written)
        .expect("écriture");
    quit(&store, first);

    let (second, _) = store.sessions().begin(workspace).expect("relance");
    let adopted = store.windows().adopt(workspace, second).expect("adoption");
    assert_eq!(adopted, vec![written]);
}

#[test]
fn une_reecriture_remplace_les_consoles_et_compte_la_revision() {
    let (store, workspace) = atelier();
    let (session, _) = store.sessions().begin(workspace).expect("lancement");
    let (a, b) = (console(&store, workspace), console(&store, workspace));
    let mut written = layout(vec![a, b]);
    store
        .windows()
        .save(workspace, session, &written)
        .expect("écriture");
    written.consoles = vec![b];
    written.active_document = Some(b);
    store
        .windows()
        .save(workspace, session, &written)
        .expect("réécriture");

    assert_eq!(
        sql::<i64>(&store, "SELECT revision FROM workspace_windows"),
        2
    );
    let adopted = store.windows().adopt(workspace, session).expect("adoption");
    assert_eq!(adopted.first().map(|l| l.consoles.clone()), Some(vec![b]));
}

/// `UNIQUE (document_id)` : une console déplacée quitte la fenêtre d'origine
/// au lieu de faire échouer l'écriture.
#[test]
fn une_console_est_dans_une_seule_fenetre() {
    let (store, workspace) = atelier();
    let (session, _) = store.sessions().begin(workspace).expect("lancement");
    let moved = console(&store, workspace);
    let left = layout(vec![moved]);
    store
        .windows()
        .save(workspace, session, &left)
        .expect("gauche");
    let mut right = layout(vec![moved]);
    right.ordinal = 1;
    store
        .windows()
        .save(workspace, session, &right)
        .expect("droite");

    let adopted = store.windows().adopt(workspace, session).expect("adoption");
    let consoles: Vec<_> = adopted.iter().map(|l| l.consoles.clone()).collect();
    assert_eq!(consoles, vec![vec![], vec![moved]]);
    assert_eq!(adopted.first().and_then(|l| l.active_document), None);
}

#[test]
fn une_console_jamais_enregistree_n_est_pas_ecrite() {
    let (store, workspace) = atelier();
    let (session, _) = store.sessions().begin(workspace).expect("lancement");
    let written = layout(vec![DocumentId::new()]);
    store
        .windows()
        .save(workspace, session, &written)
        .expect("écriture");
    assert_eq!(
        sql::<i64>(&store, "SELECT COUNT(*) FROM workspace_window_consoles"),
        0
    );
}

#[test]
fn retirer_une_fenetre_laisse_ses_documents_dans_la_bibliotheque() {
    let (store, workspace) = atelier();
    let (session, _) = store.sessions().begin(workspace).expect("lancement");
    let kept = console(&store, workspace);
    let written = layout(vec![kept]);
    store
        .windows()
        .save(workspace, session, &written)
        .expect("écriture");
    store
        .windows()
        .remove(workspace, written.window)
        .expect("retrait");

    assert!(
        store
            .windows()
            .adopt(workspace, session)
            .expect("adoption")
            .is_empty()
    );
    assert!(store.documents().get(kept).expect("lecture").is_some());
}

/// ADR-0021 prévoit deux instances sur le même fichier : l'une ne reprend pas
/// les fenêtres de l'autre tant qu'elle bat.
#[test]
fn les_fenetres_d_une_instance_vivante_restent_a_elle() {
    let (store, workspace) = atelier();
    let (alive, _) = store
        .sessions()
        .begin(workspace)
        .expect("première instance");
    store
        .windows()
        .save(workspace, alive, &layout(Vec::new()))
        .expect("écriture");

    let (other, _) = store.sessions().begin(workspace).expect("seconde instance");
    assert!(
        store
            .windows()
            .adopt(workspace, other)
            .expect("adoption")
            .is_empty()
    );

    store
        .mark_session_stale_for_tests(alive)
        .expect("vieillissement");
    assert_eq!(
        store
            .windows()
            .adopt(workspace, other)
            .expect("adoption")
            .len(),
        1
    );
    // Réécrite au nom de l'adoptante : la première, si elle revenait, ne la
    // reprendrait plus.
    assert_eq!(
        sql::<String>(&store, "SELECT app_session_id FROM workspace_windows"),
        other.to_string()
    );
}

#[test]
fn au_dela_de_seize_fenetres_le_surplus_est_oublie() {
    let (store, workspace) = atelier();
    let (session, _) = store.sessions().begin(workspace).expect("lancement");
    for ordinal in 0..20 {
        let mut written = layout(Vec::new());
        written.ordinal = ordinal;
        store
            .windows()
            .save(workspace, session, &written)
            .expect("écriture");
    }
    quit(&store, session);
    let (next, _) = store.sessions().begin(workspace).expect("relance");
    let adopted = store.windows().adopt(workspace, next).expect("adoption");
    assert_eq!(adopted.len(), WindowLayout::MAX_WINDOWS);
    assert_eq!(adopted.last().map(|l| l.ordinal), Some(15));
    assert_eq!(
        sql::<i64>(&store, "SELECT COUNT(*) FROM workspace_windows"),
        16
    );
}

/// Le fichier s'édite à la main : ce qu'il porte d'aberrant ne franchit pas la
/// lecture.
#[test]
fn une_ligne_hostile_est_ramenee_ou_ignoree() {
    let (store, workspace) = atelier();
    let (session, _) = store.sessions().begin(workspace).expect("lancement");
    let active = console(&store, workspace);
    let written = layout(vec![active]);
    store
        .windows()
        .save(workspace, session, &written)
        .expect("écriture");
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE workspace_windows SET x = 1e300, y = NULL, width = -4, height = 9e99,
                     ordinal = -3, object_location = '{\"pas\": \"un emplacement\"}'",
                [],
            )?;
            connection.execute(
                "INSERT INTO workspace_windows (id, workspace_id, app_session_id, ordinal,
                     width, height, maximized, revision, updated_at)
                 VALUES ('pas-un-uuid', ?1, 'personne', 1, 800, 600, 0, 1, 'hier')",
                params![workspace.to_string()],
            )?;
            Ok(())
        })
        .expect("altération");

    let adopted = store.windows().adopt(workspace, session).expect("adoption");
    let expected = WindowLayout {
        ordinal: 0,
        geometry: WindowGeometry {
            x: None,
            y: None,
            width: 0.0,
            height: WindowGeometry::MAX_EXTENT,
            maximized: false,
        },
        object_location: None,
        ..written
    };
    assert_eq!(adopted, vec![expected]);
    assert_eq!(
        sql::<i64>(&store, "SELECT COUNT(*) FROM workspace_windows"),
        1
    );
}

#[test]
fn une_console_fermee_ne_revient_pas() {
    let (store, workspace) = atelier();
    let (session, _) = store.sessions().begin(workspace).expect("lancement");
    let closed = console(&store, workspace);
    store
        .windows()
        .save(workspace, session, &layout(vec![closed]))
        .expect("écriture");
    store.documents().delete(closed).expect("suppression");

    let adopted = store.windows().adopt(workspace, session).expect("adoption");
    assert_eq!(adopted.first().map(|l| l.consoles.len()), Some(0));
    assert_eq!(adopted.first().and_then(|l| l.active_document), None);
}
