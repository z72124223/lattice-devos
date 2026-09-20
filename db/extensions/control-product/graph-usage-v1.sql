-- Per-call observations. These do not alter analysis receipts or task completion.
-- Both records are append-only to runtime: only the fixed functions have write access.
CREATE TABLE control_product.graph_usage_starts (
    usage_id text PRIMARY KEY CHECK (usage_id ~ '^[a-f0-9]{64}$'),
    project_id text NOT NULL CHECK (project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'),
    task_ref text CHECK (task_ref ~ '^[a-f0-9]{64}$'),
    payload jsonb NOT NULL CHECK (jsonb_typeof(payload)='object' AND octet_length(payload::text)<=4096),
    payload_digest bytea NOT NULL CHECK (octet_length(payload_digest)=32),
    recorded_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX graph_usage_starts_scope ON control_product.graph_usage_starts(project_id,task_ref,recorded_at DESC,usage_id);
CREATE TABLE control_product.graph_usage_finishes (
    usage_id text PRIMARY KEY REFERENCES control_product.graph_usage_starts(usage_id),
    payload jsonb NOT NULL CHECK (jsonb_typeof(payload)='object' AND octet_length(payload::text)<=4096),
    payload_digest bytea NOT NULL CHECK (octet_length(payload_digest)=32),
    recorded_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
REVOKE ALL ON TABLE control_product.graph_usage_starts,control_product.graph_usage_finishes
    FROM PUBLIC,lattice_runtime,lattice_guardian,lattice_readonly;

CREATE FUNCTION control_product.graph_usage_begin_v1(p jsonb) RETURNS text
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog
AS $graph_usage_begin_v1$
DECLARE written bigint;
BEGIN
    IF session_user <> 'lattice_runtime_login' OR current_setting('role') <> 'lattice_runtime' THEN
        RAISE EXCEPTION 'GRAPH_USAGE_CONTEXT_REJECTED';
    END IF;
    IF p IS NULL OR jsonb_typeof(p) IS DISTINCT FROM 'object' OR octet_length(p::text)>4096 THEN
        RAISE EXCEPTION 'GRAPH_USAGE_ARGUMENTS_REJECTED';
    END IF;
    IF (SELECT count(*) FROM jsonb_object_keys(p))<>7
       OR p-ARRAY['usage_id','project_id','task_ref','commit','operation','integration_mode','query_digest']<>'{}'::jsonb
       OR jsonb_typeof(p->'usage_id') IS DISTINCT FROM 'string' OR p->>'usage_id' !~ '^[a-f0-9]{64}$'
       OR jsonb_typeof(p->'project_id') IS DISTINCT FROM 'string' OR p->>'project_id' !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
       OR (p->'task_ref'<>'null'::jsonb AND (jsonb_typeof(p->'task_ref')<>'string' OR p->>'task_ref' !~ '^[a-f0-9]{64}$'))
       OR (p->'commit'<>'null'::jsonb AND (jsonb_typeof(p->'commit')<>'string' OR p->>'commit' !~ '^([a-f0-9]{40}|[a-f0-9]{64})$'))
       OR jsonb_typeof(p->'operation') IS DISTINCT FROM 'string' OR p->>'operation' NOT IN ('QUERY','REFRESH')
       OR jsonb_typeof(p->'integration_mode') IS DISTINCT FROM 'string' OR p->>'integration_mode' NOT IN ('CORE_ONLY','GRAPHIFY')
       OR (p->'query_digest'<>'null'::jsonb AND (jsonb_typeof(p->'query_digest')<>'string' OR p->>'query_digest' !~ '^[a-f0-9]{64}$'))
       OR (p->>'operation'='QUERY' AND (p->'commit'='null'::jsonb OR p->'query_digest'='null'::jsonb)) THEN
        RAISE EXCEPTION 'GRAPH_USAGE_ARGUMENTS_REJECTED';
    END IF;
    -- Unbound refresh source identities need not be Registry project IDs.
    IF p->>'task_ref' IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM ONLY control.task_submission_envelopes e
        JOIN ONLY control.task_ledger_streams s ON s.stream_id=e.stream_id
        WHERE e.task_ref=p->>'task_ref' AND e.project_id=p->>'project_id'
          AND s.task_subject_kind='GENERAL_TASK_INTAKE'
    ) THEN RAISE EXCEPTION 'GRAPH_USAGE_TASK_BINDING_REJECTED'; END IF;
    INSERT INTO control_product.graph_usage_starts(usage_id,project_id,task_ref,payload,payload_digest)
    VALUES(p->>'usage_id',p->>'project_id',p->>'task_ref',p,
        sha256(convert_to('LATTICE_GRAPH_USAGE_START_V1'||chr(10)||p::text,'UTF8')))
    ON CONFLICT(usage_id) DO NOTHING;
    GET DIAGNOSTICS written=ROW_COUNT;
    IF written=1 THEN RETURN 'RECORDED'; END IF;
    IF NOT EXISTS(SELECT 1 FROM ONLY control_product.graph_usage_starts WHERE usage_id=p->>'usage_id' AND payload=p) THEN
        RAISE EXCEPTION 'GRAPH_USAGE_IDEMPOTENCY_CONFLICT';
    END IF;
    RETURN 'REPLAYED';
END
$graph_usage_begin_v1$;

CREATE FUNCTION control_product.graph_usage_finish_v1(p jsonb) RETURNS text
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog
AS $graph_usage_finish_v1$
DECLARE started jsonb; written bigint;
BEGIN
    IF session_user <> 'lattice_runtime_login' OR current_setting('role') <> 'lattice_runtime' THEN
        RAISE EXCEPTION 'GRAPH_USAGE_CONTEXT_REJECTED';
    END IF;
    IF p IS NULL OR jsonb_typeof(p) IS DISTINCT FROM 'object' OR octet_length(p::text)>4096 THEN
        RAISE EXCEPTION 'GRAPH_USAGE_ARGUMENTS_REJECTED';
    END IF;
    IF (SELECT count(*) FROM jsonb_object_keys(p))<>9
       OR p-ARRAY['usage_id','outcome','source_receipt_digest','record_count','result_bytes','duration_ms','error_code','analysis_calls','query_calls']<>'{}'::jsonb
       OR jsonb_typeof(p->'usage_id') IS DISTINCT FROM 'string' OR p->>'usage_id' !~ '^[a-f0-9]{64}$'
       OR jsonb_typeof(p->'outcome') IS DISTINCT FROM 'string' OR p->>'outcome' NOT IN ('QUERIED','ANALYZED','REUSED','FAILED')
       OR (p->'source_receipt_digest'<>'null'::jsonb AND (jsonb_typeof(p->'source_receipt_digest')<>'string' OR p->>'source_receipt_digest' !~ '^[a-f0-9]{64}$'))
       OR (p->'record_count'<>'null'::jsonb AND (jsonb_typeof(p->'record_count')<>'number' OR p->>'record_count' !~ '^[0-9]{1,6}$'))
       OR (p->'result_bytes'<>'null'::jsonb AND (jsonb_typeof(p->'result_bytes')<>'number' OR p->>'result_bytes' !~ '^[0-9]{1,8}$'))
       OR jsonb_typeof(p->'duration_ms') IS DISTINCT FROM 'number' OR p->>'duration_ms' !~ '^[0-9]{1,10}$'
       OR jsonb_typeof(p->'analysis_calls') IS DISTINCT FROM 'number' OR p->>'analysis_calls' !~ '^[0-9]{1,7}$'
       OR jsonb_typeof(p->'query_calls') IS DISTINCT FROM 'number' OR p->>'query_calls' !~ '^[01]$'
       OR (p->'error_code'<>'null'::jsonb AND (jsonb_typeof(p->'error_code')<>'string' OR p->>'error_code' !~ '^[A-Z][A-Z0-9_]{0,95}$')) THEN
        RAISE EXCEPTION 'GRAPH_USAGE_ARGUMENTS_REJECTED';
    END IF;
    IF (p->>'record_count')::bigint>100000 OR (p->>'result_bytes')::bigint>16777216
       OR (p->>'duration_ms')::bigint>2592000000 OR (p->>'analysis_calls')::bigint>1000000
       OR (p->>'outcome'='ANALYZED' AND (p->>'analysis_calls')::bigint=0)
       OR (p->>'outcome' IN ('REUSED','QUERIED') AND (p->>'analysis_calls')::bigint<>0)
       OR (p->>'outcome'='QUERIED' AND (p->>'query_calls')::bigint<>1)
       OR (p->>'outcome'='FAILED' AND p->'error_code'='null'::jsonb)
       OR (p->>'outcome'<>'FAILED' AND (p->'error_code'<>'null'::jsonb OR p->'source_receipt_digest'='null'::jsonb
           OR p->'record_count'='null'::jsonb OR p->'result_bytes'='null'::jsonb)) THEN
        RAISE EXCEPTION 'GRAPH_USAGE_ARGUMENTS_REJECTED';
    END IF;
    SELECT payload INTO started FROM ONLY control_product.graph_usage_starts WHERE usage_id=p->>'usage_id';
    IF NOT FOUND THEN RAISE EXCEPTION 'GRAPH_USAGE_START_REQUIRED'; END IF;
    IF (started->>'operation'='QUERY' AND p->>'outcome' NOT IN ('QUERIED','FAILED'))
       OR (started->>'operation'='REFRESH' AND p->>'outcome' NOT IN ('ANALYZED','REUSED','FAILED'))
       OR (started->>'operation'='QUERY' AND (p->>'analysis_calls')::bigint<>0)
       OR (started->>'operation'='REFRESH' AND (p->>'query_calls')::bigint<>0) THEN
        RAISE EXCEPTION 'GRAPH_USAGE_OUTCOME_REJECTED';
    END IF;
    INSERT INTO control_product.graph_usage_finishes(usage_id,payload,payload_digest)
    VALUES(p->>'usage_id',p,sha256(convert_to('LATTICE_GRAPH_USAGE_FINISH_V1'||chr(10)||p::text,'UTF8')))
    ON CONFLICT(usage_id) DO NOTHING;
    GET DIAGNOSTICS written=ROW_COUNT;
    IF written=1 THEN RETURN 'RECORDED'; END IF;
    IF NOT EXISTS(SELECT 1 FROM ONLY control_product.graph_usage_finishes WHERE usage_id=p->>'usage_id' AND payload=p) THEN
        RAISE EXCEPTION 'GRAPH_USAGE_IDEMPOTENCY_CONFLICT';
    END IF;
    RETURN 'REPLAYED';
END
$graph_usage_finish_v1$;

CREATE FUNCTION control_product.graph_usage_summary_v1(p_project text,p_task text) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog
AS $graph_usage_summary_v1$
DECLARE result jsonb;
BEGIN
    IF session_user <> 'lattice_runtime_login' OR current_setting('role') <> 'lattice_runtime'
       OR current_setting('transaction_read_only')<>'on' THEN
        RAISE EXCEPTION 'GRAPH_USAGE_CONTEXT_REJECTED';
    END IF;
    IF p_project IS NULL OR p_project !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
       OR (p_task IS NOT NULL AND p_task !~ '^[a-f0-9]{64}$') THEN
        RAISE EXCEPTION 'GRAPH_USAGE_ARGUMENTS_REJECTED';
    END IF;
    IF p_task IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM ONLY control.task_submission_envelopes e
        JOIN ONLY control.task_ledger_streams s ON s.stream_id=e.stream_id
        WHERE e.task_ref=p_task AND e.project_id=p_project AND s.task_subject_kind='GENERAL_TASK_INTAKE'
    ) THEN RAISE EXCEPTION 'GRAPH_USAGE_TASK_BINDING_REJECTED'; END IF;
    WITH scoped AS MATERIALIZED (
        SELECT s.usage_id,s.task_ref,s.payload AS start_payload,s.payload_digest AS start_digest,
            s.recorded_at AS started_at,f.payload AS finish_payload,f.payload_digest AS finish_digest,f.recorded_at AS finished_at
        FROM ONLY control_product.graph_usage_starts s LEFT JOIN ONLY control_product.graph_usage_finishes f USING(usage_id)
        WHERE s.project_id=p_project AND (p_task IS NULL OR s.task_ref=p_task)
    ), totals AS (
        SELECT count(*) AS started,count(finish_payload) AS finished,
            count(*) FILTER(WHERE finish_payload IS NULL) AS pending,
            count(*) FILTER(WHERE finish_payload->>'outcome'='QUERIED') AS queried,
            count(*) FILTER(WHERE finish_payload->>'outcome'='ANALYZED') AS analyzed,
            count(*) FILTER(WHERE finish_payload->>'outcome'='REUSED') AS reused,
            count(*) FILTER(WHERE finish_payload->>'outcome'='FAILED') AS failed,
            coalesce(sum((finish_payload->>'analysis_calls')::bigint),0)::bigint AS analysis_calls,
            coalesce(sum((finish_payload->>'query_calls')::bigint),0)::bigint AS query_calls,
            coalesce(sum((finish_payload->>'record_count')::bigint) FILTER(WHERE finish_payload->>'outcome'='QUERIED'),0)::bigint AS query_records,
            coalesce(sum((finish_payload->>'result_bytes')::bigint),0)::bigint AS result_bytes,
            count(finish_payload->>'result_bytes') AS measured_results,
            coalesce(sum((finish_payload->>'duration_ms')::bigint),0)::bigint AS duration_ms FROM scoped
    ), recent AS (SELECT * FROM scoped ORDER BY started_at DESC,usage_id DESC LIMIT 20)
    SELECT jsonb_build_object('schema_version','lattice.graph-usage.v1','project_id',p_project,'task_ref',p_task,
        'scope',CASE WHEN p_task IS NULL THEN 'PROJECT' ELSE 'TASK' END,
        'coverage',CASE WHEN t.started=0 THEN 'UNKNOWN' WHEN t.pending>0 THEN 'INCOMPLETE' ELSE 'OBSERVED_CALLS_ONLY' END,
        'counts',to_jsonb(t),'result_bytes_scope','INNER_RESULT_JSON_UTF8',
        'recent',coalesce((SELECT jsonb_agg(jsonb_build_object(
            'usage_id',r.usage_id,'task_binding',CASE WHEN r.task_ref IS NULL THEN 'UNBOUND' ELSE 'TASK_BOUND' END,
            'start',r.start_payload,'finish',r.finish_payload,'started_at',r.started_at,'finished_at',r.finished_at,
            'start_digest',encode(r.start_digest,'hex'),'finish_digest',encode(r.finish_digest,'hex'))
            ORDER BY r.started_at DESC,r.usage_id DESC) FROM recent r),'[]'::jsonb)) INTO result FROM totals t;
    RETURN result;
END
$graph_usage_summary_v1$;
REVOKE ALL ON FUNCTION control_product.graph_usage_begin_v1(jsonb),control_product.graph_usage_finish_v1(jsonb),
    control_product.graph_usage_summary_v1(text,text) FROM PUBLIC,lattice_runtime,lattice_guardian,lattice_readonly;
GRANT EXECUTE ON FUNCTION control_product.graph_usage_begin_v1(jsonb),control_product.graph_usage_finish_v1(jsonb),
    control_product.graph_usage_summary_v1(text,text) TO lattice_runtime;
