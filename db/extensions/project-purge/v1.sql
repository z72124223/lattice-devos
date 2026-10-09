-- Operator-only maintenance receipts. No existing object, grant or trigger changes.
-- The executable validates the exact Store profile and full Registry replay before
-- entering this path. Runtime has no access to this schema or its receipts.
CREATE SCHEMA project_purge AUTHORIZATION lattice_migrator;
CREATE TABLE project_purge.identity (
    singleton boolean PRIMARY KEY CHECK(singleton),
    sql_sha256 text NOT NULL CHECK(sql_sha256 ~ '^[a-f0-9]{64}$')
);
CREATE TABLE project_purge.receipts (
    operation_id text PRIMARY KEY CHECK(operation_id ~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$'),
    scope_digest text NOT NULL CHECK(scope_digest ~ '^[a-f0-9]{64}$'),
    request_digest text NOT NULL CHECK(request_digest ~ '^[a-f0-9]{64}$'),
    result jsonb NOT NULL CHECK(jsonb_typeof(result)='object'),
    completed_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
