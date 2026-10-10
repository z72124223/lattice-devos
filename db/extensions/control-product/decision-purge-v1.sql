-- Optional exact successor of the MCP permission catalog. No project content retained.
CREATE TABLE control_product.decision_retired_keys (
    kind text NOT NULL CHECK(kind IN ('decision','request')),
    key_digest text NOT NULL CHECK(key_digest ~ '^[a-f0-9]{64}$'),
    PRIMARY KEY(kind,key_digest)
);
REVOKE ALL ON TABLE control_product.decision_retired_keys FROM PUBLIC,lattice_runtime,lattice_guardian,lattice_readonly;

CREATE OR REPLACE FUNCTION control_product.decision_write_v1(
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
 -- Retired identities are rejected before exact replay, independently of owner/content.
 IF EXISTS(SELECT 1 FROM ONLY control_product.decision_retired_keys
   WHERE (kind='decision' AND key_digest=encode(sha256(convert_to('lattice.decision.retired-id.v1'||chr(10)||p_id,'UTF8')),'hex'))
      OR (kind='request' AND key_digest=encode(sha256(convert_to('lattice.decision.retired-request.v1'||chr(10)||p_client_request,'UTF8')),'hex')))
 THEN RAISE EXCEPTION 'DECISION_RETIRED_ID_REJECTED'; END IF;
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
