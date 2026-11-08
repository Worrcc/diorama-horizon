**Unofficial** port of Diorama by Dara Adedeji (github.com/SunkenInTime/minecraft-orthogonal-map, Modrinth `diorama-map`) to Horizon. Not affiliated with or endorsed by the Diorama author or Mojang.

Status: M7. The map bakes the loaded chunks (top and south faces, nearest face per texel) into a 512 x 512 mosaic
around the player, in Diorama's colours: each block's textures from the loaded resource packs averaged at run
time, biome tints, clear water by depth, the default style's light, height shade, grade and outlines. A resource
pack change recolours the map after the reload. `/diorama colour <block>` shows a block's averaged colours.

M6 adds waypoints: **B** adds one where you stand, **U** opens the list (teleport, colour, hide, delete), and
`/dwp add|del|rename|color|list|hide|show|tp` does the rest (there is no text field, so names go through
`/dwp`). Death markers (the last 3) are added when you die. Waypoints show as pins on the map, or as badges on its
edge pointing toward them. They are kept per world in the mod's storage, in Diorama's own `waypoints.json`
format (RULES.md section 15).

## What's in and out

In: the angled minimap at the top right (about 10% of the screen width, square or round), Diorama's 50 degree
oblique projection facing north, top and south faces coloured from your loaded resource packs, biome tints, shading,
outlines, clear-water depth, Diorama's colour grade, the cave slice underground and the Nether roof slice, zoom
steps, toggle and enlarge keys, compass letters, coordinate/biome/time lines, settings, and waypoints (manual and
death markers, pins and edge badges, a list screen, teleport via `/tp`).

Out: the fullscreen map, the explored-map save, fog of war and PNG export, radar and mob faces, drawings, claims
and FTB, Xaero interop, crafts, folders and icons, chat `[Add]` links, GPU baking and sun shadow rays. See RULES.md
for the numbers taken from Diorama and the dropped features.

## Requirements

A build of the Horizon-iOS fork (github.com/Worrcc/Horizon-iOS-worc, branch `claude/mod-canvas-api`) with mod API
level 17 (canvas and client-world). It does not run in upstream Horizon nightlies, which have no WASM mods.
The mod asks for the capabilities `game-read`, `render`, `world-read`, `input`, `ui`, `storage`, `chat` and `command`.

## Limits (M7)

- On iOS (Pulley interpreter) the map runs at half resolution.
- Sprint flying can exceed the 0.3 ms per-frame budget while the map fills in.
- The test on a real phone is still pending.

## Install

- Desktop: build or download `diorama.hmod` (`target/horizon-mod/diorama.hmod`, or the release asset) and put it in
  the mods folder of the fork build, or run `cargo horizon-mod run`.
- iOS: install the fork's IPA, then add `diorama.hmod` to the app's mods folder (Files app).

## Building

```
cargo test -p diorama-core
cargo build --target wasm32-unknown-unknown --release -p diorama
cd wasm && cargo horizon-mod build        # writes target/horizon-mod/diorama.hmod
cd wasm && cargo horizon-mod test         # scenario scripts in wasm/tests
```

`cargo-horizon-mod` comes from the fork: `cargo install --git https://github.com/Worrcc/Horizon-iOS-worc --rev
5082b61b08c8dcf628e18accfe2fbe667bfee57d cargo-horizon-mod`. The SDK is pulled from the same revision. (`bench/`
is a separate workspace that needs a sibling checkout of the fork.)

## Credits

Diorama by Dara Adedeji (MIT), github.com/SunkenInTime/minecraft-orthogonal-map. No Diorama assets are copied; all
block colours are computed at run time from the player's loaded resource packs. Built on the Horizon-iOS fork of
Horizon (see the fork's own licence notices).

## License

MIT. See LICENSE (Diorama's notice kept, plus the port's).
