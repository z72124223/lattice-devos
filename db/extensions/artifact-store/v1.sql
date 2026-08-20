CREATE SCHEMA artifact_store AUTHORIZATION lattice_migrator;

CREATE TABLE artifact_store.artifact_extension_identity (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    extension_id text NOT NULL CHECK (extension_id = 'lattice-postgres-artifact-store'),
    schema_version smallint NOT NULL CHECK (schema_version = 1),
    sql_sha256 bytea NOT NULL CHECK (pg_catalog.octet_length(sql_sha256) = 32),
    manifest_sha256 bytea NOT NULL CHECK (pg_catalog.octet_length(manifest_sha256) = 32),
    database_uuid uuid NOT NULL,
    database_identity_sha256 char(64) NOT NULL CHECK (database_identity_sha256 ~ '^[a-f0-9]{64}$'),
    global_schema_version smallint NOT NULL CHECK (global_schema_version = 5),
    global_manifest_sha256 char(64) NOT NULL CHECK (global_manifest_sha256 ~ '^[a-f0-9]{64}$'),
    required_memory_schema_version smallint NOT NULL CHECK (required_memory_schema_version = 3),
    required_memory_manifest_sha256 char(64) NOT NULL CHECK (required_memory_manifest_sha256 ~ '^[a-f0-9]{64}$'),
    installed_at timestamptz NOT NULL DEFAULT pg_catalog.clock_timestamp()
);

CREATE TABLE artifact_store.artifact_extension_ledger (
    ordinal bigint PRIMARY KEY CHECK (ordinal = 1),
    event_type text NOT NULL CHECK (event_type = 'INSTALLED'),
    extension_id text NOT NULL CHECK (extension_id = 'lattice-postgres-artifact-store'),
    schema_version smallint NOT NULL CHECK (schema_version = 1),
    sql_sha256 bytea NOT NULL CHECK (pg_catalog.octet_length(sql_sha256) = 32),
    manifest_sha256 bytea NOT NULL CHECK (pg_catalog.octet_length(manifest_sha256) = 32),
    database_uuid uuid NOT NULL,
    database_identity_sha256 char(64) NOT NULL CHECK (database_identity_sha256 ~ '^[a-f0-9]{64}$'),
    global_schema_version smallint NOT NULL CHECK (global_schema_version = 5),
    global_manifest_sha256 char(64) NOT NULL CHECK (global_manifest_sha256 ~ '^[a-f0-9]{64}$'),
    required_memory_schema_version smallint NOT NULL CHECK (required_memory_schema_version = 3),
    required_memory_manifest_sha256 char(64) NOT NULL CHECK (required_memory_manifest_sha256 ~ '^[a-f0-9]{64}$'),
    recorded_at timestamptz NOT NULL DEFAULT pg_catalog.clock_timestamp()
);

CREATE TABLE artifact_store.artifact_store_head (
    store_id text PRIMARY KEY CHECK (
        pg_catalog.octet_length(store_id) BETWEEN 1 AND 256
        AND store_id ~ '^[A-Za-z0-9._:-]+$'
    ),
    row_version bigint NOT NULL CHECK (row_version > 0),
    snapshot_bytes bytea NOT NULL CHECK (pg_catalog.octet_length(snapshot_bytes) BETWEEN 1 AND 67108864),
    snapshot_bytes_sha256 bytea NOT NULL CHECK (pg_catalog.octet_length(snapshot_bytes_sha256) = 32),
    checkpoint_bytes bytea NOT NULL CHECK (pg_catalog.octet_length(checkpoint_bytes) BETWEEN 1 AND 16384),
    checkpoint_bytes_sha256 bytea NOT NULL CHECK (pg_catalog.octet_length(checkpoint_bytes_sha256) = 32),
    checkpoint_digest bytea NOT NULL CHECK (pg_catalog.octet_length(checkpoint_digest) = 32),
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.clock_timestamp()
);

CREATE TABLE artifact_store.artifact_store_transition (
    store_id text NOT NULL,
    ordinal bigint NOT NULL CHECK (ordinal > 0),
    expected_checkpoint_digest bytea NOT NULL CHECK (pg_catalog.octet_length(expected_checkpoint_digest) = 32),
    next_checkpoint_digest bytea NOT NULL CHECK (pg_catalog.octet_length(next_checkpoint_digest) = 32),
    snapshot_bytes_sha256 bytea NOT NULL CHECK (pg_catalog.octet_length(snapshot_bytes_sha256) = 32),
    checkpoint_bytes_sha256 bytea NOT NULL CHECK (pg_catalog.octet_length(checkpoint_bytes_sha256) = 32),
    recorded_at timestamptz NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    PRIMARY KEY (store_id, ordinal),
    UNIQUE (store_id, next_checkpoint_digest),
    FOREIGN KEY (store_id) REFERENCES artifact_store.artifact_store_head(store_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE FUNCTION artifact_store.artifact_store_load_for_update_v1(
    p_store_id text,
    p_target_database_name text,
    p_database_identity_sha256 text,
    p_global_manifest_sha256 text,
    p_memory_manifest_sha256 text,
    p_extension_manifest_sha256 text
) RETURNS TABLE (
    row_version bigint,
    snapshot_bytes bytea,
    snapshot_bytes_sha256 bytea,
    checkpoint_bytes bytea,
    checkpoint_bytes_sha256 bytea,
    checkpoint_digest bytea
) LANGUAGE plpgsql VOLATILE PARALLEL UNSAFE SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $artifact_load_for_update_v1$
DECLARE
    v_head artifact_store.artifact_store_head%ROWTYPE;
BEGIN
    IF pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAS03', MESSAGE = 'invalid artifact writer transaction';
    END IF;
    IF pg_catalog.octet_length(p_store_id) NOT BETWEEN 1 AND 256
       OR p_store_id !~ '^[A-Za-z0-9._:-]+$' THEN
        RAISE EXCEPTION USING ERRCODE = 'LAS01', MESSAGE = 'invalid artifact store identity';
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM ONLY control.database_identity d
          CROSS JOIN ONLY control.schema_compatibility c
          CROSS JOIN ONLY memory.codebase_memory_extension_identity m
          CROSS JOIN ONLY artifact_store.artifact_extension_identity e
         WHERE d.singleton AND c.singleton AND m.singleton AND e.singleton
           AND pg_catalog.current_database()=p_target_database_name
           AND m.database_uuid=d.database_uuid AND e.database_uuid=d.database_uuid
           AND pg_catalog.btrim(m.database_identity_sha256::text)=p_database_identity_sha256
           AND pg_catalog.btrim(e.database_identity_sha256::text)=p_database_identity_sha256
           AND c.current_schema_version=5
           AND pg_catalog.btrim(c.manifest_sha256::text)=p_global_manifest_sha256
           AND pg_catalog.btrim(e.global_manifest_sha256::text)=p_global_manifest_sha256
           AND m.extension_schema_version=3
           AND pg_catalog.btrim(m.extension_manifest_sha256::text)=p_memory_manifest_sha256
           AND pg_catalog.btrim(e.required_memory_manifest_sha256::text)=p_memory_manifest_sha256
           AND pg_catalog.encode(e.manifest_sha256,'hex')=p_extension_manifest_sha256
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'LAS05', MESSAGE = 'artifact target identity mismatch';
    END IF;
    SELECT h.* INTO v_head FROM ONLY artifact_store.artifact_store_head h
     WHERE h.store_id=p_store_id FOR UPDATE OF h;
    IF NOT FOUND THEN RETURN; END IF;
    IF v_head.snapshot_bytes_sha256 IS DISTINCT FROM pg_catalog.sha256(v_head.snapshot_bytes)
       OR v_head.checkpoint_bytes_sha256 IS DISTINCT FROM pg_catalog.sha256(v_head.checkpoint_bytes)
       OR NOT EXISTS (
           SELECT 1
             FROM ONLY artifact_store.artifact_store_transition first_transition
             JOIN ONLY artifact_store.artifact_store_transition last_transition
               ON last_transition.store_id=first_transition.store_id
            WHERE first_transition.store_id=p_store_id
              AND first_transition.ordinal=1
              AND last_transition.ordinal=v_head.row_version
              AND first_transition.expected_checkpoint_digest=first_transition.next_checkpoint_digest
              AND last_transition.next_checkpoint_digest=v_head.checkpoint_digest
              AND last_transition.snapshot_bytes_sha256=v_head.snapshot_bytes_sha256
              AND last_transition.checkpoint_bytes_sha256=v_head.checkpoint_bytes_sha256
              AND (SELECT pg_catalog.count(*)
                     FROM ONLY artifact_store.artifact_store_transition counted
                    WHERE counted.store_id=p_store_id)=v_head.row_version
       )
       OR EXISTS (
           SELECT 1
             FROM ONLY artifact_store.artifact_store_transition current_transition
             LEFT JOIN ONLY artifact_store.artifact_store_transition prior_transition
               ON prior_transition.store_id=current_transition.store_id
              AND prior_transition.ordinal=current_transition.ordinal-1
            WHERE current_transition.store_id=p_store_id
              AND current_transition.ordinal>1
              AND (prior_transition.ordinal IS NULL OR
                   current_transition.expected_checkpoint_digest IS DISTINCT FROM
                       prior_transition.next_checkpoint_digest)
       )
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAS01', MESSAGE = 'corrupt artifact physical history';
    END IF;
    RETURN QUERY SELECT v_head.row_version,v_head.snapshot_bytes,
        v_head.snapshot_bytes_sha256,v_head.checkpoint_bytes,
        v_head.checkpoint_bytes_sha256,v_head.checkpoint_digest;
END;
$artifact_load_for_update_v1$;

CREATE FUNCTION artifact_store.artifact_store_commit_snapshot_v1(
    p_store_id text,
    p_expected_checkpoint_digest bytea,
    p_next_snapshot_bytes bytea,
    p_next_snapshot_bytes_sha256 bytea,
    p_next_checkpoint_bytes bytea,
    p_next_checkpoint_bytes_sha256 bytea,
    p_next_checkpoint_digest bytea,
    p_target_database_name text,
    p_database_identity_sha256 text,
    p_global_manifest_sha256 text,
    p_memory_manifest_sha256 text,
    p_extension_manifest_sha256 text
) RETURNS text LANGUAGE plpgsql VOLATILE PARALLEL UNSAFE SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $artifact_commit_snapshot_v1$
DECLARE
    v_head artifact_store.artifact_store_head%ROWTYPE;
    v_next_ordinal bigint;
BEGIN
    IF pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAS03', MESSAGE = 'invalid artifact writer transaction';
    END IF;
    IF pg_catalog.octet_length(p_store_id) NOT BETWEEN 1 AND 256
       OR p_store_id !~ '^[A-Za-z0-9._:-]+$'
       OR pg_catalog.octet_length(p_expected_checkpoint_digest) <> 32
       OR pg_catalog.octet_length(p_next_snapshot_bytes) NOT BETWEEN 1 AND 67108864
       OR p_next_snapshot_bytes_sha256 IS DISTINCT FROM pg_catalog.sha256(p_next_snapshot_bytes)
       OR pg_catalog.octet_length(p_next_checkpoint_bytes) NOT BETWEEN 1 AND 16384
       OR p_next_checkpoint_bytes_sha256 IS DISTINCT FROM pg_catalog.sha256(p_next_checkpoint_bytes)
       OR pg_catalog.octet_length(p_next_checkpoint_digest) <> 32
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAS01', MESSAGE = 'invalid artifact commit boundary';
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM ONLY control.database_identity d
          CROSS JOIN ONLY control.schema_compatibility c
          CROSS JOIN ONLY control.runtime_admission a
          CROSS JOIN ONLY memory.codebase_memory_extension_identity m
          CROSS JOIN ONLY artifact_store.artifact_extension_identity e
         WHERE d.singleton AND c.singleton AND a.singleton AND m.singleton AND e.singleton
           AND a.admission_mode='ACTIVE'
           AND pg_catalog.current_database()=p_target_database_name
           AND m.database_uuid=d.database_uuid AND e.database_uuid=d.database_uuid
           AND pg_catalog.btrim(m.database_identity_sha256::text)=p_database_identity_sha256
           AND pg_catalog.btrim(e.database_identity_sha256::text)=p_database_identity_sha256
           AND c.current_schema_version=5
           AND pg_catalog.btrim(c.manifest_sha256::text)=p_global_manifest_sha256
           AND pg_catalog.btrim(e.global_manifest_sha256::text)=p_global_manifest_sha256
           AND m.extension_schema_version=3
           AND pg_catalog.btrim(m.extension_manifest_sha256::text)=p_memory_manifest_sha256
           AND pg_catalog.btrim(e.required_memory_manifest_sha256::text)=p_memory_manifest_sha256
           AND pg_catalog.encode(e.manifest_sha256,'hex')=p_extension_manifest_sha256
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'LAS05', MESSAGE = 'artifact target authority mismatch';
    END IF;
    SELECT h.* INTO v_head FROM ONLY artifact_store.artifact_store_head h
     WHERE h.store_id=p_store_id FOR UPDATE OF h;
    IF FOUND AND (
       v_head.snapshot_bytes_sha256 IS DISTINCT FROM pg_catalog.sha256(v_head.snapshot_bytes)
       OR v_head.checkpoint_bytes_sha256 IS DISTINCT FROM pg_catalog.sha256(v_head.checkpoint_bytes)
       OR NOT EXISTS (
           SELECT 1
             FROM ONLY artifact_store.artifact_store_transition first_transition
             JOIN ONLY artifact_store.artifact_store_transition last_transition
               ON last_transition.store_id=first_transition.store_id
            WHERE first_transition.store_id=p_store_id
              AND first_transition.ordinal=1
              AND last_transition.ordinal=v_head.row_version
              AND first_transition.expected_checkpoint_digest=first_transition.next_checkpoint_digest
              AND last_transition.next_checkpoint_digest=v_head.checkpoint_digest
              AND last_transition.snapshot_bytes_sha256=v_head.snapshot_bytes_sha256
              AND last_transition.checkpoint_bytes_sha256=v_head.checkpoint_bytes_sha256
              AND (SELECT pg_catalog.count(*)
                     FROM ONLY artifact_store.artifact_store_transition counted
                    WHERE counted.store_id=p_store_id)=v_head.row_version
       )
       OR EXISTS (
           SELECT 1
             FROM ONLY artifact_store.artifact_store_transition current_transition
             LEFT JOIN ONLY artifact_store.artifact_store_transition prior_transition
               ON prior_transition.store_id=current_transition.store_id
              AND prior_transition.ordinal=current_transition.ordinal-1
            WHERE current_transition.store_id=p_store_id
              AND current_transition.ordinal>1
              AND (prior_transition.ordinal IS NULL OR
                   current_transition.expected_checkpoint_digest IS DISTINCT FROM
                       prior_transition.next_checkpoint_digest)
       )
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'LAS01', MESSAGE = 'corrupt artifact physical history';
    END IF;
    IF NOT FOUND THEN
        IF p_expected_checkpoint_digest IS DISTINCT FROM p_next_checkpoint_digest THEN
            RAISE EXCEPTION USING ERRCODE = 'LAS04', MESSAGE = 'stale initial artifact checkpoint';
        END IF;
        INSERT INTO artifact_store.artifact_store_head(
            store_id,row_version,snapshot_bytes,snapshot_bytes_sha256,
            checkpoint_bytes,checkpoint_bytes_sha256,checkpoint_digest
        ) VALUES (
            p_store_id,1,p_next_snapshot_bytes,p_next_snapshot_bytes_sha256,
            p_next_checkpoint_bytes,p_next_checkpoint_bytes_sha256,p_next_checkpoint_digest
        );
        v_next_ordinal := 1;
    ELSIF v_head.checkpoint_digest=p_next_checkpoint_digest THEN
        RETURN 'RETRY';
    ELSE
        IF v_head.checkpoint_digest IS DISTINCT FROM p_expected_checkpoint_digest THEN
            RAISE EXCEPTION USING ERRCODE = 'LAS04', MESSAGE = 'stale artifact checkpoint';
        END IF;
        v_next_ordinal := v_head.row_version + 1;
        UPDATE ONLY artifact_store.artifact_store_head SET
            row_version=v_next_ordinal,
            snapshot_bytes=p_next_snapshot_bytes,
            snapshot_bytes_sha256=p_next_snapshot_bytes_sha256,
            checkpoint_bytes=p_next_checkpoint_bytes,
            checkpoint_bytes_sha256=p_next_checkpoint_bytes_sha256,
            checkpoint_digest=p_next_checkpoint_digest,
            updated_at=pg_catalog.clock_timestamp()
         WHERE store_id=p_store_id AND row_version=v_head.row_version
           AND checkpoint_digest=p_expected_checkpoint_digest;
        IF NOT FOUND THEN
            RAISE EXCEPTION USING ERRCODE = 'LAS04', MESSAGE = 'stale artifact update';
        END IF;
    END IF;
    INSERT INTO artifact_store.artifact_store_transition(
        store_id,ordinal,expected_checkpoint_digest,next_checkpoint_digest,
        snapshot_bytes_sha256,checkpoint_bytes_sha256
    ) VALUES (
        p_store_id,v_next_ordinal,p_expected_checkpoint_digest,p_next_checkpoint_digest,
        p_next_snapshot_bytes_sha256,p_next_checkpoint_bytes_sha256
    );
    RETURN 'COMMITTED';
END;
$artifact_commit_snapshot_v1$;

CREATE FUNCTION artifact_store.artifact_store_load_current_v1(
    p_store_id text,
    p_target_database_name text,
    p_database_identity_sha256 text,
    p_global_manifest_sha256 text,
    p_memory_manifest_sha256 text,
    p_extension_manifest_sha256 text
) RETURNS TABLE (
    row_version bigint,
    snapshot_bytes bytea,
    snapshot_bytes_sha256 bytea,
    checkpoint_bytes bytea,
    checkpoint_bytes_sha256 bytea,
    checkpoint_digest bytea
) LANGUAGE plpgsql STABLE PARALLEL SAFE SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $artifact_load_current_v1$
DECLARE
    v_head artifact_store.artifact_store_head%ROWTYPE;
BEGIN
    IF pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'repeatable read'
       OR NOT pg_catalog.current_setting('transaction_read_only')::boolean
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAS03', MESSAGE = 'invalid artifact reader transaction';
    END IF;
    IF pg_catalog.octet_length(p_store_id) NOT BETWEEN 1 AND 256
       OR p_store_id !~ '^[A-Za-z0-9._:-]+$' THEN
        RAISE EXCEPTION USING ERRCODE = 'LAS01', MESSAGE = 'invalid artifact store identity';
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM ONLY control.database_identity d
          CROSS JOIN ONLY control.schema_compatibility c
          CROSS JOIN ONLY memory.codebase_memory_extension_identity m
          CROSS JOIN ONLY artifact_store.artifact_extension_identity e
         WHERE d.singleton AND c.singleton AND m.singleton AND e.singleton
           AND pg_catalog.current_database()=p_target_database_name
           AND m.database_uuid=d.database_uuid AND e.database_uuid=d.database_uuid
           AND pg_catalog.btrim(m.database_identity_sha256::text)=p_database_identity_sha256
           AND pg_catalog.btrim(e.database_identity_sha256::text)=p_database_identity_sha256
           AND c.current_schema_version=5
           AND pg_catalog.btrim(c.manifest_sha256::text)=p_global_manifest_sha256
           AND pg_catalog.btrim(e.global_manifest_sha256::text)=p_global_manifest_sha256
           AND m.extension_schema_version=3
           AND pg_catalog.btrim(m.extension_manifest_sha256::text)=p_memory_manifest_sha256
           AND pg_catalog.btrim(e.required_memory_manifest_sha256::text)=p_memory_manifest_sha256
           AND pg_catalog.encode(e.manifest_sha256,'hex')=p_extension_manifest_sha256
    ) THEN
        RAISE EXCEPTION USING ERRCODE = 'LAS05', MESSAGE = 'artifact target identity mismatch';
    END IF;
    SELECT h.* INTO v_head FROM ONLY artifact_store.artifact_store_head h
     WHERE h.store_id=p_store_id;
    IF NOT FOUND THEN RETURN; END IF;
    IF v_head.snapshot_bytes_sha256 IS DISTINCT FROM pg_catalog.sha256(v_head.snapshot_bytes)
       OR v_head.checkpoint_bytes_sha256 IS DISTINCT FROM pg_catalog.sha256(v_head.checkpoint_bytes)
       OR NOT EXISTS (
           SELECT 1
             FROM ONLY artifact_store.artifact_store_transition first_transition
             JOIN ONLY artifact_store.artifact_store_transition last_transition
               ON last_transition.store_id=first_transition.store_id
            WHERE first_transition.store_id=p_store_id
              AND first_transition.ordinal=1
              AND last_transition.ordinal=v_head.row_version
              AND first_transition.expected_checkpoint_digest=first_transition.next_checkpoint_digest
              AND last_transition.next_checkpoint_digest=v_head.checkpoint_digest
              AND last_transition.snapshot_bytes_sha256=v_head.snapshot_bytes_sha256
              AND last_transition.checkpoint_bytes_sha256=v_head.checkpoint_bytes_sha256
              AND (SELECT pg_catalog.count(*)
                     FROM ONLY artifact_store.artifact_store_transition counted
                    WHERE counted.store_id=p_store_id)=v_head.row_version
       )
       OR EXISTS (
           SELECT 1
             FROM ONLY artifact_store.artifact_store_transition current_transition
             LEFT JOIN ONLY artifact_store.artifact_store_transition prior_transition
               ON prior_transition.store_id=current_transition.store_id
              AND prior_transition.ordinal=current_transition.ordinal-1
            WHERE current_transition.store_id=p_store_id
              AND current_transition.ordinal>1
              AND (prior_transition.ordinal IS NULL OR
                   current_transition.expected_checkpoint_digest IS DISTINCT FROM
                       prior_transition.next_checkpoint_digest)
       )
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAS01', MESSAGE = 'corrupt artifact physical history';
    END IF;
    RETURN QUERY SELECT v_head.row_version,v_head.snapshot_bytes,
        v_head.snapshot_bytes_sha256,v_head.checkpoint_bytes,
        v_head.checkpoint_bytes_sha256,v_head.checkpoint_digest;
END;
$artifact_load_current_v1$;

REVOKE ALL ON SCHEMA artifact_store FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA artifact_store FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA artifact_store FROM lattice_runtime;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA artifact_store FROM PUBLIC;
GRANT USAGE ON SCHEMA artifact_store TO lattice_runtime;
GRANT EXECUTE ON FUNCTION artifact_store.artifact_store_load_for_update_v1(text,text,text,text,text,text) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION artifact_store.artifact_store_commit_snapshot_v1(text,bytea,bytea,bytea,bytea,bytea,bytea,text,text,text,text,text) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION artifact_store.artifact_store_load_current_v1(text,text,text,text,text,text) TO lattice_runtime;
