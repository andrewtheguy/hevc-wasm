//! One coding tree block row of a picture, decoded from its own substream:
//! the syntax of §7.3.8 and the reconstruction it drives, for the Mac's shape
//! of stream.

mod inter;
mod residual;

use std::sync::{Arc, Mutex};

use crate::cabac::*;
use crate::error::{Error, Result};
use crate::intra::RefSamples;
use crate::pic::{Motion, PicState, Picture, SaoParams, PRED_INTER, PRED_INTRA, PRED_SKIP};
use crate::ps::{Pps, Sps};
use crate::shared::{MapPtr, PlanePtr};
use crate::slice::SliceHeader;
use crate::wavefront::Progress;

/// `part_mode` (Table 7-10) without the asymmetric ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartMode {
    Part2Nx2N,
    Part2NxN,
    PartNx2N,
    PartNxN,
}

/// A reference picture of the current one.
pub struct RefPic {
    pub pic: Arc<Picture>,
    pub poc: i32,
}

/// The per-block maps of the picture, as the rows share them.
#[derive(Clone, Copy)]
pub struct Maps {
    pub width: usize,
    pub height: usize,
    pub w4: usize,
    pub log2_ctb: usize,
    pub ctb_w: usize,
    pub ctb_h: usize,
    pub pred_mode: MapPtr<u8>,
    pub intra_mode: MapPtr<u8>,
    pub qp_y: MapPtr<i8>,
    pub ct_depth: MapPtr<u8>,
    pub edges: MapPtr<u8>,
    pub nz: MapPtr<u8>,
    pub motion: MapPtr<Motion>,
    pub sao: MapPtr<[SaoParams; 3]>,
}

impl Maps {
    pub fn of(st: &mut PicState) -> Self {
        Maps {
            width: st.width,
            height: st.height,
            w4: st.w4,
            log2_ctb: st.log2_ctb,
            ctb_w: st.ctb_w,
            ctb_h: st.ctb_h,
            pred_mode: MapPtr::of(&mut st.pred_mode),
            intra_mode: MapPtr::of(&mut st.intra_mode),
            qp_y: MapPtr::of(&mut st.qp_y),
            ct_depth: MapPtr::of(&mut st.ct_depth),
            edges: MapPtr::of(&mut st.edges),
            nz: MapPtr::of(&mut st.nz),
            motion: MapPtr::of(&mut st.motion),
            sao: MapPtr::of(&mut st.sao),
        }
    }

    #[inline]
    pub fn idx4(&self, x: usize, y: usize) -> usize {
        (y >> 2) * self.w4 + (x >> 2)
    }
}

/// What every row of a picture decodes against.
pub struct PictureCtx<'a> {
    pub sps: &'a Sps,
    pub pps: &'a Pps,
    pub sh: &'a SliceHeader,
    pub poc: i32,
    pub planes: [PlanePtr; 3],
    pub maps: Maps,
    pub zs: &'a [u32],
    /// `RefPicList0`.
    pub refs: &'a [RefPic],
    /// The slice's RBSP.
    pub data: &'a [u8],
    /// Where each row's substream starts in `data`.
    pub substreams: &'a [usize],
    /// How far each row has got, in coding tree blocks.
    pub progress: &'a [Progress],
    /// Each row's context models after its second coding tree block, for the
    /// row below to start from (§9.3.2.4).
    pub wpp_ctx: &'a [Mutex<Option<Contexts>>],
}

/// Buffers a row needs, kept by the thread across the rows it decodes.
pub struct Scratch {
    coeffs: Vec<i16>,
    itx_tmp: Vec<i16>,
    res: Vec<i16>,
    /// The motion compensation intermediate and the edge-extended footprint.
    mc_tmp: Vec<i16>,
    mc_pred: Vec<i16>,
    mc_pad: Vec<u8>,
    iref: RefSamples,
}

impl Default for Scratch {
    fn default() -> Self {
        Scratch {
            coeffs: vec![0; 32 * 32],
            itx_tmp: vec![0; 32 * 32],
            res: vec![0; 32 * 32],
            mc_tmp: vec![0; 64 * (64 + 7)],
            mc_pred: vec![0; 64 * 64],
            mc_pad: vec![0; (64 + 7) * (64 + 7)],
            iref: RefSamples::default(),
        }
    }
}

/// The decoding of one row.
pub struct Row<'a> {
    pic: &'a PictureCtx<'a>,
    s: &'a mut Scratch,
    cab: Cabac<'a>,
    // QP (§8.6.1)
    qp_y: i32,
    qp_y_pred: i32,
    last_cu_qp: i32,
    qp_prev_reset: bool,
    is_cu_qp_delta_coded: bool,
    cu_qp_delta_val: i32,
    // The coding unit being decoded.
    cu_x: usize,
    cu_y: usize,
    cu_size: usize,
    cu_intra: bool,
    part_mode: PartMode,
    intra_split: bool,
    max_trafo_depth: u8,
    /// `IntraPredModeC` of each quadrant: four under an NxN split, else one.
    intra_chroma_mode: [u8; 4],
}

/// Decodes row `row` of the picture into its planes and maps, waiting on the
/// row above as the wavefront requires.
pub fn decode_row(pic: &PictureCtx, s: &mut Scratch, row: usize) -> Result<()> {
    let m = &pic.maps;
    let ctb = 1usize << m.log2_ctb;
    let y0 = row * ctb;
    let start = pic.substreams[row];
    if start > pic.data.len() {
        return Err(Error::invalid("an entry point past the slice data"));
    }
    // §9.3.1: the models start as the row above left them after its second
    // coding tree block, or fresh where there is no such block.
    let ctx = if row > 0 && m.ctb_w >= 2 {
        if !pic.progress[row - 1].wait_for(2) {
            return Err(Error::invalid("the row above failed"));
        }
        let saved = pic.wpp_ctx[row - 1].lock().map_err(|_| Error::invalid("a poisoned row"))?;
        saved.clone().ok_or_else(|| Error::invalid("no models from the row above"))?
    } else {
        Contexts::init(if pic.sh.intra { 0 } else { 1 }, pic.sh.slice_qp)
    };
    let mut r = Row {
        pic,
        s,
        cab: Cabac::new(pic.data, start, ctx),
        qp_y: pic.sh.slice_qp,
        qp_y_pred: pic.sh.slice_qp,
        last_cu_qp: pic.sh.slice_qp,
        qp_prev_reset: true,
        is_cu_qp_delta_coded: false,
        cu_qp_delta_val: 0,
        cu_x: 0,
        cu_y: 0,
        cu_size: 0,
        cu_intra: true,
        part_mode: PartMode::Part2Nx2N,
        intra_split: false,
        max_trafo_depth: 0,
        intra_chroma_mode: [0; 4],
    };
    for x in 0..m.ctb_w {
        if row > 0 && !pic.progress[row - 1].wait_for((x + 2).min(m.ctb_w) as u32) {
            return Err(Error::invalid("the row above failed"));
        }
        r.decode_ctu(x * ctb, y0, row * m.ctb_w + x)?;
        if x == 1 {
            *pic.wpp_ctx[row].lock().map_err(|_| Error::invalid("a poisoned row"))? = Some(r.cab.ctx.clone());
        }
        let end_of_slice = r.cab.terminate();
        if end_of_slice != (row == m.ctb_h - 1 && x == m.ctb_w - 1) {
            return Err(Error::invalid("end_of_slice_segment_flag where the picture does not end"));
        }
        pic.progress[row].advance(x as u32 + 1);
    }
    Ok(())
}

impl<'a> Row<'a> {
    // ---- the maps, under the wavefront's contract (shared.rs) ----

    #[inline]
    fn map_get<T: Copy>(&self, map: MapPtr<T>, i: usize) -> T {
        // SAFETY: entry `i` is in this row or a finished part of the row above.
        unsafe { map.get(i) }
    }

    #[inline]
    fn fill4<T: Copy>(&self, map: MapPtr<T>, x: usize, y: usize, w: usize, h: usize, v: T) {
        // SAFETY: the rectangle is inside the coding unit, which is this row's.
        unsafe { map.fill_rect(self.pic.maps.w4, x, y, w, h, v) }
    }

    /// §6.4.1: whether the block at (`xn`, `yn`) is there for the one at
    /// (`xc`, `yc`) to use: inside the picture and decoded before it.
    #[inline]
    fn available(&self, xc: usize, yc: usize, xn: i32, yn: i32) -> bool {
        let m = &self.pic.maps;
        if xn < 0 || yn < 0 || xn as usize >= m.width || yn as usize >= m.height {
            return false;
        }
        self.pic.zs[m.idx4(xn as usize, yn as usize)] <= self.pic.zs[m.idx4(xc, yc)]
    }

    // ---- coding tree unit (§7.3.8.2) ----

    fn decode_ctu(&mut self, x0: usize, y0: usize, rs: usize) -> Result<()> {
        if self.pic.sh.sao_luma || self.pic.sh.sao_chroma {
            self.parse_sao(x0, y0, rs)?;
        }
        self.coding_quadtree(x0, y0, self.pic.sps.log2_ctb_size as usize, 0)
    }

    // ---- SAO syntax (§7.3.8.3) ----

    fn parse_sao(&mut self, x0: usize, y0: usize, rs: usize) -> Result<()> {
        let w = self.pic.maps.ctb_w;
        let merge_left = x0 > 0 && self.cab.decode(CTX_SAO_MERGE) == 1;
        let merge_up = !merge_left && y0 > 0 && self.cab.decode(CTX_SAO_MERGE) == 1;
        if merge_left || merge_up {
            let from = if merge_left { rs - 1 } else { rs - w };
            let params = self.map_get(self.pic.maps.sao, from);
            // SAFETY: this coding tree block's own entry.
            unsafe { self.pic.maps.sao.set(rs, params) };
            return Ok(());
        }
        let mut params = [SaoParams::default(); 3];
        for c in 0..3usize {
            if !(if c == 0 { self.pic.sh.sao_luma } else { self.pic.sh.sao_chroma }) {
                continue;
            }
            let type_idx = if c < 2 {
                if self.cab.decode(CTX_SAO_TYPE) == 0 {
                    0
                } else if self.cab.bypass() == 0 {
                    1
                } else {
                    2
                }
            } else {
                params[1].type_idx
            };
            params[c].type_idx = type_idx;
            if type_idx == 0 {
                continue;
            }
            // `sao_offset_abs` is truncated unary with cMax 7 at 8 bits.
            let mut abs = [0i32; 4];
            for a in abs.iter_mut() {
                *a = self.cab.bypass_ones(7) as i32;
            }
            if type_idx == 1 {
                for a in abs.iter_mut() {
                    if *a != 0 && self.cab.bypass() == 1 {
                        *a = -*a;
                    }
                }
                params[c].aux = self.cab.bypass_bits(5) as u8;
                for i in 0..4 {
                    params[c].offset[i] = abs[i] as i8;
                }
            } else {
                // The edge class of Cr is Cb's.
                params[c].aux = if c < 2 { self.cab.bypass_bits(2) as u8 } else { params[1].aux };
                params[c].offset = [abs[0] as i8, abs[1] as i8, -abs[2] as i8, -abs[3] as i8];
            }
        }
        // SAFETY: this coding tree block's own entry.
        unsafe { self.pic.maps.sao.set(rs, params) };
        Ok(())
    }

    // ---- coding quadtree (§7.3.8.4) ----

    fn coding_quadtree(&mut self, x0: usize, y0: usize, log2cb: usize, depth: u8) -> Result<()> {
        let m = self.pic.maps;
        let size = 1usize << log2cb;
        let min_cb = self.pic.sps.log2_min_cb_size as usize;
        let split = if x0 + size <= m.width && y0 + size <= m.height && log2cb > min_cb {
            let deeper = |s: &Self, xn: i32, yn: i32| s.available(x0, y0, xn, yn) && s.map_get(m.ct_depth, m.idx4(xn as usize, yn as usize)) > depth;
            let cond_l = deeper(self, x0 as i32 - 1, y0 as i32) as usize;
            let cond_a = deeper(self, x0 as i32, y0 as i32 - 1) as usize;
            self.cab.decode(CTX_SPLIT_CU + cond_l + cond_a) == 1
        } else {
            log2cb > min_cb
        };
        // §8.6.1: a quantization group starts at a quadtree node of exactly
        // `Log2MinCuQpDeltaSize`, or at a larger node that is a coding unit.
        let log2_min_qg = self.pic.sps.log2_ctb_size as usize - self.pic.pps.diff_cu_qp_delta_depth as usize;
        if log2cb == log2_min_qg || (log2cb > log2_min_qg && !split) {
            if self.pic.pps.cu_qp_delta_enabled {
                self.is_cu_qp_delta_coded = false;
                self.cu_qp_delta_val = 0;
            }
            let qp_prev = if self.qp_prev_reset { self.pic.sh.slice_qp } else { self.last_cu_qp };
            self.qp_prev_reset = false;
            let cur_ctb = (y0 >> m.log2_ctb) * m.ctb_w + (x0 >> m.log2_ctb);
            let in_ctb = |x: usize, y: usize| (y >> m.log2_ctb) * m.ctb_w + (x >> m.log2_ctb) == cur_ctb;
            let qp_a = if x0 > 0 && in_ctb(x0 - 1, y0) && self.available(x0, y0, x0 as i32 - 1, y0 as i32) { self.map_get(m.qp_y, m.idx4(x0 - 1, y0)) as i32 } else { qp_prev };
            let qp_b = if y0 > 0 && in_ctb(x0, y0 - 1) && self.available(x0, y0, x0 as i32, y0 as i32 - 1) { self.map_get(m.qp_y, m.idx4(x0, y0 - 1)) as i32 } else { qp_prev };
            self.qp_y_pred = (qp_a + qp_b + 1) >> 1;
        }
        if split {
            let half = size / 2;
            for (dx, dy) in [(0, 0), (half, 0), (0, half), (half, half)] {
                if x0 + dx < m.width && y0 + dy < m.height {
                    self.coding_quadtree(x0 + dx, y0 + dy, log2cb - 1, depth + 1)?;
                }
            }
            Ok(())
        } else {
            self.coding_unit(x0, y0, log2cb, depth)
        }
    }

    fn set_cu_qp(&mut self) {
        self.fill4(self.pic.maps.qp_y, self.cu_x, self.cu_y, self.cu_size, self.cu_size, self.qp_y as i8);
    }

    /// Marks the left and top edges of a block: `kind` 0 a transform block's,
    /// 1 a prediction block's, 2 a coding block's (both).
    fn mark_edges(&mut self, x: usize, y: usize, w: usize, h: usize, kind: u8) {
        let (bits_v, bits_h) = match kind {
            0 => (0b0001, 0b0010),
            1 => (0b0100, 0b1000),
            _ => (0b0101, 0b1010),
        };
        let m = self.pic.maps;
        let base = m.idx4(x, y);
        for r in 0..(h >> 2) {
            let i = base + r * m.w4;
            // SAFETY: the block is this row's.
            unsafe { m.edges.set(i, m.edges.get(i) | bits_v) };
        }
        for i in base..base + (w >> 2) {
            // SAFETY: as above.
            unsafe { m.edges.set(i, m.edges.get(i) | bits_h) };
        }
    }

    // ---- coding unit (§7.3.8.5) ----

    fn coding_unit(&mut self, x0: usize, y0: usize, log2cb: usize, depth: u8) -> Result<()> {
        let m = self.pic.maps;
        let n = 1usize << log2cb;
        self.cu_x = x0;
        self.cu_y = y0;
        self.cu_size = n;
        let mut skip = false;
        if !self.pic.sh.intra {
            let skipped = |s: &Self, xn: i32, yn: i32| s.available(x0, y0, xn, yn) && s.map_get(m.pred_mode, m.idx4(xn as usize, yn as usize)) == PRED_SKIP;
            let cond_l = skipped(self, x0 as i32 - 1, y0 as i32) as usize;
            let cond_a = skipped(self, x0 as i32, y0 as i32 - 1) as usize;
            skip = self.cab.decode(CTX_CU_SKIP + cond_l + cond_a) == 1;
        }
        self.qp_y = wrap_qp(self.qp_y_pred + self.cu_qp_delta_val);
        self.fill4(m.ct_depth, x0, y0, n, n, depth);
        self.set_cu_qp();
        self.mark_edges(x0, y0, n, n, 2);
        if skip {
            self.fill4(m.pred_mode, x0, y0, n, n, PRED_SKIP);
            self.fill4(m.intra_mode, x0, y0, n, n, 1);
            self.cu_intra = false;
            self.part_mode = PartMode::Part2Nx2N;
            self.prediction_unit(x0, y0, n, x0, y0, n, n, 0, true)?;
            self.last_cu_qp = self.qp_y;
            return Ok(());
        }
        self.cu_intra = self.pic.sh.intra || self.cab.decode(CTX_PRED_MODE) == 1;
        let min_cb = self.pic.sps.log2_min_cb_size as usize;
        self.part_mode = PartMode::Part2Nx2N;
        if !self.cu_intra || log2cb == min_cb {
            self.part_mode = self.parse_part_mode(log2cb);
        }
        self.fill4(m.pred_mode, x0, y0, n, n, if self.cu_intra { PRED_INTRA } else { PRED_INTER });
        self.intra_split = self.cu_intra && self.part_mode == PartMode::PartNxN;
        let mut merge_2nx2n = false;
        if self.cu_intra {
            self.parse_intra_modes(x0, y0, n)?;
        } else {
            merge_2nx2n = self.inter_prediction_units(x0, y0, n)?;
            self.fill4(m.intra_mode, x0, y0, n, n, 1);
        }
        let rqt_root_cbf = self.cu_intra || (self.part_mode == PartMode::Part2Nx2N && merge_2nx2n) || self.cab.decode(CTX_RQT_ROOT_CBF) == 1;
        if rqt_root_cbf {
            self.max_trafo_depth = if self.cu_intra { self.pic.sps.max_transform_hierarchy_depth_intra + self.intra_split as u8 } else { self.pic.sps.max_transform_hierarchy_depth_inter };
            self.transform_tree(x0, y0, log2cb, 0, true, true)?;
        }
        self.last_cu_qp = self.qp_y;
        Ok(())
    }

    fn parse_part_mode(&mut self, log2cb: usize) -> PartMode {
        if self.cab.decode(CTX_PART_MODE) == 1 {
            return PartMode::Part2Nx2N;
        }
        if self.cu_intra {
            return PartMode::PartNxN;
        }
        if self.cab.decode(CTX_PART_MODE + 1) == 1 {
            return PartMode::Part2NxN;
        }
        // At the minimum coding block size an inter block may split four
        // ways, unless it is 8×8.
        if log2cb == self.pic.sps.log2_min_cb_size as usize && log2cb > 3 && self.cab.decode(CTX_PART_MODE + 2) == 0 {
            return PartMode::PartNxN;
        }
        PartMode::PartNx2N
    }

    // ---- intra modes (§7.3.8.5, §8.4.2, §8.4.3) ----

    fn parse_intra_modes(&mut self, x0: usize, y0: usize, n: usize) -> Result<()> {
        let m = self.pic.maps;
        let parts = if self.part_mode == PartMode::PartNxN { 4 } else { 1 };
        let pb = if parts == 4 { n / 2 } else { n };
        let mut prev = [false; 4];
        for p in prev.iter_mut().take(parts) {
            *p = self.cab.decode(CTX_PREV_INTRA_LUMA_PRED) == 1;
        }
        for j in 0..parts {
            let (xp, yp) = (x0 + (j & 1) * pb, y0 + (j >> 1) * pb);
            let cand = self.mpm_candidates(xp, yp);
            let mode = if prev[j] {
                cand[self.cab.bypass_ones(2) as usize]
            } else {
                let mut mode = self.cab.bypass_bits(5) as u8;
                let mut c = cand;
                c.sort_unstable();
                for &cm in &c {
                    if mode >= cm {
                        mode += 1;
                    }
                }
                mode
            };
            self.fill4(m.intra_mode, xp, yp, pb, pb, mode);
        }
        if parts == 4 {
            self.mark_edges(x0 + pb, y0, pb, n, 1);
            self.mark_edges(x0, y0 + pb, n, pb, 1);
        }
        // Under 4:4:4 an NxN split codes a chroma mode per quadrant, each
        // derived from its own luma mode.
        for j in 0..parts {
            let (xp, yp) = (x0 + (j & 1) * pb, y0 + (j >> 1) * pb);
            let icpm = if self.cab.decode(CTX_INTRA_CHROMA_PRED_MODE) == 0 { 4 } else { self.cab.bypass_bits(2) as u8 };
            let luma = self.map_get(m.intra_mode, m.idx4(xp, yp));
            self.intra_chroma_mode[j] = match icpm {
                4 => luma,
                _ => {
                    let mode = [0u8, 26, 10, 1][icpm as usize];
                    if mode == luma {
                        34
                    } else {
                        mode
                    }
                }
            };
        }
        if parts == 1 {
            self.intra_chroma_mode = [self.intra_chroma_mode[0]; 4];
        }
        Ok(())
    }

    /// The three most probable modes of the block at (`xp`, `yp`) (§8.4.2).
    fn mpm_candidates(&self, xp: usize, yp: usize) -> [u8; 3] {
        let m = self.pic.maps;
        let cand = |xn: i32, yn: i32, above: bool| -> u8 {
            if !self.available(xp, yp, xn, yn) {
                return 1;
            }
            let i = m.idx4(xn as usize, yn as usize);
            if self.map_get(m.pred_mode, i) != PRED_INTRA {
                return 1;
            }
            // A neighbour above the coding tree block counts as DC.
            if above && (yn as usize) < ((yp >> m.log2_ctb) << m.log2_ctb) {
                return 1;
            }
            self.map_get(m.intra_mode, i)
        };
        let a = cand(xp as i32 - 1, yp as i32, false);
        let b = cand(xp as i32, yp as i32 - 1, true);
        if a == b {
            if a < 2 {
                [0, 1, 26]
            } else {
                [a, 2 + ((a + 29) % 32), 2 + ((a - 1) % 32)]
            }
        } else {
            let c = if a != 0 && b != 0 {
                0
            } else if a != 1 && b != 1 {
                1
            } else {
                26
            };
            [a, b, c]
        }
    }

    /// k-th order Exp-Golomb, bypass coded (§9.3.3.6).
    fn eg_k(&mut self, k: u32) -> Result<u32> {
        let room = 32 - k;
        let m = self.cab.bypass_ones(room);
        if m == room {
            return Err(Error::invalid("an Exp-Golomb prefix too long"));
        }
        Ok((((1u32 << m) - 1) << k) + self.cab.bypass_bits(k + m))
    }
}

/// `QpY` wrapped into `0..=51` (8.6.1, with `QpBdOffsetY` 0).
fn wrap_qp(v: i32) -> i32 {
    (v + 52).rem_euclid(52)
}
