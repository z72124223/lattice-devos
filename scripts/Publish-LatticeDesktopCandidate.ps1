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

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$projectPath = Join-Path $repositoryRoot 'apps\lattice-control-desktop\Lattice.Control.Desktop.csproj'
$controlSourceRoot = Join-Path $repositoryRoot 'apps\lattice-control'
$runtimeIdentityPath = Join-Path $repositoryRoot 'apps\lattice-control\runtime-identity.json'
$nodePath = [IO.Path]::GetFullPath(
    (Get-Command node.exe -CommandType Application -ErrorAction Stop).Source)
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

$headSha = [string](& git -C $repositoryRoot rev-parse HEAD)
if ($LASTEXITCODE -ne 0 -or $headSha -notmatch '^[0-9a-f]{40}$') {
    throw 'DESKTOP_CANDIDATE_GIT_HEAD_UNAVAILABLE'
}
$headSha = $headSha.Trim()

$statusLines = @(& git -C $repositoryRoot status --porcelain --untracked-files=all)
if ($LASTEXITCODE -ne 0) {
    throw 'DESKTOP_CANDIDATE_GIT_STATUS_UNAVAILABLE'
}
$expectedProtectedDirtyState = ' M HANDOFF.md'
if ($statusLines.Count -ne 1 -or $statusLines[0] -cne $expectedProtectedDirtyState) {
    throw ('DESKTOP_CANDIDATE_SOURCE_NOT_COMMITTED: ' + ($statusLines -join ', '))
}
$stagedPaths = @(& git -C $repositoryRoot diff --cached --name-only)
if ($LASTEXITCODE -ne 0 -or $stagedPaths.Count -ne 0) {
    throw ('DESKTOP_CANDIDATE_STAGED_CHANGES_PRESENT: ' + ($stagedPaths -join ', '))
}

if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
    $localApplicationData = [Environment]::GetFolderPath(
        [Environment+SpecialFolder]::LocalApplicationData)
    $OutputRoot = Join-Path $localApplicationData 'LATTICE\candidates'
}

$outputRootFull = [IO.Path]::GetFullPath($OutputRoot)
$candidateName = 'lattice-control-desktop-win-x64-' + $headSha.Substring(0, 12)
$candidateDirectory = [IO.Path]::GetFullPath((Join-Path $outputRootFull $candidateName))
$zipPath = [IO.Path]::GetFullPath((Join-Path $outputRootFull ($candidateName + '.zip')))
$outputPrefix = $outputRootFull.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
if (-not $candidateDirectory.StartsWith($outputPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'DESKTOP_CANDIDATE_OUTPUT_ESCAPED_ROOT'
}

[IO.Directory]::CreateDirectory($outputRootFull) | Out-Null
if (Test-Path -LiteralPath $candidateDirectory) {
    Remove-Item -LiteralPath $candidateDirectory -Recurse -Force
}
if (Test-Path -LiteralPath $zipPath) {
    Remove-Item -LiteralPath $zipPath -Force
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
} | ConvertTo-Json -Depth 4
