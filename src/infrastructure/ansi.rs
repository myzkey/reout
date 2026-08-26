use anyhow::Result;

use crate::application::OutputSanitizer;
use crate::domain::CommandOutput;

#[derive(Debug, Default, Clone, Copy)]
pub struct AnsiOutputSanitizer;

impl OutputSanitizer for AnsiOutputSanitizer {
    fn plain(&self, output: &CommandOutput) -> Result<Vec<u8>> {
        let cleaned = clean_terminal_controls(output.as_bytes(), true);
        Ok(clean_terminal_controls(
            &strip_ansi_escapes::strip(&cleaned),
            false,
        ))
    }
}

fn clean_terminal_controls(bytes: &[u8], keep_escape: bool) -> Vec<u8> {
    let mut cleaned = Vec::with_capacity(bytes.len());

    for &byte in bytes {
        match byte {
            b'\n' | b'\r' | b'\t' => cleaned.push(byte),
            0x1b if keep_escape => cleaned.push(byte),
            0x08 | 0x7f => {
                cleaned.pop();
            }
            0x00..=0x1f => {}
            _ => cleaned.push(byte),
        }
    }

    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi_and_control_bytes() {
        let sanitizer = AnsiOutputSanitizer;
        let output = CommandOutput::new(b"\x1b[31mred\x1b[0m\n^D\x08\x08ok\x04\n".to_vec());

        let plain = sanitizer.plain(&output).expect("plain output");

        assert_eq!(plain, b"red\nok\n");
    }
}
