[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:ExactMarker = 'TASK038_FOUR_TOOL_ACCEPTANCE=PASS'
$script:RepoRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$script:Runner = Join-Path $PSScriptRoot 'run-task038-four-tool-acceptance.ps1'
$script:Utf8 = [Text.UTF8Encoding]::new($false)

function Get-GateClassification {
    param(
        [Parameter(Mandatory = $true)][int]$ExitCode,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Lines
    )

    $passCount = @($Lines | Where-Object { $_ -ceq $script:ExactMarker }).Count
    $skipCount = @($Lines | Where-Object { $_ -match '(?i)(?:^|[=:\s])SKIP(?:$|[=:\s])' }).Count
    if ($ExitCode -eq 0 -and $passCount -eq 1 -and $skipCount -eq 0) { return 'PASS' }
    if ($ExitCode -eq 0 -and $passCount -eq 0 -and $skipCount -gt 0) { return 'NOT_RUN' }
    return 'FAIL'
}

function Invoke-Runner {
    param(
        [Parameter(Mandatory = $true)][string]$Binary,
        [Parameter(Mandatory = $true)][string]$BinarySha,
        [Parameter(Mandatory = $true)][string]$Commit,
        [Parameter(Mandatory = $true)][string]$Mode,
        [Parameter(Mandatory = $true)][string]$EvidenceRoot
    )

    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& (Join-Path $PSHOME 'powershell.exe') -NoLogo -NoProfile -ExecutionPolicy Bypass -File $script:Runner `
            -LatticedExecutable $Binary `
            -ExpectedBinarySha256 $BinarySha `
            -ExpectedSourceCommit $Commit `
            -SourceRepository $script:RepoRoot `
            -Mode $Mode `
            -SessionTimeoutSeconds 30 `
            -EvidenceRoot $EvidenceRoot 2>&1 | ForEach-Object { [string]$_ })
        $exitCode = [int]$LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previous
    }
    return [pscustomobject]@{
        ExitCode = $exitCode
        Lines = $output
        Classification = Get-GateClassification -ExitCode $exitCode -Lines $output
    }
}

if (-not (Test-Path -LiteralPath $script:Runner -PathType Leaf)) {
    throw 'TASK038_ACCEPT_TEST_RUNNER_MISSING'
}
if ((Get-GateClassification -ExitCode 0 -Lines @('SKIP: live prerequisite unavailable')) -cne 'NOT_RUN') {
    throw 'TASK038_ACCEPT_TEST_SKIP_NOT_RUN_REJECTED'
}
if ((Get-GateClassification -ExitCode 0 -Lines @($script:ExactMarker, $script:ExactMarker)) -cne 'FAIL') {
    throw 'TASK038_ACCEPT_TEST_DUPLICATE_MARKER_REJECTED'
}
if ((Get-GateClassification -ExitCode 0 -Lines @('ordinary output')) -cne 'FAIL') {
    throw 'TASK038_ACCEPT_TEST_MISSING_MARKER_REJECTED'
}
if ((Get-GateClassification -ExitCode 0 -Lines @($script:ExactMarker)) -cne 'PASS') {
    throw 'TASK038_ACCEPT_TEST_EXACT_MARKER_REJECTED'
}

$testRoot = Join-Path $script:RepoRoot ('target\task038-four-tool-acceptance-test\' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($testRoot) | Out-Null
$fakeBinary = Join-Path $testRoot 'latticed-fake.exe'
$fakeState = Join-Path $testRoot 'state.txt'
$source = @'
using System;
using System.IO;
using System.Text.RegularExpressions;

public static class Task038FakeMcp
{
    private static readonly string TaskRef = new string('a', 64);
    private static readonly string Ledger = new string('b', 64);
    private static readonly string Result = new string('c', 64);
    private static readonly string StatusJson = "{\"ledger_head_digest\":\"" + Ledger + "\",\"result_digest\":\"" + Result + "\",\"schema_version\":\"lattice.task.status.v1\",\"status\":\"COMPLETED\",\"task_ref\":\"" + TaskRef + "\",\"task_state\":\"COMPLETED\"}";

    private static string Response(int id, string result)
    {
        return "{\"jsonrpc\":\"2.0\",\"id\":" + id + ",\"result\":" + result + "}";
    }

    private static string ToolResult(int id, bool isError, string structured)
    {
        return Response(id, "{\"content\":[],\"isError\":" + (isError ? "true" : "false") + ",\"structuredContent\":" + structured + "}");
    }

    private static string OutputSchema()
    {
        return "{\"type\":\"object\",\"additionalProperties\":false,\"properties\":{" +
            "\"ledger_head_digest\":{\"type\":\"string\",\"pattern\":\"^[0-9a-f]{64}$\"}," +
            "\"result_digest\":{\"anyOf\":[{\"type\":\"string\",\"pattern\":\"^[0-9a-f]{64}$\"},{\"type\":\"null\"}]}," +
            "\"schema_version\":{\"type\":\"string\"}," +
            "\"status\":{\"type\":\"string\",\"enum\":[\"COMPLETED\",\"FAILED\",\"NOT_SUBMITTED\",\"RECONCILIATION_REQUIRED\"]}," +
            "\"task_ref\":{\"type\":\"string\",\"pattern\":\"^[0-9a-f]{64}$\"}," +
            "\"task_state\":{\"type\":\"string\",\"enum\":[\"AWAITING_EXECUTION_APPROVAL\",\"AWAITING_MERGE_APPROVAL\",\"BLOCKED\",\"CANCELLED\",\"COMPLETED\",\"DRAFT\",\"EXECUTING\",\"FAILED\",\"MERGING\",\"NOT_SUBMITTED\",\"PREPARING\",\"REJECTED\",\"REVIEWING\",\"STOPPING\",\"VERIFYING\"]}}," +
            "\"required\":[\"ledger_head_digest\",\"result_digest\",\"schema_version\",\"status\",\"task_ref\",\"task_state\"]}";
    }

    private static string Tools()
    {
        string empty = "{\"type\":\"object\",\"additionalProperties\":false,\"properties\":{}}";
        string submit = "{\"type\":\"object\",\"additionalProperties\":false,\"properties\":{\"client_request_id\":{\"type\":\"string\"},\"intent\":{\"type\":\"string\"}},\"required\":[\"client_request_id\",\"intent\"]}";
        string status = "{\"type\":\"object\",\"additionalProperties\":false,\"properties\":{\"task_ref\":{\"type\":\"string\"}},\"required\":[\"task_ref\"]}";
        string firstTwo = "{\"name\":\"lattice_delivery_run\",\"inputSchema\":" + empty + "},{\"name\":\"lattice_delivery_status\",\"inputSchema\":" + empty + "}";
        if (Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_TWO_TOOLS") == "1") return "[" + firstTwo + "]";
        return "[" + firstTwo + "," +
            "{\"name\":\"lattice_task_status\",\"inputSchema\":" + status + ",\"outputSchema\":" + OutputSchema() + "}," +
            "{\"name\":\"lattice_task_submit\",\"inputSchema\":" + submit + ",\"outputSchema\":" + OutputSchema() + "}]";
    }

    private static string RequestId(string line)
    {
        Match match = Regex.Match(line, "\\\"client_request_id\\\"\\s*:\\s*\\\"([^\\\"]+)\\\"");
        return match.Success ? match.Groups[1].Value : "";
    }

    public static int Main()
    {
        string state = Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_STATE");
        string line;
        while ((line = Console.ReadLine()) != null)
        {
            Match idMatch = Regex.Match(line, "\\\"id\\\"\\s*:\\s*(\\d+)");
            if (!idMatch.Success) continue;
            int id = Int32.Parse(idMatch.Groups[1].Value);
            if (line.Contains("\"method\":\"initialize\""))
            {
                Console.WriteLine(Response(id, "{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"serverInfo\":{\"name\":\"fake\",\"version\":\"1\"}}"));
            }
            else if (line.Contains("\"method\":\"server/discover\""))
            {
                Console.WriteLine(Response(id, "{\"resultType\":\"complete\"}"));
            }
            else if (line.Contains("\"method\":\"tools/list\""))
            {
                bool modern = line.Contains("io.modelcontextprotocol/protocolVersion");
                string result = modern
                    ? "{\"_meta\":{},\"cacheScope\":\"private\",\"resultType\":\"complete\",\"tools\":" + Tools() + ",\"ttlMs\":0}"
                    : "{\"tools\":" + Tools() + "}";
                Console.WriteLine(Response(id, result));
            }
            else if (line.Contains("\"name\":\"lattice_delivery_status\""))
            {
                Console.WriteLine(ToolResult(id, true, "{\"status\":\"ERROR\",\"code\":\"LATTICED_DATABASE_CONNECT_REJECTED\"}"));
            }
            else if (line.Contains("\"name\":\"lattice_task_submit\""))
            {
                string requestId = RequestId(line);
                if (String.IsNullOrEmpty(state)) return 41;
                if (!File.Exists(state)) File.WriteAllText(state, requestId);
                string stored = File.ReadAllText(state);
                if (stored != requestId)
                    Console.WriteLine(ToolResult(id, true, "{\"status\":\"ERROR\",\"code\":\"LATTICE_TASK_REQUEST_SUBSTITUTED\"}"));
                else
                    Console.WriteLine(ToolResult(id, false, StatusJson));
            }
            else if (line.Contains("\"name\":\"lattice_task_status\""))
            {
                if (line.Contains(new string('0', 64)) || String.IsNullOrEmpty(state) || !File.Exists(state))
                    Console.WriteLine(ToolResult(id, true, "{\"status\":\"ERROR\",\"code\":\"LATTICE_TASK_REFERENCE_REJECTED\"}"));
                else
                    Console.WriteLine(ToolResult(id, false, StatusJson));
            }
        }
        return 0;
    }
}
'@

Add-Type -TypeDefinition $source -Language CSharp -OutputAssembly $fakeBinary -OutputType ConsoleApplication
$binarySha = (Get-FileHash -LiteralPath $fakeBinary -Algorithm SHA256).Hash.ToLowerInvariant()
$commit = (& git -C $script:RepoRoot rev-parse HEAD).Trim().ToLowerInvariant()
if ($LASTEXITCODE -ne 0 -or $commit -notmatch '^[0-9a-f]{40}$') {
    throw 'TASK038_ACCEPT_TEST_SOURCE_COMMIT_REJECTED'
}

[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_STATE', $fakeState, 'Process')
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_TWO_TOOLS', $null, 'Process')
$positiveEvidence = Join-Path $testRoot 'positive-evidence'
$positive = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit $commit -Mode 'PROTOCOL_ONLY' -EvidenceRoot $positiveEvidence
if ($positive.Classification -cne 'PASS') {
    throw 'TASK038_ACCEPT_TEST_POSITIVE_REJECTED'
}
$positiveFinal = Get-Content -LiteralPath (Join-Path $positiveEvidence 'final.json') -Raw -Encoding UTF8 | ConvertFrom-Json
if (
    [string]$positiveFinal.status -cne 'PASS' -or
    [int]$positiveFinal.exact_pass_marker_count -ne 1 -or
    [bool]$positiveFinal.not_run -or
    [bool]$positiveFinal.delivery_run_invoked -or
    [bool]$positiveFinal.hermes_run_invoked -or
    -not [bool]$positiveFinal.process_cleanup_checked
) {
    throw 'TASK038_ACCEPT_TEST_POSITIVE_EVIDENCE_REJECTED'
}
$submitEvidence = Get-Content -LiteralPath (Join-Path $positiveEvidence '03-bounded-submit.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$durableEvidence = Get-Content -LiteralPath (Join-Path $positiveEvidence '04-durable-status.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$readOnlyEvidence = Get-Content -LiteralPath (Join-Path $positiveEvidence '02-read-only-status.json') -Raw -Encoding UTF8 | ConvertFrom-Json
if (
    ($submitEvidence.public_status | ConvertTo-Json -Compress) -cne ($durableEvidence.public_status | ConvertTo-Json -Compress) -or
    [int]$positiveFinal.sessions[2].process_id -eq [int]$positiveFinal.sessions[3].process_id -or
    -not [bool]$readOnlyEvidence.delivery_status_is_error -or
    -not [bool]$readOnlyEvidence.missing_task_status_is_error -or
    [bool]$submitEvidence.tool_is_error -or
    [bool]$submitEvidence.retry_tool_is_error -or
    -not [bool]$submitEvidence.different_key_tool_is_error
) {
    throw 'TASK038_ACCEPT_TEST_CROSS_PROCESS_REJECTED'
}
foreach ($session in @($positiveFinal.sessions)) {
    if (
        [int]$session.job_active_processes_after_exit -ne 0 -or
        -not [bool]$session.job_object_native_handle_assigned -or
        [string]$session.cleanup_identity -cne 'WINDOWS_JOB_OBJECT_AND_PROCESS_HANDLE'
    ) {
        throw 'TASK038_ACCEPT_TEST_NATIVE_CLEANUP_REJECTED'
    }
}

$shaEvidence = Join-Path $testRoot 'sha-negative-evidence'
$shaNegative = Invoke-Runner -Binary $fakeBinary -BinarySha ('0' * 64) -Commit $commit -Mode 'DISCOVERY_ONLY' -EvidenceRoot $shaEvidence
$shaFinal = Get-Content -LiteralPath (Join-Path $shaEvidence 'final.json') -Raw -Encoding UTF8 | ConvertFrom-Json
if (
    $shaNegative.Classification -cne 'FAIL' -or
    [string]$shaFinal.failure_code -cne 'TASK038_ACCEPT_BINARY_SHA_MISMATCH' -or
    @($shaFinal.sessions).Count -ne 0
) {
    throw 'TASK038_ACCEPT_TEST_BINARY_SHA_REJECTED'
}

[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_TWO_TOOLS', '1', 'Process')
$negativeEvidence = Join-Path $testRoot 'negative-evidence'
$negative = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit $commit -Mode 'DISCOVERY_ONLY' -EvidenceRoot $negativeEvidence
if ($negative.Classification -cne 'FAIL' -or @($negative.Lines | Where-Object { $_ -ceq $script:ExactMarker }).Count -ne 0) {
    throw 'TASK038_ACCEPT_TEST_NEGATIVE_MARKER_REJECTED'
}
$negativeFinal = Get-Content -LiteralPath (Join-Path $negativeEvidence 'final.json') -Raw -Encoding UTF8 | ConvertFrom-Json
if ([string]$negativeFinal.status -cne 'FAIL' -or [string]$negativeFinal.failure_code -cne 'TASK038_ACCEPT_TOOL_DISCOVERY_MISMATCH') {
    throw 'TASK038_ACCEPT_TEST_NEGATIVE_EVIDENCE_REJECTED'
}

[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_STATE', $null, 'Process')
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_TWO_TOOLS', $null, 'Process')
Write-Output 'TASK038_FOUR_TOOL_ACCEPTANCE_TEST=PASS'
Write-Output ('TEST_EVIDENCE_ROOT=' + $testRoot)
