//! Waypoints (RULES.md section 10): the waypoint record and its file, the
//! death rule, the per-world storage index, the `/dwp` command line, the
//! teleport line, and where pins and edge badges go on the map.
//!
//! The file is Diorama's own `waypoints.json` shape (WaypointStore.java:519,
//! Gson over Waypoint.java's fields): a JSON array of objects with `id`,
//! `name`, `initials`, `icon`, `x`, `y`, `z`, `dimension`, `color` (a signed
//! 32-bit ARGB, as Java's `int`), `visible`, `folder`, `kind` and `created`.
//! Fields this port does not use (`map`, `deleted`, `edited`, ...) are kept
//! as they were read, so a file copied from Diorama survives a round trip.

use crate::json::Json;
use crate::layout::clock_text;

/// Diorama's 16 waypoint colours, in order (Waypoint.java:38-41), and their
/// names (WaypointColours.java:23-26).
pub const PALETTE: [u32; 16] = [
    0xFFE5_484D,
    0xFFF7_6B15,
    0xFFFF_C53D,
    0xFF46_A758,
    0xFF12_A594,
    0xFF00_90FF,
    0xFF3E_63DD,
    0xFF8E_4EC6,
    0xFFD6_409F,
    0xFFFF_FFFF,
    0xFFB0_B4BA,
    0xFF60_646C,
    0xFF8D_6B4B,
    0xFF99_D52A,
    0xFF7C_E2FE,
    0xFFF0_C0D8,
];
pub const PALETTE_NAMES: [&str; 16] = [
    "red", "orange", "amber", "green", "teal", "sky", "indigo", "violet", "pink", "white",
    "silver", "slate", "brown", "lime", "ice", "blossom",
];
/// A death marker's colour (Waypoints.java:173) and a bed's (:153).
pub const DEATH_COLOR: u32 = 0xFF60_646C;
pub const BED_COLOR: u32 = 0xFFE5_484D;
/// Death markers kept per world (WaypointStore.java:40).
pub const DEATHS_KEPT: usize = 3;
/// The most letters a pin shows (Waypoint.java:52-53).
pub const MAX_INITIALS: usize = 4;
/// The port's caps: waypoints per world and characters per name.
pub const MAX_WAYPOINTS: usize = 256;
pub const MAX_NAME: usize = 64;
/// The "show within" choices in blocks, 0 for any distance (Settings.java:42).
pub const DISTANCES: [u32; 7] = [0, 64, 128, 256, 512, 1024, 2048];
pub const OVERWORLD: &str = "minecraft:overworld";

/// Storage: the index of worlds, and how many worlds keep waypoint files.
/// The mod may hold 64 storage keys (`STORAGE_MAX_KEYS`); the settings take
/// 16 and the index one, so 45 worlds leaves room for a few more settings.
pub const INDEX_KEY: &str = "wp-index";
pub const MAX_WORLDS: usize = 45;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Manual,
    Death,
    Bed,
    /// One of Diorama's other kinds (TEMPORARY, WAYSTONE, the portals), kept by name.
    Other(String),
}

impl Kind {
    pub fn name(&self) -> &str {
        match self {
            Kind::Manual => "MANUAL",
            Kind::Death => "DEATH",
            Kind::Bed => "BED",
            Kind::Other(s) => s,
        }
    }
    pub fn from_name(s: &str) -> Kind {
        match s {
            "MANUAL" => Kind::Manual,
            "DEATH" => Kind::Death,
            "BED" => Kind::Bed,
            other => Kind::Other(other.to_owned()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Waypoint {
    pub id: String,
    pub name: String,
    /// Empty: the name's initials.
    pub initials: String,
    pub icon: String,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub dimension: String,
    /// 0xAARRGGBB.
    pub color: u32,
    pub visible: bool,
    pub folder: String,
    pub kind: Kind,
    /// Diorama: milliseconds since 1970. The port has no wall clock, so it
    /// writes a number above every other one in the file (see [`Set::next_created`]).
    pub created: i64,
    /// Fields the port does not use, as they were read.
    pub extra: Vec<(String, Json)>,
}

/// First letter of up to two words, upper case: "Home" is H, "Iron Farm" is
/// IF; "?" for no words (Waypoint.java:139-147).
pub fn initials_for(name: &str) -> String {
    let mut out = String::new();
    for word in name.split_whitespace().take(2) {
        if let Some(c) = word.chars().next() {
            out.extend(c.to_uppercase());
        }
    }
    if out.is_empty() {
        "?".into()
    } else {
        out
    }
}

/// A name cut to [`MAX_NAME`] characters, with control characters dropped.
pub fn clean_name(name: &str) -> String {
    name.trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect()
}

impl Waypoint {
    pub fn new(
        id: String,
        name: &str,
        at: (i32, i32, i32),
        dimension: &str,
        color: u32,
    ) -> Waypoint {
        let name = clean_name(name);
        Waypoint {
            id,
            initials: initials_for(&name),
            name,
            icon: String::new(),
            x: at.0,
            y: at.1,
            z: at.2,
            dimension: dimension.to_owned(),
            color,
            visible: true,
            folder: String::new(),
            kind: Kind::Manual,
            created: 0,
            extra: Vec::new(),
        }
    }

    /// What the pin shows: the initials, at most [`MAX_INITIALS`] letters.
    /// Diorama draws a death marker with an icon; the port, which copies no
    /// art, marks one with an X unless the player gave it other initials
    /// (Diorama's own death markers carry their name's, "D1" for "Death 12:00").
    pub fn label(&self) -> String {
        if self.kind == Kind::Death
            && (self.initials.is_empty() || self.initials == initials_for(&self.name))
        {
            return "X".into();
        }
        let l = if self.initials.is_empty() {
            initials_for(&self.name)
        } else {
            self.initials.clone()
        };
        l.chars().take(MAX_INITIALS).collect()
    }

    /// From the block's centre (Waypoint.java:131-136).
    pub fn distance_to(&self, px: f64, py: f64, pz: f64) -> f64 {
        let dx = f64::from(self.x) + 0.5 - px;
        let dy = f64::from(self.y) - py;
        let dz = f64::from(self.z) + 0.5 - pz;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }

    /// Death and bed markers show at any distance (WaypointStore.java:334-340).
    pub fn always_shown(&self) -> bool {
        matches!(self.kind, Kind::Death | Kind::Bed)
    }

    pub fn to_json(&self) -> Json {
        let s = |v: &str| Json::Str(v.to_owned());
        let n = |v: i64| Json::Num(v as f64);
        let mut fields = vec![
            ("id".to_owned(), s(&self.id)),
            ("name".to_owned(), s(&self.name)),
            ("initials".to_owned(), s(&self.initials)),
            ("icon".to_owned(), s(&self.icon)),
            ("x".to_owned(), n(i64::from(self.x))),
            ("y".to_owned(), n(i64::from(self.y))),
            ("z".to_owned(), n(i64::from(self.z))),
            ("dimension".to_owned(), s(&self.dimension)),
            // Java's int: 0xFFE5484D is -1750963.
            ("color".to_owned(), n(i64::from(self.color as i32))),
            ("visible".to_owned(), Json::Bool(self.visible)),
            ("folder".to_owned(), s(&self.folder)),
            ("kind".to_owned(), s(self.kind.name())),
            ("created".to_owned(), n(self.created)),
        ];
        fields.extend(self.extra.iter().cloned());
        Json::Obj(fields)
    }

    /// One entry of the file, with what a hand-edited or older file left
    /// out filled in as Diorama does (WaypointStore.java:446-470); none
    /// when it is unusable (not an object, or no dimension).
    pub fn from_json(v: &Json, new_id: &mut dyn FnMut() -> String) -> Option<Waypoint> {
        let Json::Obj(fields) = v else { return None };
        let dimension = v.get("dimension")?.as_str()?.to_owned();
        let text = |k: &str| v.get(k).and_then(Json::as_str).map(str::to_owned);
        let int = |k: &str| v.get(k).and_then(Json::as_i64);
        let coord = |k: &str| int(k).map_or(0, |n| n.clamp(-30_000_000, 30_000_000) as i32);
        let name = text("name")
            .map(|n| clean_name(&n))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Waypoint".into());
        const KNOWN: [&str; 13] = [
            "id",
            "name",
            "initials",
            "icon",
            "x",
            "y",
            "z",
            "dimension",
            "color",
            "visible",
            "folder",
            "kind",
            "created",
        ];
        Some(Waypoint {
            id: text("id").filter(|s| !s.is_empty()).unwrap_or_else(new_id),
            initials: text("initials")
                .map(|s| s.chars().take(MAX_INITIALS).collect())
                .unwrap_or_default(),
            name,
            icon: text("icon").unwrap_or_default(),
            x: coord("x"),
            y: coord("y"),
            z: coord("z"),
            dimension,
            // Signed or not, the low 32 bits are the colour.
            color: int("color").map_or(PALETTE[0], |c| c as u32),
            visible: v.get("visible").and_then(Json::as_bool).unwrap_or(true),
            folder: text("folder").unwrap_or_default(),
            kind: text("kind").map_or(Kind::Manual, |k| Kind::from_name(&k)),
            created: int("created").unwrap_or(0),
            extra: fields
                .iter()
                .filter(|(k, _)| !KNOWN.contains(&k.as_str()))
                .cloned()
                .collect(),
        })
    }
}

/// Reads a waypoint file: the waypoints, and how many entries were skipped
/// as unusable. Err when the text is not a JSON array.
pub fn parse_file(
    text: &str,
    new_id: &mut dyn FnMut() -> String,
) -> Result<(Vec<Waypoint>, usize), String> {
    let Json::Arr(items) = Json::parse(text)? else {
        return Err("the waypoint file is not a JSON array".into());
    };
    let mut out = Vec::new();
    let mut skipped = 0;
    for item in &items {
        match Waypoint::from_json(item, new_id) {
            Some(w) if out.len() < MAX_WAYPOINTS => out.push(w),
            _ => skipped += 1,
        }
    }
    Ok((out, skipped))
}

/// The file text for `list`; temporary pins are never saved (WaypointStore.java:519).
pub fn write_file(list: &[Waypoint]) -> String {
    let items = list
        .iter()
        .filter(|w| w.kind != Kind::Other("TEMPORARY".into()))
        .map(Waypoint::to_json)
        .collect();
    let mut text = Json::Arr(items).pretty();
    text.push('\n');
    text
}

/// A small random source (splitmix64). WASM mods have no entropy source and
/// no clock, so the mod seeds it from what changes between calls (the game
/// time, the frame count, the player's position).
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }
    /// Stirs more into the state.
    pub fn mix(&mut self, v: u64) {
        self.0 ^= v.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.next_u64();
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in `0..1`.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    /// A random (version 4) UUID, as Diorama's ids are.
    pub fn uuid(&mut self) -> String {
        let a = self.next_u64();
        let b = self.next_u64();
        let a = (a & !0xF000) | 0x4000; // version 4
        let b = (b & !(0xC000u64 << 48)) | (0x8000u64 << 48); // RFC 4122 variant
        format!(
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            a >> 32,
            (a >> 16) & 0xFFFF,
            a & 0xFFFF,
            b >> 48,
            b & 0xFFFF_FFFF_FFFF
        )
    }
}

/// The hue of an ARGB colour, 0..1 (red 0).
pub fn hue_of(argb: u32) -> f32 {
    let r = ((argb >> 16) & 0xFF) as f32 / 255.0;
    let g = ((argb >> 8) & 0xFF) as f32 / 255.0;
    let b = (argb & 0xFF) as f32 / 255.0;
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    if d <= 0.0 {
        return 0.0;
    }
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    h / 6.0
}

/// An opaque colour from hue, saturation and value, each 0..1.
pub fn hsv(hue: f32, saturation: f32, value: f32) -> u32 {
    let h = hue.rem_euclid(1.0) * 6.0;
    let c = value * saturation;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = value - c;
    let byte = |v: f32| ((v + m) * 255.0 + 0.5).clamp(0.0, 255.0) as u32;
    0xFF00_0000 | (byte(r) << 16) | (byte(g) << 8) | byte(b)
}

/// How far apart two hues are round the wheel, 0..=0.5.
pub fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 1.0;
    d.min(1.0 - d)
}

/// A new waypoint's colour, as Diorama picks it (WaypointColours.java:67-79):
/// any hue at least a sixth of the wheel from `avoid`'s, saturation
/// 0.6..0.95, value 0.8..1.
pub fn random_color(rng: &mut Rng, avoid: u32) -> u32 {
    let avoid_hue = hue_of(avoid);
    let mut hue = rng.next_f32();
    // Two thirds of the wheel qualify; a few tries always find one, and the
    // cap only bounds the loop.
    for _ in 0..64 {
        if hue_distance(hue, avoid_hue) >= 1.0 / 6.0 {
            break;
        }
        hue = rng.next_f32();
    }
    let s = 0.6 + rng.next_f32() * 0.35;
    let v = 0.8 + rng.next_f32() * 0.2;
    hsv(hue, s, v)
}

/// A colour the player names: a palette name, `#RRGGBB` or `RRGGBB`.
pub fn parse_color(text: &str) -> Option<u32> {
    let t = text.trim().to_ascii_lowercase();
    if let Some(i) = PALETTE_NAMES.iter().position(|n| *n == t) {
        return Some(PALETTE[i]);
    }
    let hex = t.strip_prefix('#').unwrap_or(&t);
    (hex.len() == 6)
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
        .map(|c| 0xFF00_0000 | c)
}

/// A palette colour's name, or `#RRGGBB`.
pub fn color_name(argb: u32) -> String {
    match PALETTE
        .iter()
        .position(|&c| c & 0xFF_FFFF == argb & 0xFF_FFFF)
    {
        Some(i) => PALETTE_NAMES[i].to_owned(),
        None => format!("#{:06X}", argb & 0xFF_FFFF),
    }
}

/// The palette colour after `argb` (the first for one not in the palette).
pub fn next_palette(argb: u32) -> u32 {
    match PALETTE
        .iter()
        .position(|&c| c & 0xFF_FFFF == argb & 0xFF_FFFF)
    {
        Some(i) => PALETTE[(i + 1) % PALETTE.len()],
        None => PALETTE[0],
    }
}

/// Dark letters on a light pin, white on a dark one (0xRRGGBB).
pub fn label_color(argb: u32) -> u32 {
    let r = ((argb >> 16) & 0xFF) as f32;
    let g = ((argb >> 8) & 0xFF) as f32;
    let b = (argb & 0xFF) as f32;
    if 0.299 * r + 0.587 * g + 0.114 * b > 150.0 {
        0x10_1418
    } else {
        0xFF_FFFF
    }
}

/// A death marker's name: "Death HH:mm" (Waypoint.java:97-99). Diorama uses
/// the local wall clock; a WASM mod has none, so the port writes the world's
/// time of day (24-hour, as the clock info line shows it).
pub fn death_name(day_time: u64) -> String {
    format!("Death {}", clock_text(day_time))
}

/// The waypoints of one world, in the order they were added.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Set {
    pub list: Vec<Waypoint>,
}

impl Set {
    /// A `created` number above every other in the set, and at least `now`.
    pub fn next_created(&self, now: i64) -> i64 {
        let top = self.list.iter().map(|w| w.created).max().unwrap_or(0);
        now.max(top.saturating_add(1))
    }

    /// A new waypoint at a block (Waypoints.java:32-43): "Waypoint N" with N
    /// one more than the waypoints there are, a random colour unlike the last
    /// waypoint's. Returns its index.
    pub fn add(
        &mut self,
        name: Option<&str>,
        at: (i32, i32, i32),
        dimension: &str,
        rng: &mut Rng,
        now: i64,
    ) -> Result<usize, String> {
        if self.list.len() >= MAX_WAYPOINTS {
            return Err(format!("this world already has {MAX_WAYPOINTS} waypoints"));
        }
        let default = format!("Waypoint {}", self.list.len() + 1);
        let name = name
            .map(clean_name)
            .filter(|n| !n.is_empty())
            .unwrap_or(default);
        let avoid = self.list.last().map_or(0, |w| w.color);
        let color = random_color(rng, avoid);
        let mut w = Waypoint::new(rng.uuid(), &name, at, dimension, color);
        w.created = self.next_created(now);
        self.list.push(w);
        Ok(self.list.len() - 1)
    }

    /// A death marker (Waypoints.java:163-176), then the oldest death markers
    /// dropped until [`DEATHS_KEPT`] remain (WaypointStore.java:346-352).
    pub fn add_death(
        &mut self,
        at: (i32, i32, i32),
        dimension: &str,
        day_time: u64,
        rng: &mut Rng,
        now: i64,
    ) {
        let mut w = Waypoint::new(
            rng.uuid(),
            &death_name(day_time),
            at,
            dimension,
            DEATH_COLOR,
        );
        w.kind = Kind::Death;
        w.initials.clear();
        w.created = self.next_created(now);
        self.list.push(w);
        loop {
            let deaths = self.list.iter().filter(|w| w.kind == Kind::Death);
            if deaths.clone().count() <= DEATHS_KEPT {
                break;
            }
            let oldest = deaths.map(|w| w.created).min().unwrap_or(0);
            let at = self
                .list
                .iter()
                .position(|w| w.kind == Kind::Death && w.created == oldest);
            match at {
                Some(i) => {
                    self.list.remove(i);
                }
                None => break,
            }
        }
        if self.list.len() > MAX_WAYPOINTS {
            self.list.remove(0);
        }
    }

    /// The waypoint a player means: its number in the list (`3` or `#3`),
    /// its name (any case), or the one name it starts.
    pub fn find(&self, target: &str) -> Result<usize, String> {
        let t = target.trim();
        if t.is_empty() {
            return Err("name a waypoint, or give its number from /dwp list".into());
        }
        if let Ok(n) = t.trim_start_matches('#').parse::<usize>() {
            if (1..=self.list.len()).contains(&n) {
                return Ok(n - 1);
            }
            if t.starts_with('#') || !self.list.iter().any(|w| w.name == t) {
                return Err(format!(
                    "no waypoint number {n} (there are {})",
                    self.list.len()
                ));
            }
        }
        let lower = t.to_lowercase();
        if let Some(i) = self
            .list
            .iter()
            .position(|w| w.name.to_lowercase() == lower)
        {
            return Ok(i);
        }
        let starts: Vec<usize> = (0..self.list.len())
            .filter(|&i| self.list[i].name.to_lowercase().starts_with(&lower))
            .collect();
        match starts[..] {
            [i] => Ok(i),
            [] => Err(format!("no waypoint named {t:?}")),
            _ => Err(format!(
                "{} waypoints start with {t:?}: give the number",
                starts.len()
            )),
        }
    }
}

/// A dimension's short name for the list.
pub fn dimension_name(id: &str) -> String {
    match id {
        OVERWORLD => "Overworld".into(),
        "minecraft:the_nether" => "Nether".into(),
        "minecraft:the_end" => "End".into(),
        other => crate::layout::biome_name(other),
    }
}

/// One row of the list: "3. Home  120 m", with the dimension for one
/// elsewhere and "hidden" for one kept off the map.
pub fn row_text(n: usize, w: &Waypoint, here: Option<(&str, f64, f64, f64)>) -> String {
    let mut s = format!("{}. {}", n + 1, w.name);
    match here {
        Some((dim, x, y, z)) if dim == w.dimension => {
            s.push_str(&format!("  {} m", w.distance_to(x, y, z).round() as i64));
        }
        _ => s.push_str(&format!("  ({})", dimension_name(&w.dimension))),
    }
    if !w.visible {
        s.push_str("  hidden");
    }
    s
}

/// Whether a waypoint gets a pin on the map in `dimension`, for a
/// "show within" distance (0: any; DistanceRule WaypointStore.java:334-340).
pub fn on_map(w: &Waypoint, dimension: &str, within: u32, at: (f64, f64, f64)) -> bool {
    w.visible
        && w.dimension == dimension
        && (within == 0 || w.always_shown() || w.distance_to(at.0, at.1, at.2) <= f64::from(within))
}

/// The one command line the mod ever runs (capability `command`): a
/// vanilla `tp` of the player to a waypoint's block centre, through
/// `execute in` for another dimension. Coordinates are whole numbers from
/// the record and the dimension must look like a resource id, so nothing
/// read from a file can make it another command. None for a bad dimension.
pub fn teleport_line(w: &Waypoint, here: &str) -> Option<String> {
    let id_ok = |s: &str| {
        let (ns, path) = s.split_once(':')?;
        let ok = |p: &str, extra: &str| {
            !p.is_empty()
                && p.chars().all(|c| {
                    c.is_ascii_lowercase()
                        || c.is_ascii_digit()
                        || "_.-".contains(c)
                        || extra.contains(c)
                })
        };
        (ok(ns, "") && ok(path, "/")).then_some(())
    };
    id_ok(&w.dimension)?;
    let centre = |v: i32| f64::from(v) + 0.5;
    let tp = format!("tp @s {} {} {}", centre(w.x), w.y, centre(w.z));
    Some(if w.dimension == here {
        tp
    } else {
        format!("execute in {} run {tp}", w.dimension)
    })
}

/// The storage key of a world's waypoint file: `wp-` and the world id's
/// 64-bit FNV-1a hash in hex (storage keys are `[a-z0-9_.-]`, world ids
/// are not).
pub fn world_key(world: &str) -> String {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in world.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01B3);
    }
    format!("wp-{h:016x}")
}

/// Which worlds have waypoint files, and when each was last opened (a
/// counter), so the 46th world drops the least recently opened one.
/// Stored as text, one `used<TAB>key<TAB>world id` line per world.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Index {
    pub worlds: Vec<IndexEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexEntry {
    pub used: u64,
    pub key: String,
    pub world: String,
}

impl Index {
    pub fn parse(text: &str) -> Index {
        let worlds = text
            .lines()
            .filter_map(|l| {
                let mut p = l.splitn(3, '\t');
                let used = p.next()?.parse().ok()?;
                let key = p.next()?.to_owned();
                let world = p.next()?.to_owned();
                (key.starts_with("wp-") && key != INDEX_KEY).then_some(IndexEntry {
                    used,
                    key,
                    world,
                })
            })
            .collect();
        Index { worlds }
    }
    pub fn format(&self) -> String {
        self.worlds
            .iter()
            .map(|e| {
                format!(
                    "{}\t{}\t{}\n",
                    e.used,
                    e.key,
                    e.world.replace(['\t', '\n'], " ")
                )
            })
            .collect()
    }
    /// The world was opened: its key, and the entries dropped to keep at
    /// most `max` worlds (their files should be deleted).
    pub fn open(&mut self, world: &str, max: usize) -> (String, Vec<IndexEntry>) {
        let next = self.worlds.iter().map(|e| e.used).max().unwrap_or(0) + 1;
        let key = match self.worlds.iter_mut().find(|e| e.world == world) {
            Some(e) => {
                e.used = next;
                e.key.clone()
            }
            None => {
                let key = world_key(world);
                self.worlds.push(IndexEntry {
                    used: next,
                    key: key.clone(),
                    world: world.to_owned(),
                });
                key
            }
        };
        let mut dropped = Vec::new();
        while self.worlds.len() > max.max(1) {
            let Some(oldest) = (0..self.worlds.len())
                .filter(|&i| self.worlds[i].world != world)
                .min_by_key(|&i| self.worlds[i].used)
            else {
                break;
            };
            dropped.push(self.worlds.remove(oldest));
        }
        (key, dropped)
    }
}

/// A `/dwp` command, parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dwp {
    Help,
    Add(Option<String>),
    Del(String),
    Rename(String, String),
    Color(String, String),
    List,
    Hide(String),
    Show(String),
    Tp(String),
    /// Settings: pins on the map, death markers, show within N blocks.
    Pins(Option<bool>),
    Deaths(Option<bool>),
    Within(Option<u32>),
}

pub const DWP_HELP: &str = "/dwp add [name] | del <n|name> | rename <n|name> <new name> | \
     color <n|name> <colour|#RRGGBB> | list | hide|show <n|name> | tp <n|name> | \
     pins on|off | deaths on|off | within <blocks|0>";

/// The first word of `s`, or a "quoted phrase", and the rest.
fn first_target(s: &str) -> (String, &str) {
    let s = s.trim_start();
    if let Some(rest) = s.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return (rest[..end].to_owned(), &rest[end + 1..]);
        }
    }
    match s.find(char::is_whitespace) {
        Some(i) => (s[..i].to_owned(), &s[i..]),
        None => (s.to_owned(), ""),
    }
}

/// The whole of `s` as a target, without surrounding quotes.
fn whole_target(s: &str) -> String {
    let t = s.trim();
    t.strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(t)
        .to_owned()
}

fn on_off(s: &str) -> Result<Option<bool>, String> {
    match s.trim() {
        "" => Ok(None),
        "on" | "true" => Ok(Some(true)),
        "off" | "false" => Ok(Some(false)),
        other => Err(format!("{other:?}: say on or off")),
    }
}

/// Reads the arguments of `/dwp` (the line after the command's name).
pub fn parse_command(args: &str) -> Result<Dwp, String> {
    let args = args.trim();
    let (verb, rest) = match args.find(char::is_whitespace) {
        Some(i) => (&args[..i], args[i..].trim()),
        None => (args, ""),
    };
    let need = |what: &str| -> Result<String, String> {
        let t = whole_target(rest);
        if t.is_empty() {
            Err(format!("usage: /dwp {verb} {what}"))
        } else {
            Ok(t)
        }
    };
    Ok(match verb.to_ascii_lowercase().as_str() {
        "" | "help" => Dwp::Help,
        "add" | "new" => Dwp::Add((!rest.is_empty()).then(|| whole_target(rest))),
        "del" | "delete" | "remove" => Dwp::Del(need("<n|name>")?),
        "list" => Dwp::List,
        "hide" => Dwp::Hide(need("<n|name>")?),
        "show" => Dwp::Show(need("<n|name>")?),
        "tp" | "teleport" => Dwp::Tp(need("<n|name>")?),
        "rename" => {
            let (target, new) = first_target(rest);
            let new = whole_target(new);
            if target.is_empty() || new.is_empty() {
                return Err("usage: /dwp rename <n|name> <new name>".into());
            }
            Dwp::Rename(target, new)
        }
        "color" | "colour" => {
            let (target, colour) = match rest.rfind(char::is_whitespace) {
                Some(i) => (whole_target(&rest[..i]), rest[i..].trim().to_owned()),
                None => (String::new(), String::new()),
            };
            if target.is_empty() || colour.is_empty() {
                return Err("usage: /dwp color <n|name> <colour|#RRGGBB>".into());
            }
            Dwp::Color(target, colour)
        }
        "pins" => Dwp::Pins(on_off(rest)?),
        "deaths" => Dwp::Deaths(on_off(rest)?),
        "within" | "distance" => Dwp::Within(match rest {
            "" => None,
            r => Some(
                r.parse()
                    .map_err(|_| format!("{r:?}: give a number of blocks, 0 for any"))?,
            ),
        }),
        other => return Err(format!("unknown /dwp {other}: {DWP_HELP}")),
    })
}

// ---- Layout on the map -------------------------------------------------

/// A pin's head: `HEAD_H` high, at least `MIN_HEAD_W` wide, standing
/// `TAIL` above the spot it marks. Edge badges are heads with a `CARET`
/// pointing out of the map toward the waypoint. The port's own sizes (GUI
/// pixels), not Diorama's pin art.
pub const HEAD_H: f32 = 10.0;
pub const MIN_HEAD_W: f32 = 10.0;
pub const TAIL: f32 = 3.0;
pub const CARET: f32 = 4.0;
/// The caret turns in 32 steps (Pins.java:25).
pub const CARET_STEPS: u32 = 32;

/// The map's area on screen, in GUI pixels; a round map is the circle in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub round: bool,
}

impl View {
    pub fn centre(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    fn radius(&self) -> f32 {
        self.w.min(self.h) / 2.0
    }
    fn holds(&self, (px, py): (f32, f32)) -> bool {
        if self.round {
            let (cx, cy) = self.centre();
            let r = self.radius();
            (px - cx).powi(2) + (py - cy).powi(2) <= r * r
        } else {
            px >= self.x && py >= self.y && px <= self.x + self.w && py <= self.y + self.h
        }
    }
}

/// The head's width for a label `label_w` pixels wide.
pub fn head_width(label_w: f32) -> f32 {
    (label_w + 4.0).max(MIN_HEAD_W)
}

/// Where a waypoint goes on the map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placed {
    /// The head stands over the spot `(x, y)`.
    Standing { x: f32, y: f32 },
    /// A badge centred at `(x, y)` on the map's edge, its caret along
    /// `angle` (radians, screen axes: 0 right, pi/2 down) toward the spot.
    Badge { x: f32, y: f32, angle: f32 },
}

/// The angle snapped to [`CARET_STEPS`] steps.
pub fn snap_angle(angle: f32) -> f32 {
    let step = std::f32::consts::TAU / CARET_STEPS as f32;
    (angle / step).round() * step
}

/// Places a waypoint whose spot is at screen `(x, y)` (WaypointOverlay.java:
/// 38-78): a standing pin when all of it fits in the view, otherwise a badge
/// pulled toward the centre until it and its caret are inside — on a square
/// map dividing the offset by `max(1, |dx|/halfW, |dy|/halfH)`, on a round
/// one by `max(1, |d|/radius)`.
pub fn place(view: &View, x: f32, y: f32, head_w: f32) -> Placed {
    let (cx, cy) = view.centre();
    let corners = [
        (x - head_w / 2.0, y - TAIL - HEAD_H),
        (x + head_w / 2.0, y - TAIL - HEAD_H),
        (x - head_w / 2.0, y),
        (x + head_w / 2.0, y),
    ];
    if x.is_finite() && y.is_finite() && corners.iter().all(|&c| view.holds(c)) {
        return Placed::Standing { x, y };
    }
    let (dx, dy) = if x.is_finite() && y.is_finite() {
        (x - cx, y - cy)
    } else {
        (0.0, -1.0)
    };
    // Each value at least a pixel, so a view smaller than a badge still
    // gets finite positions.
    let reach = if view.round {
        let radius = (view.radius() - head_w.max(HEAD_H) / 2.0 - CARET).max(1.0);
        (dx.hypot(dy) / radius).max(1.0)
    } else {
        let half_w = (view.w / 2.0 - head_w / 2.0 - CARET).max(1.0);
        let half_h = (view.h / 2.0 - HEAD_H / 2.0 - CARET).max(1.0);
        (dx.abs() / half_w).max(dy.abs() / half_h).max(1.0)
    };
    Placed::Badge {
        x: cx + dx / reach,
        y: cy + dy / reach,
        angle: snap_angle(dy.atan2(dx)),
    }
}

/// What to draw for a placed waypoint: the head's rectangle `(x, y, w, h)`,
/// the tail or caret as a quad (a triangle with its last corner repeated),
/// and the label's top centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PinShape {
    pub head: (f32, f32, f32, f32),
    pub point: [(f32, f32); 4],
    pub label: (f32, f32),
}

pub fn pin_shape(placed: Placed, head_w: f32) -> PinShape {
    match placed {
        Placed::Standing { x, y } => {
            let top = y - TAIL - HEAD_H;
            let base = y - TAIL;
            PinShape {
                head: (x - head_w / 2.0, top, head_w, HEAD_H),
                point: [(x - 2.5, base), (x + 2.5, base), (x, y), (x, y)],
                label: (x, top + 1.0),
            }
        }
        Placed::Badge { x, y, angle } => {
            let (dx, dy) = (angle.cos(), angle.sin());
            // From the centre to the head's edge along the caret.
            let (a, b) = (head_w / 2.0, HEAD_H / 2.0);
            let edge = (a / dx.abs().max(1e-6)).min(b / dy.abs().max(1e-6));
            let (px, py) = (-dy, dx);
            let base = (x + dx * (edge - 0.5), y + dy * (edge - 0.5));
            let tip = (x + dx * (edge + CARET), y + dy * (edge + CARET));
            PinShape {
                head: (x - a, y - b, head_w, HEAD_H),
                point: [
                    (base.0 + px * 3.0, base.1 + py * 3.0),
                    (base.0 - px * 3.0, base.1 - py * 3.0),
                    tip,
                    tip,
                ],
                label: (x, y - b + 1.0),
            }
        }
    }
}

/// A point of the mosaic (texel `t`) on screen, for an image drawn from the
/// source window `(sx, sy, side)` into a square at `(dx, dy)` `dw` wide.
pub fn screen_of(t: (f64, f64), src: (f32, f32, f32), dst: (f32, f32, f32)) -> (f32, f32) {
    let scale = dst.2 / src.2;
    (
        dst.0 + (t.0 as f32 - src.0) * scale,
        dst.1 + (t.1 as f32 - src.1) * scale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> impl FnMut() -> String {
        let mut rng = Rng::new(7);
        move || rng.uuid()
    }

    fn sample() -> Set {
        let mut rng = Rng::new(1);
        let mut s = Set::default();
        s.add(Some("Home"), (10, 64, -20), OVERWORLD, &mut rng, 0)
            .unwrap();
        s.add(Some("Iron Farm"), (-300, 70, 5), OVERWORLD, &mut rng, 0)
            .unwrap();
        s.add(None, (8, 40, 8), "minecraft:the_nether", &mut rng, 0)
            .unwrap();
        s.add_death((1, 2, 3), OVERWORLD, 6000, &mut rng, 0);
        s.list[1].visible = false;
        s.list[1]
            .extra
            .push(("map".into(), Json::Str("caves".into())));
        s
    }

    #[test]
    fn serialise_round_trip() {
        let s = sample();
        let text = write_file(&s.list);
        let (back, skipped) = parse_file(&text, &mut ids()).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(back, s.list);
        // And again: the text is stable.
        assert_eq!(write_file(&back), text);
    }

    #[test]
    fn serialise_writes_diorama_field_names_and_java_ints() {
        let s = sample();
        let text = write_file(&s.list);
        let v = Json::parse(&text).unwrap();
        let Json::Arr(items) = &v else { panic!() };
        let first = &items[0];
        for k in [
            "id",
            "name",
            "initials",
            "icon",
            "x",
            "y",
            "z",
            "dimension",
            "color",
            "visible",
            "folder",
            "kind",
            "created",
        ] {
            assert!(first.get(k).is_some(), "{k}");
        }
        let death = &items[3];
        assert_eq!(death.get("kind").and_then(Json::as_str), Some("DEATH"));
        assert_eq!(
            death.get("name").and_then(Json::as_str),
            Some("Death 12:00")
        );
        // 0xFF60646C as Java's int.
        assert_eq!(death.get("color").and_then(Json::as_i64), Some(-10_460_052));
        assert_eq!(items[1].get("map").and_then(Json::as_str), Some("caves"));
        let id = first.get("id").and_then(Json::as_str).unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
    }

    #[test]
    fn serialise_reads_a_diorama_file_and_fills_gaps() {
        let text = r#"[
          {"id":"0b0f6a3e-1111-4222-8333-444455556666","name":"Base","initials":"","icon":"",
           "x":1,"y":2,"z":3,"dimension":"minecraft:overworld","color":-1750963,"visible":true,
           "folder":"","kind":"MANUAL","created":1762600000123,"map":null,"deleted":false,"edited":false},
          {"name":"   ","x":5,"y":6,"z":7,"dimension":"minecraft:the_end","color":4278227199,"kind":"WAYSTONE"},
          {"name":"no dimension"},
          7
        ]"#;
        let (list, skipped) = parse_file(text, &mut ids()).unwrap();
        assert_eq!(skipped, 2);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].color, 0xFFE5_484D);
        assert_eq!(list[0].created, 1_762_600_000_123);
        assert_eq!(list[0].extra.len(), 3);
        assert_eq!(list[1].name, "Waypoint");
        assert_eq!(list[1].color, 0xFF00_90FF);
        assert_eq!(list[1].kind, Kind::Other("WAYSTONE".into()));
        assert!(list[1].visible);
        assert_eq!(list[1].id.len(), 36);
        assert!(parse_file("{}", &mut ids()).is_err());
        assert!(parse_file("[", &mut ids()).is_err());
    }

    #[test]
    fn serialise_drops_temporary_pins() {
        let mut s = sample();
        s.list[0].kind = Kind::Other("TEMPORARY".into());
        let (back, _) = parse_file(&write_file(&s.list), &mut ids()).unwrap();
        assert_eq!(back.len(), s.list.len() - 1);
    }

    #[test]
    fn names_initials_and_numbers() {
        assert_eq!(initials_for("Home"), "H");
        assert_eq!(initials_for("iron farm north"), "IF");
        assert_eq!(initials_for("  "), "?");
        let s = sample();
        assert_eq!(s.list[2].name, "Waypoint 3");
        assert_eq!(s.list[2].label(), "W3");
        assert_eq!(s.find("2"), Ok(1));
        assert_eq!(s.find("#4"), Ok(3));
        assert_eq!(s.find("home"), Ok(0));
        assert_eq!(s.find("iron"), Ok(1));
        assert!(s.find("#9").is_err());
        assert!(s.find("x").is_err());
        assert!(s.find("").is_err());
    }

    #[test]
    fn deaths_keep_the_newest_three() {
        let mut rng = Rng::new(3);
        let mut s = Set::default();
        s.add(Some("Home"), (0, 0, 0), OVERWORLD, &mut rng, 0)
            .unwrap();
        for i in 0..5u64 {
            s.add_death((i as i32, 0, 0), OVERWORLD, i * 1000, &mut rng, 0);
        }
        let deaths: Vec<i32> = s
            .list
            .iter()
            .filter(|w| w.kind == Kind::Death)
            .map(|w| w.x)
            .collect();
        assert_eq!(deaths, vec![2, 3, 4]);
        assert_eq!(s.list[0].name, "Home");
        assert!(s
            .list
            .iter()
            .all(|w| w.kind != Kind::Death || w.color == DEATH_COLOR));
        assert_eq!(s.list[1].name, "Death 08:00");
        assert_eq!(s.list[1].label(), "X");
        s.list[1].initials = "D0".into();
        assert_eq!(s.list[1].label(), "X");
        s.list[1].initials = "RIP".into();
        assert_eq!(s.list[1].label(), "RIP");
        // Created numbers keep climbing past a large imported one.
        s.list[0].created = 1_762_600_000_000;
        assert_eq!(s.next_created(5), 1_762_600_000_001);
    }

    #[test]
    fn random_colours_are_bright_and_away_from_the_last() {
        let mut rng = Rng::new(42);
        let mut last = 0xFFFF_0000;
        for _ in 0..500 {
            let c = random_color(&mut rng, last);
            assert!(
                hue_distance(hue_of(c), hue_of(last)) >= 1.0 / 6.0 - 0.01,
                "{c:08x} {last:08x}"
            );
            let max = ((c >> 16) & 0xFF).max((c >> 8) & 0xFF).max(c & 0xFF);
            let min = ((c >> 16) & 0xFF).min((c >> 8) & 0xFF).min(c & 0xFF);
            assert!(max >= 203, "value {max}");
            assert!(f32::from(max as u8 - min as u8) / max as f32 >= 0.59);
            assert_eq!(c >> 24, 0xFF);
            last = c;
        }
        assert_eq!(hsv(0.0, 1.0, 1.0), 0xFFFF_0000);
        assert_eq!(hsv(1.0 / 3.0, 1.0, 1.0), 0xFF00_FF00);
        assert!((hue_of(0xFF00_00FF) - 2.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn colour_names() {
        assert_eq!(parse_color("Slate"), Some(DEATH_COLOR));
        assert_eq!(parse_color("#12a594"), Some(0xFF12_A594));
        assert_eq!(parse_color("00ff00"), Some(0xFF00_FF00));
        assert_eq!(parse_color("nope"), None);
        assert_eq!(color_name(0xFF0090FF), "sky");
        assert_eq!(color_name(0xFF123456), "#123456");
        assert_eq!(next_palette(PALETTE[15]), PALETTE[0]);
        assert_eq!(next_palette(0xFF123456), PALETTE[0]);
        assert_eq!(label_color(0xFFFFFFFF), 0x10_1418);
        assert_eq!(label_color(DEATH_COLOR), 0xFF_FFFF);
    }

    #[test]
    fn commands_parse() {
        assert_eq!(parse_command(""), Ok(Dwp::Help));
        assert_eq!(parse_command("add"), Ok(Dwp::Add(None)));
        assert_eq!(
            parse_command("add Iron Farm"),
            Ok(Dwp::Add(Some("Iron Farm".into())))
        );
        assert_eq!(
            parse_command("del \"Iron Farm\""),
            Ok(Dwp::Del("Iron Farm".into()))
        );
        assert_eq!(
            parse_command("rename 2 Iron Farm"),
            Ok(Dwp::Rename("2".into(), "Iron Farm".into()))
        );
        assert_eq!(
            parse_command("rename \"Iron Farm\" Gold Farm"),
            Ok(Dwp::Rename("Iron Farm".into(), "Gold Farm".into()))
        );
        assert_eq!(
            parse_command("colour Iron Farm #ff0000"),
            Ok(Dwp::Color("Iron Farm".into(), "#ff0000".into()))
        );
        assert_eq!(parse_command("pins off"), Ok(Dwp::Pins(Some(false))));
        assert_eq!(parse_command("within 256"), Ok(Dwp::Within(Some(256))));
        assert!(parse_command("rename 2").is_err());
        assert!(parse_command("color 2").is_err());
        assert!(parse_command("del").is_err());
        assert!(parse_command("fly").is_err());
    }

    #[test]
    fn teleport_is_only_a_tp() {
        let mut w = Waypoint::new("i".into(), "A", (10, 64, -3), OVERWORLD, PALETTE[0]);
        assert_eq!(
            teleport_line(&w, OVERWORLD).as_deref(),
            Some("tp @s 10.5 64 -2.5")
        );
        w.dimension = "minecraft:the_nether".into();
        assert_eq!(
            teleport_line(&w, OVERWORLD).as_deref(),
            Some("execute in minecraft:the_nether run tp @s 10.5 64 -2.5")
        );
        for bad in ["overworld", "a:b run kill @e", "a:B", ":x", "a:"] {
            w.dimension = bad.into();
            assert_eq!(teleport_line(&w, OVERWORLD), None, "{bad}");
        }
    }

    #[test]
    fn storage_index_keeps_45_worlds() {
        let k = world_key("sp:New World");
        assert!(k.len() == 19 && k.starts_with("wp-"));
        assert!(k[3..]
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(k, world_key("sp:New World (1)"));
        let mut idx = Index::default();
        for i in 0..MAX_WORLDS {
            let (_, dropped) = idx.open(&format!("sp:w{i}"), MAX_WORLDS);
            assert!(dropped.is_empty());
        }
        // Reopening w0 makes w1 the oldest.
        idx.open("sp:w0", MAX_WORLDS);
        let (key, dropped) = idx.open("mp:example.org", MAX_WORLDS);
        assert_eq!(key, world_key("mp:example.org"));
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].world, "sp:w1");
        let back = Index::parse(&idx.format());
        assert_eq!(back, idx);
        assert_eq!(back.worlds.len(), MAX_WORLDS);
    }

    fn square() -> View {
        View {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
            round: false,
        }
    }

    #[test]
    fn layout_pin_stands_when_it_fits() {
        assert_eq!(
            place(&square(), 50.0, 50.0, 12.0),
            Placed::Standing { x: 50.0, y: 50.0 }
        );
        // The head would poke out of the top: a badge, at the spot itself.
        match place(&square(), 50.0, 12.0, 12.0) {
            Placed::Badge { x, y, angle } => {
                assert_eq!((x, y), (50.0, 12.0));
                assert!((angle + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
            }
            p => panic!("{p:?}"),
        }
    }

    #[test]
    fn layout_badges_clamp_to_square_edges() {
        let v = square();
        let w = 12.0;
        let half_w = 50.0 - w / 2.0 - CARET;
        let half_h = 50.0 - HEAD_H / 2.0 - CARET;
        // Far east: on the right edge at the centre's height, caret east.
        let Placed::Badge { x, y, angle } = place(&v, 5000.0, 50.0, w) else {
            panic!()
        };
        assert_eq!((x, y), (50.0 + half_w, 50.0));
        assert_eq!(angle, 0.0);
        // Far north-west along the diagonal: the nearer edge wins.
        let Placed::Badge { x, y, angle } = place(&v, -950.0, -950.0, w) else {
            panic!()
        };
        assert!(half_w < half_h);
        assert!((x - (50.0 - half_w)).abs() < 1e-3 && (y - (50.0 - half_w)).abs() < 1e-3);
        assert!((angle + 3.0 * std::f32::consts::FRAC_PI_4).abs() < 1e-5);
        // Every badge, caret included, stays inside the view.
        for i in 0..64 {
            let a = i as f32 * 0.1;
            let p = place(&v, 50.0 + 900.0 * a.cos(), 50.0 + 900.0 * a.sin(), w);
            let s = pin_shape(p, w);
            for (px, py) in s.point.iter().chain(&[
                (s.head.0, s.head.1),
                (s.head.0 + s.head.2, s.head.1 + s.head.3),
            ]) {
                assert!(
                    (-0.01..=100.01).contains(px) && (-0.01..=100.01).contains(py),
                    "{a}: {px} {py}"
                );
            }
        }
    }

    #[test]
    fn layout_badges_clamp_to_the_round_rim() {
        let v = View {
            round: true,
            ..square()
        };
        let w = 10.0;
        let radius = 50.0 - w / 2.0 - CARET;
        for i in 0..32 {
            let a = i as f32 * std::f32::consts::TAU / 32.0;
            let Placed::Badge { x, y, angle } =
                place(&v, 50.0 + 400.0 * a.cos(), 50.0 + 400.0 * a.sin(), w)
            else {
                panic!()
            };
            assert!(((x - 50.0).hypot(y - 50.0) - radius).abs() < 1e-3);
            // The caret points at the waypoint: along the same angle.
            assert!(hue_distance(angle / std::f32::consts::TAU, a / std::f32::consts::TAU) < 1e-4);
        }
        // A corner of the square box is outside the circle: a badge there.
        assert!(matches!(place(&v, 8.0, 14.0, w), Placed::Badge { .. }));
        assert!(matches!(place(&v, 50.0, 60.0, w), Placed::Standing { .. }));
    }

    #[test]
    fn layout_caret_snaps_and_points_outward() {
        let step = std::f32::consts::TAU / 32.0;
        assert_eq!(snap_angle(0.4 * step), 0.0);
        assert!((snap_angle(0.6 * step) - step).abs() < 1e-6);
        let s = pin_shape(
            Placed::Badge {
                x: 90.0,
                y: 50.0,
                angle: 0.0,
            },
            10.0,
        );
        assert_eq!(s.point[2], (90.0 + 5.0 + CARET, 50.0));
        assert_eq!(s.head, (85.0, 45.0, 10.0, HEAD_H));
        let t = pin_shape(Placed::Standing { x: 20.0, y: 30.0 }, 10.0);
        assert_eq!(t.point[2], (20.0, 30.0));
        assert_eq!(t.head.1 + t.head.3, 30.0 - TAIL);
    }

    #[test]
    fn layout_degenerate_inputs_stay_finite() {
        let tiny = View {
            x: 0.0,
            y: 0.0,
            w: 4.0,
            h: 4.0,
            round: false,
        };
        for p in [
            place(&tiny, 1e9, -1e9, 30.0),
            place(&square(), f32::NAN, 3.0, 10.0),
            place(&square(), f32::INFINITY, 3.0, 10.0),
        ] {
            let Placed::Badge { x, y, angle } = p else {
                panic!("{p:?}")
            };
            assert!(x.is_finite() && y.is_finite() && angle.is_finite());
        }
    }

    #[test]
    fn layout_screen_point_matches_the_image() {
        // The window's top left texel is the box's top left; the scale is box/window.
        assert_eq!(
            screen_of((100.0, 50.0), (100.0, 50.0, 96.0), (2.0, 3.0, 48.0)),
            (2.0, 3.0)
        );
        assert_eq!(
            screen_of((196.0, 98.0), (100.0, 50.0, 96.0), (2.0, 3.0, 48.0)),
            (50.0, 27.0)
        );
    }

    #[test]
    fn on_map_rules() {
        let s = sample();
        let at = (0.0, 64.0, 0.0);
        assert!(on_map(&s.list[0], OVERWORLD, 0, at));
        assert!(!on_map(&s.list[1], OVERWORLD, 0, at)); // hidden
        assert!(!on_map(&s.list[2], OVERWORLD, 0, at)); // the Nether
        assert!(on_map(&s.list[0], OVERWORLD, 64, at));
        let mut far = s.list[0].clone();
        far.x = 5000;
        assert!(!on_map(&far, OVERWORLD, 64, at));
        far.kind = Kind::Death;
        assert!(on_map(&far, OVERWORLD, 64, at));
        assert_eq!(
            row_text(0, &s.list[0], Some((OVERWORLD, 10.5, 64.0, -19.5))),
            "1. Home  0 m"
        );
        assert_eq!(
            row_text(2, &s.list[2], Some((OVERWORLD, 0.0, 0.0, 0.0))),
            "3. Waypoint 3  (Nether)"
        );
        assert!(row_text(1, &s.list[1], None).ends_with("hidden"));
    }
}
