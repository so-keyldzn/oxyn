//! Generated DDL is applied only by these explicit isolated-database fixtures.

use crate::integration::{appliquer, session};
use oxyn_catalog::{CatalogPath, DefinitionSource};
use oxyn_core::CancelToken;
use sqlx::{AssertSqlSafe, Connection as _};

type PolicySnapshot = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);

#[test]
fn definition_sql_refuses_legacy_partition_triggers_without_heuristics() {
    let sql = include_str!("catalog/definition.sql");
    assert!(sql.contains("requires PostgreSQL trigger provenance metadata"));
    assert!(sql.contains("NOT (pg_catalog.to_jsonb(tr) ? 'tgparentid')"));
    assert!(!sql.contains("parent_trigger"));
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn definition_recreates_identity_serial_generated_columns_indexes_and_trigger_states() {
    let Some(session) = session().await else {
        return;
    };
    for sql in [
        "CREATE SCHEMA oxyn_definition",
        "CREATE TYPE oxyn_definition.status AS ENUM ('active', 'disabled')",
        r#"CREATE TABLE oxyn_definition."items"";--" (
            id bigint GENERATED ALWAYS AS IDENTITY (START WITH 21 INCREMENT BY 3 CACHE 7),
            code bigserial, status oxyn_definition.status NOT NULL DEFAULT 'active',
            email text COLLATE "C", value numeric(12,2) DEFAULT 3,
            doubled numeric GENERATED ALWAYS AS (value * 2) STORED,
            PRIMARY KEY(id), UNIQUE(code) DEFERRABLE INITIALLY DEFERRED
        ) WITH (fillfactor = 80)"#,
        r#"ALTER TABLE oxyn_definition."items"";--" ADD CONSTRAINT positive CHECK(value > 0) NOT VALID"#,
        r#"CREATE UNIQUE INDEX email_idx ON oxyn_definition."items"";--" (lower(email)) WHERE email IS NOT NULL"#,
        "CREATE FUNCTION oxyn_definition.touch() RETURNS trigger LANGUAGE plpgsql AS 'BEGIN NEW.email := upper(NEW.email); RETURN NEW; END'",
        r#"CREATE TRIGGER touch BEFORE INSERT ON oxyn_definition."items"";--" FOR EACH ROW EXECUTE FUNCTION oxyn_definition.touch()"#,
        r#"ALTER TABLE oxyn_definition."items"";--" DISABLE TRIGGER touch"#,
        r#"INSERT INTO oxyn_definition."items"";--"(email) VALUES ('original')"#,
    ] {
        appliquer(&*session, sql).await;
    }
    let path =
        CatalogPath::for_relation(None, Some("oxyn_definition"), "items\";--").expect("path");
    let definition = session
        .catalog()
        .relation_definition(&path, &CancelToken::new())
        .await
        .expect("definition");
    assert_eq!(definition.source, DefinitionSource::Reconstructed);
    assert!(definition.sql.contains("CREATE UNIQUE INDEX"));
    assert!(definition.sql.contains("NOT VALID"));
    appliquer(&*session, r#"DROP TABLE oxyn_definition."items"";--""#).await;
    let url = std::env::var("OXYN_PG_TEST_URL").expect("isolated test URL");
    let mut control = sqlx::PgConnection::connect(&url)
        .await
        .expect("fixture connection");
    sqlx::raw_sql(AssertSqlSafe(definition.sql.as_str()))
        .execute(&mut control)
        .await
        .expect("recreate from preview SQL");
    let empty: i64 = sqlx::query_scalar(r#"SELECT count(*) FROM oxyn_definition."items"";--""#)
        .fetch_one(&mut control)
        .await
        .expect("row count");
    assert_eq!(empty, 0, "DDL does not copy data");
    let row: (i64, i64, String, String) = sqlx::query_as(r#"INSERT INTO oxyn_definition."items"";--"(email) VALUES ('new') RETURNING id, code, doubled::text, email"#)
        .fetch_one(&mut control).await.expect("generated values");
    assert_eq!(row, (21, 1, "6.00".into(), "new".into()));
    let valid: bool = sqlx::query_scalar("SELECT convalidated FROM pg_constraint WHERE conname = 'positive' AND connamespace = 'oxyn_definition'::regnamespace")
        .fetch_one(&mut control).await.expect("constraint status");
    assert!(!valid);
    let disabled: String = sqlx::query_scalar(
        "SELECT tgenabled::text FROM pg_trigger WHERE tgname = 'touch' AND tgrelid = $1::regclass",
    )
    .bind(r#"oxyn_definition."items"";--""#)
    .fetch_one(&mut control)
    .await
    .expect("trigger state");
    assert_eq!(disabled, "D");
    appliquer(&*session, "DROP SCHEMA oxyn_definition CASCADE").await;
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn definition_supports_views_materialized_views_sequences_and_partitioned_roots() {
    let Some(session) = session().await else {
        return;
    };
    for sql in [
        "CREATE SCHEMA oxyn_definition_kinds",
        "CREATE FUNCTION oxyn_definition_kinds.root_touch() RETURNS trigger LANGUAGE plpgsql AS 'BEGIN RETURN NEW; END'",
        "CREATE TABLE oxyn_definition_kinds.root (id int PRIMARY KEY, tenant text) PARTITION BY RANGE(id)",
        "CREATE TRIGGER root_touch BEFORE INSERT ON oxyn_definition_kinds.root FOR EACH ROW EXECUTE FUNCTION oxyn_definition_kinds.root_touch()",
        "CREATE VIEW oxyn_definition_kinds.v (renamed) WITH (security_barrier = true) AS SELECT 1",
        "CREATE MATERIALIZED VIEW oxyn_definition_kinds.m (renamed) AS SELECT 2 WITH NO DATA",
        "CREATE SEQUENCE oxyn_definition_kinds.s AS integer START 15 INCREMENT 5 MINVALUE 10 MAXVALUE 100 CACHE 3 CYCLE",
    ] {
        appliquer(&*session, sql).await;
    }
    let url = std::env::var("OXYN_PG_TEST_URL").expect("isolated test URL");
    let mut control = sqlx::PgConnection::connect(&url)
        .await
        .expect("fixture connection");
    for (name, kind) in [
        ("root", "TABLE"),
        ("v", "VIEW"),
        ("m", "MATERIALIZED VIEW"),
        ("s", "SEQUENCE"),
    ] {
        let path =
            CatalogPath::for_relation(None, Some("oxyn_definition_kinds"), name).expect("path");
        let definition = session
            .catalog()
            .relation_definition(&path, &CancelToken::new())
            .await
            .expect("native kind");
        appliquer(
            &*session,
            &format!("DROP {kind} oxyn_definition_kinds.{name}"),
        )
        .await;
        sqlx::raw_sql(AssertSqlSafe(definition.sql.as_str()))
            .execute(&mut control)
            .await
            .expect("recreate native kind");
    }
    let value: i64 = sqlx::query_scalar("SELECT nextval('oxyn_definition_kinds.s')")
        .fetch_one(&mut control)
        .await
        .expect("sequence");
    assert_eq!(value, 15);
    let value: i32 = sqlx::query_scalar("SELECT renamed FROM oxyn_definition_kinds.v")
        .fetch_one(&mut control)
        .await
        .expect("view alias");
    assert_eq!(value, 1);
    for sql in [
        "CREATE TABLE oxyn_definition_kinds.child PARTITION OF oxyn_definition_kinds.root FOR VALUES FROM(0) TO(10)",
        "ALTER TABLE oxyn_definition_kinds.child ALTER COLUMN id SET DEFAULT 7",
        "ALTER TABLE oxyn_definition_kinds.child ALTER COLUMN tenant SET NOT NULL",
        "ALTER TABLE oxyn_definition_kinds.child ADD CONSTRAINT child_nonnegative CHECK (id >= 0)",
        "CREATE INDEX child_local ON oxyn_definition_kinds.child (tenant)",
    ] {
        appliquer(&*session, sql).await;
    }
    let child =
        CatalogPath::for_relation(None, Some("oxyn_definition_kinds"), "child").expect("child");
    let definition = session
        .catalog()
        .relation_definition(&child, &CancelToken::new())
        .await
        .expect("partition definition");
    assert!(
        definition
            .sql
            .contains("PARTITION OF oxyn_definition_kinds.root")
    );
    assert!(definition.sql.contains("FOR VALUES FROM (0) TO (10)"));
    appliquer(&*session, "DROP TABLE oxyn_definition_kinds.child").await;
    sqlx::raw_sql(AssertSqlSafe(definition.sql.as_str()))
        .execute(&mut control)
        .await
        .expect("recreate partition child");
    let parent: String = sqlx::query_scalar(
        "SELECT inhparent::regclass::text FROM pg_inherits WHERE inhrelid = $1::regclass",
    )
    .bind("oxyn_definition_kinds.child")
    .fetch_one(&mut control)
    .await
    .expect("partition parent");
    assert_eq!(parent, "oxyn_definition_kinds.root");
    let bound: String = sqlx::query_scalar(
        "SELECT pg_get_expr(relpartbound, oid) FROM pg_class WHERE oid = $1::regclass",
    )
    .bind("oxyn_definition_kinds.child")
    .fetch_one(&mut control)
    .await
    .expect("partition bound");
    assert_eq!(bound, "FOR VALUES FROM (0) TO (10)");
    let default_id: i32 = sqlx::query_scalar(
        "INSERT INTO oxyn_definition_kinds.child(tenant) VALUES ('tenant') RETURNING id",
    )
    .fetch_one(&mut control)
    .await
    .expect("partition default");
    assert_eq!(default_id, 7);
    let cloned_triggers: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_trigger WHERE tgrelid = 'oxyn_definition_kinds.child'::regclass AND tgname = 'root_touch' AND NOT tgisinternal",
    )
    .fetch_one(&mut control)
    .await
    .expect("cloned trigger");
    assert_eq!(cloned_triggers, 1);
    for sql in [
        "CREATE TABLE oxyn_definition_kinds.subroot PARTITION OF oxyn_definition_kinds.root FOR VALUES FROM(10) TO(20) PARTITION BY RANGE(id)",
        "CREATE TABLE oxyn_definition_kinds.subleaf PARTITION OF oxyn_definition_kinds.subroot FOR VALUES FROM(10) TO(15)",
    ] {
        appliquer(&*session, sql).await;
    }
    let subroot =
        CatalogPath::for_relation(None, Some("oxyn_definition_kinds"), "subroot").expect("subroot");
    let definition = session
        .catalog()
        .relation_definition(&subroot, &CancelToken::new())
        .await
        .expect("subpartition definition");
    appliquer(&*session, "DROP TABLE oxyn_definition_kinds.subleaf").await;
    appliquer(&*session, "DROP TABLE oxyn_definition_kinds.subroot").await;
    sqlx::raw_sql(AssertSqlSafe(definition.sql.as_str()))
        .execute(&mut control)
        .await
        .expect("recreate subpartition");
    let key: String =
        sqlx::query_scalar("SELECT pg_get_partkeydef(oid) FROM pg_class WHERE oid = $1::regclass")
            .bind("oxyn_definition_kinds.subroot")
            .fetch_one(&mut control)
            .await
            .expect("subpartition key");
    assert_eq!(key, "RANGE (id)");
    appliquer(&*session, "DROP SCHEMA oxyn_definition_kinds CASCADE").await;
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn definition_recreates_row_security_policies_and_states() {
    let Some(session) = session().await else {
        return;
    };
    for sql in [
        "CREATE ROLE \"oxyn rls role\"",
        "CREATE SCHEMA oxyn_definition_rls",
        r#"CREATE TABLE oxyn_definition_rls."items"";--" (tenant text, payload text)"#,
        r#"ALTER TABLE oxyn_definition_rls."items"";--" ENABLE ROW LEVEL SECURITY"#,
        r#"ALTER TABLE oxyn_definition_rls."items"";--" FORCE ROW LEVEL SECURITY"#,
        r#"CREATE POLICY "only own" ON oxyn_definition_rls."items"";--" AS RESTRICTIVE FOR SELECT TO "oxyn rls role" USING (tenant = current_user)"#,
        r#"CREATE POLICY "public writes" ON oxyn_definition_rls."items"";--" AS PERMISSIVE FOR INSERT TO PUBLIC WITH CHECK (payload <> '')"#,
    ] {
        appliquer(&*session, sql).await;
    }
    let path =
        CatalogPath::for_relation(None, Some("oxyn_definition_rls"), "items\";--").expect("path");
    let definition = session
        .catalog()
        .relation_definition(&path, &CancelToken::new())
        .await
        .expect("definition");
    assert!(definition.sql.contains("CREATE POLICY \"only own\""));
    assert!(definition.sql.contains("TO \"oxyn rls role\""));
    assert!(definition.sql.contains("FORCE ROW LEVEL SECURITY"));
    appliquer(&*session, r#"DROP TABLE oxyn_definition_rls."items"";--""#).await;
    let url = std::env::var("OXYN_PG_TEST_URL").expect("isolated test URL");
    let mut control = sqlx::PgConnection::connect(&url)
        .await
        .expect("fixture connection");
    sqlx::raw_sql(AssertSqlSafe(definition.sql.as_str()))
        .execute(&mut control)
        .await
        .expect("recreate row security definition");
    let states: (bool, bool) = sqlx::query_as(
        "SELECT relrowsecurity, relforcerowsecurity FROM pg_class WHERE oid = $1::regclass",
    )
    .bind(r#"oxyn_definition_rls."items"";--""#)
    .fetch_one(&mut control)
    .await
    .expect("row security states");
    assert_eq!(states, (true, true));
    let policies: Vec<PolicySnapshot> = sqlx::query_as(
            "SELECT policyname, permissive, cmd, array_to_string(roles, ','), qual, with_check FROM pg_policies WHERE schemaname = 'oxyn_definition_rls' ORDER BY policyname",
        )
        .fetch_all(&mut control)
        .await
        .expect("policies");
    assert_eq!(policies.len(), 2);
    assert_eq!(policies[0].0, "only own");
    assert_eq!(policies[0].1, "RESTRICTIVE");
    assert_eq!(policies[0].2, "SELECT");
    assert_eq!(policies[0].3, "oxyn rls role");
    assert_eq!(policies[0].4.as_deref(), Some("(tenant = CURRENT_USER)"));
    assert_eq!(policies[1].0, "public writes");
    assert_eq!(policies[1].1, "PERMISSIVE");
    assert_eq!(policies[1].2, "INSERT");
    assert_eq!(policies[1].3, "public");
    assert_eq!(policies[1].5.as_deref(), Some("(payload <> ''::text)"));
    appliquer(&*session, "DROP SCHEMA oxyn_definition_rls CASCADE").await;
    appliquer(&*session, "DROP ROLE \"oxyn rls role\"").await;
}
