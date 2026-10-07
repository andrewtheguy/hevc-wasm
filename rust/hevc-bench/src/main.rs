//! Decode an Annex B file, one access unit at a time, and print per picture
//! the MD5 `ffmpeg -f framemd5` prints for the cropped planar output, so a run
//! can be checked against FFmpeg; the timing goes to stderr.
//!
//!   hevc-bench FILE [THREADS] [REPEATS]

mod md5;

use std::time::Instant;

/// Splits an Annex B stream into access units: a unit starts at each VCL NAL
/// unit with `first_slice_segment_in_pic_flag`, with the parameter sets before
/// it.
fn access_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    let mut pending_start: Option<usize> = None;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            let nal_type = (data[i + 3] >> 1) & 0x3f;
            let start = if i > 0 && data[i - 1] == 0 { i - 1 } else { i };
            if nal_type < 32 {
                let first = i + 5 < data.len() && data[i + 5] & 0x80 != 0;
                if first {
                    starts.push(pending_start.take().unwrap_or(start));
                }
            } else if pending_start.is_none() {
                pending_start = Some(start);
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut units = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let e = starts.get(k + 1).copied().unwrap_or(data.len());
        units.push(&data[s..e]);
    }
    units
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: hevc-bench FILE [THREADS] [REPEATS]");
    let threads: usize = args.next().map_or(1, |s| s.parse().expect("THREADS"));
    let repeats: usize = args.next().map_or(1, |s| s.parse().expect("REPEATS"));
    let data = std::fs::read(&path).expect("read input");
    let units = access_units(&data);
    if threads > 1 {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().expect("pool");
    }
    for r in 0..repeats {
        let mut dec = hevc::Decoder::new(threads);
        let mut n = 0;
        let mut times = Vec::with_capacity(units.len());
        let t = Instant::now();
        for unit in &units {
            let t0 = Instant::now();
            let decoded = dec.decode(unit);
            times.push(t0.elapsed().as_secs_f64() * 1000.0);
            match decoded {
                Ok(Some(d)) => {
                    n += 1;
                    if r == 0 {
                        let [x, y, w, h] = d.window.map(|v| v as usize);
                        let mut m = md5::Md5::new();
                        let mut raw = Vec::new();
                        for plane in &d.picture.planes {
                            for row in y..y + h {
                                let line = &plane.data[row * plane.stride + x..row * plane.stride + x + w];
                                m.update(line);
                                raw.extend_from_slice(line);
                            }
                        }
                        println!("{}", m.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>());
                        // HEVC_DUMP=path:index writes picture `index` as raw planar 4:4:4.
                        if let Some((path, idx)) = std::env::var("HEVC_DUMP").ok().and_then(|v| v.split_once(':').map(|(p, i)| (p.to_string(), i.parse::<usize>().unwrap_or(0))))
                            && idx + 1 == n
                        {
                            std::fs::write(path, &raw).expect("dump");
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => eprintln!("unit {}: {e}", times.len() - 1),
            }
        }
        let _ = t;
        // The decode calls alone: the digests are not timed.
        let total: f64 = times.iter().sum();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
        eprintln!("{n} pictures, {total:.1} ms decoding: {:.2} ms/picture, median {:.2}, p95 {:.2}, max {:.2}", total / n.max(1) as f64, p(0.5), p(0.95), p(1.0));
    }
}
