-- LATTICE Writer Lease v4 administrative rebind boundary.
--
-- Writer v3/v3-rebind are immutable retained schema-v6 assets. This successor
-- is installed while Store remains at exact schema v6, then called only by the
-- Store-owned schema-v7 migration transaction. The v7 digest below is exact
-- and must be re-pinned with the final 0008 manifest before acceptance.

CREATE PROCEDURE writer_lease.writer_lease_rebind_v4()
LANGUAGE plpgsql
SECURITY INVOKER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_writer_lease_rebind_v4$
DECLARE
    v_ledger_shape text;
BEGIN
    LOCK TABLE writer_lease.writer_lease_extension_identity,
               writer_lease.writer_lease_extension_ledger,
               writer_lease.writer_lease_heads,
               writer_lease.writer_lease_commands,
               writer_lease.writer_lease_transitions IN SHARE ROW EXCLUSIVE MODE;

    SELECT pg_catalog.string_agg(
        l.ledger_ordinal::text || ':' || l.event_kind::text || ':' ||
        l.extension_schema_version::text || ':' || l.global_schema_version::text,
        ',' ORDER BY l.ledger_ordinal)
      INTO v_ledger_shape
      FROM ONLY writer_lease.writer_lease_extension_ledger AS l;

    IF (
        session_user = 'lattice_migrator_login'
        AND pg_catalog.current_setting('role') = 'lattice_migrator'
        AND pg_catalog.current_setting('transaction_isolation') IN
            ('read committed', 'serializable')
        AND NOT pg_catalog.current_setting('transaction_read_only')::boolean
        AND pg_catalog.current_setting('synchronous_commit') = 'on'
        AND (SELECT pg_catalog.count(*) FROM ONLY control.runtime_admission AS a
              WHERE a.singleton AND a.admission_mode = 'STOPPED'
                AND a.daemon_instance_id IS NULL AND a.daemon_epoch IS NULL) = 1
        AND (SELECT pg_catalog.count(*) FROM ONLY writer_lease.writer_lease_heads AS h
              WHERE h.current_status IN ('ACTIVE','SUSPECT')) = 0
        AND (SELECT pg_catalog.count(*)
               FROM ONLY control.schema_compatibility AS c
              WHERE c.singleton
                AND c.manifest_sha256 =
                    '7e16a8eb119cf4db9910645cabffef8b99703b7dca8ed5e4a9e193fedcd8d44c'
                AND c.current_schema_version = 7
                AND c.min_reader = 7 AND c.max_reader = 7
                AND c.min_writer = 7 AND c.max_writer = 7) = 1
        AND (SELECT pg_catalog.count(*)
               FROM ONLY memory.codebase_memory_extension_identity AS m
              WHERE m.singleton AND m.extension_schema_version = 3
                AND m.extension_manifest_sha256 =
                    'd4cc712d262ae1f7c96bd65526eab611c90e193363afd865af2126307b2903f0') = 1
        AND (SELECT pg_catalog.count(*)
               FROM ONLY writer_lease.writer_lease_extension_identity AS w
               CROSS JOIN ONLY control.database_identity AS d
               CROSS JOIN ONLY memory.codebase_memory_extension_identity AS m
              WHERE w.singleton AND m.singleton
                AND w.extension_id = 'lattice-writer-lease'
                AND w.extension_schema_version = 4
                AND w.extension_path = 'db/extensions/writer-lease/v4.sql'
                AND w.extension_sql_sha256 =
                    '51996b50c9a7d3696f8319613d35acae6257c5802b63dc4a809873721a22da09'
                AND w.extension_manifest_sha256 =
                    '73d3e435c5923797076d30cea337d84b94b2e760db6e9727033b68ace592a229'
                AND w.database_uuid = d.database_uuid
                AND m.database_uuid = d.database_uuid
                AND w.database_identity_sha256 = m.database_identity_sha256
                AND ((w.global_schema_version = 6 AND w.global_manifest_sha256 =
                    '75189dea7cd2cb95b694bade467c2b5c40373436fb1b3d48e9017b50a9d206ae')
                  OR (w.global_schema_version = 7 AND w.global_manifest_sha256 =
                    '7e16a8eb119cf4db9910645cabffef8b99703b7dca8ed5e4a9e193fedcd8d44c'))
                AND w.required_memory_schema_version = 3
                AND w.required_memory_manifest_sha256 =
                    'd4cc712d262ae1f7c96bd65526eab611c90e193363afd865af2126307b2903f0') = 1
        AND v_ledger_shape IN (
            '1:INSTALLED:3:6,2:UPGRADED:4:6',
            '1:INSTALLED:3:6,2:UPGRADED:4:6,3:REBOUND:4:7',
            '1:INSTALLED:2:5,2:UPGRADED:3:5,3:REBOUND:3:6,4:UPGRADED:4:6',
            '1:INSTALLED:2:5,2:UPGRADED:3:5,3:REBOUND:3:6,4:UPGRADED:4:6,5:REBOUND:4:7',
            '1:INSTALLED:1:3,2:UPGRADED:2:3,3:REBOUND:2:5,4:UPGRADED:3:5,5:REBOUND:3:6,6:UPGRADED:4:6',
            '1:INSTALLED:1:3,2:UPGRADED:2:3,3:REBOUND:2:5,4:UPGRADED:3:5,5:REBOUND:3:6,6:UPGRADED:4:6,7:REBOUND:4:7'
        )
    ) IS NOT TRUE THEN
        RAISE EXCEPTION 'LATTICE_WRITER_LEASE_V4_REBIND_PRECONDITION_FAILED'
            USING ERRCODE = '55000';
    END IF;

    UPDATE ONLY writer_lease.writer_lease_extension_identity
       SET global_schema_version = 7,
           global_manifest_sha256 =
               '7e16a8eb119cf4db9910645cabffef8b99703b7dca8ed5e4a9e193fedcd8d44c'
     WHERE singleton AND extension_id = 'lattice-writer-lease'
       AND extension_schema_version = 4
       AND extension_path = 'db/extensions/writer-lease/v4.sql'
       AND extension_sql_sha256 =
           '51996b50c9a7d3696f8319613d35acae6257c5802b63dc4a809873721a22da09'
       AND extension_manifest_sha256 =
           '73d3e435c5923797076d30cea337d84b94b2e760db6e9727033b68ace592a229'
       AND global_schema_version = 6
       AND global_manifest_sha256 =
           '75189dea7cd2cb95b694bade467c2b5c40373436fb1b3d48e9017b50a9d206ae'
       AND required_memory_schema_version = 3
       AND required_memory_manifest_sha256 =
           'd4cc712d262ae1f7c96bd65526eab611c90e193363afd865af2126307b2903f0';

    INSERT INTO writer_lease.writer_lease_extension_ledger (
        ledger_ordinal, singleton, extension_id, extension_schema_version,
        extension_sql_sha256, extension_manifest_sha256, database_uuid,
        database_identity_sha256, global_schema_version, global_manifest_sha256,
        required_memory_schema_version, required_memory_manifest_sha256, event_kind
    )
    SELECT CASE pg_catalog.count(*) WHEN 2 THEN 3 WHEN 4 THEN 5 WHEN 6 THEN 7 END,
           w.singleton, w.extension_id, w.extension_schema_version,
           w.extension_sql_sha256, w.extension_manifest_sha256, w.database_uuid,
           w.database_identity_sha256, w.global_schema_version, w.global_manifest_sha256,
           w.required_memory_schema_version, w.required_memory_manifest_sha256, 'REBOUND'
      FROM ONLY writer_lease.writer_lease_extension_identity AS w
      CROSS JOIN ONLY writer_lease.writer_lease_extension_ledger AS l
     WHERE w.singleton AND w.extension_schema_version = 4
       AND w.global_schema_version = 7
       AND NOT EXISTS (
           SELECT 1 FROM ONLY writer_lease.writer_lease_extension_ledger AS existing
            WHERE existing.extension_schema_version = 4
              AND existing.global_schema_version = 7
              AND existing.event_kind = 'REBOUND')
     GROUP BY w.singleton, w.extension_id, w.extension_schema_version,
              w.extension_sql_sha256, w.extension_manifest_sha256, w.database_uuid,
              w.database_identity_sha256, w.global_schema_version, w.global_manifest_sha256,
              w.required_memory_schema_version, w.required_memory_manifest_sha256
    HAVING pg_catalog.count(*) IN (2, 4, 6);

    GRANT USAGE ON SCHEMA writer_lease TO lattice_runtime;
    GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_bind_runtime_v4(
        text,bigint,bytea,text,text,text,text,text) TO lattice_runtime;
    GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_for_update_v4(
        text,bytea,bytea,bytea,text) TO lattice_runtime;
    GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_commit_plan_v1(
        text,bigint,bytea,bigint,bytea,text,bytea,text,text,bigint,bytea,bytea,
        bytea,bytea,bigint,bigint,bigint,bytea,text,bytea,text,text,text,bytea,
        text,text,text,text,bigint,bytea,text,bigint,bigint,text,bigint,text,bytea,
        bytea,bytea,bytea,bytea,text,text,bytea,bytea,bytea,text,bytea
    ) TO lattice_runtime;
    GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_commands_v1(text)
        TO lattice_runtime;
    GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_current_v1(text)
        TO lattice_runtime;
    GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_assert_current_v1(
        text,text,text,text,bytea,text,text,text,text,bigint,bytea,text,bigint,bigint,bytea
    ) TO lattice_runtime;
    GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_transitions_v1(text)
        TO lattice_runtime;

    SELECT pg_catalog.string_agg(
        l.ledger_ordinal::text || ':' || l.event_kind::text || ':' ||
        l.extension_schema_version::text || ':' || l.global_schema_version::text,
        ',' ORDER BY l.ledger_ordinal)
      INTO v_ledger_shape
      FROM ONLY writer_lease.writer_lease_extension_ledger AS l;

    IF (
        (SELECT pg_catalog.count(*)
           FROM ONLY writer_lease.writer_lease_extension_identity AS w
          WHERE w.singleton AND w.extension_schema_version = 4
            AND w.global_schema_version = 7
            AND w.global_manifest_sha256 =
                '7e16a8eb119cf4db9910645cabffef8b99703b7dca8ed5e4a9e193fedcd8d44c') = 1
        AND v_ledger_shape IN (
            '1:INSTALLED:3:6,2:UPGRADED:4:6,3:REBOUND:4:7',
            '1:INSTALLED:2:5,2:UPGRADED:3:5,3:REBOUND:3:6,4:UPGRADED:4:6,5:REBOUND:4:7',
            '1:INSTALLED:1:3,2:UPGRADED:2:3,3:REBOUND:2:5,4:UPGRADED:3:5,5:REBOUND:3:6,6:UPGRADED:4:6,7:REBOUND:4:7'
        )
        AND pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE')
        AND (SELECT pg_catalog.count(*) FILTER (WHERE pg_catalog.has_function_privilege(
                    'lattice_runtime', p.oid, 'EXECUTE'))
               FROM pg_catalog.pg_proc AS p
               JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
              WHERE n.nspname = 'writer_lease') = 7
    ) IS NOT TRUE THEN
        RAISE EXCEPTION 'LATTICE_WRITER_LEASE_V4_REBIND_POSTCONDITION_FAILED'
            USING ERRCODE = '55000';
    END IF;
END;
$lattice_writer_lease_rebind_v4$;

REVOKE ALL ON PROCEDURE writer_lease.writer_lease_rebind_v4() FROM
    PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;

COMMENT ON PROCEDURE writer_lease.writer_lease_rebind_v4() IS
    'LATTICE_WRITER_LEASE_REBIND_V4';
