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

/// Whether the eight-line edge at (`x`, `y`), `n` taps a side, is in the
/// plane.
fn inside(pl: &PlanePtr, x: usize, y: usize, dir: usize, n: usize) -> bool {
    if dir == 0 { x >= n && x + n <= pl.width && y + 8 <= pl.height } else { y >= n && y + n <= pl.height && x + 8 <= pl.width }
}

pub struct DeblockCtx<'a> {
    pub planes: [PlanePtr; 3],
    pub maps: Maps,
    pub sh: &'a SliceHeader,
    pub pps: &'a Pps,
}

impl<'a> DeblockCtx<'a> {
    /// Filters the eight lines of an edge from (`x`, `y`), `dir` 0 vertical:
    /// two segments of four lines, of strengths `bs` and with the 4×4
    /// blocks `p` and `q` either side. An edge lies on the 8×8 grid and no
    /// block is smaller, so its segments come in such pairs, and a segment
    /// of no strength is left as it is by a `beta` and a `tc` of zero.
    fn filter_edge(&self, p: [usize; 2], q: [usize; 2], x: usize, y: usize, dir: usize, bs: [u64; 2]) {
        let m = &self.maps;
        let (mut beta, mut tc, mut tcc) = ([0i32; 2], [0i32; 2], [[0i32; 2]; 2]);
        for s in 0..2 {
            if bs[s] == 0 {
                continue;
            }
            // SAFETY: both blocks' maps are complete and only read now.
            let qp = unsafe { (m.qp_y.get(p[s]) as i32 + m.qp_y.get(q[s]) as i32 + 1) >> 1 };
            beta[s] = kernels::luma_beta(qp, self.sh.beta_offset_div2);
            tc[s] = kernels::luma_tc(qp, bs[s] as i32, self.sh.tc_offset_div2);
            if bs[s] == 2 {
                for c in 0..2 {
                    let off = if c == 0 { self.pps.cb_qp_offset } else { self.pps.cr_qp_offset };
                    let qpc = (qp + off).clamp(0, 57).min(51);
                    tcc[c][s] = TC_TABLE[(qpc + 2 + (self.sh.tc_offset_div2 << 1)).clamp(0, 53) as usize] as i32;
                }
            }
        }
        let pl = self.planes[0];
        debug_assert!(inside(&pl, x, y, dir, 4));
        // SAFETY: the samples an edge touches are this thread's (see the module).
        unsafe { kernels::luma_edge(pl.at(x, y), pl.stride, dir, beta, tc) };
        if bs[0] == 2 || bs[1] == 2 {
            for c in 0..2 {
                let pl = self.planes[c + 1];
                debug_assert!(inside(&pl, x, y, dir, 2));
                // SAFETY: as above.
                unsafe { kernels::chroma_edge(pl.at(x, y), pl.stride, dir, tcc[c]) };
            }
        }
    }

    /// The edges of direction `dir` (0 vertical) of the 4×4 blocks
    /// `x4_0..x4_1` of rows `y4_0..y4_1`, from their strengths eight blocks
    /// at a time: most are zero. A vertical edge's two segments are in two
    /// rows of the map, a horizontal edge's in two entries of one.
    fn edges(&self, y4_0: usize, y4_1: usize, x4_0: usize, x4_1: usize, dir: usize) {
        let m = &self.maps;
        let field = if dir == 0 { 0x0303_0303_0303_0303u64 } else { 0x0c0c_0c0c_0c0c_0c0cu64 };
        for y4 in (y4_0..y4_1).step_by(2) {
            let mut x4 = x4_0;
            while x4 < x4_1 {
                let q0 = y4 * m.w4 + x4;
                let mask = field & (u64::MAX >> (64 - 8 * (x4_1 - x4).min(8)));
                // SAFETY: the blocks' maps are complete and only read now,
                // and the map has eight spare entries past its end.
                let (g0, g1) = unsafe { (m.bs.word(q0) & mask, if dir == 0 { m.bs.word(q0 + m.w4) & mask } else { 0 }) };
                let mut g = g0 | g1;
                while g != 0 {
                    let k = (g.trailing_zeros() >> 3) as usize & !dir;
                    let (x, y) = ((x4 + k) * 4, y4 * 4);
                    if dir == 0 {
                        self.filter_edge([q0 + k - 1, q0 + k + m.w4 - 1], [q0 + k, q0 + k + m.w4], x, y, 0, [(g0 >> (8 * k)) & 3, (g1 >> (8 * k)) & 3]);
                        g &= !(0xff << (8 * k));
                    } else {
                        self.filter_edge([q0 + k - m.w4, q0 + k + 1 - m.w4], [q0 + k, q0 + k + 1], x, y, 1, [(g0 >> (8 * k + 2)) & 3, (g0 >> (8 * k + 10)) & 3]);
                        g &= !(0xffff << (8 * k));
                    }
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
        // Most blocks have no edge with a strength, and are not scanned.
        // SAFETY: the blocks' entries are complete and only read now.
        let (own, left) = unsafe { (m.edged.get(cy * m.ctb_w + cx), if cx > 0 { m.edged.get(cy * m.ctb_w + cx - 1) } else { 0 }) };
        if own != 0 {
            self.edges(y4_0, y4_1, x4_0, x4_1, 0);
        }
        if own | left != 0 {
            let hx0 = x4_0.saturating_sub(2);
            let hx1 = if x4_1 == m.w4 { m.w4 } else { x4_1 - 2 };
            self.edges(y4_0, y4_1, hx0, hx1, 1);
        }
    }
}
