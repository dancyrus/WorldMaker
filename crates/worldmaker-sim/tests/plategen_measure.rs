//! WO-0003 Fix 2, commit M1: measure the INCUMBENT plate generator on the 5
//! fixed competition seeds at L6 and L7 and commit the numbers FIRST
//! (d2-fix2-design §6, protocol step P0). Setup-only: `SimState::setup`,
//! no pipeline run — the metrics read plate geometry only.
//!
//! `#[ignore]` because it writes a committed results file (run deliberately
//! with `--ignored`); the committed JSON is the record, per ground rule 3.

use std::path::PathBuf;
use std::sync::Arc;

use worldmaker_core::hash::seed_from_text;
use worldmaker_core::Grid;
use worldmaker_io::results::{today_utc_iso, ResultsFile};
use worldmaker_sim::tectonics::{metrics, SimState, TectonicsParams};

/// The 5 fixed competition seeds (pinned in Stage D, before any measurement).
const SEEDS: [(u64, &str); 5] = [
    (42, "42"),
    (0xc4be_0bf8_f497_a575, "cyrus"),
    (7, "7"),
    (1002, "1002"),
    (271828, "271828"),
];

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

#[test]
#[ignore = "one-shot record: writes docs/results/tectonics-feelpass-{machine}.json (run with --ignored)"]
fn measure_incumbent_plategen_baseline() {
    // The "cyrus" label is the app-default seed of the committed BEFORE
    // screenshots; pin the constant to the text it comes from.
    assert_eq!(seed_from_text("cyrus"), 0xc4be_0bf8_f497_a575);

    let params = TectonicsParams::default();
    let mut mm = serde_json::Map::new();
    mm.insert(
        "plategen_generator".into(),
        serde_json::Value::from("incumbent"),
    );
    mm.insert(
        "plategen_seed_cyrus".into(),
        serde_json::Value::from("0xc4be0bf8f497a575"),
    );

    for level in [6u32, 7] {
        let grid = Arc::new(Grid::build(level));
        for (seed, label) in SEEDS {
            let s = SimState::setup(seed, &grid, &params);
            let cv = metrics::plate_area_cv(&s.plate_id, params.plate_count);
            let rep = metrics::boundary_sinuosity(&grid, &s.plate_id);
            mm.insert(
                format!("plategen_area_cv_l{level}_seed{label}"),
                serde_json::Value::from(round4(cv)),
            );
            mm.insert(
                format!("plategen_sinuosity_l{level}_seed{label}"),
                serde_json::Value::from(round4(rep.weighted_mean)),
            );
            mm.insert(
                format!("plategen_open_segments_l{level}_seed{label}"),
                serde_json::Value::from(rep.open_segment_count),
            );
            mm.insert(
                format!("plategen_loops_l{level}_seed{label}"),
                serde_json::Value::from(rep.loop_count),
            );
        }
    }

    // F12/F5: filename from machine_name(), path anchored on the crate dir
    // (integration tests run with cwd = crates/worldmaker-sim).
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../docs/results/tectonics-feelpass-{}.json",
        worldmaker_io::results::machine_name()
    ));
    ResultsFile::new(&today_utc_iso(), serde_json::Value::Object(mm))
        .write(&path)
        .expect("failed to write results file");
    println!("wrote {}", path.display());
}
