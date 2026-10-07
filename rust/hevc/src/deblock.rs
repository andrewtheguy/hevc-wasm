//! The deblocking filter (§8.7.2), a coding tree block at a time as the
//! wavefront finishes with its neighbours (see `ctu::decode_row`). The
//! strength of every edge was settled as the blocks decoded (`ctu`), so what
//! runs here is a scan of the strengths, nearly all zero, and the filter.
//!
//! The picture's vertical edges must all be filtered before any horizontal
//! one that crosses them, so a block's vertical edges are filtered when it is
//! reached, and its horizontal edges eight columns behind: from eight columns
//! into the block on its left to eight columns short of its own right edge,
//! where the next block's vertical edge has yet to run. The last block of a
//! row runs to the picture's edge.

use crate::ctu::Maps;
use crate::kernels;
use crate::ps::Pps;
use crate::shared::PlanePtr;
use crate::slice::SliceHeader;
use crate::tables::TC_TABLE;

pub struct DeblockCtx<'a> {
    pub planes: [PlanePtr; 3],
    pub maps: Maps,
    pub sh: &'a SliceHeader,
    pub pps: &'a Pps,
}

impl<'a> DeblockCtx<'a> {
    /// Filters the edge of strength `bs` whose q block is 4×4 index `q` at
    /// (`x`, `y`), `dir` 0 vertical.
    fn filter_edge(&self, p: usize, q: usize, x: usize, y: usize, dir: usize, bs: i32) {
        let m = &self.maps;
        // SAFETY: both blocks' maps are complete and only read now.
        let qp = unsafe { (m.qp_y.get(p) as i32 + m.qp_y.get(q) as i32 + 1) >> 1 };
        let beta = kernels::luma_beta(qp, self.sh.beta_offset_div2);
        let tc = kernels::luma_tc(qp, bs, self.sh.tc_offset_div2);
        let pl = self.planes[0];
        // The edge's four lines, four samples each side of it.
        let (bx, by, bw, bh) = if dir == 0 { (x - 4, y, 8, 4) } else { (x, y - 4, 4, 8) };
        // SAFETY: the samples an edge touches are this thread's (see the module).
        let data = unsafe { pl.block_mut(bx, by, bw, bh) };
        kernels::luma_edge(data, pl.stride, x - bx, y - by, dir, beta, tc);
        if bs == 2 {
            for c in 1..3usize {
                let off = if c == 1 { self.pps.cb_qp_offset } else { self.pps.cr_qp_offset };
                let qpc = (qp + off).clamp(0, 57).min(51);
                let tc = TC_TABLE[(qpc + 2 + (self.sh.tc_offset_div2 << 1)).clamp(0, 53) as usize] as i32;
                let pl = self.planes[c];
                let (bx, by) = if dir == 0 { (x - 2, y) } else { (x, y - 2) };
                // SAFETY: as above.
                let data = unsafe { pl.block_mut(bx, by, 4, 4) };
                kernels::chroma_edge(data, pl.stride, x - bx, y - by, dir, tc);
            }
        }
    }

    /// The edges of direction `dir` (0 vertical) of the 4×4 blocks
    /// `x4_0..x4_1` of rows `y4_0..y4_1`, every `y_step`th, from their
    /// strengths eight blocks at a time: most are zero.
    fn edges(&self, y4_0: usize, y4_1: usize, y_step: usize, x4_0: usize, x4_1: usize, dir: usize) {
        let m = &self.maps;
        let (field, back) = if dir == 0 { (0x0303_0303_0303_0303u64, 1) } else { (0x0c0c_0c0c_0c0c_0c0cu64, m.w4) };
        for y4 in (y4_0..y4_1).step_by(y_step) {
            let mut x4 = x4_0;
            while x4 < x4_1 {
                let q0 = y4 * m.w4 + x4;
                let n = (x4_1 - x4).min(8);
                // SAFETY: the blocks' maps are complete and only read now,
                // and the map has eight spare entries past its end.
                let mut g = unsafe { m.bs.word(q0) } & field & (u64::MAX >> (64 - 8 * n));
                while g != 0 {
                    let k = (g.trailing_zeros() >> 3) as usize;
                    let bs = (g >> (8 * k + 2 * dir)) & 3;
                    self.filter_edge(q0 + k - back, q0 + k, (x4 + k) * 4, y4 * 4, dir, bs as i32);
                    g &= !(0xff << (8 * k));
                }
                x4 += 8;
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
        self.edges(y4_0, y4_1, 1, x4_0, x4_1, 0);
        let hx0 = x4_0.saturating_sub(2);
        let hx1 = if x4_1 == m.w4 { m.w4 } else { x4_1 - 2 };
        self.edges(y4_0, y4_1, 2, hx0, hx1, 1);
    }
}
