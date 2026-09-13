# Validation record

Initial implementation and RW extension: 2026-09-11. Status is updated from executed commands only.

## v0.3.1 hardening and virtual power cuts — 2026-09-13

**Experimental prerelease.** The completed virtual matrix contains 20 final trials across
10 scenarios, two repetitions each. It found an actual durability limitation: all four
abrupt-cut/process-kill trials after four successful application file flushes retained
**0/4 new files**. The supported filesystem commit boundary is an orderly bridge close
through volume lock/dismount. Databases and fsync-dependent workloads remain unsupported.

| Executed check | Result |
|---|---|
| Linux and Windows CI at `d4f670a` | 37 Rust tests on each platform; Windows enables WinSpd callbacks and vendored OpenSSL |
| Native C shim close/publication regression | 25 cases passed on Windows MSVC; real shim with mocked OS/WinSpd boundaries |
| New core RO / RW differential regressions | 30/30 each; RW compares all 240 MiB per image through independent Linux dm-crypt |
| Windows RW lifecycle after fixes | Eight-file workload, actual held-file close rejection/retry and Linux independent content/sparse/fsck checks passed |
| Candidate default RO and ext4 gate | Six-file copy/hash checks, RO flags, create/rename/delete/raw-write rejection, complete image hash unchanged, actual close exit 0 and device removal; ext4 rejected before password |
| Duplicate publisher | A second same-UUID copy was rejected with Win32 170 and actual process exit 1 |
| Final VM matrix | 20 completed trials; five preliminary/incomplete attempts retained and excluded, with reasons |
| Filesystem crash integrity | 18/18 filesystem cases retained canonical files, LUKS header/keyslot bytes, image size and agreeing mirrors; read-only fsck/mount passed |
| Normal-close persistence, followed by hard cut | 2/2 cases, 4/4 new files retained in each; actual close exit 0 and `clean=true` |
| Direct raw block flush persistence | 2/2 cases, three acknowledged writes each; full 240 MiB comparison after QEMU SIGKILL with lock/handles still held |
| Application file-flush durability | **Failed: 0/4 new files retained in each of four abrupt-cut/process-kill cases**; structural integrity does not change this result |
| Actual Btrfs disk-full/forced-RO close | 2/2 cases returned `CLOSE_FAILED phase=0 code=19`, `UNCLEAN_CLOSE`, actual exit 1 and `clean=false` |
| ASan fuzz smoke | Metadata 4,147,343 executions / 61 s; probe 7,833,396 / 61 s; no crash |
| Cargo audit | 0 known advisories and 0 warnings with recorded advisory database revision |
| Gitleaks history scan | One confirmed false positive: literal cryptsetup cipher value `aes-xts-plain64`; no confirmed secret |
| Independent review | Two separate AI reviewers; original P1/P2 findings fixed and re-reviewed; this is not an external human audit |

The tested Windows artifact is from [CI 34749762390](https://github.com/DPS0340/winluks/actions/runs/34749762390),
commit `d4f670a2e67240b62dea86dccc5f44f5fdd5618b`, executable SHA256
`e4c66309d46c6bf6e6f8c73630596f3e426472704050cc531a22202b1405d0b8`.
It runs on the same Windows 11 25H2/WinSpd/WinBtrfs profile described below. Release packaging
binds the binary to that successful CI artifact and verifies unchanged application/build inputs.
The earlier eight-file/duplicate/busy-close runtime used the same production fixes at `359a612`.

See the [complete power-cut report](POWERLOSS-v0.3.1.md),
[per-trial evidence](evidence/v0.3.1-powercut.json), [core results](evidence/v0.3.1-core.json),
[Windows lifecycle and Linux verification](evidence/v0.3.1-lifecycle.json),
[core AI review](reviews/v0.3.1-core-ai.md) and [storage AI review](reviews/v0.3.1-storage-ai.md).
The report distinguishes actual VM failures from deterministic syscall/native mocks, records
cut-trigger precision and describes the authenticated prewrite baseline for the raw oracle.

QEMU SIGKILL leaves the host and physical storage powered. Host/outer-disk-full, deterministic
torn sectors, controller power loss and kernel mutation tracing remain untested; ext4 Windows
publication remains blocked. The historical records below retain their original test counts
and limitations and are not additional v0.3.1 reruns.

## Historical v0.3 Btrfs RO/RW

The production code at `743d0ed` passed [Linux and Windows CI](https://github.com/DPS0340/winluks/actions/runs/34605278803).
Its hash-verified Windows artifact was then exercised in a separate RW clone of the Btrfs
guest. The test profile remains Windows 11 Enterprise Evaluation 25H2 `26200.6584`,
QEMU/KVM, four vCPUs, 8 GiB RAM, Secure Boot off and HVCI/VBS off. The pinned WinBtrfs
v1.10 driver had its `Readonly` registry policy set to 0. All storage was disposable virtual files.

| Executed check | Result |
|---|---|
| Linux build, format, Clippy and regression tests | Passed; 29 tests |
| Windows MSVC tests, WinSpd build and packaging | Passed; 28 tests (the backend-fault injection test is Unix-only) |
| RO core regression after the cipher refactor | 30/30 fixtures, complete plaintext/source hashes and boundary/policy checks |
| RW core differential oracle | 30/30 fixtures; six writes per image and all 240 MiB independently compared through Linux dm-crypt |
| LUKS boundary invariants | Image length and header/keyslot bytes preserved; canonical fixtures unchanged |
| Windows Btrfs file operations | Create, unaligned overwrite, append, truncate, sparse, Unicode, copy, rename, delete and readback passed |
| Backing-file exclusivity | A second read handle and a second write handle were both rejected during RW |
| Busy close and retry | An actually held file caused `CLOSE_BLOCKED`; the session stayed usable; closing the handle and retrying completed cleanly |
| Windows RW reopen | All eight resulting files retained their expected sizes and hashes; deleted paths remained absent |
| Windows → Linux | `btrfs check --readonly`, RO mount, file hashes, independent payload/text and sparse-allocation checks passed |
| Linux → Windows | Linux wrote a return file and unmounted cleanly; a second filesystem check and Windows RO verification of all nine files passed |
| RO in the RW-configured guest | Disk/volume RO flags, create/rename/delete/raw-write rejection and unchanged complete image hash passed |
| CLI policy | Default RO, mutually exclusive mode flags, RW conflict with an RO reader and ext4 RW publication gate passed |
| Normal shutdown | Each final session exited 0 with `clean=true`; the virtual device disappeared; no synthetic password appeared in captured console output |

The block-write matrix uses the same 24 KDF/key/hash/filesystem combinations, four mixed
volume/keyslot key-size cases and two distinct-password slot-1 cases described below. Its
write producer used `b367dbc`; the later production changes address session discovery,
shutdown and failure handling. Each disposable copy receives first/last-sector writes,
multi-sector and overlapping writes, and a 1 MiB write. Linux compares the entire decrypted
payload with its original plaintext plus independently calculated changes. Those particular
copies deliberately overwrite filesystem structures and are never mounted.

The separate filesystem test starts from a fresh Btrfs image. Windows produces the file
manifest, and Linux verifies both its hashes and independently generated content after
the Windows session has closed. Linux then writes a UTF-8 return file, unmounts and checks
the filesystem again. The returned image is opened RO in Windows; after closing, its
complete SHA256 is unchanged. This checks persistence across OS handoff as well as local
readback. It does not simulate sudden power loss or prove sector-write atomicity.

For partitionless WinBtrfs volumes, disk-extents discovery returned `ERROR_INVALID_FUNCTION`.
The implementation instead queries the storage descriptor directly on each volume handle
and matches the current session's random SCSI serial. That exact volume is locked, flushed
and dismounted on RW close. Drive letters are not used to infer the shutdown target.

Machine-readable reports: [RW core and RO regression](evidence/rw-core.json),
[Windows Btrfs RW lifecycle](evidence/btrfs-rw.json) and
[Linux/Windows round trip](evidence/btrfs-rw-roundtrip.json).
The [RW contract](RW-v0.3.ko.md) and [VM guide](../scripts/vm/README.md) describe reproduction.
These reports contain generated-fixture observations, hashes and environment details,
without credentials, disk images or captured plaintext.

The complete failure/power-loss matrix, driver-boundary mutation tracing and independent
security/storage review remain incomplete. ext4 Windows publication remains blocked in
both modes by the existing discovery gate. No real-user-disk or release-readiness claim is made.

## Historical v0.2 RO baseline

The records below describe the original RO implementation and its tested artifacts.

| Evidence | Status |
|---|---|
| Linux `cargo check` | Passed on Rust 1.98.1, OpenSSL 3.6.4 |
| cryptsetup differential fixtures | 30/30: 24 KDF/key/hash/filesystem combinations, 4 mixed volume/keyslot key-size cases, 2 distinct-password slot-1 cases with independent AF/digest hashes |
| Linux regression tests | 25 tests passed: bounded JSON, backend, metadata, filesystem policies and the ext4 publication gate |
| Parser ASan fuzz smoke test | 4,634,101 executions / 61 seconds, no failure |
| Filesystem probe ASan fuzz smoke test | 8,574,563 executions / 61 seconds, no failure |
| Final filesystem probe ASan rerun | 4,573,468 executions / 31 seconds after additional geometry/flag validation, no failure |
| Windows MSVC build and tests | Passed on `windows-2025`: 25 Rust tests, vendored OpenSSL, WinSpd bridge, G0 spike and oracle harness |
| Windows core differential oracle | Both baseline filesystems passed full 240 MiB plaintext hashes and boundary comparisons |
| G0-B WinSpd + WinBtrfs | Passed on Windows 11 25H2 26200.6584, Secure Boot off, HVCI/VBS off |
| LUKS2 Btrfs CLI runtime | Passed console UTF-8 password, publish, all file/copy hashes, mutation rejection, Ctrl+C, device removal and encrypted source hash |
| G0-E WinSpd + Ext4Fsd | No-Go: running pinned driver did not expose a filesystem volume on the partitionless disk; encrypted publication is blocked |
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

Two additional fixtures retain a different-password slot 0 and select slot 1. The selected
PBKDF2/AF hash is SHA-512 while the volume-key digest uses SHA-256. The correct slot-1
password fails on slot 0 and succeeds on slot 1; full plaintext and source hashes match.

This matrix is not the entire R01–R14 suite: it does not exercise every independently varied
AF/digest hash, every optional ext4 feature combination, all Windows lifecycle failures,
kernel mutation tracing or an independent security review. Short fuzz smoke runs do not
replace continuous fuzzing or cryptographic review.

The first Windows CI attempt found an incorrectly imported `DRIVE_FIXED` constant after
building vendored OpenSSL; the import was corrected. The second passed all build/test steps
but failed packaging because the SHA256 output file was included in its own input enumeration;
hashes are now collected before writing that file. The first local Windows 11 installer
stalled at 28%; a fresh virtual disk with a simplified four-vCPU VM reached OOBE.

## Windows runtime evidence

The VM is Windows 11 Enterprise Evaluation 25H2, `10.0.26200.6584`, QEMU/KVM, four vCPUs,
8 GiB RAM, OVMF and TPM 2.0. HVCI/VBS was not running. The clean VM exposed two packaging
problems hidden by the CI runner: the Visual C++ v14 runtime was absent, and WinSpd's MSI
stream name uses an underscore while the PE imports `winspd-x64.dll`. Both were corrected.

WinSpd 1.0.20357 loaded with Secure Boot on after its validated pinned publisher certificate
was added to the disposable guest's TrustedPublisher store. WinBtrfs v1.10 remained stopped
with error 577 / Code Integrity event 3004. The CLI returned `FS_DRIVER_UNAVAILABLE` before
requesting a password. No default Secure Boot support is claimed for this WinBtrfs build.

With Secure Boot off in the isolated Btrfs guest, G0-B published a partitionless RAW SCSI disk
with `IsReadOnly=true`; WinBtrfs mounted it and advertised `FILE_READ_ONLY_VOLUME`. All six
file hashes and copied hashes matched, including UTF-8 names, a sparse file and a subvolume.
Create, rename, delete and a raw disk write were rejected. Raw writes surfaced as Win32
1117; successful reads before/after that attempt and unchanged bytes ruled out an invalid
read target. The complete source hash was unchanged, the process exited 0, and the device
disappeared. `test-g0.ps1` recorded `gate_passed=true`.

The real CLI then unlocked the encrypted Btrfs image using its UTF-8 console password,
passed the same mounted-file/mutation tests, and closed through Ctrl+C with exit 0. The
encrypted source hash was unchanged and the device was removed. A scan of captured console
output found no synthetic password. These results cover the tested happy path and mutation
attempts, not the complete G2 failure/lifecycle matrix.

In a separate clean Ext4Fsd guest with Secure Boot/HVCI off, the pinned 0.71 installer
omitted the catalog referenced by its INF and did not register the filesystem driver.
The setup script now verifies the unchanged Microsoft-signed driver, installs its service,
applies the RO policy and requires a reboot. Both Ext2Fsd and WinSpd were running during
the valid G0-E test. The read-only partitionless RAW disk appeared, but no filesystem
volume or drive path appeared, including a separate discovery check. The process exited 0,
the device disappeared and the complete source hash stayed unchanged. That is a discovery
failure, not a passing read-only mount. No automatic GPT wrapper or driver patch was added.

Ext4 therefore returns `FS_GATE_UNPASSED` before password input/device creation in the
Windows CLI, and the publication entry point repeats the gate check. Its independent
decrypt/probe oracle remains available. Three read callbacks and zero Write/Unmap callbacks
were observed in G0-E, but no instrumentation above WinSpd's RO rejection was collected;
R13 and hidden-write behavior remain unverified. Resuming encrypted ext4 integration
requires a design decision about discovery followed by a passing G0-E, including that trace.

The final application code at `f79856e` passed [Linux and Windows CI](https://github.com/DPS0340/winluks/actions/runs/34601172950).
That Windows artifact was hash-verified and rerun in both guests: ext4 returned the gate
error without a password prompt or device, and Btrfs again passed the mounted-file,
mutation, console-close, source-hash and device-removal tests.

Machine-readable reports: [G0-B](evidence/g0-btrfs.json), [G0-E](evidence/g0-ext4.json),
[encrypted Btrfs CLI](evidence/btrfs-cli.json) and [ext4 publication gate](evidence/ext4-publication-gate.json).
These contain generated-fixture results and the tested environment, without VM credentials
or captured plaintext. They are runtime observations, not an independent audit attestation.
