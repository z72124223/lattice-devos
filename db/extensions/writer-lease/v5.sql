-- LATTICE PostgreSQL Writer Lease append-only successor profile v5.
-- This Store-v7 successor adds an explicit PROCESS_HANDOFF transition while
-- retaining all v1-v4 bytes and all existing authority/history rows.

ALTER TABLE writer_lease.writer_lease_extension_identity
    DROP CONSTRAINT writer_lease_extension_identity_profile_v4,
    ADD CONSTRAINT writer_lease_extension_identity_profile_v5 CHECK (
        extension_id = 'lattice-writer-lease'
        AND (
            (extension_schema_version = 1
             AND extension_path = 'db/extensions/writer-lease/v1.sql'
             AND global_schema_version = 3
             AND required_memory_schema_version = 2)
            OR
            (extension_schema_version = 2
             AND extension_path = 'db/extensions/writer-lease/v2.sql'
             AND global_schema_version IN (3, 5)
             AND required_memory_schema_version = CASE global_schema_version
                 WHEN 3 THEN 2 WHEN 5 THEN 3 END)
            OR
            (extension_schema_version = 3
             AND extension_path = 'db/extensions/writer-lease/v3.sql'
             AND global_schema_version IN (5, 6)
             AND required_memory_schema_version = 3)
            OR
            (extension_schema_version = 4
             AND extension_path = 'db/extensions/writer-lease/v4.sql'
             AND global_schema_version IN (6, 7)
             AND required_memory_schema_version = 3)
            OR
            (extension_schema_version = 5
             AND extension_path = 'db/extensions/writer-lease/v5.sql'
             AND global_schema_version = 7
             AND required_memory_schema_version = 3)
        )
    );

ALTER TABLE writer_lease.writer_lease_extension_ledger
    DROP CONSTRAINT writer_lease_extension_ledger_profile_v4,
    ADD CONSTRAINT writer_lease_extension_ledger_profile_v5 CHECK (
        extension_id = 'lattice-writer-lease'
        AND (
            (ledger_ordinal = 1 AND extension_schema_version = 1
             AND global_schema_version = 3 AND required_memory_schema_version = 2
             AND event_kind = 'INSTALLED')
            OR
            (ledger_ordinal = 1 AND extension_schema_version = 2
             AND global_schema_version = 5 AND required_memory_schema_version = 3
             AND event_kind = 'INSTALLED')
            OR
            (ledger_ordinal = 1 AND extension_schema_version = 3
             AND global_schema_version = 6 AND required_memory_schema_version = 3
             AND event_kind = 'INSTALLED')
            OR
            (ledger_ordinal = 2 AND extension_schema_version = 2
             AND global_schema_version = 3 AND required_memory_schema_version = 2
             AND event_kind = 'UPGRADED')
            OR
            (ledger_ordinal = 2 AND extension_schema_version = 3
             AND global_schema_version = 5 AND required_memory_schema_version = 3
             AND event_kind = 'UPGRADED')
            OR
            (ledger_ordinal = 3 AND extension_schema_version = 2
             AND global_schema_version = 5 AND required_memory_schema_version = 3
             AND event_kind = 'REBOUND')
            OR
            (ledger_ordinal = 3 AND extension_schema_version = 3
             AND global_schema_version = 6 AND required_memory_schema_version = 3
             AND event_kind = 'REBOUND')
            OR
            (ledger_ordinal = 4 AND extension_schema_version = 3
             AND global_schema_version = 5 AND required_memory_schema_version = 3
             AND event_kind = 'UPGRADED')
            OR
            (ledger_ordinal = 5 AND extension_schema_version = 3
             AND global_schema_version = 6 AND required_memory_schema_version = 3
             AND event_kind = 'REBOUND')
            OR
            (ledger_ordinal = 2 AND extension_schema_version = 4
             AND global_schema_version = 6 AND required_memory_schema_version = 3
             AND event_kind = 'UPGRADED')
            OR
            (ledger_ordinal = 3 AND extension_schema_version = 4
             AND global_schema_version = 7 AND required_memory_schema_version = 3
             AND event_kind = 'REBOUND')
            OR
            (ledger_ordinal = 4 AND extension_schema_version = 4
             AND global_schema_version = 6 AND required_memory_schema_version = 3
             AND event_kind = 'UPGRADED')
            OR
            (ledger_ordinal = 5 AND extension_schema_version = 4
             AND global_schema_version = 7 AND required_memory_schema_version = 3
             AND event_kind = 'REBOUND')
            OR
            (ledger_ordinal = 6 AND extension_schema_version = 4
             AND global_schema_version = 6 AND required_memory_schema_version = 3
             AND event_kind = 'UPGRADED')
            OR
            (ledger_ordinal = 7 AND extension_schema_version = 4
             AND global_schema_version = 7 AND required_memory_schema_version = 3
             AND event_kind = 'REBOUND')
            OR
            (ledger_ordinal = 4 AND extension_schema_version = 5
             AND global_schema_version = 7 AND required_memory_schema_version = 3
             AND event_kind = 'UPGRADED')
            OR
            (ledger_ordinal = 6 AND extension_schema_version = 5
             AND global_schema_version = 7 AND required_memory_schema_version = 3
             AND event_kind = 'UPGRADED')
            OR
            (ledger_ordinal = 8 AND extension_schema_version = 5
             AND global_schema_version = 7 AND required_memory_schema_version = 3
             AND event_kind = 'UPGRADED')
        )
    );

ALTER TABLE writer_lease.writer_lease_transitions
    DROP CONSTRAINT writer_lease_transitions_identity,
    ADD CONSTRAINT writer_lease_transitions_identity_v5 CHECK (
        ordinal > 0
        AND command_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND transition_kind IN (
            'ACQUIRE', 'HEARTBEAT', 'MARK_SUSPECT', 'PROCESS_HANDOFF', 'RELEASE', 'REVOKE'
        )
        AND pg_catalog.octet_length(transition_bytes) BETWEEN 1 AND 2097152
        AND pg_catalog.octet_length(transition_digest) = 32
    );

CREATE FUNCTION writer_lease.writer_lease_bind_runtime_v5(
    p_expected_daemon_instance_id text,
    p_expected_daemon_epoch bigint,
    p_expected_admission_observation_digest bytea,
    p_database_identity_sha256 text,
    p_global_manifest_sha256 text,
    p_memory_manifest_sha256 text,
    p_extension_sql_sha256 text,
    p_extension_manifest_sha256 text
)
RETURNS TABLE (
    daemon_instance_id text,
    daemon_epoch bigint,
    admission_observation_digest bytea
)
LANGUAGE plpgsql
STABLE
PARALLEL SAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_writer_lease_bind_runtime_v5$
DECLARE
    v_count bigint;
    v_admission_count bigint;
    v_ledger_shape text;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'repeatable read'
       OR NOT pg_catalog.current_setting('transaction_read_only')::boolean
       OR p_expected_daemon_instance_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR p_expected_daemon_epoch IS NULL OR p_expected_daemon_epoch <= 0
       OR pg_catalog.octet_length(p_expected_admission_observation_digest) <> 32
       OR p_database_identity_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_memory_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_extension_sql_sha256 !~ '^[0-9a-f]{64}$'
       OR p_extension_manifest_sha256 !~ '^[0-9a-f]{64}$'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL01', MESSAGE = 'invalid writer lease runtime binding';
    END IF;

    SELECT pg_catalog.string_agg(
        l.ledger_ordinal::text || ':' || l.event_kind::text || ':' ||
        l.extension_schema_version::text || ':' || l.global_schema_version::text,
        ',' ORDER BY l.ledger_ordinal)
      INTO v_ledger_shape
      FROM ONLY writer_lease.writer_lease_extension_ledger AS l;
    IF (v_ledger_shape IN (
        '1:INSTALLED:3:6,2:UPGRADED:4:6,3:REBOUND:4:7,4:UPGRADED:5:7',
        '1:INSTALLED:2:5,2:UPGRADED:3:5,3:REBOUND:3:6,4:UPGRADED:4:6,5:REBOUND:4:7,6:UPGRADED:5:7',
        '1:INSTALLED:1:3,2:UPGRADED:2:3,3:REBOUND:2:5,4:UPGRADED:3:5,5:REBOUND:3:6,6:UPGRADED:4:6,7:REBOUND:4:7,8:UPGRADED:5:7'
    )) IS NOT TRUE THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL02', MESSAGE = 'writer lease extension history mismatch';
    END IF;

    SELECT pg_catalog.count(*) INTO v_count
      FROM ONLY writer_lease.writer_lease_extension_identity AS w
      CROSS JOIN ONLY control.database_identity AS d
      CROSS JOIN ONLY control.schema_compatibility AS c
      CROSS JOIN ONLY memory.codebase_memory_extension_identity AS m
     WHERE w.singleton
       AND w.extension_id = 'lattice-writer-lease'
       AND w.extension_schema_version = 5
       AND w.extension_path = 'db/extensions/writer-lease/v5.sql'
       AND w.global_schema_version = 7
       AND w.required_memory_schema_version = 3
       AND w.database_uuid = d.database_uuid
       AND m.database_uuid = d.database_uuid
       AND w.database_identity_sha256 = p_database_identity_sha256
       AND m.database_identity_sha256 = p_database_identity_sha256
       AND w.global_schema_version = c.current_schema_version
       AND w.global_manifest_sha256 = p_global_manifest_sha256
       AND c.manifest_sha256 = p_global_manifest_sha256
       AND w.required_memory_schema_version = m.extension_schema_version
       AND w.required_memory_manifest_sha256 = p_memory_manifest_sha256
       AND m.extension_manifest_sha256 = p_memory_manifest_sha256
       AND w.extension_sql_sha256 = p_extension_sql_sha256
       AND w.extension_manifest_sha256 = p_extension_manifest_sha256
       AND EXISTS (
           SELECT 1 FROM ONLY writer_lease.writer_lease_extension_ledger AS l
            WHERE l.singleton = w.singleton
              AND l.extension_id = w.extension_id
              AND l.extension_schema_version = w.extension_schema_version
              AND l.extension_sql_sha256 = w.extension_sql_sha256
              AND l.extension_manifest_sha256 = w.extension_manifest_sha256
              AND l.database_uuid = w.database_uuid
              AND l.database_identity_sha256 = w.database_identity_sha256
              AND l.global_schema_version = w.global_schema_version
              AND l.global_manifest_sha256 = w.global_manifest_sha256
              AND l.required_memory_schema_version = w.required_memory_schema_version
              AND l.required_memory_manifest_sha256 = w.required_memory_manifest_sha256
              AND l.event_kind = 'UPGRADED'
       );
    IF v_count IS DISTINCT FROM 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL02', MESSAGE = 'writer lease runtime binding mismatch';
    END IF;

    SELECT pg_catalog.count(*) INTO v_admission_count
      FROM ONLY control.runtime_admission AS a
     WHERE a.singleton AND a.admission_mode = 'ACTIVE'
       AND a.daemon_instance_id = p_expected_daemon_instance_id
       AND a.daemon_epoch = p_expected_daemon_epoch
       AND a.observation_digest = p_expected_admission_observation_digest;
    IF v_admission_count IS DISTINCT FROM 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL06', MESSAGE = 'writer lease daemon binding mismatch';
    END IF;
    RETURN QUERY SELECT a.daemon_instance_id::text, a.daemon_epoch, a.observation_digest
      FROM ONLY control.runtime_admission AS a WHERE a.singleton;
END;
$lattice_writer_lease_bind_runtime_v5$;

CREATE FUNCTION writer_lease.writer_lease_load_for_update_v5(
    p_project_id text,
    p_vacant_snapshot_bytes bytea,
    p_vacant_snapshot_bytes_sha256 bytea,
    p_vacant_snapshot_digest bytea,
    p_command_id text
)
RETURNS TABLE (
    row_version bigint, snapshot_bytes bytea, snapshot_bytes_sha256 bytea,
    snapshot_digest bytea, fencing_high_water bigint, lease_revision bigint,
    command_high_water bigint, command_tail_digest bytea, current_status text,
    current_receipt_digest bytea, current_project_snapshot_id text,
    current_task_id text, current_task_revision text, current_task_spec_digest bytea,
    current_attempt_id text, current_lease_id text, current_lease_holder_id text,
    current_worktree_id text, current_holder_process_id bigint,
    current_holder_process_start_identity bytea, current_daemon_instance_id text,
    current_daemon_epoch bigint, current_fencing_token bigint,
    current_expires_at text, observed_at text, time_observation_digest bytea,
    admission_mode text, daemon_instance_id text, daemon_epoch bigint,
    admission_observation_digest bytea, existing_repository_request_bytes bytea,
    existing_repository_request_sha256 bytea
)
LANGUAGE plpgsql
VOLATILE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_writer_lease_load_for_update_v5$
DECLARE
    v_identity_count bigint;
    v_ledger_shape text;
    v_admission control.runtime_admission%ROWTYPE;
    v_head writer_lease.writer_lease_heads%ROWTYPE;
    v_observed_at text;
    v_time_digest bytea;
    v_repository_request_bytes bytea;
    v_repository_request_sha256 bytea;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
       OR p_project_id IS NULL OR p_project_id !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
       OR p_vacant_snapshot_bytes IS NULL
       OR pg_catalog.octet_length(p_vacant_snapshot_bytes) NOT BETWEEN 1 AND 16777216
       OR pg_catalog.octet_length(p_vacant_snapshot_bytes_sha256) <> 32
       OR p_vacant_snapshot_bytes_sha256 <> pg_catalog.sha256(p_vacant_snapshot_bytes)
       OR pg_catalog.octet_length(p_vacant_snapshot_digest) <> 32
       OR p_command_id IS NULL
       OR p_command_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL01', MESSAGE = 'invalid writer lease load boundary';
    END IF;

    SELECT pg_catalog.string_agg(
        l.ledger_ordinal::text || ':' || l.event_kind::text || ':' ||
        l.extension_schema_version::text || ':' || l.global_schema_version::text,
        ',' ORDER BY l.ledger_ordinal)
      INTO v_ledger_shape
      FROM ONLY writer_lease.writer_lease_extension_ledger AS l;
    IF (v_ledger_shape IN (
        '1:INSTALLED:3:6,2:UPGRADED:4:6,3:REBOUND:4:7,4:UPGRADED:5:7',
        '1:INSTALLED:2:5,2:UPGRADED:3:5,3:REBOUND:3:6,4:UPGRADED:4:6,5:REBOUND:4:7,6:UPGRADED:5:7',
        '1:INSTALLED:1:3,2:UPGRADED:2:3,3:REBOUND:2:5,4:UPGRADED:3:5,5:REBOUND:3:6,6:UPGRADED:4:6,7:REBOUND:4:7,8:UPGRADED:5:7'
    )) IS NOT TRUE THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL02', MESSAGE = 'writer lease extension history mismatch';
    END IF;
    SELECT pg_catalog.count(*) INTO v_identity_count
      FROM ONLY writer_lease.writer_lease_extension_identity AS w
      CROSS JOIN ONLY control.database_identity AS d
      CROSS JOIN ONLY control.schema_compatibility AS c
      CROSS JOIN ONLY memory.codebase_memory_extension_identity AS m
     WHERE w.singleton AND w.extension_id = 'lattice-writer-lease'
       AND w.extension_schema_version = 5
       AND w.extension_path = 'db/extensions/writer-lease/v5.sql'
       AND w.global_schema_version = 7 AND w.required_memory_schema_version = 3
       AND w.database_uuid = d.database_uuid AND m.database_uuid = d.database_uuid
       AND w.database_identity_sha256 = m.database_identity_sha256
       AND w.global_schema_version = c.current_schema_version
       AND w.global_manifest_sha256 = c.manifest_sha256
       AND w.required_memory_schema_version = m.extension_schema_version
       AND w.required_memory_manifest_sha256 = m.extension_manifest_sha256
       AND EXISTS (
           SELECT 1 FROM ONLY writer_lease.writer_lease_extension_ledger AS l
            WHERE l.singleton = w.singleton AND l.extension_id = w.extension_id
              AND l.extension_schema_version = w.extension_schema_version
              AND l.extension_sql_sha256 = w.extension_sql_sha256
              AND l.extension_manifest_sha256 = w.extension_manifest_sha256
              AND l.database_uuid = w.database_uuid
              AND l.database_identity_sha256 = w.database_identity_sha256
              AND l.global_schema_version = w.global_schema_version
              AND l.global_manifest_sha256 = w.global_manifest_sha256
              AND l.required_memory_schema_version = w.required_memory_schema_version
              AND l.required_memory_manifest_sha256 = w.required_memory_manifest_sha256
              AND l.event_kind = 'UPGRADED'
       );
    IF v_identity_count IS DISTINCT FROM 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL02', MESSAGE = 'writer lease extension identity mismatch';
    END IF;

    SELECT a.* INTO STRICT v_admission
      FROM ONLY control.runtime_admission AS a WHERE a.singleton FOR SHARE OF a;
    INSERT INTO writer_lease.writer_lease_heads (
        project_id, row_version, snapshot_schema_version, snapshot_bytes,
        snapshot_bytes_sha256, snapshot_digest, fencing_high_water,
        lease_revision, command_high_water
    ) VALUES (
        p_project_id, 0, 1, p_vacant_snapshot_bytes,
        p_vacant_snapshot_bytes_sha256, p_vacant_snapshot_digest, 0, 0, 0
    ) ON CONFLICT (project_id) DO NOTHING;
    SELECT h.* INTO STRICT v_head FROM ONLY writer_lease.writer_lease_heads AS h
     WHERE h.project_id = p_project_id FOR UPDATE OF h;
    SELECT c.repository_request_bytes, c.repository_request_sha256
      INTO v_repository_request_bytes, v_repository_request_sha256
      FROM ONLY writer_lease.writer_lease_commands AS c
     WHERE c.project_id = p_project_id AND c.command_id = p_command_id;
    v_observed_at := pg_catalog.to_char(
        pg_catalog.transaction_timestamp() AT TIME ZONE 'UTC',
        'YYYY-MM-DD"T"HH24:MI:SS.US"Z"');
    v_observed_at := pg_catalog.regexp_replace(v_observed_at, '(\.[0-9]*[1-9])0+Z$', '\1Z');
    v_observed_at := pg_catalog.regexp_replace(v_observed_at, '\.0+Z$', 'Z');
    v_time_digest := pg_catalog.sha256(
        pg_catalog.convert_to('LATTICE_WRITER_LEASE_TIME_V1', 'UTF8')
        || pg_catalog.decode('00', 'hex') || pg_catalog.convert_to(v_observed_at, 'UTF8'));
    RETURN QUERY SELECT
        v_head.row_version, v_head.snapshot_bytes, v_head.snapshot_bytes_sha256,
        v_head.snapshot_digest, v_head.fencing_high_water, v_head.lease_revision,
        v_head.command_high_water, v_head.command_tail_digest,
        v_head.current_status::text, v_head.current_receipt_digest,
        v_head.current_project_snapshot_id::text, v_head.current_task_id::text,
        v_head.current_task_revision::text, v_head.current_task_spec_digest,
        v_head.current_attempt_id::text, v_head.current_lease_id::text,
        v_head.current_lease_holder_id::text, v_head.current_worktree_id::text,
        v_head.current_holder_process_id, v_head.current_holder_process_start_identity,
        v_head.current_daemon_instance_id::text, v_head.current_daemon_epoch,
        v_head.current_fencing_token, v_head.current_expires_at, v_observed_at,
        v_time_digest, v_admission.admission_mode::text,
        v_admission.daemon_instance_id::text, v_admission.daemon_epoch,
        v_admission.observation_digest, v_repository_request_bytes,
        v_repository_request_sha256;
END;
$lattice_writer_lease_load_for_update_v5$;

REVOKE ALL ON SCHEMA writer_lease FROM
    PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA writer_lease FROM
    PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;

COMMENT ON SCHEMA writer_lease IS 'LATTICE_WRITER_LEASE_SCHEMA_V5';
COMMENT ON TABLE writer_lease.writer_lease_extension_identity IS
    'LATTICE_WRITER_LEASE_EXTENSION_IDENTITY_V5';
COMMENT ON TABLE writer_lease.writer_lease_extension_ledger IS
    'LATTICE_WRITER_LEASE_EXTENSION_LEDGER_V5';
COMMENT ON FUNCTION writer_lease.writer_lease_bind_runtime_v5(
    text,bigint,bytea,text,text,text,text,text
) IS 'PHASE4_EXACT_PROCESS_HANDOFF';
COMMENT ON FUNCTION writer_lease.writer_lease_load_for_update_v5(
    text,bytea,bytea,bytea,text
) IS 'PHASE4_EXACT_PROCESS_HANDOFF';
