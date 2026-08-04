-- LATTICE DevOS Postgres Store 1.1 foundation.
-- Transaction ownership belongs exclusively to the explicit migration runner.
-- The four fixed NOLOGIN group roles must already exist.

CREATE SCHEMA control AUTHORIZATION lattice_migrator;
CREATE SCHEMA memory AUTHORIZATION lattice_migrator;
CREATE SCHEMA readmodel AUTHORIZATION lattice_migrator;

REVOKE ALL ON SCHEMA control FROM PUBLIC;
REVOKE ALL ON SCHEMA memory FROM PUBLIC;
REVOKE ALL ON SCHEMA readmodel FROM PUBLIC;

CREATE TABLE control.database_identity (
    singleton boolean PRIMARY KEY DEFAULT true,
    database_uuid uuid NOT NULL,
    created_at timestamp with time zone NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT database_identity_singleton_true CHECK (singleton),
    CONSTRAINT database_identity_uuid_v8 CHECK (
        database_uuid::text ~ '^[0-9a-f]{8}-[0-9a-f]{4}-8[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$'
        AND database_uuid <> '00000000-0000-0000-0000-000000000000'::uuid
    )
);

CREATE TABLE control.migration_history (
    ordinal smallint PRIMARY KEY,
    migration_id varchar(64) NOT NULL UNIQUE,
    migration_path varchar(256) NOT NULL UNIQUE,
    byte_length bigint NOT NULL,
    checksum_sha256 char(64) NOT NULL,
    migration_status varchar(16) NOT NULL,
    transaction_mode varchar(24) NOT NULL,
    schema_version smallint NOT NULL,
    min_reader smallint NOT NULL,
    max_reader smallint NOT NULL,
    min_writer smallint NOT NULL,
    max_writer smallint NOT NULL,
    recorded_at timestamp with time zone NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT migration_history_positive_ordinal CHECK (ordinal > 0),
    CONSTRAINT migration_history_safe_id CHECK (
        migration_id ~ '^[0-9]{4}_[a-z0-9_]{1,59}$'
    ),
    CONSTRAINT migration_history_safe_path CHECK (
        migration_path ~ '^db/migrations/[0-9]{4}_[a-z0-9_]+[.]sql$'
    ),
    CONSTRAINT migration_history_positive_bytes CHECK (byte_length > 0),
    CONSTRAINT migration_history_sha256 CHECK (
        checksum_sha256 ~ '^[0-9a-f]{64}$'
        AND checksum_sha256 <> repeat('0', 64)
    ),
    CONSTRAINT migration_history_status CHECK (
        migration_status IN ('SUPERSEDED', 'EXECUTABLE')
    ),
    CONSTRAINT migration_history_transaction_mode CHECK (
        transaction_mode IN ('NOT_EXECUTED', 'RUNNER_OWNED')
    ),
    CONSTRAINT migration_history_mode_matches_status CHECK (
        (migration_status = 'SUPERSEDED'
            AND transaction_mode = 'NOT_EXECUTED'
            AND schema_version = 0)
        OR
        (migration_status = 'EXECUTABLE'
            AND transaction_mode = 'RUNNER_OWNED'
            AND schema_version > 0)
    ),
    CONSTRAINT migration_history_reader_range CHECK (
        min_reader >= 0 AND min_reader <= max_reader
    ),
    CONSTRAINT migration_history_writer_range CHECK (
        min_writer >= 0 AND min_writer <= max_writer
    )
);

CREATE TABLE control.schema_compatibility (
    singleton boolean PRIMARY KEY DEFAULT true,
    manifest_sha256 char(64) NOT NULL,
    current_schema_version smallint NOT NULL,
    min_reader smallint NOT NULL,
    max_reader smallint NOT NULL,
    min_writer smallint NOT NULL,
    max_writer smallint NOT NULL,
    updated_at timestamp with time zone NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT schema_compatibility_singleton_true CHECK (singleton),
    CONSTRAINT schema_compatibility_sha256 CHECK (
        manifest_sha256 ~ '^[0-9a-f]{64}$'
        AND manifest_sha256 <> repeat('0', 64)
    ),
    CONSTRAINT schema_compatibility_current_positive CHECK (
        current_schema_version > 0
    ),
    CONSTRAINT schema_compatibility_reader_range CHECK (
        min_reader > 0
        AND min_reader <= current_schema_version
        AND current_schema_version <= max_reader
    ),
    CONSTRAINT schema_compatibility_writer_range CHECK (
        min_writer > 0
        AND min_writer <= current_schema_version
        AND current_schema_version <= max_writer
    )
);

CREATE TABLE control.runtime_admission (
    singleton boolean PRIMARY KEY DEFAULT true,
    admission_mode varchar(32) NOT NULL,
    daemon_instance_id varchar(128),
    daemon_epoch bigint,
    authority_revision bigint NOT NULL DEFAULT 0,
    observation_digest bytea,
    authority_head_digest bytea,
    updated_at timestamp with time zone NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT runtime_admission_singleton_true CHECK (singleton),
    CONSTRAINT runtime_admission_mode_closed CHECK (
        admission_mode IN (
            'ACTIVE',
            'DRAINING',
            'CANARY',
            'STOPPED',
            'RECONCILIATION_REQUIRED'
        )
    ),
    CONSTRAINT runtime_admission_authority_shape CHECK (
        (
            admission_mode = 'STOPPED'
            AND daemon_instance_id IS NULL
            AND daemon_epoch IS NULL
            AND authority_revision = 0
            AND observation_digest IS NULL
            AND authority_head_digest IS NULL
        )
        OR
        (
            daemon_instance_id IS NOT NULL
            AND daemon_instance_id ~ '^[a-z0-9][a-z0-9._:-]{0,127}$'
            AND daemon_epoch IS NOT NULL
            AND daemon_epoch > 0
            AND authority_revision > 0
            AND observation_digest IS NOT NULL
            AND octet_length(observation_digest) = 32
            AND observation_digest <> decode(repeat('00', 32), 'hex')
            AND authority_head_digest IS NOT NULL
            AND octet_length(authority_head_digest) = 32
            AND authority_head_digest <> decode(repeat('00', 32), 'hex')
        )
    )
);

CREATE TABLE control.physical_heads (
    project_id varchar(64) NOT NULL,
    project_snapshot_id varchar(128) NOT NULL,
    repository_owner varchar(32) NOT NULL,
    aggregate_key_digest bytea NOT NULL,
    physical_revision bigint NOT NULL,
    state_digest bytea NOT NULL,
    head_digest bytea NOT NULL,
    updated_at timestamp with time zone NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (
        project_id,
        project_snapshot_id,
        repository_owner,
        aggregate_key_digest
    ),
    CONSTRAINT physical_heads_project_id CHECK (
        project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'
    ),
    CONSTRAINT physical_heads_snapshot_id CHECK (
        project_snapshot_id ~ '^[a-z0-9._:-]{1,128}$'
    ),
    CONSTRAINT physical_heads_owner_closed CHECK (
        repository_owner IN (
            'PROJECT_REGISTRY',
            'TASK_LEDGER',
            'WRITER_LEASE',
            'APPROVAL_VERIFIER',
            'ARTIFACT_STORE'
        )
    ),
    CONSTRAINT physical_heads_aggregate_digest CHECK (
        octet_length(aggregate_key_digest) = 32
        AND aggregate_key_digest <> decode(repeat('00', 32), 'hex')
    ),
    CONSTRAINT physical_heads_revision CHECK (physical_revision >= 0),
    CONSTRAINT physical_heads_state_digest CHECK (
        octet_length(state_digest) = 32
        AND state_digest <> decode(repeat('00', 32), 'hex')
    ),
    CONSTRAINT physical_heads_head_digest CHECK (
        octet_length(head_digest) = 32
        AND head_digest <> decode(repeat('00', 32), 'hex')
    )
);

CREATE TABLE control.terminal_transactions (
    transaction_id varchar(128) PRIMARY KEY,
    project_id varchar(64) NOT NULL,
    project_snapshot_id varchar(128) NOT NULL,
    repository_owner varchar(32) NOT NULL,
    aggregate_key_digest bytea NOT NULL,
    request_digest bytea NOT NULL,
    daemon_instance_id varchar(128) NOT NULL,
    daemon_epoch bigint NOT NULL,
    admission_mode varchar(32) NOT NULL,
    authority_revision bigint NOT NULL,
    authority_observation_digest bytea NOT NULL,
    authority_head_digest bytea NOT NULL,
    expected_revision bigint NOT NULL,
    expected_state_digest bytea NOT NULL,
    expected_head_digest bytea NOT NULL,
    domain_command_digest bytea NOT NULL,
    record_set_digest bytea NOT NULL,
    next_state_digest bytea NOT NULL,
    domain_receipt_digest bytea NOT NULL,
    checkpoint_digest bytea,
    outbox_intent_digest bytea,
    disposition varchar(32) NOT NULL,
    before_revision bigint NOT NULL,
    before_state_digest bytea NOT NULL,
    before_head_digest bytea NOT NULL,
    after_revision bigint NOT NULL,
    after_state_digest bytea NOT NULL,
    after_head_digest bytea NOT NULL,
    transaction_digest bytea NOT NULL,
    receipt_digest bytea NOT NULL,
    recorded_at timestamp with time zone NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT terminal_transactions_safe_id CHECK (
        transaction_id ~ '^[a-z0-9._:-]{1,128}$'
    ),
    CONSTRAINT terminal_transactions_daemon_instance_id CHECK (
        daemon_instance_id ~ '^[a-z0-9][a-z0-9._:-]{0,127}$'
    ),
    CONSTRAINT terminal_transactions_project_id CHECK (
        project_id ~ '^[a-z0-9][a-z0-9._-]{1,63}$'
    ),
    CONSTRAINT terminal_transactions_snapshot_id CHECK (
        project_snapshot_id ~ '^[a-z0-9._:-]{1,128}$'
    ),
    CONSTRAINT terminal_transactions_owner_closed CHECK (
        repository_owner IN (
            'PROJECT_REGISTRY',
            'TASK_LEDGER',
            'WRITER_LEASE',
            'APPROVAL_VERIFIER',
            'ARTIFACT_STORE'
        )
    ),
    CONSTRAINT terminal_transactions_authority_positive CHECK (
        daemon_epoch > 0 AND authority_revision > 0
    ),
    CONSTRAINT terminal_transactions_admission_active CHECK (
        admission_mode = 'ACTIVE'
    ),
    CONSTRAINT terminal_transactions_revisions CHECK (
        expected_revision >= 0
        AND before_revision >= 0
        AND after_revision >= 0
    ),
    CONSTRAINT terminal_transactions_disposition CHECK (
        disposition IN ('APPLIED', 'STALE_PHYSICAL_HEAD')
    ),
    CONSTRAINT terminal_transactions_revision_transition CHECK (
        (disposition = 'APPLIED'
            AND before_revision = expected_revision
            AND before_state_digest = expected_state_digest
            AND before_head_digest = expected_head_digest
            AND after_revision > before_revision
            AND after_revision - before_revision = 1
            AND after_state_digest = next_state_digest)
        OR
        (disposition = 'STALE_PHYSICAL_HEAD'
            AND NOT (
                before_revision = expected_revision
                AND before_state_digest = expected_state_digest
                AND before_head_digest = expected_head_digest
            )
            AND after_revision = before_revision
            AND after_state_digest = before_state_digest
            AND after_head_digest = before_head_digest)
    ),
    CONSTRAINT terminal_transactions_scope_head_fk FOREIGN KEY (
        project_id,
        project_snapshot_id,
        repository_owner,
        aggregate_key_digest
    ) REFERENCES control.physical_heads (
        project_id,
        project_snapshot_id,
        repository_owner,
        aggregate_key_digest
    ),
    CONSTRAINT terminal_transactions_digest_shapes CHECK (
        octet_length(aggregate_key_digest) = 32
        AND octet_length(request_digest) = 32
        AND octet_length(authority_observation_digest) = 32
        AND octet_length(authority_head_digest) = 32
        AND octet_length(expected_state_digest) = 32
        AND octet_length(expected_head_digest) = 32
        AND octet_length(domain_command_digest) = 32
        AND octet_length(record_set_digest) = 32
        AND octet_length(next_state_digest) = 32
        AND octet_length(domain_receipt_digest) = 32
        AND (checkpoint_digest IS NULL OR octet_length(checkpoint_digest) = 32)
        AND (outbox_intent_digest IS NULL OR octet_length(outbox_intent_digest) = 32)
        AND octet_length(before_state_digest) = 32
        AND octet_length(before_head_digest) = 32
        AND octet_length(after_state_digest) = 32
        AND octet_length(after_head_digest) = 32
        AND octet_length(transaction_digest) = 32
        AND octet_length(receipt_digest) = 32
    ),
    CONSTRAINT terminal_transactions_required_nonzero CHECK (
        aggregate_key_digest <> decode(repeat('00', 32), 'hex')
        AND request_digest <> decode(repeat('00', 32), 'hex')
        AND authority_observation_digest <> decode(repeat('00', 32), 'hex')
        AND authority_head_digest <> decode(repeat('00', 32), 'hex')
        AND expected_state_digest <> decode(repeat('00', 32), 'hex')
        AND expected_head_digest <> decode(repeat('00', 32), 'hex')
        AND domain_command_digest <> decode(repeat('00', 32), 'hex')
        AND record_set_digest <> decode(repeat('00', 32), 'hex')
        AND next_state_digest <> decode(repeat('00', 32), 'hex')
        AND domain_receipt_digest <> decode(repeat('00', 32), 'hex')
        AND (checkpoint_digest IS NULL
            OR checkpoint_digest <> decode(repeat('00', 32), 'hex'))
        AND (outbox_intent_digest IS NULL
            OR outbox_intent_digest <> decode(repeat('00', 32), 'hex'))
        AND before_state_digest <> decode(repeat('00', 32), 'hex')
        AND before_head_digest <> decode(repeat('00', 32), 'hex')
        AND after_state_digest <> decode(repeat('00', 32), 'hex')
        AND after_head_digest <> decode(repeat('00', 32), 'hex')
        AND transaction_digest <> decode(repeat('00', 32), 'hex')
        AND receipt_digest <> decode(repeat('00', 32), 'hex')
    )
);

INSERT INTO control.runtime_admission (singleton, admission_mode)
VALUES (true, 'STOPPED');

REVOKE ALL ON ALL TABLES IN SCHEMA control FROM PUBLIC;

GRANT USAGE ON SCHEMA control TO
    lattice_runtime,
    lattice_guardian,
    lattice_readonly;

GRANT SELECT ON
    control.database_identity,
    control.migration_history,
    control.schema_compatibility,
    control.runtime_admission
TO
    lattice_runtime,
    lattice_guardian,
    lattice_readonly;

ALTER DEFAULT PRIVILEGES FOR ROLE lattice_migrator REVOKE ALL ON TABLES FROM PUBLIC;
ALTER DEFAULT PRIVILEGES FOR ROLE lattice_migrator REVOKE ALL ON SEQUENCES FROM PUBLIC;
ALTER DEFAULT PRIVILEGES FOR ROLE lattice_migrator REVOKE ALL ON FUNCTIONS FROM PUBLIC;
ALTER DEFAULT PRIVILEGES FOR ROLE lattice_migrator REVOKE ALL ON TYPES FROM PUBLIC;

COMMENT ON SCHEMA control IS 'LATTICE_DEVOS_CONTROL_SCHEMA_V1';
COMMENT ON SCHEMA memory IS 'LATTICE_DEVOS_MEMORY_SCHEMA_V1';
COMMENT ON SCHEMA readmodel IS 'LATTICE_DEVOS_READMODEL_SCHEMA_V1';
