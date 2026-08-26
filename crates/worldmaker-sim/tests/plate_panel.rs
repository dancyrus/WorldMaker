//! WO-0003 Fix 2 dev-only plate-map panel (d2-fix2-design §5, reduced at
//! commit M3 to a standing dev tool). `#[ignore]` (precedent:
//! `generates_l9_without_panic`): run deliberately, never in CI.
//!
//! `render_plate_maps` renders equirect plate-id PNGs of `SimState::setup`
//! output — the wired generator, whatever it is — for the 5 competition
//! seeds at L7, for eyeballing future plate work. The competition-era
//! variants (`all_generators` sweep + `score_generators`) were deleted with
//! the losing candidates; the committed judge numbers live in
//! docs/results/plategen-feelpass-Daniels-MacBook-Air.json.
//!
//! std sin/cos are used for the raster only: this is a display path; nothing
//! here feeds a hash or a committed metric.

use std::path::PathBuf;
use std::sync::Arc;

use rayon::prelude::*;
use worldmaker_core::Grid;
use worldmaker_sim::tectonics::{SimState, TectonicsParams};

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
/// the panel is for comparing shapes, not hues.
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
#[ignore = "dev-only panel: renders 5 setup plate-id PNGs to target/plate-panel (run with --ignored)"]
fn render_plate_maps() {
    let dir = panel_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let grid = Arc::new(Grid::build(7));
    let params = TectonicsParams::default();
    for (seed, label) in SEEDS {
        let s = SimState::setup(seed, &grid, &params);
        let path = dir.join(format!("plates-setup-L7-seed{label}.png"));
        render_png(&grid, &s.plate_id, &path);
        println!("wrote {}", path.display());
    }
}
