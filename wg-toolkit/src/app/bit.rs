//! Minimal MSB-first bit packing, matching BigWorld's own `BitWriter`/`BitReader`
//! (`cstdmf/bit_writer.cpp`/`bit_reader.cpp` in the leaked BigWorld 14.4.1 SDK,
//! `re-work/bigworld-src-14.4.1/`) -- used by [`super::math`] to implement the packed
//! position/direction formulas from `network/msgtypes.hpp`/`.ipp`.

/// Each [`Self::add`] call appends `num_bits` bits taken from the low bits of `value`,
/// written starting from the most-significant unused bit of the current byte and
/// spilling into subsequent bytes as needed.
pub(crate) struct BitWriter<'a> {
    buf: &'a mut [u8],
    pos: u32,
}

impl<'a> BitWriter<'a> {

    pub(crate) fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub(crate) fn add(&mut self, num_bits: u32, value: u32) {
        for i in (0..num_bits).rev() {
            let bit = (value >> i) & 1;
            let byte = (self.pos / 8) as usize;
            let shift = 7 - (self.pos % 8);
            self.buf[byte] |= (bit as u8) << shift;
            self.pos += 1;
        }
    }

}

/// Counterpart to [`BitWriter`].
pub(crate) struct BitReader<'a> {
    buf: &'a [u8],
    pos: u32,
}

impl<'a> BitReader<'a> {

    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub(crate) fn get(&mut self, num_bits: u32) -> u32 {
        let mut value = 0;
        for _ in 0..num_bits {
            let byte = (self.pos / 8) as usize;
            let shift = 7 - (self.pos % 8);
            let bit = (self.buf[byte] >> shift) & 1;
            value = (value << 1) | u32::from(bit);
            self.pos += 1;
        }
        value
    }

}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_byte_roundtrip() {
        let mut data = [0u8; 1];
        let mut writer = BitWriter::new(&mut data);
        writer.add(3, 0b101);
        writer.add(5, 0b11010);
        assert_eq!(data, [0b101_11010]);

        let mut reader = BitReader::new(&data);
        assert_eq!(reader.get(3), 0b101);
        assert_eq!(reader.get(5), 0b11010);
    }

    #[test]
    fn crosses_byte_boundary() {
        // 12-bit fields packed back to back, straddling the byte boundary.
        let mut data = [0u8; 3];
        let mut writer = BitWriter::new(&mut data);
        writer.add(12, 0xABC);
        writer.add(12, 0x123);

        let mut reader = BitReader::new(&data);
        assert_eq!(reader.get(12), 0xABC);
        assert_eq!(reader.get(12), 0x123);
    }

    #[test]
    fn many_small_fields() {
        let mut data = [0u8; 5];
        let mut writer = BitWriter::new(&mut data);
        let values: [u32; 8] = [1, 0, 1, 1, 0, 0, 1, 0];
        for &v in &values {
            writer.add(5, v);
        }

        let mut reader = BitReader::new(&data);
        for &v in &values {
            assert_eq!(reader.get(5), v);
        }
    }
}
