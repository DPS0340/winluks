#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    winluks::metadata::Metadata::fuzz_json(data);
});
