use crate::{Error, Result};
#[cfg(test)]
pub(crate) mod fault;
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccessMode {
    #[default]
    ReadOnly,
    ReadWrite,
}

/// One fixed-size local file handle. RW sessions deny sharing; no create or resize.
pub struct Image {
    file: File,
    len: u64,
    mode: AccessMode,
    #[cfg(test)]
    pub(crate) fault: fault::Injector,
}
impl Image {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_mode(path, AccessMode::ReadOnly)
    }
    pub fn open_with_mode(path: &Path, mode: AccessMode) -> Result<Self> {
        let mut o = OpenOptions::new();
        o.read(true).write(mode == AccessMode::ReadWrite);
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
            o.share_mode(if mode == AccessMode::ReadOnly {
                FILE_SHARE_READ
            } else {
                0
            })
            .custom_flags(
                FILE_FLAG_OPEN_REPARSE_POINT
                    | if mode == AccessMode::ReadWrite {
                        FILE_FLAG_WRITE_THROUGH
                    } else {
                        0
                    },
            );
        }
        let file = o.open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(Error::UnsupportedProfile);
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // Advisory on Unix; uncooperative external writers remain outside this backend.
            let lock = if mode == AccessMode::ReadOnly {
                libc::LOCK_SH
            } else {
                libc::LOCK_EX
            };
            if unsafe { libc::flock(file.as_raw_fd(), lock | libc::LOCK_NB) } != 0 {
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
            mode,
            #[cfg(test)]
            fault: fault::Injector::default(),
        })
    }
    pub fn len(&self) -> u64 {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn mode(&self) -> AccessMode {
        self.mode
    }
    pub(crate) fn write_all_at(&self, offset: u64, buf: &[u8]) -> Result<()> {
        if self.mode != AccessMode::ReadWrite {
            return Err(Error::ReadOnly);
        }
        if offset > self.len || buf.len() as u64 > self.len - offset {
            return Err(Error::InvalidRange);
        }
        if self.file.metadata()?.len() != self.len {
            return Err(Error::BackendIo);
        }
        let mut done = 0;
        while done < buf.len() {
            let n = match self.write_once(offset + done as u64, &buf[done..]) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if n == 0 {
                return Err(Error::BackendIo);
            }
            done += n;
        }
        Ok(())
    }
    pub(crate) fn sync(&self) -> Result<()> {
        if self.file.metadata()?.len() != self.len {
            return Err(Error::BackendIo);
        }
        if self.mode == AccessMode::ReadWrite {
            #[cfg(test)]
            self.fault.before(fault::Operation::Sync, 0)?;
            self.file.sync_all()?;
        }
        Ok(())
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
            let n = match self.read_once(offset + done as u64, &mut buf[done..]) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if n == 0 {
                return Err(Error::BackendIo);
            }
            done += n;
        }
        Ok(())
    }
    fn write_once(&self, offset: u64, buf: &[u8]) -> std::io::Result<usize> {
        #[cfg(test)]
        let buf = &buf[..self.fault.before(fault::Operation::Write, buf.len())?];
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            self.file.write_at(buf, offset)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileExt;
            self.file.seek_write(buf, offset)
        }
    }
    fn read_once(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        #[cfg(test)]
        let buf = {
            let n = self.fault.before(fault::Operation::Read, buf.len())?;
            &mut buf[..n]
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            self.file.read_at(buf, offset)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileExt;
            self.file.seek_read(buf, offset)
        }
    }
}
