//! Pure view maths for the fullscreen map.

use crate::{layout::Rect, projection::Projection};

const MIN_SCALE: f32 = 0.25;
const MAX_SCALE: f32 = 8.0;

/// Fullscreen map camera. `centre_u/v` use Diorama's projected map coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapView {
    pub centre_u: f64,
    pub centre_v: f64,
    /// GUI pixels per block.
    pub scale: f32,
}

impl Default for MapView {
    fn default() -> Self {
        Self {
            centre_u: 0.0,
            centre_v: 0.0,
            scale: 1.0,
        }
    }
}

impl MapView {
    pub fn new(centre_u: f64, centre_v: f64, scale: f32) -> Self {
        Self {
            centre_u,
            centre_v,
            scale: scale.clamp(MIN_SCALE, MAX_SCALE),
        }
    }
    pub fn centre_on(&mut self, u: f64, v: f64) {
        self.centre_u = u;
        self.centre_v = v;
    }
    /// Move the map content by GUI pixels; dragging right moves the view west.
    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        self.centre_u -= dx as f64 / self.scale as f64;
        self.centre_v -= dy as f64 / self.scale as f64;
    }
    /// Zoom by 1.25 per step while keeping the map coordinate under the cursor fixed.
    pub fn zoom_at(&mut self, steps: f32, cursor: (f32, f32), screen: (f32, f32)) {
        let fixed = self.screen_to_map(cursor, screen);
        self.scale = (self.scale * 1.25f32.powf(steps)).clamp(MIN_SCALE, MAX_SCALE);
        let du = (cursor.0 - screen.0 / 2.0) as f64 / self.scale as f64;
        let dv = (cursor.1 - screen.1 / 2.0) as f64 / self.scale as f64;
        self.centre_u = fixed.0 - du;
        self.centre_v = fixed.1 - dv;
    }
    pub fn screen_to_map(&self, p: (f32, f32), screen: (f32, f32)) -> (f64, f64) {
        (
            self.centre_u + (p.0 - screen.0 / 2.0) as f64 / self.scale as f64,
            self.centre_v + (p.1 - screen.1 / 2.0) as f64 / self.scale as f64,
        )
    }
    pub fn map_to_screen(&self, p: (f64, f64), screen: (f32, f32)) -> (f32, f32) {
        (
            screen.0 / 2.0 + ((p.0 - self.centre_u) * self.scale as f64) as f32,
            screen.1 / 2.0 + ((p.1 - self.centre_v) * self.scale as f64) as f32,
        )
    }
    /// Clip a mosaic image to the visible map and screen. Texel (0,0) is at `image_origin_uv`.
    pub fn src_dst(
        &self,
        image_origin_uv: (f64, f64),
        texels_per_block: f32,
        image_size: (u32, u32),
        screen: (f32, f32),
    ) -> Option<(Rect, Rect)> {
        if texels_per_block <= 0.0 || self.scale <= 0.0 || image_size.0 == 0 || image_size.1 == 0 {
            return None;
        }
        let tpb = texels_per_block as f64;
        let img = (
            image_origin_uv.0,
            image_origin_uv.1,
            image_origin_uv.0 + image_size.0 as f64 / tpb,
            image_origin_uv.1 + image_size.1 as f64 / tpb,
        );
        let view = (
            self.centre_u - screen.0 as f64 / (2.0 * self.scale as f64),
            self.centre_v - screen.1 as f64 / (2.0 * self.scale as f64),
            self.centre_u + screen.0 as f64 / (2.0 * self.scale as f64),
            self.centre_v + screen.1 as f64 / (2.0 * self.scale as f64),
        );
        let (u0, v0, u1, v1) = (
            img.0.max(view.0),
            img.1.max(view.1),
            img.2.min(view.2),
            img.3.min(view.3),
        );
        if u0 >= u1 || v0 >= v1 {
            return None;
        }
        let src = Rect {
            x: ((u0 - img.0) * tpb) as f32,
            y: ((v0 - img.1) * tpb) as f32,
            w: ((u1 - u0) * tpb) as f32,
            h: ((v1 - v0) * tpb) as f32,
        };
        let dst = Rect {
            x: screen.0 / 2.0 + ((u0 - self.centre_u) * self.scale as f64) as f32,
            y: screen.1 / 2.0 + ((v0 - self.centre_v) * self.scale as f64) as f32,
            w: ((u1 - u0) * self.scale as f64) as f32,
            h: ((v1 - v0) * self.scale as f64) as f32,
        };
        Some((src, dst))
    }
    /// Find the surface block under the pointer. Inverse projection and `y_range` bound the z scan.
    /// The camera looks north, so southern (higher-z) columns are nearer: scanning south to north
    /// makes the first top or south face covering `v` the visible block.
    pub fn block_under(
        &self,
        p: (f32, f32),
        screen: (f32, f32),
        y_range: (i32, i32),
        surface_y: impl Fn(i32, i32) -> Option<i32>,
    ) -> Option<(i32, i32, i32)> {
        let (u, v) = self.screen_to_map(p, screen);
        let x = u.floor() as i32;
        let projection = Projection::default();
        if y_range.0 > y_range.1 {
            return None;
        }
        let z_lo = ((v + y_range.0 as f64 * projection.cos) / projection.sin).floor() - 1.0;
        let z_hi = ((v + (y_range.1 as f64 + 1.0) * projection.cos) / projection.sin).ceil() + 1.0;
        let z_lo = z_lo.max(i32::MIN as f64) as i32;
        let z_hi = z_hi.min(i32::MAX as f64) as i32;
        for z in (z_lo..=z_hi).rev() {
            if let Some(y) = surface_y(x, z) {
                let top_north = projection.v(y as f64 + 1.0, z as f64);
                let top_south = projection.v(y as f64 + 1.0, z as f64 + 1.0);
                if v >= top_north.min(top_south) && v <= top_north.max(top_south) {
                    return Some((x, y, z));
                }
                let y_floor = surface_y(x, z + 1).map(|s| s + 1).unwrap_or(y_range.0);
                if y + 1 > y_floor {
                    let face_low = projection.v(y_floor as f64, z as f64 + 1.0);
                    if v >= top_south.min(face_low) && v <= top_south.max(face_low) {
                        return Some((x, y, z));
                    }
                }
            }
        }
        None
    }
    /// Nearest pin within `radius` GUI pixels.
    pub fn pin_hit(
        &self,
        pins: &[(f64, f64)],
        p: (f32, f32),
        screen: (f32, f32),
        radius: f32,
    ) -> Option<usize> {
        pins.iter()
            .enumerate()
            .map(|(i, &pin)| {
                let at = self.map_to_screen(pin, screen);
                (i, (at.0 - p.0).hypot(at.1 - p.1))
            })
            .filter(|(_, d)| *d <= radius)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-5, "{a} != {b}");
    }
    #[test]
    fn coordinate_round_trip_and_pan() {
        let mut v = MapView::new(20.0, -8.0, 2.0);
        let p = (120.0, 70.0);
        let screen = (400.0, 300.0);
        let m = v.screen_to_map(p, screen);
        let q = v.map_to_screen(m, screen);
        assert!((q.0 - p.0).abs() < 1e-4 && (q.1 - p.1).abs() < 1e-4);
        v.pan_by(20.0, -10.0);
        close(v.centre_u, 10.0);
        close(v.centre_v, -3.0);
    }
    #[test]
    fn zoom_keeps_cursor_anchor_and_clamps() {
        let mut v = MapView::new(0.0, 0.0, 1.0);
        let screen = (800.0, 600.0);
        let p = (175.0, 420.0);
        let before = v.screen_to_map(p, screen);
        v.zoom_at(2.0, p, screen);
        let after = v.screen_to_map(p, screen);
        close(before.0, after.0);
        close(before.1, after.1);
        assert!((v.scale - 1.5625).abs() < 1e-6);
        v.zoom_at(100.0, p, screen);
        assert_eq!(v.scale, MAX_SCALE);
        v.zoom_at(-100.0, p, screen);
        assert_eq!(v.scale, MIN_SCALE);
    }
    #[test]
    fn image_clips_at_screen_and_image_edges() {
        let v = MapView::new(0.0, 0.0, 1.0);
        let (src, dst) = v
            .src_dst((-10.0, -10.0), 1.0, (20, 20), (10.0, 10.0))
            .unwrap();
        assert_eq!(
            src,
            Rect {
                x: 5.0,
                y: 5.0,
                w: 10.0,
                h: 10.0
            }
        );
        assert_eq!(
            dst,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0
            }
        );
        assert!(v
            .src_dst((100.0, 100.0), 1.0, (20, 20), (10.0, 10.0))
            .is_none());
    }
    #[test]
    fn flat_world_and_tower_hit_the_visible_top() {
        let mut v = MapView::new(0.5, 0.0, 4.0);
        let screen = (200.0, 200.0);
        let flat = |_x: i32, z: i32| (z == 0).then_some(0);
        let p = v.map_to_screen((0.5, Projection::default().v(1.0, 0.5)), screen);
        assert_eq!(v.block_under(p, screen, (-64, 320), flat), Some((0, 0, 0)));
        let tower = |_x: i32, z: i32| match z {
            0 => Some(0),
            1 => Some(8),
            _ => None,
        };
        v.centre_on(0.5, Projection::default().v(9.0, 1.5));
        let p = v.map_to_screen((0.5, Projection::default().v(9.0, 1.5)), screen);
        assert_eq!(v.block_under(p, screen, (-64, 320), tower), Some((0, 8, 1)));
    }
    #[test]
    fn southern_tower_wins_over_a_far_hill() {
        let projection = Projection::default();
        let mut v = MapView::new(0.5, 0.0, 4.0);
        let screen = (200.0, 200.0);
        let terrain = |_x: i32, z: i32| match z {
            0 => Some(12),
            1 => Some(20),
            _ => None,
        };
        let map_v = projection.v(21.0, 1.5);
        v.centre_on(0.5, map_v);
        let p = v.map_to_screen((0.5, map_v), screen);
        assert_eq!(
            v.block_under(p, screen, (-64, 320), terrain),
            Some((0, 20, 1))
        );
    }
    #[test]
    fn point_on_tower_south_face_hits_tower() {
        let projection = Projection::default();
        let mut v = MapView::new(0.5, 0.0, 4.0);
        let screen = (200.0, 200.0);
        let terrain = |_x: i32, z: i32| match z {
            1 => Some(8),
            2 => Some(0),
            _ => None,
        };
        let face_top = projection.v(9.0, 2.0);
        let face_bottom = projection.v(1.0, 2.0);
        let map_v = (face_top + face_bottom) / 2.0;
        v.centre_on(0.5, map_v);
        let p = v.map_to_screen((0.5, map_v), screen);
        assert_eq!(
            v.block_under(p, screen, (-64, 320), terrain),
            Some((0, 8, 1))
        );
    }
    #[test]
    fn block_under_handles_large_z() {
        let projection = Projection::default();
        let z = 100_000;
        let map_v = projection.v(1.0, z as f64 + 0.5);
        let v = MapView::new(0.5, map_v, 4.0);
        let screen = (200.0, 200.0);
        let flat = |_x: i32, at_z: i32| (at_z == z).then_some(0);
        let p = v.map_to_screen((0.5, map_v), screen);
        assert_eq!(v.block_under(p, screen, (-64, 320), flat), Some((0, 0, z)));
    }
    #[test]
    fn pin_hit_selects_nearest() {
        let v = MapView::new(0.0, 0.0, 2.0);
        let screen = (100.0, 100.0);
        assert_eq!(
            v.pin_hit(&[(0.0, 0.0), (2.0, 0.0)], (54.0, 50.0), screen, 10.0),
            Some(1)
        );
        assert_eq!(v.pin_hit(&[(0.0, 0.0)], (90.0, 90.0), screen, 5.0), None);
    }
}
