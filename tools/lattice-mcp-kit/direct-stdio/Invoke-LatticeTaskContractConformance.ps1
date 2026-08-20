[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ResolverPath,

    [Parameter(Mandatory = $true)]
    [string]$ConformanceFile
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:Utf8Strict = [Text.UTF8Encoding]::new($false, $true)
$script:Utf8 = [Text.UTF8Encoding]::new($false)

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

function Assert-NoDuplicateJsonObjectFields {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$JsonFailure,
        [Parameter(Mandatory = $true)][string]$DuplicateFailure
    )

    $scopes = [Collections.ArrayList]::new()
    for ($index = 0; $index -lt $Text.Length; $index++) {
        $character = $Text[$index]
        if ($character -eq '"') {
            $stringStart = $index
            $index++
            while ($index -lt $Text.Length) {
                if ($Text[$index] -eq [char]92) {
                    $index += 2
                    continue
                }
                if ($Text[$index] -eq '"') { break }
                $index++
            }
            if ($index -ge $Text.Length) { throw $JsonFailure }
            $lookAhead = $index + 1
            while ($lookAhead -lt $Text.Length -and [char]::IsWhiteSpace($Text[$lookAhead])) { $lookAhead++ }
            if ($lookAhead -lt $Text.Length -and $Text[$lookAhead] -eq ':') {
                if ($scopes.Count -eq 0) { throw $JsonFailure }
                $token = $Text.Substring($stringStart, ($index - $stringStart + 1))
                try { $name = $token | ConvertFrom-Json -ErrorAction Stop }
                catch { throw $JsonFailure }
                if ($name -isnot [string]) { throw $JsonFailure }
                $scope = $scopes[$scopes.Count - 1]
                if (-not $scope.Add([string]$name)) { throw $DuplicateFailure }
            }
            continue
        }
        if ($character -eq '{') {
            $null = $scopes.Add([Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal))
        }
        elseif ($character -eq '}') {
            if ($scopes.Count -eq 0) { throw $JsonFailure }
            $scopes.RemoveAt($scopes.Count - 1)
        }
    }
    if ($scopes.Count -ne 0) { throw $JsonFailure }
}

$registrySha256 = $null
$manifestSha256 = $null
$caseCount = 0
$acceptedCount = 0
$rejectedCount = 0
$resolverInvocationCount = 0
$registryTypeCount = 0
$coveredTypeCount = 0
$registryCoverage = $false
$mappedIntent = $null
$result = 'FAIL'
$firstFailure = $null

try {
    if (-not [IO.Path]::IsPathRooted($ResolverPath)) { throw 'CONFORMANCE_RESOLVER_PATH_NOT_ABSOLUTE' }
    if (-not [IO.Path]::IsPathRooted($ConformanceFile)) { throw 'CONFORMANCE_MANIFEST_PATH_NOT_ABSOLUTE' }
    try { $resolver = [IO.Path]::GetFullPath($ResolverPath) }
    catch { throw 'CONFORMANCE_RESOLVER_PATH_REJECTED' }
    try { $manifestPath = [IO.Path]::GetFullPath($ConformanceFile) }
    catch { throw 'CONFORMANCE_MANIFEST_PATH_REJECTED' }
    if (-not (Test-Path -LiteralPath $resolver -PathType Leaf)) { throw 'CONFORMANCE_RESOLVER_FILE_NOT_FOUND' }
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { throw 'CONFORMANCE_MANIFEST_FILE_NOT_FOUND' }

    $registryScript = [IO.Path]::GetFullPath((Join-Path (Split-Path -Parent $resolver) 'Get-LatticeTaskContractRegistry.ps1'))
    if (-not (Test-Path -LiteralPath $registryScript -PathType Leaf)) { throw 'CONFORMANCE_REGISTRY_FILE_NOT_FOUND' }
    $registryOutput = [Collections.Generic.List[object]]::new()
    try {
        & $registryScript | ForEach-Object { $registryOutput.Add($_) }
    }
    catch {
        throw 'CONFORMANCE_REGISTRY_INVOCATION_FAILED'
    }
    if ($registryOutput.Count -ne 1 -or $registryOutput[0] -isnot [string]) {
        throw 'CONFORMANCE_REGISTRY_OUTPUT_REJECTED'
    }
    $registryText = [string]$registryOutput[0]
    if ([string]::IsNullOrWhiteSpace($registryText)) { throw 'CONFORMANCE_REGISTRY_OUTPUT_REJECTED' }
    $registrySha256 = Get-BytesSha256 -Bytes $script:Utf8.GetBytes($registryText)
    try { $registry = $registryText | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'CONFORMANCE_REGISTRY_JSON_REJECTED' }
    Assert-NoDuplicateJsonObjectFields -Text $registryText `
        -JsonFailure 'CONFORMANCE_REGISTRY_JSON_REJECTED' `
        -DuplicateFailure 'CONFORMANCE_REGISTRY_DUPLICATE_FIELD'
    Assert-ExactPropertyOrder -Object $registry -Expected @('schema', 'entries') -Failure 'CONFORMANCE_REGISTRY_ROOT_REJECTED'
    if ($registry.schema -isnot [string] -or [string]$registry.schema -cne 'lattice.task-contract-registry.v1') {
        throw 'CONFORMANCE_REGISTRY_SCHEMA_REJECTED'
    }
    if ($registry.entries -isnot [Array]) { throw 'CONFORMANCE_REGISTRY_ENTRIES_REJECTED' }
    $registryEntries = @($registry.entries)
    if ($registryEntries.Count -lt 1 -or $registryEntries.Count -gt 32) {
        throw 'CONFORMANCE_REGISTRY_ENTRY_COUNT_REJECTED'
    }
    $registryTypes = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $registryByType = [Collections.Generic.Dictionary[string, object]]::new([StringComparer]::Ordinal)
    $canonicalEntries = [Collections.Generic.List[object]]::new()
    foreach ($entry in $registryEntries) {
        $entryFields = @('contract_schema', 'contract_type', 'parameter_fields', 'mcp_tool', 'intent', 'submit_fields')
        Assert-ExactPropertyOrder -Object $entry -Expected $entryFields -Failure 'CONFORMANCE_REGISTRY_ENTRY_REJECTED'
        if ($entry.contract_schema -isnot [string] -or [string]$entry.contract_schema -cne 'lattice.task-contract.v1') {
            throw 'CONFORMANCE_REGISTRY_ENTRY_REJECTED'
        }
        if ($entry.contract_type -isnot [string] -or [string]$entry.contract_type -cnotmatch '^[a-z][a-z0-9_]{0,63}$' -or
            -not $registryTypes.Add([string]$entry.contract_type)) {
            throw 'CONFORMANCE_REGISTRY_ENTRY_REJECTED'
        }
        if ($entry.parameter_fields -isnot [Array]) { throw 'CONFORMANCE_REGISTRY_ENTRY_REJECTED' }
        $parameterFields = @($entry.parameter_fields)
        if ($parameterFields.Count -gt 32) { throw 'CONFORMANCE_REGISTRY_ENTRY_REJECTED' }
        $seenParameterFields = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        foreach ($parameterField in $parameterFields) {
            if ($parameterField -isnot [string] -or [string]$parameterField -cnotmatch '^[a-z][a-z0-9_]{0,63}$' -or
                -not $seenParameterFields.Add([string]$parameterField)) {
                throw 'CONFORMANCE_REGISTRY_ENTRY_REJECTED'
            }
        }
        if ($entry.mcp_tool -isnot [string] -or [string]$entry.mcp_tool -cne 'lattice_task_submit') {
            throw 'CONFORMANCE_REGISTRY_ENTRY_REJECTED'
        }
        if ($entry.intent -isnot [string] -or [string]$entry.intent -cnotmatch '^[A-Z][A-Z0-9_]{0,95}$') {
            throw 'CONFORMANCE_REGISTRY_ENTRY_REJECTED'
        }
        if ($entry.submit_fields -isnot [Array]) { throw 'CONFORMANCE_REGISTRY_ENTRY_REJECTED' }
        $submitFields = @($entry.submit_fields)
        if ($submitFields.Count -ne 2 -or [string]$submitFields[0] -cne 'client_request_id' -or
            [string]$submitFields[1] -cne 'intent') {
            throw 'CONFORMANCE_REGISTRY_ENTRY_REJECTED'
        }
        $registryByType.Add([string]$entry.contract_type, $entry)
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
    if ($registryText -cne $canonicalRegistryText) { throw 'CONFORMANCE_REGISTRY_NORMALIZATION_REJECTED' }
    $registryTypeCount = $registryEntries.Count
    if ($registryByType.ContainsKey('controlled_codex_canary')) {
        $mappedIntent = [string]$registryByType['controlled_codex_canary'].intent
    }

    try { $manifestBytes = [IO.File]::ReadAllBytes($manifestPath) }
    catch { throw 'CONFORMANCE_MANIFEST_READ_FAILED' }
    $manifestSha256 = Get-BytesSha256 -Bytes $manifestBytes
    if ($manifestBytes.Length -ge 3 -and $manifestBytes[0] -eq 0xef -and $manifestBytes[1] -eq 0xbb -and $manifestBytes[2] -eq 0xbf) {
        throw 'CONFORMANCE_MANIFEST_BOM_REJECTED'
    }
    try { $manifestText = $script:Utf8Strict.GetString($manifestBytes) }
    catch { throw 'CONFORMANCE_MANIFEST_UTF8_REJECTED' }
    try { $manifest = $manifestText | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'CONFORMANCE_MANIFEST_JSON_REJECTED' }
    Assert-NoDuplicateJsonObjectFields -Text $manifestText `
        -JsonFailure 'CONFORMANCE_MANIFEST_JSON_REJECTED' `
        -DuplicateFailure 'CONFORMANCE_MANIFEST_DUPLICATE_FIELD'
    Assert-ExactPropertyOrder -Object $manifest -Expected @('schema', 'version', 'registry', 'cases') -Failure 'CONFORMANCE_MANIFEST_TOP_LEVEL_SCHEMA_REJECTED'
    if ($manifest.schema -isnot [string] -or [string]$manifest.schema -cne 'lattice.task-contract-conformance.v1') {
        throw 'CONFORMANCE_MANIFEST_SCHEMA_REJECTED'
    }
    if (($manifest.version -isnot [int] -and $manifest.version -isnot [long]) -or [long]$manifest.version -ne 1) {
        throw 'CONFORMANCE_MANIFEST_VERSION_REJECTED'
    }
    Assert-ExactPropertyOrder -Object $manifest.registry -Expected @('type_count', 'mapped_intent') -Failure 'CONFORMANCE_MANIFEST_REGISTRY_REJECTED'
    if (($manifest.registry.type_count -isnot [int] -and $manifest.registry.type_count -isnot [long]) -or
        [long]$manifest.registry.type_count -ne $registryTypeCount -or
        $manifest.registry.mapped_intent -isnot [string] -or
        [string]$manifest.registry.mapped_intent -cne [string]$mappedIntent) {
        throw 'CONFORMANCE_MANIFEST_REGISTRY_REJECTED'
    }

    if ($manifest.cases -isnot [Array]) { throw 'CONFORMANCE_MANIFEST_CASES_REJECTED' }
    $cases = @($manifest.cases)
    $caseCount = $cases.Count
    if ($caseCount -lt 16) { throw 'CONFORMANCE_MANIFEST_CASE_COUNT_REJECTED' }
    $seenCaseIds = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $positiveTypes = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $testRoot = Join-Path ([IO.Path]::GetTempPath()) ('lattice-task-contract-conformance-' + [Guid]::NewGuid().ToString('N'))
    $null = New-Item -ItemType Directory -Path $testRoot

    for ($caseIndex = 0; $caseIndex -lt $cases.Count; $caseIndex++) {
        $case = $cases[$caseIndex]
        Assert-ExactPropertyOrder -Object $case -Expected @('id', 'fixture', 'encoding', 'expected') -Failure 'CONFORMANCE_MANIFEST_CASE_SCHEMA_REJECTED'
        if ($case.id -isnot [string] -or [string]$case.id -cnotmatch '^[a-z0-9][a-z0-9-]{0,63}$' -or
            -not $seenCaseIds.Add([string]$case.id)) {
            throw 'CONFORMANCE_MANIFEST_CASE_ID_REJECTED'
        }
        if ($case.fixture -isnot [string]) { throw 'CONFORMANCE_MANIFEST_FIXTURE_REJECTED' }
        if ($case.encoding -isnot [string] -or @('utf8', 'utf8-bom') -cnotcontains [string]$case.encoding) {
            throw 'CONFORMANCE_MANIFEST_ENCODING_REJECTED'
        }
        if ($null -eq $case.expected -or $case.expected -isnot [pscustomobject]) {
            throw 'CONFORMANCE_MANIFEST_EXPECTATION_REJECTED'
        }

        $expectedResult = [string]$case.expected.result
        $positiveType = $null
        if ($expectedResult -ceq 'accepted') {
            Assert-ExactPropertyOrder -Object $case.expected -Expected @('result', 'mcp_tool') -Failure 'CONFORMANCE_MANIFEST_EXPECTATION_REJECTED'
            if ($case.expected.mcp_tool -isnot [string] -or [string]$case.expected.mcp_tool -cnotmatch '^[a-z][a-z0-9_]{0,63}$') {
                throw 'CONFORMANCE_MANIFEST_EXPECTATION_REJECTED'
            }
            try { $positiveContract = [string]$case.fixture | ConvertFrom-Json -ErrorAction Stop }
            catch { throw 'CONFORMANCE_POSITIVE_FIXTURE_REJECTED' }
            if ($null -eq $positiveContract -or $positiveContract -isnot [pscustomobject] -or
                $positiveContract.task_type -isnot [string] -or
                [string]$positiveContract.task_type -cnotmatch '^[a-z][a-z0-9_]{0,63}$') {
                throw 'CONFORMANCE_POSITIVE_FIXTURE_REJECTED'
            }
            $positiveType = [string]$positiveContract.task_type
            if (-not $registryByType.ContainsKey($positiveType)) { throw 'CONFORMANCE_POSITIVE_UNKNOWN_TYPE' }
            if (-not $positiveTypes.Add($positiveType)) { throw 'CONFORMANCE_POSITIVE_DUPLICATE_TYPE' }
        }
        elseif ($expectedResult -ceq 'rejected') {
            Assert-ExactPropertyOrder -Object $case.expected -Expected @('result', 'rejection_code') -Failure 'CONFORMANCE_MANIFEST_EXPECTATION_REJECTED'
            if ($case.expected.rejection_code -isnot [string] -or [string]$case.expected.rejection_code -cnotmatch '^TASK_CONTRACT_[A-Z0-9_]{1,96}$') {
                throw 'CONFORMANCE_MANIFEST_EXPECTATION_REJECTED'
            }
        }
        else {
            throw 'CONFORMANCE_MANIFEST_EXPECTATION_REJECTED'
        }

        $caseRoot = Join-Path $testRoot ('case-' + $caseIndex.ToString('00') + '-' + [Guid]::NewGuid().ToString('N'))
        $null = New-Item -ItemType Directory -Path $caseRoot
        $fixturePath = Join-Path $caseRoot 'task-contract.json'
        $fixtureBytes = $script:Utf8.GetBytes([string]$case.fixture)
        if ([string]$case.encoding -ceq 'utf8-bom') {
            $fixtureBytes = [byte[]](@(0xef, 0xbb, 0xbf) + @($fixtureBytes))
        }
        [IO.File]::WriteAllBytes($fixturePath, $fixtureBytes)

        $resolverOutput = [Collections.Generic.List[object]]::new()
        $resolverFailure = $null
        $resolverInvocationCount++
        try {
            & $resolver -TaskContractFile $fixturePath | ForEach-Object { $resolverOutput.Add($_) }
        }
        catch {
            $resolverFailure = $_.Exception.Message
        }

        if ($expectedResult -ceq 'accepted') {
            if (-not [string]::IsNullOrWhiteSpace([string]$resolverFailure)) { throw 'CONFORMANCE_ACCEPTED_CASE_REJECTED' }
            if ($resolverOutput.Count -ne 1) { throw 'CONFORMANCE_ACCEPTED_OUTPUT_COUNT_REJECTED' }
            $projectionText = ($resolverOutput | Out-String).Trim()
            try { $projection = $projectionText | ConvertFrom-Json -ErrorAction Stop }
            catch { throw 'CONFORMANCE_ACCEPTED_OUTPUT_JSON_REJECTED' }
            $projectionFields = @('contract_schema', 'contract_type', 'contract_file_sha256', 'mcp_tool', 'intent', 'submit_fields')
            Assert-ExactPropertyOrder -Object $projection -Expected $projectionFields -Failure 'CONFORMANCE_ACCEPTED_PROJECTION_FIELDS_REJECTED'
            $fixtureSha256 = Get-BytesSha256 -Bytes $fixtureBytes
            $registryEntry = $registryByType[$positiveType]
            $submitFields = @($projection.submit_fields)
            $expectedSubmitFields = @($registryEntry.submit_fields)
            if ([string]$projection.contract_schema -cne [string]$registryEntry.contract_schema -or
                [string]$projection.contract_type -cne $positiveType -or
                [string]$projection.contract_file_sha256 -cne $fixtureSha256 -or
                [string]$projection.mcp_tool -cne [string]$registryEntry.mcp_tool -or
                [string]$projection.mcp_tool -cne [string]$case.expected.mcp_tool -or
                [string]$projection.intent -cne [string]$registryEntry.intent -or
                $submitFields.Count -ne $expectedSubmitFields.Count) {
                throw 'CONFORMANCE_ACCEPTED_PROJECTION_MISMATCH'
            }
            for ($submitIndex = 0; $submitIndex -lt $expectedSubmitFields.Count; $submitIndex++) {
                if ([string]$submitFields[$submitIndex] -cne [string]$expectedSubmitFields[$submitIndex]) {
                    throw 'CONFORMANCE_ACCEPTED_PROJECTION_MISMATCH'
                }
            }
            $expectedProjectionText = [ordered]@{
                contract_schema = [string]$registryEntry.contract_schema
                contract_type = $positiveType
                contract_file_sha256 = $fixtureSha256
                mcp_tool = [string]$registryEntry.mcp_tool
                intent = [string]$registryEntry.intent
                submit_fields = @($expectedSubmitFields)
            } | ConvertTo-Json -Compress -Depth 5
            if ($projectionText -cne $expectedProjectionText) { throw 'CONFORMANCE_ACCEPTED_NORMALIZATION_REJECTED' }
            $acceptedCount++
        }
        else {
            if ($resolverOutput.Count -ne 0) { throw 'CONFORMANCE_REJECTION_EMITTED_PROJECTION' }
            if ([string]::IsNullOrWhiteSpace([string]$resolverFailure)) { throw 'CONFORMANCE_EXPECTED_REJECTION_MISSING' }
            if ([string]$resolverFailure -cne [string]$case.expected.rejection_code) { throw 'CONFORMANCE_REJECTION_CODE_MISMATCH' }
            $rejectedCount++
        }
    }

    $coveredTypeCount = $positiveTypes.Count
    $registryCoverage = ($coveredTypeCount -eq $registryTypeCount)
    if ($registryCoverage) {
        foreach ($registryType in $registryTypes) {
            if (-not $positiveTypes.Contains($registryType)) {
                $registryCoverage = $false
                break
            }
        }
    }
    if (-not $registryCoverage) { throw 'CONFORMANCE_REGISTRY_TYPE_UNCOVERED' }
    if ($acceptedCount -ne $registryTypeCount -or $rejectedCount -ne ($caseCount - $registryTypeCount)) {
        throw 'CONFORMANCE_MANIFEST_OUTCOME_COUNT_REJECTED'
    }
    if ($resolverInvocationCount -ne $caseCount) { throw 'CONFORMANCE_RESOLVER_INVOCATION_COUNT_REJECTED' }
    $result = 'PASS'
}
catch {
    $failure = [string]$_.Exception.Message
    if ($failure -cmatch '^CONFORMANCE_[A-Z0-9_]{1,120}$') { $firstFailure = $failure }
    else { $firstFailure = 'CONFORMANCE_RUNTIME_ERROR' }
}

[ordered]@{
    manifest_sha256 = $manifestSha256
    registry_sha256 = $registrySha256
    case_count = $caseCount
    accepted_count = $acceptedCount
    rejected_count = $rejectedCount
    resolver_invocation_count = $resolverInvocationCount
    registry_type_count = $registryTypeCount
    covered_type_count = $coveredTypeCount
    registry_coverage = $registryCoverage
    mapped_intent = $mappedIntent
    result = $result
    first_failure = $firstFailure
} | ConvertTo-Json -Compress -Depth 5
