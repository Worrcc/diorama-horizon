//! Diorama as a Horizon client mod (wasm32, API level 17). Chunks are read
//! with `client-world.columns`, baked into a 512 x 512 mosaic (top and south
//! faces, nearest face per texel), uploaded in dirty rects and shown centred
//! on the player at the top right. `changes` is polled every tick; the mosaic
//! recentres as the player walks.
//!
//! M4: Diorama's colours. Every block colour is computed here at run time
//! from the loaded resource packs: the block table names each state's
//! textures, `client-world.texture` gives their pixels and diorama-core
//! averages them (RULES.md section 3). `table-version` is polled every tick;
//! after a resource reload the textures are averaged again over the next
//! frames and the map repainted. All maths is in `diorama-core`; this file
//! only talks to Horizon.
use std::collections::BTreeSet;

use diorama_core::{
    bake::{Chunk, TEXELS_PER_BLOCK},
    cave, layout,
    layout::Corner as MapCorner,
    settings::{self, Settings},
    looks::{self, Average, Builder, Face, Looks, Table, Tint},
    style, Mosaic, Painter, Projection,
};
use horizon_mod_sdk::{
    canvas::{self, ImageShape, Shape},
    client_world::{self, BlockStates, Layer, TintSource},
    game, input,
    render::{self, Corner, FillShape, Point, QuadShape, Rect, TextShape},
    storage,
    ui::{self, ButtonWidget, Widget},
    Mod, Registration,
};

/// The mosaic's side in texels: 256 blocks across at 2 texels per block.
const SIDE: u32 = 512;
/// Chunks read around the player, and how far cached ones are kept.
const RADIUS: u32 = 12;
const FORGET: i32 = 14;
/// States read per column from the top down: enough to see through plants
/// and glass and to find the floor of shallow water.
const DEPTH: u8 = 12;
/// Block looks built per step of a rebuild.
const LOOKS_STEP: usize = 2048;
/// Chunks per `columns` call.
const BATCH: usize = 4;
/// The frame budget asked for (PLAN-v2 section 3.2), in microseconds.
const BUDGET_US: u32 = 300;
/// `work-left` is about this at the start of a frame call.
const WORK_SCALE: u64 = 1_000_000;
/// Work kept back for the upload and the corner box.
const RESERVE: u64 = 350_000;
/// Work kept back for the corner box once uploading.
const UPLOAD_RESERVE: u64 = 60_000;
/// Texel bytes uploaded per piece.
const UPLOAD_PIECE: usize = 8 * 1024;
/// Texel bytes uploaded per frame at most (the host allows 512 KiB).
const UPLOAD_BYTES: usize = 256 * 1024;
/// Rows of one strip painted per step.
const MAX_ROWS: u32 = 64;
/// Frames per cost line in the log.
const LOG_FRAMES: u32 = 600;

const BORDER: f32 = 1.0;
/// The eye above the feet, for the light and the slice.
const EYE: f64 = 1.62;
/// Ticks between looks at the light (the cave rule).
const LAYER_TICKS: u64 = 4;
const TEXT_COLOR: u32 = 0xFF_FFFF;
const NIGHT_COLOR: u32 = 0x9C_C4FF;
const NORTH_COLOR: u32 = 0xFF_6B6B;
const LETTER_BOX: u32 = 0xA010_1418;
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

/// A rebuild of the looks after the packs reloaded, done a step at a time.
struct Rebuild {
    table: Table,
    /// The sprites to average, and the next one.
    sprites: Vec<u32>,
    next: usize,
    averages: Vec<Option<Average>>,
    missing: usize,
    builder: Builder,
}

impl Rebuild {
    fn new(table: Table) -> Rebuild {
        Rebuild {
            sprites: table.used_sprites(),
            next: 0,
            averages: vec![None; table.sprites.len()],
            missing: 0,
            builder: Builder::new(&table),
            table,
        }
    }
    /// Averages one more texture or builds a few more looks; true when done.
    fn step(&mut self) -> bool {
        if let Some(&s) = self.sprites.get(self.next) {
            self.next += 1;
            let name = &self.table.sprites[s as usize];
            match client_world::texture(name) {
                Some(t) => {
                    self.averages[s as usize] = Some(looks::average(t.width, t.height, &t.rgba))
                }
                None => self.missing += 1,
            }
            return false;
        }
        self.builder.step(&self.table, &self.averages, LOOKS_STEP)
    }
    /// Everything at once (in `init`, with its large budget).
    fn run(mut self) -> (Looks, Table, usize) {
        while !self.step() {}
        let n = self.sprites.len();
        (self.builder.finish(&self.table), self.table, n)
    }
}

/// The host's block table as diorama-core reads it.
fn table_of(t: BlockStates) -> Table {
    let face = |f: &client_world::Face| Face {
        sprite: f.sprite,
        tint: match f.tint {
            TintSource::None => Tint::None,
            TintSource::Grass => Tint::Grass,
            TintSource::Foliage => Tint::Foliage,
            TintSource::DryFoliage => Tint::DryFoliage,
            TintSource::Water => Tint::Water,
            TintSource::Constant => Tint::Constant,
        },
    };
    Table {
        version: t.version,
        state_top: t.state_top.iter().map(face).collect(),
        state_side: t.state_side.iter().map(face).collect(),
        blocks: t.blocks,
        sprites: t.sprites,
        state_block: t.state_block,
        state_flags: t.state_flags,
        constant_tint: t.constant_tint,
        map_color: t.map_color,
    }
}

/// `#RRGGBB`.
fn hex(c: u32) -> String {
    format!("#{:06X}", c & 0xFF_FFFF)
}

struct Diorama {
    mosaic: Mosaic,
    painter: Painter,
    /// Block names and each state's block, for `/diorama colour`.
    blocks: Vec<String>,
    state_block: Vec<u16>,
    rebuild: Option<Rebuild>,
    image: canvas::Image,
    granted: u32,
    /// `changes`' counter for the next poll.
    since: u64,
    /// Chunks to read, nearest first when taken.
    pending: BTreeSet<(i32, i32)>,
    player_chunk: Option<(i32, i32)>,
    dimension: Option<String>,
    /// What the box last showed, so it is only sent again when it changes.
    shown: Option<Shown>,
    settings: Settings,
    /// Bumped when a setting or the info lines change: the box is sent again.
    rev: u32,
    /// The lines under the map, and whether the clock one is at night.
    info: Vec<(String, u32)>,
    /// The dimension has a roof (the Nether), and the cave rule's state: the
    /// player is underground, and the height of the slice (none: the surface).
    ceiling: bool,
    underground: bool,
    cut: Option<i32>,
    layer_tick: u64,
    cost: Cost,
    /// A few chunks a poll said changed, and the frame count then: logged when
    /// they are on the map.
    watch: Option<(u32, Vec<(i32, i32)>)>,
    frame_no: u32,
}

/// What the corner box last showed.
#[derive(PartialEq)]
struct Shown {
    rev: u32,
    side: f32,
    view: [i32; 5],
}

impl Diorama {
    fn layer(&self) -> Layer {
        self.cut.map_or(Layer::Surface, Layer::Below)
    }

    /// The slice (or the surface) the cave rule wants now; when it changed,
    /// the cached chunks are read again in that layer.
    fn update_layer(&mut self, p: &game::Player) {
        let head = (p.y + EYE).floor() as i32;
        let (bx, bz) = (p.x.floor() as i32, p.z.floor() as i32);
        let sky = |dy: i32| client_world::light(bx, head + dy, bz).map(|l| l >> 4);
        let Some(here) = sky(0) else { return };
        self.underground = cave::underground(self.underground, self.ceiling, here);
        let s = &self.settings;
        let slice = cave::slice(
            s.flag(settings::AUTO_CAVES),
            s.flag(settings::PREFER_SURFACE),
            self.underground,
            || cave::depth_to_sky(|dy| sky(dy as i32)),
            self.ceiling,
        );
        let want = slice.then(|| cave::follow(self.cut, head));
        if want != self.cut {
            horizon_mod_sdk::info(&format!(
                "diorama: layer {:?} -> {:?} (underground {}, ceiling {}, head {head})",
                self.cut, want, self.underground, self.ceiling
            ));
            self.set_cut(want);
        }
    }

    fn set_cut(&mut self, cut: Option<i32>) {
        self.cut = cut;
        self.painter.cave = cut;
        self.mosaic.outline_depth = if cut.is_some() {
            style::CAVE_OUTLINE_DEPTH
        } else {
            style::OUTLINE_DEPTH
        };
        self.pending.extend(self.mosaic.keys());
        self.rev = self.rev.wrapping_add(1);
    }

    /// The lines under the map: coordinates, biome, game time.
    fn update_info(&mut self, p: &game::Player) {
        let s = &self.settings;
        let mut lines = Vec::new();
        let (x, y, z) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        if s.flag(settings::COORDINATES) {
            lines.push((format!("{x}, {y}, {z}"), TEXT_COLOR));
        }
        if s.flag(settings::BIOME) {
            if let Some(b) = client_world::biome(x, y, z) {
                lines.push((layout::biome_name(&b), TEXT_COLOR));
            }
        }
        if s.stored(settings::GAME_TIME) == "clock" {
            if let Some(c) = game::get_clock() {
                // Night, as vanilla counts the day: 13000 to 23000 ticks.
                let dark = (13000..23000).contains(&(c.day_time % 24000));
                lines.push((
                    layout::clock_text(c.day_time),
                    if dark { NIGHT_COLOR } else { TEXT_COLOR },
                ));
            }
        }
        if lines != self.info {
            self.info = lines;
            self.rev = self.rev.wrapping_add(1);
        }
    }

    fn page(&self) {
        let widgets: Vec<Widget> = (0..settings::SETTINGS.len())
            .map(|i| {
                Widget::Button(ButtonWidget {
                    id: i as u32 + 1,
                    label: self.settings.button(i),
                })
            })
            .collect();
        ui::set_settings(&widgets);
    }

    fn save(&self, i: usize) {
        let _ = storage::set(settings::SETTINGS[i].key, self.settings.stored(i).as_bytes());
    }

    /// Setting `i` changed (a button or a key): keep it, apply it, redraw.
    fn changed(&mut self, i: usize) {
        self.save(i);
        self.rev = self.rev.wrapping_add(1);
        match i {
            settings::MINIMAP => {
                if !self.settings.flag(settings::MINIMAP) {
                    canvas::clear_corner();
                    self.shown = None;
                }
            }
            settings::AUTO_CAVES | settings::PREFER_SURFACE => self.layer_tick = 0,
            settings::STYLE => {
                // The flat style's light and colours, on the default's camera.
                self.painter.style = if self.settings.flat_style() {
                    style::Style {
                        pitch_degrees: style::DEFAULT.pitch_degrees,
                        ..style::FLAT
                    }
                } else {
                    style::DEFAULT
                };
                self.mosaic.repaint_all();
            }
            _ => {}
        }
        self.page();
    }

    fn key(&mut self, name: &str) {
        match name {
            "toggle" => {
                self.settings.cycle(settings::MINIMAP);
                self.changed(settings::MINIMAP);
            }
            "zoom_in" | "zoom_out" => {
                self.settings
                    .zoom_by(if name == "zoom_in" { 1 } else { -1 });
                self.changed(settings::ZOOM);
            }
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.mosaic.clear();
        self.underground = false;
        self.layer_tick = 0;
        if self.cut.is_some() {
            self.set_cut(None);
        }
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
        match client_world::columns(&near, self.layer(), DEPTH) {
            Ok(read) => {
                for (key, cols) in near.iter().zip(read) {
                    let chunk = cols.map(|c| {
                        let mut chunk = Chunk::new(c.tops, c.states, DEPTH);
                        chunk.water = c.water;
                        chunk.light = c.light;
                        chunk.tints = c.tints;
                        self.painter.prepare(&mut chunk);
                        chunk
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
        let side = layout::box_side(gui_w, self.settings.size_percent());
        let proj = *self.mosaic.projection();
        let (u, v) = (proj.u(p.x), proj.v(p.y, p.z));
        let anchor = self.mosaic.anchor();
        // Only resend when the view moved by a tenth of a texel or a degree.
        let key = Shown {
            rev: self.rev,
            side,
            view: [
                (u * 20.0) as i32,
                (v * 20.0) as i32,
                p.yaw.rem_euclid(360.0) as i32,
                anchor.0,
                anchor.1,
            ],
        };
        if self.shown.as_ref() == Some(&key) {
            return;
        }
        self.shown = Some(key);

        let round = self.settings.round();
        // The box is as wide as its widest info line, the map against the
        // screen edge: right corners right-aligned, left corners left.
        let widths: Vec<f32> = self.info.iter().map(|(t, _)| render::text_width(t)).collect();
        let wide = widths.iter().fold(side, |m, w| m.max(w + 2.0));
        let right = matches!(
            self.settings.corner(),
            MapCorner::TopRight | MapCorner::BottomRight
        );
        let ox = if right { wide - side } else { 0.0 };
        let border = if round { 0.0 } else { BORDER };
        let (cx, cy) = self.mosaic.texel_of(u, v);
        let inner = side - 2.0 * border;
        let (sx, sy, sw, sh) = layout::src_window(
            cx,
            cy,
            inner,
            self.settings.zoom(),
            TEXELS_PER_BLOCK,
            SIDE,
        );
        let mid = side / 2.0;
        let mut shapes = Vec::new();
        if !round {
            shapes.push(Shape::Fill(FillShape {
                rect: Rect {
                    x: ox,
                    y: 0.0,
                    w: side,
                    h: side,
                },
                color: BORDER_COLOR,
            }));
        }
        shapes.push(Shape::Image(ImageShape {
            image: self.image,
            src: Rect {
                x: sx,
                y: sy,
                w: sw,
                h: sh,
            },
            dst: Rect {
                x: ox + border,
                y: border,
                w: inner,
                h: inner,
            },
            tint: (self.settings.alpha() << 24) | 0xFF_FFFF,
            round,
        }));
        // The arrow sits where the player is in the window (off centre only
        // when the window is clipped at the mosaic's edge).
        let scale = inner / sw;
        let (ax, ay) = (
            ox + border + (cx as f32 - sx) * scale,
            border + (cy as f32 - sy) * scale,
        );
        let ax = if ax.is_finite() { ax } else { ox + mid };
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
        if self.settings.flag(settings::COMPASS) {
            // N E S W at the edges of the map (MinimapHud.java:294-309);
            // the map faces north.
            let at = [
                ("N", ox + mid, 1.0, NORTH_COLOR),
                ("E", ox + side - 5.0, mid - 4.0, TEXT_COLOR),
                ("S", ox + mid, side - 9.0, TEXT_COLOR),
                ("W", ox + 5.0, mid - 4.0, TEXT_COLOR),
            ];
            for (letter, x, y, color) in at {
                shapes.push(Shape::Fill(FillShape {
                    rect: Rect {
                        x: x - 5.0,
                        y: y - 1.0,
                        w: 10.0,
                        h: 9.0,
                    },
                    color: LETTER_BOX,
                }));
                shapes.push(Shape::Text(TextShape {
                    text: letter.into(),
                    at: Point { x, y },
                    color,
                    centered: true,
                    shadow: false,
                }));
            }
        }
        // Info lines under the map, 10 px each, centred on it and kept in the box.
        let mut height = side;
        for ((text, color), w) in self.info.iter().zip(&widths) {
            let x = (ox + mid - w / 2.0).clamp(0.0, (wide - w).max(0.0));
            shapes.push(Shape::Text(TextShape {
                text: text.clone(),
                at: Point { x, y: height + 2.0 },
                color: *color,
                centered: false,
                shadow: true,
            }));
            height += layout::LINE;
        }
        if !self.info.is_empty() {
            height += 2.0;
        }
        let corner = match self.settings.corner() {
            MapCorner::TopRight => Corner::TopRight,
            MapCorner::TopLeft => Corner::TopLeft,
            MapCorner::BottomLeft => Corner::BottomLeft,
            MapCorner::BottomRight => Corner::BottomRight,
        };
        canvas::set_corner(corner, wide, height, &shapes);
    }

    /// Starts a rebuild of the looks when the packs reloaded.
    fn check_version(&mut self) {
        let now = client_world::table_version();
        let building = self.rebuild.as_ref().map(|r| r.table.version);
        if now == self.painter.looks.version || building == Some(now) {
            return;
        }
        let table = table_of(client_world::block_table());
        horizon_mod_sdk::info(&format!(
            "diorama: resources reloaded (block table version {} -> {}): averaging {} textures again",
            self.painter.looks.version,
            table.version,
            table.used_sprites().len()
        ));
        self.rebuild = Some(Rebuild::new(table));
    }

    /// The rebuilt looks take over: every cached chunk is read again (its
    /// plants and glass are seen through by the new looks) and repainted.
    fn finish_rebuild(&mut self) {
        let Some(r) = self.rebuild.take() else { return };
        let missing = r.missing;
        let looks = r.builder.finish(&r.table);
        horizon_mod_sdk::info(&format!(
            "diorama: block looks version {} ready ({} states, {} textures missing)",
            looks.version,
            looks.looks.len(),
            missing
        ));
        self.painter.looks = looks;
        self.blocks = r.table.blocks;
        self.state_block = r.table.state_block;
        self.pending.extend(self.mosaic.keys());
    }

    /// `/diorama colour <block>`: the block's first state's averaged face
    /// colours as the map has them; `/diorama` alone: the looks' version.
    fn describe(&self, args: &str) -> Result<String, String> {
        let mut words = args.split_whitespace();
        let looks = &self.painter.looks;
        match words.next() {
            None | Some("version") => Ok(format!(
                "Diorama looks version {}, {} states",
                looks.version,
                looks.looks.len()
            )),
            Some("colour" | "color") => {
                let name = words.next().ok_or("usage: /diorama colour <block>")?;
                let name = if name.contains(':') {
                    name.to_owned()
                } else {
                    format!("minecraft:{name}")
                };
                let b = self
                    .blocks
                    .iter()
                    .position(|n| *n == name)
                    .ok_or_else(|| format!("no block {name}"))?;
                let state = self
                    .state_block
                    .iter()
                    .position(|&s| usize::from(s) == b)
                    .ok_or_else(|| format!("{name} has no state"))?;
                let l = looks.get(state as u32);
                let face = |f: looks::FaceLook| match f.tint_weight {
                    w if w <= 0.0 => hex(f.base),
                    w if w >= 1.0 => format!("{} tinted", hex(f.tinted)),
                    w => format!("{} + {} tinted {w:.2}", hex(f.base), hex(f.tinted)),
                };
                Ok(format!(
                    "{name} (state {state}): {:?}, top {}, side {}, tint {:?}",
                    l.kind,
                    face(l.top),
                    face(l.side),
                    l.tint
                ))
            }
            Some("layer") => Ok(match self.cut {
                Some(y) => format!("cave slice at y {y}"),
                None => "surface".into(),
            }),
            Some("zoom") => Ok(format!("zoom {} px per block", self.settings.zoom())),
            Some(other) => Err(format!(
                "unknown /diorama {other}: try colour <block>, layer or zoom [in|out]"
            )),
        }
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
        let table = table_of(client_world::block_table());
        let (looks, table, sprites) = Rebuild::new(table).run();
        let water = table.sprite(looks::WATER_SPRITE);
        horizon_mod_sdk::info(&format!(
            "diorama: {SIDE}x{SIDE} mosaic, frame budget {granted} us; block looks version {}: \
             {} states from {sprites} textures averaged{}",
            looks.version,
            looks.looks.len(),
            if water.is_some() {
                ""
            } else {
                " (no water texture)"
            },
        ));
        let style = style::DEFAULT;
        let settings = Settings::load(|k| {
            storage::get(k)
                .and_then(|v| String::from_utf8(v).ok())
                .map(|v| v.trim().to_owned())
        });
        // Diorama's keys: zoom in `=`, zoom out `-`; toggling the map is
        // unbound until the player picks a key (DioramaClient.java:36-44).
        for (name, label, key) in [
            ("toggle", "Toggle minimap", "key.keyboard.unknown"),
            ("zoom_in", "Zoom minimap in", "key.keyboard.equal"),
            ("zoom_out", "Zoom minimap out", "key.keyboard.minus"),
        ] {
            if let Err(e) = input::register_key(name, label, key) {
                horizon_mod_sdk::warn(&format!("register-key {name}: {e}"));
            }
        }
        let mut style = style;
        if settings.flat_style() {
            style = style::Style {
                pitch_degrees: style::DEFAULT.pitch_degrees,
                ..style::FLAT
            };
        }
        let state = Diorama {
            mosaic: Mosaic::new(Projection::new(style.pitch_degrees), SIDE),
            painter: Painter::new(looks, style),
            blocks: table.blocks,
            state_block: table.state_block,
            rebuild: None,
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
            settings,
            rev: 0,
            info: Vec::new(),
            ceiling: false,
            underground: false,
            cut: None,
            layer_tick: 0,
        };
        state.page();
        Ok((
            state,
            Registration::new().on_frame().on_tick().command("diorama"),
        ))
    }

    fn command(&mut self, _name: &str, args: &str) -> Result<Option<String>, String> {
        match args.split_whitespace().collect::<Vec<_>>()[..] {
            ["zoom", "in"] => self.key("zoom_in"),
            ["zoom", "out"] => self.key("zoom_out"),
            _ => {}
        }
        self.describe(args).map(Some)
    }

    fn ui_select(&mut self, id: u32, _row: u32) {
        let i = (id as usize).wrapping_sub(1);
        if i < settings::SETTINGS.len() {
            self.settings.cycle(i);
            self.changed(i);
        }
    }

    fn ui_settings_opened(&mut self) {
        self.page();
    }

    fn tick(&mut self, tick: u64) {
        for event in input::poll() {
            if let input::Event::KeyDown(name) = event {
                self.key(&name);
            }
        }
        self.check_version();
        let Some(p) = game::get_player() else {
            if self.dimension.take().is_some() {
                self.reset();
            }
            return;
        };
        if self.dimension.as_deref() != Some(p.dimension.as_str()) {
            self.reset();
            self.dimension = Some(p.dimension.clone());
            self.ceiling = client_world::dimension().is_some_and(|d| d.ceiling);
        }
        if !self.settings.flag(settings::MINIMAP) {
            return;
        }
        if tick >= self.layer_tick {
            self.layer_tick = tick + LAYER_TICKS;
            self.update_layer(&p);
        }
        self.update_info(&p);
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
        if !self.settings.flag(settings::MINIMAP) {
            return;
        }
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
            if let Some(r) = &mut self.rebuild {
                if r.step() {
                    self.finish_rebuild();
                }
            } else if !self.pending.is_empty() && (batches < 2 || !self.mosaic.painting()) {
                chunks += self.fetch(centre);
                batches += 1;
                self.cost.fetch += left.saturating_sub(canvas::work_left());
            } else if self.mosaic.paint_step(&self.painter, u, MAX_ROWS) {
                self.cost.paint += left.saturating_sub(canvas::work_left());
            } else {
                break;
            }
            busy = true;
        }
        let before_upload = canvas::work_left();
        let image = self.image;
        let mut failed = None;
        let outline = self.painter.style.outline;
        // Outlines are worked out as texels go up, so the upload is paid for
        // in pieces while the budget lasts.
        let mut uploaded = 0;
        while uploaded < UPLOAD_BYTES && canvas::work_left() > UPLOAD_RESERVE {
            let piece = UPLOAD_PIECE.min(UPLOAD_BYTES - uploaded);
            let n = self.mosaic.take_upload(piece, outline, |x, y, w, h, rgba| {
                if let Err(e) = canvas::update_image(image, x, y, w, h, rgba) {
                    failed = Some(e);
                }
            });
            if n == 0 {
                break;
            }
            uploaded += n;
        }
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
