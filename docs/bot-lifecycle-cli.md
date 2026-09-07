# Bot lifecycle local interface (v1)

This is a durable role transition interface, not a scheduler or a context store.
PostgreSQL guards only this managed interface. Direct Codex filesystem writes are
outside its enforcement. Native effects must be performed and independently read
back by the Codex adapter; a supplied receipt is evidence, not proof manufactured
by a database test. No task completion state is changed here.

```text
lattice-runtime bot-lifecycle --postgres-host 127.0.0.1 --postgres-port <port> --postgres-run-id <run-id>
```

Send one UTF-8 JSON object on stdin (at most 64 KiB). The existing fixed runtime
credentials apply. Unknown fields/actions fail. Exit 0 returns JSON; exit 2 returns
a bounded error. All identifiers are nonempty bounded ASCII tokens, digests are
64 lowercase hexadecimal characters. Thread IDs are UUIDs; host IDs are explicit.

Read: `{ "action":"read", "project_id":"...", "role_id":"..." }`.
Guard: `{ "action":"assert-owner", "project_id":"...", "role_id":"...",
"expected_generation":1, "owner_thread_id":"uuid", "owner_host_id":"local" }`.
Guard succeeds only in ACTIVE. It is a point-in-time admission check; callers must
carry the same generation into each subsequent managed write, not cache the result.

Every mutation has exactly these envelope fields:

```json
{
  "action": "checkpoint",
  "request_id": "unique-retry-key",
  "project_id": "registered-project-id",
  "role_id": "stable-role-name",
  "expected_revision": 1,
  "expected_generation": 1,
  "owner_thread_id": "00000000-0000-4000-8000-000000000001",
  "owner_host_id": "local",
  "body": {}
}
```

Use the exact same complete request after an uncertain outcome. Reusing an ID with
different content fails. `REPLAYED.receipt` is the original result; `current` is a
separate current role read. A historical success never grants current ownership.
Expected revisions/generations must not be silently refreshed on conflict.

| Action | Exact body | Effect |
|---|---|---|
| register | `work_ids`, `policy_digest`, `rules_digest`, `binding_receipt` | Expected revision/generation both 0; create generation/revision 1, ACTIVE. |
| add-work | `work_ids` | Add unique stable IDs; never remove previous work. |
| checkpoint | `manifest` | Store a new manifest; it must cover the complete work ID set. In PREPARING/VERIFIED it invalidates acknowledgement and returns to PREPARING. |
| prepare | `handoff_id`, `manifest` | ACTIVE → PREPARING; old owner stops admission. |
| reserve-step | `step`, `operation_key`, `input_digest` | Reserve one native effect before dispatch; repeated reservation must be reconciled, not dispatched again. |
| native-step | `step`, `operation_key`, `receipt` | Record native result plus independent readback for the reserved effect. |
| ack | `manifest_digest`, `work_ids`, `verification_receipt` | PREPARING → VERIFIED; receipt targets the created successor, exact current manifest/work set required. Envelope still names current old owner; receipt identifies verifier. |
| commit | `manifest_digest` | VERIFIED → MIGRATING; CAS transfers sole ownership to successor and increments generation. |
| finish | `{}` | MIGRATING → ACTIVE after mappings and old archive are read back. |

The supported steps are `successor_created` (PREPARING), then `routing_updated`,
`schedule_updated`, and `old_archived` (MIGRATING). Each has one stable operation
key/input digest. A RESERVED step without a completed receipt means **unknown
external outcome**. Observe the original target and record its result; do not
blindly create a replacement. The CLI does not invoke any native operation.

`successor_created` must have a different target thread and becomes the candidate.
`routing_updated` and `schedule_updated` target the current new owner. The schedule
receipt may use `schedule-not-configured` only with actual absence-readback evidence;
never create a duplicate schedule merely to satisfy the interface. `old_archived`
targets the old owner, requires both mapping receipts and `old_pending_count:0`.
New pending/in-flight entries must be reconciled before archival. Work IDs remain
retained; v1 deliberately has no delete/retire operation.

Manifest fields (all required, no extra fields):

```json
{
  "work_ids": ["stable-work-id"],
  "pending_decision_ids": [],
  "pending_input_ids": [],
  "in_flight_ids": [],
  "requirements_digest": "<sha256>",
  "authorization_digest": "<sha256>",
  "git_state_digest": "<sha256>",
  "recovery_digest": "<sha256>",
  "evidence_digest": "<sha256>",
  "next_steps_digest": "<sha256>",
  "rules_digest": "<sha256>"
}
```

The database returns `manifest_digest`; use that exact value for ack/commit.
The digest commits to PostgreSQL's canonical JSONB text, not arbitrary input bytes.
The referenced handoff artifacts must actually contain the requirements, authority,
Git/uncommitted state, recovery instructions and evidence; digest presence alone is
not a semantic handoff review.

Receipt fields (all required): `tool`, `target_thread_id`, `target_host_id`,
`result_digest`, `readback_digest`, `evidence_ref`, `success:true`,
`readback_verified:true`, `old_pending_count` (nonnegative integer).
Tools: `create_thread`, `fork_thread`, `read_thread`, `send_message_to_thread`,
`automation_update`, `set_thread_archived`, `hq-routing-update`,
`schedule-not-configured`. Use `read_thread` for register and ack. A native error,
wrong target or missing readback cannot advance the lifecycle. `hq-routing-update`
is a local mapping receipt, explicitly not a Codex native operation.

`abort` accepts `{reason_digest,candidate_disposition_receipt}` before commit only.
The receipt is null only when no successor exists; otherwise it must prove the
candidate was archived with no pending work. Any RESERVED step blocks abort.
MIGRATING cannot roll back generation or owner.

Install explicitly with `lattice-runtime bot-lifecycle-install` and the same three
PostgreSQL options. It creates `lattice_bot_lifecycle_<runId>` on a dedicated
loopback cluster. It rejects a cluster containing `lattice_task019_*` databases:
the original Store verifier permits only its own four login database grants
across the cluster. Sharing those logins across databases violates that boundary.
The original Store database, roles, credentials and installed MCP stay unchanged.
Runtime reads never install schema.
The installer returns `schemaVersion:1`, `port`, and `runId`. An existing database
is verified, never replaced. Partial or changed installations fail closed.

Policy and rules digests are immutable after registration in v1. Changed rules
block handoff until a separately reviewed versioned migration exists. Native
receipt authenticity and project membership must be checked by the caller; this
adapter verifies persisted structure and transition binding, not external tools.

Windows activation uses the installed PostgreSQL 17 binaries and the existing
configured credential without putting it in configuration or command arguments:

```text
node scripts/bot-lifecycle-postgres.mjs init
node scripts/bot-lifecycle-postgres.mjs start
node scripts/bot-lifecycle-postgres.mjs status
node scripts/bot-lifecycle-postgres.mjs stop
```

`init` requires an absent task-owned directory at
`%USERPROFILE%/AppData/Local/LATTICE/bot-lifecycle-postgres/v1`. It creates a private
directory, persistent data, one fixed free port, and identity.json with the cluster
system identifier. It never replaces existing data. `start` is idempotent and
checks the same data directory, loopback listener, port and system identifier;
occupied ports, missing data or partial initialization fail closed. `stop` uses
the same verified directory and PostgreSQL fast shutdown. No service, scheduler,
public listener or cleanup is installed. Recovery is `start` against the existing
identity; an identity or credential mismatch requires diagnosis, not re-init.
The caller can invoke `start` from an already authorized native heartbeat, then
match its port/runId to the private backend config. An OS reboot or scheduled
recovery is not proven merely by a local stop/start test.

Focused acceptance: `cargo +1.97.1 test -p lattice-runtime --test bot_lifecycle_cli
--test project_registry_recovery_cli`, release build, then
`node scripts/test-bot-lifecycle.mjs`. The integration script loads the isolated
cluster identity and retains a new synthetic fixture database. Its receipts do
not claim that actual Codex threads or schedules were changed.
