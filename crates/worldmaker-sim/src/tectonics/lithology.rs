//! Per-cell lithology: the 16-class GLiM rock taxonomy (Hartmann &
//! Moosdorf 2012), written by the crust events that create or rework crust
//! (WO-0009 S2). A passive tracer: lithology advects with crust, is stamped
//! at recorded crust events, and never feeds back into the tectonic
//! dynamics — which is what keeps the pre-existing tectonic goldens
//! untouched. The terrain stage reads it as the per-substrate erodibility
//! index (`K_LITH`), and the deposition pass writes `SU` back into its own
//! output field.
//!
//! Classes written by the sim (WO-0009 S2 step 2):
//! - setup craton cores and painted cratons → [`PA`]
//! - non-craton continent at setup → [`SM`]
//! - collision-thickened belts (distributed-zone deposits) → [`MT`]
//! - island-arc conversion → [`VI`]
//! - hotspot buildup (past the feature threshold) and ridge fill
//!   (new ocean crust, rift oceanization included) → [`VB`]
//! - rift-shoulder exposure (active continental thinning) → [`PB`]
//! - sediment deposition (terrain stage, its own output field) → [`SU`]
//!
//! Documented Phase 3+ gaps: `SC`, `EV`, `PY`, `IG` are never written (no
//! carbonate platforms, evaporite basins, pyroclastic provinces, or
//! undifferentiated igneous sources in the model yet); `SS`, `PI`, `VA`
//! await finer petrology; `WB`/`ND` are reserved (water bodies / no data).
//! Conversions the WO does not list keep their advected class — a relic
//! basin closed into a margin, a foreland shelf converted under load, or a
//! spreading-fed shelf all keep their oceanic `VB` basement (ophiolitic
//! floor under the new continent).

/// Unconsolidated sediments.
pub const SU: u8 = 0;
/// Siliciclastic sedimentary rocks.
pub const SS: u8 = 1;
/// Mixed sedimentary rocks.
pub const SM: u8 = 2;
/// Carbonate sedimentary rocks.
pub const SC: u8 = 3;
/// Pyroclastics.
pub const PY: u8 = 4;
/// Evaporites.
pub const EV: u8 = 5;
/// Metamorphic rocks.
pub const MT: u8 = 6;
/// Acid plutonic rocks.
pub const PA: u8 = 7;
/// Intermediate plutonic rocks.
pub const PI: u8 = 8;
/// Basic plutonic rocks.
pub const PB: u8 = 9;
/// Acid volcanic rocks.
pub const VA: u8 = 10;
/// Intermediate volcanic rocks.
pub const VI: u8 = 11;
/// Basic volcanic rocks.
pub const VB: u8 = 12;
/// Undifferentiated igneous rocks.
pub const IG: u8 = 13;
/// Water bodies.
pub const WB: u8 = 14;
/// No data.
pub const ND: u8 = 15;

/// Number of classes (encoding range: `0..CLASS_COUNT`).
pub const CLASS_COUNT: usize = 16;

/// GLiM two-letter code per class, indexed by class value.
pub const CODES: [&str; CLASS_COUNT] = [
    "su", "ss", "sm", "sc", "py", "ev", "mt", "pa", "pi", "pb", "va", "vi", "vb", "ig", "wb", "nd",
];

/// Human-readable name per class, indexed by class value (legend rows).
pub const NAMES: [&str; CLASS_COUNT] = [
    "Unconsolidated sediment",
    "Siliciclastic sedimentary",
    "Mixed sedimentary",
    "Carbonate sedimentary",
    "Pyroclastics",
    "Evaporites",
    "Metamorphic",
    "Acid plutonic",
    "Intermediate plutonic",
    "Basic plutonic",
    "Acid volcanic",
    "Intermediate volcanic",
    "Basic volcanic",
    "Undifferentiated igneous",
    "Water body",
    "No data",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_names_line_up() {
        assert_eq!(CODES.len(), CLASS_COUNT);
        assert_eq!(NAMES.len(), CLASS_COUNT);
        assert_eq!(CODES[SU as usize], "su");
        assert_eq!(CODES[PA as usize], "pa");
        assert_eq!(CODES[VB as usize], "vb");
        assert_eq!(CODES[ND as usize], "nd");
    }
}
