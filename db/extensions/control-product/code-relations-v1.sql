-- Additive read projection; never changes Memory receipts or retrieval audits.
CREATE FUNCTION control_product.code_relations_v1(
    p_analysis text, p_project text, p_snapshot text, p_commit text,
    p_persistence text, p_query text, p_limit integer
) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER
SET search_path = pg_catalog
AS $code_relations_v1$
DECLARE
    result jsonb;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_read_only') <> 'on'
       OR pg_catalog.current_setting('transaction_isolation') NOT IN ('repeatable read','serializable') THEN
        RAISE EXCEPTION 'CODE_RELATIONS_READ_CONTEXT_REJECTED';
    END IF;
    IF p_analysis IS NULL OR p_analysis !~ '^[0-9a-f]{64}$'
       OR p_persistence IS NULL OR p_persistence !~ '^[0-9a-f]{64}$'
       OR p_project IS NULL OR length(p_project) NOT BETWEEN 1 AND 64
       OR p_snapshot IS NULL OR octet_length(p_snapshot) NOT BETWEEN 1 AND 1024
       OR p_commit IS NULL OR p_commit !~ '^([0-9a-f]{40}|[0-9a-f]{64})$'
       OR p_query IS NULL OR octet_length(p_query) NOT BETWEEN 1 AND 512
       OR length(p_query) > 128 OR p_query ~ '[[:cntrl:]]' OR btrim(p_query) <> p_query
       OR p_limit IS NULL OR p_limit NOT BETWEEN 1 AND 32 THEN
        RAISE EXCEPTION 'CODE_RELATIONS_ARGUMENTS_REJECTED';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM ONLY memory.codebase_memory_analyses a
        WHERE a.analysis_digest=decode(p_analysis,'hex')
          AND a.project_id=p_project AND a.project_snapshot_id=p_snapshot
          AND a.commit_id=p_commit AND a.persistence_digest=decode(p_persistence,'hex')
    ) THEN
        RAISE EXCEPTION 'CODE_RELATIONS_SOURCE_REJECTED';
    END IF;
    WITH matches AS MATERIALIZED (
        SELECT r.* FROM ONLY memory.codebase_memory_records r
        WHERE r.analysis_digest=decode(p_analysis,'hex')
          AND (strpos(lower(r.subject),lower(p_query))>0
            OR strpos(lower(coalesce(r.object,'')),lower(p_query))>0
            OR strpos(lower(coalesce(r.relation,'')),lower(p_query))>0
            OR strpos(lower(r.source_path),lower(p_query))>0)
        ORDER BY r.ordinal, r.record_id LIMIT p_limit+1
    ), page AS (
        SELECT * FROM matches ORDER BY ordinal,record_id LIMIT p_limit
    )
    SELECT jsonb_build_object(
        'record_set_proof',(SELECT jsonb_agg(jsonb_build_object(
            'ordinal',r.ordinal::text,'id',encode(r.record_id,'hex'),
            'content',encode(r.content_digest,'hex')) ORDER BY r.ordinal)
            FROM (SELECT ordinal,record_id,content_digest FROM ONLY memory.codebase_memory_records
                  WHERE analysis_digest=decode(p_analysis,'hex') ORDER BY ordinal LIMIT 100001) r),
        'records',coalesce((SELECT jsonb_agg(jsonb_build_object(
            'ordinal',r.ordinal,'record_id',encode(r.record_id,'hex'),'graph_kind',r.graph_kind,
            'record_kind',r.record_kind,'review_state',r.review_state,
            'trusted_context',r.trusted_context,'subject',r.subject,'category',r.category,
            'relation',r.relation,'object',r.object,'source_path',r.source_path,
            'source_digest',encode(r.source_digest,'hex'),'line_start',r.line_start,
            'line_end',r.line_end,'confidence',r.confidence,
            'content_digest',encode(r.content_digest,'hex')) ORDER BY r.ordinal,r.record_id)
            FROM page r),'[]'::jsonb),
        'truncated',(SELECT count(*)>p_limit FROM matches)) INTO result;
    RETURN result;
END
$code_relations_v1$;
REVOKE ALL ON FUNCTION control_product.code_relations_v1(text,text,text,text,text,text,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control_product.code_relations_v1(text,text,text,text,text,text,integer) TO lattice_runtime;
