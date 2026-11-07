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

/// Resolves BITCOUNT's `start` and `end` against a string `len` units long, as Redis does:
/// negative indexes count back from the end, and both are then kept within the string.
/// Returns `None` if the range holds nothing.
fn resolve_range(start: i64, end: i64, len: i64) -> Option<(i64, i64)> {
    if len == 0 {
        return None;
    }
    let resolve = |i: i64| if i < 0 { (len + i).max(0) } else { i };
    let (start, end) = (resolve(start), resolve(end).min(len - 1));
    (start <= end).then_some((start, end))
}

/// How many bits are set in the bytes from `start` to `end` inclusive (BITCOUNT's default
/// BYTE mode).
pub fn count_in_bytes(bytes: &[u8], start: i64, end: i64) -> u64 {
    let Some((start, end)) = resolve_range(start, end, bytes.len() as i64) else {
        return 0;
    };
    let bytes = &bytes[start as usize..=end as usize];
    bytes.iter().map(|b| u64::from(b.count_ones())).sum()
}

/// How many bits are set from bit `start` to bit `end` inclusive (BITCOUNT's BIT mode).
pub fn count_in_bits(bytes: &[u8], start: i64, end: i64) -> u64 {
    let Some((start, end)) = resolve_range(start, end, bytes.len() as i64 * 8) else {
        return 0;
    };
    let (first, last) = ((start / 8) as usize, (end / 8) as usize);
    let mut count: u64 = bytes[first..=last].iter().map(|b| u64::from(b.count_ones())).sum();
    // Take off the bits of the first and last bytes that are outside the range: the
    // `start % 8` top bits of the first, and the bits after bit `end % 8` of the last.
    let (skip, keep) = ((start % 8) as u32, (end % 8) as u32);
    let before = if skip == 0 { 0 } else { 0xFFu8 << (8 - skip) };
    let after = if keep == 7 { 0 } else { 0xFFu8 >> (keep + 1) };
    count -= u64::from((bytes[first] & before).count_ones());
    count -= u64::from((bytes[last] & after).count_ones());
    count
}

/// BITOP's operations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BitOp {
    And,
}

impl BitOp {
    pub fn parse(name: &[u8]) -> Option<Self> {
        match name.to_ascii_uppercase().as_slice() {
            b"AND" => Some(BitOp::And),
            _ => None,
        }
    }
}

/// Combines `sources` byte by byte with `op`. Shorter sources count as padded with zero
/// bytes, so the result is as long as the longest.
pub fn combine(op: BitOp, sources: &[&[u8]]) -> Vec<u8> {
    let len = sources.iter().map(|source| source.len()).max().unwrap_or(0);
    (0..len)
        .map(|i| {
            let byte = |source: &&[u8]| source.get(i).copied().unwrap_or(0);
            match op {
                BitOp::And => sources.iter().map(byte).fold(0xFF, |a, b| a & b),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ands_bitmaps() {
        // 10001000 AND 10000010.
        assert_eq!(combine(BitOp::And, &[&[0b1000_1000], &[0b1000_0010]]), [0b1000_0000]);
        // A shorter source counts as padded with zeros.
        let long = [0b0100_0000, 0b0010_0000];
        assert_eq!(combine(BitOp::And, &[&long, &[0b0100_0000]]), [0b0100_0000, 0]);
        assert_eq!(combine(BitOp::And, &[&long, &[]]), [0, 0]);
        assert_eq!(combine(BitOp::And, &[&long]), long);
        assert_eq!(combine(BitOp::And, &[&[], &[]]), []);
        assert_eq!(BitOp::parse(b"and"), Some(BitOp::And));
        assert_eq!(BitOp::parse(b"nand"), None);
    }

    #[test]
    fn counts_bits_in_byte_ranges() {
        // 01000000 00100000, as SETBIT 1 and 10 leave it.
        let bytes = [0b0100_0000, 0b0010_0000];
        assert_eq!(count_in_bytes(&bytes, 0, -1), 2);
        assert_eq!(count_in_bytes(&bytes, 0, 1), 2);
        assert_eq!(count_in_bytes(&bytes, 0, 0), 1);
        assert_eq!(count_in_bytes(&bytes, 1, 1), 1);
        assert_eq!(count_in_bytes(&bytes, 1, 100), 1);
        assert_eq!(count_in_bytes(&bytes, 2, 5), 0);
        assert_eq!(count_in_bytes(&bytes, 1, 0), 0);
        assert_eq!(count_in_bytes(&bytes, -1, -1), 1);
        // As in Redis, an end that's still negative after counting back is byte 0.
        assert_eq!(count_in_bytes(&bytes, -100, -100), 1);
        assert_eq!(count_in_bytes(b"", 0, -1), 0);
        assert_eq!(count_in_bytes(b"foobar", 0, -1), 26);
    }

    #[test]
    fn counts_bits_in_bit_ranges() {
        let bytes = [0b1111_1111, 0b1111_1111];
        assert_eq!(count_in_bits(&bytes, 0, 15), 16);
        assert_eq!(count_in_bits(&bytes, 1, 1), 1);
        assert_eq!(count_in_bits(&bytes, 3, 12), 10);
        assert_eq!(count_in_bits(&bytes, 7, 8), 2);
        assert_eq!(count_in_bits(&bytes, -3, -1), 3);
        assert_eq!(count_in_bits(b"foobar", 5, 30), 17);
        assert_eq!(count_in_bits(&[0b0100_0000], 0, 0), 0);
        assert_eq!(count_in_bits(&[0b0100_0000], 1, 1), 1);
    }

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
