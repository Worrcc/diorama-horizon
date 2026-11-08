//! The map style: Diorama's light, colour grade, water and outline numbers
//! (RULES.md sections 2, 4-7; `MapStyle.java`, `bake/Lighting.java`,
//! `bake/TileBaker.java`). Only the numbers and formulas are taken; the code
//! is the port's own.
//!
//! Everything here works in linear light, as Diorama's baker does, and turns
//! into 8-bit sRGB only in [`Style::grade`].

/// A style's numbers (`MapStyle.java:27-44`). Fields the port does not use
/// (fog shape, block variation, corner occlusion: no corner AO in v1) are
/// left out; RULES.md section 12 lists them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub pitch_degrees: f64,
    pub saturation: f32,
    pub exposure: f32,
    pub sun_tint: [f32; 3],
    pub shadow_tint: [f32; 3],
    pub outline: f32,
    pub clear_water: bool,
    pub sky_occlusion: f32,
    pub sun_elevation: f64,
    pub ambient: f32,
    pub sun_strength: f32,
    pub height_shade: f32,
}

/// `MapStyle.FLAT`: `BASELINE` (MapStyle.java:48-50) with outline 0.38
/// (:53). Pitch 60, saturation 1.18, exposure 1.06, sun tint (1.06, 1, 0.9),
/// shadow tint (0.9, 0.95, 1.08), sun elevation 51.3, ambient 0.5, sun 0.484.
pub const FLAT: Style = Style {
    pitch_degrees: 60.0,
    saturation: 1.18,
    exposure: 1.06,
    sun_tint: [1.06, 1.0, 0.9],
    shadow_tint: [0.9, 0.95, 1.08],
    outline: 0.38,
    clear_water: false,
    sky_occlusion: 0.0,
    sun_elevation: 51.3,
    ambient: 0.5,
    sun_strength: 0.484,
    height_shade: 0.0,
};

/// `MapStyle.DEFAULT` (MapStyle.java:61-69): FLAT with pitch 50, grade
/// 1.35 / 1.16, sun tint (1.14, 1, 0.8), shadow tint (0.74, 0.86, 1.18), sky
/// occlusion 0.85, sun elevation 38, ambient 0.34, sun 0.62, height shade
/// 0.22 and clear water.
pub const DEFAULT: Style = Style {
    pitch_degrees: 50.0,
    saturation: 1.35,
    exposure: 1.16,
    sun_tint: [1.14, 1.0, 0.8],
    shadow_tint: [0.74, 0.86, 1.18],
    sky_occlusion: 0.85,
    sun_elevation: 38.0,
    ambient: 0.34,
    sun_strength: 0.62,
    height_shade: 0.22,
    clear_water: true,
    ..FLAT
};

/// The sun's azimuth in (x, z): west, a little south (TileBaker.java:47-48).
pub const SUN_AZIMUTH: (f64, f64) = (-0.878, 0.479);
/// A side face's light factor (`SIDE`, TileBaker.java:80).
pub const SIDE: f32 = 0.82;
/// An outline needs a neighbour this much nearer, in blocks along the view
/// (`OUTLINE_DEPTH`, TileBaker.java:84).
pub const OUTLINE_DEPTH: f32 = 1.5;
/// Outlines under water fade to a third over this much water
/// (`OUTLINE_WATER_FADE`, TileBaker.java:82).
pub const OUTLINE_WATER_FADE: f32 = 6.0;
/// Cave slices: outline threshold, the depth fade and the light's soft knee
/// (TileBaker.java:85, :91, :93; Lighting.java:13, :20, :118-123).
pub const CAVE_OUTLINE_DEPTH: f32 = 3.0;
pub const CAVE_FADE: f32 = 12.0;
pub const CAVE_FADE_START: f32 = 6.0;
const CAVE_KNEE: f32 = 0.5;
const CAVE_CEILING: f32 = 0.8;
const CAVE_BASE: f32 = 0.42;
/// What plants and glass let through, and lay over what is under them
/// (TileBaker.java:1052).
pub const PLANT_WEIGHT: f32 = 0.4;
pub const GLASS_WEIGHT: f32 = 0.25;

/// `0.2126 r + 0.7152 g + 0.0722 b` (TileBaker.java:1539-1541).
pub fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// The sRGB <-> linear tables (TileBaker.java:108-121): 256 entries one way,
/// 4096 the other, rounded to 8 bits.
pub struct Tables {
    pub to_linear: [f32; 256],
    pub to_srgb: [u8; 4096],
}

impl Tables {
    pub fn new() -> Tables {
        let mut to_linear = [0.0; 256];
        for (i, slot) in to_linear.iter_mut().enumerate() {
            let v = i as f64 / 255.0;
            *slot = if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            } as f32;
        }
        let mut to_srgb = [0; 4096];
        for (i, slot) in to_srgb.iter_mut().enumerate() {
            let v = i as f64 / 4095.0;
            let s = if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            };
            *slot = (s.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        Tables { to_linear, to_srgb }
    }
    /// 0xRRGGBB to linear light.
    pub fn linear(&self, rgb: u32) -> [f32; 3] {
        [
            self.to_linear[(rgb >> 16 & 0xFF) as usize],
            self.to_linear[(rgb >> 8 & 0xFF) as usize],
            self.to_linear[(rgb & 0xFF) as usize],
        ]
    }
}

impl Default for Tables {
    fn default() -> Self {
        Tables::new()
    }
}

impl Style {
    /// The sun direction `(x, y, z)` (TileBaker.java:247-249).
    pub fn sun(&self) -> (f64, f64, f64) {
        let e = self.sun_elevation.to_radians();
        (e.cos() * SUN_AZIMUTH.0, e.sin(), e.cos() * SUN_AZIMUTH.1)
    }

    /// The height shade factor at `y`: `1 + heightShade * clamp((y-64)/64)`
    /// (Lighting.java:66-75; water at `blockY + 1`, TileBaker.java:1289).
    pub fn height_factor(&self, y: f64) -> f32 {
        1.0 + self.height_shade * ((y - 64.0) / 64.0).clamp(-1.0, 1.0) as f32
    }

    /// The light on a daytime surface face (Lighting.java:45-92), for the
    /// port: no corner AO (ao 1) and no sun shadows, and `sky` a stand-in for
    /// Diorama's sky visibility rays (the column's sky light / 15). `top` for
    /// a top face, else a south face; `y` the hit height; `emission` 0..1.
    pub fn surface_light(&self, top: bool, y: f64, sky: f32, emission: f32) -> [f32; 3] {
        self.surface_light_at(self.side_lambert(), top, y, sky, emission)
    }

    /// A south face's sunlight relative to a top face's (TileBaker.java:1223),
    /// for [`Style::surface_light_at`]: worked out once, not per texel (the
    /// sine and cosine are dear in WebAssembly, more so on Pulley).
    pub fn side_lambert(&self) -> f64 {
        let (_, sy, sz) = self.sun();
        sz.max(0.0) / sy
    }

    /// [`Style::surface_light`] with the side face's [`Style::side_lambert`]
    /// given.
    pub fn surface_light_at(
        &self,
        side_lambert: f64,
        top: bool,
        y: f64,
        sky: f32,
        emission: f32,
    ) -> [f32; 3] {
        let lambert = if top { 1.0 } else { side_lambert };
        let direct = (f64::from(self.sun_strength) * lambert) as f32;
        let mut ambient = self.ambient;
        if self.sky_occlusion > 0.0 {
            ambient *= 1.0 - self.sky_occlusion * (1.0 - (0.15 + 0.85 * sky));
        }
        let f = self.height_factor(y);
        let mut light = [0.0; 3];
        for (c, l) in light.iter_mut().enumerate() {
            *l = (ambient * self.shadow_tint[c] + direct * self.sun_tint[c]) * f;
            if emission > 0.0 {
                *l = l.max(0.35 + emission * 0.85);
            }
            if !top {
                *l *= SIDE;
            }
        }
        light
    }

    /// How much a cave slice fades a face `below` blocks under the cut:
    /// full for the first [`CAVE_FADE_START`], then `exp(-d / CAVE_FADE)`
    /// (TileBaker.java:560-562).
    pub fn cave_fade(below: f64) -> f32 {
        ((CAVE_FADE_START - below.max(0.0) as f32).min(0.0) / CAVE_FADE).exp()
    }

    /// The light on a face in a cave slice (Lighting.java:106-133, no corner
    /// AO): `block` and `sky` light 0..15 in front of the face, `fade` from
    /// [`Style::cave_fade`], `emission` 0..1, `top` for a top face.
    pub fn cave_light(block: f32, sky: f32, fade: f32, emission: f32, top: bool) -> [f32; 3] {
        let warm = (block / 15.0).powf(1.5);
        let cool = (sky / 15.0 * 0.8).powf(1.5);
        let mut l = [
            (CAVE_BASE * 0.92 + warm * 0.9 + cool * 0.6) * fade,
            (CAVE_BASE * 0.96 + warm * 0.75 + cool * 0.65) * fade,
            (CAVE_BASE * 1.08 + warm * 0.5 + cool * 0.75) * fade,
        ];
        let peak = l[0].max(l[1]).max(l[2]);
        if peak > CAVE_KNEE {
            let range = CAVE_CEILING - CAVE_KNEE;
            let eased = CAVE_KNEE + range * (1.0 - (-(peak - CAVE_KNEE) / range).exp());
            for c in &mut l {
                *c *= eased / peak;
            }
        }
        for c in &mut l {
            if emission > 0.0 {
                *c = c.max(0.35 + emission * 0.85);
            }
            if !top {
                *c *= SIDE;
            }
        }
        l
    }

    /// The light on water in a cave slice (Lighting.java:102-104).
    pub fn cave_water_light(block: f32, sky: f32, fade: f32) -> f32 {
        (CAVE_BASE + (block.max(sky * 0.8) / 15.0).powf(1.5) * 0.8) * fade
    }

    /// The light on a water surface whose top water block is at `block_y`
    /// (TileBaker.java:1280-1290): ambient (less with less sky) plus the sun,
    /// times the height shade at `block_y + 1`.
    pub fn water_light(&self, block_y: i32, sky: f32) -> f32 {
        let mut ambient = self.ambient;
        if self.sky_occlusion > 0.0 {
            ambient *= 1.0 - self.sky_occlusion * (1.0 - (0.15 + 0.85 * sky));
        }
        (ambient + self.sun_strength) * self.height_factor(f64::from(block_y + 1))
    }

    /// Water over a floor (TileBaker.java:1292-1316): `floor` the lit floor
    /// colour, `water` the water's albedo (linear, tinted), `path` the ray's
    /// length through the water, `light` from [`Style::water_light`].
    pub fn water(&self, floor: [f32; 3], water: [f32; 3], path: f32, light: f32) -> [f32; 3] {
        let depth = path.max(0.0);
        let mut out = [0.0; 3];
        if self.clear_water {
            let peak = water[0].max(water[1]).max(water[2]).max(1e-4);
            let deep = light * (0.3 + 0.7 * (-depth / 18.0).exp());
            let l = luminance(water);
            for c in 0..3 {
                let t = (-depth * (0.45 * (1.0 - water[c] / peak) + 0.16)).exp();
                let deep_c = (l + (water[c] - l) * 1.2).max(0.0) * deep;
                out[c] = floor[c] * t + deep_c * (1.0 - t);
            }
        } else {
            let a = (1.0 - (-0.2 * depth).exp()).clamp(0.5, 0.94);
            for c in 0..3 {
                out[c] = floor[c] * (1.0 - a) + water[c] * light * a;
            }
        }
        out
    }

    /// Linear light to 0xRRGGBB through the grade (TileBaker.java:1556-1568):
    /// saturation about the luminance, exposure, then the sRGB table.
    pub fn grade(&self, t: &Tables, c: [f32; 3]) -> u32 {
        let l = luminance(c);
        let ch = |v: f32| {
            let g = (l + (v - l) * self.saturation) * self.exposure;
            // Java's (int) cast truncates toward zero, then clamps.
            let i = ((g * 4095.0) as i32).clamp(0, 4095);
            u32::from(t.to_srgb[i as usize])
        };
        (ch(c[0]) << 16) | (ch(c[1]) << 8) | ch(c[2])
    }
}

/// An outline's darkening of 0xRRGGBB: each channel times `keep`, rounded
/// (TileBaker.java `darken`; SurfaceArea.java:78-79).
pub fn darken(rgb: u32, keep: f32) -> u32 {
    let ch = |s: u32| ((((rgb >> s) & 0xFF) as f32 * keep).round() as u32).min(255);
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// How much an outline darkens a pixel `water` blocks of water under the
/// surface: `1 - outline * exp(-water / 6)` (TileBaker.java:833-834).
pub fn outline_keep(outline: f32, water: f32) -> f32 {
    1.0 - outline * (-water / OUTLINE_WATER_FADE).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn the_tables_round_trip_and_match_known_points() {
        let t = Tables::new();
        assert_eq!(t.to_linear[0], 0.0);
        assert_eq!(t.to_linear[255], 1.0);
        // sRGB 128 is 0.2158605 in linear light.
        assert!(close(t.to_linear[128], 0.215_860_5, 1e-6));
        assert_eq!(t.to_srgb[0], 0);
        assert_eq!(t.to_srgb[4095], 255);
        for i in 0..=255u32 {
            let lin = t.to_linear[i as usize];
            let back = t.to_srgb[((lin * 4095.0) as usize).min(4095)];
            assert!((i as i32 - back as i32).abs() <= 1, "{i} -> {back}");
        }
    }

    #[test]
    fn default_top_light_by_hand() {
        // Lighting.surface with lambert 1, open sky, at y 64 (no height shade):
        // r = 0.34*0.74 + 0.62*1.14 = 0.9584, g = 0.34*0.86 + 0.62 = 0.9124,
        // b = 0.34*1.18 + 0.62*0.8 = 0.8972.
        let l = DEFAULT.surface_light(true, 64.0, 1.0, 0.0);
        assert!(
            close(l[0], 0.9584, 1e-5) && close(l[1], 0.9124, 1e-5) && close(l[2], 0.8972, 1e-5)
        );
        // At y 128 the height shade adds 22 %; at 0 or below takes 22 %.
        let hi = DEFAULT.surface_light(true, 200.0, 1.0, 0.0);
        assert!(close(hi[1], 0.9124 * 1.22, 1e-5));
        let lo = DEFAULT.surface_light(true, -10.0, 1.0, 0.0);
        assert!(close(lo[1], 0.9124 * 0.78, 1e-5));
    }

    #[test]
    fn a_south_face_gets_the_side_factor_and_less_sun() {
        // Sun at 38 degrees: sz = cos 38 * 0.479, sy = sin 38; lambert = sz/sy.
        let e = 38f64.to_radians();
        let lambert = (e.cos() * 0.479 / e.sin()) as f32;
        let g = (0.34 * 0.86 + 0.62 * lambert) * SIDE;
        let l = DEFAULT.surface_light(false, 64.0, 1.0, 0.0);
        assert!(close(l[1], g, 1e-5), "{} vs {g}", l[1]);
    }

    #[test]
    fn less_sky_less_ambient_and_glow_lifts_light() {
        // sky 0: ambient * (1 - 0.85 * 0.85).
        let l = DEFAULT.surface_light(true, 64.0, 0.0, 0.0);
        let g = 0.34 * (1.0 - 0.85 * 0.85) * 0.86 + 0.62;
        assert!(close(l[1], g, 1e-5));
        // A full glow raises every channel to at least 1.2.
        let glow = DEFAULT.surface_light(true, 64.0, 0.0, 1.0);
        assert!(glow.iter().all(|&c| c >= 1.2 - 1e-6));
    }

    #[test]
    fn the_grade_saturates_and_brightens() {
        let t = Tables::new();
        // Grey stays grey: l + 0 * 1.35, times 1.16. Linear 0.2 * 1.16 = 0.232
        // -> index 950 -> sRGB 0.5193 -> 132.
        let g = DEFAULT.grade(&t, [0.2, 0.2, 0.2]);
        assert_eq!(g, 0x84_8484, "{g:06X}");
        // Black stays black, white clips.
        assert_eq!(DEFAULT.grade(&t, [0.0; 3]), 0);
        assert_eq!(DEFAULT.grade(&t, [1.0; 3]), 0xFF_FFFF);
        // A colour moves away from its grey.
        let c = DEFAULT.grade(&t, [0.3, 0.2, 0.1]);
        let plain = FLAT.grade(&t, [0.3, 0.2, 0.1]);
        let spread = |c: u32| (c >> 16 & 0xFF) as i32 - (c & 0xFF) as i32;
        assert!(spread(c) > spread(plain));
    }

    #[test]
    fn clear_water_shows_the_floor_when_shallow_and_hides_it_when_deep() {
        let floor = [0.5, 0.4, 0.2];
        let water = [0.05, 0.18, 0.78];
        let light = DEFAULT.water_light(62, 1.0);
        let none = DEFAULT.water(floor, water, 0.0, light);
        assert_eq!(none, floor);
        let shallow = DEFAULT.water(floor, water, 1.0, light);
        let deep = DEFAULT.water(floor, water, 30.0, light);
        assert!(shallow[0] < floor[0]);
        // Deep water is the deep colour, darker the deeper.
        let deeper = DEFAULT.water(floor, water, 60.0, light);
        assert!(deeper[2] < deep[2]);
        assert!(deep[2] > deep[0] && deep[2] > deep[1]);
        // By hand, blue at path 1: t = exp(-(0.45*0 + 0.16)) = 0.85214;
        // deep = light * (0.3 + 0.7 exp(-1/18)); l = lum(water).
        let t = (-0.16f32).exp();
        let l = luminance(water);
        let d = light * (0.3 + 0.7 * (-1.0f32 / 18.0).exp());
        let b = floor[2] * t + (l + (water[2] - l) * 1.2) * d * (1.0 - t);
        assert!(close(shallow[2], b, 1e-6));
    }

    #[test]
    fn murky_water_washes_between_half_and_most() {
        let w = FLAT.water([1.0; 3], [0.0; 3], 0.0, 1.0);
        assert!(close(w[0], 0.5, 1e-6)); // a is at least 0.5
        let w = FLAT.water([1.0; 3], [0.0; 3], 100.0, 1.0);
        assert!(close(w[0], 0.06, 1e-6)); // at most 0.94
    }

    #[test]
    fn outlines_darken_less_under_water() {
        assert!(close(outline_keep(0.38, 0.0), 0.62, 1e-6));
        assert!(close(
            outline_keep(0.38, 6.0),
            1.0 - 0.38 * (-1f32).exp(),
            1e-6
        ));
        assert_eq!(darken(0xFF_8040, 0.5), 0x80_4020);
        assert_eq!(darken(0x64_6464, 0.62), 0x3E_3E3E); // 62.0
    }

    #[test]
    fn cave_fade_starts_after_six_blocks() {
        assert_eq!(Style::cave_fade(0.0), 1.0);
        assert_eq!(Style::cave_fade(6.0), 1.0);
        assert!((Style::cave_fade(18.0) - (-1.0f32).exp()).abs() < 1e-6);
        // Above the cut (a negative depth) is not brighter.
        assert_eq!(Style::cave_fade(-3.0), 1.0);
    }

    #[test]
    fn cave_light_by_hand() {
        // No light at all: the base 0.42 times its tint, a side 0.82 of that.
        let dark = Style::cave_light(0.0, 0.0, 1.0, 0.0, true);
        assert!((dark[0] - 0.42 * 0.92).abs() < 1e-6);
        assert!((dark[2] - 0.42 * 1.08).abs() < 1e-6);
        let side = Style::cave_light(0.0, 0.0, 1.0, 0.0, false);
        assert!((side[1] - 0.42 * 0.96 * 0.82).abs() < 1e-6);
        // Torch light is warm and eases under the 0.8 ceiling.
        let torch = Style::cave_light(15.0, 0.0, 1.0, 0.0, true);
        assert!(torch[0] > torch[2] && torch[0] > dark[0]);
        assert!(torch.iter().all(|&c| c <= 0.8 + 1e-6));
        // Emission lifts a channel to 0.35 + 0.85 e at least.
        let glow = Style::cave_light(0.0, 0.0, 0.2, 1.0, true);
        assert!(glow.iter().all(|&c| (c - 1.2).abs() < 1e-6));
        // Depth dims it.
        assert!(Style::cave_light(0.0, 0.0, 0.3, 0.0, true)[1] < dark[1]);
    }
}
