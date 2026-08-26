use std::io::{self, Read, Write};

use anyhow::Result;
use clap::{ArgAction, Args, Parser, Subcommand};

use crate::application::ReoutUseCases;
use crate::application::parse_duration_seconds;
use crate::domain::HistoryQuery;
use crate::infrastructure::ansi::AnsiOutputSanitizer;
use crate::infrastructure::clipboard::SystemClipboard;
use crate::infrastructure::command_runner::PtyCommandRunner;
use crate::infrastructure::config::{ReoutConfig, config_path};
use crate::infrastructure::sqlite::SqliteCommandRepository;
use crate::presentation::format::{OutputFormat, filter_error_lines, format_entries, format_entry};
use crate::presentation::shell_init::{bash_init_script, fish_init_script, zsh_init_script};
use crate::presentation::table::print_entries;

type AppUseCases =
    ReoutUseCases<SqliteCommandRepository, PtyCommandRunner, SystemClipboard, AnsiOutputSanitizer>;

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

    /// Select only failed commands.
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    failed: bool,

    /// Select commands that ran in this cwd. Use "." for the current directory.
    #[arg(long, global = true)]
    cwd: Option<String>,

    /// Select commands finished since a duration such as 10m, 2h, 1d, or 1w.
    #[arg(long, global = true)]
    since: Option<String>,

    /// Select the latest command whose command string contains this value.
    #[arg(long, global = true)]
    last: Option<String>,

    /// Emit JSON instead of the default text output.
    #[arg(long, global = true, conflicts_with = "md", action = ArgAction::SetTrue)]
    json: bool,

    /// Emit Markdown instead of the default text output.
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    md: bool,

    /// Show only lines containing "error" from selected output.
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    errors: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Print shell integration code.
    Init { shell: Shell },
    /// Internal command used by shell integration.
    Capture(CaptureArgs),
    /// Internal command used by shell integration for current-shell commands.
    Import(ImportArgs),
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
    /// Delete records older than retention or a supplied age.
    Prune {
        /// Prune records older than this duration, for example 30d.
        #[arg(long)]
        older_than: Option<String>,
    },
    /// Print config path.
    ConfigPath,
    /// Print database path.
    DbPath,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum Shell {
    Zsh,
    Bash,
    Fish,
}

#[derive(Args, Debug)]
struct CaptureArgs {
    /// Command string to execute under the user's shell.
    #[arg(last = true, required = true)]
    command: Vec<String>,
}

#[derive(Args, Debug)]
struct ImportArgs {
    /// Original command string.
    #[arg(long)]
    command: String,

    /// Working directory where the command ran.
    #[arg(long)]
    cwd: String,

    /// Command exit code.
    #[arg(long)]
    exit_code: i32,
}

pub fn run() -> Result<u8> {
    let cli = Cli::parse();
    let repository = SqliteCommandRepository::open_default()?;
    let db_path = repository.path().to_path_buf();
    let config = ReoutConfig::load()?;
    let app = ReoutUseCases::new(
        repository,
        PtyCommandRunner,
        SystemClipboard,
        AnsiOutputSanitizer,
    )
    .with_ignored_commands(config.ignore_commands.clone())
    .with_retention_policy(config.retention.clone())
    .with_capture_policy(config.capture.clone());

    dispatch(cli, app, db_path, config)
}

fn dispatch(
    cli: Cli,
    app: AppUseCases,
    db_path: std::path::PathBuf,
    config: ReoutConfig,
) -> Result<u8> {
    let output_format = output_format(&cli);
    match cli.command {
        Some(Commands::Init { shell }) => {
            match shell {
                Shell::Zsh => print!("{}", zsh_init_script()),
                Shell::Bash => print!("{}", bash_init_script()),
                Shell::Fish => print!("{}", fish_init_script()),
            }
            Ok(0)
        }
        Some(Commands::Capture(args)) => {
            let command = args.command.join(" ");
            let cwd = std::env::current_dir()?.display().to_string();
            app.capture(&command, cwd)
        }
        Some(Commands::Import(args)) => {
            let mut output = Vec::new();
            io::stdin().read_to_end(&mut output)?;
            app.import_output(&args.command, args.cwd, output, args.exit_code)
        }
        Some(Commands::List { limit }) => {
            let entries = app.query(&history_query(&cli, limit)?)?;
            emit_entries(&entries, output_format)?;
            Ok(0)
        }
        Some(Commands::Show { id }) => {
            let entry = app.get_by_id(id)?;
            emit_output(&selected_output(&app, &entry, &cli, output_format)?)?;
            Ok(0)
        }
        Some(Commands::Copy { id }) => {
            let entry = app.get_by_id(id)?;
            copy_selected_output(&app, &entry, &cli, output_format)?;
            println!("Copied output from `{}`", entry.command);
            Ok(0)
        }
        Some(Commands::Find { ref query, limit }) => {
            let mut history_query = history_query(&cli, limit)?;
            history_query.command_contains = Some(query.clone());
            let entries = app.query(&history_query)?;
            emit_entries(&entries, output_format)?;
            Ok(0)
        }
        Some(Commands::Delete { id }) => {
            app.delete(id)?;
            Ok(0)
        }
        Some(Commands::Clear) => {
            app.clear()?;
            Ok(0)
        }
        Some(Commands::Prune { older_than }) => {
            let seconds = match older_than {
                Some(value) => parse_duration_seconds(&value)?,
                None => config.retention.max_age_seconds.ok_or_else(|| {
                    anyhow::anyhow!("set [retention].max_age or pass --older-than")
                })?,
            };
            let threshold = unix_now() - seconds;
            let deleted = app.prune_before(threshold)?;
            println!("Deleted {deleted} records");
            Ok(0)
        }
        Some(Commands::ConfigPath) => {
            if let Some(path) = config_path()? {
                println!("{}", path.display());
            }
            Ok(0)
        }
        Some(Commands::DbPath) => {
            println!("{}", db_path.display());
            Ok(0)
        }
        None => {
            let entry =
                if cli.last.is_some() || cli.failed || cli.cwd.is_some() || cli.since.is_some() {
                    app.latest_matching(&history_query(&cli, 1)?)?
                } else {
                    let offset = cli.offset.or(cli.shorthand_offset).unwrap_or(1);
                    app.latest_by_offset(offset)?
                };
            if cli.copy {
                copy_selected_output(&app, &entry, &cli, output_format)?;
                println!("Copied output from `{}`", entry.command);
            } else {
                emit_output(&selected_output(&app, &entry, &cli, output_format)?)?;
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

fn emit_output(bytes: &[u8]) -> Result<()> {
    io::stdout().write_all(bytes)?;
    io::stdout().flush()?;
    Ok(())
}

fn emit_entries(entries: &[crate::domain::CommandEntry], format: OutputFormat) -> Result<()> {
    if let Some(bytes) = format_entries(entries, format)? {
        emit_output(&bytes)?;
        if !bytes.ends_with(b"\n") {
            println!();
        }
    } else {
        print_entries(entries);
    }
    Ok(())
}

fn selected_output(
    app: &AppUseCases,
    entry: &crate::domain::CommandEntry,
    cli: &Cli,
    format: OutputFormat,
) -> Result<Vec<u8>> {
    let mut bytes = app.render_output(
        entry,
        cli.plain || cli.errors || format != OutputFormat::Raw,
    )?;
    if cli.errors {
        bytes = filter_error_lines(&bytes);
    }
    format_entry(entry, &bytes, format)
}

fn copy_selected_output(
    app: &AppUseCases,
    entry: &crate::domain::CommandEntry,
    cli: &Cli,
    format: OutputFormat,
) -> Result<()> {
    let bytes = selected_output(app, entry, cli, format)?;
    app.copy_bytes(&bytes)
}

fn output_format(cli: &Cli) -> OutputFormat {
    if cli.json {
        OutputFormat::Json
    } else if cli.md {
        OutputFormat::Markdown
    } else {
        OutputFormat::Raw
    }
}

fn history_query(cli: &Cli, limit: usize) -> Result<HistoryQuery> {
    let cwd = match &cli.cwd {
        Some(value) if value == "." => Some(std::env::current_dir()?.display().to_string()),
        Some(value) => Some(value.clone()),
        None => None,
    };
    let since = cli
        .since
        .as_deref()
        .map(parse_duration_seconds)
        .transpose()?
        .map(|seconds| unix_now() - seconds);

    Ok(HistoryQuery {
        command_contains: cli.last.clone(),
        cwd,
        failed_only: cli.failed,
        since,
        limit,
    })
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_secs(0))
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_offset_shorthand() {
        assert_eq!(parse_offset_shorthand("-2").unwrap(), 2);
        assert_eq!(parse_offset_shorthand("-42").unwrap(), 42);
    }

    #[test]
    fn rejects_invalid_offset_shorthand() {
        assert!(parse_offset_shorthand("2").is_err());
        assert!(parse_offset_shorthand("-0").is_err());
        assert!(parse_offset_shorthand("-abc").is_err());
    }

    #[test]
    fn resolves_output_format() {
        let raw = Cli::parse_from(["reout"]);
        assert_eq!(output_format(&raw), OutputFormat::Raw);

        let json = Cli::parse_from(["reout", "--json"]);
        assert_eq!(output_format(&json), OutputFormat::Json);

        let md = Cli::parse_from(["reout", "--md"]);
        assert_eq!(output_format(&md), OutputFormat::Markdown);
    }

    #[test]
    fn builds_history_query_from_global_filters() {
        let cli = Cli::parse_from([
            "reout", "--failed", "--cwd", "/repo", "--since", "1h", "--last", "cargo", "list",
        ]);

        let query = history_query(&cli, 50).expect("query");

        assert_eq!(query.command_contains, Some("cargo".to_string()));
        assert_eq!(query.cwd, Some("/repo".to_string()));
        assert!(query.failed_only);
        assert_eq!(query.limit, 50);
        assert!(query.since.is_some());
    }
}
