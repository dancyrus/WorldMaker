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
const K_MANTLE: f32 = 0.10;
/// Basal drag per plate cell (the normalization of the balance).
const C_DRAG: f32 = 1.25;
/// Continent-continent contact resistance per contact cell (strength = 1.0
/// until WO-0006 S2 lands the strength field).
const C_CONTACT: f32 = 900.0;
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
pub const SLAB_DETACH_MY: f32 = 100.0;
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
// ----- distributed orogeny (WO-0008 S2, model §9/S2 items) -----
/// Deformation-zone depth: W = W_BASE_KM / strength, walked inboard from
/// each contact cell and clamped to 1..=W_MAX_CELLS cells; weak crust
/// (strength ~0.3–0.8) gives 3–8 cells at L6 (300–900 km), cratonic
/// crust ≤ 1 (deformation localizes in weak lithosphere and dies at
/// cratons — Tarim and Sichuan stop the Himalayan front).
const W_BASE_KM: f32 = 260.0;
const W_MAX_CELLS: u32 = 8;
/// The zone walk stops where strength reaches cratonic grade.
const CRATON_STOP: f32 = 1.5;
/// Gravitational spreading (lower-crustal channel flow, Tibet): one
/// diffusion pass per step on continental cells above this thickness...
const SPREAD_THRESHOLD_KM: f32 = 60.0;
/// ...moving excess to the thinnest neighbor at this rate per km of
/// excess.
const SPREAD_KM_MY: f32 = 0.05;
/// Discrete island arcs (WO-0008 S2): over an OCEANIC overrider the arc
/// band grows volcanic edifices only at discrete sites — a greedy
/// maximal independent set in cell-id order (no two sites adjacent, so
/// islands emerge one by one and stay 1 cell wide) — approximating one
/// site every ~2 cells along the band. Between sites the band gains this
/// fraction of the site rate. Continent-margin arcs (Andes) stay
/// continuous.
const ARC_OFFSITE_FRACTION: f32 = 0.2;
/// Clearance rings between arc sites (and from young islands): every
/// ~4th band cell hosts an edifice. The band is one ring wide at L3-L6,
/// so 1-ring spacing would put a site on every other band cell and the
/// band would run half land; the < 30% band-land target needs this.
const ARC_SITE_RINGS: u32 = 3;
/// A new arc site never opens beside continental crust younger than this:
/// the existing edifice focuses the magmatism, so islands stay discrete
/// instead of creeping into walls as the band drifts.
const ARC_SITE_ISLAND_BLOCK_MY: f32 = 60.0;
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
/// Condition 1's absolute alternative (WO-0008 S1): a contact spanning at
/// least this many km of front is a substantial margin regardless of the
/// perimeter ratio. The 30%-of-perimeter test structurally vetoed welds
/// between GIANT plates — a locked 90-cell (~10,000 km) front ground for
/// 800 My at seed cyrus because both plates had ~400-cell perimeters —
/// while the §3 intent is only that a pinprick must never weld. 5,500 km
/// (~50 cells at L6) is far above any pinprick and in the range of real
/// terminal-collision fronts (the Himalayan front plus its syntaxes;
/// central Asian composite sutures).
pub(super) const SUTURE_ABS_CONTACT_KM: f32 = 5500.0;
/// Condition 2: mean relative speed across the contact (cm/yr) below the
/// classification dead band — the contact is kinematically indistinguishable
/// from plate interior (Gordon 1998).
const SUTURE_LOCK_CMYR: f32 = 0.4;
/// Condition 3 (amended WO-0008 S1): no oceanic region larger than
/// `RELIC_BASIN_KEEP_CELLS` within this many rings of the contact on both
/// sides — suturing is the terminal act of the Wilson cycle, after the
/// intervening ocean is consumed (Wilson 1966), but a consumed-down relic
/// sea no longer blocks the weld.
pub(super) const SUTURE_OCEAN_RINGS: u16 = 2;

// ----- relic-basin closure (WO-0008 S1, model §3 addendum) -----
/// A basin consumed down to this many cells survives as a relic sea
/// (Caspian / Black Sea) and stops blocking suture condition 3.
pub const RELIC_BASIN_KEEP_CELLS: u32 = 12;
/// A connected oceanic region counts as enclosed by a colliding pair when
/// at least this fraction of its bordering continental cells belong to
/// the two plates.
const RELIC_ENCLOSED_FRACTION: f32 = 0.8;

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
const INSULATION_START_MY: f32 = 55.0;
const INSULATION_FULL_MY: f32 = 165.0;
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
const RIFT_REFRACTORY_MY: f32 = 240.0;
/// Two active rift tips on the same plate within this many cells connect
/// along the least-strength path and merge their systems (WO-0008 S1:
/// East Africa–Red Sea–Gulf of Aden linkage).
const RIFT_LINK_CELLS: u16 = 3;
/// A rift needs a plate interior to cut: plates below this fraction of the
/// sphere deform instead of splitting (no nucleation, no split). Without
/// it, splits of splinters feed a runaway froth (measured: the census
/// railed at the 60-plate mask cap by 800 My at L5 and continents ground
/// away to 1% of the sphere by 2 Gy).
const MIN_RIFT_PLATE_FRACTION: f32 = 1.0 / 15.0;
/// Completed rifts whose split never materialized leave the ledger after
/// this long (attribution bookkeeping only; the scar cells stay).
const RIFT_ENTRY_PRUNE_MY: f32 = 400.0;
/// The split corridor: plate-interior ocean younger than this marks the
/// freshly oceanized rift line (and pre-existing basins stay out of it).
const CORRIDOR_MAX_AGE_MY: f32 = 60.0;
/// A split component smaller than this stays with the parent (seam noise).
const MIN_SPLIT_CELLS: u32 = 24;
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
/// Fossil-boundary capture (WO-0008 S1): a plate below
/// MICRO_MAX_FRACTION of the sphere whose ENTIRE boundary stays below the
/// classification dead band for this long has fossilized and merges into
/// the neighbor sharing the longest border (Kula-style capture). This is
/// the death path for split debris: without it, split births outran
/// deaths and the census inflated through every active Wilson cycle.
const CAPTURE_AFTER_MY: f32 = 60.0;
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
    /// Land fraction the whole-plate crust setup actually achieved
    /// (WO-0008 S0): continental cells over the shelf-margin budget's
    /// denominator, quantized by plate sizes. Set once at setup; a
    /// resumed run leaves it 0 (setup-only diagnostic).
    pub achieved_land_frac: f32,

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
    /// Direct forward claims only (no ring dilation): bit d set when some
    /// cell of dense plate d mapped exactly onto this cell in the scatter
    /// pass. The seam rule (WO-0008 S1) unions this with the back-rotated
    /// coverage sample so both rasterizations of the same rigid motion
    /// agree on ownership.
    direct_mask: Vec<AtomicU64>,
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
    /// Ocean consumed into continental margin by relic-basin closure
    /// (WO-0008 S1).
    pub cont_gained_by_closure: u64,
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
    /// Same-plate rift systems merged by tip linkage (WO-0008 S1).
    pub rift_link_count: u64,
    pub microplate_count: u64,
    /// Cells reassigned by the connectivity backstop (cumulative). The §7
    /// invariant target: this fires only for advection seam noise.
    pub connectivity_reassigned: u64,

    // ----- crust-volume ledger (WO-0008 S2) -----
    // Continental crustal volume in exact quantized units of
    // 0.01 km × cell area (all cells equal area up to the pentagon
    // deficit, which the spec treats as noise). Each term is the phase's
    // measured before/after delta, summed i64 in fixed order, so
    // Δ(total) − Σ(terms) ≡ 0 by construction and the interesting gate is
    // the collision path's internal exactness: volume removed from
    // consumed margin cells reappears in the distributed zone, same step.
    /// Net advection delta (consumption, accretion, gap ridges, repair).
    pub vol_advect_q: i64,
    /// Relic-basin closure conversions (WO-0008 S1 mechanic).
    pub vol_closure_q: i64,
    /// Arc growth and island conversions (creation: subduction magmatism).
    pub vol_arc_q: i64,
    /// Continental-collision distributed thickening — funded EXACTLY by
    /// the underthrust budget (see below); no free creation since S2.
    pub vol_collision_q: i64,
    /// Rift thinning and oceanization (destruction).
    pub vol_rift_q: i64,
    /// Gravitational spreading (conservative up to f32 rounding).
    pub vol_spread_q: i64,
    /// Orogeny relaxation and related decay (pre-erosion sink).
    pub vol_relax_q: i64,
    /// Keyframe quantization rounding (booked by quantize_state).
    pub vol_quantize_q: i64,
    /// Underthrusting: continental volume removed at collision margins...
    pub underthrust_removed_q: i64,
    /// ...deposited into the pairs' distributed zones...
    pub underthrust_deposited_q: i64,
    /// ...or spilled when every zone cell sat at the thickness cap.
    pub underthrust_spilled_q: i64,
    /// Pre-existing oceanic columns incorporated when foreland shelf
    /// cells convert under spilled load (ophiolite/foreland basement).
    pub underthrust_incorporated_q: i64,
    /// This step's per-pair underthrust budget (transient: filled by
    /// advect, drained by apply_collisions the same step; never
    /// keyframed).
    underthrust_budget: Vec<(u32, u32, i64)>,
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
            achieved_land_frac: 0.0,
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
            direct_mask: (0..n).map(|_| AtomicU64::new(0)).collect(),
            outs: Vec::new(),
            class: vec![ClassOut::default(); n],
            bfs_depth: vec![u16::MAX; n],
            hotspot_hints: Vec::new(),
            cont_lost_to_ridge_gap: 0,
            cont_lost_to_consumption: 0,
            cont_lost_to_rift: 0,
            cont_gained_by_advection: 0,
            cont_gained_by_arc: 0,
            cont_gained_by_closure: 0,
            suture_count: 0,
            suture_fail_extent: 0,
            suture_fail_lock: 0,
            suture_fail_ocean: 0,
            breakup_count: 0,
            rift_start_count: 0,
            rift_failed_count: 0,
            rift_link_count: 0,
            microplate_count: 0,
            connectivity_reassigned: 0,
            vol_advect_q: 0,
            vol_closure_q: 0,
            vol_arc_q: 0,
            vol_collision_q: 0,
            vol_rift_q: 0,
            vol_spread_q: 0,
            vol_relax_q: 0,
            vol_quantize_q: 0,
            underthrust_removed_q: 0,
            underthrust_deposited_q: 0,
            underthrust_spilled_q: 0,
            underthrust_incorporated_q: 0,
            underthrust_budget: Vec::new(),
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
        let v_before = self.cont_volume_q();
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
        self.vol_quantize_q += self.cont_volume_q() - v_before;
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
        // The crust-volume ledger (WO-0008 S2) measures each phase's
        // continental-volume delta in exact quantized units: the terms
        // telescope to the total change, so nothing is unexplained, and
        // apply_collisions' delta must equal its underthrust deposits
        // exactly (the conservation gate).
        let v0 = self.cont_volume_q();
        self.motion_update();
        self.advect();
        self.enforce_connectivity();
        let v1 = self.cont_volume_q();
        self.vol_advect_q += v1 - v0;
        self.classify_boundaries();
        self.accumulate_boundary_stats();
        self.apply_arcs();
        let v2 = self.cont_volume_q();
        self.vol_arc_q += v2 - v1;
        self.apply_collisions();
        let v3 = self.cont_volume_q();
        self.vol_collision_q += v3 - v2;
        self.apply_spreading();
        let v4 = self.cont_volume_q();
        self.vol_spread_q += v4 - v3;
        self.apply_rifts();
        let v5 = self.cont_volume_q();
        self.vol_rift_q += v5 - v4;
        // Sutures read this step's classification, so they run before the
        // split pass moves any cells: every ownership change after
        // enforce_connectivity is then connectivity-preserving by
        // construction (a weld is a contact union; split halves are built
        // connected).
        self.update_pair_timers_and_sutures();
        let v6 = self.cont_volume_q();
        self.vol_closure_q += v6 - v5;
        self.capture_fossilized_plates();
        self.check_rift_splits();
        self.grow_rifts();
        self.apply_hotspots();
        self.age_and_relax();
        self.vol_relax_q += self.cont_volume_q() - v6;
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
                // Venting (WO-0008 S1): breakup releases the trapped heat
                // through the new ridge, so the insulation ramp restarts
                // from the plate's last actual BREAKUP as well as its last
                // suture — without this, a supercontinental plate stayed
                // at the insulation floor through every split in a runaway
                // cascade (measured: 29.5 splits/Gy and a census of 64).
                // Failed nucleation attempts vent nothing.
                let anchor = self.plates[pid]
                    .youngest_suture_my
                    .max(self.plates[pid].youngest_breakup_my);
                let dt = self.t_my - anchor;
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

        // Per-pair subduction polarity (WO-0008 S1 seam rule, half 2):
        // between two soft crusts, WHICH side subducts is decided once per
        // unordered plate pair per step — the side with the older (denser)
        // mean crust age along the shared boundary goes under; tie → the
        // higher plate id subducts (matching classify_boundaries' tie).
        // Under the old per-cell source-age comparison the polarity
        // flip-flopped along a single front (ages vary along it), the
        // fronts interpenetrated, and the interlocking bites pinched cells
        // off both plates — the probe measured pairs consuming each other
        // BOTH ways in the same step, thousands of backstop cells per
        // 100 My. Serial, cell-id order, f64 sums — deterministic.
        let pair_override: Vec<(u32, u32, u32)> = {
            let mut acc: Vec<(u32, u32, f64, u32, f64, u32)> = Vec::new();
            for c in 0..n {
                let a = self.plate_id[c];
                for &nb in self.grid.neighbors_of(c as u32) {
                    let b = self.plate_id[nb as usize];
                    if b == a {
                        continue;
                    }
                    let (lo, hi) = (a.min(b), a.max(b));
                    let e = match acc.iter_mut().find(|e| e.0 == lo && e.1 == hi) {
                        Some(e) => e,
                        None => {
                            acc.push((lo, hi, 0.0, 0, 0.0, 0));
                            acc.last_mut().unwrap()
                        }
                    };
                    if a == lo {
                        e.2 += self.crust_age[c] as f64;
                        e.3 += 1;
                    } else {
                        e.4 += self.crust_age[c] as f64;
                        e.5 += 1;
                    }
                }
            }
            acc.iter()
                .map(|&(lo, hi, s_lo, n_lo, s_hi, n_hi)| {
                    let m_lo = s_lo / n_lo.max(1) as f64;
                    let m_hi = s_hi / n_hi.max(1) as f64;
                    // Older mean subducts; tie → higher id subducts, so the
                    // lower id overrides.
                    let winner = if m_lo > m_hi { hi } else { lo };
                    (lo, hi, winner)
                })
                .collect()
        };
        let pair_override_ref = &pair_override;

        // Zero the candidate masks.
        self.cand_mask
            .par_iter()
            .for_each(|m| m.store(0, Ordering::Relaxed));
        self.direct_mask
            .par_iter()
            .for_each(|m| m.store(0, Ordering::Relaxed));

        let grid = &self.grid;
        let plate_id = &self.plate_id;
        let cand = &self.cand_mask;
        let direct = &self.direct_mask;

        // Forward scatter: each cell claims its destination and that cell's
        // ring for its plate. Atomic OR is commutative — deterministic. The
        // exact destination is also recorded ring-free: it is the forward
        // half of the seam rule below.
        (0..n).into_par_iter().for_each(|c| {
            let d = dense_of_id[plate_id[c] as usize];
            if !committing[d as usize] {
                return; // not moving this step; gather covers it locally
            }
            let dst_pos = mat3_mul(&fwd[d as usize], grid.positions[c]);
            let dst = grid.nearest_cell(dst_pos, Some(c as u32));
            let bit = 1u64 << d;
            direct[dst as usize].fetch_or(bit, Ordering::Relaxed);
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
                // The seam rule (WO-0008 S1): a plate covers this cell when
                // EITHER rasterization of its rigid motion says so — its
                // back-rotated sample lands on its own crust (gather half),
                // OR one of its cells mapped exactly here in the forward
                // scatter (direct claim). The two half-tests rasterize the
                // same motion from opposite ends; requiring only their union
                // removes the aliasing flicker that severed seam cells from
                // their plates (the backstop ate thousands of cells per
                // 100 My under the gather-only test).
                let direct_bits = direct[c].load(Ordering::Relaxed);
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
                    } else if direct_bits & (1u64 << d) != 0 {
                        // Forward-claimed but the back sample rounded off the
                        // plate: source the crust from the first plate-owned
                        // cell in the back sample's ring (fixed CCW ring
                        // order — deterministic; the sample sits within one
                        // cell of the plate edge, so a neighbor almost
                        // always matches; give up otherwise).
                        let nb_src = grid
                            .neighbors_of(src)
                            .iter()
                            .copied()
                            .find(|&nb| plate_id[nb as usize] == pid);
                        if let Some(sc) = nb_src {
                            cover_plate[covers] = pid;
                            cover_src[covers] = sc;
                            covers += 1;
                        }
                    }
                }

                // Boundary class of this cell on the previous step, for
                // gating events at transforms (zigzag hex boundaries spray
                // false gaps/overlaps under tangential slip).
                let was_transform_only = prev_feat[c] & F_BND_TRANSFORM != 0
                    && prev_feat[c] & F_BND_DIVERGENT == 0
                    && prev_feat[c] & F_BND_CONVERGENT == 0;

                let out = match covers {
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
                            // the soft winner comes from the per-pair
                            // polarity (seam rule): one verdict per plate
                            // pair per step, so the whole front agrees. The
                            // per-cell source-age comparison remains only as
                            // the fallback for a pair with no shared
                            // boundary last step (fresh contact).
                            let mut win = 0usize;
                            if hard_count == 1 {
                                win = (0..covers).find(|&i| is_hard(i)).unwrap();
                            } else {
                                for ch in 1..covers {
                                    let (pw, pc2) = (cover_plate[win], cover_plate[ch]);
                                    let (lo, hi) = (pw.min(pc2), pw.max(pc2));
                                    let ch_wins = match pair_override_ref
                                        .iter()
                                        .find(|e| e.0 == lo && e.1 == hi)
                                    {
                                        Some(&(_, _, w)) => w == pc2,
                                        None => {
                                            let (ws, cs) =
                                                (cover_src[win] as usize, cover_src[ch] as usize);
                                            prev_age[cs] < prev_age[ws]
                                                || (prev_age[cs] == prev_age[ws] && pc2 < pw)
                                        }
                                    };
                                    if ch_wins {
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
                };
                // Continental balance (WO-0008 S1 step 6): continental
                // crust at or above SUBDUCTIBLE_CONT_KM is never erased at
                // a consuming margin. Where another plate would replace it
                // with oceanic crust, the cell's continental content
                // survives and transfers to the winner instead — terrane
                // accretion (Wrangellia-style docking): buoyant continent
                // does not go down the slab.
                if out.ctype == 0
                    && out.plate != plate_id[c]
                    && prev_ctype[c] == 1
                    && prev_thick[c] >= SUBDUCTIBLE_CONT_KM
                {
                    CellOut {
                        plate: out.plate,
                        ..keep_cell(c, 0, out.collided)
                    }
                } else {
                    out
                }
            })
            .collect_into_vec(&mut outs);

        // Seam rule, half 3 (WO-0008 S1): connectivity-preserving
        // consumption. A per-cell gather can pinch off pieces of a plate —
        // a ragged bite encloses a bay, or a front severs a neck — and the
        // §7 backstop then teleports the piece to a neighbor plate
        // (measured: 25–60k cells per 2 Gy at L6, most of the backstop
        // budget). Rigid lithosphere does not do that: the bridge holds
        // until the piece is consumed face-first. So: while any plate's
        // NEW ownership has a fragment, revert the ownership flips that
        // caused it — every changed cell inside the fragment, and every
        // cell adjacent to it that changed away from the fragment's
        // plate. Reverting only ever un-does this step's changes, so the
        // loop strictly shrinks the changed set and terminates. NO
        // severed piece becomes a plate here — promoting them was tried
        // two ways during WO-0008 S1 and both railed the census (every
        // plate-scale piece: 60-plate cap in 2 Gy; Farallon-signature
        // slices only: 42 alive and climbing, because the old
        // fragment-absorption that silently killed microplates is gone).
        // A severed piece instead stays attached through the reverted
        // neck and is consumed face-first over the following steps.
        // Serial and id-ordered — deterministic.
        {
            let mut comp_of = vec![u32::MAX; n];
            let mut queue: VecDeque<u32> = VecDeque::new();
            for pass in 0.. {
                for v in comp_of.iter_mut() {
                    *v = u32::MAX;
                }
                let mut comp_plate: Vec<u32> = Vec::new();
                let mut comp_cells: Vec<Vec<u32>> = Vec::new();
                for c0 in 0..n {
                    if comp_of[c0] != u32::MAX {
                        continue;
                    }
                    let p = outs[c0].plate;
                    let ci = comp_plate.len() as u32;
                    comp_plate.push(p);
                    comp_cells.push(Vec::new());
                    comp_of[c0] = ci;
                    queue.push_back(c0 as u32);
                    while let Some(c) = queue.pop_front() {
                        comp_cells[ci as usize].push(c);
                        for &nb in grid.neighbors_of(c) {
                            let nbu = nb as usize;
                            if comp_of[nbu] == u32::MAX && outs[nbu].plate == p {
                                comp_of[nbu] = ci;
                                queue.push_back(nb);
                            }
                        }
                    }
                }
                let mut keep = vec![u32::MAX; self.plates.len()];
                for ci in 0..comp_plate.len() {
                    let p = comp_plate[ci] as usize;
                    if keep[p] == u32::MAX
                        || comp_cells[ci].len() > comp_cells[keep[p] as usize].len()
                    {
                        keep[p] = ci as u32;
                    }
                }
                let mut reverted = false;
                for ci in 0..comp_plate.len() {
                    let p = comp_plate[ci];
                    if keep[p as usize] == ci as u32 {
                        continue;
                    }
                    for &c in &comp_cells[ci] {
                        let cu = c as usize;
                        if outs[cu].plate != plate_id[cu] {
                            outs[cu] = keep_cell(cu, 0, NONE);
                            reverted = true;
                        }
                        for &nb in grid.neighbors_of(c) {
                            let nbu = nb as usize;
                            if plate_id[nbu] == p && outs[nbu].plate != p {
                                outs[nbu] = keep_cell(nbu, 0, NONE);
                                reverted = true;
                            }
                        }
                    }
                }
                if !reverted {
                    break;
                }
                if pass >= 63 {
                    log::warn!(
                        "t={} My: seam connectivity repair did not converge",
                        self.t_my
                    );
                    break;
                }
            }
        }

        // Continental inventory guard (WO-0008 S1 step 6): a jammed plate
        // keeps rotating (the commit already happened) but its front is
        // denied, so each committed step its trailing edge sheds hard
        // continental cells that nothing replaces — the jam grind that
        // drained continents (measured: −59% to −97% of continental area
        // over 2 Gy). Physically the pinned material stays put and the
        // convergence shortens the crust (S2's ledger turns that into
        // thickness). So, per committed plate: when this step destroyed
        // more of the plate's hard continental cells (same-plate
        // continent → ocean) than the plate gained in new continental
        // cells, revert the excess losses in ascending id order — the
        // block backs up instead of vanishing. A freely moving continent
        // is untouched (gains balance losses); rasterization drift is
        // healed as a side effect. Serial, id-ordered — deterministic.
        {
            let np = self.plates.len();
            let mut gains = vec![0u32; np];
            let mut losses: Vec<Vec<u32>> = vec![Vec::new(); np];
            for (c, o) in outs.iter().enumerate() {
                let p = o.plate as usize;
                // A gain is any new continental cell under the plate's
                // flag: ocean converting at the front, or a cross-plate
                // acquisition (a jam win or docking terrane). Since
                // WO-0008 S2 the jam grind is FUNDED — cross-plate
                // consumption is captured as underthrust budget and
                // deposited into the collision zone the same step — so
                // counting acquisitions no longer lets collisions leak
                // volume; excluding them (the S1 rule) made the guard
                // revert trailing losses whose columns had ALSO been
                // deposited, duplicating crust.
                if o.ctype == 1 && (prev_ctype[c] == 0 || plate_id[c] != o.plate) {
                    gains[p] += 1;
                } else if o.ctype == 0 && plate_id[c] == o.plate && prev_ctype[c] == 1 {
                    losses[p].push(c as u32);
                }
            }
            for p in 0..np {
                let excess = losses[p].len().saturating_sub(gains[p] as usize);
                if excess == 0 {
                    continue;
                }
                // Revert gap-ridge losses FIRST (stable partition, id order
                // within each half): the jam grind on a whole-continent
                // plate shows up as trailing gap-ridge cells, while an
                // in-plate rift corridor translating with its plate makes
                // paired single-cover losses and gains — reverting those
                // would re-fill the corridor with continent and no split
                // could ever complete (measured: splits went to zero).
                let (gap, other): (Vec<u32>, Vec<u32>) = losses[p]
                    .iter()
                    .partition(|&&c| outs[c as usize].features & F_RIDGE != 0);
                for &c in gap.iter().chain(other.iter()).take(excess) {
                    outs[c as usize] = keep_cell(c as usize, 0, NONE);
                }
            }
        }

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
            // Underthrusting capture (WO-0008 S2): continental volume this
            // cell loses to ANOTHER plate at a tracked continent-continent
            // contact goes into that pair's budget — apply_collisions
            // deposits it in the pair's distributed zone this same step
            // (India's crust does not vanish under Tibet; it thickens it).
            if self.crust_type[c] == 1 && o.plate != self.plate_id[c] {
                // The removed crust is the loser's WHOLE column: the
                // winner's incoming content is a conserved shift of its
                // own crust, while the loser's column goes down as the
                // underthrust slab (all of India's crust feeds Tibet,
                // not just the thickness difference).
                let prev_q = (self.thickness[c] * 100.0).round() as i64;
                {
                    let (a, b) = (
                        self.plate_id[c].min(o.plate),
                        self.plate_id[c].max(o.plate),
                    );
                    if prev_q > 0 && self.collisions.iter().any(|t| t.a == a && t.b == b) {
                        let lost = prev_q;
                        self.underthrust_removed_q += lost;
                        match self
                            .underthrust_budget
                            .iter_mut()
                            .find(|e| e.0 == a && e.1 == b)
                        {
                            Some(e) => e.2 += lost,
                            None => self.underthrust_budget.push((a, b, lost)),
                        }
                    }
                }
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
        youngest_breakup_my: f32,
    ) -> u32 {
        let id = self.plates.len() as u32;
        self.plates.push(PlateState {
            id,
            alive: true,
            pole,
            speed_deg_my: speed,
            youngest_suture_my,
            youngest_rift_my,
            youngest_breakup_my,
            quiet_my: 0.0,
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
            // §6 severed slice (widened WO-0008 S1): a plate-scale piece
            // cut off by consumption becomes its own plate (inheriting the
            // parent's motion; the force balance owns it from the next
            // step), whatever its crust content — the connecting
            // lithosphere is gone, so it IS mechanically independent.
            // Origin is labeled TrenchTrapped when it carries the classic
            // Farallon signature (pure young ocean against the trench that
            // cut it), Severed otherwise. Sub-plate-scale pieces no longer
            // reach this pass at all: advect's repair half of the seam
            // rule keeps them attached, so what remains here is seam
            // noise for the backstop below.
            let micro_min =
                MICRO_MIN_CELLS.max((self.plate_id.len() as f32 * MICRO_MIN_FRACTION) as u32);
            if cells.len() as u32 >= micro_min && alive < MAX_ALIVE_PLATES {
                let oceanic = cells.iter().all(|&c| self.crust_type[c as usize] == 0);
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
                {
                    let parent = &self.plates[p as usize];
                    let (pole, speed, ys, yr, yb) = (
                        parent.pole,
                        parent.speed_deg_my,
                        parent.youngest_suture_my,
                        parent.youngest_rift_my,
                        parent.youngest_breakup_my,
                    );
                    let id = self.spawn_plate(pole, speed, ys, yr, yb);
                    for &c in &frag_cells[ci] {
                        self.plate_cells[self.plate_id[c as usize] as usize] -= 1;
                        self.plate_cells[id as usize] += 1;
                        self.plate_id[c as usize] = id;
                    }
                    self.events.push(TectonicEvent::Microplate {
                        id,
                        origin: if against_trench {
                            MicroplateOrigin::TrenchTrapped
                        } else {
                            MicroplateOrigin::Severed
                        },
                        t: self.t_my,
                    });
                    self.microplate_count += 1;
                    alive += 1;
                    log::debug!(
                        "t={} My: severed microplate {id} off plate {p} ({} cells)",
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
        // Discrete island-arc sites (WO-0008 S2): over an OCEANIC
        // overrider, edifices grow only at sites chosen as a greedy
        // maximal independent set in cell-id order — no two sites
        // adjacent, so islands surface one by one over tens of My and
        // stay one cell wide (Marianas, Aleutians); between sites the
        // band gains ARC_OFFSITE_FRACTION of the site rate and mostly
        // stays submarine. A CONTINENTAL overrider keeps the continuous
        // margin arc — that is the Andes, and it is correct.
        let mut is_site = vec![false; self.bfs_depth.len()];
        for c in 0..self.bfs_depth.len() {
            let d = self.bfs_depth[c];
            if d < ring_lo || d > ring_hi || self.crust_type[c] != 0 {
                continue;
            }
            // A site needs ARC_SITE_RINGS of clearance from earlier
            // sites and from young islands.
            let mut blocked = false;
            let mut frontier: Vec<u32> = vec![c as u32];
            let mut seen: Vec<u32> = vec![c as u32];
            'rings: for _ in 0..ARC_SITE_RINGS {
                let mut next: Vec<u32> = Vec::new();
                for &f in &frontier {
                    for &nb in self.grid.neighbors_of(f) {
                        let nbu = nb as usize;
                        if seen.contains(&nb) {
                            continue;
                        }
                        seen.push(nb);
                        next.push(nb);
                        if (nbu < c && is_site[nbu])
                            || (self.crust_type[nbu] == 1
                                && self.crust_age[nbu] < ARC_SITE_ISLAND_BLOCK_MY)
                        {
                            blocked = true;
                            break 'rings;
                        }
                    }
                }
                frontier = next;
            }
            is_site[c] = !blocked;
        }
        for (c, &d) in self.bfs_depth.iter().enumerate() {
            if d >= ring_lo && d <= ring_hi {
                self.features[c] |= F_ARC;
                let rate = if self.crust_type[c] != 0 {
                    ARC_GROWTH_CONT_KM_MY
                } else if is_site[c] {
                    ARC_GROWTH_OCEAN_KM_MY
                } else {
                    ARC_GROWTH_OCEAN_KM_MY * ARC_OFFSITE_FRACTION
                };
                let t = (self.thickness[c] + rate * DT_MY).min(THICKNESS_CAP_KM);
                self.thickness[c] = t;
                self.orogeny_age[c] = 0.0;
                // Conversion is also island-blocked: an edifice beside a
                // young island stays a submarine seamount (the wall
                // never assembles); it converts once the neighbor island
                // has matured.
                let beside_young_island = self.grid.neighbors_of(c as u32).iter().any(|&nb| {
                    let nbu = nb as usize;
                    self.crust_type[nbu] == 1
                        && self.crust_age[nbu] < ARC_SITE_ISLAND_BLOCK_MY
                });
                if self.crust_type[c] == 0 && t >= ISLAND_ARC_CONVERT_KM && !beside_young_island
                {
                    self.crust_type[c] = 1; // island arc: young continental crust
                    self.crust_age[c] = 0.0;
                    self.cont_gained_by_arc += 1;
                }
            }
        }
    }

    /// Total continental crustal volume in exact quantized units of
    /// 0.01 km × cell (serial, id order): the crust-volume ledger's
    /// measuring stick (WO-0008 S2).
    fn cont_volume_q(&self) -> i64 {
        let mut v = 0i64;
        for c in 0..self.crust_type.len() {
            if self.crust_type[c] == 1 {
                v += (self.thickness[c] * 100.0).round() as i64;
            }
        }
        v
    }

    /// Distributed continental-collision thickening (WO-0008 S2). The old
    /// one-cell Andes wall is gone: each colliding pair's UNDERTHRUST
    /// BUDGET — continental volume its collision margin consumed this
    /// step, captured by advect — is deposited across a deformation zone
    /// walked inboard from every contact cell: zone depth
    /// W = W_BASE_KM / strength (3–8 cells for weak crust at L6), the
    /// walk stopping where strength reaches CRATON_STOP (deformation dies
    /// at cratons — Tarim, Sichuan), deposits linearly tapered from the
    /// front. The collision path creates NO volume: every 0.01 km·cell
    /// removed at the margin reappears in the zone, exactly, same step —
    /// India underthrusting Tibet doubles crust inboard. Cells at the
    /// thickness cap absorb nothing; budget nothing can absorb is booked
    /// as spilled. Serial and id-ordered throughout.
    fn apply_collisions(&mut self) {
        let n = self.plate_id.len();
        // Contact cells (both sides) per colliding pair, id order, plus
        // orogeny-age resets: an actively converging contact keeps its
        // orogen young whether or not budget arrives this step.
        let mut pair_contacts: Vec<(u32, u32, Vec<u32>)> = Vec::new();
        let mut front: Vec<u32> = Vec::new();
        for c in 0..n {
            let cl = &self.class[c];
            let collided_here = self.outs.get(c).is_some_and(|o| o.collided != NONE);
            if self.crust_type[c] != 1 || (cl.contact_partner == NONE && !collided_here) {
                continue;
            }
            if cl.conv_cont_cmyr > 0.0 || collided_here {
                // Step-7 guard: continent-collision thickening must never
                // sit on an ocean-ocean contact — classify only sets
                // contact_partner on continent-continent edges and the
                // filter above requires this cell continental.
                debug_assert!(
                    self.crust_type[c] == 1,
                    "collision thickening reached oceanic crust at cell {c}"
                );
                self.orogeny_age[c] = 0.0;
                front.push(c as u32);
            }
            let partner = if cl.contact_partner != NONE {
                cl.contact_partner
            } else {
                self.outs[c].collided
            };
            let p = self.plate_id[c];
            let (a, b) = (p.min(partner), p.max(partner));
            match pair_contacts.iter_mut().find(|e| e.0 == a && e.1 == b) {
                Some(e) => e.2.push(c as u32),
                None => pair_contacts.push((a, b, vec![c as u32])),
            }
        }
        let budgets = std::mem::take(&mut self.underthrust_budget);
        if budgets.iter().all(|&(_, _, q)| q <= 0) {
            return;
        }
        // Depth-from-front over same-plate continental crust, for the
        // inboard walk.
        let mut depth = vec![u16::MAX; n];
        let mut queue: VecDeque<u32> = VecDeque::new();
        for &c in &front {
            depth[c as usize] = 0;
            queue.push_back(c);
        }
        while let Some(c) = queue.pop_front() {
            let dc = depth[c as usize];
            if dc >= W_MAX_CELLS as u16 {
                continue;
            }
            let p = self.plate_id[c as usize];
            for &nb in self.grid.neighbors_of(c) {
                let nbu = nb as usize;
                if depth[nbu] == u16::MAX && self.plate_id[nbu] == p && self.crust_type[nbu] == 1
                {
                    depth[nbu] = dc + 1;
                    queue.push_back(nb);
                }
            }
        }
        let cap_q = (THICKNESS_CAP_KM * 100.0).round() as i64;
        for (a, b, budget_q) in budgets {
            if budget_q <= 0 {
                continue;
            }
            let Some(contacts) = pair_contacts
                .iter()
                .find(|e| e.0 == a && e.1 == b)
                .map(|e| e.2.clone())
            else {
                // The pair lost its contact this very step: nothing to
                // deposit into — the volume spills (rare, reported).
                self.underthrust_spilled_q += budget_q;
                continue;
            };
            // The pair's zone: per contact cell, walk inboard up to W
            // cells (strength-limited, craton-stopped) with linearly
            // tapered weights (W' − i); shared cells add weight.
            let mut zone: Vec<(u32, u32)> = Vec::new(); // (cell, weight)
            for &c0 in &contacts {
                let w_cells = ((W_BASE_KM
                    / (self.strength(c0 as usize) * self.cell_spacing_km))
                    .round() as u32)
                    .clamp(1, W_MAX_CELLS);
                let mut cur = c0;
                let mut walk: Vec<u32> = vec![c0];
                for _ in 1..w_cells {
                    let mut nbs: Vec<u32> = self.grid.neighbors_of(cur).to_vec();
                    nbs.sort_unstable();
                    let mut best: Option<(u32, u16)> = None;
                    for &nb in &nbs {
                        let nbu = nb as usize;
                        if self.plate_id[nbu] != self.plate_id[c0 as usize]
                            || self.crust_type[nbu] != 1
                            || depth[nbu] == u16::MAX
                            || depth[nbu] <= depth[cur as usize]
                            || self.strength(nbu) >= CRATON_STOP
                        {
                            continue;
                        }
                        if best.is_none_or(|(_, bd)| depth[nbu] > bd) {
                            best = Some((nb, depth[nbu]));
                        }
                    }
                    let Some((nb, _)) = best else { break };
                    walk.push(nb);
                    cur = nb;
                }
                let w_len = walk.len() as u32;
                for (i, &c) in walk.iter().enumerate() {
                    let weight = w_len - i as u32; // linear taper
                    match zone.iter_mut().find(|e| e.0 == c) {
                        Some(e) => e.1 += weight,
                        None => zone.push((c, weight)),
                    }
                }
            }
            zone.sort_unstable();
            // Half the budget loads the pair's foreland shelf directly —
            // a collision builds its plateau AND its foreland fill
            // together, and the foreland conversions return the AREA the
            // margin consumed (without this the area budget bled 20-40%
            // over 2 Gy while volume sat parked in the zone).
            let mut remaining = budget_q / 3;
            let mut foreland_budget = budget_q - budget_q / 3;
            while remaining > 0 {
                let mut weight_sum = 0i64;
                for &(c, w) in &zone {
                    if ((self.thickness[c as usize] * 100.0).round() as i64) < cap_q {
                        weight_sum += w as i64;
                    }
                }
                if weight_sum == 0 {
                    break;
                }
                let mut deposited_any = false;
                for &(c, w) in &zone {
                    if remaining <= 0 {
                        break;
                    }
                    let cu = c as usize;
                    let before = (self.thickness[cu] * 100.0).round() as i64;
                    let room = cap_q - before;
                    if room <= 0 {
                        continue;
                    }
                    let share = (remaining * w as i64 / weight_sum)
                        .clamp(1, room)
                        .min(remaining);
                    self.thickness[cu] = ((before + share) as f32) * 0.01;
                    let after = (self.thickness[cu] * 100.0).round() as i64;
                    let got = after - before;
                    self.orogeny_age[cu] = 0.0;
                    remaining -= got;
                    self.underthrust_deposited_q += got;
                    if got > 0 {
                        deposited_any = true;
                    }
                }
                if !deposited_any {
                    break;
                }
            }
            foreland_budget += remaining.max(0);
            let mut remaining = foreland_budget;
            if remaining > 0 {
                // Foreland loading: oceanic same-plate cells adjacent to
                // the zone (id order), converted one full column at a
                // time.
                let mut shelf: Vec<u32> = Vec::new();
                for &(zc, _) in &zone {
                    for &nb in self.grid.neighbors_of(zc) {
                        let nbu = nb as usize;
                        if self.crust_type[nbu] == 0
                            && self.plate_id[nbu] == self.plate_id[zc as usize]
                            && !shelf.contains(&nb)
                        {
                            shelf.push(nb);
                        }
                    }
                }
                shelf.sort_unstable();
                let cont_q = (SUBDUCTIBLE_CONT_KM * 100.0).round() as i64;
                for &sc in &shelf {
                    let cu = sc as usize;
                    let before = (self.thickness[cu] * 100.0).round() as i64;
                    let need = cont_q - before;
                    if need <= 0 || remaining < need {
                        continue;
                    }
                    // One full column at a time: the cell converts within
                    // the same phase, so the ledger stays exact — the
                    // spilled load supplies `need`, the cell's own oceanic
                    // column (`before`) is incorporated basement.
                    self.thickness[cu] = (cont_q as f32) * 0.01;
                    self.crust_type[cu] = 1;
                    // The foreland is consolidated margin lithosphere
                    // under fresh load: platform-grade strength, never
                    // juvenile — age-0 (and young-shelf) forelands turned
                    // into weak-line attractors and the split rate
                    // doubled.
                    self.crust_age[cu] = self.crust_age[cu].max(300.0);
                    self.orogeny_age[cu] = self.crust_age[cu];
                    remaining -= need;
                    self.underthrust_deposited_q += need;
                    self.underthrust_incorporated_q += before;
                    if remaining <= 0 {
                        break;
                    }
                }
                if remaining > 0 {
                    self.underthrust_spilled_q += remaining;
                }
            }
        }
    }

    /// Gravitational spreading (WO-0008 S2): one diffusion pass per step —
    /// each continental cell above SPREAD_THRESHOLD_KM moves excess
    /// thickness toward its thinnest same-plate continental neighbor at
    /// SPREAD_KM_MY per km of excess (downhill only, never past the
    /// midpoint, receiver honors the cap). Lower-crustal channel flow:
    /// walls become plateaus (Tibet). Serial, id-ordered, in place —
    /// deterministic.
    fn apply_spreading(&mut self) {
        let n = self.plate_id.len();
        for c in 0..n {
            if self.crust_type[c] != 1 || self.thickness[c] <= SPREAD_THRESHOLD_KM {
                continue;
            }
            // The thinnest same-plate neighbor, continental OR oceanic:
            // collapse also spreads crust over the plate's own foreland
            // shelf (Tibet extrudes; orogens shed nappes onto their
            // margins) — the reverse of the collision's area→thickness
            // trade, and what keeps continental AREA in balance while
            // funded underthrusting consumes it at the fronts.
            let mut nbs: Vec<u32> = self.grid.neighbors_of(c as u32).to_vec();
            nbs.sort_unstable();
            let mut best: Option<(usize, f32)> = None;
            for &nb in &nbs {
                let nbu = nb as usize;
                if self.plate_id[nbu] != self.plate_id[c] {
                    continue;
                }
                if best.is_none_or(|(_, bt)| self.thickness[nbu] < bt) {
                    best = Some((nbu, self.thickness[nbu]));
                }
            }
            let Some((nbu, nb_t)) = best else { continue };
            if nb_t >= self.thickness[c] {
                continue;
            }
            let excess = self.thickness[c] - SPREAD_THRESHOLD_KM;
            let flow = (SPREAD_KM_MY * excess * DT_MY)
                .min(0.5 * (self.thickness[c] - nb_t))
                .min(THICKNESS_CAP_KM - nb_t)
                .max(0.0);
            self.thickness[c] -= flow;
            self.thickness[nbu] += flow;
            // A loaded shelf cell becomes continent once it carries a
            // full continental column — old margin lithosphere under
            // fresh load, so it keeps platform-grade strength.
            if self.crust_type[nbu] == 0 && self.thickness[nbu] >= SUBDUCTIBLE_CONT_KM {
                self.crust_type[nbu] = 1;
                self.crust_age[nbu] = self.crust_age[nbu].max(300.0);
                self.orogeny_age[nbu] = self.crust_age[nbu];
            }
        }
    }

    /// Rift thinning and oceanization (the rift half of the former
    /// apply_collisions_and_rifts).
    fn apply_rifts(&mut self) {
        let n = self.plate_id.len();
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
            let abs_cells = SUTURE_ABS_CONTACT_KM / self.cell_spacing_km;
            let extent_ok = small_contact as f32 >= SUTURE_CONTACT_FRACTION * perimeter as f32
                || small_contact as f32 >= abs_cells;
            // Condition 2: kinematically locked.
            let mean_rel = e.rel_sum / e.rel_n.max(1) as f32;
            let locked = mean_rel < SUTURE_LOCK_CMYR;
            // Relic-basin closure (WO-0008 S1, model §3 addendum): while
            // the pair is locked (conditions 1 + 2), enclosed basins near
            // the contact are consumed at their margins — the terminal
            // closure that lets condition 3 eventually pass. Runs BEFORE
            // condition 3 so this step's consumption counts.
            if extent_ok && locked {
                let rate_cmyr = mean_rel.max(CLASSIFY_CMYR);
                self.consume_relic_basins(&e.contact_cells, e.a, e.b, rate_cmyr);
            }
            // Condition 3 (checked last — it walks rings): ocean closed up
            // to relic seas.
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
            let old = self.collisions.iter().find(|t| t.a == e.a && t.b == e.b);
            let old_slow = old.map(|t| t.slow_collision_my).unwrap_or(0.0);
            let old_locked = old.map(|t| t.locked_my).unwrap_or(0.0);
            // Both pair clocks decay at 2x instead of hard-resetting on a
            // lapse (WO-0008 S1): the zigzag hex boundary sprays one-step
            // classification flickers (the documented advection problem
            // the rift-onset clock already guards against with the same
            // 2x hysteresis), and a hard reset let a single flicker erase
            // 28 My of accumulated lock — welds structurally undershot the
            // 2/Gy floor at seed cyrus in every calibration. A real unlock
            // still drains the clock in half the time it took to build.
            let decay = |v: f32| (v - RIFT_DECAY_MULT * DT_MY).max(0.0);
            let t = if holds {
                old_slow + DT_MY
            } else {
                decay(old_slow)
            };
            if t >= SUTURE_AFTER_MY && matured.is_none() {
                matured = Some(i);
            }
            next.push(PairTimer {
                a: e.a,
                b: e.b,
                slow_collision_my: t,
                locked_my: if extent_ok && locked {
                    old_locked + DT_MY
                } else {
                    decay(old_locked)
                },
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
            contact_cells: small_contact,
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

    /// Oceanic cells within SUTURE_OCEAN_RINGS rings of the contact on
    /// either plate (serial BFS seeded in contact order): the shared
    /// window of condition 3 and relic-basin closure.
    fn ocean_near_contact(&self, contact_cells: &[u32], a: u32, b: u32) -> Vec<u32> {
        let n = self.grid.cell_count() as usize;
        let mut depth = vec![u16::MAX; n];
        let mut queue: VecDeque<u32> = VecDeque::new();
        let mut window_ocean: Vec<u32> = Vec::new();
        for &c in contact_cells {
            if depth[c as usize] != u16::MAX {
                continue;
            }
            depth[c as usize] = 0;
            queue.push_back(c);
            if self.crust_type[c as usize] == 0 {
                window_ocean.push(c);
            }
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
                    depth[nbu] = dc + 1;
                    queue.push_back(nb);
                    if self.crust_type[nbu] == 0 {
                        window_ocean.push(nb);
                    }
                }
            }
        }
        window_ocean
    }

    /// The connected oceanic region containing `seed` (any plate), plus
    /// its enclosure stats for pair (a, b): bordering-continental edge
    /// count and the share of those on the pair. Serial BFS, fixed order.
    fn oceanic_region(
        &self,
        seed: u32,
        a: u32,
        b: u32,
        visited: &mut [bool],
        queue: &mut VecDeque<u32>,
    ) -> (Vec<u32>, u32, u32) {
        let mut region: Vec<u32> = Vec::new();
        visited[seed as usize] = true;
        queue.push_back(seed);
        while let Some(c) = queue.pop_front() {
            region.push(c);
            for &nb in self.grid.neighbors_of(c) {
                let nbu = nb as usize;
                if !visited[nbu] && self.crust_type[nbu] == 0 {
                    visited[nbu] = true;
                    queue.push_back(nb);
                }
            }
        }
        let (mut border, mut border_ab) = (0u32, 0u32);
        for &c in &region {
            for &nb in self.grid.neighbors_of(c) {
                let nbu = nb as usize;
                if self.crust_type[nbu] == 1 {
                    border += 1;
                    let p = self.plate_id[nbu];
                    if p == a || p == b {
                        border_ab += 1;
                    }
                }
            }
        }
        (region, border, border_ab)
    }

    /// §3 condition 3 (amended WO-0008 S1): no ENCLOSED oceanic region —
    /// bordering continent ≥ `RELIC_ENCLOSED_FRACTION` on the pair —
    /// larger than `RELIC_BASIN_KEEP_CELLS` within SUTURE_OCEAN_RINGS
    /// rings of the contact on either plate. Relic seas no longer block
    /// the weld, and neither does open ocean off the contact's flanks:
    /// suturing is about the intervening ocean between the two margins
    /// (India–Asia welded with the Indian Ocean right beside the front),
    /// and "intervening" is exactly what the enclosure test measures —
    /// the same test relic-basin closure uses, so whatever blocks here is
    /// what closure is consuming. Serial BFS in fixed order.
    fn ocean_closed(&self, contact_cells: &[u32], a: u32, b: u32) -> bool {
        let window_ocean = self.ocean_near_contact(contact_cells, a, b);
        let n = self.grid.cell_count() as usize;
        let mut visited = vec![false; n];
        let mut queue: VecDeque<u32> = VecDeque::new();
        for &c0 in &window_ocean {
            if visited[c0 as usize] {
                continue;
            }
            let (region, border, border_ab) =
                self.oceanic_region(c0, a, b, &mut visited, &mut queue);
            if region.len() as u32 > RELIC_BASIN_KEEP_CELLS
                && border > 0
                && border_ab as f32 >= RELIC_ENCLOSED_FRACTION * border as f32
            {
                return false;
            }
        }
        true
    }

    /// Relic-basin closure (WO-0008 S1, model §3 addendum). For a locked
    /// pair (conditions 1 + 2): every ENCLOSED oceanic basin near the
    /// contact — a connected oceanic region whose bordering continental
    /// cells belong ≥ `RELIC_ENCLOSED_FRACTION` to the two plates — is
    /// consumed at its margin cells (basin cells of the pair touching
    /// continent, ascending id) at the pair's convergence-equivalent rate:
    /// margin advance of `rate_cmyr` per step across the whole margin,
    /// floored at one cell per basin per step. Each consumed cell becomes
    /// young continental margin crust of its own plate (thickness copied
    /// from its lowest-id continental neighbor) and the consumed ocean
    /// feeds that plate's slab ledger — internal subduction under the
    /// plate's own margin (Mediterranean-style terminal closure). A basin
    /// is never consumed below `RELIC_BASIN_KEEP_CELLS`: what remains is a
    /// relic sea (Caspian / Black Sea). Serial and id-ordered.
    fn consume_relic_basins(&mut self, contact_cells: &[u32], a: u32, b: u32, rate_cmyr: f32) {
        let window_ocean = self.ocean_near_contact(contact_cells, a, b);
        if window_ocean.is_empty() {
            return;
        }
        let n = self.grid.cell_count() as usize;
        // cm/yr → km/My is ×10; margin cells consumed per margin cell per
        // step at this level.
        let frac = rate_cmyr * 10.0 * DT_MY / self.cell_spacing_km;
        let mut visited = vec![false; n];
        let mut queue: VecDeque<u32> = VecDeque::new();
        for &seed in &window_ocean {
            if visited[seed as usize] {
                continue;
            }
            // The seed's full connected oceanic region (any plate), with
            // the enclosure stats counted per basin-edge (deterministic; a
            // border cell shared by several basin cells simply weighs
            // more, which is fine).
            let (region, border, border_ab) =
                self.oceanic_region(seed, a, b, &mut visited, &mut queue);
            let size = region.len() as u32;
            if size <= RELIC_BASIN_KEEP_CELLS {
                continue; // already a relic sea
            }
            if border == 0 || (border_ab as f32) < RELIC_ENCLOSED_FRACTION * border as f32 {
                continue; // open ocean or another pair's basin
            }
            // Margin cells of the pair's own crust, ascending id.
            let mut margin: Vec<u32> = region
                .iter()
                .copied()
                .filter(|&c| {
                    let p = self.plate_id[c as usize];
                    (p == a || p == b)
                        && self
                            .grid
                            .neighbors_of(c)
                            .iter()
                            .any(|&nb| self.crust_type[nb as usize] == 1)
                })
                .collect();
            margin.sort_unstable();
            if margin.is_empty() {
                continue;
            }
            let want = ((margin.len() as f32 * frac).round() as u32).max(2);
            let n_consume = want.min(size - RELIC_BASIN_KEEP_CELLS) as usize;
            // (plate, cells, age sum) for the slab segments, id-ordered.
            let mut consumed: Vec<(u32, u32, f32)> = Vec::new();
            for &c in margin.iter().take(n_consume) {
                let cu = c as usize;
                let own = self.plate_id[cu];
                // Thickness template: lowest-id continental neighbor (the
                // margin the basin underthrusts).
                let mut nbs: Vec<u32> = self.grid.neighbors_of(c).to_vec();
                nbs.sort_unstable();
                let Some(donor) = nbs
                    .iter()
                    .copied()
                    .find(|&nb| self.crust_type[nb as usize] == 1)
                else {
                    continue;
                };
                match consumed.iter_mut().find(|e| e.0 == own) {
                    Some(e) => {
                        e.1 += 1;
                        e.2 += self.crust_age[cu];
                    }
                    None => consumed.push((own, 1, self.crust_age[cu])),
                }
                self.cont_gained_by_closure += 1;
                self.crust_type[cu] = 1;
                self.thickness[cu] = self.thickness[donor as usize];
                self.crust_age[cu] = 0.0;
                self.orogeny_age[cu] = 0.0;
                self.rift_age[cu] = 0.0;
                self.slab_plate[cu] = own as u16;
                self.slab_since_my[cu] = self.t_my;
            }
            consumed.sort_unstable_by_key(|e| e.0);
            for (pid, cnt, age_sum) in consumed {
                self.plates[pid as usize].slab.push(SlabSegment {
                    area_cells: cnt,
                    age_at_subduction_my: age_sum / cnt as f32,
                    subducted_at_my: self.t_my,
                    attached: true,
                });
            }
        }
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
                let mut tip = if tip_b { r.tip_b } else { r.tip_a };
                let mut done = if tip_b { r.done_b } else { r.done_a };
                for _ in 0..self.rift_prop_cells {
                    // Re-anchor: the plate may have advected out from under
                    // the stored tip id — follow the scar to a plate cell.
                    if self.plate_id[tip as usize] != r.plate {
                        let anchor = self.grid.neighbors_of(tip).iter().copied().find(|&nb| {
                            self.plate_id[nb as usize] == r.plate
                                && self.rift_age[nb as usize] >= RIFT_ONSET_MY
                        });
                        match anchor {
                            Some(nb) => tip = nb,
                            None => {
                                failed = true;
                                break;
                            }
                        }
                    }
                    // Reached the plate boundary: this tip is finished.
                    if self
                        .grid
                        .neighbors_of(tip)
                        .iter()
                        .any(|&nb| self.plate_id[nb as usize] != r.plate)
                    {
                        done = true;
                        break;
                    }
                    // Walk on: weakness GATES (amendment A: only cells the
                    // driver stress can break are walkable), the stress
                    // axis STEERS (WO-0008 S1): each tip advances into the
                    // walkable neighbor most aligned with the direction
                    // away from the OTHER tip, so the rift crosses the
                    // plate instead of curling out through the nearest
                    // margin — the m4 sliver problem: least-strength
                    // steering never dropped the supercontinent below the
                    // 1/3 insulation threshold and the breakup engine
                    // stayed permanently armed. Cracks propagate along the
                    // stress axis, deflecting into weak zones only through
                    // the gate. Ties → weaker cell → lower id.
                    let other_tip = if tip_b { r.tip_a } else { r.tip_b };
                    let axis = if tip != other_tip {
                        Some(normalize3(sub3(
                            self.grid.positions[tip as usize],
                            self.grid.positions[other_tip as usize],
                        )))
                    } else {
                        None // first advance: pure least strength
                    };
                    let mut nbs: Vec<u32> = self.grid.neighbors_of(tip).to_vec();
                    nbs.sort_unstable();
                    let mut best: Option<(u32, f32, f32)> = None; // (cell, align, strength)
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
                        if r.stress <= s {
                            continue; // the gate: stress cannot break it
                        }
                        let a = match axis {
                            Some(ax) => dot3(
                                ax,
                                normalize3(sub3(
                                    self.grid.positions[nbu],
                                    self.grid.positions[tip as usize],
                                )),
                            ),
                            None => 0.0,
                        };
                        let better = match best {
                            None => true,
                            Some((_, ba, bs)) => a > ba || (a == ba && s < bs),
                        };
                        if better {
                            best = Some((nb, a, s));
                        }
                    }
                    let Some((nb, _, _)) = best else {
                        failed = true; // amendment A: stalled = failed
                        break;
                    };
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
                    r.cells.push(nb);
                    tip = nb;
                }
                if tip_b {
                    r.tip_b = tip;
                    r.done_b = done;
                } else {
                    r.tip_a = tip;
                    r.done_a = done;
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

        // Rift linkage (WO-0008 S1, model §5 addendum): two rift systems
        // on the same plate (possible after a suture transfers the
        // loser's rift) whose active tips come within LINK range connect
        // along the least-strength path and merge into one system —
        // East Africa–Red Sea–Gulf of Aden style.
        self.link_rifts();

        // Nucleation: fixed driver order, one live rift per plate, a
        // refractory period after the plate's last rifting, and only where
        // driver stress beats the local strength (amendment A).
        let min_rift_cells = (self.plate_id.len() as f32 * MIN_RIFT_PLATE_FRACTION) as u32;
        for d in self.rift_drivers() {
            // One live rift per plate — except a supercontinental plate
            // (the amendment-B insulation case: > 1/3 of the world's
            // continental crust) may host a second arm (WO-0008 S1):
            // insulation-driven extension nucleates multiple arms, and
            // tip linkage can join them so a split halves the landmass
            // instead of shaving a sliver (the m4 sliver problem).
            let rift_cap = if self.cont_total_cells > 0
                && self.cont_cells_per_plate[d.plate as usize] as f32
                    > self.cont_total_cells as f32 * INSULATION_CONT_FRACTION
            {
                2
            } else {
                1
            };
            let live = self.rifts.iter().filter(|r| r.plate == d.plate).count();
            // The refractory models stress relief; while an arm is still
            // GROWING no split or failure has spent the plate's stress, so
            // the plate keeps probing for its second arm through the
            // growth phase (~30–80 My). Once every arm is complete and
            // waiting for oceanization the extension is localized at the
            // corridor and the refractory applies again — probing through
            // the whole 400 My waiting window cascaded the census to 39
            // at seed cyrus, while gating the second arm at all froze that
            // world at 0 splits.
            let growing = self
                .rifts
                .iter()
                .any(|r| r.plate == d.plate && !(r.done_a && r.done_b));
            let refractory = !growing
                && self.t_my - self.plates[d.plate as usize].youngest_rift_my < RIFT_REFRACTORY_MY;
            if !self.plates[d.plate as usize].alive
                || live >= rift_cap
                || self.plate_cells[d.plate as usize] < min_rift_cells
                || refractory
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
                cells: vec![d.cell],
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

    /// Rift linkage (WO-0008 S1): merge same-plate rift systems whose
    /// active (not-done) tips lie within `RIFT_LINK_CELLS` of each other,
    /// connecting the tips along the least-strength path — BFS depth from
    /// one tip, then a walk from the other through the least-strength
    /// neighbor one ring closer each hop (ties → lowest cell id). The
    /// merged system keeps the two FAR tips with their done flags, the
    /// stronger driver's stress, the earlier start time, and the first
    /// rift's kind; path cells are claimed like tip-walk cells. Restarts
    /// until no pair links — deterministic, id-ordered throughout.
    fn link_rifts(&mut self) {
        'again: loop {
            for i in 0..self.rifts.len() {
                for j in i + 1..self.rifts.len() {
                    if self.rifts[i].plate != self.rifts[j].plate
                        || !self.plates[self.rifts[i].plate as usize].alive
                    {
                        continue;
                    }
                    let (ri, rj) = (self.rifts[i].clone(), self.rifts[j].clone());
                    let tips_i = [(ri.tip_a, ri.done_a, false), (ri.tip_b, ri.done_b, true)];
                    let tips_j = [(rj.tip_a, rj.done_a, false), (rj.tip_b, rj.done_b, true)];
                    for (ti, done_i, i_is_b) in tips_i {
                        for (tj, done_j, j_is_b) in tips_j {
                            if done_i || done_j || ti == tj {
                                continue;
                            }
                            let Some(path) = self.link_path(ti, tj, ri.plate) else {
                                continue;
                            };
                            // Claim the connecting cells like tip-walk
                            // cells: continent jump-starts maturation,
                            // ocean gets a fresh ridge line.
                            let jump_start = RIFT_ONSET_MY + DT_MY;
                            for &c in &path {
                                let cu = c as usize;
                                if self.crust_type[cu] == 1 {
                                    self.rift_age[cu] = self.rift_age[cu].max(jump_start);
                                    self.features[cu] |= F_RIFT;
                                } else {
                                    self.crust_age[cu] = 0.0;
                                    self.features[cu] |= F_RIDGE;
                                }
                            }
                            // Merge j into i: i's linked tip is replaced
                            // by j's far tip.
                            let (far_tip, far_done) = if j_is_b {
                                (rj.tip_a, rj.done_a)
                            } else {
                                (rj.tip_b, rj.done_b)
                            };
                            let r = &mut self.rifts[i];
                            if i_is_b {
                                r.tip_b = far_tip;
                                r.done_b = far_done;
                            } else {
                                r.tip_a = far_tip;
                                r.done_a = far_done;
                            }
                            r.stress = r.stress.max(rj.stress);
                            r.started_my = r.started_my.min(rj.started_my);
                            r.cells.extend_from_slice(&path);
                            r.cells.extend_from_slice(&rj.cells);
                            let plate = r.plate;
                            self.rifts.remove(j);
                            self.rift_link_count += 1;
                            log::debug!("t={} My: linked two rifts on plate {plate}", self.t_my);
                            continue 'again;
                        }
                    }
                }
            }
            break;
        }
    }

    /// The least-strength connecting path between two rift tips within
    /// `RIFT_LINK_CELLS`, excluding the endpoints; None when they are
    /// farther apart than that on the plate. Cells already claimed by a
    /// rift or fresh corridor may be walked through (they cost nothing to
    /// re-claim).
    fn link_path(&self, from: u32, to: u32, plate: u32) -> Option<Vec<u32>> {
        let n = self.grid.cell_count() as usize;
        let mut depth = vec![u16::MAX; n];
        let mut queue: VecDeque<u32> = VecDeque::new();
        depth[to as usize] = 0;
        queue.push_back(to);
        while let Some(c) = queue.pop_front() {
            let dc = depth[c as usize];
            if dc >= RIFT_LINK_CELLS {
                continue;
            }
            for &nb in self.grid.neighbors_of(c) {
                let nbu = nb as usize;
                if depth[nbu] == u16::MAX && self.plate_id[nbu] == plate {
                    depth[nbu] = dc + 1;
                    queue.push_back(nb);
                }
            }
        }
        let d0 = depth[from as usize];
        if d0 == u16::MAX || d0 > RIFT_LINK_CELLS {
            return None;
        }
        let mut path = Vec::new();
        let mut cur = from;
        for d in (1..d0).rev() {
            let mut nbs: Vec<u32> = self.grid.neighbors_of(cur).to_vec();
            nbs.sort_unstable();
            let mut best: Option<(u32, f32)> = None;
            for &nb in &nbs {
                if depth[nb as usize] == d {
                    let s = self.strength(nb as usize);
                    if best.is_none_or(|(_, bs)| s < bs) {
                        best = Some((nb, s));
                    }
                }
            }
            let (nb, _) = best?;
            path.push(nb);
            cur = nb;
        }
        Some(path)
    }

    /// Fossil-boundary capture (WO-0008 S1): each plate below
    /// MICRO_MAX_FRACTION of the sphere accumulates a quiet clock while
    /// the mean relative speed over its entire boundary stays below the
    /// classification dead band; at `CAPTURE_AFTER_MY` its boundary has
    /// fossilized and it merges into the neighbor sharing the longest
    /// border. Kula-style capture: not a suture — no scar, no suture
    /// clock; ledger and rifts transfer like any merge. Serial, id-order.
    fn capture_fossilized_plates(&mut self) {
        let n = self.grid.cell_count() as usize;
        let small_max = (n as f32 * MICRO_MAX_FRACTION) as u32;
        let omegas: Vec<[f32; 3]> = (0..self.plates.len())
            .map(|pid| {
                if self.plates[pid].alive {
                    self.omega(pid as u32)
                } else {
                    [0.0; 3]
                }
            })
            .collect();
        // Mean boundary relative speed and border census per small plate.
        let np = self.plates.len();
        let mut rel_sum = vec![0.0f64; np];
        let mut rel_n = vec![0u32; np];
        let mut border: Vec<Vec<u32>> = vec![Vec::new(); np];
        let mut any_small = false;
        for (pid, b) in border.iter_mut().enumerate() {
            if self.plates[pid].alive && self.plate_cells[pid] <= small_max {
                any_small = true;
                *b = vec![0; np];
            }
        }
        if !any_small {
            return;
        }
        for c in 0..n {
            let a = self.plate_id[c];
            let au = a as usize;
            if border[au].is_empty() {
                continue; // not a small plate
            }
            let xa = self.grid.positions[c];
            for &nb in self.grid.neighbors_of(c as u32) {
                let b = self.plate_id[nb as usize];
                if b == a {
                    continue;
                }
                let mid = normalize3(add3(xa, self.grid.positions[nb as usize]));
                let rel = sub3(cross3(omegas[b as usize], mid), cross3(omegas[au], mid));
                rel_sum[au] += (dot3(rel, rel).sqrt() * RADMY_TO_CMYR) as f64;
                rel_n[au] += 1;
                border[au][b as usize] += 1;
            }
        }
        for pid in 0..np {
            if border[pid].is_empty() || rel_n[pid] == 0 {
                continue;
            }
            let quiet = (rel_sum[pid] / rel_n[pid] as f64) < CLASSIFY_CMYR as f64;
            if !quiet {
                self.plates[pid].quiet_my = 0.0;
                continue;
            }
            self.plates[pid].quiet_my += DT_MY;
            if self.plates[pid].quiet_my < CAPTURE_AFTER_MY {
                continue;
            }
            // Fossilized: merge into the longest-border neighbor (tie →
            // lowest plate id via strict >).
            let mut winner = u32::MAX;
            for (q, &cnt) in border[pid].iter().enumerate() {
                if cnt > 0
                    && self.plates[q].alive
                    && (winner == u32::MAX || cnt > border[pid][winner as usize])
                {
                    winner = q as u32;
                }
            }
            if winner == u32::MAX {
                continue;
            }
            let loser = pid as u32;
            let wu = winner as usize;
            for id in self.plate_id.iter_mut() {
                if *id == loser {
                    *id = winner;
                }
            }
            self.plate_cells[wu] += self.plate_cells[pid];
            self.plate_cells[pid] = 0;
            self.cont_cells_per_plate[wu] += self.cont_cells_per_plate[pid];
            self.cont_cells_per_plate[pid] = 0;
            self.plates[pid].alive = false;
            let segs = std::mem::take(&mut self.plates[pid].slab);
            self.plates[wu].slab.extend(segs);
            for r in self.rifts.iter_mut() {
                if r.plate == loser {
                    r.plate = winner;
                }
            }
            self.collisions.retain(|t| t.a != loser && t.b != loser);
            self.events.push(TectonicEvent::Capture {
                winner,
                loser,
                t: self.t_my,
            });
            log::debug!(
                "t={} My: fossilized plate {loser} captured by {winner}",
                self.t_my
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
            let r = self.rifts[ri].clone();
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
            // The split vents the parent's trapped mantle heat through the
            // fresh corridor (insulation anchor, WO-0008 S1).
            self.plates[pid as usize].youngest_breakup_my = self.t_my;
            let mut new_of_comp = vec![u32::MAX; comp_cells.len()];
            for &ci in &big {
                let child = self.spawn_plate(pole, speed, ys, self.t_my, self.t_my);
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
            youngest_breakup_my: super::super::keyframe::NEVER_SUTURED,
            quiet_my: 0.0,
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

    /// BFS depth from the two-plate contact, for placing test basins.
    fn contact_depth(s: &SimState) -> Vec<u16> {
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
        depth
    }

    fn grow_disc(s: &SimState, seed: u32, rings: u32) -> Vec<u32> {
        let mut cells = vec![seed];
        for _ in 0..rings {
            let mut next = cells.clone();
            for &c in &cells {
                next.extend_from_slice(s.grid.neighbors_of(c));
            }
            next.sort_unstable();
            next.dedup();
            cells = next;
        }
        cells
    }

    /// §3 condition 3 (amended WO-0008 S1) + relic-basin closure on a
    /// hand-built enclosed basin: a locked full-perimeter contact with a
    /// LARGE enclosed basin reaching the 2-ring window does not suture
    /// while the basin stays above the relic cap — and closure consumes
    /// its margins at the convergence-equivalent rate meanwhile, feeding
    /// the slab ledger.
    #[test]
    fn enclosed_basin_blocks_weld_while_closure_consumes_it() {
        let mut s = two_plate_cont_state();
        let depth = contact_depth(&s);
        // A 4-ring ocean disc around a center 6 rings in: ~61 cells,
        // reaching depth 2 (inside the condition-3 window), enclosed
        // entirely by the two colliding plates.
        let center = depth.iter().position(|&d| d == 6).unwrap() as u32;
        let disc = grow_disc(&s, center, 4);
        for &c in &disc {
            let cu = c as usize;
            s.crust_type[cu] = 0;
            s.thickness[cu] = OCEAN_THICKNESS_KM;
            s.crust_age[cu] = 80.0;
        }
        assert!(disc.len() as u32 > 3 * RELIC_BASIN_KEEP_CELLS);
        s.init_stats();

        // While the basin still reaches the 2-ring window, condition 3
        // blocks: no weld can fire inside the suture timer's first
        // SUTURE_AFTER_MY even though conditions 1+2 hold from step one.
        suture_steps(&mut s, 10);
        assert_eq!(
            s.alive_plates(),
            2,
            "an in-window enclosed basin above the relic cap must block"
        );
        assert_eq!(s.suture_count, 0);
        // The pair reads locked (conditions 1+2) even though 3 blocks...
        assert!(s
            .collisions
            .iter()
            .any(|t| t.a == 0 && t.b == 1 && t.locked_my > 0.0));
        // ...and closure has been eating the basin's margins meanwhile...
        let eaten_early = s.cont_gained_by_closure;
        assert!(eaten_early > 0, "closure must consume the enclosed basin");
        // ...as internal subduction: the consumed ocean is on the ledger.
        assert!(
            s.plates.iter().any(|p| !p.slab.is_empty()),
            "internal subduction must feed the slab ledger"
        );
        // Left locked long enough, the margin keeps retreating (and once
        // the basin is consumed below the cap or out of the window, the
        // weld resolves the collision — terminal closure).
        suture_steps(&mut s, 30);
        assert!(s.cont_gained_by_closure > eaten_early);
    }

    /// A basin small enough to be consumed down to the relic cap stops
    /// blocking: the weld fires and the relic sea survives at exactly
    /// `RELIC_BASIN_KEEP_CELLS` cells (Caspian-style).
    #[test]
    fn small_basin_becomes_relic_sea_and_weld_fires() {
        let mut s = two_plate_cont_state();
        let depth = contact_depth(&s);
        let center = depth.iter().position(|&d| d == 4).unwrap() as u32;
        // ~19-cell basin: closure trims it to the relic cap, then the
        // pair welds over it.
        let disc = grow_disc(&s, center, 2);
        for &c in &disc {
            let cu = c as usize;
            s.crust_type[cu] = 0;
            s.thickness[cu] = OCEAN_THICKNESS_KM;
            s.crust_age[cu] = 80.0;
        }
        assert!(disc.len() as u32 > RELIC_BASIN_KEEP_CELLS);
        s.init_stats();

        suture_steps(&mut s, 40);
        assert_eq!(s.alive_plates(), 1, "the relic sea must not block the weld");
        assert_eq!(s.suture_count, 1);
        let ocean_left = s.crust_type.iter().filter(|&&t| t == 0).count() as u32;
        assert_eq!(
            ocean_left, RELIC_BASIN_KEEP_CELLS,
            "the relic sea survives at the cap"
        );
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

    /// A two-plate jam world for the WO-0008 S2 collision tests: plate 1
    /// (x > 0) converges on stationary plate 0 about the +y pole, so the
    /// northern half of their shared meridian collides.
    fn jam_world(speed_0: f32, speed_1: f32) -> SimState {
        let grid = Arc::new(Grid::build(3));
        let n = grid.cell_count() as usize;
        let mut s = SimState::new_empty(&grid);
        // Both plates converge: a stationary plate never covers foreign
        // cells, so a one-sided push consumes nothing — the jam simply
        // denies the mover's advance.
        s.plates.push(test_plate(0, speed_0));
        s.plates[0].pole = [0.0, -1.0, 0.0];
        s.plates.push(test_plate(1, speed_1));
        s.plates[1].pole = [0.0, 1.0, 0.0];
        for c in 0..n {
            s.plate_id[c] = u32::from(grid.positions[c][0] > 0.0);
            s.crust_type[c] = 1;
            s.thickness[c] = 35.0;
            s.crust_age[c] = 300.0;
            s.orogeny_age[c] = 300.0;
        }
        s.t_my = 500.0;
        s.init_stats();
        s
    }

    /// WO-0008 S2: the distributed deformation zone stops at a synthetic
    /// craton, and the underthrust ledger balances exactly while the jam
    /// grinds. Sub-steps are driven manually with the continental census
    /// pinned below the amendment-B threshold each iteration (this test
    /// isolates the craton/zone mechanics from mantle insulation) and
    /// without relaxation, so the craton's thickness must be
    /// bit-identical, not just close.
    #[test]
    fn distributed_zone_stops_at_craton_and_ledger_balances() {
        let mut s = jam_world(1.0, 1.2);
        let n = s.grid.cell_count() as usize;
        let mut craton: Vec<usize> = Vec::new();
        for c in 0..n {
            let x = s.grid.positions[c][0];
            if (-0.45..-0.25).contains(&x) {
                s.crust_age[c] = 2500.0;
                s.orogeny_age[c] = 2500.0;
                s.thickness[c] = 43.0;
                craton.push(c);
            }
        }
        s.t_my = 3000.0;
        s.init_stats();
        s.cont_cells_per_plate = vec![10, 10];
        s.cont_total_cells = 100;
        assert!(s.strength(craton[0]) >= CRATON_STOP);
        let craton_before: Vec<f32> = craton.iter().map(|&c| s.thickness[c]).collect();
        let total_before: f32 = s.thickness.iter().sum();
        let mut collision_sum = 0i64;
        for _ in 0..100 {
            s.motion_update();
            s.advect();
            s.enforce_connectivity();
            s.classify_boundaries();
            s.accumulate_boundary_stats();
            s.cont_cells_per_plate = vec![10, 10];
            s.cont_total_cells = 100;
            let vb = s.cont_volume_q();
            s.apply_collisions();
            collision_sum += s.cont_volume_q() - vb;
            s.update_pair_timers_and_sutures();
            s.t_my += DT_MY;
        }
        // The ledger's exactness: what the margins lost, the zones got.
        assert!(s.underthrust_removed_q > 0, "the jam must consume margin");
        assert_eq!(
            s.underthrust_removed_q,
            s.underthrust_deposited_q + s.underthrust_spilled_q
        );
        assert_eq!(
            collision_sum,
            s.underthrust_deposited_q + s.underthrust_incorporated_q
        );
        // The step-6 conservation property is the exact continental
        // ledger asserted above (phase delta ≡ deposits + incorporated,
        // removed ≡ deposited + spilled); the all-cells thickness total
        // additionally must not run away (flip chains end in fresh
        // ridge-floor cells, so it drifts slightly rather than balancing
        // to zero).
        let total_after: f32 = s.thickness.iter().sum();
        assert!(
            (total_after - total_before).abs() / total_before < 0.03,
            "total thickness ran away: {total_before} -> {total_after}"
        );
        // Deposits never land on the craton. The craton CONTENT advects
        // with its plate (cell ids are grid-fixed), so find it by its
        // marker age: every cell carrying craton crust still reads its
        // setup thickness exactly, while somewhere off-craton the zone
        // visibly thickened.
        let mut craton_cells = 0;
        let mut zone_thickened = false;
        for c in 0..n {
            if s.crust_type[c] != 1 {
                continue;
            }
            if s.orogeny_age[c] >= 2400.0 {
                craton_cells += 1;
                assert_eq!(s.thickness[c], 43.0, "craton content at {c} deformed");
            } else if s.thickness[c] > 36.5 {
                zone_thickened = true;
            }
        }
        assert!(craton_cells > 10, "craton content lost ({craton_cells})");
        assert!(zone_thickened, "no deformation zone thickened");
        let _ = craton_before;
    }

    /// WO-0008 S2: one gravitational-spreading pass conserves total
    /// thickness and lowers the peak.
    #[test]
    fn spreading_conserves_thickness_and_lowers_peaks() {
        let grid = Arc::new(Grid::build(3));
        let n = grid.cell_count() as usize;
        let mut s = SimState::new_empty(&grid);
        s.plates.push(test_plate(0, 0.0));
        for c in 0..n {
            s.plate_id[c] = 0;
            s.crust_type[c] = 1;
            s.thickness[c] = 35.0;
            s.crust_age[c] = 500.0;
            s.orogeny_age[c] = 500.0;
        }
        s.thickness[100] = 68.0; // a Tibetan wall cell
        s.init_stats();
        let before: f64 = s.thickness.iter().map(|&t| t as f64).sum();
        s.apply_spreading();
        let after: f64 = s.thickness.iter().map(|&t| t as f64).sum();
        assert!(
            (before - after).abs() < 1e-3,
            "spreading must conserve thickness: {before} -> {after}"
        );
        assert!(s.thickness[100] < 68.0, "the wall must spread");
        let moved: f32 = s
            .grid
            .neighbors_of(100)
            .iter()
            .map(|&nb| s.thickness[nb as usize] - 35.0)
            .sum();
        assert!(moved > 0.0, "a neighbor must have received the excess");
    }

    /// WO-0008 S2: an ocean-ocean convergence band produces discrete
    /// islands, not a wall — land fraction of the arc band under 30% at
    /// 50 My, and no two converted cells adjacent (the overriding plate
    /// is stationary here, so no advection smearing muddies the check).
    #[test]
    fn ocean_ocean_band_produces_islands_not_a_wall() {
        let grid = Arc::new(Grid::build(3));
        let n = grid.cell_count() as usize;
        let mut s = SimState::new_empty(&grid);
        s.plates.push(test_plate(0, 0.0));
        // Fast enough to commit every couple of steps at L3, so the arc
        // band grows at its real cadence; an attached slab sustains the
        // convergence against the relaxation.
        s.plates.push(test_plate(1, 2.0));
        s.plates[1].pole = [0.0, 1.0, 0.0];
        s.plates[1].slab.push(SlabSegment {
            area_cells: 800,
            age_at_subduction_my: 150.0,
            subducted_at_my: 495.0,
            attached: true,
        });
        for c in 0..n {
            let p1 = grid.positions[c][0] > 0.0;
            s.plate_id[c] = u32::from(p1);
            s.crust_type[c] = 0;
            s.thickness[c] = OCEAN_THICKNESS_KM;
            // Plate 1 is older (denser): it subducts under plate 0.
            s.crust_age[c] = if p1 { 150.0 } else { 40.0 };
        }
        s.t_my = 500.0;
        s.init_stats();
        for i in 0..25 {
            s.step(0, i); // 50 My
        }
        // The band: plate-0 crust the arc actually grew (ocean above the
        // ridge-floor thickness, or converted young continent) — robust
        // to whether the final step happened to be a commit step.
        let band: Vec<usize> = (0..n)
            .filter(|&c| {
                s.plate_id[c] == 0
                    && ((s.crust_type[c] == 0 && s.thickness[c] > OCEAN_THICKNESS_KM + 1.0)
                        || (s.crust_type[c] == 1 && s.crust_age[c] < 60.0))
            })
            .collect();
        assert!(
            band.len() >= 8,
            "the trench must have grown an arc band ({} cells)",
            band.len()
        );
        let land: Vec<usize> = band
            .iter()
            .copied()
            .filter(|&c| s.crust_type[c] == 1)
            .collect();
        assert!(
            !land.is_empty(),
            "islands must have emerged by 50 My (sites convert in ~22 My)"
        );
        let frac = land.len() as f32 / band.len() as f32;
        assert!(
            frac < 0.3,
            "band land fraction {frac} must stay under 30% at 50 My"
        );
        for &c in &land {
            let wall = s
                .grid
                .neighbors_of(c as u32)
                .iter()
                .any(|&nb| land.contains(&(nb as usize)));
            assert!(!wall, "adjacent converted cells at {c}: a wall, not islands");
        }
    }

    /// Rift linkage (WO-0008 S1): two rift systems on one plate whose
    /// active tips sit within `RIFT_LINK_CELLS` merge into one system,
    /// claiming the least-strength connecting path.
    #[test]
    fn converging_rift_tips_link_and_merge() {
        let grid = Arc::new(Grid::build(3));
        let n = grid.cell_count() as usize;
        let mut s = SimState::new_empty(&grid);
        s.plates.push(test_plate(0, 0.0));
        // One weak young continental plate: every cell walkable by a
        // plume-strength rift (strength << STRESS_PLUME).
        for c in 0..n {
            s.plate_id[c] = 0;
            s.crust_type[c] = 1;
            s.thickness[c] = 32.0;
            s.crust_age[c] = 40.0;
            s.orogeny_age[c] = 40.0;
        }
        s.t_my = 100.0;
        s.init_stats();
        // Two fresh single-cell rifts two hops apart.
        let tip_1 = 100u32;
        let mid = s.grid.neighbors_of(tip_1)[0];
        let tip_2 = *s
            .grid
            .neighbors_of(mid)
            .iter()
            .find(|&&nb| nb != tip_1 && !s.grid.neighbors_of(tip_1).contains(&nb))
            .unwrap();
        for &tip in &[tip_1, tip_2] {
            s.rift_age[tip as usize] = RIFT_ONSET_MY + DT_MY;
            s.rifts.push(ActiveRift {
                plate: 0,
                kind: if tip == tip_1 {
                    RiftDriverKind::Plume
                } else {
                    RiftDriverKind::BackArc
                },
                stress: if tip == tip_1 {
                    STRESS_PLUME
                } else {
                    STRESS_BACKARC
                },
                tip_a: tip,
                tip_b: tip,
                done_a: false,
                done_b: false,
                started_my: if tip == tip_1 { 90.0 } else { 95.0 },
                cells: vec![tip],
            });
        }

        s.link_rifts();

        assert_eq!(s.rifts.len(), 1, "the two systems must merge");
        let r = &s.rifts[0];
        // The merged system keeps the two FAR tips (both rifts were
        // single-cell, so those are the original nucleation cells)...
        let tips = [r.tip_a, r.tip_b];
        assert!(tips.contains(&tip_1) && tips.contains(&tip_2), "{tips:?}");
        // ...the stronger driver's stress, and the earlier start.
        assert_eq!(r.stress, STRESS_PLUME);
        assert_eq!(r.started_my, 90.0);
        assert_eq!(s.rift_link_count, 1);
        // The connecting cell was claimed like a tip-walk cell.
        assert!(
            s.rift_age[mid as usize] > RIFT_ONSET_MY,
            "the least-strength path between the tips must be claimed"
        );
        assert!(s.features[mid as usize] & F_RIFT != 0);
    }

    /// The WO-0008 S1 seam rule on a two-plate synthetic world: a moving
    /// cap sweeping over a stationary plate for 60 My leaves the backstop
    /// idle — ownership resolves connectedly inside advect itself.
    #[test]
    fn seam_rule_keeps_backstop_idle_on_two_plate_world() {
        let grid = Arc::new(Grid::build(4));
        let n = grid.cell_count() as usize;
        let mut s = SimState::new_empty(&grid);
        s.plates.push(test_plate(0, 0.0));
        s.plates.push(test_plate(1, 0.5)); // fast enough to commit often
        s.plates[1].pole = [0.0, 0.0, 1.0];
        for c in 0..n {
            // Plate 1: a cap around +x; plate 0 the rest. All ocean, with
            // an age contrast so the pair polarity is deterministic.
            let cap = grid.positions[c][0] > 0.6;
            s.plate_id[c] = u32::from(cap);
            s.crust_type[c] = 0;
            s.thickness[c] = OCEAN_THICKNESS_KM;
            s.crust_age[c] = if cap { 40.0 } else { 120.0 };
        }
        s.t_my = 200.0;
        s.init_stats();
        for _ in 0..30 {
            s.step(0, 0);
        }
        assert_eq!(
            s.connectivity_reassigned, 0,
            "the seam rule must keep the backstop idle"
        );
        // Both plates one connected region each (nothing severed).
        for pid in 0..2u32 {
            if !s.plates[pid as usize].alive {
                continue; // fully consumed is fine; severed is not
            }
            let census = s.plate_id.iter().filter(|&&p| p == pid).count();
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
            assert_eq!(count, census, "plate {pid} must stay one component");
        }
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
            cells: vec![0],
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
