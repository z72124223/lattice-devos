# TASK-017 Independent Code And Security Review

## Target

- Worktree: `lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Specification: SPEC-002 version 14, AC-31
- Ticket: TASK-017
- Active contracts: Gateway IPC 1.1, wire protocol 1.0, Contracts 1.7,
  and Ports 1.2
- Final read-only re-review: 2026-08-01

## Review RED Findings And Resolutions

Independent review rejected earlier implementations until every accepted
finding had a direct repair and regression evidence:

- P1: replay lookup occurred before role authorization, allowing an
  unauthorized actor to observe a cached result. Role authorization now runs
  first, role denials are not cached, and both ordering cases are tested.
- P1: required binary, schema, session, expected-head, approval, projection,
  command, routing, terminal, and reply digests could contain an all-zero
  sentinel. Constructors and reply-body validation now reject every required
  zero digest before hashing, replay insertion, or service dispatch.
- P1: reply subject hashing and cloning could occur before the project-status
  page bound was validated. Body validation now precedes reply-subject
  construction and hashing.
- P2: reused task, snapshot, and attempt identifiers could bypass the IPC
  identifier bound. Shared subject validation and stop-target construction now
  enforce the 256-byte limit.
- P2: project-status replies did not enforce the request `page_size`. Replies
  now reject excess rows, project mismatch, and invalid cursor bounds.
- P2: the fake replay map was unbounded. It now has a deterministic 1,024-entry
  cap checked after authorization and exact-retry lookup but before service
  dispatch, preserving readable exact retries while denying new overflow.
- P2: `ProtectedChange` routing contradicted the claim that protected release
  was unrepresentable. Normal protected-change approval routes to Approval
  Verifier; a raw injected `ProtectedRelease` label is rejected by the codec
  and cannot reach the service.
- P2: reply-shape, substitution, role, and service-error coverage was
  incomplete. The regression matrix now covers every reply/disposition, all
  six actions by role, project/actor/subject substitution, and every stable
  service-error mapping.
- P2: duplicate TASK-017 ticket files made ticket identity ambiguous. The
  duplicate was removed, and project governance now machine-checks unique
  frontmatter `ticket_id` values plus exactly one `CURRENT TASK-nnn` marker.

Follow-up review found three additional boundary defects before final PASS:

- P1: non-NFC identifiers could canonicalize into different bytes, and a
  normalization-expanding sequence such as U+0344 could grow after the raw
  limit check. Allocation-free NFC preflight now rejects such input before
  canonical hashing, normalized allocation, replay insertion, or dispatch;
  Plan, Stop, and Project Status projections have direct regressions.
- P1: public encoder fast-fail checked some raw string lengths before proving
  normalized identity and could spend content-proportional normalization work
  on an already oversized frame. Constant-time raw lower-bound resource checks
  now run before NFC inspection, followed by exact iterative depth, node,
  collection, and escaped-byte accounting before canonical allocation.
- P2: `GatewayService` returned a generic component-attributed `PortError`,
  incorrectly assigning core failures to an adapter. Ports 1.2 now exposes
  component-free `GatewayServiceError` and `GatewayServiceResult`; the fake
  maps them to stable bounded redacted protocol failures.

The final architecture recheck also confirmed the Contracts/Gateway ownership
split, protected-release semantics, module/version documentation, and
machine-enforced governance are aligned.

## Final Result

Code review: `PASS`.

Security review: `PASS`.

Remaining findings: P0=0, P1=0, P2=0, P3=0. No blocker remains inside the
bounded pure/fake AC-31 slice.

## Verification Evidence

- Focused Rust: 70/70 tests pass: Contracts 36/36, Gateway IPC 31/31, and
  Ports 3/3.
- Locked full Rust workspace: 358/358 tests pass.
- Preserved Node characterization: 41/41 tests pass, including 3/3 project-
  governance regressions.
- Project check reports
  `check=ok files=221 constitutions=17 tickets=17 current_tasks=1`.
- Strict focused and locked-workspace Clippy with `-D warnings`: pass.
- Rust format and `git diff --check`: pass.
- Normal Gateway IPC dependency tree contains only Contracts, Ports, cjson,
  Serde/JSON, exact `unicode-normalization` 0.1.25, and approved transitive
  SHA-256/normalization dependencies.
- Forbidden filesystem, network, database, process, Git, provider, credential,
  payment, deployment, publication, and unrelated-site scans: zero
  implementation matches.
- Product-isolation scans for Playmate/陪玩, games-preview, Cloudflare, Stripe,
  D1, and Sites: zero matches in the TASK-017 implementation and governance
  surface.
- Codec, role, replay, substitution, protected-release, malformed-frame,
  resource-limit, Unicode, redaction, and service-error matrices all pass with
  zero service call where fail-closed behavior is required.

Reviewed source hashes:

- `crates/lattice-contracts/src/lib.rs`: `a70157cf3b15024fdf0de8dacecf655d88c7e6de2fd06ffda7b560ddd28d3095`
- `crates/lattice-contracts/tests/contracts.rs`: `fff38769e39964f5f746503657f3b22fa3b8315467476ab9777aee926af4afe7`
- `crates/lattice-ports/src/lib.rs`: `1c89ba4a20bffd3d473c6bdf86f853c08bc638181374e4028848815b1c2c4767`
- `crates/lattice-ports/tests/ports.rs`: `4ec8bc7d9942a9bb6fa628311b6631f554dad4a7ba4ade06437901b0f954de0f`
- `crates/lattice-gateway-ipc/src/lib.rs`: `658e7e5358cf828a930503cf0f46f9db5c0e5a4200ad2114fcac048d656787fd`
- `crates/lattice-gateway-ipc/src/typed.rs`: `62a2e79993c642e6743a9b69441edaa95473aaa716e184f0abb94d9e7303c656`
- `crates/lattice-gateway-ipc/src/fake.rs`: `eccb5a6b6711eb26badb873b03e895ab6f7f14d83970bf4adc53273086044582`
- `crates/lattice-gateway-ipc/tests/gateway_ipc.rs`: `8335152949aa94e2f8c32c5411fa6fc4341335a5ac77fb224e6e8829719d856f`
- `scripts/check-project.mjs`: `fd035a55ffc57a3ac25940366d129058ddd3831cc11fd2ac7aaf4783996ec182`
- `test/project-governance-check.test.js`: `f70fd9f92f511f2d91b4d8dd61d3c47f5d029ac18dbdd56b9f67d7286c1a9516`

## Documented Residuals

- The deterministic in-memory loopback fake is not a live local transport,
  listener, OS-authenticated peer, disconnect/restart test, or durable
  PostgreSQL truth.
- Fake peer/service evidence does not prove live OpenClaw loading, exact-
  version compatibility, provider behavior, or complete Orchestrator flow.
- Remote Rust CI, branch protection, a committed candidate, synchronization
  against a remote integration branch, primary-branch merge authorization,
  publication, and deployment remain absent.

These are explicit later-ticket or integration boundaries, not defects in the
bounded TASK-017 pure/fake Gateway IPC implementation.
