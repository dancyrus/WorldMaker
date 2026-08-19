//! Compact keyframe encoding for the tectonic history.
//!
//! A keyframe every 10 My holds the full simulation state — per-cell fields
//! packed to 16 bits each plus the per-plate and per-pair bookkeeping — so a
//! run can restart from any keyframe (plate drag now, branching later) and the
//! era picker can decode any moment for display. 16 B/cell keeps a 2 Gy run at
//! L7 (201 keyframes × 163,842 cells) near 0.5 GB, inside the 1 GB budget.

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
    /// Setup-time preferred speed the slab-pull update relaxes toward.
    pub base_speed_deg_my: f32,
    /// Simulation time (My) of this plate's most recent suture, or
    /// [`NEVER_SUTURED`] if it has none.
    pub youngest_suture_my: f32,
    /// Accumulated rotation not yet applied by advection (row-major). Slow
    /// plates bank sub-cell motion here and commit it once it reaches about
    /// one cell, so they never freeze to the grid.
    pub pending_rot: [[f32; 3]; 3],
    /// Total banked rotation angle, degrees.
    pub pending_deg: f32,
    /// Previous step's boundary composition (slab-pull inputs).
    pub boundary_cells: u32,
    pub subducting_cells: u32,
    pub colliding_cells: u32,
}

/// Sentinel for "no suture yet" — old enough that the breakup rule treats a
/// never-sutured supercontinent as eligible.
pub const NEVER_SUTURED: f32 = -1.0e9;

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
    pub plates: Vec<PlateState>,
    pub collisions: Vec<PairTimer>,
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
        self.elev_m.len() * 16
            + self.plates.len() * std::mem::size_of::<PlateState>()
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
    pub breakup_count: u64,
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
            vec![],
            vec![],
        );
        // Saturation behaves.
        assert_eq!(kf.elev_m[3], -32_768);
        assert_eq!(kf.elev_m[4], 32_767);
        assert_eq!(kf.crust_age_my[2], 65_535);
        assert_eq!(kf.thickness_ckm[3], 65_535);

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
