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
        write: unsafe extern "C" fn(*mut c_void, u64, u32, *const c_void) -> i32,
        control: unsafe extern "C" fn(*mut c_void, u32, u64, u32) -> i32,
        blocks: u64,
        read_only: u32,
        out: *mut *mut c_void,
    ) -> u32;
    fn wl_error(session: *mut c_void) -> u32;
    fn wl_close(session: *mut c_void) -> u32;
    fn wl_consumer_ready(filesystem: u32, read_only: u32, driver: *mut u16, capacity: u32) -> u32;
    fn wl_volume_ready(session: *mut c_void) -> u32;
    fn wl_lock_and_dismount(session: *mut c_void, phase: *mut u32) -> u32;
}
pub(super) fn check_consumer(filesystem: crate::probe::Filesystem, mode: AccessMode) -> Result<()> {
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
    if unsafe {
        wl_consumer_ready(
            id,
            u32::from(mode == AccessMode::ReadOnly),
            path.as_mut_ptr(),
            path.len() as u32,
        )
    } != 0
    {
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
        Err(Error::WritebackFailed) => 5,
        Err(Error::UnsupportedOperation) => 6,
        Err(_) => 4,
    }
}
unsafe extern "C" fn read_cb(p: *mut c_void, lba: u64, count: u32, buffer: *mut c_void) -> i32 {
    if p.is_null() || count > 2048 || (count != 0 && buffer.is_null()) {
        return 2;
    }
    let a = unsafe { &*p.cast::<BlockAdapter>() };
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        // Clear before calling Rust, so even a caught panic cannot expose stale plaintext.
        if count != 0 {
            unsafe { ptr::write_bytes(buffer.cast::<u8>(), 0, count as usize * 512) };
        }
        let a = unsafe { &*p.cast::<BlockAdapter>() };
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
    }));
    match outcome {
        Ok(code) => code,
        Err(_) => {
            a.mark_faulted();
            if count != 0 {
                unsafe { ptr::write_bytes(buffer.cast::<u8>(), 0, count as usize * 512) };
            }
            5
        }
    }
}
unsafe extern "C" fn write_cb(p: *mut c_void, lba: u64, count: u32, buffer: *const c_void) -> i32 {
    if p.is_null() || count > 2048 || (count != 0 && buffer.is_null()) {
        return 2;
    }
    let a = unsafe { &*p.cast::<BlockAdapter>() };
    match catch_unwind(AssertUnwindSafe(|| {
        let bytes = if count == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), count as usize * 512) }
        };
        code(a.write(lba, bytes))
    })) {
        Ok(status) => status,
        Err(_) => {
            a.mark_faulted();
            5
        }
    }
}
unsafe extern "C" fn control_cb(p: *mut c_void, op: u32, lba: u64, count: u32) -> i32 {
    if p.is_null() {
        return 4;
    }
    let a = unsafe { &*p.cast::<BlockAdapter>() };
    match catch_unwind(AssertUnwindSafe(|| {
        code(match op {
            2 => a.flush(lba, count),
            3 => a.unmap(),
            _ => Err(Error::InvalidRange),
        })
    })) {
        Ok(status) => status,
        Err(_) => {
            a.mark_faulted();
            5
        }
    }
}
pub fn serve(volume: ValidatedVolume) -> Result<()> {
    let read_only = volume.mode() == AccessMode::ReadOnly;
    super::check_consumer(volume.filesystem(), volume.mode())?;
    let mut adapter = Box::new(BlockAdapter::new(volume));
    let quit = Arc::new(AtomicBool::new(false));
    let q = quit.clone();
    ctrlc::set_handler(move || q.store(true, Ordering::Release))
        .map_err(|_| Error::DevicePublishFailed)?;
    let mut session = ptr::null_mut();
    let rc = unsafe {
        wl_create(
            (&mut *adapter as *mut BlockAdapter).cast(),
            read_cb,
            write_cb,
            control_cb,
            adapter.len() / 512,
            u32::from(read_only),
            &mut session,
        )
    };
    if rc != 0 {
        eprintln!("WINSPD_CREATE_ERROR code={rc}");
        return Err(Error::DevicePublishFailed);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut ready = unsafe { wl_volume_ready(session) };
    while ready != 0
        && std::time::Instant::now() < deadline
        && unsafe { wl_error(session) } == 0
        && !adapter.faulted()
        && !quit.load(Ordering::Acquire)
    {
        std::thread::sleep(Duration::from_millis(100));
        ready = unsafe { wl_volume_ready(session) };
    }
    let mut clean = read_only;
    if ready == 0 {
        eprintln!(
            "PUBLISHED_{} (Ctrl+C to close)",
            if read_only { "RO" } else { "RW" }
        );
        while unsafe { wl_error(session) } == 0 && !adapter.faulted() {
            if quit.swap(false, Ordering::AcqRel) {
                if read_only {
                    break;
                }
                let mut phase = 0;
                let rc = unsafe { wl_lock_and_dismount(session, &mut phase) };
                if rc == 0 {
                    clean = true;
                    break;
                }
                if phase == 1 && [5, 32, 33, 170].contains(&rc) {
                    eprintln!("CLOSE_BLOCKED code={rc}; close open files and press Ctrl+C again");
                } else {
                    eprintln!("CLOSE_FAILED phase={phase} code={rc}");
                    adapter.mark_faulted();
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    } else {
        eprintln!("VOLUME_DISCOVERY_ERROR code={ready}");
        if !read_only {
            let mut phase = 0;
            clean = unsafe { wl_lock_and_dismount(session, &mut phase) } == 0;
        }
    }
    let failed = adapter.shutdown(|| unsafe { wl_close(session) });
    eprintln!(
        "CLOSED reads={} write_callbacks={} unmap_callbacks={} flush_callbacks={} clean={}",
        adapter.reads.load(Ordering::Relaxed),
        adapter.write_attempts.load(Ordering::Relaxed),
        adapter.unmap_attempts.load(Ordering::Relaxed),
        adapter.flushes.load(Ordering::Relaxed),
        clean && !failed
    );
    if !read_only && (!clean || failed) {
        Err(Error::UncleanClose)
    } else if ready != 0 {
        Err(Error::VolumeDiscoveryFailed)
    } else if failed {
        Err(Error::DevicePublishFailed)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::fault::Operation;
    #[test]
    fn callback_panics_are_immediately_sticky_and_reads_are_zeroed() {
        for op in [Operation::Read, Operation::Write, Operation::Sync] {
            let (_dir, _path, v) = crate::volume::tests::panic_session(op);
            let mut a = BlockAdapter::new(v);
            let context = (&mut a as *mut BlockAdapter).cast();
            let mut buffer = [0xa5u8; 512];
            let result = unsafe {
                match op {
                    Operation::Read => read_cb(context, 0, 1, buffer.as_mut_ptr().cast()),
                    Operation::Write => write_cb(context, 0, 1, buffer.as_ptr().cast()),
                    Operation::Sync => control_cb(context, 2, 0, 0),
                }
            };
            assert_eq!(result, 5);
            assert!(a.faulted());
            if op == Operation::Read {
                assert_eq!(buffer, [0; 512]);
            }
            assert_eq!(a.write(0, &[0; 512]), Err(Error::WritebackFailed));
        }
    }
}
