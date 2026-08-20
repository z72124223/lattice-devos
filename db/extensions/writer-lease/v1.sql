-- LATTICE PostgreSQL Writer Lease extension profile v1.
-- The explicit administrative runner owns the transaction boundary.
-- Global Store migrations 0001-0005 and their manifest do not include this profile.
-- Pure lattice-writer-lease plan/apply/verify remains the sole semantic owner.

CREATE SCHEMA writer_lease AUTHORIZATION lattice_migrator;

CREATE TABLE writer_lease.writer_lease_extension_identity (
    singleton boolean PRIMARY KEY DEFAULT true,
    extension_id varchar(64) NOT NULL,
    extension_schema_version smallint NOT NULL,
    extension_path varchar(256) NOT NULL,
    extension_sql_sha256 char(64) NOT NULL,
    extension_manifest_sha256 char(64) NOT NULL,
    database_uuid uuid NOT NULL,
    database_identity_sha256 char(64) NOT NULL,
    global_schema_version smallint NOT NULL,
    global_manifest_sha256 char(64) NOT NULL,
    required_memory_schema_version smallint NOT NULL,
    required_memory_manifest_sha256 char(64) NOT NULL,
    installed_at timestamp with time zone NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    CONSTRAINT writer_lease_extension_identity_singleton CHECK (singleton),
    CONSTRAINT writer_lease_extension_identity_fixed CHECK (
        extension_id = 'lattice-writer-lease'
        AND extension_schema_version = 1
        AND extension_path = 'db/extensions/writer-lease/v1.sql'
        AND global_schema_version = 3
        AND required_memory_schema_version = 2
    ),
    CONSTRAINT writer_lease_extension_identity_hashes CHECK (
        extension_sql_sha256 ~ '^[0-9a-f]{64}$'
        AND extension_sql_sha256 <> pg_catalog.repeat('0', 64)
        AND extension_manifest_sha256 ~ '^[0-9a-f]{64}$'
        AND extension_manifest_sha256 <> pg_catalog.repeat('0', 64)
        AND database_identity_sha256 ~ '^[0-9a-f]{64}$'
        AND database_identity_sha256 <> pg_catalog.repeat('0', 64)
        AND global_manifest_sha256 ~ '^[0-9a-f]{64}$'
        AND global_manifest_sha256 <> pg_catalog.repeat('0', 64)
        AND required_memory_manifest_sha256 ~ '^[0-9a-f]{64}$'
        AND required_memory_manifest_sha256 <> pg_catalog.repeat('0', 64)
    )
);

CREATE TABLE writer_lease.writer_lease_extension_ledger (
    ledger_ordinal smallint PRIMARY KEY,
    singleton boolean NOT NULL UNIQUE,
    extension_id varchar(64) NOT NULL,
    extension_schema_version smallint NOT NULL,
    extension_sql_sha256 char(64) NOT NULL,
    extension_manifest_sha256 char(64) NOT NULL,
    database_uuid uuid NOT NULL,
    database_identity_sha256 char(64) NOT NULL,
    global_schema_version smallint NOT NULL,
    global_manifest_sha256 char(64) NOT NULL,
    required_memory_schema_version smallint NOT NULL,
    required_memory_manifest_sha256 char(64) NOT NULL,
    event_kind varchar(16) NOT NULL,
    recorded_at timestamp with time zone NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    CONSTRAINT writer_lease_extension_ledger_single CHECK (
        ledger_ordinal = 1 AND singleton AND event_kind = 'INSTALLED'
    ),
    CONSTRAINT writer_lease_extension_ledger_fixed CHECK (
        extension_id = 'lattice-writer-lease'
        AND extension_schema_version = 1
        AND global_schema_version = 3
        AND required_memory_schema_version = 2
    ),
    CONSTRAINT writer_lease_extension_ledger_identity_fk FOREIGN KEY (singleton)
        REFERENCES writer_lease.writer_lease_extension_identity (singleton)
);

CREATE TABLE writer_lease.writer_lease_heads (
    project_id varchar(64) PRIMARY KEY,
    row_version bigint NOT NULL,
    snapshot_schema_version smallint NOT NULL,
    snapshot_bytes bytea NOT NULL,
    snapshot_bytes_sha256 bytea NOT NULL,
    snapshot_digest bytea NOT NULL,
    fencing_high_water bigint NOT NULL,
    lease_revision bigint NOT NULL,
    command_high_water bigint NOT NULL,
    command_tail_digest bytea,
    current_status varchar(8),
    current_receipt_digest bytea,
    current_project_snapshot_id varchar(128),
    current_task_id varchar(128),
    current_task_revision varchar(20),
    current_task_spec_digest bytea,
    current_attempt_id varchar(128),
    current_lease_id varchar(128),
    current_lease_holder_id varchar(128),
    current_worktree_id varchar(128),
    current_holder_process_id bigint,
    current_holder_process_start_identity bytea,
    current_daemon_instance_id varchar(128),
    current_daemon_epoch bigint,
    current_fencing_token bigint,
    current_expires_at text,
    updated_at timestamp with time zone NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    CONSTRAINT writer_lease_heads_project CHECK (
        project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'
    ),
    CONSTRAINT writer_lease_heads_versions CHECK (
        row_version >= 0
        AND snapshot_schema_version = 1
        AND fencing_high_water >= 0
        AND lease_revision >= 0
        AND command_high_water >= 0
    ),
    CONSTRAINT writer_lease_heads_snapshot CHECK (
        pg_catalog.octet_length(snapshot_bytes) BETWEEN 1 AND 16777216
        AND pg_catalog.octet_length(snapshot_bytes_sha256) = 32
        AND snapshot_bytes_sha256 = pg_catalog.sha256(snapshot_bytes)
        AND pg_catalog.octet_length(snapshot_digest) = 32
        AND snapshot_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    ),
    CONSTRAINT writer_lease_heads_command_tail CHECK (
        (command_high_water = 0 AND command_tail_digest IS NULL)
        OR
        (command_high_water > 0 AND pg_catalog.octet_length(command_tail_digest) = 32)
    ),
    CONSTRAINT writer_lease_heads_current_closed CHECK (
        (
            current_status IS NULL
            AND current_receipt_digest IS NULL
            AND current_project_snapshot_id IS NULL
            AND current_task_id IS NULL
            AND current_task_revision IS NULL
            AND current_task_spec_digest IS NULL
            AND current_attempt_id IS NULL
            AND current_lease_id IS NULL
            AND current_lease_holder_id IS NULL
            AND current_worktree_id IS NULL
            AND current_holder_process_id IS NULL
            AND current_holder_process_start_identity IS NULL
            AND current_daemon_instance_id IS NULL
            AND current_daemon_epoch IS NULL
            AND current_fencing_token IS NULL
            AND current_expires_at IS NULL
        )
        OR
        (
            current_status IN ('ACTIVE', 'SUSPECT')
            AND pg_catalog.octet_length(current_receipt_digest) = 32
            AND current_project_snapshot_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
            AND current_task_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
            AND CASE
                WHEN current_task_revision ~ '^[1-9][0-9]{0,19}$'
                THEN current_task_revision::numeric <= 18446744073709551615
                ELSE false
            END
            AND pg_catalog.octet_length(current_task_spec_digest) = 32
            AND current_attempt_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
            AND current_lease_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
            AND current_lease_holder_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
            AND current_worktree_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
            AND current_holder_process_id > 0
            AND pg_catalog.octet_length(current_holder_process_start_identity) = 32
            AND current_daemon_instance_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
            AND current_daemon_epoch > 0
            AND current_fencing_token > 0
            AND current_fencing_token = fencing_high_water
            AND pg_catalog.octet_length(current_expires_at) BETWEEN 20 AND 40
        )
    )
);

CREATE TABLE writer_lease.writer_lease_commands (
    project_id varchar(64) NOT NULL,
    ordinal bigint NOT NULL,
    command_id varchar(128) NOT NULL,
    repository_request_bytes bytea NOT NULL,
    repository_request_sha256 bytea NOT NULL,
    request_bytes bytea NOT NULL,
    request_digest bytea NOT NULL,
    previous_receipt_digest bytea,
    outcome varchar(8) NOT NULL,
    denial_reason varchar(40),
    transition_digest bytea,
    receipt_bytes bytea NOT NULL,
    receipt_digest bytea NOT NULL,
    PRIMARY KEY (project_id, ordinal),
    CONSTRAINT writer_lease_commands_project_fk FOREIGN KEY (project_id)
        REFERENCES writer_lease.writer_lease_heads (project_id),
    CONSTRAINT writer_lease_commands_id_unique UNIQUE (project_id, command_id),
    CONSTRAINT writer_lease_commands_receipt_unique UNIQUE (project_id, receipt_digest),
    CONSTRAINT writer_lease_commands_ordinal CHECK (ordinal > 0),
    CONSTRAINT writer_lease_commands_id CHECK (
        command_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
    ),
    CONSTRAINT writer_lease_commands_bytes CHECK (
        pg_catalog.octet_length(repository_request_bytes) BETWEEN 1 AND 1048576
        AND pg_catalog.octet_length(repository_request_sha256) = 32
        AND repository_request_sha256 = pg_catalog.sha256(repository_request_bytes)
        AND pg_catalog.octet_length(request_bytes) BETWEEN 1 AND 1048576
        AND pg_catalog.octet_length(receipt_bytes) BETWEEN 1 AND 2097152
    ),
    CONSTRAINT writer_lease_commands_digests CHECK (
        pg_catalog.octet_length(request_digest) = 32
        AND (previous_receipt_digest IS NULL OR pg_catalog.octet_length(previous_receipt_digest) = 32)
        AND (transition_digest IS NULL OR pg_catalog.octet_length(transition_digest) = 32)
        AND pg_catalog.octet_length(receipt_digest) = 32
    ),
    CONSTRAINT writer_lease_commands_outcome CHECK (
        (outcome = 'APPLIED' AND denial_reason IS NULL AND transition_digest IS NOT NULL)
        OR
        (outcome = 'DENIED' AND denial_reason IN (
            'STALE_HEAD', 'WRITER_ALREADY_HELD', 'LEASE_VACANT', 'INVALID_STATE',
            'ADMISSION_DENIED', 'RUNTIME_MISMATCH', 'HEARTBEAT_REJECTED',
            'NOT_EXPIRED', 'RECOVERY_EVIDENCE_MISMATCH', 'COUNTER_EXHAUSTED'
        ) AND transition_digest IS NULL)
    )
);

CREATE TABLE writer_lease.writer_lease_transitions (
    project_id varchar(64) NOT NULL,
    ordinal bigint NOT NULL,
    command_id varchar(128) NOT NULL,
    transition_kind varchar(16) NOT NULL,
    transition_bytes bytea NOT NULL,
    transition_digest bytea NOT NULL,
    PRIMARY KEY (project_id, ordinal),
    CONSTRAINT writer_lease_transitions_command_fk FOREIGN KEY (project_id, ordinal)
        REFERENCES writer_lease.writer_lease_commands (project_id, ordinal),
    CONSTRAINT writer_lease_transitions_digest_unique UNIQUE (project_id, transition_digest),
    CONSTRAINT writer_lease_transitions_identity CHECK (
        ordinal > 0
        AND command_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND transition_kind IN ('ACQUIRE', 'HEARTBEAT', 'MARK_SUSPECT', 'RELEASE', 'REVOKE')
        AND pg_catalog.octet_length(transition_bytes) BETWEEN 1 AND 2097152
        AND pg_catalog.octet_length(transition_digest) = 32
    )
);

CREATE FUNCTION writer_lease.writer_lease_bind_runtime_v1(
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
AS $lattice_writer_lease_bind_runtime_v1$
DECLARE
    v_count bigint;
    v_admission_count bigint;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'repeatable read'
       OR NOT pg_catalog.current_setting('transaction_read_only')::boolean
       OR p_expected_daemon_instance_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR p_expected_daemon_epoch IS NULL
       OR p_expected_daemon_epoch <= 0
       OR pg_catalog.octet_length(p_expected_admission_observation_digest) <> 32
       OR p_database_identity_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_memory_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_extension_sql_sha256 !~ '^[0-9a-f]{64}$'
       OR p_extension_manifest_sha256 !~ '^[0-9a-f]{64}$'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL01', MESSAGE = 'invalid writer lease runtime binding';
    END IF;

    SELECT pg_catalog.count(*) INTO v_count
      FROM ONLY writer_lease.writer_lease_extension_identity AS w
      JOIN ONLY writer_lease.writer_lease_extension_ledger AS l USING (singleton)
      CROSS JOIN ONLY control.database_identity AS d
      CROSS JOIN ONLY control.schema_compatibility AS c
      CROSS JOIN ONLY memory.codebase_memory_extension_identity AS m
     WHERE w.singleton AND l.ledger_ordinal = 1
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
       AND l.extension_id = w.extension_id
       AND l.extension_schema_version = w.extension_schema_version
       AND l.extension_sql_sha256 = w.extension_sql_sha256
       AND l.extension_manifest_sha256 = w.extension_manifest_sha256
       AND l.database_uuid = w.database_uuid
       AND l.database_identity_sha256 = w.database_identity_sha256
       AND l.global_schema_version = w.global_schema_version
       AND l.global_manifest_sha256 = w.global_manifest_sha256
       AND l.required_memory_schema_version = w.required_memory_schema_version
       AND l.required_memory_manifest_sha256 = w.required_memory_manifest_sha256;
    IF v_count IS DISTINCT FROM 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL02', MESSAGE = 'writer lease runtime binding mismatch';
    END IF;

    SELECT pg_catalog.count(*) INTO v_admission_count
      FROM ONLY control.runtime_admission AS a
     WHERE a.singleton
       AND a.admission_mode = 'ACTIVE'
       AND a.daemon_instance_id = p_expected_daemon_instance_id
       AND a.daemon_epoch = p_expected_daemon_epoch
       AND a.observation_digest = p_expected_admission_observation_digest;
    IF v_admission_count IS DISTINCT FROM 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL06', MESSAGE = 'writer lease daemon binding mismatch';
    END IF;

    RETURN QUERY
    SELECT a.daemon_instance_id::text, a.daemon_epoch, a.observation_digest
      FROM ONLY control.runtime_admission AS a
     WHERE a.singleton;
END;
$lattice_writer_lease_bind_runtime_v1$;

CREATE FUNCTION writer_lease.writer_lease_load_for_update_v1(
    p_project_id text,
    p_vacant_snapshot_bytes bytea,
    p_vacant_snapshot_bytes_sha256 bytea,
    p_vacant_snapshot_digest bytea,
    p_command_id text
)
RETURNS TABLE (
    row_version bigint,
    snapshot_bytes bytea,
    snapshot_bytes_sha256 bytea,
    snapshot_digest bytea,
    fencing_high_water bigint,
    lease_revision bigint,
    command_high_water bigint,
    command_tail_digest bytea,
    current_status text,
    current_receipt_digest bytea,
    current_project_snapshot_id text,
    current_task_id text,
    current_task_revision text,
    current_task_spec_digest bytea,
    current_attempt_id text,
    current_lease_id text,
    current_lease_holder_id text,
    current_worktree_id text,
    current_holder_process_id bigint,
    current_holder_process_start_identity bytea,
    current_daemon_instance_id text,
    current_daemon_epoch bigint,
    current_fencing_token bigint,
    current_expires_at text,
    observed_at text,
    time_observation_digest bytea,
    admission_mode text,
    daemon_instance_id text,
    daemon_epoch bigint,
    admission_observation_digest bytea,
    existing_repository_request_bytes bytea,
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
AS $lattice_writer_lease_load_for_update_v1$
DECLARE
    v_identity_count bigint;
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
       OR p_project_id IS NULL
       OR p_project_id !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
       OR p_vacant_snapshot_bytes IS NULL
       OR pg_catalog.octet_length(p_vacant_snapshot_bytes) NOT BETWEEN 1 AND 16777216
       OR p_vacant_snapshot_bytes_sha256 IS NULL
       OR pg_catalog.octet_length(p_vacant_snapshot_bytes_sha256) <> 32
       OR p_vacant_snapshot_bytes_sha256 <> pg_catalog.sha256(p_vacant_snapshot_bytes)
       OR p_vacant_snapshot_digest IS NULL
       OR pg_catalog.octet_length(p_vacant_snapshot_digest) <> 32
       OR p_command_id IS NULL
       OR p_command_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL01', MESSAGE = 'invalid writer lease load boundary';
    END IF;

    SELECT pg_catalog.count(*) INTO v_identity_count
      FROM ONLY writer_lease.writer_lease_extension_identity AS w
      JOIN ONLY writer_lease.writer_lease_extension_ledger AS l USING (singleton)
      CROSS JOIN ONLY control.database_identity AS d
      CROSS JOIN ONLY control.schema_compatibility AS c
      CROSS JOIN ONLY memory.codebase_memory_extension_identity AS m
     WHERE w.singleton AND l.ledger_ordinal = 1
       AND w.database_uuid = d.database_uuid
       AND m.database_uuid = d.database_uuid
       AND w.database_identity_sha256 = m.database_identity_sha256
       AND w.global_schema_version = c.current_schema_version
       AND w.global_manifest_sha256 = c.manifest_sha256
       AND w.required_memory_schema_version = m.extension_schema_version
       AND w.required_memory_manifest_sha256 = m.extension_manifest_sha256
       AND l.extension_id = w.extension_id
       AND l.extension_schema_version = w.extension_schema_version
       AND l.extension_sql_sha256 = w.extension_sql_sha256
       AND l.extension_manifest_sha256 = w.extension_manifest_sha256
       AND l.database_uuid = w.database_uuid
       AND l.database_identity_sha256 = w.database_identity_sha256
       AND l.global_schema_version = w.global_schema_version
       AND l.global_manifest_sha256 = w.global_manifest_sha256
       AND l.required_memory_schema_version = w.required_memory_schema_version
       AND l.required_memory_manifest_sha256 = w.required_memory_manifest_sha256;
    IF v_identity_count IS DISTINCT FROM 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL02', MESSAGE = 'writer lease extension identity mismatch';
    END IF;

    SELECT a.* INTO STRICT v_admission
      FROM ONLY control.runtime_admission AS a
     WHERE a.singleton
     FOR SHARE OF a;

    INSERT INTO writer_lease.writer_lease_heads (
        project_id, row_version, snapshot_schema_version, snapshot_bytes,
        snapshot_bytes_sha256, snapshot_digest, fencing_high_water,
        lease_revision, command_high_water
    ) VALUES (
        p_project_id, 0, 1, p_vacant_snapshot_bytes,
        p_vacant_snapshot_bytes_sha256, p_vacant_snapshot_digest, 0, 0, 0
    ) ON CONFLICT (project_id) DO NOTHING;

    SELECT h.* INTO STRICT v_head
      FROM ONLY writer_lease.writer_lease_heads AS h
     WHERE h.project_id = p_project_id
     FOR UPDATE OF h;

    SELECT c.repository_request_bytes, c.repository_request_sha256
      INTO v_repository_request_bytes, v_repository_request_sha256
      FROM ONLY writer_lease.writer_lease_commands AS c
     WHERE c.project_id = p_project_id
       AND c.command_id = p_command_id;

    v_observed_at := pg_catalog.to_char(
        pg_catalog.transaction_timestamp() AT TIME ZONE 'UTC',
        'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'
    );
    v_observed_at := pg_catalog.regexp_replace(v_observed_at, '(\.[0-9]*[1-9])0+Z$', '\1Z');
    v_observed_at := pg_catalog.regexp_replace(v_observed_at, '\.0+Z$', 'Z');
    v_time_digest := pg_catalog.sha256(
        pg_catalog.convert_to('LATTICE_WRITER_LEASE_TIME_V1', 'UTF8')
        || pg_catalog.decode('00', 'hex')
        || pg_catalog.convert_to(v_observed_at, 'UTF8')
    );

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
        v_head.current_daemon_instance_id::text,
        v_head.current_daemon_epoch, v_head.current_fencing_token,
        v_head.current_expires_at, v_observed_at, v_time_digest,
        v_admission.admission_mode::text, v_admission.daemon_instance_id::text,
        v_admission.daemon_epoch, v_admission.observation_digest,
        v_repository_request_bytes, v_repository_request_sha256;
END;
$lattice_writer_lease_load_for_update_v1$;

CREATE FUNCTION writer_lease.writer_lease_commit_plan_v1(
    p_project_id text,
    p_expected_row_version bigint,
    p_expected_snapshot_digest bytea,
    p_expected_command_high_water bigint,
    p_expected_command_tail_digest bytea,
    p_observed_at text,
    p_time_observation_digest bytea,
    p_admission_mode text,
    p_daemon_instance_id text,
    p_daemon_epoch bigint,
    p_admission_observation_digest bytea,
    p_next_snapshot_bytes bytea,
    p_next_snapshot_bytes_sha256 bytea,
    p_next_snapshot_digest bytea,
    p_next_fencing_high_water bigint,
    p_next_lease_revision bigint,
    p_next_command_high_water bigint,
    p_next_command_tail_digest bytea,
    p_current_status text,
    p_current_receipt_digest bytea,
    p_current_project_snapshot_id text,
    p_current_task_id text,
    p_current_task_revision text,
    p_current_task_spec_digest bytea,
    p_current_attempt_id text,
    p_current_lease_id text,
    p_current_lease_holder_id text,
    p_current_worktree_id text,
    p_current_holder_process_id bigint,
    p_current_holder_process_start_identity bytea,
    p_current_daemon_instance_id text,
    p_current_daemon_epoch bigint,
    p_current_fencing_token bigint,
    p_current_expires_at text,
    p_command_ordinal bigint,
    p_command_id text,
    p_repository_request_bytes bytea,
    p_repository_request_sha256 bytea,
    p_request_bytes bytea,
    p_request_digest bytea,
    p_previous_receipt_digest bytea,
    p_outcome text,
    p_denial_reason text,
    p_transition_digest bytea,
    p_receipt_bytes bytea,
    p_receipt_digest bytea,
    p_transition_kind text,
    p_transition_bytes bytea
)
RETURNS text
LANGUAGE plpgsql
VOLATILE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_writer_lease_commit_plan_v1$
DECLARE
    v_admission control.runtime_admission%ROWTYPE;
    v_head writer_lease.writer_lease_heads%ROWTYPE;
    v_observed_at text;
    v_time_digest bytea;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
       OR p_project_id IS NULL
       OR p_project_id !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
       OR p_expected_row_version < 0
       OR p_expected_command_high_water < 0
       OR p_next_fencing_high_water < 0
       OR p_next_lease_revision < 0
       OR pg_catalog.octet_length(p_expected_snapshot_digest) <> 32
       OR p_next_command_high_water <> p_expected_command_high_water + 1
       OR p_command_ordinal <> p_next_command_high_water
       OR p_next_command_tail_digest IS DISTINCT FROM p_receipt_digest
       OR p_previous_receipt_digest IS DISTINCT FROM p_expected_command_tail_digest
       OR pg_catalog.octet_length(p_next_snapshot_bytes) NOT BETWEEN 1 AND 16777216
       OR p_next_snapshot_bytes_sha256 <> pg_catalog.sha256(p_next_snapshot_bytes)
       OR pg_catalog.octet_length(p_next_snapshot_digest) <> 32
       OR pg_catalog.octet_length(p_repository_request_bytes) NOT BETWEEN 1 AND 1048576
       OR pg_catalog.octet_length(p_repository_request_sha256) <> 32
       OR p_repository_request_sha256 <> pg_catalog.sha256(p_repository_request_bytes)
       OR pg_catalog.octet_length(p_request_digest) <> 32
       OR pg_catalog.octet_length(p_receipt_digest) <> 32
       OR (p_transition_digest IS NULL) <> (p_transition_kind IS NULL)
       OR (p_transition_digest IS NULL) <> (p_transition_bytes IS NULL)
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL01', MESSAGE = 'invalid writer lease commit boundary';
    END IF;

    SELECT a.* INTO STRICT v_admission
      FROM ONLY control.runtime_admission AS a
     WHERE a.singleton
     FOR SHARE OF a;
    v_observed_at := pg_catalog.to_char(
        pg_catalog.transaction_timestamp() AT TIME ZONE 'UTC',
        'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'
    );
    v_observed_at := pg_catalog.regexp_replace(v_observed_at, '(\.[0-9]*[1-9])0+Z$', '\1Z');
    v_observed_at := pg_catalog.regexp_replace(v_observed_at, '\.0+Z$', 'Z');
    v_time_digest := pg_catalog.sha256(
        pg_catalog.convert_to('LATTICE_WRITER_LEASE_TIME_V1', 'UTF8')
        || pg_catalog.decode('00', 'hex')
        || pg_catalog.convert_to(v_observed_at, 'UTF8')
    );
    IF p_observed_at IS DISTINCT FROM v_observed_at
       OR p_time_observation_digest IS DISTINCT FROM v_time_digest
       OR p_admission_mode IS DISTINCT FROM v_admission.admission_mode
       OR p_daemon_instance_id IS DISTINCT FROM v_admission.daemon_instance_id
       OR p_daemon_epoch IS DISTINCT FROM v_admission.daemon_epoch
       OR p_admission_observation_digest IS DISTINCT FROM v_admission.observation_digest
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL03', MESSAGE = 'writer lease observation changed';
    END IF;

    SELECT h.* INTO STRICT v_head
      FROM ONLY writer_lease.writer_lease_heads AS h
     WHERE h.project_id = p_project_id
     FOR UPDATE OF h;
    IF v_head.row_version IS DISTINCT FROM p_expected_row_version
       OR v_head.snapshot_digest IS DISTINCT FROM p_expected_snapshot_digest
       OR v_head.command_high_water IS DISTINCT FROM p_expected_command_high_water
       OR v_head.command_tail_digest IS DISTINCT FROM p_expected_command_tail_digest
    THEN
        RETURN 'STALE';
    END IF;
    IF p_next_fencing_high_water < v_head.fencing_high_water
       OR p_next_lease_revision < v_head.lease_revision
       OR (p_current_fencing_token IS NOT NULL
           AND p_current_fencing_token IS DISTINCT FROM p_next_fencing_high_water)
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL04', MESSAGE = 'writer lease monotonicity rejected';
    END IF;

    INSERT INTO writer_lease.writer_lease_commands (
        project_id, ordinal, command_id, repository_request_bytes,
        repository_request_sha256, request_bytes, request_digest,
        previous_receipt_digest, outcome, denial_reason, transition_digest,
        receipt_bytes, receipt_digest
    ) VALUES (
        p_project_id, p_command_ordinal, p_command_id,
        p_repository_request_bytes, p_repository_request_sha256,
        p_request_bytes, p_request_digest, p_previous_receipt_digest, p_outcome,
        p_denial_reason, p_transition_digest, p_receipt_bytes, p_receipt_digest
    );
    IF p_transition_digest IS NOT NULL THEN
        INSERT INTO writer_lease.writer_lease_transitions (
            project_id, ordinal, command_id, transition_kind,
            transition_bytes, transition_digest
        ) VALUES (
            p_project_id, p_command_ordinal, p_command_id, p_transition_kind,
            p_transition_bytes, p_transition_digest
        );
    END IF;

    UPDATE ONLY writer_lease.writer_lease_heads
       SET row_version = row_version + 1,
           snapshot_bytes = p_next_snapshot_bytes,
           snapshot_bytes_sha256 = p_next_snapshot_bytes_sha256,
           snapshot_digest = p_next_snapshot_digest,
           fencing_high_water = p_next_fencing_high_water,
           lease_revision = p_next_lease_revision,
           command_high_water = p_next_command_high_water,
           command_tail_digest = p_next_command_tail_digest,
           current_status = p_current_status,
           current_receipt_digest = p_current_receipt_digest,
           current_project_snapshot_id = p_current_project_snapshot_id,
           current_task_id = p_current_task_id,
           current_task_revision = p_current_task_revision,
           current_task_spec_digest = p_current_task_spec_digest,
           current_attempt_id = p_current_attempt_id,
           current_lease_id = p_current_lease_id,
           current_lease_holder_id = p_current_lease_holder_id,
           current_worktree_id = p_current_worktree_id,
           current_holder_process_id = p_current_holder_process_id,
           current_holder_process_start_identity = p_current_holder_process_start_identity,
           current_daemon_instance_id = p_current_daemon_instance_id,
           current_daemon_epoch = p_current_daemon_epoch,
           current_fencing_token = p_current_fencing_token,
           current_expires_at = p_current_expires_at,
           updated_at = pg_catalog.clock_timestamp()
     WHERE project_id = p_project_id
       AND row_version = p_expected_row_version;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL04', MESSAGE = 'writer lease CAS lost';
    END IF;
    RETURN 'APPLIED';
END;
$lattice_writer_lease_commit_plan_v1$;

CREATE FUNCTION writer_lease.writer_lease_load_commands_v1(p_project_id text)
RETURNS TABLE (
    ordinal bigint,
    command_id text,
    repository_request_bytes bytea,
    repository_request_sha256 bytea,
    request_bytes bytea,
    request_digest bytea,
    previous_receipt_digest bytea,
    outcome text,
    denial_reason text,
    transition_digest bytea,
    receipt_bytes bytea,
    receipt_digest bytea
)
LANGUAGE plpgsql
STABLE
PARALLEL SAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_writer_lease_load_commands_v1$
DECLARE
    v_isolation text;
    v_read_only boolean;
    v_command_high_water bigint;
    v_command_count bigint;
    v_physical_bytes bigint;
BEGIN
    v_isolation := pg_catalog.current_setting('transaction_isolation');
    v_read_only := pg_catalog.current_setting('transaction_read_only')::boolean;
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR NOT ((v_isolation = 'serializable' AND NOT v_read_only)
               OR (v_isolation = 'repeatable read' AND v_read_only))
       OR p_project_id IS NULL
       OR p_project_id !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL01', MESSAGE = 'invalid writer lease command replay boundary';
    END IF;
    SELECT h.command_high_water INTO STRICT v_command_high_water
      FROM ONLY writer_lease.writer_lease_heads AS h
     WHERE h.project_id = p_project_id;
    SELECT pg_catalog.count(*),
           COALESCE(pg_catalog.sum(
               pg_catalog.octet_length(c.repository_request_bytes)
               + pg_catalog.octet_length(c.request_bytes)
               + pg_catalog.octet_length(c.receipt_bytes)
           ), 0)
      INTO v_command_count, v_physical_bytes
      FROM ONLY writer_lease.writer_lease_commands AS c
     WHERE c.project_id = p_project_id;
    IF v_command_count IS DISTINCT FROM v_command_high_water
       OR v_physical_bytes > 67108864
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL02', MESSAGE = 'writer lease command replay corrupt';
    END IF;
    RETURN QUERY
    SELECT c.ordinal, c.command_id::text, c.repository_request_bytes,
           c.repository_request_sha256, c.request_bytes, c.request_digest,
           c.previous_receipt_digest, c.outcome::text, c.denial_reason::text,
           c.transition_digest, c.receipt_bytes, c.receipt_digest
      FROM ONLY writer_lease.writer_lease_commands AS c
     WHERE c.project_id = p_project_id
     ORDER BY c.ordinal;
END;
$lattice_writer_lease_load_commands_v1$;

CREATE FUNCTION writer_lease.writer_lease_load_transitions_v1(p_project_id text)
RETURNS TABLE (
    ordinal bigint,
    command_id text,
    transition_kind text,
    transition_bytes bytea,
    transition_digest bytea
)
LANGUAGE plpgsql
STABLE
PARALLEL SAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_writer_lease_load_transitions_v1$
DECLARE
    v_isolation text;
    v_read_only boolean;
    v_command_high_water bigint;
    v_transition_count bigint;
    v_physical_bytes bigint;
BEGIN
    v_isolation := pg_catalog.current_setting('transaction_isolation');
    v_read_only := pg_catalog.current_setting('transaction_read_only')::boolean;
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR NOT ((v_isolation = 'serializable' AND NOT v_read_only)
               OR (v_isolation = 'repeatable read' AND v_read_only))
       OR p_project_id IS NULL
       OR p_project_id !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL01', MESSAGE = 'invalid writer lease transition replay boundary';
    END IF;
    SELECT h.command_high_water INTO STRICT v_command_high_water
      FROM ONLY writer_lease.writer_lease_heads AS h
     WHERE h.project_id = p_project_id;
    SELECT pg_catalog.count(*),
           COALESCE(pg_catalog.sum(pg_catalog.octet_length(t.transition_bytes)), 0)
      INTO v_transition_count, v_physical_bytes
      FROM ONLY writer_lease.writer_lease_transitions AS t
     WHERE t.project_id = p_project_id;
    IF v_transition_count > v_command_high_water
       OR v_physical_bytes > 33554432
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL02', MESSAGE = 'writer lease transition replay corrupt';
    END IF;
    RETURN QUERY
    SELECT t.ordinal, t.command_id::text, t.transition_kind::text,
           t.transition_bytes, t.transition_digest
      FROM ONLY writer_lease.writer_lease_transitions AS t
     WHERE t.project_id = p_project_id
     ORDER BY t.ordinal;
END;
$lattice_writer_lease_load_transitions_v1$;

CREATE FUNCTION writer_lease.writer_lease_load_current_v1(p_project_id text)
RETURNS TABLE (
    row_version bigint,
    snapshot_bytes bytea,
    snapshot_bytes_sha256 bytea,
    snapshot_digest bytea,
    fencing_high_water bigint,
    lease_revision bigint,
    command_high_water bigint,
    command_tail_digest bytea,
    current_status text,
    current_receipt_digest bytea,
    current_project_snapshot_id text,
    current_task_id text,
    current_task_revision text,
    current_task_spec_digest bytea,
    current_attempt_id text,
    current_lease_id text,
    current_lease_holder_id text,
    current_worktree_id text,
    current_holder_process_id bigint,
    current_holder_process_start_identity bytea,
    current_daemon_instance_id text,
    current_daemon_epoch bigint,
    current_fencing_token bigint,
    current_expires_at text
)
LANGUAGE plpgsql
STABLE
PARALLEL SAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_writer_lease_load_current_v1$
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'repeatable read'
       OR NOT pg_catalog.current_setting('transaction_read_only')::boolean
       OR p_project_id IS NULL
       OR p_project_id !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL01', MESSAGE = 'invalid writer lease current boundary';
    END IF;
    RETURN QUERY
    SELECT h.row_version, h.snapshot_bytes, h.snapshot_bytes_sha256,
           h.snapshot_digest, h.fencing_high_water, h.lease_revision,
           h.command_high_water, h.command_tail_digest, h.current_status::text,
           h.current_receipt_digest, h.current_project_snapshot_id::text,
           h.current_task_id::text, h.current_task_revision::text,
           h.current_task_spec_digest, h.current_attempt_id::text,
           h.current_lease_id::text, h.current_lease_holder_id::text,
           h.current_worktree_id::text, h.current_holder_process_id,
           h.current_holder_process_start_identity,
           h.current_daemon_instance_id::text, h.current_daemon_epoch,
           h.current_fencing_token, h.current_expires_at
      FROM ONLY writer_lease.writer_lease_heads AS h
     WHERE h.project_id = p_project_id;
END;
$lattice_writer_lease_load_current_v1$;

CREATE FUNCTION writer_lease.writer_lease_assert_current_v1(
    p_project_id text,
    p_project_snapshot_id text,
    p_task_id text,
    p_task_revision text,
    p_task_spec_digest bytea,
    p_attempt_id text,
    p_lease_id text,
    p_lease_holder_id text,
    p_worktree_id text,
    p_holder_process_id bigint,
    p_holder_process_start_identity bytea,
    p_daemon_instance_id text,
    p_daemon_epoch bigint,
    p_fencing_token bigint,
    p_receipt_digest bytea
)
RETURNS boolean
LANGUAGE plpgsql
STABLE
PARALLEL SAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_writer_lease_assert_current_v1$
DECLARE
    v_count bigint;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR p_project_id IS NULL
       OR p_project_id !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
       OR p_project_snapshot_id IS NULL
       OR p_project_snapshot_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR p_task_id IS NULL
       OR p_task_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR NOT (CASE
           WHEN p_task_revision ~ '^[1-9][0-9]{0,19}$'
           THEN p_task_revision::numeric <= 18446744073709551615
           ELSE false
       END)
       OR p_task_spec_digest IS NULL OR pg_catalog.octet_length(p_task_spec_digest) <> 32
       OR p_attempt_id IS NULL
       OR p_attempt_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR p_lease_id IS NULL
       OR p_lease_holder_id IS NULL
       OR p_lease_holder_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR p_worktree_id IS NULL
       OR p_worktree_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR p_holder_process_id IS NULL OR p_holder_process_id <= 0
       OR p_holder_process_start_identity IS NULL
       OR pg_catalog.octet_length(p_holder_process_start_identity) <> 32
       OR p_daemon_instance_id IS NULL
       OR p_daemon_epoch IS NULL OR p_daemon_epoch <= 0
       OR p_fencing_token IS NULL OR p_fencing_token <= 0
       OR p_receipt_digest IS NULL OR pg_catalog.octet_length(p_receipt_digest) <> 32
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL01', MESSAGE = 'invalid writer lease assertion boundary';
    END IF;
    SELECT pg_catalog.count(*) INTO v_count
      FROM ONLY writer_lease.writer_lease_heads AS h
      CROSS JOIN ONLY control.runtime_admission AS a
     WHERE h.project_id = p_project_id
       AND h.current_status = 'ACTIVE'
       AND h.current_project_snapshot_id = p_project_snapshot_id
       AND h.current_task_id = p_task_id
       AND h.current_task_revision = p_task_revision
       AND h.current_task_spec_digest = p_task_spec_digest
       AND h.current_attempt_id = p_attempt_id
       AND h.current_lease_id = p_lease_id
       AND h.current_lease_holder_id = p_lease_holder_id
       AND h.current_worktree_id = p_worktree_id
       AND h.current_holder_process_id = p_holder_process_id
       AND h.current_holder_process_start_identity = p_holder_process_start_identity
       AND h.current_daemon_instance_id = p_daemon_instance_id
       AND h.current_daemon_epoch = p_daemon_epoch
       AND h.current_fencing_token = p_fencing_token
       AND h.current_receipt_digest = p_receipt_digest
       AND h.current_expires_at::timestamp with time zone > pg_catalog.clock_timestamp()
       AND a.singleton
       AND a.admission_mode = 'ACTIVE'
       AND a.daemon_instance_id = p_daemon_instance_id
       AND a.daemon_epoch = p_daemon_epoch;
    IF v_count IS DISTINCT FROM 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'LWL05', MESSAGE = 'writer lease authority mismatch';
    END IF;
    RETURN true;
END;
$lattice_writer_lease_assert_current_v1$;

REVOKE ALL ON SCHEMA writer_lease FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA writer_lease FROM
    PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA writer_lease FROM
    PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;

GRANT USAGE ON SCHEMA writer_lease TO lattice_runtime;
GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_bind_runtime_v1(
    text, bigint, bytea, text, text, text, text, text
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_for_update_v1(
    text, bytea, bytea, bytea, text
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_commit_plan_v1(
    text, bigint, bytea, bigint, bytea, text, bytea, text, text, bigint,
    bytea, bytea, bytea, bytea, bigint, bigint, bigint, bytea, text, bytea,
    text, text, text, bytea, text, text, text, text, bigint, bytea, text,
    bigint, bigint, text, bigint, text, bytea, bytea, bytea, bytea, bytea,
    text, text, bytea, bytea, bytea, text, bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_commands_v1(text)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_current_v1(text)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_assert_current_v1(
    text, text, text, text, bytea, text, text, text, text, bigint, bytea,
    text, bigint, bigint, bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_transitions_v1(text)
    TO lattice_runtime;

COMMENT ON SCHEMA writer_lease IS 'LATTICE_WRITER_LEASE_SCHEMA_V1';
COMMENT ON TABLE writer_lease.writer_lease_extension_identity IS
    'LATTICE_WRITER_LEASE_EXTENSION_IDENTITY_V1';
COMMENT ON TABLE writer_lease.writer_lease_extension_ledger IS
    'LATTICE_WRITER_LEASE_EXTENSION_LEDGER_V1';
COMMENT ON TABLE writer_lease.writer_lease_heads IS
    'LATTICE_WRITER_LEASE_HEADS_V1';
COMMENT ON TABLE writer_lease.writer_lease_commands IS
    'LATTICE_WRITER_LEASE_COMMANDS_V1';
COMMENT ON TABLE writer_lease.writer_lease_transitions IS
    'LATTICE_WRITER_LEASE_TRANSITIONS_V1';
