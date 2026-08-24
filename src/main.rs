use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::{ArgAction, Args, Parser, Subcommand};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use rusqlite::{Connection, OptionalExtension, params};
use signal_hook::consts::signal::SIGWINCH;
use signal_hook::iterator::Signals;

#[derive(Parser, Debug)]
#[command(name = "reout")]
#[command(version, about = "Reuse terminal command output after execution")]
struct Cli {
    /// Copy the selected output to the clipboard.
    #[arg(short, long, global = true, action = ArgAction::SetTrue)]
    copy: bool,

    /// Strip ANSI escape sequences before printing or copying.
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    plain: bool,

    /// Select output by offset. 1 is latest, 2 is one command before latest.
    #[arg(long, global = true)]
    offset: Option<usize>,

    /// Short form for offsets such as -2 or -3.
    #[arg(allow_hyphen_values = true, value_parser = parse_offset_shorthand)]
    shorthand_offset: Option<usize>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Print shell integration code.
    Init { shell: Shell },
    /// Internal command used by shell integration.
    Capture(CaptureArgs),
    /// List captured commands.
    List {
        /// Maximum number of rows to show.
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    /// Show output by id.
    Show { id: i64 },
    /// Copy output by id.
    Copy { id: i64 },
    /// Find commands by substring.
    Find {
        query: String,
        /// Maximum number of rows to show.
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    /// Delete one captured command.
    Delete { id: i64 },
    /// Delete all captured commands.
    Clear,
    /// Print database path.
    DbPath,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum Shell {
    Zsh,
}

#[derive(Args, Debug)]
struct CaptureArgs {
    /// Command string to execute under the user's shell.
    #[arg(last = true, required = true)]
    command: Vec<String>,
}

#[derive(Debug)]
#[allow(dead_code)]
struct Entry {
    id: i64,
    command: String,
    cwd: String,
    output: Vec<u8>,
    exit_code: i32,
    started_at: i64,
    finished_at: i64,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("reout: {err:#}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<u8> {
    let cli = Cli::parse();
    let db = Db::open()?;

    match cli.command {
        Some(Commands::Init { shell }) => {
            print_init(shell);
            Ok(0)
        }
        Some(Commands::Capture(args)) => {
            let command = args.command.join(" ");
            capture_command(&db, &command)
        }
        Some(Commands::List { limit }) => {
            list_entries(&db, limit)?;
            Ok(0)
        }
        Some(Commands::Show { id }) => {
            let entry = db
                .get_by_id(id)?
                .with_context(|| format!("no captured output with id {id}"))?;
            emit_output(&entry.output, cli.plain)?;
            Ok(0)
        }
        Some(Commands::Copy { id }) => {
            let entry = db
                .get_by_id(id)?
                .with_context(|| format!("no captured output with id {id}"))?;
            copy_output(&entry.output, cli.plain)?;
            println!("Copied output from `{}`", entry.command);
            Ok(0)
        }
        Some(Commands::Find { query, limit }) => {
            find_entries(&db, &query, limit)?;
            Ok(0)
        }
        Some(Commands::Delete { id }) => {
            db.delete(id)?;
            Ok(0)
        }
        Some(Commands::Clear) => {
            db.clear()?;
            Ok(0)
        }
        Some(Commands::DbPath) => {
            println!("{}", db.path.display());
            Ok(0)
        }
        None => {
            let offset = cli.offset.or(cli.shorthand_offset).unwrap_or(1);
            let entry = db
                .get_by_offset(offset)?
                .with_context(|| format!("no captured output at offset {offset}"))?;
            if cli.copy {
                copy_output(&entry.output, cli.plain)?;
                println!("Copied output from `{}`", entry.command);
            } else {
                emit_output(&entry.output, cli.plain)?;
            }
            Ok(0)
        }
    }
}

fn parse_offset_shorthand(value: &str) -> std::result::Result<usize, String> {
    let Some(rest) = value.strip_prefix('-') else {
        return Err("offset shorthand must look like -2".to_string());
    };
    let parsed = rest
        .parse::<usize>()
        .map_err(|_| "offset must be a positive integer".to_string())?;
    if parsed == 0 {
        return Err("offset must be greater than zero".to_string());
    }
    Ok(parsed)
}

fn print_init(shell: Shell) {
    match shell {
        Shell::Zsh => {
            print!(
                r#"# reout zsh integration
# Install with: eval "$(reout init zsh)"
#
# This ZLE widget runs the accepted command through `reout capture`.
# It is local-only and skips commands beginning with `reout`.

function _reout_accept_line() {{
  emulate -L zsh
  local cmd="$BUFFER"

  if [[ -z "${{cmd//[[:space:]]/}}" || "$cmd" == reout(|[[:space:]]*) ]]; then
    zle .accept-line
    return
  fi

  print -s -- "$cmd"
  BUFFER="reout capture -- ${{(q)cmd}}"
  zle .accept-line
}}

zle -N accept-line _reout_accept_line
"#
            );
        }
    }
}

fn capture_command(db: &Db, command: &str) -> Result<u8> {
    if should_ignore_command(command) {
        return Ok(0);
    }

    let cwd = std::env::current_dir()?.display().to_string();
    let started_at = unix_now();
    let (output, exit_code) = run_in_pty(command)?;
    let finished_at = unix_now();

    db.insert(command, &cwd, &output, exit_code, started_at, finished_at)?;
    Ok(exit_code_to_u8(exit_code))
}

fn run_in_pty(command: &str) -> Result<(Vec<u8>, i32)> {
    if !io::stdin().is_terminal() {
        return run_without_pty(command);
    }

    let pty_system = native_pty_system();
    let pair = pty_system.openpty(current_pty_size()).context("open pty")?;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let mut cmd = CommandBuilder::new(shell);
    cmd.arg("-lc");
    cmd.arg(command);
    if let Ok(term) = std::env::var("TERM") {
        cmd.env("TERM", term);
    }

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .context("spawn command in pty")?;
    drop(pair.slave);

    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let mut reader = pair.master.try_clone_reader().context("clone pty reader")?;
    let output_for_reader = Arc::clone(&output);
    let reader_thread = thread::spawn(move || -> io::Result<()> {
        let mut stdout = io::stdout();
        let mut buf = [0_u8; 8192];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            stdout.write_all(&buf[..n])?;
            stdout.flush()?;
            output_for_reader
                .lock()
                .expect("output lock")
                .extend_from_slice(&buf[..n]);
        }
        Ok(())
    });

    let interactive_stdin = io::stdin().is_terminal();
    let stdin_thread = if interactive_stdin {
        let mut writer = pair.master.take_writer().context("take pty writer")?;
        Some(thread::spawn(move || -> io::Result<()> {
            let mut stdin = io::stdin();
            let mut buf = [0_u8; 8192];
            loop {
                let n = stdin.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                writer.write_all(&buf[..n])?;
                writer.flush()?;
            }
            Ok(())
        }))
    } else {
        drop(pair.master.take_writer().context("take pty writer")?);
        None
    };

    let master_for_resize = pair.master;
    let resize_thread = thread::spawn(move || {
        if let Ok(mut signals) = Signals::new([SIGWINCH]) {
            for _ in signals.forever() {
                let _ = master_for_resize.resize(current_pty_size());
            }
        }
    });

    let raw_mode = if interactive_stdin {
        RawModeGuard::enable().ok()
    } else {
        None
    };
    let status = child.wait().context("wait for command")?;
    drop(raw_mode);

    reader_thread
        .join()
        .map_err(|_| anyhow::anyhow!("reader thread panicked"))?
        .context("read pty output")?;
    if let Some(stdin_thread) = &stdin_thread {
        let _ = stdin_thread.thread().id();
    }
    let _ = resize_thread.thread().id();

    let exit_code = status.exit_code() as i32;
    let output = Arc::try_unwrap(output)
        .map_err(|_| anyhow::anyhow!("output still shared"))?
        .into_inner()
        .map_err(|_| anyhow::anyhow!("output lock poisoned"))?;

    Ok((output, exit_code))
}

fn run_without_pty(command: &str) -> Result<(Vec<u8>, i32)> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let script = format!("({command}) 2>&1");
    let output = std::process::Command::new(shell)
        .arg("-lc")
        .arg(script)
        .output()
        .context("run command without pty")?;

    io::stdout().write_all(&output.stdout)?;
    io::stdout().flush()?;

    let exit_code = output.status.code().unwrap_or(1);
    Ok((output.stdout, exit_code))
}

fn current_pty_size() -> PtySize {
    if let Some((terminal_size::Width(cols), terminal_size::Height(rows))) =
        terminal_size::terminal_size()
    {
        PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    } else {
        PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> Result<Self> {
        enable_raw_mode().context("enable raw terminal mode")?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

fn should_ignore_command(command: &str) -> bool {
    let trimmed = command.trim_start();
    trimmed.is_empty() || trimmed == "reout" || trimmed.starts_with("reout ")
}

fn emit_output(output: &[u8], plain: bool) -> Result<()> {
    let bytes = maybe_plain(output, plain)?;
    io::stdout().write_all(&bytes)?;
    io::stdout().flush()?;
    Ok(())
}

fn copy_output(output: &[u8], plain: bool) -> Result<()> {
    let bytes = maybe_plain(output, plain)?;
    copy_bytes(&bytes)
}

fn maybe_plain(output: &[u8], plain: bool) -> Result<Vec<u8>> {
    if plain {
        Ok(strip_ansi_escapes::strip(output))
    } else {
        Ok(output.to_vec())
    }
}

fn copy_bytes(bytes: &[u8]) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::process::{Command, Stdio};

        let mut child = Command::new("pbcopy")
            .stdin(Stdio::piped())
            .spawn()
            .context("spawn pbcopy")?;
        child
            .stdin
            .as_mut()
            .context("open pbcopy stdin")?
            .write_all(bytes)?;
        let status = child.wait().context("wait for pbcopy")?;
        if !status.success() {
            bail!("pbcopy failed");
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = bytes;
        bail!("clipboard copy is only implemented for macOS in this MVP")
    }
}

fn list_entries(db: &Db, limit: usize) -> Result<()> {
    let entries = db.list(limit)?;
    println!("{:<5} {:<6} {:<9} COMMAND", "ID", "EXIT", "TIME");
    for entry in entries {
        println!(
            "{:<5} {:<6} {:<9} {}",
            entry.id,
            entry.exit_code,
            ago(entry.finished_at),
            one_line(&entry.command, 80)
        );
    }
    Ok(())
}

fn find_entries(db: &Db, query: &str, limit: usize) -> Result<()> {
    let entries = db.find(query, limit)?;
    println!("{:<5} {:<6} {:<9} COMMAND", "ID", "EXIT", "TIME");
    for entry in entries {
        println!(
            "{:<5} {:<6} {:<9} {}",
            entry.id,
            entry.exit_code,
            ago(entry.finished_at),
            one_line(&entry.command, 80)
        );
    }
    Ok(())
}

fn one_line(value: &str, max: usize) -> String {
    let value = value.replace('\n', " ");
    if value.chars().count() <= max {
        value
    } else {
        let mut s = value
            .chars()
            .take(max.saturating_sub(1))
            .collect::<String>();
        s.push('…');
        s
    }
}

fn ago(ts: i64) -> String {
    let now = unix_now();
    let diff = now.saturating_sub(ts);
    match diff {
        0..=59 => format!("{diff}s ago"),
        60..=3599 => format!("{}m ago", diff / 60),
        3600..=86_399 => format!("{}h ago", diff / 3600),
        _ => format!("{}d ago", diff / 86_400),
    }
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

struct Db {
    conn: Connection,
    path: PathBuf,
}

impl Db {
    fn open() -> Result<Self> {
        let path = db_path()?;
        if let Some(parent) = path.parent() {
            let existed = parent.exists();
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
            if !existed || parent.file_name().is_some_and(|name| name == "reout") {
                set_private_dir_permissions(parent)?;
            }
        }
        let conn = Connection::open(&path).with_context(|| format!("open {}", path.display()))?;
        let db = Self { conn, path };
        db.migrate()?;
        set_private_file_permissions(&db.path)?;
        Ok(db)
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

    fn insert(
        &self,
        command: &str,
        cwd: &str,
        output: &[u8],
        exit_code: i32,
        started_at: i64,
        finished_at: i64,
    ) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO commands (command, cwd, output, exit_code, started_at, finished_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![command, cwd, output, exit_code, started_at, finished_at],
        )?;
        Ok(())
    }

    fn get_by_offset(&self, offset: usize) -> Result<Option<Entry>> {
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

    fn get_by_id(&self, id: i64) -> Result<Option<Entry>> {
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

    fn list(&self, limit: usize) -> Result<Vec<Entry>> {
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

    fn find(&self, query: &str, limit: usize) -> Result<Vec<Entry>> {
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

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: row.get(0)?,
        command: row.get(1)?,
        cwd: row.get(2)?,
        output: row.get(3)?,
        exit_code: row.get(4)?,
        started_at: row.get(5)?,
        finished_at: row.get(6)?,
    })
}

fn db_path() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("REOUT_DB") {
        return Ok(PathBuf::from(path));
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".local/share")))
        .context("cannot determine data directory")?;
    Ok(base.join("reout/reout.db"))
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
