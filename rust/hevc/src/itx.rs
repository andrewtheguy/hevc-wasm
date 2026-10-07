//! Scaling (§8.6.3) and the inverse transforms (§8.6.4) of a transform block,
//! in 16 bits: the scaled coefficients, the intermediate between the two
//! passes and the residual are all within 16 bits for 8-bit samples, and the
//! sums of a pass are 32-bit, which is what the kernels multiply into.
//!
//! Each pass is a partial butterfly: the even half of an N-point transform is
//! the N/2-point transform of the even coefficients, down to the 4-point base,
//! and each level adds its odd half, a sum over the odd coefficients of the
//! block that were non-zero.

use crate::kernels;
use crate::tables::{DCT32, DST4, LEVEL_SCALE};

/// Flat scaling (§8.6.3) of a `TransCoeffLevel` of an `n`×`n` block at `qp`,
/// as each is parsed. In 32 bits: `level * levelScale * 16` fits, and the
/// `<< (qp / 6)` and `>> bdShift` fold into one shift either way.
#[derive(Clone, Copy)]
pub struct Dequant {
    scale: i32,
    shl: u32,
    shr: u32,
    round: i32,
}

impl Dequant {
    pub fn new(n: usize, qp: i32) -> Self {
        let log2n = n.trailing_zeros() as i32;
        let r = 8 + log2n - 5;
        let l = qp / 6;
        Dequant { scale: LEVEL_SCALE[(qp % 6) as usize] * 16, shl: (l - r).max(0) as u32, shr: (r - l).max(0) as u32, round: if l < r { 1 << (r - l - 1) } else { 0 } }
    }

    #[inline(always)]
    pub fn apply(&self, level: i32) -> i16 {
        ((((level * self.scale) << self.shl) + self.round) >> self.shr).clamp(-32768, 32767) as i16
    }
}

/// Where a block's non-zero coefficients are: a bit per coded column and
/// row, and how many there are.
#[derive(Clone, Copy, Default)]
pub struct Coded {
    pub cols: u32,
    pub rows: u32,
    pub count: u32,
}

impl Coded {
    /// The columns and rows up to the last coded one.
    pub fn extent(&self) -> (usize, usize) {
        (32 - self.cols.leading_zeros() as usize, 32 - self.rows.leading_zeros() as usize)
    }
}

#[inline(always)]
fn butterfly<const H: usize>(out: &mut [i32], even: &[i32; H], odd: &[i32; H]) {
    for j in 0..H {
        out[j] = even[j] + odd[j];
        out[2 * H - 1 - j] = even[j] - odd[j];
    }
}

/// The sums of the 4-point inverse DCT of `src` (`s_in` apart, `nz` of them
/// possibly non-zero).
#[inline(always)]
fn idct4(src: &[i16], s_in: usize, nz: usize, out: &mut [i32; 4]) {
    kernels::accum::<4>(out, src, s_in, 0, 1, nz, &DCT32, 8);
}

#[inline(always)]
fn idct8(src: &[i16], s_in: usize, nz: usize, out: &mut [i32; 8]) {
    let (mut even, mut odd) = ([0i32; 4], [0i32; 4]);
    idct4(src, 2 * s_in, nz.div_ceil(2), &mut even);
    kernels::accum::<4>(&mut odd, src, s_in, 1, 2, nz, &DCT32, 4);
    butterfly(out, &even, &odd);
}

#[inline(always)]
fn idct16(src: &[i16], s_in: usize, nz: usize, out: &mut [i32; 16]) {
    let (mut even, mut odd) = ([0i32; 8], [0i32; 8]);
    idct8(src, 2 * s_in, nz.div_ceil(2), &mut even);
    kernels::accum::<8>(&mut odd, src, s_in, 1, 2, nz, &DCT32, 2);
    butterfly(out, &even, &odd);
}

#[inline(always)]
fn idct32(src: &[i16], s_in: usize, nz: usize, out: &mut [i32; 32]) {
    let (mut even, mut odd) = ([0i32; 16], [0i32; 16]);
    idct16(src, 2 * s_in, nz.div_ceil(2), &mut even);
    kernels::accum::<16>(&mut odd, src, s_in, 1, 2, nz, &DCT32, 1);
    butterfly(out, &even, &odd);
}

/// The 4-point inverse DST of intra luma 4×4 blocks.
#[inline(always)]
fn idst4(src: &[i16], s_in: usize, nz: usize, out: &mut [i32; 4]) {
    kernels::accum::<4>(out, src, s_in, 0, 1, nz, &DST4, 1);
}

/// Both passes over an `N`×`N` block: columns into `tmp`, clipped to 16 bits,
/// then rows into `res`.
#[inline(always)]
fn block<const N: usize>(d: &[i16], tmp: &mut [i16], res: &mut [i16], nz_w: usize, nz_h: usize, idct: impl Fn(&[i16], usize, usize, &mut [i32; N])) {
    let mut sums = [0i32; N];
    for x in 0..nz_w {
        idct(&d[x..], N, nz_h, &mut sums);
        for j in 0..N {
            tmp[j * N + x] = ((sums[j] + 64) >> 7).clamp(-32768, 32767) as i16;
        }
    }
    for y in 0..N {
        idct(&tmp[y * N..], 1, nz_w, &mut sums);
        for (r, &s) in res[y * N..y * N + N].iter_mut().zip(&sums) {
            *r = ((s + 2048) >> 12) as i16;
        }
    }
}

/// Scaled coefficients `d` (raster, `n`×`n`, non-zero where `coded` says)
/// to the residual `res`. `dst` selects the 4×4 DST of intra luma. `tmp` is
/// the first pass's intermediate.
pub fn inverse_transform(d: &[i16], tmp: &mut [i16], res: &mut [i16], n: usize, coded: Coded, dst: bool) {
    if coded.cols <= 1 && coded.rows <= 1 && !dst {
        // A lone DC coefficient: row 0 of the matrix is the constant 64, so
        // both passes are one value.
        let v1 = ((d[0] as i32 * 64 + 64) >> 7).clamp(-32768, 32767);
        let out = ((v1 * 64 + (1 << 11)) >> 12) as i16;
        crate::kernels::fill_i16(&mut res[..n * n], out);
        return;
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return crate::kernels::simd128::inverse_transform(d, tmp, res, n, coded, dst);
    }
    #[allow(unreachable_code)]
    let (nz_w, nz_h) = coded.extent();
    let (nz_w, nz_h) = (nz_w.clamp(1, n), nz_h.clamp(1, n));
    match (n, dst) {
        (4, true) => block::<4>(d, tmp, res, nz_w, nz_h, idst4),
        (4, false) => block::<4>(d, tmp, res, nz_w, nz_h, idct4),
        (8, _) => block::<8>(d, tmp, res, nz_w, nz_h, idct8),
        (16, _) => block::<16>(d, tmp, res, nz_w, nz_h, idct16),
        _ => block::<32>(d, tmp, res, nz_w, nz_h, idct32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive(d: &[i16], n: usize, dst: bool) -> Vec<i16> {
        let step = 32 / n;
        let t = |k: usize, j: usize| -> i32 { if dst { DST4[k][j] as i32 } else { DCT32[k * step][j] as i32 } };
        let mut tmp = vec![0i32; n * n];
        for x in 0..n {
            for j in 0..n {
                let mut s = 0i32;
                for k in 0..n {
                    s += d[k * n + x] as i32 * t(k, j);
                }
                tmp[j * n + x] = ((s + 64) >> 7).clamp(-32768, 32767);
            }
        }
        let mut out = vec![0i16; n * n];
        for y in 0..n {
            for j in 0..n {
                let mut s = 0i32;
                for k in 0..n {
                    s += tmp[y * n + k] * t(k, j);
                }
                out[y * n + j] = ((s + 2048) >> 12) as i16;
            }
        }
        out
    }

    fn coded(d: &[i16], n: usize) -> Coded {
        let mut c = Coded::default();
        for (i, &v) in d.iter().enumerate() {
            if v != 0 {
                c.cols |= 1 << (i % n);
                c.rows |= 1 << (i / n);
                c.count += 1;
            }
        }
        c
    }

    #[test]
    fn butterflies_and_sparse_bounds_match_the_naive_transform() {
        let mut st = 0x1234_5678u32;
        let mut rnd = || {
            st = st.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((st >> 16) as i32 & 0x1ff) as i16 - 256
        };
        let mut tmp = vec![0i16; 32 * 32];
        let mut res = vec![0i16; 32 * 32];
        for &n in &[4usize, 8, 16, 32] {
            for &(nz_w, nz_h) in &[(1usize, 1usize), (2, 3), (n, n), (n / 2, 1), (3, n)] {
                let mut d = vec![0i16; n * n];
                for y in 0..nz_h {
                    for x in 0..nz_w {
                        d[y * n + x] = rnd();
                    }
                }
                for dst in [false, true] {
                    if dst && n != 4 {
                        continue;
                    }
                    inverse_transform(&d, &mut tmp, &mut res, n, coded(&d, n), dst);
                    assert_eq!(&res[..n * n], &naive(&d, n, dst)[..], "n={n} nz=({nz_w},{nz_h}) dst={dst}");
                }
            }
            // A few coefficients scattered over the block, as the capture's
            // are: the extent is most of the block, the coded columns and
            // rows a handful.
            for count in [1usize, 2, 7, 13] {
                let mut d = vec![0i16; n * n];
                for i in 0..count {
                    let (x, y) = ((i * 7 + 3) % n, (i * 5 + 1) % n);
                    d[y * n + x] = rnd();
                }
                inverse_transform(&d, &mut tmp, &mut res, n, coded(&d, n), false);
                assert_eq!(&res[..n * n], &naive(&d, n, false)[..], "n={n} scattered {count}");
            }
        }
    }

    #[test]
    fn dequant_flat() {
        assert_eq!(Dequant::new(4, 4).apply(10), 320);
        // QP 51 on a 4×4: the shift goes the other way.
        assert_eq!(Dequant::new(4, 51).apply(3), (((3i64 * 57 * 16) << 8) >> 5) as i16);
    }
}

/// The stages of the partial butterfly, as pairs of coefficients, in the shapes
/// the vector kernels multiply: the
/// four-point base (coefficients 0, 8, 16 and 24 of a 32-point transform) and
/// the odd halves of the 8-, 16- and 32-point transforms. A pair multiplies
/// two rows of `DCT32`, and a stage of a smaller block reads its coefficients
/// at the proportionally smaller indices.
#[cfg_attr(not(all(target_arch = "wasm32", target_feature = "simd128")), allow(dead_code))]
pub mod stages {
    use super::DCT32;

    /// `(row of DCT32, coefficient index in a 32-point transform)` of each
    /// coefficient of each stage, pairs in order.
    pub const K32: [&[usize]; 4] = [&[0, 8, 16, 24], &[4, 12, 20, 28], &[2, 6, 10, 14, 18, 22, 26, 30], &[1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25, 27, 29, 31]];

    /// Each pair's rows as `(row a, row b)` packed into one 32-bit lane per
    /// output, for a pass that runs across columns: `[stage][pair][output]`.
    pub static PACKED: [[[i32; 16]; 8]; 4] = {
        let mut t = [[[0i32; 16]; 8]; 4];
        let mut s = 0;
        while s < 4 {
            let ks = K32[s];
            let len = ks.len();
            let mut p = 0;
            while p < len / 2 {
                let mut j = 0;
                while j < len {
                    t[s][p][j] = (DCT32[ks[2 * p]][j] as u16 as i32) | ((DCT32[ks[2 * p + 1]][j] as u16 as i32) << 16);
                    j += 1;
                }
                p += 1;
            }
            s += 1;
        }
        t
    };

    /// Each pair's rows interleaved, four outputs per vector, for a pass that
    /// runs along a row: `[stage][pair][output / 4][2 * (output % 4) + which]`.
    pub static INTERLEAVED: [[[[i16; 8]; 4]; 8]; 4] = {
        let mut t = [[[[0i16; 8]; 4]; 8]; 4];
        let mut s = 0;
        while s < 4 {
            let ks = K32[s];
            let len = ks.len();
            let mut p = 0;
            while p < len / 2 {
                let mut j = 0;
                while j < len {
                    t[s][p][j / 4][2 * (j % 4)] = DCT32[ks[2 * p]][j];
                    t[s][p][j / 4][2 * (j % 4) + 1] = DCT32[ks[2 * p + 1]][j];
                    j += 1;
                }
                p += 1;
            }
            s += 1;
        }
        t
    };

    /// The 4-point DST in the same two shapes: pairs (0, 1) and (2, 3).
    pub static DST_PACKED: [[i32; 16]; 2] = {
        let mut t = [[0i32; 16]; 2];
        let mut p = 0;
        while p < 2 {
            let mut j = 0;
            while j < 4 {
                t[p][j] = (super::DST4[2 * p][j] as u16 as i32) | ((super::DST4[2 * p + 1][j] as u16 as i32) << 16);
                j += 1;
            }
            p += 1;
        }
        t
    };
    pub static DST_INTERLEAVED: [[[i16; 8]; 4]; 2] = {
        let mut t = [[[0i16; 8]; 4]; 2];
        let mut p = 0;
        while p < 2 {
            let mut j = 0;
            while j < 4 {
                t[p][0][2 * j] = super::DST4[2 * p][j];
                t[p][0][2 * j + 1] = super::DST4[2 * p + 1][j];
                j += 1;
            }
            p += 1;
        }
        t
    };
}
