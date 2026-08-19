//! Geodesic grid: a subdivided icosahedron whose vertices are the centers of
//! dual Goldberg cells (12 pentagons, the rest hexagons).
//!
//! Cell ids are the icosphere vertex indices: `u32`, stable for a given
//! subdivision level, with the 12 original icosahedron vertices always taking
//! ids 0..12 (these are the pentagons). Neighbors are stored in a CSR table,
//! ordered counter-clockwise as seen from outside the sphere.
//!
//! Vertex positions are computed in f64 and stored as f32 unit vectors, so the
//! stored geometry is bit-identical across platforms (only `sqrt` and basic
//! arithmetic are involved). Latitude/longitude are derived with `atan2`/`asin`
//! for display and projection use; nothing that feeds a determinism hash may
//! depend on them.

use rayon::prelude::*;
use std::collections::HashMap;

/// Number of cells (icosphere vertices) at subdivision level `level`.
pub fn cell_count_for_level(level: u32) -> u32 {
    10 * 4u32.pow(level) + 2
}

/// A geodesic grid at one subdivision level.
pub struct Grid {
    pub level: u32,
    /// Cell centers as unit vectors, indexed by cell id.
    pub positions: Vec<[f32; 3]>,
    /// Latitude in radians, [-pi/2, pi/2], indexed by cell id.
    pub lat: Vec<f32>,
    /// Longitude in radians, (-pi, pi], indexed by cell id.
    pub lon: Vec<f32>,
    /// CSR offsets into `neighbors`; length = cell_count + 1.
    pub neighbor_offsets: Vec<u32>,
    /// CSR neighbor lists, CCW-ordered viewed from outside the sphere.
    pub neighbors: Vec<u32>,
    /// Icosphere triangle faces (CCW from outside). This is the render mesh;
    /// the cells themselves are the dual polygons around each vertex.
    pub triangles: Vec<[u32; 3]>,
}

impl Grid {
    /// Build the grid at the given subdivision level (0..=9 supported;
    /// presets use 6..=9).
    pub fn build(level: u32) -> Grid {
        assert!(
            level <= 9,
            "subdivision level {level} not supported (max 9)"
        );
        let (positions64, triangles) = build_icosphere(level);
        let n = positions64.len();
        debug_assert_eq!(n as u32, cell_count_for_level(level));

        let positions: Vec<[f32; 3]> = positions64
            .iter()
            .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
            .collect();

        let (neighbor_offsets, neighbors) = build_csr_neighbors(&positions64, &triangles);

        let mut lat = vec![0f32; n];
        let mut lon = vec![0f32; n];
        lat.par_iter_mut()
            .zip(lon.par_iter_mut())
            .enumerate()
            .for_each(|(i, (la, lo))| {
                let p = positions64[i];
                *la = p[2].clamp(-1.0, 1.0).asin() as f32;
                *lo = p[1].atan2(p[0]) as f32;
            });

        Grid {
            level,
            positions,
            lat,
            lon,
            neighbor_offsets,
            neighbors,
            triangles,
        }
    }

    #[inline]
    pub fn cell_count(&self) -> u32 {
        self.positions.len() as u32
    }

    /// Neighbor cell ids of `cell`, CCW from outside.
    #[inline]
    pub fn neighbors_of(&self, cell: u32) -> &[u32] {
        let a = self.neighbor_offsets[cell as usize] as usize;
        let b = self.neighbor_offsets[cell as usize + 1] as usize;
        &self.neighbors[a..b]
    }

    /// Number of pentagon cells (neighbor count 5). Always 12 on a valid grid.
    pub fn pentagon_count(&self) -> usize {
        (0..self.cell_count())
            .filter(|&c| self.neighbors_of(c).len() == 5)
            .count()
    }

    /// The cell whose center is nearest to the given unit vector — i.e. the
    /// Goldberg cell containing that point (cells are the Voronoi regions of
    /// the cell centers).
    ///
    /// This is the one true position→cell mapping; every canvas hit-test and
    /// every future brush must go through it. Greedy walk on the neighbor
    /// graph: O(1) with a nearby `hint`, O(sqrt(n)) cold. Never allocates.
    pub fn nearest_cell(&self, target: [f32; 3], hint: Option<u32>) -> u32 {
        let dot = |c: u32| -> f32 {
            let p = self.positions[c as usize];
            p[0] * target[0] + p[1] * target[1] + p[2] * target[2]
        };
        // Start from the hint if given, else the best of the 12 base vertices.
        let mut current = match hint {
            Some(h) if h < self.cell_count() => h,
            _ => {
                let mut best = 0u32;
                let mut best_d = dot(0);
                for c in 1..12u32 {
                    let d = dot(c);
                    if d > best_d {
                        best_d = d;
                        best = c;
                    }
                }
                best
            }
        };
        let mut current_d = dot(current);
        loop {
            let mut best = current;
            let mut best_d = current_d;
            for &nb in self.neighbors_of(current) {
                let d = dot(nb);
                // Strict improvement, ties broken toward the lower id so the
                // walk is deterministic and cannot cycle.
                if d > best_d || (d == best_d && nb < best) {
                    best = nb;
                    best_d = d;
                }
            }
            if best == current {
                return current;
            }
            current = best;
            current_d = best_d;
        }
    }
}

/// Convert latitude/longitude (radians) to a unit vector.
#[inline]
pub fn latlon_to_unit(lat: f32, lon: f32) -> [f32; 3] {
    let (cl, sl) = (lat.cos(), lat.sin());
    [cl * lon.cos(), cl * lon.sin(), sl]
}

/// Convert a unit vector to (latitude, longitude) in radians.
#[inline]
pub fn unit_to_latlon(p: [f32; 3]) -> (f32, f32) {
    (p[2].clamp(-1.0, 1.0).asin(), p[1].atan2(p[0]))
}

/// Build the subdivided icosahedron: unit-vector vertices (f64) and CCW faces.
fn build_icosphere(level: u32) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    // Base icosahedron from the three golden-ratio rectangles.
    let phi = (1.0 + 5.0f64.sqrt()) / 2.0;
    let inv = 1.0 / (1.0 + phi * phi).sqrt();
    let a = phi * inv; // long coordinate
    let b = inv; // short coordinate
    let mut verts: Vec<[f64; 3]> = vec![
        [-b, a, 0.0],
        [b, a, 0.0],
        [-b, -a, 0.0],
        [b, -a, 0.0],
        [0.0, -b, a],
        [0.0, b, a],
        [0.0, -b, -a],
        [0.0, b, -a],
        [a, 0.0, -b],
        [a, 0.0, b],
        [-a, 0.0, -b],
        [-a, 0.0, b],
    ];
    let mut faces: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    // Enforce outward CCW winding programmatically rather than trusting the table.
    for f in &mut faces {
        let (v0, v1, v2) = (
            verts[f[0] as usize],
            verts[f[1] as usize],
            verts[f[2] as usize],
        );
        let e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
        let e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
        let nrm = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let centroid = [
            v0[0] + v1[0] + v2[0],
            v0[1] + v1[1] + v2[1],
            v0[2] + v1[2] + v2[2],
        ];
        if nrm[0] * centroid[0] + nrm[1] * centroid[1] + nrm[2] * centroid[2] < 0.0 {
            f.swap(1, 2);
        }
    }

    for _ in 0..level {
        // Midpoint cache: undirected edge -> new vertex id. The HashMap is only
        // ever *looked up* by key; iteration order never matters, and faces are
        // processed in fixed order, so vertex numbering is fully deterministic.
        let mut midpoint: HashMap<u64, u32> = HashMap::with_capacity(faces.len() * 3 / 2 + 8);
        let mut next_faces: Vec<[u32; 3]> = Vec::with_capacity(faces.len() * 4);
        let mut mid = |a: u32, b: u32, verts: &mut Vec<[f64; 3]>| -> u32 {
            let key = if a < b {
                ((a as u64) << 32) | b as u64
            } else {
                ((b as u64) << 32) | a as u64
            };
            *midpoint.entry(key).or_insert_with(|| {
                let pa = verts[a as usize];
                let pb = verts[b as usize];
                let m = [pa[0] + pb[0], pa[1] + pb[1], pa[2] + pb[2]];
                let len = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
                verts.push([m[0] / len, m[1] / len, m[2] / len]);
                (verts.len() - 1) as u32
            })
        };
        for f in &faces {
            let [v0, v1, v2] = *f;
            let m01 = mid(v0, v1, &mut verts);
            let m12 = mid(v1, v2, &mut verts);
            let m20 = mid(v2, v0, &mut verts);
            // Subdivision preserves winding.
            next_faces.push([v0, m01, m20]);
            next_faces.push([v1, m12, m01]);
            next_faces.push([v2, m20, m12]);
            next_faces.push([m01, m12, m20]);
        }
        faces = next_faces;
    }
    (verts, faces)
}

/// Build the CSR neighbor table from the triangle mesh, each ring CCW-ordered
/// (viewed from outside) starting from the neighbor with the lowest id.
fn build_csr_neighbors(verts: &[[f64; 3]], faces: &[[u32; 3]]) -> (Vec<u32>, Vec<u32>) {
    let n = verts.len();
    // Each directed edge (a -> b) of a closed CCW mesh appears exactly once,
    // so collecting directed edges gives each vertex each neighbor exactly once.
    let mut degree = vec![0u32; n];
    for f in faces {
        degree[f[0] as usize] += 1;
        degree[f[1] as usize] += 1;
        degree[f[2] as usize] += 1;
    }
    let mut offsets = vec![0u32; n + 1];
    for i in 0..n {
        offsets[i + 1] = offsets[i] + degree[i];
    }
    let mut neighbors = vec![0u32; offsets[n] as usize];
    let mut cursor: Vec<u32> = offsets[..n].to_vec();
    for f in faces {
        let [a, b, c] = *f;
        neighbors[cursor[a as usize] as usize] = b;
        cursor[a as usize] += 1;
        neighbors[cursor[b as usize] as usize] = c;
        cursor[b as usize] += 1;
        neighbors[cursor[c as usize] as usize] = a;
        cursor[c as usize] += 1;
    }

    // Sort each ring CCW by angle in the local tangent plane. Sorting is by an
    // f64 angle computed with atan2; the comparison is a total order over the
    // distinct neighbor directions, and the CCW *sequence* (which is all
    // downstream code relies on) is platform-independent even if atan2 differs
    // in the last ulp. Start each ring at its lowest neighbor id so the stored
    // order is canonical.
    let ranges: Vec<(usize, usize)> = (0..n)
        .map(|i| (offsets[i] as usize, offsets[i + 1] as usize))
        .collect();
    // Split `neighbors` into per-vertex mutable chunks for safe parallel sorting.
    let mut chunks: Vec<&mut [u32]> = Vec::with_capacity(n);
    {
        let mut rest: &mut [u32] = &mut neighbors;
        let mut prev_end = 0usize;
        for &(start, end) in &ranges {
            debug_assert_eq!(start, prev_end);
            let (head, tail) = rest.split_at_mut(end - start);
            chunks.push(head);
            rest = tail;
            prev_end = end;
        }
    }
    chunks.par_iter_mut().enumerate().for_each(|(i, ring)| {
        let p = verts[i];
        // Local tangent basis at p (robust: pick the axis least aligned with p).
        let up = if p[2].abs() < 0.9 {
            [0.0, 0.0, 1.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let e1 = normalize(cross(up, p));
        let e2 = cross(p, e1); // p × e1 completes a right-handed basis
        ring.sort_by(|&x, &y| {
            let ax = ring_angle(verts[x as usize], p, e1, e2);
            let ay = ring_angle(verts[y as usize], p, e1, e2);
            ax.partial_cmp(&ay).unwrap().then(x.cmp(&y))
        });
        // Rotate so the ring starts at the smallest id (canonical form).
        if let Some(min_pos) = ring
            .iter()
            .enumerate()
            .min_by_key(|&(_, &id)| id)
            .map(|(pos, _)| pos)
        {
            ring.rotate_left(min_pos);
        }
    });

    (offsets, neighbors)
}

#[inline]
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
fn normalize(v: [f64; 3]) -> [f64; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / len, v[1] / len, v[2] / len]
}

/// Angle of neighbor `q` around `p` in the tangent basis (e1, e2), CCW from
/// outside the sphere.
#[inline]
fn ring_angle(q: [f64; 3], p: [f64; 3], e1: [f64; 3], e2: [f64; 3]) -> f64 {
    let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
    let x = d[0] * e1[0] + d[1] * e1[1] + d[2] * e1[2];
    let y = d[0] * e2[0] + d[1] * e2[1] + d[2] * e2[2];
    y.atan2(x)
}
