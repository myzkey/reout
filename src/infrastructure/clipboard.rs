use anyhow::{Context, Result, bail};

use crate::application::Clipboard;

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn copy(&self, bytes: &[u8]) -> Result<()> {
        copy_bytes(bytes)
    }
}

fn copy_bytes(bytes: &[u8]) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let mut child = Command::new("pbcopy")
            .stdin(Stdio::piped())
            .spawn()
            .context("spawn pbcopy")?;
        child
            .stdin
            .as_mut()
            .context("open pbcopy stdin")?
            .write_all(bytes)?;
        let status = child.wait().context("wait for pbcopy")?;
        if !status.success() {
            bail!("pbcopy failed");
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = bytes;
        bail!("clipboard copy is only implemented for macOS in this MVP")
    }
}
