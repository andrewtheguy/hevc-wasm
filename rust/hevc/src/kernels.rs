//! The sample loops: what runs per sample rather than per block. Plain Rust
//! here; `simd128` holds the same loops in WebAssembly vectors, and each
//! public function dispatches to it where it is built in.

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
pub mod simd128;

use crate::tables::{BETA_TABLE, TC_TABLE};

// ---- motion compensation (§8.5.3.3.3) ----

/// `dst = src`, `w`×`h`, each at its stride.
pub fn copy_block(dst: &mut [u8], dst_stride: usize, src: &[u8], src_stride: usize, w: usize, h: usize) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return simd128::copy_block(dst, dst_stride, src, src_stride, w, h);
    }
    #[allow(unreachable_code)]
    for y in 0..h {
        dst[y * dst_stride..y * dst_stride + w].copy_from_slice(&src[y * src_stride..y * src_stride + w]);
    }
}

/// Horizontal `N`-tap filter of samples into the 14-bit intermediate:
/// `dst[y][x] = Σ t[i] * src[y][x + i]`, the first pass of a two-dimensional
/// interpolation.
pub fn fir_h<const N: usize>(src: &[u8], stride: usize, t: &[i16; N], w: usize, h: usize, dst: &mut [i16], dst_stride: usize) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return simd128::fir_h::<N>(src, stride, t, w, h, dst, dst_stride);
    }
    #[allow(unreachable_code)]
    for y in 0..h {
        let row = &src[y * stride..y * stride + w + N - 1];
        let out = &mut dst[y * dst_stride..y * dst_stride + w];
        for (x, o) in out.iter_mut().enumerate() {
            let mut acc = 0i32;
            for i in 0..N {
                acc += t[i] as i32 * row[x + i] as i32;
            }
            *o = acc as i16;
        }
    }
}

/// The intermediate as a sample (§8.5.3.3.4.2, uni-prediction).
#[inline(always)]
fn uni(v: i32) -> u8 {
    ((v + 32) >> 6).clamp(0, 255) as u8
}

/// Horizontal `N`-tap filter straight to samples: the one pass of a
/// horizontal interpolation, with its rounding.
pub fn fir_h_uni<const N: usize>(src: &[u8], stride: usize, t: &[i16; N], w: usize, h: usize, dst: &mut [u8], dst_stride: usize) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return simd128::fir_h_uni::<N>(src, stride, t, w, h, dst, dst_stride);
    }
    #[allow(unreachable_code)]
    for y in 0..h {
        let row = &src[y * stride..y * stride + w + N - 1];
        let out = &mut dst[y * dst_stride..y * dst_stride + w];
        for (x, o) in out.iter_mut().enumerate() {
            let mut acc = 0i32;
            for i in 0..N {
                acc += t[i] as i32 * row[x + i] as i32;
            }
            *o = uni(acc);
        }
    }
}

/// Vertical `N`-tap filter of samples straight to samples.
pub fn fir_v_uni<const N: usize>(src: &[u8], stride: usize, t: &[i16; N], w: usize, h: usize, dst: &mut [u8], dst_stride: usize) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return simd128::fir_v_uni::<N>(src, stride, t, w, h, dst, dst_stride);
    }
    #[allow(unreachable_code)]
    for y in 0..h {
        let out = &mut dst[y * dst_stride..y * dst_stride + w];
        for (x, o) in out.iter_mut().enumerate() {
            let mut acc = 0i32;
            for i in 0..N {
                acc += t[i] as i32 * src[(y + i) * stride + x] as i32;
            }
            *o = uni(acc);
        }
    }
}

/// Vertical `N`-tap filter of the intermediate, `>> 6`, straight to samples:
/// the second pass of a two-dimensional interpolation.
pub fn fir_hv_uni<const N: usize>(src: &[i16], stride: usize, t: &[i16; N], w: usize, h: usize, dst: &mut [u8], dst_stride: usize) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return simd128::fir_hv_uni::<N>(src, stride, t, w, h, dst, dst_stride);
    }
    #[allow(unreachable_code)]
    for y in 0..h {
        let out = &mut dst[y * dst_stride..y * dst_stride + w];
        for (x, o) in out.iter_mut().enumerate() {
            let mut acc = 0i32;
            for i in 0..N {
                acc += t[i] as i32 * src[(y + i) * stride + x] as i32;
            }
            *o = uni(acc >> 6);
        }
    }
}

// ---- reconstruction ----

/// `dst += res`, clipped to the sample range.
pub fn add_residual(dst: &mut [u8], dst_stride: usize, res: &[i16], w: usize, h: usize) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return simd128::add_residual(dst, dst_stride, res, w, h);
    }
    #[allow(unreachable_code)]
    for y in 0..h {
        let row = &mut dst[y * dst_stride..y * dst_stride + w];
        for (d, &r) in row.iter_mut().zip(&res[y * w..y * w + w]) {
            *d = (*d as i32 + r as i32).clamp(0, 255) as u8;
        }
    }
}

/// `dst = v`, for the lengths a transform block has: at least four, a multiple
/// of four.
pub fn fill_i16(dst: &mut [i16], v: i16) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return simd128::fill_i16(dst, v);
    }
    #[allow(unreachable_code)]
    dst.fill(v);
}

// ---- intra prediction (§8.4.4.2.5, §8.4.4.2.6) ----

/// Planar prediction of an `n`×`n` block from `n + 1` left and top neighbours.
pub fn planar(dst: &mut [u8], stride: usize, n: usize, left: &[u8], top: &[u8]) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return simd128::planar(dst, stride, n, left, top);
    }
    #[allow(unreachable_code)]
    let log2n = n.trailing_zeros();
    let (tn, ln) = (top[n] as i32, left[n] as i32);
    for y in 0..n {
        let l = left[y] as i32;
        let k = (n - 1 - y) as i32;
        let c = (y as i32 + 1) * ln + n as i32;
        let row = &mut dst[y * stride..y * stride + n];
        for (x, d) in row.iter_mut().enumerate() {
            *d = (((n - 1 - x) as i32 * l + (x as i32 + 1) * tn + k * top[x] as i32 + c) >> (log2n + 1)) as u8;
        }
    }
}

pub fn fill(dst: &mut [u8], stride: usize, n: usize, v: u8) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return simd128::fill(dst, stride, n, v);
    }
    #[allow(unreachable_code)]
    for y in 0..n {
        dst[y * stride..y * stride + n].fill(v);
    }
}

/// Angular prediction along rows: `refb[off + 1 + ..]` is the projected top
/// reference, and row `y` reads it at `(y + 1) * angle / 32`.
pub fn angular(dst: &mut [u8], stride: usize, n: usize, refb: &[i16], off: usize, angle: i32) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return simd128::angular(dst, stride, n, refb, off, angle);
    }
    #[allow(unreachable_code)]
    for y in 0..n {
        let pos = (y as i32 + 1) * angle;
        let (iidx, ifact) = (pos >> 5, pos & 31);
        let base = (off as i32 + iidx + 1) as usize;
        let row = &mut dst[y * stride..y * stride + n];
        if ifact == 0 {
            for (x, d) in row.iter_mut().enumerate() {
                *d = refb[base + x] as u8;
            }
        } else {
            for (x, d) in row.iter_mut().enumerate() {
                let (a, b) = (refb[base + x] as i32, refb[base + x + 1] as i32);
                *d = (((32 - ifact) * a + ifact * b + 16) >> 5) as u8;
            }
        }
    }
}

/// [`angular`] along columns: the projected left reference, column `x` read at
/// `(x + 1) * angle / 32`.
pub fn angular_t(dst: &mut [u8], stride: usize, n: usize, refb: &[i16], off: usize, angle: i32) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return simd128::angular_t(dst, stride, n, refb, off, angle);
    }
    #[allow(unreachable_code)]
    for x in 0..n {
        let pos = (x as i32 + 1) * angle;
        let (iidx, ifact) = (pos >> 5, pos & 31);
        let base = (off as i32 + iidx + 1) as usize;
        for y in 0..n {
            let a = refb[base + y] as i32;
            let v = if ifact == 0 {
                a
            } else {
                let b = refb[base + y + 1] as i32;
                ((32 - ifact) * a + ifact * b + 16) >> 5
            };
            dst[y * stride + x] = v as u8;
        }
    }
}

// ---- inverse transform (§8.6.4.2) ----

/// `out[j] = Σ src[k * s_in] * tab[k * tstep][j]` over `k = k0, k0 + kstep, ..
/// < nz`, for `LEN` outputs.
pub fn accum<const LEN: usize>(out: &mut [i32; LEN], src: &[i16], s_in: usize, k0: usize, kstep: usize, nz: usize, tab: &[[i16; 32]], tstep: usize) {
    {
        *out = [0; LEN];
        let mut k = k0;
        while k < nz {
            let c = src[k * s_in] as i32;
            if c != 0 {
                let row = &tab[k * tstep][..LEN];
                for (o, &t) in out.iter_mut().zip(row) {
                    *o += c * t as i32;
                }
            }
            k += kstep;
        }
    }
}

// ---- deblocking (§8.7.2.5) ----

/// One four-line luma edge segment at (`x`, `y`): a vertical edge (`dir` 0)
/// between columns `x - 1` and `x`, or a horizontal one between rows.
pub fn luma_edge(data: &mut [u8], stride: usize, x: usize, y: usize, dir: usize, beta: i32, tc: i32) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return simd128::luma_edge(data, stride, x, y, dir, beta, tc);
    }
    #[allow(unreachable_code)]
    let (line_step, tap_step): (isize, isize) = if dir == 0 { (stride as isize, 1) } else { (1, stride as isize) };
    let origin = (y * stride + x) as isize;
    let idx = |k: usize, i: i32| -> usize { (origin + k as isize * line_step + i as isize * tap_step) as usize };
    let g = |d: &[u8], k: usize, i: i32| d[idx(k, i)] as i32;
    let dp0 = (g(data, 0, -3) - 2 * g(data, 0, -2) + g(data, 0, -1)).abs();
    let dp3 = (g(data, 3, -3) - 2 * g(data, 3, -2) + g(data, 3, -1)).abs();
    let dq0 = (g(data, 0, 2) - 2 * g(data, 0, 1) + g(data, 0, 0)).abs();
    let dq3 = (g(data, 3, 2) - 2 * g(data, 3, 1) + g(data, 3, 0)).abs();
    let (dpq0, dpq3) = (dp0 + dq0, dp3 + dq3);
    if dpq0 + dpq3 >= beta {
        return;
    }
    let dsam = |k: usize, dpq: i32| -> bool { dpq < (beta >> 2) && (g(data, k, -4) - g(data, k, -1)).abs() + (g(data, k, 0) - g(data, k, 3)).abs() < (beta >> 3) && (g(data, k, -1) - g(data, k, 0)).abs() < ((5 * tc + 1) >> 1) };
    let strong = dsam(0, 2 * dpq0) && dsam(3, 2 * dpq3);
    let dep = dp0 + dp3 < ((beta + (beta >> 1)) >> 3);
    let deq = dq0 + dq3 < ((beta + (beta >> 1)) >> 3);
    for k in 0..4 {
        let (p3, p2, p1, p0) = (g(data, k, -4), g(data, k, -3), g(data, k, -2), g(data, k, -1));
        let (q0, q1, q2, q3) = (g(data, k, 0), g(data, k, 1), g(data, k, 2), g(data, k, 3));
        if strong {
            let t2 = 2 * tc;
            data[idx(k, -1)] = ((p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3).clamp(p0 - t2, p0 + t2) as u8;
            data[idx(k, -2)] = ((p2 + p1 + p0 + q0 + 2) >> 2).clamp(p1 - t2, p1 + t2) as u8;
            data[idx(k, -3)] = ((2 * p3 + 3 * p2 + p1 + p0 + q0 + 4) >> 3).clamp(p2 - t2, p2 + t2) as u8;
            data[idx(k, 0)] = ((p1 + 2 * p0 + 2 * q0 + 2 * q1 + q2 + 4) >> 3).clamp(q0 - t2, q0 + t2) as u8;
            data[idx(k, 1)] = ((p0 + q0 + q1 + q2 + 2) >> 2).clamp(q1 - t2, q1 + t2) as u8;
            data[idx(k, 2)] = ((p0 + q0 + q1 + 3 * q2 + 2 * q3 + 4) >> 3).clamp(q2 - t2, q2 + t2) as u8;
        } else {
            let mut delta = (9 * (q0 - p0) - 3 * (q1 - p1) + 8) >> 4;
            if delta.abs() < tc * 10 {
                delta = delta.clamp(-tc, tc);
                data[idx(k, -1)] = (p0 + delta).clamp(0, 255) as u8;
                data[idx(k, 0)] = (q0 - delta).clamp(0, 255) as u8;
                if dep {
                    let d = ((((p2 + p0 + 1) >> 1) - p1 + delta) >> 1).clamp(-(tc >> 1), tc >> 1);
                    data[idx(k, -2)] = (p1 + d).clamp(0, 255) as u8;
                }
                if deq {
                    let d = ((((q2 + q0 + 1) >> 1) - q1 - delta) >> 1).clamp(-(tc >> 1), tc >> 1);
                    data[idx(k, 1)] = (q1 + d).clamp(0, 255) as u8;
                }
            }
        }
    }
}

/// One four-line chroma edge segment (§8.7.2.5.5): one sample each side.
pub fn chroma_edge(data: &mut [u8], stride: usize, x: usize, y: usize, dir: usize, tc: i32) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return simd128::chroma_edge(data, stride, x, y, dir, tc);
    }
    #[allow(unreachable_code)]
    let (line_step, tap_step): (isize, isize) = if dir == 0 { (stride as isize, 1) } else { (1, stride as isize) };
    let origin = (y * stride + x) as isize;
    for k in 0..4isize {
        let at = |i: isize| (origin + k * line_step + i * tap_step) as usize;
        let (p1, p0, q0, q1) = (data[at(-2)] as i32, data[at(-1)] as i32, data[at(0)] as i32, data[at(1)] as i32);
        let d = ((((q0 - p0) << 2) + p1 - q1 + 4) >> 3).clamp(-tc, tc);
        data[at(-1)] = (p0 + d).clamp(0, 255) as u8;
        data[at(0)] = (q0 - d).clamp(0, 255) as u8;
    }
}

/// `tC` for a luma edge of strength `bs` at QP `qp` (§8.7.2.5.3).
pub fn luma_tc(qp: i32, bs: i32, tc_offset_div2: i32) -> i32 {
    TC_TABLE[(qp + 2 * (bs - 1) + (tc_offset_div2 << 1)).clamp(0, 53) as usize] as i32
}

pub fn luma_beta(qp: i32, beta_offset_div2: i32) -> i32 {
    BETA_TABLE[(qp + (beta_offset_div2 << 1)).clamp(0, 51) as usize] as i32
}

// ---- sample adaptive offset (§8.7.3) ----

/// Band offset in place over a `w`×`h` block: `v += band[v >> 3]`.
pub fn sao_band(data: &mut [u8], stride: usize, w: usize, h: usize, band: &[i8; 32]) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return simd128::sao_band(data, stride, w, h, band);
    }
    #[allow(unreachable_code)]
    for y in 0..h {
        for v in &mut data[y * stride..y * stride + w] {
            *v = (*v as i32 + band[(*v >> 3) as usize] as i32).clamp(0, 255) as u8;
        }
    }
}

/// Edge offset of the `w`×`h` block at `origin` of `src`, whose neighbours
/// `oa` and `ob` away are all in `src`, into `dst`.
pub fn sao_edge(dst: &mut [u8], dst_stride: usize, src: &[u8], origin: usize, src_stride: usize, w: usize, h: usize, oa: isize, ob: isize, offs: &[i8; 4]) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return simd128::sao_edge(dst, dst_stride, src, origin, src_stride, w, h, oa, ob, offs);
    }
    // `edgeIdx` 0, 1, 3, 4 take the four offsets; 2 is the plateau (Table 8-19).
    #[allow(unreachable_code)]
    let table = [offs[0], offs[1], 0, offs[2], offs[3]];
    for y in 0..h {
        for x in 0..w {
            let i = (origin + y * src_stride + x) as isize;
            let v = src[i as usize] as i32;
            let a = src[(i + oa) as usize] as i32;
            let b = src[(i + ob) as usize] as i32;
            let e = (2 + (v - a).signum() + (v - b).signum()) as usize;
            dst[y * dst_stride + x] = (v + table[e] as i32).clamp(0, 255) as u8;
        }
    }
}
