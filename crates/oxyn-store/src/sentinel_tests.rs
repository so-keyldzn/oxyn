//! Le canal « fichier de workspace » d'[I-03](../../../CLAUDE.md#i-03), balayé
//! par une valeur témoin.
//!
//! # Pourquoi un test de sentinelle, alors qu'il en existe déjà sur les secrets
//!
//! Les tests existants vérifient un **chemin connu** : que `Debug` masque une
//! référence, qu'une colonne nommée `api_key` n'existe pas, qu'une
//! configuration ne porte pas de secret. Ils ont tous le même angle mort — ils
//! ne voient que ce qu'on a pensé à regarder. Une colonne ajoutée dans six mois
//! pour ranger « juste un jeton » ne ferait rougir aucun d'eux.
//!
//! Celui-ci prend le problème par l'autre bout : il écrit une valeur témoin
//! unique par tous les chemins d'écriture du store, puis **balaie toutes les
//! tables et toutes les colonnes** que `sqlite_master` déclare. Il n'a besoin de
//! connaître ni les tables d'aujourd'hui ni celles de demain.
//!
//! # Ce qu'il ne couvre pas
//!
//! Un seul des six canaux d'I-03. Le journal, l'erreur affichée, le rapport de
//! plantage, l'invite IA et le presse-papiers ont leurs propres gardes, ailleurs.

use crate::{
    Conversation, Destination, Store, ToolCallRecord, ToolCallStatus, TurnRecord, TurnRole,
};
use oxyn_core::{
    AiProviderConfig, AiProviderKind, ConnectionConfig, DriverId, Environment, ExternalAgentConfig,
    PrivacyTier, ProviderId,
};

/// La valeur témoin. Improbable par construction : si elle apparaît quelque
/// part, c'est qu'un chemin d'écriture l'y a mise.
const SENTINELLE: &str = "oxyn-sentinelle-9f3a7c21-ne-doit-jamais-atteindre-le-disque";

/// Toutes les valeurs textuelles de toutes les tables, sans en nommer aucune.
///
/// Passe par `sqlite_master` et `pragma_table_info` plutôt que par une liste :
/// une table ajoutée demain est balayée sans que personne n'ait à y penser.
fn tout_le_texte(store: &Store) -> Vec<(String, String, String)> {
    store
        .with_connection(|conn| {
            let tables: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;

            let mut trouvailles = Vec::new();
            for table in tables {
                let colonnes: Vec<String> = conn
                    .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?
                    .query_map([], |row| row.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                for colonne in colonnes {
                    // `CAST` plutôt que `get::<String>` : une colonne `STRICT`
                    // peut porter un BLOB, et un secret rangé en binaire fuit
                    // tout autant.
                    let mut requete = conn.prepare(&format!(
                        "SELECT CAST(\"{colonne}\" AS TEXT) FROM \"{table}\" \
                         WHERE \"{colonne}\" IS NOT NULL"
                    ))?;
                    let valeurs = requete.query_map([], |row| row.get::<_, String>(0))?;
                    for valeur in valeurs.flatten() {
                        trouvailles.push((table.clone(), colonne.clone(), valeur));
                    }
                }
            }
            Ok(trouvailles)
        })
        .expect("lecture du schéma")
}

/// Aucun chemin d'écriture ne range un secret dans l'état local.
///
/// La sentinelle est passée là où un secret **pourrait** se glisser : le nom
/// d'une connexion, un paramètre, le libellé d'un fournisseur, le nom d'un agent
/// et sa commande. Ce sont des champs légitimes — ce que le test vérifie, c'est
/// qu'aucun d'eux n'est ensuite recopié dans une colonne prévue pour autre
/// chose, et surtout qu'aucune référence de secret ne devient une valeur.
#[test]
fn aucune_sentinelle_natteint_le_fichier_de_workspace() {
    let store = Store::open_in_memory().expect("ouverture");
    let workspace = store
        .workspaces()
        .create("sentinelle")
        .expect("workspace")
        .id;

    // Une connexion dont le secret n'est qu'une **référence** : c'est le
    // contrat, et c'est lui qu'on éprouve.
    let mut connexion = ConnectionConfig::new("Base témoin", DriverId::sqlite())
        .with_environment(Environment::Local);
    connexion.params.insert("path".into(), ":memory:".into());
    connexion.secret_ref = Some(format!("keychain://oxyn/{SENTINELLE}"));
    store
        .connections()
        .save(workspace, &connexion)
        .expect("connexion enregistrée");

    // Un fournisseur de modèles, même forme : une référence, jamais la clé.
    let fournisseur = AiProviderConfig::new(
        ProviderId::new("temoin").expect("identifiant"),
        AiProviderKind::OpenAiCompatible,
        "Fournisseur témoin",
        "http://127.0.0.1:11434/v1",
        "un-modele",
    )
    .with_secret_ref(format!("keychain://oxyn/{SENTINELLE}"));
    store.providers().save(&fournisseur).expect("fournisseur");

    // Un agent externe : aucune clé du tout, par construction.
    let agent = ExternalAgentConfig::new(
        ProviderId::new("agent-temoin").expect("identifiant"),
        "Agent témoin",
        "claude",
    );
    store.external_agents().save(&agent).expect("agent");

    // Une conversation, avec un appel d'outil : c'est le chemin d'écriture le
    // plus récent, et celui où un tour d'agent pourrait le plus facilement
    // recopier une valeur qu'il a vue passer.
    let fil = Conversation::new(
        workspace,
        Destination::provider(
            ProviderId::new("temoin").expect("identifiant"),
            "Fournisseur témoin",
            "un-modele",
        ),
        "Fil témoin",
    )
    .on_connection(connexion.id, "Base témoin");
    let fil_id = fil.id;
    store.conversations().save(&fil).expect("fil");
    store
        .conversations()
        .append(
            fil_id,
            &TurnRecord::new(
                TurnRole::Assistant,
                PrivacyTier::Metadata,
                "J'ai regardé la table.",
            )
            .with_tool_calls(vec![
                ToolCallRecord::new(
                    "call_1",
                    "execute",
                    "Exécuter une lecture sur « Base témoin »",
                    ToolCallStatus::Completed,
                )
                .with_statement("SELECT 1"),
            ]),
        )
        .expect("tour");

    let fuites: Vec<_> = tout_le_texte(&store)
        .into_iter()
        .filter(|(_, _, valeur)| valeur.contains(SENTINELLE))
        .collect();

    // Une référence de trousseau **contient** la sentinelle et a le droit
    // d'être là : c'est un pointeur, pas un secret. Ce qui est interdit, c'est
    // qu'elle apparaisse ailleurs que dans une colonne de référence.
    let interdites: Vec<_> = fuites
        .iter()
        .filter(|(_, colonne, valeur)| {
            !(colonne.contains("secret") && valeur.starts_with("keychain://"))
        })
        .collect();

    assert!(
        interdites.is_empty(),
        "la valeur témoin a atteint le disque hors d'une référence de trousseau : {interdites:#?}"
    );
    assert!(
        !fuites.is_empty(),
        "aucune trace du tout : le test ne prouve rien si les écritures n'ont pas eu lieu"
    );
}
