//! A session-only map camera and a second instance of the minimap baker.
use crate::*;
use diorama_core::fullmap::MapView;
use horizon_mod_sdk::PointerKind;

// Largest square of whole 16-texel strips that fits beside the 512² HUD.
// Leave room under the host's one-million-texel image cap for the HUD
// and a screen-sized saved exploration canvas.
const MAP_SIDE: u32 = 720;
const LEGEND: &str =
    "Drag pan - Wheel zoom - C centre - Right-click add waypoint - W waypoints - Esc close";
const LEGEND_SPLIT: [&str; 2] = [
    "Drag pan - Wheel zoom - C centre",
    "Right-click add waypoint - W waypoints - Esc close",
];
const LEGEND_SHORT: &str = "Drag - Wheel - C - RMB add - W list - Esc";

/// Leave six GUI pixels of padding at each edge, using the loaded font.
fn legend_lines(gui_width: f32) -> &'static [&'static str] {
    let available = (gui_width - 12.0).max(0.0);
    if render::text_width(LEGEND) <= available {
        &[LEGEND]
    } else if LEGEND_SPLIT
        .iter()
        .all(|line| render::text_width(line) <= available)
    {
        &LEGEND_SPLIT
    } else {
        &[LEGEND_SHORT]
    }
}

pub struct FullMap {
    mosaic: Mosaic,
    image: canvas::Image,
    pending: BTreeSet<(i32, i32)>,
    ready: Option<Ready>,
    since: u64,
    cut: Option<i32>,
    dimension: String,
    y_range: (i32, i32),
    view: MapView,
    cursor: (f32, f32),
    drag: Option<(f32, f32)>,
    selected: Option<String>,
    drawn: Option<Drawn>,
    fill_frames: u32,
    filled: bool,
    saved_image: Option<canvas::Image>,
    saved_view: Option<(MapView, (f32, f32))>,
    saved_size: (u32, u32),
    saved_rgba: Vec<u8>,
    /// Saved tiles still to find in the view; none when the scan is done.
    saved_queue: Option<crate::explored::Scan>,
    saved_job: Option<SavedPaint>,
    saved_waiting: Option<crate::explored::TileId>,
    saved_revision: u64,
}

enum SavedStep {
    Waiting,
    /// No job and the scan is over (or no saved layer).
    Done,
    /// Looked for the next tile, or a job had no rows left: try again.
    Scanning,
    /// Painted saved-layer rows `a..b`.
    Painted(u32, u32),
}

struct SavedPaint {
    id: ([u8; 16], i32, i32, u32),
    rgba: Vec<u8>,
    side: u32,
    row: u32,
    end: u32,
    x0: u32,
    x1: u32,
}

#[derive(PartialEq)]
struct Drawn {
    view: MapView,
    gui: (f32, f32),
    block: Option<(i32, i32, i32)>,
    player: (f64, f64, f64, f32),
    rev: u32,
    selected: Option<String>,
    cursor: (f32, f32),
}

impl FullMap {
    pub(super) fn invalidate(&mut self) {
        self.mosaic.forget_shades();
        self.pending.extend(self.mosaic.keys());
        self.ready = None;
    }
    fn touch_controls(&self) -> Vec<(layout::TouchButton, layout::Rect)> {
        layout::touch_buttons(
            render::gui_size(),
            &[
                layout::TouchButton::Centre,
                layout::TouchButton::Waypoints,
                layout::TouchButton::AddWaypoint,
                layout::TouchButton::ZoomIn,
                layout::TouchButton::ZoomOut,
            ],
        )
    }
    fn touch_actions(&self) -> Vec<(layout::TouchButton, layout::Rect)> {
        let mut buttons = layout::touch_buttons(
            render::gui_size(),
            &[
                layout::TouchButton::Teleport,
                layout::TouchButton::Rename,
                layout::TouchButton::Delete,
            ],
        );
        for (_, r) in &mut buttons {
            r.y = 34.0;
        }
        buttons
    }
    fn block(&self, bottom_bar: f32) -> Option<(i32, i32, i32)> {
        let gui = render::gui_size();
        if self.cursor.1 < 20.0 || self.cursor.1 >= gui.1 - bottom_bar {
            return None;
        }
        let uv = self.view.screen_to_map(self.cursor, gui);
        let (x, y) = self.mosaic.texel_of(uv.0, uv.1);
        if x < 0.0 || y < 0.0 || x >= MAP_SIDE as f64 || y >= MAP_SIDE as f64 {
            return None;
        }
        self.view
            .block_under(self.cursor, gui, self.y_range, |x, z| {
                self.mosaic.surface_y(x, z)
            })
    }
    fn pins(&self, d: &Diorama, bottom_bar: f32) -> Vec<(usize, (f64, f64))> {
        let proj = self.mosaic.projection();
        let gui = render::gui_size();
        d.wp.set
            .list
            .iter()
            .enumerate()
            .filter(|(_, w)| w.visible && w.dimension == self.dimension)
            .filter_map(|(i, w)| {
                let uv = (
                    proj.u(w.x as f64 + 0.5),
                    proj.v(w.y as f64, w.z as f64 + 0.5),
                );
                let (x, y) = self.view.map_to_screen(uv, gui);
                (x >= 0.0 && x <= gui.0 && y >= 24.0 && y < gui.1 - bottom_bar - 4.0)
                    .then_some((i, uv))
            })
            .take(512)
            .collect()
    }
}

impl Diorama {
    pub fn open_fullmap(&mut self) {
        if self.fullmap.is_some() {
            return;
        }
        let Some(p) = game::get_player() else { return };
        self.finish_move();
        self.list_closed();
        ui::close_screen();
        // Re-evaluate even with a hidden minimap, then freeze this slice.
        self.ceiling = client_world::dimension().is_some_and(|d| d.ceiling);
        self.update_layer(&p);
        let image = match canvas::create_image(MAP_SIDE, MAP_SIDE) {
            Ok(i) => i,
            Err(e) => {
                chat::show(&format!("Cannot open map: {e}"));
                return;
            }
        };
        let projection = *self.mosaic.projection();
        let uv = (projection.u(p.x), projection.v(p.y, p.z));
        let mut mosaic = Mosaic::with_texels(projection, MAP_SIDE, 1);
        mosaic.outline_depth = self.mosaic.outline_depth;
        mosaic.recentre(uv.0, uv.1);
        mosaic.import_chunks(self.mosaic.export_chunks(|k| {
            !self.pending.contains(&k) && self.ready.as_ref().is_none_or(|r| r.key != k)
        }));
        self.shared_updates.clear();
        let y_range = client_world::dimension().map_or((-64, 319), |d| {
            (
                d.min_y,
                d.min_y.saturating_add(d.height as i32).saturating_sub(1),
            )
        });
        self.fullmap = Some(FullMap {
            mosaic,
            image,
            pending: self.pending.clone(),
            ready: None,
            since: self.since,
            cut: self.cut,
            dimension: p.dimension.clone(),
            y_range,
            view: MapView::new(uv.0, uv.1, 1.0),
            cursor: (render::gui_size().0 / 2.0, render::gui_size().1 / 2.0),
            drag: None,
            selected: None,
            drawn: None,
            fill_frames: 0,
            filled: false,
            saved_image: None,
            saved_view: None,
            saved_size: (0, 0),
            saved_rgba: Vec::new(),
            saved_queue: None,
            saved_job: None,
            saved_waiting: None,
            saved_revision: 0,
        });
        if let Some(r) = &self.ready {
            self.fullmap
                .as_mut()
                .expect("just opened")
                .pending
                .insert(r.key);
        }
        self.fullmap_tick();
        canvas::open_canvas_screen("Diorama map", &[]);
        self.update_budget();
        self.render_fullmap(&p, true);
    }

    pub fn close_fullmap(&mut self) {
        let Some(f) = self.fullmap.take() else { return };
        canvas::close_canvas_screen();
        canvas::drop_image(f.image);
        if let Some(image) = f.saved_image {
            canvas::drop_image(image);
        }
        self.saved.flush();
        let centre = game::get_player()
            .map(|p| ((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4))
            .or(self.player_chunk);
        if let Some(pc) = centre {
            self.pending
                .retain(|k| (k.0 - pc.0).abs().max((k.1 - pc.1).abs()) <= FORGET);
            let snapshot = f.mosaic.into_chunks(|k| {
                self.shared_updates.contains(&k)
                    && !f.pending.contains(&k)
                    && f.ready.as_ref().is_none_or(|r| r.key != k)
                    && (k.0 - pc.0).abs().max((k.1 - pc.1).abs()) <= FORGET
            });
            for key in self.mosaic.import_chunks(snapshot) {
                self.pending.remove(&key);
                if self.ready.as_ref().is_some_and(|r| r.key == key) {
                    self.ready = None;
                }
            }
            self.mosaic.forget_far(pc, FORGET);
        }
        self.shared_updates.clear();
        self.shown = None;
        self.fill = Some(Fill {
            started: self.clock,
            frames: 0,
            visible: false,
            cost: Cost::default(),
        });
        self.update_budget();
    }

    pub fn fullmap_tick(&mut self) {
        let Some(f) = &mut self.fullmap else { return };
        let (since, chunks) = client_world::changes(f.since, 64);
        f.since = since;
        let (u0, v0) = f.mosaic.anchor();
        let projection = f.mosaic.projection();
        let high = f.cut.unwrap_or(f.y_range.1) as f64 + 1.0;
        let low = f.y_range.0 as f64 - diorama_core::bake::MAX_FACE as f64;
        // The image stays at its opening location. Chunks whose faces cannot
        // reach it waste bake time and could grow the cache after a teleport.
        f.pending.extend(chunks.into_iter().filter(|&(x, z)| {
            (x + 1) * 16 > u0
                && x * 16 < u0 + MAP_SIDE as i32
                && projection.v(high, (z * 16) as f64) < (v0 + MAP_SIDE as i32) as f64
                && projection.v(low, ((z + 1) * 16) as f64) > v0 as f64
        }));
    }

    pub fn fullmap_frame(&mut self, p: &game::Player) {
        self.saved_fullmap_step(p);
        let Some(mut f) = self.fullmap.take() else {
            return;
        };
        if p.dimension != f.dimension {
            self.fullmap = Some(f);
            self.close_fullmap();
            return;
        }
        // Install the second bake instance for exactly one frame, using the same
        // work-left scheduler, learnt rates, painter and upload limits as the HUD.
        std::mem::swap(&mut self.mosaic, &mut f.mosaic);
        std::mem::swap(&mut self.image, &mut f.image);
        std::mem::swap(&mut self.pending, &mut f.pending);
        std::mem::swap(&mut self.ready, &mut f.ready);
        std::mem::swap(&mut self.cut, &mut f.cut);
        let old_cave = std::mem::replace(&mut self.painter.cave, self.cut);
        let focus = self.focus_chunks.take();
        let fill = self.fill.take();
        let watch = self.watch.take();
        let centre = ((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4);
        let version = self.painter.looks.version;
        let (busy, chunks, uploaded) = self.bake_frame(centre, f.view.centre_u, false, true, p);
        f.fill_frames += 1;
        if !f.filled
            && f.saved_queue.is_none()
            && f.saved_job.is_none()
            && self.pending.is_empty()
            && self.ready.is_none()
            && self.rebuild.is_none()
        {
            let a = self.mosaic.anchor();
            if let Some((src, _)) = f.view.src_dst(
                (a.0 as f64, a.1 as f64),
                self.mosaic.texels_per_block() as f32,
                (MAP_SIDE, MAP_SIDE),
                render::gui_size(),
            ) {
                if self.mosaic.window_done(
                    src.x as u32,
                    src.y as u32,
                    (src.x + src.w).ceil() as u32,
                    (src.y + src.h).ceil() as u32,
                ) {
                    f.filled = true;
                    horizon_mod_sdk::info(&format!(
                        "fullmap fill: visible, {} frames",
                        f.fill_frames
                    ));
                }
            }
        }

        self.fill = fill;
        self.watch = watch;
        self.focus_chunks = focus;
        self.painter.cave = old_cave;
        std::mem::swap(&mut self.cut, &mut f.cut);
        std::mem::swap(&mut self.ready, &mut f.ready);
        std::mem::swap(&mut self.pending, &mut f.pending);
        std::mem::swap(&mut self.image, &mut f.image);
        std::mem::swap(&mut self.mosaic, &mut f.mosaic);
        if version != self.painter.looks.version {
            self.mosaic.forget_shades();
            self.pending.extend(self.mosaic.keys());
            self.ready = None;
        }
        self.fullmap = Some(f);
        let before = canvas::work_left();
        self.render_fullmap(p, uploaded > 0);
        self.tail_rate.after(
            before.saturating_sub(canvas::work_left()),
            1,
            canvas::work_left(),
        );
        self.account(busy || uploaded > 0, chunks, uploaded);
    }

    /// Composite saved tiles into one canvas covering the whole camera view.
    /// Resample rows under the selected performance limits, then upload one
    /// whole-width slice. The selected reserve leaves work for the live bake;
    /// its upload also shares the hook's byte allowance with this saved layer.
    fn saved_fullmap_step(&mut self, p: &game::Player) {
        let mut rows: Option<(u32, u32)> = None;
        // Bounded: each pass paints rows or scans 64 grid cells.
        let limits = budget::performance(self.settings.stored(settings::PERFORMANCE));
        for _ in 0..limits.saved_steps {
            if canvas::work_left() < limits.saved_keep {
                break;
            }
            match self.saved_fullmap_job(p) {
                SavedStep::Done | SavedStep::Waiting => break,
                SavedStep::Scanning => {}
                SavedStep::Painted(a, b) => {
                    rows = Some(rows.map_or((a, b), |(x, y)| (x.min(a), y.max(b))));
                }
            }
        }
        let Some((a, b)) = rows else { return };
        let Some(f) = &mut self.fullmap else { return };
        let Some(image) = f.saved_image else { return };
        let row_bytes = f.saved_size.0 as usize * 4;
        self.saved_upload_bytes += (f.saved_size.0 * (b - a) * 4) as usize;
        let _ = canvas::update_image(
            image,
            0,
            a,
            f.saved_size.0,
            b - a,
            &f.saved_rgba[a as usize * row_bytes..b as usize * row_bytes],
        );
        f.drawn = None;
    }

    /// One piece of saved-tile work: 8 rows of the current tile, or a step
    /// of the scan for the next one.
    fn saved_fullmap_job(&mut self, p: &game::Player) -> SavedStep {
        let Some(f) = &mut self.fullmap else {
            return SavedStep::Done;
        };
        let gui = render::gui_size();
        let limits = budget::performance(self.settings.stored(settings::PERFORMANCE));
        if f.saved_view != Some((f.view, gui)) {
            let size = (
                gui.0.clamp(1.0, limits.saved_side as f32) as u32,
                gui.1.clamp(1.0, limits.saved_side as f32) as u32,
            );
            let old = f.saved_view;
            let pixels = old.map_or_else(
                || vec![0; (size.0 * size.1 * 4) as usize],
                |(view, gui)| {
                    diorama_core::fullmap::retained_layer(
                        (view, gui, f.saved_size, &f.saved_rgba),
                        f.view,
                        render::gui_size(),
                        size,
                    )
                },
            );
            if size != f.saved_size {
                if let Some(image) = f.saved_image.take() {
                    canvas::drop_image(image);
                }
                f.saved_image = canvas::create_image(size.0, size.1).ok();
            }
            f.saved_size = size;
            f.saved_rgba = pixels;
            if let Some(image) = f.saved_image {
                self.saved_upload_bytes += f.saved_rgba.len();
                let _ = canvas::update_image(image, 0, 0, size.0, size.1, &f.saved_rgba);
            }
            if let Some((view, _)) = old {
                self.saved.prefetch(
                    p,
                    f.cut,
                    self.settings.flat_style(),
                    &f.view,
                    gui,
                    (
                        f.view.centre_u - view.centre_u,
                        f.view.centre_v - view.centre_v,
                    ),
                    limits.prefetch_tiles,
                );
            }
            f.saved_view = Some((f.view, gui));
            f.saved_queue =
                Some(
                    self.saved
                        .visible(p, f.cut, self.settings.flat_style(), &f.view, gui),
                );
            f.saved_job = None;
            f.saved_waiting = None;
            f.saved_revision = self.saved.revision;
            f.drawn = None;
        }
        if f.saved_queue.is_none()
            && f.saved_job.is_none()
            && f.saved_waiting.is_none()
            && f.saved_revision != self.saved.revision
        {
            f.saved_revision = self.saved.revision;
            f.saved_queue =
                Some(
                    self.saved
                        .visible(p, f.cut, self.settings.flat_style(), &f.view, gui),
                );
        }
        if f.saved_image.is_none() {
            return SavedStep::Done;
        }
        let side = diorama_core::explored::SIDE;
        if let Some(scan) = &f.saved_queue {
            self.saved.warm_scan(scan, limits.tile_loads);
        }
        if f.saved_job.is_none() {
            let next = f.saved_waiting.take().or_else(|| match &mut f.saved_queue {
                Some(scan) => {
                    let (done, id) = scan.step(&self.saved);
                    if done {
                        f.saved_queue = None;
                    }
                    id
                }
                None => None,
            });
            if let Some(id) = next {
                let a = f.view.map_to_screen(
                    (
                        id.1 as f64 * side as f64 / id.3 as f64,
                        id.2 as f64 * side as f64 / id.3 as f64,
                    ),
                    gui,
                );
                let b = f.view.map_to_screen(
                    (
                        (id.1 + 1) as f64 * side as f64 / id.3 as f64,
                        (id.2 + 1) as f64 * side as f64 / id.3 as f64,
                    ),
                    gui,
                );
                let x = |n: f32| {
                    (n / gui.0 * f.saved_size.0 as f32)
                        .ceil()
                        .clamp(0.0, f.saved_size.0 as f32) as u32
                };
                let y = |n: f32| {
                    (n / gui.1 * f.saved_size.1 as f32)
                        .ceil()
                        .clamp(0.0, f.saved_size.1 as f32) as u32
                };
                if let Some((side, rgba)) = self.saved.tile(id) {
                    f.saved_job = Some(SavedPaint {
                        id,
                        rgba: rgba.to_vec(),
                        side,
                        row: y(a.1),
                        end: y(b.1),
                        x0: x(a.0),
                        x1: x(b.0),
                    });
                } else if self.saved.waiting(id) {
                    f.saved_waiting = Some(id);
                    return SavedStep::Waiting;
                }
            }
        }
        let Some(mut job) = f.saved_job.take() else {
            return if f.saved_queue.is_some() {
                SavedStep::Scanning
            } else {
                SavedStep::Done
            };
        };
        let end = (job.row + limits.saved_rows).min(job.end);
        let row_bytes = f.saved_size.0 as usize * 4;
        for j in job.row..end {
            for i in job.x0..job.x1 {
                let uv = f.view.screen_to_map(
                    (
                        i as f32 / f.saved_size.0 as f32 * gui.0,
                        j as f32 / f.saved_size.1 as f32 * gui.1,
                    ),
                    gui,
                );
                let x = (uv.0 * job.id.3 as f64).floor() as i32 - job.id.1 * side as i32;
                let y = (uv.1 * job.id.3 as f64).floor() as i32 - job.id.2 * side as i32;
                if !(0..side as i32).contains(&x) || !(0..side as i32).contains(&y) {
                    continue;
                }
                let src = (((y as u32 * job.side / side) * job.side + x as u32 * job.side / side)
                    * 4) as usize;
                let at = j as usize * row_bytes + i as usize * 4;
                if job.rgba[src + 3] != 0 {
                    f.saved_rgba[at..at + 4].copy_from_slice(&job.rgba[src..src + 4]);
                }
            }
        }
        let painted = (job.row, end);
        job.row = end;
        if end < job.end {
            f.saved_job = Some(job);
        }
        if painted.1 > painted.0 {
            SavedStep::Painted(painted.0, painted.1)
        } else {
            SavedStep::Scanning
        }
    }

    fn render_fullmap(&mut self, p: &game::Player, force: bool) {
        let Some(f) = &self.fullmap else { return };
        let gui = render::gui_size();
        let touch = self.touch;
        let legend = legend_lines(gui.0);
        let bottom_bar = if touch {
            28.0
        } else {
            10.0 + 10.0 * legend.len() as f32
        };
        if !force
            && f.drawn.as_ref().is_some_and(|d| {
                d.view == f.view
                    && d.gui == gui
                    && d.cursor == f.cursor
                    && d.player == (p.x, p.y, p.z, p.yaw)
                    && d.rev == self.rev
                    && d.selected == f.selected
            })
        {
            return;
        }
        let block = f.block(bottom_bar);
        let key = Drawn {
            view: f.view,
            gui,
            block,
            player: (p.x, p.y, p.z, p.yaw),
            rev: self.rev,
            selected: f.selected.clone(),
            cursor: f.cursor,
        };
        if !force && f.drawn.as_ref() == Some(&key) {
            return;
        }
        let fill = |x, y, w, h, color| {
            Shape::Fill(FillShape {
                rect: Rect { x, y, w, h },
                color,
            })
        };
        let text = |s: String, x, y, centered, color| {
            Shape::Text(TextShape {
                text: s,
                at: Point { x, y },
                color,
                centered,
                shadow: true,
            })
        };
        let mut out = vec![fill(0.0, 0.0, gui.0, gui.1, 0xFF28_2828)];
        if let (Some(image), Some((view, old_gui))) = (f.saved_image, f.saved_view) {
            // Pointer events can redraw before the next budgeted resample.
            // Keep that last image attached to its map coordinates meanwhile.
            let a = f
                .view
                .map_to_screen(view.screen_to_map((0.0, 0.0), old_gui), gui);
            let b = f
                .view
                .map_to_screen(view.screen_to_map(old_gui, old_gui), gui);
            out.push(Shape::Image(ImageShape {
                image,
                src: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: f.saved_size.0 as f32,
                    h: f.saved_size.1 as f32,
                },
                dst: Rect {
                    x: a.0,
                    y: a.1,
                    w: b.0 - a.0,
                    h: b.1 - a.1,
                },
                tint: 0xFFFF_FFFF,
                round: false,
            }));
        }
        let a = f.mosaic.anchor();
        if let Some((src, dst)) = f.view.src_dst(
            (a.0 as f64, a.1 as f64),
            f.mosaic.texels_per_block() as f32,
            (MAP_SIDE, MAP_SIDE),
            gui,
        ) {
            let rect = |r: layout::Rect| Rect {
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
            };
            out.push(Shape::Image(ImageShape {
                image: f.image,
                src: rect(src),
                dst: rect(dst),
                tint: 0xFFFF_FFFF,
                round: false,
            }));
        }
        let proj = f.mosaic.projection();
        let (px, py) = f.view.map_to_screen((proj.u(p.x), proj.v(p.y, p.z)), gui);
        if px >= 0.0 && px <= gui.0 && py >= 20.0 && py < gui.1 - bottom_bar {
            for q in layout::arrow(px, py, 10.0, p.yaw) {
                let pt = |(x, y)| Point { x, y };
                out.push(Shape::Quad(QuadShape {
                    corners: (pt(q[0]), pt(q[1]), pt(q[2]), pt(q[3])),
                    color: ARROW_COLOR,
                }));
            }
        }
        for (i, uv) in f.pins(self, bottom_bar) {
            let w = &self.wp.set.list[i];
            let (x, y) = f.view.map_to_screen(uv, gui);
            let selected = f.selected.as_deref() == Some(w.id.as_str());
            out.push(fill(
                x - 3.0,
                y - 3.0,
                6.0,
                6.0,
                if selected {
                    0xFFFF_FFFF
                } else {
                    w.color | 0xFF00_0000
                },
            ));
            out.push(text(
                w.name.clone(),
                x,
                y - 12.0,
                true,
                w.color | 0xFF00_0000,
            ));
        }
        out.push(fill(0.0, 0.0, gui.0, 20.0, 0xEF10_1418));
        out.push(fill(
            0.0,
            gui.1 - bottom_bar,
            gui.0,
            bottom_bar,
            0xEF10_1418,
        ));
        let line = block.map_or_else(
            || "Unknown area".into(),
            |(x, y, z)| {
                let biome = client_world::biome(x, y, z)
                    .map(|b| layout::biome_name(&b))
                    .unwrap_or_else(|| "Unknown biome".into());
                format!("{x} {y} {z} {biome}")
            },
        );
        out.push(text(line, 6.0, 5.0, false, 0xFFFF_FFFF));
        // Keep north inside the map, below both the top bar and any pin actions.
        let north_y = if f.selected.is_some() { 36.0 } else { 24.0 };
        let north_w = render::text_width("N") + 8.0;
        out.push(fill(
            gui.0 - 6.0 - north_w,
            north_y - 2.0,
            north_w,
            12.0,
            0xEF10_1418,
        ));
        out.push(text(
            "N".into(),
            gui.0 - 6.0 - north_w / 2.0,
            north_y,
            true,
            0xFFFF_6B6B,
        ));
        if touch {
            let controls = f.touch_controls();
            for (button, r) in &controls {
                draw_touch_button(&mut out, *button, *r);
            }
            // Visible target for Add waypoint: this crosshair marks the map-view centre.
            let (cx, cy) = (gui.0 / 2.0, gui.1 / 2.0);
            out.push(fill(cx - 5.0, cy, 11.0, 1.0, 0xDDFF_FFFF));
            out.push(fill(cx, cy - 5.0, 1.0, 11.0, 0xDDFF_FFFF));
        } else {
            for (i, line) in legend.iter().enumerate() {
                out.push(text(
                    (*line).into(),
                    gui.0 / 2.0,
                    gui.1 - bottom_bar + 8.0 + i as f32 * 10.0,
                    true,
                    0xFFFF_FFFF,
                ));
            }
        }
        if f.selected.is_some() {
            if touch {
                out.push(fill(0.0, 32.0, gui.0, 26.0, 0xEF10_1418));
                for (button, r) in f.touch_actions() {
                    draw_touch_button(&mut out, button, r);
                }
            } else {
                out.push(fill(0.0, 20.0, gui.0, 12.0, 0xEF10_1418));
                out.push(text(
                    "Teleport (T) / Delete (Del)".into(),
                    gui.0 / 2.0,
                    22.0,
                    true,
                    0xFFFF_FFFF,
                ));
            }
        }
        canvas::set_canvas_screen(&out);
        if let Some(f) = &mut self.fullmap {
            f.drawn = Some(key);
        }
    }

    pub fn map_pointer(&mut self, kind: PointerKind, x: f32, y: f32, button: u32) {
        let Some(mut f) = self.fullmap.take() else {
            return;
        };
        f.cursor = (x, y);
        let bottom_bar = 10.0 + 10.0 * legend_lines(render::gui_size().0).len() as f32;
        if kind == PointerKind::Down && button == 0 && self.touch {
            let gui = render::gui_size();
            if f.selected.is_some() {
                let action_buttons = f.touch_actions();
                if let Some(action) = layout::hit_button(&action_buttons, x, y) {
                    let selected = f.selected.as_ref();
                    let index = self
                        .wp
                        .set
                        .list
                        .iter()
                        .position(|w| Some(&w.id) == selected);
                    match (action, index) {
                        (layout::TouchButton::Teleport, Some(i)) => match self.teleport(i) {
                            Ok(t) | Err(t) => chat::show(&t),
                        },
                        (layout::TouchButton::Rename, Some(i)) => {
                            // The chat box (and the phone's keyboard) with the rename
                            // command typed in: the player adds the name and sends it.
                            self.fullmap = Some(f);
                            self.close_fullmap();
                            chat::open(&format!("/dwp rename {} ", i + 1));
                            return;
                        }
                        (layout::TouchButton::Delete, Some(i)) => {
                            self.removed(i);
                            self.wp_changed();
                            f.selected = None;
                        }
                        _ => {}
                    }
                    self.fullmap = Some(f);
                    if let Some(p) = game::get_player() {
                        self.render_fullmap(&p, false);
                    }
                    return;
                }
            }
            let controls = f.touch_controls();
            if let Some(action) = layout::hit_button(&controls, x, y) {
                match action {
                    layout::TouchButton::Centre => {
                        if let Some(p) = game::get_player() {
                            let proj = f.mosaic.projection();
                            f.view.centre_on(proj.u(p.x), proj.v(p.y, p.z));
                        }
                    }
                    layout::TouchButton::Waypoints => {
                        self.fullmap = Some(f);
                        self.show_list();
                        return;
                    }
                    layout::TouchButton::AddWaypoint => {
                        let centre = (gui.0 / 2.0, gui.1 / 2.0);
                        let at = f.view.block_under(centre, gui, f.y_range, |wx, wz| {
                            f.mosaic.surface_y(wx, wz)
                        });
                        if let Some(at) = at {
                            match self.add_at(None, at, &f.dimension) {
                                Ok(t) | Err(t) => chat::show(&t),
                            }
                        } else {
                            chat::show("No loaded surface at the map centre");
                        }
                    }
                    layout::TouchButton::ZoomIn | layout::TouchButton::ZoomOut => {
                        let centre = (gui.0 / 2.0, gui.1 / 2.0);
                        f.view.zoom_at(
                            if action == layout::TouchButton::ZoomIn {
                                1.0
                            } else {
                                -1.0
                            },
                            centre,
                            gui,
                        );
                    }
                    _ => {}
                }
                self.fullmap = Some(f);
                if let Some(p) = game::get_player() {
                    self.render_fullmap(&p, false);
                }
                return;
            }
        }
        let bottom_bar = if self.touch { 28.0 } else { bottom_bar };
        match kind {
            PointerKind::Down if button == 1 => {
                if let Some(at) = f.block(bottom_bar) {
                    match self.add_at(None, at, &f.dimension) {
                        Ok(t) | Err(t) => chat::show(&t),
                    }
                }
            }
            PointerKind::Down
                if button == 0 && y >= 20.0 && y < render::gui_size().1 - bottom_bar =>
            {
                let pins = f.pins(self, bottom_bar);
                let uv: Vec<_> = pins.iter().map(|p| p.1).collect();
                f.selected = f
                    .view
                    .pin_hit(&uv, (x, y), render::gui_size(), 8.0)
                    .map(|i| self.wp.set.list[pins[i].0].id.clone());
                f.drag = f.selected.is_none().then_some((x, y));
            }
            PointerKind::Drag if button == 0 => {
                if let Some((px, py)) = f.drag {
                    f.view.pan_by(x - px, y - py);
                    f.drag = Some((x, y));
                }
            }
            PointerKind::Up if button == 0 => f.drag = None,
            _ => {}
        }
        self.fullmap = Some(f);
        if let Some(p) = game::get_player() {
            self.render_fullmap(&p, false);
        }
    }

    pub fn map_scroll(&mut self, x: f32, y: f32, steps: f32) {
        if let Some(f) = &mut self.fullmap {
            f.cursor = (x, y);
            f.view.zoom_at(steps, (x, y), render::gui_size());
        }
        if let Some(p) = game::get_player() {
            self.render_fullmap(&p, false);
        }
    }

    pub fn map_key(&mut self, key: &str, down: bool) {
        if !down || self.fullmap.is_none() {
            return;
        }
        match key {
            "key.keyboard.w" => {
                self.show_list();
                return;
            }
            "key.keyboard.c" => {
                if let (Some(f), Some(p)) = (&mut self.fullmap, game::get_player()) {
                    let proj = f.mosaic.projection();
                    f.view.centre_on(proj.u(p.x), proj.v(p.y, p.z));
                }
            }
            "key.keyboard.t" | "key.keyboard.delete" => {
                let selected = self.fullmap.as_ref().and_then(|f| f.selected.as_ref());
                if let Some(i) = self
                    .wp
                    .set
                    .list
                    .iter()
                    .position(|w| Some(&w.id) == selected)
                {
                    if key == "key.keyboard.t" {
                        match self.teleport(i) {
                            Ok(t) | Err(t) => chat::show(&t),
                        }
                    } else {
                        self.removed(i);
                        self.wp_changed();
                        if let Some(f) = &mut self.fullmap {
                            f.selected = None;
                        }
                    }
                }
            }
            _ => return,
        }
        if let Some(p) = game::get_player() {
            self.render_fullmap(&p, false);
        }
    }
}

fn draw_touch_button(out: &mut Vec<Shape>, button: layout::TouchButton, r: layout::Rect) {
    let rect = |x, y, w, h| Rect { x, y, w, h };
    out.push(Shape::Fill(FillShape {
        rect: rect(r.x, r.y, r.w, r.h),
        color: 0xB000_0000,
    }));
    for edge in [
        rect(r.x, r.y, r.w, 1.0),
        rect(r.x, r.y + r.h - 1.0, r.w, 1.0),
        rect(r.x, r.y, 1.0, r.h),
        rect(r.x + r.w - 1.0, r.y, 1.0, r.h),
    ] {
        out.push(Shape::Fill(FillShape {
            rect: edge,
            color: 0xCCFF_FFFF,
        }));
    }
    let label = match button {
        layout::TouchButton::Centre => "Centre",
        layout::TouchButton::Waypoints => "Waypoints",
        layout::TouchButton::AddWaypoint => "Add waypoint",
        layout::TouchButton::ZoomIn => "+",
        layout::TouchButton::ZoomOut => "−",
        layout::TouchButton::Teleport => "Teleport",
        layout::TouchButton::Rename => "Rename",
        layout::TouchButton::Delete => "Delete",
        layout::TouchButton::Done => "Done",
        layout::TouchButton::Reset => "Reset",
    };
    out.push(Shape::Text(TextShape {
        text: label.into(),
        at: Point {
            x: r.x + r.w / 2.0,
            y: r.y + 6.0,
        },
        color: 0xFFFF_FFFF,
        centered: true,
        shadow: true,
    }));
}
