SET LOCAL search_path = pg_catalog;

CREATE SCHEMA approval_verifier AUTHORIZATION lattice_migrator;
REVOKE ALL ON SCHEMA approval_verifier FROM PUBLIC;
REVOKE ALL ON SCHEMA approval_verifier FROM lattice_runtime;
REVOKE ALL ON SCHEMA approval_verifier FROM lattice_readonly;
GRANT USAGE ON SCHEMA approval_verifier TO lattice_runtime;

CREATE TABLE approval_verifier.approval_extension_identity (
    singleton boolean PRIMARY KEY DEFAULT true,
    extension_id varchar(64) NOT NULL,
    schema_version smallint NOT NULL,
    sql_sha256 bytea NOT NULL,
    manifest_sha256 bytea NOT NULL,
    database_uuid uuid NOT NULL,
    database_identity_sha256 char(64) NOT NULL,
    global_schema_version smallint NOT NULL,
    global_manifest_sha256 char(64) NOT NULL,
    required_memory_schema_version smallint NOT NULL,
    required_memory_manifest_sha256 char(64) NOT NULL,
    installed_at timestamp with time zone NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    CONSTRAINT approval_extension_identity_singleton CHECK (singleton),
    CONSTRAINT approval_extension_identity_id CHECK (extension_id = 'lattice-approval-verifier'),
    CONSTRAINT approval_extension_identity_version CHECK (schema_version = 1),
    CONSTRAINT approval_extension_identity_sql_digest CHECK (
        pg_catalog.octet_length(sql_sha256) = 32
        AND sql_sha256 <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    ),
    CONSTRAINT approval_extension_identity_manifest_digest CHECK (
        pg_catalog.octet_length(manifest_sha256) = 32
        AND manifest_sha256 <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    ),
    CONSTRAINT approval_extension_identity_database_digest CHECK (
        database_identity_sha256 ~ '^[0-9a-f]{64}$'
        AND database_identity_sha256 <> pg_catalog.repeat('0', 64)
    ),
    CONSTRAINT approval_extension_identity_global_profile CHECK (
        global_schema_version = 5
        AND global_manifest_sha256 ~ '^[0-9a-f]{64}$'
        AND global_manifest_sha256 <> pg_catalog.repeat('0', 64)
    ),
    CONSTRAINT approval_extension_identity_memory_profile CHECK (
        required_memory_schema_version = 3
        AND required_memory_manifest_sha256 ~ '^[0-9a-f]{64}$'
        AND required_memory_manifest_sha256 <> pg_catalog.repeat('0', 64)
    )
);

CREATE TABLE approval_verifier.approval_extension_ledger (
    ordinal bigint PRIMARY KEY,
    event_type varchar(32) NOT NULL,
    extension_id varchar(64) NOT NULL,
    schema_version smallint NOT NULL,
    sql_sha256 bytea NOT NULL,
    manifest_sha256 bytea NOT NULL,
    database_uuid uuid NOT NULL,
    database_identity_sha256 char(64) NOT NULL,
    global_schema_version smallint NOT NULL,
    global_manifest_sha256 char(64) NOT NULL,
    required_memory_schema_version smallint NOT NULL,
    required_memory_manifest_sha256 char(64) NOT NULL,
    recorded_at timestamp with time zone NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    CONSTRAINT approval_extension_ledger_ordinal CHECK (ordinal = 1),
    CONSTRAINT approval_extension_ledger_event CHECK (event_type = 'INSTALLED'),
    CONSTRAINT approval_extension_ledger_identity CHECK (
        extension_id = 'lattice-approval-verifier' AND schema_version = 1
    ),
    CONSTRAINT approval_extension_ledger_sql_digest CHECK (
        pg_catalog.octet_length(sql_sha256) = 32
        AND sql_sha256 <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    ),
    CONSTRAINT approval_extension_ledger_manifest_digest CHECK (
        pg_catalog.octet_length(manifest_sha256) = 32
        AND manifest_sha256 <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    ),
    CONSTRAINT approval_extension_ledger_database_digest CHECK (
        database_identity_sha256 ~ '^[0-9a-f]{64}$'
        AND database_identity_sha256 <> pg_catalog.repeat('0', 64)
    ),
    CONSTRAINT approval_extension_ledger_global_profile CHECK (
        global_schema_version = 5
        AND global_manifest_sha256 ~ '^[0-9a-f]{64}$'
        AND global_manifest_sha256 <> pg_catalog.repeat('0', 64)
    ),
    CONSTRAINT approval_extension_ledger_memory_profile CHECK (
        required_memory_schema_version = 3
        AND required_memory_manifest_sha256 ~ '^[0-9a-f]{64}$'
        AND required_memory_manifest_sha256 <> pg_catalog.repeat('0', 64)
    )
);

CREATE TABLE approval_verifier.approval_heads (
    singleton boolean PRIMARY KEY DEFAULT true,
    row_version bigint NOT NULL,
    snapshot_bytes bytea NOT NULL,
    snapshot_bytes_sha256 bytea NOT NULL,
    command_high_water bigint NOT NULL,
    command_tail_digest bytea,
    nonce_bindings_digest bytea NOT NULL,
    snapshot_digest bytea NOT NULL,
    updated_at timestamp with time zone NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    CONSTRAINT approval_heads_singleton CHECK (singleton),
    CONSTRAINT approval_heads_row_version CHECK (row_version >= 0),
    CONSTRAINT approval_heads_snapshot_bytes CHECK (
        pg_catalog.octet_length(snapshot_bytes) BETWEEN 1 AND 8388608
    ),
    CONSTRAINT approval_heads_snapshot_sha CHECK (
        pg_catalog.octet_length(snapshot_bytes_sha256) = 32
        AND snapshot_bytes_sha256 = pg_catalog.sha256(snapshot_bytes)
    ),
    CONSTRAINT approval_heads_high_water CHECK (
        command_high_water >= 0
        AND ((command_high_water = 0) = (command_tail_digest IS NULL))
    ),
    CONSTRAINT approval_heads_tail_digest CHECK (
        command_tail_digest IS NULL OR (
            pg_catalog.octet_length(command_tail_digest) = 32
            AND command_tail_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        )
    ),
    CONSTRAINT approval_heads_nonce_digest CHECK (
        pg_catalog.octet_length(nonce_bindings_digest) = 32
        AND nonce_bindings_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    ),
    CONSTRAINT approval_heads_snapshot_digest CHECK (
        pg_catalog.octet_length(snapshot_digest) = 32
        AND snapshot_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    )
);

CREATE TABLE approval_verifier.approval_commands (
    ordinal bigint PRIMARY KEY,
    command_id varchar(128) NOT NULL UNIQUE,
    approval_id varchar(128) NOT NULL,
    repository_request_bytes bytea NOT NULL,
    repository_request_sha256 bytea NOT NULL,
    command_bytes bytea NOT NULL,
    command_bytes_sha256 bytea NOT NULL,
    receipt_bytes bytea NOT NULL,
    receipt_bytes_sha256 bytea NOT NULL,
    receipt_digest bytea NOT NULL UNIQUE,
    outcome varchar(16) NOT NULL,
    denial_reason varchar(64),
    recorded_at timestamp with time zone NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    CONSTRAINT approval_commands_ordinal CHECK (ordinal > 0),
    CONSTRAINT approval_commands_command_id CHECK (
        command_id ~ '^[a-z0-9][a-z0-9._:-]{0,127}$'
    ),
    CONSTRAINT approval_commands_approval_id CHECK (
        approval_id ~ '^[a-z0-9][a-z0-9._:-]{0,127}$'
    ),
    CONSTRAINT approval_commands_request_bytes CHECK (
        pg_catalog.octet_length(repository_request_bytes) BETWEEN 1 AND 1048576
        AND repository_request_sha256 = pg_catalog.sha256(repository_request_bytes)
    ),
    CONSTRAINT approval_commands_command_bytes CHECK (
        pg_catalog.octet_length(command_bytes) BETWEEN 1 AND 1048576
        AND command_bytes_sha256 = pg_catalog.sha256(command_bytes)
    ),
    CONSTRAINT approval_commands_receipt_bytes CHECK (
        pg_catalog.octet_length(receipt_bytes) BETWEEN 1 AND 1048576
        AND receipt_bytes_sha256 = pg_catalog.sha256(receipt_bytes)
    ),
    CONSTRAINT approval_commands_receipt_digest CHECK (
        pg_catalog.octet_length(receipt_digest) = 32
        AND receipt_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    ),
    CONSTRAINT approval_commands_outcome CHECK (
        (outcome = 'APPLIED' AND denial_reason IS NULL)
        OR (outcome = 'DENIED' AND denial_reason IS NOT NULL)
    )
);

CREATE TABLE approval_verifier.approval_effect_claims (
    command_id varchar(128) PRIMARY KEY REFERENCES approval_verifier.approval_commands(command_id),
    approval_id varchar(128) NOT NULL,
    effect_kind varchar(128) NOT NULL,
    effect_id varchar(128) NOT NULL,
    effect_digest bytea NOT NULL,
    request_bytes bytea NOT NULL,
    request_bytes_sha256 bytea NOT NULL,
    observed_at text NOT NULL,
    daemon_instance_id varchar(128) NOT NULL,
    daemon_epoch bigint NOT NULL,
    admission_mode varchar(32) NOT NULL,
    claim_digest bytea NOT NULL UNIQUE,
    approval_receipt_digest bytea NOT NULL UNIQUE,
    effect_receipt_digest bytea NOT NULL UNIQUE,
    recorded_at timestamp with time zone NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    CONSTRAINT approval_effect_claims_approval_id CHECK (
        approval_id ~ '^[a-z0-9][a-z0-9._:-]{0,127}$'
    ),
    CONSTRAINT approval_effect_claims_effect_kind CHECK (
        effect_kind ~ '^[a-z0-9][a-z0-9._:-]{0,127}$'
    ),
    CONSTRAINT approval_effect_claims_effect_id CHECK (
        effect_id ~ '^[a-z0-9][a-z0-9._:-]{0,127}$'
    ),
    CONSTRAINT approval_effect_claims_effect_digest CHECK (
        pg_catalog.octet_length(effect_digest) = 32
        AND effect_digest <> pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    ),
    CONSTRAINT approval_effect_claims_request_bytes CHECK (
        pg_catalog.octet_length(request_bytes) BETWEEN 1 AND 1048576
        AND request_bytes_sha256 = pg_catalog.sha256(request_bytes)
    ),
    CONSTRAINT approval_effect_claims_observed_at CHECK (
        observed_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,6})?Z$'
    ),
    CONSTRAINT approval_effect_claims_daemon CHECK (
        daemon_instance_id ~ '^[a-z0-9][a-z0-9._:-]{0,127}$' AND daemon_epoch > 0
    ),
    CONSTRAINT approval_effect_claims_admission CHECK (admission_mode = 'ACTIVE'),
    CONSTRAINT approval_effect_claims_digests CHECK (
        pg_catalog.octet_length(claim_digest) = 32
        AND pg_catalog.octet_length(approval_receipt_digest) = 32
        AND pg_catalog.octet_length(effect_receipt_digest) = 32
    )
);

REVOKE ALL ON ALL TABLES IN SCHEMA approval_verifier FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA approval_verifier FROM lattice_runtime;
REVOKE ALL ON ALL TABLES IN SCHEMA approval_verifier FROM lattice_readonly;

CREATE FUNCTION approval_verifier.approval_verifier_load_for_update_v1(
    p_command_id text,
    p_initial_snapshot_bytes bytea,
    p_initial_snapshot_bytes_sha256 bytea,
    p_initial_nonce_bindings_digest bytea,
    p_initial_snapshot_digest bytea,
    p_target_database_name text,
    p_database_identity_sha256 text,
    p_global_manifest_sha256 text,
    p_memory_manifest_sha256 text,
    p_extension_manifest_sha256 text
) RETURNS TABLE (
    row_version bigint,
    snapshot_bytes bytea,
    snapshot_bytes_sha256 bytea,
    command_high_water bigint,
    command_tail_digest bytea,
    nonce_bindings_digest bytea,
    snapshot_digest bytea,
    existing_repository_request_bytes bytea,
    existing_repository_request_sha256 bytea,
    existing_effect_kind text,
    existing_effect_id text,
    existing_effect_digest bytea,
    existing_effect_request_bytes bytea,
    existing_effect_observed_at text,
    existing_effect_daemon_instance_id text,
    existing_effect_daemon_epoch bigint,
    existing_effect_admission_mode text,
    existing_effect_claim_digest bytea,
    existing_effect_approval_receipt_digest bytea,
    existing_effect_receipt_digest bytea,
    observed_at text,
    admission_mode text,
    daemon_instance_id text,
    daemon_epoch bigint
) LANGUAGE plpgsql VOLATILE PARALLEL UNSAFE SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $approval_load_for_update_v1$
DECLARE
    v_head approval_verifier.approval_heads%ROWTYPE;
    v_admission control.runtime_admission%ROWTYPE;
    v_request_bytes bytea;
    v_request_sha bytea;
    v_effect approval_verifier.approval_effect_claims%ROWTYPE;
    v_observed_at text;
BEGIN
    IF pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAV03', MESSAGE = 'invalid approval writer transaction';
    END IF;
    IF p_command_id !~ '^[a-z0-9][a-z0-9._:-]{0,127}$'
       OR pg_catalog.octet_length(p_initial_snapshot_bytes) NOT BETWEEN 1 AND 8388608
       OR p_initial_snapshot_bytes_sha256 IS DISTINCT FROM pg_catalog.sha256(p_initial_snapshot_bytes)
       OR pg_catalog.octet_length(p_initial_nonce_bindings_digest) <> 32
       OR pg_catalog.octet_length(p_initial_snapshot_digest) <> 32
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAV01', MESSAGE = 'invalid approval load boundary';
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM ONLY control.database_identity d
          CROSS JOIN ONLY control.schema_compatibility c
          CROSS JOIN ONLY memory.codebase_memory_extension_identity m
          CROSS JOIN ONLY approval_verifier.approval_extension_identity e
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
        RAISE EXCEPTION USING ERRCODE = 'LAV05', MESSAGE = 'approval target identity mismatch';
    END IF;

    INSERT INTO approval_verifier.approval_heads (
        singleton,row_version,snapshot_bytes,snapshot_bytes_sha256,command_high_water,
        command_tail_digest,nonce_bindings_digest,snapshot_digest
    ) VALUES (
        true,0,p_initial_snapshot_bytes,p_initial_snapshot_bytes_sha256,0,
        NULL,p_initial_nonce_bindings_digest,p_initial_snapshot_digest
    ) ON CONFLICT (singleton) DO NOTHING;

    SELECT h.* INTO STRICT v_head
      FROM ONLY approval_verifier.approval_heads AS h
     WHERE h.singleton FOR UPDATE OF h;
    SELECT a.* INTO STRICT v_admission
      FROM ONLY control.runtime_admission AS a
     WHERE a.singleton FOR SHARE OF a;
    SELECT c.repository_request_bytes,c.repository_request_sha256
      INTO v_request_bytes,v_request_sha
      FROM ONLY approval_verifier.approval_commands AS c
     WHERE c.command_id=p_command_id;
    SELECT e.* INTO v_effect
      FROM ONLY approval_verifier.approval_effect_claims e
     WHERE e.command_id=p_command_id;

    v_observed_at := pg_catalog.to_char(
        pg_catalog.transaction_timestamp() AT TIME ZONE 'UTC',
        'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'
    );
    v_observed_at := pg_catalog.regexp_replace(v_observed_at, '(\.[0-9]*[1-9])0+Z$', '\1Z');
    v_observed_at := pg_catalog.regexp_replace(v_observed_at, '\.0+Z$', 'Z');

    RETURN QUERY SELECT
        v_head.row_version,v_head.snapshot_bytes,v_head.snapshot_bytes_sha256,
        v_head.command_high_water,v_head.command_tail_digest,v_head.nonce_bindings_digest,
        v_head.snapshot_digest,v_request_bytes,v_request_sha,
        v_effect.effect_kind::text,v_effect.effect_id::text,v_effect.effect_digest,
        v_effect.request_bytes,v_effect.observed_at,v_effect.daemon_instance_id::text,
        v_effect.daemon_epoch,v_effect.admission_mode::text,v_effect.claim_digest,
        v_effect.approval_receipt_digest,v_effect.effect_receipt_digest,
        v_observed_at,
        v_admission.admission_mode::text,v_admission.daemon_instance_id::text,
        v_admission.daemon_epoch;
END;
$approval_load_for_update_v1$;

CREATE FUNCTION approval_verifier.approval_verifier_commit_plan_v1(
    p_expected_row_version bigint,
    p_expected_snapshot_digest bytea,
    p_observed_at text,
    p_admission_mode text,
    p_daemon_instance_id text,
    p_daemon_epoch bigint,
    p_next_snapshot_bytes bytea,
    p_next_snapshot_bytes_sha256 bytea,
    p_next_command_high_water bigint,
    p_next_command_tail_digest bytea,
    p_next_nonce_bindings_digest bytea,
    p_next_snapshot_digest bytea,
    p_command_id text,
    p_approval_id text,
    p_repository_request_bytes bytea,
    p_repository_request_sha256 bytea,
    p_command_bytes bytea,
    p_command_bytes_sha256 bytea,
    p_receipt_bytes bytea,
    p_receipt_bytes_sha256 bytea,
    p_receipt_digest bytea,
    p_outcome text,
    p_denial_reason text,
    p_effect_kind text,
    p_effect_id text,
    p_effect_digest bytea,
    p_effect_request_bytes bytea,
    p_effect_request_bytes_sha256 bytea,
    p_claim_digest bytea,
    p_effect_receipt_digest bytea
) RETURNS boolean LANGUAGE plpgsql VOLATILE PARALLEL UNSAFE SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $approval_commit_plan_v1$
DECLARE
    v_admission control.runtime_admission%ROWTYPE;
    v_observed_at text;
    v_updated bigint;
BEGIN
    IF pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAV03', MESSAGE = 'invalid approval writer transaction';
    END IF;
    SELECT a.* INTO STRICT v_admission
      FROM ONLY control.runtime_admission AS a
     WHERE a.singleton FOR SHARE OF a;
    v_observed_at := pg_catalog.to_char(
        pg_catalog.transaction_timestamp() AT TIME ZONE 'UTC',
        'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'
    );
    v_observed_at := pg_catalog.regexp_replace(v_observed_at, '(\.[0-9]*[1-9])0+Z$', '\1Z');
    v_observed_at := pg_catalog.regexp_replace(v_observed_at, '\.0+Z$', 'Z');
    IF p_observed_at IS DISTINCT FROM v_observed_at
       OR p_admission_mode IS DISTINCT FROM v_admission.admission_mode
       OR p_daemon_instance_id IS DISTINCT FROM v_admission.daemon_instance_id
       OR p_daemon_epoch IS DISTINCT FROM v_admission.daemon_epoch
       OR p_next_command_high_water <> p_expected_row_version + 1
       OR p_next_snapshot_bytes_sha256 IS DISTINCT FROM pg_catalog.sha256(p_next_snapshot_bytes)
       OR p_repository_request_sha256 IS DISTINCT FROM pg_catalog.sha256(p_repository_request_bytes)
       OR p_command_bytes_sha256 IS DISTINCT FROM pg_catalog.sha256(p_command_bytes)
       OR p_receipt_bytes_sha256 IS DISTINCT FROM pg_catalog.sha256(p_receipt_bytes)
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAV02', MESSAGE = 'invalid approval commit boundary';
    END IF;

    UPDATE ONLY approval_verifier.approval_heads SET
        row_version=row_version+1,
        snapshot_bytes=p_next_snapshot_bytes,
        snapshot_bytes_sha256=p_next_snapshot_bytes_sha256,
        command_high_water=p_next_command_high_water,
        command_tail_digest=p_next_command_tail_digest,
        nonce_bindings_digest=p_next_nonce_bindings_digest,
        snapshot_digest=p_next_snapshot_digest,
        updated_at=pg_catalog.clock_timestamp()
     WHERE singleton AND row_version=p_expected_row_version
       AND snapshot_digest=p_expected_snapshot_digest;
    GET DIAGNOSTICS v_updated = ROW_COUNT;
    IF v_updated <> 1 THEN
        RAISE EXCEPTION USING ERRCODE = 'LAV03', MESSAGE = 'stale approval head';
    END IF;

    INSERT INTO approval_verifier.approval_commands (
        ordinal,command_id,approval_id,repository_request_bytes,repository_request_sha256,
        command_bytes,command_bytes_sha256,receipt_bytes,receipt_bytes_sha256,
        receipt_digest,outcome,denial_reason
    ) VALUES (
        p_next_command_high_water,p_command_id,p_approval_id,p_repository_request_bytes,
        p_repository_request_sha256,p_command_bytes,p_command_bytes_sha256,p_receipt_bytes,
        p_receipt_bytes_sha256,p_receipt_digest,p_outcome,p_denial_reason
    );

    IF p_effect_receipt_digest IS NOT NULL THEN
        IF p_outcome <> 'APPLIED' OR p_effect_kind IS NULL OR p_effect_id IS NULL
           OR p_effect_digest IS NULL OR p_effect_request_bytes IS NULL
           OR p_effect_request_bytes_sha256 IS DISTINCT FROM pg_catalog.sha256(p_effect_request_bytes)
           OR p_claim_digest IS NULL OR p_admission_mode <> 'ACTIVE'
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LAV04', MESSAGE = 'invalid normal effect claim';
        END IF;
        INSERT INTO approval_verifier.approval_effect_claims (
            command_id,approval_id,effect_kind,effect_id,effect_digest,request_bytes,
            request_bytes_sha256,observed_at,daemon_instance_id,daemon_epoch,admission_mode,
            claim_digest,approval_receipt_digest,effect_receipt_digest
        ) VALUES (
            p_command_id,p_approval_id,p_effect_kind,p_effect_id,p_effect_digest,
            p_effect_request_bytes,p_effect_request_bytes_sha256,p_observed_at,
            p_daemon_instance_id,p_daemon_epoch,p_admission_mode,p_claim_digest,
            p_receipt_digest,p_effect_receipt_digest
        );
    ELSIF p_effect_kind IS NOT NULL OR p_effect_id IS NOT NULL OR p_effect_digest IS NOT NULL
       OR p_effect_request_bytes IS NOT NULL OR p_effect_request_bytes_sha256 IS NOT NULL
       OR p_claim_digest IS NOT NULL
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAV04', MESSAGE = 'partial normal effect claim';
    END IF;
    RETURN true;
END;
$approval_commit_plan_v1$;

CREATE FUNCTION approval_verifier.approval_verifier_load_current_v1(
    p_approval_id text,
    p_target_database_name text,
    p_database_identity_sha256 text,
    p_global_manifest_sha256 text,
    p_memory_manifest_sha256 text,
    p_extension_manifest_sha256 text
) RETURNS TABLE (
    snapshot_bytes bytea,
    snapshot_bytes_sha256 bytea,
    command_high_water bigint,
    command_tail_digest bytea,
    nonce_bindings_digest bytea,
    snapshot_digest bytea,
    observed_at text,
    admission_mode text,
    daemon_instance_id text,
    daemon_epoch bigint
) LANGUAGE plpgsql STABLE PARALLEL SAFE SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $approval_load_current_v1$
DECLARE
    v_head approval_verifier.approval_heads%ROWTYPE;
    v_admission control.runtime_admission%ROWTYPE;
    v_observed_at text;
BEGIN
    IF pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'repeatable read'
       OR NOT pg_catalog.current_setting('transaction_read_only')::boolean
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAV03', MESSAGE = 'invalid approval reader transaction';
    END IF;
    IF p_approval_id !~ '^[a-z0-9][a-z0-9._:-]{0,127}$' THEN
        RAISE EXCEPTION USING ERRCODE = 'LAV01', MESSAGE = 'invalid approval identity';
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM ONLY control.database_identity d
          CROSS JOIN ONLY control.schema_compatibility c
          CROSS JOIN ONLY memory.codebase_memory_extension_identity m
          CROSS JOIN ONLY approval_verifier.approval_extension_identity e
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
        RAISE EXCEPTION USING ERRCODE = 'LAV05', MESSAGE = 'approval target identity mismatch';
    END IF;
    SELECT h.* INTO v_head FROM ONLY approval_verifier.approval_heads AS h WHERE h.singleton;
    IF NOT FOUND THEN RETURN; END IF;
    SELECT a.* INTO STRICT v_admission FROM ONLY control.runtime_admission AS a WHERE a.singleton;
    v_observed_at := pg_catalog.to_char(
        pg_catalog.transaction_timestamp() AT TIME ZONE 'UTC',
        'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'
    );
    v_observed_at := pg_catalog.regexp_replace(v_observed_at, '(\.[0-9]*[1-9])0+Z$', '\1Z');
    v_observed_at := pg_catalog.regexp_replace(v_observed_at, '\.0+Z$', 'Z');
    RETURN QUERY SELECT
        v_head.snapshot_bytes,v_head.snapshot_bytes_sha256,v_head.command_high_water,
        v_head.command_tail_digest,v_head.nonce_bindings_digest,v_head.snapshot_digest,
        v_observed_at,v_admission.admission_mode::text,v_admission.daemon_instance_id::text,
        v_admission.daemon_epoch;
END;
$approval_load_current_v1$;

CREATE FUNCTION approval_verifier.approval_verifier_load_commands_v1()
RETURNS TABLE (
    ordinal bigint,
    command_id text,
    approval_id text,
    repository_request_bytes bytea,
    repository_request_sha256 bytea,
    command_bytes bytea,
    command_bytes_sha256 bytea,
    receipt_bytes bytea,
    receipt_bytes_sha256 bytea,
    receipt_digest bytea,
    outcome text,
    denial_reason text
) LANGUAGE plpgsql STABLE PARALLEL SAFE SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $approval_load_commands_v1$
BEGIN
    IF pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR NOT (
           (pg_catalog.current_setting('transaction_isolation') = 'serializable'
            AND NOT pg_catalog.current_setting('transaction_read_only')::boolean
            AND pg_catalog.current_setting('synchronous_commit') = 'on')
           OR
           (pg_catalog.current_setting('transaction_isolation') = 'repeatable read'
            AND pg_catalog.current_setting('transaction_read_only')::boolean)
       )
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAV03', MESSAGE = 'invalid approval history transaction';
    END IF;
    RETURN QUERY
    SELECT c.ordinal,c.command_id::text,c.approval_id::text,
           c.repository_request_bytes,c.repository_request_sha256,
           c.command_bytes,c.command_bytes_sha256,c.receipt_bytes,
           c.receipt_bytes_sha256,c.receipt_digest,c.outcome::text,
           c.denial_reason::text
      FROM ONLY approval_verifier.approval_commands c
     ORDER BY c.ordinal;
END;
$approval_load_commands_v1$;

CREATE FUNCTION approval_verifier.approval_verifier_load_effects_v1()
RETURNS TABLE (
    command_id text,
    approval_id text,
    effect_kind text,
    effect_id text,
    effect_digest bytea,
    request_bytes bytea,
    request_bytes_sha256 bytea,
    observed_at text,
    daemon_instance_id text,
    daemon_epoch bigint,
    admission_mode text,
    claim_digest bytea,
    approval_receipt_digest bytea,
    effect_receipt_digest bytea
) LANGUAGE plpgsql STABLE PARALLEL SAFE SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
SET lock_timeout = '5s'
SET statement_timeout = '30s'
AS $approval_load_effects_v1$
BEGIN
    IF pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR NOT (
           (pg_catalog.current_setting('transaction_isolation') = 'serializable'
            AND NOT pg_catalog.current_setting('transaction_read_only')::boolean
            AND pg_catalog.current_setting('synchronous_commit') = 'on')
           OR
           (pg_catalog.current_setting('transaction_isolation') = 'repeatable read'
            AND pg_catalog.current_setting('transaction_read_only')::boolean)
       )
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAV03', MESSAGE = 'invalid approval history transaction';
    END IF;
    RETURN QUERY
    SELECT e.command_id::text,e.approval_id::text,e.effect_kind::text,e.effect_id::text,
           e.effect_digest,e.request_bytes,e.request_bytes_sha256,e.observed_at,
           e.daemon_instance_id::text,e.daemon_epoch,e.admission_mode::text,e.claim_digest,
           e.approval_receipt_digest,e.effect_receipt_digest
      FROM ONLY approval_verifier.approval_effect_claims e
     ORDER BY e.command_id;
END;
$approval_load_effects_v1$;

REVOKE ALL ON ALL FUNCTIONS IN SCHEMA approval_verifier FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA approval_verifier FROM lattice_readonly;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA approval_verifier FROM lattice_runtime;
GRANT EXECUTE ON FUNCTION approval_verifier.approval_verifier_load_for_update_v1(
    text,bytea,bytea,bytea,bytea,text,text,text,text,text
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION approval_verifier.approval_verifier_commit_plan_v1(
    bigint,bytea,text,text,text,bigint,bytea,bytea,bigint,bytea,bytea,bytea,
    text,text,bytea,bytea,bytea,bytea,bytea,bytea,bytea,text,text,text,text,
    bytea,bytea,bytea,bytea,bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION approval_verifier.approval_verifier_load_current_v1(
    text,text,text,text,text,text
)
TO lattice_runtime;
GRANT EXECUTE ON FUNCTION approval_verifier.approval_verifier_load_commands_v1()
TO lattice_runtime;
GRANT EXECUTE ON FUNCTION approval_verifier.approval_verifier_load_effects_v1()
TO lattice_runtime;
