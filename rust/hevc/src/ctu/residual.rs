//! The transform tree (§7.3.8.8), transform units (§7.3.8.10), residual coding
//! (§7.3.8.11) and reconstruction: intra prediction, scaling, the inverse
//! transform and the residual add.

use super::{PartMode, Row};
use crate::cabac::*;
use crate::error::{Error, Result};
use crate::intra;
use crate::itx::{self, Dequant};
use crate::kernels;
use crate::tables::scan_set;

impl<'a> Row<'a> {
    pub(super) fn transform_tree(&mut self, x0: usize, y0: usize, log2: usize, depth: u8, parent_cbf_cb: bool, parent_cbf_cr: bool) -> Result<()> {
        let sps = self.pic.sps;
        let (max_tb, min_tb) = (sps.log2_max_tb_size as usize, sps.log2_min_tb_size as usize);
        let split = if log2 <= max_tb && log2 > min_tb && depth < self.max_trafo_depth && !(self.intra_split && depth == 0) {
            self.cab.decode(CTX_SPLIT_TRANSFORM + 5 - log2) == 1
        } else {
            let inter_split = sps.max_transform_hierarchy_depth_inter == 0 && !self.cu_intra && self.part_mode != PartMode::Part2Nx2N && depth == 0;
            log2 > max_tb || (self.intra_split && depth == 0) || inter_split
        };
        // Under 4:4:4 every transform block, 4×4 included, codes its chroma flags.
        let cbf_cb = (depth == 0 || parent_cbf_cb) && self.cab.decode(CTX_CBF_CHROMA + depth as usize) == 1;
        let cbf_cr = (depth == 0 || parent_cbf_cr) && self.cab.decode(CTX_CBF_CHROMA + depth as usize) == 1;
        if split {
            let half = 1usize << (log2 - 1);
            self.transform_tree(x0, y0, log2 - 1, depth + 1, cbf_cb, cbf_cr)?;
            self.transform_tree(x0 + half, y0, log2 - 1, depth + 1, cbf_cb, cbf_cr)?;
            self.transform_tree(x0, y0 + half, log2 - 1, depth + 1, cbf_cb, cbf_cr)?;
            self.transform_tree(x0 + half, y0 + half, log2 - 1, depth + 1, cbf_cb, cbf_cr)?;
            return Ok(());
        }
        let cbf_luma = !(self.cu_intra || depth != 0 || cbf_cb || cbf_cr) || self.cab.decode(CTX_CBF_LUMA + (depth == 0) as usize) == 1;
        self.transform_unit(x0, y0, log2, cbf_luma, cbf_cb, cbf_cr)
    }

    fn transform_unit(&mut self, x0: usize, y0: usize, log2: usize, cbf_luma: bool, cbf_cb: bool, cbf_cr: bool) -> Result<()> {
        let n = 1usize << log2;
        if (cbf_luma || cbf_cb || cbf_cr) && self.pic.pps.cu_qp_delta_enabled && !self.is_cu_qp_delta_coded {
            // cu_qp_delta_abs: a prefix of up to five context bins, then EG0.
            let mut v = 0u32;
            while v < 5 && self.cab.decode(CTX_CU_QP_DELTA + (v != 0) as usize) == 1 {
                v += 1;
            }
            if v == 5 {
                v += self.eg_k(0)?;
            }
            let mut delta = v as i32;
            if delta != 0 && self.cab.bypass() == 1 {
                delta = -delta;
            }
            if !(-26..=25).contains(&delta) {
                return Err(Error::invalid("CuQpDeltaVal"));
            }
            self.is_cu_qp_delta_coded = true;
            self.cu_qp_delta_val = delta;
            self.qp_y = super::wrap_qp(self.qp_y_pred + delta);
            self.set_cu_qp();
        }
        let m = self.pic.maps;
        let luma_mode = self.map_get(m.intra_mode, m.idx4(x0, y0));
        if self.cu_intra {
            self.intra_predict(0, x0, y0, n, luma_mode);
        }
        if cbf_luma {
            self.residual_block(x0, y0, log2, 0, luma_mode)?;
            self.fill4(m.nz, x0, y0, n, n, 1);
        }
        self.edge_strengths(x0, y0, n, n, true);
        // Chroma: the same block, the quadrant's mode under an NxN split.
        let half = self.cu_size / 2;
        let quadrant = if self.intra_split { ((y0 - self.cu_y >= half) as usize) * 2 + (x0 - self.cu_x >= half) as usize } else { 0 };
        let cmode = self.intra_chroma_mode[quadrant];
        for c in 1..3usize {
            if self.cu_intra {
                self.intra_predict(c, x0, y0, n, cmode);
            }
            if if c == 1 { cbf_cb } else { cbf_cr } {
                self.residual_block(x0, y0, log2, c, cmode)?;
            }
        }
        Ok(())
    }

    // ---- intra prediction (§8.4.4.2.1) ----

    /// Predicts the N×N block of component `c` at (`xb`, `yb`) from its
    /// neighbours in the picture.
    fn intra_predict(&mut self, c: usize, xb: usize, yb: usize, n: usize, mode: u8) {
        let plane = self.pic.planes[c];
        let n2 = 2 * n;
        // Availability is per 4×4 block, so it is asked once per four samples;
        // beside and above the block it is a matter of the picture's edge,
        // below and to the right of it one of decoding order too.
        let mut left_ok = [false; 16];
        let mut top_ok = [false; 16];
        for k in 0..n / 4 {
            left_ok[k] = self.in_picture(xb as i32 - 1, (yb + 4 * k) as i32);
            top_ok[k] = self.in_picture((xb + 4 * k) as i32, yb as i32 - 1);
        }
        for k in n / 4..n2 / 4 {
            left_ok[k] = self.available(xb, yb, xb as i32 - 1, (yb + 4 * k) as i32);
            top_ok[k] = self.available(xb, yb, (xb + 4 * k) as i32, yb as i32 - 1);
        }
        let corner_ok = self.in_picture(xb as i32 - 1, yb as i32 - 1);
        let refs = &mut self.s.iref;
        refs.reset(n);
        for k in 0..n2 / 4 {
            if left_ok[k] {
                for j in 4 * k..4 * k + 4 {
                    // SAFETY: the left neighbours are decoded, in this row.
                    refs.left[j] = unsafe { plane.get(xb - 1, yb + j) };
                }
                refs.left_avail[4 * k..4 * k + 4].fill(true);
            }
            if top_ok[k] {
                // SAFETY: the row above is decoded there, or it is this row.
                refs.top[4 * k..4 * k + 4].copy_from_slice(unsafe { plane.row(xb + 4 * k, yb - 1, 4) });
                refs.top_avail[4 * k..4 * k + 4].fill(true);
            }
        }
        if corner_ok {
            // SAFETY: as above.
            refs.corner = unsafe { plane.get(xb - 1, yb - 1) };
            refs.corner_avail = true;
        }
        refs.substitute(n);
        // SAFETY: the block is this row's.
        let out = unsafe { plane.block_mut(xb, yb, n, n) };
        intra::predict(refs, n, mode, c == 0, out, plane.stride);
    }

    // ---- residual coding (§7.3.8.11) and reconstruction (§8.6) ----

    fn residual_block(&mut self, x0: usize, y0: usize, log2: usize, c_idx: usize, pred_mode_intra: u8) -> Result<()> {
        let n = 1usize << log2;
        let qp = if c_idx == 0 {
            self.qp_y
        } else {
            // §8.6.1: under 4:4:4 the chroma QP is the luma's with its offset,
            // capped, not mapped through the 4:2:0 table.
            let off = if c_idx == 1 { self.pic.pps.cb_qp_offset + self.pic.sh.cb_qp_offset } else { self.pic.pps.cr_qp_offset + self.pic.sh.cr_qp_offset };
            (self.qp_y + off).clamp(0, 57).min(51)
        };
        // Each coefficient is scaled as it lands.
        let dq = itx::Dequant::new(n, qp);
        // The engine's registers in locals for the whole block.
        let mut view = self.cab.view();
        let cab = &mut view;
        // last_sig_coeff_{x,y}_prefix and suffix
        let (ctx_off, ctx_shift) = if c_idx == 0 { (3 * (log2 - 2) + ((log2 - 1) >> 2), (log2 + 1) >> 2) } else { (15, log2 - 2) };
        let cmax = (log2 << 1) - 1;
        let (bx, by) = (CTX_LAST_X_PREFIX + ctx_off, CTX_LAST_Y_PREFIX + ctx_off);
        let mut px = 0usize;
        while px < cmax && cab.decode(bx + (px >> ctx_shift)) == 1 {
            px += 1;
        }
        let mut py = 0usize;
        while py < cmax && cab.decode(by + (py >> ctx_shift)) == 1 {
            py += 1;
        }
        let mut last_x = px;
        if px > 3 {
            let nb = (px >> 1) - 1;
            last_x = ((2 + (px & 1)) << nb) + cab.bypass_bits(nb as u32) as usize;
        }
        let mut last_y = py;
        if py > 3 {
            let nb = (py >> 1) - 1;
            last_y = ((2 + (py & 1)) << nb) + cab.bypass_bits(nb as u32) as usize;
        }
        if last_x >= n || last_y >= n {
            return Err(Error::invalid("a last significant coefficient outside the block"));
        }
        // scanIdx (§7.4.9.11): 4×4 and 8×8 intra blocks scan with their mode.
        let scan_idx = if self.cu_intra && log2 <= 3 {
            if (6..=14).contains(&pred_mode_intra) {
                2
            } else if (22..=30).contains(&pred_mode_intra) {
                1
            } else {
                0
            }
        } else {
            0
        };
        if scan_idx == 2 {
            std::mem::swap(&mut last_x, &mut last_y);
        }
        let log2sb = log2 - 2;
        let nsb = 1usize << log2sb;
        let sc = scan_set(log2sb, scan_idx);
        let last_sb = sc.sb_inv[(last_y >> 2) * nsb + (last_x >> 2)] as usize;
        let last_pos = sc.pos_inv[(last_y & 3) * 4 + (last_x & 3)] as usize;
        // Only the sub-blocks up to the last can hold a coefficient: clear
        // their rows, whole, so the transform reads zeros past the live
        // columns.
        let fh = (sc.sb_rows[last_sb] as usize) << 2;
        let co = &mut self.s.coeffs[..n * n];
        kernels::fill_i16(&mut co[..fh * n], 0);
        let mut csbf = 0u64;
        let mut nz = [0usize; 2];
        let mut c1: usize = 1;
        let sig_base = CTX_SIG + if c_idx == 0 { 0 } else { 27 };
        let gt1_base = CTX_GT1 + if c_idx == 0 { 0 } else { 16 };
        let gt2_base = CTX_GT2 + if c_idx == 0 { 0 } else { 4 };
        for i in (0..=last_sb).rev() {
            let (xs, ys) = (sc.sb[i].0 as usize, sc.sb[i].1 as usize);
            let right = xs + 1 < nsb && (csbf >> ((xs + 1) * 8 + ys)) & 1 != 0;
            let below = ys + 1 < nsb && (csbf >> (xs * 8 + ys + 1)) & 1 != 0;
            let infer_dc = i < last_sb && i > 0;
            // coded_sub_block_flag: inferred for the first and last sub-blocks.
            if infer_dc && cab.decode(CTX_CSBF + (right || below) as usize + if c_idx == 0 { 0 } else { 2 }) == 0 {
                continue;
            }
            csbf |= 1 << (xs * 8 + ys);
            let prev_csbf = right as usize | ((below as usize) << 1);
            let (sig_row, sig_off) = if log2 == 2 {
                (sc.sig_4x4, 0usize)
            } else {
                let mut o = if xs > 0 || ys > 0 { 3 } else { 0 };
                if c_idx == 0 {
                    o += if log2 == 3 {
                        if scan_idx == 0 {
                            9
                        } else {
                            15
                        }
                    } else {
                        21
                    };
                } else {
                    o = if log2 == 3 { 9 } else { 12 };
                }
                (&sc.sig_nb[prev_csbf & 3], o)
            };
            let sig_ctx = sig_base + sig_off;
            let sb = SubBlock {
                sig_ctx: std::array::from_fn(|k| sig_row[k].wrapping_add(sig_ctx as u8)),
                dc_ctx: if xs == 0 && ys == 0 { sig_base } else { sig_ctx + sig_row[0] as usize },
                gt1_set: gt1_base + if i == 0 || c_idx > 0 { 0 } else { 8 },
                gt2_set: gt2_base + if i == 0 || c_idx > 0 { 0 } else { 2 },
                pos: sc.pos,
                x: xs << 2,
                y: ys << 2,
                n,
                last_pos: if i == last_sb { last_pos } else { 16 },
                infer_dc,
            };
            let (engine, c1_next) = sub_block(cab.engine(), cab.ctx, &sb, co, &dq, c1, &mut nz)?;
            cab.restore(engine);
            c1 = c1_next;
        }
        let (nz_w, nz_h) = (nz[0], nz[1]);
        let engine = cab.engine();
        self.cab.restore(engine);
        self.reconstruct_residual(x0, y0, log2, c_idx, nz_w, nz_h);
        Ok(())
    }

    /// §8.6.4 over the scaled coefficients, added to the picture.
    fn reconstruct_residual(&mut self, x0: usize, y0: usize, log2: usize, c_idx: usize, nz_w: usize, nz_h: usize) {
        let n = 1usize << log2;
        let dst = self.cu_intra && c_idx == 0 && n == 4;
        let s = &mut *self.s;
        itx::inverse_transform(&s.coeffs[..n * n], &mut s.itx_tmp, &mut s.res, n, nz_w, nz_h, dst);
        let plane = self.pic.planes[c_idx];
        // SAFETY: the block is this row's.
        let out = unsafe { plane.block_mut(x0, y0, n, n) };
        kernels::add_residual(out, plane.stride, &s.res[..n * n], n, n);
    }
}

/// What one sub-block's syntax is decoded against.
struct SubBlock<'s> {
    /// The significance context of each position after the first; that of
    /// the first is `dc_ctx`.
    sig_ctx: [u8; 16],
    dc_ctx: usize,
    /// The greater-than contexts of the sub-block, before the previous
    /// sub-block's say.
    gt1_set: usize,
    gt2_set: usize,
    /// The scan inside the sub-block.
    pos: &'s [(u8, u8); 16],
    /// The sub-block's top left in the block, and the block's width.
    x: usize,
    y: usize,
    n: usize,
    /// The scan position of the block's last coefficient, when it is in
    /// this sub-block; 16 otherwise.
    last_pos: usize,
    /// Whether the first position is significant when none other is.
    infer_dc: bool,
}

/// One coded sub-block after its flag (§7.3.8.11): the significance flags,
/// the levels and the signs, each coefficient scaled as it lands in `co`;
/// `nz` grows to cover them. `c1` is the greater-than-1 context the previous
/// sub-block left, and the one this leaves is returned with the engine. Out
/// of line, so that the engine's registers and the loops' few counters get
/// the registers of a function of their own.
#[inline(never)]
fn sub_block(engine: Engine, ctx: &mut Contexts, sb: &SubBlock, co: &mut [i16], dq: &Dequant, c1_in: usize, nz: &mut [usize; 2]) -> Result<(Engine, usize)> {
    let mut v = View::new(engine, ctx);
    let cab = &mut v;
    // significant_coeff_flag, highest position first, as a bit per position.
    let mut sig = 0u32;
    let start: i32 = if sb.last_pos < 16 {
        sig = 1 << sb.last_pos;
        sb.last_pos as i32 - 1
    } else {
        15
    };
    if start >= 0 {
        for np in (1..=start as usize).rev() {
            sig |= cab.decode(sb.sig_ctx[np & 15] as usize) << np;
        }
        if (sb.infer_dc && sig == 0) || cab.decode(sb.dc_ctx) == 1 {
            sig |= 1;
        }
    }
    if sig == 0 {
        return Ok((cab.engine(), c1_in));
    }
    let nsig = sig.count_ones() as usize;
    // coeff_abs_level_greater1_flag (§9.3.4.2.6), for the first eight.
    let g1_ctx = sb.gt1_set + ((c1_in == 0) as usize) * 4;
    let mut c1: usize = 1;
    let mut g1 = 0u32;
    let mut first_g2: usize = 16;
    let mut rest = sig;
    for _ in 0..nsig.min(8) {
        let np = (31 - rest.leading_zeros()) as usize;
        rest &= !(1 << np);
        let b = cab.decode(g1_ctx + c1);
        g1 |= b << np;
        if b == 1 {
            c1 = 0;
            if first_g2 == 16 {
                first_g2 = np;
            }
        } else if (1..3).contains(&c1) {
            c1 += 1;
        }
    }
    let g2 = first_g2 != 16 && cab.decode(sb.gt2_set + (c1_in == 0) as usize) == 1;
    // The signs, aligned so each coefficient reads the top bit.
    let mut sbits = cab.bypass_bits(nsig as u32).wrapping_shl(32 - nsig as u32);
    // coeff_abs_level_remaining
    let mut rice = 0u32;
    let (mut nz_w, mut nz_h) = (nz[0], nz[1]);
    let mut rest = sig;
    for k in 0..nsig {
        let np = (31 - rest.leading_zeros()) as usize;
        rest &= !(1 << np);
        let is_g2_pos = first_g2 == np;
        let base = 1 + ((g1 >> np) & 1) as i32 + (is_g2_pos && g2) as i32;
        let threshold = if k < 8 {
            if is_g2_pos {
                3
            } else {
                2
            }
        } else {
            1
        };
        let mut abs = base;
        if base == threshold {
            abs += coeff_remaining(cab, rice)?;
            if abs > 3 * (1 << rice) {
                rice = (rice + 1).min(4);
            }
        }
        let neg = (sbits as i32) < 0;
        sbits <<= 1;
        let v = if neg { -abs } else { abs };
        let (xc, yc) = (sb.x + sb.pos[np & 15].0 as usize, sb.y + sb.pos[np & 15].1 as usize);
        nz_w = nz_w.max(xc + 1);
        nz_h = nz_h.max(yc + 1);
        co[yc * sb.n + xc] = dq.apply(v.clamp(-32768, 32767));
    }
    *nz = [nz_w, nz_h];
    Ok((cab.engine(), c1))
}

/// `coeff_abs_level_remaining` (§9.3.3.11).
#[inline(always)]
fn coeff_remaining(cab: &mut View, rice: u32) -> Result<i32> {
    let prefix = cab.bypass_ones(32);
    if prefix >= 32 {
        return Err(Error::invalid("a coefficient prefix too long"));
    }
    if prefix < 3 {
        Ok(((prefix << rice) + cab.bypass_bits(rice)) as i32)
    } else {
        let l = prefix - 3;
        if l + rice > 31 {
            return Err(Error::invalid("a coefficient suffix too long"));
        }
        Ok(((((1u32 << l) + 2) << rice) + cab.bypass_bits(l + rice)) as i32)
    }
}

