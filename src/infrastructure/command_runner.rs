use std::io::{self, IsTerminal, Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use portable_pty::{CommandBuilder, ExitStatus, PtySize, native_pty_system};
use signal_hook::consts::signal::SIGWINCH;
use signal_hook::iterator::Signals;

use crate::application::CommandRunner;
use crate::domain::CommandOutput;

#[derive(Debug, Default, Clone, Copy)]
pub struct PtyCommandRunner;

impl CommandRunner for PtyCommandRunner {
    fn run(&self, command: &str) -> Result<(CommandOutput, i32)> {
        run_command(command)
    }
}

fn run_command(command: &str) -> Result<(CommandOutput, i32)> {
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

    let mut writer = pair.master.take_writer().context("take pty writer")?;
    let stdin_thread = thread::spawn(move || -> io::Result<()> {
        let mut stdin = io::stdin();
        let mut buf = [0_u8; 8192];
        loop {
            let n = stdin.read(&mut buf)?;
            if n == 0 {
                break;
            }
            for byte in &mut buf[..n] {
                if *byte == 0x1a {
                    *byte = 0x03;
                }
            }
            writer.write_all(&buf[..n])?;
            writer.flush()?;
        }
        Ok(())
    });

    let master_for_resize = pair.master;
    let resize_thread = thread::spawn(move || {
        if let Ok(mut signals) = Signals::new([SIGWINCH]) {
            for _ in signals.forever() {
                let _ = master_for_resize.resize(current_pty_size());
            }
        }
    });

    let raw_mode = RawModeGuard::enable().ok();
    let status = child.wait().context("wait for command")?;
    drop(raw_mode);

    reader_thread
        .join()
        .map_err(|_| anyhow::anyhow!("reader thread panicked"))?
        .context("read pty output")?;
    let _ = stdin_thread.thread().id();
    let _ = resize_thread.thread().id();

    let exit_code = exit_code_from_status(&status);
    let output = Arc::try_unwrap(output)
        .map_err(|_| anyhow::anyhow!("output still shared"))?
        .into_inner()
        .map_err(|_| anyhow::anyhow!("output lock poisoned"))?;

    Ok((CommandOutput::new(output), exit_code))
}

fn exit_code_from_status(status: &ExitStatus) -> i32 {
    if let Some(signal) = status.signal() {
        let signal = signal.to_ascii_lowercase();
        if signal.contains("interrupt") || signal.contains("sigint") {
            return 130;
        }
        if signal.contains("quit") || signal.contains("sigquit") {
            return 131;
        }
        if signal.contains("terminated") || signal.contains("sigterm") {
            return 143;
        }
        if signal.contains("hangup") || signal.contains("sighup") {
            return 129;
        }
    }

    status.exit_code() as i32
}

fn run_without_pty(command: &str) -> Result<(CommandOutput, i32)> {
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
    Ok((CommandOutput::new(output.stdout), exit_code))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_signal_statuses_to_shell_exit_codes() {
        assert_eq!(
            exit_code_from_status(&ExitStatus::with_signal("Interrupt: 2")),
            130
        );
        assert_eq!(
            exit_code_from_status(&ExitStatus::with_signal("Quit: 3")),
            131
        );
        assert_eq!(
            exit_code_from_status(&ExitStatus::with_signal("Terminated: 15")),
            143
        );
        assert_eq!(
            exit_code_from_status(&ExitStatus::with_signal("Hangup: 1")),
            129
        );
    }

    #[test]
    fn keeps_normal_exit_code() {
        assert_eq!(exit_code_from_status(&ExitStatus::with_exit_code(7)), 7);
    }
}
