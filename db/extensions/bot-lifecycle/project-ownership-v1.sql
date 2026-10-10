-- Additive ownership and permanent retirement. Original lifecycle functions and
-- survivor receipts are unchanged. Cross-database Registry proof is verified by
-- the native application while it holds the Registry read locks.
CREATE SCHEMA bot_project_ownership AUTHORIZATION lattice_migrator;
REVOKE ALL ON SCHEMA bot_project_ownership FROM PUBLIC;
GRANT USAGE ON SCHEMA bot_project_ownership TO lattice_runtime;
CREATE TABLE bot_project_ownership.bindings (
 project_id text NOT NULL, role_id text NOT NULL,
 store_digest text NOT NULL CHECK(store_digest ~ '^[a-f0-9]{64}$'),
 observation_digest text NOT NULL CHECK(observation_digest ~ '^[a-f0-9]{64}$'),
 PRIMARY KEY(project_id,role_id),
 FOREIGN KEY(project_id,role_id) REFERENCES bot_lifecycle.roles(project_id,role_id)
 DEFERRABLE INITIALLY DEFERRED
);
CREATE TABLE bot_project_ownership.retired (
 pair_digest text PRIMARY KEY CHECK(pair_digest ~ '^[a-f0-9]{64}$')
);
CREATE TABLE bot_project_ownership.receipts (
 operation_digest text PRIMARY KEY CHECK(operation_digest ~ '^[a-f0-9]{64}$'),
 scope_digest text NOT NULL CHECK(scope_digest ~ '^[a-f0-9]{64}$'),
 after_digest text NOT NULL CHECK(after_digest ~ '^[a-f0-9]{64}$'),
 role_count bigint NOT NULL CHECK(role_count>=0),
 event_count bigint NOT NULL CHECK(event_count>=0)
);
CREATE FUNCTION bot_project_ownership.pair_v1(project text,role text) RETURNS text
LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 SELECT encode(sha256(convert_to('lattice.bot-project-retirement.v1'||chr(10)||project||chr(10)||role,'UTF8')),'hex')
$$;
CREATE FUNCTION bot_project_ownership.guard_v1() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF EXISTS(SELECT 1 FROM ONLY bot_project_ownership.retired
   WHERE pair_digest=bot_project_ownership.pair_v1(NEW.project_id,NEW.role_id)) THEN
   RAISE EXCEPTION 'BOT_LIFECYCLE_PROJECT_RETIRED'; END IF;
 IF TG_TABLE_NAME='roles' AND TG_OP='INSERT' AND NOT EXISTS(
   SELECT 1 FROM ONLY bot_project_ownership.bindings
   WHERE project_id=NEW.project_id AND role_id=NEW.role_id) THEN
   RAISE EXCEPTION 'BOT_LIFECYCLE_REGISTRY_BINDING_REQUIRED'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER project_retirement_v1 BEFORE INSERT OR UPDATE ON bot_lifecycle.roles
 FOR EACH ROW EXECUTE FUNCTION bot_project_ownership.guard_v1();
CREATE TRIGGER project_retirement_v1 BEFORE INSERT OR UPDATE ON bot_lifecycle.events
 FOR EACH ROW EXECUTE FUNCTION bot_project_ownership.guard_v1();
CREATE FUNCTION bot_project_ownership.register_v1(p jsonb,store text,observation text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE prior text; result jsonb;
BEGIN
 IF session_user<>'lattice_runtime_login' OR current_setting('role')<>'lattice_runtime'
 OR current_setting('transaction_isolation')<>'serializable'
 OR p->>'action' IS DISTINCT FROM 'register'
 OR store !~ '^[a-f0-9]{64}$' OR observation !~ '^[a-f0-9]{64}$'
 OR store IS NULL OR observation IS NULL THEN RAISE EXCEPTION 'BOT_LIFECYCLE_REGISTRY_BINDING_REQUIRED'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended((p->>'project_id')||'/'||(p->>'role_id'),0));
 SELECT store_digest INTO prior FROM ONLY bot_project_ownership.bindings
 WHERE project_id=p->>'project_id' AND role_id=p->>'role_id';
 IF prior IS NULL THEN
   IF EXISTS(SELECT 1 FROM ONLY bot_lifecycle.roles WHERE project_id=p->>'project_id' AND role_id=p->>'role_id')
   OR EXISTS(SELECT 1 FROM ONLY bot_lifecycle.events WHERE project_id=p->>'project_id' AND role_id=p->>'role_id') THEN
     RAISE EXCEPTION 'BOT_LIFECYCLE_LEGACY_OWNERSHIP_UNATTRIBUTABLE'; END IF;
   INSERT INTO bot_project_ownership.bindings VALUES(p->>'project_id',p->>'role_id',store,observation);
 ELSIF prior<>store THEN RAISE EXCEPTION 'BOT_LIFECYCLE_REGISTRY_BINDING_REQUIRED'; END IF;
 IF to_regprocedure('bot_lifecycle.apply_v2(jsonb)') IS NOT NULL THEN
   EXECUTE 'SELECT bot_lifecycle.apply_v2($1)' INTO result USING p;
 ELSE result:=bot_lifecycle.apply_v1(p); END IF;
 RETURN result;
END $$;
REVOKE ALL ON ALL TABLES IN SCHEMA bot_project_ownership FROM PUBLIC,lattice_runtime;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA bot_project_ownership FROM PUBLIC,lattice_runtime;
GRANT EXECUTE ON FUNCTION bot_project_ownership.register_v1(jsonb,text,text) TO lattice_runtime;
