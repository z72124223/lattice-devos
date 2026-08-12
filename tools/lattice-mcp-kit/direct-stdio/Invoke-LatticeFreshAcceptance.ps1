[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$BinaryPath,

    [Parameter(Mandatory = $true)]
    [string]$EnvironmentFile,

    [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$')]
    [string]$ClientRequestId = ('fresh-accept-' + [Guid]::NewGuid().ToString('N').Substring(0, 16)),

    [string]$WrapperPath = (Join-Path $PSScriptRoot 'Invoke-LatticeMcp.ps1'),

    [string]$OutputRoot = (Join-Path $PSScriptRoot ('fresh-acceptance-' + [DateTimeOffset]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))),

    [ValidateRange(1, 3600)]
    [int]$InitializeTimeoutSeconds = 90,

    [ValidateRange(1, 3600)]
    [int]$SubmitTimeoutSeconds = 900,

    [ValidateRange(1, 3600)]
    [int]$StatusTimeoutSeconds = 180
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:Utf8 = [Text.UTF8Encoding]::new($false)

function Get-Sha256 {
    param([Parameter(Mandatory = $true)][string]$Path)

    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-ArtifactRecord {
    param([Parameter(Mandatory = $true)][string]$Path)

    $resolved = [IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw 'CHILD_ARTIFACT_MISSING'
    }
    return [ordered]@{ path = $resolved; sha256 = Get-Sha256 -Path $resolved }
}

function Read-WrapperResult {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Output)

    $text = ($Output | Out-String).Trim()
    if ([string]::IsNullOrWhiteSpace($text)) { throw 'CHILD_SUMMARY_OUTPUT_MISSING' }
    try { $reported = $text | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'CHILD_SUMMARY_OUTPUT_JSON_REJECTED' }

    if ($null -eq $reported.artifacts -or [string]::IsNullOrWhiteSpace([string]$reported.artifacts.summary)) {
        throw 'CHILD_SUMMARY_REFERENCE_MISSING'
    }
    $summaryArtifact = Get-ArtifactRecord -Path ([string]$reported.artifacts.summary)
    try {
        $saved = [Text.UTF8Encoding]::new($false, $true).GetString([IO.File]::ReadAllBytes($summaryArtifact.path)) |
            ConvertFrom-Json -ErrorAction Stop
    }
    catch { throw 'CHILD_SUMMARY_ARTIFACT_JSON_REJECTED' }

    if ([string]$reported.session_id -cne [string]$saved.session_id -or [string]$reported.action -cne [string]$saved.action) {
        throw 'CHILD_SUMMARY_OUTPUT_MISMATCH'
    }
    $artifacts = [ordered]@{
        summary = $summaryArtifact
        stdout = Get-ArtifactRecord -Path ([string]$saved.artifacts.stdout)
        stderr = Get-ArtifactRecord -Path ([string]$saved.artifacts.stderr)
    }
    try {
        $outputRecords = @([Text.UTF8Encoding]::new($false, $true).GetString([IO.File]::ReadAllBytes($artifacts.stdout.path)) -split "`r?`n" |
            Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
            ForEach-Object { $_ | ConvertFrom-Json -ErrorAction Stop })
    }
    catch { throw 'CHILD_OUTPUT_ARTIFACT_JSON_REJECTED' }
    if ($outputRecords.Count -eq 0) { throw 'CHILD_OUTPUT_ARTIFACT_EMPTY' }
    $callResponse = $outputRecords[-1]
    if ($null -eq $callResponse.result -or $callResponse.result.PSObject.Properties.Name -notcontains 'isError') {
        throw 'CHILD_CALL_RESPONSE_MISSING'
    }
    if ([bool]$saved.call.is_error -ne [bool]$callResponse.result.isError) { throw 'CHILD_CALL_RESPONSE_MISMATCH' }
    return [pscustomobject]@{ Summary = $saved; CallResponse = $callResponse; Artifacts = $artifacts }
}

function Get-StructuredContent {
    param(
        [Parameter(Mandatory = $true)]$Child,
        [Parameter(Mandatory = $true)][string]$Stage
    )

    if ($null -eq $Child.Summary.call -or [bool]$Child.Summary.call.is_error -or [bool]$Child.CallResponse.result.isError) {
        throw ($Stage + '_TRANSPORT_ERROR')
    }
    if ($null -eq $Child.CallResponse.result.structuredContent) {
        throw ($Stage + '_STRUCTURED_CONTENT_MISSING')
    }
    return $Child.CallResponse.result.structuredContent
}

function Get-RequiredValue {
    param(
        [Parameter(Mandatory = $true)]$Object,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    if ($Object.PSObject.Properties.Name -notcontains $Name -or $null -eq $Object.$Name) { throw $Failure }
    return $Object.$Name
}

$startedAt = [DateTimeOffset]::UtcNow
$stopwatch = [Diagnostics.Stopwatch]::StartNew()
$root = $null
$rootCreated = $false
$summaryPath = $null
$intentPath = $null
$intentSha256 = $null
$firstFailure = $null
$acceptance = $false
$accepted = $null
$children = [ordered]@{ submit = $null; status = $null }
$counts = [ordered]@{
    process_count = 0
    session_count = 0
    submit_count = 0
    status_count = 0
    retry_count = 0
}

try {
    if (-not [IO.Path]::IsPathRooted($BinaryPath)) { throw 'BINARY_PATH_NOT_ABSOLUTE' }
    $binary = [IO.Path]::GetFullPath($BinaryPath)
    $environment = [IO.Path]::GetFullPath($EnvironmentFile)
    $wrapper = [IO.Path]::GetFullPath($WrapperPath)
    $root = [IO.Path]::GetFullPath($OutputRoot)
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'BINARY_NOT_FOUND' }
    if (-not (Test-Path -LiteralPath $environment -PathType Leaf)) { throw 'ENVIRONMENT_FILE_NOT_FOUND' }
    if (-not (Test-Path -LiteralPath $wrapper -PathType Leaf)) { throw 'WRAPPER_NOT_FOUND' }
    if (Test-Path -LiteralPath $root) { throw 'OUTPUT_ROOT_MUST_BE_ABSENT' }

    $null = New-Item -ItemType Directory -Path $root
    $rootCreated = $true
    $summaryPath = Join-Path $root 'coordinator-summary.json'
    $intentPath = Join-Path $root 'request-intent.json'
    $intent = [ordered]@{
        schema = 'lattice.fresh-acceptance-intent.v1'
        action = 'TaskSubmit'
        client_request_id = $ClientRequestId
        intent = 'CONTROLLED_CODEX_CANARY'
        retry = $false
    }
    [IO.File]::WriteAllText($intentPath, (($intent | ConvertTo-Json -Compress -Depth 10) + "`n"), $script:Utf8)
    $intentSha256 = Get-Sha256 -Path $intentPath
    $intentBytes = [IO.File]::ReadAllBytes($intentPath)
    if ($intentBytes.Length -ge 3 -and $intentBytes[0] -eq 0xef -and $intentBytes[1] -eq 0xbb -and $intentBytes[2] -eq 0xbf) {
        throw 'INTENT_BOM_REJECTED'
    }
    try {
        $parsedIntent = [Text.UTF8Encoding]::new($false, $true).GetString($intentBytes) | ConvertFrom-Json -ErrorAction Stop
    }
    catch { throw 'INTENT_JSON_REJECTED' }
    if ([string]$parsedIntent.client_request_id -cne $ClientRequestId -or $parsedIntent.retry -isnot [bool] -or [bool]$parsedIntent.retry) {
        throw 'INTENT_CONTENT_MISMATCH'
    }
    if ((Get-Sha256 -Path $intentPath) -cne $intentSha256) { throw 'INTENT_HASH_MISMATCH' }

    $counts.process_count++
    $counts.session_count++
    $counts.submit_count++
    $submitOutput = @(& $wrapper `
        -BinaryPath $binary `
        -EnvironmentFile $environment `
        -Action TaskSubmit `
        -ClientRequestId $ClientRequestId `
        -TimeoutSeconds $InitializeTimeoutSeconds `
        -ToolCallTimeoutSeconds $SubmitTimeoutSeconds `
        -OutputDirectory (Join-Path $root 'submit'))
    $submit = Read-WrapperResult -Output $submitOutput
    $children.submit = [ordered]@{
        session_id = [string]$submit.Summary.session_id
        artifacts = $submit.Artifacts
    }
    $submitContent = Get-StructuredContent -Child $submit -Stage 'SUBMIT'
    if ([string](Get-RequiredValue -Object $submitContent -Name 'status' -Failure 'SUBMIT_STATUS_MISSING') -cne 'COMPLETED') {
        throw 'SUBMIT_STATUS_MISMATCH'
    }
    if ([string](Get-RequiredValue -Object $submitContent -Name 'task_state' -Failure 'SUBMIT_TASK_STATE_MISSING') -cne 'COMPLETED') {
        throw 'SUBMIT_TASK_STATE_MISMATCH'
    }
    $taskRef = [string](Get-RequiredValue -Object $submitContent -Name 'task_ref' -Failure 'SUBMIT_TASK_REF_MISSING')
    if ([string]::IsNullOrWhiteSpace($taskRef) -or $taskRef -cnotmatch '^[0-9a-f]{64}$') { throw 'SUBMIT_TASK_REF_INVALID' }
    $resultDigest = Get-RequiredValue -Object $submitContent -Name 'result_digest' -Failure 'SUBMIT_RESULT_DIGEST_MISSING'
    $ledgerHeadDigest = Get-RequiredValue -Object $submitContent -Name 'ledger_head_digest' -Failure 'SUBMIT_LEDGER_HEAD_DIGEST_MISSING'

    $counts.process_count++
    $counts.session_count++
    $counts.status_count++
    $statusOutput = @(& $wrapper `
        -BinaryPath $binary `
        -EnvironmentFile $environment `
        -Action TaskStatus `
        -TaskRef $taskRef `
        -TimeoutSeconds $InitializeTimeoutSeconds `
        -ToolCallTimeoutSeconds $StatusTimeoutSeconds `
        -OutputDirectory (Join-Path $root 'status'))
    $status = Read-WrapperResult -Output $statusOutput
    $children.status = [ordered]@{
        session_id = [string]$status.Summary.session_id
        artifacts = $status.Artifacts
    }
    $statusContent = Get-StructuredContent -Child $status -Stage 'STATUS'
    foreach ($field in @('task_ref', 'status', 'task_state', 'result_digest', 'ledger_head_digest')) {
        $statusValue = Get-RequiredValue -Object $statusContent -Name $field -Failure ('STATUS_' + $field.ToUpperInvariant() + '_MISSING')
        if ([string]$statusValue -cne [string]$submitContent.$field) { throw ('STATUS_' + $field.ToUpperInvariant() + '_MISMATCH') }
    }

    $accepted = [ordered]@{
        task_ref = $taskRef
        status = [string]$submitContent.status
        task_state = [string]$submitContent.task_state
        result_digest = $resultDigest
        ledger_head_digest = $ledgerHeadDigest
    }
    $acceptance = $true
}
catch {
    $failureText = $_.Exception.Message
    $firstFailure = $(if ($failureText -cmatch '^[A-Z][A-Z0-9_]{0,127}$') { $failureText } else { 'COORDINATOR_RUNTIME_ERROR' })
}
finally {
    $stopwatch.Stop()
    if ($rootCreated -and $null -ne $root -and (Test-Path -LiteralPath $root -PathType Container)) {
        $endedAt = [DateTimeOffset]::UtcNow
        $coordinatorSummary = [ordered]@{
            schema = 'lattice.fresh-acceptance-coordinator.v1'
            started_at_utc = $startedAt.ToString('o')
            ended_at_utc = $endedAt.ToString('o')
            elapsed_ms = $stopwatch.ElapsedMilliseconds
            counts = $counts
            timeouts_seconds = [ordered]@{
                initialize = $InitializeTimeoutSeconds
                submit = $SubmitTimeoutSeconds
                status = $StatusTimeoutSeconds
            }
            request_intent = [ordered]@{ path = $intentPath; sha256 = $intentSha256 }
            children = $children
            accepted_result = $accepted
            first_failure = $firstFailure
            acceptance = $acceptance
        }
        [IO.File]::WriteAllText($summaryPath, (($coordinatorSummary | ConvertTo-Json -Depth 30) + "`n"), $script:Utf8)
    }
}

if (-not $acceptance) { throw $firstFailure }
Get-Content -LiteralPath $summaryPath -Raw -Encoding UTF8
