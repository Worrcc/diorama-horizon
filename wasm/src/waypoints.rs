//! Waypoints in the mod: per-world files in `storage`, the death rule, the
//! `/dwp` command, keys B and U, the list screen, and the pins and edge
//! badges in the map box. Everything decided here is in
//! `diorama_core::waypoints`; this file only talks to Horizon.
//!
//! The teleport button is the only place the mod runs a command
//! (capability `command`): [`Diorama::teleport`] runs the one line
//! `waypoints::teleport_line` builds, a vanilla `tp @s x y z` (through
//! `execute in <dimension>` for another dimension), with the player's own
//! permissions.

use diorama_core::waypoints::{
    self as wp, Dwp, Index, Placed, Rng, Set, View, Waypoint, INDEX_KEY, MAX_WORLDS,
};
use horizon_mod_sdk::{
    canvas::Shape,
    chat, command, game,
    render::{self, FillShape, Point, QuadShape, Rect, TextShape},
    storage,
    ui::{self, ButtonWidget, ListWidget, Screen, TextWidget, Widget},
};

use crate::Diorama;

/// Where an unreadable waypoint file is copied before it is replaced.
const BAD_KEY: &str = "wp-unreadable";
/// Pins drawn at most (the nearest), so the box stays well under its 1024 shapes.
const MAX_PINS: usize = 64;
/// Lines `/dwp list` prints.
const LIST_LINES: usize = 20;
const OUTLINE: u32 = 0xE010_1418;

/// The list screen's widget ids (the settings page uses 1..=16).
const LIST: u32 = 100;
const TELEPORT: u32 = 101;
const COLOUR: u32 = 102;
const HIDE: u32 = 103;
const DELETE: u32 = 104;

/// The waypoint side of the mod's state.
pub struct Waypoints {
    pub set: Set,
    /// The world the set belongs to, and its storage key.
    pub world: Option<String>,
    pub key: Option<String>,
    index: Index,
    pub rng: Rng,
    /// The player was alive last tick (the death rule).
    alive: bool,
    /// The list screen is up, its picked row, and a delete waiting for a
    /// second click.
    open: bool,
    selected: Option<usize>,
    confirm: bool,
    /// Warned once that the game names no world.
    no_id: bool,
}

impl Waypoints {
    pub fn new() -> Waypoints {
        let index = storage::get(INDEX_KEY)
            .and_then(|v| String::from_utf8(v).ok())
            .map(|t| Index::parse(&t))
            .unwrap_or_default();
        Waypoints {
            set: Set::default(),
            world: None,
            key: None,
            index,
            rng: Rng::new(0xD10_4A3A),
            alive: true,
            open: false,
            selected: None,
            confirm: false,
            no_id: false,
        }
    }
}

fn here() -> Option<(game::Player, (i32, i32, i32))> {
    let p = game::get_player()?;
    let at = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
    Some((p, at))
}

/// The world's time in milliseconds (ticks are 50 ms): the floor of a new
/// waypoint's `created`.
fn now_ms() -> i64 {
    game::get_clock().map_or(0, |c| (c.game_time as i64).saturating_mul(50))
}

impl Diorama {
    fn stir(&mut self) {
        let r = &mut self.wp.rng;
        r.mix(u64::from(self.frame_no));
        if let Some(c) = game::get_clock() {
            r.mix(c.game_time);
        }
        if let Some(p) = game::get_player() {
            r.mix(p.x.to_bits() ^ p.z.to_bits().rotate_left(21) ^ p.y.to_bits().rotate_left(42));
        }
    }

    /// Every tick: the world changed (swap the set), and the death rule.
    pub fn waypoints_tick(&mut self, p: Option<&game::Player>) {
        let world = p.and(game::world_id());
        if p.is_some() && world.is_none() && !self.wp.no_id {
            self.wp.no_id = true;
            horizon_mod_sdk::warn("diorama: game.world-id gave no world: waypoints wait for one");
        }
        if world != self.wp.world {
            self.open_world(world);
            self.wp.alive = p.is_none_or(|p| p.health > 0.0);
            return;
        }
        let Some(p) = p else { return };
        let alive = p.health > 0.0;
        // Diorama marks the death from the combat-kill packet
        // (DeathPacketMixin.java:17-23). A client WASM mod sees no packets;
        // the nearest it has is the health reaching 0 (the death screen).
        if self.wp.alive && !alive && self.settings.flag(diorama_core::settings::DEATH_MARKERS) {
            self.died(p);
        }
        self.wp.alive = alive;
    }

    fn died(&mut self, p: &game::Player) {
        self.stir();
        let at = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        let day = game::get_clock().map_or(0, |c| c.day_time);
        let w = &mut self.wp;
        w.set.add_death(at, &p.dimension, day, &mut w.rng, now_ms());
        self.wp_changed();
        chat::show(&format!("Death marker at {} {} {}", at.0, at.1, at.2));
    }

    /// Loads the waypoints of `world` (none: the title screen).
    fn open_world(&mut self, world: Option<String>) {
        self.wp.set = Set::default();
        self.wp.key = None;
        self.wp.selected = None;
        self.wp.world.clone_from(&world);
        self.rev = self.rev.wrapping_add(1);
        let Some(world) = world else { return };
        let (key, dropped) = self.wp.index.open(&world, MAX_WORLDS);
        for d in &dropped {
            storage::delete(&d.key);
            chat::show(&format!(
                "Waypoints of {} were removed: Diorama keeps those of the {MAX_WORLDS} worlds opened last",
                d.world
            ));
        }
        if let Err(e) = storage::set(INDEX_KEY, self.wp.index.format().as_bytes()) {
            horizon_mod_sdk::warn(&format!("diorama: saving the world index: {e}"));
        }
        let text = storage::get(&key).and_then(|v| String::from_utf8(v).ok());
        if let Some(text) = text {
            let rng = &mut self.wp.rng;
            match wp::parse_file(&text, &mut || rng.uuid()) {
                Ok((list, skipped)) => {
                    if skipped > 0 {
                        horizon_mod_sdk::warn(&format!(
                            "diorama: skipped {skipped} unreadable waypoints in {key}"
                        ));
                    }
                    self.wp.set.list = list;
                }
                Err(e) => {
                    // As Diorama keeps an unreadable file aside (WaypointStore.java:435).
                    horizon_mod_sdk::warn(&format!(
                        "diorama: {key} is unreadable ({e}); kept as {BAD_KEY}"
                    ));
                    let _ = storage::set(BAD_KEY, text.as_bytes());
                }
            }
        }
        horizon_mod_sdk::info(&format!(
            "diorama: world {world}: {} waypoints ({key})",
            self.wp.set.list.len()
        ));
        self.wp.key = Some(key);
    }

    /// Saves the set and redraws (the box and an open list).
    fn wp_changed(&mut self) {
        self.rev = self.rev.wrapping_add(1);
        if let Some(key) = &self.wp.key {
            if let Err(e) = storage::set(key, wp::write_file(&self.wp.set.list).as_bytes()) {
                horizon_mod_sdk::warn(&format!("diorama: saving waypoints: {e}"));
                chat::show(&format!("Waypoints not saved: {e}"));
            }
        }
        if self.wp.open {
            self.show_list();
        }
    }

    fn need_world(&self) -> Result<(), String> {
        if self.wp.key.is_some() {
            Ok(())
        } else {
            Err("not in a world".into())
        }
    }

    /// A waypoint where the player stands (key B, `/dwp add`).
    pub fn add_here(&mut self, name: Option<&str>) -> Result<String, String> {
        self.need_world()?;
        let (p, at) = here().ok_or("not in a world")?;
        self.stir();
        let w = &mut self.wp;
        let i = w.set.add(name, at, &p.dimension, &mut w.rng, now_ms())?;
        self.wp_changed();
        let w = &self.wp.set.list[i];
        let mut text = format!(
            "Added {}. {} at {} {} {} ({})",
            i + 1,
            w.name,
            at.0,
            at.1,
            at.2,
            wp::color_name(w.color)
        );
        if name.is_none() {
            text.push_str(&format!(": /dwp rename {} <name> names it", i + 1));
        }
        Ok(text)
    }

    /// The one command the mod runs (see the module's notes).
    fn teleport(&mut self, i: usize) -> Result<String, String> {
        let (p, _) = here().ok_or("not in a world")?;
        let w = &self.wp.set.list[i];
        let line = wp::teleport_line(w, &p.dimension)
            .ok_or_else(|| format!("{}'s dimension {:?} is not an id", w.name, w.dimension))?;
        command::run(&line);
        Ok(format!("Teleporting to {}", w.name))
    }

    fn removed(&mut self, i: usize) {
        self.wp.set.list.remove(i);
        self.wp.selected = match self.wp.selected {
            Some(s) if s == i => None,
            Some(s) if s > i => Some(s - 1),
            s => s,
        };
    }

    fn set_flag(&mut self, i: usize, on: Option<bool>) -> String {
        let s = diorama_core::settings::SETTINGS[i].key;
        if let Some(on) = on {
            self.settings
                .set_stored(i, if on { "true" } else { "false" });
            self.changed(i);
        }
        format!("{s}: {}", if self.settings.flag(i) { "on" } else { "off" })
    }

    /// `/dwp ...`.
    pub fn dwp(&mut self, args: &str) -> Result<Option<String>, String> {
        use diorama_core::settings::{DEATH_MARKERS, WAYPOINTS_ON_MAP, WAYPOINT_DISTANCE};
        let cmd = wp::parse_command(args)?;
        let find = |s: &Self, t: &str| -> Result<usize, String> {
            s.need_world()?;
            s.wp.set.find(t)
        };
        let text = match cmd {
            Dwp::Help => wp::DWP_HELP.to_owned(),
            Dwp::Add(name) => self.add_here(name.as_deref())?,
            Dwp::Del(t) => {
                let i = find(self, &t)?;
                let name = self.wp.set.list[i].name.clone();
                self.removed(i);
                self.wp_changed();
                format!("Deleted {}. {name}", i + 1)
            }
            Dwp::Rename(t, new) => {
                let i = find(self, &t)?;
                let new = wp::clean_name(&new);
                if new.is_empty() {
                    return Err("give the new name".into());
                }
                let w = &mut self.wp.set.list[i];
                let old = std::mem::replace(&mut w.name, new.clone());
                w.initials = wp::initials_for(&new);
                self.wp_changed();
                format!("Renamed {}. {old} to {new}", i + 1)
            }
            Dwp::Color(t, c) => {
                let i = find(self, &t)?;
                let now = self.wp.set.list[i].color;
                let color = match c.to_ascii_lowercase().as_str() {
                    "next" => wp::next_palette(now),
                    "random" => {
                        self.stir();
                        wp::random_color(&mut self.wp.rng, now)
                    }
                    other => wp::parse_color(other).ok_or_else(|| {
                        format!(
                            "{other:?} is not a colour: {}, #RRGGBB, next or random",
                            wp::PALETTE_NAMES.join(", ")
                        )
                    })?,
                };
                self.wp.set.list[i].color = color;
                self.wp_changed();
                format!(
                    "{}. {} is {}",
                    i + 1,
                    self.wp.set.list[i].name,
                    wp::color_name(color)
                )
            }
            Dwp::Hide(t) | Dwp::Show(t) => {
                let show = args.trim_start().to_ascii_lowercase().starts_with("show");
                let i = find(self, &t)?;
                self.wp.set.list[i].visible = show;
                self.wp_changed();
                format!(
                    "{}. {} {}",
                    i + 1,
                    self.wp.set.list[i].name,
                    if show { "shown" } else { "hidden" }
                )
            }
            Dwp::Tp(t) => {
                let i = find(self, &t)?;
                self.teleport(i)?
            }
            Dwp::List => {
                self.need_world()?;
                let p = game::get_player();
                let at = p.as_ref().map(|p| (p.dimension.as_str(), p.x, p.y, p.z));
                let n = self.wp.set.list.len();
                for (i, w) in self.wp.set.list.iter().enumerate().take(LIST_LINES) {
                    chat::show(&wp::row_text(i, w, at));
                }
                match n {
                    0 => "No waypoints in this world: /dwp add [name] or B".into(),
                    n if n > LIST_LINES => {
                        format!("{n} waypoints, the first {LIST_LINES} above (U lists all)")
                    }
                    1 => "1 waypoint".into(),
                    n => format!("{n} waypoints"),
                }
            }
            Dwp::Pins(on) => self.set_flag(WAYPOINTS_ON_MAP, on),
            Dwp::Deaths(on) => self.set_flag(DEATH_MARKERS, on),
            Dwp::Within(blocks) => {
                if let Some(b) = blocks {
                    if !self.settings.set_stored(WAYPOINT_DISTANCE, &b.to_string()) {
                        return Err(format!(
                            "within: one of {}",
                            wp::DISTANCES.map(|d| d.to_string()).join(", ")
                        ));
                    }
                    self.changed(WAYPOINT_DISTANCE);
                }
                match self.settings.waypoint_distance() {
                    0 => "Waypoints show at any distance".into(),
                    d => format!("Waypoints show within {d} blocks"),
                }
            }
        };
        Ok(Some(text))
    }

    /// The list screen (key U): every waypoint of the world, and buttons
    /// for the one picked. Naming is `/dwp rename` (no text field).
    pub fn show_list(&mut self) {
        self.wp.open = true;
        let p = game::get_player();
        let at = p.as_ref().map(|p| (p.dimension.as_str(), p.x, p.y, p.z));
        let rows: Vec<String> = (self.wp.set.list.iter().enumerate())
            .map(|(i, w)| wp::row_text(i, w, at))
            .collect();
        let mut widgets = Vec::new();
        if rows.is_empty() {
            widgets.push(Widget::Text(TextWidget {
                id: 0,
                text: "No waypoints yet: press B, or /dwp add <name>".into(),
                color: 0,
            }));
        } else {
            let selected = self.wp.selected.filter(|&s| s < rows.len());
            widgets.push(Widget::List(ListWidget {
                id: LIST,
                rows,
                selected: selected.map(|s| s as u32),
            }));
            match selected.map(|s| &self.wp.set.list[s]) {
                Some(w) => {
                    let button = |id, label: String| Widget::Button(ButtonWidget { id, label });
                    widgets.push(button(TELEPORT, format!("Teleport to {}", w.name)));
                    widgets.push(button(
                        COLOUR,
                        format!("Colour: {}", wp::color_name(w.color)),
                    ));
                    widgets.push(button(HIDE, if w.visible { "Hide" } else { "Show" }.into()));
                    widgets.push(button(
                        DELETE,
                        if self.wp.confirm {
                            "Delete: click again".into()
                        } else {
                            "Delete".into()
                        },
                    ));
                }
                None => widgets.push(Widget::Text(TextWidget {
                    id: 0,
                    text: "Pick one. /dwp rename names it".into(),
                    color: 0xA0_A0A0,
                })),
            }
        }
        ui::open_screen(&Screen {
            title: "Waypoints".into(),
            widgets,
        });
    }

    /// A pick on the list screen; false when it was not the list's.
    pub fn list_select(&mut self, id: u32, row: u32) -> bool {
        if !(LIST..=DELETE).contains(&id) || !self.wp.open {
            return false;
        }
        let confirm = std::mem::take(&mut self.wp.confirm);
        let Some(i) = (if id == LIST {
            Some(row as usize)
        } else {
            self.wp.selected
        })
        .filter(|&i| i < self.wp.set.list.len()) else {
            return true;
        };
        match id {
            LIST => self.wp.selected = Some(i),
            TELEPORT => {
                match self.teleport(i) {
                    Ok(_) => {}
                    Err(e) => chat::show(&e),
                }
                self.wp.open = false;
                ui::close_screen();
                return true;
            }
            COLOUR => {
                let w = &mut self.wp.set.list[i];
                w.color = wp::next_palette(w.color);
                self.wp_changed();
            }
            HIDE => {
                let w = &mut self.wp.set.list[i];
                w.visible = !w.visible;
                self.wp_changed();
            }
            DELETE if confirm => {
                self.removed(i);
                self.wp_changed();
            }
            DELETE => self.wp.confirm = true,
            _ => {}
        }
        self.show_list();
        true
    }

    pub fn list_closed(&mut self) {
        self.wp.open = false;
        self.wp.confirm = false;
    }

    /// Pins and edge badges for the map at `view`, the image drawn from
    /// source window `src` (texels) into it.
    pub fn pin_shapes(
        &self,
        p: &game::Player,
        view: View,
        src: (f32, f32, f32),
        out: &mut Vec<Shape>,
    ) {
        use diorama_core::settings::WAYPOINTS_ON_MAP;
        if !self.settings.flag(WAYPOINTS_ON_MAP) {
            return;
        }
        let within = self.settings.waypoint_distance();
        let at = (p.x, p.y, p.z);
        let mut shown: Vec<(f64, &Waypoint)> = (self.wp.set.list.iter())
            .filter(|w| wp::on_map(w, &p.dimension, within, at))
            .map(|w| (w.distance_to(p.x, p.y, p.z), w))
            .collect();
        shown.sort_by(|a, b| a.0.total_cmp(&b.0));
        shown.truncate(MAX_PINS);
        let proj = *self.mosaic.projection();
        // The farthest first, so the nearest are drawn on top.
        for (_, w) in shown.iter().rev() {
            let t = self.mosaic.texel_of(
                proj.u(f64::from(w.x) + 0.5),
                proj.v(f64::from(w.y), f64::from(w.z) + 0.5),
            );
            let (x, y) = wp::screen_of(t, src, (view.x, view.y, view.w));
            let label = w.label();
            let head_w = wp::head_width(render::text_width(&label));
            let placed = wp::place(&view, x, y, head_w);
            let s = wp::pin_shape(placed, head_w);
            let pt = |(x, y): (f32, f32)| Point { x, y };
            let (hx, hy, hw, hh) = s.head;
            let q = s.point;
            let tail = |color| {
                Shape::Quad(QuadShape {
                    corners: (pt(q[0]), pt(q[1]), pt(q[2]), pt(q[3])),
                    color,
                })
            };
            // A dark rim, the point, the head, its letters.
            out.push(Shape::Fill(FillShape {
                rect: Rect {
                    x: hx - 1.0,
                    y: hy - 1.0,
                    w: hw + 2.0,
                    h: hh + 2.0,
                },
                color: OUTLINE,
            }));
            // The caret of a badge gets a rim too; a pin's tail is its colour.
            if matches!(placed, Placed::Badge { .. }) {
                let grow = |(x, y): (f32, f32)| {
                    let (dx, dy) = (x - s.label.0, y - (hy + hh / 2.0));
                    let d = dx.hypot(dy).max(1e-3);
                    (x + dx / d, y + dy / d)
                };
                out.push(Shape::Quad(QuadShape {
                    corners: (
                        pt(grow(q[0])),
                        pt(grow(q[1])),
                        pt(grow(q[2])),
                        pt(grow(q[3])),
                    ),
                    color: OUTLINE,
                }));
            }
            out.push(tail(w.color | 0xFF00_0000));
            out.push(Shape::Fill(FillShape {
                rect: Rect {
                    x: hx,
                    y: hy,
                    w: hw,
                    h: hh,
                },
                color: w.color | 0xFF00_0000,
            }));
            out.push(Shape::Text(TextShape {
                text: label,
                at: pt(s.label),
                color: wp::label_color(w.color),
                centered: true,
                shadow: false,
            }));
        }
    }
}
