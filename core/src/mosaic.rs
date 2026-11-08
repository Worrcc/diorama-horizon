//! The mosaic: one RGBA image around the player that the column baker paints
//! into, with a height buffer, the chunk cache, dirty rects and recentring
//! (PLAN-v2 section 3.2, "Mosaic").
//!
//! The view faces north, so a block's map column is just its x (RULES.md
//! section 1): everything a chunk draws lies in the 32 texel columns of its
//! chunk x. The image is cut into such **strips**, one per chunk x, and a strip
//! is repainted from the chunks of that chunk x alone. Dirty state is a row
//! interval per strip, for painting and for uploading.
//!
//! Repainting a strip's rows clears them and paints, clipped to them, every
//! cached chunk of that chunk x whose drawing reaches them. So a texel is always
//! the nearest face of the chunks as they are now, and a block removed leaves
//! nothing behind. A chunk's drawing depends on its south neighbour (its last
//! row's south faces), so a chunk that changes dirties its north neighbour's
//! rows too.

use std::collections::BTreeMap;

use crate::bake::{self, Chunk, Faces, Shade, Texel, TEXELS_PER_BLOCK};
use crate::projection::Projection;
use crate::style::{darken, outline_keep, OUTLINE_DEPTH};

/// Texel columns per strip (one chunk across).
pub const STRIP: u32 = 16 * TEXELS_PER_BLOCK;
/// Recentre when the player comes this many texels (48 blocks) from an edge
/// (PLAN-v2 section 3.2).
pub const EDGE: f64 = 96.0;
/// The anchor's map row snaps to this many map units.
const ROW_STEP: i32 = 8;

/// A row interval `[first, end)` of one strip.
type Rows = Option<(u32, u32)>;

struct Cached {
    chunk: Chunk,
    /// The map rows `[a, b)` its faces cover now (none: it draws nothing).
    span: Option<(f64, f64)>,
}

/// What the baker did, for the mod's cost log and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub jobs: u64,
    pub columns: u64,
    pub texels: u64,
    pub recentres: u64,
}

/// The mosaic: `side` x `side` texels, `side` a multiple of [`STRIP`].
pub struct Mosaic {
    proj: Projection,
    side: u32,
    /// Block x of texel column 0 (a multiple of 16) and map row of texel row 0
    /// (a multiple of [`ROW_STEP`]).
    u0: i32,
    v0: i32,
    rgba: Vec<u8>,
    /// Per texel: the height of the nearest face painted there (`-inf`: none).
    height: Vec<f32>,
    /// Per texel: the height of what is seen there (the floor under water),
    /// and the ray's path through water in front of it in eighths of a block,
    /// for outlines.
    seen: Vec<f32>,
    wet: Vec<u8>,
    chunks: BTreeMap<(i32, i32), Cached>,
    paint: Vec<Rows>,
    upload: Vec<Rows>,
    pub stats: Stats,
    /// How much nearer a neighbour must be to draw an outline: [`OUTLINE_DEPTH`],
    /// or [`crate::style::CAVE_OUTLINE_DEPTH`] in a cave slice (TileBaker.java:891).
    pub outline_depth: f32,
}

fn hull(slot: &mut Rows, (a, b): (u32, u32)) {
    if a >= b {
        return;
    }
    *slot = Some(match *slot {
        Some((x, y)) => (x.min(a), y.max(b)),
        None => (a, b),
    });
}

fn floor_to(a: f64, step: i32) -> i32 {
    (a / f64::from(step)).floor() as i32 * step
}

/// The texel buffers a repaint writes, split from the cache it reads.
struct Canvas<'a> {
    proj: &'a Projection,
    side: u32,
    v0: f64,
    rgba: &'a mut [u8],
    height: &'a mut [f32],
    seen: &'a mut [f32],
    wet: &'a mut [u8],
    stats: &'a mut Stats,
}

/// [`Mosaic`]'s `wet` scale: eighths of a block, to 31.9 blocks.
const WET_SCALE: f32 = 8.0;

impl Canvas<'_> {
    fn put(&mut self, i: u32, j: u32, y: f32, t: Texel) {
        let at = (j * self.side + i) as usize;
        if y > self.height[at] {
            let rgb = t.rgb;
            self.height[at] = y;
            self.seen[at] = t.seen;
            self.wet[at] = (t.water * WET_SCALE).round().clamp(0.0, 255.0) as u8;
            self.rgba[at * 4..at * 4 + 4].copy_from_slice(&[
                (rgb >> 16) as u8,
                (rgb >> 8) as u8,
                rgb as u8,
                255,
            ]);
            self.stats.texels += 1;
        }
    }

    /// Paints one column's faces into its two texel columns from `i`, rows
    /// `r0..r1` only. `z` is the column's world z.
    #[allow(clippy::too_many_arguments)]
    fn column(
        &mut self,
        i: u32,
        z: i32,
        f: Faces,
        chunk: &Chunk,
        col: usize,
        shade: &impl Shade,
        (r0, r1): (u32, u32),
    ) {
        self.stats.columns += 1;
        let s = bake::spans(self.proj, z, f);
        let (lo, hi) = (i64::from(r0), i64::from(r1));
        let clip = |(a, b): (i64, i64)| (a.clamp(lo, hi) as u32, b.clamp(lo, hi) as u32);
        let (a, b) = clip(bake::rows(s.top.0, s.top.1, self.v0));
        if a < b {
            let y = (f.top + 1) as f32;
            let t = shade.top(chunk, col, f64::from(y));
            for j in a..b {
                self.put(i, j, y, t);
                self.put(i + 1, j, y, t);
            }
        }
        if f.south_blocks() == 0 {
            return;
        }
        let (a, b) = clip(bake::rows(s.south.0, s.south.1, self.v0));
        let zf = f64::from(z + 1);
        for j in a..b {
            let v = bake::row_v(i64::from(j), self.v0);
            let y = bake::y_on_south(self.proj, v, zf);
            let block = (y.floor() as i32).clamp(f.south_from, f.top);
            let t = shade.south(chunk, col, block, y);
            self.put(i, j, y as f32, t);
            self.put(i + 1, j, y as f32, t);
        }
    }
}

/// The faces of local column `(x, z)` of chunk `key`, its south neighbour
/// read from the chunk or the cache.
fn column_faces(
    chunks: &BTreeMap<(i32, i32), Cached>,
    key: (i32, i32),
    chunk: &Chunk,
    x: usize,
    z: usize,
) -> Option<Faces> {
    let top = chunk.top(x, z)?;
    let south = if z < 15 {
        Some(chunk.top(x, z + 1))
    } else {
        chunks.get(&(key.0, key.1 + 1)).map(|c| c.chunk.top(x, 0))
    };
    Some(bake::faces(top, south))
}

impl Mosaic {
    /// An empty mosaic anchored at map (0, 0).
    pub fn new(proj: Projection, side: u32) -> Mosaic {
        assert!(side > 0 && side.is_multiple_of(STRIP), "side {side}");
        let n = (side * side) as usize;
        let strips = (side / STRIP) as usize;
        Mosaic {
            proj,
            side,
            u0: 0,
            v0: 0,
            rgba: vec![0; n * 4],
            height: vec![f32::NEG_INFINITY; n],
            seen: vec![0.0; n],
            wet: vec![0; n],
            chunks: BTreeMap::new(),
            paint: vec![None; strips],
            upload: vec![None; strips],
            stats: Stats::default(),
            outline_depth: OUTLINE_DEPTH,
        }
    }
    pub fn side(&self) -> u32 {
        self.side
    }
    pub fn projection(&self) -> &Projection {
        &self.proj
    }
    /// Block x of texel column 0 and map row of texel row 0.
    pub fn anchor(&self) -> (i32, i32) {
        (self.u0, self.v0)
    }
    /// The (fractional) texel of map point `(u, v)`.
    pub fn texel_of(&self, u: f64, v: f64) -> (f64, f64) {
        let t = f64::from(TEXELS_PER_BLOCK);
        ((u - f64::from(self.u0)) * t, (v - f64::from(self.v0)) * t)
    }
    /// The image, RGBA rows top first.
    pub fn pixels(&self) -> &[u8] {
        &self.rgba
    }
    /// Texel `(i, j)`'s colour 0xRRGGBB, none where nothing is painted.
    pub fn rgb_at(&self, i: u32, j: u32) -> Option<u32> {
        let at = ((j * self.side + i) * 4) as usize;
        let p = &self.rgba[at..at + 4];
        (p[3] != 0).then(|| (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2]))
    }
    /// Texel `(i, j)`'s nearest face height (`-inf`: none).
    pub fn height_at(&self, i: u32, j: u32) -> f32 {
        self.height[(j * self.side + i) as usize]
    }
    /// Chunks in the cache.
    pub fn chunks(&self) -> usize {
        self.chunks.len()
    }
    /// The cached chunks' keys.
    pub fn keys(&self) -> Vec<(i32, i32)> {
        self.chunks.keys().copied().collect()
    }
    /// Marks the whole image to paint again (the looks changed).
    pub fn repaint_all(&mut self) {
        self.paint.fill(Some((0, self.side)));
    }
    pub fn has(&self, key: (i32, i32)) -> bool {
        self.chunks.contains_key(&key)
    }
    /// Strips with rows left to paint, and to upload.
    pub fn painting(&self) -> bool {
        self.paint.iter().any(Option::is_some)
    }
    pub fn uploading(&self) -> bool {
        self.upload.iter().any(Option::is_some)
    }
    /// The rows of strip `s` waiting to be painted.
    pub fn paint_rows(&self, s: u32) -> Option<(u32, u32)> {
        self.paint[s as usize]
    }
    /// The strip holding chunk x `cx`, if it is in the image.
    pub fn strip_of(&self, cx: i32) -> Option<u32> {
        let d = i64::from(cx) * 16 - i64::from(self.u0);
        let s = d.div_euclid(16);
        (0..i64::from(self.side / STRIP))
            .contains(&s)
            .then_some(s as u32)
    }

    /// The map rows chunk `key` draws now, from the cache.
    fn span_of(&self, key: (i32, i32)) -> Option<(f64, f64)> {
        let c = self.chunks.get(&key)?;
        let mut span: Option<(f64, f64)> = None;
        for z in 0..16 {
            for x in 0..16 {
                let Some(f) = column_faces(&self.chunks, key, &c.chunk, x, z) else {
                    continue;
                };
                let s = bake::spans(&self.proj, key.1 * 16 + z as i32, f);
                let (a, b) = (s.top.0, s.top.1.max(s.south.1));
                span = Some(span.map_or((a, b), |(x, y)| (x.min(a), y.max(b))));
            }
        }
        span
    }

    /// Marks the rows of map span `(a, b)` in chunk x `cx`'s strip for painting.
    fn mark(&mut self, cx: i32, (a, b): (f64, f64)) {
        let Some(s) = self.strip_of(cx) else { return };
        let (first, end) = bake::rows(a, b, f64::from(self.v0));
        let side = i64::from(self.side);
        let (first, end) = (first.clamp(0, side) as u32, end.clamp(0, side) as u32);
        hull(&mut self.paint[s as usize], (first, end));
    }

    /// Puts a chunk's columns in the cache (none: it unloaded) and marks what
    /// it, and its north neighbour, drew before and draw now.
    pub fn set_chunk(&mut self, key: (i32, i32), chunk: Option<Chunk>) {
        let north = (key.0, key.1 - 1);
        let old_c = self.chunks.get(&key).and_then(|c| c.span);
        let old_n = self.chunks.get(&north).and_then(|c| c.span);
        match chunk {
            Some(chunk) => {
                self.chunks.insert(key, Cached { chunk, span: None });
            }
            None => {
                self.chunks.remove(&key);
            }
        }
        let new_c = self.span_of(key);
        if let Some(c) = self.chunks.get_mut(&key) {
            c.span = new_c;
        }
        let new_n = self.span_of(north);
        if let Some(n) = self.chunks.get_mut(&north) {
            n.span = new_n;
        }
        for s in [old_c, new_c, old_n, new_n].into_iter().flatten() {
            self.mark(key.0, s);
        }
    }

    /// Drops cached chunks more than `radius` chunks from `centre` (they stay
    /// in the image until something repaints their rows).
    pub fn forget_far(&mut self, centre: (i32, i32), radius: i32) {
        self.chunks
            .retain(|&(x, z), _| (x - centre.0).abs() <= radius && (z - centre.1).abs() <= radius);
    }

    /// Forgets every chunk and clears the image (a new world or dimension).
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.rgba.fill(0);
        self.height.fill(f32::NEG_INFINITY);
        self.seen.fill(0.0);
        self.wet.fill(0);
        self.paint.fill(None);
        self.upload.fill(Some((0, self.side)));
    }

    /// Paints up to `max_rows` dirty rows of the dirty strip nearest map
    /// column `near_u`. False when nothing is left to paint.
    pub fn paint_step(&mut self, shade: &impl Shade, near_u: f64, max_rows: u32) -> bool {
        let near = (near_u - f64::from(self.u0)) * f64::from(TEXELS_PER_BLOCK);
        let pick = (0..self.paint.len())
            .filter(|&s| self.paint[s].is_some())
            .min_by(|&a, &b| {
                let d =
                    |s: usize| (s as f64 * f64::from(STRIP) + f64::from(STRIP) / 2.0 - near).abs();
                d(a).total_cmp(&d(b))
            });
        let Some(s) = pick else { return false };
        let (r0, r1) = self.paint[s].expect("picked a dirty strip");
        let end = r1.min(r0 + max_rows.max(1));
        self.paint[s] = (end < r1).then_some((end, r1));
        self.repaint(s as u32, (r0, end), shade);
        // Outlines read the texels around: the rows next to these and the
        // strips beside go up again too.
        let rows = (r0.saturating_sub(1), (end + 1).min(self.side));
        hull(&mut self.upload[s], rows);
        if shade.outline() > 0.0 {
            if s > 0 {
                hull(&mut self.upload[s - 1], rows);
            }
            if s + 1 < self.upload.len() {
                hull(&mut self.upload[s + 1], rows);
            }
        }
        self.stats.jobs += 1;
        true
    }

    /// Clears rows `r0..r1` of strip `s` and paints them again from the cache.
    fn repaint(&mut self, s: u32, (r0, r1): (u32, u32), shade: &impl Shade) {
        let side = self.side;
        let i0 = s * STRIP;
        for j in r0..r1 {
            let at = (j * side + i0) as usize;
            self.height[at..at + STRIP as usize].fill(f32::NEG_INFINITY);
            self.rgba[at * 4..(at + STRIP as usize) * 4].fill(0);
        }
        let cx = (self.u0 + s as i32 * 16).div_euclid(16);
        let t = f64::from(TEXELS_PER_BLOCK);
        let (va, vb) = (
            f64::from(self.v0) + f64::from(r0) / t,
            f64::from(self.v0) + f64::from(r1) / t,
        );
        let Mosaic {
            proj,
            v0,
            rgba,
            height,
            seen,
            wet,
            chunks,
            stats,
            ..
        } = self;
        let mut canvas = Canvas {
            proj,
            side,
            v0: f64::from(*v0),
            rgba,
            height,
            seen,
            wet,
            stats,
        };
        for (&key, c) in chunks.range((cx, i32::MIN)..=(cx, i32::MAX)) {
            match c.span {
                Some((a, b)) if a < vb && b > va => {}
                _ => continue,
            }
            for z in 0..16 {
                for x in 0..16 {
                    let Some(f) = column_faces(chunks, key, &c.chunk, x, z) else {
                        continue;
                    };
                    canvas.column(
                        i0 + x as u32 * TEXELS_PER_BLOCK,
                        key.1 * 16 + z as i32,
                        f,
                        &c.chunk,
                        z * 16 + x,
                        shade,
                        (r0, r1),
                    );
                }
            }
        }
    }

    /// Moves the anchor so map point `(u, v)` (the player) is near the middle,
    /// when it has come within [`EDGE`] texels of an edge. True when it moved.
    pub fn recentre(&mut self, u: f64, v: f64) -> bool {
        let (ti, tj) = self.texel_of(u, v);
        let side = f64::from(self.side);
        let edge = EDGE.min(side / 4.0);
        let inside = |t: f64| t >= edge && t <= side - edge;
        if inside(ti) && inside(tj) {
            return false;
        }
        // Half the image in blocks along u and in map units along v.
        let half = (self.side / TEXELS_PER_BLOCK / 2) as i32;
        let u0 = floor_to(u, 16) - floor_to(f64::from(half), 16);
        let v0 = floor_to(v, ROW_STEP) - floor_to(f64::from(half), ROW_STEP);
        self.shift(u0 - self.u0, v0 - self.v0);
        true
    }

    /// Moves the anchor by `du` blocks (a multiple of 16) and `dv` map units:
    /// the texels move with the world, the uncovered ones are marked to paint
    /// and the whole image to upload.
    pub fn shift(&mut self, du: i32, dv: i32) {
        assert!(du.rem_euclid(16) == 0, "du {du}");
        self.stats.recentres += 1;
        let t = TEXELS_PER_BLOCK as i64;
        let side = i64::from(self.side);
        let (di, dj) = (i64::from(du) * t, i64::from(dv) * t);
        self.u0 += du;
        self.v0 += dv;
        self.upload.fill(Some((0, self.side)));
        if di.abs() >= side || dj.abs() >= side {
            self.rgba.fill(0);
            self.height.fill(f32::NEG_INFINITY);
            self.paint.fill(Some((0, self.side)));
            return;
        }
        let n = (self.side * self.side) as usize;
        let mut rgba = vec![0u8; n * 4];
        let mut height = vec![f32::NEG_INFINITY; n];
        let mut seen = vec![0.0f32; n];
        let mut wet = vec![0u8; n];
        let (i_from, i_to) = ((-di).max(0), (side - di).min(side));
        for j in 0..side {
            let sj = j + dj;
            if !(0..side).contains(&sj) || i_from >= i_to {
                continue;
            }
            let dst = (j * side + i_from) as usize;
            let src = (sj * side + i_from + di) as usize;
            let w = (i_to - i_from) as usize;
            height[dst..dst + w].copy_from_slice(&self.height[src..src + w]);
            seen[dst..dst + w].copy_from_slice(&self.seen[src..src + w]);
            wet[dst..dst + w].copy_from_slice(&self.wet[src..src + w]);
            rgba[dst * 4..(dst + w) * 4].copy_from_slice(&self.rgba[src * 4..(src + w) * 4]);
        }
        self.rgba = rgba;
        self.height = height;
        self.seen = seen;
        self.wet = wet;

        let strips = self.paint.len() as i64;
        let ds = i64::from(du / 16);
        let old = std::mem::replace(&mut self.paint, vec![None; strips as usize]);
        let band = if dj > 0 {
            ((side - dj) as u32, self.side)
        } else {
            (0, (-dj) as u32)
        };
        for s in 0..strips {
            let slot = &mut self.paint[s as usize];
            let from = s + ds;
            if !(0..strips).contains(&from) {
                *slot = Some((0, self.side));
                continue;
            }
            if let Some((a, b)) = old[from as usize] {
                let (a, b) = (
                    (i64::from(a) - dj).clamp(0, side) as u32,
                    (i64::from(b) - dj).clamp(0, side) as u32,
                );
                hull(slot, (a, b));
            }
            hull(slot, band);
        }
    }

    /// Texel `(i, j)`'s depth along the view, as Diorama measures it
    /// (TileBaker.java:1088, `-(sin*y + cos*z)` at the hit): with the hit on
    /// map row `v` at height `y`, `-(y/sin + v*cot)`. Nearer is smaller.
    fn depth(&self, at: usize, v: f64) -> f32 {
        let p = &self.proj;
        (-(f64::from(self.seen[at]) / p.sin + v * p.cos / p.sin)) as f32
    }

    /// Texel `(i, j)` as uploaded: its colour darkened by `outline` where a
    /// neighbour (left, right, up, down) is more than [`OUTLINE_DEPTH`] nearer,
    /// faded by the water in front (TileBaker.java:822-840). 0 when nothing
    /// is painted there.
    pub fn shown_at(&self, i: u32, j: u32, outline: f32) -> [u8; 4] {
        let side = self.side;
        let at = (j * side + i) as usize;
        let mut px = [0u8; 4];
        px.copy_from_slice(&self.rgba[at * 4..at * 4 + 4]);
        if px[3] == 0 || outline <= 0.0 {
            return px;
        }
        let t = f64::from(TEXELS_PER_BLOCK);
        let v = |j: u32| f64::from(self.v0) + (f64::from(j) + 0.5) / t;
        let here = self.depth(at, v(j));
        let nearer = |ni: u32, nj: u32| {
            let n = (nj * side + ni) as usize;
            self.rgba[n * 4 + 3] != 0 && here - self.depth(n, v(nj)) > self.outline_depth
        };
        let edge = (i > 0 && nearer(i - 1, j))
            || (i + 1 < side && nearer(i + 1, j))
            || (j > 0 && nearer(i, j - 1))
            || (j + 1 < side && nearer(i, j + 1));
        if edge {
            let keep = outline_keep(outline, f32::from(self.wet[at]) / WET_SCALE);
            let rgb = (u32::from(px[0]) << 16) | (u32::from(px[1]) << 8) | u32::from(px[2]);
            let d = darken(rgb, keep);
            px[..3].copy_from_slice(&[(d >> 16) as u8, (d >> 8) as u8, d as u8]);
        }
        px
    }

    /// Hands the dirty rects to `put(x, y, width, height, rgba)`, at most
    /// `max_bytes` of texels in all; the rest waits for the next call. Strips
    /// side by side with the same rows go as one rect. Texels go as
    /// [`Mosaic::shown_at`] draws them, with outlines of strength `outline`.
    /// Returns the bytes given.
    pub fn take_upload(
        &mut self,
        max_bytes: usize,
        outline: f32,
        mut put: impl FnMut(u32, u32, u32, u32, &[u8]),
    ) -> usize {
        let mut given = 0;
        let mut s = 0;
        let strips = self.upload.len();
        let mut buf = Vec::new();
        while s < strips {
            let Some((r0, r1)) = self.upload[s] else {
                s += 1;
                continue;
            };
            let mut e = s + 1;
            while e < strips && self.upload[e] == Some((r0, r1)) {
                e += 1;
            }
            let (x, w) = (s as u32 * STRIP, (e - s) as u32 * STRIP);
            let row = w as usize * 4;
            let rows = ((max_bytes - given) / row) as u32;
            if rows == 0 {
                break;
            }
            let end = r1.min(r0 + rows);
            buf.clear();
            for j in r0..end {
                if outline > 0.0 {
                    for i in x..x + w {
                        buf.extend_from_slice(&self.shown_at(i, j, outline));
                    }
                } else {
                    let at = ((j * self.side + x) * 4) as usize;
                    buf.extend_from_slice(&self.rgba[at..at + row]);
                }
            }
            put(x, r0, w, end - r0, &buf);
            given += buf.len();
            let rest = (end < r1).then_some((end, r1));
            self.upload[s..e].fill(rest);
            if rest.is_some() {
                break;
            }
            s = e;
        }
        given
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::{faces, MapColours, NO_TOP};

    /// Each column's state is its own id; top colour `id << 1`, south `id << 1 | 1`.
    struct Ids;
    impl Shade for Ids {
        fn top(&self, c: &Chunk, col: usize, y: f64) -> Texel {
            Texel::dry(c.state(col, 0) << 1, y)
        }
        fn south(&self, c: &Chunk, col: usize, block: i32, y: f64) -> Texel {
            let top = i32::from(c.tops[col]);
            Texel::dry((c.state(col, top - block) << 1) | 1, y)
        }
    }
    fn id(x: i32, z: i32) -> u32 {
        ((x + 100) as u32) | (((z + 100) as u32) << 9)
    }
    fn unid(c: u32) -> (i32, i32, bool) {
        let s = c >> 1;
        ((s & 511) as i32 - 100, (s >> 9) as i32 - 100, c & 1 == 0)
    }

    /// Chunks `(cx, cz)` for `cx in xs, cz in zs` from a height function.
    fn world(
        m: &mut Mosaic,
        xs: std::ops::Range<i32>,
        zs: std::ops::Range<i32>,
        h: impl Fn(i32, i32) -> Option<i16>,
    ) {
        for cx in xs {
            for cz in zs.clone() {
                let c = Chunk::from_fn(1, |x, z| {
                    let (wx, wz) = (cx * 16 + x as i32, cz * 16 + z as i32);
                    h(wx, wz).map(|t| (t, id(wx, wz)))
                });
                m.set_chunk((cx, cz), Some(c));
            }
        }
    }
    fn paint_all(m: &mut Mosaic, shade: &impl Shade) {
        while m.paint_step(shade, 0.0, 64) {}
    }

    /// The first face the ray through texel `(i, j)` enters, traced through the
    /// columns directly: in column z the ray is at heights `(y(z), y(z+1)]`
    /// (`y(z) = (z*sin - v)/cos`), inside the solid below `top + 1`, so it enters
    /// that column at `min(y(z+1), top+1)` if that is above `y(z)`; the first hit
    /// is the highest entry. Returns (hit y, column z, top face?).
    fn trace(
        m: &Mosaic,
        h: &impl Fn(i32, i32) -> Option<i16>,
        zs: std::ops::Range<i32>,
        i: u32,
        j: u32,
    ) -> Option<(f64, i32, bool, f64)> {
        let p = m.projection();
        let (u0, v0) = m.anchor();
        let x = u0 + (i / TEXELS_PER_BLOCK) as i32;
        let v = bake::row_v(i64::from(j), f64::from(v0));
        let y = |z: f64| (z * p.sin - v) / p.cos;
        let mut hits: Vec<(f64, i32, bool)> = zs
            .filter_map(|z| {
                let lid = f64::from(h(x, z)?) + 1.0;
                let (lo, hi) = (y(f64::from(z)), y(f64::from(z) + 1.0));
                let enter = hi.min(lid);
                (enter > lo).then_some((enter, z, lid <= hi))
            })
            .collect();
        hits.sort_by(|a, b| b.0.total_cmp(&a.0));
        let second = hits.get(1).map_or(f64::NEG_INFINITY, |s| s.0);
        hits.first().map(|&(y, z, top)| (y, z, top, second))
    }

    fn hills(x: i32, z: i32) -> Option<i16> {
        // Deterministic bumps from 60 to 72, with a few towers.
        let n = (x.wrapping_mul(73_856_093) ^ z.wrapping_mul(19_349_663)) as u32;
        let base = 64 + ((x / 3 + z / 4).rem_euclid(5)) as i16 - 2;
        Some(if n.is_multiple_of(37) {
            base + 8
        } else {
            base + (n % 3) as i16
        })
    }

    #[test]
    fn the_baker_matches_a_ray_trace_through_the_columns() {
        let p = Projection::default();
        let mut m = Mosaic::new(p, 64);
        // 2 x 4 chunks; the mosaic covers x 0..32 and the middle of z.
        world(&mut m, 0..2, 0..4, hills);
        m.shift(0, p.v(64.0, 32.0).floor() as i32 / 8 * 8 - 16);
        paint_all(&mut m, &Ids);
        let mut checked = 0;
        for j in 0..64 {
            for i in 0..64 {
                let Some((y, z, top, second)) = trace(&m, &hills, 0..64, i, j) else {
                    continue;
                };
                // The last row's south faces are not drawn (no neighbour), and
                // a near tie between two faces may go either way.
                if (z == 63 && !top) || y - second < 1e-4 {
                    continue;
                }
                let got = m.rgb_at(i, j).map(unid);
                let x = m.anchor().0 + (i / 2) as i32;
                assert_eq!(got, Some((x, z, top)), "texel ({i}, {j})");
                assert!((f64::from(m.height_at(i, j)) - y).abs() < 1e-3);
                checked += 1;
            }
        }
        assert!(checked > 3000, "{checked}");
    }

    /// Flat ground at 64 over 3 x 3 chunks with a tower and a staircase.
    fn scene(x: i32, z: i32) -> Option<i16> {
        match (x, z) {
            (8, 24) => Some(70),                         // a tower 6 blocks up
            (20, 18..=23) => Some(64 + (24 - z) as i16), // stairs rising north
            _ => Some(64),
        }
    }

    fn scene_mosaic() -> Mosaic {
        let p = Projection::default();
        let mut m = Mosaic::new(p, 64);
        world(&mut m, 0..2, 0..3, scene);
        m.recentre(16.0, p.v(64.0, 24.0));
        paint_all(&mut m, &Ids);
        m
    }

    fn texel_rows(m: &Mosaic, x: i32) -> Vec<Option<(i32, i32, bool)>> {
        let i = ((x - m.anchor().0) * 2) as u32;
        (0..m.side()).map(|j| m.rgb_at(i, j).map(unid)).collect()
    }

    #[test]
    fn the_tower_top_is_drawn_north_and_up_and_hides_the_ground_behind_it() {
        let m = scene_mosaic();
        let rows = texel_rows(&m, 8);
        let top = rows
            .iter()
            .position(|r| *r == Some((8, 24, true)))
            .expect("tower top");
        let south = rows
            .iter()
            .position(|r| *r == Some((8, 24, false)))
            .expect("tower side");
        assert!(top < south, "the top is above its south face");
        // 6 blocks of face, cos map units each, 2 texels per unit: about 7.7 rows.
        let side_rows = rows.iter().filter(|r| **r == Some((8, 24, false))).count();
        assert!((7..=8).contains(&side_rows), "{side_rows}");
        // The ground column the tower stands on, and the ones just north of it,
        // are hidden: the tower covers about 6*cos/sin = 5 blocks behind it.
        for z in 19..=23 {
            assert!(!rows.contains(&Some((8, z, true))), "ground at z {z} shows");
        }
        assert!(rows.contains(&Some((8, 18, true))));
        // Where the tower's top is drawn, the ground at that map row is z 24
        // minus 6*cos/sin blocks: the top sits north and up of its base.
        let p = m.projection();
        let (_, v0) = m.anchor();
        let v_top = bake::row_v(top as i64, f64::from(v0));
        assert!((p.z_at(v_top, 65.0) - 24.0 + 6.0 * p.cos / p.sin).abs() < 1.0);
    }

    #[test]
    fn the_staircase_reads_as_steps() {
        let m = scene_mosaic();
        let rows = texel_rows(&m, 20);
        // Going down the map: top, side, top, side ... for each step.
        let mut runs: Vec<(i32, bool)> = Vec::new();
        for (_, z, top) in rows.iter().flatten().copied() {
            if (18..=23).contains(&z) && runs.last() != Some(&(z, top)) {
                runs.push((z, top));
            }
        }
        let want: Vec<(i32, bool)> = (18..=23).flat_map(|z| [(z, true), (z, false)]).collect();
        assert_eq!(runs, want);
    }

    #[test]
    fn higher_faces_win_the_depth_test() {
        // A tower 16 blocks up at z 16 on ground at 64: on the map it overlaps
        // the ground north of it, and the higher hit (nearer along the ray) wins.
        let p = Projection::default();
        let h = |x: i32, z: i32| match (x, z) {
            (4, 16) => Some(80),
            (4, _) => Some(64),
            _ => None,
        };
        let mut m = Mosaic::new(p, 64);
        world(&mut m, 0..1, 0..2, h);
        m.recentre(4.0, p.v(72.0, 12.0));
        paint_all(&mut m, &Ids);
        let rows = texel_rows(&m, 4);
        assert!(rows.contains(&Some((4, 16, true))) && rows.contains(&Some((4, 16, false))));
        // It hides 16*cos/sin = 13.4 blocks of ground behind it: z 2.57 to 16.
        for z in 3..=15 {
            assert!(!rows.contains(&Some((4, z, true))), "ground at z {z} shows");
        }
        assert!(rows.contains(&Some((4, 1, true))));
        // Painting the chunks in the other order gives the same image.
        let mut n = Mosaic::new(p, 64);
        n.shift(m.anchor().0, m.anchor().1);
        world(&mut n, 0..1, 1..2, h);
        world(&mut n, 0..1, 0..1, h);
        paint_all(&mut n, &Ids);
        assert_eq!(m.pixels(), n.pixels());
    }

    #[test]
    fn a_south_face_takes_each_blocks_own_state() {
        let p = Projection::default();
        let mut m = Mosaic::new(p, 32);
        // A 3-block pillar: top state 10, then 11, then 12; ground (state 1) at 64.
        let mut c = Chunk::from_fn(3, |_, _| Some((64, 1)));
        let i = 5 * 16 + 5;
        c.tops[i] = 67;
        c.states[i * 3..i * 3 + 3].copy_from_slice(&[10, 11, 12]);
        m.set_chunk((0, 0), Some(c));
        m.recentre(5.0, p.v(64.0, 6.0));
        let shade = MapColours {
            colours: (0..16).map(|k| k * 0x10_1010).collect(),
        };
        paint_all(&mut m, &shade);
        let col = ((5 - m.anchor().0) * 2) as u32;
        let seen: Vec<u32> = (0..32).filter_map(|j| m.rgb_at(col, j)).collect();
        for s in [10u32, 11, 12] {
            assert!(
                seen.contains(&bake::scale_rgb(s * 0x10_1010, bake::SIDE_LIGHT)),
                "state {s}"
            );
        }
        assert!(seen.contains(&(10 * 0x10_1010)));
    }

    #[test]
    fn setting_a_chunk_marks_its_rows_and_its_north_neighbours() {
        let p = Projection::default();
        let mut m = Mosaic::new(p, 64);
        m.shift(0, p.v(64.0, 16.0).floor() as i32 / 8 * 8 - 16);
        world(&mut m, 0..1, 0..1, |_, _| Some(64));
        assert!(m.paint_rows(0).is_some());
        paint_all(&mut m, &Ids);
        // Chunk (0,0)'s last row has no south faces yet (no neighbour); a lower
        // chunk to the south gives it faces, so (0,0)'s rows are dirty again.
        world(&mut m, 0..1, 1..2, |_, _| Some(60));
        let (a, b) = m.paint_rows(0).expect("dirty");
        let s = bake::spans(&p, 15, faces(64, Some(Some(60))));
        let (fa, fb) = bake::rows(s.south.0, s.south.1, f64::from(m.anchor().1));
        assert!(
            i64::from(a) <= fa.max(0) && i64::from(b) >= fb.min(64),
            "{a}..{b} vs {fa}..{fb}"
        );
        paint_all(&mut m, &Ids);
        let rows = texel_rows(&m, 3);
        assert!(
            rows.contains(&Some((3, 15, false))),
            "the step's face shows"
        );
        // A chunk far outside the image marks nothing.
        world(&mut m, 9..10, 0..1, |_, _| Some(64));
        assert!(!m.painting());
    }

    #[test]
    fn removing_a_block_leaves_nothing_behind() {
        let p = Projection::default();
        let mut m = scene_mosaic();
        let before = texel_rows(&m, 8);
        assert!(before.contains(&Some((8, 24, false))));
        // The tower goes: its chunk is read again.
        world(&mut m, 0..1, 1..2, |_, _| Some(64));
        paint_all(&mut m, &Ids);
        let after = texel_rows(&m, 8);
        assert!(!after.iter().flatten().any(|&(_, z, top)| z == 24 && !top));
        // And it equals a fresh bake of the same world.
        let mut fresh = Mosaic::new(p, 64);
        fresh.shift(m.anchor().0, m.anchor().1);
        world(&mut fresh, 0..2, 0..3, |x, z| {
            if (x, z) == (8, 24) {
                Some(64)
            } else {
                scene(x, z)
            }
        });
        paint_all(&mut fresh, &Ids);
        assert_eq!(m.pixels(), fresh.pixels());
    }

    #[test]
    fn an_unloaded_chunk_is_cleared() {
        let mut m = scene_mosaic();
        m.set_chunk((1, 1), None);
        m.set_chunk((1, 0), None);
        m.set_chunk((1, 2), None);
        paint_all(&mut m, &Ids);
        let s = m.strip_of(1).unwrap();
        for j in 0..64 {
            for i in s * STRIP..(s + 1) * STRIP {
                assert_eq!(m.rgb_at(i, j), None);
            }
        }
        assert_eq!(m.chunks(), 3);
    }

    #[test]
    fn recentring_moves_the_texels_and_paints_only_what_it_uncovered() {
        let p = Projection::default();
        let h = |x: i32, z: i32| hills(x, z);
        let mut m = Mosaic::new(p, 128);
        world(&mut m, -4..8, -4..8, h);
        m.recentre(40.0, p.v(64.0, 40.0));
        paint_all(&mut m, &Ids);
        let a = m.anchor();
        // Walk east and south until it recentres.
        assert!(!m.recentre(41.0, p.v(64.0, 41.0)));
        assert!(m.recentre(70.0, p.v(64.0, 62.0)));
        let b = m.anchor();
        assert_ne!(a, b);
        let (di, dj) = (((b.0 - a.0) * 2) as u32, ((b.1 - a.1) * 2) as u32);
        // Only the uncovered strips and rows wait to be painted.
        assert!(m.painting());
        let strips = 128 / STRIP;
        for s in 0..strips {
            let rows = m.paint_rows(s).expect("rows");
            if s < strips - di / STRIP {
                assert_eq!(rows, (128 - dj, 128), "strip {s}");
            } else {
                assert_eq!(rows, (0, 128), "strip {s}");
            }
        }
        // A kept texel moved with the world.
        let kept = m.rgb_at(10, 10);
        let mut again = Mosaic::new(p, 128);
        again.shift(a.0, a.1);
        world(&mut again, -4..8, -4..8, h);
        paint_all(&mut again, &Ids);
        assert_eq!(kept, again.rgb_at(10 + di, 10 + dj));
        // After painting, it is the same as a fresh bake at the new anchor.
        paint_all(&mut m, &Ids);
        let mut fresh = Mosaic::new(p, 128);
        fresh.shift(b.0, b.1);
        world(&mut fresh, -4..8, -4..8, h);
        paint_all(&mut fresh, &Ids);
        assert_eq!(m.pixels(), fresh.pixels());
        assert_eq!(m.stats.recentres, 2);
    }

    #[test]
    fn a_far_jump_clears_and_repaints_everything() {
        let p = Projection::default();
        let mut m = Mosaic::new(p, 64);
        world(&mut m, 0..2, 0..2, |_, _| Some(64));
        paint_all(&mut m, &Ids);
        m.shift(1600, 800);
        assert!((0..2).all(|s| m.paint_rows(s) == Some((0, 64))));
        assert!(m.rgb_at(0, 0).is_none());
    }

    /// One grey for every face, with outlines; water `wet` blocks deep over
    /// columns at x >= 24 (seen at the floor `wet` lower).
    struct Grey {
        wet: f32,
    }
    impl Shade for Grey {
        fn top(&self, c: &Chunk, col: usize, y: f64) -> Texel {
            let x = (c.state(col, 0) & 511) as i32 - 100;
            if x >= 24 && self.wet > 0.0 {
                return Texel {
                    rgb: 0x64_6464,
                    seen: (y - f64::from(self.wet)) as f32,
                    water: self.wet / 0.766,
                };
            }
            Texel::dry(0x64_6464, y)
        }
        fn south(&self, _: &Chunk, _: usize, _: i32, y: f64) -> Texel {
            Texel::dry(0x64_6464, y)
        }
        fn outline(&self) -> f32 {
            0.38
        }
    }

    #[test]
    fn outlines_ring_the_ground_behind_a_tower_and_not_flat_ground() {
        let p = Projection::default();
        let mut m = Mosaic::new(p, 64);
        world(&mut m, 0..2, 0..3, scene);
        m.recentre(16.0, p.v(64.0, 24.0));
        paint_all(&mut m, &Grey { wet: 0.0 });
        let i = ((8 - m.anchor().0) * 2) as u32;
        // The highest texel row of the tower's top, and the ground row above it
        // (further along the view): that one is outlined, 100 * 0.62 = 62.
        let tower = (0..64)
            .find(|&j| m.height_at(i, j) == 71.0)
            .expect("tower top");
        assert_eq!(m.shown_at(i, tower - 1, 0.38)[..3], [62, 62, 62]);
        assert_eq!(m.shown_at(i, tower, 0.38)[..3], [100, 100, 100]);
        // Flat ground two blocks west is not.
        let flat = i - 4;
        assert!((0..64)
            .filter(|&j| m.height_at(flat, j) == 65.0)
            .all(|j| m.shown_at(flat, j, 0.38)[..3] == [100, 100, 100]));
        // Strength 0 is the plain image.
        assert_eq!(m.shown_at(i, tower - 1, 0.0)[..3], [100, 100, 100]);
    }

    #[test]
    fn a_one_block_step_is_outlined_and_water_fades_it() {
        // Ground at 64 north of z 20 and 65 from there south; the rows just
        // behind the step's edge are outlined (1/sin + 0.5 cot > 1.5).
        let p = Projection::default();
        let step = |_: i32, z: i32| Some(if z >= 20 { 65 } else { 64 });
        let outlined = |wet: f32| {
            let mut m = Mosaic::new(p, 64);
            world(&mut m, 0..2, 0..3, step);
            m.recentre(16.0, p.v(64.0, 20.0));
            paint_all(&mut m, &Grey { wet });
            let i = ((26 - m.anchor().0) * 2) as u32;
            (0..64)
                .filter(|&j| m.rgb_at(i, j).is_some())
                .map(|j| m.shown_at(i, j, 0.38)[0])
                .min()
                .unwrap()
        };
        assert_eq!(outlined(0.0), 62);
        // Under 3 blocks of water the darkening is exp(-path/6) as strong.
        let path: f32 = 3.0 / 0.766;
        let keep = 1.0 - 0.38 * (-((path * 8.0).round() / 8.0) / 6.0).exp();
        assert_eq!(outlined(3.0), (100.0 * keep).round() as u8);
    }

    #[test]
    fn painting_with_outlines_uploads_the_neighbouring_rows_and_strips() {
        let p = Projection::default();
        let mut m = Mosaic::new(p, 96);
        world(&mut m, 0..3, 0..3, |_, _| Some(64));
        m.recentre(24.0, p.v(64.0, 24.0));
        paint_all(&mut m, &Grey { wet: 0.0 });
        while m.take_upload(1 << 20, 0.38, |_, _, _, _, _| {}) > 0 {}
        m.set_chunk(
            (1, 1),
            Some(Chunk::from_fn(1, |_, _| Some((64, id(20, 20))))),
        );
        let rows = m.paint_rows(1).expect("dirty");
        paint_all(&mut m, &Grey { wet: 0.0 });
        let mut rects = Vec::new();
        m.take_upload(1 << 20, 0.38, |x, y, w, h, _| rects.push((x, y, w, h)));
        assert_eq!(rects, vec![(0, rows.0 - 1, 96, rows.1 - rows.0 + 2)]);
    }

    #[test]
    fn uploads_go_in_rects_under_the_byte_cap() {
        let mut m = scene_mosaic();
        // Everything is dirty after the recentre; drain it 4 KiB at a time.
        let mut got = vec![0u8; 64 * 64 * 4];
        let mut calls = 0;
        loop {
            let n = m.take_upload(4096, 0.0, |x, y, w, h, rgba| {
                assert_eq!(rgba.len(), (w * h * 4) as usize);
                for r in 0..h {
                    let at = (((y + r) * 64 + x) * 4) as usize;
                    got[at..at + (w * 4) as usize]
                        .copy_from_slice(&rgba[(r * w * 4) as usize..((r + 1) * w * 4) as usize]);
                }
            });
            assert!(n <= 4096);
            if n == 0 {
                break;
            }
            calls += 1;
        }
        assert!(!m.uploading());
        assert_eq!(calls, 64 * 64 * 4 / 4096);
        assert_eq!(got, m.pixels());
    }

    #[test]
    fn empty_columns_draw_nothing() {
        let p = Projection::default();
        let mut m = Mosaic::new(p, 32);
        m.set_chunk((0, 0), Some(Chunk::from_fn(1, |_, _| None)));
        assert!(!m.painting());
        let c = Chunk::new(vec![NO_TOP; 256], vec![0; 256], 1);
        m.set_chunk((0, 1), Some(c));
        paint_all(&mut m, &Ids);
        assert!(m.pixels().iter().all(|&b| b == 0));
    }
}
