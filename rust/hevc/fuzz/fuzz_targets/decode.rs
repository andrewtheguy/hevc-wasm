//! An Annex B stream, as the decoder gets one: access unit by access unit,
//! or whole where there is no picture to split it at. Every outcome is
//! allowed but a panic.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut dec = hevc::Decoder::new(1);
    let units = hevc::access_units(data);
    if units.is_empty() {
        let _ = dec.decode(data);
        return;
    }
    for unit in units {
        let _ = dec.decode(unit);
    }
});
