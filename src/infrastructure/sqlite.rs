use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};

use crate::application::CommandRepository;
use crate::domain::{CapturedCommand, CommandEntry, CommandOutput};
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
