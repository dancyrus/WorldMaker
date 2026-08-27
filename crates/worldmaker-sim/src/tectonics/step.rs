//! The tectonic time step: plate motion, semi-Lagrangian advection with
//! ownership resolution, boundary classification, and the geological events.
//!
//! Ownership per step is a forward-scatter + gather scheme: every cell first
//! claims its destination cell (and that cell's ring) for its plate in an
//! atomic candidate bitmask — atomics are only OR'd, so the result is
//! order-independent — then every cell tests which candidate plates actually
//! cover it by back-rotating and sampling previous ownership. This stays
//! correct even when a fast plate sweeps several cells in one 2 My step.
//!
//! Slow plates never freeze to the grid: each plate banks its per-step
//! rotation into a pending matrix and commits it to the advection pass only
//! once the banked angle reaches ~3/4 of a cell, so sub-cell motion
//! accumulates instead of rounding away (design-review finding).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use rand::RngCore;
use rayon::prelude::*;

use worldmaker_core::dmath::{
    add3, cross3, dot3, gaussian_f32, mat3_mul, mat3_mul3, mat3_transpose, normalize3,
    random_tangent, rotation3, scale3, sub3,
};
use worldmaker_core::rng::sub_rng;
use worldmaker_core::Grid;

use super::keyframe::{Keyframe, PairTimer, PlateState, IDENTITY3};
use super::{
    TectonicsParams, DT_MY, F_ARC, F_BND_CONVERGENT, F_BND_DIVERGENT, F_BND_TRANSFORM, F_HOTSPOT,
    F_RIDGE, F_RIFT, F_TRENCH, STAGE_ID,
};

// ----- model constants (decision log: "Phase 1 tectonic constants") -----

/// Earth radius; the grid is a unit sphere scaled by this for physical units.
pub const R_EARTH_KM: f32 = 6371.0;
/// rad/My × this = cm/yr surface speed at the rotation equator.
const RADMY_TO_CMYR: f32 = R_EARTH_KM * 0.1;
const DEG2RAD: f32 = std::f32::consts::PI / 180.0;
/// Boundary classification threshold, cm/yr of normal separation (spec).
const CLASSIFY_CMYR: f32 = 0.4;
/// Slab pull: speed gain per unit subducting-boundary fraction.
const SLAB_PULL_GAIN: f32 = 1.0;
/// Collision damping: speed loss per unit *saturated* colliding fraction.
/// Full damping (1.0) lets a jammed plate actually stall — otherwise its
/// trailing edge keeps opening ocean while its front cannot advance, and the
/// continent conveyor-belts into the collision and is destroyed.
const COLLISION_DAMP: f32 = 1.0;
/// Collision damping saturates once the colliding contact reaches this
/// fraction of the plate's boundary — or the absolute floor below, whichever
/// is larger. Even a small continental contact locks the plate (India–Asia
/// style; also terrane dockings): without this, sub-saturation contacts
/// never slow below the suture threshold and grind continental margins away
/// for the whole run.
const COLLISION_SATURATION: f32 = 0.05;
const COLLISION_SATURATION_MIN_CELLS: f32 = 4.0;
/// Per-step relaxation of speed toward its slab-pull target: braking is much
/// faster than acceleration, so a plate hitting a continent stalls within a
/// few steps instead of grinding tens of My of margin away.
const SPEED_RELAX_UP: f32 = 0.15;
const SPEED_RELAX_DOWN: f32 = 0.5;
/// Speed clamp, deg/My (spec; shared with setup's base-speed draw).
pub(super) const SPEED_MIN: f32 = 0.1;
/// Raised 1.2 → 2.0 (WO-0003 Fix 4): at tectonic_vigor 1.73 the old cap
/// pegged most base speeds and every slab-pull target at 1.2, so vigor
/// stopped spreading speeds. 2.0 sits above the fastest draw at the maximum
/// vigor 2.0 (|N(0.5, 0.15)| × 2), so the clamp is a safety rail again, not
/// the operating point.
pub(super) const SPEED_MAX: f32 = 2.0;
/// Speed floor of a fully jammed plate (f_coll = 1), deg/My. The old floor
/// `SPEED_MIN * (1 - f_coll)` collapsed to exactly zero, and with the
/// slab-pull target also zeroed by COLLISION_DAMP the clamp held jammed
/// plates at 0.00 deg/My forever (WO-0003 Fix 4 diagnosis). A jammed plate
/// now keeps this residual creep — ~0.55 cm/yr at the rotation equator, at
/// the suture threshold scale, so welded pairs still read slow and suture —
/// and can never freeze to the grid permanently.
pub(super) const SPEED_FLOOR_JAMMED: f32 = 0.05;
/// Euler-pole random walk, degrees (1 sigma) per step.
const POLE_WALK_DEG: f32 = 0.6;
/// Banked sub-cell rotation commits once it reaches this fraction of a cell.
const COMMIT_FRACTION: f32 = 0.75;
/// Oceanic crust created at ridges: thickness (km).
pub(super) const OCEAN_THICKNESS_KM: f32 = 7.0;
/// Continental crust thinner than this is subductible (young arc terranes
/// can be recycled; keeps the continental-area inventory closed).
const SUBDUCTIBLE_CONT_KM: f32 = 30.0;
/// Arc crust growth over an active subduction zone, km/My: fast while the
/// overrider is oceanic (island arcs emerge in 20–40 My), slower on
/// continental margins.
const ARC_GROWTH_OCEAN_KM_MY: f32 = 0.6;
const ARC_GROWTH_CONT_KM_MY: f32 = 0.15;
/// Oceanic overriding crust becomes continental (island arc) at this
/// thickness, km (spec: island arcs ~20–25 km).
const ISLAND_ARC_CONVERT_KM: f32 = 20.0;
/// Continental collision thickening, km/My per cm/yr of convergence.
const COLLISION_THICKEN: f32 = 0.12;
/// Crust thickness cap (km, spec).
const THICKNESS_CAP_KM: f32 = 70.0;
/// Continental rifts thin after this much sustained divergence (My)...
const RIFT_ONSET_MY: f32 = 20.0;
/// ...at this rate (km/My)...
const RIFT_THIN_KM_MY: f32 = 0.2;
/// ...and become ocean below this thickness (km). All spec.
const RIFT_OCEANIZE_KM: f32 = 25.0;
/// Rift timer decays at 2× real time on non-divergent steps (hysteresis so
/// classification noise on quasi-transform boundaries cannot mature a rift).
const RIFT_DECAY_MULT: f32 = 2.0;
/// Slow-collision threshold for suturing (cm/yr) and required duration (My).
/// Raised 0.5 → 1.2 (WO-0003 Fix 4): two fully jammed plates now creep at
/// the SPEED_FLOOR_JAMMED residual instead of stopping dead, which closes
/// their contact at up to ~1.1 cm/yr (2 × 0.05 deg/My at the rotation
/// equator). At 0.5 that creep read as *active* convergence, reset the pair
/// timer every step, and welded pairs ground against each other forever —
/// the threshold must sit above the jam-creep ceiling so a stalled
/// collision always matures toward suture.
const SUTURE_SLOW_CMYR: f32 = 1.2;
pub(super) const SUTURE_AFTER_MY: f32 = 30.0;
/// Fast-convergence steps decay the pair timer at 2× real time instead of
/// hard-resetting it (WO-0003 Fix 4): a welded pair's escape attempts
/// flicker the mean above SUTURE_SLOW_CMYR for a few steps at a time, and
/// with a hard reset those flickers postponed suturing by hundreds of My
/// (measured: a weld formed at ~1270 My did not suture until ~1610 My).
/// Same hysteresis pattern as RIFT_DECAY_MULT.
const SUTURE_DECAY_MULT: f32 = 2.0;
/// Plate-count floor (suturing stops) and ceiling (breakup stops).
const PLATE_FLOOR: usize = 6;
const PLATE_CEIL: usize = 24;
/// Supercontinent breakup: plate area fraction and minimum suture age (My).
const BREAKUP_AREA_FRACTION: f32 = 1.0 / 3.0;
const BREAKUP_SUTURE_AGE_MY: f32 = 100.0;
/// Relative angular speed given to the two halves of a breakup, deg/My.
const BREAKUP_RIFT_SPEED: f32 = 0.3;
/// Hotspot shield-building rate at the center cell / its ring, km/My, and
/// the buildup cap (km). Sized so 5–10 My of residence builds a shield that
/// can breach sea level over mature (−5,600 m) ocean floor.
const HOTSPOT_RATE_CENTER: f32 = 0.8;
const HOTSPOT_RATE_RING: f32 = 0.4;
const HOTSPOT_CAP_KM: f32 = 8.0;
/// Buildup above which the HOTSPOT feature flag shows.
const HOTSPOT_FLAG_KM: f32 = 0.5;
/// Inactive orogens relax toward this thickness (km) with a 200 My time
/// constant; the factor is exp(-DT_MY/200) precomputed as a literal so no
/// libm call sits in the sim path. Hotspot buildup decays with the same
/// constant.
const OROGENY_BASE_KM: f32 = 38.0;
const OROGENY_RELAX_FACTOR: f32 = 0.990_049_83; // exp(-2/200)
/// Crust with orogeny_age beyond this is primordial (cratons), exempt from
/// relaxation so painted and initial cratons keep their profiles.
const OROGENY_RELAX_MAX_AGE_MY: f32 = 1200.0;
/// Arc distance band inboard of the trench (km, spec).
const ARC_MIN_KM: f32 = 150.0;
const ARC_MAX_KM: f32 = 250.0;

const NONE: u32 = u32::MAX;

/// Per-cell result of the advection gather.
#[derive(Clone, Copy)]
struct CellOut {
    plate: u32,
    ctype: u32,
    features: u32,
    age: f32,
    thick: f32,
    orog: f32,
    rift: f32,
    build: f32,
    /// Plate whose crust was consumed at this cell, or NONE.
    subducted: u32,
    /// Plate jammed against `plate` in continent-continent contact, or NONE.
    collided: u32,
}

/// Per-cell result of boundary classification.
#[derive(Clone, Copy, Default)]
struct ClassOut {
    /// F_BND_* display bits.
    flags: u32,
    /// This cell sits on a continent-continent divergent edge (rift driver).
    div_cont: bool,
    /// Strongest continent-continent convergence at this cell, cm/yr (> 0).
    conv_cont_cmyr: f32,
    /// Continent-continent contact partner (any class), or NONE.
    contact_partner: u32,
    /// Normal convergence toward that partner, cm/yr (negative = separating).
    contact_conv_cmyr: f32,
    /// Cell has at least one foreign neighbor.
    boundary: bool,
}

/// The working state of one tectonic run. All arrays are cell-count long;
/// `plates` is indexed by plate id and only ever grows (dead plates keep
/// their slot with `alive = false`).
pub struct SimState {
    pub grid: Arc<Grid>,
    pub t_my: f32,
    pub cell_spacing_km: f32,

    // Per-cell state.
    pub plate_id: Vec<u32>,
    pub crust_type: Vec<u32>,
    pub crust_age: Vec<f32>,
    pub thickness: Vec<f32>,
    pub orogeny_age: Vec<f32>,
    pub rift_age: Vec<f32>,
    pub buildup: Vec<f32>,
    pub features: Vec<u32>,
    pub elev: Vec<f32>,
    pub sea_offset_m: f32,

    // Plate-level state.
    pub plates: Vec<PlateState>,
    pub collisions: Vec<PairTimer>,
    pub hotspots: Vec<[f32; 3]>,
    /// Deterministic seed for the elevation detail noise.
    pub noise_seed: u64,

    // Per-plate stats from the previous step (indexed by plate id). Mirrored
    // into PlateState at keyframe encode for bit-exact resume.
    boundary_cells: Vec<u32>,
    subducting_cells: Vec<u32>,
    colliding_cells: Vec<u32>,
    /// Cell count per plate id, kept current.
    plate_cells: Vec<u32>,

    // Reused scratch.
    cand_mask: Vec<AtomicU32>,
    outs: Vec<CellOut>,
    class: Vec<ClassOut>,
    bfs_depth: Vec<u16>,
    pub(super) hotspot_hints: Vec<u32>,

    // Cumulative continental-inventory flows (cells), for the acceptance
    // harness and stability diagnostics.
    pub cont_lost_to_ridge_gap: u64,
    pub cont_lost_to_consumption: u64,
    pub cont_lost_to_rift: u64,
    pub cont_gained_by_advection: u64,
    pub cont_gained_by_arc: u64,
    pub suture_count: u64,
    pub breakup_count: u64,
}

impl SimState {
    pub(super) fn new_empty(grid: &Arc<Grid>) -> SimState {
        let n = grid.cell_count() as usize;
        let cell_spacing_km = (4.0 * std::f64::consts::PI / n as f64).sqrt() as f32 * R_EARTH_KM;
        SimState {
            grid: grid.clone(),
            t_my: 0.0,
            cell_spacing_km,
            plate_id: vec![0; n],
            crust_type: vec![0; n],
            crust_age: vec![0.0; n],
            thickness: vec![OCEAN_THICKNESS_KM; n],
            orogeny_age: vec![0.0; n],
            rift_age: vec![0.0; n],
            buildup: vec![0.0; n],
            features: vec![0; n],
            elev: vec![0.0; n],
            sea_offset_m: 0.0,
            plates: Vec::new(),
            collisions: Vec::new(),
            hotspots: Vec::new(),
            noise_seed: 0,
            boundary_cells: Vec::new(),
            subducting_cells: Vec::new(),
            colliding_cells: Vec::new(),
            plate_cells: Vec::new(),
            cand_mask: (0..n).map(|_| AtomicU32::new(0)).collect(),
            outs: Vec::new(),
            class: vec![ClassOut::default(); n],
            bfs_depth: vec![u16::MAX; n],
            hotspot_hints: Vec::new(),
            cont_lost_to_ridge_gap: 0,
            cont_lost_to_consumption: 0,
            cont_lost_to_rift: 0,
            cont_gained_by_advection: 0,
            cont_gained_by_arc: 0,
            suture_count: 0,
            breakup_count: 0,
        }
    }

    pub fn setup(master_seed: u64, grid: &Arc<Grid>, params: &TectonicsParams) -> SimState {
        super::setup::setup(master_seed, grid, params)
    }

    /// Quantize the per-cell working state through the keyframe encoding so
    /// the state a keyframe stores IS the state the run continues from —
    /// this is what makes resume-from-keyframe bit-exact. Called right
    /// before each keyframe's elevation derive. The formulas must mirror
    /// [`Keyframe::encode`]/decode exactly.
    pub(super) fn quantize_state(&mut self) {
        // Must mirror Keyframe::encode's round-then-clamp exactly.
        let q_u16 = |v: f32| -> f32 { (v.round().clamp(0.0, 65_535.0) as u16) as f32 };
        for i in 0..self.crust_age.len() {
            self.crust_age[i] = q_u16(self.crust_age[i]);
            self.thickness[i] = q_u16(self.thickness[i] * 100.0) * 0.01;
            self.orogeny_age[i] = q_u16(self.orogeny_age[i]);
            self.rift_age[i] = q_u16(self.rift_age[i]);
            self.buildup[i] = q_u16(self.buildup[i] * 100.0) * 0.01;
        }
    }

    /// Restore full state from a keyframe (plate-drag re-runs, branching).
    /// `hotspots` come from the history the keyframe belongs to;
    /// `master_seed` re-derives the detail-noise seed the way setup does.
    pub fn from_keyframe(
        grid: &Arc<Grid>,
        master_seed: u64,
        hotspots: &[[f32; 3]],
        kf: &Keyframe,
    ) -> SimState {
        let mut s = Self::new_empty(grid);
        let n = grid.cell_count() as usize;
        assert_eq!(kf.elev_m.len(), n, "keyframe/grid mismatch");
        s.t_my = kf.t_my;
        s.sea_offset_m = kf.sea_offset_m;
        for i in 0..n {
            s.plate_id[i] = kf.plate_id[i] as u32;
            s.crust_type[i] = u32::from(kf.flags[i] & (1 << 15) != 0);
            s.crust_age[i] = kf.crust_age_my[i] as f32;
            s.thickness[i] = kf.thickness_ckm[i] as f32 * 0.01;
            s.orogeny_age[i] = kf.orogeny_age_my[i] as f32;
            s.rift_age[i] = kf.rift_age_my[i] as f32;
            s.buildup[i] = kf.buildup_ckm[i] as f32 * 0.01;
            s.features[i] = (kf.flags[i] & 0xff) as u32;
            s.elev[i] = kf.elev_m[i] as f32;
        }
        s.plates = kf.plates.clone();
        s.collisions = kf.collisions.clone();
        s.hotspots = hotspots.to_vec();
        s.hotspot_hints = vec![0; s.hotspots.len()];
        s.noise_seed = sub_rng(master_seed, STAGE_ID, "detail-noise").next_u64();
        // Stats travel inside PlateState — restore, don't recompute, so the
        // resumed run is bit-identical to the original.
        let np = s.plates.len();
        s.boundary_cells = vec![0; np];
        s.subducting_cells = vec![0; np];
        s.colliding_cells = vec![0; np];
        s.plate_cells = vec![0; np];
        for p in &s.plates {
            let i = p.id as usize;
            s.boundary_cells[i] = p.boundary_cells;
            s.subducting_cells[i] = p.subducting_cells;
            s.colliding_cells[i] = p.colliding_cells;
        }
        for &p in &s.plate_id {
            s.plate_cells[p as usize] += 1;
        }
        s
    }

    /// Recount plate cells and populate boundary stats (setup only; resumed
    /// runs restore stats from the keyframe instead).
    pub(super) fn init_stats(&mut self) {
        let np = self.plates.len();
        self.boundary_cells = vec![0; np];
        self.subducting_cells = vec![0; np];
        self.colliding_cells = vec![0; np];
        self.plate_cells = vec![0; np];
        for &p in &self.plate_id {
            self.plate_cells[p as usize] += 1;
        }
        self.classify_boundaries();
        self.accumulate_boundary_stats();
    }

    /// Advance one step. `step_idx` is the absolute step number since t = 0;
    /// all randomness is keyed on it, so resumed runs replay identically.
    pub fn step(&mut self, master_seed: u64, step_idx: u32) {
        self.motion_update(master_seed, step_idx);
        self.advect();
        self.classify_boundaries();
        self.accumulate_boundary_stats();
        self.apply_arcs();
        self.apply_collisions_and_rifts();
        self.update_pair_timers_and_sutures();
        self.maybe_breakup(master_seed, step_idx);
        self.apply_hotspots();
        self.age_and_relax();
        self.t_my += DT_MY;
    }

    // ----- F: plate motion -----

    fn motion_update(&mut self, master_seed: u64, step_idx: u32) {
        for pid in 0..self.plates.len() {
            if !self.plates[pid].alive {
                continue;
            }
            let mut rng = sub_rng(
                master_seed,
                STAGE_ID,
                &format!("plate-motion-{pid}-step{step_idx}"),
            );
            let bnd = self.boundary_cells[pid].max(1) as f32;
            let f_sub = self.subducting_cells[pid] as f32 / bnd;
            let sat = (bnd * COLLISION_SATURATION).max(COLLISION_SATURATION_MIN_CELLS);
            let f_coll = (self.colliding_cells[pid] as f32 / sat).min(1.0);
            let p = &mut self.plates[pid];
            // Slow pole random walk.
            let axis = random_tangent(&mut rng, p.pole);
            let ang = (gaussian_f32(&mut rng) * POLE_WALK_DEG * DEG2RAD).clamp(-0.05, 0.05);
            let rot = rotation3(axis, ang);
            p.pole = normalize3(mat3_mul(&rot, p.pole));
            // Slab pull / collision damping from last step's boundary makeup.
            // The speed floor eases with collision fraction but never below
            // SPEED_FLOOR_JAMMED: a fully jammed plate keeps a residual
            // convergence creep instead of stopping dead (Fix 4 liveliness).
            let target = p.base_speed_deg_my
                * (1.0 + SLAB_PULL_GAIN * f_sub)
                * (1.0 - COLLISION_DAMP * f_coll);
            let floor = SPEED_MIN - (SPEED_MIN - SPEED_FLOOR_JAMMED) * f_coll;
            let relax = if target < p.speed_deg_my {
                SPEED_RELAX_DOWN
            } else {
                SPEED_RELAX_UP
            };
            p.speed_deg_my =
                (p.speed_deg_my + relax * (target - p.speed_deg_my)).clamp(floor, SPEED_MAX);
            // Bank this step's rotation; advection commits it when it
            // reaches a usable fraction of a cell.
            let step_rot = rotation3(p.pole, p.speed_deg_my * DEG2RAD * DT_MY);
            p.pending_rot = mat3_mul3(&step_rot, &p.pending_rot);
            p.pending_deg += p.speed_deg_my * DT_MY;
        }
    }

    /// Angular velocity vector (rad/My) of a plate.
    #[inline]
    fn omega(&self, pid: u32) -> [f32; 3] {
        let p = &self.plates[pid as usize];
        scale3(p.pole, p.speed_deg_my * DEG2RAD)
    }

    // ----- A: ownership + advection -----

    fn advect(&mut self) {
        let n = self.grid.cell_count() as usize;
        let commit_deg = COMMIT_FRACTION * self.cell_spacing_km / (R_EARTH_KM * DEG2RAD);

        // Dense index for alive plates so candidates fit a u32 bitmask.
        let mut id_of_dense: Vec<u32> = Vec::new();
        let mut dense_of_id: Vec<u32> = vec![NONE; self.plates.len()];
        for (pid, p) in self.plates.iter().enumerate() {
            if p.alive {
                dense_of_id[pid] = id_of_dense.len() as u32;
                id_of_dense.push(pid as u32);
            }
        }
        let nd = id_of_dense.len();
        assert!(nd <= 32, "alive plate count {nd} exceeds candidate mask");

        // Effective rotation this step: the banked pending rotation for
        // plates past the commit threshold, identity for the rest.
        let mut fwd = Vec::with_capacity(nd);
        let mut inv = Vec::with_capacity(nd);
        let mut committing = vec![false; nd];
        for (d, &pid) in id_of_dense.iter().enumerate() {
            let p = &self.plates[pid as usize];
            let m = if p.pending_deg >= commit_deg {
                committing[d] = true;
                p.pending_rot
            } else {
                IDENTITY3
            };
            inv.push(mat3_transpose(&m));
            fwd.push(m);
        }

        // Zero the candidate masks.
        self.cand_mask
            .par_iter()
            .for_each(|m| m.store(0, Ordering::Relaxed));

        let grid = &self.grid;
        let plate_id = &self.plate_id;
        let cand = &self.cand_mask;

        // Forward scatter: each cell claims its destination and that cell's
        // ring for its plate. Atomic OR is commutative — deterministic.
        (0..n).into_par_iter().for_each(|c| {
            let d = dense_of_id[plate_id[c] as usize];
            if !committing[d as usize] {
                return; // not moving this step; gather covers it locally
            }
            let dst_pos = mat3_mul(&fwd[d as usize], grid.positions[c]);
            let dst = grid.nearest_cell(dst_pos, Some(c as u32));
            let bit = 1u32 << d;
            cand[dst as usize].fetch_or(bit, Ordering::Relaxed);
            for &nb in grid.neighbors_of(dst) {
                cand[nb as usize].fetch_or(bit, Ordering::Relaxed);
            }
        });

        // Gather: resolve ownership per cell.
        let prev_ctype = &self.crust_type;
        let prev_age = &self.crust_age;
        let prev_thick = &self.thickness;
        let prev_orog = &self.orogeny_age;
        let prev_rift = &self.rift_age;
        let prev_build = &self.buildup;
        let prev_feat = &self.features;
        let inv_ref = &inv;
        let id_of_dense_ref = &id_of_dense;
        let dense_of_id_ref = &dense_of_id;

        // Copy of a cell's previous state, used for no-op and jammed cells.
        let keep_cell = |c: usize, features: u32, collided: u32| CellOut {
            plate: plate_id[c],
            ctype: prev_ctype[c],
            features,
            age: prev_age[c],
            thick: prev_thick[c],
            orog: prev_orog[c],
            rift: prev_rift[c],
            build: prev_build[c],
            subducted: NONE,
            collided,
        };

        let mut outs = std::mem::take(&mut self.outs);
        (0..n)
            .into_par_iter()
            .map(|c| {
                let x = grid.positions[c];
                let mut mask = cand[c].load(Ordering::Relaxed);
                mask |= 1u32 << dense_of_id_ref[plate_id[c] as usize];
                for &nb in grid.neighbors_of(c as u32) {
                    mask |= 1u32 << dense_of_id_ref[plate_id[nb as usize] as usize];
                }

                // Coverage tests in ascending dense order (deterministic).
                let mut cover_plate = [0u32; 8];
                let mut cover_src = [0u32; 8];
                let mut covers = 0usize;
                let mut m = mask;
                while m != 0 && covers < 8 {
                    let d = m.trailing_zeros();
                    m &= m - 1;
                    let pid = id_of_dense_ref[d as usize];
                    let src_pos = mat3_mul(&inv_ref[d as usize], x);
                    let src = grid.nearest_cell(src_pos, Some(c as u32));
                    if plate_id[src as usize] == pid {
                        cover_plate[covers] = pid;
                        cover_src[covers] = src;
                        covers += 1;
                    }
                }

                // Boundary class of this cell on the previous step, for
                // gating events at transforms (zigzag hex boundaries spray
                // false gaps/overlaps under tangential slip).
                let was_transform_only = prev_feat[c] & F_BND_TRANSFORM != 0
                    && prev_feat[c] & F_BND_DIVERGENT == 0
                    && prev_feat[c] & F_BND_CONVERGENT == 0;

                match covers {
                    0 => {
                        if was_transform_only {
                            // Transform jitter, not spreading: keep the cell.
                            keep_cell(c, 0, NONE)
                        } else {
                            // Divergent gap: fresh ridge crust on the
                            // trailing edge of the previous owner.
                            CellOut {
                                plate: plate_id[c],
                                ctype: 0,
                                features: F_RIDGE,
                                age: 0.0,
                                thick: OCEAN_THICKNESS_KM,
                                orog: 0.0,
                                rift: 0.0,
                                build: 0.0,
                                subducted: NONE,
                                collided: NONE,
                            }
                        }
                    }
                    1 => {
                        let s = cover_src[0] as usize;
                        CellOut {
                            plate: cover_plate[0],
                            ctype: prev_ctype[s],
                            features: 0,
                            age: prev_age[s],
                            thick: prev_thick[s],
                            orog: prev_orog[s],
                            rift: prev_rift[s],
                            build: prev_build[s],
                            subducted: NONE,
                            collided: NONE,
                        }
                    }
                    _ => {
                        // Overlap. "Hard" crust (continent ≥ 30 km) cannot be
                        // consumed; two hard plates jam instead of
                        // interpenetrating.
                        let is_hard = |i: usize| {
                            prev_ctype[cover_src[i] as usize] == 1
                                && prev_thick[cover_src[i] as usize] >= SUBDUCTIBLE_CONT_KM
                        };
                        let hard_count = (0..covers).filter(|&i| is_hard(i)).count();
                        if hard_count >= 2 {
                            // Continent-continent jam: the cell freezes in
                            // place; both sides record the collision.
                            let first_hard = (0..covers).find(|&i| is_hard(i)).unwrap();
                            let other = (first_hard + 1..covers)
                                .find(|&i| is_hard(i))
                                .map(|i| cover_plate[i])
                                .unwrap_or(NONE);
                            let mine = if plate_id[c] == cover_plate[first_hard] {
                                other
                            } else {
                                cover_plate[first_hard]
                            };
                            keep_cell(c, 0, mine)
                        } else {
                            // At most one hard plate: it overrides; otherwise
                            // the youngest (least dense) soft crust overrides.
                            let mut win = 0usize;
                            if hard_count == 1 {
                                win = (0..covers).find(|&i| is_hard(i)).unwrap();
                            } else {
                                for ch in 1..covers {
                                    let (ws, cs) =
                                        (cover_src[win] as usize, cover_src[ch] as usize);
                                    if prev_age[cs] < prev_age[ws]
                                        || (prev_age[cs] == prev_age[ws]
                                            && cover_plate[ch] < cover_plate[win])
                                    {
                                        win = ch;
                                    }
                                }
                            }
                            // First non-winner is the consumed plate for
                            // slab-pull stats (multi-way overlaps are rare).
                            let loser = (0..covers).find(|&i| i != win).unwrap();
                            let s = cover_src[win] as usize;
                            let (subducted, features) = if was_transform_only {
                                (NONE, 0) // transform jitter: no trench
                            } else {
                                (cover_plate[loser], F_TRENCH)
                            };
                            CellOut {
                                plate: cover_plate[win],
                                ctype: prev_ctype[s],
                                features,
                                age: prev_age[s],
                                thick: prev_thick[s],
                                orog: prev_orog[s],
                                rift: prev_rift[s],
                                build: prev_build[s],
                                subducted,
                                collided: NONE,
                            }
                        }
                    }
                }
            })
            .collect_into_vec(&mut outs);

        // Scatter into the SoA arrays and refresh plate cell counts.
        for v in self.plate_cells.iter_mut() {
            *v = 0;
        }
        for (c, o) in outs.iter().enumerate() {
            // Inventory flows (self.crust_type[c] still holds the previous
            // value at this point in the serial scatter).
            match (self.crust_type[c], o.ctype) {
                (1, 0) if o.features & F_RIDGE != 0 => self.cont_lost_to_ridge_gap += 1,
                (1, 0) => self.cont_lost_to_consumption += 1,
                (0, 1) => self.cont_gained_by_advection += 1,
                _ => {}
            }
            self.plate_id[c] = o.plate;
            self.crust_type[c] = o.ctype;
            self.crust_age[c] = o.age;
            self.thickness[c] = o.thick;
            self.orogeny_age[c] = o.orog;
            self.rift_age[c] = o.rift;
            self.buildup[c] = o.build;
            self.features[c] = o.features;
            self.plate_cells[o.plate as usize] += 1;
        }
        self.outs = outs;

        // Consume committed pending rotations; retire fully-consumed plates.
        for (d, &pid) in id_of_dense.iter().enumerate() {
            if committing[d] {
                let p = &mut self.plates[pid as usize];
                p.pending_rot = IDENTITY3;
                p.pending_deg = 0.0;
            }
        }
        for pid in 0..self.plates.len() {
            if self.plates[pid].alive && self.plate_cells[pid] == 0 {
                self.plates[pid].alive = false;
                self.collisions
                    .retain(|t| t.a != pid as u32 && t.b != pid as u32);
                log::debug!("t={} My: plate {pid} fully consumed", self.t_my);
            }
        }
    }

    // ----- B: boundary classification -----

    fn classify_boundaries(&mut self) {
        let n = self.grid.cell_count() as usize;
        let omegas: Vec<[f32; 3]> = (0..self.plates.len())
            .map(|pid| {
                if self.plates[pid].alive {
                    self.omega(pid as u32)
                } else {
                    [0.0; 3]
                }
            })
            .collect();
        let grid = &self.grid;
        let plate_id = &self.plate_id;
        let ctype = &self.crust_type;

        let mut class = std::mem::take(&mut self.class);
        (0..n)
            .into_par_iter()
            .map(|c| {
                let a = plate_id[c];
                let xa = grid.positions[c];
                let mut out = ClassOut {
                    contact_partner: NONE,
                    ..ClassOut::default()
                };
                let mut any_div = false;
                let mut any_conv = false;
                let mut any_trans = false;
                for &nb in grid.neighbors_of(c as u32) {
                    let b = plate_id[nb as usize];
                    if b == a {
                        continue;
                    }
                    out.boundary = true;
                    let xb = grid.positions[nb as usize];
                    let mid = normalize3(add3(xa, xb));
                    let va = cross3(omegas[a as usize], mid);
                    let vb = cross3(omegas[b as usize], mid);
                    let d = sub3(xb, xa);
                    let d_t = sub3(d, scale3(mid, dot3(mid, d)));
                    let e = normalize3(d_t);
                    // dot(v_n − v_c, ê from c to n) > 0 = separating.
                    let sep_cmyr = dot3(sub3(vb, va), e) * RADMY_TO_CMYR;
                    if sep_cmyr > CLASSIFY_CMYR {
                        any_div = true;
                    } else if sep_cmyr < -CLASSIFY_CMYR {
                        any_conv = true;
                    } else {
                        any_trans = true;
                    }
                    let both_cont = ctype[c] == 1 && ctype[nb as usize] == 1;
                    if both_cont {
                        let conv = -sep_cmyr;
                        if sep_cmyr > CLASSIFY_CMYR {
                            out.div_cont = true;
                        }
                        if out.contact_partner == NONE || conv > out.contact_conv_cmyr {
                            out.contact_partner = b;
                            out.contact_conv_cmyr = conv;
                        }
                        if conv > CLASSIFY_CMYR && conv > out.conv_cont_cmyr {
                            out.conv_cont_cmyr = conv;
                        }
                    }
                }
                // Display bits: dominant class, plus divergent presence for
                // downstream rift logic.
                if any_conv {
                    out.flags |= F_BND_CONVERGENT;
                } else if any_div {
                    out.flags |= F_BND_DIVERGENT;
                } else if any_trans {
                    out.flags |= F_BND_TRANSFORM;
                }
                if any_div {
                    out.flags |= F_BND_DIVERGENT;
                }
                out
            })
            .collect_into_vec(&mut class);

        for (c, cl) in class.iter().enumerate() {
            self.features[c] |= cl.flags;
        }
        self.class = class;
    }

    /// Integer per-plate stats for next step's motion update (serial,
    /// deterministic).
    fn accumulate_boundary_stats(&mut self) {
        let np = self.plates.len();
        self.boundary_cells = vec![0; np];
        self.subducting_cells = vec![0; np];
        self.colliding_cells = vec![0; np];
        for (c, cl) in self.class.iter().enumerate() {
            if cl.boundary {
                self.boundary_cells[self.plate_id[c] as usize] += 1;
            }
            // Colliding = continent-continent contact that is not actively
            // separating. Classification-based (not overlap events), so a
            // stalled plate keeps reading as colliding and stays stalled
            // instead of oscillating.
            if cl.contact_partner != NONE && cl.contact_conv_cmyr > -CLASSIFY_CMYR {
                self.colliding_cells[self.plate_id[c] as usize] += 1;
            }
        }
        if !self.outs.is_empty() {
            for o in &self.outs {
                if o.subducted != NONE {
                    self.subducting_cells[o.subducted as usize] += 1;
                }
            }
        }
    }

    // ----- C: events -----

    /// Volcanic arcs 150–250 km inboard of this step's trenches, on the
    /// overriding plate. Multi-source BFS from trench cells, constrained to
    /// the trench cell's (overriding) plate.
    fn apply_arcs(&mut self) {
        let ring_lo = ((ARC_MIN_KM / self.cell_spacing_km).ceil() as u16).max(1);
        let ring_hi = ((ARC_MAX_KM / self.cell_spacing_km).floor() as u16).max(ring_lo);

        let depth = &mut self.bfs_depth;
        for d in depth.iter_mut() {
            *d = u16::MAX;
        }
        let mut queue: VecDeque<u32> = VecDeque::new();
        for (c, d) in depth.iter_mut().enumerate() {
            if self.features[c] & F_TRENCH != 0 {
                *d = 0;
                queue.push_back(c as u32);
            }
        }
        while let Some(c) = queue.pop_front() {
            let dc = depth[c as usize];
            if dc >= ring_hi {
                continue;
            }
            let p = self.plate_id[c as usize];
            for &nb in self.grid.neighbors_of(c) {
                let nbu = nb as usize;
                if depth[nbu] == u16::MAX && self.plate_id[nbu] == p {
                    depth[nbu] = dc + 1;
                    queue.push_back(nb);
                }
            }
        }
        for (c, &d) in self.bfs_depth.iter().enumerate() {
            if d >= ring_lo && d <= ring_hi {
                self.features[c] |= F_ARC;
                let rate = if self.crust_type[c] == 0 {
                    ARC_GROWTH_OCEAN_KM_MY
                } else {
                    ARC_GROWTH_CONT_KM_MY
                };
                let t = &mut self.thickness[c];
                *t = (*t + rate * DT_MY).min(THICKNESS_CAP_KM);
                self.orogeny_age[c] = 0.0;
                if self.crust_type[c] == 0 && *t >= ISLAND_ARC_CONVERT_KM {
                    self.crust_type[c] = 1; // island arc: young continental crust
                    self.crust_age[c] = 0.0;
                    self.cont_gained_by_arc += 1;
                }
            }
        }
    }

    /// Continental collision thickening and rift thinning/oceanization.
    fn apply_collisions_and_rifts(&mut self) {
        let n = self.plate_id.len();
        for c in 0..n {
            let conv = self.class[c].conv_cont_cmyr;
            let collided_here = self.outs.get(c).is_some_and(|o| o.collided != NONE);
            if self.crust_type[c] == 1 && (conv > 0.0 || collided_here) {
                let rate_conv = if conv > 0.0 { conv } else { CLASSIFY_CMYR };
                let dt_thick = COLLISION_THICKEN * rate_conv * DT_MY;
                self.thickness[c] = (self.thickness[c] + dt_thick).min(THICKNESS_CAP_KM);
                self.orogeny_age[c] = 0.0;
                for &nb in self.grid.neighbors_of(c as u32) {
                    let nbu = nb as usize;
                    if self.plate_id[nbu] == self.plate_id[c] && self.crust_type[nbu] == 1 {
                        self.thickness[nbu] =
                            (self.thickness[nbu] + 0.5 * dt_thick).min(THICKNESS_CAP_KM);
                        self.orogeny_age[nbu] = 0.0;
                    }
                }
            }
        }
        // Rifting with hysteresis: sustained continent-continent divergence
        // accumulates rift_age; anything else decays it at 2× so
        // classification noise near transforms cannot mature a rift. Past
        // onset the rift thins regardless (a nucleated rift matures) until
        // decay removes it.
        for c in 0..n {
            if self.crust_type[c] != 1 {
                if self.rift_age[c] > 0.0 {
                    self.rift_age[c] = 0.0;
                }
                continue;
            }
            if self.class[c].div_cont {
                self.rift_age[c] += DT_MY;
            } else if self.rift_age[c] > 0.0 {
                self.rift_age[c] = (self.rift_age[c] - RIFT_DECAY_MULT * DT_MY).max(0.0);
            }
            if self.rift_age[c] > 0.0 {
                self.features[c] |= F_RIFT;
            }
            if self.rift_age[c] > RIFT_ONSET_MY {
                self.thickness[c] -= RIFT_THIN_KM_MY * DT_MY;
                if self.thickness[c] < RIFT_OCEANIZE_KM {
                    // The continent has rifted: new ocean floor.
                    self.cont_lost_to_rift += 1;
                    self.crust_type[c] = 0;
                    self.thickness[c] = OCEAN_THICKNESS_KM;
                    self.crust_age[c] = 0.0;
                    self.rift_age[c] = 0.0;
                    self.orogeny_age[c] = 0.0;
                    self.buildup[c] = 0.0;
                    self.features[c] = (self.features[c] & !F_RIFT) | F_RIDGE;
                }
            }
        }
    }

    /// Slow-collision timers per plate pair; suture when one has run
    /// SUTURE_AFTER_MY. Serial and id-ordered throughout. "Colliding" is
    /// continent-continent CONTACT from classification (not overlap events,
    /// which are too intermittent at slow closure to ever accumulate 30 My).
    fn update_pair_timers_and_sutures(&mut self) {
        let mut pairs: Vec<(u32, u32, f32, u32)> = Vec::new();
        for (c, cl) in self.class.iter().enumerate() {
            if cl.contact_partner == NONE {
                continue;
            }
            let a = self.plate_id[c].min(cl.contact_partner);
            let b = self.plate_id[c].max(cl.contact_partner);
            match pairs.iter_mut().find(|e| e.0 == a && e.1 == b) {
                Some(e) => {
                    e.2 += cl.contact_conv_cmyr;
                    e.3 += 1;
                }
                None => pairs.push((a, b, cl.contact_conv_cmyr, 1)),
            }
        }
        pairs.sort_by_key(|e| (e.0, e.1));

        let mut next: Vec<PairTimer> = Vec::with_capacity(pairs.len());
        for (a, b, sum, count) in &pairs {
            let mean = sum / *count as f32;
            let old = self
                .collisions
                .iter()
                .find(|t| t.a == *a && t.b == *b)
                .map(|t| t.slow_collision_my)
                .unwrap_or(0.0);
            // Slow contact accumulates — including the sub-threshold and
            // mildly-negative band, which is where jammed collisions live;
            // fast active convergence decays with hysteresis (see
            // SUTURE_DECAY_MULT), so escape flickers cannot postpone a
            // mature weld indefinitely.
            let t = if mean < SUTURE_SLOW_CMYR {
                old + DT_MY
            } else {
                (old - SUTURE_DECAY_MULT * DT_MY).max(0.0)
            };
            next.push(PairTimer {
                a: *a,
                b: *b,
                slow_collision_my: t,
            });
        }
        self.collisions = next;

        let alive = self.alive_plates();
        if alive <= PLATE_FLOOR {
            return;
        }
        let Some(idx) = self
            .collisions
            .iter()
            .position(|t| t.slow_collision_my >= SUTURE_AFTER_MY)
        else {
            return;
        };
        let (a, b) = (self.collisions[idx].a, self.collisions[idx].b);
        let (winner, loser) = if self.plate_cells[a as usize] >= self.plate_cells[b as usize] {
            (a, b)
        } else {
            (b, a)
        };
        self.suture_count += 1;
        log::debug!("t={} My: suturing plate {loser} into {winner}", self.t_my);
        for pid in self.plate_id.iter_mut() {
            if *pid == loser {
                *pid = winner;
            }
        }
        self.plate_cells[winner as usize] += self.plate_cells[loser as usize];
        self.plate_cells[loser as usize] = 0;
        self.plates[loser as usize].alive = false;
        self.plates[winner as usize].youngest_suture_my = self.t_my;
        self.collisions.retain(|t| t.a != loser && t.b != loser);
    }

    /// Supercontinent breakup: an oversized plate with an old (or no) suture
    /// splits along a great circle through its continental interior.
    fn maybe_breakup(&mut self, master_seed: u64, step_idx: u32) {
        let n = self.plate_id.len();
        if self.alive_plates() >= PLATE_CEIL {
            return;
        }
        let threshold = (n as f32 * BREAKUP_AREA_FRACTION) as u32;
        // Continental-share trigger (decision log): a plate holding over a
        // third of the world's continental crust is a supercontinent even if
        // it is under a third of the sphere — without this, a floor-6 world
        // of mutually stalled continents gridlocks forever and the Wilson
        // cycle dies.
        let mut cont_per_plate = vec![0u32; self.plates.len()];
        let mut cont_total = 0u32;
        for c in 0..n {
            if self.crust_type[c] == 1 {
                cont_per_plate[self.plate_id[c] as usize] += 1;
                cont_total += 1;
            }
        }
        let cont_threshold = (cont_total as f32 * BREAKUP_AREA_FRACTION) as u32;
        let candidate = (0..self.plates.len()).find(|&pid| {
            self.plates[pid].alive
                && (self.plate_cells[pid] > threshold
                    || (cont_total > n as u32 / 20 && cont_per_plate[pid] > cont_threshold))
                && self.t_my - self.plates[pid].youngest_suture_my > BREAKUP_SUTURE_AGE_MY
        });
        // Gridlock breaker (WO-0003 Fix 4): at the plate floor suturing is
        // blocked, so a matured slow collision would weld plates forever if
        // no supercontinent ever formed to trigger a breakup (the engine
        // death diagnosed in the decision log, 2026-08-26). Break up the
        // most-continental eligible plate instead: the one-plate window is
        // immediately spent on the oldest matured pair, which is exactly the
        // breakup↔suture limit cycle — now reachable without a
        // supercontinent, so welded continents always become one plate.
        let candidate = candidate.or_else(|| {
            let gridlocked = self.alive_plates() <= PLATE_FLOOR
                && self
                    .collisions
                    .iter()
                    .any(|t| t.slow_collision_my >= SUTURE_AFTER_MY);
            if !gridlocked {
                return None;
            }
            (0..self.plates.len())
                .filter(|&pid| {
                    self.plates[pid].alive
                        && cont_per_plate[pid] >= 32 // rift needs a continental interior
                        && self.t_my - self.plates[pid].youngest_suture_my > BREAKUP_SUTURE_AGE_MY
                })
                .max_by_key(|&pid| (cont_per_plate[pid], std::cmp::Reverse(pid)))
        });
        let Some(pid) = candidate else { return };

        // Continental centroid of the plate (serial f64 sum, fixed order).
        let mut cx = [0.0f64; 3];
        let mut count = 0u32;
        for c in 0..n {
            if self.plate_id[c] == pid as u32 && self.crust_type[c] == 1 {
                let p = self.grid.positions[c];
                cx[0] += p[0] as f64;
                cx[1] += p[1] as f64;
                cx[2] += p[2] as f64;
                count += 1;
            }
        }
        if count < 32 {
            return; // no continental interior to rift through
        }
        let len = (cx[0] * cx[0] + cx[1] * cx[1] + cx[2] * cx[2]).sqrt();
        if len < 1e-9 {
            return;
        }
        let centroid = [
            (cx[0] / len) as f32,
            (cx[1] / len) as f32,
            (cx[2] / len) as f32,
        ];

        let mut rng = sub_rng(
            master_seed,
            STAGE_ID,
            &format!("breakup-step{step_idx}-plate{pid}"),
        );
        // Great circle through the centroid: its plane normal is tangent
        // there.
        let plane_n = random_tangent(&mut rng, centroid);

        let new_id = self.plates.len() as u32;
        let old = &self.plates[pid];
        // Split the angular velocity so the halves separate across the
        // plane: omega_rel × centroid ∝ plane normal.
        let omega_old = scale3(old.pole, old.speed_deg_my * DEG2RAD);
        let push = scale3(
            cross3(centroid, plane_n),
            0.5 * BREAKUP_RIFT_SPEED * DEG2RAD,
        );
        let mk_plate = |om: [f32; 3], id: u32, fallback_pole: [f32; 3]| {
            let w = dot3(om, om).sqrt();
            let speed = (w / DEG2RAD).clamp(SPEED_MIN, SPEED_MAX);
            PlateState {
                id,
                alive: true,
                pole: if w > 1e-9 {
                    scale3(om, 1.0 / w)
                } else {
                    fallback_pole
                },
                speed_deg_my: speed,
                base_speed_deg_my: speed,
                youngest_suture_my: self.t_my, // reset the breakup clock
                pending_rot: IDENTITY3,
                pending_deg: 0.0,
                boundary_cells: 0,
                subducting_cells: 0,
                colliding_cells: 0,
            }
        };
        let fallback = old.pole;
        let plate_a = mk_plate(sub3(omega_old, push), pid as u32, fallback);
        let plate_b = mk_plate(add3(omega_old, push), new_id, fallback);
        self.plates[pid] = plate_a;
        self.plates.push(plate_b);
        self.plate_cells.push(0);
        self.boundary_cells.push(0);
        self.subducting_cells.push(0);
        self.colliding_cells.push(0);

        self.breakup_count += 1;
        log::debug!(
            "t={} My: breakup of plate {pid} -> {pid} + {new_id}",
            self.t_my
        );

        // Reassign the far side; nucleate the rift near the plane.
        let rift_halfwidth = self.cell_spacing_km / R_EARTH_KM; // ~1 cell, radians
        for c in 0..n {
            if self.plate_id[c] != pid as u32 {
                continue;
            }
            let d = dot3(self.grid.positions[c], plane_n);
            if d > 0.0 {
                self.plate_id[c] = new_id;
                self.plate_cells[new_id as usize] += 1;
                self.plate_cells[pid] -= 1;
            }
            if self.crust_type[c] == 1 && d.abs() < rift_halfwidth {
                self.features[c] |= F_RIFT;
                // Jump-start past onset so the rift matures even before the
                // divergence is classified.
                self.rift_age[c] = self.rift_age[c].max(RIFT_ONSET_MY + DT_MY);
            }
        }
    }

    fn apply_hotspots(&mut self) {
        if self.hotspot_hints.len() != self.hotspots.len() {
            self.hotspot_hints = vec![0; self.hotspots.len()];
        }
        for h in 0..self.hotspots.len() {
            let hint = self.hotspot_hints[h];
            let c = self.grid.nearest_cell(self.hotspots[h], Some(hint));
            self.hotspot_hints[h] = c;
            let cu = c as usize;
            self.buildup[cu] = (self.buildup[cu] + HOTSPOT_RATE_CENTER * DT_MY).min(HOTSPOT_CAP_KM);
            for &nb in self.grid.neighbors_of(c) {
                let nbu = nb as usize;
                self.buildup[nbu] =
                    (self.buildup[nbu] + HOTSPOT_RATE_RING * DT_MY).min(HOTSPOT_CAP_KM);
            }
        }
        for c in 0..self.buildup.len() {
            if self.buildup[c] > HOTSPOT_FLAG_KM {
                self.features[c] |= F_HOTSPOT;
            }
        }
    }

    // ----- D: aging -----

    fn age_and_relax(&mut self) {
        let thick = &mut self.thickness;
        let age = &mut self.crust_age;
        let orog = &mut self.orogeny_age;
        let build = &mut self.buildup;
        let ctype = &self.crust_type;
        thick
            .par_iter_mut()
            .zip(age.par_iter_mut())
            .zip(orog.par_iter_mut())
            .zip(build.par_iter_mut())
            .zip(ctype.par_iter())
            .for_each(|((((t, a), o), b), &ct)| {
                *a += DT_MY;
                *o += DT_MY;
                // Inactive orogens relax toward the continental base; truly
                // ancient crust (cratons, orogeny_age past the window) keeps
                // its profile.
                if ct == 1
                    && *t > OROGENY_BASE_KM
                    && *o > DT_MY * 1.5
                    && *o < OROGENY_RELAX_MAX_AGE_MY
                {
                    *t = OROGENY_BASE_KM + (*t - OROGENY_BASE_KM) * OROGENY_RELAX_FACTOR;
                }
                // Old hotspot tracks subside on the same time constant.
                *b *= OROGENY_RELAX_FACTOR;
            });
    }

    /// Snapshot the current state. Per-plate boundary stats are mirrored into
    /// the plate list so a resumed run is bit-exact.
    pub fn encode_keyframe(&self) -> Keyframe {
        let mut plates = self.plates.clone();
        for p in plates.iter_mut() {
            let i = p.id as usize;
            p.boundary_cells = self.boundary_cells[i];
            p.subducting_cells = self.subducting_cells[i];
            p.colliding_cells = self.colliding_cells[i];
        }
        Keyframe::encode(
            self.t_my,
            self.sea_offset_m,
            &self.elev,
            &self.plate_id,
            &self.crust_age,
            &self.thickness,
            &self.orogeny_age,
            &self.rift_age,
            &self.buildup,
            &self.crust_type,
            &self.features,
            plates,
            self.collisions.clone(),
        )
    }

    /// Count of alive plates.
    pub fn alive_plates(&self) -> usize {
        self.plates.iter().filter(|p| p.alive).count()
    }
}
