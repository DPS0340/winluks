use std::fs;
use winluks::{Error, image::Image};
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
