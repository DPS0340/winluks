use crate::{Error, Result};
use openssl::{
    hash::{Hasher, MessageDigest, hash},
    pkcs5::pbkdf2_hmac,
    symm::{Cipher, Crypter, Mode},
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hash {
    Sha256,
    Sha512,
}
impl Hash {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "sha256" => Ok(Self::Sha256),
            "sha512" => Ok(Self::Sha512),
            _ => Err(Error::UnsupportedProfile),
        }
    }
    pub fn md(self) -> MessageDigest {
        match self {
            Self::Sha256 => MessageDigest::sha256(),
            Self::Sha512 => MessageDigest::sha512(),
        }
    }
    pub fn size(self) -> usize {
        self.md().size()
    }
    pub fn digest(self, b: &[u8]) -> Result<Vec<u8>> {
        Ok(hash(self.md(), b)?.to_vec())
    }
}
#[derive(Clone)]
pub enum Kdf {
    Pbkdf2 {
        hash: Hash,
        iterations: u32,
        salt: Vec<u8>,
    },
    Argon2 {
        id: bool,
        memory: u32,
        time: u32,
        cpus: u32,
        salt: Vec<u8>,
    },
}
impl Kdf {
    pub fn derive(&self, password: &[u8], size: usize) -> Result<Zeroizing<Vec<u8>>> {
        if size == 0 || size > 64 {
            return Err(Error::ResourceLimit);
        }
        let start = std::time::Instant::now();
        let mut out = Zeroizing::new(vec![0u8; size]);
        match self {
            Self::Pbkdf2 {
                hash,
                iterations,
                salt,
            } => {
                if *iterations == 0 || *iterations > 10_000_000 {
                    return Err(Error::ResourceLimit);
                }
                pbkdf2_hmac(password, salt, *iterations as usize, hash.md(), &mut out)?;
            }
            Self::Argon2 {
                id,
                memory,
                time,
                cpus,
                salt,
            } => {
                if *memory > 1_048_576 || *time > 10 || *cpus > 8 {
                    return Err(Error::ResourceLimit);
                }
                let p = argon2::Params::new(*memory, *time, *cpus, Some(size))
                    .map_err(|_| Error::MetadataInvalid)?;
                let mut blocks = Zeroizing::new(Vec::<argon2::Block>::new());
                blocks
                    .try_reserve_exact(p.block_count())
                    .map_err(|_| Error::ResourceLimit)?;
                blocks.resize(p.block_count(), argon2::Block::default());
                let a = argon2::Argon2::new(
                    if *id {
                        argon2::Algorithm::Argon2id
                    } else {
                        argon2::Algorithm::Argon2i
                    },
                    argon2::Version::V0x13,
                    p,
                );
                a.hash_password_into_with_memory(password, salt, &mut out, &mut *blocks)
                    .map_err(|_| Error::Crypto)?;
            }
        }
        if start.elapsed() > std::time::Duration::from_secs(120) {
            return Err(Error::ResourceLimit);
        }
        Ok(out)
    }
}

/// Each 512-byte sector is a separate XTS data unit, even in a large I/O.
pub fn decrypt_sectors(
    key: &[u8],
    first_sector: u64,
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>> {
    let cipher = match key.len() {
        32 => Cipher::aes_128_xts(),
        64 => Cipher::aes_256_xts(),
        _ => return Err(Error::UnsupportedProfile),
    };
    if !ciphertext.len().is_multiple_of(512) || ciphertext.len() > 1024 * 1024 {
        return Err(Error::InvalidRange);
    }
    let mut out = Zeroizing::new(vec![0; ciphertext.len()]);
    let mut tmp = Zeroizing::new([0u8; 528]);
    for (i, sector) in ciphertext.as_chunks::<512>().0.iter().enumerate() {
        let mut iv = [0u8; 16];
        iv[..8].copy_from_slice(
            &first_sector
                .checked_add(i as u64)
                .ok_or(Error::InvalidRange)?
                .to_le_bytes(),
        );
        let mut c = Crypter::new(cipher, Mode::Decrypt, key, Some(&iv))?;
        c.pad(false);
        let n = c.update(sector, &mut *tmp)?;
        let last = c.finalize(&mut tmp[n..])?;
        if n + last != 512 {
            return Err(Error::Crypto);
        }
        out[i * 512..i * 512 + 512].copy_from_slice(&tmp[..512]);
        tmp.zeroize();
    }
    Ok(out)
}

/// LUKS AF merge using library hashes; no hash or cipher primitive is implemented here.
pub fn af_merge(data: &[u8], key_size: usize, hash_alg: Hash) -> Result<Zeroizing<Vec<u8>>> {
    if ![32, 64].contains(&key_size) || data.len() != key_size * 4000 {
        return Err(Error::MetadataInvalid);
    }
    let mut state = Zeroizing::new(vec![0u8; key_size]);
    for stripe in data[..key_size * 3999].chunks_exact(key_size) {
        for (a, b) in state.iter_mut().zip(stripe) {
            *a ^= b;
        }
        for (index, part) in state.chunks_mut(hash_alg.size()).enumerate() {
            let mut h = Hasher::new(hash_alg.md())?;
            h.update(&(index as u32).to_be_bytes())?;
            h.update(part)?;
            let mut digest = h.finish()?;
            part.copy_from_slice(&digest[..part.len()]);
            digest.as_mut().zeroize();
        }
    }
    for (a, b) in state.iter_mut().zip(&data[key_size * 3999..]) {
        *a ^= b;
    }
    Ok(state)
}

/// A dedicated aligned allocation prevents unlocking a page shared with another key.
pub struct VolumeKey {
    ptr: std::ptr::NonNull<u8>,
    size: usize,
    locked: bool,
}
unsafe impl Send for VolumeKey {}
unsafe impl Sync for VolumeKey {}
impl VolumeKey {
    pub fn new(bytes: &[u8]) -> Result<Self> {
        if ![32, 64].contains(&bytes.len()) {
            return Err(Error::Crypto);
        }
        let layout =
            std::alloc::Layout::from_size_align(4096, 4096).map_err(|_| Error::ResourceLimit)?;
        let ptr = std::ptr::NonNull::new(unsafe { std::alloc::alloc_zeroed(layout) })
            .ok_or(Error::ResourceLimit)?;
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.as_ptr(), bytes.len());
        }
        #[cfg(unix)]
        let locked = unsafe { libc::mlock(ptr.as_ptr().cast(), 4096) } == 0;
        #[cfg(windows)]
        let locked =
            unsafe { windows_sys::Win32::System::Memory::VirtualLock(ptr.as_ptr().cast(), 4096) }
                != 0;
        Ok(Self {
            ptr,
            size: bytes.len(),
            locked,
        })
    }
    pub fn is_locked(&self) -> bool {
        self.locked
    }
    pub fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.size) }
    }
}
impl Drop for VolumeKey {
    fn drop(&mut self) {
        unsafe {
            std::slice::from_raw_parts_mut(self.ptr.as_ptr(), 4096).zeroize();
            if self.locked {
                #[cfg(unix)]
                libc::munlock(self.ptr.as_ptr().cast(), 4096);
                #[cfg(windows)]
                windows_sys::Win32::System::Memory::VirtualUnlock(self.ptr.as_ptr().cast(), 4096);
            }
            std::alloc::dealloc(
                self.ptr.as_ptr(),
                std::alloc::Layout::from_size_align(4096, 4096).expect("constant layout"),
            );
        }
    }
}
