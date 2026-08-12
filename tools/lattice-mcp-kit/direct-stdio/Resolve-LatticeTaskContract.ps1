[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$TaskContractFile
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:Utf8Strict = [Text.UTF8Encoding]::new($false, $true)

function Get-BytesSha256 {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)

    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha256.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha256.Dispose()
    }
}

function Get-TopLevelJsonPropertyNames {
    param([Parameter(Mandatory = $true)][string]$Text)

    $names = [Collections.Generic.List[string]]::new()
    $objectDepth = 0
    $arrayDepth = 0
    for ($index = 0; $index -lt $Text.Length; $index++) {
        $character = $Text[$index]
        if ($character -eq '"') {
            $stringStart = $index
            $index++
            while ($index -lt $Text.Length) {
                if ($Text[$index] -eq '\\') {
                    $index += 2
                    continue
                }
                if ($Text[$index] -eq '"') { break }
                $index++
            }
            if ($index -ge $Text.Length) { throw 'TASK_CONTRACT_JSON_REJECTED' }

            if ($objectDepth -eq 1 -and $arrayDepth -eq 0) {
                $lookAhead = $index + 1
                while ($lookAhead -lt $Text.Length -and [char]::IsWhiteSpace($Text[$lookAhead])) { $lookAhead++ }
                if ($lookAhead -lt $Text.Length -and $Text[$lookAhead] -eq ':') {
                    $token = $Text.Substring($stringStart, ($index - $stringStart + 1))
                    try { $name = $token | ConvertFrom-Json -ErrorAction Stop }
                    catch { throw 'TASK_CONTRACT_JSON_REJECTED' }
                    if ($name -isnot [string]) { throw 'TASK_CONTRACT_JSON_REJECTED' }
                    $names.Add([string]$name)
                }
            }
            continue
        }
        switch ($character) {
            '{' { $objectDepth++ }
            '}' { $objectDepth-- }
            '[' { $arrayDepth++ }
            ']' { $arrayDepth-- }
        }
    }
    return @($names)
}

function Assert-ExactPropertyOrder {
    param(
        [Parameter(Mandatory = $true)]$Object,
        [Parameter(Mandatory = $true)][string[]]$Expected,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    if ($null -eq $Object -or $Object -isnot [pscustomobject]) { throw $Failure }
    $actual = @($Object.PSObject.Properties.Name)
    if ($actual.Count -ne $Expected.Count) { throw $Failure }
    for ($index = 0; $index -lt $Expected.Count; $index++) {
        if ([string]$actual[$index] -cne $Expected[$index]) { throw $Failure }
    }
}

$registryScript = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'Get-LatticeTaskContractRegistry.ps1'))
if (-not (Test-Path -LiteralPath $registryScript -PathType Leaf)) { throw 'TASK_CONTRACT_REGISTRY_NOT_FOUND' }
$registryOutput = [Collections.Generic.List[object]]::new()
try {
    & $registryScript | ForEach-Object { $registryOutput.Add($_) }
}
catch {
    throw 'TASK_CONTRACT_REGISTRY_INVOCATION_FAILED'
}
if ($registryOutput.Count -ne 1 -or $registryOutput[0] -isnot [string]) {
    throw 'TASK_CONTRACT_REGISTRY_OUTPUT_REJECTED'
}
$registryText = [string]$registryOutput[0]
if ([string]::IsNullOrWhiteSpace($registryText)) { throw 'TASK_CONTRACT_REGISTRY_OUTPUT_REJECTED' }
try { $registry = $registryText | ConvertFrom-Json -ErrorAction Stop }
catch { throw 'TASK_CONTRACT_REGISTRY_JSON_REJECTED' }
Assert-ExactPropertyOrder -Object $registry -Expected @('schema', 'entries') -Failure 'TASK_CONTRACT_REGISTRY_ROOT_REJECTED'
if ($registry.schema -isnot [string] -or [string]$registry.schema -cne 'lattice.task-contract-registry.v1') {
    throw 'TASK_CONTRACT_REGISTRY_SCHEMA_REJECTED'
}
if ($registry.entries -isnot [Array]) { throw 'TASK_CONTRACT_REGISTRY_ENTRIES_REJECTED' }
$registryEntries = @($registry.entries)
if ($registryEntries.Count -lt 1 -or $registryEntries.Count -gt 32) {
    throw 'TASK_CONTRACT_REGISTRY_ENTRY_COUNT_REJECTED'
}

$seenContractTypes = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
$canonicalEntries = [Collections.Generic.List[object]]::new()
foreach ($entry in $registryEntries) {
    $entryFields = @('contract_schema', 'contract_type', 'parameter_fields', 'mcp_tool', 'intent', 'submit_fields')
    Assert-ExactPropertyOrder -Object $entry -Expected $entryFields -Failure 'TASK_CONTRACT_REGISTRY_ENTRY_REJECTED'
    if ($entry.contract_schema -isnot [string] -or [string]$entry.contract_schema -cne 'lattice.task-contract.v1') {
        throw 'TASK_CONTRACT_REGISTRY_ENTRY_REJECTED'
    }
    if ($entry.contract_type -isnot [string] -or [string]$entry.contract_type -cnotmatch '^[a-z][a-z0-9_]{0,63}$' -or
        -not $seenContractTypes.Add([string]$entry.contract_type)) {
        throw 'TASK_CONTRACT_REGISTRY_ENTRY_REJECTED'
    }
    if ($entry.parameter_fields -isnot [Array]) { throw 'TASK_CONTRACT_REGISTRY_ENTRY_REJECTED' }
    $parameterFields = @($entry.parameter_fields)
    if ($parameterFields.Count -gt 32) { throw 'TASK_CONTRACT_REGISTRY_ENTRY_REJECTED' }
    $seenParameterFields = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($parameterField in $parameterFields) {
        if ($parameterField -isnot [string] -or [string]$parameterField -cnotmatch '^[a-z][a-z0-9_]{0,63}$' -or
            -not $seenParameterFields.Add([string]$parameterField)) {
            throw 'TASK_CONTRACT_REGISTRY_ENTRY_REJECTED'
        }
    }
    if ($entry.mcp_tool -isnot [string] -or [string]$entry.mcp_tool -cne 'lattice_task_submit') {
        throw 'TASK_CONTRACT_REGISTRY_ENTRY_REJECTED'
    }
    if ($entry.intent -isnot [string] -or [string]$entry.intent -cnotmatch '^[A-Z][A-Z0-9_]{0,95}$') {
        throw 'TASK_CONTRACT_REGISTRY_ENTRY_REJECTED'
    }
    if ($entry.submit_fields -isnot [Array]) { throw 'TASK_CONTRACT_REGISTRY_ENTRY_REJECTED' }
    $submitFields = @($entry.submit_fields)
    if ($submitFields.Count -ne 2 -or [string]$submitFields[0] -cne 'client_request_id' -or
        [string]$submitFields[1] -cne 'intent') {
        throw 'TASK_CONTRACT_REGISTRY_ENTRY_REJECTED'
    }
    $canonicalEntries.Add([ordered]@{
        contract_schema = [string]$entry.contract_schema
        contract_type = [string]$entry.contract_type
        parameter_fields = @($parameterFields)
        mcp_tool = [string]$entry.mcp_tool
        intent = [string]$entry.intent
        submit_fields = @($submitFields)
    })
}
$canonicalRegistryText = [ordered]@{
    schema = 'lattice.task-contract-registry.v1'
    entries = @($canonicalEntries)
} | ConvertTo-Json -Compress -Depth 5
if ($registryText -cne $canonicalRegistryText) { throw 'TASK_CONTRACT_REGISTRY_NORMALIZATION_REJECTED' }

if (-not [IO.Path]::IsPathRooted($TaskContractFile)) { throw 'TASK_CONTRACT_PATH_NOT_ABSOLUTE' }
try { $contractPath = [IO.Path]::GetFullPath($TaskContractFile) }
catch { throw 'TASK_CONTRACT_PATH_REJECTED' }
if (-not (Test-Path -LiteralPath $contractPath -PathType Leaf)) { throw 'TASK_CONTRACT_FILE_NOT_FOUND' }

try { $contractBytes = [IO.File]::ReadAllBytes($contractPath) }
catch { throw 'TASK_CONTRACT_FILE_READ_FAILED' }
if ($contractBytes.Length -ge 3 -and $contractBytes[0] -eq 0xef -and $contractBytes[1] -eq 0xbb -and $contractBytes[2] -eq 0xbf) {
    throw 'TASK_CONTRACT_BOM_REJECTED'
}
try { $contractText = $script:Utf8Strict.GetString($contractBytes) }
catch { throw 'TASK_CONTRACT_UTF8_REJECTED' }
try { $contract = $contractText | ConvertFrom-Json -ErrorAction Stop }
catch { throw 'TASK_CONTRACT_JSON_REJECTED' }
if ($null -eq $contract -or $contract -isnot [pscustomobject]) { throw 'TASK_CONTRACT_TOP_LEVEL_OBJECT_REQUIRED' }

$propertyNames = @(Get-TopLevelJsonPropertyNames -Text $contractText)
$seenNames = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
foreach ($propertyName in $propertyNames) {
    if (-not $seenNames.Add($propertyName)) { throw 'TASK_CONTRACT_DUPLICATE_TOP_LEVEL_FIELD' }
}

$expectedNames = @('schema', 'task_type', 'parameters')
foreach ($propertyName in $propertyNames) {
    if ($expectedNames -cnotcontains $propertyName) { throw 'TASK_CONTRACT_UNKNOWN_TOP_LEVEL_FIELD' }
}
foreach ($expectedName in $expectedNames) {
    if ($propertyNames -cnotcontains $expectedName) { throw 'TASK_CONTRACT_MISSING_TOP_LEVEL_FIELD' }
}
if ($propertyNames.Count -ne $expectedNames.Count) { throw 'TASK_CONTRACT_TOP_LEVEL_FIELDS_REJECTED' }

if ($contract.schema -isnot [string] -or [string]$contract.schema -cne 'lattice.task-contract.v1') {
    throw 'TASK_CONTRACT_SCHEMA_REJECTED'
}
if ($contract.task_type -isnot [string]) {
    throw 'TASK_CONTRACT_TYPE_REJECTED'
}
$matchingEntries = @($registryEntries | Where-Object { [string]$_.contract_type -ceq [string]$contract.task_type })
if ($matchingEntries.Count -ne 1) { throw 'TASK_CONTRACT_TYPE_REJECTED' }
$selectedEntry = $matchingEntries[0]
if ($null -eq $contract.parameters -or $contract.parameters -isnot [pscustomobject]) {
    throw 'TASK_CONTRACT_PARAMETERS_OBJECT_REQUIRED'
}
if ([string]$selectedEntry.contract_type -ceq 'controlled_codex_canary') {
    if (@($selectedEntry.parameter_fields).Count -ne 0) { throw 'TASK_CONTRACT_REGISTRY_ENTRY_REJECTED' }
    if (@($contract.parameters.PSObject.Properties).Count -ne 0) { throw 'TASK_CONTRACT_PARAMETERS_NOT_EMPTY' }
}
else {
    throw 'TASK_CONTRACT_TYPE_REJECTED'
}

$projection = [ordered]@{
    contract_schema = [string]$selectedEntry.contract_schema
    contract_type = [string]$selectedEntry.contract_type
    contract_file_sha256 = Get-BytesSha256 -Bytes $contractBytes
    mcp_tool = [string]$selectedEntry.mcp_tool
    intent = [string]$selectedEntry.intent
    submit_fields = @($selectedEntry.submit_fields)
}
$projection | ConvertTo-Json -Compress -Depth 5
