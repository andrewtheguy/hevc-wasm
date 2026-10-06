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

// ---- inverse transform ----

/// `LEN` is 4, 8 or 16: one, two or four lanes of 32-bit sums.
pub fn accum<const LEN: usize>(out: &mut [i32; LEN], src: &[i16], s_in: usize, k0: usize, kstep: usize, nz: usize, tab: &[[i16; 32]], tstep: usize) {
    let mut acc = [i32x4_splat(0); 4];
    let mut k = k0;
    while k < nz {
        let c = src[k * s_in];
        if c != 0 {
            let cv = i16x8_splat(c);
            let row = &tab[k * tstep][..LEN];
            if LEN == 4 {
                acc[0] = i32x4_add(acc[0], i32x4_extmul_low_i16x8(cv, load_i16x4(row)));
            } else {
                for p in 0..LEN / 8 {
                    let v = load_i16x8(&row[8 * p..]);
                    acc[2 * p] = i32x4_add(acc[2 * p], i32x4_extmul_low_i16x8(cv, v));
                    acc[2 * p + 1] = i32x4_add(acc[2 * p + 1], i32x4_extmul_high_i16x8(cv, v));
                }
            }
        }
        k += kstep;
    }
    for j in 0..LEN / 4 {
        store_i32x4(&mut out[4 * j..], acc[j]);
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
