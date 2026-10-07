//! Sample adaptive offset (§8.7.3), in place, a coding tree block at a time
//! once its neighbours are deblocked (see `ctu::decode_row`). The edge
//! offset reads one deblocked sample into each neighbour; the blocks above
//! and to the left are filtered by then and the others not yet, so a block
//! saves its last line and column before filtering itself where a block
//! below or to the right will read them.

use crate::ctu::Maps;
use crate::kernels;
use crate::shared::PlanePtr;

/// The stride of the block-with-border a thread assembles for the edge
/// offset: a coding tree block of up to 64 and a sample each side.
pub const SRC_STRIDE: usize = 64 + 2;

pub struct SaoCtx {
    pub planes: [PlanePtr; 3],
    pub maps: Maps,
}

impl SaoCtx {
    /// Coding tree block (`cx`, `cy`) of every component; `src` is the
    /// thread's block-with-border. Most blocks have nothing to do: neither
    /// an offset of their own nor a neighbour's edge offset to save a line
    /// or column for.
    pub fn ctb(&self, src: &mut [u8], cx: usize, cy: usize) {
        let m = &self.maps;
        // The components block (`bx`, `by`), if there is one, applies the
        // edge offset to, as bits. The neighbours below and to the right are
        // parsed by now (see `ctu::decode_row`).
        let edge = |bx: usize, by: usize| -> u8 {
            if bx >= m.ctb_w || by >= m.ctb_h {
                return 0;
            }
            // SAFETY: the block's parameters are complete and only read now.
            let p = unsafe { m.sao.at(by * m.ctb_w + bx) };
            (0..3).map(|c| ((p[c].type_idx == 2) as u8) << c).sum()
        };
        // SAFETY: as above.
        let own = unsafe { m.sao.at(cy * m.ctb_w + cx) };
        let own: u8 = (0..3).map(|c| ((own[c].type_idx != 0) as u8) << c).sum();
        let below = edge(cx.wrapping_sub(1), cy + 1) | edge(cx, cy + 1) | edge(cx + 1, cy + 1);
        let right = edge(cx + 1, cy.wrapping_sub(1)) | edge(cx + 1, cy) | edge(cx + 1, cy + 1);
        for c in 0..3usize {
            let bit = 1 << c;
            if (own | below | right) & bit != 0 {
                self.component(src, cx, cy, c, below & bit != 0, right & bit != 0);
            }
        }
    }

    /// Component `c` of coding tree block (`cx`, `cy`): its last line and
    /// column saved where a neighbour `below` or to the `right` will read
    /// them, then its own offset. Out of line, so that the blocks with
    /// nothing to do cost no more than the check above.
    #[inline(never)]
    fn component(&self, src: &mut [u8], cx: usize, cy: usize, c: usize, below: bool, right: bool) {
        let m = &self.maps;
        let ctb = 1usize << m.log2_ctb;
        let (pw, ph) = (m.width, m.height);
        let (x0, y0) = (cx * ctb, cy * ctb);
        let (x1, y1) = ((x0 + ctb).min(pw), (y0 + ctb).min(ph));
        let (w, h) = (x1 - x0, y1 - y0);
        // SAFETY: the block's parameters are complete and only read now.
        let prm = unsafe { m.sao.at(cy * m.ctb_w + cx) }[c];
        let pl = self.planes[c];
        let stride = pl.stride;
        // SAFETY: this block is final and nothing else touches it now; the
        // deblocked line below and column to the right it reads are too
        // (see `shared`).
        let block = unsafe { pl.block_mut(x0, y0, w, h) };
        // The last line and column as deblocked, for the neighbours.
        // SAFETY: this block's own entries.
        let (last_row, last_col) = unsafe { (m.sao_rows.slice_mut((c * m.ctb_h + cy) * pw + x0, w), m.sao_cols.slice_mut((c * m.ctb_w + cx) * ph + y0, h)) };
        if below {
            last_row.copy_from_slice(&block[(h - 1) * stride..(h - 1) * stride + w]);
        }
        if right {
            for (k, v) in last_col.iter_mut().enumerate() {
                *v = block[k * stride + w - 1];
            }
        }
        match prm.type_idx {
            1 => {
                let mut band = [0i8; 32];
                for k in 0..4usize {
                    band[(k + prm.aux as usize) & 31] = prm.offset[k];
                }
                kernels::sao_band(block, stride, w, h, &band);
            }
            2 => {
                let (da, db) = match prm.aux {
                    0 => ((-1isize, 0isize), (1isize, 0isize)),
                    1 => ((0, -1), (0, 1)),
                    2 => ((-1, -1), (1, 1)),
                    _ => ((1, -1), (-1, 1)),
                };
                // A sample whose neighbour for this class would be
                // outside the picture keeps its value.
                let (ix0, ix1) = if da.0 == 0 { (x0, x1) } else { (x0.max(1), x1.min(pw - 1)) };
                let (iy0, iy1) = if da.1 == 0 { (y0, y1) } else { (y0.max(1), y1.min(ph - 1)) };
                if ix0 >= ix1 || iy0 >= iy1 {
                    return;
                }
                // The block with its border as deblocked: the line above
                // and the column to the left from what those blocks saved,
                // the rest from the picture.
                const S: usize = SRC_STRIDE;
                kernels::copy_block(&mut src[S + 1..], S, &block[..], stride, w, h);
                let (l, r) = ((x0 > 0) as usize, (x1 < pw) as usize);
                if y0 > 0 {
                    // SAFETY: the row above's entries, saved before this.
                    let above = unsafe { m.sao_rows.slice((c * m.ctb_h + cy - 1) * pw + x0 - l, w + l + r) };
                    src[1 - l..1 + w + r].copy_from_slice(above);
                }
                if y1 < ph {
                    // SAFETY: the line below, deblocked, which its own
                    // block has yet to reach.
                    let line = unsafe { pl.row(x0 - l, y1, w + l + r) };
                    src[(h + 1) * S + 1 - l..(h + 1) * S + 1 + w + r].copy_from_slice(line);
                }
                if l == 1 {
                    // SAFETY: the column to the left's entries, saved before this.
                    let left = unsafe { m.sao_cols.slice((c * m.ctb_w + cx - 1) * ph + y0, h) };
                    for (k, &v) in left.iter().enumerate() {
                        src[(k + 1) * S] = v;
                    }
                }
                if r == 1 {
                    for k in 0..h {
                        // SAFETY: the column to the right, as the line below.
                        src[(k + 1) * S + w + 1] = unsafe { pl.get(x1, y0 + k) };
                    }
                }
                let origin = (iy0 - y0 + 1) * S + (ix0 - x0 + 1);
                kernels::sao_edge(&mut block[(iy0 - y0) * stride + ix0 - x0..], stride, src, origin, S, ix1 - ix0, iy1 - iy0, da.1 * S as isize + da.0, db.1 * S as isize + db.0, &prm.offset);
            }
            _ => {}
        }
    }
}
