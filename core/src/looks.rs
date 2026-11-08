//! Block looks: each block state's colours, computed at run time from the
//! textures of the player's loaded resource packs (`bake/BlockLooks.java`,
//! RULES.md sections 3 and 4). No colour or texture is stored in the port:
//! the host's block table names each state's textures and the mod averages
//! their pixels with [`average`].

use crate::style::{Tables, GLASS_WEIGHT, PLANT_WEIGHT};

/// How a ray treats a block (BlockLooks.java:81-88).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Air,
    Solid,
    Water,
    Lava,
    /// See-through, like glass: a ray passes and picks up a little colour.
    Glass,
    /// No collision, like a flower: a ray passes and picks up some colour.
    Plant,
}

impl Kind {
    /// Rays pass it, laying this much of its colour over what is below
    /// (TileBaker.java:1049-1059).
    pub fn overlay(self) -> Option<f32> {
        match self {
            Kind::Plant => Some(PLANT_WEIGHT),
            Kind::Glass => Some(GLASS_WEIGHT),
            _ => None,
        }
    }
}

/// Where a face's tint comes from (BlockLooks.java:90-94, plus the host's
/// fixed tint for a lily pad and the like).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tint {
    #[default]
    None,
    Grass,
    Foliage,
    DryFoliage,
    Water,
    Constant,
}

/// What a state's model draws on one side, from the host's table: a sprite
/// index ([`NO_SPRITE`]: nothing) and its tint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Face {
    pub sprite: u32,
    pub tint: Tint,
}

/// `face.sprite` for a side that draws nothing.
pub const NO_SPRITE: u32 = u32::MAX;

/// `block-states.state-flags` bits (the fork's `docs/mods.md`, `client-world`).
pub mod flags {
    pub const AIR: u16 = 1;
    pub const INVISIBLE: u16 = 2;
    pub const WATER: u16 = 4;
    pub const LAVA: u16 = 8;
    pub const WATERLOGGED: u16 = 16;
    pub const SEE_THROUGH: u16 = 32;
    pub const NO_COLLISION: u16 = 64;
    pub const FULL_TOP: u16 = 128;
    pub const EMISSION_SHIFT: u16 = 8;
}

/// The host's block table, as the port needs it (`client-world.block-states`).
#[derive(Clone, Debug, Default)]
pub struct Table {
    pub version: u64,
    pub blocks: Vec<String>,
    pub sprites: Vec<String>,
    pub state_block: Vec<u16>,
    pub state_flags: Vec<u16>,
    pub state_top: Vec<Face>,
    pub state_side: Vec<Face>,
    pub constant_tint: Vec<u32>,
    pub map_color: Vec<u32>,
}

/// The fluids' still textures, which a fluid state draws (BlockLooks.java:510-531,
/// `fluidLook`), when its own faces name none.
pub const WATER_SPRITE: &str = "block/water_still";
pub const LAVA_SPRITE: &str = "block/lava_still";

impl Table {
    /// The sprite index of a texture name.
    pub fn sprite(&self, name: &str) -> Option<u32> {
        let bare = |s: &str| s.trim_start_matches("minecraft:").to_owned();
        let want = bare(name);
        self.sprites
            .iter()
            .position(|s| bare(s) == want)
            .map(|i| i as u32)
    }
    /// Every sprite a state's faces use, each once, in order: the textures to
    /// average. The fluids' still textures are included.
    pub fn used_sprites(&self) -> Vec<u32> {
        let mut seen = vec![false; self.sprites.len()];
        let mut out = Vec::new();
        let mut add = |s: u32| {
            if let Some(slot) = seen.get_mut(s as usize) {
                if !*slot {
                    *slot = true;
                    out.push(s);
                }
            }
        };
        for f in self.state_top.iter().chain(&self.state_side) {
            if f.sprite != NO_SPRITE {
                add(f.sprite);
            }
        }
        for name in [WATER_SPRITE, LAVA_SPRITE] {
            if let Some(s) = self.sprite(name) {
                add(s);
            }
        }
        out
    }
}

/// A texture's average (BlockLooks.java:860-913): colour channels 0..1 of the
/// stored sRGB values, and the share of its pixels that show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Average {
    pub rgb: [f32; 3],
    pub coverage: f32,
}

/// Pixels with less alpha than this are skipped (BlockLooks.java:888).
pub const MIN_ALPHA: u8 = 26;

/// Diorama's texture average (BlockLooks.java:860-913, RULES.md section 3):
/// over the `width` x `height` RGBA pixels (straight alpha, rows top first),
/// skip alpha < 26, weight the rest by alpha/255, and average the **stored
/// sRGB** values, not linear ones (the comment at :892-894). Coverage is the
/// visible pixels over all; no visible pixel gives black with coverage 0.
/// `rgba` shorter than the size counts only the pixels it has.
pub fn average(width: u32, height: u32, rgba: &[u8]) -> Average {
    let (mut r, mut g, mut b, mut weight) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut visible = 0u32;
    let n = (width as usize * height as usize).min(rgba.len() / 4);
    for p in rgba[..n * 4].as_chunks::<4>().0 {
        let a = p[3];
        if a < MIN_ALPHA {
            continue;
        }
        let wa = f64::from(a) / 255.0;
        r += f64::from(p[0]) * wa;
        g += f64::from(p[1]) * wa;
        b += f64::from(p[2]) * wa;
        weight += wa;
        visible += 1;
    }
    if weight == 0.0 {
        return Average {
            rgb: [0.0; 3],
            coverage: 0.0,
        };
    }
    let total = (width as f64 * height as f64).max(1.0);
    Average {
        rgb: [
            (r / weight / 255.0) as f32,
            (g / weight / 255.0) as f32,
            (b / weight / 255.0) as f32,
        ],
        coverage: (f64::from(visible) / total) as f32,
    }
}

/// One face direction's running average over its quads (BlockLooks.java:915-979):
/// tinted and untinted quads apart, each weighted by its sprite's coverage.
#[derive(Clone, Copy, Debug, Default)]
pub struct FaceColor {
    untinted: [f32; 3],
    uw: f32,
    tinted: [f32; 3],
    tw: f32,
    coverage: f32,
    count: u32,
}

/// Channels 0..1 to 0xRRGGBB, each `round(v * 255)` clamped (BlockLooks.java:974-978).
pub fn pack(c: [f32; 3]) -> u32 {
    let ch = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u32;
    (ch(c[0]) << 16) | (ch(c[1]) << 8) | ch(c[2])
}

impl FaceColor {
    /// Adds a quad drawing a sprite with this average; zero coverage adds nothing.
    pub fn add(&mut self, avg: Average, tinted: bool) {
        let w = avg.coverage;
        if w <= 0.0 {
            return;
        }
        let (sum, weight) = if tinted {
            (&mut self.tinted, &mut self.tw)
        } else {
            (&mut self.untinted, &mut self.uw)
        };
        for (s, c) in sum.iter_mut().zip(avg.rgb) {
            *s += c * w;
        }
        *weight += w;
        self.coverage = self.coverage.max(w);
        self.count += 1;
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn coverage(&self) -> f32 {
        self.coverage
    }
    fn mean(sum: [f32; 3], w: f32) -> u32 {
        if w == 0.0 {
            0
        } else {
            pack([sum[0] / w, sum[1] / w, sum[2] / w])
        }
    }
    /// The face as a [`FaceLook`]: untinted and tinted colours, and the tinted share.
    pub fn look(&self) -> FaceLook {
        let total = self.uw + self.tw;
        FaceLook {
            base: Self::mean(self.untinted, self.uw),
            tinted: Self::mean(self.tinted, self.tw),
            tint_weight: if total == 0.0 { 0.0 } else { self.tw / total },
        }
    }
}

/// A face's colours: the untinted average, the tinted average (to multiply by
/// the biome colour) and how much of the face is tinted.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FaceLook {
    pub base: u32,
    pub tinted: u32,
    pub tint_weight: f32,
}

/// One state's look.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    pub kind: Kind,
    pub tint: Tint,
    /// For [`Tint::Constant`], 0xRRGGBB.
    pub constant: u32,
    /// The light it gives, 0..1.
    pub emission: f32,
    pub top: FaceLook,
    pub side: FaceLook,
}

/// Biome colours at a column: grass, foliage, water (0xRRGGBB), as
/// `client-world.columns` gives them.
pub type Tints = [u32; 3];
/// No biome: white, what Diorama uses for a missing chunk (TileBaker.java:1133-1135).
pub const WHITE: Tints = [0xFF_FFFF; 3];

/// Every state's look.
#[derive(Clone, Debug, Default)]
pub struct Looks {
    pub version: u64,
    pub looks: Vec<Look>,
}

/// A coverage under this makes a solid block see-through, unless it is leaves
/// (BlockLooks.java:497-499).
pub const GLASS_COVERAGE: f32 = 0.55;

/// A state's kind from the host's facts (BlockLooks.java:620-635 `classify`):
/// air or drawn as nothing is air; a fluid with no collision is water or
/// lava; no collision is a plant (but snow layers are solid); else solid.
/// The host marks waterlogged blocks apart, and those have collision.
pub fn classify(block: &str, f: u16) -> Kind {
    let fluid = f & (flags::WATER | flags::LAVA) != 0;
    if f & flags::AIR != 0 || (f & flags::INVISIBLE != 0 && !fluid) {
        return Kind::Air;
    }
    if fluid && f & flags::WATERLOGGED == 0 {
        return if f & flags::LAVA != 0 {
            Kind::Lava
        } else {
            Kind::Water
        };
    }
    if f & flags::NO_COLLISION != 0 {
        return if block == "minecraft:snow" {
            Kind::Solid
        } else {
            Kind::Plant
        };
    }
    Kind::Solid
}

/// Leaves stay solid whatever their coverage (`BlockTags.LEAVES`,
/// BlockLooks.java:497). The host has no tags here: a block id ending in
/// `_leaves` stands in.
fn is_leaves(block: &str) -> bool {
    block.ends_with("_leaves")
}

impl Looks {
    /// Every state's look from the table and the sprites' averages (by sprite
    /// index; none for a texture the packs lack). BlockLooks.java:389-531:
    /// - each face from its sprite (the host names one quad per side);
    /// - a state with neither face takes its map colour, or is air without one;
    /// - an empty face takes the other one;
    /// - a solid block (not leaves) with top coverage under 0.55 is glass;
    /// - the tint is the top face's, else the side's;
    /// - water draws its still texture, tinted all over by the water colour.
    pub fn build(table: &Table, averages: &[Option<Average>]) -> Looks {
        let mut b = Builder::new(table);
        b.step(table, averages, usize::MAX);
        b.finish(table)
    }

    /// A state's look; air for an unknown state.
    pub fn get(&self, state: u32) -> Look {
        self.looks.get(state as usize).copied().unwrap_or_default()
    }

    /// The biome colour a look multiplies its tinted part by (TileBaker.java:1126-1145):
    /// the column's grass, foliage or water colour, its constant, or white.
    pub fn tint_colour(look: &Look, tints: &Tints) -> u32 {
        match look.tint {
            Tint::Grass => tints[0],
            Tint::Foliage => tints[1],
            Tint::Water => tints[2],
            Tint::Constant if look.constant != 0 => look.constant,
            // Dry foliage: the host sends no column colour for it yet.
            _ => 0xFF_FFFF,
        }
    }

    /// A state's albedo in linear light on its top or side face at a column
    /// (TileBaker.java:1114-1124): `lin(base)(1-w) + lin(tinted) lin(tint) w`.
    pub fn albedo(&self, t: &Tables, state: u32, top: bool, tints: &Tints) -> [f32; 3] {
        let look = self.get(state);
        albedo(t, &look, top, tints)
    }
}

/// The fluids' still textures' averages, found once per build.
#[derive(Clone, Copy, Debug, Default)]
struct Fluids {
    water: Option<Average>,
    lava: Option<Average>,
}

/// One state's look (see [`Looks::build`]).
fn look_of(table: &Table, averages: &[Option<Average>], fluids: Fluids, id: usize) -> Look {
    let avg = |s: u32| averages.get(s as usize).copied().flatten();
    let Fluids { water, lava } = fluids;
    let block = table
        .state_block
        .get(id)
        .and_then(|&b| table.blocks.get(usize::from(b)))
        .map_or("", String::as_str);
    let f = table.state_flags[id];
    let mut kind = classify(block, f);
    if kind == Kind::Air {
        return Look::default();
    }
    let none = Face {
        sprite: NO_SPRITE,
        tint: Tint::None,
    };
    let faces = [
        table.state_top.get(id).copied().unwrap_or(none),
        table.state_side.get(id).copied().unwrap_or(none),
    ];
    let mut top = FaceColor::default();
    let mut side = FaceColor::default();
    for (fc, face) in [&mut top, &mut side].into_iter().zip(faces) {
        if face.sprite == NO_SPRITE {
            continue;
        }
        if let Some(a) = avg(face.sprite) {
            fc.add(a, face.tint != Tint::None);
        }
    }
    if top.is_empty() && side.is_empty() && kind == Kind::Lava {
        if let Some(a) = lava {
            top.add(a, false);
            side.add(a, false);
        }
    }
    if top.is_empty() && side.is_empty() && kind != Kind::Water {
        let map = table.map_color.get(id).copied().unwrap_or(0) & 0xFF_FFFF;
        if map == 0 {
            return Look::default();
        }
        let c = |s: u32| (map >> s & 0xFF) as f32 / 255.0;
        let flat = Average {
            rgb: [c(16), c(8), c(0)],
            coverage: 1.0,
        };
        top.add(flat, false);
        side.add(flat, false);
    }
    if top.is_empty() {
        top = side;
    }
    if side.is_empty() {
        side = top;
    }
    if kind == Kind::Solid && !is_leaves(block) && top.coverage() < GLASS_COVERAGE {
        kind = Kind::Glass;
    }
    let mut tint = if faces[0].tint != Tint::None {
        faces[0].tint
    } else {
        faces[1].tint
    };
    let (mut top, mut side) = (top.look(), side.look());
    if kind == Kind::Water {
        // The fluid's still texture, tinted by the water colour all over
        // (BlockLooks.java:510-531); kelp and seagrass draw as water too.
        if let Some(a) = water.filter(|a| a.coverage > 0.0) {
            let c = pack(a.rgb);
            let all = FaceLook {
                base: c,
                tinted: c,
                tint_weight: 1.0,
            };
            (top, side) = (all, all);
            tint = Tint::Water;
        }
    }
    Look {
        kind,
        tint,
        constant: table.constant_tint.get(id).copied().unwrap_or(0) & 0xFF_FFFF,
        emission: f32::from(f >> flags::EMISSION_SHIFT & 15) / 15.0,
        top,
        side,
    }
}

/// Builds [`Looks`] a few states at a time, so a rebuild after a resource
/// reload can be spread over frames.
#[derive(Clone, Debug, Default)]
pub struct Builder {
    looks: Vec<Look>,
    fluids: Option<Fluids>,
}

impl Builder {
    pub fn new(table: &Table) -> Builder {
        Builder {
            looks: Vec::with_capacity(table.state_flags.len()),
            fluids: None,
        }
    }
    /// Builds up to `n` more states; true when every state is built.
    pub fn step(&mut self, table: &Table, averages: &[Option<Average>], n: usize) -> bool {
        let avg = |s: Option<u32>| s.and_then(|s| averages.get(s as usize).copied().flatten());
        let fluids = *self.fluids.get_or_insert_with(|| Fluids {
            water: avg(table.sprite(WATER_SPRITE)),
            lava: avg(table.sprite(LAVA_SPRITE)),
        });
        let total = table.state_flags.len();
        let end = total.min(self.looks.len().saturating_add(n));
        for id in self.looks.len()..end {
            self.looks.push(look_of(table, averages, fluids, id));
        }
        self.looks.len() >= total
    }
    pub fn finish(self, table: &Table) -> Looks {
        Looks {
            version: table.version,
            looks: self.looks,
        }
    }
}

/// [`Looks::albedo`] for a look in hand.
pub fn albedo(t: &Tables, look: &Look, top: bool, tints: &Tints) -> [f32; 3] {
    let face = if top { look.top } else { look.side };
    let w = face.tint_weight;
    let base = t.linear(face.base);
    if w == 0.0 {
        return base;
    }
    let tinted = t.linear(face.tinted);
    let tint = t.linear(Looks::tint_colour(look, tints));
    let mut out = [0.0; 3];
    for c in 0..3 {
        out[c] = base[c] * (1.0 - w) + tinted[c] * tint[c] * w;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked example (RULES.md section 3, done by hand):
    /// a 16 x 16 texture whose
    /// - top 8 rows (128 px) are opaque dark red (200, 0, 0, 255),
    /// - bottom-left 8 x 8 (64 px) are half-transparent blue (0, 0, 255, 128),
    /// - bottom-right 8 x 8 (64 px) are green under the alpha cut: 32 px at
    ///   alpha 20 and 32 px at alpha 0 (0, 255, 0, a), all skipped.
    ///
    /// Weights: 128 * 1 + 64 * 128/255 = (32640 + 8192)/255 = 40832/255.
    /// Red sum 128 * 200 = 25600; blue sum 64 * 255 * 128/255 = 8192.
    /// r = 25600 / (40832/255) / 255 = 25600/40832 = 0.6269592;
    /// b = 8192/40832 = 0.2006270; g = 0; coverage = 192/256 = 0.75.
    /// Packed: round(0.6269592 * 255) = round(159.875) = 160 = 0xA0,
    /// round(0.2006270 * 255) = round(51.16) = 51 = 0x33: 0xA00033.
    /// Averaging in linear light instead (which the rule does not do) would
    /// give red lin(200) = 0.5776 * 128 / 160.125 = 0.4617, back to sRGB 181.
    #[test]
    fn worked_example_average_of_a_known_16x16_matches_the_java_rule() {
        let mut rgba = Vec::with_capacity(16 * 16 * 4);
        for y in 0..16 {
            for x in 0..16 {
                let px: [u8; 4] = match (y < 8, x < 8) {
                    (true, _) => [200, 0, 0, 255],
                    (false, true) => [0, 0, 255, 128],
                    (false, false) if y < 12 => [0, 255, 0, 20],
                    (false, false) => [0, 255, 0, 0],
                };
                rgba.extend_from_slice(&px);
            }
        }
        let a = average(16, 16, &rgba);
        assert!((a.rgb[0] - 0.626_959_2).abs() < 1e-6, "{:?}", a);
        assert_eq!(a.rgb[1], 0.0);
        assert!((a.rgb[2] - 0.200_627).abs() < 1e-6, "{:?}", a);
        assert_eq!(a.coverage, 0.75);
        assert_eq!(pack(a.rgb), 0xA0_0033);
        // One face of one untinted quad keeps that colour.
        let mut f = FaceColor::default();
        f.add(a, false);
        assert_eq!(
            f.look(),
            FaceLook {
                base: 0xA0_0033,
                tinted: 0,
                tint_weight: 0.0
            }
        );
    }

    #[test]
    fn alpha_25_is_skipped_and_26_counts() {
        let a = average(2, 1, &[10, 20, 30, 25, 200, 100, 50, 26]);
        assert_eq!(a.coverage, 0.5);
        assert!((a.rgb[0] - 200.0 / 255.0).abs() < 1e-6);
        let none = average(1, 1, &[255, 255, 255, 0]);
        assert_eq!((none.rgb, none.coverage), ([0.0; 3], 0.0));
    }

    #[test]
    fn a_face_splits_tinted_and_untinted_quads_by_coverage() {
        // An untinted quad at coverage 1 and a tinted one at coverage 0.5:
        // tint weight 0.5 / 1.5.
        let mut f = FaceColor::default();
        f.add(
            Average {
                rgb: [1.0, 0.0, 0.0],
                coverage: 1.0,
            },
            false,
        );
        f.add(
            Average {
                rgb: [0.0, 1.0, 0.0],
                coverage: 0.5,
            },
            true,
        );
        f.add(
            Average {
                rgb: [0.0, 0.0, 1.0],
                coverage: 0.0,
            },
            true,
        );
        let l = f.look();
        assert_eq!((l.base, l.tinted), (0xFF_0000, 0x00_FF00));
        assert!((l.tint_weight - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(f.coverage(), 1.0);
    }

    fn table() -> Table {
        let face = |sprite, tint| Face { sprite, tint };
        Table {
            version: 7,
            blocks: [
                "minecraft:air",
                "minecraft:grass_block",
                "minecraft:glass",
                "minecraft:oak_leaves",
                "minecraft:water",
                "minecraft:poppy",
                "minecraft:chest",
                "minecraft:glowstone",
            ]
            .map(String::from)
            .to_vec(),
            sprites: [
                "block/grass_block_top",
                "block/grass_block_side",
                "block/glass",
                "block/oak_leaves",
                "block/water_still",
                "block/poppy",
                "block/glowstone",
            ]
            .map(String::from)
            .to_vec(),
            state_block: vec![0, 1, 2, 3, 4, 5, 6, 7],
            state_flags: vec![
                flags::AIR,
                flags::FULL_TOP,
                flags::SEE_THROUGH,
                flags::SEE_THROUGH,
                flags::WATER,
                flags::NO_COLLISION,
                0,
                15 << flags::EMISSION_SHIFT,
            ],
            state_top: vec![
                face(NO_SPRITE, Tint::None),
                face(0, Tint::Grass),
                face(2, Tint::None),
                face(3, Tint::Foliage),
                face(NO_SPRITE, Tint::None),
                face(5, Tint::None),
                face(NO_SPRITE, Tint::None),
                face(6, Tint::None),
            ],
            state_side: vec![
                face(NO_SPRITE, Tint::None),
                face(1, Tint::None),
                face(2, Tint::None),
                face(3, Tint::Foliage),
                face(NO_SPRITE, Tint::None),
                face(5, Tint::None),
                face(NO_SPRITE, Tint::None),
                face(6, Tint::None),
            ],
            constant_tint: vec![0; 8],
            map_color: vec![0, 0, 0, 0, 0, 0, 0x8F_7748, 0],
        }
    }

    fn avgs() -> Vec<Option<Average>> {
        let a = |r: f32, g: f32, b: f32, coverage: f32| {
            Some(Average {
                rgb: [r, g, b],
                coverage,
            })
        };
        vec![
            a(0.6, 0.6, 0.6, 1.0),  // grass top, grey to be tinted
            a(0.5, 0.4, 0.3, 1.0),  // grass side
            a(0.9, 0.95, 1.0, 0.2), // glass: mostly clear
            a(0.4, 0.4, 0.4, 0.3),  // leaves: holes, but leaves
            a(0.7, 0.7, 0.7, 1.0),  // water still
            a(0.8, 0.1, 0.1, 0.25), // poppy
            a(0.9, 0.8, 0.5, 1.0),  // glowstone
        ]
    }

    #[test]
    fn the_table_gives_kinds_tints_and_fallbacks() {
        let t = table();
        assert_eq!(t.used_sprites(), vec![0, 2, 3, 5, 6, 1, 4]);
        let l = Looks::build(&t, &avgs());
        assert_eq!(l.version, 7);
        let k: Vec<Kind> = l.looks.iter().map(|l| l.kind).collect();
        assert_eq!(
            k,
            [
                Kind::Air,
                Kind::Solid,
                Kind::Glass,
                Kind::Solid,
                Kind::Water,
                Kind::Plant,
                Kind::Solid,
                Kind::Solid
            ]
        );
        let grass = l.get(1);
        assert_eq!(grass.tint, Tint::Grass);
        assert_eq!(grass.top.tint_weight, 1.0);
        assert_eq!(grass.top.tinted, pack([0.6; 3]));
        assert_eq!(grass.side.tint_weight, 0.0);
        assert_eq!(grass.side.base, pack([0.5, 0.4, 0.3]));
        // Water: the still texture, all tinted.
        let water = l.get(4);
        assert_eq!((water.tint, water.top.tint_weight), (Tint::Water, 1.0));
        // A chest has no faces: its map colour.
        assert_eq!(l.get(6).top.base, 0x8F_7748);
        assert_eq!(l.get(7).emission, 1.0);
        // An unknown state is air.
        assert_eq!(l.get(99).kind, Kind::Air);
        // A missing texture with no map colour: air.
        let none = Looks::build(&t, &[]);
        assert_eq!(none.get(1).kind, Kind::Air);
    }

    #[test]
    fn the_albedo_multiplies_the_tinted_part_by_the_biome_colour() {
        let tb = Tables::new();
        let l = Looks::build(&table(), &avgs());
        let tints: Tints = [0x91_BD59, 0x77_AB2F, 0x3F_76E4];
        let a = l.albedo(&tb, 1, true, &tints);
        let base = tb.linear(pack([0.6; 3]));
        let grass = tb.linear(0x91_BD59);
        for c in 0..3 {
            assert!((a[c] - base[c] * grass[c]).abs() < 1e-7);
        }
        // Another biome's grass gives another colour; the side is untinted.
        let dry: Tints = [0xBF_B755, 0xAE_A42A, 0x3F_76E4];
        assert_ne!(l.albedo(&tb, 1, true, &dry), a);
        assert_eq!(
            l.albedo(&tb, 1, false, &dry),
            l.albedo(&tb, 1, false, &tints)
        );
        // White tints leave the tinted average as it is.
        let w = l.albedo(&tb, 1, true, &WHITE);
        assert!((w[0] - base[0]).abs() < 1e-7);
    }

    #[test]
    fn classify_follows_the_java_rule() {
        assert_eq!(classify("minecraft:air", flags::AIR), Kind::Air);
        assert_eq!(classify("minecraft:barrier", flags::INVISIBLE), Kind::Air);
        assert_eq!(classify("minecraft:water", flags::WATER), Kind::Water);
        assert_eq!(classify("minecraft:lava", flags::LAVA), Kind::Lava);
        assert_eq!(
            classify("minecraft:oak_stairs", flags::WATER | flags::WATERLOGGED),
            Kind::Solid
        );
        assert_eq!(
            classify("minecraft:short_grass", flags::NO_COLLISION),
            Kind::Plant
        );
        assert_eq!(classify("minecraft:snow", flags::NO_COLLISION), Kind::Solid);
    }
}
