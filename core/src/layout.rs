//! Where the map box goes and what is drawn on it, in GUI pixels.

/// The box side for a GUI width: 10% of it, kept to 48..=224 (PLAN-v2 section 3.2).
pub fn box_side(gui_width: f32, percent: f32) -> f32 {
    let s = (gui_width * percent / 100.0 + 0.5) as i32 as f32;
    s.clamp(48.0, 224.0)
}

/// A rectangle of the image to draw: `(x, y, w, h)` in texels, `w` = `h` =
/// `side / zoom * texels_per_block`, centred on texel `(cx, cy)` and kept inside
/// an `image` texel square.
pub fn src_window(
    cx: f64,
    cy: f64,
    side: f32,
    zoom: f32,
    texels_per_block: u32,
    image: u32,
) -> (f32, f32, f32, f32) {
    let w = (side / zoom * texels_per_block as f32).min(image as f32);
    let max = image as f32 - w;
    let x = (cx as f32 - w / 2.0).clamp(0.0, max);
    let y = (cy as f32 - w / 2.0).clamp(0.0, max);
    (x, y, w, w)
}

/// The player arrow as two triangles (each a quad with a repeated corner so any
/// triangulation draws it): tip, back corner, notch. `yaw` in degrees as F3 shows
/// it (0 faces south, 90 west); the map is north up, so south is down the screen.
pub fn arrow(cx: f32, cy: f32, size: f32, yaw_degrees: f32) -> [[(f32, f32); 4]; 2] {
    let r = yaw_degrees.to_radians();
    // Facing on screen: world (-sin yaw, cos yaw) with x right and z down.
    let (fx, fy) = (-r.sin(), r.cos());
    // Right of facing on screen.
    let (rx, ry) = (-fy, fx);
    let at = |f: f32, s: f32| {
        (
            cx + fx * f * size + rx * s * size,
            cy + fy * f * size + ry * s * size,
        )
    };
    let tip = at(0.6, 0.0);
    let notch = at(-0.2, 0.0);
    let left = at(-0.5, -0.45);
    let right = at(-0.5, 0.45);
    [[tip, right, notch, notch], [tip, notch, left, left]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_percent_clamped() {
        assert_eq!(box_side(480.0, 10.0), 48.0);
        assert_eq!(box_side(960.0, 10.0), 96.0);
        assert_eq!(box_side(300.0, 10.0), 48.0);
        assert_eq!(box_side(4000.0, 10.0), 224.0);
        assert_eq!(box_side(855.0, 10.0), 86.0);
    }

    #[test]
    fn window_is_centred_and_clipped() {
        assert_eq!(
            src_window(128.0, 128.0, 96.0, 2.0, 2, 256),
            (80.0, 80.0, 96.0, 96.0)
        );
        assert_eq!(
            src_window(10.0, 250.0, 96.0, 2.0, 2, 256),
            (0.0, 160.0, 96.0, 96.0)
        );
        assert_eq!(src_window(128.0, 128.0, 224.0, 0.5, 2, 256).2, 256.0);
    }

    #[test]
    fn arrow_points_where_the_player_faces() {
        let near =
            |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4;
        // South: down the screen.
        assert!(near(arrow(0.0, 0.0, 10.0, 0.0)[0][0], (0.0, 6.0)));
        // North: up.
        assert!(near(arrow(0.0, 0.0, 10.0, 180.0)[0][0], (0.0, -6.0)));
        // West (yaw 90): left. East (-90): right.
        assert!(near(arrow(0.0, 0.0, 10.0, 90.0)[0][0], (-6.0, 0.0)));
        assert!(near(arrow(5.0, 5.0, 10.0, -90.0)[0][0], (11.0, 5.0)));
    }

    #[test]
    fn arrow_halves_mirror() {
        let [a, b] = arrow(0.0, 0.0, 10.0, 0.0);
        assert_eq!(a[0], b[0]);
        assert_eq!(a[2], b[1]);
        assert!((a[1].0 + b[2].0).abs() < 1e-5 && (a[1].1 - b[2].1).abs() < 1e-5);
    }
}
