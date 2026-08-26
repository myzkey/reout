#[derive(Debug, Clone)]
pub struct CommandEntry {
    pub id: i64,
    pub command: String,
    pub cwd: String,
    pub output: CommandOutput,
    pub exit_code: i32,
    pub started_at: i64,
    pub finished_at: i64,
}

#[derive(Debug, Clone)]
pub struct CapturedCommand {
    pub command: String,
    pub cwd: String,
    pub output: CommandOutput,
    pub exit_code: i32,
    pub started_at: i64,
    pub finished_at: i64,
}

#[derive(Debug, Clone)]
pub struct CommandOutput {
    bytes: Vec<u8>,
}

impl CommandOutput {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn truncate(&mut self, max_bytes: usize) {
        if self.bytes.len() > max_bytes {
            self.bytes.truncate(max_bytes);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HistoryQuery {
    pub command_contains: Option<String>,
    pub cwd: Option<String>,
    pub failed_only: bool,
    pub since: Option<i64>,
    pub limit: usize,
}

#[derive(Debug, Clone, Default)]
pub struct RetentionPolicy {
    pub max_age_seconds: Option<i64>,
    pub max_entries: Option<usize>,
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct CapturePolicy {
    pub max_output_bytes: Option<usize>,
}
