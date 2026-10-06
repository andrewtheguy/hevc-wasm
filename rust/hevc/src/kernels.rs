//! The sample loops: what runs per sample rather than per block.

use crate::tables::{BETA_TABLE, TC_TABLE};

// ---- motion compensation (§8.5.3.3.3) ----

/// `dst = src`, `w`×`h`, each at its stride.
pub fn copy_block(dst: &mut [u8], dst_stride: usize, src: &[u8], src_stride: usize, w: usize, h: usize) {
    for y in 0..h {
        dst[y * dst_stride..y * dst_stride + w].copy_from_slice(&src[y * src_stride..y * src_stride + w]);
    }
}

/// Horizontal `N`-tap filter of samples into the 14-bit intermediate:
/// `dst[y][x] = Σ t[i] * src[y][x + i]`.
pub fn fir_h<const N: usize>(src: &[u8], stride: usize, t: &[i16; N], w: usize, h: usize, dst: &mut [i16], dst_stride: usize) {
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

/// Vertical `N`-tap filter of samples into the intermediate.
pub fn fir_v_u8<const N: usize>(src: &[u8], stride: usize, t: &[i16; N], w: usize, h: usize, dst: &mut [i16], dst_stride: usize) {
    for y in 0..h {
        let out = &mut dst[y * dst_stride..y * dst_stride + w];
        for (x, o) in out.iter_mut().enumerate() {
            let mut acc = 0i32;
            for i in 0..N {
                acc += t[i] as i32 * src[(y + i) * stride + x] as i32;
            }
            *o = acc as i16;
        }
    }
}

/// Vertical `N`-tap filter of the intermediate, `>> 6`, into the intermediate.
pub fn fir_v_i16<const N: usize>(src: &[i16], stride: usize, t: &[i16; N], w: usize, h: usize, dst: &mut [i16], dst_stride: usize) {
    for y in 0..h {
        let out = &mut dst[y * dst_stride..y * dst_stride + w];
        for (x, o) in out.iter_mut().enumerate() {
            let mut acc = 0i32;
            for i in 0..N {
                acc += t[i] as i32 * src[(y + i) * stride + x] as i32;
            }
            *o = (acc >> 6) as i16;
        }
    }
}

/// The intermediate as samples (§8.5.3.3.4.2, uni-prediction).
pub fn put_uni(dst: &mut [u8], dst_stride: usize, src: &[i16], w: usize, h: usize) {
    for y in 0..h {
        let row = &mut dst[y * dst_stride..y * dst_stride + w];
        for (d, &v) in row.iter_mut().zip(&src[y * w..y * w + w]) {
            *d = ((v as i32 + 32) >> 6).clamp(0, 255) as u8;
        }
    }
}

// ---- reconstruction ----

/// `dst += res`, clipped to the sample range.
pub fn add_residual(dst: &mut [u8], dst_stride: usize, res: &[i16], w: usize, h: usize) {
    for y in 0..h {
        let row = &mut dst[y * dst_stride..y * dst_stride + w];
        for (d, &r) in row.iter_mut().zip(&res[y * w..y * w + w]) {
            *d = (*d as i32 + r as i32).clamp(0, 255) as u8;
        }
    }
}

// ---- intra prediction (§8.4.4.2.5, §8.4.4.2.6) ----

/// Planar prediction of an `n`×`n` block from `n + 1` left and top neighbours.
pub fn planar(dst: &mut [u8], stride: usize, n: usize, left: &[u8], top: &[u8]) {
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
    for y in 0..n {
        dst[y * stride..y * stride + n].fill(v);
    }
}

/// Angular prediction along rows: `refb[off + 1 + ..]` is the projected top
/// reference, and row `y` reads it at `(y + 1) * angle / 32`.
pub fn angular(dst: &mut [u8], stride: usize, n: usize, refb: &[i16], off: usize, angle: i32) {
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
/// < nz`, for `len` outputs; `tab` has 32-entry rows.
pub fn accum(out: &mut [i32], src: &[i32], s_in: usize, tab: &[i16], tstep: usize, k0: usize, kstep: usize, nz: usize, len: usize) {
    out[..len].fill(0);
    let mut k = k0;
    while k < nz {
        let c = src[k * s_in];
        if c != 0 {
            let row = &tab[k * tstep * 32..k * tstep * 32 + len];
            for (o, &t) in out[..len].iter_mut().zip(row) {
                *o += c * t as i32;
            }
        }
        k += kstep;
    }
}

/// The odd half of an `n`-point partial butterfly, combined with the even
/// half already in `out[..n / 2]`.
pub fn accum_butterfly(out: &mut [i32], src: &[i32], s_in: usize, tab: &[i16], tstep: usize, nz: usize, n: usize) {
    let half = n / 2;
    let mut odd = [0i32; 16];
    accum(&mut odd[..half], src, s_in, tab, tstep, 1, 2, nz, half);
    for j in 0..half {
        let even = out[j];
        out[j] = even + odd[j];
        out[n - 1 - j] = even - odd[j];
    }
}

// ---- deblocking (§8.7.2.5) ----

/// One four-line luma edge segment at (`x`, `y`): a vertical edge (`dir` 0)
/// between columns `x - 1` and `x`, or a horizontal one between rows.
pub fn luma_edge(data: &mut [u8], stride: usize, x: usize, y: usize, dir: usize, beta: i32, tc: i32) {
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

/// Band offset over a rectangle: `dst = src + band[src >> 3]`.
pub fn sao_band(dst: &mut [u8], src: &[u8], stride: usize, x0: usize, y0: usize, w: usize, h: usize, band: &[i8; 32]) {
    for y in y0..y0 + h {
        let (d, s) = (&mut dst[y * stride + x0..y * stride + x0 + w], &src[y * stride + x0..y * stride + x0 + w]);
        for (d, &v) in d.iter_mut().zip(s) {
            *d = (v as i32 + band[(v >> 3) as usize] as i32).clamp(0, 255) as u8;
        }
    }
}

/// Edge offset over a rectangle whose neighbours at `da` and `db` are all
/// inside the picture.
pub fn sao_edge(dst: &mut [u8], src: &[u8], stride: usize, x0: usize, y0: usize, w: usize, h: usize, da: (i32, i32), db: (i32, i32), offs: &[i8; 4]) {
    // `edgeIdx` 0, 1, 3, 4 take the four offsets; 2 is the plateau (Table 8-19).
    let table = [offs[0], offs[1], 0, offs[2], offs[3]];
    let oa = da.1 as isize * stride as isize + da.0 as isize;
    let ob = db.1 as isize * stride as isize + db.0 as isize;
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let i = y * stride + x;
            let v = src[i] as i32;
            let a = src[(i as isize + oa) as usize] as i32;
            let b = src[(i as isize + ob) as usize] as i32;
            let e = (2 + (v - a).signum() + (v - b).signum()) as usize;
            dst[i] = (v + table[e] as i32).clamp(0, 255) as u8;
        }
    }
}
