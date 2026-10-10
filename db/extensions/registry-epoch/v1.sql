-- Explicit offline migration support. Does not disable or replace v1 guards.
CREATE SCHEMA registry_epoch AUTHORIZATION lattice_migrator;
REVOKE ALL ON SCHEMA registry_epoch FROM PUBLIC;
GRANT USAGE ON SCHEMA registry_epoch TO lattice_runtime;

CREATE TABLE registry_epoch.identity (
    singleton boolean PRIMARY KEY CHECK(singleton),
    sql_sha256 text NOT NULL CHECK(sql_sha256 ~ '^[a-f0-9]{64}$')
);
CREATE TABLE registry_epoch.current_seal (
    singleton boolean PRIMARY KEY CHECK(singleton),
    epoch bigint NOT NULL CHECK(epoch > 0),
    seal_digest text NOT NULL CHECK(seal_digest ~ '^[a-f0-9]{64}$'),
    payload jsonb NOT NULL CHECK(jsonb_typeof(payload) = 'object')
);
CREATE TABLE registry_epoch.used_commands (
    command_commitment text PRIMARY KEY CHECK(command_commitment ~ '^[a-f0-9]{64}$'),
    disposition text NOT NULL CHECK(disposition IN ('ARCHIVED','REDACTED'))
);
REVOKE ALL ON ALL TABLES IN SCHEMA registry_epoch FROM PUBLIC, lattice_runtime;

-- The same frozen lattice-hash-1/CJSON string domain as the pure Registry.
CREATE FUNCTION registry_epoch.command_key_v1(p_command_id text)
RETURNS text LANGUAGE sql IMMUTABLE STRICT
SET search_path = pg_catalog
AS $$
SELECT pg_catalog.encode(pg_catalog.sha256(
    pg_catalog.decode('6c6174746963652d686173682d31000006736861323536000f6c6174746963652d636a736f6e2d31002e6c6174746963652e70726f6a6563742d72656769737472792e636f6d6d616e642d69642d636f6d6d69746d656e74000131','hex')
    || pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(pg_catalog.to_json(pg_catalog.normalize(p_command_id, 'NFC'))::text,'UTF8'))::bigint)
    || pg_catalog.convert_to(pg_catalog.to_json(pg_catalog.normalize(p_command_id, 'NFC'))::text,'UTF8')), 'hex')
$$;

CREATE FUNCTION registry_epoch.read_v1()
RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
AS $$
SELECT pg_catalog.jsonb_build_object(
    'sql_sha256', (SELECT sql_sha256 FROM ONLY registry_epoch.identity WHERE singleton),
    'seal', (SELECT CASE WHEN pg_catalog.pg_column_size(payload)<=67108864 THEN pg_catalog.jsonb_build_object('epoch',epoch,'seal_digest',seal_digest,'payload',payload) ELSE '{"oversized":true}'::jsonb END
             FROM ONLY registry_epoch.current_seal WHERE singleton),
    'used_commands', (SELECT COALESCE(pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object('commitment',command_commitment,'disposition',disposition) ORDER BY command_commitment),'[]'::jsonb) FROM (SELECT command_commitment,disposition FROM ONLY registry_epoch.used_commands ORDER BY command_commitment LIMIT 65537) bounded))
$$;

CREATE FUNCTION registry_epoch.guard_used_command_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = on
AS $$
DECLARE retained_disposition text;
BEGIN
    SELECT disposition INTO retained_disposition FROM ONLY registry_epoch.used_commands
      WHERE command_commitment = registry_epoch.command_key_v1(NEW.command_id);
    IF FOUND THEN
        IF retained_disposition = 'REDACTED' THEN
            RAISE EXCEPTION USING ERRCODE='P0001', MESSAGE='REGISTRY_COMMAND_REDACTED';
        END IF;
        RAISE EXCEPTION USING ERRCODE='P0001', MESSAGE='REGISTRY_ARCHIVED_COMMAND_REPLAY_REQUIRED';
    END IF;
    RETURN NEW;
END
$$;

REVOKE ALL ON ALL FUNCTIONS IN SCHEMA registry_epoch FROM PUBLIC, lattice_runtime;
GRANT EXECUTE ON FUNCTION registry_epoch.read_v1() TO lattice_runtime;
CREATE TRIGGER registry_epoch_used_command_guard
BEFORE INSERT ON control.project_registry_commands
FOR EACH ROW EXECUTE FUNCTION registry_epoch.guard_used_command_v1();
