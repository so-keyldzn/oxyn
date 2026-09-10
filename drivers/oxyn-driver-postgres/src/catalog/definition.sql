WITH target AS (
    SELECT c.*, n.nspname,
           pg_catalog.quote_ident(n.nspname) || '.' || pg_catalog.quote_ident(c.relname) AS qualified
    FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = $1 AND c.relname = $2
), eligible AS (
    SELECT t.*, parent.qualified AS partition_parent,
           pg_catalog.pg_get_expr(t.relpartbound, t.oid, false) AS partition_bound
    FROM target t
    LEFT JOIN pg_catalog.pg_inherits inheritance ON inheritance.inhrelid = t.oid
    LEFT JOIN pg_catalog.pg_class parent_class ON parent_class.oid = inheritance.inhparent
    LEFT JOIN pg_catalog.pg_namespace parent_namespace ON parent_namespace.oid = parent_class.relnamespace
    LEFT JOIN LATERAL (SELECT pg_catalog.quote_ident(parent_namespace.nspname) || '.' || pg_catalog.quote_ident(parent_class.relname) AS qualified) parent ON true
    WHERE t.relkind IN ('r', 'p', 'v', 'm', 'S')
      AND t.relpersistence <> 't' AND t.reloftype = 0
      AND ((NOT t.relispartition AND inheritance.inhrelid IS NULL)
           OR (t.relispartition AND inheritance.inhrelid IS NOT NULL AND NOT COALESCE((pg_catalog.to_jsonb(inheritance)->>'inhdetachpending')::boolean, false)
               AND parent_class.relkind = 'p' AND parent.qualified IS NOT NULL))
      AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_constraint k
                      WHERE k.conrelid = t.oid AND k.contype = 'n' AND NOT k.convalidated)
), sequences AS (
    SELECT s.*, sc.relname, sc.relpersistence AS sequence_persistence, sn.nspname, d.refobjid, d.refobjsubid, d.deptype,
           pg_catalog.quote_ident(sn.nspname) || '.' || pg_catalog.quote_ident(sc.relname) AS qualified
    FROM pg_catalog.pg_sequence s
    JOIN pg_catalog.pg_class sc ON sc.oid = s.seqrelid
    JOIN pg_catalog.pg_namespace sn ON sn.oid = sc.relnamespace
    LEFT JOIN pg_catalog.pg_depend d ON d.objid = s.seqrelid
        AND d.classid = 'pg_catalog.pg_class'::pg_catalog.regclass
        AND d.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass AND d.deptype IN ('a', 'i')
    WHERE s.seqrelid IN (SELECT oid FROM eligible WHERE relkind = 'S')
       OR d.refobjid IN (SELECT oid FROM eligible)
), sequence_sql AS (
    SELECT s.*,
           ' INCREMENT BY ' || s.seqincrement || ' MINVALUE ' || s.seqmin ||
           ' MAXVALUE ' || s.seqmax || ' START WITH ' || s.seqstart ||
           ' CACHE ' || s.seqcache || CASE WHEN s.seqcycle THEN ' CYCLE' ELSE ' NO CYCLE' END AS options
    FROM sequences s
), table_options AS (
    SELECT t.oid,
           CASE WHEN t.relam <> 0 AND t.relkind <> 'p' THEN ' USING ' || pg_catalog.quote_ident(am.amname) ELSE '' END ||
           CASE WHEN pg_catalog.array_length(t.reloptions, 1) > 0 THEN ' WITH (' || (
               SELECT pg_catalog.string_agg(pg_catalog.quote_ident(pg_catalog.split_part(opt, '=', 1)) || '=' ||
                   pg_catalog.quote_literal(pg_catalog.substr(opt, pg_catalog.strpos(opt, '=') + 1)), ', ')
               FROM pg_catalog.unnest(t.reloptions) opt
           ) || ')' ELSE '' END ||
           CASE WHEN t.reltablespace <> 0 THEN ' TABLESPACE ' || pg_catalog.quote_ident(ts.spcname) ELSE '' END AS suffix
    FROM eligible t
    LEFT JOIN pg_catalog.pg_am am ON am.oid = t.relam
    LEFT JOIN pg_catalog.pg_tablespace ts ON ts.oid = t.reltablespace
), parts AS (
    SELECT 0 AS phase, 0::bigint AS ordinal, 'unsupported'::text AS kind,
           'DDL for temporary, inherited, foreign or unsupported objects is not available.'::text AS content
    FROM target t WHERE NOT EXISTS (SELECT 1 FROM eligible e WHERE e.oid = t.oid)
    UNION ALL
    SELECT 0, 0, 'unsupported',
           'DDL for a partition with user triggers requires PostgreSQL trigger provenance metadata.'
    FROM eligible t
    WHERE t.relispartition AND EXISTS (SELECT 1 FROM pg_catalog.pg_trigger tr
        WHERE tr.tgrelid = t.oid AND NOT tr.tgisinternal AND NOT (pg_catalog.to_jsonb(tr) ? 'tgparentid'))
    UNION ALL
    SELECT 1, s.seqrelid::bigint, 'statement',
           'CREATE ' || CASE s.sequence_persistence WHEN 'u' THEN 'UNLOGGED ' ELSE '' END || 'SEQUENCE ' || s.qualified || ' AS ' || pg_catalog.format_type(s.seqtypid, -1) || s.options
    FROM sequence_sql s WHERE s.deptype = 'a' OR s.seqrelid IN (SELECT oid FROM eligible WHERE relkind = 'S')
    UNION ALL
    SELECT 10, 0, 'head', 'CREATE ' || CASE WHEN t.relpersistence = 'u' THEN 'UNLOGGED ' ELSE '' END ||
           'TABLE ' || t.qualified || ' ('
    FROM eligible t WHERE t.relkind IN ('r', 'p') AND NOT t.relispartition
    UNION ALL
    SELECT 11, a.attnum::bigint, 'column',
           pg_catalog.quote_ident(a.attname) || ' ' || pg_catalog.format_type(a.atttypid, a.atttypmod) ||
           CASE WHEN a.attcollation <> 0 AND a.attcollation <> typ.typcollation
                THEN ' COLLATE ' || pg_catalog.quote_ident(cn.nspname) || '.' || pg_catalog.quote_ident(coll.collname) ELSE '' END ||
           CASE WHEN a.attidentity IN ('a', 'd') THEN
               ' GENERATED ' || CASE a.attidentity WHEN 'a' THEN 'ALWAYS' ELSE 'BY DEFAULT' END ||
               ' AS IDENTITY (' || (SELECT 'SEQUENCE NAME ' || s.qualified || s.options
                   FROM sequence_sql s WHERE s.refobjid = t.oid AND s.refobjsubid = a.attnum AND s.deptype = 'i') || ')'
           WHEN a.attidentity <> '' THEN NULL
           WHEN a.attgenerated IN ('s', 'v') THEN ' GENERATED ALWAYS AS (' || pg_catalog.pg_get_expr(ad.adbin, ad.adrelid) || ')' ||
               CASE a.attgenerated WHEN 's' THEN ' STORED' ELSE ' VIRTUAL' END
           WHEN a.attgenerated <> '' THEN NULL
           WHEN ad.adbin IS NOT NULL THEN ' DEFAULT ' || pg_catalog.pg_get_expr(ad.adbin, ad.adrelid)
           ELSE '' END ||
           CASE WHEN a.attnotnull THEN COALESCE((SELECT ' CONSTRAINT ' || pg_catalog.quote_ident(k.conname)
               FROM pg_catalog.pg_constraint k WHERE k.conrelid = t.oid AND k.contype = 'n'
                   AND a.attnum = ANY(k.conkey) LIMIT 1), '') || ' NOT NULL' ELSE '' END
    FROM eligible t
    JOIN pg_catalog.pg_attribute a ON a.attrelid = t.oid AND a.attnum > 0 AND NOT a.attisdropped
    JOIN pg_catalog.pg_type typ ON typ.oid = a.atttypid
    LEFT JOIN pg_catalog.pg_attrdef ad ON ad.adrelid = t.oid AND ad.adnum = a.attnum
    LEFT JOIN pg_catalog.pg_collation coll ON coll.oid = a.attcollation
    LEFT JOIN pg_catalog.pg_namespace cn ON cn.oid = coll.collnamespace
    WHERE t.relkind IN ('r', 'p') AND NOT t.relispartition
    UNION ALL
    SELECT 12, 0, 'tail', ')' || CASE WHEN t.relkind = 'p'
           THEN ' PARTITION BY ' || pg_catalog.pg_get_partkeydef(t.oid) ELSE '' END || o.suffix
    FROM eligible t JOIN table_options o ON o.oid = t.oid WHERE t.relkind IN ('r', 'p') AND NOT t.relispartition
    UNION ALL
    SELECT 10, t.oid::bigint, 'statement',
           'CREATE TABLE ' || t.qualified || ' PARTITION OF ' || t.partition_parent ||
           COALESCE((' (' || (SELECT pg_catalog.string_agg(
               pg_catalog.quote_ident(a.attname) ||
               CASE WHEN ad.adbin IS NOT NULL THEN ' DEFAULT ' || pg_catalog.pg_get_expr(ad.adbin, ad.adrelid) ELSE '' END ||
               CASE WHEN local_not_null.name IS NOT NULL THEN ' CONSTRAINT ' || pg_catalog.quote_ident(local_not_null.name) || ' NOT NULL' ELSE '' END,
               ', ' ORDER BY a.attnum)
               FROM pg_catalog.pg_attribute a
               LEFT JOIN pg_catalog.pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum
               LEFT JOIN LATERAL (SELECT k.conname AS name FROM pg_catalog.pg_constraint k
                   WHERE k.conrelid = t.oid AND k.contype = 'n' AND k.conislocal AND k.conparentid = 0
                     AND a.attnum = ANY(k.conkey) LIMIT 1) local_not_null ON true
               WHERE a.attrelid = t.oid AND a.attnum > 0 AND NOT a.attisdropped
                 AND (ad.adbin IS NOT NULL OR local_not_null.name IS NOT NULL)) || ')'), '') ||
           ' ' || t.partition_bound || CASE WHEN t.relkind = 'p' THEN ' PARTITION BY ' || pg_catalog.pg_get_partkeydef(t.oid) ELSE '' END || o.suffix
    FROM eligible t JOIN table_options o ON o.oid = t.oid WHERE t.relispartition
    UNION ALL
    SELECT 10, 0, 'statement', 'CREATE ' || CASE WHEN t.relkind = 'm' THEN 'MATERIALIZED ' ELSE '' END ||
           'VIEW ' || t.qualified || ' (' || (SELECT pg_catalog.string_agg(pg_catalog.quote_ident(a.attname), ', ' ORDER BY a.attnum)
               FROM pg_catalog.pg_attribute a WHERE a.attrelid = t.oid AND a.attnum > 0 AND NOT a.attisdropped) || ')' ||
           CASE WHEN t.relkind = 'm' THEN o.suffix ELSE
               CASE WHEN pg_catalog.array_length(t.reloptions, 1) > 0 THEN ' WITH (' || (SELECT pg_catalog.string_agg(
                   pg_catalog.quote_ident(pg_catalog.split_part(opt, '=', 1)) || '=' ||
                   pg_catalog.quote_literal(pg_catalog.substr(opt, pg_catalog.strpos(opt, '=') + 1)), ', ')
                   FROM pg_catalog.unnest(t.reloptions) opt) || ')' ELSE '' END END ||
           ' AS ' || pg_catalog.rtrim(pg_catalog.pg_get_viewdef(t.oid, false), E' \n\r\t;') ||
           CASE WHEN t.relkind = 'm' THEN ' WITH NO DATA' ELSE '' END
    FROM eligible t JOIN table_options o ON o.oid = t.oid WHERE t.relkind IN ('v', 'm')
    UNION ALL
    SELECT 20, k.oid::bigint, 'statement', 'ALTER TABLE ' || t.qualified || ' ADD CONSTRAINT ' ||
           pg_catalog.quote_ident(k.conname) || ' ' || pg_catalog.pg_get_constraintdef(k.oid, false)
    FROM eligible t JOIN pg_catalog.pg_constraint k ON k.conrelid = t.oid
    WHERE k.contype NOT IN ('n', 't') AND (NOT t.relispartition OR (k.conislocal AND k.conparentid = 0))
    UNION ALL
    SELECT 30, s.seqrelid::bigint, 'statement', 'ALTER SEQUENCE ' || s.qualified || ' OWNED BY ' || t.qualified || '.' || pg_catalog.quote_ident(a.attname)
    FROM sequence_sql s JOIN eligible t ON t.oid = s.refobjid
    JOIN pg_catalog.pg_attribute a ON a.attrelid = t.oid AND a.attnum = s.refobjsubid WHERE s.deptype = 'a'
    UNION ALL
    SELECT 40, i.indexrelid::bigint, 'statement', pg_catalog.pg_get_indexdef(i.indexrelid)
    FROM eligible t JOIN pg_catalog.pg_index i ON i.indrelid = t.oid
    WHERE NOT EXISTS (SELECT 1 FROM pg_catalog.pg_constraint k WHERE k.conindid = i.indexrelid AND k.contype IN ('p', 'u', 'x'))
      AND (NOT t.relispartition OR NOT EXISTS (SELECT 1 FROM pg_catalog.pg_inherits inherited_index WHERE inherited_index.inhrelid = i.indexrelid))
    UNION ALL
    SELECT 50, r.oid::bigint, 'statement', pg_catalog.pg_get_ruledef(r.oid, false)
    FROM eligible t JOIN pg_catalog.pg_rewrite r ON r.ev_class = t.oid WHERE r.rulename <> '_RETURN'
    UNION ALL
    SELECT 60, tr.oid::bigint, 'statement', pg_catalog.pg_get_triggerdef(tr.oid, false)
    FROM eligible t JOIN pg_catalog.pg_trigger tr ON tr.tgrelid = t.oid
    WHERE NOT tr.tgisinternal AND (NOT t.relispartition OR COALESCE((pg_catalog.to_jsonb(tr)->>'tgparentid')::oid, 0) = 0)
    UNION ALL
    SELECT 65, p.oid::bigint, 'statement', 'CREATE POLICY ' || pg_catalog.quote_ident(p.polname) ||
           ' ON ' || t.qualified || ' AS ' || CASE WHEN p.polpermissive THEN 'PERMISSIVE' ELSE 'RESTRICTIVE' END ||
           ' FOR ' || CASE p.polcmd WHEN 'r' THEN 'SELECT' WHEN 'a' THEN 'INSERT' WHEN 'w' THEN 'UPDATE'
               WHEN 'd' THEN 'DELETE' WHEN '*' THEN 'ALL' END ||
           ' TO ' || (SELECT pg_catalog.string_agg(CASE WHEN member.role_oid = 0 THEN 'PUBLIC'
                   ELSE pg_catalog.quote_ident(role.rolname) END, ', ' ORDER BY member.ordinal)
               FROM pg_catalog.unnest(p.polroles) WITH ORDINALITY AS member(role_oid, ordinal)
               LEFT JOIN pg_catalog.pg_roles role ON role.oid = member.role_oid) ||
           CASE WHEN p.polqual IS NOT NULL THEN ' USING (' || pg_catalog.pg_get_expr(p.polqual, p.polrelid, false) || ')' ELSE '' END ||
           CASE WHEN p.polwithcheck IS NOT NULL THEN ' WITH CHECK (' || pg_catalog.pg_get_expr(p.polwithcheck, p.polrelid, false) || ')' ELSE '' END
    FROM eligible t JOIN pg_catalog.pg_policy p ON p.polrelid = t.oid
    UNION ALL
    SELECT 70, tr.oid::bigint, 'statement', 'ALTER TABLE ' || t.qualified || CASE tr.tgenabled
           WHEN 'D' THEN ' DISABLE' WHEN 'R' THEN ' ENABLE REPLICA' WHEN 'A' THEN ' ENABLE ALWAYS' END ||
           ' TRIGGER ' || pg_catalog.quote_ident(tr.tgname)
    FROM eligible t JOIN pg_catalog.pg_trigger tr ON tr.tgrelid = t.oid
    WHERE NOT tr.tgisinternal AND tr.tgenabled <> 'O' AND (NOT t.relispartition OR COALESCE((pg_catalog.to_jsonb(tr)->>'tgparentid')::oid, 0) = 0)
    UNION ALL
    SELECT 70, 0, 'statement', 'ALTER TABLE ' || t.qualified || ' ENABLE ROW LEVEL SECURITY'
    FROM eligible t WHERE t.relrowsecurity
    UNION ALL
    SELECT 71, 0, 'statement', 'ALTER TABLE ' || t.qualified || ' FORCE ROW LEVEL SECURITY'
    FROM eligible t WHERE t.relforcerowsecurity
)
SELECT kind, CASE WHEN pg_catalog.octet_length(content) <= 1048576 THEN content END
FROM parts ORDER BY phase, ordinal LIMIT 8193
