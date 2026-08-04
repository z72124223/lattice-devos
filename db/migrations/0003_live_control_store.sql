-- LATTICE DevOS Postgres Store 1.2 live physical ControlStore.
-- Transaction ownership belongs exclusively to the Rust adapter or migration runner.
-- The migration runner proves that the v1 physical and terminal tables are empty.

ALTER TABLE ONLY control.terminal_transactions
    DROP CONSTRAINT terminal_transactions_scope_head_fk;

ALTER TABLE control.terminal_transactions
    ADD COLUMN store_contract_version smallint NOT NULL,
    ADD COLUMN producer_id varchar(64) NOT NULL,
    ADD COLUMN producer_version varchar(32) NOT NULL,
    ADD COLUMN runtime varchar(16) NOT NULL,
    ADD COLUMN durability varchar(32) NOT NULL,
    ADD COLUMN database_uuid uuid NOT NULL,
    ADD COLUMN database_identity_digest bytea NOT NULL,
    ADD COLUMN schema_version smallint NOT NULL,
    ADD COLUMN manifest_sha256 char(64) NOT NULL;

ALTER TABLE control.terminal_transactions
    ADD CONSTRAINT terminal_transactions_store_contract_v2 CHECK (
        store_contract_version = 2
    ),
    ADD CONSTRAINT terminal_transactions_producer_exact CHECK (
        producer_id = 'lattice-postgres-store'
        AND producer_version = '1.0'
    ),
    ADD CONSTRAINT terminal_transactions_runtime_live CHECK (
        runtime = 'LIVE'
    ),
    ADD CONSTRAINT terminal_transactions_durability_postgres CHECK (
        durability = 'DURABLE_POSTGRES'
    ),
    ADD CONSTRAINT terminal_transactions_database_uuid_v8 CHECK (
        database_uuid::text ~ '^[0-9a-f]{8}-[0-9a-f]{4}-8[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$'
        AND database_uuid <> '00000000-0000-0000-0000-000000000000'::uuid
    ),
    ADD CONSTRAINT terminal_transactions_database_identity_digest CHECK (
        octet_length(database_identity_digest) = 32
        AND database_identity_digest <> decode(repeat('00', 32), 'hex')
    ),
    ADD CONSTRAINT terminal_transactions_schema_v2 CHECK (
        schema_version = 2
    ),
    ADD CONSTRAINT terminal_transactions_manifest_sha256 CHECK (
        manifest_sha256 ~ '^[0-9a-f]{64}$'
        AND manifest_sha256 <> repeat('0', 64)
    );

CREATE FUNCTION control.store_prepare_v2(
    p_store_contract_version smallint,
    p_transaction_id text,
    p_project_id text,
    p_project_snapshot_id text,
    p_repository_owner text,
    p_aggregate_key_digest bytea,
    p_request_digest bytea,
    p_authority_runtime text,
    p_daemon_instance_id text,
    p_daemon_epoch bigint,
    p_admission_mode text,
    p_authority_revision bigint,
    p_authority_observation_digest bytea,
    p_authority_head_digest bytea,
    p_expected_head_runtime text,
    p_expected_revision bigint,
    p_expected_state_digest bytea,
    p_expected_head_digest bytea,
    p_domain_command_digest bytea,
    p_record_set_digest bytea,
    p_next_state_digest bytea,
    p_domain_receipt_digest bytea,
    p_checkpoint_digest bytea,
    p_outbox_intent_digest bytea,
    p_genesis_state_digest bytea,
    p_genesis_head_digest bytea
)
RETURNS TABLE (
    prepare_status text,
    database_uuid uuid,
    database_identity_digest bytea,
    schema_version smallint,
    manifest_sha256 text,
    head_found boolean,
    before_revision bigint,
    before_state_digest bytea,
    before_head_digest bytea,
    after_revision bigint,
    after_state_digest bytea,
    after_head_digest bytea,
    terminal_disposition text,
    terminal_transaction_digest bytea,
    terminal_receipt_digest bytea
)
LANGUAGE plpgsql
VOLATILE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
AS $lattice_store_prepare_v2$
DECLARE
    v_terminal control.terminal_transactions%ROWTYPE;
    v_database_uuid uuid;
    v_schema_version smallint;
    v_manifest_sha256 text;
    v_min_reader smallint;
    v_max_reader smallint;
    v_min_writer smallint;
    v_max_writer smallint;
    v_admission_mode text;
    v_daemon_instance_id text;
    v_daemon_epoch bigint;
    v_authority_revision bigint;
    v_authority_observation_digest bytea;
    v_authority_head_digest bytea;
    v_before_revision bigint;
    v_before_state_digest bytea;
    v_before_head_digest bytea;
    v_head_found boolean;
    v_digest bytea;
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR pg_catalog.current_setting('transaction_isolation') <> 'serializable'
       OR pg_catalog.current_setting('transaction_read_only')::boolean
       OR pg_catalog.current_setting('synchronous_commit') <> 'on'
       OR p_store_contract_version IS NULL
       OR p_store_contract_version <> 2
       OR p_transaction_id IS NULL
       OR p_transaction_id !~ '^[a-z0-9._:-]{1,128}$'
       OR p_project_id IS NULL
       OR p_project_id !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
       OR p_project_snapshot_id IS NULL
       OR p_project_snapshot_id !~ '^[a-z0-9._:-]{1,128}$'
       OR p_repository_owner IS NULL
       OR p_repository_owner NOT IN (
            'PROJECT_REGISTRY', 'TASK_LEDGER', 'WRITER_LEASE',
            'APPROVAL_VERIFIER', 'ARTIFACT_STORE'
       )
       OR p_authority_runtime IS NULL
       OR p_authority_runtime <> 'LIVE'
       OR p_expected_head_runtime IS NULL
       OR p_expected_head_runtime <> 'LIVE'
       OR p_admission_mode IS NULL
       OR p_admission_mode NOT IN (
            'ACTIVE', 'DRAINING', 'CANARY', 'STOPPED', 'RECONCILIATION_REQUIRED'
       )
       OR p_daemon_instance_id IS NULL
       OR p_daemon_instance_id !~ '^[a-z0-9][a-z0-9._:-]{0,127}$'
       OR p_daemon_epoch IS NULL
       OR p_daemon_epoch <= 0
       OR p_authority_revision IS NULL
       OR p_authority_revision <= 0
       OR p_expected_revision IS NULL
       OR p_expected_revision < 0
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid store request';
    END IF;

    FOREACH v_digest IN ARRAY ARRAY[
        p_aggregate_key_digest,
        p_request_digest,
        p_authority_observation_digest,
        p_authority_head_digest,
        p_expected_state_digest,
        p_expected_head_digest,
        p_domain_command_digest,
        p_record_set_digest,
        p_next_state_digest,
        p_domain_receipt_digest,
        p_genesis_state_digest,
        p_genesis_head_digest
    ]
    LOOP
        IF v_digest IS NULL
           OR pg_catalog.octet_length(v_digest) <> 32
           OR v_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid store digest';
        END IF;
    END LOOP;
    IF (p_checkpoint_digest IS NOT NULL AND (
            pg_catalog.octet_length(p_checkpoint_digest) <> 32
            OR p_checkpoint_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       ))
       OR (p_outbox_intent_digest IS NOT NULL AND (
            pg_catalog.octet_length(p_outbox_intent_digest) <> 32
            OR p_outbox_intent_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       ))
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid optional store digest';
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended('lattice.store.tx.v2:' || p_transaction_id, 0)
    );

    SELECT t.*
      INTO v_terminal
      FROM ONLY control.terminal_transactions AS t
     WHERE t.transaction_id = p_transaction_id
     FOR UPDATE OF t;

    IF FOUND THEN
        IF v_terminal.store_contract_version IS DISTINCT FROM p_store_contract_version
           OR v_terminal.transaction_id IS DISTINCT FROM p_transaction_id
           OR v_terminal.project_id IS DISTINCT FROM p_project_id
           OR v_terminal.project_snapshot_id IS DISTINCT FROM p_project_snapshot_id
           OR v_terminal.repository_owner IS DISTINCT FROM p_repository_owner
           OR v_terminal.aggregate_key_digest IS DISTINCT FROM p_aggregate_key_digest
           OR v_terminal.request_digest IS DISTINCT FROM p_request_digest
           OR v_terminal.runtime IS DISTINCT FROM p_authority_runtime
           OR v_terminal.daemon_instance_id IS DISTINCT FROM p_daemon_instance_id
           OR v_terminal.daemon_epoch IS DISTINCT FROM p_daemon_epoch
           OR v_terminal.admission_mode IS DISTINCT FROM p_admission_mode
           OR v_terminal.authority_revision IS DISTINCT FROM p_authority_revision
           OR v_terminal.authority_observation_digest IS DISTINCT FROM p_authority_observation_digest
           OR v_terminal.authority_head_digest IS DISTINCT FROM p_authority_head_digest
           OR v_terminal.runtime IS DISTINCT FROM p_expected_head_runtime
           OR v_terminal.expected_revision IS DISTINCT FROM p_expected_revision
           OR v_terminal.expected_state_digest IS DISTINCT FROM p_expected_state_digest
           OR v_terminal.expected_head_digest IS DISTINCT FROM p_expected_head_digest
           OR v_terminal.domain_command_digest IS DISTINCT FROM p_domain_command_digest
           OR v_terminal.record_set_digest IS DISTINCT FROM p_record_set_digest
           OR v_terminal.next_state_digest IS DISTINCT FROM p_next_state_digest
           OR v_terminal.domain_receipt_digest IS DISTINCT FROM p_domain_receipt_digest
           OR v_terminal.checkpoint_digest IS DISTINCT FROM p_checkpoint_digest
           OR v_terminal.outbox_intent_digest IS DISTINCT FROM p_outbox_intent_digest
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LTX01', MESSAGE = 'transaction identity substituted';
        END IF;
    END IF;

    SELECT d.database_uuid,
           c.current_schema_version,
           pg_catalog.btrim(c.manifest_sha256::text),
           c.min_reader,
           c.max_reader,
           c.min_writer,
           c.max_writer
      INTO v_database_uuid,
           v_schema_version,
           v_manifest_sha256,
           v_min_reader,
           v_max_reader,
           v_min_writer,
           v_max_writer
      FROM ONLY control.database_identity AS d
      CROSS JOIN ONLY control.schema_compatibility AS c
     WHERE d.singleton = true
       AND c.singleton = true
     FOR SHARE OF d, c;

    IF NOT FOUND
       OR v_schema_version IS DISTINCT FROM 2
       OR v_min_reader IS DISTINCT FROM 2
       OR v_max_reader IS DISTINCT FROM 2
       OR v_min_writer IS DISTINCT FROM 2
       OR v_max_writer IS DISTINCT FROM 2
       OR v_manifest_sha256 IS NULL
       OR v_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR v_manifest_sha256 = pg_catalog.repeat('0', 64)
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'store schema not current';
    END IF;

    IF v_terminal.transaction_id IS NOT NULL THEN
        IF v_terminal.producer_id IS DISTINCT FROM 'lattice-postgres-store'
           OR v_terminal.producer_version IS DISTINCT FROM '1.0'
           OR v_terminal.durability IS DISTINCT FROM 'DURABLE_POSTGRES'
           OR v_terminal.database_uuid IS DISTINCT FROM v_database_uuid
           OR v_terminal.schema_version IS DISTINCT FROM v_schema_version
           OR pg_catalog.btrim(v_terminal.manifest_sha256::text) IS DISTINCT FROM v_manifest_sha256
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'terminal receipt corrupt';
        END IF;
        RETURN QUERY SELECT
            'REPLAY'::text,
            v_terminal.database_uuid,
            v_terminal.database_identity_digest,
            v_terminal.schema_version,
            pg_catalog.btrim(v_terminal.manifest_sha256::text),
            true,
            v_terminal.before_revision,
            v_terminal.before_state_digest,
            v_terminal.before_head_digest,
            v_terminal.after_revision,
            v_terminal.after_state_digest,
            v_terminal.after_head_digest,
            v_terminal.disposition::text,
            v_terminal.transaction_digest,
            v_terminal.receipt_digest;
        RETURN;
    END IF;

    IF p_admission_mode IS DISTINCT FROM 'ACTIVE' THEN
        RAISE EXCEPTION USING ERRCODE = 'LAD01', MESSAGE = 'request admission denied';
    END IF;

    SELECT a.admission_mode,
           a.daemon_instance_id,
           a.daemon_epoch,
           a.authority_revision,
           a.observation_digest,
           a.authority_head_digest
      INTO v_admission_mode,
           v_daemon_instance_id,
           v_daemon_epoch,
           v_authority_revision,
           v_authority_observation_digest,
           v_authority_head_digest
      FROM ONLY control.runtime_admission AS a
     WHERE a.singleton = true
     FOR SHARE OF a;

    IF NOT FOUND OR v_admission_mode IS DISTINCT FROM 'ACTIVE' THEN
        RAISE EXCEPTION USING ERRCODE = 'LAD01', MESSAGE = 'runtime admission denied';
    END IF;

    IF v_daemon_instance_id IS DISTINCT FROM p_daemon_instance_id
       OR v_daemon_epoch IS DISTINCT FROM p_daemon_epoch
       OR v_authority_revision IS DISTINCT FROM p_authority_revision
       OR v_authority_observation_digest IS DISTINCT FROM p_authority_observation_digest
       OR v_authority_head_digest IS DISTINCT FROM p_authority_head_digest
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LAU01', MESSAGE = 'runtime authority mismatch';
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(
            'lattice.store.scope.v2:'
            || p_project_id || ':'
            || p_project_snapshot_id || ':'
            || p_repository_owner || ':'
            || pg_catalog.encode(p_aggregate_key_digest, 'hex'),
            0
        )
    );

    SELECT h.physical_revision,
           h.state_digest,
           h.head_digest
      INTO v_before_revision,
           v_before_state_digest,
           v_before_head_digest
      FROM ONLY control.physical_heads AS h
     WHERE h.project_id = p_project_id
       AND h.project_snapshot_id = p_project_snapshot_id
       AND h.repository_owner = p_repository_owner
       AND h.aggregate_key_digest = p_aggregate_key_digest
     FOR UPDATE OF h;

    v_head_found := FOUND;
    IF NOT v_head_found THEN
        v_before_revision := 0;
        v_before_state_digest := p_genesis_state_digest;
        v_before_head_digest := p_genesis_head_digest;
    END IF;

    RETURN QUERY SELECT
        'PREPARED'::text,
        v_database_uuid,
        NULL::bytea,
        v_schema_version,
        v_manifest_sha256,
        v_head_found,
        v_before_revision,
        v_before_state_digest,
        v_before_head_digest,
        NULL::bigint,
        NULL::bytea,
        NULL::bytea,
        NULL::text,
        NULL::bytea,
        NULL::bytea;
END;
$lattice_store_prepare_v2$;

CREATE FUNCTION control.store_finalize_v2(
    p_store_contract_version smallint,
    p_transaction_id text,
    p_project_id text,
    p_project_snapshot_id text,
    p_repository_owner text,
    p_aggregate_key_digest bytea,
    p_request_digest bytea,
    p_authority_runtime text,
    p_daemon_instance_id text,
    p_daemon_epoch bigint,
    p_admission_mode text,
    p_authority_revision bigint,
    p_authority_observation_digest bytea,
    p_authority_head_digest bytea,
    p_expected_head_runtime text,
    p_expected_revision bigint,
    p_expected_state_digest bytea,
    p_expected_head_digest bytea,
    p_domain_command_digest bytea,
    p_record_set_digest bytea,
    p_next_state_digest bytea,
    p_domain_receipt_digest bytea,
    p_checkpoint_digest bytea,
    p_outbox_intent_digest bytea,
    p_genesis_state_digest bytea,
    p_genesis_head_digest bytea,
    p_database_uuid uuid,
    p_database_identity_digest bytea,
    p_schema_version smallint,
    p_manifest_sha256 text,
    p_before_revision bigint,
    p_before_state_digest bytea,
    p_before_head_digest bytea,
    p_after_revision bigint,
    p_after_state_digest bytea,
    p_after_head_digest bytea,
    p_disposition text,
    p_transaction_digest bytea,
    p_receipt_digest bytea
)
RETURNS text
LANGUAGE plpgsql
VOLATILE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
AS $lattice_store_finalize_v2$
DECLARE
    v_prepare record;
    v_rows bigint;
BEGIN
    SELECT *
      INTO v_prepare
      FROM control.store_prepare_v2(
        p_store_contract_version,
        p_transaction_id,
        p_project_id,
        p_project_snapshot_id,
        p_repository_owner,
        p_aggregate_key_digest,
        p_request_digest,
        p_authority_runtime,
        p_daemon_instance_id,
        p_daemon_epoch,
        p_admission_mode,
        p_authority_revision,
        p_authority_observation_digest,
        p_authority_head_digest,
        p_expected_head_runtime,
        p_expected_revision,
        p_expected_state_digest,
        p_expected_head_digest,
        p_domain_command_digest,
        p_record_set_digest,
        p_next_state_digest,
        p_domain_receipt_digest,
        p_checkpoint_digest,
        p_outbox_intent_digest,
        p_genesis_state_digest,
        p_genesis_head_digest
      );

    IF NOT FOUND THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'prepare returned no row';
    END IF;

    IF v_prepare.prepare_status = 'REPLAY' THEN
        IF v_prepare.database_uuid IS DISTINCT FROM p_database_uuid
           OR v_prepare.database_identity_digest IS DISTINCT FROM p_database_identity_digest
           OR v_prepare.schema_version IS DISTINCT FROM p_schema_version
           OR v_prepare.manifest_sha256 IS DISTINCT FROM p_manifest_sha256
           OR v_prepare.before_revision IS DISTINCT FROM p_before_revision
           OR v_prepare.before_state_digest IS DISTINCT FROM p_before_state_digest
           OR v_prepare.before_head_digest IS DISTINCT FROM p_before_head_digest
           OR v_prepare.after_revision IS DISTINCT FROM p_after_revision
           OR v_prepare.after_state_digest IS DISTINCT FROM p_after_state_digest
           OR v_prepare.after_head_digest IS DISTINCT FROM p_after_head_digest
           OR v_prepare.terminal_disposition IS DISTINCT FROM p_disposition
           OR v_prepare.terminal_transaction_digest IS DISTINCT FROM p_transaction_digest
           OR v_prepare.terminal_receipt_digest IS DISTINCT FROM p_receipt_digest
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'terminal receipt corrupt';
        END IF;
        RETURN 'REPLAY';
    END IF;

    IF v_prepare.prepare_status IS DISTINCT FROM 'PREPARED'
       OR p_database_uuid IS NULL
       OR v_prepare.database_uuid IS DISTINCT FROM p_database_uuid
       OR p_schema_version IS NULL
       OR v_prepare.schema_version IS DISTINCT FROM p_schema_version
       OR p_manifest_sha256 IS NULL
       OR v_prepare.manifest_sha256 IS DISTINCT FROM p_manifest_sha256
       OR p_before_revision IS NULL
       OR v_prepare.before_revision IS DISTINCT FROM p_before_revision
       OR p_before_state_digest IS NULL
       OR v_prepare.before_state_digest IS DISTINCT FROM p_before_state_digest
       OR p_before_head_digest IS NULL
       OR v_prepare.before_head_digest IS DISTINCT FROM p_before_head_digest
       OR p_database_identity_digest IS NULL
       OR pg_catalog.octet_length(p_database_identity_digest) <> 32
       OR p_database_identity_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_after_revision IS NULL
       OR p_after_revision < 0
       OR p_after_state_digest IS NULL
       OR pg_catalog.octet_length(p_after_state_digest) <> 32
       OR p_after_state_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_after_head_digest IS NULL
       OR pg_catalog.octet_length(p_after_head_digest) <> 32
       OR p_after_head_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_transaction_digest IS NULL
       OR pg_catalog.octet_length(p_transaction_digest) <> 32
       OR p_transaction_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_receipt_digest IS NULL
       OR pg_catalog.octet_length(p_receipt_digest) <> 32
       OR p_receipt_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
       OR p_disposition IS NULL
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid finalized receipt';
    END IF;

    IF p_disposition = 'APPLIED' THEN
        IF p_before_revision = 9223372036854775807 THEN
            RAISE EXCEPTION USING ERRCODE = 'LRV01', MESSAGE = 'physical revision exhausted';
        END IF;

        IF p_expected_revision IS DISTINCT FROM p_before_revision
           OR p_expected_state_digest IS DISTINCT FROM p_before_state_digest
           OR p_expected_head_digest IS DISTINCT FROM p_before_head_digest
           OR p_after_revision IS DISTINCT FROM p_before_revision + 1
           OR p_after_state_digest IS DISTINCT FROM p_next_state_digest
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LST02', MESSAGE = 'applied transition invalid';
        END IF;

        IF v_prepare.head_found THEN
            UPDATE ONLY control.physical_heads
               SET physical_revision = p_after_revision,
                   state_digest = p_after_state_digest,
                   head_digest = p_after_head_digest,
                   updated_at = pg_catalog.clock_timestamp()
             WHERE project_id = p_project_id
               AND project_snapshot_id = p_project_snapshot_id
               AND repository_owner = p_repository_owner
               AND aggregate_key_digest = p_aggregate_key_digest
               AND physical_revision = p_before_revision
               AND state_digest = p_before_state_digest
               AND head_digest = p_before_head_digest;
            GET DIAGNOSTICS v_rows = ROW_COUNT;
            IF v_rows <> 1 THEN
                RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'physical head changed';
            END IF;
        ELSE
            INSERT INTO control.physical_heads (
                project_id,
                project_snapshot_id,
                repository_owner,
                aggregate_key_digest,
                physical_revision,
                state_digest,
                head_digest
            ) VALUES (
                p_project_id,
                p_project_snapshot_id,
                p_repository_owner,
                p_aggregate_key_digest,
                p_after_revision,
                p_after_state_digest,
                p_after_head_digest
            );
        END IF;
    ELSIF p_disposition = 'STALE_PHYSICAL_HEAD' THEN
        IF (p_expected_revision IS NOT DISTINCT FROM p_before_revision
            AND p_expected_state_digest IS NOT DISTINCT FROM p_before_state_digest
            AND p_expected_head_digest IS NOT DISTINCT FROM p_before_head_digest)
           OR p_after_revision IS DISTINCT FROM p_before_revision
           OR p_after_state_digest IS DISTINCT FROM p_before_state_digest
           OR p_after_head_digest IS DISTINCT FROM p_before_head_digest
        THEN
            RAISE EXCEPTION USING ERRCODE = 'LST02', MESSAGE = 'stale transition invalid';
        END IF;
    ELSE
        RAISE EXCEPTION USING ERRCODE = 'LST02', MESSAGE = 'unknown store disposition';
    END IF;

    INSERT INTO control.terminal_transactions (
        transaction_id,
        project_id,
        project_snapshot_id,
        repository_owner,
        aggregate_key_digest,
        request_digest,
        daemon_instance_id,
        daemon_epoch,
        admission_mode,
        authority_revision,
        authority_observation_digest,
        authority_head_digest,
        expected_revision,
        expected_state_digest,
        expected_head_digest,
        domain_command_digest,
        record_set_digest,
        next_state_digest,
        domain_receipt_digest,
        checkpoint_digest,
        outbox_intent_digest,
        disposition,
        before_revision,
        before_state_digest,
        before_head_digest,
        after_revision,
        after_state_digest,
        after_head_digest,
        transaction_digest,
        receipt_digest,
        store_contract_version,
        producer_id,
        producer_version,
        runtime,
        durability,
        database_uuid,
        database_identity_digest,
        schema_version,
        manifest_sha256
    ) VALUES (
        p_transaction_id,
        p_project_id,
        p_project_snapshot_id,
        p_repository_owner,
        p_aggregate_key_digest,
        p_request_digest,
        p_daemon_instance_id,
        p_daemon_epoch,
        p_admission_mode,
        p_authority_revision,
        p_authority_observation_digest,
        p_authority_head_digest,
        p_expected_revision,
        p_expected_state_digest,
        p_expected_head_digest,
        p_domain_command_digest,
        p_record_set_digest,
        p_next_state_digest,
        p_domain_receipt_digest,
        p_checkpoint_digest,
        p_outbox_intent_digest,
        p_disposition,
        p_before_revision,
        p_before_state_digest,
        p_before_head_digest,
        p_after_revision,
        p_after_state_digest,
        p_after_head_digest,
        p_transaction_digest,
        p_receipt_digest,
        p_store_contract_version,
        'lattice-postgres-store',
        '1.0',
        'LIVE',
        'DURABLE_POSTGRES',
        p_database_uuid,
        p_database_identity_digest,
        p_schema_version,
        p_manifest_sha256
    );

    RETURN 'FINALIZED';
END;
$lattice_store_finalize_v2$;

CREATE FUNCTION control.store_current_head_v2(
    p_project_id text,
    p_project_snapshot_id text,
    p_repository_owner text,
    p_aggregate_key_digest bytea
)
RETURNS TABLE (
    database_uuid uuid,
    schema_version smallint,
    manifest_sha256 text,
    head_found boolean,
    physical_revision bigint,
    state_digest bytea,
    head_digest bytea
)
LANGUAGE plpgsql
STABLE
PARALLEL UNSAFE
SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
AS $lattice_store_current_head_v2$
BEGIN
    IF session_user <> 'lattice_runtime_login'
       OR pg_catalog.current_setting('role') <> 'lattice_runtime'
       OR p_project_id IS NULL
       OR p_project_id !~ '^[a-z0-9][a-z0-9._-]{1,63}$'
       OR p_project_snapshot_id IS NULL
       OR p_project_snapshot_id !~ '^[a-z0-9._:-]{1,128}$'
       OR p_repository_owner IS NULL
       OR p_repository_owner NOT IN (
            'PROJECT_REGISTRY', 'TASK_LEDGER', 'WRITER_LEASE',
            'APPROVAL_VERIFIER', 'ARTIFACT_STORE'
       )
       OR p_aggregate_key_digest IS NULL
       OR pg_catalog.octet_length(p_aggregate_key_digest) <> 32
       OR p_aggregate_key_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
    THEN
        RAISE EXCEPTION USING ERRCODE = 'LST01', MESSAGE = 'invalid store scope';
    END IF;

    RETURN QUERY
    SELECT d.database_uuid,
           c.current_schema_version,
           pg_catalog.btrim(c.manifest_sha256::text),
           h.project_id IS NOT NULL,
           h.physical_revision,
           h.state_digest,
           h.head_digest
      FROM ONLY control.database_identity AS d
      CROSS JOIN ONLY control.schema_compatibility AS c
      LEFT JOIN ONLY control.physical_heads AS h
        ON h.project_id = p_project_id
       AND h.project_snapshot_id = p_project_snapshot_id
       AND h.repository_owner = p_repository_owner
       AND h.aggregate_key_digest = p_aggregate_key_digest
     WHERE d.singleton = true
       AND c.singleton = true
       AND c.current_schema_version = 2
       AND c.min_reader = 2
       AND c.max_reader = 2
       AND c.min_writer = 2
       AND c.max_writer = 2;
END;
$lattice_store_current_head_v2$;

REVOKE ALL ON FUNCTION control.store_prepare_v2(
    smallint, text, text, text, text, bytea, bytea, text, text, bigint,
    text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea,
    bytea, bytea, bytea, bytea, bytea, bytea
) FROM PUBLIC, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;
REVOKE ALL ON FUNCTION control.store_finalize_v2(
    smallint, text, text, text, text, bytea, bytea, text, text, bigint,
    text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea,
    bytea, bytea, bytea, bytea, bytea, bytea, uuid, bytea, smallint, text,
    bigint, bytea, bytea, bigint, bytea, bytea, text, bytea, bytea
) FROM PUBLIC, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;
REVOKE ALL ON FUNCTION control.store_current_head_v2(
    text, text, text, bytea
) FROM PUBLIC, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;

GRANT EXECUTE ON FUNCTION control.store_prepare_v2(
    smallint, text, text, text, text, bytea, bytea, text, text, bigint,
    text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea,
    bytea, bytea, bytea, bytea, bytea, bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.store_finalize_v2(
    smallint, text, text, text, text, bytea, bytea, text, text, bigint,
    text, bigint, bytea, bytea, text, bigint, bytea, bytea, bytea, bytea,
    bytea, bytea, bytea, bytea, bytea, bytea, uuid, bytea, smallint, text,
    bigint, bytea, bytea, bigint, bytea, bytea, text, bytea, bytea
) TO lattice_runtime;
GRANT EXECUTE ON FUNCTION control.store_current_head_v2(
    text, text, text, bytea
) TO lattice_runtime;

COMMENT ON SCHEMA control IS 'LATTICE_DEVOS_CONTROL_SCHEMA_V2';
COMMENT ON SCHEMA memory IS 'LATTICE_DEVOS_MEMORY_SCHEMA_V2';
COMMENT ON SCHEMA readmodel IS 'LATTICE_DEVOS_READMODEL_SCHEMA_V2';
