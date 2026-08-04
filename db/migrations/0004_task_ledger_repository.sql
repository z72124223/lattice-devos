-- LATTICE DevOS Postgres Store 1.3 durable Task Ledger repository foundation.
-- Transaction ownership belongs exclusively to the Rust adapter or migration runner.
-- PostgreSQL persists fixed columns; Task Ledger 2.1 remains the sole semantic owner.

CREATE TABLE control.task_ledger_streams (
    stream_id bytea PRIMARY KEY,
    ledger_schema_version varchar(16) NOT NULL,
    head_contract_version smallint NOT NULL,
    producer_id varchar(64) NOT NULL,
    producer_version varchar(32) NOT NULL,
    runtime varchar(16) NOT NULL,
    project_id varchar(64) NOT NULL,
    project_snapshot_id varchar(128) NOT NULL,
    task_id varchar(69) NOT NULL,
    task_revision numeric(20,0) NOT NULL,
    task_spec_digest bytea NOT NULL,
    accounting_currency char(3) NOT NULL,
    sequence numeric(20,0) NOT NULL,
    last_event_digest bytea NOT NULL,
    resource_revision numeric(20,0) NOT NULL,
    resource_projection_digest bytea NOT NULL,
    head_digest bytea NOT NULL,
    active_agents numeric(20,0) NOT NULL,
    active_implementers numeric(20,0) NOT NULL,
    elapsed_seconds numeric(20,0) NOT NULL,
    attempt_number numeric(20,0) NOT NULL,
    used_model_calls numeric(20,0) NOT NULL,
    used_external_cost varchar(256) NOT NULL,
    event_count numeric(20,0) NOT NULL,
    command_count numeric(20,0) NOT NULL,
    outbox_count numeric(20,0) NOT NULL,
    checkpoint_schema_version varchar(16) NOT NULL,
    checkpoint_digest bytea NOT NULL,
    CONSTRAINT task_ledger_streams_stream_digest CHECK (
        pg_catalog.octet_length(stream_id) = 32
        AND stream_id <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    ),
    CONSTRAINT task_ledger_streams_versions_exact CHECK (
        ledger_schema_version = '2.0'
        AND head_contract_version = 1
        AND producer_id = 'lattice-task-ledger'
        AND producer_version = '2.0'
        AND runtime = 'LIVE'
        AND checkpoint_schema_version = '1.0'
    ),
    CONSTRAINT task_ledger_streams_identity_shape CHECK (
        project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'
        AND project_snapshot_id ~ '^[a-z0-9._:-]{1,128}$'
        AND task_id ~ '^TASK-[A-Z0-9][A-Z0-9_-]{2,63}$'
        AND accounting_currency ~ '^[A-Z]{3}$'
    ),
    CONSTRAINT task_ledger_streams_task_revision_u64 CHECK (
        task_revision >= 1
        AND task_revision <= 18446744073709551615
    ),
    CONSTRAINT task_ledger_streams_position_u64 CHECK (
        sequence >= 0 AND sequence <= 18446744073709551615
        AND resource_revision >= 0 AND resource_revision <= 18446744073709551615
        AND event_count >= 0 AND event_count <= 18446744073709551615
        AND command_count >= 0 AND command_count <= 18446744073709551615
        AND outbox_count >= 0 AND outbox_count <= 18446744073709551615
    ),
    CONSTRAINT task_ledger_streams_counters_u64 CHECK (
        active_agents >= 0 AND active_agents <= 18446744073709551615
        AND active_implementers >= 0 AND active_implementers <= 18446744073709551615
        AND elapsed_seconds >= 0 AND elapsed_seconds <= 18446744073709551615
        AND attempt_number >= 0 AND attempt_number <= 18446744073709551615
        AND used_model_calls >= 0 AND used_model_calls <= 18446744073709551615
        AND active_implementers <= active_agents
    ),
    CONSTRAINT task_ledger_streams_cost_canonical CHECK (
        pg_catalog.octet_length(used_external_cost) BETWEEN 1 AND 256
        AND used_external_cost ~ '^(0|[1-9][0-9]{0,126})(\.[0-9]{0,127}[1-9])?$'
    ),
    CONSTRAINT task_ledger_streams_projection_shape CHECK (
        event_count = sequence
        AND command_count >= event_count
        AND outbox_count <= event_count
        AND resource_revision <= sequence
        AND (
            (sequence = 0
             AND last_event_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex'))
            OR
            (sequence > 0
             AND last_event_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex'))
        )
        AND (
            (resource_revision = 0
             AND resource_projection_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex'))
            OR
            (resource_revision > 0
             AND resource_projection_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex'))
        )
    ),
    CONSTRAINT task_ledger_streams_digest_shapes CHECK (
        pg_catalog.octet_length(task_spec_digest) = 32
        AND task_spec_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(last_event_digest) = 32
        AND pg_catalog.octet_length(resource_projection_digest) = 32
        AND pg_catalog.octet_length(head_digest) = 32
        AND head_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(checkpoint_digest) = 32
        AND checkpoint_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    )
);

CREATE TABLE control.task_ledger_commands (
    stream_id bytea NOT NULL,
    command_id varchar(128) NOT NULL,
    request_schema_version varchar(16) NOT NULL,
    request_digest bytea NOT NULL,
    expected_sequence numeric(20,0) NOT NULL,
    expected_last_event_digest bytea NOT NULL,
    expected_resource_revision numeric(20,0) NOT NULL,
    expected_resource_projection_digest bytea NOT NULL,
    expected_head_digest bytea NOT NULL,
    correlation_id varchar(128) NOT NULL,
    occurred_at varchar(40) NOT NULL,
    event_kind varchar(32) NOT NULL,
    actor_id varchar(128) NOT NULL,
    action_id varchar(128) NOT NULL,
    audit_outcome varchar(16) NOT NULL,
    reason_code varchar(128) NOT NULL,
    subject_digest bytea NOT NULL,
    diagnostic jsonb NOT NULL,
    has_resource_snapshot boolean NOT NULL,
    resource_active_agents numeric(20,0) NOT NULL,
    resource_active_implementers numeric(20,0) NOT NULL,
    resource_elapsed_seconds numeric(20,0) NOT NULL,
    resource_attempt_number numeric(20,0) NOT NULL,
    resource_used_model_calls numeric(20,0) NOT NULL,
    resource_used_external_cost varchar(256) NOT NULL,
    receipt_schema_version varchar(16) NOT NULL,
    before_sequence numeric(20,0) NOT NULL,
    before_last_event_digest bytea NOT NULL,
    before_resource_revision numeric(20,0) NOT NULL,
    before_resource_projection_digest bytea NOT NULL,
    before_head_digest bytea NOT NULL,
    after_sequence numeric(20,0) NOT NULL,
    after_last_event_digest bytea NOT NULL,
    after_resource_revision numeric(20,0) NOT NULL,
    after_resource_projection_digest bytea NOT NULL,
    after_head_digest bytea NOT NULL,
    command_outcome varchar(16) NOT NULL,
    denial_reason varchar(32) NOT NULL,
    event_digest bytea NOT NULL,
    receipt_digest bytea NOT NULL,
    base_checkpoint_digest bytea NOT NULL,
    result_checkpoint_digest bytea NOT NULL,
    record_set_digest bytea NOT NULL,
    store_transaction_id varchar(128) NOT NULL,
    PRIMARY KEY (stream_id, command_id),
    CONSTRAINT task_ledger_commands_stream_fk FOREIGN KEY (stream_id)
        REFERENCES control.task_ledger_streams (stream_id),
    CONSTRAINT task_ledger_commands_store_terminal_fk FOREIGN KEY (store_transaction_id)
        REFERENCES control.terminal_transactions (transaction_id),
    CONSTRAINT task_ledger_commands_versions_exact CHECK (
        request_schema_version = '2.0' AND receipt_schema_version = '2.0'
    ),
    CONSTRAINT task_ledger_commands_identifiers CHECK (
        command_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND correlation_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND actor_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND action_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND reason_code ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND store_transaction_id ~ '^task-ledger-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT task_ledger_commands_timestamp_shape CHECK (
        occurred_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
    ),
    CONSTRAINT task_ledger_commands_closed_values CHECK (
        event_kind IN (
            'TASK_CREATED', 'STATE_TRANSITION', 'POLICY_DECISION',
            'RESOURCE_SNAPSHOT', 'EFFECT_INTENT', 'EFFECT_OUTCOME',
            'EVIDENCE_RECORDED'
        )
        AND audit_outcome IN (
            'RECORDED', 'ALLOWED', 'DENIED', 'PASSED', 'FAILED',
            'BLOCKED', 'CANCELLED'
        )
        AND command_outcome IN ('APPENDED', 'DENIED')
        AND (
            (command_outcome = 'APPENDED'
             AND denial_reason = ''
             AND event_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex'))
            OR
            (command_outcome = 'DENIED'
             AND denial_reason IN ('STALE_HEAD', 'SEQUENCE_OVERFLOW')
             AND event_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex'))
        )
    ),
    CONSTRAINT task_ledger_commands_u64_values CHECK (
        expected_sequence >= 0 AND expected_sequence <= 18446744073709551615
        AND expected_resource_revision >= 0 AND expected_resource_revision <= 18446744073709551615
        AND before_sequence >= 0 AND before_sequence <= 18446744073709551615
        AND before_resource_revision >= 0 AND before_resource_revision <= 18446744073709551615
        AND after_sequence >= 0 AND after_sequence <= 18446744073709551615
        AND after_resource_revision >= 0 AND after_resource_revision <= 18446744073709551615
        AND resource_active_agents >= 0 AND resource_active_agents <= 18446744073709551615
        AND resource_active_implementers >= 0 AND resource_active_implementers <= 18446744073709551615
        AND resource_elapsed_seconds >= 0 AND resource_elapsed_seconds <= 18446744073709551615
        AND resource_attempt_number >= 0 AND resource_attempt_number <= 18446744073709551615
        AND resource_used_model_calls >= 0 AND resource_used_model_calls <= 18446744073709551615
    ),
    CONSTRAINT task_ledger_commands_resource_shape CHECK (
        resource_active_implementers <= resource_active_agents
        AND resource_used_external_cost ~ '^(0|[1-9][0-9]{0,126})(\.[0-9]{0,127}[1-9])?$'
        AND (
            (has_resource_snapshot
             AND event_kind = 'RESOURCE_SNAPSHOT')
            OR
            (NOT has_resource_snapshot
             AND event_kind <> 'RESOURCE_SNAPSHOT'
             AND resource_active_agents = 0
             AND resource_active_implementers = 0
             AND resource_elapsed_seconds = 0
             AND resource_attempt_number = 0
             AND resource_used_model_calls = 0
             AND resource_used_external_cost = '0')
        )
    ),
    CONSTRAINT task_ledger_commands_diagnostic_bounded CHECK (
        pg_catalog.octet_length(diagnostic::text) <= 65536
        AND NOT pg_catalog.jsonb_path_exists(
            diagnostic,
            'strict $.** ? (@.type() == "number")'
        )
    ),
    CONSTRAINT task_ledger_commands_digest_shapes CHECK (
        pg_catalog.octet_length(stream_id) = 32
        AND stream_id <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(request_digest) = 32
        AND request_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(expected_last_event_digest) = 32
        AND pg_catalog.octet_length(expected_resource_projection_digest) = 32
        AND pg_catalog.octet_length(expected_head_digest) = 32
        AND expected_head_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(subject_digest) = 32
        AND subject_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(before_last_event_digest) = 32
        AND pg_catalog.octet_length(before_resource_projection_digest) = 32
        AND pg_catalog.octet_length(before_head_digest) = 32
        AND before_head_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(after_last_event_digest) = 32
        AND pg_catalog.octet_length(after_resource_projection_digest) = 32
        AND pg_catalog.octet_length(after_head_digest) = 32
        AND after_head_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(event_digest) = 32
        AND pg_catalog.octet_length(receipt_digest) = 32
        AND receipt_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(base_checkpoint_digest) = 32
        AND base_checkpoint_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(result_checkpoint_digest) = 32
        AND result_checkpoint_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(record_set_digest) = 32
        AND record_set_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    )
);

CREATE TABLE control.task_ledger_events (
    stream_id bytea NOT NULL,
    sequence numeric(20,0) NOT NULL,
    event_schema_version varchar(16) NOT NULL,
    previous_event_digest bytea NOT NULL,
    command_id varchar(128) NOT NULL,
    request_digest bytea NOT NULL,
    correlation_id varchar(128) NOT NULL,
    occurred_at varchar(40) NOT NULL,
    event_kind varchar(32) NOT NULL,
    actor_id varchar(128) NOT NULL,
    action_id varchar(128) NOT NULL,
    audit_outcome varchar(16) NOT NULL,
    reason_code varchar(128) NOT NULL,
    subject_digest bytea NOT NULL,
    diagnostic jsonb NOT NULL,
    has_resource_snapshot boolean NOT NULL,
    resource_active_agents numeric(20,0) NOT NULL,
    resource_active_implementers numeric(20,0) NOT NULL,
    resource_elapsed_seconds numeric(20,0) NOT NULL,
    resource_attempt_number numeric(20,0) NOT NULL,
    resource_used_model_calls numeric(20,0) NOT NULL,
    resource_used_external_cost varchar(256) NOT NULL,
    resource_revision numeric(20,0) NOT NULL,
    resource_projection_digest bytea NOT NULL,
    event_digest bytea NOT NULL,
    PRIMARY KEY (stream_id, sequence),
    UNIQUE (stream_id, command_id),
    UNIQUE (event_digest),
    CONSTRAINT task_ledger_events_command_fk FOREIGN KEY (stream_id, command_id)
        REFERENCES control.task_ledger_commands (stream_id, command_id),
    CONSTRAINT task_ledger_events_version_exact CHECK (event_schema_version = '2.0'),
    CONSTRAINT task_ledger_events_sequence_u64 CHECK (
        sequence >= 1 AND sequence <= 18446744073709551615
        AND resource_revision >= 0 AND resource_revision <= 18446744073709551615
        AND resource_revision <= sequence
    ),
    CONSTRAINT task_ledger_events_identifiers CHECK (
        command_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND correlation_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND actor_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND action_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND reason_code ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
    ),
    CONSTRAINT task_ledger_events_timestamp_shape CHECK (
        occurred_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
    ),
    CONSTRAINT task_ledger_events_closed_values CHECK (
        event_kind IN (
            'TASK_CREATED', 'STATE_TRANSITION', 'POLICY_DECISION',
            'RESOURCE_SNAPSHOT', 'EFFECT_INTENT', 'EFFECT_OUTCOME',
            'EVIDENCE_RECORDED'
        )
        AND audit_outcome IN (
            'RECORDED', 'ALLOWED', 'DENIED', 'PASSED', 'FAILED',
            'BLOCKED', 'CANCELLED'
        )
    ),
    CONSTRAINT task_ledger_events_resource_u64 CHECK (
        resource_active_agents >= 0 AND resource_active_agents <= 18446744073709551615
        AND resource_active_implementers >= 0 AND resource_active_implementers <= 18446744073709551615
        AND resource_elapsed_seconds >= 0 AND resource_elapsed_seconds <= 18446744073709551615
        AND resource_attempt_number >= 0 AND resource_attempt_number <= 18446744073709551615
        AND resource_used_model_calls >= 0 AND resource_used_model_calls <= 18446744073709551615
        AND resource_active_implementers <= resource_active_agents
    ),
    CONSTRAINT task_ledger_events_resource_shape CHECK (
        resource_used_external_cost ~ '^(0|[1-9][0-9]{0,126})(\.[0-9]{0,127}[1-9])?$'
        AND (
            (has_resource_snapshot
             AND event_kind = 'RESOURCE_SNAPSHOT')
            OR
            (NOT has_resource_snapshot
             AND event_kind <> 'RESOURCE_SNAPSHOT'
             AND resource_active_agents = 0
             AND resource_active_implementers = 0
             AND resource_elapsed_seconds = 0
             AND resource_attempt_number = 0
             AND resource_used_model_calls = 0
             AND resource_used_external_cost = '0')
        )
    ),
    CONSTRAINT task_ledger_events_projection_shape CHECK (
        (resource_revision = 0
         AND resource_projection_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex'))
        OR
        (resource_revision > 0
         AND resource_projection_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex'))
    ),
    CONSTRAINT task_ledger_events_diagnostic_bounded CHECK (
        pg_catalog.octet_length(diagnostic::text) <= 65536
        AND NOT pg_catalog.jsonb_path_exists(
            diagnostic,
            'strict $.** ? (@.type() == "number")'
        )
    ),
    CONSTRAINT task_ledger_events_digest_shapes CHECK (
        pg_catalog.octet_length(stream_id) = 32
        AND stream_id <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(previous_event_digest) = 32
        AND pg_catalog.octet_length(request_digest) = 32
        AND request_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(subject_digest) = 32
        AND subject_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(resource_projection_digest) = 32
        AND pg_catalog.octet_length(event_digest) = 32
        AND event_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    )
);

CREATE TABLE control.task_ledger_outbox (
    admission_digest bytea PRIMARY KEY,
    admission_schema_version varchar(16) NOT NULL,
    admission_state varchar(16) NOT NULL,
    stream_id bytea NOT NULL,
    event_sequence numeric(20,0) NOT NULL,
    event_digest bytea NOT NULL,
    command_id varchar(128) NOT NULL,
    request_digest bytea NOT NULL,
    intent_digest bytea NOT NULL,
    occurred_at varchar(40) NOT NULL,
    UNIQUE (stream_id, event_sequence),
    UNIQUE (stream_id, command_id),
    UNIQUE (event_digest),
    UNIQUE (intent_digest),
    CONSTRAINT task_ledger_outbox_event_fk FOREIGN KEY (stream_id, event_sequence)
        REFERENCES control.task_ledger_events (stream_id, sequence),
    CONSTRAINT task_ledger_outbox_command_fk FOREIGN KEY (stream_id, command_id)
        REFERENCES control.task_ledger_commands (stream_id, command_id),
    CONSTRAINT task_ledger_outbox_version_state_exact CHECK (
        admission_schema_version = '1.0' AND admission_state = 'ADMITTED'
    ),
    CONSTRAINT task_ledger_outbox_sequence_u64 CHECK (
        event_sequence >= 1 AND event_sequence <= 18446744073709551615
    ),
    CONSTRAINT task_ledger_outbox_identifier CHECK (
        command_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
    ),
    CONSTRAINT task_ledger_outbox_timestamp_shape CHECK (
        occurred_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
    ),
    CONSTRAINT task_ledger_outbox_digest_shapes CHECK (
        pg_catalog.octet_length(admission_digest) = 32
        AND admission_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(stream_id) = 32
        AND stream_id <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(event_digest) = 32
        AND event_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(request_digest) = 32
        AND request_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(intent_digest) = 32
        AND intent_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    )
);
CREATE FUNCTION control.store_prepare_v3(
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
AS $lattice_store_prepare_v3$
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
       OR v_schema_version IS DISTINCT FROM 3
       OR v_min_reader IS DISTINCT FROM 3
       OR v_max_reader IS DISTINCT FROM 3
       OR v_min_writer IS DISTINCT FROM 3
       OR v_max_writer IS DISTINCT FROM 3
       OR v_manifest_sha256 IS NULL
       OR v_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR v_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 4
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
$lattice_store_prepare_v3$;

CREATE FUNCTION control.store_finalize_v3(
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
AS $lattice_store_finalize_v3$
DECLARE
    v_prepare record;
    v_rows bigint;
BEGIN
    SELECT *
      INTO v_prepare
      FROM control.store_prepare_v3(
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
$lattice_store_finalize_v3$;

CREATE FUNCTION control.store_current_head_v3(
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
AS $lattice_store_current_head_v3$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
BEGIN
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
       AND c.current_schema_version = 3
       AND c.min_reader = 3
       AND c.max_reader = 3
       AND c.min_writer = 3
       AND c.max_writer = 3
       AND pg_catalog.btrim(c.manifest_sha256::text) ~ '^[0-9a-f]{64}$'
       AND pg_catalog.btrim(c.manifest_sha256::text) <> pg_catalog.repeat('0', 64)
       AND v_manifest_entry_count = 4
       AND pg_catalog.btrim(c.manifest_sha256::text) = v_history_manifest_sha256;
END;
$lattice_store_current_head_v3$;

CREATE FUNCTION control.task_ledger_prepare_v1(
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
AS $lattice_task_ledger_prepare_v1$
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

    IF v_manifest_entry_count IS DISTINCT FROM 4 OR (
        SELECT pg_catalog.count(*)
          FROM ONLY control.schema_compatibility AS c
         WHERE c.singleton = true
           AND c.current_schema_version = 3
           AND c.min_reader = 3 AND c.max_reader = 3
           AND c.min_writer = 3 AND c.max_writer = 3
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
$lattice_task_ledger_prepare_v1$;

CREATE FUNCTION control.task_ledger_read_head_v1(
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
AS $lattice_task_ledger_read_head_v1$
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
       OR v_global_schema_version IS DISTINCT FROM 3
       OR v_min_reader IS DISTINCT FROM 3
       OR v_max_reader IS DISTINCT FROM 3
       OR v_min_writer IS DISTINCT FROM 3
       OR v_max_writer IS DISTINCT FROM 3
       OR v_global_manifest_sha256 IS NULL
       OR v_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR v_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 4
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
$lattice_task_ledger_read_head_v1$;

CREATE FUNCTION control.task_ledger_read_events_v1(
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
AS $lattice_task_ledger_read_events_v1$
BEGIN
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
$lattice_task_ledger_read_events_v1$;

CREATE FUNCTION control.task_ledger_read_commands_v1(
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
AS $lattice_task_ledger_read_commands_v1$
BEGIN
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
$lattice_task_ledger_read_commands_v1$;

CREATE FUNCTION control.task_ledger_finalize_v1(
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
AS $lattice_task_ledger_finalize_v1$
DECLARE
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
$lattice_task_ledger_finalize_v1$;

REVOKE ALL ON TABLE
    control.task_ledger_streams,
    control.task_ledger_commands,
    control.task_ledger_events,
    control.task_ledger_outbox
FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;

REVOKE EXECUTE ON FUNCTION control.store_prepare_v2(
    smallint, text, text, text, text, bytea, bytea, text, text, bigint,
    text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea,
    bytea, bytea, bytea, bytea, bytea, bytea
) FROM lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.store_finalize_v2(
    smallint, text, text, text, text, bytea, bytea, text, text, bigint,
    text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea,
    bytea, bytea, bytea, bytea, bytea, bytea, uuid, bytea, smallint, text,
    bigint, bytea, bytea, bigint, bytea, bytea, text, bytea, bytea
) FROM lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.store_current_head_v2(
    text, text, text, bytea
) FROM lattice_runtime;

REVOKE ALL ON FUNCTION control.store_prepare_v3(
    smallint, text, text, text, text, bytea, bytea, text, text, bigint,
    text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea,
    bytea, bytea, bytea, bytea, bytea, bytea
) FROM PUBLIC, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;
REVOKE ALL ON FUNCTION control.store_finalize_v3(
    smallint, text, text, text, text, bytea, bytea, text, text, bigint,
    text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea,
    bytea, bytea, bytea, bytea, bytea, bytea, uuid, bytea, smallint, text,
    bigint, bytea, bytea, bigint, bytea, bytea, text, bytea, bytea
) FROM PUBLIC, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;
REVOKE ALL ON FUNCTION control.store_current_head_v3(
    text, text, text, bytea
) FROM PUBLIC, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;
REVOKE ALL ON FUNCTION control.task_ledger_prepare_v1(
    bytea, text
) FROM PUBLIC, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;
REVOKE ALL ON FUNCTION control.task_ledger_read_head_v1(
    bytea, text, text
) FROM PUBLIC, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;
REVOKE ALL ON FUNCTION control.task_ledger_read_events_v1(
    bytea
) FROM PUBLIC, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;
REVOKE ALL ON FUNCTION control.task_ledger_read_commands_v1(
    bytea
) FROM PUBLIC, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;
REVOKE ALL ON FUNCTION control.task_ledger_finalize_v1(
    bytea, text, text, text, text, bytea, text, text, bytea, text,
    bytea, bytea, text, text, text, text, text, text, text, text,
    text, bytea, bytea, text, bytea, text, bytea, text, bytea, bytea,
    text, text, text, text, text, text, text, bytea, jsonb, boolean,
    text, text, text, text, text, text, text, bytea, text, bytea,
    bytea, text, bytea, text, bytea, bytea, text, text, bytea, bytea,
    bytea, text, boolean, text, bytea, text, bytea, boolean, bytea, bytea
) FROM PUBLIC, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;

GRANT EXECUTE ON FUNCTION control.store_prepare_v3(
    smallint, text, text, text, text, bytea, bytea, text, text, bigint,
    text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea,
    bytea, bytea, bytea, bytea, bytea, bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.store_finalize_v3(
    smallint, text, text, text, text, bytea, bytea, text, text, bigint,
    text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea,
    bytea, bytea, bytea, bytea, bytea, bytea, uuid, bytea, smallint, text,
    bigint, bytea, bytea, bigint, bytea, bytea, text, bytea, bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.store_current_head_v3(
    text, text, text, bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.task_ledger_prepare_v1(
    bytea, text
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.task_ledger_read_head_v1(
    bytea, text, text
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.task_ledger_read_events_v1(
    bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.task_ledger_read_commands_v1(
    bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.task_ledger_finalize_v1(
    bytea, text, text, text, text, bytea, text, text, bytea, text,
    bytea, bytea, text, text, text, text, text, text, text, text,
    text, bytea, bytea, text, bytea, text, bytea, text, bytea, bytea,
    text, text, text, text, text, text, text, bytea, jsonb, boolean,
    text, text, text, text, text, text, text, bytea, text, bytea,
    bytea, text, bytea, text, bytea, bytea, text, text, bytea, bytea,
    bytea, text, boolean, text, bytea, text, bytea, boolean, bytea, bytea
) TO lattice_runtime;

COMMENT ON SCHEMA control IS 'LATTICE_DEVOS_CONTROL_SCHEMA_V3';
COMMENT ON SCHEMA memory IS 'LATTICE_DEVOS_MEMORY_SCHEMA_V3';
COMMENT ON SCHEMA readmodel IS 'LATTICE_DEVOS_READMODEL_SCHEMA_V3';
