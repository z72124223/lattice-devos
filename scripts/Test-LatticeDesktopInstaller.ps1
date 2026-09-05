[CmdletBinding()]
param(
    [string]$InstallerBundleArchive = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$OutputEncoding = [Text.UTF8Encoding]::new($false)

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$productFiles = @(
    'Install-LATTICE.ps1',
    'Uninstall-LATTICE.ps1',
    'LatticeDesktopInstaller.Common.ps1',
    'INSTALL-LATTICE.txt')
foreach ($name in $productFiles) {
    if (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot $name) -PathType Leaf)) {
        throw "DESKTOP_INSTALLER_PRODUCT_SCRIPT_MISSING:$name"
    }
}

. (Join-Path $PSScriptRoot 'LatticeDesktopInstaller.Common.ps1')

function Assert-InstallerTest {
    param(
        [Parameter(Mandatory)][bool]$Condition,
        [Parameter(Mandatory)][string]$Failure
    )

    if (-not $Condition) {
        throw $Failure
    }
}

function New-TestTextFile {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$Value
    )

    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Path)) | Out-Null
    [IO.File]::WriteAllText($Path, $Value, [Text.UTF8Encoding]::new($false))
}

function New-TestZipFromDirectory {
    param(
        [Parameter(Mandatory)][string]$SourceRoot,
        [Parameter(Mandatory)][string]$ArchivePath
    )

    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::Open($ArchivePath, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($directory in @(Get-ChildItem -LiteralPath $SourceRoot -Directory -Recurse | Sort-Object FullName)) {
            $entryName = $directory.FullName.Substring($SourceRoot.Length + 1) + '\'
            $archive.CreateEntry($entryName) | Out-Null
        }
        foreach ($file in @(Get-ChildItem -LiteralPath $SourceRoot -File -Recurse | Sort-Object FullName)) {
            $entryName = $file.FullName.Substring($SourceRoot.Length + 1).Replace('\', '/')
            [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                $archive,
                $file.FullName,
                $entryName,
                [IO.Compression.CompressionLevel]::Optimal) | Out-Null
        }
    }
    finally {
        $archive.Dispose()
    }
}

function New-TestInstallerBundle {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$SourceCommit,
        [Parameter(Mandatory)][string]$PayloadMarker
    )

    $bundleRoot = Join-Path $Root ('bundle-' + $SourceCommit.Substring(0, 8))
    $payloadRoot = Join-Path $Root ('payload-' + $SourceCommit.Substring(0, 8))
    [IO.Directory]::CreateDirectory($bundleRoot) | Out-Null
    [IO.Directory]::CreateDirectory($payloadRoot) | Out-Null
    New-TestTextFile -Path (Join-Path $payloadRoot 'LATTICE.exe') -Value "fake-executable-$PayloadMarker"
    New-TestTextFile -Path (Join-Path $payloadRoot 'LATTICE.dll') -Value "fake-assembly-$PayloadMarker"
    New-TestTextFile -Path (Join-Path $payloadRoot 'PORTABLE_RELEASE_CANDIDATE.txt') -Value "fixture-$PayloadMarker"
    New-TestTextFile `
        -Path (Join-Path $payloadRoot 'control-runtime\apps\lattice-control\src\wsl2-provider-subtree-reconcile.mjs') `
        -Value "realistic-deep-payload-$PayloadMarker"
    $payloadFiles = @(Get-ChildItem -LiteralPath $payloadRoot -File -Recurse |
        Sort-Object FullName |
        ForEach-Object {
            [ordered]@{
                path = $_.FullName.Substring($payloadRoot.Length + 1).Replace('\', '/')
                length = [long]$_.Length
                sha256 = Get-LatticeSha256Hex -LiteralPath $_.FullName
            }
        })
    $payloadManifest = [ordered]@{
        schema_version = 'lattice.control.desktop-portable-candidate.v2'
        artifact_type = 'PORTABLE_RELEASE_CANDIDATE'
        source_commit = $SourceCommit
        runtime_identifier = 'win-x64'
        self_contained = $true
        launch = 'LATTICE.exe'
        control_origin = 'http://127.0.0.1:4317/'
        webview_user_data = '%LOCALAPPDATA%\LATTICE\ControlDesktop\WebView2'
        executable_sha256 = [string](@($payloadFiles | Where-Object { $_.path -ceq 'LATTICE.exe' })[0].sha256)
        files = $payloadFiles
    }
    Write-LatticeJsonAtomic -LiteralPath (Join-Path $payloadRoot 'candidate-manifest.json') -Value $payloadManifest
    New-TestZipFromDirectory -SourceRoot $payloadRoot -ArchivePath (Join-Path $bundleRoot 'payload.zip')
    foreach ($name in $productFiles) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $bundleRoot $name)
    }
    $bundleFiles = @(Get-ChildItem -LiteralPath $bundleRoot -File -Recurse |
        Sort-Object FullName |
        ForEach-Object {
            [ordered]@{
                path = $_.FullName.Substring($bundleRoot.Length + 1).Replace('\', '/')
                length = [long]$_.Length
                sha256 = Get-LatticeSha256Hex -LiteralPath $_.FullName
            }
        })
    $payloadEntry = @($bundleFiles | Where-Object { $_.path -ceq 'payload.zip' })[0]
    $installManifest = [ordered]@{
        schema_version = 'lattice.control.desktop-per-user-installer.v1'
        artifact_type = 'WINDOWS_PER_USER_INSTALLER'
        source_commit = $SourceCommit
        runtime_identifier = 'win-x64'
        payload = [ordered]@{
            path = 'payload.zip'
            length = [long]$payloadEntry.length
            sha256 = [string]$payloadEntry.sha256
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
        files = $bundleFiles
    }
    Write-LatticeJsonAtomic -LiteralPath (Join-Path $bundleRoot 'install-manifest.json') -Value $installManifest
    Remove-Item -LiteralPath $payloadRoot -Recurse -Force
    return $bundleRoot
}

function Get-ActiveCommit {
    param([Parameter(Mandatory)][object]$Paths)

    return [string](Get-Content -LiteralPath ([string]$Paths.ActiveInstall) -Raw | ConvertFrom-Json).source_commit
}

function Invoke-InstallerWrapper {
    param(
        [Parameter(Mandatory)][string]$BundleRoot,
        [Parameter(Mandatory)][string]$SandboxRoot,
        [Parameter(Mandatory)][string]$RegistryId
    )

    $output = & (Join-Path $BundleRoot 'Install-LATTICE.ps1') `
        -BundleRoot $BundleRoot `
        -TestSandboxRoot $SandboxRoot `
        -TestRegistryId $RegistryId
    return (($output -join [Environment]::NewLine) | ConvertFrom-Json)
}

function Invoke-InstallerDefaultBundleRoot {
    param(
        [Parameter(Mandatory)][string]$BundleRoot,
        [Parameter(Mandatory)][string]$SandboxRoot,
        [Parameter(Mandatory)][string]$RegistryId
    )

    $output = & (Join-Path $BundleRoot 'Install-LATTICE.ps1') `
        -TestSandboxRoot $SandboxRoot `
        -TestRegistryId $RegistryId
    return (($output -join [Environment]::NewLine) | ConvertFrom-Json)
}

function Invoke-InstallerHardKill {
    param(
        [Parameter(Mandatory)][string]$CommonPath,
        [Parameter(Mandatory)][string]$BundleRoot,
        [Parameter(Mandatory)][string]$SandboxRoot,
        [Parameter(Mandatory)][string]$RegistryId,
        [Parameter(Mandatory)][string]$HookName,
        [ValidateSet('INSTALL', 'UNINSTALL')][string]$Operation = 'INSTALL'
    )

    $workerPath = Join-Path $SandboxRoot ('hard-kill-' + [guid]::NewGuid().ToString('N') + '.ps1')
    $workerSource = @'
param(
    [Parameter(Mandatory)][string]$CommonPath,
    [Parameter(Mandatory)][string]$BundleRoot,
    [Parameter(Mandatory)][string]$SandboxRoot,
    [Parameter(Mandatory)][string]$RegistryId,
    [Parameter(Mandatory)][string]$HookName,
    [Parameter(Mandatory)][string]$Operation
)
$ErrorActionPreference = 'Stop'
. $CommonPath
function Invoke-LatticeInstallerHook {
    param([Parameter(Mandatory)][string]$Name)
    if ($Name -ceq $HookName) {
        Stop-Process -Id $PID -Force
    }
}
if ($Operation -ceq 'INSTALL') {
    Invoke-LatticeDesktopInstall `
        -BundleRoot $BundleRoot `
        -TestSandboxRoot $SandboxRoot `
        -TestRegistryId $RegistryId | Out-Null
}
else {
    Invoke-LatticeDesktopUninstall `
        -TestSandboxRoot $SandboxRoot `
        -TestRegistryId $RegistryId | Out-Null
}
throw 'DESKTOP_INSTALLER_HARD_KILL_HOOK_NOT_REACHED'
'@
    [IO.File]::WriteAllText($workerPath, $workerSource, [Text.UTF8Encoding]::new($false))
    try {
        & (Get-LatticePowerShellPath) `
            -NoLogo -NoProfile -ExecutionPolicy Bypass `
            -File $workerPath `
            -CommonPath $CommonPath `
            -BundleRoot $BundleRoot `
            -SandboxRoot $SandboxRoot `
            -RegistryId $RegistryId `
            -HookName $HookName `
            -Operation $Operation
        Assert-InstallerTest ($LASTEXITCODE -ne 0) 'DESKTOP_INSTALLER_HARD_KILL_PROCESS_SURVIVED'
    }
    finally {
        if (Test-Path -LiteralPath $workerPath) {
            Remove-Item -LiteralPath $workerPath -Force
        }
    }
}

$latticeRegistryParent = 'HKCU:\Software\LATTICE'
$installerTestsRegistryParent = 'HKCU:\Software\LATTICE\InstallerTests'
$latticeRegistryParentExisted = Test-Path -LiteralPath $latticeRegistryParent
$installerTestsRegistryParentExisted = Test-Path -LiteralPath $installerTestsRegistryParent
$temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) ('lattice-desktop-installer-' + [guid]::NewGuid().ToString('N'))
$registryId = [guid]::NewGuid().ToString('N')
$context = $null
$paths = $null
$foreignContext = $null
$foreignSandbox = ''
$junctionContext = $null
$junctionSandbox = ''
$junctionPath = ''
$ancestorJunctionPath = ''
$ancestorJunctionParent = ''
$crashContext = $null
$crashSandbox = ''
$orphanContext = $null
$orphanSandbox = ''
$primaryFailure = $null
$cleanupFailure = $null
$result = $null
$script:InstallerTestFailHook = ''

# This overrides the no-op production hook only inside this focused test process.
function Invoke-LatticeInstallerHook {
    param([Parameter(Mandatory)][string]$Name)

    if ($script:InstallerTestFailHook -ceq $Name) {
        throw "DESKTOP_INSTALLER_INJECTED_FAILURE:$Name"
    }
}

try {
    [IO.Directory]::CreateDirectory($temporaryRoot) | Out-Null
    $context = Resolve-LatticeInstallContext -TestSandboxRoot $temporaryRoot -TestRegistryId $registryId
    $paths = Get-LatticeInstallPaths -Context $context
    $controlDataPath = Join-Path $temporaryRoot 'LocalAppData\LATTICE\control\lattice-control.db'
    $webViewDataPath = Join-Path $temporaryRoot 'LocalAppData\LATTICE\ControlDesktop\WebView2\sentinel.dat'
    $foreignSiblingPath = Join-Path $temporaryRoot 'LocalAppData\Programs\foreign-owned.txt'
    New-TestTextFile -Path $controlDataPath -Value 'control-data-must-survive'
    New-TestTextFile -Path $webViewDataPath -Value 'webview-data-must-survive'
    New-TestTextFile -Path $foreignSiblingPath -Value 'foreign-program-data-must-survive'
    $controlDataHash = Get-LatticeSha256Hex -LiteralPath $controlDataPath
    $webViewDataHash = Get-LatticeSha256Hex -LiteralPath $webViewDataPath
    $foreignSiblingHash = Get-LatticeSha256Hex -LiteralPath $foreignSiblingPath

    $commit1 = '1111111111111111111111111111111111111111'
    $commit2 = '2222222222222222222222222222222222222222'
    $commit3 = '3333333333333333333333333333333333333333'
    $bundleRoot = Join-Path $temporaryRoot 'bundles'
    [IO.Directory]::CreateDirectory($bundleRoot) | Out-Null
    $bundle1 = New-TestInstallerBundle -Root $bundleRoot -SourceCommit $commit1 -PayloadMarker 'v1'
    $bundle2 = New-TestInstallerBundle -Root $bundleRoot -SourceCommit $commit2 -PayloadMarker 'v2'
    $bundle3 = New-TestInstallerBundle -Root $bundleRoot -SourceCommit $commit3 -PayloadMarker 'v3'

    $crashSandbox = Join-Path ([IO.Path]::GetTempPath()) ('lattice-desktop-installer-c-' + [guid]::NewGuid().ToString('N').Substring(0, 12))
    [IO.Directory]::CreateDirectory($crashSandbox) | Out-Null
    $crashRegistryId = [guid]::NewGuid().ToString('N')
    $crashContext = Resolve-LatticeInstallContext -TestSandboxRoot $crashSandbox -TestRegistryId $crashRegistryId
    $crashPaths = Get-LatticeInstallPaths -Context $crashContext
    Invoke-InstallerHardKill `
        -CommonPath (Join-Path $bundle1 'LatticeDesktopInstaller.Common.ps1') `
        -BundleRoot $bundle1 `
        -SandboxRoot $crashSandbox `
        -RegistryId $crashRegistryId `
        -HookName 'AfterRegistryCoreValuesWritten'
    $partialRegistry = Get-ItemProperty -LiteralPath ([string]$crashPaths.RegistryPath)
    Assert-InstallerTest ($null -eq $partialRegistry.PSObject.Properties['LatticeInstallId']) 'DESKTOP_INSTALLER_HARD_KILL_DID_NOT_LEAVE_PARTIAL_REGISTRY'
    $crashRecovered = Invoke-InstallerWrapper -BundleRoot $bundle1 -SandboxRoot $crashSandbox -RegistryId $crashRegistryId
    Assert-InstallerTest ($crashRecovered.result -ceq 'PASS') 'DESKTOP_INSTALLER_FRESH_PROCESS_ACTIVATION_NOT_RECOVERED'
    Assert-InstallerTest ((Get-ActiveCommit -Paths $crashPaths) -ceq $commit1) 'DESKTOP_INSTALLER_FRESH_PROCESS_ACTIVE_COMMIT_MISMATCH'
    Invoke-LatticeDesktopUninstall -TestSandboxRoot $crashSandbox -TestRegistryId $crashRegistryId | Out-Null

    Invoke-InstallerWrapper -BundleRoot $bundle1 -SandboxRoot $crashSandbox -RegistryId $crashRegistryId | Out-Null
    Invoke-InstallerWrapper -BundleRoot $bundle2 -SandboxRoot $crashSandbox -RegistryId $crashRegistryId | Out-Null
    Invoke-InstallerHardKill `
        -CommonPath (Join-Path $bundle2 'LatticeDesktopInstaller.Common.ps1') `
        -BundleRoot $bundle2 `
        -SandboxRoot $crashSandbox `
        -RegistryId $crashRegistryId `
        -HookName 'AfterVersionsTombstoned' `
        -Operation 'UNINSTALL'
    $uninstallRecovered = Invoke-LatticeDesktopUninstall -TestSandboxRoot $crashSandbox -TestRegistryId $crashRegistryId
    Assert-InstallerTest ($uninstallRecovered.result -ceq 'PASS' -and $uninstallRecovered.action -ceq 'UNINSTALLED') 'DESKTOP_INSTALLER_INTERRUPTED_UNINSTALL_NOT_RECOVERED'
    Assert-InstallerTest (-not (Test-Path -LiteralPath ([string]$crashPaths.InstallRoot))) 'DESKTOP_INSTALLER_INTERRUPTED_UNINSTALL_ROOT_REMAINS'
    Invoke-InstallerHardKill `
        -CommonPath (Join-Path $bundle1 'LatticeDesktopInstaller.Common.ps1') `
        -BundleRoot $bundle1 `
        -SandboxRoot $crashSandbox `
        -RegistryId $crashRegistryId `
        -HookName 'AfterOwnerMarkerPrepared'
    $pendingOwnerPath = Join-Path ([IO.Path]::GetDirectoryName([string]$crashPaths.InstallRoot)) '.LATTICE.install-owner.pending.json'
    Assert-InstallerTest (Test-Path -LiteralPath $pendingOwnerPath -PathType Leaf) 'DESKTOP_INSTALLER_OWNER_PENDING_MARKER_MISSING_AFTER_KILL'
    Assert-InstallerTest (@(Get-ChildItem -LiteralPath ([string]$crashPaths.InstallRoot) -Force).Count -eq 0) 'DESKTOP_INSTALLER_OWNER_KILL_LEFT_UNOWNED_ROOT_CONTENT'
    $ownerRecovered = Invoke-InstallerWrapper -BundleRoot $bundle1 -SandboxRoot $crashSandbox -RegistryId $crashRegistryId
    Assert-InstallerTest ($ownerRecovered.result -ceq 'PASS') 'DESKTOP_INSTALLER_OWNER_BOOTSTRAP_NOT_RECOVERED'
    Assert-InstallerTest (-not (Test-Path -LiteralPath $pendingOwnerPath)) 'DESKTOP_INSTALLER_OWNER_PENDING_MARKER_REMAINS'
    Invoke-LatticeDesktopUninstall -TestSandboxRoot $crashSandbox -TestRegistryId $crashRegistryId | Out-Null

    $first = Invoke-InstallerDefaultBundleRoot -BundleRoot $bundle1 -SandboxRoot $temporaryRoot -RegistryId $registryId
    Assert-InstallerTest ($first.result -ceq 'PASS' -and $first.action -ceq 'INSTALLED') 'DESKTOP_INSTALLER_FRESH_INSTALL_FAILED'
    Assert-InstallerTest ((Get-ActiveCommit -Paths $paths) -ceq $commit1) 'DESKTOP_INSTALLER_FRESH_ACTIVE_COMMIT_MISMATCH'
    Assert-InstallerTest (Test-Path -LiteralPath ([string]$paths.ShortcutPath) -PathType Leaf) 'DESKTOP_INSTALLER_SHORTCUT_MISSING'
    Assert-InstallerTest ((Get-LatticeShortcutTarget -ShortcutPath ([string]$paths.ShortcutPath)) -ceq [IO.Path]::GetFullPath((Join-Path ([string]$paths.VersionsRoot) "$commit1\app\LATTICE.exe"))) 'DESKTOP_INSTALLER_SHORTCUT_TARGET_MISMATCH'
    $registry = Get-ItemProperty -LiteralPath ([string]$paths.RegistryPath)
    Assert-InstallerTest ([string]$registry.LatticeSourceCommit -ceq $commit1) 'DESKTOP_INSTALLER_REGISTRY_COMMIT_MISMATCH'
    Assert-InstallerTest ([string]$registry.LatticeProduct -ceq 'LATTICE_CONTROL_DESKTOP') 'DESKTOP_INSTALLER_REGISTRY_OWNER_MISSING'
    Assert-InstallerTest (@(Get-ChildItem -LiteralPath ([string]$paths.StagingRoot) -Force).Count -eq 0) 'DESKTOP_INSTALLER_STAGING_NOT_EMPTY'

    $reentry = Invoke-InstallerWrapper -BundleRoot $bundle1 -SandboxRoot $temporaryRoot -RegistryId $registryId
    Assert-InstallerTest ($reentry.action -ceq 'REUSED') 'DESKTOP_INSTALLER_REENTRY_NOT_REUSED'
    Assert-InstallerTest (@(Get-ChildItem -LiteralPath ([string]$paths.VersionsRoot) -Directory).Count -eq 1) 'DESKTOP_INSTALLER_REENTRY_DUPLICATED_VERSION'

    $corruptBundle = Join-Path $bundleRoot 'bundle-corrupt'
    Copy-Item -LiteralPath $bundle2 -Destination $corruptBundle -Recurse
    [IO.File]::AppendAllText((Join-Path $corruptBundle 'payload.zip'), 'corrupt')
    $corruptFailed = $false
    try {
        Invoke-LatticeDesktopInstall `
            -BundleRoot $corruptBundle `
            -TestSandboxRoot $temporaryRoot `
            -TestRegistryId $registryId | Out-Null
    }
    catch {
        $corruptFailed = $_.Exception.Message -like 'LATTICE_INSTALL_BUNDLE_FILE_MISMATCH:*'
    }
    Assert-InstallerTest $corruptFailed 'DESKTOP_INSTALLER_CORRUPT_BUNDLE_NOT_REJECTED'
    Assert-InstallerTest ((Get-ActiveCommit -Paths $paths) -ceq $commit1) 'DESKTOP_INSTALLER_CORRUPT_BUNDLE_CHANGED_ACTIVE'

    $upgrade = Invoke-LatticeDesktopInstall `
        -BundleRoot $bundle2 `
        -TestSandboxRoot $temporaryRoot `
        -TestRegistryId $registryId
    Assert-InstallerTest ($upgrade.action -ceq 'INSTALLED' -and (Get-ActiveCommit -Paths $paths) -ceq $commit2) 'DESKTOP_INSTALLER_UPGRADE_FAILED'
    Assert-InstallerTest (@(Get-ChildItem -LiteralPath ([string]$paths.VersionsRoot) -Directory).Count -eq 2) 'DESKTOP_INSTALLER_UPGRADE_DID_NOT_RETAIN_PRIOR'

    $rollback = Invoke-LatticeDesktopInstall `
        -RollbackToCommit $commit1 `
        -TestSandboxRoot $temporaryRoot `
        -TestRegistryId $registryId
    Assert-InstallerTest ($rollback.action -ceq 'ROLLED_BACK' -and (Get-ActiveCommit -Paths $paths) -ceq $commit1) 'DESKTOP_INSTALLER_ROLLBACK_FAILED'

    $tamperedReceiptPath = Join-Path ([string]$paths.VersionsRoot) "$commit2\install-receipt.json"
    $originalReceiptBytes = [IO.File]::ReadAllBytes($tamperedReceiptPath)
    $tamperedReceipt = Get-Content -LiteralPath $tamperedReceiptPath -Raw | ConvertFrom-Json
    $tamperedReceipt.payload_file_count = [int]$tamperedReceipt.payload_file_count + 1
    Write-LatticeJsonAtomic -LiteralPath $tamperedReceiptPath -Value $tamperedReceipt
    $tamperedReceiptRejected = $false
    try {
        Invoke-LatticeDesktopInstall `
            -RollbackToCommit $commit2 `
            -TestSandboxRoot $temporaryRoot `
            -TestRegistryId $registryId | Out-Null
    }
    catch {
        $tamperedReceiptRejected = $_.Exception.Message -ceq 'LATTICE_INSTALL_RECEIPT_MISMATCH'
    }
    finally {
        [IO.File]::WriteAllBytes($tamperedReceiptPath, $originalReceiptBytes)
    }
    Assert-InstallerTest $tamperedReceiptRejected 'DESKTOP_INSTALLER_TAMPERED_RECEIPT_NOT_REJECTED'
    Assert-InstallerTest ((Get-ActiveCommit -Paths $paths) -ceq $commit1) 'DESKTOP_INSTALLER_TAMPERED_RECEIPT_CHANGED_ACTIVE'

    $script:InstallerTestFailHook = 'AfterRegistryActivated'
    $activationFailed = $false
    try {
        Invoke-LatticeDesktopInstall `
            -BundleRoot $bundle3 `
            -TestSandboxRoot $temporaryRoot `
            -TestRegistryId $registryId | Out-Null
    }
    catch {
        $activationFailed = $_.Exception.Message -ceq 'DESKTOP_INSTALLER_INJECTED_FAILURE:AfterRegistryActivated'
    }
    finally {
        $script:InstallerTestFailHook = ''
    }
    Assert-InstallerTest $activationFailed 'DESKTOP_INSTALLER_ACTIVATION_FAILURE_NOT_OBSERVED'
    Assert-InstallerTest ((Get-ActiveCommit -Paths $paths) -ceq $commit1) 'DESKTOP_INSTALLER_FAILURE_CHANGED_ACTIVE'
    Assert-InstallerTest ((Get-LatticeShortcutTarget -ShortcutPath ([string]$paths.ShortcutPath)) -ceq [IO.Path]::GetFullPath((Join-Path ([string]$paths.VersionsRoot) "$commit1\app\LATTICE.exe"))) 'DESKTOP_INSTALLER_FAILURE_DID_NOT_RESTORE_SHORTCUT'
    $registryAfterFailure = Get-ItemProperty -LiteralPath ([string]$paths.RegistryPath)
    Assert-InstallerTest ([string]$registryAfterFailure.LatticeSourceCommit -ceq $commit1) 'DESKTOP_INSTALLER_FAILURE_DID_NOT_RESTORE_REGISTRY'

    $owner = Get-Content -LiteralPath ([string]$paths.OwnerMarker) -Raw | ConvertFrom-Json
    $staleStage = Join-Path ([string]$paths.StagingRoot) ('stale-' + [guid]::NewGuid().ToString('N'))
    [IO.Directory]::CreateDirectory($staleStage) | Out-Null
    Write-LatticeStageOwner -StageRoot $staleStage -Owner $owner -Purpose 'INSTALL' -SourceCommit $commit2
    New-TestTextFile -Path (Join-Path $staleStage 'partial.tmp') -Value 'interrupted-stage'
    Invoke-LatticeDesktopInstall `
        -RollbackToCommit $commit1 `
        -TestSandboxRoot $temporaryRoot `
        -TestRegistryId $registryId | Out-Null
    Assert-InstallerTest (-not (Test-Path -LiteralPath $staleStage)) 'DESKTOP_INSTALLER_STALE_OWNED_STAGE_NOT_RECONCILED'

    $exactCandidate = [ordered]@{ tested = $false }
    if (-not [string]::IsNullOrWhiteSpace($InstallerBundleArchive)) {
        $archiveFull = [IO.Path]::GetFullPath($InstallerBundleArchive)
        if (-not (Test-Path -LiteralPath $archiveFull -PathType Leaf)) {
            throw 'DESKTOP_INSTALLER_EXACT_BUNDLE_ARCHIVE_MISSING'
        }
        $exactRoot = Join-Path $temporaryRoot 'exact-candidate'
        Expand-Archive -LiteralPath $archiveFull -DestinationPath $exactRoot
        $exactBundle = Get-LatticeInstallerBundle -BundleRoot $exactRoot
        $exactInstall = Invoke-InstallerWrapper -BundleRoot $exactRoot -SandboxRoot $temporaryRoot -RegistryId $registryId
        Assert-InstallerTest ((Get-ActiveCommit -Paths $paths) -ceq [string]$exactBundle.SourceCommit) 'DESKTOP_INSTALLER_EXACT_BUNDLE_NOT_ACTIVE'
        $exactCandidate = [ordered]@{
            tested = $true
            archive = $archiveFull
            archive_sha256 = Get-LatticeSha256Hex -LiteralPath $archiveFull
            source_commit = [string]$exactBundle.SourceCommit
            action = [string]$exactInstall.action
            manifest_sha256 = [string]$exactBundle.ManifestSha256
        }
    }

    $activeCommitBeforeUninstall = Get-ActiveCommit -Paths $paths
    $activeAppRoot = Join-Path ([string]$paths.VersionsRoot) "$activeCommitBeforeUninstall\app"
    $foreignEmptyDirectory = Join-Path $activeAppRoot 'foreign-empty-directory'
    [IO.Directory]::CreateDirectory($foreignEmptyDirectory) | Out-Null
    $emptyDirectoryRejected = $false
    try {
        Invoke-LatticeDesktopUninstall -TestSandboxRoot $temporaryRoot -TestRegistryId $registryId | Out-Null
    }
    catch {
        $emptyDirectoryRejected = $_.Exception.Message -ceq 'LATTICE_INSTALL_PAYLOAD_DIRECTORY_SET_MISMATCH'
    }
    Assert-InstallerTest $emptyDirectoryRejected 'DESKTOP_INSTALLER_EMPTY_DIRECTORY_NOT_REJECTED'
    Assert-InstallerTest (Test-Path -LiteralPath $foreignEmptyDirectory -PathType Container) 'DESKTOP_INSTALLER_EMPTY_DIRECTORY_DELETED'
    [IO.Directory]::Delete($foreignEmptyDirectory)

    $alternateStreamFile = Join-Path $activeAppRoot 'LATTICE.dll'
    Set-Content -LiteralPath ($alternateStreamFile + ':foreign-state') -Value 'must-not-be-deleted'
    $alternateStreamRejected = $false
    try {
        Invoke-LatticeDesktopUninstall -TestSandboxRoot $temporaryRoot -TestRegistryId $registryId | Out-Null
    }
    catch {
        $alternateStreamRejected = $_.Exception.Message -like 'LATTICE_INSTALL_PAYLOAD_ALTERNATE_DATA_STREAM_REJECTED:*'
    }
    Assert-InstallerTest $alternateStreamRejected 'DESKTOP_INSTALLER_ALTERNATE_DATA_STREAM_NOT_REJECTED'
    Assert-InstallerTest (@(Get-Item -LiteralPath $alternateStreamFile -Stream 'foreign-state').Count -eq 1) 'DESKTOP_INSTALLER_ALTERNATE_DATA_STREAM_DELETED'
    Remove-Item -LiteralPath $alternateStreamFile -Stream 'foreign-state' -Force

    $unknownRegistrySubkey = Join-Path ([string]$paths.RegistryPath) 'ForeignState'
    New-Item -Path $unknownRegistrySubkey -Force | Out-Null
    $unknownRegistryRejected = $false
    try {
        Invoke-LatticeDesktopUninstall -TestSandboxRoot $temporaryRoot -TestRegistryId $registryId | Out-Null
    }
    catch {
        $unknownRegistryRejected = $_.Exception.Message -ceq 'LATTICE_INSTALL_REGISTRY_UNKNOWN_SUBKEY'
    }
    Assert-InstallerTest $unknownRegistryRejected 'DESKTOP_INSTALLER_UNKNOWN_REGISTRY_SUBKEY_NOT_REJECTED'
    Assert-InstallerTest (Test-Path -LiteralPath $unknownRegistrySubkey) 'DESKTOP_INSTALLER_UNKNOWN_REGISTRY_SUBKEY_DELETED'
    Remove-Item -LiteralPath $unknownRegistrySubkey -Force

    $unknownVersionFile = Join-Path ([string]$paths.VersionsRoot) 'foreign-owned.txt'
    New-TestTextFile -Path $unknownVersionFile -Value 'must-not-be-deleted'
    $unknownVersionRejected = $false
    try {
        Invoke-LatticeDesktopUninstall -TestSandboxRoot $temporaryRoot -TestRegistryId $registryId | Out-Null
    }
    catch {
        $unknownVersionRejected = $_.Exception.Message -like 'LATTICE_UNINSTALL_UNKNOWN_VERSION_ENTRY:*'
    }
    Assert-InstallerTest $unknownVersionRejected 'DESKTOP_INSTALLER_UNKNOWN_VERSION_FILE_NOT_REJECTED'
    Assert-InstallerTest (Test-Path -LiteralPath $unknownVersionFile -PathType Leaf) 'DESKTOP_INSTALLER_UNKNOWN_VERSION_FILE_DELETED'
    Assert-InstallerTest (Test-Path -LiteralPath ([string]$paths.ShortcutPath) -PathType Leaf) 'DESKTOP_INSTALLER_UNKNOWN_VERSION_REMOVED_SHORTCUT'
    Assert-InstallerTest (Test-Path -LiteralPath ([string]$paths.RegistryPath)) 'DESKTOP_INSTALLER_UNKNOWN_VERSION_REMOVED_REGISTRY'
    Remove-Item -LiteralPath $unknownVersionFile -Force

    $lockedPayloadPath = Join-Path ([string]$paths.VersionsRoot) "$activeCommitBeforeUninstall\app\LATTICE.dll"
    $lockedStream = [IO.File]::Open($lockedPayloadPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    $lockedPayloadRejected = $false
    try {
        Invoke-LatticeDesktopUninstall -TestSandboxRoot $temporaryRoot -TestRegistryId $registryId | Out-Null
    }
    catch {
        $lockedPayloadRejected = $_.Exception.Message -like 'LATTICE_UNINSTALL_FILE_NOT_DELETABLE:*'
    }
    finally {
        $lockedStream.Dispose()
    }
    Assert-InstallerTest $lockedPayloadRejected 'DESKTOP_INSTALLER_LOCKED_PAYLOAD_NOT_REJECTED'
    Assert-InstallerTest ((Get-ActiveCommit -Paths $paths) -ceq $activeCommitBeforeUninstall) 'DESKTOP_INSTALLER_LOCKED_PAYLOAD_CHANGED_ACTIVE'
    Assert-InstallerTest (Test-Path -LiteralPath ([string]$paths.ShortcutPath) -PathType Leaf) 'DESKTOP_INSTALLER_LOCKED_PAYLOAD_REMOVED_SHORTCUT'
    Assert-InstallerTest (Test-Path -LiteralPath ([string]$paths.RegistryPath)) 'DESKTOP_INSTALLER_LOCKED_PAYLOAD_REMOVED_REGISTRY'

    $activeUninstaller = Join-Path ([string]$paths.VersionsRoot) "$activeCommitBeforeUninstall\installer\Uninstall-LATTICE.ps1"
    $uninstallOutput = & $activeUninstaller `
        -TestSandboxRoot $temporaryRoot `
        -TestRegistryId $registryId
    $uninstall = (($uninstallOutput -join [Environment]::NewLine) | ConvertFrom-Json)
    Assert-InstallerTest ($uninstall.result -ceq 'PASS' -and $uninstall.action -ceq 'UNINSTALLED') 'DESKTOP_INSTALLER_UNINSTALL_FAILED'
    Assert-InstallerTest (-not (Test-Path -LiteralPath ([string]$paths.InstallRoot))) 'DESKTOP_INSTALLER_PROGRAM_FILES_REMAIN'
    Assert-InstallerTest (-not (Test-Path -LiteralPath ([string]$paths.ShortcutPath))) 'DESKTOP_INSTALLER_SHORTCUT_REMAINS'
    Assert-InstallerTest (-not (Test-Path -LiteralPath ([string]$paths.RegistryPath))) 'DESKTOP_INSTALLER_REGISTRY_REMAINS'
    Assert-InstallerTest ((Get-LatticeSha256Hex -LiteralPath $controlDataPath) -ceq $controlDataHash) 'DESKTOP_INSTALLER_CONTROL_DATA_CHANGED'
    Assert-InstallerTest ((Get-LatticeSha256Hex -LiteralPath $webViewDataPath) -ceq $webViewDataHash) 'DESKTOP_INSTALLER_WEBVIEW_DATA_CHANGED'
    Assert-InstallerTest ((Get-LatticeSha256Hex -LiteralPath $foreignSiblingPath) -ceq $foreignSiblingHash) 'DESKTOP_INSTALLER_FOREIGN_SIBLING_CHANGED'
    $secondUninstall = Invoke-LatticeDesktopUninstall -TestSandboxRoot $temporaryRoot -TestRegistryId $registryId
    Assert-InstallerTest ($secondUninstall.action -ceq 'ALREADY_ABSENT') 'DESKTOP_INSTALLER_SECOND_UNINSTALL_NOT_IDEMPOTENT'

    $foreignSandbox = Join-Path ([IO.Path]::GetTempPath()) ('lattice-desktop-installer-f-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
    $foreignRegistryId = [guid]::NewGuid().ToString('N')
    $foreignContext = Resolve-LatticeInstallContext -TestSandboxRoot $foreignSandbox -TestRegistryId $foreignRegistryId
    Set-LatticeRegistryValue `
        -Path ([string]$foreignContext.RegistryPath) `
        -Name 'DisplayName' `
        -Value 'Foreign Product' `
        -Kind String
    $foreignRegistryRejected = $false
    $foreignRegistryError = ''
    try {
        Invoke-LatticeDesktopInstall `
            -BundleRoot $bundle1 `
            -TestSandboxRoot $foreignSandbox `
            -TestRegistryId $foreignRegistryId | Out-Null
    }
    catch {
        $foreignRegistryError = $_.Exception.Message
        $foreignRegistryRejected = $foreignRegistryError -like '*LATTICE_INSTALL_REGISTRY_NOT_OWNED*'
    }
    $foreignRegistry = Get-ItemProperty -LiteralPath ([string]$foreignContext.RegistryPath)
    Assert-InstallerTest $foreignRegistryRejected "DESKTOP_INSTALLER_FOREIGN_REGISTRY_NOT_REJECTED:$foreignRegistryError"
    Assert-InstallerTest (
        [string]$foreignRegistry.DisplayName -ceq 'Foreign Product' -and
        $null -eq $foreignRegistry.PSObject.Properties['LatticeProduct']) 'DESKTOP_INSTALLER_FOREIGN_REGISTRY_CHANGED'

    $orphanSandbox = Join-Path ([IO.Path]::GetTempPath()) ('lattice-desktop-installer-o-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
    $orphanRegistryId = [guid]::NewGuid().ToString('N')
    $orphanContext = Resolve-LatticeInstallContext -TestSandboxRoot $orphanSandbox -TestRegistryId $orphanRegistryId
    Set-LatticeRegistryValue -Path ([string]$orphanContext.RegistryPath) -Name 'DisplayName' -Value 'Orphan Surface' -Kind String
    $orphanRejected = $false
    try {
        Invoke-LatticeDesktopUninstall -TestSandboxRoot $orphanSandbox -TestRegistryId $orphanRegistryId | Out-Null
    }
    catch {
        $orphanRejected = $_.Exception.Message -ceq 'LATTICE_UNINSTALL_ORPHANED_SURFACE'
    }
    Assert-InstallerTest $orphanRejected 'DESKTOP_INSTALLER_ORPHANED_SURFACE_NOT_REJECTED'
    Assert-InstallerTest (Test-Path -LiteralPath ([string]$orphanContext.RegistryPath)) 'DESKTOP_INSTALLER_ORPHANED_SURFACE_DELETED'

    $junctionSandbox = Join-Path ([IO.Path]::GetTempPath()) ('lattice-desktop-installer-j-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
    $junctionRegistryId = [guid]::NewGuid().ToString('N')
    $junctionContext = Resolve-LatticeInstallContext -TestSandboxRoot $junctionSandbox -TestRegistryId $junctionRegistryId
    $junctionTarget = Join-Path $temporaryRoot 'junction-target-must-survive'
    $junctionSentinel = Join-Path $junctionTarget 'sentinel.txt'
    New-TestTextFile -Path $junctionSentinel -Value 'junction-target-must-survive'
    $junctionPath = [string]$junctionContext.InstallRoot
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($junctionPath)) | Out-Null
    New-Item -ItemType Junction -Path $junctionPath -Target $junctionTarget | Out-Null
    $junctionRejected = $false
    try {
        Invoke-LatticeDesktopInstall `
            -BundleRoot $bundle1 `
            -TestSandboxRoot $junctionSandbox `
            -TestRegistryId $junctionRegistryId | Out-Null
    }
    catch {
        $junctionRejected = $_.Exception.Message -like 'LATTICE_INSTALL_REPARSE_POINT_REJECTED:*'
    }
    Assert-InstallerTest $junctionRejected 'DESKTOP_INSTALLER_JUNCTION_ROOT_NOT_REJECTED'
    Assert-InstallerTest ((Get-LatticeSha256Hex -LiteralPath $junctionSentinel) -ceq (Get-LatticeStringSha256Hex -Value 'junction-target-must-survive')) 'DESKTOP_INSTALLER_JUNCTION_TARGET_CHANGED'
    [IO.Directory]::Delete($junctionPath)
    $junctionPath = ''
    Assert-InstallerTest (Test-Path -LiteralPath $junctionSentinel -PathType Leaf) 'DESKTOP_INSTALLER_JUNCTION_TARGET_DELETED'

    $ancestorJunctionParent = Join-Path ([IO.Path]::GetTempPath()) ('lattice-installer-parent-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
    [IO.Directory]::CreateDirectory($ancestorJunctionParent) | Out-Null
    $ancestorJunctionTarget = Join-Path $temporaryRoot 'ancestor-junction-target-must-survive'
    $ancestorJunctionSentinel = Join-Path $ancestorJunctionTarget 'sentinel.txt'
    New-TestTextFile -Path $ancestorJunctionSentinel -Value 'ancestor-junction-target-must-survive'
    $ancestorJunctionPath = Join-Path $ancestorJunctionParent 'link'
    New-Item -ItemType Junction -Path $ancestorJunctionPath -Target $ancestorJunctionTarget | Out-Null
    $sandboxThroughJunction = Join-Path $ancestorJunctionPath ('lattice-desktop-installer-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
    $ancestorJunctionRejected = $false
    try {
        Resolve-LatticeInstallContext `
            -TestSandboxRoot $sandboxThroughJunction `
            -TestRegistryId ([guid]::NewGuid().ToString('N')) | Out-Null
    }
    catch {
        $ancestorJunctionRejected = $_.Exception.Message -like 'LATTICE_INSTALL_TEST_SANDBOX_REPARSE_POINT:*'
    }
    Assert-InstallerTest $ancestorJunctionRejected 'DESKTOP_INSTALLER_ANCESTOR_JUNCTION_NOT_REJECTED'
    Assert-InstallerTest ((Get-LatticeSha256Hex -LiteralPath $ancestorJunctionSentinel) -ceq (Get-LatticeStringSha256Hex -Value 'ancestor-junction-target-must-survive')) 'DESKTOP_INSTALLER_ANCESTOR_JUNCTION_TARGET_CHANGED'
    [IO.Directory]::Delete($ancestorJunctionPath)
    $ancestorJunctionPath = ''
    [IO.Directory]::Delete($ancestorJunctionParent)
    $ancestorJunctionParent = ''

    $result = [ordered]@{
        result = 'PASS'
        staging_hash_activation = $true
        default_bundle_root_entrypoint = $true
        realistic_payload_path_staged = $true
        windows_directory_entries_supported = $true
        start_menu_shortcut = $true
        hkcu_uninstall_registration = $true
        reentry = 'REUSED'
        upgrade_retained_previous_version = $true
        rollback = 'ROLLED_BACK'
        tampered_receipt_failed_closed = $true
        corrupt_bundle_failed_closed = $true
        failure_preserved_previous_activation = $true
        fresh_process_activation_recovered = $true
        owner_bootstrap_recovered = $true
        interrupted_uninstall_recovered = $true
        stale_owned_stage_reconciled = $true
        unknown_version_file_preserved = $true
        locked_payload_preserved_install_surfaces = $true
        uninstall = 'UNINSTALLED'
        second_uninstall = 'ALREADY_ABSENT'
        control_data_preserved = $true
        webview_data_preserved = $true
        foreign_sibling_preserved = $true
        foreign_registry_preserved = $true
        unknown_registry_state_preserved = $true
        empty_directory_state_preserved = $true
        alternate_data_stream_preserved = $true
        orphaned_surface_failed_closed = $true
        junction_root_failed_closed = $true
        ancestor_junction_failed_closed = $true
        exact_candidate = $exactCandidate
        install_scope = 'DISPOSABLE_TEST_SANDBOX'
        registry_scope = [string]$context.RegistryPath
    }
}
catch {
    $primaryFailure = $_
}
finally {
    try {
        if (($null -ne $context) -and (Test-Path -LiteralPath ([string]$context.RegistryTestRoot))) {
            Remove-Item -LiteralPath ([string]$context.RegistryTestRoot) -Recurse -Force
        }
        if (($null -ne $foreignContext) -and (Test-Path -LiteralPath ([string]$foreignContext.RegistryTestRoot))) {
            Remove-Item -LiteralPath ([string]$foreignContext.RegistryTestRoot) -Recurse -Force
        }
        if (($null -ne $junctionContext) -and (Test-Path -LiteralPath ([string]$junctionContext.RegistryTestRoot))) {
            Remove-Item -LiteralPath ([string]$junctionContext.RegistryTestRoot) -Recurse -Force
        }
        if (($null -ne $crashContext) -and (Test-Path -LiteralPath ([string]$crashContext.RegistryTestRoot))) {
            Remove-Item -LiteralPath ([string]$crashContext.RegistryTestRoot) -Recurse -Force
        }
        if (($null -ne $orphanContext) -and (Test-Path -LiteralPath ([string]$orphanContext.RegistryTestRoot))) {
            Remove-Item -LiteralPath ([string]$orphanContext.RegistryTestRoot) -Recurse -Force
        }
        foreach ($parentState in @(
            [PSCustomObject]@{ Path = $installerTestsRegistryParent; Existed = $installerTestsRegistryParentExisted },
            [PSCustomObject]@{ Path = $latticeRegistryParent; Existed = $latticeRegistryParentExisted })) {
            if (-not [bool]$parentState.Existed -and (Test-Path -LiteralPath ([string]$parentState.Path))) {
                $key = Get-Item -LiteralPath ([string]$parentState.Path)
                if (@($key.GetSubKeyNames()).Count -eq 0 -and @($key.GetValueNames()).Count -eq 0) {
                    Remove-Item -LiteralPath ([string]$parentState.Path) -Force
                }
            }
        }
    }
    catch {
        $cleanupFailure = $_
    }
    try {
        if (-not [string]::IsNullOrWhiteSpace($junctionPath) -and (Test-Path -LiteralPath $junctionPath)) {
            [IO.Directory]::Delete($junctionPath)
            $junctionPath = ''
        }
        if (-not [string]::IsNullOrWhiteSpace($ancestorJunctionPath) -and (Test-Path -LiteralPath $ancestorJunctionPath)) {
            [IO.Directory]::Delete($ancestorJunctionPath)
            $ancestorJunctionPath = ''
        }
        if (-not [string]::IsNullOrWhiteSpace($ancestorJunctionParent) -and (Test-Path -LiteralPath $ancestorJunctionParent)) {
            [IO.Directory]::Delete($ancestorJunctionParent)
            $ancestorJunctionParent = ''
        }
        foreach ($sandbox in @($foreignSandbox, $junctionSandbox, $crashSandbox, $orphanSandbox)) {
            if (-not [string]::IsNullOrWhiteSpace($sandbox) -and (Test-Path -LiteralPath $sandbox)) {
                $sandboxFull = [IO.Path]::GetFullPath($sandbox)
                $temporaryPrefix = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
                    [IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
                if (-not $sandboxFull.StartsWith($temporaryPrefix, [StringComparison]::OrdinalIgnoreCase) -or
                    -not [IO.Path]::GetFileName($sandboxFull).StartsWith('lattice-desktop-installer-', [StringComparison]::Ordinal)) {
                    throw 'DESKTOP_INSTALLER_TEST_SECONDARY_SANDBOX_INVALID'
                }
                Assert-LatticeTreeHasNoReparsePoints -Root $sandboxFull -ErrorCode 'DESKTOP_INSTALLER_TEST_SECONDARY_REPARSE_POINT'
                Remove-Item -LiteralPath $sandboxFull -Recurse -Force
            }
        }
    }
    catch {
        if ($null -eq $cleanupFailure) {
            $cleanupFailure = $_
        }
    }
    try {
        $temporaryRootFull = [IO.Path]::GetFullPath($temporaryRoot)
        $temporaryPrefix = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
            [IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
        if (-not $temporaryRootFull.StartsWith($temporaryPrefix, [StringComparison]::OrdinalIgnoreCase) -or
            -not [IO.Path]::GetFileName($temporaryRootFull).StartsWith('lattice-desktop-installer-', [StringComparison]::Ordinal)) {
            throw 'DESKTOP_INSTALLER_TEST_TEMPORARY_ROOT_INVALID'
        }
        if (Test-Path -LiteralPath $temporaryRootFull) {
            Assert-LatticeTreeHasNoReparsePoints -Root $temporaryRootFull -ErrorCode 'DESKTOP_INSTALLER_TEST_TEMP_REPARSE_POINT'
            Remove-Item -LiteralPath $temporaryRootFull -Recurse -Force
        }
    }
    catch {
        if ($null -eq $cleanupFailure) {
            $cleanupFailure = $_
        }
    }
}

if ($null -ne $primaryFailure) {
    if ($null -ne $cleanupFailure) {
        Write-Warning "DESKTOP_INSTALLER_TEST_CLEANUP_FAILED_AFTER_PRIMARY:$($cleanupFailure.Exception.Message)"
    }
    throw $primaryFailure
}
if ($null -ne $cleanupFailure) {
    throw $cleanupFailure
}
if ($null -eq $result) {
    throw 'DESKTOP_INSTALLER_TEST_RESULT_MISSING'
}
$result | ConvertTo-Json -Depth 6
