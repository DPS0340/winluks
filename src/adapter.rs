use crate::{Error, Result, image::AccessMode, volume::ValidatedVolume};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use zeroize::Zeroizing;

pub struct BlockAdapter {
    volume: ValidatedVolume,
    stopping: AtomicBool,
    pub reads: AtomicU64,
    pub write_attempts: AtomicU64,
    pub unmap_attempts: AtomicU64,
    pub flushes: AtomicU64,
}
impl BlockAdapter {
    pub fn new(volume: ValidatedVolume) -> Self {
        Self {
            volume,
            stopping: AtomicBool::new(false),
            reads: AtomicU64::new(0),
            write_attempts: AtomicU64::new(0),
            unmap_attempts: AtomicU64::new(0),
            flushes: AtomicU64::new(0),
        }
    }
    pub fn len(&self) -> u64 {
        self.volume.len()
    }
    pub fn is_empty(&self) -> bool {
        self.volume.is_empty()
    }
    pub fn mode(&self) -> AccessMode {
        self.volume.mode()
    }
    pub fn faulted(&self) -> bool {
        self.volume.faulted()
    }
    #[cfg(any(test, all(windows, feature = "winspd")))]
    pub(crate) fn mark_faulted(&self) {
        self.volume.mark_faulted();
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
    pub fn write(&self, lba: u64, bytes: &[u8]) -> Result<()> {
        self.write_attempts.fetch_add(1, Ordering::Relaxed);
        if self.mode() == AccessMode::ReadOnly {
            return Err(Error::ReadOnly);
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::Stopping);
        }
        let offset = lba.checked_mul(512).ok_or(Error::InvalidRange)?;
        self.volume.write_at(offset, bytes)
    }
    pub fn unmap(&self) -> Result<()> {
        self.unmap_attempts.fetch_add(1, Ordering::Relaxed);
        Err(if self.mode() == AccessMode::ReadOnly {
            Error::ReadOnly
        } else {
            Error::UnsupportedOperation
        })
    }
    pub fn flush(&self, lba: u64, count: u32) -> Result<()> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::Stopping);
        }
        let o = lba.checked_mul(512).ok_or(Error::InvalidRange)?;
        // SCSI SYNCHRONIZE CACHE count=0 means through the end of the device.
        if count == 0 {
            if o <= self.len() {
                self.flushes.fetch_add(1, Ordering::Relaxed);
                return self.volume.flush();
            }
            return Err(Error::InvalidRange);
        }
        let n = u64::from(count) * 512;
        if o > self.len() || n > self.len() - o {
            Err(Error::InvalidRange)
        } else {
            self.flushes.fetch_add(1, Ordering::Relaxed);
            self.volume.flush()
        }
    }
    #[cfg(any(test, all(windows, feature = "winspd")))]
    pub(crate) fn shutdown(&self, drain: impl FnOnce() -> u32) -> bool {
        let sync_failed = self.flush(0, 0).is_err();
        self.stop();
        let transport_error = drain(); // Callback context remains alive until this returns.
        // A callback or dispatcher can fail during drain, after the final flush.
        sync_failed || transport_error != 0 || self.faulted()
    }
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
    }
}
#[cfg(all(windows, feature = "winspd"))]
mod windows;
#[cfg(all(windows, feature = "winspd"))]
pub use windows::serve;
pub fn check_consumer(filesystem: crate::probe::Filesystem, mode: AccessMode) -> Result<()> {
    // G0-E failed discovery on the design's partitionless disk. Keep the crypto/probe
    // available to independent oracles, but never publish this unpassed consumer.
    if filesystem == crate::probe::Filesystem::Ext4 {
        return Err(Error::FsGateUnpassed);
    }
    #[cfg(all(windows, feature = "winspd"))]
    {
        windows::check_consumer(filesystem, mode)
    }
    #[cfg(not(all(windows, feature = "winspd")))]
    {
        let _ = mode;
        Err(Error::FsDriverUnavailable)
    }
}
#[cfg(not(all(windows, feature = "winspd")))]
pub fn serve(_volume: ValidatedVolume) -> Result<()> {
    Err(Error::FsDriverUnavailable)
}
