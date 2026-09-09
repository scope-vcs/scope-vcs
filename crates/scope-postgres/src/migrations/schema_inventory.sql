WITH relations AS (
    SELECT c.*
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = current_schema()
      AND c.relname <> 'seaql_migrations'
      AND c.relkind NOT IN ('i', 'I', 't')
), indexes AS (
    SELECT i.*, c.relname, c.reloptions, c.reltablespace
    FROM pg_index i
    JOIN pg_class c ON c.oid = i.indexrelid
    JOIN relations r ON r.oid = i.indrelid
)
SELECT jsonb_build_object(
    'relations', COALESCE((
        SELECT jsonb_agg(jsonb_build_array(
            r.relname, r.relkind::text, r.relpersistence::text,
            r.relrowsecurity, r.relforcerowsecurity, r.relreplident::text,
            r.reloptions, am.amname, pg_get_partkeydef(r.oid),
            pg_get_expr(r.relpartbound, r.oid),
            CASE WHEN r.relkind IN ('v', 'm') THEN pg_get_viewdef(r.oid, false) END
        ) ORDER BY r.relname)
        FROM relations r LEFT JOIN pg_am am ON am.oid = r.relam
    ), '[]'::jsonb),
    'columns', COALESCE((
        SELECT jsonb_agg(jsonb_build_array(
            r.relname, a.attname, format_type(a.atttypid, a.atttypmod),
            a.attnotnull, a.attidentity::text, a.attgenerated::text,
            a.attstorage::text, a.attcompression::text,
            a.attstattarget, a.attoptions, a.attfdwoptions,
            col.collname, pg_get_expr(d.adbin, d.adrelid)
        ) ORDER BY r.relname, a.attnum)
        FROM relations r
        JOIN pg_attribute a ON a.attrelid = r.oid AND a.attnum > 0 AND NOT a.attisdropped
        LEFT JOIN pg_attrdef d ON d.adrelid = r.oid AND d.adnum = a.attnum
        LEFT JOIN pg_collation col ON col.oid = a.attcollation
    ), '[]'::jsonb),
    'constraints', COALESCE((
        -- PostgreSQL 18 retains generated NOT NULL names after column/table
        -- renames. Compare their column definition and flags, leaving names intact.
        SELECT jsonb_agg(jsonb_build_array(
            r.relname,
            CASE WHEN c.contype = 'n' THEN pg_get_constraintdef(c.oid, false) ELSE c.conname END,
            c.contype::text, c.convalidated,
            c.condeferrable, c.condeferred, c.connoinherit,
            CASE WHEN c.contype <> 'c' THEN pg_get_constraintdef(c.oid, false) END
        ) ORDER BY r.relname, c.contype,
            CASE WHEN c.contype = 'n' THEN pg_get_constraintdef(c.oid, false) ELSE c.conname END)
        FROM relations r JOIN pg_constraint c ON c.conrelid = r.oid
    ), '[]'::jsonb),
    'indexes', COALESCE((
        SELECT jsonb_agg(jsonb_build_array(
            relname,
            CASE WHEN indpred IS NULL THEN pg_get_indexdef(indexrelid)
                 ELSE left(pg_get_indexdef(indexrelid),
                           length(pg_get_indexdef(indexrelid)) - length(pg_get_expr(indpred, indrelid)) - 7)
            END,
            reloptions,
            indisunique, indisprimary, indisvalid, indisready,
            indislive, indisreplident, indnullsnotdistinct
        ) ORDER BY relname) FROM indexes
    ), '[]'::jsonb),
    'sequences', COALESCE((
        SELECT jsonb_agg(jsonb_build_array(
            r.relname, format_type(s.seqtypid, NULL), s.seqstart, s.seqincrement,
            s.seqmax, s.seqmin, s.seqcache, s.seqcycle, owner.relname, a.attname
        ) ORDER BY r.relname)
        FROM relations r JOIN pg_sequence s ON s.seqrelid = r.oid
        LEFT JOIN pg_depend d ON d.classid = 'pg_class'::regclass
            AND d.objid = r.oid AND d.deptype IN ('a', 'i')
        LEFT JOIN pg_class owner ON owner.oid = d.refobjid
        LEFT JOIN pg_attribute a ON a.attrelid = owner.oid AND a.attnum = d.refobjsubid
    ), '[]'::jsonb),
    'functions', COALESCE((
        SELECT jsonb_agg(jsonb_build_array(
            p.proname, pg_get_function_identity_arguments(p.oid), p.prokind::text,
            CASE WHEN p.prokind <> 'a' THEN pg_get_functiondef(p.oid) END
        ) ORDER BY p.proname, pg_get_function_identity_arguments(p.oid))
        FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
        WHERE n.nspname = current_schema()
          AND NOT EXISTS (
              SELECT 1 FROM pg_depend d JOIN pg_extension e ON e.oid = d.refobjid
              WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid
                AND d.refclassid = 'pg_extension'::regclass AND d.deptype = 'e'
                AND e.extname = 'pg_trgm'
          )
    ), '[]'::jsonb),
    'triggers', COALESCE((
        SELECT jsonb_agg(jsonb_build_array(
            r.relname, t.tgname, t.tgenabled::text, pg_get_triggerdef(t.oid, false)
        ) ORDER BY r.relname, t.tgname)
        FROM relations r JOIN pg_trigger t ON t.tgrelid = r.oid
        WHERE NOT t.tgisinternal
    ), '[]'::jsonb),
    'rules', COALESCE((
        SELECT jsonb_agg(jsonb_build_array(
            r.relname, w.rulename, w.ev_enabled::text, pg_get_ruledef(w.oid, false)
        ) ORDER BY r.relname, w.rulename)
        FROM relations r JOIN pg_rewrite w ON w.ev_class = r.oid
        WHERE w.rulename <> '_RETURN'
    ), '[]'::jsonb),
    'policies', COALESCE((
        SELECT jsonb_agg(jsonb_build_array(
            r.relname, p.polname, p.polcmd::text, p.polpermissive,
            ARRAY(SELECT CASE WHEN role_id = 0 THEN 'public' ELSE pg_get_userbyid(role_id) END
                  FROM unnest(p.polroles) role_id ORDER BY role_id),
            pg_get_expr(p.polqual, p.polrelid), pg_get_expr(p.polwithcheck, p.polrelid)
        ) ORDER BY r.relname, p.polname)
        FROM relations r JOIN pg_policy p ON p.polrelid = r.oid
    ), '[]'::jsonb),
    'types', COALESCE((
        SELECT jsonb_agg(jsonb_build_array(
            t.typname, t.typtype::text, format_type(t.typbasetype, t.typtypmod),
            t.typnotnull, t.typdefault,
            ARRAY(SELECT e.enumlabel FROM pg_enum e WHERE e.enumtypid = t.oid ORDER BY e.enumsortorder),
            ARRAY(SELECT pg_get_constraintdef(c.oid, false) FROM pg_constraint c
                  WHERE c.contypid = t.oid ORDER BY c.conname)
        ) ORDER BY t.typname)
        FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace
        WHERE n.nspname = current_schema() AND t.typtype IN ('d', 'e', 'r', 'm')
    ), '[]'::jsonb),
    'other_objects', COALESCE((
        SELECT jsonb_agg(jsonb_build_array(o.kind, o.name) ORDER BY o.kind, o.name)
        FROM (
            SELECT 'pg_type'::regclass AS catalog, oid, typnamespace AS namespace,
                   'base type' AS kind, typname AS name
            FROM pg_type WHERE typtype = 'b' AND typelem = 0
            UNION ALL SELECT 'pg_operator'::regclass, oid, oprnamespace, 'operator',
                oprname || '(' || format_type(oprleft, NULL) || ',' || format_type(oprright, NULL) || ')'
                FROM pg_operator
            UNION ALL SELECT 'pg_opclass'::regclass, oid, opcnamespace, 'operator class', opcname FROM pg_opclass
            UNION ALL SELECT 'pg_opfamily'::regclass, oid, opfnamespace, 'operator family', opfname FROM pg_opfamily
            UNION ALL SELECT 'pg_collation'::regclass, oid, collnamespace, 'collation', collname FROM pg_collation
            UNION ALL SELECT 'pg_conversion'::regclass, oid, connamespace, 'conversion', conname FROM pg_conversion
            UNION ALL SELECT 'pg_statistic_ext'::regclass, oid, stxnamespace, 'statistics', stxname FROM pg_statistic_ext
        ) o JOIN pg_namespace n ON n.oid = o.namespace
        WHERE n.nspname = current_schema()
          AND NOT EXISTS (
              SELECT 1 FROM pg_depend d JOIN pg_extension e ON e.oid = d.refobjid
              WHERE d.classid = o.catalog AND d.objid = o.oid
                AND d.refclassid = 'pg_extension'::regclass AND d.deptype = 'e'
                AND e.extname = 'pg_trgm'
          )
    ), '[]'::jsonb)
) AS inventory
