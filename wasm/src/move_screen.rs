//! Drag-to-place editor. The HUD's own shapes remain live while editing.
use crate::*;
use horizon_mod_sdk::PointerKind;

pub const MOVE: u32 = 200;
pub const OFFSET_X: u32 = 201;
pub const OFFSET_Y: u32 = 202;
const DESKTOP_HINT: &str = "Drag to move - arrows nudge (Shift x10) - R reset - Esc to save";
const TOUCH_HINT: &str = "Drag the minimap, then tap Done";

pub struct MoveScreen {
    at: (f32, f32),
    drag: Option<(f32, f32)>,
    shift: [bool; 2],
    reset: bool,
    corner: MapCorner,
}

impl Diorama {
    pub fn open_move(&mut self) {
        let Some(p) = game::get_player() else { return };
        self.close_fullmap();
        self.list_closed();
        ui::close_screen();
        self.shown = None;
        self.draw(&p);
        let (corner, offset) = layout::default_place(
            self.touch,
            self.placement_stored,
            self.settings.corner(),
            self.settings.offset(),
        );
        let r = layout::place(corner, offset, self.box_size, render::gui_size());
        self.moving = Some(MoveScreen {
            at: (r.x, r.y),
            drag: None,
            shift: [false; 2],
            reset: offset == (0.0, 0.0),
            corner,
        });
        canvas::open_canvas_screen("Move minimap", &[]);
        self.draw_move();
    }

    pub fn finish_move(&mut self) {
        let Some(m) = self.moving.take() else { return };
        let (corner, offset) = if m.reset {
            (m.corner, (0.0, 0.0))
        } else {
            layout::snap(m.at, self.box_size, render::gui_size())
        };
        let key = match corner {
            MapCorner::TopLeft => "top_left",
            MapCorner::TopRight => "top_right",
            MapCorner::BottomLeft => "bottom_left",
            MapCorner::BottomRight => "bottom_right",
        };
        self.settings.set_stored(settings::CORNER, key);
        self.settings.set_offset(offset);
        self.placement_stored = true;
        self.save(settings::CORNER);
        let (x, y) = self.settings.offset_stored();
        let _ = storage::set("offset_x", x.as_bytes());
        let _ = storage::set("offset_y", y.as_bytes());
        canvas::close_canvas_screen();
        self.rev = self.rev.wrapping_add(1);
        self.shown = None;
        self.page();
        if let Some(p) = game::get_player() {
            self.draw(&p);
        }
        if !self.settings.flag(settings::MINIMAP) {
            canvas::clear_corner();
        }
    }

    pub fn move_pointer(&mut self, kind: PointerKind, x: f32, y: f32, button: u32) {
        if button != 0 {
            return;
        }
        let Some(m) = &mut self.moving else { return };
        if kind == PointerKind::Down && self.touch {
            let buttons = layout::touch_buttons(
                render::gui_size(),
                &[layout::TouchButton::Done, layout::TouchButton::Reset],
            );
            match layout::hit_button(&buttons, x, y) {
                Some(layout::TouchButton::Done) => {
                    self.finish_move();
                    return;
                }
                Some(layout::TouchButton::Reset) => {
                    let r = layout::place(m.corner, (0.0, 0.0), self.box_size, render::gui_size());
                    m.at = (r.x, r.y);
                    m.reset = true;
                    m.drag = None;
                    self.draw_move();
                    return;
                }
                _ => {}
            }
        }
        match kind {
            PointerKind::Down
                if x >= m.at.0
                    && y >= m.at.1
                    && x <= m.at.0 + self.box_size.0
                    && y <= m.at.1 + self.box_size.1 =>
            {
                m.drag = Some((x - m.at.0, y - m.at.1));
            }
            PointerKind::Drag => {
                if let Some((dx, dy)) = m.drag {
                    m.at = (x - dx, y - dy);
                    m.reset = false;
                }
            }
            PointerKind::Up => m.drag = None,
            _ => {}
        }
        self.draw_move();
    }

    pub fn move_key(&mut self, key: &str, down: bool) {
        let Some(m) = &mut self.moving else { return };
        match key {
            "key.keyboard.left.shift" => m.shift[0] = down,
            "key.keyboard.right.shift" => m.shift[1] = down,
            _ if !down => return,
            "key.keyboard.r" => {
                let r = layout::place(m.corner, (0.0, 0.0), self.box_size, render::gui_size());
                m.at = (r.x, r.y);
                m.reset = true;
                m.drag = None;
            }
            "key.keyboard.enter" => {
                self.finish_move();
                return;
            }
            _ => {
                let step = if m.shift.iter().any(|&v| v) {
                    10.0
                } else {
                    1.0
                };
                match key {
                    "key.keyboard.left" => m.at.0 -= step,
                    "key.keyboard.right" => m.at.0 += step,
                    "key.keyboard.up" => m.at.1 -= step,
                    "key.keyboard.down" => m.at.1 += step,
                    _ => return,
                }
                m.reset = false;
            }
        }
        self.draw_move();
    }

    pub fn draw_move(&mut self) {
        let Some(m) = &mut self.moving else { return };
        let gui = render::gui_size();
        let r = layout::place(MapCorner::TopLeft, m.at, self.box_size, gui);
        m.at = (r.x, r.y);
        let mut out = vec![Shape::Fill(FillShape {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                w: gui.0,
                h: gui.1,
            },
            color: 0xB000_0000,
        })];
        for mut shape in self.box_shapes.clone() {
            let translate = |p: &mut Point| {
                p.x += r.x;
                p.y += r.y;
            };
            match &mut shape {
                Shape::Image(s) => {
                    s.dst.x += r.x;
                    s.dst.y += r.y;
                }
                Shape::Fill(s) => {
                    s.rect.x += r.x;
                    s.rect.y += r.y;
                }
                Shape::Quad(s) => {
                    translate(&mut s.corners.0);
                    translate(&mut s.corners.1);
                    translate(&mut s.corners.2);
                    translate(&mut s.corners.3);
                }
                Shape::Text(s) => translate(&mut s.at),
                _ => {}
            }
            out.push(shape);
        }
        // Dashed outline around the entire draggable HUD box.
        for (length, horizontal) in [(r.w, true), (r.h, false)] {
            let mut d = 0.0;
            while d < length {
                for far in [false, true] {
                    out.push(Shape::Fill(FillShape {
                        rect: if horizontal {
                            Rect {
                                x: r.x + d,
                                y: r.y + if far { r.h } else { 0.0 },
                                w: 3.0f32.min(length - d),
                                h: 1.0,
                            }
                        } else {
                            Rect {
                                x: r.x + if far { r.w } else { 0.0 },
                                y: r.y + d,
                                w: 1.0,
                                h: 3.0f32.min(length - d),
                            }
                        },
                        color: 0xFFFF_FFFF,
                    }));
                }
                d += 6.0;
            }
        }
        let touch = self.touch;
        out.push(Shape::Text(TextShape {
            text: if touch { TOUCH_HINT } else { DESKTOP_HINT }.into(),
            at: Point {
                x: gui.0 / 2.0,
                // Above the Done and Reset buttons on a touch screen.
                y: gui.1 - if touch { 44.0 } else { 12.0 },
            },
            color: 0xFFFF_FFFF,
            centered: true,
            shadow: true,
        }));
        if touch {
            let buttons = layout::touch_buttons(
                gui,
                &[layout::TouchButton::Done, layout::TouchButton::Reset],
            );
            for (button, r) in buttons {
                out.push(Shape::Fill(FillShape {
                    rect: Rect {
                        x: r.x,
                        y: r.y,
                        w: r.w,
                        h: r.h,
                    },
                    color: 0xB000_0000,
                }));
                out.push(Shape::Fill(FillShape {
                    rect: Rect {
                        x: r.x,
                        y: r.y,
                        w: r.w,
                        h: 1.0,
                    },
                    color: 0xCCFF_FFFF,
                }));
                out.push(Shape::Fill(FillShape {
                    rect: Rect {
                        x: r.x,
                        y: r.y + r.h - 1.0,
                        w: r.w,
                        h: 1.0,
                    },
                    color: 0xCCFF_FFFF,
                }));
                out.push(Shape::Fill(FillShape {
                    rect: Rect {
                        x: r.x,
                        y: r.y,
                        w: 1.0,
                        h: r.h,
                    },
                    color: 0xCCFF_FFFF,
                }));
                out.push(Shape::Fill(FillShape {
                    rect: Rect {
                        x: r.x + r.w - 1.0,
                        y: r.y,
                        w: 1.0,
                        h: r.h,
                    },
                    color: 0xCCFF_FFFF,
                }));
                out.push(Shape::Text(TextShape {
                    text: match button {
                        layout::TouchButton::Done => "Done",
                        _ => "Reset",
                    }
                    .into(),
                    at: Point {
                        x: r.x + r.w / 2.0,
                        y: r.y + 6.0,
                    },
                    color: 0xFFFF_FFFF,
                    centered: true,
                    shadow: true,
                }));
            }
        }
        canvas::set_canvas_screen(&out);
    }
}
