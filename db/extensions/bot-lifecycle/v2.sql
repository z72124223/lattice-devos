-- Additive extension: v1 SQL and function bodies remain byte-for-byte unchanged.
-- Installed only by the explicit, atomic bot-lifecycle-migrate transaction.
CREATE FUNCTION bot_lifecycle.envelope_v2(p jsonb) RETURNS void
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF octet_length(p::text)>65536
 OR COALESCE(p->>'project_id','') !~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'
 OR COALESCE(p->>'role_id','') !~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'
 OR COALESCE(p->>'request_id','') !~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$'
 OR COALESCE(p->>'owner_thread_id','') !~ '^[a-f0-9]{8}(-[a-f0-9]{4}){3}-[a-f0-9]{12}$'
 OR COALESCE(p->>'owner_host_id','') !~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'
 OR jsonb_typeof(p->'expected_revision') IS DISTINCT FROM 'number' OR COALESCE(p->>'expected_revision','') !~ '^[0-9]{1,15}$'
 OR jsonb_typeof(p->'expected_generation') IS DISTINCT FROM 'number' OR COALESCE(p->>'expected_generation','') !~ '^[0-9]{1,15}$'
 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF;
END $$;

CREATE FUNCTION bot_lifecycle.boundary_v2(p jsonb, target text, host text) RETURNS jsonb
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE observed timestamptz;
BEGIN
 PERFORM bot_lifecycle.keys_v1(p,ARRAY['source','observed_at','thread_id','host_id','latest_turn_id','thread_updated_at','status','latest_turn_status','pending_input_count','in_flight_count','readback_digest','evidence_ref']);
 IF p->>'source' IS DISTINCT FROM 'codex.read_thread' OR p->>'thread_id' IS DISTINCT FROM target OR p->>'host_id' IS DISTINCT FROM host
 OR p->>'status' IS DISTINCT FROM 'idle' OR p->>'latest_turn_status' IS DISTINCT FROM 'completed'
 OR p->'pending_input_count' IS DISTINCT FROM '0'::jsonb OR p->'in_flight_count' IS DISTINCT FROM '0'::jsonb
 OR COALESCE(p->>'latest_turn_id','') !~ '^[a-f0-9]{8}(-[a-f0-9]{4}){3}-[a-f0-9]{12}$'
 OR jsonb_typeof(p->'thread_updated_at') IS DISTINCT FROM 'number' OR COALESCE(p->>'thread_updated_at','') !~ '^[0-9]{1,15}$'
 OR COALESCE(p->>'readback_digest','') !~ '^[a-f0-9]{64}$'
 OR length(p->>'evidence_ref')>512 OR COALESCE(p->>'evidence_ref','') !~ '^[A-Za-z0-9][A-Za-z0-9._:/#-]*$'
 OR COALESCE(p->>'observed_at','') !~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z$'
 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED'; END IF;
 BEGIN observed:=(p->>'observed_at')::timestamptz;
 EXCEPTION WHEN OTHERS THEN RAISE EXCEPTION 'BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED'; END;
 IF observed<clock_timestamp()-interval '300 seconds' OR observed>clock_timestamp()+interval '5 seconds' THEN
 RAISE EXCEPTION 'BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED'; END IF;
 RETURN jsonb_build_object('latest_turn_id',p->>'latest_turn_id','thread_updated_at',p->'thread_updated_at');
END $$;

CREATE FUNCTION bot_lifecycle.migrate_v2(p jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog SET lock_timeout='5s' SET statement_timeout='30s' AS $$
DECLARE s jsonb; prior jsonb; b jsonb:=p->'body'; h text; rec jsonb; k text;
BEGIN
 IF session_user<>'lattice_migrator_login' OR current_setting('role')<>'lattice_migrator'
 OR current_setting('transaction_isolation')<>'serializable' OR current_setting('transaction_read_only')::boolean THEN
 RAISE EXCEPTION 'BOT_LIFECYCLE_ROLE_REJECTED'; END IF;
 PERFORM bot_lifecycle.keys_v1(p,ARRAY['action','request_id','project_id','role_id','expected_revision','expected_generation','owner_thread_id','owner_host_id','body']);
 PERFORM bot_lifecycle.envelope_v2(p);
 IF p->>'action' IS DISTINCT FROM 'migrate-contract' OR p->>'role_id'<>'control' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF;
 h:=encode(sha256(convert_to(p::text,'UTF8')),'hex');
 PERFORM pg_advisory_xact_lock(hashtextextended((p->>'project_id')||'/'||(p->>'role_id'),0));
 SELECT state INTO s FROM ONLY bot_lifecycle.roles WHERE project_id=p->>'project_id' AND role_id=p->>'role_id' FOR UPDATE;
 SELECT jsonb_build_object('digest',request_digest,'receipt',receipt) INTO prior FROM ONLY bot_lifecycle.events WHERE project_id=p->>'project_id' AND role_id=p->>'role_id' AND request_id=p->>'request_id';
 IF prior IS NOT NULL THEN
   IF prior->>'digest'<>h THEN RAISE EXCEPTION 'BOT_LIFECYCLE_IDEMPOTENCY_CONFLICT'; END IF;
   RETURN jsonb_build_object('schema_version','lattice.bot-lifecycle.v2','status','REPLAYED','receipt',prior->'receipt','current',s);
 END IF;
 PERFORM bot_lifecycle.keys_v1(b,ARRAY['from_version','to_version','expected_policy_digest','expected_rules_digest','policy_digest','rules_digest','approval_digest','authorization_receipt']);
 IF s IS NULL THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ROLE_MISSING'; END IF;
 IF s->>'generation'<>p->>'expected_generation' OR s->>'owner_thread_id'<>p->>'owner_thread_id' OR s->>'owner_host_id'<>p->>'owner_host_id' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_STALE_OWNER'; END IF;
 IF s->>'revision'<>p->>'expected_revision' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_REVISION_CONFLICT'; END IF;
 IF b->'from_version' IS DISTINCT FROM '1'::jsonb OR b->'to_version' IS DISTINCT FROM '2'::jsonb
 OR s ? 'contract_version' OR s->>'phase'<>'ACTIVE' OR s->'handoff' IS DISTINCT FROM 'null'::jsonb
 OR s->'manifest' IS DISTINCT FROM 'null'::jsonb OR s->'ack' IS DISTINCT FROM 'null'::jsonb OR s->'steps'<>'{}'::jsonb
 OR s->>'policy_digest' IS DISTINCT FROM b->>'expected_policy_digest' OR s->>'rules_digest' IS DISTINCT FROM b->>'expected_rules_digest'
 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_CONTRACT_MIGRATION_REJECTED'; END IF;
 FOREACH k IN ARRAY ARRAY['expected_policy_digest','expected_rules_digest','policy_digest','rules_digest','approval_digest'] LOOP
 IF COALESCE(b->>k,'') !~ '^[a-f0-9]{64}$' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF; END LOOP;
 PERFORM bot_lifecycle.receipt_v1(b->'authorization_receipt');
 IF b->'authorization_receipt'->>'tool'<>'read_thread' OR b->'authorization_receipt'->>'target_thread_id'<>s->>'owner_thread_id'
 OR b->'authorization_receipt'->>'target_host_id'<>s->>'owner_host_id' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_NATIVE_TARGET_MISMATCH'; END IF;
 IF (s->>'revision')::bigint>=999999999999999 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_LIMIT'; END IF;
 s:=s||jsonb_build_object('contract_version',2,'contract_migration',b,'policy_digest',b->>'policy_digest','rules_digest',b->>'rules_digest','revision',(s->>'revision')::bigint+1);
 UPDATE bot_lifecycle.roles SET state=s WHERE project_id=p->>'project_id' AND role_id=p->>'role_id';
 rec:=jsonb_build_object('request_id',p->>'request_id','request_digest',h,'action',p->>'action','state',s);
 INSERT INTO bot_lifecycle.events(project_id,role_id,request_id,request_digest,request,receipt) VALUES(p->>'project_id',p->>'role_id',p->>'request_id',h,p,rec);
 RETURN jsonb_build_object('schema_version','lattice.bot-lifecycle.v2','status','APPLIED','receipt',rec,'current',s);
END $$;

CREATE FUNCTION bot_lifecycle.apply_v2(p jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog SET lock_timeout='5s' SET statement_timeout='30s' AS $$
DECLARE s jsonb; prior jsonb; actor jsonb:=p->'actor'; actor_state jsonb; b jsonb:=p->'body'; a text:=p->>'action';
 h text; rec jsonb; g jsonb; boundary jsonb; next_boundary jsonb; result jsonb; inner_request jsonb; expires timestamptz; custom boolean:=false;
BEGIN
 IF session_user<>'lattice_runtime_login' OR current_setting('role')<>'lattice_runtime'
 OR current_setting('transaction_isolation')<>'serializable' OR current_setting('transaction_read_only')::boolean THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ROLE_REJECTED'; END IF;
 PERFORM bot_lifecycle.envelope_v2(p);
 PERFORM pg_advisory_xact_lock(hashtextextended((p->>'project_id')||'/'||(p->>'role_id'),0));
 SELECT state INTO s FROM ONLY bot_lifecycle.roles WHERE project_id=p->>'project_id' AND role_id=p->>'role_id' FOR UPDATE;
 -- Legacy roles retain the original request and state machine.
 IF s->'contract_version' IS DISTINCT FROM '2'::jsonb THEN RETURN bot_lifecycle.apply_v1(p); END IF;
 h:=encode(sha256(convert_to(p::text,'UTF8')),'hex');
 SELECT jsonb_build_object('digest',request_digest,'receipt',receipt) INTO prior FROM ONLY bot_lifecycle.events WHERE project_id=p->>'project_id' AND role_id=p->>'role_id' AND request_id=p->>'request_id';
 IF prior IS NOT NULL THEN
 IF prior->>'digest'<>h THEN RAISE EXCEPTION 'BOT_LIFECYCLE_IDEMPOTENCY_CONFLICT'; END IF;
 RETURN jsonb_build_object('schema_version','lattice.bot-lifecycle.v2','status','REPLAYED','receipt',prior->'receipt','current',s); END IF;
 PERFORM bot_lifecycle.keys_v1(p,ARRAY['action','request_id','project_id','role_id','expected_revision','expected_generation','owner_thread_id','owner_host_id','body','actor','handoff_id','native']);
 PERFORM bot_lifecycle.keys_v1(actor,ARRAY['role_id','thread_id','host_id','generation']);
 PERFORM bot_lifecycle.keys_v1(p->'native',ARRAY['old','new']);
 IF p->>'role_id'<>'control' OR COALESCE(p->>'handoff_id','') !~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$'
 OR jsonb_typeof(actor->'generation') IS DISTINCT FROM 'number' OR COALESCE(actor->>'generation','') !~ '^[0-9]{1,15}$' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF;
 IF s->>'generation'<>p->>'expected_generation' OR s->>'owner_thread_id'<>p->>'owner_thread_id' OR s->>'owner_host_id'<>p->>'owner_host_id' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_STALE_OWNER'; END IF;
 IF s->>'revision'<>p->>'expected_revision' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_REVISION_CONFLICT'; END IF;
 -- Hold actor ownership through the control transaction's commit. A plain MVCC
 -- snapshot could authorize an actor whose own replacement already committed.
 SELECT state INTO actor_state FROM ONLY bot_lifecycle.roles WHERE project_id=p->>'project_id' AND role_id=actor->>'role_id' FOR SHARE;
 IF actor_state IS NULL OR actor_state->>'owner_thread_id' IS DISTINCT FROM actor->>'thread_id'
 OR actor_state->>'owner_host_id' IS DISTINCT FROM actor->>'host_id' OR actor_state->'generation' IS DISTINCT FROM actor->'generation' THEN
 RAISE EXCEPTION 'BOT_LIFECYCLE_ACTOR_REJECTED'; END IF;
 g:=s->'executor_grant';
 IF a IN('authorize-executor','revoke-executor','abort') THEN
 IF actor->>'role_id'<>'control' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ACTOR_REJECTED'; END IF;
 IF p->'native'->'old' IS DISTINCT FROM 'null'::jsonb OR p->'native'->'new' IS DISTINCT FROM 'null'::jsonb THEN RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF;
 IF a='authorize-executor' THEN
 custom:=true;
 PERFORM bot_lifecycle.keys_v1(b,ARRAY['executor_role_id','executor_thread_id','executor_host_id','executor_generation','checkpoint_turn_id','expires_at','authorization_receipt']);
 IF s->>'phase'<>'ACTIVE' OR g->>'status'='ACTIVE' OR b->>'executor_role_id' IS DISTINCT FROM 'lattice_maintenance'
 OR COALESCE(b->>'checkpoint_turn_id','') !~ '^[a-f0-9]{8}(-[a-f0-9]{4}){3}-[a-f0-9]{12}$'
 OR jsonb_typeof(b->'executor_generation') IS DISTINCT FROM 'number'
 OR COALESCE(b->>'expires_at','') !~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z$' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_EXECUTOR_GRANT_REJECTED'; END IF;
 SELECT state INTO actor_state FROM ONLY bot_lifecycle.roles WHERE project_id=p->>'project_id' AND role_id=b->>'executor_role_id' FOR SHARE;
 IF actor_state IS NULL OR actor_state->>'phase'<>'ACTIVE' OR actor_state->>'owner_thread_id' IS DISTINCT FROM b->>'executor_thread_id'
 OR actor_state->>'owner_host_id' IS DISTINCT FROM b->>'executor_host_id' OR actor_state->'generation' IS DISTINCT FROM b->'executor_generation'
 OR b->>'executor_thread_id'=s->>'owner_thread_id' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ACTOR_REJECTED'; END IF;
 BEGIN expires:=(b->>'expires_at')::timestamptz; EXCEPTION WHEN OTHERS THEN RAISE EXCEPTION 'BOT_LIFECYCLE_EXECUTOR_GRANT_REJECTED'; END;
 IF expires<=clock_timestamp() OR expires>clock_timestamp()+interval '24 hours' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_EXECUTOR_GRANT_REJECTED'; END IF;
 PERFORM bot_lifecycle.receipt_v1(b->'authorization_receipt');
 IF b->'authorization_receipt'->>'tool'<>'read_thread' OR b->'authorization_receipt'->>'target_thread_id'<>s->>'owner_thread_id'
 OR b->'authorization_receipt'->>'target_host_id'<>s->>'owner_host_id' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_NATIVE_TARGET_MISMATCH'; END IF;
 s:=s||jsonb_build_object('executor_grant',b||jsonb_build_object('status','ACTIVE','handoff_id',p->>'handoff_id','owner_thread_id',s->>'owner_thread_id','owner_host_id',s->>'owner_host_id','owner_generation',s->'generation','rules_digest',s->>'rules_digest','policy_digest',s->>'policy_digest'),'handoff_boundary',NULL,'successor_boundary',NULL);
 ELSIF a='revoke-executor' THEN
 custom:=true;
 PERFORM bot_lifecycle.keys_v1(b,ARRAY['reason_digest']);
 IF s->>'phase'<>'ACTIVE' OR g->>'status' IS DISTINCT FROM 'ACTIVE' OR g->>'handoff_id' IS DISTINCT FROM p->>'handoff_id'
 OR COALESCE(b->>'reason_digest','') !~ '^[a-f0-9]{64}$' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_EXECUTOR_GRANT_REJECTED'; END IF;
 s:=jsonb_set(s,'{executor_grant}',g||jsonb_build_object('status','REVOKED','reason_digest',b->>'reason_digest'));
 ELSE
 IF s->>'phase' NOT IN('PREPARING','VERIFIED') OR s->'handoff'->>'handoff_id' IS DISTINCT FROM p->>'handoff_id' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_PHASE_REJECTED'; END IF;
 END IF;
 ELSIF s->>'phase'='MIGRATING' THEN
 IF actor->>'role_id'<>'control' OR s->'handoff'->>'handoff_id' IS DISTINCT FROM p->>'handoff_id'
 OR a NOT IN('reserve-step','native-step','finish') OR (a<>'finish' AND COALESCE(b->>'step','') NOT IN('routing_updated','schedule_updated','old_archived'))
 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ACTOR_REJECTED'; END IF;
 boundary:=bot_lifecycle.boundary_v2(p->'native'->'old',s->'handoff'->>'old_thread_id',s->'handoff'->>'old_host_id');
 IF boundary IS DISTINCT FROM s->'handoff_boundary' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED'; END IF;
 IF p->'native'->'new' IS DISTINCT FROM 'null'::jsonb THEN RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF;
 ELSE
 IF actor_state->>'phase'<>'ACTIVE' OR g->>'status' IS DISTINCT FROM 'ACTIVE'
 OR g->>'handoff_id' IS DISTINCT FROM p->>'handoff_id' OR g->>'executor_role_id' IS DISTINCT FROM actor->>'role_id'
 OR g->>'executor_thread_id' IS DISTINCT FROM actor->>'thread_id' OR g->>'executor_host_id' IS DISTINCT FROM actor->>'host_id' OR g->'executor_generation' IS DISTINCT FROM actor->'generation'
 OR g->>'owner_thread_id' IS DISTINCT FROM s->>'owner_thread_id' OR g->'owner_generation' IS DISTINCT FROM s->'generation'
 OR g->>'rules_digest' IS DISTINCT FROM s->>'rules_digest' OR g->>'policy_digest' IS DISTINCT FROM s->>'policy_digest'
 OR (g->>'expires_at')::timestamptz<=clock_timestamp()
 OR a NOT IN('add-work','checkpoint','prepare','reserve-step','native-step','ack','commit')
 OR (a IN('reserve-step','native-step') AND b->>'step' IS DISTINCT FROM 'successor_created') THEN RAISE EXCEPTION 'BOT_LIFECYCLE_EXECUTOR_GRANT_REJECTED'; END IF;
 boundary:=bot_lifecycle.boundary_v2(p->'native'->'old',s->>'owner_thread_id',s->>'owner_host_id');
 IF boundary->>'latest_turn_id'<>g->>'checkpoint_turn_id'
 OR (s->'handoff_boundary' IS NOT NULL AND s->'handoff_boundary'<>'null'::jsonb AND boundary<>s->'handoff_boundary') THEN RAISE EXCEPTION 'BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED'; END IF;
 s:=s||jsonb_build_object('handoff_boundary',boundary);
 IF a='prepare' AND b->>'handoff_id' IS DISTINCT FROM p->>'handoff_id' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_EXECUTOR_GRANT_REJECTED'; END IF;
 IF a IN('checkpoint','prepare') AND (b->'manifest'->'pending_input_ids' IS DISTINCT FROM '[]'::jsonb OR b->'manifest'->'in_flight_ids' IS DISTINCT FROM '[]'::jsonb) THEN RAISE EXCEPTION 'BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED'; END IF;
 IF a IN('ack','commit') THEN
 next_boundary:=bot_lifecycle.boundary_v2(p->'native'->'new',s->'handoff'->>'successor_thread_id',s->'handoff'->>'successor_host_id');
 IF a='commit' AND next_boundary IS DISTINCT FROM s->'successor_boundary' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED'; END IF;
 s:=s||jsonb_build_object('successor_boundary',next_boundary);
 ELSIF p->'native'->'new' IS DISTINCT FROM 'null'::jsonb THEN RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF;
 END IF;
 IF custom THEN
 IF (s->>'revision')::bigint>=999999999999999 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_LIMIT'; END IF;
 s:=jsonb_set(s,'{revision}',to_jsonb((s->>'revision')::bigint+1));
 ELSE
 UPDATE bot_lifecycle.roles SET state=s WHERE project_id=p->>'project_id' AND role_id=p->>'role_id';
 inner_request:=p-'actor'-'handoff_id'-'native';
 result:=bot_lifecycle.apply_v1(inner_request); s:=result->'current';
 IF a='commit' THEN s:=jsonb_set(s,'{executor_grant,status}','"CONSUMED"'::jsonb);
 ELSIF a='abort' THEN s:=jsonb_set(s,'{executor_grant,status}','"REVOKED"'::jsonb); END IF;
 END IF;
 UPDATE bot_lifecycle.roles SET state=s WHERE project_id=p->>'project_id' AND role_id=p->>'role_id';
 rec:=jsonb_build_object('request_id',p->>'request_id','request_digest',h,'action',a,'state',s);
 -- For delegated v1 transitions replace only the event just created inside this
 -- same transaction; earlier immutable events were handled by the replay branch.
 INSERT INTO bot_lifecycle.events(project_id,role_id,request_id,request_digest,request,receipt) VALUES(p->>'project_id',p->>'role_id',p->>'request_id',h,p,rec)
 ON CONFLICT(project_id,role_id,request_id) DO UPDATE SET request_digest=EXCLUDED.request_digest,request=EXCLUDED.request,receipt=EXCLUDED.receipt;
 RETURN jsonb_build_object('schema_version','lattice.bot-lifecycle.v2','status','APPLIED','receipt',rec,'current',s);
END $$;

REVOKE ALL ON FUNCTION bot_lifecycle.envelope_v2(jsonb),bot_lifecycle.boundary_v2(jsonb,text,text),bot_lifecycle.migrate_v2(jsonb),bot_lifecycle.apply_v2(jsonb) FROM PUBLIC,lattice_runtime;
REVOKE EXECUTE ON FUNCTION bot_lifecycle.apply_v1(jsonb) FROM lattice_runtime;
GRANT EXECUTE ON FUNCTION bot_lifecycle.apply_v2(jsonb) TO lattice_runtime;
