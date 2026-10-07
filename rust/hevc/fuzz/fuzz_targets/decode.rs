//! An Annex B stream, as the decoder gets one: access unit by access unit,
//! or whole where there is no picture to split it at, to two decoders, one
//! on the calling thread and one on the pool. Every outcome is allowed but a
//! panic, and a unit the two do not decode alike.

#![no_main]

use std::sync::Once;

use libfuzzer_sys::fuzz_target;

/// The pool's threads: enough for rows to wait on one another.
const THREADS: usize = 3;

fn pool() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| rayon::ThreadPoolBuilder::new().num_threads(THREADS).build_global().expect("the pool"));
}

/// What a unit decoded to, as far as the two decoders must agree: an error,
/// no picture, or the picture's samples.
fn outcome(dec: &mut hevc::Decoder, unit: &[u8]) -> Option<Option<Vec<u8>>> {
    match dec.decode(unit) {
        Err(_) => None,
        Ok(None) => Some(None),
        Ok(Some(d)) => Some(Some(samples(&d))),
    }
}

fn samples(d: &hevc::Decoded) -> Vec<u8> {
    let mut v = Vec::new();
    for p in &d.picture.planes {
        for y in 0..p.height {
            v.extend_from_slice(&p.data[y * p.stride..y * p.stride + p.width]);
        }
    }
    v
}

fuzz_target!(|data: &[u8]| {
    pool();
    let mut one = hevc::Decoder::new(1);
    let mut many = hevc::Decoder::new(THREADS);
    let units = hevc::access_units(data);
    let units = if units.is_empty() { vec![data] } else { units };
    for unit in units {
        assert!(outcome(&mut one, unit) == outcome(&mut many, unit), "one thread and {THREADS} decoded a unit differently");
    }
});
