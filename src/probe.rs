use crate::{Error, Result};
use zeroize::Zeroizing;
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Filesystem {
    Btrfs,
    Ext4,
}
fn u16le(b: &[u8], p: usize) -> u16 {
    u16::from_le_bytes(b[p..p + 2].try_into().expect("fixed field"))
}
fn u32le(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().expect("fixed field"))
}
fn u64le(b: &[u8], p: usize) -> u64 {
    u64::from_le_bytes(b[p..p + 8].try_into().expect("fixed field"))
}

pub(crate) fn check<F>(length: u64, fs: Filesystem, mut read: F) -> Result<()>
where
    F: FnMut(u64, usize) -> Result<Zeroizing<Vec<u8>>>,
{
    match fs {
        Filesystem::Ext4 => ext4(&read(1024, 1024)?, length),
        Filesystem::Btrfs => btrfs(&read(65536, 4096)?, length),
    }
}
/// E0 is a deliberately narrower allowlist than ext4's general forward-compatibility rules.
pub fn ext4(s: &[u8], length: u64) -> Result<()> {
    if s.len() != 1024 {
        return Err(Error::FsInvalid);
    }
    if u16le(s, 0x38) != 0xef53 {
        return Err(Error::FsTypeMismatch);
    }
    let compat = u32le(s, 0x5c);
    let incompat = u32le(s, 0x60);
    let ro = u32le(s, 0x64);
    if s[0x175] != 1 {
        return Err(Error::FsUnsupportedFeature);
    }
    // ext4 stores the CRC32c running state (seed ~0), without the final complement.
    if !crc32c::crc32c(&s[..0x3fc]) != u32le(s, 0x3fc) {
        return Err(Error::FsInvalid);
    }
    if u16le(s, 0x3a) != 1
        || incompat & 0x4 != 0
        || ro & 0x10000 != 0
        || u32le(s, 0xe8) != 0
        || u32le(s, 0x280) != 0
    {
        return Err(Error::FsRecoveryRequired);
    }
    const COMPAT: u32 = 0x4 | 0x8 | 0x10 | 0x20;
    const INCOMPAT: u32 = 0x2 | 0x40 | 0x80 | 0x200 | 0x2000;
    const RO: u32 = 0x1 | 0x2 | 0x8 | 0x20 | 0x40 | 0x400;
    if compat & !COMPAT != 0
        || incompat & !INCOMPAT != 0
        || ro & !RO != 0
        || compat & 4 == 0
        || incompat & 0x42 != 0x42
        || ro & 0x400 == 0
    {
        return Err(Error::FsUnsupportedFeature);
    }
    if u32le(s, 0x4c) != 1 || u16le(s, 0x58) != 256 || u32le(s, 0x18) != 2 {
        return Err(Error::FsUnsupportedFeature);
    }
    if u32le(s, 0x1c) != 2 || u32le(s, 0x14) != 0 {
        return Err(Error::FsInvalid);
    }
    if u32le(s, 0xe0) == 0 || u32le(s, 0xe4) != 0 || s[0xd0..0xe0].iter().any(|b| *b != 0) {
        return Err(Error::FsUnsupportedFeature);
    }
    let high = u32le(s, 0x150) as u64;
    let has64 = incompat & 0x80 != 0;
    if (has64 && u16le(s, 0xfe) != 64) || (!has64 && high != 0) {
        return Err(Error::FsInvalid);
    }
    let blocks = u32le(s, 4) as u64 | if has64 { high << 32 } else { 0 };
    let bytes = blocks.checked_mul(4096).ok_or(Error::FsInvalid)?;
    let bpg = u32le(s, 0x20) as u64;
    let ipg = u32le(s, 0x28) as u64;
    let inodes = u32le(s, 0) as u64;
    if bytes > length
        || blocks == 0
        || bpg == 0
        || bpg > 32768
        || u32le(s, 0x24) as u64 != bpg
        || ipg == 0
        || ipg > 32768
        || !ipg.is_multiple_of(16)
    {
        return Err(Error::FsInvalid);
    }
    let groups = blocks.div_ceil(bpg);
    if inodes == 0
        || inodes != groups.checked_mul(ipg).ok_or(Error::FsInvalid)?
        || u32le(s, 0xe0) as u64 > inodes
        || u32le(s, 0x54) < 11
    {
        return Err(Error::FsInvalid);
    }
    if u32le(s, 0x10) as u64 > inodes {
        return Err(Error::FsInvalid);
    }
    let free = u32le(s, 0xc) as u64
        | if has64 {
            (u32le(s, 0x158) as u64) << 32
        } else {
            0
        };
    if free > blocks {
        return Err(Error::FsInvalid);
    }
    Ok(())
}
/// Initial B0: CRC32c, 4 KiB sectors, single device, no outstanding log tree.
pub fn btrfs(s: &[u8], length: u64) -> Result<()> {
    if s.len() != 4096 {
        return Err(Error::FsInvalid);
    }
    if &s[0x40..0x48] != b"_BHRfS_M" {
        return Err(Error::FsTypeMismatch);
    }
    if u16le(s, 0xc4) != 0 {
        return Err(Error::FsUnsupportedFeature);
    }
    if crc32c::crc32c(&s[32..]) != u32le(s, 0) || s[4..32].iter().any(|b| *b != 0) {
        return Err(Error::FsInvalid);
    }
    if u64le(s, 0x30) != 65536
        || u64le(s, 0x70) > length
        || u64le(s, 0x70) == 0
        || u64le(s, 0x78) > u64le(s, 0x70)
    {
        return Err(Error::FsInvalid);
    }
    if u64le(s, 0x88) != 1 {
        return Err(Error::FsUnsupportedFeature);
    }
    if u64le(s, 0x60) != 0 {
        return Err(Error::FsRecoveryRequired);
    }
    // MIXED_BACKREF, DEFAULT_SUBVOL, COMPRESS_LZO, BIG_METADATA, EXTENDED_IREF,
    // SKINNY_METADATA, NO_HOLES, COMPRESS_ZSTD. No mixed groups, RAID or zoned mode.
    const ALLOWED: u64 = 1 | 2 | 8 | 16 | 32 | 64 | 256 | 512;
    if u64le(s, 0xac) != 0 || u64le(s, 0xb4) != 0 || u64le(s, 0xbc) & !ALLOWED != 0 {
        return Err(Error::FsUnsupportedFeature);
    }
    let node = u32le(s, 0x94);
    if u32le(s, 0x90) != 4096 || ![4096, 8192, 16384, 32768, 65536].contains(&node) {
        return Err(Error::FsUnsupportedFeature);
    }
    if u32le(s, 0xa0) > 2048 || s[0xc6] > 8 || s[0xc7] > 8 || s[0xc8] > 8 {
        return Err(Error::FsInvalid);
    }
    Ok(())
}
