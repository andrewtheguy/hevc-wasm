//! The specification's constant tables, built at compile time.

/// Up-right diagonal scan (§6.5.3) of a `SIZE`×`SIZE` block, as (x, y).
const fn diag_scan<const SIZE: usize, const N: usize>() -> [(u8, u8); N] {
    let mut out = [(0u8, 0u8); N];
    let (mut i, mut x, mut y) = (0usize, 0i32, 0i32);
    while i < N {
        while y >= 0 {
            if x < SIZE as i32 && y < SIZE as i32 {
                out[i] = (x as u8, y as u8);
                i += 1;
            }
            y -= 1;
            x += 1;
        }
        y = x;
        x = 0;
    }
    out
}

const fn horiz_scan<const SIZE: usize, const N: usize>() -> [(u8, u8); N] {
    let mut out = [(0u8, 0u8); N];
    let mut i = 0;
    while i < N {
        out[i] = ((i % SIZE) as u8, (i / SIZE) as u8);
        i += 1;
    }
    out
}

const fn vert_scan<const SIZE: usize, const N: usize>() -> [(u8, u8); N] {
    let mut out = [(0u8, 0u8); N];
    let mut i = 0;
    while i < N {
        out[i] = ((i / SIZE) as u8, (i % SIZE) as u8);
        i += 1;
    }
    out
}

/// `t[y * SIZE + x]` is the scan position of (x, y).
const fn invert<const SIZE: usize, const N: usize>(f: [(u8, u8); N]) -> [u8; N] {
    let mut t = [0u8; N];
    let mut i = 0;
    while i < N {
        t[(f[i].1 as usize) * SIZE + f[i].0 as usize] = i as u8;
        i += 1;
    }
    t
}

/// `t[k]` is `(max x + 1, max y + 1)` over scan positions `0..=k`.
const fn bbox<const N: usize>(f: [(u8, u8); N]) -> [(u8, u8); N] {
    let mut t = [(0u8, 0u8); N];
    let (mut w, mut h, mut i) = (0u8, 0u8, 0usize);
    while i < N {
        if f[i].0 + 1 > w {
            w = f[i].0 + 1;
        }
        if f[i].1 + 1 > h {
            h = f[i].1 + 1;
        }
        t[i] = (w, h);
        i += 1;
    }
    t
}

/// `ctxIdxMap` of `sig_coeff_flag` in a 4×4 block (§9.3.4.2.5), by (x, y).
const SIG_MAP_4X4: [u8; 16] = [0, 1, 4, 5, 2, 3, 4, 5, 6, 6, 8, 8, 7, 7, 8, 8];

/// [`SIG_MAP_4X4`] keyed by scan position.
const fn sig_by_scan(f: [(u8, u8); 16]) -> [u8; 16] {
    let mut t = [0u8; 16];
    let mut i = 0;
    while i < 16 {
        t[i] = SIG_MAP_4X4[(f[i].1 as usize) * 4 + f[i].0 as usize];
        i += 1;
    }
    t
}

/// The neighbour term of §9.3.4.2.5 for scan position `n` of a 4×4 sub-block
/// of a larger block, by `prevCsbf`.
const fn sig_nb(f: [(u8, u8); 16]) -> [[u8; 16]; 4] {
    let mut t = [[0u8; 16]; 4];
    let mut prev = 0;
    while prev < 4 {
        let mut i = 0;
        while i < 16 {
            let (xp, yp) = (f[i].0 as usize, f[i].1 as usize);
            t[prev][i] = match prev {
                0 => {
                    if xp + yp == 0 {
                        2
                    } else if xp + yp < 3 {
                        1
                    } else {
                        0
                    }
                }
                1 => {
                    if yp == 0 {
                        2
                    } else if yp == 1 {
                        1
                    } else {
                        0
                    }
                }
                2 => {
                    if xp == 0 {
                        2
                    } else if xp == 1 {
                        1
                    } else {
                        0
                    }
                }
                _ => 2,
            };
            i += 1;
        }
        prev += 1;
    }
    t
}

/// Everything the residual parser needs of one (sub-block grid size, scanIdx).
pub struct ScanSet {
    /// The sub-block scan, its inverse and its prefix bounding boxes.
    pub sb: &'static [(u8, u8)],
    pub sb_inv: &'static [u8],
    pub sb_bbox: &'static [(u8, u8)],
    /// The scan inside a 4×4 sub-block and its inverse.
    pub pos: &'static [(u8, u8); 16],
    pub pos_inv: &'static [u8; 16],
    pub sig_4x4: &'static [u8; 16],
    pub sig_nb: &'static [[u8; 16]; 4],
}

macro_rules! scans {
    ($name:ident, $size:literal, $n:literal) => {
        mod $name {
            use super::*;
            pub static DIAG: [(u8, u8); $n] = diag_scan::<$size, $n>();
            pub static HORIZ: [(u8, u8); $n] = horiz_scan::<$size, $n>();
            pub static VERT: [(u8, u8); $n] = vert_scan::<$size, $n>();
            pub static DIAG_INV: [u8; $n] = invert::<$size, $n>(diag_scan::<$size, $n>());
            pub static HORIZ_INV: [u8; $n] = invert::<$size, $n>(horiz_scan::<$size, $n>());
            pub static VERT_INV: [u8; $n] = invert::<$size, $n>(vert_scan::<$size, $n>());
            pub static DIAG_BBOX: [(u8, u8); $n] = bbox(diag_scan::<$size, $n>());
            pub static HORIZ_BBOX: [(u8, u8); $n] = bbox(horiz_scan::<$size, $n>());
            pub static VERT_BBOX: [(u8, u8); $n] = bbox(vert_scan::<$size, $n>());
        }
    };
}
scans!(s1, 1, 1);
scans!(s2, 2, 4);
scans!(s4, 4, 16);
scans!(s8, 8, 64);

static SIG_4X4: [[u8; 16]; 3] = [sig_by_scan(s4::DIAG), sig_by_scan(s4::HORIZ), sig_by_scan(s4::VERT)];
static SIG_NB: [[[u8; 16]; 4]; 3] = [sig_nb(s4::DIAG), sig_nb(s4::HORIZ), sig_nb(s4::VERT)];

macro_rules! set {
    ($m:ident, $which:ident, $pos:ident, $si:literal) => {
        paste_set!($m, $which, $pos, $si)
    };
}
macro_rules! paste_set {
    ($m:ident, DIAG, $pos:ident, $si:literal) => {
        ScanSet { sb: &$m::DIAG, sb_inv: &$m::DIAG_INV, sb_bbox: &$m::DIAG_BBOX, pos: &s4::$pos, pos_inv: &s4::DIAG_INV, sig_4x4: &SIG_4X4[$si], sig_nb: &SIG_NB[$si] }
    };
    ($m:ident, HORIZ, $pos:ident, $si:literal) => {
        ScanSet { sb: &$m::HORIZ, sb_inv: &$m::HORIZ_INV, sb_bbox: &$m::HORIZ_BBOX, pos: &s4::$pos, pos_inv: &s4::HORIZ_INV, sig_4x4: &SIG_4X4[$si], sig_nb: &SIG_NB[$si] }
    };
    ($m:ident, VERT, $pos:ident, $si:literal) => {
        ScanSet { sb: &$m::VERT, sb_inv: &$m::VERT_INV, sb_bbox: &$m::VERT_BBOX, pos: &s4::$pos, pos_inv: &s4::VERT_INV, sig_4x4: &SIG_4X4[$si], sig_nb: &SIG_NB[$si] }
    };
}

/// `SCAN_SETS[log2TrafoSize - 2][scanIdx]`: scanIdx 0 diagonal, 1 horizontal,
/// 2 vertical.
static SCAN_SETS: [[ScanSet; 3]; 4] = [
    [set!(s1, DIAG, DIAG, 0), set!(s1, HORIZ, HORIZ, 1), set!(s1, VERT, VERT, 2)],
    [set!(s2, DIAG, DIAG, 0), set!(s2, HORIZ, HORIZ, 1), set!(s2, VERT, VERT, 2)],
    [set!(s4, DIAG, DIAG, 0), set!(s4, HORIZ, HORIZ, 1), set!(s4, VERT, VERT, 2)],
    [set!(s8, DIAG, DIAG, 0), set!(s8, HORIZ, HORIZ, 1), set!(s8, VERT, VERT, 2)],
];

#[inline]
pub fn scan_set(log2sb: usize, scan_idx: usize) -> &'static ScanSet {
    &SCAN_SETS[log2sb.min(3)][scan_idx.min(2)]
}

/// Column 0 of `transMatrix`: the distinct DCT magnitudes (§8.6.4.2).
const DCT_BASE: [i16; 33] = [64, 90, 90, 90, 89, 88, 87, 85, 83, 82, 80, 78, 75, 73, 70, 67, 64, 61, 57, 54, 50, 46, 43, 38, 36, 31, 25, 22, 18, 13, 9, 4, 0];

const fn build_dct32() -> [[i16; 32]; 32] {
    let mut m = [[0i16; 32]; 32];
    let mut k = 0;
    while k < 32 {
        let mut n = 0;
        while n < 32 {
            let a = ((2 * n + 1) * k) % 128;
            m[k][n] = if k == 0 {
                64
            } else if a <= 32 {
                DCT_BASE[a]
            } else if a <= 64 {
                -DCT_BASE[64 - a]
            } else if a <= 96 {
                -DCT_BASE[a - 64]
            } else {
                DCT_BASE[128 - a]
            };
            n += 1;
        }
        k += 1;
    }
    m
}

/// `transMatrix[k][n]`; the N-point matrix is rows `k * 32 / N`, columns `0..N`.
pub static DCT32: [[i16; 32]; 32] = build_dct32();

/// The 4×4 DST-VII of intra luma 4×4 blocks, padded to the DCT's row pitch.
pub static DST4: [[i16; 32]; 4] = {
    let rows: [[i16; 4]; 4] = [[29, 55, 74, 84], [74, 74, 0, -74], [84, -29, -74, 55], [55, -84, 74, -29]];
    let mut t = [[0i16; 32]; 4];
    let mut k = 0;
    while k < 4 {
        let mut j = 0;
        while j < 4 {
            t[k][j] = rows[k][j];
            j += 1;
        }
        k += 1;
    }
    t
};

/// `intraPredAngle` by mode (Table 8-4); modes 0 and 1 are not angular.
pub static INTRA_PRED_ANGLE: [i32; 35] = [0, 0, 32, 26, 21, 17, 13, 9, 5, 2, 0, -2, -5, -9, -13, -17, -21, -26, -32, -26, -21, -17, -13, -9, -5, -2, 0, 2, 5, 9, 13, 17, 21, 26, 32];

/// `invAngle` by mode 11..=25 (Table 8-5), indexed by `mode - 11`.
pub static INV_ANGLE: [i32; 15] = [-4096, -1638, -910, -630, -482, -390, -315, -256, -315, -390, -482, -630, -910, -1638, -4096];

/// `levelScale[qP % 6]` (§8.6.3).
pub static LEVEL_SCALE: [i32; 6] = [40, 45, 51, 57, 64, 72];

/// Deblocking `β′` by Q (Table 8-12).
pub static BETA_TABLE: [u8; 52] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 22, 24, 26, 28, 30, 32, 34, 36, 38, 40, 42, 44, 46, 48, 50, 52, 54, 56, 58, 60, 62, 64];

/// Deblocking `tC′` by Q (Table 8-12).
pub static TC_TABLE: [u8; 54] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 9, 10, 11, 13, 14, 16, 18, 20, 22, 24];

/// Luma 8-tap filters `fL[xFracL]` (Table 8-11).
pub static LUMA_FILTER: [[i16; 8]; 4] = [[0, 0, 0, 64, 0, 0, 0, 0], [-1, 4, -10, 58, 17, -5, 1, 0], [-1, 4, -11, 40, 40, -11, 4, -1], [0, 1, -5, 17, 58, -10, 4, -1]];

/// Chroma 4-tap filters `fC[xFracC]` (Table 8-12).
pub static CHROMA_FILTER: [[i16; 4]; 8] = [[0, 64, 0, 0], [-2, 58, 10, -2], [-4, 54, 16, -2], [-6, 46, 28, -4], [-4, 36, 36, -4], [-4, 28, 46, -6], [-2, 16, 54, -4], [-2, 10, 58, -2]];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_and_matrix() {
        assert_eq!(&s4::DIAG[..6], &[(0, 0), (0, 1), (1, 0), (0, 2), (1, 1), (2, 0)]);
        assert_eq!(s8::DIAG[63], (7, 7));
        assert_eq!(s2::DIAG, [(0, 0), (0, 1), (1, 0), (1, 1)]);
        for (i, &(x, y)) in s8::VERT.iter().enumerate() {
            assert_eq!(s8::VERT_INV[y as usize * 8 + x as usize] as usize, i);
        }
        assert_eq!(&DCT32[8][..4], &[83, 36, -36, -83]);
        assert_eq!(&DCT32[4][..8], &[89, 75, 50, 18, -18, -50, -75, -89]);
        assert_eq!(DCT32[31][1], -13);
        assert_eq!(SIG_4X4[0][1], SIG_MAP_4X4[4]);
    }
}
