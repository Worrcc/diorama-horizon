//! The painter: Diorama's colours for the column baker's faces. A face's
//! texel is the block's albedo from the loaded packs (looks.rs) under the
//! style's daytime light, with water, plants and glass mixed in, then graded
//! (TileBaker.java `shade`, `mixWater`, `abgr`; RULES.md sections 4-7).
//!
//! Look differences from Diorama, besides those RULES.md section 12 lists
//! (no sun shadows, sky rays or corner AO):
//! - The floor under water is the same column's, straight down; Diorama's ray
//!   meets it a little further north.
//! - A water column deeper than the blocks read has a black floor, as
//!   Diorama's has when the ray finds none (TileBaker.java:1093-1097).
//! - Plants and glass over a column only tint its top face.
//! - Sky visibility is the column's sky light over 15.

use crate::bake::{Chunk, Shade, Texel, CLEAR, NO_TOP};
use crate::looks::{albedo, Kind, Looks};
use crate::projection::Projection;
use crate::style::{luminance, Style, Tables};
use std::cell::Cell;

const FACE_CACHE: usize = 1024;
#[derive(Clone, Copy, PartialEq, Eq)]
struct FaceKey {
    state: u32,
    height: u64,
    tints: [u32; 3],
    light: u8,
    cave: Option<i32>,
    version: u64,
}

/// The water path a south face of water is drawn with (a waterfall's side):
/// the port's own number, deep enough to show the water's colour.
pub const WATER_SIDE_PATH: f32 = 8.0;

/// Shades faces with a set of looks and a style.
pub struct Painter {
    pub looks: Looks,
    /// Read it here; change it with [`Painter::set_style`].
    pub style: Style,
    pub tables: Tables,
    proj: Projection,
    /// The height of the cut when the map shows a cave slice, else none
    /// (the surface view). Faces fade with their depth under the cut.
    pub cave: Option<i32>,
    /// `style.side_lambert()`, kept (it changes only with the style).
    side_lambert: f64,
    /// Exact dry face colours recur across columns and chunks. A bounded
    /// direct cache avoids repeating their albedo, lighting and grade.
    face_cache: Vec<Cell<Option<(FaceKey, u32)>>>,
}

fn mul(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

impl Painter {
    fn top_rgb(&self, chunk: &Chunk, col: usize, state: u32, y: f64) -> u32 {
        let key = (chunk.over_at(col)[3] >= 1.0).then(|| FaceKey {
            state,
            height: y.to_bits(),
            tints: chunk.tints_at(col),
            light: chunk.light.get(col).copied().unwrap_or(0xF0),
            cave: self.cave,
            version: self.looks.version,
        });
        let slot = key.map(|k| {
            let h = (k.height ^ (k.height >> 32)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            ((h >> 54) as usize
                ^ state.wrapping_mul(0x9E37_79B9) as usize
                ^ k.tints[0] as usize
                ^ (k.tints[1] as usize).rotate_left(7)
                ^ (k.light as usize).rotate_left(3))
                & (FACE_CACHE - 1)
        });
        if let (Some(key), Some(slot)) = (key, slot) {
            if let Some((old, rgb)) = self.face_cache[slot].get() {
                if key == old {
                    return rgb;
                }
            }
        }
        let c = self.lit(chunk, col, state, true, y);
        let rgb = self.style.grade(&self.tables, c);
        if let (Some(key), Some(slot)) = (key, slot) {
            self.face_cache[slot].set(Some((key, rgb)));
        }
        rgb
    }

    pub fn new(looks: Looks, style: Style) -> Painter {
        Painter {
            looks,
            proj: Projection::new(style.pitch_degrees),
            style,
            tables: Tables::new(),
            cave: None,
            side_lambert: style.side_lambert(),
            face_cache: vec![Cell::new(None); FACE_CACHE],
        }
    }

    /// Changes the style (use this, not a write to `style`: the side light
    /// is kept with it).
    pub fn set_style(&mut self, style: Style) {
        self.side_lambert = style.side_lambert();
        self.style = style;
        self.face_cache.fill(Cell::new(None));
    }

    /// The states [`Painter::prepare`] passes at a column's top (plants,
    /// glass, air): the host's `set-see-through` set, so it can lower the
    /// tops natively (`prepared-columns`, API level 23).
    pub fn see_through(&self) -> Vec<u32> {
        (0..self.looks.looks.len() as u32)
            .filter(|&s| {
                matches!(
                    self.looks.get(s).kind,
                    Kind::Plant | Kind::Glass | Kind::Air
                )
            })
            .collect()
    }

    /// The rest of [`Painter::prepare`] for a chunk the host already lowered
    /// (`prepared-columns`): what the `skips[col]` passed states, given top
    /// down and concatenated in `skipped`, lay over each column, in
    /// `chunk.over`. Same arithmetic as `prepare`, so the same pixels.
    pub fn overlays(&self, chunk: &mut Chunk, skips: &[u8], skipped: &[u32]) {
        let mut over: Vec<[f32; 4]> = Vec::new();
        let mut at = 0;
        for (col, &k) in skips.iter().enumerate().take(256) {
            let k = usize::from(k);
            if k == 0 {
                continue;
            }
            let Some(states) = skipped.get(at..at + k) else {
                break;
            };
            at += k;
            let tints = chunk.tints_at(col);
            let (mut rgb, mut through) = ([0.0f32; 3], 1.0f32);
            for &state in states {
                let look = self.looks.get(state);
                let Some(w) = look.kind.overlay() else {
                    continue;
                };
                let a = albedo(&self.tables, &look, true, &tints);
                for c in 0..3 {
                    rgb[c] += through * w * a[c];
                }
                through *= 1.0 - w;
            }
            if through < 1.0 {
                if over.is_empty() {
                    over = vec![CLEAR; 256];
                }
                over[col] = [rgb[0], rgb[1], rgb[2], through];
            }
        }
        chunk.over = over;
    }

    /// Lowers each column's top past plants, glass and air to the first block
    /// a ray stops at or enters as water, as Diorama's ray passes them
    /// (TileBaker.java:1049-1059), and keeps what they lay over it in
    /// `chunk.over`. A column with nothing to stop at in the blocks read
    /// keeps its top.
    pub fn prepare(&self, chunk: &mut Chunk) {
        let d = usize::from(chunk.depth.max(1));
        let mut over: Vec<[f32; 4]> = Vec::new();
        for col in 0..256 {
            let Some(&top) = chunk.tops.get(col) else {
                break;
            };
            if top == NO_TOP {
                continue;
            }
            let stop = (0..d).find(|&k| {
                let kind = self.looks.get(chunk.state(col, k as i32)).kind;
                !matches!(kind, Kind::Plant | Kind::Glass | Kind::Air)
            });
            let Some(k) = stop.filter(|&k| k > 0) else {
                continue;
            };
            let tints = chunk.tints_at(col);
            let (mut rgb, mut through) = ([0.0f32; 3], 1.0f32);
            for j in 0..k {
                let look = self.looks.get(chunk.state(col, j as i32));
                let Some(w) = look.kind.overlay() else {
                    continue;
                };
                let a = albedo(&self.tables, &look, true, &tints);
                for c in 0..3 {
                    rgb[c] += through * w * a[c];
                }
                through *= 1.0 - w;
            }
            let row = &mut chunk.states[col * d..(col + 1) * d];
            row.copy_within(k.., 0);
            let last = row[d - k - 1];
            row[d - k..].fill(last);
            chunk.tops[col] = top - k as i16;
            if through < 1.0 {
                if over.is_empty() {
                    over = vec![CLEAR; 256];
                }
                over[col] = [rgb[0], rgb[1], rgb[2], through];
            }
        }
        chunk.over = over;
    }

    /// A lit solid face, in linear light: albedo times the surface light,
    /// then what lies over it (TileBaker.java:1025-1031).
    fn lit(&self, chunk: &Chunk, col: usize, state: u32, top: bool, y: f64) -> [f32; 3] {
        let look = self.looks.get(state);
        let a = albedo(&self.tables, &look, top, &chunk.tints_at(col));
        let light = match self.cave {
            Some(cut) => Style::cave_light(
                chunk.block_light_at(col),
                chunk.sky_level_at(col),
                Style::cave_fade(f64::from(cut) - y),
                look.emission,
                top,
            ),
            None => self.style.surface_light_at(
                self.side_lambert,
                top,
                y,
                chunk.sky_at(col),
                look.emission,
            ),
        };
        let mut out = mul(a, light);
        let over = chunk.over_at(col);
        if top && over[3] < 1.0 {
            let l = luminance(out) / luminance(a).max(0.05);
            for c in 0..3 {
                out[c] = out[c] * over[3] + over[c] * l;
            }
        }
        out
    }

    /// The water's colour over a floor: its albedo at the column, its light
    /// at the top water block `block_y`, through `path` of water.
    fn water(
        &self,
        chunk: &Chunk,
        col: usize,
        state: u32,
        block_y: i32,
        floor: [f32; 3],
        path: f32,
    ) -> [f32; 3] {
        let w = self
            .looks
            .albedo(&self.tables, state, true, &chunk.tints_at(col));
        let light = match self.cave {
            Some(cut) => Style::cave_water_light(
                chunk.block_light_at(col),
                chunk.sky_level_at(col),
                Style::cave_fade(f64::from(cut) - f64::from(block_y)),
            ),
            None => self.style.water_light(block_y, chunk.sky_at(col)),
        };
        self.style.water(floor, w, path, light)
    }
}

impl Shade for Painter {
    fn top(&self, chunk: &Chunk, col: usize, y: f64) -> Texel {
        let s0 = chunk.state(col, 0);
        if self.looks.get(s0).kind != Kind::Water {
            return Texel::dry(self.top_rgb(chunk, col, s0, y), y);
        }
        // Water: count it down to the floor (TileBaker.java:1041-1048).
        let d = i32::from(chunk.depth.max(1));
        let wet = (0..d)
            .find(|&k| self.looks.get(chunk.state(col, k)).kind != Kind::Water)
            .unwrap_or(d);
        let block_y = y as i32 - 1;
        let (floor, deep) = if wet < d {
            let fy = y - f64::from(wet);
            (self.lit(chunk, col, chunk.state(col, wet), true, fy), wet)
        } else {
            ([0.0; 3], wet.max(i32::from(chunk.water_at(col))))
        };
        // The ray falls `sin` per unit of path.
        let path = (f64::from(deep) / self.proj.sin) as f32;
        let c = self.water(chunk, col, s0, block_y, floor, path);
        Texel {
            rgb: self.style.grade(&self.tables, c),
            seen: (y - f64::from(deep)) as f32,
            water: path,
        }
    }

    fn south(&self, chunk: &Chunk, col: usize, block: i32, y: f64) -> Texel {
        let top = chunk.tops.get(col).map_or(block, |&t| i32::from(t));
        // An overhang is drawn as a column down to the ground: where the
        // block is air (under leaves), the lowest block above it stands in.
        let state = (0..=(top - block).max(0))
            .rev()
            .map(|k| chunk.state(col, k))
            .find(|&s| self.looks.get(s).kind != Kind::Air)
            .unwrap_or_else(|| chunk.state(col, 0));
        let c = if self.looks.get(state).kind == Kind::Water {
            self.water(chunk, col, state, top, [0.0; 3], WATER_SIDE_PATH)
        } else {
            self.lit(chunk, col, state, false, y)
        };
        Texel::dry(self.style.grade(&self.tables, c), y)
    }

    fn outline(&self) -> f32 {
        self.style.outline
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::looks::{flags, Average, Face, Table, Tint, NO_SPRITE};
    use crate::style::DEFAULT;

    /// States: 0 air, 1 grass block, 2 stone, 3 water, 4 poppy, 5 glass, 6 sand.
    fn painter() -> Painter {
        let f = |sprite, tint| Face { sprite, tint };
        let table = Table {
            version: 1,
            blocks: [
                "air",
                "grass_block",
                "stone",
                "water",
                "poppy",
                "glass",
                "sand",
            ]
            .map(|b| format!("minecraft:{b}"))
            .to_vec(),
            sprites: [
                "grass_block_top",
                "stone",
                "water_still",
                "poppy",
                "glass",
                "sand",
            ]
            .map(|s| format!("block/{s}"))
            .to_vec(),
            state_block: vec![0, 1, 2, 3, 4, 5, 6],
            state_flags: vec![
                flags::AIR,
                0,
                0,
                flags::WATER,
                flags::NO_COLLISION,
                flags::SEE_THROUGH,
                0,
            ],
            state_top: vec![
                f(NO_SPRITE, Tint::None),
                f(0, Tint::Grass),
                f(1, Tint::None),
                f(2, Tint::Water),
                f(3, Tint::None),
                f(4, Tint::None),
                f(5, Tint::None),
            ],
            state_side: vec![
                f(NO_SPRITE, Tint::None),
                f(1, Tint::None),
                f(1, Tint::None),
                f(2, Tint::Water),
                f(3, Tint::None),
                f(4, Tint::None),
                f(5, Tint::None),
            ],
            constant_tint: vec![0; 7],
            map_color: vec![0; 7],
        };
        let a = |v: [f32; 3], coverage| Some(Average { rgb: v, coverage });
        let avgs = vec![
            a([0.6, 0.6, 0.6], 1.0),
            a([0.5, 0.5, 0.5], 1.0),
            a([0.7, 0.7, 0.7], 1.0),
            a([0.8, 0.1, 0.1], 0.3),
            a([0.9, 0.95, 1.0], 0.1),
            a([0.86, 0.81, 0.64], 1.0),
        ];
        Painter::new(Looks::build(&table, &avgs), DEFAULT)
    }

    fn column(states: &[u32], top: i16) -> Chunk {
        let mut c = Chunk::from_fn(8, |_, _| Some((top, 2)));
        for (k, &s) in states.iter().enumerate() {
            c.states[k] = s;
        }
        c.tints = [0x91_BD59, 0x77_AB2F, 0x3F_76E4].repeat(256);
        c
    }

    #[test]
    fn cached_dry_tops_match_exact_shading_across_light_tints_layers_and_styles() {
        let mut p = painter();
        for style in [crate::style::DEFAULT, crate::style::FLAT] {
            p.set_style(style);
            for cave in [None, Some(72), Some(100)] {
                p.cave = cave;
                for n in 0..512u32 {
                    let state = [1, 2, 6][n as usize % 3];
                    let mut c = column(&[state], (n % 64 + 32) as i16);
                    c.light = vec![(n & 255) as u8; 256];
                    c.tints[0] ^= (n % 7) * 0x01_0101;
                    let y = f64::from(c.tops[0]) + 1.0;
                    let rgb = p.style.grade(&p.tables, p.lit(&c, 0, state, true, y));
                    for _ in 0..2 {
                        assert_eq!(p.top(&c, 0, y), Texel::dry(rgb, y));
                    }
                }
            }
        }
        // Reloads and prepared plant/glass overlays must bypass old colours.
        p.cave = None;
        let mut c = column(&[1], 64);
        let _ = p.top(&c, 0, 65.0);
        p.looks.version += 1;
        p.looks.looks[1].top.base ^= 0xFF_FFFF;
        c.over = vec![[0.1, 0.2, 0.3, 0.5]; 256];
        let rgb = p.style.grade(&p.tables, p.lit(&c, 0, 1, true, 65.0));
        assert_eq!(p.top(&c, 0, 65.0), Texel::dry(rgb, 65.0));
    }

    fn channels(c: u32) -> [i32; 3] {
        [
            (c >> 16 & 0xFF) as i32,
            (c >> 8 & 0xFF) as i32,
            (c & 0xFF) as i32,
        ]
    }

    #[test]
    fn grass_takes_the_biome_colour_and_its_side_is_darker() {
        let p = painter();
        let mut c = column(&[1], 64);
        let plains = p.top(&c, 0, 65.0).rgb;
        let [r, g, b] = channels(plains);
        assert!(g > r && g > b, "{plains:06X}");
        // A desert-coloured grass is yellower.
        c.tints = [0xBF_B755, 0xAE_A42A, 0x3F_76E4].repeat(256);
        let desert = p.top(&c, 0, 65.0).rgb;
        assert_ne!(desert, plains);
        assert!(channels(desert)[0] > r);
        // The side shows stone-grey, less lit.
        let side = p.south(&c, 0, 64, 64.5).rgb;
        let top_stone = p.top(&column(&[2], 64), 0, 65.0).rgb;
        assert!(channels(side)[1] < channels(top_stone)[1]);
    }

    #[test]
    fn the_top_colour_by_hand() {
        // Stone (0.5 grey, untinted) at y 64, open sky: albedo lin(128) =
        // 0.2158605; light (0.9584, 0.9124, 0.8972); then the grade.
        let p = painter();
        let t = Tables::new();
        let lin = t.to_linear[128];
        let want = DEFAULT.grade(&t, [lin * 0.9584, lin * 0.9124, lin * 0.8972]);
        assert_eq!(p.top(&column(&[2], 63), 0, 64.0).rgb, want);
    }

    #[test]
    fn higher_ground_is_brighter() {
        let p = painter();
        let low = p.top(&column(&[2], 40), 0, 41.0).rgb;
        let high = p.top(&column(&[2], 100), 0, 101.0).rgb;
        assert!(channels(high)[1] > channels(low)[1]);
    }

    #[test]
    fn water_is_bluer_and_darker_the_deeper_it_is() {
        let p = painter();
        let shallow = p.top(&column(&[3, 6], 62), 0, 63.0);
        let deep = p.top(&column(&[3, 3, 3, 3, 3, 6], 62), 0, 63.0);
        let sand = p.top(&column(&[6], 62), 0, 63.0);
        assert!(shallow.water > 1.0 && deep.water > shallow.water);
        assert_eq!(shallow.seen, 62.0);
        assert_eq!(deep.seen, 58.0);
        let [sr, _, sb] = channels(shallow.rgb);
        let [dr, dg, db] = channels(deep.rgb);
        assert!(sr < channels(sand.rgb)[0]);
        assert!(dr < sr && db > dr && db > dg, "{:06X}", deep.rgb);
        assert!(dr + dg + db < sr + channels(shallow.rgb)[1] + sb);
        // Deeper than the blocks read: the host's count, over black.
        let mut ocean = column(&[3; 8], 62);
        ocean.water = vec![30; 256];
        let o = p.top(&ocean, 0, 63.0);
        assert!((o.water - (30.0 / Projection::default().sin) as f32).abs() < 1e-4);
        assert!(channels(o.rgb).iter().sum::<i32>() < dr + dg + db);
    }

    #[test]
    fn plants_and_glass_are_seen_through() {
        let p = painter();
        // A poppy on grass at 64: the top drops to the grass, tinted red.
        let mut c = column(&[4, 1], 65);
        p.prepare(&mut c);
        assert_eq!(c.tops[0], 64);
        assert_eq!(c.state(0, 0), 1);
        let o = c.over_at(0);
        assert!((o[3] - 0.6).abs() < 1e-6);
        let grass = p.top(&column(&[1], 64), 0, 65.0).rgb;
        let under_poppy = p.top(&c, 0, 65.0).rgb;
        assert!(channels(under_poppy)[0] > channels(grass)[0]);
        // Glass over air over stone: the stone, a quarter glass.
        let mut g = column(&[5, 0, 0, 2], 70);
        p.prepare(&mut g);
        assert_eq!((g.tops[0], g.state(0, 0)), (67, 2));
        assert!((g.over_at(0)[3] - 0.75).abs() < 1e-6);
        // Columns with nothing to stop at keep their top; others are untouched.
        let mut all = column(&[5; 8], 70);
        p.prepare(&mut all);
        assert_eq!(all.tops[0], 70);
        assert_eq!(c.tops[1], 65);
        assert!(all.over.is_empty());
    }

    /// What the host's `prepared-columns` does, from its documented rule.
    #[allow(clippy::needless_range_loop)]
    fn host_prepare(c: &mut Chunk, see: &[u32]) -> (Vec<u8>, Vec<u32>) {
        let d = usize::from(c.depth.max(1));
        let (mut skips, mut skipped) = (vec![0u8; 256], Vec::new());
        for col in 0..256 {
            if c.tops[col] == NO_TOP {
                continue;
            }
            let Some(k) = (0..d).find(|&k| !see.contains(&c.state(col, k as i32))) else {
                continue;
            };
            if k == 0 {
                continue;
            }
            let row = &mut c.states[col * d..(col + 1) * d];
            skipped.extend_from_slice(&row[..k]);
            row.copy_within(k.., 0);
            let last = row[d - k - 1];
            row[d - k..].fill(last);
            c.tops[col] -= k as i16;
            skips[col] = k as u8;
        }
        (skips, skipped)
    }

    #[test]
    fn host_prepared_columns_paint_as_prepare_does() {
        let p = painter();
        let see = p.see_through();
        for (states, top) in [
            (&[4, 1][..], 65),
            (&[5, 0, 0, 2][..], 70),
            (&[5; 8][..], 70),
            (&[1][..], 64),
            (&[4, 5, 4, 0, 3][..], 80),
        ] {
            let mut a = column(states, top);
            // Column 1: a plant over glass over grass, to check that the
            // skipped states of several columns line up.
            a.states[8..11].copy_from_slice(&[4, 5, 1]);
            let mut b = a.clone();
            p.prepare(&mut a);
            let (skips, skipped) = host_prepare(&mut b, &see);
            p.overlays(&mut b, &skips, &skipped);
            assert_eq!(a.tops, b.tops, "{states:?}");
            assert_eq!(a.states, b.states, "{states:?}");
            assert_eq!(a.over, b.over, "{states:?}");
        }
    }

    #[test]
    fn an_overhangs_face_over_air_takes_the_block_above() {
        // Leaves (stone here) at 70 over air: the face of the air below is
        // drawn as the block above it, not black.
        let p = painter();
        let c = column(&[2, 0, 0, 0], 70);
        assert_eq!(p.south(&c, 0, 68, 68.5), p.south(&c, 0, 70, 68.5));
        assert_ne!(p.south(&c, 0, 68, 68.5).rgb, 0);
    }

    #[test]
    fn a_cave_slice_is_lit_by_block_light_and_fades_with_depth() {
        let mut p = painter();
        let mut c = column(&[2], 40);
        let day = p.top(&c, 0, 41.0).rgb;
        p.cave = Some(48);
        // Unlit rock a few blocks under the cut: the dim base light.
        let dim = p.top(&c, 0, 41.0).rgb;
        assert!(channels(dim)[1] < channels(day)[1]);
        // A torch beside it lights it up.
        c.light = vec![0x0F; 256];
        let lit = p.top(&c, 0, 41.0).rgb;
        assert!(channels(lit)[0] > channels(dim)[0]);
        // Far under the cut it fades.
        c.light = Vec::new();
        let deep = p.top(&c, 0, 41.0).rgb;
        p.cave = Some(80);
        assert!(channels(p.top(&c, 0, 41.0).rgb)[1] < channels(deep)[1]);
    }
}
