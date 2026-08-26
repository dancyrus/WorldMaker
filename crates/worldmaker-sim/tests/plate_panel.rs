//! WO-0003 Fix 2 dev-only judge-panel outputs (d2-fix2-design §5). Both
//! tests are `#[ignore]` (precedent: `generates_l9_without_panic`): they are
//! run deliberately, not in CI.
//!
//! - `render_plate_maps`: equirect plate-id PNGs for all 4 generators × the
//!   5 competition seeds at L7, for the judge panel to score shapes.
//! - `score_generators`: CV + sinuosity for all 4 generators × 5 seeds ×
//!   {L6, L7}, written to docs/results/plategen-feelpass-{machine}.json —
//!   the judge's numbers are committed JSON, per ground rule 3.
//!
//! std sin/cos are used for the raster only: this is a display path; nothing
//! here feeds a hash or a committed metric.

use std::path::PathBuf;
use std::sync::Arc;

use rayon::prelude::*;
use worldmaker_core::Grid;
use worldmaker_io::results::{machine_name, today_utc_iso, ResultsFile};
use worldmaker_sim::tectonics::plate_gen::{all_generators, PlateGenParams};
use worldmaker_sim::tectonics::{metrics, SimState, TectonicsParams};

/// The 5 fixed competition seeds (pinned in Stage D, before any measurement).
const SEEDS: [(u64, &str); 5] = [
    (42, "42"),
    (0xc4be_0bf8_f497_a575, "cyrus"),
    (7, "7"),
    (1002, "1002"),
    (271828, "271828"),
];

/// Fixed color table copied from layers.rs PLATE_COLORS values (the sim
/// crate cannot link the app binary). The panel indexes by `plate_id % 24`
/// while the app ranks by plate size — hues will not match the app (F10);
/// judges compare shapes, not hues.
const PLATE_COLORS: [[u8; 3]; 24] = [
    [230, 25, 75],
    [60, 180, 75],
    [255, 225, 25],
    [0, 130, 200],
    [245, 130, 48],
    [145, 30, 180],
    [70, 240, 240],
    [240, 50, 230],
    [210, 245, 60],
    [250, 190, 212],
    [0, 128, 128],
    [220, 190, 255],
    [170, 110, 40],
    [255, 250, 200],
    [128, 0, 0],
    [170, 255, 195],
    [128, 128, 0],
    [255, 215, 180],
    [0, 0, 128],
    [128, 128, 128],
    [255, 255, 255],
    [0, 220, 130],
    [100, 155, 255],
    [60, 60, 60],
];

const W: usize = 1024;
const H: usize = 512;

/// Output dir: env WM_PLATE_PANEL_DIR verbatim when set, else
/// target/plate-panel/ anchored on the workspace root (F5: integration
/// tests run with cwd = crates/worldmaker-sim).
fn panel_dir() -> PathBuf {
    match std::env::var("WM_PLATE_PANEL_DIR") {
        Ok(d) if !d.trim().is_empty() => PathBuf::from(d),
        _ => PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/plate-panel"
        )),
    }
}

fn render_png(grid: &Grid, plate_id: &[u32], path: &std::path::Path) {
    let n = grid.cell_count() as usize;
    // Boundary cells (≥1 foreign CSR neighbor) darkened so sinuosity is
    // visible at a glance.
    let mut is_boundary = vec![false; n];
    for (c, ib) in is_boundary.iter_mut().enumerate() {
        *ib = grid
            .neighbors_of(c as u32)
            .iter()
            .any(|&nb| plate_id[nb as usize] != plate_id[c]);
    }

    let mut buf = vec![0u8; W * H * 3];
    buf.par_chunks_mut(W * 3)
        .enumerate()
        .for_each(|(row, out)| {
            let lat =
                std::f32::consts::FRAC_PI_2 - std::f32::consts::PI * (row as f32 + 0.5) / H as f32;
            let (sin_lat, cos_lat) = lat.sin_cos();
            let mut hint: Option<u32> = None;
            for col in 0..W {
                let lon = -std::f32::consts::PI
                    + 2.0 * std::f32::consts::PI * (col as f32 + 0.5) / W as f32;
                let (sin_lon, cos_lon) = lon.sin_cos();
                let p = [cos_lat * cos_lon, cos_lat * sin_lon, sin_lat];
                let cell = grid.nearest_cell(p, hint);
                hint = Some(cell);
                let pid = plate_id[cell as usize] as usize;
                let base = PLATE_COLORS[pid % PLATE_COLORS.len()];
                let k = if is_boundary[cell as usize] {
                    0.45
                } else {
                    1.0
                };
                for ch in 0..3 {
                    out[col * 3 + ch] = (base[ch] as f32 * k) as u8;
                }
            }
        });

    let img =
        image::RgbImage::from_raw(W as u32, H as u32, buf).expect("raster buffer size mismatch");
    img.save(path).expect("failed to write panel PNG");
}

#[test]
#[ignore = "dev-only judge panel: renders 20 plate-id PNGs to target/plate-panel (run with --ignored)"]
fn render_plate_maps() {
    let dir = panel_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let grid = Arc::new(Grid::build(7));
    let params = TectonicsParams::default();
    let gp = PlateGenParams::from(&params);
    for g in all_generators() {
        for (seed, label) in SEEDS {
            let plate_id = if g.name() == "incumbent" {
                // Sanity: the setup.rs wiring must match the trait impl.
                let s = SimState::setup(seed, &grid, &params);
                let direct = g.generate(seed, &grid, &gp);
                assert_eq!(
                    s.plate_id, direct,
                    "SimState::setup diverged from Incumbent::generate"
                );
                s.plate_id
            } else {
                g.generate(seed, &grid, &gp)
            };
            let path = dir.join(format!("plates-{}-L7-seed{}.png", g.name(), label));
            render_png(&grid, &plate_id, &path);
            println!("wrote {}", path.display());
        }
    }
}

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

#[test]
#[ignore = "one-shot record: writes docs/results/plategen-feelpass-{machine}.json (run with --ignored)"]
fn score_generators() {
    let params = TectonicsParams::default();
    let gp = PlateGenParams::from(&params);
    let mut mm = serde_json::Map::new();
    mm.insert(
        "plategen_seed_cyrus".into(),
        serde_json::Value::from("0xc4be0bf8f497a575"),
    );
    for level in [6u32, 7] {
        let grid = Arc::new(Grid::build(level));
        for g in all_generators() {
            for (seed, label) in SEEDS {
                let ids = g.generate(seed, &grid, &gp);
                let cv = metrics::plate_area_cv(&ids, params.plate_count);
                let rep = metrics::boundary_sinuosity(&grid, &ids);
                let name = g.name();
                mm.insert(
                    format!("{name}_area_cv_l{level}_seed{label}"),
                    serde_json::Value::from(round4(cv)),
                );
                mm.insert(
                    format!("{name}_sinuosity_l{level}_seed{label}"),
                    serde_json::Value::from(round4(rep.weighted_mean)),
                );
                mm.insert(
                    format!("{name}_open_segments_l{level}_seed{label}"),
                    serde_json::Value::from(rep.open_segment_count),
                );
                mm.insert(
                    format!("{name}_loops_l{level}_seed{label}"),
                    serde_json::Value::from(rep.loop_count),
                );
            }
        }
    }
    // F12: filename from machine_name() at runtime, F5: crate-dir anchored.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../docs/results/plategen-feelpass-{}.json",
        machine_name()
    ));
    ResultsFile::new(&today_utc_iso(), serde_json::Value::Object(mm))
        .write(&path)
        .expect("failed to write results file");
    println!("wrote {}", path.display());
}
