use crate::{
    Error, Result,
    crypto::{self, VolumeKey},
    image::{AccessMode, Image},
    metadata::Metadata,
    probe::{self, Filesystem},
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

pub const MAX_TRANSFER: usize = 1024 * 1024;
pub struct UnlockedVolume {
    image: Image,
    offset: u64,
    len: u64,
    key: VolumeKey,
    io: Mutex<()>,
    faulted: AtomicBool,
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
            io: Mutex::new(()),
            faulted: AtomicBool::new(false),
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
        let _guard = self.io.lock().map_err(|_| Error::WritebackFailed)?;
        if self.faulted.load(Ordering::Acquire) {
            return Err(Error::WritebackFailed);
        }
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
    pub fn mode(&self) -> AccessMode {
        self.volume.image.mode()
    }
    pub fn faulted(&self) -> bool {
        self.volume.faulted.load(Ordering::Acquire)
    }
    pub(crate) fn mark_faulted(&self) {
        self.volume.faulted.store(true, Ordering::Release);
    }
    /// Acknowledged writes are synchronous and durable at the backing file boundary.
    /// Only aligned payload data can change; LUKS metadata/keyslots are outside this range.
    pub fn write_at(&self, offset: u64, plaintext: &[u8]) -> Result<()> {
        if self.mode() != AccessMode::ReadWrite {
            return Err(Error::ReadOnly);
        }
        checked_range(self.len(), offset, plaintext.len())?;
        if !offset.is_multiple_of(512) || !plaintext.len().is_multiple_of(512) {
            return Err(Error::InvalidRange);
        }
        let _guard = self.volume.io.lock().map_err(|_| Error::WritebackFailed)?;
        if self.faulted() {
            return Err(Error::WritebackFailed);
        }
        if plaintext.is_empty() {
            return Ok(());
        }
        let encrypted = crypto::encrypt_sectors(self.volume.key.bytes(), offset / 512, plaintext)?;
        let target = self
            .volume
            .offset
            .checked_add(offset)
            .ok_or(Error::InvalidRange)?;
        if self
            .volume
            .image
            .write_all_at(target, &encrypted)
            .and_then(|_| self.volume.image.sync())
            .is_err()
        {
            self.mark_faulted();
            return Err(Error::WritebackFailed);
        }
        Ok(())
    }
    pub fn flush(&self) -> Result<()> {
        let _guard = self.volume.io.lock().map_err(|_| Error::WritebackFailed)?;
        if self.faulted() {
            return Err(Error::WritebackFailed);
        }
        if self.volume.image.sync().is_err() {
            self.mark_faulted();
            return Err(Error::WritebackFailed);
        }
        Ok(())
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::BlockAdapter;
    use std::fs;

    // Construct only the payload boundary for range/lifecycle tests. Cipher compatibility
    // is checked separately against Linux dm-crypt, not this helper's encrypt/decrypt pair.
    fn session(mode: AccessMode) -> (tempfile::TempDir, std::path::PathBuf, ValidatedVolume) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("volume.img");
        fs::write(&path, vec![0xa5; 1024 + 4096 + 512]).unwrap();
        let key: Vec<u8> = (0..64).collect();
        let v = ValidatedVolume {
            volume: UnlockedVolume {
                image: Image::open_with_mode(&path, mode).unwrap(),
                offset: 1024,
                len: 4096,
                key: VolumeKey::new(&key).unwrap(),
                io: Mutex::new(()),
                faulted: AtomicBool::new(false),
            },
            filesystem: Filesystem::Btrfs,
        };
        (dir, path, v)
    }

    #[test]
    fn writes_preserve_headers_trailer_and_geometry() {
        let (_dir, path, v) = session(AccessMode::ReadWrite);
        v.write_at(0, &[0x11; 512]).unwrap();
        v.write_at(3072, &[0x22; 1024]).unwrap();
        assert_eq!(&*v.read_at(0, 512).unwrap(), &[0x11; 512]);
        assert_eq!(&*v.read_at(3072, 1024).unwrap(), &[0x22; 1024]);
        for (o, n) in [
            (1, 512),
            (0, 511),
            (4096, 512),
            (u64::MAX, 512),
            (0, MAX_TRANSFER + 512),
        ] {
            assert_eq!(v.write_at(o, &vec![0; n]), Err(Error::InvalidRange));
        }
        v.write_at(4096, &[]).unwrap();
        v.flush().unwrap();
        drop(v);
        let bytes = fs::read(path).unwrap();
        assert_eq!(bytes.len(), 1024 + 4096 + 512);
        assert!(bytes[..1024].iter().all(|b| *b == 0xa5));
        assert!(bytes[1024 + 512..1024 + 3072].iter().all(|b| *b == 0xa5));
        assert!(bytes[5120..].iter().all(|b| *b == 0xa5));
    }

    #[test]
    fn mode_range_unmap_and_stop_are_enforced() {
        let (_dir, path, v) = session(AccessMode::ReadOnly);
        let a = BlockAdapter::new(v);
        assert_eq!(a.write(0, &[0; 512]), Err(Error::ReadOnly));
        assert_eq!(a.unmap(), Err(Error::ReadOnly));
        drop(a);
        assert!(fs::read(path).unwrap().iter().all(|b| *b == 0xa5));
        let (_dir, _path, v) = session(AccessMode::ReadWrite);
        let a = BlockAdapter::new(v);
        assert_eq!(a.unmap(), Err(Error::UnsupportedOperation));
        assert_eq!(a.write(u64::MAX, &[0; 512]), Err(Error::InvalidRange));
        assert_eq!(a.flush(8, 1), Err(Error::InvalidRange));
        a.flush(8, 0).unwrap();
        a.stop();
        assert_eq!(a.write(0, &[0; 512]), Err(Error::Stopping));
        assert_eq!(a.flush(0, 0), Err(Error::Stopping));
    }

    #[cfg(unix)]
    #[test]
    fn backend_failure_permanently_faults_the_session() {
        let (_dir, path, v) = session(AccessMode::ReadWrite);
        let external = fs::OpenOptions::new().write(true).open(path).unwrap();
        external.set_len(1).unwrap(); // Deliberately ignore the advisory Linux lock.
        assert_eq!(v.write_at(0, &[0; 512]), Err(Error::WritebackFailed));
        assert!(v.faulted());
        external.set_len(5632).unwrap();
        assert_eq!(v.write_at(0, &[0; 512]), Err(Error::WritebackFailed));
        assert_eq!(v.read_at(0, 512), Err(Error::WritebackFailed));
        assert_eq!(v.flush(), Err(Error::WritebackFailed));
    }
}
