[CmdletBinding()]
param(
    [string]$OutputDirectory,
    [switch]$AllowDirty
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-Sha256Hex {
    param([Parameter(Mandatory)][string]$Path)

    $stream = [IO.File]::OpenRead($Path)
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha256.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha256.Dispose()
        $stream.Dispose()
    }
}

$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repository 'dist'
}
$outputRoot = [IO.Path]::GetFullPath($OutputDirectory)
if ($outputRoot -eq $repository) {
    throw 'ALPHA_OUTPUT_ROOT_REJECTED'
}

$dirtyPaths = @(& git -C $repository status --porcelain)
if ($LASTEXITCODE -ne 0) {
    throw 'ALPHA_GIT_STATUS_FAILED'
}
if ($dirtyPaths.Count -gt 0 -and -not $AllowDirty) {
    throw 'ALPHA_SOURCE_DIRTY'
}

$metadataText = (& cargo metadata --manifest-path (Join-Path $repository 'Cargo.toml') --format-version 1 --no-deps) -join "`n"
if ($LASTEXITCODE -ne 0) {
    throw 'ALPHA_CARGO_METADATA_FAILED'
}
$metadata = $metadataText | ConvertFrom-Json
$cliPackage = @($metadata.packages | Where-Object { $_.name -eq 'lattice-cli' })
if ($cliPackage.Count -ne 1) {
    throw 'ALPHA_CLI_PACKAGE_IDENTITY_REJECTED'
}
$version = [string]$cliPackage[0].version
if ($version -ne '0.1.0-alpha.1') {
    throw 'ALPHA_VERSION_REJECTED'
}

& cargo build --manifest-path (Join-Path $repository 'Cargo.toml') -p lattice-cli --release --locked
if ($LASTEXITCODE -ne 0) {
    throw 'ALPHA_BUILD_FAILED'
}

$binary = Join-Path $repository 'target\release\lattice.exe'
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
    throw 'ALPHA_BINARY_MISSING'
}
$reportedVersion = (& $binary version) -join "`n"
if ($LASTEXITCODE -ne 0 -or $reportedVersion -ne "LATTICE DevOS $version") {
    throw 'ALPHA_VERSION_SMOKE_FAILED'
}
$statusText = (& $binary status --json) -join "`n"
if ($LASTEXITCODE -ne 0) {
    throw 'ALPHA_STATUS_SMOKE_FAILED'
}
$status = $statusText | ConvertFrom-Json
if ($status.platform -ne 'LATTICE DevOS' -or $status.state -ne 'bootstrap-ready') {
    throw 'ALPHA_STATUS_IDENTITY_REJECTED'
}
$tiers = @($status.components | ForEach-Object { [string]$_.tier } | Sort-Object -Unique)
$tierDifferences = @(Compare-Object -ReferenceObject @('core', 'deferred', 'optional', 'retired') -DifferenceObject $tiers)
if ($tierDifferences.Count -ne 0) {
    throw 'ALPHA_STATUS_TIERS_REJECTED'
}

[IO.Directory]::CreateDirectory($outputRoot) | Out-Null
$packageName = "lattice-devos-$version-windows-x86_64"
$archive = Join-Path $outputRoot "$packageName.zip"
$checksumPath = "$archive.sha256"
if ((Test-Path -LiteralPath $archive) -or (Test-Path -LiteralPath $checksumPath)) {
    throw 'ALPHA_OUTPUT_ALREADY_EXISTS'
}

$temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$staging = [IO.Path]::GetFullPath((Join-Path $temporaryRoot ("lattice-alpha-" + [Guid]::NewGuid().ToString('N'))))
if (-not $staging.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'ALPHA_STAGING_ROOT_REJECTED'
}

try {
    $packageRoot = Join-Path $staging $packageName
    [IO.Directory]::CreateDirectory($packageRoot) | Out-Null
    Copy-Item -LiteralPath $binary -Destination (Join-Path $packageRoot 'lattice.exe')
    Copy-Item -LiteralPath (Join-Path $repository 'docs\releases\v0.1.0-alpha.1.md') -Destination (Join-Path $packageRoot 'README.md')

    $sourceCommit = (& git -C $repository rev-parse HEAD) -join ''
    if ($LASTEXITCODE -ne 0 -or $sourceCommit -notmatch '^[0-9a-f]{40}$') {
        throw 'ALPHA_SOURCE_COMMIT_REJECTED'
    }
    $binaryHash = Get-Sha256Hex -Path (Join-Path $packageRoot 'lattice.exe')
    $manifest = [ordered]@{
        product = 'LATTICE DevOS'
        version = $version
        channel = 'developer-preview'
        target = 'windows-x86_64'
        source_commit = $sourceCommit
        source_dirty = ($dirtyPaths.Count -gt 0)
        binary = 'lattice.exe'
        binary_sha256 = $binaryHash
        capabilities = @('version', 'status', 'status-json')
    }
    $manifestJson = $manifest | ConvertTo-Json -Depth 4
    [IO.File]::WriteAllText(
        (Join-Path $packageRoot 'manifest.json'),
        $manifestJson,
        [Text.UTF8Encoding]::new($false)
    )

    Compress-Archive -LiteralPath $packageRoot -DestinationPath $archive -CompressionLevel Optimal
    $archiveHash = Get-Sha256Hex -Path $archive
    "$archiveHash  $([IO.Path]::GetFileName($archive))" | Set-Content -LiteralPath $checksumPath -Encoding ascii
}
finally {
    if (Test-Path -LiteralPath $staging) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
}

Write-Output 'LATTICE_ALPHA_PACKAGE_READY=1'
Write-Output "version=$version"
Write-Output "archive=$archive"
Write-Output "checksum=$checksumPath"
