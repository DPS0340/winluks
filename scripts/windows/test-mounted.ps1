param(
    [Parameter(Mandatory=$true)][string]$Manifest,
    [Parameter(Mandatory=$true)][ValidatePattern('^[A-Z]:\\$')][string]$Root,
    [Parameter(Mandatory=$true)][int]$DiskNumber,
    [Parameter(Mandatory=$true)][string]$Results
)
$ErrorActionPreference='Stop'
$admin=[Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (!$admin.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Administrator required for mutation tests' }
if ((Get-CimInstance Win32_ComputerSystem).Model -notmatch 'QEMU|KVM|Standard PC') { throw 'Disposable QEMU VM required' }
$disk=Get-Disk -Number $DiskNumber
if ($disk.FriendlyName -notmatch '^WinSpd winluks RO' -or !$disk.IsReadOnly) { throw 'Expected the published read-only winluks fixture disk' }
$fixture=Get-Content -Raw -Encoding UTF8 $Manifest | ConvertFrom-Json
if (!( 'WinluksTestNative' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class WinluksTestNative {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool GetVolumeInformation(string root, IntPtr name, uint namesize,
        out uint serial, out uint max, out uint flags, IntPtr fs, uint fssize);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern SafeFileHandle CreateFile(string path, uint access, uint sharing,
        IntPtr security, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool WriteFile(SafeFileHandle handle, byte[] data, uint count,
        out uint written, IntPtr overlapped);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool ReadFile(SafeFileHandle handle, byte[] data, uint count,
        out uint read, IntPtr overlapped);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool SetFilePointerEx(SafeFileHandle handle, long distance, out long position, uint method);
    public static uint VolumeFlags(string root) {
        uint serial, max, flags;
        if (!GetVolumeInformation(root, IntPtr.Zero, 0, out serial, out max, out flags, IntPtr.Zero, 0))
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        return flags;
    }
    public static int RawWriteError(int disk) {
        using (var h=CreateFile(@"\\.\PhysicalDrive"+disk, 0xC0000000, 3, IntPtr.Zero, 3, 0, IntPtr.Zero)) {
            if (h.IsInvalid) {
                int openError=Marshal.GetLastWin32Error();
                if (openError==5 || openError==19) return openError;
                throw new System.ComponentModel.Win32Exception(openError);
            }
            uint transferred; long position;
            byte[] before=new byte[512], changed=new byte[512], after=new byte[512];
            if (!ReadFile(h,before,512,out transferred,IntPtr.Zero) || transferred!=512)
                throw new Exception("Raw read control failed");
            for(int i=0;i<512;i++) changed[i]=(byte)(before[i]^0x5a);
            if (!SetFilePointerEx(h,0,out position,0)) throw new Exception("Raw seek control failed");
            if (WriteFile(h, changed, 512, out transferred, IntPtr.Zero)) return 0;
            int error=Marshal.GetLastWin32Error();
            if (!SetFilePointerEx(h,0,out position,0) ||
                !ReadFile(h,after,512,out transferred,IntPtr.Zero) || transferred!=512)
                throw new Exception("Raw readback control failed");
            for(int i=0;i<512;i++) if(before[i]!=after[i]) throw new Exception("Raw data changed");
            return error;
        }
    }
}
'@
}
New-Item -ItemType Directory -Force $Results | Out-Null
$report=@{fixture=$fixture.name; disk_read_only=$disk.IsReadOnly; volume_read_only=$false; file_hashes_match=$false; mutation_errors=@{}; passed=$false}
try {
    $report.volume_read_only=([WinluksTestNative]::VolumeFlags($Root) -band 0x80000) -ne 0
    if (!$report.volume_read_only) { throw 'Filesystem does not advertise FILE_READ_ONLY_VOLUME' }
    foreach ($file in $fixture.files.PSObject.Properties) {
        $path=Join-Path $Root $file.Name
        if ((Get-FileHash -LiteralPath $path).Hash -ne $file.Value.sha256) { throw 'Filesystem file hash mismatch' }
        $destination=Join-Path "$Results\copied" $file.Name
        New-Item -ItemType Directory -Force (Split-Path -Parent $destination) | Out-Null
        Copy-Item -LiteralPath $path -Destination $destination
        if ((Get-FileHash -LiteralPath $destination).Hash -ne $file.Value.sha256) { throw 'Copied file hash mismatch' }
    }
    $report.file_hashes_match=$true
    $operations=@{
        create={ [IO.File]::WriteAllBytes((Join-Path $Root 'write-must-fail.bin'),[byte[]](1,2,3)) }
        rename={ Move-Item -LiteralPath (Join-Path $Root 'hello.txt') -Destination (Join-Path $Root 'renamed-must-fail.txt') }
        delete={ Remove-Item -LiteralPath (Join-Path $Root 'empty') }
    }
    foreach($name in $operations.Keys) {
        $errorCode=$null
        try { & $operations[$name] } catch { $errorCode=$_.Exception.HResult }
        if ($null -eq $errorCode) { throw "Unexpected filesystem mutation success: $name" }
        $report.mutation_errors[$name]=$errorCode
    }
    $rawError=[WinluksTestNative]::RawWriteError($DiskNumber)
    $report.mutation_errors.raw_write=$rawError
    # WinSpd's kernel write-protect rejection can surface as ERROR_IO_DEVICE (1117).
    # Successful raw reads before/after and unchanged bytes rule out an invalid read target.
    if ($rawError -notin @(5,19,1117)) { throw "Raw write returned unexpected status: $rawError" }
    $report.passed=$true
} finally {
    $report | ConvertTo-Json -Depth 5 | Set-Content "$Results\mounted.json"
}
