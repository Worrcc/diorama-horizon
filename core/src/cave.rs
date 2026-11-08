//! When the map shows a cave slice instead of the surface, and where it cuts
//! (RULES.md section 8; MapSession.java:623-696). Pure rules: the mod reads
//! the light and the dimension from Horizon and asks here.
//!
//! The port's own choices, flagged in the M5 report: "does not see the sky"
//! is a sky light below 15 at the eye (Horizon has no `canSeeSky`), and the
//! cut sits at the head, not 2 to 5 blocks above it (see [`follow`]).

/// A roof thinner than this is not a cave (MapSession.java:50).
pub const MIN_CAVE_DEPTH: u32 = 8;
/// How far up the sky is looked for (MapSession.java:51).
pub const MAX_SKY_DEPTH: u32 = 32;
/// Sky light below which the player becomes "underground", and the one below
/// which they stay so (MapSession.java:629-634), so a cave mouth does not flip
/// the map back and forth.
pub const ENTER_SKY: u8 = 5;
pub const STAY_SKY: u8 = 10;

/// Is the player underground? A dimension with a ceiling always is. Else with
/// hysteresis on the sky light (0..15) at the eye; a sky light of 15 counts as
/// seeing the sky.
pub fn underground(was: bool, ceiling: bool, sky: u8) -> bool {
    if ceiling {
        return true;
    }
    let covered = sky < 15;
    if was {
        covered && sky < STAY_SKY
    } else {
        covered && sky < ENTER_SKY
    }
}

/// Blocks from the eye up to the first block with sky light 15, at most
/// [`MAX_SKY_DEPTH`]; [`MAX_SKY_DEPTH`] + 1 when there is none
/// (MapSession.java:687-696). `sky(dy)` is the sky light `dy` blocks above
/// the eye, none past the loaded world.
pub fn depth_to_sky(mut sky: impl FnMut(u32) -> Option<u8>) -> u32 {
    for dy in 0..=MAX_SKY_DEPTH {
        match sky(dy) {
            Some(15) => return dy,
            Some(_) => {}
            None => break,
        }
    }
    MAX_SKY_DEPTH + 1
}

/// Whether the map shows a slice. Not underground: the surface. Underground
/// under a roof (sky within [`MIN_CAVE_DEPTH`] above) with `prefer_surface`:
/// the surface. Otherwise the slice (MapSession.java:661-671). `auto_caves`
/// off keeps the surface everywhere. `depth` is only asked for when needed.
pub fn slice(
    auto_caves: bool,
    prefer_surface: bool,
    underground: bool,
    depth: impl FnOnce() -> u32,
    ceiling: bool,
) -> bool {
    if !auto_caves || !underground {
        return false;
    }
    // A ceiling dimension has no sky to find: always a slice (RULES.md 13.6).
    ceiling || !prefer_surface || depth() >= MIN_CAVE_DEPTH
}

/// The height the slice cuts at, given the one before. The cut is the head's
/// block y (the floor under it is what the map shows). It follows the head
/// down at once, since a lower roof would hide the map, and up only when the
/// head is 3 or more above it, so a jump or a stair does not rebake the map
/// (Diorama's `updateCaveLayer` is the same idea with its own cut).
pub fn follow(cut: Option<i32>, head: i32) -> i32 {
    match cut {
        Some(c) if head >= c && head < c + 3 => c,
        _ => head,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nether_is_always_underground() {
        assert!(underground(false, true, 15));
        assert!(underground(true, true, 15));
    }

    #[test]
    fn underground_has_hysteresis() {
        // Not underground: needs sky light under 5.
        assert!(!underground(false, false, 5));
        assert!(underground(false, false, 4));
        // Underground: stays until 10.
        assert!(underground(true, false, 9));
        assert!(!underground(true, false, 10));
        // Full sky light is the open sky either way.
        assert!(!underground(true, false, 15));
    }

    #[test]
    fn depth_to_sky_counts_up_to_light_15() {
        assert_eq!(depth_to_sky(|dy| Some(if dy >= 3 { 15 } else { 0 })), 3);
        assert_eq!(depth_to_sky(|_| Some(0)), MAX_SKY_DEPTH + 1);
        assert_eq!(depth_to_sky(|dy| (dy < 2).then_some(0)), MAX_SKY_DEPTH + 1);
        assert_eq!(depth_to_sky(|_| Some(15)), 0);
    }

    #[test]
    fn a_roof_keeps_the_surface_unless_it_is_deep() {
        let deep = || MAX_SKY_DEPTH + 1;
        let roof = || 3;
        assert!(!slice(true, true, false, deep, false));
        assert!(!slice(true, true, true, roof, false));
        assert!(slice(true, true, true, deep, false));
        assert!(slice(true, false, true, roof, false));
        assert!(!slice(false, false, true, deep, false));
        // The Nether does not look for sky.
        assert!(slice(true, true, true, || panic!("not asked"), true));
    }

    #[test]
    fn the_cut_follows_down_at_once_and_up_after_three() {
        assert_eq!(follow(None, 40), 40);
        assert_eq!(follow(Some(40), 38), 38);
        assert_eq!(follow(Some(40), 41), 40);
        assert_eq!(follow(Some(40), 42), 40);
        assert_eq!(follow(Some(40), 43), 43);
    }
}
