CREATE TABLE control.project_registry_state (
    singleton boolean PRIMARY KEY DEFAULT true,
    runtime varchar(16) NOT NULL,
    command_ordinal bigint NOT NULL,
    observation_count bigint NOT NULL,
    project_count bigint NOT NULL,
    command_count bigint NOT NULL,
    reservation_count bigint NOT NULL,
    retained_bytes bigint NOT NULL,
    checkpoint_digest bytea NOT NULL,
    stage_command_id varchar(128),
    stage_ordinal bigint,
    stage_base_checkpoint_digest bytea,
    stage_result_checkpoint_digest bytea,
    stage_record_set_digest bytea,
    stage_observation boolean,
    stage_project boolean,
    stage_reservation_delete_count bigint,
    stage_reservation_insert_count bigint,
    CONSTRAINT project_registry_state_singleton_true CHECK (singleton),
    CONSTRAINT project_registry_state_runtime_live CHECK (runtime = 'LIVE'),
    CONSTRAINT project_registry_state_counts_nonnegative CHECK (
        command_ordinal >= 0 AND observation_count >= 0 AND project_count >= 0
        AND command_count >= 0 AND reservation_count >= 0 AND retained_bytes >= 103
    ),
    CONSTRAINT project_registry_state_limits CHECK (
        project_count <= 4096 AND command_count <= 65536
        AND retained_bytes <= 67108864 AND command_ordinal = command_count
    ),
    CONSTRAINT project_registry_state_digest CHECK (
        pg_catalog.octet_length(checkpoint_digest) = 32
        AND checkpoint_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    ),
    CONSTRAINT project_registry_state_stage_shape CHECK (
        (stage_command_id IS NULL AND stage_ordinal IS NULL
         AND stage_base_checkpoint_digest IS NULL AND stage_result_checkpoint_digest IS NULL
         AND stage_record_set_digest IS NULL AND stage_observation IS NULL
         AND stage_project IS NULL AND stage_reservation_delete_count IS NULL
         AND stage_reservation_insert_count IS NULL)
        OR
        (stage_command_id IS NOT NULL AND stage_ordinal IS NOT NULL AND stage_ordinal > 0
         AND pg_catalog.octet_length(stage_base_checkpoint_digest) = 32
         AND pg_catalog.octet_length(stage_result_checkpoint_digest) = 32
         AND pg_catalog.octet_length(stage_record_set_digest) = 32
         AND stage_observation IS NOT NULL AND stage_project IS NOT NULL
         AND stage_reservation_delete_count IS NOT NULL AND stage_reservation_delete_count >= 0
         AND stage_reservation_insert_count IS NOT NULL AND stage_reservation_insert_count >= 0)
    )
);

CREATE TABLE control.project_registry_observations (
    observation_digest bytea PRIMARY KEY,
    canonical_root text NOT NULL,
    root_identity_digest bytea NOT NULL,
    repository_identity_digest bytea NOT NULL,
    file_identity_digest bytea NOT NULL,
    primary_ref varchar(512) NOT NULL,
    primary_ref_storage_digest bytea NOT NULL,
    CONSTRAINT project_registry_observations_digest CHECK (
        pg_catalog.octet_length(observation_digest) = 32
        AND observation_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    ),
    CONSTRAINT project_registry_observations_root_bound CHECK (
        pg_catalog.octet_length(pg_catalog.convert_to(canonical_root, 'UTF8')) BETWEEN 1 AND 131072
    ),
    CONSTRAINT project_registry_observations_primary_ref CHECK (
        primary_ref ~ '^refs/(heads|tags)/[A-Za-z0-9][A-Za-z0-9._/-]*$'
    ),
    CONSTRAINT project_registry_observations_identity_digests CHECK (
        pg_catalog.octet_length(root_identity_digest) = 32
        AND pg_catalog.octet_length(repository_identity_digest) = 32
        AND pg_catalog.octet_length(file_identity_digest) = 32
        AND pg_catalog.octet_length(primary_ref_storage_digest) = 32
        AND root_identity_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND repository_identity_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND file_identity_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND primary_ref_storage_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    )
);

CREATE TABLE control.project_registry_projects (
    project_id varchar(64) PRIMARY KEY,
    project_class varchar(16) NOT NULL,
    accepted_observation_digest bytea NOT NULL REFERENCES control.project_registry_observations(observation_digest),
    pending_observation_digest bytea REFERENCES control.project_registry_observations(observation_digest),
    drift_canonical_root boolean NOT NULL,
    drift_repository boolean NOT NULL,
    drift_file boolean NOT NULL,
    drift_primary_ref_name boolean NOT NULL,
    drift_primary_ref_storage boolean NOT NULL,
    authority_contract_version smallint NOT NULL,
    authority_producer_id varchar(128) NOT NULL,
    authority_producer_version varchar(32) NOT NULL,
    authority_runtime varchar(16) NOT NULL,
    authority_snapshot_id text NOT NULL,
    authority_registry_revision numeric(20,0) NOT NULL,
    authority_lifecycle varchar(32) NOT NULL,
    authority_primary_ref varchar(512) NOT NULL,
    authority_primary_ref_storage_digest bytea NOT NULL,
    authority_observation_digest bytea NOT NULL REFERENCES control.project_registry_observations(observation_digest),
    authority_receipt_digest bytea NOT NULL,
    CONSTRAINT project_registry_projects_id CHECK (project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'),
    CONSTRAINT project_registry_projects_class CHECK (project_class IN ('USER_PROJECT', 'LATTICE_SYSTEM')),
    CONSTRAINT project_registry_projects_pending_distinct CHECK (
        pending_observation_digest IS NULL OR pending_observation_digest <> accepted_observation_digest
    ),
    CONSTRAINT project_registry_projects_authority CHECK (
        authority_contract_version = 1 AND authority_producer_id = 'lattice-project-registry'
        AND authority_producer_version = '1.0' AND authority_runtime = 'LIVE'
        AND authority_registry_revision BETWEEN 1 AND 18446744073709551615
        AND authority_lifecycle IN ('ACTIVE', 'SUSPENDED', 'RECONCILIATION_REQUIRED')
        AND pg_catalog.octet_length(authority_primary_ref_storage_digest) = 32
        AND pg_catalog.octet_length(authority_observation_digest) = 32
        AND pg_catalog.octet_length(authority_receipt_digest) = 32
    ),
    CONSTRAINT project_registry_projects_shape CHECK (
        (authority_lifecycle = 'ACTIVE' AND pending_observation_digest IS NULL
         AND authority_observation_digest = accepted_observation_digest)
        OR
        (authority_lifecycle = 'RECONCILIATION_REQUIRED' AND pending_observation_digest IS NOT NULL
         AND authority_observation_digest = pending_observation_digest)
        OR
        (authority_lifecycle = 'SUSPENDED' AND pending_observation_digest IS NULL)
    )
);

CREATE TABLE control.project_registry_commands (
    ordinal bigint PRIMARY KEY,
    command_id varchar(128) NOT NULL UNIQUE,
    action varchar(16) NOT NULL,
    project_id varchar(64) NOT NULL,
    project_class varchar(16),
    observation_digest bytea REFERENCES control.project_registry_observations(observation_digest),
    before_present boolean NOT NULL,
    before_producer_id varchar(128), before_producer_version varchar(32), before_runtime varchar(16),
    before_project_id varchar(64), before_snapshot_id text, before_registry_revision numeric(20,0),
    before_lifecycle varchar(32), before_project_class varchar(16), before_primary_ref varchar(512),
    before_primary_ref_storage_digest bytea, before_observation_digest bytea, before_receipt_digest bytea,
    decision varchar(32), evidence_digest bytea, request_digest bytea NOT NULL,
    outcome varchar(16) NOT NULL,
    denial_reason varchar(64), denial_dimension varchar(32), denial_existing_project_id varchar(64),
    denial_lifecycle varchar(32), denial_expected_decision varchar(32), denial_found_decision varchar(32),
    semantic_before_receipt_digest bytea, semantic_after_receipt_digest bytea,
    authority_receipt_digest bytea,
    drift_canonical_root boolean NOT NULL, drift_repository boolean NOT NULL, drift_file boolean NOT NULL,
    drift_primary_ref_name boolean NOT NULL, drift_primary_ref_storage boolean NOT NULL,
    result_digest bytea NOT NULL,
    base_runtime varchar(16) NOT NULL, base_ordinal bigint NOT NULL,
    base_observation_count bigint NOT NULL, base_project_count bigint NOT NULL,
    base_command_count bigint NOT NULL, base_reservation_count bigint NOT NULL,
    base_retained_bytes bigint NOT NULL, base_checkpoint_digest bytea NOT NULL,
    result_runtime varchar(16) NOT NULL, result_ordinal bigint NOT NULL,
    result_observation_count bigint NOT NULL, result_project_count bigint NOT NULL,
    result_command_count bigint NOT NULL, result_reservation_count bigint NOT NULL,
    result_retained_bytes bigint NOT NULL, result_checkpoint_digest bytea NOT NULL,
    record_set_digest bytea NOT NULL,
    authority_runtime varchar(16) NOT NULL, daemon_instance_id varchar(128) NOT NULL,
    daemon_epoch bigint NOT NULL, admission_mode varchar(32) NOT NULL,
    daemon_authority_revision bigint NOT NULL, daemon_observation_digest bytea NOT NULL,
    daemon_head_digest bytea NOT NULL,
    transaction_digest bytea NOT NULL, persistence_receipt_digest bytea NOT NULL,
    CONSTRAINT project_registry_commands_ordinal_positive CHECK (ordinal > 0),
    CONSTRAINT project_registry_commands_id CHECK (
        command_id <> '' AND command_id = pg_catalog.btrim(command_id)
    ),
    CONSTRAINT project_registry_commands_action CHECK (action IN ('REGISTER', 'OBSERVE', 'SUSPEND', 'RECONCILE')),
    CONSTRAINT project_registry_commands_project_id CHECK (project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'),
    CONSTRAINT project_registry_commands_outcome CHECK (outcome IN ('APPLIED', 'DENIED', 'BLOCKED')),
    CONSTRAINT project_registry_commands_runtime CHECK (
        base_runtime = 'LIVE' AND result_runtime = 'LIVE' AND authority_runtime = 'LIVE'
        AND admission_mode = 'ACTIVE' AND daemon_epoch > 0 AND daemon_authority_revision > 0
    ),
    CONSTRAINT project_registry_commands_chain CHECK (
        base_ordinal >= 0 AND result_ordinal = ordinal AND ordinal = base_ordinal + 1
        AND base_command_count = base_ordinal AND result_command_count = result_ordinal
        AND base_retained_bytes >= 103 AND result_retained_bytes >= 103
        AND result_project_count BETWEEN 0 AND 4096 AND result_command_count BETWEEN 1 AND 65536
        AND result_retained_bytes <= 67108864
    ),
    CONSTRAINT project_registry_commands_before_shape CHECK (
        (before_present AND before_producer_id IS NOT NULL AND before_producer_version IS NOT NULL
         AND before_runtime IS NOT NULL AND before_project_id IS NOT NULL AND before_snapshot_id IS NOT NULL
         AND before_registry_revision IS NOT NULL AND before_lifecycle IS NOT NULL
         AND before_project_class IS NOT NULL AND before_primary_ref IS NOT NULL
         AND before_primary_ref_storage_digest IS NOT NULL AND before_observation_digest IS NOT NULL
         AND before_receipt_digest IS NOT NULL)
        OR
        (NOT before_present AND before_producer_id IS NULL AND before_producer_version IS NULL
         AND before_runtime IS NULL AND before_project_id IS NULL AND before_snapshot_id IS NULL
         AND before_registry_revision IS NULL AND before_lifecycle IS NULL
         AND before_project_class IS NULL AND before_primary_ref IS NULL
         AND before_primary_ref_storage_digest IS NULL AND before_observation_digest IS NULL
         AND before_receipt_digest IS NULL)
    ),
    CONSTRAINT project_registry_commands_digest_lengths CHECK (
        pg_catalog.octet_length(request_digest) = 32 AND pg_catalog.octet_length(result_digest) = 32
        AND pg_catalog.octet_length(base_checkpoint_digest) = 32
        AND pg_catalog.octet_length(result_checkpoint_digest) = 32
        AND pg_catalog.octet_length(record_set_digest) = 32
        AND pg_catalog.octet_length(daemon_observation_digest) = 32
        AND pg_catalog.octet_length(daemon_head_digest) = 32
        AND pg_catalog.octet_length(transaction_digest) = 32
        AND pg_catalog.octet_length(persistence_receipt_digest) = 32
    )
);

CREATE TABLE control.project_registry_identity_reservations (
    dimension varchar(32) NOT NULL,
    identity_digest bytea NOT NULL,
    reservation_status varchar(16) NOT NULL,
    project_id varchar(64) NOT NULL REFERENCES control.project_registry_projects(project_id),
    PRIMARY KEY (dimension, identity_digest, reservation_status),
    CONSTRAINT project_registry_reservations_dimension CHECK (dimension IN ('CANONICAL_ROOT', 'REPOSITORY', 'FILE')),
    CONSTRAINT project_registry_reservations_status CHECK (reservation_status IN ('ACCEPTED', 'PENDING')),
    CONSTRAINT project_registry_reservations_digest CHECK (
        pg_catalog.octet_length(identity_digest) = 32
        AND identity_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    )
);

INSERT INTO control.project_registry_state (
    singleton, runtime, command_ordinal, observation_count, project_count,
    command_count, reservation_count, retained_bytes, checkpoint_digest
) VALUES (
    true, 'LIVE', 0, 0, 0, 0, 0, 103,
    pg_catalog.decode('5bb1f9d9adf7228bef7ea45cc029e79d71761c4dfed3abe086634917db38c173', 'hex')
);

CREATE FUNCTION control.store_prepare_v4(
    p_global_schema_version smallint,
    p_global_manifest_sha256 text,
    p_store_contract_version smallint,
    p_transaction_id text,
    p_project_id text,
    p_project_snapshot_id text,
    p_repository_owner text,
    p_aggregate_key_digest bytea,
    p_request_digest bytea,
    p_authority_runtime text,
    p_daemon_instance_id text,
    p_daemon_epoch bigint,
    p_admission_mode text,
    p_authority_revision bigint,
    p_authority_observation_digest bytea,
    p_authority_head_digest bytea,
    p_expected_head_runtime text,
    p_expected_revision bigint,
    p_expected_state_digest bytea,
    p_expected_head_digest bytea,
    p_domain_command_digest bytea,
    p_record_set_digest bytea,
    p_next_state_digest bytea,
    p_domain_receipt_digest bytea,
    p_checkpoint_digest bytea,
    p_outbox_intent_digest bytea,
    p_genesis_state_digest bytea,
    p_genesis_head_digest bytea
)
RETURNS TABLE (
    prepare_status text,
    database_uuid uuid,
    database_identity_digest bytea,
    schema_version smallint,
    manifest_sha256 text,
    head_found boolean,
    before_revision bigint,
    before_state_digest bytea,
    before_head_digest bytea,
    after_revision bigint,
    after_state_digest bytea,
    after_head_digest bytea,
    terminal_disposition text,
    terminal_transaction_digest bytea,
    terminal_receipt_digest bytea,
    global_schema_version smallint,
    global_manifest_sha256 text
)
LANGUAGE plpgsql
VOLATILE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_store_prepare_v4$
DECLARE
    v_terminal control.terminal_transactions%ROWTYPE;
    v_database_uuid uuid;
    v_schema_version smallint;
    v_manifest_sha256 text;
    v_compatibility_found boolean;
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
    v_min_reader smallint;
    v_max_reader smallint;
    v_min_writer smallint;
    v_max_writer smallint;
    v_admission_mode text;
    v_daemon_instance_id text;
    v_daemon_epoch bigint;
    v_authority_revision bigint;
    v_authority_observation_digest bytea;
    v_authority_head_digest bytea;
    v_before_revision bigint;
    v_before_state_digest bytea;
    v_before_head_digest bytea;
    v_head_found boolean;
    v_digest bytea;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
       OR p_store_contract_version IS NULL
       OR p_store_contract_version <> 2
       OR p_transaction_id IS NULL
       OR p_transaction_id !~ '^[a-z0-9._:-]{1,128}$'
       OR p_project_id IS NULL
       OR p_project_id !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
       OR p_project_snapshot_id IS NULL
       OR p_project_snapshot_id !~ '^[a-z0-9._:-]{1,128}$'
       OR p_repository_owner IS NULL
       OR p_repository_owner NOT IN (
            'PROJECT_REGISTRY', 'TASK_LEDGER', 'WRITER_LEASE',
            'APPROVAL_VERIFIER', 'ARTIFACT_STORE'
       )
       OR p_authority_runtime IS NULL
       OR p_authority_runtime <> 'LIVE'
       OR p_expected_head_runtime IS NULL
       OR p_expected_head_runtime <> 'LIVE'
       OR p_admission_mode IS NULL
       OR p_admission_mode NOT IN (
            'ACTIVE', 'DRAINING', 'CANARY', 'STOPPED', 'RECONCILIATION_REQUIRED'
       )
       OR p_daemon_instance_id IS NULL
       OR p_daemon_instance_id !~ '^[a-z0-9][a-z0-9._:-]{0,127}$'
       OR p_daemon_epoch IS NULL
       OR p_daemon_epoch <= 0
       OR p_authority_revision IS NULL
       OR p_authority_revision <= 0
       OR p_expected_revision IS NULL
       OR p_expected_revision < 0
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid store request';
    END IF;

    FOREACH v_digest IN ARRAY ARRAY[
        p_aggregate_key_digest,
        p_request_digest,
        p_authority_observation_digest,
        p_authority_head_digest,
        p_expected_state_digest,
        p_expected_head_digest,
        p_domain_command_digest,
        p_record_set_digest,
        p_next_state_digest,
        p_domain_receipt_digest,
        p_genesis_state_digest,
        p_genesis_head_digest
    ]
    LOOP
        IF v_digest IS NULL
           OR pg_catalog.octet_length(v_digest) <> 32
           OR v_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid store digest';
        END IF;
    END LOOP;
    IF (p_checkpoint_digest IS NOT NULL AND (
            pg_catalog.octet_length(p_checkpoint_digest) <> 32
            OR p_checkpoint_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       ))
       OR (p_outbox_intent_digest IS NOT NULL AND (
            pg_catalog.octet_length(p_outbox_intent_digest) <> 32
            OR p_outbox_intent_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       ))
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid optional store digest';
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended('lattice.store.tx.v2:' || p_transaction_id, 0)
    );

    SELECT t.*
      INTO v_terminal
      FROM ONLY control.terminal_transactions AS t
     WHERE t.transaction_id = p_transaction_id
     FOR UPDATE OF t;

    IF FOUND THEN
        IF v_terminal.store_contract_version IS DISTINCT FROM p_store_contract_version
           OR v_terminal.transaction_id IS DISTINCT FROM p_transaction_id
           OR v_terminal.project_id IS DISTINCT FROM p_project_id
           OR v_terminal.project_snapshot_id IS DISTINCT FROM p_project_snapshot_id
           OR v_terminal.repository_owner IS DISTINCT FROM p_repository_owner
           OR v_terminal.aggregate_key_digest IS DISTINCT FROM p_aggregate_key_digest
           OR v_terminal.request_digest IS DISTINCT FROM p_request_digest
           OR v_terminal.runtime IS DISTINCT FROM p_authority_runtime
           OR v_terminal.daemon_instance_id IS DISTINCT FROM p_daemon_instance_id
           OR v_terminal.daemon_epoch IS DISTINCT FROM p_daemon_epoch
           OR v_terminal.admission_mode IS DISTINCT FROM p_admission_mode
           OR v_terminal.authority_revision IS DISTINCT FROM p_authority_revision
           OR v_terminal.authority_observation_digest IS DISTINCT FROM p_authority_observation_digest
           OR v_terminal.authority_head_digest IS DISTINCT FROM p_authority_head_digest
           OR v_terminal.runtime IS DISTINCT FROM p_expected_head_runtime
           OR v_terminal.expected_revision IS DISTINCT FROM p_expected_revision
           OR v_terminal.expected_state_digest IS DISTINCT FROM p_expected_state_digest
           OR v_terminal.expected_head_digest IS DISTINCT FROM p_expected_head_digest
           OR v_terminal.domain_command_digest IS DISTINCT FROM p_domain_command_digest
           OR v_terminal.record_set_digest IS DISTINCT FROM p_record_set_digest
           OR v_terminal.next_state_digest IS DISTINCT FROM p_next_state_digest
           OR v_terminal.domain_receipt_digest IS DISTINCT FROM p_domain_receipt_digest
           OR v_terminal.checkpoint_digest IS DISTINCT FROM p_checkpoint_digest
           OR v_terminal.outbox_intent_digest IS DISTINCT FROM p_outbox_intent_digest
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LTX01', MESSAGE = 'transaction identity substituted';
        END IF;
    END IF;

    SELECT d.database_uuid,
           c.current_schema_version,
           pg_catalog.btrim(c.manifest_sha256::text),
           c.min_reader,
           c.max_reader,
           c.min_writer,
           c.max_writer
      INTO v_database_uuid,
           v_schema_version,
           v_manifest_sha256,
           v_min_reader,
           v_max_reader,
           v_min_writer,
           v_max_writer
      FROM ONLY control.database_identity AS d
      CROSS JOIN ONLY control.schema_compatibility AS c
      WHERE d.singleton = true
       AND c.singleton = true
      FOR SHARE OF d, c;
    v_compatibility_found := FOUND;

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF NOT v_compatibility_found
       OR v_schema_version IS DISTINCT FROM 4
       OR v_min_reader IS DISTINCT FROM 4
       OR v_max_reader IS DISTINCT FROM 4
       OR v_min_writer IS DISTINCT FROM 4
       OR v_max_writer IS DISTINCT FROM 4
       OR v_manifest_sha256 IS NULL
       OR v_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR v_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM v_manifest_sha256
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'store schema not current';
    END IF;

    IF v_terminal.transaction_id IS NOT NULL THEN
        IF v_terminal.producer_id IS DISTINCT FROM 'lattice-postgres-store'
           OR v_terminal.producer_version IS DISTINCT FROM '1.0'
           OR v_terminal.durability IS DISTINCT FROM 'DURABLE_POSTGRES'
           OR v_terminal.database_uuid IS DISTINCT FROM v_database_uuid
           OR v_terminal.schema_version IS DISTINCT FROM 2
           OR pg_catalog.btrim(v_terminal.manifest_sha256::text) IS DISTINCT FROM '4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129'
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'terminal receipt corrupt';
        END IF;
        RETURN QUERY SELECT
            'REPLAY'::text,
            v_terminal.database_uuid,
            v_terminal.database_identity_digest,
            v_terminal.schema_version,
            pg_catalog.btrim(v_terminal.manifest_sha256::text),
            true,
            v_terminal.before_revision,
            v_terminal.before_state_digest,
            v_terminal.before_head_digest,
            v_terminal.after_revision,
            v_terminal.after_state_digest,
            v_terminal.after_head_digest,
            v_terminal.disposition::text,
            v_terminal.transaction_digest,
            v_terminal.receipt_digest,
            v_schema_version,
            v_manifest_sha256;
        RETURN;
    END IF;

    IF p_admission_mode IS DISTINCT FROM 'ACTIVE' THEN
        RAISE EXCEPTION USING ERRCODE = 'LAD01', MESSAGE = 'request admission denied';
    END IF;

    SELECT a.admission_mode,
           a.daemon_instance_id,
           a.daemon_epoch,
           a.authority_revision,
           a.observation_digest,
           a.authority_head_digest
      INTO v_admission_mode,
           v_daemon_instance_id,
           v_daemon_epoch,
           v_authority_revision,
           v_authority_observation_digest,
           v_authority_head_digest
      FROM ONLY control.runtime_admission AS a
     WHERE a.singleton = true
     FOR SHARE OF a;

    IF NOT FOUND OR v_admission_mode IS DISTINCT FROM 'ACTIVE' THEN
        RAISE EXCEPTION USING ERRCODE = 'LAD01', MESSAGE = 'runtime admission denied';
    END IF;

    IF v_daemon_instance_id IS DISTINCT FROM p_daemon_instance_id
       OR v_daemon_epoch IS DISTINCT FROM p_daemon_epoch
       OR v_authority_revision IS DISTINCT FROM p_authority_revision
       OR v_authority_observation_digest IS DISTINCT FROM p_authority_observation_digest
       OR v_authority_head_digest IS DISTINCT FROM p_authority_head_digest
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAU01', MESSAGE = 'runtime authority mismatch';
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(
            'lattice.store.scope.v2:'
            || p_project_id || ':'
            || p_project_snapshot_id || ':'
            || p_repository_owner || ':'
            || pg_catalog.encode(p_aggregate_key_digest, 'hex'),
            0
        )
    );

    SELECT h.physical_revision,
           h.state_digest,
           h.head_digest
      INTO v_before_revision,
           v_before_state_digest,
           v_before_head_digest
      FROM ONLY control.physical_heads AS h
     WHERE h.project_id = p_project_id
       AND h.project_snapshot_id = p_project_snapshot_id
       AND h.repository_owner = p_repository_owner
       AND h.aggregate_key_digest = p_aggregate_key_digest
     FOR UPDATE OF h;

    v_head_found := FOUND;
    IF NOT v_head_found THEN
        v_before_revision := 0;
        v_before_state_digest := p_genesis_state_digest;
        v_before_head_digest := p_genesis_head_digest;
    END IF;

    RETURN QUERY SELECT
        'PREPARED'::text,
        v_database_uuid,
        NULL::bytea,
        2::smallint,
        '4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129'::text,
        v_head_found,
        v_before_revision,
        v_before_state_digest,
        v_before_head_digest,
        NULL::bigint,
        NULL::bytea,
        NULL::bytea,
        NULL::text,
        NULL::bytea,
        NULL::bytea,
        v_schema_version,
        v_manifest_sha256;
END;
$lattice_store_prepare_v4$;

CREATE FUNCTION control.store_finalize_v4(
    p_global_schema_version smallint,
    p_global_manifest_sha256 text,
    p_store_contract_version smallint,
    p_transaction_id text,
    p_project_id text,
    p_project_snapshot_id text,
    p_repository_owner text,
    p_aggregate_key_digest bytea,
    p_request_digest bytea,
    p_authority_runtime text,
    p_daemon_instance_id text,
    p_daemon_epoch bigint,
    p_admission_mode text,
    p_authority_revision bigint,
    p_authority_observation_digest bytea,
    p_authority_head_digest bytea,
    p_expected_head_runtime text,
    p_expected_revision bigint,
    p_expected_state_digest bytea,
    p_expected_head_digest bytea,
    p_domain_command_digest bytea,
    p_record_set_digest bytea,
    p_next_state_digest bytea,
    p_domain_receipt_digest bytea,
    p_checkpoint_digest bytea,
    p_outbox_intent_digest bytea,
    p_genesis_state_digest bytea,
    p_genesis_head_digest bytea,
    p_database_uuid uuid,
    p_database_identity_digest bytea,
    p_schema_version smallint,
    p_manifest_sha256 text,
    p_before_revision bigint,
    p_before_state_digest bytea,
    p_before_head_digest bytea,
    p_after_revision bigint,
    p_after_state_digest bytea,
    p_after_head_digest bytea,
    p_disposition text,
    p_transaction_digest bytea,
    p_receipt_digest bytea
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
AS $lattice_store_finalize_v4$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
    v_prepare record;
    v_rows bigint;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    SELECT *
      INTO v_prepare
      FROM control.store_prepare_v4(
        p_global_schema_version,
        p_global_manifest_sha256,
        p_store_contract_version,
        p_transaction_id,
        p_project_id,
        p_project_snapshot_id,
        p_repository_owner,
        p_aggregate_key_digest,
        p_request_digest,
        p_authority_runtime,
        p_daemon_instance_id,
        p_daemon_epoch,
        p_admission_mode,
        p_authority_revision,
        p_authority_observation_digest,
        p_authority_head_digest,
        p_expected_head_runtime,
        p_expected_revision,
        p_expected_state_digest,
        p_expected_head_digest,
        p_domain_command_digest,
        p_record_set_digest,
        p_next_state_digest,
        p_domain_receipt_digest,
        p_checkpoint_digest,
        p_outbox_intent_digest,
        p_genesis_state_digest,
        p_genesis_head_digest
      );

    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'prepare returned no row';
    END IF;

    IF v_prepare.prepare_status = 'REPLAY' THEN
        IF v_prepare.database_uuid IS DISTINCT FROM p_database_uuid
           OR v_prepare.database_identity_digest IS DISTINCT FROM p_database_identity_digest
           OR v_prepare.schema_version IS DISTINCT FROM p_schema_version
           OR v_prepare.manifest_sha256 IS DISTINCT FROM p_manifest_sha256
           OR v_prepare.before_revision IS DISTINCT FROM p_before_revision
           OR v_prepare.before_state_digest IS DISTINCT FROM p_before_state_digest
           OR v_prepare.before_head_digest IS DISTINCT FROM p_before_head_digest
           OR v_prepare.after_revision IS DISTINCT FROM p_after_revision
           OR v_prepare.after_state_digest IS DISTINCT FROM p_after_state_digest
           OR v_prepare.after_head_digest IS DISTINCT FROM p_after_head_digest
           OR v_prepare.terminal_disposition IS DISTINCT FROM p_disposition
           OR v_prepare.terminal_transaction_digest IS DISTINCT FROM p_transaction_digest
           OR v_prepare.terminal_receipt_digest IS DISTINCT FROM p_receipt_digest
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'terminal receipt corrupt';
        END IF;
        RETURN 'REPLAY';
    END IF;

    IF v_prepare.prepare_status IS DISTINCT FROM 'PREPARED'
       OR p_database_uuid IS NULL
       OR v_prepare.database_uuid IS DISTINCT FROM p_database_uuid
       OR p_schema_version IS NULL
       OR v_prepare.schema_version IS DISTINCT FROM p_schema_version
       OR p_manifest_sha256 IS NULL
       OR v_prepare.manifest_sha256 IS DISTINCT FROM p_manifest_sha256
       OR p_before_revision IS NULL
       OR v_prepare.before_revision IS DISTINCT FROM p_before_revision
       OR p_before_state_digest IS NULL
       OR v_prepare.before_state_digest IS DISTINCT FROM p_before_state_digest
       OR p_before_head_digest IS NULL
       OR v_prepare.before_head_digest IS DISTINCT FROM p_before_head_digest
       OR p_database_identity_digest IS NULL
       OR pg_catalog.octet_length(p_database_identity_digest) <> 32
       OR p_database_identity_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_after_revision IS NULL
       OR p_after_revision < 0
       OR p_after_state_digest IS NULL
       OR pg_catalog.octet_length(p_after_state_digest) <> 32
       OR p_after_state_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_after_head_digest IS NULL
       OR pg_catalog.octet_length(p_after_head_digest) <> 32
       OR p_after_head_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_transaction_digest IS NULL
       OR pg_catalog.octet_length(p_transaction_digest) <> 32
       OR p_transaction_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_receipt_digest IS NULL
       OR pg_catalog.octet_length(p_receipt_digest) <> 32
       OR p_receipt_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_disposition IS NULL
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid finalized receipt';
    END IF;

    IF p_disposition = 'APPLIED' THEN
        IF p_before_revision = 9223372036854775807 THEN
            RAISE EXCEPTION USING ERRCODE = 'LRV01', MESSAGE = 'physical revision exhausted';
        END IF;

        IF p_expected_revision IS DISTINCT FROM p_before_revision
           OR p_expected_state_digest IS DISTINCT FROM p_before_state_digest
           OR p_expected_head_digest IS DISTINCT FROM p_before_head_digest
           OR p_after_revision IS DISTINCT FROM p_before_revision + 1
           OR p_after_state_digest IS DISTINCT FROM p_next_state_digest
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LST02', MESSAGE = 'applied transition invalid';
        END IF;

        IF v_prepare.head_found THEN
            UPDATE ONLY control.physical_heads
               SET physical_revision = p_after_revision,
                   state_digest = p_after_state_digest,
                   head_digest = p_after_head_digest,
                   updated_at = pg_catalog.clock_timestamp()
             WHERE project_id = p_project_id
               AND project_snapshot_id = p_project_snapshot_id
               AND repository_owner = p_repository_owner
               AND aggregate_key_digest = p_aggregate_key_digest
               AND physical_revision = p_before_revision
               AND state_digest = p_before_state_digest
               AND head_digest = p_before_head_digest;
            GET DIAGNOSTICS v_rows = ROW_COUNT;
            IF v_rows <> 1 THEN
                RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'physical head changed';
            END IF;
        ELSE
            INSERT INTO control.physical_heads (
                project_id,
                project_snapshot_id,
                repository_owner,
                aggregate_key_digest,
                physical_revision,
                state_digest,
                head_digest
            ) VALUES (
                p_project_id,
                p_project_snapshot_id,
                p_repository_owner,
                p_aggregate_key_digest,
                p_after_revision,
                p_after_state_digest,
                p_after_head_digest
            );
        END IF;
    ELSIF p_disposition = 'STALE_PHYSICAL_HEAD' THEN
        IF (p_expected_revision IS NOT DISTINCT FROM p_before_revision
            AND p_expected_state_digest IS NOT DISTINCT FROM p_before_state_digest
            AND p_expected_head_digest IS NOT DISTINCT FROM p_before_head_digest)
           OR p_after_revision IS DISTINCT FROM p_before_revision
           OR p_after_state_digest IS DISTINCT FROM p_before_state_digest
           OR p_after_head_digest IS DISTINCT FROM p_before_head_digest
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LST02', MESSAGE = 'stale transition invalid';
        END IF;
    ELSE
        RAISE EXCEPTION USING ERRCODE = 'LST02', MESSAGE = 'unknown store disposition';
    END IF;

    INSERT INTO control.terminal_transactions (
        transaction_id,
        project_id,
        project_snapshot_id,
        repository_owner,
        aggregate_key_digest,
        request_digest,
        daemon_instance_id,
        daemon_epoch,
        admission_mode,
        authority_revision,
        authority_observation_digest,
        authority_head_digest,
        expected_revision,
        expected_state_digest,
        expected_head_digest,
        domain_command_digest,
        record_set_digest,
        next_state_digest,
        domain_receipt_digest,
        checkpoint_digest,
        outbox_intent_digest,
        disposition,
        before_revision,
        before_state_digest,
        before_head_digest,
        after_revision,
        after_state_digest,
        after_head_digest,
        transaction_digest,
        receipt_digest,
        store_contract_version,
        producer_id,
        producer_version,
        runtime,
        durability,
        database_uuid,
        database_identity_digest,
        schema_version,
        manifest_sha256
    ) VALUES (
        p_transaction_id,
        p_project_id,
        p_project_snapshot_id,
        p_repository_owner,
        p_aggregate_key_digest,
        p_request_digest,
        p_daemon_instance_id,
        p_daemon_epoch,
        p_admission_mode,
        p_authority_revision,
        p_authority_observation_digest,
        p_authority_head_digest,
        p_expected_revision,
        p_expected_state_digest,
        p_expected_head_digest,
        p_domain_command_digest,
        p_record_set_digest,
        p_next_state_digest,
        p_domain_receipt_digest,
        p_checkpoint_digest,
        p_outbox_intent_digest,
        p_disposition,
        p_before_revision,
        p_before_state_digest,
        p_before_head_digest,
        p_after_revision,
        p_after_state_digest,
        p_after_head_digest,
        p_transaction_digest,
        p_receipt_digest,
        p_store_contract_version,
        'lattice-postgres-store',
        '1.0',
        'LIVE',
        'DURABLE_POSTGRES',
        p_database_uuid,
        p_database_identity_digest,
        p_schema_version,
        p_manifest_sha256
    );

    RETURN 'FINALIZED';
END;
$lattice_store_finalize_v4$;

CREATE FUNCTION control.store_current_head_v4(
    p_global_schema_version smallint,
    p_global_manifest_sha256 text,
    p_project_id text,
    p_project_snapshot_id text,
    p_repository_owner text,
    p_aggregate_key_digest bytea
)
RETURNS TABLE (
    database_uuid uuid,
    schema_version smallint,
    manifest_sha256 text,
    head_found boolean,
    physical_revision bigint,
    state_digest bytea,
    head_digest bytea,
    global_schema_version smallint,
    global_manifest_sha256 text
)
LANGUAGE plpgsql
STABLE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_store_current_head_v4$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR p_project_id IS NULL
       OR p_project_id !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
       OR p_project_snapshot_id IS NULL
       OR p_project_snapshot_id !~ '^[a-z0-9._:-]{1,128}$'
       OR p_repository_owner IS NULL
       OR p_repository_owner NOT IN (
            'PROJECT_REGISTRY', 'TASK_LEDGER', 'WRITER_LEASE',
            'APPROVAL_VERIFIER', 'ARTIFACT_STORE'
       )
       OR p_aggregate_key_digest IS NULL
       OR pg_catalog.octet_length(p_aggregate_key_digest) <> 32
       OR p_aggregate_key_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid store scope';
    END IF;

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    RETURN QUERY
    SELECT d.database_uuid,
           2::smallint,
           '4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129'::text,
           h.project_id IS NOT NULL,
           h.physical_revision,
           h.state_digest,
           h.head_digest,
           c.current_schema_version,
           pg_catalog.btrim(c.manifest_sha256::text)
      FROM ONLY control.database_identity AS d
      CROSS JOIN ONLY control.schema_compatibility AS c
      LEFT JOIN ONLY control.physical_heads AS h
        ON h.project_id = p_project_id
       AND h.project_snapshot_id = p_project_snapshot_id
       AND h.repository_owner = p_repository_owner
       AND h.aggregate_key_digest = p_aggregate_key_digest
     WHERE d.singleton = true
       AND c.singleton = true
       AND c.current_schema_version = 4
       AND c.min_reader = 4
       AND c.max_reader = 4
       AND c.min_writer = 4
       AND c.max_writer = 4
       AND pg_catalog.btrim(c.manifest_sha256::text) ~ '^[0-9a-f]{64}$'
       AND pg_catalog.btrim(c.manifest_sha256::text) <> pg_catalog.repeat('0', 64)
       AND v_manifest_entry_count = 5
       AND pg_catalog.btrim(c.manifest_sha256::text) = v_history_manifest_sha256;
END;
$lattice_store_current_head_v4$;

CREATE FUNCTION control.task_ledger_prepare_v2(
    p_global_schema_version smallint,
    p_global_manifest_sha256 text,
    p_stream_id bytea,
    p_command_id text
)
RETURNS TABLE (
    stream_found boolean,
    command_found boolean,
    retained_request_digest bytea,
    retained_receipt_digest bytea,
    retained_base_checkpoint_digest bytea,
    retained_result_checkpoint_digest bytea,
    retained_store_transaction_id text,
    terminal_found boolean,
    physical_state_digest bytea
)
LANGUAGE plpgsql
VOLATILE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_task_ledger_prepare_v2$
DECLARE
    v_stream_found boolean;
    v_command control.task_ledger_commands%ROWTYPE;
    v_terminal_found boolean;
    v_project_id text;
    v_project_snapshot_id text;
    v_physical_count bigint;
    v_physical_state_digest bytea;
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
       OR p_stream_id IS NULL
       OR pg_catalog.octet_length(p_stream_id) <> 32
       OR p_stream_id = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_command_id IS NULL
       OR p_command_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid ledger prepare key';
    END IF;

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF v_manifest_entry_count IS DISTINCT FROM 5 OR (
        SELECT pg_catalog.count(*)
          FROM ONLY control.schema_compatibility AS c
         WHERE c.singleton = true
           AND c.current_schema_version = 4
           AND c.min_reader = 4 AND c.max_reader = 4
           AND c.min_writer = 4 AND c.max_writer = 4
           AND pg_catalog.btrim(c.manifest_sha256::text) ~ '^[0-9a-f]{64}$'
           AND pg_catalog.btrim(c.manifest_sha256::text) <> pg_catalog.repeat('0', 64)
           AND pg_catalog.btrim(c.manifest_sha256::text) = v_history_manifest_sha256
    ) <> 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'ledger schema not current';
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(
            'lattice.task-ledger.stream.v1:' || pg_catalog.encode(p_stream_id, 'hex'),
            0
        )
    );

    SELECT s.project_id, s.project_snapshot_id
      INTO v_project_id, v_project_snapshot_id
      FROM ONLY control.task_ledger_streams AS s
     WHERE s.stream_id = p_stream_id
     FOR UPDATE OF s;
    v_stream_found := FOUND;

    SELECT c.*
      INTO v_command
      FROM ONLY control.task_ledger_commands AS c
     WHERE c.stream_id = p_stream_id
       AND c.command_id = p_command_id
     FOR UPDATE OF c;

    IF FOUND THEN
        SELECT EXISTS (
            SELECT 1
              FROM ONLY control.terminal_transactions AS t
             WHERE t.transaction_id = v_command.store_transaction_id
        )
          INTO v_terminal_found;
    ELSE
        v_terminal_found := false;
    END IF;

    SELECT pg_catalog.count(*)
      INTO v_physical_count
      FROM ONLY control.physical_heads AS h
     WHERE h.repository_owner = 'TASK_LEDGER'
       AND h.aggregate_key_digest = p_stream_id;
    IF v_physical_count > 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'ledger physical scope corrupt';
    END IF;

    SELECT h.state_digest
      INTO v_physical_state_digest
      FROM ONLY control.physical_heads AS h
     WHERE h.repository_owner = 'TASK_LEDGER'
       AND h.aggregate_key_digest = p_stream_id
       AND (
            NOT v_stream_found
            OR (
                h.project_id = v_project_id
                AND h.project_snapshot_id = v_project_snapshot_id
            )
       )
     FOR SHARE OF h;
    IF v_physical_count = 1 AND NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'ledger physical scope corrupt';
    END IF;

    RETURN QUERY SELECT
        v_stream_found,
        v_command.command_id IS NOT NULL,
        v_command.request_digest,
        v_command.receipt_digest,
        v_command.base_checkpoint_digest,
        v_command.result_checkpoint_digest,
        v_command.store_transaction_id::text,
        v_terminal_found,
        v_physical_state_digest;
END;
$lattice_task_ledger_prepare_v2$;

CREATE FUNCTION control.task_ledger_read_head_v2(
    p_global_schema_version smallint,
    p_global_manifest_sha256 text,
    p_stream_id bytea,
    p_expected_project_id text,
    p_expected_project_snapshot_id text
)
RETURNS TABLE (
    stream_id bytea,
    ledger_schema_version text,
    head_contract_version smallint,
    producer_id text,
    producer_version text,
    runtime text,
    project_id text,
    project_snapshot_id text,
    task_id text,
    task_revision text,
    task_spec_digest bytea,
    accounting_currency text,
    sequence text,
    last_event_digest bytea,
    resource_revision text,
    resource_projection_digest bytea,
    head_digest bytea,
    active_agents text,
    active_implementers text,
    elapsed_seconds text,
    attempt_number text,
    used_model_calls text,
    used_external_cost text,
    retained_event_count text,
    retained_command_count text,
    retained_outbox_count text,
    checkpoint_schema_version text,
    checkpoint_digest bytea,
    actual_event_count text,
    actual_command_count text,
    actual_outbox_count text,
    physical_head_found boolean,
    physical_revision bigint,
    physical_state_digest bytea,
    physical_head_digest bytea,
    global_schema_version smallint,
    global_manifest_sha256 text
)
LANGUAGE plpgsql
STABLE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_task_ledger_read_head_v2$
DECLARE
    v_stream_found boolean;
    v_project_id text;
    v_project_snapshot_id text;
    v_physical_count bigint;
    v_global_schema_version smallint;
    v_global_manifest_sha256 text;
    v_min_reader smallint;
    v_max_reader smallint;
    v_min_writer smallint;
    v_max_writer smallint;
    v_compatibility_found boolean;
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR p_stream_id IS NULL
       OR pg_catalog.octet_length(p_stream_id) <> 32
       OR p_stream_id = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_expected_project_id IS NULL
       OR p_expected_project_id !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
       OR p_expected_project_snapshot_id IS NULL
       OR p_expected_project_snapshot_id !~ '^[a-z0-9._:-]{1,128}$'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid ledger stream';
    END IF;

    SELECT c.current_schema_version,
           pg_catalog.btrim(c.manifest_sha256::text),
           c.min_reader,
           c.max_reader,
           c.min_writer,
           c.max_writer
      INTO v_global_schema_version,
           v_global_manifest_sha256,
           v_min_reader,
           v_max_reader,
           v_min_writer,
           v_max_writer
      FROM ONLY control.schema_compatibility AS c
     WHERE c.singleton = true;
    v_compatibility_found := FOUND;

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF NOT v_compatibility_found
       OR v_global_schema_version IS DISTINCT FROM 4
       OR v_min_reader IS DISTINCT FROM 4
       OR v_max_reader IS DISTINCT FROM 4
       OR v_min_writer IS DISTINCT FROM 4
       OR v_max_writer IS DISTINCT FROM 4
       OR v_global_manifest_sha256 IS NULL
       OR v_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR v_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM v_global_manifest_sha256
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'ledger schema not current';
    END IF;

    SELECT s.project_id, s.project_snapshot_id
      INTO v_project_id, v_project_snapshot_id
      FROM ONLY control.task_ledger_streams AS s
     WHERE s.stream_id = p_stream_id;
    v_stream_found := FOUND;
    IF v_stream_found AND (
        v_project_id IS DISTINCT FROM p_expected_project_id
        OR v_project_snapshot_id IS DISTINCT FROM p_expected_project_snapshot_id
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'ledger stream scope corrupt';
    END IF;

    SELECT pg_catalog.count(*)
      INTO v_physical_count
      FROM ONLY control.physical_heads AS h
     WHERE h.repository_owner = 'TASK_LEDGER'
       AND h.aggregate_key_digest = p_stream_id;
    IF v_physical_count > 1
       OR (v_physical_count = 1 AND NOT EXISTS (
            SELECT 1
              FROM ONLY control.physical_heads AS h
             WHERE h.repository_owner = 'TASK_LEDGER'
               AND h.aggregate_key_digest = p_stream_id
               AND h.project_id = p_expected_project_id
               AND h.project_snapshot_id = p_expected_project_snapshot_id
       ))
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'ledger physical scope corrupt';
    END IF;

    RETURN QUERY
    SELECT s.stream_id,
           s.ledger_schema_version::text,
           s.head_contract_version,
           s.producer_id::text,
           s.producer_version::text,
           s.runtime::text,
           s.project_id::text,
           s.project_snapshot_id::text,
           s.task_id::text,
           s.task_revision::text,
           s.task_spec_digest,
           s.accounting_currency::text,
           s.sequence::text,
           s.last_event_digest,
           s.resource_revision::text,
           s.resource_projection_digest,
           s.head_digest,
           s.active_agents::text,
           s.active_implementers::text,
           s.elapsed_seconds::text,
           s.attempt_number::text,
           s.used_model_calls::text,
           s.used_external_cost::text,
           s.event_count::text,
           s.command_count::text,
           s.outbox_count::text,
           s.checkpoint_schema_version::text,
           s.checkpoint_digest,
           (SELECT pg_catalog.count(*)::text
              FROM ONLY control.task_ledger_events AS e
             WHERE e.stream_id = s.stream_id),
           (SELECT pg_catalog.count(*)::text
              FROM ONLY control.task_ledger_commands AS c
             WHERE c.stream_id = s.stream_id),
           (SELECT pg_catalog.count(*)::text
              FROM ONLY control.task_ledger_outbox AS o
             WHERE o.stream_id = s.stream_id),
           h.project_id IS NOT NULL,
           h.physical_revision,
           h.state_digest,
           h.head_digest,
           v_global_schema_version,
           v_global_manifest_sha256
      FROM ONLY control.task_ledger_streams AS s
      LEFT JOIN ONLY control.physical_heads AS h
        ON h.project_id = p_expected_project_id
       AND h.project_snapshot_id = p_expected_project_snapshot_id
       AND h.repository_owner = 'TASK_LEDGER'
       AND h.aggregate_key_digest = s.stream_id
     WHERE s.stream_id = p_stream_id;
END;
$lattice_task_ledger_read_head_v2$;

CREATE FUNCTION control.task_ledger_read_events_v2(
    p_global_schema_version smallint,
    p_global_manifest_sha256 text,
    p_stream_id bytea
)
RETURNS TABLE (
    stream_id bytea,
    event_sequence text,
    event_schema_version text,
    previous_event_digest bytea,
    command_id text,
    request_digest bytea,
    correlation_id text,
    occurred_at text,
    event_kind text,
    actor_id text,
    action_id text,
    audit_outcome text,
    reason_code text,
    subject_digest bytea,
    diagnostic jsonb,
    has_resource_snapshot boolean,
    resource_active_agents text,
    resource_active_implementers text,
    resource_elapsed_seconds text,
    resource_attempt_number text,
    resource_used_model_calls text,
    resource_used_external_cost text,
    resource_revision text,
    resource_projection_digest bytea,
    event_digest bytea,
    outbox_found boolean,
    admission_digest bytea,
    admission_schema_version text,
    admission_state text,
    admission_intent_digest bytea,
    admission_occurred_at text
)
LANGUAGE plpgsql
STABLE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_task_ledger_read_events_v2$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR p_stream_id IS NULL
       OR pg_catalog.octet_length(p_stream_id) <> 32
       OR p_stream_id = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid ledger stream';
    END IF;

    RETURN QUERY
    SELECT e.stream_id,
           e.sequence::text,
           e.event_schema_version::text,
           e.previous_event_digest,
           e.command_id::text,
           e.request_digest,
           e.correlation_id::text,
           e.occurred_at::text,
           e.event_kind::text,
           e.actor_id::text,
           e.action_id::text,
           e.audit_outcome::text,
           e.reason_code::text,
           e.subject_digest,
           e.diagnostic,
           e.has_resource_snapshot,
           e.resource_active_agents::text,
           e.resource_active_implementers::text,
           e.resource_elapsed_seconds::text,
           e.resource_attempt_number::text,
           e.resource_used_model_calls::text,
           e.resource_used_external_cost::text,
           e.resource_revision::text,
           e.resource_projection_digest,
           e.event_digest,
           o.admission_digest IS NOT NULL,
           o.admission_digest,
           o.admission_schema_version::text,
           o.admission_state::text,
           o.intent_digest,
           o.occurred_at::text
      FROM ONLY control.task_ledger_events AS e
       LEFT JOIN ONLY control.task_ledger_outbox AS o
         ON o.stream_id = e.stream_id
        AND o.event_sequence = e.sequence
        AND o.event_digest = e.event_digest
        AND o.command_id = e.command_id
        AND o.request_digest = e.request_digest
      WHERE e.stream_id = p_stream_id
     ORDER BY e.sequence;
END;
$lattice_task_ledger_read_events_v2$;

CREATE FUNCTION control.task_ledger_read_commands_v2(
    p_global_schema_version smallint,
    p_global_manifest_sha256 text,
    p_stream_id bytea
)
RETURNS TABLE (
    stream_id bytea,
    command_id text,
    request_schema_version text,
    request_digest bytea,
    expected_sequence text,
    expected_last_event_digest bytea,
    expected_resource_revision text,
    expected_resource_projection_digest bytea,
    expected_head_digest bytea,
    correlation_id text,
    occurred_at text,
    event_kind text,
    actor_id text,
    action_id text,
    audit_outcome text,
    reason_code text,
    subject_digest bytea,
    diagnostic jsonb,
    has_resource_snapshot boolean,
    resource_active_agents text,
    resource_active_implementers text,
    resource_elapsed_seconds text,
    resource_attempt_number text,
    resource_used_model_calls text,
    resource_used_external_cost text,
    receipt_schema_version text,
    before_sequence text,
    before_last_event_digest bytea,
    before_resource_revision text,
    before_resource_projection_digest bytea,
    before_head_digest bytea,
    after_sequence text,
    after_last_event_digest bytea,
    after_resource_revision text,
    after_resource_projection_digest bytea,
    after_head_digest bytea,
    command_outcome text,
    denial_reason text,
    event_digest bytea,
    command_receipt_digest bytea,
    base_checkpoint_digest bytea,
    result_checkpoint_digest bytea,
    command_record_set_digest bytea,
    store_transaction_id text,
    store_found boolean,
    store_contract_version smallint,
    store_producer_id text,
    store_producer_version text,
    store_runtime text,
    store_durability text,
    store_database_uuid uuid,
    store_database_identity_digest bytea,
    store_schema_version smallint,
    store_manifest_sha256 text,
    store_project_id text,
    store_project_snapshot_id text,
    store_repository_owner text,
    store_aggregate_key_digest bytea,
    store_request_digest bytea,
    store_daemon_instance_id text,
    store_daemon_epoch bigint,
    store_admission_mode text,
    store_authority_revision bigint,
    store_authority_observation_digest bytea,
    store_authority_head_digest bytea,
    store_expected_revision bigint,
    store_expected_state_digest bytea,
    store_expected_head_digest bytea,
    store_domain_command_digest bytea,
    store_record_set_digest bytea,
    store_next_state_digest bytea,
    store_domain_receipt_digest bytea,
    store_checkpoint_digest bytea,
    store_outbox_intent_digest bytea,
    store_disposition text,
    store_before_revision bigint,
    store_before_state_digest bytea,
    store_before_head_digest bytea,
    store_after_revision bigint,
    store_after_state_digest bytea,
    store_after_head_digest bytea,
    store_transaction_digest bytea,
    store_receipt_digest bytea
)
LANGUAGE plpgsql
STABLE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_task_ledger_read_commands_v2$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR p_stream_id IS NULL
       OR pg_catalog.octet_length(p_stream_id) <> 32
       OR p_stream_id = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid ledger stream';
    END IF;

    RETURN QUERY
    SELECT c.stream_id,
           c.command_id::text,
           c.request_schema_version::text,
           c.request_digest,
           c.expected_sequence::text,
           c.expected_last_event_digest,
           c.expected_resource_revision::text,
           c.expected_resource_projection_digest,
           c.expected_head_digest,
           c.correlation_id::text,
           c.occurred_at::text,
           c.event_kind::text,
           c.actor_id::text,
           c.action_id::text,
           c.audit_outcome::text,
           c.reason_code::text,
           c.subject_digest,
           c.diagnostic,
           c.has_resource_snapshot,
           c.resource_active_agents::text,
           c.resource_active_implementers::text,
           c.resource_elapsed_seconds::text,
           c.resource_attempt_number::text,
           c.resource_used_model_calls::text,
           c.resource_used_external_cost::text,
           c.receipt_schema_version::text,
           c.before_sequence::text,
           c.before_last_event_digest,
           c.before_resource_revision::text,
           c.before_resource_projection_digest,
           c.before_head_digest,
           c.after_sequence::text,
           c.after_last_event_digest,
           c.after_resource_revision::text,
           c.after_resource_projection_digest,
           c.after_head_digest,
           c.command_outcome::text,
           c.denial_reason::text,
           c.event_digest,
           c.receipt_digest,
           c.base_checkpoint_digest,
           c.result_checkpoint_digest,
           c.record_set_digest,
           c.store_transaction_id::text,
           t.transaction_id IS NOT NULL,
           t.store_contract_version,
           t.producer_id::text,
           t.producer_version::text,
           t.runtime::text,
           t.durability::text,
           t.database_uuid,
           t.database_identity_digest,
           t.schema_version,
           pg_catalog.btrim(t.manifest_sha256::text),
           t.project_id::text,
           t.project_snapshot_id::text,
           t.repository_owner::text,
           t.aggregate_key_digest,
           t.request_digest,
           t.daemon_instance_id::text,
           t.daemon_epoch,
           t.admission_mode::text,
           t.authority_revision,
           t.authority_observation_digest,
           t.authority_head_digest,
           t.expected_revision,
           t.expected_state_digest,
           t.expected_head_digest,
           t.domain_command_digest,
           t.record_set_digest,
           t.next_state_digest,
           t.domain_receipt_digest,
           t.checkpoint_digest,
           t.outbox_intent_digest,
           t.disposition::text,
           t.before_revision,
           t.before_state_digest,
           t.before_head_digest,
           t.after_revision,
           t.after_state_digest,
           t.after_head_digest,
           t.transaction_digest,
           t.receipt_digest
      FROM ONLY control.task_ledger_commands AS c
      LEFT JOIN ONLY control.terminal_transactions AS t
        ON t.transaction_id = c.store_transaction_id
     WHERE c.stream_id = p_stream_id
     ORDER BY c.command_id;
END;
$lattice_task_ledger_read_commands_v2$;

CREATE FUNCTION control.task_ledger_finalize_v2(
    p_global_schema_version smallint,
    p_global_manifest_sha256 text,
    p_stream_id bytea,
    p_project_id text,
    p_project_snapshot_id text,
    p_task_id text,
    p_task_revision text,
    p_task_spec_digest bytea,
    p_accounting_currency text,
    p_next_sequence text,
    p_next_last_event_digest bytea,
    p_next_resource_revision text,
    p_next_resource_projection_digest bytea,
    p_next_head_digest bytea,
    p_next_active_agents text,
    p_next_active_implementers text,
    p_next_elapsed_seconds text,
    p_next_attempt_number text,
    p_next_used_model_calls text,
    p_next_used_external_cost text,
    p_next_event_count text,
    p_next_command_count text,
    p_next_outbox_count text,
    p_base_checkpoint_digest bytea,
    p_next_checkpoint_digest bytea,
    p_command_id text,
    p_request_digest bytea,
    p_expected_sequence text,
    p_expected_last_event_digest bytea,
    p_expected_resource_revision text,
    p_expected_resource_projection_digest bytea,
    p_expected_head_digest bytea,
    p_correlation_id text,
    p_occurred_at text,
    p_event_kind text,
    p_actor_id text,
    p_action_id text,
    p_audit_outcome text,
    p_reason_code text,
    p_subject_digest bytea,
    p_diagnostic jsonb,
    p_has_resource_snapshot boolean,
    p_resource_active_agents text,
    p_resource_active_implementers text,
    p_resource_elapsed_seconds text,
    p_resource_attempt_number text,
    p_resource_used_model_calls text,
    p_resource_used_external_cost text,
    p_before_sequence text,
    p_before_last_event_digest bytea,
    p_before_resource_revision text,
    p_before_resource_projection_digest bytea,
    p_before_head_digest bytea,
    p_after_sequence text,
    p_after_last_event_digest bytea,
    p_after_resource_revision text,
    p_after_resource_projection_digest bytea,
    p_after_head_digest bytea,
    p_command_outcome text,
    p_denial_reason text,
    p_event_digest bytea,
    p_receipt_digest bytea,
    p_record_set_digest bytea,
    p_store_transaction_id text,
    p_append_event boolean,
    p_event_sequence text,
    p_previous_event_digest bytea,
    p_event_resource_revision text,
    p_event_resource_projection_digest bytea,
    p_admit_outbox boolean,
    p_admission_digest bytea,
    p_intent_digest bytea
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
AS $lattice_task_ledger_finalize_v2$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
    v_stream control.task_ledger_streams%ROWTYPE;
    v_terminal control.terminal_transactions%ROWTYPE;
    v_stream_found boolean;
    v_terminal_found boolean;
    v_terminal_current_xact boolean;
    v_rows bigint;
    v_actual_events bigint;
    v_actual_commands bigint;
    v_actual_outbox bigint;
    v_text text;
    v_digest bytea;
    v_zero bytea := pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex');
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
       OR p_stream_id IS NULL
       OR pg_catalog.octet_length(p_stream_id) <> 32
       OR p_stream_id = v_zero
       OR p_project_id IS NULL
       OR p_project_id !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
       OR p_project_snapshot_id IS NULL
       OR p_project_snapshot_id !~ '^[a-z0-9._:-]{1,128}$'
       OR p_task_id IS NULL
       OR p_task_id !~ '^TASK-[A-Z0-9][A-Z0-9_-]{2,63}$'
       OR p_accounting_currency IS NULL
       OR p_accounting_currency !~ '^[A-Z]{3}$'
       OR p_command_id IS NULL
       OR p_command_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR p_correlation_id IS NULL
       OR p_correlation_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR p_actor_id IS NULL
       OR p_actor_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR p_action_id IS NULL
       OR p_action_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR p_reason_code IS NULL
       OR p_reason_code !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR p_occurred_at IS NULL
       OR p_occurred_at !~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
       OR p_event_kind IS NULL
       OR p_event_kind NOT IN (
            'TASK_CREATED', 'STATE_TRANSITION', 'POLICY_DECISION',
            'RESOURCE_SNAPSHOT', 'EFFECT_INTENT', 'EFFECT_OUTCOME',
            'EVIDENCE_RECORDED'
       )
       OR p_audit_outcome IS NULL
       OR p_audit_outcome NOT IN (
            'RECORDED', 'ALLOWED', 'DENIED', 'PASSED', 'FAILED',
            'BLOCKED', 'CANCELLED'
       )
       OR p_command_outcome IS NULL
       OR p_command_outcome NOT IN ('APPENDED', 'DENIED')
       OR p_store_transaction_id IS NULL
       OR p_store_transaction_id !~ '^task-ledger-v1:[0-9a-f]{64}$'
       OR p_diagnostic IS NULL
       OR pg_catalog.octet_length(p_diagnostic::text) > 65536
       OR pg_catalog.jsonb_path_exists(
            p_diagnostic,
            'strict $.** ? (@.type() == "number")'
       )
       OR p_has_resource_snapshot IS NULL
       OR p_append_event IS NULL
       OR p_admit_outbox IS NULL
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid ledger finalization';
    END IF;

    FOREACH v_text IN ARRAY ARRAY[
        p_task_revision,
        p_next_sequence,
        p_next_resource_revision,
        p_next_active_agents,
        p_next_active_implementers,
        p_next_elapsed_seconds,
        p_next_attempt_number,
        p_next_used_model_calls,
        p_next_event_count,
        p_next_command_count,
        p_next_outbox_count,
        p_expected_sequence,
        p_expected_resource_revision,
        p_resource_active_agents,
        p_resource_active_implementers,
        p_resource_elapsed_seconds,
        p_resource_attempt_number,
        p_resource_used_model_calls,
        p_before_sequence,
        p_before_resource_revision,
        p_after_sequence,
        p_after_resource_revision,
        p_event_sequence,
        p_event_resource_revision
    ]
    LOOP
        IF v_text IS NULL
           OR v_text !~ '^(0|[1-9][0-9]{0,19})$'
           OR v_text::numeric > 18446744073709551615
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid ledger u64';
        END IF;
    END LOOP;

    IF p_task_revision::numeric < 1
       OR p_next_active_implementers::numeric > p_next_active_agents::numeric
       OR p_resource_active_implementers::numeric > p_resource_active_agents::numeric
       OR p_next_used_external_cost IS NULL
       OR pg_catalog.octet_length(p_next_used_external_cost) NOT BETWEEN 1 AND 256
       OR p_next_used_external_cost !~ '^(0|[1-9][0-9]{0,126})(\.[0-9]{0,127}[1-9])?$'
       OR p_resource_used_external_cost IS NULL
       OR pg_catalog.octet_length(p_resource_used_external_cost) NOT BETWEEN 1 AND 256
       OR p_resource_used_external_cost !~ '^(0|[1-9][0-9]{0,126})(\.[0-9]{0,127}[1-9])?$'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid ledger resource values';
    END IF;

    FOREACH v_digest IN ARRAY ARRAY[
        p_task_spec_digest,
        p_next_last_event_digest,
        p_next_resource_projection_digest,
        p_next_head_digest,
        p_base_checkpoint_digest,
        p_next_checkpoint_digest,
        p_request_digest,
        p_expected_last_event_digest,
        p_expected_resource_projection_digest,
        p_expected_head_digest,
        p_subject_digest,
        p_before_last_event_digest,
        p_before_resource_projection_digest,
        p_before_head_digest,
        p_after_last_event_digest,
        p_after_resource_projection_digest,
        p_after_head_digest,
        p_event_digest,
        p_receipt_digest,
        p_record_set_digest,
        p_previous_event_digest,
        p_event_resource_projection_digest,
        p_admission_digest,
        p_intent_digest
    ]
    LOOP
        IF v_digest IS NULL OR pg_catalog.octet_length(v_digest) <> 32 THEN
            RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid ledger digest';
        END IF;
    END LOOP;

    IF p_task_spec_digest = v_zero
       OR p_next_head_digest = v_zero
       OR p_base_checkpoint_digest = v_zero
       OR p_next_checkpoint_digest = v_zero
       OR p_base_checkpoint_digest = p_next_checkpoint_digest
       OR p_request_digest = v_zero
       OR p_expected_head_digest = v_zero
       OR p_subject_digest = v_zero
       OR p_before_head_digest = v_zero
       OR p_after_head_digest = v_zero
       OR p_receipt_digest = v_zero
       OR p_record_set_digest = v_zero
       OR p_next_sequence::numeric <> p_after_sequence::numeric
       OR p_next_last_event_digest IS DISTINCT FROM p_after_last_event_digest
       OR p_next_resource_revision::numeric <> p_after_resource_revision::numeric
       OR p_next_resource_projection_digest IS DISTINCT FROM p_after_resource_projection_digest
       OR p_next_head_digest IS DISTINCT FROM p_after_head_digest
       OR p_next_event_count::numeric <> p_next_sequence::numeric
       OR p_expected_resource_revision::numeric > p_expected_sequence::numeric
       OR p_before_resource_revision::numeric > p_before_sequence::numeric
       OR p_after_resource_revision::numeric > p_after_sequence::numeric
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST02', MESSAGE = 'ledger projection mismatch';
    END IF;

    IF (p_has_resource_snapshot AND p_event_kind <> 'RESOURCE_SNAPSHOT')
       OR (NOT p_has_resource_snapshot AND (
            p_event_kind = 'RESOURCE_SNAPSHOT'
            OR p_resource_active_agents::numeric <> 0
            OR p_resource_active_implementers::numeric <> 0
            OR p_resource_elapsed_seconds::numeric <> 0
            OR p_resource_attempt_number::numeric <> 0
            OR p_resource_used_model_calls::numeric <> 0
            OR p_resource_used_external_cost <> '0'
       ))
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST02', MESSAGE = 'ledger resource shape mismatch';
    END IF;

    IF p_append_event THEN
        IF p_command_outcome <> 'APPENDED'
           OR p_denial_reason IS DISTINCT FROM ''
           OR p_event_digest = v_zero
           OR p_event_sequence::numeric <> p_after_sequence::numeric
           OR p_after_last_event_digest IS DISTINCT FROM p_event_digest
           OR p_previous_event_digest IS DISTINCT FROM p_before_last_event_digest
           OR p_event_resource_revision::numeric <> p_after_resource_revision::numeric
           OR p_event_resource_projection_digest IS DISTINCT FROM p_after_resource_projection_digest
           OR p_after_sequence::numeric <> p_before_sequence::numeric + 1
           OR p_expected_sequence::numeric <> p_before_sequence::numeric
           OR p_expected_last_event_digest IS DISTINCT FROM p_before_last_event_digest
           OR p_expected_resource_revision::numeric <> p_before_resource_revision::numeric
           OR p_expected_resource_projection_digest IS DISTINCT FROM p_before_resource_projection_digest
           OR p_expected_head_digest IS DISTINCT FROM p_before_head_digest
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LST02', MESSAGE = 'ledger appended shape mismatch';
        END IF;
    ELSE
        IF p_command_outcome <> 'DENIED'
           OR p_denial_reason NOT IN ('STALE_HEAD', 'SEQUENCE_OVERFLOW')
           OR p_event_digest <> v_zero
           OR p_event_sequence::numeric <> 0
           OR p_previous_event_digest <> v_zero
           OR p_event_resource_revision::numeric <> 0
           OR p_event_resource_projection_digest <> v_zero
           OR p_after_sequence::numeric <> p_before_sequence::numeric
           OR p_after_last_event_digest IS DISTINCT FROM p_before_last_event_digest
           OR p_after_resource_revision::numeric <> p_before_resource_revision::numeric
           OR p_after_resource_projection_digest IS DISTINCT FROM p_before_resource_projection_digest
           OR p_after_head_digest IS DISTINCT FROM p_before_head_digest
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LST02', MESSAGE = 'ledger denial shape mismatch';
        END IF;
    END IF;

    IF p_admit_outbox THEN
        IF NOT p_append_event
           OR p_event_kind <> 'EFFECT_INTENT'
           OR p_audit_outcome <> 'RECORDED'
           OR p_admission_digest = v_zero
           OR p_intent_digest IS DISTINCT FROM p_subject_digest
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LST02', MESSAGE = 'ledger admission shape mismatch';
        END IF;
    ELSIF (
        p_append_event
        AND p_event_kind = 'EFFECT_INTENT'
        AND p_audit_outcome = 'RECORDED'
    ) OR p_admission_digest <> v_zero OR p_intent_digest <> v_zero THEN
        RAISE EXCEPTION USING ERRCODE = 'LST02', MESSAGE = 'unexpected ledger admission';
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(
            'lattice.task-ledger.stream.v1:' || pg_catalog.encode(p_stream_id, 'hex'),
            0
        )
    );

    SELECT s.*
      INTO v_stream
      FROM ONLY control.task_ledger_streams AS s
     WHERE s.stream_id = p_stream_id
     FOR UPDATE OF s;
    v_stream_found := FOUND;

    IF v_stream_found THEN
        IF v_stream.project_id IS DISTINCT FROM p_project_id
           OR v_stream.project_snapshot_id IS DISTINCT FROM p_project_snapshot_id
           OR v_stream.task_id IS DISTINCT FROM p_task_id
           OR v_stream.task_revision <> p_task_revision::numeric
           OR v_stream.task_spec_digest IS DISTINCT FROM p_task_spec_digest
           OR pg_catalog.btrim(v_stream.accounting_currency::text) IS DISTINCT FROM p_accounting_currency
           OR v_stream.checkpoint_digest IS DISTINCT FROM p_base_checkpoint_digest
           OR v_stream.sequence <> p_before_sequence::numeric
           OR v_stream.last_event_digest IS DISTINCT FROM p_before_last_event_digest
           OR v_stream.resource_revision <> p_before_resource_revision::numeric
           OR v_stream.resource_projection_digest IS DISTINCT FROM p_before_resource_projection_digest
           OR v_stream.head_digest IS DISTINCT FROM p_before_head_digest
           OR p_next_command_count::numeric <> v_stream.command_count + 1
           OR p_next_event_count::numeric <> v_stream.event_count + (CASE WHEN p_append_event THEN 1 ELSE 0 END)
           OR p_next_outbox_count::numeric <> v_stream.outbox_count + (CASE WHEN p_admit_outbox THEN 1 ELSE 0 END)
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LCP01', MESSAGE = 'ledger checkpoint changed';
        END IF;
        IF p_append_event AND p_has_resource_snapshot THEN
            IF p_next_resource_revision::numeric <> v_stream.resource_revision + 1
               OR p_next_resource_projection_digest = v_zero
               OR p_next_active_agents::numeric <> p_resource_active_agents::numeric
               OR p_next_active_implementers::numeric <> p_resource_active_implementers::numeric
               OR p_next_elapsed_seconds::numeric <> p_resource_elapsed_seconds::numeric
               OR p_next_attempt_number::numeric <> p_resource_attempt_number::numeric
               OR p_next_used_model_calls::numeric <> p_resource_used_model_calls::numeric
               OR p_next_used_external_cost IS DISTINCT FROM p_resource_used_external_cost
            THEN
                RAISE EXCEPTION USING ERRCODE = 'LST02', MESSAGE = 'snapshot projection mismatch';
            END IF;
        ELSIF p_next_resource_revision::numeric <> v_stream.resource_revision
           OR p_next_resource_projection_digest IS DISTINCT FROM v_stream.resource_projection_digest
           OR p_next_active_agents::numeric <> v_stream.active_agents
           OR p_next_active_implementers::numeric <> v_stream.active_implementers
           OR p_next_elapsed_seconds::numeric <> v_stream.elapsed_seconds
           OR p_next_attempt_number::numeric <> v_stream.attempt_number
           OR p_next_used_model_calls::numeric <> v_stream.used_model_calls
           OR p_next_used_external_cost IS DISTINCT FROM v_stream.used_external_cost
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LST02', MESSAGE = 'non-snapshot changed resources';
        END IF;
    ELSE
        IF p_before_sequence::numeric <> 0
           OR p_before_last_event_digest <> v_zero
           OR p_before_resource_revision::numeric <> 0
           OR p_before_resource_projection_digest <> v_zero
           OR p_next_command_count::numeric <> 1
           OR p_next_event_count::numeric <> (CASE WHEN p_append_event THEN 1 ELSE 0 END)
           OR p_next_outbox_count::numeric <> (CASE WHEN p_admit_outbox THEN 1 ELSE 0 END)
           OR (p_append_event AND p_has_resource_snapshot AND (
                p_next_resource_revision::numeric <> 1
                OR p_next_resource_projection_digest = v_zero
                OR p_next_active_agents::numeric <> p_resource_active_agents::numeric
                OR p_next_active_implementers::numeric <> p_resource_active_implementers::numeric
                OR p_next_elapsed_seconds::numeric <> p_resource_elapsed_seconds::numeric
                OR p_next_attempt_number::numeric <> p_resource_attempt_number::numeric
                OR p_next_used_model_calls::numeric <> p_resource_used_model_calls::numeric
                OR p_next_used_external_cost IS DISTINCT FROM p_resource_used_external_cost
           ))
           OR ((NOT p_append_event OR NOT p_has_resource_snapshot) AND (
                p_next_resource_revision::numeric <> 0
                OR p_next_resource_projection_digest <> v_zero
                OR p_next_active_agents::numeric <> 0
                OR p_next_active_implementers::numeric <> 0
                OR p_next_elapsed_seconds::numeric <> 0
                OR p_next_attempt_number::numeric <> 0
                OR p_next_used_model_calls::numeric <> 0
                OR p_next_used_external_cost <> '0'
           ))
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LCP01', MESSAGE = 'invalid structural zero base';
        END IF;
    END IF;

    SELECT t.*
      INTO v_terminal
      FROM ONLY control.terminal_transactions AS t
     WHERE t.transaction_id = p_store_transaction_id
     FOR SHARE OF t;
    v_terminal_found := FOUND;
    IF v_terminal_found THEN
        SELECT t.xmin = pg_catalog.pg_current_xact_id()::xid
          INTO v_terminal_current_xact
          FROM ONLY control.terminal_transactions AS t
         WHERE t.transaction_id = p_store_transaction_id;
    ELSE
        v_terminal_current_xact := false;
    END IF;
    IF NOT v_terminal_found
       OR v_terminal_current_xact IS DISTINCT FROM true
       OR v_terminal.store_contract_version IS DISTINCT FROM 2
       OR v_terminal.project_id IS DISTINCT FROM p_project_id
       OR v_terminal.project_snapshot_id IS DISTINCT FROM p_project_snapshot_id
       OR v_terminal.repository_owner IS DISTINCT FROM 'TASK_LEDGER'
       OR v_terminal.aggregate_key_digest IS DISTINCT FROM p_stream_id
       OR v_terminal.domain_command_digest IS DISTINCT FROM p_request_digest
       OR v_terminal.record_set_digest IS DISTINCT FROM p_record_set_digest
       OR v_terminal.next_state_digest IS DISTINCT FROM p_next_checkpoint_digest
       OR v_terminal.domain_receipt_digest IS DISTINCT FROM p_receipt_digest
       OR v_terminal.checkpoint_digest IS DISTINCT FROM p_next_checkpoint_digest
       OR v_terminal.outbox_intent_digest IS DISTINCT FROM
            (CASE WHEN p_admit_outbox THEN p_admission_digest ELSE NULL::bytea END)
       OR v_terminal.disposition IS DISTINCT FROM 'APPLIED'
       OR v_terminal.expected_revision::numeric IS DISTINCT FROM p_next_command_count::numeric - 1
       OR v_terminal.before_revision::numeric IS DISTINCT FROM p_next_command_count::numeric - 1
       OR v_terminal.after_revision::numeric IS DISTINCT FROM p_next_command_count::numeric
       OR (v_stream_found AND (
            v_terminal.expected_state_digest IS DISTINCT FROM p_base_checkpoint_digest
            OR v_terminal.before_state_digest IS DISTINCT FROM p_base_checkpoint_digest
       ))
       OR v_terminal.after_state_digest IS DISTINCT FROM p_next_checkpoint_digest
       OR v_terminal.schema_version IS DISTINCT FROM 2
       OR pg_catalog.btrim(v_terminal.manifest_sha256::text) IS DISTINCT FROM
            '4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'ledger store pair corrupt';
    END IF;

    IF v_stream_found THEN
        UPDATE ONLY control.task_ledger_streams
           SET sequence = p_next_sequence::numeric,
               last_event_digest = p_next_last_event_digest,
               resource_revision = p_next_resource_revision::numeric,
               resource_projection_digest = p_next_resource_projection_digest,
               head_digest = p_next_head_digest,
               active_agents = p_next_active_agents::numeric,
               active_implementers = p_next_active_implementers::numeric,
               elapsed_seconds = p_next_elapsed_seconds::numeric,
               attempt_number = p_next_attempt_number::numeric,
               used_model_calls = p_next_used_model_calls::numeric,
               used_external_cost = p_next_used_external_cost,
               event_count = p_next_event_count::numeric,
               command_count = p_next_command_count::numeric,
               outbox_count = p_next_outbox_count::numeric,
               checkpoint_digest = p_next_checkpoint_digest
         WHERE stream_id = p_stream_id
           AND checkpoint_digest = p_base_checkpoint_digest;
        GET DIAGNOSTICS v_rows = ROW_COUNT;
        IF v_rows <> 1 THEN
            RAISE EXCEPTION USING ERRCODE = 'LCP01', MESSAGE = 'ledger checkpoint changed';
        END IF;
    ELSE
        INSERT INTO control.task_ledger_streams (
            stream_id, ledger_schema_version, head_contract_version,
            producer_id, producer_version, runtime, project_id,
            project_snapshot_id, task_id, task_revision, task_spec_digest,
            accounting_currency, sequence, last_event_digest, resource_revision,
            resource_projection_digest, head_digest, active_agents,
            active_implementers, elapsed_seconds, attempt_number, used_model_calls,
            used_external_cost, event_count, command_count, outbox_count,
            checkpoint_schema_version, checkpoint_digest
        ) VALUES (
            p_stream_id, '2.0', 1, 'lattice-task-ledger', '2.0', 'LIVE',
            p_project_id, p_project_snapshot_id, p_task_id, p_task_revision::numeric,
            p_task_spec_digest, p_accounting_currency, p_next_sequence::numeric,
            p_next_last_event_digest, p_next_resource_revision::numeric,
            p_next_resource_projection_digest, p_next_head_digest,
            p_next_active_agents::numeric, p_next_active_implementers::numeric,
            p_next_elapsed_seconds::numeric, p_next_attempt_number::numeric,
            p_next_used_model_calls::numeric, p_next_used_external_cost,
            p_next_event_count::numeric, p_next_command_count::numeric,
            p_next_outbox_count::numeric, '1.0', p_next_checkpoint_digest
        );
    END IF;

    INSERT INTO control.task_ledger_commands (
        stream_id, command_id, request_schema_version, request_digest,
        expected_sequence, expected_last_event_digest, expected_resource_revision,
        expected_resource_projection_digest, expected_head_digest, correlation_id,
        occurred_at, event_kind, actor_id, action_id, audit_outcome, reason_code,
        subject_digest, diagnostic, has_resource_snapshot, resource_active_agents,
        resource_active_implementers, resource_elapsed_seconds,
        resource_attempt_number, resource_used_model_calls,
        resource_used_external_cost, receipt_schema_version, before_sequence,
        before_last_event_digest, before_resource_revision,
        before_resource_projection_digest, before_head_digest, after_sequence,
        after_last_event_digest, after_resource_revision,
        after_resource_projection_digest, after_head_digest, command_outcome,
        denial_reason, event_digest, receipt_digest, base_checkpoint_digest,
        result_checkpoint_digest, record_set_digest, store_transaction_id
    ) VALUES (
        p_stream_id, p_command_id, '2.0', p_request_digest,
        p_expected_sequence::numeric, p_expected_last_event_digest,
        p_expected_resource_revision::numeric, p_expected_resource_projection_digest,
        p_expected_head_digest, p_correlation_id, p_occurred_at, p_event_kind,
        p_actor_id, p_action_id, p_audit_outcome, p_reason_code, p_subject_digest,
        p_diagnostic, p_has_resource_snapshot, p_resource_active_agents::numeric,
        p_resource_active_implementers::numeric, p_resource_elapsed_seconds::numeric,
        p_resource_attempt_number::numeric, p_resource_used_model_calls::numeric,
        p_resource_used_external_cost, '2.0', p_before_sequence::numeric,
        p_before_last_event_digest, p_before_resource_revision::numeric,
        p_before_resource_projection_digest, p_before_head_digest,
        p_after_sequence::numeric, p_after_last_event_digest,
        p_after_resource_revision::numeric, p_after_resource_projection_digest,
        p_after_head_digest, p_command_outcome, p_denial_reason, p_event_digest,
        p_receipt_digest, p_base_checkpoint_digest, p_next_checkpoint_digest,
        p_record_set_digest, p_store_transaction_id
    );

    IF p_append_event THEN
        INSERT INTO control.task_ledger_events (
            stream_id, sequence, event_schema_version, previous_event_digest,
            command_id, request_digest, correlation_id, occurred_at, event_kind,
            actor_id, action_id, audit_outcome, reason_code, subject_digest,
            diagnostic, has_resource_snapshot, resource_active_agents,
            resource_active_implementers, resource_elapsed_seconds,
            resource_attempt_number, resource_used_model_calls,
            resource_used_external_cost, resource_revision,
            resource_projection_digest, event_digest
        ) VALUES (
            p_stream_id, p_event_sequence::numeric, '2.0', p_previous_event_digest,
            p_command_id, p_request_digest, p_correlation_id, p_occurred_at,
            p_event_kind, p_actor_id, p_action_id, p_audit_outcome, p_reason_code,
            p_subject_digest, p_diagnostic, p_has_resource_snapshot,
            p_resource_active_agents::numeric, p_resource_active_implementers::numeric,
            p_resource_elapsed_seconds::numeric, p_resource_attempt_number::numeric,
            p_resource_used_model_calls::numeric, p_resource_used_external_cost,
            p_event_resource_revision::numeric, p_event_resource_projection_digest,
            p_event_digest
        );
    END IF;

    IF p_admit_outbox THEN
        INSERT INTO control.task_ledger_outbox (
            admission_digest, admission_schema_version, admission_state,
            stream_id, event_sequence, event_digest, command_id, request_digest,
            intent_digest, occurred_at
        ) VALUES (
            p_admission_digest, '1.0', 'ADMITTED', p_stream_id,
            p_event_sequence::numeric, p_event_digest, p_command_id,
            p_request_digest, p_intent_digest, p_occurred_at
        );
    END IF;

    SELECT pg_catalog.count(*)
      INTO v_actual_events
      FROM ONLY control.task_ledger_events AS e
     WHERE e.stream_id = p_stream_id;
    SELECT pg_catalog.count(*)
      INTO v_actual_commands
      FROM ONLY control.task_ledger_commands AS c
     WHERE c.stream_id = p_stream_id;
    SELECT pg_catalog.count(*)
      INTO v_actual_outbox
      FROM ONLY control.task_ledger_outbox AS o
     WHERE o.stream_id = p_stream_id;
    IF v_actual_events::numeric <> p_next_event_count::numeric
       OR v_actual_commands::numeric <> p_next_command_count::numeric
       OR v_actual_outbox::numeric <> p_next_outbox_count::numeric
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'ledger row count corrupt';
    END IF;

    RETURN 'FINALIZED';
END;
$lattice_task_ledger_finalize_v2$;

CREATE FUNCTION control.project_registry_prepare_v1(
    p_global_schema_version smallint,
    p_global_manifest_sha256 text,
    p_command_id text,
    p_request_digest bytea,
    p_authority_runtime text,
    p_daemon_instance_id text,
    p_daemon_epoch bigint,
    p_admission_mode text,
    p_authority_revision bigint,
    p_authority_observation_digest bytea,
    p_authority_head_digest bytea,
    p_expected_base_checkpoint_digest bytea
)
RETURNS TABLE (
    prepare_status text, retained_request_digest bytea, retained_result_digest bytea,
    retained_record_set_digest bytea, retained_persistence_receipt_digest bytea,
    retained_base_checkpoint_digest bytea, retained_result_checkpoint_digest bytea,
    current_ordinal bigint, current_observation_count bigint, current_project_count bigint,
    current_command_count bigint, current_reservation_count bigint,
    current_retained_bytes bigint, current_checkpoint_digest bytea
)
LANGUAGE plpgsql
VOLATILE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_project_registry_prepare_v1$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
    v_state control.project_registry_state%ROWTYPE;
    v_command control.project_registry_commands%ROWTYPE;
    v_admission control.runtime_admission%ROWTYPE;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
       OR p_command_id IS NULL OR p_command_id = ''
       OR p_command_id IS DISTINCT FROM pg_catalog.btrim(p_command_id)
       OR p_request_digest IS NULL OR pg_catalog.octet_length(p_request_digest) <> 32
       OR p_authority_runtime NOT IN ('FAKE', 'LIVE')
       OR p_daemon_instance_id IS NULL OR p_daemon_instance_id !~ '^[a-z0-9][a-z0-9._:-]{0,127}$'
       OR p_daemon_epoch IS NULL OR p_daemon_epoch <= 0
       OR p_admission_mode NOT IN ('ACTIVE', 'STOPPED')
       OR p_authority_revision IS NULL OR p_authority_revision <= 0
       OR pg_catalog.octet_length(p_authority_observation_digest) <> 32
       OR pg_catalog.octet_length(p_authority_head_digest) <> 32
       OR pg_catalog.octet_length(p_expected_base_checkpoint_digest) <> 32
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LPR01', MESSAGE = 'invalid registry prepare';
    END IF;

    SELECT * INTO v_state FROM ONLY control.project_registry_state WHERE singleton = true;
    IF NOT FOUND OR v_state.stage_command_id IS NOT NULL THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'registry state corrupt';
    END IF;
    SELECT * INTO v_command FROM ONLY control.project_registry_commands WHERE command_id = p_command_id;
    IF FOUND THEN
        IF v_command.ordinal > v_state.command_ordinal THEN
            RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'registry command not published';
        END IF;
        IF v_command.request_digest IS DISTINCT FROM p_request_digest THEN
            RETURN QUERY SELECT 'COMMAND_CONFLICT'::text, NULL::bytea, NULL::bytea, NULL::bytea,
                NULL::bytea, NULL::bytea, NULL::bytea, v_state.command_ordinal,
                v_state.observation_count, v_state.project_count, v_state.command_count,
                v_state.reservation_count, v_state.retained_bytes, v_state.checkpoint_digest;
            RETURN;
        END IF;
        RETURN QUERY SELECT 'REPLAY'::text, v_command.request_digest, v_command.result_digest,
            v_command.record_set_digest, v_command.persistence_receipt_digest,
            v_command.base_checkpoint_digest, v_command.result_checkpoint_digest,
            v_state.command_ordinal, v_state.observation_count, v_state.project_count,
            v_state.command_count, v_state.reservation_count, v_state.retained_bytes,
            v_state.checkpoint_digest;
        RETURN;
    END IF;

    IF p_authority_runtime IS DISTINCT FROM 'LIVE' THEN
        RAISE EXCEPTION USING ERRCODE = 'LAR01', MESSAGE = 'registry runtime not live';
    END IF;
    IF p_admission_mode IS DISTINCT FROM 'ACTIVE' THEN
        RAISE EXCEPTION USING ERRCODE = 'LAM01', MESSAGE = 'registry admission not active';
    END IF;

    SELECT * INTO v_state FROM ONLY control.project_registry_state WHERE singleton = true FOR UPDATE;
    IF NOT FOUND OR v_state.stage_command_id IS NOT NULL
       OR v_state.checkpoint_digest IS DISTINCT FROM p_expected_base_checkpoint_digest
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCP01', MESSAGE = 'registry checkpoint changed';
    END IF;
    SELECT * INTO v_admission FROM ONLY control.runtime_admission WHERE singleton = true FOR SHARE;
    IF NOT FOUND OR v_admission.admission_mode IS DISTINCT FROM 'ACTIVE'
       OR v_admission.daemon_instance_id IS DISTINCT FROM p_daemon_instance_id
       OR v_admission.daemon_epoch IS DISTINCT FROM p_daemon_epoch
       OR v_admission.authority_revision IS DISTINCT FROM p_authority_revision
       OR v_admission.observation_digest IS DISTINCT FROM p_authority_observation_digest
       OR v_admission.authority_head_digest IS DISTINCT FROM p_authority_head_digest
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAD01', MESSAGE = 'registry authority not current';
    END IF;
    RETURN QUERY SELECT 'NEW'::text, NULL::bytea, NULL::bytea, NULL::bytea, NULL::bytea,
        NULL::bytea, NULL::bytea, v_state.command_ordinal, v_state.observation_count,
        v_state.project_count, v_state.command_count, v_state.reservation_count,
        v_state.retained_bytes, v_state.checkpoint_digest;
END;
$lattice_project_registry_prepare_v1$;

CREATE FUNCTION control.project_registry_read_state_v1(
    p_global_schema_version smallint,
    p_global_manifest_sha256 text
)
RETURNS TABLE (runtime text, command_ordinal bigint, observation_count bigint,
    project_count bigint, command_count bigint, reservation_count bigint,
    retained_bytes bigint, checkpoint_digest bytea, stage_command_id text)
LANGUAGE plpgsql
STABLE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_project_registry_read_state_v1$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    RETURN QUERY
    SELECT s.runtime::text, s.command_ordinal, s.observation_count, s.project_count,
           s.command_count, s.reservation_count, s.retained_bytes, s.checkpoint_digest,
           s.stage_command_id::text
      FROM ONLY control.project_registry_state AS s WHERE s.singleton = true;
END;
$lattice_project_registry_read_state_v1$;

CREATE FUNCTION control.project_registry_read_observations_v1(
    p_global_schema_version smallint,
    p_global_manifest_sha256 text
)
RETURNS TABLE (observation_digest bytea, canonical_root text, root_identity_digest bytea,
    repository_identity_digest bytea, file_identity_digest bytea, primary_ref text,
    primary_ref_storage_digest bytea)
LANGUAGE plpgsql
STABLE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_project_registry_read_observations_v1$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    RETURN QUERY
    SELECT o.observation_digest, o.canonical_root, o.root_identity_digest,
           o.repository_identity_digest, o.file_identity_digest, o.primary_ref::text,
           o.primary_ref_storage_digest
      FROM ONLY control.project_registry_observations AS o ORDER BY o.observation_digest;
END;
$lattice_project_registry_read_observations_v1$;

CREATE FUNCTION control.project_registry_read_projects_v1(
    p_global_schema_version smallint,
    p_global_manifest_sha256 text
)
RETURNS TABLE (project_id text, project_class text, accepted_observation_digest bytea,
    pending_observation_digest bytea, drift_canonical_root boolean, drift_repository boolean,
    drift_file boolean, drift_primary_ref_name boolean, drift_primary_ref_storage boolean,
    authority_contract_version smallint, authority_producer_id text, authority_producer_version text,
    authority_runtime text, authority_snapshot_id text, authority_registry_revision text,
    authority_lifecycle text, authority_primary_ref text,
    authority_primary_ref_storage_digest bytea, authority_observation_digest bytea,
    authority_receipt_digest bytea)
LANGUAGE plpgsql
STABLE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_project_registry_read_projects_v1$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    RETURN QUERY
    SELECT p.project_id::text, p.project_class::text, p.accepted_observation_digest,
           p.pending_observation_digest, p.drift_canonical_root, p.drift_repository,
           p.drift_file, p.drift_primary_ref_name, p.drift_primary_ref_storage,
           p.authority_contract_version, p.authority_producer_id::text,
           p.authority_producer_version::text, p.authority_runtime::text,
           p.authority_snapshot_id, p.authority_registry_revision::text,
           p.authority_lifecycle::text, p.authority_primary_ref::text,
           p.authority_primary_ref_storage_digest, p.authority_observation_digest,
           p.authority_receipt_digest
      FROM ONLY control.project_registry_projects AS p ORDER BY p.project_id;
END;
$lattice_project_registry_read_projects_v1$;

CREATE FUNCTION control.project_registry_read_commands_v1(
    p_global_schema_version smallint,
    p_global_manifest_sha256 text
)
RETURNS SETOF control.project_registry_commands
LANGUAGE plpgsql
STABLE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_project_registry_read_commands_v1$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    RETURN QUERY
    SELECT c.* FROM ONLY control.project_registry_commands AS c ORDER BY c.ordinal;
END;
$lattice_project_registry_read_commands_v1$;

CREATE FUNCTION control.project_registry_read_reservations_v1(
    p_global_schema_version smallint,
    p_global_manifest_sha256 text
)
RETURNS TABLE (dimension text, identity_digest bytea, reservation_status text, project_id text)
LANGUAGE plpgsql
STABLE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_project_registry_read_reservations_v1$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    RETURN QUERY
    SELECT r.dimension::text, r.identity_digest, r.reservation_status::text,
           r.project_id::text FROM ONLY control.project_registry_identity_reservations AS r
      ORDER BY r.dimension, r.identity_digest, r.reservation_status, r.project_id;
END;
$lattice_project_registry_read_reservations_v1$;

CREATE FUNCTION control.project_registry_stage_command_v1(
    p_global_schema_version smallint, p_global_manifest_sha256 text,
    p_ordinal bigint, p_command_id text, p_action text, p_project_id text,
    p_project_class text, p_observation_digest bytea,
    p_before_present boolean, p_before_producer_id text, p_before_producer_version text,
    p_before_runtime text, p_before_project_id text, p_before_snapshot_id text,
    p_before_registry_revision numeric, p_before_lifecycle text, p_before_project_class text,
    p_before_primary_ref text, p_before_primary_ref_storage_digest bytea,
    p_before_observation_digest bytea, p_before_receipt_digest bytea,
    p_decision text, p_evidence_digest bytea, p_request_digest bytea,
    p_outcome text, p_denial_reason text, p_denial_dimension text,
    p_denial_existing_project_id text, p_denial_lifecycle text,
    p_denial_expected_decision text, p_denial_found_decision text,
    p_semantic_before_receipt_digest bytea, p_semantic_after_receipt_digest bytea,
    p_authority_receipt_digest bytea,
    p_drift_canonical_root boolean, p_drift_repository boolean, p_drift_file boolean,
    p_drift_primary_ref_name boolean, p_drift_primary_ref_storage boolean,
    p_result_digest bytea,
    p_base_runtime text, p_base_ordinal bigint, p_base_observation_count bigint,
    p_base_project_count bigint, p_base_command_count bigint, p_base_reservation_count bigint,
    p_base_retained_bytes bigint, p_base_checkpoint_digest bytea,
    p_result_runtime text, p_result_ordinal bigint, p_result_observation_count bigint,
    p_result_project_count bigint, p_result_command_count bigint,
    p_result_reservation_count bigint, p_result_retained_bytes bigint,
    p_result_checkpoint_digest bytea, p_record_set_digest bytea,
    p_authority_runtime text, p_daemon_instance_id text, p_daemon_epoch bigint,
    p_admission_mode text, p_daemon_authority_revision bigint,
    p_daemon_observation_digest bytea, p_daemon_head_digest bytea,
    p_transaction_digest bytea, p_persistence_receipt_digest bytea,
    p_stage_observation boolean, p_canonical_root text, p_root_identity_digest bytea,
    p_repository_identity_digest bytea, p_file_identity_digest bytea,
    p_primary_ref text, p_primary_ref_storage_digest bytea
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
AS $lattice_project_registry_stage_command_v1$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
    v_rows bigint;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR p_ordinal IS NULL OR p_ordinal <= 0 OR p_ordinal <> p_base_ordinal + 1
       OR p_result_ordinal IS DISTINCT FROM p_ordinal
       OR p_command_id IS NULL OR p_request_digest IS NULL OR p_result_digest IS NULL
       OR p_base_runtime IS DISTINCT FROM 'LIVE' OR p_result_runtime IS DISTINCT FROM 'LIVE'
       OR p_authority_runtime IS DISTINCT FROM 'LIVE' OR p_admission_mode IS DISTINCT FROM 'ACTIVE'
       OR p_stage_observation IS NULL
    THEN RAISE EXCEPTION USING ERRCODE = 'LPR01', MESSAGE = 'invalid registry command stage'; END IF;

    IF p_stage_observation THEN
        INSERT INTO control.project_registry_observations (
            observation_digest, canonical_root, root_identity_digest,
            repository_identity_digest, file_identity_digest, primary_ref,
            primary_ref_storage_digest
        ) VALUES (
            p_observation_digest, p_canonical_root, p_root_identity_digest,
            p_repository_identity_digest, p_file_identity_digest, p_primary_ref,
            p_primary_ref_storage_digest
        );
    ELSIF p_observation_digest IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM ONLY control.project_registry_observations AS o
         WHERE o.observation_digest = p_observation_digest
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'registry observation absent';
    END IF;

    INSERT INTO control.project_registry_commands VALUES (
        p_ordinal, p_command_id, p_action, p_project_id, p_project_class, p_observation_digest,
        p_before_present, p_before_producer_id, p_before_producer_version, p_before_runtime,
        p_before_project_id, p_before_snapshot_id, p_before_registry_revision,
        p_before_lifecycle, p_before_project_class, p_before_primary_ref,
        p_before_primary_ref_storage_digest, p_before_observation_digest, p_before_receipt_digest,
        p_decision, p_evidence_digest, p_request_digest, p_outcome, p_denial_reason,
        p_denial_dimension, p_denial_existing_project_id, p_denial_lifecycle,
        p_denial_expected_decision, p_denial_found_decision,
        p_semantic_before_receipt_digest, p_semantic_after_receipt_digest,
        p_authority_receipt_digest, p_drift_canonical_root, p_drift_repository, p_drift_file,
        p_drift_primary_ref_name, p_drift_primary_ref_storage, p_result_digest,
        p_base_runtime, p_base_ordinal, p_base_observation_count, p_base_project_count,
        p_base_command_count, p_base_reservation_count, p_base_retained_bytes,
        p_base_checkpoint_digest, p_result_runtime, p_result_ordinal,
        p_result_observation_count, p_result_project_count, p_result_command_count,
        p_result_reservation_count, p_result_retained_bytes, p_result_checkpoint_digest,
        p_record_set_digest, p_authority_runtime, p_daemon_instance_id, p_daemon_epoch,
        p_admission_mode, p_daemon_authority_revision, p_daemon_observation_digest,
        p_daemon_head_digest, p_transaction_digest, p_persistence_receipt_digest
    );

    UPDATE ONLY control.project_registry_state SET
        stage_command_id = p_command_id, stage_ordinal = p_ordinal,
        stage_base_checkpoint_digest = p_base_checkpoint_digest,
        stage_result_checkpoint_digest = p_result_checkpoint_digest,
        stage_record_set_digest = p_record_set_digest,
        stage_observation = p_stage_observation, stage_project = false,
        stage_reservation_delete_count = 0, stage_reservation_insert_count = 0
     WHERE singleton = true AND stage_command_id IS NULL
       AND command_ordinal = p_base_ordinal AND observation_count = p_base_observation_count
       AND project_count = p_base_project_count AND command_count = p_base_command_count
       AND reservation_count = p_base_reservation_count AND retained_bytes = p_base_retained_bytes
       AND checkpoint_digest = p_base_checkpoint_digest;
    GET DIAGNOSTICS v_rows = ROW_COUNT;
    IF v_rows <> 1 THEN RAISE EXCEPTION USING ERRCODE = 'LCP01', MESSAGE = 'registry stage base changed'; END IF;
    RETURN 'STAGED';
END;
$lattice_project_registry_stage_command_v1$;

CREATE FUNCTION control.project_registry_stage_project_v1(
    p_global_schema_version smallint, p_global_manifest_sha256 text,
    p_project_id text, p_project_class text, p_accepted_observation_digest bytea,
    p_pending_observation_digest bytea, p_drift_canonical_root boolean,
    p_drift_repository boolean, p_drift_file boolean, p_drift_primary_ref_name boolean,
    p_drift_primary_ref_storage boolean, p_authority_contract_version smallint,
    p_authority_producer_id text, p_authority_producer_version text,
    p_authority_runtime text, p_authority_snapshot_id text,
    p_authority_registry_revision numeric, p_authority_lifecycle text,
    p_authority_primary_ref text, p_authority_primary_ref_storage_digest bytea,
    p_authority_observation_digest bytea, p_authority_receipt_digest bytea
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
AS $lattice_project_registry_stage_project_v1$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
    v_rows bigint;
    v_deleted bigint;
    v_inserted bigint;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR p_project_id IS NULL OR p_project_class IS NULL
       OR p_authority_runtime IS DISTINCT FROM 'LIVE'
    THEN RAISE EXCEPTION USING ERRCODE = 'LPR01', MESSAGE = 'invalid registry project stage'; END IF;
    IF (SELECT pg_catalog.count(*) FROM ONLY control.project_registry_state AS s
         WHERE s.singleton = true AND s.stage_command_id IS NOT NULL AND NOT s.stage_project) <> 1
    THEN RAISE EXCEPTION USING ERRCODE = 'LCP01', MESSAGE = 'registry command not staged'; END IF;
    IF (SELECT pg_catalog.count(*) FROM ONLY control.project_registry_observations AS o
         WHERE o.observation_digest = p_accepted_observation_digest) <> 1
       OR (p_pending_observation_digest IS NOT NULL AND
           (SELECT pg_catalog.count(*) FROM ONLY control.project_registry_observations AS o
             WHERE o.observation_digest = p_pending_observation_digest) <> 1)
       OR (SELECT pg_catalog.count(*) FROM ONLY control.project_registry_observations AS o
            WHERE o.observation_digest = p_authority_observation_digest) <> 1
    THEN RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'registry project observation absent'; END IF;
    IF EXISTS (
        SELECT 1
          FROM ONLY control.project_registry_identity_reservations AS r
          JOIN ONLY control.project_registry_observations AS o
            ON o.observation_digest IN (p_accepted_observation_digest, p_pending_observation_digest)
         WHERE r.project_id <> p_project_id
           AND ((r.dimension = 'CANONICAL_ROOT' AND r.identity_digest = o.root_identity_digest)
             OR (r.dimension = 'REPOSITORY' AND r.identity_digest = o.repository_identity_digest)
             OR (r.dimension = 'FILE' AND r.identity_digest = o.file_identity_digest))
    ) THEN RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'registry identity owner changed'; END IF;

    SELECT pg_catalog.count(*) INTO v_deleted
      FROM ONLY control.project_registry_identity_reservations AS r WHERE r.project_id = p_project_id;
    DELETE FROM ONLY control.project_registry_identity_reservations WHERE project_id = p_project_id;
    UPDATE ONLY control.project_registry_projects SET
        project_class = p_project_class, accepted_observation_digest = p_accepted_observation_digest,
        pending_observation_digest = p_pending_observation_digest,
        drift_canonical_root = p_drift_canonical_root, drift_repository = p_drift_repository,
        drift_file = p_drift_file, drift_primary_ref_name = p_drift_primary_ref_name,
        drift_primary_ref_storage = p_drift_primary_ref_storage,
        authority_contract_version = p_authority_contract_version,
        authority_producer_id = p_authority_producer_id,
        authority_producer_version = p_authority_producer_version,
        authority_runtime = p_authority_runtime, authority_snapshot_id = p_authority_snapshot_id,
        authority_registry_revision = p_authority_registry_revision,
        authority_lifecycle = p_authority_lifecycle, authority_primary_ref = p_authority_primary_ref,
        authority_primary_ref_storage_digest = p_authority_primary_ref_storage_digest,
        authority_observation_digest = p_authority_observation_digest,
        authority_receipt_digest = p_authority_receipt_digest
      WHERE project_id = p_project_id;
    GET DIAGNOSTICS v_rows = ROW_COUNT;
    IF v_rows = 0 THEN
        INSERT INTO control.project_registry_projects VALUES (
            p_project_id, p_project_class, p_accepted_observation_digest,
            p_pending_observation_digest, p_drift_canonical_root, p_drift_repository,
            p_drift_file, p_drift_primary_ref_name, p_drift_primary_ref_storage,
            p_authority_contract_version, p_authority_producer_id,
            p_authority_producer_version, p_authority_runtime, p_authority_snapshot_id,
            p_authority_registry_revision, p_authority_lifecycle, p_authority_primary_ref,
            p_authority_primary_ref_storage_digest, p_authority_observation_digest,
            p_authority_receipt_digest
        );
    ELSIF v_rows <> 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'registry project multiplicity';
    END IF;

    INSERT INTO control.project_registry_identity_reservations
        (dimension, identity_digest, reservation_status, project_id)
    SELECT x.dimension, x.identity_digest, x.reservation_status, p_project_id
      FROM (
        SELECT 'CANONICAL_ROOT'::text AS dimension, o.root_identity_digest AS identity_digest,
               'ACCEPTED'::text AS reservation_status
          FROM ONLY control.project_registry_observations AS o WHERE o.observation_digest = p_accepted_observation_digest
        UNION ALL SELECT 'REPOSITORY', o.repository_identity_digest, 'ACCEPTED'
          FROM ONLY control.project_registry_observations AS o WHERE o.observation_digest = p_accepted_observation_digest
        UNION ALL SELECT 'FILE', o.file_identity_digest, 'ACCEPTED'
          FROM ONLY control.project_registry_observations AS o WHERE o.observation_digest = p_accepted_observation_digest
        UNION ALL SELECT 'CANONICAL_ROOT', o.root_identity_digest, 'PENDING'
          FROM ONLY control.project_registry_observations AS o WHERE o.observation_digest = p_pending_observation_digest
        UNION ALL SELECT 'REPOSITORY', o.repository_identity_digest, 'PENDING'
          FROM ONLY control.project_registry_observations AS o WHERE o.observation_digest = p_pending_observation_digest
        UNION ALL SELECT 'FILE', o.file_identity_digest, 'PENDING'
          FROM ONLY control.project_registry_observations AS o WHERE o.observation_digest = p_pending_observation_digest
      ) AS x;
    GET DIAGNOSTICS v_inserted = ROW_COUNT;
    UPDATE ONLY control.project_registry_state SET stage_project = true,
        stage_reservation_delete_count = v_deleted,
        stage_reservation_insert_count = v_inserted
      WHERE singleton = true AND stage_command_id IS NOT NULL AND NOT stage_project;
    GET DIAGNOSTICS v_rows = ROW_COUNT;
    IF v_rows <> 1 THEN RAISE EXCEPTION USING ERRCODE = 'LCP01', MESSAGE = 'registry project stage changed'; END IF;
    RETURN 'STAGED';
END;
$lattice_project_registry_stage_project_v1$;

CREATE FUNCTION control.project_registry_finalize_v1(
    p_global_schema_version smallint, p_global_manifest_sha256 text,
    p_command_id text, p_ordinal bigint,
    p_base_runtime text, p_base_ordinal bigint, p_base_observation_count bigint,
    p_base_project_count bigint, p_base_command_count bigint, p_base_reservation_count bigint,
    p_base_retained_bytes bigint, p_base_checkpoint_digest bytea,
    p_result_runtime text, p_result_ordinal bigint, p_result_observation_count bigint,
    p_result_project_count bigint, p_result_command_count bigint,
    p_result_reservation_count bigint, p_result_retained_bytes bigint,
    p_result_checkpoint_digest bytea, p_record_set_digest bytea,
    p_transaction_digest bytea, p_persistence_receipt_digest bytea,
    p_stage_observation boolean, p_stage_project boolean,
    p_reservation_delete_count bigint, p_reservation_insert_count bigint
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
AS $lattice_project_registry_finalize_v1$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
    v_state control.project_registry_state%ROWTYPE;
    v_command_current boolean;
    v_project_current_count bigint;
    v_observation_current_count bigint;
    v_reservation_current_count bigint;
    v_actual_observations bigint;
    v_actual_projects bigint;
    v_actual_commands bigint;
    v_actual_reservations bigint;
    v_rows bigint;
BEGIN

    SELECT pg_catalog.count(*),
           pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') ||
                   pg_catalog.decode('00', 'hex') ||
                   COALESCE(
                       pg_catalog.string_agg(
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_id, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_path, 'UTF8') ||
                           pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.migration_status, 'UTF8') ||
                           pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) ||
                               pg_catalog.convert_to(h.transaction_mode, 'UTF8') ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) ||
                           pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer),
                           pg_catalog.decode('', 'hex') ORDER BY h.ordinal
                       ),
                       pg_catalog.decode('', 'hex')
                   )
               ),
               'hex'
           )
      INTO v_manifest_entry_count, v_history_manifest_sha256
      FROM ONLY control.migration_history AS h;

    IF p_global_schema_version IS NULL
       OR p_global_schema_version <> 4
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 5
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 4 AND c.max_reader = 4
              AND c.min_writer = 4 AND c.max_writer = 4
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR p_result_runtime IS DISTINCT FROM 'LIVE' OR p_base_runtime IS DISTINCT FROM 'LIVE'
       OR p_ordinal <> p_base_ordinal + 1 OR p_result_ordinal <> p_ordinal
       OR p_result_command_count <> p_ordinal OR p_result_command_count > 65536
       OR p_result_project_count > 4096 OR p_result_retained_bytes > 67108864
    THEN RAISE EXCEPTION USING ERRCODE = 'LPR01', MESSAGE = 'invalid registry finalization'; END IF;
    SELECT * INTO v_state FROM ONLY control.project_registry_state WHERE singleton = true FOR UPDATE;
    IF NOT FOUND OR v_state.command_ordinal <> p_base_ordinal
       OR v_state.observation_count <> p_base_observation_count
       OR v_state.project_count <> p_base_project_count
       OR v_state.command_count <> p_base_command_count
       OR v_state.reservation_count <> p_base_reservation_count
       OR v_state.retained_bytes <> p_base_retained_bytes
       OR v_state.checkpoint_digest IS DISTINCT FROM p_base_checkpoint_digest
       OR v_state.stage_command_id IS DISTINCT FROM p_command_id
       OR v_state.stage_ordinal IS DISTINCT FROM p_ordinal
       OR v_state.stage_base_checkpoint_digest IS DISTINCT FROM p_base_checkpoint_digest
       OR v_state.stage_result_checkpoint_digest IS DISTINCT FROM p_result_checkpoint_digest
       OR v_state.stage_record_set_digest IS DISTINCT FROM p_record_set_digest
       OR v_state.stage_observation IS DISTINCT FROM p_stage_observation
       OR v_state.stage_project IS DISTINCT FROM p_stage_project
       OR v_state.stage_reservation_delete_count IS DISTINCT FROM p_reservation_delete_count
       OR v_state.stage_reservation_insert_count IS DISTINCT FROM p_reservation_insert_count
    THEN RAISE EXCEPTION USING ERRCODE = 'LCP01', MESSAGE = 'registry finalization shape changed'; END IF;

    SELECT c.xmin = pg_catalog.pg_current_xact_id()::xid
      INTO v_command_current FROM ONLY control.project_registry_commands AS c
     WHERE c.ordinal = p_ordinal AND c.command_id = p_command_id
       AND c.result_checkpoint_digest = p_result_checkpoint_digest
       AND c.record_set_digest = p_record_set_digest
       AND c.transaction_digest = p_transaction_digest
       AND c.persistence_receipt_digest = p_persistence_receipt_digest;
    IF v_command_current IS DISTINCT FROM true THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'registry command not current transaction';
    END IF;
    SELECT pg_catalog.count(*) INTO v_observation_current_count
      FROM ONLY control.project_registry_observations AS o
     WHERE o.xmin = pg_catalog.pg_current_xact_id()::xid;
    SELECT pg_catalog.count(*) INTO v_project_current_count
      FROM ONLY control.project_registry_projects AS p
     WHERE p.xmin = pg_catalog.pg_current_xact_id()::xid;
    SELECT pg_catalog.count(*) INTO v_reservation_current_count
      FROM ONLY control.project_registry_identity_reservations AS r
     WHERE r.xmin = pg_catalog.pg_current_xact_id()::xid;
    IF v_observation_current_count <> (CASE WHEN p_stage_observation THEN 1 ELSE 0 END)
       OR v_project_current_count <> (CASE WHEN p_stage_project THEN 1 ELSE 0 END)
       OR v_reservation_current_count <> p_reservation_insert_count
    THEN RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'registry stage rows not current transaction'; END IF;

    SELECT pg_catalog.count(*) INTO v_actual_observations FROM ONLY control.project_registry_observations;
    SELECT pg_catalog.count(*) INTO v_actual_projects FROM ONLY control.project_registry_projects;
    SELECT pg_catalog.count(*) INTO v_actual_commands FROM ONLY control.project_registry_commands;
    SELECT pg_catalog.count(*) INTO v_actual_reservations FROM ONLY control.project_registry_identity_reservations;
    IF v_actual_observations <> p_result_observation_count
       OR v_actual_projects <> p_result_project_count
       OR v_actual_commands <> p_result_command_count
       OR v_actual_reservations <> p_result_reservation_count
    THEN RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'registry row counts corrupt'; END IF;

    UPDATE ONLY control.project_registry_state SET
        command_ordinal = p_result_ordinal, observation_count = p_result_observation_count,
        project_count = p_result_project_count, command_count = p_result_command_count,
        reservation_count = p_result_reservation_count, retained_bytes = p_result_retained_bytes,
        checkpoint_digest = p_result_checkpoint_digest,
        stage_command_id = NULL, stage_ordinal = NULL, stage_base_checkpoint_digest = NULL,
        stage_result_checkpoint_digest = NULL, stage_record_set_digest = NULL,
        stage_observation = NULL, stage_project = NULL,
        stage_reservation_delete_count = NULL, stage_reservation_insert_count = NULL
      WHERE singleton = true;
    GET DIAGNOSTICS v_rows = ROW_COUNT;
    IF v_rows <> 1 THEN RAISE EXCEPTION USING ERRCODE = 'LCP01', MESSAGE = 'registry publication failed'; END IF;
    RETURN 'FINALIZED';
END;
$lattice_project_registry_finalize_v1$;

REVOKE ALL ON TABLE control.project_registry_state, control.project_registry_observations, control.project_registry_projects, control.project_registry_commands, control.project_registry_identity_reservations FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
REVOKE EXECUTE ON FUNCTION control.store_prepare_v3(smallint, text, text, text, text, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea) FROM lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.store_finalize_v3(smallint, text, text, text, text, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, uuid, bytea, smallint, text, bigint, bytea, bytea, bigint, bytea, bytea, text, bytea, bytea) FROM lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.store_current_head_v3(text, text, text, bytea) FROM lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.task_ledger_prepare_v1(bytea, text) FROM lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.task_ledger_read_head_v1(bytea, text, text) FROM lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.task_ledger_read_events_v1(bytea) FROM lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.task_ledger_read_commands_v1(bytea) FROM lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.task_ledger_finalize_v1(bytea, text, text, text, text, bytea, text, text, bytea, text, bytea, bytea, text, text, text, text, text, text, text, text, text, bytea, bytea, text, bytea, text, bytea, text, bytea, bytea, text, text, text, text, text, text, text, bytea, jsonb, boolean, text, text, text, text, text, text, text, bytea, text, bytea, bytea, text, bytea, text, bytea, bytea, text, text, bytea, bytea, bytea, text, boolean, text, bytea, text, bytea, boolean, bytea, bytea) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.store_prepare_v4(smallint, text, smallint, text, text, text, text, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.store_prepare_v4(smallint, text, smallint, text, text, text, text, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.store_finalize_v4(smallint, text, smallint, text, text, text, text, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, uuid, bytea, smallint, text, bigint, bytea, bytea, bigint, bytea, bytea, text, bytea, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.store_finalize_v4(smallint, text, smallint, text, text, text, text, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, uuid, bytea, smallint, text, bigint, bytea, bytea, bigint, bytea, bytea, text, bytea, bytea) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.store_current_head_v4(smallint, text, text, text, text, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.store_current_head_v4(smallint, text, text, text, text, bytea) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.task_ledger_prepare_v2(smallint, text, bytea, text) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.task_ledger_prepare_v2(smallint, text, bytea, text) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.task_ledger_read_head_v2(smallint, text, bytea, text, text) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.task_ledger_read_head_v2(smallint, text, bytea, text, text) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.task_ledger_read_events_v2(smallint, text, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.task_ledger_read_events_v2(smallint, text, bytea) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.task_ledger_read_commands_v2(smallint, text, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.task_ledger_read_commands_v2(smallint, text, bytea) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.task_ledger_finalize_v2(smallint, text, bytea, text, text, text, text, bytea, text, text, bytea, text, bytea, bytea, text, text, text, text, text, text, text, text, text, bytea, bytea, text, bytea, text, bytea, text, bytea, bytea, text, text, text, text, text, text, text, bytea, jsonb, boolean, text, text, text, text, text, text, text, bytea, text, bytea, bytea, text, bytea, text, bytea, bytea, text, text, bytea, bytea, bytea, text, boolean, text, bytea, text, bytea, boolean, bytea, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.task_ledger_finalize_v2(smallint, text, bytea, text, text, text, text, bytea, text, text, bytea, text, bytea, bytea, text, text, text, text, text, text, text, text, text, bytea, bytea, text, bytea, text, bytea, text, bytea, bytea, text, text, text, text, text, text, text, bytea, jsonb, boolean, text, text, text, text, text, text, text, bytea, text, bytea, bytea, text, bytea, text, bytea, bytea, text, text, bytea, bytea, bytea, text, boolean, text, bytea, text, bytea, boolean, bytea, bytea) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_prepare_v1(smallint, text, text, bytea, text, text, bigint, text, bigint, bytea, bytea, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_prepare_v1(smallint, text, text, bytea, text, text, bigint, text, bigint, bytea, bytea, bytea) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_read_state_v1(smallint, text) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_read_state_v1(smallint, text) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_read_observations_v1(smallint, text) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_read_observations_v1(smallint, text) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_read_projects_v1(smallint, text) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_read_projects_v1(smallint, text) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_read_commands_v1(smallint, text) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_read_commands_v1(smallint, text) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_read_reservations_v1(smallint, text) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_read_reservations_v1(smallint, text) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_stage_command_v1(smallint, text, bigint, text, text, text, text, bytea, boolean, text, text, text, text, text, numeric, text, text, text, bytea, bytea, bytea, text, bytea, bytea, text, text, text, text, text, text, text, bytea, bytea, bytea, boolean, boolean, boolean, boolean, boolean, bytea, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, bytea, bytea, boolean, text, bytea, bytea, bytea, text, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_stage_command_v1(smallint, text, bigint, text, text, text, text, bytea, boolean, text, text, text, text, text, numeric, text, text, text, bytea, bytea, bytea, text, bytea, bytea, text, text, text, text, text, text, text, bytea, bytea, bytea, boolean, boolean, boolean, boolean, boolean, bytea, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, bytea, bytea, boolean, text, bytea, bytea, bytea, text, bytea) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_stage_project_v1(smallint, text, text, text, bytea, bytea, boolean, boolean, boolean, boolean, boolean, smallint, text, text, text, text, numeric, text, text, bytea, bytea, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_stage_project_v1(smallint, text, text, text, bytea, bytea, boolean, boolean, boolean, boolean, boolean, smallint, text, text, text, text, numeric, text, text, bytea, bytea, bytea) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_finalize_v1(smallint, text, text, bigint, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, bytea, bytea, bytea, boolean, boolean, bigint, bigint) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_finalize_v1(smallint, text, text, bigint, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, bytea, bytea, bytea, boolean, boolean, bigint, bigint) TO lattice_runtime;

COMMENT ON SCHEMA control IS 'LATTICE_DEVOS_CONTROL_SCHEMA_V4';
COMMENT ON SCHEMA memory IS 'LATTICE_DEVOS_MEMORY_SCHEMA_V4';
COMMENT ON SCHEMA readmodel IS 'LATTICE_DEVOS_READMODEL_SCHEMA_V4';
