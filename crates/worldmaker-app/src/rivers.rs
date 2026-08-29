//! River and lake extraction from a terrain run (WO-0009 S3).
//!
//! Display-side companion to the S2 terrain stage: reads the stage's final
//! receiver tree, discharge and lake depths (all committed sim outputs —
//! nothing here feeds a golden) and produces smoothed river polylines for
//! the boundary-ribbon render path. A river exists where discharge exceeds
//! [`RIVER_MIN_DISCHARGE_M3S`]; its drawn width scales with sqrt(Q)
//! (Leopold & Maddock 1953 downstream hydraulic geometry, w ∝ Q^0.5), and
//! every segment carries its Strahler order.
//!
//! Segmentation: a segment runs source → next confluence (junction cells
//! close the upstream segment and open the downstream one, so chains meet
//! on screen), and a segment reaching water appends its first ocean or
//! lake cell so the line visually enters the water body — rivers never
//! cross a lake; the outlet spill cell starts a fresh source segment
//! downstream. Deterministic: serial id-ordered scans and the same
//! (descending filled surface, id tie-break) topological order the terrain
//! stage routes with.

use worldmaker_core::Grid;
use worldmaker_sim::terrain::{TerrainOutput, RECV_NONE};

use crate::boundaries::BoundaryChain;

/// Minimum discharge (m³/s) for a drawn river. Physical, so the network a
/// world shows does not depend on grid level: ~a Rhine-scale river
/// (2000 m³/s ≈ 60 km³/yr). At L7 one cell's own runoff is ~100 m³/s, so
/// a drawn river drains tens of cells and the network reads dendritic
/// instead of furring every coastal cell.
pub const RIVER_MIN_DISCHARGE_M3S: f32 = 2000.0;

/// Ribbon btype for rivers: fs_bnd maps btype → LUT row 5 texel (btype−1),
/// so 10 reads texel 9 — RIVER_BLUE in layers.rs.
pub const BTYPE_RIVER: u8 = 10;

/// Width scale ∝ sqrt(Q/threshold), clamped: 1.0 at the threshold, capped
/// so a continental trunk stays a line, not a band.
const WIDTH_MAX_SCALE: f32 = 4.0;

/// One river segment: source (or confluence) down to the next confluence
/// or the water body it enters. `cells` run upstream → downstream; when the
/// river reaches water the final cell is the first ocean or lake cell.
pub struct RiverSegment {
    /// Strahler order of this segment (the order of its first cell).
    pub order: u8,
    /// Cell path, upstream → downstream.
    pub cells: Vec<u32>,
    /// True when the last cell is the water cell the river enters
    /// (ocean or lake) rather than a confluence handoff.
    pub enters_water: bool,
}

/// The full extracted river network for one terrain run.
pub struct RiverSet {
    pub segments: Vec<RiverSegment>,
    /// Strahler order per cell (0 = not a river cell). Grid-sized.
    pub strahler: Vec<u8>,
    pub threshold_m3s: f32,
}

/// Is `c` a drawn-river cell: subaerial land (not a lake bed), enough
/// discharge, and a receiver to flow to.
fn is_river(out: &TerrainOutput, threshold: f32, c: usize) -> bool {
    out.elev_m[c] > 0.0
        && out.lake_depth_m[c] <= 0.0
        && out.discharge_m3s[c] >= threshold
        && out.receiver[c] != RECV_NONE
}

/// Extract the river network from one terrain run.
pub fn extract(out: &TerrainOutput, threshold_m3s: f32) -> RiverSet {
    let n = out.elev_m.len();
    let river: Vec<bool> = (0..n).map(|c| is_river(out, threshold_m3s, c)).collect();

    // River-donor count per cell (donors iterate ascending id).
    let mut donor_count = vec![0u16; n];
    for c in 0..n {
        if river[c] {
            let r = out.receiver[c] as usize;
            if river[r] {
                donor_count[r] += 1;
            }
        }
    }

    // Strahler order, donors before receivers: the terrain stage's
    // topological order (descending filled surface, id tie-break).
    let mut order: Vec<u32> = (0..n as u32).collect();
    order.sort_unstable_by(|&a, &b| {
        out.water_surface_m[b as usize]
            .total_cmp(&out.water_surface_m[a as usize])
            .then_with(|| a.cmp(&b))
    });
    let mut strahler = vec![0u8; n];
    // Per-receiver running (max donor order, count at that max).
    let mut max_in = vec![0u8; n];
    let mut max_in_count = vec![0u8; n];
    for &c in &order {
        let cu = c as usize;
        if !river[cu] {
            continue;
        }
        let s = if max_in[cu] == 0 {
            1 // source
        } else if max_in_count[cu] >= 2 {
            max_in[cu] + 1
        } else {
            max_in[cu]
        };
        strahler[cu] = s;
        let r = out.receiver[cu] as usize;
        if river[r] {
            use std::cmp::Ordering;
            match s.cmp(&max_in[r]) {
                Ordering::Greater => {
                    max_in[r] = s;
                    max_in_count[r] = 1;
                }
                Ordering::Equal => max_in_count[r] = max_in_count[r].saturating_add(1),
                Ordering::Less => {}
            }
        }
    }

    // Segments: every source (no river donors) and every junction (≥2
    // river donors) heads a downstream walk; the walk closes AT the next
    // junction (inclusive, so chains meet) or appends the water cell it
    // enters. Heads scan in ascending id — deterministic.
    let mut segments = Vec::new();
    for h in 0..n {
        if !river[h] || (donor_count[h] != 0 && donor_count[h] < 2) {
            continue;
        }
        let mut cells = vec![h as u32];
        let mut enters_water = false;
        let mut cur = h;
        loop {
            let r = out.receiver[cur] as usize;
            if river[r] {
                cells.push(r as u32);
                if donor_count[r] >= 2 {
                    break; // the junction closes this segment
                }
                cur = r;
            } else {
                // Ocean or lake: the line enters the water body. (A
                // receiver is never dry land: discharge only grows
                // downstream, so a sub-threshold land receiver is
                // impossible.)
                cells.push(r as u32);
                enters_water = true;
                break;
            }
        }
        segments.push(RiverSegment {
            order: strahler[h],
            cells,
            enters_water,
        });
    }

    RiverSet {
        segments,
        strahler,
        threshold_m3s,
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    [v[0] / len, v[1] / len, v[2] / len]
}

/// One Chaikin corner-cut on (point, width) pairs, open chain, endpoints
/// pinned; new points renormalized to the sphere, widths cut in lockstep
/// so the ribbon tapers smoothly through the cut corners.
fn chaikin_open(pts: &mut Vec<[f32; 3]>, widths: &mut Vec<f32>) {
    let n = pts.len();
    if n < 3 {
        return;
    }
    let mut np = Vec::with_capacity(2 * n);
    let mut nw = Vec::with_capacity(2 * n);
    np.push(pts[0]);
    nw.push(widths[0]);
    for i in 0..n - 1 {
        let (a, b) = (pts[i], pts[i + 1]);
        let (wa, wb) = (widths[i], widths[i + 1]);
        for t in [0.25f32, 0.75] {
            np.push(normalize([
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ]));
            nw.push(wa + (wb - wa) * t);
        }
    }
    np.push(pts[n - 1]);
    nw.push(widths[n - 1]);
    *pts = np;
    *widths = nw;
}

/// Turn the network into smoothed ribbon chains: Chaikin ×2 on the sphere
/// (endpoints pinned), width per point ∝ sqrt(Q) clamped to
/// [1, WIDTH_MAX_SCALE], btype [`BTYPE_RIVER`].
pub fn to_chains(set: &RiverSet, grid: &Grid, out: &TerrainOutput) -> Vec<BoundaryChain> {
    let mut chains = Vec::with_capacity(set.segments.len());
    for seg in &set.segments {
        if seg.cells.len() < 2 {
            continue;
        }
        let mut pts: Vec<[f32; 3]> = seg
            .cells
            .iter()
            .map(|&c| grid.positions[c as usize])
            .collect();
        let mut widths: Vec<f32> = seg
            .cells
            .iter()
            .map(|&c| {
                (out.discharge_m3s[c as usize] / set.threshold_m3s)
                    .max(0.0)
                    .sqrt()
                    .clamp(1.0, WIDTH_MAX_SCALE)
            })
            .collect();
        // The appended water cell draws at the last land width: the river's
        // mouth keeps its size instead of jumping to the coast cell's
        // (often huge) collected discharge.
        if seg.enters_water {
            let k = widths.len();
            widths[k - 1] = widths[k - 2];
        }
        for _ in 0..2 {
            chaikin_open(&mut pts, &mut widths);
        }
        chains.push(BoundaryChain {
            btype: BTYPE_RIVER,
            pts,
            widths,
            closed: false,
        });
    }
    chains
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use worldmaker_core::hash::seed_from_text;
    use worldmaker_sim::tectonics::{self, TectonicsParams};
    use worldmaker_sim::terrain;
    use worldmaker_sim::{StageContext, WorldState};

    fn suite_world(seed: u64) -> (Arc<Grid>, TerrainOutput) {
        let grid = Arc::new(Grid::build(6));
        let world = WorldState::new(grid.clone());
        let hist = tectonics::run_history(
            &StageContext::new(seed),
            &world,
            &TectonicsParams::default(),
            None,
        )
        .unwrap();
        let kf = hist.keyframes.last().unwrap();
        let out = terrain::run_terrain(&grid, kf, seed, 30.0);
        (grid, out)
    }

    /// WO-0009 S3 step 6 invariants, per suite world:
    /// - no-uphill at the polyline level: every hop of every segment
    ///   descends the epsilon-filled water surface;
    /// - a river never crosses a lake without entering it: no interior
    ///   point of a chain is a lake or ocean cell (water cells appear only
    ///   as a final entering point);
    /// - continuity: consecutive polyline cells are grid neighbors;
    /// - Strahler sanity: junctions of two equal orders step up by one,
    ///   and orders never decrease downstream.
    fn assert_river_invariants(seed: u64, label: &str) {
        let (grid, out) = suite_world(seed);
        let set = extract(&out, RIVER_MIN_DISCHARGE_M3S);
        assert!(
            !set.segments.is_empty(),
            "{label}: no rivers at all above {} m³/s",
            RIVER_MIN_DISCHARGE_M3S
        );
        for (si, seg) in set.segments.iter().enumerate() {
            assert!(seg.cells.len() >= 2, "{label}: segment {si} degenerate");
            assert!(seg.order >= 1, "{label}: segment {si} has order 0");
            for w in seg.cells.windows(2) {
                let (a, b) = (w[0] as usize, w[1] as usize);
                // Continuity: each hop is a grid edge.
                assert!(
                    grid.neighbors_of(w[0]).contains(&w[1]),
                    "{label}: segment {si} jumps {a} → {b} (not neighbors)"
                );
                // No-uphill on the filled surface.
                assert!(
                    out.water_surface_m[b] < out.water_surface_m[a],
                    "{label}: segment {si} flows uphill at {a} → {b}"
                );
            }
            // Interior points are river land cells; water appears only as
            // the final entering point.
            for (i, &c) in seg.cells.iter().enumerate() {
                let cu = c as usize;
                let water = out.elev_m[cu] <= 0.0 || out.lake_depth_m[cu] > 0.0;
                if i + 1 < seg.cells.len() {
                    assert!(
                        !water,
                        "{label}: segment {si} crosses water cell {cu} without entering"
                    );
                } else if seg.enters_water {
                    assert!(water, "{label}: segment {si} claims water entry on land");
                }
            }
            // Downstream monotone Strahler along the land part.
            let land = &seg.cells[..seg.cells.len() - usize::from(seg.enters_water)];
            for w in land.windows(2) {
                assert!(
                    set.strahler[w[1] as usize] >= set.strahler[w[0] as usize],
                    "{label}: Strahler order drops downstream at {} → {}",
                    w[0],
                    w[1]
                );
            }
        }
        // Strahler junction rule, network-wide: a cell fed by ≥2 donors of
        // its max order m has order m+1; otherwise it keeps m.
        let n = out.elev_m.len();
        for c in 0..n {
            if set.strahler[c] == 0 {
                continue;
            }
            let donors: Vec<usize> = grid
                .neighbors_of(c as u32)
                .iter()
                .map(|&nb| nb as usize)
                .filter(|&nb| set.strahler[nb] > 0 && out.receiver[nb] == c as u32)
                .collect();
            if donors.is_empty() {
                assert_eq!(set.strahler[c], 1, "source cell {c} not order 1");
            } else {
                let m = donors.iter().map(|&d| set.strahler[d]).max().unwrap();
                let at_max = donors.iter().filter(|&&d| set.strahler[d] == m).count();
                let expect = if at_max >= 2 { m + 1 } else { m };
                assert_eq!(
                    set.strahler[c], expect,
                    "Strahler rule broken at cell {c} (max {m}, count {at_max})"
                );
            }
        }
        // Smoothed chains exist and stay on the unit sphere with sane widths.
        let chains = to_chains(&set, &grid, &out);
        assert_eq!(chains.len(), set.segments.len());
        for ch in &chains {
            assert_eq!(ch.btype, BTYPE_RIVER);
            assert!(!ch.closed);
            assert_eq!(ch.pts.len(), ch.widths.len());
            for p in &ch.pts {
                let d = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                assert!((d - 1.0).abs() < 1e-3, "chain point off the sphere");
            }
            for &w in &ch.widths {
                assert!(
                    (1.0..=WIDTH_MAX_SCALE).contains(&w),
                    "width {w} out of range"
                );
            }
        }
    }

    #[test]
    fn river_invariants_seed42() {
        assert_river_invariants(42, "seed 42");
    }

    #[test]
    fn river_invariants_seed_cyrus() {
        assert_river_invariants(seed_from_text("cyrus"), "seed cyrus");
    }

    const EARTH_R_KM: f64 = 6371.0;

    fn geodesic_km(a: [f32; 3], b: [f32; 3]) -> f64 {
        let dot =
            (a[0] as f64 * b[0] as f64 + a[1] as f64 * b[1] as f64 + a[2] as f64 * b[2] as f64)
                .clamp(-1.0, 1.0);
        dot.acos() * EARTH_R_KM
    }

    /// 2-means split of the elevation distribution plus Ashman's D
    /// (D > 2 = clearly bimodal) — the WO-0002 hypsometry machinery from
    /// harness.rs, applied to the POST-EROSION surface (WO-0009 S3 gate).
    fn hypsometry_2means(elev: &[f32]) -> (f64, f64, f64) {
        let (mut c_low, mut c_high) = (-4000.0f64, 400.0f64);
        for _ in 0..40 {
            let (mut s0, mut n0, mut s1, mut n1) = (0.0f64, 0u64, 0.0f64, 0u64);
            for &e in elev {
                let e = e as f64;
                if (e - c_low).abs() <= (e - c_high).abs() {
                    s0 += e;
                    n0 += 1;
                } else {
                    s1 += e;
                    n1 += 1;
                }
            }
            if n0 > 0 {
                c_low = s0 / n0 as f64;
            }
            if n1 > 0 {
                c_high = s1 / n1 as f64;
            }
        }
        let (mut v0, mut n0, mut v1, mut n1) = (0.0f64, 0u64, 0.0f64, 0u64);
        for &e in elev {
            let e = e as f64;
            if (e - c_low).abs() <= (e - c_high).abs() {
                v0 += (e - c_low) * (e - c_low);
                n0 += 1;
            } else {
                v1 += (e - c_high) * (e - c_high);
                n1 += 1;
            }
        }
        let var0 = v0 / n0.max(1) as f64;
        let var1 = v1 / n1.max(1) as f64;
        let d = (2.0f64).sqrt() * (c_high - c_low).abs() / (var0 + var1).max(1e-9).sqrt();
        (c_low, c_high, d)
    }

    /// The WO-0009 S3 hypsometry gate: erosion must not have destroyed the
    /// continent/ocean bimodality.
    #[test]
    fn post_erosion_hypsometry_still_bimodal_seed42() {
        let (_grid, out) = suite_world(42);
        let (c_low, c_high, d) = hypsometry_2means(&out.elev_m);
        assert!(
            d > 2.0 && c_low < -2500.0 && c_high.abs() < 1500.0,
            "post-erosion hypsometry no longer bimodal: D {d:.2}, modes {c_low:.0} / {c_high:.0} m"
        );
    }

    /// All the WO-0009 S3 step-5 statistics for one world.
    fn river_stats(grid: &Grid, out: &TerrainOutput, set: &RiverSet) -> serde_json::Value {
        let n = out.elev_m.len();
        let cell_area_km2 = 4.0 * std::f64::consts::PI * EARTH_R_KM * EARTH_R_KM / n as f64;
        let land: Vec<usize> = (0..n).filter(|&c| out.elev_m[c] > 0.0).collect();
        let land_area_km2 = land.len() as f64 * cell_area_km2;

        // Drawn-network drainage density (km channel / km² land), plus the
        // full flow-tree density (every land cell drains): both recorded
        // against benchmarks Table 6.4 — Earth's densities count streams
        // far below any grid cell, so ours sit under the coarse band by
        // construction; the numbers are records, not gates.
        let mut drawn_km = 0.0f64;
        for seg in &set.segments {
            let land_cells = &seg.cells[..seg.cells.len() - usize::from(seg.enters_water)];
            for w in land_cells.windows(2) {
                drawn_km +=
                    geodesic_km(grid.positions[w[0] as usize], grid.positions[w[1] as usize]);
            }
        }
        let mut channel_km = 0.0f64;
        for &c in &land {
            let r = out.receiver[c];
            if r != RECV_NONE {
                channel_km += geodesic_km(grid.positions[c], grid.positions[r as usize]);
            }
        }

        // Longest continuous drawn river: from every source, walk the
        // drawn network downstream, summing geodesic hop lengths.
        // Receivers strictly descend the filled surface, so the walk
        // terminates; continuity per hop is asserted by the invariants
        // test.
        let is_river = |c: usize| set.strahler[c] > 0;
        let mut longest_km = 0.0f64;
        for &c in &land {
            if !is_river(c) {
                continue;
            }
            let has_river_donor = grid
                .neighbors_of(c as u32)
                .iter()
                .any(|&nb| is_river(nb as usize) && out.receiver[nb as usize] == c as u32);
            if has_river_donor {
                continue; // not a source
            }
            let mut len = 0.0f64;
            let mut cur = c;
            loop {
                let r = out.receiver[cur];
                if r == RECV_NONE {
                    break;
                }
                len += geodesic_km(grid.positions[cur], grid.positions[r as usize]);
                if !is_river(r as usize) {
                    break; // entered ocean or a lake
                }
                cur = r as usize;
            }
            longest_km = longest_km.max(len);
        }

        // Basin census: propagate each land (and lake-bed — spill-level
        // lakes drain through) cell's terminal sink down the flow tree
        // (reverse topological order resolves receivers before donors),
        // then count distinct terminals reached by land cells. A terminal
        // is the first ocean cell (the basin's coastal outlet) or a
        // RECV_NONE pit (truly endorheic).
        let mut order: Vec<u32> = (0..n as u32).collect();
        order.sort_unstable_by(|&a, &b| {
            out.water_surface_m[b as usize]
                .total_cmp(&out.water_surface_m[a as usize])
                .then_with(|| a.cmp(&b))
        });
        let mut root: Vec<u32> = (0..n as u32).collect();
        for &c in order.iter().rev() {
            let cu = c as usize;
            let r = out.receiver[cu];
            if out.elev_m[cu] > 0.0 && r != RECV_NONE {
                root[cu] = root[r as usize];
            }
        }
        let mut terminals: Vec<u32> = land.iter().map(|&c| root[c]).collect();
        terminals.sort_unstable();
        terminals.dedup();
        let basin_count = terminals.len();
        let mut basin_cells: std::collections::BTreeMap<u32, usize> = Default::default();
        for &c in &land {
            *basin_cells.entry(root[c]).or_default() += 1;
        }
        let big_basins = basin_cells.values().filter(|&&v| v >= 100).count();
        // Endorheic: the terminal never reached the ocean.
        let endorheic = terminals
            .iter()
            .filter(|&&t| out.elev_m[t as usize] > 0.0)
            .count();

        // Distinct lakes: connected components of lake cells.
        let mut lake_seen = vec![false; n];
        let mut lake_count = 0usize;
        for c in 0..n {
            if out.lake_depth_m[c] <= 0.0 || lake_seen[c] {
                continue;
            }
            lake_count += 1;
            let mut stack = vec![c as u32];
            lake_seen[c] = true;
            while let Some(top) = stack.pop() {
                for &nb in grid.neighbors_of(top) {
                    let nbu = nb as usize;
                    if out.lake_depth_m[nbu] > 0.0 && !lake_seen[nbu] {
                        lake_seen[nbu] = true;
                        stack.push(nb);
                    }
                }
            }
        }

        let (c_low, c_high, ashman_d) = hypsometry_2means(&out.elev_m);
        let max_order = set.segments.iter().map(|s| s.order).max().unwrap_or(0);
        let lake_cells = (0..n).filter(|&c| out.lake_depth_m[c] > 0.0).count();

        serde_json::json!({
            "river_threshold_m3s": set.threshold_m3s,
            "river_cells": set.strahler.iter().filter(|&&s| s > 0).count(),
            "river_segments": set.segments.len(),
            "max_strahler_order": max_order,
            "drawn_network_km": drawn_km,
            "drawn_drainage_density_km_per_km2": drawn_km / land_area_km2.max(1.0),
            "channel_density_km_per_km2": channel_km / land_area_km2.max(1.0),
            "benchmark_table_6_4_bands_km_per_km2": {
                "coarse": [0.0, 5.0], "medium": [5.0, 14.0],
                "note": "recorded, not gated: Table 6.4 counts sub-cell streams no ≥100 km grid can resolve",
            },
            "longest_river_km": longest_km,
            "basin_count": basin_count,
            "basins_min_100_cells": big_basins,
            "endorheic_basins": endorheic,
            "lake_count": lake_count,
            "lake_cells": lake_cells,
            "land_area_km2": land_area_km2,
            "hypsometry": {
                "mode_low_m": c_low, "mode_high_m": c_high,
                "ashman_d": ashman_d, "bimodal": ashman_d > 2.0,
            },
        })
    }

    /// Dev probe (rule 3: chat numbers don't count): the step-5 river and
    /// hypsometry statistics to docs/results/rivers-wo0009-s3-<machine>.json.
    /// Run with:
    ///   cargo test -p worldmaker-app --release rivers_probe -- --ignored --nocapture
    #[test]
    #[ignore = "dev probe: writes docs/results/rivers-wo0009-s3-<machine>.json"]
    fn rivers_probe() {
        let mut metrics = serde_json::Map::new();
        metrics.insert(
            "config".into(),
            serde_json::json!({
                "level": 6, "span_my": 500.0, "morpho_my": 30.0,
                "keyframe": "last",
                "river_threshold_m3s": RIVER_MIN_DISCHARGE_M3S,
            }),
        );
        for (label, seed) in [("seed_42", 42u64), ("seed_cyrus", seed_from_text("cyrus"))] {
            let (grid, out) = suite_world(seed);
            let set = extract(&out, RIVER_MIN_DISCHARGE_M3S);
            metrics.insert(label.into(), river_stats(&grid, &out, &set));
        }
        let metrics = serde_json::Value::Object(metrics);
        eprintln!("{}", serde_json::to_string_pretty(&metrics).unwrap());
        let file =
            worldmaker_io::ResultsFile::new(&worldmaker_io::results::today_utc_iso(), metrics);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../docs/results/rivers-wo0009-s3-{}.json",
            worldmaker_io::results::machine_name()
        ));
        file.write(&path).unwrap();
        eprintln!("wrote {}", path.display());
    }
}
