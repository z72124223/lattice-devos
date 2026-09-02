Set-StrictMode -Version Latest

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$script:LatticeInstallOwnerSchema = 'lattice.control.desktop-install-owner.v1'
$script:LatticeInstallManifestSchema = 'lattice.control.desktop-per-user-installer.v1'
$script:LatticeInstallReceiptSchema = 'lattice.control.desktop-install-receipt.v1'
$script:LatticeActiveInstallSchema = 'lattice.control.desktop-active-install.v1'
$script:LatticeStageOwnerSchema = 'lattice.control.desktop-stage-owner.v1'
$script:LatticeActivationJournalSchema = 'lattice.control.desktop-activation-journal.v1'
$script:LatticeUninstallJournalSchema = 'lattice.control.desktop-uninstall-journal.v1'
$script:LatticeProduct = 'LATTICE_CONTROL_DESKTOP'
$script:LatticeRegistryValueNames = @(
    'DisplayName',
    'DisplayVersion',
    'Publisher',
    'InstallLocation',
    'DisplayIcon',
    'UninstallString',
    'QuietUninstallString',
    'NoModify',
    'NoRepair',
    'EstimatedSize',
    'InstallDate',
    'LatticeProduct',
    'LatticeInstallId',
    'LatticeSourceCommit')

function Get-LatticeSha256Hex {
    param([Parameter(Mandatory)][string]$LiteralPath)

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

function Get-LatticeStringSha256Hex {
    param([Parameter(Mandatory)][string]$Value)

    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes($Value)
        return ([BitConverter]::ToString($algorithm.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $algorithm.Dispose()
    }
}

function Get-LatticeRequiredProperty {
    param(
        [Parameter(Mandatory)][object]$InputObject,
        [Parameter(Mandatory)][string]$Name,
        [string]$ErrorCode = 'LATTICE_INSTALL_MANIFEST_PROPERTY_MISSING'
    )

    $property = $InputObject.PSObject.Properties[$Name]
    if ($null -eq $property) {
        throw "${ErrorCode}:$Name"
    }
    return $property.Value
}

function Test-LatticePathWithinRoot {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$Root
    )

    $fullPath = [IO.Path]::GetFullPath($Path)
    $fullRoot = [IO.Path]::GetFullPath($Root).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar)
    $prefix = $fullRoot + [IO.Path]::DirectorySeparatorChar
    return $fullPath.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
}

function Assert-LatticeNotReparsePoint {
    param(
        [Parameter(Mandatory)][string]$LiteralPath,
        [string]$ErrorCode = 'LATTICE_INSTALL_REPARSE_POINT_REJECTED'
    )

    if (-not (Test-Path -LiteralPath $LiteralPath)) {
        return
    }
    $item = Get-Item -LiteralPath $LiteralPath -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "${ErrorCode}:$LiteralPath"
    }
}

function Assert-LatticePathAncestorsHaveNoReparsePoints {
    param(
        [Parameter(Mandatory)][string]$LiteralPath,
        [string]$ErrorCode = 'LATTICE_INSTALL_REPARSE_POINT_REJECTED'
    )

    $fullPath = [IO.Path]::GetFullPath($LiteralPath)
    $volumeRoot = [IO.Path]::GetPathRoot($fullPath)
    $current = $volumeRoot.TrimEnd([IO.Path]::DirectorySeparatorChar)
    foreach ($segment in $fullPath.Substring($volumeRoot.Length).Split(
        [IO.Path]::DirectorySeparatorChar,
        [StringSplitOptions]::RemoveEmptyEntries)) {
        $current = Join-Path $current $segment
        if (-not (Test-Path -LiteralPath $current)) {
            break
        }
        Assert-LatticeNotReparsePoint -LiteralPath $current -ErrorCode $ErrorCode
    }
}

function Assert-LatticeTreeHasNoReparsePoints {
    param(
        [Parameter(Mandatory)][string]$Root,
        [string]$ErrorCode = 'LATTICE_INSTALL_REPARSE_POINT_REJECTED'
    )

    Assert-LatticeNotReparsePoint -LiteralPath $Root -ErrorCode $ErrorCode
    $pending = [Collections.Generic.Queue[string]]::new()
    $pending.Enqueue([IO.Path]::GetFullPath($Root))
    while ($pending.Count -gt 0) {
        $directory = $pending.Dequeue()
        foreach ($item in @(Get-ChildItem -LiteralPath $directory -Force)) {
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "${ErrorCode}:$($item.FullName)"
            }
            if ($item.PSIsContainer) {
                $pending.Enqueue($item.FullName)
            }
        }
    }
}

function Assert-LatticeTreeHasNoAlternateDataStreams {
    param(
        [Parameter(Mandatory)][string]$Root,
        [string]$ErrorCode = 'LATTICE_INSTALL_ALTERNATE_DATA_STREAM_REJECTED'
    )

    foreach ($file in @(Get-ChildItem -LiteralPath $Root -Force -File -Recurse)) {
        foreach ($stream in @(Get-Item -LiteralPath $file.FullName -Stream *)) {
            if ([string]$stream.Stream -cne ':$DATA') {
                throw "${ErrorCode}:$($file.FullName):$($stream.Stream)"
            }
        }
    }
}

function Assert-LatticeSafeRelativePath {
    param([Parameter(Mandatory)][string]$RelativePath)

    if ([string]::IsNullOrWhiteSpace($RelativePath) -or
        [IO.Path]::IsPathRooted($RelativePath) -or
        $RelativePath.Contains('\') -or
        $RelativePath.Contains(':') -or
        $RelativePath -match '(^|/)\.\.?(/|$)') {
        throw "LATTICE_INSTALL_MANIFEST_PATH_INVALID:$RelativePath"
    }
}

function Write-LatticeJsonAtomic {
    param(
        [Parameter(Mandatory)][string]$LiteralPath,
        [Parameter(Mandatory)][object]$Value
    )

    $fullPath = [IO.Path]::GetFullPath($LiteralPath)
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporaryPath = Join-Path $parent ('.lattice-' + [guid]::NewGuid().ToString('N') + '.tmp')
    try {
        [IO.File]::WriteAllText(
            $temporaryPath,
            (($Value | ConvertTo-Json -Depth 10) + [Environment]::NewLine),
            [Text.UTF8Encoding]::new($false))
        if (Test-Path -LiteralPath $fullPath -PathType Leaf) {
            $backupPath = Join-Path $parent ('.lattice-' + [guid]::NewGuid().ToString('N') + '.bak')
            try {
                [IO.File]::Replace($temporaryPath, $fullPath, $backupPath)
            }
            finally {
                if (Test-Path -LiteralPath $backupPath) {
                    Remove-Item -LiteralPath $backupPath -Force
                }
            }
        }
        else {
            [IO.File]::Move($temporaryPath, $fullPath)
        }
    }
    finally {
        if (Test-Path -LiteralPath $temporaryPath) {
            Remove-Item -LiteralPath $temporaryPath -Force
        }
    }
}

function Get-LatticeInstallManifest {
    param([Parameter(Mandatory)][string]$ManifestPath)

    try {
        $manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
    }
    catch {
        throw 'LATTICE_INSTALL_MANIFEST_INVALID_JSON'
    }
    $schema = [string](Get-LatticeRequiredProperty $manifest 'schema_version')
    $artifactType = [string](Get-LatticeRequiredProperty $manifest 'artifact_type')
    $sourceCommit = [string](Get-LatticeRequiredProperty $manifest 'source_commit')
    $runtimeIdentifier = [string](Get-LatticeRequiredProperty $manifest 'runtime_identifier')
    $payload = Get-LatticeRequiredProperty $manifest 'payload'
    $install = Get-LatticeRequiredProperty $manifest 'install'
    if ($schema -cne $script:LatticeInstallManifestSchema -or
        $artifactType -cne 'WINDOWS_PER_USER_INSTALLER' -or
        $sourceCommit -cnotmatch '^[0-9a-f]{40}$' -or
        $runtimeIdentifier -cne 'win-x64' -or
        [string](Get-LatticeRequiredProperty $payload 'path') -cne 'payload.zip' -or
        [string](Get-LatticeRequiredProperty $payload 'sha256') -cnotmatch '^[0-9a-f]{64}$' -or
        [string](Get-LatticeRequiredProperty $payload 'manifest_schema') -cne 'lattice.control.desktop-portable-candidate.v2' -or
        [string](Get-LatticeRequiredProperty $install 'scope') -cne 'CURRENT_USER' -or
        [bool](Get-LatticeRequiredProperty $install 'requires_elevation') -or
        [string](Get-LatticeRequiredProperty $install 'root') -cne '%LOCALAPPDATA%\Programs\LATTICE' -or
        [string](Get-LatticeRequiredProperty $install 'start_menu_shortcut') -cne '%APPDATA%\Microsoft\Windows\Start Menu\Programs\LATTICE\LATTICE.lnk' -or
        [string](Get-LatticeRequiredProperty $install 'uninstall_registry') -cne 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\LATTICE' -or
        [string](Get-LatticeRequiredProperty $install 'uninstaller') -cne 'Uninstall-LATTICE.ps1') {
        throw 'LATTICE_INSTALL_MANIFEST_CONTRACT_MISMATCH'
    }
    return $manifest
}

function Get-LatticeInstallerBundle {
    param([Parameter(Mandatory)][string]$BundleRoot)

    $bundleRootFull = [IO.Path]::GetFullPath($BundleRoot)
    Assert-LatticeTreeHasNoReparsePoints -Root $bundleRootFull -ErrorCode 'LATTICE_INSTALL_BUNDLE_REPARSE_POINT_REJECTED'
    $manifestPath = Join-Path $bundleRootFull 'install-manifest.json'
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw 'LATTICE_INSTALL_MANIFEST_MISSING'
    }
    $manifest = Get-LatticeInstallManifest -ManifestPath $manifestPath
    $entries = @(Get-LatticeRequiredProperty $manifest 'files')
    if ($entries.Count -lt 5 -or $entries.Count -gt 32) {
        throw 'LATTICE_INSTALL_MANIFEST_FILE_SET_INVALID'
    }
    $expected = @{}
    foreach ($entry in $entries) {
        $relativePath = [string](Get-LatticeRequiredProperty $entry 'path')
        Assert-LatticeSafeRelativePath -RelativePath $relativePath
        $lengthText = [string](Get-LatticeRequiredProperty $entry 'length')
        $sha256 = [string](Get-LatticeRequiredProperty $entry 'sha256')
        if ($relativePath -ceq 'install-manifest.json' -or
            $lengthText -notmatch '^(0|[1-9][0-9]*)$' -or
            $sha256 -cnotmatch '^[0-9a-f]{64}$' -or
            $expected.ContainsKey($relativePath)) {
            throw "LATTICE_INSTALL_MANIFEST_FILE_ENTRY_INVALID:$relativePath"
        }
        $expected[$relativePath] = [PSCustomObject]@{
            Length = [long]::Parse($lengthText, [Globalization.CultureInfo]::InvariantCulture)
            Sha256 = $sha256
        }
    }
    foreach ($requiredPath in @(
        'payload.zip',
        'Install-LATTICE.ps1',
        'Uninstall-LATTICE.ps1',
        'LatticeDesktopInstaller.Common.ps1',
        'INSTALL-LATTICE.txt')) {
        if (-not $expected.ContainsKey($requiredPath)) {
            throw "LATTICE_INSTALL_BUNDLE_REQUIRED_FILE_MISSING:$requiredPath"
        }
    }
    $reparsePoints = @(Get-ChildItem -LiteralPath $bundleRootFull -Force -Recurse |
        Where-Object { ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 })
    if ($reparsePoints.Count -ne 0) {
        throw 'LATTICE_INSTALL_BUNDLE_REPARSE_POINT_REJECTED'
    }
    $actualFiles = @(Get-ChildItem -LiteralPath $bundleRootFull -Force -File -Recurse |
        Where-Object { $_.FullName -cne $manifestPath })
    if ($actualFiles.Count -ne $expected.Count) {
        throw 'LATTICE_INSTALL_BUNDLE_FILE_COUNT_MISMATCH'
    }
    foreach ($actualFile in $actualFiles) {
        $relativePath = $actualFile.FullName.Substring($bundleRootFull.Length + 1).Replace('\', '/')
        if (-not $expected.ContainsKey($relativePath)) {
            throw "LATTICE_INSTALL_BUNDLE_UNDECLARED_FILE:$relativePath"
        }
        $entry = $expected[$relativePath]
        if ($actualFile.Length -ne $entry.Length -or
            (Get-LatticeSha256Hex -LiteralPath $actualFile.FullName) -cne $entry.Sha256) {
            throw "LATTICE_INSTALL_BUNDLE_FILE_MISMATCH:$relativePath"
        }
    }
    $payload = Get-LatticeRequiredProperty $manifest 'payload'
    if ($expected['payload.zip'].Length -ne [long](Get-LatticeRequiredProperty $payload 'length') -or
        $expected['payload.zip'].Sha256 -cne [string](Get-LatticeRequiredProperty $payload 'sha256')) {
        throw 'LATTICE_INSTALL_BUNDLE_PAYLOAD_METADATA_MISMATCH'
    }
    return [PSCustomObject]@{
        Root = $bundleRootFull
        Manifest = $manifest
        ManifestPath = $manifestPath
        ManifestSha256 = Get-LatticeSha256Hex -LiteralPath $manifestPath
        SourceCommit = [string]$manifest.source_commit
        PayloadPath = Join-Path $bundleRootFull 'payload.zip'
        PayloadSha256 = [string]$payload.sha256
        Files = $expected
    }
}

function Expand-LatticePortablePayload {
    param(
        [Parameter(Mandatory)][string]$ArchivePath,
        [Parameter(Mandatory)][string]$DestinationPath,
        [Parameter(Mandatory)][string]$ExpectedSourceCommit,
        [Parameter(Mandatory)][string]$FinalPayloadPath
    )

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $destinationFull = [IO.Path]::GetFullPath($DestinationPath)
    $finalPayloadFull = [IO.Path]::GetFullPath($FinalPayloadPath)
    $archive = [IO.Compression.ZipFile]::OpenRead([IO.Path]::GetFullPath($ArchivePath))
    try {
        $seen = @{}
        $extractionPlan = @()
        foreach ($entry in $archive.Entries) {
            $entryPath = [string]$entry.FullName
            if ([string]::IsNullOrWhiteSpace($entryPath)) {
                continue
            }
            $canonicalEntryPath = $entryPath.Replace('\', '/')
            if ($canonicalEntryPath.Contains(':') -or
                $canonicalEntryPath.StartsWith('/', [StringComparison]::Ordinal) -or
                $canonicalEntryPath -match '(^|/)\.\.?(/|$)') {
                throw "LATTICE_INSTALL_PAYLOAD_ARCHIVE_PATH_INVALID:$entryPath"
            }
            $normalized = $canonicalEntryPath.TrimEnd('/')
            if ([string]::IsNullOrWhiteSpace($normalized)) {
                throw "LATTICE_INSTALL_PAYLOAD_ARCHIVE_PATH_INVALID:$entryPath"
            }
            if ($seen.ContainsKey($normalized)) {
                throw "LATTICE_INSTALL_PAYLOAD_ARCHIVE_DUPLICATE:$normalized"
            }
            $seen[$normalized] = $true
            $target = [IO.Path]::GetFullPath((Join-Path $destinationFull $normalized.Replace('/', '\')))
            if (-not (Test-LatticePathWithinRoot -Path $target -Root $destinationFull)) {
                throw "LATTICE_INSTALL_PAYLOAD_ARCHIVE_ESCAPED_ROOT:$entryPath"
            }
            if (($entry.ExternalAttributes -band [int][IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "LATTICE_INSTALL_PAYLOAD_ARCHIVE_REPARSE_POINT:$entryPath"
            }
            $isDirectory = $canonicalEntryPath.EndsWith('/', [StringComparison]::Ordinal)
            $finalTarget = [IO.Path]::GetFullPath((Join-Path $finalPayloadFull $normalized.Replace('/', '\')))
            if (-not (Test-LatticePathWithinRoot -Path $finalTarget -Root $finalPayloadFull)) {
                throw "LATTICE_INSTALL_PAYLOAD_ARCHIVE_ESCAPED_FINAL_ROOT:$entryPath"
            }
            foreach ($candidate in @($target, $finalTarget)) {
                $candidateDirectory = if ($isDirectory) { $candidate } else { [IO.Path]::GetDirectoryName($candidate) }
                if ($candidate.Length -ge 260 -or $candidateDirectory.Length -ge 248) {
                    throw "LATTICE_INSTALL_PAYLOAD_PATH_BUDGET_EXCEEDED:$entryPath"
                }
            }
            $extractionPlan += [PSCustomObject]@{
                Entry = $entry
                Target = $target
                IsDirectory = $isDirectory
            }
        }

        [IO.Directory]::CreateDirectory($destinationFull) | Out-Null
        foreach ($planned in $extractionPlan) {
            if ([bool]$planned.IsDirectory) {
                [IO.Directory]::CreateDirectory([string]$planned.Target) | Out-Null
                continue
            }
            [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName([string]$planned.Target)) | Out-Null
            $input = ([IO.Compression.ZipArchiveEntry]$planned.Entry).Open()
            try {
                $output = [IO.File]::Open(
                    [string]$planned.Target,
                    [IO.FileMode]::CreateNew,
                    [IO.FileAccess]::Write,
                    [IO.FileShare]::None)
                try {
                    $input.CopyTo($output)
                }
                finally { $output.Dispose() }
            }
            finally { $input.Dispose() }
        }
    }
    finally {
        $archive.Dispose()
    }
    return Get-LatticePortablePayload -PayloadRoot $destinationFull -ExpectedSourceCommit $ExpectedSourceCommit
}

function Get-LatticePortablePayload {
    param(
        [Parameter(Mandatory)][string]$PayloadRoot,
        [Parameter(Mandatory)][string]$ExpectedSourceCommit
    )

    $payloadRootFull = [IO.Path]::GetFullPath($PayloadRoot)
    Assert-LatticeTreeHasNoReparsePoints -Root $payloadRootFull -ErrorCode 'LATTICE_INSTALL_PAYLOAD_REPARSE_POINT_REJECTED'
    Assert-LatticeTreeHasNoAlternateDataStreams -Root $payloadRootFull -ErrorCode 'LATTICE_INSTALL_PAYLOAD_ALTERNATE_DATA_STREAM_REJECTED'
    $manifestPath = Join-Path $payloadRootFull 'candidate-manifest.json'
    try {
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    }
    catch {
        throw 'LATTICE_INSTALL_PAYLOAD_MANIFEST_INVALID_JSON'
    }
    if ([string](Get-LatticeRequiredProperty $manifest 'schema_version') -cne 'lattice.control.desktop-portable-candidate.v2' -or
        [string](Get-LatticeRequiredProperty $manifest 'artifact_type') -cne 'PORTABLE_RELEASE_CANDIDATE' -or
        [string](Get-LatticeRequiredProperty $manifest 'source_commit') -cne $ExpectedSourceCommit -or
        [string](Get-LatticeRequiredProperty $manifest 'runtime_identifier') -cne 'win-x64' -or
        [string](Get-LatticeRequiredProperty $manifest 'launch') -cne 'LATTICE.exe') {
        throw 'LATTICE_INSTALL_PAYLOAD_MANIFEST_CONTRACT_MISMATCH'
    }
    $entries = @(Get-LatticeRequiredProperty $manifest 'files')
    if ($entries.Count -eq 0 -or $entries.Count -gt 4096) {
        throw 'LATTICE_INSTALL_PAYLOAD_FILE_SET_INVALID'
    }
    $expected = @{}
    foreach ($entry in $entries) {
        $relativePath = [string](Get-LatticeRequiredProperty $entry 'path')
        Assert-LatticeSafeRelativePath -RelativePath $relativePath
        $lengthText = [string](Get-LatticeRequiredProperty $entry 'length')
        $sha256 = [string](Get-LatticeRequiredProperty $entry 'sha256')
        if ($relativePath -ceq 'candidate-manifest.json' -or
            $lengthText -notmatch '^(0|[1-9][0-9]*)$' -or
            $sha256 -cnotmatch '^[0-9a-f]{64}$' -or
            $expected.ContainsKey($relativePath)) {
            throw "LATTICE_INSTALL_PAYLOAD_FILE_ENTRY_INVALID:$relativePath"
        }
        $expected[$relativePath] = [PSCustomObject]@{
            Length = [long]::Parse($lengthText, [Globalization.CultureInfo]::InvariantCulture)
            Sha256 = $sha256
        }
    }
    foreach ($requiredPath in @('LATTICE.exe', 'LATTICE.dll', 'PORTABLE_RELEASE_CANDIDATE.txt')) {
        if (-not $expected.ContainsKey($requiredPath)) {
            throw "LATTICE_INSTALL_PAYLOAD_REQUIRED_FILE_MISSING:$requiredPath"
        }
    }
    $expectedDirectories = @{}
    foreach ($relativePath in $expected.Keys) {
        $directory = [IO.Path]::GetDirectoryName($relativePath.Replace('/', '\'))
        while (-not [string]::IsNullOrWhiteSpace($directory)) {
            $expectedDirectories[$directory.Replace('\', '/')] = $true
            $directory = [IO.Path]::GetDirectoryName($directory)
        }
    }
    $actualDirectories = @(Get-ChildItem -LiteralPath $payloadRootFull -Force -Directory -Recurse)
    if ($actualDirectories.Count -ne $expectedDirectories.Count) {
        throw 'LATTICE_INSTALL_PAYLOAD_DIRECTORY_SET_MISMATCH'
    }
    foreach ($actualDirectory in $actualDirectories) {
        $relativeDirectory = $actualDirectory.FullName.Substring($payloadRootFull.Length + 1).Replace('\', '/')
        if (-not $expectedDirectories.ContainsKey($relativeDirectory)) {
            throw "LATTICE_INSTALL_PAYLOAD_UNDECLARED_DIRECTORY:$relativeDirectory"
        }
    }
    $reparsePoints = @(Get-ChildItem -LiteralPath $payloadRootFull -Force -Recurse |
        Where-Object { ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 })
    if ($reparsePoints.Count -ne 0) {
        throw 'LATTICE_INSTALL_PAYLOAD_REPARSE_POINT_REJECTED'
    }
    $actualFiles = @(Get-ChildItem -LiteralPath $payloadRootFull -Force -File -Recurse |
        Where-Object { $_.FullName -cne $manifestPath })
    if ($actualFiles.Count -ne $expected.Count) {
        throw 'LATTICE_INSTALL_PAYLOAD_FILE_COUNT_MISMATCH'
    }
    foreach ($actualFile in $actualFiles) {
        $relativePath = $actualFile.FullName.Substring($payloadRootFull.Length + 1).Replace('\', '/')
        if (-not $expected.ContainsKey($relativePath)) {
            throw "LATTICE_INSTALL_PAYLOAD_UNDECLARED_FILE:$relativePath"
        }
        $entry = $expected[$relativePath]
        if ($actualFile.Length -ne $entry.Length -or
            (Get-LatticeSha256Hex -LiteralPath $actualFile.FullName) -cne $entry.Sha256) {
            throw "LATTICE_INSTALL_PAYLOAD_FILE_MISMATCH:$relativePath"
        }
    }
    return [PSCustomObject]@{
        Root = $payloadRootFull
        Manifest = $manifest
        ManifestPath = $manifestPath
        ManifestSha256 = Get-LatticeSha256Hex -LiteralPath $manifestPath
        FileCount = $expected.Count
    }
}

function Resolve-LatticeInstallContext {
    param(
        [string]$TestSandboxRoot = '',
        [string]$TestRegistryId = ''
    )

    if ([string]::IsNullOrWhiteSpace($TestSandboxRoot)) {
        if (-not [string]::IsNullOrWhiteSpace($TestRegistryId)) {
            throw 'LATTICE_INSTALL_TEST_SCOPE_INCOMPLETE'
        }
        $localApplicationData = $env:LOCALAPPDATA
        $roamingApplicationData = $env:APPDATA
        if ([string]::IsNullOrWhiteSpace($localApplicationData)) {
            $localApplicationData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
        }
        if ([string]::IsNullOrWhiteSpace($roamingApplicationData)) {
            $roamingApplicationData = [Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)
        }
        return [PSCustomObject]@{
            TestMode = $false
            SandboxRoot = $null
            InstallRoot = [IO.Path]::GetFullPath((Join-Path $localApplicationData 'Programs\LATTICE'))
            StartMenuProgramsRoot = [IO.Path]::GetFullPath((Join-Path $roamingApplicationData 'Microsoft\Windows\Start Menu\Programs'))
            RegistryPath = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\LATTICE'
            RegistryTestRoot = $null
        }
    }
    if ([string]::IsNullOrWhiteSpace($TestRegistryId) -or $TestRegistryId -cnotmatch '^[0-9a-f]{32}$') {
        throw 'LATTICE_INSTALL_TEST_REGISTRY_ID_INVALID'
    }
    $sandboxFull = [IO.Path]::GetFullPath($TestSandboxRoot)
    $temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
        [IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    if (-not $sandboxFull.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase) -or
        -not [IO.Path]::GetFileName($sandboxFull).StartsWith('lattice-desktop-installer-', [StringComparison]::Ordinal)) {
        throw 'LATTICE_INSTALL_TEST_SANDBOX_INVALID'
    }
    Assert-LatticePathAncestorsHaveNoReparsePoints `
        -LiteralPath $sandboxFull `
        -ErrorCode 'LATTICE_INSTALL_TEST_SANDBOX_REPARSE_POINT'
    $registryTestRoot = "HKCU:\Software\LATTICE\InstallerTests\$TestRegistryId"
    return [PSCustomObject]@{
        TestMode = $true
        SandboxRoot = $sandboxFull
        InstallRoot = [IO.Path]::GetFullPath((Join-Path $sandboxFull 'LocalAppData\Programs\LATTICE'))
        StartMenuProgramsRoot = [IO.Path]::GetFullPath((Join-Path $sandboxFull 'RoamingAppData\Microsoft\Windows\Start Menu\Programs'))
        RegistryPath = "$registryTestRoot\Uninstall\LATTICE"
        RegistryTestRoot = $registryTestRoot
    }
}

function Get-LatticeInstallPaths {
    param([Parameter(Mandatory)][object]$Context)

    $installRoot = [IO.Path]::GetFullPath([string]$Context.InstallRoot)
    if ([IO.Path]::GetFileName($installRoot) -cne 'LATTICE') {
        throw 'LATTICE_INSTALL_ROOT_INVALID'
    }
    return [PSCustomObject]@{
        InstallRoot = $installRoot
        OwnerMarker = Join-Path $installRoot 'install-owner.json'
        ActiveInstall = Join-Path $installRoot 'active-install.json'
        VersionsRoot = Join-Path $installRoot 'versions'
        StagingRoot = Join-Path $installRoot '.staging'
        ShortcutDirectory = Join-Path ([string]$Context.StartMenuProgramsRoot) 'LATTICE'
        ShortcutPath = Join-Path ([string]$Context.StartMenuProgramsRoot) 'LATTICE\LATTICE.lnk'
        RegistryPath = [string]$Context.RegistryPath
    }
}

function Initialize-LatticeOwnedInstallRoot {
    param([Parameter(Mandatory)][object]$Paths)

    $root = [string]$Paths.InstallRoot
    $markerPath = [string]$Paths.OwnerMarker
    $pendingMarkerPath = Join-Path ([IO.Path]::GetDirectoryName($root)) '.LATTICE.install-owner.pending.json'
    if (Test-Path -LiteralPath $root -PathType Container) {
        Assert-LatticeNotReparsePoint -LiteralPath $root
        if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) {
            $entries = @(Get-ChildItem -LiteralPath $root -Force)
            if ($entries.Count -ne 0) {
                throw 'LATTICE_INSTALL_ROOT_NOT_OWNED'
            }
        }
    }
    else {
        [IO.Directory]::CreateDirectory($root) | Out-Null
    }
    if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) {
        if (Test-Path -LiteralPath $pendingMarkerPath -PathType Leaf) {
            try {
                $marker = Get-Content -LiteralPath $pendingMarkerPath -Raw | ConvertFrom-Json
            }
            catch {
                throw 'LATTICE_INSTALL_OWNER_PENDING_INVALID_JSON'
            }
        }
        else {
            $marker = [ordered]@{
                schema_version = $script:LatticeInstallOwnerSchema
                product = $script:LatticeProduct
                install_id = [guid]::NewGuid().ToString('N')
                owner_sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
                install_root = $root
            }
            Write-LatticeJsonAtomic -LiteralPath $pendingMarkerPath -Value $marker
        }
        if ([string]$marker.schema_version -cne $script:LatticeInstallOwnerSchema -or
            [string]$marker.product -cne $script:LatticeProduct -or
            [string]$marker.install_id -cnotmatch '^[0-9a-f]{32}$' -or
            -not [string]::Equals([string]$marker.install_root, $root, [StringComparison]::OrdinalIgnoreCase) -or
            [string]$marker.owner_sid -cne [Security.Principal.WindowsIdentity]::GetCurrent().User.Value) {
            throw 'LATTICE_INSTALL_OWNER_PENDING_MISMATCH'
        }
        Invoke-LatticeInstallerHook -Name 'AfterOwnerMarkerPrepared'
        [IO.File]::Move($pendingMarkerPath, $markerPath)
    }
    elseif (Test-Path -LiteralPath $pendingMarkerPath) {
        throw 'LATTICE_INSTALL_OWNER_PENDING_CONFLICT'
    }
    Assert-LatticeNotReparsePoint -LiteralPath $markerPath
    try {
        $existing = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
    }
    catch {
        throw 'LATTICE_INSTALL_OWNER_MARKER_INVALID_JSON'
    }
    if ([string]$existing.schema_version -cne $script:LatticeInstallOwnerSchema -or
        [string]$existing.product -cne $script:LatticeProduct -or
        [string]$existing.install_id -cnotmatch '^[0-9a-f]{32}$' -or
        -not [string]::Equals([string]$existing.install_root, $root, [StringComparison]::OrdinalIgnoreCase) -or
        [string]$existing.owner_sid -cne [Security.Principal.WindowsIdentity]::GetCurrent().User.Value) {
        throw 'LATTICE_INSTALL_OWNER_MARKER_MISMATCH'
    }
    return $existing
}

function Initialize-LatticeInstallDirectories {
    param([Parameter(Mandatory)][object]$Paths)

    [IO.Directory]::CreateDirectory([string]$Paths.VersionsRoot) | Out-Null
    [IO.Directory]::CreateDirectory([string]$Paths.StagingRoot) | Out-Null
    Assert-LatticeNotReparsePoint -LiteralPath ([string]$Paths.VersionsRoot)
    Assert-LatticeNotReparsePoint -LiteralPath ([string]$Paths.StagingRoot)
}

function Write-LatticeStageOwner {
    param(
        [Parameter(Mandatory)][string]$StageRoot,
        [Parameter(Mandatory)][object]$Owner,
        [Parameter(Mandatory)][ValidateSet('INSTALL', 'ACTIVATION_BACKUP', 'UNINSTALL_BACKUP')][string]$Purpose,
        [string]$SourceCommit = ''
    )

    $marker = [ordered]@{
        schema_version = $script:LatticeStageOwnerSchema
        product = $script:LatticeProduct
        install_id = [string]$Owner.install_id
        purpose = $Purpose
        source_commit = $SourceCommit
    }
    Write-LatticeJsonAtomic -LiteralPath (Join-Path $StageRoot '.lattice-stage-owner.json') -Value $marker
}

function Clear-LatticeOwnedStaging {
    param(
        [Parameter(Mandatory)][object]$Paths,
        [Parameter(Mandatory)][object]$Owner
    )

    if (-not (Test-Path -LiteralPath ([string]$Paths.StagingRoot) -PathType Container)) {
        return $false
    }
    Assert-LatticeNotReparsePoint -LiteralPath ([string]$Paths.StagingRoot)
    foreach ($entry in @(Get-ChildItem -LiteralPath ([string]$Paths.StagingRoot) -Force)) {
        if (-not $entry.PSIsContainer -or
            ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "LATTICE_INSTALL_STAGING_ENTRY_NOT_OWNED:$($entry.Name)"
        }
        $markerPath = Join-Path $entry.FullName '.lattice-stage-owner.json'
        try {
            $marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
        }
        catch {
            throw "LATTICE_INSTALL_STAGING_MARKER_INVALID:$($entry.Name)"
        }
        if ([string]$marker.schema_version -cne $script:LatticeStageOwnerSchema -or
            [string]$marker.product -cne $script:LatticeProduct -or
            [string]$marker.install_id -cne [string]$Owner.install_id -or
            [string]$marker.purpose -cnotmatch '^(INSTALL|ACTIVATION_BACKUP|UNINSTALL_BACKUP)$') {
            throw "LATTICE_INSTALL_STAGING_MARKER_MISMATCH:$($entry.Name)"
        }
        Assert-LatticeTreeHasNoReparsePoints `
            -Root $entry.FullName `
            -ErrorCode 'LATTICE_INSTALL_STAGING_REPARSE_POINT_REJECTED'
        if ([string]$marker.purpose -ceq 'ACTIVATION_BACKUP') {
            Restore-LatticeActivationBackup `
                -Paths $Paths `
                -Owner $Owner `
                -BackupRoot $entry.FullName `
                -StageMarker $marker
        }
        elseif ([string]$marker.purpose -ceq 'UNINSTALL_BACKUP') {
            Complete-LatticeInterruptedUninstall `
                -Paths $Paths `
                -Owner $Owner `
                -BackupRoot $entry.FullName `
                -StageMarker $marker
            return $true
        }
        Remove-Item -LiteralPath $entry.FullName -Recurse -Force
    }
    return $false
}

function Get-LatticeRegistrySnapshot {
    param([Parameter(Mandatory)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return [PSCustomObject]@{ Exists = $false; Values = @() }
    }
    $key = Get-Item -LiteralPath $Path
    if (@($key.GetSubKeyNames()).Count -ne 0) {
        throw 'LATTICE_INSTALL_REGISTRY_UNKNOWN_SUBKEY'
    }
    $unknownValueNames = @($key.GetValueNames() | Where-Object { $script:LatticeRegistryValueNames -cnotcontains $_ })
    if ($unknownValueNames.Count -ne 0) {
        throw "LATTICE_INSTALL_REGISTRY_UNKNOWN_VALUE:$($unknownValueNames[0])"
    }
    $values = @()
    foreach ($name in @($key.GetValueNames() | Sort-Object)) {
        $values += [PSCustomObject]@{
            Name = $name
            Value = $key.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            Kind = [string]$key.GetValueKind($name)
        }
    }
    return [PSCustomObject]@{ Exists = $true; Values = $values }
}

function Restore-LatticeRegistrySnapshot {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][object]$Snapshot
    )

    if (Test-Path -LiteralPath $Path) {
        $current = Get-Item -LiteralPath $Path
        if (@($current.GetSubKeyNames()).Count -ne 0) {
            throw 'LATTICE_INSTALL_REGISTRY_UNKNOWN_SUBKEY'
        }
        $unknownValueNames = @($current.GetValueNames() | Where-Object { $script:LatticeRegistryValueNames -cnotcontains $_ })
        if ($unknownValueNames.Count -ne 0) {
            throw "LATTICE_INSTALL_REGISTRY_UNKNOWN_VALUE:$($unknownValueNames[0])"
        }
        Remove-Item -LiteralPath $Path -Force
    }
    if (-not [bool]$Snapshot.Exists) {
        return
    }
    New-Item -Path $Path -Force | Out-Null
    foreach ($value in @($Snapshot.Values)) {
        New-ItemProperty `
            -Path $Path `
            -Name ([string]$value.Name) `
            -Value $value.Value `
            -PropertyType ([string]$value.Kind) `
            -Force | Out-Null
    }
}

function Set-LatticeActivationJournalPhase {
    param(
        [Parameter(Mandatory)][string]$BackupRoot,
        [Parameter(Mandatory)][object]$Journal,
        [Parameter(Mandatory)][ValidateSet(
            'PREPARED',
            'SHORTCUT_ACTIVATED',
            'REGISTRY_ACTIVATED',
            'ACTIVE_RECEIPT_WRITTEN',
            'COMMITTED',
            'ROLLED_BACK')][string]$Phase
    )

    $Journal.phase = $Phase
    Write-LatticeJsonAtomic -LiteralPath (Join-Path $BackupRoot 'activation-journal.json') -Value $Journal
}

function Restore-LatticeActivationBackup {
    param(
        [Parameter(Mandatory)][object]$Paths,
        [Parameter(Mandatory)][object]$Owner,
        [Parameter(Mandatory)][string]$BackupRoot,
        [Parameter(Mandatory)][object]$StageMarker
    )

    $journalPath = Join-Path $BackupRoot 'activation-journal.json'
    $registrySnapshotPath = Join-Path $BackupRoot 'registry-snapshot.json'
    try {
        $journal = Get-Content -LiteralPath $journalPath -Raw | ConvertFrom-Json
        $registrySnapshot = Get-Content -LiteralPath $registrySnapshotPath -Raw | ConvertFrom-Json
    }
    catch {
        throw 'LATTICE_INSTALL_ACTIVATION_RECONCILIATION_REQUIRED'
    }
    if ([string]$journal.schema_version -cne $script:LatticeActivationJournalSchema -or
        [string]$journal.product -cne $script:LatticeProduct -or
        [string]$journal.install_id -cne [string]$Owner.install_id -or
        [string]$journal.operation_id -cnotmatch '^[0-9a-f]{32}$' -or
        [string]$journal.after_source_commit -cne [string]$StageMarker.source_commit -or
        [string]$journal.after_source_commit -cnotmatch '^[0-9a-f]{40}$' -or
        ([string]$journal.before_source_commit -ne '' -and [string]$journal.before_source_commit -cnotmatch '^[0-9a-f]{40}$') -or
        [string]$journal.shortcut_path -cne [string]$Paths.ShortcutPath -or
        [string]$journal.registry_path -cne [string]$Paths.RegistryPath -or
        [string]$journal.active_install_path -cne [string]$Paths.ActiveInstall -or
        -not [string]::Equals(
            [string]$journal.after_version_root,
            [IO.Path]::GetFullPath((Join-Path ([string]$Paths.VersionsRoot) ([string]$journal.after_source_commit))),
            [StringComparison]::OrdinalIgnoreCase) -or
        [string]$journal.registry_snapshot_sha256 -cne (Get-LatticeSha256Hex -LiteralPath $registrySnapshotPath) -or
        [string]$journal.phase -cnotmatch '^(PREPARED|SHORTCUT_ACTIVATED|REGISTRY_ACTIVATED|ACTIVE_RECEIPT_WRITTEN|COMMITTED|ROLLED_BACK)$') {
        throw 'LATTICE_INSTALL_ACTIVATION_RECONCILIATION_REQUIRED'
    }

    $shortcutBackup = Join-Path $BackupRoot 'shortcut.lnk'
    $activeBackup = Join-Path $BackupRoot 'active-install.json'
    if ([bool]$journal.shortcut_existed) {
        if (-not (Test-Path -LiteralPath $shortcutBackup -PathType Leaf) -or
            [string]$journal.shortcut_backup_sha256 -cne (Get-LatticeSha256Hex -LiteralPath $shortcutBackup)) {
            throw 'LATTICE_INSTALL_ACTIVATION_RECONCILIATION_REQUIRED'
        }
    }
    elseif (Test-Path -LiteralPath $shortcutBackup) {
        throw 'LATTICE_INSTALL_ACTIVATION_RECONCILIATION_REQUIRED'
    }
    if ([bool]$journal.active_existed) {
        if (-not (Test-Path -LiteralPath $activeBackup -PathType Leaf) -or
            [string]$journal.active_backup_sha256 -cne (Get-LatticeSha256Hex -LiteralPath $activeBackup)) {
            throw 'LATTICE_INSTALL_ACTIVATION_RECONCILIATION_REQUIRED'
        }
    }
    elseif (Test-Path -LiteralPath $activeBackup) {
        throw 'LATTICE_INSTALL_ACTIVATION_RECONCILIATION_REQUIRED'
    }

    if ([string]$journal.phase -ceq 'COMMITTED') {
        $active = Get-Content -LiteralPath ([string]$Paths.ActiveInstall) -Raw | ConvertFrom-Json
        $registry = Get-ItemProperty -LiteralPath ([string]$Paths.RegistryPath)
        $shortcutTarget = Get-LatticeShortcutTarget -ShortcutPath ([string]$Paths.ShortcutPath)
        if ([string]$active.install_id -cne [string]$Owner.install_id -or
            [string]$active.source_commit -cne [string]$journal.after_source_commit -or
            [string]$registry.LatticeInstallId -cne [string]$Owner.install_id -or
            [string]$registry.LatticeSourceCommit -cne [string]$journal.after_source_commit -or
            -not [string]::Equals(
                $shortcutTarget,
                (Join-Path ([string]$journal.after_version_root) 'app\LATTICE.exe'),
                [StringComparison]::OrdinalIgnoreCase)) {
            throw 'LATTICE_INSTALL_ACTIVATION_RECONCILIATION_REQUIRED'
        }
        return
    }

    Restore-LatticeRegistrySnapshot -Path ([string]$Paths.RegistryPath) -Snapshot $registrySnapshot
    if (Test-Path -LiteralPath ([string]$Paths.ShortcutPath)) {
        Remove-Item -LiteralPath ([string]$Paths.ShortcutPath) -Force
    }
    if ([bool]$journal.shortcut_existed) {
        [IO.Directory]::CreateDirectory([string]$Paths.ShortcutDirectory) | Out-Null
        Copy-Item -LiteralPath $shortcutBackup -Destination ([string]$Paths.ShortcutPath)
    }
    if (Test-Path -LiteralPath ([string]$Paths.ActiveInstall)) {
        Remove-Item -LiteralPath ([string]$Paths.ActiveInstall) -Force
    }
    if ([bool]$journal.active_existed) {
        Copy-Item -LiteralPath $activeBackup -Destination ([string]$Paths.ActiveInstall)
    }
    Set-LatticeActivationJournalPhase -BackupRoot $BackupRoot -Journal $journal -Phase 'ROLLED_BACK'
}

function Set-LatticeUninstallJournalPhase {
    param(
        [Parameter(Mandatory)][string]$BackupRoot,
        [Parameter(Mandatory)][object]$Journal,
        [Parameter(Mandatory)][ValidateSet('PREPARED', 'REMOVAL_STARTED')][string]$Phase
    )

    $Journal.phase = $Phase
    Write-LatticeJsonAtomic -LiteralPath (Join-Path $BackupRoot 'uninstall-journal.json') -Value $Journal
}

function Complete-LatticeInterruptedUninstall {
    param(
        [Parameter(Mandatory)][object]$Paths,
        [Parameter(Mandatory)][object]$Owner,
        [Parameter(Mandatory)][string]$BackupRoot,
        [Parameter(Mandatory)][object]$StageMarker
    )

    $journalPath = Join-Path $BackupRoot 'uninstall-journal.json'
    $registrySnapshotPath = Join-Path $BackupRoot 'registry-snapshot.json'
    try {
        $journal = Get-Content -LiteralPath $journalPath -Raw | ConvertFrom-Json
    }
    catch {
        throw 'LATTICE_UNINSTALL_RECONCILIATION_REQUIRED'
    }
    $expectedTombstone = Join-Path ([string]$Paths.InstallRoot) ('.r-' + [string]$journal.operation_id)
    if ([string]$journal.schema_version -cne $script:LatticeUninstallJournalSchema -or
        [string]$journal.product -cne $script:LatticeProduct -or
        [string]$journal.install_id -cne [string]$Owner.install_id -or
        [string]$journal.operation_id -cnotmatch '^[0-9a-f]{12}$' -or
        [string]$journal.phase -cnotmatch '^(PREPARED|REMOVAL_STARTED)$' -or
        [string]$journal.registry_path -cne [string]$Paths.RegistryPath -or
        [string]$journal.shortcut_path -cne [string]$Paths.ShortcutPath -or
        [string]$journal.active_install_path -cne [string]$Paths.ActiveInstall -or
        -not [string]::Equals([string]$journal.tombstone_path, $expectedTombstone, [StringComparison]::OrdinalIgnoreCase) -or
        [string]$journal.registry_snapshot_sha256 -cne (Get-LatticeSha256Hex -LiteralPath $registrySnapshotPath) -or
        [string]$StageMarker.source_commit -cne [string]$journal.active_source_commit) {
        throw 'LATTICE_UNINSTALL_RECONCILIATION_REQUIRED'
    }
    foreach ($backup in @(
        [PSCustomObject]@{ Exists = [bool]$journal.shortcut_existed; Path = (Join-Path $BackupRoot 'shortcut.lnk'); Hash = [string]$journal.shortcut_backup_sha256 },
        [PSCustomObject]@{ Exists = [bool]$journal.active_existed; Path = (Join-Path $BackupRoot 'active-install.json'); Hash = [string]$journal.active_backup_sha256 })) {
        if ([bool]$backup.Exists) {
            if (-not (Test-Path -LiteralPath ([string]$backup.Path) -PathType Leaf) -or
                [string]$backup.Hash -cne (Get-LatticeSha256Hex -LiteralPath ([string]$backup.Path))) {
                throw 'LATTICE_UNINSTALL_RECONCILIATION_REQUIRED'
            }
        }
        elseif (Test-Path -LiteralPath ([string]$backup.Path)) {
            throw 'LATTICE_UNINSTALL_RECONCILIATION_REQUIRED'
        }
    }

    $stagingEntries = @(Get-ChildItem -LiteralPath ([string]$Paths.StagingRoot) -Force)
    if ($stagingEntries.Count -ne 1 -or
        -not [string]::Equals($stagingEntries[0].FullName, $BackupRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'LATTICE_UNINSTALL_RECONCILIATION_REQUIRED'
    }
    if ([string]$journal.phase -ceq 'PREPARED') {
        Set-LatticeUninstallJournalPhase -BackupRoot $BackupRoot -Journal $journal -Phase 'REMOVAL_STARTED'
    }

    $tombstonePath = [string]$journal.tombstone_path
    $versionsExist = Test-Path -LiteralPath ([string]$Paths.VersionsRoot) -PathType Container
    $tombstoneExists = Test-Path -LiteralPath $tombstonePath -PathType Container
    if ($versionsExist -and $tombstoneExists) {
        throw 'LATTICE_UNINSTALL_RECONCILIATION_REQUIRED'
    }
    if ($versionsExist) {
        [IO.Directory]::Move([string]$Paths.VersionsRoot, $tombstonePath)
        $tombstoneExists = $true
    }
    Invoke-LatticeInstallerHook -Name 'AfterVersionsTombstoned'

    if (Test-Path -LiteralPath ([string]$Paths.ShortcutPath)) {
        Remove-Item -LiteralPath ([string]$Paths.ShortcutPath) -Force
    }
    if (Test-Path -LiteralPath ([string]$Paths.RegistryPath)) {
        Get-LatticeRegistrySnapshot -Path ([string]$Paths.RegistryPath) | Out-Null
        Remove-Item -LiteralPath ([string]$Paths.RegistryPath) -Force
    }
    if (Test-Path -LiteralPath ([string]$Paths.ActiveInstall)) {
        Remove-Item -LiteralPath ([string]$Paths.ActiveInstall) -Force
    }
    if ($tombstoneExists -and (Test-Path -LiteralPath $tombstonePath)) {
        Assert-LatticeTreeHasNoReparsePoints -Root $tombstonePath -ErrorCode 'LATTICE_UNINSTALL_TOMBSTONE_REPARSE_POINT_REJECTED'
        $tombstoneIoPath = if ($tombstonePath.StartsWith('\\', [StringComparison]::Ordinal)) {
            '\\?\UNC\' + $tombstonePath.Substring(2)
        }
        else {
            '\\?\' + $tombstonePath
        }
        [IO.Directory]::Delete($tombstoneIoPath, $true)
    }
    if (Test-Path -LiteralPath ([string]$Paths.OwnerMarker)) {
        Remove-Item -LiteralPath ([string]$Paths.OwnerMarker) -Force
    }
    if (Test-Path -LiteralPath ([string]$Paths.StagingRoot)) {
        Remove-Item -LiteralPath ([string]$Paths.StagingRoot) -Recurse -Force
    }
    if (Test-Path -LiteralPath ([string]$Paths.InstallRoot)) {
        $remaining = @(Get-ChildItem -LiteralPath ([string]$Paths.InstallRoot) -Force)
        if ($remaining.Count -ne 0) {
            throw 'LATTICE_UNINSTALL_RECONCILIATION_REQUIRED'
        }
        Remove-Item -LiteralPath ([string]$Paths.InstallRoot) -Force
    }
    if ((Test-Path -LiteralPath ([string]$Paths.ShortcutDirectory) -PathType Container) -and
        @(Get-ChildItem -LiteralPath ([string]$Paths.ShortcutDirectory) -Force).Count -eq 0) {
        Remove-Item -LiteralPath ([string]$Paths.ShortcutDirectory) -Force
    }
}

function Set-LatticeRegistryValue {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)]$Value,
        [Parameter(Mandatory)][Microsoft.Win32.RegistryValueKind]$Kind
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        New-Item -Path $Path -Force | Out-Null
    }
    New-ItemProperty `
        -Path $Path `
        -Name $Name `
        -Value $Value `
        -PropertyType ([string]$Kind) `
        -Force | Out-Null
}

function Get-LatticePowerShellPath {
    $windowsPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    if (-not (Test-Path -LiteralPath $windowsPowerShell -PathType Leaf)) {
        throw 'LATTICE_INSTALL_WINDOWS_POWERSHELL_MISSING'
    }
    return $windowsPowerShell
}

function Quote-LatticeCommandArgument {
    param([Parameter(Mandatory)][string]$Value)

    return '"' + $Value.Replace('"', '\"') + '"'
}

function New-LatticeShortcut {
    param(
        [Parameter(Mandatory)][string]$ShortcutPath,
        [Parameter(Mandatory)][string]$TargetPath,
        [Parameter(Mandatory)][string]$WorkingDirectory
    )

    $directory = [IO.Path]::GetDirectoryName($ShortcutPath)
    [IO.Directory]::CreateDirectory($directory) | Out-Null
    $temporaryPath = Join-Path $directory ('LATTICE.' + [guid]::NewGuid().ToString('N') + '.lnk')
    $shell = New-Object -ComObject WScript.Shell
    try {
        $shortcut = $shell.CreateShortcut($temporaryPath)
        $shortcut.TargetPath = $TargetPath
        $shortcut.WorkingDirectory = $WorkingDirectory
        $shortcut.IconLocation = "$TargetPath,0"
        $shortcut.Description = 'LATTICE Control'
        $shortcut.Save()
        Move-Item -LiteralPath $temporaryPath -Destination $ShortcutPath -Force
    }
    finally {
        if (Test-Path -LiteralPath $temporaryPath) {
            Remove-Item -LiteralPath $temporaryPath -Force
        }
        if ($null -ne $shell) {
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
        }
    }
}

function Get-LatticeShortcutTarget {
    param([Parameter(Mandatory)][string]$ShortcutPath)

    $shell = New-Object -ComObject WScript.Shell
    try {
        return [IO.Path]::GetFullPath([string]$shell.CreateShortcut($ShortcutPath).TargetPath)
    }
    finally {
        if ($null -ne $shell) {
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
        }
    }
}

function Invoke-LatticeInstallerHook {
    param([Parameter(Mandatory)][string]$Name)
}

function Enter-LatticeInstallerMutex {
    param([Parameter(Mandatory)][string]$InstallRoot)

    $digest = Get-LatticeStringSha256Hex -Value ([IO.Path]::GetFullPath($InstallRoot).ToLowerInvariant())
    $mutex = [Threading.Mutex]::new($false, ('Local\LATTICE_CONTROL_DESKTOP_INSTALL_' + $digest.Substring(0, 24)))
    try {
        if (-not $mutex.WaitOne([TimeSpan]::FromSeconds(30))) {
            $mutex.Dispose()
            throw 'LATTICE_INSTALL_MUTEX_TIMEOUT'
        }
    }
    catch [Threading.AbandonedMutexException] {
        # The abandoned mutex is acquired by this thread; owned staging is reconciled next.
        return $mutex
    }
    catch {
        $mutex.Dispose()
        throw
    }
    return $mutex
}

function Exit-LatticeInstallerMutex {
    param([Threading.Mutex]$Mutex)

    if ($null -ne $Mutex) {
        try { $Mutex.ReleaseMutex() } catch { }
        $Mutex.Dispose()
    }
}

function Assert-LatticeVersionFilesDeletable {
    param([Parameter(Mandatory)][array]$VersionDirectories)

    if ($null -eq ('Lattice.DesktopInstaller.NativeFileDeleteProbe' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace Lattice.DesktopInstaller {
    public static class NativeFileDeleteProbe {
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern SafeFileHandle CreateFileW(
            string fileName,
            uint desiredAccess,
            uint shareMode,
            IntPtr securityAttributes,
            uint creationDisposition,
            uint flagsAndAttributes,
            IntPtr templateFile);
    }
}
'@
    }
    foreach ($directory in $VersionDirectories) {
        foreach ($file in @(Get-ChildItem -LiteralPath $directory.FullName -Force -File -Recurse)) {
            $handle = [Lattice.DesktopInstaller.NativeFileDeleteProbe]::CreateFileW(
                $file.FullName,
                0x00010000,
                0x00000007,
                [IntPtr]::Zero,
                3,
                0x00000080,
                [IntPtr]::Zero)
            if ($handle.IsInvalid) {
                $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
                $handle.Dispose()
                throw "LATTICE_UNINSTALL_FILE_NOT_DELETABLE:${errorCode}:$($file.FullName)"
            }
            $handle.Dispose()
        }
    }
}

function Get-LatticeInstalledVersion {
    param(
        [Parameter(Mandatory)][string]$VersionRoot,
        [Parameter(Mandatory)][string]$ExpectedSourceCommit,
        [Parameter(Mandatory)][string]$ExpectedInstallId
    )

    $versionRootFull = [IO.Path]::GetFullPath($VersionRoot)
    Assert-LatticeTreeHasNoReparsePoints -Root $versionRootFull -ErrorCode 'LATTICE_INSTALL_VERSION_REPARSE_POINT_REJECTED'
    $appRoot = Join-Path $versionRootFull 'app'
    $installerRoot = Join-Path $versionRootFull 'installer'
    Assert-LatticeNotReparsePoint -LiteralPath $appRoot
    Assert-LatticeNotReparsePoint -LiteralPath $installerRoot
    $expectedTopNames = @(
        '.lattice-stage-owner.json',
        'app',
        'installer',
        'install-manifest.json',
        'install-receipt.json')
    $actualTopNames = @(Get-ChildItem -LiteralPath $versionRootFull -Force | ForEach-Object Name | Sort-Object)
    if (($actualTopNames -join "`n") -cne (($expectedTopNames | Sort-Object) -join "`n")) {
        throw 'LATTICE_INSTALL_VERSION_LAYOUT_MISMATCH'
    }
    $manifestPath = Join-Path $versionRootFull 'install-manifest.json'
    $manifest = Get-LatticeInstallManifest -ManifestPath $manifestPath
    if ([string]$manifest.source_commit -cne $ExpectedSourceCommit) {
        throw 'LATTICE_INSTALL_VERSION_SOURCE_MISMATCH'
    }
    try {
        $stageOwner = Get-Content -LiteralPath (Join-Path $versionRootFull '.lattice-stage-owner.json') -Raw | ConvertFrom-Json
    }
    catch {
        throw 'LATTICE_INSTALL_VERSION_STAGE_OWNER_INVALID_JSON'
    }
    if ([string]$stageOwner.schema_version -cne $script:LatticeStageOwnerSchema -or
        [string]$stageOwner.product -cne $script:LatticeProduct -or
        [string]$stageOwner.install_id -cne $ExpectedInstallId -or
        [string]$stageOwner.purpose -cne 'INSTALL' -or
        [string]$stageOwner.source_commit -cne $ExpectedSourceCommit) {
        throw 'LATTICE_INSTALL_VERSION_STAGE_OWNER_MISMATCH'
    }
    $manifestEntries = @{}
    foreach ($entry in @($manifest.files)) {
        $manifestEntries[[string]$entry.path] = $entry
    }
    $installerNames = @(
        'Install-LATTICE.ps1',
        'Uninstall-LATTICE.ps1',
        'LatticeDesktopInstaller.Common.ps1')
    $actualInstallerItems = @(Get-ChildItem -LiteralPath $installerRoot -Force)
    foreach ($item in $actualInstallerItems) {
        if ($item.PSIsContainer -or
            ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "LATTICE_INSTALL_VERSION_INSTALLER_ENTRY_INVALID:$($item.Name)"
        }
    }
    $actualInstallerNames = @($actualInstallerItems | ForEach-Object Name | Sort-Object)
    if (($actualInstallerNames -join "`n") -cne (($installerNames | Sort-Object) -join "`n")) {
        throw 'LATTICE_INSTALL_VERSION_INSTALLER_SET_MISMATCH'
    }
    foreach ($name in $installerNames) {
        if (-not $manifestEntries.ContainsKey($name)) {
            throw "LATTICE_INSTALL_VERSION_INSTALLER_MANIFEST_MISSING:$name"
        }
        $path = Join-Path $installerRoot $name
        $entry = $manifestEntries[$name]
        if ((Get-Item -LiteralPath $path).Length -ne [long]$entry.length -or
            (Get-LatticeSha256Hex -LiteralPath $path) -cne [string]$entry.sha256) {
            throw "LATTICE_INSTALL_VERSION_INSTALLER_MISMATCH:$name"
        }
    }
    $payload = Get-LatticePortablePayload `
        -PayloadRoot $appRoot `
        -ExpectedSourceCommit $ExpectedSourceCommit
    $receiptPath = Join-Path $versionRootFull 'install-receipt.json'
    try {
        $receipt = Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
    }
    catch {
        throw 'LATTICE_INSTALL_RECEIPT_INVALID_JSON'
    }
    if ([string]$receipt.schema_version -cne $script:LatticeInstallReceiptSchema -or
        [string]$receipt.product -cne $script:LatticeProduct -or
        [string]$receipt.install_id -cne $ExpectedInstallId -or
        [string]$receipt.source_commit -cne $ExpectedSourceCommit -or
        [string]$receipt.install_manifest_sha256 -cne (Get-LatticeSha256Hex -LiteralPath $manifestPath) -or
        [string]$receipt.payload_archive_sha256 -cne [string]$manifest.payload.sha256 -or
        [string]$receipt.payload_manifest_sha256 -cne $payload.ManifestSha256 -or
        [int]$receipt.payload_file_count -ne [int]$payload.FileCount) {
        throw 'LATTICE_INSTALL_RECEIPT_MISMATCH'
    }
    return [PSCustomObject]@{
        Root = $versionRootFull
        SourceCommit = $ExpectedSourceCommit
        Manifest = $manifest
        ManifestPath = $manifestPath
        ManifestSha256 = Get-LatticeSha256Hex -LiteralPath $manifestPath
        Payload = $payload
        Receipt = $receipt
        ReceiptPath = $receiptPath
        ExecutablePath = Join-Path $versionRootFull 'app\LATTICE.exe'
        UninstallerPath = Join-Path $versionRootFull 'installer\Uninstall-LATTICE.ps1'
    }
}

function Set-LatticeUninstallRegistration {
    param(
        [Parameter(Mandatory)][object]$Context,
        [Parameter(Mandatory)][object]$Paths,
        [Parameter(Mandatory)][object]$Owner,
        [Parameter(Mandatory)][object]$Version
    )

    $powershellPath = Get-LatticePowerShellPath
    $arguments = @(
        (Quote-LatticeCommandArgument $powershellPath),
        '-NoLogo', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
        (Quote-LatticeCommandArgument ([string]$Version.UninstallerPath))
    )
    if ([bool]$Context.TestMode) {
        $arguments += @(
            '-TestSandboxRoot', (Quote-LatticeCommandArgument ([string]$Context.SandboxRoot)),
            '-TestRegistryId', (Quote-LatticeCommandArgument ([IO.Path]::GetFileName([string]$Context.RegistryTestRoot)))
        )
    }
    $uninstallCommand = $arguments -join ' '
    $registryPath = [string]$Paths.RegistryPath
    if (Test-Path -LiteralPath $registryPath) {
        Get-LatticeRegistrySnapshot -Path $registryPath | Out-Null
        $existing = Get-ItemProperty -LiteralPath $registryPath
        if ($null -eq $existing.PSObject.Properties['LatticeProduct'] -or
            $null -eq $existing.PSObject.Properties['LatticeInstallId'] -or
            $null -eq $existing.PSObject.Properties['InstallLocation'] -or
            [string]$existing.LatticeProduct -cne $script:LatticeProduct -or
            [string]$existing.LatticeInstallId -cne [string]$Owner.install_id -or
            -not [string]::Equals(
                [string]$existing.InstallLocation,
                [string]$Paths.InstallRoot,
                [StringComparison]::OrdinalIgnoreCase)) {
            throw 'LATTICE_INSTALL_REGISTRY_NOT_OWNED'
        }
    }
    $estimatedSize = [int][Math]::Ceiling((
        (Get-ChildItem -LiteralPath ([string]$Version.Root) -File -Recurse | Measure-Object Length -Sum).Sum
    ) / 1KB)
    Set-LatticeRegistryValue -Path $registryPath -Name 'DisplayName' -Value 'LATTICE Control' -Kind String
    Set-LatticeRegistryValue -Path $registryPath -Name 'DisplayVersion' -Value (('1.0.0+' + [string]$Version.SourceCommit).Substring(0, 20)) -Kind String
    Set-LatticeRegistryValue -Path $registryPath -Name 'Publisher' -Value 'LATTICE' -Kind String
    Set-LatticeRegistryValue -Path $registryPath -Name 'InstallLocation' -Value ([string]$Paths.InstallRoot) -Kind String
    Set-LatticeRegistryValue -Path $registryPath -Name 'DisplayIcon' -Value ([string]$Version.ExecutablePath + ',0') -Kind String
    Set-LatticeRegistryValue -Path $registryPath -Name 'UninstallString' -Value $uninstallCommand -Kind String
    Set-LatticeRegistryValue -Path $registryPath -Name 'QuietUninstallString' -Value ($uninstallCommand + ' -Quiet') -Kind String
    Set-LatticeRegistryValue -Path $registryPath -Name 'NoModify' -Value 1 -Kind DWord
    Set-LatticeRegistryValue -Path $registryPath -Name 'NoRepair' -Value 1 -Kind DWord
    Set-LatticeRegistryValue -Path $registryPath -Name 'EstimatedSize' -Value $estimatedSize -Kind DWord
    Set-LatticeRegistryValue -Path $registryPath -Name 'InstallDate' -Value ([DateTime]::UtcNow.ToString('yyyyMMdd')) -Kind String
    Invoke-LatticeInstallerHook -Name 'AfterRegistryCoreValuesWritten'
    Set-LatticeRegistryValue -Path $registryPath -Name 'LatticeProduct' -Value $script:LatticeProduct -Kind String
    Set-LatticeRegistryValue -Path $registryPath -Name 'LatticeInstallId' -Value ([string]$Owner.install_id) -Kind String
    Set-LatticeRegistryValue -Path $registryPath -Name 'LatticeSourceCommit' -Value ([string]$Version.SourceCommit) -Kind String
}

function Invoke-LatticeActivation {
    param(
        [Parameter(Mandatory)][object]$Context,
        [Parameter(Mandatory)][object]$Paths,
        [Parameter(Mandatory)][object]$Owner,
        [Parameter(Mandatory)][object]$Version
    )

    $operationId = [guid]::NewGuid().ToString('N')
    $backupRoot = Join-Path ([string]$Paths.StagingRoot) ('activation-' + $operationId)
    [IO.Directory]::CreateDirectory($backupRoot) | Out-Null
    Write-LatticeStageOwner -StageRoot $backupRoot -Owner $Owner -Purpose 'INSTALL' -SourceCommit ([string]$Version.SourceCommit)
    $shortcutExisted = Test-Path -LiteralPath ([string]$Paths.ShortcutPath) -PathType Leaf
    $activeExisted = Test-Path -LiteralPath ([string]$Paths.ActiveInstall) -PathType Leaf
    if ($shortcutExisted) {
        Copy-Item -LiteralPath ([string]$Paths.ShortcutPath) -Destination (Join-Path $backupRoot 'shortcut.lnk')
    }
    if ($activeExisted) {
        Copy-Item -LiteralPath ([string]$Paths.ActiveInstall) -Destination (Join-Path $backupRoot 'active-install.json')
    }
    $registrySnapshot = Get-LatticeRegistrySnapshot -Path ([string]$Paths.RegistryPath)
    $registrySnapshotPath = Join-Path $backupRoot 'registry-snapshot.json'
    Write-LatticeJsonAtomic -LiteralPath $registrySnapshotPath -Value $registrySnapshot
    $beforeSourceCommit = ''
    if ($activeExisted) {
        try {
            $beforeActive = Get-Content -LiteralPath ([string]$Paths.ActiveInstall) -Raw | ConvertFrom-Json
        }
        catch {
            throw 'LATTICE_INSTALL_ACTIVE_RECEIPT_INVALID_JSON'
        }
        if ([string]$beforeActive.schema_version -cne $script:LatticeActiveInstallSchema -or
            [string]$beforeActive.product -cne $script:LatticeProduct -or
            [string]$beforeActive.install_id -cne [string]$Owner.install_id -or
            [string]$beforeActive.source_commit -cnotmatch '^[0-9a-f]{40}$') {
            throw 'LATTICE_INSTALL_ACTIVE_RECEIPT_MISMATCH'
        }
        $beforeSourceCommit = [string]$beforeActive.source_commit
    }
    $journal = [PSCustomObject][ordered]@{
        schema_version = $script:LatticeActivationJournalSchema
        product = $script:LatticeProduct
        install_id = [string]$Owner.install_id
        operation_id = $operationId
        phase = 'PREPARED'
        before_source_commit = $beforeSourceCommit
        after_source_commit = [string]$Version.SourceCommit
        after_version_root = [string]$Version.Root
        shortcut_path = [string]$Paths.ShortcutPath
        registry_path = [string]$Paths.RegistryPath
        active_install_path = [string]$Paths.ActiveInstall
        shortcut_existed = $shortcutExisted
        shortcut_backup_sha256 = if ($shortcutExisted) { Get-LatticeSha256Hex -LiteralPath (Join-Path $backupRoot 'shortcut.lnk') } else { '' }
        active_existed = $activeExisted
        active_backup_sha256 = if ($activeExisted) { Get-LatticeSha256Hex -LiteralPath (Join-Path $backupRoot 'active-install.json') } else { '' }
        registry_snapshot_sha256 = Get-LatticeSha256Hex -LiteralPath $registrySnapshotPath
    }
    Set-LatticeActivationJournalPhase -BackupRoot $backupRoot -Journal $journal -Phase 'PREPARED'
    Write-LatticeStageOwner -StageRoot $backupRoot -Owner $Owner -Purpose 'ACTIVATION_BACKUP' -SourceCommit ([string]$Version.SourceCommit)
    $cleanupBackup = $false
    try {
        if ($shortcutExisted) {
            $existingTarget = Get-LatticeShortcutTarget -ShortcutPath ([string]$Paths.ShortcutPath)
            if (-not (Test-LatticePathWithinRoot -Path $existingTarget -Root ([string]$Paths.InstallRoot))) {
                throw 'LATTICE_INSTALL_SHORTCUT_NOT_OWNED'
            }
        }
        New-LatticeShortcut `
            -ShortcutPath ([string]$Paths.ShortcutPath) `
            -TargetPath ([string]$Version.ExecutablePath) `
            -WorkingDirectory ([IO.Path]::GetDirectoryName([string]$Version.ExecutablePath))
        Set-LatticeActivationJournalPhase -BackupRoot $backupRoot -Journal $journal -Phase 'SHORTCUT_ACTIVATED'
        Invoke-LatticeInstallerHook -Name 'AfterShortcutActivated'
        Set-LatticeUninstallRegistration -Context $Context -Paths $Paths -Owner $Owner -Version $Version
        Set-LatticeActivationJournalPhase -BackupRoot $backupRoot -Journal $journal -Phase 'REGISTRY_ACTIVATED'
        Invoke-LatticeInstallerHook -Name 'AfterRegistryActivated'
        $active = [ordered]@{
            schema_version = $script:LatticeActiveInstallSchema
            product = $script:LatticeProduct
            install_id = [string]$Owner.install_id
            source_commit = [string]$Version.SourceCommit
            version_root = [string]$Version.Root
            executable = [string]$Version.ExecutablePath
            receipt = [string]$Version.ReceiptPath
            activated_at_utc = [DateTimeOffset]::UtcNow.ToString('o')
        }
        Write-LatticeJsonAtomic -LiteralPath ([string]$Paths.ActiveInstall) -Value $active
        Set-LatticeActivationJournalPhase -BackupRoot $backupRoot -Journal $journal -Phase 'ACTIVE_RECEIPT_WRITTEN'
        Invoke-LatticeInstallerHook -Name 'AfterActiveReceiptWritten'
        Set-LatticeActivationJournalPhase -BackupRoot $backupRoot -Journal $journal -Phase 'COMMITTED'
        $cleanupBackup = $true
    }
    catch {
        $marker = Get-Content -LiteralPath (Join-Path $backupRoot '.lattice-stage-owner.json') -Raw | ConvertFrom-Json
        Restore-LatticeActivationBackup -Paths $Paths -Owner $Owner -BackupRoot $backupRoot -StageMarker $marker
        $cleanupBackup = $true
        throw
    }
    finally {
        if ($cleanupBackup -and (Test-Path -LiteralPath $backupRoot)) {
            Remove-Item -LiteralPath $backupRoot -Recurse -Force
        }
    }
}

function Invoke-LatticeDesktopInstall {
    param(
        [string]$BundleRoot = '',
        [string]$RollbackToCommit = '',
        [string]$TestSandboxRoot = '',
        [string]$TestRegistryId = ''
    )

    $context = Resolve-LatticeInstallContext -TestSandboxRoot $TestSandboxRoot -TestRegistryId $TestRegistryId
    $paths = Get-LatticeInstallPaths -Context $context
    $mutex = Enter-LatticeInstallerMutex -InstallRoot ([string]$paths.InstallRoot)
    try {
        $owner = Initialize-LatticeOwnedInstallRoot -Paths $paths
        $recoveredUninstall = Clear-LatticeOwnedStaging -Paths $paths -Owner $owner
        if ($recoveredUninstall) {
            $owner = Initialize-LatticeOwnedInstallRoot -Paths $paths
        }
        Initialize-LatticeInstallDirectories -Paths $paths
        $action = 'INSTALLED'
        if (-not [string]::IsNullOrWhiteSpace($RollbackToCommit)) {
            if ($RollbackToCommit -cnotmatch '^[0-9a-f]{40}$') {
                throw 'LATTICE_INSTALL_ROLLBACK_COMMIT_INVALID'
            }
            $versionRoot = Join-Path ([string]$paths.VersionsRoot) $RollbackToCommit
            if (-not (Test-Path -LiteralPath $versionRoot -PathType Container)) {
                throw 'LATTICE_INSTALL_ROLLBACK_VERSION_NOT_FOUND'
            }
            $version = Get-LatticeInstalledVersion `
                -VersionRoot $versionRoot `
                -ExpectedSourceCommit $RollbackToCommit `
                -ExpectedInstallId ([string]$owner.install_id)
            $action = 'ROLLED_BACK'
        }
        else {
            if ([string]::IsNullOrWhiteSpace($BundleRoot)) {
                throw 'LATTICE_INSTALL_BUNDLE_ROOT_REQUIRED'
            }
            $bundle = Get-LatticeInstallerBundle -BundleRoot $BundleRoot
            $versionRoot = Join-Path ([string]$paths.VersionsRoot) ([string]$bundle.SourceCommit)
            if (Test-Path -LiteralPath $versionRoot -PathType Container) {
                $version = Get-LatticeInstalledVersion `
                    -VersionRoot $versionRoot `
                    -ExpectedSourceCommit ([string]$bundle.SourceCommit) `
                    -ExpectedInstallId ([string]$owner.install_id)
                if ([string]$version.ManifestSha256 -cne [string]$bundle.ManifestSha256) {
                    throw 'LATTICE_INSTALL_EXISTING_VERSION_MANIFEST_MISMATCH'
                }
                $action = 'REUSED'
            }
            else {
                $stageRoot = Join-Path ([string]$paths.StagingRoot) (
                    ([string]$bundle.SourceCommit).Substring(0, 12) + '-' +
                    [guid]::NewGuid().ToString('N').Substring(0, 8))
                try {
                    [IO.Directory]::CreateDirectory($stageRoot) | Out-Null
                    Write-LatticeStageOwner `
                        -StageRoot $stageRoot `
                        -Owner $owner `
                        -Purpose 'INSTALL' `
                        -SourceCommit ([string]$bundle.SourceCommit)
                    $appRoot = Join-Path $stageRoot 'app'
                    $payload = Expand-LatticePortablePayload `
                        -ArchivePath ([string]$bundle.PayloadPath) `
                        -DestinationPath $appRoot `
                        -ExpectedSourceCommit ([string]$bundle.SourceCommit) `
                        -FinalPayloadPath (Join-Path $versionRoot 'app')
                    $installerRoot = Join-Path $stageRoot 'installer'
                    [IO.Directory]::CreateDirectory($installerRoot) | Out-Null
                    foreach ($name in @(
                        'Install-LATTICE.ps1',
                        'Uninstall-LATTICE.ps1',
                        'LatticeDesktopInstaller.Common.ps1')) {
                        Copy-Item -LiteralPath (Join-Path ([string]$bundle.Root) $name) -Destination (Join-Path $installerRoot $name)
                    }
                    Copy-Item -LiteralPath ([string]$bundle.ManifestPath) -Destination (Join-Path $stageRoot 'install-manifest.json')
                    $receipt = [ordered]@{
                        schema_version = $script:LatticeInstallReceiptSchema
                        product = $script:LatticeProduct
                        install_id = [string]$owner.install_id
                        source_commit = [string]$bundle.SourceCommit
                        install_manifest_sha256 = [string]$bundle.ManifestSha256
                        payload_archive_sha256 = [string]$bundle.PayloadSha256
                        payload_manifest_sha256 = [string]$payload.ManifestSha256
                        payload_file_count = [int]$payload.FileCount
                        installed_at_utc = [DateTimeOffset]::UtcNow.ToString('o')
                        preserved_user_data = @(
                            '%LOCALAPPDATA%\LATTICE\control\lattice-control.db',
                            '%LOCALAPPDATA%\LATTICE\ControlDesktop\WebView2')
                    }
                    Write-LatticeJsonAtomic -LiteralPath (Join-Path $stageRoot 'install-receipt.json') -Value $receipt
                    $verifiedStage = Get-LatticeInstalledVersion `
                        -VersionRoot $stageRoot `
                        -ExpectedSourceCommit ([string]$bundle.SourceCommit) `
                        -ExpectedInstallId ([string]$owner.install_id)
                    Invoke-LatticeInstallerHook -Name 'AfterStageVerified'
                    [IO.Directory]::Move($stageRoot, $versionRoot)
                    $version = Get-LatticeInstalledVersion `
                        -VersionRoot $versionRoot `
                        -ExpectedSourceCommit ([string]$bundle.SourceCommit) `
                        -ExpectedInstallId ([string]$owner.install_id)
                }
                finally {
                    if ($null -ne (Get-Variable stageRoot -ErrorAction SilentlyContinue) -and
                        (Test-Path -LiteralPath $stageRoot)) {
                        Remove-Item -LiteralPath $stageRoot -Recurse -Force
                    }
                }
            }
        }
        Invoke-LatticeInstallerHook -Name 'AfterVersionPrepared'
        Invoke-LatticeActivation -Context $context -Paths $paths -Owner $owner -Version $version
        return [ordered]@{
            result = 'PASS'
            action = $action
            source_commit = [string]$version.SourceCommit
            install_scope = 'CURRENT_USER'
            requires_elevation = $false
            install_root = [string]$paths.InstallRoot
            version_root = [string]$version.Root
            executable = [string]$version.ExecutablePath
            shortcut = [string]$paths.ShortcutPath
            uninstall_registry = [string]$paths.RegistryPath
            receipt = [string]$version.ReceiptPath
        }
    }
    finally {
        Exit-LatticeInstallerMutex -Mutex $mutex
    }
}

function Assert-LatticeOwnedUninstallSurface {
    param(
        [Parameter(Mandatory)][object]$Paths,
        [Parameter(Mandatory)][object]$Owner
    )

    if (Test-Path -LiteralPath ([string]$Paths.ShortcutPath) -PathType Leaf) {
        $shortcutTarget = Get-LatticeShortcutTarget -ShortcutPath ([string]$Paths.ShortcutPath)
        if (-not (Test-LatticePathWithinRoot -Path $shortcutTarget -Root ([string]$Paths.InstallRoot))) {
            throw 'LATTICE_UNINSTALL_SHORTCUT_NOT_OWNED'
        }
    }
    if (Test-Path -LiteralPath ([string]$Paths.RegistryPath)) {
        Get-LatticeRegistrySnapshot -Path ([string]$Paths.RegistryPath) | Out-Null
        $registry = Get-ItemProperty -LiteralPath ([string]$Paths.RegistryPath)
        if ([string]$registry.LatticeProduct -cne $script:LatticeProduct -or
            [string]$registry.LatticeInstallId -cne [string]$Owner.install_id -or
            -not [string]::Equals([string]$registry.InstallLocation, [string]$Paths.InstallRoot, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'LATTICE_UNINSTALL_REGISTRY_NOT_OWNED'
        }
    }
}

function Invoke-LatticeDesktopUninstall {
    param(
        [string]$TestSandboxRoot = '',
        [string]$TestRegistryId = '',
        [switch]$Quiet
    )

    $context = Resolve-LatticeInstallContext -TestSandboxRoot $TestSandboxRoot -TestRegistryId $TestRegistryId
    $paths = Get-LatticeInstallPaths -Context $context
    if (-not (Test-Path -LiteralPath ([string]$paths.InstallRoot) -PathType Container)) {
        if ((Test-Path -LiteralPath ([string]$paths.ShortcutPath)) -or
            (Test-Path -LiteralPath ([string]$paths.RegistryPath))) {
            throw 'LATTICE_UNINSTALL_ORPHANED_SURFACE'
        }
        return [ordered]@{ result = 'PASS'; action = 'ALREADY_ABSENT'; install_root = [string]$paths.InstallRoot }
    }
    $mutex = Enter-LatticeInstallerMutex -InstallRoot ([string]$paths.InstallRoot)
    try {
        $owner = Initialize-LatticeOwnedInstallRoot -Paths $paths
        $recoveredUninstall = Clear-LatticeOwnedStaging -Paths $paths -Owner $owner
        if ($recoveredUninstall) {
            return [ordered]@{
                result = 'PASS'
                action = 'UNINSTALLED'
                install_root = [string]$paths.InstallRoot
                preserved_user_data = @(
                    '%LOCALAPPDATA%\LATTICE\control\lattice-control.db',
                    '%LOCALAPPDATA%\LATTICE\ControlDesktop\WebView2')
            }
        }
        Initialize-LatticeInstallDirectories -Paths $paths
        Assert-LatticeOwnedUninstallSurface -Paths $paths -Owner $owner
        $rootItems = @(Get-ChildItem -LiteralPath ([string]$paths.InstallRoot) -Force)
        $allowedRootEntries = @('.staging', 'active-install.json', 'install-owner.json', 'versions')
        foreach ($item in $rootItems) {
            if ($allowedRootEntries -cnotcontains $item.Name) {
                throw "LATTICE_UNINSTALL_UNKNOWN_ROOT_ENTRY:$($item.Name)"
            }
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "LATTICE_UNINSTALL_REPARSE_POINT_REJECTED:$($item.Name)"
            }
            if (($item.Name -in @('.staging', 'versions')) -and -not $item.PSIsContainer) {
                throw "LATTICE_UNINSTALL_ROOT_ENTRY_TYPE_MISMATCH:$($item.Name)"
            }
            if (($item.Name -in @('active-install.json', 'install-owner.json')) -and $item.PSIsContainer) {
                throw "LATTICE_UNINSTALL_ROOT_ENTRY_TYPE_MISMATCH:$($item.Name)"
            }
        }
        $stagingEntries = @(Get-ChildItem -LiteralPath ([string]$paths.StagingRoot) -Force)
        if ($stagingEntries.Count -ne 0) {
            throw 'LATTICE_UNINSTALL_STAGING_NOT_EMPTY'
        }
        $versionEntries = @(Get-ChildItem -LiteralPath ([string]$paths.VersionsRoot) -Force)
        foreach ($entry in $versionEntries) {
            if (-not $entry.PSIsContainer -or
                ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
                $entry.Name -cnotmatch '^[0-9a-f]{40}$') {
                throw "LATTICE_UNINSTALL_UNKNOWN_VERSION_ENTRY:$($entry.Name)"
            }
            Get-LatticeInstalledVersion `
                -VersionRoot $entry.FullName `
                -ExpectedSourceCommit $entry.Name `
                -ExpectedInstallId ([string]$owner.install_id) | Out-Null
        }
        $versionDirectories = @($versionEntries)
        if (Test-Path -LiteralPath ([string]$paths.ActiveInstall) -PathType Leaf) {
            try {
                $active = Get-Content -LiteralPath ([string]$paths.ActiveInstall) -Raw | ConvertFrom-Json
            }
            catch {
                throw 'LATTICE_UNINSTALL_ACTIVE_RECEIPT_INVALID_JSON'
            }
            if ([string]$active.schema_version -cne $script:LatticeActiveInstallSchema -or
                [string]$active.product -cne $script:LatticeProduct -or
                [string]$active.install_id -cne [string]$owner.install_id -or
                [string]$active.source_commit -cnotmatch '^[0-9a-f]{40}$' -or
                -not (Test-LatticePathWithinRoot -Path ([string]$active.version_root) -Root ([string]$paths.InstallRoot))) {
                throw 'LATTICE_UNINSTALL_ACTIVE_RECEIPT_MISMATCH'
            }
        }
        Assert-LatticeVersionFilesDeletable -VersionDirectories $versionDirectories
        $operationId = [guid]::NewGuid().ToString('N').Substring(0, 12)
        $uninstallBackupRoot = Join-Path ([string]$paths.StagingRoot) ('uninstall-' + $operationId)
        $tombstonePath = Join-Path ([string]$paths.InstallRoot) ('.r-' + $operationId)
        [IO.Directory]::CreateDirectory($uninstallBackupRoot) | Out-Null
        Write-LatticeStageOwner -StageRoot $uninstallBackupRoot -Owner $owner -Purpose 'INSTALL'
        $shortcutExisted = Test-Path -LiteralPath ([string]$paths.ShortcutPath) -PathType Leaf
        $activeExisted = Test-Path -LiteralPath ([string]$paths.ActiveInstall) -PathType Leaf
        if ($shortcutExisted) {
            Copy-Item -LiteralPath ([string]$paths.ShortcutPath) -Destination (Join-Path $uninstallBackupRoot 'shortcut.lnk')
        }
        if ($activeExisted) {
            Copy-Item -LiteralPath ([string]$paths.ActiveInstall) -Destination (Join-Path $uninstallBackupRoot 'active-install.json')
        }
        $registrySnapshot = Get-LatticeRegistrySnapshot -Path ([string]$paths.RegistryPath)
        $registrySnapshotPath = Join-Path $uninstallBackupRoot 'registry-snapshot.json'
        Write-LatticeJsonAtomic -LiteralPath $registrySnapshotPath -Value $registrySnapshot
        $activeSourceCommit = if ($null -ne (Get-Variable active -ErrorAction SilentlyContinue)) { [string]$active.source_commit } else { '' }
        $journal = [PSCustomObject][ordered]@{
            schema_version = $script:LatticeUninstallJournalSchema
            product = $script:LatticeProduct
            install_id = [string]$owner.install_id
            operation_id = $operationId
            phase = 'PREPARED'
            active_source_commit = $activeSourceCommit
            version_count = $versionDirectories.Count
            tombstone_path = $tombstonePath
            shortcut_path = [string]$paths.ShortcutPath
            registry_path = [string]$paths.RegistryPath
            active_install_path = [string]$paths.ActiveInstall
            shortcut_existed = $shortcutExisted
            shortcut_backup_sha256 = if ($shortcutExisted) { Get-LatticeSha256Hex -LiteralPath (Join-Path $uninstallBackupRoot 'shortcut.lnk') } else { '' }
            active_existed = $activeExisted
            active_backup_sha256 = if ($activeExisted) { Get-LatticeSha256Hex -LiteralPath (Join-Path $uninstallBackupRoot 'active-install.json') } else { '' }
            registry_snapshot_sha256 = Get-LatticeSha256Hex -LiteralPath $registrySnapshotPath
        }
        Set-LatticeUninstallJournalPhase -BackupRoot $uninstallBackupRoot -Journal $journal -Phase 'PREPARED'
        Write-LatticeStageOwner -StageRoot $uninstallBackupRoot -Owner $owner -Purpose 'UNINSTALL_BACKUP' -SourceCommit $activeSourceCommit
        $stageMarker = Get-Content -LiteralPath (Join-Path $uninstallBackupRoot '.lattice-stage-owner.json') -Raw | ConvertFrom-Json
        Complete-LatticeInterruptedUninstall `
            -Paths $paths `
            -Owner $owner `
            -BackupRoot $uninstallBackupRoot `
            -StageMarker $stageMarker
        return [ordered]@{
            result = 'PASS'
            action = 'UNINSTALLED'
            install_root = [string]$paths.InstallRoot
            preserved_user_data = @(
                '%LOCALAPPDATA%\LATTICE\control\lattice-control.db',
                '%LOCALAPPDATA%\LATTICE\ControlDesktop\WebView2')
        }
    }
    finally {
        Exit-LatticeInstallerMutex -Mutex $mutex
    }
}
