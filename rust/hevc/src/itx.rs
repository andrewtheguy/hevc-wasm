//! Scaling (§8.6.3) and the inverse transforms (§8.6.4) of a transform block,
//! over the rectangle of coefficients the parser found non-zero.

use crate::kernels;
use crate::tables::{DCT32, DST4, LEVEL_SCALE};

const COEFF_MIN: i32 = -32768;
const COEFF_MAX: i32 = 32767;

/// `TransCoeffLevel` to scaled coefficients, in place, flat scaling.
pub fn dequant(coeffs: &mut [i32], n: usize, nz_w: usize, nz_h: usize, qp: i32) {
    let log2n = n.trailing_zeros() as i32;
    let bd_shift = 8 + log2n - 5;
    let scale = LEVEL_SCALE[(qp % 6) as usize] << (qp / 6);
    let add = 1i64 << (bd_shift - 1);
    for y in 0..nz_h {
        for c in &mut coeffs[y * n..y * n + nz_w] {
            if *c != 0 {
                *c = (((*c as i64 * 16 * scale as i64) + add) >> bd_shift).clamp(COEFF_MIN as i64, COEFF_MAX as i64) as i32;
            }
        }
    }
}

/// The partial sums of an `n`-point inverse DCT by partial butterfly: the
/// even half is the `n/2`-point transform of the even coefficients, down to
/// a 4-point base, and each level adds its odd half.
fn idct_sums(src: &[i32], s_in: usize, n: usize, nz: usize, out: &mut [i32]) {
    let levels = n.trailing_zeros() as usize - 2;
    let mut nzs = [0usize; 4];
    let (mut z, mut m) = (nz.min(n), n);
    for slot in nzs.iter_mut().take(levels + 1) {
        *slot = z;
        if m > 4 {
            m /= 2;
            z = z.div_ceil(2).min(m);
        }
    }
    let tab = DCT32.as_flattened();
    kernels::accum(&mut out[..4], src, s_in << levels, tab, 8, 0, 1, nzs[levels], 4);
    for d in (0..levels).rev() {
        let m = n >> d;
        kernels::accum_butterfly(&mut out[..m], src, s_in << d, tab, 32 >> m.trailing_zeros(), nzs[d], m);
    }
}

/// One column (`CLIP`, into the clipped intermediate) or row of the transform.
fn idct_1d<const CLIP: bool, T: Copy>(src: &[i32], s_in: usize, dst: &mut [T], s_out: usize, n: usize, nz: usize, shift: u32, dst4: bool, clip_to: impl Fn(i32) -> T) {
    let mut sums = [0i32; 32];
    if dst4 {
        kernels::accum(&mut sums[..4], src, s_in, DST4.as_flattened(), 1, 0, 1, nz.min(4), 4);
    } else {
        idct_sums(src, s_in, n, nz, &mut sums[..n]);
    }
    let add = 1i32 << (shift - 1);
    for i in 0..n {
        let v = (sums[i] + add) >> shift;
        dst[i * s_out] = clip_to(if CLIP { v.clamp(COEFF_MIN, COEFF_MAX) } else { v });
    }
}

/// Scaled coefficients `d` (raster, `n`×`n`, non-zero within `nz_w`×`nz_h`)
/// to the residual `res`. `dst` selects the 4×4 DST of intra luma. `tmp` is
/// the first stage's intermediate.
pub fn inverse_transform(d: &[i32], tmp: &mut [i32], res: &mut [i16], n: usize, nz_w: usize, nz_h: usize, dst: bool) {
    let nn = n * n;
    if nz_w <= 1 && nz_h <= 1 && !dst {
        // A lone DC coefficient: row 0 of the matrix is the constant 64, so
        // both stages are one value.
        let v1 = (((d[0] as i64 * 64 + 64) >> 7) as i32).clamp(COEFF_MIN, COEFF_MAX);
        let out = ((v1 as i64 * 64 + (1 << 11)) >> 12) as i16;
        res[..nn].fill(out);
        return;
    }
    let (nz_w, nz_h) = (nz_w.clamp(1, n), nz_h.clamp(1, n));
    for x in 0..nz_w {
        idct_1d::<true, i32>(&d[x..], n, &mut tmp[x..], n, n, nz_h, 7, dst, |v| v);
    }
    for y in 0..n {
        idct_1d::<false, i16>(&tmp[y * n..], 1, &mut res[y * n..], 1, n, nz_w, 12, dst, |v| v as i16);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive(d: &[i32], n: usize, dst: bool) -> Vec<i16> {
        let step = 32 / n;
        let t = |k: usize, j: usize| -> i32 { if dst { DST4[k][j] as i32 } else { DCT32[k * step][j] as i32 } };
        let mut tmp = vec![0i32; n * n];
        for x in 0..n {
            for j in 0..n {
                let mut s = 0i32;
                for k in 0..n {
                    s += d[k * n + x] * t(k, j);
                }
                tmp[j * n + x] = ((s + 64) >> 7).clamp(COEFF_MIN, COEFF_MAX);
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

    #[test]
    fn butterfly_and_sparse_bounds_match_the_naive_transform() {
        let mut st = 0x1234_5678u32;
        let mut rnd = || {
            st = st.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((st >> 16) as i32 & 0x1ff) - 256
        };
        let mut tmp = vec![0i32; 32 * 32];
        let mut res = vec![0i16; 32 * 32];
        for &n in &[4usize, 8, 16, 32] {
            for &(nz_w, nz_h) in &[(1usize, 1usize), (2, 3), (n, n), (n / 2, 1)] {
                let mut d = vec![0i32; n * n];
                for y in 0..nz_h {
                    for x in 0..nz_w {
                        d[y * n + x] = rnd();
                    }
                }
                for dst in [false, true] {
                    if dst && n != 4 {
                        continue;
                    }
                    inverse_transform(&d, &mut tmp, &mut res, n, nz_w, nz_h, dst);
                    assert_eq!(&res[..n * n], &naive(&d, n, dst)[..], "n={n} nz=({nz_w},{nz_h}) dst={dst}");
                }
            }
        }
    }

    #[test]
    fn dequant_flat() {
        let mut c = vec![0i32; 16];
        c[0] = 10;
        dequant(&mut c, 4, 4, 4, 4);
        assert_eq!(c[0], 320);
    }
}
