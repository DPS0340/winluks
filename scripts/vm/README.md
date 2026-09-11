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
