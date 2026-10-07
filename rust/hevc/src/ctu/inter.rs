//! Inter prediction of a P slice: the prediction unit syntax (§7.3.8.6,
//! §7.3.8.9), merge mode (§8.5.3.2.2 to §8.5.3.2.5), the motion vector
//! predictor (§8.5.3.2.6, §8.5.3.2.7), and motion compensation (§8.5.3.3)
//! from one reference.

use super::{PartMode, Row};
use crate::cabac::*;
use crate::error::{Error, Result};
use crate::kernels;
use crate::pic::{Motion, PRED_INTER, PRED_SKIP};
use crate::tables::{CHROMA_FILTER, LUMA_FILTER};

/// `tx` of §8.5.3.2.7 for every `td` in `-128..=127`.
const fn build_tx() -> [i32; 256] {
    let mut t = [0i32; 256];
    let mut i = 0;
    while i < 256 {
        let td = i as i32 - 128;
        t[i] = if td == 0 { 0 } else { (16384 + (if td < 0 { -td } else { td } >> 1)) / td };
        i += 1;
    }
    t
}
static TX_BY_TD: [i32; 256] = build_tx();

/// A motion vector scaled by the ratio of two POC distances (§8.5.3.2.7).
fn scale_mv(mv: [i32; 2], td: i32, tb: i32) -> [i32; 2] {
    let td = td.clamp(-128, 127);
    let tb = tb.clamp(-128, 127);
    if td == 0 {
        return mv;
    }
    let tx = TX_BY_TD[(td + 128) as usize];
    let dsf = ((tb * tx + 32) >> 6).clamp(-4096, 4095);
    let s = |v: i32| -> i32 {
        let p = dsf * v;
        let r = (p.abs() + 127) >> 8;
        (if p < 0 { -r } else { r }).clamp(-32768, 32767)
    };
    [s(mv[0]), s(mv[1])]
}

/// A prediction unit's motion while it is derived.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Mv {
    mv: [i32; 2],
    ref_idx: i32,
}

impl Mv {
    fn of(m: Motion) -> Self {
        Mv { mv: [m.mv[0] as i32, m.mv[1] as i32], ref_idx: m.ref_idx as i32 }
    }
}

impl<'a> Row<'a> {
    /// Parses and predicts every prediction unit of an inter coding unit;
    /// returns the `merge_flag` of a 2N×2N one.
    pub(super) fn inter_prediction_units(&mut self, x0: usize, y0: usize, n: usize) -> Result<bool> {
        let h = n / 2;
        let parts: &[(usize, usize, usize, usize)] = match self.part_mode {
            PartMode::Part2Nx2N => &[(x0, y0, n, n)],
            PartMode::Part2NxN => &[(x0, y0, n, h), (x0, y0 + h, n, h)],
            PartMode::PartNx2N => &[(x0, y0, h, n), (x0 + h, y0, h, n)],
            PartMode::PartNxN => &[(x0, y0, h, h), (x0 + h, y0, h, h), (x0, y0 + h, h, h), (x0 + h, y0 + h, h, h)],
        };
        let mut merge = false;
        for (i, &(x, y, w, hh)) in parts.iter().enumerate() {
            merge = self.prediction_unit(x0, y0, n, x, y, w, hh, i, false)?;
            if i > 0 {
                self.edge_strengths(x, y, w, hh, false);
            }
        }
        Ok(merge && self.part_mode == PartMode::Part2Nx2N)
    }

    /// §7.3.8.6 and §8.5.3 for one prediction unit; returns its `merge_flag`.
    pub(super) fn prediction_unit(&mut self, xcb: usize, ycb: usize, ncb: usize, xp: usize, yp: usize, w: usize, h: usize, part_idx: usize, skip: bool) -> Result<bool> {
        let merge = skip || self.cab.decode(CTX_MERGE_FLAG) == 1;
        let mv = if merge {
            let max = self.pic.sh.max_num_merge_cand;
            let idx = if max <= 1 || self.cab.decode(CTX_MERGE_IDX) == 0 { 0 } else { 1 + self.cab.bypass_ones(max as u32 - 2) as usize };
            self.merge_motion(xcb, ycb, ncb, xp, yp, w, h, part_idx, idx)?
        } else {
            let num = self.pic.sh.num_ref_idx_l0_active;
            let mut ref_idx = 0i32;
            if num > 1 {
                while ref_idx < num as i32 - 1 {
                    let bin = if ref_idx < 2 { self.cab.decode(CTX_REF_IDX + ref_idx as usize) } else { self.cab.bypass() };
                    if bin == 0 {
                        break;
                    }
                    ref_idx += 1;
                }
            }
            let mvd = self.mvd_coding()?;
            let mvp_flag = self.cab.decode(CTX_MVP_FLAG);
            if ref_idx as usize >= self.pic.refs.len() {
                return Err(Error::invalid("a reference index beyond the list"));
            }
            let mvp = self.amvp(xcb, ycb, ncb, xp, yp, w, h, part_idx, ref_idx, mvp_flag);
            let mut mv = Mv { mv: [0; 2], ref_idx };
            for c in 0..2 {
                let u = (mvp[c] + mvd[c] + 65536) & 0xffff;
                mv.mv[c] = if u >= 32768 { u - 65536 } else { u };
            }
            mv
        };
        let m = self.pic.maps;
        self.fill4(m.motion, xp, yp, w, h, Motion { mv: [mv.mv[0] as i16, mv.mv[1] as i16], ref_idx: mv.ref_idx as i8 });
        self.motion_compensate(xp, yp, w, h, mv);
        Ok(merge)
    }

    /// `mvd_coding()` (§7.3.8.9).
    fn mvd_coding(&mut self) -> Result<[i32; 2]> {
        let gt0 = [self.cab.decode(CTX_MVD_GT0) == 1, self.cab.decode(CTX_MVD_GT0) == 1];
        let mut gt1 = [false; 2];
        for c in 0..2 {
            if gt0[c] {
                gt1[c] = self.cab.decode(CTX_MVD_GT1) == 1;
            }
        }
        let mut mvd = [0i32; 2];
        for c in 0..2 {
            if gt0[c] {
                let mut abs = 1;
                if gt1[c] {
                    abs = 2 + self.eg_k(1)?;
                    // §7.4.9.9: within 16 bits, as the vector is.
                    if abs > 1 << 15 {
                        return Err(Error::invalid("a motion vector difference beyond 16 bits"));
                    }
                }
                let mut abs = abs as i32;
                if self.cab.bypass() == 1 {
                    abs = -abs;
                }
                mvd[c] = abs;
            }
        }
        Ok(mvd)
    }

    /// §6.4.2: the inter neighbour at (`xn`, `yn`) a prediction block may use,
    /// as its 4×4 index; `before` when it is to the left or above, in a row
    /// or column of the block, and so decoded before it wherever it is.
    fn pb_avail(&self, xcb: usize, ycb: usize, ncb: usize, xp: usize, yp: usize, w: usize, h: usize, part_idx: usize, xn: i32, yn: i32, before: bool) -> Option<usize> {
        let m = self.pic.maps;
        if !self.in_picture(xn, yn) {
            return None;
        }
        let (xnu, ynu) = (xn as usize, yn as usize);
        let same_cb = xnu >= xcb && xnu < xcb + ncb && ynu >= ycb && ynu < ycb + ncb;
        let avail = if same_cb { !(w * 2 == ncb && h * 2 == ncb && part_idx == 1 && ycb + h <= ynu && xcb + w > xnu) } else { before || self.available(xp, yp, xn, yn) };
        if !avail {
            return None;
        }
        let i = m.idx4(xnu, ynu);
        matches!(self.map_get(m.pred_mode, i), PRED_INTER | PRED_SKIP).then_some(i)
    }

    #[inline]
    fn motion_at(&self, i: usize) -> Mv {
        Mv::of(self.map_get(self.pic.maps.motion, i))
    }

    /// The merge candidate at `merge_idx` (§8.5.3.2.2 to §8.5.3.2.5), of a P
    /// slice: spatial, then zero vectors. The list is built only as far as
    /// the index, since each candidate is found and pruned on its own, and
    /// the index is nearly always 0.
    #[inline(never)]
    fn merge_motion(&mut self, xcb: usize, ycb: usize, ncb: usize, xp0: usize, yp0: usize, w0: usize, h0: usize, part_idx0: usize, merge_idx: usize) -> Result<Mv> {
        let plevel = self.pic.pps.log2_parallel_merge_level as usize;
        let (xp, yp, w, h, part_idx) = if plevel > 2 && ncb == 8 { (xcb, ycb, ncb, ncb, 0) } else { (xp0, yp0, w0, h0, part_idx0) };
        let pm = self.part_mode;
        let same_mer = |xn: i32, yn: i32| -> bool { (xp >> plevel) as i32 == xn >> plevel && (yp >> plevel) as i32 == yn >> plevel };
        let (xi, yi, wi, hi) = (xp as i32, yp as i32, w as i32, h as i32);
        // The candidates found so far: the next one that survives the
        // pruning is number `found`.
        let mut found = 0usize;
        let at = |s: &Self, xn: i32, yn: i32, before: bool| s.pb_avail(xcb, ycb, ncb, xp, yp, w, h, part_idx, xn, yn, before).map(|i| s.motion_at(i));
        // A1
        let (xa1, ya1) = (xi - 1, yi + hi - 1);
        let a1 = if !same_mer(xa1, ya1) && !(part_idx == 1 && pm == PartMode::PartNx2N) { at(self, xa1, ya1, true) } else { None };
        if let Some(m) = a1 {
            if found == merge_idx {
                return Ok(m);
            }
            found += 1;
        }
        // B1
        let (xb1, yb1) = (xi + wi - 1, yi - 1);
        let b1 = if !same_mer(xb1, yb1) && !(part_idx == 1 && pm == PartMode::Part2NxN) { at(self, xb1, yb1, true) } else { None };
        if let Some(m) = b1.filter(|&m| a1 != Some(m)) {
            if found == merge_idx {
                return Ok(m);
            }
            found += 1;
        }
        // B0
        let (xb0, yb0) = (xi + wi, yi - 1);
        let b0 = if !same_mer(xb0, yb0) { at(self, xb0, yb0, false) } else { None };
        if let Some(m) = b0.filter(|&m| b1 != Some(m)) {
            if found == merge_idx {
                return Ok(m);
            }
            found += 1;
        }
        // A0
        let (xa0, ya0) = (xi - 1, yi + hi);
        let a0 = if !same_mer(xa0, ya0) { at(self, xa0, ya0, false) } else { None };
        if let Some(m) = a0.filter(|&m| a1 != Some(m)) {
            if found == merge_idx {
                return Ok(m);
            }
            found += 1;
        }
        // B2, only while fewer than four survived.
        let (xb2, yb2) = (xi - 1, yi - 1);
        let b2 = if found != 4 && !same_mer(xb2, yb2) { at(self, xb2, yb2, true) } else { None };
        if let Some(m) = b2.filter(|&m| a1 != Some(m) && b1 != Some(m)) {
            if found == merge_idx {
                return Ok(m);
            }
            found += 1;
        }
        // Zero vectors, each reference in turn, then the first again.
        let max = self.pic.sh.max_num_merge_cand as usize;
        let num_ref = self.pic.refs.len();
        if merge_idx >= max {
            return Err(Error::invalid("merge_idx beyond the candidates"));
        }
        let zero_idx = merge_idx - found;
        Ok(Mv { mv: [0, 0], ref_idx: if zero_idx < num_ref { zero_idx as i32 } else { 0 } })
    }

    /// The motion vector predictor (§8.5.3.2.6, §8.5.3.2.7) for `ref_idx`.
    fn amvp(&self, xcb: usize, ycb: usize, ncb: usize, xp: usize, yp: usize, w: usize, h: usize, part_idx: usize, ref_idx: i32, mvp_flag: u32) -> [i32; 2] {
        let refs = self.pic.refs;
        let target_poc = refs[ref_idx as usize].poc;
        let (xi, yi, wi, hi) = (xp as i32, yp as i32, w as i32, h as i32);
        let a_pos = [(xi - 1, yi + hi, false), (xi - 1, yi + hi - 1, true)];
        let b_pos = [(xi + wi, yi - 1, false), (xi + wi - 1, yi - 1, true), (xi - 1, yi - 1, true)];
        let avail = |p: (i32, i32, bool)| self.pb_avail(xcb, ycb, ncb, xp, yp, w, h, part_idx, p.0, p.1, p.2).map(|i| self.motion_at(i));
        // The same picture, unscaled; or any, scaled by POC distance.
        let direct = |m: &Mv| -> Option<[i32; 2]> { (refs[m.ref_idx as usize].poc == target_poc).then_some(m.mv) };
        let scaled = |m: &Mv| -> Option<[i32; 2]> {
            let poc = refs[m.ref_idx as usize].poc;
            Some(if poc == target_poc { m.mv } else { scale_mv(m.mv, self.pic_poc() - poc, self.pic_poc() - target_poc) })
        };
        let cand_a = [avail(a_pos[0]), avail(a_pos[1])];
        let is_scaled = cand_a[0].is_some() || cand_a[1].is_some();
        let mut mv_a = cand_a.iter().flatten().find_map(direct);
        if mv_a.is_none() {
            mv_a = cand_a.iter().flatten().find_map(scaled);
        }
        let cand_b = [avail(b_pos[0]), avail(b_pos[1]), avail(b_pos[2])];
        let mut mv_b = cand_b.iter().flatten().find_map(direct);
        if !is_scaled && mv_b.is_some() {
            mv_a = mv_b;
        }
        if !is_scaled {
            mv_b = cand_b.iter().flatten().find_map(scaled);
        }
        let mut list = [[0i32; 2]; 2];
        let mut n = 0;
        if let Some(a) = mv_a {
            list[n] = a;
            n += 1;
        }
        if let Some(b) = mv_b {
            if mv_a != Some(b) {
                list[n] = b;
            }
        }
        list[mvp_flag as usize]
    }

    fn pic_poc(&self) -> i32 {
        self.pic.poc
    }

    /// §8.5.3.3: the prediction of the block from its reference, into the
    /// picture.
    fn motion_compensate(&mut self, xp: usize, yp: usize, w: usize, h: usize, mv: Mv) {
        if mv.ref_idx == 0 && mv.mv == [0, 0] {
            // The block's samples are the first reference's, which the row
            // started as (`PictureCtx::base`).
            return;
        }
        let r = &self.pic.refs[mv.ref_idx as usize].pic;
        let s = &mut *self.s;
        for c in 0..3usize {
            let plane = &r.planes[c];
            let dst_plane = self.pic.planes[c];
            // SAFETY: the block is this row's.
            let dst = unsafe { dst_plane.block_mut(xp, yp, w, h) };
            // A luma vector is in quarter samples; under 4:4:4 the chroma reads
            // the same vector, its quarters being the even eighths of the
            // chroma filter.
            let (xi, yi) = (xp as i32 + (mv.mv[0] >> 2), yp as i32 + (mv.mv[1] >> 2));
            let (fx, fy) = ((mv.mv[0] & 3) as usize, (mv.mv[1] & 3) as usize);
            if fx == 0 && fy == 0 && xi >= 0 && yi >= 0 && xi as usize + w <= plane.width && yi as usize + h <= plane.height {
                let off = yi as usize * plane.stride + xi as usize;
                kernels::copy_block(dst, dst_plane.stride, &plane.data[off..], plane.stride, w, h);
                continue;
            }
            let taps = if c == 0 { 8 } else { 4 };
            let margin = taps / 2 - 1;
            let (fw, fh) = (w + taps - 1, h + taps - 1);
            let (x0, y0) = (xi - margin as i32, yi - margin as i32);
            let inside = x0 >= 0 && y0 >= 0 && (x0 as usize + fw) <= plane.width && (y0 as usize + fh) <= plane.height;
            let (src, stride): (&[u8], usize) = if inside {
                (&plane.data[y0 as usize * plane.stride + x0 as usize..], plane.stride)
            } else {
                pad_footprint(&mut s.mc_pad, plane, x0, y0, fw, fh);
                (&s.mc_pad, fw)
            };
            if c == 0 {
                interp::<8>(src, stride, &LUMA_FILTER, fx, fy, w, h, dst, dst_plane.stride, &mut s.mc_tmp);
            } else {
                interp::<4>(src, stride, &CHROMA_FILTER, 2 * fx, 2 * fy, w, h, dst, dst_plane.stride, &mut s.mc_tmp);
            }
        }
    }
}

/// Copies the `fw`×`fh` footprint at (`x0`, `y0`) of `plane` into `pad`,
/// repeating the edge samples where it leaves the picture (§8.5.3.3.2's
/// clamping of the coordinates).
fn pad_footprint(pad: &mut [u8], plane: &crate::pic::Plane, x0: i32, y0: i32, fw: usize, fh: usize) {
    let (pw, ph) = (plane.width as i32, plane.height as i32);
    let left = (-x0).clamp(0, fw as i32) as usize;
    let right = ((x0 + fw as i32) - pw).clamp(0, fw as i32) as usize;
    let mid = fw - left - right;
    let sx_mid = (x0 + left as i32) as usize;
    let sx_all = x0.clamp(0, pw - 1) as usize;
    let mut prev_sy = usize::MAX;
    for y in 0..fh {
        let sy = (y0 + y as i32).clamp(0, ph - 1) as usize;
        if sy == prev_sy {
            pad.copy_within((y - 1) * fw..y * fw, y * fw);
            continue;
        }
        prev_sy = sy;
        let row = &plane.data[sy * plane.stride..sy * plane.stride + plane.width];
        let out = &mut pad[y * fw..y * fw + fw];
        if mid > 0 {
            out[left..left + mid].copy_from_slice(&row[sx_mid..sx_mid + mid]);
            out[..left].fill(row[0]);
            out[left + mid..].fill(row[plane.width - 1]);
        } else {
            out.fill(row[sx_all]);
        }
    }
}

/// The `N`-tap interpolation of one block from an in-bounds footprint whose
/// top left is `src`, into its samples (§8.5.3.3.3 with §8.5.3.3.4.2): one
/// pass with the rounding, or two with `tmp` between them.
fn interp<const N: usize>(src: &[u8], stride: usize, taps: &[[i16; N]], fx: usize, fy: usize, w: usize, h: usize, dst: &mut [u8], dst_stride: usize, tmp: &mut [i16]) {
    let m = N / 2 - 1;
    match (fx, fy) {
        (0, 0) => kernels::copy_block(dst, dst_stride, &src[m * stride + m..], stride, w, h),
        (fx, 0) => kernels::fir_h_uni::<N>(&src[m * stride..], stride, &taps[fx], w, h, dst, dst_stride),
        (0, fy) => kernels::fir_v_uni::<N>(&src[m..], stride, &taps[fy], w, h, dst, dst_stride),
        (fx, fy) => {
            kernels::fir_h::<N>(src, stride, &taps[fx], w, h + N - 1, tmp, w);
            kernels::fir_hv_uni::<N>(tmp, w, &taps[fy], w, h, dst, dst_stride);
        }
    }
}
