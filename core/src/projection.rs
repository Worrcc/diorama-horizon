//! The north-facing map projection (RULES.md section 1, Projection.java).
//!
//! The camera looks north and down at `pitch`. A world point `(x, y, z)` lands on
//! map column `u = x` (east to the right) and map row `v = z*sin - y*cos` (south
//! down, higher blocks up). Map units are blocks along `u`.

/// Sine and cosine of the style pitch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projection {
    pub sin: f64,
    pub cos: f64,
}

impl Projection {
    /// The projection for a pitch in degrees (Diorama's default is 50).
    pub fn new(pitch_degrees: f64) -> Self {
        let r = pitch_degrees.to_radians();
        Projection {
            sin: r.sin(),
            cos: r.cos(),
        }
    }
    /// Map column of `x`.
    pub fn u(&self, x: f64) -> f64 {
        x
    }
    /// Map row of `(y, z)`.
    pub fn v(&self, y: f64, z: f64) -> f64 {
        z * self.sin - y * self.cos
    }
    /// The z where the ray through map row `v` crosses height `y` (`zAt`).
    pub fn z_at(&self, v: f64, y: f64) -> f64 {
        (v + y * self.cos) / self.sin
    }
}

impl Default for Projection {
    fn default() -> Self {
        Projection::new(crate::DEFAULT_PITCH_DEGREES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn z_at_inverts_v() {
        let p = Projection::default();
        for &(y, z) in &[(64.0, 10.25), (-12.0, -300.5), (0.0, 0.0), (319.0, 7.0)] {
            assert!((p.z_at(p.v(y, z), y) - z).abs() < 1e-9);
        }
    }

    #[test]
    fn v_matches_the_formula_at_50_degrees() {
        let p = Projection::default();
        let (s, c) = (50f64.to_radians().sin(), 50f64.to_radians().cos());
        assert!((p.v(70.0, 12.0) - (12.0 * s - 70.0 * c)).abs() < 1e-12);
        assert_eq!(p.u(-5.5), -5.5);
    }
}
