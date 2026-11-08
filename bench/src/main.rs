//! Times the Diorama mod's `frame` and `tick` calls in Horizon's own mod
//! runtime (the fork's `horizon-mods`), on a made-up world: rolling grass
//! hills with plants, trees and lakes, loaded 12 chunks around the player as
//! the game streams them (M7, PLAN-v2 section 5).
//!
//! ```text
//! cargo run --release -- ../target/horizon-mod/diorama.hmod           # Cranelift
//! cargo run --release --features pulley -- ../target/horizon-mod/diorama.hmod
//! ```
//!
//! Scenarios, at 60 frames and 20 ticks a second: joining (until the map is
//! filled), standing still, `/diorama refill`, and flying east at creative
//! sprint-flight speed (21.6 blocks a second) for 60 s. Each prints the frame
//! call's average, its 99th percentile and its worst, how many went over
//! 0.3 ms and 0.5 ms, and the fill-in frames the mod logs. Nothing here is
//! taken from Minecraft or Diorama: the blocks are named, their textures are
//! flat colours with a little noise.
use horizon_mods::{
    state_flags, BlockFace, BlockTable, ChunkColumns, ClientDimension, ClientTexture,
    ClientWorldView, Clock, ColumnLayer, FaceTint, Mods, Player, Runs, Snapshot, Status, NO_SPRITE,
    NO_TOP,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const RENDER_DISTANCE: i32 = 12;
const FPS: f64 = 60.0;
const SPRINT_FLIGHT: f64 = 21.6;

/// Block states: name, flags, top sprite and tint, side sprite and tint.
const STATES: &[(&str, u16, &str, FaceTint, &str, FaceTint)] = {
    use FaceTint::{Foliage as F, Grass as G, None as N, Water as W};
    let full = state_flags::FULL_TOP;
    let plant = state_flags::NO_COLLISION | state_flags::SEE_THROUGH;
    &[
        ("minecraft:air", state_flags::AIR, "", N, "", N),
        (
            "minecraft:grass_block",
            full,
            "block/grass_top",
            G,
            "block/grass_side",
            N,
        ),
        ("minecraft:dirt", full, "block/dirt", N, "block/dirt", N),
        ("minecraft:stone", full, "block/stone", N, "block/stone", N),
        ("minecraft:sand", full, "block/sand", N, "block/sand", N),
        (
            "minecraft:water",
            state_flags::WATER,
            "block/water_still",
            W,
            "block/water_still",
            W,
        ),
        (
            "minecraft:short_grass",
            plant,
            "block/tuft",
            G,
            "block/tuft",
            G,
        ),
        (
            "minecraft:poppy",
            plant,
            "block/flower",
            N,
            "block/flower",
            N,
        ),
        (
            "minecraft:oak_log",
            full,
            "block/log_top",
            N,
            "block/log",
            N,
        ),
        (
            "minecraft:oak_leaves",
            state_flags::SEE_THROUGH,
            "block/leaves",
            F,
            "block/leaves",
            F,
        ),
    ]
};
const AIR: u32 = 0;
const GRASS: u32 = 1;
const DIRT: u32 = 2;
const STONE: u32 = 3;
const SAND: u32 = 4;
const WATER: u32 = 5;
const TUFT: u32 = 6;
const FLOWER: u32 = 7;
const LOG: u32 = 8;
const LEAVES: u32 = 9;

/// Sprite name, colour, share of see-through pixels.
const SPRITES: &[(&str, u32, u32)] = &[
    ("block/grass_top", 0x9a9a9a, 0),
    ("block/grass_side", 0x7a6a48, 0),
    ("block/dirt", 0x86603e, 0),
    ("block/stone", 0x7d7d7d, 0),
    ("block/sand", 0xdbcf9e, 0),
    ("block/water_still", 0xb0b0b0, 0),
    ("block/tuft", 0xa0a0a0, 60),
    ("block/flower", 0xc0302a, 75),
    ("block/log_top", 0xa08050, 0),
    ("block/log", 0x6b5233, 0),
    ("block/leaves", 0x909090, 25),
];

fn hash(x: i32, z: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (z as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^ (h >> 12)
}

fn ground(x: i32, z: i32) -> i32 {
    let (x, z) = (f64::from(x), f64::from(z));
    let h = 66.0 + 7.0 * (x * 0.031).sin() + 6.0 * (z * 0.043).cos() + 3.0 * ((x + z) * 0.11).sin();
    h.floor() as i32
}

const SEA: i32 = 62;
/// Frames after a chunk loads at which its counter moves again.
const AFTER: [u64; 3] = [15, 40, 90];

/// The tree whose crown covers column (x, z), if any: its trunk and ground.
fn tree_near(x: i32, z: i32) -> Option<(i32, i32, i32)> {
    for dz in -2..=2 {
        for dx in -2..=2 {
            let (tx, tz) = (x + dx, z + dz);
            if tx.rem_euclid(9) == 4 && tz.rem_euclid(11) == 5 && hash(tx, tz) % 3 != 0 {
                let g = ground(tx, tz);
                if g > SEA {
                    return Some((tx, tz, g));
                }
            }
        }
    }
    None
}

fn block(x: i32, y: i32, z: i32) -> u32 {
    let g = ground(x, z);
    if let Some((tx, tz, tg)) = tree_near(x, z) {
        let trunk = tx == x && tz == z;
        if trunk && y > tg && y <= tg + 5 {
            return LOG;
        }
        let r = (x - tx).abs().max((z - tz).abs());
        if (y == tg + 6 && r <= 1) || ((tg + 4..=tg + 5).contains(&y) && r <= 2 && !trunk) {
            return LEAVES;
        }
    }
    if y > g {
        if y <= SEA {
            return WATER;
        }
        if y == g + 1 && g > SEA {
            return match hash(x, z) % 13 {
                0..=3 => TUFT,
                4 => FLOWER,
                _ => AIR,
            };
        }
        return AIR;
    }
    if g <= SEA + 1 && y >= g - 2 {
        SAND
    } else if y == g {
        GRASS
    } else if y >= g - 3 {
        DIRT
    } else {
        STONE
    }
}

/// What the game has loaded: every chunk within the render distance, each
/// with the change counter it got when it came in (or went out).
#[derive(Default)]
struct Loaded {
    centre: Option<(i32, i32)>,
    revisions: BTreeMap<(i32, i32), u64>,
    unloaded: BTreeMap<(i32, i32), u64>,
    revision: u64,
    /// Chunks to touch again at a frame: in the game a chunk's counter moves
    /// a few times after it loads (its light and its neighbours finish), so
    /// flying reads each chunk about 3.7 times (M7's game runs).
    later: BTreeMap<u64, Vec<(i32, i32)>>,
    frame: u64,
}
impl Loaded {
    fn follow(&mut self, centre: (i32, i32), frame: u64) {
        self.frame = frame;
        if let Some(touch) = self.later.remove(&frame) {
            for c in touch {
                if let Some(r) = self.revisions.get_mut(&c) {
                    self.revision += 1;
                    *r = self.revision;
                }
            }
        }
        if self.centre == Some(centre) {
            return;
        }
        self.centre = Some(centre);
        let near =
            |c: &(i32, i32)| (c.0 - centre.0).abs().max((c.1 - centre.1).abs()) <= RENDER_DISTANCE;
        let gone: Vec<_> = self
            .revisions
            .keys()
            .copied()
            .filter(|c| !near(c))
            .collect();
        for c in gone {
            self.revisions.remove(&c);
            self.revision += 1;
            self.unloaded.insert(c, self.revision);
        }
        for cz in centre.1 - RENDER_DISTANCE..=centre.1 + RENDER_DISTANCE {
            for cx in centre.0 - RENDER_DISTANCE..=centre.0 + RENDER_DISTANCE {
                if !self.revisions.contains_key(&(cx, cz)) {
                    self.revision += 1;
                    self.revisions.insert((cx, cz), self.revision);
                    self.unloaded.remove(&(cx, cz));
                    if self.frame > 0 {
                        for d in AFTER {
                            self.later.entry(self.frame + d).or_default().push((cx, cz));
                        }
                    }
                }
            }
        }
    }
}

/// Columns made once per chunk, layer and depth (before the timing starts),
/// so the game's side of `columns` costs a copy, as the game's own does
/// about as much.
type Made = Arc<Mutex<BTreeMap<((i32, i32), Option<i32>, u8), ChunkColumns>>>;

#[derive(Clone)]
struct View(Arc<Mutex<Loaded>>, Made);
impl View {
    fn make(&self, (cx, cz): (i32, i32), layer: ColumnLayer, depth: u8) -> ChunkColumns {
        let key = (
            (cx, cz),
            match layer {
                ColumnLayer::Surface => None,
                ColumnLayer::Below(y) => Some(y),
            },
            depth,
        );
        if let Some(c) = self.1.lock().unwrap().get(&key) {
            return c.clone();
        }
        let below = key.1.unwrap_or(i32::MAX);
        let d = usize::from(depth);
        let mut out = ChunkColumns {
            revision: 0,
            tops: vec![NO_TOP; 256],
            states: vec![AIR; 256 * d],
            water: vec![0; 256],
            light: vec![0xF0; 256],
            tints: [0x91_BD59, 0x77_AB2F, 0x3F_76E4].repeat(256),
        };
        for i in 0..256 {
            let (x, z) = (cx * 16 + (i % 16) as i32, cz * 16 + (i / 16) as i32);
            let mut top = (ground(x, z) + 8).min(below);
            while top > -64 && block(x, top, z) == AIR {
                top -= 1;
            }
            out.tops[i] = top as i16;
            for k in 0..d {
                out.states[i * d + k] = block(x, top - k as i32, z);
            }
            out.water[i] = (0..64)
                .take_while(|&k| block(x, top - k, z) == WATER)
                .count() as u8;
        }
        self.1.lock().unwrap().insert(key, out.clone());
        out
    }
}
impl ClientWorldView for View {
    fn dimension(&self) -> Option<ClientDimension> {
        Some(ClientDimension {
            id: "minecraft:overworld".into(),
            min_y: -64,
            height: 384,
            ceiling: false,
        })
    }
    fn block_table(&mut self) -> BlockTable {
        let sprite = |name: &str| match SPRITES.iter().position(|s| s.0 == name) {
            Some(i) => i as u32,
            None => NO_SPRITE,
        };
        let n = STATES.len();
        BlockTable {
            version: 1,
            blocks: STATES.iter().map(|s| s.0.to_owned()).collect(),
            sprites: SPRITES.iter().map(|s| s.0.to_owned()).collect(),
            state_block: (0..n as u16).collect(),
            state_flags: STATES.iter().map(|s| s.1).collect(),
            state_top: (STATES.iter())
                .map(|s| BlockFace {
                    sprite: sprite(s.2),
                    tint: s.3,
                })
                .collect(),
            state_side: (STATES.iter())
                .map(|s| BlockFace {
                    sprite: sprite(s.4),
                    tint: s.5,
                })
                .collect(),
            constant_tint: vec![0; n],
            map_color: vec![0x7f_b238; n],
        }
    }
    fn texture(&mut self, name: &str) -> Option<ClientTexture> {
        let &(_, rgb, holes) = SPRITES.iter().find(|s| s.0 == name)?;
        let mut rgba = Vec::with_capacity(16 * 16 * 4);
        for i in 0..256 {
            let h = hash(i, rgb as i32);
            let k = 0.85 + f64::from(h % 30) / 100.0;
            for shift in [16, 8, 0] {
                rgba.push((f64::from((rgb >> shift) & 0xff) * k).min(255.0) as u8);
            }
            rgba.push(if h % 100 < holes { 0 } else { 255 });
        }
        Some(ClientTexture {
            width: 16,
            height: 16,
            rgba,
        })
    }
    fn columns(
        &mut self,
        chunk: (i32, i32),
        layer: ColumnLayer,
        depth: u8,
    ) -> Option<ChunkColumns> {
        let revision = *self.0.lock().unwrap().revisions.get(&chunk)?;
        let mut out = self.make(chunk, layer, depth);
        out.revision = revision;
        Some(out)
    }
    fn changes(&mut self, since: u64) -> (u64, Vec<(i32, i32)>) {
        let l = self.0.lock().unwrap();
        let moved = (l.revisions.iter().chain(&l.unloaded))
            .filter(|(_, &r)| r > since)
            .map(|(&c, _)| c)
            .collect();
        (l.revision, moved)
    }
    fn light(&self, _x: i32, _y: i32, _z: i32) -> Option<u8> {
        Some(0xF0)
    }
    fn biome(&self, _x: i32, _y: i32, _z: i32) -> Option<String> {
        Some("minecraft:plains".into())
    }
}

/// The mod's log lines, kept for the fill-in lines.
static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
struct Keep;
impl log::Log for Keep {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        let line = r.args().to_string();
        if std::env::var_os("BENCH_LOG").is_some() {
            println!("    {line}");
        }
        LINES.lock().unwrap().push(line);
    }
    fn flush(&self) {}
}

struct Bench {
    mods: Mods,
    view: View,
    player: Player,
    frame: u64,
    tick: u64,
}

#[derive(Default)]
struct Times {
    frames: Vec<Duration>,
    ticks: Vec<Duration>,
}

impl Bench {
    fn snapshot(&mut self) {
        let centre = (
            (self.player.x.floor() as i32) >> 4,
            (self.player.z.floor() as i32) >> 4,
        );
        self.view.0.lock().unwrap().follow(centre, self.frame);
        self.mods.set_snapshot(Snapshot {
            player: Some(self.player.clone()),
            clock: Some(Clock {
                game_time: self.tick,
                day_time: 6000,
            }),
            // A 1280 x 720 window at GUI scale 2.
            gui_size: (640.0, 360.0),
            world_id: Some("sp:Bench".into()),
            ..Snapshot::default()
        });
    }
    /// One frame (and a tick every third): its times go into `t`.
    fn step(&mut self, t: &mut Times) {
        self.snapshot();
        self.mods.lend_client_world(Box::new(self.view.clone()));
        if self.frame % 3 == 0 {
            self.tick += 1;
            let started = Instant::now();
            self.mods.tick(self.tick);
            t.ticks.push(started.elapsed());
        }
        let started = Instant::now();
        self.mods.frame((1.0 / FPS) as f32);
        t.frames.push(started.elapsed());
        self.frame += 1;
        self.mods.take_client_world();
        self.mods.take_canvas_uploads(false);
        self.mods.drain();
        if let Status::Failed(e) = &self.mods.entries()[0].status {
            panic!("the mod stopped: {e}");
        }
    }
    /// Frames until the mod logs that the whole mosaic is filled (at most `cap`).
    fn until_filled(&mut self, cap: u32, t: &mut Times) -> (Option<String>, Option<String>) {
        let from = LINES.lock().unwrap().len();
        let (mut visible, mut all) = (None, None);
        for _ in 0..cap {
            self.step(t);
            for line in &LINES.lock().unwrap()[from..] {
                if line.contains("fill: the box") {
                    visible.get_or_insert_with(|| line.clone());
                }
                if line.contains("fill: all") {
                    all.get_or_insert_with(|| line.clone());
                }
            }
            if all.is_some() {
                break;
            }
        }
        (visible, all)
    }
}

fn report(name: &str, t: &Times) {
    let us = |d: &Duration| d.as_secs_f64() * 1e6;
    let line = |what: &str, v: &[Duration]| {
        if v.is_empty() {
            return;
        }
        let mut s: Vec<f64> = v.iter().map(us).collect();
        s.sort_by(f64::total_cmp);
        let avg = s.iter().sum::<f64>() / s.len() as f64;
        let p99 = s[(s.len() * 99 / 100).min(s.len() - 1)];
        let over = |l: f64| s.iter().filter(|&&x| x > l).count();
        let worst = (v.iter().enumerate())
            .max_by_key(|(_, d)| **d)
            .map_or(0, |(i, _)| i);
        println!(
            "  {name:<8} {what:<6} {:>5} calls: avg {avg:>6.0} µs, p99 {p99:>6.0} µs, max {:>6.0} µs, \
             {} over 300 µs, {} over 500 µs (worst: call {worst})",
            s.len(),
            s[s.len() - 1],
            over(300.0),
            over(500.0),
        );
    };
    line("frame", &t.frames);
    line("tick", &t.ticks);
}

/// The fill-in line's frame count, as seconds at 60 fps.
fn fill(line: &Option<String>) -> String {
    let Some(l) = line else {
        return "not filled".into();
    };
    let frames: Option<f64> = l
        .split(", ")
        .last()
        .and_then(|f| f.split_whitespace().next())
        .and_then(|n| n.parse().ok());
    match frames {
        Some(n) => format!("{n:.0} frames = {:.2} s at 60 fps", n / FPS),
        None => l.clone(),
    }
}

/// `--native`: the mod's bake (read, paint, upload) of the 625 chunks
/// around the origin, run natively with diorama-core, phase by phase.
fn native() {
    use diorama_core::{bake::Chunk, looks, style, Mosaic, Painter, Projection};
    let mut view = View(Arc::new(Mutex::new(Loaded::default())), Made::default());
    view.0.lock().unwrap().follow((0, 0), 0);
    let t = view.block_table();
    let face = |f: &BlockFace| looks::Face {
        sprite: f.sprite,
        tint: match f.tint {
            FaceTint::Grass => looks::Tint::Grass,
            FaceTint::Foliage => looks::Tint::Foliage,
            FaceTint::Water => looks::Tint::Water,
            _ => looks::Tint::None,
        },
    };
    let table = looks::Table {
        version: 1,
        state_top: t.state_top.iter().map(face).collect(),
        state_side: t.state_side.iter().map(face).collect(),
        blocks: t.blocks,
        sprites: t.sprites,
        state_block: t.state_block,
        state_flags: t.state_flags,
        constant_tint: t.constant_tint,
        map_color: t.map_color,
    };
    let averages: Vec<_> = (table.sprites.iter())
        .map(|s| {
            view.texture(s)
                .map(|t| looks::average(t.width, t.height, &t.rgba))
        })
        .collect();
    let mut b = looks::Builder::new(&table);
    while !b.step(&table, &averages, 1 << 20) {}
    let painter = Painter::new(b.finish(&table), style::DEFAULT);
    let depth = std::env::var("BENCH_DEPTH")
        .ok()
        .and_then(|d| d.parse().ok())
        .unwrap_or(12);
    let side = std::env::var("BENCH_SIDE")
        .ok()
        .and_then(|d| d.parse().ok())
        .unwrap_or(512);
    let reps: u32 = std::env::var("BENCH_REPS")
        .ok()
        .and_then(|d| d.parse().ok())
        .unwrap_or(20);
    let keys: Vec<_> = (-12..=12)
        .flat_map(|z| (-12..=12).map(move |x| (x, z)))
        .collect();
    let cols: Vec<_> = keys
        .iter()
        .map(|&k| view.columns(k, ColumnLayer::Surface, depth).unwrap())
        .collect();
    let (mut read, mut paint, mut upload) = (Duration::ZERO, Duration::ZERO, Duration::ZERO);
    let (mut steps, mut bytes) = (0u64, 0usize);
    let (mut prep, mut set) = (Duration::ZERO, Duration::ZERO);
    let mut stats_last = diorama_core::mosaic::Stats::default();
    for _ in 0..reps {
        let mut m = Mosaic::new(Projection::new(style::DEFAULT.pitch_degrees), side);
        m.recentre(8.0, m.projection().v(70.0, 8.0));
        let started = Instant::now();
        for (k, c) in keys.iter().zip(&cols) {
            let c = c.clone();
            let mut chunk = Chunk::new(c.tops, c.states, depth);
            chunk.water = c.water;
            chunk.light = c.light;
            chunk.tints = c.tints;
            let p0 = Instant::now();
            painter.prepare(&mut chunk);
            prep += p0.elapsed();
            let p0 = Instant::now();
            m.set_chunk(*k, Some(chunk));
            set += p0.elapsed();
        }
        read += started.elapsed();
        let started = Instant::now();
        let rows = std::env::var("BENCH_ROWS")
            .ok()
            .and_then(|d| d.parse().ok())
            .unwrap_or(64);
        while m.paint_step(&painter, 8.0, rows) {
            steps += 1;
        }
        paint += started.elapsed();
        let started = Instant::now();
        loop {
            let n = m.take_upload(8 << 10, painter.style.outline, |_, _, _, _, _| {});
            if n == 0 {
                break;
            }
            bytes += n;
        }
        upload += started.elapsed();
        stats_last = m.stats;
    }
    let per = |d: Duration| d.as_secs_f64() * 1e3 / f64::from(reps);
    println!(
        "native, {side} side, depth {depth}: read {:.2} ms ({:.1} µs a chunk), paint {:.2} ms \
         ({} steps, {:.0} ns a texel), upload {:.2} ms ({} KiB)",
        per(read),
        per(read) * 1e3 / keys.len() as f64,
        per(paint),
        steps / u64::from(reps),
        per(paint) * 1e6 / f64::from(side * side),
        per(upload),
        bytes / 1024 / reps as usize,
    );
    println!(
        "  of the read: prepare {:.2} ms, set-chunk {:.2} ms",
        per(prep),
        per(set)
    );
    let st = stats_last;
    println!(
        "  painting: {} jobs, {} columns, {} texels",
        st.jobs, st.columns, st.texels
    );
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--native") {
        return native();
    }
    let hmod = std::env::args()
        .nth(1)
        .expect("usage: diorama-bench <diorama.hmod>");
    let hmod = std::fs::canonicalize(&hmod).expect("the .hmod");
    log::set_logger(&Keep).unwrap();
    log::set_max_level(log::LevelFilter::Info);
    // The mod's storage goes under the working directory: a scratch one.
    let dir = std::env::temp_dir().join(format!("diorama-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_current_dir(&dir).unwrap();
    let engine = if cfg!(feature = "pulley") {
        "Pulley"
    } else {
        "Cranelift"
    };
    println!("diorama-bench: {} on {engine}", hmod.display());

    let mut b = Bench {
        mods: Mods::default(),
        view: View(Arc::new(Mutex::new(Loaded::default())), Made::default()),
        player: Player {
            x: 8.5,
            y: 90.0,
            z: 8.5,
            yaw: -90.0,
            pitch: 10.0,
            health: 20.0,
            food: 20,
            dimension: "minecraft:overworld".into(),
        },
        frame: 0,
        tick: 0,
    };
    // The world along the flight, made before anything is timed.
    let made = Instant::now();
    let reach = RENDER_DISTANCE + 1;
    let east = ((60.0 * SPRINT_FLIGHT) as i32 >> 4) + reach;
    for cz in -reach..=reach {
        for cx in -reach..=east {
            // The depth the mod reads (BENCH_DEPTH, else 12).
            let depth = std::env::var("BENCH_DEPTH")
                .ok()
                .and_then(|d| d.parse().ok())
                .unwrap_or(12);
            b.view.make((cx, cz), ColumnLayer::Surface, depth);
        }
    }
    println!("  world made in {:.1} s", made.elapsed().as_secs_f64());
    b.snapshot();
    b.mods.lend_client_world(Box::new(b.view.clone()));
    let started = Instant::now();
    b.mods.load_into(Runs::Client, &[hmod], |_| true, |_| false);
    b.mods.take_client_world();
    if let Status::Failed(e) = &b.mods.entries()[0].status {
        panic!("the mod didn't start: {e}");
    }
    println!("  init {:.0} ms", started.elapsed().as_secs_f64() * 1e3);

    let mut t = Times::default();
    let (visible, all) = b.until_filled(6000, &mut t);
    report("join", &t);
    println!(
        "  join     fill: box {}; all {}",
        fill(&visible),
        fill(&all)
    );

    let mut t = Times::default();
    for _ in 0..600 {
        b.step(&mut t);
    }
    report("stand", &t);

    b.mods.lend_client_world(Box::new(b.view.clone()));
    b.mods.command("diorama refill");
    b.mods.take_client_world();
    let mut t = Times::default();
    let (visible, all) = b.until_filled(6000, &mut t);
    report("refill", &t);
    println!(
        "  refill   fill: box {}; all {}",
        fill(&visible),
        fill(&all)
    );

    // Moving inside what is baked: 20 s back and forth over 40 blocks at
    // flight speed, once the whole mosaic is filled (BENCH_SETTLE=1 waits
    // for it first): the cost of the box following the player alone.
    if std::env::var_os("BENCH_SETTLE").is_some() {
        let mut t = Times::default();
        let (_, all) = b.until_filled(12_000, &mut t);
        println!("  settle   fill: all {}", fill(&all));
        let mut t = Times::default();
        let x0 = b.player.x;
        for i in 0..(20.0 * FPS) as u32 {
            let phase = f64::from(i) * SPRINT_FLIGHT / FPS / 40.0;
            b.player.x = x0 + 20.0 * (phase * std::f64::consts::PI).sin();
            b.step(&mut t);
        }
        report("hover", &t);
        b.player.x = x0;
    }

    let mut t = Times::default();
    let frames = (60.0 * FPS) as u32;
    for _ in 0..frames {
        b.player.x += SPRINT_FLIGHT / FPS;
        b.step(&mut t);
    }
    report("fly", &t);
    let lines = LINES.lock().unwrap();
    if let Some(l) = lines.iter().rev().find(|l| l.starts_with("bake:")) {
        println!("  last bake line: {l}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
