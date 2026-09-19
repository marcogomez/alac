//! The bit reader, big endian, one to thirty two bits at a time.
//!
//! The shift helpers accept any count. A count that is negative or at least
//! the width of the type gives zero for an unsigned value and a copy of the
//! sign bit for a signed one, which is what the decoder's arithmetic expects.

/// Shift left. A count that is negative or too wide gives zero.
#[inline]
pub fn shl_u32(value: u32, count: i32) -> u32 {
    if !(0..32).contains(&count) {
        0
    } else {
        value << count
    }
}

/// Shift right. A count that is negative or too wide gives zero.
#[inline]
pub fn shr_u32(value: u32, count: i32) -> u32 {
    if !(0..32).contains(&count) {
        0
    } else {
        value >> count
    }
}

/// Shift a signed value left, keeping the bits that run off the top.
#[inline]
pub fn shl_i32(value: i32, count: i32) -> i32 {
    if !(0..32).contains(&count) {
        0
    } else {
        ((value as u32) << count) as i32
    }
}

/// Shift a signed value right, filling from the sign bit.
#[inline]
pub fn shr_i32(value: i32, count: i32) -> i32 {
    if !(0..32).contains(&count) {
        if value < 0 {
            -1
        } else {
            0
        }
    } else {
        value >> count
    }
}

/// Shift a wide signed value left, keeping the bits that run off the top. A
/// count that is negative or too wide gives zero.
#[inline]
pub fn shl_i64(value: i64, count: i32) -> i64 {
    if !(0..64).contains(&count) {
        0
    } else {
        ((value as u64) << count) as i64
    }
}

/// Shift a wide signed value right, filling from the sign bit.
#[inline]
pub fn shr_i64(value: i64, count: i32) -> i64 {
    if !(0..64).contains(&count) {
        if value < 0 {
            -1
        } else {
            0
        }
    } else {
        value >> count
    }
}

/// Reads bits out of a frame, from the front, most significant bit first.
pub struct Bits<'a> {
    data: &'a [u8],
    /// How far into the frame the next read starts, in bytes.
    index: usize,
    /// How many bits of the byte at `index` have already been read.
    accumulator: i32,
}

impl<'a> Bits<'a> {
    /// A reader over one frame.
    pub fn new(data: &'a [u8]) -> Self {
        Bits {
            data,
            index: 0,
            accumulator: 0,
        }
    }

    /// One byte of the frame, or zero once the frame has run out, so a frame
    /// that has been cut short reads as zeros rather than panicking.
    #[inline]
    fn byte(&self, at: usize) -> u32 {
        match self.data.get(at) {
            Some(value) => *value as u32,
            None => 0,
        }
    }

    /// Reads one to sixteen bits.
    pub fn read16(&mut self, bits: i32) -> u32 {
        let mut result = self.byte(self.index) << 16;
        result |= self.byte(self.index + 1) << 8;
        result |= self.byte(self.index + 2);

        // move the bits already read off the top, keep twenty four bits, and
        // then take the top `bits` of those
        result = shl_u32(result, self.accumulator);
        result &= 0x00ff_ffff;
        result = shr_u32(result, 24 - bits);

        let moved = self.accumulator + bits;
        self.step(moved);
        result
    }

    /// Reads one to thirty two bits.
    pub fn read(&mut self, bits: i32) -> u32 {
        let mut result: i32 = 0;
        let mut left = bits;
        if left > 16 {
            left -= 16;
            result = shl_i32(self.read16(16) as i32, left);
        }
        result |= self.read16(left) as i32;
        result as u32
    }

    /// Reads a single bit.
    pub fn read_bit(&mut self) -> u32 {
        let mut result = self.byte(self.index);
        result = shl_u32(result, self.accumulator);
        result = (result >> 7) & 1;

        let moved = self.accumulator + 1;
        self.step(moved);
        result
    }

    /// Puts bits back, so the next read sees them again.
    pub fn unread(&mut self, bits: i32) {
        self.step(self.accumulator - bits);
    }

    /// Takes a new position in bits and splits it into a byte and a remainder.
    #[inline]
    fn step(&mut self, moved: i32) {
        let bytes = i64::from(moved >> 3);
        let index = self.index as i64 + bytes;
        self.index = if index < 0 { 0 } else { index as usize };
        // the mask keeps this in 0 to 7 even when `moved` went negative, which
        // is what unreading past the start of the current byte does
        self.accumulator = moved & 7;
    }
}
