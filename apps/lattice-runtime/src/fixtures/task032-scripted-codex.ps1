[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ExpectedSelfSha256,
    [Parameter(Mandatory = $true)][ValidateSet('Schema', 'Server')][string]$Mode,
    [string]$SchemaRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$selfHash = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($ExpectedSelfSha256 -notmatch '^[0-9a-f]{64}$' -or $selfHash -ne $ExpectedSelfSha256) { exit 90 }
if ($env:LATTICE_DELIVERY_CODEX_MODE -ne 'SCRIPTED_ACCEPTANCE') { exit 91 }

if ($Mode -eq 'Schema') {
    if ([string]::IsNullOrWhiteSpace($SchemaRoot)) { exit 12 }
    $schemaRoot = [System.IO.Path]::GetFullPath($SchemaRoot)
    if (Test-Path -LiteralPath $schemaRoot) { exit 20 }
    New-Item -ItemType Directory -Path $schemaRoot -Force:$false | Out-Null
    [System.IO.File]::WriteAllText(
        (Join-Path $schemaRoot 'lattice-scripted-app-server.json'),
        '{"title":"LATTICE scripted app-server","type":"object"}',
        [System.Text.UTF8Encoding]::new($false)
    )
    exit 0
}

if ($Mode -ne 'Server' -or -not [string]::IsNullOrEmpty($SchemaRoot)) { exit 11 }
if ([string]::IsNullOrWhiteSpace($env:CODEX_HOME) -or [string]::IsNullOrWhiteSpace($env:LATTICE_DELIVERY_CODEX_HOME)) { exit 30 }
$actualHome = [System.IO.Path]::GetFullPath($env:CODEX_HOME)
$expectedHome = [System.IO.Path]::GetFullPath($env:LATTICE_DELIVERY_CODEX_HOME)
if (-not [string]::Equals($actualHome, $expectedHome, [System.StringComparison]::OrdinalIgnoreCase)) { exit 31 }
$markerPath = Join-Path $actualHome '.lattice-codex-home-v1'
if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) { exit 32 }
$marker = [System.IO.File]::ReadAllBytes($markerPath)
$expectedMarker = [System.Text.Encoding]::UTF8.GetBytes("lattice.codex-home.v1`n")
if ([Convert]::ToBase64String($marker) -ne [Convert]::ToBase64String($expectedMarker)) { exit 33 }

$deadlineRegression = $env:LATTICE_DELIVERY_DEADLINE_REGRESSION -eq '1'
if ($deadlineRegression) {
    if ($env:LATTICE_DELIVERY_SCRIPTED_DELAY_MILLISECONDS -ne '20000') { exit 34 }
    if ([string]::IsNullOrWhiteSpace($env:LATTICE_DELIVERY_FIXTURE_ROOT) -or [string]::IsNullOrWhiteSpace($env:LATTICE_DELIVERY_SCRIPTED_INVOCATION_LOG)) { exit 35 }
    $fixtureRoot = [System.IO.Path]::GetFullPath($env:LATTICE_DELIVERY_FIXTURE_ROOT)
    $invocationLog = [System.IO.Path]::GetFullPath($env:LATTICE_DELIVERY_SCRIPTED_INVOCATION_LOG)
    $expectedInvocationLog = [System.IO.Path]::GetFullPath((Join-Path $fixtureRoot 'scripted-server-invocations.log'))
    if (-not [string]::Equals($invocationLog, $expectedInvocationLog, [System.StringComparison]::OrdinalIgnoreCase)) { exit 36 }
    if (Test-Path -LiteralPath $invocationLog) {
        $logItem = Get-Item -LiteralPath $invocationLog -Force
        if ($logItem.PSIsContainer -or ($logItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) { exit 37 }
    }
    [System.IO.File]::AppendAllText($invocationLog, "server`n", [System.Text.Encoding]::ASCII)
}
elseif (
    -not [string]::IsNullOrWhiteSpace($env:LATTICE_DELIVERY_SCRIPTED_DELAY_MILLISECONDS) -or
    -not [string]::IsNullOrWhiteSpace($env:LATTICE_DELIVERY_SCRIPTED_INVOCATION_LOG)
) { exit 38 }

function Read-Request {
    $line = [Console]::In.ReadLine()
    if ($null -eq $line) { exit 40 }
    try { return $line | ConvertFrom-Json -ErrorAction Stop } catch { exit 41 }
}

$initialize = Read-Request
if ([string]$initialize.method -ne 'initialize' -or [int]$initialize.id -ne 0) { exit 42 }
[Console]::Out.WriteLine((([ordered]@{
    id = 0
    result = [ordered]@{
        userAgent = 'codex_cli_rs/0.144.6'
        platformFamily = 'windows'
        platformOs = 'windows'
        codexHome = $actualHome
    }
}) | ConvertTo-Json -Depth 8 -Compress))

$initialized = Read-Request
if ([string]$initialized.method -ne 'initialized') { exit 43 }
$thread = Read-Request
$currentDirectory = [System.IO.Path]::GetFullPath((Get-Location).Path)
$threadDirectory = [System.IO.Path]::GetFullPath([string]$thread.params.cwd)
if (
    [string]$thread.method -ne 'thread/start' -or
    [int]$thread.id -ne 1 -or
    [string]$thread.params.approvalPolicy -ne 'never' -or
    [string]$thread.params.sandbox -ne 'workspace-write' -or
    -not [string]::Equals($threadDirectory, $currentDirectory, [System.StringComparison]::OrdinalIgnoreCase)
) { exit 44 }
[Console]::Out.WriteLine('{"id":1,"result":{"thread":{"id":"thread-task032-scripted"}}}')

$turn = Read-Request
$turnDirectory = [System.IO.Path]::GetFullPath([string]$turn.params.cwd)
$inputs = @($turn.params.input)
$roots = @($turn.params.sandboxPolicy.writableRoots)
if (
    [string]$turn.method -ne 'turn/start' -or
    [int]$turn.id -ne 2 -or
    [string]$turn.params.threadId -ne 'thread-task032-scripted' -or
    [string]$turn.params.approvalPolicy -ne 'never' -or
    [string]$turn.params.sandboxPolicy.type -ne 'workspaceWrite' -or
    [bool]$turn.params.sandboxPolicy.networkAccess -ne $false -or
    $inputs.Count -ne 1 -or
    [string]$inputs[0].type -ne 'text' -or
    [string]$inputs[0].text -ne 'Create answer.txt in the current repository with exactly the bytes LATTICE_DELIVERY_OK followed by one newline. Use one standalone apply_patch operation in an exec call that performs no verification or other tool work. Confirm that call has completed, then use a separate verification call to read and validate the exact bytes. Do not combine file creation and verification in the same exec call. If any exec result says Script running with cell ID, call functions.wait with that exact cell_id until Script completed is received, and require exit code 0 before reporting success. Never terminate a yielded cell or claim completion from a running marker. Do not modify any other path. Do not stage or commit files and do not run Git commands.' -or
    $roots.Count -ne 1 -or
    -not [string]::Equals([System.IO.Path]::GetFullPath([string]$roots[0]), $currentDirectory, [System.StringComparison]::OrdinalIgnoreCase) -or
    -not [string]::Equals($turnDirectory, $currentDirectory, [System.StringComparison]::OrdinalIgnoreCase)
) { exit 45 }

if ($deadlineRegression) { Start-Sleep -Milliseconds 20000 }

[System.IO.File]::WriteAllBytes(
    (Join-Path $currentDirectory 'answer.txt'),
    [System.Text.Encoding]::ASCII.GetBytes("LATTICE_DELIVERY_OK`n")
)
[Console]::Out.WriteLine('{"id":2,"result":{"turn":{"id":"turn-task032-scripted"}}}')
[Console]::Out.WriteLine('{"method":"item/completed","params":{"threadId":"thread-task032-scripted","turnId":"turn-task032-scripted","item":{"id":"tool-apply-patch","type":"dynamicToolCall","tool":"exec","arguments":{"command":"apply_patch fixture"},"status":"completed","success":true,"contentItems":[{"type":"inputText","text":"Script completed\nExit code: 0"}]},"completedAtMs":1}}')
[Console]::Out.WriteLine('{"method":"item/completed","params":{"threadId":"thread-task032-scripted","turnId":"turn-task032-scripted","item":{"id":"tool-verify","type":"dynamicToolCall","tool":"exec","arguments":{"command":"verify fixture"},"status":"completed","success":true,"contentItems":[{"type":"inputText","text":"Script completed\nExit code: 0"}]},"completedAtMs":2}}')
[Console]::Out.WriteLine('{"method":"turn/completed","params":{"threadId":"thread-task032-scripted","turn":{"id":"turn-task032-scripted","items":[{"id":"agent-final","type":"agentMessage","text":"Delivery complete."}],"itemsView":"summary","status":"completed","error":null}}}')
Start-Sleep -Seconds 60
