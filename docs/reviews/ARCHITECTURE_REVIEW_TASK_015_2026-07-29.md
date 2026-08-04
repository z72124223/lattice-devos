# TASK-015 Architecture Review

## Triggers

- New Approval Verifier semantic owner and public planner/replay/checkpoint API.
- Material Contracts 1.5 and Policy 2.6 public-contract amendments.
- Approval subject, proof, nonce, currentness, revocation, and protected trust-
  lane risk.
- Future PostgreSQL, Review Runtime, OpenClaw, and Guardian composition
  boundaries.

## Independent Result

`PASS`. P0=0, P1=0, P2=0, and P3=0.

Confirmed:

- Contracts 1.5 owns immutable neutral representation and structural
  validation only. It has no dependency, hashing, state-machine, clock, or I/O
  responsibility.
- Approval Verifier 1.0 is the sole semantic owner of complete typed approval
  subject hashing, challenge/proof validation, nonce binding, exact retry,
  currentness, typed revocation, normal claim preconditions, raw replay, and
  trusted-checkpoint comparison.
- Policy 2.6 owns deterministic allow/deny sufficiency only. Its production
  dependencies remain Contracts and Task Domain; Approval Verifier is a one-
  way test-only dependency for real fake-owner composition evidence.
- Approval Verifier's normal graph contains only Contracts, cjson mechanics,
  pinned `time`, and their approved transitive dependencies. It performs no
  filesystem, Git, database, process, network, environment, random, provider,
  credential, payment, publication, deployment, or product-repository I/O.
- Normal claim and protected Guardian claim remain separate. No public
  protected consume command exists.
- Typed revocation is frozen consistently across SPEC-002 v11, ADR-013,
  Approval Verifier constitution 1.0, TASK-015, source, and tests: exact current
  head, eligible verified state, same validity interval, original approver,
  non-zero evidence, revision advance, terminal current-head loss, retry,
  replay, and checkpoint binding.
- The fake proves revocation binding only. Future normal and protected live
  revocation evidence must be authenticated by the OS and Guardian trust
  adapters respectively. Approval Verifier 1.0 has no administrator or
  emergency override.
- PostgreSQL may serialize and persist Verifier state but may not duplicate
  or redefine its state machine or hashes. Guardian owns protected activation
  order, not approval semantics. Review Runtime remains the separate owner of
  independent review authority.
- No second gateway, durable truth, product-code writer, or protected-release
  authority was introduced. The unrelated playmate website remains absent.

## Review RED And Resolution

Architecture review initially rejected the slice because implementation had
chosen revocation authority without governance trace. The implementation used
the original approving actor, exact current head, validity window, and
evidence digest, but SPEC/ADR/constitution/ticket did not freeze that public
lifecycle decision.

The four governance artifacts now explicitly define eligible states, exact
revoker, time/head/evidence requirements, terminal state and replay behavior,
fake-versus-live authentication ownership, and the absence of any 1.0
override. A follow-up P3 found the Verifier constitution's Public Contracts
list still omitted revoke; that list and Owned Data section were amended.
Independent re-review closed both findings.

## Enforcement Truth

| Gate | Classification | Evidence |
|---|---|---|
| Typed subject/challenge/proof/revocation/replay semantics | machine-enforced locally | Rust privacy/closed enums and 28 Approval tests |
| Policy owner receipt/current-head and R3 fail-closed sufficiency | machine-enforced locally | 84 Policy tests |
| Contracts/Verifier/Policy dependency direction | machine-observed locally | manifests and Cargo trees |
| No public protected consume | machine-enforced structurally | closed command enum and tests |
| Dependency/no-I/O/legacy-Boolean boundaries | locally inspected and linted | Clippy and source scans |
| Governance semantics and module ownership | documented plus structurally checked | SPEC v11, ADR-013, constitutions, ticket, project check |
| Live current-head and revocation evidence authentication | documented/deferred | OS/Guardian adapters |
| PostgreSQL uniqueness, DB time, atomic claim, durability, restart | missing/deferred | later MVP-1 store tickets |
| Review Runtime owner authority | missing/deferred | R3 intentionally fails closed |
| One Gateway/One Truth/One Writer at composed runtime | documented-only at this slice | future Orchestrator/PostgreSQL integration |
| Remote Rust CI and branch protection | missing/unverified | current remote workflow is not evidence |

## Verification

- Contracts: 25 tests pass.
- Approval Verifier: 1 unit plus 27 integration tests pass.
- Policy: 84 tests pass.
- Full locked Rust workspace: 218 tests pass.
- Strict locked workspace Clippy with `-D warnings`: pass.
- `cargo fmt --all -- --check`: pass.
- `npm.cmd run verify`: 38 Node tests pass.
- Cargo dependency tree, forbidden-I/O/legacy-Boolean scans,
  `npm.cmd run check`, and `git diff --check`: pass.

## Residual Non-Blocking Owner Work

- Real OS identity, cryptographic verification, key protection, and Guardian
  trust-root evidence are not implemented.
- PostgreSQL must provide global uniqueness, transaction serialization,
  database time, atomic effect/normal claim, checkpoint durability, restart,
  and stale-state revalidation while reusing Verifier semantics.
- Protected claim remains exclusive to future Guardian/PostgreSQL
  `claim_activation`.
- Review Runtime, OpenClaw approval IPC, remote Rust CI, branch protection,
  publication, deployment, and primary-branch merge remain missing or
  separately protected.

These residuals are explicitly outside the bounded pure/fake AC-29 slice and
do not weaken its local architecture result.
