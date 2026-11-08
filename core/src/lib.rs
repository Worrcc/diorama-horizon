//! Pure logic for the Diorama port: projection, baking, settings, waypoints.
//! Numbers and formulas come from RULES.md. No Horizon dependencies.

pub mod bake;
pub mod budget;
pub mod cave;
pub mod json;
pub mod layout;
pub mod looks;
pub mod mosaic;
pub mod paint;
pub mod pattern;
pub mod projection;
pub mod settings;
pub mod style;
pub mod waypoints;

pub use mosaic::Mosaic;
pub use paint::Painter;
pub use projection::Projection;

/// Default camera pitch in degrees (RULES.md section 1; MapStyle.java:62).
pub const DEFAULT_PITCH_DEGREES: f64 = 50.0;

/// Map row of a point: `v = z*sin(pitch) - y*cos(pitch)` (RULES.md section 1).
pub fn map_v(y: f64, z: f64, pitch_degrees: f64) -> f64 {
    Projection::new(pitch_degrees).v(y, z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_ground_row_is_z_times_sin() {
        let v = map_v(0.0, 10.0, DEFAULT_PITCH_DEGREES);
        assert!((v - 10.0 * DEFAULT_PITCH_DEGREES.to_radians().sin()).abs() < 1e-12);
    }

    #[test]
    fn higher_blocks_move_up_the_map() {
        assert!(map_v(70.0, 0.0, 50.0) < map_v(64.0, 0.0, 50.0));
    }
}
