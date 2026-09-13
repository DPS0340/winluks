param(
    [Parameter(Mandatory=$true)][ValidatePattern('^[A-Z]:\\$')][string]$Root,
    [Parameter(Mandatory=$true)][int]$DiskNumber,
    [ValidateSet('flushed','stream','fill')][string]$Mode='flushed'
)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
if ((Get-CimInstance Win32_ComputerSystem).Model -notmatch 'QEMU|KVM|Standard PC') { throw 'Disposable QEMU VM required' }
$disk=Get-Disk -Number $DiskNumber
if ($disk.FriendlyName -notmatch '^WinSpd winluks RW' -or $disk.IsReadOnly) { throw 'Expected writable winluks fixture disk' }
if (Test-Path -LiteralPath (Join-Path $Root 'powercut')) { throw 'Use a fresh fixture copy' }
New-Item -ItemType Directory (Join-Path $Root 'powercut') | Out-Null
Add-Type @'
using System;
using System.IO;
using System.Security.Cryptography;
public static class PowercutWorkload {
    public static void Run(string root, string mode) {
        int count = mode == "fill" ? 1024 : (mode == "stream" ? 96 : 4);
        byte[] bytes = new byte[1024*1024];
        for (int n=0;n<count;n++) {
            for(int i=0;i<bytes.Length;i++) bytes[i]=(byte)(i*17+n*29+83);
            string relative="powercut/record-"+n.ToString("D3")+".bin";
            using (var f=new FileStream(Path.Combine(root,relative),FileMode.CreateNew,FileAccess.Write,FileShare.Read,4096,FileOptions.WriteThrough)) {
                f.Write(bytes,0,bytes.Length);
                if(mode != "stream") f.Flush(true);
            }
            string hash;
            using(var sha=SHA256.Create()) hash=BitConverter.ToString(sha.ComputeHash(bytes)).Replace("-","").ToLowerInvariant();
            Console.WriteLine("EVENT {\"kind\":\"file-"+(mode=="stream"?"written":"flushed")+"\",\"path\":\""+relative+"\",\"size\":"+bytes.Length+",\"sha256\":\""+hash+"\"}");
            Console.Out.Flush();
        }
        Console.WriteLine("EVENT {\"kind\":\"workload-complete\"}");
        Console.Out.Flush();
    }
}
'@
try { [PowercutWorkload]::Run($Root,$Mode) }
catch {
    $errorObject=$_.Exception
    while($errorObject.InnerException) { $errorObject=$errorObject.InnerException }
    Write-Output ('EVENT '+(@{kind='workload-error';hresult=$errorObject.HResult} | ConvertTo-Json -Compress))
    exit 1
}
