[CmdletBinding()]
param(
    [switch]$LibraryOnly,
    [switch]$SelfTestOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$script:Task051ExpectedTask050Commit = '8e5ba40d38b781afff7028841bd981c8dd2b9721'
$script:Task051ExpectedTask050Tree = 'b4478be2801814ffc630cbf113b0a4ffa3a1b591'
$script:Task051DependencyState = 'TASK050_FULLY_VERIFIED'
$script:Task051PublicStatusSchema = 'lattice.task.status.v1'
$script:Task051ExpectedTools = @(
    'lattice_delivery_run',
    'lattice_delivery_status',
    'lattice_task_submit',
    'lattice_task_status'
)
$script:Task051ExpectedStatusKeys = @(
    'ledger_head_digest',
    'result_digest',
    'schema_version',
    'status',
    'task_ref',
    'task_state'
)

function Get-Task051Sha256 {
    param([Parameter(Mandatory = $true)][string]$Path)

    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-Task051StringSha256 {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value)

    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Value)
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try { $hash = $algorithm.ComputeHash($bytes) }
    finally { $algorithm.Dispose() }
    return ([BitConverter]::ToString($hash)).Replace('-', '').ToLowerInvariant()
}

function Assert-Task051NoReparseAncestor {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Boundary,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $canonicalPath = [IO.Path]::GetFullPath($Path).TrimEnd('\')
    $canonicalBoundary = [IO.Path]::GetFullPath($Boundary).TrimEnd('\')
    $prefix = $canonicalBoundary + '\'
    if (
        -not $canonicalPath.Equals($canonicalBoundary, [StringComparison]::OrdinalIgnoreCase) -and
        -not $canonicalPath.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
    ) {
        throw $FailureCode
    }
    $current = $canonicalPath
    while ($true) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw $FailureCode
            }
        }
        if ($current.Equals($canonicalBoundary, [StringComparison]::OrdinalIgnoreCase)) {
            break
        }
        $parent = Split-Path -Parent $current
        if ([string]::IsNullOrWhiteSpace($parent) -or $parent.Equals($current, [StringComparison]::OrdinalIgnoreCase)) {
            throw $FailureCode
        }
        $current = $parent
    }
}

function Assert-Task051OwnerOnlyAcl {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][bool]$Directory
    )

    try {
        $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
        $actual = if ($Directory) {
            [IO.Directory]::GetAccessControl($Path)
        }
        else {
            [IO.File]::GetAccessControl($Path)
        }
        $rules = @($actual.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]))
        if (
            -not $actual.AreAccessRulesProtected -or
            [string]$actual.GetOwner([Security.Principal.SecurityIdentifier]) -cne [string]$sid -or
            $rules.Count -ne 1 -or
            [string]$rules[0].IdentityReference -cne [string]$sid -or
            $rules[0].AccessControlType -ne [Security.AccessControl.AccessControlType]::Allow -or
            $rules[0].IsInherited -or
            $rules[0].PropagationFlags -ne [Security.AccessControl.PropagationFlags]::None -or
            ($Directory -and $rules[0].InheritanceFlags -ne [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit') -or
            (-not $Directory -and $rules[0].InheritanceFlags -ne [Security.AccessControl.InheritanceFlags]::None) -or
            (($rules[0].FileSystemRights -band [Security.AccessControl.FileSystemRights]::FullControl) -ne [Security.AccessControl.FileSystemRights]::FullControl)
        ) {
            throw 'TASK051_OWNER_ONLY_ACL_REJECTED'
        }
    }
    catch {
        throw 'TASK051_OWNER_ONLY_ACL_REJECTED'
    }
}

function Set-Task051OwnerOnlyAcl {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][bool]$Directory
    )

    try {
        $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
        if ($Directory) {
            $security = [Security.AccessControl.DirectorySecurity]::new()
            $rule = [Security.AccessControl.FileSystemAccessRule]::new(
                $sid,
                [Security.AccessControl.FileSystemRights]::FullControl,
                [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit',
                [Security.AccessControl.PropagationFlags]::None,
                [Security.AccessControl.AccessControlType]::Allow
            )
            $security.SetOwner($sid)
            $security.SetAccessRuleProtection($true, $false)
            [void]$security.AddAccessRule($rule)
            [IO.Directory]::SetAccessControl($Path, $security)
        }
        else {
            $security = [Security.AccessControl.FileSecurity]::new()
            $rule = [Security.AccessControl.FileSystemAccessRule]::new(
                $sid,
                [Security.AccessControl.FileSystemRights]::FullControl,
                [Security.AccessControl.AccessControlType]::Allow
            )
            $security.SetOwner($sid)
            $security.SetAccessRuleProtection($true, $false)
            [void]$security.AddAccessRule($rule)
            [IO.File]::SetAccessControl($Path, $security)
        }
        Assert-Task051OwnerOnlyAcl -Path $Path -Directory $Directory
    }
    catch {
        throw 'TASK051_OWNER_ONLY_ACL_REJECTED'
    }
}

function New-Task051OwnerOnlyDirectory {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (Test-Path -LiteralPath $Path) { throw 'TASK051_OWNER_ONLY_DIRECTORY_NOT_FRESH' }
    try {
        $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
        $security = [Security.AccessControl.DirectorySecurity]::new()
        $rule = [Security.AccessControl.FileSystemAccessRule]::new(
            $sid,
            [Security.AccessControl.FileSystemRights]::FullControl,
            [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit',
            [Security.AccessControl.PropagationFlags]::None,
            [Security.AccessControl.AccessControlType]::Allow
        )
        $security.SetOwner($sid)
        $security.SetAccessRuleProtection($true, $false)
        [void]$security.AddAccessRule($rule)
        [void][IO.Directory]::CreateDirectory($Path, $security)
        Set-Task051OwnerOnlyAcl -Path $Path -Directory $true
    }
    catch {
        throw 'TASK051_OWNER_ONLY_DIRECTORY_REJECTED'
    }
}

function Initialize-Task051CargoHome {
    param([Parameter(Mandatory = $true)][string]$Destination)

    $sourceRegistry = [IO.Path]::GetFullPath((Join-Path $env:USERPROFILE '.cargo\registry'))
    if (
        -not (Test-Path -LiteralPath $sourceRegistry -PathType Container) -or
        (Get-Item -LiteralPath $sourceRegistry -Force).Attributes -band [IO.FileAttributes]::ReparsePoint -or
        @(Get-ChildItem -LiteralPath $sourceRegistry -Recurse -Force -Attributes ReparsePoint -ErrorAction Stop).Count -ne 0
    ) {
        throw 'TASK051_CARGO_CACHE_SOURCE_REJECTED'
    }
    foreach ($required in @('cache', 'index', 'src')) {
        if (-not (Test-Path -LiteralPath (Join-Path $sourceRegistry $required) -PathType Container)) {
            throw 'TASK051_CARGO_CACHE_SOURCE_REJECTED'
        }
    }
    if (Test-Path -LiteralPath $Destination) {
        throw 'TASK051_CARGO_HOME_NOT_FRESH'
    }

    New-Task051OwnerOnlyDirectory -Path $Destination
    $destinationRegistry = Join-Path $Destination 'registry'
    New-Task051OwnerOnlyDirectory -Path $destinationRegistry
    $copyOutput = @(& robocopy.exe $sourceRegistry $destinationRegistry /E /COPY:DAT /DCOPY:DAT /XJ /R:1 /W:1 /NFL /NDL /NJH /NJS /NP 2>&1 | ForEach-Object { [string]$_ })
    $copyExitCode = $LASTEXITCODE
    if ($copyExitCode -lt 0 -or $copyExitCode -gt 7) {
        throw ('TASK051_CARGO_CACHE_COPY_REJECTED|' + (Get-Task051StringSha256 -Value ($copyOutput -join [char]10)))
    }
    Assert-Task051OwnerOnlyAcl -Path $Destination -Directory $true
    Assert-Task051OwnerOnlyAcl -Path $destinationRegistry -Directory $true
    foreach ($required in @('cache', 'index', 'src')) {
        if (-not (Test-Path -LiteralPath (Join-Path $destinationRegistry $required) -PathType Container)) {
            throw 'TASK051_CARGO_CACHE_COPY_REJECTED'
        }
    }
}

function Remove-Task051OwnedDirectory {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$AllowedRoot,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    try {
        $fullPath = [IO.Path]::GetFullPath($Path)
        $fullAllowedRoot = [IO.Path]::GetFullPath($AllowedRoot)
        $allowedPrefix = $fullAllowedRoot.TrimEnd('\') + '\'
        if (-not $fullPath.StartsWith($allowedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw $FailureCode
        }
        if (Test-Path -LiteralPath $fullPath -PathType Container) {
            [IO.Directory]::Delete(('\\?\' + $fullPath), $true)
        }
        if (Test-Path -LiteralPath $fullPath) {
            throw $FailureCode
        }
    }
    catch {
        throw $FailureCode
    }
}

function New-Task051RunRootAlias {
    param([Parameter(Mandatory = $true)][string]$RunRoot)

    $fullRunRoot = [IO.Path]::GetFullPath($RunRoot).TrimEnd('\')
    $mappings = @(& subst.exe)
    if ($LASTEXITCODE -ne 0) { throw 'TASK051_RUN_ALIAS_QUERY_REJECTED' }
    $occupiedDrives = @([IO.DriveInfo]::GetDrives() | ForEach-Object { $_.Name.ToUpperInvariant() })
    foreach ($codePoint in 90..68) {
        $drive = ([char]$codePoint).ToString() + ':'
        $root = $drive + '\'
        $mappingPrefix = $drive + '\: => '
        if (
            $occupiedDrives -contains $root.ToUpperInvariant() -or
            @($mappings | Where-Object { ([string]$_).StartsWith($mappingPrefix, [StringComparison]::OrdinalIgnoreCase) }).Count -ne 0
        ) {
            continue
        }
        & subst.exe $drive $fullRunRoot
        if ($LASTEXITCODE -ne 0) { throw 'TASK051_RUN_ALIAS_CREATE_REJECTED' }
        $created = @(& subst.exe | Where-Object { ([string]$_).StartsWith($mappingPrefix, [StringComparison]::OrdinalIgnoreCase) })
        if (
            $LASTEXITCODE -ne 0 -or
            $created.Count -ne 1 -or
            -not ([string]$created[0]).Substring($mappingPrefix.Length).Equals($fullRunRoot, [StringComparison]::OrdinalIgnoreCase) -or
            -not (Test-Path -LiteralPath $root -PathType Container)
        ) {
            & subst.exe $drive /D 2>$null
            $removeExitCode = $LASTEXITCODE
            $remaining = @(& subst.exe | Where-Object { ([string]$_).StartsWith($mappingPrefix, [StringComparison]::OrdinalIgnoreCase) })
            if ($removeExitCode -ne 0 -or $LASTEXITCODE -ne 0 -or $remaining.Count -ne 0) {
                throw 'TASK051_RUN_ALIAS_CLEANUP_REJECTED'
            }
            throw 'TASK051_RUN_ALIAS_CREATE_REJECTED'
        }
        return [pscustomobject]@{ Drive = $drive; Root = $root; RunRoot = $fullRunRoot }
    }
    throw 'TASK051_RUN_ALIAS_UNAVAILABLE'
}

function Remove-Task051RunRootAlias {
    param([Parameter(Mandatory = $true)][object]$Alias)

    $mappingPrefix = [string]$Alias.Drive + '\: => '
    $current = @(& subst.exe | Where-Object { ([string]$_).StartsWith($mappingPrefix, [StringComparison]::OrdinalIgnoreCase) })
    if (
        $LASTEXITCODE -ne 0 -or
        $current.Count -ne 1 -or
        -not ([string]$current[0]).Substring($mappingPrefix.Length).Equals([string]$Alias.RunRoot, [StringComparison]::OrdinalIgnoreCase)
    ) {
        throw 'TASK051_RUN_ALIAS_CLEANUP_REJECTED'
    }
    & subst.exe ([string]$Alias.Drive) /D
    if ($LASTEXITCODE -ne 0) { throw 'TASK051_RUN_ALIAS_CLEANUP_REJECTED' }
    $remaining = @(& subst.exe | Where-Object { ([string]$_).StartsWith($mappingPrefix, [StringComparison]::OrdinalIgnoreCase) })
    if ($LASTEXITCODE -ne 0 -or $remaining.Count -ne 0 -or (Test-Path -LiteralPath ([string]$Alias.Root))) {
        throw 'TASK051_RUN_ALIAS_CLEANUP_REJECTED'
    }
}

function Get-Task051PostgresProcessSnapshot {
    try {
        $identities = @(
            Get-CimInstance -ClassName Win32_Process -Filter "Name = 'postgres.exe'" -ErrorAction Stop |
                ForEach-Object {
                    if ([long]$_.ProcessId -lt 1 -or [string]::IsNullOrWhiteSpace([string]$_.CreationDate)) {
                        throw 'TASK051_POSTGRES_PROCESS_SNAPSHOT_REJECTED'
                    }
                    $createdAt = [DateTimeOffset]([DateTime]$_.CreationDate)
                    ([string][long]$_.ProcessId) + '|' + $createdAt.ToFileTime().ToString()
                } |
                Sort-Object -Unique
        )
        return $identities
    }
    catch {
        throw 'TASK051_POSTGRES_PROCESS_SNAPSHOT_REJECTED'
    }
}

function Test-Task051PostgresProcessSnapshotClosed {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Baseline,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Current
    )

    foreach ($identity in @($Baseline) + @($Current)) {
        if ([string]$identity -cnotmatch '\A[1-9][0-9]{0,9}\|[1-9][0-9]{0,19}\z') {
            return $false
        }
    }
    if (
        @($Baseline | Sort-Object -Unique).Count -ne @($Baseline).Count -or
        @($Current | Sort-Object -Unique).Count -ne @($Current).Count
    ) {
        return $false
    }
    return (@($Current | Where-Object { $_ -cnotin $Baseline }).Count -eq 0)
}

function Test-Task051RunRootAliasReleaseSafe {
    param(
        [Parameter(Mandatory = $true)][object]$Alias,
        [Parameter(Mandatory = $true)][string]$RunRoot,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$BaselinePostgresProcesses
    )

    try {
        $fullRunRoot = [IO.Path]::GetFullPath($RunRoot).TrimEnd('\')
        $aliasRoot = [IO.Path]::GetFullPath([string]$Alias.Root)
        if (
            -not ([string]$Alias.RunRoot).Equals($fullRunRoot, [StringComparison]::OrdinalIgnoreCase) -or
            -not (Test-Path -LiteralPath ([string]$Alias.Root) -PathType Container)
        ) {
            return $false
        }

        $processes = @(Get-CimInstance -ClassName Win32_Process -ErrorAction Stop)
        $currentPostgresIdentities = @(
            $processes |
                Where-Object { [string]$_.Name -ieq 'postgres.exe' } |
                ForEach-Object {
                    if ([long]$_.ProcessId -lt 1 -or [string]::IsNullOrWhiteSpace([string]$_.CreationDate)) {
                        throw 'TASK051_RUN_ALIAS_RELEASE_PROCESS_REJECTED'
                    }
                    $createdAt = [DateTimeOffset]([DateTime]$_.CreationDate)
                    ([string][long]$_.ProcessId) + '|' + $createdAt.ToFileTime().ToString()
                } |
                Sort-Object -Unique
        )
        if (-not (Test-Task051PostgresProcessSnapshotClosed -Baseline $BaselinePostgresProcesses -Current $currentPostgresIdentities)) {
            return $false
        }
        foreach ($process in $processes) {
            $commandLine = [string]$process.CommandLine
            $executablePath = [string]$process.ExecutablePath
            if (
                (-not [string]::IsNullOrWhiteSpace($commandLine) -and (
                    $commandLine.IndexOf($aliasRoot, [StringComparison]::OrdinalIgnoreCase) -ge 0 -or
                    $commandLine.IndexOf($fullRunRoot, [StringComparison]::OrdinalIgnoreCase) -ge 0
                )) -or
                (-not [string]::IsNullOrWhiteSpace($executablePath) -and (
                    $executablePath.IndexOf($aliasRoot, [StringComparison]::OrdinalIgnoreCase) -ge 0 -or
                    $executablePath.IndexOf($fullRunRoot, [StringComparison]::OrdinalIgnoreCase) -ge 0
                ))
            ) {
                return $false
            }
        }

        $clusterParent = Join-Path $fullRunRoot 'task019-postgres'
        if (
            -not (Test-Path -LiteralPath $clusterParent -PathType Container) -or
            ((Get-Item -LiteralPath $clusterParent -Force -ErrorAction Stop).Attributes -band [IO.FileAttributes]::ReparsePoint) -or
            @(Get-ChildItem -LiteralPath $clusterParent -Force -ErrorAction Stop).Count -ne 0
        ) {
            return $false
        }

        $receiptRoot = Join-Path $fullRunRoot 'task019-holder-receipts'
        if (
            -not (Test-Path -LiteralPath $receiptRoot -PathType Container) -or
            ((Get-Item -LiteralPath $receiptRoot -Force -ErrorAction Stop).Attributes -band [IO.FileAttributes]::ReparsePoint)
        ) {
            return $false
        }
        Assert-Task051OwnerOnlyAcl -Path $receiptRoot -Directory $true
        $receiptEntries = @(Get-ChildItem -LiteralPath $receiptRoot -Force -ErrorAction Stop)
        if ($receiptEntries.Count -ne 1 -or [bool]$receiptEntries[0].PSIsContainer) {
            return $false
        }
        $receipt = $receiptEntries[0]
        Assert-Task051RegularFile -Path $receipt.FullName -FailureCode 'TASK051_RUN_ALIAS_RELEASE_RECEIPT_REJECTED'
        Assert-Task051OwnerOnlyAcl -Path $receipt.FullName -Directory $false
        $lines = @(Get-Content -LiteralPath $receipt.FullName -Encoding utf8)
        if ($lines.Count -lt 5) { return $false }
        $events = @($lines | ForEach-Object { $_ | ConvertFrom-Json -ErrorAction Stop })
        $first = $events[0]
        $runId = [string]$first.run_id
        $sessionId = [string]$first.session_id
        $port = [int]$first.port
        if (
            $runId -cnotmatch '\A[0-9a-f]{32}\z' -or
            $sessionId -cnotmatch '\A[0-9a-f]{32}\z' -or
            $receipt.Name -cne ($runId + '.jsonl') -or
            $port -lt 1 -or
            [string]$first.event_type -cne 'HOLDER_OPEN'
        ) {
            return $false
        }
        $previousHmac = '0' * 64
        $consumerSessionId = [string]$first.consumer_session_id
        $nonceCommitment = [string]$first.nonce_commitment
        $allowedEventTypes = @(
            'HOLDER_OPEN', 'MARKER_CREATED', 'INITIAL_POSTMASTER_READY',
            'INITIAL_POSTMASTER_STOPPED', 'RESTART_POSTMASTER_READY',
            'CONSUMER_STARTED', 'CONSUMER_EXITED', 'HOLDER_STOP_REQUESTED',
            'HOLDER_STOPPED', 'CATALOG_SIGNATURES_MEASURED', 'CATALOG_SIGNATURES_PARTIAL',
            'CATALOG_DIAGNOSTIC_FAILED', 'LIVE_GATE_FAILED', 'TASK076_WRITER_V2_VERIFIED',
            'CLEANUP_REQUESTED', 'CLEANUP_COMPLETED', 'RECEIPT_CLOSED'
        )
        if (
            $consumerSessionId -cnotmatch '\A[0-9a-f]{32}\z' -or
            $nonceCommitment -cnotmatch '\A[0-9a-f]{64}\z'
        ) {
            return $false
        }
        for ($index = 0; $index -lt $events.Count; $index++) {
            $event = $events[$index]
            $payloadSha256 = Get-Task051StringSha256 -Value ($event.payload | ConvertTo-Json -Compress -Depth 20)
            if (
                [string]$event.schema -cne 'lattice.task019.postgres-holder-authority.v1' -or
                [string]$event.event_type -cnotin $allowedEventTypes -or
                [string]$event.run_id -cne $runId -or
                [string]$event.session_id -cne $sessionId -or
                [string]$event.consumer_session_id -cne $consumerSessionId -or
                [string]$event.host -cne '127.0.0.1' -or
                [int]$event.port -ne $port -or
                (@($event.excluded_ports) -join ',') -cne '5432,64272,55432' -or
                [string]$event.nonce_commitment -cne $nonceCommitment -or
                [long]$event.ordinal -ne ($index + 1) -or
                [string]$event.previous_hmac_sha256 -cne $previousHmac -or
                [string]$event.payload_sha256 -cne $payloadSha256 -or
                [string]$event.event_hmac_sha256 -cnotmatch '\A[0-9a-f]{64}\z'
            ) {
                return $false
            }
            $previousHmac = [string]$event.event_hmac_sha256
        }
        $expectedClusterRoot = [IO.Path]::GetFullPath((Join-Path $clusterParent $runId))
        $expectedAliasData = [IO.Path]::GetFullPath((Join-Path ([string]$Alias.Root) ('task019-postgres\' + $runId + '\data')))
        if (
            -not ([IO.Path]::GetFullPath([string]$first.payload.cluster_root).Equals($expectedClusterRoot, [StringComparison]::OrdinalIgnoreCase)) -or
            -not ([IO.Path]::GetFullPath([string]$first.payload.data_directory).Equals($expectedAliasData, [StringComparison]::OrdinalIgnoreCase)) -or
            [string]$first.payload.authority_receipt_path -cne $receipt.FullName
        ) {
            return $false
        }
        $tail = @($events | Select-Object -Last 4)
        if (
            ($tail.event_type -join ',') -cne 'HOLDER_STOPPED,CLEANUP_REQUESTED,CLEANUP_COMPLETED,RECEIPT_CLOSED' -or
            $tail[0].payload.pg_ctl_status_stopped -isnot [bool] -or $tail[0].payload.pg_ctl_status_stopped -cne $true -or
            $tail[0].payload.listener_absent -isnot [bool] -or $tail[0].payload.listener_absent -cne $true -or
            $tail[2].payload.cluster_root_absent -isnot [bool] -or $tail[2].payload.cluster_root_absent -cne $true -or
            $tail[2].payload.listener_absent -isnot [bool] -or $tail[2].payload.listener_absent -cne $true -or
            $tail[3].payload.cleanup_complete -isnot [bool] -or $tail[3].payload.cleanup_complete -cne $true -or
            [long]$tail[3].payload.final_event_count_before_close -ne ($events.Count - 1) -or
            -not ([IO.Path]::GetFullPath([string]$tail[2].payload.cluster_root).Equals($expectedClusterRoot, [StringComparison]::OrdinalIgnoreCase))
        ) {
            return $false
        }
        $portListeners = @(
            Get-NetTCPConnection -State Listen -ErrorAction Stop |
                Where-Object { [int]$_.LocalPort -eq $port }
        )
        if ($portListeners.Count -ne 0) {
            return $false
        }

        $postgresPath = [IO.Path]::GetFullPath([string]$first.payload.tool_identity.postgres_path)
        Assert-Task051RegularFile -Path $postgresPath -FailureCode 'TASK051_RUN_ALIAS_RELEASE_POSTGRES_REJECTED'
        if ((Get-Task051Sha256 -Path $postgresPath) -cne [string]$first.payload.tool_identity.postgres_sha256) {
            return $false
        }
        foreach ($event in $events) {
            $pidProperty = $event.payload.PSObject.Properties['listener_process_id']
            if ($null -eq $pidProperty -or [long]$pidProperty.Value -lt 1) { continue }
            $known = @($processes | Where-Object { [long]$_.ProcessId -eq [long]$pidProperty.Value })
            if ($known.Count -gt 1) { return $false }
            if ($known.Count -eq 1) {
                $creationProperty = $event.payload.PSObject.Properties['listener_process_creation_time']
                if ($null -eq $creationProperty -or [string]::IsNullOrWhiteSpace([string]$known[0].CreationDate)) {
                    return $false
                }
                $actualCreation = ([DateTimeOffset]([DateTime]$known[0].CreationDate)).ToFileTime().ToString()
                if ($actualCreation -ceq [string]$creationProperty.Value) {
                    return $false
                }
            }
        }
        foreach ($process in @($processes | Where-Object { [string]$_.Name -ieq 'postgres.exe' })) {
            $commandLine = [string]$process.CommandLine
            $executablePath = [string]$process.ExecutablePath
            if ([string]::IsNullOrWhiteSpace($commandLine) -or [string]::IsNullOrWhiteSpace($executablePath)) {
                continue
            }
            if (
                [IO.Path]::GetFullPath($executablePath).Equals($postgresPath, [StringComparison]::OrdinalIgnoreCase) -and
                (
                    $commandLine.IndexOf($expectedAliasData, [StringComparison]::OrdinalIgnoreCase) -ge 0 -or
                    $commandLine.IndexOf($expectedClusterRoot, [StringComparison]::OrdinalIgnoreCase) -ge 0
                )
            ) {
                return $false
            }
        }
        return $true
    }
    catch {
        return $false
    }
}

function Write-Task051JsonEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )

    if (Test-Path -LiteralPath $Path) { throw 'TASK051_EVIDENCE_NOT_FRESH' }
    $json = ($Value | ConvertTo-Json -Compress -Depth 50) + [char]10
    Assert-SecretFreeText -Text $json -FailureCode 'TASK051_EVIDENCE_SECRET_REJECTED'
    [IO.File]::WriteAllText($Path, $json, [Text.UTF8Encoding]::new($false))
    Set-Task051OwnerOnlyAcl -Path $Path -Directory $false
    return [pscustomobject]@{
        Path = [IO.Path]::GetFullPath($Path)
        Sha256 = Get-Task051Sha256 -Path $Path
    }
}

function ConvertTo-Task051TomlLiteral {
    param([Parameter(Mandatory = $true)][string]$Value)

    if ($Value.Contains("'")) {
        throw 'TASK051_TOML_LITERAL_REJECTED'
    }
    return "'" + $Value + "'"
}

function Assert-Task051RegularFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        $item.PSIsContainer -or
        -not ($item -is [IO.FileInfo]) -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw $FailureCode
    }
}

function Get-Task051OfficialCodexBundlePolicy {
    return @(
        [pscustomobject]@{
            RelativePath = 'codex-official\0.146.0\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex.exe'
            Sha256 = 'bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb'
        },
        [pscustomobject]@{
            RelativePath = 'codex-official\0.146.0\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\codex-resources\codex-windows-sandbox-setup.exe'
            Sha256 = 'c12d225b34e7f82cdab6bbc714797abed661f40e158104694953889750121cef'
        },
        [pscustomobject]@{
            RelativePath = 'codex-official\0.146.0\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\codex-resources\codex-command-runner.exe'
            Sha256 = '0102fa1820ecd03bb03a991fd2303a1a484118f7da8a71864f88ec94bca61d6d'
        },
        [pscustomobject]@{
            RelativePath = 'codex-official\0.146.0\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex-code-mode-host.exe'
            Sha256 = '6ef1de0e04d859f8f4f6d4d64f0f3ceeec28658423d91de160f5e804280d1c36'
        },
        [pscustomobject]@{
            RelativePath = 'codex-official\0.146.0\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\codex-path\rg.exe'
            Sha256 = '14231169855ec5205cf5a1b6f1db358ff4aed4247c86b69ce8aae647c77f6680'
        },
        [pscustomobject]@{
            RelativePath = 'codex-official\0.146.0\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\codex-package.json'
            Sha256 = 'aaa0646d6b615da94187b51efd50c69621a00867761161ae55cc16cfd545bec7'
        },
        [pscustomobject]@{
            RelativePath = 'codex-official\0.146.0\node_modules\@openai\codex\package.json'
            Sha256 = '24dd8c63a4d2b7bc2ded86c887974f842093ce4f2ed8473267a91e036c38da20'
        }
    )
}

function Assert-Task051OfficialCodexBundle {
    param(
        [Parameter(Mandatory = $true)][string]$BundleTargetRoot,
        [Parameter(Mandatory = $true)][string]$Boundary,
        [switch]$ValidateVersion
    )

    $bundleTarget = [IO.Path]::GetFullPath($BundleTargetRoot)
    $canonicalBoundary = [IO.Path]::GetFullPath($Boundary)
    if (-not (Test-Path -LiteralPath $bundleTarget -PathType Container)) {
        throw 'TASK051_OFFICIAL_CODEX_BUNDLE_REJECTED'
    }
    Assert-Task051NoReparseAncestor -Path $bundleTarget -Boundary $canonicalBoundary -FailureCode 'TASK051_OFFICIAL_CODEX_BUNDLE_REJECTED'

    $policy = @(Get-Task051OfficialCodexBundlePolicy)
    foreach ($entry in $policy) {
        $path = [IO.Path]::GetFullPath((Join-Path $bundleTarget ([string]$entry.RelativePath)))
        Assert-Task051NoReparseAncestor -Path $path -Boundary $bundleTarget -FailureCode 'TASK051_OFFICIAL_CODEX_BUNDLE_REJECTED'
        Assert-Task051RegularFile -Path $path -FailureCode 'TASK051_OFFICIAL_CODEX_BUNDLE_REJECTED'
        if ((Get-Task051Sha256 -Path $path) -cne [string]$entry.Sha256) {
            throw 'TASK051_OFFICIAL_CODEX_BUNDLE_REJECTED'
        }
    }

    $launcher = [IO.Path]::GetFullPath((Join-Path $bundleTarget ([string]$policy[0].RelativePath)))
    if ($ValidateVersion) {
        $versionOutput = @(& $launcher --version 2>&1 | ForEach-Object { [string]$_ })
        if ($LASTEXITCODE -ne 0 -or $versionOutput.Count -ne 1 -or $versionOutput[0] -cne 'codex-cli 0.146.0') {
            throw 'TASK051_OFFICIAL_CODEX_BUNDLE_REJECTED'
        }
    }
    return $launcher
}

function Copy-Task051OfficialCodexBundle {
    param(
        [Parameter(Mandatory = $true)][string]$SourceTargetRoot,
        [Parameter(Mandatory = $true)][string]$SourceBoundary,
        [Parameter(Mandatory = $true)][string]$DestinationTargetRoot,
        [Parameter(Mandatory = $true)][string]$DestinationBoundary
    )

    try {
        [void](Assert-Task051OfficialCodexBundle -BundleTargetRoot $SourceTargetRoot -Boundary $SourceBoundary)
        $sourceTarget = [IO.Path]::GetFullPath($SourceTargetRoot)
        $destinationTarget = [IO.Path]::GetFullPath($DestinationTargetRoot)
        if (Test-Path -LiteralPath $destinationTarget) {
            throw 'TASK051_OFFICIAL_CODEX_BUNDLE_COPY_REJECTED'
        }
        New-Task051OwnerOnlyDirectory -Path $destinationTarget
        Assert-Task051NoReparseAncestor -Path $destinationTarget -Boundary $DestinationBoundary -FailureCode 'TASK051_OFFICIAL_CODEX_BUNDLE_COPY_REJECTED'
        $destinationBundle = Join-Path $destinationTarget 'codex-official'
        [IO.Directory]::CreateDirectory($destinationBundle) | Out-Null
        Set-Task051OwnerOnlyAcl -Path $destinationBundle -Directory $true
        Assert-Task051NoReparseAncestor -Path $destinationBundle -Boundary $DestinationBoundary -FailureCode 'TASK051_OFFICIAL_CODEX_BUNDLE_COPY_REJECTED'

        foreach ($entry in @(Get-Task051OfficialCodexBundlePolicy)) {
            $source = [IO.Path]::GetFullPath((Join-Path $sourceTarget ([string]$entry.RelativePath)))
            $destination = [IO.Path]::GetFullPath((Join-Path $destinationTarget ([string]$entry.RelativePath)))
            $destinationParent = Split-Path -Parent $destination
            [IO.Directory]::CreateDirectory($destinationParent) | Out-Null
            Set-Task051OwnerOnlyAcl -Path $destinationParent -Directory $true
            Assert-Task051NoReparseAncestor -Path $destinationParent -Boundary $destinationTarget -FailureCode 'TASK051_OFFICIAL_CODEX_BUNDLE_COPY_REJECTED'
            [IO.File]::Copy($source, $destination, $false)
            Set-Task051OwnerOnlyAcl -Path $destination -Directory $false
            if ((Get-Task051Sha256 -Path $destination) -cne [string]$entry.Sha256) {
                throw 'TASK051_OFFICIAL_CODEX_BUNDLE_COPY_REJECTED'
            }
        }

        [void](Assert-Task051OfficialCodexBundle -BundleTargetRoot $SourceTargetRoot -Boundary $SourceBoundary)
        return Assert-Task051OfficialCodexBundle -BundleTargetRoot $destinationTarget -Boundary $DestinationBoundary -ValidateVersion
    }
    catch {
        throw 'TASK051_OFFICIAL_CODEX_BUNDLE_COPY_REJECTED'
    }
}

function Assert-Task051PublicStatus {
    param(
        [Parameter(Mandatory = $true)]$Value,
        [Parameter(Mandatory = $true)][ValidateSet('SUBMIT', 'STATUS')][string]$Kind
    )

    $keys = @($Value.PSObject.Properties.Name | Sort-Object)
    if (($keys -join [char]10) -cne (($script:Task051ExpectedStatusKeys | Sort-Object) -join [char]10)) {
        throw 'TASK051_PUBLIC_STATUS_SHAPE_REJECTED'
    }
    if (
        [string]$Value.schema_version -cne $script:Task051PublicStatusSchema -or
        [string]$Value.status -cne 'COMPLETED' -or
        [string]$Value.task_state -cne 'COMPLETED' -or
        [string]$Value.task_ref -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$Value.ledger_head_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$Value.result_digest -cnotmatch '\A[0-9a-f]{64}\z'
    ) {
        throw ('TASK051_' + $Kind + '_SEMANTICS_REJECTED')
    }
}

function Assert-Task051SameStatus {
    param(
        [Parameter(Mandatory = $true)]$Expected,
        [Parameter(Mandatory = $true)]$Actual
    )

    Assert-Task051PublicStatus -Value $Expected -Kind 'STATUS'
    Assert-Task051PublicStatus -Value $Actual -Kind 'STATUS'
    foreach ($name in $script:Task051ExpectedStatusKeys) {
        if ([string]$Expected.$name -cne [string]$Actual.$name) {
            throw 'TASK051_PUBLIC_STATUS_REPLAY_REJECTED'
        }
    }
}

function Assert-Task051DistinctProcessIds {
    param([Parameter(Mandatory = $true)][int[]]$ProcessIds)

    if (
        $ProcessIds.Count -ne 4 -or
        @($ProcessIds | Where-Object { $_ -le 0 }).Count -ne 0 -or
        @($ProcessIds | Sort-Object -Unique).Count -ne 4
    ) {
        throw 'TASK051_CODEX_FRESH_PROCESS_REJECTED'
    }
}

function Read-Task051JsonLines {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $items = [Collections.Generic.List[object]]::new()
    foreach ($line in @($Text -split '\r?\n')) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        try { $items.Add(($line | ConvertFrom-Json -ErrorAction Stop)) }
        catch { throw $FailureCode }
    }
    if ($items.Count -eq 0) { throw $FailureCode }
    return @($items)
}

function Get-Task051AppServerResponse {
    param(
        [Parameter(Mandatory = $true)][IO.TextReader]$Reader,
        [Parameter(Mandatory = $true)][int]$Id,
        [Parameter(Mandatory = $true)][int]$TimeoutSeconds,
        [scriptblock]$PollAction,
        [ref]$PollResult,
        [string]$PollTimeoutFailureCode = 'TASK051_APP_SERVER_RESPONSE_TIMEOUT'
    )

    $watch = [Diagnostics.Stopwatch]::StartNew()
    $readTask = $null
    $response = $null
    $pollComplete = $null -eq $PollAction
    while ($watch.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
        if (-not $pollComplete) {
            $pollValues = @(& $PollAction)
            if ($pollValues.Count -gt 1) { throw 'TASK051_APP_SERVER_RESPONSE_REJECTED' }
            if ($pollValues.Count -eq 1 -and $null -ne $pollValues[0]) {
                $pollComplete = $true
                if ($null -ne $PollResult) { $PollResult.Value = $pollValues[0] }
            }
        }
        if ($null -ne $response -and $pollComplete) { return $response }
        if ($null -eq $response) {
            if ($null -eq $readTask) { $readTask = $Reader.ReadLineAsync() }
            $remainingMilliseconds = [Math]::Max(1, [int](($TimeoutSeconds - $watch.Elapsed.TotalSeconds) * 1000))
            $sliceMilliseconds = if ($null -ne $PollAction) { [Math]::Min(20, $remainingMilliseconds) } else { $remainingMilliseconds }
            if ($readTask.Wait([TimeSpan]::FromMilliseconds($sliceMilliseconds))) {
                $line = [string]$readTask.GetAwaiter().GetResult()
                $readTask = $null
                if ($null -eq $line) { break }
                try { $item = $line | ConvertFrom-Json -ErrorAction Stop }
                catch { throw 'TASK051_APP_SERVER_JSON_REJECTED' }
                if ($item.PSObject.Properties.Name -contains 'id' -and [int]$item.id -eq $Id) {
                    if ($item.PSObject.Properties.Name -contains 'error') {
                        throw 'TASK051_APP_SERVER_RESPONSE_REJECTED'
                    }
                    $response = $item
                }
            }
        }
        else {
            Start-Sleep -Milliseconds 20
        }
    }
    if ($null -ne $response -and -not $pollComplete) { throw $PollTimeoutFailureCode }
    throw 'TASK051_APP_SERVER_RESPONSE_TIMEOUT'
}

function Set-Task051ClosedEnvironment {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.ProcessStartInfo]$StartInfo,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Additional
    )

    $baseNames = @(
        'ALLUSERSPROFILE', 'APPDATA', 'CommonProgramFiles', 'CommonProgramFiles(x86)',
        'CommonProgramW6432', 'ComSpec', 'LOCALAPPDATA', 'NUMBER_OF_PROCESSORS',
        'OS', 'Path', 'PATHEXT', 'PROCESSOR_ARCHITECTURE', 'PROCESSOR_IDENTIFIER',
        'PROCESSOR_LEVEL', 'PROCESSOR_REVISION', 'ProgramData', 'ProgramFiles',
        'ProgramFiles(x86)', 'ProgramW6432', 'PSModulePath', 'SystemDrive',
        'SystemRoot', 'TEMP', 'TMP', 'USERDOMAIN', 'USERNAME', 'USERPROFILE', 'windir'
    )
    $captured = [ordered]@{}
    foreach ($name in $baseNames) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if (-not [string]::IsNullOrWhiteSpace($value)) { $captured[$name] = $value }
    }
    $StartInfo.EnvironmentVariables.Clear()
    foreach ($entry in $captured.GetEnumerator()) {
        $StartInfo.EnvironmentVariables[[string]$entry.Key] = [string]$entry.Value
    }
    foreach ($entry in $Additional.GetEnumerator()) {
        $StartInfo.EnvironmentVariables[[string]$entry.Key] = [string]$entry.Value
    }
}

function Start-Task051OwnedProcess {
    param([Parameter(Mandatory = $true)][Diagnostics.ProcessStartInfo]$StartInfo)

    foreach ($name in @(
        'New-Task038KillOnCloseJob', 'Start-Task038SuspendedProcess',
        'Add-Task038ProcessToJob', 'Resume-Task038SuspendedProcess',
        'Stop-Task038Job', 'Close-Task038Job', 'Stop-Task038ProcessTree'
    )) {
        if ($null -eq (Get-Command $name -CommandType Function -ErrorAction SilentlyContinue)) {
            throw 'TASK051_PROCESS_CONTAINMENT_UNAVAILABLE'
        }
    }
    $job = [IntPtr]::Zero
    $suspended = $null
    try {
        $job = New-Task038KillOnCloseJob
        $suspended = Start-Task038SuspendedProcess -StartInfo $StartInfo
        Add-Task038ProcessToJob -Job $job -Process $suspended.Process
        Resume-Task038SuspendedProcess -SuspendedProcess $suspended
        return [pscustomobject]@{
            Job = $job
            Suspended = $suspended
            Process = $suspended.Process
        }
    }
    catch {
        $primaryFailure = $_
        $cleanupFailed = $false
        if ($job -ne [IntPtr]::Zero) {
            try { Stop-Task038Job -Job $job } catch { $cleanupFailed = $true }
            try { Close-Task038Job -Job $job } catch { $cleanupFailed = $true }
        }
        if ($null -ne $suspended) {
            try { Stop-Task038ProcessTree -Process $suspended.Process } catch { $cleanupFailed = $true }
            try { $suspended.Dispose() } catch { $cleanupFailed = $true }
        }
        if ($cleanupFailed) { throw 'TASK051_PROCESS_START_CLEANUP_REJECTED' }
        throw $primaryFailure
    }
}

function Stop-Task051OwnedProcess {
    param([Parameter(Mandatory = $true)]$Owned)

    $cleanupFailure = $null
    try { Stop-Task038Job -Job ([IntPtr]$Owned.Job) }
    catch { $cleanupFailure = 'TASK051_PROCESS_JOB_TERMINATION_REJECTED' }
    try { Close-Task038Job -Job ([IntPtr]$Owned.Job) }
    catch {
        if ($null -eq $cleanupFailure) {
            $cleanupFailure = 'TASK051_PROCESS_JOB_CLEANUP_REJECTED'
        }
    }
    try { $Owned.Suspended.Dispose() }
    catch {
        if ($null -eq $cleanupFailure) {
            $cleanupFailure = 'TASK051_PROCESS_HANDLE_CLEANUP_REJECTED'
        }
    }
    if ($null -ne $cleanupFailure) { throw $cleanupFailure }
}

function Initialize-Task051ProcessIdentityInterop {
    if ($null -ne ('LatticeTask051ProcessIdentityInterop' -as [type])) { return }
    try {
        Add-Type -ErrorAction Stop -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

public sealed class LatticeTask051OwnedProcessAuthority : IDisposable
{
    private readonly object sync = new object();
    private IntPtr processHandle;

    internal LatticeTask051OwnedProcessAuthority(IntPtr processHandle)
    {
        this.processHandle = processHandle;
    }

    internal IntPtr ProcessHandle { get { return processHandle; } }
    public string ImagePath { get; internal set; }
    public long CreationFileTimeUtc { get; internal set; }

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool GetExitCodeProcess(IntPtr process, out UInt32 exitCode);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr handle);

    public bool IsAlive()
    {
        lock (sync)
        {
            if (processHandle == IntPtr.Zero)
            {
                throw new InvalidOperationException("TASK051_PROCESS_INTEROP_LIVENESS_QUERY");
            }
            UInt32 exitCode;
            if (!GetExitCodeProcess(processHandle, out exitCode))
            {
                throw new InvalidOperationException("TASK051_PROCESS_INTEROP_LIVENESS_QUERY");
            }
            return exitCode == 259;
        }
    }

    public void CloseExact()
    {
        lock (sync)
        {
            if (processHandle == IntPtr.Zero)
            {
                return;
            }
            if (!CloseHandle(processHandle))
            {
                throw new InvalidOperationException("TASK051_PROCESS_INTEROP_CLOSE");
            }
            processHandle = IntPtr.Zero;
            GC.SuppressFinalize(this);
        }
    }

    public void Dispose()
    {
        CloseExact();
    }

    ~LatticeTask051OwnedProcessAuthority()
    {
        lock (sync)
        {
            if (processHandle != IntPtr.Zero)
            {
                CloseHandle(processHandle);
                processHandle = IntPtr.Zero;
            }
        }
    }
}

public static class LatticeTask051ProcessIdentityInterop
{
    private const UInt32 ProcessQueryLimitedInformation = 0x1000;

    public static string ClassifyOpenFailure(Int32 error)
    {
        if (error == 5)
        {
            return "TASK051_PROCESS_INTEROP_OPEN_ACCESS";
        }
        if (error == 87)
        {
            return "TASK051_PROCESS_INTEROP_OPEN_STALE_PID";
        }
        return "TASK051_PROCESS_INTEROP_OPEN_OTHER";
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct FileTime
    {
        public UInt32 Low;
        public UInt32 High;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern IntPtr OpenProcess(UInt32 desiredAccess, bool inheritHandle, UInt32 processId);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool IsProcessInJob(IntPtr process, IntPtr job, out bool result);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool QueryFullProcessImageName(
        IntPtr process,
        UInt32 flags,
        StringBuilder imagePath,
        ref UInt32 size);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool GetProcessTimes(
        IntPtr process,
        out FileTime creation,
        out FileTime exit,
        out FileTime kernel,
        out FileTime user);

    public static LatticeTask051OwnedProcessAuthority Acquire(IntPtr job, Int32 processId)
    {
        return AcquireCore(job, processId, true);
    }

    public static LatticeTask051OwnedProcessAuthority AcquireForSelfTest(Int32 processId)
    {
        return AcquireCore(IntPtr.Zero, processId, false);
    }

    private static LatticeTask051OwnedProcessAuthority AcquireCore(
        IntPtr job,
        Int32 processId,
        bool requireExactJob)
    {
        if ((requireExactJob && job == IntPtr.Zero) || processId < 1)
        {
            throw new InvalidOperationException("TASK051_PROCESS_INTEROP_INPUT");
        }
        IntPtr process = OpenProcess(ProcessQueryLimitedInformation, false, (UInt32)processId);
        if (process == IntPtr.Zero)
        {
            int error = Marshal.GetLastWin32Error();
            throw new InvalidOperationException(ClassifyOpenFailure(error));
        }
        var authority = new LatticeTask051OwnedProcessAuthority(process);
        try
        {
            bool inJob;
            if (!IsProcessInJob(authority.ProcessHandle, job, out inJob))
            {
                throw new InvalidOperationException("TASK051_PROCESS_INTEROP_JOB_QUERY");
            }
            if (requireExactJob && !inJob)
            {
                throw new InvalidOperationException("TASK051_PROCESS_INTEROP_JOB_MEMBERSHIP");
            }
            var imagePath = new StringBuilder(32768);
            UInt32 imagePathLength = (UInt32)imagePath.Capacity;
            if (!QueryFullProcessImageName(authority.ProcessHandle, 0, imagePath, ref imagePathLength))
            {
                throw new InvalidOperationException("TASK051_PROCESS_INTEROP_IMAGE_QUERY");
            }
            FileTime creation;
            FileTime exit;
            FileTime kernel;
            FileTime user;
            if (!GetProcessTimes(authority.ProcessHandle, out creation, out exit, out kernel, out user))
            {
                throw new InvalidOperationException("TASK051_PROCESS_INTEROP_TIME_QUERY");
            }
            UInt64 creationValue = ((UInt64)creation.High << 32) | creation.Low;
            authority.ImagePath = imagePath.ToString();
            authority.CreationFileTimeUtc = checked((Int64)creationValue);
            if (!authority.IsAlive())
            {
                throw new InvalidOperationException("TASK051_PROCESS_INTEROP_EXITED");
            }
            return authority;
        }
        catch
        {
            authority.CloseExact();
            throw;
        }
    }
}
'@
    }
    catch {
        throw 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_INTEROP_INIT_REJECTED'
    }
}

function Read-Task051McpSessionOpen {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$ExpectedNativeIdentity,
        [Parameter(Mandatory = $true)][string]$EvidenceRoot,
        [Parameter(Mandatory = $true)][ValidateScript({ $_ -cmatch '\A[0-9a-f]{32}\z' })][string]$SessionId,
        [Parameter(Mandatory = $true)][ValidateScript({ $_ -cmatch '\A[0-9a-f]{64}\z' })][string]$SafeConfigSha256,
        [switch]$DetailedFailure
    )

    $genericFailureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_REJECTED'
    $failureCode = if ($DetailedFailure) {
        'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_SOURCE_REJECTED'
    }
    else {
        $genericFailureCode
    }
    try {
        $canonicalPath = [IO.Path]::GetFullPath($Path)
        $canonicalRoot = [IO.Path]::GetFullPath($EvidenceRoot)
        Assert-Task051NoReparseAncestor -Path $canonicalPath -Boundary $canonicalRoot -FailureCode $failureCode
        Assert-Task051OwnerOnlyAcl -Path $canonicalPath -Directory $false
        if (-not (Test-LatticeWindowsNativePathIdentity -Path $canonicalPath -Directory $false -ExpectedToken $ExpectedNativeIdentity)) {
            throw $failureCode
        }
        if ($DetailedFailure) { $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_READ_REJECTED' }
        $bytes = [IO.File]::ReadAllBytes($canonicalPath)
        if ($DetailedFailure) { $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_FRAMING_REJECTED' }
        if (
            $bytes.Length -lt 1 -or
            $bytes.Length -gt 65536 -or
            ($bytes.Length -ge 3 -and $bytes[0] -eq 0xef -and $bytes[1] -eq 0xbb -and $bytes[2] -eq 0xbf)
        ) {
            throw $failureCode
        }
        $text = [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
        if (-not $text.EndsWith("`n", [StringComparison]::Ordinal) -or $text.Contains("`r")) {
            throw $failureCode
        }
        $lines = @($text.Split([string[]]@("`n"), [StringSplitOptions]::None))
        if ($lines.Count -ne 2 -or $lines[1] -cne '') { throw $failureCode }
        if ($DetailedFailure) { $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_JSON_REJECTED' }
        $record = $lines[0] | ConvertFrom-Json -ErrorAction Stop
        if ($DetailedFailure) { $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_KEYS_REJECTED' }
        $actualKeys = @($record.PSObject.Properties.Name | Sort-Object)
        $expectedKeys = @(
            'dispatch_accepted_count', 'event_sha256', 'observed_at_unix_nanos',
            'ordinal', 'previous_event_sha256', 'process_id', 'record_type',
            'request_id_sha256', 'safe_config_sha256', 'schema', 'session_id', 'tool_name'
        ) | Sort-Object
        if (($actualKeys -join "`n") -cne ($expectedKeys -join "`n")) { throw $failureCode }
        if ($DetailedFailure) { $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_FIELDS_REJECTED' }
        if (
            ($record.process_id -isnot [int] -and $record.process_id -isnot [long]) -or
            ($record.ordinal -isnot [int] -and $record.ordinal -isnot [long]) -or
            ($record.dispatch_accepted_count -isnot [int] -and $record.dispatch_accepted_count -isnot [long]) -or
            $record.observed_at_unix_nanos -isnot [string] -or
            [string]$record.schema -cne 'lattice.mcp.acceptance-dispatch.v1' -or
            [string]$record.record_type -cne 'SESSION_OPEN' -or
            [string]$record.session_id -cne $SessionId -or
            [string]$record.safe_config_sha256 -cne $SafeConfigSha256 -or
            [long]$record.process_id -lt 1 -or
            [long]$record.process_id -gt [int]::MaxValue -or
            [long]$record.ordinal -ne 1 -or
            [long]$record.dispatch_accepted_count -ne 0 -or
            [string]$record.observed_at_unix_nanos -cnotmatch '\A[1-9][0-9]*\z' -or
            [string]$record.previous_event_sha256 -cne ('0' * 64) -or
            [string]$record.event_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
            $null -ne $record.tool_name -or
            $null -ne $record.request_id_sha256
        ) {
            throw $failureCode
        }
        if ($DetailedFailure) { $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_HASH_REJECTED' }
        $hashInput = @(
            'lattice.mcp.acceptance-dispatch-hash.v1',
            ('0' * 64),
            $SessionId,
            $SafeConfigSha256,
            'SESSION_OPEN',
            '1',
            [string]$record.process_id,
            'null',
            'null',
            '0',
            [string]$record.observed_at_unix_nanos
        ) -join "`n"
        if ([string]$record.event_sha256 -cne (Get-Task051StringSha256 -Value $hashInput)) {
            throw $failureCode
        }
        if ($DetailedFailure) { $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_PROJECTION_REJECTED' }
        return [pscustomobject]@{
            ProcessId = [int]$record.process_id
            ObservedAtUnixNanos = [long]$record.observed_at_unix_nanos
            EventSha256 = [string]$record.event_sha256
        }
    }
    catch {
        throw $failureCode
    }
}

function Test-Task051McpSessionOpenReady {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$ExpectedNativeIdentity,
        [Parameter(Mandatory = $true)][string]$EvidenceRoot
    )

    $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_REJECTED'
    $stream = $null
    try {
        $canonicalPath = [IO.Path]::GetFullPath($Path)
        $canonicalRoot = [IO.Path]::GetFullPath($EvidenceRoot)
        Assert-Task051NoReparseAncestor -Path $canonicalPath -Boundary $canonicalRoot -FailureCode $failureCode
        Assert-Task051OwnerOnlyAcl -Path $canonicalPath -Directory $false
        if (-not (Test-LatticeWindowsNativePathIdentity -Path $canonicalPath -Directory $false -ExpectedToken $ExpectedNativeIdentity)) {
            throw $failureCode
        }
        $share = [IO.FileShare]([int][IO.FileShare]::ReadWrite -bor [int][IO.FileShare]::Delete)
        $stream = [IO.FileStream]::new($canonicalPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, $share)
        if ($stream.Length -eq 0) { return $false }
        if ($stream.Length -gt 65536) { throw $failureCode }
        $stream.Position = $stream.Length - 1
        return $stream.ReadByte() -eq 10
    }
    catch {
        throw $failureCode
    }
    finally {
        if ($null -ne $stream) { $stream.Dispose() }
    }
}

function Get-Task051OwnedProcessEvidence {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Job,
        [Parameter(Mandatory = $true)][ValidateRange(1, 2147483647)][int]$ProcessId,
        [Parameter(Mandatory = $true)][string]$ExpectedExecutable,
        [Parameter(Mandatory = $true)][ValidateScript({ $_ -cmatch '\A[0-9a-f]{64}\z' })][string]$ExpectedExecutableSha256,
        [Parameter(Mandatory = $true)][string]$ExpectedExecutableNativeIdentity,
        [Parameter(Mandatory = $true)][Diagnostics.Process]$OwnerProcess,
        [Parameter(Mandatory = $true)][long]$ObservedAtUnixNanos
    )

    $native = $null
    $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_INTEROP_REJECTED'
    Initialize-Task051ProcessIdentityInterop
    try {
        $native = [LatticeTask051ProcessIdentityInterop]::Acquire($Job, $ProcessId)
    }
    catch {
        $leaf = $_.Exception
        while ($null -ne $leaf.InnerException) { $leaf = $leaf.InnerException }
        switch -CaseSensitive ([string]$leaf.Message) {
            'TASK051_PROCESS_INTEROP_OPEN_ACCESS' { throw 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_OPEN_ACCESS_REJECTED' }
            'TASK051_PROCESS_INTEROP_OPEN_STALE_PID' { throw 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_OPEN_STALE_PID_REJECTED' }
            'TASK051_PROCESS_INTEROP_OPEN_OTHER' { throw 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_OPEN_OTHER_REJECTED' }
            'TASK051_PROCESS_INTEROP_JOB_QUERY' { throw 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_JOB_QUERY_REJECTED' }
            'TASK051_PROCESS_INTEROP_JOB_MEMBERSHIP' { throw 'TASK038_CURRENT_CODEX_DISCOVERY_JOB_MEMBERSHIP_REJECTED' }
            'TASK051_PROCESS_INTEROP_IMAGE_QUERY' { throw 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_IMAGE_QUERY_REJECTED' }
            'TASK051_PROCESS_INTEROP_TIME_QUERY' { throw 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_TIME_QUERY_REJECTED' }
            'TASK051_PROCESS_INTEROP_LIVENESS_QUERY' { throw 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_LIVENESS_REJECTED' }
            'TASK051_PROCESS_INTEROP_EXITED' { throw 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_LIVENESS_REJECTED' }
            'TASK051_PROCESS_INTEROP_CLOSE' { throw 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_CLOSE_REJECTED' }
            default { throw $failureCode }
        }
    }
    try {
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_LIVENESS_REJECTED'
        if (-not $native.IsAlive() -or [string]::IsNullOrWhiteSpace([string]$native.ImagePath)) {
            throw $failureCode
        }
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_FILE_REJECTED'
        Assert-Task051RegularFile -Path ([string]$native.ImagePath) -FailureCode $failureCode
        Assert-Task051RegularFile -Path $ExpectedExecutable -FailureCode $failureCode
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_NATIVE_IDENTITY_REJECTED'
        $expectedIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $ExpectedExecutable -Directory $false
        $actualIdentity = Get-LatticeWindowsNativePathIdentityToken -Path ([string]$native.ImagePath) -Directory $false
        if (
            [string]$expectedIdentity -cne $ExpectedExecutableNativeIdentity -or
            [string]$actualIdentity -cne $ExpectedExecutableNativeIdentity
        ) {
            throw $failureCode
        }
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_SHA256_REJECTED'
        $currentExpectedSha256 = Get-Task051Sha256 -Path $ExpectedExecutable
        $actualSha256 = Get-Task051Sha256 -Path ([string]$native.ImagePath)
        if (
            $currentExpectedSha256 -cne $ExpectedExecutableSha256 -or
            $actualSha256 -cne $ExpectedExecutableSha256
        ) {
            throw $failureCode
        }
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_CREATION_REJECTED'
        $ownerCreationFileTimeUtc = $OwnerProcess.StartTime.ToUniversalTime().ToFileTimeUtc()
        $observedUnixHundredNanoseconds = [decimal]$ObservedAtUnixNanos / [decimal]100
        $observedFileTimeUtc = [long][decimal]::Floor($observedUnixHundredNanoseconds) + 116444736000000000L
        if (
            [long]$native.CreationFileTimeUtc -lt $ownerCreationFileTimeUtc -or
            [long]$native.CreationFileTimeUtc -gt $observedFileTimeUtc
        ) {
            throw $failureCode
        }
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_LIVENESS_REJECTED'
        if (-not $native.IsAlive()) {
            throw $failureCode
        }
        return [pscustomobject]@{
            ProcessId = $ProcessId
            ImagePath = [string]$native.ImagePath
            ImageSha256 = $actualSha256
            NativeIdentity = [string]$actualIdentity
            CreationFileTimeUtc = [long]$native.CreationFileTimeUtc
            Authority = $native
        }
    }
    catch {
        $message = [string]$_.Exception.Message
        try { if ($null -ne $native) { $native.CloseExact() } }
        catch { throw 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_CLOSE_REJECTED' }
        if ($message -match '^TASK038_CURRENT_CODEX_DISCOVERY_[A-Z0-9_]+_REJECTED$') { throw $message }
        throw $failureCode
    }
}

function New-Task051CodexHome {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$Phase,
        [Parameter(Mandatory = $true)][string]$Latticed,
        [Parameter(Mandatory = $true)][string[]]$EnvironmentNames
    )

    $codexHome = [IO.Path]::GetFullPath((Join-Path $Root ('codex-' + $Phase)))
    if (Test-Path -LiteralPath $codexHome) { throw 'TASK051_CODEX_HOME_NOT_FRESH' }
    New-Task051OwnerOnlyDirectory -Path $codexHome
    try {
        $authSource = [Environment]::GetEnvironmentVariable('LATTICE_TASK051_AUTH_SOURCE', 'Process')
        Assert-Task051RegularFile -Path $authSource -FailureCode 'TASK051_CODEX_AUTH_SOURCE_REJECTED'
        $authPath = Join-Path $codexHome 'auth.json'
        [IO.File]::Copy($authSource, $authPath, $false)
        Set-Task051OwnerOnlyAcl -Path $authPath -Directory $false
        $envLiteral = @($EnvironmentNames | Sort-Object -Unique | ForEach-Object {
            ConvertTo-Task051TomlLiteral -Value $_
        }) -join ', '
        $config = @(
            'approval_policy = "never"',
            'sandbox_mode = "read-only"',
            '',
            '[mcp_servers.lattice]',
            ('command = ' + (ConvertTo-Task051TomlLiteral -Value ([IO.Path]::GetFullPath($Latticed)))),
            ('env_vars = [' + $envLiteral + ']'),
            'required = true',
            'startup_timeout_sec = 30',
            'tool_timeout_sec = 330',
            ''
        ) -join [char]10
        $configPath = Join-Path $codexHome 'config.toml'
        [IO.File]::WriteAllText($configPath, $config, [Text.UTF8Encoding]::new($false))
        return [pscustomobject]@{
            Path = $codexHome
            ConfigPath = $configPath
            ConfigSha256 = Get-Task051Sha256 -Path $configPath
            AuthPath = $authPath
        }
    }
    catch {
        $primaryFailure = $_
        $cleanupFailed = $false
        $provisionedAuth = Join-Path $codexHome 'auth.json'
        if (Test-Path -LiteralPath $provisionedAuth -PathType Leaf) {
            try { [IO.File]::Delete($provisionedAuth) } catch { $cleanupFailed = $true }
        }
        if (Test-Path -LiteralPath $provisionedAuth) { $cleanupFailed = $true }
        if (Test-Path -LiteralPath $codexHome -PathType Container) {
            try { [IO.Directory]::Delete($codexHome, $true) } catch { $cleanupFailed = $true }
        }
        if ($cleanupFailed -or (Test-Path -LiteralPath $codexHome)) {
            throw 'TASK051_CODEX_HOME_PROVISIONING_CLEANUP_REJECTED'
        }
        throw $primaryFailure
    }
}

function Remove-Task051CodexCredential {
    param([Parameter(Mandatory = $true)]$CodexHome)

    $auth = [string]$CodexHome.AuthPath
    if (Test-Path -LiteralPath $auth -PathType Leaf) {
        [IO.File]::Delete($auth)
    }
    if (Test-Path -LiteralPath $auth) {
        throw 'TASK051_CODEX_AUTH_CLEANUP_REJECTED'
    }
}

function Complete-Task051InvocationCleanup {
    param(
        $Owned,
        [Parameter(Mandatory = $true)]$CodexHome,
        [int]$KnownServerProcessId = 0,
        $ServerAuthority
    )

    $cleanupFailure = $null
    if ($null -ne $Owned) {
        try { Stop-Task051OwnedProcess -Owned $Owned }
        catch { $cleanupFailure = [string]$_.Exception.Message }
    }
    if ($null -ne $ServerAuthority) {
        try {
            if ($ServerAuthority.IsAlive() -and $null -eq $cleanupFailure) {
                $cleanupFailure = 'TASK051_LATTICED_PROCESS_CLEANUP_REJECTED'
            }
        }
        catch {
            if ($null -eq $cleanupFailure) {
                $cleanupFailure = 'TASK051_LATTICED_PROCESS_CLEANUP_REJECTED'
            }
        }
        try { $ServerAuthority.CloseExact() }
        catch {
            if ($null -eq $cleanupFailure) {
                $cleanupFailure = 'TASK051_PROCESS_HANDLE_CLEANUP_REJECTED'
            }
        }
    }
    try { Remove-Task051CodexCredential -CodexHome $CodexHome }
    catch {
        if ($null -eq $cleanupFailure) {
            $cleanupFailure = [string]$_.Exception.Message
        }
    }
    if (
        $KnownServerProcessId -gt 0 -and
        $null -ne (Get-Process -Id $KnownServerProcessId -ErrorAction SilentlyContinue) -and
        $null -eq $cleanupFailure
    ) {
        $cleanupFailure = 'TASK051_LATTICED_PROCESS_CLEANUP_REJECTED'
    }
    if ($null -ne $cleanupFailure) { throw $cleanupFailure }
}

function Get-Task051McpEnvironment {
    param(
        [Parameter(Mandatory = $true)][ValidateSet('FRESH', 'RESUME_EXISTING')][string]$RunMode,
        [Parameter(Mandatory = $true)]$Authority,
        [Parameter(Mandatory = $true)][string]$DatabasePassword,
        [Parameter(Mandatory = $true)][string]$DeliveryRoot,
        [Parameter(Mandatory = $true)][string]$SchemaDirectory,
        [Parameter(Mandatory = $true)][string]$LauncherSha256,
        [Parameter(Mandatory = $true)][string]$LauncherVersion,
        [Parameter(Mandatory = $true)][string]$AcceptanceEvidencePath,
        [Parameter(Mandatory = $true)][string]$AcceptanceSessionId,
        [Parameter(Mandatory = $true)][string]$SafeConfigSha256,
        [Parameter(Mandatory = $true)][string]$ObservedEffectPath,
        [Parameter(Mandatory = $true)][string]$ObservedEffectNonce
    )

    $values = [ordered]@{
        LATTICE_FULL_CHAIN_RUN_MODE = $RunMode
        LATTICE_TASK_INGRESS_KIND = 'LOCAL_CANONICAL_MCP_ACCEPTANCE'
        LATTICE_TASK_INGRESS_PROFILE_SHA256 = [string]$script:IngressProfileDigest
        LATTICE_DELIVERY_CODEX_MODE = 'OFFICIAL_CODEX_APP_SERVER'
        LATTICE_DELIVERY_TIMEOUT_SECONDS = '300'
        LATTICE_TASK019_HOST = [string]$PostgresHost
        LATTICE_TASK019_PORT = [string]$PostgresPort
        LATTICE_TASK019_RUN_ID = [string]$PostgresRunId
        LATTICE_TASK019_PASSWORD = $DatabasePassword
        LATTICE_STORE_DAEMON_INSTANCE_ID = [string]$Authority.daemon_instance_id
        LATTICE_STORE_DAEMON_EPOCH = [string]$Authority.daemon_epoch
        LATTICE_STORE_AUTHORITY_REVISION = [string]$Authority.authority_revision
        LATTICE_STORE_OBSERVATION_DIGEST = [string]$Authority.observation_digest
        LATTICE_STORE_AUTHORITY_HEAD_DIGEST = [string]$Authority.head_digest
        LATTICE_MCP_ACCEPTANCE_EVIDENCE_PATH = $AcceptanceEvidencePath
        LATTICE_MCP_ACCEPTANCE_SESSION_ID = $AcceptanceSessionId
        LATTICE_MCP_ACCEPTANCE_SAFE_CONFIG_SHA256 = $SafeConfigSha256
        LATTICE_MCP_OBSERVED_EFFECT_PATH = $ObservedEffectPath
        LATTICE_MCP_OBSERVED_EFFECT_NONCE = $ObservedEffectNonce
    }
    if ($RunMode -ceq 'FRESH') {
        $values.LATTICE_DELIVERY_LAUNCHER = [string]$script:OfficialCodex
        $values.LATTICE_DELIVERY_LAUNCHER_VERSION = $LauncherVersion
        $values.LATTICE_DELIVERY_LAUNCHER_SHA256 = $LauncherSha256
        $values.LATTICE_DELIVERY_SCHEMA_DIR = $SchemaDirectory
        $values.LATTICE_DELIVERY_CODEX_HOME = [string]$script:CodexHome
        $values.LATTICE_DELIVERY_ROOT = $DeliveryRoot
        $values.LATTICE_DELIVERY_GIT_EXE = [string]$script:Git
    }
    return $values
}

function Convert-Task051AppServerTools {
    param([Parameter(Mandatory = $true)][object]$Tools)

    $toolRecords = [Collections.Generic.List[object]]::new()
    foreach ($property in @($Tools.PSObject.Properties)) {
        $inputSchemaProperty = $property.Value.PSObject.Properties['inputSchema']
        if ($null -eq $inputSchemaProperty -or $null -eq $inputSchemaProperty.Value) {
            throw 'TASK051_APP_SERVER_DISCOVERY_REJECTED'
        }
        $record = [ordered]@{
            name = [string]$property.Name
            inputSchema = $inputSchemaProperty.Value
        }
        $outputSchemaProperty = $property.Value.PSObject.Properties['outputSchema']
        if ($null -ne $outputSchemaProperty -and $null -ne $outputSchemaProperty.Value) {
            $record.outputSchema = $outputSchemaProperty.Value
        }
        $toolRecords.Add([pscustomobject]$record)
    }
    return @($toolRecords)
}

function Get-Task051UniqueMcpServer {
    param(
        [Parameter(Mandatory = $true)][object[]]$Servers,
        [Parameter(Mandatory = $true)][string]$Name
    )

    $matches = @($Servers | Where-Object { [string]$_.name -ceq $Name })
    if ($matches.Count -eq 0) {
        throw 'TASK038_CURRENT_CODEX_DISCOVERY_LATTICE_SERVER_ZERO_REJECTED'
    }
    if ($matches.Count -gt 1) {
        throw 'TASK038_CURRENT_CODEX_DISCOVERY_LATTICE_SERVER_DUPLICATE_REJECTED'
    }
    return $matches[0]
}

function Get-Task051McpToolNames {
    param([Parameter(Mandatory = $true)][AllowNull()][object]$Tools)

    if ($null -eq $Tools -or $Tools -isnot [Management.Automation.PSCustomObject]) {
        throw 'TASK038_CURRENT_CODEX_DISCOVERY_TOOL_SHAPE_REJECTED'
    }
    $names = [Collections.Generic.List[string]]::new()
    foreach ($property in @($Tools.PSObject.Properties)) {
        $names.Add([string]$property.Name)
    }
    return @($names | Sort-Object)
}

function Invoke-Task051CodexDiscovery {
    param(
        [Parameter(Mandatory = $true)][string]$Phase,
        [Parameter(Mandatory = $true)][string]$EvidenceRoot,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Environment,
        [Parameter(Mandatory = $true)][string]$AcceptanceEvidencePath,
        [Parameter(Mandatory = $true)][string]$AcceptanceNativeIdentity,
        [Parameter(Mandatory = $true)][ValidateScript({ $_ -cmatch '\A[0-9a-f]{32}\z' })][string]$AcceptanceSessionId,
        [Parameter(Mandatory = $true)][ValidateScript({ $_ -cmatch '\A[0-9a-f]{64}\z' })][string]$SafeConfigSha256,
        [Parameter(Mandatory = $true)][ValidateScript({ $_ -cmatch '\A[0-9a-f]{64}\z' })][string]$ExpectedLatticedSha256,
        [Parameter(Mandatory = $true)][string]$ExpectedLatticedNativeIdentity
    )

    $codex = [Environment]::GetEnvironmentVariable('LATTICE_TASK051_CURRENT_CODEX', 'Process')
    Assert-Task051RegularFile -Path $codex -FailureCode 'TASK051_CURRENT_CODEX_REJECTED'
    $codexHome = $null
    $owned = $null
    $process = $null
    $serverProcessId = 0
    $sessionOpen = $null
    $processEvidence = $null
    $serverAuthority = $null
    $watchState = [pscustomobject]@{
        ServerProcessId = 0
        SessionOpen = $null
        ProcessEvidence = $null
    }
    $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_HOME_REJECTED'
    try {
        if (
            [string]$Environment['LATTICE_MCP_ACCEPTANCE_EVIDENCE_PATH'] -cne $AcceptanceEvidencePath -or
            [string]$Environment['LATTICE_MCP_ACCEPTANCE_SESSION_ID'] -cne $AcceptanceSessionId -or
            [string]$Environment['LATTICE_MCP_ACCEPTANCE_SAFE_CONFIG_SHA256'] -cne $SafeConfigSha256
        ) {
            throw 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_REJECTED'
        }
        $codexHome = New-Task051CodexHome -Root $EvidenceRoot -Phase $Phase -Latticed $script:Latticed -EnvironmentNames @($Environment.Keys)
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_START_INFO_REJECTED'
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $codex
        $info.Arguments = 'app-server --stdio'
        $info.WorkingDirectory = $script:RepositoryRoot
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $true
        $info.RedirectStandardInput = $true
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        $childEnvironment = [ordered]@{ CODEX_HOME = [string]$codexHome.Path }
        foreach ($entry in $Environment.GetEnumerator()) { $childEnvironment[$entry.Key] = [string]$entry.Value }
        Set-Task051ClosedEnvironment -StartInfo $info -Additional $childEnvironment
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_START_REJECTED'
        $owned = Start-Task051OwnedProcess -StartInfo $info
        $process = $owned.Process
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_INITIALIZE_REQUEST_REJECTED'
        $initialize = [ordered]@{
            method = 'initialize'
            id = 1
            params = [ordered]@{
                clientInfo = [ordered]@{ name = 'lattice-task051-acceptance'; version = '1' }
                capabilities = [ordered]@{ experimentalApi = $true }
            }
        } | ConvertTo-Json -Compress -Depth 10
        $owned.Suspended.StandardInput.WriteLine($initialize)
        $owned.Suspended.StandardInput.Flush()
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_INITIALIZE_RESPONSE_REJECTED'
        $init = Get-Task051AppServerResponse -Reader $owned.Suspended.StandardOutput -Id 1 -TimeoutSeconds 30
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_IDENTITY_REJECTED'
        $expectedUserAgent = [Environment]::GetEnvironmentVariable('LATTICE_TASK051_CURRENT_CODEX_USER_AGENT', 'Process')
        if (
            [string]::IsNullOrWhiteSpace($expectedUserAgent) -or
            $expectedUserAgent.IndexOfAny([char[]]@("`r", "`n", [char]0)) -ge 0 -or
            [string]$init.result.userAgent -cne $expectedUserAgent
        ) {
            throw 'TASK051_APP_SERVER_IDENTITY_REJECTED'
        }
        $owned.Suspended.StandardInput.WriteLine('{"method":"initialized","params":{}}')
        $owned.Suspended.StandardInput.Flush()
        $sessionOpenPoll = {
            if ($null -ne $watchState.ProcessEvidence) { return $watchState }
            try {
                $ready = Test-Task051McpSessionOpenReady `
                    -Path $AcceptanceEvidencePath `
                    -ExpectedNativeIdentity $AcceptanceNativeIdentity `
                    -EvidenceRoot $EvidenceRoot
            }
            catch {
                throw 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_READY_REJECTED'
            }
            if (-not $ready) { return $null }
            try {
                $parsedSessionOpen = Read-Task051McpSessionOpen `
                    -Path $AcceptanceEvidencePath `
                    -ExpectedNativeIdentity $AcceptanceNativeIdentity `
                    -EvidenceRoot $EvidenceRoot `
                    -SessionId $AcceptanceSessionId `
                    -SafeConfigSha256 $SafeConfigSha256 `
                    -DetailedFailure
            }
            catch {
                $parseFailure = [string]$_.Exception.Message
                switch -CaseSensitive ($parseFailure) {
                    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_SOURCE_REJECTED' { throw $parseFailure }
                    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_READ_REJECTED' { throw $parseFailure }
                    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_FRAMING_REJECTED' { throw $parseFailure }
                    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_JSON_REJECTED' { throw $parseFailure }
                    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_KEYS_REJECTED' { throw $parseFailure }
                    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_FIELDS_REJECTED' { throw $parseFailure }
                    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_HASH_REJECTED' { throw $parseFailure }
                    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_PROJECTION_REJECTED' { throw $parseFailure }
                }
                throw 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_REJECTED'
            }
            $watchState.ServerProcessId = [int]$parsedSessionOpen.ProcessId
            $capturedProcessEvidence = Get-Task051OwnedProcessEvidence `
                -Job ([IntPtr]$owned.Job) `
                -ProcessId ([int]$watchState.ServerProcessId) `
                -ExpectedExecutable $script:Latticed `
                -ExpectedExecutableSha256 $ExpectedLatticedSha256 `
                -ExpectedExecutableNativeIdentity $ExpectedLatticedNativeIdentity `
                -OwnerProcess $process `
                -ObservedAtUnixNanos ([long]$parsedSessionOpen.ObservedAtUnixNanos)
            $watchState.SessionOpen = $parsedSessionOpen
            $watchState.ProcessEvidence = $capturedProcessEvidence
            return $watchState
        }
        $list = $null
        $listRequest = $null
        $server = $null
        $toolNames = @()
        for ($attempt = 0; $attempt -lt 3; $attempt++) {
            $requestId = 2 + $attempt
            $listRequest = '{"method":"mcpServerStatus/list","id":' + [string]$requestId + ',"params":{"detail":"full"}}'
            $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_LIST_REQUEST_REJECTED'
            $owned.Suspended.StandardInput.WriteLine($listRequest)
            $owned.Suspended.StandardInput.Flush()
            $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_LIST_RESPONSE_REJECTED'
            if ($attempt -eq 0) {
                $pollResult = $null
                $list = Get-Task051AppServerResponse `
                    -Reader $owned.Suspended.StandardOutput `
                    -Id $requestId `
                    -TimeoutSeconds 60 `
                    -PollAction $sessionOpenPoll `
                    -PollResult ([ref]$pollResult) `
                    -PollTimeoutFailureCode 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_TIMEOUT_REJECTED'
                if ($null -eq $pollResult -or $null -eq $watchState.ProcessEvidence) {
                    throw 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_POLL_RESULT_REJECTED'
                }
                $serverProcessId = [int]$watchState.ServerProcessId
                $sessionOpen = $watchState.SessionOpen
                $processEvidence = $watchState.ProcessEvidence
                $serverAuthority = $processEvidence.Authority
            }
            else {
                $list = Get-Task051AppServerResponse -Reader $owned.Suspended.StandardOutput -Id $requestId -TimeoutSeconds 60
            }
            $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_SERVER_COUNT_ZERO_REJECTED'
            $servers = @($list.result.data)
            if ($servers.Count -eq 0) {
                throw 'TASK051_APP_SERVER_DISCOVERY_REJECTED'
            }
            $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_LATTICE_SERVER_COUNT_REJECTED'
            $server = Get-Task051UniqueMcpServer -Servers $servers -Name 'lattice'
            $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_TOOL_SHAPE_REJECTED'
            $toolNames = @(Get-Task051McpToolNames -Tools $server.tools)
            if ($toolNames.Count -gt 0) { break }
            if ($attempt -lt 2) { Start-Sleep -Milliseconds 250 }
        }
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_TOOL_COUNT_ZERO_REJECTED'
        if ($toolNames.Count -eq 0) {
            throw 'TASK051_APP_SERVER_DISCOVERY_REJECTED'
        }
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_TOOL_COUNT_REJECTED'
        if ($toolNames.Count -ne $script:Task051ExpectedTools.Count) {
            throw 'TASK051_APP_SERVER_DISCOVERY_REJECTED'
        }
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_TOOL_NAMES_REJECTED'
        if (($toolNames -join [char]10) -cne (($script:Task051ExpectedTools | Sort-Object) -join [char]10)) {
            throw 'TASK051_APP_SERVER_DISCOVERY_REJECTED'
        }
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_RESOURCES_REJECTED'
        if (@($server.resources).Count -ne 0) {
            throw 'TASK051_APP_SERVER_DISCOVERY_REJECTED'
        }
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_RESOURCE_TEMPLATES_REJECTED'
        if (@($server.resourceTemplates).Count -ne 0) {
            throw 'TASK051_APP_SERVER_DISCOVERY_REJECTED'
        }
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_TOOL_SCHEMA_REJECTED'
        $toolRecords = @(Convert-Task051AppServerTools -Tools $server.tools)
        Assert-ToolDiscovery -Response ([pscustomobject]@{
            result = [pscustomobject]@{ tools = $toolRecords }
        })
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_REPLAY_READ_REJECTED'
        try {
            $sessionOpenReplay = Read-Task051McpSessionOpen `
                -Path $AcceptanceEvidencePath `
                -ExpectedNativeIdentity $AcceptanceNativeIdentity `
                -EvidenceRoot $EvidenceRoot `
                -SessionId $AcceptanceSessionId `
                -SafeConfigSha256 $SafeConfigSha256
        }
        catch {
            throw $failureCode
        }
        if (
            $null -eq $sessionOpen -or
            $null -eq $processEvidence -or
            $null -eq $serverAuthority -or
            [int]$sessionOpenReplay.ProcessId -ne [int]$sessionOpen.ProcessId -or
            [long]$sessionOpenReplay.ObservedAtUnixNanos -ne [long]$sessionOpen.ObservedAtUnixNanos -or
            [string]$sessionOpenReplay.EventSha256 -cne [string]$sessionOpen.EventSha256
        ) {
            throw 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_REPLAY_MISMATCH_REJECTED'
        }
        $failureCode = 'TASK038_CURRENT_CODEX_DISCOVERY_EVIDENCE_WRITE_REJECTED'
        $evidence = Write-Task051JsonEvidence -Path (Join-Path $EvidenceRoot ('task051-' + $Phase + '-discovery.json')) -Value ([ordered]@{
            schema_version = 'lattice.task051.current-codex-discovery.v1'
            phase = $Phase
            codex_process_id = [int]$process.Id
            latticed_process_id = $serverProcessId
            latticed_sha256 = [string]$processEvidence.ImageSha256
            latticed_native_identity = [string]$processEvidence.NativeIdentity
            latticed_creation_file_time_utc = [long]$processEvidence.CreationFileTimeUtc
            session_open_event_sha256 = [string]$sessionOpen.EventSha256
            codex_sha256 = Get-Task051Sha256 -Path $codex
            config_sha256 = [string]$codexHome.ConfigSha256
            user_agent = [string]$init.result.userAgent
            tool_names = $toolNames
            tool_schemas = @($toolRecords)
            initialize_request_sha256 = Get-Task051StringSha256 -Value $initialize
            initialize_response_sha256 = Get-Task051StringSha256 -Value ($init | ConvertTo-Json -Compress -Depth 50)
            list_request_sha256 = Get-Task051StringSha256 -Value $listRequest
            list_response_sha256 = Get-Task051StringSha256 -Value ($list | ConvertTo-Json -Compress -Depth 50)
        })
        return [pscustomobject]@{
            ProcessId = [int]$process.Id
            ServerProcessId = $serverProcessId
            ServerSha256 = [string]$processEvidence.ImageSha256
            ServerNativeIdentity = [string]$processEvidence.NativeIdentity
            ConfigSha256 = [string]$codexHome.ConfigSha256
            CodexSha256 = Get-Task051Sha256 -Path $codex
            UserAgent = [string]$init.result.userAgent
            ToolNames = $toolNames
            EvidencePath = [string]$evidence.Path
            EvidenceSha256 = [string]$evidence.Sha256
        }
    }
    catch {
        $message = [string]$_.Exception.Message
        if ($message -match '^(?:TASK038|LATTICE)_[A-Z0-9_]{1,127}(?:\|[A-Z0-9_]{1,127}|\|[0-9a-f]{64})*$') {
            throw $message
        }
        throw $failureCode
    }
    finally {
        try { if ($null -ne $owned) { $owned.Suspended.StandardInput.Close() } } catch {}
        if ($null -ne $codexHome) {
            $knownServerProcessId = if ($serverProcessId -gt 0) { $serverProcessId } else { [int]$watchState.ServerProcessId }
            $authorityForCleanup = if ($null -ne $serverAuthority) { $serverAuthority } elseif ($null -ne $watchState.ProcessEvidence) { $watchState.ProcessEvidence.Authority } else { $null }
            Complete-Task051InvocationCleanup `
                -Owned $owned `
                -CodexHome $codexHome `
                -KnownServerProcessId $knownServerProcessId `
                -ServerAuthority $authorityForCleanup
        }
    }
}

function Get-Task051ExecStructuredContent {
    param(
        [Parameter(Mandatory = $true)][object[]]$Events,
        [Parameter(Mandatory = $true)][string]$Tool,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$ExpectedArguments
    )

    $calls = [Collections.Generic.List[object]]::new()
    foreach ($event in $Events) {
        if ([string]$event.type -in @('item.started', 'item.completed')) {
            $itemType = [string]$event.item.type
            if ($itemType -notin @('mcp_tool_call', 'agent_message', 'reasoning', 'todo_list')) {
                throw 'TASK051_CODEX_UNEXPECTED_TOOL_REJECTED'
            }
            if ([string]$event.type -ceq 'item.completed' -and $itemType -ceq 'mcp_tool_call') {
                $calls.Add($event.item)
            }
        }
    }
    if ($calls.Count -ne 1) {
        if ($Tool -ceq 'lattice_task_submit') { throw 'TASK051_CODEX_SUBMIT_CALL_COUNT_REJECTED' }
        throw 'TASK051_CODEX_STATUS_CALL_COUNT_REJECTED'
    }
    $call = $calls[0]
    if ([string]$call.server -cne 'lattice' -or [string]$call.tool -cne $Tool -or [string]$call.status -cne 'completed') {
        throw 'TASK051_CODEX_TOOL_IDENTITY_REJECTED'
    }
    $actualArguments = $call.arguments
    $actualKeys = @($actualArguments.PSObject.Properties.Name | Sort-Object)
    $expectedKeys = @($ExpectedArguments.Keys | Sort-Object)
    if (($actualKeys -join [char]10) -cne ($expectedKeys -join [char]10)) {
        throw 'TASK051_CODEX_TOOL_ARGUMENT_REJECTED'
    }
    foreach ($name in $expectedKeys) {
        if ([string]$actualArguments.$name -cne [string]$ExpectedArguments[$name]) {
            throw 'TASK051_CODEX_TOOL_ARGUMENT_REJECTED'
        }
    }
    if ($null -ne $call.error -or $null -eq $call.result) {
        throw 'TASK051_CODEX_TOOL_RESULT_REJECTED'
    }
    $resultKeys = @($call.result.PSObject.Properties.Name | Sort-Object)
    if (($resultKeys -join ',') -cne '_meta,content,structured_content') {
        throw 'TASK051_CODEX_TOOL_RESULT_ENVELOPE_REJECTED'
    }
    $content = @($call.result.content)
    if (
        $content.Count -ne 1 -or
        (@($content[0].PSObject.Properties.Name | Sort-Object) -join ',') -cne 'text,type' -or
        [string]$content[0].type -cne 'text' -or
        -not ($content[0].text -is [string])
    ) {
        throw 'TASK051_CODEX_TOOL_RESULT_ENVELOPE_REJECTED'
    }
    $meta = $call.result._meta
    if ($null -eq $meta) { throw 'TASK051_CODEX_TOOL_RESULT_ENVELOPE_REJECTED' }
    $serverInfo = $meta.PSObject.Properties['io.modelcontextprotocol/serverInfo']
    if (
        @($meta.PSObject.Properties).Count -ne 1 -or
        $null -eq $serverInfo -or
        (@($serverInfo.Value.PSObject.Properties.Name | Sort-Object) -join ',') -cne 'name,title,version' -or
        [string]$serverInfo.Value.name -cne 'latticed' -or
        [string]$serverInfo.Value.title -cne 'LATTICE DevOS' -or
        [string]$serverInfo.Value.version -cne '1.0.0'
    ) {
        throw 'TASK051_CODEX_TOOL_RESULT_ENVELOPE_REJECTED'
    }
    $structured = $call.result.structured_content
    if ($null -eq $structured) { throw 'TASK051_CODEX_TOOL_RESULT_REJECTED' }
    try { $contentValue = [string]$content[0].text | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'TASK051_CODEX_TOOL_RESULT_ENVELOPE_REJECTED' }
    Assert-Task051SameStatus -Expected $structured -Actual $contentValue
    return [pscustomobject]@{
        StructuredContent = $structured
        ContentSha256 = Get-Task051StringSha256 -Value ([string]$content[0].text)
        MetaSha256 = Get-Task051StringSha256 -Value ($meta | ConvertTo-Json -Compress -Depth 20)
        ResultSha256 = Get-Task051StringSha256 -Value ($call.result | ConvertTo-Json -Compress -Depth 30)
    }
}

function Invoke-Task051CodexTool {
    param(
        [Parameter(Mandatory = $true)][string]$Phase,
        [Parameter(Mandatory = $true)][ValidateSet('lattice_task_submit', 'lattice_task_status')][string]$Tool,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Arguments,
        [Parameter(Mandatory = $true)][ValidateSet('FRESH', 'RESUME_EXISTING')][string]$RunMode,
        [Parameter(Mandatory = $true)][string]$EvidenceRoot,
        [Parameter(Mandatory = $true)]$Authority,
        [Parameter(Mandatory = $true)][string]$DatabasePassword,
        [Parameter(Mandatory = $true)][string]$DeliveryRoot,
        [Parameter(Mandatory = $true)][string]$SchemaDirectory,
        [Parameter(Mandatory = $true)][string]$LauncherSha256,
        [Parameter(Mandatory = $true)][string]$LauncherVersion
    )

    $codex = [Environment]::GetEnvironmentVariable('LATTICE_TASK051_CURRENT_CODEX', 'Process')
    Assert-Task051RegularFile -Path $codex -FailureCode 'TASK051_CURRENT_CODEX_REJECTED'
    $sessionId = [Guid]::NewGuid().ToString('N')
    $acceptanceSink = New-Task038McpAcceptanceEvidenceSink -EvidenceRoot $EvidenceRoot -SessionId $sessionId
    $observedSink = New-Task038McpObservedEffectEvidenceSink -AcceptanceEvidencePath ([string]$acceptanceSink.path) -SessionId $sessionId
    $safeConfig = Get-Task051StringSha256 -Value (@(
        'lattice.task051.current-codex-session.v1',
        $Phase,
        $Tool,
        $RunMode,
        ($Arguments | ConvertTo-Json -Compress -Depth 8)
    ) -join [char]10)
    $environment = Get-Task051McpEnvironment -RunMode $RunMode -Authority $Authority -DatabasePassword $DatabasePassword -DeliveryRoot $DeliveryRoot -SchemaDirectory $SchemaDirectory -LauncherSha256 $LauncherSha256 -LauncherVersion $LauncherVersion -AcceptanceEvidencePath ([string]$acceptanceSink.path) -AcceptanceSessionId $sessionId -SafeConfigSha256 $safeConfig -ObservedEffectPath ([string]$observedSink.path) -ObservedEffectNonce ([string]$observedSink.nonce)
    $codexHome = $null
    $owned = $null
    $process = $null
    $serverProcessId = 0
    try {
        $codexHome = New-Task051CodexHome -Root $EvidenceRoot -Phase $Phase -Latticed $script:Latticed -EnvironmentNames @($environment.Keys)
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $codex
        $info.Arguments = 'exec --ephemeral --json --color never --sandbox read-only --ignore-rules --skip-git-repo-check -C "' + $EvidenceRoot + '" -'
        $info.WorkingDirectory = $EvidenceRoot
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $true
        $info.RedirectStandardInput = $true
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        $childEnvironment = [ordered]@{ CODEX_HOME = [string]$codexHome.Path }
        foreach ($entry in $environment.GetEnumerator()) { $childEnvironment[$entry.Key] = [string]$entry.Value }
        Set-Task051ClosedEnvironment -StartInfo $info -Additional $childEnvironment
        $owned = Start-Task051OwnedProcess -StartInfo $info
        $process = $owned.Process
        $prompt = if ($Tool -ceq 'lattice_task_submit') {
            'Call only the MCP tool lattice_task_submit on server lattice exactly once with client_request_id "' + [string]$Arguments.client_request_id + '" and intent "CONTROLLED_CODEX_CANARY". Do not use any other tool. After the tool returns, output TASK051_CODEX_SUBMIT_OK.'
        }
        else {
            'Call only the MCP tool lattice_task_status on server lattice exactly once with task_ref "' + [string]$Arguments.task_ref + '". Do not use any other tool. After the tool returns, output TASK051_CODEX_STATUS_OK.'
        }
        $owned.Suspended.StandardInput.Write($prompt)
        $owned.Suspended.StandardInput.Close()
        $stdoutTask = $owned.Suspended.StandardOutput.ReadToEndAsync()
        $stderrTask = $owned.Suspended.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(360000)) {
            throw 'TASK051_CODEX_EXEC_TIMEOUT'
        }
        $stdout = [string]$stdoutTask.GetAwaiter().GetResult()
        $stderr = [string]$stderrTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0 -or $stdout.Length -gt 1048576 -or $stderr.Length -gt 1048576) {
            throw ('TASK051_CODEX_EXEC_REJECTED|' + (Get-Task051StringSha256 -Value $stderr))
        }
        Assert-SecretFreeText -Text ($stdout + [char]10 + $stderr) -FailureCode 'TASK051_CODEX_OUTPUT_SECRET_REJECTED'
        $events = @(Read-Task051JsonLines -Text $stdout -FailureCode 'TASK051_CODEX_EVENT_JSON_REJECTED')
        $envelope = Get-Task051ExecStructuredContent -Events $events -Tool $Tool -ExpectedArguments $Arguments
        $structured = $envelope.StructuredContent
        Assert-Task051PublicStatus -Value $structured -Kind $(if ($Tool -ceq 'lattice_task_submit') { 'SUBMIT' } else { 'STATUS' })
        $firstDispatchLine = Get-Content -LiteralPath ([string]$acceptanceSink.path) -TotalCount 1
        $serverProcessId = [int](($firstDispatchLine | ConvertFrom-Json -ErrorAction Stop).process_id)
        $dispatch = Read-Task038McpAcceptanceEvidence -Path ([string]$acceptanceSink.path) -ExpectedNativeIdentity ([string]$acceptanceSink.native_identity) -SessionId $sessionId -SafeConfigSha256 $safeConfig -ProcessId $serverProcessId -ExpectedDispatchCount 1
        $effects = Read-Task038McpObservedEffectEvidence -Path ([string]$observedSink.path) -ExpectedNativeIdentity ([string]$observedSink.native_identity) -SessionId $sessionId -SafeConfigSha256 $safeConfig -Nonce ([string]$observedSink.nonce) -ProcessId $serverProcessId
        if ($null -ne (Get-Process -Id $serverProcessId -ErrorAction SilentlyContinue)) {
            throw 'TASK051_LATTICED_PROCESS_CLEANUP_REJECTED'
        }
        if (
            [long]$effects.completed_probe_count -ne 1 -or
            [long]$effects.rejected_probe_count -ne 0 -or
            [long]$effects.session_counters.dispatch -ne 1 -or
            [long]$effects.session_counters.database -lt 1 -or
            [long]$effects.session_counters.network -lt 1
        ) {
            throw 'TASK051_CODEX_OBSERVED_EFFECT_REJECTED'
        }
        if (
            $Tool -ceq 'lattice_task_status' -and (
                [long]$effects.session_counters.filesystem -ne 0 -or
                [long]$effects.session_counters.process -ne 0 -or
                [long]$effects.session_counters.codex -ne 0
            )
        ) {
            throw 'TASK051_CODEX_STATUS_DUPLICATE_EFFECT_REJECTED'
        }
        $evidence = Write-Task051JsonEvidence -Path (Join-Path $EvidenceRoot ('task051-' + $Phase + '-tool-call.json')) -Value ([ordered]@{
            schema_version = 'lattice.task051.current-codex-tool-call.v1'
            phase = $Phase
            tool = $Tool
            run_mode = $RunMode
            codex_process_id = [int]$process.Id
            latticed_process_id = $serverProcessId
            codex_sha256 = Get-Task051Sha256 -Path $codex
            config_sha256 = [string]$codexHome.ConfigSha256
            safe_config_sha256 = $safeConfig
            prompt_sha256 = Get-Task051StringSha256 -Value $prompt
            arguments_sha256 = Get-Task051StringSha256 -Value ($Arguments | ConvertTo-Json -Compress -Depth 10)
            content_sha256 = [string]$envelope.ContentSha256
            meta_sha256 = [string]$envelope.MetaSha256
            result_sha256 = [string]$envelope.ResultSha256
            structured_content_sha256 = Get-Task051StringSha256 -Value ($structured | ConvertTo-Json -Compress -Depth 20)
            dispatch_raw_sha256 = [string]$dispatch.raw_sha256
            observed_effect_raw_sha256 = [string]$effects.raw_sha256
            observed_effect_final_hmac_sha256 = [string]$effects.final_hmac_sha256
            observed_effect_counters = $effects.session_counters
        })
        return [pscustomobject]@{
            ProcessId = [int]$process.Id
            ServerProcessId = $serverProcessId
            StructuredContent = $structured
            ConfigSha256 = [string]$codexHome.ConfigSha256
            CodexSha256 = Get-Task051Sha256 -Path $codex
            DispatchRawSha256 = [string]$dispatch.raw_sha256
            ObservedEffectRawSha256 = [string]$effects.raw_sha256
            ObservedEffectFinalHmacSha256 = [string]$effects.final_hmac_sha256
            ObservedEffectCounters = $effects.session_counters
            EvidencePath = [string]$evidence.Path
            EvidenceSha256 = [string]$evidence.Sha256
        }
    }
    finally {
        if ($null -ne $codexHome) {
            Complete-Task051InvocationCleanup -Owned $owned -CodexHome $codexHome -KnownServerProcessId $serverProcessId
        }
    }
}

function Replace-Task051Exact {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Old,
        [Parameter(Mandatory = $true)][string]$New,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $count = ([regex]::Matches($Source, [regex]::Escape($Old))).Count
    if ($count -ne 1) { throw ($FailureCode + '|' + $count) }
    return $Source.Replace($Old, $New)
}

function Convert-Task051Task038Source {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$ScriptsRoot,
        [Parameter(Mandatory = $true)][string]$RunnerPath
    )

    $qScripts = ConvertTo-Task051TomlLiteral -Value ([IO.Path]::GetFullPath($ScriptsRoot))
    $qRunner = ConvertTo-Task051TomlLiteral -Value ([IO.Path]::GetFullPath($RunnerPath))
    $Source = $Source.Replace('$PSScriptRoot', $qScripts)
    $Source = Replace-Task051Exact -Source $Source -Old 'Set-StrictMode -Version Latest' -New ('. ' + $qRunner + ' -LibraryOnly' + [char]10 + 'Set-StrictMode -Version Latest') -FailureCode 'TASK051_TASK038_LIBRARY_INSERT_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$repositoryTarget = Get-CanonicalPath -Path (Join-Path $script:RepositoryRoot ''target'')' -New '$repositoryTarget = Get-CanonicalPath -Path $env:LATTICE_TASK051_RUN_ROOT' -FailureCode 'TASK051_TASK038_TARGET_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$task038CargoTarget = Get-CanonicalPath -Path (Join-Path $repositoryTarget ''task038-main'')' -New '$task038CargoTarget = Get-CanonicalPath -Path $env:CARGO_TARGET_DIR' -FailureCode 'TASK051_TASK038_CARGO_TARGET_PATH_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old 'Assert-NoReparseAncestor -Path $task038CargoTarget -Boundary $script:RepositoryRoot -FailureCode ''TASK038_CARGO_TARGET_REJECTED''' -New 'Assert-NoReparseAncestor -Path $task038CargoTarget -Boundary $env:CARGO_TARGET_DIR -FailureCode ''TASK038_CARGO_TARGET_REJECTED''' -FailureCode 'TASK051_TASK038_CARGO_TARGET_BOUNDARY_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$cargoHostTarget = ''x86_64-pc-windows-msvc''' -New @'
$cargoHostTarget = 'x86_64-pc-windows-msvc'
$cargoVersion = Invoke-NativeText -Executable $script:Cargo -WorkingDirectory $script:RepositoryRoot -Arguments @('-vV')
Assert-SecretFreeText -Text $cargoVersion.Text -FailureCode 'TASK051_TASK038_CARGO_HOST_REJECTED'
$cargoHostLines = @($cargoVersion.Text -split '\r?\n' | Where-Object { $_ -like 'host: *' })
if (
    $cargoVersion.ExitCode -ne 0 -or
    $cargoHostLines.Count -ne 1 -or
    [string]$cargoHostLines[0] -cne ('host: ' + $cargoHostTarget)
) {
    throw 'TASK051_TASK038_CARGO_HOST_REJECTED'
}
'@ -FailureCode 'TASK051_TASK038_CARGO_HOST_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '    ''--target-dir'', $task038CargoTarget, ''--target'', $cargoHostTarget' -New '    ''--target-dir'', $task038CargoTarget' -FailureCode 'TASK051_TASK038_HOST_TARGET_ARGUMENT_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$script:Latticed = Get-CanonicalPath -Path (Join-Path $task038CargoTarget ($cargoHostTarget + ''\debug\latticed.exe''))' -New '$script:Latticed = Get-CanonicalPath -Path (Join-Path $task038CargoTarget ''debug\latticed.exe'')' -FailureCode 'TASK051_TASK038_HOST_TARGET_BINARY_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$candidateLatticedSha256 = Get-FileSha256 -Path $script:Latticed' -New @'
$candidateLatticedSha256 = Get-FileSha256 -Path $script:Latticed
$candidateLatticedNativeIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $script:Latticed -Directory $false
'@ -FailureCode 'TASK051_TASK038_CANDIDATE_BINARY_COMMITMENT_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old @'
        'INITIAL_POSTMASTER_STOPPED', 'RESTART_POSTMASTER_READY', 'CONSUMER_STARTED'
'@ -New @'
        'INITIAL_POSTMASTER_STOPPED', 'RESTART_POSTMASTER_READY',
        'TASK076_WRITER_V2_VERIFIED', 'CONSUMER_STARTED'
'@ -FailureCode 'TASK051_TASK038_HOLDER_EVENT_SEQUENCE_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$consumer = $records[5].payload' -New '$consumer = $records[6].payload' -FailureCode 'TASK051_TASK038_HOLDER_CONSUMER_INDEX_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$expectedTypes = @(' -New @'
$failureCode = 'TASK038_POSTGRES_HOLDER_PREFIX_REJECTED'
$expectedTypes = @(
'@ -FailureCode 'TASK051_TASK038_HOLDER_PREFIX_STAGE_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$previous = ''0'' * 64' -New @'
$failureCode = 'TASK038_POSTGRES_HOLDER_CHAIN_REJECTED'
$previous = '0' * 64
'@ -FailureCode 'TASK051_TASK038_HOLDER_CHAIN_STAGE_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$holder = $records[0].payload' -New @'
$failureCode = 'TASK038_POSTGRES_HOLDER_STATE_REJECTED'
$holder = $records[0].payload
'@ -FailureCode 'TASK051_TASK038_HOLDER_STATE_STAGE_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '        $listeners = @(Get-NetTCPConnection -State Listen -LocalPort $PostgresPort -ErrorAction Stop | Where-Object {' -New @'
        $failureCode = 'TASK038_POSTGRES_HOLDER_LISTENER_SOCKET_REJECTED'
        $listeners = @(Get-NetTCPConnection -State Listen -LocalPort $PostgresPort -ErrorAction Stop | Where-Object {
'@ -FailureCode 'TASK051_TASK038_HOLDER_LISTENER_STAGE_TRANSFORM_REJECTED'
    $listenerDiagnosticBlock = @'
        $failureCode = 'TASK038_POSTGRES_HOLDER_LISTENER_PROCESS_QUERY_REJECTED'
        $listenerProcess = Get-CimInstance -ClassName Win32_Process -Filter ('ProcessId = ' + [long]$restart.listener_process_id) -ErrorAction Stop
        $failureCode = 'TASK038_POSTGRES_HOLDER_LISTENER_EXECUTABLE_REJECTED'
        if (-not (Test-ExactPath -Actual ([string]$listenerProcess.ExecutablePath) -Expected $script:PostgresExecutable)) {
            throw $failureCode
        }
        $failureCode = 'TASK038_POSTGRES_HOLDER_LISTENER_RECEIPT_REJECTED'
        if (
            -not (Test-ExactPath -Actual ([string]$restart.listener_executable_path) -Expected $script:PostgresExecutable) -or
            [string]$restart.listener_executable_sha256 -cne $expectedPostgresExecutableSha256 -or
            [string]$restart.listener_executable_native_identity -cne $script:PostgresExecutableNativeIdentity -or
            -not (Test-ExactPath -Actual ([string]$restart.listener_data_directory) -Expected $script:PostgresData) -or
            [string]$restart.listener_host -cne '127.0.0.1' -or
            [long]$restart.listener_port -ne [long]$PostgresPort
        ) {
            throw $failureCode
        }
        $failureCode = 'TASK038_POSTGRES_HOLDER_LISTENER_CREATION_REJECTED'
        if (([DateTimeOffset]([DateTime]$listenerProcess.CreationDate)).ToUniversalTime().ToFileTime().ToString() -cne [string]$restart.listener_process_creation_time) {
            throw $failureCode
        }
        $failureCode = 'TASK038_POSTGRES_HOLDER_LISTENER_REJECTED'
'@
    $Source = Replace-Task051Exact -Source $Source -Old '        $listenerProcess = Get-CimInstance -ClassName Win32_Process -Filter (''ProcessId = '' + [long]$restart.listener_process_id) -ErrorAction Stop' -New $listenerDiagnosticBlock -FailureCode 'TASK051_TASK038_HOLDER_LISTENER_DIAGNOSTIC_TRANSFORM_REJECTED'
    $listenerCommandLineCondition = @'
            -not (Test-ExactPath -Actual ([string]$restart.listener_executable_path) -Expected $script:PostgresExecutable) -or
            [string]$restart.listener_executable_sha256 -cne $expectedPostgresExecutableSha256 -or
            [string]$restart.listener_executable_native_identity -cne $script:PostgresExecutableNativeIdentity -or
            -not (Test-ExactPath -Actual ([string]$restart.listener_data_directory) -Expected $script:PostgresData) -or
            [string]$restart.listener_host -cne '127.0.0.1' -or
            [long]$restart.listener_port -ne [long]$PostgresPort -or
'@
    $Source = Replace-Task051Exact -Source $Source -Old '            [string]$listenerProcess.CommandLine -notlike (''*'' + $script:PostgresData + ''*'') -or' -New $listenerCommandLineCondition -FailureCode 'TASK051_TASK038_HOLDER_LISTENER_RECEIPT_TRANSFORM_REJECTED'
    $postgresDataBlock = @'
$script:PostgresData = Get-CanonicalPath -Path $PostgresDataDirectory
$dataItem = Get-Item -LiteralPath $script:PostgresData -Force -ErrorAction SilentlyContinue
if ($null -eq $dataItem -or -not $dataItem.PSIsContainer -or ($dataItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw 'TASK038_POSTGRES_DATA_REJECTED'
}
Assert-NoReparseAncestor -Path $script:PostgresData -Boundary $script:RepositoryRoot -FailureCode 'TASK038_POSTGRES_DATA_REJECTED'
$clusterRoot = Get-CanonicalPath -Path (Split-Path -Parent $script:PostgresData)
'@
    $task051PostgresDataBlock = @'
$script:PostgresData = Get-CanonicalPath -Path $PostgresDataDirectory
$task051PhysicalPostgresData = Get-CanonicalPath -Path (Join-Path $env:LATTICE_TASK051_RUN_ROOT ('task019-postgres\' + $PostgresRunId + '\data'))
$dataItem = Get-Item -LiteralPath $script:PostgresData -Force -ErrorAction SilentlyContinue
$task051PhysicalDataItem = Get-Item -LiteralPath $task051PhysicalPostgresData -Force -ErrorAction SilentlyContinue
if (
    $null -eq $dataItem -or -not $dataItem.PSIsContainer -or ($dataItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
    $null -eq $task051PhysicalDataItem -or -not $task051PhysicalDataItem.PSIsContainer -or ($task051PhysicalDataItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
) {
    throw 'TASK038_POSTGRES_DATA_REJECTED'
}
Assert-NoReparseAncestor -Path $script:PostgresData -Boundary $env:LATTICE_TASK051_RUN_ALIAS_ROOT -FailureCode 'TASK038_POSTGRES_DATA_REJECTED'
Assert-NoReparseAncestor -Path $task051PhysicalPostgresData -Boundary $repositoryTarget -FailureCode 'TASK038_POSTGRES_DATA_REJECTED'
if (
    (Get-LatticeWindowsNativePathIdentityToken -Path $script:PostgresData -Directory $true) -cne
    (Get-LatticeWindowsNativePathIdentityToken -Path $task051PhysicalPostgresData -Directory $true)
) {
    throw 'TASK038_POSTGRES_DATA_NATIVE_LINK_REJECTED'
}
$script:Task051PhysicalPostgresRoot = Get-CanonicalPath -Path (Split-Path -Parent $task051PhysicalPostgresData)
$script:Task051PhysicalPostgresParent = Get-CanonicalPath -Path (Split-Path -Parent $script:Task051PhysicalPostgresRoot)
$clusterRoot = $script:Task051PhysicalPostgresRoot
'@
    $Source = Replace-Task051Exact -Source $Source -Old $postgresDataBlock -New $task051PostgresDataBlock -FailureCode 'TASK051_TASK038_POSTGRES_DATA_ALIAS_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '-not (Test-ExactPath -Actual ([string]$holder.cluster_root) -Expected (Split-Path -Parent $script:PostgresData)) -or' -New '-not (Test-ExactPath -Actual ([string]$holder.cluster_root) -Expected $script:Task051PhysicalPostgresRoot) -or' -FailureCode 'TASK051_TASK038_HOLDER_ROOT_TRANSFORM_REJECTED'
    $holderDiagnosticBlock = @'
$currentOwnerExecutable = Get-CanonicalPath -Path ([string]$currentOwner.Path)
if (
    [long]$holder.owner_process_id -ne [long]$PID -or
    [string]$holder.owner_process_creation_time -cne $currentOwner.StartTime.ToUniversalTime().ToFileTimeUtc().ToString() -or
    -not (Test-ExactPath -Actual ([string]$holder.owner_process_executable) -Expected $currentOwnerExecutable) -or
    [string]$holder.owner_process_executable_sha256 -cne (Get-FileSha256 -Path $currentOwnerExecutable) -or
    [string]$holder.owner_process_executable_native_identity -cne (Get-LatticeWindowsNativePathIdentityToken -Path $currentOwnerExecutable -Directory $false)
) { throw 'TASK038_POSTGRES_HOLDER_OWNER_PROCESS_REJECTED' }
if (
    -not (Test-ExactPath -Actual ([string]$holder.cluster_root) -Expected $script:Task051PhysicalPostgresRoot) -or
    -not (Test-ExactPath -Actual ([string]$holder.data_directory) -Expected $script:PostgresData) -or
    -not (Test-ExactPath -Actual ([string]$holder.authority_receipt_path) -Expected $ReceiptPath) -or
    [string]$holder.authority_receipt_native_identity -cne $receiptNativeIdentity
) { throw 'TASK038_POSTGRES_HOLDER_SCOPE_REJECTED' }
if (
    [string]$holder.tool_identity.postgres_version -cne '17.10' -or
    [string]$holder.tool_identity.postgres_sha256 -cne $expectedPostgresExecutableSha256 -or
    [string]$holder.tool_identity.psql_sha256 -cne $expectedPsqlExecutableSha256 -or
    [string]$holder.tool_identity.pg_ctl_sha256 -cne $expectedPgCtlExecutableSha256 -or
    [string]$holder.tool_identity.postgres_native_identity -cne $script:PostgresExecutableNativeIdentity -or
    [string]$holder.tool_identity.psql_native_identity -cne $script:PsqlNativeIdentity -or
    [string]$holder.tool_identity.pg_ctl_native_identity -cne $script:PgCtlNativeIdentity
) { throw 'TASK038_POSTGRES_HOLDER_TOOL_REJECTED' }
if (
    -not (Test-ExactPath -Actual ([string]$marker.marker_path) -Expected $ClusterMarkerPath) -or
    [string]$restart.marker_raw_sha256 -cne $ClusterMarkerRawSha256 -or
    [string]$restart.marker_native_identity -cne (Get-LatticeWindowsNativePathIdentityToken -Path $ClusterMarkerPath -Directory $false) -or
    [string]$initial.system_identifier -cne [string]$restart.system_identifier -or
    [string]$initial.postmaster_started_at -ceq [string]$restart.restart_postmaster_started_at -or
    -not [bool]$stopped.pg_ctl_status_stopped -or
    -not [bool]$stopped.port_listener_absent -or
    ([long]$initial.listener_process_id -eq [long]$restart.listener_process_id -and [string]$initial.listener_process_creation_time -ceq [string]$restart.listener_process_creation_time)
) { throw 'TASK038_POSTGRES_HOLDER_MARKER_REJECTED' }
if (
    [string]$consumer.consumer_session_id -cne $ConsumerSessionId -or
    [long]$consumer.holder_process_id -ne [long]$PID -or
    [long]$consumer.listener_process_id -ne [long]$restart.listener_process_id -or
    [string]$consumer.listener_process_creation_time -cne [string]$restart.listener_process_creation_time
) { throw 'TASK038_POSTGRES_HOLDER_CONSUMER_REJECTED' }
'@
    $Source = Replace-Task051Exact -Source $Source -Old '$currentOwnerExecutable = Get-CanonicalPath -Path ([string]$currentOwner.Path)' -New $holderDiagnosticBlock -FailureCode 'TASK051_TASK038_HOLDER_DIAGNOSTIC_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old @'
    $captureRoot = Get-CanonicalPath -Path (Split-Path -Parent $DataDirectory)
    Assert-NoReparseAncestor `
        -Path $captureRoot `
        -Boundary $script:RepositoryRoot `
        -FailureCode 'TASK038_POSTGRES_RESTART_ROOT_REJECTED'
    if (-not (Test-ExactPath -Actual $ServerLog -Expected (Join-Path $captureRoot 'postgres.log'))) {
'@ -New @'
    $captureRoot = Get-CanonicalPath -Path $script:Task051PhysicalPostgresRoot
    Assert-NoReparseAncestor `
        -Path $captureRoot `
        -Boundary $repositoryTarget `
        -FailureCode 'TASK038_POSTGRES_RESTART_ROOT_REJECTED'
    if (-not (Test-ExactPath -Actual $ServerLog -Expected (Join-Path $script:Task051PhysicalPostgresRoot 'postgres.log'))) {
'@ -FailureCode 'TASK051_TASK038_RESTART_ROOT_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old @'
    $canonicalOutputDirectory = Get-CanonicalPath -Path $OutputDirectory
    $outputDirectoryItem = Get-Item -LiteralPath $canonicalOutputDirectory -Force -ErrorAction SilentlyContinue
'@ -New @'
    $task051PhysicalOutputDirectory = Get-CanonicalPath -Path $OutputDirectory
    if (-not (Test-ExactPath -Actual $task051PhysicalOutputDirectory -Expected $script:Task051PhysicalPostgresRoot)) {
        throw 'TASK038_NATIVE_OUTPUT_DIRECTORY_REJECTED'
    }
    $canonicalOutputDirectory = Get-CanonicalPath -Path (Split-Path -Parent $script:PostgresData)
    if (
        (Get-LatticeWindowsNativePathIdentityToken -Path $canonicalOutputDirectory -Directory $true) -cne
        (Get-LatticeWindowsNativePathIdentityToken -Path $task051PhysicalOutputDirectory -Directory $true)
    ) {
        throw 'TASK038_NATIVE_OUTPUT_DIRECTORY_REJECTED'
    }
    $outputDirectoryItem = Get-Item -LiteralPath $canonicalOutputDirectory -Force -ErrorAction SilentlyContinue
'@ -FailureCode 'TASK051_TASK038_CAPTURE_ALIAS_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old @'
    Assert-NoReparseAncestor `
        -Path $canonicalOutputDirectory `
        -Boundary $script:RepositoryRoot `
        -FailureCode 'TASK038_NATIVE_OUTPUT_DIRECTORY_REJECTED'
'@ -New @'
    Assert-NoReparseAncestor `
        -Path $canonicalOutputDirectory `
        -Boundary $env:LATTICE_TASK051_RUN_ALIAS_ROOT `
        -FailureCode 'TASK038_NATIVE_OUTPUT_DIRECTORY_REJECTED'
'@ -FailureCode 'TASK051_TASK038_CAPTURE_BOUNDARY_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old @'
$script:PostgresContainmentSnapshot = New-LatticeWindowsNativeContainmentSnapshot `
    -ParentPath $repositoryTarget `
    -RootPath $clusterRoot `
'@ -New @'
$script:PostgresContainmentSnapshot = New-LatticeWindowsNativeContainmentSnapshot `
    -ParentPath $script:Task051PhysicalPostgresParent `
    -RootPath $clusterRoot `
'@ -FailureCode 'TASK051_TASK038_CONTAINMENT_PARENT_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old "schema_version = 'lattice.task038.local-canonical-mcp-acceptance.v1'" -New "schema_version = 'lattice.task051.task038-derived-acceptance.v1'" -FailureCode 'TASK051_TASK038_EVIDENCE_SCHEMA_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old "foreach (`$cargoVariable in @('CARGO_TARGET_DIR', 'CARGO_BUILD_TARGET')) {" -New @'
$expectedTask051CargoTarget = Get-CanonicalPath -Path $env:CARGO_TARGET_DIR
foreach ($cargoVariable in @('CARGO_TARGET_DIR', 'CARGO_BUILD_TARGET')) {
'@ -FailureCode 'TASK051_TASK038_CARGO_TARGET_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old "    if (-not [string]::IsNullOrEmpty([Environment]::GetEnvironmentVariable(`$cargoVariable, 'Process'))) {" -New @'
    $cargoValue = [Environment]::GetEnvironmentVariable($cargoVariable, 'Process')
    if (
        ($cargoVariable -ceq 'CARGO_TARGET_DIR' -and -not (Test-ExactPath -Actual $cargoValue -Expected $expectedTask051CargoTarget)) -or
        ($cargoVariable -ceq 'CARGO_BUILD_TARGET' -and -not [string]::IsNullOrEmpty($cargoValue))
    ) {
'@ -FailureCode 'TASK051_TASK038_CARGO_TARGET_CONDITION_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$canonicalRepository = Get-CanonicalPath -Path $RepositoryRoot' -New '$canonicalRepository = Get-CanonicalPath -Path (Join-Path $env:LATTICE_TASK051_RUN_ROOT ''__repository-boundary-sentinel'')' -FailureCode 'TASK051_TASK038_HOME_BOUNDARY_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old 'if (Test-PathOverlap -Left $script:CodexCredentialSource -Right $script:RepositoryRoot) {' -New 'if (-not (Test-ExactPath -Actual $script:CodexCredentialSource -Expected (Join-Path $env:LATTICE_TASK051_RUN_ROOT ''credential-source''))) {' -FailureCode 'TASK051_TASK038_CREDENTIAL_BOUNDARY_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$executionParent = Get-CanonicalPath -Path (Join-Path (Split-Path -Parent $source) ''task038-execution-homes'')' -New '$executionParent = Get-CanonicalPath -Path (Join-Path $env:LATTICE_TASK051_RUN_ROOT ''t38h'')' -FailureCode 'TASK051_TASK038_EXECUTION_HOME_SHORT_PATH_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old "    Assert-NoReparsePath -Path `$executionParent -FailureCode 'TASK038_CODEX_EXECUTION_PARENT_REJECTED'" -New "    Assert-NoReparseAncestor -Path `$executionParent -Boundary `$env:LATTICE_TASK051_RUN_ROOT -FailureCode 'TASK038_CODEX_EXECUTION_PARENT_REJECTED'" -FailureCode 'TASK051_TASK038_EXECUTION_HOME_BOUNDARY_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$canonicalRoot = Get-CanonicalPath -Path $Root' -New @'
    $canonicalRoot = Get-CanonicalPath -Path $Root
    $task051LongPathRoot = '\\?\' + $canonicalRoot
'@ -FailureCode 'TASK051_TASK038_LONG_PATH_FOOTPRINT_ROOT_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$rootItem = Get-Item -LiteralPath $canonicalRoot -Force -ErrorAction SilentlyContinue' -New '$rootItem = Get-Item -LiteralPath $task051LongPathRoot -Force -ErrorAction SilentlyContinue' -FailureCode 'TASK051_TASK038_LONG_PATH_FOOTPRINT_ITEM_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old 'Get-ChildItem -LiteralPath $canonicalRoot -Recurse -Force | ForEach-Object {' -New 'Get-ChildItem -LiteralPath $task051LongPathRoot -Recurse -Force | ForEach-Object {' -FailureCode 'TASK051_TASK038_LONG_PATH_FOOTPRINT_ENUMERATION_TRANSFORM_REJECTED'
    if ([regex]::Matches($Source, [regex]::Escape('Substring($canonicalRoot.Length)')).Count -ne 2) {
        throw 'TASK051_TASK038_LONG_PATH_FOOTPRINT_RELATIVE_TRANSFORM_REJECTED'
    }
    $Source = $Source.Replace('Substring($canonicalRoot.Length)', 'Substring($task051LongPathRoot.Length)')
    $Source = Replace-Task051Exact -Source $Source -Old '$executionHome = Get-CanonicalPath -Path $Path' -New @'
    $executionHome = Get-CanonicalPath -Path $Path
    $task051LongExecutionHome = '\\?\' + $executionHome
'@ -FailureCode 'TASK051_TASK038_LONG_PATH_CLEANUP_ROOT_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$authPath = Join-Path $executionHome ''auth.json''' -New '$authPath = $task051LongExecutionHome + ''\auth.json''' -FailureCode 'TASK051_TASK038_LONG_PATH_CLEANUP_AUTH_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$ownerPath = Join-Path $executionHome ''.lattice-task038-execution-owner-v1''' -New '$ownerPath = $task051LongExecutionHome + ''\.lattice-task038-execution-owner-v1''' -FailureCode 'TASK051_TASK038_LONG_PATH_CLEANUP_OWNER_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old @'
    $authPath = $task051LongExecutionHome + '\auth.json'
    if (Test-Path -LiteralPath $authPath) {
        [IO.File]::Delete($authPath)
    }
    if (Test-Path -LiteralPath $authPath) {
        throw 'TASK038_CODEX_EXECUTION_HOME_SECRET_CLEANUP_REJECTED'
    }
    $ownerPath = $task051LongExecutionHome + '\.lattice-task038-execution-owner-v1'
    Assert-RegularFile -Path $ownerPath -FailureCode 'TASK038_CODEX_EXECUTION_HOME_CLEANUP_REJECTED'
    if (
        [IO.File]::ReadAllText($ownerPath, [Text.Encoding]::UTF8) -cne
        ("lattice.task038-execution-home.v1:" + $AcceptanceId + "`n")
    ) {
        throw 'TASK038_CODEX_EXECUTION_HOME_CLEANUP_REJECTED'
    }
'@ -New @'
    $ownerPath = $task051LongExecutionHome + '\.lattice-task038-execution-owner-v1'
    Assert-RegularFile -Path $ownerPath -FailureCode 'TASK038_CODEX_EXECUTION_HOME_CLEANUP_REJECTED'
    if (
        [IO.File]::ReadAllText($ownerPath, [Text.Encoding]::UTF8) -cne
        ("lattice.task038-execution-home.v1:" + $AcceptanceId + "`n")
    ) {
        throw 'TASK038_CODEX_EXECUTION_HOME_CLEANUP_REJECTED'
    }
    $authPath = $task051LongExecutionHome + '\auth.json'
    if (Test-Path -LiteralPath $authPath) {
        [IO.File]::Delete($authPath)
    }
    if (Test-Path -LiteralPath $authPath) {
        throw 'TASK038_CODEX_EXECUTION_HOME_SECRET_CLEANUP_REJECTED'
    }
'@ -FailureCode 'TASK051_TASK038_LONG_PATH_CLEANUP_OWNER_ORDER_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$items = @(Get-ChildItem -LiteralPath $executionHome -Recurse -Force)' -New '$items = @(Get-ChildItem -LiteralPath $task051LongExecutionHome -Recurse -Force)' -FailureCode 'TASK051_TASK038_LONG_PATH_CLEANUP_ENUMERATION_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old 'Remove-Item -LiteralPath $executionHome -Recurse -Force' -New '[IO.Directory]::Delete($task051LongExecutionHome, $true)' -FailureCode 'TASK051_TASK038_LONG_PATH_CLEANUP_DELETE_TRANSFORM_REJECTED'
    $oldWriterCall = 'Invoke-WriterLeaseLiveSuite -Identity $identity -Authority $authority -DatabaseName $databaseName -MigratorDsn $migratorDsn -RuntimeDsn $runtimeDsn -AdminDsn $adminDsn -EvidencePath (Join-Path $evidenceRoot ''writer-lease-live.json'')'
    $newWriterCall = 'Write-JsonEvidence -Path (Join-Path $evidenceRoot ''writer-lease-live.json'') -Value ([ordered]@{ schema_version = ''lattice.task051.writer-v2-delegation.v1''; status = ''DELEGATED_TO_TASK076_CURRENT_GATE'' })'
    $Source = Replace-Task051Exact -Source $Source -Old $oldWriterCall -New $newWriterCall -FailureCode 'TASK051_TASK038_WRITER_TRANSFORM_REJECTED'
    $Source = $Source.Replace('$Footprint.sequence -ne 12 -or $Footprint.event_count -ne 12 -or $Footprint.command_count -ne 12', '$Footprint.sequence -ne 13 -or $Footprint.event_count -ne 13 -or $Footprint.command_count -ne 13')
    $Source = $Source.Replace('$Footprint.created_action_id -ne ''CONTROLLED_CODEX_CANARY''', '$Footprint.created_action_id -ne ''CONTROLLED_CODEX_CANARY_AUTONOMY_V1''')
    $Source = Replace-Task051Exact -Source $Source -Old @'
  COALESCE((SELECT count(*) FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'), 0)::text AS task_created,
'@ -New @'
  COALESCE((SELECT count(*) FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'), 0)::text AS task_created,
  COALESCE((SELECT count(*) FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='AUTONOMY_RECEIPT_RECORDED'), 0)::text AS autonomy_events,
  COALESCE((SELECT count(*) FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), 0)::text AS autonomy_rows,
  COALESCE((SELECT a.event_sequence::text FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_sequence,
  COALESCE((SELECT a.receipt_schema_version FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_schema,
  COALESCE((SELECT a.intent_version FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_intent,
  COALESCE((SELECT a.task_kind FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_task_kind,
  COALESCE((SELECT a.execution_preapproved::text FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_execution_preapproved,
  COALESCE((SELECT a.requires_new_authority::text FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_requires_new_authority,
  COALESCE((SELECT a.irreversible_or_high_risk::text FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_irreversible_or_high_risk,
  COALESCE((SELECT a.observed_task_state FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_observed_state,
  COALESCE((SELECT a.model FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_model,
  COALESCE((SELECT a.verification FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_verification,
  COALESCE((SELECT a.risk_class FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_risk,
  COALESCE((SELECT a.disposition FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_disposition,
  COALESCE((SELECT a.decision_reason FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_reason,
  COALESCE((SELECT a.authority_mode FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_authority_mode,
  COALESCE((SELECT encode(a.event_digest, 'hex') FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_event_digest,
  COALESCE((SELECT encode(e.event_digest, 'hex') FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.sequence=2), '') AS autonomy_sequence2_event_digest,
  COALESCE((SELECT encode(a.process_start_authority_digest, 'hex') FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_process_start_digest,
  COALESCE((SELECT encode(a.ingress_profile_adapter_commitment, 'hex') FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_ingress_digest,
  COALESCE((SELECT encode(a.store_authority_head_digest, 'hex') FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_store_head_digest,
  COALESCE((SELECT encode(a.authority_digest, 'hex') FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_authority_digest,
  COALESCE((SELECT encode(a.receipt_digest, 'hex') FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_receipt_digest,
  COALESCE((SELECT encode(a.writer_lease_receipt_digest, 'hex') FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_writer_receipt_digest,
  COALESCE((SELECT encode(a.writer_lease_head_digest, 'hex') FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_writer_head_digest,
  COALESCE((SELECT a.writer_fencing_token::text FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id), '') AS autonomy_writer_fence,
'@ -FailureCode 'TASK051_TASK038_AUTONOMY_QUERY_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '        task_created = [int]$row.task_created' -New @'
        task_created = [int]$row.task_created
        autonomy_events = [int]$row.autonomy_events
        autonomy_rows = [int]$row.autonomy_rows
        autonomy_sequence = [int]$row.autonomy_sequence
        autonomy_schema = [string]$row.autonomy_schema
        autonomy_intent = [string]$row.autonomy_intent
        autonomy_task_kind = [string]$row.autonomy_task_kind
        autonomy_execution_preapproved = [string]$row.autonomy_execution_preapproved
        autonomy_requires_new_authority = [string]$row.autonomy_requires_new_authority
        autonomy_irreversible_or_high_risk = [string]$row.autonomy_irreversible_or_high_risk
        autonomy_observed_state = [string]$row.autonomy_observed_state
        autonomy_model = [string]$row.autonomy_model
        autonomy_verification = [string]$row.autonomy_verification
        autonomy_risk = [string]$row.autonomy_risk
        autonomy_disposition = [string]$row.autonomy_disposition
        autonomy_reason = [string]$row.autonomy_reason
        autonomy_authority_mode = [string]$row.autonomy_authority_mode
        autonomy_event_digest = [string]$row.autonomy_event_digest
        autonomy_sequence2_event_digest = [string]$row.autonomy_sequence2_event_digest
        autonomy_process_start_digest = [string]$row.autonomy_process_start_digest
        autonomy_ingress_digest = [string]$row.autonomy_ingress_digest
        autonomy_store_head_digest = [string]$row.autonomy_store_head_digest
        autonomy_authority_digest = [string]$row.autonomy_authority_digest
        autonomy_receipt_digest = [string]$row.autonomy_receipt_digest
        autonomy_writer_receipt_digest = [string]$row.autonomy_writer_receipt_digest
        autonomy_writer_head_digest = [string]$row.autonomy_writer_head_digest
        autonomy_writer_fence = [long]$row.autonomy_writer_fence
        writer_receipt_chain = [string]$row.writer_receipt_chain
'@ -FailureCode 'TASK051_TASK038_AUTONOMY_MAP_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$Footprint.task_created -ne 1 -or $Footprint.state_transitions -ne 8 -or' -New @'
$Footprint.task_created -ne 1 -or
        $Footprint.autonomy_events -ne 1 -or $Footprint.autonomy_rows -ne 1 -or
        $Footprint.autonomy_sequence -ne 2 -or
        $Footprint.autonomy_schema -cne 'lattice.autonomy-receipt/1.0' -or
        $Footprint.autonomy_intent -cne '1.0' -or $Footprint.autonomy_risk -cne 'R0' -or
        $Footprint.autonomy_task_kind -cne 'FEATURE' -or
        $Footprint.autonomy_execution_preapproved -cne 'true' -or
        $Footprint.autonomy_requires_new_authority -cne 'false' -or
        $Footprint.autonomy_irreversible_or_high_risk -cne 'false' -or
        $Footprint.autonomy_observed_state -cne 'DRAFT' -or
        $Footprint.autonomy_disposition -cne 'PROCEED' -or
        $Footprint.autonomy_reason -cne 'ROUTINE_AUTHORIZED' -or
        $Footprint.autonomy_model -cne 'GOVERNED_CODEX_WRITER' -or
        $Footprint.autonomy_verification -cne 'FOCUSED_CHECKS' -or
        $Footprint.autonomy_authority_mode -cne 'P0_PROCESS_START_PROFILE_V1' -or
        $Footprint.autonomy_event_digest -cnotmatch '^[0-9a-f]{64}$' -or
        $Footprint.autonomy_sequence2_event_digest -cne $Footprint.autonomy_event_digest -or
        $Footprint.autonomy_process_start_digest -cnotmatch '^[0-9a-f]{64}$' -or
        $Footprint.autonomy_ingress_digest -cnotmatch '^[0-9a-f]{64}$' -or
        $Footprint.autonomy_store_head_digest -cnotmatch '^[0-9a-f]{64}$' -or
        $Footprint.autonomy_authority_digest -cnotmatch '^[0-9a-f]{64}$' -or
        $Footprint.autonomy_receipt_digest -cnotmatch '^[0-9a-f]{64}$' -or
        $Footprint.autonomy_writer_receipt_digest -cnotmatch '^[0-9a-f]{64}$' -or
        $Footprint.autonomy_writer_head_digest -cnotmatch '^[0-9a-f]{64}$' -or
        $Footprint.autonomy_writer_fence -lt 1 -or
        $Footprint.state_transitions -ne 8 -or
'@ -FailureCode 'TASK051_TASK038_AUTONOMY_ASSERT_TRANSFORM_REJECTED'
    $Source = $Source.Replace('[long]$submitSession.ObservedEffectEvidence.session_counters.filesystem -lt 1', '[long]$submitSession.ObservedEffectEvidence.session_counters.filesystem -ne 0')
    $Source = $Source.Replace('[long]$submitSession.ObservedEffectEvidence.session_counters.process -lt 1', '[long]$submitSession.ObservedEffectEvidence.session_counters.process -ne 0')
    $Source = $Source.Replace('[long]$submitSession.ObservedEffectEvidence.session_counters.codex -lt 1', '[long]$submitSession.ObservedEffectEvidence.session_counters.codex -ne 0')
    $Source = Replace-Task051Exact -Source $Source -Old "Assert-CompletedDatabaseFootprint -Footprint `$databaseAfterSubmit -PublicStatus `$submitted -ExpectedCommandId ('mcp-submit:' + `$sameClientRequestId) -Baseline `$before" -New @'
if (
    $databaseAfterSubmit.autonomy_process_start_digest -cne $databaseAfterSubmit.created_process_start_authority_digest -or
    $databaseAfterSubmit.autonomy_ingress_digest -cne $databaseAfterSubmit.created_profile_adapter_commitment -or
    $databaseAfterSubmit.autonomy_store_head_digest -cne [string]$authority.head_digest -or
    $databaseAfterSubmit.autonomy_writer_fence -ne $databaseAfterSubmit.writer_fencing_high_water -or
    @($databaseAfterSubmit.writer_receipt_chain -split ':' | Where-Object { $_ -ceq $databaseAfterSubmit.autonomy_writer_receipt_digest }).Count -ne 1
) {
    throw 'TASK051_AUTONOMY_RECEIPT_LINKAGE_REJECTED'
}
Assert-CompletedDatabaseFootprint -Footprint $databaseAfterSubmit -PublicStatus $submitted -ExpectedCommandId ('mcp-submit:' + $sameClientRequestId) -Baseline $before
'@ -FailureCode 'TASK051_AUTONOMY_LINKAGE_ASSERT_TRANSFORM_REJECTED'
    $beforeLegacy = @'
$legacyFrames = @(
'@
    $currentSubmit = @'
$task051DiscoverySessionId = [Guid]::NewGuid().ToString('N')
$task051DiscoverySink = New-Task038McpAcceptanceEvidenceSink -EvidenceRoot $evidenceRoot -SessionId $task051DiscoverySessionId
$task051DiscoveryObserved = New-Task038McpObservedEffectEvidenceSink -AcceptanceEvidencePath ([string]$task051DiscoverySink.path) -SessionId $task051DiscoverySessionId
$task051DiscoverySafeConfig = Get-Task051StringSha256 -Value ('TASK051_DISCOVERY|' + $acceptanceId + '|' + $candidateLatticedSha256 + '|' + $candidateLatticedNativeIdentity)
$task051DiscoveryEnvironment = Get-Task051McpEnvironment -RunMode 'FRESH' -Authority $authority -DatabasePassword $databasePassword -DeliveryRoot $deliveryRoot -SchemaDirectory $schemaDirectory -LauncherSha256 $launcherSha256 -LauncherVersion $codexVersion.Text.Trim() -AcceptanceEvidencePath ([string]$task051DiscoverySink.path) -AcceptanceSessionId $task051DiscoverySessionId -SafeConfigSha256 $task051DiscoverySafeConfig -ObservedEffectPath ([string]$task051DiscoveryObserved.path) -ObservedEffectNonce ([string]$task051DiscoveryObserved.nonce)
$task051Discovery = Invoke-Task051CodexDiscovery -Phase 'discovery' -EvidenceRoot $evidenceRoot -Environment $task051DiscoveryEnvironment -AcceptanceEvidencePath ([string]$task051DiscoverySink.path) -AcceptanceNativeIdentity ([string]$task051DiscoverySink.native_identity) -AcceptanceSessionId $task051DiscoverySessionId -SafeConfigSha256 $task051DiscoverySafeConfig -ExpectedLatticedSha256 $candidateLatticedSha256 -ExpectedLatticedNativeIdentity $candidateLatticedNativeIdentity
$task051SubmitArguments = [ordered]@{ client_request_id = $sameClientRequestId; intent = 'CONTROLLED_CODEX_CANARY' }
$task051Submit = Invoke-Task051CodexTool -Phase 'submit' -Tool 'lattice_task_submit' -Arguments $task051SubmitArguments -RunMode 'FRESH' -EvidenceRoot $evidenceRoot -Authority $authority -DatabasePassword $databasePassword -DeliveryRoot $deliveryRoot -SchemaDirectory $schemaDirectory -LauncherSha256 $launcherSha256 -LauncherVersion $codexVersion.Text.Trim()
$task051Submitted = $task051Submit.StructuredContent
$legacyFrames = @(
'@
    $Source = Replace-Task051Exact -Source $Source -Old $beforeLegacy -New $currentSubmit -FailureCode 'TASK051_TASK038_SUBMIT_INSERT_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old 'Assert-SamePublicTaskStatus -Expected $submitted -Actual $retried' -New @'
Assert-SamePublicTaskStatus -Expected $submitted -Actual $retried
Assert-SamePublicTaskStatus -Expected $task051Submitted -Actual $submitted
'@ -FailureCode 'TASK051_TASK038_SUBMIT_COMPARE_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '    $postgresBeforeRestart = Get-PostgresProcessEvidence -Password $databasePassword -DatabaseName $databaseName' -New @'
$task051PreStatusArguments = [ordered]@{ task_ref = [string]$submitted.task_ref }
$task051PreStatus = Invoke-Task051CodexTool -Phase 'status-pre-restart' -Tool 'lattice_task_status' -Arguments $task051PreStatusArguments -RunMode 'RESUME_EXISTING' -EvidenceRoot $evidenceRoot -Authority $authority -DatabasePassword $databasePassword -DeliveryRoot $deliveryRoot -SchemaDirectory $schemaDirectory -LauncherSha256 $launcherSha256 -LauncherVersion $codexVersion.Text.Trim()
Assert-SamePublicTaskStatus -Expected $submitted -Actual $task051PreStatus.StructuredContent

    $postgresBeforeRestart = Get-PostgresProcessEvidence -Password $databasePassword -DatabaseName $databaseName
'@ -FailureCode 'TASK051_TASK038_PRE_STATUS_INSERT_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$modernMeta = [ordered]@{' -New @'
$task051PostStatusArguments = [ordered]@{ task_ref = [string]$submitted.task_ref }
$task051PostStatus = Invoke-Task051CodexTool -Phase 'status-post-restart' -Tool 'lattice_task_status' -Arguments $task051PostStatusArguments -RunMode 'RESUME_EXISTING' -EvidenceRoot $evidenceRoot -Authority $authority -DatabasePassword $databasePassword -DeliveryRoot $deliveryRoot -SchemaDirectory $schemaDirectory -LauncherSha256 $launcherSha256 -LauncherVersion $codexVersion.Text.Trim()
Assert-SamePublicTaskStatus -Expected $submitted -Actual $task051PostStatus.StructuredContent
Assert-Task051DistinctProcessIds -ProcessIds @(
    [int]$task051Discovery.ProcessId,
    [int]$task051Submit.ProcessId,
    [int]$task051PreStatus.ProcessId,
    [int]$task051PostStatus.ProcessId
)
$task051ServerProcessIds = @(
    [int]$task051Discovery.ServerProcessId
    [int]$task051Submit.ServerProcessId
    [int]$task051PreStatus.ServerProcessId
    [int]$task051PostStatus.ServerProcessId
)
if (@($task051ServerProcessIds | Sort-Object -Unique).Count -ne 4) {
    throw 'TASK051_LATTICED_FRESH_PROCESS_REJECTED'
}

$modernMeta = [ordered]@{
'@ -FailureCode 'TASK051_TASK038_POST_STATUS_INSERT_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old 'Assert-SamePublicTaskStatus -Expected $submitted -Actual $status' -New @'
Assert-SamePublicTaskStatus -Expected $submitted -Actual $status
Assert-SamePublicTaskStatus -Expected $task051PostStatus.StructuredContent -Actual $status
'@ -FailureCode 'TASK051_TASK038_POST_STATUS_COMPARE_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '    writer_lease_live_suite_passed_without_skip = $true' -New @'
    writer_v2_gate_delegated_to_task076 = $true
    task051_current_codex_sha256 = [string]$task051Submit.CodexSha256
    task051_discovery_process_id = [int]$task051Discovery.ProcessId
    task051_discovery_server_process_id = [int]$task051Discovery.ServerProcessId
    task051_submit_process_id = [int]$task051Submit.ProcessId
    task051_pre_restart_status_process_id = [int]$task051PreStatus.ProcessId
    task051_post_restart_status_process_id = [int]$task051PostStatus.ProcessId
    task051_submit_server_process_id = [int]$task051Submit.ServerProcessId
    task051_pre_restart_server_process_id = [int]$task051PreStatus.ServerProcessId
    task051_post_restart_server_process_id = [int]$task051PostStatus.ServerProcessId
    task051_submit_dispatch_raw_sha256 = [string]$task051Submit.DispatchRawSha256
    task051_pre_restart_dispatch_raw_sha256 = [string]$task051PreStatus.DispatchRawSha256
    task051_post_restart_dispatch_raw_sha256 = [string]$task051PostStatus.DispatchRawSha256
    task051_discovery_evidence_path = [string]$task051Discovery.EvidencePath
    task051_discovery_evidence_sha256 = [string]$task051Discovery.EvidenceSha256
    task051_submit_evidence_path = [string]$task051Submit.EvidencePath
    task051_submit_evidence_sha256 = [string]$task051Submit.EvidenceSha256
    task051_pre_restart_evidence_path = [string]$task051PreStatus.EvidencePath
    task051_pre_restart_evidence_sha256 = [string]$task051PreStatus.EvidenceSha256
    task051_post_restart_evidence_path = [string]$task051PostStatus.EvidencePath
    task051_post_restart_evidence_sha256 = [string]$task051PostStatus.EvidenceSha256
    task051_autonomy_receipt_verified = $true
'@ -FailureCode 'TASK051_TASK038_FINAL_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$fixtureParent = Get-CanonicalPath -Path (Join-Path $repositoryTarget ''lattice-delivery'')' -New '$fixtureParent = Get-CanonicalPath -Path $repositoryTarget' -FailureCode 'TASK051_TASK038_COMPACT_FIXTURE_PARENT_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$fixtureRoot = Get-CanonicalPath -Path (Join-Path $fixtureParent $acceptanceId)' -New '$fixtureRoot = Get-CanonicalPath -Path (Join-Path $fixtureParent ''d'')' -FailureCode 'TASK051_TASK038_COMPACT_FIXTURE_ROOT_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$evidenceRoot = Join-Path $fixtureRoot ''evidence''' -New '$evidenceRoot = Join-Path $fixtureRoot ''e''' -FailureCode 'TASK051_TASK038_COMPACT_EVIDENCE_ROOT_TRANSFORM_REJECTED'
    if (
        $Source.IndexOf('CONTROLLED_CODEX_CANARY_AUTONOMY_V1', [StringComparison]::Ordinal) -lt 0 -or
        $Source.IndexOf('TASK051_AUTONOMY_RECEIPT_REJECTED', [StringComparison]::Ordinal) -ge 0
    ) {
        throw 'TASK051_AUTONOMY_RECEIPT_REJECTED'
    }
    return $Source
}

function Convert-Task051Task019Source {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$ScriptsRoot
    )

    $qScripts = ConvertTo-Task051TomlLiteral -Value ([IO.Path]::GetFullPath($ScriptsRoot))
    $Source = $Source.Replace('$PSScriptRoot', $qScripts)
    $Source = Replace-Task051Exact -Source $Source -Old '$repositoryTarget = Get-CanonicalPath -Path (Join-Path $repositoryRoot ''target'')' -New '$repositoryTarget = Get-CanonicalPath -Path $env:LATTICE_TASK051_RUN_ROOT' -FailureCode 'TASK051_TASK019_TARGET_TRANSFORM_REJECTED'
    $emptyDiagnosticParameter = @'
function Get-Task019AllowlistedDiagnosticTokens {
    param([Parameter(Mandatory = $true)][object[]]$Output)
'@
    $allowEmptyDiagnosticParameter = @'
function Get-Task019AllowlistedDiagnosticTokens {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Output)
'@
    $Source = Replace-Task051Exact -Source $Source -Old $emptyDiagnosticParameter -New $allowEmptyDiagnosticParameter -FailureCode 'TASK051_TASK019_EMPTY_DIAGNOSTIC_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '$dataDirectory = Join-Path $clusterRoot ''data''' -New @'
$dataDirectory = Join-Path $env:LATTICE_TASK051_RUN_ALIAS_ROOT ('task019-postgres\' + $runId + '\data')
$task051CargoOutputRoot = Join-Path $env:LATTICE_TASK051_RUN_ALIAS_ROOT ('task019-postgres\' + $runId)
'@ -FailureCode 'TASK051_TASK019_PGDATA_ALIAS_TRANSFORM_REJECTED'
    $doubleQuotedCargoOutput = 'Join-Path $clusterRoot ".cargo'
    $singleQuotedCargoOutput = "Join-Path `$clusterRoot '.cargo"
    if (
        [regex]::Matches($Source, [regex]::Escape($doubleQuotedCargoOutput)).Count -ne 12 -or
        [regex]::Matches($Source, [regex]::Escape($singleQuotedCargoOutput)).Count -ne 2
    ) {
        throw 'TASK051_TASK019_CARGO_OUTPUT_ALIAS_TRANSFORM_REJECTED'
    }
    $Source = $Source.Replace($doubleQuotedCargoOutput, 'Join-Path $task051CargoOutputRoot ".cargo')
    $Source = $Source.Replace($singleQuotedCargoOutput, "Join-Path `$task051CargoOutputRoot '.cargo")
    $writerOwnerCleanup = @'
        foreach ($path in @($stdoutPath, $stderrPath)) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        }
        $null = Invoke-HarnessPsqlRows -Psql $Psql -DatabaseName $databaseName `
            -Port $Port -Password $Password -Query $stopQuery `
            -FailureCode 'TASK019_WRITER_LEASE_OWNER_STOP_REJECTED'
'@
    $writerOwnerVerifiedCleanup = @'
        $task051WriterOwnerOutputCleanupFailed = $false
        foreach ($path in @($stdoutPath, $stderrPath)) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $path) {
                $task051WriterOwnerOutputCleanupFailed = $true
            }
        }
        $null = Invoke-HarnessPsqlRows -Psql $Psql -DatabaseName $databaseName `
            -Port $Port -Password $Password -Query $stopQuery `
            -FailureCode 'TASK019_WRITER_LEASE_OWNER_STOP_REJECTED'
        if ($task051WriterOwnerOutputCleanupFailed) {
            throw 'TASK051_WRITER_OWNER_OUTPUT_DELETE_FAILED'
        }
'@
    $Source = Replace-Task051Exact -Source $Source -Old $writerOwnerCleanup -New $writerOwnerVerifiedCleanup -FailureCode 'TASK051_WRITER_OWNER_OUTPUT_CLEANUP_TRANSFORM_REJECTED'
    $catalogCleanup = @'
        foreach ($path in @($stdoutPath, $stderrPath)) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        }
    }
    if ($exitCode -ne 0) {
'@
    $catalogVerifiedCleanup = @'
        foreach ($path in @($stdoutPath, $stderrPath)) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $path) {
                throw 'TASK051_CATALOG_OUTPUT_DELETE_FAILED'
            }
        }
    }
    if ($exitCode -ne 0) {
'@
    $Source = Replace-Task051Exact -Source $Source -Old $catalogCleanup -New $catalogVerifiedCleanup -FailureCode 'TASK051_CATALOG_OUTPUT_CLEANUP_TRANSFORM_REJECTED'
    $Source = Replace-Task051Exact -Source $Source -Old '            Remove-Item -LiteralPath $clusterRoot -Recurse -Force' -New '            Remove-Task051OwnedDirectory -Path $clusterRoot -AllowedRoot $repositoryTarget -FailureCode ''TASK051_TASK019_CLUSTER_CLEANUP_REJECTED''' -FailureCode 'TASK051_TASK019_CLUSTER_CLEANUP_TRANSFORM_REJECTED'
    $Source = $Source.Replace('$task038HookPath = Get-LatticeTask038AcceptanceHookPath -ScriptDirectory ' + $qScripts + ' -RepositoryRoot $repositoryRoot', '$task038HookPath = [IO.Path]::GetFullPath($env:LATTICE_TASK051_GENERATED_TASK038)')
    $conflictStart = $Source.IndexOf('$RunTask076WriterLeaseGate -and (', [StringComparison]::Ordinal)
    $conflictEndMarker = "throw 'TASK076_WRITER_LEASE_GATE_HOOK_FORBIDDEN'"
    if ($conflictStart -lt 0) { throw 'TASK051_TASK019_COMPAT_TRANSFORM_REJECTED' }
    $conflictEnd = $Source.IndexOf($conflictEndMarker, $conflictStart, [StringComparison]::Ordinal)
    if ($conflictEnd -lt 0) {
        throw 'TASK051_TASK019_COMPAT_TRANSFORM_REJECTED'
    }
    $conflictLength = ($conflictEnd + $conflictEndMarker.Length) - $conflictStart
    $conflictSlice = $Source.Substring($conflictStart, $conflictLength)
    $conflictLine = '        $RunTask038AcceptanceHook -or $RunTask038TunnelHook -or'
    if ($conflictSlice.IndexOf($conflictLine, [StringComparison]::Ordinal) -lt 0) {
        throw 'TASK051_TASK019_COMPAT_TRANSFORM_REJECTED'
    }
    $conflictSlice = $conflictSlice.Replace($conflictLine, '        $RunTask038TunnelHook -or')
    $Source = $Source.Substring(0, $conflictStart) + $conflictSlice + $Source.Substring($conflictStart + $conflictLength)
    $Source = $Source.Replace('''V5'', ''V5_MEMORY_V3'', ''V3_MEMORY_V2'', ''V3_MEMORY_V2_WRITER_LEASE_V1''', '''V5'', ''V5_MEMORY_V3'', ''V3_MEMORY_V2'', ''V3_MEMORY_V2_WRITER_LEASE_V1'', ''V5_MEMORY_V3_WRITER_LEASE_V2''')
    $acceptanceProfile = @'
    elseif ($RunTask038AcceptanceHook) {
'@
    $profileIndex = $Source.IndexOf($acceptanceProfile, [StringComparison]::Ordinal)
    if ($profileIndex -lt 0) { throw 'TASK051_TASK019_PROFILE_TRANSFORM_REJECTED' }
    $tail = $Source.Substring($profileIndex)
    $oldProfile = "-ExpectedProfile 'V3_MEMORY_V2_WRITER_LEASE_V1'"
    if (($tail.IndexOf($oldProfile, [StringComparison]::Ordinal)) -lt 0) {
        throw 'TASK051_TASK019_PROFILE_TRANSFORM_REJECTED'
    }
    $tail = $tail.Remove($tail.IndexOf($oldProfile, [StringComparison]::Ordinal), $oldProfile.Length).Insert($tail.IndexOf($oldProfile, [StringComparison]::Ordinal), "-ExpectedProfile 'V5_MEMORY_V3_WRITER_LEASE_V2'")
    $Source = $Source.Substring(0, $profileIndex) + $tail
    if (
        $Source.IndexOf('V5_MEMORY_V3_WRITER_LEASE_V2', [StringComparison]::Ordinal) -lt 0 -or
        $Source.IndexOf('$env:LATTICE_TASK051_GENERATED_TASK038', [StringComparison]::Ordinal) -lt 0
    ) {
        throw 'TASK051_TASK019_SOURCE_TRANSFORM_REJECTED'
    }
    return $Source
}

function Invoke-Task051SelfTest {
    if (
        -not (Test-Task051PostgresProcessSnapshotClosed -Baseline @('100|1000') -Current @('100|1000')) -or
        (Test-Task051PostgresProcessSnapshotClosed -Baseline @('100|1000') -Current @('100|1000', '101|1001')) -or
        (Test-Task051PostgresProcessSnapshotClosed -Baseline @('100|1000') -Current @('invalid'))
    ) {
        throw 'TASK051_POSTGRES_PROCESS_SNAPSHOT_SELF_TEST_REJECTED'
    }
    $status = [pscustomobject]@{
        schema_version = $script:Task051PublicStatusSchema
        status = 'COMPLETED'
        task_state = 'COMPLETED'
        task_ref = '1' * 64
        ledger_head_digest = '2' * 64
        result_digest = '3' * 64
    }
    Assert-Task051PublicStatus -Value $status -Kind 'SUBMIT'
    Assert-Task051SameStatus -Expected $status -Actual $status
    Assert-Task051DistinctProcessIds -ProcessIds @(101, 102, 103, 104)
    $optionalOutputRecords = @(Convert-Task051AppServerTools -Tools ([pscustomobject]@{
        delivery = [pscustomobject]@{ inputSchema = [pscustomobject]@{ type = 'object' } }
        task = [pscustomobject]@{
            inputSchema = [pscustomobject]@{ type = 'object' }
            outputSchema = [pscustomobject]@{ type = 'object' }
        }
    }))
    if (
        $optionalOutputRecords.Count -ne 2 -or
        $null -ne $optionalOutputRecords[0].PSObject.Properties['outputSchema'] -or
        $null -eq $optionalOutputRecords[1].PSObject.Properties['outputSchema']
    ) {
        throw 'TASK051_APP_SERVER_OPTIONAL_OUTPUT_SCHEMA_SELF_TEST_REJECTED'
    }
    $selectedServer = Get-Task051UniqueMcpServer -Servers @(
        [pscustomobject]@{ name = 'codex_apps'; tools = [pscustomobject]@{} },
        [pscustomobject]@{ name = 'lattice'; tools = [pscustomobject]@{} }
    ) -Name 'lattice'
    if ([string]$selectedServer.name -cne 'lattice') {
        throw 'TASK051_APP_SERVER_LATTICE_SERVER_SELECTION_SELF_TEST_REJECTED'
    }
    $emptyToolNames = @(Get-Task051McpToolNames -Tools ([pscustomobject]@{}))
    $mappedToolNames = @(Get-Task051McpToolNames -Tools ([pscustomobject]@{
        lattice_task_status = [pscustomobject]@{}
        lattice_task_submit = [pscustomobject]@{}
    }))
    if (
        $emptyToolNames.Count -ne 0 -or
        ($mappedToolNames -join ',') -cne 'lattice_task_status,lattice_task_submit'
    ) {
        throw 'TASK051_APP_SERVER_TOOL_MAP_SELF_TEST_REJECTED'
    }
    foreach ($invalidTools in @($null, @())) {
        $rejected = $false
        try { [void](Get-Task051McpToolNames -Tools $invalidTools) }
        catch { $rejected = [string]$_.Exception.Message -ceq 'TASK038_CURRENT_CODEX_DISCOVERY_TOOL_SHAPE_REJECTED' }
        if (-not $rejected) { throw 'TASK051_APP_SERVER_TOOL_MAP_SELF_TEST_REJECTED' }
    }
    foreach ($invalidServers in @(
        @([pscustomobject]@{ name = 'codex_apps' }),
        @([pscustomobject]@{ name = 'lattice' }, [pscustomobject]@{ name = 'lattice' })
    )) {
        $rejected = $false
        try { [void](Get-Task051UniqueMcpServer -Servers $invalidServers -Name 'lattice') }
        catch {
            $rejected = [string]$_.Exception.Message -in @(
                'TASK038_CURRENT_CODEX_DISCOVERY_LATTICE_SERVER_ZERO_REJECTED',
                'TASK038_CURRENT_CODEX_DISCOVERY_LATTICE_SERVER_DUPLICATE_REJECTED'
            )
        }
        if (-not $rejected) {
            throw 'TASK051_APP_SERVER_LATTICE_SERVER_SELECTION_SELF_TEST_REJECTED'
        }
    }
    $events = @(
        [pscustomobject]@{
            type = 'item.completed'
            item = [pscustomobject]@{
                type = 'mcp_tool_call'
                server = 'lattice'
                tool = 'lattice_task_status'
                status = 'completed'
                arguments = [pscustomobject]@{ task_ref = '1' * 64 }
                error = $null
                result = [pscustomobject]@{
                    content = @([pscustomobject]@{
                        type = 'text'
                        text = ($status | ConvertTo-Json -Compress -Depth 10)
                    })
                    _meta = [pscustomobject]@{
                        'io.modelcontextprotocol/serverInfo' = [pscustomobject]@{
                            name = 'latticed'
                            title = 'LATTICE DevOS'
                            version = '1.0.0'
                        }
                    }
                    structured_content = $status
                }
            }
        }
    )
    $parsed = Get-Task051ExecStructuredContent -Events $events -Tool 'lattice_task_status' -ExpectedArguments ([ordered]@{ task_ref = '1' * 64 })
    Assert-Task051SameStatus -Expected $status -Actual $parsed.StructuredContent
    try {
        Assert-Task051DistinctProcessIds -ProcessIds @(101, 102, 102, 104)
        throw 'TASK051_SELF_TEST_FALSE_PASS'
    }
    catch {
        if ([string]$_.Exception.Message -cne 'TASK051_CODEX_FRESH_PROCESS_REJECTED') { throw }
    }
    try {
        $duplicateEvents = @($events[0], $events[0])
        $null = Get-Task051ExecStructuredContent -Events $duplicateEvents -Tool 'lattice_task_status' -ExpectedArguments ([ordered]@{ task_ref = '1' * 64 })
        throw 'TASK051_SELF_TEST_FALSE_PASS'
    }
    catch {
        if ([string]$_.Exception.Message -cne 'TASK051_CODEX_STATUS_CALL_COUNT_REJECTED') { throw }
    }
    try {
        $collabEvents = @($events[0], [pscustomobject]@{
            type = 'item.completed'
            item = [pscustomobject]@{ type = 'collab_tool_call' }
        })
        $null = Get-Task051ExecStructuredContent -Events $collabEvents -Tool 'lattice_task_status' -ExpectedArguments ([ordered]@{ task_ref = '1' * 64 })
        throw 'TASK051_SELF_TEST_FALSE_PASS'
    }
    catch {
        if ([string]$_.Exception.Message -cne 'TASK051_CODEX_UNEXPECTED_TOOL_REJECTED') { throw }
    }
    try {
        $extraEnvelopeEvents = @(($events | ConvertTo-Json -Depth 20) | ConvertFrom-Json -ErrorAction Stop)
        $extraEnvelopeEvents[0].item.result | Add-Member -NotePropertyName isError -NotePropertyValue $false
        $null = Get-Task051ExecStructuredContent -Events $extraEnvelopeEvents -Tool 'lattice_task_status' -ExpectedArguments ([ordered]@{ task_ref = '1' * 64 })
        throw 'TASK051_SELF_TEST_FALSE_PASS'
    }
    catch {
        if ([string]$_.Exception.Message -cne 'TASK051_CODEX_TOOL_RESULT_ENVELOPE_REJECTED') { throw }
    }
    try {
        $wrongSchema = $status.PSObject.Copy()
        $wrongSchema.schema_version = 'lattice.task.status.v2'
        Assert-Task051PublicStatus -Value $wrongSchema -Kind 'STATUS'
        throw 'TASK051_SELF_TEST_FALSE_PASS'
    }
    catch {
        if ([string]$_.Exception.Message -cne 'TASK051_STATUS_SEMANTICS_REJECTED') { throw }
    }
    try {
        $extraStatus = [pscustomobject]@{
            schema_version = $script:Task051PublicStatusSchema
            status = 'COMPLETED'
            task_state = 'COMPLETED'
            task_ref = '1' * 64
            ledger_head_digest = '2' * 64
            result_digest = '3' * 64
            autonomy_receipt = 'forbidden'
        }
        Assert-Task051PublicStatus -Value $extraStatus -Kind 'STATUS'
        throw 'TASK051_SELF_TEST_FALSE_PASS'
    }
    catch {
        if ([string]$_.Exception.Message -cne 'TASK051_PUBLIC_STATUS_SHAPE_REJECTED') { throw }
    }
    $aclParent = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\target\task051-p0-platform-live-acceptance'))
    if (-not (Test-Path -LiteralPath $aclParent -PathType Container)) {
        [IO.Directory]::CreateDirectory($aclParent) | Out-Null
    }
    $aclRoot = Join-Path $aclParent ('selftest-' + [Guid]::NewGuid().ToString('N'))
    $authBefore = [Environment]::GetEnvironmentVariable('LATTICE_TASK051_AUTH_SOURCE', 'Process')
    try {
        New-Task051OwnerOnlyDirectory -Path $aclRoot
        $selfTestTempBefore = [Environment]::GetEnvironmentVariable('TEMP', 'Process')
        $selfTestTmpBefore = [Environment]::GetEnvironmentVariable('TMP', 'Process')
        try {
            [Environment]::SetEnvironmentVariable('TEMP', $aclRoot, 'Process')
            [Environment]::SetEnvironmentVariable('TMP', $aclRoot, 'Process')
            Initialize-Task051ProcessIdentityInterop
            foreach ($openFailureCase in @(
                [pscustomobject]@{ Error = 5; Expected = 'TASK051_PROCESS_INTEROP_OPEN_ACCESS' },
                [pscustomobject]@{ Error = 87; Expected = 'TASK051_PROCESS_INTEROP_OPEN_STALE_PID' },
                [pscustomobject]@{ Error = 0; Expected = 'TASK051_PROCESS_INTEROP_OPEN_OTHER' },
                [pscustomobject]@{ Error = 6; Expected = 'TASK051_PROCESS_INTEROP_OPEN_OTHER' },
                [pscustomobject]@{ Error = 8; Expected = 'TASK051_PROCESS_INTEROP_OPEN_OTHER' }
            )) {
                if ([LatticeTask051ProcessIdentityInterop]::ClassifyOpenFailure([int]$openFailureCase.Error) -cne [string]$openFailureCase.Expected) {
                    throw 'TASK051_PROCESS_OPEN_CLASSIFIER_SELF_TEST_REJECTED'
                }
            }
        }
        finally {
            [Environment]::SetEnvironmentVariable('TEMP', $selfTestTempBefore, 'Process')
            [Environment]::SetEnvironmentVariable('TMP', $selfTestTmpBefore, 'Process')
        }
        $pollState = [pscustomobject]@{ Count = 0 }
        $pollResult = $null
        $pollReader = [IO.StringReader]::new("{`"id`":99,`"result`":{}}`n")
        try {
            $pollResponse = Get-Task051AppServerResponse `
                -Reader $pollReader `
                -Id 99 `
                -TimeoutSeconds 5 `
                -PollAction {
                    $pollState.Count++
                    if ($pollState.Count -ge 3) { return [pscustomobject]@{ Captured = $true } }
                    return $null
                } `
                -PollResult ([ref]$pollResult)
            if (
                [int]$pollResponse.id -ne 99 -or
                $pollState.Count -ne 3 -or
                $null -eq $pollResult -or
                -not [bool]$pollResult.Captured
            ) {
                throw 'TASK051_APP_SERVER_POLL_SELF_TEST_REJECTED'
            }
        }
        finally {
            $pollReader.Dispose()
        }
        $malformedPollState = [pscustomobject]@{ Count = 0 }
        $malformedPollReader = [IO.StringReader]::new("{`"id`":100,`"result`":{}}`n")
        try {
            $malformedPollFailure = $null
            try {
                [void](Get-Task051AppServerResponse `
                    -Reader $malformedPollReader `
                    -Id 100 `
                    -TimeoutSeconds 5 `
                    -PollAction {
                        $malformedPollState.Count++
                        throw 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_JSON_REJECTED'
                    })
            }
            catch {
                $malformedPollFailure = [string]$_.Exception.Message
            }
            if (
                $malformedPollState.Count -ne 1 -or
                $malformedPollFailure -cne 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_JSON_REJECTED'
            ) {
                throw 'TASK051_MCP_SESSION_OPEN_PARSE_DIAGNOSTIC_SELF_TEST_REJECTED'
            }
        }
        finally {
            $malformedPollReader.Dispose()
        }
        $selfTestChild = $null
        $selfTestAuthority = $null
        try {
            $selfTestPowerShell = [IO.Path]::GetFullPath((Join-Path $PSHOME 'powershell.exe'))
            $selfTestInfo = [Diagnostics.ProcessStartInfo]::new()
            $selfTestInfo.FileName = $selfTestPowerShell
            $selfTestInfo.Arguments = '-NoLogo -NoProfile -NonInteractive -Command "Start-Sleep -Seconds 30"'
            $selfTestInfo.WorkingDirectory = $aclRoot
            $selfTestInfo.UseShellExecute = $false
            $selfTestInfo.CreateNoWindow = $true
            $selfTestInfo.RedirectStandardInput = $true
            $selfTestInfo.RedirectStandardOutput = $true
            $selfTestInfo.RedirectStandardError = $true
            Set-Task051ClosedEnvironment -StartInfo $selfTestInfo -Additional ([ordered]@{})
            $selfTestChild = [Diagnostics.Process]::Start($selfTestInfo)
            $selfTestAuthority = [LatticeTask051ProcessIdentityInterop]::AcquireForSelfTest([int]$selfTestChild.Id)
            if (
                $null -eq $selfTestAuthority -or
                -not $selfTestAuthority.IsAlive() -or
                [IO.Path]::GetFullPath([string]$selfTestAuthority.ImagePath) -ine $selfTestPowerShell -or
                (Get-Task051Sha256 -Path ([string]$selfTestAuthority.ImagePath)) -cne (Get-Task051Sha256 -Path $selfTestPowerShell)
            ) {
                throw 'TASK051_RETAINED_PROCESS_AUTHORITY_SELF_TEST_REJECTED'
            }
            $selfTestChild.Kill()
            $selfTestChild.WaitForExit()
            if ($selfTestAuthority.IsAlive()) {
                throw 'TASK051_RETAINED_PROCESS_AUTHORITY_SELF_TEST_REJECTED'
            }
            $selfTestAuthority.CloseExact()
            $selfTestAuthority.CloseExact()
            $selfTestAuthority = $null
        }
        finally {
            $selfTestProcessCleanupFailure = $null
            if ($null -ne $selfTestChild) {
                try { if (-not $selfTestChild.HasExited) { $selfTestChild.Kill(); $selfTestChild.WaitForExit() } }
                catch { $selfTestProcessCleanupFailure = 'TASK051_RETAINED_PROCESS_AUTHORITY_SELF_TEST_CLEANUP_REJECTED' }
                try { $selfTestChild.Dispose() }
                catch { $selfTestProcessCleanupFailure = 'TASK051_RETAINED_PROCESS_AUTHORITY_SELF_TEST_CLEANUP_REJECTED' }
            }
            if ($null -ne $selfTestAuthority) {
                try { $selfTestAuthority.CloseExact() }
                catch { $selfTestProcessCleanupFailure = 'TASK051_RETAINED_PROCESS_AUTHORITY_SELF_TEST_CLEANUP_REJECTED' }
            }
            if ($null -ne $selfTestProcessCleanupFailure) { throw $selfTestProcessCleanupFailure }
        }
        $cargoLinkPathProbe = Join-Path $aclRoot 't\debug\deps\liblattice_postgres_codebase_memory-0123456789abcdef.rlib'
        if ($cargoLinkPathProbe.Length -ge 260) {
            throw 'TASK051_CARGO_LINK_PATH_BUDGET_REJECTED'
        }
        $longIoRoot = Join-Path $aclRoot 'long-path-io'
        $longIoDirectory = '\\?\' + $longIoRoot
        foreach ($index in 1..3) {
            $longIoDirectory += ('\segment-' + ('x' * 40) + $index)
            [IO.Directory]::CreateDirectory($longIoDirectory) | Out-Null
        }
        $longIoFile = $longIoDirectory + ('\rollout-' + ('y' * 70) + '.jsonl')
        [IO.File]::WriteAllText($longIoFile, 'probe')
        $longIoFiles = @(Get-ChildItem -LiteralPath ('\\?\' + $longIoRoot) -Recurse -File -Force)
        if (
            $longIoFiles.Count -ne 1 -or
            $longIoFiles[0].FullName.Length -le 260 -or
            [string]::IsNullOrWhiteSpace((Get-FileHash -LiteralPath $longIoFiles[0].FullName -Algorithm SHA256).Hash)
        ) {
            throw 'TASK051_LONG_PATH_IO_SELF_TEST_REJECTED'
        }
        [IO.Directory]::Delete(('\\?\' + $longIoRoot), $true)
        if (Test-Path -LiteralPath $longIoRoot) { throw 'TASK051_LONG_PATH_IO_SELF_TEST_REJECTED' }
        $realDirectory = Join-Path $aclRoot 'real-directory'
        $junction = Join-Path $aclRoot 'junction'
        [IO.Directory]::CreateDirectory($realDirectory) | Out-Null
        New-Item -ItemType Junction -Path $junction -Target $realDirectory | Out-Null
        try {
            Assert-Task051NoReparseAncestor -Path $junction -Boundary $aclRoot -FailureCode 'TASK051_SELF_TEST_REPARSE_REJECTED'
            throw 'TASK051_SELF_TEST_FALSE_PASS'
        }
        catch {
            if ([string]$_.Exception.Message -cne 'TASK051_SELF_TEST_REPARSE_REJECTED') { throw }
        }
        finally {
            if (Test-Path -LiteralPath $junction) { [IO.Directory]::Delete($junction, $false) }
        }
        $fakeAuth = Join-Path $aclRoot 'fake-auth.json'
        [IO.File]::WriteAllText($fakeAuth, '{"fake":true}', [Text.UTF8Encoding]::new($false))
        Set-Task051OwnerOnlyAcl -Path $fakeAuth -Directory $false
        function Test-LatticeWindowsNativePathIdentity {
            param($Path, $Directory, $ExpectedToken)
            return (-not $Directory -and [string]$ExpectedToken -ceq 'task051-selftest-native')
        }
        $sessionOpenPath = Join-Path $aclRoot 'session-open.jsonl'
        $sessionOpenId = [Guid]::NewGuid().ToString('N')
        $sessionOpenSafeConfig = 'a' * 64
        $sessionOpenObserved = '1770000000000000000'
        $sessionOpenPid = 1234
        $sessionOpenHashInput = @(
            'lattice.mcp.acceptance-dispatch-hash.v1',
            ('0' * 64),
            $sessionOpenId,
            $sessionOpenSafeConfig,
            'SESSION_OPEN',
            '1',
            [string]$sessionOpenPid,
            'null',
            'null',
            '0',
            $sessionOpenObserved
        ) -join "`n"
        $sessionOpenRecord = [ordered]@{
            schema = 'lattice.mcp.acceptance-dispatch.v1'
            record_type = 'SESSION_OPEN'
            session_id = $sessionOpenId
            safe_config_sha256 = $sessionOpenSafeConfig
            process_id = [int]$sessionOpenPid
            ordinal = [int]1
            dispatch_accepted_count = [int]0
            observed_at_unix_nanos = $sessionOpenObserved
            previous_event_sha256 = '0' * 64
            event_sha256 = Get-Task051StringSha256 -Value $sessionOpenHashInput
            tool_name = $null
            request_id_sha256 = $null
        }
        $writeSessionOpen = {
            param([Parameter(Mandatory = $true)]$Record, [switch]$Duplicate)
            $line = $Record | ConvertTo-Json -Compress -Depth 10
            $text = if ($Duplicate) { $line + "`n" + $line + "`n" } else { $line + "`n" }
            [IO.File]::WriteAllText($sessionOpenPath, $text, [Text.UTF8Encoding]::new($false))
            Set-Task051OwnerOnlyAcl -Path $sessionOpenPath -Directory $false
        }
        [IO.File]::WriteAllText($sessionOpenPath, '', [Text.UTF8Encoding]::new($false))
        Set-Task051OwnerOnlyAcl -Path $sessionOpenPath -Directory $false
        if (Test-Task051McpSessionOpenReady -Path $sessionOpenPath -ExpectedNativeIdentity 'task051-selftest-native' -EvidenceRoot $aclRoot) {
            throw 'TASK051_MCP_SESSION_OPEN_READY_SELF_TEST_REJECTED'
        }
        $partialSessionOpen = $sessionOpenRecord | ConvertTo-Json -Compress -Depth 10
        [IO.File]::WriteAllText($sessionOpenPath, $partialSessionOpen, [Text.UTF8Encoding]::new($false))
        Set-Task051OwnerOnlyAcl -Path $sessionOpenPath -Directory $false
        if (Test-Task051McpSessionOpenReady -Path $sessionOpenPath -ExpectedNativeIdentity 'task051-selftest-native' -EvidenceRoot $aclRoot) {
            throw 'TASK051_MCP_SESSION_OPEN_READY_SELF_TEST_REJECTED'
        }
        & $writeSessionOpen -Record $sessionOpenRecord
        if (-not (Test-Task051McpSessionOpenReady -Path $sessionOpenPath -ExpectedNativeIdentity 'task051-selftest-native' -EvidenceRoot $aclRoot)) {
            throw 'TASK051_MCP_SESSION_OPEN_READY_SELF_TEST_REJECTED'
        }
        $parsedSessionOpen = Read-Task051McpSessionOpen -Path $sessionOpenPath -ExpectedNativeIdentity 'task051-selftest-native' -EvidenceRoot $aclRoot -SessionId $sessionOpenId -SafeConfigSha256 $sessionOpenSafeConfig
        if (
            [int]$parsedSessionOpen.ProcessId -ne $sessionOpenPid -or
            [long]$parsedSessionOpen.ObservedAtUnixNanos -ne [long]$sessionOpenObserved -or
            [string]$parsedSessionOpen.EventSha256 -cne [string]$sessionOpenRecord.event_sha256
        ) {
            throw 'TASK051_MCP_SESSION_OPEN_SELF_TEST_REJECTED'
        }
        foreach ($invalidSessionOpen in @(
            [pscustomobject]@{ Kind = 'BAD_HASH'; Value = 'b' * 64 },
            [pscustomobject]@{ Kind = 'STRING_PID'; Value = [string]$sessionOpenPid },
            [pscustomobject]@{ Kind = 'NUMERIC_OBSERVED'; Value = [long]$sessionOpenObserved },
            [pscustomobject]@{ Kind = 'NATIVE_IDENTITY'; Value = 'task051-wrong-native' },
            [pscustomobject]@{ Kind = 'DUPLICATE'; Value = $null }
        )) {
            $candidateRecord = [ordered]@{}
            foreach ($entry in $sessionOpenRecord.GetEnumerator()) { $candidateRecord[$entry.Key] = $entry.Value }
            $expectedIdentity = 'task051-selftest-native'
            $duplicate = $false
            if ($invalidSessionOpen.Kind -ceq 'BAD_HASH') { $candidateRecord.event_sha256 = $invalidSessionOpen.Value }
            elseif ($invalidSessionOpen.Kind -ceq 'STRING_PID') { $candidateRecord.process_id = $invalidSessionOpen.Value }
            elseif ($invalidSessionOpen.Kind -ceq 'NUMERIC_OBSERVED') { $candidateRecord.observed_at_unix_nanos = $invalidSessionOpen.Value }
            elseif ($invalidSessionOpen.Kind -ceq 'NATIVE_IDENTITY') { $expectedIdentity = $invalidSessionOpen.Value }
            elseif ($invalidSessionOpen.Kind -ceq 'DUPLICATE') { $duplicate = $true }
            & $writeSessionOpen -Record $candidateRecord -Duplicate:$duplicate
            $rejected = $false
            try {
                [void](Read-Task051McpSessionOpen -Path $sessionOpenPath -ExpectedNativeIdentity $expectedIdentity -EvidenceRoot $aclRoot -SessionId $sessionOpenId -SafeConfigSha256 $sessionOpenSafeConfig)
            }
            catch {
                $rejected = [string]$_.Exception.Message -ceq 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_REJECTED'
            }
            if (-not $rejected) { throw 'TASK051_MCP_SESSION_OPEN_SELF_TEST_REJECTED' }
        }
        & $writeSessionOpen -Record $sessionOpenRecord
        $detailedSessionOpen = Read-Task051McpSessionOpen -Path $sessionOpenPath -ExpectedNativeIdentity 'task051-selftest-native' -EvidenceRoot $aclRoot -SessionId $sessionOpenId -SafeConfigSha256 $sessionOpenSafeConfig -DetailedFailure
        if ([int]$detailedSessionOpen.ProcessId -ne $sessionOpenPid) {
            throw 'TASK051_MCP_SESSION_OPEN_PARSE_DIAGNOSTIC_SELF_TEST_REJECTED'
        }
        foreach ($detailedInvalidSessionOpen in @(
            [pscustomobject]@{ Kind = 'SOURCE'; Expected = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_SOURCE_REJECTED' },
            [pscustomobject]@{ Kind = 'FRAMING'; Expected = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_FRAMING_REJECTED' },
            [pscustomobject]@{ Kind = 'JSON'; Expected = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_JSON_REJECTED' },
            [pscustomobject]@{ Kind = 'KEYS'; Expected = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_KEYS_REJECTED' },
            [pscustomobject]@{ Kind = 'FIELDS'; Expected = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_FIELDS_REJECTED' },
            [pscustomobject]@{ Kind = 'HASH'; Expected = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_HASH_REJECTED' },
            [pscustomobject]@{ Kind = 'PROJECTION'; Expected = 'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_PROJECTION_REJECTED' }
        )) {
            $candidateRecord = [ordered]@{}
            foreach ($entry in $sessionOpenRecord.GetEnumerator()) { $candidateRecord[$entry.Key] = $entry.Value }
            $expectedIdentity = 'task051-selftest-native'
            $duplicate = $false
            $rawText = $null
            if ($detailedInvalidSessionOpen.Kind -ceq 'SOURCE') { $expectedIdentity = 'task051-wrong-native' }
            elseif ($detailedInvalidSessionOpen.Kind -ceq 'FRAMING') { $duplicate = $true }
            elseif ($detailedInvalidSessionOpen.Kind -ceq 'JSON') { $rawText = "{not-json}`n" }
            elseif ($detailedInvalidSessionOpen.Kind -ceq 'KEYS') { [void]$candidateRecord.Remove('tool_name') }
            elseif ($detailedInvalidSessionOpen.Kind -ceq 'FIELDS') { $candidateRecord.process_id = [string]$sessionOpenPid }
            elseif ($detailedInvalidSessionOpen.Kind -ceq 'HASH') { $candidateRecord.event_sha256 = 'b' * 64 }
            elseif ($detailedInvalidSessionOpen.Kind -ceq 'PROJECTION') {
                $candidateRecord.observed_at_unix_nanos = '9223372036854775808'
                $candidateRecord.event_sha256 = Get-Task051StringSha256 -Value (@(
                    'lattice.mcp.acceptance-dispatch-hash.v1', ('0' * 64), $sessionOpenId,
                    $sessionOpenSafeConfig, 'SESSION_OPEN', '1', [string]$sessionOpenPid,
                    'null', 'null', '0', [string]$candidateRecord.observed_at_unix_nanos
                ) -join "`n")
            }
            if ($null -ne $rawText) {
                [IO.File]::WriteAllText($sessionOpenPath, $rawText, [Text.UTF8Encoding]::new($false))
                Set-Task051OwnerOnlyAcl -Path $sessionOpenPath -Directory $false
            }
            else {
                & $writeSessionOpen -Record $candidateRecord -Duplicate:$duplicate
            }
            $actualDetailedFailure = $null
            try {
                [void](Read-Task051McpSessionOpen -Path $sessionOpenPath -ExpectedNativeIdentity $expectedIdentity -EvidenceRoot $aclRoot -SessionId $sessionOpenId -SafeConfigSha256 $sessionOpenSafeConfig -DetailedFailure)
            }
            catch {
                $actualDetailedFailure = [string]$_.Exception.Message
            }
            if ($actualDetailedFailure -cne [string]$detailedInvalidSessionOpen.Expected) {
                throw 'TASK051_MCP_SESSION_OPEN_PARSE_DIAGNOSTIC_SELF_TEST_REJECTED'
            }
        }
        Remove-Item -LiteralPath $sessionOpenPath -Force
        [Environment]::SetEnvironmentVariable('LATTICE_TASK051_AUTH_SOURCE', $fakeAuth, 'Process')
        try {
            $null = New-Task051CodexHome -Root $aclRoot -Phase 'provisioning-failure' -Latticed "C:\invalid'path\latticed.exe" -EnvironmentNames @('PATH')
            throw 'TASK051_SELF_TEST_FALSE_PASS'
        }
        catch {
            if ([string]$_.Exception.Message -notin @('TASK051_TOML_LITERAL_REJECTED', 'TASK051_SELF_TEST_FALSE_PASS')) {
                throw
            }
            if ([string]$_.Exception.Message -ceq 'TASK051_SELF_TEST_FALSE_PASS') { throw }
        }
        if (Test-Path -LiteralPath (Join-Path $aclRoot 'codex-provisioning-failure')) {
            throw 'TASK051_CODEX_HOME_PROVISIONING_SELF_TEST_REJECTED'
        }
        $fakeAlias = [pscustomobject]@{ Root = $aclRoot; RunRoot = $aclRoot }
        $selfTestPostgresBaseline = @(Get-Task051PostgresProcessSnapshot)
        if (Test-Task051RunRootAliasReleaseSafe -Alias $fakeAlias -RunRoot $aclRoot -BaselinePostgresProcesses $selfTestPostgresBaseline) {
            throw 'TASK051_RUN_ALIAS_MISSING_RECEIPT_SELF_TEST_REJECTED'
        }
        $clusterParent = Join-Path $aclRoot 'task019-postgres'
        $receiptRoot = Join-Path $aclRoot 'task019-holder-receipts'
        [IO.Directory]::CreateDirectory($clusterParent) | Out-Null
        [IO.Directory]::CreateDirectory($receiptRoot) | Out-Null
        if (Test-Task051RunRootAliasReleaseSafe -Alias $fakeAlias -RunRoot $aclRoot -BaselinePostgresProcesses $selfTestPostgresBaseline) {
            throw 'TASK051_RUN_ALIAS_EMPTY_RECEIPT_SELF_TEST_REJECTED'
        }
        $preservedCluster = Join-Path $clusterParent 'preserved'
        [IO.Directory]::CreateDirectory($preservedCluster) | Out-Null
        if (Test-Task051RunRootAliasReleaseSafe -Alias $fakeAlias -RunRoot $aclRoot -BaselinePostgresProcesses $selfTestPostgresBaseline) {
            throw 'TASK051_RUN_ALIAS_PRESERVATION_SELF_TEST_REJECTED'
        }
        [IO.Directory]::Delete($clusterParent, $true)
        [IO.Directory]::Delete($receiptRoot, $true)
        [IO.Directory]::CreateDirectory($clusterParent) | Out-Null
        [IO.Directory]::CreateDirectory($receiptRoot) | Out-Null
        Set-Task051OwnerOnlyAcl -Path $receiptRoot -Directory $true
        $releaseRunId = [Guid]::NewGuid().ToString('N')
        $releaseSessionId = [Guid]::NewGuid().ToString('N')
        $releaseConsumerSessionId = [Guid]::NewGuid().ToString('N')
        $releaseReceiptPath = Join-Path $receiptRoot ($releaseRunId + '.jsonl')
        $releaseClusterRoot = Join-Path $clusterParent $releaseRunId
        $releaseDataDirectory = Join-Path $aclRoot ('task019-postgres\' + $releaseRunId + '\data')
        $releaseToolPath = [IO.Path]::GetFullPath([string](Get-Process -Id $PID -ErrorAction Stop).Path)
        $occupiedReleasePorts = @(Get-NetTCPConnection -State Listen -ErrorAction Stop | ForEach-Object { [int]$_.LocalPort })
        $releasePort = @(49151..49250 | Where-Object { $_ -notin $occupiedReleasePorts } | Select-Object -First 1)
        if ($releasePort.Count -ne 1) { throw 'TASK051_RUN_ALIAS_SELF_TEST_PORT_REJECTED' }
        $writeReleaseReceipt = {
            param(
                [Parameter(Mandatory = $true)]$CleanupComplete,
                [Parameter(Mandatory = $true)][bool]$TamperRequestedPayload
            )
            $payloads = @(
                [ordered]@{
                    cluster_root = $releaseClusterRoot
                    data_directory = $releaseDataDirectory
                    authority_receipt_path = $releaseReceiptPath
                    tool_identity = [ordered]@{
                        postgres_path = $releaseToolPath
                        postgres_sha256 = Get-Task051Sha256 -Path $releaseToolPath
                    }
                },
                [ordered]@{ pg_ctl_status_stopped = $true; listener_absent = $true },
                [ordered]@{ cleanup_requested = $true },
                [ordered]@{ cluster_root = $releaseClusterRoot; cluster_root_absent = $true; listener_absent = $true },
                [ordered]@{ final_event_count_before_close = 4L; cleanup_complete = $CleanupComplete }
            )
            $eventTypes = @('HOLDER_OPEN', 'HOLDER_STOPPED', 'CLEANUP_REQUESTED', 'CLEANUP_COMPLETED', 'RECEIPT_CLOSED')
            $previousHmac = '0' * 64
            $records = [Collections.Generic.List[string]]::new()
            for ($eventIndex = 0; $eventIndex -lt $eventTypes.Count; $eventIndex++) {
                $payload = $payloads[$eventIndex]
                $payloadSha256 = Get-Task051StringSha256 -Value ($payload | ConvertTo-Json -Compress -Depth 20)
                if ($TamperRequestedPayload -and $eventIndex -eq 2) {
                    $payload['tampered_after_digest'] = $true
                }
                $eventHmac = Get-Task051StringSha256 -Value ('task051-release-selftest-' + [string]$eventIndex)
                $record = [ordered]@{
                    schema = 'lattice.task019.postgres-holder-authority.v1'
                    event_type = $eventTypes[$eventIndex]
                    session_id = $releaseSessionId
                    consumer_session_id = $releaseConsumerSessionId
                    run_id = $releaseRunId
                    host = '127.0.0.1'
                    port = [long]$releasePort[0]
                    excluded_ports = @(5432, 64272, 55432)
                    deadline_utc = [DateTimeOffset]::UtcNow.AddMinutes(5).ToString('o')
                    nonce_commitment = 'a' * 64
                    ordinal = [long]($eventIndex + 1)
                    observed_at_utc = [DateTimeOffset]::UtcNow.ToString('o')
                    payload = $payload
                    payload_sha256 = $payloadSha256
                    previous_hmac_sha256 = $previousHmac
                    event_hmac_sha256 = $eventHmac
                }
                $records.Add(($record | ConvertTo-Json -Compress -Depth 24))
                $previousHmac = $eventHmac
            }
            [IO.File]::WriteAllText($releaseReceiptPath, (($records -join [char]10) + [char]10), [Text.UTF8Encoding]::new($false))
            Set-Task051OwnerOnlyAcl -Path $releaseReceiptPath -Directory $false
        }
        & $writeReleaseReceipt -CleanupComplete $true -TamperRequestedPayload:$false
        if (-not (Test-Task051RunRootAliasReleaseSafe -Alias $fakeAlias -RunRoot $aclRoot -BaselinePostgresProcesses $selfTestPostgresBaseline)) {
            throw 'TASK051_RUN_ALIAS_VALID_RECEIPT_SELF_TEST_REJECTED'
        }
        & $writeReleaseReceipt -CleanupComplete 'false' -TamperRequestedPayload:$false
        if (Test-Task051RunRootAliasReleaseSafe -Alias $fakeAlias -RunRoot $aclRoot -BaselinePostgresProcesses $selfTestPostgresBaseline) {
            throw 'TASK051_RUN_ALIAS_BOOLEAN_TAMPER_SELF_TEST_REJECTED'
        }
        & $writeReleaseReceipt -CleanupComplete $true -TamperRequestedPayload:$true
        if (Test-Task051RunRootAliasReleaseSafe -Alias $fakeAlias -RunRoot $aclRoot -BaselinePostgresProcesses $selfTestPostgresBaseline) {
            throw 'TASK051_RUN_ALIAS_PAYLOAD_TAMPER_SELF_TEST_REJECTED'
        }
        Remove-Item -LiteralPath $releaseReceiptPath -Force
        [IO.Directory]::Delete($clusterParent, $true)
        [IO.Directory]::Delete($receiptRoot, $true)
        $selfTestRepositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
        $selfTestWorkspaceRoot = [IO.Path]::GetFullPath((Split-Path -Parent $selfTestRepositoryRoot))
        $selfTestBundleSource = Join-Path $selfTestWorkspaceRoot 'chatgpt-mcp\target'
        $bundleAlias = $null
        try {
            $bundleAlias = New-Task051RunRootAlias -RunRoot $aclRoot
            $selfTestBundleTargetAlias = Join-Path $bundleAlias.Root 'official-bundle-selftest'
            $selfTestLauncher = Copy-Task051OfficialCodexBundle -SourceTargetRoot $selfTestBundleSource -SourceBoundary $selfTestWorkspaceRoot -DestinationTargetRoot $selfTestBundleTargetAlias -DestinationBoundary $selfTestBundleTargetAlias
            if ((Get-Task051Sha256 -Path $selfTestLauncher) -cne 'bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb') {
                throw 'TASK051_OFFICIAL_CODEX_BUNDLE_SELF_TEST_REJECTED'
            }
            Assert-Task051OwnerOnlyAcl -Path $selfTestBundleTargetAlias -Directory $true
            Assert-Task051OwnerOnlyAcl -Path $selfTestLauncher -Directory $false

            $tamperedManifest = Join-Path $selfTestBundleTargetAlias 'codex-official\0.146.0\node_modules\@openai\codex\package.json'
            [IO.File]::WriteAllText($tamperedManifest, '{"tampered":true}', [Text.UTF8Encoding]::new($false))
            $tamperRejected = $false
            try {
                [void](Assert-Task051OfficialCodexBundle -BundleTargetRoot $selfTestBundleTargetAlias -Boundary $selfTestBundleTargetAlias -ValidateVersion)
            }
            catch {
                $tamperRejected = [string]$_.Exception.Message -ceq 'TASK051_OFFICIAL_CODEX_BUNDLE_REJECTED'
            }
            if (-not $tamperRejected) { throw 'TASK051_OFFICIAL_CODEX_BUNDLE_SELF_TEST_REJECTED' }
        }
        finally {
            if ($null -ne $bundleAlias) {
                $selfTestBundleTargetPhysical = Join-Path $aclRoot 'official-bundle-selftest'
                Remove-Task051OwnedDirectory -Path $selfTestBundleTargetPhysical -AllowedRoot $aclRoot -FailureCode 'TASK051_OFFICIAL_CODEX_BUNDLE_SELF_TEST_REJECTED'
                Remove-Task051RunRootAlias -Alias $bundleAlias
                if (Test-Path -LiteralPath $selfTestBundleTargetPhysical) {
                    throw 'TASK051_OFFICIAL_CODEX_BUNDLE_SELF_TEST_REJECTED'
                }
            }
        }
        $reparseBundleSource = Join-Path $aclRoot 'official-bundle-reparse-source'
        New-Task051OwnerOnlyDirectory -Path $reparseBundleSource
        $reparseBundleLink = Join-Path $reparseBundleSource 'codex-official'
        New-Item -ItemType Junction -Path $reparseBundleLink -Target (Join-Path $selfTestBundleSource 'codex-official') | Out-Null
        try {
            $reparseRejected = $false
            try {
                [void](Assert-Task051OfficialCodexBundle -BundleTargetRoot $reparseBundleSource -Boundary $aclRoot)
            }
            catch {
                $reparseRejected = [string]$_.Exception.Message -ceq 'TASK051_OFFICIAL_CODEX_BUNDLE_REJECTED'
            }
            if (-not $reparseRejected) { throw 'TASK051_OFFICIAL_CODEX_BUNDLE_SELF_TEST_REJECTED' }
        }
        finally {
            if (Test-Path -LiteralPath $reparseBundleLink) { [IO.Directory]::Delete($reparseBundleLink, $false) }
            if (Test-Path -LiteralPath $reparseBundleSource) { [IO.Directory]::Delete($reparseBundleSource, $false) }
        }
        $directoryAcl = [IO.Directory]::GetAccessControl($aclRoot)
        $usersSid = [Security.Principal.SecurityIdentifier]::new(
            [Security.Principal.WellKnownSidType]::BuiltinUsersSid,
            $null
        )
        $directoryAcl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
            $usersSid,
            [Security.AccessControl.FileSystemRights]::Read,
            [Security.AccessControl.AccessControlType]::Allow
        ))
        [IO.Directory]::SetAccessControl($aclRoot, $directoryAcl)
        try {
            Assert-Task051OwnerOnlyAcl -Path $aclRoot -Directory $true
            throw 'TASK051_SELF_TEST_FALSE_PASS'
        }
        catch {
            if ([string]$_.Exception.Message -cne 'TASK051_OWNER_ONLY_ACL_REJECTED') { throw }
        }
    }
    finally {
        [Environment]::SetEnvironmentVariable('LATTICE_TASK051_AUTH_SOURCE', $authBefore, 'Process')
        if (Test-Path -LiteralPath $aclRoot -PathType Container) {
            [IO.Directory]::Delete(('\\?\' + $aclRoot), $true)
        }
        if (Test-Path -LiteralPath $aclRoot) {
            throw 'TASK051_SELF_TEST_ROOT_CLEANUP_REJECTED'
        }
    }
    $jobEvents = [Collections.Generic.List[string]]::new()
    $script:Task051SelfTestJobEvents = $jobEvents
    function New-Task038KillOnCloseJob { $script:Task051SelfTestJobEvents.Add('New'); return [IntPtr]1 }
    function Start-Task038SuspendedProcess {
        param($StartInfo)
        $script:Task051SelfTestJobEvents.Add('Start')
        $value = [pscustomobject]@{ Process = [Diagnostics.Process]::GetCurrentProcess() }
        $value | Add-Member -MemberType ScriptMethod -Name Dispose -Value { $script:Task051SelfTestJobEvents.Add('Dispose') }
        return $value
    }
    function Add-Task038ProcessToJob { param($Job, $Process); $script:Task051SelfTestJobEvents.Add('Add'); throw 'TASK051_FAKE_ADD_FAILURE' }
    function Resume-Task038SuspendedProcess { param($SuspendedProcess); $script:Task051SelfTestJobEvents.Add('Resume') }
    function Stop-Task038Job { param($Job); $script:Task051SelfTestJobEvents.Add('StopJob') }
    function Close-Task038Job { param($Job); $script:Task051SelfTestJobEvents.Add('CloseJob') }
    function Stop-Task038ProcessTree { param($Process); $script:Task051SelfTestJobEvents.Add('StopTree') }
    try {
        $null = Start-Task051OwnedProcess -StartInfo ([Diagnostics.ProcessStartInfo]::new())
        throw 'TASK051_SELF_TEST_FALSE_PASS'
    }
    catch {
        if ([string]$_.Exception.Message -cne 'TASK051_FAKE_ADD_FAILURE') { throw }
    }
    if (($jobEvents -join ',') -cne 'New,Start,Add,StopJob,CloseJob,StopTree,Dispose') {
        throw 'TASK051_PROCESS_START_CLEANUP_SELF_TEST_REJECTED'
    }
    $repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
    $task038Path = Join-Path $PSScriptRoot 'run-task038-task-submit.ps1'
    $task019Path = Join-Path $PSScriptRoot 'run-task019-postgres.ps1'
    $task038 = Convert-Task051Task038Source -Source ([IO.File]::ReadAllText($task038Path)) -ScriptsRoot $PSScriptRoot -RunnerPath $PSCommandPath
    $task019 = Convert-Task051Task019Source -Source ([IO.File]::ReadAllText($task019Path)) -ScriptsRoot $PSScriptRoot
    if ($task038.IndexOf('Restart-DisposablePostgres', [StringComparison]::Ordinal) -lt 0) {
        throw 'TASK051_POSTGRES_RESTART_TRANSFORM_REJECTED'
    }
    foreach ($cargoFragment in @('CARGO_HOME', 'CARGO_NET_OFFLINE', 'TASK051_CARGO_CACHE_SOURCE_REJECTED')) {
        if ([IO.File]::ReadAllText($PSCommandPath).IndexOf($cargoFragment, [StringComparison]::Ordinal) -lt 0) {
            throw ('TASK051_CARGO_CONTAINMENT_SELF_TEST_REJECTED|' + $cargoFragment)
        }
    }
    $tokens = $null
    $errors = $null
    [void][Management.Automation.Language.Parser]::ParseInput($task038, [ref]$tokens, [ref]$errors)
    if (@($errors).Count -ne 0) { throw 'TASK051_TASK038_TRANSFORM_PARSE_REJECTED' }
    [void][Management.Automation.Language.Parser]::ParseInput($task019, [ref]$tokens, [ref]$errors)
    if (@($errors).Count -ne 0) { throw 'TASK051_TASK019_TRANSFORM_PARSE_REJECTED' }
    if (
        [regex]::Matches($task038, [regex]::Escape('$candidateLatticedNativeIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $script:Latticed -Directory $false')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('-ExpectedLatticedSha256 $candidateLatticedSha256 -ExpectedLatticedNativeIdentity $candidateLatticedNativeIdentity')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape("'TASK051_DISCOVERY|' + `$acceptanceId + '|' + `$candidateLatticedSha256 + '|' + `$candidateLatticedNativeIdentity")).Count -ne 1
    ) {
        throw 'TASK051_TASK038_CANDIDATE_BINARY_COMMITMENT_SELF_TEST_REJECTED'
    }
    if (
        [regex]::Matches($task038, [regex]::Escape("`$task051PhysicalPostgresData = Get-CanonicalPath -Path (Join-Path `$env:LATTICE_TASK051_RUN_ROOT ('task019-postgres\' + `$PostgresRunId + '\data'))")).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('Assert-NoReparseAncestor -Path $script:PostgresData -Boundary $env:LATTICE_TASK051_RUN_ALIAS_ROOT')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('Get-LatticeWindowsNativePathIdentityToken -Path $task051PhysicalPostgresData -Directory $true')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('$script:Task051PhysicalPostgresRoot = Get-CanonicalPath -Path (Split-Path -Parent $task051PhysicalPostgresData)')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('$script:Task051PhysicalPostgresParent = Get-CanonicalPath -Path (Split-Path -Parent $script:Task051PhysicalPostgresRoot)')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('$clusterRoot = $script:Task051PhysicalPostgresRoot')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('-Expected (Split-Path -Parent $script:PostgresData)')).Count -ne 0 -or
        [regex]::Matches($task038, [regex]::Escape('-Expected $script:Task051PhysicalPostgresRoot')).Count -ne 3 -or
        [regex]::Matches($task038, [regex]::Escape("-Expected (Join-Path `$script:Task051PhysicalPostgresRoot 'postgres.log')")).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('$captureRoot = Get-CanonicalPath -Path (Split-Path -Parent $DataDirectory)')).Count -ne 0 -or
        [regex]::Matches($task038, [regex]::Escape('$captureRoot = Get-CanonicalPath -Path $script:Task051PhysicalPostgresRoot')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('-ParentPath $repositoryTarget')).Count -ne 0 -or
        [regex]::Matches($task038, [regex]::Escape('-ParentPath $script:Task051PhysicalPostgresParent')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('$task051PhysicalOutputDirectory = Get-CanonicalPath -Path $OutputDirectory')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('$canonicalOutputDirectory = Get-CanonicalPath -Path (Split-Path -Parent $script:PostgresData)')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('-Boundary $env:LATTICE_TASK051_RUN_ALIAS_ROOT')).Count -ne 2 -or
        [regex]::Matches($task038, [regex]::Escape("throw 'TASK038_POSTGRES_DATA_NATIVE_LINK_REJECTED'")).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape("`$executionParent = Get-CanonicalPath -Path (Join-Path (Split-Path -Parent `$source) 'task038-execution-homes')")).Count -ne 0 -or
        [regex]::Matches($task038, [regex]::Escape("`$executionParent = Get-CanonicalPath -Path (Join-Path `$env:LATTICE_TASK051_RUN_ROOT 't38h')")).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('Assert-NoReparseAncestor -Path $executionParent -Boundary $env:LATTICE_TASK051_RUN_ROOT')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('$task038CargoTarget = Get-CanonicalPath -Path $env:CARGO_TARGET_DIR')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape("Join-Path `$env:CARGO_TARGET_DIR 'task038-main'")).Count -ne 0 -or
        [regex]::Matches($task038, [regex]::Escape("`$cargoHostLines = @(`$cargoVersion.Text -split '\r?\n' | Where-Object { `$_ -like 'host: *' })")).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape("'--target-dir', `$task038CargoTarget, '--target', `$cargoHostTarget")).Count -ne 0 -or
        [regex]::Matches($task038, [regex]::Escape("'--target-dir', `$task038CargoTarget")).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape("Join-Path `$task038CargoTarget 'debug\latticed.exe'")).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape("'TASK076_WRITER_V2_VERIFIED', 'CONSUMER_STARTED'")).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('$consumer = $records[5].payload')).Count -ne 0 -or
        [regex]::Matches($task038, [regex]::Escape('$consumer = $records[6].payload')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape("Join-Path `$repositoryTarget 'lattice-delivery'")).Count -ne 0 -or
        [regex]::Matches($task038, [regex]::Escape('$fixtureParent = Get-CanonicalPath -Path $repositoryTarget')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('Join-Path $fixtureParent $acceptanceId')).Count -ne 0 -or
        [regex]::Matches($task038, [regex]::Escape("`$fixtureRoot = Get-CanonicalPath -Path (Join-Path `$fixtureParent 'd')")).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape("`$evidenceRoot = Join-Path `$fixtureRoot 'e'")).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('LATTICE_TASK051_RUN_ALIAS_ROOT ''task038-execution-homes''')).Count -ne 0 -or
        [regex]::Matches($task038, [regex]::Escape('TASK038_CODEX_EXECUTION_PARENT_NATIVE_LINK_REJECTED')).Count -ne 0 -or
        [regex]::Matches($task038, [regex]::Escape("`$task051LongPathRoot = '\\?\' + `$canonicalRoot")).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('Get-ChildItem -LiteralPath $task051LongPathRoot -Recurse -Force')).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape("`$task051LongExecutionHome = '\\?\' + `$executionHome")).Count -ne 1 -or
        [regex]::Matches($task038, [regex]::Escape('[IO.Directory]::Delete($task051LongExecutionHome, $true)')).Count -ne 1
    ) {
        throw 'TASK051_TASK038_POSTGRES_DATA_ALIAS_SELF_TEST_REJECTED'
    }
    $compactRunRoot = Join-Path $repositoryRoot ('target\task051-p0-platform-live-acceptance\' + [Guid]::NewGuid().ToString('N'))
    New-Task051OwnerOnlyDirectory -Path $compactRunRoot | Out-Null
    try {
        $compactEvidencePath = Join-Path $compactRunRoot ('d\e\mcp-dispatch\mcp-observed-effects\' + ('a' * 32) + '.jsonl')
        if ($compactEvidencePath.Length -ge 260) {
            throw 'TASK051_TASK038_COMPACT_EVIDENCE_PATH_BUDGET_REJECTED'
        }
        $compactEvidenceParent = Split-Path -Parent $compactEvidencePath
        [IO.Directory]::CreateDirectory($compactEvidenceParent) | Out-Null
        Assert-Task051NoReparseAncestor -Path $compactEvidenceParent -Boundary $compactRunRoot -FailureCode 'TASK051_TASK038_COMPACT_EVIDENCE_REPARSE_REJECTED'
        [IO.File]::WriteAllText($compactEvidencePath, "task051-compact-evidence-self-test`n", [Text.UTF8Encoding]::new($false))
        if ([IO.File]::ReadAllText($compactEvidencePath, [Text.Encoding]::UTF8) -cne "task051-compact-evidence-self-test`n") {
            throw 'TASK051_TASK038_COMPACT_EVIDENCE_IO_REJECTED'
        }
        [IO.File]::Delete($compactEvidencePath)
        if (Test-Path -LiteralPath $compactEvidencePath) {
            throw 'TASK051_TASK038_COMPACT_EVIDENCE_IO_REJECTED'
        }
    }
    finally {
        if (Test-Path -LiteralPath $compactRunRoot -PathType Container) {
            [IO.Directory]::Delete(('\\?\' + $compactRunRoot), $true)
        }
        if (Test-Path -LiteralPath $compactRunRoot) {
            throw 'TASK051_TASK038_COMPACT_EVIDENCE_CLEANUP_REJECTED'
        }
    }
    foreach ($holderDiagnostic in @(
        'TASK038_POSTGRES_HOLDER_PREFIX_REJECTED',
        'TASK038_POSTGRES_HOLDER_CHAIN_REJECTED',
        'TASK038_POSTGRES_HOLDER_OWNER_PROCESS_REJECTED',
        'TASK038_POSTGRES_HOLDER_SCOPE_REJECTED',
        'TASK038_POSTGRES_HOLDER_TOOL_REJECTED',
        'TASK038_POSTGRES_HOLDER_MARKER_REJECTED',
        'TASK038_POSTGRES_HOLDER_CONSUMER_REJECTED',
        'TASK038_POSTGRES_HOLDER_LISTENER_SOCKET_REJECTED',
        'TASK038_POSTGRES_HOLDER_LISTENER_PROCESS_QUERY_REJECTED',
        'TASK038_POSTGRES_HOLDER_LISTENER_EXECUTABLE_REJECTED',
        'TASK038_POSTGRES_HOLDER_LISTENER_RECEIPT_REJECTED',
        'TASK038_POSTGRES_HOLDER_LISTENER_CREATION_REJECTED',
        'TASK038_POSTGRES_HOLDER_LISTENER_REJECTED'
    )) {
        if ([regex]::Matches($task038, [regex]::Escape($holderDiagnostic)).Count -ne 1) {
            throw 'TASK051_TASK038_HOLDER_DIAGNOSTIC_SELF_TEST_REJECTED'
        }
    }
    if (
        [regex]::Matches(
            $task019,
            [regex]::Escape('param([Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Output)')
        ).Count -ne 1
    ) {
        throw 'TASK051_TASK019_EMPTY_DIAGNOSTIC_SELF_TEST_REJECTED'
    }
    if (
        [regex]::Matches($task019, [regex]::Escape('Join-Path $clusterRoot ".cargo')).Count -ne 0 -or
        [regex]::Matches($task019, [regex]::Escape("Join-Path `$clusterRoot '.cargo")).Count -ne 0 -or
        [regex]::Matches($task019, [regex]::Escape('Join-Path $task051CargoOutputRoot')).Count -ne 14
    ) {
        throw 'TASK051_TASK019_CARGO_OUTPUT_ALIAS_SELF_TEST_REJECTED'
    }
    if (
        [regex]::Matches($task019, [regex]::Escape("`$dataDirectory = Join-Path `$clusterRoot 'data'")).Count -ne 0 -or
        [regex]::Matches($task019, [regex]::Escape("`$dataDirectory = Join-Path `$env:LATTICE_TASK051_RUN_ALIAS_ROOT ('task019-postgres\' + `$runId + '\data')")).Count -ne 1 -or
        [regex]::Matches($task019, [regex]::Escape("            '--pgdata', `$dataDirectory,")).Count -ne 1
    ) {
        throw 'TASK051_TASK019_RUNTIME_PGDATA_ALIAS_SELF_TEST_REJECTED'
    }
    foreach ($cleanupFailure in @(
        'TASK051_WRITER_OWNER_OUTPUT_DELETE_FAILED',
        'TASK051_CATALOG_OUTPUT_DELETE_FAILED'
    )) {
        if ([regex]::Matches($task019, [regex]::Escape($cleanupFailure)).Count -ne 1) {
            throw 'TASK051_TASK019_CARGO_OUTPUT_CLEANUP_SELF_TEST_REJECTED'
        }
    }
    $writerOwnerDeleteObserved = $task019.IndexOf('$task051WriterOwnerOutputCleanupFailed = $true', [StringComparison]::Ordinal)
    if ($writerOwnerDeleteObserved -lt 0) {
        throw 'TASK051_TASK019_WRITER_OWNER_CLEANUP_ORDER_SELF_TEST_REJECTED'
    }
    $writerOwnerStopCompleted = $task019.IndexOf("-FailureCode 'TASK019_WRITER_LEASE_OWNER_STOP_REJECTED'", $writerOwnerDeleteObserved, [StringComparison]::Ordinal)
    if ($writerOwnerStopCompleted -lt 0) {
        throw 'TASK051_TASK019_WRITER_OWNER_CLEANUP_ORDER_SELF_TEST_REJECTED'
    }
    $writerOwnerDeleteRejected = $task019.IndexOf("throw 'TASK051_WRITER_OWNER_OUTPUT_DELETE_FAILED'", $writerOwnerStopCompleted, [StringComparison]::Ordinal)
    if (
        $writerOwnerStopCompleted -le $writerOwnerDeleteObserved -or
        $writerOwnerDeleteRejected -le $writerOwnerStopCompleted
    ) {
        throw 'TASK051_TASK019_WRITER_OWNER_CLEANUP_ORDER_SELF_TEST_REJECTED'
    }
    Write-Output 'TASK051_SOURCE_TRANSFORM_SELF_TEST=PASS'
    Write-Output 'TASK051_CODEX_EVENT_PARSER_SELF_TEST=PASS'
    Write-Output 'TASK051_APP_SERVER_DISCOVERY_SELF_TEST=PASS'
    Write-Output 'TASK051_PROCESS_OPEN_CLASSIFIER_SELF_TEST=PASS'
    Write-Output 'TASK051_RETAINED_PROCESS_AUTHORITY_SELF_TEST=PASS'
    Write-Output 'TASK051_MCP_SESSION_OPEN_PARSE_DIAGNOSTIC_SELF_TEST=PASS'
    Write-Output 'TASK051_OFFICIAL_CODEX_BUNDLE_SELF_TEST=PASS'
    Write-Output 'TASK051_OWNER_ONLY_CREDENTIAL_SELF_TEST=PASS'
    Write-Output 'TASK051_PROCESS_CONTAINMENT_SELF_TEST=PASS'
    Write-Output 'TASK051_RUNNER_SELF_TEST=PASS'
}

if ($LibraryOnly) { return }
if ($SelfTestOnly) {
    Invoke-Task051SelfTest
    return
}

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$allowedRoot = [IO.Path]::GetFullPath((Join-Path $repositoryRoot 'target\task051-p0-platform-live-acceptance'))
Assert-Task051NoReparseAncestor -Path $allowedRoot -Boundary $repositoryRoot -FailureCode 'TASK051_ALLOWED_ROOT_REPARSE_REJECTED'
if (-not (Test-Path -LiteralPath $allowedRoot -PathType Container)) {
    [IO.Directory]::CreateDirectory($allowedRoot) | Out-Null
}
Assert-Task051NoReparseAncestor -Path $allowedRoot -Boundary $repositoryRoot -FailureCode 'TASK051_ALLOWED_ROOT_REPARSE_REJECTED'
$runId = [Guid]::NewGuid().ToString('N')
$runRoot = [IO.Path]::GetFullPath((Join-Path $allowedRoot $runId))
if (Test-Path -LiteralPath $runRoot) { throw 'TASK051_RUN_ROOT_NOT_FRESH' }
Assert-Task051NoReparseAncestor -Path $runRoot -Boundary $repositoryRoot -FailureCode 'TASK051_RUN_ROOT_REPARSE_REJECTED'
New-Task051OwnerOnlyDirectory -Path $runRoot
Assert-Task051NoReparseAncestor -Path $runRoot -Boundary $repositoryRoot -FailureCode 'TASK051_RUN_ROOT_REPARSE_REJECTED'
$originalConfig = [IO.Path]::GetFullPath((Join-Path $env:USERPROFILE '.codex\config.toml'))
$authSource = [IO.Path]::GetFullPath((Join-Path $env:USERPROFILE '.codex\auth.json'))
Assert-Task051RegularFile -Path $originalConfig -FailureCode 'TASK051_ORIGINAL_CONFIG_REJECTED'
Assert-Task051RegularFile -Path $authSource -FailureCode 'TASK051_CODEX_AUTH_SOURCE_REJECTED'
$originalConfigSha256 = Get-Task051Sha256 -Path $originalConfig
$branch = (& git -C $repositoryRoot branch --show-current).Trim()
$head = (& git -C $repositoryRoot rev-parse HEAD).Trim()
$tree = (& git -C $repositoryRoot rev-parse 'HEAD^{tree}').Trim()
$status = @(& git -C $repositoryRoot status --porcelain=v1 --untracked-files=all)
if ($branch -cne 'feature/task-051-p0-platform-live-acceptance' -or $status.Count -ne 0) {
    throw 'TASK051_CANDIDATE_SOURCE_REJECTED'
}
& git -C $repositoryRoot merge-base --is-ancestor $script:Task051ExpectedTask050Commit HEAD
if ($LASTEXITCODE -ne 0) { throw 'TASK051_TASK050_PROVENANCE_REJECTED' }
$task050Tree = (& git -C $repositoryRoot rev-parse ($script:Task051ExpectedTask050Commit + '^{tree}')).Trim()
if ($task050Tree -cne $script:Task051ExpectedTask050Tree) { throw 'TASK051_TASK050_PROVENANCE_REJECTED' }
$currentCodexCandidates = @(Get-ChildItem -LiteralPath (Join-Path $env:LOCALAPPDATA 'OpenAI\Codex\bin') -Filter codex.exe -Recurse -File -ErrorAction Stop | Sort-Object LastWriteTimeUtc -Descending)
if ($currentCodexCandidates.Count -lt 1) { throw 'TASK051_CURRENT_CODEX_REJECTED' }
$currentCodex = [IO.Path]::GetFullPath($currentCodexCandidates[0].FullName)
$currentCodexVersion = (& $currentCodex --version).Trim()
if ($currentCodexVersion -cne 'codex-cli 0.147.0-alpha.6.6') { throw 'TASK051_CURRENT_CODEX_REJECTED' }
$currentCodexSemanticVersion = $currentCodexVersion.Substring('codex-cli '.Length)
$currentCodexUserAgent = 'lattice-task051-acceptance/' + $currentCodexSemanticVersion + ' (Windows ' + [Environment]::OSVersion.Version.ToString(3) + '; x86_64) unknown (lattice-task051-acceptance; 1)'
$workspaceRoot = [IO.Path]::GetFullPath((Split-Path -Parent $repositoryRoot))
$officialCodexSourceTarget = [IO.Path]::GetFullPath((Join-Path $workspaceRoot 'chatgpt-mcp\target'))
$officialCodex = $null
$officialCodexSha256 = 'bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb'
$privateOfficialCodexBundleTarget = Join-Path $runRoot 'official-bundle-target'
$credentialSource = Join-Path $runRoot 'credential-source'
$generatedTask038 = Join-Path $runRoot 'run-task038-task-submit.generated.ps1'
$generatedTask019 = Join-Path $runRoot 'run-task019-postgres.generated.ps1'
$cargoTarget = Join-Path $runRoot 't'
$cargoHome = Join-Path $runRoot 'cargo-home'
$tempRoot = Join-Path $runRoot 'temp'
$environmentBefore = [ordered]@{}
foreach ($name in @(
    'CARGO_HOME',
    'CARGO_NET_OFFLINE',
    'CARGO_TARGET_DIR',
    'TEMP',
    'TMP',
    'LATTICE_TASK051_RUN_ROOT',
    'LATTICE_TASK051_RUN_ALIAS_ROOT',
    'LATTICE_TASK051_GENERATED_TASK038',
    'LATTICE_TASK051_CURRENT_CODEX',
    'LATTICE_TASK051_CURRENT_CODEX_USER_AGENT',
    'LATTICE_TASK051_AUTH_SOURCE',
    'LATTICE_TASK051_ORIGINAL_CONFIG_SHA256'
)) {
    $environmentBefore[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
$primaryFailure = $null
$harnessOutput = @()
$runAlias = $null
$externalResourcesMayExist = $false
$postgresProcessBaseline = @(Get-Task051PostgresProcessSnapshot)
try {
    $runAlias = New-Task051RunRootAlias -RunRoot $runRoot
    $privateOfficialCodexBundleTargetAlias = Join-Path $runAlias.Root 'official-bundle-target'
    $officialCodex = Copy-Task051OfficialCodexBundle -SourceTargetRoot $officialCodexSourceTarget -SourceBoundary $workspaceRoot -DestinationTargetRoot $privateOfficialCodexBundleTargetAlias -DestinationBoundary $privateOfficialCodexBundleTargetAlias
    New-Task051OwnerOnlyDirectory -Path $credentialSource
    $credentialAuth = Join-Path $credentialSource 'auth.json'
    [IO.File]::Copy($authSource, $credentialAuth, $false)
    Set-Task051OwnerOnlyAcl -Path $credentialAuth -Directory $false
    [IO.File]::WriteAllText((Join-Path $credentialSource '.lattice-codex-home-v1'), ('lattice.codex-home.v1' + [char]10), [Text.UTF8Encoding]::new($false))
    $task038Source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'run-task038-task-submit.ps1'))
    $task019Source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'run-task019-postgres.ps1'))
    $task038Source = Convert-Task051Task038Source -Source $task038Source -ScriptsRoot $PSScriptRoot -RunnerPath $PSCommandPath
    $task019Source = Convert-Task051Task019Source -Source $task019Source -ScriptsRoot $PSScriptRoot
    [IO.File]::WriteAllText($generatedTask038, $task038Source, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($generatedTask019, $task019Source, [Text.UTF8Encoding]::new($false))
    Initialize-Task051CargoHome -Destination $cargoHome
    New-Task051OwnerOnlyDirectory -Path $tempRoot
    [IO.Directory]::CreateDirectory($cargoTarget) | Out-Null
    [Environment]::SetEnvironmentVariable('CARGO_HOME', (Join-Path $runAlias.Root 'cargo-home'), 'Process')
    [Environment]::SetEnvironmentVariable('CARGO_NET_OFFLINE', 'true', 'Process')
    [Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', (Join-Path $runAlias.Root 't'), 'Process')
    [Environment]::SetEnvironmentVariable('TEMP', (Join-Path $runAlias.Root 'temp'), 'Process')
    [Environment]::SetEnvironmentVariable('TMP', (Join-Path $runAlias.Root 'temp'), 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK051_RUN_ROOT', $runRoot, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK051_RUN_ALIAS_ROOT', $runAlias.Root, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK051_GENERATED_TASK038', $generatedTask038, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK051_CURRENT_CODEX', $currentCodex, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK051_CURRENT_CODEX_USER_AGENT', $currentCodexUserAgent, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK051_AUTH_SOURCE', $authSource, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK051_ORIGINAL_CONFIG_SHA256', $originalConfigSha256, 'Process')
    $externalResourcesMayExist = $true
    $harnessOutput = @(& $generatedTask019 -RunTask076WriterLeaseGate -RunTask038AcceptanceHook -Task038OfficialCodexExecutable $officialCodex -Task038CodexAuthHome $credentialSource 2>&1 | ForEach-Object { [string]$_ })
    if ($LASTEXITCODE -ne 0) { throw 'TASK051_TASK019_HARNESS_REJECTED' }
}
catch {
    $primaryFailure = [string]$_.Exception.Message
}
finally {
    $cleanupFailure = $null
    if (Test-Path -LiteralPath $credentialSource -PathType Container) {
        try { [IO.Directory]::Delete($credentialSource, $true) }
        catch { $cleanupFailure = 'TASK051_CREDENTIAL_SOURCE_CLEANUP_REJECTED' }
    }
    if (Test-Path -LiteralPath $credentialSource) {
        $cleanupFailure = 'TASK051_CREDENTIAL_SOURCE_CLEANUP_REJECTED'
    }
    try {
        Remove-Task051OwnedDirectory -Path $cargoHome -AllowedRoot $runRoot -FailureCode 'TASK051_CARGO_HOME_CLEANUP_REJECTED'
    }
    catch {
        if ($null -eq $cleanupFailure) {
            $cleanupFailure = 'TASK051_CARGO_HOME_CLEANUP_REJECTED'
        }
    }
    try {
        Remove-Task051OwnedDirectory -Path $tempRoot -AllowedRoot $runRoot -FailureCode 'TASK051_TEMP_CLEANUP_REJECTED'
    }
    catch {
        if ($null -eq $cleanupFailure) {
            $cleanupFailure = 'TASK051_TEMP_CLEANUP_REJECTED'
        }
    }
    foreach ($entry in $environmentBefore.GetEnumerator()) {
        try { [Environment]::SetEnvironmentVariable([string]$entry.Key, $entry.Value, 'Process') }
        catch {
            if ($null -eq $cleanupFailure) {
                $cleanupFailure = 'TASK051_PROCESS_ENVIRONMENT_CLEANUP_REJECTED'
            }
        }
    }
    if ($null -ne $runAlias) {
        try {
            if (
                $externalResourcesMayExist -and
                -not (Test-Task051RunRootAliasReleaseSafe -Alias $runAlias -RunRoot $runRoot -BaselinePostgresProcesses $postgresProcessBaseline)
            ) {
                throw 'TASK051_RUN_ALIAS_PRESERVED_FOR_ACTIVE_RESOURCE'
            }
            try {
                Remove-Task051OwnedDirectory -Path $privateOfficialCodexBundleTarget -AllowedRoot $runRoot -FailureCode 'TASK051_OFFICIAL_CODEX_BUNDLE_CLEANUP_REJECTED'
            }
            catch {
                if ($null -eq $cleanupFailure) {
                    $cleanupFailure = 'TASK051_OFFICIAL_CODEX_BUNDLE_CLEANUP_REJECTED'
                }
            }
            Remove-Task051RunRootAlias -Alias $runAlias
        }
        catch {
            if ($null -eq $cleanupFailure) {
                $cleanupFailure = [string]$_.Exception.Message
            }
        }
    }
    if ($null -ne $cleanupFailure) { throw $cleanupFailure }
}
if ((Get-Task051Sha256 -Path $originalConfig) -cne $originalConfigSha256) {
    throw 'TASK051_CONFIG_ROLLBACK_REJECTED'
}
if ($null -ne $primaryFailure) {
    throw ('TASK051_LIVE_ACCEPTANCE_REJECTED|' + (Get-Task051StringSha256 -Value ($harnessOutput -join [char]10)) + '|' + $primaryFailure)
}
$task038FinalCandidates = @(Get-ChildItem -LiteralPath $runRoot -Filter final.json -Recurse -File | Where-Object {
    try {
        $candidate = Get-Content -Raw -LiteralPath $_.FullName | ConvertFrom-Json -ErrorAction Stop
        [string]$candidate.schema_version -ceq 'lattice.task051.task038-derived-acceptance.v1' -and
        [string]$candidate.status -ceq 'PASS'
    }
    catch { $false }
})
if ($task038FinalCandidates.Count -ne 1) { throw 'TASK051_TASK038_FINAL_EVIDENCE_REJECTED' }
$task038FinalPath = $task038FinalCandidates[0].FullName
$task038Final = Get-Content -Raw -LiteralPath $task038FinalPath | ConvertFrom-Json -ErrorAction Stop
$currentCodexEvidence = @(
    [pscustomobject]@{ Path = [string]$task038Final.task051_discovery_evidence_path; Sha256 = [string]$task038Final.task051_discovery_evidence_sha256 }
    [pscustomobject]@{ Path = [string]$task038Final.task051_submit_evidence_path; Sha256 = [string]$task038Final.task051_submit_evidence_sha256 }
    [pscustomobject]@{ Path = [string]$task038Final.task051_pre_restart_evidence_path; Sha256 = [string]$task038Final.task051_pre_restart_evidence_sha256 }
    [pscustomobject]@{ Path = [string]$task038Final.task051_post_restart_evidence_path; Sha256 = [string]$task038Final.task051_post_restart_evidence_sha256 }
)
$runRootPrefix = $runRoot.TrimEnd('\') + '\'
foreach ($entry in $currentCodexEvidence) {
    $evidencePath = [IO.Path]::GetFullPath([string]$entry.Path)
    if (
        -not $evidencePath.StartsWith($runRootPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        [string]$entry.Sha256 -cnotmatch '\A[0-9a-f]{64}\z'
    ) {
        throw 'TASK051_CURRENT_CODEX_EVIDENCE_LINKAGE_REJECTED'
    }
    Assert-Task051RegularFile -Path $evidencePath -FailureCode 'TASK051_CURRENT_CODEX_EVIDENCE_LINKAGE_REJECTED'
    if ((Get-Task051Sha256 -Path $evidencePath) -cne [string]$entry.Sha256) {
        throw 'TASK051_CURRENT_CODEX_EVIDENCE_LINKAGE_REJECTED'
    }
}
$requiredMarkers = @(
    'TASK076_WRITER_LEASE_V2_BRIDGE=PASS',
    'TASK076_WRITER_LEASE_V2_FRESH_CURRENT=PASS',
    'LATTICED_LOCAL_MCP_ACCEPTANCE=PASS'
)
foreach ($marker in $requiredMarkers) {
    if (@($harnessOutput | Where-Object { $_ -ceq $marker }).Count -ne 1) {
        throw ('TASK051_INNER_MARKER_REJECTED|' + $marker)
    }
}
$final = [ordered]@{
    schema_version = 'lattice.task051.p0-platform-live-acceptance.v1'
    status = 'VERIFIED'
    run_id = $runId
    source_branch = $branch
    source_commit = $head
    source_tree = $tree
    accepted_task050_commit = $script:Task051ExpectedTask050Commit
    accepted_task050_tree = $script:Task051ExpectedTask050Tree
    dependency_state = $script:Task051DependencyState
    current_codex_path = $currentCodex
    current_codex_version = $currentCodexVersion
    current_codex_sha256 = Get-Task051Sha256 -Path $currentCodex
    task038_official_codex_sha256 = $officialCodexSha256
    original_config_path = $originalConfig
    original_config_sha256 = $originalConfigSha256
    original_config_unchanged = $true
    process_local_registration = $true
    exact_four_tool_discovery = $true
    discovery_evidence_path = [string]$task038Final.task051_discovery_evidence_path
    discovery_evidence_sha256 = [string]$task038Final.task051_discovery_evidence_sha256
    current_codex_submit_once = $true
    submit_evidence_path = [string]$task038Final.task051_submit_evidence_path
    submit_evidence_sha256 = [string]$task038Final.task051_submit_evidence_sha256
    fresh_pre_restart_status = $true
    pre_restart_status_evidence_path = [string]$task038Final.task051_pre_restart_evidence_path
    pre_restart_status_evidence_sha256 = [string]$task038Final.task051_pre_restart_evidence_sha256
    physical_postgres_restart = $true
    fresh_post_restart_status = $true
    post_restart_status_evidence_path = [string]$task038Final.task051_post_restart_evidence_path
    post_restart_status_evidence_sha256 = [string]$task038Final.task051_post_restart_evidence_sha256
    exact_six_field_status = $true
    autonomy_receipt_verified = [bool]$task038Final.task051_autonomy_receipt_verified
    duplicate_effects = 0
    disposable_credential_source_removed = -not (Test-Path -LiteralPath $credentialSource)
    task038_final_path = $task038FinalPath
    task038_final_sha256 = Get-Task051Sha256 -Path $task038FinalPath
}
$finalPath = Join-Path $runRoot 'task051-final.json'
[IO.File]::WriteAllText($finalPath, (($final | ConvertTo-Json -Compress -Depth 10) + [char]10), [Text.UTF8Encoding]::new($false))
Write-Output ('TASK051_RECEIPT_PATH=' + $finalPath)
Write-Output ('TASK051_RECEIPT_SHA256=' + (Get-Task051Sha256 -Path $finalPath))
Write-Output 'TASK051_P0_PLATFORM_LIVE_ACCEPTANCE=PASS'
