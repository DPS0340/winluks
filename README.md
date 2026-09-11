# winluks

Experimental **LUKS2 partition-image bridge for Windows 11 x64**, with **Btrfs read-only and read-write access**.

The Rust core handles the restricted LUKS2 encryption profile. A small C adapter publishes
its plaintext through WinSpd, and the existing, unmodified WinBtrfs driver provides files.
Read-only is the default; writes require an explicit `--read-write` session.

| Filesystem | Windows RO | Windows RW |
|---|---|---|
| Single-device Btrfs, supported B0 profile | Tested | Tested |
| ext4 | Blocked by the discovery gate | Blocked by the discovery gate |

Ext4 decrypt/encrypt/probe oracles pass, but its pinned driver did not expose the selected
partitionless disk as a filesystem volume. The CLI returns `FS_GATE_UNPASSED` for that path.

The tested Windows 11 25H2 profile has **Secure Boot and HVCI off**. This is a development
project tested with disposable virtual images; full failure/power-loss testing and independent
security/storage review remain incomplete.

- [Implementation plan (한국어)](docs/PLAN.md)
- [RW v0.3 contract and test plan (한국어)](docs/RW-v0.3.ko.md)
- [Validation results and runtime evidence](docs/VALIDATION.md)
- [Original RO v0.2 design (한국어)](docs/design-v0.2.0.ko.md)

## Build

Rust 1.89 or later, a C compiler and OpenSSL development files are required.

```sh
cargo build --locked
cargo test --locked
cargo run -- inspect --image /absolute/path/to/fixture.luks2.img
```

`inspect` opens the image read-only and checks the LUKS layer. The filesystem remains
`unknown_locked` until unlock. Keys, salts, digests, source paths and UUIDs are not printed.

Windows builds additionally use `--features winspd,vendored-openssl` and the pinned WinSpd
SDK. The [Windows workflow](.github/workflows/ci.yml) builds `winluks2.exe` and the test
harnesses. Deploy the matching `winspd-x64.dll` beside the executable in a trusted directory,
and install Microsoft's x64 Visual C++ v14 runtime.

## Use on Windows

Install the pinned WinSpd/WinBtrfs drivers separately. For the disposable QEMU lab, prepare
RW-capable driver policy with the script below, then reboot:

```powershell
.\scripts\windows\prepare-drivers.ps1 -Filesystem btrfs -AccessMode rw -TrustPinnedPublishers
```

The bridge checks the running consumer and its pinned binary before password input, then
checks the mounted volume's identity and actual RO/RW flags. See the [VM guide](scripts/vm/README.md)
for driver signing, security settings and the separate test clones.

From an administrator console, use a local image with the LUKS2 header at byte zero:

```powershell
# Read-only is the default; --read-only is also accepted.
.\winluks2.exe open --image C:\fixtures\volume.img --keyslot 0 --filesystem btrfs

# Modifies this image in place. Use a disposable copy for development.
.\winluks2.exe open --image C:\fixtures\working-copy.img --keyslot 0 --filesystem btrfs --read-write
```

Passwords use a console with echo disabled. There is no password argument or environment
variable. RW takes an exclusive image handle, preserves its size and writes only the encrypted
data segment. LUKS headers and keyslots are not modified. Each completed block write is flushed
to the backing file; filesystem caches are also flushed during normal close.

Press **Ctrl+C** to close. RW locks and dismounts the exact session's filesystem before draining
callbacks and removing the disk. If files remain open, `CLOSE_BLOCKED` leaves the session running:
close those files and press Ctrl+C again. `UNCLEAN_CLOSE` reports a failed orderly shutdown;
check a copy offline before using it again for RW.

Physical-device paths, remote files, resize, discard, automatic repair and driver fallback are
unsupported. The LUKS encryption sector is 512 bytes; the Btrfs filesystem sector is 4096 bytes.
A qcow2, GPT or MBR container is not a valid input image.

## Validation and virtual lab

All test storage is file-backed. A Linux VM generates independent cryptsetup/filesystem
fixtures; a Windows VM works on local NTFS copies. Transfer frozen images between guests,
and detach an image before handing its modified copy to another OS.

Executed v0.3 checks include:

- 30 RO regression fixtures and 30 RW differential fixtures, including mixed key sizes and slots.
- Linux dm-crypt comparisons of every byte after boundary, overlapping and 1 MiB writes.
- Windows file creation, overwrite, append, truncate, sparse files, Unicode, copy, rename and delete.
- Busy-close rejection, successful retry, device removal and persistent Windows remounts.
- Linux `btrfs check --readonly`, independent file-content checks and a Linux-to-Windows return file.
- RO mutation rejection with unchanged source bytes in the RW-configured guest.

See [detailed results](docs/VALIDATION.md) and [reproduction scripts](scripts/vm/README.md).
VM rollback alone is not used as evidence of correct read-only or write durability behavior.

## License and dependencies

GPL-3.0-or-later. Driver binaries and Microsoft installation media are not checked into Git.
OpenSSL and RustCrypto Argon2 supply cryptographic primitives; `Cargo.lock` pins Rust
packages. See the [third-party notes](docs/THIRD_PARTY.md).

WinSpd - Windows Storage Proxy Driver, Copyright (C) Bill Zissimopoulos.
[WinSpd source](https://github.com/winfsp/winspd) and [license](third_party/WinSpd-LICENSE.txt).
