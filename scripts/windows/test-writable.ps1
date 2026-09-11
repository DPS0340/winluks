param(
    [Parameter(Mandatory=$true)][string]$Manifest,
    [Parameter(Mandatory=$true)][ValidatePattern('^[A-Z]:\\$')][string]$Root,
    [Parameter(Mandatory=$true)][int]$DiskNumber,
    [Parameter(Mandatory=$true)][string]$Results,
    [switch]$VerifyOnly
)
$ErrorActionPreference='Stop'
if ((Get-CimInstance Win32_ComputerSystem).Model -notmatch 'QEMU|KVM|Standard PC') { throw 'Disposable QEMU VM required' }
$disk=Get-Disk -Number $DiskNumber
if ($disk.FriendlyName -notmatch '^WinSpd winluks RW' -or $disk.IsReadOnly) { throw 'Expected the writable winluks virtual fixture disk' }
Add-Type @'
using System;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class WinluksRwTest {
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)]
    static extern bool GetVolumeInformation(string root, IntPtr name,uint n,out uint serial,out uint max,out uint flags,IntPtr fs,uint f);
    [DllImport("kernel32.dll",SetLastError=true)]
    static extern bool DeviceIoControl(SafeFileHandle h,uint code,IntPtr input,uint insize,IntPtr output,uint outsize,out uint bytes,IntPtr overlapped);
    public static void CheckWritable(string root) {
        uint serial,max,flags;
        if(!GetVolumeInformation(root,IntPtr.Zero,0,out serial,out max,out flags,IntPtr.Zero,0))
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        if((flags&0x80000)!=0) throw new Exception("Filesystem is read-only");
    }
    static byte[] Pattern(int size,int seed) {
        byte[] data=new byte[size]; for(int i=0;i<size;i++) data[i]=(byte)(i*17+seed); return data;
    }
    public static void Payload(string path) {
        using(var f=new FileStream(path,FileMode.CreateNew,FileAccess.ReadWrite,FileShare.None,4096,FileOptions.WriteThrough)) {
            byte[] first=Pattern(600123,31), patch=Pattern(1025,91), tail=Pattern(256123,149);
            f.Write(first,0,first.Length); f.Position=511; f.Write(patch,0,patch.Length);
            f.Position=f.Length; f.Write(tail,0,tail.Length); f.SetLength(730111); f.Flush(true);
            f.Position=511; byte[] read=new byte[patch.Length];
            if(f.Read(read,0,read.Length)!=read.Length) throw new Exception("Short readback");
            for(int i=0;i<read.Length;i++) if(read[i]!=patch[i]) throw new Exception("Overwrite mismatch");
        }
        using(var f=File.OpenRead(path)) {
            if(f.Length!=730111) throw new Exception("Truncate mismatch");
            f.Position=600123;
            if(f.ReadByte()!=149) throw new Exception("Append mismatch");
        }
    }
    public static void Sparse(string path) {
        using(var f=new FileStream(path,FileMode.CreateNew,FileAccess.ReadWrite,FileShare.None)) {
            uint n;
            if(!DeviceIoControl(f.SafeFileHandle,0x900C4,IntPtr.Zero,0,IntPtr.Zero,0,out n,IntPtr.Zero))
                throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
            byte[] tail=Pattern(7,201);
            f.WriteByte(0x71); f.Position=32*1024*1024; f.Write(tail,0,tail.Length); f.Flush(true);
        }
    }
}
'@
[WinluksRwTest]::CheckWritable($Root)
New-Item -ItemType Directory -Force $Results | Out-Null
$expectedPath=Join-Path $Results 'expected.json'
if (!$VerifyOnly) {
    if (Test-Path -LiteralPath (Join-Path $Root 'rw-tests')) { throw 'Use a fresh disposable copy' }
    $m=Get-Content -Raw -Encoding UTF8 $Manifest | ConvertFrom-Json
    foreach ($file in $m.files.PSObject.Properties) {
        if ((Get-FileHash -LiteralPath (Join-Path $Root $file.Name)).Hash -ne $file.Value.sha256) { throw 'Initial file hash mismatch' }
    }
    New-Item -ItemType Directory (Join-Path $Root 'rw-tests\staging') | Out-Null
    [WinluksRwTest]::Payload((Join-Path $Root 'rw-tests\staging\payload.bin'))
    [WinluksRwTest]::Sparse((Join-Path $Root 'rw-tests\staging\sparse.bin'))
    [IO.File]::WriteAllText((Join-Path $Root 'rw-tests\staging\한글-파일.txt'),'Windows RW 검증',[Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath (Join-Path $Root 'rw-tests\staging') -Destination (Join-Path $Root 'rw-tests\done')
    Copy-Item -LiteralPath (Join-Path $Root 'rw-tests\done\payload.bin') -Destination (Join-Path $Root 'rw-tests\copy.bin')
    if ((Get-FileHash -LiteralPath (Join-Path $Root 'rw-tests\copy.bin')).Hash -ne (Get-FileHash -LiteralPath (Join-Path $Root 'rw-tests\done\payload.bin')).Hash) { throw 'Copy mismatch' }
    Remove-Item -LiteralPath (Join-Path $Root 'rw-tests\copy.bin')
    Move-Item -LiteralPath (Join-Path $Root 'hello.txt') -Destination (Join-Path $Root 'hello-renamed.txt')
    [IO.File]::AppendAllText((Join-Path $Root 'hello-renamed.txt'),"RW append`n",[Text.UTF8Encoding]::new($false))
    Remove-Item -LiteralPath (Join-Path $Root 'empty')
    $files=@{}
    foreach($file in $m.files.PSObject.Properties) {
        if ($file.Name -notin @('hello.txt','empty')) { $files[$file.Name]=$file.Value }
    }
    foreach($name in @('hello-renamed.txt','rw-tests/done/payload.bin','rw-tests/done/sparse.bin','rw-tests/done/한글-파일.txt')) {
        $p=Join-Path $Root $name
        $files[$name]=@{sha256=(Get-FileHash -LiteralPath $p).Hash.ToLowerInvariant();size=(Get-Item -LiteralPath $p).Length}
    }
    @{fixture=$m.name;files=$files;absent=@('hello.txt','empty','rw-tests/staging','rw-tests/copy.bin')} | ConvertTo-Json -Depth 6 | Set-Content $expectedPath -Encoding UTF8
}
$expected=Get-Content $expectedPath -Raw -Encoding UTF8 | ConvertFrom-Json
foreach($file in $expected.files.PSObject.Properties) {
    $path=Join-Path $Root $file.Name
    if ((Get-FileHash -LiteralPath $path).Hash -ne $file.Value.sha256 -or (Get-Item -LiteralPath $path).Length -ne $file.Value.size) { throw 'Written file hash or size mismatch' }
}
foreach($name in $expected.absent) { if(Test-Path -LiteralPath (Join-Path $Root $name)) { throw 'Deleted/renamed path remains' } }
$result=@{passed=$true;filesystem_writable=$true;disk_writable=$true;file_count=@($expected.files.PSObject.Properties).Count;phase= $(if($VerifyOnly){'reopen'}else{'write'});operations=@('create','unaligned-overwrite','append','truncate','sparse','unicode','copy','rename','delete','readback')}
$result | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $Results ($result.phase+'.json')) -Encoding UTF8
$result | ConvertTo-Json -Depth 4
