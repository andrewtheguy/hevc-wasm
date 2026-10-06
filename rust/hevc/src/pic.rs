//! Pictures, and the per-block state of the one being decoded.

use std::sync::Arc;

use crate::ps::Sps;

/// One plane of 8-bit samples, rows `stride` bytes apart.
pub struct Plane {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub data: Vec<u8>,
}

impl Plane {
    pub fn new(width: usize, height: usize) -> Self {
        // Rows start 64 bytes apart at least, for the kernels' vectors.
        let stride = width.div_ceil(64) * 64;
        Plane { width, height, stride, data: vec![0; stride * height] }
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> u8 {
        self.data[y * self.stride + x]
    }
}

/// A decoded picture at the coded size: luma, Cb, Cr, all the same size.
pub struct Picture {
    pub planes: [Plane; 3],
    pub poc: i32,
}

impl Picture {
    pub fn new(width: usize, height: usize) -> Self {
        Picture { planes: [Plane::new(width, height), Plane::new(width, height), Plane::new(width, height)], poc: 0 }
    }

    pub fn width(&self) -> usize {
        self.planes[0].width
    }

    pub fn height(&self) -> usize {
        self.planes[0].height
    }
}

/// `CuPredMode` per 4×4.
pub const PRED_INTRA: u8 = 1;
pub const PRED_INTER: u8 = 2;
pub const PRED_SKIP: u8 = 3;

/// A 4×4 block's motion: its vector in quarter samples and its reference.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Motion {
    pub mv: [i16; 2],
    pub ref_idx: i8,
}

/// One coding tree block's SAO, for one component.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SaoParams {
    /// 0 off, 1 band, 2 edge.
    pub type_idx: u8,
    /// The band position, or the edge class.
    pub aux: u8,
    /// Band: the four bands from `aux`; edge: categories 1 to 4.
    pub offset: [i8; 4],
}

/// Per-4×4 and per-coding-tree-block maps of the picture being decoded.
pub struct PicState {
    pub width: usize,
    pub height: usize,
    /// Size in 4×4 blocks.
    pub w4: usize,
    pub log2_ctb: usize,
    pub ctb_w: usize,
    pub ctb_h: usize,
    /// `MinTbAddrZs` per 4×4 (§6.5.2): the decoding order of the blocks, which
    /// is what decides a neighbour's availability.
    pub zs: Arc<[u32]>,
    pub pred_mode: Vec<u8>,
    /// Luma intra mode.
    pub intra_mode: Vec<u8>,
    pub qp_y: Vec<i8>,
    pub ct_depth: Vec<u8>,
    /// Bit 0: the left edge is a transform block edge; bit 1: the top edge is;
    /// bits 2 and 3 the same for prediction block edges.
    pub edges: Vec<u8>,
    /// The luma transform block has a non-zero coefficient.
    pub nz: Vec<u8>,
    pub motion: Vec<Motion>,
    /// Per coding tree block: SAO for Y, Cb, Cr.
    pub sao: Vec<[SaoParams; 3]>,
}

/// `MinTbAddrZs` at 4×4 granularity, raster order of the blocks in a picture
/// of `w4` by `h4` of them with coding tree blocks of `1 << log2_ctb`.
fn zscan(w4: usize, h4: usize, log2_ctb: usize, ctb_w: usize) -> Arc<[u32]> {
    let shift = log2_ctb - 2;
    let mut zs = vec![0u32; w4 * h4];
    for y in 0..h4 {
        for x in 0..w4 {
            let ctb_rs = (ctb_w * (y >> shift) + (x >> shift)) as u32;
            let mut v = ctb_rs << (shift * 2);
            for i in 0..shift {
                let m = 1usize << i;
                if m & x != 0 {
                    v += (m * m) as u32;
                }
                if m & y != 0 {
                    v += (2 * m * m) as u32;
                }
            }
            zs[y * w4 + x] = v;
        }
    }
    zs.into()
}

impl PicState {
    pub fn new(sps: &Sps) -> Self {
        let (width, height) = (sps.width as usize, sps.height as usize);
        let (w4, h4) = (width.div_ceil(4), height.div_ceil(4));
        let log2_ctb = sps.log2_ctb_size as usize;
        let (ctb_w, ctb_h) = (sps.pic_width_in_ctbs as usize, sps.pic_height_in_ctbs as usize);
        let n4 = w4 * h4;
        PicState {
            width,
            height,
            w4,
            log2_ctb,
            ctb_w,
            ctb_h,
            zs: zscan(w4, h4, log2_ctb, ctb_w),
            pred_mode: vec![0; n4],
            intra_mode: vec![1; n4],
            qp_y: vec![0; n4],
            ct_depth: vec![0; n4],
            edges: vec![0; n4],
            nz: vec![0; n4],
            motion: vec![Motion::default(); n4],
            sao: vec![[SaoParams::default(); 3]; ctb_w * ctb_h],
        }
    }

    /// Whether this state was built for `sps`'s geometry.
    pub fn fits(&self, sps: &Sps) -> bool {
        self.width == sps.width as usize && self.height == sps.height as usize && self.log2_ctb == sps.log2_ctb_size as usize
    }

    /// Clears what a picture does not write everywhere.
    pub fn reset(&mut self) {
        self.edges.fill(0);
        self.nz.fill(0);
    }
}
