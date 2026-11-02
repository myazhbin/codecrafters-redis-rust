use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    SimpleString(String),
    Error(String),
    Integer(i64),
    BulkString(Vec<u8>),
    NullBulkString,
    Array(Vec<Value>),
    NullArray,
}

impl Value {
    /// A command as clients send it: an array of bulk strings.
    pub fn command<T: AsRef<[u8]>>(args: &[T]) -> Value {
        let args = args.iter().map(|arg| arg.as_ref().to_vec());
        Value::Array(args.map(Value::BulkString).collect())
    }

    /// Parses one value from the start of `buf`. Returns `None` if `buf` doesn't hold a
    /// complete value yet, otherwise the value and the number of bytes it took up.
    pub fn parse(buf: &[u8]) -> Result<Option<(Value, usize)>> {
        let Some((line, mut pos)) = read_line(buf) else {
            return Ok(None);
        };
        let Some((&kind, line)) = line.split_first() else {
            bail!("empty line");
        };

        let value = match kind {
            b'+' => Value::SimpleString(String::from_utf8(line.to_vec())?),
            b'-' => Value::Error(String::from_utf8(line.to_vec())?),
            b':' => Value::Integer(parse_int(line)?),
            b'$' => match parse_int(line)? {
                -1 => Value::NullBulkString,
                len => {
                    let end = pos + usize::try_from(len)?;
                    if buf.len() < end + 2 {
                        return Ok(None);
                    }
                    if &buf[end..end + 2] != b"\r\n" {
                        bail!("bulk string not terminated by CRLF");
                    }
                    let data = buf[pos..end].to_vec();
                    pos = end + 2;
                    Value::BulkString(data)
                }
            },
            b'*' => match parse_int(line)? {
                -1 => Value::NullArray,
                len => {
                    let mut items = Vec::new();
                    for _ in 0..usize::try_from(len)? {
                        let Some((item, used)) = Value::parse(&buf[pos..])? else {
                            return Ok(None);
                        };
                        items.push(item);
                        pos += used;
                    }
                    Value::Array(items)
                }
            },
            _ => bail!("unknown RESP type byte {:?}", kind as char),
        };
        Ok(Some((value, pos)))
    }

    pub fn encode(&self) -> Vec<u8> {
        match self {
            Value::SimpleString(s) => format!("+{s}\r\n").into_bytes(),
            Value::Error(msg) => format!("-{msg}\r\n").into_bytes(),
            Value::Integer(n) => format!(":{n}\r\n").into_bytes(),
            Value::BulkString(data) => {
                let mut out = format!("${}\r\n", data.len()).into_bytes();
                out.extend_from_slice(data);
                out.extend_from_slice(b"\r\n");
                out
            }
            Value::NullBulkString => b"$-1\r\n".to_vec(),
            Value::Array(items) => {
                let mut out = format!("*{}\r\n", items.len()).into_bytes();
                for item in items {
                    out.extend(item.encode());
                }
                out
            }
            Value::NullArray => b"*-1\r\n".to_vec(),
        }
    }
}

/// Parses a file from the start of `buf`, sent like a bulk string but with no CRLF at the
/// end, as a master sends its RDB file. Returns `None` if `buf` doesn't hold all of it yet,
/// otherwise the file's contents and the number of bytes it took up.
pub fn parse_file(buf: &[u8]) -> Result<Option<(Vec<u8>, usize)>> {
    let Some((line, pos)) = read_line(buf) else {
        return Ok(None);
    };
    let Some(len) = line.strip_prefix(b"$") else {
        bail!("expected a file, got {:?}", String::from_utf8_lossy(line));
    };
    let end = pos + usize::try_from(parse_int(len)?)?;
    if buf.len() < end {
        return Ok(None);
    }
    Ok(Some((buf[pos..end].to_vec(), end)))
}

/// Returns the first line in `buf` (without its CRLF) and the offset just past the CRLF.
fn read_line(buf: &[u8]) -> Option<(&[u8], usize)> {
    let end = buf.windows(2).position(|w| w == b"\r\n")?;
    Some((&buf[..end], end + 2))
}

fn parse_int(line: &[u8]) -> Result<i64> {
    Ok(std::str::from_utf8(line)?.parse()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ECHO_HEY: &[u8] = b"*2\r\n$4\r\nECHO\r\n$3\r\nhey\r\n";

    #[test]
    fn parses_command_array() {
        let expected = Value::Array(vec![
            Value::BulkString(b"ECHO".to_vec()),
            Value::BulkString(b"hey".to_vec()),
        ]);
        assert_eq!(
            Value::parse(ECHO_HEY).unwrap(),
            Some((expected, ECHO_HEY.len()))
        );
    }

    #[test]
    fn waits_for_more_data_on_partial_input() {
        for end in 0..ECHO_HEY.len() {
            assert_eq!(Value::parse(&ECHO_HEY[..end]).unwrap(), None);
        }
    }

    #[test]
    fn parses_only_the_first_of_several_values() {
        let parsed = Value::parse(b"+OK\r\n:42\r\n").unwrap();
        assert_eq!(parsed, Some((Value::SimpleString("OK".into()), 5)));
    }

    #[test]
    fn rejects_malformed_input() {
        assert!(Value::parse(b"?\r\n").is_err());
        assert!(Value::parse(b"$3\r\nheyX\r\n").is_err());
    }

    #[test]
    fn encodes_values() {
        assert_eq!(
            Value::BulkString(b"hey".to_vec()).encode(),
            b"$3\r\nhey\r\n"
        );
        assert_eq!(Value::SimpleString("PONG".into()).encode(), b"+PONG\r\n");
        let array = Value::Array(vec![Value::Integer(1), Value::NullBulkString]);
        assert_eq!(array.encode(), b"*2\r\n:1\r\n$-1\r\n");
    }

    #[test]
    fn builds_commands() {
        assert_eq!(Value::command(&["ECHO", "hey"]).encode(), ECHO_HEY);
        let args = [b"ECHO".to_vec(), b"hey".to_vec()];
        assert_eq!(Value::command(&args).encode(), ECHO_HEY);
    }

    #[test]
    fn parses_a_file_without_a_final_crlf() {
        let parsed = parse_file(b"$3\r\nabc+OK\r\n").unwrap();
        assert_eq!(parsed, Some((b"abc".to_vec(), 7)));
    }

    #[test]
    fn waits_for_the_rest_of_a_file() {
        let file = b"$3\r\nabc";
        for end in 0..file.len() {
            assert_eq!(parse_file(&file[..end]).unwrap(), None);
        }
    }

    #[test]
    fn rejects_what_isnt_a_file() {
        assert!(parse_file(b"+OK\r\n").is_err());
        assert!(parse_file(b"$abc\r\n").is_err());
        assert!(parse_file(b"$-1\r\n").is_err());
    }
}
