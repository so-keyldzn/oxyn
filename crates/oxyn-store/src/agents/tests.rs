//! Ce que la table des agents externes doit garantir.

use super::*;
use oxyn_core::ExternalAgentConfig;

fn agent(id: &str, label: &str) -> ExternalAgentConfig {
    ExternalAgentConfig::new(
        ProviderId::new(id).expect("un identifiant valide"),
        label,
        "claude",
    )
    .with_args(["--acp"])
}

#[test]
fn une_declaration_se_relit_a_lidentique() {
    let store = Store::open_in_memory().expect("ouverture");
    let ecrit = agent("claude-code", "Claude Code");
    store.external_agents().save(&ecrit).expect("écriture");

    let relus = store.external_agents().list().expect("lecture");
    assert_eq!(relus.len(), 1);
    assert_eq!(relus[0].id, ecrit.id);
    assert_eq!(relus[0].command, "claude");
    assert_eq!(
        relus[0].args,
        ["--acp"],
        "les arguments survivent au disque"
    );
}

/// La table n'a **aucune** colonne où ranger un secret.
///
/// C'est la garantie qui fonde ce mode : un agent porte sa propre
/// authentification. Le test lit le schéma plutôt que la documentation, parce
/// qu'une colonne ajoutée plus tard « juste pour un jeton » ne ferait rougir
/// aucun autre test.
#[test]
fn la_table_na_aucune_colonne_de_secret() {
    let store = Store::open_in_memory().expect("ouverture");
    let colonnes: Vec<String> = store
        .with_connection(|conn| {
            let mut requete =
                conn.prepare("SELECT name FROM pragma_table_info('external_agents')")?;
            let noms = requete.query_map([], |row| row.get(0))?;
            Ok(noms.collect::<rusqlite::Result<Vec<String>>>()?)
        })
        .expect("lecture du schéma");

    for interdite in ["secret_ref", "secret", "api_key", "token", "password"] {
        assert!(
            !colonnes.iter().any(|nom| nom == interdite),
            "`{interdite}` n'a rien à faire ici : un agent externe ne confie aucune clé"
        );
    }
}

/// Une déclaration invalide n'atteint pas le disque.
#[test]
fn une_commande_hostile_est_refusee_avant_le_disque() {
    let store = Store::open_in_memory().expect("ouverture");
    let mut hostile = agent("hostile", "Hostile");
    hostile.command = "claude\n--evil".to_owned();

    assert!(
        store.external_agents().save(&hostile).is_err(),
        "un saut de ligne dans la commande n'a aucun usage légitime"
    );
    assert!(
        store.external_agents().list().expect("lecture").is_empty(),
        "rien ne doit avoir été écrit"
    );
}

/// Une ligne devenue illisible est écartée, pas propagée en erreur.
///
/// Un `args` corrompu par un éditeur SQLite ne doit pas rendre l'écran de
/// configuration inutilisable : l'agent disparaît de la liste, avec une trace.
#[test]
fn une_ligne_illisible_est_ecartee_sans_casser_la_liste() {
    let store = Store::open_in_memory().expect("ouverture");
    store
        .external_agents()
        .save(&agent("bon", "Bon agent"))
        .expect("écriture");
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO external_agents (id, label, command, args, env, created_at, updated_at)
                 VALUES ('casse', 'Cassé', 'claude', 'pas du json', '[]', ?1, ?1)",
                params![Utc::now()],
            )?;
            Ok(())
        })
        .expect("insertion directe");

    let relus = store.external_agents().list().expect("lecture");
    assert_eq!(relus.len(), 1, "la ligne saine reste servie");
    assert_eq!(relus[0].label, "Bon agent");
}

#[test]
fn retirer_une_declaration_ne_touche_pas_les_autres() {
    let store = Store::open_in_memory().expect("ouverture");
    store
        .external_agents()
        .save(&agent("un", "Un"))
        .expect("écriture");
    store
        .external_agents()
        .save(&agent("deux", "Deux"))
        .expect("écriture");

    let cible = ProviderId::new("un").expect("identifiant");
    assert!(store.external_agents().remove(&cible).expect("suppression"));
    assert!(
        !store.external_agents().remove(&cible).expect("suppression"),
        "retirer deux fois ne ment pas sur le second passage"
    );

    let restants = store.external_agents().list().expect("lecture");
    assert_eq!(restants.len(), 1);
    assert_eq!(restants[0].label, "Deux");
}
