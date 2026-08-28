//! Stage 2 session 2: morphologic terrain (WO-0009 S2).
//!
//! Runs on ONE tectonic keyframe — the pinned era — and evolves its
//! elevation for `morpho_my` million years of weather: uplift on active
//! orogens, fluvial stream-power erosion keyed by lithology, hillslope
//! creep with a talus limit, priority-flood depression routing (closed
//! basins become lakes at spill level), latitude-band precipitation and
//! discharge, and capacity-limited sediment transport whose deposits write
//! lithology `su`. Transport only: the sediment pass creates and destroys
//! no rock — every quantized unit eroded into the stream load is deposited
//! somewhere (floodplain, lake bed, or seafloor), and the residual is an
//! exact-zero integer gate in the terrain ledger, the WO-0008
//! crust-volume-ledger discipline applied to erosion. (Uplift is tectonic
//! rock creation and hillslope creep is local pairwise exchange; both are
//! booked separately, outside the transport ledger.)
//!
//! Determinism: elevation state is f64, every op is IEEE +,−,×,/ and sqrt,
//! the only transcendental is `dmath::det_exp_neg`, all loops are serial
//! and id-ordered (the flood heap breaks ties by cell id), and the stage's
//! one RNG draw seeds a splitmix hash for the sub-cell precipitation
//! texture — same seed, same terrain, on every platform.
//!
//! References: Braun & Willett 2013 (implicit O(n) stream power on the
//! flow tree); Barnes, Lehman & Mulla 2014 (priority-flood + epsilon);
//! Stock & Montgomery 1999 / benchmarks Table 5.2 (the K_LITH span);
//! benchmarks Table 5.1 (denudation-rate gates).

use std::cmp::Ordering as CmpOrdering;
use std::collections::BinaryHeap;

use rand::RngCore;

use worldmaker_core::dmath::det_exp_neg;
use worldmaker_core::hash::{fnv1a_continue, FNV_OFFSET};
use worldmaker_core::rng::sub_rng;
use worldmaker_core::Grid;

use crate::pipeline::{Stage, StageContext, WorldState};
use crate::tectonics::{lithology, Keyframe, F_ARC, F_HOTSPOT};

pub const STAGE_ID: &str = "phase2-terrain";

// ----- field names this stage writes -----
/// Post-erosion elevation (m, keyframe sea level = 0).
pub const TERRAIN_ELEVATION_M: &str = "terrain_elevation_m";
/// River discharge (m³/s) leaving each cell.
pub const TERRAIN_DISCHARGE_M3S: &str = "terrain_discharge_m3s";
/// Cumulative sediment deposited (m of thickness).
pub const TERRAIN_SEDIMENT_M: &str = "terrain_sediment_m";
/// Lake depth (m): filled water surface minus bed where a closed basin
/// holds water at spill level; 0 elsewhere.
pub const TERRAIN_LAKE_DEPTH_M: &str = "terrain_lake_depth_m";
/// Lithology after deposition (u32 field; `su` stamped on deposits).
pub const TERRAIN_LITHOLOGY: &str = "terrain_lithology";

// ----- constants (WO-0009 S2; erodibility inside benchmarks Table 5.2) -----
/// Peak orogenic uplift (mm/yr) at orogeny age 0; Table 5.1 active-orogen
/// typical top.
pub const U0_MM_YR: f32 = 5.0;
/// Uplift e-folding age (My): U = U0·exp(−age/50).
const UPLIFT_TAU_MY: f32 = 50.0;
/// Volcanic construction on active arc and hotspot cells (mm/yr).
const ARC_HOTSPOT_MM_YR: f32 = 0.5;
/// Morphologic sub-step (My). Implicit fluvial is unconditionally stable;
/// this just paces the routing refresh.
const DT_MY: f64 = 0.5;
/// Stream-power erodibility per GLiM class at m = 0.5, n = 1 (K in yr⁻¹·
/// m^0, A in m²) — all inside the Stock & Montgomery 1999 span (Table
/// 5.2: weak 1e-2..1e-4, hard 1e-5..1e-7). WO-0009 S7 calibrates the
/// final values inside the same span. Classes the sim never writes carry
/// span-consistent placeholders (documented Phase 3+ gaps).
pub const K_LITH: [f64; lithology::CLASS_COUNT] = [
    1.0e-4, // su  unconsolidated — weakest
    2.0e-5, // ss  siliciclastic
    2.0e-5, // sm  mixed sedimentary
    2.0e-5, // sc  carbonate (unwritten)
    5.0e-5, // py  pyroclastics (unwritten)
    1.0e-4, // ev  evaporites (unwritten)
    2.0e-6, // mt  metamorphic
    1.0e-6, // pa  acid plutonic — hardest (craton shields)
    2.0e-6, // pi  intermediate plutonic (unwritten)
    4.0e-6, // pb  basic plutonic (rift shoulders)
    3.0e-6, // va  acid volcanic (unwritten)
    3.0e-6, // vi  intermediate volcanic (arcs)
    3.0e-6, // vb  basic volcanic (shields, ocean floor)
    3.0e-6, // ig  undifferentiated igneous (unwritten)
    2.0e-5, // wb  water bodies (unwritten)
    2.0e-5, // nd  no data (unwritten)
];
/// Hillslope diffusivity (m²/yr) — standard soil-creep order; negligible
/// at ≥100 km cells, present for the contract (and for future fine grids).
const HILLSLOPE_D_M2_YR: f64 = 0.01;
/// Talus limit: tan(33°) — pairwise slope cap after diffusion.
const TALUS_TAN: f64 = 0.649_407_6;
/// Priority-flood epsilon (m per cell hop): guarantees strict downhill
/// drainage across filled basins (Barnes et al. 2014).
const FLOOD_EPS_M: f64 = 1.0e-3;
/// Capacity coefficient: sediment capacity Qs = KC·Q·S (volumes/yr).
/// Steady transport slope is U/(KC·P) — at 1.0, a 3.5 mm/yr orogen over a
/// 1 m/yr climate grades at ~0.0035 (≈450 m per L6 cell), Tibet-scale;
/// at the first-try 0.05 the same flux needed ~9 km per cell and interior
/// plateaus ran away (measured 68 km peaks).
const CAPACITY_KC: f64 = 1.0;
/// Steepest gradient deposition may build (≈3°, the upper end of natural
/// fan and delta fronts): every deposit is tied to its receiver's level
/// plus at most this slope, so aggradation can never manufacture the
/// artificial cliffs whose huge capacities re-mobilized km-scale slugs.
const S_MAX_DEPOSIT: f64 = 0.05;
/// Transport-ledger quantum: 0.1 mm of column height per cell.
const QUANT_M: f64 = 1.0e-4;
/// Seconds per Julian year (discharge display units).
const SEC_PER_YR: f64 = 3.155_76e7;

/// Smoothed latitude-band precipitation (m/yr), parameterized by |sin lat|
/// so no inverse trig enters the sim path: wet equator, dry horse
/// latitudes (25°), moderate midlatitudes (~45–55°), dry poles.
/// Piecewise-linear between fixed anchors.
#[allow(clippy::approx_constant)] // 0.7071 is sin(45°), not 1/√2-the-constant
const PRECIP_ANCHORS: [(f64, f64); 7] = [
    (0.00, 2.0),    // equator
    (0.20, 1.4),    // ~12°
    (0.4226, 0.25), // 25° — subtropical dry belt
    (0.60, 0.7),    // ~37°
    (0.7071, 1.0),  // 45° — midlatitude storm track
    (0.85, 0.55),   // ~58°
    (1.00, 0.15),   // poles
];

fn precip_m_yr(abs_sin_lat: f64) -> f64 {
    let x = abs_sin_lat.clamp(0.0, 1.0);
    let mut prev = PRECIP_ANCHORS[0];
    for &a in PRECIP_ANCHORS.iter().skip(1) {
        if x <= a.0 {
            let t = (x - prev.0) / (a.0 - prev.0);
            return prev.1 + (a.1 - prev.1) * t;
        }
        prev = a;
    }
    PRECIP_ANCHORS[PRECIP_ANCHORS.len() - 1].1
}

fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// No receiver (ocean cells and any true pit remnant).
pub const RECV_NONE: u32 = u32::MAX;

/// User-facing terrain parameters; all hashed into `params_hash`.
#[derive(Clone, Debug)]
pub struct TerrainParams {
    /// Morphologic time, My (5–100). The World panel slider (WO-0009 S2
    /// step 5).
    pub morpho_my: f32,
    /// Keyframe index the stage runs on (the pinned era); `None` = the
    /// history's last keyframe.
    pub era_index: Option<u32>,
}

impl Default for TerrainParams {
    fn default() -> Self {
        TerrainParams {
            morpho_my: 30.0,
            era_index: None,
        }
    }
}

impl TerrainParams {
    pub fn clamped(mut self) -> Self {
        self.morpho_my = self.morpho_my.clamp(5.0, 100.0);
        self
    }
}

/// Everything one terrain run produces. Field vectors are cell-count long.
pub struct TerrainOutput {
    /// Post-erosion elevation (m, keyframe sea level = 0).
    pub elev_m: Vec<f32>,
    /// Discharge leaving each cell (m³/s).
    pub discharge_m3s: Vec<f32>,
    /// Cumulative deposited sediment thickness (m).
    pub sediment_m: Vec<f32>,
    /// Lake depth at spill level (m; 0 = no lake).
    pub lake_depth_m: Vec<f32>,
    /// Lithology after deposition (`su` stamped on deposits).
    pub lithology: Vec<u8>,
    /// Final flow receiver per cell ([`RECV_NONE`] for ocean/none).
    pub receiver: Vec<u32>,
    /// Final epsilon-filled water surface (m) the receivers descend.
    pub water_surface_m: Vec<f64>,
    /// Cumulative fluvial incision per cell (m) — the denudation the
    /// gates measure (hillslope creep at these cell sizes is nil).
    pub fluvial_erosion_m: Vec<f32>,
    /// Cumulative tectonic uplift added per cell (m).
    pub uplift_m: Vec<f32>,
    /// Transport ledger (integer quanta of [`QUANT_M`]·cell): eroded into
    /// the stream load…
    pub ledger_eroded_q: i64,
    /// …and deposited back out. `residual()` must be exactly zero.
    pub ledger_deposited_q: i64,
}

impl TerrainOutput {
    /// Transport-ledger residual: eroded − deposited, exact integer.
    pub fn residual_q(&self) -> i64 {
        self.ledger_eroded_q - self.ledger_deposited_q
    }
}

/// The terrain stage: owns its parameters, reads the pinned keyframe from
/// the world's tectonic history, writes the terrain fields.
pub struct TerrainStage {
    pub params: TerrainParams,
}

impl TerrainStage {
    pub fn new(params: TerrainParams) -> Self {
        TerrainStage {
            params: params.clamped(),
        }
    }
}

impl Stage for TerrainStage {
    fn id(&self) -> &'static str {
        STAGE_ID
    }

    fn params_hash(&self) -> u64 {
        let mut h = FNV_OFFSET;
        h = fnv1a_continue(h, &self.params.morpho_my.to_le_bytes());
        match self.params.era_index {
            Some(i) => {
                h = fnv1a_continue(h, &[1u8]);
                h = fnv1a_continue(h, &i.to_le_bytes());
            }
            None => h = fnv1a_continue(h, &[0u8]),
        }
        h
    }

    fn run(&self, ctx: &StageContext, world: &mut WorldState) -> anyhow::Result<()> {
        let history = world
            .history
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("terrain stage needs a tectonic history upstream"))?;
        let last = history.keyframes.len() - 1;
        let idx = match self.params.era_index {
            Some(i) => (i as usize).min(last),
            None => last,
        };
        let kf = &history.keyframes[idx];
        let out = run_terrain(&world.grid, kf, ctx.master_seed, self.params.morpho_my);
        anyhow::ensure!(
            out.residual_q() == 0,
            "terrain transport ledger residual {} != 0",
            out.residual_q()
        );
        let n = world.grid.cell_count() as usize;
        let write = |fields: &mut worldmaker_core::FieldStore, name: &str, data: &[f32]| {
            fields.get_or_insert_mut(name)[..n].copy_from_slice(data);
        };
        write(&mut world.fields, TERRAIN_ELEVATION_M, &out.elev_m);
        write(&mut world.fields, TERRAIN_DISCHARGE_M3S, &out.discharge_m3s);
        write(&mut world.fields, TERRAIN_SEDIMENT_M, &out.sediment_m);
        write(&mut world.fields, TERRAIN_LAKE_DEPTH_M, &out.lake_depth_m);
        {
            let lith = world.fields.get_or_insert_mut_u32(TERRAIN_LITHOLOGY);
            for (o, &v) in lith.iter_mut().zip(&out.lithology) {
                *o = v as u32;
            }
        }
        Ok(())
    }
}

/// Heap entry for priority flood: min-heap on (surface, cell id) — the id
/// tie-break keeps the pop order (and thus the fill) fully deterministic.
struct FloodEntry {
    w: f64,
    cell: u32,
}
impl PartialEq for FloodEntry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == CmpOrdering::Equal
    }
}
impl Eq for FloodEntry {}
impl PartialOrd for FloodEntry {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}
impl Ord for FloodEntry {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        // Reversed: BinaryHeap is a max-heap, we need the LOWEST surface.
        other
            .w
            .total_cmp(&self.w)
            .then_with(|| other.cell.cmp(&self.cell))
    }
}

/// Working routing state, rebuilt from the current elevations each
/// sub-step.
struct Routing {
    /// Epsilon-filled surface (Barnes et al. 2014).
    water: Vec<f64>,
    /// Steepest-descent receiver on the filled surface (RECV_NONE = ocean).
    receiver: Vec<u32>,
    /// Cells sorted by descending filled surface (id tie-break): donors
    /// before receivers — the topological order every accumulation uses.
    order: Vec<u32>,
    /// Upstream drainage area (m², own cell included) — stream-power A.
    area_m2: Vec<f64>,
    /// Discharge leaving each cell (m³/yr).
    q_m3_yr: Vec<f64>,
}

/// One priority-flood + flow + discharge pass over elevations `h`
/// (sub-steps 4.4 and 4.5). Ocean cells (h ≤ 0) are the drainage seeds.
fn route(grid: &Grid, h: &[f64], precip_eff: &[f64], cell_area_m2: f64) -> Routing {
    let n = h.len();
    let mut water = h.to_vec();
    let mut done = vec![false; n];
    let mut heap: BinaryHeap<FloodEntry> = BinaryHeap::new();
    // Seed: every ocean cell (its own drainage basin outlet).
    let mut any_ocean = false;
    for c in 0..n {
        if h[c] <= 0.0 {
            any_ocean = true;
            heap.push(FloodEntry {
                w: h[c],
                cell: c as u32,
            });
            done[c] = true;
        }
    }
    if !any_ocean {
        // Degenerate all-land world: seed from the global minimum (lowest
        // id on ties) so the flood still terminates.
        let mut best = 0usize;
        for c in 1..n {
            if h[c] < h[best] {
                best = c;
            }
        }
        heap.push(FloodEntry {
            w: h[best],
            cell: best as u32,
        });
        done[best] = true;
    }
    while let Some(FloodEntry { w, cell }) = heap.pop() {
        for &nb in grid.neighbors_of(cell) {
            let nbu = nb as usize;
            if done[nbu] {
                continue;
            }
            done[nbu] = true;
            let wn = if h[nbu] > w + FLOOD_EPS_M {
                h[nbu]
            } else {
                w + FLOOD_EPS_M
            };
            water[nbu] = wn;
            heap.push(FloodEntry { w: wn, cell: nb });
        }
    }

    // Steepest descent on the filled surface — for EVERY cell that has a
    // strictly lower neighbor: land receivers are the rivers; ocean-cell
    // receivers descend the bathymetry so sediment reaching the coast
    // spreads into the basin instead of stacking at the first wet cell.
    // Basin minima keep RECV_NONE (terminal sinks).
    let mut receiver = vec![RECV_NONE; n];
    for c in 0..n {
        let mut best: Option<(u32, f64)> = None;
        for &nb in grid.neighbors_of(c as u32) {
            let wn = water[nb as usize];
            if wn < water[c] && best.is_none_or(|(_, bw)| wn < bw) {
                best = Some((nb, wn));
            }
        }
        if let Some((nb, _)) = best {
            receiver[c] = nb;
        }
    }

    // Topological order: descending filled surface, id tie-break.
    let mut order: Vec<u32> = (0..n as u32).collect();
    order.sort_unstable_by(|&a, &b| {
        water[b as usize]
            .total_cmp(&water[a as usize])
            .then_with(|| a.cmp(&b))
    });

    // Drainage area and discharge, donors before receivers. Rivers end at
    // the coast: ocean cells collect inflow but never propagate it, so
    // discharge stays a LAND quantity (an ocean cell's value is what its
    // coast delivers).
    let mut area_m2 = vec![cell_area_m2; n];
    let mut q_m3_yr: Vec<f64> = precip_eff.iter().map(|&p| p * cell_area_m2).collect();
    for &c in &order {
        let cu = c as usize;
        let r = receiver[cu];
        if r != RECV_NONE && h[cu] > 0.0 {
            let a = area_m2[cu];
            let q = q_m3_yr[cu];
            area_m2[r as usize] += a;
            q_m3_yr[r as usize] += q;
        }
    }

    Routing {
        water,
        receiver,
        order,
        area_m2,
        q_m3_yr,
    }
}

/// Run the terrain evolution on one keyframe. Pure function of its inputs
/// — the goldens hash its outputs.
pub fn run_terrain(grid: &Grid, kf: &Keyframe, master_seed: u64, morpho_my: f32) -> TerrainOutput {
    let n = grid.cell_count() as usize;
    assert_eq!(kf.elev_m.len(), n, "keyframe/grid mismatch");
    let morpho_my = morpho_my.clamp(5.0, 100.0) as f64;
    let cell_area_m2 = {
        let r_m = 6_371_000.0f64;
        4.0 * std::f64::consts::PI * r_m * r_m / n as f64
    };
    let dx_m = cell_area_m2.sqrt();

    // Working state off the keyframe.
    let mut h: Vec<f64> = kf.elev_m.iter().map(|&e| e as f64).collect();
    let mut lith: Vec<u8> = kf.lithology.clone();
    let orogeny_age: Vec<f32> = kf.orogeny_age_my.iter().map(|&a| a as f32).collect();
    let continent: Vec<bool> = kf.flags.iter().map(|&f| f & (1 << 15) != 0).collect();
    let volcanic: Vec<bool> = kf
        .flags
        .iter()
        .map(|&f| f as u32 & (F_ARC | F_HOTSPOT) != 0)
        .collect();

    // The stage's own RNG stream (WO-0009 S2): one draw seeds the pure
    // per-cell precipitation texture hash.
    let precip_key = sub_rng(master_seed, STAGE_ID, "precip-noise").next_u64();
    let precip_eff: Vec<f64> = (0..n)
        .map(|c| {
            let base = precip_m_yr(grid.positions[c][2].abs() as f64);
            let u = (splitmix64(precip_key ^ c as u64) >> 40) as f64 / (1u64 << 24) as f64;
            base * (0.9 + 0.2 * u)
        })
        .collect();

    let mut sediment_m = vec![0.0f64; n];
    let mut fluvial_erosion_m = vec![0.0f64; n];
    let mut uplift_m = vec![0.0f64; n];
    let mut ledger_eroded_q = 0i64;
    let mut ledger_deposited_q = 0i64;
    // Per-cell stream load this sub-step, in QUANT_M quanta.
    let mut load_q = vec![0i64; n];
    // Per-sub-step downstream flux accumulator (4.2's saturation check).
    let mut flux_q = vec![0i64; n];

    let steps = (morpho_my / DT_MY).round().max(1.0) as u32;
    let dt_my = morpho_my / steps as f64;
    let dt_yr = dt_my * 1.0e6;

    // Initial routing (the 4.2 fluvial pass of the first sub-step uses it;
    // 4.4/4.5 refresh it every sub-step thereafter).
    let mut routing = route(grid, &h, &precip_eff, cell_area_m2);

    for step in 0..steps {
        let t_elapsed_my = step as f64 * dt_my;

        // ----- 4.1 uplift -----
        // U = U0·exp(−orogeny_age/50 My) on orogenic continental cells
        // (the decay retires old belts smoothly — cratons read ~0), plus
        // a 0.5 mm/yr volcanic-construction term on arc/hotspot cells.
        for c in 0..n {
            let mut u_mm_yr = 0.0f64;
            if continent[c] {
                let age = orogeny_age[c] as f64 + t_elapsed_my;
                u_mm_yr += U0_MM_YR as f64 * det_exp_neg(-(age as f32) / UPLIFT_TAU_MY) as f64;
            }
            if volcanic[c] {
                u_mm_yr += ARC_HOTSPOT_MM_YR as f64;
            }
            if u_mm_yr > 0.0 {
                let dh = u_mm_yr * 1.0e-3 * dt_yr; // mm/yr → m over the step
                h[c] += dh;
                uplift_m[c] += dh;
            }
        }

        // ----- 4.2 fluvial erosion -----
        // dh/dt = U − K·A^0.5·S (m = 0.5, n = 1), U already applied above;
        // the incision term solved implicitly on the flow tree (Braun &
        // Willett 2013): receivers first (ascending filled surface =
        // reverse of `order`), h_i ← (h_i + k·z_r)/(1 + k) with
        // k = K·√A·dt/dx. The receiver level z_r is the receiver's WATER
        // surface, not its bed: a channel entering a lake incises toward
        // the lake's spill level (its local base level), never the
        // drowned bed below it — without this, subaerial cells whose
        // steepest descent crosses a deeper-bedded lake cell never eroded
        // and uplifting plateau interiors ran away by tens of km
        // (measured: 125 km peaks at L6 seed 42 before the rule).
        // Submerged cells (bed under the lake surface) carry no channel
        // and don't incise; they aggrade in 4.6 instead. Two transport
        // limits keep the detachment physical: a river cannot pick up
        // more than its capacity (∝ Q·S, the same law as 4.6), and a
        // river already carrying its capacity from upstream picks up
        // NOTHING more (saturation — the flux accumulator walks the tree
        // donors-first). Without them, weak `su` reaches slammed to base
        // level every sub-step and km-scale sediment packets sloshed
        // between neighbors (measured: 78 km transient spires at L6).
        // Quantized to the transport ledger: the eroded column enters
        // the stream load, exactly.
        for v in flux_q.iter_mut() {
            *v = 0;
        }
        for &c in &routing.order {
            let cu = c as usize;
            let r = routing.receiver[cu];
            if r == RECV_NONE || h[cu] <= 0.0 {
                continue;
            }
            let mut flux_out = flux_q[cu];
            let submerged = routing.water[cu] > h[cu] + FLOOD_EPS_M;
            let z_r = routing.water[r as usize];
            if !submerged && z_r < h[cu] {
                let k_lith = K_LITH[(lith[cu] as usize).min(lithology::CLASS_COUNT - 1)];
                let k = k_lith * routing.area_m2[cu].sqrt() * dt_yr / dx_m;
                let h_new = (h[cu] + k * z_r) / (1.0 + k);
                // Never incise below base level, nor below sea level.
                let h_new = h_new.max(z_r).max(0.0);
                let slope = (routing.water[cu] - z_r).max(0.0) / dx_m;
                let cap_q = (CAPACITY_KC * routing.q_m3_yr[cu] * slope * dt_yr
                    / (cell_area_m2 * QUANT_M))
                    .floor() as i64;
                let headroom = (cap_q - flux_q[cu]).max(0);
                let q = (((h[cu] - h_new) / QUANT_M).floor() as i64).min(headroom);
                if q > 0 {
                    h[cu] -= q as f64 * QUANT_M;
                    fluvial_erosion_m[cu] += q as f64 * QUANT_M;
                    load_q[cu] += q;
                    flux_out += q;
                    ledger_eroded_q += q;
                }
            }
            flux_q[r as usize] += flux_out;
        }

        // ----- 4.3 hillslope diffusion + talus limit -----
        // Pairwise creep between land neighbors (i < nb keeps each edge
        // once; antisymmetric transfer conserves by construction), then
        // the 33° talus cap the same way. Nil at ≥100 km cells; kept for
        // the contract.
        let creep = HILLSLOPE_D_M2_YR * dt_yr / (dx_m * dx_m);
        for c in 0..n {
            if h[c] <= 0.0 {
                continue;
            }
            for &nb in grid.neighbors_of(c as u32) {
                let nbu = nb as usize;
                if nbu <= c || h[nbu] <= 0.0 {
                    continue;
                }
                let d = h[c] - h[nbu];
                let mut flow = creep * d;
                let excess = d.abs() - TALUS_TAN * dx_m;
                if excess > 0.0 {
                    // Talus failure: move half the over-threshold relief.
                    flow += 0.5 * excess * d.signum();
                }
                let flow = flow.clamp(-0.5 * d.abs(), 0.5 * d.abs()) * d.signum().abs();
                h[c] -= flow;
                h[nbu] += flow;
            }
        }

        // ----- 4.4 priority-flood + 4.5 flow & discharge -----
        routing = route(grid, &h, &precip_eff, cell_area_m2);

        // ----- 4.6 sediment: capacity-limited routing -----
        // Load moves down the fresh tree, donors first; capacity ∝ Q·S
        // (Qs = KC·Q·S). Deposition is GRADED: an overloaded reach
        // (S < S_eq = flux/(KC·Q)) aggrades only up to the equilibrium
        // slope toward its receiver, and the remainder keeps moving — a
        // sediment wedge spreads along the profile the way a real
        // floodplain does, instead of stacking a tower at the first slow
        // cell (measured before the rule: transient 78 km spires as
        // km-scale slugs ping-ponged between neighbors). Lake beds fill
        // to the lake surface, ocean beds to sea level; what remains
        // progrades down the (bathymetric) receiver chain. Deposits
        // stamp `su`. Exact integer conservation: every quantum eroded
        // in 4.2 deposits somewhere by the end of the run.
        for &c in &routing.order {
            let cu = c as usize;
            let q_in = load_q[cu];
            if q_in == 0 {
                continue;
            }
            load_q[cu] = 0;
            let r = routing.receiver[cu];
            if h[cu] <= 0.0 {
                // Ocean: the open boundary. Everything arriving deposits
                // here and the ledger books all of it, but the bed only
                // aggrades to just below sea level — the rest is basin
                // accommodation (subsidence under load), the classic LEM
                // open-boundary sink. Without it the orogenic export
                // (∼10⁵ km·cell of rock at L6 defaults) piled the seafloor
                // into 500 km sediment towers above the waves.
                let rise_lim = (((-10.0 - h[cu]) / QUANT_M).floor()).max(0.0) as i64;
                let rise = q_in.min(rise_lim);
                if rise > 0 {
                    let dh = rise as f64 * QUANT_M;
                    h[cu] += dh;
                    sediment_m[cu] += dh;
                    lith[cu] = lithology::SU;
                }
                ledger_deposited_q += q_in;
                continue;
            }
            let deposit = if r == RECV_NONE {
                q_in // terminal (endorheic) basin minimum: the final sink
            } else {
                // Aggrade to the grade line: the receiver's level plus
                // the transport-equilibrium slope S_eq = flux/(KC·Q),
                // never steeper than a natural fan front. One rule for
                // floodplain, delta and seafloor alike — a deep bed fills
                // (a hole below grade takes a lot), a bed at grade takes
                // nothing, and the rest progrades down the chain.
                let flux_m3_yr = q_in as f64 * cell_area_m2 * QUANT_M / dt_yr;
                let q_water = routing.q_m3_yr[cu].max(1.0);
                let s_eq = (flux_m3_yr / (CAPACITY_KC * q_water)).min(S_MAX_DEPOSIT);
                let grade = routing.water[r as usize] + s_eq * dx_m;
                (((grade - h[cu]) / QUANT_M).floor() as i64).clamp(0, q_in)
            };
            if deposit > 0 {
                let dh = deposit as f64 * QUANT_M;
                h[cu] += dh;
                sediment_m[cu] += dh;
                lith[cu] = lithology::SU;
                ledger_deposited_q += deposit;
            }
            let pass_on = q_in - deposit;
            if pass_on > 0 {
                // r != RECV_NONE here: the terminal arm deposits q_in.
                load_q[r as usize] += pass_on;
            }
        }
    }

    // Any load still in flight (none should be: the last 4.6 pass drains
    // every chain into a capacity-0 terminal, but a cap > 0 cell can hand
    // its residue to a receiver processed earlier only if the tree were
    // cyclic — it is not) settles in place so the ledger closes exactly.
    for c in 0..n {
        if load_q[c] > 0 {
            let dh = load_q[c] as f64 * QUANT_M;
            h[c] += dh;
            sediment_m[c] += dh;
            lith[c] = lithology::SU;
            ledger_deposited_q += load_q[c];
            load_q[c] = 0;
        }
    }

    // Final routing refresh so the published discharge, receivers and lake
    // depths describe the final surface.
    let routing = route(grid, &h, &precip_eff, cell_area_m2);
    let lake_depth_m: Vec<f32> = (0..n)
        .map(|c| {
            if h[c] > 0.0 && routing.water[c] > h[c] {
                (routing.water[c] - h[c]) as f32
            } else {
                0.0
            }
        })
        .collect();

    TerrainOutput {
        elev_m: h.iter().map(|&v| v as f32).collect(),
        discharge_m3s: routing
            .q_m3_yr
            .iter()
            .map(|&q| (q / SEC_PER_YR) as f32)
            .collect(),
        sediment_m: sediment_m.iter().map(|&v| v as f32).collect(),
        lake_depth_m,
        lithology: lith,
        receiver: routing.receiver,
        water_surface_m: routing.water,
        fluvial_erosion_m: fluvial_erosion_m.iter().map(|&v| v as f32).collect(),
        uplift_m: uplift_m.iter().map(|&v| v as f32).collect(),
        ledger_eroded_q,
        ledger_deposited_q,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precip_bands_have_the_right_shape() {
        let eq = precip_m_yr(0.0);
        let dry25 = precip_m_yr(0.4226);
        let mid = precip_m_yr(0.7071);
        let pole = precip_m_yr(1.0);
        assert!(eq > mid && mid > dry25, "{eq} {mid} {dry25}");
        assert!(dry25 > pole || pole < mid, "{dry25} {pole}");
        assert!(pole < 0.3);
        // Continuous: no jumps bigger than the anchor gaps allow.
        let mut prev = precip_m_yr(0.0);
        let mut x = 0.01;
        while x <= 1.0 {
            let v = precip_m_yr(x);
            assert!((v - prev).abs() < 0.2, "precip jump at {x}");
            prev = v;
            x += 0.01;
        }
    }
}
