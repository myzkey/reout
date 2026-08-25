use anyhow::Result;

use crate::domain::{CapturedCommand, CommandEntry, CommandOutput};

pub trait CommandRepository {
    fn insert(&self, command: &CapturedCommand) -> Result<()>;
    fn get_by_offset(&self, offset: usize) -> Result<Option<CommandEntry>>;
    fn get_by_id(&self, id: i64) -> Result<Option<CommandEntry>>;
    fn list(&self, limit: usize) -> Result<Vec<CommandEntry>>;
    fn find(&self, query: &str, limit: usize) -> Result<Vec<CommandEntry>>;
    fn delete(&self, id: i64) -> Result<()>;
    fn clear(&self) -> Result<()>;
}

pub trait CommandRunner {
    fn run(&self, command: &str) -> Result<(CommandOutput, i32)>;
}

pub trait Clipboard {
    fn copy(&self, bytes: &[u8]) -> Result<()>;
}

pub trait OutputSanitizer {
    fn plain(&self, output: &CommandOutput) -> Result<Vec<u8>>;
}
