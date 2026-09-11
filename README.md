# winluks

Experimental **read-only LUKS2 partition-image bridge for Windows 11 x64**.

The independent Rust core decrypts a restricted LUKS2 profile. A small C adapter publishes
the plaintext through WinSpd. Existing WinBtrfs and Ext4Fsd drivers are separate filesystem
consumers; neither is modified. This is a development repository, **not a validated recovery tool**.

- [Implementation plan (한국어)](docs/PLAN.md)
- [Original design v0.2.0 (한국어)](docs/design-v0.2.0.ko.md)
- [Validation status](docs/VALIDATION.md)

## Build

Rust 1.89 or later, a C compiler, and OpenSSL development files are required.

```sh
cargo build --locked
cargo test --locked
cargo run -- inspect --image /absolute/path/to/fixture.luks2.img
```

`inspect` checks the LUKS layer only and reports `unknown_locked` for the filesystem.
It does not print keyslot contents, salts, digests, paths or UUIDs.

Windows integration builds additionally use `--features winspd` and the pinned WinSpd SDK.
The Windows workflow documents the MSVC build. The matching `winspd-x64.dll` must be
deployed next to the executable in a trusted directory. Install Microsoft's current x64
Visual C++ v14 Redistributable. Driver installation is separate; the bridge checks the
selected driver's service, explicit RO policy and pinned on-disk binary hash before unlock.
Reboot after driver installation or policy changes, as directed by the lab setup script.

```powershell
.\winluks2-ro.exe open --image D:\fixtures\sample.luks2.img --keyslot 0 --filesystem ext4 --read-only
```

The password is read from a local console with echo disabled. The device is published only
after key validation and the selected filesystem's policy check. Ctrl+C drains callbacks and
closes the device. There is no write/force switch, physical-device backend, repair operation,
network backend, automatic driver fallback or password argument/environment variable.

## Test environment

Use disposable QEMU/KVM guests and generated data. The Linux guest supplies the independent
cryptsetup/filesystem oracle; the Windows guest reads a byte-identical image on local NTFS.
All disks are virtual. Keep the frozen canonical fixture separate from disposable test copies.
VM rollback and unchanged qcow2 bases do not prove that the application rejected writes.

The LUKS encryption sector is 512 bytes. An ext4 E0 filesystem block is 4096 bytes; these are
different units. Image byte zero must be the LUKS2 header, not a GPT/MBR or qcow2 header.

## License and dependencies

GPL-3.0-or-later. Driver binaries and Microsoft installation media are not redistributed here.
OpenSSL and RustCrypto Argon2 supply cryptographic primitives. The dependency lockfile pins
the Rust dependency graph. See [third-party notes](docs/THIRD_PARTY.md).
