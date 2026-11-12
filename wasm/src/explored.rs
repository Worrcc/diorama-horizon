//! Saved final texels; independent of chunk acquisition/prepare (including M3).
use super::*;
use diorama_core::explored::{self as codec, Encoder, BYTES, SIDE as TILE};
use std::collections::{BTreeMap, VecDeque};

// Native tests exercise the real scheduler with an isolated storage implementation.
#[cfg(test)]
use test_storage as storage;

const ROOT: &str = "map_index_v1";
const PAGES: usize = 64;
const LIMIT: u64 = 900 << 20;
const KEY_LIMIT: usize = 60_000;
/// Tiles kept in memory (16 KiB each): the minimap's and the fullscreen
/// map's together, so neither evicts the other's.
const CACHE: usize = 640;
/// Two MiB of breathing room while bounded save batches catch up. Dirty tiles
/// are never discarded to make room; acquisition pauses at this hard bound.
const OVERFLOW: usize = 128;
const CAPTURE_RESERVE: usize = 8;
const PREVIEW_SIDE: u32 = 8;
const PREVIEWS: usize = CACHE * 8;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
    /// copy is merged under it (in `step`, within the read allowance) before it is saved.
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
#[derive(Clone)]
struct ScanLayer {
    ns: [u8; 16],
    tpb: u32,
    x0: i32,
    x1: i32,
    y1: i32,
    x: i32,
    y: i32,
}
#[derive(Clone)]
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
    /// Last known coarse copies cost 256 bytes each; eviction from the full
    /// cache can still draw immediately while its disk read is queued.
    previews: BTreeMap<Key, (u64, Vec<u8>)>,
    requests: VecDeque<Key>,
    loads_left: usize,
    pub revision: u64,
    pub captured: u64,
    pub empty_chunks: u64,
    pub unavailable_chunks: u64,
    pub failed_chunks: u64,
    pub unidentified_chunks: u64,
    pub unsaved_chunks: u64,
    pub world: Option<String>,
    context: Option<Context>,
    saving: Option<Saving>,
    index_dirty: BTreeSet<usize>,
    stopped: bool,
    root: bool,
    /// Restores are restarted after a shift, world/layer or detail change.
    anchor: Option<([u8; 16], i32, i32, u32)>,
    restore: Vec<(Key, u32)>,
    /// Captured tiles waiting for their saved copy to merge under them.
    merges: BTreeSet<Key>,
    restored: bool,
    hold: bool,
    write_left: usize,
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
            previews: BTreeMap::new(),
            requests: VecDeque::new(),
            loads_left: 0,
            revision: 0,
            captured: 0,
            empty_chunks: 0,
            unavailable_chunks: 0,
            failed_chunks: 0,
            unidentified_chunks: 0,
            unsaved_chunks: 0,
            world: saved_world(),
            context: None,
            saving: None,
            index_dirty: BTreeSet::new(),
            stopped: false,
            root,
            anchor: None,
            restore: Vec::new(),
            merges: BTreeSet::new(),
            restored: false,
            hold: false,
            write_left: (1 << 20) - (64 << 10),
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
    pub(super) fn namespace(
        &self,
        p: &game::Player,
        cut: Option<i32>,
        tpb: u32,
        flat: bool,
    ) -> Option<[u8; 16]> {
        if let Some(c) = &self.context {
            if c.cut == cut
                && c.flat == flat
                && c.dimension == p.dimension
                && self.world.as_deref() == Some(c.world.as_str())
            {
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
            let tile = self.tiles.remove(&key).unwrap();
            self.remember_preview(key, &tile.rgba);
            true
        } else {
            self.tiles.len() < CACHE + OVERFLOW
        }
    }
    fn remember_preview(&mut self, key: Key, rgba: &[u8]) {
        let mut preview = vec![0; (PREVIEW_SIDE * PREVIEW_SIDE * 4) as usize];
        let scale = TILE / PREVIEW_SIDE;
        for y in 0..PREVIEW_SIDE {
            for x in 0..PREVIEW_SIDE {
                // Any explored sample keeps a partly explored cell from vanishing.
                let mut sample = [0; 4];
                for j in 0..scale {
                    for i in 0..scale {
                        let at = (((y * scale + j) * TILE + x * scale + i) * 4) as usize;
                        if rgba[at + 3] != 0 {
                            sample.copy_from_slice(&rgba[at..at + 4]);
                        }
                    }
                }
                let at = ((y * PREVIEW_SIDE + x) * 4) as usize;
                preview[at..at + 4].copy_from_slice(&sample);
            }
        }
        if self.previews.len() >= PREVIEWS && !self.previews.contains_key(&key) {
            if let Some(oldest) = self
                .previews
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .map(|(k, _)| *k)
            {
                self.previews.remove(&oldest);
            }
        }
        self.previews.insert(key, (self.serial, preview));
    }
    fn load(&mut self, key: Key, empty: bool) -> bool {
        self.serial += 1;
        if let Some(t) = self.tiles.get_mut(&key) {
            t.used = self.serial;
            if t.base_pending {
                if self.loads_left == 0 {
                    return false;
                }
                self.merge(key);
            }
            return true;
        }
        if !self.room() {
            return false;
        }
        let rgba = if self.pages[key.page()].contains_key(&key) {
            if self.loads_left == 0 {
                return false;
            }
            self.loads_left -= 1;
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
        self.revision = self.revision.wrapping_add(1);
        true
    }
    pub fn invalidate(&mut self) {
        self.anchor = None;
        self.restore.clear();
    }
    pub fn stopped(&self) -> bool {
        self.stopped
    }
    pub fn can_capture(&mut self) -> bool {
        if self.stopped {
            return true;
        }
        self.dirty_count < CACHE + OVERFLOW - CAPTURE_RESERVE
    }

    pub fn remember_detail(&mut self) {
        if !self.stopped {
            self.set("map_detail", b"1");
        }
    }
    pub fn flush(&mut self) {
        // Saves are eager now: the next bounded batch already includes every
        // dirty tile, including while dragging or after leaving the world.
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
        let near: Vec<_> = self
            .restore
            .iter()
            .rev()
            .take(CAPTURE_RESERVE)
            .map(|v| v.0)
            .collect();
        for key in near.into_iter().rev() {
            self.request(key, true);
        }
        if let Some((key, density)) = self.restore.pop() {
            self.hold = true;
            if self.resident(key) {
                m.restore_tile(
                    (key.x * TILE as i32, key.y * TILE as i32),
                    density,
                    TILE,
                    &self.tiles[&key].rgba,
                );
                self.restored = true;
            } else if self.known(key) {
                self.request(key, true);
                self.restore.push((key, density));
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
        if self.loads_left == 0 {
            return;
        }
        self.loads_left -= 1;
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

    /// Saves final texels from a chunk capture or a mosaic upload (outlines
    /// already in). Transparent texels preserve previously explored terrain: `rgba` is a `w` x `h` rect at texel `(x, y)` of a mosaic
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
        let Some(ns) = self.namespace(p, cut, tpb, flat) else {
            return;
        };
        self.capture(ns, anchor, tpb, (x, y, w, h), rgba, now);
    }
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        &mut self,
        ns: [u8; 16],
        anchor: (i32, i32),
        tpb: u32,
        (x, y, w, h): (u32, u32, u32, u32),
        rgba: &[u8],
        now: f64,
    ) {
        if self.stopped || rgba.len() < (w * h * 4) as usize {
            return;
        }
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
                    let mut changed = false;
                    for (dst, px) in t.rgba[at..at + n]
                        .as_chunks_mut::<4>()
                        .0
                        .iter_mut()
                        .zip(src.as_chunks::<4>().0)
                    {
                        if px[3] != 0 && dst != px {
                            *dst = *px;
                            changed = true;
                        }
                    }
                    if changed {
                        self.revision = self.revision.wrapping_add(1);
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
        if data.len() > self.write_left {
            return false;
        }
        if let Err(e) = storage::set(key, data) {
            self.stopped = true;
            horizon_mod_sdk::warn(&format!("explored map: saving stopped: {e}"));
            false
        } else {
            self.write_left -= data.len();
            self.written += data.len() as u64;
            true
        }
    }
    /// All storage I/O runs here, before baking. Tile data writes are bounded
    /// by the selected batch; no 45-second delay lets a drag fill the cache.
    pub fn step(&mut self, granted: u32, limits: budget::Performance) {
        self.loads_left = limits.tile_loads;
        // The host caps all writes in one hook, including index pages. Leave
        // traffic for independent settings and waypoints in that same hook.
        self.write_left = (1 << 20) - (64 << 10);
        if self.stopped {
            self.load_requests(limits.tile_loads);
            return;
        }
        // Demand reads get part of the shared allowance before background
        // merges, so a continuous capture stream cannot starve the map.
        self.load_requests(limits.tile_loads / 2);
        for unit in 0..limits.tile_saves {
            if unit > 0 && work_left() <= limits.saved_keep {
                break;
            }
            if self.bytes > LIMIT || self.lru.len() >= KEY_LIMIT {
                if let Some((_, key)) = self.lru.pop_first() {
                    if let Some(r) = self.pages[key.page()].remove(&key) {
                        self.bytes -= r.bytes as u64;
                    }
                    storage::delete(&key.name());
                    if self.tiles.remove(&key).is_some_and(|t| t.dirty) {
                        self.dirty_count -= 1;
                    }
                    self.merges.remove(&key);
                    if self.saving.as_ref().is_some_and(|s| s.key == key) {
                        self.saving = None;
                    }
                    self.previews.remove(&key);
                    self.index_dirty.insert(key.page());
                }
                continue;
            }
            // Old data must merge before a captured tile can be saved.
            if self.loads_left > 0 {
                if let Some(&key) = self.merges.first() {
                    self.merge(key);
                }
            }
            if self.saving.is_none() {
                let next = self
                    .tiles
                    .iter()
                    .filter(|(_, t)| t.dirty && !t.base_pending)
                    .min_by(|(_, a), (_, b)| a.changed.total_cmp(&b.changed))
                    .map(|(k, t)| (*k, t.generation));
                if let Some((key, generation)) = next {
                    self.saving = Some(Saving {
                        key,
                        generation,
                        encoder: Encoder::new(self.tiles[&key].rgba.clone()),
                    });
                }
            }
            if let Some(job) = &self.saving {
                let page = &self.pages[job.key.page()];
                let page_bytes = (page.len() + usize::from(!page.contains_key(&job.key))) * 36;
                if page_bytes > (1 << 20) - BYTES - (64 << 10) {
                    self.stopped = true;
                    horizon_mod_sdk::warn("explored map: saving stopped: index page is full");
                    return;
                }
                // Keep tile and index atomic within the hook's write allowance.
                let needed = BYTES + 1 + page_bytes + if self.root { 0 } else { 4 };
                if needed > self.write_left {
                    break;
                }
            }
            let Some(mut job) = self.saving.take() else {
                break;
            };
            let before = work_left();
            // One tile is 4096 pixels. The batch limits both encoding and writes.
            let done = job.encoder.step(BYTES / 4);
            self.encode_work += before.saturating_sub(work_left()) * granted as u64;
            self.encode_pixels += (BYTES / 4) as u64;
            if !done {
                self.saving = Some(job);
                break;
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
            // Publish the page in this hook, before a leave can interrupt.
            self.write_page(job.key.page());
            if self.stopped {
                return;
            }
        }
        if let Some(&page) = self.index_dirty.first() {
            self.write_page(page);
        }
        while self.tiles.len() > CACHE {
            let before = self.tiles.len();
            self.room();
            if self.tiles.len() == before {
                break;
            }
        }
        self.load_requests(limits.tile_loads);
    }

    fn load_requests(&mut self, count: usize) {
        // Visible requests outrank directional prefetch. Unavailable storage
        // is forgotten so a broken/missing record cannot stall a view forever.
        let requests = self.requests.len().min(count);
        for unit in 0..requests {
            if unit > 0 && work_left() <= budget::RESERVE {
                break;
            }
            if self.loads_left == 0 {
                break;
            }
            let key = self.requests.pop_front().unwrap();
            if !self.load(key, false) {
                if self.tiles.len() >= CACHE + OVERFLOW {
                    self.requests.push_front(key);
                    break;
                }
                if let Some(r) = self.pages[key.page()].remove(&key) {
                    self.bytes -= r.bytes as u64;
                    self.lru.remove(&(r.used, key));
                    self.index_dirty.insert(key.page());
                    self.previews.remove(&key);
                }
            }
        }
    }

    fn known(&self, key: Key) -> bool {
        self.tiles.contains_key(&key) || self.pages[key.page()].contains_key(&key)
    }
    fn resident(&mut self, key: Key) -> bool {
        self.serial += 1;
        let resident = if let Some(t) = self.tiles.get_mut(&key) {
            t.used = self.serial;
            !t.base_pending
        } else {
            false
        };
        if resident {
            self.touch(key);
        }
        resident
    }
    fn request(&mut self, key: Key, visible: bool) {
        if self.tiles.get(&key).is_some_and(|t| !t.base_pending) || !self.known(key) {
            return;
        }
        if let Some(at) = self.requests.iter().position(|k| *k == key) {
            if !visible {
                return;
            }
            self.requests.remove(at);
        }
        if self.requests.len() >= CACHE {
            self.requests.pop_back();
        }
        if visible {
            self.requests.push_front(key);
        } else {
            self.requests.push_back(key);
        }
    }
    pub fn warm_scan(&mut self, scan: &Scan, count: usize) {
        let mut scan = scan.clone();
        let mut keys = Vec::new();
        for _ in 0..count {
            let (done, id) = scan.step(self);
            if let Some(id) = id {
                keys.push(Key {
                    ns: id.0,
                    x: id.1,
                    y: id.2,
                });
            }
            if done {
                break;
            }
        }
        for key in keys.into_iter().rev() {
            self.request(key, true);
        }
    }
    pub fn waiting(&self, id: TileId) -> bool {
        self.known(Key {
            ns: id.0,
            x: id.1,
            y: id.2,
        })
    }
    /// Warm a strip just beyond the leading edge; visible misses still go first.
    #[allow(clippy::too_many_arguments)]
    pub fn prefetch(
        &mut self,
        p: &game::Player,
        cut: Option<i32>,
        flat: bool,
        view: &diorama_core::fullmap::MapView,
        gui: (f32, f32),
        movement: (f64, f64),
        count: usize,
    ) {
        let tpb = 1;
        let Some(ns) = self.namespace(p, cut, tpb, flat) else {
            return;
        };
        let horizontal = movement.0.abs() >= movement.1.abs();
        if movement == (0.0, 0.0) {
            return;
        }
        for i in 0..count {
            let offset = (i as f64 - count.saturating_sub(1) as f64 / 2.0) * TILE as f64;
            let (u, v) = if horizontal {
                (
                    view.centre_u
                        + movement.0.signum()
                            * (gui.0 as f64 / view.scale as f64 / 2.0 + TILE as f64),
                    view.centre_v + offset,
                )
            } else {
                (
                    view.centre_u + offset,
                    view.centre_v
                        + movement.1.signum()
                            * (gui.1 as f64 / view.scale as f64 / 2.0 + TILE as f64),
                )
            };
            for density in [1, 2] {
                let ns = if density == 1 {
                    ns
                } else {
                    let Some(ns) = self.namespace(p, cut, density, flat) else {
                        continue;
                    };
                    ns
                };
                self.request(
                    Key {
                        ns,
                        x: ((u * density as f64).floor() as i32).div_euclid(TILE as i32),
                        y: ((v * density as f64).floor() as i32).div_euclid(TILE as i32),
                    },
                    false,
                );
            }
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
        for tpb in [2, 1] {
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
    pub fn tile(&mut self, id: TileId) -> Option<(u32, &[u8])> {
        let key = Key {
            ns: id.0,
            x: id.1,
            y: id.2,
        };
        if self.resident(key) {
            Some((TILE, &self.tiles[&key].rgba))
        } else {
            self.request(key, true);
            self.touch(key);
            self.previews.get_mut(&key).map(|(used, rgba)| {
                *used = self.serial;
                (PREVIEW_SIDE, rgba.as_slice())
            })
        }
    }
}

fn work_left() -> u64 {
    #[cfg(not(test))]
    {
        canvas::work_left()
    }
    #[cfg(test)]
    {
        budget::WORK_SCALE
    }
}

fn saved_world() -> Option<String> {
    #[cfg(not(test))]
    {
        game::world_id()
    }
    #[cfg(test)]
    {
        Some("test-world".into())
    }
}

#[cfg(test)]
mod test_storage {
    use std::{cell::RefCell, collections::BTreeMap};
    thread_local! {
        static FILES: RefCell<BTreeMap<String, Vec<u8>>> = const { RefCell::new(BTreeMap::new()) };
        pub static READS: RefCell<usize> = const { RefCell::new(0) };
        pub static WRITES: RefCell<Vec<(String, usize)>> = const { RefCell::new(Vec::new()) };
    }
    pub fn clear() {
        FILES.with_borrow_mut(|f| f.clear());
        READS.with_borrow_mut(|v| *v = 0);
        WRITES.with_borrow_mut(|v| v.clear());
    }
    pub fn get(key: &str) -> Option<Vec<u8>> {
        READS.with_borrow_mut(|v| *v += 1);
        FILES.with_borrow(|f| f.get(key).cloned())
    }
    pub fn set(key: &str, value: &[u8]) -> Result<(), String> {
        WRITES.with_borrow_mut(|v| v.push((key.into(), value.len())));
        FILES.with_borrow_mut(|f| f.insert(key.into(), value.to_vec()));
        Ok(())
    }
    pub fn delete(key: &str) {
        FILES.with_borrow_mut(|f| f.remove(key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diorama_core::{
        bake::{Chunk, Shade, Texel},
        projection::Projection,
    };

    fn key(x: i32) -> Key {
        Key {
            ns: [1; 16],
            x,
            y: 0,
        }
    }
    fn tile(used: u64, dirty: bool) -> Tile {
        Tile {
            rgba: [10, 20, 30, 255].repeat(BYTES / 4),
            dirty,
            used,
            generation: 1,
            changed: 0.0,
            base_pending: false,
        }
    }
    fn saved() -> Saved {
        test_storage::clear();
        Saved::new()
    }

    #[test]
    fn clean_lru_is_evicted_before_any_dirty_tile_and_overflow_is_bounded() {
        let mut s = saved();
        for i in 0..CACHE {
            s.tiles.insert(key(i as i32), tile(i as u64, true));
        }
        s.dirty_count = CACHE;
        s.tiles.get_mut(&key(10)).unwrap().dirty = false;
        s.tiles.get_mut(&key(20)).unwrap().dirty = false;
        s.dirty_count -= 2;
        assert!(s.room());
        assert!(!s.tiles.contains_key(&key(10)));
        assert!(s.tiles.contains_key(&key(0)));
        s.tiles.insert(key(CACHE as i32), tile(1000, true));
        assert!(s.room());
        assert!(!s.tiles.contains_key(&key(20)));
        for i in s.tiles.len()..CACHE + OVERFLOW {
            s.tiles.insert(key(10_000 + i as i32), tile(1000, true));
        }
        assert_eq!(s.tiles.len(), CACHE + OVERFLOW);
        assert!(!s.room());
        assert_eq!(s.tiles.len(), CACHE + OVERFLOW);
    }

    #[test]
    fn saves_are_bounded_batches_and_dirty_backlog_drains_during_a_drag() {
        for limits in budget::PERFORMANCE {
            let mut s = saved();
            for i in 0..20 {
                s.tiles.insert(key(i), tile(i as u64, true));
            }
            s.dirty_count = 20;
            s.step(1000, limits);
            let writes = test_storage::WRITES.with_borrow(|v| v.clone());
            assert_eq!(
                writes.iter().filter(|(k, _)| k.starts_with("mt1.")).count(),
                limits.tile_saves
            );
            assert!(writes.iter().map(|v| v.1).sum::<usize>() <= (1 << 20) - (64 << 10));
            assert_eq!(s.dirty_count, 20 - limits.tile_saves);
            for _ in 0..20 {
                s.step(1000, limits);
            }
            assert_eq!(s.dirty_count, 0);
        }
    }

    #[test]
    fn tile_requests_do_no_io_and_reads_including_merges_share_one_bound() {
        let mut s = saved();
        let mut encoder = Encoder::new(tile(0, false).rgba);
        assert!(encoder.step(BYTES / 4));
        let encoded = encoder.finish();
        for i in 0..12 {
            let k = key(i);
            test_storage::set(&k.name(), &encoded).unwrap();
            s.pages[k.page()].insert(
                k,
                Record {
                    bytes: encoded.len() as u32,
                    used: 0,
                },
            );
            s.bytes += encoded.len() as u64;
            assert!(s.tile((k.ns, k.x, k.y, 1)).is_none());
        }
        let before = test_storage::READS.with_borrow(|n| *n);
        let limits = budget::performance("performant");
        s.step(1000, limits);
        let reads = test_storage::READS.with_borrow(|n| *n) - before;
        assert_eq!(reads, limits.tile_loads);
        assert_eq!(s.tiles.len(), limits.tile_loads);
    }

    #[test]
    fn evicted_tile_draws_its_preview_while_the_storage_read_waits() {
        let mut s = saved();
        for i in 0..CACHE {
            let k = key(i as i32);
            s.tiles.insert(k, tile(i as u64, false));
            s.pages[k.page()].insert(
                k,
                Record {
                    bytes: 7,
                    used: i as u64,
                },
            );
        }
        assert!(s.room());
        assert!(!s.tiles.contains_key(&key(0)));
        let before = test_storage::READS.with_borrow(|v| *v);
        let (side, pixels) = s.tile((key(0).ns, 0, 0, 1)).unwrap();
        assert_eq!(side, PREVIEW_SIDE);
        assert!(pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|px| *px == [10, 20, 30, 255]));
        assert_eq!(test_storage::READS.with_borrow(|v| *v), before);
        assert_eq!(s.requests.front().copied(), Some(key(0)));
    }

    #[test]
    fn transparent_recapture_never_erases_a_drawn_pixel() {
        let mut s = saved();
        let ns = key(0).ns;
        s.capture(ns, (0, 0), 1, (0, 0, 1, 1), &[1, 2, 3, 255], 0.0);
        s.capture(ns, (0, 0), 1, (0, 0, 1, 1), &[0; 4], 0.1);
        assert_eq!(&s.tiles[&key(0)].rgba[..4], &[1, 2, 3, 255]);
    }

    #[test]
    fn captures_merge_disk_texels_and_share_reads_with_visible_requests() {
        let mut s = saved();
        let k = key(0);
        let mut encoder = Encoder::new(tile(0, false).rgba);
        encoder.step(BYTES / 4);
        let data = encoder.finish();
        test_storage::set(&k.name(), &data).unwrap();
        s.pages[k.page()].insert(
            k,
            Record {
                bytes: data.len() as u32,
                used: 0,
            },
        );
        s.bytes = data.len() as u64;
        // Capture into a saved tile without synchronously reading its base.
        s.capture(k.ns, (0, 0), 1, (0, 0, 1, 1), &[99, 98, 97, 255], 0.0);
        assert!(s.tiles[&k].base_pending);
        let before = test_storage::READS.with_borrow(|v| *v);
        s.step(1000, budget::performance("performant"));
        assert!(test_storage::READS.with_borrow(|v| *v) - before <= 2);
        assert_eq!(&s.tiles[&k].rgba[..8], &[99, 98, 97, 255, 10, 20, 30, 255]);
        assert_eq!(s.dirty_count, 0);
    }

    struct Colour;
    impl Shade for Colour {
        fn top(&self, _: &Chunk, _: usize, y: f64) -> Texel {
            Texel::dry(0x123456, y)
        }
        fn south(&self, _: &Chunk, _: usize, _: i32, y: f64) -> Texel {
            Texel::dry(0x654321, y)
        }
    }
    #[test]
    fn undrawn_chunk_outside_both_images_reaches_persistent_tiles() {
        let mut s = saved();
        for density in [1, 2] {
            let mut mosaic = Mosaic::with_texels(Projection::default(), 64, density);
            let chunk = Chunk::from_fn(1, |_, _| Some((64, 1)));
            mosaic.set_chunk((-100, 100), Some(chunk));
            assert!(mosaic.pixels().iter().all(|v| *v == 0));
            let mut job = mosaic.capture_chunk((-100, 100)).unwrap();
            let ns = [density as u8; 16];
            let mut complete = false;
            for _ in 0..100 {
                complete = job.step(&Colour, 8, |anchor, tpb, rect, rgba| {
                    s.capture(ns, anchor, tpb, rect, rgba, 0.0);
                });
                if complete {
                    break;
                }
            }
            assert!(complete);
            assert!(s.tiles.iter().any(|(key, tile)| key.ns == ns && tile.dirty));
            // No mosaic paint or upload was needed for persistence.
            assert!(mosaic.pixels().iter().all(|v| *v == 0));
        }
        for _ in 0..100 {
            s.step(1000, budget::performance("speedy"));
        }
        assert_eq!(s.dirty_count, 0);
        for (key, tile) in &s.tiles {
            let disk = test_storage::get(&key.name()).unwrap();
            assert_eq!(codec::decode(&disk).unwrap(), tile.rgba);
        }
    }
}
