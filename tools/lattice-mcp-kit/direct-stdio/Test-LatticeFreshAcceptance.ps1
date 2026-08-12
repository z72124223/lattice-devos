[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false)
$coordinator = Join-Path $PSScriptRoot 'Invoke-LatticeFreshAcceptance.ps1'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('lattice-fresh-acceptance-test-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $testRoot
$fakeWrapper = Join-Path $testRoot 'Fake-LatticeMcp.ps1'
$fakeBinary = Join-Path $testRoot 'fake-latticed.exe'
$environmentFile = Join-Path $testRoot 'environment.json'
$taskContractFile = Join-Path $testRoot 'task-contract.json'

function Assert-True {
    param([Parameter(Mandatory = $true)][bool]$Condition, [Parameter(Mandatory = $true)][string]$Message)
    if (-not $Condition) { throw ('ASSERTION_FAILED|' + $Message) }
}

function Read-JsonFile {
    param([Parameter(Mandatory = $true)][string]$Path)
    return [Text.UTF8Encoding]::new($false, $true).GetString([IO.File]::ReadAllBytes($Path)) | ConvertFrom-Json -ErrorAction Stop
}

function Read-Invocations {
    param([Parameter(Mandatory = $true)][string]$Root)
    return @((Get-Content -LiteralPath (Join-Path $Root 'fake-wrapper-invocations.jsonl') -Encoding UTF8) | ForEach-Object {
        $_ | ConvertFrom-Json -ErrorAction Stop
    })
}

function Invoke-TestCase {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][bool]$ShouldSucceed
    )

    $outputRoot = Join-Path $testRoot $Name
    $threw = $false
    try {
        $resultText = & $coordinator `
            -BinaryPath $fakeBinary `
            -EnvironmentFile $environmentFile `
            -TaskContractFile $taskContractFile `
            -ClientRequestId ('test-' + $Name) `
            -WrapperPath $fakeWrapper `
            -OutputRoot $outputRoot
        if (-not $ShouldSucceed) { throw 'EXPECTED_COORDINATOR_FAILURE' }
        $result = $resultText | ConvertFrom-Json -ErrorAction Stop
        Assert-True -Condition ([bool]$result.acceptance) -Message ($Name + ' acceptance')
    }
    catch {
        $threw = $true
        if ($ShouldSucceed) { throw }
    }
    Assert-True -Condition ($threw -eq (-not $ShouldSucceed)) -Message ($Name + ' exit behavior')
    Assert-True -Condition (Test-Path -LiteralPath (Join-Path $outputRoot 'preserve.marker') -PathType Leaf) -Message ($Name + ' no cleanup marker')
    return [pscustomobject]@{
        Root = $outputRoot
        Summary = Read-JsonFile -Path (Join-Path $outputRoot 'coordinator-summary.json')
        Invocations = @(Read-Invocations -Root $outputRoot)
    }
}

function Assert-ContractRejectedBeforeWrapper {
    param([Parameter(Mandatory = $true)][string]$Field)

    $contractPath = Join-Path $testRoot ('rejected-' + $Field + '.json')
    $outputRoot = Join-Path $testRoot ('rejected-' + $Field)
    $contractJson = '{"schema":"lattice.task-contract.v1","task_type":"controlled_codex_canary","parameters":{},"' + $Field + '":"blocked"}'
    [IO.File]::WriteAllText($contractPath, $contractJson, $utf8)
    $threw = $false
    try {
        $null = & $coordinator `
            -BinaryPath $fakeBinary `
            -EnvironmentFile $environmentFile `
            -TaskContractFile $contractPath `
            -ClientRequestId ('test-rejected-' + $Field) `
            -WrapperPath $fakeWrapper `
            -OutputRoot $outputRoot
    }
    catch { $threw = $true }
    Assert-True -Condition $threw -Message ($Field + ' contract rejected')
    Assert-True -Condition (-not (Test-Path -LiteralPath $outputRoot)) -Message ($Field + ' rejected before output creation')
}

$fakeSource = @'
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$BinaryPath,
    [Parameter(Mandatory = $true)][string]$EnvironmentFile,
    [Parameter(Mandatory = $true)][ValidateSet('TaskSubmit', 'TaskStatus')][string]$Action,
    [string]$ClientRequestId,
    [string]$TaskRef,
    [int]$TimeoutSeconds,
    [int]$ToolCallTimeoutSeconds,
    [Parameter(Mandatory = $true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false)
$root = Split-Path -Parent $OutputDirectory
$caseName = Split-Path -Leaf $root
$logPath = Join-Path $root 'fake-wrapper-invocations.jsonl'
$entry = [ordered]@{
    action = $Action
    binary_path = $BinaryPath
    environment_file = $EnvironmentFile
    timeout_seconds = $TimeoutSeconds
    tool_call_timeout_seconds = $ToolCallTimeoutSeconds
}
[IO.File]::AppendAllText($logPath, (($entry | ConvertTo-Json -Compress) + "`n"), $utf8)
[IO.File]::WriteAllText((Join-Path $root 'preserve.marker'), 'preserve', $utf8)
$invocationCount = @(Get-Content -LiteralPath $logPath -Encoding UTF8).Count
$sessionDirectory = Join-Path $OutputDirectory ('session-' + $invocationCount.ToString('00') + '-' + $Action.ToLowerInvariant())
$null = New-Item -ItemType Directory -Path $sessionDirectory
$taskRefValue = ('a' * 64)
$status = 'COMPLETED'
$taskState = 'COMPLETED'
$resultDigest = ('b' * 64)
$ledgerDigest = ('c' * 64)
if ($caseName -eq 'submit-domain-failure' -and $Action -eq 'TaskSubmit') {
    $status = 'FAILED'
    $taskState = 'FAILED'
    $resultDigest = $null
    $ledgerDigest = $null
}
if ($caseName -eq 'status-mismatch' -and $Action -eq 'TaskStatus') {
    $ledgerDigest = ('d' * 64)
}
$structured = [ordered]@{
    task_ref = $taskRefValue
    status = $status
    task_state = $taskState
    result_digest = $resultDigest
    ledger_head_digest = $ledgerDigest
}
$stdoutPath = Join-Path $sessionDirectory 'stdout.jsonl'
$stderrPath = Join-Path $sessionDirectory 'stderr.log'
$summaryPath = Join-Path $sessionDirectory 'summary.json'
$callResponse = [ordered]@{
    jsonrpc = '2.0'
    id = 3
    result = [ordered]@{ isError = $false; structuredContent = $structured }
}
[IO.File]::WriteAllText($stdoutPath, (($callResponse | ConvertTo-Json -Compress -Depth 10) + "`n"), $utf8)
[IO.File]::WriteAllText($stderrPath, '', $utf8)
$summary = [ordered]@{
    schema = 'fake.lattice-direct-stdio-client.v1'
    session_id = ('fake-session-' + $invocationCount)
    action = $Action
    classification = 'CALL_OK'
    success = $true
    process = [ordered]@{ started = $true; cleanup_attempted = $false }
    call = [ordered]@{
        is_error = $false
        result = $callResponse.result
    }
    artifacts = [ordered]@{ stdout = $stdoutPath; stderr = $stderrPath; summary = $summaryPath }
}
$json = $summary | ConvertTo-Json -Depth 20
[IO.File]::WriteAllText($summaryPath, ($json + "`n"), $utf8)
$json
'@

[IO.File]::WriteAllText($fakeWrapper, $fakeSource, $utf8)
[IO.File]::WriteAllText($fakeBinary, 'offline fake; never executed', $utf8)
[IO.File]::WriteAllText($environmentFile, "{}`n", $utf8)
[IO.File]::WriteAllText($taskContractFile, '{"schema":"lattice.task-contract.v1","task_type":"controlled_codex_canary","parameters":{}}', $utf8)

$success = Invoke-TestCase -Name 'success' -ShouldSucceed $true
Assert-True -Condition ($success.Invocations.Count -eq 2) -Message 'success exactly two invocations'
Assert-True -Condition ($success.Invocations[0].action -ceq 'TaskSubmit' -and $success.Invocations[1].action -ceq 'TaskStatus') -Message 'success submit then status'
Assert-True -Condition ($success.Invocations[0].timeout_seconds -eq 90 -and $success.Invocations[0].tool_call_timeout_seconds -eq 900) -Message 'submit timeout forwarding'
Assert-True -Condition ($success.Invocations[1].timeout_seconds -eq 90 -and $success.Invocations[1].tool_call_timeout_seconds -eq 180) -Message 'status timeout forwarding'
Assert-True -Condition ($success.Summary.counts.process_count -eq 2 -and $success.Summary.counts.session_count -eq 2) -Message 'success process and session counts'
Assert-True -Condition ($success.Summary.counts.submit_count -eq 1 -and $success.Summary.counts.status_count -eq 1 -and $success.Summary.counts.retry_count -eq 0 -and $success.Summary.counts.cleanup_count -eq 0) -Message 'success action counts'

$intentPath = Join-Path $success.Root 'request-intent.json'
$intentBytes = [IO.File]::ReadAllBytes($intentPath)
Assert-True -Condition (-not ($intentBytes.Length -ge 3 -and $intentBytes[0] -eq 0xef -and $intentBytes[1] -eq 0xbb -and $intentBytes[2] -eq 0xbf)) -Message 'intent has no BOM'
$intentText = [Text.UTF8Encoding]::new($false, $true).GetString($intentBytes)
$intent = $intentText | ConvertFrom-Json -ErrorAction Stop
Assert-True -Condition ($intent.retry -is [bool] -and -not [bool]$intent.retry) -Message 'intent retry is native false'
Assert-True -Condition ($intentText -match '"retry":false') -Message 'intent JSON contains false literal'
Assert-True -Condition ([string]$intent.contract_type -ceq 'controlled_codex_canary') -Message 'intent safe contract type'
Assert-True -Condition ([string]$intent.contract_file_sha256 -ceq (Get-FileHash -LiteralPath $taskContractFile -Algorithm SHA256).Hash.ToLowerInvariant()) -Message 'intent contract hash'
Assert-True -Condition ([string]$intent.intent -ceq 'CONTROLLED_CODEX_CANARY') -Message 'intent fixed mapped intent'
Assert-True -Condition ((Get-FileHash -LiteralPath $intentPath -Algorithm SHA256).Hash.ToLowerInvariant() -ceq [string]$success.Summary.request_intent.sha256) -Message 'intent hash verified'

$submitFailure = Invoke-TestCase -Name 'submit-domain-failure' -ShouldSucceed $false
Assert-True -Condition ($submitFailure.Invocations.Count -eq 1 -and $submitFailure.Invocations[0].action -ceq 'TaskSubmit') -Message 'submit failure has no status'
Assert-True -Condition ($submitFailure.Summary.counts.submit_count -eq 1 -and $submitFailure.Summary.counts.status_count -eq 0 -and $submitFailure.Summary.counts.retry_count -eq 0 -and $submitFailure.Summary.counts.cleanup_count -eq 0) -Message 'submit failure counts'
Assert-True -Condition (-not [bool]$submitFailure.Summary.acceptance -and [string]$submitFailure.Summary.first_failure -ceq 'SUBMIT_STATUS_MISMATCH') -Message 'submit failure closed'

$statusMismatch = Invoke-TestCase -Name 'status-mismatch' -ShouldSucceed $false
Assert-True -Condition ($statusMismatch.Invocations.Count -eq 2) -Message 'status mismatch exactly two invocations'
Assert-True -Condition ($statusMismatch.Invocations[0].action -ceq 'TaskSubmit' -and $statusMismatch.Invocations[1].action -ceq 'TaskStatus') -Message 'status mismatch submit then status'
Assert-True -Condition ($statusMismatch.Summary.counts.submit_count -eq 1 -and $statusMismatch.Summary.counts.status_count -eq 1 -and $statusMismatch.Summary.counts.retry_count -eq 0 -and $statusMismatch.Summary.counts.cleanup_count -eq 0) -Message 'status mismatch counts'
Assert-True -Condition (-not [bool]$statusMismatch.Summary.acceptance -and [string]$statusMismatch.Summary.first_failure -ceq 'STATUS_LEDGER_HEAD_DIGEST_MISMATCH') -Message 'status mismatch fails closed'

foreach ($case in @($success, $submitFailure, $statusMismatch)) {
    Assert-True -Condition (Test-Path -LiteralPath (Join-Path $case.Root 'request-intent.json') -PathType Leaf) -Message 'intent preserved'
    Assert-True -Condition (Test-Path -LiteralPath (Join-Path $case.Root 'coordinator-summary.json') -PathType Leaf) -Message 'summary preserved'
    Assert-True -Condition ($case.Summary.counts.retry_count -eq 0) -Message 'no retry'
    Assert-True -Condition ($case.Summary.counts.cleanup_count -eq 0) -Message 'no cleanup'
}

$dangerousFields = @('shell', 'command', 'sql', 'path', 'file_write', 'env', 'credential')
foreach ($field in $dangerousFields) {
    Assert-ContractRejectedBeforeWrapper -Field $field
}

[ordered]@{
    result = 'PASS'
    test_root = $testRoot
    cases = 10
    rejected_before_wrapper = 7
    fake_wrapper_invocations = 5
    submit_count = 3
    status_count = 2
    retry_count = 0
    cleanup_count = 0
} | ConvertTo-Json
