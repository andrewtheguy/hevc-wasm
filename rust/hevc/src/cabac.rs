//! CABAC (§9.3): the arithmetic decoding engine and the context models of the
//! syntax this decoder reads.

/// `rangeTabLps` (Table 9-46).
#[rustfmt::skip]
const RANGE_LPS: [[u8; 4]; 64] = [
    [128, 176, 208, 240], [128, 167, 197, 227], [128, 158, 187, 216], [123, 150, 178, 205],
    [116, 142, 169, 195], [111, 135, 160, 185], [105, 128, 152, 175], [100, 122, 144, 166],
    [95, 116, 137, 158], [90, 110, 130, 150], [85, 104, 123, 142], [81, 99, 117, 135],
    [77, 94, 111, 128], [73, 89, 105, 122], [69, 85, 100, 116], [66, 80, 95, 110],
    [62, 76, 90, 104], [59, 72, 86, 99], [56, 69, 81, 94], [53, 65, 77, 89],
    [51, 62, 73, 85], [48, 59, 69, 80], [46, 56, 66, 76], [43, 53, 63, 72],
    [41, 50, 59, 69], [39, 48, 56, 65], [37, 45, 54, 62], [35, 43, 51, 59],
    [33, 41, 48, 56], [32, 39, 46, 53], [30, 37, 43, 50], [29, 35, 41, 48],
    [27, 33, 39, 45], [26, 31, 37, 43], [24, 30, 35, 41], [23, 28, 33, 39],
    [22, 27, 32, 37], [21, 26, 30, 35], [20, 24, 29, 33], [19, 23, 27, 31],
    [18, 22, 26, 30], [17, 21, 25, 28], [16, 20, 23, 27], [15, 19, 22, 25],
    [14, 18, 21, 24], [14, 17, 20, 23], [13, 16, 19, 22], [12, 15, 18, 21],
    [12, 14, 17, 20], [11, 14, 16, 19], [11, 13, 15, 18], [10, 12, 15, 17],
    [10, 12, 14, 16], [9, 11, 13, 15], [9, 11, 12, 14], [8, 10, 12, 14],
    [8, 9, 11, 13], [7, 9, 11, 12], [7, 9, 10, 12], [7, 8, 10, 11],
    [6, 8, 9, 11], [6, 7, 9, 10], [6, 7, 8, 9], [2, 2, 2, 2],
];

/// `[transIdxLps, transIdxMps]` by state (Table 9-47).
#[rustfmt::skip]
const STATE_TRANS: [[u8; 2]; 64] = [
    [0, 1], [0, 2], [1, 3], [2, 4], [2, 5], [4, 6], [4, 7], [5, 8], [6, 9], [7, 10], [8, 11], [9, 12],
    [9, 13], [11, 14], [11, 15], [12, 16], [13, 17], [13, 18], [15, 19], [15, 20], [16, 21], [16, 22],
    [18, 23], [18, 24], [19, 25], [19, 26], [21, 27], [21, 28], [22, 29], [22, 30], [23, 31], [24, 32],
    [24, 33], [25, 34], [26, 35], [26, 36], [27, 37], [27, 38], [28, 39], [29, 40], [29, 41], [30, 42],
    [30, 43], [30, 44], [31, 45], [32, 46], [32, 47], [33, 48], [33, 49], [33, 50], [34, 51], [34, 52],
    [35, 53], [35, 54], [35, 55], [36, 56], [36, 57], [36, 58], [37, 59], [37, 60], [37, 61], [38, 62],
    [38, 62], [63, 63],
];

// The context models, as offsets into one array: the syntax of I and P
// slices without the tools the Mac leaves off.
pub const CTX_SAO_MERGE: usize = 0; // 1
pub const CTX_SAO_TYPE: usize = 1; // 1
pub const CTX_SPLIT_CU: usize = 2; // 3
pub const CTX_CU_SKIP: usize = 5; // 3
pub const CTX_CU_QP_DELTA: usize = 8; // 2
pub const CTX_PRED_MODE: usize = 10; // 1
pub const CTX_PART_MODE: usize = 11; // 3 (the fourth is AMP's)
pub const CTX_PREV_INTRA_LUMA_PRED: usize = 14; // 1
pub const CTX_INTRA_CHROMA_PRED_MODE: usize = 15; // 1
pub const CTX_MERGE_FLAG: usize = 16; // 1
pub const CTX_MERGE_IDX: usize = 17; // 1
pub const CTX_REF_IDX: usize = 18; // 2
pub const CTX_MVD_GT0: usize = 20; // 1
pub const CTX_MVD_GT1: usize = 21; // 1
pub const CTX_MVP_FLAG: usize = 22; // 1
pub const CTX_RQT_ROOT_CBF: usize = 23; // 1
pub const CTX_SPLIT_TRANSFORM: usize = 24; // 3
pub const CTX_CBF_LUMA: usize = 27; // 2
pub const CTX_CBF_CHROMA: usize = 29; // 5
pub const CTX_LAST_X_PREFIX: usize = 34; // 18
pub const CTX_LAST_Y_PREFIX: usize = 52; // 18
pub const CTX_CSBF: usize = 70; // 4 (luma 2, chroma 2)
pub const CTX_SIG: usize = 74; // 42 (luma 27, chroma 15)
pub const CTX_GT1: usize = 116; // 24 (luma 16, chroma 8)
pub const CTX_GT2: usize = 140; // 6 (luma 4, chroma 2)
pub const NUM_CTX: usize = 146;

const CNU: u8 = 154;
/// `initValue` per context for `initType` 0 (I slices) and 1 (P slices with
/// `cabac_init_flag` 0), Tables 9-5 to 9-37.
#[rustfmt::skip]
static INIT_VALUES: [[u8; NUM_CTX]; 2] = [
    [
        153, 200, 139, 141, 157, CNU, CNU, CNU, 154, 154, CNU, 184, CNU, CNU, 184, 63, CNU, CNU, CNU, CNU, CNU, CNU, CNU, CNU,
        153, 138, 138, 111, 141, 94, 138, 182, 154, 154,
        110, 110, 124, 125, 140, 153, 125, 127, 140, 109, 111, 143, 127, 111, 79, 108, 123, 63,
        110, 110, 124, 125, 140, 153, 125, 127, 140, 109, 111, 143, 127, 111, 79, 108, 123, 63,
        91, 171, 134, 141,
        111, 111, 125, 110, 110, 94, 124, 108, 124, 107, 125, 141, 179, 153, 125, 107, 125, 141, 179, 153, 125, 107, 125, 141, 179, 153, 125,
        140, 139, 182, 182, 152, 136, 152, 136, 153, 136, 139, 111, 136, 139, 111,
        140, 92, 137, 138, 140, 152, 138, 139, 153, 74, 149, 92, 139, 107, 122, 152,
        140, 179, 166, 182, 140, 227, 122, 197,
        138, 153, 136, 167, 152, 152,
    ],
    [
        153, 185, 107, 139, 126, 197, 185, 201, 154, 154, 149, 154, 139, 154, 154, 152, 110, 122, 153, 153, 140, 198, 168, 79,
        124, 138, 94, 153, 111, 149, 107, 167, 154, 154,
        125, 110, 94, 110, 95, 79, 125, 111, 110, 78, 110, 111, 111, 95, 94, 108, 123, 108,
        125, 110, 94, 110, 95, 79, 125, 111, 110, 78, 110, 111, 111, 95, 94, 108, 123, 108,
        121, 140, 61, 154,
        155, 154, 139, 153, 139, 123, 123, 63, 153, 166, 183, 140, 136, 153, 154, 166, 183, 140, 136, 153, 154, 166, 183, 140, 136, 153, 154,
        170, 153, 123, 123, 107, 121, 107, 121, 167, 151, 183, 140, 151, 183, 140,
        154, 196, 196, 167, 154, 152, 167, 182, 182, 134, 149, 136, 153, 121, 136, 137,
        169, 194, 166, 167, 154, 167, 137, 182,
        107, 167, 91, 122, 107, 167,
    ],
];

/// Per (model byte, range quartile): `lps | transMps << 8 | transLps << 16`,
/// with a model byte being `pStateIdx * 2 + valMps` and the state-0 flip of the
/// MPS folded in.
const fn build_fused() -> [u32; 128 * 4] {
    let mut t = [0u32; 128 * 4];
    let mut s = 0;
    while s < 128 {
        let p = s >> 1;
        let mps = (s & 1) as u8;
        let mut q = 0;
        while q < 4 {
            let lps = RANGE_LPS[p][q] as u32;
            let tm = ((STATE_TRANS[p][1] << 1) | mps) as u32;
            let new_mps = if p == 0 { 1 - mps } else { mps };
            let tl = ((STATE_TRANS[p][0] << 1) | new_mps) as u32;
            t[s * 4 + q] = lps | (tm << 8) | (tl << 16);
            q += 1;
        }
        s += 1;
    }
    t
}
static FUSED: [u32; 128 * 4] = build_fused();

/// Bit position of `ivlOffset` inside [`Cabac::low`].
const OFF: u32 = 41;
const REFILL_AT: i32 = 8;

/// The context models alone: what the wavefront hands from one row to the next.
#[derive(Clone)]
pub struct Contexts(pub [u8; NUM_CTX]);

impl Contexts {
    /// §9.3.2.2 for `init_type` 0 (I) or 1 (P) at `SliceQpY`.
    pub fn init(init_type: usize, slice_qp: i32) -> Self {
        let q = slice_qp.clamp(0, 51);
        let mut ctx = [0u8; NUM_CTX];
        for (c, &init_value) in ctx.iter_mut().zip(&INIT_VALUES[init_type]) {
            let init_value = init_value as i32;
            let m = (init_value >> 4) * 5 - 45;
            let n = ((init_value & 15) << 3) - 16;
            let pre = (((m * q) >> 4) + n).clamp(1, 126);
            *c = if pre <= 63 { ((63 - pre) as u8) << 1 } else { (((pre - 64) as u8) << 1) | 1 };
        }
        Contexts(ctx)
    }
}

/// The arithmetic decoder over one substream of a slice's data.
pub struct Cabac<'a> {
    data: &'a [u8],
    byte_pos: usize,
    /// `ivlOffset << 41`, with the bits not yet consumed below.
    low: u64,
    cnt: i32,
    range: u32,
    pub ctx: Contexts,
}

impl<'a> Cabac<'a> {
    /// The engine at byte `start` of `data` (§9.3.2.5), with the models `ctx`.
    pub fn new(data: &'a [u8], start: usize, ctx: Contexts) -> Self {
        let mut e = Cabac { data, byte_pos: start, low: 0, cnt: 0, range: 510, ctx };
        e.reinit_at(start);
        e
    }

    /// Restart the arithmetic registers at byte `byte`, keeping the models.
    pub fn reinit_at(&mut self, byte: usize) {
        self.byte_pos = byte;
        self.low = 0;
        self.cnt = 0;
        self.range = 510;
        self.refill();
        self.low <<= 9;
        self.cnt -= 9;
    }

    #[cold]
    #[inline(never)]
    fn refill_tail(&self) -> u32 {
        let b = |i: usize| self.data.get(self.byte_pos + i).copied().unwrap_or(0) as u32;
        (b(0) << 24) | (b(1) << 16) | (b(2) << 8) | b(3)
    }

    #[inline]
    fn refill(&mut self) {
        let v = match self.data.get(self.byte_pos..self.byte_pos + 4) {
            Some(c) => u32::from_be_bytes([c[0], c[1], c[2], c[3]]),
            None => self.refill_tail(),
        };
        self.low |= (v as u64) << ((OFF as i32 - 32 - self.cnt) as u32);
        self.byte_pos += 4;
        self.cnt += 32;
    }

    #[inline(always)]
    fn renorm(&mut self) {
        let n = self.range.leading_zeros() - 23;
        self.range <<= n;
        self.low <<= n;
        self.cnt -= n as i32;
        if self.cnt < REFILL_AT {
            self.refill();
        }
    }

    /// A context-coded bin (§9.3.4.3.2).
    #[inline(always)]
    pub fn decode(&mut self, ctx_idx: usize) -> u32 {
        let s = self.ctx.0[ctx_idx] as usize;
        let q = ((self.range >> 6) & 3) as usize;
        let e = FUSED[((s & 127) << 2) | q];
        let lps = e & 0xFF;
        self.range -= lps;
        let scaled = (self.range as u64) << OFF;
        let mask64 = ((scaled as i64 - self.low as i64 - 1) >> 63) as u64;
        let mask = mask64 as u32;
        self.low -= scaled & mask64;
        self.range = self.range.wrapping_add(lps.wrapping_sub(self.range) & mask);
        self.ctx.0[ctx_idx] = ((e >> (8 + (mask & 8))) & 0xFF) as u8;
        let bin = (s as u32 ^ mask) & 1;
        self.renorm();
        bin
    }

    #[inline(always)]
    fn bypass_cmp(low: u64, scaled: u64) -> (u64, u32) {
        let d = low.wrapping_sub(scaled);
        let m = ((d as i64) >> 63) as u64;
        (d.wrapping_add(scaled & m), (!m as u32) & 1)
    }

    /// A bypass bin (§9.3.4.3.4).
    #[inline(always)]
    pub fn bypass(&mut self) -> u32 {
        self.low <<= 1;
        self.cnt -= 1;
        if self.cnt < REFILL_AT {
            self.refill();
        }
        let (low, bin) = Self::bypass_cmp(self.low, (self.range as u64) << OFF);
        self.low = low;
        bin
    }

    /// `n` bypass bins, most significant first.
    #[inline]
    pub fn bypass_bits(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let scaled = (self.range as u64) << OFF;
        let (mut low, mut cnt) = (self.low, self.cnt);
        let mut v = 0u32;
        for _ in 0..n {
            low <<= 1;
            cnt -= 1;
            if cnt < REFILL_AT {
                self.low = low;
                self.cnt = cnt;
                self.refill();
                low = self.low;
                cnt = self.cnt;
            }
            let (l, bin) = Self::bypass_cmp(low, scaled);
            low = l;
            v = (v << 1) | bin;
        }
        self.low = low;
        self.cnt = cnt;
        v
    }

    /// Bypass bins up to the first 0, or `max` ones: how many were 1.
    #[inline]
    pub fn bypass_ones(&mut self, max: u32) -> u32 {
        let scaled = (self.range as u64) << OFF;
        let (mut low, mut cnt) = (self.low, self.cnt);
        let mut k = 0;
        while k < max {
            low <<= 1;
            cnt -= 1;
            if cnt < REFILL_AT {
                self.low = low;
                self.cnt = cnt;
                self.refill();
                low = self.low;
                cnt = self.cnt;
            }
            if low < scaled {
                break;
            }
            low -= scaled;
            k += 1;
        }
        self.low = low;
        self.cnt = cnt;
        k
    }

    /// The terminate bin (§9.3.4.3.5): true at the end of a substream.
    #[inline(always)]
    pub fn terminate(&mut self) -> bool {
        self.range -= 2;
        if self.low >= (self.range as u64) << OFF {
            true
        } else {
            self.renorm();
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_matches_the_spec_examples() {
        let c = Contexts::init(0, 26);
        // initValue 139 (split_cu_flag ctx 0): slope 8, offset 11: pre 63, state 0, mps 0.
        assert_eq!(c.0[CTX_SPLIT_CU], 0);
        // initValue 200 (sao_type_idx): pre 72, state 8, mps 1.
        assert_eq!(c.0[CTX_SAO_TYPE], (8 << 1) | 1);
        let mut e = Cabac::new(&[0xFF, 0xFF, 0xFF, 0xFF], 0, Contexts::init(0, 30));
        assert!(e.terminate());
    }
}
