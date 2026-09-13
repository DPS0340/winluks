# winluks v0.3.1 — experimental Windows x64

Use disposable, trusted local LUKS2 partition images inside the documented isolated VM.
Btrfs RO/RW is supported in that tested profile; ext4 Windows publication is blocked.
The executable is not Authenticode-signed. This package is not a kernel driver or an audit certificate.

Install the pinned WinSpd 1.0.20357 and WinBtrfs v1.10 drivers separately, plus Microsoft's
Visual C++ v14 x64 runtime. Keep the matching `winspd-x64.dll` beside `winluks2.exe` in a trusted
directory. The recorded VM profile uses Secure Boot/HVCI/VBS off; it does not establish support
for the default security configuration of an everyday PC.

From an administrator console:

```powershell
# Default read-only
.\winluks2.exe open --image C:\fixtures\volume.img --keyslot 0 --filesystem btrfs
# Writes modify this disposable copy in place
.\winluks2.exe open --image C:\fixtures\working-copy.img --keyslot 0 --filesystem btrfs --read-write
```

**RW durability limitation:** the pinned WinBtrfs driver does not guarantee that an application's
`FlushFileBuffers` commits its filesystem transaction. Newly written files may disappear after a
crash even if the application reported a successful flush. Databases and workloads relying on
fsync/FlushFileBuffers durability are outside this experimental profile. The supported filesystem
commit boundary is a successful orderly close through the bridge's volume lock and dismount.
The backing-file flush of completed block writes is a separate boundary.

Close files and press Ctrl+C. `CLOSE_BLOCKED` keeps the session alive: close remaining handles
and retry. `CLOSE_FAILED`, `UNCLEAN_CLOSE`, mirror disagreement or a recovery-required error needs
offline inspection of a copy. Do not repeatedly force RW mounts or use repair on the canonical image.
Only one cooperating v0.3.1 publisher can run at a time. Do not concurrently mount same-UUID copies
through older winluks versions or other publishers.

Read `docs/VALIDATION.md`, `docs/POWERLOSS-v0.3.1.md` and the two AI review reports in `docs/reviews/`
for exact results and residual limits. The tests cut QEMU/guest power while the host remains running;
they do not prove host/controller/device power-loss safety or atomic sector writes. No automatic
recovery, physical disks, remote files, resize or discard support is provided.

SHA256 manifests verify package bytes. Corresponding source, vendored Rust/OpenSSL dependencies
and pinned WinSpd source are provided in the source archive beside this ZIP. GPL-3.0-or-later;
see `LICENSE`, `third_party/WinSpd-LICENSE.txt` and `docs/THIRD_PARTY.md`.
