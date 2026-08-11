[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$BinaryPath,

    [string[]]$BinaryArgument = @(),

    [ValidateSet('Discovery', 'Call', 'TaskSubmit', 'TaskStatus')]
    [string]$Action = 'Discovery',

    [string]$ToolName,

    [string]$ArgumentsJson = '{}',

    [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$')]
    [string]$ClientRequestId = ('direct-stdio-' + [Guid]::NewGuid().ToString('N').Substring(0, 16)),

    [ValidatePattern('^[0-9a-f]{64}$')]
    [string]$TaskRef,

    [string]$EnvironmentFile,

    [ValidateRange(1, 3600)]
    [int]$TimeoutSeconds = 30,

    [string]$OutputDirectory = (Join-Path $PSScriptRoot 'results')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$script:ExpectedTools = @(
    'lattice_delivery_run',
    'lattice_delivery_status',
    'lattice_task_status',
    'lattice_task_submit'
)
$script:Utf8 = [Text.UTF8Encoding]::new($false)
$script:RedactionValues = [Collections.Generic.List[string]]::new()
$script:StdoutLines = [Collections.Generic.List[string]]::new()
$script:StderrText = ''
$script:FailureClassification = $null

function Get-FileSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)

    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Test-SensitiveEnvironmentName {
    param([Parameter(Mandatory = $true)][string]$Name)

    return (
        $Name -like 'LATTICE_*' -or
        $Name -match '^(?i:HERMES_|OPENCLAW_)' -or
        $Name -match '(?i)(^|_)(API_?KEY|TOKEN|SECRET|PASSWORD|CREDENTIALS?|CONNECTION_STRING|DSN)($|_)' -or
        $Name -match '^(?i:AWS_|AZURE_|GOOGLE_|GITHUB_|GH_)' -or
        $Name -in @(
            'PGPASSWORD', 'PGPASSFILE', 'DATABASE_URL', 'CODEX_HOME',
            'GIT_ASKPASS', 'SSH_ASKPASS', 'SSH_AUTH_SOCK', 'RUST_LOG'
        )
    )
}

function Test-SecretEnvironmentName {
    param([Parameter(Mandatory = $true)][string]$Name)

    return $Name -match '(?i)(^|_)(API_?KEY|TOKEN|SECRET|PASSWORD|CREDENTIALS?|CONNECTION_STRING|DSN)($|_)'
}

function Protect-Text {
    param([AllowNull()][AllowEmptyString()][string]$Text)

    if ($null -eq $Text) { return $null }
    $protected = $Text
    foreach ($value in $script:RedactionValues) {
        if (-not [string]::IsNullOrEmpty($value)) {
            $protected = $protected.Replace($value, '[REDACTED]')
        }
    }
    return $protected
}

function ConvertTo-SafeObject {
    param([AllowNull()]$Value)

    if ($null -eq $Value) { return $null }
    $json = $Value | ConvertTo-Json -Compress -Depth 50
    $safeJson = Protect-Text -Text $json
    return $safeJson | ConvertFrom-Json -ErrorAction Stop
}

function ConvertTo-ProcessArgument {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value)

    if ($Value.Length -gt 0 -and $Value -notmatch '[\s"]') { return $Value }
    $builder = [Text.StringBuilder]::new()
    $null = $builder.Append('"')
    $backslashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq '\') {
            $backslashes++
            continue
        }
        if ($character -eq '"') {
            $null = $builder.Append(('\' * (($backslashes * 2) + 1)))
            $null = $builder.Append('"')
            $backslashes = 0
            continue
        }
        if ($backslashes -gt 0) {
            $null = $builder.Append(('\' * $backslashes))
            $backslashes = 0
        }
        $null = $builder.Append($character)
    }
    if ($backslashes -gt 0) {
        $null = $builder.Append(('\' * ($backslashes * 2)))
    }
    $null = $builder.Append('"')
    return $builder.ToString()
}

function Read-EnvironmentFile {
    param([AllowNull()][AllowEmptyString()][string]$Path)

    $values = [ordered]@{}
    if ([string]::IsNullOrWhiteSpace($Path)) { return $values }
    $resolved = [IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw 'ENVIRONMENT_FILE_NOT_FOUND'
    }
    $bytes = [IO.File]::ReadAllBytes($resolved)
    if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xef -and $bytes[1] -eq 0xbb -and $bytes[2] -eq 0xbf) {
        throw 'ENVIRONMENT_FILE_BOM_REJECTED'
    }
    try {
        $text = [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
        $object = $text | ConvertFrom-Json -ErrorAction Stop
    }
    catch {
        throw 'ENVIRONMENT_FILE_JSON_REJECTED'
    }
    if ($null -eq $object -or $object -is [array] -or $object -is [string]) {
        throw 'ENVIRONMENT_FILE_OBJECT_REQUIRED'
    }
    foreach ($property in @($object.PSObject.Properties)) {
        $name = [string]$property.Name
        if ($name -cnotmatch '^[A-Z][A-Z0-9_]{0,127}$' -or $null -eq $property.Value) {
            throw 'ENVIRONMENT_ENTRY_REJECTED'
        }
        if ($property.Value -is [Collections.IEnumerable] -and $property.Value -isnot [string]) {
            throw 'ENVIRONMENT_ENTRY_REJECTED'
        }
        $value = [Convert]::ToString($property.Value, [Globalization.CultureInfo]::InvariantCulture)
        $values[$name] = $value
        if ((Test-SecretEnvironmentName -Name $name) -or ((Test-SensitiveEnvironmentName -Name $name) -and $value.Length -ge 8)) {
            if (-not $script:RedactionValues.Contains($value)) {
                $script:RedactionValues.Add($value)
            }
        }
    }
    return $values
}

function Set-ClosedChildEnvironment {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.ProcessStartInfo]$StartInfo,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Values
    )

    foreach ($name in @($StartInfo.EnvironmentVariables.Keys)) {
        if (Test-SensitiveEnvironmentName -Name ([string]$name)) {
            $StartInfo.EnvironmentVariables.Remove([string]$name)
        }
    }
    foreach ($entry in $Values.GetEnumerator()) {
        $StartInfo.EnvironmentVariables[[string]$entry.Key] = [string]$entry.Value
    }
    $StartInfo.EnvironmentVariables['NO_COLOR'] = '1'
}

function Stop-ProcessTree {
    param([Parameter(Mandatory = $true)][Diagnostics.Process]$Process)

    if ($Process.HasExited) { return $true }
    try {
        & taskkill.exe /PID $Process.Id /T /F 2>$null | Out-Null
        $null = $Process.WaitForExit(5000)
    }
    catch {
        try { $Process.Kill() } catch {}
        try { $null = $Process.WaitForExit(5000) } catch {}
    }
    return $Process.HasExited
}

function Get-RemainingMilliseconds {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Stopwatch]$Stopwatch,
        [Parameter(Mandatory = $true)][int]$Timeout
    )

    $remaining = ([long]$Timeout * 1000L) - $Stopwatch.ElapsedMilliseconds
    if ($remaining -le 0) { throw 'MCP_CLIENT_TIMEOUT' }
    return [int][Math]::Min([long]$remaining, [long][int]::MaxValue)
}

function Write-JsonLine {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)]$Message
    )

    $line = $Message | ConvertTo-Json -Compress -Depth 50
    $Process.StandardInput.WriteLine($line)
    $Process.StandardInput.Flush()
}

function Read-JsonLine {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)][Diagnostics.Stopwatch]$Stopwatch,
        [Parameter(Mandatory = $true)][int]$Timeout
    )

    $task = $Process.StandardOutput.ReadLineAsync()
    $remaining = Get-RemainingMilliseconds -Stopwatch $Stopwatch -Timeout $Timeout
    if (-not $task.Wait($remaining)) { throw 'MCP_CLIENT_TIMEOUT' }
    $line = $task.Result
    if ($null -eq $line) { throw 'MCP_STDOUT_EOF_BEFORE_RESPONSE' }
    $script:StdoutLines.Add((Protect-Text -Text $line))
    try { return $line | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'MCP_STDOUT_JSON_REJECTED' }
}

function Assert-JsonRpcSuccess {
    param(
        [Parameter(Mandatory = $true)]$Response,
        [Parameter(Mandatory = $true)][string]$Stage
    )

    if ([string]$Response.jsonrpc -cne '2.0') { throw ('MCP_RESPONSE_INVALID|' + $Stage) }
    if ($Response.PSObject.Properties.Name -contains 'error') {
        $script:FailureClassification = 'JSONRPC_ERROR'
        throw ('MCP_JSONRPC_ERROR|' + $Stage)
    }
    if (-not ($Response.PSObject.Properties.Name -contains 'result')) {
        throw ('MCP_RESPONSE_INVALID|' + $Stage)
    }
}

function Get-ToolErrorCode {
    param([AllowNull()]$Response)

    if ($null -eq $Response) { return $null }
    if ($Response.PSObject.Properties.Name -contains 'error') {
        return [string]$Response.error.code
    }
    $structured = $Response.result.structuredContent
    if ($null -ne $structured) {
        foreach ($name in @('code', 'error_code', 'errorCode')) {
            if ($structured.PSObject.Properties.Name -contains $name) {
                return [string]$structured.$name
            }
        }
    }
    return $null
}

$startedAt = [DateTimeOffset]::UtcNow
$sessionId = [Guid]::NewGuid().ToString('N')
$sessionDirectory = $null
$process = $null
$stderrTask = $null
$stopwatch = [Diagnostics.Stopwatch]::StartNew()
$classification = 'CLIENT_SETUP_FAILED'
$success = $false
$processStarted = $false
$processId = $null
$exitCode = $null
$cleanupAttempted = $false
$cleanupSucceeded = $true
$initializeResponse = $null
$toolsResponse = $null
$callResponse = $null
$observedTools = @()
$exactFour = $false
$environmentValues = [ordered]@{}
$binary = $null
$binarySha256 = $null
$failureMessage = $null

try {
    $binary = [IO.Path]::GetFullPath($BinaryPath)
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'BINARY_NOT_FOUND' }
    $binarySha256 = Get-FileSha256 -Path $binary
    $environmentValues = Read-EnvironmentFile -Path $EnvironmentFile

    switch ($Action) {
        'Call' {
            if ([string]::IsNullOrWhiteSpace($ToolName)) { throw 'TOOL_NAME_REQUIRED' }
            try { $callArguments = $ArgumentsJson | ConvertFrom-Json -ErrorAction Stop }
            catch { throw 'TOOL_ARGUMENTS_JSON_REJECTED' }
        }
        'TaskSubmit' {
            $ToolName = 'lattice_task_submit'
            $callArguments = [ordered]@{
                client_request_id = $ClientRequestId
                intent = 'CONTROLLED_CODEX_CANARY'
            }
        }
        'TaskStatus' {
            if ([string]::IsNullOrWhiteSpace($TaskRef)) { throw 'TASK_REF_REQUIRED' }
            $ToolName = 'lattice_task_status'
            $callArguments = [ordered]@{ task_ref = $TaskRef }
        }
        default { $callArguments = $null }
    }

    $root = [IO.Path]::GetFullPath($OutputDirectory)
    $null = New-Item -ItemType Directory -Path $root -Force
    $sessionDirectory = Join-Path $root ('session-' + $startedAt.UtcDateTime.ToString('yyyyMMddTHHmmssfffZ') + '-' + $sessionId.Substring(0, 8))
    $null = New-Item -ItemType Directory -Path $sessionDirectory

    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $binary
    $startInfo.Arguments = [string]::Join(' ', @($BinaryArgument | ForEach-Object { ConvertTo-ProcessArgument -Value $_ }))
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.CreateNoWindow = $true
    $startInfo.StandardOutputEncoding = $script:Utf8
    $startInfo.StandardErrorEncoding = $script:Utf8
    Set-ClosedChildEnvironment -StartInfo $startInfo -Values $environmentValues

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw 'PROCESS_START_REJECTED' }
    $processStarted = $true
    $processId = $process.Id
    $stderrTask = $process.StandardError.ReadToEndAsync()

    Write-JsonLine -Process $process -Message ([ordered]@{
        jsonrpc = '2.0'
        id = 1
        method = 'initialize'
        params = [ordered]@{
            protocolVersion = '2025-11-25'
            capabilities = [ordered]@{}
            clientInfo = [ordered]@{ name = 'lattice-direct-stdio-client'; version = '1.0.0' }
        }
    })
    $initializeResponse = Read-JsonLine -Process $process -Stopwatch $stopwatch -Timeout $TimeoutSeconds
    Assert-JsonRpcSuccess -Response $initializeResponse -Stage 'initialize'

    Write-JsonLine -Process $process -Message ([ordered]@{
        jsonrpc = '2.0'
        method = 'notifications/initialized'
    })
    Write-JsonLine -Process $process -Message ([ordered]@{
        jsonrpc = '2.0'
        id = 2
        method = 'tools/list'
        params = [ordered]@{}
    })
    $toolsResponse = Read-JsonLine -Process $process -Stopwatch $stopwatch -Timeout $TimeoutSeconds
    Assert-JsonRpcSuccess -Response $toolsResponse -Stage 'tools/list'
    if ($null -eq $toolsResponse.result.tools) { throw 'MCP_TOOL_LIST_INVALID' }
    $observedTools = @($toolsResponse.result.tools | ForEach-Object { [string]$_.name })
    $sortedObserved = @($observedTools | Sort-Object -CaseSensitive)
    $exactFour = ($sortedObserved.Count -eq $script:ExpectedTools.Count) -and (($sortedObserved -join "`n") -ceq ($script:ExpectedTools -join "`n"))
    if (-not $exactFour) {
        $classification = 'TOOL_SET_MISMATCH'
        throw 'MCP_EXACT_FOUR_TOOLS_REJECTED'
    }

    if ($Action -eq 'Discovery') {
        $classification = 'DISCOVERY_OK'
        $success = $true
    }
    else {
        Write-JsonLine -Process $process -Message ([ordered]@{
            jsonrpc = '2.0'
            id = 3
            method = 'tools/call'
            params = [ordered]@{ name = $ToolName; arguments = $callArguments }
        })
        $callResponse = Read-JsonLine -Process $process -Stopwatch $stopwatch -Timeout $TimeoutSeconds
        Assert-JsonRpcSuccess -Response $callResponse -Stage 'tools/call'
        if ([bool]$callResponse.result.isError) {
            $classification = 'TOOL_ERROR'
            $success = $false
        }
        else {
            $classification = 'CALL_OK'
            $success = $true
        }
    }
}
catch {
    $failureMessage = Protect-Text -Text $_.Exception.Message
    if ($classification -in @('TOOL_SET_MISMATCH', 'TOOL_ERROR')) {}
    elseif ($script:FailureClassification) { $classification = $script:FailureClassification }
    elseif ($_.Exception.Message -ceq 'MCP_CLIENT_TIMEOUT') { $classification = 'TIMEOUT' }
    elseif ($_.Exception.Message -ceq 'MCP_STDOUT_EOF_BEFORE_RESPONSE') { $classification = 'PROCESS_EXITED_BEFORE_RESPONSE' }
    elseif ($_.Exception.Message -in @('MCP_STDOUT_JSON_REJECTED', 'MCP_TOOL_LIST_INVALID') -or $_.Exception.Message -like 'MCP_RESPONSE_INVALID*') { $classification = 'PROTOCOL_ERROR' }
    elseif ($processStarted) { $classification = 'CLIENT_RUNTIME_ERROR' }
    else { $classification = 'CLIENT_SETUP_FAILED' }
    $success = $false
}
finally {
    $stopwatch.Stop()
    if ($null -ne $process) {
        if (-not $process.HasExited) {
            try { $process.StandardInput.Close() } catch {}
            try { $null = $process.WaitForExit(1000) } catch {}
        }
        if (-not $process.HasExited) {
            $cleanupAttempted = $true
            $cleanupSucceeded = Stop-ProcessTree -Process $process
        }
        if ($process.HasExited) { $exitCode = $process.ExitCode }
        if ($null -ne $stderrTask) {
            try {
                if ($stderrTask.Wait(2000)) { $script:StderrText = Protect-Text -Text $stderrTask.Result }
            }
            catch {}
        }
    }
}

if ($null -eq $sessionDirectory) {
    $root = [IO.Path]::GetFullPath($OutputDirectory)
    $null = New-Item -ItemType Directory -Path $root -Force
    $sessionDirectory = Join-Path $root ('session-' + $startedAt.UtcDateTime.ToString('yyyyMMddTHHmmssfffZ') + '-' + $sessionId.Substring(0, 8))
    $null = New-Item -ItemType Directory -Path $sessionDirectory -Force
}

$stdoutPath = Join-Path $sessionDirectory 'stdout.jsonl'
$stderrPath = Join-Path $sessionDirectory 'stderr.log'
$summaryPath = Join-Path $sessionDirectory 'summary.json'
[IO.File]::WriteAllText($stdoutPath, $(if ($script:StdoutLines.Count -eq 0) { '' } else { ([string]::Join("`n", $script:StdoutLines) + "`n") }), $script:Utf8)
[IO.File]::WriteAllText($stderrPath, $(if ([string]::IsNullOrEmpty($script:StderrText)) { '' } else { $script:StderrText }), $script:Utf8)

$safeInitialize = ConvertTo-SafeObject -Value $initializeResponse
$safeTools = ConvertTo-SafeObject -Value $toolsResponse
$safeCall = ConvertTo-SafeObject -Value $callResponse
$summary = [ordered]@{
    schema = 'lattice.direct-stdio-client.v1'
    session_id = $sessionId
    started_at_utc = $startedAt.ToString('o')
    duration_ms = $stopwatch.ElapsedMilliseconds
    action = $Action
    classification = $classification
    success = $success
    protocol = 'mcp-jsonrpc-2.0-json-lines'
    binary = [ordered]@{
        path = $binary
        sha256 = $binarySha256
        argument_count = @($BinaryArgument).Count
    }
    process = [ordered]@{
        started = $processStarted
        id = $processId
        exit_code = $exitCode
        cleanup_attempted = $cleanupAttempted
        cleanup_succeeded = $cleanupSucceeded
    }
    environment_keys = @($environmentValues.Keys)
    discovery = [ordered]@{
        initialize_received = ($null -ne $initializeResponse)
        negotiated_protocol = $(if ($null -ne $safeInitialize) { [string]$safeInitialize.result.protocolVersion } else { $null })
        tools_list_received = ($null -ne $toolsResponse)
        tool_names = @($observedTools)
        exact_four = $exactFour
    }
    call = $(if ($null -eq $callResponse) { $null } else {
        [ordered]@{
            tool_name = $ToolName
            is_error = [bool]$callResponse.result.isError
            error_code = Get-ToolErrorCode -Response $callResponse
            result = $safeCall.result
        }
    })
    failure_message = $failureMessage
    artifacts = [ordered]@{
        stdout = $stdoutPath
        stderr = $stderrPath
        summary = $summaryPath
    }
}
$summaryJson = $summary | ConvertTo-Json -Depth 50
[IO.File]::WriteAllText($summaryPath, ($summaryJson + "`n"), $script:Utf8)

if ($null -ne $process) { $process.Dispose() }
$summaryJson
