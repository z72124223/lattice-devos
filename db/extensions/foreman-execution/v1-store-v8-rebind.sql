-- LATTICE managed foreman execution extension v1 Store-v8 rebind.
-- The explicit Rust runner owns the transaction and admits only the exact
-- Store-v7 predecessor. Business tables, capabilities, and v1 provenance stay
-- immutable; only the global binding, append-only ledger, schema comment, and
-- identity reader advance to the exact Store-v8 successor.

COMMENT ON SCHEMA foreman_execution IS
    'LATTICE_FOREMAN_EXECUTION_EXTENSION_V1_STORE_V8';

ALTER TABLE foreman_execution.extension_identity
    DROP CONSTRAINT extension_identity_exact_profile;

UPDATE ONLY foreman_execution.extension_identity
   SET global_schema_version = 8,
       global_manifest_sha256 =
           '2b1fcbbc81261c28ab06ac3180f75c2ee458e57a4adc7e49bc399209f421de60'
 WHERE singleton
   AND extension_id = 'lattice-postgres-foreman'
   AND extension_schema_version = 1
   AND extension_path = 'db/extensions/foreman-execution/v1.sql'
   AND global_schema_version = 7
   AND global_manifest_sha256 =
       '584a446464ab2f7ebd8b85543ba36a6d52b0a708502c39d2653b8814d84313f8';

ALTER TABLE foreman_execution.extension_identity
    ADD CONSTRAINT extension_identity_exact_profile CHECK (
        extension_id = 'lattice-postgres-foreman'
        AND extension_schema_version = 1
        AND extension_path = 'db/extensions/foreman-execution/v1.sql'
        AND extension_sql_bytes > 0
        AND database_name ~ '^[a-z][a-z0-9_]{2,62}$'
        AND database_name <> 'postgres'
        AND database_uuid::text ~ '^[0-9a-f]{8}-[0-9a-f]{4}-8[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$'
        AND global_schema_version = 8
        AND global_manifest_sha256 =
            '2b1fcbbc81261c28ab06ac3180f75c2ee458e57a4adc7e49bc399209f421de60'
    );

ALTER TABLE foreman_execution.extension_ledger
    DROP CONSTRAINT extension_ledger_exact_install;

ALTER TABLE foreman_execution.extension_ledger
    ADD CONSTRAINT extension_ledger_exact_history CHECK (
        extension_id = 'lattice-postgres-foreman'
        AND extension_schema_version = 1
        AND (
            (
                ledger_ordinal = 1
                AND event_kind = 'INSTALLED'
                AND global_schema_version = 7
                AND global_manifest_sha256 =
                    '584a446464ab2f7ebd8b85543ba36a6d52b0a708502c39d2653b8814d84313f8'
            )
            OR
            (
                ledger_ordinal = 2
                AND event_kind = 'REBOUND'
                AND global_schema_version = 8
                AND global_manifest_sha256 =
                    '2b1fcbbc81261c28ab06ac3180f75c2ee458e57a4adc7e49bc399209f421de60'
            )
        )
    );

INSERT INTO foreman_execution.extension_ledger (
    ledger_ordinal, extension_id, extension_schema_version,
    extension_sql_sha256, extension_manifest_sha256, database_uuid,
    database_identity_sha256, global_schema_version,
    global_manifest_sha256, event_kind
)
SELECT 2, l.extension_id, l.extension_schema_version,
       l.extension_sql_sha256, l.extension_manifest_sha256, l.database_uuid,
       l.database_identity_sha256, 8,
       '2b1fcbbc81261c28ab06ac3180f75c2ee458e57a4adc7e49bc399209f421de60',
       'REBOUND'
  FROM ONLY foreman_execution.extension_ledger AS l
 WHERE l.ledger_ordinal = 1
   AND l.extension_id = 'lattice-postgres-foreman'
   AND l.extension_schema_version = 1
   AND l.event_kind = 'INSTALLED'
   AND l.global_schema_version = 7
   AND l.global_manifest_sha256 =
       '584a446464ab2f7ebd8b85543ba36a6d52b0a708502c39d2653b8814d84313f8';

CREATE OR REPLACE FUNCTION foreman_execution.read_extension_identity_v1()
RETURNS TABLE(
    extension_id text,
    extension_schema_version smallint,
    extension_path text,
    extension_sql_bytes bigint,
    extension_sql_sha256 text,
    extension_manifest_sha256 text,
    database_name text,
    database_uuid text,
    database_identity_sha256 text,
    global_schema_version smallint,
    global_manifest_sha256 text
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT i.extension_id::text, i.extension_schema_version,
           i.extension_path::text, i.extension_sql_bytes,
           pg_catalog.btrim(i.extension_sql_sha256)::text,
           pg_catalog.btrim(i.extension_manifest_sha256)::text,
           i.database_name::text, i.database_uuid::text,
           pg_catalog.btrim(i.database_identity_sha256)::text,
           i.global_schema_version,
           pg_catalog.btrim(i.global_manifest_sha256)::text
      FROM ONLY foreman_execution.extension_identity AS i
      JOIN ONLY foreman_execution.extension_ledger AS l1
        ON l1.ledger_ordinal = 1
       AND l1.extension_id = i.extension_id
       AND l1.extension_schema_version = i.extension_schema_version
       AND l1.extension_sql_sha256 = i.extension_sql_sha256
       AND l1.extension_manifest_sha256 = i.extension_manifest_sha256
       AND l1.database_uuid = i.database_uuid
       AND l1.database_identity_sha256 = i.database_identity_sha256
       AND l1.global_schema_version = 7
       AND l1.global_manifest_sha256 =
           '584a446464ab2f7ebd8b85543ba36a6d52b0a708502c39d2653b8814d84313f8'
       AND l1.event_kind = 'INSTALLED'
      JOIN ONLY foreman_execution.extension_ledger AS l2
        ON l2.ledger_ordinal = 2
       AND l2.extension_id = i.extension_id
       AND l2.extension_schema_version = i.extension_schema_version
       AND l2.extension_sql_sha256 = i.extension_sql_sha256
       AND l2.extension_manifest_sha256 = i.extension_manifest_sha256
       AND l2.database_uuid = i.database_uuid
       AND l2.database_identity_sha256 = i.database_identity_sha256
       AND l2.global_schema_version = i.global_schema_version
       AND l2.global_manifest_sha256 = i.global_manifest_sha256
       AND l2.event_kind = 'REBOUND'
      JOIN ONLY control.database_identity AS d
        ON d.singleton AND d.database_uuid = i.database_uuid
      JOIN ONLY control.schema_compatibility AS c
        ON c.singleton
       AND c.current_schema_version = 8
       AND c.min_reader = 8 AND c.max_reader = 8
       AND c.min_writer = 8 AND c.max_writer = 8
       AND pg_catalog.btrim(c.manifest_sha256) = i.global_manifest_sha256
      WHERE i.singleton
        AND i.global_schema_version = 8
        AND i.global_manifest_sha256 =
            '2b1fcbbc81261c28ab06ac3180f75c2ee458e57a4adc7e49bc399209f421de60'
        AND i.database_name = pg_catalog.current_database()
$$;
