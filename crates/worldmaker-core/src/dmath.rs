//! Deterministic math for the simulation path.
//!
//! The determinism contract bans platform libm calls (sin, cos, exp, atan2,
//! powf, ...) from anything that feeds a committed golden hash: they are not
//! required to be correctly rounded and differ between platforms in the last
//! ulp. Everything here uses only +, −, ×, / and sqrt — all IEEE-exact — in a
//! fixed evaluation order, so results are bit-identical everywhere.
//!
//! Floats drawn from RNG streams are likewise derived here from raw bits, so
//! results never depend on the `rand` crate's float-sampling internals.

use rand::RngCore;

/// Deterministic sin and cos for small angles, |x| ≤ 0.75 rad.
///
/// Fixed-order Taylor polynomials (sin to x⁷, cos to x⁶). Max absolute error
/// on the valid range is ~2e-6 (x⁹/362880 resp. x⁸/40320 at 0.75) — plenty
/// for rotation geometry — and the result is bit-identical on every platform.
/// Callers must keep |x| in range; the tectonic step angle is ≤ 0.042 rad.
#[inline]
pub fn det_sin_cos(x: f32) -> (f32, f32) {
    debug_assert!(x.abs() <= 0.75, "det_sin_cos out of range: {x}");
    let x2 = x * x;
    // Horner form, fixed order.
    let s = x * (1.0 - x2 * (1.0 / 6.0 - x2 * (1.0 / 120.0 - x2 * (1.0 / 5040.0))));
    let c = 1.0 - x2 * (0.5 - x2 * (1.0 / 24.0 - x2 * (1.0 / 720.0)));
    (s, c)
}

/// Deterministic e^x for x ≤ 0 (WO-0009 S2: the terrain stage's uplift
/// decay). Range reduction x = n·ln2 + r with |r| ≤ ln2/2, 2ⁿ built from
/// exponent bits, exp(r) as a fixed-order degree-6 Taylor polynomial —
/// only +, −, ×, / and bit assembly, so the result is bit-identical on
/// every platform. Relative error ≲ 3e-7 on the valid range; inputs
/// below −87 underflow to exactly 0. Callers must pass x ≤ 0.
#[inline]
pub fn det_exp_neg(x: f32) -> f32 {
    debug_assert!(x <= 0.0, "det_exp_neg needs x <= 0: {x}");
    if x < -87.0 {
        return 0.0;
    }
    const LN2: f32 = core::f32::consts::LN_2;
    // n = round(x / ln2); n in [-126, 0] on the valid range.
    let n = (x * (1.0 / LN2) + 0.5).floor() as i32;
    let r = x - n as f32 * LN2;
    // exp(r), Horner, fixed order (|r| ≤ ~0.347).
    let p = 1.0
        + r * (1.0
            + r * (0.5 + r * (1.0 / 6.0 + r * (1.0 / 24.0 + r * (1.0 / 120.0 + r / 720.0)))));
    // 2^n via exponent bits (n ≥ -126 keeps it normal).
    let two_n = f32::from_bits(((127 + n) as u32) << 23);
    p * two_n
}

// ----- vec3 helpers (f32, fixed op order) -----

#[inline]
pub fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
pub fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
pub fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = dot3(v, v).sqrt();
    [v[0] / len, v[1] / len, v[2] / len]
}

#[inline]
pub fn scale3(v: [f32; 3], s: f32) -> [f32; 3] {
    [v[0] * s, v[1] * s, v[2] * s]
}

#[inline]
pub fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
pub fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Great-circle arc length (radians) between two unit vectors, full range
/// [0, π], deterministic to the bit on every platform.
///
/// Algorithm (WO-0003 Fix 2, d2-fix2-design §3.3): arguments are first
/// canonicalized by lexicographic comparison of their f32 bit patterns, so
/// `arc_len3(a, b)` is bit-identical to `arc_len3(b, a)` by construction.
/// Then 4 fixed midpoint-normalize bisections walk the far point to within
/// angle/16 (≤ π/16 ≈ 0.196 rad) of the base point, and the residual chord
/// `c` is converted to an angle by a fixed-order odd series for `2·asin(c/2)`
/// (c + c³/24 + 3c⁵/640). Only +, −, ×, / and sqrt — all IEEE-exact — in a
/// fixed evaluation order.
///
/// Error envelope (measured against f64 acos): ~1e-6 absolute typical,
/// degrading as ~1.2e-7/(π−θ) near the antipodal point (the `a+b` midpoint
/// direction cancels there); ~1.3e-4 at θ = π−1e-3. The antipodal guard
/// returns exactly π when |a+b|² < 1e-12 (true angle within ~1e-6 rad of π).
pub fn arc_len3(a: [f32; 3], b: [f32; 3]) -> f32 {
    // Canonicalize on VALUE bit patterns so symmetry is exact (F11/R5).
    let ka = [a[0].to_bits(), a[1].to_bits(), a[2].to_bits()];
    let kb = [b[0].to_bits(), b[1].to_bits(), b[2].to_bits()];
    let (base, far) = if ka <= kb { (a, b) } else { (b, a) };
    let s = add3(base, far);
    if dot3(s, s) < 1e-12 {
        return std::f32::consts::PI; // antipodal guard, documented above
    }
    let mut m = far;
    for _ in 0..4 {
        // Fixed depth: each step halves the angle to the base point.
        m = normalize3(add3(base, m));
    }
    let d = sub3(m, base);
    let c = dot3(d, d).sqrt(); // residual chord, ≤ 2·sin(π/32) ≈ 0.196
    16.0 * (c * (1.0 + c * c * (1.0 / 24.0 + c * c * (3.0 / 640.0))))
}

/// 3×3 rotation matrix (row-major rows) for rotating by `angle_rad` about the
/// unit `axis`, via Rodrigues' formula with deterministic trig.
/// `angle_rad` must be within [`det_sin_cos`]'s range.
pub fn rotation3(axis: [f32; 3], angle_rad: f32) -> [[f32; 3]; 3] {
    let (s, c) = det_sin_cos(angle_rad);
    let t = 1.0 - c;
    let [x, y, z] = axis;
    [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ]
}

/// Apply a row-major 3×3 matrix to a vector.
#[inline]
pub fn mat3_mul(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Row-major 3×3 matrix product a·b.
pub fn mat3_mul3(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0f32; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

/// Transpose of a rotation matrix = its inverse.
#[inline]
pub fn mat3_transpose(m: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    [
        [m[0][0], m[1][0], m[2][0]],
        [m[0][1], m[1][1], m[2][1]],
        [m[0][2], m[1][2], m[2][2]],
    ]
}

// ----- RNG-derived samples (raw bits, independent of `rand` internals) -----

/// Uniform f32 in [0, 1) from the top 24 bits of one u32 draw.
#[inline]
pub fn uniform_f32(rng: &mut impl RngCore) -> f32 {
    (rng.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// Uniform f32 in [lo, hi).
#[inline]
pub fn uniform_range(rng: &mut impl RngCore, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * uniform_f32(rng)
}

/// Approximately standard-normal sample via Irwin–Hall (12 uniforms − 6).
/// No transcendentals; good to ~3 sigma, which is all the pole walks need.
#[inline]
pub fn gaussian_f32(rng: &mut impl RngCore) -> f32 {
    let mut sum = 0.0f32;
    for _ in 0..12 {
        sum += uniform_f32(rng);
    }
    sum - 6.0
}

/// Uniform random unit vector by rejection sampling from the cube — no trig,
/// deterministic given the stream.
pub fn random_unit_vec(rng: &mut impl RngCore) -> [f32; 3] {
    loop {
        let v = [
            uniform_range(rng, -1.0, 1.0),
            uniform_range(rng, -1.0, 1.0),
            uniform_range(rng, -1.0, 1.0),
        ];
        let d = dot3(v, v);
        if d > 1e-4 && d <= 1.0 {
            return normalize3(v);
        }
    }
}

/// A unit vector perpendicular to `v`, chosen deterministically from the RNG
/// (used for tangent-plane random walks).
pub fn random_tangent(rng: &mut impl RngCore, v: [f32; 3]) -> [f32; 3] {
    loop {
        let r = random_unit_vec(rng);
        let t = cross3(v, r);
        let d = dot3(t, t);
        if d > 1e-4 {
            return normalize3(t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn det_trig_matches_std_within_tolerance() {
        let mut x = -0.75f32;
        while x <= 0.75 {
            let (s, c) = det_sin_cos(x);
            assert!((s - x.sin()).abs() < 3e-6, "sin({x}): {s} vs {}", x.sin());
            assert!((c - x.cos()).abs() < 3e-6, "cos({x}): {c} vs {}", x.cos());
            x += 0.01;
        }
        let (s0, c0) = det_sin_cos(0.0);
        assert_eq!(s0, 0.0);
        assert_eq!(c0, 1.0);
    }

    #[test]
    fn rotation_rotates_and_preserves_length() {
        // 0.5 rad about z takes +x toward +y.
        let m = rotation3([0.0, 0.0, 1.0], 0.5);
        let v = mat3_mul(&m, [1.0, 0.0, 0.0]);
        assert!((v[0] - 0.5f32.cos()).abs() < 3e-6);
        assert!((v[1] - 0.5f32.sin()).abs() < 3e-6);
        assert!(v[2].abs() < 1e-7);
        // Transpose inverts.
        let inv = mat3_transpose(&m);
        let back = mat3_mul(&inv, v);
        assert!((back[0] - 1.0).abs() < 1e-6 && back[1].abs() < 1e-6);
        // Axis is fixed.
        let a = mat3_mul(&m, [0.0, 0.0, 1.0]);
        assert!((a[2] - 1.0).abs() < 1e-6);
    }

    /// Tolerance for `arc_len3` at a reference angle `theta` (radians),
    /// per the pinned F11 amendment: 1e-5·θ + 5e-6 for θ ≤ 3.0, then the
    /// documented absolute tolerances for the near-antipodal band.
    fn arc_tol(theta: f64) -> f64 {
        if theta <= 3.0 {
            1e-5 * theta + 5e-6
        } else if theta <= 3.1 {
            2e-5
        } else {
            5e-4 // documented band up to θ = π − 1e-3
        }
    }

    /// f64 reference angle between two f32 unit vectors. Computed as
    /// 2·atan2(|a−b|, |a+b|) — identical to acos(a·b) in exact arithmetic but
    /// well-conditioned at both ends of [0, π], where acos(dot) loses ~half
    /// the significant digits (acos'(x) → ∞ at ±1).
    fn ref_angle(a: [f32; 3], b: [f32; 3]) -> f64 {
        let mut diff = 0.0f64;
        let mut sum = 0.0f64;
        for i in 0..3 {
            let d = a[i] as f64 - b[i] as f64;
            let s = a[i] as f64 + b[i] as f64;
            diff += d * d;
            sum += s * s;
        }
        2.0 * diff.sqrt().atan2(sum.sqrt())
    }

    /// Build a unit f32 pair at (approximately) the given angle from a base
    /// direction and a tangent, both drawn from a fixed stream. The reference
    /// for the assertion is the f64 angle of the actual f32 vectors.
    fn pair_at_angle(rng: &mut impl rand::RngCore, theta: f64) -> ([f32; 3], [f32; 3]) {
        let a = random_unit_vec(rng);
        let t = random_tangent(rng, a);
        let (s, c) = (theta.sin(), theta.cos());
        let b = [
            (c * a[0] as f64 + s * t[0] as f64) as f32,
            (c * a[1] as f64 + s * t[1] as f64) as f32,
            (c * a[2] as f64 + s * t[2] as f64) as f32,
        ];
        (a, normalize3(b))
    }

    #[test]
    fn arc_len3_matches_f64_acos_at_pinned_angles() {
        use crate::rng::sub_rng;
        let pi = std::f64::consts::PI;
        let angles = [
            0.0,
            1e-4,
            0.01,
            0.3,
            0.75,
            pi / 2.0,
            2.5,
            3.0,
            3.1,
            pi - 1e-3,
        ];
        let mut rng = sub_rng(9, "dmath-test", "arclen-angles");
        for &theta in &angles {
            for _ in 0..16 {
                let (a, b) = pair_at_angle(&mut rng, theta);
                let want = ref_angle(a, b);
                let got = arc_len3(a, b) as f64;
                let tol = arc_tol(want);
                assert!(
                    (got - want).abs() <= tol,
                    "arc_len3 off at theta={theta}: got {got}, want {want}, tol {tol}"
                );
            }
        }
    }

    #[test]
    fn arc_len3_matches_f64_acos_on_random_pairs() {
        use crate::rng::sub_rng;
        let mut rng = sub_rng(9, "dmath-test", "arclen");
        let mut tested = 0;
        for _ in 0..200 {
            let a = random_unit_vec(&mut rng);
            let b = random_unit_vec(&mut rng);
            let want = ref_angle(a, b);
            if want > std::f64::consts::PI - 1e-3 {
                // Outside the tested envelope; the fixed-angle test covers
                // the near-antipodal band explicitly.
                continue;
            }
            tested += 1;
            let got = arc_len3(a, b) as f64;
            let tol = arc_tol(want);
            assert!(
                (got - want).abs() <= tol,
                "arc_len3 off on random pair: got {got}, want {want}, tol {tol}"
            );
        }
        assert!(tested >= 195, "too many pairs skipped: {tested}");
    }

    #[test]
    fn arc_len3_is_bit_exactly_symmetric_and_total() {
        use crate::rng::sub_rng;
        let mut rng = sub_rng(9, "dmath-test", "arclen-sym");
        for _ in 0..200 {
            let a = random_unit_vec(&mut rng);
            let b = random_unit_vec(&mut rng);
            assert_eq!(
                arc_len3(a, b).to_bits(),
                arc_len3(b, a).to_bits(),
                "arc_len3 must be bit-exactly symmetric (canonicalized order)"
            );
        }
        // Antipodal guard: exactly pi, both ways.
        let a = normalize3([0.3, -0.7, 0.648]);
        let na = [-a[0], -a[1], -a[2]];
        assert_eq!(arc_len3(a, na), std::f32::consts::PI);
        assert_eq!(arc_len3(na, a), std::f32::consts::PI);
        // Coincident points measure zero (within chord rounding).
        assert!(arc_len3(a, a).abs() < 1e-6);
    }

    #[test]
    fn samplers_are_sane_and_reproducible() {
        use crate::rng::sub_rng;
        let mut r1 = sub_rng(9, "dmath-test", "samples");
        let mut r2 = sub_rng(9, "dmath-test", "samples");
        for _ in 0..100 {
            let u = uniform_f32(&mut r1);
            assert!((0.0..1.0).contains(&u));
            assert_eq!(u, uniform_f32(&mut r2), "stream must reproduce");
        }
        let mut r = sub_rng(9, "dmath-test", "gauss");
        let mut sum = 0.0f64;
        let mut sq = 0.0f64;
        const N: usize = 4000;
        for _ in 0..N {
            let g = gaussian_f32(&mut r) as f64;
            sum += g;
            sq += g * g;
        }
        let mean = sum / N as f64;
        let var = sq / N as f64 - mean * mean;
        assert!(mean.abs() < 0.06, "gaussian mean off: {mean}");
        assert!((var - 1.0).abs() < 0.1, "gaussian var off: {var}");

        let mut r = sub_rng(9, "dmath-test", "unitvec");
        for _ in 0..100 {
            let v = random_unit_vec(&mut r);
            assert!((dot3(v, v) - 1.0).abs() < 1e-5);
            let t = random_tangent(&mut r, v);
            assert!(dot3(t, v).abs() < 1e-5);
            assert!((dot3(t, t) - 1.0).abs() < 1e-5);
        }
    }

    /// det_exp_neg tracks libm exp closely on its range (the reference is
    /// only a test oracle — production code never calls libm) and hits the
    /// exact endpoints.
    #[test]
    fn det_exp_neg_matches_reference() {
        assert_eq!(det_exp_neg(0.0), 1.0);
        assert_eq!(det_exp_neg(-100.0), 0.0);
        let mut x = -86.0f32;
        while x <= 0.0 {
            let got = det_exp_neg(x) as f64;
            let want = (x as f64).exp();
            let rel = (got - want).abs() / want;
            assert!(rel < 1e-5, "det_exp_neg({x}) = {got}, want {want}");
            x += 0.173;
        }
    }
}
