-- LATTICE Codebase Memory extension profile v1.
-- The explicit administrative runner owns the transaction boundary.
-- This file must not contain transaction control or dynamic SQL.
-- Global Store migrations and their manifest do not include this profile.

CREATE TABLE memory.codebase_memory_extension_identity (
    singleton boolean PRIMARY KEY DEFAULT true,
    extension_id varchar(64) NOT NULL,
    extension_schema_version smallint NOT NULL,
    extension_path varchar(256) NOT NULL,
    extension_sql_sha256 char(64) NOT NULL,
    extension_manifest_sha256 char(64) NOT NULL,
    database_uuid uuid NOT NULL,
    database_identity_sha256 char(64) NOT NULL,
    global_schema_version smallint NOT NULL,
    global_manifest_sha256 char(64) NOT NULL,
    installed_at timestamp with time zone NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    CONSTRAINT codebase_memory_extension_identity_singleton CHECK (singleton),
    CONSTRAINT codebase_memory_extension_identity_id CHECK (
        extension_id = 'lattice-codebase-memory'
    ),
    CONSTRAINT codebase_memory_extension_identity_version CHECK (
        extension_schema_version = 1
    ),
    CONSTRAINT codebase_memory_extension_identity_path CHECK (
        extension_path = 'db/extensions/codebase-memory/v1.sql'
    ),
    CONSTRAINT codebase_memory_extension_identity_global_version CHECK (
        global_schema_version = 3
    ),
    CONSTRAINT codebase_memory_extension_identity_hashes CHECK (
        extension_sql_sha256 ~ '^[0-9a-f]{64}$'
        AND extension_sql_sha256 <> pg_catalog.repeat('0', 64)
        AND extension_manifest_sha256 ~ '^[0-9a-f]{64}$'
        AND extension_manifest_sha256 <> pg_catalog.repeat('0', 64)
        AND database_identity_sha256 ~ '^[0-9a-f]{64}$'
        AND database_identity_sha256 <> pg_catalog.repeat('0', 64)
        AND global_manifest_sha256 ~ '^[0-9a-f]{64}$'
        AND global_manifest_sha256 <> pg_catalog.repeat('0', 64)
    )
);

CREATE TABLE memory.codebase_memory_extension_ledger (
    ledger_ordinal smallint PRIMARY KEY,
    singleton boolean NOT NULL UNIQUE,
    extension_id varchar(64) NOT NULL,
    extension_schema_version smallint NOT NULL,
    extension_sql_sha256 char(64) NOT NULL,
    extension_manifest_sha256 char(64) NOT NULL,
    database_uuid uuid NOT NULL,
    database_identity_sha256 char(64) NOT NULL,
    global_schema_version smallint NOT NULL,
    global_manifest_sha256 char(64) NOT NULL,
    event_kind varchar(16) NOT NULL,
    recorded_at timestamp with time zone NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    CONSTRAINT codebase_memory_extension_ledger_single_entry CHECK (
        ledger_ordinal = 1 AND singleton
    ),
    CONSTRAINT codebase_memory_extension_ledger_identity CHECK (
        extension_id = 'lattice-codebase-memory'
        AND extension_schema_version = 1
        AND global_schema_version = 3
        AND event_kind = 'INSTALLED'
    ),
    CONSTRAINT codebase_memory_extension_ledger_hashes CHECK (
        extension_sql_sha256 ~ '^[0-9a-f]{64}$'
        AND extension_manifest_sha256 ~ '^[0-9a-f]{64}$'
        AND database_identity_sha256 ~ '^[0-9a-f]{64}$'
        AND global_manifest_sha256 ~ '^[0-9a-f]{64}$'
    ),
    CONSTRAINT codebase_memory_extension_ledger_identity_fk FOREIGN KEY (singleton)
        REFERENCES memory.codebase_memory_extension_identity (singleton)
);

CREATE TABLE memory.codebase_memory_analyses (
    analysis_digest bytea PRIMARY KEY,
    contract_version smallint NOT NULL,
    request_id text NOT NULL,
    task_id text NOT NULL,
    attempt_id text NOT NULL,
    project_snapshot_id text NOT NULL,
    subject_digest bytea NOT NULL,
    project_id varchar(64) NOT NULL,
    commit_id varchar(64) NOT NULL,
    query_digest bytea NOT NULL,
    configuration_digest bytea NOT NULL,
    retrieval_limit smallint NOT NULL,
    tree_id varchar(64) NOT NULL,
    manifest_digest bytea NOT NULL,
    exclusion_digest bytea NOT NULL,
    graphify_identity_digest bytea NOT NULL,
    graph_artifact_digest bytea NOT NULL,
    raw_output_digest bytea NOT NULL,
    raw_evidence_digest bytea NOT NULL,
    record_set_digest bytea NOT NULL,
    record_count integer NOT NULL,
    persistence_digest bytea NOT NULL UNIQUE,
    recorded_at timestamp with time zone NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    CONSTRAINT codebase_memory_analyses_contract CHECK (contract_version = 1),
    CONSTRAINT codebase_memory_analyses_identifiers CHECK (
        pg_catalog.octet_length(request_id) BETWEEN 1 AND 1024
        AND pg_catalog.octet_length(task_id) BETWEEN 1 AND 1024
        AND pg_catalog.octet_length(attempt_id) BETWEEN 1 AND 1024
        AND pg_catalog.octet_length(project_snapshot_id) BETWEEN 1 AND 1024
        AND project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'
        AND commit_id ~ '^(?:[0-9a-f]{40}|[0-9a-f]{64})$'
        AND tree_id ~ '^(?:[0-9a-f]{40}|[0-9a-f]{64})$'
    ),
    CONSTRAINT codebase_memory_analyses_limit_count CHECK (
        retrieval_limit BETWEEN 1 AND 100
        AND record_count BETWEEN 1 AND 100000
    ),
    CONSTRAINT codebase_memory_analyses_request_unique UNIQUE (
        project_id,
        project_snapshot_id,
        commit_id,
        query_digest,
        configuration_digest,
        retrieval_limit
    ),
    CONSTRAINT codebase_memory_analyses_digest_shapes CHECK (
        pg_catalog.octet_length(analysis_digest) = 32
        AND pg_catalog.octet_length(subject_digest) = 32
        AND pg_catalog.octet_length(query_digest) = 32
        AND pg_catalog.octet_length(configuration_digest) = 32
        AND pg_catalog.octet_length(manifest_digest) = 32
        AND pg_catalog.octet_length(exclusion_digest) = 32
        AND pg_catalog.octet_length(graphify_identity_digest) = 32
        AND pg_catalog.octet_length(graph_artifact_digest) = 32
        AND pg_catalog.octet_length(raw_output_digest) = 32
        AND pg_catalog.octet_length(raw_evidence_digest) = 32
        AND pg_catalog.octet_length(record_set_digest) = 32
        AND pg_catalog.octet_length(persistence_digest) = 32
    )
);

CREATE TABLE memory.codebase_memory_records (
    analysis_digest bytea NOT NULL,
    ordinal integer NOT NULL,
    record_id bytea NOT NULL,
    graph_kind varchar(8) NOT NULL,
    record_kind varchar(16) NOT NULL,
    review_state varchar(16) NOT NULL,
    trusted_context boolean NOT NULL,
    subject text NOT NULL,
    category text NOT NULL,
    relation text,
    object text,
    source_path text NOT NULL,
    source_digest bytea NOT NULL,
    line_start integer,
    line_end integer,
    confidence varchar(16) NOT NULL,
    content_digest bytea NOT NULL,
    PRIMARY KEY (analysis_digest, ordinal),
    CONSTRAINT codebase_memory_records_id_unique UNIQUE (analysis_digest, record_id),
    CONSTRAINT codebase_memory_records_analysis_fk FOREIGN KEY (analysis_digest)
        REFERENCES memory.codebase_memory_analyses (analysis_digest),
    CONSTRAINT codebase_memory_records_ordinal CHECK (
        ordinal BETWEEN 1 AND 100000
    ),
    CONSTRAINT codebase_memory_records_kinds CHECK (
        graph_kind IN ('NODE', 'EDGE')
        AND record_kind = 'OBSERVATION'
        AND review_state = 'CANDIDATE'
        AND NOT trusted_context
        AND confidence IN ('EXTRACTED', 'INFERRED', 'AMBIGUOUS')
    ),
    CONSTRAINT codebase_memory_records_shape CHECK (
        (graph_kind = 'NODE' AND relation IS NULL AND object IS NULL)
        OR
        (graph_kind = 'EDGE' AND relation IS NOT NULL AND object IS NOT NULL)
    ),
    CONSTRAINT codebase_memory_records_text_bounds CHECK (
        pg_catalog.octet_length(subject) BETWEEN 1 AND 4096
        AND pg_catalog.octet_length(category) BETWEEN 1 AND 4096
        AND (relation IS NULL OR pg_catalog.octet_length(relation) BETWEEN 1 AND 4096)
        AND (object IS NULL OR pg_catalog.octet_length(object) BETWEEN 1 AND 4096)
        AND pg_catalog.octet_length(source_path) BETWEEN 1 AND 1024
    ),
    CONSTRAINT codebase_memory_records_line_range CHECK (
        (line_start IS NULL AND line_end IS NULL)
        OR
        (line_start > 0 AND line_end >= line_start)
    ),
    CONSTRAINT codebase_memory_records_digest_shapes CHECK (
        pg_catalog.octet_length(analysis_digest) = 32
        AND pg_catalog.octet_length(record_id) = 32
        AND pg_catalog.octet_length(source_digest) = 32
        AND pg_catalog.octet_length(content_digest) = 32
    )
);

CREATE TABLE memory.codebase_memory_retrieval_audits (
    retrieval_digest bytea PRIMARY KEY,
    analysis_digest bytea NOT NULL UNIQUE,
    persistence_digest bytea NOT NULL,
    query_digest bytea NOT NULL,
    algorithm varchar(64) NOT NULL,
    retrieval_limit smallint NOT NULL,
    disposition varchar(16) NOT NULL,
    result_record_ids bytea[] NOT NULL,
    result_record_digests bytea[] NOT NULL,
    result_scores bigint[] NOT NULL,
    result_set_digest bytea NOT NULL,
    recorded_at timestamp with time zone NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    CONSTRAINT codebase_memory_retrieval_analysis_fk FOREIGN KEY (analysis_digest)
        REFERENCES memory.codebase_memory_analyses (analysis_digest),
    CONSTRAINT codebase_memory_retrieval_algorithm CHECK (
        algorithm = 'lattice-structural-retrieval-v1'
    ),
    CONSTRAINT codebase_memory_retrieval_limit CHECK (
        retrieval_limit BETWEEN 1 AND 100
    ),
    CONSTRAINT codebase_memory_retrieval_arrays CHECK (
        pg_catalog.cardinality(result_record_ids) = pg_catalog.cardinality(result_record_digests)
        AND pg_catalog.cardinality(result_record_ids) = pg_catalog.cardinality(result_scores)
        AND pg_catalog.cardinality(result_record_ids) <= retrieval_limit
        AND pg_catalog.array_position(result_record_ids, NULL) IS NULL
        AND pg_catalog.array_position(result_record_digests, NULL) IS NULL
        AND pg_catalog.array_position(result_scores, NULL) IS NULL
    ),
    CONSTRAINT codebase_memory_retrieval_disposition CHECK (
        (disposition = 'NO_ANSWER' AND pg_catalog.cardinality(result_record_ids) = 0)
        OR
        (disposition = 'RESULTS' AND pg_catalog.cardinality(result_record_ids) > 0)
    ),
    CONSTRAINT codebase_memory_retrieval_digest_shapes CHECK (
        pg_catalog.octet_length(retrieval_digest) = 32
        AND pg_catalog.octet_length(analysis_digest) = 32
        AND pg_catalog.octet_length(persistence_digest) = 32
        AND pg_catalog.octet_length(query_digest) = 32
        AND pg_catalog.octet_length(result_set_digest) = 32
    )
);

CREATE TABLE memory.codebase_memory_receipts (
    receipt_digest bytea PRIMARY KEY,
    analysis_digest bytea NOT NULL UNIQUE,
    retrieval_digest bytea NOT NULL UNIQUE,
    persistence_digest bytea NOT NULL,
    query_digest bytea NOT NULL,
    recorded_at timestamp with time zone NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    CONSTRAINT codebase_memory_receipts_analysis_fk FOREIGN KEY (analysis_digest)
        REFERENCES memory.codebase_memory_analyses (analysis_digest),
    CONSTRAINT codebase_memory_receipts_retrieval_fk FOREIGN KEY (retrieval_digest)
        REFERENCES memory.codebase_memory_retrieval_audits (retrieval_digest),
    CONSTRAINT codebase_memory_receipts_digest_shapes CHECK (
        pg_catalog.octet_length(receipt_digest) = 32
        AND pg_catalog.octet_length(analysis_digest) = 32
        AND pg_catalog.octet_length(retrieval_digest) = 32
        AND pg_catalog.octet_length(persistence_digest) = 32
        AND pg_catalog.octet_length(query_digest) = 32
    )
);

CREATE FUNCTION memory.codebase_memory_persist_analysis_v1(
    p_database_identity_digest bytea,
    p_global_manifest_digest bytea,
    p_extension_sql_digest bytea,
    p_extension_manifest_digest bytea,
    p_contract_version smallint,
    p_request_id text,
    p_task_id text,
    p_attempt_id text,
    p_project_snapshot_id text,
    p_subject_digest bytea,
    p_project_id text,
    p_commit_id text,
    p_query_digest bytea,
    p_configuration_digest bytea,
    p_retrieval_limit smallint,
    p_tree_id text,
    p_manifest_digest bytea,
    p_exclusion_digest bytea,
    p_graphify_identity_digest bytea,
    p_graph_artifact_digest bytea,
    p_raw_output_digest bytea,
    p_raw_evidence_digest bytea,
    p_record_set_digest bytea,
    p_analysis_digest bytea,
    p_persistence_digest bytea,
    p_record_ordinals integer[],
    p_record_ids bytea[],
    p_graph_kinds text[],
    p_subjects text[],
    p_categories text[],
    p_relations text[],
    p_objects text[],
    p_source_paths text[],
    p_source_digests bytea[],
    p_line_starts integer[],
    p_line_ends integer[],
    p_confidences text[],
    p_content_digests bytea[]
)
RETURNS TABLE (persistence_status text, record_count integer)
LANGUAGE plpgsql
VOLATILE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_codebase_memory_persist_analysis_v1$
DECLARE
    v_analysis memory.codebase_memory_analyses%ROWTYPE;
    v_count integer;
    v_digest bytea;
    v_index integer;
    v_previous_record_id bytea;
    v_database_uuid uuid;
    v_identity_count bigint;
    v_record_count bigint;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCM01', MESSAGE = 'invalid memory runtime boundary';
    END IF;

    SELECT d.database_uuid, pg_catalog.count(*)
      INTO v_database_uuid, v_identity_count
      FROM ONLY memory.codebase_memory_extension_identity AS i
      JOIN ONLY memory.codebase_memory_extension_ledger AS l USING (singleton)
      CROSS JOIN ONLY control.database_identity AS d
      CROSS JOIN ONLY control.schema_compatibility AS c
     WHERE i.singleton
       AND l.ledger_ordinal = 1
       AND i.extension_id = 'lattice-codebase-memory'
       AND i.extension_schema_version = 1
       AND i.extension_path = 'db/extensions/codebase-memory/v1.sql'
       AND i.database_uuid = d.database_uuid
       AND i.global_schema_version = 3
       AND c.singleton
       AND c.current_schema_version = 3
       AND pg_catalog.decode(pg_catalog.btrim(i.database_identity_sha256), 'hex') = p_database_identity_digest
       AND pg_catalog.decode(pg_catalog.btrim(i.global_manifest_sha256), 'hex') = p_global_manifest_digest
       AND pg_catalog.decode(pg_catalog.btrim(i.extension_sql_sha256), 'hex') = p_extension_sql_digest
       AND pg_catalog.decode(pg_catalog.btrim(i.extension_manifest_sha256), 'hex') = p_extension_manifest_digest
       AND pg_catalog.btrim(i.global_manifest_sha256) = pg_catalog.btrim(c.manifest_sha256)
       AND l.extension_id = i.extension_id
       AND l.extension_schema_version = i.extension_schema_version
       AND l.extension_sql_sha256 = i.extension_sql_sha256
       AND l.extension_manifest_sha256 = i.extension_manifest_sha256
       AND l.database_uuid = i.database_uuid
       AND l.database_identity_sha256 = i.database_identity_sha256
       AND l.global_schema_version = i.global_schema_version
       AND l.global_manifest_sha256 = i.global_manifest_sha256
       AND l.event_kind = 'INSTALLED'
     GROUP BY d.database_uuid;
    IF v_identity_count IS DISTINCT FROM 1 OR v_database_uuid IS NULL THEN
        RAISE EXCEPTION USING ERRCODE = 'LCM02', MESSAGE = 'memory extension identity mismatch';
    END IF;

    v_count := pg_catalog.cardinality(p_record_ordinals);
    IF p_contract_version IS DISTINCT FROM 1
       OR p_request_id IS NULL OR pg_catalog.octet_length(p_request_id) NOT BETWEEN 1 AND 1024
       OR p_task_id IS NULL OR pg_catalog.octet_length(p_task_id) NOT BETWEEN 1 AND 1024
       OR p_attempt_id IS NULL OR pg_catalog.octet_length(p_attempt_id) NOT BETWEEN 1 AND 1024
       OR p_project_snapshot_id IS NULL OR pg_catalog.octet_length(p_project_snapshot_id) NOT BETWEEN 1 AND 1024
       OR p_project_id IS NULL OR p_project_id !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
       OR p_commit_id IS NULL OR p_commit_id !~ '^(?:[0-9a-f]{40}|[0-9a-f]{64})$'
       OR p_tree_id IS NULL OR p_tree_id !~ '^(?:[0-9a-f]{40}|[0-9a-f]{64})$'
       OR p_retrieval_limit IS NULL OR p_retrieval_limit NOT BETWEEN 1 AND 100
       OR v_count IS NULL OR v_count NOT BETWEEN 1 AND 100000
       OR pg_catalog.cardinality(p_record_ids) IS DISTINCT FROM v_count
       OR pg_catalog.cardinality(p_graph_kinds) IS DISTINCT FROM v_count
       OR pg_catalog.cardinality(p_subjects) IS DISTINCT FROM v_count
       OR pg_catalog.cardinality(p_categories) IS DISTINCT FROM v_count
       OR pg_catalog.cardinality(p_relations) IS DISTINCT FROM v_count
       OR pg_catalog.cardinality(p_objects) IS DISTINCT FROM v_count
       OR pg_catalog.cardinality(p_source_paths) IS DISTINCT FROM v_count
       OR pg_catalog.cardinality(p_source_digests) IS DISTINCT FROM v_count
       OR pg_catalog.cardinality(p_line_starts) IS DISTINCT FROM v_count
       OR pg_catalog.cardinality(p_line_ends) IS DISTINCT FROM v_count
       OR pg_catalog.cardinality(p_confidences) IS DISTINCT FROM v_count
       OR pg_catalog.cardinality(p_content_digests) IS DISTINCT FROM v_count
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCM03', MESSAGE = 'invalid memory analysis';
    END IF;

    FOREACH v_digest IN ARRAY ARRAY[
        p_database_identity_digest, p_global_manifest_digest,
        p_extension_sql_digest, p_extension_manifest_digest,
        p_subject_digest, p_query_digest, p_configuration_digest,
        p_manifest_digest, p_exclusion_digest, p_graphify_identity_digest,
        p_graph_artifact_digest, p_raw_output_digest, p_raw_evidence_digest,
        p_record_set_digest, p_analysis_digest, p_persistence_digest
    ]
    LOOP
        IF v_digest IS NULL
           OR pg_catalog.octet_length(v_digest) <> 32
           OR v_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LCM03', MESSAGE = 'invalid memory digest';
        END IF;
    END LOOP;

    FOR v_index IN 1..v_count LOOP
        IF p_record_ordinals[v_index] IS DISTINCT FROM v_index
           OR p_record_ids[v_index] IS NULL
           OR pg_catalog.octet_length(p_record_ids[v_index]) <> 32
           OR p_content_digests[v_index] IS NULL
           OR pg_catalog.octet_length(p_content_digests[v_index]) <> 32
           OR p_source_digests[v_index] IS NULL
           OR pg_catalog.octet_length(p_source_digests[v_index]) <> 32
           OR p_graph_kinds[v_index] NOT IN ('NODE', 'EDGE')
           OR p_confidences[v_index] NOT IN ('EXTRACTED', 'INFERRED', 'AMBIGUOUS')
           OR p_subjects[v_index] IS NULL OR pg_catalog.octet_length(p_subjects[v_index]) NOT BETWEEN 1 AND 4096
           OR p_categories[v_index] IS NULL OR pg_catalog.octet_length(p_categories[v_index]) NOT BETWEEN 1 AND 4096
           OR p_source_paths[v_index] IS NULL OR pg_catalog.octet_length(p_source_paths[v_index]) NOT BETWEEN 1 AND 1024
           OR ((p_graph_kinds[v_index] = 'NODE') AND (p_relations[v_index] IS NOT NULL OR p_objects[v_index] IS NOT NULL))
           OR ((p_graph_kinds[v_index] = 'EDGE') AND (p_relations[v_index] IS NULL OR p_objects[v_index] IS NULL))
           OR ((p_line_starts[v_index] IS NULL) <> (p_line_ends[v_index] IS NULL))
           OR (p_line_starts[v_index] IS NOT NULL AND (p_line_starts[v_index] <= 0 OR p_line_ends[v_index] < p_line_starts[v_index]))
           OR (v_previous_record_id IS NOT NULL AND p_record_ids[v_index] <= v_previous_record_id)
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LCM03', MESSAGE = 'invalid memory record';
        END IF;
        v_previous_record_id := p_record_ids[v_index];
    END LOOP;

    SELECT * INTO v_analysis
      FROM ONLY memory.codebase_memory_analyses
     WHERE analysis_digest = p_analysis_digest;
    IF FOUND THEN
        IF v_analysis.contract_version IS DISTINCT FROM p_contract_version
           OR v_analysis.request_id IS DISTINCT FROM p_request_id
           OR v_analysis.task_id IS DISTINCT FROM p_task_id
           OR v_analysis.attempt_id IS DISTINCT FROM p_attempt_id
           OR v_analysis.project_snapshot_id IS DISTINCT FROM p_project_snapshot_id
           OR v_analysis.subject_digest IS DISTINCT FROM p_subject_digest
           OR v_analysis.project_id IS DISTINCT FROM p_project_id
           OR v_analysis.commit_id IS DISTINCT FROM p_commit_id
           OR v_analysis.query_digest IS DISTINCT FROM p_query_digest
           OR v_analysis.configuration_digest IS DISTINCT FROM p_configuration_digest
           OR v_analysis.retrieval_limit IS DISTINCT FROM p_retrieval_limit
           OR v_analysis.tree_id IS DISTINCT FROM p_tree_id
           OR v_analysis.manifest_digest IS DISTINCT FROM p_manifest_digest
           OR v_analysis.exclusion_digest IS DISTINCT FROM p_exclusion_digest
           OR v_analysis.graphify_identity_digest IS DISTINCT FROM p_graphify_identity_digest
           OR v_analysis.graph_artifact_digest IS DISTINCT FROM p_graph_artifact_digest
           OR v_analysis.raw_output_digest IS DISTINCT FROM p_raw_output_digest
           OR v_analysis.raw_evidence_digest IS DISTINCT FROM p_raw_evidence_digest
           OR v_analysis.record_set_digest IS DISTINCT FROM p_record_set_digest
           OR v_analysis.record_count IS DISTINCT FROM v_count
           OR v_analysis.persistence_digest IS DISTINCT FROM p_persistence_digest
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LCM04', MESSAGE = 'memory analysis substitution';
        END IF;
        SELECT pg_catalog.count(*) INTO v_record_count
          FROM ONLY memory.codebase_memory_records
         WHERE analysis_digest = p_analysis_digest;
        IF v_record_count IS DISTINCT FROM v_count::bigint THEN
            RAISE EXCEPTION USING ERRCODE = 'LCM05', MESSAGE = 'memory record set corrupt';
        END IF;
        FOR v_index IN 1..v_count LOOP
            PERFORM 1
              FROM ONLY memory.codebase_memory_records AS r
             WHERE r.analysis_digest = p_analysis_digest
               AND r.ordinal = p_record_ordinals[v_index]
               AND r.record_id = p_record_ids[v_index]
               AND r.graph_kind = p_graph_kinds[v_index]
               AND r.record_kind = 'OBSERVATION'
               AND r.review_state = 'CANDIDATE'
               AND NOT r.trusted_context
               AND r.subject = p_subjects[v_index]
               AND r.category = p_categories[v_index]
               AND r.relation IS NOT DISTINCT FROM p_relations[v_index]
               AND r.object IS NOT DISTINCT FROM p_objects[v_index]
               AND r.source_path = p_source_paths[v_index]
               AND r.source_digest = p_source_digests[v_index]
               AND r.line_start IS NOT DISTINCT FROM p_line_starts[v_index]
               AND r.line_end IS NOT DISTINCT FROM p_line_ends[v_index]
               AND r.confidence = p_confidences[v_index]
               AND r.content_digest = p_content_digests[v_index];
            IF NOT FOUND THEN
                RAISE EXCEPTION USING ERRCODE = 'LCM05', MESSAGE = 'memory record set corrupt';
            END IF;
        END LOOP;
        RETURN QUERY SELECT 'REPLAYED'::text, v_count;
        RETURN;
    END IF;

    IF EXISTS (
        SELECT 1 FROM ONLY memory.codebase_memory_analyses AS a
         WHERE a.project_id = p_project_id
           AND a.project_snapshot_id = p_project_snapshot_id
           AND a.commit_id = p_commit_id
           AND a.query_digest = p_query_digest
           AND a.configuration_digest = p_configuration_digest
           AND a.retrieval_limit = p_retrieval_limit
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'LCM04', MESSAGE = 'memory request substitution';
    END IF;

    INSERT INTO memory.codebase_memory_analyses (
        analysis_digest, contract_version, request_id, task_id, attempt_id,
        project_snapshot_id, subject_digest, project_id, commit_id, query_digest,
        configuration_digest, retrieval_limit, tree_id, manifest_digest,
        exclusion_digest, graphify_identity_digest, graph_artifact_digest,
        raw_output_digest, raw_evidence_digest, record_set_digest, record_count,
        persistence_digest
    ) VALUES (
        p_analysis_digest, p_contract_version, p_request_id, p_task_id, p_attempt_id,
        p_project_snapshot_id, p_subject_digest, p_project_id, p_commit_id, p_query_digest,
        p_configuration_digest, p_retrieval_limit, p_tree_id, p_manifest_digest,
        p_exclusion_digest, p_graphify_identity_digest, p_graph_artifact_digest,
        p_raw_output_digest, p_raw_evidence_digest, p_record_set_digest, v_count,
        p_persistence_digest
    );

    FOR v_index IN 1..v_count LOOP
        INSERT INTO memory.codebase_memory_records (
            analysis_digest, ordinal, record_id, graph_kind, record_kind,
            review_state, trusted_context, subject, category, relation, object,
            source_path, source_digest, line_start, line_end, confidence,
            content_digest
        ) VALUES (
            p_analysis_digest, p_record_ordinals[v_index], p_record_ids[v_index],
            p_graph_kinds[v_index], 'OBSERVATION', 'CANDIDATE', false,
            p_subjects[v_index], p_categories[v_index], p_relations[v_index],
            p_objects[v_index], p_source_paths[v_index], p_source_digests[v_index],
            p_line_starts[v_index], p_line_ends[v_index], p_confidences[v_index],
            p_content_digests[v_index]
        );
    END LOOP;
    RETURN QUERY SELECT 'PERSISTED'::text, v_count;
END;
$lattice_codebase_memory_persist_analysis_v1$;

CREATE FUNCTION memory.codebase_memory_persist_retrieval_v1(
    p_database_identity_digest bytea,
    p_global_manifest_digest bytea,
    p_extension_sql_digest bytea,
    p_extension_manifest_digest bytea,
    p_analysis_digest bytea,
    p_persistence_digest bytea,
    p_query_digest bytea,
    p_retrieval_limit smallint,
    p_disposition text,
    p_result_record_ids bytea[],
    p_result_record_digests bytea[],
    p_result_scores bigint[],
    p_result_set_digest bytea,
    p_retrieval_digest bytea,
    p_receipt_digest bytea
)
RETURNS TABLE (retrieval_status text)
LANGUAGE plpgsql
VOLATILE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_codebase_memory_persist_retrieval_v1$
DECLARE
    v_analysis memory.codebase_memory_analyses%ROWTYPE;
    v_audit memory.codebase_memory_retrieval_audits%ROWTYPE;
    v_receipt memory.codebase_memory_receipts%ROWTYPE;
    v_count integer;
    v_digest bytea;
    v_index integer;
    v_previous_id bytea;
    v_previous_score bigint;
    v_identity_count bigint;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCM01', MESSAGE = 'invalid memory runtime boundary';
    END IF;
    SELECT pg_catalog.count(*) INTO v_identity_count
      FROM ONLY memory.codebase_memory_extension_identity AS i
      JOIN ONLY memory.codebase_memory_extension_ledger AS l USING (singleton)
      CROSS JOIN ONLY control.database_identity AS d
      CROSS JOIN ONLY control.schema_compatibility AS c
     WHERE i.singleton AND l.ledger_ordinal = 1
       AND i.database_uuid = d.database_uuid
       AND i.global_schema_version = c.current_schema_version
       AND c.current_schema_version = 3
       AND pg_catalog.decode(pg_catalog.btrim(i.database_identity_sha256), 'hex') = p_database_identity_digest
       AND pg_catalog.decode(pg_catalog.btrim(i.global_manifest_sha256), 'hex') = p_global_manifest_digest
       AND pg_catalog.decode(pg_catalog.btrim(i.extension_sql_sha256), 'hex') = p_extension_sql_digest
       AND pg_catalog.decode(pg_catalog.btrim(i.extension_manifest_sha256), 'hex') = p_extension_manifest_digest
       AND pg_catalog.btrim(i.global_manifest_sha256) = pg_catalog.btrim(c.manifest_sha256)
       AND l.extension_id = i.extension_id
       AND l.extension_schema_version = i.extension_schema_version
       AND l.extension_sql_sha256 = i.extension_sql_sha256
       AND l.extension_manifest_sha256 = i.extension_manifest_sha256
       AND l.database_uuid = i.database_uuid
       AND l.database_identity_sha256 = i.database_identity_sha256
       AND l.global_schema_version = i.global_schema_version
       AND l.global_manifest_sha256 = i.global_manifest_sha256
       AND l.event_kind = 'INSTALLED';
    IF v_identity_count IS DISTINCT FROM 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'LCM02', MESSAGE = 'memory extension identity mismatch';
    END IF;

    FOREACH v_digest IN ARRAY ARRAY[
        p_database_identity_digest, p_global_manifest_digest,
        p_extension_sql_digest, p_extension_manifest_digest,
        p_analysis_digest, p_persistence_digest, p_query_digest,
        p_result_set_digest, p_retrieval_digest, p_receipt_digest
    ]
    LOOP
        IF v_digest IS NULL OR pg_catalog.octet_length(v_digest) <> 32
           OR v_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LCM03', MESSAGE = 'invalid retrieval digest';
        END IF;
    END LOOP;

    v_count := pg_catalog.cardinality(p_result_record_ids);
    IF p_retrieval_limit IS NULL OR p_retrieval_limit NOT BETWEEN 1 AND 100
       OR v_count IS NULL OR v_count > p_retrieval_limit
       OR pg_catalog.cardinality(p_result_record_digests) IS DISTINCT FROM v_count
       OR pg_catalog.cardinality(p_result_scores) IS DISTINCT FROM v_count
       OR (p_disposition = 'NO_ANSWER' AND v_count <> 0)
       OR (p_disposition = 'RESULTS' AND v_count = 0)
       OR p_disposition NOT IN ('RESULTS', 'NO_ANSWER')
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCM03', MESSAGE = 'invalid memory retrieval';
    END IF;

    SELECT * INTO v_analysis
      FROM ONLY memory.codebase_memory_analyses
     WHERE analysis_digest = p_analysis_digest;
    IF NOT FOUND
       OR v_analysis.persistence_digest IS DISTINCT FROM p_persistence_digest
       OR v_analysis.query_digest IS DISTINCT FROM p_query_digest
       OR v_analysis.retrieval_limit IS DISTINCT FROM p_retrieval_limit
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCM04', MESSAGE = 'retrieval analysis substitution';
    END IF;

    FOR v_index IN 1..v_count LOOP
        IF p_result_record_ids[v_index] IS NULL
           OR pg_catalog.octet_length(p_result_record_ids[v_index]) <> 32
           OR p_result_record_digests[v_index] IS NULL
           OR pg_catalog.octet_length(p_result_record_digests[v_index]) <> 32
           OR p_result_scores[v_index] IS NULL OR p_result_scores[v_index] <= 0
           OR (v_index > 1 AND p_result_record_ids[v_index] = ANY(p_result_record_ids[1:v_index - 1]))
           OR (v_previous_score IS NOT NULL AND p_result_scores[v_index] > v_previous_score)
           OR (v_previous_score = p_result_scores[v_index] AND p_result_record_ids[v_index] <= v_previous_id)
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LCM03', MESSAGE = 'invalid ranked result';
        END IF;
        PERFORM 1 FROM ONLY memory.codebase_memory_records AS r
         WHERE r.analysis_digest = p_analysis_digest
           AND r.record_id = p_result_record_ids[v_index]
           AND r.content_digest = p_result_record_digests[v_index];
        IF NOT FOUND THEN
            RAISE EXCEPTION USING ERRCODE = 'LCM04', MESSAGE = 'unknown ranked result';
        END IF;
        v_previous_id := p_result_record_ids[v_index];
        v_previous_score := p_result_scores[v_index];
    END LOOP;

    SELECT * INTO v_audit
      FROM ONLY memory.codebase_memory_retrieval_audits
     WHERE retrieval_digest = p_retrieval_digest;
    IF FOUND THEN
        SELECT * INTO v_receipt
          FROM ONLY memory.codebase_memory_receipts
         WHERE receipt_digest = p_receipt_digest;
        IF NOT FOUND
           OR v_audit.analysis_digest IS DISTINCT FROM p_analysis_digest
           OR v_audit.persistence_digest IS DISTINCT FROM p_persistence_digest
           OR v_audit.query_digest IS DISTINCT FROM p_query_digest
           OR v_audit.retrieval_limit IS DISTINCT FROM p_retrieval_limit
           OR v_audit.disposition IS DISTINCT FROM p_disposition
           OR v_audit.result_record_ids IS DISTINCT FROM p_result_record_ids
           OR v_audit.result_record_digests IS DISTINCT FROM p_result_record_digests
           OR v_audit.result_scores IS DISTINCT FROM p_result_scores
           OR v_audit.result_set_digest IS DISTINCT FROM p_result_set_digest
           OR v_receipt.analysis_digest IS DISTINCT FROM p_analysis_digest
           OR v_receipt.retrieval_digest IS DISTINCT FROM p_retrieval_digest
           OR v_receipt.persistence_digest IS DISTINCT FROM p_persistence_digest
           OR v_receipt.query_digest IS DISTINCT FROM p_query_digest
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LCM04', MESSAGE = 'memory retrieval substitution';
        END IF;
        RETURN QUERY SELECT 'REPLAYED'::text;
        RETURN;
    END IF;
    IF EXISTS (
        SELECT 1 FROM ONLY memory.codebase_memory_retrieval_audits
         WHERE analysis_digest = p_analysis_digest
    ) OR EXISTS (
        SELECT 1 FROM ONLY memory.codebase_memory_receipts
         WHERE analysis_digest = p_analysis_digest OR receipt_digest = p_receipt_digest
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'LCM04', MESSAGE = 'memory receipt substitution';
    END IF;

    INSERT INTO memory.codebase_memory_retrieval_audits (
        retrieval_digest, analysis_digest, persistence_digest, query_digest,
        algorithm, retrieval_limit, disposition, result_record_ids,
        result_record_digests, result_scores, result_set_digest
    ) VALUES (
        p_retrieval_digest, p_analysis_digest, p_persistence_digest, p_query_digest,
        'lattice-structural-retrieval-v1', p_retrieval_limit, p_disposition,
        p_result_record_ids, p_result_record_digests, p_result_scores,
        p_result_set_digest
    );
    INSERT INTO memory.codebase_memory_receipts (
        receipt_digest, analysis_digest, retrieval_digest, persistence_digest,
        query_digest
    ) VALUES (
        p_receipt_digest, p_analysis_digest, p_retrieval_digest,
        p_persistence_digest, p_query_digest
    );
    RETURN QUERY SELECT 'PERSISTED'::text;
END;
$lattice_codebase_memory_persist_retrieval_v1$;

CREATE FUNCTION memory.codebase_memory_load_receipt_v1(
    p_database_identity_digest bytea,
    p_global_manifest_digest bytea,
    p_extension_sql_digest bytea,
    p_extension_manifest_digest bytea,
    p_contract_version smallint,
    p_request_id text,
    p_task_id text,
    p_attempt_id text,
    p_project_snapshot_id text,
    p_subject_digest bytea,
    p_project_id text,
    p_commit_id text,
    p_query_digest bytea,
    p_configuration_digest bytea,
    p_retrieval_limit smallint
)
RETURNS TABLE (
    analysis_digest bytea,
    record_set_digest bytea,
    record_count integer,
    persistence_digest bytea,
    disposition text,
    result_record_ids bytea[],
    result_record_digests bytea[],
    result_scores bigint[],
    result_set_digest bytea,
    retrieval_digest bytea,
    receipt_digest bytea
)
LANGUAGE plpgsql
STABLE
PARALLEL RESTRICTED
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $lattice_codebase_memory_load_receipt_v1$
DECLARE
    v_identity_count bigint;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'repeatable read'
       OR NOT pg_catalog.current_setting('transaction_read_only')::boolean
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LCM01', MESSAGE = 'invalid memory replay boundary';
    END IF;
    SELECT pg_catalog.count(*) INTO v_identity_count
      FROM ONLY memory.codebase_memory_extension_identity AS i
      JOIN ONLY memory.codebase_memory_extension_ledger AS l USING (singleton)
      CROSS JOIN ONLY control.database_identity AS d
      CROSS JOIN ONLY control.schema_compatibility AS c
     WHERE i.singleton AND l.ledger_ordinal = 1
       AND i.database_uuid = d.database_uuid
       AND i.global_schema_version = c.current_schema_version
       AND c.current_schema_version = 3
       AND pg_catalog.decode(pg_catalog.btrim(i.database_identity_sha256), 'hex') = p_database_identity_digest
       AND pg_catalog.decode(pg_catalog.btrim(i.global_manifest_sha256), 'hex') = p_global_manifest_digest
       AND pg_catalog.decode(pg_catalog.btrim(i.extension_sql_sha256), 'hex') = p_extension_sql_digest
       AND pg_catalog.decode(pg_catalog.btrim(i.extension_manifest_sha256), 'hex') = p_extension_manifest_digest
       AND pg_catalog.btrim(i.global_manifest_sha256) = pg_catalog.btrim(c.manifest_sha256)
       AND l.extension_id = i.extension_id
       AND l.extension_schema_version = i.extension_schema_version
       AND l.extension_sql_sha256 = i.extension_sql_sha256
       AND l.extension_manifest_sha256 = i.extension_manifest_sha256
       AND l.database_uuid = i.database_uuid
       AND l.database_identity_sha256 = i.database_identity_sha256
       AND l.global_schema_version = i.global_schema_version
       AND l.global_manifest_sha256 = i.global_manifest_sha256
       AND l.event_kind = 'INSTALLED';
    IF v_identity_count IS DISTINCT FROM 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'LCM02', MESSAGE = 'memory extension identity mismatch';
    END IF;

    RETURN QUERY
    SELECT a.analysis_digest,
           a.record_set_digest,
           a.record_count,
           a.persistence_digest,
           r.disposition::text,
           r.result_record_ids,
           r.result_record_digests,
           r.result_scores,
           r.result_set_digest,
           r.retrieval_digest,
           x.receipt_digest
      FROM ONLY memory.codebase_memory_analyses AS a
      JOIN ONLY memory.codebase_memory_retrieval_audits AS r
        ON r.analysis_digest = a.analysis_digest
      JOIN ONLY memory.codebase_memory_receipts AS x
        ON x.analysis_digest = a.analysis_digest
       AND x.retrieval_digest = r.retrieval_digest
       AND x.persistence_digest = a.persistence_digest
       AND x.query_digest = a.query_digest
     WHERE a.contract_version = p_contract_version
       AND a.request_id = p_request_id
       AND a.task_id = p_task_id
       AND a.attempt_id = p_attempt_id
       AND a.project_snapshot_id = p_project_snapshot_id
       AND a.subject_digest = p_subject_digest
       AND a.project_id = p_project_id
       AND a.commit_id = p_commit_id
       AND a.query_digest = p_query_digest
       AND a.configuration_digest = p_configuration_digest
       AND a.retrieval_limit = p_retrieval_limit
       AND r.persistence_digest = a.persistence_digest
       AND r.query_digest = a.query_digest
       AND r.retrieval_limit = a.retrieval_limit;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'LCM06', MESSAGE = 'memory receipt unavailable';
    END IF;
END;
$lattice_codebase_memory_load_receipt_v1$;

REVOKE ALL ON ALL TABLES IN SCHEMA memory FROM
    PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;

REVOKE ALL ON ALL FUNCTIONS IN SCHEMA memory FROM
    PUBLIC, lattice_runtime, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;

GRANT USAGE ON SCHEMA memory TO lattice_runtime;
GRANT EXECUTE ON FUNCTION memory.codebase_memory_persist_analysis_v1(
    bytea, bytea, bytea, bytea, smallint, text, text, text, text, bytea,
    text, text, bytea, bytea, smallint, text, bytea, bytea, bytea, bytea,
    bytea, bytea, bytea, bytea, bytea, integer[], bytea[], text[], text[],
    text[], text[], text[], text[], bytea[], integer[], integer[], text[], bytea[]
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION memory.codebase_memory_persist_retrieval_v1(
    bytea, bytea, bytea, bytea, bytea, bytea, bytea, smallint, text,
    bytea[], bytea[], bigint[], bytea, bytea, bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION memory.codebase_memory_load_receipt_v1(
    bytea, bytea, bytea, bytea, smallint, text, text, text, text, bytea,
    text, text, bytea, bytea, smallint
) TO lattice_runtime;

COMMENT ON TABLE memory.codebase_memory_extension_identity IS
    'LATTICE_CODEBASE_MEMORY_EXTENSION_IDENTITY_V1';
COMMENT ON TABLE memory.codebase_memory_extension_ledger IS
    'LATTICE_CODEBASE_MEMORY_EXTENSION_LEDGER_V1';
COMMENT ON TABLE memory.codebase_memory_analyses IS
    'LATTICE_CODEBASE_MEMORY_ANALYSES_V1';
COMMENT ON TABLE memory.codebase_memory_records IS
    'LATTICE_CODEBASE_MEMORY_RECORDS_V1';
COMMENT ON TABLE memory.codebase_memory_retrieval_audits IS
    'LATTICE_CODEBASE_MEMORY_RETRIEVAL_AUDITS_V1';
COMMENT ON TABLE memory.codebase_memory_receipts IS
    'LATTICE_CODEBASE_MEMORY_RECEIPTS_V1';
COMMENT ON FUNCTION memory.codebase_memory_persist_analysis_v1(
    bytea, bytea, bytea, bytea, smallint, text, text, text, text, bytea,
    text, text, bytea, bytea, smallint, text, bytea, bytea, bytea, bytea,
    bytea, bytea, bytea, bytea, bytea, integer[], bytea[], text[], text[],
    text[], text[], text[], text[], bytea[], integer[], integer[], text[], bytea[]
) IS 'LATTICE_CODEBASE_MEMORY_PERSIST_ANALYSIS_V1';
COMMENT ON FUNCTION memory.codebase_memory_persist_retrieval_v1(
    bytea, bytea, bytea, bytea, bytea, bytea, bytea, smallint, text,
    bytea[], bytea[], bigint[], bytea, bytea, bytea
) IS 'LATTICE_CODEBASE_MEMORY_PERSIST_RETRIEVAL_V1';
COMMENT ON FUNCTION memory.codebase_memory_load_receipt_v1(
    bytea, bytea, bytea, bytea, smallint, text, text, text, text, bytea,
    text, text, bytea, bytea, smallint
) IS 'LATTICE_CODEBASE_MEMORY_LOAD_RECEIPT_V1';
