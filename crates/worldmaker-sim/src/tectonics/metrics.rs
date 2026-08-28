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

use std::collections::VecDeque;

use worldmaker_core::dmath::{arc_len3, normalize3};
use worldmaker_core::Grid;

use super::keyframe::{Keyframe, TectonicsHistory};
use super::step::SimState;

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

// ----- WO-0011 plate-shape series (lifted from the shape probe) -----
// The three series the shape probe prints, as reusable functions: the probe
// calls them (behavior unchanged from its inline originals), and WO-0011 S3
// arms gates on them. Serial, cell-id order, f64 accumulation.

/// Mean over alive plates (with at least one cell) of boundary_cells /
/// area — the probe's "compact" series. A compact cap is low; a shredded
/// comb is high. `alive` is indexed by plate id.
pub fn mean_boundary_per_area(grid: &Grid, plate_id: &[u32], alive: &[bool]) -> f64 {
    let mut boundary = vec![0u32; alive.len()];
    let mut area = vec![0u32; alive.len()];
    for (c, &p) in plate_id.iter().enumerate() {
        area[p as usize] += 1;
        let nbs = grid.neighbors_of(c as u32);
        let same = nbs.iter().filter(|&&nb| plate_id[nb as usize] == p).count();
        if same < nbs.len() {
            boundary[p as usize] += 1;
        }
    }
    let mut sum = 0.0f64;
    let mut plates = 0usize;
    for p in 0..alive.len() {
        if alive[p] && area[p] > 0 {
            sum += boundary[p] as f64 / area[p] as f64;
            plates += 1;
        }
    }
    sum / plates.max(1) as f64
}

/// Fraction of all owned cells that are "fingers": at most 2 same-plate
/// neighbours (thin strips and necks) — the probe's "finger" series.
pub fn finger_fraction(grid: &Grid, plate_id: &[u32]) -> f64 {
    let mut fingers = 0u64;
    for (c, &p) in plate_id.iter().enumerate() {
        let same = grid
            .neighbors_of(c as u32)
            .iter()
            .filter(|&&nb| plate_id[nb as usize] == p)
            .count();
        if same <= 2 {
            fingers += 1;
        }
    }
    fingers as f64 / plate_id.len() as f64
}

/// The biggest plate's share of the sphere (cell-count fraction) — the
/// probe's "largest" series, the welding indicator.
pub fn largest_plate_share(plate_id: &[u32], plate_count: usize) -> f64 {
    let mut area = vec![0u64; plate_count];
    for &p in plate_id {
        area[p as usize] += 1;
    }
    *area.iter().max().unwrap_or(&0) as f64 / plate_id.len() as f64
}

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
/// and stays caught. WO-0006 S3 lowered this below the slowest transient
/// physical episode the calibration probes measured (a small plate
/// spinning about a near-internal pole at ~0.02 deg/My while its slab
/// balance rebuilt, self-resolving within ~300 My): under the force
/// balance a genuinely frozen plate reads 0.00 exactly, transient stalls
/// read 0.01+, and §9 metric 8 (armed in plate_physics_gates.rs)
/// independently polices sustained sub-0.05 speeds outside collisions.
pub const LIVELINESS_FREE_MIN_SPEED: f32 = 0.02;
/// §9 metric 8 (WO-0006 S3, replacing the old gate 7.2): no alive plate
/// holds speed < `LIVELINESS_SPEED_FLOOR` deg/My for more than
/// `LIVELINESS_SLOW_MAX_MY` outside a continent-continent collision.
/// "In a §3-qualifying collision" is read as: the plate carries a §3
/// pair timer (cc contact) — the calibration probe measured every slow
/// span as a contact stall, which §1 endorses (a long strong contact can
/// stall a plate) and §7 requires (the plates stop, staying rigid), so
/// the exemption covers the contact itself, not only the locked phase.
/// Exempt samples pause the slow clock without resetting it, so a plate
/// that idles slow with NO contact is still caught.
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
            // Collision exemption (WO-0006 S3, same reading as §9 metric
            // 8): a plate in continent-continent contact may hold still —
            // the force balance stalls it (§1) and the plates stay rigid
            // (§7); the old "fresh collision only" cap guarded against the
            // jam-creep floor faking welds, a mode that no longer exists.
            // The contact must cover at least half the window's keyframes
            // (a stall has a mechanical cause for as long as the contact
            // lasts; a single flicker exempts nothing).
            let contact_frames = (k..=k + per_window)
                .filter(|&j| contact_age[j][p.id as usize] > 0.0)
                .count();
            if contact_frames * 2 >= per_window {
                continue;
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
            let free_mover = speed_sum / speed_n.max(1) as f64 >= LIVELINESS_FREE_MIN_SPEED as f64;
            if free_mover {
                continue;
            }
            overlap_violations.push(format!(
                "plate {} overlap {:.3} over {}..{} My ({} cells, speed {:.2} deg/My, \
                 contact in {}/{} window keyframes)",
                p.id,
                ov,
                a.t_my,
                b.t_my,
                start,
                p.speed_deg_my,
                contact_frames,
                per_window + 1
            ));
        }
    }

    // §9 metric 8 (WO-0006 S3): the slow clock pauses — without resetting —
    // while the plate sits in a continent-continent collision (it carries a
    // §3 pair timer), and the violation is > 200 My of slow time outside
    // such collisions. Sutured plates are dead and drop out.
    let mut speed_violations = Vec::new();
    let mut slow_my = vec![0.0f32; np];
    let mut flagged = vec![false; np];
    for kf in &hist.keyframes {
        for p in &kf.plates {
            let pid = p.id as usize;
            if !p.alive {
                slow_my[pid] = 0.0;
            } else if p.speed_deg_my < LIVELINESS_SPEED_FLOOR {
                let exempt = kf.collisions.iter().any(|t| t.a == p.id || t.b == p.id);
                if exempt {
                    continue; // pause, don't reset
                }
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

// ----- WO-0006 S3: §9 acceptance metrics (plate-physics-model.md) -----
//
// Canonical here forever, like the feel and liveliness gates above: the CI
// gate test (tests/plate_physics_gates.rs) and the calibration harness both
// drive this tracker, so the committed calibration numbers and the armed
// gates are the same implementation by construction. Everything is serial
// and id-ordered; f64 accumulation in fixed traversal order.
//
// The eight §9 items are enforced as NINE gates: item 5 splits into 5a
// (zero exclaves — the §7 invariant) and 5b (the backstop cell budget),
// which fail independently and mean different things.

/// Sampling cadence (My) for the §9 tracker: the L6/L7 keyframe interval,
/// so sampled quantities match what any keyframe consumer would see.
pub const PHYS_SAMPLE_MY: f32 = 10.0;
/// deg/My × this = cm/yr at the rotation equator (π/180 × 637.1).
pub const DEG_MY_TO_CMYR: f64 = std::f64::consts::PI / 180.0 * 637.1;

/// Metric 1: alive plate count band (no clamp anywhere in the sim)...
pub const M1_ALIVE_MIN: u32 = 6;
/// See [`M1_ALIVE_MIN`].
pub const M1_ALIVE_MAX: u32 = 25;
/// ...with real variation across the run...
pub const M1_STDDEV_MIN: f64 = 1.5;
/// ...and never pinned at one value longer than this.
pub const M1_PINNED_MAX_MY: f32 = 500.0;
/// Metric 2: sutures per Gy (a handful of major welds per few hundred My;
/// band tightened to Dan's 2–6 target in WO-0008 S1 with the relic-basin
/// closure that unblocked condition 3).
pub const M2_SUTURES_PER_GY_MIN: f64 = 2.0;
/// See [`M2_SUTURES_PER_GY_MIN`].
pub const M2_SUTURES_PER_GY_MAX: f64 = 6.0;
/// WO-0008 S1 relic-basin gate: a locked collision older than this must
/// hold zero enclosed oceanic regions larger than the relic cap within
/// the suture window (closure has had time to finish its work).
pub const RELIC_LOCKED_GATE_MY: f32 = 60.0;
/// WO-0008 S1 continental-area gate: total continental cell count at the
/// end of a 2 Gy run within this fraction of its t = 0 value.
pub const CONT_AREA_TOLERANCE: f64 = 0.15;
/// WO-0008 S2 orogen-width gate: at least one collision zone per run
/// reaches a deformed width of this many cells...
pub const S2_OROGEN_WIDTH_CELLS: u32 = 3;
/// ...with crust at least this thick spanning it.
pub const S2_OROGEN_THICK_KM: f32 = 45.0;
/// WO-0008 S2 island-arc gate: a connected landmass that is entirely
/// younger than this and isolated from older continent flags as an
/// ocean-ocean arc WALL when it reaches `S2_ARC_WALL_CELLS` cells. The
/// discrete-site rework produces 1-cell islands; separately-born islands
/// drift together and dock into small islets (arcs assemble — real), and
/// advection rasterization can briefly duplicate an island into a
/// neighbor cell — but a band-length WALL (the pre-S2 failure: a
/// continuous ridge of land the moment a band matured) must never
/// appear. The strict per-boundary production property is enforced by
/// the synthetic ocean-ocean test; this runtime gate catches wall-scale
/// regressions.
pub const S2_ARC_WALL_AGE_MY: f32 = 50.0;
/// See [`S2_ARC_WALL_AGE_MY`].
pub const S2_ARC_WALL_CELLS: usize = 8;
/// Metric 3: rift-to-oceanization splits per Gy, each §5-attributed.
pub const M3_SPLITS_PER_GY_MIN: f64 = 2.0;
/// See [`M3_SPLITS_PER_GY_MIN`].
pub const M3_SPLITS_PER_GY_MAX: f64 = 8.0;
/// Metric 4: largest-plate sphere share outside supercontinent epochs.
pub const M4_LARGEST_SHARE_MAX: f64 = 0.45;
/// A supercontinent epoch: one plate holds over this fraction of the
/// world's continental crust.
pub const M4_SUPERCONTINENT_CONT_FRACTION: f64 = 1.0 / 3.0;
/// Epochs allowed per run (0–2 per 2 Gy). Sub-`M4_EPOCH_MERGE_GAP_MY`
/// dips below the 1/3 threshold do not end an epoch: the census flaps a few
/// cells around the line at sample cadence, and the metric's own dispersal
/// window (100–300 My) says an epoch is a sustained span, not a crossing.
pub const M4_EPOCHS_MAX: u32 = 2;
/// See [`M4_EPOCHS_MAX`].
pub const M4_EPOCH_MERGE_GAP_MY: f32 = 100.0;
/// ...each dispersing within this long of forming.
pub const M4_EPOCH_DISPERSE_MY: f32 = 300.0;
/// Metric 5b: backstop reassignments per 100 My window (5a is zero
/// multi-component plates at every sample, no constant needed).
pub const M5_BACKSTOP_MAX_CELLS_PER_100MY: u64 = 10;
/// Metric 6 counts *collisions*: contact zones that actually converged
/// (normal approach above the classification dead band, cm/yr) at some
/// point — a grazing or already-locked contact that never converged is not
/// a collision and builds nothing.
pub const M6_CONVERGING_CMYR: f32 = 0.4;
/// A contact zone is a zone, not a classification streak: the same pair
/// re-contacting within this gap is the same collision zone (orogens pause
/// and resume; a flicker of separation does not spawn a new zone).
pub const M6_MERGE_GAP_MY: f32 = 50.0;
/// Metric 6: continent-continent contact zones persisting this long...
pub const M6_PERSIST_MY: f32 = 20.0;
/// ...must reach this crust thickness somewhere along the zone...
pub const M6_RELIEF_THICKNESS_KM: f32 = 45.0;
/// ...in at least this fraction of cases. Re-measured at the WO-0008 S2
/// close per its step 8 (honest number minus five points): with the free
/// COLLISION_THICKEN creation gone, relief is funded entirely by
/// underthrust deposits, and the honest re-measure read 73% (seed cyrus)
/// and 39% (seed 42) — the S2 orogen-width gate (a >45 km zone at least
/// 3 cells deep) carries the wide-orogen requirement now, and this
/// fraction gates gross regression.
pub const M6_RELIEF_FRACTION_MIN: f64 = 0.34;
/// Metric 7: run mean plate speed (cm/yr, MORVEL-anchored)...
pub const M7_MEAN_CMYR_MIN: f64 = 2.0;
/// See [`M7_MEAN_CMYR_MIN`].
pub const M7_MEAN_CMYR_MAX: f64 = 6.0;
/// ...slab-attached plates at least this many times faster than slab-free
/// plates (Forsyth & Uyeda's trench-connectivity correlation). The
/// slab-free side uses settled plates (see `M7_DRIFT_SETTLE_MY`); when the
/// world offers fewer than `M7_MIN_POPULATION` settled slab-free
/// plate-samples (every long-lived plate subducts somewhere), the ratio
/// falls back to the S1 measurement: plates with above-median attached
/// slab area vs below-median (the same trench-connectivity correlation,
/// always measurable)...
pub const M7_SLAB_RATIO_MIN: f64 = 2.0;
/// See [`M7_SLAB_RATIO_MIN`].
pub const M7_MIN_POPULATION: u64 = 20;
/// ...and slab-free continental plates drifting in this band (cm/yr), with
/// no floor constant anywhere for it to come from. "Drifting" is measured
/// at equilibrium: plates slab-free for at least `M7_DRIFT_SETTLE_MY`
/// (past the TAU_MY relaxation, so inherited split/parent speed has
/// decayed) and free of continent-continent contact (a collision-stalled
/// plate is stalled, not drifting).
pub const M7_SLABFREE_CONT_CMYR_MIN: f64 = 0.3;
/// See [`M7_SLABFREE_CONT_CMYR_MIN`].
pub const M7_SLABFREE_CONT_CMYR_MAX: f64 = 2.0;
/// See [`M7_SLABFREE_CONT_CMYR_MIN`].
pub const M7_DRIFT_SETTLE_MY: f32 = 30.0;
// Metric 8 reuses LIVELINESS_SPEED_FLOOR / LIVELINESS_SLOW_MAX_MY above.

/// One continent-continent contact episode for metric 6: an unordered plate
/// pair seen in consecutive samples, with the peak crust thickness observed
/// on the cells flanking the contact while it lasted.
struct ContactEpisode {
    a: u32,
    b: u32,
    first_my: f32,
    last_my: f32,
    peak_km: f32,
    /// The pair converged (normal approach > `M6_CONVERGING_CMYR`) at some
    /// sample — only then is the episode a collision.
    converged: bool,
    open: bool,
}

/// A supercontinent epoch (metric 4): from the sample where one plate first
/// held > 1/3 of continental crust to the sample where it stopped.
struct Epoch {
    start_my: f32,
    end_my: Option<f32>,
}

/// §9 metrics accumulator. Feed it [`PhysicsTracker::sample`] every
/// `PHYS_SAMPLE_MY` (including once at the start), then
/// [`PhysicsTracker::finish`]. Reads only public [`SimState`] state, off the
/// sim path — nothing here feeds back into the dynamics.
pub struct PhysicsTracker {
    first_my: f32,
    last_my: f32,
    // Baselines of the cumulative SimState counters at tracker start.
    base_sutures: u64,
    base_splits: u64,
    base_events: usize,
    base_backstop: u64,
    // Metric 1.
    alive_counts: Vec<u32>,
    pinned_run_my: f32,
    pinned_max_my: f32,
    // Metric 4.
    max_share_outside_epochs: f64,
    epochs: Vec<Epoch>,
    /// (t, largest plate's share of continental crust) per sample —
    /// diagnostics for the supercontinent story.
    pub cont_share_series: Vec<(f32, f64)>,
    // Metric 5.
    exclave_samples: u32,
    backstop_cum: Vec<u64>,
    // Metric 6.
    episodes: Vec<ContactEpisode>,
    // Metric 7 (f64 sums in sample order, then plate-id order).
    speed_sum: f64,
    speed_n: u64,
    slab_speed_sum: f64,
    slab_n: u64,
    free_speed_sum: f64,
    free_n: u64,
    free_cont_speed_sum: f64,
    free_cont_n: u64,
    /// Consecutive slab-free time per plate id (the drift-settle clock).
    slab_free_my: Vec<f32>,
    // Median-split fallback accumulators (S1's attached-area measurement).
    above_median_speed_sum: f64,
    above_median_n: u64,
    below_median_speed_sum: f64,
    below_median_n: u64,
    /// Settled slab-free plate-samples (the primary ratio's free side).
    settled_free_speed_sum: f64,
    settled_free_n: u64,
    // Metric 8 (per plate id).
    slow_my: Vec<f32>,
    slow_flagged: Vec<bool>,
    slow_violations: Vec<String>,
    // WO-0008 S2 gates.
    /// Widest contiguous >45 km run walked inboard from a contact cell,
    /// over all samples (cells).
    max_orogen_width: u32,
    /// Samples where an isolated all-young landmass of 2+ cells existed
    /// (the ocean-ocean arc-wall violation).
    arc_wall_violations: Vec<String>,
    // WO-0008 S1 gates: relic basins inside old locked collisions, and
    // the continental-area balance. A basin flags only when it PERSISTS
    // across two consecutive samples (closure clears a late-enclosed
    // basin within a sample interval; a one-sample transient while it is
    // mid-clearing is not "closure failed").
    relic_violations: Vec<String>,
    relic_pending: Vec<(u32, u32)>,
    cont_cells_first: Option<u64>,
    cont_cells_last: u64,
    // Scratch.
    comp_seen: Vec<bool>,
}

/// Everything the §9 gates assert, with the measured values behind each
/// verdict (the calibration JSON records these verbatim).
pub struct PhysicsReport {
    pub span_my: f32,
    pub alive_min: u32,
    pub alive_max: u32,
    pub alive_stddev: f64,
    pub alive_pinned_max_my: f32,
    pub sutures_per_gy: f64,
    pub suture_bad_condition_count: u32,
    pub splits_per_gy: f64,
    pub splits_unattributed: i64,
    pub max_share_outside_epochs: f64,
    pub supercontinent_epochs: u32,
    pub longest_epoch_my: f32,
    pub open_epoch_my: f32,
    pub exclave_samples: u32,
    pub backstop_max_per_100my: u64,
    pub relief_episodes: u32,
    pub relief_reached: u32,
    pub mean_speed_cmyr: f64,
    pub slab_attached_mean_cmyr: f64,
    pub slab_free_mean_cmyr: f64,
    pub slab_free_plate_samples: u64,
    pub settled_free_mean_cmyr: f64,
    pub settled_free_plate_samples: u64,
    pub above_median_mean_cmyr: f64,
    pub below_median_mean_cmyr: f64,
    pub slab_free_cont_mean_cmyr: f64,
    pub slab_free_cont_plate_samples: u64,
    pub slow_violations: Vec<String>,
    /// (t, largest plate's continental-crust share) per sample.
    pub cont_share_series: Vec<(f32, f64)>,
    /// WO-0008 S1: samples where a locked collision older than
    /// `RELIC_LOCKED_GATE_MY` still held an enclosed oceanic region larger
    /// than the relic cap in its suture window.
    pub relic_basin_violations: Vec<String>,
    /// WO-0008 S1: continental cell count at the first and last sample.
    pub cont_cells_start: u64,
    pub cont_cells_end: u64,
    /// WO-0008 S2: widest contiguous >45 km inboard run from a contact.
    pub max_orogen_width: u32,
    /// WO-0008 S2: isolated all-young landmasses of 2+ cells (arc walls).
    pub arc_wall_violations: Vec<String>,
    /// WO-0008 S2 crust-volume ledger (quantized 0.01 km·cell units).
    pub vol_advect_q: i64,
    pub vol_closure_q: i64,
    pub vol_arc_q: i64,
    pub vol_collision_q: i64,
    pub vol_rift_q: i64,
    pub vol_spread_q: i64,
    pub vol_relax_q: i64,
    pub vol_quantize_q: i64,
    pub underthrust_removed_q: i64,
    pub underthrust_deposited_q: i64,
    pub underthrust_spilled_q: i64,
    pub underthrust_incorporated_q: i64,
}

impl PhysicsReport {
    /// The §9 verdicts as named gates (see the module note on the 8-item →
    /// 9-gate mapping), in §9 order, followed by the two WO-0008 S1 gates
    /// (relic basins, continental-area balance).
    pub fn gates(&self) -> Vec<(&'static str, bool, String)> {
        let relief_fraction = if self.relief_episodes == 0 {
            1.0
        } else {
            self.relief_reached as f64 / self.relief_episodes as f64
        };
        // Primary ratio: attached vs SETTLED slab-free; S1 median-split
        // fallback when the settled-free population is too thin to measure.
        let free_measurable = self.settled_free_plate_samples >= M7_MIN_POPULATION;
        let slab_ratio = if free_measurable {
            self.slab_attached_mean_cmyr / self.settled_free_mean_cmyr
        } else if self.below_median_mean_cmyr > 0.0 {
            self.above_median_mean_cmyr / self.below_median_mean_cmyr
        } else {
            f64::NAN
        };
        // The drift band binds only when its population is measurable; the
        // S1 record already established slab-free samples can be
        // structurally absent at L6 (every long-lived plate subducts).
        let drift_measurable = self.slab_free_cont_plate_samples >= M7_MIN_POPULATION;
        let drift_ok = !drift_measurable
            || (self.slab_free_cont_mean_cmyr >= M7_SLABFREE_CONT_CMYR_MIN
                && self.slab_free_cont_mean_cmyr <= M7_SLABFREE_CONT_CMYR_MAX);
        let cont_ratio = if self.cont_cells_start == 0 {
            1.0
        } else {
            self.cont_cells_end as f64 / self.cont_cells_start as f64
        };
        vec![
            (
                "m1_plate_count",
                self.alive_min >= M1_ALIVE_MIN
                    && self.alive_max <= M1_ALIVE_MAX
                    && self.alive_stddev >= M1_STDDEV_MIN
                    && self.alive_pinned_max_my <= M1_PINNED_MAX_MY,
                format!(
                    "alive {}..{}, stddev {:.2}, longest pin {} My",
                    self.alive_min, self.alive_max, self.alive_stddev, self.alive_pinned_max_my
                ),
            ),
            (
                "m2_suture_frequency",
                self.sutures_per_gy >= M2_SUTURES_PER_GY_MIN
                    && self.sutures_per_gy <= M2_SUTURES_PER_GY_MAX
                    && self.suture_bad_condition_count == 0,
                format!(
                    "{:.1} sutures/Gy, {} with a sub-threshold contact record",
                    self.sutures_per_gy, self.suture_bad_condition_count
                ),
            ),
            (
                "m3_split_frequency",
                self.splits_per_gy >= M3_SPLITS_PER_GY_MIN
                    && self.splits_per_gy <= M3_SPLITS_PER_GY_MAX
                    && self.splits_unattributed == 0,
                format!(
                    "{:.1} splits/Gy, {} unattributed",
                    self.splits_per_gy, self.splits_unattributed
                ),
            ),
            (
                "m4_largest_share",
                self.max_share_outside_epochs < M4_LARGEST_SHARE_MAX
                    && self.supercontinent_epochs <= M4_EPOCHS_MAX
                    && self.longest_epoch_my <= M4_EPOCH_DISPERSE_MY
                    && self.open_epoch_my <= M4_EPOCH_DISPERSE_MY,
                format!(
                    "max share outside epochs {:.1}%, {} epochs, longest {} My, open {} My",
                    self.max_share_outside_epochs * 100.0,
                    self.supercontinent_epochs,
                    self.longest_epoch_my,
                    self.open_epoch_my
                ),
            ),
            (
                "m5a_zero_exclaves",
                self.exclave_samples == 0,
                format!(
                    "{} samples with a multi-component plate",
                    self.exclave_samples
                ),
            ),
            (
                "m5b_backstop_budget",
                self.backstop_max_per_100my <= M5_BACKSTOP_MAX_CELLS_PER_100MY,
                format!(
                    "worst window {} cells / 100 My",
                    self.backstop_max_per_100my
                ),
            ),
            (
                "m6_collision_relief",
                relief_fraction >= M6_RELIEF_FRACTION_MIN,
                format!(
                    "{} of {} persistent contacts reached {} km ({:.0}%)",
                    self.relief_reached,
                    self.relief_episodes,
                    M6_RELIEF_THICKNESS_KM,
                    relief_fraction * 100.0
                ),
            ),
            (
                "m7_force_ranked_speeds",
                self.mean_speed_cmyr >= M7_MEAN_CMYR_MIN
                    && self.mean_speed_cmyr <= M7_MEAN_CMYR_MAX
                    && slab_ratio >= M7_SLAB_RATIO_MIN
                    && drift_ok,
                format!(
                    "mean {:.2} cm/yr; ratio {:.2} ({}: attached {:.2} vs {} {:.2}); \
                     settled free continental {:.2} cm/yr ({} samples{})",
                    self.mean_speed_cmyr,
                    slab_ratio,
                    if free_measurable {
                        "settled-free"
                    } else {
                        "median-split fallback"
                    },
                    self.slab_attached_mean_cmyr,
                    if free_measurable {
                        "settled free"
                    } else {
                        "below-median"
                    },
                    if free_measurable {
                        self.settled_free_mean_cmyr
                    } else {
                        self.below_median_mean_cmyr
                    },
                    self.slab_free_cont_mean_cmyr,
                    self.slab_free_cont_plate_samples,
                    if drift_measurable {
                        ""
                    } else {
                        ", unmeasurable: non-binding"
                    }
                ),
            ),
            (
                "m8_liveliness",
                self.slow_violations.is_empty(),
                if self.slow_violations.is_empty() {
                    "no plate slow > 200 My outside a qualifying collision".to_owned()
                } else {
                    self.slow_violations.join("; ")
                },
            ),
            (
                "s1_relic_basins",
                self.relic_basin_violations.is_empty(),
                if self.relic_basin_violations.is_empty() {
                    format!(
                        "no enclosed basin > {} cells in a collision locked > {} My",
                        super::step::RELIC_BASIN_KEEP_CELLS,
                        RELIC_LOCKED_GATE_MY
                    )
                } else {
                    self.relic_basin_violations.join("; ")
                },
            ),
            (
                "s2_orogen_width",
                self.max_orogen_width >= S2_OROGEN_WIDTH_CELLS,
                format!(
                    "widest >45 km inboard run: {} cells (need >= {})",
                    self.max_orogen_width, S2_OROGEN_WIDTH_CELLS
                ),
            ),
            (
                "s2_island_arcs",
                self.arc_wall_violations.is_empty(),
                if self.arc_wall_violations.is_empty() {
                    "no isolated all-young landmass wider than 1 cell".to_owned()
                } else {
                    self.arc_wall_violations.join("; ")
                },
            ),
            (
                "s2_volume_ledger",
                self.vol_collision_q
                    == self.underthrust_deposited_q + self.underthrust_incorporated_q
                    && self.underthrust_removed_q
                        == self.underthrust_deposited_q + self.underthrust_spilled_q,
                format!(
                    "collision {} vs deposited+incorporated {} (removed {} = deposited + spilled {}); \
                     advect {} closure {} arc {} rift {} spread {} relax {} quantize {}",
                    self.vol_collision_q,
                    self.underthrust_deposited_q + self.underthrust_incorporated_q,
                    self.underthrust_removed_q,
                    self.underthrust_deposited_q + self.underthrust_spilled_q,
                    self.vol_advect_q,
                    self.vol_closure_q,
                    self.vol_arc_q,
                    self.vol_rift_q,
                    self.vol_spread_q,
                    self.vol_relax_q,
                    self.vol_quantize_q
                ),
            ),
            (
                "s1_cont_area",
                (cont_ratio - 1.0).abs() <= CONT_AREA_TOLERANCE,
                format!(
                    "continental cells {} -> {} ({:+.1}% over the run)",
                    self.cont_cells_start,
                    self.cont_cells_end,
                    (cont_ratio - 1.0) * 100.0
                ),
            ),
        ]
    }

    pub fn pass(&self) -> bool {
        self.gates().iter().all(|(_, ok, _)| *ok)
    }
}

impl PhysicsTracker {
    /// Start tracking; call before the first step, then `sample` every
    /// `PHYS_SAMPLE_MY` (the caller samples the t=0 state too).
    pub fn new(sim: &SimState) -> PhysicsTracker {
        PhysicsTracker {
            first_my: sim.t_my,
            last_my: sim.t_my,
            base_sutures: sim.suture_count,
            base_splits: sim.breakup_count,
            base_events: sim.events.len(),
            base_backstop: sim.connectivity_reassigned,
            alive_counts: Vec::new(),
            pinned_run_my: 0.0,
            pinned_max_my: 0.0,
            max_share_outside_epochs: 0.0,
            epochs: Vec::new(),
            cont_share_series: Vec::new(),
            exclave_samples: 0,
            backstop_cum: Vec::new(),
            episodes: Vec::new(),
            speed_sum: 0.0,
            speed_n: 0,
            slab_speed_sum: 0.0,
            slab_n: 0,
            free_speed_sum: 0.0,
            free_n: 0,
            free_cont_speed_sum: 0.0,
            free_cont_n: 0,
            slab_free_my: Vec::new(),
            above_median_speed_sum: 0.0,
            above_median_n: 0,
            below_median_speed_sum: 0.0,
            below_median_n: 0,
            settled_free_speed_sum: 0.0,
            settled_free_n: 0,
            slow_my: Vec::new(),
            slow_flagged: Vec::new(),
            slow_violations: Vec::new(),
            max_orogen_width: 0,
            arc_wall_violations: Vec::new(),
            relic_violations: Vec::new(),
            relic_pending: Vec::new(),
            cont_cells_first: None,
            cont_cells_last: 0,
            comp_seen: vec![false; sim.grid.cell_count() as usize],
        }
    }

    pub fn sample(&mut self, sim: &SimState) {
        let dt = if self.alive_counts.is_empty() {
            0.0
        } else {
            sim.t_my - self.last_my
        };
        self.last_my = sim.t_my;
        let np = sim.plates.len();

        // Cell censuses in one id-ordered pass: cells and continental cells
        // per plate, total continental cells.
        let mut cells = vec![0u32; np];
        let mut cont_cells = vec![0u32; np];
        let mut cont_total = 0u64;
        for (c, &p) in sim.plate_id.iter().enumerate() {
            cells[p as usize] += 1;
            if sim.crust_type[c] == 1 {
                cont_cells[p as usize] += 1;
                cont_total += 1;
            }
        }

        // WO-0008 S1: continental-area balance endpoints.
        if self.cont_cells_first.is_none() {
            self.cont_cells_first = Some(cont_total);
        }
        self.cont_cells_last = cont_total;

        // WO-0008 S1 relic-basin gate: a pair locked past the gate age
        // must hold no enclosed oceanic region larger than the relic cap
        // in its suture window (closure has had time to finish). A basin
        // seen at only ONE sample is a late-enclosure transient that
        // closure is already clearing; it flags when it persists to the
        // next sample too.
        let mut pending: Vec<(u32, u32)> = Vec::new();
        for t in &sim.collisions {
            if t.locked_my > RELIC_LOCKED_GATE_MY {
                if let Some((size, frac)) = oversized_enclosed_basin(sim, t.a, t.b) {
                    if self.relic_pending.contains(&(t.a, t.b)) {
                        if self.relic_violations.len() < 20 {
                            self.relic_violations.push(format!(
                                "t={} My: pair ({},{}) locked {} My holds an enclosed \
                                 basin of {} cells ({:.0}% pair border) across two samples",
                                sim.t_my,
                                t.a,
                                t.b,
                                t.locked_my,
                                size,
                                frac * 100.0
                            ));
                        }
                    } else {
                        pending.push((t.a, t.b));
                    }
                }
            }
        }
        self.relic_pending = pending;

        // WO-0008 S2 orogen width: from every continent-continent contact
        // cell, walk inboard (away from the partner) while thickness stays
        // above the gate threshold; record the longest run. Serial, id
        // order; the walk picks the thickest qualifying neighbor not yet
        // visited on this walk.
        for c in 0..sim.plate_id.len() {
            if sim.crust_type[c] != 1 || sim.thickness[c] <= S2_OROGEN_THICK_KM {
                continue;
            }
            let p = sim.plate_id[c];
            let touches_partner = sim
                .grid
                .neighbors_of(c as u32)
                .iter()
                .any(|&nb| sim.plate_id[nb as usize] != p && sim.crust_type[nb as usize] == 1);
            if !touches_partner {
                continue;
            }
            let mut width = 1u32;
            let mut cur = c as u32;
            let mut prev = u32::MAX;
            loop {
                let mut best: Option<(u32, f32)> = Some((u32::MAX, S2_OROGEN_THICK_KM));
                for &nb in sim.grid.neighbors_of(cur) {
                    let nbu = nb as usize;
                    if nb == prev
                        || sim.plate_id[nbu] != p
                        || sim.crust_type[nbu] != 1
                        || sim.thickness[nbu] <= S2_OROGEN_THICK_KM
                        || sim
                            .grid
                            .neighbors_of(nb)
                            .iter()
                            .any(|&x| sim.plate_id[x as usize] != p)
                    {
                        continue;
                    }
                    if best.is_none_or(|(_, bt)| sim.thickness[nbu] > bt) {
                        best = Some((nb, sim.thickness[nbu]));
                    }
                }
                match best {
                    Some((nb, _)) if nb != u32::MAX && width < 16 => {
                        prev = cur;
                        cur = nb;
                        width += 1;
                    }
                    _ => break,
                }
            }
            self.max_orogen_width = self.max_orogen_width.max(width);
        }

        // WO-0008 S2 arc-wall gate: a connected landmass of 2+ continental
        // cells, ALL younger than S2_ARC_WALL_AGE_MY, with no older
        // continental neighbor, is an ocean-ocean arc wall (discrete-site
        // islands stay 1 cell; margin accretion touches old continent).
        {
            let seen = &mut self.comp_seen;
            for v in seen.iter_mut() {
                *v = false;
            }
            let mut queue: VecDeque<u32> = VecDeque::new();
            for c0 in 0..sim.plate_id.len() {
                if seen[c0] || sim.crust_type[c0] != 1 || sim.crust_age[c0] >= S2_ARC_WALL_AGE_MY {
                    continue;
                }
                let mut cells: Vec<u32> = Vec::new();
                let mut touches_old = false;
                let mut closure_filled = false;
                seen[c0] = true;
                queue.push_back(c0 as u32);
                while let Some(c) = queue.pop_front() {
                    cells.push(c);
                    // Relic-basin closure marks its conversions with an
                    // internal slab under the cell's own plate: a landmass
                    // containing them is a FILLED BASIN at a locked
                    // collision (arc-terrane amalgamation), not an arc
                    // wall.
                    if sim.slab_plate[c as usize] == sim.plate_id[c as usize] as u16 {
                        closure_filled = true;
                    }
                    for &nb in sim.grid.neighbors_of(c) {
                        let nbu = nb as usize;
                        if sim.crust_type[nbu] != 1 {
                            continue;
                        }
                        if sim.crust_age[nbu] >= S2_ARC_WALL_AGE_MY {
                            touches_old = true;
                        } else if !seen[nbu] {
                            seen[nbu] = true;
                            queue.push_back(nb);
                        }
                    }
                }
                if !touches_old
                    && !closure_filled
                    && cells.len() >= S2_ARC_WALL_CELLS
                    && self.arc_wall_violations.len() < 20
                {
                    self.arc_wall_violations.push(format!(
                        "t={} My: isolated all-young landmass of {} cells (first cell {})",
                        sim.t_my,
                        cells.len(),
                        cells[0]
                    ));
                }
            }
            for v in self.comp_seen.iter_mut() {
                *v = false;
            }
        }

        // Metric 1: alive count, band + pin runs.
        let alive: u32 = sim.plates.iter().filter(|p| p.alive).count() as u32;
        if let Some(&prev) = self.alive_counts.last() {
            if prev == alive {
                self.pinned_run_my += dt;
            } else {
                self.pinned_run_my = 0.0;
            }
            self.pinned_max_my = self.pinned_max_my.max(self.pinned_run_my);
        }
        self.alive_counts.push(alive);

        // Metric 4: largest sphere share and supercontinent state.
        let n_cells = sim.plate_id.len() as f64;
        let largest_share = cells.iter().copied().max().unwrap_or(0) as f64 / n_cells;
        let largest_cont = cont_cells.iter().copied().max().unwrap_or(0) as f64;
        let in_epoch =
            cont_total > 0 && largest_cont / cont_total as f64 > M4_SUPERCONTINENT_CONT_FRACTION;
        self.cont_share_series
            .push((sim.t_my, largest_cont / (cont_total as f64).max(1.0)));
        let epoch_open = self.epochs.last().is_some_and(|e| e.end_my.is_none());
        match (in_epoch, epoch_open) {
            (true, false) => {
                // Re-open the previous epoch when the dip was shorter than
                // the merge gap — that is threshold flapping, not dispersal.
                match self.epochs.last_mut() {
                    Some(e) if sim.t_my - e.end_my.unwrap_or(sim.t_my) < M4_EPOCH_MERGE_GAP_MY => {
                        e.end_my = None
                    }
                    _ => self.epochs.push(Epoch {
                        start_my: sim.t_my,
                        end_my: None,
                    }),
                }
            }
            (false, true) => self.epochs.last_mut().unwrap().end_my = Some(sim.t_my),
            _ => {}
        }
        if !in_epoch && largest_share > self.max_share_outside_epochs {
            self.max_share_outside_epochs = largest_share;
        }

        // Metric 5a: per-plate connected components (serial BFS, id order).
        let seen = &mut self.comp_seen;
        for s in seen.iter_mut() {
            *s = false;
        }
        let mut comps_of_plate = vec![0u32; np];
        let mut queue: VecDeque<u32> = VecDeque::new();
        for c0 in 0..sim.plate_id.len() {
            if seen[c0] {
                continue;
            }
            let p = sim.plate_id[c0];
            comps_of_plate[p as usize] += 1;
            seen[c0] = true;
            queue.push_back(c0 as u32);
            while let Some(c) = queue.pop_front() {
                for &nb in sim.grid.neighbors_of(c) {
                    let nbu = nb as usize;
                    if !seen[nbu] && sim.plate_id[nbu] == p {
                        seen[nbu] = true;
                        queue.push_back(nb);
                    }
                }
            }
        }
        if comps_of_plate.iter().any(|&k| k > 1) {
            self.exclave_samples += 1;
        }

        // Metric 5b: cumulative backstop counter per sample.
        self.backstop_cum
            .push(sim.connectivity_reassigned - self.base_backstop);

        // Metric 6: continent-continent contact pairs this sample, with the
        // peak thickness on the flanking cells and whether the pair is
        // converging (the same relative-motion math classify_boundaries
        // uses, from the public plate poles). Id-ordered scan; the pair
        // list stays tiny, linear search is fine and deterministic.
        let omega = |pid: u32| -> [f64; 3] {
            let p = &sim.plates[pid as usize];
            let w = p.speed_deg_my as f64 * std::f64::consts::PI / 180.0;
            [
                p.pole[0] as f64 * w,
                p.pole[1] as f64 * w,
                p.pole[2] as f64 * w,
            ]
        };
        let mut pairs: Vec<(u32, u32, f32, bool)> = Vec::new();
        for c in 0..sim.plate_id.len() {
            if sim.crust_type[c] != 1 {
                continue;
            }
            let pc = sim.plate_id[c];
            for &nb in sim.grid.neighbors_of(c as u32) {
                let nbu = nb as usize;
                let pn = sim.plate_id[nbu];
                if pn == pc || sim.crust_type[nbu] != 1 {
                    continue;
                }
                let (a, b) = (pc.min(pn), pc.max(pn));
                let thick = sim.thickness[c].max(sim.thickness[nbu]);
                // Normal approach speed at the shared edge midpoint, cm/yr:
                // dot(v_nb − v_c, ê from c toward nb) < 0 = converging.
                let xa = sim.grid.positions[c];
                let xb = sim.grid.positions[nbu];
                let mid = [
                    (xa[0] + xb[0]) as f64,
                    (xa[1] + xb[1]) as f64,
                    (xa[2] + xb[2]) as f64,
                ];
                let ml = (mid[0] * mid[0] + mid[1] * mid[1] + mid[2] * mid[2]).sqrt();
                let mid = [mid[0] / ml, mid[1] / ml, mid[2] / ml];
                let (wa, wb) = (omega(pc), omega(pn));
                let rel_w = [wb[0] - wa[0], wb[1] - wa[1], wb[2] - wa[2]];
                let rel = [
                    rel_w[1] * mid[2] - rel_w[2] * mid[1],
                    rel_w[2] * mid[0] - rel_w[0] * mid[2],
                    rel_w[0] * mid[1] - rel_w[1] * mid[0],
                ];
                let d = [
                    (xb[0] - xa[0]) as f64,
                    (xb[1] - xa[1]) as f64,
                    (xb[2] - xa[2]) as f64,
                ];
                let dn = d[0] * mid[0] + d[1] * mid[1] + d[2] * mid[2];
                let dt = [d[0] - dn * mid[0], d[1] - dn * mid[1], d[2] - dn * mid[2]];
                let dl = (dt[0] * dt[0] + dt[1] * dt[1] + dt[2] * dt[2])
                    .sqrt()
                    .max(1e-12);
                let sep_cmyr = (rel[0] * dt[0] + rel[1] * dt[1] + rel[2] * dt[2]) / dl * 637.1;
                let converging = sep_cmyr < -(M6_CONVERGING_CMYR as f64);
                match pairs.iter_mut().find(|e| e.0 == a && e.1 == b) {
                    Some(e) => {
                        e.2 = e.2.max(thick);
                        e.3 |= converging;
                    }
                    None => pairs.push((a, b, thick, converging)),
                }
            }
        }
        for ep in self.episodes.iter_mut() {
            if !ep.open {
                continue;
            }
            match pairs.iter().find(|e| e.0 == ep.a && e.1 == ep.b) {
                Some(&(_, _, thick, conv)) => {
                    ep.last_my = sim.t_my;
                    ep.peak_km = ep.peak_km.max(thick);
                    ep.converged |= conv;
                }
                None => ep.open = false,
            }
        }
        for &(a, b, thick, conv) in &pairs {
            if !self.episodes.iter().any(|e| e.open && e.a == a && e.b == b) {
                self.episodes.push(ContactEpisode {
                    a,
                    b,
                    first_my: sim.t_my,
                    last_my: sim.t_my,
                    peak_km: thick,
                    converged: conv,
                    open: true,
                });
            }
        }

        // Metrics 7 and 8 per alive plate, plate-id order. The median of
        // attached slab area over alive plates feeds the S1 fallback ratio.
        let mut attached_areas: Vec<u32> = Vec::new();
        for p in sim.plates.iter().filter(|p| p.alive) {
            attached_areas.push(
                p.slab
                    .iter()
                    .filter(|s| s.attached)
                    .map(|s| s.area_cells)
                    .sum(),
            );
        }
        attached_areas.sort_unstable();
        let median_attached = if attached_areas.is_empty() {
            0
        } else {
            attached_areas[attached_areas.len() / 2]
        };
        if self.slow_my.len() < np {
            self.slow_my.resize(np, 0.0);
            self.slow_flagged.resize(np, false);
            self.slab_free_my.resize(np, 0.0);
        }
        for pid in 0..np {
            let p = &sim.plates[pid];
            if !p.alive {
                self.slow_my[pid] = 0.0;
                continue;
            }
            let v = p.speed_deg_my as f64 * DEG_MY_TO_CMYR;
            self.speed_sum += v;
            self.speed_n += 1;
            let attached: u32 = p
                .slab
                .iter()
                .filter(|s| s.attached)
                .map(|s| s.area_cells)
                .sum();
            if attached > median_attached {
                self.above_median_speed_sum += v;
                self.above_median_n += 1;
            } else {
                self.below_median_speed_sum += v;
                self.below_median_n += 1;
            }
            if attached > 0 {
                self.slab_speed_sum += v;
                self.slab_n += 1;
                self.slab_free_my[pid] = 0.0;
            } else {
                self.free_speed_sum += v;
                self.free_n += 1;
                self.slab_free_my[pid] += dt;
                let in_contact = sim
                    .collisions
                    .iter()
                    .any(|t| t.a == pid as u32 || t.b == pid as u32);
                if self.slab_free_my[pid] >= M7_DRIFT_SETTLE_MY && !in_contact {
                    self.settled_free_speed_sum += v;
                    self.settled_free_n += 1;
                    if cont_cells[pid] * 2 >= cells[pid] {
                        self.free_cont_speed_sum += v;
                        self.free_cont_n += 1;
                    }
                }
            }
            // Metric 8: identical rule to `liveliness` above — the slow
            // clock pauses (not resets) while the plate is in a
            // continent-continent collision (it carries a §3 pair timer).
            if p.speed_deg_my < LIVELINESS_SPEED_FLOOR {
                let exempt = sim
                    .collisions
                    .iter()
                    .any(|t| t.a == pid as u32 || t.b == pid as u32);
                if !exempt {
                    self.slow_my[pid] += dt;
                    if self.slow_my[pid] > LIVELINESS_SLOW_MAX_MY && !self.slow_flagged[pid] {
                        self.slow_flagged[pid] = true;
                        self.slow_violations.push(format!(
                            "plate {} below {} deg/My for {} My ending {} My ({} cells)",
                            pid, LIVELINESS_SPEED_FLOOR, self.slow_my[pid], sim.t_my, cells[pid]
                        ));
                    }
                }
            } else {
                self.slow_my[pid] = 0.0;
            }
        }
    }

    pub fn finish(self, sim: &SimState) -> PhysicsReport {
        let span_my = self.last_my - self.first_my;
        let per_gy = |count: u64| count as f64 / (span_my as f64 / 1000.0).max(1e-9);

        // Metric 1 aggregates.
        let alive_min = self.alive_counts.iter().copied().min().unwrap_or(0);
        let alive_max = self.alive_counts.iter().copied().max().unwrap_or(0);
        let mean = self.alive_counts.iter().map(|&a| a as f64).sum::<f64>()
            / self.alive_counts.len().max(1) as f64;
        let var = self
            .alive_counts
            .iter()
            .map(|&a| (a as f64 - mean) * (a as f64 - mean))
            .sum::<f64>()
            / self.alive_counts.len().max(1) as f64;

        // Metric 2: every suture event since the baseline must carry a
        // contact record at or above the §3 minimum (conditions 2 and 3
        // have no other code path; the recorded fraction is the audit).
        let mut suture_bad = 0u32;
        let mut split_events = 0i64;
        for e in &sim.events[self.base_events..] {
            match e {
                super::keyframe::TectonicEvent::Suture {
                    contact_fraction,
                    contact_cells,
                    ..
                } => {
                    // §3 condition 1 (amended WO-0008 S1): the fraction
                    // threshold OR the absolute margin-span floor.
                    let abs_cells = super::step::SUTURE_ABS_CONTACT_KM / sim.cell_spacing_km;
                    if *contact_fraction < super::step::SUTURE_CONTACT_FRACTION
                        && (*contact_cells as f32) < abs_cells
                    {
                        suture_bad += 1;
                    }
                }
                super::keyframe::TectonicEvent::Split { .. } => split_events += 1,
                _ => {}
            }
        }
        let splits = sim.breakup_count - self.base_splits;
        // Metric 3: every split must appear in the event log with its §5
        // driver — a count mismatch means a split came from somewhere else.
        let splits_unattributed = splits as i64 - split_events;

        // Metric 4 aggregates.
        let mut longest_epoch = 0.0f32;
        let mut open_epoch = 0.0f32;
        let mut epoch_count = 0u32;
        for e in &self.epochs {
            epoch_count += 1;
            match e.end_my {
                Some(end) => longest_epoch = longest_epoch.max(end - e.start_my),
                None => open_epoch = self.last_my - e.start_my,
            }
        }

        // Metric 5b: worst 100 My window of backstop reassignments.
        let win = (100.0 / PHYS_SAMPLE_MY).round() as usize;
        let mut backstop_max = 0u64;
        for i in win..self.backstop_cum.len() {
            backstop_max = backstop_max.max(self.backstop_cum[i] - self.backstop_cum[i - win]);
        }
        if self.backstop_cum.len() > 1 && self.backstop_cum.len() <= win {
            backstop_max = *self.backstop_cum.last().unwrap();
        }

        // Metric 6 aggregates: merge same-pair episodes separated by less
        // than the merge gap (same zone), then judge persistence and peak.
        let mut episodes = self.episodes;
        episodes.sort_by(|x, y| {
            (x.a, x.b)
                .cmp(&(y.a, y.b))
                .then(x.first_my.partial_cmp(&y.first_my).unwrap())
        });
        let mut merged: Vec<ContactEpisode> = Vec::new();
        for e in episodes {
            match merged.last_mut() {
                Some(m) if m.a == e.a && m.b == e.b && e.first_my - m.last_my < M6_MERGE_GAP_MY => {
                    m.last_my = e.last_my;
                    m.peak_km = m.peak_km.max(e.peak_km);
                    m.converged |= e.converged;
                }
                _ => merged.push(e),
            }
        }
        let mut relief_episodes = 0u32;
        let mut relief_reached = 0u32;
        for e in &merged {
            if e.converged && e.last_my - e.first_my >= M6_PERSIST_MY {
                relief_episodes += 1;
                if e.peak_km > M6_RELIEF_THICKNESS_KM {
                    relief_reached += 1;
                }
            }
        }

        let mean_of = |sum: f64, n: u64| if n == 0 { 0.0 } else { sum / n as f64 };
        PhysicsReport {
            span_my,
            alive_min,
            alive_max,
            alive_stddev: var.sqrt(),
            alive_pinned_max_my: self.pinned_max_my,
            sutures_per_gy: per_gy(sim.suture_count - self.base_sutures),
            suture_bad_condition_count: suture_bad,
            splits_per_gy: per_gy(splits),
            splits_unattributed,
            max_share_outside_epochs: self.max_share_outside_epochs,
            supercontinent_epochs: epoch_count,
            longest_epoch_my: longest_epoch,
            open_epoch_my: open_epoch,
            exclave_samples: self.exclave_samples,
            backstop_max_per_100my: backstop_max,
            relief_episodes,
            relief_reached,
            mean_speed_cmyr: mean_of(self.speed_sum, self.speed_n),
            slab_attached_mean_cmyr: mean_of(self.slab_speed_sum, self.slab_n),
            slab_free_mean_cmyr: mean_of(self.free_speed_sum, self.free_n),
            slab_free_plate_samples: self.free_n,
            settled_free_mean_cmyr: mean_of(self.settled_free_speed_sum, self.settled_free_n),
            settled_free_plate_samples: self.settled_free_n,
            above_median_mean_cmyr: mean_of(self.above_median_speed_sum, self.above_median_n),
            below_median_mean_cmyr: mean_of(self.below_median_speed_sum, self.below_median_n),
            slab_free_cont_mean_cmyr: mean_of(self.free_cont_speed_sum, self.free_cont_n),
            slab_free_cont_plate_samples: self.free_cont_n,
            slow_violations: self.slow_violations,
            cont_share_series: self.cont_share_series,
            relic_basin_violations: self.relic_violations,
            cont_cells_start: self.cont_cells_first.unwrap_or(0),
            cont_cells_end: self.cont_cells_last,
            max_orogen_width: self.max_orogen_width,
            arc_wall_violations: self.arc_wall_violations,
            vol_advect_q: sim.vol_advect_q,
            vol_closure_q: sim.vol_closure_q,
            vol_arc_q: sim.vol_arc_q,
            vol_collision_q: sim.vol_collision_q,
            vol_rift_q: sim.vol_rift_q,
            vol_spread_q: sim.vol_spread_q,
            vol_relax_q: sim.vol_relax_q,
            vol_quantize_q: sim.vol_quantize_q,
            underthrust_removed_q: sim.underthrust_removed_q,
            underthrust_deposited_q: sim.underthrust_deposited_q,
            underthrust_spilled_q: sim.underthrust_spilled_q,
            underthrust_incorporated_q: sim.underthrust_incorporated_q,
        }
    }
}

/// WO-0008 S1 relic-basin audit: the largest ENCLOSED oceanic region
/// (bordering continental cells ≥ 80% on the pair, counted per basin
/// edge — the same test the closure mechanic applies) larger than the
/// relic cap within `SUTURE_OCEAN_RINGS` of the pair's contact. Returns
/// (region cells, pair-border fraction) for the first such region in
/// cell-id order, or None. Serial, id-ordered.
fn oversized_enclosed_basin(sim: &SimState, a: u32, b: u32) -> Option<(u32, f64)> {
    use std::collections::VecDeque as Dq;
    let n = sim.plate_id.len();
    // Contact cells: continental cells of either plate touching the other
    // plate's continent (both sides, id order).
    let mut depth = vec![u16::MAX; n];
    let mut queue: Dq<u32> = Dq::new();
    #[allow(clippy::needless_range_loop)] // depth is written at c, not iterated
    for c in 0..n {
        if sim.crust_type[c] != 1 {
            continue;
        }
        let p = sim.plate_id[c];
        let other = if p == a {
            b
        } else if p == b {
            a
        } else {
            continue;
        };
        let touches = sim
            .grid
            .neighbors_of(c as u32)
            .iter()
            .any(|&nb| sim.plate_id[nb as usize] == other && sim.crust_type[nb as usize] == 1);
        if touches {
            depth[c] = 0;
            queue.push_back(c as u32);
        }
    }
    // Suture window: rings on the two plates.
    let mut window_ocean: Vec<u32> = Vec::new();
    while let Some(c) = queue.pop_front() {
        let dc = depth[c as usize];
        if dc >= super::step::SUTURE_OCEAN_RINGS {
            continue;
        }
        for &nb in sim.grid.neighbors_of(c) {
            let nbu = nb as usize;
            let p = sim.plate_id[nbu];
            if depth[nbu] == u16::MAX && (p == a || p == b) {
                depth[nbu] = dc + 1;
                queue.push_back(nb);
                if sim.crust_type[nbu] == 0 {
                    window_ocean.push(nb);
                }
            }
        }
    }
    // Ocean regions touching the window, with the enclosure test.
    let mut visited = vec![false; n];
    for &c0 in &window_ocean {
        if visited[c0 as usize] {
            continue;
        }
        let mut region: Vec<u32> = Vec::new();
        visited[c0 as usize] = true;
        queue.push_back(c0);
        while let Some(c) = queue.pop_front() {
            region.push(c);
            for &nb in sim.grid.neighbors_of(c) {
                let nbu = nb as usize;
                if !visited[nbu] && sim.crust_type[nbu] == 0 {
                    visited[nbu] = true;
                    queue.push_back(nb);
                }
            }
        }
        if region.len() as u32 <= super::step::RELIC_BASIN_KEEP_CELLS {
            continue;
        }
        let (mut border, mut border_ab) = (0u64, 0u64);
        for &c in &region {
            for &nb in sim.grid.neighbors_of(c) {
                let nbu = nb as usize;
                if sim.crust_type[nbu] == 1 {
                    border += 1;
                    let p = sim.plate_id[nbu];
                    if p == a || p == b {
                        border_ab += 1;
                    }
                }
            }
        }
        if border > 0 && border_ab as f64 >= 0.8 * border as f64 {
            return Some((region.len() as u32, border_ab as f64 / border as f64));
        }
    }
    None
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
