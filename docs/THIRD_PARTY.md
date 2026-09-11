# Third-party implementation and build inputs

| Component | Fixed reference | Use |
|---|---|---|
| WinSpd | `55c53bc454afbbba38bd1692f52beb77ed59142f`, v1.0B1 / 1.0.20357 | Existing user-mode API and kernel storage driver |
| WinBtrfs | design reference `a0648190b4b238ebc016ce247d91ccc814a8633b`, test candidate v1.10 | Separate, unmodified filesystem driver |
| Ext4Fsd | design reference `da27068c3e22f605caa5e7e52fb3535ae37031d0`, test candidate v0.71 | Separate, unmodified filesystem driver |
| cryptsetup | `ff3f1723e0e65003e35b82094642b0f0a3583ddb` | Format reference and independent Linux oracle; not linked |
| Linux ext4/Btrfs on-disk definitions | `50d05c7c76c96b90462f24debacca971d2e86713` | Format and policy reference |
| OpenSSL Rust bindings and OpenSSL | `Cargo.lock`; host OpenSSL version in validation record | AES-XTS, PBKDF2, SHA-256/512 |
| RustCrypto Argon2 | `Cargo.lock` | Argon2i/id v0x13 with caller-owned zeroizing workspace |
| zeroize / subtle / crc32c | `Cargo.lock` | Buffer cleanup, constant-time comparison, filesystem checksums |

WinSpd headers and DLL import library are obtained from upstream at build time. WinSpd has
GPLv3/commercial terms and its FOSS exception; review the upstream license before packaging.
Ext4Fsd is a separate GPLv2 driver. Drivers and Windows media are not checked into Git.
This repository uses GPL-3.0-or-later, including AF merge logic informed by cryptsetup's
GPL-2.0-or-later implementation. Preserve upstream notices when redistributing upstream code.

OpenSSL's context is freed through its Rust binding. The application zeroizes passwords,
derived keys, AF state, plaintext staging and the volume key. This does not erase Windows
caches/pagefiles/crash dumps or guarantee cleanup after process termination. A dedicated
page-aligned key allocation is locked where possible; failure is reported, not hidden.

Source revisions and binary release identities are recorded separately. A release name or
successful signature check is not proof of HVCI compatibility or the absence of hidden writes.
