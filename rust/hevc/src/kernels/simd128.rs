//! The sample loops in WebAssembly's 128-bit vectors: the same arithmetic as
//! the plain loops in `kernels`, eight or sixteen samples at a time. Every
//! kernel is exact: integer operations in the same order, so a run on either
//! path reconstructs the same picture.
//!
//! The only `unsafe` here is loading and storing a vector through a slice that
//! was just bounds-checked to hold it.

use core::arch::wasm32::*;

#[inline(always)]
fn load_u8x8(s: &[u8]) -> v128 {
    u64x2(u64::from_le_bytes(s[..8].try_into().unwrap()), 0)
}

#[inline(always)]
fn load_u8x4(s: &[u8]) -> v128 {
    u32x4(u32::from_le_bytes(s[..4].try_into().unwrap()), 0, 0, 0)
}

#[inline(always)]
fn load_i16x8(s: &[i16]) -> v128 {
    let s = &s[..8];
    // SAFETY: eight values are there, and an unaligned load is allowed.
    unsafe { v128_load(s.as_ptr() as *const v128) }
}

#[inline(always)]
fn load_i16x4(s: &[i16]) -> v128 {
    let s = &s[..4];
    // SAFETY: four values are there, and an unaligned load is allowed.
    unsafe { v128_load64_zero(s.as_ptr() as *const u64) }
}

#[inline(always)]
fn store_i16x8(d: &mut [i16], v: v128) {
    let d = &mut d[..8];
    // SAFETY: eight slots are there, and an unaligned store is allowed.
    unsafe { v128_store(d.as_mut_ptr() as *mut v128, v) }
}

#[inline(always)]
fn store_i16x4(d: &mut [i16], v: v128) {
    let b = u64x2_extract_lane::<0>(v).to_le_bytes();
    for i in 0..4 {
        d[i] = i16::from_le_bytes([b[2 * i], b[2 * i + 1]]);
    }
}

#[inline(always)]
fn store_u8x8(d: &mut [u8], v: v128) {
    d[..8].copy_from_slice(&u64x2_extract_lane::<0>(v).to_le_bytes());
}

#[inline(always)]
fn store_u8x4(d: &mut [u8], v: v128) {
    d[..4].copy_from_slice(&u32x4_extract_lane::<0>(v).to_le_bytes());
}

/// The eight (or four, for a width of four) samples at `x` of a row of
/// `w`, widened to 16 bits.
#[inline(always)]
fn load_samples(row: &[u8], w: usize) -> v128 {
    u16x8_extend_low_u8x16(if w >= 8 { load_u8x8(row) } else { load_u8x4(row) })
}

#[inline(always)]
fn store_samples(d: &mut [u8], w: usize, v: v128) {
    let b = u8x16_narrow_i16x8(v, v);
    if w >= 8 {
        store_u8x8(d, b)
    } else {
        store_u8x4(d, b)
    }
}

// ---- copies and fills ----

/// Checks once that `w`×`h` at `stride` lies inside `s`.
#[inline(always)]
fn holds<T>(s: &[T], stride: usize, w: usize, h: usize) {
    assert!(w > 0 && h > 0 && s.len() >= (h - 1) * stride + w);
}

/// `dst = src`, `w`×`h`, by vector: a `memory.copy` per row is a call into the
/// runtime, which for a block's rows costs more than the bytes. The block loops
/// here check their bounds once and then run on pointers, since a check per
/// vector was most of the work of a copy. The copy runs down each sixteen-wide
/// column, so the loads step by the stride and the prefetcher follows them,
/// and so a row of a wide block has no tail to test.
pub fn copy_block(dst: &mut [u8], dst_stride: usize, src: &[u8], src_stride: usize, w: usize, h: usize) {
    holds(dst, dst_stride, w, h);
    holds(src, src_stride, w, h);
    let (d, s) = (dst.as_mut_ptr(), src.as_ptr());
    let mut x = 0;
    while x + 16 <= w {
        for y in 0..h {
            // SAFETY: inside the block `holds` checked.
            unsafe { v128_store(d.add(y * dst_stride + x) as *mut v128, v128_load(s.add(y * src_stride + x) as *const v128)) };
        }
        x += 16;
    }
    if x < w {
        for y in 0..h {
            // SAFETY: as above.
            unsafe {
                let (dr, sr) = (d.add(y * dst_stride), s.add(y * src_stride));
                let mut x = x;
                if x + 8 <= w {
                    (dr.add(x) as *mut u64).write_unaligned((sr.add(x) as *const u64).read_unaligned());
                    x += 8;
                }
                if x + 4 <= w {
                    (dr.add(x) as *mut u32).write_unaligned((sr.add(x) as *const u32).read_unaligned());
                    x += 4;
                }
                while x < w {
                    *dr.add(x) = *sr.add(x);
                    x += 1;
                }
            }
        }
    }
}

/// An `n`×`n` block of one value.
pub fn fill(dst: &mut [u8], stride: usize, n: usize, v: u8) {
    holds(dst, stride, n, n);
    let vv = u8x16_splat(v);
    let d = dst.as_mut_ptr();
    for y in 0..n {
        // SAFETY: inside the block `holds` checked.
        unsafe {
            let dr = d.add(y * stride);
            let mut x = 0;
            while x + 16 <= n {
                v128_store(dr.add(x) as *mut v128, vv);
                x += 16;
            }
            if x + 8 <= n {
                v128_store64_lane::<0>(vv, dr.add(x) as *mut u64);
                x += 8;
            }
            if x + 4 <= n {
                v128_store32_lane::<0>(vv, dr.add(x) as *mut u32);
            }
        }
    }
}

pub fn fill_i16(dst: &mut [i16], v: i16) {
    let vv = i16x8_splat(v);
    let mut x = 0;
    while x + 8 <= dst.len() {
        store_i16x8(&mut dst[x..], vv);
        x += 8;
    }
    if x < dst.len() {
        store_i16x4(&mut dst[x..], vv);
    }
}

// ---- motion compensation ----

/// The eight (or four, for a width of four) samples at `p`, widened to 16
/// bits.
///
/// # Safety
/// `w` samples are readable at `p`.
#[inline(always)]
unsafe fn samples_at(p: *const u8, w: usize) -> v128 {
    // SAFETY: the caller's.
    unsafe { u16x8_extend_low_u8x16(if w >= 8 { v128_load64_zero(p as *const u64) } else { v128_load32_zero(p as *const u32) }) }
}

/// Eight (or four) 16-bit values at `p`.
///
/// # Safety
/// `w` values are readable at `p`.
#[inline(always)]
unsafe fn i16s_at(p: *const i16, w: usize) -> v128 {
    // SAFETY: the caller's.
    unsafe {
        if w >= 8 {
            v128_load(p as *const v128)
        } else {
            v128_load64_zero(p as *const u64)
        }
    }
}

/// Eight (or four) 16-bit sums as samples at `p`: `(v + 32) >> 6`, clipped.
///
/// # Safety
/// `w` bytes are writable at `p`.
#[inline(always)]
unsafe fn store_uni(p: *mut u8, w: usize, v: v128) {
    let b = u8x16_narrow_i16x8(i16x8_shr(i16x8_add(v, i16x8_splat(32)), 6), v);
    // SAFETY: the caller's.
    unsafe {
        if w >= 8 {
            v128_store64_lane::<0>(b, p as *mut u64);
        } else {
            v128_store32_lane::<0>(b, p as *mut u32);
        }
    }
}

/// The sums of an `N`-tap filter over 16-bit inputs never leave 16 bits for
/// 8-bit samples (the positive taps sum to 88 at most), so the passes over
/// samples accumulate in 16-bit lanes. Like the block copies, the filters
/// check their bounds once and run on pointers.
pub fn fir_h<const N: usize>(src: &[u8], stride: usize, t: &[i16; N], w: usize, h: usize, dst: &mut [i16], dst_stride: usize) {
    holds(src, stride, w + N - 1, h);
    holds(dst, dst_stride, w, h);
    let taps: [v128; N] = core::array::from_fn(|i| i16x8_splat(t[i]));
    let (s, d) = (src.as_ptr(), dst.as_mut_ptr());
    for y in 0..h {
        // SAFETY: inside the blocks `holds` checked; a width of four reads
        // and writes only its four.
        unsafe {
            let (sr, dr) = (s.add(y * stride), d.add(y * dst_stride));
            let mut x = 0;
            while x < w {
                let mut acc = i16x8_splat(0);
                for i in 0..N {
                    acc = i16x8_add(acc, i16x8_mul(samples_at(sr.add(x + i), w), taps[i]));
                }
                if w >= 8 {
                    v128_store(dr.add(x) as *mut v128, acc);
                } else {
                    v128_store64_lane::<0>(acc, dr.add(x) as *mut u64);
                }
                x += 8;
            }
        }
    }
}

pub fn fir_h_uni<const N: usize>(src: &[u8], stride: usize, t: &[i16; N], w: usize, h: usize, dst: &mut [u8], dst_stride: usize) {
    holds(src, stride, w + N - 1, h);
    holds(dst, dst_stride, w, h);
    let taps: [v128; N] = core::array::from_fn(|i| i16x8_splat(t[i]));
    let (s, d) = (src.as_ptr(), dst.as_mut_ptr());
    for y in 0..h {
        // SAFETY: as in `fir_h`.
        unsafe {
            let (sr, dr) = (s.add(y * stride), d.add(y * dst_stride));
            let mut x = 0;
            while x < w {
                let mut acc = i16x8_splat(0);
                for i in 0..N {
                    acc = i16x8_add(acc, i16x8_mul(samples_at(sr.add(x + i), w), taps[i]));
                }
                store_uni(dr.add(x), w, acc);
                x += 8;
            }
        }
    }
}

pub fn fir_v_uni<const N: usize>(src: &[u8], stride: usize, t: &[i16; N], w: usize, h: usize, dst: &mut [u8], dst_stride: usize) {
    holds(src, stride, w, h + N - 1);
    holds(dst, dst_stride, w, h);
    let taps: [v128; N] = core::array::from_fn(|i| i16x8_splat(t[i]));
    let (s, d) = (src.as_ptr(), dst.as_mut_ptr());
    for y in 0..h {
        // SAFETY: as in `fir_h`.
        unsafe {
            let (sr, dr) = (s.add(y * stride), d.add(y * dst_stride));
            let mut x = 0;
            while x < w {
                let mut acc = i16x8_splat(0);
                for i in 0..N {
                    acc = i16x8_add(acc, i16x8_mul(samples_at(sr.add(i * stride + x), w), taps[i]));
                }
                store_uni(dr.add(x), w, acc);
                x += 8;
            }
        }
    }
}

/// The pass over the intermediate needs 32-bit sums.
pub fn fir_hv_uni<const N: usize>(src: &[i16], stride: usize, t: &[i16; N], w: usize, h: usize, dst: &mut [u8], dst_stride: usize) {
    holds(src, stride, w, h + N - 1);
    holds(dst, dst_stride, w, h);
    let taps: [v128; N] = core::array::from_fn(|i| i16x8_splat(t[i]));
    let (s, d) = (src.as_ptr(), dst.as_mut_ptr());
    for y in 0..h {
        // SAFETY: as in `fir_h`.
        unsafe {
            let (sr, dr) = (s.add(y * stride), d.add(y * dst_stride));
            let mut x = 0;
            while x < w {
                let mut lo = i32x4_splat(0);
                let mut hi = i32x4_splat(0);
                for i in 0..N {
                    let v = i16s_at(sr.add(i * stride + x), w);
                    lo = i32x4_add(lo, i32x4_extmul_low_i16x8(v, taps[i]));
                    hi = i32x4_add(hi, i32x4_extmul_high_i16x8(v, taps[i]));
                }
                store_uni(dr.add(x), w, i16x8_narrow_i32x4(i32x4_shr(lo, 6), i32x4_shr(hi, 6)));
                x += 8;
            }
        }
    }
}

// ---- reconstruction ----

pub fn add_residual(dst: &mut [u8], dst_stride: usize, res: &[i16], w: usize, h: usize) {
    holds(dst, dst_stride, w, h);
    holds(res, w, w, h);
    let (d, s) = (dst.as_mut_ptr(), res.as_ptr());
    for y in 0..h {
        // SAFETY: inside the blocks `holds` checked; a width of four reads
        // and writes only its four.
        unsafe {
            let (dr, sr) = (d.add(y * dst_stride), s.add(y * w));
            let mut x = 0;
            while x < w {
                let (p, v) = if w >= 8 {
                    (v128_load64_zero(dr.add(x) as *const u64), v128_load(sr.add(x) as *const v128))
                } else {
                    (v128_load32_zero(dr.add(x) as *const u32), v128_load64_zero(sr.add(x) as *const u64))
                };
                let sum = i16x8_add(u16x8_extend_low_u8x16(p), v);
                let b = u8x16_narrow_i16x8(sum, sum);
                if w >= 8 {
                    v128_store64_lane::<0>(b, dr.add(x) as *mut u64);
                } else {
                    v128_store32_lane::<0>(b, dr.add(x) as *mut u32);
                }
                x += 8;
            }
        }
    }
}

// ---- intra prediction ----

pub fn planar(dst: &mut [u8], stride: usize, n: usize, left: &[u8], top: &[u8]) {
    let shift = n.trailing_zeros() + 1;
    let (tn, ln) = (top[n] as i16, left[n] as i16);
    // Per column: `n - 1 - x`, and `(x + 1) * top[n]`.
    let mut xs = [0i16; 32];
    let mut bs = [0i16; 32];
    for x in 0..n {
        xs[x] = (n - 1 - x) as i16;
        bs[x] = (x as i16 + 1) * tn;
    }
    for y in 0..n {
        let l = i16x8_splat(left[y] as i16);
        let k = i16x8_splat((n - 1 - y) as i16);
        let c = i16x8_splat((y as i16 + 1) * ln + n as i16);
        let row = &mut dst[y * stride..y * stride + n];
        let mut x = 0;
        while x < n {
            let t = load_samples(&top[x..], n);
            let (a, b) = if n >= 8 { (load_i16x8(&xs[x..]), load_i16x8(&bs[x..])) } else { (load_i16x4(&xs[x..]), load_i16x4(&bs[x..])) };
            let v = i16x8_add(i16x8_add(i16x8_mul(a, l), b), i16x8_add(i16x8_mul(k, t), c));
            store_samples(&mut row[x..], n, i16x8_shr(v, shift));
            x += 8;
        }
    }
}

/// One row of an angular prediction: `refb[base..]` and `refb[base + 1..]`
/// blended by `ifact` in 32nds.
#[inline(always)]
fn angular_row(out: &mut [u8], n: usize, refb: &[i16], base: usize, ifact: i32) {
    let (fa, fb) = (i16x8_splat((32 - ifact) as i16), i16x8_splat(ifact as i16));
    let round = i16x8_splat(16);
    let mut x = 0;
    while x < n {
        let (a, b) = if n >= 8 { (load_i16x8(&refb[base + x..]), load_i16x8(&refb[base + x + 1..])) } else { (load_i16x4(&refb[base + x..]), load_i16x4(&refb[base + x + 1..])) };
        let v = if ifact == 0 { a } else { i16x8_shr(i16x8_add(i16x8_add(i16x8_mul(a, fa), i16x8_mul(b, fb)), round), 5) };
        store_samples(&mut out[x..], n, v);
        x += 8;
    }
}

pub fn angular(dst: &mut [u8], stride: usize, n: usize, refb: &[i16], off: usize, angle: i32) {
    for y in 0..n {
        let pos = (y as i32 + 1) * angle;
        let base = (off as i32 + (pos >> 5) + 1) as usize;
        angular_row(&mut dst[y * stride..y * stride + n], n, refb, base, pos & 31);
    }
}

pub fn angular_t(dst: &mut [u8], stride: usize, n: usize, refb: &[i16], off: usize, angle: i32) {
    // The columns, each made as a row, then turned.
    let mut tmp = [0u8; 32 * 32];
    for x in 0..n {
        let pos = (x as i32 + 1) * angle;
        let base = (off as i32 + (pos >> 5) + 1) as usize;
        angular_row(&mut tmp[x * n..x * n + n], n, refb, base, pos & 31);
    }
    for y in 0..n {
        let row = &mut dst[y * stride..y * stride + n];
        for x in 0..n {
            row[x] = tmp[x * n + y];
        }
    }
}

// ---- the inverse transform ----

/// Coefficients `A` and `B` of the row in `v` (eight per vector), as one
/// 32-bit lane splatted: what `dot` multiplies against an interleaved pair of
/// transform rows.
macro_rules! splat_pair {
    ($v:expr, $a:literal, $b:literal) => {
        i16x8_shuffle::<{ $a % 8 }, { 8 + $b % 8 }, { $a % 8 }, { 8 + $b % 8 }, { $a % 8 }, { 8 + $b % 8 }, { $a % 8 }, { 8 + $b % 8 }>($v[$a / 8], $v[$b / 8])
    };
}

/// The row's coefficients as vectors, the ones past the `nz` live ones zero:
/// the column pass wrote exact zeros up to the next multiple of eight, and
/// nothing beyond.
#[inline(always)]
fn row_vecs<const N: usize>(row: &[i16], nz: usize) -> [v128; 4] {
    let mut v = [i16x8_splat(0); 4];
    if N == 4 {
        v[0] = load_i16x4(row);
        return v;
    }
    for k in 0..N / 8 {
        if 8 * k < nz {
            v[k] = load_i16x8(&row[8 * k..]);
        }
    }
    v
}

/// The sums of one stage along a row: `out[j] += a * Ta[j] + b * Tb[j]` over
/// the first `live` of the stage's `pairs`, four outputs per `dot`.
#[inline(always)]
fn stage_row<const P: usize>(acc: &mut [v128; 4], chunks: usize, table: &[[[i16; 8]; 4]], pairs: &[v128; P], live: usize) {
    for p in 0..P {
        if p == live {
            break;
        }
        for c in 0..chunks {
            acc[c] = i32x4_add(acc[c], i32x4_dot_i16x8(load_i16x8(&table[p][c]), pairs[p]));
        }
    }
}

/// How many of a stage's pairs start before coefficient `nz` of an `N`-point
/// row: the pairs are in order of their first coefficient.
#[inline(always)]
fn live_pairs<const N: usize>(stage: usize, nz: usize) -> usize {
    crate::itx::stages::K32[stage].iter().step_by(2).take_while(|&&k| k * N / 32 < nz).count()
}

/// Even and odd halves into the `n` outputs of one row: `out[j] = e + o`,
/// `out[n - 1 - j] = e - o`.
#[inline(always)]
fn butterfly_row(out: &mut [v128; 8], even: &[v128; 4], odd: &[v128; 4], n: usize) {
    let half = n / 2;
    for c in 0..half / 4 {
        out[c] = i32x4_add(even[c], odd[c]);
        let d = i32x4_sub(even[c], odd[c]);
        out[(n - 4 - 4 * c) / 4] = i32x4_shuffle::<3, 2, 1, 0>(d, d);
    }
}

/// The `N`-point inverse DCT of `row` (`nz` live inputs), as 32-bit sums in
/// `out[..N / 4]`. The pairs of each stage are the coefficients `K32` names,
/// scaled to `N`.
#[inline(always)]
fn idct_row<const N: usize>(row: &[i16], nz: usize, out: &mut [v128; 8]) {
    use crate::itx::stages::INTERLEAVED;
    let zero = i32x4_splat(0);
    let v = row_vecs::<N>(row, nz);
    let mut base = [zero; 4];
    let mut odd8 = [zero; 4];
    let mut odd16 = [zero; 4];
    let mut odd32 = [zero; 4];
    match N {
        4 => stage_row(&mut base, 1, &INTERLEAVED[0], &[splat_pair!(v, 0, 1), splat_pair!(v, 2, 3)], live_pairs::<N>(0, nz)),
        8 => {
            stage_row(&mut base, 1, &INTERLEAVED[0], &[splat_pair!(v, 0, 2), splat_pair!(v, 4, 6)], live_pairs::<N>(0, nz));
            stage_row(&mut odd8, 1, &INTERLEAVED[1], &[splat_pair!(v, 1, 3), splat_pair!(v, 5, 7)], live_pairs::<N>(1, nz));
        }
        16 => {
            stage_row(&mut base, 1, &INTERLEAVED[0], &[splat_pair!(v, 0, 4), splat_pair!(v, 8, 12)], live_pairs::<N>(0, nz));
            stage_row(&mut odd8, 1, &INTERLEAVED[1], &[splat_pair!(v, 2, 6), splat_pair!(v, 10, 14)], live_pairs::<N>(1, nz));
            stage_row(&mut odd16, 2, &INTERLEAVED[2], &[splat_pair!(v, 1, 3), splat_pair!(v, 5, 7), splat_pair!(v, 9, 11), splat_pair!(v, 13, 15)], live_pairs::<N>(2, nz));
        }
        _ => {
            stage_row(&mut base, 1, &INTERLEAVED[0], &[splat_pair!(v, 0, 8), splat_pair!(v, 16, 24)], live_pairs::<N>(0, nz));
            stage_row(&mut odd8, 1, &INTERLEAVED[1], &[splat_pair!(v, 4, 12), splat_pair!(v, 20, 28)], live_pairs::<N>(1, nz));
            stage_row(&mut odd16, 2, &INTERLEAVED[2], &[splat_pair!(v, 2, 6), splat_pair!(v, 10, 14), splat_pair!(v, 18, 22), splat_pair!(v, 26, 30)], live_pairs::<N>(2, nz));
            stage_row(
                &mut odd32,
                4,
                &INTERLEAVED[3],
                &[splat_pair!(v, 1, 3), splat_pair!(v, 5, 7), splat_pair!(v, 9, 11), splat_pair!(v, 13, 15), splat_pair!(v, 17, 19), splat_pair!(v, 21, 23), splat_pair!(v, 25, 27), splat_pair!(v, 29, 31)],
                live_pairs::<N>(3, nz),
            );
        }
    }
    if N == 4 {
        out[0] = base[0];
        return;
    }
    let mut e8 = [zero; 8];
    butterfly_row(&mut e8, &base, &odd8, 8);
    if N == 8 {
        out[0] = e8[0];
        out[1] = e8[1];
        return;
    }
    let mut e16 = [zero; 8];
    butterfly_row(&mut e16, &[e8[0], e8[1], zero, zero], &odd16, 16);
    if N == 16 {
        for c in 0..4 {
            out[c] = e16[c];
        }
        return;
    }
    butterfly_row(out, &[e16[0], e16[1], e16[2], e16[3]], &odd32, 32);
}

/// The 4-point inverse DST of `row`.
#[inline(always)]
fn idst_row(row: &[i16], nz: usize, out: &mut [v128; 8]) {
    use crate::itx::stages::DST_INTERLEAVED;
    let v = row_vecs::<4>(row, nz);
    let mut acc = [i32x4_splat(0); 4];
    stage_row(&mut acc, 1, &DST_INTERLEAVED, &[splat_pair!(v, 0, 1), splat_pair!(v, 2, 3)], if nz > 2 { 2 } else { 1 });
    out[0] = acc[0];
}

/// Rows `ka` and `kb` of the block at columns `x..x + 8`, interleaved: the
/// pairs `dot` takes, four columns per vector.
#[inline(always)]
fn zip_rows(d: &[i16], n: usize, ka: usize, kb: Option<usize>, x: usize) -> (v128, v128) {
    let a = if n >= 8 { load_i16x8(&d[ka * n + x..]) } else { load_i16x4(&d[ka * n + x..]) };
    let b = match kb {
        Some(kb) => {
            if n >= 8 {
                load_i16x8(&d[kb * n + x..])
            } else {
                load_i16x4(&d[kb * n + x..])
            }
        }
        None => i16x8_splat(0),
    };
    (i16x8_shuffle::<0, 8, 1, 9, 2, 10, 3, 11>(a, b), i16x8_shuffle::<4, 12, 5, 13, 6, 14, 7, 15>(a, b))
}

/// One stage down the columns `x..x + 8`: `acc[j] += Ta[j] * d[ka][x..] +
/// Tb[j] * d[kb][x..]`, two vectors of four columns per output. The pairs'
/// rows are zipped once, and the outputs go four at a time so their eight
/// sums stay in registers across the pairs.
#[inline(always)]
fn stage_cols(acc: &mut [[v128; 2]; 16], len: usize, table: &[[i32; 16]], d: &[i16], n: usize, x: usize, nz_h: usize, k: impl Fn(usize) -> usize) {
    let mut zips = [[i16x8_splat(0); 2]; 8];
    let mut live = 0;
    for p in 0..table.len() {
        let ka = k(2 * p);
        if ka >= nz_h {
            break;
        }
        let kb = k(2 * p + 1);
        let (lo, hi) = zip_rows(d, n, ka, (kb < nz_h).then_some(kb), x);
        zips[p] = [lo, hi];
        live = p + 1;
    }
    let mut j = 0;
    while j < len {
        let mut a = [i32x4_splat(0); 8];
        for p in 0..live {
            let [lo, hi] = zips[p];
            let t = &table[p];
            for i in 0..4 {
                let tv = i32x4_splat(t[j + i]);
                a[2 * i] = i32x4_add(a[2 * i], i32x4_dot_i16x8(lo, tv));
                a[2 * i + 1] = i32x4_add(a[2 * i + 1], i32x4_dot_i16x8(hi, tv));
            }
        }
        for i in 0..4 {
            acc[j + i] = [a[2 * i], a[2 * i + 1]];
        }
        j += 4;
    }
}

#[inline(always)]
fn butterfly_cols(out: &mut [[v128; 2]; 32], even: &[[v128; 2]; 16], odd: &[[v128; 2]; 16], n: usize) {
    for j in 0..n / 2 {
        out[j] = [i32x4_add(even[j][0], odd[j][0]), i32x4_add(even[j][1], odd[j][1])];
        out[n - 1 - j] = [i32x4_sub(even[j][0], odd[j][0]), i32x4_sub(even[j][1], odd[j][1])];
    }
}

/// The `N`-point inverse DCT down eight columns at once, from the rows of `d`
/// at `x`, as 32-bit sums per output row.
#[inline(always)]
fn idct_cols<const N: usize>(d: &[i16], x: usize, nz_h: usize, out: &mut [[v128; 2]; 32]) {
    use crate::itx::stages::{K32, PACKED};
    let zero = [i32x4_splat(0); 2];
    let k = |stage: usize| move |i: usize| K32[stage][i] * N / 32;
    // Each level's even half is the level below's output, in place in `out`.
    let mut even = [zero; 16];
    stage_cols(&mut even, 4, &PACKED[0][..2], d, N, x, nz_h, k(0));
    if N == 4 {
        for j in 0..4 {
            out[j] = even[j];
        }
        return;
    }
    let mut odd = [zero; 16];
    stage_cols(&mut odd, 4, &PACKED[1][..2], d, N, x, nz_h, k(1));
    butterfly_cols(out, &even, &odd, 8);
    if N == 8 {
        return;
    }
    for j in 0..8 {
        even[j] = out[j];
    }
    let mut odd = [zero; 16];
    stage_cols(&mut odd, 8, &PACKED[2][..4], d, N, x, nz_h, k(2));
    butterfly_cols(out, &even, &odd, 16);
    if N == 16 {
        return;
    }
    for j in 0..16 {
        even[j] = out[j];
    }
    let mut odd = [zero; 16];
    stage_cols(&mut odd, 16, &PACKED[3][..8], d, N, x, nz_h, k(3));
    butterfly_cols(out, &even, &odd, 32);
}

#[inline(always)]
fn idst_cols(d: &[i16], x: usize, nz_h: usize, out: &mut [[v128; 2]; 32]) {
    use crate::itx::stages::DST_PACKED;
    let zero = [i32x4_splat(0); 2];
    let mut base = [zero; 16];
    stage_cols(&mut base, 4, &DST_PACKED, d, 4, x, nz_h, |i| i);
    for j in 0..4 {
        out[j] = base[j];
    }
}

/// Both passes of an `N`×`N` block: columns, eight at a time, into `tmp`,
/// clipped to 16 bits; then each row into `res`.
#[inline(always)]
fn block<const N: usize>(d: &[i16], tmp: &mut [i16], res: &mut [i16], nz_w: usize, nz_h: usize, cols: impl Fn(&[i16], usize, usize, &mut [[v128; 2]; 32]), rows: impl Fn(&[i16], usize, &mut [v128; 8])) {
    let round1 = i32x4_splat(64);
    let mut sums = [[i32x4_splat(0); 2]; 32];
    let mut x = 0;
    while x < nz_w {
        cols(d, x, nz_h, &mut sums);
        for j in 0..N {
            let lo = i32x4_shr(i32x4_add(sums[j][0], round1), 7);
            let hi = i32x4_shr(i32x4_add(sums[j][1], round1), 7);
            let v = i16x8_narrow_i32x4(lo, hi);
            if N >= 8 {
                store_i16x8(&mut tmp[j * N + x..], v);
            } else {
                store_i16x4(&mut tmp[j * N + x..], v);
            }
        }
        x += 8;
    }
    let round2 = i32x4_splat(2048);
    let mut out = [i32x4_splat(0); 8];
    for y in 0..N {
        rows(&tmp[y * N..y * N + N], nz_w, &mut out);
        let r = &mut res[y * N..y * N + N];
        let mut j = 0;
        while j < N {
            let lo = i32x4_shr(i32x4_add(out[j / 4], round2), 12);
            if N >= 8 {
                let hi = i32x4_shr(i32x4_add(out[j / 4 + 1], round2), 12);
                store_i16x8(&mut r[j..], i16x8_narrow_i32x4(lo, hi));
            } else {
                store_i16x4(&mut r[j..], i16x8_narrow_i32x4(lo, lo));
            }
            j += 8;
        }
    }
}

pub fn inverse_transform(d: &[i16], tmp: &mut [i16], res: &mut [i16], n: usize, nz_w: usize, nz_h: usize, dst: bool) {
    match (n, dst) {
        (4, true) => block::<4>(d, tmp, res, nz_w, nz_h, idst_cols, idst_row),
        (4, false) => block::<4>(d, tmp, res, nz_w, nz_h, idct_cols::<4>, idct_row::<4>),
        (8, _) => block::<8>(d, tmp, res, nz_w, nz_h, idct_cols::<8>, idct_row::<8>),
        (16, _) => block::<16>(d, tmp, res, nz_w, nz_h, idct_cols::<16>, idct_row::<16>),
        _ => block::<32>(d, tmp, res, nz_w, nz_h, idct_cols::<32>, idct_row::<32>),
    }
}

// ---- deblocking ----

/// Checks that the four-line edge segment at (`x`, `y`) with `n` taps a side
/// lies inside `data`.
#[inline(always)]
fn edge_holds(data: &[u8], stride: usize, x: usize, y: usize, dir: usize, n: usize) {
    let inside = if dir == 0 { x >= n && (y + 3) * stride + x + n <= data.len() } else { y >= n && (y + n - 1) * stride + x + 4 <= data.len() };
    assert!(inside);
}

/// The four lines of one tap of an edge, from their bytes in the low lanes.
#[inline(always)]
fn tap(v: v128) -> v128 {
    u16x8_extend_low_u8x16(v)
}

/// `|a - b|`.
#[inline(always)]
fn abs_diff(a: v128, b: v128) -> v128 {
    i16x8_abs(i16x8_sub(a, b))
}

/// `v` clipped to `c ± t`.
#[inline(always)]
fn clip_near(v: v128, c: v128, t: v128) -> v128 {
    i16x8_min(i16x8_max(v, i16x8_sub(c, t)), i16x8_add(c, t))
}

/// `v` clipped to `±t`.
#[inline(always)]
fn clip_abs(v: v128, t: v128) -> v128 {
    i16x8_min(i16x8_max(v, i16x8_neg(t)), t)
}

/// The eight taps `p3..p0, q0..q3` across the edge whose first line's `q0`
/// is at `p`, each as its four lines in the low lanes. A horizontal edge's
/// lines are columns, so a tap is four bytes of a row; a vertical edge's
/// are rows, so the taps are the columns of a 4×8 block, transposed by
/// shuffles.
///
/// # Safety
/// The segment must lie inside the plane (`edge_holds`).
#[inline(always)]
unsafe fn luma_taps(p: *const u8, stride: usize, dir: usize) -> [v128; 8] {
    if dir == 1 {
        let row = |i: isize| unsafe { tap(v128_load32_zero(p.offset((i - 4) * stride as isize) as *const u32)) };
        [row(0), row(1), row(2), row(3), row(4), row(5), row(6), row(7)]
    } else {
        let line = |k: usize| unsafe { p.add(k * stride).sub(4) as *const u64 };
        let a = unsafe { v128_load64_lane::<1>(v128_load64_zero(line(0)), line(1)) };
        let b = unsafe { v128_load64_lane::<1>(v128_load64_zero(line(2)), line(3)) };
        macro_rules! column {
            ($i:literal) => {
                tap(i8x16_shuffle::<$i, { 8 + $i }, { 16 + $i }, { 24 + $i }, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0>(a, b))
            };
        }
        [column!(0), column!(1), column!(2), column!(3), column!(4), column!(5), column!(6), column!(7)]
    }
}

/// Stores the taps `luma_taps` loaded, as changed: for a horizontal edge the
/// three (`wide`) or two rows each side, for a vertical edge the four lines
/// whole, transposed back.
///
/// # Safety
/// As `luma_taps`.
#[inline(always)]
unsafe fn store_luma_taps(p: *mut u8, stride: usize, dir: usize, t: &[v128; 8], wide: bool) {
    if dir == 1 {
        let from = if wide { 1 } else { 2 };
        for i in from..8 - from {
            // SAFETY: the caller's.
            unsafe { v128_store32_lane::<0>(u8x16_narrow_i16x8(t[i], t[i]), p.offset((i as isize - 4) * stride as isize) as *mut u32) };
        }
    } else {
        let line = |k: usize| unsafe { p.add(k * stride).sub(4) as *mut u64 };
        // Each narrowing puts one tap's lines at 0..4 and the next's at 8..12;
        // the shuffles gather each line's eight bytes.
        let (n0, n1, n2, n3) = (u8x16_narrow_i16x8(t[0], t[1]), u8x16_narrow_i16x8(t[2], t[3]), u8x16_narrow_i16x8(t[4], t[5]), u8x16_narrow_i16x8(t[6], t[7]));
        let ps = i8x16_shuffle::<0, 8, 16, 24, 1, 9, 17, 25, 2, 10, 18, 26, 3, 11, 19, 27>(n0, n1);
        let qs = i8x16_shuffle::<0, 8, 16, 24, 1, 9, 17, 25, 2, 10, 18, 26, 3, 11, 19, 27>(n2, n3);
        let l01 = i8x16_shuffle::<0, 1, 2, 3, 16, 17, 18, 19, 4, 5, 6, 7, 20, 21, 22, 23>(ps, qs);
        let l23 = i8x16_shuffle::<8, 9, 10, 11, 24, 25, 26, 27, 12, 13, 14, 15, 28, 29, 30, 31>(ps, qs);
        // SAFETY: the caller's.
        unsafe {
            v128_store64_lane::<0>(l01, line(0));
            v128_store64_lane::<1>(l01, line(1));
            v128_store64_lane::<0>(l23, line(2));
            v128_store64_lane::<1>(l23, line(3));
        }
    }
}

pub fn luma_edge(data: &mut [u8], stride: usize, x: usize, y: usize, dir: usize, beta: i32, tc: i32) {
    edge_holds(data, stride, x, y, dir, 4);
    let p = data[y * stride + x..].as_mut_ptr();
    // SAFETY: checked above.
    let t = unsafe { luma_taps(p, stride, dir) };
    let [p3, p2, p1, p0, q0, q1, q2, q3] = t;
    let dp = i16x8_abs(i16x8_sub(i16x8_add(p2, p0), i16x8_shl(p1, 1)));
    let dq = i16x8_abs(i16x8_sub(i16x8_add(q2, q0), i16x8_shl(q1, 1)));
    let (dp0, dp3) = (i16x8_extract_lane::<0>(dp) as i32, i16x8_extract_lane::<3>(dp) as i32);
    let (dq0, dq3) = (i16x8_extract_lane::<0>(dq) as i32, i16x8_extract_lane::<3>(dq) as i32);
    let (dpq0, dpq3) = (dp0 + dq0, dp3 + dq3);
    if dpq0 + dpq3 >= beta {
        return;
    }
    let span = i16x8_add(abs_diff(p3, p0), abs_diff(q0, q3));
    let gap = abs_diff(p0, q0);
    let dsam = |dpq: i32, span: i32, gap: i32| 2 * dpq < beta >> 2 && span < beta >> 3 && gap < (5 * tc + 1) >> 1;
    let strong = dsam(dpq0, i16x8_extract_lane::<0>(span) as i32, i16x8_extract_lane::<0>(gap) as i32) && dsam(dpq3, i16x8_extract_lane::<3>(span) as i32, i16x8_extract_lane::<3>(gap) as i32);
    let side = (beta + (beta >> 1)) >> 3;
    let (dep, deq) = (dp0 + dp3 < side, dq0 + dq3 < side);
    let tcv = i16x8_splat(tc as i16);
    let (two, four) = (i16x8_splat(2), i16x8_splat(4));
    let out = if strong {
        let t2 = i16x8_shl(tcv, 1);
        let (p1p0, q0q1) = (i16x8_add(p1, p0), i16x8_add(q0, q1));
        let np0 = clip_near(i16x8_shr(i16x8_add(i16x8_add(i16x8_add(p2, i16x8_shl(i16x8_add(p1p0, q0), 1)), q1), four), 3), p0, t2);
        let np1 = clip_near(i16x8_shr(i16x8_add(i16x8_add(p2, i16x8_add(p1p0, q0)), two), 2), p1, t2);
        let np2 = clip_near(i16x8_shr(i16x8_add(i16x8_add(i16x8_add(i16x8_shl(p3, 1), i16x8_mul(p2, i16x8_splat(3))), i16x8_add(p1p0, q0)), four), 3), p2, t2);
        let nq0 = clip_near(i16x8_shr(i16x8_add(i16x8_add(i16x8_add(p1, i16x8_shl(i16x8_add(p0, q0q1), 1)), q2), four), 3), q0, t2);
        let nq1 = clip_near(i16x8_shr(i16x8_add(i16x8_add(p0, i16x8_add(q0q1, q2)), two), 2), q1, t2);
        let nq2 = clip_near(i16x8_shr(i16x8_add(i16x8_add(i16x8_add(p0, q0q1), i16x8_add(i16x8_mul(q2, i16x8_splat(3)), i16x8_shl(q3, 1))), four), 3), q2, t2);
        [p3, np2, np1, np0, nq0, nq1, nq2, q3]
    } else {
        let delta = i16x8_shr(i16x8_add(i16x8_sub(i16x8_mul(i16x8_sub(q0, p0), i16x8_splat(9)), i16x8_mul(i16x8_sub(q1, p1), i16x8_splat(3))), i16x8_splat(8)), 4);
        let apply = i16x8_lt(i16x8_abs(delta), i16x8_mul(tcv, i16x8_splat(10)));
        let delta = clip_abs(delta, tcv);
        let half = i16x8_shr(tcv, 1);
        let np0 = v128_bitselect(i16x8_add(p0, delta), p0, apply);
        let nq0 = v128_bitselect(i16x8_sub(q0, delta), q0, apply);
        let np1 = if dep {
            let d = clip_abs(i16x8_shr(i16x8_add(i16x8_sub(i16x8_shr(i16x8_add(i16x8_add(p2, p0), i16x8_splat(1)), 1), p1), delta), 1), half);
            v128_bitselect(i16x8_add(p1, d), p1, apply)
        } else {
            p1
        };
        let nq1 = if deq {
            let d = clip_abs(i16x8_shr(i16x8_sub(i16x8_sub(i16x8_shr(i16x8_add(i16x8_add(q2, q0), i16x8_splat(1)), 1), q1), delta), 1), half);
            v128_bitselect(i16x8_add(q1, d), q1, apply)
        } else {
            q1
        };
        [p3, p2, np1, np0, nq0, nq1, q2, q3]
    };
    // SAFETY: checked above.
    unsafe { store_luma_taps(p, stride, dir, &out, strong) }
}

pub fn chroma_edge(data: &mut [u8], stride: usize, x: usize, y: usize, dir: usize, tc: i32) {
    edge_holds(data, stride, x, y, dir, 2);
    let p = data[y * stride + x..].as_mut_ptr();
    // The taps `p1, p0, q0, q1`, as `luma_taps` makes them; a vertical
    // edge's four lines of four bytes fill one vector.
    // SAFETY: checked above.
    let [p1, p0, q0, q1] = unsafe {
        if dir == 1 {
            let row = |i: isize| tap(v128_load32_zero(p.offset((i - 2) * stride as isize) as *const u32));
            [row(0), row(1), row(2), row(3)]
        } else {
            let line = |k: usize| p.add(k * stride).sub(2) as *const u32;
            let a = v128_load32_lane::<3>(v128_load32_lane::<2>(v128_load32_lane::<1>(v128_load32_zero(line(0)), line(1)), line(2)), line(3));
            macro_rules! column {
                ($i:literal) => {
                    tap(i8x16_shuffle::<$i, { 4 + $i }, { 8 + $i }, { 12 + $i }, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0>(a, a))
                };
            }
            [column!(0), column!(1), column!(2), column!(3)]
        }
    };
    let tcv = i16x8_splat(tc as i16);
    let d = clip_abs(i16x8_shr(i16x8_add(i16x8_sub(i16x8_add(i16x8_shl(i16x8_sub(q0, p0), 2), p1), q1), i16x8_splat(4)), 3), tcv);
    let (np0, nq0) = (i16x8_add(p0, d), i16x8_sub(q0, d));
    // SAFETY: checked above.
    unsafe {
        if dir == 1 {
            v128_store32_lane::<0>(u8x16_narrow_i16x8(np0, np0), p.sub(stride) as *mut u32);
            v128_store32_lane::<0>(u8x16_narrow_i16x8(nq0, nq0), p as *mut u32);
        } else {
            let n = u8x16_narrow_i16x8(np0, nq0);
            let pairs = i8x16_shuffle::<0, 8, 1, 9, 2, 10, 3, 11, 0, 0, 0, 0, 0, 0, 0, 0>(n, n);
            let line = |k: usize| p.add(k * stride).sub(1) as *mut u16;
            v128_store16_lane::<0>(pairs, line(0));
            v128_store16_lane::<1>(pairs, line(1));
            v128_store16_lane::<2>(pairs, line(2));
            v128_store16_lane::<3>(pairs, line(3));
        }
    }
}

// ---- sample adaptive offset ----

/// `v + off`, `off` a signed offset of at most ±7, clipped to the sample range.
#[inline(always)]
fn add_offset(v: v128, off: v128) -> v128 {
    let zero = i8x16_splat(0);
    let pos = i8x16_max(off, zero);
    let neg = i8x16_max(i8x16_sub(zero, off), zero);
    u8x16_sub_sat(u8x16_add_sat(v, pos), neg)
}

pub fn sao_band(data: &mut [u8], stride: usize, w: usize, h: usize, band: &[i8; 32]) {
    holds(data, stride, w, h);
    let lo = i8x16(band[0], band[1], band[2], band[3], band[4], band[5], band[6], band[7], band[8], band[9], band[10], band[11], band[12], band[13], band[14], band[15]);
    let hi = i8x16(band[16], band[17], band[18], band[19], band[20], band[21], band[22], band[23], band[24], band[25], band[26], band[27], band[28], band[29], band[30], band[31]);
    let sixteen = u8x16_splat(16);
    let d = data.as_mut_ptr();
    for y in 0..h {
        // SAFETY: inside the block `holds` checked.
        unsafe {
            let dr = d.add(y * stride);
            let mut x = 0;
            while x + 16 <= w {
                let v = v128_load(dr.add(x) as *const v128);
                let idx = u8x16_shr(v, 3);
                // A swizzle index past 15 reads as 0, so the two halves sum.
                let off = v128_or(u8x16_swizzle(lo, idx), u8x16_swizzle(hi, u8x16_sub(idx, sixteen)));
                v128_store(dr.add(x) as *mut v128, add_offset(v, off));
                x += 16;
            }
            for x in x..w {
                let v = *dr.add(x);
                *dr.add(x) = (v as i32 + band[(v >> 3) as usize] as i32).clamp(0, 255) as u8;
            }
        }
    }
}

pub fn sao_edge(dst: &mut [u8], dst_stride: usize, src: &[u8], origin: usize, src_stride: usize, w: usize, h: usize, oa: isize, ob: isize, offs: &[i8; 4]) {
    holds(dst, dst_stride, w, h);
    // The block and its neighbours in `src`, checked once.
    let (lo, hi) = (oa.min(ob).min(0), oa.max(ob).max(0));
    assert!(w > 0 && h > 0 && origin as isize + lo >= 0 && origin + (h - 1) * src_stride + w - 1 < (src.len() as isize - hi) as usize);
    let table = [offs[0], offs[1], 0, offs[2], offs[3]];
    let tab = i8x16(table[0], table[1], 0, table[3], table[4], 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
    let two = i8x16_splat(2);
    let (d, s) = (dst.as_mut_ptr(), src.as_ptr());
    for y in 0..h {
        // SAFETY: inside the blocks checked above.
        unsafe {
            let dr = d.add(y * dst_stride);
            let sr = s.add(origin + y * src_stride);
            let mut x = 0;
            while x + 16 <= w {
                let p = sr.add(x);
                let v = v128_load(p as *const v128);
                let a = v128_load(p.offset(oa) as *const v128);
                let b = v128_load(p.offset(ob) as *const v128);
                // sign(v - a) is (v < a) - (v > a) with the masks being -1.
                let sa = i8x16_sub(u8x16_lt(v, a), u8x16_gt(v, a));
                let sb = i8x16_sub(u8x16_lt(v, b), u8x16_gt(v, b));
                let e = i8x16_add(two, i8x16_add(sa, sb));
                v128_store(dr.add(x) as *mut v128, add_offset(v, u8x16_swizzle(tab, e)));
                x += 16;
            }
            for x in x..w {
                let p = sr.add(x);
                let v = *p as i32;
                let a = *p.offset(oa) as i32;
                let b = *p.offset(ob) as i32;
                let e = (2 + (v - a).signum() + (v - b).signum()) as usize;
                *dr.add(x) = (v + table[e] as i32).clamp(0, 255) as u8;
            }
        }
    }
}
