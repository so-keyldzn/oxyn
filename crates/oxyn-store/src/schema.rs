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
//! | `catalog_cache` | l'introspection mise en cache, par connexion | oui |
//! | `documents` | onglets et requêtes sauvegardés | oui |
//! | `workspace_preferences` | versioned display preferences | yes |
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
            "catalog_cache",
            "documents",
            "workspace_preferences",
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
            "catalog_cache",
            "documents",
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
