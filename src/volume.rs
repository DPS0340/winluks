use crate::{
    Error, Result,
    crypto::{self, VolumeKey},
    image::Image,
    metadata::Metadata,
    probe::{self, Filesystem},
};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

pub const MAX_TRANSFER: usize = 1024 * 1024;
pub struct UnlockedVolume {
    image: Image,
    offset: u64,
    len: u64,
    key: VolumeKey,
}
pub struct ValidatedVolume {
    volume: UnlockedVolume,
    filesystem: Filesystem,
}
impl UnlockedVolume {
    pub fn unlock(image: Image, keyslot: u32, password: &[u8]) -> Result<Self> {
        // A metadata plan cannot be substituted from another image: parse the owned handle.
        let m = Metadata::read(&image)?;
        if password.len() > 4096 {
            return Err(Error::ResourceLimit);
        }
        std::str::from_utf8(password).map_err(|_| Error::UnlockFailed)?;
        let s = m.slot(keyslot)?;
        let derived = s.kdf.derive(password, s.area_key_size)?;
        let mut encrypted = Zeroizing::new(vec![0u8; s.key_size * 4000]);
        image.read_exact_at(s.area_offset, &mut encrypted)?;
        let af = crypto::decrypt_sectors(&derived, 0, &encrypted)?;
        let key = crypto::af_merge(&af, s.key_size, s.af_hash)?;
        let digest = s.digest_kdf.derive(&key, s.digest.len())?;
        if !bool::from(digest.as_slice().ct_eq(s.digest.as_slice())) {
            return Err(Error::UnlockFailed);
        }
        let key = VolumeKey::new(&key)?;
        Ok(Self {
            image,
            offset: m.offset,
            len: m.length,
            key,
        })
    }
    pub fn len(&self) -> u64 {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn key_is_locked(&self) -> bool {
        self.key.is_locked()
    }
    pub fn read_at(&self, offset: u64, len: usize) -> Result<Zeroizing<Vec<u8>>> {
        checked_range(self.len, offset, len)?;
        if len == 0 {
            return Ok(Zeroizing::new(Vec::new()));
        }
        let begin = offset / 512 * 512;
        let end = offset
            .checked_add(len as u64)
            .and_then(|e| e.checked_add(511))
            .ok_or(Error::InvalidRange)?
            / 512
            * 512;
        // At most one extra sector for a non-aligned probe read.
        if end - begin > MAX_TRANSFER as u64 {
            return Err(Error::InvalidRange);
        }
        let mut encrypted = Zeroizing::new(vec![0; (end - begin) as usize]);
        self.image.read_exact_at(
            self.offset.checked_add(begin).ok_or(Error::InvalidRange)?,
            &mut encrypted,
        )?;
        let plain = crypto::decrypt_sectors(self.key.bytes(), begin / 512, &encrypted)?;
        let start = (offset - begin) as usize;
        Ok(Zeroizing::new(plain[start..start + len].to_vec()))
    }
    pub fn validate(self, filesystem: Filesystem) -> Result<ValidatedVolume> {
        probe::check(self.len, filesystem, |o, n| self.read_at(o, n))?;
        Ok(ValidatedVolume {
            volume: self,
            filesystem,
        })
    }
}
impl ValidatedVolume {
    pub fn len(&self) -> u64 {
        self.volume.len()
    }
    pub fn is_empty(&self) -> bool {
        self.volume.is_empty()
    }
    pub fn filesystem(&self) -> Filesystem {
        self.filesystem
    }
    pub fn key_is_locked(&self) -> bool {
        self.volume.key_is_locked()
    }
    pub fn read_at(&self, offset: u64, len: usize) -> Result<Zeroizing<Vec<u8>>> {
        self.volume.read_at(offset, len)
    }
}
pub(crate) fn checked_range(total: u64, offset: u64, len: usize) -> Result<()> {
    if len > MAX_TRANSFER || offset > total || len as u64 > total - offset {
        Err(Error::InvalidRange)
    } else {
        Ok(())
    }
}
