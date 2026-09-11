# Virtual lab

Use QEMU/KVM guests with file-backed disks only. Recommended: Linux 4 vCPU / 8 GiB,
Windows 8 vCPU / 24 GiB. Bind management forwarding to `127.0.0.1`; do not expose
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
