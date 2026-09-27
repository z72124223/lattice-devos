-- Exact successor of the known Control Product + Graph Usage function catalog.
-- Only this read projection changes; no tables, ACLs or observation rows change.
-- Binding comes from relational claim/dispatch/turn/input columns, never payload.binding.
CREATE OR REPLACE FUNCTION control_product.snapshot_v1(p_project_id text,p_task_refs text[])
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
       'mcp_permission',jsonb_build_object('version',1,'denied',EXISTS(
         SELECT 1 FROM ONLY control_product.conversation_observations q
         JOIN ONLY control_product.conversation_observations r ON r.claim_id=q.claim_id
           AND r.approval_id=q.approval_id AND r.thread_id=q.thread_id AND r.turn_id=q.turn_id AND r.input_id=q.input_id
           AND r.kind='QUESTION_RESOLVED' AND r.sequence>q.sequence
         JOIN LATERAL (SELECT d.sequence,d.input_id FROM ONLY control_product.conversation_observations d
           WHERE d.claim_id=c.claim_id AND d.kind='DISPATCH_STARTED' ORDER BY d.sequence DESC LIMIT 1) dispatch ON true
         JOIN LATERAL (SELECT b.thread_id,b.turn_id FROM ONLY control_product.conversation_observations b
           WHERE b.claim_id=c.claim_id AND b.kind='TURN_BOUND' AND b.sequence>dispatch.sequence
           ORDER BY b.sequence DESC LIMIT 1) turn_binding ON true
         WHERE q.claim_id=c.claim_id AND q.kind='QUESTION_REQUESTED' AND q.sequence>dispatch.sequence
           AND q.input_id=dispatch.input_id AND q.thread_id=turn_binding.thread_id AND q.turn_id=turn_binding.turn_id
           AND q.payload->>'method'='mcpServer/elicitation/request'
           AND r.payload->>'method'='mcpServer/elicitation/request'
           AND r.payload->'response'->>'action' IN('decline','cancel'))),
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
