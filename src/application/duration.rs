use anyhow::{Result, bail};

pub fn parse_duration_seconds(value: &str) -> Result<i64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("duration must not be empty");
    }

    let (number, multiplier) = match trimmed.as_bytes().last().copied() {
        Some(b's') => (&trimmed[..trimmed.len() - 1], 1),
        Some(b'm') => (&trimmed[..trimmed.len() - 1], 60),
        Some(b'h') => (&trimmed[..trimmed.len() - 1], 60 * 60),
        Some(b'd') => (&trimmed[..trimmed.len() - 1], 60 * 60 * 24),
        Some(b'w') => (&trimmed[..trimmed.len() - 1], 60 * 60 * 24 * 7),
        Some(b'0'..=b'9') => (trimmed, 1),
        _ => bail!("duration must look like 30s, 10m, 2h, 7d, or 1w"),
    };

    let number = number
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("duration must start with a positive integer"))?;
    if number <= 0 {
        bail!("duration must be greater than zero");
    }

    Ok(number * multiplier)
}

pub fn parse_size_bytes(value: &str) -> Result<usize> {
    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        bail!("size must not be empty");
    }

    let units = [
        ("kib", 1024usize),
        ("kb", 1024usize),
        ("mib", 1024usize * 1024),
        ("mb", 1024usize * 1024),
        ("gib", 1024usize * 1024 * 1024),
        ("gb", 1024usize * 1024 * 1024),
        ("b", 1usize),
    ];

    let (number, multiplier) = units
        .iter()
        .find_map(|(suffix, multiplier)| {
            trimmed
                .strip_suffix(suffix)
                .map(|number| (number.trim(), *multiplier))
        })
        .unwrap_or((trimmed.as_str(), 1));

    let number = number
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("size must start with a positive integer"))?;
    if number == 0 {
        bail!("size must be greater than zero");
    }

    Ok(number * multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_duration_units() {
        assert_eq!(parse_duration_seconds("30s").unwrap(), 30);
        assert_eq!(parse_duration_seconds("10m").unwrap(), 600);
        assert_eq!(parse_duration_seconds("2h").unwrap(), 7200);
        assert_eq!(parse_duration_seconds("7d").unwrap(), 604800);
        assert_eq!(parse_duration_seconds("1w").unwrap(), 604800);
        assert_eq!(parse_duration_seconds("42").unwrap(), 42);
    }

    #[test]
    fn rejects_invalid_durations() {
        assert!(parse_duration_seconds("").is_err());
        assert!(parse_duration_seconds("0s").is_err());
        assert!(parse_duration_seconds("-1d").is_err());
        assert!(parse_duration_seconds("1mo").is_err());
        assert!(parse_duration_seconds("abc").is_err());
    }

    #[test]
    fn parses_size_units() {
        assert_eq!(parse_size_bytes("42").unwrap(), 42);
        assert_eq!(parse_size_bytes("10b").unwrap(), 10);
        assert_eq!(parse_size_bytes("1kb").unwrap(), 1024);
        assert_eq!(parse_size_bytes("2mb").unwrap(), 2 * 1024 * 1024);
        assert_eq!(parse_size_bytes("1gb").unwrap(), 1024 * 1024 * 1024);
    }

    #[test]
    fn rejects_invalid_sizes() {
        assert!(parse_size_bytes("").is_err());
        assert!(parse_size_bytes("0").is_err());
        assert!(parse_size_bytes("abc").is_err());
        assert!(parse_size_bytes("1tb").is_err());
    }
}
