use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use crate::application::CommandRepository;
use crate::domain::{CapturedCommand, CommandEntry, CommandOutput, HistoryQuery};
use crate::infrastructure::paths::db_path;

pub struct SqliteCommandRepository {
    conn: Connection,
    path: PathBuf,
}

impl SqliteCommandRepository {
    pub fn open_default() -> Result<Self> {
        Self::open(db_path()?)
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            let existed = parent.exists();
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
            if !existed || parent.file_name().is_some_and(|name| name == "reout") {
                set_private_dir_permissions(parent)?;
            }
        }
        let conn = Connection::open(&path).with_context(|| format!("open {}", path.display()))?;
        let repository = Self { conn, path };
        repository.migrate()?;
        set_private_file_permissions(&repository.path)?;
        Ok(repository)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS commands (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                command TEXT NOT NULL,
                cwd TEXT NOT NULL,
                output BLOB NOT NULL,
                exit_code INTEGER NOT NULL,
                started_at INTEGER NOT NULL,
                finished_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_commands_finished_at ON commands(finished_at DESC);
            CREATE INDEX IF NOT EXISTS idx_commands_command ON commands(command);
            "#,
        )?;
        Ok(())
    }
}

impl CommandRepository for SqliteCommandRepository {
    fn insert(&self, command: &CapturedCommand) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO commands (command, cwd, output, exit_code, started_at, finished_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                command.command,
                command.cwd,
                command.output.as_bytes(),
                command.exit_code,
                command.started_at,
                command.finished_at
            ],
        )?;
        Ok(())
    }

    fn get_by_offset(&self, offset: usize) -> Result<Option<CommandEntry>> {
        if offset == 0 {
            bail!("offset must be greater than zero");
        }
        self.conn
            .query_row(
                r#"
                SELECT id, command, cwd, output, exit_code, started_at, finished_at
                FROM commands
                ORDER BY finished_at DESC, id DESC
                LIMIT 1 OFFSET ?1
                "#,
                params![(offset - 1) as i64],
                row_to_entry,
            )
            .optional()
            .map_err(Into::into)
    }

    fn get_latest_matching(&self, query: &HistoryQuery) -> Result<Option<CommandEntry>> {
        let mut query = query.clone();
        query.limit = 1;
        Ok(self.query(&query)?.into_iter().next())
    }

    fn get_by_id(&self, id: i64) -> Result<Option<CommandEntry>> {
        self.conn
            .query_row(
                r#"
                SELECT id, command, cwd, output, exit_code, started_at, finished_at
                FROM commands
                WHERE id = ?1
                "#,
                params![id],
                row_to_entry,
            )
            .optional()
            .map_err(Into::into)
    }

    fn list(&self, limit: usize) -> Result<Vec<CommandEntry>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, command, cwd, output, exit_code, started_at, finished_at
            FROM commands
            ORDER BY finished_at DESC, id DESC
            LIMIT ?1
            "#,
        )?;
        let entries = stmt
            .query_map(params![limit as i64], row_to_entry)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    fn query(&self, query: &HistoryQuery) -> Result<Vec<CommandEntry>> {
        let (sql, values) = build_history_query(query);
        let mut stmt = self.conn.prepare(&sql)?;
        let entries = stmt
            .query_map(params_from_iter(values), row_to_entry)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    fn find(&self, query: &str, limit: usize) -> Result<Vec<CommandEntry>> {
        let pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, command, cwd, output, exit_code, started_at, finished_at
            FROM commands
            WHERE command LIKE ?1 ESCAPE '\'
            ORDER BY finished_at DESC, id DESC
            LIMIT ?2
            "#,
        )?;
        let entries = stmt
            .query_map(params![pattern, limit as i64], row_to_entry)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    fn prune_before(&self, finished_before: i64) -> Result<usize> {
        let affected = self.conn.execute(
            "DELETE FROM commands WHERE finished_at < ?1",
            params![finished_before],
        )?;
        Ok(affected)
    }

    fn prune_to_max_entries(&self, max_entries: usize) -> Result<usize> {
        let affected = self.conn.execute(
            r#"
            DELETE FROM commands
            WHERE id NOT IN (
                SELECT id
                FROM commands
                ORDER BY finished_at DESC, id DESC
                LIMIT ?1
            )
            "#,
            params![max_entries as i64],
        )?;
        Ok(affected)
    }

    fn prune_to_max_bytes(&self, max_bytes: usize) -> Result<usize> {
        let rows = self.list_all()?;
        let mut total = 0usize;
        let mut delete_ids = Vec::new();

        for (index, entry) in rows.into_iter().enumerate() {
            total = total.saturating_add(entry.output.len());
            if index > 0 && total > max_bytes {
                delete_ids.push(entry.id);
            }
        }

        let mut deleted = 0usize;
        for id in delete_ids {
            deleted += self
                .conn
                .execute("DELETE FROM commands WHERE id = ?1", params![id])?;
        }

        Ok(deleted)
    }

    fn delete(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM commands WHERE id = ?1", params![id])?;
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        self.conn.execute("DELETE FROM commands", [])?;
        Ok(())
    }
}

impl SqliteCommandRepository {
    fn list_all(&self) -> Result<Vec<CommandEntry>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, command, cwd, output, exit_code, started_at, finished_at
            FROM commands
            ORDER BY finished_at DESC, id DESC
            "#,
        )?;
        let entries = stmt
            .query_map([], row_to_entry)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }
}

fn build_history_query(query: &HistoryQuery) -> (String, Vec<Value>) {
    let mut sql = String::from(
        r#"
        SELECT id, command, cwd, output, exit_code, started_at, finished_at
        FROM commands
        WHERE 1 = 1
        "#,
    );
    let mut values = Vec::<Value>::new();

    if let Some(command_contains) = &query.command_contains {
        sql.push_str(" AND command LIKE ? ESCAPE '\\'");
        values.push(Value::Text(format!("%{}%", escape_like(command_contains))));
    }

    if let Some(cwd) = &query.cwd {
        sql.push_str(" AND cwd = ?");
        values.push(Value::Text(cwd.clone()));
    }

    if query.failed_only {
        sql.push_str(" AND exit_code != 0");
    }

    if let Some(since) = query.since {
        sql.push_str(" AND finished_at >= ?");
        values.push(Value::Integer(since));
    }

    sql.push_str(" ORDER BY finished_at DESC, id DESC LIMIT ?");
    values.push(Value::Integer(query.limit as i64));

    (sql, values)
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<CommandEntry> {
    Ok(CommandEntry {
        id: row.get(0)?,
        command: row.get(1)?,
        cwd: row.get(2)?,
        output: CommandOutput::new(row.get(3)?),
        exit_code: row.get(4)?,
        started_at: row.get(5)?,
        finished_at: row.get(6)?,
    })
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> (tempfile::TempDir, SqliteCommandRepository) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("reout.db");
        let repo = SqliteCommandRepository::open(path).expect("open repo");
        (dir, repo)
    }

    fn captured(
        command: &str,
        cwd: &str,
        output: &[u8],
        exit_code: i32,
        started_at: i64,
        finished_at: i64,
    ) -> CapturedCommand {
        CapturedCommand {
            command: command.to_string(),
            cwd: cwd.to_string(),
            output: CommandOutput::new(output.to_vec()),
            exit_code,
            started_at,
            finished_at,
        }
    }

    fn seed(repo: &SqliteCommandRepository) {
        repo.insert(&captured("echo ok", "/repo", b"ok\n", 0, 10, 20))
            .expect("insert");
        repo.insert(&captured("cargo test", "/repo", b"error\n", 1, 30, 40))
            .expect("insert");
        repo.insert(&captured("git status", "/other", b"clean\n", 0, 50, 60))
            .expect("insert");
    }

    #[test]
    fn opens_database_and_exposes_path() {
        let (dir, repo) = repository();

        assert!(repo.path().starts_with(dir.path()));
        assert!(repo.path().exists());
    }

    #[test]
    fn inserts_and_reads_entries_by_id_and_offset() {
        let (_dir, repo) = repository();
        seed(&repo);

        let latest = repo.get_by_offset(1).expect("offset").expect("entry");
        assert_eq!(latest.command, "git status");
        assert_eq!(latest.output.as_bytes(), b"clean\n");

        let previous = repo.get_by_offset(2).expect("offset").expect("entry");
        assert_eq!(previous.command, "cargo test");

        let by_id = repo.get_by_id(1).expect("id").expect("entry");
        assert_eq!(by_id.command, "echo ok");
        assert_eq!(by_id.cwd, "/repo");
        assert_eq!(by_id.exit_code, 0);
        assert_eq!(by_id.started_at, 10);
        assert_eq!(by_id.finished_at, 20);
    }

    #[test]
    fn rejects_zero_offset() {
        let (_dir, repo) = repository();

        assert!(repo.get_by_offset(0).is_err());
    }

    #[test]
    fn lists_latest_first_with_limit() {
        let (_dir, repo) = repository();
        seed(&repo);

        let entries = repo.list(2).expect("list");

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "git status");
        assert_eq!(entries[1].command, "cargo test");
    }

    #[test]
    fn finds_command_substrings_and_escapes_like_wildcards() {
        let (_dir, repo) = repository();
        seed(&repo);
        repo.insert(&captured("echo 100% done", "/repo", b"done\n", 0, 70, 80))
            .expect("insert");

        assert_eq!(repo.find("test", 10).expect("find").len(), 1);
        assert_eq!(repo.find("100%", 10).expect("find").len(), 1);
        assert!(repo.find("100_", 10).expect("find").is_empty());
    }

    #[test]
    fn queries_by_command_cwd_failed_since_and_limit() {
        let (_dir, repo) = repository();
        seed(&repo);

        let query = HistoryQuery {
            command_contains: Some("test".to_string()),
            cwd: Some("/repo".to_string()),
            failed_only: true,
            since: Some(30),
            limit: 5,
        };
        let entries = repo.query(&query).expect("query");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "cargo test");

        let limited = repo
            .query(&HistoryQuery {
                limit: 1,
                ..HistoryQuery::default()
            })
            .expect("query");
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].command, "git status");
    }

    #[test]
    fn gets_latest_matching_entry() {
        let (_dir, repo) = repository();
        seed(&repo);

        let entry = repo
            .get_latest_matching(&HistoryQuery {
                cwd: Some("/repo".to_string()),
                limit: 100,
                ..HistoryQuery::default()
            })
            .expect("latest")
            .expect("entry");

        assert_eq!(entry.command, "cargo test");
    }

    #[test]
    fn deletes_clears_and_prunes_entries() {
        let (_dir, repo) = repository();
        seed(&repo);

        repo.delete(2).expect("delete");
        assert!(repo.get_by_id(2).expect("id").is_none());

        let pruned = repo.prune_before(50).expect("prune");
        assert_eq!(pruned, 1);
        assert_eq!(repo.list(10).expect("list").len(), 1);

        repo.clear().expect("clear");
        assert!(repo.list(10).expect("list").is_empty());
    }

    #[test]
    fn prunes_to_max_entries_by_deleting_oldest_rows() {
        let (_dir, repo) = repository();
        seed(&repo);

        let deleted = repo.prune_to_max_entries(2).expect("prune entries");
        let entries = repo.list(10).expect("list");

        assert_eq!(deleted, 1);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "git status");
        assert_eq!(entries[1].command, "cargo test");
    }

    #[test]
    fn prunes_to_max_bytes_by_keeping_latest_rows_within_budget() {
        let (_dir, repo) = repository();
        repo.insert(&captured("old-large", "/repo", b"12345", 0, 10, 10))
            .expect("insert");
        repo.insert(&captured("middle", "/repo", b"1234", 0, 20, 20))
            .expect("insert");
        repo.insert(&captured("latest", "/repo", b"12", 0, 30, 30))
            .expect("insert");

        let deleted = repo.prune_to_max_bytes(6).expect("prune bytes");
        let entries = repo.list(10).expect("list");

        assert_eq!(deleted, 1);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "latest");
        assert_eq!(entries[1].command, "middle");
    }

    #[cfg(unix)]
    #[test]
    fn database_file_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, repo) = repository();
        let mode = fs::metadata(repo.path())
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(mode, 0o600);
    }
}
