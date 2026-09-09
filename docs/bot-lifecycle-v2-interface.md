# Bot lifecycle v2 integration interface
Status: implemented candidate interface; isolated database acceptance is separate from live HQ activation.
Scope: control self-handoff only. Other roles retain v1 JSON through the candidate binary. No native effects are invoked by the CLI.

## Commands
All use exactly --postgres-host 127.0.0.1 --postgres-port <port> --postgres-run-id <32 lowercase hex>.
- bot-lifecycle-install: creates v1 only; existing installation is verified without migration and reports its actual schemaVersion (1 or 2).
- bot-lifecycle-migrate: one JSON request on stdin, max 64 KiB. Migrator connection; verifies exact v1 or v2 installation, then atomic schema extension plus one control contract migration. Never installs during a read.
- bot-lifecycle: reads, guards, ordinary v1 and explicit v2 requests on stdin.
The new binary supports both installations. Old binaries fail closed on the extended function/privilege catalog; they are not compatible with a migrated database. Original v1 SQL/function bodies/events remain preserved. Runtime loses direct EXECUTE on apply_v1 after migration; apply_v2 delegates legacy roles internally.

## Migration request (exact keys)
```json
{"action":"migrate-contract","request_id":"control-contract-v2","project_id":"registered-project","role_id":"control","expected_revision":1,"expected_generation":1,"owner_thread_id":"00000000-0000-4000-8000-000000000001","owner_host_id":"local","body":{"from_version":1,"to_version":2,"expected_policy_digest":"<64hex>","expected_rules_digest":"<64hex>","policy_digest":"<new64hex>","rules_digest":"<new64hex>","approval_digest":"<reviewed-contract-and-code64hex>","authorization_receipt":{"tool":"read_thread","target_thread_id":"00000000-0000-4000-8000-000000000001","target_host_id":"local","result_digest":"<64hex>","readback_digest":"<64hex>","evidence_ref":"inbox/bot-lifecycle/decision.json","success":true,"readback_verified":true,"old_pending_count":0}}}
```
Only control ACTIVE with null handoff/manifest/ack and empty steps can migrate. Retains owner, generation and every work ID; revision +1. Same exact request replays; altered content or stale version/revision fails. Other roles are untouched.
Response: {"schema_version":"lattice.bot-lifecycle.v2","status":"APPLIED"|"REPLAYED","receipt":{"request_id":...,"request_digest":...,"action":...,"state":...},"current":...}.
Migration responses additionally return v1_sql_sha256 and v2_sql_sha256 from the verified embedded sources.
State adds contract_version:2 and contract_migration (approved body). Existing state fields retain their meanings.

## v2 mutation envelope (exact keys)
All existing v1 envelope fields PLUS:
```json
{"actor":{"role_id":"lattice_maintenance","thread_id":"actual-uuid","host_id":"local","generation":2},"handoff_id":"same-original-work-id","native":{"old":null,"new":null}}
```
owner_thread_id/host/generation describe expected current CONTROL owner, never impersonate actor.
Actor is checked against that role's ACTIVE current registration in the same project. For new control in MIGRATING, actor matches current control owner/generation and may perform only handoff completion steps.
Actor/executor role rows remain locked through the control transaction commit, so a concurrent replacement cannot overtake the authorized operation.
All v2 requests require these fields, including current control's authorization. Adapter must bind actor to its actual native calling task; UUID possession is not authentication.

Actions:
- authorize-executor: body exactly {executor_role_id:"lattice_maintenance",executor_thread_id,executor_host_id,executor_generation,checkpoint_turn_id,expires_at,authorization_receipt}. Current control actor only, ACTIVE, no live grant; expiry ISO UTC, >now and <=24h. Receipt is actual control read_thread evidence of explicit final-turn authorization. native.old/new null; granting does NOT assert idle. Grant bound to envelope handoff_id/current owner and contract digests; revision +1.
- revoke-executor: body {reason_digest}; current control actor only, ACTIVE before preparation. Revokes grant with evidence; cannot silently abort in-flight handoff.
- add-work/checkpoint/prepare/reserve-step/native-step/ack/commit: existing v1 bodies unchanged; prepare body handoff_id must equal envelope handoff_id. Only exact active granted executor before CAS. reserve/native step restricted to successor_created. Fresh native.old required on every such mutation; ack and commit additionally require native.new. checkpoint/prepare manifests require no pending/inflight inputs.
- commit consumes grant atomically with owner CAS. Afterward executor has no mutation authority.
- reserve-step/native-step for routing_updated/schedule_updated/old_archived, finish: new CONTROL actor only, phase MIGRATING, matching handoff. Fresh native.old required, native.new null. Existing reservation, target, archive readiness and receipt checks stay intact.
- abort: current control actor only, PREPARING/VERIFIED; body unchanged. Existing unknown-effect and candidate-disposition guards remain. native.old/new null (recovery can occur when old wakes); successful abort revokes grant.
Same exact retries return historical receipt plus current state. Replays do not confer authority. Ordinary mutations on a migrated control without v2 envelope fail. Legacy roles cannot use v2 actor fields.

## Native boundary (all keys required, unknown/null counts rejected)
```json
{"source":"codex.read_thread","observed_at":"2026-09-08T00:00:00.000Z","thread_id":"actual-uuid","host_id":"local","latest_turn_id":"actual-uuid","thread_updated_at":1788825600,"status":"idle","latest_turn_status":"completed","pending_input_count":0,"in_flight_count":0,"readback_digest":"<64hex>","evidence_ref":"inbox/bot-lifecycle/native.json"}
```
Database checks age <=300s (at most 5s future), status idle or notLoaded, completed latest turn, explicit zero counts and exact target/source turn. This matches the existing native safeBoundary semantics: archived tasks can be notLoaded without changing their completed turn or updatedAt. Preserve the actual native status; never rewrite notLoaded to idle. HQ must derive from actual native evidence, never manufacture it. First executor operation pins old latest_turn_id/thread_updated_at to handoff_boundary; subsequent pre-CAS and post-CAS operations require unchanged boundary. New boundary at ack/commit must target registered successor and remain identical across ack/commit. Active successor may resume only after CAS.
All source/artifact/inventory verification in HQ prepare/verify stays mandatory; boundary fields cannot replace it.

## Read-only guards
Existing read and assert-owner unchanged; assert-owner only ACTIVE.
New assert-handoff-owner exact fields:
{action:"assert-handoff-owner",project_id,role_id:"control",expected_generation,owner_thread_id,owner_host_id,handoff_id}.
Succeeds HANDOFF_OWNER_CURRENT only for migrated control's current new owner in MIGRATING with matching handoff and accepted CAS state. Does not grant ordinary work before finish/ACTIVE.

## Limitations and activation
### Archived metadata reconciliation

`bot-lifecycle-reconcile-archive` is a separate, explicit migrator command with
the same fixed loopback options and 64 KiB JSON limit. It atomically installs
the additive `archive-reconcile-v2.sql` function and reconciles one exact
control handoff. Existing v1/v2 functions, policies, events and permissions
remain unchanged; the runtime principal cannot execute the recovery function.
Use the candidate binary for all lifecycle readers after activation: old
binaries reject the additional catalog function. A failed request rolls back
both installation and state. Schema version remains 2.

The action is `reconcile-archived-boundary`, using the v2 envelope with the
actual current control actor, fresh `native.old` and null `native.new`. It is
limited to MIGRATING, a consumed executor grant, completed routing/schedule,
and an already reserved old archive operation. The body contains exactly:
`manifest_digest`, `anchor_request_id`, `pre_native_digest`,
`archive_operation_key`, `archive_receipt`, and `history_proof`.
The anchor is the immutable successful commit event; its old native digest
must match the pre-archive observation. The same completed turn is mandatory,
and the actual archived timestamp must be a regression, never forward drift.

`history_proof` contains `pre_turn_digest`, `post_turn_digest`,
`rollout_sha256`, `evidence_digest`, `archived`, `archived_at`,
`durable_updated_at`, `latest_turn_id`, `new_input_count`, `in_flight_count`.
The trusted `scripts/bot-lifecycle-archive-reconcile.mjs` adapter binds the
actual calling task, compares the complete native turn, including command and argument fields, against the
commit-bound observation, rereads the native archive receipt and SQLite
archive status, hashes the archived rollout, and rejects new input or pending
actions. Summary-only native observations are insufficient. Database checks
these closed fields and the exact owner/generation/revision/manifest/operation.

Recovery retains `archive_reconciliation.original_boundary`, the observed
boundary, handoff ID and all evidence in current state and an immutable event.
Only the pinned metadata boundary and revision change. The owner must still
record the existing archive effect using its original operation key, then
finish and pass ACTIVE admission. Do not archive again or edit timestamps.
Exact replay returns the historical receipt without restoring authority.

Adapter CLI: `prepare <HQ-root> <input.json> <saved-request.json>`, then
`execute <HQ-root> <saved-request.json> <result.json> <fresh-self-native-ref>`.
Input includes `actor`, `projectId`, `requestId`, `preNativePath`,
`postNativePath`, `anchorPath`, `archiveNativePath`, `selfNativePath`,
`evidencePath`; source references are relative to the shared HQ root.
Prepare and execute within the original observation's five-minute window.
Unknown outcomes replay identical saved request bytes; never refresh a request
ID's timestamp or revision. Preserve a real failure if evidence expires.

Focused recovery acceptance adds
`$env:LATTICE_BOT_LIFECYCLE_ARCHIVE_RECOVERY = '1'` to the isolated v2 test below,
and `node --test scripts/bot-lifecycle-archive-reconcile.test.mjs`.

Receipts are verified structurally in DB; trusted native adapter verifies authenticity, pending state, source snapshots and caller binding. Tests use explicitly synthetic native receipts. Live HQ DB migration, binary selection, executor grant, candidate creation and actual handoff are separate activation steps owned by CONTROL, not performed by implementation.

If old source changes after CAS, this interface refuses further completion and
retains MIGRATING. It does not invent a new checkpoint or reverse generation;
explicit recovery of new input requires review. Before CAS, owner-authorized abort
retains v2 metadata and revokes the grant, subject to the original unknown-effect
and candidate-disposition rules.

Focused acceptance (PowerShell, explicit candidate path):
```powershell
$env:LATTICE_BOT_LIFECYCLE_TEST_BINARY = 'C:/absolute/candidate/lattice-runtime.exe'
node scripts/test-bot-lifecycle-v2.mjs
node scripts/test-bot-lifecycle.mjs
```
Both scripts create and retain fresh synthetic fixture databases on the dedicated
loopback lifecycle cluster. They never mutate the shared registered roles. The
v2 test retains a deliberately partial fixture as fail-closed evidence separately
from the successfully migrated fixture. No Codex candidate, schedule or archive
is created by these tests.
