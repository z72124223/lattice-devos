-- Explicit, atomic recovery only. Existing v1/v2 function bodies stay unchanged.
CREATE FUNCTION bot_lifecycle.reconcile_archive_v2(p jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog SET lock_timeout='5s' SET statement_timeout='30s' AS $$
DECLARE s jsonb; prior jsonb; anchor jsonb; b jsonb:=p->'body'; actor jsonb:=p->'actor';
 observed jsonb; original jsonb; h text; rec jsonb; k text; proof jsonb:=b->'history_proof';
BEGIN
 IF session_user<>'lattice_migrator_login' OR current_setting('role')<>'lattice_migrator'
 OR current_setting('transaction_isolation')<>'serializable' OR current_setting('transaction_read_only')::boolean
 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ROLE_REJECTED'; END IF;
 PERFORM bot_lifecycle.envelope_v2(p);
 PERFORM bot_lifecycle.keys_v1(p,ARRAY['action','request_id','project_id','role_id','expected_revision','expected_generation','owner_thread_id','owner_host_id','handoff_id','actor','native','body']);
 PERFORM bot_lifecycle.keys_v1(actor,ARRAY['role_id','thread_id','host_id','generation']);
 PERFORM bot_lifecycle.keys_v1(p->'native',ARRAY['old','new']);
 IF p->>'action' IS DISTINCT FROM 'reconcile-archived-boundary' OR p->>'role_id'<>'control'
 OR p->'native'->'new' IS DISTINCT FROM 'null'::jsonb THEN RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF;
 h:=encode(sha256(convert_to(p::text,'UTF8')),'hex');
 PERFORM pg_advisory_xact_lock(hashtextextended((p->>'project_id')||'/'||(p->>'role_id'),0));
 SELECT state INTO s FROM ONLY bot_lifecycle.roles WHERE project_id=p->>'project_id' AND role_id=p->>'role_id' FOR UPDATE;
 SELECT jsonb_build_object('digest',request_digest,'receipt',receipt) INTO prior FROM ONLY bot_lifecycle.events WHERE project_id=p->>'project_id' AND role_id=p->>'role_id' AND request_id=p->>'request_id';
 IF prior IS NOT NULL THEN
   IF prior->>'digest'<>h THEN RAISE EXCEPTION 'BOT_LIFECYCLE_IDEMPOTENCY_CONFLICT'; END IF;
   RETURN jsonb_build_object('schema_version','lattice.bot-lifecycle.v2','status','REPLAYED','receipt',prior->'receipt','current',s);
 END IF;
 IF s IS NULL THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ROLE_MISSING'; END IF;
 IF s->'generation' IS DISTINCT FROM p->'expected_generation' OR s->>'owner_thread_id'<>p->>'owner_thread_id' OR s->>'owner_host_id'<>p->>'owner_host_id'
 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_STALE_OWNER'; END IF;
 IF actor IS DISTINCT FROM jsonb_build_object('role_id','control','thread_id',s->>'owner_thread_id','host_id',s->>'owner_host_id','generation',s->'generation')
 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ACTOR_REJECTED'; END IF;
 IF s->'revision' IS DISTINCT FROM p->'expected_revision' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_REVISION_CONFLICT'; END IF;
 IF s->'contract_version' IS DISTINCT FROM '2'::jsonb OR s->>'phase'<>'MIGRATING' OR s->'archive_reconciliation'->>'handoff_id'=p->>'handoff_id'
 OR s->'handoff'->>'handoff_id' IS DISTINCT FROM p->>'handoff_id' OR s->'handoff'->>'successor_thread_id'<>s->>'owner_thread_id'
 OR s->'executor_grant'->>'status' IS DISTINCT FROM 'CONSUMED' OR s->'ack'->>'manifest_digest' IS DISTINCT FROM s->>'manifest_digest'
 OR s->'steps'->'routing_updated'->>'status' IS DISTINCT FROM 'COMPLETED' OR s->'steps'->'schedule_updated'->>'status' IS DISTINCT FROM 'COMPLETED'
 OR s->'steps'->'old_archived'->>'status' IS DISTINCT FROM 'RESERVED'
 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ARCHIVE_RECONCILE_REJECTED'; END IF;
 PERFORM bot_lifecycle.keys_v1(b,ARRAY['manifest_digest','anchor_request_id','pre_native_digest','archive_operation_key','archive_receipt','history_proof']);
 IF b->>'manifest_digest' IS DISTINCT FROM s->>'manifest_digest' OR b->>'archive_operation_key' IS DISTINCT FROM s->'steps'->'old_archived'->>'operation_key'
 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ARCHIVE_RECONCILE_REJECTED'; END IF;
 SELECT request INTO anchor FROM ONLY bot_lifecycle.events WHERE project_id=p->>'project_id' AND role_id=p->>'role_id' AND request_id=b->>'anchor_request_id';
 original:=s->'handoff_boundary';
 IF anchor IS NULL OR anchor->>'action'<>'commit' OR anchor->>'handoff_id'<>p->>'handoff_id'
 OR anchor->'body'->>'manifest_digest'<>s->>'manifest_digest'
 OR anchor->'native'->'old'->>'readback_digest' IS DISTINCT FROM b->>'pre_native_digest'
 OR anchor->'native'->'old'->>'latest_turn_id' IS DISTINCT FROM original->>'latest_turn_id'
 OR anchor->'native'->'old'->'thread_updated_at' IS DISTINCT FROM original->'thread_updated_at'
 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ARCHIVE_RECONCILE_REJECTED'; END IF;
 observed:=bot_lifecycle.boundary_v2(p->'native'->'old',s->'handoff'->>'old_thread_id',s->'handoff'->>'old_host_id');
 IF p->'native'->'old'->>'status'<>'notLoaded' OR observed->>'latest_turn_id'<>original->>'latest_turn_id'
 OR (observed->>'thread_updated_at')::bigint<1 OR (observed->>'thread_updated_at')::bigint>=(original->>'thread_updated_at')::bigint
 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED'; END IF;
 PERFORM bot_lifecycle.receipt_v1(b->'archive_receipt');
 IF b->'archive_receipt'->>'tool'<>'set_thread_archived' OR b->'archive_receipt'->>'target_thread_id'<>s->'handoff'->>'old_thread_id'
 OR b->'archive_receipt'->>'target_host_id'<>s->'handoff'->>'old_host_id' OR b->'archive_receipt'->'old_pending_count'<>'0'::jsonb
 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_NATIVE_TARGET_MISMATCH'; END IF;
 PERFORM bot_lifecycle.keys_v1(proof,ARRAY['pre_turn_digest','post_turn_digest','rollout_sha256','evidence_digest','archived','archived_at','durable_updated_at','latest_turn_id','new_input_count','in_flight_count']);
 FOREACH k IN ARRAY ARRAY['pre_turn_digest','post_turn_digest','rollout_sha256','evidence_digest'] LOOP
 IF COALESCE(proof->>k,'') !~ '^[a-f0-9]{64}$' THEN RAISE EXCEPTION 'BOT_LIFECYCLE_INPUT_REJECTED'; END IF; END LOOP;
 IF proof->>'pre_turn_digest'<>proof->>'post_turn_digest' OR proof->'archived' IS DISTINCT FROM 'true'::jsonb
 OR proof->>'latest_turn_id' IS DISTINCT FROM original->>'latest_turn_id'
 OR proof->'durable_updated_at' IS DISTINCT FROM observed->'thread_updated_at'
 OR proof->'new_input_count' IS DISTINCT FROM '0'::jsonb OR proof->'in_flight_count' IS DISTINCT FROM '0'::jsonb
 OR jsonb_typeof(proof->'archived_at') IS DISTINCT FROM 'number' OR COALESCE(proof->>'archived_at','') !~ '^[0-9]{1,15}$'
 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ARCHIVE_RECONCILE_REJECTED'; END IF;
 IF (proof->>'archived_at')::bigint<(original->>'thread_updated_at')::bigint OR (proof->>'archived_at')::bigint>extract(epoch FROM clock_timestamp())+5
 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_ARCHIVE_RECONCILE_REJECTED'; END IF;
 IF (s->>'revision')::bigint>=999999999999999 THEN RAISE EXCEPTION 'BOT_LIFECYCLE_LIMIT'; END IF;
 -- Retain the original boundary and all proof; only adopt the actually observed
 -- archived metadata. No owner/generation/work/step/authority changes occur.
 s:=s||jsonb_build_object('archive_reconciliation',b||jsonb_build_object('handoff_id',p->>'handoff_id','original_boundary',original,'observed_boundary',observed),
   'handoff_boundary',observed,'revision',(s->>'revision')::bigint+1);
 UPDATE bot_lifecycle.roles SET state=s WHERE project_id=p->>'project_id' AND role_id=p->>'role_id';
 rec:=jsonb_build_object('request_id',p->>'request_id','request_digest',h,'action',p->>'action','state',s);
 INSERT INTO bot_lifecycle.events(project_id,role_id,request_id,request_digest,request,receipt) VALUES(p->>'project_id',p->>'role_id',p->>'request_id',h,p,rec);
 RETURN jsonb_build_object('schema_version','lattice.bot-lifecycle.v2','status','APPLIED','receipt',rec,'current',s);
END $$;
REVOKE ALL ON FUNCTION bot_lifecycle.reconcile_archive_v2(jsonb) FROM PUBLIC,lattice_runtime;
