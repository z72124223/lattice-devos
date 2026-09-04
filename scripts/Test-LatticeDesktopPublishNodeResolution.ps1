[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$publisherPath = Join-Path $PSScriptRoot 'Publish-LatticeDesktopCandidate.ps1'
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(
    $publisherPath, [ref]$tokens, [ref]$parseErrors)
$definition = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -ceq 'Resolve-LatticeDesktopPublishNodeApplication'
}, $true))
if ($parseErrors.Count -ne 0 -or $definition.Count -ne 1) {
    throw 'DESKTOP_PUBLISH_NODE_TEST_RESOLVER_MISSING'
}
Invoke-Expression $definition[0].Extent.Text

$temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) (
    'lattice-desktop-publish-node-test-' + [Guid]::NewGuid().ToString('N'))
$originalPath = $env:PATH
try {
    $directories = @(
        (Join-Path $temporaryRoot 'first'),
        (Join-Path $temporaryRoot 'second'))
    $sourceExecutable = [Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
    $nodePaths = @($directories | ForEach-Object {
        [IO.Directory]::CreateDirectory($_) | Out-Null
        $path = Join-Path $_ 'node.exe'
        [IO.File]::Copy($sourceExecutable, $path)
        $path
    })
    $env:PATH = $directories -join [IO.Path]::PathSeparator
    $applications = @(Get-Command node.exe -CommandType Application -All -ErrorAction Stop)
    $selected = Resolve-LatticeDesktopPublishNodeApplication -CommandCandidates $applications
    if ($applications.Count -ne 2 -or $selected -isnot [string] -or
        -not [string]::Equals($selected, $nodePaths[0], [StringComparison]::OrdinalIgnoreCase)) {
        throw 'DESKTOP_PUBLISH_NODE_TEST_SELECTION_NOT_DETERMINISTIC'
    }

    $invalid = @(
        [pscustomobject]@{ CommandType = 'Application'; Source = $nodePaths },
        [pscustomobject]@{ CommandType = 'Application'; Source = 'Microsoft.PowerShell.Core\FileSystem::' + $nodePaths[0] },
        [pscustomobject]@{ CommandType = 'Application'; Source = Join-Path $temporaryRoot 'missing\node.exe' },
        [pscustomobject]@{ CommandType = 'Function'; Source = $nodePaths[0] })
    $afterInvalid = Resolve-LatticeDesktopPublishNodeApplication `
        -CommandCandidates @($invalid + $applications[1])
    if (-not [string]::Equals(
        $afterInvalid, $nodePaths[1], [StringComparison]::OrdinalIgnoreCase)) {
        throw 'DESKTOP_PUBLISH_NODE_TEST_INVALID_CANDIDATE_ACCEPTED'
    }
    try {
        Resolve-LatticeDesktopPublishNodeApplication -CommandCandidates $invalid | Out-Null
        throw 'DESKTOP_PUBLISH_NODE_TEST_INVALID_SET_NOT_REJECTED'
    }
    catch {
        if ($_.Exception.Message -cne 'DESKTOP_CANDIDATE_NODE_APPLICATION_UNAVAILABLE') {
            throw
        }
    }
    Write-Output 'LATTICE_DESKTOP_PUBLISH_NODE_RESOLUTION_TEST_PASS'
}
finally {
    $env:PATH = $originalPath
    $temporaryRootFull = [IO.Path]::GetFullPath($temporaryRoot)
    $temporaryPrefix = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
        [IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    if (-not $temporaryRootFull.StartsWith($temporaryPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        -not [IO.Path]::GetFileName($temporaryRootFull).StartsWith(
            'lattice-desktop-publish-node-test-', [StringComparison]::Ordinal)) {
        throw 'DESKTOP_PUBLISH_NODE_TEST_TEMPORARY_ROOT_INVALID'
    }
    if (Test-Path -LiteralPath $temporaryRootFull) {
        Remove-Item -LiteralPath $temporaryRootFull -Recurse -Force
    }
}
