# LATTICE MCP Kit: direct stdio client

This reusable PowerShell client launches a specified LATTICE MCP executable over stdio, performs MCP initialization, requires the current exact four-tool catalog, and writes one redacted result directory per invocation.

## Current tool catalog

- `lattice_delivery_run`
- `lattice_delivery_status`
- `lattice_task_submit`
- `lattice_task_status`

Discovery fails closed with `TOOL_SET_MISMATCH` if the executable advertises anything other than this exact set.

## Actions

- `Discovery` initializes MCP and verifies the exact tool catalog without calling a tool.
- `TaskSubmit` calls `lattice_task_submit` with the fixed `CONTROLLED_CODEX_CANARY` intent and a bounded `client_request_id`.
- `TaskStatus` calls `lattice_task_status` with a lowercase 64-character `task_ref`.
- `Call` is the generic low-level action and requires an explicit tool name and JSON arguments.

Each invocation starts a fresh child process. For cross-session use, retain the `task_ref` returned by `TaskSubmit`, then pass it to a later `TaskStatus` invocation. The client does not treat its own process or output directory as durable task truth.

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

By default, redacted `stdout.jsonl`, `stderr.log`, and `summary.json` files are written below `results/session-...`. A tool response with `isError=true` is reported as `TOOL_ERROR`; it is not a successful acceptance result.

## Fresh acceptance coordinator

`Invoke-LatticeFreshAcceptance.ps1` requires an absolute `TaskContractFile`, resolves it through the versioned closed registry, verifies the contract-file hash from the normalized projection, and only then records a secret-free request intent. It submits exactly once, and only after a completed submit starts one wholly independent status session. It requires exact equality of `task_ref`, `status`, `task_state`, `result_digest`, and `ledger_head_digest`. It never retries, polls, replays, cleans up, or reads the environment file contents.

The minimal UTF-8-without-BOM contract is:

```json
{"schema":"lattice.task-contract.v1","task_type":"controlled_codex_canary","parameters":{}}
```

The current closed registry contains exactly one type: `controlled_codex_canary`, mapped to the fixed `CONTROLLED_CODEX_CANARY` intent for `lattice_task_submit`. Future types require an explicit source mapping plus focused schema and coordinator tests. Contract fields can never dispatch arbitrary shell, command, SQL, path, file-write, environment, credential, or free-form task payload data. The generic low-level client action is not reachable from this typed contract coordinator.

## Offline typed-contract conformance

`task-contract.conformance.v1.json` is the UTF-8-without-BOM, version 1 source of positive and exact-rejection vectors for the closed typed-contract boundary. `Invoke-LatticeTaskContractConformance.ps1` materializes every vector as a fresh temporary fixture, invokes the resolver exactly once per case, and emits one secret-free summary.

```powershell
$conformance = Join-Path $PSScriptRoot 'Invoke-LatticeTaskContractConformance.ps1'

& $conformance `
  -ResolverPath (Join-Path $PSScriptRoot 'Resolve-LatticeTaskContract.ps1') `
  -ConformanceFile (Join-Path $PSScriptRoot 'task-contract.conformance.v1.json')
```

Every future source-owned typed contract requires both an explicit resolver mapping and new positive and negative conformance vectors. The current registry remains exactly one canary type, `controlled_codex_canary`, mapped only to `CONTROLLED_CODEX_CANARY`. Neither the manifest nor the resolver permits arbitrary shell, command, SQL, path, file-write, environment, credential, or free-form task payloads.

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
& (Join-Path $PSScriptRoot 'Test-LatticeTaskContract.ps1')
& (Join-Path $PSScriptRoot 'Test-LatticeFreshAcceptance.ps1')
```
