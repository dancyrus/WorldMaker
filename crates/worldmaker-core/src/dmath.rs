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
}
