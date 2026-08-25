use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use crate::application::{Clipboard, CommandRepository, CommandRunner, OutputSanitizer};
use crate::domain::{CapturedCommand, CommandEntry};

pub struct ReoutUseCases<R, C, S, T> {
    repository: R,
    runner: C,
    clipboard: S,
    sanitizer: T,
}

impl<R, C, S, T> ReoutUseCases<R, C, S, T>
where
    R: CommandRepository,
    C: CommandRunner,
    S: Clipboard,
    T: OutputSanitizer,
{
    pub fn new(repository: R, runner: C, clipboard: S, sanitizer: T) -> Self {
        Self {
            repository,
            runner,
            clipboard,
            sanitizer,
        }
    }

    pub fn capture(&self, command: &str, cwd: String) -> Result<u8> {
        if should_ignore_command(command) {
            return Ok(0);
        }

        let started_at = unix_now();
        let (output, exit_code) = self.runner.run(command)?;
        let finished_at = unix_now();

        self.repository.insert(&CapturedCommand {
            command: command.to_string(),
            cwd,
            output,
            exit_code,
            started_at,
            finished_at,
        })?;

        Ok(exit_code_to_u8(exit_code))
    }

    pub fn latest_by_offset(&self, offset: usize) -> Result<CommandEntry> {
        if offset == 0 {
            bail!("offset must be greater than zero");
        }
        self.repository
            .get_by_offset(offset)?
            .with_context(|| format!("no captured output at offset {offset}"))
    }

    pub fn get_by_id(&self, id: i64) -> Result<CommandEntry> {
        self.repository
            .get_by_id(id)?
            .with_context(|| format!("no captured output with id {id}"))
    }

    pub fn list(&self, limit: usize) -> Result<Vec<CommandEntry>> {
        self.repository.list(limit)
    }

    pub fn find(&self, query: &str, limit: usize) -> Result<Vec<CommandEntry>> {
        self.repository.find(query, limit)
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.repository.delete(id)
    }

    pub fn clear(&self) -> Result<()> {
        self.repository.clear()
    }

    pub fn render_output(&self, entry: &CommandEntry, plain: bool) -> Result<Vec<u8>> {
        if plain {
            self.sanitizer.plain(&entry.output)
        } else {
            Ok(entry.output.as_bytes().to_vec())
        }
    }

    pub fn copy_output(&self, entry: &CommandEntry, plain: bool) -> Result<()> {
        let bytes = self.render_output(entry, plain)?;
        self.clipboard.copy(&bytes)
    }
}

fn should_ignore_command(command: &str) -> bool {
    let trimmed = command.trim_start();
    trimmed.is_empty() || trimmed == "reout" || trimmed.starts_with("reout ")
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs() as i64
}

fn exit_code_to_u8(code: i32) -> u8 {
    if (0..=255).contains(&code) {
        code as u8
    } else {
        1
    }
}
