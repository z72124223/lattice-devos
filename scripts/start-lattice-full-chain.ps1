[CmdletBinding()]
param(
    [ValidateSet('Serve', 'Smoke')]
    [string]$Mode = 'Serve',
    [string]$ExecutablePath,
    [switch]$SkipBuild,
    [switch]$McpOnly,
    [ValidateRange(1, 300)]
    [int]$TimeoutSeconds = 30
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

if ($McpOnly -and $Mode -ne 'Smoke') {
    throw 'LATTICE_OPERATOR_MCP_ONLY_MODE_REJECTED'
}
if ($McpOnly -and [string]::IsNullOrWhiteSpace($ExecutablePath)) {
    throw 'LATTICE_OPERATOR_MCP_ONLY_EXECUTABLE_REQUIRED'
}

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$cargo = @(Get-Command 'cargo.exe' -CommandType Application -ErrorAction Stop)[0]

Push-Location $repositoryRoot
try {
    if (-not $SkipBuild) {
        & $cargo.Source 'build' '-p' 'lattice-runtime' '--bin' 'lattice-full-chain' '--locked' '--quiet'
        if ($LASTEXITCODE -ne 0) {
            throw 'LATTICE_OPERATOR_BUILD_FAILED'
        }
    }

    if ([string]::IsNullOrWhiteSpace($ExecutablePath)) {
        $metadataText = (& $cargo.Source 'metadata' '--no-deps' '--format-version' '1' '--locked') -join "`n"
        if ($LASTEXITCODE -ne 0) {
            throw 'LATTICE_OPERATOR_METADATA_FAILED'
        }
        $metadata = $metadataText | ConvertFrom-Json
        $ExecutablePath = Join-Path ([string]$metadata.target_directory) 'debug\lattice-full-chain.exe'
    }

    $resolvedExecutable = [System.IO.Path]::GetFullPath($ExecutablePath)
    if (-not (Test-Path -LiteralPath $resolvedExecutable -PathType Leaf)) {
        throw 'LATTICE_OPERATOR_EXECUTABLE_MISSING'
    }

    if ($Mode -eq 'Serve') {
        & $resolvedExecutable
        if ($LASTEXITCODE -ne 0) {
            throw "LATTICE_OPERATOR_CHILD_EXIT_$LASTEXITCODE"
        }
        return
    }

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $resolvedExecutable
    $startInfo.WorkingDirectory = $repositoryRoot
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    $originalConsoleInputEncoding = [Console]::InputEncoding
    [Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false)
    if (-not $process.Start()) {
        [Console]::InputEncoding = $originalConsoleInputEncoding
        throw 'LATTICE_OPERATOR_START_FAILED'
    }

    try {
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $requests = @(
            '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"lattice-operator","version":"1"}}}',
            '{"jsonrpc":"2.0","method":"notifications/initialized"}',
            '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
        )
        $inputBytes = [System.Text.UTF8Encoding]::new($false).GetBytes(
            ($requests -join [Environment]::NewLine) + [Environment]::NewLine
        )
        if ($inputBytes[0] -ne 0x7b) {
            throw 'LATTICE_OPERATOR_MCP_INPUT_ENCODING_REJECTED'
        }
        $inputStream = $process.StandardInput.BaseStream
        $inputStream.Write($inputBytes, 0, $inputBytes.Length)
        $inputStream.Flush()
        $inputStream.Close()

        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $process.Kill()
            $process.WaitForExit()
            throw 'LATTICE_OPERATOR_SMOKE_TIMEOUT'
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) {
            $stableCode = @($stderr -split "`r?`n" | Where-Object { $_ -match '^[A-Z][A-Z0-9_]+$' }) | Select-Object -Last 1
            if (-not [string]::IsNullOrWhiteSpace($stableCode)) {
                throw "LATTICE_OPERATOR_CHILD_FAILED:$stableCode"
            }
            throw "LATTICE_OPERATOR_CHILD_EXIT_$($process.ExitCode)"
        }
    }
    finally {
        [Console]::InputEncoding = $originalConsoleInputEncoding
        if (-not $process.HasExited) {
            $process.Kill()
            $process.WaitForExit()
        }
        $process.Dispose()
    }

    $responseLines = @($stdout -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($responseLines.Count -ne 2) {
        throw 'LATTICE_OPERATOR_MCP_RESPONSE_COUNT_REJECTED'
    }
    try {
        $initialize = $responseLines[0] | ConvertFrom-Json
        $toolsResponse = $responseLines[1] | ConvertFrom-Json
    }
    catch {
        throw 'LATTICE_OPERATOR_MCP_RESPONSE_JSON_REJECTED'
    }
    if (
        [int]$initialize.id -ne 1 -or
        [string]$initialize.result.protocolVersion -ne '2025-11-25' -or
        $null -eq $initialize.result.capabilities.tools
    ) {
        throw 'LATTICE_OPERATOR_MCP_INITIALIZE_REJECTED'
    }
    if ([int]$toolsResponse.id -ne 2) {
        throw 'LATTICE_OPERATOR_MCP_TOOLS_LIST_REJECTED'
    }
    $toolNames = @($toolsResponse.result.tools | ForEach-Object { [string]$_.name })
    if (($toolNames -join ',') -ne 'lattice_delivery_run,lattice_delivery_status') {
        throw 'LATTICE_OPERATOR_MCP_TOOL_SET_REJECTED'
    }

    $openClawReady = $null
    if (-not $McpOnly) {
        foreach ($line in @($stderr -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })) {
            try {
                $candidate = $line | ConvertFrom-Json
                if (
                    [string]$candidate.event -eq 'ready' -and
                    [string]$candidate.entrypoint -eq 'openclaw-typed'
                ) {
                    $openClawReady = $candidate
                    break
                }
            }
            catch {
                continue
            }
        }
        if ($null -eq $openClawReady) {
            throw 'LATTICE_OPERATOR_OPENCLAW_PUMP_NOT_READY'
        }
    }

    [ordered]@{
        status = 'PASS'
        component = 'lattice-full-chain-operator-smoke'
        executable = [System.IO.Path]::GetFileName($resolvedExecutable)
        mcp_initialize = $true
        mcp_tools = $toolNames
        openclaw_pump = $(if ($McpOnly) { 'NOT_CHECKED' } else { 'READY' })
        openclaw_runtime_kind = $(if ($McpOnly) { $null } else { [string]$openClawReady.runtime_kind })
    } | ConvertTo-Json -Compress
}
finally {
    Pop-Location
}
