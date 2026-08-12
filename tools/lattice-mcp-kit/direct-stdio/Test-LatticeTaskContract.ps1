[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$resolver = Join-Path $PSScriptRoot 'Resolve-LatticeTaskContract.ps1'
$harness = Join-Path $PSScriptRoot 'Invoke-LatticeTaskContractConformance.ps1'
$manifest = Join-Path $PSScriptRoot 'task-contract.conformance.v1.json'

function Assert-True {
    param([Parameter(Mandatory = $true)][bool]$Condition, [Parameter(Mandatory = $true)][string]$Message)
    if (-not $Condition) { throw ('ASSERTION_FAILED|' + $Message) }
}

$output = @(& $harness -ResolverPath $resolver -ConformanceFile $manifest)
Assert-True -Condition ($output.Count -eq 1) -Message 'harness emits exactly one summary'
$summaryText = [string]$output[0]
try { $summary = $summaryText | ConvertFrom-Json -ErrorAction Stop }
catch { throw 'ASSERTION_FAILED|harness summary is valid JSON' }

$expectedFields = @('manifest_sha256', 'case_count', 'accepted_count', 'rejected_count', 'resolver_invocation_count', 'registry_type_count', 'mapped_intent', 'result', 'first_failure')
$actualFields = @($summary.PSObject.Properties.Name)
Assert-True -Condition ($actualFields.Count -eq $expectedFields.Count) -Message 'summary field count'
for ($index = 0; $index -lt $expectedFields.Count; $index++) {
    Assert-True -Condition ([string]$actualFields[$index] -ceq $expectedFields[$index]) -Message ('summary field order ' + $index)
}

$expectedManifestSha256 = (Get-FileHash -LiteralPath $manifest -Algorithm SHA256).Hash.ToLowerInvariant()
Assert-True -Condition ([string]$summary.result -ceq 'PASS') -Message 'conformance PASS'
Assert-True -Condition ($null -eq $summary.first_failure) -Message 'no first failure'
Assert-True -Condition ([string]$summary.manifest_sha256 -ceq $expectedManifestSha256) -Message 'manifest SHA256'
Assert-True -Condition ([int]$summary.case_count -eq 16) -Message 'case count'
Assert-True -Condition ([int]$summary.accepted_count -eq 1) -Message 'accepted count'
Assert-True -Condition ([int]$summary.rejected_count -eq 15) -Message 'rejected count'
Assert-True -Condition ([int]$summary.resolver_invocation_count -eq [int]$summary.case_count) -Message 'one resolver invocation per case'
Assert-True -Condition ([int]$summary.registry_type_count -eq 1) -Message 'one registry type'
Assert-True -Condition ([string]$summary.mapped_intent -ceq 'CONTROLLED_CODEX_CANARY') -Message 'fixed mapped intent'

$summaryText
