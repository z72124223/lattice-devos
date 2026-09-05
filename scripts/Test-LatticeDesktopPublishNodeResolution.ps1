[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$publisherPath = Join-Path $PSScriptRoot 'Publish-LatticeDesktopCandidate.ps1'
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(
    $publisherPath, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -ne 0) {
    throw 'DESKTOP_PUBLISH_TEST_PARSE_ERROR'
}
foreach ($functionName in @(
    'Resolve-LatticeDesktopPublishNodeApplication',
    'Assert-LatticeDesktopPublishSourceState',
    'Copy-LatticeControlBackgroundScripts')) {
    $definition = @($ast.FindAll({
        param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
            $node.Name -ceq $functionName
    }, $true))
    if ($definition.Count -ne 1) {
        throw "DESKTOP_PUBLISH_TEST_FUNCTION_MISSING:$functionName"
    }
    Invoke-Expression $definition[0].Extent.Text
}

Assert-LatticeDesktopPublishSourceState -StatusLines @() -StagedPaths @()
Assert-LatticeDesktopPublishSourceState -StatusLines @(' M HANDOFF.md') -StagedPaths @()
foreach ($rejectedState in @(
    @{ Status = @(' M apps/lattice-control/src/server.mjs'); Staged = @(); Error = 'SOURCE_NOT_COMMITTED' },
    @{ Status = @(' M HANDOFF.md', '?? other.txt'); Staged = @(); Error = 'SOURCE_NOT_COMMITTED' },
    @{ Status = @('M  HANDOFF.md'); Staged = @('HANDOFF.md'); Error = 'STAGED_CHANGES_PRESENT' },
    @{ Status = @('MM HANDOFF.md'); Staged = @('HANDOFF.md'); Error = 'STAGED_CHANGES_PRESENT' })) {
    try {
        Assert-LatticeDesktopPublishSourceState -StatusLines $rejectedState.Status -StagedPaths $rejectedState.Staged
        throw 'DESKTOP_PUBLISH_TEST_UNCOMMITTED_SOURCE_ACCEPTED'
    }
    catch {
        if (-not $_.Exception.Message.StartsWith('DESKTOP_CANDIDATE_' + $rejectedState.Error + ':')) {
            throw
        }
    }
}

$temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) (
    'lattice-desktop-publish-node-test-' + [Guid]::NewGuid().ToString('N'))
$originalPath = $env:PATH
try {
    $repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
    $runtimeRoot = Join-Path $temporaryRoot 'control-runtime'
    Copy-LatticeControlBackgroundScripts -RepositoryRoot $repositoryRoot -ControlRuntimeDirectory $runtimeRoot
    $packagedScripts = @(Get-ChildItem -LiteralPath (Join-Path $runtimeRoot 'scripts') -File)
    if ($packagedScripts.Count -ne 3) {
        throw 'DESKTOP_PUBLISH_TEST_BACKGROUND_FILE_SET_MISMATCH'
    }
    foreach ($scriptFile in $packagedScripts) {
        $source = Join-Path $PSScriptRoot $scriptFile.Name
        if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -cne
            (Get-FileHash -LiteralPath $scriptFile.FullName -Algorithm SHA256).Hash) {
            throw 'DESKTOP_PUBLISH_TEST_BACKGROUND_FILE_CONTENT_MISMATCH'
        }
    }
    $realNodePath = @(Get-Command node.exe -CommandType Application -All -ErrorAction Stop)[0].Source
    Copy-Item -LiteralPath $realNodePath -Destination (Join-Path $runtimeRoot 'node.exe')
    $missingDestination = Join-Path $temporaryRoot 'missing-source-runtime'
    try {
        Copy-LatticeControlBackgroundScripts -RepositoryRoot $temporaryRoot -ControlRuntimeDirectory $missingDestination
        throw 'DESKTOP_PUBLISH_TEST_MISSING_BACKGROUND_SCRIPT_ACCEPTED'
    }
    catch {
        if ($_.Exception.Message -cne 'DESKTOP_CANDIDATE_BACKGROUND_SCRIPT_MISSING:Install-LatticeControlBackgroundTask.ps1') {
            throw
        }
    }
    if (Test-Path -LiteralPath $missingDestination) {
        throw 'DESKTOP_PUBLISH_TEST_MISSING_BACKGROUND_SOURCE_CREATED_PAYLOAD'
    }

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

    $candidateAst = [Management.Automation.Language.Parser]::ParseFile(
        (Join-Path $PSScriptRoot 'Test-LatticeDesktopCandidate.ps1'), [ref]$tokens, [ref]$parseErrors)
    $nodeAssignment = @($candidateAst.FindAll({
        param($node)
        $node -is [Management.Automation.Language.AssignmentStatementAst] -and
            $node.Left.Extent.Text -ceq '$nodePath'
    }, $true))
    if ($parseErrors.Count -ne 0 -or $nodeAssignment.Count -ne 1) {
        throw 'DESKTOP_CANDIDATE_NODE_TEST_ASSIGNMENT_MISSING'
    }
    $candidateDirectoryFull = $temporaryRoot
    $controlRuntimeExecutable = 'control-runtime/node.exe'
    Invoke-Expression $nodeAssignment[0].Extent.Text
    if ($nodePath -isnot [string] -or $nodePath -cne (Join-Path $runtimeRoot 'node.exe')) {
        throw 'DESKTOP_CANDIDATE_NODE_TEST_PACKAGED_NODE_NOT_SELECTED'
    }
    $nodeStart = [Diagnostics.ProcessStartInfo]::new()
    $nodeStart.FileName = $nodePath
    $nodeStart.Arguments = '--version'
    $nodeStart.UseShellExecute = $false
    $nodeStart.CreateNoWindow = $true
    $nodeStart.RedirectStandardOutput = $true
    $nodeProcess = [Diagnostics.Process]::Start($nodeStart)
    try {
        $nodeVersion = $nodeProcess.StandardOutput.ReadToEnd()
        $nodeProcess.WaitForExit()
        if ($nodeProcess.ExitCode -ne 0 -or $nodeVersion.Trim() -cnotmatch '^v[0-9]+\.[0-9]+\.[0-9]+$') {
            throw 'DESKTOP_CANDIDATE_NODE_TEST_PACKAGED_NODE_LAUNCH_FAILED'
        }
    } finally { $nodeProcess.Dispose() }

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
    Write-Output 'LATTICE_DESKTOP_PUBLISH_SOURCE_STATE_AND_BACKGROUND_PAYLOAD_TEST_PASS'
    Write-Output 'LATTICE_DESKTOP_CANDIDATE_PACKAGED_NODE_TEST_PASS'
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
