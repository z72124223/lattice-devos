# ADR-025: Durable Artifact metadata and disposable owned-root bytes

- Status: accepted
- Date: 2026-08-20
- Decision owners: LATTICE maintainers
- Related: SPEC-002 v37, ADR-014, TASK-025, Artifact Store 1.1,
  PostgreSQL Artifact Store 1.0, Artifact Owned Root 1.0

## Context

Artifact Store 1.0 already owns project-scoped identity, generation,
provenance references, quotas, staging reservations, read claims, retention,
delete claims, terminal receipts, unknown outcomes, reconciliation, strict
metadata replay and independent checkpoints. It deliberately performs no I/O.
TASK-025 must make metadata durable and bytes physically usable without
letting PostgreSQL or a directory become a second semantic owner.

## Decision

1. Artifact Store advances to 1.1 only to expose bounded canonical metadata
   snapshot/checkpoint bytes and one component-free optimistic repository
   contract. Existing 1.0 commands, receipts, hash domains and denial meaning
   remain unchanged.
2. `lattice-postgres-artifact-store` persists complete metadata snapshots,
   independently retained checkpoint fields and immutable transition rows.
   A serializable compare-and-swap locks one store row and accepts a next
   snapshot only after pure bounded replay and exact expected-checkpoint
   comparison. Exact vacant/single-command successor validation retains every
   prior receipt; SQL never invents an Artifact transition.
3. `lattice-artifact-owned-root` owns only byte mechanics under one verified,
   opaque root capability. Operation APIs accept typed object identity and
   streams, never caller paths, globs, SQL, credentials or cleanup roots.
4. Root admission requires an existing exact owner marker, absolute canonical
   root, physical identity, non-reparse regular marker, single link, and
   ancestor/descendant separation from every registered product root.
5. Internal object paths use only a SHA-256 namespace of `project_id`, fixed
   algorithm text, content digest and generation. User filenames, media types,
   schemas and provenance never become path components.
6. Staging uses a pinned same-root temporary-file primitive, incremental
   length/digest bounds, file flush and supported directory flush. Publication
   uses atomic no-clobber rename; a concurrent loser reuses only a digest- and
   length-verified winner.
7. Reads reopen and recheck regular-file identity, link count, length and
   digest. Unlink receives an exact object identity plus non-empty durable
   claim token, rechecks root/containment/file identity and removes exactly one
   file. No recursive deletion or scan-to-authority API exists.
8. Database or filesystem ambiguity never returns success. The composition
   root records the existing `RECONCILIATION_REQUIRED` semantic outcome and
   retains worst-case quota until exact metadata-plus-byte evidence resolves
   it. Directory scanning can quarantine adapter-owned staging names but can
   never publish metadata or promote an orphan.

## Consequences

- PostgreSQL is durable metadata truth; an owned-root object is usable only
  when replay-verified metadata identifies it and a verified read matches it.
- Equal bytes in different projects remain different internal paths and cannot
  leak existence across namespaces.
- The adapter can clean disposable tests only from an explicit in-memory list
  of exact files it created; it cannot accept or recursively traverse a root.
- Cross-module Registry/effect/daemon/capability currentness remains a
  composition transaction concern. The Artifact adapter accepts no caller
  Boolean that can substitute for those checks.

## Rejected alternatives

- Directory scans as object truth or orphan promotion.
- Caller-supplied paths, filenames, cleanup roots or recursive deletion.
- SQL reimplementation of lifecycle, quota, retention or delete legality.
- Cross-project content-addressed paths.
- Treating a successful rename as durable metadata publication.

## Verification

- Pure vectors prove canonical byte bounds, strict parsing, checkpoint
  substitution/rollback denial and repository trait shape.
- PostgreSQL tests prove serializable CAS, exact retry, concurrent stale-writer
  denial, corruption detection, fresh connection/process, restart and closed
  runtime privileges in a marker-owned disposable database.
- Owned-root tests prove unverified/overlapping/reparse/link/path denial,
  empty and bounded streaming, no-clobber races, verified reads, exact unlink,
  quarantined staging and absence of recursive/public path operations.
