[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$wrapperPath = Join-Path $PSScriptRoot 'Invoke-LatticeMcp.ps1'
$tokens = $null
$parserErrors = $null
$wrapperAst = [Management.Automation.Language.Parser]::ParseFile(
    $wrapperPath,
    [ref]$tokens,
    [ref]$parserErrors)
if ($parserErrors.Count -ne 0) {
    throw 'DIRECT_STDIO_WRAPPER_PARSE_REJECTED'
}

$utf8Assignment = $wrapperAst.Find({
    param($node)
    $node -is [Management.Automation.Language.AssignmentStatementAst] -and
        $node.Left.Extent.Text -ceq '$script:Utf8'
}, $true)
$writeFunction = $wrapperAst.Find({
    param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -ceq 'Write-JsonLine'
}, $true)
if ($null -eq $utf8Assignment -or $null -eq $writeFunction) {
    throw 'DIRECT_STDIO_FRAME_PATH_MISSING'
}

Invoke-Expression $utf8Assignment.Extent.Text
Invoke-Expression $writeFunction.Extent.Text

$wrapperText = [IO.File]::ReadAllText($wrapperPath)
foreach ($requiredSource in @(
    '$startInfo.StandardInputEncoding = $script:Utf8',
    '[Console]::InputEncoding = $script:Utf8',
    '[Console]::InputEncoding = $originalConsoleInputEncoding',
    '$process.StandardInput.NewLine = "`n"'
)) {
    if (-not $wrapperText.Contains($requiredSource)) {
        throw 'DIRECT_STDIO_ENCODING_PATH_REJECTED'
    }
}

function Assert-Frame {
    param(
        [Parameter(Mandatory = $true)]$Message,
        [Parameter(Mandatory = $true)][string]$ExpectedMethod,
        [Parameter(Mandatory = $true)][int]$ExpectedId
    )

    $stream = [IO.MemoryStream]::new()
    try {
        $writer = [IO.StreamWriter]::new($stream, $script:Utf8, 1024, $true)
        try {
            $writer.NewLine = "`n"
            Write-JsonLine -Writer $writer -Message $Message
        }
        finally {
            $writer.Dispose()
        }

        $bytes = $stream.ToArray()
        if ($bytes.Length -lt 2 -or $bytes[0] -ne 0x7b) {
            throw 'DIRECT_STDIO_FRAME_FIRST_BYTE_REJECTED'
        }
        if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xef -and $bytes[1] -eq 0xbb -and $bytes[2] -eq 0xbf) {
            throw 'DIRECT_STDIO_FRAME_BOM_REJECTED'
        }
        if ($bytes[$bytes.Length - 1] -ne 0x0a -or $bytes[$bytes.Length - 2] -eq 0x0d) {
            throw 'DIRECT_STDIO_FRAME_NEWLINE_REJECTED'
        }

        $decoded = [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
        $parsed = $decoded.Substring(0, $decoded.Length - 1) | ConvertFrom-Json -ErrorAction Stop
        if ([string]$parsed.method -cne $ExpectedMethod -or [int]$parsed.id -ne $ExpectedId) {
            throw 'DIRECT_STDIO_FRAME_JSON_REJECTED'
        }
    }
    finally {
        $stream.Dispose()
    }
}

Assert-Frame -ExpectedMethod 'initialize' -ExpectedId 1 -Message ([ordered]@{
    jsonrpc = '2.0'
    id = 1
    method = 'initialize'
    params = [ordered]@{
        protocolVersion = '2025-11-25'
        capabilities = [ordered]@{}
        clientInfo = [ordered]@{ name = 'lattice-direct-stdio-client'; version = '1.0.0' }
    }
})
Assert-Frame -ExpectedMethod 'tools/list' -ExpectedId 2 -Message ([ordered]@{
    jsonrpc = '2.0'
    id = 2
    method = 'tools/list'
    params = [ordered]@{}
})

Write-Output 'DIRECT_STDIO_STDIN_ENCODING_OFFLINE=PASS'
