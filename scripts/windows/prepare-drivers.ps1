param(
    [Parameter(Mandatory=$true)][ValidateSet('btrfs','ext4')][string]$Filesystem,
    [string]$LabDirectory = 'C:\winluks-lab',
    [switch]$TrustPinnedPublishers
)
$ErrorActionPreference = 'Stop'
$admin = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (!$admin.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Administrator required' }
$model = (Get-CimInstance Win32_ComputerSystem).Model
if ($model -notmatch 'QEMU|KVM|Standard PC') { throw 'Use a disposable QEMU/KVM test VM' }
New-Item -ItemType Directory -Force $LabDirectory | Out-Null
function Trust-Catalog($Path, $Hash, $Thumbprint) {
    if ((Get-FileHash $Path).Hash -ne $Hash) { throw 'Catalog hash mismatch' }
    $sig = Get-AuthenticodeSignature $Path
    if ($sig.Status -ne 'Valid' -or $sig.SignerCertificate.Thumbprint -ne $Thumbprint) { throw 'Catalog signer mismatch' }
    $cert = Join-Path $LabDirectory ($Thumbprint+'.cer')
    Export-Certificate -Cert $sig.SignerCertificate -FilePath $cert | Out-Null
    # -f creates an empty TrustedPublisher store on a fresh Windows installation.
    certutil.exe -f -addstore TrustedPublisher $cert | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Pinned publisher enrollment failed' }
}
$packages = @{
    vcruntime = @('https://aka.ms/vc14/vc_redist.x64.exe','vc_redist.x64.exe','843068991daaa1f73ad9f6239bce4d0f6a07a51f18c37ea2a867e9beca71295c')
    winspd = @('https://github.com/winfsp/winspd/releases/download/v1.0B1/winspd-1.0.20357.msi','winspd.msi','f1157eef805dcbec78a477f2b4ee5abc0049c8a9329444e5d18cab01d3604265')
    btrfs = @('https://github.com/maharmstone/btrfs/releases/download/v1.10/btrfs-1.10.zip','btrfs.zip','82303494b4fd4c23ad7c6fd69a886cb1a317a0293ee6e283b7c009117133b0c3')
    ext4 = @('https://github.com/bobranten/Ext4Fsd/releases/download/v0.71/Ext2Fsd-0.71-setup.exe','ext4fsd.exe','3c127f8e70c6b056a0185850efb71b9c45d2ff493b7df6b7b648eb89ec84214d')
}
foreach ($name in @('vcruntime','winspd',$Filesystem)) {
    $pkg = $packages[$name]; $path = Join-Path $LabDirectory $pkg[1]
    if (!(Test-Path $path)) { Invoke-WebRequest $pkg[0] -OutFile $path }
    if ((Get-FileHash $path -Algorithm SHA256).Hash -ne $pkg[2]) { throw "Package hash mismatch: $name" }
}
$runtime = Join-Path $LabDirectory 'vc_redist.x64.exe'
if ((Get-AuthenticodeSignature $runtime).Status -ne 'Valid') { throw 'Microsoft runtime signature is not valid' }
$proc = Start-Process $runtime -ArgumentList '/install /quiet /norestart' -Wait -PassThru
if ($proc.ExitCode -notin @(0,3010,1638)) { throw "Visual C++ runtime install error $($proc.ExitCode)" }
$msi = Join-Path $LabDirectory 'winspd.msi'
if ((Get-AuthenticodeSignature $msi).Status -ne 'Valid') { throw 'WinSpd MSI signature is not valid' }
if ($TrustPinnedPublishers) {
    $extract = Join-Path $LabDirectory 'winspd-admin'
    New-Item -ItemType Directory -Force $extract | Out-Null
    $proc = Start-Process msiexec.exe -ArgumentList "/a `"$msi`" /qn TARGETDIR=`"$extract`"" -Wait -PassThru
    if ($proc.ExitCode -ne 0) { throw 'WinSpd administrative extraction failed' }
    $catalogs = @(Get-ChildItem $extract -Recurse -Filter winspd-x64.cat)
    if ($catalogs.Count -ne 1) { throw 'WinSpd catalog not uniquely located' }
    Trust-Catalog $catalogs[0].FullName 'bcaff78f0801e64e11b39f2bf790e24b25c88251d7bdb9bc40c245b9a3d0cf93' '1352B76EF1884FBBC9E41381A9CFAA225DB75FC6'
}
$proc = Start-Process msiexec.exe -ArgumentList "/i `"$msi`" /qn /norestart /l*v `"$LabDirectory\winspd-install.log`"" -Wait -PassThru
if ($proc.ExitCode -notin @(0,3010)) { throw "WinSpd install error $($proc.ExitCode)" }
if ($Filesystem -eq 'btrfs') {
    $dir = Join-Path $LabDirectory 'btrfs'
    Expand-Archive (Join-Path $LabDirectory 'btrfs.zip') $dir -Force
    if ((Get-AuthenticodeSignature "$dir\amd64\btrfs.sys").Status -ne 'Valid') { throw 'WinBtrfs signature is not valid' }
    if ($TrustPinnedPublishers) { Trust-Catalog "$dir\btrfs.cat" 'e185aee58499d2180d194c25fa38e2d795c8792f177b47a2e7311aae53213f4a' '76071A48C4071173E0593533F0C0C7AD5BA11530' }
    pnputil.exe /add-driver "$dir\btrfs.inf" /install
    if ($LASTEXITCODE -notin @(0,3010)) { throw 'WinBtrfs driver package install failed' }
    if (!(Test-Path HKLM:\SYSTEM\CurrentControlSet\Services\btrfs)) { throw 'WinBtrfs service was not installed' }
    New-ItemProperty HKLM:\SYSTEM\CurrentControlSet\Services\btrfs -Name Readonly -PropertyType DWord -Value 1 -Force | Out-Null
} else {
    $exe = Join-Path $LabDirectory 'ext4fsd.exe'
    $driver = Join-Path $env:ProgramFiles 'Ext2Fsd\Ext2Fsd.sys'
    # NSIS installer from the pinned Ext4Fsd release (not an unrelated Ext2Fsd release).
    # Re-running its helper on an existing service can display a modal dialog.
    if (!(Test-Path $driver)) {
        $proc = Start-Process $exe -ArgumentList '/S' -Wait -PassThru
        if ($proc.ExitCode -ne 0) { throw "Ext4Fsd install error $($proc.ExitCode)" }
    }
    # The pinned NSIS package references a catalog it does not include. Register the
    # unchanged, Microsoft-signed filesystem driver through SCM; signature enforcement
    # still applies when Windows loads it. Do not modify the INF or the driver.
    if ((Get-FileHash $driver).Hash -ne '06f6b4a6bc7aaf568d0442a3415394b2b7806c1bd94d35b314bebfa0993898d9') { throw 'Ext4Fsd driver hash mismatch' }
    if ((Get-AuthenticodeSignature $driver).Status -ne 'Valid') { throw 'Ext4Fsd driver signature is not valid' }
    if (!(Get-Service Ext2Fsd -ErrorAction SilentlyContinue)) {
        Copy-Item $driver "$env:windir\System32\drivers\Ext2Fsd.sys"
        sc.exe create Ext2Fsd type= filesys start= system error= normal binPath= '\SystemRoot\System32\drivers\Ext2Fsd.sys'
        if ($LASTEXITCODE -ne 0) { throw 'Ext4Fsd service registration failed' }
    }
    $params = 'HKLM:\SYSTEM\CurrentControlSet\Services\Ext2Fsd\Parameters'
    New-Item -Force $params | Out-Null
    New-ItemProperty $params -Name AutoMount -PropertyType DWord -Value 1 -Force | Out-Null
    New-ItemProperty $params -Name CheckingBitmap -PropertyType DWord -Value 0 -Force | Out-Null
    foreach ($name in @('WritingSupport','Ext3ForceWriting')) { New-ItemProperty $params -Name $name -PropertyType DWord -Value 0 -Force | Out-Null }
    New-ItemProperty $params -Name CodePage -PropertyType String -Value 'utf8' -Force | Out-Null
    New-ItemProperty $params -Name Readonly -PropertyType DWord -Value 1 -Force | Out-Null
}
@{
    os = (Get-CimInstance Win32_OperatingSystem).Version
    secure_boot = (Confirm-SecureBootUEFI)
    device_guard = (Get-CimInstance -Namespace root\Microsoft\Windows\DeviceGuard -ClassName Win32_DeviceGuard | Select-Object SecurityServicesConfigured,SecurityServicesRunning)
    filesystem = $Filesystem
    reboot_required = $true
} | ConvertTo-Json -Depth 4 | Set-Content "$LabDirectory\driver-setup.json"
Write-Output 'Reboot the test VM before publishing any fixture. Installation is not a passed G0 gate.'
