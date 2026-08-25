use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::domain::CommandEntry;

pub fn print_entries(entries: &[CommandEntry]) {
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
}

pub fn print_find_results(entries: &[CommandEntry]) {
    print_entries(entries);
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
