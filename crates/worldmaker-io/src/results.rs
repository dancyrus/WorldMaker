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
/// Windows sets COMPUTERNAME; some Unix shells export HOSTNAME; macOS (and
/// non-interactive Unix shells) set neither, so ask the OS directly.
pub fn machine_name() -> String {
    if let Ok(name) = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")) {
        if !name.trim().is_empty() {
            return name.trim().to_string();
        }
    }
    if let Ok(out) = std::process::Command::new("hostname").arg("-s").output() {
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }
    "unknown-machine".to_string()
}

/// Today's UTC date as YYYY-MM-DD, without a date-time dependency
/// (Hinnant's civil-from-days algorithm).
pub fn today_utc_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_date_is_sane() {
        let d = today_utc_iso();
        assert_eq!(d.len(), 10);
        let year: i32 = d[0..4].parse().unwrap();
        assert!((2026..2100).contains(&year), "year out of range: {d}");
        assert_eq!(&d[4..5], "-");
        let month: u32 = d[5..7].parse().unwrap();
        let day: u32 = d[8..10].parse().unwrap();
        assert!((1..=12).contains(&month) && (1..=31).contains(&day));
    }

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
