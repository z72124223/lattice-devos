[CmdletBinding()]
param(
    [string]$OutputRoot = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-Sha256Hex {
    param([string]$LiteralPath)

    $stream = [IO.File]::Open(
        [IO.Path]::GetFullPath($LiteralPath),
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        [IO.FileShare]::Read)
    try {
        $algorithm = [Security.Cryptography.SHA256]::Create()
        try {
            return ([BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
        }
        finally {
            $algorithm.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Resolve-LatticeDesktopPublishNodeApplication {
    param([object[]]$CommandCandidates)

    foreach ($commandCandidate in $CommandCandidates) {
        if ($null -eq $commandCandidate) {
            continue
        }
        $commandTypeProperty = $commandCandidate.PSObject.Properties['CommandType']
        $sourceProperty = $commandCandidate.PSObject.Properties['Source']
        if ($null -eq $commandTypeProperty -or
            [string]$commandTypeProperty.Value -cne 'Application' -or
            $null -eq $sourceProperty -or
            $sourceProperty.Value -isnot [string]) {
            continue
        }
        $source = [string]$sourceProperty.Value
        if ([string]::IsNullOrWhiteSpace($source) -or
            $source.Contains('::') -or
            -not [IO.Path]::IsPathRooted($source)) {
            continue
        }
        try {
            $fullPath = [IO.Path]::GetFullPath($source)
        }
        catch {
            continue
        }
        if (Test-Path -LiteralPath $fullPath -PathType Leaf) {
            return $fullPath
        }
    }
    throw 'DESKTOP_CANDIDATE_NODE_APPLICATION_UNAVAILABLE'
}

function Assert-LatticeDesktopPublishSourceState {
    param([string[]]$StatusLines = @(), [string[]]$StagedPaths = @())

    if ($StagedPaths.Count -ne 0) {
        throw ('DESKTOP_CANDIDATE_STAGED_CHANGES_PRESENT: ' + ($StagedPaths -join ', '))
    }
    if ($StatusLines.Count -ne 0 -and
        ($StatusLines.Count -ne 1 -or $StatusLines[0] -cne ' M HANDOFF.md')) {
        throw ('DESKTOP_CANDIDATE_SOURCE_NOT_COMMITTED: ' + ($StatusLines -join ', '))
    }
}

function Copy-LatticeControlBackgroundScripts {
    param([string]$RepositoryRoot, [string]$ControlRuntimeDirectory)

    $scriptNames = @(
        'Install-LatticeControlBackgroundTask.ps1',
        'Run-LatticeControlBackgroundTask.ps1',
        'Test-LatticeControlBackgroundTask.ps1')
    foreach ($name in $scriptNames) {
        if (-not (Test-Path -LiteralPath (Join-Path $RepositoryRoot "scripts\$name") -PathType Leaf)) {
            throw "DESKTOP_CANDIDATE_BACKGROUND_SCRIPT_MISSING:$name"
        }
    }
    $destination = Join-Path $ControlRuntimeDirectory 'scripts'
    [IO.Directory]::CreateDirectory($destination) | Out-Null
    foreach ($name in $scriptNames) {
        Copy-Item -LiteralPath (Join-Path $RepositoryRoot "scripts\$name") -Destination (Join-Path $destination $name)
    }
}

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$projectPath = Join-Path $repositoryRoot 'apps\lattice-control-desktop\Lattice.Control.Desktop.csproj'
$controlSourceRoot = Join-Path $repositoryRoot 'apps\lattice-control'
$runtimeIdentityPath = Join-Path $repositoryRoot 'apps\lattice-control\runtime-identity.json'
$dataScopeContractPath = Join-Path $repositoryRoot 'apps\lattice-control\data-scope-contract.json'
$installerScriptPath = Join-Path $repositoryRoot 'scripts\Install-LATTICE.ps1'
$uninstallerScriptPath = Join-Path $repositoryRoot 'scripts\Uninstall-LATTICE.ps1'
$installerCommonPath = Join-Path $repositoryRoot 'scripts\LatticeDesktopInstaller.Common.ps1'
$installerNoticePath = Join-Path $repositoryRoot 'scripts\INSTALL-LATTICE.txt'
foreach ($installerSourcePath in @(
    $installerScriptPath,
    $uninstallerScriptPath,
    $installerCommonPath,
    $installerNoticePath)) {
    if (-not (Test-Path -LiteralPath $installerSourcePath -PathType Leaf)) {
        throw "DESKTOP_INSTALLER_SOURCE_MISSING:$installerSourcePath"
    }
}
try {
    $nodeApplications = @(
        Get-Command node.exe -CommandType Application -All -ErrorAction Stop)
}
catch {
    throw 'DESKTOP_CANDIDATE_NODE_APPLICATION_UNAVAILABLE'
}
$nodePath = Resolve-LatticeDesktopPublishNodeApplication `
    -CommandCandidates $nodeApplications
$nodeVersion = ([string](& $nodePath --version)).Trim()
if ($LASTEXITCODE -ne 0 -or $nodeVersion -notmatch '^v([0-9]+\.[0-9]+\.[0-9]+)$') {
    throw 'DESKTOP_CANDIDATE_NODE_VERSION_UNAVAILABLE'
}
$parsedNodeVersion = [Version]$Matches[1]
if ($parsedNodeVersion -lt [Version]'24.15.0') {
    throw "DESKTOP_CANDIDATE_NODE_VERSION_UNSUPPORTED:$nodeVersion"
}
try {
    $runtimeIdentity = Get-Content -LiteralPath $runtimeIdentityPath -Raw | ConvertFrom-Json
}
catch {
    throw 'DESKTOP_CANDIDATE_CONTROL_IDENTITY_INVALID_JSON'
}
if (
    [string]$runtimeIdentity.schema_version -cne 'lattice.control.runtime-identity.v1' -or
    [string]$runtimeIdentity.product -cne 'LATTICE_CONTROL' -or
    [string]::IsNullOrWhiteSpace([string]$runtimeIdentity.version)
) {
    throw 'DESKTOP_CANDIDATE_CONTROL_IDENTITY_INVALID'
}
try {
    $dataScopeContract = Get-Content -LiteralPath $dataScopeContractPath -Raw | ConvertFrom-Json
}
catch {
    throw 'DESKTOP_CANDIDATE_DATA_SCOPE_CONTRACT_INVALID_JSON'
}
if (
    [string]$dataScopeContract.schema_version -cne 'lattice.control.data-scope.v1' -or
    [string]$dataScopeContract.store -cne 'CONTROL_SQLITE' -or
    [int]$dataScopeContract.store_schema_version -ne 7 -or
    [string]$dataScopeContract.authority_class -cne 'CONTROL_LOCAL_PRODUCT_STATE' -or
    [string]$dataScopeContract.registry_authority -cne 'NONE'
) {
    throw 'DESKTOP_CANDIDATE_DATA_SCOPE_CONTRACT_INVALID'
}

$headSha = [string](& git -C $repositoryRoot rev-parse HEAD)
if ($LASTEXITCODE -ne 0 -or $headSha -notmatch '^[0-9a-f]{40}$') {
    throw 'DESKTOP_CANDIDATE_GIT_HEAD_UNAVAILABLE'
}
$headSha = $headSha.Trim()

$statusLines = @(& git -C $repositoryRoot status --porcelain --untracked-files=all)
if ($LASTEXITCODE -ne 0) {
    throw 'DESKTOP_CANDIDATE_GIT_STATUS_UNAVAILABLE'
}
$stagedPaths = @(& git -C $repositoryRoot diff --cached --name-only)
if ($LASTEXITCODE -ne 0) {
    throw 'DESKTOP_CANDIDATE_GIT_STAGED_STATUS_UNAVAILABLE'
}
Assert-LatticeDesktopPublishSourceState -StatusLines $statusLines -StagedPaths $stagedPaths

if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
    $localApplicationData = [Environment]::GetFolderPath(
        [Environment+SpecialFolder]::LocalApplicationData)
    $OutputRoot = Join-Path $localApplicationData 'LATTICE\candidates'
}

$outputRootFull = [IO.Path]::GetFullPath($OutputRoot)
$candidateName = 'lattice-control-desktop-win-x64-' + $headSha.Substring(0, 12)
$candidateDirectory = [IO.Path]::GetFullPath((Join-Path $outputRootFull $candidateName))
$zipPath = [IO.Path]::GetFullPath((Join-Path $outputRootFull ($candidateName + '.zip')))
$installerName = $candidateName + '-per-user-installer'
$installerDirectory = [IO.Path]::GetFullPath((Join-Path $outputRootFull $installerName))
$installerZipPath = [IO.Path]::GetFullPath((Join-Path $outputRootFull ($installerName + '.zip')))
$outputPrefix = $outputRootFull.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
if (-not $candidateDirectory.StartsWith($outputPrefix, [StringComparison]::OrdinalIgnoreCase) -or
    -not $installerDirectory.StartsWith($outputPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'DESKTOP_CANDIDATE_OUTPUT_ESCAPED_ROOT'
}

[IO.Directory]::CreateDirectory($outputRootFull) | Out-Null
if (Test-Path -LiteralPath $candidateDirectory) {
    Remove-Item -LiteralPath $candidateDirectory -Recurse -Force
}
if (Test-Path -LiteralPath $zipPath) {
    Remove-Item -LiteralPath $zipPath -Force
}
if (Test-Path -LiteralPath $installerDirectory) {
    Remove-Item -LiteralPath $installerDirectory -Recurse -Force
}
if (Test-Path -LiteralPath $installerZipPath) {
    Remove-Item -LiteralPath $installerZipPath -Force
}

$publishArguments = @(
    'publish',
    $projectPath,
    '-c', 'Release',
    '-r', 'win-x64',
    '--self-contained', 'true',
    '--nologo',
    '-p:PublishSingleFile=false',
    '-p:DebugType=None',
    '-p:DebugSymbols=false',
    '-o', $candidateDirectory
)
& dotnet @publishArguments
if ($LASTEXITCODE -ne 0) {
    throw "DESKTOP_CANDIDATE_PUBLISH_FAILED:$LASTEXITCODE"
}

$controlRuntimeDirectory = Join-Path $candidateDirectory 'control-runtime'
$bundledControlRoot = Join-Path $controlRuntimeDirectory 'apps\lattice-control'
[IO.Directory]::CreateDirectory($bundledControlRoot) | Out-Null
Copy-Item -LiteralPath $nodePath -Destination (Join-Path $controlRuntimeDirectory 'node.exe')
Copy-Item -LiteralPath (Join-Path $controlSourceRoot 'src') -Destination $bundledControlRoot -Recurse
Copy-Item -LiteralPath (Join-Path $controlSourceRoot 'public') -Destination $bundledControlRoot -Recurse
Copy-Item -LiteralPath $runtimeIdentityPath -Destination (
    Join-Path $bundledControlRoot 'runtime-identity.json')
Copy-Item -LiteralPath $dataScopeContractPath -Destination (
    Join-Path $bundledControlRoot 'data-scope-contract.json')
Copy-LatticeControlBackgroundScripts -RepositoryRoot $repositoryRoot -ControlRuntimeDirectory $controlRuntimeDirectory

$runtimeNodeRelativePath = 'control-runtime/node.exe'
$runtimeServerRelativePath = 'control-runtime/apps/lattice-control/src/server.mjs'
$runtimeNodePath = Join-Path $candidateDirectory ($runtimeNodeRelativePath.Replace('/', '\'))
$runtimeServerPath = Join-Path $candidateDirectory ($runtimeServerRelativePath.Replace('/', '\'))
if (-not (Test-Path -LiteralPath $runtimeNodePath -PathType Leaf) -or
    -not (Test-Path -LiteralPath $runtimeServerPath -PathType Leaf)) {
    throw 'DESKTOP_CANDIDATE_CONTROL_RUNTIME_MISSING'
}

$executablePath = Join-Path $candidateDirectory 'LATTICE.exe'
$managedAssembly = Join-Path $candidateDirectory 'LATTICE.dll'
$candidateNotice = Join-Path $candidateDirectory 'PORTABLE_RELEASE_CANDIDATE.txt'
if (-not (Test-Path -LiteralPath $executablePath -PathType Leaf)) {
    throw 'DESKTOP_CANDIDATE_EXECUTABLE_MISSING'
}
if (-not (Test-Path -LiteralPath $managedAssembly -PathType Leaf)) {
    throw 'DESKTOP_CANDIDATE_MANAGED_ASSEMBLY_MISSING'
}
if (-not (Test-Path -LiteralPath $candidateNotice -PathType Leaf)) {
    throw 'DESKTOP_CANDIDATE_NOTICE_MISSING'
}

$artifactFiles = @(Get-ChildItem -LiteralPath $candidateDirectory -File -Recurse |
    Sort-Object -Property FullName |
    ForEach-Object {
        [ordered]@{
            path = $_.FullName.Substring($candidateDirectory.Length + 1).Replace('\', '/')
            length = [long]$_.Length
            sha256 = Get-Sha256Hex -LiteralPath $_.FullName
        }
    })
if ($artifactFiles.Count -eq 0) {
    throw 'DESKTOP_CANDIDATE_FILE_SET_EMPTY'
}

$manifest = [ordered]@{
    schema_version = 'lattice.control.desktop-portable-candidate.v2'
    artifact_type = 'PORTABLE_RELEASE_CANDIDATE'
    source_commit = $headSha
    runtime_identifier = 'win-x64'
    self_contained = $true
    launch = 'LATTICE.exe'
    control_origin = 'http://127.0.0.1:4317/'
    webview_user_data = '%LOCALAPPDATA%\LATTICE\ControlDesktop\WebView2'
    executable_sha256 = Get-Sha256Hex -LiteralPath $executablePath
    control_runtime = [ordered]@{
        identity_schema = [string]$runtimeIdentity.schema_version
        product = [string]$runtimeIdentity.product
        version = [string]$runtimeIdentity.version
        data_scope_schema = [string]$dataScopeContract.schema_version
        store_schema_version = [int]$dataScopeContract.store_schema_version
        node_version = $nodeVersion
        node_sha256 = Get-Sha256Hex -LiteralPath $runtimeNodePath
        executable = $runtimeNodeRelativePath
        server = $runtimeServerRelativePath
        database = '%LOCALAPPDATA%\LATTICE\control\lattice-control.db'
    }
    files = $artifactFiles
}
$manifestPath = Join-Path $candidateDirectory 'candidate-manifest.json'
[IO.File]::WriteAllText(
    $manifestPath,
    (($manifest | ConvertTo-Json -Depth 6) + [Environment]::NewLine),
    [Text.UTF8Encoding]::new($false))

Compress-Archive -Path (Join-Path $candidateDirectory '*') -DestinationPath $zipPath -CompressionLevel Optimal
$archiveSha256 = Get-Sha256Hex -LiteralPath $zipPath

[IO.Directory]::CreateDirectory($installerDirectory) | Out-Null
$payloadArchivePath = Join-Path $installerDirectory 'payload.zip'
Copy-Item -LiteralPath $zipPath -Destination $payloadArchivePath
Copy-Item -LiteralPath $installerScriptPath -Destination (Join-Path $installerDirectory 'Install-LATTICE.ps1')
Copy-Item -LiteralPath $uninstallerScriptPath -Destination (Join-Path $installerDirectory 'Uninstall-LATTICE.ps1')
Copy-Item -LiteralPath $installerCommonPath -Destination (Join-Path $installerDirectory 'LatticeDesktopInstaller.Common.ps1')
Copy-Item -LiteralPath $installerNoticePath -Destination (Join-Path $installerDirectory 'INSTALL-LATTICE.txt')

$installerFiles = @(Get-ChildItem -LiteralPath $installerDirectory -File -Recurse |
    Sort-Object -Property FullName |
    ForEach-Object {
        [ordered]@{
            path = $_.FullName.Substring($installerDirectory.Length + 1).Replace('\', '/')
            length = [long]$_.Length
            sha256 = Get-Sha256Hex -LiteralPath $_.FullName
        }
    })
$payloadEntry = @($installerFiles | Where-Object { $_.path -ceq 'payload.zip' })
if ($installerFiles.Count -ne 5 -or $payloadEntry.Count -ne 1) {
    throw 'DESKTOP_INSTALLER_FILE_SET_INVALID'
}
$installerManifest = [ordered]@{
    schema_version = 'lattice.control.desktop-per-user-installer.v1'
    artifact_type = 'WINDOWS_PER_USER_INSTALLER'
    source_commit = $headSha
    runtime_identifier = 'win-x64'
    payload = [ordered]@{
        path = 'payload.zip'
        length = [long]$payloadEntry[0].length
        sha256 = [string]$payloadEntry[0].sha256
        manifest_schema = 'lattice.control.desktop-portable-candidate.v2'
    }
    install = [ordered]@{
        scope = 'CURRENT_USER'
        requires_elevation = $false
        root = '%LOCALAPPDATA%\Programs\LATTICE'
        versions = 'versions\<source-commit>'
        start_menu_shortcut = '%APPDATA%\Microsoft\Windows\Start Menu\Programs\LATTICE\LATTICE.lnk'
        uninstall_registry = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\LATTICE'
        uninstaller = 'Uninstall-LATTICE.ps1'
        preserves = @(
            '%LOCALAPPDATA%\LATTICE\control\lattice-control.db',
            '%LOCALAPPDATA%\LATTICE\ControlDesktop\WebView2')
    }
    files = $installerFiles
}
$installerManifestPath = Join-Path $installerDirectory 'install-manifest.json'
[IO.File]::WriteAllText(
    $installerManifestPath,
    (($installerManifest | ConvertTo-Json -Depth 7) + [Environment]::NewLine),
    [Text.UTF8Encoding]::new($false))
Compress-Archive `
    -Path (Join-Path $installerDirectory '*') `
    -DestinationPath $installerZipPath `
    -CompressionLevel Optimal
$installerArchiveSha256 = Get-Sha256Hex -LiteralPath $installerZipPath

[ordered]@{
    result = 'PASS'
    artifact_type = 'PORTABLE_RELEASE_CANDIDATE'
    source_commit = $headSha
    directory = $candidateDirectory
    archive = $zipPath
    archive_sha256 = $archiveSha256
    manifest = $manifestPath
    executable = $executablePath
    executable_sha256 = $manifest.executable_sha256
    control_runtime = $manifest.control_runtime
    installer = [ordered]@{
        artifact_type = 'WINDOWS_PER_USER_INSTALLER'
        schema_version = 'lattice.control.desktop-per-user-installer.v1'
        directory = $installerDirectory
        archive = $installerZipPath
        archive_sha256 = $installerArchiveSha256
        manifest = $installerManifestPath
        manifest_sha256 = Get-Sha256Hex -LiteralPath $installerManifestPath
        payload_archive_sha256 = $archiveSha256
        install_scope = 'CURRENT_USER'
        requires_elevation = $false
    }
} | ConvertTo-Json -Depth 4
