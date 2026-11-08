//! The column baker's geometry: which faces a column shows and where they land
//! on the map (PLAN-v2 section 3.3, `bake/TileBaker.java` rewritten).
//!
//! Diorama marches one ray per pixel north and down (`y -= sin`, `z -= cos` per
//! unit t, TileBaker.java:1013-1014) and stops at the first top or south face it
//! enters (:1064-1074; RULES.md section 2). The port paints whole columns instead:
//! every column of a chunk is a solid from its top block down, so the only faces a
//! ray can enter are its top face and the part of its south face above the south
//! neighbour's top. Each face is painted as a span of texel rows, and a height
//! buffer keeps the nearest hit per texel.
//!
//! Depth: along the ray through one texel (fixed map row `v`), the parameter is
//! `t = -(v*cot + y/sin)` plus a constant, so a nearer hit is simply a higher one.
//! The buffer holds the hit's height `y` and a higher `y` wins. That is Diorama's
//! "first hit along t", exact for a world made of columns.
//!
//! Look difference (PLAN-v2 risk 3): an overhang (leaves over air, a bridge) is
//! drawn as a column down to the ground: the gap under it is not seen.

use crate::projection::Projection;

/// `client-world` `tops` for a column with no block (air all the way down).
pub const NO_TOP: i16 = i16::MIN;
/// A south face's light factor (`SIDE`, TileBaker.java:80; RULES.md section 2).
pub const SIDE_LIGHT: f32 = 0.82;
/// The port's cap on how many blocks of one south face are painted (a cliff
/// over the void stops here). Not a Diorama number.
pub const MAX_FACE: i32 = 48;
/// Image texels per block along `u`, and per map unit along `v`.
pub const TEXELS_PER_BLOCK: u32 = 2;

/// One chunk's columns as `client-world.columns` returns them: 256 columns,
/// x fastest (column `z * 16 + x`), each with `depth` states from its top down,
/// and per column the water depth from the top, the light above the top
/// (sky << 4 | block) and the biome colours (grass, foliage, water).
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub tops: Vec<i16>,
    pub states: Vec<u32>,
    pub depth: u8,
    /// Empty: no water anywhere.
    pub water: Vec<u8>,
    /// Empty: full sky light.
    pub light: Vec<u8>,
    /// 3 per column; empty: white (no biome colour).
    pub tints: Vec<u32>,
    /// What plants and glass above the top lay over it, per column (linear
    /// r, g, b and the share let through); empty: none anywhere.
    pub over: Vec<[f32; 4]>,
}

/// [`Chunk::over`] for a column with nothing over it.
pub const CLEAR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

impl Chunk {
    /// A chunk of these tops and states, with no water, full sky light and
    /// no biome colours.
    pub fn new(tops: Vec<i16>, states: Vec<u32>, depth: u8) -> Chunk {
        Chunk {
            tops,
            states,
            depth: depth.max(1),
            water: Vec::new(),
            light: Vec::new(),
            tints: Vec::new(),
            over: Vec::new(),
        }
    }
    /// A chunk from a function of the local column `(x, z)`: its top block's y
    /// and the state of every block in it (none: an empty column).
    pub fn from_fn(depth: u8, f: impl Fn(usize, usize) -> Option<(i16, u32)>) -> Chunk {
        let d = usize::from(depth.max(1));
        let mut tops = vec![NO_TOP; 256];
        let mut states = vec![0; 256 * d];
        for z in 0..16 {
            for x in 0..16 {
                if let Some((top, state)) = f(x, z) {
                    let i = z * 16 + x;
                    tops[i] = top;
                    states[i * d..(i + 1) * d].fill(state);
                }
            }
        }
        Chunk::new(tops, states, depth)
    }
    /// The top block's y of local column `(x, z)`, none for an empty column.
    pub fn top(&self, x: usize, z: usize) -> Option<i32> {
        match self.tops.get(z * 16 + x) {
            Some(&t) if t != NO_TOP => Some(i32::from(t)),
            _ => None,
        }
    }
    /// The state `k` blocks below the top of column `i` (0: the top block).
    /// Below the depth read, the deepest one read stands in.
    pub fn state(&self, i: usize, k: i32) -> u32 {
        let d = usize::from(self.depth.max(1));
        let k = (k.max(0) as usize).min(d - 1);
        self.states.get(i * d + k).copied().unwrap_or(0)
    }
    /// Column `i`'s water depth from its top, as the host counted it.
    pub fn water_at(&self, i: usize) -> u8 {
        self.water.get(i).copied().unwrap_or(0)
    }
    /// Column `i`'s sky light above its top, 0..1.
    pub fn sky_at(&self, i: usize) -> f32 {
        self.light.get(i).map_or(1.0, |l| f32::from(l >> 4) / 15.0)
    }
    /// Column `i`'s block light above its top, 0..15.
    pub fn block_light_at(&self, i: usize) -> f32 {
        self.light.get(i).map_or(0.0, |l| f32::from(l & 15))
    }
    /// Column `i`'s sky light above its top, 0..15.
    pub fn sky_level_at(&self, i: usize) -> f32 {
        self.light.get(i).map_or(15.0, |l| f32::from(l >> 4))
    }
    /// Column `i`'s grass, foliage and water colours.
    pub fn tints_at(&self, i: usize) -> [u32; 3] {
        match self.tints.get(i * 3..i * 3 + 3) {
            Some(t) => [t[0], t[1], t[2]],
            None => [0xFF_FFFF; 3],
        }
    }
    /// What lies over column `i`'s top.
    pub fn over_at(&self, i: usize) -> [f32; 4] {
        self.over.get(i).copied().unwrap_or(CLEAR)
    }
}

/// The faces one column shows: its top face at height `top + 1`, and its south
/// face covering blocks `south_from..=top` (none when `south_from > top`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Faces {
    pub top: i32,
    pub south_from: i32,
}

impl Faces {
    /// How many blocks of south face show.
    pub fn south_blocks(&self) -> i32 {
        (self.top + 1 - self.south_from).max(0)
    }
}

/// The faces of a column with top block `top`, given its south neighbour:
/// `None` when that column isn't loaded (no face: the map's edge),
/// `Some(None)` when it is empty, `Some(Some(t))` its top block.
///
/// The south face shows above the neighbour's top (a ray through the face
/// below it would have entered the neighbour first), capped at [`MAX_FACE`].
pub fn faces(top: i32, south: Option<Option<i32>>) -> Faces {
    let floor = top + 1 - MAX_FACE;
    let south_from = match south {
        None => top + 1,
        Some(None) => floor,
        Some(Some(t)) => (t + 1).max(floor),
    };
    Faces { top, south_from }
}

/// Map row spans of a column's faces, in map units: the column's world z is `z`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spans {
    /// The top face: `[top.0, top.1)`.
    pub top: (f64, f64),
    /// The south face: `[south.0, south.1)` (empty when `south.0 >= south.1`).
    pub south: (f64, f64),
}

/// Where a column's faces land on the map (RULES.md section 1:
/// `v = z*sin - y*cos`). The top face lies at `y = top + 1` from `z` to `z + 1`;
/// the south face stands at `z + 1` from `y = top + 1` down to `south_from`.
pub fn spans(p: &Projection, z: i32, f: Faces) -> Spans {
    let zf = f64::from(z);
    let lid = f64::from(f.top + 1);
    let a = p.v(lid, zf);
    let b = p.v(lid, zf + 1.0);
    let c = p.v(f64::from(f.south_from.min(f.top + 1)), zf + 1.0);
    Spans {
        top: (a, b),
        south: (b, c),
    }
}

/// The texel rows whose centres fall in `[a, b)` of the map, for an image whose
/// row 0 starts at map row `v0`: `first..end` (may be empty or out of the image).
pub fn rows(a: f64, b: f64, v0: f64) -> (i64, i64) {
    let t = f64::from(TEXELS_PER_BLOCK);
    let first = ((a - v0) * t - 0.5).ceil() as i64;
    let end = ((b - v0) * t - 0.5).ceil() as i64;
    (first, end.max(first))
}

/// The map row at the centre of texel row `j` of an image starting at `v0`.
pub fn row_v(j: i64, v0: f64) -> f64 {
    v0 + (j as f64 + 0.5) / f64::from(TEXELS_PER_BLOCK)
}

/// The height where the ray through map row `v` meets the plane `z = zf`
/// (a south face standing there): `y = (z*sin - v) / cos`.
pub fn y_on_south(p: &Projection, v: f64, zf: f64) -> f64 {
    (zf * p.sin - v) / p.cos
}

/// What a face paints into one texel: its colour (0xRRGGBB, graded), the
/// height of what is seen there for outlines (the floor under water) and the
/// ray's path through water in front of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Texel {
    pub rgb: u32,
    pub seen: f32,
    pub water: f32,
}

impl Texel {
    /// A dry texel seen at the face's own height.
    pub fn dry(rgb: u32, y: f64) -> Texel {
        Texel {
            rgb,
            seen: y as f32,
            water: 0.0,
        }
    }
}

/// How faces are coloured: the top face of column `col` (its state
/// `chunk.state(col, 0)`), hit at height `y`, and the south face of its
/// block at height `block`, hit at height `y`.
pub trait Shade {
    fn top(&self, chunk: &Chunk, col: usize, y: f64) -> Texel;
    fn south(&self, chunk: &Chunk, col: usize, block: i32, y: f64) -> Texel;
    /// How much outlines darken (`MapStyle.outline`), 0 for none.
    fn outline(&self) -> f32 {
        0.0
    }
}

/// A colour times a light factor, per channel, rounded and clamped.
pub fn scale_rgb(c: u32, f: f32) -> u32 {
    let ch = |s: u32| (((c >> s) & 0xFF) as f32 * f + 0.5).clamp(0.0, 255.0) as u32;
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// M3's placeholder look, kept as the fallback before the block looks are
/// built: each state's map colour as the host reports it (computed by the
/// game, nothing copied), grey when it has none; south faces darker by
/// [`SIDE_LIGHT`]; no outlines.
#[derive(Clone, Debug, Default)]
pub struct MapColours {
    pub colours: Vec<u32>,
}

/// The colour of a state with no map colour.
pub const NO_COLOUR: u32 = 0x80_8080;

impl MapColours {
    pub fn of(&self, state: u32) -> u32 {
        match self.colours.get(state as usize) {
            Some(&c) if c != 0 => c & 0xFF_FFFF,
            _ => NO_COLOUR,
        }
    }
}

impl Shade for MapColours {
    fn top(&self, chunk: &Chunk, col: usize, y: f64) -> Texel {
        Texel::dry(self.of(chunk.state(col, 0)), y)
    }
    fn south(&self, chunk: &Chunk, col: usize, block: i32, y: f64) -> Texel {
        let top = chunk.tops.get(col).map_or(block, |&t| i32::from(t));
        let state = chunk.state(col, top - block);
        Texel::dry(scale_rgb(self.of(state), SIDE_LIGHT), y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> Projection {
        Projection::default()
    }

    #[test]
    fn a_step_shows_one_block_of_south_face() {
        // Column at 65 with its south neighbour at 64.
        let f = faces(65, Some(Some(64)));
        assert_eq!(f.south_from, 65);
        assert_eq!(f.south_blocks(), 1);
        // Level ground: no south face. Higher neighbour: none either.
        assert_eq!(faces(64, Some(Some(64))).south_blocks(), 0);
        assert_eq!(faces(64, Some(Some(70))).south_blocks(), 0);
    }

    #[test]
    fn a_tower_shows_its_whole_height_above_the_ground() {
        let f = faces(70, Some(Some(64)));
        assert_eq!(f.south_blocks(), 6);
        // The face meets the neighbour's top face with no gap or overlap.
        let s = spans(&p(), 10, f);
        let ground = spans(&p(), 11, faces(64, Some(Some(64))));
        assert!((s.south.1 - ground.top.0).abs() < 1e-12);
        // And the top face meets the south face.
        assert!((s.top.1 - s.south.0).abs() < 1e-12);
    }

    #[test]
    fn the_tower_top_is_drawn_north_and_up_of_its_base() {
        // RULES.md section 1: higher blocks go up the map, by cos per block.
        let pr = p();
        let tower = spans(&pr, 10, faces(70, Some(Some(64))));
        let ground_here = spans(&pr, 10, faces(64, Some(Some(64))));
        let up = ground_here.top.0 - tower.top.0;
        assert!((up - 6.0 * pr.cos).abs() < 1e-9, "{up}");
    }

    #[test]
    fn an_overhang_is_drawn_as_a_column() {
        // Leaves at 72 over air, ground at 64 to the south: the face runs from
        // the leaves down to the ground (the gap under them is not seen).
        let f = faces(72, Some(Some(64)));
        assert_eq!((f.south_from, f.south_blocks()), (65, 8));
    }

    #[test]
    fn faces_stop_at_the_cap_and_at_unloaded_neighbours() {
        assert_eq!(faces(100, Some(None)).south_blocks(), MAX_FACE);
        assert_eq!(faces(100, Some(Some(-64))).south_blocks(), MAX_FACE);
        assert_eq!(faces(100, None).south_blocks(), 0);
    }

    #[test]
    fn face_heights_on_the_map() {
        // A top face is sin map units tall, a block of south face cos.
        let pr = p();
        let s = spans(&pr, 0, faces(64, Some(Some(60))));
        assert!(((s.top.1 - s.top.0) - pr.sin).abs() < 1e-12);
        assert!(((s.south.1 - s.south.0) - 4.0 * pr.cos).abs() < 1e-12);
    }

    #[test]
    fn rows_take_the_texels_whose_centres_are_inside() {
        // Texel j's centre is at v0 + (j + 0.5) / 2.
        assert_eq!(rows(0.0, 1.0, 0.0), (0, 2));
        assert_eq!(rows(0.3, 0.7, 0.0), (1, 1)); // centre 0.75 is out
        assert_eq!(rows(0.2, 0.8, 0.0), (0, 2)); // 0.25 and 0.75 are in
        assert_eq!(rows(-3.0, -2.0, -4.0), (2, 4));
        assert!((row_v(3, -4.0) - (-2.25)).abs() < 1e-12);
    }

    #[test]
    fn a_south_face_hit_is_on_the_ray() {
        let pr = p();
        let y = y_on_south(&pr, pr.v(66.25, 11.0), 11.0);
        assert!((y - 66.25).abs() < 1e-9);
    }

    #[test]
    fn chunk_states_clamp_to_the_depth_read() {
        let mut c = Chunk::from_fn(3, |x, z| (x == 1 && z == 2).then_some((70, 5)));
        assert_eq!(c.top(1, 2), Some(70));
        assert_eq!(c.top(0, 0), None);
        let i = 2 * 16 + 1;
        c.states[i * 3 + 1] = 6;
        c.states[i * 3 + 2] = 7;
        assert_eq!(
            [c.state(i, 0), c.state(i, 1), c.state(i, 2), c.state(i, 9)],
            [5, 6, 7, 7]
        );
    }

    #[test]
    fn map_colours_shade_south_faces() {
        let m = MapColours {
            colours: vec![0, 0x64_6464, 0xFF_0000],
        };
        let c = |s: u32| Chunk::from_fn(1, move |_, _| Some((64, s)));
        assert_eq!(m.top(&c(0), 0, 65.0).rgb, NO_COLOUR);
        assert_eq!(m.top(&c(99), 0, 65.0).rgb, NO_COLOUR);
        assert_eq!(m.top(&c(1), 0, 65.0), Texel::dry(0x64_6464, 65.0));
        assert_eq!(m.south(&c(1), 0, 64, 64.5).rgb, 0x52_5252); // 100 * 0.82 = 82
        assert_eq!(m.south(&c(2), 0, 64, 64.5).rgb, 0xD1_0000); // 255 * 0.82 = 209.1
    }

    #[test]
    fn missing_column_extras_read_as_dry_open_and_white() {
        let c = Chunk::from_fn(1, |_, _| Some((64, 1)));
        assert_eq!((c.water_at(3), c.sky_at(3)), (0, 1.0));
        assert_eq!(c.tints_at(3), [0xFF_FFFF; 3]);
        assert_eq!(c.over_at(3), CLEAR);
        let mut c = c;
        c.light = vec![0x75; 256];
        c.tints = (0..768).collect();
        assert!((c.sky_at(0) - 7.0 / 15.0).abs() < 1e-6);
        assert_eq!(c.tints_at(2), [6, 7, 8]);
    }
}
