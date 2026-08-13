-- TASK-050: one event-owned, fixed-scalar autonomy receipt subject.
-- The migration runner owns the surrounding transaction.

ALTER TABLE control.task_ledger_commands
    DROP CONSTRAINT task_ledger_commands_closed_values;
ALTER TABLE control.task_ledger_commands
    ADD CONSTRAINT task_ledger_commands_closed_values CHECK (
        event_kind IN (
            'TASK_CREATED', 'AUTONOMY_RECEIPT_RECORDED', 'STATE_TRANSITION',
            'POLICY_DECISION', 'RESOURCE_SNAPSHOT', 'EFFECT_INTENT',
            'EFFECT_OUTCOME', 'EVIDENCE_RECORDED'
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
    );

ALTER TABLE control.task_ledger_events
    DROP CONSTRAINT task_ledger_events_closed_values;
ALTER TABLE control.task_ledger_events
    ADD CONSTRAINT task_ledger_events_closed_values CHECK (
        event_kind IN (
            'TASK_CREATED', 'AUTONOMY_RECEIPT_RECORDED', 'STATE_TRANSITION',
            'POLICY_DECISION', 'RESOURCE_SNAPSHOT', 'EFFECT_INTENT',
            'EFFECT_OUTCOME', 'EVIDENCE_RECORDED'
        )
        AND audit_outcome IN (
            'RECORDED', 'ALLOWED', 'DENIED', 'PASSED', 'FAILED',
            'BLOCKED', 'CANCELLED'
        )
    );

-- The v3 store and v1 Task Ledger entry points deliberately fail closed when
-- the recorded manifest does not have the exact number of entries they were
-- shipped with.  The 0004 prefix is verified before this migration runs, so
-- rewrite only those four pinned definitions to admit the new fifth entry.
-- Keeping the entry-point signatures stable preserves the existing runtime
-- protocol while making a 0004 -> 0005 upgrade immediately usable.
DO $lattice_task_050_manifest_count_upgrade$
DECLARE
    v_function_name text;
    v_function_oid oid;
    v_definition text;
    v_rewritten_definition text;
BEGIN
    FOREACH v_function_name IN ARRAY ARRAY[
        'store_prepare_v3',
        'store_current_head_v3',
        'task_ledger_prepare_v1',
        'task_ledger_read_head_v1'
    ]
    LOOP
        SELECT p.oid
          INTO STRICT v_function_oid
          FROM pg_catalog.pg_proc p
          JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
         WHERE n.nspname = 'control'
           AND p.proname = v_function_name;

        v_definition := pg_catalog.pg_get_functiondef(v_function_oid);
        v_rewritten_definition := pg_catalog.replace(
            pg_catalog.replace(
                v_definition,
                'v_manifest_entry_count IS DISTINCT FROM 4',
                'v_manifest_entry_count IS DISTINCT FROM 5'
            ),
            'v_manifest_entry_count = 4',
            'v_manifest_entry_count = 5'
        );

        IF v_rewritten_definition = v_definition THEN
            RAISE EXCEPTION 'pinned manifest-count guard not found in control.%',
                v_function_name;
        END IF;

        EXECUTE v_rewritten_definition;
    END LOOP;
END
$lattice_task_050_manifest_count_upgrade$;

DO $lattice_task_050_finalize_event_upgrade$
DECLARE
    v_function_oid oid;
    v_definition text;
    v_rewritten_definition text;
BEGIN
    SELECT p.oid
      INTO STRICT v_function_oid
      FROM pg_catalog.pg_proc p
      JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
     WHERE n.nspname = 'control'
       AND p.proname = 'task_ledger_finalize_v1';

    v_definition := pg_catalog.pg_get_functiondef(v_function_oid);
    v_rewritten_definition := pg_catalog.replace(
        v_definition,
        '''TASK_CREATED'', ''STATE_TRANSITION'', ''POLICY_DECISION''',
        '''TASK_CREATED'', ''AUTONOMY_RECEIPT_RECORDED'', ''STATE_TRANSITION'', ''POLICY_DECISION'''
    );
    IF v_rewritten_definition = v_definition THEN
        RAISE EXCEPTION 'pinned event-kind guard not found in control.task_ledger_finalize_v1';
    END IF;
    EXECUTE v_rewritten_definition;
END
$lattice_task_050_finalize_event_upgrade$;

CREATE TABLE control.task_ledger_autonomy_receipts (
    stream_id bytea NOT NULL,
    event_sequence numeric(20,0) NOT NULL,
    event_digest bytea NOT NULL,
    receipt_schema_version varchar(40) NOT NULL,
    intent_version varchar(8) NOT NULL,
    task_kind varchar(24) NOT NULL,
    risk_class varchar(4) NOT NULL,
    execution_preapproved boolean NOT NULL,
    requires_new_authority boolean NOT NULL,
    irreversible_or_high_risk boolean NOT NULL,
    observed_task_state varchar(40) NOT NULL,
    disposition varchar(16) NOT NULL,
    decision_reason varchar(40) NOT NULL,
    model varchar(40),
    verification varchar(40),
    authority_mode varchar(40) NOT NULL,
    process_start_authority_digest bytea NOT NULL,
    ingress_profile_adapter_commitment bytea NOT NULL,
    store_authority_head_digest bytea NOT NULL,
    policy_decision_receipt_digest bytea,
    policy_owner_head_digest bytea,
    approval_receipt_digest bytea,
    approval_owner_head_digest bytea,
    writer_lease_receipt_digest bytea,
    writer_lease_head_digest bytea,
    writer_fencing_token numeric(20,0),
    authority_digest bytea NOT NULL,
    receipt_digest bytea NOT NULL,
    PRIMARY KEY (stream_id, event_sequence),
    UNIQUE (event_digest),
    UNIQUE (receipt_digest),
    CONSTRAINT task_ledger_autonomy_receipts_event_fk
        FOREIGN KEY (stream_id, event_sequence)
        REFERENCES control.task_ledger_events (stream_id, sequence),
    CONSTRAINT task_ledger_autonomy_receipts_version_exact CHECK (
        receipt_schema_version = 'lattice.autonomy-receipt/1.0'
        AND intent_version = '1.0'
        AND authority_mode = 'P0_PROCESS_START_PROFILE_V1'
    ),
    CONSTRAINT task_ledger_autonomy_receipts_closed_values CHECK (
        task_kind IN ('FEATURE', 'BUG_FIX', 'CONFIGURATION', 'RESEARCH')
        AND risk_class IN ('R0', 'R1', 'R2', 'R3')
        AND observed_task_state IN (
            'DRAFT', 'AWAITING_EXECUTION_APPROVAL', 'PREPARING', 'EXECUTING',
            'VERIFYING', 'REVIEWING', 'AWAITING_MERGE_APPROVAL', 'MERGING',
            'COMPLETED', 'FAILED', 'STOPPING'
        )
        AND disposition IN ('PROCEED', 'ASK_USER')
        AND decision_reason IN (
            'ROUTINE_AUTHORIZED', 'NEW_USER_DECISION', 'NEW_AUTHORITY',
            'HIGH_RISK_OR_IRREVERSIBLE'
        )
        AND (model IS NULL OR model IN ('GOVERNED_CODEX_WRITER', 'NO_MODEL'))
        AND (verification IS NULL OR verification IN (
            'FOCUSED_CHECKS', 'BUILD_AND_FOCUSED_CHECKS', 'READ_ONLY_EVIDENCE'
        ))
    ),
    CONSTRAINT task_ledger_autonomy_receipts_decision_shape CHECK (
        (disposition = 'PROCEED'
         AND decision_reason = 'ROUTINE_AUTHORIZED'
         AND model IS NOT NULL AND verification IS NOT NULL
         AND writer_lease_receipt_digest IS NOT NULL
         AND writer_lease_head_digest IS NOT NULL
         AND writer_fencing_token IS NOT NULL)
        OR
        (disposition = 'ASK_USER'
         AND model IS NULL AND verification IS NULL
         AND writer_lease_receipt_digest IS NULL
         AND writer_lease_head_digest IS NULL
         AND writer_fencing_token IS NULL)
    ),
    CONSTRAINT task_ledger_autonomy_receipts_p0_owner_shape CHECK (
        policy_decision_receipt_digest IS NULL
        AND policy_owner_head_digest IS NULL
        AND approval_receipt_digest IS NULL
        AND approval_owner_head_digest IS NULL
    ),
    CONSTRAINT task_ledger_autonomy_receipts_digest_shapes CHECK (
        pg_catalog.octet_length(stream_id) = 32
        AND pg_catalog.octet_length(event_digest) = 32
        AND pg_catalog.octet_length(process_start_authority_digest) = 32
        AND pg_catalog.octet_length(ingress_profile_adapter_commitment) = 32
        AND pg_catalog.octet_length(store_authority_head_digest) = 32
        AND (writer_lease_receipt_digest IS NULL
             OR pg_catalog.octet_length(writer_lease_receipt_digest) = 32)
        AND (writer_lease_head_digest IS NULL
             OR pg_catalog.octet_length(writer_lease_head_digest) = 32)
        AND pg_catalog.octet_length(authority_digest) = 32
        AND pg_catalog.octet_length(receipt_digest) = 32
        AND stream_id <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND event_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND process_start_authority_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND ingress_profile_adapter_commitment <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND store_authority_head_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND authority_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND receipt_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    ),
    CONSTRAINT task_ledger_autonomy_receipts_u64 CHECK (
        event_sequence = 2
        AND writer_fencing_token >= 1
        AND writer_fencing_token <= 18446744073709551615
    )
);

CREATE FUNCTION control.task_ledger_record_autonomy_receipt_v1(
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

CREATE FUNCTION control.task_ledger_read_autonomy_receipts_v1(p_stream_id bytea)
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
) FROM PUBLIC;
REVOKE ALL ON FUNCTION control.task_ledger_read_autonomy_receipts_v1(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control.task_ledger_record_autonomy_receipt_v1(
    bytea,text,bytea,text,text,text,text,boolean,boolean,boolean,text,text,text,
    text,text,text,bytea,bytea,bytea,bytea,bytea,text,bytea,bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.task_ledger_read_autonomy_receipts_v1(bytea)
    TO lattice_runtime;
