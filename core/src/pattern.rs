//! The M2 test pattern: a checkerboard of 1-block cells lying flat at one height,
//! drawn through the projection into an RGBA image. Colours are the port's own.

use crate::projection::Projection;

/// Light and dark cells, 0xRRGGBB (plain greys, not from Diorama).
pub const LIGHT: u32 = 0xC8C8C8;
pub const DARK: u32 = 0x5A5A5A;

/// Image texels per block along `u` (and per map unit along `v`).
pub const TEXELS_PER_BLOCK: u32 = 2;

/// Where an image sits on the map: its top left texel's map position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    /// Map column and row at the top left of texel (0, 0).
    pub u0: f64,
    pub v0: f64,
    /// The height the checkerboard lies at.
    pub ground_y: i32,
}

impl Anchor {
    /// The anchor that puts the player near the middle of a `side` texel image,
    /// snapped to `step` blocks so it only moves now and then.
    pub fn around(proj: &Projection, x: f64, y: f64, z: f64, side: u32, step: f64) -> Self {
        let ground_y = y.floor() as i32;
        let half = f64::from(side) / f64::from(TEXELS_PER_BLOCK) / 2.0;
        let u = proj.u(x);
        let v = proj.v(f64::from(ground_y), z);
        Anchor {
            u0: floor(u / step) * step - half,
            v0: floor(v / step) * step - half,
            ground_y,
        }
    }
    /// The texel position (fractional) of map point `(u, v)`.
    pub fn texel(&self, u: f64, v: f64) -> (f64, f64) {
        let t = f64::from(TEXELS_PER_BLOCK);
        ((u - self.u0) * t, (v - self.v0) * t)
    }
}

fn floor(a: f64) -> f64 {
    let t = a as i64 as f64;
    if t > a {
        t - 1.0
    } else {
        t
    }
}

/// Whether block `(bx, bz)` is a light cell.
pub fn is_light(bx: i64, bz: i64) -> bool {
    (bx + bz).rem_euclid(2) == 0
}

/// The colour of texel `(i, j)`: the cell the texel centre's ray hits at `ground_y`.
pub fn texel_color(proj: &Projection, a: &Anchor, i: u32, j: u32) -> u32 {
    let t = f64::from(TEXELS_PER_BLOCK);
    let u = a.u0 + (f64::from(i) + 0.5) / t;
    let v = a.v0 + (f64::from(j) + 0.5) / t;
    let x = u;
    let z = proj.z_at(v, f64::from(a.ground_y));
    if is_light(floor(x) as i64, floor(z) as i64) {
        LIGHT
    } else {
        DARK
    }
}

/// A `side` x `side` RGBA image (opaque, rows top first) of the checkerboard.
pub fn checkerboard(proj: &Projection, a: &Anchor, side: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((side * side * 4) as usize);
    for j in 0..side {
        for i in 0..side {
            let c = texel_color(proj, a, i, j);
            rgba.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]);
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_alternate_along_x() {
        assert!(is_light(0, 0));
        assert!(!is_light(1, 0));
        assert!(!is_light(-1, 0));
        assert!(is_light(-1, -1));
    }

    #[test]
    fn a_block_is_two_texels_wide_and_its_projected_depth_tall() {
        let p = Projection::default();
        let a = Anchor {
            u0: 0.0,
            v0: p.v(64.0, 0.0),
            ground_y: 64,
        };
        // Across: 2 texels per block.
        let row: Vec<u32> = (0..8).map(|i| texel_color(&p, &a, i, 0)).collect();
        assert_eq!(row, [LIGHT, LIGHT, DARK, DARK, LIGHT, LIGHT, DARK, DARK]);
        // Down: a block of z is sin(50) map units, about 1.53 texels.
        let col: Vec<u32> = (0..6).map(|j| texel_color(&p, &a, 0, j)).collect();
        let changes = col.windows(2).filter(|w| w[0] != w[1]).count();
        assert_eq!(changes, 3, "{col:?}");
    }

    #[test]
    fn the_player_lands_near_the_image_middle() {
        let p = Projection::default();
        for &(x, y, z) in &[
            (3.5, 70.0, -12.5),
            (-1000.2, 5.0, 2048.9),
            (0.0, -60.0, 0.0),
        ] {
            let a = Anchor::around(&p, x, y, z, 256, 8.0);
            let (tu, tv) = a.texel(p.u(x), p.v(f64::from(a.ground_y), z));
            assert!((112.0..=144.0).contains(&tu), "{tu}");
            assert!((112.0..=144.0).contains(&tv), "{tv}");
        }
    }

    #[test]
    fn image_size_and_alpha() {
        let p = Projection::default();
        let a = Anchor::around(&p, 0.0, 64.0, 0.0, 16, 8.0);
        let img = checkerboard(&p, &a, 16);
        assert_eq!(img.len(), 16 * 16 * 4);
        assert!(img.chunks(4).all(|c| c[3] == 255));
    }
}
