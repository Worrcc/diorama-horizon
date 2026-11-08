//! Diorama as a Horizon client mod (wasm32, API level 17). M3: the column
//! baker. Chunks are read with `client-world.columns`, baked into a 512 x 512
//! mosaic (top and south faces, nearest face per texel), uploaded in dirty
//! rects and shown centred on the player at the top right. `changes` is polled
//! every tick; the mosaic recentres as the player walks.
//!
//! Colours are a placeholder until M4: each state's map colour as the game
//! reports it, south faces darker. All maths is in `diorama-core`; this file
//! only talks to Horizon.
use std::collections::BTreeSet;

use diorama_core::{
    bake::{Chunk, MapColours, TEXELS_PER_BLOCK},
    layout, Mosaic, Projection,
};
use horizon_mod_sdk::{
    canvas::{self, ImageShape, Shape},
    client_world::{self, Layer},
    game,
    render::{self, Corner, FillShape, Point, QuadShape, Rect},
    Mod, Registration,
};

/// The mosaic's side in texels: 256 blocks across at 2 texels per block.
const SIDE: u32 = 512;
/// Chunks read around the player, and how far cached ones are kept.
const RADIUS: u32 = 12;
const FORGET: i32 = 14;
/// States read per column from the top down.
const DEPTH: u8 = 8;
/// Chunks per `columns` call.
const BATCH: usize = 4;
/// The frame budget asked for (PLAN-v2 section 3.2), in microseconds.
const BUDGET_US: u32 = 300;
/// `work-left` is about this at the start of a frame call.
const WORK_SCALE: u64 = 1_000_000;
/// Work kept back for the upload and the corner box.
const RESERVE: u64 = 200_000;
/// Texel bytes uploaded per frame at most (the host allows 512 KiB).
const UPLOAD_BYTES: usize = 256 * 1024;
/// Rows of one strip painted per step.
const MAX_ROWS: u32 = 64;
/// Frames per cost line in the log.
const LOG_FRAMES: u32 = 600;

/// GUI pixels per block (Diorama's default zoom, Settings.java:69).
const ZOOM: f32 = 2.0;
/// Box side as a percentage of the GUI width.
const SIZE_PERCENT: f32 = 10.0;
const BORDER: f32 = 1.0;
const BORDER_COLOR: u32 = 0xFF20_2020;
const ARROW_COLOR: u32 = 0xFFE8_3030;
const ARROW_OUTLINE: u32 = 0xFF00_0000;

/// The bake's cost over a window of frames, from `work-left` (the host
/// measures the mod's fuel rate per device, host calls included).
#[derive(Default)]
struct Cost {
    frames: u32,
    busy: u32,
    sum_us: u64,
    busy_us: u64,
    max_us: u64,
    chunks: u64,
    uploaded: u64,
    /// `work-left` units spent reading chunks, painting and uploading.
    fetch: u64,
    paint: u64,
    upload: u64,
}

struct Diorama {
    mosaic: Mosaic,
    shade: MapColours,
    image: canvas::Image,
    granted: u32,
    /// `changes`' counter for the next poll.
    since: u64,
    /// Chunks to read, nearest first when taken.
    pending: BTreeSet<(i32, i32)>,
    player_chunk: Option<(i32, i32)>,
    dimension: Option<String>,
    /// What the box last showed, so it is only sent again when it changes.
    shown: Option<(f32, [i32; 5])>,
    cost: Cost,
    /// A few chunks a poll said changed, and the frame count then: logged when
    /// they are on the map.
    watch: Option<(u32, Vec<(i32, i32)>)>,
    frame_no: u32,
}

impl Diorama {
    fn reset(&mut self) {
        self.mosaic.clear();
        self.pending.clear();
        self.since = 0;
        self.player_chunk = None;
        self.watch = None;
    }

    /// Reads up to [`BATCH`] pending chunks nearest `centre` into the mosaic.
    fn fetch(&mut self, centre: (i32, i32)) -> usize {
        let mut near: Vec<(i32, i32)> = Vec::with_capacity(BATCH + 1);
        let dist = |c: &(i32, i32)| (c.0 - centre.0).abs().max((c.1 - centre.1).abs());
        for &c in &self.pending {
            if near.len() < BATCH || dist(&c) < dist(&near[BATCH - 1]) {
                let at = near.partition_point(|n| dist(n) <= dist(&c));
                near.insert(at, c);
                near.truncate(BATCH);
            }
        }
        for c in &near {
            self.pending.remove(c);
        }
        match client_world::columns(&near, Layer::Surface, DEPTH) {
            Ok(read) => {
                for (key, cols) in near.iter().zip(read) {
                    let chunk = cols.map(|c| Chunk {
                        tops: c.tops,
                        states: c.states,
                        depth: DEPTH,
                    });
                    self.mosaic.set_chunk(*key, chunk);
                }
            }
            Err(e) => horizon_mod_sdk::warn(&format!("columns: {e}")),
        }
        near.len()
    }

    fn draw(&mut self, p: &game::Player) {
        let (gui_w, _) = render::gui_size();
        let side = layout::box_side(gui_w, SIZE_PERCENT);
        let proj = *self.mosaic.projection();
        let (u, v) = (proj.u(p.x), proj.v(p.y, p.z));
        let anchor = self.mosaic.anchor();
        // Only resend when the view moved by a tenth of a texel or a degree.
        let key = (
            side,
            [
                (u * 20.0) as i32,
                (v * 20.0) as i32,
                p.yaw.rem_euclid(360.0) as i32,
                anchor.0,
                anchor.1,
            ],
        );
        if self.shown == Some(key) {
            return;
        }
        self.shown = Some(key);

        let (cx, cy) = self.mosaic.texel_of(u, v);
        let inner = side - 2.0 * BORDER;
        let (sx, sy, sw, sh) = layout::src_window(cx, cy, inner, ZOOM, TEXELS_PER_BLOCK, SIDE);
        let mid = side / 2.0;
        let mut shapes = vec![
            Shape::Fill(FillShape {
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: side,
                    h: side,
                },
                color: BORDER_COLOR,
            }),
            Shape::Image(ImageShape {
                image: self.image,
                src: Rect {
                    x: sx,
                    y: sy,
                    w: sw,
                    h: sh,
                },
                dst: Rect {
                    x: BORDER,
                    y: BORDER,
                    w: inner,
                    h: inner,
                },
                tint: 0xFFFF_FFFF,
                round: false,
            }),
        ];
        // The arrow sits where the player is in the window (off centre only
        // when the window is clipped at the mosaic's edge).
        let scale = inner / sw;
        let (ax, ay) = (
            BORDER + (cx as f32 - sx) * scale,
            BORDER + (cy as f32 - sy) * scale,
        );
        let ax = if ax.is_finite() { ax } else { mid };
        let ay = if ay.is_finite() { ay } else { mid };
        let size = (side / 10.0).clamp(6.0, 14.0);
        for (s, color) in [(size + 2.0, ARROW_OUTLINE), (size, ARROW_COLOR)] {
            for q in layout::arrow(ax, ay, s, p.yaw) {
                let pt = |(x, y): (f32, f32)| Point { x, y };
                shapes.push(Shape::Quad(QuadShape {
                    corners: (pt(q[0]), pt(q[1]), pt(q[2]), pt(q[3])),
                    color,
                }));
            }
        }
        canvas::set_corner(Corner::TopRight, side, side, &shapes);
    }

    /// Adds a frame's cost to the window and logs it when the window is full.
    fn account(&mut self, busy: bool, chunks: usize, uploaded: usize) {
        let left = canvas::work_left();
        let used = WORK_SCALE.saturating_sub(left);
        let us = used * u64::from(self.granted) / WORK_SCALE;
        let c = &mut self.cost;
        c.frames += 1;
        c.sum_us += us;
        c.max_us = c.max_us.max(us);
        c.chunks += chunks as u64;
        c.uploaded += uploaded as u64;
        if busy {
            c.busy += 1;
            c.busy_us += us;
        }
        if c.frames >= LOG_FRAMES {
            let s = self.mosaic.stats;
            horizon_mod_sdk::info(&format!(
                "bake: {} frames, avg {} us/frame, busy {} frames avg {} us, max {} us \
                 (budget {} us); split read {} us, paint {} us, upload {} us; \
                 {} chunks read, {} KiB uploaded; {} cached, {} pending; \
                 totals: {} jobs, {} columns, {} texels, {} recentres",
                c.frames,
                c.sum_us / u64::from(c.frames),
                c.busy,
                c.busy_us.checked_div(u64::from(c.busy)).unwrap_or(0),
                c.max_us,
                self.granted,
                c.fetch * u64::from(self.granted) / WORK_SCALE,
                c.paint * u64::from(self.granted) / WORK_SCALE,
                c.upload * u64::from(self.granted) / WORK_SCALE,
                c.chunks,
                c.uploaded / 1024,
                self.mosaic.chunks(),
                self.pending.len(),
                s.jobs,
                s.columns,
                s.texels,
                s.recentres,
            ));
            self.cost = Cost::default();
        }
    }
}

impl Mod for Diorama {
    fn init() -> Result<(Self, Registration), String> {
        let image = canvas::create_image(SIDE, SIDE)?;
        let granted = canvas::request_frame_budget(BUDGET_US);
        let table = client_world::block_table();
        horizon_mod_sdk::info(&format!(
            "diorama: {SIDE}x{SIDE} mosaic, frame budget {granted} us, {} block states",
            table.map_color.len()
        ));
        let state = Diorama {
            mosaic: Mosaic::new(Projection::default(), SIDE),
            shade: MapColours {
                colours: table.map_color,
            },
            image,
            granted,
            since: 0,
            pending: BTreeSet::new(),
            player_chunk: None,
            dimension: None,
            shown: None,
            cost: Cost::default(),
            watch: None,
            frame_no: 0,
        };
        Ok((state, Registration::new().on_frame().on_tick()))
    }

    fn tick(&mut self, _tick: u64) {
        let Some(p) = game::get_player() else {
            if self.dimension.take().is_some() {
                self.reset();
            }
            return;
        };
        if self.dimension.as_deref() != Some(p.dimension.as_str()) {
            self.reset();
            self.dimension = Some(p.dimension.clone());
        }
        let pc = ((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4);
        if self.player_chunk != Some(pc) {
            // Chunks loaded long ago come into range as the player walks:
            // `changes` from 0 lists every loaded one.
            self.player_chunk = Some(pc);
            self.mosaic.forget_far(pc, FORGET);
            let (_, all) = client_world::changes(0, RADIUS);
            let fresh: Vec<_> = all.into_iter().filter(|c| !self.mosaic.has(*c)).collect();
            self.pending.extend(fresh);
        }
        let first = self.since == 0;
        let (now, changed) = client_world::changes(self.since, RADIUS);
        self.since = now;
        if !first && !changed.is_empty() && changed.len() <= 4 && self.watch.is_none() {
            horizon_mod_sdk::info(&format!("changes: {changed:?} (counter {now})"));
            self.watch = Some((self.frame_no, changed.clone()));
        }
        self.pending.extend(changed);
    }

    fn frame(&mut self, _seconds: f32) {
        self.frame_no = self.frame_no.wrapping_add(1);
        let Some(p) = game::get_player() else {
            if self.shown.take().is_some() {
                canvas::clear_corner();
            }
            return;
        };
        let proj = *self.mosaic.projection();
        let (u, v) = (proj.u(p.x), proj.v(p.y, p.z));
        let centre = self
            .player_chunk
            .unwrap_or(((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4));
        let mut busy = self.mosaic.recentre(u, v);
        let (mut chunks, mut batches) = (0, 0);
        loop {
            let left = canvas::work_left();
            if left <= RESERVE {
                break;
            }
            if !self.pending.is_empty() && (batches < 2 || !self.mosaic.painting()) {
                chunks += self.fetch(centre);
                batches += 1;
                self.cost.fetch += left.saturating_sub(canvas::work_left());
            } else if self.mosaic.paint_step(&self.shade, u, MAX_ROWS) {
                self.cost.paint += left.saturating_sub(canvas::work_left());
            } else {
                break;
            }
            busy = true;
        }
        let before_upload = canvas::work_left();
        let image = self.image;
        let mut failed = None;
        let uploaded = self.mosaic.take_upload(UPLOAD_BYTES, |x, y, w, h, rgba| {
            if let Err(e) = canvas::update_image(image, x, y, w, h, rgba) {
                failed = Some(e);
            }
        });
        if let Some(e) = failed {
            horizon_mod_sdk::warn(&format!("update-image: {e}"));
        }
        self.cost.upload += before_upload.saturating_sub(canvas::work_left());
        if let Some((at, keys)) = &self.watch {
            let done = !self.mosaic.painting()
                && !self.mosaic.uploading()
                && keys.iter().all(|k| !self.pending.contains(k));
            if done {
                horizon_mod_sdk::info(&format!(
                    "changes {keys:?} on the map {} frames after the poll",
                    self.frame_no.wrapping_sub(*at)
                ));
                self.watch = None;
            }
        }
        self.draw(&p);
        self.account(busy || uploaded > 0, chunks, uploaded);
    }
}
horizon_mod_sdk::export_mod!(Diorama);
