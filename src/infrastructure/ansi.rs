use anyhow::Result;

use crate::application::OutputSanitizer;
use crate::domain::CommandOutput;

#[derive(Debug, Default, Clone, Copy)]
pub struct AnsiOutputSanitizer;

impl OutputSanitizer for AnsiOutputSanitizer {
    fn plain(&self, output: &CommandOutput) -> Result<Vec<u8>> {
        Ok(strip_ansi_escapes::strip(output.as_bytes()))
    }
}
