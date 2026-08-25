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
}
