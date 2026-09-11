use crate::{Error, Result, volume::ValidatedVolume};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use zeroize::Zeroizing;

pub struct ReadOnlyAdapter {
    volume: ValidatedVolume,
    stopping: AtomicBool,
    pub reads: AtomicU64,
    pub write_attempts: AtomicU64,
    pub unmap_attempts: AtomicU64,
}
impl ReadOnlyAdapter {
    pub fn new(volume: ValidatedVolume) -> Self {
        Self {
            volume,
            stopping: AtomicBool::new(false),
            reads: AtomicU64::new(0),
            write_attempts: AtomicU64::new(0),
            unmap_attempts: AtomicU64::new(0),
        }
    }
    pub fn len(&self) -> u64 {
        self.volume.len()
    }
    pub fn is_empty(&self) -> bool {
        self.volume.is_empty()
    }
    pub fn read(&self, lba: u64, count: u32) -> Result<Zeroizing<Vec<u8>>> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::Stopping);
        }
        let o = lba.checked_mul(512).ok_or(Error::InvalidRange)?;
        let n = (count as usize)
            .checked_mul(512)
            .ok_or(Error::InvalidRange)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.volume.read_at(o, n)
    }
    pub fn write(&self) -> Result<()> {
        self.write_attempts.fetch_add(1, Ordering::Relaxed);
        Err(Error::ReadOnly)
    }
    pub fn unmap(&self) -> Result<()> {
        self.unmap_attempts.fetch_add(1, Ordering::Relaxed);
        Err(Error::ReadOnly)
    }
    pub fn flush(&self, lba: u64, count: u32) -> Result<()> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::Stopping);
        }
        let o = lba.checked_mul(512).ok_or(Error::InvalidRange)?;
        // SCSI SYNCHRONIZE CACHE count=0 means through the end of the device.
        if count == 0 {
            if o <= self.len() {
                return Ok(());
            }
            return Err(Error::InvalidRange);
        }
        let n = u64::from(count) * 512;
        if o > self.len() || n > self.len() - o {
            Err(Error::InvalidRange)
        } else {
            Ok(())
        }
    }
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
    }
}
#[cfg(all(windows, feature = "winspd"))]
mod windows;
#[cfg(all(windows, feature = "winspd"))]
pub use windows::{check_consumer, serve};
#[cfg(not(all(windows, feature = "winspd")))]
pub fn check_consumer(_filesystem: crate::probe::Filesystem) -> Result<()> {
    Err(Error::FsDriverUnavailable)
}
#[cfg(not(all(windows, feature = "winspd")))]
pub fn serve(_volume: ValidatedVolume) -> Result<()> {
    Err(Error::FsDriverUnavailable)
}
