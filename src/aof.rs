//! The append-only file (AOF): a log of every write, kept so that the server can replay it
//! at startup to get its data back.
//!
//! The AOF's files live in a directory of their own inside `dir`, named by
//! `appenddirname`. A manifest there, `<appendfilename>.manifest`, lists them, one per
//! line, such as `file appendonly.aof.1.incr.aof seq 1 type i`. Type `i` is an
//! incremental file, which writes are appended to; there can also be a base file (`b`)
//! that the incremental ones carry on from, and old files kept as history (`h`).

use crate::config::{Config, Fsync};
use crate::resp::Value;
use anyhow::{Context, Result, bail};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The incremental AOF file, open for writes to be appended to it.
pub struct Aof {
    file: File,
    fsync: Fsync,
}

impl Aof {
    /// Opens the incremental file listed in `entries` for appending. With `everysec`, a
    /// thread flushes it to disk once a second.
    pub fn open(config: &Config, entries: &[ManifestEntry]) -> Result<Aof> {
        let incr = incr_file(entries).context("the manifest lists no incremental file")?;
        let path = dir(config).join(&incr.file);
        let file = OpenOptions::new()
            .append(true)
            .open(&path)
            .with_context(|| format!("couldn't open the append-only file {}", path.display()))?;
        if config.appendfsync == Fsync::Everysec {
            let file = file.try_clone()?;
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    if let Err(e) = file.sync_data() {
                        eprintln!("couldn't flush the append-only file: {e}");
                    }
                }
            });
        }
        Ok(Aof {
            file,
            fsync: config.appendfsync,
        })
    }

    /// Appends a write, in RESP, as clients send commands. With `always`, it's flushed to
    /// disk straight away, which is before the client that sent it gets a reply.
    pub fn append(&mut self, command: &[Vec<u8>]) {
        let data = Value::command(command).encode();
        let mut result = self.file.write_all(&data);
        if result.is_ok() && self.fsync == Fsync::Always {
            result = self.file.sync_data();
        }
        if let Err(e) = result {
            eprintln!("couldn't write to the append-only file: {e}");
        }
    }
}

/// How a write is logged: as it's sent to replicas, except that a SET's expiry in seconds
/// or milliseconds from now becomes the Unix time it'll expire at (`now_ms` being now), so
/// that replaying the file later doesn't put the expiry off.
pub fn logged_form<T: AsRef<[u8]>>(command: &[T], now_ms: u64) -> Vec<Vec<u8>> {
    let mut command: Vec<Vec<u8>> = command.iter().map(|arg| arg.as_ref().to_vec()).collect();
    if let [name, _, _, unit, amount] = &command[..]
        && name.eq_ignore_ascii_case(b"SET")
        && let Ok(amount) = std::str::from_utf8(amount).unwrap_or_default().parse::<u64>()
    {
        let at = match unit.to_ascii_uppercase().as_slice() {
            b"EX" => Some(now_ms.saturating_add(amount.saturating_mul(1000))),
            b"PX" => Some(now_ms.saturating_add(amount)),
            _ => None,
        };
        if let Some(at) = at {
            command[3] = b"PXAT".to_vec();
            command[4] = at.to_string().into_bytes();
        }
    }
    command
}

/// The directory that holds the AOF's files.
pub fn dir(config: &Config) -> PathBuf {
    Path::new(&config.dir).join(&config.appenddirname)
}

/// Where the manifest is.
fn manifest_path(config: &Config) -> PathBuf {
    dir(config).join(format!("{}.manifest", config.appendfilename))
}

/// Gets the AOF's files ready at startup. A manifest that's already there says which
/// file is the incremental one; otherwise the server starts with a new
/// `<appendfilename>.1.incr.aof` and a manifest listing it. Returns the manifest's files.
pub fn set_up(config: &Config) -> Result<Vec<ManifestEntry>> {
    let dir = dir(config);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("couldn't create the append-only directory {}", dir.display()))?;
    let manifest_path = manifest_path(config);
    let entries = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => parse_manifest(&text)
            .with_context(|| format!("couldn't read the manifest {}", manifest_path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let entries = vec![ManifestEntry {
                file: format!("{}.1.incr.aof", config.appendfilename),
                seq: 1,
                kind: FileKind::Incr,
            }];
            // The file goes first, so that the manifest never lists one that isn't there.
            create_file(&dir.join(&entries[0].file))?;
            std::fs::write(&manifest_path, format_manifest(&entries)).with_context(|| {
                format!("couldn't write the manifest {}", manifest_path.display())
            })?;
            entries
        }
        Err(e) => {
            return Err(e)
                .with_context(|| format!("couldn't read the manifest {}", manifest_path.display()));
        }
    };
    let Some(incr) = incr_file(&entries) else {
        bail!("the manifest {} lists no incremental file", manifest_path.display());
    };
    create_file(&dir.join(&incr.file))?;
    Ok(entries)
}

/// Creates an empty file at `path`, unless there's one there already.
fn create_file(path: &Path) -> Result<()> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("couldn't create the append-only file {}", path.display()))?;
    Ok(())
}

/// One line of the manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct ManifestEntry {
    /// The file's name, in the append-only directory.
    pub file: String,
    pub seq: u64,
    pub kind: FileKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FileKind {
    Base,
    Incr,
    History,
}

/// The incremental file that writes go to: the last one in the manifest.
pub fn incr_file(entries: &[ManifestEntry]) -> Option<&ManifestEntry> {
    entries.iter().rev().find(|entry| entry.kind == FileKind::Incr)
}

/// Parses a manifest: one file per line, given as `key value` pairs separated by spaces
/// (`file <name> seq <n> type <b|i|h>`, in any order). Blank lines and lines starting
/// with `#` are skipped, and a name with spaces or other special characters is quoted.
pub fn parse_manifest(text: &str) -> Result<Vec<ManifestEntry>> {
    let mut entries = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parse_line = || -> Result<ManifestEntry> {
            let tokens = split_args(line)?;
            if tokens.len() % 2 != 0 {
                bail!("a key without a value");
            }
            let (mut file, mut seq, mut kind) = (None, None, None);
            for pair in tokens.chunks(2) {
                let value = &pair[1];
                match pair[0].as_str() {
                    "file" => file = Some(value.clone()),
                    "seq" => seq = Some(value.parse().context("seq isn't a number")?),
                    "type" => {
                        kind = Some(match value.as_str() {
                            "b" => FileKind::Base,
                            "i" => FileKind::Incr,
                            "h" => FileKind::History,
                            _ => bail!("unknown file type {value:?}"),
                        })
                    }
                    // Other keys may be added by later versions of Redis.
                    _ => {}
                }
            }
            match (file, seq, kind) {
                (Some(file), Some(seq), Some(kind)) => Ok(ManifestEntry { file, seq, kind }),
                _ => bail!("it needs a file, seq and type"),
            }
        };
        entries.push(parse_line().with_context(|| format!("line {}: {line:?}", number + 1))?);
    }
    Ok(entries)
}

/// The manifest that lists `entries`, one per line, each line ending in a newline.
pub fn format_manifest(entries: &[ManifestEntry]) -> String {
    let mut text = String::new();
    for entry in entries {
        let kind = match entry.kind {
            FileKind::Base => "b",
            FileKind::Incr => "i",
            FileKind::History => "h",
        };
        let file = quote_if_needed(&entry.file);
        text += &format!("file {file} seq {} type {kind}\n", entry.seq);
    }
    text
}

/// A name as the manifest holds it: as it is, unless it has spaces, quotes or other
/// special characters, in which case it's quoted with those escaped.
fn quote_if_needed(name: &str) -> String {
    let plain = |c: char| c.is_ascii_graphic() && c != '"' && c != '\'' && c != '\\';
    if !name.is_empty() && name.chars().all(plain) {
        return name.to_string();
    }
    let mut quoted = String::from("\"");
    for c in name.chars() {
        match c {
            '"' => quoted += "\\\"",
            '\\' => quoted += "\\\\",
            '\n' => quoted += "\\n",
            '\r' => quoted += "\\r",
            '\t' => quoted += "\\t",
            c if c.is_control() => quoted += &format!("\\x{:02x}", c as u32),
            c => quoted.push(c),
        }
    }
    quoted + "\""
}

/// Splits a manifest line into words, as Redis does: on spaces, with a word in double
/// quotes able to hold spaces and escapes like `\"`, and one in single quotes taken as is
/// (apart from `\'`).
fn split_args(line: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&first) = chars.peek() else {
            return Ok(words);
        };
        let mut word = String::new();
        if first == '"' || first == '\'' {
            chars.next();
            loop {
                match (chars.next(), first) {
                    (None, _) => bail!("an unclosed quote"),
                    (Some(c), q) if c == q => break,
                    (Some('\\'), '"') => match chars.next() {
                        Some('n') => word.push('\n'),
                        Some('r') => word.push('\r'),
                        Some('t') => word.push('\t'),
                        Some('x') => {
                            let hex: String = chars.by_ref().take(2).collect();
                            let byte = u8::from_str_radix(&hex, 16).context("a bad \\x escape")?;
                            word.push(char::from(byte));
                        }
                        Some(c) => word.push(c),
                        None => bail!("an unclosed quote"),
                    },
                    (Some('\\'), '\'') if chars.peek() == Some(&'\'') => {
                        word.push('\'');
                        chars.next();
                    }
                    (Some(c), _) => word.push(c),
                }
            }
            // A closing quote has to end the word.
            if chars.peek().is_some_and(|c| !c.is_whitespace()) {
                bail!("a closing quote not followed by a space");
            }
        } else {
            while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                word.push(c);
            }
        }
        words.push(word);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(file: &str, seq: u64, kind: FileKind) -> ManifestEntry {
        ManifestEntry {
            file: file.into(),
            seq,
            kind,
        }
    }

    fn logged(command: &[&str]) -> Vec<String> {
        let logged = logged_form(command, 1_000_000);
        logged.into_iter().map(|arg| String::from_utf8(arg).unwrap()).collect()
    }

    #[test]
    fn logs_writes_as_they_came_except_for_relative_expiries() {
        assert_eq!(logged(&["SET", "k", "v"]), ["SET", "k", "v"]);
        assert_eq!(logged(&["INCR", "k"]), ["INCR", "k"]);
        assert_eq!(logged(&["SET", "k", "v", "EX", "10"]), ["SET", "k", "v", "PXAT", "1010000"]);
        assert_eq!(logged(&["set", "k", "v", "px", "500"]), ["set", "k", "v", "PXAT", "1000500"]);
        // Already absolute.
        assert_eq!(logged(&["SET", "k", "v", "PXAT", "5"]), ["SET", "k", "v", "PXAT", "5"]);
        assert_eq!(logged(&["SET", "k", "v", "EXAT", "5"]), ["SET", "k", "v", "EXAT", "5"]);
    }

    #[test]
    fn formats_a_new_manifest() {
        let entries = [entry("appendonly.aof.1.incr.aof", 1, FileKind::Incr)];
        assert_eq!(
            format_manifest(&entries),
            "file appendonly.aof.1.incr.aof seq 1 type i\n"
        );
    }

    #[test]
    fn parses_manifests() {
        let text = "file appendonly.aof.1.base.rdb seq 1 type b\n\
                    file appendonly.aof.1.incr.aof seq 1 type h\n\
                    # a comment\n\
                    \n\
                    type i  seq 2 file appendonly.aof.2.incr.aof\n";
        let entries = parse_manifest(text).unwrap();
        assert_eq!(
            entries,
            [
                entry("appendonly.aof.1.base.rdb", 1, FileKind::Base),
                entry("appendonly.aof.1.incr.aof", 1, FileKind::History),
                entry("appendonly.aof.2.incr.aof", 2, FileKind::Incr),
            ]
        );
        assert_eq!(incr_file(&entries), Some(&entries[2]));
        assert_eq!(parse_manifest("").unwrap(), []);
    }

    #[test]
    fn quoted_names_round_trip() {
        for name in ["my file.aof", "quote\".aof", "back\\slash", "tab\there", "x'y"] {
            let entries = [entry(name, 3, FileKind::Incr)];
            let text = format_manifest(&entries);
            assert_eq!(parse_manifest(&text).unwrap(), entries, "{text}");
        }
        let entries = parse_manifest("file 'single quoted' seq 1 type i").unwrap();
        assert_eq!(entries[0].file, "single quoted");
    }

    #[test]
    fn rejects_bad_manifests() {
        for text in [
            "file a.aof seq 1",
            "file a.aof seq x type i",
            "file a.aof seq 1 type z",
            "file a.aof seq 1 type",
            "file \"a.aof seq 1 type i",
            "file \"a\"b seq 1 type i",
        ] {
            assert!(parse_manifest(text).is_err(), "{text}");
        }
    }
}
