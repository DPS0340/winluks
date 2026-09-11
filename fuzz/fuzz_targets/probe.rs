#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let _ = winluks::probe::ext4(data, 1 << 40);
    let _ = winluks::probe::btrfs(data, 1 << 40);
    // Repair checksums as well, so the fuzzer exercises policy and geometry.
    if data.len() == 1024 {
        let mut b = data.to_vec();
        let c = !crc32c::crc32c(&b[..1020]);
        b[1020..].copy_from_slice(&c.to_le_bytes());
        let _ = winluks::probe::ext4(&b, 1 << 40);
    }
    if data.len() == 4096 {
        let mut b = data.to_vec();
        let c = crc32c::crc32c(&b[32..]);
        b[..4].copy_from_slice(&c.to_le_bytes());
        let _ = winluks::probe::btrfs(&b, 1 << 40);
    }
});
