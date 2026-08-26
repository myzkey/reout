# reout

[日本語版](README.ja.md)

Command output should be reusable after execution.

`reout` is a local-first CLI that captures terminal command output so it can be
shown, copied, listed, and searched after the command has already finished.

The core is intentionally command-agnostic. Terraform, Git, Docker, Kubernetes,
test runners, compilers, package managers, and custom CLIs are all treated as:

- command
- cwd
- terminal output
- exit code
- started_at
- finished_at

## MVP

Current target:

- macOS
- zsh
- SQLite at `$XDG_DATA_HOME/reout/reout.db` or `~/.local/share/reout/reout.db`
- local-only storage
- PTY-backed execution when stdin is a terminal

Install locally:

```bash
cargo install --path .
```

Enable zsh integration:

```bash
eval "$(reout init zsh)"
```

After that, ordinary commands accepted from zsh are transparently routed through
`reout`. `reout` commands themselves are not captured.

The zsh integration uses two capture paths:

- External commands run through `reout capture -- <command>`, which uses a PTY.
- Current-shell aliases and functions run in the current zsh process and are
  saved through `reout import`.

Temporarily disable integration:

```bash
REOUT_DISABLED=1
```

## Usage

Show the latest output:

```bash
reout
```

Copy the latest output:

```bash
reout -c
reout --copy
```

Strip ANSI escapes:

```bash
reout --plain
reout --plain -c
```

Show older outputs:

```bash
reout -2
reout --offset 2
```

List history:

```bash
reout list
```

Show or copy by id:

```bash
reout show 42
reout copy 42
```

Find by command substring:

```bash
reout find test
reout find kubectl
reout find "terraform plan"
```

Filter history:

```bash
reout --failed
reout --cwd .
reout --since 1d
reout --last cargo
reout list --failed --since 1w
```

Structured output:

```bash
reout --json
reout --md
reout list --json
reout show 42 --md
```

Extract simple error lines:

```bash
reout --errors
reout --errors -c
```

Delete:

```bash
reout delete 42
reout clear
reout prune --older-than 30d
```

Retention is also applied automatically after each captured or imported command
when configured.

Pipe like a normal Unix command:

```bash
reout --plain | grep ERROR
reout show 42 | rg timeout
```

## Design Notes

### Architecture

The implementation follows a clean architecture style:

```text
src/
  domain/          Core entities such as captured command output
  application/     Use cases and ports, independent from SQLite, PTY, and clap
  infrastructure/  SQLite, PTY command runner, clipboard, paths, ANSI stripping
  presentation/    clap CLI, shell init scripts, terminal table formatting
  main.rs          Thin process entrypoint
```

Dependency direction:

```text
presentation -> application -> domain
infrastructure -> application -> domain
```

The application layer depends on traits such as `CommandRepository`,
`CommandRunner`, `Clipboard`, and `OutputSanitizer`. SQLite, PTY, and `pbcopy`
are adapters behind those ports.

### 1. Shell Integration

zsh exposes ZLE widgets that can replace the accepted line before the shell
executes it. The MVP installs a custom `accept-line` widget and replaces:

```bash
any-command
```

with:

```bash
reout capture -- any-command
```

This avoids requiring the user to type `reout run ...` manually.

The integration skips shell-state commands such as `cd`, `export`, `alias`,
`source`, `jobs`, `fg`, and background jobs. Those commands must run in the
current shell, not through the capture proxy. It also skips full-screen or
session-oriented interactive commands such as `vim`, `less`, `top`, `ssh`,
`fzf`, `tmux`, and `screen` to avoid breaking normal terminal use.

### 2. PTY proxy

When stdin is a terminal, `reout capture` opens a pseudo-terminal and runs the
command through `$SHELL -lc <command>`. It forwards PTY output to the real
terminal while recording the same byte stream into SQLite.

This preserves the behavior of TTY-aware programs better than stdout
redirection. Terminal resize is forwarded with `SIGWINCH`, and stdin is put in
raw mode. Ctrl+C is forwarded and recorded as exit code `130`. Ctrl+Z is mapped
to Ctrl+C inside `reout capture` so captured commands do not leave the proxy
hung in a stopped state.

### 3. stdout and stderr order

The MVP stores a single `output` BLOB captured from the PTY master. That gives a
better approximation of what the user saw than separate stdout/stderr pipes.

Non-TTY execution falls back to a plain process run with stdout/stderr combined;
this keeps scripted tests clean but is not the main interactive path.

### 4. interactive commands

The PTY path is designed for commands such as:

```bash
vim
less
top
ssh
fzf
git add -p
docker compose logs -f
kubectl logs -f
```

Long-running interactive sessions can produce very large records. Use ignore
rules and retention pruning for commands whose output should not be kept.

### 5. SQLite schema

```sql
CREATE TABLE commands (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  command TEXT NOT NULL,
  cwd TEXT NOT NULL,
  output BLOB NOT NULL,
  exit_code INTEGER NOT NULL,
  started_at INTEGER NOT NULL,
  finished_at INTEGER NOT NULL
);
```

### 6. Atuin and related tools

Atuin's shell integration records command metadata using shell lifecycle hooks.
`reout` follows the same broad idea of shell integration, but it captures the
terminal output stream as the primary artifact. The MVP uses ZLE command-line
rewriting because preexec/precmd hooks alone can observe command metadata but
cannot capture output after the fact.

### 7. Security

`reout` never sends captured output anywhere. The database directory is created
with private permissions where `reout` owns the directory, and the database file
is chmodded to `0600` on Unix platforms.

Config shape:

```toml
[ignore]
commands = [
  "reout*",
  "cat .env*",
  "aws configure*"
]

[retention]
max_age = "30d"
max_entries = 1000
max_bytes = "1gb"

[capture]
max_output_bytes = "10mb"
```

## Known Limitations

- Current-shell aliases and functions are supported through a non-PTY import
  path. That path preserves alias/function resolution, but commands on that path
  do not see stdout as a TTY.
- `stdout` and `stderr` are not stored separately in the MVP.
- Clipboard copy currently uses macOS `pbcopy`.
- zsh is the only transparent shell integration. `reout init bash` and
  `reout init fish` intentionally report that they are not implemented yet.
- `--errors` is a simple line filter over captured output. It is not stderr
  extraction.

## Verified Locally

- `cargo check`
- `cargo test`
- `cargo clippy -- -D warnings`
- Non-TTY capture, latest output, offset output, list, find, and plain output
- Interactive zsh integration with `cd`, `echo`, `reout --plain`, and
  `reout list`
- Interactive zsh integration with current-shell aliases and functions
- PTY stdin forwarding with `cat`
- Ctrl+C forwarding with `sleep 10`
- Ctrl+Z hang avoidance with `sleep 10`
- Config ignore patterns with `REOUT_CONFIG`
- JSON, Markdown, failed, cwd, since, last, errors, and prune commands

## Config

Default config path:

```bash
reout config-path
```

Example:

```toml
[ignore]
commands = [
  "reout*",
  "cat .env*",
  "aws configure*"
]

[retention]
max_age = "30d"
```

Run retention manually:

```bash
reout prune
```

Configured retention is also enforced automatically after `reout capture` and
`reout import`.

`--last terraform` is a generic command substring search, not
Terraform-specific parsing.
