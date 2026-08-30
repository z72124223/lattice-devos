-- LATTICE managed foreman execution extension v1.
-- The explicit Rust runner owns the transaction and supplies identity rows.
-- This profile is subordinate to Task Ledger and contains no workflow phase.

CREATE SCHEMA foreman_execution AUTHORIZATION lattice_migrator;
COMMENT ON SCHEMA foreman_execution IS
    'LATTICE_FOREMAN_EXECUTION_EXTENSION_V1_STORE_V7';

REVOKE ALL ON SCHEMA foreman_execution FROM PUBLIC;

CREATE TABLE foreman_execution.extension_identity (
    singleton boolean PRIMARY KEY DEFAULT true,
    extension_id varchar(64) NOT NULL,
    extension_schema_version smallint NOT NULL,
    extension_path varchar(256) NOT NULL,
    extension_sql_bytes bigint NOT NULL,
    extension_sql_sha256 char(64) NOT NULL,
    extension_manifest_sha256 char(64) NOT NULL,
    database_name varchar(63) NOT NULL,
    database_uuid uuid NOT NULL,
    database_identity_sha256 char(64) NOT NULL,
    global_schema_version smallint NOT NULL,
    global_manifest_sha256 char(64) NOT NULL,
    installed_at timestamp with time zone NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT extension_identity_singleton CHECK (singleton),
    CONSTRAINT extension_identity_exact_profile CHECK (
        extension_id = 'lattice-postgres-foreman'
        AND extension_schema_version = 1
        AND extension_path = 'db/extensions/foreman-execution/v1.sql'
        AND extension_sql_bytes > 0
        AND database_name ~ '^[a-z][a-z0-9_]{2,62}$'
        AND database_name <> 'postgres'
        AND database_uuid::text ~ '^[0-9a-f]{8}-[0-9a-f]{4}-8[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$'
        AND global_schema_version = 7
        AND global_manifest_sha256 = '584a446464ab2f7ebd8b85543ba36a6d52b0a708502c39d2653b8814d84313f8'
    ),
    CONSTRAINT extension_identity_digest_shapes CHECK (
        extension_sql_sha256 ~ '^[0-9a-f]{64}$'
        AND extension_sql_sha256 <> repeat('0', 64)
        AND extension_manifest_sha256 ~ '^[0-9a-f]{64}$'
        AND extension_manifest_sha256 <> repeat('0', 64)
        AND database_identity_sha256 ~ '^[0-9a-f]{64}$'
        AND database_identity_sha256 <> repeat('0', 64)
    )
);

CREATE TABLE foreman_execution.extension_ledger (
    ledger_ordinal smallint PRIMARY KEY,
    extension_id varchar(64) NOT NULL,
    extension_schema_version smallint NOT NULL,
    extension_sql_sha256 char(64) NOT NULL,
    extension_manifest_sha256 char(64) NOT NULL,
    database_uuid uuid NOT NULL,
    database_identity_sha256 char(64) NOT NULL,
    global_schema_version smallint NOT NULL,
    global_manifest_sha256 char(64) NOT NULL,
    event_kind varchar(16) NOT NULL,
    recorded_at timestamp with time zone NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT extension_ledger_exact_install CHECK (
        ledger_ordinal = 1
        AND extension_id = 'lattice-postgres-foreman'
        AND extension_schema_version = 1
        AND event_kind = 'INSTALLED'
        AND global_schema_version = 7
        AND global_manifest_sha256 = '584a446464ab2f7ebd8b85543ba36a6d52b0a708502c39d2653b8814d84313f8'
    ),
    CONSTRAINT extension_ledger_digest_shapes CHECK (
        extension_sql_sha256 ~ '^[0-9a-f]{64}$'
        AND extension_sql_sha256 <> repeat('0', 64)
        AND extension_manifest_sha256 ~ '^[0-9a-f]{64}$'
        AND extension_manifest_sha256 <> repeat('0', 64)
        AND database_identity_sha256 ~ '^[0-9a-f]{64}$'
        AND database_identity_sha256 <> repeat('0', 64)
    )
);

CREATE TABLE foreman_execution.child_events (
    ledger_event_digest bytea PRIMARY KEY,
    ledger_stream_id bytea NOT NULL,
    ledger_event_sequence numeric(20,0) NOT NULL,
    ledger_command_id varchar(128) NOT NULL,
    ledger_request_digest bytea NOT NULL,
    ledger_payload_digest bytea NOT NULL,
    action_id varchar(128) NOT NULL,
    record_kind varchar(32) NOT NULL,
    task_ref bytea NOT NULL,
    recorded_at timestamp with time zone NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (ledger_stream_id, ledger_event_sequence),
    UNIQUE (ledger_stream_id, ledger_command_id),
    CONSTRAINT child_events_task_event_fk FOREIGN KEY (
        ledger_stream_id, ledger_event_sequence
    ) REFERENCES control.task_ledger_events (stream_id, sequence),
    CONSTRAINT child_events_event_digest_fk FOREIGN KEY (ledger_event_digest)
        REFERENCES control.task_ledger_events (event_digest),
    CONSTRAINT child_events_closed_values CHECK (
        record_kind IN (
            'TASK_PROMOTION', 'WORKER_ATTEMPT', 'WORKER_OBSERVATION',
            'VERIFICATION', 'ARTIFACT_REFERENCE', 'APPROVAL_EVIDENCE'
        )
        AND action_id IN (
            'RECORD_TASK_EXECUTION_BINDING_V1',
            'DISPATCH_WORKER_ATTEMPT_V1',
            'RECORD_WORKER_OBSERVATION_V1',
            'RECORD_TASK_VERIFICATION_V1',
            'RECORD_ARTIFACT_REFERENCE_V1',
            'RECORD_APPROVAL_EVIDENCE_V1'
        )
        AND ledger_event_sequence BETWEEN 1 AND 18446744073709551615
        AND ledger_command_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
    ),
    CONSTRAINT child_events_digest_shapes CHECK (
        octet_length(ledger_event_digest) = 32
        AND ledger_event_digest <> decode(repeat('00', 32), 'hex')
        AND octet_length(ledger_stream_id) = 32
        AND ledger_stream_id <> decode(repeat('00', 32), 'hex')
        AND octet_length(ledger_request_digest) = 32
        AND ledger_request_digest <> decode(repeat('00', 32), 'hex')
        AND octet_length(ledger_payload_digest) = 32
        AND ledger_payload_digest <> decode(repeat('00', 32), 'hex')
        AND octet_length(task_ref) = 32
        AND task_ref <> decode(repeat('00', 32), 'hex')
    )
);

-- One bounded latest dependency observation for preparation. This row is a
-- rebuttable owner observation (BLOCKED or CLEARED), not a Task state,
-- execution authority, queue, or workflow transition.
CREATE TABLE foreman_execution.preparation_observations (
    task_ref bytea PRIMARY KEY,
    project_id varchar(64) NOT NULL,
    project_snapshot_id varchar(159) NOT NULL,
    intake_stream_id bytea NOT NULL UNIQUE,
    intake_event_digest bytea NOT NULL UNIQUE,
    project_authority_receipt_digest bytea NOT NULL,
    observation_kind varchar(48) NOT NULL,
    subject_digest bytea NOT NULL,
    observed_at varchar(40) NOT NULL,
    observation_digest bytea NOT NULL,
    observation_generation bigint NOT NULL DEFAULT 1,
    updated_at timestamp with time zone NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT preparation_observations_intake_stream_fk FOREIGN KEY (intake_stream_id)
        REFERENCES control.task_submission_envelopes (stream_id),
    CONSTRAINT preparation_observations_intake_event_fk FOREIGN KEY (intake_event_digest)
        REFERENCES control.task_ledger_events (event_digest),
    CONSTRAINT preparation_observations_shape CHECK (
        project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'
        AND project_snapshot_id ~ '^[a-z0-9._:-]{1,159}$'
        AND observation_kind IN (
            'WORKTREE_NOT_CLEAN', 'PROJECT_REGISTRY_CURRENTNESS_CONFLICT', 'CLEARED'
        )
        AND observed_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
        AND observation_generation BETWEEN 1 AND 9223372036854775807
    ),
    CONSTRAINT preparation_observations_digest_shapes CHECK (
        octet_length(task_ref) = 32
        AND octet_length(intake_stream_id) = 32
        AND octet_length(intake_event_digest) = 32
        AND octet_length(project_authority_receipt_digest) = 32
        AND octet_length(subject_digest) = 32
        AND octet_length(observation_digest) = 32
        AND task_ref <> decode(repeat('00', 32), 'hex')
        AND intake_stream_id <> decode(repeat('00', 32), 'hex')
        AND intake_event_digest <> decode(repeat('00', 32), 'hex')
        AND project_authority_receipt_digest <> decode(repeat('00', 32), 'hex')
        AND subject_digest <> decode(repeat('00', 32), 'hex')
        AND observation_digest <> decode(repeat('00', 32), 'hex')
    )
);

-- Immutable source/spec reservation recorded before the first successor
-- Task-Ledger effect. This is lineage evidence, not a task phase or execution
-- authority; Task Ledger remains the sole task-state owner.
CREATE TABLE foreman_execution.promotion_intents (
    task_ref bytea PRIMARY KEY,
    project_id varchar(64) NOT NULL,
    project_snapshot_id varchar(159) NOT NULL,
    intake_stream_id bytea NOT NULL UNIQUE,
    intake_event_digest bytea NOT NULL UNIQUE,
    project_authority_receipt_digest bytea NOT NULL,
    successor_stream_id bytea NOT NULL UNIQUE,
    task_spec_digest bytea NOT NULL,
    approval_subject_digest bytea NOT NULL,
    budget_digest bytea NOT NULL,
    global_active_limit smallint NOT NULL,
    per_task_active_limit smallint NOT NULL,
    repair_retry_limit smallint NOT NULL,
    max_duration_seconds bigint NOT NULL,
    max_total_tokens bigint NOT NULL,
    max_model_calls bigint NOT NULL,
    external_cost_status varchar(16) NOT NULL,
    external_cost_limit_micros bigint,
    issued_at varchar(40) NOT NULL,
    deadline_at varchar(40) NOT NULL,
    budget_pointer varchar(80) NOT NULL,
    verification_policy_digest bytea NOT NULL,
    base_ref varchar(255) NOT NULL,
    base_commit char(40) NOT NULL,
    source_clean boolean NOT NULL,
    intent_digest bytea NOT NULL UNIQUE,
    recorded_at timestamp with time zone NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT promotion_intents_intake_stream_fk FOREIGN KEY (intake_stream_id)
        REFERENCES control.task_submission_envelopes (stream_id),
    CONSTRAINT promotion_intents_intake_event_fk FOREIGN KEY (intake_event_digest)
        REFERENCES control.task_ledger_events (event_digest),
    CONSTRAINT promotion_intents_project_shape CHECK (
        project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'
        AND project_snapshot_id ~ '^[a-z0-9._:-]{1,159}$'
    ),
    CONSTRAINT promotion_intents_budget_shape CHECK (
        global_active_limit = 4
        AND per_task_active_limit = 1
        AND repair_retry_limit BETWEEN 0 AND 2
        AND max_duration_seconds > 0
        AND max_total_tokens > 0
        AND max_model_calls > 0
        AND external_cost_status IN ('UNAVAILABLE', 'LIMIT_MICROS')
        AND ((external_cost_status = 'UNAVAILABLE' AND external_cost_limit_micros IS NULL)
             OR (external_cost_status = 'LIMIT_MICROS' AND external_cost_limit_micros IS NOT NULL
                 AND external_cost_limit_micros >= 0))
        AND issued_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
        AND deadline_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
        AND issued_at::timestamp with time zone < deadline_at::timestamp with time zone
        AND budget_pointer = 'budget:sha256:' || encode(budget_digest, 'hex')
    ),
    CONSTRAINT promotion_intents_source_shape CHECK (
        octet_length(base_ref) BETWEEN 1 AND 255
        AND base_ref NOT LIKE 'refs/remotes/%'
        AND base_ref NOT LIKE '%://%'
        AND base_ref !~ '[[:space:][:cntrl:]]'
        AND base_commit ~ '^[0-9a-f]{40}$'
        AND source_clean
    ),
    CONSTRAINT promotion_intents_digest_shapes CHECK (
        octet_length(task_ref) = 32
        AND octet_length(intake_stream_id) = 32
        AND octet_length(intake_event_digest) = 32
        AND octet_length(project_authority_receipt_digest) = 32
        AND octet_length(successor_stream_id) = 32
        AND octet_length(task_spec_digest) = 32
        AND octet_length(approval_subject_digest) = 32
        AND octet_length(budget_digest) = 32
        AND octet_length(verification_policy_digest) = 32
        AND octet_length(intent_digest) = 32
        AND task_ref <> decode(repeat('00', 32), 'hex')
        AND intake_stream_id <> decode(repeat('00', 32), 'hex')
        AND intake_event_digest <> decode(repeat('00', 32), 'hex')
        AND project_authority_receipt_digest <> decode(repeat('00', 32), 'hex')
        AND successor_stream_id <> decode(repeat('00', 32), 'hex')
        AND task_spec_digest <> decode(repeat('00', 32), 'hex')
        AND approval_subject_digest <> decode(repeat('00', 32), 'hex')
        AND budget_digest <> decode(repeat('00', 32), 'hex')
        AND verification_policy_digest <> decode(repeat('00', 32), 'hex')
        AND intent_digest <> decode(repeat('00', 32), 'hex')
    )
);

CREATE TABLE foreman_execution.task_promotions (
    task_ref bytea PRIMARY KEY REFERENCES foreman_execution.promotion_intents,
    project_id varchar(64) NOT NULL,
    project_snapshot_id varchar(159) NOT NULL,
    intake_stream_id bytea NOT NULL,
    intake_event_digest bytea NOT NULL,
    project_authority_receipt_digest bytea NOT NULL,
    successor_stream_id bytea NOT NULL UNIQUE,
    successor_task_created_event_digest bytea NOT NULL,
    task_spec_digest bytea NOT NULL,
    approval_subject_digest bytea NOT NULL,
    budget_digest bytea NOT NULL,
    global_active_limit smallint NOT NULL,
    per_task_active_limit smallint NOT NULL,
    repair_retry_limit smallint NOT NULL,
    max_duration_seconds bigint NOT NULL,
    max_total_tokens bigint NOT NULL,
    max_model_calls bigint NOT NULL,
    external_cost_status varchar(16) NOT NULL,
    external_cost_limit_micros bigint,
    deadline_at varchar(40) NOT NULL,
    budget_pointer varchar(80) NOT NULL,
    verification_policy_digest bytea NOT NULL,
    binding_digest bytea NOT NULL UNIQUE,
    base_ref varchar(255) NOT NULL,
    base_commit char(40) NOT NULL,
    ledger_event_digest bytea NOT NULL UNIQUE REFERENCES foreman_execution.child_events,
    CONSTRAINT task_promotions_project_shape CHECK (
        project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'
        AND project_snapshot_id ~ '^[a-z0-9._:-]{1,159}$'
    ),
    CONSTRAINT task_promotions_budget_shape CHECK (
        global_active_limit = 4
        AND per_task_active_limit = 1
        AND repair_retry_limit BETWEEN 0 AND 2
        AND max_duration_seconds > 0
        AND max_total_tokens > 0
        AND max_model_calls > 0
        AND external_cost_status IN ('UNAVAILABLE', 'LIMIT_MICROS')
        AND ((external_cost_status = 'UNAVAILABLE' AND external_cost_limit_micros IS NULL)
             OR (external_cost_status = 'LIMIT_MICROS' AND external_cost_limit_micros IS NOT NULL
                 AND external_cost_limit_micros >= 0))
        AND deadline_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
        AND budget_pointer = 'budget:sha256:' || encode(budget_digest, 'hex')
    ),
    CONSTRAINT task_promotions_source_shape CHECK (
        octet_length(base_ref) BETWEEN 1 AND 255
        AND base_ref NOT LIKE 'refs/remotes/%'
        AND base_ref NOT LIKE '%://%'
        AND base_ref !~ '[[:space:][:cntrl:]]'
        AND base_commit ~ '^[0-9a-f]{40}$'
    ),
    CONSTRAINT task_promotions_digest_shapes CHECK (
        octet_length(task_ref) = 32
        AND octet_length(intake_stream_id) = 32
        AND octet_length(intake_event_digest) = 32
        AND octet_length(project_authority_receipt_digest) = 32
        AND octet_length(successor_stream_id) = 32
        AND octet_length(successor_task_created_event_digest) = 32
        AND octet_length(task_spec_digest) = 32
        AND octet_length(approval_subject_digest) = 32
        AND octet_length(budget_digest) = 32
        AND octet_length(verification_policy_digest) = 32
        AND octet_length(binding_digest) = 32
        AND task_ref <> decode(repeat('00', 32), 'hex')
        AND intake_stream_id <> decode(repeat('00', 32), 'hex')
        AND intake_event_digest <> decode(repeat('00', 32), 'hex')
        AND project_authority_receipt_digest <> decode(repeat('00', 32), 'hex')
        AND successor_stream_id <> decode(repeat('00', 32), 'hex')
        AND successor_task_created_event_digest <> decode(repeat('00', 32), 'hex')
        AND task_spec_digest <> decode(repeat('00', 32), 'hex')
        AND approval_subject_digest <> decode(repeat('00', 32), 'hex')
        AND budget_digest <> decode(repeat('00', 32), 'hex')
        AND verification_policy_digest <> decode(repeat('00', 32), 'hex')
        AND binding_digest <> decode(repeat('00', 32), 'hex')
    )
);

CREATE TABLE foreman_execution.worker_attempts (
    task_ref bytea NOT NULL REFERENCES foreman_execution.task_promotions,
    attempt_number smallint NOT NULL,
    attempt_id varchar(128) NOT NULL UNIQUE,
    successor_stream_id bytea NOT NULL,
    task_spec_digest bytea NOT NULL,
    binding_digest bytea NOT NULL,
    budget_digest bytea NOT NULL,
    foreman_generation bigint NOT NULL,
    model varchar(32) NOT NULL,
    reasoning varchar(16) NOT NULL,
    writer_fence bigint NOT NULL,
    foreman_checkpoint_digest bytea NOT NULL,
    approval_receipt_digest bytea NOT NULL,
    packet_digest bytea NOT NULL,
    execution_environment_ref varchar(128) NOT NULL,
    worktree_digest bytea NOT NULL,
    base_commit_digest bytea NOT NULL,
    model_reason varchar(48) NOT NULL,
    model_reason_digest bytea NOT NULL,
    claimed_at varchar(40) NOT NULL,
    payload_digest bytea NOT NULL,
    ledger_event_digest bytea NOT NULL UNIQUE REFERENCES foreman_execution.child_events,
    PRIMARY KEY (task_ref, attempt_number),
    CONSTRAINT worker_attempts_closed_values CHECK (
        attempt_number BETWEEN 1 AND 3
        AND attempt_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND foreman_generation > 0
        AND model IN ('gpt-5.6-luna', 'gpt-5.6-terra', 'gpt-5.6-sol')
        AND (
            (model = 'gpt-5.6-luna' AND model_reason = 'BOUNDED_STATE_EVIDENCE_DOCUMENTATION')
            OR (model = 'gpt-5.6-terra' AND model_reason = 'ROUTINE_ENGINEERING')
            OR (
                model = 'gpt-5.6-sol'
                AND model_reason IN ('P0', 'ARCHITECTURE', 'SECURITY', 'HIGH_RISK', 'TERRA_INSUFFICIENT')
            )
        )
        AND reasoning IN ('low', 'medium', 'high', 'xhigh', 'max', 'ultra')
        AND writer_fence > 0
        AND execution_environment_ref ~ '^execution-environment:sha256:[a-f0-9]{64}$'
        AND execution_environment_ref <>
            'execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000000'
        AND claimed_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
    ),
    CONSTRAINT worker_attempts_digest_shapes CHECK (
        octet_length(task_ref) = 32
        AND octet_length(successor_stream_id) = 32
        AND octet_length(task_spec_digest) = 32
        AND octet_length(binding_digest) = 32
        AND octet_length(budget_digest) = 32
        AND octet_length(foreman_checkpoint_digest) = 32
        AND octet_length(approval_receipt_digest) = 32
        AND octet_length(packet_digest) = 32
        AND octet_length(worktree_digest) = 32
        AND octet_length(base_commit_digest) = 32
        AND octet_length(model_reason_digest) = 32
        AND octet_length(payload_digest) = 32
        AND successor_stream_id <> decode(repeat('00', 32), 'hex')
        AND task_spec_digest <> decode(repeat('00', 32), 'hex')
        AND binding_digest <> decode(repeat('00', 32), 'hex')
        AND budget_digest <> decode(repeat('00', 32), 'hex')
        AND foreman_checkpoint_digest <> decode(repeat('00', 32), 'hex')
        AND approval_receipt_digest <> decode(repeat('00', 32), 'hex')
        AND packet_digest <> decode(repeat('00', 32), 'hex')
        AND worktree_digest <> decode(repeat('00', 32), 'hex')
        AND base_commit_digest <> decode(repeat('00', 32), 'hex')
        AND model_reason_digest <> decode(repeat('00', 32), 'hex')
        AND payload_digest <> decode(repeat('00', 32), 'hex')
    )
);

CREATE TABLE foreman_execution.pending_worker_claims (
    task_ref bytea NOT NULL REFERENCES foreman_execution.task_promotions,
    attempt_number smallint NOT NULL,
    attempt_id varchar(128) NOT NULL,
    successor_stream_id bytea NOT NULL,
    task_spec_digest bytea NOT NULL,
    binding_digest bytea NOT NULL,
    budget_digest bytea NOT NULL,
    foreman_generation bigint NOT NULL,
    model varchar(32) NOT NULL,
    reasoning varchar(16) NOT NULL,
    writer_fence bigint NOT NULL,
    foreman_checkpoint_digest bytea NOT NULL,
    approval_receipt_digest bytea NOT NULL,
    packet_digest bytea NOT NULL,
    execution_environment_ref varchar(128) NOT NULL,
    worktree_digest bytea NOT NULL,
    base_commit_digest bytea NOT NULL,
    model_reason varchar(48) NOT NULL,
    model_reason_digest bytea NOT NULL,
    claimed_at varchar(40) NOT NULL,
    payload_digest bytea NOT NULL,
    max_attempts smallint NOT NULL,
    ledger_event_digest bytea NOT NULL UNIQUE REFERENCES foreman_execution.child_events,
    reserved_at timestamp with time zone NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (task_ref),
    CONSTRAINT pending_worker_claims_closed_values CHECK (
        attempt_number BETWEEN 1 AND 3
        AND max_attempts BETWEEN 1 AND 3
        AND attempt_number <= max_attempts
        AND attempt_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND foreman_generation > 0
        AND model IN ('gpt-5.6-luna', 'gpt-5.6-terra', 'gpt-5.6-sol')
        AND (
            (model = 'gpt-5.6-luna' AND model_reason = 'BOUNDED_STATE_EVIDENCE_DOCUMENTATION')
            OR (model = 'gpt-5.6-terra' AND model_reason = 'ROUTINE_ENGINEERING')
            OR (
                model = 'gpt-5.6-sol'
                AND model_reason IN ('P0', 'ARCHITECTURE', 'SECURITY', 'HIGH_RISK', 'TERRA_INSUFFICIENT')
            )
        )
        AND reasoning IN ('low', 'medium', 'high', 'xhigh', 'max', 'ultra')
        AND writer_fence > 0
        AND execution_environment_ref ~ '^execution-environment:sha256:[a-f0-9]{64}$'
        AND execution_environment_ref <>
            'execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000000'
        AND claimed_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
    ),
    CONSTRAINT pending_worker_claims_digest_shapes CHECK (
        octet_length(task_ref) = 32
        AND octet_length(successor_stream_id) = 32
        AND octet_length(task_spec_digest) = 32
        AND octet_length(binding_digest) = 32
        AND octet_length(budget_digest) = 32
        AND octet_length(foreman_checkpoint_digest) = 32
        AND octet_length(approval_receipt_digest) = 32
        AND octet_length(packet_digest) = 32
        AND octet_length(worktree_digest) = 32
        AND octet_length(base_commit_digest) = 32
        AND octet_length(model_reason_digest) = 32
        AND octet_length(payload_digest) = 32
        AND task_ref <> decode(repeat('00', 32), 'hex')
        AND successor_stream_id <> decode(repeat('00', 32), 'hex')
        AND task_spec_digest <> decode(repeat('00', 32), 'hex')
        AND binding_digest <> decode(repeat('00', 32), 'hex')
        AND budget_digest <> decode(repeat('00', 32), 'hex')
        AND foreman_checkpoint_digest <> decode(repeat('00', 32), 'hex')
        AND approval_receipt_digest <> decode(repeat('00', 32), 'hex')
        AND packet_digest <> decode(repeat('00', 32), 'hex')
        AND worktree_digest <> decode(repeat('00', 32), 'hex')
        AND base_commit_digest <> decode(repeat('00', 32), 'hex')
        AND model_reason_digest <> decode(repeat('00', 32), 'hex')
        AND payload_digest <> decode(repeat('00', 32), 'hex')
    )
);

CREATE TABLE foreman_execution.execution_environments (
    task_ref bytea NOT NULL REFERENCES foreman_execution.task_promotions,
    attempt_number smallint NOT NULL,
    attempt_id varchar(128) NOT NULL,
    packet_digest bytea NOT NULL,
    descriptor_schema varchar(64) NOT NULL,
    environment_kind varchar(24) NOT NULL,
    canonical_descriptor text NOT NULL,
    distribution varchar(64) NOT NULL,
    distribution_os_id varchar(64) NOT NULL,
    distribution_version varchar(128) NOT NULL,
    distribution_codename varchar(64) NOT NULL,
    distribution_os_release_digest bytea NOT NULL,
    distribution_kernel_release varchar(128) NOT NULL,
    distribution_identity_ref varchar(128) NOT NULL,
    distribution_identity_digest bytea NOT NULL,
    gateway_path varchar(1024) NOT NULL,
    gateway_version varchar(128) NOT NULL,
    gateway_digest bytea NOT NULL,
    linux_repository_path varchar(1024) NOT NULL,
    linux_codex_home_path varchar(1024) NOT NULL,
    codex_config_ref varchar(128) NOT NULL,
    codex_config_digest bytea NOT NULL,
    repository_head varchar(40) NOT NULL,
    repository_identity_ref varchar(128) NOT NULL,
    repository_identity_digest bytea NOT NULL,
    launcher_path varchar(1024) NOT NULL,
    launcher_version varchar(128) NOT NULL,
    launcher_digest bytea NOT NULL,
    node_path varchar(1024) NOT NULL,
    node_version varchar(128) NOT NULL,
    node_digest bytea NOT NULL,
    git_path varchar(1024) NOT NULL,
    git_version varchar(128) NOT NULL,
    git_digest bytea NOT NULL,
    supervisor_path varchar(1024) NOT NULL,
    supervisor_digest bytea NOT NULL,
    dbus_run_session_path varchar(1024) NOT NULL,
    dbus_run_session_digest bytea NOT NULL,
    setsid_path varchar(1024) NOT NULL,
    setsid_digest bytea NOT NULL,
    keyring_daemon_path varchar(1024) NOT NULL,
    keyring_daemon_digest bytea NOT NULL,
    keyring_library_path varchar(1024) NOT NULL,
    keyring_library_manifest_ref varchar(128) NOT NULL,
    keyring_library_manifest_digest bytea NOT NULL,
    xdg_runtime_dir varchar(1024) NOT NULL,
    credential_authority_kind varchar(48) NOT NULL,
    credential_authority_ref varchar(128) NOT NULL,
    credential_authority_digest bytea NOT NULL,
    process_fence_schema varchar(64) NOT NULL,
    process_fence_kind varchar(48) NOT NULL,
    systemd_run_path varchar(1024) NOT NULL,
    systemd_run_version varchar(128) NOT NULL,
    systemd_run_digest bytea NOT NULL,
    systemctl_path varchar(1024) NOT NULL,
    systemctl_version varchar(128) NOT NULL,
    systemctl_digest bytea NOT NULL,
    supervisor_bootstrap_node_path varchar(1024) NOT NULL,
    supervisor_bootstrap_node_version varchar(128) NOT NULL,
    supervisor_bootstrap_node_digest bytea NOT NULL,
    immutable_probe_lsattr_path varchar(1024) NOT NULL,
    immutable_probe_lsattr_version varchar(128) NOT NULL,
    immutable_probe_lsattr_digest bytea NOT NULL,
    noninteractive_root_probe_path varchar(1024) NOT NULL,
    noninteractive_root_probe_version varchar(128) NOT NULL,
    noninteractive_root_probe_digest bytea NOT NULL,
    cgroup_mount varchar(1024) NOT NULL,
    user_runtime_dir varchar(1024) NOT NULL,
    unit_prefix varchar(64) NOT NULL,
    process_fence_identity_ref varchar(128) NOT NULL,
    process_fence_identity_digest bytea NOT NULL,
    verification_toolchain_schema varchar(64) NOT NULL,
    verification_task_ref bytea NOT NULL,
    verification_task_root varchar(1024) NOT NULL,
    verification_isolation_root varchar(1024) NOT NULL,
    verification_owner_uid bigint NOT NULL,
    verification_home_dir varchar(1024) NOT NULL,
    verification_temp_dir varchar(1024) NOT NULL,
    npm_cache varchar(1024) NOT NULL,
    cargo_home varchar(1024) NOT NULL,
    cargo_target_dir varchar(1024) NOT NULL,
    cargo_host varchar(128) NOT NULL,
    npm_path varchar(1024) NOT NULL,
    npm_version varchar(128) NOT NULL,
    npm_digest bytea NOT NULL,
    cargo_path varchar(1024) NOT NULL,
    cargo_version varchar(128) NOT NULL,
    cargo_digest bytea NOT NULL,
    rustc_path varchar(1024) NOT NULL,
    rustc_version varchar(128) NOT NULL,
    rustc_digest bytea NOT NULL,
    rustdoc_path varchar(1024) NOT NULL,
    rustdoc_version varchar(128) NOT NULL,
    rustdoc_digest bytea NOT NULL,
    sandbox_path varchar(1024) NOT NULL,
    sandbox_version varchar(128) NOT NULL,
    sandbox_digest bytea NOT NULL,
    sandbox_helper_path varchar(1024) NOT NULL,
    sandbox_helper_version varchar(128) NOT NULL,
    sandbox_helper_digest bytea NOT NULL,
    verification_toolchain_identity_ref varchar(128) NOT NULL,
    verification_toolchain_identity_digest bytea NOT NULL,
    immutable_snapshot_ref varchar(128) NOT NULL,
    immutable_snapshot_digest bytea NOT NULL,
    sandbox_policy_ref varchar(128) NOT NULL,
    sandbox_policy_digest bytea NOT NULL,
    privilege_boundary_ref varchar(128) NOT NULL,
    privilege_boundary_digest bytea NOT NULL,
    path_mapping_windows_path varchar(1024) NOT NULL,
    path_mapping_linux_path varchar(1024) NOT NULL,
    path_mapping_ref varchar(128) NOT NULL,
    path_mapping_digest bytea NOT NULL,
    execution_domain_digest bytea NOT NULL,
    environment_ref varchar(128) NOT NULL,
    recorded_at timestamp with time zone NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (task_ref, attempt_number),
    CONSTRAINT execution_environments_closed_values CHECK (
        attempt_number BETWEEN 1 AND 3
        AND attempt_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND descriptor_schema = 'lattice.execution-environment.wsl2-linux/1.1'
        AND environment_kind = 'WSL2_LINUX'
        AND credential_authority_kind = 'LINUX_KEYRING'
        AND process_fence_schema = 'lattice.wsl2-cgroup-v2-fence/1.0'
        AND process_fence_kind = 'SYSTEMD_USER_SERVICE_CGROUP_V2'
        AND supervisor_bootstrap_node_path = '/usr/bin/node'
        AND immutable_probe_lsattr_path = '/usr/bin/lsattr'
        AND noninteractive_root_probe_path = '/usr/bin/sudo'
        AND cgroup_mount = '/sys/fs/cgroup'
        AND verification_toolchain_schema = 'lattice.wsl2-verification-toolchain/1.0'
        AND verification_owner_uid > 0
        AND immutable_snapshot_ref ~ '^wsl2-immutable-snapshot:sha256:[a-f0-9]{64}$'
        AND sandbox_policy_ref ~ '^wsl2-sandbox-policy:sha256:[a-f0-9]{64}$'
        AND privilege_boundary_ref ~ '^wsl2-privilege-boundary:sha256:[a-f0-9]{64}$'
        AND path_mapping_linux_path = linux_repository_path
        AND sandbox_path = launcher_path
        AND sandbox_version = launcher_version
        AND sandbox_digest = launcher_digest
        AND pg_catalog.octet_length(canonical_descriptor) BETWEEN 1 AND 16384
    ),
    CONSTRAINT execution_environments_digest_shapes CHECK (
        octet_length(task_ref) = 32
        AND octet_length(packet_digest) = 32
        AND octet_length(distribution_os_release_digest) = 32
        AND octet_length(distribution_identity_digest) = 32
        AND octet_length(gateway_digest) = 32
        AND octet_length(codex_config_digest) = 32
        AND octet_length(repository_identity_digest) = 32
        AND octet_length(launcher_digest) = 32
        AND octet_length(node_digest) = 32
        AND octet_length(git_digest) = 32
        AND octet_length(supervisor_digest) = 32
        AND octet_length(dbus_run_session_digest) = 32
        AND octet_length(setsid_digest) = 32
        AND octet_length(keyring_daemon_digest) = 32
        AND octet_length(keyring_library_manifest_digest) = 32
        AND octet_length(credential_authority_digest) = 32
        AND octet_length(systemd_run_digest) = 32
        AND octet_length(systemctl_digest) = 32
        AND octet_length(supervisor_bootstrap_node_digest) = 32
        AND octet_length(immutable_probe_lsattr_digest) = 32
        AND octet_length(noninteractive_root_probe_digest) = 32
        AND octet_length(process_fence_identity_digest) = 32
        AND octet_length(verification_task_ref) = 32
        AND octet_length(npm_digest) = 32
        AND octet_length(cargo_digest) = 32
        AND octet_length(rustc_digest) = 32
        AND octet_length(rustdoc_digest) = 32
        AND octet_length(sandbox_digest) = 32
        AND octet_length(sandbox_helper_digest) = 32
        AND octet_length(verification_toolchain_identity_digest) = 32
        AND octet_length(immutable_snapshot_digest) = 32
        AND octet_length(sandbox_policy_digest) = 32
        AND octet_length(privilege_boundary_digest) = 32
        AND octet_length(path_mapping_digest) = 32
        AND octet_length(execution_domain_digest) = 32
        AND execution_domain_digest <> decode(repeat('00', 32), 'hex')
        AND environment_ref = 'execution-environment:sha256:' || encode(execution_domain_digest, 'hex')
        AND distribution_identity_ref = 'wsl2-distribution:sha256:' || encode(distribution_identity_digest, 'hex')
        AND credential_authority_ref = 'wsl2-credential-authority:sha256:' || encode(credential_authority_digest, 'hex')
        AND keyring_library_manifest_ref = 'keyring-library-manifest:sha256:' || encode(keyring_library_manifest_digest, 'hex')
        AND process_fence_identity_ref = 'wsl2-process-fence-authority:sha256:' || encode(process_fence_identity_digest, 'hex')
        AND verification_toolchain_identity_ref = 'wsl2-verification-toolchain:sha256:' || encode(verification_toolchain_identity_digest, 'hex')
        AND immutable_snapshot_ref = 'wsl2-immutable-snapshot:sha256:' || encode(immutable_snapshot_digest, 'hex')
        AND sandbox_policy_ref = 'wsl2-sandbox-policy:sha256:' || encode(sandbox_policy_digest, 'hex')
        AND privilege_boundary_ref = 'wsl2-privilege-boundary:sha256:' || encode(privilege_boundary_digest, 'hex')
    )
);

CREATE TABLE foreman_execution.worker_observations (
    task_ref bytea NOT NULL,
    attempt_number smallint NOT NULL,
    observation_ordinal bigint NOT NULL,
    attempt_id varchar(128) NOT NULL,
    successor_stream_id bytea NOT NULL,
    binding_digest bytea NOT NULL,
    observation_kind varchar(32) NOT NULL,
    thread_id varchar(128) NOT NULL,
    turn_id varchar(128),
    app_server_generation bigint NOT NULL,
    app_server_identity_digest bytea NOT NULL,
    observed_at varchar(40) NOT NULL,
    evidence_digest bytea NOT NULL,
    payload_digest bytea NOT NULL,
    ledger_event_digest bytea NOT NULL UNIQUE REFERENCES foreman_execution.child_events,
    PRIMARY KEY (task_ref, attempt_number, observation_ordinal),
    CONSTRAINT worker_observations_attempt_fk FOREIGN KEY (task_ref, attempt_number)
        REFERENCES foreman_execution.worker_attempts,
    CONSTRAINT worker_observations_closed_values CHECK (
        observation_ordinal > 0
        AND observation_kind IN (
            'THREAD_ACCEPTED', 'TURN_ACCEPTED', 'TURN_STARTED',
            'PRESTART_TERMINAL_FAILED',
            'MEANINGFUL_PROGRESS', 'HEARTBEAT', 'STALL_CLASSIFIED',
            'INTERRUPT_REQUESTED', 'RECONCILED', 'TERMINAL_COMPLETED',
            'TERMINAL_FAILED', 'TERMINAL_INTERRUPTED'
        )
        AND thread_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND (turn_id IS NULL OR turn_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$')
        AND ((observation_kind = 'THREAD_ACCEPTED' AND turn_id IS NULL)
             OR (observation_kind <> 'THREAD_ACCEPTED' AND turn_id IS NOT NULL))
        AND app_server_generation > 0
        AND observed_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
    ),
    CONSTRAINT worker_observations_digest_shapes CHECK (
        octet_length(task_ref) = 32
        AND octet_length(successor_stream_id) = 32
        AND octet_length(binding_digest) = 32
        AND octet_length(app_server_identity_digest) = 32
        AND octet_length(evidence_digest) = 32
        AND octet_length(payload_digest) = 32
        AND successor_stream_id <> decode(repeat('00', 32), 'hex')
        AND binding_digest <> decode(repeat('00', 32), 'hex')
        AND app_server_identity_digest <> decode(repeat('00', 32), 'hex')
        AND evidence_digest <> decode(repeat('00', 32), 'hex')
        AND payload_digest <> decode(repeat('00', 32), 'hex')
    )
);

CREATE UNIQUE INDEX worker_observations_one_terminal
    ON foreman_execution.worker_observations (task_ref, attempt_number)
    WHERE observation_kind IN (
        'PRESTART_TERMINAL_FAILED', 'TERMINAL_COMPLETED',
        'TERMINAL_FAILED', 'TERMINAL_INTERRUPTED'
    );

CREATE TABLE foreman_execution.verification_records (
    task_ref bytea NOT NULL,
    attempt_number smallint NOT NULL,
    attempt_id varchar(128) NOT NULL,
    successor_stream_id bytea NOT NULL,
    task_spec_digest bytea NOT NULL,
    binding_digest bytea NOT NULL,
    outcome varchar(16) NOT NULL,
    verification_profile_digest bytea NOT NULL,
    base_commit_digest bytea NOT NULL,
    result_commit_digest bytea NOT NULL,
    tree_digest bytea NOT NULL,
    diff_digest bytea NOT NULL,
    result_digest bytea NOT NULL,
    evidence_artifact_digest bytea NOT NULL,
    review_digest bytea,
    verified_at varchar(40) NOT NULL,
    payload_digest bytea NOT NULL,
    ledger_event_digest bytea NOT NULL UNIQUE REFERENCES foreman_execution.child_events,
    PRIMARY KEY (task_ref, attempt_number),
    CONSTRAINT verification_records_attempt_fk FOREIGN KEY (task_ref, attempt_number)
        REFERENCES foreman_execution.worker_attempts,
    CONSTRAINT verification_records_closed_values CHECK (
        outcome IN ('PASSED', 'FAILED')
        AND verified_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
    ),
    CONSTRAINT verification_records_digest_shapes CHECK (
        octet_length(task_ref) = 32
        AND octet_length(successor_stream_id) = 32
        AND octet_length(task_spec_digest) = 32
        AND octet_length(binding_digest) = 32
        AND octet_length(verification_profile_digest) = 32
        AND octet_length(base_commit_digest) = 32
        AND octet_length(result_commit_digest) = 32
        AND octet_length(tree_digest) = 32
        AND octet_length(diff_digest) = 32
        AND octet_length(result_digest) = 32
        AND octet_length(evidence_artifact_digest) = 32
        AND (review_digest IS NULL OR octet_length(review_digest) = 32)
        AND octet_length(payload_digest) = 32
        AND successor_stream_id <> decode(repeat('00', 32), 'hex')
        AND task_spec_digest <> decode(repeat('00', 32), 'hex')
        AND binding_digest <> decode(repeat('00', 32), 'hex')
        AND verification_profile_digest <> decode(repeat('00', 32), 'hex')
        AND base_commit_digest <> decode(repeat('00', 32), 'hex')
        AND result_commit_digest <> decode(repeat('00', 32), 'hex')
        AND tree_digest <> decode(repeat('00', 32), 'hex')
        AND diff_digest <> decode(repeat('00', 32), 'hex')
        AND result_digest <> decode(repeat('00', 32), 'hex')
        AND evidence_artifact_digest <> decode(repeat('00', 32), 'hex')
        AND (review_digest IS NULL OR review_digest <> decode(repeat('00', 32), 'hex'))
        AND payload_digest <> decode(repeat('00', 32), 'hex')
    )
);

CREATE TABLE foreman_execution.artifact_references (
    project_id varchar(64) NOT NULL,
    task_ref bytea NOT NULL,
    attempt_number smallint NOT NULL,
    evidence_kind varchar(32) NOT NULL,
    media_type varchar(256) NOT NULL,
    payload_schema varchar(256) NOT NULL,
    producer_id varchar(256) NOT NULL,
    producer_version varchar(256) NOT NULL,
    producer_digest bytea NOT NULL,
    created_at varchar(40) NOT NULL,
    evidence_bytes bytea NOT NULL,
    content_digest bytea NOT NULL,
    descriptor_digest bytea NOT NULL,
    ledger_event_digest bytea NOT NULL UNIQUE REFERENCES foreman_execution.child_events,
    PRIMARY KEY (task_ref, attempt_number, descriptor_digest),
    CONSTRAINT artifact_references_attempt_fk FOREIGN KEY (task_ref, attempt_number)
        REFERENCES foreman_execution.worker_attempts,
    CONSTRAINT artifact_references_shape CHECK (
        project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'
        AND evidence_kind IN (
            'WORKER_LIFECYCLE', 'GIT_SNAPSHOT', 'VERIFICATION_RESULT',
            'REVIEW_RESULT', 'RESOURCE_OBSERVATION'
        )
        AND media_type ~ '^[A-Za-z0-9][A-Za-z0-9._:/+-]{0,255}$'
        AND payload_schema ~ '^[A-Za-z0-9][A-Za-z0-9._:/+-]{0,255}$'
        AND producer_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/+-]{0,255}$'
        AND producer_version ~ '^[A-Za-z0-9][A-Za-z0-9._:/+-]{0,255}$'
        AND created_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
        AND octet_length(evidence_bytes) BETWEEN 0 AND 1048576
        AND octet_length(producer_digest) = 32
        AND octet_length(content_digest) = 32
        AND octet_length(descriptor_digest) = 32
        AND producer_digest <> decode(repeat('00', 32), 'hex')
        AND content_digest <> decode(repeat('00', 32), 'hex')
        AND descriptor_digest <> decode(repeat('00', 32), 'hex')
    )
);

-- A single per-task durable outbox row closes the cross-repository crash
-- window. It is not task state: it retains only one exact Artifact Store
-- object plus the already owner-planned Task Ledger request/link until the
-- Ledger event and subordinate child row are both durably present.
CREATE TABLE foreman_execution.staged_artifact_references (
    task_ref bytea PRIMARY KEY,
    project_id varchar(64) NOT NULL,
    attempt_number smallint NOT NULL,
    evidence_kind varchar(32) NOT NULL,
    media_type varchar(256) NOT NULL,
    payload_schema varchar(256) NOT NULL,
    producer_id varchar(256) NOT NULL,
    producer_version varchar(256) NOT NULL,
    producer_digest bytea NOT NULL,
    created_at varchar(40) NOT NULL,
    evidence_bytes bytea NOT NULL,
    content_digest bytea NOT NULL,
    descriptor_digest bytea NOT NULL,
    ledger_stream_id bytea NOT NULL,
    before_sequence numeric(20,0) NOT NULL,
    before_last_event_digest bytea NOT NULL,
    before_resource_revision numeric(20,0) NOT NULL,
    before_resource_projection_digest bytea NOT NULL,
    before_head_digest bytea NOT NULL,
    ledger_event_sequence numeric(20,0) NOT NULL,
    ledger_event_digest bytea NOT NULL UNIQUE,
    ledger_command_id varchar(128) NOT NULL,
    ledger_request_digest bytea NOT NULL,
    ledger_payload_digest bytea NOT NULL,
    correlation_id varchar(128) NOT NULL,
    command_occurred_at varchar(40) NOT NULL,
    staged_at timestamp with time zone NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (ledger_stream_id, ledger_command_id),
    -- A pre-provider terminal blocker may be staged while the exact worker
    -- packet is still a pending reservation.  The staging function admits
    -- only that closed blocker shape; finalization still requires the worker
    -- attempt FK and is performed atomically with pending-attempt closure.
    CONSTRAINT staged_artifact_references_shape CHECK (
        project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'
        AND attempt_number BETWEEN 1 AND 3
        AND evidence_kind IN (
            'WORKER_LIFECYCLE', 'GIT_SNAPSHOT', 'VERIFICATION_RESULT',
            'REVIEW_RESULT', 'RESOURCE_OBSERVATION'
        )
        AND media_type ~ '^[A-Za-z0-9][A-Za-z0-9._:/+-]{0,255}$'
        AND payload_schema ~ '^[A-Za-z0-9][A-Za-z0-9._:/+-]{0,255}$'
        AND producer_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/+-]{0,255}$'
        AND producer_version ~ '^[A-Za-z0-9][A-Za-z0-9._:/+-]{0,255}$'
        AND created_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
        AND command_occurred_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
        AND ledger_command_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND correlation_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND pg_catalog.octet_length(evidence_bytes) BETWEEN 0 AND 1048576
        AND before_sequence BETWEEN 1 AND 18446744073709551614
        AND before_resource_revision BETWEEN 0 AND before_sequence
        AND ledger_event_sequence = before_sequence + 1
        AND ledger_event_sequence BETWEEN 2 AND 18446744073709551615
    ),
    CONSTRAINT staged_artifact_references_digest_shapes CHECK (
        pg_catalog.octet_length(task_ref) = 32
        AND pg_catalog.octet_length(producer_digest) = 32
        AND pg_catalog.octet_length(content_digest) = 32
        AND pg_catalog.octet_length(descriptor_digest) = 32
        AND pg_catalog.octet_length(ledger_stream_id) = 32
        AND pg_catalog.octet_length(before_last_event_digest) = 32
        AND pg_catalog.octet_length(before_resource_projection_digest) = 32
        AND pg_catalog.octet_length(before_head_digest) = 32
        AND pg_catalog.octet_length(ledger_event_digest) = 32
        AND pg_catalog.octet_length(ledger_request_digest) = 32
        AND pg_catalog.octet_length(ledger_payload_digest) = 32
        AND task_ref <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND producer_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND content_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND descriptor_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND ledger_stream_id <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND before_last_event_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND before_head_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND ledger_event_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND ledger_request_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND ledger_payload_digest = descriptor_digest
        AND (
            (before_resource_revision = 0
             AND before_resource_projection_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex'))
            OR
            (before_resource_revision > 0
             AND before_resource_projection_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex'))
        )
    )
);

-- One-shot provider-effect claims are subordinate receipts for a retained
-- Task Ledger attempt. They do not contain a task phase and cannot replace
-- either Task Ledger state or Artifact Store evidence.
CREATE TABLE foreman_execution.provider_dispatch_claims (
    task_ref bytea NOT NULL,
    attempt_number smallint NOT NULL,
    operation_kind varchar(32) NOT NULL,
    attempt_id varchar(128) NOT NULL,
    binding_digest bytea NOT NULL,
    writer_fence bigint NOT NULL,
    foreman_generation bigint NOT NULL,
    foreman_checkpoint_digest bytea NOT NULL,
    anchor_digest bytea NOT NULL,
    supporting_digest bytea NOT NULL,
    subject_digest bytea NOT NULL,
    dispatch_digest bytea NOT NULL,
    claim_receipt_digest bytea NOT NULL,
    claimed_at timestamp with time zone NOT NULL,
    PRIMARY KEY (task_ref, attempt_number, operation_kind),
    UNIQUE (dispatch_digest),
    UNIQUE (claim_receipt_digest),
    CONSTRAINT provider_dispatch_claims_attempt_fk FOREIGN KEY (task_ref, attempt_number)
        REFERENCES foreman_execution.worker_attempts,
    CONSTRAINT provider_dispatch_claims_closed_values CHECK (
        attempt_number BETWEEN 1 AND 3
        AND operation_kind IN ('WORKER_THREAD', 'WORKER_TURN', 'REVIEW_THREAD', 'REVIEW_TURN')
        AND attempt_id ~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
        AND writer_fence > 0
        AND foreman_generation > 0
    ),
    CONSTRAINT provider_dispatch_claims_digest_shapes CHECK (
        octet_length(task_ref) = 32
        AND octet_length(binding_digest) = 32
        AND octet_length(foreman_checkpoint_digest) = 32
        AND octet_length(anchor_digest) = 32
        AND octet_length(supporting_digest) = 32
        AND octet_length(subject_digest) = 32
        AND octet_length(dispatch_digest) = 32
        AND octet_length(claim_receipt_digest) = 32
        AND task_ref <> decode(repeat('00', 32), 'hex')
        AND binding_digest <> decode(repeat('00', 32), 'hex')
        AND foreman_checkpoint_digest <> decode(repeat('00', 32), 'hex')
        AND anchor_digest <> decode(repeat('00', 32), 'hex')
        AND supporting_digest <> decode(repeat('00', 32), 'hex')
        AND subject_digest <> decode(repeat('00', 32), 'hex')
        AND claim_receipt_digest <> decode(repeat('00', 32), 'hex')
        AND dispatch_digest <> decode(repeat('00', 32), 'hex')
    )
);

-- A closed attempt is not a synthetic verification. It is a typed, replayable
-- index over one exact blocker artifact. A blocker that was originally
-- retained because its provider effect was ambiguous additionally requires a
-- separate immutable exact no-effect reconciliation proof; the original
-- blocker is never rewritten or deleted.
CREATE TABLE foreman_execution.attempt_closures (
    task_ref bytea NOT NULL,
    attempt_number smallint NOT NULL,
    provider_disposition varchar(32) NOT NULL,
    blocker_code varchar(96) NOT NULL,
    blocker_descriptor_digest bytea NOT NULL,
    reconciliation_proof_descriptor_digest bytea,
    writer_fence bigint NOT NULL,
    closed_at varchar(40) NOT NULL,
    PRIMARY KEY (task_ref, attempt_number),
    UNIQUE (task_ref, attempt_number, blocker_descriptor_digest),
    CONSTRAINT attempt_closures_attempt_fk FOREIGN KEY (task_ref, attempt_number)
        REFERENCES foreman_execution.worker_attempts,
    CONSTRAINT attempt_closures_blocker_fk FOREIGN KEY (
        task_ref, attempt_number, blocker_descriptor_digest
    ) REFERENCES foreman_execution.artifact_references (
        task_ref, attempt_number, descriptor_digest
    ),
    CONSTRAINT attempt_closures_reconciliation_proof_fk FOREIGN KEY (
        task_ref, attempt_number, reconciliation_proof_descriptor_digest
    ) REFERENCES foreman_execution.artifact_references (
        task_ref, attempt_number, descriptor_digest
    ),
    CONSTRAINT attempt_closures_closed_values CHECK (
        provider_disposition = 'PROVEN_INACTIVE'
        AND (
            (
                blocker_code IN (
                    'LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT',
                    'LATTICE_MANAGED_PRESTART_CONFIGURATION_REJECTED',
                    'LATTICE_MANAGED_HEARTBEAT_TIMEOUT_WHILE_IN_PROGRESS',
                    'LATTICE_MANAGED_DEADLINE_EXCEEDED',
                    'LATTICE_MANAGED_MODEL_UNAVAILABLE',
                    'LATTICE_MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED',
                    'LATTICE_MANAGED_REVIEW_MODEL_PROBE_TIMEOUT_NO_PROVIDER_EFFECT',
                    'LATTICE_MANAGED_RETRY_BUDGET_EXHAUSTED',
                    'LATTICE_MANAGED_VERIFICATION_FAILED',
                    'LATTICE_MANAGED_REVIEW_RESULT_REJECTED',
                    'LATTICE_MANAGED_TOKEN_BUDGET_EXHAUSTED',
                    'LATTICE_MANAGED_MODEL_CALL_BUDGET_EXHAUSTED',
                    'LATTICE_MANAGED_MODEL_USAGE_RECONCILIATION_REQUIRED',
                    'LATTICE_MANAGED_REPOSITORY_LINEAGE_MISMATCH'
                )
                AND reconciliation_proof_descriptor_digest IS NULL
            )
            OR (
                blocker_code IN (
                    'LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL',
                    'LATTICE_MANAGED_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED',
                    'LATTICE_MANAGED_BRIDGE_HEARTBEAT_TIMEOUT_RECONCILIATION_REQUIRED',
                    'LATTICE_MANAGED_THREAD_START_RPC_INVALID_PARAMS',
                    'LATTICE_MANAGED_THREAD_START_RPC_REJECTED',
                    'LATTICE_MANAGED_TURN_START_RPC_INVALID_PARAMS',
                    'LATTICE_MANAGED_TURN_START_RPC_REJECTED'
                )
                AND reconciliation_proof_descriptor_digest IS NOT NULL
            )
        )
        AND writer_fence > 0
        AND closed_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
        AND octet_length(blocker_descriptor_digest) = 32
        AND blocker_descriptor_digest <> decode(repeat('00', 32), 'hex')
        AND (
            reconciliation_proof_descriptor_digest IS NULL
            OR (
                octet_length(reconciliation_proof_descriptor_digest) = 32
                AND reconciliation_proof_descriptor_digest <>
                    decode(repeat('00', 32), 'hex')
                AND reconciliation_proof_descriptor_digest <>
                    blocker_descriptor_digest
            )
        )
    )
);

CREATE TABLE foreman_execution.approval_owner_snapshots (
    snapshot_digest bytea PRIMARY KEY,
    snapshot_content_digest bytea NOT NULL,
    snapshot_bytes bytea NOT NULL,
    command_high_water bigint NOT NULL,
    command_tail_digest bytea NOT NULL,
    nonce_bindings_digest bytea NOT NULL,
    CONSTRAINT approval_owner_snapshots_closed_values CHECK (
        octet_length(snapshot_digest) = 32
        AND octet_length(snapshot_content_digest) = 32
        AND octet_length(snapshot_bytes) BETWEEN 1 AND 16777216
        AND command_high_water > 0
        AND octet_length(command_tail_digest) = 32
        AND octet_length(nonce_bindings_digest) = 32
        AND snapshot_digest <> decode(repeat('00', 32), 'hex')
        AND snapshot_content_digest <> decode(repeat('00', 32), 'hex')
        AND command_tail_digest <> decode(repeat('00', 32), 'hex')
        AND nonce_bindings_digest <> decode(repeat('00', 32), 'hex')
    )
);

CREATE TABLE foreman_execution.approval_evidence (
    task_ref bytea NOT NULL REFERENCES foreman_execution.task_promotions,
    successor_stream_id bytea NOT NULL,
    task_spec_digest bytea NOT NULL,
    approval_subject_digest bytea NOT NULL,
    budget_digest bytea NOT NULL,
    authority_source varchar(48) NOT NULL,
    capability varchar(48) NOT NULL,
    authority_evidence_digest bytea NOT NULL,
    approval_receipt_digest bytea,
    issued_at varchar(40) NOT NULL,
    expires_at varchar(40) NOT NULL,
    authority_digest bytea NOT NULL,
    approval_owner_snapshot_digest bytea
        REFERENCES foreman_execution.approval_owner_snapshots(snapshot_digest),
    ledger_event_digest bytea NOT NULL UNIQUE REFERENCES foreman_execution.child_events,
    PRIMARY KEY (task_ref, authority_digest),
    CONSTRAINT approval_evidence_closed_values CHECK (
        authority_source IN ('CLOSED_POLICY_NO_APPROVAL_REQUIRED', 'VERIFIED_APPROVAL')
        AND capability = 'LOCAL_REVERSIBLE_TASK_EXECUTION'
        AND ((authority_source = 'VERIFIED_APPROVAL'
              AND approval_receipt_digest IS NOT NULL
              AND approval_owner_snapshot_digest IS NOT NULL)
             OR (authority_source = 'CLOSED_POLICY_NO_APPROVAL_REQUIRED'
                 AND approval_receipt_digest IS NULL
                 AND approval_owner_snapshot_digest IS NULL))
        AND issued_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
        AND expires_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
        AND issued_at < expires_at
        AND octet_length(successor_stream_id) = 32
        AND octet_length(task_spec_digest) = 32
        AND octet_length(approval_subject_digest) = 32
        AND octet_length(budget_digest) = 32
        AND octet_length(authority_evidence_digest) = 32
        AND (approval_receipt_digest IS NULL OR octet_length(approval_receipt_digest) = 32)
        AND octet_length(authority_digest) = 32
        AND (approval_owner_snapshot_digest IS NULL
             OR octet_length(approval_owner_snapshot_digest) = 32)
        AND successor_stream_id <> decode(repeat('00', 32), 'hex')
        AND approval_subject_digest <> decode(repeat('00', 32), 'hex')
        AND task_spec_digest <> decode(repeat('00', 32), 'hex')
        AND budget_digest <> decode(repeat('00', 32), 'hex')
        AND authority_evidence_digest <> decode(repeat('00', 32), 'hex')
        AND (approval_receipt_digest IS NULL OR approval_receipt_digest <> decode(repeat('00', 32), 'hex'))
        AND authority_digest <> decode(repeat('00', 32), 'hex')
        AND (approval_owner_snapshot_digest IS NULL
             OR approval_owner_snapshot_digest <> decode(repeat('00', 32), 'hex'))
    )
);

CREATE FUNCTION foreman_execution.assert_task_ledger_event_v1(
    p_stream_id bytea,
    p_event_sequence numeric,
    p_event_digest bytea,
    p_command_id text,
    p_request_digest bytea,
    p_payload_digest bytea,
    p_action_id text
) RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
BEGIN
    IF (
        SELECT pg_catalog.count(*)
          FROM ONLY control.task_ledger_events AS e
          JOIN ONLY control.task_ledger_commands AS c
            ON c.stream_id = e.stream_id AND c.command_id = e.command_id
         WHERE e.stream_id = p_stream_id
           AND e.sequence = p_event_sequence
           AND e.event_digest = p_event_digest
           AND e.command_id = p_command_id
           AND e.request_digest = p_request_digest
           AND e.subject_digest = p_payload_digest
           AND e.action_id = p_action_id
           AND c.command_outcome = 'APPENDED'
           AND c.event_digest = e.event_digest
           AND c.request_digest = e.request_digest
           AND c.subject_digest = e.subject_digest
           AND c.action_id = e.action_id
    ) <> 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_TASK_LEDGER_EVENT_MISMATCH';
    END IF;
END;
$$;

CREATE FUNCTION foreman_execution.insert_child_event_v1(
    p_record_kind text,
    p_task_ref bytea,
    p_stream_id bytea,
    p_event_sequence numeric,
    p_event_digest bytea,
    p_command_id text,
    p_request_digest bytea,
    p_payload_digest bytea,
    p_action_id text
) RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM ONLY foreman_execution.child_events AS e
         WHERE e.ledger_event_digest = p_event_digest
            OR (e.ledger_stream_id = p_stream_id AND e.ledger_event_sequence = p_event_sequence)
            OR (e.ledger_stream_id = p_stream_id AND e.ledger_command_id = p_command_id)
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_CHILD_EVENT_SUBSTITUTION';
    END IF;
    PERFORM foreman_execution.assert_task_ledger_event_v1(
        p_stream_id, p_event_sequence, p_event_digest, p_command_id,
        p_request_digest, p_payload_digest, p_action_id
    );
    INSERT INTO foreman_execution.child_events (
        ledger_event_digest, ledger_stream_id, ledger_event_sequence,
        ledger_command_id, ledger_request_digest, ledger_payload_digest,
        action_id, record_kind, task_ref
    ) VALUES (
        p_event_digest, p_stream_id, p_event_sequence, p_command_id,
        p_request_digest, p_payload_digest, p_action_id, p_record_kind, p_task_ref
    );
END;
$$;

CREATE FUNCTION foreman_execution.assert_exact_child_event_v1(
    p_record_kind text,
    p_task_ref bytea,
    p_stream_id bytea,
    p_event_sequence numeric,
    p_event_digest bytea,
    p_command_id text,
    p_request_digest bytea,
    p_payload_digest bytea,
    p_action_id text
) RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
BEGIN
    PERFORM foreman_execution.assert_task_ledger_event_v1(
        p_stream_id, p_event_sequence, p_event_digest, p_command_id,
        p_request_digest, p_payload_digest, p_action_id
    );
    IF (
        SELECT pg_catalog.count(*)
          FROM ONLY foreman_execution.child_events AS e
         WHERE e.ledger_event_digest = p_event_digest
           AND e.ledger_stream_id = p_stream_id
           AND e.ledger_event_sequence = p_event_sequence
           AND e.ledger_command_id = p_command_id
           AND e.ledger_request_digest = p_request_digest
           AND e.ledger_payload_digest = p_payload_digest
           AND e.action_id = p_action_id
           AND e.record_kind = p_record_kind
           AND e.task_ref = p_task_ref
    ) <> 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_CHILD_EVENT_REPLAY_MISMATCH';
    END IF;
END;
$$;

CREATE FUNCTION foreman_execution.record_preparation_observation_v1(
    p_task_ref bytea, p_project_id text, p_project_snapshot_id text,
    p_project_authority_receipt_digest bytea, p_observation_kind text,
    p_subject_digest bytea, p_observed_at text, p_observation_digest bytea
) RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_existing foreman_execution.preparation_observations%ROWTYPE;
    v_intake_stream_id bytea;
    v_intake_event_digest bytea;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);
    IF p_task_ref IS NULL OR pg_catalog.octet_length(p_task_ref) <> 32
       OR p_task_ref = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_observation_kind NOT IN (
           'WORKTREE_NOT_CLEAN', 'PROJECT_REGISTRY_CURRENTNESS_CONFLICT', 'CLEARED'
       )
       OR p_subject_digest IS NULL OR pg_catalog.octet_length(p_subject_digest) <> 32
       OR p_subject_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_observation_digest IS NULL OR pg_catalog.octet_length(p_observation_digest) <> 32
       OR p_observation_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_observed_at !~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$' THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_PREPARATION_OBSERVATION_INPUT_INVALID';
    END IF;

    SELECT submission.stream_id, submission.event_digest
      INTO v_intake_stream_id, v_intake_event_digest
      FROM ONLY control.task_submission_envelopes AS submission
      JOIN ONLY control.task_ingress_claims AS ingress
        ON ingress.ingress_id = submission.ingress_id
       AND ingress.client_request_id = submission.client_request_id
       AND ingress.request_kind = 'GENERAL_TASK'
       AND ingress.stream_id = submission.stream_id
       AND ingress.event_sequence = submission.event_sequence
       AND ingress.event_digest = submission.event_digest
      JOIN ONLY control.task_ledger_streams AS intake_stream
        ON intake_stream.stream_id = submission.stream_id
       AND intake_stream.project_id = submission.project_id
       AND intake_stream.project_snapshot_id = submission.project_snapshot_id
       AND intake_stream.task_id = submission.task_id
       AND intake_stream.task_revision = submission.task_revision
       AND intake_stream.task_subject_kind = 'GENERAL_TASK_INTAKE'
       AND intake_stream.sequence = submission.event_sequence
       AND intake_stream.last_event_digest = submission.event_digest
      JOIN ONLY control.task_ledger_events AS intake_event
        ON intake_event.stream_id = submission.stream_id
       AND intake_event.sequence = submission.event_sequence
       AND intake_event.event_digest = submission.event_digest
       AND intake_event.event_kind = 'TASK_CREATED'
       AND intake_event.action_id = 'GENERAL_TASK_INTAKE_V1'
     WHERE pg_catalog.decode(submission.task_ref, 'hex') = p_task_ref
       AND submission.project_id = p_project_id
       AND submission.project_snapshot_id = p_project_snapshot_id
       AND submission.project_authority_receipt_digest = p_project_authority_receipt_digest
       AND submission.schema_version = 'lattice.task-ledger.task-submission/1.0'
       AND submission.task_subject_kind = 'GENERAL_TASK_INTAKE';
    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_PREPARATION_OBSERVATION_LINEAGE_MISMATCH';
    END IF;

    SELECT * INTO v_existing
      FROM ONLY foreman_execution.preparation_observations
     WHERE task_ref = p_task_ref
     FOR UPDATE;
    IF FOUND THEN
        IF v_existing.project_id IS DISTINCT FROM p_project_id
           OR v_existing.project_snapshot_id IS DISTINCT FROM p_project_snapshot_id
           OR v_existing.intake_stream_id IS DISTINCT FROM v_intake_stream_id
           OR v_existing.intake_event_digest IS DISTINCT FROM v_intake_event_digest
           OR v_existing.project_authority_receipt_digest IS DISTINCT FROM p_project_authority_receipt_digest THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_PREPARATION_OBSERVATION_SUBSTITUTION';
        END IF;
        IF v_existing.observation_kind = p_observation_kind
           AND v_existing.subject_digest = p_subject_digest
           AND v_existing.observed_at = p_observed_at
           AND v_existing.observation_digest = p_observation_digest THEN
            RETURN 'EXACT_REPLAY';
        END IF;
        UPDATE ONLY foreman_execution.preparation_observations
           SET observation_kind = p_observation_kind,
               subject_digest = p_subject_digest,
               observed_at = p_observed_at,
               observation_digest = p_observation_digest,
               observation_generation = observation_generation + 1,
               updated_at = pg_catalog.clock_timestamp()
         WHERE task_ref = p_task_ref;
        RETURN 'INSERTED';
    END IF;
    INSERT INTO foreman_execution.preparation_observations (
        task_ref, project_id, project_snapshot_id, intake_stream_id,
        intake_event_digest, project_authority_receipt_digest,
        observation_kind, subject_digest, observed_at, observation_digest
    ) VALUES (
        p_task_ref, p_project_id, p_project_snapshot_id, v_intake_stream_id,
        v_intake_event_digest, p_project_authority_receipt_digest,
        p_observation_kind, p_subject_digest, p_observed_at, p_observation_digest
    );
    RETURN 'INSERTED';
END;
$$;

CREATE FUNCTION foreman_execution.read_preparation_observation_v1(p_task_ref bytea)
RETURNS TABLE(
    project_id text, project_snapshot_id text,
    project_authority_receipt_digest bytea, observation_kind text,
    subject_digest bytea, observed_at text, observation_digest bytea
)
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_existing foreman_execution.preparation_observations%ROWTYPE;
    v_lineage_count bigint;
BEGIN
    SELECT * INTO v_existing
      FROM ONLY foreman_execution.preparation_observations AS observation
     WHERE observation.task_ref = p_task_ref;
    IF NOT FOUND THEN
        RETURN;
    END IF;

    SELECT pg_catalog.count(*) INTO v_lineage_count
      FROM ONLY control.task_submission_envelopes AS submission
      JOIN ONLY control.task_ledger_events AS intake_event
        ON intake_event.stream_id = submission.stream_id
       AND intake_event.sequence = submission.event_sequence
       AND intake_event.event_digest = submission.event_digest
       AND intake_event.event_kind = 'TASK_CREATED'
       AND intake_event.action_id = 'GENERAL_TASK_INTAKE_V1'
     WHERE pg_catalog.decode(submission.task_ref, 'hex') = p_task_ref
       AND submission.stream_id = v_existing.intake_stream_id
       AND submission.event_digest = v_existing.intake_event_digest
       AND submission.project_id = v_existing.project_id
       AND submission.project_snapshot_id = v_existing.project_snapshot_id
       AND submission.project_authority_receipt_digest =
           v_existing.project_authority_receipt_digest
       AND submission.schema_version = 'lattice.task-ledger.task-submission/1.0'
       AND submission.task_subject_kind = 'GENERAL_TASK_INTAKE'
       AND submission.admission_action = 'GENERAL_TASK_INTAKE_V1';
    IF v_lineage_count <> 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_PREPARATION_OBSERVATION_LINEAGE_MISMATCH';
    END IF;

    RETURN QUERY
    SELECT v_existing.project_id::text,
           v_existing.project_snapshot_id::text,
           v_existing.project_authority_receipt_digest,
           v_existing.observation_kind::text,
           v_existing.subject_digest, v_existing.observed_at::text,
           v_existing.observation_digest;
END;
$$;

CREATE FUNCTION foreman_execution.record_promotion_intent_v1(
    p_task_ref bytea, p_project_id text, p_project_snapshot_id text,
    p_project_authority_receipt_digest bytea, p_successor_stream_id bytea,
    p_task_spec_digest bytea, p_approval_subject_digest bytea,
    p_budget_digest bytea, p_global_active_limit smallint,
    p_per_task_active_limit smallint, p_repair_retry_limit smallint,
    p_max_duration_seconds bigint, p_max_total_tokens bigint,
    p_max_model_calls bigint, p_external_cost_status text,
    p_external_cost_limit_micros bigint, p_issued_at text,
    p_deadline_at text, p_budget_pointer text,
    p_verification_policy_digest bytea, p_base_ref text,
    p_base_commit text, p_source_clean boolean, p_intent_digest bytea
) RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_existing foreman_execution.promotion_intents%ROWTYPE;
    v_intake_stream_id bytea;
    v_intake_event_digest bytea;
    v_lineage_count bigint;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);
    SELECT * INTO v_existing FROM ONLY foreman_execution.promotion_intents
     WHERE task_ref = p_task_ref;
    IF FOUND THEN
        SELECT pg_catalog.count(*) INTO v_lineage_count
          FROM ONLY control.task_submission_envelopes AS submission
          JOIN ONLY control.task_ledger_events AS intake_event
            ON intake_event.stream_id = submission.stream_id
           AND intake_event.sequence = submission.event_sequence
           AND intake_event.event_digest = submission.event_digest
           AND intake_event.event_kind = 'TASK_CREATED'
           AND intake_event.action_id = 'GENERAL_TASK_INTAKE_V1'
         WHERE pg_catalog.decode(submission.task_ref, 'hex') = p_task_ref
           AND submission.stream_id = v_existing.intake_stream_id
           AND submission.event_digest = v_existing.intake_event_digest
           AND submission.project_id = v_existing.project_id
           AND submission.project_snapshot_id = v_existing.project_snapshot_id
           AND submission.project_authority_receipt_digest =
               v_existing.project_authority_receipt_digest
           AND submission.schema_version = 'lattice.task-ledger.task-submission/1.0'
           AND submission.task_subject_kind = 'GENERAL_TASK_INTAKE'
           AND submission.admission_action = 'GENERAL_TASK_INTAKE_V1';
        IF v_lineage_count <> 1 THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_PROMOTION_INTENT_LINEAGE_MISMATCH';
        END IF;
        IF v_existing.project_id IS DISTINCT FROM p_project_id
           OR v_existing.project_snapshot_id IS DISTINCT FROM p_project_snapshot_id
           OR v_existing.project_authority_receipt_digest IS DISTINCT FROM p_project_authority_receipt_digest
           OR v_existing.successor_stream_id IS DISTINCT FROM p_successor_stream_id
           OR v_existing.task_spec_digest IS DISTINCT FROM p_task_spec_digest
           OR v_existing.approval_subject_digest IS DISTINCT FROM p_approval_subject_digest
           OR v_existing.budget_digest IS DISTINCT FROM p_budget_digest
           OR v_existing.global_active_limit IS DISTINCT FROM p_global_active_limit
           OR v_existing.per_task_active_limit IS DISTINCT FROM p_per_task_active_limit
           OR v_existing.repair_retry_limit IS DISTINCT FROM p_repair_retry_limit
           OR v_existing.max_duration_seconds IS DISTINCT FROM p_max_duration_seconds
           OR v_existing.max_total_tokens IS DISTINCT FROM p_max_total_tokens
           OR v_existing.max_model_calls IS DISTINCT FROM p_max_model_calls
           OR v_existing.external_cost_status IS DISTINCT FROM p_external_cost_status
           OR v_existing.external_cost_limit_micros IS DISTINCT FROM p_external_cost_limit_micros
           OR v_existing.issued_at IS DISTINCT FROM p_issued_at
           OR v_existing.deadline_at IS DISTINCT FROM p_deadline_at
           OR v_existing.budget_pointer IS DISTINCT FROM p_budget_pointer
           OR v_existing.verification_policy_digest IS DISTINCT FROM p_verification_policy_digest
           OR v_existing.base_ref IS DISTINCT FROM p_base_ref
           OR v_existing.base_commit IS DISTINCT FROM p_base_commit
           OR v_existing.source_clean IS DISTINCT FROM p_source_clean
           OR v_existing.intent_digest IS DISTINCT FROM p_intent_digest THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PROMOTION_INTENT_SUBSTITUTION';
        END IF;
        RETURN 'EXACT_REPLAY';
    END IF;
    IF p_task_ref IS NULL OR pg_catalog.octet_length(p_task_ref) <> 32
       OR p_task_ref = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_successor_stream_id IS NULL OR pg_catalog.octet_length(p_successor_stream_id) <> 32
       OR p_successor_stream_id = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR NOT p_source_clean
       OR p_intent_digest IS NULL OR pg_catalog.octet_length(p_intent_digest) <> 32
       OR p_intent_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex') THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PROMOTION_INTENT_INPUT_INVALID';
    END IF;
    SELECT submission.stream_id, submission.event_digest
      INTO v_intake_stream_id, v_intake_event_digest
      FROM ONLY control.task_submission_envelopes AS submission
      JOIN ONLY control.task_ingress_claims AS ingress
        ON ingress.ingress_id = submission.ingress_id
       AND ingress.client_request_id = submission.client_request_id
       AND ingress.request_kind = 'GENERAL_TASK'
       AND ingress.stream_id = submission.stream_id
       AND ingress.event_sequence = submission.event_sequence
       AND ingress.event_digest = submission.event_digest
       AND ingress.command_id = submission.command_id
       AND ingress.command_request_digest = submission.request_digest
      JOIN ONLY control.task_ledger_streams AS intake_stream
        ON intake_stream.stream_id = submission.stream_id
       AND intake_stream.project_id = submission.project_id
       AND intake_stream.project_snapshot_id = submission.project_snapshot_id
       AND intake_stream.task_id = submission.task_id
       AND intake_stream.task_revision = submission.task_revision
       AND intake_stream.task_subject_kind = submission.task_subject_kind
       AND intake_stream.task_subject_digest = submission.intake_digest
       AND intake_stream.sequence = submission.event_sequence
       AND intake_stream.last_event_digest = submission.event_digest
      JOIN ONLY control.task_ledger_events AS intake_event
        ON intake_event.stream_id = submission.stream_id
       AND intake_event.sequence = submission.event_sequence
       AND intake_event.event_digest = submission.event_digest
       AND intake_event.command_id = submission.command_id
       AND intake_event.request_digest = submission.request_digest
       AND intake_event.subject_digest = submission.envelope_digest
      JOIN ONLY control.task_ledger_commands AS intake_command
        ON intake_command.stream_id = submission.stream_id
       AND intake_command.command_id = submission.command_id
       AND intake_command.request_digest = submission.request_digest
       AND intake_command.event_digest = submission.event_digest
      JOIN ONLY control.project_registry_projects AS project
        ON project.project_id = submission.project_id
       AND project.authority_snapshot_id = submission.project_snapshot_id
       AND project.authority_receipt_digest = submission.project_authority_receipt_digest
     WHERE pg_catalog.decode(submission.task_ref, 'hex') = p_task_ref
       AND submission.project_id = p_project_id
       AND submission.project_snapshot_id = p_project_snapshot_id
       AND submission.project_authority_receipt_digest = p_project_authority_receipt_digest
       AND submission.schema_version = 'lattice.task-ledger.task-submission/1.0'
       AND submission.task_subject_kind = 'GENERAL_TASK_INTAKE'
       AND submission.admission_action = 'GENERAL_TASK_INTAKE_V1'
       AND intake_stream.ledger_schema_version = '2.0'
       AND intake_stream.head_contract_version = 1
       AND intake_stream.producer_id = 'lattice-task-ledger'
       AND intake_stream.producer_version = '2.0'
       AND intake_stream.runtime = 'LIVE'
       AND intake_stream.task_spec_digest IS NULL
       AND intake_stream.accounting_currency IS NULL
       AND intake_event.event_schema_version = '2.0'
       AND intake_event.event_kind = 'TASK_CREATED'
       AND intake_event.action_id = 'GENERAL_TASK_INTAKE_V1'
       AND intake_event.audit_outcome = 'RECORDED'
       AND intake_event.reason_code = 'GENERAL_TASK_INTAKE_RECORDED'
       AND intake_event.diagnostic = 'null'::jsonb
       AND intake_command.command_outcome = 'APPENDED'
       AND intake_command.event_kind = 'TASK_CREATED'
       AND intake_command.action_id = 'GENERAL_TASK_INTAKE_V1'
       AND intake_command.audit_outcome = 'RECORDED'
       AND intake_command.reason_code = 'GENERAL_TASK_INTAKE_RECORDED'
       AND intake_command.subject_digest = submission.envelope_digest
       AND project.project_class = 'USER_PROJECT'
       AND project.authority_runtime = 'LIVE'
       AND project.authority_lifecycle = 'ACTIVE'
       AND project.pending_observation_digest IS NULL
       AND NOT project.drift_canonical_root
       AND NOT project.drift_repository
       AND NOT project.drift_file
       AND NOT project.drift_primary_ref_name
       AND NOT project.drift_primary_ref_storage
       AND project.authority_observation_digest = project.accepted_observation_digest;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PROMOTION_INTENT_LINEAGE_MISMATCH';
    END IF;
    INSERT INTO foreman_execution.promotion_intents (
        task_ref, project_id, project_snapshot_id, intake_stream_id,
        intake_event_digest, project_authority_receipt_digest,
        successor_stream_id, task_spec_digest, approval_subject_digest,
        budget_digest, global_active_limit, per_task_active_limit,
        repair_retry_limit, max_duration_seconds, max_total_tokens,
        max_model_calls, external_cost_status, external_cost_limit_micros,
        issued_at, deadline_at, budget_pointer, verification_policy_digest,
        base_ref, base_commit, source_clean, intent_digest
    ) VALUES (
        p_task_ref, p_project_id, p_project_snapshot_id, v_intake_stream_id,
        v_intake_event_digest, p_project_authority_receipt_digest,
        p_successor_stream_id, p_task_spec_digest, p_approval_subject_digest,
        p_budget_digest, p_global_active_limit, p_per_task_active_limit,
        p_repair_retry_limit, p_max_duration_seconds, p_max_total_tokens,
        p_max_model_calls, p_external_cost_status, p_external_cost_limit_micros,
        p_issued_at, p_deadline_at, p_budget_pointer, p_verification_policy_digest,
        p_base_ref, p_base_commit, p_source_clean, p_intent_digest
    );
    RETURN 'INSERTED';
END;
$$;

CREATE FUNCTION foreman_execution.read_promotion_intent_v1(p_task_ref bytea)
RETURNS TABLE(
    project_id text, project_snapshot_id text,
    project_authority_receipt_digest bytea, successor_stream_id bytea,
    task_spec_digest bytea, approval_subject_digest bytea, budget_digest bytea,
    global_active_limit smallint, per_task_active_limit smallint,
    repair_retry_limit smallint, max_duration_seconds bigint,
    max_total_tokens bigint, max_model_calls bigint,
    external_cost_status text, external_cost_limit_micros bigint,
    issued_at text, deadline_at text, budget_pointer text,
    verification_policy_digest bytea, base_ref text, base_commit text,
    source_clean boolean, intent_digest bytea, recorded_at text
)
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_intent foreman_execution.promotion_intents%ROWTYPE;
    v_lineage_count bigint;
    v_candidate_count bigint;
    v_exact_candidate_count bigint;
BEGIN
    SELECT * INTO v_intent
      FROM ONLY foreman_execution.promotion_intents AS intent
     WHERE intent.task_ref = p_task_ref;
    IF NOT FOUND THEN
        RETURN;
    END IF;

    SELECT pg_catalog.count(*) INTO v_lineage_count
      FROM ONLY control.task_submission_envelopes AS submission
      JOIN ONLY control.task_ledger_events AS intake_event
        ON intake_event.stream_id = submission.stream_id
       AND intake_event.sequence = submission.event_sequence
       AND intake_event.event_digest = submission.event_digest
       AND intake_event.event_kind = 'TASK_CREATED'
       AND intake_event.action_id = 'GENERAL_TASK_INTAKE_V1'
     WHERE pg_catalog.decode(submission.task_ref, 'hex') = p_task_ref
       AND submission.stream_id = v_intent.intake_stream_id
       AND submission.event_digest = v_intent.intake_event_digest
       AND submission.project_id = v_intent.project_id
       AND submission.project_snapshot_id = v_intent.project_snapshot_id
       AND submission.project_authority_receipt_digest =
           v_intent.project_authority_receipt_digest
       AND submission.schema_version = 'lattice.task-ledger.task-submission/1.0'
       AND submission.task_subject_kind = 'GENERAL_TASK_INTAKE'
       AND submission.admission_action = 'GENERAL_TASK_INTAKE_V1';
    IF v_lineage_count <> 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_PROMOTION_INTENT_LINEAGE_MISMATCH';
    END IF;

    -- A crash after successor admission but before promotion binding must
    -- never permit a mutable Git re-sample to open another Task Ledger
    -- stream.  The retained intent is the unique lineage anchor; zero
    -- successor candidates is the pre-admission crash window, exactly one
    -- matching candidate is replayable, and every other shape is ambiguous.
    SELECT pg_catalog.count(*),
           pg_catalog.count(*) FILTER (
               WHERE successor.stream_id = intent.successor_stream_id
                 AND successor.task_spec_digest = intent.task_spec_digest
           )
      INTO v_candidate_count, v_exact_candidate_count
      FROM ONLY foreman_execution.promotion_intents AS intent
      JOIN ONLY control.task_ledger_streams AS intake
        ON intake.stream_id = intent.intake_stream_id
       AND intake.project_id = intent.project_id
       AND intake.project_snapshot_id = intent.project_snapshot_id
       AND intake.task_subject_kind = 'GENERAL_TASK_INTAKE'
      JOIN ONLY control.task_ledger_streams AS successor
        ON successor.project_id = intake.project_id
       AND successor.project_snapshot_id = intake.project_snapshot_id
       AND successor.task_id = intake.task_id
       AND successor.task_revision = intake.task_revision
       AND successor.task_subject_kind = 'TASK_SPEC'
       AND successor.runtime = 'LIVE'
      JOIN ONLY control.task_ledger_events AS created
        ON created.stream_id = successor.stream_id
       AND created.sequence = 1
       AND created.event_kind = 'TASK_CREATED'
       AND created.action_id = 'MANAGED_GENERAL_TASK_V1'
     WHERE intent.task_ref = p_task_ref;
    IF v_candidate_count > 1
       OR (v_candidate_count = 1 AND v_exact_candidate_count <> 1) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_PROMOTION_SUCCESSOR_AMBIGUOUS';
    END IF;

    RETURN QUERY
    SELECT i.project_id::text, i.project_snapshot_id::text,
           i.project_authority_receipt_digest, i.successor_stream_id,
           i.task_spec_digest, i.approval_subject_digest, i.budget_digest,
           i.global_active_limit, i.per_task_active_limit,
           i.repair_retry_limit, i.max_duration_seconds,
           i.max_total_tokens, i.max_model_calls,
           i.external_cost_status::text, i.external_cost_limit_micros,
           i.issued_at::text, i.deadline_at::text, i.budget_pointer::text,
           i.verification_policy_digest, i.base_ref::text, i.base_commit::text,
           i.source_clean, i.intent_digest,
           pg_catalog.to_char(i.recorded_at AT TIME ZONE 'UTC',
                              'YYYY-MM-DD"T"HH24:MI:SS.US"Z"')
      FROM ONLY foreman_execution.promotion_intents AS i
     WHERE i.task_ref = p_task_ref;
END;
$$;

CREATE FUNCTION foreman_execution.record_task_promotion_v1(
    p_task_ref bytea, p_project_id text, p_project_snapshot_id text,
    p_intake_stream_id bytea, p_intake_event_digest bytea,
    p_project_authority_receipt_digest bytea, p_successor_stream_id bytea,
    p_successor_task_created_event_digest bytea, p_task_spec_digest bytea,
    p_approval_subject_digest bytea, p_budget_digest bytea,
    p_global_active_limit smallint, p_per_task_active_limit smallint,
    p_repair_retry_limit smallint, p_max_duration_seconds bigint,
    p_max_total_tokens bigint, p_max_model_calls bigint,
    p_external_cost_status text, p_external_cost_limit_micros bigint,
    p_deadline_at text, p_budget_pointer text,
    p_verification_policy_digest bytea, p_binding_digest bytea,
    p_base_ref text, p_base_commit text,
    p_stream_id bytea, p_event_sequence numeric, p_event_digest bytea,
    p_command_id text, p_request_digest bytea, p_payload_digest bytea
) RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE v_existing foreman_execution.task_promotions%ROWTYPE;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);
    IF (SELECT pg_catalog.count(*)
          FROM ONLY foreman_execution.promotion_intents AS intent
          JOIN ONLY control.task_submission_envelopes AS submission
            ON submission.stream_id = intent.intake_stream_id
           AND submission.event_digest = intent.intake_event_digest
           AND pg_catalog.decode(submission.task_ref, 'hex') = intent.task_ref
           AND submission.project_id = intent.project_id
           AND submission.project_snapshot_id = intent.project_snapshot_id
           AND submission.project_authority_receipt_digest =
               intent.project_authority_receipt_digest
           AND submission.schema_version = 'lattice.task-ledger.task-submission/1.0'
           AND submission.task_subject_kind = 'GENERAL_TASK_INTAKE'
           AND submission.admission_action = 'GENERAL_TASK_INTAKE_V1'
          JOIN ONLY control.task_ledger_events AS intake_event
            ON intake_event.stream_id = submission.stream_id
           AND intake_event.sequence = submission.event_sequence
           AND intake_event.event_digest = submission.event_digest
           AND intake_event.event_kind = 'TASK_CREATED'
           AND intake_event.action_id = 'GENERAL_TASK_INTAKE_V1'
         WHERE intent.task_ref = p_task_ref) <> 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_PROMOTION_INTENT_LINEAGE_MISMATCH';
    END IF;
    SELECT * INTO v_existing FROM ONLY foreman_execution.task_promotions
     WHERE task_ref = p_task_ref;
    IF FOUND THEN
        IF v_existing.project_id IS DISTINCT FROM p_project_id
           OR v_existing.project_snapshot_id IS DISTINCT FROM p_project_snapshot_id
           OR v_existing.intake_stream_id IS DISTINCT FROM p_intake_stream_id
           OR v_existing.intake_event_digest IS DISTINCT FROM p_intake_event_digest
           OR v_existing.project_authority_receipt_digest IS DISTINCT FROM p_project_authority_receipt_digest
           OR v_existing.successor_stream_id IS DISTINCT FROM p_successor_stream_id
           OR v_existing.successor_task_created_event_digest IS DISTINCT FROM p_successor_task_created_event_digest
           OR v_existing.task_spec_digest IS DISTINCT FROM p_task_spec_digest
           OR v_existing.approval_subject_digest IS DISTINCT FROM p_approval_subject_digest
           OR v_existing.budget_digest IS DISTINCT FROM p_budget_digest
           OR v_existing.global_active_limit IS DISTINCT FROM p_global_active_limit
           OR v_existing.per_task_active_limit IS DISTINCT FROM p_per_task_active_limit
           OR v_existing.repair_retry_limit IS DISTINCT FROM p_repair_retry_limit
           OR v_existing.max_duration_seconds IS DISTINCT FROM p_max_duration_seconds
           OR v_existing.max_total_tokens IS DISTINCT FROM p_max_total_tokens
           OR v_existing.max_model_calls IS DISTINCT FROM p_max_model_calls
           OR v_existing.external_cost_status IS DISTINCT FROM p_external_cost_status
           OR v_existing.external_cost_limit_micros IS DISTINCT FROM p_external_cost_limit_micros
           OR v_existing.deadline_at IS DISTINCT FROM p_deadline_at
           OR v_existing.budget_pointer IS DISTINCT FROM p_budget_pointer
           OR v_existing.verification_policy_digest IS DISTINCT FROM p_verification_policy_digest
           OR v_existing.binding_digest IS DISTINCT FROM p_binding_digest
           OR v_existing.base_ref IS DISTINCT FROM p_base_ref
           OR v_existing.base_commit IS DISTINCT FROM p_base_commit
           OR v_existing.ledger_event_digest IS DISTINCT FROM p_event_digest THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PROMOTION_SUBSTITUTION';
        END IF;
        PERFORM foreman_execution.assert_exact_child_event_v1(
            'TASK_PROMOTION', p_task_ref, p_stream_id, p_event_sequence,
            p_event_digest, p_command_id, p_request_digest, p_payload_digest,
            'RECORD_TASK_EXECUTION_BINDING_V1'
        );
        RETURN 'EXACT_REPLAY';
    END IF;
    IF p_stream_id IS DISTINCT FROM p_successor_stream_id
       OR p_payload_digest IS DISTINCT FROM p_binding_digest
       OR p_budget_pointer IS DISTINCT FROM 'budget:sha256:' || pg_catalog.encode(p_budget_digest, 'hex')
       OR (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.promotion_intents AS intent
            WHERE intent.task_ref = p_task_ref
              AND intent.project_id = p_project_id
              AND intent.project_snapshot_id = p_project_snapshot_id
              AND intent.intake_stream_id = p_intake_stream_id
              AND intent.intake_event_digest = p_intake_event_digest
              AND intent.project_authority_receipt_digest = p_project_authority_receipt_digest
              AND intent.successor_stream_id = p_successor_stream_id
              AND intent.task_spec_digest = p_task_spec_digest
              AND intent.approval_subject_digest = p_approval_subject_digest
              AND intent.budget_digest = p_budget_digest
              AND intent.global_active_limit = p_global_active_limit
              AND intent.per_task_active_limit = p_per_task_active_limit
              AND intent.repair_retry_limit = p_repair_retry_limit
              AND intent.max_duration_seconds = p_max_duration_seconds
              AND intent.max_total_tokens = p_max_total_tokens
              AND intent.max_model_calls = p_max_model_calls
              AND intent.external_cost_status = p_external_cost_status
              AND intent.external_cost_limit_micros IS NOT DISTINCT FROM p_external_cost_limit_micros
              AND intent.deadline_at = p_deadline_at
              AND intent.budget_pointer = p_budget_pointer
              AND intent.verification_policy_digest = p_verification_policy_digest
              AND intent.base_ref = p_base_ref
              AND intent.base_commit = p_base_commit
              AND intent.source_clean) <> 1
       OR (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_streams AS s
            WHERE s.stream_id = p_successor_stream_id
              AND s.project_id = p_project_id
              AND s.project_snapshot_id = p_project_snapshot_id
              AND s.task_spec_digest = p_task_spec_digest
              AND s.runtime = 'LIVE') <> 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PROMOTION_LINEAGE_MISMATCH';
    END IF;
    PERFORM foreman_execution.insert_child_event_v1(
        'TASK_PROMOTION', p_task_ref, p_stream_id, p_event_sequence,
        p_event_digest, p_command_id, p_request_digest, p_payload_digest,
        'RECORD_TASK_EXECUTION_BINDING_V1'
    );
    INSERT INTO foreman_execution.task_promotions VALUES (
        p_task_ref, p_project_id, p_project_snapshot_id, p_intake_stream_id,
        p_intake_event_digest, p_project_authority_receipt_digest,
        p_successor_stream_id, p_successor_task_created_event_digest,
        p_task_spec_digest, p_approval_subject_digest, p_budget_digest,
        p_global_active_limit, p_per_task_active_limit, p_repair_retry_limit,
        p_max_duration_seconds, p_max_total_tokens, p_max_model_calls,
        p_external_cost_status, p_external_cost_limit_micros, p_deadline_at,
        p_budget_pointer,
        p_verification_policy_digest, p_binding_digest, p_base_ref,
        p_base_commit, p_event_digest
    );
    RETURN 'INSERTED';
END;
$$;

CREATE FUNCTION foreman_execution.reserve_worker_attempt_v1(
    p_task_ref bytea, p_successor_stream_id bytea, p_task_spec_digest bytea,
    p_binding_digest bytea, p_budget_digest bytea, p_attempt_id text,
    p_attempt_number smallint, p_foreman_generation bigint, p_model text,
    p_reasoning text, p_writer_fence bigint, p_foreman_checkpoint_digest bytea,
    p_approval_receipt_digest bytea, p_packet_digest bytea,
    p_execution_environment_ref text, p_worktree_digest bytea, p_base_commit_digest bytea,
    p_model_reason text, p_model_reason_digest bytea, p_claimed_at text, p_payload_digest bytea,
    p_max_attempts smallint, p_stream_id bytea, p_event_sequence numeric,
    p_event_digest bytea, p_command_id text, p_request_digest bytea
) RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_existing foreman_execution.worker_attempts%ROWTYPE;
    v_pending foreman_execution.pending_worker_claims%ROWTYPE;
    v_previous foreman_execution.worker_attempts%ROWTYPE;
    v_max smallint;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);
    IF p_execution_environment_ref !~ '^execution-environment:sha256:[a-f0-9]{64}$'
       OR p_execution_environment_ref =
          'execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000000' THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_REF_REJECTED';
    END IF;
    IF p_max_attempts NOT BETWEEN 1 AND 3 OR p_attempt_number > p_max_attempts THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_RETRY_BUDGET_EXHAUSTED';
    END IF;
    IF (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.task_promotions AS p
         WHERE p.task_ref = p_task_ref AND p.successor_stream_id = p_successor_stream_id
           AND p.task_spec_digest = p_task_spec_digest AND p.binding_digest = p_binding_digest
           AND p.budget_digest = p_budget_digest
           AND p.global_active_limit = 4 AND p.per_task_active_limit = 1
           AND p.repair_retry_limit + 1 = p_max_attempts) <> 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_ATTEMPT_BINDING_MISMATCH';
    END IF;
    SELECT * INTO v_existing FROM ONLY foreman_execution.worker_attempts
     WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number;
    IF FOUND THEN
        IF v_existing.successor_stream_id IS DISTINCT FROM p_successor_stream_id
           OR v_existing.task_spec_digest IS DISTINCT FROM p_task_spec_digest
           OR v_existing.binding_digest IS DISTINCT FROM p_binding_digest
           OR v_existing.budget_digest IS DISTINCT FROM p_budget_digest
           OR v_existing.attempt_id IS DISTINCT FROM p_attempt_id
           OR v_existing.foreman_generation IS DISTINCT FROM p_foreman_generation
           OR v_existing.model IS DISTINCT FROM p_model
           OR v_existing.reasoning IS DISTINCT FROM p_reasoning
           OR v_existing.writer_fence IS DISTINCT FROM p_writer_fence
           OR v_existing.foreman_checkpoint_digest IS DISTINCT FROM p_foreman_checkpoint_digest
           OR v_existing.approval_receipt_digest IS DISTINCT FROM p_approval_receipt_digest
           OR v_existing.packet_digest IS DISTINCT FROM p_packet_digest
           OR v_existing.execution_environment_ref IS DISTINCT FROM p_execution_environment_ref
           OR v_existing.worktree_digest IS DISTINCT FROM p_worktree_digest
           OR v_existing.base_commit_digest IS DISTINCT FROM p_base_commit_digest
           OR v_existing.model_reason IS DISTINCT FROM p_model_reason
           OR v_existing.model_reason_digest IS DISTINCT FROM p_model_reason_digest
           OR v_existing.claimed_at IS DISTINCT FROM p_claimed_at
           OR v_existing.payload_digest IS DISTINCT FROM p_payload_digest
           OR v_existing.ledger_event_digest IS DISTINCT FROM p_event_digest
           OR EXISTS (SELECT 1 FROM ONLY foreman_execution.pending_worker_claims AS pending
                       WHERE pending.task_ref = p_task_ref) THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_ATTEMPT_SUBSTITUTION';
        END IF;
        PERFORM foreman_execution.assert_exact_child_event_v1(
            'WORKER_ATTEMPT', p_task_ref, p_stream_id, p_event_sequence,
            p_event_digest, p_command_id, p_request_digest, p_payload_digest,
            'DISPATCH_WORKER_ATTEMPT_V1'
        );
        PERFORM pg_catalog.count(*)
          FROM foreman_execution.read_execution_environment_rows_v1(p_task_ref);
        RETURN 'EXACT_REPLAY';
    END IF;
    SELECT * INTO v_pending FROM ONLY foreman_execution.pending_worker_claims
     WHERE task_ref = p_task_ref;
    IF FOUND THEN
        IF v_pending.attempt_number IS DISTINCT FROM p_attempt_number
           OR v_pending.successor_stream_id IS DISTINCT FROM p_successor_stream_id
           OR v_pending.task_spec_digest IS DISTINCT FROM p_task_spec_digest
           OR v_pending.binding_digest IS DISTINCT FROM p_binding_digest
           OR v_pending.budget_digest IS DISTINCT FROM p_budget_digest
           OR v_pending.attempt_id IS DISTINCT FROM p_attempt_id
           OR v_pending.foreman_generation IS DISTINCT FROM p_foreman_generation
           OR v_pending.model IS DISTINCT FROM p_model
           OR v_pending.reasoning IS DISTINCT FROM p_reasoning
           OR v_pending.writer_fence IS DISTINCT FROM p_writer_fence
           OR v_pending.foreman_checkpoint_digest IS DISTINCT FROM p_foreman_checkpoint_digest
           OR v_pending.approval_receipt_digest IS DISTINCT FROM p_approval_receipt_digest
           OR v_pending.packet_digest IS DISTINCT FROM p_packet_digest
           OR v_pending.execution_environment_ref IS DISTINCT FROM p_execution_environment_ref
           OR v_pending.worktree_digest IS DISTINCT FROM p_worktree_digest
           OR v_pending.base_commit_digest IS DISTINCT FROM p_base_commit_digest
           OR v_pending.model_reason IS DISTINCT FROM p_model_reason
           OR v_pending.model_reason_digest IS DISTINCT FROM p_model_reason_digest
           OR v_pending.claimed_at IS DISTINCT FROM p_claimed_at
           OR v_pending.payload_digest IS DISTINCT FROM p_payload_digest
           OR v_pending.max_attempts IS DISTINCT FROM p_max_attempts
           OR v_pending.ledger_event_digest IS DISTINCT FROM p_event_digest THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PENDING_CLAIM_SUBSTITUTION';
        END IF;
        PERFORM foreman_execution.assert_exact_child_event_v1(
            'WORKER_ATTEMPT', p_task_ref, p_stream_id, p_event_sequence,
            p_event_digest, p_command_id, p_request_digest, p_payload_digest,
            'DISPATCH_WORKER_ATTEMPT_V1'
        );
        PERFORM pg_catalog.count(*)
          FROM foreman_execution.read_execution_environment_rows_v1(p_task_ref);
        RETURN 'EXACT_REPLAY';
    END IF;
    IF p_model NOT IN ('gpt-5.6-luna','gpt-5.6-terra','gpt-5.6-sol') THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_MODEL_NOT_ALLOWED';
    END IF;
    IF NOT (
        (p_model = 'gpt-5.6-luna' AND p_model_reason = 'BOUNDED_STATE_EVIDENCE_DOCUMENTATION')
        OR (p_model = 'gpt-5.6-terra' AND p_model_reason = 'ROUTINE_ENGINEERING')
        OR (
            p_model = 'gpt-5.6-sol'
            AND p_model_reason IN ('P0','ARCHITECTURE','SECURITY','HIGH_RISK','TERRA_INSUFFICIENT')
        )
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_MODEL_REASON_NOT_ALLOWED';
    END IF;
    SELECT COALESCE(pg_catalog.max(attempt_number), 0)::smallint INTO v_max
      FROM ONLY foreman_execution.worker_attempts WHERE task_ref = p_task_ref;
    IF p_attempt_number <> v_max + 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_ATTEMPT_SEQUENCE_MISMATCH';
    END IF;
    IF p_attempt_number > 1 THEN
        SELECT * INTO v_previous FROM ONLY foreman_execution.worker_attempts
         WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number - 1;
        IF NOT FOUND OR p_writer_fence <= v_previous.writer_fence
           OR p_foreman_generation < v_previous.foreman_generation
           OR NOT (
               EXISTS (
                   SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
                    WHERE o.task_ref = p_task_ref AND o.attempt_number = p_attempt_number - 1
                      AND o.observation_kind IN ('PRESTART_TERMINAL_FAILED','TERMINAL_COMPLETED','TERMINAL_FAILED','TERMINAL_INTERRUPTED')
               )
               OR EXISTS (
                   SELECT 1 FROM ONLY foreman_execution.attempt_closures AS closure
                    WHERE closure.task_ref = p_task_ref
                      AND closure.attempt_number = p_attempt_number - 1
               )
           ) THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_RETRY_PREDECESSOR_NOT_TERMINAL';
        END IF;
    END IF;
    IF p_stream_id IS DISTINCT FROM p_successor_stream_id THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_ATTEMPT_STREAM_MISMATCH';
    END IF;
    PERFORM foreman_execution.insert_child_event_v1(
        'WORKER_ATTEMPT', p_task_ref, p_stream_id, p_event_sequence,
        p_event_digest, p_command_id, p_request_digest, p_payload_digest,
        'DISPATCH_WORKER_ATTEMPT_V1'
    );
    INSERT INTO foreman_execution.pending_worker_claims (
        task_ref, attempt_number, attempt_id, successor_stream_id,
        task_spec_digest, binding_digest, budget_digest, foreman_generation,
        model, reasoning, writer_fence, foreman_checkpoint_digest,
        approval_receipt_digest, packet_digest, execution_environment_ref, worktree_digest,
        base_commit_digest, model_reason, model_reason_digest, claimed_at, payload_digest,
        max_attempts, ledger_event_digest
    ) VALUES (
        p_task_ref, p_attempt_number, p_attempt_id, p_successor_stream_id,
        p_task_spec_digest, p_binding_digest, p_budget_digest,
        p_foreman_generation, p_model, p_reasoning, p_writer_fence,
        p_foreman_checkpoint_digest, p_approval_receipt_digest,
        p_packet_digest, p_execution_environment_ref, p_worktree_digest, p_base_commit_digest,
        p_model_reason, p_model_reason_digest, p_claimed_at, p_payload_digest,
        p_max_attempts, p_event_digest
    );
    RETURN 'RESERVED';
END;
$$;

CREATE FUNCTION foreman_execution.canonical_json_v1(p_value jsonb)
RETURNS text
LANGUAGE plpgsql
IMMUTABLE
STRICT
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_result text;
BEGIN
    CASE pg_catalog.jsonb_typeof(p_value)
        WHEN 'object' THEN
            SELECT '{' || COALESCE(
                pg_catalog.string_agg(
                    pg_catalog.to_json(entry.key)::text || ':' ||
                    foreman_execution.canonical_json_v1(entry.value),
                    ',' ORDER BY entry.key
                ),
                ''
            ) || '}'
              INTO v_result
              FROM pg_catalog.jsonb_each(p_value) AS entry(key, value);
        WHEN 'array' THEN
            SELECT '[' || COALESCE(
                pg_catalog.string_agg(
                    foreman_execution.canonical_json_v1(entry.value),
                    ',' ORDER BY entry.ordinality
                ),
                ''
            ) || ']'
              INTO v_result
              FROM pg_catalog.jsonb_array_elements(p_value)
                   WITH ORDINALITY AS entry(value, ordinality);
        ELSE
            v_result := p_value::text;
    END CASE;
    RETURN v_result;
END;
$$;

CREATE FUNCTION foreman_execution.record_execution_environment_v1(
    p_task_ref bytea, p_attempt_number smallint, p_attempt_id text,
    p_packet_digest bytea, p_descriptor_json text, p_environment_ref text
) RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_descriptor jsonb;
    v_distribution jsonb;
    v_gateway jsonb;
    v_linux jsonb;
    v_credential jsonb;
    v_fence jsonb;
    v_toolchain jsonb;
    v_immutable_snapshot jsonb;
    v_sandbox_policy jsonb;
    v_sandbox_policy_template jsonb;
    v_privilege_boundary jsonb;
    v_mapping jsonb;
    v_canonical_descriptor text;
    v_canonical_subject text;
    v_execution_domain_digest bytea;
    v_expected_ref text;
    v_expected_nested_ref text;
    v_linux_home text;
    v_anchor_count bigint;
    v_anchor_id text;
    v_anchor_packet bytea;
    v_anchor_environment_ref text;
    v_active_anchor_count bigint;
    v_node_version_match text[];
    v_descriptor_scan_nodes bigint;
    v_descriptor_scan_depth_exceeded boolean;
    v_descriptor_scan_secret boolean;
    v_existing foreman_execution.execution_environments%ROWTYPE;
BEGIN
    IF p_attempt_number NOT BETWEEN 1 AND 3
       OR p_attempt_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR pg_catalog.octet_length(p_task_ref) <> 32
       OR pg_catalog.octet_length(p_packet_digest) <> 32
       OR p_task_ref = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_packet_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR pg_catalog.octet_length(p_descriptor_json) NOT BETWEEN 1 AND 16384
       OR p_environment_ref !~ '^execution-environment:sha256:[a-f0-9]{64}$' THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED';
    END IF;
    BEGIN
        v_descriptor := p_descriptor_json::jsonb;
    EXCEPTION WHEN OTHERS THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED';
    END;
    WITH RECURSIVE descriptor_string_nodes(node_value, node_depth) AS (
        SELECT v_descriptor, 0
        UNION ALL
        SELECT child.node_value, parent.node_depth + 1
          FROM descriptor_string_nodes AS parent
          CROSS JOIN LATERAL (
              SELECT array_value AS node_value
                FROM pg_catalog.jsonb_array_elements(
                    CASE WHEN pg_catalog.jsonb_typeof(parent.node_value) = 'array'
                         THEN parent.node_value ELSE '[]'::jsonb END
                ) AS array_entry(array_value)
              UNION ALL
              SELECT object_value AS node_value
                FROM pg_catalog.jsonb_each(
                    CASE WHEN pg_catalog.jsonb_typeof(parent.node_value) = 'object'
                         THEN parent.node_value ELSE '{}'::jsonb END
                ) AS object_entry(object_key, object_value)
          ) AS child
         WHERE parent.node_depth < 16
    ), descriptor_string_leaves(string_value) AS (
        SELECT node_value #>> '{}'
          FROM descriptor_string_nodes
         WHERE pg_catalog.jsonb_typeof(node_value) = 'string'
    )
    SELECT pg_catalog.count(*),
           COALESCE(pg_catalog.bool_or(
               node_depth >= 16
               AND pg_catalog.jsonb_typeof(node_value) IN ('array', 'object')
           ), false),
           COALESCE((
               SELECT pg_catalog.bool_or(
                   pg_catalog.octet_length(string_value) > 4096
                   OR string_value ~* 'bearer[[:space:]]'
                   OR string_value ~* '-----begin[^\r\n]*private key-----'
                   OR string_value ~* '://[^/?#[:space:]]*@'
                   OR string_value ~* '(ghp_|gho_|ghu_|ghs_|ghr_|github_pat_|glpat-|npm_|pypi-|xoxa-|xoxb-|xoxp-|xoxr-|xoxs-)'
                   OR string_value ~* '(^|[^[:alnum:]])sk-'
                   OR string_value ~* '(^|[^[:alnum:]_-])(password|passphrase|passwd|pwd|token|access[ _-]token|refresh[ _-]token|id[ _-]token|session[ _-]token|api[ _-]?key|apikey|client[ _-]secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
                   OR string_value ~ '(^|[^[:alnum:]])(AKIA|ASIA)[0-9A-Z]{16}([^[:alnum:]]|$)'
               )
                 FROM descriptor_string_leaves
           ), false)
      INTO v_descriptor_scan_nodes, v_descriptor_scan_depth_exceeded,
           v_descriptor_scan_secret
      FROM descriptor_string_nodes;
    IF v_descriptor_scan_nodes > 512
       OR v_descriptor_scan_depth_exceeded
       OR v_descriptor_scan_secret THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED';
    END IF;
    IF pg_catalog.jsonb_typeof(v_descriptor) <> 'object'
       OR (SELECT pg_catalog.count(*) FROM pg_catalog.jsonb_object_keys(v_descriptor)) <> 14
       OR NOT (v_descriptor ?& ARRAY[
            'schema','kind','distribution','distribution_identity','gateway','linux',
            'credential_authority','process_fence','verification_toolchain',
            'immutable_snapshot','sandbox_policy','privilege_boundary',
            'path_mapping','identity_digest'
       ])
       OR v_descriptor->>'schema' <> 'lattice.execution-environment.wsl2-linux/1.1'
       OR v_descriptor->>'kind' <> 'WSL2_LINUX'
       OR v_descriptor->>'identity_digest' <> p_environment_ref
       OR v_descriptor->>'distribution' !~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$' THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED';
    END IF;
    v_distribution := v_descriptor->'distribution_identity';
    v_gateway := v_descriptor->'gateway';
    v_linux := v_descriptor->'linux';
    v_credential := v_descriptor->'credential_authority';
    v_fence := v_descriptor->'process_fence';
    v_toolchain := v_descriptor->'verification_toolchain';
    v_immutable_snapshot := v_descriptor->'immutable_snapshot';
    v_sandbox_policy := v_descriptor->'sandbox_policy';
    v_privilege_boundary := v_descriptor->'privilege_boundary';
    v_mapping := v_descriptor->'path_mapping';
    IF pg_catalog.jsonb_typeof(v_distribution) <> 'object'
       OR (SELECT pg_catalog.count(*) FROM pg_catalog.jsonb_object_keys(v_distribution)) <> 6
       OR NOT (v_distribution ?& ARRAY[
            'os_id','os_version_id','os_version_codename','os_release_sha256',
            'kernel_release','identity_digest'
       ])
       OR v_distribution->>'os_id' !~ '^[a-z0-9._-]+$'
       OR v_distribution->>'os_version_id' !~ '^[0-9]+(\.[0-9]+)*$'
       OR v_distribution->>'os_version_codename' !~ '^[a-z0-9._-]+$'
       OR v_distribution->>'os_release_sha256' !~ '^[a-f0-9]{64}$'
       OR v_distribution->>'kernel_release' !~ 'microsoft-standard-WSL2$'
       OR v_distribution->>'identity_digest' !~ '^wsl2-distribution:sha256:[a-f0-9]{64}$'
       OR pg_catalog.jsonb_typeof(v_gateway) <> 'object'
       OR (SELECT pg_catalog.count(*) FROM pg_catalog.jsonb_object_keys(v_gateway)) <> 3
       OR NOT (v_gateway ?& ARRAY['windows_path','version','sha256'])
       OR v_gateway->>'windows_path' !~* '^[A-Z]:\\.*\\wsl\.exe$'
       OR v_gateway->>'version' !~ '^[0-9]{1,6}(\.[0-9]{1,6}){2,3}$'
       OR v_gateway->>'sha256' !~ '^[a-f0-9]{64}$' THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED';
    END IF;
    IF pg_catalog.jsonb_typeof(v_linux) <> 'object'
       OR (SELECT pg_catalog.count(*) FROM pg_catalog.jsonb_object_keys(v_linux)) <> 25
       OR NOT (v_linux ?& ARRAY[
            'launcher_path','launcher_version','launcher_sha256','node_path','node_version',
            'node_sha256','git_path','git_version','git_sha256','supervisor_path',
            'supervisor_sha256','codex_home','config_digest','cwd','repository_head',
            'repository_identity','dbus_run_session_path','dbus_run_session_sha256',
            'setsid_path','setsid_sha256','keyring_daemon_path','keyring_daemon_sha256',
            'keyring_library_path','keyring_library_manifest_digest','xdg_runtime_dir'
       ])
       OR v_linux->>'config_digest' !~ '^codex-config:sha256:[a-f0-9]{64}$'
       OR v_linux->>'repository_identity' !~ '^repository:sha256:[a-f0-9]{64}$'
       OR v_linux->>'repository_head' !~ '^[a-f0-9]{40}$'
       OR v_linux->>'launcher_version'
            !~ '^codex-cli [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}$'
       OR v_linux->>'node_version' !~ '^v[0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}$'
       OR v_linux->>'git_version'
            !~ '^git version [0-9]{1,6}(\.[0-9]{1,6}){2}$'
       OR v_linux->>'codex_home' !~ '^/home/'
       OR v_linux->>'cwd' !~ '^/home/'
       OR v_linux->>'keyring_library_path' !~ '^/home/'
       OR v_linux->>'keyring_library_manifest_digest'
            !~ '^keyring-library-manifest:sha256:[a-f0-9]{64}$'
       OR v_linux->>'xdg_runtime_dir' !~ '^/run/user/[0-9]+$'
       OR EXISTS (
            SELECT 1 FROM (VALUES
                (v_linux->>'launcher_sha256'), (v_linux->>'node_sha256'),
                (v_linux->>'git_sha256'), (v_linux->>'supervisor_sha256'),
                (v_linux->>'dbus_run_session_sha256'), (v_linux->>'setsid_sha256'),
                (v_linux->>'keyring_daemon_sha256')
            ) AS digest(value) WHERE digest.value !~ '^[a-f0-9]{64}$'
       )
       OR EXISTS (
            SELECT 1 FROM (VALUES
                (v_linux->>'launcher_path'), (v_linux->>'node_path'),
                (v_linux->>'git_path'), (v_linux->>'supervisor_path'),
                (v_linux->>'codex_home'), (v_linux->>'cwd'),
                (v_linux->>'dbus_run_session_path'), (v_linux->>'setsid_path'),
                (v_linux->>'keyring_daemon_path'), (v_linux->>'keyring_library_path'),
                (v_linux->>'xdg_runtime_dir')
            ) AS candidate_path(value)
            WHERE candidate_path.value !~ '^/' OR candidate_path.value ~ '(^|/)\.\.?(/|$)'
               OR pg_catalog.strpos(candidate_path.value, '//') > 0
               OR pg_catalog.right(candidate_path.value, 1) = '/'
               OR candidate_path.value ~ '^/mnt/'
               OR candidate_path.value !~ '^/[A-Za-z0-9._~/-]+$'
       ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED';
    END IF;
    IF pg_catalog.jsonb_typeof(v_credential) <> 'object'
       OR (SELECT pg_catalog.count(*) FROM pg_catalog.jsonb_object_keys(v_credential)) <> 2
       OR NOT (v_credential ?& ARRAY['kind','authority_digest'])
       OR v_credential->>'kind' <> 'LINUX_KEYRING'
       OR v_credential->>'authority_digest' !~ '^wsl2-credential-authority:sha256:[a-f0-9]{64}$'
       OR pg_catalog.jsonb_typeof(v_fence) <> 'object'
       OR (SELECT pg_catalog.count(*) FROM pg_catalog.jsonb_object_keys(v_fence)) <> 15
       OR NOT (v_fence ?& ARRAY[
            'schema','kind','systemd_run_path','systemd_run_version','systemd_run_sha256',
            'systemctl_path','systemctl_version','systemctl_sha256','cgroup_mount',
            'user_runtime_dir','unit_prefix','supervisor_bootstrap_node',
            'immutable_probe_lsattr','noninteractive_root_probe','identity_digest'
       ])
       OR v_fence->>'schema' <> 'lattice.wsl2-cgroup-v2-fence/1.0'
       OR v_fence->>'kind' <> 'SYSTEMD_USER_SERVICE_CGROUP_V2'
       OR EXISTS (
            SELECT 1 FROM (VALUES
                (v_fence->>'systemd_run_path'), (v_fence->>'systemctl_path')
            ) AS fence_path(value)
            WHERE fence_path.value !~ '^/' OR fence_path.value ~ '(^|/)\.\.?(/|$)'
               OR pg_catalog.strpos(fence_path.value, '//') > 0
               OR pg_catalog.right(fence_path.value, 1) = '/'
               OR fence_path.value ~ '^/mnt/'
               OR fence_path.value !~ '^/[A-Za-z0-9._~/-]+$'
       )
       OR v_fence->>'systemd_run_sha256' !~ '^[a-f0-9]{64}$'
       OR v_fence->>'systemctl_sha256' !~ '^[a-f0-9]{64}$'
       OR v_fence->>'systemd_run_version'
            !~ '^systemd [0-9]{2,4}( \([A-Za-z0-9.+:~_-]+\))?$'
       OR v_fence->>'systemctl_version'
            !~ '^systemd [0-9]{2,4}( \([A-Za-z0-9.+:~_-]+\))?$'
       OR pg_catalog.jsonb_typeof(v_fence->'supervisor_bootstrap_node') <> 'object'
       OR (SELECT pg_catalog.count(*)
             FROM pg_catalog.jsonb_object_keys(v_fence->'supervisor_bootstrap_node')) <> 3
       OR NOT (v_fence->'supervisor_bootstrap_node' ?& ARRAY['path','version','sha256'])
       OR v_fence->'supervisor_bootstrap_node'->>'path' IS DISTINCT FROM '/usr/bin/node'
       OR v_fence->'supervisor_bootstrap_node'->>'version'
            !~ '^v[0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}$'
       OR v_fence->'supervisor_bootstrap_node'->>'sha256' !~ '^[a-f0-9]{64}$'
       OR pg_catalog.jsonb_typeof(v_fence->'immutable_probe_lsattr') <> 'object'
       OR (SELECT pg_catalog.count(*)
             FROM pg_catalog.jsonb_object_keys(v_fence->'immutable_probe_lsattr')) <> 3
       OR NOT (v_fence->'immutable_probe_lsattr' ?& ARRAY['path','version','sha256'])
       OR v_fence->'immutable_probe_lsattr'->>'path' IS DISTINCT FROM '/usr/bin/lsattr'
       OR v_fence->'immutable_probe_lsattr'->>'version'
            !~ '^lsattr [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6} \([0-9]{1,2}-[A-Za-z]{3}-[0-9]{4}\)$'
       OR v_fence->'immutable_probe_lsattr'->>'sha256' !~ '^[a-f0-9]{64}$'
       OR pg_catalog.jsonb_typeof(v_fence->'noninteractive_root_probe') <> 'object'
       OR (SELECT pg_catalog.count(*)
             FROM pg_catalog.jsonb_object_keys(v_fence->'noninteractive_root_probe')) <> 3
       OR NOT (v_fence->'noninteractive_root_probe' ?& ARRAY['path','version','sha256'])
       OR v_fence->'noninteractive_root_probe'->>'path' IS DISTINCT FROM '/usr/bin/sudo'
       OR v_fence->'noninteractive_root_probe'->>'version'
            !~ '^(Sudo version [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}(p[0-9]{1,64})?|sudo-rs [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}-[0-9A-Za-z.+~_-]{1,64})$'
       OR v_fence->'noninteractive_root_probe'->>'sha256' !~ '^[a-f0-9]{64}$'
       OR v_fence->>'cgroup_mount' <> '/sys/fs/cgroup'
       OR v_fence->>'user_runtime_dir' IS DISTINCT FROM v_linux->>'xdg_runtime_dir'
       OR v_fence->>'unit_prefix' !~ '^lattice-wsl2-[a-f0-9]{16}$'
       OR v_fence->>'identity_digest' !~ '^wsl2-process-fence-authority:sha256:[a-f0-9]{64}$' THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED';
    END IF;
    IF pg_catalog.jsonb_typeof(v_toolchain) <> 'object'
       OR (SELECT pg_catalog.count(*) FROM pg_catalog.jsonb_object_keys(v_toolchain)) <> 18
       OR NOT (v_toolchain ?& ARRAY[
            'schema','task_ref','task_root','isolation_root','owner_uid','home_dir','temp_dir',
            'npm_cache','cargo_home','cargo_target_dir','cargo_host','npm','cargo','rustc',
            'rustdoc','sandbox','sandbox_helper','identity_digest'
       ])
       OR v_toolchain->>'schema' <> 'lattice.wsl2-verification-toolchain/1.0'
       OR v_toolchain->>'task_ref' IS DISTINCT FROM pg_catalog.encode(p_task_ref, 'hex')
       OR v_toolchain->>'cargo_host' !~ '^[A-Za-z0-9._-]+$'
       OR pg_catalog.jsonb_typeof(v_toolchain->'owner_uid') <> 'number'
       OR (v_toolchain->>'owner_uid')::bigint <= 0
       OR EXISTS (
            SELECT 1 FROM (VALUES
                (v_toolchain->>'task_root'), (v_toolchain->>'isolation_root'),
                (v_toolchain->>'home_dir'), (v_toolchain->>'temp_dir'),
                (v_toolchain->>'npm_cache'), (v_toolchain->>'cargo_home'),
                (v_toolchain->>'cargo_target_dir'),
                (v_toolchain->'npm'->>'path'), (v_toolchain->'cargo'->>'path'),
                (v_toolchain->'rustc'->>'path'), (v_toolchain->'rustdoc'->>'path'),
                (v_toolchain->'sandbox'->>'path')
            ) AS toolchain_path(value)
            WHERE toolchain_path.value IS NULL
               OR toolchain_path.value !~ '^/'
               OR toolchain_path.value ~ '(^|/)\.\.?(/|$)'
               OR pg_catalog.strpos(toolchain_path.value, '//') > 0
               OR pg_catalog.right(toolchain_path.value, 1) = '/'
               OR toolchain_path.value ~ '^/mnt/'
               OR toolchain_path.value !~ '^/[A-Za-z0-9._~/-]+$'
       )
       OR v_toolchain->>'task_root' !~ '^/home/'
       OR NOT pg_catalog.starts_with(
            v_toolchain->>'isolation_root', v_toolchain->>'task_root' || '/'
       )
       OR NOT pg_catalog.starts_with(
            v_linux->>'cwd', v_toolchain->>'task_root' || '/managed-worktrees/'
       )
       OR v_linux->>'codex_home' IS DISTINCT FROM v_toolchain->>'task_root' || '/codex-home'
       OR NOT pg_catalog.starts_with(
            v_linux->>'launcher_path', v_toolchain->>'task_root' || '/'
       )
       OR NOT pg_catalog.starts_with(
            v_linux->>'node_path', v_toolchain->>'task_root' || '/'
       )
       OR NOT pg_catalog.starts_with(
            v_linux->>'supervisor_path', v_toolchain->>'task_root' || '/'
       )
       OR EXISTS (
            SELECT 1 FROM (VALUES
                (v_toolchain->>'home_dir'), (v_toolchain->>'temp_dir'),
                (v_toolchain->>'npm_cache'), (v_toolchain->>'cargo_home'),
                (v_toolchain->>'cargo_target_dir')
            ) AS isolated(value)
            WHERE NOT pg_catalog.starts_with(
                      isolated.value, v_toolchain->>'isolation_root' || '/'
                  )
               OR isolated.value !~ '^/[A-Za-z0-9._~/-]+$'
       )
       OR EXISTS (
            SELECT 1
              FROM (VALUES
                  (v_toolchain->'npm'), (v_toolchain->'cargo'),
                  (v_toolchain->'rustc'), (v_toolchain->'rustdoc'),
                  (v_toolchain->'sandbox')
              ) AS tool(identity)
             WHERE pg_catalog.jsonb_typeof(tool.identity) <> 'object'
                OR (SELECT pg_catalog.count(*)
                      FROM pg_catalog.jsonb_object_keys(tool.identity)) <> 3
                OR NOT (tool.identity ?& ARRAY['path','version','sha256'])
                OR NOT pg_catalog.starts_with(
                    tool.identity->>'path', v_toolchain->>'task_root' || '/'
                )
                OR pg_catalog.length(tool.identity->>'version')
                    NOT BETWEEN 1 AND 128
                OR tool.identity->>'sha256' !~ '^[a-f0-9]{64}$'
       )
       OR v_toolchain->'sandbox'->>'path' IS DISTINCT FROM v_linux->>'launcher_path'
       OR v_toolchain->'sandbox'->>'version' IS DISTINCT FROM v_linux->>'launcher_version'
       OR v_toolchain->'sandbox'->>'sha256' IS DISTINCT FROM v_linux->>'launcher_sha256'
       OR pg_catalog.jsonb_typeof(v_toolchain->'sandbox_helper') <> 'object'
       OR (SELECT pg_catalog.count(*)
             FROM pg_catalog.jsonb_object_keys(v_toolchain->'sandbox_helper')) <> 3
       OR NOT (v_toolchain->'sandbox_helper' ?& ARRAY['path','version','sha256'])
       OR v_toolchain->'sandbox_helper'->>'path' IS DISTINCT FROM '/usr/bin/bwrap'
       OR v_toolchain->'npm'->>'version'
            !~ '^[0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}$'
       OR v_toolchain->'cargo'->>'version'
            !~ '^cargo [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6} \([a-f0-9]{7,40} [0-9]{4}-[0-9]{2}-[0-9]{2}\)$'
       OR v_toolchain->'rustc'->>'version'
            !~ '^rustc [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6} \([a-f0-9]{7,40} [0-9]{4}-[0-9]{2}-[0-9]{2}\)$'
       OR v_toolchain->'rustdoc'->>'version'
            !~ '^rustdoc [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6} \([a-f0-9]{7,40} [0-9]{4}-[0-9]{2}-[0-9]{2}\)$'
       OR v_toolchain->'sandbox'->>'version'
            !~ '^codex-cli [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}$'
       OR v_toolchain->'sandbox_helper'->>'version'
            !~ '^bubblewrap [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}$'
       OR v_toolchain->'sandbox_helper'->>'sha256' !~ '^[a-f0-9]{64}$'
       OR EXISTS (
            SELECT 1 FROM (VALUES
                (v_gateway->>'version'), (v_linux->>'launcher_version'),
                (v_linux->>'node_version'), (v_linux->>'git_version'),
                (v_fence->>'systemd_run_version'), (v_fence->>'systemctl_version'),
                (v_fence->'supervisor_bootstrap_node'->>'version'),
                (v_fence->'immutable_probe_lsattr'->>'version'),
                (v_fence->'noninteractive_root_probe'->>'version'),
                (v_toolchain->'npm'->>'version'), (v_toolchain->'cargo'->>'version'),
                (v_toolchain->'rustc'->>'version'), (v_toolchain->'rustdoc'->>'version'),
                (v_toolchain->'sandbox_helper'->>'version')
            ) AS version_text(value)
            WHERE pg_catalog.length(version_text.value) NOT BETWEEN 1 AND 128
       )
       OR v_toolchain->>'identity_digest' !~ '^wsl2-verification-toolchain:sha256:[a-f0-9]{64}$'
       OR pg_catalog.jsonb_typeof(v_mapping) <> 'object'
       OR (SELECT pg_catalog.count(*) FROM pg_catalog.jsonb_object_keys(v_mapping)) <> 3
       OR NOT (v_mapping ?& ARRAY['windows_path','linux_path','digest'])
       OR v_mapping->>'linux_path' IS DISTINCT FROM v_linux->>'cwd'
       OR pg_catalog.lower(v_mapping->>'windows_path') IS DISTINCT FROM pg_catalog.lower(
            pg_catalog.chr(92) || pg_catalog.chr(92) || 'wsl.localhost' ||
            pg_catalog.chr(92) || (v_descriptor->>'distribution') ||
            pg_catalog.replace(v_mapping->>'linux_path', '/', pg_catalog.chr(92))
       )
       OR v_mapping->>'digest' !~ '^path-mapping:sha256:[a-f0-9]{64}$' THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED';
    END IF;
    v_expected_nested_ref := 'path-mapping:sha256:' || pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(
            foreman_execution.canonical_json_v1(pg_catalog.jsonb_build_object(
                'distribution', v_descriptor->'distribution',
                'windows_path', v_mapping->'windows_path',
                'linux_path', v_mapping->'linux_path',
                'repository_identity', v_linux->'repository_identity',
                'repository_head', v_linux->'repository_head'
            )), 'UTF8'
        )), 'hex'
    );
    IF v_mapping->>'digest' IS DISTINCT FROM v_expected_nested_ref THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_DIGEST_MISMATCH';
    END IF;
    IF pg_catalog.jsonb_typeof(v_immutable_snapshot) <> 'object'
       OR (SELECT pg_catalog.count(*)
             FROM pg_catalog.jsonb_object_keys(v_immutable_snapshot)) <> 10
       OR NOT (v_immutable_snapshot ?& ARRAY[
            'schema','task_root_path','task_root_device','task_root_inode',
            'task_root_owner_uid','task_root_owner_gid','task_root_mode',
            'task_root_immutable','trees','snapshot_digest'
       ])
       OR v_immutable_snapshot->>'schema'
            IS DISTINCT FROM 'lattice.wsl2-immutable-snapshot/1.0'
       OR v_immutable_snapshot->>'task_root_path'
            IS DISTINCT FROM v_toolchain->>'task_root'
       OR pg_catalog.jsonb_typeof(v_immutable_snapshot->'task_root_device') <> 'string'
       OR v_immutable_snapshot->>'task_root_device' !~ '^[1-9][0-9]{0,19}$'
       OR (v_immutable_snapshot->>'task_root_device')::numeric > 18446744073709551615
       OR pg_catalog.jsonb_typeof(v_immutable_snapshot->'task_root_inode') <> 'string'
       OR v_immutable_snapshot->>'task_root_inode' !~ '^[1-9][0-9]{0,19}$'
       OR (v_immutable_snapshot->>'task_root_inode')::numeric > 18446744073709551615
       OR pg_catalog.jsonb_typeof(v_immutable_snapshot->'task_root_owner_uid') <> 'number'
       OR (v_immutable_snapshot->>'task_root_owner_uid')::bigint <> 0
       OR pg_catalog.jsonb_typeof(v_immutable_snapshot->'task_root_owner_gid') <> 'number'
       OR (v_immutable_snapshot->>'task_root_owner_gid')::bigint <> 0
       OR v_immutable_snapshot->>'task_root_mode' IS DISTINCT FROM '0555'
       OR pg_catalog.jsonb_typeof(v_immutable_snapshot->'task_root_immutable') <> 'boolean'
       OR (v_immutable_snapshot->>'task_root_immutable')::boolean IS DISTINCT FROM true
       OR v_immutable_snapshot->>'snapshot_digest'
            !~ '^wsl2-immutable-snapshot:sha256:[a-f0-9]{64}$'
       OR pg_catalog.jsonb_typeof(v_immutable_snapshot->'trees') <> 'object'
       OR (SELECT pg_catalog.count(*)
             FROM pg_catalog.jsonb_object_keys(v_immutable_snapshot->'trees')) <> 5
       OR NOT (v_immutable_snapshot->'trees' ?& ARRAY[
            'codex','supervisor_runtime','node','rust','keyring'
       ])
       OR (SELECT pg_catalog.count(DISTINCT tree_value->>'root')
             FROM pg_catalog.jsonb_each(v_immutable_snapshot->'trees')
                  AS tree_entry(tree_name, tree_value)) <> 5
       OR EXISTS (
            SELECT 1
              FROM pg_catalog.jsonb_each(v_immutable_snapshot->'trees')
                   AS left_tree(tree_name, tree_value)
              CROSS JOIN pg_catalog.jsonb_each(v_immutable_snapshot->'trees')
                   AS right_tree(tree_name, tree_value)
             WHERE left_tree.tree_name < right_tree.tree_name
               AND (
                    left_tree.tree_value->>'root' = right_tree.tree_value->>'root'
                    OR pg_catalog.starts_with(
                        left_tree.tree_value->>'root',
                        right_tree.tree_value->>'root' || '/'
                    )
                    OR pg_catalog.starts_with(
                        right_tree.tree_value->>'root',
                        left_tree.tree_value->>'root' || '/'
                    )
               )
       )
       OR EXISTS (
            SELECT 1
              FROM (VALUES
                    ('codex'), ('supervisor_runtime'), ('node'), ('rust'), ('keyring')
              ) AS tree(tree_name)
             WHERE pg_catalog.jsonb_typeof(
                       v_immutable_snapshot->'trees'->(tree.tree_name)
                   ) <> 'object'
                OR (SELECT pg_catalog.count(*)
                      FROM pg_catalog.jsonb_object_keys(
                          v_immutable_snapshot->'trees'->(tree.tree_name)
                      )) <> 2
                OR NOT (
                    v_immutable_snapshot->'trees'->(tree.tree_name)
                    ?& ARRAY['root','manifest_digest']
                )
                 OR NOT pg_catalog.starts_with(
                     v_immutable_snapshot->'trees'->(tree.tree_name)->>'root',
                     v_toolchain->>'task_root' || '/'
                 )
                 OR v_immutable_snapshot->'trees'->(tree.tree_name)->>'root' !~ '^/'
                 OR v_immutable_snapshot->'trees'->(tree.tree_name)->>'root'
                        ~ '(^|/)\.\.?(/|$)'
                 OR pg_catalog.strpos(
                        v_immutable_snapshot->'trees'->(tree.tree_name)->>'root', '//'
                    ) > 0
                 OR pg_catalog.right(
                        v_immutable_snapshot->'trees'->(tree.tree_name)->>'root', 1
                    ) = '/'
                 OR v_immutable_snapshot->'trees'->(tree.tree_name)->>'root' ~ '^/mnt/'
                 OR v_immutable_snapshot->'trees'->(tree.tree_name)->>'root'
                        !~ '^/[A-Za-z0-9._~/-]+$'
                 OR pg_catalog.substr(
                    v_immutable_snapshot->'trees'->(tree.tree_name)->>'root',
                    pg_catalog.length(v_toolchain->>'task_root') + 2
                ) = ''
                OR pg_catalog.strpos(
                    pg_catalog.substr(
                        v_immutable_snapshot->'trees'->(tree.tree_name)->>'root',
                        pg_catalog.length(v_toolchain->>'task_root') + 2
                    ),
                    '/'
                ) > 0
                OR v_immutable_snapshot->'trees'->(tree.tree_name)->>'manifest_digest'
                    !~ '^immutable-tree-manifest:sha256:[a-f0-9]{64}$'
       )
       OR v_linux->>'launcher_path' IS DISTINCT FROM
            v_immutable_snapshot->'trees'->'codex'->>'root' || '/bin/codex'
       OR NOT pg_catalog.starts_with(
            v_toolchain->'sandbox'->>'path',
            v_immutable_snapshot->'trees'->'codex'->>'root' || '/'
       )
       OR NOT pg_catalog.starts_with(
            v_linux->>'supervisor_path',
            v_immutable_snapshot->'trees'->'supervisor_runtime'->>'root' || '/'
       )
       OR NOT pg_catalog.starts_with(
            v_linux->>'node_path',
            v_immutable_snapshot->'trees'->'node'->>'root' || '/'
       )
       OR NOT pg_catalog.starts_with(
            v_toolchain->'npm'->>'path',
            v_immutable_snapshot->'trees'->'node'->>'root' || '/'
       )
       OR NOT pg_catalog.starts_with(
            v_toolchain->'cargo'->>'path',
            v_immutable_snapshot->'trees'->'rust'->>'root' || '/'
       )
       OR NOT pg_catalog.starts_with(
            v_toolchain->'rustc'->>'path',
            v_immutable_snapshot->'trees'->'rust'->>'root' || '/'
       )
       OR NOT pg_catalog.starts_with(
            v_toolchain->'rustdoc'->>'path',
            v_immutable_snapshot->'trees'->'rust'->>'root' || '/'
       )
       OR v_linux->>'keyring_daemon_path' IS DISTINCT FROM
            v_immutable_snapshot->'trees'->'keyring'->>'root' ||
            '/root/usr/bin/gnome-keyring-daemon'
       OR v_linux->>'keyring_library_path' IS DISTINCT FROM
            v_immutable_snapshot->'trees'->'keyring'->>'root' || '/packages'
       OR pg_catalog.jsonb_typeof(v_sandbox_policy) <> 'object'
       OR (SELECT pg_catalog.count(*)
             FROM pg_catalog.jsonb_object_keys(v_sandbox_policy)) <> 2
       OR NOT (v_sandbox_policy ?& ARRAY['schema','policy_digest'])
       OR v_sandbox_policy->>'schema'
            IS DISTINCT FROM 'lattice.wsl2-sandbox-policy/1.0'
       OR v_sandbox_policy->>'policy_digest'
            !~ '^wsl2-sandbox-policy:sha256:[a-f0-9]{64}$'
       OR pg_catalog.jsonb_typeof(v_privilege_boundary) <> 'object'
       OR (SELECT pg_catalog.count(*)
             FROM pg_catalog.jsonb_object_keys(v_privilege_boundary)) <> 6
       OR NOT (v_privilege_boundary ?& ARRAY[
            'schema','effective_uid','effective_gid','effective_capabilities_digest',
            'noninteractive_root_unavailable','boundary_digest'
       ])
       OR v_privilege_boundary->>'schema'
            IS DISTINCT FROM 'lattice.wsl2-privilege-boundary/1.0'
       OR pg_catalog.jsonb_typeof(v_privilege_boundary->'effective_uid') <> 'number'
       OR (v_privilege_boundary->>'effective_uid')::bigint
            IS DISTINCT FROM (v_toolchain->>'owner_uid')::bigint
       OR pg_catalog.jsonb_typeof(v_privilege_boundary->'effective_gid') <> 'number'
       OR (v_privilege_boundary->>'effective_gid')::bigint <= 0
       OR v_privilege_boundary->>'effective_capabilities_digest'
            !~ '^linux-capabilities:sha256:[a-f0-9]{64}$'
       OR pg_catalog.jsonb_typeof(
            v_privilege_boundary->'noninteractive_root_unavailable'
          ) <> 'boolean'
       OR (v_privilege_boundary->>'noninteractive_root_unavailable')::boolean
            IS DISTINCT FROM true
       OR v_privilege_boundary->>'boundary_digest'
            !~ '^wsl2-privilege-boundary:sha256:[a-f0-9]{64}$' THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED';
    END IF;
    v_node_version_match := pg_catalog.regexp_match(
        v_linux->>'node_version', '^v([0-9]+)\.([0-9]+)\.([0-9]+)$'
    );
    IF v_node_version_match IS NULL THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED';
    END IF;
    IF ARRAY[
        v_node_version_match[1]::bigint,
        v_node_version_match[2]::bigint,
        v_node_version_match[3]::bigint
    ] < ARRAY[24::bigint, 15::bigint, 0::bigint] THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED';
    END IF;

    v_expected_nested_ref := 'wsl2-distribution:sha256:' || pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(
            foreman_execution.canonical_json_v1(
                pg_catalog.jsonb_build_object('distribution', v_descriptor->'distribution') ||
                (v_distribution - 'identity_digest')
            ), 'UTF8'
        )), 'hex'
    );
    IF v_distribution->>'identity_digest' IS DISTINCT FROM v_expected_nested_ref THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_DIGEST_MISMATCH';
    END IF;
    v_expected_nested_ref := 'wsl2-credential-authority:sha256:' || pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(
            foreman_execution.canonical_json_v1(pg_catalog.jsonb_build_object(
                'kind', v_credential->'kind',
                'distribution_identity_ref', v_distribution->'identity_digest',
                'codex_home', v_linux->'codex_home',
                'config_digest', v_linux->'config_digest',
                'keyring_daemon_path', v_linux->'keyring_daemon_path',
                'keyring_daemon_sha256', v_linux->'keyring_daemon_sha256',
                'keyring_library_path', v_linux->'keyring_library_path',
                'keyring_library_manifest_digest', v_linux->'keyring_library_manifest_digest',
                'xdg_runtime_dir', v_linux->'xdg_runtime_dir'
            )), 'UTF8'
        )), 'hex'
    );
    IF v_credential->>'authority_digest' IS DISTINCT FROM v_expected_nested_ref THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_DIGEST_MISMATCH';
    END IF;
    v_expected_nested_ref := 'wsl2-process-fence-authority:sha256:' || pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(
            foreman_execution.canonical_json_v1(
                pg_catalog.jsonb_build_object(
                    'distribution_identity_ref', v_distribution->'identity_digest'
                ) || (v_fence - 'identity_digest')
            ), 'UTF8'
        )), 'hex'
    );
    IF v_fence->>'identity_digest' IS DISTINCT FROM v_expected_nested_ref THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_DIGEST_MISMATCH';
    END IF;
    v_expected_nested_ref := 'wsl2-verification-toolchain:sha256:' || pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(
            foreman_execution.canonical_json_v1(v_toolchain - 'identity_digest'), 'UTF8'
        )), 'hex'
    );
    IF v_toolchain->>'identity_digest' IS DISTINCT FROM v_expected_nested_ref THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_DIGEST_MISMATCH';
    END IF;
    v_expected_nested_ref := 'wsl2-immutable-snapshot:sha256:' || pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(
            foreman_execution.canonical_json_v1(
                v_immutable_snapshot - 'snapshot_digest'
            ), 'UTF8'
        )), 'hex'
    );
    IF v_immutable_snapshot->>'snapshot_digest' IS DISTINCT FROM v_expected_nested_ref THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_DIGEST_MISMATCH';
    END IF;
    v_linux_home := (pg_catalog.regexp_match(
        v_toolchain->>'task_root', '^(/home/[^/]+)'
    ))[1];
    v_sandbox_policy_template := pg_catalog.jsonb_build_object(
        'schema', 'lattice.wsl2-sandbox-template/1.0',
        'permission_profile_type', 'managed',
        'filesystem_type', 'restricted',
        'network', 'restricted',
        'base_entries', pg_catalog.jsonb_build_array(
            pg_catalog.jsonb_build_object(
                'path', pg_catalog.jsonb_build_object(
                    'type', 'special',
                    'value', pg_catalog.jsonb_build_object('kind', 'minimal')
                ),
                'access', 'read'
            ),
            pg_catalog.jsonb_build_object(
                'path', pg_catalog.jsonb_build_object(
                    'type', 'path', 'path', v_toolchain->'task_root'
                ),
                'access', 'read'
            )
        ),
        'role_writes', pg_catalog.jsonb_build_object(
            'PREFLIGHT', pg_catalog.jsonb_build_array(
                v_linux->'cwd', v_toolchain->'home_dir', v_toolchain->'temp_dir',
                v_toolchain->'npm_cache', v_toolchain->'cargo_home',
                v_toolchain->'cargo_target_dir'
            ),
            'NODE', pg_catalog.jsonb_build_array(
                v_toolchain->'home_dir', v_toolchain->'temp_dir',
                v_toolchain->'npm_cache'
            ),
            'CARGO', pg_catalog.jsonb_build_array(
                v_toolchain->'home_dir', v_toolchain->'temp_dir',
                v_toolchain->'cargo_home', v_toolchain->'cargo_target_dir'
            ),
            'GIT', pg_catalog.jsonb_build_object(
                'bootstrap', pg_catalog.jsonb_build_array(
                    '$GIT_CONTROL_HOME', '$GIT_CONTROL_TMPDIR'
                ),
                'guarded_object_write', pg_catalog.jsonb_build_array(
                    '$GIT_CONTROL_HOME', '$GIT_CONTROL_TMPDIR',
                    '$GIT_COMMON_DIR/objects'
                ),
                'guarded_index_write', pg_catalog.jsonb_build_array(
                    '$GIT_CONTROL_HOME', '$GIT_CONTROL_TMPDIR',
                    '$GIT_CONTROL_ROOT/candidate-index'
                )
            )
        ),
        'deny_entries', pg_catalog.jsonb_build_array(
            pg_catalog.jsonb_build_object(
                'path', v_linux->'codex_home', 'missing_path_behavior', 'skip'
            ),
            pg_catalog.jsonb_build_object(
                'path', v_linux_home || '/.codex', 'missing_path_behavior', 'skip'
            ),
            pg_catalog.jsonb_build_object(
                'path', '/mnt', 'missing_path_behavior', 'skip'
            ),
            pg_catalog.jsonb_build_object(
                'path', v_linux->'xdg_runtime_dir', 'missing_path_behavior', 'skip'
            )
        ),
        'codex_linux_sandbox_exe', NULL::text,
        'sandbox_cwd', 'file://' || (v_linux->>'cwd'),
        'use_legacy_landlock', false
    );
    v_expected_nested_ref := 'wsl2-sandbox-policy:sha256:' || pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(
            foreman_execution.canonical_json_v1(v_sandbox_policy_template), 'UTF8'
        )), 'hex'
    );
    IF v_sandbox_policy->>'policy_digest' IS DISTINCT FROM v_expected_nested_ref THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_DIGEST_MISMATCH';
    END IF;
    v_expected_nested_ref := 'wsl2-privilege-boundary:sha256:' || pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(
            foreman_execution.canonical_json_v1(
                v_privilege_boundary - 'boundary_digest'
            ), 'UTF8'
        )), 'hex'
    );
    IF v_privilege_boundary->>'boundary_digest' IS DISTINCT FROM v_expected_nested_ref THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_DIGEST_MISMATCH';
    END IF;
    v_canonical_descriptor := foreman_execution.canonical_json_v1(v_descriptor);
    v_canonical_subject := foreman_execution.canonical_json_v1(v_descriptor - 'identity_digest');
    v_execution_domain_digest := pg_catalog.sha256(
        pg_catalog.convert_to(v_canonical_subject, 'UTF8')
    );
    v_expected_ref := 'execution-environment:sha256:' ||
        pg_catalog.encode(v_execution_domain_digest, 'hex');
    IF p_environment_ref IS DISTINCT FROM v_expected_ref
       OR v_descriptor->>'identity_digest' IS DISTINCT FROM v_expected_ref THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_DIGEST_MISMATCH';
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);
    SELECT pg_catalog.count(*), pg_catalog.min(anchor.attempt_id),
           pg_catalog.decode(
               pg_catalog.min(pg_catalog.encode(anchor.packet_digest, 'hex')), 'hex'
           ), pg_catalog.min(anchor.execution_environment_ref),
           COALESCE(pg_catalog.sum(anchor.active_anchor), 0)
      INTO v_anchor_count, v_anchor_id, v_anchor_packet, v_anchor_environment_ref,
           v_active_anchor_count
      FROM (
          SELECT pending.attempt_id::text, pending.packet_digest,
                 pending.execution_environment_ref::text, 0::bigint AS active_anchor
            FROM ONLY foreman_execution.pending_worker_claims AS pending
           WHERE pending.task_ref = p_task_ref
             AND pending.attempt_number = p_attempt_number
          UNION ALL
          SELECT attempt.attempt_id::text, attempt.packet_digest,
                 attempt.execution_environment_ref::text, 1::bigint AS active_anchor
            FROM ONLY foreman_execution.worker_attempts AS attempt
           WHERE attempt.task_ref = p_task_ref
             AND attempt.attempt_number = p_attempt_number
      ) AS anchor;
    IF v_anchor_count <> 1
       OR v_anchor_id IS DISTINCT FROM p_attempt_id
       OR v_anchor_packet IS DISTINCT FROM p_packet_digest
       OR v_anchor_environment_ref IS DISTINCT FROM p_environment_ref THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_ATTEMPT_MISMATCH';
    END IF;
    SELECT environment.* INTO v_existing
      FROM ONLY foreman_execution.execution_environments AS environment
     WHERE environment.task_ref = p_task_ref
       AND environment.attempt_number = p_attempt_number;
    IF FOUND THEN
        IF v_existing.attempt_id IS DISTINCT FROM p_attempt_id
           OR v_existing.packet_digest IS DISTINCT FROM p_packet_digest
           OR v_existing.canonical_descriptor IS DISTINCT FROM v_canonical_descriptor
           OR v_existing.execution_domain_digest IS DISTINCT FROM v_execution_domain_digest
           OR v_existing.environment_ref IS DISTINCT FROM p_environment_ref
           OR v_existing.linux_repository_path IS DISTINCT FROM v_linux->>'cwd'
           OR v_existing.cargo_path IS DISTINCT FROM v_toolchain->'cargo'->>'path'
           OR v_existing.process_fence_identity_ref IS DISTINCT FROM v_fence->>'identity_digest'
           OR v_existing.immutable_snapshot_ref IS DISTINCT FROM
                v_immutable_snapshot->>'snapshot_digest'
           OR v_existing.sandbox_policy_ref IS DISTINCT FROM
                v_sandbox_policy->>'policy_digest'
           OR v_existing.privilege_boundary_ref IS DISTINCT FROM
                v_privilege_boundary->>'boundary_digest' THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_SUBSTITUTION';
        END IF;
        PERFORM 1
          FROM foreman_execution.read_execution_environment_rows_v1(p_task_ref) AS environment
         WHERE environment.attempt_number = p_attempt_number
           AND environment.environment_ref = p_environment_ref;
        IF NOT FOUND THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_SUBSTITUTION';
        END IF;
        RETURN 'EXACT_REPLAY';
    END IF;
    IF v_active_anchor_count <> 0 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_SUBSTITUTION';
    END IF;
    INSERT INTO foreman_execution.execution_environments (
        task_ref, attempt_number, attempt_id, packet_digest,
        descriptor_schema, environment_kind, canonical_descriptor,
        distribution, distribution_os_id, distribution_version, distribution_codename,
        distribution_os_release_digest, distribution_kernel_release,
        distribution_identity_ref, distribution_identity_digest,
        gateway_path, gateway_version, gateway_digest,
        linux_repository_path, linux_codex_home_path, codex_config_ref,
        codex_config_digest, repository_head, repository_identity_ref,
        repository_identity_digest, launcher_path, launcher_version, launcher_digest,
        node_path, node_version, node_digest, git_path, git_version, git_digest,
        supervisor_path, supervisor_digest, dbus_run_session_path,
        dbus_run_session_digest, setsid_path, setsid_digest, keyring_daemon_path,
        keyring_daemon_digest, keyring_library_path, keyring_library_manifest_ref,
        keyring_library_manifest_digest, xdg_runtime_dir,
        credential_authority_kind, credential_authority_ref,
        credential_authority_digest, process_fence_schema, process_fence_kind,
        systemd_run_path, systemd_run_version, systemd_run_digest,
        systemctl_path, systemctl_version, systemctl_digest, cgroup_mount,
        supervisor_bootstrap_node_path, supervisor_bootstrap_node_version,
        supervisor_bootstrap_node_digest,
        immutable_probe_lsattr_path, immutable_probe_lsattr_version,
        immutable_probe_lsattr_digest, noninteractive_root_probe_path,
        noninteractive_root_probe_version, noninteractive_root_probe_digest,
        user_runtime_dir, unit_prefix, process_fence_identity_ref,
        process_fence_identity_digest, verification_toolchain_schema,
        verification_task_ref, verification_task_root, verification_isolation_root,
        verification_owner_uid, verification_home_dir, verification_temp_dir,
        npm_cache, cargo_home, cargo_target_dir, cargo_host,
        npm_path, npm_version, npm_digest, cargo_path, cargo_version, cargo_digest,
        rustc_path, rustc_version, rustc_digest, rustdoc_path, rustdoc_version,
        rustdoc_digest, sandbox_path, sandbox_version, sandbox_digest,
        sandbox_helper_path, sandbox_helper_version, sandbox_helper_digest,
        verification_toolchain_identity_ref, verification_toolchain_identity_digest,
        immutable_snapshot_ref, immutable_snapshot_digest,
        sandbox_policy_ref, sandbox_policy_digest,
        privilege_boundary_ref, privilege_boundary_digest,
        path_mapping_windows_path, path_mapping_linux_path, path_mapping_ref,
        path_mapping_digest, execution_domain_digest, environment_ref
    ) VALUES (
        p_task_ref, p_attempt_number, p_attempt_id, p_packet_digest,
        v_descriptor->>'schema', v_descriptor->>'kind', v_canonical_descriptor,
        v_descriptor->>'distribution', v_distribution->>'os_id',
        v_distribution->>'os_version_id', v_distribution->>'os_version_codename',
        pg_catalog.decode(v_distribution->>'os_release_sha256', 'hex'),
        v_distribution->>'kernel_release', v_distribution->>'identity_digest',
        pg_catalog.decode(pg_catalog.right(v_distribution->>'identity_digest', 64), 'hex'),
        v_gateway->>'windows_path', v_gateway->>'version',
        pg_catalog.decode(v_gateway->>'sha256', 'hex'),
        v_linux->>'cwd', v_linux->>'codex_home', v_linux->>'config_digest',
        pg_catalog.decode(pg_catalog.right(v_linux->>'config_digest', 64), 'hex'),
        v_linux->>'repository_head', v_linux->>'repository_identity',
        pg_catalog.decode(pg_catalog.right(v_linux->>'repository_identity', 64), 'hex'),
        v_linux->>'launcher_path', v_linux->>'launcher_version',
        pg_catalog.decode(v_linux->>'launcher_sha256', 'hex'),
        v_linux->>'node_path', v_linux->>'node_version',
        pg_catalog.decode(v_linux->>'node_sha256', 'hex'),
        v_linux->>'git_path', v_linux->>'git_version',
        pg_catalog.decode(v_linux->>'git_sha256', 'hex'),
        v_linux->>'supervisor_path', pg_catalog.decode(v_linux->>'supervisor_sha256', 'hex'),
        v_linux->>'dbus_run_session_path', pg_catalog.decode(v_linux->>'dbus_run_session_sha256', 'hex'),
        v_linux->>'setsid_path', pg_catalog.decode(v_linux->>'setsid_sha256', 'hex'),
        v_linux->>'keyring_daemon_path', pg_catalog.decode(v_linux->>'keyring_daemon_sha256', 'hex'),
        v_linux->>'keyring_library_path', v_linux->>'keyring_library_manifest_digest',
        pg_catalog.decode(pg_catalog.right(
            v_linux->>'keyring_library_manifest_digest', 64
        ), 'hex'), v_linux->>'xdg_runtime_dir',
        v_credential->>'kind', v_credential->>'authority_digest',
        pg_catalog.decode(pg_catalog.right(v_credential->>'authority_digest', 64), 'hex'),
        v_fence->>'schema', v_fence->>'kind', v_fence->>'systemd_run_path',
        v_fence->>'systemd_run_version', pg_catalog.decode(v_fence->>'systemd_run_sha256', 'hex'),
        v_fence->>'systemctl_path', v_fence->>'systemctl_version',
        pg_catalog.decode(v_fence->>'systemctl_sha256', 'hex'), v_fence->>'cgroup_mount',
        v_fence->'supervisor_bootstrap_node'->>'path',
        v_fence->'supervisor_bootstrap_node'->>'version',
        pg_catalog.decode(v_fence->'supervisor_bootstrap_node'->>'sha256', 'hex'),
        v_fence->'immutable_probe_lsattr'->>'path',
        v_fence->'immutable_probe_lsattr'->>'version',
        pg_catalog.decode(v_fence->'immutable_probe_lsattr'->>'sha256', 'hex'),
        v_fence->'noninteractive_root_probe'->>'path',
        v_fence->'noninteractive_root_probe'->>'version',
        pg_catalog.decode(v_fence->'noninteractive_root_probe'->>'sha256', 'hex'),
        v_fence->>'user_runtime_dir', v_fence->>'unit_prefix', v_fence->>'identity_digest',
        pg_catalog.decode(pg_catalog.right(v_fence->>'identity_digest', 64), 'hex'),
        v_toolchain->>'schema', pg_catalog.decode(v_toolchain->>'task_ref', 'hex'),
        v_toolchain->>'task_root', v_toolchain->>'isolation_root',
        (v_toolchain->>'owner_uid')::bigint, v_toolchain->>'home_dir',
        v_toolchain->>'temp_dir', v_toolchain->>'npm_cache', v_toolchain->>'cargo_home',
        v_toolchain->>'cargo_target_dir', v_toolchain->>'cargo_host',
        v_toolchain->'npm'->>'path', v_toolchain->'npm'->>'version',
        pg_catalog.decode(v_toolchain->'npm'->>'sha256', 'hex'),
        v_toolchain->'cargo'->>'path', v_toolchain->'cargo'->>'version',
        pg_catalog.decode(v_toolchain->'cargo'->>'sha256', 'hex'),
        v_toolchain->'rustc'->>'path', v_toolchain->'rustc'->>'version',
        pg_catalog.decode(v_toolchain->'rustc'->>'sha256', 'hex'),
        v_toolchain->'rustdoc'->>'path', v_toolchain->'rustdoc'->>'version',
        pg_catalog.decode(v_toolchain->'rustdoc'->>'sha256', 'hex'),
        v_toolchain->'sandbox'->>'path', v_toolchain->'sandbox'->>'version',
        pg_catalog.decode(v_toolchain->'sandbox'->>'sha256', 'hex'),
        v_toolchain->'sandbox_helper'->>'path',
        v_toolchain->'sandbox_helper'->>'version',
        pg_catalog.decode(v_toolchain->'sandbox_helper'->>'sha256', 'hex'),
        v_toolchain->>'identity_digest',
        pg_catalog.decode(pg_catalog.right(v_toolchain->>'identity_digest', 64), 'hex'),
        v_immutable_snapshot->>'snapshot_digest',
        pg_catalog.decode(pg_catalog.right(
            v_immutable_snapshot->>'snapshot_digest', 64
        ), 'hex'),
        v_sandbox_policy->>'policy_digest',
        pg_catalog.decode(pg_catalog.right(v_sandbox_policy->>'policy_digest', 64), 'hex'),
        v_privilege_boundary->>'boundary_digest',
        pg_catalog.decode(pg_catalog.right(
            v_privilege_boundary->>'boundary_digest', 64
        ), 'hex'),
        v_mapping->>'windows_path', v_mapping->>'linux_path', v_mapping->>'digest',
        pg_catalog.decode(pg_catalog.right(v_mapping->>'digest', 64), 'hex'),
        v_execution_domain_digest, p_environment_ref
    );
    RETURN 'INSERTED';
END;
$$;

CREATE FUNCTION foreman_execution.claim_worker_attempt_v1(
    p_task_ref bytea, p_successor_stream_id bytea, p_task_spec_digest bytea,
    p_binding_digest bytea, p_budget_digest bytea, p_attempt_id text,
    p_attempt_number smallint, p_foreman_generation bigint, p_model text,
    p_reasoning text, p_writer_fence bigint, p_foreman_checkpoint_digest bytea,
    p_approval_receipt_digest bytea, p_packet_digest bytea,
    p_execution_environment_ref text, p_worktree_digest bytea, p_base_commit_digest bytea,
    p_model_reason text, p_model_reason_digest bytea, p_claimed_at text, p_payload_digest bytea,
    p_max_attempts smallint, p_stream_id bytea, p_event_sequence numeric,
    p_event_digest bytea, p_command_id text, p_request_digest bytea
) RETURNS TABLE(disposition text, global_active bigint, task_active bigint)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_existing foreman_execution.worker_attempts%ROWTYPE;
    v_pending foreman_execution.pending_worker_claims%ROWTYPE;
    v_previous foreman_execution.worker_attempts%ROWTYPE;
    v_environment foreman_execution.execution_environments%ROWTYPE;
    v_global bigint;
    v_task bigint;
    v_max smallint;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);
    IF p_execution_environment_ref !~ '^execution-environment:sha256:[a-f0-9]{64}$'
       OR p_execution_environment_ref =
          'execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000000' THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_REF_REJECTED';
    END IF;
    IF p_max_attempts NOT BETWEEN 1 AND 3 OR p_attempt_number > p_max_attempts THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_RETRY_BUDGET_EXHAUSTED';
    END IF;
    IF (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.task_promotions AS p
         WHERE p.task_ref = p_task_ref AND p.successor_stream_id = p_successor_stream_id
           AND p.task_spec_digest = p_task_spec_digest AND p.binding_digest = p_binding_digest
           AND p.budget_digest = p_budget_digest
           AND p.global_active_limit = 4 AND p.per_task_active_limit = 1
           AND p.repair_retry_limit + 1 = p_max_attempts) <> 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_ATTEMPT_BINDING_MISMATCH';
    END IF;
    SELECT environment.* INTO v_environment
      FROM ONLY foreman_execution.execution_environments AS environment
     WHERE environment.task_ref = p_task_ref
       AND environment.attempt_number = p_attempt_number;
    IF p_execution_environment_ref =
       'execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000001' THEN
        IF FOUND THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_ATTEMPT_MISMATCH';
        END IF;
        PERFORM pg_catalog.count(*)
          FROM foreman_execution.read_execution_environment_rows_v1(p_task_ref);
    ELSE
        IF NOT FOUND THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_REQUIRED';
        END IF;
        IF v_environment.environment_ref IS DISTINCT FROM p_execution_environment_ref
           OR v_environment.attempt_id IS DISTINCT FROM p_attempt_id
           OR v_environment.packet_digest IS DISTINCT FROM p_packet_digest THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_ATTEMPT_MISMATCH';
        END IF;
        PERFORM 1
          FROM foreman_execution.read_execution_environment_rows_v1(p_task_ref) AS environment
         WHERE environment.attempt_number = p_attempt_number
           AND environment.environment_ref = p_execution_environment_ref;
        IF NOT FOUND THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_REQUIRED';
        END IF;
    END IF;
    SELECT * INTO v_existing FROM ONLY foreman_execution.worker_attempts
     WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number;
    IF FOUND THEN
        IF v_existing.successor_stream_id IS DISTINCT FROM p_successor_stream_id
           OR v_existing.task_spec_digest IS DISTINCT FROM p_task_spec_digest
           OR v_existing.binding_digest IS DISTINCT FROM p_binding_digest
           OR v_existing.budget_digest IS DISTINCT FROM p_budget_digest
           OR v_existing.attempt_id IS DISTINCT FROM p_attempt_id
           OR v_existing.foreman_generation IS DISTINCT FROM p_foreman_generation
           OR v_existing.model IS DISTINCT FROM p_model
           OR v_existing.reasoning IS DISTINCT FROM p_reasoning
           OR v_existing.writer_fence IS DISTINCT FROM p_writer_fence
           OR v_existing.foreman_checkpoint_digest IS DISTINCT FROM p_foreman_checkpoint_digest
           OR v_existing.approval_receipt_digest IS DISTINCT FROM p_approval_receipt_digest
           OR v_existing.packet_digest IS DISTINCT FROM p_packet_digest
           OR v_existing.execution_environment_ref IS DISTINCT FROM p_execution_environment_ref
           OR v_existing.worktree_digest IS DISTINCT FROM p_worktree_digest
           OR v_existing.base_commit_digest IS DISTINCT FROM p_base_commit_digest
           OR v_existing.model_reason IS DISTINCT FROM p_model_reason
           OR v_existing.model_reason_digest IS DISTINCT FROM p_model_reason_digest
           OR v_existing.claimed_at IS DISTINCT FROM p_claimed_at
           OR v_existing.payload_digest IS DISTINCT FROM p_payload_digest
           OR v_existing.ledger_event_digest IS DISTINCT FROM p_event_digest THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_ATTEMPT_SUBSTITUTION';
        END IF;
        PERFORM foreman_execution.assert_exact_child_event_v1(
            'WORKER_ATTEMPT', p_task_ref, p_stream_id, p_event_sequence,
            p_event_digest, p_command_id, p_request_digest, p_payload_digest,
            'DISPATCH_WORKER_ATTEMPT_V1'
        );
        SELECT pg_catalog.count(*) INTO v_global
          FROM ONLY foreman_execution.worker_attempts AS a
         WHERE NOT EXISTS (
             SELECT 1 FROM ONLY foreman_execution.attempt_closures AS closure
              WHERE closure.task_ref = a.task_ref
                AND closure.attempt_number = a.attempt_number
         ) AND
         NOT EXISTS (
             SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
              WHERE o.task_ref = a.task_ref AND o.attempt_number = a.attempt_number
                AND o.observation_kind IN ('PRESTART_TERMINAL_FAILED','TERMINAL_FAILED','TERMINAL_INTERRUPTED')
         ) AND (
             NOT EXISTS (
                 SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
                  WHERE o.task_ref = a.task_ref AND o.attempt_number = a.attempt_number
                    AND o.observation_kind = 'TERMINAL_COMPLETED'
             ) OR NOT EXISTS (
                 SELECT 1 FROM ONLY foreman_execution.verification_records AS v
                  WHERE v.task_ref = a.task_ref AND v.attempt_number = a.attempt_number
             )
         );
        SELECT pg_catalog.count(*) INTO v_task
          FROM ONLY foreman_execution.worker_attempts AS a
         WHERE a.task_ref = p_task_ref AND NOT EXISTS (
             SELECT 1 FROM ONLY foreman_execution.attempt_closures AS closure
              WHERE closure.task_ref = a.task_ref
                AND closure.attempt_number = a.attempt_number
         ) AND NOT EXISTS (
             SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
              WHERE o.task_ref = a.task_ref AND o.attempt_number = a.attempt_number
                AND o.observation_kind IN ('PRESTART_TERMINAL_FAILED','TERMINAL_FAILED','TERMINAL_INTERRUPTED')
         ) AND (
             NOT EXISTS (
                 SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
                  WHERE o.task_ref = a.task_ref AND o.attempt_number = a.attempt_number
                    AND o.observation_kind = 'TERMINAL_COMPLETED'
             ) OR NOT EXISTS (
                 SELECT 1 FROM ONLY foreman_execution.verification_records AS v
                  WHERE v.task_ref = a.task_ref AND v.attempt_number = a.attempt_number
             )
         );
        RETURN QUERY SELECT 'EXACT_REPLAY'::text, v_global, v_task;
        RETURN;
    END IF;
    SELECT * INTO v_pending FROM ONLY foreman_execution.pending_worker_claims
     WHERE task_ref = p_task_ref;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PENDING_CLAIM_REQUIRED';
    END IF;
    IF v_pending.attempt_number IS DISTINCT FROM p_attempt_number
       OR v_pending.successor_stream_id IS DISTINCT FROM p_successor_stream_id
       OR v_pending.task_spec_digest IS DISTINCT FROM p_task_spec_digest
       OR v_pending.binding_digest IS DISTINCT FROM p_binding_digest
       OR v_pending.budget_digest IS DISTINCT FROM p_budget_digest
       OR v_pending.attempt_id IS DISTINCT FROM p_attempt_id
       OR v_pending.foreman_generation IS DISTINCT FROM p_foreman_generation
       OR v_pending.model IS DISTINCT FROM p_model
       OR v_pending.reasoning IS DISTINCT FROM p_reasoning
       OR v_pending.writer_fence IS DISTINCT FROM p_writer_fence
       OR v_pending.foreman_checkpoint_digest IS DISTINCT FROM p_foreman_checkpoint_digest
           OR v_pending.approval_receipt_digest IS DISTINCT FROM p_approval_receipt_digest
           OR v_pending.packet_digest IS DISTINCT FROM p_packet_digest
           OR v_pending.execution_environment_ref IS DISTINCT FROM p_execution_environment_ref
           OR v_pending.worktree_digest IS DISTINCT FROM p_worktree_digest
       OR v_pending.base_commit_digest IS DISTINCT FROM p_base_commit_digest
       OR v_pending.model_reason IS DISTINCT FROM p_model_reason
       OR v_pending.model_reason_digest IS DISTINCT FROM p_model_reason_digest
       OR v_pending.claimed_at IS DISTINCT FROM p_claimed_at
       OR v_pending.payload_digest IS DISTINCT FROM p_payload_digest
       OR v_pending.max_attempts IS DISTINCT FROM p_max_attempts
       OR v_pending.ledger_event_digest IS DISTINCT FROM p_event_digest THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PENDING_CLAIM_SUBSTITUTION';
    END IF;
    -- Once a terminal pre-provider blocker has been staged, only the atomic
    -- pending-closure function may materialize this packet.  This closes the
    -- cross-owner stage/Ledger/closure crash window against a competing claim.
    IF EXISTS (
        SELECT 1 FROM ONLY foreman_execution.staged_artifact_references AS staged
         WHERE staged.task_ref = p_task_ref
           AND staged.attempt_number = p_attempt_number
           AND staged.evidence_kind = 'WORKER_LIFECYCLE'
           AND staged.payload_schema = 'lattice.managed-blocker.v1'
           AND staged.producer_id = 'lattice-foreman'
           AND staged.producer_version = '1'
           AND staged.producer_digest = v_pending.foreman_checkpoint_digest
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_PENDING_CLOSURE_REQUIRED';
    END IF;
    PERFORM foreman_execution.assert_exact_child_event_v1(
        'WORKER_ATTEMPT', p_task_ref, p_stream_id, p_event_sequence,
        p_event_digest, p_command_id, p_request_digest, p_payload_digest,
        'DISPATCH_WORKER_ATTEMPT_V1'
    );
    IF p_model NOT IN ('gpt-5.6-luna','gpt-5.6-terra','gpt-5.6-sol') THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_MODEL_NOT_ALLOWED';
    END IF;
    IF NOT (
        (p_model = 'gpt-5.6-luna' AND p_model_reason = 'BOUNDED_STATE_EVIDENCE_DOCUMENTATION')
        OR (p_model = 'gpt-5.6-terra' AND p_model_reason = 'ROUTINE_ENGINEERING')
        OR (
            p_model = 'gpt-5.6-sol'
            AND p_model_reason IN ('P0','ARCHITECTURE','SECURITY','HIGH_RISK','TERRA_INSUFFICIENT')
        )
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_MODEL_REASON_NOT_ALLOWED';
    END IF;
    SELECT COALESCE(pg_catalog.max(attempt_number), 0)::smallint INTO v_max
      FROM ONLY foreman_execution.worker_attempts WHERE task_ref = p_task_ref;
    IF p_attempt_number <> v_max + 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_ATTEMPT_SEQUENCE_MISMATCH';
    END IF;
    IF p_attempt_number > 1 THEN
        SELECT * INTO v_previous FROM ONLY foreman_execution.worker_attempts
         WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number - 1;
        IF NOT FOUND OR p_writer_fence <= v_previous.writer_fence
           OR p_foreman_generation < v_previous.foreman_generation
           OR NOT (
               EXISTS (
                   SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
                    WHERE o.task_ref = p_task_ref AND o.attempt_number = p_attempt_number - 1
                      AND o.observation_kind IN ('PRESTART_TERMINAL_FAILED','TERMINAL_COMPLETED','TERMINAL_FAILED','TERMINAL_INTERRUPTED')
               )
               OR EXISTS (
                   SELECT 1 FROM ONLY foreman_execution.attempt_closures AS closure
                    WHERE closure.task_ref = p_task_ref
                      AND closure.attempt_number = p_attempt_number - 1
               )
           ) THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_RETRY_PREDECESSOR_NOT_TERMINAL';
        END IF;
    END IF;
    SELECT pg_catalog.count(*) INTO v_global FROM ONLY foreman_execution.worker_attempts AS a
     WHERE NOT EXISTS (SELECT 1 FROM ONLY foreman_execution.attempt_closures AS closure
        WHERE closure.task_ref = a.task_ref AND closure.attempt_number = a.attempt_number)
       AND NOT EXISTS (SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
        WHERE o.task_ref = a.task_ref AND o.attempt_number = a.attempt_number
          AND o.observation_kind IN ('PRESTART_TERMINAL_FAILED','TERMINAL_FAILED','TERMINAL_INTERRUPTED'))
       AND (NOT EXISTS (SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
              WHERE o.task_ref = a.task_ref AND o.attempt_number = a.attempt_number
                AND o.observation_kind = 'TERMINAL_COMPLETED')
            OR NOT EXISTS (SELECT 1 FROM ONLY foreman_execution.verification_records AS v
              WHERE v.task_ref = a.task_ref AND v.attempt_number = a.attempt_number));
    SELECT pg_catalog.count(*) INTO v_task FROM ONLY foreman_execution.worker_attempts AS a
     WHERE a.task_ref = p_task_ref
       AND NOT EXISTS (SELECT 1 FROM ONLY foreman_execution.attempt_closures AS closure
        WHERE closure.task_ref = a.task_ref AND closure.attempt_number = a.attempt_number)
       AND NOT EXISTS (SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
        WHERE o.task_ref = a.task_ref AND o.attempt_number = a.attempt_number
          AND o.observation_kind IN ('PRESTART_TERMINAL_FAILED','TERMINAL_FAILED','TERMINAL_INTERRUPTED'))
       AND (NOT EXISTS (SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
              WHERE o.task_ref = a.task_ref AND o.attempt_number = a.attempt_number
                AND o.observation_kind = 'TERMINAL_COMPLETED')
            OR NOT EXISTS (SELECT 1 FROM ONLY foreman_execution.verification_records AS v
              WHERE v.task_ref = a.task_ref AND v.attempt_number = a.attempt_number));
    IF v_global >= 4 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_GLOBAL_CAPACITY_EXHAUSTED';
    END IF;
    IF v_task >= 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_TASK_CAPACITY_EXHAUSTED';
    END IF;
    IF p_stream_id IS DISTINCT FROM p_successor_stream_id THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_ATTEMPT_STREAM_MISMATCH';
    END IF;
    INSERT INTO foreman_execution.worker_attempts (
        task_ref, attempt_number, attempt_id, successor_stream_id,
        task_spec_digest, binding_digest, budget_digest, foreman_generation,
        model, reasoning, writer_fence, foreman_checkpoint_digest,
        approval_receipt_digest, packet_digest, execution_environment_ref, worktree_digest,
        base_commit_digest, model_reason, model_reason_digest, claimed_at, payload_digest,
        ledger_event_digest
    ) VALUES (
        p_task_ref, p_attempt_number, p_attempt_id, p_successor_stream_id,
        p_task_spec_digest, p_binding_digest, p_budget_digest,
        p_foreman_generation, p_model, p_reasoning, p_writer_fence,
        p_foreman_checkpoint_digest, p_approval_receipt_digest, p_packet_digest,
        p_execution_environment_ref, p_worktree_digest, p_base_commit_digest,
        p_model_reason, p_model_reason_digest,
        p_claimed_at, p_payload_digest, p_event_digest
    );
    DELETE FROM ONLY foreman_execution.pending_worker_claims
     WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number
       AND ledger_event_digest = p_event_digest;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PENDING_CLAIM_REQUIRED';
    END IF;
    RETURN QUERY SELECT 'CLAIMED'::text, v_global + 1, v_task + 1;
END;
$$;

CREATE FUNCTION foreman_execution.record_worker_observation_v1(
    p_task_ref bytea, p_successor_stream_id bytea, p_binding_digest bytea,
    p_attempt_id text, p_attempt_number smallint, p_observation_kind text,
    p_thread_id text, p_turn_id text, p_app_server_generation bigint,
    p_app_server_identity_digest bytea, p_observed_at text,
    p_evidence_digest bytea, p_payload_digest bytea,
    p_stream_id bytea, p_event_sequence numeric, p_event_digest bytea,
    p_command_id text, p_request_digest bytea
) RETURNS TABLE(disposition text, observation_ordinal bigint)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_existing foreman_execution.worker_observations%ROWTYPE;
    v_ordinal bigint;
    v_previous_generation bigint;
    v_previous_identity_digest bytea;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);
    SELECT o.* INTO v_existing FROM ONLY foreman_execution.worker_observations AS o
     WHERE o.ledger_event_digest = p_event_digest;
    IF FOUND THEN
        IF v_existing.task_ref IS DISTINCT FROM p_task_ref OR v_existing.attempt_number IS DISTINCT FROM p_attempt_number
           OR v_existing.successor_stream_id IS DISTINCT FROM p_successor_stream_id OR v_existing.binding_digest IS DISTINCT FROM p_binding_digest
           OR v_existing.attempt_id IS DISTINCT FROM p_attempt_id OR v_existing.observation_kind IS DISTINCT FROM p_observation_kind
           OR v_existing.thread_id IS DISTINCT FROM p_thread_id OR v_existing.turn_id IS DISTINCT FROM p_turn_id
           OR v_existing.app_server_generation IS DISTINCT FROM p_app_server_generation
           OR v_existing.app_server_identity_digest IS DISTINCT FROM p_app_server_identity_digest
           OR v_existing.observed_at IS DISTINCT FROM p_observed_at
           OR v_existing.evidence_digest IS DISTINCT FROM p_evidence_digest OR v_existing.payload_digest IS DISTINCT FROM p_payload_digest THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_OBSERVATION_SUBSTITUTION';
        END IF;
        PERFORM foreman_execution.assert_exact_child_event_v1(
            'WORKER_OBSERVATION', p_task_ref, p_stream_id, p_event_sequence,
            p_event_digest, p_command_id, p_request_digest, p_payload_digest,
            'RECORD_WORKER_OBSERVATION_V1'
        );
        RETURN QUERY SELECT 'EXACT_REPLAY'::text, v_existing.observation_ordinal;
        RETURN;
    END IF;
    IF p_stream_id IS DISTINCT FROM p_successor_stream_id
       OR NOT EXISTS (SELECT 1 FROM ONLY foreman_execution.worker_attempts AS a
           WHERE a.task_ref = p_task_ref AND a.attempt_number = p_attempt_number
             AND a.attempt_id = p_attempt_id AND a.successor_stream_id = p_successor_stream_id
             AND a.binding_digest = p_binding_digest) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_OBSERVATION_ATTEMPT_MISMATCH';
    END IF;
    IF EXISTS (SELECT 1 FROM ONLY foreman_execution.attempt_closures AS closure
        WHERE closure.task_ref = p_task_ref
          AND closure.attempt_number = p_attempt_number) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_OBSERVATION_AFTER_CLOSURE';
    END IF;
    IF EXISTS (SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
        WHERE o.task_ref = p_task_ref AND o.attempt_number = p_attempt_number
          AND o.observation_kind IN ('PRESTART_TERMINAL_FAILED','TERMINAL_COMPLETED','TERMINAL_FAILED','TERMINAL_INTERRUPTED')) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_OBSERVATION_AFTER_TERMINAL';
    END IF;
    IF EXISTS (SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
        WHERE o.task_ref = p_task_ref AND o.attempt_number = p_attempt_number
          AND o.thread_id <> p_thread_id) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_THREAD_SUBSTITUTION';
    END IF;
    IF p_observation_kind <> 'THREAD_ACCEPTED' AND EXISTS (
        SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
         WHERE o.task_ref = p_task_ref AND o.attempt_number = p_attempt_number
           AND o.turn_id IS NOT NULL AND o.turn_id <> p_turn_id
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_TURN_SUBSTITUTION';
    END IF;
    SELECT o.app_server_generation, o.app_server_identity_digest
      INTO v_previous_generation, v_previous_identity_digest
      FROM ONLY foreman_execution.worker_observations AS o
     WHERE o.task_ref = p_task_ref AND o.attempt_number = p_attempt_number
     ORDER BY o.observation_ordinal DESC
     LIMIT 1;
    IF FOUND AND p_observation_kind <> 'RECONCILED'
       AND (v_previous_generation IS DISTINCT FROM p_app_server_generation
            OR v_previous_identity_digest IS DISTINCT FROM p_app_server_identity_digest) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_APP_SERVER_IDENTITY_DRIFT';
    END IF;
    IF p_observation_kind = 'THREAD_ACCEPTED' AND EXISTS (
        SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
         WHERE o.task_ref = p_task_ref AND o.attempt_number = p_attempt_number
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_THREAD_ACCEPTED_NOT_FIRST';
    END IF;
    IF p_observation_kind <> 'THREAD_ACCEPTED' AND NOT EXISTS (
        SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
         WHERE o.task_ref = p_task_ref AND o.attempt_number = p_attempt_number
           AND o.observation_kind = 'THREAD_ACCEPTED'
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_THREAD_NOT_ACCEPTED';
    END IF;
    IF p_observation_kind = 'TURN_ACCEPTED' AND EXISTS (
        SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
         WHERE o.task_ref = p_task_ref AND o.attempt_number = p_attempt_number
           AND o.observation_kind <> 'THREAD_ACCEPTED'
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_TURN_ACCEPTED_OUT_OF_ORDER';
    END IF;
    IF p_observation_kind IN ('TURN_STARTED','PRESTART_TERMINAL_FAILED','MEANINGFUL_PROGRESS','HEARTBEAT','STALL_CLASSIFIED','INTERRUPT_REQUESTED','RECONCILED','TERMINAL_COMPLETED','TERMINAL_FAILED','TERMINAL_INTERRUPTED')
       AND NOT EXISTS (SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
           WHERE o.task_ref = p_task_ref AND o.attempt_number = p_attempt_number
             AND o.observation_kind = 'TURN_ACCEPTED') THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_TURN_NOT_ACCEPTED';
    END IF;
    IF p_observation_kind IN ('MEANINGFUL_PROGRESS','HEARTBEAT','STALL_CLASSIFIED','INTERRUPT_REQUESTED','RECONCILED','TERMINAL_COMPLETED','TERMINAL_FAILED','TERMINAL_INTERRUPTED')
       AND NOT EXISTS (SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
           WHERE o.task_ref = p_task_ref AND o.attempt_number = p_attempt_number
             AND o.observation_kind = 'TURN_STARTED') THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_TURN_NOT_STARTED';
    END IF;
    IF p_observation_kind = 'TURN_STARTED' AND EXISTS (
        SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
         WHERE o.task_ref = p_task_ref AND o.attempt_number = p_attempt_number
           AND o.observation_kind = 'TURN_STARTED'
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_TURN_STARTED_DUPLICATE';
    END IF;
    IF p_observation_kind = 'PRESTART_TERMINAL_FAILED' AND EXISTS (
        SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
         WHERE o.task_ref = p_task_ref AND o.attempt_number = p_attempt_number
           AND o.observation_kind = 'TURN_STARTED'
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PRESTART_TERMINAL_AFTER_START';
    END IF;
    SELECT COALESCE(pg_catalog.max(o.observation_ordinal), 0) + 1 INTO v_ordinal
      FROM ONLY foreman_execution.worker_observations AS o
     WHERE o.task_ref = p_task_ref AND o.attempt_number = p_attempt_number;
    PERFORM foreman_execution.insert_child_event_v1(
        'WORKER_OBSERVATION', p_task_ref, p_stream_id, p_event_sequence,
        p_event_digest, p_command_id, p_request_digest, p_payload_digest,
        'RECORD_WORKER_OBSERVATION_V1'
    );
    INSERT INTO foreman_execution.worker_observations (
        task_ref, attempt_number, observation_ordinal, attempt_id,
        successor_stream_id, binding_digest, observation_kind,
        thread_id, turn_id, app_server_generation, app_server_identity_digest,
        observed_at, evidence_digest, payload_digest, ledger_event_digest
    ) VALUES (
        p_task_ref, p_attempt_number, v_ordinal, p_attempt_id,
        p_successor_stream_id, p_binding_digest, p_observation_kind,
        p_thread_id, p_turn_id, p_app_server_generation,
        p_app_server_identity_digest, p_observed_at, p_evidence_digest,
        p_payload_digest, p_event_digest
    );
    RETURN QUERY SELECT 'INSERTED'::text, v_ordinal;
END;
$$;

CREATE FUNCTION foreman_execution.record_verification_v1(
    p_task_ref bytea, p_successor_stream_id bytea, p_task_spec_digest bytea,
    p_binding_digest bytea, p_attempt_id text, p_attempt_number smallint,
    p_outcome text, p_verification_profile_digest bytea,
    p_base_commit_digest bytea, p_result_commit_digest bytea, p_tree_digest bytea,
    p_diff_digest bytea, p_result_digest bytea, p_evidence_artifact_digest bytea,
    p_review_digest bytea, p_verified_at text, p_payload_digest bytea,
    p_stream_id bytea, p_event_sequence numeric, p_event_digest bytea,
    p_command_id text, p_request_digest bytea
) RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE v_existing foreman_execution.verification_records%ROWTYPE;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);
    SELECT * INTO v_existing FROM ONLY foreman_execution.verification_records
     WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number;
    IF FOUND THEN
        IF v_existing.attempt_id IS DISTINCT FROM p_attempt_id OR v_existing.successor_stream_id IS DISTINCT FROM p_successor_stream_id
           OR v_existing.task_spec_digest IS DISTINCT FROM p_task_spec_digest OR v_existing.binding_digest IS DISTINCT FROM p_binding_digest
           OR v_existing.outcome IS DISTINCT FROM p_outcome OR v_existing.verification_profile_digest IS DISTINCT FROM p_verification_profile_digest
           OR v_existing.base_commit_digest IS DISTINCT FROM p_base_commit_digest OR v_existing.result_commit_digest IS DISTINCT FROM p_result_commit_digest
           OR v_existing.tree_digest IS DISTINCT FROM p_tree_digest OR v_existing.diff_digest IS DISTINCT FROM p_diff_digest
           OR v_existing.result_digest IS DISTINCT FROM p_result_digest OR v_existing.evidence_artifact_digest IS DISTINCT FROM p_evidence_artifact_digest
           OR v_existing.review_digest IS DISTINCT FROM p_review_digest OR v_existing.verified_at IS DISTINCT FROM p_verified_at
           OR v_existing.payload_digest IS DISTINCT FROM p_payload_digest OR v_existing.ledger_event_digest IS DISTINCT FROM p_event_digest THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_VERIFICATION_SUBSTITUTION';
        END IF;
        PERFORM foreman_execution.assert_exact_child_event_v1(
            'VERIFICATION', p_task_ref, p_stream_id, p_event_sequence,
            p_event_digest, p_command_id, p_request_digest, p_payload_digest,
            'RECORD_TASK_VERIFICATION_V1'
        );
        RETURN 'EXACT_REPLAY';
    END IF;
    IF p_stream_id IS DISTINCT FROM p_successor_stream_id OR NOT EXISTS (
        SELECT 1 FROM ONLY foreman_execution.worker_attempts AS a
         WHERE a.task_ref = p_task_ref AND a.attempt_number = p_attempt_number
           AND a.attempt_id = p_attempt_id AND a.task_spec_digest = p_task_spec_digest
           AND a.binding_digest = p_binding_digest
    ) OR NOT EXISTS (
        SELECT 1 FROM ONLY foreman_execution.worker_observations AS o
         WHERE o.task_ref = p_task_ref AND o.attempt_number = p_attempt_number
           AND o.observation_kind IN ('PRESTART_TERMINAL_FAILED','TERMINAL_COMPLETED','TERMINAL_FAILED','TERMINAL_INTERRUPTED')
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_VERIFICATION_NOT_TERMINAL';
    END IF;
    PERFORM foreman_execution.insert_child_event_v1(
        'VERIFICATION', p_task_ref, p_stream_id, p_event_sequence,
        p_event_digest, p_command_id, p_request_digest, p_payload_digest,
        'RECORD_TASK_VERIFICATION_V1'
    );
    INSERT INTO foreman_execution.verification_records VALUES (
        p_task_ref, p_attempt_number, p_attempt_id, p_successor_stream_id,
        p_task_spec_digest, p_binding_digest, p_outcome,
        p_verification_profile_digest, p_base_commit_digest,
        p_result_commit_digest, p_tree_digest, p_diff_digest, p_result_digest,
        p_evidence_artifact_digest, p_review_digest, p_verified_at,
        p_payload_digest, p_event_digest
    );
    RETURN 'INSERTED';
END;
$$;

CREATE FUNCTION foreman_execution.stage_artifact_reference_v1(
    p_project_id text, p_task_ref bytea, p_attempt_number smallint,
    p_evidence_kind text, p_media_type text, p_payload_schema text,
    p_producer_id text, p_producer_version text, p_producer_digest bytea,
    p_created_at text, p_evidence_bytes bytea, p_content_digest bytea,
    p_descriptor_bytes bytea, p_descriptor_digest bytea,
    p_stream_id bytea, p_before_sequence numeric,
    p_before_last_event_digest bytea, p_before_resource_revision numeric,
    p_before_resource_projection_digest bytea, p_before_head_digest bytea,
    p_event_sequence numeric, p_event_digest bytea, p_command_id text,
    p_request_digest bytea, p_payload_digest bytea, p_correlation_id text,
    p_command_occurred_at text
) RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_retained foreman_execution.artifact_references%ROWTYPE;
    v_staged foreman_execution.staged_artifact_references%ROWTYPE;
    v_ledger_observed boolean;
    v_attempt_count bigint;
    v_attempt_bytes bigint;
    v_task_count bigint;
    v_task_bytes bigint;
    v_evidence_json jsonb;
    v_evidence_text text;
    v_expected_descriptor_bytes bytea;
    v_descriptor_frame bytea;
BEGIN
    IF p_content_digest IS DISTINCT FROM pg_catalog.sha256(p_evidence_bytes) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_ARTIFACT_CONTENT_DIGEST_MISMATCH';
    END IF;
    IF p_media_type <> 'application/json' THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_ARTIFACT_MEDIA_TYPE_REJECTED';
    END IF;
    BEGIN
        v_evidence_text := pg_catalog.convert_from(p_evidence_bytes, 'UTF8');
        v_evidence_json := pg_catalog.convert_from(p_evidence_bytes, 'UTF8')::jsonb;
    EXCEPTION
        WHEN character_not_in_repertoire OR invalid_text_representation THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_ARTIFACT_SECRET_REJECTED';
    END;
    IF v_evidence_json IS NULL
       OR v_evidence_text ~* 'bearer[[:space:]]'
       OR v_evidence_text ~* '"(authorization|password|token|api[_-]?key|private[_-]?key)"[[:space:]]*:'
       OR v_evidence_text ~* '-----begin[^\r\n]*private key-----'
       OR v_evidence_text ~* '://[^/?#[:space:]]*@'
       OR v_evidence_text ~* '(^|[^[:alnum:]])(sk-|gh[pousr]_|github_pat_|glpat-|xox[baprs]-)[[:alnum:]_-]{8,}'
       OR v_evidence_text ~* '(^|[^[:alnum:]])akia[0-9a-z]{16}([^[:alnum:]]|$)' THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_ARTIFACT_SECRET_REJECTED';
    END IF;

    v_expected_descriptor_bytes := pg_catalog.convert_to(
        '{"attempt":' || pg_catalog.to_json(p_attempt_number::text)::text ||
        ',"byte_length":' || pg_catalog.to_json(pg_catalog.octet_length(p_evidence_bytes)::text)::text ||
        ',"content_digest":' || pg_catalog.to_json(pg_catalog.encode(p_content_digest, 'hex'))::text ||
        ',"created_at":' || pg_catalog.to_json(p_created_at)::text ||
        ',"kind":' || pg_catalog.to_json(p_evidence_kind)::text ||
        ',"media_type":' || pg_catalog.to_json(p_media_type)::text ||
        ',"payload_schema":' || pg_catalog.to_json(p_payload_schema)::text ||
        ',"producer_digest":' || pg_catalog.to_json(pg_catalog.encode(p_producer_digest, 'hex'))::text ||
        ',"producer_id":' || pg_catalog.to_json(p_producer_id)::text ||
        ',"producer_version":' || pg_catalog.to_json(p_producer_version)::text ||
        ',"project_id":' || pg_catalog.to_json(p_project_id)::text ||
        ',"record_schema":"lattice.artifact.managed-evidence/1.0"' ||
        ',"task_ref":' || pg_catalog.to_json(pg_catalog.encode(p_task_ref, 'hex'))::text || '}',
        'UTF8'
    );
    v_descriptor_frame :=
        pg_catalog.convert_to('lattice-hash-1', 'UTF8') || decode('00', 'hex') ||
        pg_catalog.int2send(6::smallint) || pg_catalog.convert_to('sha256', 'UTF8') ||
        pg_catalog.int2send(15::smallint) || pg_catalog.convert_to('lattice-cjson-1', 'UTF8') ||
        pg_catalog.int2send(33::smallint) || pg_catalog.convert_to('lattice.artifact.managed-evidence', 'UTF8') ||
        pg_catalog.int2send(3::smallint) || pg_catalog.convert_to('1.0', 'UTF8') ||
        pg_catalog.int8send(pg_catalog.octet_length(v_expected_descriptor_bytes)::bigint) ||
        v_expected_descriptor_bytes;
    IF p_descriptor_bytes IS DISTINCT FROM v_expected_descriptor_bytes
       OR p_descriptor_digest IS DISTINCT FROM pg_catalog.sha256(v_descriptor_frame) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_ARTIFACT_DESCRIPTOR_DIGEST_MISMATCH';
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);
    SELECT * INTO v_retained FROM ONLY foreman_execution.artifact_references
     WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number
       AND descriptor_digest = p_descriptor_digest;
    IF FOUND THEN
        IF v_retained.project_id IS DISTINCT FROM p_project_id OR v_retained.evidence_kind IS DISTINCT FROM p_evidence_kind
           OR v_retained.media_type IS DISTINCT FROM p_media_type OR v_retained.payload_schema IS DISTINCT FROM p_payload_schema
           OR v_retained.producer_id IS DISTINCT FROM p_producer_id OR v_retained.producer_version IS DISTINCT FROM p_producer_version
           OR v_retained.producer_digest IS DISTINCT FROM p_producer_digest OR v_retained.created_at IS DISTINCT FROM p_created_at
           OR v_retained.evidence_bytes IS DISTINCT FROM p_evidence_bytes OR v_retained.content_digest IS DISTINCT FROM p_content_digest
           OR v_retained.ledger_event_digest IS DISTINCT FROM p_event_digest THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_ARTIFACT_SUBSTITUTION';
        END IF;
        PERFORM foreman_execution.assert_exact_child_event_v1(
            'ARTIFACT_REFERENCE', p_task_ref, p_stream_id, p_event_sequence,
            p_event_digest, p_command_id, p_request_digest, p_payload_digest,
            'RECORD_ARTIFACT_REFERENCE_V1'
        );
        IF (SELECT pg_catalog.count(*)
              FROM ONLY control.task_ledger_commands AS command
             WHERE command.stream_id = p_stream_id
               AND command.command_id = p_command_id
               AND command.expected_sequence = p_before_sequence
               AND command.expected_last_event_digest = p_before_last_event_digest
               AND command.expected_resource_revision = p_before_resource_revision
               AND command.expected_resource_projection_digest = p_before_resource_projection_digest
               AND command.expected_head_digest = p_before_head_digest
               AND command.correlation_id = p_correlation_id
               AND command.occurred_at = p_command_occurred_at
               AND command.request_digest = p_request_digest
               AND command.subject_digest = p_payload_digest
               AND command.action_id = 'RECORD_ARTIFACT_REFERENCE_V1'
               AND command.command_outcome = 'APPENDED'
               AND command.event_digest = p_event_digest) <> 1 THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_ARTIFACT_STAGE_SUBSTITUTION';
        END IF;
        RETURN 'EXACT_REPLAY';
    END IF;

    SELECT * INTO v_staged FROM ONLY foreman_execution.staged_artifact_references
     WHERE task_ref = p_task_ref;
    IF FOUND THEN
        IF v_staged.project_id IS DISTINCT FROM p_project_id
           OR v_staged.attempt_number IS DISTINCT FROM p_attempt_number
           OR v_staged.evidence_kind IS DISTINCT FROM p_evidence_kind
           OR v_staged.media_type IS DISTINCT FROM p_media_type
           OR v_staged.payload_schema IS DISTINCT FROM p_payload_schema
           OR v_staged.producer_id IS DISTINCT FROM p_producer_id
           OR v_staged.producer_version IS DISTINCT FROM p_producer_version
           OR v_staged.producer_digest IS DISTINCT FROM p_producer_digest
           OR v_staged.created_at IS DISTINCT FROM p_created_at
           OR v_staged.evidence_bytes IS DISTINCT FROM p_evidence_bytes
           OR v_staged.content_digest IS DISTINCT FROM p_content_digest
           OR v_staged.descriptor_digest IS DISTINCT FROM p_descriptor_digest
           OR v_staged.ledger_stream_id IS DISTINCT FROM p_stream_id
           OR v_staged.before_sequence IS DISTINCT FROM p_before_sequence
           OR v_staged.before_last_event_digest IS DISTINCT FROM p_before_last_event_digest
           OR v_staged.before_resource_revision IS DISTINCT FROM p_before_resource_revision
           OR v_staged.before_resource_projection_digest IS DISTINCT FROM p_before_resource_projection_digest
           OR v_staged.before_head_digest IS DISTINCT FROM p_before_head_digest
           OR v_staged.ledger_event_sequence IS DISTINCT FROM p_event_sequence
           OR v_staged.ledger_event_digest IS DISTINCT FROM p_event_digest
           OR v_staged.ledger_command_id IS DISTINCT FROM p_command_id
           OR v_staged.ledger_request_digest IS DISTINCT FROM p_request_digest
           OR v_staged.ledger_payload_digest IS DISTINCT FROM p_payload_digest
           OR v_staged.correlation_id IS DISTINCT FROM p_correlation_id
           OR v_staged.command_occurred_at IS DISTINCT FROM p_command_occurred_at THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_ARTIFACT_STAGE_SUBSTITUTION';
        END IF;
        RETURN 'EXACT_REPLAY';
    END IF;

    IF NOT (
        EXISTS (SELECT 1 FROM ONLY foreman_execution.worker_attempts AS a
            JOIN ONLY foreman_execution.task_promotions AS p ON p.task_ref = a.task_ref
            WHERE a.task_ref = p_task_ref AND a.attempt_number = p_attempt_number
              AND a.successor_stream_id = p_stream_id
              AND p.project_id = p_project_id)
        OR EXISTS (
            SELECT 1
              FROM ONLY foreman_execution.pending_worker_claims AS pending
              JOIN ONLY foreman_execution.task_promotions AS promotion
                ON promotion.task_ref = pending.task_ref
             WHERE pending.task_ref = p_task_ref
               AND pending.attempt_number = p_attempt_number
               AND pending.successor_stream_id = p_stream_id
               AND promotion.project_id = p_project_id
               AND p_evidence_kind = 'WORKER_LIFECYCLE'
               AND p_media_type = 'application/json'
               AND p_payload_schema = 'lattice.managed-blocker.v1'
               AND p_producer_id = 'lattice-foreman'
               AND p_producer_version = '1'
               AND p_producer_digest = pending.foreman_checkpoint_digest
               AND pg_catalog.convert_from(p_evidence_bytes, 'UTF8')::jsonb
                    ->> 'schema' = 'lattice.managed-blocker.v1'
               AND (pg_catalog.convert_from(p_evidence_bytes, 'UTF8')::jsonb
                    ->> 'attempt')::smallint = p_attempt_number
               AND pg_catalog.convert_from(p_evidence_bytes, 'UTF8')::jsonb
                    ->> 'code' IN (
                        'LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT',
                        'LATTICE_MANAGED_PRESTART_CONFIGURATION_REJECTED',
                        'LATTICE_MANAGED_MODEL_UNAVAILABLE',
                        'LATTICE_MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED'
                    )
        )
    )
       OR p_payload_digest IS DISTINCT FROM p_descriptor_digest
       OR p_event_sequence IS DISTINCT FROM p_before_sequence + 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_ARTIFACT_ATTEMPT_MISMATCH';
    END IF;

    SELECT EXISTS (
        SELECT 1 FROM ONLY control.task_ledger_events AS event
         WHERE event.event_digest = p_event_digest
            OR (event.stream_id = p_stream_id AND event.sequence = p_event_sequence)
            OR (event.stream_id = p_stream_id AND event.command_id = p_command_id)
    ) INTO v_ledger_observed;
    IF v_ledger_observed THEN
        PERFORM foreman_execution.assert_task_ledger_event_v1(
            p_stream_id, p_event_sequence, p_event_digest, p_command_id,
            p_request_digest, p_payload_digest, 'RECORD_ARTIFACT_REFERENCE_V1'
        );
        IF (SELECT pg_catalog.count(*)
              FROM ONLY control.task_ledger_commands AS command
             WHERE command.stream_id = p_stream_id
               AND command.command_id = p_command_id
               AND command.expected_sequence = p_before_sequence
               AND command.expected_last_event_digest = p_before_last_event_digest
               AND command.expected_resource_revision = p_before_resource_revision
               AND command.expected_resource_projection_digest = p_before_resource_projection_digest
               AND command.expected_head_digest = p_before_head_digest
               AND command.correlation_id = p_correlation_id
               AND command.occurred_at = p_command_occurred_at
               AND command.request_digest = p_request_digest
               AND command.subject_digest = p_payload_digest
               AND command.action_id = 'RECORD_ARTIFACT_REFERENCE_V1'
               AND command.command_outcome = 'APPENDED'
               AND command.event_digest = p_event_digest) <> 1 THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_ARTIFACT_STAGE_LEDGER_HEAD_MISMATCH';
        END IF;
    ELSIF EXISTS (
        SELECT 1 FROM ONLY control.task_ledger_commands AS command
         WHERE command.stream_id = p_stream_id AND command.command_id = p_command_id
    ) OR (SELECT pg_catalog.count(*)
            FROM ONLY control.task_ledger_streams AS stream
           WHERE stream.stream_id = p_stream_id
             AND stream.sequence = p_before_sequence
             AND stream.last_event_digest = p_before_last_event_digest
             AND stream.resource_revision = p_before_resource_revision
             AND stream.resource_projection_digest = p_before_resource_projection_digest
             AND stream.head_digest = p_before_head_digest) <> 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_ARTIFACT_STAGE_LEDGER_HEAD_MISMATCH';
    END IF;

    SELECT pg_catalog.count(*),
           COALESCE(pg_catalog.sum(pg_catalog.octet_length(reference.evidence_bytes)), 0)
      INTO v_attempt_count, v_attempt_bytes
      FROM (
          SELECT retained.attempt_number, retained.evidence_bytes
            FROM ONLY foreman_execution.artifact_references AS retained
           WHERE retained.task_ref = p_task_ref
          UNION ALL
          SELECT staged.attempt_number, staged.evidence_bytes
            FROM ONLY foreman_execution.staged_artifact_references AS staged
           WHERE staged.task_ref = p_task_ref
      ) AS reference
     WHERE reference.attempt_number = p_attempt_number;
    IF v_attempt_count >= 64
       OR v_attempt_bytes + pg_catalog.octet_length(p_evidence_bytes) > 8388608 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_ARTIFACT_ATTEMPT_QUOTA_EXHAUSTED';
    END IF;
    SELECT pg_catalog.count(*),
           COALESCE(pg_catalog.sum(pg_catalog.octet_length(reference.evidence_bytes)), 0)
      INTO v_task_count, v_task_bytes
      FROM (
          SELECT retained.evidence_bytes
            FROM ONLY foreman_execution.artifact_references AS retained
           WHERE retained.task_ref = p_task_ref
          UNION ALL
          SELECT staged.evidence_bytes
            FROM ONLY foreman_execution.staged_artifact_references AS staged
           WHERE staged.task_ref = p_task_ref
      ) AS reference;
    IF v_task_count >= 192
       OR v_task_bytes + pg_catalog.octet_length(p_evidence_bytes) > 25165824 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_ARTIFACT_TASK_QUOTA_EXHAUSTED';
    END IF;
    INSERT INTO foreman_execution.staged_artifact_references (
        task_ref, project_id, attempt_number, evidence_kind, media_type,
        payload_schema, producer_id, producer_version, producer_digest,
        created_at, evidence_bytes, content_digest, descriptor_digest,
        ledger_stream_id, before_sequence, before_last_event_digest,
        before_resource_revision, before_resource_projection_digest,
        before_head_digest, ledger_event_sequence, ledger_event_digest,
        ledger_command_id, ledger_request_digest, ledger_payload_digest,
        correlation_id, command_occurred_at
    ) VALUES (
        p_task_ref, p_project_id, p_attempt_number, p_evidence_kind, p_media_type,
        p_payload_schema, p_producer_id, p_producer_version, p_producer_digest,
        p_created_at, p_evidence_bytes, p_content_digest, p_descriptor_digest,
        p_stream_id, p_before_sequence, p_before_last_event_digest,
        p_before_resource_revision, p_before_resource_projection_digest,
        p_before_head_digest, p_event_sequence, p_event_digest, p_command_id,
        p_request_digest, p_payload_digest, p_correlation_id,
        p_command_occurred_at
    );
    RETURN 'INSERTED';
END;
$$;

CREATE FUNCTION foreman_execution.finalize_staged_artifact_reference_v1(
    p_task_ref bytea, p_attempt_number smallint, p_descriptor_digest bytea
) RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_retained foreman_execution.artifact_references%ROWTYPE;
    v_staged foreman_execution.staged_artifact_references%ROWTYPE;
    v_child foreman_execution.child_events%ROWTYPE;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);
    SELECT * INTO v_retained FROM ONLY foreman_execution.artifact_references
     WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number
       AND descriptor_digest = p_descriptor_digest;
    IF FOUND THEN
        SELECT * INTO v_child FROM ONLY foreman_execution.child_events
         WHERE ledger_event_digest = v_retained.ledger_event_digest;
        IF NOT FOUND THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_CHILD_EVENT_REPLAY_MISMATCH';
        END IF;
        PERFORM foreman_execution.assert_exact_child_event_v1(
            'ARTIFACT_REFERENCE', p_task_ref, v_child.ledger_stream_id,
            v_child.ledger_event_sequence, v_child.ledger_event_digest,
            v_child.ledger_command_id, v_child.ledger_request_digest,
            v_child.ledger_payload_digest, 'RECORD_ARTIFACT_REFERENCE_V1'
        );
        RETURN 'EXACT_REPLAY';
    END IF;

    SELECT * INTO v_staged FROM ONLY foreman_execution.staged_artifact_references
     WHERE task_ref = p_task_ref;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_ARTIFACT_STAGE_REQUIRED';
    END IF;
    IF v_staged.descriptor_digest IS DISTINCT FROM p_descriptor_digest THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_ARTIFACT_STAGE_SUBSTITUTION';
    END IF;
    IF v_staged.attempt_number IS DISTINCT FROM p_attempt_number THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_ARTIFACT_STAGE_SUBSTITUTION';
    END IF;

    PERFORM foreman_execution.insert_child_event_v1(
        'ARTIFACT_REFERENCE', v_staged.task_ref, v_staged.ledger_stream_id,
        v_staged.ledger_event_sequence, v_staged.ledger_event_digest,
        v_staged.ledger_command_id, v_staged.ledger_request_digest,
        v_staged.ledger_payload_digest, 'RECORD_ARTIFACT_REFERENCE_V1'
    );
    INSERT INTO foreman_execution.artifact_references (
        project_id, task_ref, attempt_number, evidence_kind, media_type,
        payload_schema, producer_id, producer_version, producer_digest,
        created_at, evidence_bytes, content_digest, descriptor_digest,
        ledger_event_digest
    ) VALUES (
        v_staged.project_id, v_staged.task_ref, v_staged.attempt_number,
        v_staged.evidence_kind, v_staged.media_type, v_staged.payload_schema,
        v_staged.producer_id, v_staged.producer_version,
        v_staged.producer_digest, v_staged.created_at,
        v_staged.evidence_bytes, v_staged.content_digest,
        v_staged.descriptor_digest, v_staged.ledger_event_digest
    );
    DELETE FROM ONLY foreman_execution.staged_artifact_references
     WHERE task_ref = v_staged.task_ref
       AND descriptor_digest = v_staged.descriptor_digest;
    RETURN 'INSERTED';
END;
$$;

CREATE FUNCTION foreman_execution.claim_provider_dispatch_v1(
    p_task_ref bytea, p_attempt_number smallint, p_operation_kind text,
    p_attempt_id text, p_binding_digest bytea, p_writer_fence bigint,
    p_foreman_generation bigint, p_foreman_checkpoint_digest bytea,
    p_anchor_digest bytea, p_supporting_digest bytea, p_subject_digest bytea,
    p_dispatch_digest bytea, p_foreman_stream_id bytea
) RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_attempt foreman_execution.worker_attempts%ROWTYPE;
    v_environment foreman_execution.execution_environments%ROWTYPE;
    v_existing foreman_execution.provider_dispatch_claims%ROWTYPE;
    v_admission control.runtime_admission%ROWTYPE;
    v_claimed_at_text text;
    v_claim_receipt_digest bytea;
    v_existing_claimed_at_text text;
    v_existing_receipt_digest bytea;
BEGIN
    IF p_attempt_number NOT BETWEEN 1 AND 3
       OR p_operation_kind NOT IN ('WORKER_THREAD', 'WORKER_TURN', 'REVIEW_THREAD', 'REVIEW_TURN')
       OR p_writer_fence <= 0 OR p_foreman_generation <= 0
       OR p_attempt_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR pg_catalog.octet_length(p_task_ref) <> 32
       OR pg_catalog.octet_length(p_binding_digest) <> 32
       OR pg_catalog.octet_length(p_foreman_checkpoint_digest) <> 32
       OR pg_catalog.octet_length(p_anchor_digest) <> 32
       OR pg_catalog.octet_length(p_supporting_digest) <> 32
       OR pg_catalog.octet_length(p_subject_digest) <> 32
       OR pg_catalog.octet_length(p_dispatch_digest) <> 32
       OR pg_catalog.octet_length(p_foreman_stream_id) <> 32
       OR p_task_ref = decode(repeat('00', 32), 'hex')
       OR p_binding_digest = decode(repeat('00', 32), 'hex')
       OR p_foreman_checkpoint_digest = decode(repeat('00', 32), 'hex')
       OR p_anchor_digest = decode(repeat('00', 32), 'hex')
       OR p_supporting_digest = decode(repeat('00', 32), 'hex')
       OR p_subject_digest = decode(repeat('00', 32), 'hex')
       OR p_dispatch_digest = decode(repeat('00', 32), 'hex')
       OR p_foreman_stream_id = decode(repeat('00', 32), 'hex') THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_INPUT_REJECTED';
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);
    v_claimed_at_text := pg_catalog.to_char(
        pg_catalog.transaction_timestamp() AT TIME ZONE 'UTC',
        'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'
    );
    v_claim_receipt_digest := pg_catalog.sha256(
        pg_catalog.convert_to('LATTICE_FOREMAN_PROVIDER_DISPATCH_RECEIPT_V1', 'UTF8')
        || pg_catalog.decode('00', 'hex')
        || pg_catalog.convert_to(pg_catalog.encode(p_dispatch_digest, 'hex'), 'UTF8')
        || pg_catalog.decode('00', 'hex')
        || pg_catalog.convert_to(v_claimed_at_text, 'UTF8')
    );
    SELECT * INTO v_attempt FROM ONLY foreman_execution.worker_attempts AS attempt
     WHERE attempt.task_ref = p_task_ref
       AND attempt.attempt_number = p_attempt_number;
    IF NOT FOUND
       OR v_attempt.attempt_id IS DISTINCT FROM p_attempt_id
       OR v_attempt.binding_digest IS DISTINCT FROM p_binding_digest
       OR v_attempt.writer_fence IS DISTINCT FROM p_writer_fence
       OR v_attempt.foreman_generation IS DISTINCT FROM p_foreman_generation
       OR v_attempt.foreman_checkpoint_digest IS DISTINCT FROM p_foreman_checkpoint_digest THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_ATTEMPT_MISMATCH';
    END IF;
    SELECT environment.* INTO v_environment
      FROM ONLY foreman_execution.execution_environments AS environment
     WHERE environment.task_ref = p_task_ref
       AND environment.attempt_number = p_attempt_number;
    IF v_attempt.execution_environment_ref =
       'execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000001' THEN
        IF FOUND THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_EXECUTION_ENVIRONMENT_NOT_CURRENT';
        END IF;
        BEGIN
            PERFORM pg_catalog.count(*)
              FROM foreman_execution.read_execution_environment_rows_v1(p_task_ref);
        EXCEPTION WHEN SQLSTATE 'P0001' THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_EXECUTION_ENVIRONMENT_NOT_CURRENT';
        END;
    ELSE
        IF NOT FOUND
           OR v_environment.environment_ref IS DISTINCT FROM
                v_attempt.execution_environment_ref
           OR v_environment.attempt_id IS DISTINCT FROM v_attempt.attempt_id
           OR v_environment.packet_digest IS DISTINCT FROM v_attempt.packet_digest THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_EXECUTION_ENVIRONMENT_NOT_CURRENT';
        END IF;
        BEGIN
            PERFORM 1
              FROM foreman_execution.read_execution_environment_rows_v1(p_task_ref) AS environment
             WHERE environment.attempt_number = p_attempt_number
               AND environment.environment_ref = v_attempt.execution_environment_ref;
            IF NOT FOUND THEN
                RAISE EXCEPTION USING ERRCODE = 'P0001',
                    MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_EXECUTION_ENVIRONMENT_NOT_CURRENT';
            END IF;
        EXCEPTION WHEN SQLSTATE 'P0001' THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_EXECUTION_ENVIRONMENT_NOT_CURRENT';
        END;
    END IF;
    -- Exact replay is historical reconciliation authority, not a new provider
    -- effect. Validate every immutable claim field first, then return before
    -- consulting the latest Foreman generation or the current Writer expiry.
    -- A later generation and an expired/recovered Writer must not make an
    -- already claimed exact thread/turn undiscoverable after restart.
    SELECT * INTO v_existing FROM ONLY foreman_execution.provider_dispatch_claims AS dispatch
     WHERE dispatch.task_ref = p_task_ref
       AND dispatch.attempt_number = p_attempt_number
       AND dispatch.operation_kind = p_operation_kind;
    IF FOUND THEN
        v_existing_claimed_at_text := pg_catalog.to_char(
            v_existing.claimed_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'
        );
        v_existing_receipt_digest := pg_catalog.sha256(
            pg_catalog.convert_to('LATTICE_FOREMAN_PROVIDER_DISPATCH_RECEIPT_V1', 'UTF8')
            || pg_catalog.decode('00', 'hex')
            || pg_catalog.convert_to(pg_catalog.encode(v_existing.dispatch_digest, 'hex'), 'UTF8')
            || pg_catalog.decode('00', 'hex')
            || pg_catalog.convert_to(v_existing_claimed_at_text, 'UTF8')
        );
        IF v_existing.attempt_id IS DISTINCT FROM p_attempt_id
           OR v_existing.binding_digest IS DISTINCT FROM p_binding_digest
           OR v_existing.writer_fence IS DISTINCT FROM p_writer_fence
           OR v_existing.foreman_generation IS DISTINCT FROM p_foreman_generation
           OR v_existing.foreman_checkpoint_digest IS DISTINCT FROM p_foreman_checkpoint_digest
           OR v_existing.anchor_digest IS DISTINCT FROM p_anchor_digest
           OR v_existing.supporting_digest IS DISTINCT FROM p_supporting_digest
           OR v_existing.subject_digest IS DISTINCT FROM p_subject_digest
           OR v_existing.dispatch_digest IS DISTINCT FROM p_dispatch_digest
           OR v_existing.claim_receipt_digest IS DISTINCT FROM v_existing_receipt_digest THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_SUBSTITUTION';
        END IF;
        RETURN 'EXACT_REPLAY';
    END IF;
    -- A new provider effect is admitted only while the exact execution
    -- authority retained for this attempt and the Project Registry identity
    -- captured by the promotion are both current.  Share-lock all three rows
    -- through claim insertion so Registry drift, approval substitution, or
    -- authority expiry cannot race this check. Historical exact replay above
    -- remains discoverable after authority expiry because it creates no new
    -- provider effect.
    PERFORM 1
      FROM ONLY foreman_execution.task_promotions AS promotion
      JOIN ONLY foreman_execution.approval_evidence AS authority
        ON authority.task_ref = promotion.task_ref
       AND authority.authority_digest = v_attempt.approval_receipt_digest
       AND authority.successor_stream_id = v_attempt.successor_stream_id
       AND authority.task_spec_digest = v_attempt.task_spec_digest
       AND authority.approval_subject_digest = promotion.approval_subject_digest
       AND authority.budget_digest = v_attempt.budget_digest
      JOIN ONLY control.project_registry_projects AS project
        ON project.project_id = promotion.project_id
       AND project.authority_snapshot_id = promotion.project_snapshot_id
       AND project.authority_receipt_digest = promotion.project_authority_receipt_digest
     WHERE promotion.task_ref = p_task_ref
       AND promotion.successor_stream_id = v_attempt.successor_stream_id
       AND promotion.task_spec_digest = v_attempt.task_spec_digest
       AND promotion.binding_digest = v_attempt.binding_digest
       AND promotion.budget_digest = v_attempt.budget_digest
       AND v_attempt.approval_receipt_digest = authority.authority_digest
       AND authority.capability = 'LOCAL_REVERSIBLE_TASK_EXECUTION'
       AND (
           (authority.authority_source = 'VERIFIED_APPROVAL'
            AND authority.approval_receipt_digest IS NOT NULL)
           OR
           (authority.authority_source = 'CLOSED_POLICY_NO_APPROVAL_REQUIRED'
            AND authority.approval_receipt_digest IS NULL)
       )
       AND pg_catalog.clock_timestamp() >= authority.issued_at::timestamp with time zone
       AND pg_catalog.clock_timestamp() < authority.expires_at::timestamp with time zone
       AND project.project_class = 'USER_PROJECT'
       AND project.authority_contract_version = 1
       AND project.authority_producer_id = 'lattice-project-registry'
       AND project.authority_producer_version = '1.0'
       AND project.authority_runtime = 'LIVE'
       AND project.authority_lifecycle = 'ACTIVE'
       AND project.pending_observation_digest IS NULL
       AND NOT project.drift_canonical_root
       AND NOT project.drift_repository
       AND NOT project.drift_file
       AND NOT project.drift_primary_ref_name
       AND NOT project.drift_primary_ref_storage
       AND project.authority_observation_digest = project.accepted_observation_digest
     FOR SHARE OF authority, promotion, project;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_AUTHORITY_NOT_CURRENT';
    END IF;
    -- The provider-effect fence is also subordinate to the one fixed durable
    -- Task Ledger Foreman stream. The caller supplies only the Rust-derived
    -- stream digest; this transaction independently proves its complete fixed
    -- identity, current checkpoint, and latest SoleForeman ACTIVE generation.
    PERFORM 1
          FROM ONLY control.task_ledger_streams AS foreman_stream
          JOIN ONLY control.task_ledger_foreman_snapshots AS foreman_snapshot
            ON foreman_snapshot.stream_id = foreman_stream.stream_id
           AND foreman_snapshot.event_sequence = foreman_stream.sequence
         WHERE foreman_stream.stream_id = p_foreman_stream_id
           AND foreman_stream.ledger_schema_version = '2.0'
           AND foreman_stream.producer_id = 'lattice-task-ledger'
           AND foreman_stream.producer_version = '2.0'
           AND foreman_stream.runtime = 'LIVE'
           AND foreman_stream.project_id = 'lattice-control'
           AND foreman_stream.project_snapshot_id = 'foreman-coordination-v1'
           AND foreman_stream.task_id = 'TASK-FOREMAN-COORDINATION'
           AND foreman_stream.task_revision = 1
           AND foreman_stream.task_subject_kind = 'TASK_SPEC'
           AND foreman_stream.task_subject_digest = decode(repeat('79', 32), 'hex')
           AND foreman_stream.task_spec_digest = decode(repeat('79', 32), 'hex')
           AND foreman_stream.accounting_currency = 'USD'
           AND foreman_stream.checkpoint_digest = p_foreman_checkpoint_digest
           AND foreman_snapshot.generation = p_foreman_generation
           AND foreman_snapshot.foreman_state = 'ACTIVE'
           AND foreman_snapshot.worker_id = 'sole-foreman-v1'
           AND foreman_snapshot.thread_id = 'lattice-devos-sole-foreman-v1'
           AND foreman_snapshot.task_id = 'TASK-FOREMAN-COORDINATION'
           AND NOT EXISTS (
               SELECT 1
                 FROM ONLY control.task_ledger_foreman_snapshots AS newer_snapshot
                WHERE newer_snapshot.stream_id = foreman_snapshot.stream_id
                   AND newer_snapshot.generation > foreman_snapshot.generation
            )
          FOR SHARE OF foreman_stream;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_FOREMAN_FENCE_STALE';
    END IF;
    -- The effect claim is admissible only while the durable Writer head that
    -- fenced this exact task, attempt, and worktree remains current.  This is
    -- intentionally in the same transaction as the claim insertion so a
    -- writer rotation between local preparation and provider RPC fails closed.
    SELECT admission.* INTO v_admission
      FROM ONLY control.runtime_admission AS admission
     WHERE admission.singleton
     FOR SHARE OF admission;
    IF NOT FOUND
       OR v_admission.admission_mode IS DISTINCT FROM 'ACTIVE' THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_WRITER_FENCE_STALE';
    END IF;
    PERFORM 1
          FROM ONLY foreman_execution.task_promotions AS promotion
          JOIN ONLY writer_lease.writer_lease_heads AS lease
            ON lease.project_id = promotion.project_id
           AND lease.current_status = 'ACTIVE'
           AND lease.current_project_snapshot_id = promotion.project_snapshot_id
           AND lease.current_task_spec_digest = v_attempt.task_spec_digest
           AND lease.current_attempt_id = v_attempt.attempt_id
           AND lease.current_lease_holder_id = 'lattice-foreman'
           AND lease.current_worktree_id =
               'WORK-' || pg_catalog.upper(pg_catalog.substr(
                   pg_catalog.encode(v_attempt.task_ref, 'hex'), 1, 59
               ))
           AND lease.current_lease_id =
               'managed-lease-' || pg_catalog.encode(v_attempt.task_ref, 'hex') ||
               '-' || v_attempt.attempt_number::text
            AND lease.current_fencing_token = v_attempt.writer_fence
            AND lease.current_expires_at::timestamp with time zone > pg_catalog.clock_timestamp()
            AND lease.current_daemon_instance_id = v_admission.daemon_instance_id
            AND lease.current_daemon_epoch = v_admission.daemon_epoch
         WHERE promotion.task_ref = p_task_ref
         FOR SHARE OF lease;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_WRITER_FENCE_STALE';
    END IF;
    IF p_operation_kind = 'WORKER_THREAD' AND (
        v_attempt.payload_digest IS DISTINCT FROM p_anchor_digest
        OR v_attempt.packet_digest IS DISTINCT FROM p_supporting_digest
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_ANCHOR_MISMATCH';
    ELSIF p_operation_kind = 'WORKER_TURN' AND NOT EXISTS (
        SELECT 1 FROM ONLY foreman_execution.worker_observations AS observed
         WHERE observed.task_ref = p_task_ref
           AND observed.attempt_number = p_attempt_number
           AND observed.observation_kind = 'THREAD_ACCEPTED'
           AND observed.turn_id IS NULL
           AND observed.payload_digest = p_anchor_digest
           AND observed.evidence_digest = p_supporting_digest
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_ANCHOR_MISMATCH';
    ELSIF p_operation_kind = 'REVIEW_THREAD' AND (
        NOT EXISTS (
            SELECT 1 FROM ONLY foreman_execution.worker_observations AS terminal
             WHERE terminal.task_ref = p_task_ref
               AND terminal.attempt_number = p_attempt_number
               AND terminal.observation_kind = 'TERMINAL_COMPLETED'
               AND terminal.payload_digest = p_anchor_digest
        ) OR NOT EXISTS (
            SELECT 1 FROM ONLY foreman_execution.artifact_references AS artifact
             WHERE artifact.task_ref = p_task_ref
               AND artifact.attempt_number = p_attempt_number
               AND artifact.evidence_kind = 'GIT_SNAPSHOT'
               AND artifact.descriptor_digest = p_supporting_digest
        )
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_ANCHOR_MISMATCH';
    ELSIF p_operation_kind = 'REVIEW_TURN' AND (
        NOT EXISTS (
            SELECT 1 FROM ONLY foreman_execution.artifact_references AS lifecycle
             WHERE lifecycle.task_ref = p_task_ref
               AND lifecycle.attempt_number = p_attempt_number
               AND lifecycle.evidence_kind = 'WORKER_LIFECYCLE'
               AND lifecycle.payload_schema = 'lattice.managed-review-lifecycle/1.0'
               AND lifecycle.producer_id = 'lattice-managed-semantic-reviewer'
               AND lifecycle.producer_version = '1.0'
               AND lifecycle.descriptor_digest = p_anchor_digest
        ) OR NOT EXISTS (
            SELECT 1 FROM ONLY foreman_execution.provider_dispatch_claims AS review
             WHERE review.task_ref = p_task_ref
               AND review.attempt_number = p_attempt_number
               AND review.operation_kind = 'REVIEW_THREAD'
               AND review.supporting_digest = p_supporting_digest
        )
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_ANCHOR_MISMATCH';
    END IF;
    IF EXISTS (
        SELECT 1 FROM ONLY foreman_execution.attempt_closures AS closure
         WHERE closure.task_ref = p_task_ref
           AND closure.attempt_number = p_attempt_number
    ) OR EXISTS (
        SELECT 1 FROM ONLY foreman_execution.verification_records AS verified
         WHERE verified.task_ref = p_task_ref
           AND verified.attempt_number = p_attempt_number
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PROVIDER_DISPATCH_ATTEMPT_CLOSED';
    END IF;
    INSERT INTO foreman_execution.provider_dispatch_claims (
        task_ref, attempt_number, operation_kind, attempt_id, binding_digest,
        writer_fence, foreman_generation, foreman_checkpoint_digest,
        anchor_digest, supporting_digest, subject_digest, dispatch_digest,
        claim_receipt_digest, claimed_at
    ) VALUES (
        p_task_ref, p_attempt_number, p_operation_kind, p_attempt_id,
        p_binding_digest, p_writer_fence, p_foreman_generation,
        p_foreman_checkpoint_digest, p_anchor_digest, p_supporting_digest,
        p_subject_digest, p_dispatch_digest, v_claim_receipt_digest,
        v_claimed_at_text::timestamp with time zone
    );
    RETURN 'CLAIMED';
END;
$$;

CREATE FUNCTION foreman_execution.read_provider_dispatch_claim_v1(
    p_task_ref bytea, p_attempt_number smallint, p_operation_kind text
) RETURNS TABLE(
    attempt_id text,
    binding_digest bytea,
    writer_fence bigint,
    foreman_generation bigint,
    foreman_checkpoint_digest bytea,
    anchor_digest bytea,
    supporting_digest bytea,
    subject_digest bytea,
    dispatch_digest bytea,
    claim_receipt_digest bytea,
    claimed_at text
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT dispatch.attempt_id::text, dispatch.binding_digest,
           dispatch.writer_fence, dispatch.foreman_generation,
           dispatch.foreman_checkpoint_digest, dispatch.anchor_digest,
           dispatch.supporting_digest, dispatch.subject_digest,
           dispatch.dispatch_digest, dispatch.claim_receipt_digest,
           pg_catalog.to_char(
               dispatch.claimed_at AT TIME ZONE 'UTC',
               'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'
           )
      FROM ONLY foreman_execution.provider_dispatch_claims AS dispatch
     WHERE dispatch.task_ref = p_task_ref
       AND dispatch.attempt_number = p_attempt_number
       AND dispatch.operation_kind = p_operation_kind
$$;

CREATE FUNCTION foreman_execution.record_attempt_closure_v1(
    p_task_ref bytea, p_attempt_number smallint, p_blocker_code text,
    p_blocker_descriptor_digest bytea, p_writer_fence bigint
) RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_existing foreman_execution.attempt_closures%ROWTYPE;
    v_artifact foreman_execution.artifact_references%ROWTYPE;
    v_payload jsonb;
    v_expected_reason text;
    v_expected_retryable boolean;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);
    PERFORM pg_catalog.count(*)
      FROM foreman_execution.read_execution_environment_rows_v1(p_task_ref);
    SELECT * INTO v_existing FROM ONLY foreman_execution.attempt_closures
     WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number;
    IF FOUND THEN
        IF v_existing.provider_disposition <> 'PROVEN_INACTIVE'
           OR v_existing.blocker_code IS DISTINCT FROM p_blocker_code
           OR v_existing.blocker_descriptor_digest IS DISTINCT FROM p_blocker_descriptor_digest
           OR v_existing.reconciliation_proof_descriptor_digest IS NOT NULL
           OR v_existing.writer_fence IS DISTINCT FROM p_writer_fence THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_ATTEMPT_CLOSURE_SUBSTITUTION';
        END IF;
        RETURN 'EXACT_REPLAY';
    END IF;
    SELECT * INTO v_artifact FROM ONLY foreman_execution.artifact_references AS artifact
     WHERE artifact.task_ref = p_task_ref
       AND artifact.attempt_number = p_attempt_number
       AND artifact.descriptor_digest = p_blocker_descriptor_digest
       AND artifact.evidence_kind = 'WORKER_LIFECYCLE'
       AND artifact.payload_schema = 'lattice.managed-blocker.v1'
       AND artifact.producer_id = 'lattice-foreman'
       AND artifact.producer_version = '1';
    IF NOT FOUND OR NOT EXISTS (
        SELECT 1 FROM ONLY foreman_execution.worker_attempts AS a
         WHERE a.task_ref = p_task_ref AND a.attempt_number = p_attempt_number
           AND a.writer_fence = p_writer_fence
           AND a.foreman_checkpoint_digest = v_artifact.producer_digest
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_ATTEMPT_CLOSURE_BINDING_MISMATCH';
    END IF;
    v_payload := pg_catalog.convert_from(v_artifact.evidence_bytes, 'UTF8')::jsonb;
    v_expected_reason := CASE p_blocker_code
        WHEN 'LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT' THEN 'TASK_BOUND_EXECUTION_AUTHORITY_NOT_CURRENT'
        WHEN 'LATTICE_MANAGED_PRESTART_CONFIGURATION_REJECTED' THEN 'TRUSTED_WORKER_OR_VERIFIER_CONFIGURATION_REJECTED_BEFORE_PROVIDER_EFFECT'
        WHEN 'LATTICE_MANAGED_HEARTBEAT_TIMEOUT_WHILE_IN_PROGRESS' THEN 'HEARTBEAT_TIMEOUT_EXACT_TURN_STILL_IN_PROGRESS'
        WHEN 'LATTICE_MANAGED_DEADLINE_EXCEEDED' THEN 'DEADLINE_REACHED_BEFORE_EXACT_TERMINAL'
        WHEN 'LATTICE_MANAGED_MODEL_UNAVAILABLE' THEN 'SELECTED_ALLOWLISTED_MODEL_UNAVAILABLE_NO_SUBSTITUTION'
        WHEN 'LATTICE_MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED' THEN 'WORKER_MODEL_PROBE_TIMED_OUT_EXACT_PRESTART_SUBTREE_REAPED'
        WHEN 'LATTICE_MANAGED_REVIEW_MODEL_PROBE_TIMEOUT_NO_PROVIDER_EFFECT' THEN 'REVIEW_MODEL_PROBE_TIMED_OUT_NO_REVIEW_PROVIDER_EFFECT'
        WHEN 'LATTICE_MANAGED_RETRY_BUDGET_EXHAUSTED' THEN 'ATTEMPT_ONE_PLUS_TWO_REPAIRS_EXHAUSTED'
        WHEN 'LATTICE_MANAGED_VERIFICATION_FAILED' THEN 'INDEPENDENT_VERIFICATION_FAILED'
        WHEN 'LATTICE_MANAGED_REVIEW_RESULT_REJECTED' THEN 'REVIEW_RESULT_OR_EVIDENCE_FAILED_CLOSED'
        WHEN 'LATTICE_MANAGED_TOKEN_BUDGET_EXHAUSTED' THEN 'CUMULATIVE_TOKEN_BUDGET_EXHAUSTED'
        WHEN 'LATTICE_MANAGED_MODEL_CALL_BUDGET_EXHAUSTED' THEN 'CUMULATIVE_MODEL_CALL_BUDGET_EXHAUSTED'
        WHEN 'LATTICE_MANAGED_MODEL_USAGE_RECONCILIATION_REQUIRED' THEN 'EXACT_STARTED_MODEL_CALL_HAS_NO_TERMINAL_CUMULATIVE_USAGE'
        WHEN 'LATTICE_MANAGED_REPOSITORY_LINEAGE_MISMATCH' THEN 'LIVE_REPOSITORY_DOES_NOT_MATCH_RETAINED_PROMOTION_SOURCE'
        ELSE NULL END;
    v_expected_retryable := CASE p_blocker_code
        WHEN 'LATTICE_MANAGED_HEARTBEAT_TIMEOUT_WHILE_IN_PROGRESS' THEN true
        WHEN 'LATTICE_MANAGED_VERIFICATION_FAILED' THEN true
        ELSE false END;
    IF v_expected_reason IS NULL
       OR pg_catalog.jsonb_typeof(v_payload) <> 'object'
       OR (SELECT pg_catalog.count(*) FROM pg_catalog.jsonb_object_keys(v_payload)) <> 5
       OR v_payload->>'schema' <> 'lattice.managed-blocker.v1'
       OR (v_payload->>'attempt')::smallint <> p_attempt_number
       OR v_payload->>'code' <> p_blocker_code
       OR v_payload->>'reason' <> v_expected_reason
       OR (v_payload->>'retryable')::boolean IS DISTINCT FROM v_expected_retryable THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_ATTEMPT_CLOSURE_BLOCKER_REJECTED';
    END IF;
    -- A WorkerTurn is an external-effect boundary: it always requires its
    -- exact worker terminal.  Without a WorkerTurn, the only no-terminal
    -- cases are no retained worker provider effect at all, or one accepted
    -- worker thread with no later worker observation.
    IF EXISTS (
        SELECT 1 FROM ONLY foreman_execution.provider_dispatch_claims AS dispatch
         WHERE dispatch.task_ref = p_task_ref
           AND dispatch.attempt_number = p_attempt_number
           AND dispatch.operation_kind = 'WORKER_TURN'
    ) AND NOT EXISTS (
        SELECT 1 FROM ONLY foreman_execution.worker_observations AS terminal
         WHERE terminal.task_ref = p_task_ref
           AND terminal.attempt_number = p_attempt_number
           AND terminal.observation_kind IN (
               'TERMINAL_COMPLETED','TERMINAL_FAILED','TERMINAL_INTERRUPTED'
           )
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_ATTEMPT_CLOSURE_PROVIDER_STILL_POSSIBLY_ACTIVE';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM ONLY foreman_execution.worker_observations AS terminal
         WHERE terminal.task_ref = p_task_ref
           AND terminal.attempt_number = p_attempt_number
           AND terminal.observation_kind IN (
               'TERMINAL_COMPLETED','TERMINAL_FAILED','TERMINAL_INTERRUPTED'
           )
    ) AND NOT (
        NOT EXISTS (
            SELECT 1 FROM ONLY foreman_execution.provider_dispatch_claims AS dispatch
             WHERE dispatch.task_ref = p_task_ref
               AND dispatch.attempt_number = p_attempt_number
               AND dispatch.operation_kind = 'WORKER_THREAD'
        )
        AND NOT EXISTS (
            SELECT 1 FROM ONLY foreman_execution.worker_observations AS observed
             WHERE observed.task_ref = p_task_ref
               AND observed.attempt_number = p_attempt_number
        )
    ) AND NOT (
        EXISTS (
            SELECT 1 FROM ONLY foreman_execution.provider_dispatch_claims AS dispatch
             WHERE dispatch.task_ref = p_task_ref
               AND dispatch.attempt_number = p_attempt_number
               AND dispatch.operation_kind = 'WORKER_THREAD'
        )
        AND 1 = (
            SELECT pg_catalog.count(*) FROM ONLY foreman_execution.worker_observations AS accepted
             WHERE accepted.task_ref = p_task_ref
               AND accepted.attempt_number = p_attempt_number
               AND accepted.observation_kind = 'THREAD_ACCEPTED'
        )
        AND NOT EXISTS (
            SELECT 1 FROM ONLY foreman_execution.worker_observations AS observed
             WHERE observed.task_ref = p_task_ref
               AND observed.attempt_number = p_attempt_number
               AND observed.observation_kind <> 'THREAD_ACCEPTED'
        )
        AND NOT EXISTS (
            SELECT 1 FROM ONLY foreman_execution.provider_dispatch_claims AS dispatch
             WHERE dispatch.task_ref = p_task_ref
               AND dispatch.attempt_number = p_attempt_number
               AND dispatch.operation_kind = 'WORKER_TURN'
        )
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_ATTEMPT_CLOSURE_PROVIDER_STILL_POSSIBLY_ACTIVE';
    END IF;
    -- A completed worker only proves that its own provider turn is quiescent.
    -- Once either reviewer effect was admitted, require the exact claimed
    -- REVIEW_THREAD -> REVIEW_TURN anchor plus one matching admitted/reconciled
    -- turn lifecycle and terminal.  An unrelated TURN_TERMINAL artifact must
    -- never authorize Writer/capacity release.
    IF EXISTS (
        SELECT 1 FROM ONLY foreman_execution.provider_dispatch_claims AS dispatch
         WHERE dispatch.task_ref = p_task_ref
           AND dispatch.attempt_number = p_attempt_number
           AND dispatch.operation_kind IN ('REVIEW_THREAD', 'REVIEW_TURN')
    ) AND NOT EXISTS (
        WITH review_thread AS (
            SELECT *
              FROM ONLY foreman_execution.provider_dispatch_claims AS dispatch
             WHERE dispatch.task_ref = p_task_ref
               AND dispatch.attempt_number = p_attempt_number
               AND dispatch.operation_kind = 'REVIEW_THREAD'
        ), review_turn AS (
            SELECT *
              FROM ONLY foreman_execution.provider_dispatch_claims AS dispatch
             WHERE dispatch.task_ref = p_task_ref
               AND dispatch.attempt_number = p_attempt_number
               AND dispatch.operation_kind = 'REVIEW_TURN'
        ), anchor AS (
            SELECT lifecycle.*,
                   pg_catalog.convert_from(lifecycle.evidence_bytes, 'UTF8')::jsonb AS payload
              FROM ONLY foreman_execution.artifact_references AS lifecycle
              JOIN review_turn
                ON review_turn.anchor_digest = lifecycle.descriptor_digest
             WHERE lifecycle.task_ref = p_task_ref
               AND lifecycle.attempt_number = p_attempt_number
               AND lifecycle.evidence_kind = 'WORKER_LIFECYCLE'
               AND lifecycle.payload_schema = 'lattice.managed-review-lifecycle/1.0'
               AND lifecycle.producer_id = 'lattice-managed-semantic-reviewer'
               AND lifecycle.producer_version = '1.0'
        ), admitted_turn AS (
            SELECT lifecycle.*,
                   pg_catalog.convert_from(lifecycle.evidence_bytes, 'UTF8')::jsonb AS payload
              FROM ONLY foreman_execution.artifact_references AS lifecycle
             WHERE lifecycle.task_ref = p_task_ref
               AND lifecycle.attempt_number = p_attempt_number
               AND lifecycle.evidence_kind = 'WORKER_LIFECYCLE'
               AND lifecycle.payload_schema = 'lattice.managed-review-lifecycle/1.0'
               AND lifecycle.producer_id = 'lattice-managed-semantic-reviewer'
               AND lifecycle.producer_version = '1.0'
               AND (pg_catalog.convert_from(lifecycle.evidence_bytes, 'UTF8')::jsonb
                        ->> 'event_type') IN (
                            'TURN_START_ACCEPTED','TURN_STARTED','TURN_RECONCILED',
                            'THREAD_RECONCILED'
                        )
        ), terminal AS (
            SELECT lifecycle.*,
                   pg_catalog.convert_from(lifecycle.evidence_bytes, 'UTF8')::jsonb AS payload
              FROM ONLY foreman_execution.artifact_references AS lifecycle
             WHERE lifecycle.task_ref = p_task_ref
               AND lifecycle.attempt_number = p_attempt_number
               AND lifecycle.evidence_kind = 'WORKER_LIFECYCLE'
               AND lifecycle.payload_schema = 'lattice.managed-review-lifecycle/1.0'
               AND lifecycle.producer_id = 'lattice-managed-semantic-reviewer'
               AND lifecycle.producer_version = '1.0'
               AND (pg_catalog.convert_from(lifecycle.evidence_bytes, 'UTF8')::jsonb
                        ->> 'event_type') = 'TURN_TERMINAL'
        )
        SELECT 1
          FROM review_thread
          JOIN review_turn
            ON review_turn.task_ref = review_thread.task_ref
           AND review_turn.attempt_number = review_thread.attempt_number
           AND review_turn.attempt_id = review_thread.attempt_id
           AND review_turn.binding_digest = review_thread.binding_digest
           AND review_turn.writer_fence = review_thread.writer_fence
           AND review_turn.foreman_generation = review_thread.foreman_generation
           AND review_turn.foreman_checkpoint_digest =
               review_thread.foreman_checkpoint_digest
           AND review_turn.supporting_digest = review_thread.supporting_digest
          JOIN anchor ON true
          JOIN admitted_turn
            ON admitted_turn.producer_digest = anchor.producer_digest
           AND admitted_turn.payload->>'task_ref' = anchor.payload->>'task_ref'
           AND admitted_turn.payload->>'attempt' = anchor.payload->>'attempt'
           AND admitted_turn.payload->>'subject_digest' = anchor.payload->>'subject_digest'
           AND admitted_turn.payload->>'prompt_digest' = anchor.payload->>'prompt_digest'
           AND admitted_turn.payload->>'model_call_identity' =
               anchor.payload->>'model_call_identity'
           AND admitted_turn.payload->>'thread_id' = anchor.payload->>'thread_id'
          JOIN terminal
            ON terminal.producer_digest = admitted_turn.producer_digest
           AND terminal.payload->>'task_ref' = admitted_turn.payload->>'task_ref'
           AND terminal.payload->>'attempt' = admitted_turn.payload->>'attempt'
           AND terminal.payload->>'subject_digest' = admitted_turn.payload->>'subject_digest'
           AND terminal.payload->>'prompt_digest' = admitted_turn.payload->>'prompt_digest'
           AND terminal.payload->>'model_call_identity' =
               admitted_turn.payload->>'model_call_identity'
           AND terminal.payload->>'thread_id' = admitted_turn.payload->>'thread_id'
           AND terminal.payload->>'turn_id' = admitted_turn.payload->>'turn_id'
           AND terminal.payload->>'app_server_generation' =
               admitted_turn.payload->>'app_server_generation'
           AND (terminal.payload->>'sequence')::bigint =
               (admitted_turn.payload->>'sequence')::bigint + 1
         WHERE anchor.payload->>'schema' = 'lattice.managed-review-lifecycle/1.0'
           AND (
               anchor.payload->>'event_type' = 'THREAD_STARTED'
               OR (
                   anchor.payload->>'event_type' = 'THREAD_RECONCILED'
                   AND anchor.payload->>'turn_id' IS NULL
               )
           )
           AND anchor.payload->>'task_ref' = pg_catalog.encode(p_task_ref, 'hex')
           AND (anchor.payload->>'attempt')::smallint = p_attempt_number
           AND anchor.payload->>'model' = 'gpt-5.6-terra'
           AND anchor.payload->>'reasoning' = 'medium'
           AND anchor.payload->>'model_reason' = 'INDEPENDENT_CODE_REVIEW'
           AND admitted_turn.payload->>'turn_id' IS NOT NULL
           AND terminal.payload->>'terminal_status' IN ('completed','interrupted','failed')
           AND 1 = (
               SELECT pg_catalog.count(*) FROM terminal AS exact_terminal
                WHERE exact_terminal.payload->>'task_ref' = anchor.payload->>'task_ref'
                  AND exact_terminal.payload->>'attempt' = anchor.payload->>'attempt'
                  AND exact_terminal.payload->>'subject_digest' = anchor.payload->>'subject_digest'
                  AND exact_terminal.payload->>'prompt_digest' = anchor.payload->>'prompt_digest'
                  AND exact_terminal.payload->>'model_call_identity' =
                      anchor.payload->>'model_call_identity'
                  AND exact_terminal.payload->>'thread_id' = anchor.payload->>'thread_id'
                  AND exact_terminal.payload->>'turn_id' = admitted_turn.payload->>'turn_id'
                  AND exact_terminal.payload->>'app_server_generation' =
                      admitted_turn.payload->>'app_server_generation'
                  AND (exact_terminal.payload->>'sequence')::bigint =
                      (admitted_turn.payload->>'sequence')::bigint + 1
           )
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_ATTEMPT_CLOSURE_REVIEWER_STILL_POSSIBLY_ACTIVE';
    END IF;
    INSERT INTO foreman_execution.attempt_closures (
        task_ref, attempt_number, provider_disposition, blocker_code,
        blocker_descriptor_digest, reconciliation_proof_descriptor_digest,
        writer_fence, closed_at
    ) VALUES (
        p_task_ref, p_attempt_number, 'PROVEN_INACTIVE', p_blocker_code,
        p_blocker_descriptor_digest, NULL, p_writer_fence, v_artifact.created_at
    );
    RETURN 'INSERTED';
END;
$$;

-- Closes one retained worker blocker only after a second immutable Artifact
-- Store object records the typed exact reconciliation result. The original
-- blocker remains byte-for-byte immutable. Claim, provider-dispatch, and this
-- closure share the same transaction lock, so no new effect can cross the
-- proof/closure boundary.
CREATE FUNCTION foreman_execution.close_retained_worker_without_provider_effect_v1(
    p_task_ref bytea, p_attempt_number smallint, p_blocker_code text,
    p_blocker_descriptor_digest bytea,
    p_reconciliation_proof_descriptor_digest bytea, p_writer_fence bigint
) RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_existing foreman_execution.attempt_closures%ROWTYPE;
    v_attempt foreman_execution.worker_attempts%ROWTYPE;
    v_blocker foreman_execution.artifact_references%ROWTYPE;
    v_proof foreman_execution.artifact_references%ROWTYPE;
    v_blocker_payload jsonb;
    v_proof_payload jsonb;
    v_expected_reason text;
    v_thread_claim_count bigint;
    v_turn_claim_count bigint;
    v_other_claim_count bigint;
    v_observation_count bigint;
    v_thread_claimed boolean;
    v_turn_claimed boolean;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);
    PERFORM pg_catalog.count(*)
      FROM foreman_execution.read_execution_environment_rows_v1(p_task_ref);
    SELECT * INTO v_existing FROM ONLY foreman_execution.attempt_closures
     WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number;
    IF FOUND THEN
        IF v_existing.provider_disposition <> 'PROVEN_INACTIVE'
           OR v_existing.blocker_code IS DISTINCT FROM p_blocker_code
           OR v_existing.blocker_descriptor_digest IS DISTINCT FROM p_blocker_descriptor_digest
           OR v_existing.reconciliation_proof_descriptor_digest IS DISTINCT FROM
                p_reconciliation_proof_descriptor_digest
           OR v_existing.writer_fence IS DISTINCT FROM p_writer_fence THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_RETAINED_CLOSURE_SUBSTITUTION';
        END IF;
        RETURN 'EXACT_REPLAY';
    END IF;

    SELECT * INTO v_attempt FROM ONLY foreman_execution.worker_attempts
     WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number;
    SELECT * INTO v_blocker FROM ONLY foreman_execution.artifact_references AS artifact
     WHERE artifact.task_ref = p_task_ref
       AND artifact.attempt_number = p_attempt_number
       AND artifact.descriptor_digest = p_blocker_descriptor_digest
       AND artifact.evidence_kind = 'WORKER_LIFECYCLE'
       AND artifact.payload_schema = 'lattice.managed-blocker.v1'
       AND artifact.producer_id = 'lattice-foreman'
       AND artifact.producer_version = '1';
    SELECT * INTO v_proof FROM ONLY foreman_execution.artifact_references AS artifact
     WHERE artifact.task_ref = p_task_ref
       AND artifact.attempt_number = p_attempt_number
       AND artifact.descriptor_digest = p_reconciliation_proof_descriptor_digest
       AND artifact.evidence_kind = 'WORKER_LIFECYCLE'
       AND artifact.payload_schema = 'lattice.managed-no-provider-effect-proof.v1'
       AND artifact.producer_id = 'lattice-foreman'
       AND artifact.producer_version = '1';
    IF v_attempt.task_ref IS NULL
       OR v_attempt.writer_fence IS DISTINCT FROM p_writer_fence
       OR v_blocker.task_ref IS NULL
       OR v_proof.task_ref IS NULL
       OR v_blocker.descriptor_digest = v_proof.descriptor_digest
       OR v_blocker.producer_digest IS DISTINCT FROM v_attempt.foreman_checkpoint_digest
       OR v_proof.producer_digest IS DISTINCT FROM v_attempt.foreman_checkpoint_digest
       OR v_proof.created_at::timestamp with time zone <
            v_blocker.created_at::timestamp with time zone THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_RETAINED_CLOSURE_BINDING_MISMATCH';
    END IF;

    v_blocker_payload := pg_catalog.convert_from(v_blocker.evidence_bytes, 'UTF8')::jsonb;
    v_expected_reason := CASE p_blocker_code
        WHEN 'LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL' THEN
            'PROVIDER_PROCESS_EXITED_WITHOUT_EXACT_TURN_TERMINAL'
        WHEN 'LATTICE_MANAGED_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED' THEN
            'BOUNDED_EXACT_PROVIDER_RECONCILIATION_EXHAUSTED'
        WHEN 'LATTICE_MANAGED_BRIDGE_HEARTBEAT_TIMEOUT_RECONCILIATION_REQUIRED' THEN
            'BRIDGE_SILENCE_REQUIRES_EXACT_PROVIDER_RECONCILIATION'
        WHEN 'LATTICE_MANAGED_THREAD_START_RPC_INVALID_PARAMS' THEN
            'WORKER_THREAD_START_REJECTED_REQUIRES_EXACT_NO_EFFECT_RECONCILIATION'
        WHEN 'LATTICE_MANAGED_THREAD_START_RPC_REJECTED' THEN
            'WORKER_THREAD_START_RPC_REJECTED_EFFECT_REMAINS_AMBIGUOUS'
        WHEN 'LATTICE_MANAGED_TURN_START_RPC_INVALID_PARAMS' THEN
            'WORKER_TURN_START_REJECTED_REQUIRES_EXACT_NO_EFFECT_RECONCILIATION'
        WHEN 'LATTICE_MANAGED_TURN_START_RPC_REJECTED' THEN
            'WORKER_TURN_START_RPC_REJECTED_EFFECT_REMAINS_AMBIGUOUS'
        ELSE NULL END;
    IF v_expected_reason IS NULL
       OR pg_catalog.jsonb_typeof(v_blocker_payload) <> 'object'
       OR (SELECT pg_catalog.count(*) FROM pg_catalog.jsonb_object_keys(v_blocker_payload)) <> 5
       OR v_blocker_payload->>'schema' <> 'lattice.managed-blocker.v1'
       OR (v_blocker_payload->>'attempt')::smallint <> p_attempt_number
       OR v_blocker_payload->>'code' <> p_blocker_code
       OR v_blocker_payload->>'reason' <> v_expected_reason
       OR (v_blocker_payload->>'retryable')::boolean IS DISTINCT FROM false THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_RETAINED_CLOSURE_BLOCKER_REJECTED';
    END IF;

    v_proof_payload := pg_catalog.convert_from(v_proof.evidence_bytes, 'UTF8')::jsonb;
    IF pg_catalog.jsonb_typeof(v_proof_payload) <> 'object'
       OR (SELECT pg_catalog.count(*) FROM pg_catalog.jsonb_object_keys(v_proof_payload)) <> 9
       OR v_proof_payload->>'schema' <> 'lattice.managed-no-provider-effect-proof.v1'
       OR v_proof_payload->>'task_ref' <> pg_catalog.encode(p_task_ref, 'hex')
       OR (v_proof_payload->>'attempt')::smallint <> p_attempt_number
       OR v_proof_payload->>'blocker_descriptor_digest' <>
            pg_catalog.encode(p_blocker_descriptor_digest, 'hex')
       OR v_proof_payload->>'proof_kind' NOT IN (
            'PROVEN_NO_PROVIDER_CANDIDATE', 'EXACT_EMPTY_THREAD_NO_TURN'
       )
       OR pg_catalog.jsonb_typeof(v_proof_payload->'worker_thread_claimed') <> 'boolean'
       OR pg_catalog.jsonb_typeof(v_proof_payload->'worker_turn_claimed') <> 'boolean'
       OR NOT (v_proof_payload ? 'thread_observation_payload_digest')
       OR NOT (v_proof_payload ? 'thread_observation_evidence_digest') THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_RETAINED_CLOSURE_PROOF_REJECTED';
    END IF;
    v_thread_claimed := (v_proof_payload->>'worker_thread_claimed')::boolean;
    v_turn_claimed := (v_proof_payload->>'worker_turn_claimed')::boolean;

    SELECT pg_catalog.count(*) FILTER (WHERE operation_kind = 'WORKER_THREAD'),
           pg_catalog.count(*) FILTER (WHERE operation_kind = 'WORKER_TURN'),
           pg_catalog.count(*) FILTER (
               WHERE operation_kind NOT IN ('WORKER_THREAD','WORKER_TURN')
           )
      INTO v_thread_claim_count, v_turn_claim_count, v_other_claim_count
      FROM ONLY foreman_execution.provider_dispatch_claims
     WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number;
    SELECT pg_catalog.count(*) INTO v_observation_count
      FROM ONLY foreman_execution.worker_observations
     WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number;

    IF v_thread_claim_count NOT BETWEEN 0 AND 1
       OR v_turn_claim_count NOT BETWEEN 0 AND 1
       OR v_other_claim_count <> 0
       OR v_thread_claimed IS DISTINCT FROM (v_thread_claim_count = 1)
       OR v_turn_claimed IS DISTINCT FROM (v_turn_claim_count = 1) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_RETAINED_CLOSURE_PROVIDER_STILL_POSSIBLY_ACTIVE';
    END IF;

    IF v_proof_payload->>'proof_kind' = 'PROVEN_NO_PROVIDER_CANDIDATE' THEN
        IF v_thread_claimed
           OR v_turn_claimed
           OR v_observation_count <> 0
           OR pg_catalog.jsonb_typeof(
                v_proof_payload->'thread_observation_payload_digest'
              ) <> 'null'
           OR pg_catalog.jsonb_typeof(
                v_proof_payload->'thread_observation_evidence_digest'
              ) <> 'null' THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_RETAINED_CLOSURE_PROVIDER_STILL_POSSIBLY_ACTIVE';
        END IF;
    ELSE
        IF NOT v_thread_claimed
           OR v_observation_count <> 1
           OR pg_catalog.jsonb_typeof(
                v_proof_payload->'thread_observation_payload_digest'
              ) <> 'string'
           OR pg_catalog.jsonb_typeof(
                v_proof_payload->'thread_observation_evidence_digest'
              ) <> 'string'
           OR v_proof_payload->>'thread_observation_payload_digest' !~ '^[0-9a-f]{64}$'
           OR v_proof_payload->>'thread_observation_evidence_digest' !~ '^[0-9a-f]{64}$'
           OR 1 <> (
               SELECT pg_catalog.count(*)
                 FROM ONLY foreman_execution.worker_observations AS observed
                WHERE observed.task_ref = p_task_ref
                  AND observed.attempt_number = p_attempt_number
                  AND observed.observation_kind = 'THREAD_ACCEPTED'
                  AND observed.turn_id IS NULL
                  AND observed.payload_digest = pg_catalog.decode(
                        v_proof_payload->>'thread_observation_payload_digest', 'hex'
                  )
                  AND observed.evidence_digest = pg_catalog.decode(
                        v_proof_payload->>'thread_observation_evidence_digest', 'hex'
                  )
           ) THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_RETAINED_CLOSURE_PROVIDER_STILL_POSSIBLY_ACTIVE';
        END IF;
    END IF;

    INSERT INTO foreman_execution.attempt_closures (
        task_ref, attempt_number, provider_disposition, blocker_code,
        blocker_descriptor_digest, reconciliation_proof_descriptor_digest,
        writer_fence, closed_at
    ) VALUES (
        p_task_ref, p_attempt_number, 'PROVEN_INACTIVE', p_blocker_code,
        p_blocker_descriptor_digest, p_reconciliation_proof_descriptor_digest,
        p_writer_fence, v_proof.created_at
    );
    RETURN 'INSERTED';
END;
$$;

-- Converts one exact pending worker packet directly into a terminal closed
-- attempt.  The Ledger-linked blocker is staged before this call, but the
-- worker attempt, finalized Artifact Store reference, closure, and pending-row
-- removal are one advisory-locked transaction.  There is therefore no crash
-- window in which the packet is claimable after its no-provider-effect proof
-- has become durable.
CREATE FUNCTION foreman_execution.close_pending_worker_attempt_v1(
    p_task_ref bytea, p_attempt_number smallint, p_blocker_code text,
    p_blocker_descriptor_digest bytea, p_writer_fence bigint
) RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_pending foreman_execution.pending_worker_claims%ROWTYPE;
    v_existing foreman_execution.worker_attempts%ROWTYPE;
    v_closure foreman_execution.attempt_closures%ROWTYPE;
    v_stage foreman_execution.staged_artifact_references%ROWTYPE;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);
    SELECT * INTO v_closure FROM ONLY foreman_execution.attempt_closures
     WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number;
    IF FOUND THEN
        IF EXISTS (SELECT 1 FROM ONLY foreman_execution.pending_worker_claims
                    WHERE task_ref = p_task_ref)
           OR v_closure.blocker_code IS DISTINCT FROM p_blocker_code
           OR v_closure.blocker_descriptor_digest IS DISTINCT FROM p_blocker_descriptor_digest
           OR v_closure.writer_fence IS DISTINCT FROM p_writer_fence THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_PENDING_CLOSURE_SUBSTITUTION';
        END IF;
        PERFORM foreman_execution.record_attempt_closure_v1(
            p_task_ref, p_attempt_number, p_blocker_code,
            p_blocker_descriptor_digest, p_writer_fence
        );
        PERFORM pg_catalog.count(*)
          FROM foreman_execution.read_execution_environment_rows_v1(p_task_ref);
        RETURN 'EXACT_REPLAY';
    END IF;

    SELECT * INTO v_pending FROM ONLY foreman_execution.pending_worker_claims
     WHERE task_ref = p_task_ref;
    SELECT * INTO v_existing FROM ONLY foreman_execution.worker_attempts
     WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number;
    SELECT * INTO v_stage FROM ONLY foreman_execution.staged_artifact_references
     WHERE task_ref = p_task_ref;
    IF v_existing.task_ref IS NOT NULL
       OR v_pending.task_ref IS NULL
       OR v_pending.attempt_number IS DISTINCT FROM p_attempt_number
       OR v_pending.writer_fence IS DISTINCT FROM p_writer_fence
       OR v_stage.task_ref IS NULL
       OR v_stage.attempt_number IS DISTINCT FROM p_attempt_number
       OR v_stage.descriptor_digest IS DISTINCT FROM p_blocker_descriptor_digest
       OR v_stage.evidence_kind <> 'WORKER_LIFECYCLE'
       OR v_stage.payload_schema <> 'lattice.managed-blocker.v1'
       OR v_stage.producer_id <> 'lattice-foreman'
       OR v_stage.producer_version <> '1'
       OR v_stage.producer_digest IS DISTINCT FROM v_pending.foreman_checkpoint_digest
       OR EXISTS (SELECT 1 FROM ONLY foreman_execution.provider_dispatch_claims
                   WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number)
       OR EXISTS (SELECT 1 FROM ONLY foreman_execution.worker_observations
                   WHERE task_ref = p_task_ref AND attempt_number = p_attempt_number) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_PENDING_CLOSURE_REJECTED';
    END IF;
    PERFORM pg_catalog.count(*)
      FROM foreman_execution.read_execution_environment_rows_v1(p_task_ref);
    IF v_pending.execution_environment_ref <>
       'execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000001'
       AND NOT EXISTS (
           SELECT 1
             FROM ONLY foreman_execution.execution_environments AS environment
            WHERE environment.task_ref = v_pending.task_ref
              AND environment.attempt_number = v_pending.attempt_number
              AND environment.attempt_id = v_pending.attempt_id
              AND environment.packet_digest = v_pending.packet_digest
              AND environment.environment_ref = v_pending.execution_environment_ref
       ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_PENDING_CLOSURE_EXECUTION_ENVIRONMENT_REQUIRED';
    END IF;

    INSERT INTO foreman_execution.worker_attempts (
        task_ref, attempt_number, attempt_id, successor_stream_id,
        task_spec_digest, binding_digest, budget_digest, foreman_generation,
        model, reasoning, writer_fence, foreman_checkpoint_digest,
        approval_receipt_digest, packet_digest, execution_environment_ref, worktree_digest,
        base_commit_digest, model_reason, model_reason_digest, claimed_at,
        payload_digest, ledger_event_digest
    ) VALUES (
        v_pending.task_ref, v_pending.attempt_number, v_pending.attempt_id,
        v_pending.successor_stream_id, v_pending.task_spec_digest,
        v_pending.binding_digest, v_pending.budget_digest,
        v_pending.foreman_generation, v_pending.model, v_pending.reasoning,
        v_pending.writer_fence, v_pending.foreman_checkpoint_digest,
        v_pending.approval_receipt_digest, v_pending.packet_digest,
        v_pending.execution_environment_ref, v_pending.worktree_digest,
        v_pending.base_commit_digest,
        v_pending.model_reason, v_pending.model_reason_digest,
        v_pending.claimed_at, v_pending.payload_digest,
        v_pending.ledger_event_digest
    );
    PERFORM foreman_execution.finalize_staged_artifact_reference_v1(
        p_task_ref, p_attempt_number, p_blocker_descriptor_digest
    );
    DELETE FROM ONLY foreman_execution.pending_worker_claims
     WHERE task_ref = p_task_ref
       AND attempt_number = p_attempt_number
       AND ledger_event_digest = v_pending.ledger_event_digest;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_PENDING_CLOSURE_REJECTED';
    END IF;
    PERFORM foreman_execution.record_attempt_closure_v1(
        p_task_ref, p_attempt_number, p_blocker_code,
        p_blocker_descriptor_digest, p_writer_fence
    );
    RETURN 'INSERTED';
END;
$$;

-- Holds the same global Foreman serialization key across the restart
-- Writer-blocker durable predicate reload and the existing
-- stage -> Task Ledger append -> finalize outbox sequence.  This is a
-- session lock, rather than a transaction lock, so each outbox durability
-- boundary remains independently recoverable after process death.
CREATE FUNCTION foreman_execution.begin_restart_writer_blocker_guard_v1(
    p_task_ref bytea, p_attempt_number smallint
) RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
BEGIN
    IF pg_catalog.octet_length(p_task_ref) <> 32
       OR p_attempt_number NOT BETWEEN 1 AND 3 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_RESTART_WRITER_BLOCKER_GUARD_REJECTED';
    END IF;
    PERFORM pg_catalog.pg_advisory_lock(7212400260826);
    RETURN true;
END;
$$;

CREATE FUNCTION foreman_execution.end_restart_writer_blocker_guard_v1(
    p_task_ref bytea, p_attempt_number smallint
) RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
BEGIN
    IF pg_catalog.octet_length(p_task_ref) <> 32
       OR p_attempt_number NOT BETWEEN 1 AND 3
       OR NOT pg_catalog.pg_advisory_unlock(7212400260826) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_RESTART_WRITER_BLOCKER_GUARD_REJECTED';
    END IF;
    RETURN true;
END;
$$;

CREATE FUNCTION foreman_execution.record_approval_evidence_v1(
    p_task_ref bytea, p_successor_stream_id bytea, p_task_spec_digest bytea,
    p_approval_subject_digest bytea, p_budget_digest bytea,
    p_authority_source text, p_capability text,
    p_authority_evidence_digest bytea, p_approval_receipt_digest bytea,
    p_issued_at text, p_expires_at text, p_authority_digest bytea,
    p_approval_owner_snapshot_digest bytea,
    p_approval_owner_snapshot_content_digest bytea,
    p_approval_owner_snapshot_bytes bytea,
    p_approval_command_high_water bigint,
    p_approval_command_tail_digest bytea,
    p_approval_nonce_bindings_digest bytea,
    p_stream_id bytea, p_event_sequence numeric, p_event_digest bytea,
    p_command_id text, p_request_digest bytea, p_payload_digest bytea
) RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_existing foreman_execution.approval_evidence%ROWTYPE;
    v_owner foreman_execution.approval_owner_snapshots%ROWTYPE;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);
    IF p_authority_source = 'VERIFIED_APPROVAL'
       AND NOT pg_catalog.pg_has_role(
           session_user, 'lattice_migrator', 'MEMBER'
       ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_APPROVAL_OWNER_ROLE_REQUIRED';
    END IF;
    IF (p_authority_source = 'VERIFIED_APPROVAL' AND NOT
        (p_approval_owner_snapshot_digest IS NOT NULL
         AND p_approval_owner_snapshot_content_digest IS NOT NULL
         AND p_approval_owner_snapshot_bytes IS NOT NULL
         AND p_approval_command_high_water IS NOT NULL
         AND p_approval_command_tail_digest IS NOT NULL
         AND p_approval_nonce_bindings_digest IS NOT NULL))
       OR (p_authority_source <> 'VERIFIED_APPROVAL' AND
           (p_approval_owner_snapshot_digest IS NOT NULL
            OR p_approval_owner_snapshot_content_digest IS NOT NULL
            OR p_approval_owner_snapshot_bytes IS NOT NULL
            OR p_approval_command_high_water IS NOT NULL
            OR p_approval_command_tail_digest IS NOT NULL
            OR p_approval_nonce_bindings_digest IS NOT NULL)) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_APPROVAL_OWNER_SNAPSHOT_REQUIRED';
    END IF;
    IF p_authority_source = 'VERIFIED_APPROVAL' THEN
        IF pg_catalog.octet_length(p_approval_owner_snapshot_bytes) NOT BETWEEN 1 AND 16777216
           OR p_approval_owner_snapshot_content_digest IS DISTINCT FROM
              pg_catalog.sha256(p_approval_owner_snapshot_bytes)
           OR p_approval_command_high_water <= 0 THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_APPROVAL_OWNER_SNAPSHOT_REJECTED';
        END IF;
        SELECT * INTO v_owner FROM ONLY foreman_execution.approval_owner_snapshots
         WHERE snapshot_digest = p_approval_owner_snapshot_digest;
        IF FOUND THEN
            IF v_owner.snapshot_content_digest IS DISTINCT FROM p_approval_owner_snapshot_content_digest
               OR v_owner.snapshot_bytes IS DISTINCT FROM p_approval_owner_snapshot_bytes
               OR v_owner.command_high_water IS DISTINCT FROM p_approval_command_high_water
               OR v_owner.command_tail_digest IS DISTINCT FROM p_approval_command_tail_digest
               OR v_owner.nonce_bindings_digest IS DISTINCT FROM p_approval_nonce_bindings_digest THEN
                RAISE EXCEPTION USING ERRCODE = 'P0001',
                    MESSAGE = 'FOREMAN_APPROVAL_OWNER_SNAPSHOT_SUBSTITUTION';
            END IF;
        ELSE
            INSERT INTO foreman_execution.approval_owner_snapshots VALUES (
                p_approval_owner_snapshot_digest,
                p_approval_owner_snapshot_content_digest,
                p_approval_owner_snapshot_bytes,
                p_approval_command_high_water,
                p_approval_command_tail_digest,
                p_approval_nonce_bindings_digest
            );
        END IF;
    END IF;
    SELECT * INTO v_existing FROM ONLY foreman_execution.approval_evidence
     WHERE task_ref = p_task_ref AND authority_digest = p_authority_digest;
    IF FOUND THEN
        IF v_existing.successor_stream_id IS DISTINCT FROM p_successor_stream_id
           OR v_existing.task_spec_digest IS DISTINCT FROM p_task_spec_digest
           OR v_existing.approval_subject_digest IS DISTINCT FROM p_approval_subject_digest
           OR v_existing.budget_digest IS DISTINCT FROM p_budget_digest
           OR v_existing.authority_source IS DISTINCT FROM p_authority_source
           OR v_existing.capability IS DISTINCT FROM p_capability
           OR v_existing.authority_evidence_digest IS DISTINCT FROM p_authority_evidence_digest
           OR v_existing.approval_receipt_digest IS DISTINCT FROM p_approval_receipt_digest
           OR v_existing.issued_at IS DISTINCT FROM p_issued_at
           OR v_existing.expires_at IS DISTINCT FROM p_expires_at
           OR v_existing.approval_owner_snapshot_digest IS DISTINCT FROM p_approval_owner_snapshot_digest
           OR v_existing.ledger_event_digest IS DISTINCT FROM p_event_digest THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_APPROVAL_SUBSTITUTION';
        END IF;
        PERFORM foreman_execution.assert_exact_child_event_v1(
            'APPROVAL_EVIDENCE', p_task_ref, p_stream_id, p_event_sequence,
            p_event_digest, p_command_id, p_request_digest, p_payload_digest,
            'RECORD_APPROVAL_EVIDENCE_V1'
        );
        RETURN 'EXACT_REPLAY';
    END IF;
    IF (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.task_promotions AS p
         WHERE p.task_ref = p_task_ref AND p.successor_stream_id = p_successor_stream_id
           AND p.approval_subject_digest = p_approval_subject_digest
           AND p.task_spec_digest = p_task_spec_digest AND p.budget_digest = p_budget_digest) <> 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_APPROVAL_BINDING_MISMATCH';
    END IF;
    IF p_stream_id IS DISTINCT FROM p_successor_stream_id
       OR p_payload_digest IS DISTINCT FROM p_authority_digest THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_APPROVAL_EVENT_MISMATCH';
    END IF;
    PERFORM foreman_execution.insert_child_event_v1(
        'APPROVAL_EVIDENCE', p_task_ref, p_stream_id, p_event_sequence,
        p_event_digest, p_command_id, p_request_digest, p_payload_digest,
        'RECORD_APPROVAL_EVIDENCE_V1'
    );
    INSERT INTO foreman_execution.approval_evidence VALUES (
        p_task_ref, p_successor_stream_id, p_task_spec_digest,
        p_approval_subject_digest, p_budget_digest, p_authority_source,
        p_capability, p_authority_evidence_digest, p_approval_receipt_digest,
        p_issued_at, p_expires_at, p_authority_digest,
        p_approval_owner_snapshot_digest, p_event_digest
    );
    RETURN 'INSERTED';
END;
$$;

CREATE FUNCTION foreman_execution.read_extension_identity_v1()
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
      JOIN ONLY foreman_execution.extension_ledger AS l
        ON l.ledger_ordinal = 1
       AND l.extension_id = i.extension_id
       AND l.extension_schema_version = i.extension_schema_version
       AND l.extension_sql_sha256 = i.extension_sql_sha256
       AND l.extension_manifest_sha256 = i.extension_manifest_sha256
       AND l.database_uuid = i.database_uuid
       AND l.database_identity_sha256 = i.database_identity_sha256
       AND l.global_schema_version = i.global_schema_version
       AND l.global_manifest_sha256 = i.global_manifest_sha256
       AND l.event_kind = 'INSTALLED'
      JOIN ONLY control.database_identity AS d
        ON d.singleton AND d.database_uuid = i.database_uuid
      JOIN ONLY control.schema_compatibility AS c
        ON c.singleton
       AND c.current_schema_version = i.global_schema_version
       AND pg_catalog.btrim(c.manifest_sha256) = i.global_manifest_sha256
     WHERE i.singleton
       AND i.database_name = pg_catalog.current_database()
$$;

CREATE FUNCTION foreman_execution.read_task_promotion_source_v1(p_task_ref bytea)
RETURNS TABLE(base_ref text, base_commit text)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT p.base_ref::text, p.base_commit::text
      FROM ONLY foreman_execution.task_promotions AS p
     WHERE p.task_ref = p_task_ref
$$;

CREATE FUNCTION foreman_execution.read_pending_worker_attempt_v1(p_task_ref bytea)
RETURNS TABLE(
    successor_stream_id bytea,
    task_spec_digest bytea,
    binding_digest bytea,
    budget_digest bytea,
    attempt_id text,
    attempt_number smallint,
    foreman_generation bigint,
    model text,
    reasoning text,
    writer_fence bigint,
    foreman_checkpoint_digest bytea,
    approval_receipt_digest bytea,
    packet_digest bytea,
    execution_environment_ref text,
    worktree_digest bytea,
    base_commit_digest bytea,
    model_reason text,
    model_reason_digest bytea,
    claimed_at text,
    payload_digest bytea,
    max_attempts smallint,
    reserved_at text,
    ledger_event_digest bytea
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT pending.successor_stream_id, pending.task_spec_digest,
           pending.binding_digest, pending.budget_digest,
           pending.attempt_id::text, pending.attempt_number,
           pending.foreman_generation, pending.model::text,
           pending.reasoning::text, pending.writer_fence,
           pending.foreman_checkpoint_digest,
           pending.approval_receipt_digest, pending.packet_digest,
           pending.execution_environment_ref::text,
           pending.worktree_digest, pending.base_commit_digest,
           pending.model_reason::text, pending.model_reason_digest, pending.claimed_at::text,
           pending.payload_digest, pending.max_attempts,
           pg_catalog.to_char(
               pending.reserved_at AT TIME ZONE 'UTC',
               'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'
           ),
           pending.ledger_event_digest
      FROM ONLY foreman_execution.pending_worker_claims AS pending
     WHERE pending.task_ref = p_task_ref
$$;

CREATE FUNCTION foreman_execution.read_execution_environment_rows_v1(p_task_ref bytea)
RETURNS SETOF foreman_execution.execution_environments
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_environment foreman_execution.execution_environments%ROWTYPE;
    v_descriptor jsonb;
    v_sandbox_policy_template jsonb;
    v_expected_sandbox_policy_ref text;
    v_linux_home text;
    v_anchor_count bigint;
    v_anchor_id text;
    v_anchor_packet bytea;
    v_anchor_environment_ref text;
BEGIN
    IF pg_catalog.octet_length(p_task_ref) <> 32
       OR p_task_ref = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex') THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM (
              SELECT pending.attempt_number
                FROM ONLY foreman_execution.pending_worker_claims AS pending
               WHERE pending.task_ref = p_task_ref
              UNION ALL
              SELECT attempt.attempt_number
                FROM ONLY foreman_execution.worker_attempts AS attempt
               WHERE attempt.task_ref = p_task_ref
          ) AS anchor
         GROUP BY anchor.attempt_number
        HAVING pg_catalog.count(*) <> 1
    ) OR EXISTS (
        SELECT 1
          FROM (
              SELECT pending.task_ref, pending.attempt_number,
                     pending.attempt_id::text, pending.packet_digest,
                     pending.execution_environment_ref::text,
                     'PENDING'::text AS anchor_state
                FROM ONLY foreman_execution.pending_worker_claims AS pending
               WHERE pending.task_ref = p_task_ref
              UNION ALL
              SELECT attempt.task_ref, attempt.attempt_number,
                     attempt.attempt_id::text, attempt.packet_digest,
                     attempt.execution_environment_ref::text,
                     'ACTIVE'::text AS anchor_state
                FROM ONLY foreman_execution.worker_attempts AS attempt
               WHERE attempt.task_ref = p_task_ref
          ) AS anchor
         WHERE (
             anchor.execution_environment_ref =
                'execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000001'
             AND EXISTS (
                 SELECT 1
                   FROM ONLY foreman_execution.execution_environments AS environment
                  WHERE environment.task_ref = anchor.task_ref
                    AND environment.attempt_number = anchor.attempt_number
             )
         ) OR (
             anchor.execution_environment_ref <>
                'execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000001'
             AND NOT EXISTS (
                 SELECT 1
                   FROM ONLY foreman_execution.execution_environments AS environment
                  WHERE environment.task_ref = anchor.task_ref
                    AND environment.attempt_number = anchor.attempt_number
                    AND environment.attempt_id = anchor.attempt_id
                    AND environment.packet_digest = anchor.packet_digest
                    AND environment.environment_ref = anchor.execution_environment_ref
             )
             AND (
                 anchor.anchor_state = 'ACTIVE'
                 OR EXISTS (
                     SELECT 1
                       FROM ONLY foreman_execution.execution_environments AS environment
                      WHERE environment.task_ref = anchor.task_ref
                        AND environment.attempt_number = anchor.attempt_number
                 )
             )
         )
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH';
    END IF;
    FOR v_environment IN
        SELECT environment.*
          FROM ONLY foreman_execution.execution_environments AS environment
         WHERE environment.task_ref = p_task_ref
         ORDER BY environment.attempt_number
    LOOP
        v_descriptor := v_environment.canonical_descriptor::jsonb;
        IF EXISTS (
            SELECT 1
              FROM (VALUES
                    (v_descriptor->'linux'->>'launcher_path'),
                    (v_descriptor->'linux'->>'node_path'),
                    (v_descriptor->'linux'->>'git_path'),
                    (v_descriptor->'linux'->>'supervisor_path'),
                    (v_descriptor->'linux'->>'cwd'),
                    (v_descriptor->'linux'->>'codex_home'),
                    (v_descriptor->'linux'->>'dbus_run_session_path'),
                    (v_descriptor->'linux'->>'setsid_path'),
                    (v_descriptor->'linux'->>'keyring_daemon_path'),
                    (v_descriptor->'linux'->>'keyring_library_path'),
                    (v_descriptor->'linux'->>'xdg_runtime_dir'),
                    (v_descriptor->'process_fence'->>'systemd_run_path'),
                    (v_descriptor->'process_fence'->>'systemctl_path'),
                    (v_descriptor->'process_fence'->'supervisor_bootstrap_node'->>'path'),
                    (v_descriptor->'process_fence'->'immutable_probe_lsattr'->>'path'),
                    (v_descriptor->'process_fence'->'noninteractive_root_probe'->>'path'),
                    (v_descriptor->'verification_toolchain'->>'task_root'),
                    (v_descriptor->'verification_toolchain'->>'isolation_root'),
                    (v_descriptor->'verification_toolchain'->>'home_dir'),
                    (v_descriptor->'verification_toolchain'->>'temp_dir'),
                    (v_descriptor->'verification_toolchain'->>'npm_cache'),
                    (v_descriptor->'verification_toolchain'->>'cargo_home'),
                    (v_descriptor->'verification_toolchain'->>'cargo_target_dir'),
                    (v_descriptor->'verification_toolchain'->'npm'->>'path'),
                    (v_descriptor->'verification_toolchain'->'cargo'->>'path'),
                    (v_descriptor->'verification_toolchain'->'rustc'->>'path'),
                    (v_descriptor->'verification_toolchain'->'rustdoc'->>'path'),
                    (v_descriptor->'verification_toolchain'->'sandbox'->>'path'),
                    (v_descriptor->'verification_toolchain'->'sandbox_helper'->>'path'),
                    (v_descriptor->'immutable_snapshot'->'trees'->'codex'->>'root'),
                    (v_descriptor->'immutable_snapshot'->'trees'->'supervisor_runtime'->>'root'),
                    (v_descriptor->'immutable_snapshot'->'trees'->'node'->>'root'),
                    (v_descriptor->'immutable_snapshot'->'trees'->'rust'->>'root'),
                    (v_descriptor->'immutable_snapshot'->'trees'->'keyring'->>'root')
              ) AS sandbox_path(value)
             WHERE sandbox_path.value IS NULL
                OR sandbox_path.value !~ '^/'
                OR sandbox_path.value ~ '(^|/)\.\.?(/|$)'
                OR pg_catalog.strpos(sandbox_path.value, '//') > 0
                OR pg_catalog.right(sandbox_path.value, 1) = '/'
                OR sandbox_path.value ~ '^/mnt/'
                OR sandbox_path.value !~ '^/[A-Za-z0-9._~/-]+$'
        ) THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH';
        END IF;
        v_linux_home := (pg_catalog.regexp_match(
            v_descriptor->'verification_toolchain'->>'task_root',
            '^(/home/[^/]+)'
        ))[1];
        IF v_linux_home IS NULL THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH';
        END IF;
        v_sandbox_policy_template := pg_catalog.jsonb_build_object(
            'schema', 'lattice.wsl2-sandbox-template/1.0',
            'permission_profile_type', 'managed',
            'filesystem_type', 'restricted',
            'network', 'restricted',
            'base_entries', pg_catalog.jsonb_build_array(
                pg_catalog.jsonb_build_object(
                    'path', pg_catalog.jsonb_build_object(
                        'type', 'special',
                        'value', pg_catalog.jsonb_build_object('kind', 'minimal')
                    ),
                    'access', 'read'
                ),
                pg_catalog.jsonb_build_object(
                    'path', pg_catalog.jsonb_build_object(
                        'type', 'path',
                        'path', v_descriptor->'verification_toolchain'->'task_root'
                    ),
                    'access', 'read'
                )
            ),
            'role_writes', pg_catalog.jsonb_build_object(
                'PREFLIGHT', pg_catalog.jsonb_build_array(
                    v_descriptor->'linux'->'cwd',
                    v_descriptor->'verification_toolchain'->'home_dir',
                    v_descriptor->'verification_toolchain'->'temp_dir',
                    v_descriptor->'verification_toolchain'->'npm_cache',
                    v_descriptor->'verification_toolchain'->'cargo_home',
                    v_descriptor->'verification_toolchain'->'cargo_target_dir'
                ),
                'NODE', pg_catalog.jsonb_build_array(
                    v_descriptor->'verification_toolchain'->'home_dir',
                    v_descriptor->'verification_toolchain'->'temp_dir',
                    v_descriptor->'verification_toolchain'->'npm_cache'
                ),
                'CARGO', pg_catalog.jsonb_build_array(
                    v_descriptor->'verification_toolchain'->'home_dir',
                    v_descriptor->'verification_toolchain'->'temp_dir',
                    v_descriptor->'verification_toolchain'->'cargo_home',
                    v_descriptor->'verification_toolchain'->'cargo_target_dir'
                ),
                'GIT', pg_catalog.jsonb_build_object(
                    'bootstrap', pg_catalog.jsonb_build_array(
                        '$GIT_CONTROL_HOME', '$GIT_CONTROL_TMPDIR'
                    ),
                    'guarded_object_write', pg_catalog.jsonb_build_array(
                        '$GIT_CONTROL_HOME', '$GIT_CONTROL_TMPDIR',
                        '$GIT_COMMON_DIR/objects'
                    ),
                    'guarded_index_write', pg_catalog.jsonb_build_array(
                        '$GIT_CONTROL_HOME', '$GIT_CONTROL_TMPDIR',
                        '$GIT_CONTROL_ROOT/candidate-index'
                    )
                )
            ),
            'deny_entries', pg_catalog.jsonb_build_array(
                pg_catalog.jsonb_build_object(
                    'path', v_descriptor->'linux'->'codex_home',
                    'missing_path_behavior', 'skip'
                ),
                pg_catalog.jsonb_build_object(
                    'path', v_linux_home || '/.codex',
                    'missing_path_behavior', 'skip'
                ),
                pg_catalog.jsonb_build_object(
                    'path', '/mnt', 'missing_path_behavior', 'skip'
                ),
                pg_catalog.jsonb_build_object(
                    'path', v_descriptor->'linux'->'xdg_runtime_dir',
                    'missing_path_behavior', 'skip'
                )
            ),
            'codex_linux_sandbox_exe', NULL::text,
            'sandbox_cwd', 'file://' || (v_descriptor->'linux'->>'cwd'),
            'use_legacy_landlock', false
        );
        v_expected_sandbox_policy_ref :=
            'wsl2-sandbox-policy:sha256:' || pg_catalog.encode(
                pg_catalog.sha256(pg_catalog.convert_to(
                    foreman_execution.canonical_json_v1(v_sandbox_policy_template),
                    'UTF8'
                )), 'hex'
            );
        SELECT pg_catalog.count(*), pg_catalog.min(anchor.attempt_id),
               pg_catalog.decode(
                   pg_catalog.min(pg_catalog.encode(anchor.packet_digest, 'hex')), 'hex'
               ), pg_catalog.min(anchor.execution_environment_ref)
          INTO v_anchor_count, v_anchor_id, v_anchor_packet, v_anchor_environment_ref
          FROM (
              SELECT pending.attempt_id::text, pending.packet_digest,
                     pending.execution_environment_ref::text
                FROM ONLY foreman_execution.pending_worker_claims AS pending
               WHERE pending.task_ref = v_environment.task_ref
                 AND pending.attempt_number = v_environment.attempt_number
              UNION ALL
              SELECT attempt.attempt_id::text, attempt.packet_digest,
                     attempt.execution_environment_ref::text
                FROM ONLY foreman_execution.worker_attempts AS attempt
               WHERE attempt.task_ref = v_environment.task_ref
                 AND attempt.attempt_number = v_environment.attempt_number
          ) AS anchor;
        IF v_anchor_count <> 1
           OR v_anchor_id IS DISTINCT FROM v_environment.attempt_id
           OR v_anchor_packet IS DISTINCT FROM v_environment.packet_digest
           OR v_anchor_environment_ref IS DISTINCT FROM v_environment.environment_ref
           OR v_environment.environment_ref IS DISTINCT FROM
                'execution-environment:sha256:' ||
                pg_catalog.encode(v_environment.execution_domain_digest, 'hex')
           OR v_environment.canonical_descriptor IS DISTINCT FROM
                foreman_execution.canonical_json_v1(v_descriptor)
           OR v_descriptor->>'identity_digest'
                IS DISTINCT FROM v_environment.environment_ref
           OR pg_catalog.sha256(pg_catalog.convert_to(
                foreman_execution.canonical_json_v1(
                    v_descriptor - 'identity_digest'
                ), 'UTF8'
              )) IS DISTINCT FROM v_environment.execution_domain_digest
           OR v_environment.descriptor_schema IS DISTINCT FROM v_descriptor->>'schema'
           OR v_environment.environment_kind IS DISTINCT FROM v_descriptor->>'kind'
           OR v_environment.distribution IS DISTINCT FROM v_descriptor->>'distribution'
           OR v_environment.distribution_os_id IS DISTINCT FROM
                v_descriptor->'distribution_identity'->>'os_id'
           OR v_environment.distribution_version IS DISTINCT FROM
                v_descriptor->'distribution_identity'->>'os_version_id'
           OR v_environment.distribution_codename IS DISTINCT FROM
                v_descriptor->'distribution_identity'->>'os_version_codename'
           OR v_environment.distribution_os_release_digest IS DISTINCT FROM
                pg_catalog.decode(
                    v_descriptor->'distribution_identity'->>'os_release_sha256', 'hex'
                )
           OR v_environment.distribution_kernel_release IS DISTINCT FROM
                v_descriptor->'distribution_identity'->>'kernel_release'
           OR v_environment.distribution_identity_ref IS DISTINCT FROM
                v_descriptor->'distribution_identity'->>'identity_digest'
           OR v_environment.distribution_identity_digest IS DISTINCT FROM
                pg_catalog.decode(pg_catalog.right(
                    v_descriptor->'distribution_identity'->>'identity_digest', 64
                ), 'hex')
           OR v_environment.gateway_path IS DISTINCT FROM v_descriptor->'gateway'->>'windows_path'
           OR v_environment.gateway_version IS DISTINCT FROM v_descriptor->'gateway'->>'version'
           OR v_environment.gateway_digest IS DISTINCT FROM
                pg_catalog.decode(v_descriptor->'gateway'->>'sha256', 'hex')
           OR v_environment.linux_repository_path IS DISTINCT FROM v_descriptor->'linux'->>'cwd'
           OR v_environment.linux_codex_home_path IS DISTINCT FROM v_descriptor->'linux'->>'codex_home'
           OR v_environment.codex_config_ref IS DISTINCT FROM
                v_descriptor->'linux'->>'config_digest'
           OR v_environment.codex_config_digest IS DISTINCT FROM
                pg_catalog.decode(pg_catalog.right(
                    v_descriptor->'linux'->>'config_digest', 64
                ), 'hex')
           OR v_environment.repository_head IS DISTINCT FROM v_descriptor->'linux'->>'repository_head'
           OR v_environment.repository_identity_ref IS DISTINCT FROM
                v_descriptor->'linux'->>'repository_identity'
           OR v_environment.repository_identity_digest IS DISTINCT FROM
                pg_catalog.decode(pg_catalog.right(
                    v_descriptor->'linux'->>'repository_identity', 64
                ), 'hex')
           OR v_environment.launcher_path IS DISTINCT FROM v_descriptor->'linux'->>'launcher_path'
           OR v_environment.launcher_version IS DISTINCT FROM
                v_descriptor->'linux'->>'launcher_version'
           OR v_environment.launcher_digest IS DISTINCT FROM
                pg_catalog.decode(v_descriptor->'linux'->>'launcher_sha256', 'hex')
           OR v_environment.node_path IS DISTINCT FROM v_descriptor->'linux'->>'node_path'
           OR v_environment.node_version IS DISTINCT FROM v_descriptor->'linux'->>'node_version'
           OR v_environment.node_digest IS DISTINCT FROM
                pg_catalog.decode(v_descriptor->'linux'->>'node_sha256', 'hex')
           OR v_environment.git_path IS DISTINCT FROM v_descriptor->'linux'->>'git_path'
           OR v_environment.git_version IS DISTINCT FROM v_descriptor->'linux'->>'git_version'
           OR v_environment.git_digest IS DISTINCT FROM
                pg_catalog.decode(v_descriptor->'linux'->>'git_sha256', 'hex')
           OR v_environment.supervisor_path IS DISTINCT FROM v_descriptor->'linux'->>'supervisor_path'
           OR v_environment.supervisor_digest IS DISTINCT FROM
                pg_catalog.decode(v_descriptor->'linux'->>'supervisor_sha256', 'hex')
           OR v_environment.dbus_run_session_path IS DISTINCT FROM
                v_descriptor->'linux'->>'dbus_run_session_path'
           OR v_environment.dbus_run_session_digest IS DISTINCT FROM
                pg_catalog.decode(v_descriptor->'linux'->>'dbus_run_session_sha256', 'hex')
           OR v_environment.setsid_path IS DISTINCT FROM v_descriptor->'linux'->>'setsid_path'
           OR v_environment.setsid_digest IS DISTINCT FROM
                pg_catalog.decode(v_descriptor->'linux'->>'setsid_sha256', 'hex')
           OR v_environment.keyring_daemon_path IS DISTINCT FROM
                v_descriptor->'linux'->>'keyring_daemon_path'
           OR v_environment.keyring_daemon_digest IS DISTINCT FROM
                pg_catalog.decode(v_descriptor->'linux'->>'keyring_daemon_sha256', 'hex')
           OR v_environment.keyring_library_path IS DISTINCT FROM
                v_descriptor->'linux'->>'keyring_library_path'
           OR v_environment.keyring_library_manifest_ref IS DISTINCT FROM
                v_descriptor->'linux'->>'keyring_library_manifest_digest'
           OR v_environment.keyring_library_manifest_digest IS DISTINCT FROM
                pg_catalog.decode(pg_catalog.right(
                    v_descriptor->'linux'->>'keyring_library_manifest_digest', 64
                ), 'hex')
           OR v_environment.xdg_runtime_dir IS DISTINCT FROM
                v_descriptor->'linux'->>'xdg_runtime_dir'
           OR v_environment.credential_authority_kind IS DISTINCT FROM
                v_descriptor->'credential_authority'->>'kind'
           OR v_environment.credential_authority_ref IS DISTINCT FROM
                v_descriptor->'credential_authority'->>'authority_digest'
           OR v_environment.credential_authority_digest IS DISTINCT FROM
                pg_catalog.decode(pg_catalog.right(
                    v_descriptor->'credential_authority'->>'authority_digest', 64
                ), 'hex')
           OR v_environment.process_fence_schema IS DISTINCT FROM
                v_descriptor->'process_fence'->>'schema'
           OR v_environment.process_fence_kind IS DISTINCT FROM v_descriptor->'process_fence'->>'kind'
           OR v_environment.systemd_run_path IS DISTINCT FROM
                v_descriptor->'process_fence'->>'systemd_run_path'
           OR v_environment.systemd_run_version IS DISTINCT FROM
                v_descriptor->'process_fence'->>'systemd_run_version'
           OR v_environment.systemd_run_digest IS DISTINCT FROM
                pg_catalog.decode(v_descriptor->'process_fence'->>'systemd_run_sha256', 'hex')
           OR v_environment.systemctl_path IS DISTINCT FROM
                v_descriptor->'process_fence'->>'systemctl_path'
           OR v_environment.systemctl_version IS DISTINCT FROM
                v_descriptor->'process_fence'->>'systemctl_version'
           OR v_environment.systemctl_digest IS DISTINCT FROM
                pg_catalog.decode(v_descriptor->'process_fence'->>'systemctl_sha256', 'hex')
           OR v_environment.supervisor_bootstrap_node_path IS DISTINCT FROM
                v_descriptor->'process_fence'->'supervisor_bootstrap_node'->>'path'
           OR v_environment.supervisor_bootstrap_node_version IS DISTINCT FROM
                v_descriptor->'process_fence'->'supervisor_bootstrap_node'->>'version'
           OR v_environment.supervisor_bootstrap_node_digest IS DISTINCT FROM
                pg_catalog.decode(
                    v_descriptor->'process_fence'->'supervisor_bootstrap_node'->>'sha256', 'hex'
                )
           OR v_environment.immutable_probe_lsattr_path IS DISTINCT FROM
                v_descriptor->'process_fence'->'immutable_probe_lsattr'->>'path'
           OR v_environment.immutable_probe_lsattr_version IS DISTINCT FROM
                v_descriptor->'process_fence'->'immutable_probe_lsattr'->>'version'
           OR v_environment.immutable_probe_lsattr_digest IS DISTINCT FROM
                pg_catalog.decode(
                    v_descriptor->'process_fence'->'immutable_probe_lsattr'->>'sha256', 'hex'
                )
           OR v_environment.noninteractive_root_probe_path IS DISTINCT FROM
                v_descriptor->'process_fence'->'noninteractive_root_probe'->>'path'
           OR v_environment.noninteractive_root_probe_version IS DISTINCT FROM
                v_descriptor->'process_fence'->'noninteractive_root_probe'->>'version'
           OR v_environment.noninteractive_root_probe_digest IS DISTINCT FROM
                pg_catalog.decode(
                    v_descriptor->'process_fence'->'noninteractive_root_probe'->>'sha256', 'hex'
                )
           OR v_environment.cgroup_mount IS DISTINCT FROM
                v_descriptor->'process_fence'->>'cgroup_mount'
           OR v_environment.user_runtime_dir IS DISTINCT FROM
                v_descriptor->'process_fence'->>'user_runtime_dir'
           OR v_environment.unit_prefix IS DISTINCT FROM
                v_descriptor->'process_fence'->>'unit_prefix'
           OR v_environment.process_fence_identity_ref IS DISTINCT FROM
                v_descriptor->'process_fence'->>'identity_digest'
           OR v_environment.process_fence_identity_digest IS DISTINCT FROM
                pg_catalog.decode(pg_catalog.right(
                    v_descriptor->'process_fence'->>'identity_digest', 64
                ), 'hex')
           OR v_environment.verification_toolchain_schema IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->>'schema'
           OR v_environment.verification_task_ref IS DISTINCT FROM
                pg_catalog.decode(v_descriptor->'verification_toolchain'->>'task_ref', 'hex')
           OR v_environment.verification_task_root IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->>'task_root'
           OR v_environment.verification_isolation_root IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->>'isolation_root'
           OR v_environment.verification_owner_uid IS DISTINCT FROM
                (v_descriptor->'verification_toolchain'->>'owner_uid')::bigint
           OR v_environment.verification_home_dir IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->>'home_dir'
           OR v_environment.verification_temp_dir IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->>'temp_dir'
           OR v_environment.npm_cache IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->>'npm_cache'
           OR v_environment.cargo_home IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->>'cargo_home'
           OR v_environment.cargo_target_dir IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->>'cargo_target_dir'
           OR v_environment.cargo_host IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->>'cargo_host'
           OR v_environment.npm_path IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->'npm'->>'path'
           OR v_environment.npm_version IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->'npm'->>'version'
           OR v_environment.npm_digest IS DISTINCT FROM pg_catalog.decode(
                v_descriptor->'verification_toolchain'->'npm'->>'sha256', 'hex'
              )
           OR v_environment.cargo_path IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->'cargo'->>'path'
           OR v_environment.cargo_version IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->'cargo'->>'version'
           OR v_environment.cargo_digest IS DISTINCT FROM pg_catalog.decode(
                v_descriptor->'verification_toolchain'->'cargo'->>'sha256', 'hex'
              )
           OR v_environment.rustc_path IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->'rustc'->>'path'
           OR v_environment.rustc_version IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->'rustc'->>'version'
           OR v_environment.rustc_digest IS DISTINCT FROM pg_catalog.decode(
                v_descriptor->'verification_toolchain'->'rustc'->>'sha256', 'hex'
              )
           OR v_environment.rustdoc_path IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->'rustdoc'->>'path'
           OR v_environment.rustdoc_version IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->'rustdoc'->>'version'
           OR v_environment.rustdoc_digest IS DISTINCT FROM pg_catalog.decode(
                v_descriptor->'verification_toolchain'->'rustdoc'->>'sha256', 'hex'
              )
           OR v_environment.sandbox_path IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->'sandbox'->>'path'
           OR v_environment.sandbox_version IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->'sandbox'->>'version'
           OR v_environment.sandbox_digest IS DISTINCT FROM pg_catalog.decode(
                v_descriptor->'verification_toolchain'->'sandbox'->>'sha256', 'hex'
              )
           OR v_environment.sandbox_helper_path IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->'sandbox_helper'->>'path'
           OR v_environment.sandbox_helper_version IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->'sandbox_helper'->>'version'
           OR v_environment.sandbox_helper_digest IS DISTINCT FROM pg_catalog.decode(
                v_descriptor->'verification_toolchain'->'sandbox_helper'->>'sha256', 'hex'
              )
           OR v_environment.verification_toolchain_identity_ref IS DISTINCT FROM
                v_descriptor->'verification_toolchain'->>'identity_digest'
           OR v_environment.verification_toolchain_identity_digest IS DISTINCT FROM
                pg_catalog.decode(pg_catalog.right(
                    v_descriptor->'verification_toolchain'->>'identity_digest', 64
                ), 'hex')
           OR v_environment.immutable_snapshot_ref IS DISTINCT FROM
                v_descriptor->'immutable_snapshot'->>'snapshot_digest'
           OR v_environment.immutable_snapshot_digest IS DISTINCT FROM
                pg_catalog.decode(pg_catalog.right(
                    v_descriptor->'immutable_snapshot'->>'snapshot_digest', 64
                ), 'hex')
           OR v_environment.sandbox_policy_ref IS DISTINCT FROM
                v_descriptor->'sandbox_policy'->>'policy_digest'
           OR v_descriptor->'sandbox_policy'->>'policy_digest'
                IS DISTINCT FROM v_expected_sandbox_policy_ref
           OR v_environment.sandbox_policy_digest IS DISTINCT FROM
                pg_catalog.decode(pg_catalog.right(
                    v_descriptor->'sandbox_policy'->>'policy_digest', 64
                ), 'hex')
           OR v_environment.privilege_boundary_ref IS DISTINCT FROM
                v_descriptor->'privilege_boundary'->>'boundary_digest'
           OR v_environment.privilege_boundary_digest IS DISTINCT FROM
                pg_catalog.decode(pg_catalog.right(
                    v_descriptor->'privilege_boundary'->>'boundary_digest', 64
                ), 'hex')
           OR v_environment.path_mapping_windows_path IS DISTINCT FROM
                v_descriptor->'path_mapping'->>'windows_path'
           OR v_environment.path_mapping_linux_path IS DISTINCT FROM
                v_descriptor->'path_mapping'->>'linux_path'
           OR v_environment.path_mapping_ref IS DISTINCT FROM
                v_descriptor->'path_mapping'->>'digest'
           OR v_environment.path_mapping_digest IS DISTINCT FROM
                pg_catalog.decode(pg_catalog.right(
                    v_descriptor->'path_mapping'->>'digest', 64
                ), 'hex') THEN
            RAISE EXCEPTION USING ERRCODE = 'P0001',
                MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH';
        END IF;
    END LOOP;
    RETURN QUERY
    SELECT environment.*
      FROM ONLY foreman_execution.execution_environments AS environment
     WHERE environment.task_ref = p_task_ref
     ORDER BY environment.attempt_number;
END;
$$;

CREATE FUNCTION foreman_execution.read_worker_budget_v1(p_task_ref bytea)
RETURNS TABLE(
    global_active_limit smallint,
    per_task_active_limit smallint,
    repair_retry_limit smallint,
    max_duration_seconds bigint,
    max_total_tokens bigint,
    max_model_calls bigint,
    external_cost_status text,
    external_cost_limit_micros bigint,
    deadline_at text,
    budget_pointer text,
    budget_digest bytea
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT p.global_active_limit, p.per_task_active_limit, p.repair_retry_limit,
           p.max_duration_seconds, p.max_total_tokens, p.max_model_calls,
           p.external_cost_status::text, p.external_cost_limit_micros,
           p.deadline_at::text, p.budget_pointer::text, p.budget_digest
      FROM ONLY foreman_execution.task_promotions AS p
     WHERE p.task_ref = p_task_ref
$$;

CREATE FUNCTION foreman_execution.read_staged_artifact_reference_v1(p_task_ref bytea)
RETURNS TABLE(
    project_id text,
    attempt_number smallint,
    evidence_kind text,
    media_type text,
    payload_schema text,
    producer_id text,
    producer_version text,
    producer_digest bytea,
    created_at text,
    evidence_bytes bytea,
    content_digest bytea,
    descriptor_digest bytea,
    stream_project_id text,
    project_snapshot_id text,
    task_id text,
    task_revision text,
    task_spec_digest bytea,
    accounting_currency text,
    ledger_stream_id bytea,
    before_sequence text,
    before_last_event_digest bytea,
    before_resource_revision text,
    before_resource_projection_digest bytea,
    before_head_digest bytea,
    ledger_event_sequence text,
    ledger_event_digest bytea,
    ledger_command_id text,
    ledger_request_digest bytea,
    ledger_payload_digest bytea,
    correlation_id text,
    command_occurred_at text,
    staged_at text
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT staged.project_id::text, staged.attempt_number,
           staged.evidence_kind::text, staged.media_type::text,
           staged.payload_schema::text, staged.producer_id::text,
           staged.producer_version::text, staged.producer_digest,
           staged.created_at::text, staged.evidence_bytes,
           staged.content_digest, staged.descriptor_digest,
           stream.project_id::text, stream.project_snapshot_id::text,
           stream.task_id::text, stream.task_revision::text,
           stream.task_spec_digest, stream.accounting_currency::text,
           staged.ledger_stream_id, staged.before_sequence::text,
           staged.before_last_event_digest,
           staged.before_resource_revision::text,
           staged.before_resource_projection_digest,
           staged.before_head_digest, staged.ledger_event_sequence::text,
           staged.ledger_event_digest, staged.ledger_command_id::text,
           staged.ledger_request_digest, staged.ledger_payload_digest,
           staged.correlation_id::text, staged.command_occurred_at::text,
           pg_catalog.to_char(
               staged.staged_at AT TIME ZONE 'UTC',
               'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'
           )
      FROM ONLY foreman_execution.staged_artifact_references AS staged
      JOIN ONLY control.task_ledger_streams AS stream
        ON stream.stream_id = staged.ledger_stream_id
     WHERE staged.task_ref = p_task_ref
     ORDER BY staged.task_ref
$$;

CREATE FUNCTION foreman_execution.read_managed_evidence_v1(
    p_task_ref bytea,
    p_attempt_number smallint
) RETURNS TABLE(
    project_id text,
    evidence_kind text,
    media_type text,
    payload_schema text,
    producer_id text,
    producer_version text,
    producer_digest bytea,
    created_at text,
    evidence_bytes bytea,
    content_digest bytea,
    descriptor_digest bytea
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT r.project_id::text, r.evidence_kind::text, r.media_type::text,
           r.payload_schema::text, r.producer_id::text,
           r.producer_version::text, r.producer_digest, r.created_at::text,
           r.evidence_bytes, r.content_digest, r.descriptor_digest
      FROM ONLY foreman_execution.artifact_references AS r
     WHERE r.task_ref = p_task_ref AND r.attempt_number = p_attempt_number
     ORDER BY r.descriptor_digest
$$;

CREATE FUNCTION foreman_execution.read_attempt_closure_v1(
    p_task_ref bytea, p_attempt_number smallint
) RETURNS TABLE(
    provider_disposition text,
    blocker_code text,
    blocker_descriptor_digest bytea,
    reconciliation_proof_descriptor_digest bytea,
    writer_fence bigint,
    closed_at text
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT closure.provider_disposition::text, closure.blocker_code::text,
           closure.blocker_descriptor_digest,
           closure.reconciliation_proof_descriptor_digest, closure.writer_fence,
           closure.closed_at::text
      FROM ONLY foreman_execution.attempt_closures AS closure
     WHERE closure.task_ref = p_task_ref
       AND closure.attempt_number = p_attempt_number
$$;

CREATE FUNCTION foreman_execution.read_child_event_link_v1(p_event_digest bytea)
RETURNS TABLE(
    project_id text,
    project_snapshot_id text,
    task_id text,
    task_revision text,
    task_spec_digest bytea,
    accounting_currency text,
    stream_id bytea,
    before_sequence text,
    before_last_event_digest bytea,
    before_resource_revision text,
    before_resource_projection_digest bytea,
    before_head_digest bytea,
    event_sequence text,
    event_digest bytea,
    command_id text,
    request_digest bytea,
    payload_digest bytea
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT s.project_id::text, s.project_snapshot_id::text, s.task_id::text,
           s.task_revision::text, s.task_spec_digest, s.accounting_currency::text,
           e.ledger_stream_id, c.before_sequence::text,
           c.before_last_event_digest, c.before_resource_revision::text,
           c.before_resource_projection_digest, c.before_head_digest,
           e.ledger_event_sequence::text, e.ledger_event_digest,
           e.ledger_command_id::text, e.ledger_request_digest,
           e.ledger_payload_digest
      FROM ONLY foreman_execution.child_events AS e
      JOIN ONLY control.task_ledger_streams AS s ON s.stream_id = e.ledger_stream_id
      JOIN ONLY control.task_ledger_commands AS c
        ON c.stream_id = e.ledger_stream_id AND c.command_id = e.ledger_command_id
     WHERE e.ledger_event_digest = p_event_digest
       AND c.command_outcome = 'APPENDED'
       AND c.event_digest = e.ledger_event_digest
$$;

CREATE FUNCTION foreman_execution.read_task_promotion_row_v1(p_task_ref bytea)
RETURNS TABLE(
    intake_stream_id bytea,
    intake_event_digest bytea,
    project_authority_receipt_digest bytea,
    successor_stream_id bytea,
    successor_task_created_event_digest bytea,
    task_spec_digest bytea,
    approval_subject_digest bytea,
    budget_digest bytea,
    verification_policy_digest bytea,
    binding_digest bytea,
    ledger_event_digest bytea
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT p.intake_stream_id, p.intake_event_digest,
           p.project_authority_receipt_digest, p.successor_stream_id,
           p.successor_task_created_event_digest, p.task_spec_digest,
           p.approval_subject_digest, p.budget_digest,
           p.verification_policy_digest, p.binding_digest,
           p.ledger_event_digest
      FROM ONLY foreman_execution.task_promotions AS p
     WHERE p.task_ref = p_task_ref
$$;

CREATE FUNCTION foreman_execution.read_worker_attempt_rows_v1(p_task_ref bytea)
RETURNS TABLE(
    successor_stream_id bytea,
    task_spec_digest bytea,
    binding_digest bytea,
    budget_digest bytea,
    attempt_id text,
    attempt_number smallint,
    foreman_generation bigint,
    model text,
    reasoning text,
    writer_fence bigint,
    foreman_checkpoint_digest bytea,
    approval_receipt_digest bytea,
    packet_digest bytea,
    worktree_digest bytea,
    base_commit_digest bytea,
    model_reason text,
    model_reason_digest bytea,
    claimed_at text,
    payload_digest bytea,
    ledger_event_digest bytea
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT a.successor_stream_id, a.task_spec_digest, a.binding_digest,
           a.budget_digest, a.attempt_id::text, a.attempt_number,
           a.foreman_generation, a.model::text, a.reasoning::text,
           a.writer_fence, a.foreman_checkpoint_digest,
           a.approval_receipt_digest, a.packet_digest, a.worktree_digest,
           a.base_commit_digest, a.model_reason::text, a.model_reason_digest, a.claimed_at::text,
           a.payload_digest, a.ledger_event_digest
      FROM ONLY foreman_execution.worker_attempts AS a
     WHERE a.task_ref = p_task_ref
     ORDER BY a.attempt_number
$$;

CREATE FUNCTION foreman_execution.read_worker_observation_rows_v1(p_task_ref bytea)
RETURNS TABLE(
    successor_stream_id bytea,
    binding_digest bytea,
    attempt_id text,
    attempt_number smallint,
    observation_kind text,
    thread_id text,
    turn_id text,
    app_server_generation bigint,
    app_server_identity_digest bytea,
    observed_at text,
    evidence_digest bytea,
    payload_digest bytea,
    ledger_event_digest bytea
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT o.successor_stream_id, o.binding_digest, o.attempt_id::text,
           o.attempt_number, o.observation_kind::text, o.thread_id::text,
           o.turn_id::text, o.app_server_generation, o.app_server_identity_digest,
           o.observed_at::text,
           o.evidence_digest, o.payload_digest, o.ledger_event_digest
      FROM ONLY foreman_execution.worker_observations AS o
     WHERE o.task_ref = p_task_ref
     ORDER BY o.attempt_number, o.observation_ordinal
$$;

CREATE FUNCTION foreman_execution.read_verification_rows_v1(p_task_ref bytea)
RETURNS TABLE(
    successor_stream_id bytea,
    task_spec_digest bytea,
    binding_digest bytea,
    attempt_id text,
    attempt_number smallint,
    outcome text,
    verification_profile_digest bytea,
    base_commit_digest bytea,
    result_commit_digest bytea,
    tree_digest bytea,
    diff_digest bytea,
    result_digest bytea,
    evidence_artifact_digest bytea,
    review_digest bytea,
    verified_at text,
    payload_digest bytea,
    ledger_event_digest bytea
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT v.successor_stream_id, v.task_spec_digest, v.binding_digest,
           v.attempt_id::text, v.attempt_number, v.outcome::text,
           v.verification_profile_digest, v.base_commit_digest,
           v.result_commit_digest, v.tree_digest, v.diff_digest,
           v.result_digest, v.evidence_artifact_digest, v.review_digest,
           v.verified_at::text, v.payload_digest, v.ledger_event_digest
      FROM ONLY foreman_execution.verification_records AS v
     WHERE v.task_ref = p_task_ref
     ORDER BY v.attempt_number
$$;

CREATE FUNCTION foreman_execution.read_execution_authority_rows_v1(p_task_ref bytea)
RETURNS TABLE(
    successor_stream_id bytea,
    task_spec_digest bytea,
    approval_subject_digest bytea,
    budget_digest bytea,
    authority_source text,
    capability text,
    authority_evidence_digest bytea,
    approval_receipt_digest bytea,
    issued_at text,
    expires_at text,
    authority_digest bytea,
    approval_owner_snapshot_digest bytea,
    approval_owner_snapshot_content_digest bytea,
    approval_owner_snapshot_bytes bytea,
    approval_command_high_water bigint,
    approval_command_tail_digest bytea,
    approval_nonce_bindings_digest bytea
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT a.successor_stream_id, a.task_spec_digest,
           a.approval_subject_digest, a.budget_digest,
           a.authority_source::text, a.capability::text,
           a.authority_evidence_digest, a.approval_receipt_digest,
           a.issued_at::text, a.expires_at::text, a.authority_digest,
           a.approval_owner_snapshot_digest, s.snapshot_content_digest,
           s.snapshot_bytes, s.command_high_water, s.command_tail_digest,
           s.nonce_bindings_digest
      FROM ONLY foreman_execution.approval_evidence AS a
      LEFT JOIN ONLY foreman_execution.approval_owner_snapshots AS s
        ON s.snapshot_digest = a.approval_owner_snapshot_digest
     WHERE a.task_ref = p_task_ref
     ORDER BY a.authority_digest
$$;

CREATE FUNCTION foreman_execution.read_execution_authority_v1(
    p_task_ref bytea,
    p_authority_digest bytea
)
RETURNS TABLE(
    successor_stream_id bytea,
    task_spec_digest bytea,
    approval_subject_digest bytea,
    budget_digest bytea,
    authority_source text,
    capability text,
    authority_evidence_digest bytea,
    approval_receipt_digest bytea,
    issued_at text,
    expires_at text,
    authority_digest bytea,
    approval_owner_snapshot_digest bytea,
    approval_owner_snapshot_content_digest bytea,
    approval_owner_snapshot_bytes bytea,
    approval_command_high_water bigint,
    approval_command_tail_digest bytea,
    approval_nonce_bindings_digest bytea
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT a.successor_stream_id, a.task_spec_digest,
           a.approval_subject_digest, a.budget_digest,
           a.authority_source::text, a.capability::text,
           a.authority_evidence_digest, a.approval_receipt_digest,
           a.issued_at::text, a.expires_at::text, a.authority_digest,
           a.approval_owner_snapshot_digest, s.snapshot_content_digest,
           s.snapshot_bytes, s.command_high_water, s.command_tail_digest,
           s.nonce_bindings_digest
      FROM ONLY foreman_execution.approval_evidence AS a
      LEFT JOIN ONLY foreman_execution.approval_owner_snapshots AS s
        ON s.snapshot_digest = a.approval_owner_snapshot_digest
     WHERE a.task_ref = p_task_ref
       AND a.authority_digest = p_authority_digest
$$;

CREATE FUNCTION foreman_execution.read_reference_event_rows_v1(p_task_ref bytea)
RETURNS TABLE(
    record_kind text,
    attempt_number smallint,
    reference_digest bytea,
    ledger_event_digest bytea
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    SELECT 'ARTIFACT_REFERENCE'::text, r.attempt_number,
           r.descriptor_digest, r.ledger_event_digest
      FROM ONLY foreman_execution.artifact_references AS r
     WHERE r.task_ref = p_task_ref
    UNION ALL
    SELECT 'APPROVAL_EVIDENCE'::text, NULL::smallint,
           a.authority_digest, a.ledger_event_digest
      FROM ONLY foreman_execution.approval_evidence AS a
     WHERE a.task_ref = p_task_ref
     ORDER BY 1, 2 NULLS FIRST, 3
$$;

CREATE FUNCTION foreman_execution.list_restart_task_refs_v1(
    p_after_restart_priority smallint,
    p_after_task_ref bytea,
    p_limit smallint
)
RETURNS TABLE(
    task_ref bytea,
    attempt_number smallint,
    attempt_id text,
    restart_kind text,
    last_observed_at text,
    restart_priority smallint
)
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
BEGIN
    IF p_limit NOT BETWEEN 1 AND 256 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_RESTART_TASK_LIMIT_INVALID';
    END IF;
    IF (p_after_restart_priority IS NULL) <> (p_after_task_ref IS NULL)
       OR (p_after_restart_priority IS NOT NULL
           AND p_after_restart_priority NOT BETWEEN 0 AND 6)
       OR (p_after_task_ref IS NOT NULL
           AND (pg_catalog.octet_length(p_after_task_ref) <> 32
                OR p_after_task_ref = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex'))) THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_RESTART_TASK_CURSOR_INVALID';
    END IF;
    RETURN QUERY
    WITH latest_attempt AS (
        SELECT attempt.task_ref,
               pg_catalog.max(attempt.attempt_number)::smallint AS attempt_number
          FROM ONLY foreman_execution.worker_attempts AS attempt
         GROUP BY attempt.task_ref
    ), restart_candidates AS (
        -- A committed general intake is already authoritative Task Ledger
        -- DRAFT work. Discover it directly from that owner after a process
        -- crash between intake commit and Foreman promotion; no shadow queue
        -- or second task state is introduced. Every retained link and the
        -- current Project Registry identity are revalidated before exposing
        -- only the opaque task_ref to the scheduler.
        SELECT pg_catalog.decode(submission.task_ref, 'hex') AS task_ref,
               NULL::smallint AS attempt_number,
               NULL::text AS attempt_id,
               CASE WHEN project.project_id IS NOT NULL
                          AND project.project_class = 'USER_PROJECT'
                          AND project.authority_contract_version = 1
                          AND project.authority_producer_id = 'lattice-project-registry'
                          AND project.authority_producer_version = '1.0'
                          AND project.authority_runtime = 'LIVE'
                          AND project.authority_lifecycle = 'ACTIVE'
                          AND project.pending_observation_digest IS NULL
                          AND NOT project.drift_canonical_root
                          AND NOT project.drift_repository
                          AND NOT project.drift_file
                          AND NOT project.drift_primary_ref_name
                          AND NOT project.drift_primary_ref_storage
                          AND project.authority_observation_digest =
                              project.accepted_observation_digest
                    THEN 'DRAFT_PENDING_PROMOTION'::text
                    ELSE 'DRAFT_PROJECT_RECONCILIATION_REQUIRED'::text END AS restart_kind,
               intake_event.occurred_at::text AS last_observed_at
          FROM ONLY control.task_submission_envelopes AS submission
          JOIN ONLY control.task_ingress_claims AS ingress
            ON ingress.ingress_id = submission.ingress_id
           AND ingress.client_request_id = submission.client_request_id
           AND ingress.request_kind = 'GENERAL_TASK'
           AND ingress.stream_id = submission.stream_id
           AND ingress.event_sequence = submission.event_sequence
           AND ingress.event_digest = submission.event_digest
           AND ingress.command_id = submission.command_id
           AND ingress.command_request_digest = submission.request_digest
          JOIN ONLY control.task_ledger_streams AS intake_stream
            ON intake_stream.stream_id = submission.stream_id
           AND intake_stream.project_id = submission.project_id
           AND intake_stream.project_snapshot_id = submission.project_snapshot_id
           AND intake_stream.task_id = submission.task_id
           AND intake_stream.task_revision = submission.task_revision
           AND intake_stream.task_subject_kind = submission.task_subject_kind
           AND intake_stream.task_subject_digest = submission.intake_digest
           AND intake_stream.sequence = submission.event_sequence
           AND intake_stream.last_event_digest = submission.event_digest
          JOIN ONLY control.task_ledger_events AS intake_event
            ON intake_event.stream_id = submission.stream_id
           AND intake_event.sequence = submission.event_sequence
           AND intake_event.event_digest = submission.event_digest
           AND intake_event.command_id = submission.command_id
           AND intake_event.request_digest = submission.request_digest
           AND intake_event.subject_digest = submission.envelope_digest
          JOIN ONLY control.task_ledger_commands AS intake_command
            ON intake_command.stream_id = submission.stream_id
           AND intake_command.command_id = submission.command_id
           AND intake_command.request_digest = submission.request_digest
           AND intake_command.event_digest = submission.event_digest
           LEFT JOIN ONLY control.project_registry_projects AS project
             ON project.project_id = submission.project_id
            AND project.authority_snapshot_id = submission.project_snapshot_id
            AND project.authority_receipt_digest = submission.project_authority_receipt_digest
         WHERE submission.schema_version = 'lattice.task-ledger.task-submission/1.0'
           AND submission.task_subject_kind = 'GENERAL_TASK_INTAKE'
           AND submission.admission_action = 'GENERAL_TASK_INTAKE_V1'
           AND intake_stream.ledger_schema_version = '2.0'
           AND intake_stream.head_contract_version = 1
           AND intake_stream.producer_id = 'lattice-task-ledger'
           AND intake_stream.producer_version = '2.0'
           AND intake_stream.runtime = 'LIVE'
           AND intake_stream.task_spec_digest IS NULL
           AND intake_stream.accounting_currency IS NULL
           AND intake_event.event_schema_version = '2.0'
           AND intake_event.event_kind = 'TASK_CREATED'
           AND intake_event.action_id = 'GENERAL_TASK_INTAKE_V1'
           AND intake_event.audit_outcome = 'RECORDED'
           AND intake_event.reason_code = 'GENERAL_TASK_INTAKE_RECORDED'
           AND intake_event.diagnostic = 'null'::jsonb
           AND intake_command.command_outcome = 'APPENDED'
           AND intake_command.event_kind = 'TASK_CREATED'
           AND intake_command.action_id = 'GENERAL_TASK_INTAKE_V1'
           AND intake_command.audit_outcome = 'RECORDED'
           AND intake_command.reason_code = 'GENERAL_TASK_INTAKE_RECORDED'
           AND intake_command.subject_digest = submission.envelope_digest
            AND NOT EXISTS (
                   SELECT 1 FROM ONLY foreman_execution.task_promotions AS promotion
                    WHERE promotion.task_ref = pg_catalog.decode(submission.task_ref, 'hex')
               )
        UNION ALL
        SELECT promotion.task_ref,
               NULL::smallint AS attempt_number,
               NULL::text AS attempt_id,
               'PROMOTED_NO_ATTEMPT'::text AS restart_kind,
               pg_catalog.to_char(
                   child.recorded_at AT TIME ZONE 'UTC',
                   'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'
               ) AS last_observed_at
          FROM ONLY foreman_execution.task_promotions AS promotion
          JOIN ONLY foreman_execution.child_events AS child
            ON child.ledger_event_digest = promotion.ledger_event_digest
         WHERE NOT EXISTS (
                   SELECT 1 FROM ONLY foreman_execution.worker_attempts AS attempt
                    WHERE attempt.task_ref = promotion.task_ref
               )
           AND NOT EXISTS (
                   SELECT 1 FROM ONLY foreman_execution.pending_worker_claims AS pending
                    WHERE pending.task_ref = promotion.task_ref
               )
        UNION ALL
        SELECT pending.task_ref, pending.attempt_number,
               pending.attempt_id::text,
               CASE WHEN pending_project.project_id IS NOT NULL
                          AND pending_project.project_class = 'USER_PROJECT'
                          AND pending_project.authority_contract_version = 1
                          AND pending_project.authority_producer_id = 'lattice-project-registry'
                          AND pending_project.authority_producer_version = '1.0'
                          AND pending_project.authority_runtime = 'LIVE'
                          AND pending_project.authority_lifecycle = 'ACTIVE'
                          AND pending_project.pending_observation_digest IS NULL
                          AND NOT pending_project.drift_canonical_root
                          AND NOT pending_project.drift_repository
                          AND NOT pending_project.drift_file
                          AND NOT pending_project.drift_primary_ref_name
                          AND NOT pending_project.drift_primary_ref_storage
                          AND pending_project.authority_observation_digest =
                              pending_project.accepted_observation_digest
                    THEN 'CAPACITY_WAIT'::text
                    ELSE 'PROJECT_RECONCILIATION_REQUIRED'::text END,
               pg_catalog.to_char(
                   pending.reserved_at AT TIME ZONE 'UTC',
                   'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'
               )
          FROM ONLY foreman_execution.pending_worker_claims AS pending
          JOIN ONLY foreman_execution.task_promotions AS promotion
            ON promotion.task_ref = pending.task_ref
          LEFT JOIN ONLY control.project_registry_projects AS pending_project
            ON pending_project.project_id = promotion.project_id
           AND pending_project.authority_snapshot_id = promotion.project_snapshot_id
           AND pending_project.authority_receipt_digest =
               promotion.project_authority_receipt_digest
        UNION ALL
        SELECT attempt.task_ref, attempt.attempt_number,
               attempt.attempt_id::text,
               CASE WHEN EXISTS (
                   SELECT 1 FROM ONLY foreman_execution.attempt_closures AS closure
                    WHERE closure.task_ref = attempt.task_ref
                      AND closure.attempt_number = attempt.attempt_number
               ) THEN 'ATTEMPT_CLOSED_PENDING_RELEASE'::text
               WHEN EXISTS (
                   SELECT 1 FROM ONLY foreman_execution.verification_records AS verified
                    WHERE verified.task_ref = attempt.task_ref
                      AND verified.attempt_number = attempt.attempt_number
               ) THEN 'VERIFICATION_RECONCILE_REQUIRED'::text
               WHEN EXISTS (
                   SELECT 1 FROM ONLY foreman_execution.worker_observations AS terminal
                    WHERE terminal.task_ref = attempt.task_ref
                      AND terminal.attempt_number = attempt.attempt_number
                      AND terminal.observation_kind IN (
                          'TERMINAL_COMPLETED','TERMINAL_FAILED','TERMINAL_INTERRUPTED',
                          'PRESTART_TERMINAL_FAILED'
                      )
                ) THEN 'TERMINAL_PENDING_VERIFICATION'::text
               WHEN active_project.project_id IS NULL
                         OR active_project.project_class IS DISTINCT FROM 'USER_PROJECT'
                         OR active_project.authority_contract_version IS DISTINCT FROM 1
                         OR active_project.authority_producer_id IS DISTINCT FROM
                            'lattice-project-registry'
                         OR active_project.authority_producer_version IS DISTINCT FROM '1.0'
                         OR active_project.authority_runtime IS DISTINCT FROM 'LIVE'
                         OR active_project.authority_lifecycle IS DISTINCT FROM 'ACTIVE'
                         OR active_project.pending_observation_digest IS NOT NULL
                         OR active_project.drift_canonical_root IS DISTINCT FROM FALSE
                         OR active_project.drift_repository IS DISTINCT FROM FALSE
                         OR active_project.drift_file IS DISTINCT FROM FALSE
                         OR active_project.drift_primary_ref_name IS DISTINCT FROM FALSE
                         OR active_project.drift_primary_ref_storage IS DISTINCT FROM FALSE
                         OR active_project.authority_observation_digest IS DISTINCT FROM
                            active_project.accepted_observation_digest
                    THEN 'PROJECT_RECONCILIATION_REQUIRED'::text
               WHEN lease.project_id IS NULL
                         OR lease.current_status IS DISTINCT FROM 'ACTIVE'
                         OR lease.current_expires_at::timestamp with time zone <=
                            pg_catalog.clock_timestamp()
                         OR NOT EXISTS (
                             SELECT 1
                               FROM ONLY control.runtime_admission AS admission
                              WHERE admission.singleton
                                AND admission.admission_mode = 'ACTIVE'
                                AND admission.daemon_instance_id =
                                    lease.current_daemon_instance_id
                                AND admission.daemon_epoch = lease.current_daemon_epoch
                         )
                         OR lease.current_project_snapshot_id IS DISTINCT FROM
                             promotion.project_snapshot_id
                         OR lease.current_task_spec_digest IS DISTINCT FROM
                             attempt.task_spec_digest
                         OR lease.current_attempt_id IS DISTINCT FROM attempt.attempt_id
                         OR lease.current_lease_holder_id IS DISTINCT FROM 'lattice-foreman'
                         OR lease.current_worktree_id IS DISTINCT FROM
                             'WORK-' || pg_catalog.upper(pg_catalog.substr(
                                 pg_catalog.encode(attempt.task_ref, 'hex'), 1, 59
                             ))
                         OR lease.current_lease_id IS DISTINCT FROM
                             'managed-lease-' || pg_catalog.encode(attempt.task_ref, 'hex') ||
                             '-' || attempt.attempt_number::text
                         OR lease.current_fencing_token IS DISTINCT FROM attempt.writer_fence
                    THEN 'WRITER_RECONCILIATION_REQUIRED'::text
               ELSE 'ATTEMPT_RECONCILE_REQUIRED'::text END,
               COALESCE((SELECT pg_catalog.max(observed.observed_at)::text
                  FROM ONLY foreman_execution.worker_observations AS observed
                 WHERE observed.task_ref = attempt.task_ref
                   AND observed.attempt_number = attempt.attempt_number),
                   (SELECT closure.closed_at::text
                      FROM ONLY foreman_execution.attempt_closures AS closure
                     WHERE closure.task_ref = attempt.task_ref
                       AND closure.attempt_number = attempt.attempt_number))
          FROM latest_attempt AS latest
          JOIN ONLY foreman_execution.worker_attempts AS attempt
            ON attempt.task_ref = latest.task_ref
           AND attempt.attempt_number = latest.attempt_number
          JOIN ONLY foreman_execution.task_promotions AS promotion
            ON promotion.task_ref = attempt.task_ref
           LEFT JOIN ONLY control.project_registry_projects AS active_project
             ON active_project.project_id = promotion.project_id
            AND active_project.authority_snapshot_id = promotion.project_snapshot_id
            AND active_project.authority_receipt_digest =
                promotion.project_authority_receipt_digest
           LEFT JOIN ONLY writer_lease.writer_lease_heads AS lease
             ON lease.project_id = promotion.project_id
         WHERE NOT EXISTS (
                   SELECT 1 FROM ONLY foreman_execution.pending_worker_claims AS pending
                    WHERE pending.task_ref = attempt.task_ref
               )
    ), prioritized_candidates AS (
        SELECT candidate.task_ref, candidate.attempt_number,
               candidate.attempt_id, candidate.restart_kind,
               candidate.last_observed_at,
               CASE candidate.restart_kind
                   WHEN 'ATTEMPT_CLOSED_PENDING_RELEASE' THEN 0
                   WHEN 'VERIFICATION_RECONCILE_REQUIRED' THEN 1
                   WHEN 'TERMINAL_PENDING_VERIFICATION' THEN 2
                    WHEN 'PROJECT_RECONCILIATION_REQUIRED' THEN 3
                    WHEN 'ATTEMPT_RECONCILE_REQUIRED' THEN 3
                    WHEN 'WRITER_RECONCILIATION_REQUIRED' THEN 3
                   WHEN 'CAPACITY_WAIT' THEN 4
                   WHEN 'PROMOTED_NO_ATTEMPT' THEN 5
                    WHEN 'DRAFT_PENDING_PROMOTION' THEN 6
                    WHEN 'DRAFT_PROJECT_RECONCILIATION_REQUIRED' THEN 6
                   ELSE 7 END::smallint AS restart_priority
          FROM restart_candidates AS candidate
    )
    SELECT candidate.task_ref, candidate.attempt_number, candidate.attempt_id,
           candidate.restart_kind, candidate.last_observed_at,
           candidate.restart_priority
      FROM prioritized_candidates AS candidate
     WHERE p_after_restart_priority IS NULL
        OR (candidate.restart_priority, candidate.task_ref) >
           (p_after_restart_priority, p_after_task_ref)
     ORDER BY candidate.restart_priority, candidate.task_ref
     LIMIT p_limit;
END;
$$;

CREATE FUNCTION foreman_execution.list_active_task_refs_v1(p_limit smallint)
RETURNS TABLE(
    task_ref bytea,
    attempt_number smallint,
    attempt_id text,
    restart_kind text,
    last_observed_at text
)
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
BEGIN
    IF p_limit NOT BETWEEN 1 AND 256 THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001',
            MESSAGE = 'FOREMAN_ACTIVE_TASK_LIMIT_INVALID';
    END IF;
    RETURN QUERY
    WITH latest_attempt AS (
        SELECT a.task_ref, pg_catalog.max(a.attempt_number)::smallint AS attempt_number
          FROM ONLY foreman_execution.worker_attempts AS a
         GROUP BY a.task_ref
    )
    SELECT a.task_ref, a.attempt_number, a.attempt_id::text,
           CASE WHEN EXISTS (
               SELECT 1 FROM ONLY foreman_execution.worker_observations AS terminal
                WHERE terminal.task_ref = a.task_ref
                  AND terminal.attempt_number = a.attempt_number
                  AND terminal.observation_kind IN (
                      'TERMINAL_COMPLETED','TERMINAL_FAILED','TERMINAL_INTERRUPTED',
                      'PRESTART_TERMINAL_FAILED'
                  )
           ) THEN 'TERMINAL_PENDING_VERIFICATION'::text
           ELSE 'ATTEMPT_RECONCILE_REQUIRED'::text END,
           (SELECT pg_catalog.max(observed.observed_at)::text
              FROM ONLY foreman_execution.worker_observations AS observed
             WHERE observed.task_ref = a.task_ref
               AND observed.attempt_number = a.attempt_number)
      FROM latest_attempt AS latest
      JOIN ONLY foreman_execution.worker_attempts AS a
        ON a.task_ref = latest.task_ref
       AND a.attempt_number = latest.attempt_number
     WHERE NOT EXISTS (
         SELECT 1 FROM ONLY foreman_execution.verification_records AS verified
          WHERE verified.task_ref = a.task_ref
            AND verified.attempt_number = a.attempt_number
     )
       AND NOT EXISTS (
         SELECT 1 FROM ONLY foreman_execution.attempt_closures AS closure
          WHERE closure.task_ref = a.task_ref
            AND closure.attempt_number = a.attempt_number
     )
     ORDER BY a.task_ref
     LIMIT p_limit;
END;
$$;

CREATE FUNCTION foreman_execution.read_task_replay_v1(p_task_ref bytea)
RETURNS TABLE(
    record_kind text,
    record_state text,
    attempt_number smallint,
    record_ordinal bigint,
    record_digest bytea,
    ledger_stream_id bytea,
    ledger_event_sequence numeric,
    ledger_event_digest bytea,
    recorded_at text
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
    WITH replay_rows AS (
        SELECT e.record_kind::text AS record_kind,
               CASE WHEN e.record_kind = 'WORKER_ATTEMPT'
                          AND pending.ledger_event_digest IS NOT NULL
                    THEN 'PENDING_CLAIM'::text
                    ELSE 'RETAINED'::text
               END AS record_state,
               CASE e.record_kind
                   WHEN 'WORKER_ATTEMPT' THEN COALESCE(
                       a.attempt_number, pending.attempt_number
                   )
                   WHEN 'WORKER_OBSERVATION' THEN o.attempt_number
                   WHEN 'VERIFICATION' THEN v.attempt_number
                   WHEN 'ARTIFACT_REFERENCE' THEN r.attempt_number
                   ELSE NULL
               END AS attempt_number,
               CASE e.record_kind
                   WHEN 'WORKER_OBSERVATION' THEN o.observation_ordinal
                   WHEN 'ARTIFACT_REFERENCE' THEN e.ledger_event_sequence::bigint
                   ELSE 1::bigint
               END AS record_ordinal,
               CASE e.record_kind
                   WHEN 'TASK_PROMOTION' THEN p.binding_digest
                   WHEN 'WORKER_ATTEMPT' THEN COALESCE(
                       a.payload_digest, pending.payload_digest
                   )
                   WHEN 'WORKER_OBSERVATION' THEN o.payload_digest
                   WHEN 'VERIFICATION' THEN v.payload_digest
                   WHEN 'ARTIFACT_REFERENCE' THEN r.descriptor_digest
                   WHEN 'APPROVAL_EVIDENCE' THEN q.authority_digest
               END AS record_digest,
               e.ledger_stream_id, e.ledger_event_sequence,
               e.ledger_event_digest,
               pg_catalog.to_char(
                   e.recorded_at AT TIME ZONE 'UTC',
                   'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'
               ) AS recorded_at
          FROM ONLY foreman_execution.child_events AS e
          LEFT JOIN ONLY foreman_execution.task_promotions AS p ON p.ledger_event_digest = e.ledger_event_digest
          LEFT JOIN ONLY foreman_execution.worker_attempts AS a ON a.ledger_event_digest = e.ledger_event_digest
          LEFT JOIN ONLY foreman_execution.pending_worker_claims AS pending ON pending.ledger_event_digest = e.ledger_event_digest
          LEFT JOIN ONLY foreman_execution.worker_observations AS o ON o.ledger_event_digest = e.ledger_event_digest
          LEFT JOIN ONLY foreman_execution.verification_records AS v ON v.ledger_event_digest = e.ledger_event_digest
          LEFT JOIN ONLY foreman_execution.artifact_references AS r ON r.ledger_event_digest = e.ledger_event_digest
          LEFT JOIN ONLY foreman_execution.approval_evidence AS q ON q.ledger_event_digest = e.ledger_event_digest
         WHERE e.task_ref = p_task_ref
        UNION ALL
        SELECT ('PROVIDER_DISPATCH_' || dispatch.operation_kind)::text,
               'RETAINED'::text,
               dispatch.attempt_number,
               CASE dispatch.operation_kind
                   WHEN 'WORKER_THREAD' THEN 101::bigint
                   WHEN 'WORKER_TURN' THEN 102::bigint
                   WHEN 'REVIEW_THREAD' THEN 103::bigint
                   WHEN 'REVIEW_TURN' THEN 104::bigint
               END,
               dispatch.claim_receipt_digest,
               event.ledger_stream_id, event.ledger_event_sequence,
               event.ledger_event_digest,
               pg_catalog.to_char(
                   dispatch.claimed_at AT TIME ZONE 'UTC',
                   'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'
               )
          FROM ONLY foreman_execution.provider_dispatch_claims AS dispatch
          JOIN ONLY foreman_execution.worker_attempts AS attempt
            ON attempt.task_ref = dispatch.task_ref
           AND attempt.attempt_number = dispatch.attempt_number
          JOIN ONLY foreman_execution.child_events AS event
            ON event.ledger_event_digest = attempt.ledger_event_digest
         WHERE dispatch.task_ref = p_task_ref
    )
    SELECT replay.record_kind, replay.record_state, replay.attempt_number,
           replay.record_ordinal, replay.record_digest,
           replay.ledger_stream_id, replay.ledger_event_sequence,
           replay.ledger_event_digest, replay.recorded_at
      FROM replay_rows AS replay
      ORDER BY replay.ledger_event_sequence,
               CASE replay.record_kind
                   WHEN 'TASK_PROMOTION' THEN 1
                   WHEN 'WORKER_ATTEMPT' THEN 2
                   WHEN 'PROVIDER_DISPATCH_WORKER_THREAD' THEN 3
                   WHEN 'PROVIDER_DISPATCH_WORKER_TURN' THEN 4
                   WHEN 'PROVIDER_DISPATCH_REVIEW_THREAD' THEN 5
                   WHEN 'PROVIDER_DISPATCH_REVIEW_TURN' THEN 6
                   WHEN 'WORKER_OBSERVATION' THEN 7
                   WHEN 'APPROVAL_EVIDENCE' THEN 8
                   WHEN 'ARTIFACT_REFERENCE' THEN 9
                   WHEN 'VERIFICATION' THEN 10
                   ELSE 32767
               END,
               replay.record_ordinal, replay.record_kind
$$;

REVOKE ALL ON ALL TABLES IN SCHEMA foreman_execution FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA foreman_execution FROM lattice_runtime;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA foreman_execution FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA foreman_execution FROM lattice_runtime;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA foreman_execution FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA foreman_execution FROM lattice_runtime;
GRANT USAGE ON SCHEMA foreman_execution TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.record_preparation_observation_v1(
    bytea,text,text,bytea,text,bytea,text,bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.record_promotion_intent_v1(
    bytea,text,text,bytea,bytea,bytea,bytea,bytea,
    smallint,smallint,smallint,bigint,bigint,bigint,text,bigint,
    text,text,text,bytea,text,text,boolean,bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.record_task_promotion_v1(
    bytea,text,text,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,
    smallint,smallint,smallint,bigint,bigint,bigint,text,bigint,text,text,bytea,bytea,
    text,text,bytea,numeric,bytea,text,bytea,bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.reserve_worker_attempt_v1(
    bytea,bytea,bytea,bytea,bytea,text,smallint,bigint,text,text,bigint,bytea,
    bytea,bytea,text,bytea,bytea,text,bytea,text,bytea,smallint,bytea,numeric,bytea,text,bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.record_execution_environment_v1(
    bytea,smallint,text,bytea,text,text
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.claim_worker_attempt_v1(
    bytea,bytea,bytea,bytea,bytea,text,smallint,bigint,text,text,bigint,bytea,
    bytea,bytea,text,bytea,bytea,text,bytea,text,bytea,smallint,bytea,numeric,bytea,text,bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.record_worker_observation_v1(
    bytea,bytea,bytea,text,smallint,text,text,text,bigint,bytea,text,bytea,bytea,
    bytea,numeric,bytea,text,bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.record_verification_v1(
    bytea,bytea,bytea,bytea,text,smallint,text,bytea,bytea,bytea,bytea,bytea,
    bytea,bytea,bytea,text,bytea,bytea,numeric,bytea,text,bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.stage_artifact_reference_v1(
    text,bytea,smallint,text,text,text,text,text,bytea,text,bytea,bytea,bytea,bytea,
    bytea,numeric,bytea,numeric,bytea,bytea,numeric,bytea,text,bytea,bytea,text,text
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.finalize_staged_artifact_reference_v1(
    bytea,smallint,bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.claim_provider_dispatch_v1(
    bytea,smallint,text,text,bytea,bigint,bigint,bytea,bytea,bytea,bytea,bytea,bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_provider_dispatch_claim_v1(
    bytea,smallint,text
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.record_attempt_closure_v1(
    bytea,smallint,text,bytea,bigint
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.close_retained_worker_without_provider_effect_v1(
    bytea,smallint,text,bytea,bytea,bigint
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.close_pending_worker_attempt_v1(
    bytea,smallint,text,bytea,bigint
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.begin_restart_writer_blocker_guard_v1(
    bytea,smallint
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.end_restart_writer_blocker_guard_v1(
    bytea,smallint
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.record_approval_evidence_v1(
    bytea,bytea,bytea,bytea,bytea,text,text,bytea,bytea,text,text,bytea,
    bytea,bytea,bytea,bigint,bytea,bytea,
    bytea,numeric,bytea,text,bytea,bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_extension_identity_v1()
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_preparation_observation_v1(bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_promotion_intent_v1(bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_task_promotion_source_v1(bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_pending_worker_attempt_v1(bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_execution_environment_rows_v1(bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_worker_budget_v1(bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_staged_artifact_reference_v1(bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_managed_evidence_v1(bytea,smallint)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_attempt_closure_v1(bytea,smallint)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_child_event_link_v1(bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_task_promotion_row_v1(bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_worker_attempt_rows_v1(bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_worker_observation_rows_v1(bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_verification_rows_v1(bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_execution_authority_rows_v1(bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_execution_authority_v1(bytea,bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_reference_event_rows_v1(bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.list_restart_task_refs_v1(smallint,bytea,smallint)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.list_active_task_refs_v1(smallint)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION foreman_execution.read_task_replay_v1(bytea)
    TO lattice_runtime;

REVOKE CREATE ON SCHEMA foreman_execution FROM lattice_runtime;
