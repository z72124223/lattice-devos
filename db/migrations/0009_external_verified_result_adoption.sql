-- External verified-result adoption: schema-v7 to schema-v8.
-- Historical migrations remain immutable.  This migration owns no worker,
-- provider, or Writer-Lease authority.

ALTER TABLE control.task_ledger_commands
    DROP CONSTRAINT task_ledger_commands_closed_values;
ALTER TABLE control.task_ledger_commands
    ADD CONSTRAINT task_ledger_commands_closed_values CHECK (
        event_kind IN ('TASK_CREATED', 'AUTONOMY_RECEIPT_RECORDED',
            'FOREMAN_SNAPSHOT_RECORDED', 'EXTERNAL_VERIFIED_RESULT_ADOPTED',
            'STATE_TRANSITION', 'POLICY_DECISION', 'RESOURCE_SNAPSHOT',
            'EFFECT_INTENT', 'EFFECT_OUTCOME', 'EVIDENCE_RECORDED')
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
            'FOREMAN_SNAPSHOT_RECORDED', 'EXTERNAL_VERIFIED_RESULT_ADOPTED',
            'STATE_TRANSITION', 'POLICY_DECISION', 'RESOURCE_SNAPSHOT',
            'EFFECT_INTENT', 'EFFECT_OUTCOME', 'EVIDENCE_RECORDED')
        AND audit_outcome IN ('RECORDED', 'ALLOWED', 'DENIED', 'PASSED', 'FAILED', 'BLOCKED', 'CANCELLED')
    );

-- This is an immutable receipt catalog, not a second Artifact Store.  It is
-- populated only by the existing migrator-owned external-verification ingress;
-- Runtime receives a security-definer read surface and cannot create evidence.
CREATE TABLE control.external_verified_result_evidence (
    adoption_digest bytea PRIMARY KEY,
    project_id varchar(64) NOT NULL,
    project_snapshot_id varchar(159) NOT NULL,
    task_ref char(64) NOT NULL,
    source_sha char(40) NOT NULL,
    target_sha char(40) NOT NULL,
    remote_target_sha char(40) NOT NULL,
    push_merge_receipt_ref varchar(80) NOT NULL,
    deployment_receipt_ref varchar(80) NOT NULL,
    deployment_artifact_ref varchar(80) NOT NULL,
    independent_acceptance_ref varchar(80) NOT NULL,
    protected_action_approval_refs varchar(80)[] NOT NULL,
    deployment_artifact_sha256 bytea NOT NULL,
    config_command_sha256 bytea NOT NULL,
    independent_verifier varchar(128) NOT NULL,
    non_force_push_merge boolean NOT NULL,
    descriptor_digest bytea NOT NULL UNIQUE,
    CHECK (project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'
        AND project_snapshot_id ~ '^[a-z0-9._:-]{1,159}$'
        AND task_ref ~ '^[0-9a-f]{64}$'
        AND source_sha ~ '^[0-9a-f]{40}$' AND target_sha ~ '^[0-9a-f]{40}$'
        AND remote_target_sha = target_sha AND source_sha <> target_sha
        AND push_merge_receipt_ref ~ '^evidence:sha256:[0-9a-f]{64}$'
        AND deployment_receipt_ref ~ '^evidence:sha256:[0-9a-f]{64}$'
        AND deployment_artifact_ref ~ '^evidence:sha256:[0-9a-f]{64}$'
        AND independent_acceptance_ref ~ '^evidence:sha256:[0-9a-f]{64}$'
        AND pg_catalog.cardinality(protected_action_approval_refs) BETWEEN 1 AND 8
        AND pg_catalog.array_position(protected_action_approval_refs, NULL) IS NULL
        AND pg_catalog.array_to_string(protected_action_approval_refs, ',')
            ~ '^evidence:sha256:[0-9a-f]{64}(,evidence:sha256:[0-9a-f]{64}){0,7}$'
        AND pg_catalog.octet_length(adoption_digest) = 32
        AND adoption_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(deployment_artifact_sha256) = 32
        AND deployment_artifact_sha256 <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(config_command_sha256) = 32
        AND config_command_sha256 <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND pg_catalog.octet_length(descriptor_digest) = 32
        AND descriptor_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        AND independent_verifier ~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$'
        AND non_force_push_merge)
);
REVOKE ALL ON TABLE control.external_verified_result_evidence FROM PUBLIC;
REVOKE ALL ON TABLE control.external_verified_result_evidence FROM lattice_runtime;
REVOKE ALL ON TABLE control.external_verified_result_evidence FROM lattice_guardian;
REVOKE ALL ON TABLE control.external_verified_result_evidence FROM lattice_readonly;

CREATE TABLE control.task_external_verified_result_adoptions (
    stream_id bytea NOT NULL,
    event_sequence numeric(20,0) NOT NULL,
    event_digest bytea NOT NULL UNIQUE,
    command_id varchar(128) NOT NULL,
    request_digest bytea NOT NULL,
    adoption_digest bytea NOT NULL UNIQUE REFERENCES control.external_verified_result_evidence,
    evidence_descriptor_digest bytea NOT NULL,
    PRIMARY KEY (stream_id, event_sequence),
    UNIQUE (stream_id, command_id),
    FOREIGN KEY (stream_id, event_sequence) REFERENCES control.task_ledger_events (stream_id, sequence),
    FOREIGN KEY (stream_id, command_id) REFERENCES control.task_ledger_commands (stream_id, command_id),
    CHECK (event_sequence = 2 AND pg_catalog.octet_length(stream_id) = 32
        AND pg_catalog.octet_length(event_digest) = 32
        AND pg_catalog.octet_length(request_digest) = 32
        AND pg_catalog.octet_length(adoption_digest) = 32
        AND pg_catalog.octet_length(evidence_descriptor_digest) = 32
        AND command_id ~ '^external-result-adoption:[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$')
);
REVOKE ALL ON TABLE control.task_external_verified_result_adoptions FROM PUBLIC;
REVOKE ALL ON TABLE control.task_external_verified_result_adoptions FROM lattice_runtime;
REVOKE ALL ON TABLE control.task_external_verified_result_adoptions FROM lattice_guardian;
REVOKE ALL ON TABLE control.task_external_verified_result_adoptions FROM lattice_readonly;

CREATE FUNCTION control.external_verified_result_evidence_read_v1(p_adoption_digest bytea)
RETURNS TABLE (
    project_id text, project_snapshot_id text, task_ref text, source_sha text, target_sha text,
    remote_target_sha text, push_merge_receipt_ref text, deployment_receipt_ref text,
    deployment_artifact_ref text, independent_acceptance_ref text,
    protected_action_approval_refs text[], deployment_artifact_sha256 bytea,
    config_command_sha256 bytea, independent_verifier text, non_force_push_merge boolean,
    descriptor_digest bytea
)
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog SET row_security = on SET lock_timeout = '5s' SET statement_timeout = '30s'
AS $external_verified_result_evidence_read_v1$
    SELECT e.project_id, e.project_snapshot_id, e.task_ref, e.source_sha, e.target_sha,
           e.remote_target_sha, e.push_merge_receipt_ref, e.deployment_receipt_ref,
           e.deployment_artifact_ref, e.independent_acceptance_ref,
           e.protected_action_approval_refs, e.deployment_artifact_sha256,
           e.config_command_sha256, e.independent_verifier, e.non_force_push_merge,
           e.descriptor_digest
      FROM ONLY control.external_verified_result_evidence AS e
     WHERE e.adoption_digest = p_adoption_digest
       AND session_user = 'lattice_runtime_login'
       AND pg_catalog.current_setting('role') = 'lattice_runtime'
$external_verified_result_evidence_read_v1$;
REVOKE ALL ON FUNCTION control.external_verified_result_evidence_read_v1(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control.external_verified_result_evidence_read_v1(bytea) TO lattice_runtime;

CREATE FUNCTION control.external_verified_result_adoption_preflight_v1(
    p_project_id text, p_project_snapshot_id text, p_task_ref text
) RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog SET row_security = on SET lock_timeout = '5s' SET statement_timeout = '30s'
AS $external_verified_result_adoption_preflight_v1$
DECLARE v_writer_status text;
BEGIN
    IF session_user <> 'lattice_runtime_login' OR pg_catalog.current_setting('role') <> 'lattice_runtime' THEN
        RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'EXTERNAL_RESULT_ADOPTION_ROLE_REJECTED';
    END IF;
    PERFORM 1 FROM ONLY control.task_submission_envelopes AS s
     WHERE s.task_ref = p_task_ref AND s.project_id = p_project_id
       AND s.project_snapshot_id = p_project_snapshot_id FOR SHARE;
    IF NOT FOUND THEN RETURN false; END IF;
    PERFORM 1 FROM ONLY foreman_execution.worker_attempts AS a
     WHERE a.task_ref = pg_catalog.decode(p_task_ref, 'hex') FOR SHARE;
    IF FOUND THEN RETURN false; END IF;
    SELECT h.current_status INTO v_writer_status
      FROM ONLY writer_lease.writer_lease_heads AS h
     WHERE h.project_id = p_project_id FOR SHARE;
    IF v_writer_status IN ('ACTIVE', 'SUSPECT') THEN RETURN false; END IF;
    RETURN true;
END
$external_verified_result_adoption_preflight_v1$;
REVOKE ALL ON FUNCTION control.external_verified_result_adoption_preflight_v1(text,text,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control.external_verified_result_adoption_preflight_v1(text,text,text) TO lattice_runtime;
