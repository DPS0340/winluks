use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::{Seek, SeekFrom, Write},
    path::PathBuf,
};
use tempfile::TempDir;
use winluks::{Error, image::Image, metadata::Metadata};
const H: usize = 16384;
fn model() -> Value {
    json!({
     "keyslots":{"0":{"type":"luks2","key_size":32,"area":{"type":"raw","offset":"32768","size":"131072","encryption":"aes-xts-plain64","key_size":64},"kdf":{"type":"pbkdf2","hash":"sha256","iterations":1000,"salt":STANDARD.encode([0u8;32])},"af":{"type":"luks1","hash":"sha512","stripes":4000}}},
     "tokens":{},"segments":{"0":{"type":"crypt","offset":"2097152","size":"dynamic","iv_tweak":"0","encryption":"aes-xts-plain64","sector_size":512}},
     "digests":{"0":{"type":"pbkdf2","keyslots":["0"],"segments":["0"],"hash":"sha256","iterations":1000,"salt":STANDARD.encode([0u8;32]),"digest":STANDARD.encode([1u8;32])}},
     "config":{"json_size":"12288","keyslots_size":"1048576"}
    })
}
fn header(text: &str, secondary: bool, seq: u64) -> Vec<u8> {
    let mut h = vec![0u8; H];
    h[..6].copy_from_slice(if secondary {
        b"SKUL\xba\xbe"
    } else {
        b"LUKS\xba\xbe"
    });
    h[6..8].copy_from_slice(&2u16.to_be_bytes());
    h[8..16].copy_from_slice(&(H as u64).to_be_bytes());
    h[16..24].copy_from_slice(&seq.to_be_bytes());
    h[72..78].copy_from_slice(b"sha256");
    h[168..204].copy_from_slice(b"00000000-0000-4000-8000-000000000001");
    h[256..264].copy_from_slice(&(if secondary { H as u64 } else { 0 }).to_be_bytes());
    h[4096..4096 + text.len()].copy_from_slice(text.as_bytes());
    let sum = openssl::sha::sha256(&h);
    h[448..480].copy_from_slice(&sum);
    h
}
fn image(a: Vec<u8>, b: Vec<u8>) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("fixture.img");
    let mut f = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&p)
        .unwrap();
    f.set_len(3 * 1024 * 1024).unwrap();
    f.write_all(&a).unwrap();
    f.seek(SeekFrom::Start(H as u64)).unwrap();
    f.write_all(&b).unwrap();
    drop(f);
    (dir, p)
}
fn check(v: Value) -> Result<Metadata, Error> {
    let text = v.to_string();
    let (_d, p) = image(header(&text, false, 1), header(&text, true, 1));
    Metadata::read(&Image::open(&p)?)
}
#[test]
fn valid_different_key_lengths() {
    let m = check(model()).unwrap();
    assert_eq!(m.keyslots(), vec![0]);
    assert_eq!(m.volume_length(), 1024 * 1024);
}
#[test]
fn checksum_and_seqid() {
    let t = model().to_string();
    let mut a = header(&t, false, 1);
    a[5000] ^= 1;
    let (_d, p) = image(a, header(&t, true, 1));
    assert!(matches!(
        Metadata::read(&Image::open(&p).unwrap()),
        Err(Error::MetadataRecoveryRequired)
    ));
    let (_d, p) = image(header(&t, false, 1), header(&t, true, 2));
    assert!(matches!(
        Metadata::read(&Image::open(&p).unwrap()),
        Err(Error::MetadataRecoveryRequired)
    ));
}
#[test]
fn semantic_duplicate_json() {
    let t = model().to_string().replacen("{", "{\"tokens\":{},", 1);
    let (_d, p) = image(header(&t, false, 1), header(&t, true, 1));
    assert!(matches!(
        Metadata::read(&Image::open(&p).unwrap()),
        Err(Error::MetadataInvalid)
    ));
}
#[test]
fn stripes_policy() {
    for stripes in [0, 3999, 4001, u64::MAX] {
        let mut v = model();
        v["keyslots"]["0"]["af"]["stripes"] = json!(stripes);
        assert!(matches!(check(v), Err(Error::UnsupportedProfile)));
    }
}
#[test]
fn data_profile() {
    for (k, v) in [
        ("sector_size", json!(4096)),
        ("iv_tweak", json!("1")),
        ("encryption", json!("aes-cbc-plain")),
    ] {
        let mut m = model();
        m["segments"]["0"][k] = v;
        assert!(matches!(check(m), Err(Error::UnsupportedProfile)));
    }
}
#[test]
fn unknown_requirements_and_reencryption() {
    let mut v = model();
    v["config"]["requirements"] = json!({"mandatory":["online-reencrypt-v2"]});
    assert!(matches!(check(v), Err(Error::ReencryptionUnsupported)));
    let mut v = model();
    v["config"]["requirements"] = json!({"mandatory":["unknown"]});
    assert!(matches!(check(v), Err(Error::UnsupportedProfile)));
    let mut v = model();
    v["keyslots"]["0"]["type"] = json!("reencrypt");
    assert!(matches!(check(v), Err(Error::ReencryptionUnsupported)));
}
#[test]
fn area_overlap_and_out_of_bounds() {
    let mut v = model();
    v["keyslots"]["1"] = v["keyslots"]["0"].clone();
    assert!(matches!(check(v), Err(Error::MetadataInvalid)));
    for val in ["0", "18446744073709551615", "18446744073709551616"] {
        let mut v = model();
        v["keyslots"]["0"]["area"]["offset"] = json!(val);
        assert!(matches!(check(v), Err(Error::MetadataInvalid)));
    }
}
#[test]
fn kdf_resource_limits() {
    let mut v = model();
    v["keyslots"]["0"]["kdf"]["iterations"] = json!(10_000_001);
    assert!(matches!(check(v), Err(Error::ResourceLimit)));
    let mut v = model();
    v["keyslots"]["0"]["kdf"] = json!({"type":"argon2id","memory":1048577,"time":4,"cpus":1,"salt":STANDARD.encode([0u8;32])});
    assert!(matches!(check(v), Err(Error::ResourceLimit)));
}
#[test]
fn unbound_digest_does_not_offer_unlock_slot() {
    let mut v = model();
    v["digests"]["0"]["segments"] = json!([]);
    assert!(check(v).unwrap().keyslots().is_empty());
}
#[test]
fn missing_and_duplicate_bindings() {
    let mut v = model();
    v["digests"]["0"]["keyslots"] = json!(["1"]);
    assert!(matches!(check(v), Err(Error::MetadataInvalid)));
    let mut v = model();
    v["digests"]["1"] = v["digests"]["0"].clone();
    assert!(matches!(check(v), Err(Error::MetadataInvalid)));
}
#[test]
fn malformed_types() {
    let mut v = model();
    v["segments"]["0"]["offset"] = json!(2097152);
    assert!(matches!(check(v), Err(Error::MetadataInvalid)));
    let mut v = model();
    v["config"]["json_size"] = json!("012288");
    assert!(matches!(check(v), Err(Error::MetadataInvalid)));
}
