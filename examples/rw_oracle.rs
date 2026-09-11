//! Disposable block-write oracle. Never modifies the supplied canonical fixture.
use openssl::hash::{Hasher, MessageDigest};
use std::{fs, path::PathBuf};
use winluks::{
    Error,
    adapter::BlockAdapter,
    image::{AccessMode, Image},
    metadata::Metadata,
    probe::Filesystem,
    volume::UnlockedVolume,
};
use zeroize::Zeroizing;

fn prefix(image: &Image, len: u64) -> Result<String, Box<dyn std::error::Error>> {
    let mut hash = Hasher::new(MessageDigest::sha256())?;
    let mut offset = 0;
    while offset < len {
        let mut buffer = vec![0; (len - offset).min(1024 * 1024) as usize];
        image.read_exact_at(offset, &mut buffer)?;
        hash.update(&buffer)?;
        offset += buffer.len() as u64;
    }
    Ok(hash.finish()?.iter().map(|b| format!("{b:02x}")).collect())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let manifest = PathBuf::from(args.next().ok_or("fixture manifest required")?);
    let output = PathBuf::from(args.next().ok_or("new output directory required")?);
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let m: serde_json::Value = serde_json::from_slice(&fs::read(&manifest)?)?;
    let dir = manifest.parent().ok_or("manifest directory")?;
    fs::create_dir(&output)?; // Refuse to overwrite an earlier experiment.
    let path = output.join("volume.img");
    fs::copy(dir.join(m["image"].as_str().ok_or("image")?), &path)?;
    let key = Zeroizing::new(fs::read(
        dir.join(m["password_file"].as_str().ok_or("password file")?),
    )?);
    let image = Image::open_with_mode(&path, AccessMode::ReadWrite)?;
    let meta = Metadata::read(&image)?;
    let offset = meta.data_offset();
    let before = prefix(&image, offset)?;
    let original_size = image.len();
    let fs = match m["filesystem"].as_str() {
        Some("btrfs") => Filesystem::Btrfs,
        Some("ext4") => Filesystem::Ext4,
        _ => return Err("filesystem".into()),
    };
    let volume =
        UnlockedVolume::unlock(image, m["keyslot"].as_u64().ok_or("keyslot")? as u32, &key)?
            .validate(fs)?;
    let a = BlockAdapter::new(volume);
    let changes = [
        (0, 512),
        (512, 1024),
        (65024, 1536),
        (131072, 1024 * 1024),
        (131584, 512),
        (a.len() - 512, 512),
    ];
    let mut report = Vec::new();
    for (index, (offset, length)) in changes.iter().enumerate() {
        let seed = (index * 29 + 0x53) as u8;
        let bytes = Zeroizing::new(
            (0..*length)
                .map(|i| (i as u8).wrapping_mul(17).wrapping_add(seed))
                .collect::<Vec<_>>(),
        );
        a.write(offset / 512, &bytes)?;
        assert_eq!(
            a.read(offset / 512, (*length / 512) as u32)?.as_slice(),
            bytes.as_slice()
        );
        report.push(serde_json::json!({"offset":offset,"length":length,"seed":seed}));
    }
    assert_eq!(a.write(u64::MAX, &[0; 512]), Err(Error::InvalidRange));
    assert_eq!(a.write(a.len() / 512, &[0; 512]), Err(Error::InvalidRange));
    assert_eq!(a.unmap(), Err(Error::UnsupportedOperation));
    a.flush(0, 0)?;
    a.stop();
    assert_eq!(a.write(0, &[0; 512]), Err(Error::Stopping));
    drop(a);
    let after = Image::open(&path)?;
    assert_eq!(after.len(), original_size);
    assert_eq!(prefix(&after, offset)?, before);
    fs::write(
        output.join("writes.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "fixture":m["name"], "plaintext_bytes":m["plaintext_bytes"], "changes":report,
            "pattern":"byte[i] = (17*i + seed) mod 256", "header_sha256":before,
            "header_bytes":offset, "source_image_sha256":m["image_sha256"],
            "source_plaintext_sha256":m["plaintext_sha256"]
        }))?,
    )?;
    println!(
        "PASS {}: RW ranges, overlap, flush, RO metadata boundary, exclusive file",
        m["name"].as_str().unwrap()
    );
    Ok(())
}
