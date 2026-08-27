//! Compact keyframe encoding for the tectonic history.
//!
//! A keyframe every 10 My holds the full simulation state — per-cell fields
//! packed to 16 bits each plus the per-plate and per-pair bookkeeping — so a
//! run can restart from any keyframe (plate drag now, branching later) and the
//! era picker can decode any moment for display. 22 B/cell (WO-0006: slab
//! ledger fields per Dan's ruling, plus the S2 suture scar) keeps a 2 Gy run
//! at L7 (201 keyframes × 163,842 cells) near 0.72 GB, inside the 1 GB
//! budget; S3 re-measures.

use worldmaker_core::FieldStore;

use super::{
    CRUST_AGE_MY, CRUST_TYPE, ELEVATION_M, FEATURES, HOTSPOT_BUILDUP_KM, OROGENY_AGE_MY, PLATE_ID,
    RIFT_AGE_MY,
};

/// Row-major 3×3 identity, the empty pending rotation.
pub const IDENTITY3: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// Per-plate state, snapshotted per keyframe and live during the run.
/// Everything here is needed for a bit-exact restart from a keyframe: the
/// pending sub-cell rotation and the previous step's boundary composition
/// both feed the next step.
#[derive(Clone, Debug)]
pub struct PlateState {
    pub id: u32,
    pub alive: bool,
    /// Euler pole, unit vector.
    pub pole: [f32; 3],
    /// Current angular speed, deg/My (always ≥ 0; the pole encodes direction).
    pub speed_deg_my: f32,
    /// Simulation time (My) of this plate's most recent suture, or
    /// [`NEVER_SUTURED`] if it has none.
    pub youngest_suture_my: f32,
    /// When this plate last nucleated a rift or was born of a split
    /// ([`NEVER_SUTURED`] = never): rifting relieves the stress a driver
    /// needs, so nucleation observes a refractory period from here
    /// (WO-0006 S2).
    pub youngest_rift_my: f32,
    /// Accumulated rotation not yet applied by advection (row-major). Slow
    /// plates bank sub-cell motion here and commit it once it reaches about
    /// one cell, so they never freeze to the grid.
    pub pending_rot: [[f32; 3]; 3],
    /// Total banked rotation angle, degrees.
    pub pending_deg: f32,
    /// Slab ledger: what this plate has subducted, one merged segment per
    /// consuming step, in chronological (fixed) order. Attached segments are
    /// the slab-pull drivers of the force balance; a dead plate's remaining
    /// segments transfer to the plate that consumed it (Dan's ruling:
    /// slabs keep pulling after the subducting plate dies).
    pub slab: Vec<SlabSegment>,
    /// Previous step's boundary composition (force-balance inputs).
    pub boundary_cells: u32,
    pub subducting_cells: u32,
    pub colliding_cells: u32,
    /// Strength-weighted continent-continent contact (Σ strength(cell) over
    /// colliding contact cells, WO-0006 S2): the R_bnd input — a contact
    /// with a craton resists harder than one with a fresh suture.
    pub colliding_strength: f32,
    /// Cells on a divergent boundary (ridge-push driver).
    pub ridge_cells: u32,
    /// Transform-only boundary cells (transform-friction resistance).
    pub transform_cells: u32,
    /// Summed boundary torque directions from the previous step (unnormalized;
    /// the pole relaxes toward its direction).
    pub drive_torque: [f32; 3],
}

/// One entry of a plate's slab ledger: crust consumed at its trenches in one
/// step, merged per plate per step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlabSegment {
    /// Cells consumed.
    pub area_cells: u32,
    /// Mean crust age when it went under (older = denser = stronger pull).
    pub age_at_subduction_my: f32,
    /// Simulation time of consumption.
    pub subducted_at_my: f32,
    /// Still mechanically coupled to the surface plate; detaches after
    /// `SLAB_DETACH_MY` and stops pulling.
    pub attached: bool,
}

/// Sentinel for "no suture yet" (per plate and per cell) — old enough that
/// every age-since-suture ramp (strength healing, mantle insulation) reads a
/// never-sutured plate as fully aged.
pub const NEVER_SUTURED: f32 = -1.0e9;

/// The three physical rift drivers of model §5. `u8` repr so the keyframe
/// stores it directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RiftDriverKind {
    /// A mantle plume that has sat under continental crust ≥ 20 My.
    Plume = 0,
    /// Back-arc extension inboard of a trench consuming old lithosphere.
    BackArc = 1,
    /// Slab pull on two roughly opposite sides puts the interior in tension.
    OpposingSlabs = 2,
}

/// One live rift: nucleated by a §5 driver, growing along the path of least
/// strength (amendment A: it advances only while driver stress exceeds local
/// strength). Two tips walk outward from the nucleation cell; a tip is done
/// when it reaches the plate boundary. A completed rift (both tips done)
/// stays in the ledger until its corridor oceanizes and splits the plate —
/// the split event is attributed to `kind`. Part of the keyframe: rift
/// growth is sim state, so resume must replay it bit-exactly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActiveRift {
    pub plate: u32,
    pub kind: RiftDriverKind,
    /// Driver stress, compared against `strength()` at every advance.
    pub stress: f32,
    pub tip_a: u32,
    pub tip_b: u32,
    pub done_a: bool,
    pub done_b: bool,
    /// Nucleation time (My): completed rifts are pruned from the ledger a
    /// long time after starting if their split never materializes.
    pub started_my: f32,
}

/// How a microplate came to exist (model §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicroplateOrigin {
    /// Plate area isolated against an active trench when the connecting
    /// crust was consumed (Farallon → Juan de Fuca remnant).
    TrenchTrapped,
    /// A back-arc rift oceanized and detached the arc sliver.
    BackArcBasin,
    /// A rift re-nucleated on the far side of a microcontinent and
    /// transferred it (Jan Mayen style).
    RidgeJump,
}

/// One entry of the run's event log (WO-0006 S2): every suture and split
/// carries the condition or driver that caused it. Diagnostics only — events
/// never feed back into the dynamics, so they are not keyframed.
#[derive(Clone, Debug, PartialEq)]
pub enum TectonicEvent {
    /// Plate `b` merged into `a` after the three §3 conditions held for
    /// `SUTURE_AFTER_MY`; the contact spanned `contact_fraction` of the
    /// smaller plate's perimeter when it fired.
    Suture {
        a: u32,
        b: u32,
        t: f32,
        contact_fraction: f32,
    },
    RiftStart {
        plate: u32,
        driver: RiftDriverKind,
        t: f32,
    },
    /// The rift stalled (stress no longer exceeded strength ahead of a tip);
    /// its cells keep maturing as a failed-rift scar.
    RiftFailed { plate: u32, t: f32 },
    /// A rift corridor oceanized across the plate and split it.
    Split {
        parent: u32,
        child: u32,
        driver: RiftDriverKind,
        t: f32,
    },
    Microplate {
        id: u32,
        origin: MicroplateOrigin,
        t: f32,
    },
}

/// Slow-collision timer for one unordered plate pair (a < b), for the suture
/// rule. Kept sorted by (a, b) — fixed iteration order.
#[derive(Clone, Debug)]
pub struct PairTimer {
    pub a: u32,
    pub b: u32,
    pub slow_collision_my: f32,
}

/// Bit 15 of the packed keyframe flags stores crust_type (1 = continent).
/// Bits 0..=7 are the `features` bits.
const KF_CONTINENT_BIT: u16 = 1 << 15;

/// One snapshot of the world at time `t_my`. All Vecs are cell-count long.
#[derive(Clone)]
pub struct Keyframe {
    pub t_my: f32,
    /// Sea-level offset (m) that was subtracted so 0 = solved sea level.
    pub sea_offset_m: f32,
    pub elev_m: Vec<i16>,
    pub plate_id: Vec<u16>,
    pub crust_age_my: Vec<u16>,
    /// Crust thickness in centi-km (km × 100).
    pub thickness_ckm: Vec<u16>,
    pub orogeny_age_my: Vec<u16>,
    pub rift_age_my: Vec<u16>,
    /// Hotspot buildup in centi-km (km × 100).
    pub buildup_ckm: Vec<u16>,
    /// Feature bits 0..=7, crust_type at bit 15.
    pub flags: Vec<u16>,
    /// Plate whose slab lies beneath this cell (`u16::MAX` = none).
    pub slab_plate: Vec<u16>,
    /// When that slab went under (My; 0 where `slab_plate` is none).
    pub slab_since_my: Vec<u16>,
    /// When this cell last sat on a suture (My; `u16::MAX` = never) — the
    /// suture scar that feeds the strength field (WO-0006 S2).
    pub suture_at_my: Vec<u16>,
    /// Per hotspot: how long it has sat under continental crust (My,
    /// continuous) — the plume rift driver's clock. Indexed like the
    /// history's hotspot list.
    pub hotspot_cont_my: Vec<u16>,
    /// Live rift ledger (active and completed-awaiting-split).
    pub rifts: Vec<ActiveRift>,
    pub plates: Vec<PlateState>,
    pub collisions: Vec<PairTimer>,
}

/// `suture_at_my` cell encoding: `u16::MAX` is the NEVER_SUTURED sentinel,
/// so real times saturate one step short of it.
#[inline]
fn enc_suture(v: f32) -> u16 {
    if v < 0.0 {
        u16::MAX
    } else {
        v.round().clamp(0.0, 65_534.0) as u16
    }
}

/// The value a decoded suture cell holds; quantize_state must mirror this.
#[inline]
pub(super) fn dec_suture(q: u16) -> f32 {
    if q == u16::MAX {
        NEVER_SUTURED
    } else {
        q as f32
    }
}

/// Round, then clamp. Rounding (not truncation) is what makes the encoding
/// idempotent: a value that already sits on the quantization grid — e.g.
/// q × 0.01 recovered from a previous decode — re-encodes to exactly q even
/// though q × 0.01 × 100 is only q ± float error. Resume-from-keyframe
/// bit-exactness depends on this.
#[inline]
fn enc_u16(v: f32) -> u16 {
    v.round().clamp(0.0, 65_535.0) as u16
}

#[inline]
fn enc_i16(v: f32) -> i16 {
    v.round().clamp(-32_768.0, 32_767.0) as i16
}

impl Keyframe {
    /// Approximate heap size in bytes (per-cell arrays dominate).
    pub fn approx_bytes(&self) -> usize {
        self.elev_m.len() * 22
            + self.plates.len() * std::mem::size_of::<PlateState>()
            + self
                .plates
                .iter()
                .map(|p| p.slab.len() * std::mem::size_of::<SlabSegment>())
                .sum::<usize>()
            + self.collisions.len() * std::mem::size_of::<PairTimer>()
    }

    /// Pack raw f32/u32 working arrays into a keyframe. Thickness and buildup
    /// are quantized to 0.01 km, ages saturate at 65,535 My, elevation at
    /// ±32,767 m — all far beyond the physical ranges the model produces.
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        t_my: f32,
        sea_offset_m: f32,
        elev: &[f32],
        plate: &[u32],
        crust_age: &[f32],
        thickness: &[f32],
        orogeny_age: &[f32],
        rift_age: &[f32],
        buildup: &[f32],
        crust_type: &[u32],
        features: &[u32],
        slab_plate: &[u16],
        slab_since_my: &[f32],
        suture_at_my: &[f32],
        hotspot_cont_my: &[f32],
        rifts: Vec<ActiveRift>,
        plates: Vec<PlateState>,
        collisions: Vec<PairTimer>,
    ) -> Keyframe {
        let n = elev.len();
        let mut kf = Keyframe {
            t_my,
            sea_offset_m,
            elev_m: Vec::with_capacity(n),
            plate_id: Vec::with_capacity(n),
            crust_age_my: Vec::with_capacity(n),
            thickness_ckm: Vec::with_capacity(n),
            orogeny_age_my: Vec::with_capacity(n),
            rift_age_my: Vec::with_capacity(n),
            buildup_ckm: Vec::with_capacity(n),
            flags: Vec::with_capacity(n),
            slab_plate: slab_plate.to_vec(),
            slab_since_my: Vec::with_capacity(n),
            suture_at_my: Vec::with_capacity(n),
            hotspot_cont_my: hotspot_cont_my.iter().map(|&v| enc_u16(v)).collect(),
            rifts,
            plates,
            collisions,
        };
        for i in 0..n {
            kf.elev_m.push(enc_i16(elev[i]));
            kf.plate_id.push(plate[i] as u16);
            kf.crust_age_my.push(enc_u16(crust_age[i]));
            kf.thickness_ckm.push(enc_u16(thickness[i] * 100.0));
            kf.orogeny_age_my.push(enc_u16(orogeny_age[i]));
            kf.rift_age_my.push(enc_u16(rift_age[i]));
            kf.buildup_ckm.push(enc_u16(buildup[i] * 100.0));
            kf.slab_since_my.push(enc_u16(slab_since_my[i]));
            kf.suture_at_my.push(enc_suture(suture_at_my[i]));
            let mut f = (features[i] & 0xff) as u16;
            if crust_type[i] != 0 {
                f |= KF_CONTINENT_BIT;
            }
            kf.flags.push(f);
        }
        kf
    }

    /// Decode into the canonical field names on a [`FieldStore`]. This is how
    /// a keyframe becomes "the present": downstream stages read these fields
    /// and never know about time.
    pub fn write_fields(&self, fields: &mut FieldStore) {
        let n = self.elev_m.len();
        assert_eq!(n, fields.cell_count() as usize, "keyframe/grid mismatch");
        let dec = |out: &mut [f32], data: &[u16], k: f32| {
            for (o, &v) in out.iter_mut().zip(data) {
                *o = v as f32 * k;
            }
        };
        {
            let elev = fields.get_or_insert_mut(ELEVATION_M);
            for (o, &v) in elev.iter_mut().zip(&self.elev_m) {
                *o = v as f32;
            }
        }
        dec(
            fields.get_or_insert_mut(CRUST_AGE_MY),
            &self.crust_age_my,
            1.0,
        );
        dec(
            fields.get_or_insert_mut(super::CRUST_THICKNESS_KM),
            &self.thickness_ckm,
            0.01,
        );
        dec(
            fields.get_or_insert_mut(OROGENY_AGE_MY),
            &self.orogeny_age_my,
            1.0,
        );
        dec(
            fields.get_or_insert_mut(RIFT_AGE_MY),
            &self.rift_age_my,
            1.0,
        );
        dec(
            fields.get_or_insert_mut(HOTSPOT_BUILDUP_KM),
            &self.buildup_ckm,
            0.01,
        );
        {
            let plate = fields.get_or_insert_mut_u32(PLATE_ID);
            for (o, &v) in plate.iter_mut().zip(&self.plate_id) {
                *o = v as u32;
            }
        }
        {
            let ct = fields.get_or_insert_mut_u32(CRUST_TYPE);
            for (o, &v) in ct.iter_mut().zip(&self.flags) {
                *o = u32::from(v & KF_CONTINENT_BIT != 0);
            }
        }
        {
            let feat = fields.get_or_insert_mut_u32(FEATURES);
            for (o, &v) in feat.iter_mut().zip(&self.flags) {
                *o = (v & 0xff) as u32;
            }
        }
    }
}

/// Whole-run diagnostics: continental-inventory flows (cells, cumulative)
/// and Wilson-cycle event counts. Recorded into the acceptance results.
#[derive(Clone, Debug, Default)]
pub struct RunDiagnostics {
    pub cont_lost_to_ridge_gap: u64,
    pub cont_lost_to_consumption: u64,
    pub cont_lost_to_rift: u64,
    pub cont_gained_by_advection: u64,
    pub cont_gained_by_arc: u64,
    pub suture_count: u64,
    /// Rift-to-oceanization plate splits (WO-0006 S2: the only breakup path).
    pub breakup_count: u64,
    pub rift_start_count: u64,
    pub rift_failed_count: u64,
    pub microplate_count: u64,
}

/// The full keyframed history of one tectonic run.
pub struct TectonicsHistory {
    pub dt_my: f32,
    pub keyframe_interval_my: f32,
    pub keyframes: Vec<Keyframe>,
    /// Fixed mantle hotspot points (unit vectors) used by the run.
    pub hotspots: Vec<[f32; 3]>,
    pub diagnostics: RunDiagnostics,
}

impl TectonicsHistory {
    pub fn approx_bytes(&self) -> usize {
        self.keyframes.iter().map(Keyframe::approx_bytes).sum()
    }

    /// Index of the keyframe nearest to `t_my`.
    pub fn nearest_index(&self, t_my: f32) -> usize {
        if self.keyframes.is_empty() {
            return 0;
        }
        let i = (t_my / self.keyframe_interval_my).round() as isize;
        i.clamp(0, self.keyframes.len() as isize - 1) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let n = 6u32;
        let elev = vec![-5600.5, 0.4, 2500.0, -40_000.0, 40_000.0, 12.0];
        let plate = vec![0u32, 1, 2, 3, 65_535, 5];
        let age = vec![0.0f32, 10.0, 100_000.0, 55.4, 1.0, 2.0];
        let thick = vec![7.0f32, 35.0, 70.0, 700.0, 0.0, 20.5];
        let orog = vec![0.0f32; 6];
        let rift = vec![0.0f32, 21.0, 0.0, 0.0, 0.0, 0.0];
        let build = vec![0.0f32, 0.0, 3.25, 0.0, 0.0, 0.0];
        let ctype = vec![0u32, 1, 1, 0, 0, 1];
        let feats = vec![0u32, 0b1_0000, 0b100, 0b10, 0, 0b1];
        let slab_plate = vec![u16::MAX, 3, u16::MAX, u16::MAX, 0, u16::MAX];
        let slab_since = vec![0.0f32, 140.0, 0.0, 0.0, 12.0, 0.0];
        let suture_at = vec![NEVER_SUTURED, 118.0, NEVER_SUTURED, 0.0, 90_000.0, 30.4];
        let hotspot_cont = vec![0.0f32, 24.0, 7.6];
        let rifts = vec![ActiveRift {
            plate: 2,
            kind: RiftDriverKind::BackArc,
            stress: 0.5,
            tip_a: 1,
            tip_b: 4,
            done_a: true,
            done_b: false,
            started_my: 100.0,
        }];
        let kf = Keyframe::encode(
            120.0,
            -230.0,
            &elev,
            &plate,
            &age,
            &thick,
            &orog,
            &rift,
            &build,
            &ctype,
            &feats,
            &slab_plate,
            &slab_since,
            &suture_at,
            &hotspot_cont,
            rifts.clone(),
            vec![],
            vec![],
        );
        // Saturation behaves.
        assert_eq!(kf.elev_m[3], -32_768);
        assert_eq!(kf.elev_m[4], 32_767);
        assert_eq!(kf.crust_age_my[2], 65_535);
        assert_eq!(kf.thickness_ckm[3], 65_535);
        // Slab ledger cells round-trip exactly (WO-0006 S1).
        assert_eq!(kf.slab_plate, slab_plate);
        assert_eq!(kf.slab_since_my, &[0u16, 140, 0, 0, 12, 0]);
        // Suture scars round-trip; NEVER_SUTURED maps to the u16::MAX
        // sentinel and real times saturate below it (WO-0006 S2).
        assert_eq!(kf.suture_at_my, &[u16::MAX, 118, u16::MAX, 0, 65_534, 30]);
        assert_eq!(dec_suture(kf.suture_at_my[0]), NEVER_SUTURED);
        assert_eq!(dec_suture(kf.suture_at_my[1]), 118.0);
        // Hotspot residence clocks and the rift ledger survive the trip.
        assert_eq!(kf.hotspot_cont_my, &[0u16, 24, 8]);
        assert_eq!(kf.rifts, rifts);

        let mut fields = FieldStore::new(n);
        kf.write_fields(&mut fields);
        assert_eq!(fields.get(ELEVATION_M).unwrap()[0], -5601.0); // rounds half away
        assert_eq!(
            fields.get(super::super::CRUST_THICKNESS_KM).unwrap()[5],
            20.5
        );
        assert_eq!(fields.get_u32(PLATE_ID).unwrap()[4], 65_535);
        assert_eq!(fields.get_u32(CRUST_TYPE).unwrap(), &[0, 1, 1, 0, 0, 1]);
        assert_eq!(
            fields.get_u32(FEATURES).unwrap(),
            &[0, 0b1_0000, 0b100, 0b10, 0, 0b1]
        );
        assert_eq!(fields.get(RIFT_AGE_MY).unwrap()[1], 21.0);
        assert_eq!(fields.get(HOTSPOT_BUILDUP_KM).unwrap()[2], 3.25);
    }

    #[test]
    fn nearest_index_snaps() {
        let hist = TectonicsHistory {
            dt_my: 2.0,
            keyframe_interval_my: 10.0,
            keyframes: (0..51)
                .map(|i| Keyframe {
                    t_my: i as f32 * 10.0,
                    sea_offset_m: 0.0,
                    elev_m: vec![],
                    plate_id: vec![],
                    crust_age_my: vec![],
                    thickness_ckm: vec![],
                    orogeny_age_my: vec![],
                    rift_age_my: vec![],
                    buildup_ckm: vec![],
                    flags: vec![],
                    slab_plate: vec![],
                    slab_since_my: vec![],
                    suture_at_my: vec![],
                    hotspot_cont_my: vec![],
                    rifts: vec![],
                    plates: vec![],
                    collisions: vec![],
                })
                .collect(),
            hotspots: vec![],
            diagnostics: RunDiagnostics::default(),
        };
        assert_eq!(hist.nearest_index(0.0), 0);
        assert_eq!(hist.nearest_index(14.9), 1);
        assert_eq!(hist.nearest_index(15.1), 2);
        assert_eq!(hist.nearest_index(9_999.0), 50);
    }
}
