//! Plate-boundary smoothed polylines (WO-0003 Fix 3, d3a §8).
//!
//! CPU chain extraction from a keyframe's plate assignment: mixed icosphere
//! triangles are nodes (their normalized centroids approximate the Voronoi
//! vertices of the cell diagram), each boundary-crossing Delaunay edge links
//! its two flanking mixed triangles, chains split by boundary type, and
//! Chaikin corner-cutting ×2 on the sphere smooths them — junction endpoints
//! pinned, junction-free loops smoothed periodically (judgement A9).
//!
//! Render-only and display-side: nothing here feeds a golden. Still fully
//! deterministic — serial, id-ordered walks, no hashing. The ribbon
//! pipelines that draw the chains live in render.rs.

use worldmaker_core::Grid;
use worldmaker_sim::tectonics::{F_BND_CONVERGENT, F_BND_DIVERGENT};

/// One smoothed boundary polyline; `btype` is the boundary code (1 trench /
/// convergent, 2 ridge / divergent, 3 transform), `pts` unit vectors.
/// `closed` marks a junction-free loop: its last point connects back to its
/// first (the ribbon draws the wrap segment).
pub struct BoundaryChain {
    pub btype: u8,
    pub pts: Vec<[f32; 3]>,
    pub closed: bool,
}

/// All boundary chains for the viewed keyframe; empty when the layer draws
/// none.
pub struct BoundarySet {
    pub chains: Vec<BoundaryChain>,
}

impl BoundarySet {
    pub fn empty() -> BoundarySet {
        BoundarySet { chains: Vec::new() }
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    [v[0] / len, v[1] / len, v[2] / len]
}

/// Point at parameter `t` along the chord a→b, renormalized to the sphere.
fn chord_point(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    normalize([
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ])
}

/// Boundary type for a crossing edge from its two cells' F_BND bits, with
/// the legacy bands' priority (convergent > divergent > transform,
/// layers::bake_values). A crossing whose cells carry no boundary bit this
/// keyframe (transient classification gap) draws as transform.
fn edge_btype(fa: u16, fb: u16) -> u8 {
    let f = (fa as u32) | (fb as u32);
    if f & F_BND_CONVERGENT != 0 {
        1
    } else if f & F_BND_DIVERGENT != 0 {
        2
    } else {
        // Transform — or unclassified this keyframe, which draws the same.
        3
    }
}

struct Node {
    point: [f32; 3],
    junction: bool,
    links: Vec<u32>,
}

struct Link {
    n: [u32; 2],
    btype: u8,
}

/// Extract smoothed boundary chains for one keyframe's plate assignment.
/// Pure function of its arguments; deterministic (serial, id-ordered).
pub fn extract(grid: &Grid, plate_id: &[u16], flags: &[u16]) -> BoundarySet {
    // 1. Nodes: triangles with >= 2 distinct corner plate ids ("mixed");
    //    node point = normalized centroid; 3 distinct ids = junction.
    //    While scanning, collect each mixed triangle's boundary-crossing
    //    edges as (min, max, tri) so step 2 never touches the ~2n clean
    //    triangles again.
    let mut node_of_tri: Vec<u32> = vec![u32::MAX; grid.triangles.len()];
    let mut nodes: Vec<Node> = Vec::new();
    let mut crossing: Vec<(u32, u32, u32)> = Vec::new();
    for (t, tri) in grid.triangles.iter().enumerate() {
        let [i, j, k] = *tri;
        let (pi, pj, pk) = (
            plate_id[i as usize],
            plate_id[j as usize],
            plate_id[k as usize],
        );
        if pi == pj && pj == pk {
            continue;
        }
        let junction = pi != pj && pj != pk && pi != pk;
        node_of_tri[t] = nodes.len() as u32;
        let (a, b, c) = (
            grid.positions[i as usize],
            grid.positions[j as usize],
            grid.positions[k as usize],
        );
        nodes.push(Node {
            point: normalize([a[0] + b[0] + c[0], a[1] + b[1] + c[1], a[2] + b[2] + c[2]]),
            junction,
            links: Vec::new(),
        });
        for (u, v) in [(i, j), (j, k), (k, i)] {
            if plate_id[u as usize] != plate_id[v as usize] {
                crossing.push((u.min(v), u.max(v), t as u32));
            }
        }
    }

    // 2. Links: each crossing edge appears in exactly two mixed triangles
    //    (both flanking triangles contain its differing endpoints). Sorting
    //    by (a, b, tri) enumerates edges in id order — deterministic.
    crossing.sort_unstable();
    let mut links: Vec<Link> = Vec::new();
    let mut i0 = 0;
    while i0 < crossing.len() {
        let (a, b, _) = crossing[i0];
        let mut i1 = i0 + 1;
        while i1 < crossing.len() && crossing[i1].0 == a && crossing[i1].1 == b {
            i1 += 1;
        }
        debug_assert_eq!(
            i1 - i0,
            2,
            "crossing edge without two flanking mixed triangles"
        );
        if i1 - i0 == 2 {
            let n1 = node_of_tri[crossing[i0].2 as usize];
            let n2 = node_of_tri[crossing[i0 + 1].2 as usize];
            let id = links.len() as u32;
            links.push(Link {
                n: [n1, n2],
                btype: edge_btype(flags[a as usize], flags[b as usize]),
            });
            nodes[n1 as usize].links.push(id);
            nodes[n2 as usize].links.push(id);
        }
        i0 = i1;
    }

    // A non-junction node has exactly 2 incident links, a junction 3: the
    // boundary network is a graph of degree-2 paths between degree-3
    // junctions plus junction-free loops.
    let other_end = |l: &Link, node: u32| -> u32 {
        if l.n[0] == node {
            l.n[1]
        } else {
            l.n[0]
        }
    };
    // At a degree-2 node, the link that is not `not_this`.
    let other_link = |nodes: &[Node], node: u32, not_this: u32| -> Option<u32> {
        nodes[node as usize]
            .links
            .iter()
            .copied()
            .find(|&l| l != not_this)
    };

    let mut used = vec![false; links.len()];
    let mut raw: Vec<(u8, Vec<[f32; 3]>, bool)> = Vec::new();

    // 3a. Chains from junction nodes, in ascending node (= triangle) order;
    //     each walk runs through degree-2 nodes until another junction or a
    //     type change (chains are single-type; the continuation is picked up
    //     by pass 3b if no junction ever reaches it).
    for ni in 0..nodes.len() {
        if !nodes[ni].junction {
            continue;
        }
        for li in 0..nodes[ni].links.len() {
            let l0 = nodes[ni].links[li];
            if used[l0 as usize] {
                continue;
            }
            let bt = links[l0 as usize].btype;
            let mut pts = vec![nodes[ni].point];
            let mut cur_node = ni as u32;
            let mut cur_link = l0;
            loop {
                used[cur_link as usize] = true;
                let next = other_end(&links[cur_link as usize], cur_node);
                pts.push(nodes[next as usize].point);
                if nodes[next as usize].junction {
                    break;
                }
                let Some(nl) = other_link(&nodes, next, cur_link) else {
                    break;
                };
                if used[nl as usize] || links[nl as usize].btype != bt {
                    break;
                }
                cur_node = next;
                cur_link = nl;
            }
            raw.push((bt, pts, false));
        }
    }

    // 3b. Remaining links: junction-free loops (single-type → closed) and
    //     arcs between two type changes on such loops (→ open, endpoints
    //     pinned like junctions). Each component is anchored at its lowest
    //     link index — deterministic.
    for l0 in 0..links.len() {
        if used[l0] {
            continue;
        }
        let bt = links[l0].btype;
        used[l0] = true;
        let mut head = links[l0].n[0];
        let mut head_link = l0 as u32;
        let mut tail = links[l0].n[1];
        let mut tail_link = l0 as u32;
        let mut pts = vec![nodes[head as usize].point, nodes[tail as usize].point];
        let mut closed = false;
        // Extend at the tail.
        loop {
            if nodes[tail as usize].junction {
                break;
            }
            let Some(nl) = other_link(&nodes, tail, tail_link) else {
                break;
            };
            if used[nl as usize] || links[nl as usize].btype != bt {
                break;
            }
            let next = other_end(&links[nl as usize], tail);
            used[nl as usize] = true;
            if next == head {
                closed = true;
                break;
            }
            pts.push(nodes[next as usize].point);
            tail = next;
            tail_link = nl;
        }
        // Extend at the head (never needed once a loop closed).
        if !closed {
            let mut front: Vec<[f32; 3]> = Vec::new();
            loop {
                if nodes[head as usize].junction {
                    break;
                }
                let Some(nl) = other_link(&nodes, head, head_link) else {
                    break;
                };
                if used[nl as usize] || links[nl as usize].btype != bt {
                    break;
                }
                let next = other_end(&links[nl as usize], head);
                used[nl as usize] = true;
                front.push(nodes[next as usize].point);
                head = next;
                head_link = nl;
            }
            front.reverse();
            front.extend(pts);
            pts = front;
        }
        raw.push((bt, pts, closed));
    }

    // 4. Chaikin ×2 on the sphere (every new point renormalized): open
    //    chains keep their (junction / type-change) endpoints pinned;
    //    closed loops cut periodically.
    let chains = raw
        .into_iter()
        .filter(|(_, pts, _)| pts.len() >= 2)
        .map(|(btype, mut pts, closed)| {
            for _ in 0..2 {
                pts = chaikin_once(&pts, closed);
            }
            BoundaryChain { btype, pts, closed }
        })
        .collect();
    BoundarySet { chains }
}

/// One Chaikin corner-cutting pass on the sphere.
fn chaikin_once(pts: &[[f32; 3]], closed: bool) -> Vec<[f32; 3]> {
    let n = pts.len();
    if closed {
        let mut out = Vec::with_capacity(2 * n);
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            out.push(chord_point(a, b, 0.25));
            out.push(chord_point(a, b, 0.75));
        }
        out
    } else {
        let mut out = Vec::with_capacity(2 * n);
        out.push(pts[0]);
        for i in 0..n - 1 {
            let (a, b) = (pts[i], pts[i + 1]);
            out.push(chord_point(a, b, 0.25));
            out.push(chord_point(a, b, 0.75));
        }
        out.push(pts[n - 1]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use worldmaker_core::Grid;
    use worldmaker_sim::tectonics::F_BND_TRANSFORM;

    fn unit_ok(p: &[f32; 3]) -> bool {
        let n = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        (n - 1.0).abs() < 1e-3
    }

    #[test]
    fn two_plate_world_gives_one_closed_unit_loop() {
        let grid = Grid::build(4);
        let plate: Vec<u16> = (0..grid.cell_count())
            .map(|c| (grid.positions[c as usize][2] > 0.0) as u16)
            .collect();
        let flags = vec![F_BND_CONVERGENT as u16; grid.cell_count() as usize];
        let set = extract(&grid, &plate, &flags);
        assert_eq!(set.chains.len(), 1, "hemisphere split must be one loop");
        let ch = &set.chains[0];
        assert!(ch.closed);
        assert_eq!(ch.btype, 1);
        assert!(ch.pts.len() >= 8);
        assert!(ch.pts.iter().all(unit_ok));
        // Chaikin x2 quadruples a closed loop's point count.
        assert_eq!(ch.pts.len() % 4, 0);
        // The equatorial loop stays near the equator.
        assert!(ch.pts.iter().all(|p| p[2].abs() < 0.2));
        // Deterministic: a second extraction is bit-identical.
        let set2 = extract(&grid, &plate, &flags);
        assert_eq!(set2.chains.len(), 1);
        assert_eq!(set2.chains[0].pts, ch.pts);
    }

    #[test]
    fn three_plate_world_pins_open_chains_at_polar_junctions() {
        let grid = Grid::build(4);
        let seeds: [[f32; 3]; 3] = [
            [1.0, 0.0, 0.0],
            [-0.5, 0.866_025, 0.0],
            [-0.5, -0.866_025, 0.0],
        ];
        let plate: Vec<u16> = (0..grid.cell_count())
            .map(|c| {
                let p = grid.positions[c as usize];
                let mut best = 0u16;
                let mut bd = f32::NEG_INFINITY;
                for (i, s) in seeds.iter().enumerate() {
                    let d = p[0] * s[0] + p[1] * s[1] + p[2] * s[2];
                    if d > bd {
                        bd = d;
                        best = i as u16;
                    }
                }
                best
            })
            .collect();
        let flags = vec![F_BND_DIVERGENT as u16; grid.cell_count() as usize];
        let set = extract(&grid, &plate, &flags);
        // Three meridional boundaries meeting at two polar triple junctions
        // (possibly with short junction-to-junction connectors where two
        // mixed junction triangles are adjacent).
        assert!(set.chains.len() >= 3, "expected >= 3 chains");
        for ch in &set.chains {
            assert!(!ch.closed, "no junction-free loop in a beachball world");
            assert_eq!(ch.btype, 2);
            assert!(ch.pts.iter().all(unit_ok));
            // Every endpoint is a (pinned) junction near a pole.
            for e in [ch.pts.first().unwrap(), ch.pts.last().unwrap()] {
                assert!(
                    e[2].abs() > 0.8,
                    "chain endpoint away from the poles: {e:?}"
                );
            }
        }
    }

    #[test]
    fn type_change_splits_a_junction_free_loop_into_open_arcs() {
        let grid = Grid::build(4);
        let plate: Vec<u16> = (0..grid.cell_count())
            .map(|c| (grid.positions[c as usize][2] > 0.0) as u16)
            .collect();
        // Convergent flags on x > 0, transform on x <= 0: the equatorial
        // loop must split into single-type open arcs (their endpoints
        // pinned at the type changes), with both types present.
        let flags: Vec<u16> = (0..grid.cell_count())
            .map(|c| {
                if grid.positions[c as usize][0] > 0.0 {
                    F_BND_CONVERGENT as u16
                } else {
                    F_BND_TRANSFORM as u16
                }
            })
            .collect();
        let set = extract(&grid, &plate, &flags);
        assert!(set.chains.len() >= 2, "type change must split the loop");
        assert!(set.chains.iter().all(|c| !c.closed));
        let types: std::collections::BTreeSet<u8> = set.chains.iter().map(|c| c.btype).collect();
        assert!(types.contains(&1) && types.contains(&3));
    }
}
