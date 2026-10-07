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
/// MPS folded in. Sized for any byte, so the index needs no check; the upper
/// half is never read.
const fn build_fused() -> [u32; 256 * 4] {
    let mut t = [0u32; 256 * 4];
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
static FUSED: [u32; 256 * 4] = build_fused();

/// Bit position of `ivlOffset` inside [`Cabac::low`]. Under it sit the bits
/// read ahead, a 1 just under the last of them, and zeros under that: the
/// marker rises as the bits are used, and when it is within `REFILL_AT` of
/// `OFF` nothing is set under `MARK`, which is the test for a refill. One
/// register fewer than a count of the bits, and the arithmetic above `OFF`
/// never sees the marker.
const OFF: u32 = 41;
const REFILL_AT: u32 = 8;
const MARK: u64 = (1 << (OFF - REFILL_AT)) - 1;
/// `low` with no bits read ahead.
const EMPTY: u64 = 1 << (OFF - 1);

/// The allocated length of the models, so an index masked to it needs no
/// bounds check: the pad is never read or written.
pub const CTX_PAD: usize = 256;

/// The context models alone: what the wavefront hands from one row to the next.
#[derive(Clone)]
pub struct Contexts(pub [u8; CTX_PAD]);

impl Contexts {
    /// §9.3.2.2 for `init_type` 0 (I) or 1 (P) at `SliceQpY`.
    pub fn init(init_type: usize, slice_qp: i32) -> Self {
        let q = slice_qp.clamp(0, 51);
        let mut ctx = [0u8; CTX_PAD];
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

/// The arithmetic decoder's registers (§9.3.2.5), a value: what a function
/// decodes with, in locals of its own, through a [`View`].
#[derive(Clone, Copy)]
pub struct Engine {
    /// The next byte of the data to read ahead, by address: one register,
    /// where the data and an index would be three.
    next: *const u8,
    /// The end of the data.
    end: *const u8,
    /// `ivlOffset << OFF`, with the bits read ahead and their marker below.
    low: u64,
    range: u32,
}

/// The arithmetic decoder over one substream of a slice's data.
pub struct Cabac<'a> {
    data: &'a [u8],
    engine: Engine,
    pub ctx: Contexts,
}

impl<'a> Cabac<'a> {
    /// The engine at byte `start` of `data` (§9.3.2.5), with the models `ctx`.
    pub fn new(data: &'a [u8], start: usize, ctx: Contexts) -> Self {
        let mut e = Cabac { data, engine: Engine { next: data.as_ptr(), end: data.as_ptr_range().end, low: EMPTY, range: 510 }, ctx };
        e.reinit_at(start);
        e
    }

    /// Restart the arithmetic registers at byte `byte`, keeping the models.
    pub fn reinit_at(&mut self, byte: usize) {
        assert!(byte <= self.data.len());
        self.engine = Engine { next: self.data[byte..].as_ptr(), end: self.data.as_ptr_range().end, low: EMPTY, range: 510 };
        let mut v = self.view();
        v.refill();
        v.low <<= 9;
        self.engine = v.engine();
    }

    /// The decoder with its registers as values: what a hot loop decodes
    /// with, so the registers live in locals rather than in this struct. Put
    /// back with [`Self::restore`].
    #[inline(always)]
    pub fn view(&mut self) -> View<'_, 'a> {
        View::new(self.engine, &mut self.ctx)
    }

    #[inline(always)]
    pub fn restore(&mut self, v: Engine) {
        self.engine = v;
    }

    #[inline(always)]
    pub fn decode(&mut self, ctx_idx: usize) -> u32 {
        let mut v = self.view();
        let bin = v.decode(ctx_idx);
        self.engine = v.engine();
        bin
    }

    #[inline(always)]
    pub fn bypass(&mut self) -> u32 {
        let mut v = self.view();
        let bin = v.bypass();
        self.engine = v.engine();
        bin
    }

    #[inline]
    pub fn bypass_bits(&mut self, n: u32) -> u32 {
        let mut v = self.view();
        let bits = v.bypass_bits(n);
        self.engine = v.engine();
        bits
    }

    #[inline]
    pub fn bypass_ones(&mut self, max: u32) -> u32 {
        let mut v = self.view();
        let ones = v.bypass_ones(max);
        self.engine = v.engine();
        ones
    }

    #[inline(always)]
    pub fn terminate(&mut self) -> bool {
        let mut v = self.view();
        let end = v.terminate();
        self.engine = v.engine();
        end
    }
}

/// The decoder with its registers in hand: see [`Cabac::view`]. A local of
/// the function decoding, whose address never leaves it, so that the
/// registers are locals too.
pub struct View<'v, 'a> {
    next: *const u8,
    end: *const u8,
    pub ctx: &'v mut Contexts,
    low: u64,
    range: u32,
    _data: std::marker::PhantomData<&'a [u8]>,
}

impl<'v, 'a> View<'v, 'a> {
    #[inline(always)]
    pub fn new(e: Engine, ctx: &'v mut Contexts) -> Self {
        View { next: e.next, end: e.end, ctx, low: e.low, range: e.range, _data: std::marker::PhantomData }
    }

    #[inline(always)]
    pub fn engine(&self) -> Engine {
        Engine { next: self.next, end: self.end, low: self.low, range: self.range }
    }

    #[inline(always)]
    pub fn restore(&mut self, e: Engine) {
        self.next = e.next;
        self.low = e.low;
        self.range = e.range;
    }

    /// The next word of the data under the bits in hand, the marker moved
    /// under it. The data ends in [`RBSP_PAD`](crate::nal::RBSP_PAD) zeros,
    /// so a word is whole up to the end, and past it the standard's zeros;
    /// without a call, so that the registers stay in registers around it.
    #[inline(always)]
    fn refill(&mut self) {
        let v = if self.next as usize + 4 <= self.end as usize {
            // SAFETY: `next` starts inside the data and `next..next + 4` ends
            // by its end.
            u32::from_be(unsafe { self.next.cast::<u32>().read_unaligned() })
        } else {
            0
        };
        let p = self.low.trailing_zeros();
        self.low ^= 1 << p;
        self.low |= ((v as u64) << 1 | 1) << (p - 32);
        self.next = self.next.wrapping_add(4);
    }

    /// `low` shifted up by `n` with the refill it may need.
    #[inline(always)]
    fn consume(&mut self, n: u32) {
        self.low <<= n;
        if self.low & MARK == 0 {
            self.refill();
        }
    }

    /// A context-coded bin (§9.3.4.3.2). The most probable symbol is the
    /// one most bins are, and its path is short: the range shrinks by the
    /// other symbol's share, and doubling it at most puts it back.
    #[inline(always)]
    pub fn decode(&mut self, ctx_idx: usize) -> u32 {
        let ctx_idx = ctx_idx & (CTX_PAD - 1);
        let s = self.ctx.0[ctx_idx] as u32;
        let q = (self.range >> 6) & 3;
        let e = FUSED[((s << 2) | q) as usize];
        let lps = e & 0xFF;
        let range = self.range - lps;
        let scaled = (range as u64) << OFF;
        if self.low < scaled {
            self.ctx.0[ctx_idx] = (e >> 8) as u8;
            let under = ((range as i32 - 256) >> 31) as u32;
            self.range = range + (range & under);
            self.low += self.low & (under as i32 as i64 as u64);
            if self.low & MARK == 0 {
                self.refill();
            }
            s & 1
        } else {
            self.low -= scaled;
            self.ctx.0[ctx_idx] = (e >> 16) as u8;
            let n = lps.leading_zeros() - 23;
            self.range = lps << n;
            self.consume(n);
            (s & 1) ^ 1
        }
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
        self.consume(1);
        let (low, bin) = Self::bypass_cmp(self.low, (self.range as u64) << OFF);
        self.low = low;
        bin
    }

    /// `n` bypass bins, most significant first.
    #[inline]
    pub fn bypass_bits(&mut self, n: u32) -> u32 {
        let scaled = (self.range as u64) << OFF;
        let mut v = 0u32;
        for _ in 0..n {
            self.consume(1);
            let (l, bin) = Self::bypass_cmp(self.low, scaled);
            self.low = l;
            v = (v << 1) | bin;
        }
        v
    }

    /// Bypass bins up to the first 0, or `max` ones: how many were 1.
    #[inline]
    pub fn bypass_ones(&mut self, max: u32) -> u32 {
        let scaled = (self.range as u64) << OFF;
        let mut k = 0;
        while k < max {
            self.consume(1);
            if self.low < scaled {
                break;
            }
            self.low -= scaled;
            k += 1;
        }
        k
    }

    /// The terminate bin (§9.3.4.3.5): true at the end of a substream.
    #[inline(always)]
    pub fn terminate(&mut self) -> bool {
        self.range -= 2;
        if self.low >= (self.range as u64) << OFF {
            true
        } else {
            let n = self.range.leading_zeros() - 23;
            self.range <<= n;
            self.consume(n);
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

    /// The engine reads zeros past the data, however many bins are asked of
    /// it, from wherever in the data it starts: the end included.
    #[test]
    fn runs_out_of_data_into_zeros() {
        let data = [0x5A; 11];
        for start in [0, 3, 9, 10, 11] {
            let mut e = Cabac::new(&data, start, Contexts::init(1, 26));
            for i in 0..10_000 {
                e.decode(i % NUM_CTX);
                e.bypass_bits(7);
                e.bypass_ones(32);
                e.terminate();
            }
            // Past the end, the next word is whole zeros: every bypass bin 0.
            assert_eq!(e.bypass_bits(32), 0);
        }
    }
}
