//! Map legends (WO-0007 step 4): one pure content builder per layer, one
//! egui renderer in app.rs. The spec is plain data so tests can assert every
//! layer produces a legend without a GPU or a window.
//!
//! The legend always describes the VIEWED keyframe (`viewing_kf`), not the
//! latest one — the caller hands the keyframe in (WO-0007 step 5).

use eframe::egui;
use worldmaker_sim::tectonics::{lithology, Keyframe, SLAB_DETACH_MY};

use crate::layers::{self, Layer};

/// deg/My → cm/yr at the Earth's surface (peak speed, 90° from the Euler
/// pole): ω[rad/My]·R. R = 6371 km = 6.371e8 cm; 1 My = 1e6 yr.
pub const CM_YR_PER_DEG_MY: f32 = (core::f64::consts::PI / 180.0 * 6.371e8 / 1e6) as f32;

/// Vertical color-bar samples, bottom (t = 0) to top (t = 1).
pub const RAMP_SAMPLES: usize = 64;

pub struct LegendSpec {
    pub title: &'static str,
    pub kind: LegendKind,
}

pub enum LegendKind {
    /// Vertical color bar. `colors` run bottom → top; ticks and the optional
    /// marker are fractions 0..=1 from the bottom, with their labels.
    Ramp {
        colors: Vec<[f32; 3]>,
        ticks: Vec<(f32, String)>,
        marker: Option<(f32, String)>,
    },
    /// One color swatch per row, largest plates first, plus "+N more".
    Swatches {
        rows: Vec<SwatchRow>,
        more_count: usize,
    },
    /// Arrow-length scale: the longest drawn arrow corresponds to
    /// `max_speed_cm_yr` (arrow length is proportional to plate speed).
    ArrowScale {
        max_speed_cm_yr: f32,
        note: &'static str,
    },
}

pub struct SwatchRow {
    pub color: [f32; 3],
    pub label: String,
}

/// Land fraction (percent of cells at or above `sea_level_m`, keyframe-
/// relative meters), sampled every 8th cell (WO-0007 step 3).
pub fn land_fraction_pct(elev_m: &[i16], sea_level_m: f32) -> f32 {
    let mut land = 0usize;
    let mut total = 0usize;
    let mut i = 0;
    while i < elev_m.len() {
        total += 1;
        if elev_m[i] as f32 >= sea_level_m {
            land += 1;
        }
        i += 8;
    }
    if total == 0 {
        0.0
    } else {
        land as f32 * 100.0 / total as f32
    }
}

/// Elevation-bar span, keyframe-relative meters (matches the slider range
/// bottom and the land ramp top).
const ELEV_BAR_MIN_M: f32 = -6000.0;
const ELEV_BAR_MAX_M: f32 = 5500.0;

fn elev_frac(e_m: f32) -> f32 {
    ((e_m - ELEV_BAR_MIN_M) / (ELEV_BAR_MAX_M - ELEV_BAR_MIN_M)).clamp(0.0, 1.0)
}

/// Build the legend for one layer over the viewed keyframe.
pub fn legend_spec(layer: Layer, kf: &Keyframe, sea_level_m: f32) -> LegendSpec {
    legend_spec_with(layer, kf, sea_level_m, None)
}

/// [`legend_spec`] with an optional displayed-lithology override
/// (WO-0009 S2): when the terrain view is on, the Lithology legend counts
/// the deposition-stamped classes actually on screen (`su` included).
pub fn legend_spec_with(
    layer: Layer,
    kf: &Keyframe,
    sea_level_m: f32,
    lith_override: Option<&[u8]>,
) -> LegendSpec {
    match layer {
        Layer::Elevation => {
            // The bar is labeled in keyframe-relative meters; each sample is
            // colored exactly as the map colors that elevation under the
            // CURRENT sea level (hypsometric(e − sea)), so the coastline
            // break rides the marker as the slider moves.
            let colors = (0..RAMP_SAMPLES)
                .map(|i| {
                    let t = (i as f32 + 0.5) / RAMP_SAMPLES as f32;
                    let e = ELEV_BAR_MIN_M + t * (ELEV_BAR_MAX_M - ELEV_BAR_MIN_M);
                    layers::hypsometric(e - sea_level_m)
                })
                .collect();
            let ticks = [-6000.0f32, -4000.0, -2000.0, 0.0, 1000.0, 3000.0, 5500.0]
                .iter()
                .map(|&e| (elev_frac(e), format!("{e:.0} m")))
                .collect();
            let marker = Some((elev_frac(sea_level_m), format!("sea {sea_level_m:.0} m")));
            LegendSpec {
                title: "Elevation",
                kind: LegendKind::Ramp {
                    colors,
                    ticks,
                    marker,
                },
            }
        }
        Layer::Plates | Layer::PlateVelocity | Layer::VelocityField => {
            let n = kf.plate_id.len().max(1);
            // Cell census per alive plate, id-ordered (deterministic).
            let mut rows: Vec<(u32, usize, f32)> = kf
                .plates
                .iter()
                .filter(|p| p.alive)
                .map(|p| {
                    let cells = kf
                        .plate_id
                        .iter()
                        .filter(|&&pid| pid as u32 == p.id)
                        .count();
                    (p.id, cells, p.speed_deg_my)
                })
                .filter(|&(_, cells, _)| cells > 0)
                .collect();
            rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let more_count = rows.len().saturating_sub(12);
            let rows = rows
                .into_iter()
                .take(12)
                .map(|(id, cells, speed)| SwatchRow {
                    color: layers::plate_color(id),
                    label: format!(
                        "{id}  {:.1}%  {:.1} cm/yr",
                        cells as f32 * 100.0 / n as f32,
                        speed * CM_YR_PER_DEG_MY
                    ),
                })
                .collect();
            let swatches = LegendKind::Swatches { rows, more_count };
            match layer {
                Layer::Plates => LegendSpec {
                    title: "Plates",
                    kind: swatches,
                },
                // The two velocity layers show the arrow scale (their base
                // shading is the Plates layer; the arrows are the content).
                _ => {
                    let max_speed = kf
                        .plates
                        .iter()
                        .filter(|p| p.alive)
                        .map(|p| p.speed_deg_my)
                        .fold(0.0f32, f32::max);
                    LegendSpec {
                        title: if layer == Layer::PlateVelocity {
                            "Plate velocity"
                        } else {
                            "Velocity field"
                        },
                        kind: LegendKind::ArrowScale {
                            max_speed_cm_yr: max_speed * CM_YR_PER_DEG_MY,
                            note: "white = velocity",
                        },
                    }
                }
            }
        }
        Layer::CrustAge => {
            // bake_values stores t = 1 − age/AGE_MAX_MY, so age 0 (ridge)
            // is the BRIGHT end. Bar bottom = oldest, top = youngest.
            let colors = (0..RAMP_SAMPLES)
                .map(|i| {
                    let t = (i as f32 + 0.5) / RAMP_SAMPLES as f32;
                    layers::viridis(t)
                })
                .collect();
            let max = layers::AGE_MAX_MY;
            let ticks = [0.0f32, 50.0, 100.0, max]
                .iter()
                .map(|&age| (1.0 - age / max, format!("{age:.0} My")))
                .collect();
            LegendSpec {
                title: "Crust age",
                kind: LegendKind::Ramp {
                    colors,
                    ticks,
                    marker: None,
                },
            }
        }
        Layer::Thickness => {
            // bake_values maps thickness 5..70 km linearly onto batlow.
            let colors = (0..RAMP_SAMPLES)
                .map(|i| {
                    let t = (i as f32 + 0.5) / RAMP_SAMPLES as f32;
                    layers::batlow(t)
                })
                .collect();
            let ticks = [5.0f32, 20.0, 35.0, 50.0, 70.0]
                .iter()
                .map(|&km| (((km - 5.0) / 65.0), format!("{km:.0} km")))
                .collect();
            LegendSpec {
                title: "Thickness",
                kind: LegendKind::Ramp {
                    colors,
                    ticks,
                    marker: None,
                },
            }
        }
        Layer::Lithology => {
            // Only the classes actually present in the displayed field
            // (WO-0009 S2 step 3), largest area first, id tie-break.
            let lith = lith_override.unwrap_or(&kf.lithology);
            let n = lith.len().max(1);
            let mut counts = [0usize; lithology::CLASS_COUNT];
            for &l in lith {
                counts[(l as usize).min(lithology::CLASS_COUNT - 1)] += 1;
            }
            let mut present: Vec<(usize, usize)> = counts
                .iter()
                .enumerate()
                .filter(|&(_, &c)| c > 0)
                .map(|(i, &c)| (i, c))
                .collect();
            present.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let rows = present
                .into_iter()
                .map(|(class, cells)| SwatchRow {
                    color: layers::lithology_color(class),
                    label: format!(
                        "{} {:.1}%  {}",
                        lithology::CODES[class],
                        cells as f32 * 100.0 / n as f32,
                        lithology::NAMES[class],
                    ),
                })
                .collect();
            LegendSpec {
                title: "Lithology",
                kind: LegendKind::Swatches {
                    rows,
                    more_count: 0,
                },
            }
        }
        Layer::Overlay => {
            // Two swatches: a slab the step it went under (fade 1.0) and one
            // most of the way to detachment, both over the dimmed base —
            // exactly the shader's mix(base·0.4, slab_color, fade).
            let slab = layers::plate_color(0);
            let base = [slab[0] * 0.4, slab[1] * 0.4, slab[2] * 0.4];
            let mix = |t: f32| {
                [
                    base[0] + (slab[0] - base[0]) * t,
                    base[1] + (slab[1] - base[1]) * t,
                    base[2] + (slab[2] - base[2]) * t,
                ]
            };
            let rows = vec![
                SwatchRow {
                    color: mix(1.0),
                    label: "attached slab (0 My)".into(),
                },
                SwatchRow {
                    color: mix(0.15),
                    label: format!("detaching ({SLAB_DETACH_MY:.0} My)"),
                },
            ];
            LegendSpec {
                title: "Overlay",
                kind: LegendKind::Swatches {
                    rows,
                    more_count: 0,
                },
            }
        }
    }
}

// ----- egui rendering (the one legend implementation, WO-0007 step 4) -----

fn c32(c: [f32; 3]) -> egui::Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    egui::Color32::from_rgb(q(c[0]), q(c[1]), q(c[2]))
}

/// Render one legend body. Pure egui — the caller owns the anchored panel
/// and the collapsing header.
pub fn legend_body(ui: &mut egui::Ui, spec: &LegendSpec) {
    match &spec.kind {
        LegendKind::Ramp {
            colors,
            ticks,
            marker,
        } => {
            let (bar_w, bar_h, label_w, pad_v) = (14.0, 150.0, 64.0, 7.0);
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(bar_w + 10.0 + label_w, bar_h + 2.0 * pad_v),
                egui::Sense::hover(),
            );
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.min.x, rect.min.y + pad_v),
                egui::vec2(bar_w, bar_h),
            );
            let p = ui.painter();
            let n = colors.len().max(1);
            for (i, c) in colors.iter().enumerate() {
                let y1 = bar.bottom() - bar.height() * i as f32 / n as f32;
                let y0 = bar.bottom() - bar.height() * (i + 1) as f32 / n as f32;
                p.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(bar.left(), y0),
                        egui::pos2(bar.right(), y1),
                    ),
                    0.0,
                    c32(*c),
                );
            }
            let text = ui.visuals().text_color();
            for (t, label) in ticks {
                let y = bar.bottom() - t.clamp(0.0, 1.0) * bar.height();
                p.line_segment(
                    [egui::pos2(bar.right(), y), egui::pos2(bar.right() + 4.0, y)],
                    egui::Stroke::new(1.0, text),
                );
                p.text(
                    egui::pos2(bar.right() + 6.0, y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    egui::FontId::proportional(10.0),
                    text,
                );
            }
            if let Some((t, label)) = marker {
                let y = bar.bottom() - t.clamp(0.0, 1.0) * bar.height();
                p.line_segment(
                    [
                        egui::pos2(bar.left() - 2.0, y),
                        egui::pos2(bar.right() + 2.0, y),
                    ],
                    egui::Stroke::new(2.0, egui::Color32::WHITE),
                );
                ui.horizontal(|ui| {
                    let (r, _) =
                        ui.allocate_exact_size(egui::vec2(12.0, 3.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, 0.0, egui::Color32::WHITE);
                    ui.small(label);
                });
            }
        }
        LegendKind::Swatches { rows, more_count } => {
            for row in rows {
                ui.horizontal(|ui| {
                    let (r, _) =
                        ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, 2.0, c32(row.color));
                    ui.small(&row.label);
                });
            }
            if *more_count > 0 {
                ui.small(format!("+{more_count} more"));
            }
        }
        LegendKind::ArrowScale {
            max_speed_cm_yr,
            note,
        } => {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(80.0, 14.0), egui::Sense::hover());
            let p = ui.painter();
            let y = rect.center().y;
            let a = egui::pos2(rect.min.x + 2.0, y);
            let b = egui::pos2(rect.min.x + 74.0, y);
            let stroke = egui::Stroke::new(2.0, egui::Color32::WHITE);
            p.line_segment([a, b], stroke);
            p.line_segment([b, b + egui::vec2(-7.0, -4.0)], stroke);
            p.line_segment([b, b + egui::vec2(-7.0, 4.0)], stroke);
            ui.small(format!("longest arrow = {max_speed_cm_yr:.1} cm/yr"));
            ui.small(*note);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use worldmaker_sim::tectonics::PlateState;

    /// A tiny synthetic keyframe: 16 cells over two alive plates.
    fn synthetic_kf() -> Keyframe {
        let n = 16;
        let plate = |id: u32, speed: f32| PlateState {
            id,
            alive: true,
            pole: [0.0, 0.0, 1.0],
            speed_deg_my: speed,
            youngest_suture_my: f32::MAX,
            youngest_rift_my: f32::MAX,
            youngest_breakup_my: f32::MAX,
            quiet_my: 0.0,
            pending_rot: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            pending_deg: 0.0,
            slab: Vec::new(),
            boundary_cells: 0,
            subducting_cells: 0,
            colliding_cells: 0,
            colliding_strength: 0.0,
            ridge_cells: 0,
            transform_cells: 0,
            drive_torque: [0.0, 0.0, 0.0],
        };
        Keyframe {
            t_my: 100.0,
            sea_offset_m: 0.0,
            water_mass_kg: 0.0,
            elev_m: (0..n).map(|i| (i as i32 * 800 - 6400) as i16).collect(),
            plate_id: (0..n).map(|i| (i % 2) as u16).collect(),
            crust_age_my: vec![10; n],
            thickness_ckm: vec![700; n],
            orogeny_age_my: vec![0; n],
            rift_age_my: vec![0; n],
            buildup_ckm: vec![0; n],
            flags: vec![0; n],
            lithology: vec![worldmaker_sim::tectonics::lithology::SM; n],
            slab_plate: vec![u16::MAX; n],
            slab_since_my: vec![0; n],
            suture_at_my: vec![u16::MAX; n],
            hotspot_cont_my: Vec::new(),
            rifts: Vec::new(),
            plates: vec![plate(0, 0.5), plate(1, 1.0)],
            collisions: Vec::new(),
            welds: Vec::new(),
        }
    }

    /// WO-0007 step 6: lowering sea level can only grow (or hold) the
    /// land-fraction readout.
    #[test]
    fn land_fraction_grows_as_sea_level_drops() {
        let elev: Vec<i16> = (0..1000).map(|i| (i * 12 - 7000) as i16).collect();
        let at_zero = land_fraction_pct(&elev, 0.0);
        let at_deep = land_fraction_pct(&elev, -6000.0);
        assert!(
            at_deep >= at_zero,
            "land fraction at −6000 m ({at_deep}) < at 0 m ({at_zero})"
        );
        // And on this synthetic set it must strictly grow.
        assert!(at_deep > at_zero);
        // Empty input must not panic or divide by zero.
        assert_eq!(land_fraction_pct(&[], 0.0), 0.0);
    }

    /// WO-0007 step 6: every layer returns non-empty legend content.
    #[test]
    fn every_layer_has_legend_content() {
        let kf = synthetic_kf();
        for layer in Layer::ALL {
            let spec = legend_spec(layer, &kf, -500.0);
            assert!(!spec.title.is_empty());
            match spec.kind {
                LegendKind::Ramp { colors, ticks, .. } => {
                    assert_eq!(colors.len(), RAMP_SAMPLES, "{}", spec.title);
                    assert!(!ticks.is_empty(), "{}", spec.title);
                    for (t, label) in &ticks {
                        assert!((0.0..=1.0).contains(t), "{}: tick {t}", spec.title);
                        assert!(!label.is_empty());
                    }
                }
                LegendKind::Swatches { rows, .. } => {
                    assert!(!rows.is_empty(), "{}", spec.title);
                    for row in &rows {
                        assert!(!row.label.is_empty());
                    }
                }
                LegendKind::ArrowScale {
                    max_speed_cm_yr,
                    note,
                } => {
                    assert!(max_speed_cm_yr > 0.0, "{}", spec.title);
                    assert!(!note.is_empty());
                }
            }
        }
    }

    /// The elevation legend's sea marker rides the slider.
    #[test]
    fn elevation_marker_tracks_sea_level() {
        let kf = synthetic_kf();
        let frac_at = |sea: f32| match legend_spec(Layer::Elevation, &kf, sea).kind {
            LegendKind::Ramp {
                marker: Some((f, _)),
                ..
            } => f,
            _ => panic!("elevation legend lost its marker"),
        };
        assert!(frac_at(-4000.0) < frac_at(0.0));
        assert!(frac_at(0.0) < frac_at(3000.0));
    }

    /// Plates legend: 12-row cap plus the "+N more" count.
    #[test]
    fn plates_legend_caps_at_twelve() {
        let mut kf = synthetic_kf();
        let n = 32;
        kf.elev_m = vec![0; n];
        kf.plate_id = (0..n).map(|i| (i % 16) as u16).collect();
        kf.plates = (0..16)
            .map(|id| {
                let mut p = kf.plates[0].clone();
                p.id = id;
                p
            })
            .collect();
        match legend_spec(Layer::Plates, &kf, 0.0).kind {
            LegendKind::Swatches { rows, more_count } => {
                assert_eq!(rows.len(), 12);
                assert_eq!(more_count, 4);
            }
            _ => panic!("plates legend is not swatches"),
        }
    }
}
