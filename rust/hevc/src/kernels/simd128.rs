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
fn load_u8x16(s: &[u8]) -> v128 {
    let s = &s[..16];
    // SAFETY: sixteen bytes are there, and an unaligned load is allowed.
    unsafe { v128_load(s.as_ptr() as *const v128) }
}

#[inline(always)]
fn load_i16x8(s: &[i16]) -> v128 {
    let s = &s[..8];
    // SAFETY: eight values are there, and an unaligned load is allowed.
    unsafe { v128_load(s.as_ptr() as *const v128) }
}

#[inline(always)]
fn load_i16x4(s: &[i16]) -> v128 {
    i16x8(s[0], s[1], s[2], s[3], 0, 0, 0, 0)
}

#[inline(always)]
fn load_i32x4(s: &[i32]) -> v128 {
    let s = &s[..4];
    // SAFETY: four values are there, and an unaligned load is allowed.
    unsafe { v128_load(s.as_ptr() as *const v128) }
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
fn store_i32x4(d: &mut [i32], v: v128) {
    let d = &mut d[..4];
    // SAFETY: as above.
    unsafe { v128_store(d.as_mut_ptr() as *mut v128, v) }
}

#[inline(always)]
fn store_u8x16(d: &mut [u8], v: v128) {
    let d = &mut d[..16];
    // SAFETY: as above.
    unsafe { v128_store(d.as_mut_ptr() as *mut v128, v) }
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

/// `dst = src`, `w`×`h`, by vector: a `memory.copy` per row is a call into the
/// runtime, which for a block's rows costs more than the bytes.
pub fn copy_block(dst: &mut [u8], dst_stride: usize, src: &[u8], src_stride: usize, w: usize, h: usize) {
    for y in 0..h {
        let (d, s) = (&mut dst[y * dst_stride..y * dst_stride + w], &src[y * src_stride..y * src_stride + w]);
        let mut x = 0;
        while x + 16 <= w {
            store_u8x16(&mut d[x..], load_u8x16(&s[x..]));
            x += 16;
        }
        if x + 8 <= w {
            store_u8x8(&mut d[x..], load_u8x8(&s[x..]));
            x += 8;
        }
        if x + 4 <= w {
            store_u8x4(&mut d[x..], load_u8x4(&s[x..]));
            x += 4;
        }
        for x in x..w {
            d[x] = s[x];
        }
    }
}

/// An `n`×`n` block of one value.
pub fn fill(dst: &mut [u8], stride: usize, n: usize, v: u8) {
    let vv = u8x16_splat(v);
    for y in 0..n {
        let row = &mut dst[y * stride..y * stride + n];
        let mut x = 0;
        while x + 16 <= n {
            store_u8x16(&mut row[x..], vv);
            x += 16;
        }
        if x + 8 <= n {
            store_u8x8(&mut row[x..], vv);
            x += 8;
        }
        if x + 4 <= n {
            store_u8x4(&mut row[x..], vv);
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

/// The sums of an `N`-tap filter over 16-bit inputs never leave 16 bits for
/// 8-bit samples (the positive taps sum to 80 at most), so the horizontal
/// and sample-vertical passes accumulate in 16-bit lanes.
pub fn fir_h<const N: usize>(src: &[u8], stride: usize, t: &[i16; N], w: usize, h: usize, dst: &mut [i16], dst_stride: usize) {
    let taps: [v128; N] = core::array::from_fn(|i| i16x8_splat(t[i]));
    for y in 0..h {
        let row = &src[y * stride..y * stride + w + N - 1];
        let out = &mut dst[y * dst_stride..y * dst_stride + w];
        let mut x = 0;
        while x < w {
            let mut acc = i16x8_splat(0);
            for i in 0..N {
                acc = i16x8_add(acc, i16x8_mul(load_samples(&row[x + i..], w), taps[i]));
            }
            if w >= 8 {
                store_i16x8(&mut out[x..], acc);
            } else {
                store_i16x4(&mut out[x..], acc);
            }
            x += 8;
        }
    }
}

pub fn fir_v_u8<const N: usize>(src: &[u8], stride: usize, t: &[i16; N], w: usize, h: usize, dst: &mut [i16], dst_stride: usize) {
    let taps: [v128; N] = core::array::from_fn(|i| i16x8_splat(t[i]));
    for y in 0..h {
        let out = &mut dst[y * dst_stride..y * dst_stride + w];
        let mut x = 0;
        while x < w {
            let mut acc = i16x8_splat(0);
            for i in 0..N {
                acc = i16x8_add(acc, i16x8_mul(load_samples(&src[(y + i) * stride + x..], w), taps[i]));
            }
            if w >= 8 {
                store_i16x8(&mut out[x..], acc);
            } else {
                store_i16x4(&mut out[x..], acc);
            }
            x += 8;
        }
    }
}

/// The vertical pass over the intermediate needs 32-bit sums.
pub fn fir_v_i16<const N: usize>(src: &[i16], stride: usize, t: &[i16; N], w: usize, h: usize, dst: &mut [i16], dst_stride: usize) {
    let taps: [v128; N] = core::array::from_fn(|i| i16x8_splat(t[i]));
    for y in 0..h {
        let out = &mut dst[y * dst_stride..y * dst_stride + w];
        let mut x = 0;
        while x < w {
            let mut lo = i32x4_splat(0);
            let mut hi = i32x4_splat(0);
            for i in 0..N {
                let s = &src[(y + i) * stride + x..];
                let v = if w >= 8 { load_i16x8(s) } else { load_i16x4(s) };
                lo = i32x4_add(lo, i32x4_extmul_low_i16x8(v, taps[i]));
                hi = i32x4_add(hi, i32x4_extmul_high_i16x8(v, taps[i]));
            }
            let r = i16x8_narrow_i32x4(i32x4_shr(lo, 6), i32x4_shr(hi, 6));
            if w >= 8 {
                store_i16x8(&mut out[x..], r);
            } else {
                store_i16x4(&mut out[x..], r);
            }
            x += 8;
        }
    }
}

pub fn put_uni(dst: &mut [u8], dst_stride: usize, src: &[i16], w: usize, h: usize) {
    let round = i16x8_splat(32);
    for y in 0..h {
        let row = &mut dst[y * dst_stride..y * dst_stride + w];
        let s = &src[y * w..y * w + w];
        let mut x = 0;
        while x < w {
            let v = if w >= 8 { load_i16x8(&s[x..]) } else { load_i16x4(&s[x..]) };
            store_samples(&mut row[x..], w, i16x8_shr(i16x8_add(v, round), 6));
            x += 8;
        }
    }
}

// ---- reconstruction ----

pub fn add_residual(dst: &mut [u8], dst_stride: usize, res: &[i16], w: usize, h: usize) {
    for y in 0..h {
        let row = &mut dst[y * dst_stride..y * dst_stride + w];
        let r = &res[y * w..y * w + w];
        let mut x = 0;
        while x < w {
            let d = load_samples(&row[x..], w);
            let v = if w >= 8 { load_i16x8(&r[x..]) } else { load_i16x4(&r[x..]) };
            store_samples(&mut row[x..], w, i16x8_add(d, v));
            x += 8;
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

// ---- scaling and the inverse transform ----

/// Scaling in 32 bits: `c * levelScale * 16` fits, and the `<< (qp / 6)` and
/// `>> bdShift` fold into one shift either way.
pub fn dequant(coeffs: &mut [i16], n: usize, nz_w: usize, nz_h: usize, qp: i32) {
    let log2n = n.trailing_zeros() as i32;
    let r = 8 + log2n - 5;
    let l = qp / 6;
    let scale = i16x8_splat((crate::tables::LEVEL_SCALE[(qp % 6) as usize] * 16) as i16);
    let round = i32x4_splat(if l < r { 1 << (r - l - 1) } else { 0 });
    let (shl, shr) = ((l - r).max(0) as u32, (r - l).max(0) as u32);
    for y in 0..nz_h {
        let row = &mut coeffs[y * n..y * n + n];
        let mut x = 0;
        while x < nz_w {
            let c = if n >= 8 { load_i16x8(&row[x..]) } else { load_i16x4(&row[x..]) };
            let lo = i32x4_shr(i32x4_add(i32x4_shl(i32x4_extmul_low_i16x8(c, scale), shl), round), shr);
            let hi = i32x4_shr(i32x4_add(i32x4_shl(i32x4_extmul_high_i16x8(c, scale), shl), round), shr);
            let v = i16x8_narrow_i32x4(lo, hi);
            if n >= 8 {
                store_i16x8(&mut row[x..], v);
            } else {
                store_i16x4(&mut row[x..], v);
            }
            x += 8;
        }
    }
}

/// `(a, b)` as one 32-bit lane, splatted, for `dot` against an interleaved pair.
#[inline(always)]
fn pair(a: i16, b: i16) -> v128 {
    i32x4_splat((a as u16 as u32 | ((b as u16 as u32) << 16)) as i32)
}

/// The sums of one stage along a row: `out[j] += a * Ta[j] + b * Tb[j]` over
/// the stage's pairs, four outputs per `dot`. `inputs` gives each pair's two
/// coefficients, zero past the live ones.
#[inline(always)]
fn stage_row(acc: &mut [v128; 4], chunks: usize, table: &[[[i16; 8]; 4]], inputs: impl Fn(usize) -> Option<(i16, i16)>) {
    for (p, t) in table.iter().enumerate() {
        let Some((a, b)) = inputs(p) else { break };
        if a == 0 && b == 0 {
            continue;
        }
        let s = pair(a, b);
        for c in 0..chunks {
            acc[c] = i32x4_add(acc[c], i32x4_dot_i16x8(load_i16x8(&t[c]), s));
        }
    }
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
/// `out[..N / 4]`.
#[inline(always)]
fn idct_row<const N: usize>(row: &[i16], nz: usize, out: &mut [v128; 8]) {
    use crate::itx::stages::{INTERLEAVED, K32};
    let zero = i32x4_splat(0);
    // The input of pair `p` of a stage: coefficient `K32[..] * N / 32`.
    let input = |stage: usize| {
        move |p: usize| {
            let ka = K32[stage][2 * p] * N / 32;
            if ka >= nz {
                return None;
            }
            let kb = K32[stage][2 * p + 1] * N / 32;
            Some((row[ka], if kb < nz { row[kb] } else { 0 }))
        }
    };
    let mut base = [zero; 4];
    stage_row(&mut base, 1, &INTERLEAVED[0][..2], input(0));
    if N == 4 {
        out[0] = base[0];
        return;
    }
    let mut odd8 = [zero; 4];
    stage_row(&mut odd8, 1, &INTERLEAVED[1][..2], input(1));
    let mut e8 = [zero; 8];
    butterfly_row(&mut e8, &base, &odd8, 8);
    if N == 8 {
        out[0] = e8[0];
        out[1] = e8[1];
        return;
    }
    let mut odd16 = [zero; 4];
    stage_row(&mut odd16, 2, &INTERLEAVED[2][..4], input(2));
    let mut e16 = [zero; 8];
    butterfly_row(&mut e16, &[e8[0], e8[1], zero, zero], &odd16, 16);
    if N == 16 {
        for c in 0..4 {
            out[c] = e16[c];
        }
        return;
    }
    let mut odd32 = [zero; 4];
    stage_row(&mut odd32, 4, &INTERLEAVED[3][..8], input(3));
    butterfly_row(out, &[e16[0], e16[1], e16[2], e16[3]], &odd32, 32);
}

/// The 4-point inverse DST of `row`.
#[inline(always)]
fn idst_row(row: &[i16], nz: usize, out: &mut [v128; 8]) {
    use crate::itx::stages::DST_INTERLEAVED;
    let mut acc = i32x4_splat(0);
    for p in 0..2 {
        let (ka, kb) = (2 * p, 2 * p + 1);
        if ka >= nz {
            break;
        }
        let (a, b) = (row[ka], if kb < nz { row[kb] } else { 0 });
        if a != 0 || b != 0 {
            acc = i32x4_add(acc, i32x4_dot_i16x8(load_i16x8(&DST_INTERLEAVED[p]), pair(a, b)));
        }
    }
    out[0] = acc;
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
/// Tb[j] * d[kb][x..]`, two vectors of four columns per output.
#[inline(always)]
fn stage_cols(acc: &mut [[v128; 2]; 16], len: usize, table: &[[i32; 16]], d: &[i16], n: usize, x: usize, nz_h: usize, k: impl Fn(usize) -> usize) {
    for (p, t) in table.iter().enumerate() {
        let ka = k(2 * p);
        if ka >= nz_h {
            break;
        }
        let kb = k(2 * p + 1);
        let (lo, hi) = zip_rows(d, n, ka, (kb < nz_h).then_some(kb), x);
        for j in 0..len {
            let tv = i32x4_splat(t[j]);
            acc[j][0] = i32x4_add(acc[j][0], i32x4_dot_i16x8(lo, tv));
            acc[j][1] = i32x4_add(acc[j][1], i32x4_dot_i16x8(hi, tv));
        }
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

// ---- sample adaptive offset ----

/// `v + off`, `off` a signed offset of at most ±7, clipped to the sample range.
#[inline(always)]
fn add_offset(v: v128, off: v128) -> v128 {
    let zero = i8x16_splat(0);
    let pos = i8x16_max(off, zero);
    let neg = i8x16_max(i8x16_sub(zero, off), zero);
    u8x16_sub_sat(u8x16_add_sat(v, pos), neg)
}

pub fn sao_band(dst: &mut [u8], src: &[u8], stride: usize, x0: usize, y0: usize, w: usize, h: usize, band: &[i8; 32]) {
    let lo = i8x16(band[0], band[1], band[2], band[3], band[4], band[5], band[6], band[7], band[8], band[9], band[10], band[11], band[12], band[13], band[14], band[15]);
    let hi = i8x16(band[16], band[17], band[18], band[19], band[20], band[21], band[22], band[23], band[24], band[25], band[26], band[27], band[28], band[29], band[30], band[31]);
    let sixteen = u8x16_splat(16);
    for y in y0..y0 + h {
        let (d, s) = (&mut dst[y * stride + x0..y * stride + x0 + w], &src[y * stride + x0..y * stride + x0 + w]);
        let mut x = 0;
        while x + 16 <= w {
            let v = load_u8x16(&s[x..]);
            let idx = u8x16_shr(v, 3);
            // A swizzle index past 15 reads as 0, so the two halves sum.
            let off = v128_or(u8x16_swizzle(lo, idx), u8x16_swizzle(hi, u8x16_sub(idx, sixteen)));
            store_u8x16(&mut d[x..], add_offset(v, off));
            x += 16;
        }
        for x in x..w {
            d[x] = (s[x] as i32 + band[(s[x] >> 3) as usize] as i32).clamp(0, 255) as u8;
        }
    }
}

pub fn sao_edge(dst: &mut [u8], src: &[u8], stride: usize, x0: usize, y0: usize, w: usize, h: usize, da: (i32, i32), db: (i32, i32), offs: &[i8; 4]) {
    let table = [offs[0], offs[1], 0, offs[2], offs[3]];
    let tab = i8x16(table[0], table[1], 0, table[3], table[4], 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
    let two = i8x16_splat(2);
    let oa = da.1 as isize * stride as isize + da.0 as isize;
    let ob = db.1 as isize * stride as isize + db.0 as isize;
    for y in y0..y0 + h {
        let mut x = x0;
        while x + 16 <= x0 + w {
            let i = y * stride + x;
            let v = load_u8x16(&src[i..]);
            let a = load_u8x16(&src[(i as isize + oa) as usize..]);
            let b = load_u8x16(&src[(i as isize + ob) as usize..]);
            // sign(v - a) is (v < a) - (v > a) with the masks being -1.
            let sa = i8x16_sub(u8x16_lt(v, a), u8x16_gt(v, a));
            let sb = i8x16_sub(u8x16_lt(v, b), u8x16_gt(v, b));
            let e = i8x16_add(two, i8x16_add(sa, sb));
            store_u8x16(&mut dst[i..], add_offset(v, u8x16_swizzle(tab, e)));
            x += 16;
        }
        for x in x..x0 + w {
            let i = y * stride + x;
            let v = src[i] as i32;
            let a = src[(i as isize + oa) as usize] as i32;
            let b = src[(i as isize + ob) as usize] as i32;
            let e = (2 + (v - a).signum() + (v - b).signum()) as usize;
            dst[i] = (v + table[e] as i32).clamp(0, 255) as u8;
        }
    }
}
