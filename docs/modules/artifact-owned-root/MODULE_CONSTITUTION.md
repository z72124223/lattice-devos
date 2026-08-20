---
module_id: artifact-owned-root
name: Artifact Disposable Owned Root Adapter
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-20
---

## Mission

Perform bounded staging, digest verification, flush, atomic no-clobber
publication, verified reads, exact one-object unlink and staging quarantine
inside one physically verified disposable Artifact root.

## Non-Goals

- Own or infer Artifact identity, metadata, references, quota, lifecycle,
  retention, currentness, delete authority or truth.
- Accept caller paths, filenames, globs, cleanup roots, SQL or credentials.
- Scan directories to discover or promote authoritative objects.
- Recursively delete or access any registered product root.

## Owned Data

- One exact owner marker and admitted physical root identity.
- Adapter-created staging names and derived internal object paths.
- Ephemeral physical verification/quarantine observations.

## Public Contracts

- Admission returns an opaque capability only after marker, physical identity,
  containment and product-root separation checks.
- Byte operations accept typed object identity and streams only.
- Unlink accepts one exact object plus non-empty claim token and removes at most
  that verified regular file.

## Invariants

1. The root and all traversed components remain under the admitted canonical
   root and are not reparse points, symlinks, devices or non-directories.
2. Marker/object files are regular, single-link files; alternate data streams
   and device syntax are rejected.
3. Internal paths derive only from hashed project namespace, fixed algorithm,
   digest and generation.
4. Publish never overwrites. A concurrent loser verifies exact winning bytes.
5. Unknown write/rename/unlink outcomes are reported as reconciliation
   required and never imply metadata success or quota release.
6. No recursive deletion API exists. Test cleanup may only remove enumerated
   files and already-empty directories after root re-verification.

## Allowed Dependencies

Rust standard library, `lattice-contracts`, exact pinned `sha2`, `same-file`
and `tempfile` mechanics, plus fixed Windows `fsutil.exe` link enumeration and
`powershell.exe` alternate-stream enumeration. External process arguments are
fixed and internal paths are passed as data, never interpolated into scripts.

## Forbidden Dependencies

Artifact semantic internals, PostgreSQL, Store, Ledger, Registry, Policy,
Approval, Writer Lease, Orchestrator, provider adapters and product roots.

## Failure, Compatibility, And Migration

Unverified, overlapping, linked, reparse, escaped, substituted, corrupt,
oversized, incomplete or ambiguous physical state fails closed. No automatic
recursive repair or cleanup is permitted.

## Acceptance Gates

Disposable filesystem admission, streaming, no-clobber concurrency, verified
read, exact unlink, quarantine and adversarial path/link tests on the current
platform; strict Clippy, format and repository checks.

## Change Policy

Marker, path derivation, identity/link checks, publish/unlink, cleanup or
dependency changes require a versioned constitution and SPEC/ADR trace.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-08-20 | SPEC-002 v37, ADR-025, TASK-025 | Verified disposable root and path-free byte staging/read/exact unlink | User TASK-023-025 development directive |
