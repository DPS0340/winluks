use crate::{
    Error, Result,
    crypto::{Hash, Kdf},
    image::Image,
    strict_json,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub struct Metadata {
    pub(crate) offset: u64,
    pub(crate) length: u64,
    slots: BTreeMap<u32, Slot>,
}
#[derive(Clone)]
pub(crate) struct Slot {
    pub key_size: usize,
    pub area_key_size: usize,
    pub area_offset: u64,
    pub af_hash: Hash,
    pub kdf: Kdf,
    pub digest_kdf: Kdf,
    pub digest: Vec<u8>,
}
type Obj = Map<String, Value>;
fn object(v: &Value) -> Result<&Obj> {
    v.as_object().ok_or(Error::MetadataInvalid)
}
fn get<'a>(o: &'a Obj, k: &str) -> Result<&'a Value> {
    o.get(k).ok_or(Error::MetadataInvalid)
}
fn text<'a>(o: &'a Obj, k: &str) -> Result<&'a str> {
    get(o, k)?.as_str().ok_or(Error::MetadataInvalid)
}
fn num(o: &Obj, k: &str) -> Result<u64> {
    get(o, k)?.as_u64().ok_or(Error::MetadataInvalid)
}
fn decimal(s: &str) -> Result<u64> {
    if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0'))
    {
        return Err(Error::MetadataInvalid);
    }
    s.parse().map_err(|_| Error::MetadataInvalid)
}
fn dec(o: &Obj, k: &str) -> Result<u64> {
    decimal(text(o, k)?)
}
fn only(o: &Obj, allowed: &[&str]) -> Result<()> {
    if o.keys().any(|k| !allowed.contains(&k.as_str())) {
        Err(Error::UnsupportedProfile)
    } else {
        Ok(())
    }
}
fn b64(o: &Obj, k: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(text(o, k)?)
        .map_err(|_| Error::MetadataInvalid)
}
fn id(s: &str, max: u32) -> Result<u32> {
    let n = decimal(s)?;
    if n > max as u64 {
        return Err(Error::MetadataInvalid);
    }
    Ok(n as u32)
}
fn ids(o: &Obj, k: &str, max: u32) -> Result<Vec<u32>> {
    let mut out = Vec::new();
    for x in get(o, k)?.as_array().ok_or(Error::MetadataInvalid)? {
        let n = id(x.as_str().ok_or(Error::MetadataInvalid)?, max)?;
        if out.contains(&n) {
            return Err(Error::MetadataInvalid);
        }
        out.push(n);
    }
    Ok(out)
}
fn bounded(o: &Obj, k: &str, max: u32) -> Result<u32> {
    let n = num(o, k)?;
    if n == 0 || n > max as u64 {
        return Err(Error::ResourceLimit);
    }
    Ok(n as u32)
}
fn parse_kdf(o: &Obj) -> Result<Kdf> {
    let salt = b64(o, "salt")?;
    if salt.len() != 32 {
        return Err(Error::MetadataInvalid);
    }
    match text(o, "type")? {
        "pbkdf2" => {
            only(o, &["type", "hash", "iterations", "salt"])?;
            Ok(Kdf::Pbkdf2 {
                hash: Hash::parse(text(o, "hash")?)?,
                iterations: bounded(o, "iterations", 10_000_000)?,
                salt,
            })
        }
        "argon2i" | "argon2id" => {
            only(o, &["type", "time", "memory", "cpus", "salt"])?;
            let memory = bounded(o, "memory", 1_048_576)?;
            let time = bounded(o, "time", 10)?;
            let cpus = bounded(o, "cpus", 8)?;
            if memory < 8 * cpus {
                return Err(Error::MetadataInvalid);
            }
            Ok(Kdf::Argon2 {
                id: text(o, "type")? == "argon2id",
                memory,
                time,
                cpus,
                salt,
            })
        }
        _ => Err(Error::UnsupportedProfile),
    }
}
fn cstr(b: &[u8]) -> Result<&str> {
    let end = b
        .iter()
        .position(|x| *x == 0)
        .ok_or(Error::MetadataInvalid)?;
    if b[end..].iter().any(|x| *x != 0) {
        return Err(Error::MetadataInvalid);
    }
    std::str::from_utf8(&b[..end]).map_err(|_| Error::MetadataInvalid)
}
fn be64(b: &[u8]) -> u64 {
    u64::from_be_bytes(b.try_into().expect("fixed header field"))
}
struct Header {
    size: u64,
    seq: u64,
    uuid: String,
    label: String,
    subsystem: String,
    json: Value,
}
fn header(image: &Image, offset: u64) -> Result<Header> {
    let mut fixed = [0u8; 4096];
    image
        .read_exact_at(offset, &mut fixed)
        .map_err(|_| Error::MetadataRecoveryRequired)?;
    let magic: &[u8] = if offset == 0 {
        b"LUKS\xba\xbe"
    } else {
        b"SKUL\xba\xbe"
    };
    if &fixed[..6] != magic || fixed[6..8] != [0, 2] {
        return Err(Error::MetadataRecoveryRequired);
    }
    let size = be64(&fixed[8..16]);
    if !(16 * 1024..=4 * 1024 * 1024).contains(&size) || !size.is_power_of_two() {
        return Err(Error::MetadataInvalid);
    }
    if offset != be64(&fixed[256..264]) || (offset != 0 && offset != size) {
        return Err(Error::MetadataRecoveryRequired);
    }
    if fixed[264..448]
        .iter()
        .chain(fixed[512..].iter())
        .any(|b| *b != 0)
    {
        return Err(Error::UnsupportedProfile);
    }
    let alg = Hash::parse(cstr(&fixed[72..104])?)?;
    let uuid = cstr(&fixed[168..208])?.to_string();
    uuid::Uuid::parse_str(&uuid).map_err(|_| Error::MetadataInvalid)?;
    let mut data = vec![0u8; size as usize];
    image
        .read_exact_at(offset, &mut data)
        .map_err(|_| Error::MetadataRecoveryRequired)?;
    let expected = data[448..448 + alg.size()].to_vec();
    if data[448 + alg.size()..512].iter().any(|b| *b != 0) {
        return Err(Error::MetadataRecoveryRequired);
    }
    data[448..512].fill(0);
    if alg.digest(&data)? != expected {
        return Err(Error::MetadataRecoveryRequired);
    }
    let json_area = &data[4096..];
    let end = json_area
        .iter()
        .position(|b| *b == 0)
        .ok_or(Error::MetadataInvalid)?;
    if json_area[end..].iter().any(|b| *b != 0) {
        return Err(Error::MetadataInvalid);
    }
    Ok(Header {
        size,
        seq: be64(&fixed[16..24]),
        uuid,
        label: cstr(&fixed[24..72])?.into(),
        subsystem: cstr(&fixed[208..256])?.into(),
        json: strict_json::parse(&json_area[..end])?,
    })
}
impl Metadata {
    pub fn read(image: &Image) -> Result<Self> {
        let a = header(image, 0)?;
        let b = header(image, a.size)?;
        if a.size != b.size
            || a.seq != b.seq
            || a.uuid != b.uuid
            || a.label != b.label
            || a.subsystem != b.subsystem
            || a.json != b.json
        {
            return Err(Error::MetadataRecoveryRequired);
        }
        Self::from_json(&a.json, a.size, image.len())
    }
    fn from_json(v: &Value, hdr_size: u64, image_len: u64) -> Result<Self> {
        let root = object(v)?;
        only(
            root,
            &["keyslots", "tokens", "segments", "digests", "config"],
        )?;
        let config = object(get(root, "config")?)?;
        only(
            config,
            &["json_size", "keyslots_size", "flags", "requirements"],
        )?;
        if dec(config, "json_size")? != hdr_size - 4096 {
            return Err(Error::MetadataInvalid);
        }
        if let Some(flags) = config.get("flags")
            && !flags.as_array().ok_or(Error::MetadataInvalid)?.is_empty()
        {
            return Err(Error::UnsupportedProfile);
        }
        if let Some(r) = config.get("requirements") {
            let r = object(r)?;
            only(r, &["mandatory"])?;
            if let Some(m) = r.get("mandatory") {
                let mandatory = m.as_array().ok_or(Error::MetadataInvalid)?;
                for x in mandatory {
                    let s = x.as_str().ok_or(Error::MetadataInvalid)?;
                    if s.contains("reencrypt") {
                        return Err(Error::ReencryptionUnsupported);
                    }
                }
                if !mandatory.is_empty() {
                    return Err(Error::UnsupportedProfile);
                }
            }
        }
        let keyslots_size = dec(config, "keyslots_size")?;
        if keyslots_size > 128 * 1024 * 1024 || !keyslots_size.is_multiple_of(4096) {
            return Err(Error::ResourceLimit);
        }
        let area_start = hdr_size.checked_mul(2).ok_or(Error::MetadataInvalid)?;
        let area_end = area_start
            .checked_add(keyslots_size)
            .ok_or(Error::MetadataInvalid)?;
        let segs = object(get(root, "segments")?)?;
        for seg in segs.values() {
            let o = object(seg)?;
            if let Some(f) = o.get("flags") {
                for flag in f.as_array().ok_or(Error::MetadataInvalid)? {
                    if flag
                        .as_str()
                        .ok_or(Error::MetadataInvalid)?
                        .contains("reencrypt")
                    {
                        return Err(Error::ReencryptionUnsupported);
                    }
                }
            }
        }
        if segs.len() != 1 {
            return Err(Error::UnsupportedProfile);
        }
        let (segid, seg) = segs.iter().next().ok_or(Error::MetadataInvalid)?;
        let segid = id(segid, 31)?;
        let seg = object(seg)?;
        only(
            seg,
            &[
                "type",
                "offset",
                "size",
                "iv_tweak",
                "encryption",
                "sector_size",
                "flags",
            ],
        )?;
        if text(seg, "type")? != "crypt"
            || text(seg, "encryption")? != "aes-xts-plain64"
            || num(seg, "sector_size")? != 512
            || dec(seg, "iv_tweak")? != 0
        {
            return Err(Error::UnsupportedProfile);
        }
        if let Some(f) = seg.get("flags")
            && !f.as_array().ok_or(Error::MetadataInvalid)?.is_empty()
        {
            return Err(Error::UnsupportedProfile);
        }
        let offset = dec(seg, "offset")?;
        if offset < area_end || offset > image_len || !offset.is_multiple_of(512) {
            return Err(Error::MetadataInvalid);
        }
        let length = match text(seg, "size")? {
            "dynamic" => image_len - offset,
            s => decimal(s)?,
        };
        if length == 0 || length > image_len - offset || !length.is_multiple_of(512) {
            return Err(Error::MetadataInvalid);
        }
        let keys = object(get(root, "keyslots")?)?;
        if keys.len() > 32 {
            return Err(Error::ResourceLimit);
        }
        let mut parsed = BTreeMap::new();
        let mut ranges = Vec::new();
        for (k, v) in keys {
            let key_id = id(k, 31)?;
            let o = object(v)?;
            if text(o, "type")? == "reencrypt" {
                return Err(Error::ReencryptionUnsupported);
            }
            only(o, &["type", "key_size", "area", "kdf", "af", "priority"])?;
            if text(o, "type")? != "luks2" {
                return Err(Error::UnsupportedProfile);
            }
            if let Some(p) = o.get("priority")
                && p.as_u64().ok_or(Error::MetadataInvalid)? > 2
            {
                return Err(Error::MetadataInvalid);
            }
            let key_size = num(o, "key_size")?;
            let area = object(get(o, "area")?)?;
            only(area, &["type", "offset", "size", "encryption", "key_size"])?;
            let area_key_size = num(area, "key_size")?;
            if ![32, 64].contains(&key_size)
                || ![32, 64].contains(&area_key_size)
                || text(area, "type")? != "raw"
                || text(area, "encryption")? != "aes-xts-plain64"
            {
                return Err(Error::UnsupportedProfile);
            }
            let start = dec(area, "offset")?;
            let size = dec(area, "size")?;
            let end = start.checked_add(size).ok_or(Error::MetadataInvalid)?;
            if size > 64 * 1024 * 1024 {
                return Err(Error::ResourceLimit);
            }
            if size < key_size * 4000
                || start < area_start
                || end > area_end
                || !start.is_multiple_of(4096)
                || !size.is_multiple_of(4096)
            {
                return Err(Error::MetadataInvalid);
            }
            if ranges.iter().any(|&(s, e)| start < e && s < end) {
                return Err(Error::MetadataInvalid);
            }
            ranges.push((start, end));
            let af = object(get(o, "af")?)?;
            only(af, &["type", "stripes", "hash"])?;
            if text(af, "type")? != "luks1" || num(af, "stripes")? != 4000 {
                return Err(Error::UnsupportedProfile);
            }
            let kdf = parse_kdf(object(get(o, "kdf")?)?)?;
            parsed.insert(
                key_id,
                (
                    key_size as usize,
                    area_key_size as usize,
                    start,
                    Hash::parse(text(af, "hash")?)?,
                    kdf,
                ),
            );
        }
        let tokens = object(get(root, "tokens")?)?;
        if tokens.len() > 32 {
            return Err(Error::ResourceLimit);
        }
        for (t, v) in tokens {
            id(t, 31)?;
            object(v)?;
        }
        let digests = object(get(root, "digests")?)?;
        if digests.len() > 32 {
            return Err(Error::ResourceLimit);
        }
        let mut slots = BTreeMap::new();
        for (d, v) in digests {
            id(d, 31)?;
            let o = object(v)?;
            only(
                o,
                &[
                    "type",
                    "keyslots",
                    "segments",
                    "hash",
                    "iterations",
                    "salt",
                    "digest",
                ],
            )?;
            if text(o, "type")? != "pbkdf2" {
                return Err(Error::UnsupportedProfile);
            }
            let digest = b64(o, "digest")?;
            let salt = b64(o, "salt")?;
            let hash = Hash::parse(text(o, "hash")?)?;
            if salt.len() != 32 || (digest.len() != 20 && digest.len() != hash.size()) {
                return Err(Error::MetadataInvalid);
            }
            let iterations = bounded(o, "iterations", 10_000_000)?;
            let ds = ids(o, "segments", 31)?;
            if ds.iter().any(|s| *s != segid) {
                return Err(Error::MetadataInvalid);
            }
            for k in ids(o, "keyslots", 31)? {
                let (key_size, area_key_size, area_offset, af_hash, kdf) =
                    parsed.get(&k).ok_or(Error::MetadataInvalid)?;
                if ds.is_empty() {
                    continue;
                }
                let slot = Slot {
                    key_size: *key_size,
                    area_key_size: *area_key_size,
                    area_offset: *area_offset,
                    af_hash: *af_hash,
                    kdf: kdf.clone(),
                    digest_kdf: Kdf::Pbkdf2 {
                        hash,
                        iterations,
                        salt: salt.clone(),
                    },
                    digest: digest.clone(),
                };
                if slots.insert(k, slot).is_some() {
                    return Err(Error::MetadataInvalid);
                }
            }
        }
        Ok(Self {
            offset,
            length,
            slots,
        })
    }
    pub fn keyslots(&self) -> Vec<u32> {
        self.slots.keys().copied().collect()
    }
    pub fn volume_length(&self) -> u64 {
        self.length
    }
    pub(crate) fn slot(&self, id: u32) -> Result<&Slot> {
        self.slots.get(&id).ok_or(Error::UnsupportedProfile)
    }
}
