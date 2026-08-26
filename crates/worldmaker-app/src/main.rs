//! WorldMaker application entry point: logging, GPU preflight, argument
//! parsing, and the eframe launch.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod harness;
mod layers;
mod pending_edits;
mod render;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use eframe::egui;
use eframe::egui_wgpu::{wgpu, SurfaceConfig, WgpuConfiguration};

use worldmaker_core::hash::hash_f32_slice;
use worldmaker_core::Grid;
use worldmaker_sim::noise_stage::ELEVATION_FIELD;
use worldmaker_sim::{NoiseElevationStage, Pipeline, StageContext, WorldState};

struct Args {
    screenshots_dir: Option<PathBuf>,
    perf_out: Option<PathBuf>,
    determinism_out: Option<PathBuf>,
    tectonics_out: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut out = Args {
        screenshots_dir: None,
        perf_out: None,
        determinism_out: None,
        tectonics_out: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--screenshots" => out.screenshots_dir = args.next().map(PathBuf::from),
            "--perf-out" => out.perf_out = args.next().map(PathBuf::from),
            "--determinism-out" => out.determinism_out = args.next().map(PathBuf::from),
            "--tectonics-results" => out.tectonics_out = args.next().map(PathBuf::from),
            other => log::warn!("ignoring unknown argument: {other}"),
        }
    }
    out
}

fn init_logging() {
    let log_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let spec = flexi_logger::FileSpec::default()
        .directory(log_dir)
        .basename("worldmaker");
    match flexi_logger::Logger::try_with_env_or_str("info").and_then(|l| {
        l.log_to_file(spec)
            .rotate(
                flexi_logger::Criterion::Size(2 * 1024 * 1024),
                flexi_logger::Naming::Numbers,
                flexi_logger::Cleanup::KeepLogFiles(5),
            )
            .start()
    }) {
        Ok(_) => log::info!("WorldMaker {} starting", env!("CARGO_PKG_VERSION")),
        Err(e) => eprintln!("logging unavailable: {e}"),
    }
}

/// Plain-language error dialog (native message box on Windows, stderr
/// elsewhere), then clean exit.
fn fatal_dialog(title: &str, text: &str) -> ! {
    log::error!("{title}: {text}");
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "user32")]
        extern "system" {
            fn MessageBoxW(hwnd: usize, text: *const u16, caption: *const u16, utype: u32) -> i32;
        }
        let wide = |s: &str| -> Vec<u16> {
            std::ffi::OsStr::new(s)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        };
        let t = wide(text);
        let c = wide(title);
        unsafe {
            // 0x10 = MB_ICONERROR
            MessageBoxW(0, t.as_ptr(), c.as_ptr(), 0x10);
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("{title}\n\n{text}");
    }
    std::process::exit(1)
}

/// Verify a GPU is reachable before handing control to eframe, so failure is
/// a friendly dialog instead of a panic backtrace.
fn gpu_preflight() {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }));
    match adapter {
        Ok(a) => {
            let info = a.get_info();
            log::info!(
                "GPU: {} ({:?}, {:?})",
                info.name,
                info.backend,
                info.device_type
            );
        }
        Err(e) => {
            let machine = worldmaker_io::results::machine_name();
            fatal_dialog(
                "WorldMaker cannot start",
                &format!(
                    "WorldMaker could not find a usable graphics device on this \
                     computer ({machine}).\n\n\
                     The most likely fix is updating the graphics driver — on this \
                     PC that is the NVIDIA GeForce driver, available from \
                     nvidia.com/drivers.\n\n\
                     If the problem continues, a log file named worldmaker_*.log \
                     next to WorldMaker.exe has the technical details.\n\n\
                     Technical detail: {e}"
                ),
            );
        }
    }
}

/// CPU-only determinism harness: hash the elevation field for a fixed seed,
/// twice, at two grid levels, and write the results JSON.
fn run_determinism_harness(out: &std::path::Path) -> anyhow::Result<()> {
    let mut metrics = serde_json::Map::new();
    for level in [6u32, 7] {
        let mut hashes = Vec::new();
        for _run in 0..2 {
            let grid = Arc::new(Grid::build(level));
            let mut world = WorldState::new(grid);
            let mut pipe = Pipeline::new();
            pipe.push(Box::new(NoiseElevationStage::default()));
            pipe.run(&StageContext::new(42), &mut world)?;
            hashes.push(hash_f32_slice(world.fields.get(ELEVATION_FIELD).unwrap()));
        }
        metrics.insert(
            format!("elevation_hash_L{level}_seed42"),
            serde_json::json!(format!("{:#018x}", hashes[0])),
        );
        metrics.insert(
            format!("elevation_hash_L{level}_seed42_repeat_matches"),
            serde_json::json!(hashes[0] == hashes[1]),
        );
    }
    let file = worldmaker_io::ResultsFile::new(
        &worldmaker_io::results::today_utc_iso(),
        serde_json::Value::Object(metrics),
    );
    file.write(out)?;
    log::info!("determinism results written to {}", out.display());
    Ok(())
}

fn main() {
    init_logging();
    let args = parse_args();

    if let Some(det_out) = &args.determinism_out {
        if let Err(e) = run_determinism_harness(det_out) {
            log::error!("determinism harness failed: {e:#}");
            std::process::exit(1);
        }
    }
    if let Some(tect_out) = &args.tectonics_out {
        if let Err(e) = harness::run_tectonics_harness(tect_out) {
            log::error!("tectonics harness failed: {e:#}");
            std::process::exit(1);
        }
    }
    // Headless-only invocation: done without opening a window.
    if (args.determinism_out.is_some() || args.tectonics_out.is_some())
        && args.perf_out.is_none()
        && args.screenshots_dir.is_none()
    {
        return;
    }

    gpu_preflight();

    // Perf mode measures grid builds before the window opens.
    let mut grid_build_ms = Vec::new();
    if args.perf_out.is_some() {
        for level in 6..=9u32 {
            let t0 = Instant::now();
            let g = Grid::build(level);
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            log::info!(
                "perf: grid L{level} ({} cells) built in {ms:.1} ms",
                g.cell_count()
            );
            grid_build_ms.push((level, (ms * 10.0).round() / 10.0));
        }
    }

    let perf_mode = args.perf_out.is_some();
    let script = app::Script {
        screenshots_dir: args.screenshots_dir,
        perf_out: args.perf_out,
        grid_build_ms,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WorldMaker")
            .with_inner_size([1600.0, 900.0])
            .with_min_inner_size([960.0, 600.0]),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: WgpuConfiguration {
            surface: SurfaceConfig {
                // Perf runs measure real frame cost, not the monitor's refresh.
                present_mode: if perf_mode {
                    wgpu::PresentMode::AutoNoVsync
                } else {
                    wgpu::PresentMode::AutoVsync
                },
                desired_maximum_frame_latency: None,
            },
            ..Default::default()
        },
        ..Default::default()
    };

    let result = eframe::run_native(
        "WorldMaker",
        options,
        Box::new(move |cc| Ok(Box::new(app::WorldApp::new(cc, script)))),
    );
    if let Err(e) = result {
        fatal_dialog(
            "WorldMaker could not open its window",
            &format!(
                "The app failed while creating its window or graphics surface.\n\n\
                 The most likely fix is updating the graphics driver (NVIDIA \
                 GeForce on this PC).\n\nTechnical detail: {e}"
            ),
        );
    }
    log::info!("WorldMaker exiting normally");
}
