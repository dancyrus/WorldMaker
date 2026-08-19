//! Grid invariants: counts, topology, determinism, and the position→cell walk.

use worldmaker_core::grid::{cell_count_for_level, latlon_to_unit, Grid};
use worldmaker_core::hash::{hash_f32_slice, splitmix64};

#[test]
fn vertex_count_matches_formula_per_level() {
    for level in 0..=6 {
        let g = Grid::build(level);
        assert_eq!(
            g.cell_count(),
            cell_count_for_level(level),
            "cell count wrong at level {level}"
        );
        assert_eq!(
            g.triangles.len() as u64,
            20 * 4u64.pow(level),
            "face count wrong at L{level}"
        );
    }
    assert_eq!(cell_count_for_level(6), 40_962);
    assert_eq!(cell_count_for_level(7), 163_842);
    assert_eq!(cell_count_for_level(8), 655_362);
    assert_eq!(cell_count_for_level(9), 2_621_442);
}

#[test]
fn generates_l6_through_l8_without_panic() {
    for level in 6..=8 {
        let g = Grid::build(level);
        assert_eq!(g.cell_count(), cell_count_for_level(level));
        assert_eq!(g.pentagon_count(), 12, "pentagon count wrong at L{level}");
    }
}

#[test]
fn pentagon_count_is_12_and_degrees_are_5_or_6() {
    let g = Grid::build(5);
    let mut pentagons = 0;
    for c in 0..g.cell_count() {
        let deg = g.neighbors_of(c).len();
        assert!(deg == 5 || deg == 6, "cell {c} has degree {deg}");
        if deg == 5 {
            pentagons += 1;
            assert!(
                c < 12,
                "pentagons must be the 12 original icosahedron vertices"
            );
        }
    }
    assert_eq!(pentagons, 12);
}

#[test]
fn neighbor_table_is_symmetric() {
    let g = Grid::build(5);
    for c in 0..g.cell_count() {
        for &nb in g.neighbors_of(c) {
            assert_ne!(nb, c, "cell {c} lists itself as a neighbor");
            assert!(
                g.neighbors_of(nb).contains(&c),
                "asymmetric neighbors: {c} lists {nb} but not vice versa"
            );
        }
    }
}

#[test]
fn positions_are_unit_vectors() {
    let g = Grid::build(4);
    for p in &g.positions {
        let len2 = p[0] * p[0] + p[1] * p[1] + p[2] * p[2];
        assert!((len2 - 1.0).abs() < 1e-5);
    }
}

#[test]
fn grid_build_is_bit_deterministic() {
    let a = Grid::build(4);
    let b = Grid::build(4);
    let flat_a: Vec<f32> = a.positions.iter().flatten().copied().collect();
    let flat_b: Vec<f32> = b.positions.iter().flatten().copied().collect();
    assert_eq!(hash_f32_slice(&flat_a), hash_f32_slice(&flat_b));
    assert_eq!(a.neighbors, b.neighbors);
    assert_eq!(a.neighbor_offsets, b.neighbor_offsets);
    assert_eq!(a.triangles, b.triangles);
}

/// The greedy walk must return the true nearest cell (cells are Voronoi
/// regions of cell centers, so brute-force max dot product is ground truth).
#[test]
fn nearest_cell_matches_brute_force() {
    let g = Grid::build(5); // 10,242 cells
    let mut state = 0xC0FFEEu64;
    let mut rand_unit = || {
        // Deterministic pseudo-random points via splitmix64, rejection-sampled
        // to the unit ball then normalized.
        loop {
            state = state.wrapping_add(1);
            let a = splitmix64(state);
            let x = ((a >> 40) as f32) / 8_388_608.0 - 1.0;
            let y = (((a >> 16) & 0xFF_FFFF) as f32) / 8_388_608.0 - 1.0;
            let z = ((splitmix64(a) >> 40) as f32) / 8_388_608.0 - 1.0;
            let l2 = x * x + y * y + z * z;
            if l2 > 1e-4 && l2 <= 1.0 {
                let l = l2.sqrt();
                return [x / l, y / l, z / l];
            }
        }
    };
    let mut hint = None;
    for i in 0..2000 {
        let p = rand_unit();
        let brute = (0..g.cell_count())
            .max_by(|&a, &b| {
                let pa = g.positions[a as usize];
                let pb = g.positions[b as usize];
                let da = pa[0] * p[0] + pa[1] * p[1] + pa[2] * p[2];
                let db = pb[0] * p[0] + pb[1] * p[1] + pb[2] * p[2];
                da.partial_cmp(&db).unwrap().then(b.cmp(&a))
            })
            .unwrap();
        let walked_cold = g.nearest_cell(p, None);
        let walked_hinted = g.nearest_cell(p, hint);
        assert_eq!(walked_cold, brute, "cold walk wrong on sample {i}");
        assert_eq!(walked_hinted, brute, "hinted walk wrong on sample {i}");
        hint = Some(walked_hinted);
    }
}

#[test]
fn every_cell_center_maps_to_itself() {
    let g = Grid::build(6);
    let mut hint = None;
    for c in 0..g.cell_count() {
        let found = g.nearest_cell(g.positions[c as usize], hint);
        assert_eq!(found, c, "cell {c} center did not resolve to itself");
        hint = Some(found);
    }
}

#[test]
fn latlon_conversion_roundtrips() {
    let g = Grid::build(4);
    for c in (0..g.cell_count()).step_by(7) {
        let p = latlon_to_unit(g.lat[c as usize], g.lon[c as usize]);
        let q = g.positions[c as usize];
        let dot = p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
        assert!(
            dot > 0.999_99,
            "lat/lon of cell {c} does not point back at it (dot {dot})"
        );
    }
}
