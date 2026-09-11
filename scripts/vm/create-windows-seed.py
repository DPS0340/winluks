#!/usr/bin/env python3
"""Create PRIVATE unattended-setup media for a disposable, empty Windows VM.

Requires pycdlib. The generated ISO contains a synthetic local administrator password.
Attach only to a fresh VM whose disk 0 is an empty file-backed disposable disk.
Never commit the generated files. Windows licensing/evaluation terms apply.
"""
import argparse
import io
from pathlib import Path
import secrets
import string
import pycdlib

ap=argparse.ArgumentParser(description=__doc__)
ap.add_argument('--output',type=Path,required=True)
ap.add_argument('--ssh-public-key',type=Path,required=True)
a=ap.parse_args()
a.output.mkdir(parents=True,mode=0o700,exist_ok=True)
if any((a.output/name).exists() for name in ('windows-password.key','windows-seed.iso')):
    ap.error('refusing to overwrite existing private setup credentials or media')
password='WL-'+''.join(secrets.choice(string.ascii_letters+string.digits) for _ in range(28))+'!'
import os
with os.fdopen(os.open(a.output/'windows-password.key',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600),'w') as f:
    f.write(password)
xml=fr'''<?xml version="1.0" encoding="utf-8"?>
<unattend xmlns="urn:schemas-microsoft-com:unattend">
 <settings pass="windowsPE">
  <component name="Microsoft-Windows-International-Core-WinPE" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS">
   <SetupUILanguage><UILanguage>en-US</UILanguage></SetupUILanguage><InputLocale>en-US</InputLocale><SystemLocale>en-US</SystemLocale><UILanguage>en-US</UILanguage><UserLocale>en-US</UserLocale>
  </component>
  <component name="Microsoft-Windows-Setup" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS" xmlns:wcm="http://schemas.microsoft.com/WMIConfig/2002/State">
   <DiskConfiguration><Disk wcm:action="add"><DiskID>0</DiskID><WillWipeDisk>true</WillWipeDisk>
    <CreatePartitions>
     <CreatePartition wcm:action="add"><Order>1</Order><Type>EFI</Type><Size>260</Size></CreatePartition>
     <CreatePartition wcm:action="add"><Order>2</Order><Type>MSR</Type><Size>16</Size></CreatePartition>
     <CreatePartition wcm:action="add"><Order>3</Order><Type>Primary</Type><Extend>true</Extend></CreatePartition>
    </CreatePartitions>
    <ModifyPartitions>
     <ModifyPartition wcm:action="add"><Order>1</Order><PartitionID>1</PartitionID><Format>FAT32</Format><Label>System</Label></ModifyPartition>
     <ModifyPartition wcm:action="add"><Order>2</Order><PartitionID>3</PartitionID><Format>NTFS</Format><Letter>C</Letter><Label>WindowsLab</Label></ModifyPartition>
    </ModifyPartitions>
   </Disk><WillShowUI>OnError</WillShowUI></DiskConfiguration>
   <ImageInstall><OSImage><InstallFrom><MetaData wcm:action="add"><Key>/IMAGE/INDEX</Key><Value>1</Value></MetaData></InstallFrom><InstallTo><DiskID>0</DiskID><PartitionID>3</PartitionID></InstallTo><WillShowUI>OnError</WillShowUI></OSImage></ImageInstall>
   <UserData><AcceptEula>true</AcceptEula><FullName>winluks lab</FullName><Organization>Development</Organization></UserData>
  </component>
 </settings>
 <settings pass="specialize">
  <component name="Microsoft-Windows-Shell-Setup" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS"><ComputerName>WINLUKS-LAB</ComputerName><TimeZone>UTC</TimeZone></component>
 </settings>
 <settings pass="oobeSystem">
  <component name="Microsoft-Windows-International-Core" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS"><InputLocale>en-US</InputLocale><SystemLocale>en-US</SystemLocale><UILanguage>en-US</UILanguage><UserLocale>en-US</UserLocale></component>
  <component name="Microsoft-Windows-Shell-Setup" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS" xmlns:wcm="http://schemas.microsoft.com/WMIConfig/2002/State">
   <OOBE><HideEULAPage>true</HideEULAPage><HideOnlineAccountScreens>true</HideOnlineAccountScreens><HideWirelessSetupInOOBE>true</HideWirelessSetupInOOBE><ProtectYourPC>3</ProtectYourPC></OOBE>
   <UserAccounts><LocalAccounts><LocalAccount wcm:action="add"><Name>lab</Name><Group>Administrators</Group><Password><Value>{password}</Value><PlainText>true</PlainText></Password></LocalAccount></LocalAccounts></UserAccounts>
   <AutoLogon><Password><Value>{password}</Value><PlainText>true</PlainText></Password><Enabled>true</Enabled><LogonCount>2</LogonCount><Username>lab</Username></AutoLogon>
   <FirstLogonCommands><SynchronousCommand wcm:action="add"><Order>1</Order><RequiresUserInput>false</RequiresUserInput><CommandLine>powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "Get-Volume | Where-Object DriveType -eq CD-ROM | ForEach-Object {{ $p=$_.DriveLetter+':\bootstrap.ps1'; if(Test-Path $p){{ &amp; $p }} }}"</CommandLine></SynchronousCommand></FirstLogonCommands>
  </component>
 </settings>
</unattend>'''
bootstrap=r'''$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force C:\winluks-lab | Out-Null
Start-Transcript C:\winluks-lab\bootstrap.log
powercfg /hibernate off
powercfg /change standby-timeout-ac 0
powercfg /change monitor-timeout-ac 0
Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0
Set-Service sshd -StartupType Automatic
New-Item -ItemType Directory -Force "$env:ProgramData\ssh" | Out-Null
Copy-Item "$PSScriptRoot\lab.pub" "$env:ProgramData\ssh\administrators_authorized_keys"
icacls "$env:ProgramData\ssh\administrators_authorized_keys" /inheritance:r /grant '*S-1-5-32-544:F' /grant '*S-1-5-18:F'
Start-Service sshd
if (!(Get-NetFirewallRule -Name OpenSSH-Server-In-TCP -ErrorAction SilentlyContinue)) {
  New-NetFirewallRule -Name OpenSSH-Server-In-TCP -DisplayName 'winluks lab SSH' -Direction Inbound -Protocol TCP -LocalPort 22 -Action Allow
}
New-Item C:\winluks-lab\ready -ItemType File -Force | Out-Null
Stop-Transcript
'''
import xml.etree.ElementTree as ET
ET.fromstring(xml)
iso=pycdlib.PyCdlib();iso.new(interchange_level=3,joliet=3,vol_ident='WINLUKS_SEED')
for name,data in [('Autounattend.xml',xml.encode()),('bootstrap.ps1',bootstrap.encode()),('lab.pub',a.ssh_public_key.read_bytes())]:
    iso.add_fp(io.BytesIO(data),len(data),iso_path='/'+name.upper()+';1',joliet_path='/'+name)
with os.fdopen(os.open(a.output/'windows-seed.iso',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600),'wb') as f:
    iso.write_fp(f)
iso.close()
print('Private seed media created; attach only to the disposable VM.')
