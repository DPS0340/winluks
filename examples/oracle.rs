//! Test harness only: accepts a locally generated synthetic fixture manifest.
use openssl::hash::{Hasher, MessageDigest};
use std::{fs, path::PathBuf};
use winluks::{
    Error, adapter::BlockAdapter, image::Image, metadata::Metadata, probe::Filesystem,
    volume::UnlockedVolume,
};
use zeroize::Zeroizing;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("fixture manifest required")?,
    );
    let m: serde_json::Value = serde_json::from_slice(&fs::read(&manifest)?)?;
    let dir = manifest.parent().ok_or("manifest directory")?;
    let image_path = dir.join(m["image"].as_str().ok_or("image field")?);
    let key = Zeroizing::new(fs::read(
        dir.join(m["password_file"].as_str().ok_or("password_file field")?),
    )?);
    let im = Image::open(&image_path)?;
    let slot = u32::try_from(m["keyslot"].as_u64().ok_or("keyslot field")?)?;
    let meta = Metadata::read(&im)?;
    assert_eq!(meta.volume_length(), m["plaintext_bytes"].as_u64().unwrap());
    assert!(matches!(
        UnlockedVolume::unlock(Image::open(&image_path)?, slot, b"intentionally-wrong"),
        Err(Error::UnlockFailed)
    ));
    if m["different_slot_password"].as_bool() == Some(true) && meta.keyslots().contains(&0) {
        assert!(matches!(
            UnlockedVolume::unlock(Image::open(&image_path)?, 0, &key),
            Err(Error::UnlockFailed)
        ));
    }
    let v = UnlockedVolume::unlock(im, slot, &key)?;
    let mut h = Hasher::new(MessageDigest::sha256())?;
    let mut o = 0;
    while o < v.len() {
        let n = (v.len() - o).min(1024 * 1024) as usize;
        h.update(&v.read_at(o, n)?)?;
        o += n as u64;
    }
    let got = h
        .finish()?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    assert_eq!(got, m["plaintext_sha256"].as_str().unwrap());
    assert!(matches!(v.read_at(u64::MAX, 512), Err(Error::InvalidRange)));
    assert!(matches!(v.read_at(v.len(), 512), Err(Error::InvalidRange)));
    assert!(v.read_at(v.len(), 0)?.is_empty());
    // Independent boundary and non-aligned byte comparisons against cryptsetup output.
    if dir.join(m["plaintext"].as_str().unwrap()).exists() {
        let oracle = Image::open(&dir.join(m["plaintext"].as_str().unwrap()))?;
        for (o, n) in [
            (0, 512),
            (511, 1025),
            (1024, 1024),
            (65536, 4096),
            (v.len() - 512, 512),
        ] {
            let mut expected = Zeroizing::new(vec![0; n]);
            oracle.read_exact_at(o, &mut expected)?;
            assert!(v.read_at(o, n)?.as_slice() == expected.as_slice());
        }
    }
    let fs = match m["filesystem"].as_str().unwrap() {
        "ext4" => Filesystem::Ext4,
        "btrfs" => Filesystem::Btrfs,
        _ => return Err("filesystem".into()),
    };
    let a = BlockAdapter::new(v.validate(fs)?);
    assert_eq!(a.write(0, &[0u8; 512]), Err(Error::ReadOnly));
    assert_eq!(a.unmap(), Err(Error::ReadOnly));
    assert!(a.flush(0, 0).is_ok());
    a.stop();
    assert!(matches!(a.read(0, 1), Err(Error::Stopping)));
    let mut file = fs::File::open(&image_path)?;
    let mut h = Hasher::new(MessageDigest::sha256())?;
    let mut b = vec![0u8; 1024 * 1024];
    loop {
        use std::io::Read;
        let n = file.read(&mut b)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n])?;
    }
    let got = h
        .finish()?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    assert_eq!(got, m["image_sha256"].as_str().unwrap());
    println!(
        "PASS {}: unlock, whole-volume hash, boundaries, filesystem probe, RO callbacks, image hash",
        m["name"].as_str().unwrap()
    );
    Ok(())
}
