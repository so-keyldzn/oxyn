//! Le schéma de l'état local et ses migrations.
//!
//! Les migrations sont des constantes SQL numérotées, appliquées **chacune dans
//! sa transaction**, et enregistrées dans `schema_version`. Une migration qui
//! échoue laisse le schéma exactement dans l'état où elle l'a trouvé : SQLite
//! sait annuler du DDL, contrairement à la plupart de ses concurrents, et c'est
//! ce qui rend cette stratégie tenable sans script de rattrapage.
//!
//! **Une migration publiée ne se réécrit jamais.** Elle a déjà tourné chez
//! quelqu'un ; la modifier fait diverger deux installations qui rapportent le
//! même numéro de schéma. On en ajoute une.
//!
//! # Les tables
//!
//! | Table | Ce qu'elle porte | Effaçable |
//! |---|---|---|
//! | `workspaces` | l'unité de persistance de l'état utilisateur | oui |
//! | `connections` | métadonnées de connexion — **jamais un secret** | oui |
//! | `query_history` | ce que l'utilisateur a exécuté | oui, purge explicite |
//! | `audit_journal` | la piste d'audit — **append-only** | **non** |
//! | `documents` | onglets et requêtes sauvegardés | oui |
//! | `workspace_preferences` | versioned display preferences | yes |
//! | `app_sessions` | ce qui distingue un arrêt propre d'un plantage | oui |
//! | `ai_providers` | les fournisseurs déclarés — **par machine** | oui |
//! | `external_agents` | les agents externes déclarés — **par machine**, sans secret | oui |
//! | `ai_conversations` | les fils de l'assistant, par connexion | oui, élagage et suppression |
//! | `ai_conversation_turns` | leur transcription, **jamais** une valeur de la base | oui, avec leur fil |
//! | `ai_conversation_nodes` | l'arbre des échanges ; un échange retenu ne garde que sa question, et ses mentions en JSON — noms et adresses, jamais une valeur | oui, avec leur fil |
//! | `ai_egress` | ce qui est parti vers un destinataire IA — noms, jamais valeurs — **append-only** | **non** |
//!
//! # Pourquoi `STRICT`
//!
//! Sans `STRICT`, SQLite range volontiers la chaîne `"demain"` dans une colonne
//! `INTEGER` : l'affinité de type n'est qu'une préférence. Un journal d'audit
//! dont les colonnes acceptent n'importe quoi n'est pas une piste d'audit.
//! `STRICT` demande SQLite ≥ 3.37 ; la version embarquée par
//! `libsqlite3-sys` 0.35 (feature `bundled`, ADR-0010) est 3.50.2.
//!
//! # Ce que le déclencheur d'inviolabilité couvre, et ce qu'il ne couvre pas
//!
//! Deux déclencheurs `BEFORE UPDATE` et `BEFORE DELETE` sur `audit_journal`
//! avortent toute tentative, **y compris depuis le `sqlite3` en ligne de
//! commande** : la garantie tient au fichier, pas au code Rust. En revanche
//! aucun déclencheur ne survit à un `DROP TABLE`, à un `PRAGMA writable_schema`
//! ni à la réécriture du fichier avec un éditeur hexadécimal. La protection est
//! contre l'erreur et contre un agent qui voudrait effacer sa trace par les
//! moyens ordinaires du produit — pas contre un attaquant qui a déjà les droits
//! d'écriture sur le disque de l'utilisateur.

use rusqlite::Connection;

use crate::error::{Result, StoreError};

/// Une migration, telle qu'elle est enregistrée dans `schema_version`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Migration {
    /// Numéro, strictement croissant et jamais réutilisé.
    pub(crate) version: u32,
    /// Nom court, écrit dans `schema_version` pour rendre la table lisible.
    pub(crate) name: &'static str,
    /// Le lot SQL. Peut contenir plusieurs instructions.
    pub(crate) sql: &'static str,
}

/// Table de suivi des migrations. Créée hors migration : c'est elle qui dit
/// quelles migrations restent à appliquer.
const SCHEMA_VERSION_TABLE: &str = "\
CREATE TABLE IF NOT EXISTS schema_version (
    version    INTEGER PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    applied_at TEXT    NOT NULL
) STRICT;";

/// Migration 1 — le schéma de la phase 0.
const M0001_INITIAL: &str = "\
CREATE TABLE workspaces (
    id         TEXT PRIMARY KEY NOT NULL,
    name       TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

-- `params` est un objet JSON de valeurs NON secrètes ; `secret_ref` est une
-- référence au trousseau du système, jamais le secret (SECURITY, I-03).
CREATE TABLE connections (
    id           TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    driver       TEXT NOT NULL,
    environment  TEXT NOT NULL,
    params       TEXT NOT NULL,
    secret_ref   TEXT,
    read_only    INTEGER NOT NULL,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
) STRICT;

CREATE INDEX connections_by_workspace ON connections (workspace_id, name);

-- `connection_id` n'a volontairement PAS de clé étrangère : supprimer une
-- connexion n'efface pas ce que l'utilisateur a exécuté avec elle. Le nom est
-- recopié pour que l'historique reste lisible après cette suppression.
CREATE TABLE query_history (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ts              TEXT NOT NULL,
    connection_id   TEXT,
    connection_name TEXT,
    actor_kind      TEXT NOT NULL,
    actor_id        TEXT,
    language        TEXT NOT NULL,
    statement       TEXT NOT NULL,
    intent          TEXT NOT NULL,
    duration_ms     INTEGER,
    row_count       INTEGER,
    status          TEXT NOT NULL,
    error           TEXT
) STRICT;

CREATE INDEX query_history_by_ts ON query_history (ts DESC);
CREATE INDEX query_history_by_connection ON query_history (connection_id, ts DESC);

-- APPEND-ONLY. Aucune clé étrangère non plus : la piste d'audit survit à la
-- suppression de la connexion, du workspace et de l'agent qu'elle incrimine.
-- AUTOINCREMENT plutôt que le rowid nu : un identifiant réutilisé permettrait
-- à une ligne d'en usurper une autre dans une piste d'audit.
CREATE TABLE audit_journal (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    ts               TEXT NOT NULL,
    command_id       TEXT,
    actor_kind       TEXT NOT NULL,
    actor_id         TEXT,
    agent_session_id TEXT,
    connection_id    TEXT,
    command_kind     TEXT NOT NULL,
    statement        TEXT,
    intent           TEXT NOT NULL,
    risk             TEXT NOT NULL,
    policy_decision  TEXT NOT NULL,
    decision_reason  TEXT,
    approved_by      TEXT,
    duration_ms      INTEGER,
    rows_affected    INTEGER,
    error            TEXT
) STRICT;

CREATE INDEX audit_journal_by_ts ON audit_journal (ts DESC);
CREATE INDEX audit_journal_by_connection ON audit_journal (connection_id, ts DESC);

CREATE TRIGGER audit_journal_forbid_update
BEFORE UPDATE ON audit_journal
BEGIN
    SELECT RAISE(ABORT, 'audit_journal is append-only: UPDATE is forbidden');
END;

CREATE TRIGGER audit_journal_forbid_delete
BEFORE DELETE ON audit_journal
BEGIN
    SELECT RAISE(ABORT, 'audit_journal is append-only: DELETE is forbidden');
END;

CREATE TABLE catalog_cache (
    connection_id  TEXT PRIMARY KEY NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
    payload        TEXT NOT NULL,
    refreshed_at   TEXT NOT NULL,
    server_version TEXT
) STRICT;

CREATE TABLE documents (
    id            TEXT PRIMARY KEY NOT NULL,
    workspace_id  TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    title         TEXT NOT NULL,
    language      TEXT NOT NULL,
    content       TEXT NOT NULL,
    connection_id TEXT REFERENCES connections(id) ON DELETE SET NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
) STRICT;

CREATE INDEX documents_by_workspace ON documents (workspace_id, updated_at DESC);
";

/// Migration 2 — la famille de l'erreur, à côté de son message.
///
/// Le message seul ne suffit pas : « délai dépassé après 30 s » ne dit pas que
/// le serveur a peut-être appliqué l'écriture. Sans cette colonne, une interface
/// qui veut refuser un bouton « relancer » n'a d'autre choix que d'analyser du
/// texte français — ce que `.claude/rules/rust.md` interdit précisément parce
/// qu'un message change et qu'un appelant qui l'analysait casse en silence
/// (I-13).
///
/// `NULL` sur les lignes antérieures, et sur toute ligne qui n'a pas échoué.
const M0002_HISTORY_ERROR_CLASS: &str = "\
ALTER TABLE query_history ADD COLUMN error_class TEXT;";

/// Migration 3 — la même famille d'erreur, dans la piste d'audit.
///
/// `query_history` la porte depuis la migration 2 ; l'absence dans
/// `audit_journal` serait l'asymétrie prise à l'envers. C'est la table qu'on
/// relit **après** incident, celle qui consigne les commandes que l'historique
/// ignore, et celle qu'on ne peut pas corriger ensuite : elle est append-only.
///
/// `ALTER TABLE ... ADD COLUMN` n'est pas un `UPDATE` : le déclencheur
/// d'inviolabilité ne s'y oppose pas, et aucune ligne existante n'est réécrite.
const M0003_JOURNAL_ERROR_CLASS: &str = "\
ALTER TABLE audit_journal ADD COLUMN error_class TEXT;";

const M0004_WORKSPACE_PREFERENCES: &str = "CREATE TABLE workspace_preferences (
    workspace_id TEXT PRIMARY KEY NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK(revision > 0),
    payload TEXT NOT NULL CHECK(length(CAST(payload AS BLOB)) <= 4096),
    updated_at TEXT NOT NULL
) STRICT;";

const M0005_QUERY_LIBRARY: &str = "
ALTER TABLE documents ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE documents ADD COLUMN saved_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE documents ADD COLUMN is_saved INTEGER NOT NULL DEFAULT 1;
ALTER TABLE documents ADD COLUMN is_open INTEGER NOT NULL DEFAULT 0;
ALTER TABLE documents ADD COLUMN is_deleted INTEGER NOT NULL DEFAULT 0;
ALTER TABLE documents ADD COLUMN saved_content TEXT;
ALTER TABLE documents ADD COLUMN saved_title TEXT;
UPDATE documents SET saved_content=content, saved_title=title;
ALTER TABLE query_history ADD COLUMN result_id TEXT;
CREATE INDEX documents_by_open ON documents(workspace_id, is_open, id);
";

/// Ce qui distingue un arrêt propre d'un arrêt anormal.
///
/// `closed_at` n'est renseigné que par une fermeture ordinaire, **après** que
/// les écritures locales ont été vidées ; `heartbeat_at` vieillit tant qu'une
/// instance travaille. Une session sans fermeture dont le battement a vieilli
/// est un plantage ; une session sans fermeture au battement récent est une
/// autre instance, bien vivante
/// ([ADR-0021](../../../docs/adr/0021-marqueur-d-arret.md)).
///
/// Le pid n'y figure pas : le vérifier demanderait ce que la politique `unsafe`
/// du dépôt refuse, et un pid réutilisé ferait mentir le test.
const M0006_APP_SESSIONS: &str = "CREATE TABLE app_sessions (
    id           TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    started_at   TEXT NOT NULL,
    heartbeat_at TEXT NOT NULL,
    closed_at    TEXT
) STRICT;
CREATE INDEX app_sessions_open ON app_sessions(workspace_id, closed_at, heartbeat_at);
";

/// Un fournisseur de modèles se déclare **par machine**, et un texte écrit par
/// un agent porte son origine.
///
/// `ai_providers` n'a **pas** de `workspace_id`, et c'est la décision, pas un
/// oubli : un Ollama qui écoute sur la machine sert tous les workspaces, et le
/// dupliquer par workspace créerait autant d'endroits où sa configuration peut
/// diverger. Ce qui reste par connexion, c'est le niveau de confidentialité —
/// un fournisseur commun ne fait pas un niveau commun
/// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md),
/// [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md)).
///
/// Conséquence directe : la table n'a aucune clé étrangère, donc supprimer un
/// workspace ne fait pas disparaître les fournisseurs de l'utilisateur.
///
/// Aucune clé n'y est écrite : `secret_ref` désigne une entrée du trousseau du
/// système, comme pour `connections` (I-03). `reach` n'a **pas** de colonne :
/// le classement local/distant se recalcule à chaque ouverture, parce qu'une
/// valeur en base serait une réponse DNS d'hier appliquée à un envoi
/// d'aujourd'hui.
///
/// `documents.provenance` est un JSON borné à 512 octets, `NULL` par défaut.
/// `NULL` veut dire « écrit par l'utilisateur » — c'est le cas de toutes les
/// lignes existantes, et c'est vrai. Le `CHECK` reprend la forme de
/// `workspace_preferences.payload` : sans lui, cette métadonnée deviendrait
/// l'endroit où l'on range « juste un peu de contexte », c'est-à-dire des
/// invites et des réponses de modèle dans le fichier de workspace.
const M0007_AI_PROVIDERS: &str = "CREATE TABLE ai_providers (
    id         TEXT PRIMARY KEY NOT NULL,
    kind       TEXT NOT NULL,
    label      TEXT NOT NULL,
    base_url   TEXT NOT NULL,
    model      TEXT NOT NULL,
    secret_ref TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE INDEX ai_providers_by_label ON ai_providers (label, id);
ALTER TABLE documents ADD COLUMN provenance TEXT
    CHECK(provenance IS NULL OR length(CAST(provenance AS BLOB)) <= 512);
";

/// Migration 8 — le niveau de confidentialité, là où il appartient.
///
/// [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md) attache le niveau à
/// la **connexion**, et `ConnectionConfig` le porte depuis le début. Il n'était
/// écrit nulle part : toute connexion relue repartait à la valeur par défaut,
/// ce qui rendait `Local` inatteignable d'une session à l'autre — un réglage
/// pris sur une base client était perdu à la fermeture, sans message.
///
/// `NULL` sur les lignes antérieures, et c'est **exact** : elles ont été
/// écrites par un binaire qui ne connaissait pas ce réglage, donc l'utilisateur
/// n'en a jamais choisi. La lecture y applique le défaut d'ADR-0006,
/// `Metadata`. Une valeur **illisible**, elle, n'est pas une absence : la
/// lecture retombe alors sur le niveau le plus contraignant, `Local`, parce
/// qu'une base dont on ne sait plus ce qu'elle autorisait ne doit pas obtenir
/// le bénéfice du doute.
const M0008_CONNECTION_PRIVACY: &str = "ALTER TABLE connections ADD COLUMN privacy_tier TEXT;";

/// Migration 9 — les agents externes déclarés.
///
/// [ADR-0026](../../../docs/adr/0026-agents-externes-acp.md) ajoute un second
/// mode de destination : un programme déjà installé chez l'utilisateur, lancé en
/// sous-processus. Table séparée d'`ai_providers`, et non colonnes ajoutées :
/// un agent n'a ni point d'accès, ni modèle, ni **référence de secret**, et les
/// faire cohabiter aurait produit une table dont la moitié des colonnes ne veut
/// rien dire selon la ligne.
///
/// **Aucune colonne de secret, et c'est le sujet.** Un agent porte sa propre
/// authentification ; Oxyn n'en détient aucune. La seule façon certaine de ne
/// pas divulguer une clé est de ne pas l'avoir ([I-03](../../../CLAUDE.md#i-03)).
///
/// Pas de `workspace_id` non plus, pour la raison d'`ai_providers` : un agent
/// installé sur la machine sert tous les workspaces.
///
/// `args` et `env` sont des JSON bornés — même forme de `CHECK` que
/// `documents.provenance`. Sans la borne, un fichier d'état écrit par un tiers
/// ferait allouer à l'ouverture ce qu'il veut. `env` **ne doit pas** porter de
/// secret : ce qui est là part dans l'environnement d'un processus, visible de
/// la table des processus sur certains systèmes.
///
/// Pas de colonne de portée, et cette fois ce n'est pas parce qu'elle serait
/// périmée comme pour un fournisseur : la portée d'un agent externe est
/// **inconnaissable**, donc il n'y a rien à écrire.
const M0009_EXTERNAL_AGENTS: &str = "CREATE TABLE external_agents (
    id         TEXT PRIMARY KEY NOT NULL,
    label      TEXT NOT NULL,
    command    TEXT NOT NULL,
    args       TEXT NOT NULL
        CHECK(length(CAST(args AS BLOB)) <= 4096),
    env        TEXT NOT NULL
        CHECK(length(CAST(env AS BLOB)) <= 4096),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE INDEX external_agents_by_label ON external_agents (label, id);
";

/// Migration 10 — les conversations de l'assistant, et leurs tours.
///
/// Deux tables et non une : une conversation a une identité, un titre et une
/// destination qui ne changent pas à chaque tour, et les recopier sur chaque
/// ligne de transcription ferait d'un renommage une réécriture de tout le fil.
///
/// # Ce qui n'a **pas** de clé étrangère, et ce qui en a une
///
/// `ai_conversation_turns.conversation_id` en a une, avec `ON DELETE CASCADE`,
/// et c'est le mécanisme qui tient la promesse de l'élagage : supprimer une
/// conversation emporte ses tours **dans la même transaction**, donc il n'existe
/// pas d'état où la moitié d'un transcript subsiste. Une transcription tronquée
/// par le milieu est un transcript qui ment.
///
/// `connection_id` et `destination_id` n'en ont pas, exactement comme
/// `query_history.connection_id` : supprimer une connexion ou retirer un
/// fournisseur n'efface pas ce que l'utilisateur a demandé avec lui. Le nom de
/// connexion et le libellé de destination sont recopiés pour que le fil reste
/// lisible après cette suppression.
///
/// # Le niveau de confidentialité est sur le **tour**
///
/// Pas sur la conversation : un utilisateur peut changer le niveau d'une
/// connexion en cours de fil, et un niveau rangé en tête dirait alors faux de
/// tous les tours antérieurs. Une relecture d'audit demande sous quel régime
/// **ce tour-là** a eu lieu ([I-04](../../../CLAUDE.md#i-04),
/// [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md)).
///
/// # Ce qui est borné dans le fichier, et pourquoi là
///
/// `text`, `reasoning` et `tool_calls` portent un `CHECK` de taille — même
/// forme que `documents.provenance` et `workspace_preferences.payload`. Les
/// deux derniers sont remplis par un **fournisseur** : sans borne, une charge
/// de raisonnement emballée ferait allouer à l'ouverture ce qu'un tiers veut.
/// La borne vit dans le fichier et non seulement dans le code, donc elle est
/// opposable au `sqlite3` autant qu'à Oxyn.
///
/// # Ce qu'il n'y a pas de colonne pour écrire
///
/// Ni les arguments d'un appel d'outil, ni le résultat d'une requête, ni une
/// valeur liée, ni une clé. `tool_calls` porte un **rendu** : nom de l'outil,
/// instruction, issue. C'est la même méthode qu'`external_agents`, qui n'a pas
/// de colonne de secret : la façon certaine de ne pas écrire une valeur est de
/// ne pas avoir d'endroit où la mettre ([I-03](../../../CLAUDE.md#i-03)).
const M0010_AI_CONVERSATIONS: &str = "CREATE TABLE ai_conversations (
    id                TEXT PRIMARY KEY NOT NULL,
    workspace_id      TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    connection_id     TEXT,
    connection_name   TEXT,
    destination_kind  TEXT NOT NULL,
    destination_id    TEXT,
    destination_label TEXT NOT NULL,
    model             TEXT,
    title             TEXT NOT NULL
        CHECK(length(CAST(title AS BLOB)) <= 512),
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
) STRICT;
CREATE INDEX ai_conversations_by_connection ON ai_conversations (connection_id, updated_at DESC);
CREATE INDEX ai_conversations_by_workspace ON ai_conversations (workspace_id, updated_at DESC);

CREATE TABLE ai_conversation_turns (
    conversation_id    TEXT NOT NULL REFERENCES ai_conversations(id) ON DELETE CASCADE,
    ordinal            INTEGER NOT NULL,
    ts                 TEXT NOT NULL,
    role               TEXT NOT NULL,
    privacy_tier       TEXT NOT NULL,
    agent_session_id   TEXT,
    text               TEXT NOT NULL
        CHECK(length(CAST(text AS BLOB)) <= 1048576),
    reasoning          TEXT
        CHECK(reasoning IS NULL OR length(CAST(reasoning AS BLOB)) <= 262144),
    tool_calls         TEXT
        CHECK(tool_calls IS NULL OR length(CAST(tool_calls AS BLOB)) <= 65536),
    stop_reason        TEXT,
    prompt_tokens      INTEGER,
    completion_tokens  INTEGER,
    cache_write_tokens INTEGER,
    cache_read_tokens  INTEGER,
    reasoning_tokens   INTEGER,
    PRIMARY KEY (conversation_id, ordinal)
) STRICT;
";

/// Migration 11 — les deux bornes que la migration 10 n'avait pas mises dans le
/// fichier : le nombre de tours d'un fil, et la taille de `stop_reason`.
///
/// Sans elles, la relecture d'un fil n'était bornée que par le code qui écrit :
/// un `StopReason::Other` de la taille d'une trame SSE, ou des tours écrits par
/// `sqlite3` au-delà de la borne, et l'ouverture d'un fil allouait ce qu'un
/// tiers voulait ([I-06](../../../CLAUDE.md#i-06)).
///
/// # Des déclencheurs, et non un `CHECK`
///
/// SQLite n'ajoute pas de `CHECK` à une table existante : il faudrait la
/// reconstruire en recopiant ses lignes. Et une ligne déjà sur le disque qui
/// dépasserait la nouvelle borne n'aurait alors que deux issues, toutes deux
/// fausses : faire échouer la migration — donc l'ouverture de l'état local —,
/// ou réécrire la donnée de l'utilisateur pendant la copie.
///
/// Un déclencheur `BEFORE INSERT` / `BEFORE UPDATE` borne les **écritures**,
/// y compris celles d'un `sqlite3`, sans toucher à ce qui existe. Ce qui existe
/// se relit selon la règle de `encoding.rs` : une valeur trop grande de
/// `stop_reason` n'est jamais chargée et se relit `Unspecified`, et les tours
/// en surnombre sont lus par pages bornées comme les autres. C'est la forme du
/// déclencheur d'inviolabilité d'`audit_journal`, pour la même raison : la
/// garantie tient au fichier, pas au code Rust.
///
/// Les deux nombres sont ceux de `MAX_TURNS_PER_CONVERSATION` et de
/// `MAX_STOP_REASON_BYTES` ; un test vérifie qu'ils ne divergent pas.
const M0011_AI_CONVERSATION_BOUNDS: &str = "CREATE TRIGGER ai_conversation_turns_bounds_insert
BEFORE INSERT ON ai_conversation_turns
WHEN NEW.ordinal < 0 OR NEW.ordinal >= 512
  OR (NEW.stop_reason IS NOT NULL AND length(CAST(NEW.stop_reason AS BLOB)) > 256)
BEGIN
    SELECT RAISE(ABORT, 'ai_conversation_turns: ordinal or stop_reason exceeds its bound');
END;
CREATE TRIGGER ai_conversation_turns_bounds_update
BEFORE UPDATE OF ordinal, stop_reason ON ai_conversation_turns
WHEN NEW.ordinal < 0 OR NEW.ordinal >= 512
  OR (NEW.stop_reason IS NOT NULL AND length(CAST(NEW.stop_reason AS BLOB)) > 256)
BEGIN
    SELECT RAISE(ABORT, 'ai_conversation_turns: ordinal or stop_reason exceeds its bound');
END;
";

/// Migration 12 — le journal des sorties de données vers un destinataire IA.
///
/// Une table à part, et non des colonnes ajoutées à `audit_journal` : ses
/// colonnes — `command_kind`, la triade du `PolicyGate`, `statement`,
/// `rows_affected` — ont un autre sens, et les détourner rendrait illisibles les
/// deux questions. La lecture de l'échantillon reste journalisée là-bas comme le
/// `PreviewRelation` qu'elle est ; `command_id` relie les deux.
///
/// # La rétention du journal d'audit, c'est-à-dire aucune
///
/// Mêmes déclencheurs d'inviolabilité qu'`audit_journal`, mêmes absences de clé
/// étrangère : l'entrée survit à la suppression de la connexion, du fournisseur
/// et de la conversation qu'elle nomme. Une sortie qu'on pourrait effacer
/// répondrait « rien n'est sorti » à la seule question pour laquelle elle
/// existe.
///
/// # Aucune valeur, et ce que le fichier refuse pour le garantir
///
/// Il n'y a pas de colonne pour une valeur. Ce qui pourrait en faire passer une
/// est `columns`, une liste JSON de **noms** : le `CHECK` exige un tableau borné,
/// et le déclencheur refuse tout élément qui n'est pas une chaîne, qui est vide
/// ou qui dépasse la longueur d'un identifiant. Un nombre, un objet, une ligne
/// collée ne s'y rangent pas. Les bornes sont celles de `oxyn-store::egress`,
/// et un test vérifie qu'elles ne divergent pas.
const M0012_AI_EGRESS: &str = "CREATE TABLE ai_egress (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ts              TEXT NOT NULL,
    connection_id   TEXT NOT NULL,
    command_id      TEXT,
    source          TEXT NOT NULL
        CHECK(length(CAST(source AS BLOB)) BETWEEN 1 AND 1024),
    columns         TEXT NOT NULL
        -- json_array_length rend 0 pour tout ce qui n'est pas un tableau :
        -- `BETWEEN 1` exige donc un tableau non vide, sans clause json_type.
        CHECK(json_valid(columns)
              AND json_array_length(columns) BETWEEN 1 AND 256
              AND length(CAST(columns AS BLOB)) <= 131072),
    row_count       INTEGER NOT NULL CHECK(row_count BETWEEN 0 AND 1000),
    recipient_id    TEXT NOT NULL CHECK(length(CAST(recipient_id AS BLOB)) <= 64),
    model           TEXT CHECK(model IS NULL OR length(CAST(model AS BLOB)) <= 128),
    reach           TEXT NOT NULL CHECK(length(CAST(reach AS BLOB)) <= 16),
    conversation_id TEXT,
    node            INTEGER CHECK(node IS NULL OR node >= 0)
) STRICT;
CREATE INDEX ai_egress_by_connection ON ai_egress (connection_id, id DESC);

CREATE TRIGGER ai_egress_column_names_only
BEFORE INSERT ON ai_egress
WHEN EXISTS (SELECT 1 FROM json_each(NEW.columns)
              WHERE type <> 'text' OR length(CAST(value AS BLOB)) NOT BETWEEN 1 AND 256)
BEGIN
    SELECT RAISE(ABORT, 'ai_egress.columns holds column names only');
END;

CREATE TRIGGER ai_egress_forbid_update
BEFORE UPDATE ON ai_egress
BEGIN
    SELECT RAISE(ABORT, 'ai_egress is append-only: UPDATE is forbidden');
END;

CREATE TRIGGER ai_egress_forbid_delete
BEFORE DELETE ON ai_egress
BEGIN
    SELECT RAISE(ABORT, 'ai_egress is append-only: DELETE is forbidden');
END;
";

/// Migration 13 — l'arbre d'une conversation, et l'échange qui a reçu un
/// échantillon.
///
/// La migration 10 rangeait une transcription **linéaire**. Le panneau tient un
/// **arbre** : une régénération ou une édition crée une version sœur, et
/// l'utilisateur navigue entre elles. `ai_conversation_nodes` porte un échange
/// — sa question, son destinataire, son issue — et les tours existants s'y
/// rattachent par `node`.
///
/// # Ce que le fichier rend impossible, plutôt que de le vérifier
///
/// * **Un cycle.** `parent < node` : un parent est toujours plus ancien que son
///   enfant, et l'identifiant est attribué par le store à l'ajout. Aucune suite
///   de parents ne peut revenir sur elle-même, `sqlite3` compris.
/// * **Un parent d'une autre conversation.** La clé étrangère est composite,
///   `(conversation_id, parent)` : le parent se cherche **dans la même
///   conversation**, et nulle part ailleurs.
/// * **Changer la structure après coup.** Un déclencheur refuse la mise à jour
///   de `conversation_id`, `node` ou `parent` : une version qui changerait de
///   parent réécrirait l'histoire qu'on a montrée.
///
/// # L'échange qui a reçu un échantillon ne garde que sa question
///
/// Décision de l'utilisateur : un échange dont un échantillon de lignes a été
/// envoyé garde sa question, ses **compteurs** — lignes et colonnes, jamais les
/// noms — et son issue. Jamais la réponse, le raisonnement, un appel d'outil ni
/// un message d'erreur : la réponse peut citer les valeurs de l'échantillon, et
/// le fichier de workspace est l'un des six canaux d'I-03.
///
/// La règle tient au fichier, pas à l'appelant :
/// * un déclencheur refuse tout tour rattaché à un échange retenu, à
///   l'insertion comme à la mise à jour ;
/// * poser le marqueur **efface** les tours déjà écrits pour cet échange ;
/// * le marqueur ne se retire pas.
///
/// L'effacement suppose `secure_delete`, que le store pose à l'ouverture : sans
/// lui, SQLite laisse le texte supprimé dans ses pages libres, et le fichier le
/// contiendrait encore. Un `sqlite3` lancé sans ce réglage peut poser le
/// marqueur sans écraser les octets — la garantie « illisible sur le disque »
/// vaut pour ce qu'Oxyn écrit.
///
/// # Anciennes lignes
///
/// Les tours de la migration 10 ont `node = NULL` et restent lisibles par
/// `transcript_page`. Une conversation sans nœud n'a simplement pas de branche.
const M0013_AI_CONVERSATION_TREE: &str = "CREATE TABLE ai_conversation_nodes (
    conversation_id   TEXT NOT NULL REFERENCES ai_conversations(id) ON DELETE CASCADE,
    node              INTEGER NOT NULL CHECK(node BETWEEN 0 AND 255),
    parent            INTEGER CHECK(parent IS NULL OR (parent >= 0 AND parent < node)),
    created_at        TEXT NOT NULL,
    privacy_tier      TEXT NOT NULL CHECK(length(CAST(privacy_tier AS BLOB)) <= 16),
    question          TEXT NOT NULL CHECK(length(CAST(question AS BLOB)) <= 1048576),
    destination_kind  TEXT NOT NULL CHECK(length(CAST(destination_kind AS BLOB)) <= 16),
    destination_id    TEXT CHECK(destination_id IS NULL OR length(CAST(destination_id AS BLOB)) <= 64),
    destination_label TEXT NOT NULL CHECK(length(CAST(destination_label AS BLOB)) <= 128),
    model             TEXT CHECK(model IS NULL OR length(CAST(model AS BLOB)) <= 128),
    sample_withheld   INTEGER NOT NULL DEFAULT 0 CHECK(sample_withheld IN (0, 1)),
    sample_rows       INTEGER CHECK(sample_rows IS NULL OR sample_rows BETWEEN 0 AND 1000),
    sample_columns    INTEGER CHECK(sample_columns IS NULL OR sample_columns BETWEEN 0 AND 256),
    outcome           TEXT CHECK(outcome IS NULL OR length(CAST(outcome AS BLOB)) <= 16),
    outcome_detail    TEXT CHECK(outcome_detail IS NULL OR length(CAST(outcome_detail AS BLOB)) <= 32),
    retryable         INTEGER CHECK(retryable IS NULL OR retryable IN (0, 1)),
    CHECK(sample_withheld = 1 OR (sample_rows IS NULL AND sample_columns IS NULL)),
    CHECK(outcome IS NOT NULL OR (outcome_detail IS NULL AND retryable IS NULL)),
    PRIMARY KEY (conversation_id, node),
    FOREIGN KEY (conversation_id, parent)
        REFERENCES ai_conversation_nodes(conversation_id, node)
) STRICT;

ALTER TABLE ai_conversation_turns ADD COLUMN node INTEGER CHECK(node IS NULL OR node >= 0);
CREATE INDEX ai_conversation_turns_by_node ON ai_conversation_turns (conversation_id, node, ordinal);
ALTER TABLE ai_conversations ADD COLUMN selected_node INTEGER;

CREATE TRIGGER ai_conversation_nodes_structure_is_final
BEFORE UPDATE OF conversation_id, node, parent ON ai_conversation_nodes
BEGIN
    SELECT RAISE(ABORT, 'ai_conversation_nodes: a node never changes place');
END;

CREATE TRIGGER ai_conversation_nodes_withholding_is_final
BEFORE UPDATE OF sample_withheld ON ai_conversation_nodes
WHEN OLD.sample_withheld = 1 AND NEW.sample_withheld = 0
BEGIN
    SELECT RAISE(ABORT, 'ai_conversation_nodes: a withheld sample stays withheld');
END;

CREATE TRIGGER ai_conversation_nodes_withholding_erases_answers
AFTER UPDATE OF sample_withheld ON ai_conversation_nodes
WHEN NEW.sample_withheld = 1
BEGIN
    DELETE FROM ai_conversation_turns
     WHERE conversation_id = NEW.conversation_id AND node = NEW.node;
END;

CREATE TRIGGER ai_conversation_turns_node_insert
BEFORE INSERT ON ai_conversation_turns
WHEN NEW.node IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM ai_conversation_nodes
     WHERE conversation_id = NEW.conversation_id AND node = NEW.node AND sample_withheld = 0)
BEGIN
    SELECT RAISE(ABORT, 'ai_conversation_turns: unknown node, or a node whose sample is withheld');
END;

CREATE TRIGGER ai_conversation_turns_node_update
BEFORE UPDATE ON ai_conversation_turns
WHEN NEW.node IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM ai_conversation_nodes
     WHERE conversation_id = NEW.conversation_id AND node = NEW.node AND sample_withheld = 0)
BEGIN
    SELECT RAISE(ABORT, 'ai_conversation_turns: unknown node, or a node whose sample is withheld');
END;

CREATE TRIGGER ai_conversations_selected_node_insert
BEFORE INSERT ON ai_conversations
WHEN NEW.selected_node IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'ai_conversations: a new conversation has no node to select');
END;

CREATE TRIGGER ai_conversations_selected_node_update
BEFORE UPDATE OF selected_node ON ai_conversations
WHEN NEW.selected_node IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM ai_conversation_nodes
     WHERE conversation_id = NEW.id AND node = NEW.selected_node)
BEGIN
    SELECT RAISE(ABORT, 'ai_conversations: the selected node belongs to another conversation');
END;
";

/// Migration 14 — ce qu'une question a nommé d'un `@`.
///
/// Une conversation relue montre ses questions comme elles ont été posées,
/// puces comprises. La liste est un tableau **JSON** — lisible sans Oxyn
/// ([I-11](../../../CLAUDE.md#i-11)) — que le fichier vérifie et borne ; `NULL`
/// pour une question sans mention, et pour toute ligne antérieure : elle se
/// relit sans puce. Jamais une valeur de ligne : une sorte, un nom, une adresse.
const M0014_AI_EXCHANGE_MENTIONS: &str =
    "ALTER TABLE ai_conversation_nodes ADD COLUMN mentions TEXT
    CHECK(mentions IS NULL
          OR (json_valid(mentions) AND json_type(mentions) = 'array'
              AND length(CAST(mentions AS BLOB)) <= 32768));
";

/// Migration 15 — le cache de catalogue persisté quitte le fichier.
///
/// La table `catalog_cache` de la migration 1 n'a jamais eu d'appelant : le
/// catalogue vit en mémoire dans `oxyn-exec`, se relit du serveur à chaque
/// connexion, et Oxyn n'offre pas de consultation hors ligne
/// ([ARCHITECTURE §6](../../../docs/ARCHITECTURE.md#6-le-catalogue)). Une
/// table sans écrivain laisse croire à un lecteur du fichier qu'elle compte.
///
/// Rien de l'utilisateur ne se perd : ce qu'elle aurait pu contenir est une
/// copie de ce que le serveur rend, jamais un travail saisi dans Oxyn
/// ([I-11](../../../CLAUDE.md#i-11)). Revenir en arrière, c'est une migration
/// qui recrée la table avec le DDL de la migration 1, restée lisible ci-dessus ;
/// une persistance future passera d'abord par un ADR.
///
/// `IF EXISTS` : un fichier dont la table a déjà été retirée au `sqlite3` doit
/// s'ouvrir, pas échouer à la migration.
const M0015_DROP_CATALOG_CACHE: &str = "DROP TABLE IF EXISTS catalog_cache;";

/// Migration 16 — quand l'utilisateur a vérifié sur le serveur une écriture à l'issue inconnue.
///
/// Sans elle, une seule écriture expirée rappelait l'avertissement de reprise
/// à chaque lancement, pour toujours — et un avertissement qu'on ne peut pas
/// faire taire cesse d'être lu, y compris le jour où une autre écriture a
/// réellement été interrompue. Un horodatage en texte ISO 8601 plutôt qu'un
/// drapeau : lisible par `sqlite3` sans Oxyn ([I-11](../../../CLAUDE.md#i-11)),
/// et il dit *quand* la vérification a été faite. `NULL` sur toute ligne
/// antérieure : rien n'a été vérifié.
const M0016_HISTORY_RECONCILED: &str = "ALTER TABLE query_history ADD COLUMN reconciled_at TEXT;";

/// Migration 17 — quand un lancement suivant a annoncé un arrêt anormal.
///
/// Sans elle, une session abandonnée le restait pour toujours : un seul
/// plantage montrait l'écran de reprise à chaque lancement, fermetures propres
/// comprises. `closed_at` reste `NULL` — inscrire une fermeture ici mentirait
/// sur la façon dont le lancement s'est terminé. Un horodatage plutôt qu'un
/// drapeau, pour la même raison que la migration 16 : il se lit sans Oxyn
/// ([I-11](../../../CLAUDE.md#i-11)). `NULL` sur toute ligne antérieure : les
/// sessions déjà abandonnées sont annoncées une dernière fois.
const M0017_APP_SESSIONS_REPORTED: &str = "ALTER TABLE app_sessions ADD COLUMN reported_at TEXT;";

/// Migration 18 — la disposition des fenêtres
/// ([ADR-0043](../../../docs/adr/0043-multi-fenetre.md)).
///
/// Des colonnes plutôt qu'un JSON : l'appartenance d'une console à une seule
/// fenêtre est une contrainte que le fichier tient lui-même,
/// `UNIQUE (document_id)`, et une écriture qui la violerait échoue au lieu de
/// produire deux fenêtres rivales sur le même document. Lisible avec
/// n'importe quel client SQLite ([I-11](../../../CLAUDE.md#i-11)).
/// `app_session_id` nomme le dernier lancement qui a écrit la ligne : un
/// lancement n'adopte que celles d'un lancement terminé, pas celles d'une
/// autre instance vivante sur le même fichier. `object_location` a la forme
/// qu'il avait dans les préférences.
const M0018_WORKSPACE_WINDOWS: &str = "CREATE TABLE workspace_windows (
    id               TEXT PRIMARY KEY NOT NULL,
    workspace_id     TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    app_session_id   TEXT NOT NULL,
    ordinal          INTEGER NOT NULL,
    x                REAL,
    y                REAL,
    width            REAL NOT NULL,
    height           REAL NOT NULL,
    maximized        INTEGER NOT NULL,
    object_location  TEXT,
    active_document  TEXT REFERENCES documents(id) ON DELETE SET NULL,
    revision         INTEGER NOT NULL,
    updated_at       TEXT NOT NULL
) STRICT;
CREATE TABLE workspace_window_consoles (
    window_id    TEXT NOT NULL REFERENCES workspace_windows(id) ON DELETE CASCADE,
    document_id  TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    PRIMARY KEY (window_id, document_id),
    UNIQUE (document_id)
) STRICT;
CREATE INDEX workspace_windows_order ON workspace_windows(workspace_id, ordinal);
";

/// Toutes les migrations, dans l'ordre d'application.
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        sql: M0001_INITIAL,
    },
    Migration {
        version: 2,
        name: "history_error_class",
        sql: M0002_HISTORY_ERROR_CLASS,
    },
    Migration {
        version: 3,
        name: "journal_error_class",
        sql: M0003_JOURNAL_ERROR_CLASS,
    },
    Migration {
        version: 4,
        name: "workspace_preferences",
        sql: M0004_WORKSPACE_PREFERENCES,
    },
    Migration {
        version: 5,
        name: "query_library",
        sql: M0005_QUERY_LIBRARY,
    },
    Migration {
        version: 6,
        name: "app_sessions",
        sql: M0006_APP_SESSIONS,
    },
    Migration {
        version: 7,
        name: "ai_providers",
        sql: M0007_AI_PROVIDERS,
    },
    Migration {
        version: 8,
        name: "connection_privacy_tier",
        sql: M0008_CONNECTION_PRIVACY,
    },
    Migration {
        version: 9,
        name: "external_agents",
        sql: M0009_EXTERNAL_AGENTS,
    },
    Migration {
        version: 10,
        name: "ai_conversations",
        sql: M0010_AI_CONVERSATIONS,
    },
    Migration {
        version: 11,
        name: "ai_conversation_bounds",
        sql: M0011_AI_CONVERSATION_BOUNDS,
    },
    Migration {
        version: 12,
        name: "ai_egress",
        sql: M0012_AI_EGRESS,
    },
    Migration {
        version: 13,
        name: "ai_conversation_tree",
        sql: M0013_AI_CONVERSATION_TREE,
    },
    Migration {
        version: 14,
        name: "ai_exchange_mentions",
        sql: M0014_AI_EXCHANGE_MENTIONS,
    },
    Migration {
        version: 15,
        name: "drop_catalog_cache",
        sql: M0015_DROP_CATALOG_CACHE,
    },
    Migration {
        version: 16,
        name: "history_reconciled",
        sql: M0016_HISTORY_RECONCILED,
    },
    Migration {
        version: 17,
        name: "app_sessions_reported",
        sql: M0017_APP_SESSIONS_REPORTED,
    },
    Migration {
        version: 18,
        name: "workspace_windows",
        sql: M0018_WORKSPACE_WINDOWS,
    },
];

/// Version de schéma que ce binaire sait produire.
#[must_use]
pub fn latest_version() -> u32 {
    match MIGRATIONS.last() {
        Some(derniere) => derniere.version,
        // Inatteignable tant que `MIGRATIONS` n'est pas vide, mais un `expect`
        // ici paniquerait à l'ouverture de l'application.
        None => 0,
    }
}

/// Version de schéma actuellement inscrite dans le fichier.
///
/// Rend `0` sur une base neuve.
///
/// # Erreurs
/// [`StoreError::Sqlite`] si la table de suivi est illisible.
pub fn current_version(conn: &Connection) -> Result<u32> {
    let suivi_present: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'schema_version'",
        [],
        |row| row.get(0),
    )?;
    if suivi_present == 0 {
        return Ok(0);
    }

    // `MAX()` rend toujours une ligne, `NULL` sur une table vide.
    let brut = conn.query_row("SELECT MAX(version) FROM schema_version", [], |row| {
        row.get::<_, Option<i64>>(0)
    })?;
    Ok(brut.map_or(0, |v| u32::try_from(v).unwrap_or(u32::MAX)))
}

/// Applique les migrations manquantes.
///
/// Idempotent : appelé sur une base déjà à jour, il ne fait rien et ne modifie
/// pas `schema_version`.
///
/// # Erreurs
/// * [`StoreError::SchemaTooRecent`] si le fichier vient d'une version
///   ultérieure d'Oxyn — on refuse d'écrire dans un schéma qu'on ne comprend
///   pas plutôt que de corrompre la piste d'audit ;
/// * [`StoreError::Migration`] si un lot SQL est refusé. La transaction est
///   alors annulée et le schéma reste dans son état antérieur.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_VERSION_TABLE)?;

    let actuelle = current_version(conn)?;
    let cible = latest_version();
    if actuelle > cible {
        return Err(StoreError::SchemaTooRecent {
            found: actuelle,
            supported: cible,
        });
    }

    for migration in MIGRATIONS.iter().filter(|m| m.version > actuelle) {
        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)
            .map_err(|source| StoreError::Migration {
                version: migration.version,
                name: migration.name,
                source,
            })?;
        tx.execute(
            "INSERT INTO schema_version (version, name, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                i64::from(migration.version),
                migration.name,
                chrono::Utc::now()
            ],
        )
        .map_err(|source| StoreError::Migration {
            version: migration.version,
            name: migration.name,
            source,
        })?;
        tx.commit()?;
        tracing::debug!(
            version = migration.version,
            name = migration.name,
            "applied local state migration"
        );
    }

    Ok(())
}

/// Builds a local state file **at exactly `version`**, for tests that need a
/// file written by an earlier build.
///
/// Applying only the migrations up to `version` is the one form that does not
/// rot: a test that instead opened a current file and undid the later
/// migrations by hand broke every time a migration was added — twice already.
///
/// # Panics
/// On any SQLite failure: this is test scaffolding, and a fixture that cannot
/// be built is a broken test, not a condition to handle.
#[cfg(test)]
pub(crate) fn file_at_version(path: &std::path::Path, version: u32) -> Connection {
    let mut conn = Connection::open(path).expect("fixture file opens");
    conn.pragma_update(None, "foreign_keys", true)
        .expect("foreign keys on the fixture");
    conn.execute_batch(SCHEMA_VERSION_TABLE)
        .expect("schema_version on the fixture");
    for migration in MIGRATIONS.iter().filter(|m| m.version <= version) {
        let tx = conn.transaction().expect("fixture transaction");
        tx.execute_batch(migration.sql)
            .expect("an earlier migration applies");
        tx.execute(
            "INSERT INTO schema_version (version, name, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![migration.version, migration.name, chrono::Utc::now()],
        )
        .expect("fixture version recorded");
        tx.commit().expect("fixture commit");
    }
    conn
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_migree() -> Connection {
        let mut conn = Connection::open_in_memory().expect("une base en mémoire s'ouvre toujours");
        migrate(&mut conn).expect("le schéma initial s'applique");
        conn
    }

    #[test]
    fn les_numeros_de_migration_sont_uniques_et_croissants() {
        let mut precedent = 0;
        for migration in MIGRATIONS {
            assert!(
                migration.version > precedent,
                "la migration `{}` casse l'ordre",
                migration.name
            );
            precedent = migration.version;
        }
        assert_eq!(latest_version(), precedent);
    }

    #[test]
    fn migrer_est_idempotent() {
        let mut conn = Connection::open_in_memory().expect("base en mémoire");

        migrate(&mut conn).expect("première application");
        let apres_une = current_version(&conn).expect("version lisible");
        assert_eq!(apres_une, latest_version());

        migrate(&mut conn).expect("seconde application");
        migrate(&mut conn).expect("troisième application");
        assert_eq!(current_version(&conn).expect("version lisible"), apres_une);

        let lignes: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
            .expect("comptage");
        assert_eq!(
            lignes,
            i64::from(latest_version()),
            "une migration ne doit être inscrite qu'une fois"
        );
    }

    /// Une base **déjà en v1, avec des lignes**, monte sans les perdre.
    ///
    /// `migrer_est_idempotent` part de zéro et applique tout d'un bloc : il ne
    /// prouve rien sur le seul cas qui existe chez un utilisateur, celui d'un
    /// fichier écrit par une version précédente. Ce test traverse les deux
    /// migrations incrémentales du dépôt, dont celle qui touche la table
    /// **append-only** : un `ADD COLUMN` n'est pas un `UPDATE`, mais c'est
    /// exactement le genre d'affirmation qui mérite un test plutôt qu'un
    /// raisonnement.
    #[test]
    fn une_base_deja_en_v1_monte_sans_perdre_ses_lignes() {
        let mut conn = Connection::open_in_memory().expect("base en mémoire");
        conn.execute_batch(SCHEMA_VERSION_TABLE).expect("suivi");
        conn.execute_batch(M0001_INITIAL).expect("schéma de la v1");
        conn.execute(
            "INSERT INTO schema_version (version, name, applied_at) VALUES (1, 'initial', ?1)",
            rusqlite::params![chrono::Utc::now()],
        )
        .expect("inscription de la v1");
        conn.execute(
            "INSERT INTO query_history
                 (ts, actor_kind, language, statement, intent, status, error)
             VALUES (?1, 'human', '\"sql\"', 'INSERT INTO commandes VALUES (1)', 'write',
                     'failed', 'délai dépassé après 30s')",
            rusqlite::params![chrono::Utc::now()],
        )
        .expect("une ligne écrite par la version précédente");
        conn.execute(
            "INSERT INTO audit_journal
                 (ts, actor_kind, command_kind, intent, risk, policy_decision, error)
             VALUES (?1, 'agent', 'Execute', 'write', '\"none\"', 'allow',
                     'délai dépassé après 30s')",
            rusqlite::params![chrono::Utc::now()],
        )
        .expect("une entrée d'audit écrite par la version précédente");

        migrate(&mut conn).expect("montée jusqu'à la version courante");

        assert_eq!(current_version(&conn).expect("version"), latest_version());
        // La ligne survit, et sa famille est `NULL` : personne ne peut la
        // deviner sans analyser son texte, ce que la colonne existe pour
        // remplacer. `HistoryRecord::is_retryable` traite ce `NULL` comme
        // « on ne sait pas », donc comme non rejouable (I-13).
        let (statement, class): (String, Option<String>) = conn
            .query_row(
                "SELECT statement, error_class FROM query_history",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("la ligne antérieure a survécu");
        assert!(statement.starts_with("INSERT"));
        assert_eq!(class, None);

        // La piste d'audit aussi : ajouter une colonne ne réécrit aucune ligne,
        // donc le déclencheur d'inviolabilité n'a rien à refuser.
        let (kind, class): (String, Option<String>) = conn
            .query_row(
                "SELECT command_kind, error_class FROM audit_journal",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("l'entrée d'audit antérieure a survécu");
        assert_eq!(kind, "Execute");
        assert_eq!(class, None);
    }

    #[test]
    fn query_library_migration_preserves_legacy_named_copies() {
        let mut connection = Connection::open_in_memory().expect("connection");
        connection
            .execute_batch(SCHEMA_VERSION_TABLE)
            .expect("versions");
        for migration in MIGRATIONS.iter().filter(|migration| migration.version < 5) {
            connection
                .execute_batch(migration.sql)
                .expect("legacy schema");
            connection
                .execute(
                    "INSERT INTO schema_version(version,name,applied_at) VALUES(?1,?2,?3)",
                    rusqlite::params![migration.version, migration.name, chrono::Utc::now()],
                )
                .expect("version");
        }
        connection.execute_batch("INSERT INTO workspaces VALUES('workspace','Legacy','2026-09-10','2026-09-10');
            INSERT INTO documents VALUES('document','workspace','Report','\"sql\"','SELECT 1',NULL,'2026-09-10','2026-09-10');").expect("legacy document");
        migrate(&mut connection).expect("migration");
        let state: (String, String, bool, bool, i64, i64) = connection.query_row(
            "SELECT saved_content,saved_title,is_saved,is_open,revision,saved_revision FROM documents",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        ).expect("preserved baseline");
        assert_eq!(
            state,
            ("SELECT 1".into(), "Report".into(), true, false, 0, 0)
        );
    }

    /// Un document écrit avant la migration 7 n'a **pas** de provenance, et
    /// c'est vrai : c'est l'utilisateur qui l'a écrit.
    ///
    /// Le test tient aussi la seconde moitié de la décision — la borne de 512
    /// octets est dans le fichier, donc opposable à `sqlite3` autant qu'à
    /// Oxyn : sans elle, la colonne deviendrait un endroit où archiver la
    /// conversation.
    #[test]
    fn une_provenance_absente_veut_dire_ecrite_par_l_utilisateur() {
        let mut conn = Connection::open_in_memory().expect("base en mémoire");
        conn.execute_batch(SCHEMA_VERSION_TABLE).expect("suivi");
        for migration in MIGRATIONS.iter().filter(|migration| migration.version < 7) {
            conn.execute_batch(migration.sql).expect("schéma antérieur");
            conn.execute(
                "INSERT INTO schema_version(version,name,applied_at) VALUES(?1,?2,?3)",
                rusqlite::params![migration.version, migration.name, chrono::Utc::now()],
            )
            .expect("inscription");
        }
        conn.execute_batch(
            "INSERT INTO workspaces VALUES('workspace','Atelier','2026-09-10','2026-09-10');
             INSERT INTO documents (id,workspace_id,title,language,content,created_at,updated_at)
             VALUES('document','workspace','Rapport','\"sql\"','SELECT 1','2026-09-10','2026-09-10');",
        )
        .expect("document écrit par la version précédente");

        migrate(&mut conn).expect("montée jusqu'à la version courante");

        let (contenu, provenance): (String, Option<String>) = conn
            .query_row("SELECT content, provenance FROM documents", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .expect("le document a survécu");
        assert_eq!(contenu, "SELECT 1");
        assert_eq!(
            provenance, None,
            "une ligne antérieure n'appartient à aucun agent"
        );

        let trop_gros = format!("{{\"model\":\"{}\"}}", "x".repeat(512));
        let refus = conn.execute(
            "UPDATE documents SET provenance = ?1 WHERE id = 'document'",
            rusqlite::params![trop_gros],
        );
        assert!(
            refus.is_err(),
            "le budget de 512 octets doit tenir dans le fichier, pas seulement dans le code"
        );
    }

    /// Un état local écrit par la version précédente s'ouvre sans rien perdre.
    ///
    /// C'est le seul cas qui existe chez un utilisateur : `migrer_est_idempotent`
    /// part de zéro et n'en prouve rien. Ce test pose un fichier **en v9 avec
    /// des lignes** — un workspace, une connexion, un document, une entrée
    /// d'historique, une trace d'audit —, applique la migration 10, et vérifie
    /// que tout est encore là, les deux tables neuves comprises et vides.
    ///
    /// Vides, et c'est exact : personne n'a jamais eu de conversation persistée
    /// avant cette migration. Une table neuve qui se peuplerait toute seule
    /// inventerait de l'historique.
    #[test]
    fn un_etat_local_en_v9_s_ouvre_sans_perdre_ses_lignes() {
        let mut conn = Connection::open_in_memory().expect("base en mémoire");
        conn.execute_batch(SCHEMA_VERSION_TABLE).expect("suivi");
        for migration in MIGRATIONS.iter().filter(|migration| migration.version < 10) {
            conn.execute_batch(migration.sql).expect("schéma antérieur");
            conn.execute(
                "INSERT INTO schema_version(version,name,applied_at) VALUES(?1,?2,?3)",
                rusqlite::params![migration.version, migration.name, chrono::Utc::now()],
            )
            .expect("inscription");
        }
        conn.execute_batch(
            "INSERT INTO workspaces VALUES('workspace','Atelier','2026-09-10','2026-09-10');
             INSERT INTO connections (id,workspace_id,name,driver,environment,params,read_only,
                                      created_at,updated_at)
             VALUES('connexion','workspace','base client','postgres','production','{}',0,
                    '2026-09-10','2026-09-10');
             INSERT INTO documents (id,workspace_id,title,language,content,created_at,updated_at)
             VALUES('document','workspace','Rapport','\"sql\"','SELECT 1','2026-09-10','2026-09-10');
             INSERT INTO query_history (ts,actor_kind,language,statement,intent,status)
             VALUES('2026-09-10','human','\"sql\"','SELECT 1','read','succeeded');
             INSERT INTO audit_journal (ts,actor_kind,command_kind,intent,risk,policy_decision)
             VALUES('2026-09-10','agent','Execute','read','\"none\"','allow');",
        )
        .expect("des lignes écrites par la version précédente");

        migrate(&mut conn).expect("montée jusqu'à la version courante");
        assert_eq!(current_version(&conn).expect("version"), latest_version());

        for (table, attendu) in [
            ("workspaces", 1),
            ("connections", 1),
            ("documents", 1),
            ("query_history", 1),
            ("audit_journal", 1),
            // Neuves, donc vides : rien n'invente de conversation passée.
            ("ai_conversations", 0),
            ("ai_conversation_turns", 0),
        ] {
            let lignes: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .expect("comptage");
            assert_eq!(lignes, attendu, "table `{table}`");
        }

        // Et la table neuve est utilisable dans la foulée, clé étrangère vers le
        // workspace existant comprise.
        conn.execute(
            "INSERT INTO ai_conversations
                 (id, workspace_id, destination_kind, destination_label, title,
                  created_at, updated_at)
             VALUES ('fil','workspace','provider','Anthropic','Un fil','2026-09-16','2026-09-16')",
            [],
        )
        .expect("le fil s'écrit dans le schéma migré");
    }

    /// Un fichier en v14 dont le cache de catalogue est rempli perd la table,
    /// et rien d'autre : la connexion qu'elle référençait reste.
    #[test]
    fn la_migration_15_retire_le_cache_de_catalogue_sans_toucher_aux_connexions() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let mut conn = file_at_version(&dir.path().join("v14.sqlite3"), 14);
        conn.execute_batch(
            "INSERT INTO workspaces VALUES('workspace','Atelier','2026-09-10','2026-09-10');
             INSERT INTO connections (id,workspace_id,name,driver,environment,params,read_only,
                                      created_at,updated_at)
             VALUES('connexion','workspace','base client','postgres','production','{}',0,
                    '2026-09-10','2026-09-10');
             INSERT INTO catalog_cache (connection_id,payload,refreshed_at)
             VALUES('connexion','{}','2026-09-10');",
        )
        .expect("a v14 file with a cached catalog");

        migrate(&mut conn).expect("montée jusqu'à la version courante");

        let table: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'catalog_cache'",
                [],
                |row| row.get(0),
            )
            .expect("interrogation du schéma");
        assert_eq!(table, 0, "la table sans appelant a quitté le fichier");
        let connexions: i64 = conn
            .query_row("SELECT COUNT(*) FROM connections", [], |row| row.get(0))
            .expect("comptage");
        assert_eq!(connexions, 1);
    }

    /// `IF EXISTS` : une table déjà retirée à la main n'empêche pas l'ouverture.
    #[test]
    fn la_migration_15_tolere_une_table_deja_retiree() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let mut conn = file_at_version(&dir.path().join("v14.sqlite3"), 14);
        conn.execute_batch("DROP TABLE catalog_cache;")
            .expect("removed by hand");

        migrate(&mut conn).expect("la migration s'applique quand même");
        assert_eq!(current_version(&conn).expect("version"), latest_version());
    }

    #[test]
    fn un_schema_venu_du_futur_est_refuse() {
        let mut conn = base_migree();
        conn.execute(
            "INSERT INTO schema_version (version, name, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![9_999_i64, "venue-du-futur", chrono::Utc::now()],
        )
        .expect("insertion");

        let erreur = migrate(&mut conn).expect_err("le futur ne s'applique pas à l'envers");
        assert!(matches!(
            erreur,
            StoreError::SchemaTooRecent { found: 9_999, .. }
        ));
    }

    #[test]
    fn les_tables_attendues_existent() {
        let conn = base_migree();
        for table in [
            "workspaces",
            "connections",
            "query_history",
            "audit_journal",
            "documents",
            "workspace_preferences",
            "app_sessions",
            "ai_providers",
            "ai_conversations",
            "ai_conversation_turns",
            "ai_egress",
            "ai_conversation_nodes",
            "schema_version",
        ] {
            let presente: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .expect("interrogation du schéma");
            assert_eq!(presente, 1, "table `{table}` absente");
        }
    }

    #[test]
    fn les_tables_du_domaine_sont_strictes() {
        // Sans STRICT, SQLite range une chaîne dans une colonne INTEGER.
        let conn = base_migree();
        for table in [
            "workspaces",
            "connections",
            "query_history",
            "audit_journal",
            "documents",
            "ai_providers",
            "ai_conversations",
            "ai_conversation_turns",
            "ai_egress",
            "ai_conversation_nodes",
        ] {
            let sql: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .expect("définition de table");
            assert!(
                sql.contains("STRICT"),
                "la table `{table}` n'est pas STRICT"
            );
        }
    }

    #[test]
    fn les_declencheurs_d_inviolabilite_existent() {
        let conn = base_migree();
        for declencheur in ["audit_journal_forbid_update", "audit_journal_forbid_delete"] {
            let present: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'trigger' AND name = ?1",
                    [declencheur],
                    |row| row.get(0),
                )
                .expect("interrogation du schéma");
            assert_eq!(present, 1, "déclencheur `{declencheur}` absent");
        }
    }
}
