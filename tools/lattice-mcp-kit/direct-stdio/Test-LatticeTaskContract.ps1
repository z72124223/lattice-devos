[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false)
$resolver = Join-Path $PSScriptRoot 'Resolve-LatticeTaskContract.ps1'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('lattice-task-contract-test-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $testRoot
$caseCount = 0

function Assert-True {
    param([Parameter(Mandatory = $true)][bool]$Condition, [Parameter(Mandatory = $true)][string]$Message)
    if (-not $Condition) { throw ('ASSERTION_FAILED|' + $Message) }
}

function Write-ContractCase {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Json,
        [switch]$WithBom
    )

    $path = Join-Path $testRoot ($Name + '.json')
    $bytes = $utf8.GetBytes($Json)
    if ($WithBom) {
        $bytes = [byte[]](@(0xef, 0xbb, 0xbf) + @($bytes))
    }
    [IO.File]::WriteAllBytes($path, $bytes)
    return $path
}

function Assert-ResolverRejects {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Json,
        [Parameter(Mandatory = $true)][string]$Failure,
        [switch]$WithBom
    )

    $script:caseCount++
    $path = Write-ContractCase -Name $Name -Json $Json -WithBom:$WithBom
    $actualFailure = $null
    try { $null = & $resolver -TaskContractFile $path }
    catch { $actualFailure = $_.Exception.Message }
    Assert-True -Condition (-not [string]::IsNullOrWhiteSpace([string]$actualFailure)) -Message ($Name + ' rejected')
    Assert-True -Condition ([string]$actualFailure -ceq $Failure) -Message ($Name + ' fixed failure classification')
}

$caseCount++
$positivePath = Write-ContractCase -Name 'controlled-canary' -Json '{"schema":"lattice.task-contract.v1","task_type":"controlled_codex_canary","parameters":{}}'
$projectionText = (& $resolver -TaskContractFile $positivePath | Out-String).Trim()
$projection = $projectionText | ConvertFrom-Json -ErrorAction Stop
$projectionNames = @($projection.PSObject.Properties.Name)
$expectedProjectionNames = @('contract_schema', 'contract_type', 'contract_file_sha256', 'mcp_tool', 'intent', 'submit_fields')
Assert-True -Condition ($projectionNames.Count -eq $expectedProjectionNames.Count) -Message 'projection field count'
for ($index = 0; $index -lt $expectedProjectionNames.Count; $index++) {
    Assert-True -Condition ($projectionNames[$index] -ceq $expectedProjectionNames[$index]) -Message ('projection field ' + $index)
}
Assert-True -Condition ([string]$projection.contract_schema -ceq 'lattice.task-contract.v1') -Message 'projection schema'
Assert-True -Condition ([string]$projection.contract_type -ceq 'controlled_codex_canary') -Message 'projection type'
Assert-True -Condition ([string]$projection.mcp_tool -ceq 'lattice_task_submit') -Message 'projection tool'
Assert-True -Condition ([string]$projection.intent -ceq 'CONTROLLED_CODEX_CANARY') -Message 'projection intent'
Assert-True -Condition ([string]$projection.contract_file_sha256 -ceq (Get-FileHash -LiteralPath $positivePath -Algorithm SHA256).Hash.ToLowerInvariant()) -Message 'projection contract hash'
Assert-True -Condition (@($projection.submit_fields).Count -eq 2) -Message 'closed submit field count'
Assert-True -Condition ([string]$projection.submit_fields[0] -ceq 'client_request_id' -and [string]$projection.submit_fields[1] -ceq 'intent') -Message 'closed submit fields'

Assert-ResolverRejects -Name 'wrong-schema' -Json '{"schema":"lattice.task-contract.v2","task_type":"controlled_codex_canary","parameters":{}}' -Failure 'TASK_CONTRACT_SCHEMA_REJECTED'
Assert-ResolverRejects -Name 'wrong-task-type' -Json '{"schema":"lattice.task-contract.v1","task_type":"arbitrary_task","parameters":{}}' -Failure 'TASK_CONTRACT_TYPE_REJECTED'
Assert-ResolverRejects -Name 'unknown-top-level' -Json '{"schema":"lattice.task-contract.v1","task_type":"controlled_codex_canary","parameters":{},"unknown":false}' -Failure 'TASK_CONTRACT_UNKNOWN_TOP_LEVEL_FIELD'
Assert-ResolverRejects -Name 'missing-top-level' -Json '{"schema":"lattice.task-contract.v1","task_type":"controlled_codex_canary"}' -Failure 'TASK_CONTRACT_MISSING_TOP_LEVEL_FIELD'
Assert-ResolverRejects -Name 'duplicate-top-level' -Json '{"schema":"lattice.task-contract.v1","schema":"lattice.task-contract.v1","task_type":"controlled_codex_canary","parameters":{}}' -Failure 'TASK_CONTRACT_DUPLICATE_TOP_LEVEL_FIELD'
Assert-ResolverRejects -Name 'nonempty-parameters' -Json '{"schema":"lattice.task-contract.v1","task_type":"controlled_codex_canary","parameters":{"shell":"blocked"}}' -Failure 'TASK_CONTRACT_PARAMETERS_NOT_EMPTY'
Assert-ResolverRejects -Name 'bom' -Json '{"schema":"lattice.task-contract.v1","task_type":"controlled_codex_canary","parameters":{}}' -Failure 'TASK_CONTRACT_BOM_REJECTED' -WithBom
Assert-ResolverRejects -Name 'invalid-json' -Json '{"schema":"lattice.task-contract.v1"' -Failure 'TASK_CONTRACT_JSON_REJECTED'

$dangerousFields = @('shell', 'command', 'sql', 'path', 'file_write', 'env', 'credential')
foreach ($field in $dangerousFields) {
    $json = '{"schema":"lattice.task-contract.v1","task_type":"controlled_codex_canary","parameters":{},"' + $field + '":"blocked"}'
    Assert-ResolverRejects -Name ('dangerous-' + $field) -Json $json -Failure 'TASK_CONTRACT_UNKNOWN_TOP_LEVEL_FIELD'
}

[ordered]@{
    result = 'PASS'
    cases = $caseCount
    accepted_contracts = 1
    rejected_contracts = ($caseCount - 1)
    registry_type_count = 1
    mapped_intent = 'CONTROLLED_CODEX_CANARY'
} | ConvertTo-Json
