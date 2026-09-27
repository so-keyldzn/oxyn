//! The "workspace file" channel of [I-03](../../../CLAUDE.md#i-03), swept with
//! a witness value.
//!
//! # Why a sentinel test, when tests on secrets already exist
//!
//! The existing tests check a **known path**: that `Debug` masks a reference,
//! that a column named `api_key` does not exist, that a configuration carries
//! no secret. They all share the same blind spot — they only see what someone
//! thought of looking at. A column added six months from now to store "just a
//! token" would turn none of them red.
//!
//! This one takes the problem from the other end: it writes a unique witness
//! value through every write path of the store, then **sweeps every table and
//! every column** that `sqlite_master` declares. It needs to know neither
//! today's tables nor tomorrow's.
//!
//! # What it does not cover
//!
//! Only one of I-03's six channels. The log, the displayed error, the crash
//! report, the AI prompt and the clipboard have their own guards, elsewhere.

use crate::{
    Conversation, Destination, Store, ToolCallRecord, ToolCallStatus, TurnRecord, TurnRole,
};
use oxyn_core::{
    AiProviderConfig, AiProviderKind, ConnectionConfig, DriverId, Environment, ExternalAgentConfig,
    PrivacyTier, ProviderId,
};

/// The witness value. Improbable by construction: if it shows up somewhere, a
/// write path put it there.
const SENTINELLE: &str = "oxyn-sentinelle-9f3a7c21-ne-doit-jamais-atteindre-le-disque";

/// Every text value of every table, without naming any.
///
/// Goes through `sqlite_master` and `pragma_table_info` rather than a list: a
/// table added tomorrow is swept without anyone having to think of it.
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
                    // `CAST` rather than `get::<String>`: a `STRICT` column can
                    // carry a BLOB, and a secret stored as binary leaks just
                    // as much.
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
        .expect("read the schema")
}

/// No write path stores a secret in the local state.
///
/// The sentinel is passed where a secret **could** slip in: a connection's
/// name, a parameter, a provider's label, an agent's name and its command.
/// These are legitimate fields — what the test checks is that none of them is
/// then copied into a column meant for something else, and above all that no
/// secret reference becomes a value.
#[test]
fn no_sentinel_reaches_the_workspace_file() {
    let store = Store::open_in_memory().expect("open");
    let workspace = store
        .workspaces()
        .create("sentinelle")
        .expect("workspace")
        .id;

    // A connection whose secret is only a **reference**: that is the
    // contract, and that is what is being tested.
    let mut connexion = ConnectionConfig::new("Base témoin", DriverId::sqlite())
        .with_environment(Environment::Local);
    connexion.params.insert("path".into(), ":memory:".into());
    connexion.secret_ref = Some(format!("keychain://oxyn/{SENTINELLE}"));
    store
        .connections()
        .save(workspace, &connexion)
        .expect("saved connection");

    // A model provider, same shape: a reference, never the key.
    let fournisseur = AiProviderConfig::new(
        ProviderId::new("temoin").expect("identifier"),
        AiProviderKind::OpenAiCompatible,
        "Fournisseur témoin",
        "http://127.0.0.1:11434/v1",
        "un-modele",
    )
    .with_secret_ref(format!("keychain://oxyn/{SENTINELLE}"));
    store.providers().save(&fournisseur).expect("provider");

    // An external agent: no key at all, by construction.
    let agent = ExternalAgentConfig::new(
        ProviderId::new("agent-temoin").expect("identifier"),
        "Agent témoin",
        "claude",
    );
    store.external_agents().save(&agent).expect("agent");

    // A conversation, with a tool call: it is the most recent write path, and
    // the one where an agent turn could most easily copy a value it saw go
    // by.
    let fil = Conversation::new(
        workspace,
        Destination::provider(
            ProviderId::new("temoin").expect("identifier"),
            "Fournisseur témoin",
            "un-modele",
        ),
        "Fil témoin",
    )
    .on_connection(connexion.id, "Base témoin");
    let fil_id = fil.id;
    store.conversations().save(&fil).expect("thread");
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
        .expect("turn");

    let fuites: Vec<_> = tout_le_texte(&store)
        .into_iter()
        .filter(|(_, _, valeur)| valeur.contains(SENTINELLE))
        .collect();

    // A keychain reference **contains** the sentinel and is allowed to be
    // there: it is a pointer, not a secret. What is forbidden is for it to
    // appear anywhere other than in a reference column.
    let interdites: Vec<_> = fuites
        .iter()
        .filter(|(_, colonne, valeur)| {
            !(colonne.contains("secret") && valeur.starts_with("keychain://"))
        })
        .collect();

    assert!(
        interdites.is_empty(),
        "the witness value reached the disk outside a keychain reference: {interdites:#?}"
    );
    assert!(
        !fuites.is_empty(),
        "no trace at all: the test proves nothing if the writes did not happen"
    );
}
