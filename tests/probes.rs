use winluks::{Error, Result, probe};

// Exercise the actual publication entry point without adding a public library API.
#[allow(dead_code)]
#[path = "../src/probe.rs"]
mod publication_probe;
const EXT: &[u8] = include_bytes!("data/ext4-superblock.bin");
const BTR: &[u8] = include_bytes!("data/btrfs-superblock.bin");
const LEN: u64 = 240 * 1024 * 1024;
fn ext_change(f: impl FnOnce(&mut [u8])) -> Vec<u8> {
    let mut b = EXT.to_vec();
    f(&mut b);
    let c = !crc32c::crc32c(&b[..1020]);
    b[1020..].copy_from_slice(&c.to_le_bytes());
    b
}
fn btr_change(f: impl FnOnce(&mut [u8])) -> Vec<u8> {
    let mut b = BTR.to_vec();
    f(&mut b);
    let c = crc32c::crc32c(&b[32..]);
    b[..4].copy_from_slice(&c.to_le_bytes());
    b
}
#[test]
fn real_linux_superblocks() {
    assert_eq!(probe::ext4(EXT, LEN), Ok(()));
    assert_eq!(probe::btrfs(BTR, LEN), Ok(()));
}
#[test]
fn ext4_corrupt_checksum() {
    let mut b = EXT.to_vec();
    b[8] ^= 1;
    assert_eq!(probe::ext4(&b, LEN), Err(Error::FsInvalid));
}
#[test]
fn ext4_recovery_conditions() {
    for (o, value) in [(0x3a, 0u32), (0xe8, 12), (0x280, 12)] {
        let b = ext_change(|b| {
            if o == 0x3a {
                b[o..o + 2].copy_from_slice(&(value as u16).to_le_bytes())
            } else {
                b[o..o + 4].copy_from_slice(&value.to_le_bytes())
            }
        });
        assert_eq!(probe::ext4(&b, LEN), Err(Error::FsRecoveryRequired));
    }
    let b = ext_change(|b| b[0x60] |= 4);
    assert_eq!(probe::ext4(&b, LEN), Err(Error::FsRecoveryRequired));
}
#[test]
fn every_unlisted_ext4_feature_rejected() {
    for (offset, allowed) in [(0x5c, 0x3cu32), (0x60, 0x22c2), (0x64, 0x46b)] {
        for bit in 0..32 {
            if allowed & (1 << bit) != 0 {
                continue;
            }
            let b = ext_change(|b| {
                let v = u32::from_le_bytes(b[offset..offset + 4].try_into().unwrap()) | (1 << bit);
                b[offset..offset + 4].copy_from_slice(&v.to_le_bytes());
            });
            assert!(
                matches!(
                    probe::ext4(&b, LEN),
                    Err(Error::FsUnsupportedFeature | Error::FsRecoveryRequired)
                ),
                "offset={offset:x} bit={bit}"
            );
        }
    }
}
#[test]
fn ext4_required_features_and_geometry() {
    for (o, mask) in [(0x5c, 4u32), (0x60, 2), (0x60, 0x40), (0x64, 0x400)] {
        let b = ext_change(|b| {
            let v = u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) & !mask;
            b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        });
        assert_eq!(probe::ext4(&b, LEN), Err(Error::FsUnsupportedFeature));
    }
    for o in [4, 0x20, 0x28] {
        let b = ext_change(|b| b[o..o + 4].fill(0));
        assert_eq!(probe::ext4(&b, LEN), Err(Error::FsInvalid));
    }
    let b = ext_change(|b| b[0x150..0x154].fill(255));
    assert_eq!(probe::ext4(&b, LEN), Err(Error::FsInvalid));
}
#[test]
fn wrong_types_and_truncation() {
    assert_eq!(probe::ext4(&EXT[..1000], LEN), Err(Error::FsInvalid));
    let b = ext_change(|b| b[0x38] = 0);
    assert_eq!(probe::ext4(&b, LEN), Err(Error::FsTypeMismatch));
    let b = btr_change(|b| b[0x40] = 0);
    assert_eq!(probe::btrfs(&b, LEN), Err(Error::FsTypeMismatch));
}
#[test]
fn btrfs_policy() {
    let b = btr_change(|b| b[0x38] |= 4);
    assert_eq!(probe::btrfs(&b, LEN), Err(Error::FsRecoveryRequired));
    let b = btr_change(|b| b[0x88] = 2);
    assert_eq!(probe::btrfs(&b, LEN), Err(Error::FsUnsupportedFeature));
    let b = btr_change(|b| b[0x60] = 1);
    assert_eq!(probe::btrfs(&b, LEN), Err(Error::FsRecoveryRequired));
    let b = btr_change(|b| b[0xbd] |= 4);
    assert_eq!(probe::btrfs(&b, LEN), Err(Error::FsUnsupportedFeature));
}

const BTRFS_MIRRORS: [u64; 3] = [65_536, 67_108_864, 274_877_906_944];

fn btrfs_mirrors(length: u64) -> std::collections::BTreeMap<u64, Vec<u8>> {
    BTRFS_MIRRORS
        .into_iter()
        .filter(|offset| *offset + 4096 <= length)
        .map(|offset| {
            let bytes = btr_change(|s| s[0x30..0x38].copy_from_slice(&offset.to_le_bytes()));
            (offset, bytes)
        })
        .collect()
}

fn check_btrfs_volume(
    mirrors: &std::collections::BTreeMap<u64, Vec<u8>>,
    length: u64,
) -> (Result<()>, Vec<u64>) {
    let mut reads = Vec::new();
    let result = publication_probe::check(
        length,
        publication_probe::Filesystem::Btrfs,
        |offset, count| {
            reads.push(offset);
            assert_eq!(count, 4096);
            mirrors
                .get(&offset)
                .cloned()
                .map(zeroize::Zeroizing::new)
                .ok_or(Error::BackendIo)
        },
    );
    (result, reads)
}

fn update_mirror(mirror: &mut [u8], mutate: impl FnOnce(&mut [u8])) {
    mutate(mirror);
    let checksum = crc32c::crc32c(&mirror[32..]);
    mirror[..4].copy_from_slice(&checksum.to_le_bytes());
}

#[test]
fn btrfs_publication_checks_every_available_mirror() {
    // Virtual reads cover the 256 GiB mirror without allocating a large image.
    for length in [LEN, BTRFS_MIRRORS[2] + 4096] {
        let mirrors = btrfs_mirrors(length);
        let (result, reads) = check_btrfs_volume(&mirrors, length);
        assert_eq!(result, Ok(()));
        assert_eq!(reads, mirrors.keys().copied().collect::<Vec<_>>());
    }
}

#[test]
fn btrfs_publication_rejects_newer_mirror_with_log_tree() {
    let mut mirrors = btrfs_mirrors(LEN);
    update_mirror(mirrors.get_mut(&BTRFS_MIRRORS[1]).unwrap(), |s| {
        let generation = u64::from_le_bytes(s[0x48..0x50].try_into().unwrap());
        s[0x48..0x50].copy_from_slice(&(generation + 1).to_le_bytes());
        s[0x60..0x68].copy_from_slice(&4096u64.to_le_bytes());
    });
    // WinBtrfs v1.10 read_superblock selects the greatest checksum-valid generation.
    // The primary remains clean: validating it alone accepts this recovery image.
    let (result, _) = check_btrfs_volume(&mirrors, LEN);
    assert!(matches!(result, Err(Error::FsRecoveryRequired)));
}

#[test]
fn btrfs_publication_rejects_divergent_mirrors() {
    for changed_field in [0x20, 0x48, 0x50, 0xbc] {
        let mut mirrors = btrfs_mirrors(LEN);
        update_mirror(mirrors.get_mut(&BTRFS_MIRRORS[1]).unwrap(), |s| {
            // Filesystem identity, generation, tree root, or supported feature policy.
            s[changed_field] ^= if changed_field == 0x50 { 0x10 } else { 1 };
        });
        let (result, _) = check_btrfs_volume(&mirrors, LEN);
        assert!(
            result.is_err(),
            "accepted divergent field {changed_field:#x}"
        );
    }
}

#[test]
fn btrfs_publication_rejects_damaged_or_unreadable_mirrors() {
    let mut mirrors = btrfs_mirrors(LEN);
    mirrors.get_mut(&BTRFS_MIRRORS[1]).unwrap()[0] ^= 1;
    assert!(check_btrfs_volume(&mirrors, LEN).0.is_err());
    mirrors.remove(&BTRFS_MIRRORS[1]);
    assert_eq!(check_btrfs_volume(&mirrors, LEN).0, Err(Error::BackendIo));
}
