//! Committed plate-map metrics for WO-0003 Fix 2: plate area CV and boundary
//! sinuosity (design of record: docs/plan/feel-pass-design/d2-fix2-design.md
//! §3, as amended by findings F1, F4, F6, F11).
//!
//! This module is public forever: the CI gate test and the app acceptance
//! harness both call it, and the committed gate constants are defined in
//! terms of these exact implementations. Everything here is serial and
//! id-ordered; f64 accumulation in fixed traversal order; every distance is
//! `worldmaker_core::dmath::arc_len3` — one implementation for numerator,
//! denominator, incumbent measurement and gate alike.

use worldmaker_core::dmath::{arc_len3, normalize3};
use worldmaker_core::Grid;

use super::keyframe::{Keyframe, TectonicsHistory};

/// FINAL feel gates on the t=0 plate map (WO-0003 Fix 2; judge record §3,
/// re-confirmed unchanged by the re-judging addendum §A4). Evaluated on the
/// pinned gate triple — L7 seed 42, L6 seed 7, L6 seed cyrus — by the CI
/// gate test (tests/plategen_gate_tests.rs, which carries the full
/// derivation and the verbatim incumbent/winner measurements) and echoed +
/// re-evaluated into the results JSON by the acceptance harness. One
/// definition, two enforcement points.
pub const GATE_CV: f64 = 0.50;
/// See `GATE_CV`.
pub const GATE_SINUOSITY: f64 = 1.18;

/// Coefficient of variation of plate areas, on cell counts (pinned; the
/// pentagon area deficit is noise at this precision).
///
/// Panics if any id is out of range or any plate is empty — generators must
/// emit dense non-empty ids `0..plate_count`.
pub fn plate_area_cv(plate_id: &[u32], plate_count: u32) -> f64 {
    assert!(plate_count > 0, "plate_count must be positive");
    let mut counts = vec![0u64; plate_count as usize];
    for &p in plate_id {
        assert!(
            p < plate_count,
            "plate id {p} out of range 0..{plate_count}"
        );
        counts[p as usize] += 1;
    }
    let n = plate_id.len() as f64;
    let p = plate_count as f64;
    let mean = n / p;
    let mut sum_sq = 0.0f64;
    // Plate-id order, serial, f64 — bit-stable.
    for (i, &c) in counts.iter().enumerate() {
        assert!(c > 0, "plate {i} is empty");
        let d = c as f64 - mean;
        sum_sq += d * d;
    }
    (sum_sq / p).sqrt() / mean
}

/// Everything `boundary_sinuosity` measures. `weighted_mean` is the gated
/// number; the rest are diagnostics recorded in results JSON.
pub struct SinuosityReport {
    /// (Σ open-segment polyline + Σ loop polyline) ÷
    /// (Σ open-segment junction great-circle + Σ loop π·pseudo-diameter).
    pub weighted_mean: f64,
    pub open_segment_count: u32,
    /// Junction-free closed loops plus zero-length lassos (both endpoints on
    /// the same junction triangle) — both score len/(π·two-sweep diameter).
    pub loop_count: u32,
    /// Grid triangles whose 3 corner cells carry 3 distinct plate ids.
    pub junction_count: u32,
    /// Numerator of `weighted_mean` (radians).
    pub total_polyline_rad: f64,
    /// Denominator of `weighted_mean` (radians).
    pub total_baseline_rad: f64,
}

/// Boundary sinuosity of a plate map, per the pinned definition: the plate
/// interface is decomposed into junction-terminated open segments and
/// junction-free closed loops; each open segment's polyline is the
/// lower-plate-side cell-center chain with the two junction points prepended/
/// appended; sinuosity = polyline length ÷ junction great-circle distance,
/// averaged weighted by that great-circle distance. Loops (and lassos, whose
/// junction distance is exactly zero) score len/(π·two-sweep pseudo-diameter)
/// with weight π·diameter.
///
/// Panics if the map has no interface edge (F4: a loud panic, never NaN).
pub fn boundary_sinuosity(grid: &Grid, plate_id: &[u32]) -> SinuosityReport {
    let n = grid.cell_count() as usize;
    assert_eq!(plate_id.len(), n, "plate_id length must match the grid");

    // ---- interface edges, explicitly sorted by (a, b) ----
    // CSR rings are CCW-ordered and only rotated to start at the lowest
    // neighbor id, NOT ascending (F1) — so the enumeration below is sorted in
    // `a` but not in `b`; the explicit sort is mandatory for the binary-search
    // lookups to be correct.
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for a in 0..n as u32 {
        for &b in grid.neighbors_of(a) {
            if b > a && plate_id[a as usize] != plate_id[b as usize] {
                edges.push((a, b));
            }
        }
    }
    edges.sort_unstable();
    assert!(
        !edges.is_empty(),
        "sinuosity undefined: no interface edges (single-plate map?)"
    );
    let mut visited = vec![false; edges.len()];

    let edge_index = |a: u32, b: u32| -> usize {
        // Walk pairs are normalized to (min, max) by every caller (F1).
        debug_assert!(a < b);
        edges
            .binary_search(&(a, b))
            .expect("walk produced a pair that is not an interface edge")
    };

    // The two cells flanking edge (a, b): the third corners of the two mesh
    // triangles containing the edge, read from a's CCW ring (a = lower id).
    let flankers = |a: u32, b: u32| -> (u32, u32) {
        let ring = grid.neighbors_of(a);
        let deg = ring.len();
        let i = ring
            .iter()
            .position(|&x| x == b)
            .expect("edge endpoints must be neighbors");
        (ring[(i + deg - 1) % deg], ring[(i + 1) % deg])
    };

    // Junction point: normalize3 of the three corner positions summed in
    // ascending cell-id order (fixed order ⇒ bit-identical for the same
    // physical junction, whichever walk reaches it).
    let junction_point = |mut t: [u32; 3]| -> [f32; 3] {
        t.sort_unstable();
        let p0 = grid.positions[t[0] as usize];
        let p1 = grid.positions[t[1] as usize];
        let p2 = grid.positions[t[2] as usize];
        normalize3([
            p0[0] + p1[0] + p2[0],
            p0[1] + p1[1] + p2[1],
            p0[2] + p1[2] + p2[2],
        ])
    };

    let sorted3 = |mut t: [u32; 3]| -> [u32; 3] {
        t.sort_unstable();
        t
    };

    // Two-sweep pseudo-diameter of a loop's polyline vertices: v0 = lowest
    // cell id; v1 = farthest from v0 (ties to the lower cell id); v2 =
    // farthest from v1 (same tie rule); diameter = arc(v1, v2).
    let two_sweep_diam = |chain: &[u32]| -> f64 {
        let v0 = *chain.iter().min().expect("chain is non-empty");
        let farthest = |from: u32| -> u32 {
            let fp = grid.positions[from as usize];
            let mut best = chain[0];
            let mut best_d = arc_len3(fp, grid.positions[chain[0] as usize]);
            for &v in &chain[1..] {
                let d = arc_len3(fp, grid.positions[v as usize]);
                if d > best_d || (d == best_d && v < best) {
                    best_d = d;
                    best = v;
                }
            }
            best
        };
        let v1 = farthest(v0);
        let v2 = farthest(v1);
        arc_len3(grid.positions[v1 as usize], grid.positions[v2 as usize]) as f64
    };

    let mut open_segment_count = 0u32;
    let mut loop_count = 0u32;
    let mut junction_count = 0u32;
    let mut total_polyline = 0.0f64;
    let mut total_baseline = 0.0f64;

    // The side cell of an interface edge on the lower plate's side.
    let side_cell = |a: u32, b: u32, pair_lo: u32| -> u32 {
        if plate_id[a as usize] == pair_lo {
            a
        } else {
            b
        }
    };

    // Chain length along cell centers (consecutive vertices only).
    let chain_len = |chain: &[u32]| -> f64 {
        let mut len = 0.0f64;
        for w in chain.windows(2) {
            len += arc_len3(grid.positions[w[0] as usize], grid.positions[w[1] as usize]) as f64;
        }
        len
    };

    // ---- pass 1: junction-seeded walks (canonical: grid.triangles order,
    // then the triangle's 3 interface edges in ascending (a, b) order) ----
    for tri in &grid.triangles {
        let [x, y, z] = *tri;
        let (px, py, pz) = (
            plate_id[x as usize],
            plate_id[y as usize],
            plate_id[z as usize],
        );
        if px == py || py == pz || px == pz {
            continue;
        }
        junction_count += 1;
        let norm_pair = |u: u32, v: u32| if u < v { (u, v) } else { (v, u) };
        let mut tri_edges = [norm_pair(x, y), norm_pair(y, z), norm_pair(x, z)];
        tri_edges.sort_unstable();
        for &(sa, sb) in &tri_edges {
            let start_idx = edge_index(sa, sb);
            if visited[start_idx] {
                continue;
            }
            // Walk away from this triangle.
            let start_tri = sorted3([x, y, z]);
            let j1 = junction_point([x, y, z]);
            let pair_lo = plate_id[sa as usize].min(plate_id[sb as usize]);
            let mut chain: Vec<u32> = vec![side_cell(sa, sb, pair_lo)];
            visited[start_idx] = true;
            let (mut a, mut b) = (sa, sb);
            let mut x_prev = x ^ y ^ z ^ sa ^ sb; // the corner not on the edge
            loop {
                let (f1, f2) = flankers(a, b);
                let x_next = if f1 == x_prev { f2 } else { f1 };
                let (pa, pb) = (plate_id[a as usize], plate_id[b as usize]);
                let pn = plate_id[x_next as usize];
                if pn != pa && pn != pb {
                    // Terminate at junction {a, b, x_next}.
                    let end_tri = sorted3([a, b, x_next]);
                    let j2 = junction_point([a, b, x_next]);
                    let mut len = arc_len3(j1, grid.positions[chain[0] as usize]) as f64;
                    len += chain_len(&chain);
                    len += arc_len3(grid.positions[*chain.last().unwrap() as usize], j2) as f64;
                    if end_tri == start_tri {
                        // Lasso: both endpoints on the same junction triangle
                        // — its junction distance is exactly zero, so it
                        // scores as a loop: len/(π·diam), weight π·diam.
                        let diam = two_sweep_diam(&chain);
                        total_polyline += len;
                        total_baseline += std::f64::consts::PI * diam;
                        loop_count += 1;
                    } else {
                        let gc = arc_len3(j1, j2) as f64;
                        total_polyline += len;
                        total_baseline += gc;
                        open_segment_count += 1;
                    }
                    break;
                }
                // Continue: replace the same-plate endpoint with x_next.
                let (na, nb, np) = if pn == pa {
                    (x_next, b, a)
                } else {
                    (a, x_next, b)
                };
                let (na, nb) = if na < nb { (na, nb) } else { (nb, na) };
                let idx = edge_index(na, nb);
                debug_assert!(!visited[idx], "open walk revisited an edge");
                visited[idx] = true;
                let sc = side_cell(na, nb, pair_lo);
                if *chain.last().unwrap() != sc {
                    chain.push(sc);
                }
                x_prev = np;
                a = na;
                b = nb;
            }
        }
    }

    // ---- pass 2: junction-free closed loops (edge-list order) ----
    for start_idx in 0..edges.len() {
        if visited[start_idx] {
            continue;
        }
        let (sa, sb) = edges[start_idx];
        let (f1, f2) = flankers(sa, sb);
        // Initial direction: toward the flanking neighbor with the lower
        // cell id ⇒ the "previous" third cell is the higher one.
        let mut x_prev = f1.max(f2);
        let pair_lo = plate_id[sa as usize].min(plate_id[sb as usize]);
        let mut chain: Vec<u32> = vec![side_cell(sa, sb, pair_lo)];
        visited[start_idx] = true;
        let (mut a, mut b) = (sa, sb);
        loop {
            let (g1, g2) = flankers(a, b);
            let x_next = if g1 == x_prev { g2 } else { g1 };
            let (pa, pb) = (plate_id[a as usize], plate_id[b as usize]);
            let pn = plate_id[x_next as usize];
            debug_assert!(
                pn == pa || pn == pb,
                "loop pass hit a junction — junction-seeded pass missed it"
            );
            let (na, nb, np) = if pn == pa {
                (x_next, b, a)
            } else {
                (a, x_next, b)
            };
            let (na, nb) = if na < nb { (na, nb) } else { (nb, na) };
            if (na, nb) == (sa, sb) {
                break; // closed
            }
            let idx = edge_index(na, nb);
            debug_assert!(!visited[idx], "loop walk revisited an edge");
            visited[idx] = true;
            let sc = side_cell(na, nb, pair_lo);
            if *chain.last().unwrap() != sc {
                chain.push(sc);
            }
            x_prev = np;
            a = na;
            b = nb;
        }
        // F6: dedup across the wrap, then add the closing hop.
        if chain.len() > 1 && chain.last() == chain.first() {
            chain.pop();
        }
        let mut len = chain_len(&chain);
        len += arc_len3(
            grid.positions[*chain.last().unwrap() as usize],
            grid.positions[chain[0] as usize],
        ) as f64;
        let diam = two_sweep_diam(&chain);
        total_polyline += len;
        total_baseline += std::f64::consts::PI * diam;
        loop_count += 1;
    }

    debug_assert!(visited.iter().all(|&v| v), "unvisited interface edges");
    assert!(
        total_baseline > 0.0,
        "sinuosity undefined: no interface edges carry weight"
    );

    SinuosityReport {
        weighted_mean: total_polyline / total_baseline,
        open_segment_count,
        loop_count,
        junction_count,
        total_polyline_rad: total_polyline,
        total_baseline_rad: total_baseline,
    }
}

// ----- WO-0003 Fix 4: plate-liveliness gates -----

/// Liveliness gate constants (WO-0003 Fix 4, WO-0003-S2 step 7). Like the
/// feel gates above these are canonical here and enforced twice: by the CI
/// gate tests (tests/liveliness_tests.rs) and by the acceptance harness.
/// Gate 7.1: no alive plate of ≥ `LIVELINESS_MIN_PLATE_CELLS` cells keeps
/// ownership overlap ≥ `LIVELINESS_OVERLAP_MAX` across any
/// `LIVELINESS_OVERLAP_WINDOW_MY` window, unless it sits in a
/// continent-continent collision younger than the suture timer.
pub const LIVELINESS_OVERLAP_WINDOW_MY: f32 = 300.0;
/// See [`LIVELINESS_OVERLAP_WINDOW_MY`].
pub const LIVELINESS_OVERLAP_MAX: f64 = 0.985;
/// See [`LIVELINESS_OVERLAP_WINDOW_MY`].
pub const LIVELINESS_MIN_PLATE_CELLS: u32 = 50;
/// Free-mover exemption for gate 7.1: a plate whose mean speed over the
/// window's keyframes stays at or above this is rotating about a
/// near-internal Euler pole or drifting uninvaded — not frozen: its crust
/// visibly moves even though its ownership footprint holds
/// (Easter-microplate style spinners; giant plates whose Euler pole sits
/// inside them). Since WO-0006 there is no speed floor and no jam creep, so
/// sustained speed cannot be faked by a clamp; a frozen plate reads ~0.0
/// and stays caught.
pub const LIVELINESS_FREE_MIN_SPEED: f32 = 0.1;
/// Gate 7.2: no alive plate holds speed < `LIVELINESS_SPEED_FLOOR` deg/My
/// for more than `LIVELINESS_SLOW_MAX_MY` contiguous.
pub const LIVELINESS_SPEED_FLOOR: f32 = 0.05;
/// See [`LIVELINESS_SPEED_FLOOR`].
pub const LIVELINESS_SLOW_MAX_MY: f32 = 200.0;

/// Result of [`liveliness`]: human-readable violation lines, empty = pass.
pub struct LivelinessReport {
    /// Gate 7.1 violations (one line per plate × window).
    pub overlap_violations: Vec<String>,
    /// Gate 7.2 violations (one line per plate, at first breach).
    pub speed_violations: Vec<String>,
}

impl LivelinessReport {
    pub fn pass(&self) -> bool {
        self.overlap_violations.is_empty() && self.speed_violations.is_empty()
    }
}

/// Cells owned by `plate` in `a` still owned by it in `b`, over the count
/// in `a` (the WO-0003-S2 ownership-overlap definition; 0 start cells → 0).
pub fn ownership_overlap(a: &Keyframe, b: &Keyframe, plate: u16) -> (f64, u32) {
    let (mut start, mut kept) = (0u32, 0u32);
    for c in 0..a.plate_id.len() {
        if a.plate_id[c] == plate {
            start += 1;
            if b.plate_id[c] == plate {
                kept += 1;
            }
        }
    }
    let overlap = if start == 0 {
        0.0
    } else {
        kept as f64 / start as f64
    };
    (overlap, start)
}

/// Evaluate both liveliness gates over a run's keyframe history. Serial and
/// keyframe/id-ordered; used by tests and the harness, never the sim path.
///
/// The gate-7.1 exemption: a plate whose continent-continent contact is
/// younger than `SUTURE_AFTER_MY` at the window's end may hold still — a
/// fresh jam belongs to the suture rule, which resolves it. Contact age is
/// the consecutive keyframe run over which the plate has carried any pair
/// timer (conservative: ages from distinct successive partners chain).
pub fn liveliness(hist: &TectonicsHistory) -> LivelinessReport {
    let kf_my = hist.keyframe_interval_my;
    let per_window = (LIVELINESS_OVERLAP_WINDOW_MY / kf_my) as usize;
    let np = hist
        .keyframes
        .iter()
        .map(|kf| kf.plates.len())
        .max()
        .unwrap_or(0);

    // Continuous continent-continent contact age per plate per keyframe.
    let mut contact_age = vec![vec![0.0f32; np]; hist.keyframes.len()];
    for (k, kf) in hist.keyframes.iter().enumerate() {
        for t in &kf.collisions {
            for pid in [t.a as usize, t.b as usize] {
                let prev = if k == 0 { 0.0 } else { contact_age[k - 1][pid] };
                contact_age[k][pid] = contact_age[k][pid].max(prev + kf_my);
            }
        }
    }

    let mut overlap_violations = Vec::new();
    for k in 0..hist.keyframes.len().saturating_sub(per_window) {
        let a = &hist.keyframes[k];
        let b = &hist.keyframes[k + per_window];
        for p in &b.plates {
            if !p.alive || !a.plates.iter().any(|q| q.id == p.id && q.alive) {
                continue;
            }
            let (ov, start) = ownership_overlap(a, b, p.id as u16);
            if start < LIVELINESS_MIN_PLATE_CELLS || ov < LIVELINESS_OVERLAP_MAX {
                continue;
            }
            let age = contact_age[k + per_window][p.id as usize];
            if age > 0.0 && age <= super::step::SUTURE_AFTER_MY {
                continue; // fresh collision: the suture rule owns this
            }
            // Free mover: window-mean speed at or above
            // LIVELINESS_FREE_MIN_SPEED — the plate spins or drifts in
            // plain sight; only its uninvaded footprint holds. Under the
            // WO-0006 force balance there is no speed floor or jam creep,
            // so sustained speed IS motion; the old extra requirement of a
            // fresh contact guarded against floor-creep welds reading as
            // movers, a mode that no longer exists (S3 replaces these
            // gates with the §9 acceptance metrics).
            let (mut speed_sum, mut speed_n) = (0.0f64, 0u32);
            for kf in &hist.keyframes[k..=k + per_window] {
                if let Some(q) = kf.plates.iter().find(|q| q.id == p.id) {
                    speed_sum += q.speed_deg_my as f64;
                    speed_n += 1;
                }
            }
            let free_mover =
                speed_sum / speed_n.max(1) as f64 >= LIVELINESS_FREE_MIN_SPEED as f64;
            if free_mover {
                continue;
            }
            overlap_violations.push(format!(
                "plate {} overlap {:.3} over {}..{} My ({} cells, speed {:.2} deg/My, contact age {} My)",
                p.id, ov, a.t_my, b.t_my, start, p.speed_deg_my, age
            ));
        }
    }

    let mut speed_violations = Vec::new();
    let mut slow_my = vec![0.0f32; np];
    let mut flagged = vec![false; np];
    for kf in &hist.keyframes {
        for p in &kf.plates {
            let pid = p.id as usize;
            if !p.alive {
                slow_my[pid] = 0.0;
            } else if p.speed_deg_my < LIVELINESS_SPEED_FLOOR {
                slow_my[pid] += kf_my;
                if slow_my[pid] > LIVELINESS_SLOW_MAX_MY && !flagged[pid] {
                    flagged[pid] = true;
                    speed_violations.push(format!(
                        "plate {} below {} deg/My for {} My ending {} My (speed {:.3})",
                        pid, LIVELINESS_SPEED_FLOOR, slow_my[pid], kf.t_my, p.speed_deg_my
                    ));
                }
            } else {
                slow_my[pid] = 0.0;
            }
        }
    }

    LivelinessReport {
        overlap_violations,
        speed_violations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use worldmaker_core::dmath::dot3;

    /// Hemisphere split at L4: two plates, one junction-free closed loop.
    /// Any L4 map has rings whose CCW order puts a larger id before a smaller
    /// one, so this also regression-pins the F1 explicit edge sort.
    #[test]
    fn hemisphere_split_is_one_loop_with_constructed_cv() {
        let grid = Grid::build(4);
        let n = grid.cell_count() as usize;
        let plate_id: Vec<u32> = (0..n)
            .map(|c| u32::from(grid.positions[c][2] <= 0.0))
            .collect();

        // CV against counts the test itself constructed (F13: a z>0 split is
        // NOT exactly equal halves — assert the constructed value).
        let n0 = plate_id.iter().filter(|&&p| p == 0).count() as f64;
        let n1 = plate_id.iter().filter(|&&p| p == 1).count() as f64;
        assert!(n0 > 0.0 && n1 > 0.0);
        let mean = (n0 + n1) / 2.0;
        let expected_cv =
            (((n0 - mean) * (n0 - mean) + (n1 - mean) * (n1 - mean)) / 2.0).sqrt() / mean;
        let cv = plate_area_cv(&plate_id, 2);
        assert!(
            (cv - expected_cv).abs() < 1e-12,
            "cv {cv} != constructed {expected_cv}"
        );

        // A bisected sphere has ONE closed-loop boundary and zero junctions.
        let rep = boundary_sinuosity(&grid, &plate_id);
        assert_eq!(rep.junction_count, 0);
        assert_eq!(rep.open_segment_count, 0);
        assert_eq!(rep.loop_count, 1);
        // The equatorial loop: length ≈ 2π (hex zigzag inflates a little);
        // two-sweep diameter ≈ π (near-antipodal ring vertices), so the loop
        // rule scores ≈ 2π/π² ≈ 0.64 here — the ≈1 calibration is for small
        // caps, not hemispheres. Pin the pieces, not just the ratio.
        assert!(
            rep.total_polyline_rad > 2.0 * std::f64::consts::PI * 0.99
                && rep.total_polyline_rad < 2.0 * std::f64::consts::PI * 1.30,
            "equator loop length implausible: {}",
            rep.total_polyline_rad
        );
        let diam = rep.total_baseline_rad / std::f64::consts::PI;
        assert!(
            diam > 2.9 && diam <= std::f64::consts::PI,
            "two-sweep diameter implausible: {diam}"
        );
        let expected = rep.total_polyline_rad / rep.total_baseline_rad;
        assert!((rep.weighted_mean - expected).abs() < 1e-15);
    }

    /// Three-plate Voronoi wedge at L4: three open segments meeting at two
    /// triple junctions; near-great-circle boundaries measure ≈ 1 within the
    /// hex-zigzag allowance.
    #[test]
    fn three_wedges_pin_junctions_and_near_unit_sinuosity() {
        let grid = Grid::build(4);
        let n = grid.cell_count() as usize;
        // Three irregular seed directions (no symmetry degeneracies): the
        // Voronoi boundaries are great-circle arcs meeting at 2 antipodal
        // triple points.
        let dirs = [
            normalize3([1.0, 0.05, 0.15]),
            normalize3([-0.45, 0.90, 0.12]),
            normalize3([-0.40, -0.88, 0.20]),
        ];
        let plate_id: Vec<u32> = (0..n)
            .map(|c| {
                let p = grid.positions[c];
                let mut best = 0u32;
                let mut best_d = -2.0f32;
                for (k, d) in dirs.iter().enumerate() {
                    let s = dot3(p, *d);
                    if s > best_d {
                        best_d = s;
                        best = k as u32;
                    }
                }
                best
            })
            .collect();
        let rep = boundary_sinuosity(&grid, &plate_id);
        assert_eq!(rep.junction_count, 2, "expected exactly two triple points");
        assert_eq!(rep.open_segment_count, 3);
        assert_eq!(rep.loop_count, 0);
        assert!(
            rep.weighted_mean >= 0.999 && rep.weighted_mean < 1.20,
            "near-great-circle boundaries should measure ~1 (hex zigzag \
             allowance): {}",
            rep.weighted_mean
        );
    }

    /// F4: a boundary-free map panics loudly instead of returning NaN.
    #[test]
    #[should_panic(expected = "sinuosity undefined")]
    fn single_plate_map_panics() {
        let grid = Grid::build(3);
        let plate_id = vec![0u32; grid.cell_count() as usize];
        let _ = boundary_sinuosity(&grid, &plate_id);
    }
}
