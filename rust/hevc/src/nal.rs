//! NAL units: the Annex B framing of an access unit, the two-byte header, and
//! emulation prevention.

/// `nal_unit_type` values this decoder acts on.
pub const TRAIL_N: u8 = 0;
pub const TRAIL_R: u8 = 1;
pub const IDR_W_RADL: u8 = 19;
pub const IDR_N_LP: u8 = 20;
pub const SPS: u8 = 33;
pub const PPS: u8 = 34;

#[derive(Debug, Clone, Copy)]
pub struct NalHeader {
    pub nal_type: u8,
    pub layer_id: u8,
    pub temporal_id: u8,
}

impl NalHeader {
    pub fn parse(nal: &[u8]) -> Option<Self> {
        let b0 = *nal.first()?;
        let b1 = *nal.get(1)?;
        if b0 & 0x80 != 0 || b1 & 7 == 0 {
            return None;
        }
        Some(NalHeader { nal_type: (b0 >> 1) & 0x3f, layer_id: ((b0 & 1) << 5) | (b1 >> 3), temporal_id: (b1 & 7) - 1 })
    }
}

/// Zero bytes [`Rbsp::unescape`] leaves after the payload, so that a reader
/// may take whole words up to its end and read zeros past it, as the
/// standard has it read, without a byte-wise tail.
pub const RBSP_PAD: usize = 8;

/// A NAL payload with its emulation-prevention bytes removed, and where they
/// were: a slice's entry points count them (§7.4.7.1).
#[derive(Default)]
pub struct Rbsp {
    /// The payload, then [`RBSP_PAD`] zeros.
    pub data: Vec<u8>,
    /// Index in the escaped payload of each removed `03`, ascending.
    epb_pos: Vec<usize>,
}

impl Rbsp {
    /// Removes the emulation-prevention bytes of `ebsp` into `self`.
    pub fn unescape(&mut self, ebsp: &[u8]) {
        self.data.clear();
        self.epb_pos.clear();
        self.data.reserve(ebsp.len() + RBSP_PAD);
        let mut zeros = 0usize;
        for (i, &b) in ebsp.iter().enumerate() {
            if zeros >= 2 && b == 3 {
                self.epb_pos.push(i);
                zeros = 0;
                continue;
            }
            self.data.push(b);
            zeros = if b == 0 { zeros + 1 } else { 0 };
        }
        self.data.extend_from_slice(&[0; RBSP_PAD]);
    }

    /// An offset into the escaped payload as an offset into `data`.
    pub fn escaped_to_rbsp(&self, escaped: usize) -> usize {
        escaped - self.epb_pos.iter().take_while(|&&p| p < escaped).count()
    }

    /// An offset into `data` as an offset into the escaped payload.
    pub fn rbsp_to_escaped(&self, rbsp: usize) -> usize {
        let mut e = rbsp;
        for &p in &self.epb_pos {
            if p <= e {
                e += 1;
            } else {
                break;
            }
        }
        e
    }
}

/// The access units of an Annex B byte stream (§7.4.2.4.4), each what
/// [`Decoder::decode`](crate::Decoder::decode) takes: a unit starts at each
/// VCL NAL unit with `first_slice_segment_in_pic_flag`, with the non-VCL
/// units before it. Bytes before the first unit, and a stream with no slice
/// in it, are no unit.
pub fn access_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    let mut pending_start: Option<usize> = None;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            let nal_type = (data[i + 3] >> 1) & 0x3f;
            let start = if i > 0 && data[i - 1] == 0 { i - 1 } else { i };
            if nal_type < 32 {
                let first = i + 5 < data.len() && data[i + 5] & 0x80 != 0;
                if first {
                    starts.push(pending_start.take().unwrap_or(start));
                }
            } else if pending_start.is_none() {
                pending_start = Some(start);
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut units = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let e = starts.get(k + 1).copied().unwrap_or(data.len());
        units.push(&data[s..e]);
    }
    units
}

/// The NAL units of an Annex B byte stream, header and escaped payload each,
/// start codes and trailing zeros removed.
pub fn split_annex_b(stream: &[u8]) -> Vec<&[u8]> {
    fn push<'a>(nals: &mut Vec<&'a [u8]>, stream: &'a [u8], s: usize, mut end: usize) {
        while end > s && stream[end - 1] == 0 {
            end -= 1;
        }
        if end > s {
            nals.push(&stream[s..end]);
        }
    }
    let mut nals = Vec::new();
    let mut start: Option<usize> = None;
    let mut i = 0;
    while i + 2 < stream.len() {
        if stream[i] == 0 && stream[i + 1] == 0 && stream[i + 2] == 1 {
            if let Some(s) = start {
                push(&mut nals, stream, s, i);
            }
            start = Some(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    if let Some(s) = start {
        push(&mut nals, stream, s, stream.len());
    }
    nals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_and_split() {
        let esc = [0u8, 0, 3, 1, 5, 0, 0, 3, 0, 9];
        let mut r = Rbsp::default();
        r.unescape(&esc);
        assert_eq!(r.data[..8], [0, 0, 1, 5, 0, 0, 0, 9]);
        assert_eq!(r.data[8..], [0; RBSP_PAD]);
        assert_eq!(r.escaped_to_rbsp(9), 7);
        assert_eq!(r.rbsp_to_escaped(7), 9);
        let s = [0, 0, 0, 1, 0x42, 1, 0xAA, 0, 0, 1, 0x44, 1, 0xBB, 0, 0];
        assert_eq!(split_annex_b(&s), vec![&[0x42u8, 1, 0xAA][..], &[0x44u8, 1, 0xBB][..]]);
        let h = NalHeader::parse(&[0x42, 0x01]).unwrap();
        assert_eq!((h.nal_type, h.layer_id, h.temporal_id), (SPS, 0, 0));
    }
}
