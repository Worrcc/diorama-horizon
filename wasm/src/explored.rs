//! Saved final texels; independent of chunk acquisition/prepare (including M3).
use super::*;
use diorama_core::explored::{self as codec, Encoder, BYTES, SIDE as TILE};
use std::collections::BTreeMap;

const ROOT: &str = "map_index_v1";
const PAGES: usize = 64;
const LIMIT: u64 = 900 << 20;
const KEY_LIMIT: usize = 60_000;
/// Tiles kept in memory (16 KiB each): the minimap's and the fullscreen
/// map's together, so neither evicts the other's.
const CACHE: usize = 640;
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    ns: [u8; 16],
    x: i32,
    y: i32,
}
impl Key {
    fn page(self) -> usize {
        let mut h = u32::from_le_bytes(self.ns[..4].try_into().unwrap());
        h ^= (self.x as u32).wrapping_mul(0x9e3779b9);
        h ^= (self.y as u32).wrapping_mul(0x85ebca6b);
        (h ^ (h >> 16)) as usize % PAGES
    }
    fn name(self) -> String {
        let mut name = String::from("mt1.");
        use std::fmt::Write;
        for b in self.ns {
            let _ = write!(name, "{b:02x}");
        }
        let _ = write!(name, ".{:08x}{:08x}", self.x as u32, self.y as u32);
        name
    }
}
struct Record {
    bytes: u32,
    used: u64,
}
struct Tile {
    rgba: Vec<u8>,
    dirty: bool,
    generation: u64,
    used: u64,
    changed: f64,
    /// Made blank by a capture while its saved copy was still on disk: that
    /// copy is merged under it (in `step`, one a frame) before it is saved.
    base_pending: bool,
}
struct Saving {
    key: Key,
    generation: u64,
    encoder: Encoder,
}

struct Context {
    world: String,
    dimension: String,
    cut: Option<i32>,
    flat: bool,
    ids: [[u8; 16]; 2],
}

pub type TileId = ([u8; 16], i32, i32, u32);
struct ScanLayer {
    ns: [u8; 16],
    tpb: u32,
    x0: i32,
    x1: i32,
    y1: i32,
    x: i32,
    y: i32,
}
pub struct Scan {
    layers: Vec<ScanLayer>,
}
impl Scan {
    /// Examine at most 64 grid cells, even at the widest zoom or sparse views.
    pub fn step(&mut self, saved: &Saved) -> (bool, Option<TileId>) {
        for _ in 0..64 {
            let Some(layer) = self.layers.last_mut() else {
                return (true, None);
            };
            if layer.y > layer.y1 {
                self.layers.pop();
                continue;
            }
            let key = Key {
                ns: layer.ns,
                x: layer.x,
                y: layer.y,
            };
            let tpb = layer.tpb;
            layer.x += 1;
            if layer.x > layer.x1 {
                layer.x = layer.x0;
                layer.y += 1;
            }
            if saved.pages[key.page()].contains_key(&key) || saved.tiles.contains_key(&key) {
                return (false, Some((key.ns, key.x, key.y, tpb)));
            }
        }
        (false, None)
    }
}

pub struct Saved {
    pages: [BTreeMap<Key, Record>; PAGES],
    lru: BTreeSet<(u64, Key)>,
    bytes: u64,
    serial: u64,
    tiles: BTreeMap<Key, Tile>,
    dirty_count: usize,
    pub world: Option<String>,
    context: Option<Context>,
    saving: Option<Saving>,
    index_dirty: BTreeSet<usize>,
    stopped: bool,
    root: bool,
    force: bool,
    /// Restores are restarted after a shift, world/layer or detail change.
    anchor: Option<([u8; 16], i32, i32, u32)>,
    restore: Vec<(Key, u32)>,
    /// Captured tiles waiting for their saved copy to merge under them.
    merges: BTreeSet<Key>,
    restored: bool,
    hold: bool,
    pub written: u64,
    pub encode_work: u64,
    pub encode_pixels: u64,
}
impl Saved {
    pub fn new() -> Self {
        let root = storage::get(ROOT).as_deref() == Some(b"DIM1");
        let mut s = Self {
            pages: std::array::from_fn(|_| BTreeMap::new()),
            lru: BTreeSet::new(),
            bytes: 0,
            serial: 0,
            tiles: BTreeMap::new(),
            dirty_count: 0,
            world: game::world_id(),
            context: None,
            saving: None,
            index_dirty: BTreeSet::new(),
            stopped: false,
            root,
            force: false,
            anchor: None,
            restore: Vec::new(),
            merges: BTreeSet::new(),
            restored: false,
            hold: false,
            written: 0,
            encode_work: 0,
            encode_pixels: 0,
        };
        if root {
            for page in 0..PAGES {
                if let Some(raw) = storage::get(&Self::page_name(page)) {
                    if !raw.len().is_multiple_of(36) {
                        continue;
                    }
                    for r in raw.as_chunks::<36>().0.iter().take(KEY_LIMIT) {
                        let key = Key {
                            ns: r[..16].try_into().unwrap(),
                            x: i32::from_le_bytes(r[16..20].try_into().unwrap()),
                            y: i32::from_le_bytes(r[20..24].try_into().unwrap()),
                        };
                        let bytes = u32::from_le_bytes(r[24..28].try_into().unwrap());
                        let used = u64::from_le_bytes(r[28..36].try_into().unwrap());
                        if key.page() != page
                            || bytes as usize > BYTES + 1
                            || s.lru.len() >= KEY_LIMIT
                        {
                            continue;
                        }
                        s.bytes += bytes as u64;
                        s.serial = s.serial.max(used);
                        s.pages[page].insert(key, Record { bytes, used });
                        s.lru.insert((used, key));
                    }
                }
            }
        }
        s
    }
    fn page_name(page: usize) -> String {
        format!("map_index_v1_{page:02}")
    }
    pub fn sync(&mut self, p: &game::Player, cut: Option<i32>, flat: bool) {
        let Some(world) = self.world.as_deref() else {
            self.context = None;
            return;
        };
        if self.context.as_ref().is_some_and(|c| {
            c.world == world && c.dimension == p.dimension && c.cut == cut && c.flat == flat
        }) {
            return;
        }
        self.context = Some(Context {
            world: world.to_owned(),
            dimension: p.dimension.clone(),
            cut,
            flat,
            ids: [
                codec::namespace(world, &p.dimension, cut, 1, flat),
                codec::namespace(world, &p.dimension, cut, 2, flat),
            ],
        });
    }
    fn namespace(
        &self,
        p: &game::Player,
        cut: Option<i32>,
        tpb: u32,
        flat: bool,
    ) -> Option<[u8; 16]> {
        if let Some(c) = &self.context {
            if c.cut == cut && c.flat == flat && c.dimension == p.dimension {
                return c.ids.get(tpb as usize - 1).copied();
            }
        }
        Some(codec::namespace(
            self.world.as_deref()?,
            &p.dimension,
            cut,
            tpb,
            flat,
        ))
    }

    fn touch(&mut self, key: Key) {
        self.serial += 1;
        if let Some(r) = self.pages[key.page()].get_mut(&key) {
            self.lru.remove(&(r.used, key));
            r.used = self.serial;
            self.lru.insert((r.used, key));
            self.index_dirty.insert(key.page());
        }
    }
    fn room(&mut self) -> bool {
        if self.tiles.len() < CACHE {
            return true;
        }
        let oldest = self
            .tiles
            .iter()
            .filter(|(k, t)| {
                !t.dirty && !t.base_pending && self.saving.as_ref().is_none_or(|s| s.key != **k)
            })
            .min_by_key(|(_, t)| t.used)
            .map(|(k, _)| *k);
        if let Some(key) = oldest {
            self.tiles.remove(&key);
            true
        } else {
            self.force = true;
            false
        }
    }
    fn load(&mut self, key: Key, empty: bool) -> bool {
        self.serial += 1;
        if let Some(t) = self.tiles.get_mut(&key) {
            t.used = self.serial;
            if t.base_pending {
                self.merge(key);
            }
            return true;
        }
        if !self.room() {
            return false;
        }
        let rgba = if self.pages[key.page()].contains_key(&key) {
            storage::get(&key.name()).and_then(|v| codec::decode(&v))
        } else {
            None
        };
        let Some(rgba) = rgba.or_else(|| empty.then(|| vec![0; BYTES])) else {
            return false;
        };
        self.tiles.insert(
            key,
            Tile {
                rgba,
                dirty: false,
                generation: 0,
                used: self.serial,
                changed: 0.0,
                base_pending: false,
            },
        );
        self.touch(key);
        true
    }
    pub fn invalidate(&mut self) {
        self.anchor = None;
        self.restore.clear();
    }
    pub fn can_capture(&mut self) -> bool {
        if self.stopped {
            return true;
        }
        if self.dirty_count >= CACHE - 4 {
            self.force = true;
            false
        } else {
            true
        }
    }

    pub fn remember_detail(&mut self) {
        if !self.stopped {
            self.set("map_detail", b"1");
        }
    }
    pub fn flush(&mut self) {
        self.force = true;
    }
    /// Prioritise the box; every restore is before that frame's first chunk read.
    pub fn restore_hud(
        &mut self,
        m: &mut Mosaic,
        p: &game::Player,
        cut: Option<i32>,
        flat: bool,
    ) -> bool {
        self.hold = false;
        let tpb = m.texels_per_block();
        let Some(ns) = self.namespace(p, cut, tpb, flat) else {
            return false;
        };
        let (u, v) = m.anchor();
        let anchor = (ns, u, v, m.side());
        if self.anchor != Some(anchor) {
            self.anchor = Some(anchor);
            self.restore.clear();
            self.restored = false;
            for density in [3 - tpb, tpb] {
                let Some(id) = self.namespace(p, cut, density, flat) else {
                    continue;
                };
                let x0 = (u * density as i32).div_euclid(TILE as i32);
                let y0 = (v * density as i32).div_euclid(TILE as i32);
                let x1 =
                    ((u + (m.side() / tpb) as i32) * density as i32 - 1).div_euclid(TILE as i32);
                let y1 =
                    ((v + (m.side() / tpb) as i32) * density as i32 - 1).div_euclid(TILE as i32);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let key = Key { ns: id, x, y };
                        if self.pages[key.page()].contains_key(&key)
                            || self.tiles.contains_key(&key)
                        {
                            self.restore.push((key, density));
                        }
                    }
                }
            }
            let (cx, cy) = m.texel_of(p.x, m.projection().v(p.y, p.z));
            self.restore.sort_by_key(|(k, d)| {
                let x = k.x * TILE as i32 / *d as i32 - u - cx as i32 / tpb as i32;
                let y = k.y * TILE as i32 / *d as i32 - v - cy as i32 / tpb as i32;
                std::cmp::Reverse(x.abs().max(y.abs()))
            });
        }
        if let Some((key, density)) = self.restore.pop() {
            self.hold = true;
            if self.load(key, false) {
                m.restore_tile(
                    (key.x * TILE as i32, key.y * TILE as i32),
                    density,
                    TILE,
                    &self.tiles[&key].rgba,
                );
                self.restored = true;
            }
            // Restore the box before baking, but distant tiles continue lazily.
            return true;
        }
        false
    }
    pub fn restoring(&self) -> bool {
        self.hold || !self.restore.is_empty()
    }
    pub fn box_restored(&self, m: &Mosaic, (x, y, w, h): (f32, f32, f32, f32)) -> bool {
        let (u, v) = m.anchor();
        let t = m.texels_per_block() as f32;
        let a = (u as f32 + x / t, v as f32 + y / t);
        let b = (a.0 + w / t, a.1 + h / t);
        !self.restore.iter().any(|(k, d)| {
            let size = TILE as f32 / *d as f32;
            let p = (k.x as f32 * size, k.y as f32 * size);
            p.0 < b.0 && p.1 < b.1 && p.0 + size > a.0 && p.1 + size > a.1
        })
    }
    pub fn has_restored(&self) -> bool {
        self.restored
    }
    /// A tile to capture into, without reading the disk mid-upload: a tile
    /// saved earlier but not in memory starts blank and its saved copy is
    /// merged under it later (`step`).
    fn cached_for_capture(&mut self, key: Key) -> bool {
        self.serial += 1;
        if let Some(t) = self.tiles.get_mut(&key) {
            t.used = self.serial;
            return true;
        }
        if !self.room() {
            return false;
        }
        let on_disk = self.pages[key.page()].contains_key(&key);
        self.tiles.insert(
            key,
            Tile {
                rgba: vec![0; BYTES],
                dirty: false,
                generation: 0,
                used: self.serial,
                changed: 0.0,
                base_pending: on_disk,
            },
        );
        if on_disk {
            self.merges.insert(key);
        }
        self.touch(key);
        true
    }

    /// Lays a tile's saved copy under what was captured into it since.
    fn merge(&mut self, key: Key) {
        self.merges.remove(&key);
        let saved = storage::get(&key.name()).and_then(|v| codec::decode(&v));
        let Some(t) = self.tiles.get_mut(&key) else {
            return;
        };
        t.base_pending = false;
        let Some(saved) = saved.filter(|v| v.len() == t.rgba.len()) else {
            return;
        };
        for (px, old) in t
            .rgba
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(saved.as_chunks::<4>().0)
        {
            if px[3] == 0 {
                *px = *old;
            }
        }
    }

    /// Saves texels as they are uploaded (outlines and the saved background
    /// already in): `rgba` is a `w` x `h` rect at texel `(x, y)` of a mosaic
    /// anchored at block `anchor` with `tpb` texels per block. No texel is
    /// computed twice: what the upload built is what is kept.
    #[allow(clippy::too_many_arguments)]
    pub fn capture_rect(
        &mut self,
        anchor: (i32, i32),
        tpb: u32,
        (x, y, w, h): (u32, u32, u32, u32),
        rgba: &[u8],
        p: &game::Player,
        cut: Option<i32>,
        flat: bool,
        now: f64,
    ) {
        if self.stopped || rgba.len() < (w * h * 4) as usize {
            return;
        }
        let Some(ns) = self.namespace(p, cut, tpb, flat) else {
            return;
        };
        let (ox, oy) = (anchor.0 * tpb as i32, anchor.1 * tpb as i32);
        let row_bytes = w as usize * 4;
        for r in 0..h {
            let wy = oy + (y + r) as i32;
            let line = &rgba[r as usize * row_bytes..(r as usize + 1) * row_bytes];
            let mut i = 0u32;
            while i < w {
                let wx = ox + (x + i) as i32;
                let key = Key {
                    ns,
                    x: wx.div_euclid(TILE as i32),
                    y: wy.div_euclid(TILE as i32),
                };
                let tx = wx.rem_euclid(TILE as i32) as usize;
                let n = (w - i).min(TILE - tx as u32) as usize * 4;
                let src = &line[i as usize * 4..i as usize * 4 + n];
                if (self.tiles.contains_key(&key) || src.iter().any(|&v| v != 0))
                    && self.cached_for_capture(key)
                {
                    let t = self.tiles.get_mut(&key).unwrap();
                    let at = (wy.rem_euclid(TILE as i32) as usize * TILE as usize + tx) * 4;
                    if t.rgba[at..at + n] != *src {
                        t.rgba[at..at + n].copy_from_slice(src);
                        if !t.dirty {
                            self.dirty_count += 1;
                            t.changed = now;
                        }
                        t.dirty = true;
                        t.generation += 1;
                    }
                }
                i += n as u32 / 4;
            }
        }
    }
    fn set(&mut self, key: &str, data: &[u8]) -> bool {
        if let Err(e) = storage::set(key, data) {
            self.stopped = true;
            horizon_mod_sdk::warn(&format!("explored map: saving stopped: {e}"));
            false
        } else {
            self.written += data.len() as u64;
            true
        }
    }
    /// One bounded unit per frame, also while outside the world. No leave loop.
    pub fn step(&mut self, now: f64, idle: bool, granted: u32) {
        if self.stopped {
            return;
        }
        if self.bytes > LIMIT || self.lru.len() >= KEY_LIMIT {
            if let Some((used, key)) = self.lru.pop_first() {
                if let Some(r) = self.pages[key.page()].remove(&key) {
                    self.bytes -= r.bytes as u64;
                }
                storage::delete(&key.name());
                if self.tiles.remove(&key).is_some_and(|t| t.dirty) {
                    self.dirty_count -= 1;
                }
                self.index_dirty.insert(key.page());
                let _ = used;
            }
            return;
        }
        if let Some(&key) = self.merges.first() {
            self.merge(key);
            return;
        }
        if let Some(mut job) = self.saving.take() {
            let before = canvas::work_left();
            let done = job.encoder.step(256);
            self.encode_work += before.saturating_sub(canvas::work_left()) * granted as u64;
            self.encode_pixels += 256;
            if !done {
                self.saving = Some(job);
                return;
            }
            let data = job.encoder.finish();
            if !self.set(&job.key.name(), &data) {
                return;
            }
            self.touch(job.key);
            let old = self.pages[job.key.page()].insert(
                job.key,
                Record {
                    bytes: data.len() as u32,
                    used: self.serial,
                },
            );
            self.bytes = self.bytes + data.len() as u64 - old.map_or(0, |r| r.bytes as u64);
            self.lru.insert((self.serial, job.key));
            self.index_dirty.insert(job.key.page());
            if let Some(t) = self.tiles.get_mut(&job.key) {
                if t.generation == job.generation {
                    t.dirty = false;
                    self.dirty_count -= 1;
                }
            }
            // Publish the page in the same hook, before a leave can interrupt.
            self.write_page(job.key.page());
            return;
        }
        let next = self
            .tiles
            .iter()
            .filter(|(_, t)| {
                t.dirty
                    && !t.base_pending
                    && (self.force || now - t.changed >= 45.0 || (idle && now - t.changed >= 1.0))
            })
            .min_by(|(_, a), (_, b)| a.changed.total_cmp(&b.changed))
            .map(|(k, t)| (*k, t.generation));
        if let Some((key, generation)) = next {
            self.saving = Some(Saving {
                key,
                generation,
                encoder: Encoder::new(self.tiles[&key].rgba.clone()),
            });
        } else if let Some(&page) = self.index_dirty.first() {
            self.write_page(page);
        } else if !self.tiles.values().any(|t| t.dirty) {
            self.force = false;
        }
    }
    fn write_page(&mut self, page: usize) {
        if !self.root {
            if !self.set(ROOT, b"DIM1") {
                return;
            }
            self.root = true;
        }
        let mut data = Vec::with_capacity(self.pages[page].len() * 36);
        for (k, r) in &self.pages[page] {
            data.extend_from_slice(&k.ns);
            data.extend_from_slice(&k.x.to_le_bytes());
            data.extend_from_slice(&k.y.to_le_bytes());
            data.extend_from_slice(&r.bytes.to_le_bytes());
            data.extend_from_slice(&r.used.to_le_bytes());
        }
        // Reserve traffic for tile data and independent settings/waypoints.
        if data.len() > (1 << 20) - BYTES - (64 << 10) {
            self.stopped = true;
            horizon_mod_sdk::warn("explored map: saving stopped: index page is full");
        } else if self.set(&Self::page_name(page), &data) {
            self.index_dirty.remove(&page);
        }
    }
    pub fn pending(&self) -> bool {
        self.saving.is_some()
            || self.tiles.values().any(|t| t.dirty)
            || !self.index_dirty.is_empty()
    }
    /// Saved tile images cover any fullscreen view, independently of its bake.
    pub fn visible(
        &self,
        p: &game::Player,
        cut: Option<i32>,
        flat: bool,
        view: &diorama_core::fullmap::MapView,
        gui: (f32, f32),
    ) -> Scan {
        let mut layers = Vec::new();
        for tpb in [1, 2] {
            let Some(ns) = self.namespace(p, cut, tpb, flat) else {
                continue;
            };
            let a = view.screen_to_map((0.0, 0.0), gui);
            let b = view.screen_to_map(gui, gui);
            let tile = |v: f64| ((v * tpb as f64).floor() as i32).div_euclid(TILE as i32);
            layers.push(ScanLayer {
                ns,
                tpb,
                x0: tile(a.0),
                x1: tile(b.0),
                y1: tile(b.1),
                x: tile(a.0),
                y: tile(a.1),
            });
        }
        Scan { layers }
    }
    pub fn tile(&mut self, id: ([u8; 16], i32, i32, u32)) -> Option<&[u8]> {
        let k = Key {
            ns: id.0,
            x: id.1,
            y: id.2,
        };
        self.load(k, false).then(|| self.tiles[&k].rgba.as_slice())
    }
}
