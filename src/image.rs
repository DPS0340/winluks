use crate::{Error, Result};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

/// No write, resize, discard or path-reopen operation is exposed.
pub struct Image {
    file: File,
    len: u64,
}
impl Image {
    pub fn open(path: &Path) -> Result<Self> {
        let mut o = OpenOptions::new();
        o.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::*;
            let s = path.to_str().ok_or(Error::UnsupportedProfile)?;
            // Accept ordinary absolute drive paths only, including neither ADS nor DOS devices.
            let b = s.as_bytes();
            if b.len() < 3
                || !b[0].is_ascii_alphabetic()
                || b[1] != b':'
                || b[2] != b'\\'
                || s[2..].contains(':')
            {
                return Err(Error::UnsupportedProfile);
            }
            o.share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        }
        let file = o.open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(Error::UnsupportedProfile);
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // Advisory on Unix; fixtures must be immutable and have no concurrent writer.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } != 0 {
                return Err(Error::BackendIo);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
            use windows_sys::Win32::Storage::FileSystem::*;
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(Error::UnsupportedProfile);
            }
            let mut remote = std::mem::MaybeUninit::<FILE_REMOTE_PROTOCOL_INFO>::zeroed();
            let h = file.as_raw_handle();
            if unsafe { GetFileType(h) } != FILE_TYPE_DISK {
                return Err(Error::UnsupportedProfile);
            }
            if unsafe {
                GetFileInformationByHandleEx(
                    h,
                    FileRemoteProtocolInfo,
                    remote.as_mut_ptr().cast(),
                    std::mem::size_of::<FILE_REMOTE_PROTOCOL_INFO>() as u32,
                )
            } != 0
            {
                return Err(Error::UnsupportedProfile);
            }
            // Verify the opened handle resolves to a local drive; never reopen the path.
            let mut name = vec![0u16; 32768];
            let n = unsafe {
                GetFinalPathNameByHandleW(
                    h,
                    name.as_mut_ptr(),
                    name.len() as u32,
                    FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
                )
            } as usize;
            if n == 0 || n >= name.len() {
                return Err(Error::BackendIo);
            }
            let s = String::from_utf16(&name[..n]).map_err(|_| Error::UnsupportedProfile)?;
            let b = s.as_bytes();
            if !s.starts_with("\\\\?\\")
                || b.len() < 7
                || !b[4].is_ascii_alphabetic()
                || b[5] != b':'
                || b[6] != b'\\'
            {
                return Err(Error::UnsupportedProfile);
            }
            let root = [name[4], b':' as u16, b'\\' as u16, 0];
            if unsafe { GetDriveTypeW(root.as_ptr()) }
                != windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED
            {
                return Err(Error::UnsupportedProfile);
            }
        }
        Ok(Self {
            file,
            len: meta.len(),
        })
    }
    pub fn len(&self) -> u64 {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        if offset > self.len || buf.len() as u64 > self.len - offset {
            return Err(Error::InvalidRange);
        }
        if self.file.metadata()?.len() != self.len {
            return Err(Error::BackendIo);
        }
        let mut done = 0;
        while done < buf.len() {
            #[cfg(unix)]
            let n = {
                use std::os::unix::fs::FileExt;
                self.file.read_at(&mut buf[done..], offset + done as u64)?
            };
            #[cfg(windows)]
            let n = {
                use std::os::windows::fs::FileExt;
                self.file
                    .seek_read(&mut buf[done..], offset + done as u64)?
            };
            if n == 0 {
                return Err(Error::BackendIo);
            }
            done += n;
        }
        Ok(())
    }
}
