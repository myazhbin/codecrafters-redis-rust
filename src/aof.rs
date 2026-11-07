//! The append-only file (AOF): a log of every write, kept so that the server can replay it
//! at startup to get its data back.
//!
//! The AOF's files live in a directory of their own inside `dir`, named by
//! `appenddirname`.

use crate::config::Config;
use anyhow::{Context, Result};
use std::path::PathBuf;

/// The directory that holds the AOF's files.
pub fn dir(config: &Config) -> PathBuf {
    std::path::Path::new(&config.dir).join(&config.appenddirname)
}

/// Gets the AOF's directory ready at startup, creating it if it isn't there yet.
pub fn set_up(config: &Config) -> Result<()> {
    let dir = dir(config);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("couldn't create the append-only directory {}", dir.display()))
}
