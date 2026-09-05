-- PostgreSQL owns product facts; Codex owns execution and conversation history.
-- Installed transactionally by the existing migrator after Store v8 verification.
CREATE SCHEMA control_product AUTHORIZATION lattice_migrator;
REVOKE ALL ON SCHEMA control_product FROM PUBLIC;
GRANT USAGE ON SCHEMA control_product TO lattice_runtime;

CREATE TABLE control_product.extension_identity (
    singleton boolean PRIMARY KEY CHECK (singleton),
    database_uuid uuid NOT NULL,
    store_manifest char(64) NOT NULL CHECK (store_manifest ~ '^[a-f0-9]{64}$'),
    sql_sha256 char(64) NOT NULL CHECK (sql_sha256 ~ '^[a-f0-9]{64}$')
);

CREATE TABLE control_product.work_metadata (
    task_ref char(64) PRIMARY KEY CHECK (task_ref ~ '^[a-f0-9]{64}$'),
    project_id varchar(64) NOT NULL,
    title varchar(256) NOT NULL CHECK (length(title) BETWEEN 1 AND 256),
    success_criteria varchar(8192) NOT NULL CHECK (length(success_criteria) BETWEEN 1 AND 8192),
    priority integer NOT NULL CHECK (priority BETWEEN 0 AND 3),
    parent_ref char(64),
    dependency_refs text[] NOT NULL DEFAULT '{}',
    revision bigint NOT NULL CHECK (revision > 0),
    request_id varchar(128) NOT NULL UNIQUE,
    request_digest char(64) NOT NULL CHECK (request_digest ~ '^[a-f0-9]{64}$'),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK (parent_ref IS NULL OR (parent_ref ~ '^[a-f0-9]{64}$' AND parent_ref <> task_ref)),
    CHECK (cardinality(dependency_refs) <= 64 AND array_position(dependency_refs, NULL) IS NULL
        AND NOT task_ref = ANY(dependency_refs))
);

CREATE TABLE control_product.conversation_claims (
    claim_id varchar(128) PRIMARY KEY,
    task_ref char(64) NOT NULL CHECK (task_ref ~ '^[a-f0-9]{64}$'),
    project_id varchar(64) NOT NULL,
    phase varchar(16) NOT NULL CHECK (phase IN ('EXECUTION','VERIFICATION')),
    request_digest char(64) NOT NULL CHECK (request_digest ~ '^[a-f0-9]{64}$'),
    prompt varchar(16384) NOT NULL CHECK (length(prompt) BETWEEN 1 AND 16384),
    model varchar(128) NOT NULL,
    worktree_path varchar(2048) NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (task_ref,phase)
);

CREATE TABLE control_product.conversation_observations (
    claim_id varchar(128) NOT NULL REFERENCES control_product.conversation_claims,
    sequence bigint NOT NULL CHECK (sequence > 0),
    request_id varchar(128) NOT NULL UNIQUE,
    request_digest char(64) NOT NULL CHECK (request_digest ~ '^[a-f0-9]{64}$'),
    kind varchar(32) NOT NULL CHECK (kind IN ('THREAD_BOUND','DISPATCH_STARTED','TURN_BOUND',
        'PROGRESS','APPROVAL_REQUESTED','APPROVAL_RESOLVED','TURN_COMPLETED','TURN_FAILED',
        'INTERRUPTED','ARCHIVED','REOPENED','VERIFICATION_FAILED','VERIFICATION_PASSED',
        'INPUT_QUEUED','QUESTION_REQUESTED','QUESTION_RESOLVED','CLAIM_FAILED')),
    thread_id varchar(128),
    turn_id varchar(128),
    summary varchar(16384) NOT NULL,
    evidence_ref varchar(80),
    approval_id varchar(128),
    approval_decision varchar(24),
    observed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    input_id varchar(128),
    payload jsonb,
    execution_sequence bigint,
    PRIMARY KEY (claim_id,sequence),
    CHECK (evidence_ref IS NULL OR evidence_ref ~ '^evidence:sha256:[a-f0-9]{64}$'),
    CHECK (approval_decision IS NULL OR approval_decision IN ('accept','acceptForSession','decline','cancel'))
    ,CHECK (payload IS NULL OR (jsonb_typeof(payload)='object' AND octet_length(payload::text)<=16384))
);

CREATE TABLE control_product.decisions (
    decision_id varchar(128) PRIMARY KEY,
    project_id varchar(64) NOT NULL,
    task_ref char(64),
    subject varchar(512) NOT NULL,
    content varchar(8192) NOT NULL,
    reason varchar(8192) NOT NULL,
    source varchar(24) NOT NULL CHECK (source IN ('USER','VERIFIED_AI','HERMES_SUGGESTION','user_confirmation','approved_document')),
    supersedes_id varchar(128) REFERENCES control_product.decisions,
    request_digest char(64) NOT NULL CHECK (request_digest ~ '^[a-f0-9]{64}$'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    source_reference varchar(512) NOT NULL,
    client_request_id varchar(128) NOT NULL UNIQUE,
    decision_sequence bigint NOT NULL UNIQUE CHECK (decision_sequence BETWEEN 1 AND 10000),
    CHECK (task_ref IS NULL OR task_ref ~ '^[a-f0-9]{64}$'),
    CHECK (length(subject)>0 AND length(content)>0 AND length(reason)>0)
);

-- A singleton decision head supplies the existing integer revision/digest contract.
CREATE TABLE control_product.decision_state (
    singleton boolean PRIMARY KEY CHECK (singleton),
    revision bigint NOT NULL CHECK (revision BETWEEN 0 AND 10000),
    digest char(64) NOT NULL CHECK (digest ~ '^[a-f0-9]{64}$')
);
INSERT INTO control_product.decision_state VALUES(true,0,encode(sha256(convert_to('[]','UTF8')),'hex'));
CREATE UNIQUE INDEX decision_confirmed_root_v1 ON control_product.decisions(project_id,subject)
    WHERE supersedes_id IS NULL AND source IN('user_confirmation','approved_document');
CREATE UNIQUE INDEX decision_confirmed_child_v1 ON control_product.decisions(supersedes_id)
    WHERE supersedes_id IS NOT NULL;

-- Trusted verification ingress owns these immutable bytes. Runtime can only read.
CREATE TABLE control_product.local_verified_result_evidence (
    adoption_digest bytea PRIMARY KEY CHECK (octet_length(adoption_digest)=32),
    project_id varchar(64) NOT NULL,
    project_snapshot_id varchar(159) NOT NULL,
    task_ref char(64) NOT NULL CHECK (task_ref ~ '^[a-f0-9]{64}$'),
    client_request_id varchar(64) NOT NULL,
    expected_head_digest bytea NOT NULL CHECK (octet_length(expected_head_digest)=32),
    artifact_ref varchar(80) NOT NULL CHECK (artifact_ref ~ '^evidence:sha256:[a-f0-9]{64}$'),
    acceptance_ref varchar(80) NOT NULL CHECK (acceptance_ref ~ '^evidence:sha256:[a-f0-9]{64}$'),
    artifact_sha256 bytea NOT NULL CHECK (octet_length(artifact_sha256)=32),
    acceptance_sha256 bytea NOT NULL CHECK (octet_length(acceptance_sha256)=32),
    independent_verifier varchar(128) NOT NULL,
    runner_profile varchar(64) NOT NULL CHECK (runner_profile='NODE_TEST_V1'),
    descriptor_digest bytea NOT NULL UNIQUE CHECK (octet_length(descriptor_digest)=32)
);

CREATE TABLE control_product.local_result_bindings (
    stream_id bytea PRIMARY KEY CHECK (octet_length(stream_id)=32),
    event_digest bytea NOT NULL UNIQUE CHECK (octet_length(event_digest)=32),
    command_id varchar(128) NOT NULL,
    request_digest bytea NOT NULL CHECK (octet_length(request_digest)=32),
    adoption_digest bytea NOT NULL UNIQUE REFERENCES control_product.local_verified_result_evidence,
    descriptor_digest bytea NOT NULL CHECK (octet_length(descriptor_digest)=32)
);

REVOKE ALL ON ALL TABLES IN SCHEMA control_product FROM PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly;

CREATE FUNCTION control_product.identity_read_v1()
RETURNS TABLE (database_uuid text,store_manifest text,sql_sha256 text,current_database_uuid text,current_store_manifest text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog
AS $identity$
 SELECT i.database_uuid::text,i.store_manifest::text,i.sql_sha256::text,d.database_uuid::text,c.manifest_sha256::text
 FROM ONLY control_product.extension_identity i CROSS JOIN ONLY control.database_identity d
 CROSS JOIN ONLY control.schema_compatibility c
 WHERE i.singleton AND d.singleton AND c.singleton AND c.current_schema_version=8
$identity$;

CREATE FUNCTION control_product.task_refs_v1(p_project_id text,p_after text,p_limit integer)
RETURNS TABLE(task_ref text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog
AS $refs$
 SELECT s.task_ref::text FROM ONLY control.task_submission_envelopes s
 WHERE s.project_id=p_project_id AND s.task_ref>p_after
   AND p_limit BETWEEN 1 AND 256
   AND session_user='lattice_runtime_login' AND current_setting('role')='lattice_runtime'
 ORDER BY s.task_ref LIMIT LEAST(GREATEST(p_limit,0),256)
$refs$;

CREATE FUNCTION control_product.snapshot_v1(p_project_id text,p_task_refs text[])
RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog
AS $snapshot$
 SELECT jsonb_build_object(
   'metadata',COALESCE((SELECT jsonb_agg(to_jsonb(m) ORDER BY m.task_ref) FROM ONLY control_product.work_metadata m WHERE m.project_id=p_project_id AND m.task_ref=ANY(p_task_refs)),'[]'::jsonb),
   'claims',COALESCE((SELECT jsonb_agg(to_jsonb(c)||jsonb_build_object(
       'last_sequence',COALESCE((SELECT max(sequence) FROM control_product.conversation_observations x WHERE x.claim_id=c.claim_id),0),
       'thread_id',(SELECT thread_id FROM control_product.conversation_observations x WHERE x.claim_id=c.claim_id AND x.kind='THREAD_BOUND' ORDER BY sequence LIMIT 1),
       'turn_id',(SELECT turn_id FROM control_product.conversation_observations x WHERE x.claim_id=c.claim_id AND x.kind='TURN_BOUND' AND x.sequence>COALESCE((SELECT max(d.sequence) FROM control_product.conversation_observations d WHERE d.claim_id=c.claim_id AND d.kind='DISPATCH_STARTED'),0) ORDER BY sequence DESC LIMIT 1),
       'execution_sequence',(SELECT execution_sequence FROM control_product.conversation_observations x WHERE x.claim_id=c.claim_id AND x.kind='DISPATCH_STARTED' ORDER BY sequence DESC LIMIT 1),
       'input_id',(SELECT input_id FROM control_product.conversation_observations x WHERE x.claim_id=c.claim_id AND x.kind='DISPATCH_STARTED' ORDER BY sequence DESC LIMIT 1),
       'dispatch_started',EXISTS(SELECT 1 FROM control_product.conversation_observations x WHERE x.claim_id=c.claim_id AND x.kind='DISPATCH_STARTED'),
       'dispatch_sequence',(SELECT max(sequence) FROM control_product.conversation_observations x WHERE x.claim_id=c.claim_id AND x.kind='DISPATCH_STARTED'),
       'repair_attempts',(SELECT count(*) FROM control_product.conversation_observations x WHERE x.claim_id=c.claim_id AND x.kind='INPUT_QUEUED' AND x.input_id LIKE 'repair:%'),
       'verification_outcome',(SELECT to_jsonb(x) FROM control_product.conversation_observations x
         WHERE x.claim_id=c.claim_id AND x.kind IN('VERIFICATION_PASSED','VERIFICATION_FAILED')
           AND x.turn_id=(SELECT b.turn_id FROM control_product.conversation_observations b
             WHERE b.claim_id=c.claim_id AND b.kind='TURN_BOUND'
               AND b.sequence>COALESCE((SELECT max(d.sequence) FROM control_product.conversation_observations d WHERE d.claim_id=c.claim_id AND d.kind='DISPATCH_STARTED'),0)
             ORDER BY b.sequence DESC LIMIT 1)
         ORDER BY x.sequence DESC LIMIT 1),
       'turn_status',(SELECT kind FROM control_product.conversation_observations x WHERE x.claim_id=c.claim_id AND x.kind IN('DISPATCH_STARTED','TURN_BOUND','TURN_COMPLETED','TURN_FAILED','INTERRUPTED','CLAIM_FAILED') ORDER BY sequence DESC LIMIT 1),
       'archived',COALESCE((SELECT kind='ARCHIVED' FROM control_product.conversation_observations x WHERE x.claim_id=c.claim_id AND x.kind IN('ARCHIVED','REOPENED') ORDER BY sequence DESC LIMIT 1),false),
       'pending_inputs',COALESCE((SELECT jsonb_agg(to_jsonb(x) ORDER BY x.sequence) FROM control_product.conversation_observations x WHERE x.claim_id=c.claim_id AND x.kind='INPUT_QUEUED' AND NOT EXISTS(SELECT 1 FROM control_product.conversation_observations sent WHERE sent.claim_id=c.claim_id AND sent.kind='DISPATCH_STARTED' AND sent.input_id=x.input_id)),'[]'::jsonb),
       'pending_questions',COALESCE((SELECT jsonb_agg(to_jsonb(x) ORDER BY x.sequence) FROM control_product.conversation_observations x WHERE x.claim_id=c.claim_id AND x.kind IN('APPROVAL_REQUESTED','QUESTION_REQUESTED')
           AND NOT EXISTS(SELECT 1 FROM control_product.conversation_observations done WHERE done.claim_id=c.claim_id AND ((done.approval_id=x.approval_id AND done.kind IN('APPROVAL_RESOLVED','QUESTION_RESOLVED')) OR (done.turn_id=x.turn_id AND done.kind IN('TURN_COMPLETED','TURN_FAILED','INTERRUPTED'))))),'[]'::jsonb)
     ) ORDER BY c.claim_id) FROM ONLY control_product.conversation_claims c WHERE c.project_id=p_project_id AND c.task_ref=ANY(p_task_refs)),'[]'::jsonb),
   'observations',COALESCE((SELECT jsonb_agg(to_jsonb(o) ORDER BY o.claim_id,o.sequence) FROM ONLY control_product.conversation_claims c CROSS JOIN LATERAL (
       SELECT x.* FROM ONLY control_product.conversation_observations x WHERE x.claim_id=c.claim_id ORDER BY x.sequence DESC LIMIT 100
     ) o WHERE c.project_id=p_project_id AND c.task_ref=ANY(p_task_refs)),'[]'::jsonb),
   'decisions',COALESCE((SELECT jsonb_agg(to_jsonb(d) ORDER BY d.created_at,d.decision_id) FROM (
       SELECT x.* FROM ONLY control_product.decisions x WHERE x.project_id=p_project_id ORDER BY x.created_at DESC,x.decision_id DESC LIMIT 256
     ) d),'[]'::jsonb))
 WHERE cardinality(p_task_refs)<=256 AND session_user='lattice_runtime_login' AND current_setting('role')='lattice_runtime'
$snapshot$;

CREATE FUNCTION control_product.metadata_write_v1(p_task_ref text,p_request_id text,p_digest text,p_expected bigint,p_title text,p_criteria text,p_priority integer,p_parent text,p_dependencies text[])
RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog SET lock_timeout='5s' SET statement_timeout='30s'
AS $metadata$
DECLARE v_project text; v_existing control_product.work_metadata%ROWTYPE;
BEGIN
 IF session_user<>'lattice_runtime_login' OR current_setting('role')<>'lattice_runtime' OR current_setting('transaction_isolation')<>'serializable' OR current_setting('transaction_read_only')::boolean THEN RAISE EXCEPTION 'CONTROL_PRODUCT_ROLE_REJECTED'; END IF;
 SELECT s.project_id INTO v_project FROM ONLY control.task_submission_envelopes s WHERE s.task_ref=p_task_ref FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'CONTROL_PRODUCT_TASK_MISSING'; END IF;
 SELECT * INTO v_existing FROM ONLY control_product.work_metadata WHERE task_ref=p_task_ref FOR UPDATE;
 IF FOUND AND v_existing.request_id=p_request_id THEN
   IF v_existing.request_digest<>p_digest THEN RAISE EXCEPTION 'CONTROL_PRODUCT_IDEMPOTENCY_CONFLICT'; END IF;
   RETURN to_jsonb(v_existing);
 END IF;
 IF COALESCE(v_existing.revision,0)<>p_expected THEN RAISE EXCEPTION 'CONTROL_PRODUCT_REVISION_CONFLICT'; END IF;
 IF EXISTS(SELECT 1 FROM ONLY control_product.conversation_claims WHERE task_ref=p_task_ref)
   AND v_existing.success_criteria IS DISTINCT FROM p_criteria THEN RAISE EXCEPTION 'CONTROL_PRODUCT_ACCEPTANCE_ALREADY_STARTED'; END IF;
 IF p_parent=p_task_ref OR p_task_ref=ANY(p_dependencies) OR cardinality(p_dependencies)>64 OR cardinality(p_dependencies)<>(SELECT count(DISTINCT x) FROM unnest(p_dependencies) x)
   OR EXISTS(SELECT 1 FROM unnest(array_append(p_dependencies,p_parent)) x WHERE x IS NOT NULL AND NOT EXISTS(SELECT 1 FROM ONLY control.task_submission_envelopes s WHERE s.task_ref=x AND s.project_id=v_project))
 THEN RAISE EXCEPTION 'CONTROL_PRODUCT_RELATION_REJECTED'; END IF;
 -- Parent and dependency graphs are independently acyclic.
 IF EXISTS(WITH RECURSIVE ancestors(ref) AS (
   SELECT p_parent WHERE p_parent IS NOT NULL UNION SELECT m.parent_ref FROM ancestors a JOIN control_product.work_metadata m ON m.task_ref=a.ref WHERE m.parent_ref IS NOT NULL
 ) SELECT 1 FROM ancestors WHERE ref=p_task_ref)
 OR EXISTS(WITH RECURSIVE dependencies(ref) AS (
   SELECT unnest(p_dependencies) UNION SELECT unnest(m.dependency_refs) FROM dependencies d JOIN control_product.work_metadata m ON m.task_ref=d.ref
 ) SELECT 1 FROM dependencies WHERE ref=p_task_ref) THEN RAISE EXCEPTION 'CONTROL_PRODUCT_RELATION_CYCLE'; END IF;
 INSERT INTO control_product.work_metadata(task_ref,project_id,title,success_criteria,priority,parent_ref,dependency_refs,revision,request_id,request_digest)
 VALUES(p_task_ref,v_project,p_title,p_criteria,p_priority,p_parent,p_dependencies,p_expected+1,p_request_id,p_digest)
 ON CONFLICT(task_ref) DO UPDATE SET title=EXCLUDED.title,success_criteria=EXCLUDED.success_criteria,priority=EXCLUDED.priority,parent_ref=EXCLUDED.parent_ref,dependency_refs=EXCLUDED.dependency_refs,revision=EXCLUDED.revision,request_id=EXCLUDED.request_id,request_digest=EXCLUDED.request_digest,updated_at=clock_timestamp()
 RETURNING * INTO v_existing;
 RETURN to_jsonb(v_existing);
END
$metadata$;

CREATE FUNCTION control_product.claim_v1(p_task_ref text,p_claim_id text,p_digest text,p_phase text,p_prompt text,p_model text,p_worktree text)
RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog SET lock_timeout='5s' SET statement_timeout='30s'
AS $claim$
DECLARE v_project text; v_existing control_product.conversation_claims%ROWTYPE;
BEGIN
 IF session_user<>'lattice_runtime_login' OR current_setting('role')<>'lattice_runtime' OR current_setting('transaction_isolation')<>'serializable' OR current_setting('transaction_read_only')::boolean THEN RAISE EXCEPTION 'CONTROL_PRODUCT_ROLE_REJECTED'; END IF;
 SELECT s.project_id INTO v_project FROM ONLY control.task_submission_envelopes s WHERE s.task_ref=p_task_ref FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'CONTROL_PRODUCT_TASK_MISSING'; END IF;
 SELECT * INTO v_existing FROM ONLY control_product.conversation_claims WHERE task_ref=p_task_ref AND phase=p_phase FOR UPDATE;
 IF FOUND THEN
   IF v_existing.claim_id<>p_claim_id OR v_existing.request_digest<>p_digest THEN RAISE EXCEPTION 'CONTROL_PRODUCT_EXECUTION_ALREADY_CLAIMED'; END IF;
   RETURN to_jsonb(v_existing);
 END IF;
 IF octet_length(p_prompt)>16384 OR NOT EXISTS(SELECT 1 FROM ONLY control_product.work_metadata WHERE task_ref=p_task_ref)
   OR EXISTS(SELECT 1 FROM ONLY foreman_execution.worker_attempts WHERE task_ref=decode(p_task_ref,'hex'))
   OR NOT EXISTS(SELECT 1 FROM ONLY control.task_submission_envelopes e JOIN ONLY control.task_ledger_streams s ON s.stream_id=e.stream_id
       WHERE e.task_ref=p_task_ref AND s.sequence=1 AND s.task_subject_kind='GENERAL_TASK_INTAKE')
 THEN RAISE EXCEPTION 'CONTROL_PRODUCT_EXECUTION_REJECTED'; END IF;
 IF p_phase='VERIFICATION' AND NOT EXISTS(
     SELECT 1 FROM control_product.conversation_claims c JOIN control_product.conversation_observations o ON o.claim_id=c.claim_id
     WHERE c.task_ref=p_task_ref AND c.phase='EXECUTION' AND o.kind='TURN_COMPLETED'
       AND NOT EXISTS(SELECT 1 FROM control_product.conversation_observations later WHERE later.claim_id=c.claim_id AND later.sequence>o.sequence AND later.kind='DISPATCH_STARTED'))
 THEN RAISE EXCEPTION 'CONTROL_PRODUCT_EXECUTION_NOT_FINISHED'; END IF;
 IF p_phase='EXECUTION' AND EXISTS(
     SELECT 1 FROM control_product.conversation_claims c WHERE c.project_id=v_project AND c.task_ref<>p_task_ref AND c.phase='EXECUTION'
       AND NOT EXISTS(SELECT 1 FROM control_product.conversation_observations done WHERE done.claim_id=c.claim_id AND done.kind IN('TURN_COMPLETED','TURN_FAILED','INTERRUPTED','CLAIM_FAILED')
         AND NOT EXISTS(SELECT 1 FROM control_product.conversation_observations later WHERE later.claim_id=c.claim_id AND later.sequence>done.sequence AND later.kind='DISPATCH_STARTED')))
 THEN RAISE EXCEPTION 'CONTROL_PRODUCT_PROJECT_ALREADY_EXECUTING'; END IF;
 INSERT INTO control_product.conversation_claims(claim_id,task_ref,project_id,phase,request_digest,prompt,model,worktree_path)
 VALUES(p_claim_id,p_task_ref,v_project,p_phase,p_digest,p_prompt,p_model,p_worktree) RETURNING * INTO v_existing;
 RETURN to_jsonb(v_existing);
END
$claim$;

CREATE FUNCTION control_product.observe_v1(p_claim_id text,p_request_id text,p_digest text,p_expected bigint,p_kind text,p_thread text,p_turn text,p_summary text,p_evidence text,p_approval text,p_decision text,p_input text DEFAULT NULL,p_payload jsonb DEFAULT NULL)
RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog SET lock_timeout='5s' SET statement_timeout='30s'
AS $observe$
DECLARE v_existing control_product.conversation_observations%ROWTYPE; v_claim control_product.conversation_claims%ROWTYPE;
 v_sequence bigint; v_thread text; v_turn text; v_input text; v_dispatch bigint; v_terminal boolean; v_archived boolean;
 v_request control_product.conversation_observations%ROWTYPE;
 v_execution_sequence bigint; v_execution_claim text;
BEGIN
 IF session_user<>'lattice_runtime_login' OR current_setting('role')<>'lattice_runtime' OR current_setting('transaction_isolation')<>'serializable' OR current_setting('transaction_read_only')::boolean THEN RAISE EXCEPTION 'CONTROL_PRODUCT_ROLE_REJECTED'; END IF;
 SELECT * INTO v_claim FROM ONLY control_product.conversation_claims WHERE claim_id=p_claim_id;
 IF NOT FOUND THEN RAISE EXCEPTION 'CONTROL_PRODUCT_CLAIM_MISSING'; END IF;
 PERFORM 1 FROM ONLY control.task_submission_envelopes WHERE task_ref=v_claim.task_ref FOR UPDATE;
 PERFORM 1 FROM ONLY control_product.conversation_claims WHERE claim_id=p_claim_id FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'CONTROL_PRODUCT_CLAIM_MISSING'; END IF;
 SELECT * INTO v_existing FROM ONLY control_product.conversation_observations WHERE request_id=p_request_id;
 IF FOUND THEN
   IF v_existing.claim_id<>p_claim_id OR v_existing.request_digest<>p_digest THEN RAISE EXCEPTION 'CONTROL_PRODUCT_IDEMPOTENCY_CONFLICT'; END IF;
   RETURN to_jsonb(v_existing);
 END IF;
 SELECT COALESCE(max(sequence),0) INTO v_sequence FROM ONLY control_product.conversation_observations WHERE claim_id=p_claim_id;
 IF v_sequence<>p_expected THEN RAISE EXCEPTION 'CONTROL_PRODUCT_REVISION_CONFLICT'; END IF;
 IF octet_length(p_summary)>16384 THEN RAISE EXCEPTION 'CONTROL_PRODUCT_TEXT_LIMIT_EXCEEDED'; END IF;
 SELECT thread_id INTO v_thread FROM ONLY control_product.conversation_observations WHERE claim_id=p_claim_id AND kind='THREAD_BOUND' ORDER BY sequence LIMIT 1;
 IF (p_kind='THREAD_BOUND' AND (p_thread IS NULL OR v_thread IS NOT NULL OR p_turn IS NOT NULL))
   OR (p_kind NOT IN('THREAD_BOUND','CLAIM_FAILED') AND (v_thread IS NULL OR p_thread IS DISTINCT FROM v_thread))
 THEN RAISE EXCEPTION 'CONTROL_PRODUCT_THREAD_MISMATCH'; END IF;
 SELECT sequence,input_id INTO v_dispatch,v_input FROM ONLY control_product.conversation_observations
   WHERE claim_id=p_claim_id AND kind='DISPATCH_STARTED' ORDER BY sequence DESC LIMIT 1;
 SELECT turn_id INTO v_turn FROM ONLY control_product.conversation_observations
   WHERE claim_id=p_claim_id AND kind='TURN_BOUND' AND sequence>COALESCE(v_dispatch,0) ORDER BY sequence DESC LIMIT 1;
 SELECT EXISTS(SELECT 1 FROM ONLY control_product.conversation_observations WHERE claim_id=p_claim_id
   AND sequence>COALESCE(v_dispatch,0) AND kind IN('TURN_COMPLETED','TURN_FAILED','INTERRUPTED')) INTO v_terminal;
 SELECT COALESCE((SELECT kind='ARCHIVED' FROM ONLY control_product.conversation_observations
   WHERE claim_id=p_claim_id AND kind IN('ARCHIVED','REOPENED') ORDER BY sequence DESC LIMIT 1),false) INTO v_archived;
 IF p_kind='CLAIM_FAILED' THEN
   IF v_thread IS NOT NULL OR p_thread IS NOT NULL OR p_turn IS NOT NULL OR v_dispatch IS NOT NULL THEN RAISE EXCEPTION 'CONTROL_PRODUCT_CLAIM_FAILURE_REJECTED'; END IF;
 ELSIF p_kind='INPUT_QUEUED' THEN
   IF p_input IS NULL OR p_input=p_claim_id OR p_turn IS NOT NULL OR length(p_summary)=0
     OR EXISTS(SELECT 1 FROM control_product.conversation_observations WHERE claim_id=p_claim_id AND kind='INPUT_QUEUED' AND input_id=p_input)
     OR (SELECT count(*) FROM control_product.conversation_observations q WHERE q.claim_id=p_claim_id AND q.kind='INPUT_QUEUED'
         AND NOT EXISTS(SELECT 1 FROM control_product.conversation_observations d WHERE d.claim_id=p_claim_id AND d.kind='DISPATCH_STARTED' AND d.input_id=q.input_id))>=4
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_INPUT_REJECTED'; END IF;
 ELSIF p_kind='DISPATCH_STARTED' THEN
   IF v_archived OR p_turn IS NOT NULL OR p_input IS NULL OR (v_dispatch IS NOT NULL AND NOT v_terminal)
     OR (p_input<>p_claim_id AND NOT EXISTS(SELECT 1 FROM control_product.conversation_observations WHERE claim_id=p_claim_id AND kind='INPUT_QUEUED' AND input_id=p_input))
     OR EXISTS(SELECT 1 FROM control_product.conversation_observations WHERE claim_id=p_claim_id AND kind='DISPATCH_STARTED' AND input_id=p_input)
     OR NOT EXISTS(SELECT 1 FROM control.task_submission_envelopes e JOIN control.task_ledger_streams s ON s.stream_id=e.stream_id WHERE e.task_ref=v_claim.task_ref AND s.sequence=1)
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_DISPATCH_REJECTED'; END IF;
   IF v_claim.phase='EXECUTION' AND EXISTS(
     SELECT 1 FROM control_product.conversation_claims c WHERE c.project_id=v_claim.project_id AND c.task_ref<>v_claim.task_ref AND c.phase='EXECUTION'
       AND NOT EXISTS(SELECT 1 FROM control_product.conversation_observations done WHERE done.claim_id=c.claim_id AND done.kind IN('TURN_COMPLETED','TURN_FAILED','INTERRUPTED','CLAIM_FAILED')
         AND NOT EXISTS(SELECT 1 FROM control_product.conversation_observations later WHERE later.claim_id=c.claim_id AND later.sequence>done.sequence AND later.kind='DISPATCH_STARTED')))
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_PROJECT_ALREADY_EXECUTING'; END IF;
   IF EXISTS(SELECT 1 FROM control_product.work_metadata m CROSS JOIN LATERAL unnest(m.dependency_refs) d(ref)
       WHERE m.task_ref=v_claim.task_ref AND NOT EXISTS(
         SELECT 1 FROM control.task_submission_envelopes e JOIN control.task_ledger_streams s ON s.stream_id=e.stream_id WHERE e.task_ref=d.ref AND s.sequence=2))
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_DEPENDENCY_NOT_COMPLETED'; END IF;
   IF v_claim.phase='VERIFICATION' THEN
     SELECT claim_id INTO v_execution_claim FROM control_product.conversation_claims WHERE task_ref=v_claim.task_ref AND phase='EXECUTION';
     SELECT max(sequence) INTO v_execution_sequence FROM control_product.conversation_observations WHERE claim_id=v_execution_claim AND kind='DISPATCH_STARTED';
     IF v_execution_sequence IS NULL OR NOT EXISTS(SELECT 1 FROM control_product.conversation_observations WHERE claim_id=v_execution_claim AND sequence>v_execution_sequence AND kind='TURN_COMPLETED')
     THEN RAISE EXCEPTION 'CONTROL_PRODUCT_EXECUTION_NOT_FINISHED'; END IF;
   END IF;
 ELSIF p_kind='TURN_BOUND' THEN
   IF v_dispatch IS NULL OR v_turn IS NOT NULL OR p_turn IS NULL OR p_input IS DISTINCT FROM v_input
     OR EXISTS(SELECT 1 FROM control_product.conversation_observations WHERE claim_id=p_claim_id AND kind='TURN_BOUND' AND turn_id=p_turn)
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_TURN_MISMATCH'; END IF;
 ELSIF p_kind IN('ARCHIVED','REOPENED') THEN
   IF p_turn IS DISTINCT FROM v_turn OR (p_kind='ARCHIVED' AND v_archived)
     OR (p_kind='REOPENED' AND NOT v_archived)
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_ARCHIVE_REJECTED'; END IF;
 ELSIF p_kind IN('VERIFICATION_PASSED','VERIFICATION_FAILED') THEN
   IF v_claim.phase<>'VERIFICATION' OR NOT v_terminal OR p_turn IS DISTINCT FROM v_turn
     OR p_input IS DISTINCT FROM v_input OR (p_kind='VERIFICATION_PASSED' AND NOT EXISTS(
       SELECT 1 FROM control_product.conversation_observations WHERE claim_id=p_claim_id AND turn_id=v_turn AND kind='TURN_COMPLETED'))
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_VERIFICATION_REJECTED'; END IF;
   SELECT claim_id INTO v_execution_claim FROM control_product.conversation_claims WHERE task_ref=v_claim.task_ref AND phase='EXECUTION';
   SELECT execution_sequence INTO v_execution_sequence FROM control_product.conversation_observations WHERE claim_id=p_claim_id AND sequence=v_dispatch;
   IF v_execution_sequence IS NULL OR v_execution_sequence IS DISTINCT FROM (SELECT max(sequence) FROM control_product.conversation_observations WHERE claim_id=v_execution_claim AND kind='DISPATCH_STARTED')
     OR NOT EXISTS(SELECT 1 FROM control_product.conversation_observations WHERE claim_id=v_execution_claim AND sequence>v_execution_sequence AND kind='TURN_COMPLETED')
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_VERIFICATION_STALE'; END IF;
 ELSIF p_kind<>'THREAD_BOUND' THEN
   IF v_turn IS NULL OR v_terminal OR p_turn IS DISTINCT FROM v_turn OR p_input IS DISTINCT FROM v_input
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_TURN_MISMATCH'; END IF;
 END IF;
 IF p_kind IN('APPROVAL_REQUESTED','QUESTION_REQUESTED') THEN
   IF p_approval IS NULL OR length(p_approval)=0 OR p_payload IS NULL OR p_decision IS NOT NULL
     OR EXISTS(SELECT 1 FROM control_product.conversation_observations WHERE claim_id=p_claim_id AND approval_id=p_approval)
     OR (SELECT count(*) FROM control_product.conversation_observations q WHERE q.claim_id=p_claim_id AND q.turn_id=p_turn AND q.kind IN('APPROVAL_REQUESTED','QUESTION_REQUESTED')
       AND NOT EXISTS(SELECT 1 FROM control_product.conversation_observations r WHERE r.claim_id=p_claim_id AND r.approval_id=q.approval_id AND r.kind IN('APPROVAL_RESOLVED','QUESTION_RESOLVED')))>=2
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_QUESTION_REJECTED'; END IF;
 ELSIF p_kind IN('APPROVAL_RESOLVED','QUESTION_RESOLVED') THEN
   SELECT * INTO v_request FROM control_product.conversation_observations WHERE claim_id=p_claim_id AND approval_id=p_approval
     AND kind=CASE WHEN p_kind='APPROVAL_RESOLVED' THEN 'APPROVAL_REQUESTED' ELSE 'QUESTION_REQUESTED' END;
   IF NOT FOUND OR v_request.turn_id IS DISTINCT FROM p_turn OR v_request.input_id IS DISTINCT FROM p_input
     OR EXISTS(SELECT 1 FROM control_product.conversation_observations WHERE claim_id=p_claim_id AND approval_id=p_approval AND kind IN('APPROVAL_RESOLVED','QUESTION_RESOLVED'))
     OR (p_kind='APPROVAL_RESOLVED' AND p_decision IS NULL)
     OR (p_kind='QUESTION_RESOLVED' AND (p_payload IS NULL OR p_decision IS NOT NULL))
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_QUESTION_REJECTED'; END IF;
 ELSIF p_approval IS NOT NULL OR p_decision IS NOT NULL OR p_payload IS NOT NULL THEN
   RAISE EXCEPTION 'CONTROL_PRODUCT_UNEXPECTED_QUESTION';
 END IF;
 INSERT INTO control_product.conversation_observations(claim_id,sequence,request_id,request_digest,kind,thread_id,turn_id,summary,evidence_ref,approval_id,approval_decision,input_id,payload,execution_sequence)
 VALUES(p_claim_id,p_expected+1,p_request_id,p_digest,p_kind,p_thread,p_turn,p_summary,p_evidence,p_approval,p_decision,p_input,p_payload,CASE WHEN p_kind='DISPATCH_STARTED' AND v_claim.phase='VERIFICATION' THEN v_execution_sequence ELSE NULL END) RETURNING * INTO v_existing;
 RETURN to_jsonb(v_existing);
END
$observe$;

-- Full decision reads use the same durable head as writes. Task previews do not
-- participate in this contract and can remain clipped independently.
CREATE FUNCTION control_product.decision_row_v1(p_id text)
RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog
AS $decision_row$
 SELECT jsonb_build_object('id',d.decision_id,'scope',d.project_id,'subject',d.subject,
   'content',d.content,'rationale',d.reason,
   'source',jsonb_build_object('kind',d.source,'reference',d.source_reference),
   'status',CASE WHEN EXISTS(SELECT 1 FROM ONLY control_product.decisions child WHERE child.supersedes_id=d.decision_id) THEN 'superseded' ELSE 'current' END,
   'supersedes_decision_id',d.supersedes_id,
   'created_at',to_char(d.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.MS"Z"'))
 FROM ONLY control_product.decisions d WHERE d.decision_id=p_id
   AND d.source IN('user_confirmation','approved_document')
$decision_row$;

CREATE FUNCTION control_product.decision_snapshot_v1(
 p_mode text,p_scope text,p_id text,p_subject text,p_query text,p_limit integer,p_depth integer,p_revision bigint,p_digest text)
RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog
SET statement_timeout='30s'
AS $decision_snapshot$
DECLARE v_head control_product.decision_state%ROWTYPE; v_target control_product.decisions%ROWTYPE;
 v_base jsonb; v_result jsonb; v_rows jsonb; v_count bigint; v_index bigint; v_start bigint; v_bound integer; v_length integer;
BEGIN
 IF session_user<>'lattice_runtime_login' OR current_setting('role')<>'lattice_runtime'
 THEN RAISE EXCEPTION 'CONTROL_PRODUCT_ROLE_REJECTED'; END IF;
 SELECT * INTO STRICT v_head FROM ONLY control_product.decision_state WHERE singleton;
 v_base:=jsonb_build_object('source',jsonb_build_object('kind','POSTGRESQL_CONTROL_PRODUCT','authority','POSTGRESQL_TASK_LEDGER'),
   'revision',v_head.revision,'digest',v_head.digest);
 IF p_mode='current' THEN
   IF p_scope IS NULL OR p_id IS NOT NULL OR p_query IS NOT NULL OR p_depth IS NOT NULL
     OR p_revision IS NOT NULL OR p_digest IS NOT NULL OR p_limit IS NULL OR p_limit NOT BETWEEN 1 AND 32
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_INPUT_REJECTED'; END IF;
 ELSIF p_mode='read' THEN
   IF p_id IS NULL OR p_scope IS NOT NULL OR p_subject IS NOT NULL OR p_query IS NOT NULL OR p_limit IS NOT NULL
     OR p_depth IS NULL OR p_depth NOT BETWEEN 1 AND 64 OR p_revision IS NULL OR p_digest IS NULL
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_INPUT_REJECTED'; END IF;
 ELSIF p_mode='search' THEN
   IF p_scope IS NULL OR p_id IS NOT NULL OR p_subject IS NOT NULL OR p_depth IS NOT NULL
     OR p_query IS NULL OR length(btrim(p_query))=0 OR octet_length(p_query)>128
     OR p_limit IS NULL OR p_limit NOT BETWEEN 1 AND 20 OR p_revision IS NULL OR p_digest IS NULL
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_INPUT_REJECTED'; END IF;
 ELSE RAISE EXCEPTION 'CONTROL_PRODUCT_INPUT_REJECTED'; END IF;
 IF p_mode<>'current' AND (p_revision IS DISTINCT FROM v_head.revision OR p_digest IS DISTINCT FROM v_head.digest)
 THEN RAISE EXCEPTION 'DECISION_REVISION_MISMATCH'; END IF;
 IF p_scope IS NOT NULL AND NOT EXISTS(SELECT 1 FROM ONLY control.project_registry_projects WHERE project_id=p_scope)
 THEN RAISE EXCEPTION 'CONTROL_PRODUCT_DECISION_SCOPE_REJECTED'; END IF;
 IF p_mode='current' THEN
   SELECT COALESCE(jsonb_agg(control_product.decision_row_v1(x.decision_id) ORDER BY x.subject COLLATE "C",x.decision_id COLLATE "C"),'[]'::jsonb),count(*) INTO v_rows,v_count
   FROM (SELECT d.decision_id,d.subject FROM ONLY control_product.decisions d
     WHERE d.project_id=p_scope AND d.source IN('user_confirmation','approved_document')
       AND (p_subject IS NULL OR d.subject=p_subject)
       AND NOT EXISTS(SELECT 1 FROM ONLY control_product.decisions child WHERE child.supersedes_id=d.decision_id)
     ORDER BY d.subject COLLATE "C",d.decision_id COLLATE "C" LIMIT p_limit+1) x;
   IF v_count>p_limit THEN v_rows:=v_rows-p_limit; END IF;
   v_result:=v_base||jsonb_build_object('schema_version','lattice.control.current-decisions-packet.v1',
     'scope',p_scope,'subject',p_subject,'decisions',v_rows,'truncated',v_count>p_limit);
   v_bound:=262144;
 ELSIF p_mode='search' THEN
   SELECT COALESCE(jsonb_agg(control_product.decision_row_v1(x.decision_id) ORDER BY x.created_at DESC,x.decision_id COLLATE "C" DESC),'[]'::jsonb),count(*) INTO v_rows,v_count
   FROM (SELECT d.decision_id,d.created_at FROM ONLY control_product.decisions d
     WHERE d.project_id=p_scope AND d.source IN('user_confirmation','approved_document')
       AND (strpos(lower(d.subject),lower(p_query))>0 OR strpos(lower(d.content),lower(p_query))>0
         OR strpos(lower(d.reason),lower(p_query))>0 OR strpos(lower(d.source_reference),lower(p_query))>0)
     ORDER BY d.created_at DESC,d.decision_id COLLATE "C" DESC LIMIT p_limit+1) x;
   IF v_count>p_limit THEN v_rows:=v_rows-p_limit; END IF;
   v_result:=v_base||jsonb_build_object('schema_version','lattice.control.decision-search.v1',
     'scope',p_scope,'query',p_query,'decisions',v_rows,'truncated',v_count>p_limit);
   v_bound:=196608;
 ELSE
   SELECT * INTO v_target FROM ONLY control_product.decisions WHERE decision_id=p_id AND source IN('user_confirmation','approved_document');
   IF NOT FOUND THEN RAISE EXCEPTION 'DECISION_NOT_FOUND'; END IF;
   -- The single root/child indexes and write checks make this an ordered chain.
   SELECT count(*),count(*) FILTER(WHERE decision_sequence<v_target.decision_sequence) INTO v_count,v_index
     FROM ONLY control_product.decisions WHERE project_id=v_target.project_id AND subject=v_target.subject
       AND source IN('user_confirmation','approved_document');
   v_start:=LEAST(v_index,GREATEST(0,v_count-p_depth));
   SELECT COALESCE(jsonb_agg(control_product.decision_row_v1(x.decision_id) ORDER BY x.decision_sequence),'[]'::jsonb) INTO v_rows
     FROM (SELECT decision_id,decision_sequence FROM ONLY control_product.decisions
       WHERE project_id=v_target.project_id AND subject=v_target.subject AND source IN('user_confirmation','approved_document')
       ORDER BY decision_sequence OFFSET v_start LIMIT p_depth) x;
   v_result:=v_base||jsonb_build_object('schema_version','lattice.control.decision-read.v1',
     'decision',control_product.decision_row_v1(p_id),'lineage',v_rows,
     'truncated_before',v_start>0,'truncated_after',v_start+jsonb_array_length(v_rows)<v_count);
   v_bound:=524288;
 END IF;
 -- Keep every returned row complete; shrink only the bounded result window.
 WHILE octet_length(v_result::text)>v_bound LOOP
   v_length:=jsonb_array_length(v_rows);
   IF v_length<=1 THEN RAISE EXCEPTION 'DECISION_OUTPUT_LIMIT_EXCEEDED'; END IF;
   IF p_mode='read' THEN
     IF v_index-v_start>=v_start+v_length-1-v_index AND v_start<v_index THEN
       v_rows:=v_rows-0; v_start:=v_start+1;
     ELSE
       v_rows:=v_rows-(v_length-1);
     END IF;
     v_result:=v_result||jsonb_build_object('lineage',v_rows,
       'truncated_before',v_start>0,'truncated_after',v_start+jsonb_array_length(v_rows)<v_count);
   ELSE
     v_rows:=v_rows-(v_length-1);
     v_result:=v_result||jsonb_build_object('decisions',v_rows,'truncated',true);
   END IF;
 END LOOP;
 RETURN v_result;
END
$decision_snapshot$;

CREATE FUNCTION control_product.decision_write_v1(
 p_id text,p_project text,p_task text,p_subject text,p_content text,p_reason text,p_source text,p_source_reference text,
 p_supersedes text,p_client_request text,p_expected_revision bigint,p_expected_digest text,p_digest text)
RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog SET lock_timeout='5s' SET statement_timeout='30s'
AS $decision$
DECLARE v_existing control_product.decisions%ROWTYPE; v_current control_product.decisions%ROWTYPE;
 v_head control_product.decision_state%ROWTYPE; v_result jsonb; v_changed boolean:=false; v_created timestamptz;
BEGIN
 IF session_user<>'lattice_runtime_login' OR current_setting('role')<>'lattice_runtime'
   OR current_setting('transaction_isolation')<>'serializable' OR current_setting('transaction_read_only')::boolean
 THEN RAISE EXCEPTION 'CONTROL_PRODUCT_ROLE_REJECTED'; END IF;
 -- One durable head closes the check/write race across all subjects and projects.
 SELECT * INTO STRICT v_head FROM ONLY control_product.decision_state WHERE singleton FOR UPDATE;
 SELECT * INTO v_existing FROM ONLY control_product.decisions WHERE client_request_id=p_client_request;
 IF FOUND THEN
   IF v_existing.request_digest IS DISTINCT FROM p_digest OR v_existing.decision_id IS DISTINCT FROM p_id
   THEN RAISE EXCEPTION 'DECISION_IDEMPOTENCY_CONFLICT'; END IF;
 ELSE
   IF p_expected_revision IS DISTINCT FROM v_head.revision OR p_expected_digest IS DISTINCT FROM v_head.digest
   THEN RAISE EXCEPTION 'DECISION_REVISION_MISMATCH'; END IF;
   IF p_source IS NULL OR p_source NOT IN('user_confirmation','approved_document')
     OR p_source_reference IS NULL OR octet_length(p_source_reference) NOT BETWEEN 1 AND 512
     OR p_client_request IS NULL OR length(p_client_request) NOT BETWEEN 1 AND 128
     OR p_subject IS NULL OR octet_length(p_subject) NOT BETWEEN 1 AND 256
     OR p_content IS NULL OR octet_length(p_content) NOT BETWEEN 1 AND 4096
     OR p_reason IS NULL OR octet_length(p_reason) NOT BETWEEN 1 AND 4096
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_INPUT_REJECTED'; END IF;
   IF (p_source='user_confirmation' AND p_source_reference !~ '^thread:[A-Za-z0-9][A-Za-z0-9._-]{0,127}/(?:turn|delegation):[A-Za-z0-9][A-Za-z0-9._:-]{0,127}(?:#[A-Za-z0-9][A-Za-z0-9._:-]{0,127})?$')
     OR (p_source='approved_document' AND p_source_reference !~ '^(?:file:[A-Za-z0-9][A-Za-z0-9._/-]{0,255}[A-Za-z0-9._/-]{0,128}|document:[A-Za-z0-9][A-Za-z0-9._:/-]{0,255}[A-Za-z0-9._:/-]{0,128})#[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$')
   THEN RAISE EXCEPTION 'DECISION_SOURCE_REJECTED'; END IF;
   IF NOT EXISTS(SELECT 1 FROM ONLY control.project_registry_projects WHERE project_id=p_project)
     OR (p_task IS NOT NULL AND NOT EXISTS(SELECT 1 FROM ONLY control.task_submission_envelopes WHERE task_ref=p_task AND project_id=p_project))
   THEN RAISE EXCEPTION 'CONTROL_PRODUCT_DECISION_SCOPE_REJECTED'; END IF;
   IF EXISTS(SELECT 1 FROM ONLY control_product.decisions WHERE decision_id=p_id)
   THEN RAISE EXCEPTION 'DECISION_IDEMPOTENCY_CONFLICT'; END IF;
   SELECT d.* INTO v_current FROM ONLY control_product.decisions d
     WHERE d.project_id=p_project AND d.subject=p_subject AND d.source IN('user_confirmation','approved_document')
       AND NOT EXISTS(SELECT 1 FROM ONLY control_product.decisions child WHERE child.supersedes_id=d.decision_id);
   IF p_supersedes IS NULL THEN
     IF FOUND THEN RAISE EXCEPTION 'DECISION_CURRENT_EXISTS'; END IF;
   ELSE
     IF NOT EXISTS(SELECT 1 FROM ONLY control_product.decisions WHERE decision_id=p_supersedes)
     THEN RAISE EXCEPTION 'DECISION_SUPERSESSION_TARGET_NOT_FOUND'; END IF;
     IF NOT EXISTS(SELECT 1 FROM ONLY control_product.decisions WHERE decision_id=p_supersedes AND project_id=p_project AND subject=p_subject AND source IN('user_confirmation','approved_document'))
     THEN RAISE EXCEPTION 'DECISION_CROSS_SCOPE_SUPERSESSION_REJECTED'; END IF;
     IF v_current.decision_id IS DISTINCT FROM p_supersedes
     THEN RAISE EXCEPTION 'DECISION_SUPERSESSION_TARGET_NOT_CURRENT'; END IF;
   END IF;
   IF v_head.revision>=10000 THEN RAISE EXCEPTION 'DECISION_STORE_LIMIT_EXCEEDED'; END IF;
   SELECT GREATEST(clock_timestamp(),COALESCE(max(created_at)+interval '1 millisecond',clock_timestamp())) INTO v_created FROM ONLY control_product.decisions;
   INSERT INTO control_product.decisions(decision_id,project_id,task_ref,subject,content,reason,source,source_reference,
     supersedes_id,client_request_id,decision_sequence,request_digest,created_at)
   VALUES(p_id,p_project,p_task,p_subject,p_content,p_reason,p_source,p_source_reference,
     p_supersedes,p_client_request,v_head.revision+1,p_digest,v_created) RETURNING * INTO v_existing;
   -- Request digests bind complete immutable row content; order makes the head deterministic.
   UPDATE control_product.decision_state SET revision=v_head.revision+1,
     digest=(SELECT encode(sha256(convert_to(COALESCE(jsonb_agg(jsonb_build_array(decision_id,request_digest) ORDER BY decision_id COLLATE "C"),'[]'::jsonb)::text,'UTF8')),'hex')
       FROM ONLY control_product.decisions WHERE source IN('user_confirmation','approved_document'))
     WHERE singleton RETURNING * INTO v_head;
   v_changed:=true;
 END IF;
 v_result:=jsonb_build_object('schema_version','lattice.control.decision-mutation.v1',
   'source',jsonb_build_object('kind','POSTGRESQL_CONTROL_PRODUCT','authority','POSTGRESQL_TASK_LEDGER'),
   'changed',v_changed,'revision',v_head.revision,'digest',v_head.digest,
   'decision',control_product.decision_row_v1(v_existing.decision_id));
 RETURN v_result;
END
$decision$;

-- Exact retained response lookup is independent of the clipped observation list.
CREATE FUNCTION control_product.question_resolution_v1(p_project text,p_task text,p_question text)
RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog
AS $question_resolution$
 SELECT jsonb_build_object('claim_id',r.claim_id,'turn_id',r.turn_id,'input_id',r.input_id,
   'approval_id',r.approval_id,'approval_decision',r.approval_decision,'payload',r.payload,
   'kind',r.kind,'sequence',r.sequence,'method',q.payload->>'method')
 FROM ONLY control_product.conversation_claims c
 JOIN ONLY control_product.conversation_observations r ON r.claim_id=c.claim_id
 JOIN ONLY control_product.conversation_observations q ON q.claim_id=r.claim_id AND q.approval_id=r.approval_id
   AND q.kind=CASE WHEN r.kind='APPROVAL_RESOLVED' THEN 'APPROVAL_REQUESTED' ELSE 'QUESTION_REQUESTED' END
   AND q.turn_id=r.turn_id AND q.input_id=r.input_id
 WHERE c.project_id=p_project AND c.task_ref=p_task AND r.approval_id=p_question
   AND r.kind IN('APPROVAL_RESOLVED','QUESTION_RESOLVED')
   AND session_user='lattice_runtime_login' AND current_setting('role')='lattice_runtime'
 ORDER BY r.sequence DESC LIMIT 1
$question_resolution$;


CREATE FUNCTION control_product.local_result_read_v1(p_digest bytea)
RETURNS SETOF control_product.local_verified_result_evidence
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog
AS $local_read$
 SELECT * FROM ONLY control_product.local_verified_result_evidence WHERE adoption_digest=p_digest
 AND session_user='lattice_runtime_login' AND current_setting('role')='lattice_runtime'
$local_read$;

CREATE FUNCTION control_product.external_result_binding_matches_v1(p_stream_id bytea,p_event_sequence text,p_event_digest bytea,p_command_id text,p_request_digest bytea,p_adoption_digest bytea,p_descriptor_digest bytea)
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog
AS $binding$
 SELECT session_user='lattice_runtime_login' AND current_setting('role')='lattice_runtime' AND EXISTS(
 SELECT 1 FROM ONLY control.task_external_verified_result_adoptions a
 JOIN control.task_ledger_events e ON e.stream_id=a.stream_id AND e.sequence=a.event_sequence
 JOIN control.task_ledger_commands c ON c.stream_id=a.stream_id AND c.command_id=a.command_id
 JOIN control.external_verified_result_evidence d ON d.adoption_digest=a.adoption_digest
 JOIN control.task_submission_envelopes s ON s.stream_id=a.stream_id
 WHERE a.stream_id=p_stream_id AND a.event_sequence=p_event_sequence::numeric
 AND a.event_digest=p_event_digest AND a.command_id=p_command_id AND a.request_digest=p_request_digest
 AND a.adoption_digest=p_adoption_digest AND a.evidence_descriptor_digest=p_descriptor_digest
 AND e.event_kind='EXTERNAL_VERIFIED_RESULT_ADOPTED' AND e.event_digest=a.event_digest AND e.subject_digest=a.adoption_digest
 AND c.event_kind=e.event_kind AND c.request_digest=a.request_digest AND c.event_digest=a.event_digest
 AND d.descriptor_digest=a.evidence_descriptor_digest AND d.task_ref=s.task_ref AND d.project_id=s.project_id AND d.project_snapshot_id=s.project_snapshot_id)
$binding$;

CREATE FUNCTION control_product.local_result_binding_matches_v1(p_stream_id bytea,p_event_sequence text,p_event_digest bytea,p_command_id text,p_request_digest bytea,p_adoption_digest bytea,p_descriptor_digest bytea)
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog
AS $local_binding$
 SELECT session_user='lattice_runtime_login' AND current_setting('role')='lattice_runtime' AND p_event_sequence='2' AND EXISTS(
 SELECT 1 FROM ONLY control_product.local_result_bindings a
 JOIN control.task_ledger_events e ON e.stream_id=a.stream_id AND e.sequence=2
 JOIN control.task_ledger_commands c ON c.stream_id=a.stream_id AND c.command_id=a.command_id
 JOIN control_product.local_verified_result_evidence d ON d.adoption_digest=a.adoption_digest
 JOIN control.task_submission_envelopes s ON s.stream_id=a.stream_id
 WHERE a.stream_id=p_stream_id AND a.event_digest=p_event_digest AND a.command_id=p_command_id
 AND a.request_digest=p_request_digest AND a.adoption_digest=p_adoption_digest AND a.descriptor_digest=p_descriptor_digest
 AND e.event_kind='EVIDENCE_RECORDED' AND e.action_id='LOCAL_VERIFIED_RESULT_ADOPTED'
 AND e.event_digest=a.event_digest AND e.subject_digest=a.adoption_digest
 AND c.event_kind=e.event_kind AND c.request_digest=a.request_digest AND c.event_digest=a.event_digest
 AND d.descriptor_digest=a.descriptor_digest AND d.task_ref=s.task_ref AND d.project_id=s.project_id AND d.project_snapshot_id=s.project_snapshot_id)
$local_binding$;

REVOKE ALL ON ALL FUNCTIONS IN SCHEMA control_product FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control_product.identity_read_v1(),
 control_product.task_refs_v1(text,text,integer),control_product.snapshot_v1(text,text[]),
 control_product.metadata_write_v1(text,text,text,bigint,text,text,integer,text,text[]),
 control_product.claim_v1(text,text,text,text,text,text,text),
 control_product.observe_v1(text,text,text,bigint,text,text,text,text,text,text,text,text,jsonb),
 control_product.decision_write_v1(text,text,text,text,text,text,text,text,text,text,bigint,text,text),
 control_product.decision_snapshot_v1(text,text,text,text,text,integer,integer,bigint,text),
 control_product.question_resolution_v1(text,text,text),
 control_product.local_result_read_v1(bytea),
 control_product.local_result_binding_matches_v1(bytea,text,bytea,text,bytea,bytea,bytea),
 control_product.external_result_binding_matches_v1(bytea,text,bytea,text,bytea,bytea,bytea)
 TO lattice_runtime;

CREATE FUNCTION control_product.task_ledger_finalize_general_result_v1(
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
    p_event_resource_projection_digest bytea,
    p_result_profile text
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
AS $lattice_task_ledger_finalize_general_result_v1$
DECLARE
    v_kind text; v_action text; v_reason text; v_prefix text;
    v_stream control.task_ledger_streams%ROWTYPE;
    v_submission control.task_submission_envelopes%ROWTYPE;
    v_descriptor bytea;
    v_manifest_entry_count bigint;
    v_history_manifest_sha256 text;
    v_terminal control.terminal_transactions%ROWTYPE;
    v_terminal_current_xact boolean;
    v_text text;
    v_digest bytea;
    v_zero bytea := pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex');
BEGIN
    IF p_result_profile='EXTERNAL_VERIFIED_RESULT_V1' THEN
        v_kind := 'EXTERNAL_VERIFIED_RESULT_ADOPTED'; v_action := 'ADOPT_VERIFIED_RESULT_V1';
        v_reason := 'EXTERNAL_VERIFIED_RESULT_ADOPTED'; v_prefix := 'external-result-adoption:';
    ELSIF p_result_profile='LOCAL_VERIFIED_RESULT_V1' THEN
        v_kind := 'EVIDENCE_RECORDED'; v_action := 'LOCAL_VERIFIED_RESULT_ADOPTED';
        v_reason := 'LOCAL_VERIFIED_RESULT_ADOPTED'; v_prefix := 'local-result-adoption:';
    ELSE RAISE EXCEPTION 'CONTROL_PRODUCT_RESULT_PROFILE_REJECTED'; END IF;
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
       OR p_global_schema_version <> 8
       OR p_global_manifest_sha256 IS NULL
       OR p_global_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_global_manifest_sha256 = pg_catalog.repeat('0', 64)
       OR v_manifest_entry_count IS DISTINCT FROM 10
       OR v_history_manifest_sha256 IS DISTINCT FROM p_global_manifest_sha256
       OR (SELECT pg_catalog.count(*)
             FROM ONLY control.schema_compatibility AS c
            WHERE c.singleton = true
              AND c.current_schema_version = 8
              AND c.min_reader = 8 AND c.max_reader = 8
              AND c.min_writer = 8 AND c.max_writer = 8
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
       OR p_next_sequence::numeric <> 2
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
       OR p_next_event_count::numeric <> 2
       OR p_next_command_count::numeric <> 2
       OR p_next_outbox_count::numeric <> 0
       OR p_base_checkpoint_digest = v_zero
       OR p_next_checkpoint_digest = v_zero
       OR p_base_checkpoint_digest = p_next_checkpoint_digest
       OR p_request_digest = v_zero
       OR p_expected_sequence::numeric <> 1
       OR p_expected_last_event_digest = v_zero
       OR p_expected_resource_revision::numeric <> 0
       OR p_expected_resource_projection_digest <> v_zero
       OR p_expected_head_digest = v_zero
       OR p_event_subject_digest = v_zero
       OR p_before_sequence::numeric <> 1
       OR p_before_last_event_digest IS DISTINCT FROM p_expected_last_event_digest
       OR p_before_resource_revision::numeric <> 0
       OR p_before_resource_projection_digest <> v_zero
       OR p_before_head_digest IS DISTINCT FROM p_expected_head_digest
       OR p_after_sequence::numeric <> 2
       OR p_after_last_event_digest IS DISTINCT FROM p_event_digest
       OR p_after_resource_revision::numeric <> 0
       OR p_after_resource_projection_digest <> v_zero
       OR p_after_head_digest IS DISTINCT FROM p_next_head_digest
       OR p_event_digest = v_zero
       OR p_receipt_digest = v_zero
       OR p_record_set_digest = v_zero
       OR p_event_sequence::numeric <> 2
       OR p_previous_event_digest IS DISTINCT FROM p_expected_last_event_digest
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
    SELECT * INTO v_stream FROM ONLY control.task_ledger_streams WHERE stream_id=p_stream_id FOR UPDATE;
    IF NOT FOUND OR v_stream.task_subject_kind<>'GENERAL_TASK_INTAKE'
       OR v_stream.task_spec_digest IS NOT NULL OR v_stream.accounting_currency IS NOT NULL
       OR v_stream.ledger_schema_version<>'2.0' OR v_stream.runtime<>'LIVE'
       OR v_stream.active_agents<>0 OR v_stream.active_implementers<>0 OR v_stream.elapsed_seconds<>0
       OR v_stream.attempt_number<>0 OR v_stream.used_model_calls<>0 OR v_stream.used_external_cost<>'0'
       OR v_stream.project_id IS DISTINCT FROM p_project_id OR v_stream.project_snapshot_id IS DISTINCT FROM p_project_snapshot_id
       OR v_stream.task_id IS DISTINCT FROM p_task_id OR v_stream.task_revision<>p_task_revision::numeric
       OR v_stream.task_subject_digest IS DISTINCT FROM p_task_subject_digest
       OR v_stream.sequence<>1 OR v_stream.event_count<>1 OR v_stream.command_count<>1 OR v_stream.outbox_count<>0
       OR v_stream.last_event_digest IS DISTINCT FROM p_expected_last_event_digest
       OR v_stream.head_digest IS DISTINCT FROM p_expected_head_digest OR v_stream.checkpoint_digest IS DISTINCT FROM p_base_checkpoint_digest
       OR v_stream.resource_revision<>0 OR v_stream.resource_projection_digest<>v_zero
    THEN RAISE EXCEPTION 'CONTROL_PRODUCT_RESULT_HEAD_REJECTED'; END IF;
    SELECT * INTO v_submission FROM ONLY control.task_submission_envelopes WHERE stream_id=p_stream_id FOR SHARE;
    IF NOT FOUND OR v_submission.project_id IS DISTINCT FROM p_project_id
       OR v_submission.project_snapshot_id IS DISTINCT FROM p_project_snapshot_id
       OR p_command_id IS DISTINCT FROM v_prefix||v_submission.client_request_id
       OR p_correlation_id IS DISTINCT FROM 'general-task-intake-v1'
       OR p_actor_id IS DISTINCT FROM (SELECT actor_id FROM ONLY control.task_ledger_events WHERE stream_id=p_stream_id AND sequence=1 AND event_kind='TASK_CREATED')
       OR NOT EXISTS(SELECT 1 FROM ONLY control.task_ledger_events WHERE stream_id=p_stream_id AND sequence=1
           AND action_id='GENERAL_TASK_INTAKE_V1' AND command_id='mcp-submit:'||v_submission.client_request_id
           AND subject_digest=v_submission.envelope_digest)
       OR NOT control.external_verified_result_adoption_preflight_v1(p_project_id,p_project_snapshot_id,v_submission.task_ref)
    THEN RAISE EXCEPTION 'CONTROL_PRODUCT_RESULT_IDENTITY_REJECTED'; END IF;
    IF p_result_profile='EXTERNAL_VERIFIED_RESULT_V1' THEN
      SELECT descriptor_digest INTO v_descriptor FROM ONLY control.external_verified_result_evidence
       WHERE adoption_digest=p_event_subject_digest AND project_id=p_project_id AND project_snapshot_id=p_project_snapshot_id AND task_ref=v_submission.task_ref FOR SHARE;
    ELSE
      SELECT descriptor_digest INTO v_descriptor FROM ONLY control_product.local_verified_result_evidence
       WHERE adoption_digest=p_event_subject_digest AND project_id=p_project_id AND project_snapshot_id=p_project_snapshot_id
         AND task_ref=v_submission.task_ref AND client_request_id=v_submission.client_request_id AND expected_head_digest=p_expected_head_digest FOR SHARE;
    END IF;
    IF v_descriptor IS NULL THEN RAISE EXCEPTION 'CONTROL_PRODUCT_RESULT_EVIDENCE_MISSING'; END IF;
    -- A Control-created conversation must finish the exact execution generation
    -- accepted by the retained independent verifier. The submission lock also
    -- serializes this check against a new dispatch; old receipts cannot close it.
    IF p_result_profile='LOCAL_VERIFIED_RESULT_V1'
       AND EXISTS(SELECT 1 FROM control_product.conversation_claims WHERE task_ref=v_submission.task_ref)
       AND NOT EXISTS(
         SELECT 1 FROM control_product.conversation_claims e
         JOIN control_product.conversation_claims v ON v.task_ref=e.task_ref AND v.phase='VERIFICATION'
         JOIN control_product.conversation_observations ed ON ed.claim_id=e.claim_id AND ed.kind='DISPATCH_STARTED'
         JOIN control_product.conversation_observations vd ON vd.claim_id=v.claim_id AND vd.kind='DISPATCH_STARTED' AND vd.execution_sequence=ed.sequence
         JOIN control_product.conversation_observations passed ON passed.claim_id=v.claim_id AND passed.sequence>vd.sequence AND passed.kind='VERIFICATION_PASSED'
         JOIN control_product.local_verified_result_evidence d ON d.adoption_digest=p_event_subject_digest
           AND d.acceptance_ref=passed.evidence_ref AND d.independent_verifier='codex:'||passed.thread_id||':'||passed.turn_id
         WHERE e.task_ref=v_submission.task_ref AND e.phase='EXECUTION'
           AND NOT EXISTS(SELECT 1 FROM control_product.conversation_observations later WHERE later.claim_id=e.claim_id AND later.kind='DISPATCH_STARTED' AND later.sequence>ed.sequence)
           AND NOT EXISTS(SELECT 1 FROM control_product.conversation_observations later WHERE later.claim_id=v.claim_id AND later.kind='DISPATCH_STARTED' AND later.sequence>vd.sequence)
           AND NOT EXISTS(SELECT 1 FROM control_product.conversation_observations later WHERE later.claim_id=v.claim_id AND later.kind='VERIFICATION_FAILED' AND later.sequence>passed.sequence)
           AND EXISTS(SELECT 1 FROM control_product.conversation_observations done WHERE done.claim_id=e.claim_id AND done.kind='TURN_COMPLETED' AND done.sequence>ed.sequence)
       )
    THEN RAISE EXCEPTION 'CONTROL_PRODUCT_VERIFICATION_STALE'; END IF;

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
       OR v_terminal.expected_revision IS DISTINCT FROM 1
       OR v_terminal.before_revision IS DISTINCT FROM 1
       OR v_terminal.after_revision IS DISTINCT FROM 2
       OR v_terminal.before_state_digest IS DISTINCT FROM p_base_checkpoint_digest
       OR v_terminal.after_state_digest IS DISTINCT FROM p_next_checkpoint_digest
       OR v_terminal.schema_version IS DISTINCT FROM 2
       OR pg_catalog.btrim(v_terminal.manifest_sha256::text) IS DISTINCT FROM
            '4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'general intake store pair corrupt';
    END IF;

    UPDATE ONLY control.task_ledger_streams SET sequence=2,last_event_digest=p_event_digest,
        head_digest=p_next_head_digest,event_count=2,command_count=2,checkpoint_digest=p_next_checkpoint_digest
      WHERE stream_id=p_stream_id;

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
        p_stream_id,p_command_id,'2.0',p_request_digest,1,p_expected_last_event_digest,0,v_zero,
        p_expected_head_digest,p_correlation_id,p_occurred_at,v_kind,
        p_actor_id,v_action,'RECORDED',v_reason,
        p_event_subject_digest,'null'::jsonb,false,0,0,0,0,0,'0','2.0',1,p_expected_last_event_digest,
        0,v_zero,p_before_head_digest,2,p_event_digest,0,v_zero,p_after_head_digest,
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
        p_stream_id,2,'2.0',p_previous_event_digest,p_command_id,p_request_digest,p_correlation_id,
        p_occurred_at,v_kind,p_actor_id,v_action,'RECORDED',
        v_reason,p_event_subject_digest,'null'::jsonb,false,
        0,0,0,0,0,'0',0,v_zero,p_event_digest
    );

    IF (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_streams AS s WHERE s.stream_id=p_stream_id) <> 1
       OR (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_commands AS c WHERE c.stream_id=p_stream_id) <> 2
       OR (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_events AS e WHERE e.stream_id=p_stream_id) <> 2
       OR (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_outbox AS o WHERE o.stream_id=p_stream_id) <> 0
       OR (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_autonomy_receipts AS a WHERE a.stream_id=p_stream_id) <> 0
       OR (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_foreman_snapshots AS f WHERE f.stream_id=p_stream_id) <> 0
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'general intake row count corrupt';
    END IF;
    IF p_result_profile='EXTERNAL_VERIFIED_RESULT_V1' THEN
      IF control.external_verified_result_adoption_bind_v1(p_stream_id,'2',p_event_digest,p_command_id,p_request_digest,p_event_subject_digest,v_descriptor)<>'RECORDED' THEN
        RAISE EXCEPTION 'CONTROL_PRODUCT_RESULT_BINDING_REJECTED';
      END IF;
    ELSE
      INSERT INTO control_product.local_result_bindings(stream_id,event_digest,command_id,request_digest,adoption_digest,descriptor_digest)
      VALUES(p_stream_id,p_event_digest,p_command_id,p_request_digest,p_event_subject_digest,v_descriptor);
    END IF;
    RETURN 'FINALIZED';
END;
$lattice_task_ledger_finalize_general_result_v1$;

REVOKE ALL ON FUNCTION control_product.task_ledger_finalize_general_result_v1(smallint,text,bytea,text,text,text,text,text,bytea,text,bytea,text,bytea,bytea,text,text,text,text,text,text,text,text,text,bytea,bytea,text,bytea,text,bytea,text,bytea,bytea,text,text,text,bytea,text,bytea,text,bytea,bytea,text,bytea,text,bytea,bytea,bytea,bytea,bytea,text,text,bytea,text,bytea,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control_product.task_ledger_finalize_general_result_v1(smallint,text,bytea,text,text,text,text,text,bytea,text,bytea,text,bytea,bytea,text,text,text,text,text,text,text,text,text,bytea,bytea,text,bytea,text,bytea,text,bytea,bytea,text,text,text,bytea,text,bytea,text,bytea,bytea,text,bytea,text,bytea,bytea,bytea,bytea,bytea,text,text,bytea,text,bytea,text) TO lattice_runtime;

CREATE FUNCTION control_product.task_ingress_historical_closure_v1()
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
                  AND s.sequence IN (1,2)
                  AND s.last_event_digest=(SELECT last.event_digest FROM ONLY control.task_ledger_events last
                      WHERE last.stream_id=s.stream_id AND last.sequence=s.sequence)
                  AND (s.sequence=1 OR EXISTS (
                      SELECT 1 FROM ONLY control.task_ledger_events terminal
                      JOIN ONLY control.task_ledger_commands command ON command.stream_id=terminal.stream_id
                        AND command.command_id=terminal.command_id
                      WHERE terminal.stream_id=s.stream_id AND terminal.sequence=2
                        AND terminal.previous_event_digest=v.event_digest
                        AND terminal.actor_id=e.actor_id AND terminal.correlation_id='general-task-intake-v1'
                        AND terminal.audit_outcome='RECORDED' AND terminal.diagnostic='null'::jsonb
                        AND NOT terminal.has_resource_snapshot AND terminal.resource_revision=0
                        AND terminal.resource_projection_digest=pg_catalog.decode(pg_catalog.repeat('00',32),'hex')
                        AND command.command_outcome='APPENDED' AND command.denial_reason=''
                        AND command.expected_sequence=1 AND command.before_sequence=1 AND command.after_sequence=2
                        AND command.expected_head_digest=m.after_head_digest AND command.before_head_digest=m.after_head_digest
                        AND command.after_head_digest=s.head_digest AND command.result_checkpoint_digest=s.checkpoint_digest
                        AND ROW(command.request_digest,command.correlation_id,command.occurred_at,command.event_kind,
                            command.actor_id,command.action_id,command.audit_outcome,command.reason_code,command.subject_digest,
                            command.diagnostic,command.has_resource_snapshot,command.event_digest)
                            IS NOT DISTINCT FROM ROW(terminal.request_digest,terminal.correlation_id,terminal.occurred_at,terminal.event_kind,
                            terminal.actor_id,terminal.action_id,terminal.audit_outcome,terminal.reason_code,terminal.subject_digest,
                            terminal.diagnostic,terminal.has_resource_snapshot,terminal.event_digest)
                        AND ((terminal.event_kind='EXTERNAL_VERIFIED_RESULT_ADOPTED'
                            AND terminal.action_id='ADOPT_VERIFIED_RESULT_V1' AND terminal.reason_code='EXTERNAL_VERIFIED_RESULT_ADOPTED'
                            AND terminal.command_id='external-result-adoption:'||v.client_request_id
                            AND EXISTS (SELECT 1 FROM ONLY control.task_external_verified_result_adoptions b
                                JOIN ONLY control.external_verified_result_evidence d ON d.adoption_digest=b.adoption_digest
                                WHERE b.stream_id=s.stream_id AND b.event_sequence=2 AND b.event_digest=terminal.event_digest
                                  AND b.command_id=terminal.command_id AND b.request_digest=terminal.request_digest
                                  AND b.adoption_digest=terminal.subject_digest AND b.evidence_descriptor_digest=d.descriptor_digest
                                  AND d.task_ref=v.task_ref AND d.project_id=v.project_id AND d.project_snapshot_id=v.project_snapshot_id))
                          OR (terminal.event_kind='EVIDENCE_RECORDED'
                            AND terminal.action_id='LOCAL_VERIFIED_RESULT_ADOPTED' AND terminal.reason_code='LOCAL_VERIFIED_RESULT_ADOPTED'
                            AND terminal.command_id='local-result-adoption:'||v.client_request_id
                            AND EXISTS (SELECT 1 FROM ONLY control_product.local_result_bindings b
                                JOIN ONLY control_product.local_verified_result_evidence d ON d.adoption_digest=b.adoption_digest
                                WHERE b.stream_id=s.stream_id AND b.event_digest=terminal.event_digest
                                  AND b.command_id=terminal.command_id AND b.request_digest=terminal.request_digest
                                  AND b.adoption_digest=terminal.subject_digest AND b.descriptor_digest=d.descriptor_digest
                                  AND d.task_ref=v.task_ref AND d.project_id=v.project_id AND d.project_snapshot_id=v.project_snapshot_id
                                  AND d.client_request_id=v.client_request_id AND d.expected_head_digest=command.expected_head_digest)))
                  ))
                  AND s.resource_revision=0
                  AND s.resource_projection_digest=pg_catalog.decode(pg_catalog.repeat('00',32),'hex')
                  AND s.active_agents=0 AND s.active_implementers=0
                  AND s.elapsed_seconds=0 AND s.attempt_number=0
                  AND s.used_model_calls=0 AND s.used_external_cost='0'
                  AND s.event_count=s.sequence AND s.command_count=s.sequence AND s.outbox_count=0
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
                  AND (s.sequence=2 OR m.after_head_digest=s.head_digest)
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
REVOKE ALL ON FUNCTION control_product.task_ingress_historical_closure_v1() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control_product.task_ingress_historical_closure_v1() TO lattice_runtime;
