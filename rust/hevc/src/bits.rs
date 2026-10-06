//! An MSB-first bit reader with Exp-Golomb codes, over an RBSP (emulation
//! prevention already removed).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfData;

pub struct BitReader<'a> {
    data: &'a [u8],
    /// In bits from the start of `data`.
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0 }
    }

    pub fn byte_pos(&self) -> usize {
        self.pos / 8
    }

    pub fn align_to_byte(&mut self) {
        self.pos = (self.pos + 7) & !7;
    }

    #[inline]
    pub fn read_flag(&mut self) -> Result<bool, OutOfData> {
        Ok(self.read_bits(1)? == 1)
    }

    /// `u(n)`, `n` up to 32.
    pub fn read_bits(&mut self, n: u32) -> Result<u32, OutOfData> {
        if n == 0 {
            return Ok(0);
        }
        if n > 24 {
            let hi = self.read_bits(n - 16)?;
            let lo = self.read_bits(16)?;
            return Ok((hi << 16) | lo);
        }
        if self.pos + n as usize > self.data.len() * 8 {
            return Err(OutOfData);
        }
        let v = self.peek_bits(n);
        self.pos += n as usize;
        Ok(v)
    }

    /// The next `n` bits (`n` up to 24) without consuming them, zero past the end.
    #[inline]
    pub fn peek_bits(&self, n: u32) -> u32 {
        let byte = self.pos / 8;
        let off = (self.pos % 8) as u32;
        let b = |i: usize| *self.data.get(byte + i).unwrap_or(&0) as u32;
        let acc = (b(0) << 24) | (b(1) << 16) | (b(2) << 8) | b(3);
        if n == 0 {
            return 0;
        }
        (acc >> (32 - off - n)) & ((1u32 << n) - 1)
    }

    /// `ue(v)`.
    pub fn read_ue(&mut self) -> Result<u32, OutOfData> {
        let mut zeros = 0u32;
        while self.read_bits(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return Err(OutOfData);
            }
        }
        if zeros == 0 {
            return Ok(0);
        }
        Ok((1u32 << zeros) - 1 + self.read_bits(zeros)?)
    }

    /// `ue(v)` that must not exceed `max`.
    pub fn read_ue_max(&mut self, max: u32) -> Result<u32, OutOfData> {
        let v = self.read_ue()?;
        if v > max {
            return Err(OutOfData);
        }
        Ok(v)
    }

    /// `se(v)`.
    pub fn read_se(&mut self) -> Result<i32, OutOfData> {
        let k = self.read_ue()?;
        let m = k.div_ceil(2) as i32;
        Ok(if k % 2 == 1 { m } else { -m })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp_golomb() {
        // ue: 1 -> 0, 010 -> 1, 011 -> 2, 00100 -> 3, 00111 -> 6
        let bytes = [0b1010_0110u8, 0b0100_0011, 0b1000_0000];
        let mut r = BitReader::new(&bytes);
        assert_eq!([r.read_ue().unwrap(), r.read_ue().unwrap(), r.read_ue().unwrap(), r.read_ue().unwrap(), r.read_ue().unwrap()], [0, 1, 2, 3, 6]);
        let bytes = [0b0100_1100u8, 0b1000_0000];
        let mut r = BitReader::new(&bytes);
        assert_eq!([r.read_se().unwrap(), r.read_se().unwrap(), r.read_se().unwrap()], [1, -1, 2]);
        let mut r = BitReader::new(&[0xDE, 0xAD, 0xBE, 0xEF, 0x80]);
        assert_eq!(r.read_bits(32).unwrap(), 0xDEAD_BEEF);
        assert!(r.read_flag().unwrap());
        assert_eq!(r.read_bits(7).unwrap(), 0);
        assert_eq!(r.read_flag(), Err(OutOfData));
    }
}
