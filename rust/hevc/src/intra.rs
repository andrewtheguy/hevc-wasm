//! Intra sample prediction (§8.4.4.2): the neighbours of a block, substituted
//! where missing and filtered where the mode asks, then planar, DC or one of
//! the 33 angular predictions.

use crate::kernels;
use crate::tables::{INTRA_PRED_ANGLE, INV_ANGLE};

pub const MODE_PLANAR: u8 = 0;
pub const MODE_DC: u8 = 1;
pub const MODE_HOR: u8 = 10;
pub const MODE_VER: u8 = 26;

/// The neighbours of an N×N block: `left[y] = p[-1][y]` for `y` in `0..2N`,
/// `top[x] = p[x][-1]` for `x` in `0..2N`, and `corner = p[-1][-1]`, each with
/// whether it exists.
pub struct RefSamples {
    pub left: [u8; 64],
    pub top: [u8; 64],
    pub corner: u8,
    pub left_avail: [bool; 64],
    pub top_avail: [bool; 64],
    pub corner_avail: bool,
    /// The projected reference of an angular mode, indices `-N..=2N` at an
    /// offset of N, with room past the end for the kernel's last read.
    refb: [i16; 128],
    /// `left` and `top` with the N+1th sample, for planar.
    pl: [u8; 33],
    pt: [u8; 33],
}

impl Default for RefSamples {
    fn default() -> Self {
        RefSamples { left: [0; 64], top: [0; 64], corner: 0, left_avail: [false; 64], top_avail: [false; 64], corner_avail: false, refb: [0; 128], pl: [0; 33], pt: [0; 33] }
    }
}

impl RefSamples {
    pub fn reset(&mut self, n: usize) {
        self.left_avail[..2 * n].fill(false);
        self.top_avail[..2 * n].fill(false);
        self.corner_avail = false;
    }

    /// §8.4.4.2.2: missing neighbours take the nearest present one, searching
    /// from the bottom of the left column round to the end of the top row.
    pub fn substitute(&mut self, n: usize) {
        let n2 = 2 * n;
        if !(self.corner_avail || self.left_avail[..n2].iter().any(|&a| a) || self.top_avail[..n2].iter().any(|&a| a)) {
            self.left[..n2].fill(128);
            self.top[..n2].fill(128);
            self.corner = 128;
            return;
        }
        if !self.left_avail[n2 - 1] {
            let mut found = None;
            for y in (0..n2 - 1).rev() {
                if self.left_avail[y] {
                    found = Some(self.left[y]);
                    break;
                }
            }
            if found.is_none() && self.corner_avail {
                found = Some(self.corner);
            }
            if found.is_none() {
                found = (0..n2).find(|&x| self.top_avail[x]).map(|x| self.top[x]);
            }
            self.left[n2 - 1] = found.unwrap_or(0);
            self.left_avail[n2 - 1] = true;
        }
        let mut prev = self.left[n2 - 1];
        for y in (0..n2 - 1).rev() {
            if !self.left_avail[y] {
                self.left[y] = prev;
            }
            prev = self.left[y];
        }
        if !self.corner_avail {
            self.corner = prev;
        }
        prev = self.corner;
        for x in 0..n2 {
            if !self.top_avail[x] {
                self.top[x] = prev;
            }
            prev = self.top[x];
        }
    }

    /// §8.4.4.2.3: the `[1 2 1]` smoothing of the neighbours, for the modes
    /// and sizes it applies to. Every component takes it under 4:4:4.
    fn filter(&mut self, n: usize, mode: u8) {
        if mode == MODE_DC || n == 4 {
            return;
        }
        let min_dist = (mode as i32 - 26).abs().min((mode as i32 - 10).abs());
        let thres = match n {
            8 => 7,
            16 => 1,
            _ => 0,
        };
        if min_dist <= thres {
            return;
        }
        let n2 = 2 * n;
        let corner = self.corner as i32;
        let new_corner = ((self.left[0] as i32 + 2 * corner + self.top[0] as i32 + 2) >> 2) as u8;
        let (mut lp, mut tp) = (corner, corner);
        for i in 0..n2 - 1 {
            let (lc, tc) = (self.left[i] as i32, self.top[i] as i32);
            self.left[i] = ((self.left[i + 1] as i32 + 2 * lc + lp + 2) >> 2) as u8;
            self.top[i] = ((self.top[i + 1] as i32 + 2 * tc + tp + 2) >> 2) as u8;
            lp = lc;
            tp = tc;
        }
        self.corner = new_corner;
    }
}

/// Predicts an N×N block of `mode` into `out` from substituted neighbours.
/// The edge adjustments of DC, vertical and horizontal apply to luma alone.
pub fn predict(refs: &mut RefSamples, n: usize, mode: u8, luma: bool, out: &mut [u8], stride: usize) {
    refs.filter(n, mode);
    match mode {
        MODE_PLANAR => {
            refs.pl[..=n].copy_from_slice(&refs.left[..=n]);
            refs.pt[..=n].copy_from_slice(&refs.top[..=n]);
            kernels::planar(out, stride, n, &refs.pl, &refs.pt);
        }
        MODE_DC => {
            let mut sum = n as i32;
            for i in 0..n {
                sum += refs.top[i] as i32 + refs.left[i] as i32;
            }
            let dc = sum >> (n.trailing_zeros() + 1);
            kernels::fill(out, stride, n, dc as u8);
            if luma && n < 32 {
                out[0] = ((refs.left[0] as i32 + 2 * dc + refs.top[0] as i32 + 2) >> 2) as u8;
                for x in 1..n {
                    out[x] = ((refs.top[x] as i32 + 3 * dc + 2) >> 2) as u8;
                }
                for y in 1..n {
                    out[y * stride] = ((refs.left[y] as i32 + 3 * dc + 2) >> 2) as u8;
                }
            }
        }
        _ => {
            let RefSamples { left, top, corner, refb, .. } = refs;
            let corner = *corner;
            let angle = INTRA_PRED_ANGLE[mode as usize];
            let off = n;
            // Modes from 18 run along the top row; the others along the left
            // column, with the roles of the two swapped.
            let (main, side) = if mode >= 18 { (&*top, &*left) } else { (&*left, &*top) };
            refb[off] = corner as i16;
            for i in 0..n {
                refb[off + 1 + i] = main[i] as i16;
            }
            if angle < 0 {
                let last = (n as i32 * angle) >> 5;
                if last < -1 {
                    let inv = INV_ANGLE[(mode - 11) as usize];
                    let mut x = -1;
                    while x >= last {
                        let idx = -1 + ((x * inv + 128) >> 8);
                        refb[(off as i32 + x) as usize] = if idx < 0 { corner as i16 } else { side[idx as usize] as i16 };
                        x -= 1;
                    }
                }
            } else {
                for i in n..2 * n {
                    refb[off + 1 + i] = main[i] as i16;
                }
            }
            if mode >= 18 {
                kernels::angular(out, stride, n, refb, off, angle);
                if mode == MODE_VER && luma && n < 32 {
                    for y in 0..n {
                        out[y * stride] = (top[0] as i32 + ((left[y] as i32 - corner as i32) >> 1)).clamp(0, 255) as u8;
                    }
                }
            } else {
                kernels::angular_t(out, stride, n, refb, off, angle);
                if mode == MODE_HOR && luma && n < 32 {
                    for x in 0..n {
                        out[x] = (left[0] as i32 + ((top[x] as i32 - corner as i32) >> 1)).clamp(0, 255) as u8;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_neighbours_predict_flat() {
        let mut r = RefSamples::default();
        r.left = [50; 64];
        r.top = [50; 64];
        r.corner = 50;
        let mut out = [0u8; 64];
        for m in 0..35 {
            predict(&mut r, 8, m, true, &mut out, 8);
            assert!(out.iter().all(|&v| v == 50), "mode {m}");
        }
    }

    #[test]
    fn substitution_takes_the_first_present() {
        let mut r = RefSamples::default();
        for x in 0..8 {
            r.top[x] = 100 + x as u8;
            r.top_avail[x] = true;
        }
        r.substitute(4);
        assert!(r.left[..8].iter().all(|&v| v == 100));
        assert_eq!(r.corner, 100);
        let mut r = RefSamples::default();
        r.substitute(8);
        assert!(r.left[..16].iter().all(|&v| v == 128));
    }
}
