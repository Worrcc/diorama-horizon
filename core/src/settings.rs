//! The mod's settings in Horizon's per-key style (PLAN-v2 section 4): one
//! storage key per setting, as plain text, read in `init` with an unknown
//! value falling back to the default, one cycling button each.
//! The order is the settings page's: Horizon's page has no scrolling, so the
//! settings that matter most come first.
//! Defaults are Diorama's (RULES.md section 9) except the size, which is
//! the plan's 10% of the screen width.

use crate::layout::Corner;

/// One setting: its storage key, its page label and its values, stored text
/// and shown text, the default first.
pub struct Setting {
    pub key: &'static str,
    pub label: &'static str,
    pub values: &'static [(&'static str, &'static str)],
}

const ON_OFF: &[(&str, &str)] = &[("true", "ON"), ("false", "OFF")];

pub const MINIMAP: usize = 0;
pub const CORNER: usize = 1;
pub const SIZE: usize = 2;
pub const ZOOM: usize = 3;
pub const OPACITY: usize = 4;
pub const SHAPE: usize = 5;
pub const AUTO_CAVES: usize = 6;
pub const PREFER_SURFACE: usize = 7;
pub const COORDINATES: usize = 8;
pub const BIOME: usize = 9;
pub const COMPASS: usize = 10;
pub const GAME_TIME: usize = 11;
pub const STYLE: usize = 12;
pub const WAYPOINTS_ON_MAP: usize = 13;
pub const DEATH_MARKERS: usize = 14;
pub const WAYPOINT_DISTANCE: usize = 15;

pub const SETTINGS: [Setting; 16] = [
    Setting {
        key: "minimap",
        label: "Minimap",
        values: ON_OFF,
    },
    Setting {
        key: "corner",
        label: "Corner",
        values: &[
            ("top_right", "Top right"),
            ("top_left", "Top left"),
            ("bottom_left", "Bottom left"),
            ("bottom_right", "Bottom right"),
        ],
    },
    Setting {
        key: "size",
        label: "Size (% of screen width)",
        values: &[
            ("10", "10%"),
            ("8", "8%"),
            ("12", "12%"),
            ("15", "15%"),
            ("20", "20%"),
        ],
    },
    Setting {
        key: "zoom",
        label: "Zoom (pixels per block)",
        values: &[
            ("2", "2"),
            ("3", "3"),
            ("4", "4"),
            ("0.5", "0.5"),
            ("1", "1"),
            ("1.5", "1.5"),
        ],
    },
    Setting {
        key: "opacity",
        label: "Opacity",
        values: &[("100", "100%"), ("80", "80%"), ("60", "60%"), ("40", "40%")],
    },
    Setting {
        key: "shape",
        label: "Shape",
        values: &[("square", "Square"), ("round", "Round")],
    },
    Setting {
        key: "auto_caves",
        label: "Cave view underground",
        values: ON_OFF,
    },
    Setting {
        key: "prefer_surface",
        label: "Keep the surface under a roof",
        values: ON_OFF,
    },
    Setting {
        key: "coordinates",
        label: "Coordinates",
        values: ON_OFF,
    },
    Setting {
        key: "biome",
        label: "Biome name",
        values: ON_OFF,
    },
    Setting {
        key: "compass",
        label: "Compass letters",
        values: ON_OFF,
    },
    Setting {
        key: "game_time",
        label: "Game time",
        values: &[("off", "OFF"), ("clock", "Clock")],
    },
    Setting {
        key: "style",
        label: "Style",
        values: &[("default", "Default"), ("flat", "Flat")],
    },
    // Waypoints (Settings.java:112-124). The page does not scroll, so these
    // come last; `/dwp pins|deaths|within` sets them too.
    Setting {
        key: "waypoints_on_map",
        label: "Waypoints on the map",
        values: ON_OFF,
    },
    Setting {
        key: "death_markers",
        label: "Death markers",
        values: ON_OFF,
    },
    Setting {
        key: "waypoint_distance",
        label: "Show waypoints within",
        values: &[
            ("0", "Any distance"),
            ("64", "64 blocks"),
            ("128", "128 blocks"),
            ("256", "256 blocks"),
            ("512", "512 blocks"),
            ("1024", "1024 blocks"),
            ("2048", "2048 blocks"),
        ],
    },
];

/// The zoom steps in pixels per block, ascending: the plan's subset of
/// Diorama's table (Settings.java:34), which also has 0.1875, 0.25, 0.375
/// and 0.75 (RULES.md section 13).
pub const ZOOM_STEPS: [f32; 6] = [0.5, 1.0, 1.5, 2.0, 3.0, 4.0];

/// The step nearest `zoom` (Settings.java:297-305).
pub fn zoom_step(zoom: f32) -> usize {
    let mut best = 0;
    for (i, z) in ZOOM_STEPS.iter().enumerate() {
        if (z - zoom).abs() < (ZOOM_STEPS[best] - zoom).abs() {
            best = i;
        }
    }
    best
}

/// The settings' current values, as an index into each one's `values`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    at: [usize; SETTINGS.len()],
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            at: [0; SETTINGS.len()],
        }
    }
}

impl Settings {
    /// Reads every setting with `get(key)`, an unknown value keeping the default.
    pub fn load(get: impl Fn(&str) -> Option<String>) -> Settings {
        let mut s = Settings::default();
        for (i, def) in SETTINGS.iter().enumerate() {
            if let Some(v) = get(def.key) {
                let v = v.trim();
                if let Some(at) = def.values.iter().position(|(stored, _)| *stored == v) {
                    s.at[i] = at;
                }
            }
        }
        s
    }
    /// The next value of setting `i`, wrapping.
    pub fn cycle(&mut self, i: usize) {
        self.at[i] = (self.at[i] + 1) % SETTINGS[i].values.len();
    }
    /// The text kept for setting `i`.
    pub fn stored(&self, i: usize) -> &'static str {
        SETTINGS[i].values[self.at[i]].0
    }
    /// "Name: Value" for setting `i`.
    pub fn button(&self, i: usize) -> String {
        format!(
            "{}: {}",
            SETTINGS[i].label, SETTINGS[i].values[self.at[i]].1
        )
    }
    pub fn flag(&self, i: usize) -> bool {
        self.stored(i) == "true"
    }
    pub fn corner(&self) -> Corner {
        match self.stored(CORNER) {
            "top_left" => Corner::TopLeft,
            "bottom_left" => Corner::BottomLeft,
            "bottom_right" => Corner::BottomRight,
            _ => Corner::TopRight,
        }
    }
    /// Box side as a percentage of the GUI width.
    pub fn size_percent(&self) -> f32 {
        self.stored(SIZE).parse().unwrap_or(10.0)
    }
    pub fn round(&self) -> bool {
        self.stored(SHAPE) == "round"
    }
    /// Pixels per block.
    pub fn zoom(&self) -> f32 {
        self.stored(ZOOM).parse().unwrap_or(2.0)
    }
    /// Sets the zoom to the step `step` (clamped to the table).
    pub fn set_zoom_step(&mut self, step: usize) {
        let z = ZOOM_STEPS[step.min(ZOOM_STEPS.len() - 1)];
        let text = format!("{z}");
        if let Some(at) = SETTINGS[ZOOM].values.iter().position(|(s, _)| *s == text) {
            self.at[ZOOM] = at;
        }
    }
    /// One zoom step in (+1, nearer) or out (-1), kept inside the table.
    pub fn zoom_by(&mut self, delta: i32) {
        let step = (zoom_step(self.zoom()) as i32 + delta).clamp(0, ZOOM_STEPS.len() as i32 - 1);
        self.set_zoom_step(step as usize);
    }
    /// The tint alpha for the opacity, 0..=255.
    pub fn alpha(&self) -> u32 {
        let pct: u32 = self.stored(OPACITY).parse().unwrap_or(100);
        (pct * 255 + 50) / 100
    }
    /// Sets setting `i` to its stored text `value`; false (and unchanged)
    /// for a value it does not have.
    pub fn set_stored(&mut self, i: usize, value: &str) -> bool {
        match SETTINGS[i].values.iter().position(|(v, _)| *v == value) {
            Some(at) => {
                self.at[i] = at;
                true
            }
            None => false,
        }
    }
    /// "Show waypoints within" in blocks, 0 for any distance.
    pub fn waypoint_distance(&self) -> u32 {
        self.stored(WAYPOINT_DISTANCE).parse().unwrap_or(0)
    }
    pub fn flat_style(&self) -> bool {
        self.stored(STYLE) == "flat"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_diorama_s() {
        let s = Settings::default();
        assert!(s.flag(MINIMAP) && s.flag(AUTO_CAVES) && s.flag(PREFER_SURFACE));
        assert_eq!(s.corner(), Corner::TopRight);
        assert_eq!(s.size_percent(), 10.0);
        assert_eq!(s.zoom(), 2.0);
        assert_eq!(s.alpha(), 255);
        assert!(!s.round() && !s.flat_style());
        assert_eq!(s.stored(GAME_TIME), "off");
        assert!(s.flag(WAYPOINTS_ON_MAP) && s.flag(DEATH_MARKERS));
        assert_eq!(s.waypoint_distance(), 0);
    }

    #[test]
    fn unknown_values_keep_the_default_and_known_ones_load() {
        let s = Settings::load(|k| match k {
            "corner" => Some("bottom_left\n".into()),
            "zoom" => Some("7".into()),
            "size" => Some("15".into()),
            _ => None,
        });
        assert_eq!(s.corner(), Corner::BottomLeft);
        assert_eq!(s.zoom(), 2.0);
        assert_eq!(s.size_percent(), 15.0);
    }

    #[test]
    fn cycling_wraps_and_round_trips_through_storage() {
        let mut s = Settings::default();
        for _ in 0..SETTINGS[CORNER].values.len() {
            s.cycle(CORNER);
        }
        assert_eq!(s.corner(), Corner::TopRight);
        s.cycle(CORNER);
        s.cycle(MINIMAP);
        let kept: Vec<_> = (0..SETTINGS.len())
            .map(|i| (SETTINGS[i].key, s.stored(i)))
            .collect();
        let back = Settings::load(|k| {
            kept.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        });
        assert_eq!(back, s);
        assert_eq!(s.button(CORNER), "Corner: Top left");
        assert!(!s.flag(MINIMAP));
    }

    #[test]
    fn zoom_steps_stop_at_the_ends() {
        let mut s = Settings::default();
        s.zoom_by(1);
        assert_eq!(s.zoom(), 3.0);
        s.zoom_by(1);
        s.zoom_by(1);
        assert_eq!(s.zoom(), 4.0);
        for _ in 0..10 {
            s.zoom_by(-1);
        }
        assert_eq!(s.zoom(), 0.5);
        s.zoom_by(1);
        assert_eq!(s.zoom(), 1.0);
        assert_eq!(zoom_step(1.4), 2);
        // Every step has a stored value, so the keys can reach all of them.
        for step in 0..ZOOM_STEPS.len() {
            s.set_zoom_step(step);
            assert_eq!(s.zoom(), ZOOM_STEPS[step]);
        }
    }
}
