//! Machine-labelled results files (docs/results/*.json).
//!
//! Ground rule 3: every test and benchmark number is committed as JSON with
//! this schema. Use this writer rather than hand-rolling JSON so the schema
//! stays consistent — see docs/results/README.md.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ResultsFile {
    /// Computer name of the machine that produced the numbers.
    pub machine: String,
    /// ISO date (YYYY-MM-DD) the numbers were produced.
    pub date: String,
    /// Workspace crate version that produced them.
    pub app_version: String,
    /// One object of snake_case metrics, units in the key name (_ms, _fps, ...).
    pub metrics: serde_json::Value,
}

impl ResultsFile {
    pub fn new(date: &str, metrics: serde_json::Value) -> Self {
        ResultsFile {
            machine: machine_name(),
            date: date.to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            metrics,
        }
    }

    /// Write pretty-printed JSON, creating parent directories as needed.
    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json + "\n")?;
        Ok(())
    }
}

/// The local machine's name, for labelling results files.
/// Windows sets COMPUTERNAME; most Unixes set HOSTNAME (CI sets neither
/// reliably, hence the fallback).
pub fn machine_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown-machine".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_valid_schema() {
        let rf = ResultsFile::new("2026-08-19", serde_json::json!({ "answer_count": 42 }));
        let dir = std::env::temp_dir().join("worldmaker-io-test");
        let path = dir.join("sample.json");
        rf.write(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let back: ResultsFile = serde_json::from_str(&text).unwrap();
        assert_eq!(back.date, "2026-08-19");
        assert_eq!(back.metrics["answer_count"], 42);
        assert!(!back.machine.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
