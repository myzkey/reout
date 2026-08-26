use anyhow::Result;
use serde::Serialize;

use crate::domain::CommandEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Raw,
    Json,
    Markdown,
}

#[derive(Serialize)]
struct EntryJson<'a> {
    id: i64,
    command: &'a str,
    cwd: &'a str,
    output: String,
    exit_code: i32,
    started_at: i64,
    finished_at: i64,
}

#[derive(Serialize)]
struct EntrySummaryJson<'a> {
    id: i64,
    command: &'a str,
    cwd: &'a str,
    exit_code: i32,
    started_at: i64,
    finished_at: i64,
}

pub fn format_entry(entry: &CommandEntry, output: &[u8], format: OutputFormat) -> Result<Vec<u8>> {
    match format {
        OutputFormat::Raw => Ok(output.to_vec()),
        OutputFormat::Json => {
            let value = EntryJson {
                id: entry.id,
                command: &entry.command,
                cwd: &entry.cwd,
                output: String::from_utf8_lossy(output).into_owned(),
                exit_code: entry.exit_code,
                started_at: entry.started_at,
                finished_at: entry.finished_at,
            };
            Ok(serde_json::to_vec_pretty(&value)?)
        }
        OutputFormat::Markdown => {
            let text = format!(
                "# reout {}\n\n- Command: `{}`\n- CWD: `{}`\n- Exit: `{}`\n\n```text\n{}\n```\n",
                entry.id,
                entry.command.replace('`', "\\`"),
                entry.cwd.replace('`', "\\`"),
                entry.exit_code,
                String::from_utf8_lossy(output)
            );
            Ok(text.into_bytes())
        }
    }
}

pub fn format_entries(entries: &[CommandEntry], format: OutputFormat) -> Result<Option<Vec<u8>>> {
    match format {
        OutputFormat::Raw => Ok(None),
        OutputFormat::Json => {
            let values = entries
                .iter()
                .map(|entry| EntrySummaryJson {
                    id: entry.id,
                    command: &entry.command,
                    cwd: &entry.cwd,
                    exit_code: entry.exit_code,
                    started_at: entry.started_at,
                    finished_at: entry.finished_at,
                })
                .collect::<Vec<_>>();
            Ok(Some(serde_json::to_vec_pretty(&values)?))
        }
        OutputFormat::Markdown => {
            let mut text = String::from("| ID | Exit | Command | CWD |\n|---:|---:|---|---|\n");
            for entry in entries {
                text.push_str(&format!(
                    "| {} | {} | `{}` | `{}` |\n",
                    entry.id,
                    entry.exit_code,
                    entry.command.replace('|', "\\|").replace('`', "\\`"),
                    entry.cwd.replace('|', "\\|").replace('`', "\\`")
                ));
            }
            Ok(Some(text.into_bytes()))
        }
    }
}

pub fn filter_error_lines(bytes: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    let mut output = text
        .lines()
        .filter(|line| line.to_ascii_lowercase().contains("error"))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes();
    if !output.is_empty() {
        output.push(b'\n');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CommandEntry, CommandOutput};

    fn entry() -> CommandEntry {
        CommandEntry {
            id: 42,
            command: "cargo test | rg `error`".to_string(),
            cwd: "/tmp/project".to_string(),
            output: CommandOutput::new(b"ignored".to_vec()),
            exit_code: 1,
            started_at: 100,
            finished_at: 200,
        }
    }

    #[test]
    fn formats_raw_entry_output() {
        let formatted = format_entry(&entry(), b"raw\n", OutputFormat::Raw).expect("format");

        assert_eq!(formatted, b"raw\n");
    }

    #[test]
    fn formats_entry_as_json_with_output() {
        let formatted = format_entry(&entry(), b"error\n", OutputFormat::Json).expect("format");
        let value: serde_json::Value = serde_json::from_slice(&formatted).expect("json");

        assert_eq!(value["id"], 42);
        assert_eq!(value["command"], "cargo test | rg `error`");
        assert_eq!(value["output"], "error\n");
        assert_eq!(value["exit_code"], 1);
    }

    #[test]
    fn formats_entry_as_markdown() {
        let formatted = format_entry(&entry(), b"error\n", OutputFormat::Markdown).expect("format");
        let text = String::from_utf8(formatted).expect("utf8");

        assert!(text.contains("# reout 42"));
        assert!(text.contains("Command: `cargo test | rg \\`error\\``"));
        assert!(text.contains("```text\nerror\n\n```"));
    }

    #[test]
    fn formats_entry_list_as_json() {
        let formatted = format_entries(&[entry()], OutputFormat::Json)
            .expect("format")
            .expect("bytes");
        let value: serde_json::Value = serde_json::from_slice(&formatted).expect("json");

        assert_eq!(value[0]["id"], 42);
        assert!(value[0].get("output").is_none());
    }

    #[test]
    fn formats_entry_list_as_markdown_table() {
        let formatted = format_entries(&[entry()], OutputFormat::Markdown)
            .expect("format")
            .expect("bytes");
        let text = String::from_utf8(formatted).expect("utf8");

        assert!(text.starts_with("| ID | Exit | Command | CWD |"));
        assert!(text.contains("| 42 | 1 | `cargo test \\| rg \\`error\\``"));
    }

    #[test]
    fn raw_entry_list_uses_table_renderer() {
        assert!(
            format_entries(&[entry()], OutputFormat::Raw)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn filters_error_lines_case_insensitively() {
        let filtered = filter_error_lines(b"ok\nERROR bad\nwarn\nfatal error\n");

        assert_eq!(filtered, b"ERROR bad\nfatal error\n");
    }

    #[test]
    fn filter_error_lines_keeps_empty_output_empty() {
        assert_eq!(filter_error_lines(b"ok\nwarn\n"), b"");
    }
}
