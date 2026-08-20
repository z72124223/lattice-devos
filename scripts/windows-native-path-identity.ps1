Set-StrictMode -Version Latest

function Initialize-LatticeWindowsNativePathIdentity {
    if ($null -ne ('Lattice.P0.WindowsNativePathIdentity' -as [type])) {
        return
    }

    Add-Type -Language CSharp -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Globalization;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace Lattice.P0
{
    [StructLayout(LayoutKind.Sequential)]
    internal struct LatticeFileId128
    {
        internal ulong Low;
        internal ulong High;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct LatticeFileIdInfo
    {
        internal ulong VolumeSerialNumber;
        internal LatticeFileId128 FileId;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct LatticeFileAttributeTagInfo
    {
        internal uint FileAttributes;
        internal uint ReparseTag;
    }

    public static class WindowsNativePathIdentity
    {
        private const uint FileReadAttributes = 0x00000080;
        private const uint FileShareRead = 0x00000001;
        private const uint FileShareWrite = 0x00000002;
        private const uint FileShareDelete = 0x00000004;
        private const uint OpenExisting = 3;
        private const uint FileFlagBackupSemantics = 0x02000000;
        private const uint FileFlagOpenReparsePoint = 0x00200000;
        private const uint FileAttributeDirectory = 0x00000010;
        private const uint FileAttributeReparsePoint = 0x00000400;
        private const int FileAttributeTagInfoClass = 9;
        private const int FileIdInfoClass = 18;

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern SafeFileHandle CreateFileW(
            string fileName,
            uint desiredAccess,
            uint shareMode,
            IntPtr securityAttributes,
            uint creationDisposition,
            uint flagsAndAttributes,
            IntPtr templateFile);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetFileInformationByHandleEx(
            SafeFileHandle file,
            int informationClass,
            out LatticeFileIdInfo information,
            uint bufferSize);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetFileInformationByHandleEx(
            SafeFileHandle file,
            int informationClass,
            out LatticeFileAttributeTagInfo information,
            uint bufferSize);

        public static string Capture(string path, bool expectedDirectory)
        {
            if (String.IsNullOrWhiteSpace(path))
            {
                throw new ArgumentException("A native identity path is required.", "path");
            }

            using (SafeFileHandle handle = CreateFileW(
                path,
                FileReadAttributes,
                FileShareRead | FileShareWrite | FileShareDelete,
                IntPtr.Zero,
                OpenExisting,
                FileFlagBackupSemantics | FileFlagOpenReparsePoint,
                IntPtr.Zero))
            {
                if (handle.IsInvalid)
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }

                LatticeFileAttributeTagInfo attributes;
                if (!GetFileInformationByHandleEx(
                    handle,
                    FileAttributeTagInfoClass,
                    out attributes,
                    (uint)Marshal.SizeOf(typeof(LatticeFileAttributeTagInfo))))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }

                bool isDirectory = (attributes.FileAttributes & FileAttributeDirectory) != 0;
                bool isReparsePoint = (attributes.FileAttributes & FileAttributeReparsePoint) != 0;
                if (isReparsePoint || isDirectory != expectedDirectory)
                {
                    throw new InvalidOperationException("Native path type or reparse-point gate rejected the object.");
                }

                LatticeFileIdInfo identity;
                if (!GetFileInformationByHandleEx(
                    handle,
                    FileIdInfoClass,
                    out identity,
                    (uint)Marshal.SizeOf(typeof(LatticeFileIdInfo))))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }

                return String.Format(
                    CultureInfo.InvariantCulture,
                    "lattice.win-file-id.v1:{0:x16}:{1:x16}{2:x16}:{3}",
                    identity.VolumeSerialNumber,
                    identity.FileId.High,
                    identity.FileId.Low,
                    isDirectory ? "d" : "f");
            }
        }

        public static bool Matches(string path, bool expectedDirectory, string expectedToken)
        {
            if (String.IsNullOrWhiteSpace(expectedToken))
            {
                return false;
            }

            try
            {
                return String.Equals(
                    Capture(path, expectedDirectory),
                    expectedToken,
                    StringComparison.Ordinal);
            }
            catch
            {
                return false;
            }
        }
    }
}
'@
}

function Get-LatticeWindowsNativePathIdentityToken {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][bool]$Directory
    )

    Initialize-LatticeWindowsNativePathIdentity
    $fullPath = [IO.Path]::GetFullPath($Path)
    return [Lattice.P0.WindowsNativePathIdentity]::Capture($fullPath, $Directory)
}

function Test-LatticeWindowsNativePathIdentity {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][bool]$Directory,
        [Parameter(Mandatory = $true)][string]$ExpectedToken
    )

    Initialize-LatticeWindowsNativePathIdentity
    $fullPath = [IO.Path]::GetFullPath($Path)
    return [Lattice.P0.WindowsNativePathIdentity]::Matches($fullPath, $Directory, $ExpectedToken)
}

function New-LatticeWindowsNativeContainmentSnapshot {
    param(
        [Parameter(Mandatory = $true)][string]$ParentPath,
        [Parameter(Mandatory = $true)][string]$RootPath,
        [Parameter(Mandatory = $true)][string]$MarkerPath
    )

    $parent = [IO.Path]::GetFullPath($ParentPath).TrimEnd('\')
    $root = [IO.Path]::GetFullPath($RootPath).TrimEnd('\')
    $marker = [IO.Path]::GetFullPath($MarkerPath)
    if (
        -not [string]::Equals([IO.Path]::GetDirectoryName($root).TrimEnd('\'), $parent, [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals([IO.Path]::GetDirectoryName($marker).TrimEnd('\'), $root, [StringComparison]::OrdinalIgnoreCase)
    ) {
        throw 'LATTICE_WINDOWS_NATIVE_CONTAINMENT_RELATION_REJECTED'
    }

    $parentIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $parent -Directory $true
    $rootIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $root -Directory $true
    $markerIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $marker -Directory $false
    $tokenPattern = '^lattice\.win-file-id\.v1:(?<volume>[0-9a-f]{16}):[0-9a-f]{32}:[df]$'
    $parentMatch = [regex]::Match($parentIdentity, $tokenPattern)
    $rootMatch = [regex]::Match($rootIdentity, $tokenPattern)
    $markerMatch = [regex]::Match($markerIdentity, $tokenPattern)
    if (
        -not $parentMatch.Success -or
        -not $rootMatch.Success -or
        -not $markerMatch.Success -or
        $parentMatch.Groups['volume'].Value -cne $rootMatch.Groups['volume'].Value -or
        $parentMatch.Groups['volume'].Value -cne $markerMatch.Groups['volume'].Value
    ) {
        throw 'LATTICE_WINDOWS_NATIVE_CONTAINMENT_VOLUME_REJECTED'
    }

    return [pscustomobject][ordered]@{
        schema = 'lattice.windows-native-containment.v1'
        parent_path = $parent
        parent_identity = $parentIdentity
        root_path = $root
        root_identity = $rootIdentity
        marker_path = $marker
        marker_identity = $markerIdentity
    }
}

function Test-LatticeWindowsNativeContainmentSnapshot {
    param([Parameter(Mandatory = $true)]$Snapshot)

    try {
        if ($null -eq $Snapshot -or [string]$Snapshot.schema -cne 'lattice.windows-native-containment.v1') {
            return $false
        }

        $parent = [IO.Path]::GetFullPath([string]$Snapshot.parent_path).TrimEnd('\')
        $root = [IO.Path]::GetFullPath([string]$Snapshot.root_path).TrimEnd('\')
        $marker = [IO.Path]::GetFullPath([string]$Snapshot.marker_path)
        if (
            -not [string]::Equals([IO.Path]::GetDirectoryName($root).TrimEnd('\'), $parent, [StringComparison]::OrdinalIgnoreCase) -or
            -not [string]::Equals([IO.Path]::GetDirectoryName($marker).TrimEnd('\'), $root, [StringComparison]::OrdinalIgnoreCase)
        ) {
            return $false
        }

        return (
            (Test-LatticeWindowsNativePathIdentity -Path $parent -Directory $true -ExpectedToken ([string]$Snapshot.parent_identity)) -and
            (Test-LatticeWindowsNativePathIdentity -Path $root -Directory $true -ExpectedToken ([string]$Snapshot.root_identity)) -and
            (Test-LatticeWindowsNativePathIdentity -Path $marker -Directory $false -ExpectedToken ([string]$Snapshot.marker_identity))
        )
    }
    catch {
        return $false
    }
}

function Assert-LatticeWindowsNativeContainmentSnapshot {
    param(
        [Parameter(Mandatory = $true)]$Snapshot,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    if (-not (Test-LatticeWindowsNativeContainmentSnapshot -Snapshot $Snapshot)) {
        throw $FailureCode
    }
}
