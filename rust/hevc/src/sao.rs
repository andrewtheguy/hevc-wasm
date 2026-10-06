//! Sample adaptive offset (§8.7.3), in place, a coding tree block at a time
//! once its neighbours are deblocked (see `ctu::decode_row`). The edge
//! offset reads one deblocked sample into each neighbour; the blocks above
//! and to the left are filtered by then and the others not yet, so every
//! block saves its last line and column before filtering itself, for the
//! blocks below and to the right to read.

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
    /// thread's block-with-border.
    pub fn ctb(&self, src: &mut [u8], cx: usize, cy: usize) {
        let m = &self.maps;
        let ctb = 1usize << m.log2_ctb;
        let (pw, ph) = (m.width, m.height);
        let (x0, y0) = (cx * ctb, cy * ctb);
        let (x1, y1) = ((x0 + ctb).min(pw), (y0 + ctb).min(ph));
        let (w, h) = (x1 - x0, y1 - y0);
        // SAFETY: the block's parameters are complete and only read now.
        let params = unsafe { m.sao.get(cy * m.ctb_w + cx) };
        for c in 0..3usize {
            let pl = self.planes[c];
            let stride = pl.stride;
            // SAFETY: this block and the deblocked samples round it are final
            // and nothing else touches them now (see `shared`).
            let frame = unsafe { pl.block_mut(0, 0, pw, ph) };
            // The last line and column as deblocked, for the neighbours.
            // SAFETY: this block's own entries.
            let (last_row, last_col) = unsafe { (m.sao_rows.slice_mut((c * m.ctb_h + cy) * pw + x0, w), m.sao_cols.slice_mut((c * m.ctb_w + cx) * ph + y0, h)) };
            last_row.copy_from_slice(&frame[(y1 - 1) * stride + x0..(y1 - 1) * stride + x1]);
            for (k, v) in last_col.iter_mut().enumerate() {
                *v = frame[(y0 + k) * stride + x1 - 1];
            }
            let prm = params[c];
            match prm.type_idx {
                1 => {
                    let mut band = [0i8; 32];
                    for k in 0..4usize {
                        band[(k + prm.aux as usize) & 31] = prm.offset[k];
                    }
                    kernels::sao_band(&mut frame[y0 * stride + x0..], stride, w, h, &band);
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
                        continue;
                    }
                    // The block with its border as deblocked: the line above
                    // and the column to the left from what those blocks saved,
                    // the rest from the picture.
                    const S: usize = SRC_STRIDE;
                    kernels::copy_block(&mut src[S + 1..], S, &frame[y0 * stride + x0..], stride, w, h);
                    let (l, r) = ((x0 > 0) as usize, (x1 < pw) as usize);
                    if y0 > 0 {
                        // SAFETY: the row above's entries, saved before this.
                        let above = unsafe { m.sao_rows.slice((c * m.ctb_h + cy - 1) * pw + x0 - l, w + l + r) };
                        src[1 - l..1 + w + r].copy_from_slice(above);
                    }
                    if y1 < ph {
                        src[(h + 1) * S + 1 - l..(h + 1) * S + 1 + w + r].copy_from_slice(&frame[y1 * stride + x0 - l..y1 * stride + x1 + r]);
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
                            src[(k + 1) * S + w + 1] = frame[(y0 + k) * stride + x1];
                        }
                    }
                    let origin = (iy0 - y0 + 1) * S + (ix0 - x0 + 1);
                    kernels::sao_edge(&mut frame[iy0 * stride + ix0..], stride, src, origin, S, ix1 - ix0, iy1 - iy0, da.1 * S as isize + da.0, db.1 * S as isize + db.0, &prm.offset);
                }
                _ => {}
            }
        }
    }
}
