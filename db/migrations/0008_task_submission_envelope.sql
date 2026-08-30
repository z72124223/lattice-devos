-- General task intake: exact schema-v6 to schema-v7 authoritative submission binding.
-- The migration runner owns the surrounding transaction. Historical migrations stay immutable.

-- Every retained pre-v7 stream is a Task-Spec stream. Preserve its canonical
-- Task Ledger identity while adding a neutral, closed subject discriminator for
-- schema-v7 general intake. The legacy Task-Spec columns remain populated only
-- for TASK_SPEC and are structurally NULL for GENERAL_TASK_INTAKE.
-- Project Registry snapshots can occupy 159 ASCII bytes at the closed maximum:
-- 64-byte project id + ':registry:' + 20-digit u64 revision + ':' + 64 hex.
ALTER TABLE control.physical_heads
    DROP CONSTRAINT physical_heads_snapshot_id,
    ALTER COLUMN project_snapshot_id TYPE varchar(159),
    ADD CONSTRAINT physical_heads_snapshot_id CHECK (
        project_snapshot_id ~ '^[a-z0-9._:-]{1,159}$'
    );
ALTER TABLE control.terminal_transactions
    DROP CONSTRAINT terminal_transactions_snapshot_id,
    ALTER COLUMN project_snapshot_id TYPE varchar(159),
    ADD CONSTRAINT terminal_transactions_snapshot_id CHECK (
        project_snapshot_id ~ '^[a-z0-9._:-]{1,159}$'
    );
ALTER TABLE control.task_ledger_streams
    ALTER COLUMN project_snapshot_id TYPE varchar(159);

ALTER TABLE control.task_ledger_streams
    ADD COLUMN task_subject_kind varchar(32),
    ADD COLUMN task_subject_digest bytea;
UPDATE ONLY control.task_ledger_streams
   SET task_subject_kind = 'TASK_SPEC',
       task_subject_digest = task_spec_digest;
ALTER TABLE control.task_ledger_streams
    ALTER COLUMN task_subject_kind SET NOT NULL,
    ALTER COLUMN task_subject_digest SET NOT NULL,
    ALTER COLUMN task_spec_digest DROP NOT NULL,
    ALTER COLUMN accounting_currency DROP NOT NULL,
    DROP CONSTRAINT task_ledger_streams_identity_shape,
    DROP CONSTRAINT task_ledger_streams_digest_shapes;
ALTER TABLE control.task_ledger_streams
    ADD CONSTRAINT task_ledger_streams_identity_shape CHECK (
        project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'
        AND project_snapshot_id ~ '^[a-z0-9._:-]{1,159}$'
        AND task_id ~ '^TASK-[A-Z0-9][A-Z0-9_-]{2,63}$'
    ),
    ADD CONSTRAINT task_ledger_streams_subject_shape CHECK (
        task_subject_kind IN ('TASK_SPEC', 'GENERAL_TASK_INTAKE')
        AND pg_catalog.octet_length(task_subject_digest) = 32
        AND task_subject_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND (
            (task_subject_kind = 'TASK_SPEC'
             AND task_spec_digest IS NOT NULL
             AND pg_catalog.octet_length(task_spec_digest) = 32
             AND task_spec_digest = task_subject_digest
             AND accounting_currency IS NOT NULL
             AND accounting_currency ~ '^[A-Z]{3}$')
            OR
            (task_subject_kind = 'GENERAL_TASK_INTAKE'
             AND task_spec_digest IS NULL
             AND accounting_currency IS NULL)
        )
    ),
    ADD CONSTRAINT task_ledger_streams_digest_shapes CHECK (
        pg_catalog.octet_length(last_event_digest) = 32
        AND pg_catalog.octet_length(resource_projection_digest) = 32
        AND pg_catalog.octet_length(head_digest) = 32
        AND head_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(checkpoint_digest) = 32
        AND checkpoint_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    );

ALTER TABLE control.task_ledger_commands
    DROP CONSTRAINT task_ledger_commands_closed_values;
ALTER TABLE control.task_ledger_commands
    ADD CONSTRAINT task_ledger_commands_closed_values CHECK (
        event_kind IN ('TASK_CREATED', 'AUTONOMY_RECEIPT_RECORDED',
            'FOREMAN_SNAPSHOT_RECORDED', 'STATE_TRANSITION', 'POLICY_DECISION',
            'RESOURCE_SNAPSHOT', 'EFFECT_INTENT', 'EFFECT_OUTCOME', 'EVIDENCE_RECORDED')
        AND audit_outcome IN ('RECORDED', 'ALLOWED', 'DENIED', 'PASSED', 'FAILED', 'BLOCKED', 'CANCELLED')
        AND command_outcome IN ('APPENDED', 'DENIED')
        AND ((command_outcome = 'APPENDED' AND denial_reason = ''
              AND event_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex'))
             OR (command_outcome = 'DENIED'
                 AND denial_reason IN ('STALE_HEAD', 'SEQUENCE_OVERFLOW')
                 AND event_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')))
    );

ALTER TABLE control.task_ledger_events
    DROP CONSTRAINT task_ledger_events_closed_values;
ALTER TABLE control.task_ledger_events
    ADD CONSTRAINT task_ledger_events_closed_values CHECK (
        event_kind IN ('TASK_CREATED', 'AUTONOMY_RECEIPT_RECORDED',
            'FOREMAN_SNAPSHOT_RECORDED', 'STATE_TRANSITION', 'POLICY_DECISION',
            'RESOURCE_SNAPSHOT', 'EFFECT_INTENT', 'EFFECT_OUTCOME', 'EVIDENCE_RECORDED')
        AND audit_outcome IN ('RECORDED', 'ALLOWED', 'DENIED', 'PASSED', 'FAILED', 'BLOCKED', 'CANCELLED')
    );

ALTER TABLE control.project_registry_commands
    DROP CONSTRAINT project_registry_commands_persistence_profile;
ALTER TABLE control.project_registry_commands
    ADD CONSTRAINT project_registry_commands_persistence_profile CHECK (
        persistence_schema_version BETWEEN 4 AND 7
        AND persistence_manifest_sha256 ~ '^[0-9a-f]{64}$'
        AND persistence_manifest_sha256 <> pg_catalog.repeat('0', 64)
    );

CREATE TABLE IF NOT EXISTS control.task_ledger_foreman_snapshots (
    stream_id bytea NOT NULL,
    event_sequence numeric(20,0) NOT NULL,
    event_digest bytea NOT NULL,
    command_id varchar(128) NOT NULL,
    request_digest bytea NOT NULL,
    record_schema varchar(64) NOT NULL,
    payload_schema varchar(64) NOT NULL,
    payload_digest bytea NOT NULL,
    worker_id varchar(256) NOT NULL,
    thread_id varchar(256) NOT NULL,
    task_id varchar(256) NOT NULL,
    branch_ref varchar(256) NOT NULL,
    worktree_ref varchar(256) NOT NULL,
    head_sha1 char(40) NOT NULL,
    foreman_state varchar(16) NOT NULL,
    blocker_ref varchar(256),
    heartbeat_digest_ref varchar(96) NOT NULL,
    authority_digest_ref varchar(96) NOT NULL,
    evidence_digest_ref varchar(96) NOT NULL,
    generation numeric(20,0) NOT NULL,
    epistemic_schema varchar(64),
    observed_fact_refs text[],
    hypothesis_refs text[],
    confidence varchar(16),
    unknown_refs text[],
    evidence_refs text[],
    counterevidence_refs text[],
    checked_at char(20),
    expires_at char(20),
    refresh_trigger varchar(32),
    decision_ref varchar(96),
    probe_ref varchar(96),
    falsifier_ref varchar(96),
    PRIMARY KEY (stream_id, event_sequence),
    UNIQUE (event_digest),
    UNIQUE (stream_id, command_id),
    FOREIGN KEY (stream_id, event_sequence)
        REFERENCES control.task_ledger_events (stream_id, sequence),
    FOREIGN KEY (stream_id, command_id)
        REFERENCES control.task_ledger_commands (stream_id, command_id),
    CHECK (pg_catalog.octet_length(stream_id) = 32
        AND pg_catalog.octet_length(event_digest) = 32
        AND pg_catalog.octet_length(request_digest) = 32
        AND pg_catalog.octet_length(payload_digest) = 32
        AND event_sequence >= 1 AND event_sequence <= 18446744073709551615
        AND generation >= 1 AND generation <= 18446744073709551615),
    CHECK (record_schema = 'lattice.task-ledger.foreman-record/1.0'
        AND payload_schema = 'lattice.foreman-snapshot/1.0'
        AND worker_id ~ '^[!-~]+$'
        AND thread_id ~ '^[!-~]+$'
        AND task_id ~ '^[!-~]+$'
        AND branch_ref ~ '^[!-~]+$'
        AND worktree_ref ~ '^[!-~]+$'
        AND head_sha1 ~ '^[0-9a-f]{40}$'
        AND foreman_state IN ('ACTIVE', 'BLOCKED', 'COMPLETED')
        AND heartbeat_digest_ref ~ '^heartbeat:sha256:[0-9a-f]{64}$'
        AND authority_digest_ref ~ '^authority:sha256:[0-9a-f]{64}$'
        AND evidence_digest_ref ~ '^evidence:sha256:[0-9a-f]{64}$'
        AND worker_id !~* '^(sk-|bearer )|password|full chat|begin private'
        AND thread_id !~* '^(sk-|bearer )|password|full chat|begin private'
        AND task_id !~* '^(sk-|bearer )|password|full chat|begin private'
        AND branch_ref !~* '^(sk-|bearer )|password|full chat|begin private'
        AND worktree_ref !~* '^(sk-|bearer )|password|full chat|begin private'
        AND (blocker_ref IS NULL OR (blocker_ref ~ '^[!-~]+$'
             AND blocker_ref !~* '^(sk-|bearer )|password|full chat|begin private'))
        AND ((foreman_state = 'BLOCKED' AND blocker_ref IS NOT NULL)
             OR (foreman_state <> 'BLOCKED' AND blocker_ref IS NULL))),
    CHECK ((epistemic_schema IS NULL
            AND observed_fact_refs IS NULL AND hypothesis_refs IS NULL
            AND confidence IS NULL AND unknown_refs IS NULL
            AND evidence_refs IS NULL AND counterevidence_refs IS NULL
            AND checked_at IS NULL AND expires_at IS NULL
            AND refresh_trigger IS NULL AND decision_ref IS NULL
            AND probe_ref IS NULL AND falsifier_ref IS NULL)
        OR (epistemic_schema = 'lattice.foreman-epistemic/1.0'
            AND observed_fact_refs IS NOT NULL AND hypothesis_refs IS NOT NULL
            AND confidence IN ('UNKNOWN','LOW','MEDIUM','HIGH')
            AND unknown_refs IS NOT NULL AND evidence_refs IS NOT NULL
            AND counterevidence_refs IS NOT NULL AND checked_at IS NOT NULL
            AND expires_at IS NOT NULL
            AND refresh_trigger IN ('EXPIRY','NEW_EVIDENCE','COUNTEREVIDENCE','DEPENDENCY_CHANGE')
            AND checked_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$'
            AND expires_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$'
            AND expires_at > checked_at
            AND pg_catalog.cardinality(observed_fact_refs) <= 64
            AND pg_catalog.cardinality(hypothesis_refs) <= 64
            AND pg_catalog.cardinality(unknown_refs) <= 64
            AND pg_catalog.cardinality(evidence_refs) <= 64
            AND pg_catalog.cardinality(counterevidence_refs) <= 64
            AND pg_catalog.array_position(observed_fact_refs, NULL) IS NULL
            AND pg_catalog.array_position(hypothesis_refs, NULL) IS NULL
            AND pg_catalog.array_position(unknown_refs, NULL) IS NULL
            AND pg_catalog.array_position(evidence_refs, NULL) IS NULL
            AND pg_catalog.array_position(counterevidence_refs, NULL) IS NULL
            AND pg_catalog.array_to_string(observed_fact_refs, ',')
                ~ '^$|^fact:sha256:[0-9a-f]{64}(,fact:sha256:[0-9a-f]{64})*$'
            AND pg_catalog.array_to_string(hypothesis_refs, ',')
                ~ '^$|^hypothesis:sha256:[0-9a-f]{64}(,hypothesis:sha256:[0-9a-f]{64})*$'
            AND pg_catalog.array_to_string(unknown_refs, ',')
                ~ '^$|^unknown:sha256:[0-9a-f]{64}(,unknown:sha256:[0-9a-f]{64})*$'
            AND pg_catalog.array_to_string(evidence_refs, ',')
                ~ '^$|^evidence:sha256:[0-9a-f]{64}(,evidence:sha256:[0-9a-f]{64})*$'
            AND pg_catalog.array_to_string(counterevidence_refs, ',')
                ~ '^$|^counterevidence:sha256:[0-9a-f]{64}(,counterevidence:sha256:[0-9a-f]{64})*$'
            AND decision_ref ~ '^decision:sha256:[0-9a-f]{64}$'
            AND probe_ref ~ '^probe:sha256:[0-9a-f]{64}$'
            AND falsifier_ref ~ '^falsifier:sha256:[0-9a-f]{64}$'))
);
REVOKE ALL ON TABLE control.task_ledger_foreman_snapshots FROM PUBLIC;
REVOKE ALL ON TABLE control.task_ledger_foreman_snapshots FROM lattice_runtime;
REVOKE ALL ON TABLE control.task_ledger_foreman_snapshots FROM lattice_guardian;
REVOKE ALL ON TABLE control.task_ledger_foreman_snapshots FROM lattice_readonly;

CREATE OR REPLACE FUNCTION control.store_prepare_v5(
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
AS $lattice_store_prepare_v5$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
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
       OR p_project_snapshot_id !~ '^[a-z0-9._:-]{1,159}$'
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
       OR v_schema_version IS DISTINCT FROM 7
       OR v_min_reader IS DISTINCT FROM 7
       OR v_max_reader IS DISTINCT FROM 7
       OR v_min_writer IS DISTINCT FROM 7
       OR v_max_writer IS DISTINCT FROM 7
       OR v_manifest_sha256 IS NULL
       OR v_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR v_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
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
$lattice_store_prepare_v5$;

CREATE OR REPLACE FUNCTION control.store_finalize_v5(
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
AS $lattice_store_finalize_v5$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    SELECT *
      INTO v_prepare
      FROM control.store_prepare_v5(
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
$lattice_store_finalize_v5$;

CREATE OR REPLACE FUNCTION control.store_current_head_v5(
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
AS $lattice_store_current_head_v5$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR p_project_id IS NULL
       OR p_project_id !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
       OR p_project_snapshot_id IS NULL
       OR p_project_snapshot_id !~ '^[a-z0-9._:-]{1,159}$'
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
       AND c.current_schema_version = 7
       AND c.min_reader = 7
       AND c.max_reader = 7
       AND c.min_writer = 7
       AND c.max_writer = 7
       AND pg_catalog.btrim(c.manifest_sha256::text) ~ '^[0-9a-f]{64}$'
       AND pg_catalog.btrim(c.manifest_sha256::text) <> pg_catalog.repeat('0', 64)
       AND v_manifest_entry_count = 8
       AND pg_catalog.btrim(c.manifest_sha256::text) = v_history_manifest_sha256;
END;
$lattice_store_current_head_v5$;

CREATE OR REPLACE FUNCTION control.task_ledger_prepare_v3(
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
AS $lattice_task_ledger_prepare_v3$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
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

    IF v_manifest_entry_count IS DISTINCT FROM 8 OR (
        SELECT pg_catalog.count(*)
          FROM ONLY control.schema_compatibility AS c
         WHERE c.singleton = true
           AND c.current_schema_version = 7
           AND c.min_reader = 7 AND c.max_reader = 7
           AND c.min_writer = 7 AND c.max_writer = 7
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
$lattice_task_ledger_prepare_v3$;

CREATE OR REPLACE FUNCTION control.task_ledger_read_head_v3(
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
AS $lattice_task_ledger_read_head_v3$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
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
       OR p_expected_project_snapshot_id !~ '^[a-z0-9._:-]{1,159}$'
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
       OR v_global_schema_version IS DISTINCT FROM 7
       OR v_min_reader IS DISTINCT FROM 7
       OR v_max_reader IS DISTINCT FROM 7
       OR v_min_writer IS DISTINCT FROM 7
       OR v_max_writer IS DISTINCT FROM 7
       OR v_global_manifest_sha256 IS NULL
       OR v_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR v_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
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
$lattice_task_ledger_read_head_v3$;

CREATE FUNCTION control.task_ledger_read_head_v4(
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
    task_subject_kind text,
    task_subject_digest bytea,
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
LANGUAGE sql
STABLE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_task_ledger_read_head_v4$
    SELECT h.stream_id,h.ledger_schema_version,h.head_contract_version,
           h.producer_id,h.producer_version,h.runtime,h.project_id,
           h.project_snapshot_id,h.task_id,h.task_revision,
           s.task_subject_kind::text,s.task_subject_digest,
           h.task_spec_digest,h.accounting_currency,h.sequence,
           h.last_event_digest,h.resource_revision,h.resource_projection_digest,
           h.head_digest,h.active_agents,h.active_implementers,h.elapsed_seconds,
           h.attempt_number,h.used_model_calls,h.used_external_cost,
           h.retained_event_count,h.retained_command_count,h.retained_outbox_count,
           h.checkpoint_schema_version,h.checkpoint_digest,h.actual_event_count,
           h.actual_command_count,h.actual_outbox_count,h.physical_head_found,
           h.physical_revision,h.physical_state_digest,h.physical_head_digest,
           h.global_schema_version,h.global_manifest_sha256
      FROM control.task_ledger_read_head_v3(
               p_global_schema_version,p_global_manifest_sha256,p_stream_id,
               p_expected_project_id,p_expected_project_snapshot_id) AS h
      JOIN ONLY control.task_ledger_streams AS s ON s.stream_id=h.stream_id
$lattice_task_ledger_read_head_v4$;

CREATE OR REPLACE FUNCTION control.task_ledger_read_events_v3(
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
AS $lattice_task_ledger_read_events_v3$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
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
$lattice_task_ledger_read_events_v3$;

CREATE OR REPLACE FUNCTION control.task_ledger_read_commands_v3(
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
AS $lattice_task_ledger_read_commands_v3$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
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
$lattice_task_ledger_read_commands_v3$;

CREATE OR REPLACE FUNCTION control.task_ledger_finalize_v3(
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
AS $lattice_task_ledger_finalize_v3$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
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
       OR p_project_snapshot_id !~ '^[a-z0-9._:-]{1,159}$'
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
            'TASK_CREATED', 'AUTONOMY_RECEIPT_RECORDED', 'STATE_TRANSITION', 'POLICY_DECISION',
            'RESOURCE_SNAPSHOT', 'EFFECT_INTENT', 'EFFECT_OUTCOME',
            'EVIDENCE_RECORDED', 'FOREMAN_SNAPSHOT_RECORDED'
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
           OR v_stream.task_subject_kind IS DISTINCT FROM 'TASK_SPEC'
           OR v_stream.task_subject_digest IS DISTINCT FROM p_task_spec_digest
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
            project_snapshot_id, task_id, task_revision, task_subject_kind,
            task_subject_digest, task_spec_digest, accounting_currency,
            sequence, last_event_digest, resource_revision,
            resource_projection_digest, head_digest, active_agents,
            active_implementers, elapsed_seconds, attempt_number, used_model_calls,
            used_external_cost, event_count, command_count, outbox_count,
            checkpoint_schema_version, checkpoint_digest
        ) VALUES (
            p_stream_id, '2.0', 1, 'lattice-task-ledger', '2.0', 'LIVE',
            p_project_id, p_project_snapshot_id, p_task_id, p_task_revision::numeric,
            'TASK_SPEC', p_task_spec_digest, p_task_spec_digest,
            p_accounting_currency, p_next_sequence::numeric,
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
$lattice_task_ledger_finalize_v3$;

-- General intake has a deliberately separate finalizer. It can create only a
-- vacant GENERAL_TASK_INTAKE stream and its first TASK_CREATED record. It has
-- no resource, outbox, autonomy, foreman, execution, or Writer-Lease lane.
CREATE FUNCTION control.task_ledger_finalize_general_intake_v1(
    p_global_schema_version smallint,
    p_global_manifest_sha256 text,
    p_stream_id bytea,
    p_project_id text,
    p_project_snapshot_id text,
    p_task_id text,
    p_task_revision text,
    p_task_subject_kind text,
    p_task_subject_digest bytea,
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
    p_actor_id text,
    p_event_subject_digest bytea,
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
    p_event_digest bytea,
    p_receipt_digest bytea,
    p_record_set_digest bytea,
    p_store_transaction_id text,
    p_event_sequence text,
    p_previous_event_digest bytea,
    p_event_resource_revision text,
    p_event_resource_projection_digest bytea
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
AS $lattice_task_ledger_finalize_general_intake_v1$
DECLARE
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
    v_terminal control.terminal_transactions%ROWTYPE;
    v_terminal_current_xact boolean;
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = 7
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
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
       OR p_project_snapshot_id !~ '^[a-z0-9._:-]{1,159}$'
       OR p_task_id IS NULL
       OR p_task_id !~ '^TASK-[A-Z0-9][A-Z0-9_-]{2,63}$'
       OR p_task_subject_kind IS DISTINCT FROM 'GENERAL_TASK_INTAKE'
       OR p_command_id IS NULL
       OR p_command_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR p_correlation_id IS NULL
       OR p_correlation_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR p_actor_id IS NULL
       OR p_actor_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR p_occurred_at IS NULL
       OR p_occurred_at !~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$'
       OR p_store_transaction_id IS NULL
       OR p_store_transaction_id !~ '^task-ledger-v1:[0-9a-f]{64}$'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid general intake finalization';
    END IF;

    FOREACH v_text IN ARRAY ARRAY[
        p_task_revision,p_next_sequence,p_next_resource_revision,
        p_next_active_agents,p_next_active_implementers,p_next_elapsed_seconds,
        p_next_attempt_number,p_next_used_model_calls,p_next_event_count,
        p_next_command_count,p_next_outbox_count,p_expected_sequence,
        p_expected_resource_revision,p_before_sequence,p_before_resource_revision,
        p_after_sequence,p_after_resource_revision,p_event_sequence,
        p_event_resource_revision
    ]
    LOOP
        IF v_text IS NULL
           OR v_text !~ '^(0|[1-9][0-9]{0,19})$'
           OR v_text::numeric > 18446744073709551615
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid general intake u64';
        END IF;
    END LOOP;

    FOREACH v_digest IN ARRAY ARRAY[
        p_task_subject_digest,p_next_last_event_digest,
        p_next_resource_projection_digest,p_next_head_digest,
        p_base_checkpoint_digest,p_next_checkpoint_digest,p_request_digest,
        p_expected_last_event_digest,p_expected_resource_projection_digest,
        p_expected_head_digest,p_event_subject_digest,p_before_last_event_digest,
        p_before_resource_projection_digest,p_before_head_digest,
        p_after_last_event_digest,p_after_resource_projection_digest,
        p_after_head_digest,p_event_digest,p_receipt_digest,p_record_set_digest,
        p_previous_event_digest,p_event_resource_projection_digest
    ]
    LOOP
        IF v_digest IS NULL OR pg_catalog.octet_length(v_digest) <> 32 THEN
            RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid general intake digest';
        END IF;
    END LOOP;

    IF p_task_revision::numeric < 1
       OR p_task_subject_digest = v_zero
       OR p_next_sequence::numeric <> 1
       OR p_next_last_event_digest IS DISTINCT FROM p_event_digest
       OR p_next_resource_revision::numeric <> 0
       OR p_next_resource_projection_digest <> v_zero
       OR p_next_head_digest = v_zero
       OR p_next_active_agents::numeric <> 0
       OR p_next_active_implementers::numeric <> 0
       OR p_next_elapsed_seconds::numeric <> 0
       OR p_next_attempt_number::numeric <> 0
       OR p_next_used_model_calls::numeric <> 0
       OR p_next_used_external_cost IS DISTINCT FROM '0'
       OR p_next_event_count::numeric <> 1
       OR p_next_command_count::numeric <> 1
       OR p_next_outbox_count::numeric <> 0
       OR p_base_checkpoint_digest = v_zero
       OR p_next_checkpoint_digest = v_zero
       OR p_base_checkpoint_digest = p_next_checkpoint_digest
       OR p_request_digest = v_zero
       OR p_expected_sequence::numeric <> 0
       OR p_expected_last_event_digest <> v_zero
       OR p_expected_resource_revision::numeric <> 0
       OR p_expected_resource_projection_digest <> v_zero
       OR p_expected_head_digest = v_zero
       OR p_event_subject_digest = v_zero
       OR p_before_sequence::numeric <> 0
       OR p_before_last_event_digest <> v_zero
       OR p_before_resource_revision::numeric <> 0
       OR p_before_resource_projection_digest <> v_zero
       OR p_before_head_digest IS DISTINCT FROM p_expected_head_digest
       OR p_after_sequence::numeric <> 1
       OR p_after_last_event_digest IS DISTINCT FROM p_event_digest
       OR p_after_resource_revision::numeric <> 0
       OR p_after_resource_projection_digest <> v_zero
       OR p_after_head_digest IS DISTINCT FROM p_next_head_digest
       OR p_event_digest = v_zero
       OR p_receipt_digest = v_zero
       OR p_record_set_digest = v_zero
       OR p_event_sequence::numeric <> 1
       OR p_previous_event_digest <> v_zero
       OR p_event_resource_revision::numeric <> 0
       OR p_event_resource_projection_digest <> v_zero
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST02', MESSAGE = 'general intake shape mismatch';
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(
            'lattice.task-ledger.stream.v1:' || pg_catalog.encode(p_stream_id, 'hex'),
            0
        )
    );
    IF EXISTS (SELECT 1 FROM ONLY control.task_ledger_streams AS s WHERE s.stream_id=p_stream_id)
       OR EXISTS (SELECT 1 FROM ONLY control.task_ledger_commands AS c WHERE c.stream_id=p_stream_id)
       OR EXISTS (SELECT 1 FROM ONLY control.task_ledger_events AS e WHERE e.stream_id=p_stream_id)
       OR EXISTS (SELECT 1 FROM ONLY control.task_ledger_outbox AS o WHERE o.stream_id=p_stream_id)
       OR EXISTS (SELECT 1 FROM ONLY control.task_ledger_autonomy_receipts AS a WHERE a.stream_id=p_stream_id)
       OR EXISTS (SELECT 1 FROM ONLY control.task_ledger_foreman_snapshots AS f WHERE f.stream_id=p_stream_id)
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCP01', MESSAGE = 'general intake stream not vacant';
    END IF;

    SELECT t.*
      INTO v_terminal
      FROM ONLY control.terminal_transactions AS t
     WHERE t.transaction_id = p_store_transaction_id
     FOR SHARE OF t;
    IF FOUND THEN
        SELECT t.xmin = pg_catalog.pg_current_xact_id()::xid
          INTO v_terminal_current_xact
          FROM ONLY control.terminal_transactions AS t
         WHERE t.transaction_id = p_store_transaction_id;
    ELSE
        v_terminal_current_xact := false;
    END IF;
    IF v_terminal_current_xact IS DISTINCT FROM true
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
       OR v_terminal.outbox_intent_digest IS NOT NULL
       OR v_terminal.disposition IS DISTINCT FROM 'APPLIED'
       OR v_terminal.expected_revision IS DISTINCT FROM 0
       OR v_terminal.before_revision IS DISTINCT FROM 0
       OR v_terminal.after_revision IS DISTINCT FROM 1
       OR v_terminal.after_state_digest IS DISTINCT FROM p_next_checkpoint_digest
       OR v_terminal.schema_version IS DISTINCT FROM 2
       OR pg_catalog.btrim(v_terminal.manifest_sha256::text) IS DISTINCT FROM
            '4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'general intake store pair corrupt';
    END IF;

    INSERT INTO control.task_ledger_streams (
        stream_id,ledger_schema_version,head_contract_version,producer_id,
        producer_version,runtime,project_id,project_snapshot_id,task_id,
        task_revision,task_subject_kind,task_subject_digest,task_spec_digest,
        accounting_currency,sequence,last_event_digest,resource_revision,
        resource_projection_digest,head_digest,active_agents,active_implementers,
        elapsed_seconds,attempt_number,used_model_calls,used_external_cost,
        event_count,command_count,outbox_count,checkpoint_schema_version,
        checkpoint_digest
    ) VALUES (
        p_stream_id,'2.0',1,'lattice-task-ledger','2.0','LIVE',p_project_id,
        p_project_snapshot_id,p_task_id,p_task_revision::numeric,
        p_task_subject_kind,p_task_subject_digest,NULL,NULL,p_next_sequence::numeric,
        p_next_last_event_digest,p_next_resource_revision::numeric,
        p_next_resource_projection_digest,p_next_head_digest,
        p_next_active_agents::numeric,p_next_active_implementers::numeric,
        p_next_elapsed_seconds::numeric,p_next_attempt_number::numeric,
        p_next_used_model_calls::numeric,p_next_used_external_cost,
        p_next_event_count::numeric,p_next_command_count::numeric,
        p_next_outbox_count::numeric,'1.0',p_next_checkpoint_digest
    );

    INSERT INTO control.task_ledger_commands (
        stream_id,command_id,request_schema_version,request_digest,
        expected_sequence,expected_last_event_digest,expected_resource_revision,
        expected_resource_projection_digest,expected_head_digest,correlation_id,
        occurred_at,event_kind,actor_id,action_id,audit_outcome,reason_code,
        subject_digest,diagnostic,has_resource_snapshot,resource_active_agents,
        resource_active_implementers,resource_elapsed_seconds,
        resource_attempt_number,resource_used_model_calls,resource_used_external_cost,
        receipt_schema_version,before_sequence,before_last_event_digest,
        before_resource_revision,before_resource_projection_digest,before_head_digest,
        after_sequence,after_last_event_digest,after_resource_revision,
        after_resource_projection_digest,after_head_digest,command_outcome,
        denial_reason,event_digest,receipt_digest,base_checkpoint_digest,
        result_checkpoint_digest,record_set_digest,store_transaction_id
    ) VALUES (
        p_stream_id,p_command_id,'2.0',p_request_digest,0,v_zero,0,v_zero,
        p_expected_head_digest,p_correlation_id,p_occurred_at,'TASK_CREATED',
        p_actor_id,'GENERAL_TASK_INTAKE_V1','RECORDED','GENERAL_TASK_INTAKE_RECORDED',
        p_event_subject_digest,'null'::jsonb,false,0,0,0,0,0,'0','2.0',0,v_zero,
        0,v_zero,p_before_head_digest,1,p_event_digest,0,v_zero,p_after_head_digest,
        'APPENDED','',p_event_digest,p_receipt_digest,p_base_checkpoint_digest,
        p_next_checkpoint_digest,p_record_set_digest,p_store_transaction_id
    );

    INSERT INTO control.task_ledger_events (
        stream_id,sequence,event_schema_version,previous_event_digest,command_id,
        request_digest,correlation_id,occurred_at,event_kind,actor_id,action_id,
        audit_outcome,reason_code,subject_digest,diagnostic,has_resource_snapshot,
        resource_active_agents,resource_active_implementers,resource_elapsed_seconds,
        resource_attempt_number,resource_used_model_calls,resource_used_external_cost,
        resource_revision,resource_projection_digest,event_digest
    ) VALUES (
        p_stream_id,1,'2.0',v_zero,p_command_id,p_request_digest,p_correlation_id,
        p_occurred_at,'TASK_CREATED',p_actor_id,'GENERAL_TASK_INTAKE_V1','RECORDED',
        'GENERAL_TASK_INTAKE_RECORDED',p_event_subject_digest,'null'::jsonb,false,
        0,0,0,0,0,'0',0,v_zero,p_event_digest
    );

    IF (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_streams AS s WHERE s.stream_id=p_stream_id) <> 1
       OR (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_commands AS c WHERE c.stream_id=p_stream_id) <> 1
       OR (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_events AS e WHERE e.stream_id=p_stream_id) <> 1
       OR (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_outbox AS o WHERE o.stream_id=p_stream_id) <> 0
       OR (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_autonomy_receipts AS a WHERE a.stream_id=p_stream_id) <> 0
       OR (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_foreman_snapshots AS f WHERE f.stream_id=p_stream_id) <> 0
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'general intake row count corrupt';
    END IF;
    RETURN 'FINALIZED';
END;
$lattice_task_ledger_finalize_general_intake_v1$;

CREATE OR REPLACE FUNCTION control.project_registry_prepare_v2(
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
AS $lattice_project_registry_prepare_v2$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
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
$lattice_project_registry_prepare_v2$;

CREATE OR REPLACE FUNCTION control.project_registry_read_state_v2(
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
AS $lattice_project_registry_read_state_v2$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
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
$lattice_project_registry_read_state_v2$;

CREATE OR REPLACE FUNCTION control.project_registry_read_observations_v2(
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
AS $lattice_project_registry_read_observations_v2$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
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
$lattice_project_registry_read_observations_v2$;

CREATE OR REPLACE FUNCTION control.project_registry_read_projects_v2(
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
AS $lattice_project_registry_read_projects_v2$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
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
$lattice_project_registry_read_projects_v2$;

CREATE OR REPLACE FUNCTION control.project_registry_read_commands_v2(
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
AS $lattice_project_registry_read_commands_v2$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    RETURN QUERY
    SELECT c.* FROM ONLY control.project_registry_commands AS c ORDER BY c.ordinal;
END;
$lattice_project_registry_read_commands_v2$;

CREATE OR REPLACE FUNCTION control.project_registry_read_reservations_v2(
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
AS $lattice_project_registry_read_reservations_v2$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
              AND pg_catalog.btrim(c.manifest_sha256::text) = p_global_manifest_sha256) <> 1
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'global schema profile not current';
    END IF;
    RETURN QUERY
    SELECT r.dimension::text, r.identity_digest, r.reservation_status::text,
           r.project_id::text FROM ONLY control.project_registry_identity_reservations AS r
      ORDER BY r.dimension, r.identity_digest, r.reservation_status, r.project_id;
END;
$lattice_project_registry_read_reservations_v2$;

CREATE OR REPLACE FUNCTION control.project_registry_stage_command_v2(
    p_global_schema_version smallint, p_global_manifest_sha256 text,
    p_persistence_schema_version smallint, p_persistence_manifest_sha256 text,
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
AS $lattice_project_registry_stage_command_v2$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
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
       OR p_persistence_schema_version IS DISTINCT FROM 7
       OR p_persistence_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
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
        p_daemon_head_digest, p_transaction_digest, p_persistence_receipt_digest,
        p_persistence_schema_version, p_persistence_manifest_sha256
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
$lattice_project_registry_stage_command_v2$;

CREATE OR REPLACE FUNCTION control.project_registry_stage_project_v2(
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
AS $lattice_project_registry_stage_project_v2$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
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
$lattice_project_registry_stage_project_v2$;

CREATE OR REPLACE FUNCTION control.project_registry_finalize_v2(
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
AS $lattice_project_registry_finalize_v2$
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
       OR p_global_schema_version <> 7
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 8
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = p_global_schema_version
              AND c.min_reader = 7 AND c.max_reader = 7
              AND c.min_writer = 7 AND c.max_writer = 7
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
       AND c.persistence_receipt_digest = p_persistence_receipt_digest
       AND c.persistence_schema_version = 7
       AND c.persistence_manifest_sha256 = p_global_manifest_sha256;
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
$lattice_project_registry_finalize_v2$;

-- The exact schema-v5 prefix already owns task_ledger_autonomy_receipts.
-- This migration only rebinds its two functions to the seven-entry manifest.

CREATE OR REPLACE FUNCTION control.task_ledger_record_autonomy_receipt_v1(
    p_stream_id bytea, p_event_sequence text, p_event_digest bytea,
    p_receipt_schema_version text, p_intent_version text, p_task_kind text,
    p_risk_class text, p_execution_preapproved boolean,
    p_requires_new_authority boolean, p_irreversible_or_high_risk boolean,
    p_observed_task_state text, p_disposition text, p_decision_reason text,
    p_model text, p_verification text, p_authority_mode text,
    p_process_start_authority_digest bytea,
    p_ingress_profile_adapter_commitment bytea,
    p_store_authority_head_digest bytea,
    p_writer_lease_receipt_digest bytea, p_writer_lease_head_digest bytea,
    p_writer_fencing_token text, p_authority_digest bytea, p_receipt_digest bytea
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
AS $lattice_task_ledger_record_autonomy_receipt_v1$
DECLARE
    v_event control.task_ledger_events%ROWTYPE;
    v_existing control.task_ledger_autonomy_receipts%ROWTYPE;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
       OR p_stream_id IS NULL
       OR pg_catalog.octet_length(p_stream_id) <> 32
       OR p_stream_id = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_event_sequence IS DISTINCT FROM '2'
       OR p_event_digest IS NULL
       OR pg_catalog.octet_length(p_event_digest) <> 32
       OR p_event_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_authority_digest IS NULL
       OR pg_catalog.octet_length(p_authority_digest) <> 32
       OR p_authority_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_receipt_digest IS NULL
       OR pg_catalog.octet_length(p_receipt_digest) <> 32
       OR p_receipt_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAU01';
    END IF;
    SELECT * INTO v_event
      FROM ONLY control.task_ledger_events
     WHERE stream_id = p_stream_id AND sequence = p_event_sequence::numeric
     FOR SHARE;
    IF NOT FOUND
       OR v_event.event_kind IS DISTINCT FROM 'AUTONOMY_RECEIPT_RECORDED'
       OR v_event.action_id IS DISTINCT FROM 'RECORD_AUTONOMY_RECEIPT_V1'
       OR v_event.audit_outcome IS DISTINCT FROM 'RECORDED'
       OR v_event.reason_code IS DISTINCT FROM 'AUTONOMY_DECISION_RECORDED'
       OR v_event.diagnostic IS DISTINCT FROM 'null'::jsonb
       OR v_event.event_digest IS DISTINCT FROM p_event_digest
       OR v_event.subject_digest IS DISTINCT FROM p_receipt_digest
       OR NOT EXISTS (
            SELECT 1 FROM ONLY control.task_ledger_events created
             WHERE created.stream_id = p_stream_id AND created.sequence = 1
               AND created.event_kind = 'TASK_CREATED'
       ) THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01';
    END IF;
    SELECT * INTO v_existing
      FROM ONLY control.task_ledger_autonomy_receipts
     WHERE stream_id = p_stream_id AND event_sequence = p_event_sequence::numeric;
    IF FOUND THEN
        IF v_existing.event_sequence::text IS DISTINCT FROM p_event_sequence
           OR v_existing.event_digest IS DISTINCT FROM p_event_digest
           OR v_existing.receipt_schema_version IS DISTINCT FROM p_receipt_schema_version
           OR v_existing.intent_version IS DISTINCT FROM p_intent_version
           OR v_existing.task_kind IS DISTINCT FROM p_task_kind
           OR v_existing.risk_class IS DISTINCT FROM p_risk_class
           OR v_existing.execution_preapproved IS DISTINCT FROM p_execution_preapproved
           OR v_existing.requires_new_authority IS DISTINCT FROM p_requires_new_authority
           OR v_existing.irreversible_or_high_risk IS DISTINCT FROM p_irreversible_or_high_risk
           OR v_existing.observed_task_state IS DISTINCT FROM p_observed_task_state
           OR v_existing.disposition IS DISTINCT FROM p_disposition
           OR v_existing.decision_reason IS DISTINCT FROM p_decision_reason
           OR v_existing.model IS DISTINCT FROM p_model
           OR v_existing.verification IS DISTINCT FROM p_verification
           OR v_existing.authority_mode IS DISTINCT FROM p_authority_mode
           OR v_existing.process_start_authority_digest IS DISTINCT FROM p_process_start_authority_digest
           OR v_existing.ingress_profile_adapter_commitment IS DISTINCT FROM p_ingress_profile_adapter_commitment
           OR v_existing.store_authority_head_digest IS DISTINCT FROM p_store_authority_head_digest
           OR v_existing.writer_lease_receipt_digest IS DISTINCT FROM p_writer_lease_receipt_digest
           OR v_existing.writer_lease_head_digest IS DISTINCT FROM p_writer_lease_head_digest
           OR v_existing.writer_fencing_token::text IS DISTINCT FROM p_writer_fencing_token
           OR v_existing.receipt_digest IS DISTINCT FROM p_receipt_digest
           OR v_existing.authority_digest IS DISTINCT FROM p_authority_digest THEN
            RAISE EXCEPTION USING ERRCODE = 'LTX01';
        END IF;
        RETURN 'RETAINED';
    END IF;
    INSERT INTO control.task_ledger_autonomy_receipts (
        stream_id, event_sequence, event_digest, receipt_schema_version,
        intent_version, task_kind, risk_class, execution_preapproved,
        requires_new_authority, irreversible_or_high_risk, observed_task_state,
        disposition, decision_reason, model, verification, authority_mode,
        process_start_authority_digest, ingress_profile_adapter_commitment,
        store_authority_head_digest, policy_decision_receipt_digest,
        policy_owner_head_digest, approval_receipt_digest, approval_owner_head_digest,
        writer_lease_receipt_digest, writer_lease_head_digest, writer_fencing_token,
        authority_digest, receipt_digest
    ) VALUES (
        p_stream_id, p_event_sequence::numeric, p_event_digest,
        p_receipt_schema_version, p_intent_version, p_task_kind, p_risk_class,
        p_execution_preapproved, p_requires_new_authority,
        p_irreversible_or_high_risk, p_observed_task_state, p_disposition,
        p_decision_reason, p_model, p_verification, p_authority_mode,
        p_process_start_authority_digest, p_ingress_profile_adapter_commitment,
        p_store_authority_head_digest, NULL, NULL, NULL, NULL,
        p_writer_lease_receipt_digest, p_writer_lease_head_digest,
        p_writer_fencing_token::numeric, p_authority_digest, p_receipt_digest
    );
    RETURN 'RECORDED';
END
$lattice_task_ledger_record_autonomy_receipt_v1$;

CREATE OR REPLACE FUNCTION control.task_ledger_read_autonomy_receipts_v1(p_stream_id bytea)
RETURNS TABLE (
    stream_id bytea, event_sequence text, event_digest bytea,
    receipt_schema_version text, intent_version text, task_kind text,
    risk_class text, execution_preapproved boolean,
    requires_new_authority boolean, irreversible_or_high_risk boolean,
    observed_task_state text, disposition text, decision_reason text,
    model text, verification text, authority_mode text,
    process_start_authority_digest bytea,
    ingress_profile_adapter_commitment bytea,
    store_authority_head_digest bytea, writer_lease_receipt_digest bytea,
    writer_lease_head_digest bytea, writer_fencing_token text,
    authority_digest bytea, receipt_digest bytea
)
LANGUAGE plpgsql
STABLE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_task_ledger_read_autonomy_receipts_v1$
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
    SELECT r.stream_id, r.event_sequence::text, r.event_digest,
           r.receipt_schema_version::text, r.intent_version::text,
           r.task_kind::text, r.risk_class::text, r.execution_preapproved,
           r.requires_new_authority, r.irreversible_or_high_risk,
           r.observed_task_state::text, r.disposition::text,
           r.decision_reason::text, r.model::text, r.verification::text,
           r.authority_mode::text, r.process_start_authority_digest,
           r.ingress_profile_adapter_commitment, r.store_authority_head_digest,
           r.writer_lease_receipt_digest, r.writer_lease_head_digest,
           r.writer_fencing_token::text, r.authority_digest, r.receipt_digest
      FROM ONLY control.task_ledger_autonomy_receipts r
     WHERE r.stream_id = p_stream_id
     ORDER BY r.event_sequence;
END
$lattice_task_ledger_read_autonomy_receipts_v1$;

REVOKE ALL ON TABLE control.task_ledger_autonomy_receipts FROM PUBLIC;
REVOKE ALL ON TABLE control.task_ledger_autonomy_receipts FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.task_ledger_record_autonomy_receipt_v1(
    bytea,text,bytea,text,text,text,text,boolean,boolean,boolean,text,text,text,
    text,text,text,bytea,bytea,bytea,bytea,bytea,text,bytea,bytea
) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
REVOKE ALL ON FUNCTION control.task_ledger_read_autonomy_receipts_v1(bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.task_ledger_record_autonomy_receipt_v1(
    bytea,text,bytea,text,text,text,text,boolean,boolean,boolean,text,text,text,
    text,text,text,bytea,bytea,bytea,bytea,bytea,text,bytea,bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.task_ledger_read_autonomy_receipts_v1(bytea)
    TO lattice_runtime;
REVOKE ALL ON TABLE control.task_ledger_autonomy_receipts FROM lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
REVOKE EXECUTE ON FUNCTION control.store_prepare_v4(smallint, text, smallint, text, text, text, text, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.store_prepare_v5(smallint, text, smallint, text, text, text, text, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.store_prepare_v5(smallint, text, smallint, text, text, text, text, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.store_finalize_v4(smallint, text, smallint, text, text, text, text, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, uuid, bytea, smallint, text, bigint, bytea, bytea, bigint, bytea, bytea, text, bytea, bytea) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.store_finalize_v5(smallint, text, smallint, text, text, text, text, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, uuid, bytea, smallint, text, bigint, bytea, bytea, bigint, bytea, bytea, text, bytea, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.store_finalize_v5(smallint, text, smallint, text, text, text, text, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, uuid, bytea, smallint, text, bigint, bytea, bytea, bigint, bytea, bytea, text, bytea, bytea) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.store_current_head_v4(smallint, text, text, text, text, bytea) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.store_current_head_v5(smallint, text, text, text, text, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.store_current_head_v5(smallint, text, text, text, text, bytea) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.task_ledger_prepare_v2(smallint, text, bytea, text) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.task_ledger_prepare_v3(smallint, text, bytea, text) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.task_ledger_prepare_v3(smallint, text, bytea, text) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.task_ledger_read_head_v2(smallint, text, bytea, text, text) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.task_ledger_read_head_v3(smallint, text, bytea, text, text) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
REVOKE ALL ON FUNCTION control.task_ledger_read_head_v4(smallint, text, bytea, text, text) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.task_ledger_read_head_v4(smallint, text, bytea, text, text) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.task_ledger_read_events_v2(smallint, text, bytea) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.task_ledger_read_events_v3(smallint, text, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.task_ledger_read_events_v3(smallint, text, bytea) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.task_ledger_read_commands_v2(smallint, text, bytea) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.task_ledger_read_commands_v3(smallint, text, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.task_ledger_read_commands_v3(smallint, text, bytea) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.task_ledger_finalize_v2(smallint, text, bytea, text, text, text, text, bytea, text, text, bytea, text, bytea, bytea, text, text, text, text, text, text, text, text, text, bytea, bytea, text, bytea, text, bytea, text, bytea, bytea, text, text, text, text, text, text, text, bytea, jsonb, boolean, text, text, text, text, text, text, text, bytea, text, bytea, bytea, text, bytea, text, bytea, bytea, text, text, bytea, bytea, bytea, text, boolean, text, bytea, text, bytea, boolean, bytea, bytea) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.task_ledger_finalize_v3(smallint, text, bytea, text, text, text, text, bytea, text, text, bytea, text, bytea, bytea, text, text, text, text, text, text, text, text, text, bytea, bytea, text, bytea, text, bytea, text, bytea, bytea, text, text, text, text, text, text, text, bytea, jsonb, boolean, text, text, text, text, text, text, text, bytea, text, bytea, bytea, text, bytea, text, bytea, bytea, text, text, bytea, bytea, bytea, text, boolean, text, bytea, text, bytea, boolean, bytea, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.task_ledger_finalize_v3(smallint, text, bytea, text, text, text, text, bytea, text, text, bytea, text, bytea, bytea, text, text, text, text, text, text, text, text, text, bytea, bytea, text, bytea, text, bytea, text, bytea, bytea, text, text, text, text, text, text, text, bytea, jsonb, boolean, text, text, text, text, text, text, text, bytea, text, bytea, bytea, text, bytea, text, bytea, bytea, text, text, bytea, bytea, bytea, text, boolean, text, bytea, text, bytea, boolean, bytea, bytea) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.task_ledger_finalize_general_intake_v1(
    smallint,text,bytea,text,text,text,text,text,bytea,text,bytea,text,bytea,bytea,
    text,text,text,text,text,text,text,text,text,bytea,bytea,text,bytea,text,bytea,
    text,bytea,bytea,text,text,text,bytea,text,bytea,text,bytea,bytea,text,bytea,
    text,bytea,bytea,bytea,bytea,bytea,text,text,bytea,text,bytea
) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
       lattice_migrator_login, lattice_runtime_login, lattice_guardian_login,
       lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.task_ledger_finalize_general_intake_v1(
    smallint,text,bytea,text,text,text,text,text,bytea,text,bytea,text,bytea,bytea,
    text,text,text,text,text,text,text,text,text,bytea,bytea,text,bytea,text,bytea,
    text,bytea,bytea,text,text,text,bytea,text,bytea,text,bytea,bytea,text,bytea,
    text,bytea,bytea,bytea,bytea,bytea,text,text,bytea,text,bytea
) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.project_registry_prepare_v1(smallint, text, text, bytea, text, text, bigint, text, bigint, bytea, bytea, bytea) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_prepare_v2(smallint, text, text, bytea, text, text, bigint, text, bigint, bytea, bytea, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_prepare_v2(smallint, text, text, bytea, text, text, bigint, text, bigint, bytea, bytea, bytea) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.project_registry_read_state_v1(smallint, text) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_read_state_v2(smallint, text) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_read_state_v2(smallint, text) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.project_registry_read_observations_v1(smallint, text) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_read_observations_v2(smallint, text) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_read_observations_v2(smallint, text) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.project_registry_read_projects_v1(smallint, text) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_read_projects_v2(smallint, text) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_read_projects_v2(smallint, text) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.project_registry_read_commands_v1(smallint, text) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_read_commands_v2(smallint, text) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_read_commands_v2(smallint, text) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.project_registry_read_reservations_v1(smallint, text) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_read_reservations_v2(smallint, text) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_read_reservations_v2(smallint, text) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.project_registry_stage_command_v1(smallint, text, bigint, text, text, text, text, bytea, boolean, text, text, text, text, text, numeric, text, text, text, bytea, bytea, bytea, text, bytea, bytea, text, text, text, text, text, text, text, bytea, bytea, bytea, boolean, boolean, boolean, boolean, boolean, bytea, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, bytea, bytea, boolean, text, bytea, bytea, bytea, text, bytea) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_stage_command_v2(smallint, text, smallint, text, bigint, text, text, text, text, bytea, boolean, text, text, text, text, text, numeric, text, text, text, bytea, bytea, bytea, text, bytea, bytea, text, text, text, text, text, text, text, bytea, bytea, bytea, boolean, boolean, boolean, boolean, boolean, bytea, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, bytea, bytea, boolean, text, bytea, bytea, bytea, text, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_stage_command_v2(smallint, text, smallint, text, bigint, text, text, text, text, bytea, boolean, text, text, text, text, text, numeric, text, text, text, bytea, bytea, bytea, text, bytea, bytea, text, text, text, text, text, text, text, bytea, bytea, bytea, boolean, boolean, boolean, boolean, boolean, bytea, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, bytea, text, text, bigint, text, bigint, bytea, bytea, bytea, bytea, boolean, text, bytea, bytea, bytea, text, bytea) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.project_registry_stage_project_v1(smallint, text, text, text, bytea, bytea, boolean, boolean, boolean, boolean, boolean, smallint, text, text, text, text, numeric, text, text, bytea, bytea, bytea) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_stage_project_v2(smallint, text, text, text, bytea, bytea, boolean, boolean, boolean, boolean, boolean, smallint, text, text, text, text, numeric, text, text, bytea, bytea, bytea) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_stage_project_v2(smallint, text, text, text, bytea, bytea, boolean, boolean, boolean, boolean, boolean, smallint, text, text, text, text, numeric, text, text, bytea, bytea, bytea) TO lattice_runtime;
REVOKE EXECUTE ON FUNCTION control.project_registry_finalize_v1(smallint, text, text, bigint, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, bytea, bytea, bytea, boolean, boolean, bigint, bigint) FROM lattice_runtime;
REVOKE ALL ON FUNCTION control.project_registry_finalize_v2(smallint, text, text, bigint, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, bytea, bytea, bytea, boolean, boolean, bigint, bigint) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.project_registry_finalize_v2(smallint, text, text, bigint, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, text, bigint, bigint, bigint, bigint, bigint, bigint, bytea, bytea, bytea, bytea, boolean, boolean, bigint, bigint) TO lattice_runtime;
COMMENT ON SCHEMA control IS 'LATTICE_DEVOS_CONTROL_SCHEMA_V7';
COMMENT ON SCHEMA memory IS 'LATTICE_DEVOS_MEMORY_SCHEMA_V7';
COMMENT ON SCHEMA readmodel IS 'LATTICE_DEVOS_READMODEL_SCHEMA_V7';

CREATE OR REPLACE FUNCTION control.task_ledger_record_foreman_snapshot_v1(
    p_writer_project_id text, p_writer_project_snapshot_id text,
    p_writer_task_id text, p_writer_task_revision text,
    p_writer_task_spec_digest bytea, p_writer_attempt_id text,
    p_writer_lease_id text, p_writer_lease_holder_id text,
    p_writer_worktree_id text, p_writer_holder_process_id bigint,
    p_writer_holder_process_start_identity bytea,
    p_writer_daemon_instance_id text, p_writer_daemon_epoch bigint,
    p_writer_fencing_token bigint, p_writer_receipt_digest bytea,
    p_stream_id bytea, p_event_sequence text, p_event_digest bytea,
    p_command_id text, p_request_digest bytea,
    p_record_schema text, p_payload_schema text, p_payload_digest bytea,
    p_worker_id text, p_thread_id text, p_task_id text, p_branch_ref text,
    p_worktree_ref text, p_head_sha1 text, p_foreman_state text,
    p_blocker_ref text, p_heartbeat_digest_ref text, p_authority_digest_ref text,
    p_evidence_digest_ref text,
    p_generation text, p_epistemic_schema text,
    p_observed_fact_refs text[], p_hypothesis_refs text[], p_confidence text,
    p_unknown_refs text[], p_evidence_refs text[], p_counterevidence_refs text[],
    p_checked_at text, p_expires_at text, p_refresh_trigger text,
    p_decision_ref text, p_probe_ref text, p_falsifier_ref text
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
AS $lattice_task_ledger_record_foreman_snapshot_v1$
DECLARE
    v_event control.task_ledger_events%ROWTYPE;
    v_inserted bigint;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
       OR NOT writer_lease.writer_lease_assert_current_v1(
            p_writer_project_id, p_writer_project_snapshot_id, p_writer_task_id,
            p_writer_task_revision, p_writer_task_spec_digest, p_writer_attempt_id,
            p_writer_lease_id, p_writer_lease_holder_id, p_writer_worktree_id,
            p_writer_holder_process_id, p_writer_holder_process_start_identity,
            p_writer_daemon_instance_id, p_writer_daemon_epoch,
            p_writer_fencing_token, p_writer_receipt_digest)
       OR p_stream_id IS NULL OR pg_catalog.octet_length(p_stream_id) <> 32
       OR p_event_digest IS NULL OR pg_catalog.octet_length(p_event_digest) <> 32
       OR p_request_digest IS NULL OR pg_catalog.octet_length(p_request_digest) <> 32
       OR p_payload_digest IS NULL OR pg_catalog.octet_length(p_payload_digest) <> 32
       OR p_event_sequence !~ '^[1-9][0-9]{0,19}$'
       OR p_generation !~ '^[1-9][0-9]{0,19}$'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LFW01';
    END IF;

    SELECT * INTO v_event
      FROM ONLY control.task_ledger_events
     WHERE stream_id = p_stream_id
       AND sequence = p_event_sequence::numeric
       AND xmin = pg_catalog.pg_current_xact_id()::xid
     FOR SHARE;
    IF NOT FOUND
       OR v_event.event_digest IS DISTINCT FROM p_event_digest
       OR v_event.command_id IS DISTINCT FROM p_command_id
       OR v_event.request_digest IS DISTINCT FROM p_request_digest
       OR v_event.event_kind IS DISTINCT FROM 'FOREMAN_SNAPSHOT_RECORDED'
       OR v_event.action_id IS DISTINCT FROM 'RECORD_FOREMAN_SNAPSHOT_V1'
       OR v_event.audit_outcome IS DISTINCT FROM 'RECORDED'
       OR v_event.reason_code IS DISTINCT FROM 'FOREMAN_SNAPSHOT_RECORDED'
       OR v_event.subject_digest IS DISTINCT FROM p_payload_digest
       OR v_event.diagnostic IS DISTINCT FROM 'null'::jsonb
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01';
    END IF;

    INSERT INTO control.task_ledger_foreman_snapshots (
        stream_id,event_sequence,event_digest,command_id,request_digest,
        record_schema,payload_schema,payload_digest,worker_id,thread_id,task_id,
        branch_ref,worktree_ref,head_sha1,foreman_state,blocker_ref,
        heartbeat_digest_ref,authority_digest_ref,evidence_digest_ref,generation,epistemic_schema,
        observed_fact_refs,hypothesis_refs,confidence,unknown_refs,evidence_refs,
        counterevidence_refs,checked_at,expires_at,refresh_trigger,
        decision_ref,probe_ref,falsifier_ref
    ) VALUES (
        p_stream_id,p_event_sequence::numeric,p_event_digest,p_command_id,p_request_digest,
        p_record_schema,p_payload_schema,p_payload_digest,p_worker_id,p_thread_id,p_task_id,
        p_branch_ref,p_worktree_ref,p_head_sha1,p_foreman_state,p_blocker_ref,
        p_heartbeat_digest_ref,p_authority_digest_ref,p_evidence_digest_ref,
        p_generation::numeric,p_epistemic_schema,
        p_observed_fact_refs,p_hypothesis_refs,p_confidence,p_unknown_refs,p_evidence_refs,
        p_counterevidence_refs,p_checked_at,p_expires_at,p_refresh_trigger,
        p_decision_ref,p_probe_ref,p_falsifier_ref
    );
    GET DIAGNOSTICS v_inserted = ROW_COUNT;
    IF v_inserted <> 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'LTX01';
    END IF;
    RETURN 'RECORDED';
END
$lattice_task_ledger_record_foreman_snapshot_v1$;

CREATE OR REPLACE FUNCTION control.task_ledger_read_foreman_snapshots_v1(p_stream_id bytea)
RETURNS TABLE (
    stream_id bytea,event_sequence text,event_digest bytea,command_id text,
    request_digest bytea,record_schema text,payload_schema text,payload_digest bytea,
    worker_id text,thread_id text,task_id text,branch_ref text,worktree_ref text,
    head_sha1 text,foreman_state text,blocker_ref text,heartbeat_digest_ref text,
    authority_digest_ref text,evidence_digest_ref text,generation text,epistemic_schema text,
    observed_fact_refs text[],hypothesis_refs text[],confidence text,
    unknown_refs text[],evidence_refs text[],counterevidence_refs text[],
    checked_at text,expires_at text,refresh_trigger text,
    decision_ref text,probe_ref text,falsifier_ref text
)
LANGUAGE sql
STABLE
PARALLEL SAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_task_ledger_read_foreman_snapshots_v1$
    SELECT f.stream_id,f.event_sequence::text,f.event_digest,f.command_id::text,
           f.request_digest,f.record_schema::text,f.payload_schema::text,f.payload_digest,
           f.worker_id::text,f.thread_id::text,f.task_id::text,f.branch_ref::text,
           f.worktree_ref::text,f.head_sha1::text,f.foreman_state::text,f.blocker_ref::text,
           f.heartbeat_digest_ref::text,f.authority_digest_ref::text,
           f.evidence_digest_ref::text,f.generation::text,
           f.epistemic_schema::text,f.observed_fact_refs,f.hypothesis_refs,f.confidence::text,
           f.unknown_refs,f.evidence_refs,f.counterevidence_refs,f.checked_at::text,
           f.expires_at::text,f.refresh_trigger::text,f.decision_ref::text,
           f.probe_ref::text,f.falsifier_ref::text
      FROM control.task_ledger_foreman_snapshots AS f
      JOIN control.task_ledger_events AS e
        ON e.stream_id = f.stream_id AND e.sequence = f.event_sequence
       AND e.event_digest = f.event_digest AND e.command_id = f.command_id
       AND e.request_digest = f.request_digest
       AND e.event_kind = 'FOREMAN_SNAPSHOT_RECORDED'
       AND e.subject_digest = f.payload_digest
     WHERE f.stream_id = p_stream_id
       AND session_user = 'lattice_runtime_login'
       AND pg_catalog.current_setting('role') = 'lattice_runtime'
     ORDER BY f.event_sequence
$lattice_task_ledger_read_foreman_snapshots_v1$;

REVOKE ALL ON FUNCTION control.task_ledger_record_foreman_snapshot_v1(
    text,text,text,text,bytea,text,text,text,text,bigint,bytea,text,bigint,bigint,bytea,
    bytea,text,bytea,text,bytea,text,text,bytea,text,text,text,text,text,text,text,text,
    text,text,text,text,text,text[],text[],text,text[],text[],text[],text,text,text,text,text,text
) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control.task_ledger_record_foreman_snapshot_v1(
    text,text,text,text,bytea,text,text,text,text,bigint,bytea,text,bigint,bigint,bytea,
    bytea,text,bytea,text,bytea,text,text,bytea,text,text,text,text,text,text,text,text,
    text,text,text,text,text,text[],text[],text,text[],text[],text[],text,text,text,text,text,text
) TO lattice_runtime;
REVOKE ALL ON FUNCTION control.task_ledger_read_foreman_snapshots_v1(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control.task_ledger_read_foreman_snapshots_v1(bytea) TO lattice_runtime;

CREATE TABLE control.task_ingress_claims (
    schema_version varchar(64) NOT NULL,
    ingress_id varchar(64) NOT NULL,
    client_request_id varchar(64) NOT NULL,
    request_kind varchar(32) NOT NULL,
    ingress_request_digest bytea NOT NULL,
    stream_id bytea NOT NULL,
    event_sequence numeric(20,0) NOT NULL,
    event_digest bytea NOT NULL,
    command_id varchar(128) NOT NULL,
    command_request_digest bytea NOT NULL,
    PRIMARY KEY (ingress_id, client_request_id),
    UNIQUE (ingress_id, client_request_id, stream_id),
    UNIQUE (stream_id),
    FOREIGN KEY (stream_id) REFERENCES control.task_ledger_streams (stream_id),
    FOREIGN KEY (stream_id, event_sequence)
        REFERENCES control.task_ledger_events (stream_id, sequence),
    FOREIGN KEY (stream_id, command_id)
        REFERENCES control.task_ledger_commands (stream_id, command_id),
    CHECK (schema_version = 'lattice.task-ledger.task-ingress-claim/1.0'),
    CHECK (ingress_id ~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
        AND client_request_id ~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
        AND (client_request_id COLLATE pg_catalog."C") !~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
        AND NOT ((client_request_id COLLATE pg_catalog."C") ~* '-----begin '
            AND (client_request_id COLLATE pg_catalog."C") ~* 'private key-----')
        AND (pg_catalog.translate(client_request_id, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") !~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
        AND (client_request_id COLLATE pg_catalog."C") !~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'),
    CHECK (request_kind IN ('CONTROLLED_CODEX_CANARY', 'GENERAL_TASK')),
    CHECK (pg_catalog.octet_length(ingress_request_digest) = 32
        AND ingress_request_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(stream_id) = 32
        AND stream_id <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND event_sequence = 1
        AND pg_catalog.octet_length(event_digest) = 32
        AND event_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(command_request_digest) = 32
        AND command_request_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex'))
);

REVOKE ALL ON TABLE control.task_ingress_claims FROM PUBLIC;
REVOKE ALL ON TABLE control.task_ingress_claims FROM lattice_runtime;
REVOKE ALL ON TABLE control.task_ingress_claims FROM lattice_guardian;
REVOKE ALL ON TABLE control.task_ingress_claims FROM lattice_readonly;

-- A pre-v7 controlled-canary command key was not globally unique across Task
-- Ledger streams. Preserve every such durable identity when two or more
-- historical streams map to the same v7 ingress key. This relation is a
-- migration-owned deny/lineage index: it never selects a winning task and no
-- runtime role may read or write its physical rows directly.
CREATE TABLE control.task_ingress_historical_ambiguities (
    schema_version varchar(64) NOT NULL,
    ingress_id varchar(64) NOT NULL,
    client_request_id varchar(64) NOT NULL,
    request_kind varchar(32) NOT NULL,
    ingress_request_digest bytea NOT NULL,
    stream_id bytea NOT NULL,
    event_sequence numeric(20,0) NOT NULL,
    event_digest bytea NOT NULL,
    command_id varchar(128) NOT NULL,
    command_request_digest bytea NOT NULL,
    CONSTRAINT task_ingress_historical_ambiguities_pkey
        PRIMARY KEY (ingress_id, client_request_id, stream_id),
    CONSTRAINT task_ingress_historical_ambiguities_stream_key
        UNIQUE (stream_id),
    CONSTRAINT task_ingress_historical_ambiguities_stream_fk
        FOREIGN KEY (stream_id) REFERENCES control.task_ledger_streams (stream_id),
    CONSTRAINT task_ingress_historical_ambiguities_event_fk
        FOREIGN KEY (stream_id, event_sequence)
        REFERENCES control.task_ledger_events (stream_id, sequence),
    CONSTRAINT task_ingress_historical_ambiguities_command_fk
        FOREIGN KEY (stream_id, command_id)
        REFERENCES control.task_ledger_commands (stream_id, command_id),
    CONSTRAINT task_ingress_historical_ambiguities_schema_check
        CHECK (schema_version = 'lattice.task-ledger.task-ingress-historical-ambiguity/1.0'),
    CONSTRAINT task_ingress_historical_ambiguities_identity_check
        CHECK (ingress_id ~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
        AND client_request_id ~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
        AND (client_request_id COLLATE pg_catalog."C") !~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
        AND NOT ((client_request_id COLLATE pg_catalog."C") ~* '-----begin '
            AND (client_request_id COLLATE pg_catalog."C") ~* 'private key-----')
        AND (pg_catalog.translate(client_request_id, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") !~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
        AND (client_request_id COLLATE pg_catalog."C") !~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'),
    CONSTRAINT task_ingress_historical_ambiguities_kind_check
        CHECK (request_kind = 'CONTROLLED_CODEX_CANARY'),
    CONSTRAINT task_ingress_historical_ambiguities_digest_check
        CHECK (pg_catalog.octet_length(ingress_request_digest) = 32
        AND ingress_request_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(stream_id) = 32
        AND stream_id <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND event_sequence = 1
        AND pg_catalog.octet_length(event_digest) = 32
        AND event_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(command_request_digest) = 32
        AND command_request_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex'))
);

REVOKE ALL ON TABLE control.task_ingress_historical_ambiguities FROM PUBLIC;
REVOKE ALL ON TABLE control.task_ingress_historical_ambiguities FROM lattice_runtime;
REVOKE ALL ON TABLE control.task_ingress_historical_ambiguities FROM lattice_guardian;
REVOKE ALL ON TABLE control.task_ingress_historical_ambiguities FROM lattice_readonly;

-- Existing controlled-canary task identities predate the shared ingress
-- locator. Preserve them without rewriting Ledger history: their complete
-- canonical stream ID is the semantic request digest for the fixed canary
-- family, while the original command request digest remains a separate link.
-- A historical command whose otherwise-valid client key now matches the
-- shared secret predicate cannot be safely indexed or echoed. Abort the
-- upgrade with a static diagnostic rather than leaving a silently unclaimed
-- task identity or copying credential-shaped data into the new locator.
DO $lattice_task_ingress_historical_client_request_guard_v1$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM (
                SELECT pg_catalog.substring(e.command_id::text, 12) AS client_request_id
                  FROM control.task_ledger_events AS e
                  JOIN control.task_ledger_commands AS c
                    ON c.stream_id=e.stream_id AND c.command_id=e.command_id
                   AND c.request_digest=e.request_digest
                 WHERE e.sequence=1
                   AND e.event_kind='TASK_CREATED'
                   AND e.action_id IN ('CONTROLLED_CODEX_CANARY','CONTROLLED_CODEX_CANARY_AUTONOMY_V1')
                   AND e.command_id::text ~ '^mcp-submit:[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
               ) AS historical
         WHERE (historical.client_request_id COLLATE pg_catalog."C") ~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
            OR ((historical.client_request_id COLLATE pg_catalog."C") ~* '-----begin '
                AND (historical.client_request_id COLLATE pg_catalog."C") ~* 'private key-----')
            OR (pg_catalog.translate(historical.client_request_id, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") ~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
            OR (historical.client_request_id COLLATE pg_catalog."C") ~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
    ) THEN
        RAISE EXCEPTION USING
            ERRCODE = '23514',
            MESSAGE = 'LATTICE_TASK_INGRESS_HISTORICAL_CLIENT_REQUEST_ID_REJECTED';
    END IF;
END
$lattice_task_ingress_historical_client_request_guard_v1$;

-- A candidate-shaped TASK_CREATED event is durable ingress history only when
-- the Task Ledger recorded it. Reject contradictory historical audit state
-- instead of silently freeing the same global request key for reuse.
DO $lattice_task_ingress_historical_audit_guard_v1$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM control.task_ledger_events AS e
          JOIN control.task_ledger_commands AS c
            ON c.stream_id=e.stream_id AND c.command_id=e.command_id
         WHERE e.sequence=1
           AND e.event_kind='TASK_CREATED'
           AND e.action_id IN ('CONTROLLED_CODEX_CANARY','CONTROLLED_CODEX_CANARY_AUTONOMY_V1')
           AND e.command_id::text ~ '^mcp-submit:[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
           AND e.audit_outcome IS DISTINCT FROM 'RECORDED'
    ) THEN
        RAISE EXCEPTION USING
            ERRCODE = '23514',
            MESSAGE = 'LATTICE_TASK_INGRESS_HISTORICAL_AUDIT_OUTCOME_REJECTED';
    END IF;
END
$lattice_task_ingress_historical_audit_guard_v1$;

-- An appended mcp-submit command without its exact sequence-one event must not
-- disappear from the historical keyspace merely because the event-side FK is
-- one-way. Reject the incomplete durable pair before any backfill is written.
DO $lattice_task_ingress_historical_command_event_presence_guard_v1$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM control.task_ledger_commands AS c
          LEFT JOIN control.task_ledger_events AS e
            ON e.stream_id=c.stream_id AND e.command_id=c.command_id
         WHERE c.command_id::text ~ '^mcp-submit:[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
           AND c.command_outcome='APPENDED'
           AND (e.stream_id IS NULL
             OR e.sequence IS DISTINCT FROM 1
             OR e.event_kind IS DISTINCT FROM 'TASK_CREATED'
             OR e.action_id NOT IN ('CONTROLLED_CODEX_CANARY','CONTROLLED_CODEX_CANARY_AUTONOMY_V1')
             OR e.audit_outcome IS DISTINCT FROM 'RECORDED')
    ) THEN
        RAISE EXCEPTION USING
            ERRCODE = '23514',
            MESSAGE = 'LATTICE_TASK_INGRESS_HISTORICAL_COMMAND_BINDING_REJECTED';
    END IF;
END
$lattice_task_ingress_historical_command_event_presence_guard_v1$;

DO $lattice_task_ingress_historical_command_binding_guard_v1$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM control.task_ledger_events AS e
          JOIN control.task_ledger_commands AS c
            ON c.stream_id=e.stream_id AND c.command_id=e.command_id
         WHERE e.command_id::text ~ '^mcp-submit:[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
           AND (e.sequence IS DISTINCT FROM 1
             OR e.event_kind IS DISTINCT FROM 'TASK_CREATED'
             OR e.action_id NOT IN ('CONTROLLED_CODEX_CANARY','CONTROLLED_CODEX_CANARY_AUTONOMY_V1')
             OR e.audit_outcome IS DISTINCT FROM 'RECORDED'
             OR c.event_kind IS DISTINCT FROM 'TASK_CREATED'
             OR c.action_id NOT IN ('CONTROLLED_CODEX_CANARY','CONTROLLED_CODEX_CANARY_AUTONOMY_V1')
             OR ROW(
                   c.request_digest,c.correlation_id,c.occurred_at,c.event_kind,
                   c.actor_id,c.action_id,c.audit_outcome,c.reason_code,
                   c.subject_digest,c.diagnostic,c.has_resource_snapshot,
                   c.resource_active_agents,c.resource_active_implementers,
                   c.resource_elapsed_seconds,c.resource_attempt_number,
                   c.resource_used_model_calls,c.resource_used_external_cost,
                   c.event_digest
               ) IS DISTINCT FROM ROW(
                   e.request_digest,e.correlation_id,e.occurred_at,e.event_kind,
                   e.actor_id,e.action_id,e.audit_outcome,e.reason_code,
                   e.subject_digest,e.diagnostic,e.has_resource_snapshot,
                   e.resource_active_agents,e.resource_active_implementers,
                   e.resource_elapsed_seconds,e.resource_attempt_number,
                   e.resource_used_model_calls,e.resource_used_external_cost,
                   e.event_digest
               )
             OR c.command_outcome IS DISTINCT FROM 'APPENDED'
             OR c.denial_reason IS DISTINCT FROM ''
             OR c.expected_sequence IS DISTINCT FROM 0
             OR c.before_sequence IS DISTINCT FROM 0
             OR c.after_sequence IS DISTINCT FROM e.sequence
             OR e.previous_event_digest IS DISTINCT FROM pg_catalog.decode(pg_catalog.repeat('00',32),'hex')
             OR c.expected_last_event_digest IS DISTINCT FROM e.previous_event_digest
             OR c.before_last_event_digest IS DISTINCT FROM e.previous_event_digest
             OR c.after_last_event_digest IS DISTINCT FROM e.event_digest
             OR c.expected_resource_revision IS DISTINCT FROM c.before_resource_revision
             OR c.expected_resource_projection_digest IS DISTINCT FROM c.before_resource_projection_digest
             OR c.expected_head_digest IS DISTINCT FROM c.before_head_digest
             OR c.after_resource_revision IS DISTINCT FROM e.resource_revision
             OR c.after_resource_projection_digest IS DISTINCT FROM e.resource_projection_digest)
    ) THEN
        RAISE EXCEPTION USING
            ERRCODE = '23514',
            MESSAGE = 'LATTICE_TASK_INGRESS_HISTORICAL_COMMAND_BINDING_REJECTED';
    END IF;
END
$lattice_task_ingress_historical_command_binding_guard_v1$;

WITH historical AS (
    SELECT 'lattice.task-ledger.task-ingress-claim/1.0'::varchar(64) AS schema_version,
           'lattice_task_submit.v1'::varchar(64) AS ingress_id,
           pg_catalog.substring(e.command_id::text, 12)::varchar(64) AS client_request_id,
           'CONTROLLED_CODEX_CANARY'::varchar(32) AS request_kind,
           e.stream_id AS ingress_request_digest,e.stream_id,e.sequence AS event_sequence,
           e.event_digest,e.command_id,e.request_digest AS command_request_digest
      FROM control.task_ledger_events AS e
      JOIN control.task_ledger_commands AS c
        ON c.stream_id=e.stream_id AND c.command_id=e.command_id
       AND ROW(
               c.request_digest,c.correlation_id,c.occurred_at,c.event_kind,
               c.actor_id,c.action_id,c.audit_outcome,c.reason_code,
               c.subject_digest,c.diagnostic,c.has_resource_snapshot,
               c.resource_active_agents,c.resource_active_implementers,
               c.resource_elapsed_seconds,c.resource_attempt_number,
               c.resource_used_model_calls,c.resource_used_external_cost,
               c.event_digest
           ) IS NOT DISTINCT FROM ROW(
               e.request_digest,e.correlation_id,e.occurred_at,e.event_kind,
               e.actor_id,e.action_id,e.audit_outcome,e.reason_code,
               e.subject_digest,e.diagnostic,e.has_resource_snapshot,
               e.resource_active_agents,e.resource_active_implementers,
               e.resource_elapsed_seconds,e.resource_attempt_number,
               e.resource_used_model_calls,e.resource_used_external_cost,
               e.event_digest
           )
       AND c.command_outcome='APPENDED'
       AND c.denial_reason=''
       AND c.expected_sequence=0
       AND c.before_sequence=0
       AND c.after_sequence=e.sequence
       AND e.previous_event_digest=pg_catalog.decode(pg_catalog.repeat('00',32),'hex')
       AND c.expected_last_event_digest=e.previous_event_digest
       AND c.before_last_event_digest=e.previous_event_digest
       AND c.after_last_event_digest=e.event_digest
       AND c.expected_resource_revision=c.before_resource_revision
       AND c.expected_resource_projection_digest=c.before_resource_projection_digest
       AND c.expected_head_digest=c.before_head_digest
       AND c.after_resource_revision=e.resource_revision
       AND c.after_resource_projection_digest=e.resource_projection_digest
      WHERE e.sequence=1
       AND e.event_kind='TASK_CREATED'
       AND e.action_id IN ('CONTROLLED_CODEX_CANARY','CONTROLLED_CODEX_CANARY_AUTONOMY_V1')
       AND e.audit_outcome='RECORDED'
       AND e.command_id::text ~ '^mcp-submit:[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       AND (pg_catalog.substring(e.command_id::text, 12) COLLATE pg_catalog."C") !~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
       AND NOT ((pg_catalog.substring(e.command_id::text, 12) COLLATE pg_catalog."C") ~* '-----begin '
           AND (pg_catalog.substring(e.command_id::text, 12) COLLATE pg_catalog."C") ~* 'private key-----')
       AND (pg_catalog.translate(pg_catalog.substring(e.command_id::text, 12), U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") !~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
       AND (pg_catalog.substring(e.command_id::text, 12) COLLATE pg_catalog."C") !~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
), classified AS (
    SELECT historical.*,
           count(*) OVER (
            PARTITION BY historical.ingress_id, historical.client_request_id
        ) AS historical_identity_count
      FROM historical
)
INSERT INTO control.task_ingress_claims (
    schema_version,ingress_id,client_request_id,request_kind,
    ingress_request_digest,stream_id,event_sequence,event_digest,command_id,
    command_request_digest
)
SELECT classified.schema_version,classified.ingress_id,classified.client_request_id,
       classified.request_kind,classified.ingress_request_digest,classified.stream_id,
       classified.event_sequence,classified.event_digest,classified.command_id,
       classified.command_request_digest
  FROM classified
 WHERE classified.historical_identity_count = 1;

WITH historical AS (
    SELECT 'lattice.task-ledger.task-ingress-historical-ambiguity/1.0'::varchar(64) AS schema_version,
           'lattice_task_submit.v1'::varchar(64) AS ingress_id,
           pg_catalog.substring(e.command_id::text, 12)::varchar(64) AS client_request_id,
           'CONTROLLED_CODEX_CANARY'::varchar(32) AS request_kind,
           e.stream_id AS ingress_request_digest,e.stream_id,e.sequence AS event_sequence,
           e.event_digest,e.command_id,e.request_digest AS command_request_digest
      FROM control.task_ledger_events AS e
      JOIN control.task_ledger_commands AS c
        ON c.stream_id=e.stream_id AND c.command_id=e.command_id
       AND ROW(
               c.request_digest,c.correlation_id,c.occurred_at,c.event_kind,
               c.actor_id,c.action_id,c.audit_outcome,c.reason_code,
               c.subject_digest,c.diagnostic,c.has_resource_snapshot,
               c.resource_active_agents,c.resource_active_implementers,
               c.resource_elapsed_seconds,c.resource_attempt_number,
               c.resource_used_model_calls,c.resource_used_external_cost,
               c.event_digest
           ) IS NOT DISTINCT FROM ROW(
               e.request_digest,e.correlation_id,e.occurred_at,e.event_kind,
               e.actor_id,e.action_id,e.audit_outcome,e.reason_code,
               e.subject_digest,e.diagnostic,e.has_resource_snapshot,
               e.resource_active_agents,e.resource_active_implementers,
               e.resource_elapsed_seconds,e.resource_attempt_number,
               e.resource_used_model_calls,e.resource_used_external_cost,
               e.event_digest
           )
       AND c.command_outcome='APPENDED'
       AND c.denial_reason=''
       AND c.expected_sequence=0
       AND c.before_sequence=0
       AND c.after_sequence=e.sequence
       AND e.previous_event_digest=pg_catalog.decode(pg_catalog.repeat('00',32),'hex')
       AND c.expected_last_event_digest=e.previous_event_digest
       AND c.before_last_event_digest=e.previous_event_digest
       AND c.after_last_event_digest=e.event_digest
       AND c.expected_resource_revision=c.before_resource_revision
       AND c.expected_resource_projection_digest=c.before_resource_projection_digest
       AND c.expected_head_digest=c.before_head_digest
       AND c.after_resource_revision=e.resource_revision
       AND c.after_resource_projection_digest=e.resource_projection_digest
     WHERE e.sequence=1
       AND e.event_kind='TASK_CREATED'
       AND e.action_id IN ('CONTROLLED_CODEX_CANARY','CONTROLLED_CODEX_CANARY_AUTONOMY_V1')
       AND e.audit_outcome='RECORDED'
       AND e.command_id::text ~ '^mcp-submit:[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       AND (pg_catalog.substring(e.command_id::text, 12) COLLATE pg_catalog."C") !~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
       AND NOT ((pg_catalog.substring(e.command_id::text, 12) COLLATE pg_catalog."C") ~* '-----begin '
           AND (pg_catalog.substring(e.command_id::text, 12) COLLATE pg_catalog."C") ~* 'private key-----')
       AND (pg_catalog.translate(pg_catalog.substring(e.command_id::text, 12), U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") !~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
       AND (pg_catalog.substring(e.command_id::text, 12) COLLATE pg_catalog."C") !~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
), classified AS (
    SELECT historical.*,
           count(*) OVER (
            PARTITION BY historical.ingress_id, historical.client_request_id
        ) AS historical_identity_count
      FROM historical
)
INSERT INTO control.task_ingress_historical_ambiguities (
    schema_version,ingress_id,client_request_id,request_kind,
    ingress_request_digest,stream_id,event_sequence,event_digest,command_id,
    command_request_digest
)
SELECT classified.schema_version,classified.ingress_id,classified.client_request_id,
       classified.request_kind,classified.ingress_request_digest,classified.stream_id,
       classified.event_sequence,classified.event_digest,classified.command_id,
       classified.command_request_digest
  FROM classified
 WHERE classified.historical_identity_count > 1;

CREATE FUNCTION control.task_ingress_prepare_v1(
    p_ingress_id text,
    p_client_request_id text,
    p_request_kind text,
    p_ingress_request_digest bytea,
    p_stream_id bytea
)
RETURNS TABLE (
    found boolean,
    schema_version text,
    ingress_id text,
    client_request_id text,
    request_kind text,
    ingress_request_digest bytea,
    stream_id bytea,
    event_sequence text,
    event_digest bytea,
    command_id text,
    command_request_digest bytea,
    event_kind text,
    event_action text,
    event_audit_outcome text
)
LANGUAGE plpgsql
VOLATILE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_task_ingress_prepare_v1$
DECLARE
    v_existing control.task_ingress_claims%ROWTYPE;
    v_event control.task_ledger_events%ROWTYPE;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
       OR p_ingress_id IS NULL
       OR p_ingress_id !~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       OR p_client_request_id IS NULL
       OR p_client_request_id !~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       OR (p_client_request_id COLLATE pg_catalog."C") ~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
       OR ((p_client_request_id COLLATE pg_catalog."C") ~* '-----begin '
           AND (p_client_request_id COLLATE pg_catalog."C") ~* 'private key-----')
       OR (pg_catalog.translate(p_client_request_id, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") ~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
       OR (p_client_request_id COLLATE pg_catalog."C") ~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
       OR p_request_kind NOT IN ('CONTROLLED_CODEX_CANARY','GENERAL_TASK')
       OR p_ingress_request_digest IS NULL
       OR pg_catalog.octet_length(p_ingress_request_digest) <> 32
       OR p_ingress_request_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_stream_id IS NULL
       OR pg_catalog.octet_length(p_stream_id) <> 32
       OR p_stream_id = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01';
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(p_ingress_id || ':' || p_client_request_id, 0)
    );
    IF EXISTS (
        SELECT 1
          FROM ONLY control.task_ingress_historical_ambiguities AS a
         WHERE a.ingress_id=p_ingress_id
           AND a.client_request_id=p_client_request_id
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'LTX01',
            MESSAGE = 'LATTICE_TASK_INGRESS_HISTORICAL_AMBIGUOUS';
    END IF;
    SELECT * INTO v_existing
      FROM ONLY control.task_ingress_claims AS c
     WHERE c.ingress_id=p_ingress_id
       AND c.client_request_id=p_client_request_id
     FOR SHARE;
    IF FOUND AND (
        v_existing.request_kind IS DISTINCT FROM p_request_kind
        OR v_existing.ingress_request_digest IS DISTINCT FROM p_ingress_request_digest
        OR v_existing.stream_id IS DISTINCT FROM p_stream_id
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'LTX01';
    END IF;
    IF NOT FOUND THEN
        RETURN QUERY SELECT false,NULL::text,NULL::text,NULL::text,NULL::text,
            NULL::bytea,NULL::bytea,NULL::text,NULL::bytea,NULL::text,NULL::bytea,
            NULL::text,NULL::text,NULL::text;
        RETURN;
    END IF;
    SELECT * INTO v_event
      FROM ONLY control.task_ledger_events AS e
     WHERE e.stream_id=v_existing.stream_id
       AND e.sequence=v_existing.event_sequence
       AND e.event_digest=v_existing.event_digest
       AND e.command_id=v_existing.command_id
       AND e.request_digest=v_existing.command_request_digest
     FOR SHARE;
    RETURN QUERY SELECT true,v_existing.schema_version::text,
        v_existing.ingress_id::text,v_existing.client_request_id::text,
        v_existing.request_kind::text,v_existing.ingress_request_digest,
        v_existing.stream_id,v_existing.event_sequence::text,
        v_existing.event_digest,v_existing.command_id::text,
        v_existing.command_request_digest,v_event.event_kind::text,
        v_event.action_id::text,v_event.audit_outcome::text;
END
$lattice_task_ingress_prepare_v1$;

CREATE FUNCTION control.task_ingress_record_v1(
    p_schema_version text,
    p_ingress_id text,
    p_client_request_id text,
    p_request_kind text,
    p_ingress_request_digest bytea,
    p_stream_id bytea,
    p_event_sequence text,
    p_event_digest bytea,
    p_command_id text,
    p_command_request_digest bytea
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
AS $lattice_task_ingress_record_v1$
DECLARE
    v_event control.task_ledger_events%ROWTYPE;
    v_existing control.task_ingress_claims%ROWTYPE;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
       OR p_schema_version <> 'lattice.task-ledger.task-ingress-claim/1.0'
       OR p_schema_version IS NULL
       OR p_ingress_id IS NULL
       OR p_ingress_id !~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       OR p_client_request_id IS NULL
       OR p_client_request_id !~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       OR (p_client_request_id COLLATE pg_catalog."C") ~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
       OR ((p_client_request_id COLLATE pg_catalog."C") ~* '-----begin '
           AND (p_client_request_id COLLATE pg_catalog."C") ~* 'private key-----')
       OR (pg_catalog.translate(p_client_request_id, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") ~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
       OR (p_client_request_id COLLATE pg_catalog."C") ~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
       OR p_request_kind IS NULL
       OR p_request_kind NOT IN ('CONTROLLED_CODEX_CANARY','GENERAL_TASK')
       OR p_ingress_request_digest IS NULL
       OR pg_catalog.octet_length(p_ingress_request_digest) <> 32
       OR p_ingress_request_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_stream_id IS NULL
       OR pg_catalog.octet_length(p_stream_id) <> 32
       OR p_stream_id = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_event_sequence IS NULL
       OR p_event_sequence <> '1'
       OR p_event_digest IS NULL
       OR pg_catalog.octet_length(p_event_digest) <> 32
       OR p_event_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_command_id IS NULL
       OR p_command_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'
       OR p_command_id IS DISTINCT FROM 'mcp-submit:' || p_client_request_id
       OR p_command_request_digest IS NULL
       OR pg_catalog.octet_length(p_command_request_digest) <> 32
       OR p_command_request_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01';
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(p_ingress_id || ':' || p_client_request_id, 0)
    );
    IF EXISTS (
        SELECT 1
          FROM ONLY control.task_ingress_historical_ambiguities AS a
         WHERE a.ingress_id=p_ingress_id
           AND a.client_request_id=p_client_request_id
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'LTX01',
            MESSAGE = 'LATTICE_TASK_INGRESS_HISTORICAL_AMBIGUOUS';
    END IF;
    SELECT * INTO v_existing
      FROM ONLY control.task_ingress_claims AS c
     WHERE c.ingress_id=p_ingress_id
       AND c.client_request_id=p_client_request_id
     FOR SHARE;
    IF FOUND THEN
        IF v_existing.schema_version IS DISTINCT FROM p_schema_version
           OR v_existing.request_kind IS DISTINCT FROM p_request_kind
           OR v_existing.ingress_request_digest IS DISTINCT FROM p_ingress_request_digest
           OR v_existing.stream_id IS DISTINCT FROM p_stream_id
           OR v_existing.event_sequence::text IS DISTINCT FROM p_event_sequence
           OR v_existing.event_digest IS DISTINCT FROM p_event_digest
           OR v_existing.command_id IS DISTINCT FROM p_command_id
           OR v_existing.command_request_digest IS DISTINCT FROM p_command_request_digest
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LTX01';
        END IF;
        RETURN 'REPLAYED';
    END IF;

    SELECT * INTO v_event
      FROM ONLY control.task_ledger_events AS e
     WHERE e.stream_id=p_stream_id
       AND e.sequence=p_event_sequence::numeric
       AND e.xmin=pg_catalog.pg_current_xact_id()::xid
     FOR SHARE;
    IF NOT FOUND
       OR v_event.event_digest IS DISTINCT FROM p_event_digest
       OR v_event.command_id IS DISTINCT FROM p_command_id
       OR v_event.request_digest IS DISTINCT FROM p_command_request_digest
       OR v_event.event_kind IS DISTINCT FROM 'TASK_CREATED'
       OR v_event.audit_outcome IS DISTINCT FROM 'RECORDED'
       OR (p_request_kind='CONTROLLED_CODEX_CANARY'
           AND v_event.action_id IS DISTINCT FROM 'CONTROLLED_CODEX_CANARY_AUTONOMY_V1')
        OR (p_request_kind='GENERAL_TASK'
            AND v_event.action_id IS DISTINCT FROM 'GENERAL_TASK_INTAKE_V1')
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01';
    END IF;

    INSERT INTO control.task_ingress_claims (
        schema_version,ingress_id,client_request_id,request_kind,
        ingress_request_digest,stream_id,event_sequence,event_digest,command_id,
        command_request_digest
    ) VALUES (
        p_schema_version,p_ingress_id,p_client_request_id,p_request_kind,
        p_ingress_request_digest,p_stream_id,p_event_sequence::numeric,
        p_event_digest,p_command_id,p_command_request_digest
    );
    RETURN 'RECORDED';
END
$lattice_task_ingress_record_v1$;

CREATE FUNCTION control.task_ingress_read_by_request_v1(
    p_ingress_id text,
    p_client_request_id text
)
RETURNS TABLE (
    schema_version text,ingress_id text,client_request_id text,request_kind text,
    ingress_request_digest bytea,stream_id bytea,event_sequence text,
    event_digest bytea,command_id text,command_request_digest bytea,
    event_kind text,event_action text,event_audit_outcome text
)
LANGUAGE plpgsql
STABLE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_task_ingress_read_by_request_v1$
BEGIN
    IF session_user='lattice_runtime_login'
       AND pg_catalog.current_setting('role')='lattice_runtime'
       AND EXISTS (
           SELECT 1
             FROM ONLY control.task_ingress_historical_ambiguities AS a
            WHERE a.ingress_id=p_ingress_id
              AND a.client_request_id=p_client_request_id
       )
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LTX01',
            MESSAGE = 'LATTICE_TASK_INGRESS_HISTORICAL_AMBIGUOUS';
    END IF;
    RETURN QUERY
    SELECT c.schema_version::text,c.ingress_id::text,c.client_request_id::text,
           c.request_kind::text,c.ingress_request_digest,c.stream_id,
           c.event_sequence::text,c.event_digest,c.command_id::text,
           c.command_request_digest,e.event_kind::text,e.action_id::text,
           e.audit_outcome::text
      FROM ONLY control.task_ingress_claims AS c
      LEFT JOIN ONLY control.task_ledger_events AS e
        ON e.stream_id=c.stream_id AND e.sequence=c.event_sequence
       AND e.event_digest=c.event_digest AND e.command_id=c.command_id
       AND e.request_digest=c.command_request_digest
     WHERE c.ingress_id=p_ingress_id
       AND c.client_request_id=p_client_request_id
       AND p_ingress_id ~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       AND p_client_request_id ~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       AND (p_client_request_id COLLATE pg_catalog."C") !~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
       AND NOT ((p_client_request_id COLLATE pg_catalog."C") ~* '-----begin '
                AND (p_client_request_id COLLATE pg_catalog."C") ~* 'private key-----')
       AND (pg_catalog.translate(p_client_request_id, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") !~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
       AND (p_client_request_id COLLATE pg_catalog."C") !~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
       AND session_user='lattice_runtime_login'
       AND pg_catalog.current_setting('role')='lattice_runtime';
END
$lattice_task_ingress_read_by_request_v1$;

CREATE TABLE control.task_submission_envelopes (
    schema_version varchar(64) NOT NULL,
    ingress_id varchar(64) NOT NULL,
    client_request_id varchar(64) NOT NULL,
    objective varchar(2048) NOT NULL,
    project_display_name varchar(256) NOT NULL,
    project_authority_receipt_digest bytea NOT NULL,
    project_id varchar(64) NOT NULL,
    project_snapshot_id varchar(159) NOT NULL,
    task_id varchar(128) NOT NULL,
    task_revision numeric(20,0) NOT NULL,
    task_subject_kind varchar(32) NOT NULL,
    intake_digest bytea NOT NULL,
    stream_id bytea NOT NULL,
    task_ref char(64) NOT NULL,
    admission_action varchar(64) NOT NULL,
    envelope_digest bytea NOT NULL,
    event_sequence numeric(20,0) NOT NULL,
    event_digest bytea NOT NULL,
    command_id varchar(128) NOT NULL,
    request_digest bytea NOT NULL,
    ingress_request_digest bytea NOT NULL,
    PRIMARY KEY (ingress_id, client_request_id),
    UNIQUE (task_ref),
    UNIQUE (stream_id),
    FOREIGN KEY (stream_id) REFERENCES control.task_ledger_streams (stream_id),
    FOREIGN KEY (stream_id, event_sequence)
        REFERENCES control.task_ledger_events (stream_id, sequence),
    FOREIGN KEY (stream_id, command_id)
        REFERENCES control.task_ledger_commands (stream_id, command_id),
    FOREIGN KEY (ingress_id, client_request_id, stream_id)
        REFERENCES control.task_ingress_claims (ingress_id, client_request_id, stream_id),
    CHECK (schema_version = 'lattice.task-ledger.task-submission/1.0'
        AND admission_action = 'GENERAL_TASK_INTAKE_V1'),
    CHECK (ingress_id ~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
        AND client_request_id ~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
        AND (client_request_id COLLATE pg_catalog."C") !~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
        AND NOT ((client_request_id COLLATE pg_catalog."C") ~* '-----begin '
                 AND (client_request_id COLLATE pg_catalog."C") ~* 'private key-----')
        AND (pg_catalog.translate(client_request_id, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") !~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
        AND (client_request_id COLLATE pg_catalog."C") !~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'),
    CHECK (objective <> '' AND objective = pg_catalog.btrim(objective)
        AND objective IS NFC NORMALIZED
        AND pg_catalog.translate(objective, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) = pg_catalog.btrim(pg_catalog.translate(objective, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)))
        AND pg_catalog.char_length(objective) <= 512
        AND pg_catalog.octet_length(objective) <= 2048
        AND (objective COLLATE pg_catalog."C") !~ U&'[\0001-\001F\007F-\009F]'
        AND project_display_name <> ''
        AND project_display_name = pg_catalog.btrim(project_display_name)
        AND project_display_name IS NFC NORMALIZED
        AND pg_catalog.translate(project_display_name, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) = pg_catalog.btrim(pg_catalog.translate(project_display_name, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)))
        AND pg_catalog.char_length(project_display_name) <= 64
        AND pg_catalog.octet_length(project_display_name) <= 256
        AND (project_display_name COLLATE pg_catalog."C") !~ U&'[\0001-\001F\007F-\009F]'),
    CHECK ((objective COLLATE pg_catalog."C") !~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
        AND NOT ((objective COLLATE pg_catalog."C") ~* '-----begin '
                 AND (objective COLLATE pg_catalog."C") ~* 'private key-----')
        AND (pg_catalog.translate(objective, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") !~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
        AND (objective COLLATE pg_catalog."C") !~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
        AND (project_display_name COLLATE pg_catalog."C") !~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
        AND NOT ((project_display_name COLLATE pg_catalog."C") ~* '-----begin '
                 AND (project_display_name COLLATE pg_catalog."C") ~* 'private key-----')
        AND (pg_catalog.translate(project_display_name, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") !~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
        AND (project_display_name COLLATE pg_catalog."C") !~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'),
    CHECK (project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'
        AND project_snapshot_id ~ '^[a-z0-9._:-]{1,159}$'
        AND (project_id COLLATE pg_catalog."C") !~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
        AND NOT ((project_id COLLATE pg_catalog."C") ~* '-----begin '
                 AND (project_id COLLATE pg_catalog."C") ~* 'private key-----')
        AND (pg_catalog.translate(project_id, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") !~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
        AND (project_id COLLATE pg_catalog."C") !~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
        AND (project_snapshot_id COLLATE pg_catalog."C") !~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
        AND NOT ((project_snapshot_id COLLATE pg_catalog."C") ~* '-----begin '
                 AND (project_snapshot_id COLLATE pg_catalog."C") ~* 'private key-----')
        AND (pg_catalog.translate(project_snapshot_id, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") !~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
        AND (project_snapshot_id COLLATE pg_catalog."C") !~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
        AND task_id ~ '^TASK-[A-Z0-9][A-Z0-9_-]{2,63}$'
        AND task_revision >= 1 AND task_revision <= 18446744073709551615
        AND task_subject_kind = 'GENERAL_TASK_INTAKE'),
    CHECK (task_ref ~ '^[0-9a-f]{64}$'
        AND pg_catalog.octet_length(project_authority_receipt_digest) = 32
        AND project_authority_receipt_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(intake_digest) = 32
        AND intake_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(stream_id) = 32
        AND stream_id <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(envelope_digest) = 32
        AND envelope_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND event_sequence = 1
        AND pg_catalog.octet_length(event_digest) = 32
        AND event_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(request_digest) = 32
        AND request_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(ingress_request_digest) = 32
        AND ingress_request_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex'))
);

REVOKE ALL ON TABLE control.task_submission_envelopes FROM PUBLIC;
REVOKE ALL ON TABLE control.task_submission_envelopes FROM lattice_runtime;
REVOKE ALL ON TABLE control.task_submission_envelopes FROM lattice_guardian;
REVOKE ALL ON TABLE control.task_submission_envelopes FROM lattice_readonly;

-- Runtime, Guardian, and ReadOnly cannot inspect migration-owned ambiguity
-- rows directly. Expose only one fixed boolean closure so every fresh role can
-- reject lineage drift without learning historical task identities.
CREATE FUNCTION control.task_ingress_historical_closure_v1()
RETURNS boolean
LANGUAGE sql
STABLE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_task_ingress_historical_closure_v1$
WITH candidate_audit_mismatch AS (
    SELECT 1
      FROM ONLY control.task_ledger_events AS e
      JOIN ONLY control.task_ledger_commands AS c
        ON c.stream_id=e.stream_id AND c.command_id=e.command_id
     WHERE e.sequence=1
       AND e.event_kind='TASK_CREATED'
       AND e.action_id IN ('CONTROLLED_CODEX_CANARY','CONTROLLED_CODEX_CANARY_AUTONOMY_V1')
       AND e.command_id::text ~ '^mcp-submit:[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       AND e.audit_outcome IS DISTINCT FROM 'RECORDED'
), candidate_event_presence_mismatch AS (
    SELECT 1
      FROM ONLY control.task_ledger_commands AS c
      LEFT JOIN ONLY control.task_ledger_events AS e
        ON e.stream_id=c.stream_id AND e.command_id=c.command_id
     WHERE c.command_outcome='APPENDED'
       AND c.action_id IN ('CONTROLLED_CODEX_CANARY','CONTROLLED_CODEX_CANARY_AUTONOMY_V1')
       AND c.command_id::text ~ '^mcp-submit:[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       AND (e.stream_id IS NULL
         OR e.sequence IS DISTINCT FROM 1
         OR e.event_kind IS DISTINCT FROM 'TASK_CREATED'
         OR e.action_id NOT IN ('CONTROLLED_CODEX_CANARY','CONTROLLED_CODEX_CANARY_AUTONOMY_V1')
         OR e.audit_outcome IS DISTINCT FROM 'RECORDED')
), candidate_binding_mismatch AS (
    SELECT 1
      FROM ONLY control.task_ledger_events AS e
      JOIN ONLY control.task_ledger_commands AS c
        ON c.stream_id=e.stream_id AND c.command_id=e.command_id
     WHERE e.sequence=1
       AND e.command_id::text ~ '^mcp-submit:[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       AND (
           (e.event_kind='TASK_CREATED'
            AND e.action_id IN ('CONTROLLED_CODEX_CANARY','CONTROLLED_CODEX_CANARY_AUTONOMY_V1'))
           OR
           (c.event_kind='TASK_CREATED'
            AND c.action_id IN ('CONTROLLED_CODEX_CANARY','CONTROLLED_CODEX_CANARY_AUTONOMY_V1'))
       )
       AND (ROW(
               c.request_digest,c.correlation_id,c.occurred_at,c.event_kind,
               c.actor_id,c.action_id,c.audit_outcome,c.reason_code,
               c.subject_digest,c.diagnostic,c.has_resource_snapshot,
               c.resource_active_agents,c.resource_active_implementers,
               c.resource_elapsed_seconds,c.resource_attempt_number,
               c.resource_used_model_calls,c.resource_used_external_cost,
               c.event_digest
           ) IS DISTINCT FROM ROW(
               e.request_digest,e.correlation_id,e.occurred_at,e.event_kind,
               e.actor_id,e.action_id,e.audit_outcome,e.reason_code,
               e.subject_digest,e.diagnostic,e.has_resource_snapshot,
               e.resource_active_agents,e.resource_active_implementers,
               e.resource_elapsed_seconds,e.resource_attempt_number,
               e.resource_used_model_calls,e.resource_used_external_cost,
               e.event_digest
           )
         OR c.command_outcome IS DISTINCT FROM 'APPENDED'
         OR c.denial_reason IS DISTINCT FROM ''
         OR c.expected_sequence IS DISTINCT FROM 0
         OR c.before_sequence IS DISTINCT FROM 0
         OR c.after_sequence IS DISTINCT FROM e.sequence
         OR e.previous_event_digest IS DISTINCT FROM pg_catalog.decode(pg_catalog.repeat('00',32),'hex')
         OR c.expected_last_event_digest IS DISTINCT FROM e.previous_event_digest
         OR c.before_last_event_digest IS DISTINCT FROM e.previous_event_digest
         OR c.after_last_event_digest IS DISTINCT FROM e.event_digest
         OR c.expected_resource_revision IS DISTINCT FROM c.before_resource_revision
         OR c.expected_resource_projection_digest IS DISTINCT FROM c.before_resource_projection_digest
         OR c.expected_head_digest IS DISTINCT FROM c.before_head_digest
         OR c.after_resource_revision IS DISTINCT FROM e.resource_revision
         OR c.after_resource_projection_digest IS DISTINCT FROM e.resource_projection_digest)
), historical AS (
    SELECT 'lattice_task_submit.v1'::varchar(64) AS ingress_id,
           pg_catalog.substring(e.command_id::text, 12)::varchar(64) AS client_request_id,
           'CONTROLLED_CODEX_CANARY'::varchar(32) AS request_kind,
           e.stream_id AS ingress_request_digest,e.stream_id,e.sequence AS event_sequence,
           e.event_digest,e.command_id,e.request_digest AS command_request_digest
      FROM ONLY control.task_ledger_events AS e
      JOIN ONLY control.task_ledger_commands AS c
        ON c.stream_id=e.stream_id AND c.command_id=e.command_id
       AND ROW(
               c.request_digest,c.correlation_id,c.occurred_at,c.event_kind,
               c.actor_id,c.action_id,c.audit_outcome,c.reason_code,
               c.subject_digest,c.diagnostic,c.has_resource_snapshot,
               c.resource_active_agents,c.resource_active_implementers,
               c.resource_elapsed_seconds,c.resource_attempt_number,
               c.resource_used_model_calls,c.resource_used_external_cost,
               c.event_digest
           ) IS NOT DISTINCT FROM ROW(
               e.request_digest,e.correlation_id,e.occurred_at,e.event_kind,
               e.actor_id,e.action_id,e.audit_outcome,e.reason_code,
               e.subject_digest,e.diagnostic,e.has_resource_snapshot,
               e.resource_active_agents,e.resource_active_implementers,
               e.resource_elapsed_seconds,e.resource_attempt_number,
               e.resource_used_model_calls,e.resource_used_external_cost,
               e.event_digest
           )
       AND c.command_outcome='APPENDED'
       AND c.denial_reason=''
       AND c.expected_sequence=0
       AND c.before_sequence=0
       AND c.after_sequence=e.sequence
       AND e.previous_event_digest=pg_catalog.decode(pg_catalog.repeat('00',32),'hex')
       AND c.expected_last_event_digest=e.previous_event_digest
       AND c.before_last_event_digest=e.previous_event_digest
       AND c.after_last_event_digest=e.event_digest
       AND c.expected_resource_revision=c.before_resource_revision
       AND c.expected_resource_projection_digest=c.before_resource_projection_digest
       AND c.expected_head_digest=c.before_head_digest
       AND c.after_resource_revision=e.resource_revision
       AND c.after_resource_projection_digest=e.resource_projection_digest
     WHERE e.sequence=1
       AND e.event_kind='TASK_CREATED'
       AND e.action_id IN ('CONTROLLED_CODEX_CANARY','CONTROLLED_CODEX_CANARY_AUTONOMY_V1')
       AND e.audit_outcome='RECORDED'
       AND e.command_id::text ~ '^mcp-submit:[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       AND (pg_catalog.substring(e.command_id::text, 12) COLLATE pg_catalog."C") !~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
       AND NOT ((pg_catalog.substring(e.command_id::text, 12) COLLATE pg_catalog."C") ~* '-----begin '
           AND (pg_catalog.substring(e.command_id::text, 12) COLLATE pg_catalog."C") ~* 'private key-----')
       AND (pg_catalog.translate(pg_catalog.substring(e.command_id::text, 12), U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") !~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
       AND (pg_catalog.substring(e.command_id::text, 12) COLLATE pg_catalog."C") !~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
), classified AS (
    SELECT historical.*,
           count(*) OVER (
               PARTITION BY historical.ingress_id,historical.client_request_id
           ) AS historical_identity_count
      FROM historical
), expected_claims AS (
    SELECT 'lattice.task-ledger.task-ingress-claim/1.0'::varchar(64) AS schema_version,
           classified.ingress_id,classified.client_request_id,classified.request_kind,
           classified.ingress_request_digest,classified.stream_id,
           classified.event_sequence,classified.event_digest,classified.command_id,
           classified.command_request_digest
      FROM classified
     WHERE classified.historical_identity_count=1
), actual_candidate_claims AS (
    SELECT c.schema_version,c.ingress_id,c.client_request_id,c.request_kind,
           c.ingress_request_digest,c.stream_id,c.event_sequence,c.event_digest,
           c.command_id,c.command_request_digest
     FROM ONLY control.task_ingress_claims AS c
     WHERE c.ingress_id='lattice_task_submit.v1'
       AND NOT (
           c.request_kind='GENERAL_TASK'
           AND EXISTS (
               SELECT 1
                 FROM ONLY control.task_submission_envelopes AS v
                 JOIN ONLY control.task_ledger_streams AS s
                   ON s.stream_id=v.stream_id
                 JOIN ONLY control.task_ledger_events AS e
                   ON e.stream_id=v.stream_id AND e.sequence=v.event_sequence
                  AND e.event_digest=v.event_digest AND e.command_id=v.command_id
                  AND e.request_digest=v.request_digest
                 JOIN ONLY control.task_ledger_commands AS m
                   ON m.stream_id=v.stream_id AND m.command_id=v.command_id
                  AND m.request_digest=v.request_digest
                WHERE v.ingress_id=c.ingress_id
                  AND v.client_request_id=c.client_request_id
                  AND v.stream_id=c.stream_id
                  AND v.event_sequence=c.event_sequence
                  AND v.event_digest=c.event_digest
                  AND v.command_id=c.command_id
                  AND v.request_digest=c.command_request_digest
                  AND v.ingress_request_digest=c.ingress_request_digest
                  AND v.schema_version='lattice.task-ledger.task-submission/1.0'
                  AND v.admission_action='GENERAL_TASK_INTAKE_V1'
                  AND s.project_id=v.project_id
                  AND s.project_snapshot_id=v.project_snapshot_id
                  AND s.task_id=v.task_id
                  AND s.task_revision=v.task_revision
                  AND s.task_subject_kind='GENERAL_TASK_INTAKE'
                  AND s.task_subject_digest=v.intake_digest
                  AND s.task_spec_digest IS NULL
                  AND s.accounting_currency IS NULL
                  AND s.sequence=v.event_sequence
                  AND s.last_event_digest=v.event_digest
                  AND s.resource_revision=0
                  AND s.resource_projection_digest=pg_catalog.decode(pg_catalog.repeat('00',32),'hex')
                  AND s.active_agents=0 AND s.active_implementers=0
                  AND s.elapsed_seconds=0 AND s.attempt_number=0
                  AND s.used_model_calls=0 AND s.used_external_cost='0'
                  AND s.event_count=1 AND s.command_count=1 AND s.outbox_count=0
                  AND e.event_kind='TASK_CREATED'
                  AND e.action_id='GENERAL_TASK_INTAKE_V1'
                  AND e.audit_outcome='RECORDED'
                  AND e.reason_code='GENERAL_TASK_INTAKE_RECORDED'
                  AND e.subject_digest=v.envelope_digest
                  AND e.diagnostic='null'::jsonb
                  AND NOT e.has_resource_snapshot
                  AND e.resource_active_agents=0 AND e.resource_active_implementers=0
                  AND e.resource_elapsed_seconds=0 AND e.resource_attempt_number=0
                  AND e.resource_used_model_calls=0 AND e.resource_used_external_cost='0'
                  AND e.previous_event_digest=pg_catalog.decode(pg_catalog.repeat('00',32),'hex')
                  AND e.resource_revision=0
                  AND e.resource_projection_digest=pg_catalog.decode(pg_catalog.repeat('00',32),'hex')
                  AND ROW(
                          m.request_digest,m.correlation_id,m.occurred_at,m.event_kind,
                          m.actor_id,m.action_id,m.audit_outcome,m.reason_code,
                          m.subject_digest,m.diagnostic,m.has_resource_snapshot,
                          m.resource_active_agents,m.resource_active_implementers,
                          m.resource_elapsed_seconds,m.resource_attempt_number,
                          m.resource_used_model_calls,m.resource_used_external_cost,
                          m.event_digest
                      ) IS NOT DISTINCT FROM ROW(
                          e.request_digest,e.correlation_id,e.occurred_at,e.event_kind,
                          e.actor_id,e.action_id,e.audit_outcome,e.reason_code,
                          e.subject_digest,e.diagnostic,e.has_resource_snapshot,
                          e.resource_active_agents,e.resource_active_implementers,
                          e.resource_elapsed_seconds,e.resource_attempt_number,
                          e.resource_used_model_calls,e.resource_used_external_cost,
                          e.event_digest
                      )
                  AND m.command_outcome='APPENDED'
                  AND m.denial_reason=''
                  AND m.expected_sequence=0 AND m.before_sequence=0
                  AND m.after_sequence=v.event_sequence
                  AND m.expected_last_event_digest=e.previous_event_digest
                  AND m.before_last_event_digest=e.previous_event_digest
                  AND m.after_last_event_digest=e.event_digest
                  AND m.expected_resource_revision=m.before_resource_revision
                  AND m.expected_resource_projection_digest=m.before_resource_projection_digest
                  AND m.expected_head_digest=m.before_head_digest
                  AND m.after_resource_revision=e.resource_revision
                  AND m.after_resource_projection_digest=e.resource_projection_digest
                  AND m.after_head_digest=s.head_digest
           )
       )
), expected_ambiguities AS (
    SELECT 'lattice.task-ledger.task-ingress-historical-ambiguity/1.0'::varchar(64)
               AS schema_version,
           classified.ingress_id,classified.client_request_id,classified.request_kind,
           classified.ingress_request_digest,classified.stream_id,
           classified.event_sequence,classified.event_digest,classified.command_id,
           classified.command_request_digest
      FROM classified
     WHERE classified.historical_identity_count>1
), claim_mismatch AS (
    (SELECT * FROM expected_claims EXCEPT SELECT * FROM actual_candidate_claims)
    UNION ALL
    (SELECT * FROM actual_candidate_claims EXCEPT SELECT * FROM expected_claims)
), ambiguity_mismatch AS (
    (SELECT * FROM expected_ambiguities
     EXCEPT
     SELECT a.schema_version,a.ingress_id,a.client_request_id,a.request_kind,
            a.ingress_request_digest,a.stream_id,a.event_sequence,a.event_digest,
            a.command_id,a.command_request_digest
       FROM ONLY control.task_ingress_historical_ambiguities AS a)
    UNION ALL
    (SELECT a.schema_version,a.ingress_id,a.client_request_id,a.request_kind,
            a.ingress_request_digest,a.stream_id,a.event_sequence,a.event_digest,
            a.command_id,a.command_request_digest
       FROM ONLY control.task_ingress_historical_ambiguities AS a
     EXCEPT
     SELECT * FROM expected_ambiguities)
)
SELECT (
           (session_user='lattice_migrator_login'
                AND pg_catalog.current_setting('role')='lattice_migrator')
        OR (session_user='lattice_runtime_login'
                AND pg_catalog.current_setting('role')='lattice_runtime')
        OR (session_user='lattice_guardian_login'
                AND pg_catalog.current_setting('role')='lattice_guardian')
        OR (session_user='lattice_readonly_login'
                AND pg_catalog.current_setting('role')='lattice_readonly')
       )
   AND NOT EXISTS (SELECT 1 FROM candidate_audit_mismatch)
   AND NOT EXISTS (SELECT 1 FROM candidate_event_presence_mismatch)
   AND NOT EXISTS (SELECT 1 FROM candidate_binding_mismatch)
   AND NOT EXISTS (SELECT 1 FROM claim_mismatch)
   AND NOT EXISTS (SELECT 1 FROM ambiguity_mismatch)
   AND NOT EXISTS (
       SELECT 1
         FROM classified AS duplicate
         JOIN ONLY control.task_ingress_claims AS c
           ON c.ingress_id=duplicate.ingress_id
          AND c.client_request_id=duplicate.client_request_id
        WHERE duplicate.historical_identity_count>1
   )
   AND NOT EXISTS (
       SELECT 1
         FROM ONLY control.task_ingress_claims AS c
         JOIN ONLY control.task_ingress_historical_ambiguities AS a
           ON a.ingress_id=c.ingress_id
          AND a.client_request_id=c.client_request_id
   )
$lattice_task_ingress_historical_closure_v1$;

REVOKE ALL ON FUNCTION control.task_ingress_prepare_v1(text,text,text,bytea,bytea)
    FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
         lattice_migrator_login, lattice_runtime_login, lattice_guardian_login,
         lattice_readonly_login;
REVOKE ALL ON FUNCTION control.task_ingress_record_v1(
    text,text,text,text,bytea,bytea,text,bytea,text,bytea
) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
       lattice_migrator_login, lattice_runtime_login, lattice_guardian_login,
       lattice_readonly_login;
REVOKE ALL ON FUNCTION control.task_ingress_read_by_request_v1(text,text)
    FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
         lattice_migrator_login, lattice_runtime_login, lattice_guardian_login,
         lattice_readonly_login;
REVOKE ALL ON FUNCTION control.task_ingress_historical_closure_v1()
    FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
         lattice_migrator_login, lattice_runtime_login, lattice_guardian_login,
         lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.task_ingress_prepare_v1(text,text,text,bytea,bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.task_ingress_record_v1(
    text,text,text,text,bytea,bytea,text,bytea,text,bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.task_ingress_read_by_request_v1(text,text)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.task_ingress_historical_closure_v1()
    TO lattice_migrator, lattice_runtime, lattice_guardian, lattice_readonly;

CREATE FUNCTION control.task_submission_prepare_v1(
    p_ingress_id text,
    p_client_request_id text,
    p_envelope_digest bytea
)
RETURNS TABLE (
    found boolean,
    schema_version text,
    ingress_id text,
    client_request_id text,
    objective text,
    project_display_name text,
    project_authority_receipt_digest bytea,
    project_id text,
    project_snapshot_id text,
    task_id text,
    task_revision text,
    task_subject_kind text,
    intake_digest bytea,
    stream_id bytea,
    task_ref text,
    admission_action text,
    envelope_digest bytea,
    event_sequence text,
    event_digest bytea,
    command_id text,
    request_digest bytea,
    ingress_request_digest bytea
)
LANGUAGE plpgsql
VOLATILE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_task_submission_prepare_v1$
DECLARE
    v_existing control.task_submission_envelopes%ROWTYPE;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
       OR p_ingress_id IS NULL
       OR p_ingress_id !~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       OR p_client_request_id IS NULL
       OR p_client_request_id !~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       OR (p_client_request_id COLLATE pg_catalog."C") ~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
       OR ((p_client_request_id COLLATE pg_catalog."C") ~* '-----begin '
           AND (p_client_request_id COLLATE pg_catalog."C") ~* 'private key-----')
       OR (pg_catalog.translate(p_client_request_id, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") ~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
       OR (p_client_request_id COLLATE pg_catalog."C") ~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
       OR p_envelope_digest IS NULL
       OR pg_catalog.octet_length(p_envelope_digest) <> 32
       OR p_envelope_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01';
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(p_ingress_id || ':' || p_client_request_id, 0)
    );
    SELECT * INTO v_existing
      FROM ONLY control.task_submission_envelopes AS s
     WHERE s.ingress_id = p_ingress_id
       AND s.client_request_id = p_client_request_id
     FOR SHARE;
    IF FOUND AND v_existing.envelope_digest IS DISTINCT FROM p_envelope_digest THEN
        RAISE EXCEPTION USING ERRCODE = 'LTX01';
    END IF;
    IF NOT FOUND THEN
        RETURN QUERY SELECT false,
            NULL::text,NULL::text,NULL::text,NULL::text,NULL::text,NULL::bytea,
            NULL::text,NULL::text,NULL::text,NULL::text,NULL::text,NULL::bytea,
            NULL::bytea,NULL::text,NULL::text,NULL::bytea,NULL::text,NULL::bytea,
            NULL::text,NULL::bytea,NULL::bytea;
        RETURN;
    END IF;
    RETURN QUERY SELECT true,
        v_existing.schema_version::text,v_existing.ingress_id::text,
        v_existing.client_request_id::text,v_existing.objective::text,
        v_existing.project_display_name::text,
        v_existing.project_authority_receipt_digest,v_existing.project_id::text,
        v_existing.project_snapshot_id::text,v_existing.task_id::text,
        v_existing.task_revision::text,v_existing.task_subject_kind::text,
        v_existing.intake_digest,v_existing.stream_id,
        v_existing.task_ref::text,v_existing.admission_action::text,
        v_existing.envelope_digest,v_existing.event_sequence::text,
        v_existing.event_digest,v_existing.command_id::text,v_existing.request_digest,
        v_existing.ingress_request_digest;
END
$lattice_task_submission_prepare_v1$;

CREATE FUNCTION control.task_submission_record_v1(
    p_schema_version text,
    p_ingress_id text,
    p_client_request_id text,
    p_objective text,
    p_project_display_name text,
    p_project_authority_receipt_digest bytea,
    p_project_id text,
    p_project_snapshot_id text,
    p_task_id text,
    p_task_revision text,
    p_task_subject_kind text,
    p_intake_digest bytea,
    p_stream_id bytea,
    p_task_ref text,
    p_admission_action text,
    p_envelope_digest bytea,
    p_event_sequence text,
    p_event_digest bytea,
    p_command_id text,
    p_request_digest bytea,
    p_ingress_request_digest bytea
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
AS $lattice_task_submission_record_v1$
DECLARE
    v_event control.task_ledger_events%ROWTYPE;
    v_stream control.task_ledger_streams%ROWTYPE;
    v_claim control.task_ingress_claims%ROWTYPE;
    v_project control.project_registry_projects%ROWTYPE;
    v_existing control.task_submission_envelopes%ROWTYPE;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
       OR p_schema_version IS DISTINCT FROM 'lattice.task-ledger.task-submission/1.0'
       OR p_client_request_id IS NULL
       OR p_client_request_id !~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       OR (p_client_request_id COLLATE pg_catalog."C") ~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
       OR ((p_client_request_id COLLATE pg_catalog."C") ~* '-----begin '
           AND (p_client_request_id COLLATE pg_catalog."C") ~* 'private key-----')
       OR (pg_catalog.translate(p_client_request_id, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") ~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
       OR (p_client_request_id COLLATE pg_catalog."C") ~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
       OR p_objective IS NULL
       OR p_objective = ''
       OR p_objective IS DISTINCT FROM pg_catalog.btrim(p_objective)
       OR p_objective IS NOT NFC NORMALIZED
       OR pg_catalog.translate(p_objective, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) IS DISTINCT FROM pg_catalog.btrim(pg_catalog.translate(p_objective, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)))
       OR pg_catalog.char_length(p_objective) > 512
       OR pg_catalog.octet_length(p_objective) > 2048
       OR (p_objective COLLATE pg_catalog."C") ~ U&'[\0001-\001F\007F-\009F]'
       OR (p_objective COLLATE pg_catalog."C") ~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
       OR ((p_objective COLLATE pg_catalog."C") ~* '-----begin '
           AND (p_objective COLLATE pg_catalog."C") ~* 'private key-----')
       OR (pg_catalog.translate(p_objective, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") ~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
       OR (p_objective COLLATE pg_catalog."C") ~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
       OR p_project_display_name IS NULL
       OR p_project_display_name = ''
       OR p_project_display_name IS DISTINCT FROM pg_catalog.btrim(p_project_display_name)
       OR p_project_display_name IS NOT NFC NORMALIZED
       OR pg_catalog.translate(p_project_display_name, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) IS DISTINCT FROM pg_catalog.btrim(pg_catalog.translate(p_project_display_name, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)))
       OR pg_catalog.char_length(p_project_display_name) > 64
       OR pg_catalog.octet_length(p_project_display_name) > 256
       OR (p_project_display_name COLLATE pg_catalog."C") ~ U&'[\0001-\001F\007F-\009F]'
       OR (p_project_display_name COLLATE pg_catalog."C") ~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
       OR ((p_project_display_name COLLATE pg_catalog."C") ~* '-----begin '
           AND (p_project_display_name COLLATE pg_catalog."C") ~* 'private key-----')
       OR (pg_catalog.translate(p_project_display_name, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") ~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
       OR (p_project_display_name COLLATE pg_catalog."C") ~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
       OR (p_project_id COLLATE pg_catalog."C") ~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
       OR ((p_project_id COLLATE pg_catalog."C") ~* '-----begin '
           AND (p_project_id COLLATE pg_catalog."C") ~* 'private key-----')
       OR (pg_catalog.translate(p_project_id, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") ~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
       OR (p_project_id COLLATE pg_catalog."C") ~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
       OR (p_project_snapshot_id COLLATE pg_catalog."C") ~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
       OR ((p_project_snapshot_id COLLATE pg_catalog."C") ~* '-----begin '
           AND (p_project_snapshot_id COLLATE pg_catalog."C") ~* 'private key-----')
       OR (pg_catalog.translate(p_project_snapshot_id, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") ~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
       OR (p_project_snapshot_id COLLATE pg_catalog."C") ~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
       OR p_task_subject_kind IS DISTINCT FROM 'GENERAL_TASK_INTAKE'
       OR p_admission_action IS DISTINCT FROM 'GENERAL_TASK_INTAKE_V1'
       OR p_intake_digest IS NULL
       OR pg_catalog.octet_length(p_intake_digest) <> 32
       OR p_intake_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_ingress_request_digest IS NULL
       OR pg_catalog.octet_length(p_ingress_request_digest) <> 32
       OR p_ingress_request_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01';
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(p_ingress_id || ':' || p_client_request_id, 0)
    );
    SELECT * INTO v_existing
      FROM ONLY control.task_submission_envelopes AS s
     WHERE s.ingress_id = p_ingress_id
       AND s.client_request_id = p_client_request_id
     FOR SHARE;
    IF FOUND THEN
        IF v_existing.schema_version IS DISTINCT FROM p_schema_version
           OR v_existing.objective IS DISTINCT FROM p_objective
           OR v_existing.project_display_name IS DISTINCT FROM p_project_display_name
           OR v_existing.project_authority_receipt_digest IS DISTINCT FROM p_project_authority_receipt_digest
           OR v_existing.project_id IS DISTINCT FROM p_project_id
           OR v_existing.project_snapshot_id IS DISTINCT FROM p_project_snapshot_id
           OR v_existing.task_id IS DISTINCT FROM p_task_id
           OR v_existing.task_revision::text IS DISTINCT FROM p_task_revision
           OR v_existing.task_subject_kind IS DISTINCT FROM p_task_subject_kind
           OR v_existing.intake_digest IS DISTINCT FROM p_intake_digest
           OR v_existing.stream_id IS DISTINCT FROM p_stream_id
           OR v_existing.task_ref IS DISTINCT FROM p_task_ref
           OR v_existing.admission_action IS DISTINCT FROM p_admission_action
           OR v_existing.envelope_digest IS DISTINCT FROM p_envelope_digest
           OR v_existing.event_sequence::text IS DISTINCT FROM p_event_sequence
           OR v_existing.event_digest IS DISTINCT FROM p_event_digest
           OR v_existing.command_id IS DISTINCT FROM p_command_id
           OR v_existing.request_digest IS DISTINCT FROM p_request_digest
           OR v_existing.ingress_request_digest IS DISTINCT FROM p_ingress_request_digest
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LTX01';
        END IF;
        RETURN 'REPLAYED';
    END IF;

    SELECT p.* INTO v_project
      FROM ONLY control.project_registry_projects AS p
     WHERE p.project_id=p_project_id
     FOR SHARE OF p;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'LPG01', MESSAGE = 'project registry currentness conflict';
    END IF;
    IF v_project.authority_lifecycle IS DISTINCT FROM 'ACTIVE' THEN
        RAISE EXCEPTION USING ERRCODE = 'LPG02', MESSAGE = 'project registry project inactive';
    END IF;
    IF v_project.authority_runtime IS DISTINCT FROM 'LIVE'
       OR v_project.drift_canonical_root IS DISTINCT FROM false
       OR v_project.drift_repository IS DISTINCT FROM false
       OR v_project.drift_file IS DISTINCT FROM false
       OR v_project.drift_primary_ref_name IS DISTINCT FROM false
       OR v_project.drift_primary_ref_storage IS DISTINCT FROM false
       OR v_project.authority_snapshot_id IS DISTINCT FROM p_project_snapshot_id
       OR v_project.authority_receipt_digest IS DISTINCT FROM p_project_authority_receipt_digest
       OR v_project.authority_observation_digest IS DISTINCT FROM v_project.accepted_observation_digest
       OR v_project.pending_observation_digest IS NOT NULL
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LPG01', MESSAGE = 'project registry currentness conflict';
    END IF;

    SELECT * INTO v_claim
      FROM ONLY control.task_ingress_claims AS c
     WHERE c.ingress_id=p_ingress_id
       AND c.client_request_id=p_client_request_id
       AND c.stream_id=p_stream_id
       AND c.request_kind='GENERAL_TASK'
       AND c.ingress_request_digest=p_ingress_request_digest
       AND c.xmin=pg_catalog.pg_current_xact_id()::xid
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01';
    END IF;

    SELECT * INTO v_event
      FROM ONLY control.task_ledger_events AS e
     WHERE e.stream_id = p_stream_id
       AND e.sequence = p_event_sequence::numeric
       AND e.xmin = pg_catalog.pg_current_xact_id()::xid
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01';
    END IF;
    SELECT * INTO v_stream
      FROM ONLY control.task_ledger_streams AS s
     WHERE s.stream_id = p_stream_id
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01';
    END IF;
    IF v_event.event_digest IS DISTINCT FROM p_event_digest
       OR v_event.command_id IS DISTINCT FROM p_command_id
       OR v_event.request_digest IS DISTINCT FROM p_request_digest
       OR v_event.event_kind IS DISTINCT FROM 'TASK_CREATED'
       OR v_event.action_id IS DISTINCT FROM 'GENERAL_TASK_INTAKE_V1'
       OR v_event.audit_outcome IS DISTINCT FROM 'RECORDED'
       OR v_event.reason_code IS DISTINCT FROM 'GENERAL_TASK_INTAKE_RECORDED'
       OR v_event.subject_digest IS DISTINCT FROM p_envelope_digest
       OR v_event.diagnostic IS DISTINCT FROM 'null'::jsonb
       OR v_stream.project_id IS DISTINCT FROM p_project_id
       OR v_stream.project_snapshot_id IS DISTINCT FROM p_project_snapshot_id
       OR v_stream.task_id IS DISTINCT FROM p_task_id
       OR v_stream.task_revision::text IS DISTINCT FROM p_task_revision
       OR v_stream.task_subject_kind IS DISTINCT FROM p_task_subject_kind
       OR v_stream.task_subject_digest IS DISTINCT FROM p_intake_digest
       OR v_stream.task_spec_digest IS NOT NULL
       OR v_stream.accounting_currency IS NOT NULL
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01';
    END IF;

    INSERT INTO control.task_submission_envelopes (
        schema_version,ingress_id,client_request_id,objective,project_display_name,
        project_authority_receipt_digest,project_id,project_snapshot_id,task_id,
        task_revision,task_subject_kind,intake_digest,stream_id,task_ref,
        admission_action,envelope_digest,event_sequence,event_digest,command_id,request_digest,
        ingress_request_digest
    ) VALUES (
        p_schema_version,p_ingress_id,p_client_request_id,p_objective,p_project_display_name,
        p_project_authority_receipt_digest,p_project_id,p_project_snapshot_id,p_task_id,
        p_task_revision::numeric,p_task_subject_kind,p_intake_digest,p_stream_id,p_task_ref,
        p_admission_action,p_envelope_digest,p_event_sequence::numeric,p_event_digest,
        p_command_id,p_request_digest,p_ingress_request_digest
    );
    RETURN 'RECORDED';
EXCEPTION
    WHEN unique_violation THEN
        RAISE EXCEPTION USING ERRCODE = 'LTX01';
END
$lattice_task_submission_record_v1$;

CREATE FUNCTION control.task_submission_read_by_task_ref_v1(p_task_ref text)
RETURNS TABLE (
    schema_version text,ingress_id text,client_request_id text,objective text,
    project_display_name text,project_authority_receipt_digest bytea,
    project_id text,project_snapshot_id text,task_id text,task_revision text,
    task_subject_kind text,intake_digest bytea,stream_id bytea,task_ref text,
    admission_action text,envelope_digest bytea,event_sequence text,event_digest bytea,
    command_id text,request_digest bytea,ingress_request_digest bytea
)
LANGUAGE sql
STABLE
PARALLEL SAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_task_submission_read_by_task_ref_v1$
    SELECT s.schema_version::text,s.ingress_id::text,s.client_request_id::text,
           s.objective::text,s.project_display_name::text,
           s.project_authority_receipt_digest,s.project_id::text,
           s.project_snapshot_id::text,s.task_id::text,s.task_revision::text,
           s.task_subject_kind::text,s.intake_digest,s.stream_id,s.task_ref::text,
           s.admission_action::text,s.envelope_digest,s.event_sequence::text,
           s.event_digest,s.command_id::text,s.request_digest,s.ingress_request_digest
      FROM ONLY control.task_submission_envelopes AS s
     WHERE s.task_ref=p_task_ref
       AND session_user='lattice_runtime_login'
       AND pg_catalog.current_setting('role')='lattice_runtime'
$lattice_task_submission_read_by_task_ref_v1$;

CREATE FUNCTION control.task_submission_read_by_request_v1(
    p_ingress_id text,
    p_client_request_id text
)
RETURNS TABLE (
    schema_version text,ingress_id text,client_request_id text,objective text,
    project_display_name text,project_authority_receipt_digest bytea,
    project_id text,project_snapshot_id text,task_id text,task_revision text,
    task_subject_kind text,intake_digest bytea,stream_id bytea,task_ref text,
    admission_action text,envelope_digest bytea,event_sequence text,event_digest bytea,
    command_id text,request_digest bytea,ingress_request_digest bytea
)
LANGUAGE sql
STABLE
PARALLEL SAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_task_submission_read_by_request_v1$
    SELECT s.schema_version::text,s.ingress_id::text,s.client_request_id::text,
           s.objective::text,s.project_display_name::text,
           s.project_authority_receipt_digest,s.project_id::text,
           s.project_snapshot_id::text,s.task_id::text,s.task_revision::text,
           s.task_subject_kind::text,s.intake_digest,s.stream_id,s.task_ref::text,
           s.admission_action::text,s.envelope_digest,s.event_sequence::text,
           s.event_digest,s.command_id::text,s.request_digest,s.ingress_request_digest
      FROM ONLY control.task_submission_envelopes AS s
     WHERE s.ingress_id=p_ingress_id AND s.client_request_id=p_client_request_id
       AND p_ingress_id ~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       AND p_client_request_id ~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
       AND (p_client_request_id COLLATE pg_catalog."C") !~* '(bearer |(^|[^A-Za-z0-9])sk-|github_pat_|gh[pousr]_|glpat-|npm_|pypi-|xox[abprs]-)'
       AND NOT ((p_client_request_id COLLATE pg_catalog."C") ~* '-----begin '
                AND (p_client_request_id COLLATE pg_catalog."C") ~* 'private key-----')
       AND (pg_catalog.translate(p_client_request_id, U&'\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000', pg_catalog.repeat(' ',19)) COLLATE pg_catalog."C") !~* '(^|[^A-Za-z0-9_-])(password|passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token|id_token|id-token|session_token|session-token|api_key|api-key|apikey|client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization)[[:space:]]*["'']?[[:space:]]*[:=]'
       AND (p_client_request_id COLLATE pg_catalog."C") !~ '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)'
       AND session_user='lattice_runtime_login'
       AND pg_catalog.current_setting('role')='lattice_runtime'
$lattice_task_submission_read_by_request_v1$;

REVOKE ALL ON FUNCTION control.task_submission_prepare_v1(text,text,bytea)
    FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
         lattice_migrator_login, lattice_runtime_login, lattice_guardian_login,
         lattice_readonly_login;
REVOKE ALL ON FUNCTION control.task_submission_record_v1(
    text,text,text,text,text,bytea,text,text,text,text,text,bytea,bytea,text,text,
    bytea,text,bytea,text,bytea,bytea
) FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
       lattice_migrator_login, lattice_runtime_login, lattice_guardian_login,
       lattice_readonly_login;
REVOKE ALL ON FUNCTION control.task_submission_read_by_task_ref_v1(text)
    FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
         lattice_migrator_login, lattice_runtime_login, lattice_guardian_login,
         lattice_readonly_login;
REVOKE ALL ON FUNCTION control.task_submission_read_by_request_v1(text,text)
    FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
         lattice_migrator_login, lattice_runtime_login, lattice_guardian_login,
         lattice_readonly_login;
GRANT EXECUTE ON FUNCTION control.task_submission_prepare_v1(text,text,bytea)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.task_submission_record_v1(
    text,text,text,text,text,bytea,text,text,text,text,text,bytea,bytea,text,text,
    bytea,text,bytea,text,bytea,bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.task_submission_read_by_task_ref_v1(text)
    TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.task_submission_read_by_request_v1(text,text)
    TO lattice_runtime;
