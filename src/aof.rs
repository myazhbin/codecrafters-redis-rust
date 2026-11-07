//! The append-only file (AOF): a log of every write, kept so that the server can replay it
//! at startup to get its data back.
//!
//! The AOF's files live in a directory of their own inside `dir`, named by
//! `appenddirname`. Writes are appended to an incremental file named after
//! `appendfilename`: `<appendfilename>.1.incr.aof` for the first one.

use crate::config::Config;
use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::path::PathBuf;

/// The directory that holds the AOF's files.
pub fn dir(config: &Config) -> PathBuf {
    std::path::Path::new(&config.dir).join(&config.appenddirname)
}

/// The name of the first incremental AOF file.
pub fn first_incr_name(config: &Config) -> String {
    format!("{}.1.incr.aof", config.appendfilename)
}

/// Gets the AOF's files ready at startup, creating the directory and an empty
/// incremental file if they aren't there yet. A file that's already there is kept as is.
pub fn set_up(config: &Config) -> Result<()> {
    let dir = dir(config);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("couldn't create the append-only directory {}", dir.display()))?;
    let path = dir.join(first_incr_name(config));
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("couldn't create the append-only file {}", path.display()))?;
    Ok(())
}
