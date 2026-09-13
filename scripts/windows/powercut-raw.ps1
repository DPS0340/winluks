param([Parameter(Mandatory=$true)][ValidatePattern('^[A-Z]:\\$')][string]$Root,[Parameter(Mandatory=$true)][int]$DiskNumber)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
if ((Get-CimInstance Win32_ComputerSystem).Model -notmatch 'QEMU|KVM|Standard PC') { throw 'Disposable QEMU VM required' }
$disk=Get-Disk -Number $DiskNumber
if ($disk.FriendlyName -notmatch '^WinSpd winluks RW' -or $disk.IsReadOnly) { throw 'Expected writable winluks fixture disk' }
Add-Type @'
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using Microsoft.Win32.SafeHandles;
public static class RawCut {
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern SafeFileHandle CreateFile(string p,uint access,uint share,IntPtr sa,uint creation,uint flags,IntPtr template);
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool DeviceIoControl(SafeFileHandle h,uint code,IntPtr input,uint ni,IntPtr output,uint no,out uint n,IntPtr ov);
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool WriteFile(SafeFileHandle h,IntPtr bytes,uint length,out uint n,IntPtr ov);
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool ReadFile(SafeFileHandle h,IntPtr bytes,uint length,out uint n,IntPtr ov);
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool FlushFileBuffers(SafeFileHandle h);
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool SetFilePointerEx(SafeFileHandle h,long offset,out long result,uint method);
 [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr VirtualAlloc(IntPtr addr,UIntPtr size,uint allocation,uint protect);
 [DllImport("kernel32.dll")] static extern bool VirtualFree(IntPtr addr,UIntPtr size,uint free);
 static void Check(bool ok) { if(!ok)throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error()); }
 public static void Run(string root,int disk,long size) {
  if(File.Exists(@"C:\winluks-lab\powercut\raw-go")) throw new Exception("Fresh trial without a pre-existing go marker required");
  using(var volume=CreateFile("\\\\.\\"+root.Substring(0,2),0xc0000000,3,IntPtr.Zero,3,0,IntPtr.Zero)) {
   Check(!volume.IsInvalid);uint n;
   Check(DeviceIoControl(volume,0x90018,IntPtr.Zero,0,IntPtr.Zero,0,out n,IntPtr.Zero));
   Check(DeviceIoControl(volume,0x90020,IntPtr.Zero,0,IntPtr.Zero,0,out n,IntPtr.Zero));
   using(var raw=CreateFile("\\\\.\\PhysicalDrive"+disk,0xc0000000,3,IntPtr.Zero,3,0xa0000000,IntPtr.Zero)) {
    Check(!raw.IsInvalid);
    IntPtr memory=VirtualAlloc(IntPtr.Zero,(UIntPtr)(1024*1024),0x3000,4);Check(memory!=IntPtr.Zero);
    try {
     // Locking WinBtrfs can commit filesystem metadata. Capture the complete state
     // AFTER that commit and BEFORE the tested writes, then authenticate it in the
     // host-observed event stream. The Linux oracle decrypts the post-cut image.
     string baseline=@"C:\winluks-lab\powercut\raw-before.bin";
     using(var output=new FileStream(baseline,FileMode.CreateNew,FileAccess.Write,FileShare.None,1024*1024,FileOptions.WriteThrough)) {
      for(long offset=0;offset<size;) {
       uint count=(uint)Math.Min(1024*1024,size-offset);Check(ReadFile(raw,memory,count,out n,IntPtr.Zero));
       if(n!=count)throw new Exception("Short baseline read");
       byte[] bytes=new byte[count];Marshal.Copy(memory,bytes,0,bytes.Length);output.Write(bytes,0,bytes.Length);offset+=count;
      }
      output.Flush(true);
     }
     using(var input=File.OpenRead(baseline)) using(var hash=SHA256.Create()) {
      string digest=BitConverter.ToString(hash.ComputeHash(input)).Replace("-","").ToLowerInvariant();
      Console.WriteLine("EVENT {\"kind\":\"raw-baseline\",\"bytes\":"+size+",\"sha256\":\""+digest+"\"}");Console.Out.Flush();
     }
     // The host copies and authenticates the baseline before allowing any writes.
     // A newly created NTFS directory entry may be absent in a no-replay crash view.
     while(!File.Exists(@"C:\winluks-lab\powercut\raw-go")) System.Threading.Thread.Sleep(50);
     long[] offsets={0,128*1024,size-512};int[] lengths={512,1024*1024,512};
     for(int j=0;j<3;j++) {
      int seed=83+29*j;byte[] bytes=new byte[lengths[j]];
      for(int i=0;i<bytes.Length;i++)bytes[i]=(byte)(17*i+seed);
      Marshal.Copy(bytes,0,memory,bytes.Length);long actual;
      Check(SetFilePointerEx(raw,offsets[j],out actual,0));if(actual!=offsets[j])throw new Exception("Seek mismatch");
      Check(WriteFile(raw,memory,(uint)bytes.Length,out n,IntPtr.Zero));if(n!=bytes.Length)throw new Exception("Short write");
      Check(FlushFileBuffers(raw));
      Console.WriteLine("EVENT {\"kind\":\"block-flushed\",\"offset\":"+offsets[j]+",\"length\":"+bytes.Length+",\"seed\":"+seed+"}");Console.Out.Flush();
     }
     Console.WriteLine("EVENT {\"kind\":\"workload-complete\"}");Console.Out.Flush();
     // Keep the FS dismounted and locked until the host cuts this disposable VM.
     System.Threading.Thread.Sleep(System.Threading.Timeout.Infinite);
    } finally { VirtualFree(memory,UIntPtr.Zero,0x8000); }
   }
  }
 }
}
'@
[RawCut]::Run($Root,$DiskNumber,$disk.Size)
