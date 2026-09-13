# Virtual lab

Use QEMU/KVM guests with file-backed disks only. Recommended: Linux 4 vCPU / 8 GiB,
Windows 4 vCPU / 8 GiB for the tested SATA setup (increase after installation if needed). Bind management forwarding to `127.0.0.1`; do not expose
guest administration to the LAN. Keep keys, media and disk images outside the repository.

Linux baseline: Debian 13 generic amd64 cloud image, SHA512 verified against Debian's
published checksum file. Bootstrap with cloud-init, a generated SSH key, and packages:
`cryptsetup btrfs-progs e2fsprogs python3 gcc pkg-config libssl-dev curl ca-certificates git`.

Windows baseline: official Windows 11 Enterprise evaluation ISO, SHA256 checked against
Microsoft's published hash PDF. Q35, OVMF UEFI, swtpm 2.0, SATA OS disk during setup.
The evaluation period and Windows licensing terms still apply. No activation bypass is used.

Run inside the Linux VM:

```sh
sudo python3 scripts/linux/make-fixtures.py --output /home/lab/fixtures
# Expanded independent cipher/KDF/hash matrix:
sudo python3 scripts/linux/make-fixtures.py --output /home/lab/matrix --matrix
```

Transfer the generated directories to the host and to a local NTFS disk in Windows.
Never use a writable shared LUKS volume. Check the image hash after transfer.
The manifest's `password_file` is a synthetic test credential, never a user credential.
The `oracle` example is an explicit test harness, not the production password interface:

```sh
cargo run --release --example oracle -- /absolute/fixture/manifest.json
```

Save powered-off VM baselines and clone them separately for Btrfs and ext4. Preserve
UEFI variable and TPM state with the matching disk snapshot. Record Secure Boot and
the actual VBS/HVCI state inside Windows. Collect logs and image hashes before rollback.

Do not count WinSpd callback counters as evidence of no filesystem mutation requests;
its miniport may have rejected writes earlier. G0-E requires tracing that earlier boundary.

## Windows setup and normal boots

Create the private seed using `create-windows-seed.py --output PRIVATE_VM_DIR
--ssh-public-key LAB_PUBLIC_KEY` in a Python environment with `pycdlib`. It creates a random
local administrator password and an unattended ISO that **erases guest disk 0** during
installation. Attach it only to a fresh disposable virtual OS disk.

Place `windows.qcow2`, `windows-vars.fd` and `windows-seed.iso` in that private directory.
Use a Microsoft-enrolled Secure Boot variable store matching the OVMF code and swtpm 2.0.
The runner needs `qemu-system-x86_64` and `swtpm` (or set `WINLUKS_SWTPM` to its binary).

```sh
# First installation only; answer the ISO boot prompt on loopback VNC port 5918.
scripts/vm/run-windows.sh PRIVATE_VM_DIR OVMF_CODE.secboot.fd --install WINDOWS_ISO
# After shutdown: normal boot omits all setup media.
scripts/vm/run-windows.sh PRIVATE_VM_DIR OVMF_CODE.secboot.fd
```

The setup enables public-key OpenSSH on loopback host port 22281. Before installing test
drivers, shut down and preserve the disk, matching UEFI variables and TPM state as the clean
baseline. Make a separate disposable copy for each filesystem. Run `prepare-drivers.ps1`
as administrator in each copy, reboot, then use `test-g0.ps1` with the CI G0 executable and
a **plaintext** fixture manifest. The report deliberately does not turn uncollected mutation
tracing into a passed gate. Never initialize or format a RAW disk to make G0 appear to pass.

The executed Btrfs profile uses Secure Boot **off** and VBS/HVCI **off**. WinBtrfs v1.10
failed to load with the clean VM's Secure Boot policy enabled. Keep that result distinct
from the successful experimental profile; record the actual guest security state each time.

For unattended driver setup, `-TrustPinnedPublishers` explicitly enrolls the validated,
hash-pinned WinSpd/WinBtrfs catalog signing certificates in the disposable guest's
TrustedPublisher store. It does not supply replacement driver signatures. Ext4Fsd's pinned
installer omits its referenced catalog; the script registers the unchanged Microsoft-signed
filesystem driver as a system-start service and sets RO policy before reboot.

```powershell
.\prepare-drivers.ps1 -Filesystem btrfs -TrustPinnedPublishers
# In the independent ext4 clone:
.\prepare-drivers.ps1 -Filesystem ext4 -TrustPinnedPublishers
```

The test harness `test-mounted.ps1` deliberately attempts file creation, rename, deletion
and raw writes only after checking the QEMU model, winluks disk identity and read-only flags.
Use only generated fixtures. Capture its JSON, normal-close output and full encrypted source
hash before shutting down a test clone.

## v0.3 Btrfs RW

Clone the powered-off Btrfs guest for RW experiments. Inside that disposable clone, run
`prepare-drivers.ps1 -Filesystem btrfs -AccessMode rw -TrustPinnedPublishers`, then reboot.
Use `winluks2.exe open --image C:\fixtures\working-copy.img --keyslot 0 --filesystem btrfs --read-write`
from an administrator console. The same driver configuration also supports the default RO mode.
Use a new copy of a canonical fixture for each file-level test, and preserve the canonical file.

While the volume is published, run `test-writable.ps1` with its fixture manifest, drive root,
the exact `WinSpd winluks RW` disk number and a results directory. It generates expected file
hashes. Keep a file handle open to test Ctrl+C's `CLOSE_BLOCKED` behavior; release it and retry.
After a normal close, reopen the same image and run the script with `-VerifyOnly`.

Once detached, copy that image and `expected.json` to the Linux VM:

```sh
sudo python3 scripts/linux/verify-btrfs-rw.py \
  --manifest /home/lab/fixtures/btrfs-pbkdf2-512-sha256/manifest.json \
  --image /home/lab/windows-rw/volume.img --expected /home/lab/windows-rw/expected.json \
  --results /home/lab/windows-rw/verified --write-return-file
```

This checks unchanged LUKS headers, `btrfs check --readonly`, independent content and sparse
allocation. The optional return step records a new Linux file after the read-only verification,
unmounts cleanly and checks again. Transfer it back to Windows and open with `--read-only`;
use the generated `ro-manifest.json` with `test-mounted.ps1` for file hashes and mutation rejection.

The separate **block** oracle deliberately overwrites filesystem structures in disposable
copies to test cipher/range boundaries. Do not mount those copies:

```sh
cargo run --release --example rw_oracle -- CANONICAL_MANIFEST NEW_OUTPUT_DIRECTORY
sudo python3 scripts/linux/verify-block-writes.py \
  --fixture-root /home/lab --writes-root /home/lab/rw-core --results /home/lab/rw-core-results.json
```

Keep the output hierarchy aligned with the canonical manifests (for example,
`rw-core/matrix/FIXTURE/writes.json` corresponds to `matrix/FIXTURE/manifest.json`).
The Linux verifier uses original Linux plaintext and read-only dm-crypt mappings as its oracle.

## v0.3.1 failure and power-cut reproduction

The [versioned report](../../docs/POWERLOSS-v0.3.1.md) records the exact scope, failed
application-flush durability and preliminary harness failures. The matrix uses fresh,
file-backed Windows overlays; no physical host shutdown is performed.

Prepare the private layout expected by `run-powercut.py`:

- `LAB/windows-powercut-base/{windows.qcow2,windows-vars.fd,tpm/}`: a **powered-off** RW-ready
  Windows guest with the matching tested EXE/DLL under `C:\winluks-lab\bin`, the unchanged
  Btrfs fixture at `C:\winluks-lab\powercut\volume.img`, and existing lab scripts under
  `C:\winluks-lab\scripts`. Do not put `raw-before.bin` or `raw-go` in the base.
- `LAB/fixtures/btrfs-pbkdf2-512-sha256/`: the canonical manifest, image, plaintext and
  synthetic password file. Place the same fixture at `/home/lab/fixtures/...` in Linux.
- `LAB/vm/{lab_ed25519,known_hosts,linux-qmp.sock}`: generated lab SSH identity and the
  running Linux guest QMP socket. Linux SSH forwards to 22280, Windows to 22281; the
  Windows runner uses the loopback VNC port 5918.
- Linux needs `ntfs-3g`, `dislocker`, cryptsetup and btrfs-progs; it must have a free
  `pcie-root-port,id=cut-port,chassis=1,slot=5` for the read-only crash disk hotplug.
  Use noninteractive sudo only in this disposable guest. A Python environment on the
  host needs `pexpect`, plus QEMU/KVM and the configured swtpm binary.

The runner currently uses `/usr/share/edk2/x64/OVMF_CODE.secboot.4m.fd` and
`LAB/swtpm/bin/swtpm`; adapt these local runtime paths for another host. Each trial ID
must be new. Preserve every backing file without booting/modifying it while overlays exist.

```sh
python scripts/vm/run-powercut.py --lab PRIVATE_LAB --repo REPO \
  --trial power-flushed-01 --case power-flushed
```

Run two fresh IDs for each of `power-idle`, `power-flushed`, `power-stream`, `power-close`,
`process-idle`, `process-flushed`, `process-stream`, `normal-close`, `disk-full`, `raw-block`.
The runner copies the current workload and Linux verifier scripts, records password-free
console/events/actual exit status, verifies the trial PID and sends QEMU SIGKILL. It then
attaches the unbooted OS disk read-only to Linux, extracts the local LUKS file read-only,
and invokes `verify-powercut.py`. Keep the raw logs and images private; publish only
sanitized fixture observations, hashes and test summaries.

For raw trials, the Windows script holds the exact target volume locked/dismounted. The
host receives and authenticates all prewrite plaintext before creating the go marker;
then three exact unbuffered/write-through writes and flushes are observed before the cut.
The Linux verifier receives that baseline separately and compares the complete plaintext.
A missing/mismatched baseline or incomplete event sequence fails verification.

Normal-close requires four nonempty expected records, actual exit 0 and `clean=true`.
The pinned driver's disk-full case requires confirmed ENOSPC and an explicit unclean,
forced-RO close result. For abrupt file-level cases, `required_checks_passed` in the
low-level Linux JSON describes structural/canonical checks; inspect the nonempty
`records` list separately. It must **not** be reported as passing file-flush durability.
A stream/close-request trigger does not establish a cut within a specific backend write.

If a verifier fails, preserve its trial and logs; do not reboot Windows or repair the
crash image to obtain a passing result. Check for a remaining read-only QMP attachment
before the next trial and detach that exact `cut-device`/`cut-image`. Stop only task-created
QEMU/swtpm instances when the campaign ends. Retain incomplete attempts with their reasons.
