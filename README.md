**Unofficial** port of Diorama by Dara Adedeji (github.com/SunkenInTime/minecraft-orthogonal-map, Modrinth `diorama-map`) to Horizon. Not affiliated with or endorsed by the Diorama author or Mojang.

Status: M4. The map bakes the loaded chunks (top and south faces, nearest face per texel) into a 512 x 512 mosaic
around the player, in Diorama's colours: each block's textures from the loaded resource packs averaged at run
time, biome tints, clear water by depth, the default style's light, height shade, grade and outlines. A resource
pack change recolours the map after the reload. `/diorama colour <block>` shows a block's averaged colours.

## What's in and out

TODO (PLAN-v2 section 1). See RULES.md for the numbers taken from Diorama and the dropped features.

## Requirements

TODO: the Horizon-iOS fork build with API level 17. It does not run in upstream Horizon nightlies.

## Install

TODO: desktop (fork build) and iOS.

## Building

TODO: `cargo test -p diorama-core`, `cargo build --target wasm32-unknown-unknown -p diorama`, packaging `diorama.hmod`.

## Credits

Diorama by Dara Adedeji (MIT). No Diorama assets are copied; all block colours are computed at run time from the
player's loaded resource packs.

## License

MIT. See LICENSE (Diorama's notice kept, plus the port's).
