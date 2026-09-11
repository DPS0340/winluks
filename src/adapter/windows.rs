use super::*;
use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::Arc,
    time::Duration,
};
unsafe extern "C" {
    fn wl_create(
        context: *mut c_void,
        read: unsafe extern "C" fn(*mut c_void, u64, u32, *mut c_void) -> i32,
        control: unsafe extern "C" fn(*mut c_void, u32, u64, u32) -> i32,
        blocks: u64,
        out: *mut *mut c_void,
    ) -> u32;
    fn wl_error(session: *mut c_void) -> u32;
    fn wl_close(session: *mut c_void);
    fn wl_consumer_ready(filesystem: u32, driver: *mut u16, capacity: u32) -> u32;
}
pub(super) fn check_consumer(filesystem: crate::probe::Filesystem) -> Result<()> {
    use crate::probe::Filesystem;
    use std::os::windows::ffi::OsStringExt;
    let (id, expected) = match filesystem {
        Filesystem::Btrfs => (
            0,
            "3c46f0f82726e374cec3d2e36defd2c672c68903895a510a29762f6f93866797",
        ),
        Filesystem::Ext4 => (
            1,
            "06f6b4a6bc7aaf568d0442a3415394b2b7806c1bd94d35b314bebfa0993898d9",
        ),
    };
    let mut path = [0u16; 32768];
    if unsafe { wl_consumer_ready(id, path.as_mut_ptr(), path.len() as u32) } != 0 {
        return Err(Error::FsDriverUnavailable);
    }
    let n = path
        .iter()
        .position(|c| *c == 0)
        .ok_or(Error::FsDriverUnavailable)?;
    let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&path[..n]));
    let image = crate::image::Image::open(&path).map_err(|_| Error::FsDriverUnavailable)?;
    if image.is_empty() || image.len() > 16 * 1024 * 1024 {
        return Err(Error::FsDriverUnavailable);
    }
    let mut bytes = vec![0; image.len() as usize];
    image
        .read_exact_at(0, &mut bytes)
        .map_err(|_| Error::FsDriverUnavailable)?;
    let digest = openssl::sha::sha256(&bytes);
    let actual: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    if actual != expected {
        return Err(Error::FsDriverUnavailable);
    }
    Ok(())
}
fn code(r: Result<()>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(Error::ReadOnly) => 1,
        Err(Error::InvalidRange) => 2,
        Err(Error::Stopping) => 3,
        Err(_) => 4,
    }
}
unsafe extern "C" fn read_cb(p: *mut c_void, lba: u64, count: u32, buffer: *mut c_void) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if p.is_null() || count > 2048 || (count != 0 && buffer.is_null()) {
            return 2;
        }
        // Clear before calling Rust, so even a caught panic cannot expose stale plaintext.
        if count != 0 {
            unsafe { ptr::write_bytes(buffer.cast::<u8>(), 0, count as usize * 512) };
        }
        let a = unsafe { &*p.cast::<ReadOnlyAdapter>() };
        match a.read(lba, count) {
            Ok(data) => {
                if !data.is_empty() {
                    unsafe {
                        ptr::copy_nonoverlapping(data.as_ptr(), buffer.cast(), data.len());
                    }
                }
                0
            }
            Err(e) => {
                if count != 0 {
                    unsafe {
                        ptr::write_bytes(buffer.cast::<u8>(), 0, count as usize * 512);
                    }
                }
                code(Err(e))
            }
        }
    }))
    .unwrap_or(4)
}
unsafe extern "C" fn control_cb(p: *mut c_void, op: u32, lba: u64, count: u32) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if p.is_null() {
            return 4;
        }
        let a = unsafe { &*p.cast::<ReadOnlyAdapter>() };
        code(match op {
            1 => a.write(),
            2 => a.flush(lba, count),
            3 => a.unmap(),
            _ => Err(Error::InvalidRange),
        })
    }))
    .unwrap_or(4)
}
pub fn serve(volume: ValidatedVolume) -> Result<()> {
    super::check_consumer(volume.filesystem())?;
    let mut adapter = Box::new(ReadOnlyAdapter::new(volume));
    let quit = Arc::new(AtomicBool::new(false));
    let q = quit.clone();
    ctrlc::set_handler(move || q.store(true, Ordering::Release))
        .map_err(|_| Error::DevicePublishFailed)?;
    let mut session = ptr::null_mut();
    let rc = unsafe {
        wl_create(
            (&mut *adapter as *mut ReadOnlyAdapter).cast(),
            read_cb,
            control_cb,
            adapter.len() / 512,
            &mut session,
        )
    };
    if rc != 0 {
        eprintln!("WINSPD_CREATE_ERROR code={rc}");
        return Err(Error::DevicePublishFailed);
    }
    eprintln!("PUBLISHED_RO (Ctrl+C to close)");
    while !quit.load(Ordering::Acquire) && unsafe { wl_error(session) } == 0 {
        std::thread::sleep(Duration::from_millis(100));
    }
    adapter.stop();
    let failed = unsafe { wl_error(session) } != 0;
    unsafe { wl_close(session) }; // Returns only after callbacks have drained; Box remains alive.
    eprintln!(
        "CLOSED reads={} write_callbacks={} unmap_callbacks={}",
        adapter.reads.load(Ordering::Relaxed),
        adapter.write_attempts.load(Ordering::Relaxed),
        adapter.unmap_attempts.load(Ordering::Relaxed)
    );
    if failed {
        Err(Error::DevicePublishFailed)
    } else {
        Ok(())
    }
}
