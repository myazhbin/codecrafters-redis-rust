//! Bitmaps, which are strings looked at bit by bit. Bit 0 is the most significant bit of
//! the first byte, so setting bit 1 of an empty string gives the byte 01000000.

/// The highest bit offset: Redis caps strings at 512MB, which is 2^32 bits.
pub const MAX_OFFSET: u64 = (1 << 32) - 1;

/// Parses a bit offset, which has to be from 0 to `MAX_OFFSET`.
pub fn parse_offset(arg: &[u8]) -> Option<u64> {
    let offset: u64 = std::str::from_utf8(arg).ok()?.parse().ok()?;
    (offset <= MAX_OFFSET).then_some(offset)
}

/// The byte a bit is in, and the mask that picks the bit out of it.
fn locate(offset: u64) -> (usize, u8) {
    ((offset / 8) as usize, 0x80 >> (offset % 8))
}

/// Sets the bit at `offset` to `on`, growing the string with zero bytes if it's too short
/// to have that bit. Returns what the bit was before.
pub fn set_bit(bytes: &mut Vec<u8>, offset: u64, on: bool) -> bool {
    let (byte, mask) = locate(offset);
    if bytes.len() <= byte {
        bytes.resize(byte + 1, 0);
    }
    let was_on = bytes[byte] & mask != 0;
    if on {
        bytes[byte] |= mask;
    } else {
        bytes[byte] &= !mask;
    }
    was_on
}

/// The bit at `offset`, where bits past the end of the string are 0.
pub fn get_bit(bytes: &[u8], offset: u64) -> bool {
    let (byte, mask) = locate(offset);
    bytes.get(byte).is_some_and(|b| b & mask != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_0_is_the_top_bit_of_the_first_byte() {
        let mut bytes = Vec::new();
        assert!(!set_bit(&mut bytes, 1, true));
        assert_eq!(bytes, [0b0100_0000]);
        assert!(!set_bit(&mut bytes, 3, true));
        assert_eq!(bytes, [0b0101_0000]);
        assert!(set_bit(&mut bytes, 3, true));
        assert!(set_bit(&mut bytes, 1, false));
        assert_eq!(bytes, [0b0001_0000]);
    }

    #[test]
    fn grows_the_string_with_zeros() {
        let mut bytes = vec![0b0100_0000];
        assert!(!set_bit(&mut bytes, 10, true));
        assert_eq!(bytes, [0b0100_0000, 0b0010_0000]);
        assert!(!set_bit(&mut bytes, 40, false));
        assert_eq!(bytes.len(), 6);
    }

    #[test]
    fn reads_bits() {
        // "A" is 01000001.
        assert!(!get_bit(b"A", 0));
        assert!(get_bit(b"A", 1));
        assert!(get_bit(b"A", 7));
        assert!(!get_bit(b"A", 8));
        assert!(!get_bit(b"", 0));
        assert!(!get_bit(b"A", MAX_OFFSET));
    }

    #[test]
    fn parses_offsets() {
        assert_eq!(parse_offset(b"0"), Some(0));
        assert_eq!(parse_offset(b"4294967295"), Some(MAX_OFFSET));
        assert_eq!(parse_offset(b"4294967296"), None);
        assert_eq!(parse_offset(b"-1"), None);
        assert_eq!(parse_offset(b"x"), None);
    }
}
