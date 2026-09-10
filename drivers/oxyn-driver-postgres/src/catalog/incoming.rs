//! Reverse foreign-key discovery through the same cancellable catalog connection.

use super::*;
use oxyn_catalog::IncomingForeignKey;

const SQL_INCOMING_KEYS: &str = r#"
SELECT sn.nspname::text, sc.relname::text, k.conname::text,
       ARRAY(SELECT a.attname::text FROM pg_catalog.unnest(k.conkey) WITH ORDINALITY AS x(num, ord)
             JOIN pg_catalog.pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = x.num ORDER BY x.ord LIMIT 129),
       ARRAY(SELECT a.attname::text FROM pg_catalog.unnest(k.confkey) WITH ORDINALITY AS x(num, ord)
             JOIN pg_catalog.pg_attribute a ON a.attrelid = k.confrelid AND a.attnum = x.num ORDER BY x.ord LIMIT 129),
       k.confdeltype::text,
       CASE WHEN EXISTS (
           SELECT 1 FROM pg_catalog.unnest(k.conkey) WITH ORDINALITY AS x(src, ord)
           JOIN pg_catalog.pg_attribute sa ON sa.attrelid = k.conrelid AND sa.attnum = x.src
           JOIN pg_catalog.pg_attribute ta ON ta.attrelid = k.confrelid AND ta.attnum = k.confkey[x.ord::int]
           WHERE sa.atttypid <> ta.atttypid OR sa.attcollation <> ta.attcollation
       ) OR EXISTS (
           SELECT 1 FROM pg_catalog.pg_index i
           JOIN pg_catalog.pg_opclass op ON op.oid = ANY(i.indclass)
           WHERE i.indexrelid = k.conindid AND NOT op.opcdefault
       ) OR EXISTS (
           SELECT 1 FROM pg_catalog.pg_index i
           WHERE i.indrelid = k.conrelid AND i.indisunique
             AND (i.indpred IS NOT NULL OR i.indexprs IS NOT NULL
                  OR EXISTS (SELECT 1 FROM pg_catalog.pg_opclass op
                             WHERE op.oid = ANY(i.indclass) AND NOT op.opcdefault)
                  OR EXISTS (
                      SELECT 1 FROM pg_catalog.unnest(i.indkey) WITH ORDINALITY AS x(num, ord)
                      JOIN pg_catalog.pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = x.num
                      WHERE x.ord <= i.indnkeyatts AND i.indcollation[(x.ord - 1)::int] IS DISTINCT FROM a.attcollation
                  ))
       ) THEN NULL::boolean ELSE EXISTS (
           SELECT 1 FROM pg_catalog.pg_index i
           WHERE i.indrelid = k.conrelid AND i.indisunique AND i.indisvalid AND i.indisready
             AND i.indpred IS NULL AND i.indexprs IS NULL
             AND NOT EXISTS (
                 SELECT 1 FROM pg_catalog.unnest(i.indkey) WITH ORDINALITY AS x(num, ord)
                 JOIN pg_catalog.pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = x.num
                 WHERE x.ord <= i.indnkeyatts
                   AND (NOT x.num = ANY(k.conkey) OR i.indcollation[(x.ord - 1)::int] IS DISTINCT FROM a.attcollation)
             )
             AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_opclass op
                             WHERE op.oid = ANY(i.indclass) AND NOT op.opcdefault)
       ) END
FROM pg_catalog.pg_constraint k
JOIN pg_catalog.pg_class tc ON tc.oid = k.confrelid
JOIN pg_catalog.pg_namespace tn ON tn.oid = tc.relnamespace
JOIN pg_catalog.pg_class sc ON sc.oid = k.conrelid
JOIN pg_catalog.pg_namespace sn ON sn.oid = sc.relnamespace
WHERE tn.nspname = $1 AND tc.relname = $2 AND k.contype = 'f'
ORDER BY sn.nspname, sc.relname, k.conname LIMIT 1025
"#;

impl PostgresCatalog {
    pub(super) async fn read_incoming_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<IncomingForeignKey>> {
        self.capabilities
            .require(Capabilities::INCOMING_FOREIGN_KEYS)?;
        let (namespace, name) = self.require_relation(relation)?;
        let rows = self
            .fetch(cancel, SQL_INCOMING_KEYS, &[namespace, name])
            .await?;
        if rows.len() > 1024 {
            return Err(OxynError::CatalogUnavailable(
                "incoming foreign keys exceed 1024 entries".into(),
            ));
        }
        rows.iter()
            .map(|row| {
                if cancel.is_cancelled() {
                    return Err(OxynError::Cancelled);
                }
                let source = CatalogPath::for_relation(
                    Some(&self.database),
                    Some(&read_text(row, 0)?),
                    read_text(row, 1)?,
                )?;
                let fields = row
                    .try_get::<Vec<String>, _>(3)
                    .map_err(|_| invalid_key())?;
                let target_fields = row
                    .try_get::<Vec<String>, _>(4)
                    .map_err(|_| invalid_key())?;
                let mut key = ForeignKey::new(
                    read_text(row, 2)?,
                    fields,
                    ForeignKeyTarget {
                        relation: CatalogPath::for_relation(
                            Some(&self.database),
                            Some(namespace),
                            name,
                        )?,
                        fields: target_fields,
                    },
                );
                if key.fields.len() > 128 || !key.is_well_formed() {
                    return Err(invalid_key());
                }
                key.on_delete = match read_text(row, 5)?.as_str() {
                    "a" => ReferentialAction::NoAction,
                    "r" => ReferentialAction::Restrict,
                    "c" => ReferentialAction::Cascade,
                    "n" => ReferentialAction::SetNull,
                    "d" => ReferentialAction::SetDefault,
                    _ => return Err(invalid_key()),
                };
                let source_unique = row
                    .try_get::<Option<bool>, _>(6)
                    .map_err(|_| invalid_key())?;
                Ok(IncomingForeignKey {
                    source,
                    key,
                    source_unique,
                })
            })
            .collect()
    }
}

fn invalid_key() -> OxynError {
    OxynError::CatalogUnavailable("inconsistent incoming foreign key metadata".into())
}
