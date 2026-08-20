[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$catalogScript = Join-Path $PSScriptRoot 'Get-LatticeMcpFailureCatalog.ps1'
$ledger = Join-Path (Split-Path -Parent $PSScriptRoot) 'WINDOW_LEDGER.jsonl'

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message
    )

    if (-not $Condition) { throw ('ASSERTION_FAILED|' + $Message) }
}

Assert-True -Condition (Test-Path -LiteralPath $catalogScript -PathType Leaf) -Message 'failure catalog script exists'

$parseErrors = $null
$null = [Management.Automation.Language.Parser]::ParseFile(
    $catalogScript,
    [ref]$null,
    [ref]$parseErrors
)
Assert-True -Condition ($parseErrors.Count -eq 0) -Message 'failure catalog AST parses'

$output = @(& $catalogScript)
Assert-True -Condition ($output.Count -eq 1 -and $output[0] -is [string]) -Message 'catalog emits one JSON document'
$catalogText = [string]$output[0]
try { $catalog = $catalogText | ConvertFrom-Json -ErrorAction Stop }
catch { throw 'ASSERTION_FAILED|catalog JSON parses' }

$expectedRootFields = @(
    'schema',
    'version',
    'ledger',
    'evidence_sets',
    'failure_code_count',
    'status_counts',
    'entries'
)
$actualRootFields = @($catalog.PSObject.Properties.Name)
Assert-True -Condition ($actualRootFields.Count -eq $expectedRootFields.Count) -Message 'catalog root field count'
for ($index = 0; $index -lt $expectedRootFields.Count; $index++) {
    Assert-True -Condition ([string]$actualRootFields[$index] -ceq $expectedRootFields[$index]) -Message ('catalog root field order ' + $index)
}

Assert-True -Condition ([string]$catalog.schema -ceq 'lattice.mcp-failure-catalog.v1') -Message 'catalog schema'
Assert-True -Condition ([int]$catalog.version -eq 1) -Message 'catalog version'

$expectedLedgerFields = @('path', 'sha256', 'record_count')
$actualLedgerFields = @($catalog.ledger.PSObject.Properties.Name)
Assert-True -Condition ($actualLedgerFields.Count -eq $expectedLedgerFields.Count) -Message 'ledger field count'
for ($index = 0; $index -lt $expectedLedgerFields.Count; $index++) {
    Assert-True -Condition ([string]$actualLedgerFields[$index] -ceq $expectedLedgerFields[$index]) -Message ('ledger field order ' + $index)
}
Assert-True -Condition ([string]$catalog.ledger.path -ceq '../WINDOW_LEDGER.jsonl') -Message 'relative ledger path'
Assert-True -Condition ([string]$catalog.ledger.sha256 -ceq (Get-FileHash -LiteralPath $ledger -Algorithm SHA256).Hash.ToLowerInvariant()) -Message 'ledger SHA256'
$ledgerRecordCount = @(Get-Content -LiteralPath $ledger -Encoding UTF8 | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }).Count
Assert-True -Condition ([int]$catalog.ledger.record_count -eq $ledgerRecordCount) -Message 'ledger record count'

$expectedProductionScripts = @(
    'Get-LatticeMcpFailureCatalog.ps1',
    'Get-LatticeTaskContractRegistry.ps1',
    'Invoke-LatticeFreshAcceptance.ps1',
    'Invoke-LatticeMcp.ps1',
    'Invoke-LatticeTaskContractConformance.ps1',
    'Resolve-LatticeTaskContract.ps1'
)
$expectedTestScripts = @(
    'Test-LatticeFreshAcceptance.ps1',
    'Test-LatticeTaskContract.ps1',
    'Test-StdinFrameEncoding.ps1',
    'Test-ToolCallTimeout.ps1'
)
Assert-True -Condition (@($catalog.evidence_sets.production_scripts).Count -eq $expectedProductionScripts.Count) -Message 'production evidence count'
Assert-True -Condition (@($catalog.evidence_sets.test_scripts).Count -eq $expectedTestScripts.Count) -Message 'test evidence count'
for ($index = 0; $index -lt $expectedProductionScripts.Count; $index++) {
    Assert-True -Condition ([string]$catalog.evidence_sets.production_scripts[$index] -ceq $expectedProductionScripts[$index]) -Message ('production evidence order ' + $index)
}
for ($index = 0; $index -lt $expectedTestScripts.Count; $index++) {
    Assert-True -Condition ([string]$catalog.evidence_sets.test_scripts[$index] -ceq $expectedTestScripts[$index]) -Message ('test evidence order ' + $index)
}
Assert-True -Condition (@($catalog.evidence_sets.documentation).Count -eq 1 -and [string]$catalog.evidence_sets.documentation[0] -ceq 'README.md') -Message 'documentation evidence'

$entries = @($catalog.entries)
Assert-True -Condition ($entries.Count -gt 0) -Message 'catalog has structured failure codes'
Assert-True -Condition ([int]$catalog.failure_code_count -eq $entries.Count) -Message 'failure code count'

$allowedStatuses = @('regression_tested', 'implementation_known', 'documented_only', 'recorded_only')
$seenCodes = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
$previousCode = $null
$observedCounts = @{
    regression_tested = 0
    implementation_known = 0
    documented_only = 0
    recorded_only = 0
}
foreach ($entry in $entries) {
    $expectedEntryFields = @('code', 'status', 'ledger_occurrences', 'production_evidence', 'test_evidence', 'documentation_evidence')
    $actualEntryFields = @($entry.PSObject.Properties.Name)
    Assert-True -Condition ($actualEntryFields.Count -eq $expectedEntryFields.Count) -Message ('entry field count ' + [string]$entry.code)
    for ($index = 0; $index -lt $expectedEntryFields.Count; $index++) {
        Assert-True -Condition ([string]$actualEntryFields[$index] -ceq $expectedEntryFields[$index]) -Message ('entry field order ' + [string]$entry.code + ' ' + $index)
    }

    $code = [string]$entry.code
    $status = [string]$entry.status
    Assert-True -Condition ($code -cmatch '^[A-Z][A-Z0-9_]{2,127}$') -Message ('failure code shape ' + $code)
    Assert-True -Condition ($seenCodes.Add($code)) -Message ('unique failure code ' + $code)
    if ($null -ne $previousCode) {
        Assert-True -Condition ([StringComparer]::Ordinal.Compare($previousCode, $code) -lt 0) -Message ('failure code order ' + $code)
    }
    $previousCode = $code
    Assert-True -Condition ($allowedStatuses -ccontains $status) -Message ('status value ' + $code)
    Assert-True -Condition ([int]$entry.ledger_occurrences -gt 0) -Message ('positive ledger occurrences ' + $code)

    foreach ($path in @($entry.production_evidence) + @($entry.test_evidence) + @($entry.documentation_evidence)) {
        Assert-True -Condition (-not [IO.Path]::IsPathRooted([string]$path)) -Message ('relative evidence path ' + $code)
        Assert-True -Condition (-not ([string]$path).Contains('..')) -Message ('bounded evidence path ' + $code)
    }

    switch ($status) {
        'regression_tested' {
            Assert-True -Condition (@($entry.test_evidence).Count -gt 0) -Message ('tested evidence ' + $code)
        }
        'implementation_known' {
            Assert-True -Condition (@($entry.test_evidence).Count -eq 0 -and @($entry.production_evidence).Count -gt 0) -Message ('implementation evidence ' + $code)
        }
        'documented_only' {
            Assert-True -Condition (@($entry.test_evidence).Count -eq 0 -and @($entry.production_evidence).Count -eq 0 -and @($entry.documentation_evidence).Count -gt 0) -Message ('documentation evidence ' + $code)
        }
        'recorded_only' {
            Assert-True -Condition (@($entry.test_evidence).Count -eq 0 -and @($entry.production_evidence).Count -eq 0 -and @($entry.documentation_evidence).Count -eq 0) -Message ('recorded-only evidence ' + $code)
        }
    }
    $observedCounts[$status]++
}

foreach ($status in $allowedStatuses) {
    Assert-True -Condition ([int]$catalog.status_counts.$status -eq [int]$observedCounts[$status]) -Message ('status count ' + $status)
}

$knownRecordedFailure = @($entries | Where-Object { [string]$_.code -ceq 'PREFLIGHT_FAILURE_PRE_REQUEST_RECORD_WRITE_ABORTED' })
Assert-True -Condition ($knownRecordedFailure.Count -eq 1) -Message 'known pre-request failure is cataloged'
Assert-True -Condition ([string]$knownRecordedFailure[0].status -ceq 'recorded_only') -Message 'known pre-request failure remains recorded-only'
Assert-True -Condition ([int]$knownRecordedFailure[0].ledger_occurrences -ge 3) -Message 'known pre-request failure occurrence count'

$knownTestedFailure = @($entries | Where-Object { [string]$_.code -ceq 'MCP_CLIENT_TIMEOUT' })
Assert-True -Condition ($knownTestedFailure.Count -eq 1) -Message 'embedded timeout failure is cataloged'
Assert-True -Condition ([string]$knownTestedFailure[0].status -ceq 'regression_tested') -Message 'timeout failure is classified as regression-tested'
Assert-True -Condition (@($knownTestedFailure[0].production_evidence) -ccontains 'Invoke-LatticeMcp.ps1') -Message 'timeout production evidence'
Assert-True -Condition (@($knownTestedFailure[0].test_evidence) -ccontains 'Test-ToolCallTimeout.ps1') -Message 'timeout test evidence'

$knownEmbeddedFailure = @($entries | Where-Object { [string]$_.code -ceq 'CODEX_SCHEMA_OUTPUT_EXISTS' })
Assert-True -Condition ($knownEmbeddedFailure.Count -eq 1) -Message 'embedded text receipt failure is cataloged'
Assert-True -Condition ([int]$knownEmbeddedFailure[0].ledger_occurrences -gt 0) -Message 'embedded text receipt occurrence count'

Assert-True -Condition (-not $catalogText.Contains($PSScriptRoot)) -Message 'catalog does not expose absolute workspace path'

$readme = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'README.md') -Raw -Encoding UTF8
foreach ($requiredText in @(
    '## Failure catalog',
    'Get-LatticeMcpFailureCatalog.ps1',
    'Test-LatticeMcpFailureCatalog.ps1',
    'regression_tested',
    'implementation_known',
    'documented_only',
    'recorded_only',
    'catalog test is intentionally excluded from test evidence'
)) {
    Assert-True -Condition ($readme.Contains($requiredText)) -Message ('README failure catalog text ' + $requiredText)
}

([ordered]@{
    schema = 'lattice.mcp-failure-catalog-test-summary.v1'
    result = 'PASS'
    ledger_record_count = [int]$catalog.ledger.record_count
    failure_code_count = $entries.Count
    status_counts = $catalog.status_counts
    known_recorded_failure = [string]$knownRecordedFailure[0].code
} | ConvertTo-Json -Compress -Depth 6)
