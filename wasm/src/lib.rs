//! Diorama as a Horizon client mod (wasm32, API level 17). M2: a flat checkerboard
//! projected as the map will be, with the player arrow, in a box at the top right.
//! All maths is in `diorama-core`; this file only talks to Horizon.
use diorama_core::{
    layout,
    pattern::{self, Anchor, TEXELS_PER_BLOCK},
    Projection,
};
use horizon_mod_sdk::{
    canvas::{self, ImageShape, Shape},
    game,
    render::{self, Corner, FillShape, Point, QuadShape, Rect},
    Mod, Registration,
};

/// The image's side in texels: 128 blocks across at 2 texels per block.
const IMAGE: u32 = 256;
/// The anchor moves in steps of this many blocks (the image is redrawn then).
const STEP: f64 = 8.0;
/// GUI pixels per block (Diorama's default zoom).
const ZOOM: f32 = 2.0;
/// Box side as a percentage of the GUI width.
const SIZE_PERCENT: f32 = 10.0;
const BORDER: f32 = 1.0;
const BORDER_COLOR: u32 = 0xFF20_2020;
const ARROW_COLOR: u32 = 0xFFE8_3030;
const ARROW_OUTLINE: u32 = 0xFF00_0000;

struct Diorama {
    proj: Projection,
    image: canvas::Image,
    anchor: Option<Anchor>,
    /// What the box last showed, so it is only sent again when it changes.
    shown: Option<(f32, [i32; 3])>,
}

impl Diorama {
    fn draw(&mut self) {
        let Some(p) = game::get_player() else {
            if self.shown.take().is_some() {
                canvas::clear_corner();
            }
            return;
        };
        let (gui_w, _) = render::gui_size();
        let side = layout::box_side(gui_w, SIZE_PERCENT);

        let ground = p.y.floor();
        let (u, v) = (self.proj.u(p.x), self.proj.v(ground, p.z));
        let want = Anchor::around(&self.proj, p.x, p.y, p.z, IMAGE, STEP);
        if self.anchor != Some(want) {
            let rgba = pattern::checkerboard(&self.proj, &want, IMAGE);
            match canvas::update_image(self.image, 0, 0, IMAGE, IMAGE, &rgba) {
                Ok(()) => self.anchor = Some(want),
                Err(e) => horizon_mod_sdk::warn(&format!("update-image: {e}")),
            }
        }
        let Some(anchor) = self.anchor else { return };

        // Only resend when the view moved by a tenth of a texel or a degree.
        let key = (
            side,
            [
                (u * 20.0) as i32,
                (v * 20.0) as i32,
                p.yaw.rem_euclid(360.0) as i32,
            ],
        );
        if self.shown == Some(key) {
            return;
        }
        self.shown = Some(key);

        let (cx, cy) = anchor.texel(u, v);
        let (sx, sy, sw, sh) =
            layout::src_window(cx, cy, side - 2.0 * BORDER, ZOOM, TEXELS_PER_BLOCK, IMAGE);
        let full = side;
        let inner = side - 2.0 * BORDER;
        let mid = full / 2.0;
        let mut shapes = vec![
            Shape::Fill(FillShape {
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: full,
                    h: full,
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
        let size = (full / 10.0).clamp(6.0, 14.0);
        for (s, color) in [(size + 2.0, ARROW_OUTLINE), (size, ARROW_COLOR)] {
            for q in layout::arrow(mid, mid, s, p.yaw) {
                let pt = |(x, y): (f32, f32)| Point { x, y };
                shapes.push(Shape::Quad(QuadShape {
                    corners: (pt(q[0]), pt(q[1]), pt(q[2]), pt(q[3])),
                    color,
                }));
            }
        }
        canvas::set_corner(Corner::TopRight, full, full, &shapes);
    }
}

impl Mod for Diorama {
    fn init() -> Result<(Self, Registration), String> {
        let image = canvas::create_image(IMAGE, IMAGE)?;
        let state = Diorama {
            proj: Projection::default(),
            image,
            anchor: None,
            shown: None,
        };
        Ok((state, Registration::new().on_frame()))
    }
    fn frame(&mut self, _seconds: f32) {
        self.draw();
    }
}
horizon_mod_sdk::export_mod!(Diorama);
