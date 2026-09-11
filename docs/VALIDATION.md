# Validation record

Initial implementation session: 2026-09-11. Status is updated from executed commands only.

| Evidence | Status |
|---|---|
| Linux `cargo check` | Passed on Rust 1.98.1, OpenSSL 3.6.4 |
| cryptsetup differential fixtures | Pending |
| Windows MSVC build | Pending |
| G0-B WinSpd + WinBtrfs | Pending |
| G0-E WinSpd + Ext4Fsd | Pending |
| Driver-boundary mutation tracing | Pending |
| Full R01–R14 matrix, fuzzing and independent review | Pending |

No gate is passed merely because code exists or a driver is installed. This repository does
not claim security audit completion or safety for real user disks. G3 is outside this virtual
fixture implementation session; G4 remains unavailable until its independent review and
runtime requirements have been met.
