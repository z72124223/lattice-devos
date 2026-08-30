[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ExpectedSelfSha256,
    [Parameter(Mandatory = $true)][ValidateSet('Schema', 'Server')][string]$Mode,
    [string]$SchemaRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module Microsoft.PowerShell.Utility -ErrorAction Stop
Import-Module Microsoft.PowerShell.Management -ErrorAction Stop

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

function Write-JsonLine {
    param([Parameter(Mandatory = $true)]$Value)

    [Console]::Out.WriteLine(($Value | ConvertTo-Json -Depth 20 -Compress))
    [Console]::Out.Flush()
}

function Read-ManagedRequest {
    $line = [Console]::In.ReadLine()
    if ($null -eq $line) { return $null }
    if ([Text.Encoding]::UTF8.GetByteCount($line) -gt 1048576) { exit 61 }
    try { return $line | ConvertFrom-Json -ErrorAction Stop } catch { exit 62 }
}

function Invoke-ManagedActiveRestartServer {
    param([Parameter(Mandatory = $true)][string]$FixtureRoot)

    $statePath = Join-Path $FixtureRoot 'managed-active-state.json'
    $eventPath = Join-Path $FixtureRoot 'managed-active-events.jsonl'
    $generationPath = Join-Path $FixtureRoot 'managed-server-generations.jsonl'
    foreach ($path in @($statePath, $eventPath, $generationPath)) {
        if (Test-Path -LiteralPath $path) {
            $item = Get-Item -LiteralPath $path -Force
            if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                exit 63
            }
        }
    }
    $fixtureId = [IO.Path]::GetFileName($FixtureRoot)
    if ($fixtureId -cnotmatch '^[0-9a-f]{32}$') { exit 64 }
    $threadId = 'thread-phase4-restart-' + $fixtureId
    $turnId = 'turn-phase4-restart-' + $fixtureId
    $utf8 = [Text.UTF8Encoding]::new($false)
    $serverProcess = Get-Process -Id $PID -ErrorAction Stop
    $serverPid = [int]$serverProcess.Id
    $serverStartUtcTicks = [long]$serverProcess.StartTime.ToUniversalTime().Ticks
    if ($serverPid -le 0 -or $serverStartUtcTicks -le 0) { exit 74 }
    [IO.File]::AppendAllText(
        $generationPath,
        (([ordered]@{
            schema = 'lattice.phase4-scripted-server-generation.v1'
            server_pid = $serverPid
            server_start_utc_ticks = $serverStartUtcTicks
        } | ConvertTo-Json -Compress) + "`n"),
        $utf8
    )

    function Write-ManagedEvent {
        param([Parameter(Mandatory = $true)][string]$Kind)
        [IO.File]::AppendAllText(
            $eventPath,
            (([ordered]@{
                schema = 'lattice.phase4-scripted-server-event.v1'
                event = $Kind
                server_pid = $serverPid
                server_start_utc_ticks = $serverStartUtcTicks
            } | ConvertTo-Json -Compress) + "`n"),
            $utf8
        )
    }

    function Read-ManagedState {
        if (-not (Test-Path -LiteralPath $statePath -PathType Leaf)) { return $null }
        try { $state = [IO.File]::ReadAllText($statePath, $utf8) | ConvertFrom-Json -ErrorAction Stop }
        catch { exit 65 }
        if (
            [string]$state.thread_id -cne $threadId -or
            $state.turn_started -isnot [bool] -or
            ([bool]$state.turn_started -and [string]$state.turn_id -cne $turnId) -or
            (-not [bool]$state.turn_started -and $null -ne $state.turn_id) -or
            ([string]$state.cwd -notmatch '^[A-Za-z]:[\\/]' -and
                [string]$state.cwd -notmatch '^\\\\\?\\[A-Za-z]:\\') -or
            [long]$state.created_at -le 0 -or
            [string]$state.status -notin @('pending', 'inProgress', 'interrupted') -or
            (-not [bool]$state.turn_started -and [string]$state.status -cne 'pending')
        ) { exit 66 }
        return $state
    }

    function Write-ManagedState {
        param(
            [Parameter(Mandatory = $true)][string]$Cwd,
            [Parameter(Mandatory = $true)][long]$CreatedAt,
            [Parameter(Mandatory = $true)][bool]$TurnStarted,
            [Parameter(Mandatory = $true)][ValidateSet('pending', 'inProgress', 'interrupted')][string]$Status
        )
        $state = [ordered]@{
            thread_id = $threadId
            turn_id = $(if ($TurnStarted) { $turnId } else { $null })
            turn_started = $TurnStarted
            cwd = [IO.Path]::GetFullPath($Cwd)
            created_at = $CreatedAt
            status = $Status
        }
        [IO.File]::WriteAllText(
            $statePath,
            (($state | ConvertTo-Json -Depth 8 -Compress) + "`n"),
            $utf8
        )
    }

    function Get-ManagedThread {
        param([Parameter(Mandatory = $true)]$State)
        [object[]]$turns = if ([bool]$State.turn_started) {
            [ordered]@{
                id = [string]$State.turn_id
                status = [string]$State.status
                items = @()
            }
        }
        else { @() }
        return [ordered]@{
            id = [string]$State.thread_id
            cwd = [string]$State.cwd
            createdAt = [long]$State.created_at
            turns = $turns
        }
    }

    $initialize = Read-ManagedRequest
    if ($null -eq $initialize -or [string]$initialize.method -cne 'initialize' -or
        $null -eq $initialize.PSObject.Properties['id']) { exit 67 }
    Write-JsonLine ([ordered]@{
        id = $initialize.id
        result = [ordered]@{
            userAgent = 'codex_cli_rs/0.144.6'
            platformFamily = 'windows'
            platformOs = 'windows'
            codexHome = $actualHome
        }
    })
    $initialized = Read-ManagedRequest
    if ($null -eq $initialized -or [string]$initialized.method -cne 'initialized') { exit 68 }

    while ($true) {
        $request = Read-ManagedRequest
        if ($null -eq $request) {
            Write-ManagedEvent -Kind 'SERVER_EXIT'
            return
        }
        $method = [string]$request.method
        if ($method -ceq 'account/read') {
            if ($null -eq $request.params -or [bool]$request.params.refreshToken) { exit 75 }
            Write-ManagedEvent -Kind 'ACCOUNT_READ'
            Write-JsonLine ([ordered]@{
                id = $request.id
                result = [ordered]@{
                    account = [ordered]@{
                        type = 'chatgpt'
                        email = 'must-not-escape@example.invalid'
                    }
                    requiresOpenaiAuth = $true
                }
            })
            continue
        }
        if ($method -ceq 'model/list') {
            Write-ManagedEvent -Kind 'MODEL_LIST'
            Write-JsonLine ([ordered]@{
                id = $request.id
                result = [ordered]@{
                    data = @([ordered]@{ id = 'gpt-5.6-terra'; model = 'gpt-5.6-terra' })
                    nextCursor = $null
                }
            })
            continue
        }
        if ($method -ceq 'thread/list') {
            Write-ManagedEvent -Kind 'THREAD_LIST'
            $state = Read-ManagedState
            if ($null -eq $state) {
                # Windows PowerShell pipeline assignment turns an empty @()
                # expression into $null, which serializes as an object instead
                # of the protocol's required empty page array.
                $threads = [object[]]@()
            }
            else {
                $threads = [object[]]@(Get-ManagedThread -State $state)
            }
            Write-JsonLine ([ordered]@{
                id = $request.id
                result = [ordered]@{ data = $threads; nextCursor = $null }
            })
            continue
        }
        if ($method -ceq 'thread/start') {
            if ($null -ne (Read-ManagedState) -or
                [string]$request.params.approvalPolicy -cne 'never' -or
                [string]$request.params.sandbox -cne 'workspace-write' -or
                [string]$request.params.model -cne 'gpt-5.6-terra') { exit 69 }
            $cwd = [IO.Path]::GetFullPath([string]$request.params.cwd)
            $createdAt = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
            Write-ManagedState -Cwd $cwd -CreatedAt $createdAt -TurnStarted $false -Status 'pending'
            Write-ManagedEvent -Kind 'THREAD_START'
            Write-JsonLine ([ordered]@{
                id = $request.id
                result = [ordered]@{ thread = [ordered]@{ id = $threadId; cwd = $cwd; createdAt = $createdAt } }
            })
            Write-JsonLine ([ordered]@{
                method = 'thread/started'
                params = [ordered]@{ thread = [ordered]@{ id = $threadId; cwd = $cwd; createdAt = $createdAt } }
            })
            continue
        }
        if ($method -ceq 'turn/start') {
            $state = Read-ManagedState
            if ($null -eq $state -or [string]$request.params.threadId -cne $threadId -or
                [bool]$state.turn_started -or
                @($request.params.input).Count -ne 1 -or
                [string]$request.params.input[0].type -cne 'text' -or
                [string]$request.params.input[0].text -cnotmatch '^\[LATTICE_MANAGED_ATTEMPT ') { exit 70 }
            Write-ManagedState -Cwd ([string]$state.cwd) -CreatedAt ([long]$state.created_at) `
                -TurnStarted $true -Status 'inProgress'
            Write-ManagedEvent -Kind 'TURN_START'
            Write-JsonLine ([ordered]@{
                id = $request.id
                result = [ordered]@{ turn = [ordered]@{ id = $turnId; status = 'inProgress' } }
            })
            Write-JsonLine ([ordered]@{
                method = 'turn/started'
                params = [ordered]@{
                    threadId = $threadId
                    turn = [ordered]@{ id = $turnId; status = 'inProgress' }
                }
            })
            continue
        }
        if ($method -in @('thread/resume', 'thread/read')) {
            $state = Read-ManagedState
            if ($null -eq $state -or [string]$request.params.threadId -cne $threadId) { exit 71 }
            Write-ManagedEvent -Kind $method.ToUpperInvariant().Replace('/', '_')
            Write-JsonLine ([ordered]@{
                id = $request.id
                result = [ordered]@{ thread = Get-ManagedThread -State $state }
            })
            continue
        }
        if ($method -ceq 'turn/interrupt') {
            $state = Read-ManagedState
            if ($null -eq $state -or [string]$request.params.threadId -cne $threadId -or
                [string]$request.params.turnId -cne $turnId -or
                -not [bool]$state.turn_started -or [string]$state.status -cne 'inProgress') { exit 72 }
            Write-ManagedState -Cwd ([string]$state.cwd) -CreatedAt ([long]$state.created_at) `
                -TurnStarted $true -Status 'interrupted'
            Write-ManagedEvent -Kind 'TURN_INTERRUPT'
            Write-JsonLine ([ordered]@{ id = $request.id; result = [ordered]@{} })
            Write-JsonLine ([ordered]@{
                method = 'turn/completed'
                params = [ordered]@{
                    threadId = $threadId
                    turn = [ordered]@{ id = $turnId; status = 'interrupted'; items = @(); error = $null }
                }
            })
            Write-ManagedEvent -Kind 'TURN_TERMINAL_ACK'
            continue
        }
        exit 73
    }
}

$managedRestartMarkerPath = Join-Path $PSScriptRoot '.lattice-managed-active-restart-v1'
if (Test-Path -LiteralPath $managedRestartMarkerPath -PathType Leaf) {
    $managedMarkerItem = Get-Item -LiteralPath $managedRestartMarkerPath -Force
    $expectedManagedMarker = [Text.Encoding]::ASCII.GetBytes("lattice.phase4.scripted-active-restart.v1`n")
    $actualManagedMarker = [IO.File]::ReadAllBytes($managedRestartMarkerPath)
    if (($managedMarkerItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        [Convert]::ToBase64String($actualManagedMarker) -cne
        [Convert]::ToBase64String($expectedManagedMarker)) { exit 60 }
    Invoke-ManagedActiveRestartServer -FixtureRoot ([IO.Path]::GetFullPath($PSScriptRoot))
    exit 0
}

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
$promptDigest = [System.Security.Cryptography.SHA256]::Create()
try {
    $promptSha256 = -join (
        $promptDigest.ComputeHash([System.Text.Encoding]::UTF8.GetBytes([string]$inputs[0].text)) |
            ForEach-Object { $_.ToString('x2') }
    )
}
finally {
    $promptDigest.Dispose()
}
$expectedPromptSha256 = 'a96dbe826a690eaa0d89f3b42000a1f4194b762fd5da6ebab00096a3f9ff8461'
if (
    [string]$turn.method -ne 'turn/start' -or
    [int]$turn.id -ne 2 -or
    [string]$turn.params.threadId -ne 'thread-task032-scripted' -or
    [string]$turn.params.approvalPolicy -ne 'never' -or
    [string]$turn.params.sandboxPolicy.type -ne 'workspaceWrite' -or
    [bool]$turn.params.sandboxPolicy.networkAccess -ne $false -or
    $inputs.Count -ne 1 -or
    [string]$inputs[0].type -ne 'text' -or
    $promptSha256 -ne $expectedPromptSha256 -or
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
[Console]::Out.WriteLine('{"method":"turn/started","params":{"threadId":"thread-task032-scripted","turn":{"id":"turn-task032-scripted","status":"inProgress"}}}')
[Console]::Out.WriteLine('{"method":"item/completed","params":{"threadId":"thread-task032-scripted","turnId":"turn-task032-scripted","item":{"id":"tool-shell-write","type":"dynamicToolCall","tool":"exec","arguments":{"command":"code-mode nested tools.shell_command write fixture"},"status":"completed","success":true,"contentItems":[{"type":"inputText","text":"Script completed\nExit code: 0\nOutput:\nExit code: 0"}]},"completedAtMs":1}}')
[Console]::Out.WriteLine('{"method":"item/completed","params":{"threadId":"thread-task032-scripted","turnId":"turn-task032-scripted","item":{"id":"tool-shell-verify","type":"dynamicToolCall","tool":"exec","arguments":{"command":"code-mode nested tools.shell_command verify fixture"},"status":"completed","success":true,"contentItems":[{"type":"inputText","text":"Script completed\nExit code: 0\nOutput:\nExit code: 0"}]},"completedAtMs":2}}')
[Console]::Out.WriteLine('{"method":"turn/completed","params":{"threadId":"thread-task032-scripted","turn":{"id":"turn-task032-scripted","items":[{"id":"agent-final","type":"agentMessage","text":"Delivery complete."}],"itemsView":"summary","status":"completed","error":null}}}')
Start-Sleep -Seconds 60
