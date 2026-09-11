use std::fs;
use winluks::{
    Error,
    image::{AccessMode, Image},
};

#[test]
fn rw_sessions_are_exclusive_and_do_not_create_or_truncate() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("exclusive.img");
    assert!(Image::open_with_mode(&p, AccessMode::ReadWrite).is_err());
    assert!(!p.exists());
    fs::write(&p, [0x5a; 1024]).unwrap();
    let ro = Image::open(&p).unwrap();
    assert!(Image::open_with_mode(&p, AccessMode::ReadWrite).is_err());
    drop(ro);
    let rw = Image::open_with_mode(&p, AccessMode::ReadWrite).unwrap();
    assert_eq!(rw.len(), 1024);
    assert!(Image::open(&p).is_err());
    assert!(Image::open_with_mode(&p, AccessMode::ReadWrite).is_err());
    #[cfg(windows)]
    {
        assert!(fs::File::open(&p).is_err());
        assert!(fs::OpenOptions::new().write(true).open(&p).is_err());
        assert!(fs::remove_file(&p).is_err());
    }
    drop(rw);
    assert_eq!(fs::read(&p).unwrap(), [0x5a; 1024]);
}
#[test]
fn file_boundaries() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("file.img");
    fs::write(&p, [1, 2, 3, 4]).unwrap();
    let i = Image::open(&p).unwrap();
    let mut b = [0u8; 2];
    i.read_exact_at(2, &mut b).unwrap();
    assert_eq!(b, [3, 4]);
    assert_eq!(i.read_exact_at(u64::MAX, &mut b), Err(Error::InvalidRange));
    assert_eq!(i.read_exact_at(3, &mut b), Err(Error::InvalidRange));
    assert!(i.read_exact_at(4, &mut []).is_ok());
}
#[test]
fn directory_rejected() {
    let d = tempfile::tempdir().unwrap();
    assert!(Image::open(d.path()).is_err());
}
#[cfg(unix)]
#[test]
fn special_inputs_rejected() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("link");
    std::os::unix::fs::symlink("/dev/null", &p).unwrap();
    assert!(Image::open(&p).is_err());
    assert!(Image::open(std::path::Path::new("/dev/null")).is_err());
}
#[cfg(unix)]
#[test]
fn changing_size_is_an_io_error() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("file.img");
    fs::write(&p, [0u8; 512]).unwrap();
    let i = Image::open(&p).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&p)
        .unwrap()
        .set_len(1)
        .unwrap();
    assert_eq!(i.read_exact_at(0, &mut [0u8; 512]), Err(Error::BackendIo));
}
#[cfg(windows)]
#[test]
fn windows_session_prevents_write_and_delete_handles() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("immutable.img");
    fs::write(&p, [0u8; 512]).unwrap();
    let image = Image::open(&p).unwrap();
    assert!(fs::OpenOptions::new().write(true).open(&p).is_err());
    assert!(fs::remove_file(&p).is_err());
    drop(image);
    fs::remove_file(&p).unwrap();
}
#[cfg(windows)]
#[test]
fn windows_device_remote_and_stream_paths_are_rejected() {
    for path in [
        r"\\.\PhysicalDrive0",
        r"\\server\share\disk.img",
        r"C:\disk.img:stream",
        r"C:relative.img",
    ] {
        assert!(matches!(
            Image::open(std::path::Path::new(path)),
            Err(Error::UnsupportedProfile)
        ));
    }
}
