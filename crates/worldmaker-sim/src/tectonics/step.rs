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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use rand::RngCore;
use rayon::prelude::*;

use worldmaker_core::dmath::{
    add3, cross3, dot3, mat3_mul, mat3_mul3, mat3_transpose, normalize3, rotation3, scale3, sub3,
};
use worldmaker_core::rng::sub_rng;
use worldmaker_core::Grid;

use super::keyframe::{
    dec_suture, ActiveRift, Keyframe, MicroplateOrigin, PairTimer, PlateState, RiftDriverKind,
    SlabSegment, TectonicEvent, IDENTITY3, NEVER_SUTURED,
};
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

// ----- force balance (WO-0006, plate-physics-model.md §1) -----
// Plates are inertialess Stokes flow: speed is the quotient of driving
// forces over resistances, relaxed over the mantle re-equilibration time.
// Units: forces in cell-units such that v_target comes out in deg/My; the
// S1 values below are placeholders calibrated only to §9 metric 7's coarse
// shape (mean speed and slab ranking) — final calibration is WO-0006 S3.
/// Slab pull per attached-slab cell (age-weighted): the dominant driver.
const K_SLAB: f32 = 0.66;
/// Ridge push per divergent boundary cell (~an order below slab pull).
const K_RIDGE: f32 = 0.25;
/// Residual mantle traction per plate cell; K_MANTLE / C_DRAG is the
/// residual drift of a slab-free, ridge-free plate (~0.8 cm/yr).
const K_MANTLE: f32 = 0.09;
/// Basal drag per plate cell (the normalization of the balance).
const C_DRAG: f32 = 1.25;
/// Continent-continent contact resistance per contact cell (strength = 1.0
/// until WO-0006 S2 lands the strength field).
const C_CONTACT: f32 = 450.0;
/// Transform friction per transform-only boundary cell.
const C_TRANSFORM: f32 = 0.5;
/// Speed/pole relaxation time, My (mantle re-equilibration; India's
/// slowdown played out over ~10–20 My).
const TAU_MY: f32 = 20.0;
/// Slab-pull age weight saturates at this subduction age (half-space
/// cooling: older = colder = denser).
const SLAB_AGE_REF_MY: f32 = 80.0;
/// Safety rail only (~22 cm/yr; Cretaceous India is the fastest sustained
/// plate motion known). Not an operating point, and there is NO floor: a
/// plate whose forces vanish coasts down to the residual drift.
pub(super) const SPEED_MAX: f32 = 2.0;
/// A slab segment detaches (stops pulling) this long after it went under:
/// upper-mantle transit takes ~10–30 My and post-collision slab breakoff is
/// observed ~10–20 My after continental arrival (von Blanckenburg & Davies
/// 1995). Segments detach individually (Dan's amendment C): continuous
/// subduction keeps a rolling attached slab; pull fades only after
/// subduction stops. Detached segments are dropped after 2× this age.
pub(super) const SLAB_DETACH_MY: f32 = 100.0;
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
/// Rift timer decays at 2× real time on non-divergent steps below the onset
/// threshold (hysteresis so classification noise on quasi-transform
/// boundaries cannot mature a rift). Past onset a rift matures regardless —
/// a nucleated or failed rift keeps thinning as a scar (WO-0006 S2).
const RIFT_DECAY_MULT: f32 = 2.0;

// ----- suture (WO-0006 S2, model §3): three conditions, all sustained -----
pub(super) const SUTURE_AFTER_MY: f32 = 30.0;
/// Condition 1: continent-continent contact must span this fraction of the
/// smaller plate's perimeter (Dan's ruling, WO-0005) — a pinprick contact
/// must never weld two plates.
pub(super) const SUTURE_CONTACT_FRACTION: f32 = 0.3;
/// Condition 2: mean relative speed across the contact (cm/yr) below the
/// classification dead band — the contact is kinematically indistinguishable
/// from plate interior (Gordon 1998).
const SUTURE_LOCK_CMYR: f32 = 0.4;
/// Condition 3: every cell within this many rings of the contact on both
/// sides must be continental — suturing is the terminal act of the Wilson
/// cycle, after the intervening ocean is consumed (Wilson 1966).
const SUTURE_OCEAN_RINGS: u16 = 2;

// ----- lithosphere strength (WO-0006 S2, model §4) -----
// S(c) = S_type · g_age · g_suture · thickness penalties · g_insulation.
// Exact coefficients are S3 calibration targets; the load-bearing property
// is the ordering craton > old ocean > young continent > fresh suture/rift.
const STRENGTH_OCEAN_TYPE: f32 = 1.0;
const STRENGTH_CONT_TYPE: f32 = 0.78;
/// g_age = clamp(age_ref / 500 My, 0.2, 2.0); age_ref is crust_age on ocean
/// and min(crust_age, orogeny_age) on continent.
const STRENGTH_AGE_REF_MY: f32 = 350.0;
const STRENGTH_AGE_MIN: f32 = 0.9;
const STRENGTH_AGE_MAX: f32 = 2.0;
/// A fresh suture scores g_suture = 0.3, healing to 1.0 over 300 My
/// (sutures localize deformation for hundreds of My — Vauchez et al. 1997).
const SUTURE_WEAK_FLOOR: f32 = 0.5;
const SUTURE_HEAL_MY: f32 = 150.0;
/// Continental thickness penalty reference; over-thickened hot orogens are
/// additionally weak while young (hot crust flows).
const STRENGTH_THICK_REF_KM: f32 = 20.0;
const HOT_OROGEN_THICK_KM: f32 = 50.0;
const HOT_OROGEN_AGE_MY: f32 = 50.0;
const HOT_OROGEN_FACTOR: f32 = 1.0;
// Amendment B (Dan): supercontinent breakup comes from mantle-insulation
// weakening of the strength field. For continental cells of a plate holding
// over 1/3 of the world's continental crust, strength falls toward 0.5×,
// ramping in over 100–300 My since that plate last sutured (Gurnis 1988:
// trapped heat softens the lithosphere above).
const INSULATION_CONT_FRACTION: f32 = 1.0 / 3.0;
const INSULATION_START_MY: f32 = 100.0;
const INSULATION_FULL_MY: f32 = 300.0;
const INSULATION_FLOOR: f32 = 0.18;

// ----- rifting (WO-0006 S2, model §5 + amendment A) -----
/// A plume qualifies as a rift driver once it has sat under continental
/// crust this long (Afar / East African Rift).
const PLUME_UNDER_CONT_MY: f32 = 20.0;
/// Back-arc extension fires behind a trench whose newest slab segment
/// subducted lithosphere older than this (old = steep rollback; Uyeda &
/// Kanamori 1979), in a band this far inboard of the trench.
const BACKARC_SLAB_AGE_MY: f32 = 60.0;
const BACKARC_MIN_KM: f32 = 200.0;
const BACKARC_MAX_KM: f32 = 600.0;
/// Opposing slab pull: two pull directions at least 120° apart
/// (cos 120° = −0.5) put the plate interior in net tension.
const OPPOSING_PULL_COS: f32 = -0.5;
/// Driver stresses, compared against `strength()` (amendment A: a rift
/// nucleates and advances only where stress exceeds strength). S2
/// placeholders shaped by the §4 anchors — a plume must beat an ordinary
/// continent (~0.6) but never a craton (~2.0); back-arc stress only beats
/// weak young arc crust. Final calibration is S3.
const STRESS_PLUME: f32 = 1.0;
const STRESS_BACKARC: f32 = 0.5;
const STRESS_OPPOSING: f32 = 0.7;
/// Rift tip propagation speed (km/My): the East African Rift lengthened
/// over ~10–20 My. Converted to cells/step at the active level.
const RIFT_PROP_KM_MY: f32 = 75.0;
/// A plate that nucleated a rift (or was born of a split) cannot nucleate
/// another for this long: rifting relieves the extensional stress a driver
/// needs, and re-accumulation takes 10⁷–10⁸ yr (top of range: continental
/// breakup recurs on ~2×10⁸ yr per plate). Without this, every plume
/// re-fires the step after each failure or split and the census runs away
/// (measured: 12 → 28 plates in 200 My at L5).
const RIFT_REFRACTORY_MY: f32 = 200.0;
/// A rift needs a plate interior to cut: plates below this fraction of the
/// sphere deform instead of splitting (no nucleation, no split). Without
/// it, splits of splinters feed a runaway froth (measured: the census
/// railed at the 60-plate mask cap by 800 My at L5 and continents ground
/// away to 1% of the sphere by 2 Gy).
const MIN_RIFT_PLATE_FRACTION: f32 = 1.0 / 50.0;
/// Completed rifts whose split never materialized leave the ledger after
/// this long (attribution bookkeeping only; the scar cells stay).
const RIFT_ENTRY_PRUNE_MY: f32 = 400.0;
/// The split corridor: plate-interior ocean younger than this marks the
/// freshly oceanized rift line (and pre-existing basins stay out of it).
const CORRIDOR_MAX_AGE_MY: f32 = 60.0;
/// A split component smaller than this stays with the parent (seam noise).
const MIN_SPLIT_CELLS: u32 = 8;
/// Young-ocean boundary cells count as ridge drive even below the
/// divergence threshold: a fresh corridor's ridge swell pushes its flanks
/// apart before any divergence exists to classify (model §5: "their new
/// ridge supplies ridge push" — the bootstrap out of a fresh split).
const NASCENT_RIDGE_AGE_MY: f32 = 30.0;

// ----- microplates (WO-0006 S2, model §6) -----
/// A trench-trapped fragment must be at least this big to become a plate —
/// the larger of this floor and MICRO_MIN_FRACTION of the sphere (~3× Juan
/// de Fuca; measured at 1/300 the promotion churned ~100 microplates/Gy of
/// consumption debris and railed the census). Smaller orphans are seam
/// noise and get reassigned (§7).
const MICRO_MIN_CELLS: u32 = 12;
const MICRO_MIN_FRACTION: f32 = 1.0 / 100.0;
/// A split child below this fraction of the sphere logs as a microplate.
const MICRO_MAX_FRACTION: f32 = 0.02;
/// Alive-plate headroom guard: the advection candidate mask is a u64, so
/// fragment promotion falls back to reassignment near the limit.
const MAX_ALIVE_PLATES: usize = 60;
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
/// "No slab beneath this cell" sentinel for the per-cell `slab_plate` field.
pub const SLAB_NONE: u16 = u16::MAX;

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
    /// Crust age of the consumed cell (slab-ledger input; 0 if none).
    subducted_age: f32,
    /// Plate jammed against `plate` in continent-continent contact, or NONE.
    collided: u32,
    /// Advected slab-ledger cell fields (from the source cell).
    slab_plate: u16,
    slab_since: f32,
    /// Advected suture scar (NEVER_SUTURED where none).
    suture_at: f32,
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
    /// Full relative speed |v_other − v_self| at that partner, cm/yr — the
    /// §3 locked-kinematics input (a fast transform slide is not locked).
    contact_rel_cmyr: f32,
    /// Cell has at least one foreign neighbor.
    boundary: bool,
    /// Cell sits on a young-ocean boundary counted as ridge drive even when
    /// the divergence is below the dead band (fresh-corridor bootstrap).
    nascent_ridge: bool,
    /// This cell's contribution to its plate's drive torque: subducting
    /// edges pull toward the trench, divergent edges push off the ridge.
    torque: [f32; 3],
    /// The slab-pull part of `torque` alone (this plate subducting): the
    /// opposing-slab rift driver compares these directions.
    slab_pull: [f32; 3],
}

/// One qualifying rift driver (model §5): where a rift may nucleate this
/// step, and with how much stress.
struct RiftDriver {
    plate: u32,
    cell: u32,
    kind: RiftDriverKind,
    stress: f32,
    /// Index of the driving hotspot for a plume driver (its residence clock
    /// resets on nucleation — the plume head has vented), else u32::MAX.
    hotspot: u32,
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
    /// Plate whose slab lies beneath this cell (SLAB_NONE = none); rides
    /// with the overriding plate's cells.
    pub slab_plate: Vec<u16>,
    /// When that slab went under (My; 0 where slab_plate is SLAB_NONE).
    pub slab_since_my: Vec<f32>,
    /// When this cell last sat on a suture (NEVER_SUTURED = never): the
    /// scar that weakens the strength field and localizes later rifting.
    pub suture_at_my: Vec<f32>,

    // Plate-level state.
    pub plates: Vec<PlateState>,
    pub collisions: Vec<PairTimer>,
    pub hotspots: Vec<[f32; 3]>,
    /// Per hotspot: continuous time under continental crust (My) — the
    /// plume rift driver's clock.
    pub hotspot_cont_my: Vec<f32>,
    /// Live rift ledger (model §5): active and completed-awaiting-split.
    pub rifts: Vec<ActiveRift>,
    /// Run event log (WO-0006 S2): every suture and split with its
    /// condition or driver. Diagnostics only; never feeds the dynamics.
    pub events: Vec<TectonicEvent>,
    /// Deterministic seed for the elevation detail noise.
    pub noise_seed: u64,

    // Per-plate stats from the previous step (indexed by plate id). Mirrored
    // into PlateState at keyframe encode for bit-exact resume.
    boundary_cells: Vec<u32>,
    subducting_cells: Vec<u32>,
    colliding_cells: Vec<u32>,
    /// Σ strength(cell) over colliding contact cells (the R_bnd input).
    colliding_strength: Vec<f32>,
    ridge_cells: Vec<u32>,
    transform_cells: Vec<u32>,
    /// Summed boundary torque directions per plate (pole-update input).
    torques: Vec<[f32; 3]>,
    /// Cell count per plate id, kept current.
    plate_cells: Vec<u32>,
    /// Continental-cell census per plate and total, refreshed each step in
    /// accumulate_boundary_stats (the amendment-B insulation input).
    cont_cells_per_plate: Vec<u32>,
    cont_total_cells: u32,
    /// Rift tip advance budget per step, from RIFT_PROP_KM_MY at this level.
    rift_prop_cells: u32,

    // Reused scratch.
    cand_mask: Vec<AtomicU64>,
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
    /// Suture-condition diagnostics (WO-0006 S3 calibration): pair-steps
    /// where a continent-continent contact existed but §3 condition 1
    /// (extent), 2 (lock), or 3 (ocean closed; only evaluated when 1 and 2
    /// hold) failed. Diagnostics only — never feeds the dynamics.
    pub suture_fail_extent: u64,
    pub suture_fail_lock: u64,
    pub suture_fail_ocean: u64,
    /// Rift-to-oceanization splits (the only breakup path since S2).
    pub breakup_count: u64,
    pub rift_start_count: u64,
    pub rift_failed_count: u64,
    pub microplate_count: u64,
    /// Cells reassigned by the connectivity backstop (cumulative). The §7
    /// invariant target: this fires only for advection seam noise.
    pub connectivity_reassigned: u64,
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
            slab_plate: vec![SLAB_NONE; n],
            slab_since_my: vec![0.0; n],
            suture_at_my: vec![NEVER_SUTURED; n],
            plates: Vec::new(),
            collisions: Vec::new(),
            hotspots: Vec::new(),
            hotspot_cont_my: Vec::new(),
            rifts: Vec::new(),
            events: Vec::new(),
            noise_seed: 0,
            boundary_cells: Vec::new(),
            subducting_cells: Vec::new(),
            colliding_cells: Vec::new(),
            colliding_strength: Vec::new(),
            ridge_cells: Vec::new(),
            transform_cells: Vec::new(),
            torques: Vec::new(),
            plate_cells: Vec::new(),
            cont_cells_per_plate: Vec::new(),
            cont_total_cells: 0,
            rift_prop_cells: ((RIFT_PROP_KM_MY * DT_MY / cell_spacing_km).round() as u32).max(1),
            cand_mask: (0..n).map(|_| AtomicU64::new(0)).collect(),
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
            suture_fail_extent: 0,
            suture_fail_lock: 0,
            suture_fail_ocean: 0,
            breakup_count: 0,
            rift_start_count: 0,
            rift_failed_count: 0,
            microplate_count: 0,
            connectivity_reassigned: 0,
        }
    }

    pub fn setup(master_seed: u64, grid: &Arc<Grid>, params: &TectonicsParams) -> SimState {
        super::setup::setup(master_seed, grid, params)
    }

    /// Quantize the per-cell working state through the keyframe encoding so
    /// the state a keyframe stores IS the state the run continues from —
    /// this is what makes resume-from-keyframe bit-exact. Called right
    /// before each keyframe's elevation derive. The formulas must mirror
    /// [`Keyframe::encode`]/decode exactly. Public since WO-0006 S3 so the
    /// probe, calibration harness, and gate tests can drive `step()` with
    /// the exact keyframe cadence `run_history` uses (idempotent, safe).
    pub fn quantize_state(&mut self) {
        // Must mirror Keyframe::encode's round-then-clamp exactly.
        let q_u16 = |v: f32| -> f32 { (v.round().clamp(0.0, 65_535.0) as u16) as f32 };
        // Suture cells mirror enc_suture/dec_suture: never-sutured stays the
        // sentinel; real times saturate one step short of it.
        let q_suture = |v: f32| -> f32 {
            if v < 0.0 {
                NEVER_SUTURED
            } else {
                (v.round().clamp(0.0, 65_534.0) as u16) as f32
            }
        };
        for i in 0..self.crust_age.len() {
            self.crust_age[i] = q_u16(self.crust_age[i]);
            self.thickness[i] = q_u16(self.thickness[i] * 100.0) * 0.01;
            self.orogeny_age[i] = q_u16(self.orogeny_age[i]);
            self.rift_age[i] = q_u16(self.rift_age[i]);
            self.buildup[i] = q_u16(self.buildup[i] * 100.0) * 0.01;
            self.slab_since_my[i] = q_u16(self.slab_since_my[i]);
            self.suture_at_my[i] = q_suture(self.suture_at_my[i]);
        }
        for v in self.hotspot_cont_my.iter_mut() {
            *v = q_u16(*v);
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
            s.slab_plate[i] = kf.slab_plate[i];
            s.slab_since_my[i] = kf.slab_since_my[i] as f32;
            s.suture_at_my[i] = dec_suture(kf.suture_at_my[i]);
        }
        s.plates = kf.plates.clone();
        s.collisions = kf.collisions.clone();
        s.rifts = kf.rifts.clone();
        s.hotspots = hotspots.to_vec();
        s.hotspot_cont_my = kf.hotspot_cont_my.iter().map(|&v| v as f32).collect();
        assert_eq!(
            s.hotspot_cont_my.len(),
            s.hotspots.len(),
            "keyframe/hotspot mismatch"
        );
        s.hotspot_hints = vec![0; s.hotspots.len()];
        s.noise_seed = sub_rng(master_seed, STAGE_ID, "detail-noise").next_u64();
        // Stats travel inside PlateState — restore, don't recompute, so the
        // resumed run is bit-identical to the original.
        let np = s.plates.len();
        s.boundary_cells = vec![0; np];
        s.subducting_cells = vec![0; np];
        s.colliding_cells = vec![0; np];
        s.colliding_strength = vec![0.0; np];
        s.ridge_cells = vec![0; np];
        s.transform_cells = vec![0; np];
        s.torques = vec![[0.0; 3]; np];
        s.plate_cells = vec![0; np];
        s.cont_cells_per_plate = vec![0; np];
        for p in &s.plates {
            let i = p.id as usize;
            s.boundary_cells[i] = p.boundary_cells;
            s.subducting_cells[i] = p.subducting_cells;
            s.colliding_cells[i] = p.colliding_cells;
            s.colliding_strength[i] = p.colliding_strength;
            s.ridge_cells[i] = p.ridge_cells;
            s.transform_cells[i] = p.transform_cells;
            s.torques[i] = p.drive_torque;
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
        self.colliding_strength = vec![0.0; np];
        self.ridge_cells = vec![0; np];
        self.transform_cells = vec![0; np];
        self.torques = vec![[0.0; 3]; np];
        self.plate_cells = vec![0; np];
        self.cont_cells_per_plate = vec![0; np];
        for &p in &self.plate_id {
            self.plate_cells[p as usize] += 1;
        }
        self.classify_boundaries();
        self.accumulate_boundary_stats();
    }

    /// Advance one step. The whole step is RNG-free since WO-0006 S2 (the
    /// last draw died with the random breakup); the seed/step parameters are
    /// kept for API stability and S3's calibration hooks.
    pub fn step(&mut self, _master_seed: u64, _step_idx: u32) {
        self.motion_update();
        self.advect();
        self.enforce_connectivity();
        self.classify_boundaries();
        self.accumulate_boundary_stats();
        self.apply_arcs();
        self.apply_collisions_and_rifts();
        // Sutures read this step's classification, so they run before the
        // split pass moves any cells: every ownership change after
        // enforce_connectivity is then connectivity-preserving by
        // construction (a weld is a contact union; split halves are built
        // connected).
        self.update_pair_timers_and_sutures();
        self.check_rift_splits();
        self.grow_rifts();
        self.apply_hotspots();
        self.age_and_relax();
        self.t_my += DT_MY;
    }

    // ----- lithosphere strength (model §4 + amendment B) -----

    /// Per-cell strength S(c). The load-bearing ordering is
    /// craton > old ocean > young continent > fresh suture or rift:
    /// a 2,500 My craton scores ~2.0, old ocean ~1.0, a young continent
    /// ~0.12, a fresh suture ~0.04. Primordial continental crust (age_ref
    /// past the orogeny-relaxation window) keeps the full ocean-grade
    /// S_type — the model's own anchor puts cratons at ~2.0, which the 0.6
    /// continent factor alone cannot reach (logged decision).
    ///
    /// Reads the continental census refreshed by
    /// `accumulate_boundary_stats`, so callers must run after it each step.
    pub fn strength(&self, c: usize) -> f32 {
        let cont = self.crust_type[c] == 1;
        let age_ref = if cont {
            self.crust_age[c].min(self.orogeny_age[c])
        } else {
            self.crust_age[c]
        };
        let craton = cont && age_ref >= OROGENY_RELAX_MAX_AGE_MY;
        let s_type = if cont && !craton {
            STRENGTH_CONT_TYPE
        } else {
            STRENGTH_OCEAN_TYPE
        };
        let g_age = (age_ref / STRENGTH_AGE_REF_MY).clamp(STRENGTH_AGE_MIN, STRENGTH_AGE_MAX);
        let g_suture = SUTURE_WEAK_FLOOR
            + (1.0 - SUTURE_WEAK_FLOOR)
                * ((self.t_my - self.suture_at_my[c]) / SUTURE_HEAL_MY).clamp(0.0, 1.0);
        let mut s = s_type * g_age * g_suture;
        if cont {
            s *= (self.thickness[c] / STRENGTH_THICK_REF_KM).clamp(0.5, 1.0);
            if self.thickness[c] > HOT_OROGEN_THICK_KM && self.orogeny_age[c] < HOT_OROGEN_AGE_MY {
                s *= HOT_OROGEN_FACTOR;
            }
            // Amendment B: mantle insulation under a supercontinental plate
            // weakens its continental lithosphere toward 0.5×, ramping in
            // 100–300 My after that plate last sutured.
            let pid = self.plate_id[c] as usize;
            if self.cont_total_cells > 0
                && self.cont_cells_per_plate[pid] as f32
                    > self.cont_total_cells as f32 * INSULATION_CONT_FRACTION
            {
                let dt = self.t_my - self.plates[pid].youngest_suture_my;
                let ramp = ((dt - INSULATION_START_MY)
                    / (INSULATION_FULL_MY - INSULATION_START_MY))
                    .clamp(0.0, 1.0);
                s *= 1.0 - (1.0 - INSULATION_FLOOR) * ramp;
            }
        }
        s
    }

    // ----- F: plate motion (WO-0006 force balance, model §1) -----

    /// Inertialess force balance: v_target = drivers / resistances, relaxed
    /// over TAU_MY; the pole relaxes toward the summed boundary torque
    /// direction. RNG-free — poles wander exactly when the plate's boundary
    /// makeup changes.
    fn motion_update(&mut self) {
        for pid in 0..self.plates.len() {
            if !self.plates[pid].alive {
                continue;
            }
            // Slab pull from attached ledger segments, weighted by thermal
            // age at subduction (serial, fixed segment order).
            let mut slab_weighted = 0.0f32;
            for seg in &self.plates[pid].slab {
                if seg.attached {
                    slab_weighted += seg.area_cells as f32
                        * (seg.age_at_subduction_my / SLAB_AGE_REF_MY).min(1.0);
                }
            }
            let area = self.plate_cells[pid].max(1) as f32;
            let f_slab = K_SLAB * slab_weighted;
            let f_ridge = K_RIDGE * self.ridge_cells[pid] as f32;
            let f_resid = K_MANTLE * area;
            let r_drag = C_DRAG * area;
            // Boundary resistance is strength-weighted (model §4): a contact
            // with a craton resists harder than one with a fresh suture.
            let r_bnd = C_CONTACT * self.colliding_strength[pid]
                + C_TRANSFORM * self.transform_cells[pid] as f32;
            let v_target = (f_slab + f_ridge + f_resid) / (r_drag + r_bnd);
            let torque = self.torques[pid];
            let p = &mut self.plates[pid];
            p.speed_deg_my =
                (p.speed_deg_my + (DT_MY / TAU_MY) * (v_target - p.speed_deg_my)).min(SPEED_MAX);
            // Pole: relax toward the boundary-torque direction. A plate with
            // no boundary drivers keeps its pole (nothing is steering it).
            let len = dot3(torque, torque).sqrt();
            if len > 1e-12 {
                let omega_target = scale3(torque, 1.0 / len);
                p.pole = normalize3(add3(
                    p.pole,
                    scale3(sub3(omega_target, p.pole), DT_MY / TAU_MY),
                ));
            }
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
        assert!(nd <= 64, "alive plate count {nd} exceeds candidate mask");

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
            let bit = 1u64 << d;
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
        let prev_slab_plate = &self.slab_plate;
        let prev_slab_since = &self.slab_since_my;
        let prev_suture = &self.suture_at_my;
        let inv_ref = &inv;
        let id_of_dense_ref = &id_of_dense;
        let dense_of_id_ref = &dense_of_id;
        // Plate speeds for the continent-continent overlap rule: the cell
        // resolves to the slower plate (§7 cause removal — no frozen cells).
        let speeds: Vec<f32> = self.plates.iter().map(|p| p.speed_deg_my).collect();
        let speeds_ref = &speeds;

        // Copy of a cell's previous state, used for no-op cells.
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
            subducted_age: 0.0,
            collided,
            slab_plate: prev_slab_plate[c],
            slab_since: prev_slab_since[c],
            suture_at: prev_suture[c],
        };

        let mut outs = std::mem::take(&mut self.outs);
        (0..n)
            .into_par_iter()
            .map(|c| {
                let x = grid.positions[c];
                let mut mask = cand[c].load(Ordering::Relaxed);
                mask |= 1u64 << dense_of_id_ref[plate_id[c] as usize];
                for &nb in grid.neighbors_of(c as u32) {
                    mask |= 1u64 << dense_of_id_ref[plate_id[nb as usize] as usize];
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
                                subducted_age: 0.0,
                                collided: NONE,
                                slab_plate: SLAB_NONE,
                                slab_since: 0.0,
                                suture_at: NEVER_SUTURED,
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
                            subducted_age: 0.0,
                            collided: NONE,
                            slab_plate: prev_slab_plate[s],
                            slab_since: prev_slab_since[s],
                            suture_at: prev_suture[s],
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
                            // Continent-continent overlap: the cell resolves
                            // to the SLOWER hard plate (§7 cause removal —
                            // rigid plates cannot shed frozen cells; the
                            // force balance is what slows the plates). Both
                            // sides still record the collision for orogeny.
                            let mut win = usize::MAX;
                            for i in 0..covers {
                                if !is_hard(i) {
                                    continue;
                                }
                                if win == usize::MAX {
                                    win = i;
                                    continue;
                                }
                                let (sw, si) = (
                                    speeds_ref[cover_plate[win] as usize],
                                    speeds_ref[cover_plate[i] as usize],
                                );
                                if si < sw || (si == sw && cover_plate[i] < cover_plate[win]) {
                                    win = i;
                                }
                            }
                            let other = (0..covers)
                                .find(|&i| is_hard(i) && i != win)
                                .map(|i| cover_plate[i])
                                .unwrap_or(NONE);
                            let s = cover_src[win] as usize;
                            CellOut {
                                plate: cover_plate[win],
                                ctype: prev_ctype[s],
                                features: 0,
                                age: prev_age[s],
                                thick: prev_thick[s],
                                orog: prev_orog[s],
                                rift: prev_rift[s],
                                build: prev_build[s],
                                subducted: NONE,
                                subducted_age: 0.0,
                                collided: other,
                                slab_plate: prev_slab_plate[s],
                                slab_since: prev_slab_since[s],
                                suture_at: prev_suture[s],
                            }
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
                            // the slab ledger (multi-way overlaps are rare).
                            let loser = (0..covers).find(|&i| i != win).unwrap();
                            let s = cover_src[win] as usize;
                            let (subducted, subducted_age, features) = if was_transform_only {
                                (NONE, 0.0, 0) // transform jitter: no trench
                            } else {
                                (
                                    cover_plate[loser],
                                    prev_age[cover_src[loser] as usize],
                                    F_TRENCH,
                                )
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
                                subducted_age,
                                collided: NONE,
                                slab_plate: prev_slab_plate[s],
                                slab_since: prev_slab_since[s],
                                suture_at: prev_suture[s],
                            }
                        }
                    }
                }
            })
            .collect_into_vec(&mut outs);

        // Scatter into the SoA arrays and refresh plate cell counts. Slab
        // ledger: consumption this step is merged into one segment per
        // consumed plate (serial, id-ordered — deterministic), and the
        // overriding plate's trench cell records whose slab went under.
        for v in self.plate_cells.iter_mut() {
            *v = 0;
        }
        let np = self.plates.len();
        let mut consumed_cells = vec![0u32; np];
        let mut consumed_age_sum = vec![0.0f32; np];
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
            self.suture_at_my[c] = o.suture_at;
            self.plate_cells[o.plate as usize] += 1;
            if o.subducted != NONE {
                consumed_cells[o.subducted as usize] += 1;
                consumed_age_sum[o.subducted as usize] += o.subducted_age;
                // The slab hangs under the margin it subducted beneath.
                self.slab_plate[c] = o.subducted as u16;
                self.slab_since_my[c] = self.t_my;
            } else {
                self.slab_plate[c] = o.slab_plate;
                self.slab_since_my[c] = o.slab_since;
            }
        }
        for pid in 0..np {
            if consumed_cells[pid] > 0 {
                self.plates[pid].slab.push(SlabSegment {
                    area_cells: consumed_cells[pid],
                    age_at_subduction_my: consumed_age_sum[pid] / consumed_cells[pid] as f32,
                    subducted_at_my: self.t_my,
                    attached: true,
                });
            }
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
                // Slabs keep pulling after the subducting plate dies (Dan's
                // ruling): the ledger transfers to the plate that consumed
                // most of it this step (tie → lowest plate id).
                let mut best: Option<(u32, u32)> = None; // (consumer, count)
                let mut counts = vec![0u32; self.plates.len()];
                for o in &self.outs {
                    if o.subducted == pid as u32 {
                        counts[o.plate as usize] += 1;
                    }
                }
                for (consumer, &cnt) in counts.iter().enumerate() {
                    if cnt > 0 && best.is_none_or(|(_, b)| cnt > b) {
                        best = Some((consumer as u32, cnt));
                    }
                }
                if let Some((consumer, _)) = best {
                    let segs = std::mem::take(&mut self.plates[pid].slab);
                    self.plates[consumer as usize].slab.extend(segs);
                }
                log::debug!("t={} My: plate {pid} fully consumed", self.t_my);
            }
        }
    }

    /// Append a new plate inheriting `pole`/`speed`/event clocks, with an
    /// empty slab ledger (model §6: a new plate lives or dies by its own
    /// force balance), extending every per-plate array. Returns its id.
    fn spawn_plate(
        &mut self,
        pole: [f32; 3],
        speed: f32,
        youngest_suture_my: f32,
        youngest_rift_my: f32,
    ) -> u32 {
        let id = self.plates.len() as u32;
        self.plates.push(PlateState {
            id,
            alive: true,
            pole,
            speed_deg_my: speed,
            youngest_suture_my,
            youngest_rift_my,
            pending_rot: IDENTITY3,
            pending_deg: 0.0,
            slab: Vec::new(),
            boundary_cells: 0,
            subducting_cells: 0,
            colliding_cells: 0,
            colliding_strength: 0.0,
            ridge_cells: 0,
            transform_cells: 0,
            drive_torque: [0.0; 3],
        });
        self.plate_cells.push(0);
        self.boundary_cells.push(0);
        self.subducting_cells.push(0);
        self.colliding_cells.push(0);
        self.colliding_strength.push(0.0);
        self.ridge_cells.push(0);
        self.transform_cells.push(0);
        self.torques.push([0.0; 3]);
        self.cont_cells_per_plate.push(0);
        id
    }

    /// §7 invariant: every alive plate is one connected region, every step.
    /// Serial BFS in cell-id order; per plate the largest component is kept
    /// (tie → the component holding the lowest cell id, i.e. the earliest
    /// discovered); every other fragment is reassigned to the neighbor plate
    /// sharing the longest border with it (tie → lowest plate id). Targets
    /// are chosen from pre-pass ownership, so the sweep is order-free.
    ///
    /// One §6 exception: a sizable fragment pressed against an active trench
    /// is a trench-trapped slice (Farallon → Juan de Fuca) and becomes a
    /// microplate instead of being absorbed. Orphan seam noise (below
    /// MICRO_MIN_CELLS) is never a microplate.
    ///
    /// The sweep runs to a fixpoint: because every fragment's target comes
    /// from pre-pass ownership, two adjacent fragments can strand each
    /// other (F1 joins Q through F2's cells while F2 leaves Q — found via
    /// the S2 probe; the single-pass version shipped in S1 with the same
    /// composition gap). One repeat normally suffices.
    fn enforce_connectivity(&mut self) {
        for _ in 0..8 {
            if !self.connectivity_pass() {
                return;
            }
        }
        log::warn!("t={} My: connectivity sweep did not converge", self.t_my);
    }

    /// One §7 sweep. Returns true if any fragment was found (another pass
    /// should re-check the result).
    fn connectivity_pass(&mut self) -> bool {
        let n = self.grid.cell_count() as usize;
        // Component labeling in cell-id order.
        let mut comp_of = vec![u32::MAX; n];
        let mut comp_plate: Vec<u32> = Vec::new();
        let mut comp_size: Vec<u32> = Vec::new();
        let mut queue: VecDeque<u32> = VecDeque::new();
        for c0 in 0..n {
            if comp_of[c0] != u32::MAX {
                continue;
            }
            let p = self.plate_id[c0];
            let ci = comp_plate.len() as u32;
            comp_plate.push(p);
            comp_size.push(0);
            comp_of[c0] = ci;
            queue.push_back(c0 as u32);
            while let Some(c) = queue.pop_front() {
                comp_size[ci as usize] += 1;
                for &nb in self.grid.neighbors_of(c) {
                    let nbu = nb as usize;
                    if comp_of[nbu] == u32::MAX && self.plate_id[nbu] == p {
                        comp_of[nbu] = ci;
                        queue.push_back(nb);
                    }
                }
            }
        }
        // Keeper per plate: largest component; ties resolve to the earliest
        // discovered, which holds the lowest cell id (strict > keeps it).
        let mut keep = vec![u32::MAX; self.plates.len()];
        for ci in 0..comp_plate.len() {
            let p = comp_plate[ci] as usize;
            if keep[p] == u32::MAX || comp_size[ci] > comp_size[keep[p] as usize] {
                keep[p] = ci as u32;
            }
        }
        // Reassign each fragment to the pre-pass neighbor plate with the
        // longest shared border (tie → lowest plate id). One pass gathers
        // fragment cell lists; borders are then counted per fragment.
        let mut frag_cells: Vec<Vec<u32>> = vec![Vec::new(); comp_plate.len()];
        for (c, &ci) in comp_of.iter().enumerate() {
            if keep[comp_plate[ci as usize] as usize] != ci {
                frag_cells[ci as usize].push(c as u32);
            }
        }
        let any_fragment = frag_cells.iter().any(|c| !c.is_empty());
        let mut target = vec![u32::MAX; comp_plate.len()];
        let mut alive = self.alive_plates();
        for ci in 0..comp_plate.len() {
            let cells = &frag_cells[ci];
            if cells.is_empty() {
                continue;
            }
            let p = comp_plate[ci];
            // §6 trench-trapped slice: a big, purely oceanic fragment
            // against an active trench becomes its own plate (inheriting
            // the parent's motion; the force balance owns it from the next
            // step). Continental fragments are collision debris and get
            // reassigned like any orphan.
            let micro_min =
                MICRO_MIN_CELLS.max((self.plate_id.len() as f32 * MICRO_MIN_FRACTION) as u32);
            if cells.len() as u32 >= micro_min && alive < MAX_ALIVE_PLATES {
                let oceanic = cells.iter().all(|&c| self.crust_type[c as usize] == 0);
                // The Farallon signature: the slice still holds young crust
                // from the ridge the trench just consumed. Old interior
                // ocean shorn off a plate is ordinary debris, not a
                // Juan de Fuca remnant.
                let had_ridge = cells
                    .iter()
                    .any(|&c| self.crust_age[c as usize] < CORRIDOR_MAX_AGE_MY);
                let against_trench = oceanic
                    && had_ridge
                    && cells.iter().any(|&c| {
                        self.features[c as usize] & F_TRENCH != 0
                            || self
                                .grid
                                .neighbors_of(c)
                                .iter()
                                .any(|&nb| self.features[nb as usize] & F_TRENCH != 0)
                    });
                if against_trench {
                    let parent = &self.plates[p as usize];
                    let (pole, speed, ys, yr) = (
                        parent.pole,
                        parent.speed_deg_my,
                        parent.youngest_suture_my,
                        parent.youngest_rift_my,
                    );
                    let id = self.spawn_plate(pole, speed, ys, yr);
                    for &c in &frag_cells[ci] {
                        self.plate_cells[self.plate_id[c as usize] as usize] -= 1;
                        self.plate_cells[id as usize] += 1;
                        self.plate_id[c as usize] = id;
                    }
                    self.events.push(TectonicEvent::Microplate {
                        id,
                        origin: MicroplateOrigin::TrenchTrapped,
                        t: self.t_my,
                    });
                    self.microplate_count += 1;
                    alive += 1;
                    log::debug!(
                        "t={} My: trench-trapped microplate {id} off plate {p} ({} cells)",
                        self.t_my,
                        frag_cells[ci].len()
                    );
                    continue;
                }
            }
            let cells = &frag_cells[ci];
            // Sized fresh: a promotion above may have grown the plate list.
            let mut border: Vec<u32> = vec![0; self.plates.len()];
            for &c in cells {
                for &nb in self.grid.neighbors_of(c) {
                    let q = self.plate_id[nb as usize];
                    if q != p {
                        border[q as usize] += 1;
                    }
                }
            }
            let mut best = u32::MAX;
            for (q, &cnt) in border.iter().enumerate() {
                if cnt > 0 && (best == u32::MAX || cnt > border[best as usize]) {
                    best = q as u32;
                }
            }
            target[ci] = best;
        }
        for (c, &ci) in comp_of.iter().enumerate() {
            let t = target[ci as usize];
            if t != u32::MAX {
                self.plate_cells[self.plate_id[c] as usize] -= 1;
                self.plate_cells[t as usize] += 1;
                self.plate_id[c] = t;
                self.connectivity_reassigned += 1;
            }
        }
        any_fragment
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
        let thick = &self.thickness;
        let age = &self.crust_age;

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
                    let rel = sub3(vb, va);
                    let rel_cmyr = dot3(rel, rel).sqrt() * RADMY_TO_CMYR;
                    // dot(v_n − v_c, ê from c to n) > 0 = separating.
                    let sep_cmyr = dot3(rel, e) * RADMY_TO_CMYR;
                    if sep_cmyr > CLASSIFY_CMYR {
                        any_div = true;
                        // Ridge push: this plate slides AWAY from the ridge
                        // (torque sense opposite the boundary direction).
                        let t = cross3(xa, e);
                        out.torque = sub3(out.torque, t);
                    } else if sep_cmyr < -CLASSIFY_CMYR {
                        any_conv = true;
                        // Slab pull steers the SUBDUCTING side toward its
                        // trench. Which side subducts mirrors the advection
                        // overlap rule: hard continental crust overrides;
                        // between soft crusts the older (denser) goes under
                        // (tie → the higher plate id, since the lower id
                        // wins the override).
                        let hard_a = ctype[c] == 1 && thick[c] >= SUBDUCTIBLE_CONT_KM;
                        let hard_b =
                            ctype[nb as usize] == 1 && thick[nb as usize] >= SUBDUCTIBLE_CONT_KM;
                        let a_subducts = if hard_a || hard_b {
                            !hard_a && hard_b
                        } else {
                            age[c] > age[nb as usize] || (age[c] == age[nb as usize] && a > b)
                        };
                        if a_subducts {
                            let t = cross3(xa, e);
                            out.torque = add3(out.torque, t);
                            out.slab_pull = add3(out.slab_pull, t);
                        }
                    } else {
                        any_trans = true;
                        // Nascent ridge: young ocean flanking a quiet
                        // boundary still carries a ridge swell, so it drives
                        // (ridge_cells + torque) without being classified
                        // divergent — the bootstrap that lets a fresh split
                        // corridor push its halves apart.
                        if ctype[c] == 0
                            && ctype[nb as usize] == 0
                            && age[c].min(age[nb as usize]) < NASCENT_RIDGE_AGE_MY
                        {
                            out.nascent_ridge = true;
                            out.torque = sub3(out.torque, cross3(xa, e));
                        }
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
                            out.contact_rel_cmyr = rel_cmyr;
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

    /// Per-plate stats for next step's motion update (serial, cell-id
    /// ordered — the one f32 reduction, colliding_strength, is deterministic
    /// by fixed order).
    fn accumulate_boundary_stats(&mut self) {
        let np = self.plates.len();
        self.boundary_cells = vec![0; np];
        self.subducting_cells = vec![0; np];
        self.colliding_cells = vec![0; np];
        self.colliding_strength = vec![0.0; np];
        self.ridge_cells = vec![0; np];
        self.transform_cells = vec![0; np];
        self.torques = vec![[0.0; 3]; np];
        // Continental census first: strength()'s insulation factor reads it.
        self.cont_cells_per_plate = vec![0; np];
        self.cont_total_cells = 0;
        for (c, &p) in self.plate_id.iter().enumerate() {
            if self.crust_type[c] == 1 {
                self.cont_cells_per_plate[p as usize] += 1;
                self.cont_total_cells += 1;
            }
        }
        for c in 0..self.class.len() {
            let cl = self.class[c];
            let p = self.plate_id[c] as usize;
            if cl.boundary {
                self.boundary_cells[p] += 1;
            }
            // Force-balance inputs: divergent cells drive ridge push
            // (nascent young-ocean ridges count too), transform-only cells
            // resist, and the per-cell torque contributions sum in cell-id
            // order (deterministic).
            if cl.flags & F_BND_DIVERGENT != 0 || cl.nascent_ridge {
                self.ridge_cells[p] += 1;
            }
            if cl.flags & F_BND_TRANSFORM != 0 {
                self.transform_cells[p] += 1;
            }
            self.torques[p] = add3(self.torques[p], cl.torque);
            // Colliding = continent-continent contact that is not actively
            // separating, weighted by the strength of the resisting crust
            // (model §4). Classification-based (not overlap events), so a
            // stalled plate keeps reading as colliding and stays stalled
            // instead of oscillating.
            if cl.contact_partner != NONE && cl.contact_conv_cmyr > -CLASSIFY_CMYR {
                self.colliding_cells[p] += 1;
                self.colliding_strength[p] += self.strength(c);
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
        // The BFS runs out to the back-arc band's far edge: rift_drivers()
        // reads the same depth field later this step.
        let ring_max = ((BACKARC_MAX_KM / self.cell_spacing_km).floor() as u16).max(ring_hi);

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
            if dc >= ring_max {
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
        // Rifting with hysteresis below onset: sustained continent-continent
        // divergence accumulates rift_age; anything else decays it at 2× so
        // classification noise near transforms cannot mature a rift. PAST
        // onset a rift matures unconditionally (WO-0006 S2): nucleated rift
        // paths are jump-started past onset, and a stalled (failed) rift
        // keeps accumulating toward oceanization as a scar.
        for c in 0..n {
            if self.crust_type[c] != 1 {
                if self.rift_age[c] > 0.0 {
                    self.rift_age[c] = 0.0;
                }
                continue;
            }
            if self.class[c].div_cont || self.rift_age[c] > RIFT_ONSET_MY {
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
                    self.suture_at_my[c] = NEVER_SUTURED; // the scar reopened
                    self.features[c] = (self.features[c] & !F_RIFT) | F_RIDGE;
                }
            }
        }
    }

    /// §3 suture: a pair timer accumulates only while ALL THREE conditions
    /// hold — (1) continent-continent contact along ≥ 30% of the smaller
    /// plate's perimeter, (2) mean relative speed across the contact below
    /// SUTURE_LOCK_CMYR, (3) every cell within 2 rings of the contact on
    /// both sides continental. Any lapse resets the timer ("sustained" is
    /// literal). At SUTURE_AFTER_MY the pair welds: smaller merges into
    /// larger and every contact cell records the suture scar. Serial and
    /// id-ordered throughout.
    fn update_pair_timers_and_sutures(&mut self) {
        struct PairAcc {
            a: u32,
            b: u32,
            /// Contact cells on the a / b side respectively.
            cells_a: u32,
            cells_b: u32,
            rel_sum: f32,
            rel_n: u32,
            contact_cells: Vec<u32>,
        }
        let mut pairs: Vec<PairAcc> = Vec::new();
        for (c, cl) in self.class.iter().enumerate() {
            if cl.contact_partner == NONE {
                continue;
            }
            let p = self.plate_id[c];
            let (a, b) = (p.min(cl.contact_partner), p.max(cl.contact_partner));
            let e = match pairs.iter_mut().find(|e| e.a == a && e.b == b) {
                Some(e) => e,
                None => {
                    pairs.push(PairAcc {
                        a,
                        b,
                        cells_a: 0,
                        cells_b: 0,
                        rel_sum: 0.0,
                        rel_n: 0,
                        contact_cells: Vec::new(),
                    });
                    pairs.last_mut().unwrap()
                }
            };
            if p == a {
                e.cells_a += 1;
            } else {
                e.cells_b += 1;
            }
            e.rel_sum += cl.contact_rel_cmyr;
            e.rel_n += 1;
            e.contact_cells.push(c as u32);
        }
        pairs.sort_by_key(|e| (e.a, e.b));

        let mut next: Vec<PairTimer> = Vec::with_capacity(pairs.len());
        let mut matured: Option<usize> = None;
        for (i, e) in pairs.iter().enumerate() {
            // Condition 1: contact extent on the smaller plate's perimeter.
            let (small, small_contact) =
                if self.plate_cells[e.a as usize] <= self.plate_cells[e.b as usize] {
                    (e.a, e.cells_a)
                } else {
                    (e.b, e.cells_b)
                };
            let perimeter = self.boundary_cells[small as usize].max(1);
            let extent_ok = small_contact as f32 >= SUTURE_CONTACT_FRACTION * perimeter as f32;
            // Condition 2: kinematically locked.
            let locked = (e.rel_sum / e.rel_n.max(1) as f32) < SUTURE_LOCK_CMYR;
            // Condition 3 (checked last — it walks rings): ocean closed.
            let holds = extent_ok && locked && self.ocean_closed(&e.contact_cells, e.a, e.b);
            // Which condition binds (calibration diagnostics; ocean is only
            // known when 1 and 2 hold, matching the short-circuit).
            if !extent_ok {
                self.suture_fail_extent += 1;
            }
            if !locked {
                self.suture_fail_lock += 1;
            }
            if extent_ok && locked && !holds {
                self.suture_fail_ocean += 1;
            }
            let old = self
                .collisions
                .iter()
                .find(|t| t.a == e.a && t.b == e.b)
                .map(|t| t.slow_collision_my)
                .unwrap_or(0.0);
            let t = if holds { old + DT_MY } else { 0.0 };
            if t >= SUTURE_AFTER_MY && matured.is_none() {
                matured = Some(i);
            }
            next.push(PairTimer {
                a: e.a,
                b: e.b,
                slow_collision_my: t,
            });
        }
        self.collisions = next;

        // One suture per step: the first matured pair in (a, b) order.
        let Some(idx) = matured else { return };
        let e = &pairs[idx];
        let (a, b) = (e.a, e.b);
        let (winner, loser) = if self.plate_cells[a as usize] >= self.plate_cells[b as usize] {
            (a, b)
        } else {
            (b, a)
        };
        let (small_contact, small) = if loser == a {
            (e.cells_a, a)
        } else {
            (e.cells_b, b)
        };
        let contact_fraction =
            small_contact as f32 / self.boundary_cells[small as usize].max(1) as f32;
        self.suture_count += 1;
        self.events.push(TectonicEvent::Suture {
            a: winner,
            b: loser,
            t: self.t_my,
            contact_fraction,
        });
        log::debug!("t={} My: suturing plate {loser} into {winner}", self.t_my);
        // The scar is data: every contact cell (both sides) records the weld.
        for &c in &e.contact_cells {
            self.suture_at_my[c as usize] = self.t_my;
        }
        for pid in self.plate_id.iter_mut() {
            if *pid == loser {
                *pid = winner;
            }
        }
        self.plate_cells[winner as usize] += self.plate_cells[loser as usize];
        self.plate_cells[loser as usize] = 0;
        self.cont_cells_per_plate[winner as usize] += self.cont_cells_per_plate[loser as usize];
        self.cont_cells_per_plate[loser as usize] = 0;
        self.plates[loser as usize].alive = false;
        self.plates[winner as usize].youngest_suture_my = self.t_my;
        // The merged plate inherits the loser's slab ledger (slabs keep
        // sinking under the weld) and its live rifts (the scar cells are
        // its cells now).
        let segs = std::mem::take(&mut self.plates[loser as usize].slab);
        self.plates[winner as usize].slab.extend(segs);
        for r in self.rifts.iter_mut() {
            if r.plate == loser {
                r.plate = winner;
            }
        }
        self.collisions.retain(|t| t.a != loser && t.b != loser);
    }

    /// §3 condition 3: no ocean within SUTURE_OCEAN_RINGS rings of the
    /// contact on either plate. Serial BFS seeded in cell-id order.
    fn ocean_closed(&self, contact_cells: &[u32], a: u32, b: u32) -> bool {
        let n = self.grid.cell_count() as usize;
        let mut depth = vec![u16::MAX; n];
        let mut queue: VecDeque<u32> = VecDeque::new();
        for &c in contact_cells {
            if self.crust_type[c as usize] == 0 {
                return false;
            }
            depth[c as usize] = 0;
            queue.push_back(c);
        }
        while let Some(c) = queue.pop_front() {
            let dc = depth[c as usize];
            if dc >= SUTURE_OCEAN_RINGS {
                continue;
            }
            for &nb in self.grid.neighbors_of(c) {
                let nbu = nb as usize;
                let p = self.plate_id[nbu];
                if depth[nbu] == u16::MAX && (p == a || p == b) {
                    if self.crust_type[nbu] == 0 {
                        return false;
                    }
                    depth[nbu] = dc + 1;
                    queue.push_back(nb);
                }
            }
        }
        true
    }

    // ----- rifting (model §5 + amendment A) and splits -----

    /// The physical rift drivers present this step, in fixed evaluation
    /// order: plumes (hotspot index order), back-arc bands (plate id
    /// order), opposing slab pull (plate id order). No RNG, no plate-count
    /// trigger, no area quota.
    fn rift_drivers(&self) -> Vec<RiftDriver> {
        let mut out = Vec::new();
        // Plume under continent ≥ 20 My.
        for h in 0..self.hotspots.len() {
            if self.hotspot_cont_my[h] < PLUME_UNDER_CONT_MY {
                continue;
            }
            let c = self
                .grid
                .nearest_cell(self.hotspots[h], Some(self.hotspot_hints[h]));
            if self.crust_type[c as usize] != 1 {
                continue;
            }
            out.push(RiftDriver {
                plate: self.plate_id[c as usize],
                cell: c,
                kind: RiftDriverKind::Plume,
                stress: STRESS_PLUME,
                hotspot: h as u32,
            });
        }
        // Back-arc band 200–600 km inboard of a trench consuming old
        // lithosphere: nucleates at the band's weakest cell. Reuses this
        // step's trench-distance BFS (apply_arcs).
        let band_lo = ((BACKARC_MIN_KM / self.cell_spacing_km).ceil() as u16).max(1);
        let band_hi = ((BACKARC_MAX_KM / self.cell_spacing_km).floor() as u16).max(band_lo);
        for pid in 0..self.plates.len() {
            if !self.plates[pid].alive {
                continue;
            }
            let Some(newest) = self.plates[pid].slab.last() else {
                continue;
            };
            if !(newest.attached && newest.age_at_subduction_my > BACKARC_SLAB_AGE_MY) {
                continue;
            }
            let mut best: Option<(u32, f32)> = None;
            for (c, &d) in self.bfs_depth.iter().enumerate() {
                if d < band_lo || d > band_hi || self.plate_id[c] != pid as u32 {
                    continue;
                }
                let s = self.strength(c);
                if best.is_none_or(|(_, bs)| s < bs) {
                    best = Some((c as u32, s));
                }
            }
            if let Some((cell, _)) = best {
                out.push(RiftDriver {
                    plate: pid as u32,
                    cell,
                    kind: RiftDriverKind::BackArc,
                    stress: STRESS_BACKARC,
                    hotspot: u32::MAX,
                });
            }
        }
        // Opposing slab pull: two subducting-edge groups with pull
        // directions ≥ 120° apart put the interior in tension; the rift
        // nucleates at the plate's weakest continental cell.
        for pid in 0..self.plates.len() {
            if !self.plates[pid].alive || !self.plates[pid].slab.iter().any(|s| s.attached) {
                continue;
            }
            let mut dirs: Vec<[f32; 3]> = Vec::new();
            for (c, cl) in self.class.iter().enumerate() {
                if self.plate_id[c] != pid as u32 {
                    continue;
                }
                let m2 = dot3(cl.slab_pull, cl.slab_pull);
                if m2 > 1e-12 {
                    dirs.push(scale3(cl.slab_pull, 1.0 / m2.sqrt()));
                }
            }
            let opposing = dirs.iter().enumerate().any(|(i, u)| {
                dirs[i + 1..]
                    .iter()
                    .any(|v| dot3(*u, *v) <= OPPOSING_PULL_COS)
            });
            if !opposing {
                continue;
            }
            let mut best: Option<(u32, f32)> = None;
            for c in 0..self.plate_id.len() {
                if self.plate_id[c] != pid as u32 || self.crust_type[c] != 1 {
                    continue;
                }
                let s = self.strength(c);
                if best.is_none_or(|(_, bs)| s < bs) {
                    best = Some((c as u32, s));
                }
            }
            if let Some((cell, _)) = best {
                out.push(RiftDriver {
                    plate: pid as u32,
                    cell,
                    kind: RiftDriverKind::OpposingSlabs,
                    stress: STRESS_OPPOSING,
                    hotspot: u32::MAX,
                });
            }
        }
        out
    }

    /// Amendment A rift life cycle. Existing rifts advance each tip up to
    /// `rift_prop_cells` cells along the neighbor of least strength, only
    /// while stress > strength(next); a tip that reaches the plate boundary
    /// is done; a rift that cannot advance stalls and is recorded as failed
    /// (its cells keep maturing as a scar). Then new rifts nucleate at
    /// driver cells whose stress exceeds local strength — one live rift per
    /// plate. All walks are id-ordered and RNG-free.
    fn grow_rifts(&mut self) {
        let jump_start = RIFT_ONSET_MY + DT_MY;
        let mut rifts = std::mem::take(&mut self.rifts);
        let mut keep = vec![true; rifts.len()];
        for (ri, r) in rifts.iter_mut().enumerate() {
            if !self.plates[r.plate as usize].alive {
                keep[ri] = false; // plate consumed or merged away
                continue;
            }
            if r.done_a && r.done_b {
                // Completed: waiting for its corridor to oceanize and split
                // the plate (check_rift_splits removes it then). Prune if
                // the split never materializes.
                if self.t_my - r.started_my > RIFT_ENTRY_PRUNE_MY {
                    keep[ri] = false;
                }
                continue;
            }
            let mut failed = false;
            for tip_b in [false, true] {
                if failed || (!tip_b && r.done_a) || (tip_b && r.done_b) {
                    continue;
                }
                let tip = if tip_b { &mut r.tip_b } else { &mut r.tip_a };
                let done = if tip_b { &mut r.done_b } else { &mut r.done_a };
                for _ in 0..self.rift_prop_cells {
                    // Re-anchor: the plate may have advected out from under
                    // the stored tip id — follow the scar to a plate cell.
                    if self.plate_id[*tip as usize] != r.plate {
                        let anchor = self.grid.neighbors_of(*tip).iter().copied().find(|&nb| {
                            self.plate_id[nb as usize] == r.plate
                                && self.rift_age[nb as usize] >= RIFT_ONSET_MY
                        });
                        match anchor {
                            Some(nb) => *tip = nb,
                            None => {
                                failed = true;
                                break;
                            }
                        }
                    }
                    // Reached the plate boundary: this tip is finished.
                    if self
                        .grid
                        .neighbors_of(*tip)
                        .iter()
                        .any(|&nb| self.plate_id[nb as usize] != r.plate)
                    {
                        *done = true;
                        break;
                    }
                    // Walk to the weakest unclaimed neighbor (tie → lowest
                    // id via strict <; neighbors come in fixed CCW order,
                    // so sort candidates by id first).
                    let mut nbs: Vec<u32> = self.grid.neighbors_of(*tip).to_vec();
                    nbs.sort_unstable();
                    let mut best: Option<(u32, f32)> = None;
                    for &nb in &nbs {
                        let nbu = nb as usize;
                        let claimed_cont =
                            self.crust_type[nbu] == 1 && self.rift_age[nbu] >= RIFT_ONSET_MY;
                        let claimed_ocean =
                            self.crust_type[nbu] == 0 && self.crust_age[nbu] < CORRIDOR_MAX_AGE_MY;
                        if claimed_cont || claimed_ocean {
                            continue;
                        }
                        let s = self.strength(nbu);
                        if best.is_none_or(|(_, bs)| s < bs) {
                            best = Some((nb, s));
                        }
                    }
                    let Some((nb, s_next)) = best else {
                        failed = true;
                        break;
                    };
                    if r.stress <= s_next {
                        failed = true; // amendment A: stalled = failed
                        break;
                    }
                    let nbu = nb as usize;
                    if self.crust_type[nbu] == 1 {
                        // Claim continental path: jump-start maturation.
                        self.rift_age[nbu] = self.rift_age[nbu].max(jump_start);
                        self.features[nbu] |= F_RIFT;
                    } else {
                        // The rift propagates into plate-interior ocean as
                        // a fresh ridge line: new crust along the axis.
                        self.crust_age[nbu] = 0.0;
                        self.features[nbu] |= F_RIDGE;
                    }
                    *tip = nb;
                }
            }
            if failed {
                self.events.push(TectonicEvent::RiftFailed {
                    plate: r.plate,
                    t: self.t_my,
                });
                self.rift_failed_count += 1;
                keep[ri] = false;
                log::debug!("t={} My: rift on plate {} failed", self.t_my, r.plate);
            }
        }
        let mut it = keep.iter();
        rifts.retain(|_| *it.next().unwrap());
        self.rifts = rifts;

        // Nucleation: fixed driver order, one live rift per plate, a
        // refractory period after the plate's last rifting, and only where
        // driver stress beats the local strength (amendment A).
        let min_rift_cells = (self.plate_id.len() as f32 * MIN_RIFT_PLATE_FRACTION) as u32;
        for d in self.rift_drivers() {
            if !self.plates[d.plate as usize].alive
                || self.rifts.iter().any(|r| r.plate == d.plate)
                || self.plate_cells[d.plate as usize] < min_rift_cells
                || self.t_my - self.plates[d.plate as usize].youngest_rift_my < RIFT_REFRACTORY_MY
            {
                continue;
            }
            if d.stress <= self.strength(d.cell as usize) {
                continue;
            }
            self.plates[d.plate as usize].youngest_rift_my = self.t_my;
            if d.hotspot != u32::MAX {
                // The plume head has vented into the rift; its residence
                // clock starts over.
                self.hotspot_cont_my[d.hotspot as usize] = 0.0;
            }
            let cu = d.cell as usize;
            self.rift_age[cu] = self.rift_age[cu].max(jump_start);
            self.features[cu] |= F_RIFT;
            self.rifts.push(ActiveRift {
                plate: d.plate,
                kind: d.kind,
                stress: d.stress,
                tip_a: d.cell,
                tip_b: d.cell,
                done_a: false,
                done_b: false,
                started_my: self.t_my,
            });
            self.events.push(TectonicEvent::RiftStart {
                plate: d.plate,
                driver: d.kind,
                t: self.t_my,
            });
            self.rift_start_count += 1;
            log::debug!(
                "t={} My: rift nucleated on plate {} ({:?})",
                self.t_my,
                d.plate,
                d.kind
            );
        }
    }

    /// Model §5 split: when a completed rift's corridor (young plate-
    /// interior ocean) cuts the plate's remaining cells into disconnected
    /// regions, the plate splits along it. The halves keep the parent's
    /// pole and speed — from the next step the force balance owns them
    /// (their new ridge supplies ridge push; no imposed rift speed).
    /// §6: a small child logs as a microplate (back-arc basin or ridge
    /// jump, by driver and content).
    fn check_rift_splits(&mut self) {
        let n = self.grid.cell_count() as usize;
        let micro_max = (n as f32 * MICRO_MAX_FRACTION) as u32;
        let mut ri = 0;
        while ri < self.rifts.len() {
            let r = self.rifts[ri];
            if !(r.done_a && r.done_b)
                || !self.plates[r.plate as usize].alive
                // A splinter deforms rather than splitting further.
                || self.plate_cells[r.plate as usize]
                    < (n as f32 * MIN_RIFT_PLATE_FRACTION) as u32
                // Candidate-mask headroom: a deferred split just waits.
                || self.alive_plates() >= MAX_ALIVE_PLATES
            {
                ri += 1;
                continue;
            }
            let pid = r.plate;
            // Label the plate's non-corridor cells into components.
            let corridor =
                |s: &Self, c: usize| s.crust_type[c] == 0 && s.crust_age[c] <= CORRIDOR_MAX_AGE_MY;
            let mut comp_of = vec![u32::MAX; n];
            let mut comp_cells: Vec<Vec<u32>> = Vec::new();
            let mut queue: VecDeque<u32> = VecDeque::new();
            for c0 in 0..n {
                if self.plate_id[c0] != pid || comp_of[c0] != u32::MAX || corridor(self, c0) {
                    continue;
                }
                let ci = comp_cells.len() as u32;
                comp_cells.push(Vec::new());
                comp_of[c0] = ci;
                queue.push_back(c0 as u32);
                while let Some(c) = queue.pop_front() {
                    comp_cells[ci as usize].push(c);
                    for &nb in self.grid.neighbors_of(c) {
                        let nbu = nb as usize;
                        if comp_of[nbu] == u32::MAX
                            && self.plate_id[nbu] == pid
                            && !corridor(self, nbu)
                        {
                            comp_of[nbu] = ci;
                            queue.push_back(nb);
                        }
                    }
                }
            }
            // Big components (id order preserved); need at least two for a
            // split. The largest keeps the plate id (tie → earliest, which
            // holds the lowest cell id).
            let mut big: Vec<u32> = (0..comp_cells.len() as u32)
                .filter(|&ci| comp_cells[ci as usize].len() as u32 >= MIN_SPLIT_CELLS)
                .collect();
            if big.len() < 2 {
                ri += 1;
                continue;
            }
            let keeper = *big
                .iter()
                .max_by_key(|&&ci| comp_cells[ci as usize].len())
                .unwrap();
            big.retain(|&ci| ci != keeper);

            // New plates for the other components, inheriting the parent's
            // motion and suture clock; empty slab ledger. Both parent and
            // children start a fresh rift refractory: the split just spent
            // the stress a new driver would need.
            let parent = &self.plates[pid as usize];
            let (pole, speed, ys) = (parent.pole, parent.speed_deg_my, parent.youngest_suture_my);
            self.plates[pid as usize].youngest_rift_my = self.t_my;
            let mut new_of_comp = vec![u32::MAX; comp_cells.len()];
            for &ci in &big {
                let child = self.spawn_plate(pole, speed, ys, self.t_my);
                new_of_comp[ci as usize] = child;
                let cells = &comp_cells[ci as usize];
                let cont = cells
                    .iter()
                    .filter(|&&c| self.crust_type[c as usize] == 1)
                    .count();
                self.breakup_count += 1;
                self.events.push(TectonicEvent::Split {
                    parent: pid,
                    child,
                    driver: r.kind,
                    t: self.t_my,
                });
                if (cells.len() as u32) < micro_max {
                    let origin = if r.kind == RiftDriverKind::BackArc {
                        Some(MicroplateOrigin::BackArcBasin)
                    } else if cont > 0 {
                        Some(MicroplateOrigin::RidgeJump)
                    } else {
                        None
                    };
                    if let Some(origin) = origin {
                        self.events.push(TectonicEvent::Microplate {
                            id: child,
                            origin,
                            t: self.t_my,
                        });
                        self.microplate_count += 1;
                    }
                }
                log::debug!(
                    "t={} My: split of plate {pid} -> {pid} + {child} ({:?}, {} cells)",
                    self.t_my,
                    r.kind,
                    cells.len()
                );
            }
            // Assign cells: big components keep/take their plate; corridor
            // cells and sub-threshold fragments join whichever big
            // component reaches them first (multi-source BFS, id-seeded —
            // each new plate stays one connected region by construction).
            let mut owner = vec![u32::MAX; n];
            let mut queue: VecDeque<u32> = VecDeque::new();
            for ci in 0..comp_cells.len() as u32 {
                let plate = if ci == keeper {
                    pid
                } else if new_of_comp[ci as usize] != u32::MAX {
                    new_of_comp[ci as usize]
                } else {
                    continue; // sub-threshold fragment: filled by the BFS
                };
                for &c in &comp_cells[ci as usize] {
                    owner[c as usize] = plate;
                    queue.push_back(c);
                }
            }
            while let Some(c) = queue.pop_front() {
                let o = owner[c as usize];
                for &nb in self.grid.neighbors_of(c) {
                    let nbu = nb as usize;
                    if owner[nbu] == u32::MAX && self.plate_id[nbu] == pid {
                        owner[nbu] = o;
                        queue.push_back(nb);
                    }
                }
            }
            // The continental census (insulation input) is NOT adjusted
            // here: crust types changed since it was taken this step, so
            // deltas could underflow — accumulate_boundary_stats refreshes
            // it next step.
            for (c, &o) in owner.iter().enumerate() {
                if self.plate_id[c] == pid && o != u32::MAX && o != pid {
                    self.plate_id[c] = o;
                    self.plate_cells[pid as usize] -= 1;
                    self.plate_cells[o as usize] += 1;
                }
            }
            // The rift has done its work.
            self.rifts.remove(ri);
        }
    }

    fn apply_hotspots(&mut self) {
        if self.hotspot_hints.len() != self.hotspots.len() {
            self.hotspot_hints = vec![0; self.hotspots.len()];
        }
        if self.hotspot_cont_my.len() != self.hotspots.len() {
            self.hotspot_cont_my = vec![0.0; self.hotspots.len()];
        }
        for h in 0..self.hotspots.len() {
            let hint = self.hotspot_hints[h];
            let c = self.grid.nearest_cell(self.hotspots[h], Some(hint));
            self.hotspot_hints[h] = c;
            let cu = c as usize;
            // Plume residence clock: continuous time under continental
            // crust (the §5 plume rift driver's condition).
            if self.crust_type[cu] == 1 {
                self.hotspot_cont_my[h] += DT_MY;
            } else {
                self.hotspot_cont_my[h] = 0.0;
            }
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
        // Slab detachment: a segment stops pulling SLAB_DETACH_MY after it
        // went under; long-detached segments leave the ledger. Fixed plate
        // and segment order.
        let t = self.t_my;
        for p in self.plates.iter_mut() {
            for seg in p.slab.iter_mut() {
                if seg.attached && t - seg.subducted_at_my > SLAB_DETACH_MY {
                    seg.attached = false;
                }
            }
            p.slab
                .retain(|seg| seg.attached || t - seg.subducted_at_my <= 2.0 * SLAB_DETACH_MY);
        }
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
            p.colliding_strength = self.colliding_strength[i];
            p.ridge_cells = self.ridge_cells[i];
            p.transform_cells = self.transform_cells[i];
            p.drive_torque = self.torques[i];
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
            &self.slab_plate,
            &self.slab_since_my,
            &self.suture_at_my,
            &self.hotspot_cont_my,
            self.rifts.clone(),
            plates,
            self.collisions.clone(),
        )
    }

    /// Count of alive plates.
    pub fn alive_plates(&self) -> usize {
        self.plates.iter().filter(|p| p.alive).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_plate(id: u32, speed: f32) -> PlateState {
        PlateState {
            id,
            alive: true,
            pole: [0.0, 0.0, 1.0],
            speed_deg_my: speed,
            youngest_suture_my: super::super::keyframe::NEVER_SUTURED,
            youngest_rift_my: super::super::keyframe::NEVER_SUTURED,
            pending_rot: IDENTITY3,
            pending_deg: 0.0,
            slab: Vec::new(),
            boundary_cells: 0,
            subducting_cells: 0,
            colliding_cells: 0,
            colliding_strength: 0.0,
            ridge_cells: 0,
            transform_cells: 0,
            drive_torque: [0.0; 3],
        }
    }

    /// A minimal one-plate state for exercising motion_update directly.
    fn one_plate_state(area_cells: u32, speed: f32) -> SimState {
        let grid = Arc::new(Grid::build(2));
        let mut s = SimState::new_empty(&grid);
        s.plates.push(test_plate(0, speed));
        s.plate_cells = vec![area_cells];
        s.boundary_cells = vec![0];
        s.subducting_cells = vec![0];
        s.colliding_cells = vec![0];
        s.colliding_strength = vec![0.0];
        s.ridge_cells = vec![0];
        s.transform_cells = vec![0];
        s.torques = vec![[0.0; 3]];
        s.cont_cells_per_plate = vec![0];
        s
    }

    /// §1: with no slab, no ridge, and no boundary resistance, the balance
    /// is residual traction over basal drag — speed relaxes to
    /// K_MANTLE / C_DRAG regardless of plate size or starting speed.
    #[test]
    fn zero_drivers_relax_to_residual_drift() {
        let v_resid = K_MANTLE / C_DRAG;
        for (area, start) in [(100u32, 1.5f32), (5000, 0.0), (321, v_resid)] {
            let mut s = one_plate_state(area, start);
            for _ in 0..2000 {
                s.motion_update();
            }
            let v = s.plates[0].speed_deg_my;
            assert!(
                (v - v_resid).abs() < 1e-4,
                "area {area}, start {start}: relaxed to {v}, want {v_resid}"
            );
        }
    }

    /// §1: an attached slab segment is a driver — the same plate with one
    /// attached segment ends up faster than without.
    #[test]
    fn attached_slab_outpulls_slab_free() {
        let mut free = one_plate_state(1000, 0.3);
        let mut pulled = one_plate_state(1000, 0.3);
        pulled.plates[0].slab.push(SlabSegment {
            area_cells: 200,
            age_at_subduction_my: 80.0,
            subducted_at_my: 0.0,
            attached: true,
        });
        // A detached copy of the same segment must NOT pull.
        let mut detached = one_plate_state(1000, 0.3);
        detached.plates[0].slab.push(SlabSegment {
            area_cells: 200,
            age_at_subduction_my: 80.0,
            subducted_at_my: 0.0,
            attached: false,
        });
        for _ in 0..100 {
            free.motion_update();
            pulled.motion_update();
            detached.motion_update();
        }
        let (vf, vp, vd) = (
            free.plates[0].speed_deg_my,
            pulled.plates[0].speed_deg_my,
            detached.plates[0].speed_deg_my,
        );
        assert!(vp > vf, "attached slab must pull: {vp} <= {vf}");
        assert_eq!(vd, vf, "a detached slab must not pull");
    }

    /// §7: a hand-built two-fragment plate leaves enforce_connectivity as
    /// one component, the smaller fragment reassigned to its border plate.
    #[test]
    fn connectivity_backstop_reunifies_fragments() {
        let grid = Arc::new(Grid::build(3));
        let n = grid.cell_count() as usize;
        let mut s = SimState::new_empty(&grid);
        s.plates.push(test_plate(0, 0.3));
        s.plates.push(test_plate(1, 0.3));
        // Plate 1 = a large patch around cell 40 (3 rings) plus a distant
        // exclave around cell 600 (1 ring); plate 0 owns the rest.
        let grow = |seed: u32, rings: u32| {
            let mut cells = vec![seed];
            for _ in 0..rings {
                let mut next = cells.clone();
                for &c in &cells {
                    next.extend_from_slice(grid.neighbors_of(c));
                }
                next.sort_unstable();
                next.dedup();
                cells = next;
            }
            cells
        };
        let main_patch = grow(40, 3);
        let exclave = grow(600, 1);
        assert!(main_patch.len() > exclave.len());
        for &c in main_patch.iter().chain(&exclave) {
            s.plate_id[c as usize] = 1;
        }
        s.plate_cells = vec![0, 0];
        for &p in &s.plate_id {
            s.plate_cells[p as usize] += 1;
        }

        s.enforce_connectivity();

        assert_eq!(s.connectivity_reassigned, exclave.len() as u64);
        // Every plate-1 cell now reachable from the main patch: BFS count
        // equals the plate-1 census (one component).
        let census = s.plate_id.iter().filter(|&&p| p == 1).count();
        let mut seen = vec![false; n];
        let start = *main_patch.first().unwrap() as usize;
        assert_eq!(s.plate_id[start], 1);
        let mut queue = VecDeque::from([start as u32]);
        seen[start] = true;
        let mut count = 0;
        while let Some(c) = queue.pop_front() {
            count += 1;
            for &nb in grid.neighbors_of(c) {
                if !seen[nb as usize] && s.plate_id[nb as usize] == 1 {
                    seen[nb as usize] = true;
                    queue.push_back(nb);
                }
            }
        }
        assert_eq!(count, census, "plate 1 must be a single component");
        assert_eq!(census, main_patch.len(), "the largest fragment is kept");
        // The exclave went to the only bordering plate.
        for &c in &exclave {
            assert_eq!(s.plate_id[c as usize], 0);
        }
    }

    /// Two hemispheric continental plates on L3, everything locked
    /// (speed 0): the §3 suture testbed. Plate 1 owns z ≥ 0.
    fn two_plate_cont_state() -> SimState {
        let grid = Arc::new(Grid::build(3));
        let n = grid.cell_count() as usize;
        let mut s = SimState::new_empty(&grid);
        s.plates.push(test_plate(0, 0.0));
        s.plates.push(test_plate(1, 0.0));
        for c in 0..n {
            s.plate_id[c] = u32::from(grid.positions[c][2] >= 0.0);
            s.crust_type[c] = 1;
            s.thickness[c] = 35.0;
            s.crust_age[c] = 500.0;
            s.orogeny_age[c] = 500.0;
        }
        s.t_my = 500.0;
        s.init_stats();
        s
    }

    /// Run only the suture-relevant sub-steps (no advection: nothing moves).
    fn suture_steps(s: &mut SimState, steps: u32) {
        for _ in 0..steps {
            s.classify_boundaries();
            s.accumulate_boundary_stats();
            s.update_pair_timers_and_sutures();
            s.t_my += DT_MY;
        }
    }

    /// §3 condition 1: a pinprick contact never sutures, no matter how long
    /// it stays locked.
    #[test]
    fn pinprick_contact_never_sutures() {
        let grid = Arc::new(Grid::build(3));
        let n = grid.cell_count() as usize;
        let mut s = SimState::new_empty(&grid);
        s.plates.push(test_plate(0, 0.0));
        s.plates.push(test_plate(1, 0.0));
        // Plate 1 = a 2-ring continental blob; plate 0 = the rest, ocean
        // except ONE continental cell touching the blob, so the
        // continent-continent contact is a single-cell pinprick.
        let mut blob = vec![40u32];
        for _ in 0..2 {
            let mut next = blob.clone();
            for &c in &blob {
                next.extend_from_slice(grid.neighbors_of(c));
            }
            next.sort_unstable();
            next.dedup();
            blob = next;
        }
        for c in 0..n {
            s.plate_id[c] = 0;
            s.crust_type[c] = 0;
            s.thickness[c] = OCEAN_THICKNESS_KM;
            s.crust_age[c] = 80.0;
        }
        for &c in &blob {
            s.plate_id[c as usize] = 1;
            s.crust_type[c as usize] = 1;
            s.thickness[c as usize] = 35.0;
            s.crust_age[c as usize] = 500.0;
            s.orogeny_age[c as usize] = 500.0;
        }
        // The pinprick: one plate-0 neighbor of the blob becomes continent.
        let pin = (0..n as u32)
            .find(|&c| {
                s.plate_id[c as usize] == 0
                    && grid
                        .neighbors_of(c)
                        .iter()
                        .any(|&nb| s.plate_id[nb as usize] == 1)
            })
            .unwrap();
        s.crust_type[pin as usize] = 1;
        s.thickness[pin as usize] = 35.0;
        s.orogeny_age[pin as usize] = 500.0;
        s.t_my = 500.0;
        s.init_stats();

        suture_steps(&mut s, 40);
        assert_eq!(s.alive_plates(), 2, "a pinprick contact must never weld");
        assert_eq!(s.suture_count, 0);
        assert!(s.events.is_empty());
        assert!(s.collisions.iter().all(|t| t.slow_collision_my == 0.0));
    }

    /// §3 condition 3: a locked full-perimeter contact with ocean two rings
    /// from the weld does not suture...
    #[test]
    fn locked_contact_with_nearby_ocean_does_not_suture() {
        let mut s = two_plate_cont_state();
        // Find a cell exactly 2 rings from the contact and flood it.
        let n = s.grid.cell_count() as usize;
        let mut depth = vec![u16::MAX; n];
        let mut queue: VecDeque<u32> = VecDeque::new();
        for (c, d) in depth.iter_mut().enumerate() {
            let foreign = s
                .grid
                .neighbors_of(c as u32)
                .iter()
                .any(|&nb| s.plate_id[nb as usize] != s.plate_id[c]);
            if foreign {
                *d = 0;
                queue.push_back(c as u32);
            }
        }
        while let Some(c) = queue.pop_front() {
            let dc = depth[c as usize];
            for &nb in s.grid.neighbors_of(c) {
                if depth[nb as usize] == u16::MAX {
                    depth[nb as usize] = dc + 1;
                    queue.push_back(nb);
                }
            }
        }
        let wet = depth.iter().position(|&d| d == 2).unwrap();
        s.crust_type[wet] = 0;
        s.thickness[wet] = OCEAN_THICKNESS_KM;
        s.crust_age[wet] = 80.0;

        suture_steps(&mut s, 40);
        assert_eq!(s.alive_plates(), 2, "open ocean near the contact: no weld");
        assert_eq!(s.suture_count, 0);
        assert!(s.collisions.iter().all(|t| t.slow_collision_my == 0.0));
    }

    /// ...and the same contact with the ocean closed sutures at 30 My,
    /// writing the scar on the contact cells and logging the conditions.
    #[test]
    fn locked_continental_contact_sutures_at_30_my() {
        let mut s = two_plate_cont_state();
        let t0 = s.t_my;
        suture_steps(&mut s, 16);
        assert_eq!(s.alive_plates(), 1, "closed locked contact must weld");
        assert_eq!(s.suture_count, 1);
        let Some(TectonicEvent::Suture {
            t,
            contact_fraction,
            ..
        }) = s.events.first()
        else {
            panic!("expected a suture event, got {:?}", s.events.first());
        };
        // Timer hits 30 My on the 15th accumulation (timers start at 0).
        assert_eq!(*t, t0 + 14.0 * DT_MY);
        assert!(
            *contact_fraction >= SUTURE_CONTACT_FRACTION,
            "recorded contact fraction {contact_fraction} under the §3 minimum"
        );
        // The scar is on the cells: every cell of the old contact carries
        // suture_at_my = fire time.
        let scarred = s.suture_at_my.iter().filter(|&&v| v == *t).count();
        assert!(scarred > 10, "suture scar missing ({scarred} cells)");
    }

    /// Amendment A: a plume driver under a craton (strength ≥ 1.5) does not
    /// nucleate a rift...
    fn plume_state() -> (SimState, usize) {
        let grid = Arc::new(Grid::build(3));
        let n = grid.cell_count() as usize;
        let mut s = SimState::new_empty(&grid);
        s.plates.push(test_plate(0, 0.0));
        for c in 0..n {
            s.plate_id[c] = 0;
            s.crust_type[c] = 1;
            s.thickness[c] = 40.0;
            s.crust_age[c] = 2500.0;
            s.orogeny_age[c] = 2500.0;
        }
        let cell = 100usize;
        s.hotspots = vec![grid.positions[cell]];
        s.hotspot_hints = vec![0];
        s.hotspot_cont_my = vec![PLUME_UNDER_CONT_MY + 5.0];
        s.t_my = 3000.0;
        s.init_stats();
        // Pin the continental census below the amendment-B supercontinent
        // threshold: this test isolates the craton/suture strength contrast
        // (insulation has its own dynamics in the probe).
        s.cont_cells_per_plate = vec![10];
        s.cont_total_cells = 100;
        (s, cell)
    }

    #[test]
    fn plume_under_craton_does_not_nucleate() {
        let (mut s, cell) = plume_state();
        assert!(
            s.strength(cell) >= 1.5,
            "craton strength {} under the model's ~2.0 anchor",
            s.strength(cell)
        );
        s.grow_rifts();
        assert!(s.rifts.is_empty(), "a craton must not rift under a plume");
        assert!(s.events.is_empty());
    }

    /// ...but the same plume under a 100 My-old suture does.
    #[test]
    fn plume_under_old_suture_nucleates() {
        let (mut s, cell) = plume_state();
        s.suture_at_my[cell] = s.t_my - 100.0;
        s.orogeny_age[cell] = 100.0; // the weld's orogeny is young
        assert!(s.strength(cell) < STRESS_PLUME);
        s.grow_rifts();
        assert_eq!(s.rifts.len(), 1, "the suture scar must rift");
        assert_eq!(s.rifts[0].plate, 0);
        assert_eq!(s.rifts[0].kind, RiftDriverKind::Plume);
        assert!(matches!(
            s.events.first(),
            Some(TectonicEvent::RiftStart {
                driver: RiftDriverKind::Plume,
                ..
            })
        ));
        assert!(
            s.rift_age[cell] > RIFT_ONSET_MY,
            "nucleation must jump-start maturation"
        );
    }

    /// Model §5: an oceanized corridor splits the plate into two connected
    /// halves, each of which reads a new ridge into its force balance.
    #[test]
    fn split_plate_has_two_components_each_with_a_ridge() {
        let grid = Arc::new(Grid::build(4));
        let n = grid.cell_count() as usize;
        let mut s = SimState::new_empty(&grid);
        s.plates.push(test_plate(0, 0.3));
        for c in 0..n {
            s.plate_id[c] = 0;
            if grid.positions[c][2].abs() < 0.1 {
                // The oceanized rift corridor: fresh ridge-age ocean.
                s.crust_type[c] = 0;
                s.thickness[c] = OCEAN_THICKNESS_KM;
                s.crust_age[c] = 0.0;
            } else {
                s.crust_type[c] = 1;
                s.thickness[c] = 35.0;
                s.crust_age[c] = 500.0;
                s.orogeny_age[c] = 500.0;
            }
        }
        s.t_my = 500.0;
        s.rifts.push(ActiveRift {
            plate: 0,
            kind: RiftDriverKind::Plume,
            stress: STRESS_PLUME,
            tip_a: 0,
            tip_b: 0,
            done_a: true,
            done_b: true,
            started_my: 450.0,
        });
        s.init_stats();

        s.check_rift_splits();

        assert_eq!(s.alive_plates(), 2, "the corridor must split the plate");
        assert!(s.rifts.is_empty(), "the spent rift leaves the ledger");
        assert!(matches!(
            s.events.first(),
            Some(TectonicEvent::Split {
                parent: 0,
                driver: RiftDriverKind::Plume,
                ..
            })
        ));
        // Each half is one connected region.
        for pid in 0..2u32 {
            let census = s.plate_id.iter().filter(|&&p| p == pid).count();
            assert!(census > 0);
            let start = s.plate_id.iter().position(|&p| p == pid).unwrap();
            let mut seen = vec![false; n];
            seen[start] = true;
            let mut queue = VecDeque::from([start as u32]);
            let mut count = 0;
            while let Some(c) = queue.pop_front() {
                count += 1;
                for &nb in grid.neighbors_of(c) {
                    if !seen[nb as usize] && s.plate_id[nb as usize] == pid {
                        seen[nb as usize] = true;
                        queue.push_back(nb);
                    }
                }
            }
            assert_eq!(count, census, "plate {pid} must be one component");
        }
        // Each half reads its new ridge: the young-ocean corridor boundary
        // supplies ridge drive even at zero divergence (the bootstrap).
        s.classify_boundaries();
        s.accumulate_boundary_stats();
        assert!(s.ridge_cells[0] > 0, "half 0 has no ridge drive");
        assert!(s.ridge_cells[1] > 0, "half 1 has no ridge drive");
    }
}
