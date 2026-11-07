//! Reading RDB files, the snapshots of a dataset that Redis loads at startup. Only string
//! values are supported, which is all this server's RDB files hold.
//!
//! The format: a `REDIS` header with a four-digit version, then a series of sections, each
//! starting with an opcode byte: metadata (`FA`), the start of a database (`FE`), hash
//! table sizes (`FB`), a key's expiry (`FD` in seconds, `FC` in milliseconds), and the
//! end of the file (`FF`, then a checksum). Any other byte starts a key: it gives the
//! value's type, and the key and value follow.

use anyhow::{Context, Result, bail};

/// A key read from an RDB file.
#[derive(Debug, PartialEq)]
pub struct Key {
    /// The database the key is in.
    pub db: u64,
    pub key: Vec<u8>,
    /// The value, which is always a string.
    pub value: Vec<u8>,
    /// When the key expires, as a Unix time in milliseconds.
    pub expires_at_ms: Option<u64>,
}

/// Parses a whole RDB file into the keys it holds.
pub fn parse(data: &[u8]) -> Result<Vec<Key>> {
    let mut file = Reader { data, pos: 0 };
    let header = file.bytes(9).context("the file is too short for an RDB header")?;
    if &header[..5] != b"REDIS" || !header[5..].iter().all(u8::is_ascii_digit) {
        bail!("not an RDB file: it starts with {:?}", String::from_utf8_lossy(header));
    }
    let mut keys = Vec::new();
    let mut db = 0;
    // An expiry comes just before the key it's for.
    let mut expires_at_ms = None;
    loop {
        match file.byte()? {
            // Metadata, such as the Redis version that wrote the file.
            0xFA => {
                file.string()?;
                file.string()?;
            }
            0xFE => db = file.length()?,
            // How big to make the database's hash tables: keys, then keys with expiries.
            0xFB => {
                file.length()?;
                file.length()?;
            }
            0xFD => {
                let secs = u32::from_le_bytes(file.array()?);
                expires_at_ms = Some(u64::from(secs) * 1000);
            }
            0xFC => expires_at_ms = Some(u64::from_le_bytes(file.array()?)),
            // How long the next key has gone unused, or how often it's used, for eviction.
            0xF8 => {
                file.length()?;
            }
            0xF9 => {
                file.byte()?;
            }
            // The end of the file, which goes on to a checksum that isn't checked here.
            0xFF => return Ok(keys),
            // A string key.
            0x00 => keys.push(Key {
                db,
                key: file.string()?,
                value: file.string()?,
                expires_at_ms: expires_at_ms.take(),
            }),
            other => bail!("unsupported RDB opcode or value type {other:#04x}"),
        }
    }
}

/// Reads through the bytes of an RDB file.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

/// What a length-encoded number says: either a length, or how the string that would
/// have followed it is encoded instead.
enum Length {
    Len(u64),
    /// An integer, stored in binary in 1, 2 or 4 bytes, but meaning its decimal digits.
    Int(usize),
    /// A string compressed with LZF.
    Lzf,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(len).filter(|&end| end <= self.data.len());
        let Some(end) = end else {
            bail!("the RDB file ends partway through (at byte {})", self.data.len());
        };
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.bytes(N)?.try_into().unwrap())
    }

    /// A length-encoded number. Its first two bits say how it's stored: in the rest of
    /// the byte (00), in the rest of it and the next byte (01), or in the 4 or 8 bytes
    /// that follow (10, with the rest of the byte 0 or 1). 11 means a special encoding.
    fn length_or_encoding(&mut self) -> Result<Length> {
        let first = self.byte()?;
        let rest = u64::from(first & 0x3F);
        Ok(match first >> 6 {
            0b00 => Length::Len(rest),
            0b01 => Length::Len(rest << 8 | u64::from(self.byte()?)),
            0b10 if first == 0x81 => Length::Len(u64::from_be_bytes(self.array()?)),
            0b10 => Length::Len(u64::from(u32::from_be_bytes(self.array()?))),
            _ => match rest {
                0 => Length::Int(1),
                1 => Length::Int(2),
                2 => Length::Int(4),
                3 => Length::Lzf,
                _ => bail!("unknown string encoding {first:#04x}"),
            },
        })
    }

    /// A length-encoded number that has to be a plain length.
    fn length(&mut self) -> Result<u64> {
        match self.length_or_encoding()? {
            Length::Len(len) => Ok(len),
            _ => bail!("expected a length, found a string encoding"),
        }
    }

    fn string(&mut self) -> Result<Vec<u8>> {
        Ok(match self.length_or_encoding()? {
            Length::Len(len) => self.bytes(usize::try_from(len)?)?.to_vec(),
            Length::Int(size) => {
                let bytes = self.bytes(size)?;
                // Little-endian and signed, so the last byte's top bit is the sign.
                let mut n = if bytes[size - 1] & 0x80 != 0 { -1i64 } else { 0 };
                for &byte in bytes.iter().rev() {
                    n = n << 8 | i64::from(byte);
                }
                n.to_string().into_bytes()
            }
            Length::Lzf => {
                let compressed_len = usize::try_from(self.length()?)?;
                let len = usize::try_from(self.length()?)?;
                lzf_decompress(self.bytes(compressed_len)?, len)?
            }
        })
    }
}

/// Decompresses LZF data, which is a series of runs: a control byte below 32 is followed
/// by that many plus one literal bytes, and any other copies an earlier part of the output.
fn lzf_decompress(input: &[u8], len: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(len);
    let mut input = input.iter().copied();
    let mut next = || input.next().context("LZF data ends partway through");
    while let Ok(control) = next() {
        let control = usize::from(control);
        if control < 32 {
            for _ in 0..=control {
                out.push(next()?);
            }
            continue;
        }
        // A copy: its length minus 2 is in the top 3 bits, unless they're all set, when
        // the next byte has the rest. The distance back is in the low 5 bits and a byte.
        let mut copy_len = control >> 5;
        if copy_len == 7 {
            copy_len += usize::from(next()?);
        }
        let distance = ((control & 0x1F) << 8 | usize::from(next()?)) + 1;
        let Some(start) = out.len().checked_sub(distance) else {
            bail!("LZF data refers back before its start");
        };
        // Byte by byte, because a copy can overlap what it's producing.
        for i in start..start + copy_len + 2 {
            out.push(out[i]);
        }
    }
    if out.len() != len {
        bail!("LZF data came to {} bytes, not {len}", out.len());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &[u8] = b"REDIS0011\xfa\x09redis-ver\x066.0.16";
    const END: &[u8] = b"\xff\x89\x3b\xb7\x4e\xf8\x0f\x77\x19";

    fn file(body: &[u8]) -> Vec<u8> {
        [HEADER, body, END].concat()
    }

    fn key(key: &str, value: &str, expires_at_ms: Option<u64>) -> Key {
        Key {
            db: 0,
            key: key.into(),
            value: value.into(),
            expires_at_ms,
        }
    }

    #[test]
    fn parses_the_stage_descriptions_example() {
        let body = b"\xfe\x00\xfb\x03\x02\
            \x00\x06foobar\x06bazqux\
            \xfc\x15\x72\xe7\x07\x8f\x01\x00\x00\x00\x03foo\x03bar\
            \xfd\x52\xed\x2a\x66\x00\x03baz\x03qux";
        assert_eq!(
            parse(&file(body)).unwrap(),
            [
                key("foobar", "bazqux", None),
                key("foo", "bar", Some(1713824559637)),
                key("baz", "qux", Some(1714089298000)),
            ]
        );
    }

    #[test]
    fn parses_an_empty_file() {
        assert_eq!(parse(&file(b"")).unwrap(), []);
        // The empty file a master sends its replicas.
        let empty = b"REDIS0011\xfa\x09redis-ver\x057.2.0\xfa\x0aredis-bits\xc0\x40\
            \xfa\x05ctime\xc2\x6d\x08\xbc\x65\xfa\x08used-mem\xc2\xb0\xc4\x10\x00\
            \xfa\x08aof-base\xc0\x00\xff\xf0\x6e\x3b\xfe\xc0\xff\x5a\xa2";
        assert_eq!(parse(empty).unwrap(), []);
    }

    #[test]
    fn reads_integer_encoded_strings() {
        let body = b"\xfe\x00\x00\x01a\xc0\x7b\x00\x01b\xc1\x39\x30\x00\x01c\xc2\x87\xd6\x12\x00\
            \x00\x01d\xc0\xff\x00\x01e\xc1\x00\x80\x00\x01f\xc2\xff\xff\xff\xff\x00\xc0\x05\x01g";
        let keys = parse(&file(body)).unwrap();
        let values: Vec<(&[u8], &[u8])> =
            keys.iter().map(|k| (&k.key[..], &k.value[..])).collect();
        assert_eq!(
            values,
            [
                (&b"a"[..], &b"123"[..]),
                (b"b", b"12345"),
                (b"c", b"1234567"),
                (b"d", b"-1"),
                (b"e", b"-32768"),
                (b"f", b"-1"),
                (b"5", b"g"),
            ]
        );
    }

    #[test]
    fn reads_longer_lengths() {
        let value = "x".repeat(700);
        let body = [b"\x00\x01k\x42\xbc".as_slice(), value.as_bytes()].concat();
        assert_eq!(parse(&file(&body)).unwrap(), [key("k", &value, None)]);
        let value = "y".repeat(17000);
        let body = [b"\x00\x01k\x80\x00\x00\x42\x68".as_slice(), value.as_bytes()].concat();
        assert_eq!(parse(&file(&body)).unwrap(), [key("k", &value, None)]);
        let body = [b"\x00\x01k\x81\x00\x00\x00\x00\x00\x00\x42\x68".as_slice(), value.as_bytes()].concat();
        assert_eq!(parse(&file(&body)).unwrap(), [key("k", &value, None)]);
    }

    #[test]
    fn reads_lzf_compressed_strings() {
        // A literal "a", then 9 bytes copied from 1 back; and "abc" then a copy of it.
        let body = b"\x00\x01a\xc3\x05\x0a\x00a\xe0\x00\x00\x00\x01b\xc3\x06\x06\x02abc\x20\x02";
        assert_eq!(
            parse(&file(body)).unwrap(),
            [key("a", "aaaaaaaaaa", None), key("b", "abcabc", None)]
        );
        assert!(parse(&file(b"\x00\x01a\xc3\x03\x05\x00a\x21")).is_err());
        assert!(parse(&file(b"\x00\x01a\xc3\x02\x02\x20\x05")).is_err());
    }

    #[test]
    fn keeps_track_of_databases_and_skips_eviction_hints() {
        let body = b"\xfe\x00\x00\x01a\x011\xfe\x02\xfb\x01\x00\xf8\x05\xf9\x07\x00\x01b\x012";
        let keys = parse(&file(body)).unwrap();
        assert_eq!(keys[0], key("a", "1", None));
        assert_eq!((keys[1].db, &keys[1].key[..]), (2, &b"b"[..]));
    }

    #[test]
    fn an_expiry_is_only_for_the_next_key() {
        let body = b"\xfc\x01\x00\x00\x00\x00\x00\x00\x00\x00\x01a\x011\x00\x01b\x012";
        assert_eq!(
            parse(&file(body)).unwrap(),
            [key("a", "1", Some(1)), key("b", "2", None)]
        );
    }

    #[test]
    fn rejects_what_it_cant_read() {
        assert!(parse(b"").is_err());
        assert!(parse(b"REDIS").is_err());
        assert!(parse(b"NOTRDB0011\xff").is_err());
        assert!(parse(b"REDISabcd\xff").is_err());
        // No end-of-file marker, or a key cut short.
        assert!(parse(HEADER).is_err());
        assert!(parse(&[HEADER, b"\x00\x03fo"].concat()).is_err());
        // A list, which isn't supported.
        assert!(parse(&file(b"\x01\x01l\x01\x01a")).is_err());
    }
}
