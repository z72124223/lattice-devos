# ADR-015: Versioned Local Gateway IPC Boundary

- Status: accepted for TASK-017 under the user's directive to continue the
  approved LATTICE plan through MVP-3
- Date: 2026-08-01
- Decision owner: user
- Related: SPEC-002 v14, ADR-004, ADR-006, TASK-017

## Context

The TASK-009 gateway skeleton contains only `Invocation + GatewayAction` and
returns a generic `GatewayEvidence`. It cannot safely represent a new Task Spec
submission, exact status target, normal approval challenge, task-stop target,
terminal routing receipt, exact retry, or an ambiguous outcome. It also labels
the Rust service result as OpenClaw-produced evidence.

Starting a live OpenClaw plugin or choosing a Windows transport before these
semantics are frozen would let transport details become control authority. A
fake protocol boundary must therefore precede live compatibility work.

## Decision

Create pure Rust `lattice-gateway-ipc` 1.1 as the wire-semantic owner of the
versioned gateway protocol, canonical frame codec, parser/encoded-frame limits,
request/reply digests, exact-command replay behavior, and an in-memory fake
loopback. `lattice-contracts` owns the neutral in-process representations,
shared protocol identifiers, and constructor-level identifier/cursor/page
bounds; changing shared values requires a coordinated versioned amendment.

`lattice-contracts` 1.7 carries only neutral immutable request, peer-context,
reply, target, disposition, and denial representations. `lattice-ports` 1.2
keeps `GatewayService`, but its signature becomes:

```rust
fn handle(
    &mut self,
    peer: GatewayPeerContext,
    request: GatewayRequest,
) -> GatewayServiceResult<GatewayReply>;
```

`GatewayServiceError` carries a stable kind/code but no external `Component`;
a Rust-core routing or reply-binding failure cannot be falsely attributed to
OpenClaw. External adapter/store traits continue to use component-attributed
`PortError`.

The peer context is server-derived and separate from untrusted request bytes.
TASK-017 can construct only visibly fake peer context. A request field such as
`authenticated`, `admin`, `human`, or `protected` does not exist.

## Closed Request Set

`GatewayRequestBody` is a closed action-specific enum:

- `Submit(TaskSpecSubmission)`;
- `Plan(ExactTaskTarget)`;
- `Status(StatusTarget)` for project, task, or prior command status;
- `Approve(NormalApprovalRoute)`;
- `Reject(NormalApprovalRoute)`;
- `Stop(TaskStopTarget)`.

Action is derived from the variant. There is no generic action string, shell,
SQL, arbitrary path, provider request, daemon-stop, release rollback, Guardian
activation, or product-writer operation.

Every request binds protocol version, command ID, correlation ID, exact
project/task subject where applicable, and a mechanically verified request
digest. The fake command key is `(project_id, actor_id, command_id)`:

- the same key plus identical request digest returns the same terminal reply;
- the same key plus different content returns `COMMAND_SUBSTITUTION` and does
  not call `GatewayService` again.

Role authorization is checked before this replay lookup. The fake retains at
most 1,024 terminal command records; a new key at capacity fails before a
service call, while an already retained exact retry remains readable.

Live durability for this receipt belongs to PostgreSQL and remains deferred.

## Task Spec Submission

Submit carries:

```text
TaskSpecSubmission {
  schema_id = "lattice.task-spec",
  schema_version = "2.1",
  exact SubjectBinding,
  bounded canonical_document,
  claimed_spec_digest
}
```

The document is canonical JSON using the existing `lattice-cjson-1` mechanism.
The codec rejects non-canonical bytes, duplicate keys, numbers, unknown frame
fields, invalid UTF-8, unknown schema/protocol versions, excessive depth or
node count, and a raw frame larger than 1 MiB before parsing. Typed encoder,
request, and reply inputs must already be NFC; allocation-free preflight rejects
normalization drift or expansion before canonical hashing/allocation. It
recomputes the domain-separated Task Spec digest and checks the binding fields.

IPC does not decide Task Spec validity. Orchestrator later passes the document
to Task Domain, which remains the only owner of Task Spec 2.1 field semantics,
normalization, validation, and `spec_hash`. Raw Task Spec bytes never enter
receipts, `Debug`, or diagnostics.

## Typed Replies

`GatewayReply` binds protocol version, command/correlation ID, action, request
digest, reply digest, and one closed body:

- submit accepted;
- plan routed;
- bounded status observed;
- normal approval or rejection routed;
- stop routed as `REQUESTED`, `ALREADY_TERMINAL`, or
  `RECONCILIATION_REQUIRED`;
- stable denied result;
- explicit unknown outcome.

An ingress or routing receipt proves only what it says. It never proves that a
Task completed, an approval became authority, Codex stopped, a lease released,
or a protected release activated.

## Approval And Stop Boundaries

The normal gateway can route only a bound normal approval challenge and a
digest reference to a presentation. It cannot carry `approved: true`, create
an `ApprovalAuthorityReceipt`, supply a Guardian trust root, expose a nonce or
proof, or satisfy protected-release approval. Protected release is
unrepresentable as a typed request; an injected raw `PROTECTED_RELEASE` label
is a codec rejection before service dispatch. `ProtectedChange` remains a
normal route to the external Approval Verifier and must not be confused with
protected-release authority.

Normal stop means request one exact task/attempt stop. It is not global daemon
stop, process kill, interrupt success, lease release, rollback, or `STOPPED`.
Unknown downstream effect remains `RECONCILIATION_REQUIRED`.

## Dependency Direction

```text
lattice-ports -> lattice-contracts

lattice-gateway-ipc
  -> lattice-contracts
  -> lattice-ports
  -> lattice-cjson
  -> unicode-normalization (exact NFC preflight only)

lattice-orchestrator
  -> lattice-contracts
  -> lattice-task-domain
  -> lattice-policy
  -> lattice-ports

latticed -> lattice-gateway-ipc + lattice-orchestrator + concrete adapters
```

The OpenClaw TypeScript plugin will depend only on the versioned generated IPC
schema/client artifact. `lattice-gateway-ipc` does not depend on Orchestrator,
PostgreSQL, Git, Codex, a provider, credentials, or a product repository.

## TASK-017 Evidence Boundary

TASK-017 implements canonical encode/decode, limits, pure fake peer context,
an in-memory loopback with injected `GatewayService`, deterministic scripted
replies, exact retry/substitution, role allowlists, redacted diagnostics, and
fault simulation. It opens no socket, pipe, or listener and performs no file,
database, process, network, Git, provider, credential, product, payment,
publication, deployment, or protected-release effect.

This closes only the pure/fake portion of AC-07. MVP-2 must separately choose
and verify OS-local transport, ACL/peer identity, session lifecycle, exact
OpenClaw/plugin/schema/binary compatibility, real disconnect/restart behavior,
and PostgreSQL terminal receipts. Fake evidence never implies live support.

## Consequences

- One Gateway gains a complete testable contract without becoming One Truth.
- Task Domain semantics are not copied into the IPC crate.
- The normal gateway cannot represent protected-release authority or direct
  database/Git/provider/product access.
- Live Windows transport remains a replaceable, separately reviewed adapter
  decision.
