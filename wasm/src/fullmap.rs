//! A session-only map camera and a second instance of the minimap baker.
use crate::*;
use diorama_core::fullmap::MapView;
use horizon_mod_sdk::PointerKind;

// Largest square of whole 16-texel strips that fits beside the 512² HUD.
const MAP_SIDE: u32 = 880;
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
}

#[derive(PartialEq)]
struct Drawn {
    view: MapView,
    gui: (f32, f32),
    block: Option<(i32, i32, i32)>,
    player: (f64, f64, f64, f32),
    rev: u32,
    selected: Option<String>,
}

impl FullMap {
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
        let y_range = client_world::dimension().map_or((-64, 319), |d| {
            (
                d.min_y,
                d.min_y.saturating_add(d.height as i32).saturating_sub(1),
            )
        });
        self.fullmap = Some(FullMap {
            mosaic,
            image,
            pending: BTreeSet::new(),
            ready: None,
            since: 0,
            cut: self.cut,
            dimension: p.dimension.clone(),
            y_range,
            view: MapView::new(uv.0, uv.1, 1.0),
            cursor: (render::gui_size().0 / 2.0, render::gui_size().1 / 2.0),
            drag: None,
            selected: None,
            drawn: None,
        });
        self.fullmap_tick();
        canvas::open_canvas_screen("Diorama map", &[]);
        self.render_fullmap(&p, true);
    }

    pub fn close_fullmap(&mut self) {
        let Some(f) = self.fullmap.take() else { return };
        canvas::close_canvas_screen();
        canvas::drop_image(f.image);
        // Re-read HUD chunks in case they changed or packs reloaded while paused.
        self.mosaic.forget_shades();
        self.pending.extend(self.mosaic.keys());
        if let Some(r) = self.ready.take() {
            self.pending.insert(r.key);
        }
        self.since = 0;
        self.shown = None;
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
        let (busy, chunks, uploaded) = self.bake_frame(centre, f.view.centre_u, false, true);

        self.fill = fill;
        self.watch = watch;
        self.focus_chunks = focus;
        self.painter.cave = old_cave;
        std::mem::swap(&mut self.cut, &mut f.cut);
        std::mem::swap(&mut self.ready, &mut f.ready);
        std::mem::swap(&mut self.pending, &mut f.pending);
        std::mem::swap(&mut self.image, &mut f.image);
        std::mem::swap(&mut self.mosaic, &mut f.mosaic);
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

    fn render_fullmap(&mut self, p: &game::Player, force: bool) {
        let Some(f) = &self.fullmap else { return };
        let gui = render::gui_size();
        let legend = legend_lines(gui.0);
        let bottom_bar = 10.0 + 10.0 * legend.len() as f32;
        let block = f.block(bottom_bar);
        let key = Drawn {
            view: f.view,
            gui,
            block,
            player: (p.x, p.y, p.z, p.yaw),
            rev: self.rev,
            selected: f.selected.clone(),
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
        for (i, line) in legend.iter().enumerate() {
            out.push(text(
                (*line).into(),
                gui.0 / 2.0,
                gui.1 - bottom_bar + 8.0 + i as f32 * 10.0,
                true,
                0xFFFF_FFFF,
            ));
        }
        if f.selected.is_some() {
            out.push(fill(0.0, 20.0, gui.0, 12.0, 0xEF10_1418));
            out.push(text(
                "Teleport (T) / Delete (Del)".into(),
                gui.0 / 2.0,
                22.0,
                true,
                0xFFFF_FFFF,
            ));
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
