[CmdletBinding(DefaultParameterSetName = 'Test')]
param(
    [Parameter(Mandatory = $true, ParameterSetName = 'Server')]
    [switch]$FixtureServer,

    [Parameter(ParameterSetName = 'Server')]
    [ValidateRange(0, 10000)]
    [int]$InitializeDelayMilliseconds = 0,

    [Parameter(ParameterSetName = 'Server')]
    [ValidateRange(0, 10000)]
    [int]$ToolsListDelayMilliseconds = 0,

    [Parameter(ParameterSetName = 'Server')]
    [ValidateRange(0, 10000)]
    [int]$ToolCallDelayMilliseconds = 0
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:Utf8 = [Text.UTF8Encoding]::new($false)
$script:ExpectedTools = @(
    'lattice_delivery_reconcile',
    'lattice_delivery_run',
    'lattice_delivery_status',
    'lattice_foreman_checkpoint',
    'lattice_runtime_status',
    'lattice_task_status',
    'lattice_task_submit'
)

function Write-FixtureResponse {
    param([Parameter(Mandatory = $true)]$Message)

    $line = $Message | ConvertTo-Json -Compress -Depth 20
    [Console]::Out.WriteLine($line)
    [Console]::Out.Flush()
}

function Start-FixtureServer {
    [Console]::InputEncoding = $script:Utf8
    [Console]::OutputEncoding = $script:Utf8

    while ($true) {
        $line = [Console]::In.ReadLine()
        if ($null -eq $line) { return }
        $request = $line | ConvertFrom-Json -ErrorAction Stop
        if (-not ($request.PSObject.Properties.Name -contains 'id')) { continue }

        switch ([int]$request.id) {
            1 {
                Start-Sleep -Milliseconds $InitializeDelayMilliseconds
                Write-FixtureResponse -Message ([ordered]@{
                    jsonrpc = '2.0'
                    id = 1
                    result = [ordered]@{ protocolVersion = '2025-11-25' }
                })
            }
            2 {
                Start-Sleep -Milliseconds $ToolsListDelayMilliseconds
                Write-FixtureResponse -Message ([ordered]@{
                    jsonrpc = '2.0'
                    id = 2
                    result = [ordered]@{
                        tools = @($script:ExpectedTools | ForEach-Object {
                            [ordered]@{ name = $_ }
                        })
                    }
                })
            }
            3 {
                if ([string]$request.params.name -cne 'lattice_task_submit') {
                    throw 'FIXTURE_TOOL_NAME_REJECTED'
                }
                Start-Sleep -Milliseconds $ToolCallDelayMilliseconds
                Write-FixtureResponse -Message ([ordered]@{
                    jsonrpc = '2.0'
                    id = 3
                    result = [ordered]@{
                        isError = $false
                        content = @()
                        structuredContent = [ordered]@{ status = 'COMPLETED' }
                    }
                })
                return
            }
            default { throw 'FIXTURE_REQUEST_ID_REJECTED' }
        }
    }
}

if ($FixtureServer) {
    Start-FixtureServer
    exit 0
}

$wrapperPath = Join-Path $PSScriptRoot 'Invoke-LatticeMcp.ps1'
$tokens = $null
$parserErrors = $null
$wrapperAst = [Management.Automation.Language.Parser]::ParseFile(
    $wrapperPath,
    [ref]$tokens,
    [ref]$parserErrors)
if ($parserErrors.Count -ne 0) {
    throw 'DIRECT_STDIO_WRAPPER_PARSE_REJECTED'
}

$toolCallTimeoutParameter = $wrapperAst.ParamBlock.Parameters | Where-Object {
    $_.Name.VariablePath.UserPath -ceq 'ToolCallTimeoutSeconds'
}
$testRoot = Join-Path $PSScriptRoot ('.timeout-fixture-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $testRoot

function Invoke-WrapperCase {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][int]$TimeoutSeconds,
        [AllowNull()][Nullable[int]]$ToolCallTimeoutSeconds,
        [Parameter(Mandatory = $true)][int]$InitializeDelayMilliseconds,
        [Parameter(Mandatory = $true)][int]$ToolsListDelayMilliseconds,
        [Parameter(Mandatory = $true)][int]$ToolCallDelayMilliseconds
    )

    $hostPath = (Get-Process -Id $PID).Path
    $caseRoot = Join-Path $testRoot $Name
    $binaryArguments = @(
        '-NoLogo',
        '-NoProfile',
        '-NonInteractive',
        '-File',
        $PSCommandPath,
        '-FixtureServer',
        '-InitializeDelayMilliseconds',
        [string]$InitializeDelayMilliseconds,
        '-ToolsListDelayMilliseconds',
        [string]$ToolsListDelayMilliseconds,
        '-ToolCallDelayMilliseconds',
        [string]$ToolCallDelayMilliseconds
    )
    $invokeParameters = @{
        BinaryPath = $hostPath
        BinaryArgument = $binaryArguments
        Action = 'TaskSubmit'
        ClientRequestId = ('timeout-fixture-' + $Name)
        TimeoutSeconds = $TimeoutSeconds
        OutputDirectory = $caseRoot
    }
    if ($null -ne $ToolCallTimeoutSeconds) {
        $invokeParameters.ToolCallTimeoutSeconds = [int]$ToolCallTimeoutSeconds
    }

    $output = @(& $wrapperPath @invokeParameters)
    $json = [string]::Join([Environment]::NewLine, @($output | ForEach-Object { [string]$_ }))
    return $json | ConvertFrom-Json -ErrorAction Stop
}

function Assert-JsonLinesUtf8NoBom {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][int[]]$ExpectedIds
    )

    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xef -and $bytes[1] -eq 0xbb -and $bytes[2] -eq 0xbf) {
        throw 'DIRECT_STDIO_STDOUT_BOM_REJECTED'
    }
    if ($bytes.Length -eq 0 -or $bytes[$bytes.Length - 1] -ne 0x0a) {
        throw 'DIRECT_STDIO_STDOUT_NEWLINE_REJECTED'
    }
    for ($index = 1; $index -lt $bytes.Length; $index++) {
        if ($bytes[$index - 1] -eq 0x0d -and $bytes[$index] -eq 0x0a) {
            throw 'DIRECT_STDIO_STDOUT_CRLF_REJECTED'
        }
    }

    $text = [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
    $lines = @($text.TrimEnd("`n").Split("`n"))
    $ids = @($lines | ForEach-Object {
        [int](($_ | ConvertFrom-Json -ErrorAction Stop).id)
    })
    if (($ids -join ',') -cne ($ExpectedIds -join ',')) {
        throw 'DIRECT_STDIO_STDOUT_IDS_REJECTED'
    }
}

try {
    if ($null -eq $toolCallTimeoutParameter) {
        $sharedDeadline = Invoke-WrapperCase `
            -Name 'shared-deadline-red' `
            -TimeoutSeconds 4 `
            -ToolCallTimeoutSeconds $null `
            -InitializeDelayMilliseconds 900 `
            -ToolsListDelayMilliseconds 900 `
            -ToolCallDelayMilliseconds 2200
        if ([string]$sharedDeadline.classification -cne 'TIMEOUT' -or [bool]$sharedDeadline.success) {
            throw 'DIRECT_STDIO_SHARED_DEADLINE_RED_NOT_REPRODUCED'
        }
        if (-not [bool]$sharedDeadline.discovery.initialize_received -or
            -not [bool]$sharedDeadline.discovery.tools_list_received) {
            throw 'DIRECT_STDIO_SHARED_DEADLINE_RED_HANDSHAKE_INCOMPLETE'
        }
        Write-Output 'DIRECT_STDIO_SHARED_DEADLINE_RED=TIMEOUT'
        throw 'DIRECT_STDIO_TOOL_CALL_TIMEOUT_PARAMETER_MISSING'
    }

    if ($toolCallTimeoutParameter.DefaultValue.Extent.Text -cne '$TimeoutSeconds') {
        throw 'DIRECT_STDIO_TOOL_CALL_TIMEOUT_DEFAULT_REJECTED'
    }
    $parameterMetadata = (Get-Command -Name $wrapperPath).Parameters['ToolCallTimeoutSeconds']
    $range = @($parameterMetadata.Attributes | Where-Object {
        $_ -is [Management.Automation.ValidateRangeAttribute]
    })
    if ($range.Count -ne 1 -or [int]$range[0].MinRange -gt 900 -or [int]$range[0].MaxRange -lt 900) {
        throw 'DIRECT_STDIO_TOOL_CALL_TIMEOUT_900_REJECTED'
    }

    $defaultCompatibility = Invoke-WrapperCase `
        -Name 'default-compatibility' `
        -TimeoutSeconds 2 `
        -ToolCallTimeoutSeconds $null `
        -InitializeDelayMilliseconds 0 `
        -ToolsListDelayMilliseconds 0 `
        -ToolCallDelayMilliseconds 50
    if ([string]$defaultCompatibility.classification -cne 'CALL_OK' -or
        -not [bool]$defaultCompatibility.success) {
        throw 'DIRECT_STDIO_TOOL_CALL_TIMEOUT_DEFAULT_RUNTIME_REJECTED'
    }

    $insideDeadline = Invoke-WrapperCase `
        -Name 'inside-tool-call-deadline' `
        -TimeoutSeconds 4 `
        -ToolCallTimeoutSeconds 4 `
        -InitializeDelayMilliseconds 900 `
        -ToolsListDelayMilliseconds 900 `
        -ToolCallDelayMilliseconds 2200
    if ([string]$insideDeadline.classification -cne 'CALL_OK' -or -not [bool]$insideDeadline.success) {
        throw ('DIRECT_STDIO_FRESH_TOOL_CALL_DEADLINE_REJECTED|' +
            [string]$insideDeadline.classification + '|' +
            [string]$insideDeadline.failure_message + '|initialize=' +
            [string]$insideDeadline.discovery.initialize_received + '|tools=' +
            [string]$insideDeadline.discovery.tools_list_received)
    }
    if ([bool]$insideDeadline.process.cleanup_attempted) {
        throw 'DIRECT_STDIO_SUCCESS_CLEANUP_REJECTED'
    }
    Assert-JsonLinesUtf8NoBom -Path ([string]$insideDeadline.artifacts.stdout) -ExpectedIds @(1, 2, 3)

    $beyondDeadline = Invoke-WrapperCase `
        -Name 'beyond-tool-call-deadline' `
        -TimeoutSeconds 2 `
        -ToolCallTimeoutSeconds 1 `
        -InitializeDelayMilliseconds 100 `
        -ToolsListDelayMilliseconds 100 `
        -ToolCallDelayMilliseconds 4000
    if ([string]$beyondDeadline.classification -cne 'TIMEOUT' -or
        [string]$beyondDeadline.failure_message -cne 'MCP_CLIENT_TIMEOUT' -or
        [bool]$beyondDeadline.success) {
        throw ('DIRECT_STDIO_OWN_TOOL_CALL_DEADLINE_REJECTED|' +
            [string]$beyondDeadline.classification + '|' +
            [string]$beyondDeadline.failure_message)
    }
    if (-not [bool]$beyondDeadline.process.cleanup_attempted -or -not [bool]$beyondDeadline.process.cleanup_succeeded) {
        throw 'DIRECT_STDIO_TIMEOUT_STOP_PATH_REJECTED'
    }
    Assert-JsonLinesUtf8NoBom -Path ([string]$beyondDeadline.artifacts.stdout) -ExpectedIds @(1, 2)

    Write-Output 'DIRECT_STDIO_TOOL_CALL_TIMEOUT_OFFLINE=PASS'
    Write-Output 'DIRECT_STDIO_TOOL_CALL_TIMEOUT_DEFAULT_COMPATIBILITY=PASS'
    Write-Output 'DIRECT_STDIO_TOOL_CALL_TIMEOUT_900_BINDING=PASS'
    Write-Output 'DIRECT_STDIO_TOOL_CALL_TIMEOUT_STOP_PATH=PASS'
}
finally {
    if (Test-Path -LiteralPath $testRoot) {
        [IO.Directory]::Delete($testRoot, $true)
    }
}
