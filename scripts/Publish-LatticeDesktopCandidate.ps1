[CmdletBinding()]
param(
    [string]$OutputRoot = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$projectPath = Join-Path $repositoryRoot 'apps\lattice-control-desktop\Lattice.Control.Desktop.csproj'

$headSha = [string](& git -C $repositoryRoot rev-parse HEAD)
if ($LASTEXITCODE -ne 0 -or $headSha -notmatch '^[0-9a-f]{40}$') {
    throw 'DESKTOP_CANDIDATE_GIT_HEAD_UNAVAILABLE'
}
$headSha = $headSha.Trim()

$statusLines = @(& git -C $repositoryRoot status --porcelain --untracked-files=all)
if ($LASTEXITCODE -ne 0) {
    throw 'DESKTOP_CANDIDATE_GIT_STATUS_UNAVAILABLE'
}
$unexpectedChanges = @($statusLines | Where-Object {
    $_.Length -lt 4 -or $_.Substring(3) -ne 'HANDOFF.md'
})
if ($unexpectedChanges.Count -ne 0) {
    throw ('DESKTOP_CANDIDATE_SOURCE_NOT_COMMITTED: ' + ($unexpectedChanges -join ', '))
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
            sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    })
if ($artifactFiles.Count -eq 0) {
    throw 'DESKTOP_CANDIDATE_FILE_SET_EMPTY'
}

$manifest = [ordered]@{
    schema_version = 'lattice.control.desktop-portable-candidate.v1'
    artifact_type = 'PORTABLE_RELEASE_CANDIDATE'
    source_commit = $headSha
    runtime_identifier = 'win-x64'
    self_contained = $true
    launch = 'LATTICE.exe'
    control_origin = 'http://127.0.0.1:4317/'
    webview_user_data = '%LOCALAPPDATA%\LATTICE\ControlDesktop\WebView2'
    executable_sha256 = (Get-FileHash -LiteralPath $executablePath -Algorithm SHA256).Hash.ToLowerInvariant()
    files = $artifactFiles
}
$manifestPath = Join-Path $candidateDirectory 'candidate-manifest.json'
[IO.File]::WriteAllText(
    $manifestPath,
    (($manifest | ConvertTo-Json -Depth 6) + [Environment]::NewLine),
    [Text.UTF8Encoding]::new($false))

Compress-Archive -Path (Join-Path $candidateDirectory '*') -DestinationPath $zipPath -CompressionLevel Optimal
$archiveSha256 = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()

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
} | ConvertTo-Json -Depth 4
