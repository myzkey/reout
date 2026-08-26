use anyhow::Result;

use crate::domain::{CapturedCommand, CommandEntry, CommandOutput, HistoryQuery};

pub trait CommandRepository {
    fn insert(&self, command: &CapturedCommand) -> Result<()>;
    fn get_by_offset(&self, offset: usize) -> Result<Option<CommandEntry>>;
    fn get_latest_matching(&self, query: &HistoryQuery) -> Result<Option<CommandEntry>>;
    fn get_by_id(&self, id: i64) -> Result<Option<CommandEntry>>;
    fn list(&self, limit: usize) -> Result<Vec<CommandEntry>>;
    fn query(&self, query: &HistoryQuery) -> Result<Vec<CommandEntry>>;
    fn find(&self, query: &str, limit: usize) -> Result<Vec<CommandEntry>>;
    fn prune_before(&self, finished_before: i64) -> Result<usize>;
    fn prune_to_max_entries(&self, max_entries: usize) -> Result<usize>;
    fn prune_to_max_bytes(&self, max_bytes: usize) -> Result<usize>;
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
