use std::process::ExitCode;

fn main() -> ExitCode {
    match reout::presentation::cli::run() {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("reout: {err:#}");
            ExitCode::from(1)
        }
    }
}
