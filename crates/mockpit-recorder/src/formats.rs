//! Recording format handling

use anyhow::Result;

/// Recording format for saving sessions
#[derive(Debug, Clone, Copy)]
pub enum RecordingFormat {
    Json,
    Yaml,
    Har,
}

impl RecordingFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            RecordingFormat::Json => "json",
            RecordingFormat::Yaml => "yaml",
            RecordingFormat::Har => "har",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "json" => Ok(RecordingFormat::Json),
            "yaml" | "yml" => Ok(RecordingFormat::Yaml),
            "har" => Ok(RecordingFormat::Har),
            _ => Err(anyhow::anyhow!("Invalid recording format: {}", s)),
        }
    }
}
