use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use crate::application::{Clipboard, CommandRepository, CommandRunner, OutputSanitizer};
use crate::domain::{
    CapturePolicy, CapturedCommand, CommandEntry, CommandOutput, HistoryQuery, RetentionPolicy,
};

pub struct ReoutUseCases<R, C, S, T> {
    repository: R,
    runner: C,
    clipboard: S,
    sanitizer: T,
    ignored_commands: Vec<String>,
    retention: RetentionPolicy,
    capture_policy: CapturePolicy,
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
            ignored_commands: Vec::new(),
            retention: RetentionPolicy::default(),
            capture_policy: CapturePolicy::default(),
        }
    }

    pub fn with_ignored_commands(mut self, patterns: Vec<String>) -> Self {
        self.ignored_commands = patterns;
        self
    }

    pub fn with_retention_policy(mut self, retention: RetentionPolicy) -> Self {
        self.retention = retention;
        self
    }

    pub fn with_capture_policy(mut self, capture_policy: CapturePolicy) -> Self {
        self.capture_policy = capture_policy;
        self
    }

    pub fn capture(&self, command: &str, cwd: String) -> Result<u8> {
        if should_ignore_command(command) || matches_any_pattern(command, &self.ignored_commands) {
            return Ok(0);
        }

        let started_at = unix_now();
        let (mut output, exit_code) = self.runner.run(command)?;
        self.apply_capture_policy(&mut output);
        let finished_at = unix_now();

        self.repository.insert(&CapturedCommand {
            command: command.to_string(),
            cwd,
            output,
            exit_code,
            started_at,
            finished_at,
        })?;
        self.apply_retention(finished_at)?;

        Ok(exit_code_to_u8(exit_code))
    }

    pub fn import_output(
        &self,
        command: &str,
        cwd: String,
        output: Vec<u8>,
        exit_code: i32,
    ) -> Result<u8> {
        if should_ignore_command(command) || matches_any_pattern(command, &self.ignored_commands) {
            return Ok(0);
        }

        let finished_at = unix_now();
        let mut output = CommandOutput::new(output);
        self.apply_capture_policy(&mut output);

        self.repository.insert(&CapturedCommand {
            command: command.to_string(),
            cwd,
            output,
            exit_code,
            started_at: finished_at,
            finished_at,
        })?;
        self.apply_retention(finished_at)?;

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

    pub fn latest_matching(&self, query: &HistoryQuery) -> Result<CommandEntry> {
        self.repository
            .get_latest_matching(query)?
            .with_context(|| "no captured output matched the query")
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

    pub fn query(&self, query: &HistoryQuery) -> Result<Vec<CommandEntry>> {
        self.repository.query(query)
    }

    pub fn prune_before(&self, finished_before: i64) -> Result<usize> {
        self.repository.prune_before(finished_before)
    }

    pub fn apply_retention(&self, now: i64) -> Result<usize> {
        let mut deleted = 0;
        if let Some(max_age_seconds) = self.retention.max_age_seconds {
            deleted += self.repository.prune_before(now - max_age_seconds)?;
        }
        if let Some(max_entries) = self.retention.max_entries {
            deleted += self.repository.prune_to_max_entries(max_entries)?;
        }
        if let Some(max_bytes) = self.retention.max_bytes {
            deleted += self.repository.prune_to_max_bytes(max_bytes)?;
        }
        Ok(deleted)
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

    pub fn copy_bytes(&self, bytes: &[u8]) -> Result<()> {
        self.clipboard.copy(bytes)
    }

    fn apply_capture_policy(&self, output: &mut CommandOutput) {
        if let Some(max_output_bytes) = self.capture_policy.max_output_bytes {
            output.truncate(max_output_bytes);
        }
    }
}

fn should_ignore_command(command: &str) -> bool {
    if std::env::var_os("REOUT_DISABLED").is_some() {
        return true;
    }

    let trimmed = command.trim_start();
    trimmed.is_empty() || trimmed == "reout" || trimmed.starts_with("reout ")
}

fn matches_any_pattern(command: &str, patterns: &[String]) -> bool {
    patterns
        .iter()
        .any(|pattern| wildcard_match(pattern, command))
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    let pattern_parts = pattern.split('*').collect::<Vec<_>>();
    if pattern_parts.len() == 1 {
        return pattern == value;
    }

    let mut remainder = value;
    let starts_with_wildcard = pattern.starts_with('*');
    let ends_with_wildcard = pattern.ends_with('*');

    for (index, part) in pattern_parts
        .iter()
        .filter(|part| !part.is_empty())
        .enumerate()
    {
        if index == 0 && !starts_with_wildcard {
            let Some(stripped) = remainder.strip_prefix(part) else {
                return false;
            };
            remainder = stripped;
            continue;
        }

        let Some(found_at) = remainder.find(part) else {
            return false;
        };
        remainder = &remainder[found_at + part.len()..];
    }

    ends_with_wildcard
        || pattern_parts
            .last()
            .is_none_or(|part| remainder.is_empty() || part.is_empty())
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use anyhow::Result;

    use super::*;
    use crate::application::{Clipboard, CommandRepository, CommandRunner, OutputSanitizer};
    use crate::domain::{CapturedCommand, CommandOutput};

    #[derive(Default)]
    struct FakeRepository {
        entries: RefCell<Vec<CapturedCommand>>,
        command_entries: RefCell<Vec<CommandEntry>>,
        deleted_ids: RefCell<Vec<i64>>,
        cleared: RefCell<bool>,
        pruned_before: RefCell<Option<i64>>,
        pruned_to_entries: RefCell<Option<usize>>,
        pruned_to_bytes: RefCell<Option<usize>>,
    }

    impl CommandRepository for FakeRepository {
        fn insert(&self, command: &CapturedCommand) -> Result<()> {
            self.entries.borrow_mut().push(command.clone());
            Ok(())
        }

        fn get_by_offset(&self, offset: usize) -> Result<Option<CommandEntry>> {
            Ok(self.command_entries.borrow().get(offset - 1).cloned())
        }

        fn get_latest_matching(&self, query: &HistoryQuery) -> Result<Option<CommandEntry>> {
            Ok(self.query(query)?.into_iter().next())
        }

        fn get_by_id(&self, id: i64) -> Result<Option<CommandEntry>> {
            Ok(self
                .command_entries
                .borrow()
                .iter()
                .find(|entry| entry.id == id)
                .cloned())
        }

        fn list(&self, limit: usize) -> Result<Vec<CommandEntry>> {
            Ok(self
                .command_entries
                .borrow()
                .iter()
                .take(limit)
                .cloned()
                .collect())
        }

        fn query(&self, query: &HistoryQuery) -> Result<Vec<CommandEntry>> {
            let entries = self
                .command_entries
                .borrow()
                .iter()
                .filter(|entry| {
                    query
                        .command_contains
                        .as_ref()
                        .is_none_or(|value| entry.command.contains(value))
                })
                .filter(|entry| query.cwd.as_ref().is_none_or(|cwd| &entry.cwd == cwd))
                .filter(|entry| !query.failed_only || entry.exit_code != 0)
                .filter(|entry| query.since.is_none_or(|since| entry.finished_at >= since))
                .take(query.limit)
                .cloned()
                .collect();
            Ok(entries)
        }

        fn find(&self, _query: &str, _limit: usize) -> Result<Vec<CommandEntry>> {
            Ok(Vec::new())
        }

        fn prune_before(&self, finished_before: i64) -> Result<usize> {
            *self.pruned_before.borrow_mut() = Some(finished_before);
            Ok(2)
        }

        fn prune_to_max_entries(&self, max_entries: usize) -> Result<usize> {
            *self.pruned_to_entries.borrow_mut() = Some(max_entries);
            Ok(3)
        }

        fn prune_to_max_bytes(&self, max_bytes: usize) -> Result<usize> {
            *self.pruned_to_bytes.borrow_mut() = Some(max_bytes);
            Ok(4)
        }

        fn delete(&self, id: i64) -> Result<()> {
            self.deleted_ids.borrow_mut().push(id);
            Ok(())
        }

        fn clear(&self) -> Result<()> {
            *self.cleared.borrow_mut() = true;
            Ok(())
        }
    }

    struct FakeRunner;

    impl CommandRunner for FakeRunner {
        fn run(&self, _command: &str) -> Result<(CommandOutput, i32)> {
            Ok((CommandOutput::new(b"ok\n".to_vec()), 7))
        }
    }

    #[derive(Default)]
    struct FakeClipboard {
        copied: RefCell<Vec<u8>>,
    }

    impl Clipboard for FakeClipboard {
        fn copy(&self, bytes: &[u8]) -> Result<()> {
            self.copied.borrow_mut().extend_from_slice(bytes);
            Ok(())
        }
    }

    struct FakeSanitizer;

    impl OutputSanitizer for FakeSanitizer {
        fn plain(&self, output: &CommandOutput) -> Result<Vec<u8>> {
            Ok(output.as_bytes().to_ascii_uppercase())
        }
    }

    fn command_entry(
        id: i64,
        command: &str,
        cwd: &str,
        exit_code: i32,
        finished_at: i64,
    ) -> CommandEntry {
        CommandEntry {
            id,
            command: command.to_string(),
            cwd: cwd.to_string(),
            output: CommandOutput::new(format!("output-{id}\n").into_bytes()),
            exit_code,
            started_at: finished_at - 1,
            finished_at,
        }
    }

    #[test]
    fn capture_runs_command_and_persists_result() {
        let app = ReoutUseCases::new(
            FakeRepository::default(),
            FakeRunner,
            FakeClipboard::default(),
            FakeSanitizer,
        );

        let exit_code = app
            .capture("cargo test", "/tmp/project".to_string())
            .expect("capture");

        assert_eq!(exit_code, 7);
        let saved = app.repository.entries.borrow();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].command, "cargo test");
        assert_eq!(saved[0].cwd, "/tmp/project");
        assert_eq!(saved[0].output.as_bytes(), b"ok\n");
        assert_eq!(saved[0].exit_code, 7);
    }

    #[test]
    fn capture_ignores_reout_commands() {
        let app = ReoutUseCases::new(
            FakeRepository::default(),
            FakeRunner,
            FakeClipboard::default(),
            FakeSanitizer,
        );

        let exit_code = app
            .capture("reout -c", "/tmp/project".to_string())
            .expect("capture");

        assert_eq!(exit_code, 0);
        assert!(app.repository.entries.borrow().is_empty());
    }

    #[test]
    fn capture_ignores_configured_wildcard_patterns() {
        let app = ReoutUseCases::new(
            FakeRepository::default(),
            FakeRunner,
            FakeClipboard::default(),
            FakeSanitizer,
        )
        .with_ignored_commands(vec!["secret*".to_string(), "* token".to_string()]);

        assert_eq!(
            app.capture("secret command", "/tmp/project".to_string())
                .expect("capture"),
            0
        );
        assert_eq!(
            app.capture("aws token", "/tmp/project".to_string())
                .expect("capture"),
            0
        );
        assert!(app.repository.entries.borrow().is_empty());
    }

    #[test]
    fn import_output_persists_supplied_output_without_running_command() {
        let app = ReoutUseCases::new(
            FakeRepository::default(),
            FakeRunner,
            FakeClipboard::default(),
            FakeSanitizer,
        );

        let exit_code = app
            .import_output("alias hi", "/tmp".to_string(), b"hello\n".to_vec(), 9)
            .expect("import");

        assert_eq!(exit_code, 9);
        let saved = app.repository.entries.borrow();
        assert_eq!(saved[0].command, "alias hi");
        assert_eq!(saved[0].cwd, "/tmp");
        assert_eq!(saved[0].output.as_bytes(), b"hello\n");
        assert_eq!(saved[0].exit_code, 9);
    }

    #[test]
    fn capture_truncates_output_when_configured() {
        let app = ReoutUseCases::new(
            FakeRepository::default(),
            FakeRunner,
            FakeClipboard::default(),
            FakeSanitizer,
        )
        .with_capture_policy(CapturePolicy {
            max_output_bytes: Some(2),
        });

        app.capture("cargo test", "/tmp/project".to_string())
            .expect("capture");

        let saved = app.repository.entries.borrow();
        assert_eq!(saved[0].output.as_bytes(), b"ok");
    }

    #[test]
    fn import_truncates_output_when_configured() {
        let app = ReoutUseCases::new(
            FakeRepository::default(),
            FakeRunner,
            FakeClipboard::default(),
            FakeSanitizer,
        )
        .with_capture_policy(CapturePolicy {
            max_output_bytes: Some(3),
        });

        app.import_output("alias hi", "/tmp".to_string(), b"abcdef".to_vec(), 0)
            .expect("import");

        let saved = app.repository.entries.borrow();
        assert_eq!(saved[0].output.as_bytes(), b"abc");
    }

    #[test]
    fn exit_codes_outside_process_range_return_failure_to_shell() {
        let app = ReoutUseCases::new(
            FakeRepository::default(),
            FakeRunner,
            FakeClipboard::default(),
            FakeSanitizer,
        );

        assert_eq!(
            app.import_output("weird", "/tmp".to_string(), Vec::new(), 300)
                .expect("import"),
            1
        );
    }

    #[test]
    fn retrieves_entries_by_offset_id_and_query() {
        let repository = FakeRepository::default();
        repository.command_entries.borrow_mut().extend([
            command_entry(3, "cargo test", "/repo", 1, 300),
            command_entry(2, "git status", "/repo", 0, 200),
            command_entry(1, "pnpm test", "/web", 0, 100),
        ]);
        let app = ReoutUseCases::new(
            repository,
            FakeRunner,
            FakeClipboard::default(),
            FakeSanitizer,
        );

        assert_eq!(app.latest_by_offset(2).unwrap().id, 2);
        assert_eq!(app.get_by_id(1).unwrap().command, "pnpm test");
        assert!(app.latest_by_offset(0).is_err());

        let query = HistoryQuery {
            command_contains: Some("test".to_string()),
            cwd: Some("/repo".to_string()),
            failed_only: true,
            since: Some(250),
            limit: 10,
        };
        assert_eq!(app.latest_matching(&query).unwrap().id, 3);
        assert_eq!(app.query(&query).unwrap().len(), 1);
    }

    #[test]
    fn returns_errors_when_requested_entries_are_missing() {
        let app = ReoutUseCases::new(
            FakeRepository::default(),
            FakeRunner,
            FakeClipboard::default(),
            FakeSanitizer,
        );

        assert!(app.latest_by_offset(1).is_err());
        assert!(app.get_by_id(404).is_err());
        assert!(
            app.latest_matching(&HistoryQuery {
                limit: 1,
                ..HistoryQuery::default()
            })
            .is_err()
        );
    }

    #[test]
    fn copy_output_uses_plain_rendering_when_requested() {
        let clipboard = FakeClipboard::default();
        let app = ReoutUseCases::new(
            FakeRepository::default(),
            FakeRunner,
            clipboard,
            FakeSanitizer,
        );
        let entry = command_entry(1, "echo ok", "/tmp", 0, 2);

        app.copy_output(&entry, true).expect("copy");

        assert_eq!(app.clipboard.copied.borrow().as_slice(), b"OUTPUT-1\n");
    }

    #[test]
    fn copy_bytes_delegates_to_clipboard() {
        let clipboard = FakeClipboard::default();
        let app = ReoutUseCases::new(
            FakeRepository::default(),
            FakeRunner,
            clipboard,
            FakeSanitizer,
        );

        app.copy_bytes(b"raw").expect("copy");

        assert_eq!(app.clipboard.copied.borrow().as_slice(), b"raw");
    }

    #[test]
    fn delete_clear_and_prune_delegate_to_repository() {
        let app = ReoutUseCases::new(
            FakeRepository::default(),
            FakeRunner,
            FakeClipboard::default(),
            FakeSanitizer,
        );

        app.delete(42).expect("delete");
        app.clear().expect("clear");
        let pruned = app.prune_before(123).expect("prune");

        assert_eq!(app.repository.deleted_ids.borrow().as_slice(), &[42]);
        assert!(*app.repository.cleared.borrow());
        assert_eq!(*app.repository.pruned_before.borrow(), Some(123));
        assert_eq!(pruned, 2);
    }

    #[test]
    fn apply_retention_runs_all_configured_pruners() {
        let app = ReoutUseCases::new(
            FakeRepository::default(),
            FakeRunner,
            FakeClipboard::default(),
            FakeSanitizer,
        )
        .with_retention_policy(RetentionPolicy {
            max_age_seconds: Some(10),
            max_entries: Some(100),
            max_bytes: Some(1024),
        });

        let deleted = app.apply_retention(1000).expect("retention");

        assert_eq!(deleted, 9);
        assert_eq!(*app.repository.pruned_before.borrow(), Some(990));
        assert_eq!(*app.repository.pruned_to_entries.borrow(), Some(100));
        assert_eq!(*app.repository.pruned_to_bytes.borrow(), Some(1024));
    }

    #[test]
    fn capture_applies_retention_after_insert() {
        let app = ReoutUseCases::new(
            FakeRepository::default(),
            FakeRunner,
            FakeClipboard::default(),
            FakeSanitizer,
        )
        .with_retention_policy(RetentionPolicy {
            max_age_seconds: None,
            max_entries: Some(1),
            max_bytes: None,
        });

        app.capture("cargo test", "/tmp/project".to_string())
            .expect("capture");

        assert_eq!(*app.repository.pruned_to_entries.borrow(), Some(1));
    }

    #[test]
    fn render_output_uses_sanitizer_for_plain_output() {
        let app = ReoutUseCases::new(
            FakeRepository::default(),
            FakeRunner,
            FakeClipboard::default(),
            FakeSanitizer,
        );
        let entry = CommandEntry {
            id: 1,
            command: "echo ok".to_string(),
            cwd: "/tmp".to_string(),
            output: CommandOutput::new(b"ok\n".to_vec()),
            exit_code: 0,
            started_at: 1,
            finished_at: 2,
        };

        let rendered = app.render_output(&entry, true).expect("render");

        assert_eq!(rendered, b"OK\n");
    }
}
