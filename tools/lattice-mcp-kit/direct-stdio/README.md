# LATTICE MCP Kit: direct stdio client

Canonical `latticed` exposes the exact seven-tool MCP catalog below. The
PowerShell helper in this directory launches a specified executable over stdio
and writes one redacted result directory per invocation. Its `TaskSubmit`
shortcut supports Phase 3 general-task creation while retaining the Phase 2
canary request when no objective is supplied. The separate file-based typed
contract coordinator remains deliberately canary-only.

## Current tool catalog

- `lattice_delivery_run`
- `lattice_delivery_status`
- `lattice_runtime_status`
- `lattice_delivery_reconcile`
- `lattice_task_submit`
- `lattice_task_status`
- `lattice_foreman_checkpoint`

Discovery fails closed with `TOOL_SET_MISMATCH` if the executable advertises
anything other than this exact set. Successful summaries use
`schema=lattice.direct-stdio-client.v2` and `discovery.exact_seven=true`.

## Actions

- `Discovery` initializes MCP and verifies the exact seven-tool catalog without
  calling a tool.
- `TaskSubmit` with `-Objective` sends a general task plus zero or one of
  `-ProjectId` / `-ProjectName`. Without `-Objective`, it retains the exact
  `CONTROLLED_CODEX_CANARY` request for compatibility.
- `TaskStatus` defaults to calling `lattice_task_status` with only a lowercase
  64-character `task_ref`. If `-ClientRequestId` is explicitly bound, the
  wrapper also sends it so a retained legacy canary can be queried.
- `Call` is the generic low-level action and requires an explicit tool name and
  JSON arguments.

Each invocation starts a fresh child process. For cross-session use, retain the `task_ref` returned by `TaskSubmit`, then pass it to a later `TaskStatus` invocation. The client does not treat its own process or output directory as durable task truth.

## General task submission

Use `lattice_task_submit` when a user asks LATTICE to create, record, track, or
durably resume a task for an already registered project. The recommended
request shape is:

```json
{
  "client_request_id": "codex-character-system-001",
  "objective": "完成角色系統",
  "project_name": "AI 劇本"
}
```

`project_id` may be used instead of `project_name`. Supply neither only when
the eligible Control catalog contains exactly one project. Never supply both.
The general-task `intent` field is accepted as a compatibility alias for
`objective`; new callers should prefer `objective`. A caller never supplies a
project path, Registry receipt, Task Spec, command, permission, model, lease,
approval, workspace, or execution setting.

Control Project Catalog data is only a locator. A successful submission also
requires a replay-verified current project in the PostgreSQL Project Registry;
`registry_authority: NONE` in Control is not promoted into authority. Missing,
ambiguous, unreadable, drifted, or unregistered projects return a typed
repairable error rather than selecting another project.
Control-valid display names that are non-NFC or exceed the formal
64-scalar/256-byte task bound remain catalog locators but cannot become task
data: selecting an otherwise eligible row returns
`REGISTERED_PROJECT_NAME_UNSUPPORTED`. Rename or re-register that display name
canonically. Legacy rows are not formal binding candidates, so they do not
poison an eligible project's exact-ID, unique-name, or selector-free selection;
an exact ID that selects a legacy row instead returns `PROJECT_IS_NOT_REGISTERED`.
Likewise, a recognized secret-shaped project ID cannot become a formal task
binding: MCP rejects caller-supplied values before dispatch, while selecting an
otherwise eligible unsupported catalog ID by name returns
`REGISTERED_PROJECT_ID_UNSUPPORTED` before any Registry mutation.

`client_request_id` is a strict idempotency key inside the server-owned ingress.
The same key with the same objective and formal project returns the same
`task_ref`; the same key with another objective or project is rejected. Canary
and general submissions share that key namespace, so the same key cannot be
reused to switch modes. The objective and project name must be trimmed,
already NFC, within the advertised bounds, and free of NUL/control characters
or secret material. Objective text is stored as task data and is never executed
as a shell, SQL, path, permission, configuration, provider instruction, or
approval.

A successful general submission returns `lattice.task.status.v3` with
`status=SUBMITTED`, `task_state=DRAFT`, the durable `task_ref`, Ledger-head
digest, exact objective, Control project ID/display name, formal project
snapshot ID, and nullable result/failure fields. Read it later—even in a new
server process—with:

```json
{"task_ref":"<64-character-lowercase-sha256>"}
```

General task creation records a formal `GENERAL_TASK_INTAKE` pre-specification
identity with one `GENERAL_TASK_INTAKE_V1` create event. It contains no Task
Spec, accounting currency, autonomy classification/receipt, transition,
result, or effect. It does not create a specification or tickets and does not
start Codex/model execution, workspace/Git mutation, payment, external action,
merge, deployment, or release. Codex should return `task_ref` and `task_state`
to the user, then wait for a separate authorized operation before expanding or
executing the objective.

## Child environment

`-EnvironmentFile` accepts a UTF-8 JSON object whose keys are uppercase environment-variable names. These values are applied only to the spawned child process; the current PowerShell environment is not modified. Sensitive inherited variables are removed before the supplied child environment is added, and supplied sensitive values are redacted from saved output.

`environment.fresh.example.json` contains placeholders for a fresh authorized run. `environment.resume-discovery.placeholder.json` is a non-secret fail-closed discovery/resume probe. Never replace placeholders with real credentials in a committed file.

## Usage

```powershell
$client = Join-Path $PSScriptRoot 'Invoke-LatticeMcp.ps1'
$binary = '<absolute-path-to-latticed.exe>'

& $client `
  -BinaryPath $binary `
  -Action Discovery `
  -EnvironmentFile (Join-Path $PSScriptRoot 'environment.resume-discovery.placeholder.json')

$submit = & $client `
  -BinaryPath $binary `
  -Action TaskSubmit `
  -ClientRequestId ('direct-stdio-' + [Guid]::NewGuid().ToString('N').Substring(0, 16)) `
  -Objective '完成角色系統' `
  -ProjectName 'AI 劇本' `
  -EnvironmentFile '<runtime-environment.json>' |
    ConvertFrom-Json

$taskRef = [string]$submit.call.result.structuredContent.task_ref

# This may be run later from a different PowerShell or Codex session.
& $client `
  -BinaryPath $binary `
  -Action TaskStatus `
  -TaskRef $taskRef `
  -EnvironmentFile '<runtime-environment.json>'
```

For the retained canary path, omit `-Objective`, `-ProjectId`, and
`-ProjectName`. The wrapper then sends exactly
`{"client_request_id":"...","intent":"CONTROLLED_CODEX_CANARY"}`.
General input is validated locally for a secret-free bounded ASCII
`client_request_id`, bounded UTF-8/NFC text, surrounding whitespace,
Unicode-scalar length (including valid surrogate pairs), control characters,
recognized secret shapes, a canonical secret-free lowercase Control project
ID, and mutually exclusive selectors before any child process starts. Unpaired
UTF-16 surrogates are rejected.

When querying a canary task, explicitly pass the same `-ClientRequestId` used
for submission together with `-TaskRef`. General-task status remains the
default task-ref-only form shown above.

By default, redacted `stdout.jsonl`, `stderr.log`, and `summary.json` files are written below `results/session-...`. A tool response with `isError=true` is reported as `TOOL_ERROR`; it is not a successful acceptance result.

## Fresh acceptance coordinator

`Invoke-LatticeFreshAcceptance.ps1` requires an absolute `TaskContractFile`, resolves it through the versioned closed registry, verifies the contract-file hash from the normalized projection, and only then records a secret-free request intent. It submits exactly once, and only after a completed submit starts one wholly independent status session. It requires exact equality of `task_ref`, `status`, `task_state`, `result_digest`, and `ledger_head_digest`. It never retries, polls, replays, cleans up, or reads the environment file contents.

The minimal UTF-8-without-BOM contract is:

```json
{"schema":"lattice.task-contract.v1","task_type":"controlled_codex_canary","parameters":{}}
```

The adjacent source-owned `Get-LatticeTaskContractRegistry.ps1` emits the
normalized closed registry consumed by this Phase 2 acceptance coordinator. It
currently contains exactly one mapping: `controlled_codex_canary` to the fixed
`CONTROLLED_CODEX_CANARY` intent for `lattice_task_submit`. This coordinator is
deliberately not the Phase 3 general-task entrance. Its typed boundary permits
no general objective, shell, command, SQL, path, file-write, environment,
credential, or payload, and its generic low-level action is unreachable from
the coordinator.

## Offline typed-contract conformance

`task-contract.conformance.v1.json` is the UTF-8-without-BOM, version 1 source of positive and exact-rejection vectors for the closed typed-contract boundary. `Invoke-LatticeTaskContractConformance.ps1` invokes and hashes the adjacent normalized registry, materializes every vector as a fresh temporary fixture, invokes the resolver exactly once per case, and emits one secret-free summary. Its automatic coverage gate requires exactly one accepted positive vector for every registry type and rejects duplicate, unknown, or uncovered positive types.

```powershell
$conformance = Join-Path $PSScriptRoot 'Invoke-LatticeTaskContractConformance.ps1'

& $conformance `
  -ResolverPath (Join-Path $PSScriptRoot 'Resolve-LatticeTaskContract.ps1') `
  -ConformanceFile (Join-Path $PSScriptRoot 'task-contract.conformance.v1.json')
```

Every future source-owned acceptance contract requires an explicit registry
entry, closed resolver parameter handling, positive and negative conformance
vectors, and actual product/server support. The current file-based coordinator
registry remains exactly one canary type, `controlled_codex_canary`, mapped
only to `CONTROLLED_CODEX_CANARY`; this does not narrow the canonical Phase 3
MCP schema documented above. Neither coordinator registry, manifest, nor
resolver permits a general objective, shell, command, SQL, path, file-write,
environment, credential, or payload.

```powershell
$coordinator = Join-Path $PSScriptRoot 'Invoke-LatticeFreshAcceptance.ps1'

& $coordinator `
  -BinaryPath 'C:\absolute\path\to\latticed.exe' `
  -EnvironmentFile 'C:\absolute\path\to\runtime-environment.json' `
  -TaskContractFile 'C:\absolute\path\to\task-contract.json' `
  -OutputRoot 'C:\absolute\fresh-worker-output'
```

The output root must not already exist. Initialization, submit, and status timeouts default to 90, 900, and 180 seconds respectively. Override them with `-InitializeTimeoutSeconds`, `-SubmitTimeoutSeconds`, and `-StatusTimeoutSeconds`; retry count remains zero.

Run the deterministic offline fake-wrapper test without starting LATTICE, MCP, PostgreSQL, or the live wrapper process:

```powershell
& (Join-Path $PSScriptRoot 'Test-Invoke-LatticeMcp.ps1')
& (Join-Path $PSScriptRoot 'Test-LatticeTaskContract.ps1')
& (Join-Path $PSScriptRoot 'Test-LatticeFreshAcceptance.ps1')
```

`Test-Invoke-LatticeMcp.ps1` launches only a local PowerShell JSON-lines fake.
It verifies fresh exact-seven discovery, selector-free/name/ID general submit,
canary compatibility, general task-ref-only status, legacy canary status with
an explicitly bound client request ID, Unicode-scalar boundaries, fail-closed
selector and input validation, secret-free summaries, and catalog mismatch
handling. It makes no live service, PostgreSQL, model, execution, or
external-action call.

## Failure catalog

`Get-LatticeMcpFailureCatalog.ps1` reads the fixed adjacent production, test,
and README evidence sets plus the parent `WINDOW_LEDGER.jsonl`. It emits one
secret-free, deterministic `lattice.mcp-failure-catalog.v1` JSON document. The
catalog includes structured JSON failure fields and exact failure codes found
inside legacy text receipts; it never emits receipt text, environment values,
credentials, or absolute paths.

Each failure code receives the strongest current evidence classification:

- `regression_tested`: an exact code is named by an offline regression test.
- `implementation_known`: an exact code exists in current production tooling,
  but no test names it.
- `documented_only`: only the current README names it.
- `recorded_only`: the ledger records it, but current code, tests, and README do
  not name it.

The classification is evidence inventory, not proof that every failure is
resolved. Add or update a focused test when promoting a code to
`regression_tested`. The catalog test is intentionally excluded from test evidence,
so validating the inventory cannot circularly promote an unrelated failure code.

```powershell
& (Join-Path $PSScriptRoot 'Get-LatticeMcpFailureCatalog.ps1')
& (Join-Path $PSScriptRoot 'Test-LatticeMcpFailureCatalog.ps1')
```
