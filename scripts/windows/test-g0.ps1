param(
    [Parameter(Mandatory=$true)][string]$Manifest,
    [Parameter(Mandatory=$true)][string]$G0Executable,
    [Parameter(Mandatory=$true)][string]$Results
)
$ErrorActionPreference = 'Stop'
$fixture = Get-Content -Raw -Encoding UTF8 $Manifest | ConvertFrom-Json
$dir = Split-Path -Parent (Resolve-Path $Manifest)
$image = Join-Path $dir $fixture.plaintext
if ((Get-FileHash $image -Algorithm SHA256).Hash -ne $fixture.plaintext_sha256) { throw 'Plaintext fixture hash mismatch' }
New-Item -ItemType Directory -Force $Results | Out-Null
$old = @(Get-Disk | Select-Object -ExpandProperty Number)
$oldLetters = @(Get-Volume | Where-Object DriveLetter | Select-Object -ExpandProperty DriveLetter)
$proc = Start-Process $G0Executable -ArgumentList "`"$image`" 120" -PassThru -RedirectStandardOutput "$Results\g0-stdout.txt" -RedirectStandardError "$Results\g0-stderr.txt"
$report = @{ fixture=$fixture.name; published=$false; file_hashes_match=$false; source_unchanged=$false; mutation_trace='not_collected'; gate_passed=$false }
try {
    $new = @()
    for ($i=0;$i -lt 30;$i++) {
        Start-Sleep 1
        $new = @(Get-Disk | Where-Object { $_.Number -notin $old -and $_.FriendlyName -match 'winluks' })
        if ($new.Count -eq 1) { break }
        if ($proc.HasExited) { throw 'G0 process exited before publication' }
    }
    if ($new.Count -ne 1) { throw 'Expected exactly one new winluks disk' }
    $report.published=$true
    $report.disk=$new | Select-Object Number,FriendlyName,IsReadOnly,PartitionStyle,OperationalStatus
    $volumes=@($new | Get-Partition -ErrorAction SilentlyContinue | Get-Volume -ErrorAction SilentlyContinue | Where-Object DriveLetter)
    # Some partition-less filesystems appear directly as volumes. Consider only newly
    # assigned letters and verify every expected file hash before accepting the mount.
    if ($volumes.Count -eq 0) {
        $volumes=@(Get-Volume | Where-Object { $_.DriveLetter -and $_.DriveLetter -notin $oldLetters -and [string]$_.FileSystem -ieq $fixture.filesystem })
    }
    if ($volumes.Count -ne 1) { throw 'No unique filesystem mount discovered; G0 layout gate is unpassed' }
    $root=$volumes[0].DriveLetter+':\'
    foreach ($file in $fixture.files.PSObject.Properties) {
        $path=Join-Path $root $file.Name
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $file.Value.sha256) { throw 'Filesystem file hash mismatch' }
        $destination=Join-Path "$Results\copied" $file.Name
        New-Item -ItemType Directory -Force (Split-Path -Parent $destination) | Out-Null
        Copy-Item -LiteralPath $path -Destination $destination
        if ((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -ne $file.Value.sha256) { throw 'Copied file hash mismatch' }
    }
    $report.file_hashes_match=$true
    $report.os=(Get-CimInstance Win32_OperatingSystem).Version
    $report.secure_boot=Confirm-SecureBootUEFI
    # Never infer the ext4 mutation boundary from callback counters.
} finally {
    if (!$proc.HasExited) { $proc.WaitForExit(140000) | Out-Null }
    $report.exit_code=if($proc.HasExited){$proc.ExitCode}else{$null}
    $report.source_unchanged=((Get-FileHash $image -Algorithm SHA256).Hash -eq $fixture.plaintext_sha256)
    $report | ConvertTo-Json -Depth 6 | Set-Content "$Results\g0.json"
}
