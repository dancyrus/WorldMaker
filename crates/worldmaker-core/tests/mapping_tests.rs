//! Screen↔cell mapping: the projection round-trip must land in the same cell,
//! and the globe path and flat path must agree on the cell under a given
//! lat/lon. Every brush in every later phase depends on this.

use worldmaker_core::grid::{latlon_to_unit, Grid};
use worldmaker_core::proj::Projection;

/// cell → flat-map position → cell returns the same cell, for every cell,
/// in both projections.
#[test]
fn projection_roundtrip_returns_same_cell() {
    let g = Grid::build(6);
    for proj in Projection::ALL {
        let mut hint = None;
        for c in 0..g.cell_count() {
            let (lat, lon) = (g.lat[c as usize], g.lon[c as usize]);
            let (x, y) = proj.project(lat, lon);
            let (lat2, lon2) = proj
                .invert(x, y)
                .unwrap_or_else(|| panic!("{proj:?}: invert failed for cell {c}"));
            let back = g.nearest_cell(latlon_to_unit(lat2, lon2), hint);
            assert_eq!(
                back, c,
                "{proj:?}: cell {c} at ({lat}, {lon}) round-tripped to cell {back}"
            );
            hint = Some(back);
        }
    }
}

/// The globe view resolves a ground position by intersecting a ray with the
/// sphere and calling nearest_cell on the hit point; the flat view resolves it
/// by inverting the projection and calling nearest_cell on the lat/lon unit
/// vector. Both must agree everywhere (here: a dense lat/lon sweep).
#[test]
fn globe_and_flat_agree_on_cell_under_latlon() {
    let g = Grid::build(6);
    let mut hint = None;
    for ilat in (-88..=88).step_by(4) {
        for ilon in (-178..=178).step_by(4) {
            let lat = (ilat as f32).to_radians();
            let lon = (ilon as f32).to_radians();
            // Globe path: the ground position directly as a unit vector.
            let globe_cell = g.nearest_cell(latlon_to_unit(lat, lon), hint);
            hint = Some(globe_cell);
            for proj in Projection::ALL {
                // Flat path: project to map coords, invert, then resolve.
                let (x, y) = proj.project(lat, lon);
                let (lat2, lon2) = proj.invert(x, y).unwrap();
                let flat_cell = g.nearest_cell(latlon_to_unit(lat2, lon2), hint);
                assert_eq!(
                    globe_cell, flat_cell,
                    "{proj:?}: globe and flat disagree at lat {ilat} lon {ilon}"
                );
            }
        }
    }
}

/// Pole and antimeridian edge cases must resolve without panicking and stay
/// consistent between views.
#[test]
fn mapping_edge_cases() {
    let g = Grid::build(5);
    let poles = [(std::f32::consts::FRAC_PI_2, 0.0), (-std::f32::consts::FRAC_PI_2, 0.0)];
    for (lat, lon) in poles {
        let cell = g.nearest_cell(latlon_to_unit(lat, lon), None);
        for proj in Projection::ALL {
            let (x, y) = proj.project(lat, lon);
            let (lat2, lon2) = proj.invert(x, y).unwrap();
            assert_eq!(cell, g.nearest_cell(latlon_to_unit(lat2, lon2), None));
        }
    }
    // Antimeridian: ±180° meet at the same ground position.
    let west = latlon_to_unit(0.3, std::f32::consts::PI);
    let east = latlon_to_unit(0.3, -std::f32::consts::PI);
    assert_eq!(g.nearest_cell(west, None), g.nearest_cell(east, None));
}
