//! Ce que le journal des sorties IA doit tenir.
//!
//! Quatre garanties, chacune silencieuse si elle se perd : la relecture est
//! fidèle, le journal ne s'efface pas, le fichier refuse ce qui ne tient pas
//! dans ses bornes, et **aucune valeur** n'a d'endroit où se ranger.

use super::*;
use crate::Store;
use oxyn_core::{ConnectionConfig, DriverId, WorkspaceId};

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

/// Une sortie ordinaire : cinq lignes de deux colonnes vers un fournisseur.
fn sortie(connexion: ConnectionId) -> EgressRecord {
    EgressRecord::new(
        connexion,
        "ventes.public.clients",
        vec!["email".into(), "pays".into()],
        5,
        ProviderId::new("anthropic-1a2b3c4d").expect("identifiant"),
        EgressReach::Remote,
    )
}

/// Toutes les entrées d'une connexion, page par page.
fn toutes(store: &Store, connexion: ConnectionId) -> Vec<EgressEntry> {
    let mut entrees = Vec::new();
    let mut avant = None;
    loop {
        let page = store
            .egress()
            .for_connection(connexion, avant, 2)
            .expect("page");
        entrees.extend(page.entries);
        match page.next {
            Some(suivant) => avant = Some(suivant),
            None => return entrees,
        }
    }
}

/// Exécute du SQL hors de l'API, comme le ferait `sqlite3`.
fn sql(store: &Store, requete: &str) -> Result<usize> {
    store.with_connection(|conn| Ok(conn.execute(requete, [])?))
}

// --- Aller-retour -----------------------------------------------------------

/// Une sortie relue est la sortie écrite, champ par champ.
#[test]
fn une_sortie_relue_est_la_sortie_ecrite() {
    let (store, _, connexion) = decor();
    let commande = CommandId::new();
    let fil = ConversationId::new();
    let ecrite = sortie(connexion)
        .read_by(commande)
        .with_model("un-modele")
        .in_conversation(fil, Some(3));

    let id = store.egress().append(&ecrite).expect("écriture");
    let relues = toutes(&store, connexion);
    assert_eq!(relues.len(), 1);
    assert_eq!(relues[0].id, id);
    assert!(relues[0].record == ecrite, "{:?}", relues[0].record);
}

/// Un agent externe n'a pas de modèle, et sa portée est inconnaissable.
#[test]
fn une_sortie_vers_un_agent_externe_se_relit_sans_modele() {
    let (store, _, connexion) = decor();
    let ecrite = EgressRecord::new(
        connexion,
        "clients",
        vec!["id".into()],
        1,
        ProviderId::new("agent-9f3a7c21").expect("identifiant"),
        EgressReach::Unresolved,
    );
    store.egress().append(&ecrite).expect("écriture");
    let relue = &toutes(&store, connexion)[0].record;
    assert_eq!(relue.model, None);
    assert!(relue.reach.leaves_machine(), "inconnu compte pour distant");
}

/// La lecture est paginée, bornée, la plus récente d'abord, et ne mélange pas
/// les connexions.
#[test]
fn la_lecture_est_paginee_et_par_connexion() {
    let (store, workspace, connexion) = decor();
    let autre = ConnectionConfig::new("autre base", DriverId::sqlite());
    store
        .connections()
        .save(workspace, &autre)
        .expect("connexion");
    let mut ids = Vec::new();
    for _ in 0..5 {
        ids.push(store.egress().append(&sortie(connexion)).expect("écriture"));
    }
    store.egress().append(&sortie(autre.id)).expect("écriture");

    let relues: Vec<i64> = toutes(&store, connexion).iter().map(|e| e.id).collect();
    ids.reverse();
    assert_eq!(
        relues, ids,
        "toutes, une seule fois, la plus récente d'abord"
    );

    let page = store
        .egress()
        .for_connection(connexion, None, 0)
        .expect("page");
    assert_eq!(page.entries.len(), 1, "`limit` nul vaut un");
    let page = store
        .egress()
        .for_connection(connexion, None, u16::MAX)
        .expect("page");
    assert_eq!(page.entries.len(), 5);
    assert_eq!(page.next, None, "pas de page vide après la dernière");
}

// --- Rétention : aucune -----------------------------------------------------

/// Le journal ne s'efface pas, ni par `sqlite3`, ni par la suppression de ce
/// qu'il nomme.
///
/// Une sortie qu'on pourrait effacer répondrait « rien n'est sorti » à la seule
/// question pour laquelle elle existe.
#[test]
fn le_journal_des_sorties_ne_s_efface_pas() {
    let (store, workspace, connexion) = decor();
    store.egress().append(&sortie(connexion)).expect("écriture");

    assert!(
        sql(&store, "UPDATE ai_egress SET row_count = 0").is_err(),
        "UPDATE refusé"
    );
    assert!(
        sql(&store, "DELETE FROM ai_egress").is_err(),
        "DELETE refusé"
    );

    store.connections().delete(connexion).expect("suppression");
    store.workspaces().delete(workspace).expect("suppression");
    assert_eq!(
        toutes(&store, connexion).len(),
        1,
        "l'entrée survit à la connexion et au workspace"
    );
}

// --- Bornes -----------------------------------------------------------------

/// L'API refuse ce qui ne tient pas, sans rien écrire et sans citer la valeur.
#[test]
fn l_api_refuse_ce_qui_depasse_ses_bornes() {
    let (store, _, connexion) = decor();
    let temoin = "valeur-temoin-alice@example.test";
    let refus: Vec<(&str, EgressRecord)> = vec![
        ("source vide", {
            let mut r = sortie(connexion);
            r.source.clear();
            r
        }),
        ("source trop longue", {
            let mut r = sortie(connexion);
            r.source = "s".repeat(MAX_SOURCE_BYTES + 1);
            r
        }),
        ("aucune colonne", {
            let mut r = sortie(connexion);
            r.columns.clear();
            r
        }),
        ("trop de colonnes", {
            let mut r = sortie(connexion);
            r.columns = (0..=MAX_COLUMNS).map(|i| format!("c{i}")).collect();
            r
        }),
        ("nom vide", {
            let mut r = sortie(connexion);
            r.columns.push(String::new());
            r
        }),
        ("nom trop long", {
            let mut r = sortie(connexion);
            r.columns.push("n".repeat(MAX_COLUMN_NAME_BYTES + 1));
            r
        }),
        ("ligne collée comme nom", {
            let mut r = sortie(connexion);
            r.columns.push(format!("{temoin}\tFR\n"));
            r
        }),
        ("trop de lignes", {
            let mut r = sortie(connexion);
            r.rows = MAX_ROWS + 1;
            r
        }),
        (
            "modèle trop long",
            sortie(connexion).with_model("m".repeat(MAX_MODEL_BYTES + 1)),
        ),
    ];
    for (cas, record) in refus {
        let erreur = store.egress().append(&record).expect_err(cas);
        assert!(
            !erreur.to_string().contains(temoin),
            "{cas} : le message cite la valeur : {erreur}"
        );
    }
    assert!(
        toutes(&store, connexion).is_empty(),
        "aucun refus n'écrit rien"
    );
}

/// Le fichier tient les mêmes bornes pour un tiers armé de `sqlite3`, et en
/// particulier refuse qu'une valeur se range dans la liste des colonnes.
#[test]
fn le_fichier_refuse_une_valeur_deguisee_en_nom_de_colonne() {
    let (store, _, connexion) = decor();
    let inserer = |columns: &str, rows: i64, source: &str| {
        store.with_connection(|conn| {
            Ok(conn.execute(
                "INSERT INTO ai_egress
                     (ts, connection_id, source, columns, row_count, recipient_id, reach)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'anthropic-1a2b3c4d', 'remote')",
                rusqlite::params![Utc::now(), connexion.to_string(), source, columns, rows],
            )?)
        })
    };

    inserer(r#"["email"]"#, 5, "clients").expect("la forme admise passe");
    let long = "n".repeat(MAX_COLUMN_NAME_BYTES + 1);
    let trop: Vec<String> = (0..=MAX_COLUMNS).map(|i| format!("c{i}")).collect();
    for (cas, columns, rows, source) in [
        ("un nombre", "[42]".to_owned(), 5, "clients"),
        (
            "un objet",
            r#"[{"email":"alice@example.test"}]"#.to_owned(),
            5,
            "clients",
        ),
        (
            "un tableau imbriqué",
            r#"[["alice@example.test"]]"#.to_owned(),
            5,
            "clients",
        ),
        ("un nom vide", r#"[""]"#.to_owned(), 5, "clients"),
        ("un nom trop long", format!(r#"["{long}"]"#), 5, "clients"),
        ("pas un tableau", r#""email""#.to_owned(), 5, "clients"),
        (
            "un objet au lieu d'un tableau",
            r#"{"email":"alice@example.test"}"#.to_owned(),
            5,
            "clients",
        ),
        ("pas du JSON", "email, pays".to_owned(), 5, "clients"),
        ("aucune colonne", "[]".to_owned(), 5, "clients"),
        (
            "trop de colonnes",
            serde_json::to_string(&trop).expect("encodage"),
            5,
            "clients",
        ),
        (
            "trop de lignes",
            r#"["email"]"#.to_owned(),
            i64::from(MAX_ROWS) + 1,
            "clients",
        ),
        ("lignes négatives", r#"["email"]"#.to_owned(), -1, "clients"),
        ("source vide", r#"["email"]"#.to_owned(), 5, ""),
    ] {
        assert!(
            inserer(&columns, rows, source).is_err(),
            "{cas} doit être refusé"
        );
    }
}

/// Les bornes du fichier sont celles du code.
///
/// Une borne qui divergerait ferait refuser au fichier ce que le code croit
/// permis — l'utilisateur perdrait la trace d'une sortie réelle, et la donnée
/// partirait quand même si l'appelant n'arrête pas l'envoi.
#[test]
fn les_bornes_du_fichier_sont_celles_du_code() {
    let (store, _, _) = decor();
    let schema: String = store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT group_concat(sql, char(10)) FROM sqlite_schema WHERE tbl_name = 'ai_egress'",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("schéma");
    for attendu in [
        format!("BETWEEN 1 AND {MAX_SOURCE_BYTES}"),
        format!("BETWEEN 1 AND {MAX_COLUMNS}"),
        format!("NOT BETWEEN 1 AND {MAX_COLUMN_NAME_BYTES}"),
        format!("BETWEEN 0 AND {MAX_ROWS}"),
        format!("<= {MAX_MODEL_BYTES}"),
    ] {
        assert!(
            schema.contains(&attendu),
            "borne absente du fichier : {attendu}"
        );
    }
}

// --- Aucune valeur ----------------------------------------------------------

/// La table n'a que ces colonnes, et aucune ne peut porter une valeur.
///
/// Verrou de schéma : ajouter une colonne — « juste un échantillon pour le
/// contexte » — fait rougir ce test, et oblige à relire la garantie plutôt qu'à
/// la perdre sans bruit.
#[test]
fn la_table_n_a_aucune_colonne_pour_une_valeur() {
    let (store, _, _) = decor();
    let colonnes: Vec<String> = store
        .with_connection(|conn| {
            let mut requete = conn.prepare("SELECT name FROM pragma_table_info('ai_egress')")?;
            let noms = requete.query_map([], |row| row.get(0))?;
            Ok(noms.collect::<rusqlite::Result<Vec<String>>>()?)
        })
        .expect("schéma");
    assert_eq!(
        colonnes,
        [
            "id",
            "ts",
            "connection_id",
            "command_id",
            "source",
            "columns",
            "row_count",
            "recipient_id",
            "model",
            "reach",
            "conversation_id",
            "node",
        ]
    );
}

/// Le `Debug` montre la source et les noms de colonnes, rien d'autre.
#[test]
fn le_debug_montre_la_source_et_les_noms_seulement() {
    let connexion = ConnectionId::new();
    let record = sortie(connexion).with_model("modele-temoin");
    let rendu = format!("{record:?}");
    assert!(rendu.contains("ventes.public.clients"), "{rendu}");
    assert!(rendu.contains("email"), "{rendu}");
    assert!(!rendu.contains(&connexion.to_string()), "{rendu}");
    assert!(!rendu.contains("anthropic-1a2b3c4d"), "{rendu}");
    assert!(!rendu.contains("modele-temoin"), "{rendu}");
}

// --- Migration ----------------------------------------------------------------

/// Un état local en version 11 s'ouvre, garde ses lignes, et gagne une table
/// **vide** : personne n'a jamais journalisé de sortie avant cette migration,
/// et une table qui se peuplerait inventerait un historique.
#[test]
fn un_etat_local_en_v11_s_ouvre_et_gagne_un_journal_vide() {
    let racine = tempfile::tempdir().expect("répertoire temporaire");
    let chemin = racine.path().join("oxyn.sqlite3");
    let connexion;
    {
        let store = Store::open_at(&chemin).expect("ouverture");
        let workspace = store.workspaces().create("atelier").expect("workspace").id;
        let config = ConnectionConfig::new("base client", DriverId::postgres());
        store
            .connections()
            .save(workspace, &config)
            .expect("connexion");
        connexion = config.id;
        store
            .with_connection(|conn| {
                conn.execute_batch(
                    "DROP TABLE ai_egress;
                     DELETE FROM schema_version WHERE version >= 12;",
                )?;
                Ok(())
            })
            .expect("état d'une version antérieure");
    }

    let store = Store::open_at(&chemin).expect("montée jusqu'à la version courante");
    assert_eq!(
        store.schema_version().expect("version"),
        crate::latest_schema_version()
    );
    assert_eq!(store.workspaces().list().expect("liste").len(), 1);
    assert!(toutes(&store, connexion).is_empty());
    store
        .egress()
        .append(&sortie(connexion))
        .expect("le journal migré est utilisable");
}

// --- Relecture ----------------------------------------------------------------

/// Une portée illisible se relit « inconnue », jamais « locale ».
#[test]
fn une_portee_illisible_ne_devient_jamais_locale() {
    let (store, _, connexion) = decor();
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO ai_egress
                     (ts, connection_id, source, columns, row_count, recipient_id, reach)
                 VALUES (?1, ?2, 'clients', '[\"email\"]', 5, 'anthropic-1a2b3c4d', 'lan')",
                rusqlite::params![Utc::now(), connexion.to_string()],
            )?;
            Ok(())
        })
        .expect("ligne écrite hors d'Oxyn");
    let relue = &toutes(&store, connexion)[0].record;
    assert_eq!(relue.reach, EgressReach::Unresolved);
    assert!(relue.reach.leaves_machine());
}
