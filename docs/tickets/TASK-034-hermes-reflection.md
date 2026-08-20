---
ticket_id: TASK-034
title: Hermes reflection containment acceptance
module_id: hermes-adapter
status: complete
parallel_safe: false
depends_on: []
branch: feature/task-034-hermes-reflection
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
allowed_paths:
  - docs/tickets/TASK-034-hermes-reflection.md
  - crates/lattice-graphify-adapter/src/identity.rs
  - crates/lattice-hermes-adapter/src/containment.rs
  - crates/lattice-hermes-adapter/src/wsl_outer_runner.py
  - crates/lattice-hermes-adapter/tests/reflection_api.rs
---

# TASK-034 — Hermes reflection containment acceptance

## Objective

Verify the pinned, read-only Hermes reflection adapter and record a terminal
result without treating scripted tests as proof of whole-process containment.

## Result

**COMPLETE — the versioned Ubuntu security-package identity is accepted and
the current-machine containment canary passes.**

The exact WSL/bubblewrap socketpair canary was rerun on 2026-08-21 after the
approved versioned security-pin update and passed against `/usr/bin/bwrap`
SHA-256
`0abea81db798ebf6b4742ac0664802d97521547a353c2a0dbdc21d76cbbfd2c0`.
The historical vulnerable identity
`8e19e40e7d5f7a7e8b488c7926feb040eab6ed10c58fa360e266d2f70670e92b`
is retained only as a rejection fixture and is not executable policy.

## Evidence

- Rejection-first tests failed to compile before implementation because the
  versioned identity constants and validator did not exist (RED, exit 101).
- `cargo test -p lattice-hermes-adapter -p lattice-graphify-adapter
  --all-targets --locked`: all non-ignored tests passed; Hermes 21 passed and
  3 ignored, with the Graphify test binaries also reporting no failures.
- `cargo clippy -p lattice-hermes-adapter -p lattice-graphify-adapter
  --all-targets --locked -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- The ignored real canary
  `tests::reflection_api::wsl_bwrap_socketpair_inherited_fd_canary_is_live_verified`
  was run with `--ignored --exact --nocapture` and passed (1 passed, 0 failed).
- The rejection test proves that the current official binary is accepted while
  both the old vulnerable digest and an unknown digest fail with
  `HERMES_BWRAP_SECURITY_IDENTITY_REJECTED`.

## Scope and next action

No PostgreSQL, TASK-051, TASK-041, Issue 7/8, default branch, merge,
deployment, or release action occurred. The update preserves exact SHA
equality and fail-closed behavior; it does not add an allowlist, bypass, or
caller-selectable identity.

## Security review — 2026-08-21

Ubuntu's official changelog, CVE record, and USN-8288-1 identify
`0.11.1-1ubuntu0.1` as the Resolute security update for CVE-2026-41163. The
patch prevents ptrace while bubblewrap is executing privileged setup; Ubuntu
classifies the issue as a sandbox bypass/local privilege escalation and names
`0.11.1-1ubuntu0.1` as the fixed 26.04 LTS version.

Official references: `https://ubuntu.com/security/CVE-2026-41163` and
`https://ubuntu.com/security/notices/USN-8288-1`.

- Installed security package: `bubblewrap 0.11.1-1ubuntu0.1`.
- Installed binary SHA-256:
  `0abea81db798ebf6b4742ac0664802d97521547a353c2a0dbdc21d76cbbfd2c0`.
- Security package `.deb` SHA-256:
  `b353088d1003adb3f760deeccfb84c47928a36c8dc102bf680efc94eb19f4408`.
- The old and security packages have the same runtime dependencies, file
  count, 0755 non-setuid binary mode, no file capabilities, version output,
  normalized help output, and successful fixed namespace/mount command shape.
  The material difference is the security-patched executable plus its
  changelog and generated manual page.
- An unattended upgrade installed the security package on 2026-08-07. A
  downgrade and package hold were briefly applied before the later safety gate
  arrived; the exact canary passed once in that state. The system was then
  immediately restored to `0.11.1-1ubuntu0.1` and the hold was removed. That
  old-binary pass is not acceptance evidence because it reintroduced the known
  security defect.

The current exact canary hashes the fixed system path `/usr/bin/bwrap` inside
the hard-coded `Ubuntu` distribution. It cannot exercise an unpacked binary in
an isolated path without changing the committed boundary or replacing the
system path, so no safe isolated exact-canary route exists in this version.

## Implemented safe repair and review

The fixed foreman approved a versioned amendment. Graphify execution identity
v1.1 and Hermes private socketpair receipt v2 now bind the exact Ubuntu package
version, official source, `.deb` digest, and executable digest. Hermes performs
one exact executable-digest comparison; the runner and parser carry the same
versioned provenance. The old digest is compiled only in tests and is rejected.

A separate read-only code/security review found no findings: unknown and old
identities fail closed, no hash allowlist or bypass was introduced, and every
duplicate runner/parser/receipt pin agrees. Reviewer independence is not proven
because this was a separate self-review pass. Architecture review was triggered
by the cross-adapter security identity and private receipt revision; it found no
public contract, dependency, data-ownership, database, or module-mission drift.
The security package remains installed without an apt hold.
