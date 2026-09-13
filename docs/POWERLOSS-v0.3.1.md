# v0.3.1 failure and power-cut results

Executed on 2026-09-13, using disposable virtual images only. **v0.3.1 remains an experimental
prerelease. Application file-flush durability failed in the tested WinBtrfs profile.**

Two separate AI reviewers inspected the implementation, supplied regression cases and
reviewed the fixes and evidence. They did not implement the production fixes. This is an
independent AI review within the development task, not an external human security audit.

## Outcome

The final matrix has **10 cases × 2 fresh overlays = 20 trials**. Every final crash disk was
read in a separate Linux guest before Windows could reboot or repair it. Five preliminary
or incomplete attempts are retained and explained below; they are excluded from these counts.

| Case and exact observed trigger | Repeats | Result |
|---|---:|---|
| Idle publication → QEMU SIGKILL | 2 | Canonical files, LUKS metadata, agreeing Btrfs mirrors and read-only filesystem check preserved |
| Four file writes + successful `FlushFileBuffers` observations → QEMU SIGKILL | 2 | **File-flush durability FAIL: 0/4 new files retained in each**; filesystem structure and canonical files intact |
| First observed streaming file-write completion → QEMU SIGKILL | 2 | Uncommitted new data absent; filesystem structure and canonical files intact |
| Four flushed files, Ctrl+C close request queued → immediate QEMU SIGKILL | 2 | Close did not report success; 0/4 new files retained in each; filesystem structure intact |
| Idle publication → publisher process killed → QEMU SIGKILL | 2 | Virtual device disappeared; filesystem structure and canonical files intact |
| Four flushed files → publisher process killed → QEMU SIGKILL | 2 | **File-flush durability FAIL: 0/4 new files retained in each**; virtual device disappeared and filesystem structure intact |
| First observed streaming completion → publisher killed → QEMU SIGKILL | 2 | Device disappeared; 96 completion records captured by the time the cut finished, none persisted; filesystem structure intact |
| Four flushed files → orderly close exit 0 and `clean=true` → QEMU SIGKILL | 2 | **4/4 files retained in each**, with expected content, agreeing mirrors and clean filesystem check |
| Btrfs filesystem filled to `ERROR_DISK_FULL` → close → QEMU SIGKILL | 2 | 221 file-flush records then ENOSPC; WinBtrfs forced RO; bridge correctly returned `CLOSE_FAILED phase=0 code=19`, `UNCLEAN_CLOSE`, exit 1 and `clean=false`; canonical files intact, new files absent |
| Locked/dismounted volume → three exact raw writes + successful block flushes → QEMU SIGKILL with handles still held | 2 | **All three writes persisted in each trial**; Linux compared all 251,658,240 plaintext bytes with the authenticated prewrite baseline plus independently calculated changes |

The **18 filesystem trials** all passed `btrfs check --readonly`, a `ro,nologreplay` mount,
canonical file-content checks and mirror agreement. The two raw trials deliberately overwrite
filesystem structures and use a full block oracle instead of a filesystem check. Every final
trial preserved the LUKS header/keyslot region, image length and canonical source image.
These integrity results must not be described as passing application-flush durability.

Machine-readable results: [power-cut matrix](evidence/v0.3.1-powercut.json),
[30-case core regressions and tool checks](evidence/v0.3.1-core.json).

## What was fixed before testing

- **Btrfs mirror gate:** validate all superblocks present at 64 KiB, 64 MiB and 256 GiB,
  including location, checksum, profile and normalized content. Reject disagreement or
  recovery state before WinBtrfs can select a newer unchecked mirror. Four independently
  written regression cases failed on the baseline and passed after the fix.
- **Final close result:** drain callbacks before sampling final sticky backend/adapter and
  dispatcher errors. Explicit stop cancellation is distinguished from a genuine prior error.
- **Forced read-only and close phases:** check the actual volume mode before/after lock;
  only the expected busy lock errors leave a retryable session. Flush, dismount, transport
  and forced-RO errors cannot be reported as a clean close. The disk-full trials exercise
  the actual driver forced-RO path, beyond the mocked regression cases.
- **Callback faults:** immediately mark read/write/control panics as persistent failures;
  zero failed read buffers. Retry interrupted I/O, preserve short-I/O handling and reject
  further writes after write/flush failures. The injector is compiled only for tests.
- **Publication coordination:** hold a global mutex through shutdown so cooperating v0.3.1
  publishers cannot expose same-UUID copies simultaneously. An actual second publication
  was rejected with Win32 170 and process exit 1. Older versions and other publishers do
  not participate in this guard.

See the [core AI report](reviews/v0.3.1-core-ai.md) and
[storage AI report](reviews/v0.3.1-storage-ai.md) for initial findings, fix re-review,
independently checked evidence and residual limits.

## Why application flush is insufficient here

In the pinned [WinBtrfs v1.10 source](https://github.com/maharmstone/btrfs/tree/v1.10),
`drv_flush_buffers` flushes file cache data without guaranteeing a filesystem transaction
commit; the volume-handle flush path itself does not perform that commit. `lock_volume`
invokes the filesystem write/commit path. The bridge therefore supports **successful orderly
volume lock/dismount close** as its filesystem commit boundary. The actual tests demonstrate
that a file flush can succeed and still lose the new file after an abrupt termination.

The bridge's backing-file flush after each completed **block** write is a separate boundary.
It cannot make a filesystem transaction durable before the filesystem sends it to the bridge.
Databases and other workloads relying on fsync/FlushFileBuffers durability are outside this
experimental profile. This patch does not change WinBtrfs or claim to repair its flush semantics.

## Evidence chain and cut precision

The candidate is the hash-verified executable from
[CI 34749762390](https://github.com/DPS0340/winluks/actions/runs/34749762390), application commit
`d4f670a2e67240b62dea86dccc5f44f5fdd5618b`, SHA256
`e4c66309d46c6bf6e6f8c73630596f3e426472704050cc531a22202b1405d0b8`.
Release packaging verifies that application source and CI build inputs have not changed since
that build, and verifies the downloaded CI artifact's provenance and bytes. Later changes
are tests, evidence, review reports, packaging and user documentation.

The Windows 11 Enterprise Evaluation 25H2 `26200.6584` guest runs WinSpd 1.0.20357 and
WinBtrfs 1.10, with Secure Boot/HVCI/VBS off. Its SATA qcow2 OS disk uses `cache=none` and
`aio=threads`; the 256 MiB LUKS2 image is a local NTFS file with a 240 MiB plaintext payload.
Each run starts from a fresh overlay of a powered-off base and matching UEFI/TPM state.
Backing-chain size and modification timestamps remained unchanged through the final matrix.

The host verifies the exact trial QEMU process before SIGKILL. It attaches the **unbooted**
crash disk read-only to Debian 13 and extracts the LUKS image from read-only NTFS. The
Windows evaluation guest's existing clear-key, used-space BitLocker state is opened using
read-only dislocker; no new credentials or writable outer mapping are used. Linux then
opens LUKS with read-only dm-crypt and validates it without log replay or repair.

Raw tests first lock/dismount the target volume, then read all plaintext through the raw
Windows disk handle. Locking can legitimately commit Btrfs metadata, so the original
pre-mount plaintext is not the correct baseline for this particular persistence comparison.
The host captures and authenticates the complete prewrite baseline **before** sending a go
marker; only then does Windows issue the three writes. The Linux oracle verifies the
baseline hash/length and exact `(offset, length, seed)` observations, and compares every
byte after independent dm-crypt decryption. The baseline uses the Windows bridge read path;
the separate 30-case differential crypto oracle instead starts from independently generated
Linux plaintext. No metadata region is omitted from the raw comparison.

Streaming cuts occur after the first observed write event. Process-kill scenarios allow
additional activity while the host kills the publisher and checks device removal. Close
cuts occur after sending Ctrl+C, without claiming a particular instruction within lock,
flush or dismount. These tests do **not** deterministically stop an in-flight backend sector
write. Raw workloads keep the lock, dismount and raw handle alive indefinitely until the cut.
An independent process-handle observer supplies actual orderly/failed close exit codes;
the console SSH exit code is not used as a substitute.

## Preliminary attempts retained

- `normal-01`: Linux QMP hotplug setup was incomplete; the preliminary base was later booted.
  This disk is excluded from durability evidence. A new frozen base and final runs replaced it.
- `normal-02`: Ctrl+C canceled the console wrapper before it could print the application's
  exit status. Replaced with the independent process-handle observer; not counted as a pass.
- `raw-01`: buffered stdout combined with `select()` could hide completion until after a
  finite held-lock interval. The timing claim was not reliable. Replaced with unbuffered
  observation, an infinite held-lock wait and an alive-at-cut check.
- `raw-02`: full comparison failed because volume locking had already committed Btrfs
  metadata relative to the original Linux baseline. All three tested write ranges were
  intact on independent inspection, but the full-oracle failure is retained. The replacement
  records the precise prewrite state and still compares the entire payload.
- `raw-03`: the newly created baseline sidecar was missing from the no-replay NTFS crash
  view. Verification was incomplete. The replacement captures and authenticates the baseline
  on the host before permitting writes, so it does not rely on that directory entry surviving.

## Limits and remaining work

QEMU SIGKILL stops the guest and hypervisor while the host kernel, storage caches and devices
remain powered. It is **not physical host/controller/device power removal** and does not
establish atomic sector writes. This finite, repeated virtual matrix covers the listed
observed triggers; it is not exhaustive over all possible crash timings or Windows caching
states. Mocked syscall/native-boundary injections are also distinct from physical faults.

Host disk-full, outer NTFS-full, deterministic sector tearing, arbitrary delayed/reordered
writes, device firmware caches, kernel mutation tracing and long-duration random crash
campaigns remain untested. ext4 Windows publication remains gated off. Real user disks,
automatic repair, authenticated encryption, default Secure Boot/HVCI compatibility and an
external cryptographic/security audit are outside this release's claims.
