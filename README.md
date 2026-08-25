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

After that, ordinary commands accepted from zsh are transparently rewritten to
run through `reout capture -- <command>`. `reout` commands themselves are not
captured.

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

Delete:

```bash
reout delete 42
reout clear
```

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

### 2. PTY proxy

When stdin is a terminal, `reout capture` opens a pseudo-terminal and runs the
command through `$SHELL -lc <command>`. It forwards PTY output to the real
terminal while recording the same byte stream into SQLite.

This preserves the behavior of TTY-aware programs better than stdout
redirection. Terminal resize is forwarded with `SIGWINCH`, and stdin is put in
raw mode so Ctrl+C and Ctrl+Z travel through the PTY line discipline.

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

Long-running interactive sessions can produce very large records. Retention and
ignore rules are planned but not yet implemented.

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

Planned config shape:

```toml
[ignore]
commands = [
  "reout*",
  "cat .env*",
  "aws configure*"
]

[retention]
max_age = "30d"
max_bytes = "1GB"
```

## Known Limitations

- zsh aliases and functions from the current interactive shell may not always
  behave exactly like direct execution because captured commands are run via
  `$SHELL -lc`.
- `stdout` and `stderr` are not stored separately in the MVP.
- Clipboard copy currently uses macOS `pbcopy`.
- There is no retention policy yet, so large command outputs remain until
  deleted manually.

## Future Commands

The schema and command model leave room for:

```bash
reout --failed
reout --cwd .
reout --since 1d
reout --last cargo
reout --last terraform
reout --json
reout --md
reout --errors
reout prune
```

`--last terraform` should stay a generic command-prefix or substring search,
not Terraform-specific parsing.
