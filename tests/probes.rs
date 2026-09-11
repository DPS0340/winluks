use winluks::{Error, probe};
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
