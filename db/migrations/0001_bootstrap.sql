-- LATTICE DevOS PostgreSQL namespace draft.
-- This file is intentionally not executed by TASK-008.
-- Tables, roles, grants, extensions, and live connection settings are deferred.

BEGIN;

CREATE SCHEMA IF NOT EXISTS control;
CREATE SCHEMA IF NOT EXISTS memory;
CREATE SCHEMA IF NOT EXISTS readmodel;

COMMIT;
