# Validation record

Initial implementation session: 2026-09-11. Status is updated from executed commands only.

| Evidence | Status |
|---|---|
| Linux `cargo check` | Passed on Rust 1.98.1, OpenSSL 3.6.4 |
| cryptsetup differential fixtures | 24/24 KDF/key/hash/filesystem combinations passed; another 4/4 mixed volume/keyslot key-size fixtures passed |
| Linux regression tests | 24 tests passed: bounded JSON, backend, metadata, filesystem policies |
| Parser ASan fuzz smoke test | 4,634,101 executions / 61 seconds, no failure |
| Filesystem probe ASan fuzz smoke test | 8,574,563 executions / 61 seconds, no failure |
| Windows MSVC build and tests | Passed on `windows-2025`: vendored OpenSSL, Rust tests, WinSpd bridge, G0 spike and oracle harness |
| G0-B WinSpd + WinBtrfs | Pending |
| G0-E WinSpd + Ext4Fsd | Pending |
| Driver-boundary mutation tracing | Pending |
| Full R01–R14 matrix, fuzzing and independent review | Pending |

No gate is passed merely because code exists or a driver is installed. This repository does
not claim security audit completion or safety for real user disks. G3 is outside this virtual
fixture implementation session; G4 remains unavailable until its independent review and
runtime requirements have been met.

## Executed Linux comparisons

Debian 13 VM, kernel `6.12.107+deb13-amd64`, cryptsetup 2.7.5. Fixtures were created,
cleanly unmounted, reopened through read-only dm-crypt, and checked with `e2fsck -fn`
or `btrfs check --readonly`. ext4's file oracle used `ro,noload` on the read-only mapping.

The 24-case matrix is Btrfs/ext4 × PBKDF2/Argon2i/Argon2id × 256/512-bit XTS keys ×
SHA-256/SHA-512. The additional four cases use independent volume/keyslot key lengths
of 256/512 and 512/256 bits for both filesystems. Each case compared the complete 240 MiB
plaintext SHA256, rejected a wrong password, passed the filesystem probe, rejected direct
Write/Unmap calls, and retained the complete encrypted image SHA256. Baseline fixtures
also compared non-aligned reads with independent plaintext files.

This matrix is not the entire R01–R14 suite: it does not exercise every independently varied
AF/digest hash, every optional ext4 feature combination, all Windows lifecycle failures,
kernel mutation tracing or an independent security review. Short fuzz smoke runs do not
replace continuous fuzzing or cryptographic review.

The first Windows CI attempt found an incorrectly imported `DRIVE_FIXED` constant after
building vendored OpenSSL; the import was corrected. The second passed all build/test steps
but failed packaging because the SHA256 output file was included in its own input enumeration;
hashes are now collected before writing that file. The first local Windows 11 installer
stalled at 28%; a fresh virtual disk with a simplified four-vCPU VM reached OOBE.
