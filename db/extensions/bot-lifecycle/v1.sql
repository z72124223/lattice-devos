-- Dedicated lifecycle database. No changes to the existing Store database.
CREATE SCHEMA bot_lifecycle AUTHORIZATION lattice_migrator;
REVOKE ALL ON SCHEMA bot_lifecycle FROM PUBLIC;
GRANT USAGE ON SCHEMA bot_lifecycle TO lattice_runtime;
CREATE TABLE bot_lifecycle.identity (
 singleton boolean PRIMARY KEY CHECK(singleton), run_id text NOT NULL,
 sql_sha256 text NOT NULL CHECK(sql_sha256 ~ '^[a-f0-9]{64}$')
);
CREATE TABLE bot_lifecycle.roles (
 project_id text NOT NULL, role_id text NOT NULL, state jsonb NOT NULL,
 PRIMARY KEY(project_id,role_id)
);
CREATE TABLE bot_lifecycle.events (
 project_id text NOT NULL, role_id text NOT NULL, request_id text NOT NULL,
 request_digest text NOT NULL, request jsonb NOT NULL, receipt jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(project_id,role_id,request_id)
);

CREATE FUNCTION bot_lifecycle.keys_v1(p jsonb, keys text[]) RETURNS void
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF jsonb_typeof(p) IS DISTINCT FROM 'object' OR
    (SELECT array_agg(k ORDER BY k) FROM jsonb_object_keys(p) k) IS DISTINCT FROM
    (SELECT array_agg(k ORDER BY k) FROM unnest(keys) k) THEN
   RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF;
END $$;
CREATE FUNCTION bot_lifecycle.ids_v1(p jsonb) RETURNS jsonb
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE v jsonb;
BEGIN
 IF jsonb_typeof(p) IS DISTINCT FROM 'array' OR jsonb_array_length(p)>128 THEN
   RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF;
 IF EXISTS(SELECT 1 FROM jsonb_array_elements(p) x WHERE jsonb_typeof(x)<>'string'
   OR (x#>>'{}') !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$') THEN
   RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF;
 SELECT COALESCE(jsonb_agg(x ORDER BY x),'[]'::jsonb) INTO v FROM (SELECT DISTINCT x FROM jsonb_array_elements(p) x) s;
 IF jsonb_array_length(v)<>jsonb_array_length(p) THEN RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF;
 RETURN v;
END $$;
CREATE FUNCTION bot_lifecycle.receipt_v1(p jsonb) RETURNS void
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 PERFORM bot_lifecycle.keys_v1(p,ARRAY['tool','target_thread_id','target_host_id','result_digest','readback_digest','evidence_ref','success','readback_verified','old_pending_count']);
 IF p->'success' IS DISTINCT FROM 'true'::jsonb OR p->'readback_verified' IS DISTINCT FROM 'true'::jsonb
   OR COALESCE(p->>'tool','') NOT IN('create_thread','fork_thread','read_thread','send_message_to_thread','automation_update','set_thread_archived','hq-routing-update','schedule-not-configured')
   OR COALESCE(p->>'target_thread_id','') !~ '^[a-f0-9]{8}(-[a-f0-9]{4}){3}-[a-f0-9]{12}$'
   OR COALESCE(p->>'target_host_id','') !~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'
   OR COALESCE(p->>'result_digest','') !~ '^[a-f0-9]{64}$'
   OR COALESCE(p->>'readback_digest','') !~ '^[a-f0-9]{64}$'
   OR length(p->>'evidence_ref')>512
   OR COALESCE(p->>'evidence_ref','') !~ '^[A-Za-z0-9][A-Za-z0-9._:/#-]*$'
   OR jsonb_typeof(p->'old_pending_count') IS DISTINCT FROM 'number'
   OR COALESCE(p->>'old_pending_count','') !~ '^[0-9]{1,6}$' THEN
   RAISE EXCEPTION 'BOT_LIFECYCLE_NATIVE_EVIDENCE_REJECTED'; END IF;
END $$;
CREATE FUNCTION bot_lifecycle.manifest_v1(p jsonb, work jsonb) RETURNS jsonb
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE k text;
BEGIN
 PERFORM bot_lifecycle.keys_v1(p,ARRAY['work_ids','pending_decision_ids','pending_input_ids','in_flight_ids','requirements_digest','authorization_digest','git_state_digest','recovery_digest','evidence_digest','next_steps_digest','rules_digest']);
 IF bot_lifecycle.ids_v1(p->'work_ids') IS DISTINCT FROM work THEN RAISE EXCEPTION 'BOT_LIFECYCLE_WORK_SET_MISMATCH'; END IF;
 FOREACH k IN ARRAY ARRAY['pending_decision_ids','pending_input_ids','in_flight_ids'] LOOP
   p := jsonb_set(p,ARRAY[k],bot_lifecycle.ids_v1(p->k));
 END LOOP;
 FOREACH k IN ARRAY ARRAY['requirements_digest','authorization_digest','git_state_digest','recovery_digest','evidence_digest','next_steps_digest','rules_digest'] LOOP
   IF COALESCE(p->>k,'') !~ '^[a-f0-9]{64}$' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF;
 END LOOP;
 RETURN jsonb_set(p,'{work_ids}',work);
END $$;
CREATE FUNCTION bot_lifecycle.identity_read_v1() RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('run_id',run_id,'sql_sha256',sql_sha256,'database',current_database())
 FROM ONLY bot_lifecycle.identity WHERE singleton
$$;
CREATE FUNCTION bot_lifecycle.read_v1(p_project text,p_role text) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE s jsonb;
BEGIN
 IF session_user<>'lattice_runtime_login' OR current_setting('role')<>'lattice_runtime' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ROLE_REJECTED'; END IF;
 SELECT state INTO s FROM ONLY bot_lifecycle.roles WHERE project_id=p_project AND role_id=p_role;
 RETURN jsonb_build_object('schema_version','lattice.bot-lifecycle.v1','status','READ','current',s);
END $$;
CREATE FUNCTION bot_lifecycle.apply_v1(p jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog SET lock_timeout='5s' SET statement_timeout='30s' AS $$
DECLARE a text:=p->>'action'; b jsonb:=p->'body'; s jsonb; old jsonb; rec jsonb;
 reqhash text; m jsonb; ids jsonb; step text; entry jsonb; receipt jsonb; rev bigint; gen bigint;
BEGIN
 IF session_user<>'lattice_runtime_login' OR current_setting('role')<>'lattice_runtime'
   OR current_setting('transaction_isolation')<>'serializable' OR current_setting('transaction_read_only')::boolean THEN
   RAISE EXCEPTION 'BOT_LIFECYCLE_ROLE_REJECTED'; END IF;
 PERFORM bot_lifecycle.keys_v1(p,ARRAY['action','request_id','project_id','role_id','expected_revision','expected_generation','owner_thread_id','owner_host_id','body']);
 IF octet_length(p::text)>65536 OR COALESCE(p->>'project_id','') !~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'
   OR COALESCE(p->>'role_id','') !~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'
   OR COALESCE(p->>'request_id','') !~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$'
   OR COALESCE(p->>'owner_thread_id','') !~ '^[a-f0-9]{8}(-[a-f0-9]{4}){3}-[a-f0-9]{12}$'
   OR COALESCE(p->>'owner_host_id','') !~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'
   OR jsonb_typeof(p->'expected_revision') IS DISTINCT FROM 'number' OR COALESCE(p->>'expected_revision','') !~ '^[0-9]{1,15}$'
   OR jsonb_typeof(p->'expected_generation') IS DISTINCT FROM 'number' OR COALESCE(p->>'expected_generation','') !~ '^[0-9]{1,15}$'
   OR a NOT IN('register','add-work','checkpoint','prepare','reserve-step','native-step','ack','commit','finish','abort') THEN
   RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF;
 reqhash:=encode(sha256(convert_to(p::text,'UTF8')),'hex');
 -- A role lock serializes transition decisions, including concurrent registration.
 PERFORM pg_advisory_xact_lock(hashtextextended((p->>'project_id')||'/'||(p->>'role_id'),0));
 SELECT state INTO s FROM ONLY bot_lifecycle.roles WHERE project_id=p->>'project_id' AND role_id=p->>'role_id' FOR UPDATE;
 SELECT jsonb_build_object('digest',request_digest,'receipt',e.receipt) INTO old FROM ONLY bot_lifecycle.events e
   WHERE project_id=p->>'project_id' AND role_id=p->>'role_id' AND request_id=p->>'request_id';
 IF old IS NOT NULL THEN
   IF old->>'digest'<>reqhash THEN RAISE EXCEPTION 'BOT_LIFECYCLE_IDEMPOTENCY_CONFLICT'; END IF;
   RETURN jsonb_build_object('schema_version','lattice.bot-lifecycle.v1','status','REPLAYED','receipt',old->'receipt','current',s);
 END IF;
 IF a='register' THEN
   PERFORM bot_lifecycle.keys_v1(b,ARRAY['work_ids','policy_digest','rules_digest','binding_receipt']);
   IF s IS NOT NULL OR (p->>'expected_revision')::bigint<>0 OR (p->>'expected_generation')::bigint<>0 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_REVISION_CONFLICT'; END IF;
   PERFORM bot_lifecycle.receipt_v1(b->'binding_receipt');
   IF b->'binding_receipt'->>'tool'<>'read_thread' OR b->'binding_receipt'->>'target_thread_id'<>p->>'owner_thread_id'
     OR b->'binding_receipt'->>'target_host_id'<>p->>'owner_host_id' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_NATIVE_TARGET_MISMATCH'; END IF;
   IF COALESCE(b->>'policy_digest','') !~ '^[a-f0-9]{64}$' OR COALESCE(b->>'rules_digest','') !~ '^[a-f0-9]{64}$' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF;
   s:=jsonb_build_object('project_id',p->>'project_id','role_id',p->>'role_id','revision',1,'generation',1,
     'owner_thread_id',p->>'owner_thread_id','owner_host_id',p->>'owner_host_id','phase','ACTIVE',
     'work_ids',bot_lifecycle.ids_v1(b->'work_ids'),'policy_digest',b->>'policy_digest','rules_digest',b->>'rules_digest',
     'manifest',NULL,'manifest_digest',NULL,'handoff',NULL,'ack',NULL,'steps','{}'::jsonb);
 ELSE
   IF s IS NULL THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ROLE_MISSING'; END IF;
   IF s->>'generation'<>p->>'expected_generation' OR s->>'owner_thread_id'<>p->>'owner_thread_id' OR s->>'owner_host_id'<>p->>'owner_host_id' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_STALE_OWNER'; END IF;
   IF s->>'revision'<>p->>'expected_revision' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_REVISION_CONFLICT'; END IF;
   rev:=(s->>'revision')::bigint; gen:=(s->>'generation')::bigint;
   IF rev>=999999999999999 OR gen>=999999999999999 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_LIMIT'; END IF;
   IF a IN('add-work','checkpoint') AND s->>'phase'='MIGRATING' AND s->'steps'->'old_archived' IS NOT NULL THEN
     RAISE EXCEPTION 'BOT_LIFECYCLE_ARCHIVE_NOT_READY'; END IF;
   CASE a
   WHEN 'add-work' THEN
     PERFORM bot_lifecycle.keys_v1(b,ARRAY['work_ids']); ids:=bot_lifecycle.ids_v1(b->'work_ids');
     SELECT jsonb_agg(x ORDER BY x) INTO ids FROM (SELECT DISTINCT x FROM jsonb_array_elements((s->'work_ids')||ids) x) u;
     ids:=bot_lifecycle.ids_v1(COALESCE(ids,'[]'::jsonb));
     s:=s||jsonb_build_object('work_ids',ids,'manifest',NULL,'manifest_digest',NULL,'ack',NULL);
     IF s->>'phase'='VERIFIED' THEN s:=s||'{"phase":"PREPARING"}'::jsonb; END IF;
   WHEN 'checkpoint','prepare' THEN
     IF a='prepare' THEN
       PERFORM bot_lifecycle.keys_v1(b,ARRAY['handoff_id','manifest']);
       IF s->>'phase'<>'ACTIVE' OR COALESCE(b->>'handoff_id','') !~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_PHASE_REJECTED'; END IF;
       s:=s||jsonb_build_object('phase','PREPARING','handoff',jsonb_build_object('handoff_id',b->>'handoff_id',
         'old_thread_id',s->>'owner_thread_id','old_host_id',s->>'owner_host_id','from_generation',gen),'steps','{}'::jsonb);
     ELSE PERFORM bot_lifecycle.keys_v1(b,ARRAY['manifest']); END IF;
     m:=bot_lifecycle.manifest_v1(b->'manifest',s->'work_ids');
     IF m->>'rules_digest'<>s->>'rules_digest' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_RULES_MISMATCH'; END IF;
     s:=s||jsonb_build_object('manifest',m,'manifest_digest',encode(sha256(convert_to(m::text,'UTF8')),'hex'),'ack',NULL);
     IF s->>'phase'='VERIFIED' THEN s:=s||'{"phase":"PREPARING"}'::jsonb; END IF;
   WHEN 'reserve-step','native-step' THEN
     IF a='reserve-step' THEN PERFORM bot_lifecycle.keys_v1(b,ARRAY['step','operation_key','input_digest']);
     ELSE PERFORM bot_lifecycle.keys_v1(b,ARRAY['step','operation_key','receipt']); END IF;
     step:=b->>'step'; entry:=s->'steps'->step;
     IF COALESCE(step,'') NOT IN('successor_created','routing_updated','schedule_updated','old_archived')
       OR COALESCE(b->>'operation_key','') !~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$'
       OR (step='successor_created' AND s->>'phase'<>'PREPARING')
       OR (step<>'successor_created' AND s->>'phase'<>'MIGRATING') THEN RAISE EXCEPTION 'BOT_LIFECYCLE_PHASE_REJECTED'; END IF;
     IF a='reserve-step' THEN
       IF step='old_archived' AND (COALESCE(s->'steps'->'routing_updated'->>'status','')<>'COMPLETED'
         OR COALESCE(s->'steps'->'schedule_updated'->>'status','')<>'COMPLETED'
         OR s->'manifest' IS NULL OR s->'manifest'='null'::jsonb
         OR jsonb_array_length(s->'manifest'->'pending_input_ids')<>0 OR jsonb_array_length(s->'manifest'->'in_flight_ids')<>0) THEN
         RAISE EXCEPTION 'BOT_LIFECYCLE_ARCHIVE_NOT_READY'; END IF;
       IF entry IS NOT NULL THEN RAISE EXCEPTION 'BOT_LIFECYCLE_STEP_ALREADY_RESERVED'; END IF;
       IF COALESCE(b->>'input_digest','') !~ '^[a-f0-9]{64}$' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF;
       entry:=jsonb_build_object('status','RESERVED','operation_key',b->>'operation_key','input_digest',b->>'input_digest');
     ELSE
       IF entry IS NULL OR entry->>'status'<>'RESERVED' OR entry->>'operation_key'<>b->>'operation_key' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_STEP_NOT_RESERVED'; END IF;
       receipt:=b->'receipt'; PERFORM bot_lifecycle.receipt_v1(receipt);
       IF step='successor_created' THEN
         IF receipt->>'tool' NOT IN('create_thread','fork_thread') OR receipt->>'target_thread_id'=s->>'owner_thread_id' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_NATIVE_TARGET_MISMATCH'; END IF;
         s:=jsonb_set(s,'{handoff}',(s->'handoff')||jsonb_build_object('successor_thread_id',receipt->>'target_thread_id','successor_host_id',receipt->>'target_host_id'));
       ELSIF step='old_archived' THEN
         IF receipt->>'tool'<>'set_thread_archived' OR receipt->>'target_thread_id'<>s->'handoff'->>'old_thread_id' OR receipt->>'target_host_id'<>s->'handoff'->>'old_host_id'
           OR receipt->>'old_pending_count'<>'0' OR COALESCE(s->'steps'->'routing_updated'->>'status','')<>'COMPLETED'
           OR COALESCE(s->'steps'->'schedule_updated'->>'status','')<>'COMPLETED'
           OR s->'manifest' IS NULL OR s->'manifest'='null'::jsonb
           OR jsonb_array_length(s->'manifest'->'pending_input_ids')<>0 OR jsonb_array_length(s->'manifest'->'in_flight_ids')<>0 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ARCHIVE_NOT_READY'; END IF;
       ELSE
         IF receipt->>'target_thread_id'<>s->>'owner_thread_id' OR receipt->>'target_host_id'<>s->>'owner_host_id'
           OR (step='routing_updated' AND receipt->>'tool'<>'hq-routing-update')
           OR (step='schedule_updated' AND receipt->>'tool' NOT IN('automation_update','schedule-not-configured')) THEN RAISE EXCEPTION 'BOT_LIFECYCLE_NATIVE_TARGET_MISMATCH'; END IF;
       END IF;
       entry:=entry||jsonb_build_object('status','COMPLETED','receipt',receipt);
     END IF;
     s:=jsonb_set(s,ARRAY['steps',step],entry);
   WHEN 'ack' THEN
     PERFORM bot_lifecycle.keys_v1(b,ARRAY['manifest_digest','work_ids','verification_receipt']);
     receipt:=b->'verification_receipt'; PERFORM bot_lifecycle.receipt_v1(receipt);
     IF s->>'phase'<>'PREPARING' OR COALESCE(s->'steps'->'successor_created'->>'status','')<>'COMPLETED'
       OR s->>'manifest_digest' IS NULL OR s->>'manifest_digest' IS DISTINCT FROM b->>'manifest_digest'
       OR bot_lifecycle.ids_v1(b->'work_ids') IS DISTINCT FROM s->'work_ids'
       OR receipt->>'tool'<>'read_thread' OR receipt->>'target_thread_id' IS DISTINCT FROM s->'handoff'->>'successor_thread_id'
       OR receipt->>'target_host_id' IS DISTINCT FROM s->'handoff'->>'successor_host_id' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ACK_REJECTED'; END IF;
     s:=s||jsonb_build_object('phase','VERIFIED','ack',jsonb_build_object('manifest_digest',b->>'manifest_digest','receipt',receipt));
   WHEN 'commit' THEN
     PERFORM bot_lifecycle.keys_v1(b,ARRAY['manifest_digest']);
     IF s->>'phase'<>'VERIFIED' OR s->>'manifest_digest' IS NULL
       OR s->>'manifest_digest' IS DISTINCT FROM b->>'manifest_digest'
       OR s->'ack'->>'manifest_digest' IS DISTINCT FROM s->>'manifest_digest' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_UNVERIFIED_SWITCH'; END IF;
     s:=s||jsonb_build_object('phase','MIGRATING','generation',gen+1,'owner_thread_id',s->'handoff'->>'successor_thread_id','owner_host_id',s->'handoff'->>'successor_host_id');
   WHEN 'finish' THEN
     PERFORM bot_lifecycle.keys_v1(b,ARRAY[]::text[]);
     IF s->>'phase'<>'MIGRATING' OR COALESCE(s->'steps'->'old_archived'->>'status','')<>'COMPLETED' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_MIGRATION_INCOMPLETE'; END IF;
     s:=s||'{"phase":"ACTIVE"}'::jsonb;
   WHEN 'abort' THEN
     PERFORM bot_lifecycle.keys_v1(b,ARRAY['reason_digest','candidate_disposition_receipt']);
     IF s->>'phase' NOT IN('PREPARING','VERIFIED') OR COALESCE(b->>'reason_digest','') !~ '^[a-f0-9]{64}$'
       OR EXISTS(SELECT 1 FROM jsonb_each(s->'steps') e WHERE e.value->>'status'='RESERVED') THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ABORT_UNSAFE'; END IF;
     IF s->'handoff'->>'successor_thread_id' IS NOT NULL THEN
       receipt:=b->'candidate_disposition_receipt'; PERFORM bot_lifecycle.receipt_v1(receipt);
       IF receipt->>'tool'<>'set_thread_archived' OR receipt->>'target_thread_id'<>s->'handoff'->>'successor_thread_id'
         OR receipt->>'target_host_id'<>s->'handoff'->>'successor_host_id' OR receipt->>'old_pending_count'<>'0' THEN
         RAISE EXCEPTION 'BOT_LIFECYCLE_ABORT_UNSAFE'; END IF;
     ELSIF b->'candidate_disposition_receipt' IS DISTINCT FROM 'null'::jsonb THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ABORT_UNSAFE'; END IF;
     s:=s||jsonb_build_object('phase','ACTIVE','ack',NULL,'abort_evidence',b);
   END CASE;
   s:=jsonb_set(s,'{revision}',to_jsonb(rev+1));
 END IF;
 INSERT INTO bot_lifecycle.roles(project_id,role_id,state) VALUES(p->>'project_id',p->>'role_id',s)
   ON CONFLICT(project_id,role_id) DO UPDATE SET state=EXCLUDED.state;
 rec:=jsonb_build_object('request_id',p->>'request_id','request_digest',reqhash,'action',a,'state',s);
 INSERT INTO bot_lifecycle.events(project_id,role_id,request_id,request_digest,request,receipt)
   VALUES(p->>'project_id',p->>'role_id',p->>'request_id',reqhash,p,rec);
 RETURN jsonb_build_object('schema_version','lattice.bot-lifecycle.v1','status','APPLIED','receipt',rec,'current',s);
END $$;
REVOKE ALL ON ALL TABLES IN SCHEMA bot_lifecycle FROM PUBLIC,lattice_runtime;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA bot_lifecycle FROM PUBLIC;
GRANT EXECUTE ON FUNCTION bot_lifecycle.identity_read_v1(),bot_lifecycle.read_v1(text,text),bot_lifecycle.apply_v1(jsonb) TO lattice_runtime;
