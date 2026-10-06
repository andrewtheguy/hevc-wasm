//! The deblocking filter (§8.7.2), a coding tree block at a time as the
//! wavefront finishes with its neighbours (see `ctu::decode_row`).
//!
//! The picture's vertical edges must all be filtered before any horizontal
//! one that crosses them, so a block's vertical edges are filtered when it is
//! reached, and its horizontal edges eight columns behind: from eight columns
//! into the block on its left to eight columns short of its own right edge,
//! where the next block's vertical edge has yet to run. The last block of a
//! row runs to the picture's edge.

use crate::ctu::Maps;
use crate::kernels;
use crate::pic::{Motion, PRED_INTRA};
use crate::ps::Pps;
use crate::shared::PlanePtr;
use crate::slice::SliceHeader;
use crate::tables::TC_TABLE;

pub struct DeblockCtx<'a> {
    pub planes: [PlanePtr; 3],
    pub maps: Maps,
    pub sh: &'a SliceHeader,
    pub pps: &'a Pps,
    /// POC of each entry of `RefPicList0`.
    pub ref_pocs: &'a [i32],
}

impl<'a> DeblockCtx<'a> {
    /// §8.7.2.4: the strength of the edge between 4×4 blocks `p` and `q`.
    fn boundary_strength(&self, p: usize, q: usize, tu_edge: bool) -> i32 {
        let m = &self.maps;
        // SAFETY: both blocks' maps are complete and only read now.
        let (pm_p, pm_q) = unsafe { (m.pred_mode.get(p), m.pred_mode.get(q)) };
        if pm_p == PRED_INTRA || pm_q == PRED_INTRA {
            return 2;
        }
        // SAFETY: as above.
        if tu_edge && unsafe { m.nz.get(p) != 0 || m.nz.get(q) != 0 } {
            return 1;
        }
        // SAFETY: as above.
        let (mp, mq): (Motion, Motion) = unsafe { (m.motion.get(p), m.motion.get(q)) };
        if self.ref_pocs[mp.ref_idx as usize] != self.ref_pocs[mq.ref_idx as usize] {
            return 1;
        }
        ((mp.mv[0] as i32 - mq.mv[0] as i32).abs() >= 4 || (mp.mv[1] as i32 - mq.mv[1] as i32).abs() >= 4) as i32
    }

    /// Filters the edge of strength `bs` whose q block is 4×4 index `q` at
    /// (`x`, `y`), `dir` 0 vertical.
    fn filter_edge(&self, p: usize, q: usize, x: usize, y: usize, dir: usize, bs: i32) {
        let m = &self.maps;
        // SAFETY: both blocks' maps are complete and only read now.
        let qp = unsafe { (m.qp_y.get(p) as i32 + m.qp_y.get(q) as i32 + 1) >> 1 };
        let beta = kernels::luma_beta(qp, self.sh.beta_offset_div2);
        let tc = kernels::luma_tc(qp, bs, self.sh.tc_offset_div2);
        let pl = self.planes[0];
        // SAFETY: the samples an edge touches are this thread's (see the module).
        let data = unsafe { pl.block_mut(0, 0, pl.width, pl.height) };
        kernels::luma_edge(data, pl.stride, x, y, dir, beta, tc);
        if bs == 2 {
            for c in 1..3usize {
                let off = if c == 1 { self.pps.cb_qp_offset } else { self.pps.cr_qp_offset };
                let qpc = (qp + off).clamp(0, 57).min(51);
                let tc = TC_TABLE[(qpc + 2 + (self.sh.tc_offset_div2 << 1)).clamp(0, 53) as usize] as i32;
                let pl = self.planes[c];
                // SAFETY: as above.
                let data = unsafe { pl.block_mut(0, 0, pl.width, pl.height) };
                kernels::chroma_edge(data, pl.stride, x, y, dir, tc);
            }
        }
    }

    /// The edges of coding tree block (`cx`, `cy`): its vertical ones, and
    /// the horizontal ones eight columns behind.
    pub fn ctb(&self, cx: usize, cy: usize) {
        let m = &self.maps;
        let ctb4 = 1usize << (m.log2_ctb - 2);
        let (x4_0, y4_0) = (cx * ctb4, cy * ctb4);
        let (x4_1, y4_1) = ((x4_0 + ctb4).min(m.w4), (y4_0 + ctb4).min(m.height.div_ceil(4)));
        for y4 in y4_0..y4_1 {
            for x4 in (x4_0.max(2)..x4_1).step_by(2) {
                let q = y4 * m.w4 + x4;
                // SAFETY: the block's maps are complete and only read now.
                let e = unsafe { m.edges.get(q) };
                if e & 0b0101 == 0 {
                    continue;
                }
                let bs = self.boundary_strength(q - 1, q, e & 1 != 0);
                if bs > 0 {
                    self.filter_edge(q - 1, q, x4 * 4, y4 * 4, 0, bs);
                }
            }
        }
        let hx0 = x4_0.saturating_sub(2);
        let hx1 = if x4_1 == m.w4 { m.w4 } else { x4_1 - 2 };
        for y4 in (y4_0.max(2)..y4_1).step_by(2) {
            for x4 in hx0..hx1 {
                let q = y4 * m.w4 + x4;
                // SAFETY: as above.
                let e = unsafe { m.edges.get(q) };
                if e & 0b1010 == 0 {
                    continue;
                }
                let bs = self.boundary_strength(q - m.w4, q, e & 2 != 0);
                if bs > 0 {
                    self.filter_edge(q - m.w4, q, x4 * 4, y4 * 4, 1, bs);
                }
            }
        }
    }
}
