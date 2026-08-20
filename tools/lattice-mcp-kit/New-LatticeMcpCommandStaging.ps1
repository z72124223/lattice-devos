[CmdletBinding(DefaultParameterSetName = 'Transform')]
param(
    [Parameter(Mandatory = $true, ParameterSetName = 'Transform')]
    [ValidateNotNullOrEmpty()]
    [string]$SourceConfigPath,

    [Parameter(Mandatory = $true, ParameterSetName = 'Transform')]
    [ValidateNotNullOrEmpty()]
    [string]$DestinationStagingPath,

    [Parameter(Mandatory = $true, ParameterSetName = 'Transform')]
    [ValidateNotNullOrEmpty()]
    [string]$ExpectedCommandPath,

    [Parameter(Mandatory = $true, ParameterSetName = 'Transform')]
    [ValidatePattern('^[0-9a-fA-F]{64}$')]
    [string]$ExpectedCommandSha256,

    [Parameter(ParameterSetName = 'Transform')]
    [ValidatePattern('^[A-Za-z0-9_-]+$')]
    [string]$ServerName = 'lattice-devos',

    [Parameter(ParameterSetName = 'Transform')]
    [ValidateRange(0, 1024)]
    [int]$ExpectedEnvironmentKeyCount = 21,

    [Parameter(Mandatory = $true, ParameterSetName = 'SelfTest')]
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:Utf8Strict = [Text.UTF8Encoding]::new($false, $true)
$script:Utf8NoBom = [Text.UTF8Encoding]::new($false)

function Test-ByteRangeEqual {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Left,
        [Parameter(Mandatory = $true)][int]$LeftOffset,
        [Parameter(Mandatory = $true)][byte[]]$Right,
        [Parameter(Mandatory = $true)][int]$RightOffset,
        [Parameter(Mandatory = $true)][int]$Count
    )

    if (
        $LeftOffset -lt 0 -or
        $RightOffset -lt 0 -or
        $Count -lt 0 -or
        ($LeftOffset + $Count) -gt $Left.Length -or
        ($RightOffset + $Count) -gt $Right.Length
    ) {
        return $false
    }
    for ($index = 0; $index -lt $Count; $index++) {
        if ($Left[$LeftOffset + $index] -ne $Right[$RightOffset + $index]) {
            return $false
        }
    }
    return $true
}

function ConvertFrom-LatticeTomlCommandLiteral {
    param([Parameter(Mandatory = $true)][string]$Literal)

    if ($Literal.Length -ge 2 -and $Literal[0] -eq "'" -and $Literal[$Literal.Length - 1] -eq "'") {
        return $Literal.Substring(1, $Literal.Length - 2)
    }
    if ($Literal.Length -ge 2 -and $Literal[0] -eq '"' -and $Literal[$Literal.Length - 1] -eq '"') {
        try {
            $value = ConvertFrom-Json -InputObject $Literal -ErrorAction Stop
        }
        catch {
            throw 'LATTICE_MCP_STAGING_COMMAND_LITERAL_REJECTED'
        }
        if ($value -isnot [string]) {
            throw 'LATTICE_MCP_STAGING_COMMAND_LITERAL_REJECTED'
        }
        return [string]$value
    }
    throw 'LATTICE_MCP_STAGING_COMMAND_LITERAL_REJECTED'
}

function ConvertTo-LatticeTomlCommandLiteral {
    param(
        [Parameter(Mandatory = $true)][string]$Value,
        [Parameter(Mandatory = $true)][ValidateSet('Literal', 'Basic')][string]$QuoteForm
    )

    if ($Value.IndexOfAny([char[]]@(0x0a, 0x0d)) -ge 0) {
        throw 'LATTICE_MCP_STAGING_COMMAND_PATH_REJECTED'
    }
    if ($QuoteForm -ceq 'Literal') {
        if ($Value.Contains("'")) {
            throw 'LATTICE_MCP_STAGING_COMMAND_PATH_REJECTED'
        }
        return "'" + $Value + "'"
    }
    $escaped = $Value.Replace('\', '\\').Replace('"', '\"')
    return '"' + $escaped + '"'
}

function Get-LatticeMcpConfigLayout {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][int]$EnvironmentKeyCount
    )

    $escapedName = [Text.RegularExpressions.Regex]::Escape($Name)
    $sectionPattern = '(?m)^[ \t]*\[mcp_servers\.' + $escapedName + '\][ \t]*(?:#[^\r\n]*)?(?:\r?\n|\z)'
    $sectionMatches = [Text.RegularExpressions.Regex]::Matches(
        $Text,
        $sectionPattern,
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    if ($sectionMatches.Count -ne 1) {
        throw 'LATTICE_MCP_STAGING_SERVER_SECTION_REJECTED'
    }

    $section = $sectionMatches[0]
    $bodyStart = $section.Index + $section.Length
    $following = [Text.RegularExpressions.Regex]::Match(
        $Text.Substring($bodyStart),
        '(?m)^[ \t]*\[[^\r\n]+\][ \t]*(?:#[^\r\n]*)?(?:\r?\n|\z)',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    $bodyEnd = $Text.Length
    if ($following.Success) {
        $bodyEnd = $bodyStart + $following.Index
    }
    $body = $Text.Substring($bodyStart, $bodyEnd - $bodyStart)

    $commandKeyMatches = [Text.RegularExpressions.Regex]::Matches(
        $body,
        '(?m)^[ \t]*command[ \t]*=',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    $commandMatches = [Text.RegularExpressions.Regex]::Matches(
        $body,
        '(?m)^(?<prefix>[ \t]*command[ \t]*=[ \t]*)(?<value>''[^''\r\n]*''|"(?:[^"\\\r\n]|\\.)*")(?<suffix>[ \t]*(?:#[^\r\n]*)?)(?:\r?\n|\z)',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    if ($commandKeyMatches.Count -ne 1 -or $commandMatches.Count -ne 1) {
        throw 'LATTICE_MCP_STAGING_COMMAND_REJECTED'
    }
    if (
        [Text.RegularExpressions.Regex]::IsMatch(
            $body,
            '(?m)^[ \t]*(?:args|transport)[ \t]*=',
            [Text.RegularExpressions.RegexOptions]::CultureInvariant
        )
    ) {
        throw 'LATTICE_MCP_STAGING_STDIO_SHAPE_REJECTED'
    }

    $environmentPattern = '(?m)^[ \t]*\[mcp_servers\.' + $escapedName + '\.env\][ \t]*(?:#[^\r\n]*)?(?:\r?\n|\z)'
    $environmentMatches = [Text.RegularExpressions.Regex]::Matches(
        $Text,
        $environmentPattern,
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    if ($environmentMatches.Count -ne 1) {
        throw 'LATTICE_MCP_STAGING_ENV_SECTION_REJECTED'
    }
    $environment = $environmentMatches[0]
    $environmentBodyStart = $environment.Index + $environment.Length
    $environmentFollowing = [Text.RegularExpressions.Regex]::Match(
        $Text.Substring($environmentBodyStart),
        '(?m)^[ \t]*\[[^\r\n]+\][ \t]*(?:#[^\r\n]*)?(?:\r?\n|\z)',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    $environmentBodyEnd = $Text.Length
    if ($environmentFollowing.Success) {
        $environmentBodyEnd = $environmentBodyStart + $environmentFollowing.Index
    }
    $environmentBody = $Text.Substring($environmentBodyStart, $environmentBodyEnd - $environmentBodyStart)
    $environmentEntries = [Text.RegularExpressions.Regex]::Matches(
        $environmentBody,
        '(?m)^[ \t]*(?<key>[A-Za-z_][A-Za-z0-9_-]*)[ \t]*=',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    $environmentKeys = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($entry in $environmentEntries) {
        if (-not $environmentKeys.Add($entry.Groups['key'].Value)) {
            throw 'LATTICE_MCP_STAGING_ENV_KEYS_REJECTED'
        }
    }
    if ($environmentEntries.Count -ne $EnvironmentKeyCount) {
        throw 'LATTICE_MCP_STAGING_ENV_KEYS_REJECTED'
    }

    $command = $commandMatches[0]
    $literal = $command.Groups['value'].Value
    $quoteForm = 'Basic'
    if ($literal[0] -eq "'") {
        $quoteForm = 'Literal'
    }
    return [pscustomobject]@{
        ValueIndex = $bodyStart + $command.Groups['value'].Index
        ValueLength = $command.Groups['value'].Length
        Literal = $literal
        QuoteForm = $quoteForm
        CommandPath = ConvertFrom-LatticeTomlCommandLiteral -Literal $literal
        EnvironmentKeyCount = $environmentEntries.Count
    }
}

function Assert-LatticeMcpStagingBytes {
    param(
        [Parameter(Mandatory = $true)][byte[]]$SourceBytes,
        [Parameter(Mandatory = $true)][string]$SourceText,
        [Parameter(Mandatory = $true)]$SourceLayout,
        [Parameter(Mandatory = $true)][byte[]]$StagingBytes,
        [Parameter(Mandatory = $true)][string]$StagingText,
        [Parameter(Mandatory = $true)][int]$Offset,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][int]$EnvironmentKeyCount,
        [Parameter(Mandatory = $true)][string]$ExpectedPath
    )

    $stagingLayout = Get-LatticeMcpConfigLayout `
        -Text $StagingText `
        -Name $Name `
        -EnvironmentKeyCount $EnvironmentKeyCount
    if ($stagingLayout.QuoteForm -cne $SourceLayout.QuoteForm) {
        throw 'LATTICE_MCP_STAGING_QUOTE_FORM_CHANGED'
    }
    if (-not [String]::Equals($stagingLayout.CommandPath, $ExpectedPath, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'LATTICE_MCP_STAGING_COMMAND_PATH_MISMATCH'
    }

    $sourcePrefixLength = $Offset + $script:Utf8Strict.GetByteCount(
        $SourceText.Substring(0, $SourceLayout.ValueIndex)
    )
    $sourceValueLength = $script:Utf8Strict.GetByteCount($SourceLayout.Literal)
    $stagingPrefixLength = $Offset + $script:Utf8Strict.GetByteCount(
        $StagingText.Substring(0, $stagingLayout.ValueIndex)
    )
    $stagingValueLength = $script:Utf8Strict.GetByteCount($stagingLayout.Literal)
    $sourceSuffixLength = $SourceBytes.Length - $sourcePrefixLength - $sourceValueLength
    $stagingSuffixLength = $StagingBytes.Length - $stagingPrefixLength - $stagingValueLength
    if (
        $sourcePrefixLength -ne $stagingPrefixLength -or
        $sourceSuffixLength -ne $stagingSuffixLength -or
        -not (Test-ByteRangeEqual `
            -Left $SourceBytes `
            -LeftOffset 0 `
            -Right $StagingBytes `
            -RightOffset 0 `
            -Count $sourcePrefixLength) -or
        -not (Test-ByteRangeEqual `
            -Left $SourceBytes `
            -LeftOffset ($sourcePrefixLength + $sourceValueLength) `
            -Right $StagingBytes `
            -RightOffset ($stagingPrefixLength + $stagingValueLength) `
            -Count $sourceSuffixLength)
    ) {
        throw 'LATTICE_MCP_STAGING_NON_COMMAND_BYTES_CHANGED'
    }
    return $stagingLayout
}

function Invoke-LatticeMcpCommandStaging {
    param(
        [Parameter(Mandatory = $true)][string]$SourcePath,
        [Parameter(Mandatory = $true)][string]$StagingPath,
        [Parameter(Mandatory = $true)][string]$CommandPath,
        [Parameter(Mandatory = $true)][string]$CommandSha256,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][int]$EnvironmentKeyCount
    )

    $source = [IO.Path]::GetFullPath($SourcePath)
    $staging = [IO.Path]::GetFullPath($StagingPath)
    $command = [IO.Path]::GetFullPath($CommandPath)
    if (
        [String]::Equals($source, $staging, [StringComparison]::OrdinalIgnoreCase) -or
        -not (Test-Path -LiteralPath $source -PathType Leaf) -or
        -not (Test-Path -LiteralPath $command -PathType Leaf) -or
        (Test-Path -LiteralPath $staging)
    ) {
        throw 'LATTICE_MCP_STAGING_PATH_REJECTED'
    }
    $stagingParent = Split-Path -Parent $staging
    if (-not (Test-Path -LiteralPath $stagingParent -PathType Container)) {
        throw 'LATTICE_MCP_STAGING_PATH_REJECTED'
    }
    $actualCommandSha256 = (Get-FileHash -LiteralPath $command -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualCommandSha256 -cne $CommandSha256.ToLowerInvariant()) {
        throw 'LATTICE_MCP_STAGING_COMMAND_HASH_REJECTED'
    }

    $sourceBytes = [IO.File]::ReadAllBytes($source)
    $hasBom = (
        $sourceBytes.Length -ge 3 -and
        $sourceBytes[0] -eq 0xef -and
        $sourceBytes[1] -eq 0xbb -and
        $sourceBytes[2] -eq 0xbf
    )
    $offset = 0
    if ($hasBom) {
        $offset = 3
    }
    $sourceText = $script:Utf8Strict.GetString($sourceBytes, $offset, $sourceBytes.Length - $offset)
    $sourceLayout = Get-LatticeMcpConfigLayout `
        -Text $sourceText `
        -Name $Name `
        -EnvironmentKeyCount $EnvironmentKeyCount
    $replacementLiteral = ConvertTo-LatticeTomlCommandLiteral `
        -Value $command `
        -QuoteForm $sourceLayout.QuoteForm

    $prefixLength = $offset + $script:Utf8Strict.GetByteCount(
        $sourceText.Substring(0, $sourceLayout.ValueIndex)
    )
    $oldValueLength = $script:Utf8Strict.GetByteCount($sourceLayout.Literal)
    $replacementBytes = $script:Utf8NoBom.GetBytes($replacementLiteral)
    $suffixOffset = $prefixLength + $oldValueLength
    $outputBytes = [byte[]]::new(
        $prefixLength + $replacementBytes.Length + ($sourceBytes.Length - $suffixOffset)
    )
    [Array]::Copy($sourceBytes, 0, $outputBytes, 0, $prefixLength)
    [Array]::Copy($replacementBytes, 0, $outputBytes, $prefixLength, $replacementBytes.Length)
    [Array]::Copy(
        $sourceBytes,
        $suffixOffset,
        $outputBytes,
        $prefixLength + $replacementBytes.Length,
        $sourceBytes.Length - $suffixOffset
    )

    $stream = [IO.File]::Open($staging, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $stream.Write($outputBytes, 0, $outputBytes.Length)
        $stream.Flush($true)
    }
    finally {
        $stream.Dispose()
    }

    $stageReload = [IO.File]::ReadAllBytes($staging)
    $stageHasBom = (
        $stageReload.Length -ge 3 -and
        $stageReload[0] -eq 0xef -and
        $stageReload[1] -eq 0xbb -and
        $stageReload[2] -eq 0xbf
    )
    if ($stageHasBom -ne $hasBom) {
        throw 'LATTICE_MCP_STAGING_BOM_CHANGED'
    }
    $stageDecoded = $script:Utf8Strict.GetString($stageReload, $offset, $stageReload.Length - $offset)
    $stagingLayout = Assert-LatticeMcpStagingBytes `
        -SourceBytes $sourceBytes `
        -SourceText $sourceText `
        -SourceLayout $sourceLayout `
        -StagingBytes $stageReload `
        -StagingText $stageDecoded `
        -Offset $offset `
        -Name $Name `
        -EnvironmentKeyCount $EnvironmentKeyCount `
        -ExpectedPath $command
    $stagingSha256 = (Get-FileHash -LiteralPath $staging -Algorithm SHA256).Hash.ToLowerInvariant()

    return [pscustomobject]@{
        schema = 'lattice.mcp-command-staging.v1'
        success = $true
        source_path = $source
        destination_staging_path = $staging
        expected_command_path = $command
        expected_command_sha256 = $actualCommandSha256
        staging_sha256 = $stagingSha256
        quote_form = $stagingLayout.QuoteForm
        utf8_bom_offset = $offset
        environment_key_count = $stagingLayout.EnvironmentKeyCount
        non_command_bytes_unchanged = $true
        source_replaced = $false
    }
}

function Invoke-LatticeMcpCommandStagingSelfTest {
    $fixtureRoot = Join-Path `
        ([IO.Path]::GetTempPath()) `
        ('lattice-mcp-command-staging-fixture-' + [Guid]::NewGuid().ToString('N'))
    [IO.Directory]::CreateDirectory($fixtureRoot) | Out-Null
    try {
        $commandPath = Join-Path $fixtureRoot 'new command\latticed.exe'
        [IO.Directory]::CreateDirectory((Split-Path -Parent $commandPath)) | Out-Null
        [IO.File]::WriteAllBytes($commandPath, [Text.Encoding]::ASCII.GetBytes('non-live-latticed-fixture'))
        $commandSha256 = (Get-FileHash -LiteralPath $commandPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $environmentLines = @()
        for ($number = 1; $number -le 21; $number++) {
            $environmentLines += ('LATTICE_FIXTURE_{0:D2} = "fake-value-{0:D2}"' -f $number)
        }
        $cases = @(
            [pscustomobject]@{
                Name = 'literal-no-bom'
                CommandLiteral = "'C:\old\latticed.exe'"
                Bom = $false
                ExpectedOffset = 0
            },
            [pscustomobject]@{
                Name = 'basic-with-bom'
                CommandLiteral = '"C:\\Old Path\\latticed.exe"'
                Bom = $true
                ExpectedOffset = 3
            }
        )
        $caseResults = @()
        foreach ($case in $cases) {
            $sourcePath = Join-Path $fixtureRoot ($case.Name + '.source.toml')
            $stagingPath = Join-Path $fixtureRoot ($case.Name + '.staging.toml')
            $lines = @(
                'model = "fixture-model"',
                '',
                '[mcp_servers.lattice-devos]',
                ('command = ' + $case.CommandLiteral),
                '',
                '[mcp_servers.lattice-devos.env]'
            ) + $environmentLines + @(
                '',
                '[unrelated]',
                'preserved = "exactly"'
            )
            $text = ($lines -join "`r`n") + "`r`n"
            $textBytes = $script:Utf8NoBom.GetBytes($text)
            if ($case.Bom) {
                $sourceBytes = [byte[]]::new($textBytes.Length + 3)
                $sourceBytes[0] = 0xef
                $sourceBytes[1] = 0xbb
                $sourceBytes[2] = 0xbf
                [Array]::Copy($textBytes, 0, $sourceBytes, 3, $textBytes.Length)
            }
            else {
                $sourceBytes = $textBytes
            }
            [IO.File]::WriteAllBytes($sourcePath, $sourceBytes)

            $result = & $PSCommandPath `
                -SourceConfigPath $sourcePath `
                -DestinationStagingPath $stagingPath `
                -ExpectedCommandPath $commandPath `
                -ExpectedCommandSha256 $commandSha256 `
                -ExpectedEnvironmentKeyCount 21
            if (
                $null -eq $result -or
                -not [bool]$result.success -or
                [int]$result.utf8_bom_offset -ne [int]$case.ExpectedOffset -or
                [int]$result.environment_key_count -ne 21 -or
                -not [bool]$result.non_command_bytes_unchanged -or
                [bool]$result.source_replaced
            ) {
                throw ('LATTICE_MCP_STAGING_SELF_TEST_REJECTED|' + $case.Name)
            }
            $caseResults += [pscustomobject]@{
                name = $case.Name
                quote_form = [string]$result.quote_form
                utf8_bom_offset = [int]$result.utf8_bom_offset
                environment_key_count = [int]$result.environment_key_count
                non_command_bytes_unchanged = [bool]$result.non_command_bytes_unchanged
            }
        }
        return [pscustomobject]@{
            schema = 'lattice.mcp-command-staging-self-test.v1'
            success = $true
            live_config_accessed = $false
            case_count = $caseResults.Count
            cases = $caseResults
        }
    }
    finally {
        if (Test-Path -LiteralPath $fixtureRoot -PathType Container) {
            [IO.Directory]::Delete($fixtureRoot, $true)
        }
    }
}

if ($PSCmdlet.ParameterSetName -ceq 'SelfTest') {
    Invoke-LatticeMcpCommandStagingSelfTest
}
else {
    Invoke-LatticeMcpCommandStaging `
        -SourcePath $SourceConfigPath `
        -StagingPath $DestinationStagingPath `
        -CommandPath $ExpectedCommandPath `
        -CommandSha256 $ExpectedCommandSha256 `
        -Name $ServerName `
        -EnvironmentKeyCount $ExpectedEnvironmentKeyCount
}
