use std::io::{self, Write};

use anyhow::Result;
use clap::{ArgAction, Args, Parser, Subcommand};

use crate::application::ReoutUseCases;
use crate::infrastructure::ansi::AnsiOutputSanitizer;
use crate::infrastructure::clipboard::SystemClipboard;
use crate::infrastructure::command_runner::PtyCommandRunner;
use crate::infrastructure::sqlite::SqliteCommandRepository;
use crate::presentation::shell_init::zsh_init_script;
use crate::presentation::table::{print_entries, print_find_results};

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

pub fn run() -> Result<u8> {
    let cli = Cli::parse();
    let repository = SqliteCommandRepository::open_default()?;
    let db_path = repository.path().to_path_buf();
    let app = ReoutUseCases::new(
        repository,
        PtyCommandRunner,
        SystemClipboard,
        AnsiOutputSanitizer,
    );

    dispatch(cli, app, db_path)
}

fn dispatch(cli: Cli, app: AppUseCases, db_path: std::path::PathBuf) -> Result<u8> {
    match cli.command {
        Some(Commands::Init { shell }) => {
            match shell {
                Shell::Zsh => print!("{}", zsh_init_script()),
            }
            Ok(0)
        }
        Some(Commands::Capture(args)) => {
            let command = args.command.join(" ");
            let cwd = std::env::current_dir()?.display().to_string();
            app.capture(&command, cwd)
        }
        Some(Commands::List { limit }) => {
            print_entries(&app.list(limit)?);
            Ok(0)
        }
        Some(Commands::Show { id }) => {
            let entry = app.get_by_id(id)?;
            emit_output(&app.render_output(&entry, cli.plain)?)?;
            Ok(0)
        }
        Some(Commands::Copy { id }) => {
            let entry = app.get_by_id(id)?;
            app.copy_output(&entry, cli.plain)?;
            println!("Copied output from `{}`", entry.command);
            Ok(0)
        }
        Some(Commands::Find { query, limit }) => {
            print_find_results(&app.find(&query, limit)?);
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
        Some(Commands::DbPath) => {
            println!("{}", db_path.display());
            Ok(0)
        }
        None => {
            let offset = cli.offset.or(cli.shorthand_offset).unwrap_or(1);
            let entry = app.latest_by_offset(offset)?;
            if cli.copy {
                app.copy_output(&entry, cli.plain)?;
                println!("Copied output from `{}`", entry.command);
            } else {
                emit_output(&app.render_output(&entry, cli.plain)?)?;
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
