use std::path::PathBuf;

use anyhow::{Context, Result};

pub fn db_path() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("REOUT_DB") {
        return Ok(PathBuf::from(path));
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".local/share")))
        .context("cannot determine data directory")?;
    Ok(base.join("reout/reout.db"))
}
