//! Where the map box goes and what is drawn on it, in GUI pixels.

/// A screen corner for the map box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopRight,
    TopLeft,
    BottomLeft,
    BottomRight,
}

/// A rectangle `(x, y, w, h)` in GUI pixels or image texels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Fullscreen touch controls in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchButton {
    Centre,
    Waypoints,
    AddWaypoint,
    ZoomIn,
    ZoomOut,
    Teleport,
    Rename,
    Delete,
    Done,
    Reset,
}

/// Tappable controls laid out as one bottom row, with a small screen-safe gap.
/// Short controls share the space evenly; the longer labels get more room.
pub fn touch_buttons(gui: (f32, f32), buttons: &[TouchButton]) -> Vec<(TouchButton, Rect)> {
    let gap = 4.0;
    let pad = 4.0;
    let natural: Vec<f32> = buttons
        .iter()
        .map(|b| match b {
            TouchButton::Centre => 48.0,
            TouchButton::Waypoints => 62.0,
            TouchButton::AddWaypoint => 82.0,
            TouchButton::ZoomIn | TouchButton::ZoomOut => 28.0,
            TouchButton::Teleport => 66.0,
            TouchButton::Rename => 52.0,
            TouchButton::Delete => 48.0,
            TouchButton::Done | TouchButton::Reset => 64.0,
        })
        .collect();
    let natural_width: f32 =
        natural.iter().sum::<f32>() + gap * buttons.len().saturating_sub(1) as f32;
    let available = (gui.0 - pad * 2.0).max(0.0);
    let scale = if natural_width > available && natural_width > 0.0 {
        ((available - gap * buttons.len().saturating_sub(1) as f32).max(0.0)
            / natural.iter().sum::<f32>().max(1.0))
        .min(1.0)
    } else {
        1.0
    };
    let widths: Vec<f32> = natural.into_iter().map(|w| w * scale).collect();
    let total = widths.iter().sum::<f32>() + gap * buttons.len().saturating_sub(1) as f32;
    let mut x = ((gui.0 - total) / 2.0).max(0.0);
    let y = (gui.1 - 28.0).max(0.0);
    buttons
        .iter()
        .copied()
        .zip(widths)
        .map(|(button, w)| {
            let rect = Rect {
                x,
                y,
                w,
                h: 20.0f32.min(gui.1),
            };
            x += w + gap;
            (button, rect)
        })
        .collect()
}

pub fn hit_button(buttons: &[(TouchButton, Rect)], x: f32, y: f32) -> Option<TouchButton> {
    buttons.iter().find_map(|(button, r)| {
        (x >= r.x && y >= r.y && x <= r.x + r.w && y <= r.y + r.h).then_some(*button)
    })
}

/// Touch devices reserve the top-right for host controls. Use the left corner
/// only when no placement has ever been saved; explicit settings always win.
pub fn default_place(
    touch: bool,
    stored: bool,
    corner: Corner,
    offset: (f32, f32),
) -> (Corner, (f32, f32)) {
    if touch && !stored {
        (Corner::TopLeft, (28.0, 4.0))
    } else {
        (corner, offset)
    }
}

/// Place a box inward from an anchor corner, keeping it on screen.
pub fn place(anchor: Corner, offset: (f32, f32), size: (f32, f32), gui: (f32, f32)) -> Rect {
    let max_x = (gui.0 - size.0).max(0.0);
    let max_y = (gui.1 - size.1).max(0.0);
    let (x, y) = match anchor {
        Corner::TopLeft => (offset.0, offset.1),
        Corner::TopRight => (gui.0 - size.0 - offset.0, offset.1),
        Corner::BottomLeft => (offset.0, gui.1 - size.1 - offset.1),
        Corner::BottomRight => (gui.0 - size.0 - offset.0, gui.1 - size.1 - offset.1),
    };
    Rect {
        x: x.clamp(0.0, max_x),
        y: y.clamp(0.0, max_y),
        w: size.0,
        h: size.1,
    }
}

/// Anchor a box to the corner nearest its centre and return its inward offset.
pub fn snap(top_left: (f32, f32), size: (f32, f32), gui: (f32, f32)) -> (Corner, (f32, f32)) {
    let x = top_left.0.clamp(0.0, (gui.0 - size.0).max(0.0));
    let y = top_left.1.clamp(0.0, (gui.1 - size.1).max(0.0));
    let cx = x + size.0 / 2.0;
    let cy = y + size.1 / 2.0;
    let left = cx <= gui.0 / 2.0;
    let top = cy <= gui.1 / 2.0;
    let corner = match (top, left) {
        (true, true) => Corner::TopLeft,
        (true, false) => Corner::TopRight,
        (false, true) => Corner::BottomLeft,
        (false, false) => Corner::BottomRight,
    };
    let ox = if left {
        x
    } else {
        (gui.0 - size.0 - x).max(0.0)
    };
    let oy = if top {
        y
    } else {
        (gui.1 - size.1 - y).max(0.0)
    };
    (corner, (ox, oy))
}

/// Info line height in GUI pixels (MinimapHud.java:174).
pub const LINE: f32 = 10.0;

/// "minecraft:dark_forest" as a name: "Dark Forest" (Diorama shows the game's
/// translated name; the port has no language file, so it capitalises the id).
pub fn biome_name(id: &str) -> String {
    let path = id.rsplit(':').next().unwrap_or(id);
    path.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The game time as a 24 hour clock (MinimapHud.java:337-350): the day-time
/// tick 0 is 06:00, `1000` ticks an hour.
pub fn clock_text(day_time: u64) -> String {
    let in_day = (day_time + 6000) % 24000;
    format!("{:02}:{:02}", in_day / 1000, (in_day % 1000) * 60 / 1000)
}

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
    fn names_and_clock() {
        assert_eq!(biome_name("minecraft:dark_forest"), "Dark Forest");
        assert_eq!(biome_name("plains"), "Plains");
        assert_eq!(clock_text(0), "06:00");
        assert_eq!(clock_text(6000), "12:00");
        assert_eq!(clock_text(18000), "00:00");
        assert_eq!(clock_text(23999), "05:59");
    }

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

    #[test]
    fn placement_and_snap_round_trip_all_corners() {
        for c in [
            Corner::TopLeft,
            Corner::TopRight,
            Corner::BottomLeft,
            Corner::BottomRight,
        ] {
            let r = place(c, (13.0, 17.0), (80.0, 60.0), (500.0, 400.0));
            let (corner, offset) = snap((r.x, r.y), (r.w, r.h), (500.0, 400.0));
            assert_eq!(corner, c);
            assert_eq!(place(corner, offset, (r.w, r.h), (500.0, 400.0)), r);
        }
    }

    #[test]
    fn placement_clamps_and_oversize_pins_to_zero() {
        let r = place(
            Corner::BottomRight,
            (900.0, 900.0),
            (80.0, 60.0),
            (500.0, 400.0),
        );
        assert_eq!((r.x, r.y), (0.0, 0.0));
        let r = place(Corner::TopLeft, (0.0, 0.0), (600.0, 500.0), (500.0, 400.0));
        assert_eq!((r.x, r.y), (0.0, 0.0));
    }

    #[test]
    fn touch_button_rows_fit_common_gui_sizes_and_hit_test() {
        use TouchButton::*;
        let row = [Centre, Waypoints, AddWaypoint, ZoomIn, ZoomOut];
        for gui in [(1280.0, 720.0), (640.0, 360.0), (284.0, 160.0)] {
            let buttons = touch_buttons(gui, &row);
            assert_eq!(buttons.len(), row.len());
            assert!(buttons
                .iter()
                .all(|(_, r)| r.x >= 0.0 && r.x + r.w <= gui.0 + 0.01));
            assert!(buttons
                .iter()
                .all(|(_, r)| r.y >= 0.0 && r.y + r.h <= gui.1 + 0.01));
            for (button, r) in &buttons {
                assert_eq!(
                    hit_button(&buttons, r.x + r.w / 2.0, r.y + r.h / 2.0),
                    Some(*button)
                );
            }
        }
        let move_row = touch_buttons((284.0, 160.0), &[Done, Reset]);
        assert_eq!(move_row[0].1.w, 64.0);
        assert_eq!(hit_button(&move_row, 0.0, 0.0), None);
    }

    #[test]
    fn touch_default_placement_only_applies_without_saved_placement() {
        assert_eq!(
            default_place(true, false, Corner::TopRight, (0.0, 0.0)),
            (Corner::TopLeft, (28.0, 4.0))
        );
        assert_eq!(
            default_place(true, true, Corner::TopRight, (0.0, 0.0)),
            (Corner::TopRight, (0.0, 0.0))
        );
        assert_eq!(
            default_place(true, true, Corner::BottomLeft, (11.0, 9.0)),
            (Corner::BottomLeft, (11.0, 9.0))
        );
        assert_eq!(
            default_place(false, false, Corner::TopRight, (0.0, 0.0)),
            (Corner::TopRight, (0.0, 0.0))
        );
    }
}
