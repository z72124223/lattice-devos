[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ledgerPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'WINDOW_LEDGER.jsonl'
$productionScripts = @(
    'Get-LatticeMcpFailureCatalog.ps1',
    'Get-LatticeTaskContractRegistry.ps1',
    'Invoke-LatticeFreshAcceptance.ps1',
    'Invoke-LatticeMcp.ps1',
    'Invoke-LatticeTaskContractConformance.ps1',
    'Resolve-LatticeTaskContract.ps1'
)
$testScripts = @(
    'Test-LatticeFreshAcceptance.ps1',
    'Test-LatticeTaskContract.ps1',
    'Test-StdinFrameEncoding.ps1',
    'Test-ToolCallTimeout.ps1'
)
$documentation = @('README.md')
$failureCodePattern = '^[A-Z][A-Z0-9_]{2,127}$'
$failureFieldNames = @('error_code', 'failure', 'failure_code', 'first_failure')
$occurrences = [Collections.Generic.Dictionary[string, int]]::new([StringComparer]::Ordinal)

function Add-FailureCode {
    param([AllowNull()][object]$Value)

    if ($Value -isnot [string] -or [string]$Value -cnotmatch $failureCodePattern) { return }
    $code = [string]$Value
    if ($occurrences.ContainsKey($code)) {
        $occurrences[$code]++
    }
    else {
        $occurrences.Add($code, 1)
    }
}

function Read-StrictUtf8 {
    param([Parameter(Mandatory = $true)][string]$Path)

    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xef -and $bytes[1] -eq 0xbb -and $bytes[2] -eq 0xbf) {
        throw 'MCP_FAILURE_CATALOG_BOM_REJECTED'
    }
    try { return [Text.UTF8Encoding]::new($false, $true).GetString($bytes) }
    catch { throw 'MCP_FAILURE_CATALOG_UTF8_REJECTED' }
}

function Add-EmbeddedFailureCodes {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    $pattern = '(?m)(?:failure_code|first_failure|error_code|failure)\s*[:=]\s*([A-Z][A-Z0-9_]{2,127})'
    foreach ($match in [regex]::Matches($Text, $pattern)) {
        Add-FailureCode -Value $match.Groups[1].Value
    }
}

function Visit-FailureFields {
    param(
        [AllowNull()][object]$Node,
        [string[]]$Path = @()
    )

    if ($null -eq $Node) { return }
    if ($Node -is [string]) {
        Add-EmbeddedFailureCodes -Text ([string]$Node)
        return
    }
    if ($Node -is [Array]) {
        foreach ($item in $Node) { Visit-FailureFields -Node $item -Path $Path }
        return
    }
    if ($Node -isnot [pscustomobject]) { return }

    $parentName = if ($Path.Count -eq 0) { '' } else { [string]$Path[$Path.Count - 1] }
    foreach ($property in $Node.PSObject.Properties) {
        $name = [string]$property.Name
        $value = $property.Value
        if (($failureFieldNames -ccontains $name) -or ($name -ceq 'code' -and $parentName -ceq 'first_failure')) {
            Add-FailureCode -Value $value
        }
        Visit-FailureFields -Node $value -Path (@($Path) + $name)
    }
}

function Read-EvidenceSet {
    param([Parameter(Mandatory = $true)][string[]]$Names)

    $result = [ordered]@{}
    foreach ($name in $Names) {
        $path = Join-Path $PSScriptRoot $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw 'MCP_FAILURE_CATALOG_EVIDENCE_FILE_NOT_FOUND'
        }
        $result.Add($name, (Read-StrictUtf8 -Path $path))
    }
    $result
}

function Test-CodeMention {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text,
        [Parameter(Mandatory = $true)][string]$Code
    )

    $pattern = '(?<![A-Z0-9_])' + [regex]::Escape($Code) + '(?![A-Z0-9_])'
    return [regex]::IsMatch($Text, $pattern, [Text.RegularExpressions.RegexOptions]::CultureInvariant)
}

if (-not (Test-Path -LiteralPath $ledgerPath -PathType Leaf)) {
    throw 'MCP_FAILURE_CATALOG_LEDGER_NOT_FOUND'
}
$ledgerText = Read-StrictUtf8 -Path $ledgerPath
$ledgerLines = @($ledgerText -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
foreach ($line in $ledgerLines) {
    try { $record = $line | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'MCP_FAILURE_CATALOG_LEDGER_JSON_REJECTED' }
    Visit-FailureFields -Node $record
}
if ($occurrences.Count -eq 0) { throw 'MCP_FAILURE_CATALOG_EMPTY' }

$productionText = Read-EvidenceSet -Names $productionScripts
$testText = Read-EvidenceSet -Names $testScripts
$documentationText = Read-EvidenceSet -Names $documentation

[string[]]$codes = @($occurrences.Keys)
[Array]::Sort($codes, [StringComparer]::Ordinal)
$entries = @()
$statusCounts = [ordered]@{
    regression_tested = 0
    implementation_known = 0
    documented_only = 0
    recorded_only = 0
}

foreach ($code in $codes) {
    $productionEvidence = @()
    foreach ($name in $productionScripts) {
        if (Test-CodeMention -Text ([string]$productionText[$name]) -Code $code) { $productionEvidence += $name }
    }
    $testEvidence = @()
    foreach ($name in $testScripts) {
        if (Test-CodeMention -Text ([string]$testText[$name]) -Code $code) { $testEvidence += $name }
    }
    $documentationEvidence = @()
    foreach ($name in $documentation) {
        if (Test-CodeMention -Text ([string]$documentationText[$name]) -Code $code) { $documentationEvidence += $name }
    }

    $status = if ($testEvidence.Count -gt 0) {
        'regression_tested'
    }
    elseif ($productionEvidence.Count -gt 0) {
        'implementation_known'
    }
    elseif ($documentationEvidence.Count -gt 0) {
        'documented_only'
    }
    else {
        'recorded_only'
    }
    $statusCounts[$status]++
    $entries += [ordered]@{
        code = $code
        status = $status
        ledger_occurrences = [int]$occurrences[$code]
        production_evidence = @($productionEvidence)
        test_evidence = @($testEvidence)
        documentation_evidence = @($documentationEvidence)
    }
}

([ordered]@{
    schema = 'lattice.mcp-failure-catalog.v1'
    version = 1
    ledger = [ordered]@{
        path = '../WINDOW_LEDGER.jsonl'
        sha256 = (Get-FileHash -LiteralPath $ledgerPath -Algorithm SHA256).Hash.ToLowerInvariant()
        record_count = $ledgerLines.Count
    }
    evidence_sets = [ordered]@{
        production_scripts = @($productionScripts)
        test_scripts = @($testScripts)
        documentation = @($documentation)
    }
    failure_code_count = $entries.Count
    status_counts = $statusCounts
    entries = @($entries)
} | ConvertTo-Json -Compress -Depth 8)
