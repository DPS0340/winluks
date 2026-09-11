param(
    [Parameter(Mandatory=$true)][ValidateSet('btrfs','ext4')][string]$Filesystem,
    [string]$LabDirectory = 'C:\winluks-lab'
)
$ErrorActionPreference = 'Stop'
$admin = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (!$admin.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Administrator required' }
$model = (Get-CimInstance Win32_ComputerSystem).Model
if ($model -notmatch 'QEMU|KVM|Standard PC') { throw 'Use a disposable QEMU/KVM test VM' }
New-Item -ItemType Directory -Force $LabDirectory | Out-Null
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
$proc = Start-Process msiexec.exe -ArgumentList "/i `"$msi`" /qn /norestart /l*v `"$LabDirectory\winspd-install.log`"" -Wait -PassThru
if ($proc.ExitCode -notin @(0,3010)) { throw "WinSpd install error $($proc.ExitCode)" }
if ($Filesystem -eq 'btrfs') {
    $dir = Join-Path $LabDirectory 'btrfs'
    Expand-Archive (Join-Path $LabDirectory 'btrfs.zip') $dir -Force
    if ((Get-AuthenticodeSignature "$dir\amd64\btrfs.sys").Status -ne 'Valid') { throw 'WinBtrfs signature is not valid' }
    Start-Process rundll32.exe -ArgumentList "setupapi.dll,InstallHinfSection DefaultInstall 132 `"$dir\btrfs.inf`"" -Wait
    if (!(Test-Path HKLM:\SYSTEM\CurrentControlSet\Services\btrfs)) { throw 'WinBtrfs service was not installed' }
    New-ItemProperty HKLM:\SYSTEM\CurrentControlSet\Services\btrfs -Name Readonly -PropertyType DWord -Value 1 -Force | Out-Null
} else {
    $exe = Join-Path $LabDirectory 'ext4fsd.exe'
    # NSIS installer from the pinned Ext4Fsd release (not an unrelated Ext2Fsd release).
    $proc = Start-Process $exe -ArgumentList '/S' -Wait -PassThru
    if ($proc.ExitCode -ne 0) { throw "Ext4Fsd install error $($proc.ExitCode)" }
    $params = 'HKLM:\SYSTEM\CurrentControlSet\Services\Ext2Fsd\Parameters'
    New-Item -Force $params | Out-Null
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
