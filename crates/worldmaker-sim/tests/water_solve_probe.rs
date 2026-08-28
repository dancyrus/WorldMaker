//! WO-0009 step 11 dev probe: the conserved-water sea-level solve over a
//! full 2 Gy L6 run at seeds 42 and "cyrus" — the t = 0 water inventory
//! and the solved level per keyframe — written to
//! docs/results/water-solve-<seed>.json.
//!
//! Run with:
//!   cargo test -p worldmaker-sim --release water_solve_probe -- --ignored --nocapture

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use worldmaker_core::hash::seed_from_text;
use worldmaker_core::Grid;
use worldmaker_io::results::{today_utc_iso, ResultsFile};
use worldmaker_sim::tectonics::{self, TectonicsParams};
use worldmaker_sim::{StageContext, WorldState};

#[test]
#[ignore = "dev probe: writes docs/results/water-solve-<seed>.json"]
fn water_solve_probe() {
    let grid = Arc::new(Grid::build(6));
    let date = today_utc_iso();
    for (label, seed) in [("42", 42u64), ("cyrus", seed_from_text("cyrus"))] {
        eprintln!("probing seed {label} ({seed:#x}) ...");
        let world = WorldState::new(grid.clone());
        let params = TectonicsParams {
            span_my: 2000.0,
            ..TectonicsParams::default()
        };
        let hist = tectonics::run_history(&StageContext::new(seed), &world, &params, None).unwrap();
        let n = grid.cell_count() as f64;
        let levels: Vec<serde_json::Value> = hist
            .keyframes
            .iter()
            .map(|kf| {
                let land = kf.elev_m.iter().filter(|&&e| e >= 0).count() as f64 / n;
                json!({
                    "t_my": kf.t_my,
                    "solved_level_m": kf.sea_offset_m,
                    "land_fraction": (land * 10_000.0).round() / 10_000.0,
                })
            })
            .collect();
        let s0 = hist.keyframes[0].sea_offset_m;
        let (mut min_s, mut max_s) = (f32::MAX, f32::MIN);
        for kf in &hist.keyframes {
            min_s = min_s.min(kf.sea_offset_m);
            max_s = max_s.max(kf.sea_offset_m);
        }
        let metrics = json!({
            "config": {
                "seed": format!("{seed:#x}"),
                "seed_label": label,
                "level": 6,
                "span_my": 2000.0,
                "params": "TectonicsParams::default()",
            },
            "water_mass_kg": hist.keyframes[0].water_mass_kg,
            "solved_level_t0_m": s0,
            "solved_level_min_m": min_s,
            "solved_level_max_m": max_s,
            "shoreline_swing_m": max_s - min_s,
            "keyframes": levels,
        });
        eprintln!(
            "seed {label}: mass {:.4e} kg, level t0 {s0:.1} m, range [{min_s:.1}, {max_s:.1}] m",
            hist.keyframes[0].water_mass_kg
        );
        let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/results"))
            .join(format!("water-solve-{label}.json"));
        ResultsFile::new(&date, metrics).write(&path).unwrap();
        eprintln!("wrote {}", path.display());
    }
}
