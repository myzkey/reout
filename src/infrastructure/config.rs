use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::application::{parse_duration_seconds, parse_size_bytes};
use crate::domain::{CapturePolicy, RetentionPolicy};

#[derive(Debug, Clone, Default)]
pub struct ReoutConfig {
    pub ignore_commands: Vec<String>,
    pub retention: RetentionPolicy,
    pub capture: CapturePolicy,
}

#[derive(Debug, Deserialize, Default)]
struct ConfigFile {
    ignore: Option<IgnoreSection>,
    retention: Option<RetentionSection>,
    capture: Option<CaptureSection>,
}

#[derive(Debug, Deserialize, Default)]
struct IgnoreSection {
    commands: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
struct RetentionSection {
    max_age: Option<String>,
    max_entries: Option<usize>,
    max_bytes: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct CaptureSection {
    max_output_bytes: Option<String>,
}

impl ReoutConfig {
    pub fn load() -> Result<Self> {
        let Some(path) = config_path()? else {
            return Ok(Self::default());
        };
        if !path.exists() {
            return Ok(Self::default());
        }

        let content =
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        Self::from_toml_str(&content).with_context(|| format!("parse {}", path.display()))
    }

    pub(crate) fn from_toml_str(content: &str) -> Result<Self> {
        let parsed = toml::from_str::<ConfigFile>(content)?;
        let retention = parsed.retention.unwrap_or_default();
        let max_age_seconds = retention
            .max_age
            .map(|value| parse_duration_seconds(&value))
            .transpose()?;
        let max_bytes = retention
            .max_bytes
            .map(|value| parse_size_bytes(&value))
            .transpose()?;

        let capture = parsed.capture.unwrap_or_default();
        let max_output_bytes = capture
            .max_output_bytes
            .map(|value| parse_size_bytes(&value))
            .transpose()?;

        Ok(Self {
            ignore_commands: parsed
                .ignore
                .and_then(|ignore| ignore.commands)
                .unwrap_or_default(),
            retention: RetentionPolicy {
                max_age_seconds,
                max_entries: retention.max_entries,
                max_bytes,
            },
            capture: CapturePolicy { max_output_bytes },
        })
    }
}

pub fn config_path() -> Result<Option<PathBuf>> {
    if let Ok(path) = std::env::var("REOUT_CONFIG") {
        return Ok(Some(PathBuf::from(path)));
    }

    let Some(base) = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
    else {
        return Ok(None);
    };

    Ok(Some(base.join("reout/config.toml")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_config() {
        let config = ReoutConfig::from_toml_str("").expect("config");

        assert!(config.ignore_commands.is_empty());
        assert_eq!(config.retention.max_age_seconds, None);
        assert_eq!(config.retention.max_entries, None);
        assert_eq!(config.retention.max_bytes, None);
        assert_eq!(config.capture.max_output_bytes, None);
    }

    #[test]
    fn parses_ignore_retention_and_capture() {
        let config = ReoutConfig::from_toml_str(
            r#"
            [ignore]
            commands = ["reout*", "cat .env*"]

            [retention]
            max_age = "30d"
            max_entries = 100
            max_bytes = "10mb"

            [capture]
            max_output_bytes = "1mb"
            "#,
        )
        .expect("config");

        assert_eq!(config.ignore_commands, vec!["reout*", "cat .env*"]);
        assert_eq!(config.retention.max_age_seconds, Some(2_592_000));
        assert_eq!(config.retention.max_entries, Some(100));
        assert_eq!(config.retention.max_bytes, Some(10 * 1024 * 1024));
        assert_eq!(config.capture.max_output_bytes, Some(1024 * 1024));
    }

    #[test]
    fn rejects_invalid_retention_duration() {
        let result = ReoutConfig::from_toml_str(
            r#"
            [retention]
            max_age = "forever"
            "#,
        );

        assert!(result.is_err());
    }
}
