//! Sample adaptive offset (§8.7.3): from the deblocked picture into the
//! output one, a coding tree block row at a time.

use crate::ctu::Maps;
use crate::kernels;
use crate::pic::Plane;
use crate::shared::PlanePtr;

pub struct SaoCtx<'a> {
    /// The deblocked picture.
    pub src: [&'a Plane; 3],
    /// The picture being made.
    pub dst: [PlanePtr; 3],
    pub maps: Maps,
}

impl<'a> SaoCtx<'a> {
    /// Coding tree block row `row` of every component.
    pub fn row(&self, row: usize) {
        let m = &self.maps;
        let ctb = 1usize << m.log2_ctb;
        let (pw, ph) = (m.width, m.height);
        let y0 = row * ctb;
        let y1 = (y0 + ctb).min(ph);
        for rx in 0..m.ctb_w {
            // SAFETY: the maps are complete and only read now.
            let params = unsafe { m.sao.get(row * m.ctb_w + rx) };
            let x0 = rx * ctb;
            let x1 = (x0 + ctb).min(pw);
            let (w, h) = (x1 - x0, y1 - y0);
            for c in 0..3usize {
                let src = self.src[c];
                let dst = self.dst[c];
                let stride = src.stride;
                // SAFETY: this coding tree block row is this thread's.
                let out = unsafe { dst.block_mut(0, 0, pw, ph) };
                let prm = params[c];
                match prm.type_idx {
                    1 => {
                        let mut band = [0i8; 32];
                        for k in 0..4usize {
                            band[(k + prm.aux as usize) & 31] = prm.offset[k];
                        }
                        kernels::sao_band(out, &src.data, stride, x0, y0, w, h, &band);
                    }
                    2 => {
                        let (da, db) = match prm.aux {
                            0 => ((-1i32, 0i32), (1i32, 0i32)),
                            1 => ((0, -1), (0, 1)),
                            2 => ((-1, -1), (1, 1)),
                            _ => ((1, -1), (-1, 1)),
                        };
                        // A sample whose neighbour for this class would be
                        // outside the picture keeps its value.
                        let (ix0, ix1) = if da.0 == 0 { (x0, x1) } else { (x0.max(1), x1.min(pw - 1)) };
                        let (iy0, iy1) = if da.1 == 0 { (y0, y1) } else { (y0.max(1), y1.min(ph - 1)) };
                        for y in y0..y1 {
                            let inner = y >= iy0 && y < iy1;
                            let (a, b) = if inner { (ix0, ix1) } else { (x1, x1) };
                            out[y * stride + x0..y * stride + a].copy_from_slice(&src.data[y * stride + x0..y * stride + a]);
                            out[y * stride + b..y * stride + x1].copy_from_slice(&src.data[y * stride + b..y * stride + x1]);
                        }
                        if ix0 < ix1 && iy0 < iy1 {
                            kernels::sao_edge(out, &src.data, stride, ix0, iy0, ix1 - ix0, iy1 - iy0, da, db, &prm.offset);
                        }
                    }
                    _ => kernels::copy_block(&mut out[y0 * stride + x0..], stride, &src.data[y0 * stride + x0..], stride, w, h),
                }
            }
        }
    }
}
