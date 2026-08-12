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
if ($contract.task_type -isnot [string] -or [string]$contract.task_type -cne 'controlled_codex_canary') {
    throw 'TASK_CONTRACT_TYPE_REJECTED'
}
if ($null -eq $contract.parameters -or $contract.parameters -isnot [pscustomobject]) {
    throw 'TASK_CONTRACT_PARAMETERS_OBJECT_REQUIRED'
}
if (@($contract.parameters.PSObject.Properties).Count -ne 0) { throw 'TASK_CONTRACT_PARAMETERS_NOT_EMPTY' }

$projection = [ordered]@{
    contract_schema = 'lattice.task-contract.v1'
    contract_type = 'controlled_codex_canary'
    contract_file_sha256 = Get-BytesSha256 -Bytes $contractBytes
    mcp_tool = 'lattice_task_submit'
    intent = 'CONTROLLED_CODEX_CANARY'
    submit_fields = @('client_request_id', 'intent')
}
$projection | ConvertTo-Json -Compress -Depth 5
