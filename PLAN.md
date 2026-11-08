# Diorama -> Horizon: port plan (planner draft, awaiting approval)

Source: Diorama 0.7.0 for 26.3 (Modrinth `kwpTgbCk`, `diorama-map`; github.com/SunkenInTime/minecraft-orthogonal-map),
MIT, (c) 2026 Dara Adedeji. Client-only. Route: **native mixin mod (`.hnmod`)**, desktop only. No content `.hmod`
(Diorama adds no blocks or items).

## 0. What I found (drives the scope)

- **Jar size:** ~1.64 MB of bytecode, ~130 top-level classes. Largest: `MapScreen` (64 KB), `render/MapLevel` (57 KB),
  `bake/TileBaker` (40 KB), `waypoint/WaypointListScreen` (37 KB), `SettingsScreen` (35 KB), `bake/GpuMesher` (31 KB),
  `bake/BlockLooks` (30 KB). Porting all of it is months, not weeks.
- **What makes Diorama Diorama** is the projection, not the HUD: an oblique orthographic view looking north and down
  at a pitch (`Projection`: `v = z*sin(pitch) - y*cos(pitch)`, depth `t = -(y*sin + z*cos)`; tiles are 128 blocks,
  4 px per block, 512 px). Tiles are baked on the GPU (`bake.glsl`, `GpuTileBaker`, `BlazeBakeBackend`/`GlBakeBackend`)
  with textured block faces, sun shadow maps, 13 sky-occlusion rays, cave/roof lighting and colour grading.
- **Horizon already has a minimap** (`horizon-client-render/src/minimap.rs`, drawn by `game/extras.rs` and
  `extras_hud::draw_minimap`), but only for a native mod that answers the `native_minimap` hook (Horizon's own official
  "Minimap" native mod in horizon-rust/horizon-mods `native/minimap`). It is top-down map-item colours, a 128x128
  image in the GUI atlas (`MINIMAP_SPRITE`), with corner placement, round/square, player arrow and N marker.
  Horizon also has map items (`game/maps.rs`) and the vanilla locator bar (`lan_waypoints.rs`), but **no fullscreen
  map, no waypoint manager, no radar**.
- **No hook point lets a mod supply its own minimap image or open its own screen.** `native_minimap` only picks
  corner/size/scale/rotate/round; Horizon composes the image itself. To show the angled view the mod must take over
  `Game::update_minimap` (private, `game/extras.rs`) - an internal target, so the bundle must **pin a commit**
  rather than use `"hooks": N`.

## 1. Scope

**v1 = "Diorama minimap":** the angled diorama view as the HUD minimap, re-using Horizon's minimap frame.
1. Oblique projection minimap, north-up (the view is inherently north-facing; Horizon draws the turning arrow).
2. CPU tile baker: per-chunk tiles of top faces + south faces with flat face shading, height shade, water depth tint,
   block colours from Horizon's `BlockFacts` map colours (biome tint for grass/foliage/water if reachable, see Q6).
3. Cave / Nether-roof slice view: when the player is covered (sky light 0 at the head, as Horizon does) or in the
   Nether, bake a slice under the player's head height (Diorama's `CaveSight` rule, simplified).
4. Zoom levels (4, 2, 1, 0.5 px per block) on keys; toggle minimap key; corner, size, round/square in Options > Mods.
5. Info line under the map via `native_hud`: coords, biome (`game.biome_name_at`), optional.
6. Settings persisted with `native_set_setting` (one file per key in `horizon-mods/.horizon-mod-data/diorama/`).

**v1.1 (separate approval, optional):** waypoints - `/diorama wp add|del|list` via `native_command`, markers composited
into the minimap image and edge arrows, death marker from the death screen, stored as a small JSON/TOML in the data dir.

**Deferred (explicitly not v1):** fullscreen draggable map (`MapScreen`, `MapViewport`, `MapLevel`; needs a custom
screen and a larger texture - neither exists for mods); persistent explored map + fog (`TileStore`, `SavedChunks`,
`PngWriter`, `FogTextures`); PNG export; drawing/measure; radar (`Radar`, `MobFaces`, `XaeroRadarIcons`); claims /
FTB Chunks; Cobblemon/Pixelmon compat; Xaero channel interop and server rules (`XaeroChannels`, `ServerRules`,
payload mixins); waypoint GUI screens, folders, chat `[Add]` links, `/tp`; GPU sun shadows and sky-occlusion rays;
textured faces; Diorama's frame PNGs (assets are never copied).

**Why this cut:** the minimap is the one Diorama feature Horizon can host today with existing plumbing (image slot,
frame, corner stacking, settings, keys). Everything screen-based needs new Horizon UI hooks first. The baker written
for v1 is also the core every later feature reuses.

**Honest size estimate:** v1 is ~1,200-1,800 lines of Rust (projection/style ~150, baker ~600-800, compositor ~250,
settings/keys/HUD ~300, glue ~150) plus tests; 4-6 builder sessions including ~8-12 native builds (3-5 min each).
v1.1 waypoints: +600-900 lines. A full-feature port: several months and needs Horizon API additions.

## 2. Source analysis (v1 classes -> Horizon)

| Diorama (v1) | Role | Horizon target |
|---|---|---|
| `Projection`, `MapStyle` | pitch, sin/cos, tile constants, light/grade defaults | `projection.rs`, `style.rs` (constants from `javap -c -constants` of `MapStyle`, `Settings` `<clinit>`) |
| `bake/TileBaker`, `GpuMesher`, `ShapeFaces`, `TileSurface`, `SurfaceArea`, `TallChunks` | which faces of which blocks are visible, tile layout | CPU baker `bake.rs`: per chunk, per column from top block down, emit top face + south face where `z+1` is open; z-buffer on depth `t` |
| `bake/VoxelReader`, `ChunkSource`, `data/ChunkSnapshot`, `Sections` | read loaded chunk blocks | `game.world.chunks` / `game.world.block(x,y,z)`, `game.res.facts()` (solid, fluid, map colour) |
| `bake/BlockLooks`, `BiomeTints`, `TintProbe` | block colour from textures + biome tint | `BlockFacts` map colour (`MAP_COLORS`, as Horizon's `minimap::Palette`), biome tint if available (Q6) |
| `bake/Lighting`, `RockRays`, `light/Lights` | sun, occlusion, cave light | **simplified**: fixed face factors (top 1.0, south ~0.8), height shade, block/sky light from `world.light` in cave view. Shadows/rays dropped for v1 |
| `CaveSight`, `DepthGauge` | when to switch to cave/roof slice | same rule Horizon's minimap uses (`world.light(head) >> 4 == 0` or Nether), slice at head Y |
| `MinimapHud`, `render/MinimapFrames`, `Circle` | corner map, round mask, frame | Horizon `draw_extras`/`draw_minimap` (frame, round, arrow, N). Mod answers `native_minimap` |
| `Settings`, `SettingsScreen` (minimap part) | options | `native_settings` / `native_settings_click` buttons (cycle values), `native_setting`/`native_set_setting` |
| `DioramaClient` (key mappings) | keys | `native_mods` (declares keys), `native_mod_key` + `native_key` |
| `mixin/ClientChunkCacheMixin`, `LevelChunkMixin` | chunk load/change events | not needed: poll `world.chunks` keys + `world.revision`, rescan at most 1/s like Horizon's minimap |
| `mixin/GameRendererMixin` | draw hook | not needed (`update_minimap` takeover + HUD) |

Cannot port directly: Blaze3D/GL bake pipelines and GLSL shaders (mods get no GPU pass; replaced by CPU bake into the
128x128 RGBA patch), Mojang `NativeImage`/`DynamicTexture` (-> `horizon_client::font::Patch` into `map_patches`),
Mojang screens (no mod screen API), networking mixins (no client payload hook; dropped).

## 3. Design

- **Hooks (in `horizon-mixin.toml`, pinned `horizon_revision`):**
  - `native_minimap` HEAD: when on, set `Some(NativeMinimap { corner, size, blocks_per_pixel, rotate: false, round })`.
  - `horizon_client::game::Game<'_>::update_minimap` (in `game/extras.rs`) **HEAD, cancellable**: run the mod's
    budgeted bake + compose, write the 128x128 RGBA into `game.extras.minimap.image` (draw_extras needs it non-empty)
    and push a `Patch` at the `MINIMAP_SPRITE` atlas rect into `game.map_patches`; cancel the original. When the mod is
    off, fall through so Horizon's own minimap logic still runs.
  - `native_key` (zoom in/out, toggle), `native_hud` (info line), `native_mods` (title, keys), `native_settings`
    and `native_settings_click` (options page), `native_world_lifecycle` (clear tiles on world close).
- **Module** `runtime.rs` mounted in `crates/horizon-client/src/game.rs` as `crate::game::diorama` (can reach
  `Game`'s private fields): `static STATE: Mutex<State>` with tile cache `FxHashMap<(i32,i32), Tile>`, settings,
  zoom, last composed view. Helpers keep their bodies free of closures/`?`/macros where the hook kind requires it
  (HEAD is fine); real logic lives in the module.
- **Bake budget:** 0.3 ms CPU per frame (Horizon's `MINIMAP_BUDGET`), nearest chunks first, recompose only when the
  view or a tile changed. A tile = 16 px wide x (16*sin + height span*cos) px at 1 px/block, upsampled/downsampled
  in compose per zoom. Worker thread only if the budget proves too small (decide after M3 measurements).
- **Compose:** centre the image on the player's projected feet `(x, z*sin - y*cos)` so Horizon's centred arrow is
  right; unknown chunks dark grey (Horizon's `UNKNOWN`).
- **Files** (`/home/worc/Projects/diorama-horizon/`): `README.md` (credits + MIT notice + "unofficial"),
  `LICENSE` (Diorama's MIT text kept, plus port copyright), `NEXT.md`, `RULES.md` (constants/formulas from bytecode),
  `bundle/diorama/{horizon-mixin.toml, runtime.rs, projection.rs?, minimap.rs, update_minimap.rs, key.rs, hud.rs,
  mods.rs, settings.rs, settings_click.rs, lifecycle.rs}` (multi-file module only if horizon-mixins supports it;
  otherwise one `runtime.rs` with inline submodules), `dist/diorama-<ver>.hnmod`, `work/` (ignored: extracted classes).
- **.hnmod:** built with `python3 ~/horizon-mods/tools/package_hnmod.py bundle/diorama <mixins-commit 3f99a5c60dfa...>`;
  install to `~/.local/share/horizon/horizon-native-mods/diorama.hnmod`, line `mod.diorama:true` in
  `~/.local/share/horizon/horizon-mods.txt`.

## 4. Milestones (each verifiable)

- **M0 - Rules + skeleton (no build).** Extract classes to `work/`, `javap` `Projection`, `MapStyle`, `Settings`,
  `CaveSight`, `TileBaker` (face rules), `MinimapHud` (sizes, zoom steps); write `RULES.md` with numbers and UNKNOWNs.
  Scaffold repo, write `horizon-mixin.toml` with all injections and stub helpers. **Verify:** `horizon-mixins plan`
  dry-run succeeds (proves `update_minimap` is hookable with cancellable HEAD and bindings type-check by name).
- **M1 - Takeover proof.** Mod answers `native_minimap`, cancels `update_minimap`, writes a test pattern (gradient +
  player cross). Build once. **Verify:** `horizon-ctl launch --world T --creative --wait`, `screenshot`, ffmpeg crop
  of the corner shows the pattern in Horizon's frame; turning the mod off in Options > Mods restores nothing/no crash.
- **M2 - Projection + flat baker.** Top faces only, map colours, oblique placement, height shade; unit tests for
  projection and face visibility (pure Rust tests in a scratch crate, no game build). **Verify:** screenshot over a
  flat world with a placed tower (`horizon-ctl` commands to place blocks) shows the tower's top offset north/up.
- **M3 - South faces, water, z-buffer, budget.** Add side faces, water depth tint, incremental tiles, rescans on
  block change. **Verify:** screenshots of a hill and a lake; placing/breaking a block updates within ~1 s;
  `native_frame_rate` / frame-time log shows bake stays <= 0.3 ms per frame while flying.
- **M4 - Cave/Nether slice + zoom + keys + settings + info line.** **Verify:** `key <zoom> tap` screenshots at each
  zoom; teleport underground and into the Nether, screenshot shows the slice; settings survive a restart
  (files in `.horizon-mod-data/diorama/`); minimap hides under F1 and gives way to F3.
- **M5 - Polish + ship v1.** Perf pass, README/credits, package `.hnmod` to `dist/`, `git init` + commit (local
  unless the user says publish). **Verify:** clean build from the packaged `.hnmod` together with `perf_plus` and
  `xiaoxiang_core`; 10-minute play session without errors in the game log.
- **M6 (v1.1, optional)** - waypoints + death marker, after a separate go-ahead.

Test recipe (per skill): run horizon-ctl from the built `native-<commit>-<key>` version folder with
`HORIZON_HOME=~/.local/share/horizon/instances/native-mods`; `options-set pauseOnLostFocus false`; `screenshot` +
ffmpeg crop of the minimap corner; read images back to check.

## 5. Risks

1. **Internal target fragility:** `update_minimap` is private, not a stable hook; the mod must pin a commit and
   breaks on Horizon updates. Mitigation: keep the takeover to one small injection; later propose an upstream hook
   (`native_minimap_image(&mut Vec<u8>)`) to Horizon (MIT, ports may go upstream with credit).
2. **Fidelity gap:** without textures, shadows and occlusion rays it looks like an angled map-colour view, not
   Diorama's painted look; 128x128 texels limits detail. Users may judge it "not Diorama". Textured colours/cheap
   heightmap shadows are a v1.x step.
3. **CPU budget / frame time:** an oblique bake touches ~16x more blocks per tile than a top-down scan (side faces,
   occlusion behind hills). Risk of stutter at high zoom-out; may need a worker thread with chunk snapshots.
4. Pin mismatch: launcher's selected nightly is `2ebf00a`, but the cached native source and both existing native mods
   pin `fbd7288`; all mods of one build must pin the same commit.
5. Conflict with Horizon's own Minimap native mod (first `native_minimap` answer wins): don't run both.
6. horizon-mixins limits (closures/`?`/macros in OVERWRITE/RETURN targets, unique ids) - mitigated by HEAD-only design.

## 6. Open questions for the user

1. **Scope:** OK with v1 = angled minimap only (fullscreen map, explored-map persistence, waypoints GUI, radar,
   drawing, export deferred)? Include v1.1 waypoints now or later?
2. **Pinned commit:** pin `fbd7288` (matches perf_plus/xiaoxiang_core and the cached source) or move all three mods
   to `2ebf00a` (newer selected nightly; downloads new source and rebuilds everything)?
3. **Licence/publishing:** keep it local, or publish a repo? Either way README credits Dara Adedeji, keeps the MIT
   notice, says "unofficial port"; no Diorama assets are copied.
4. **Config format:** Horizon-native per-key settings files (recommended) vs. reading Diorama's own config file?
   (Only matters if you want settings imported from the Java mod.)
5. **Platform:** desktop only (native mods can't run on iOS). Fine?
6. **Colours:** accept map-item colours for v1, or should M3 also try average texture colours from the resource
   pack (`game.pack.assets`), adding ~1 session and risk?
7. **Performance budget:** 0.3 ms/frame (Horizon's minimap) acceptable, or allow more for a bigger/zoomed-out map?
8. **Defaults:** corner (top right?), size (e.g. 100 GUI px), round or square, default pitch (Diorama's default from
   `MapStyle`).

## 7. After approval: who does what

The builder does M0-M5 in order, stopping to flag anything that contradicts this plan (e.g. `update_minimap` not
hookable, `MINIMAP_SPRITE` rect missing, budget blown). Builds and game launches only when you say "build".

| Step | Kind | Builder |
|---|---|---|
| M0 class extraction, `javap` dumps, `RULES.md` numbers | mechanical bulk | Sonnet, reading delegated to `luna` (Codex) per your global rule |
| M0 repo scaffold, README/LICENSE, toml + stub helpers, dry-run | mechanical | Sonnet |
| M1 takeover injection + test pattern | judgment (first contact with internals) | Opus |
| M2-M3 baker, face visibility, z-buffer, budgeting, compose | judgment (core algorithm, perf) | Opus |
| M2 unit tests for projection/visibility | mechanical | Sonnet or `luna` |
| M4 settings page, keys, info line | mostly mechanical given M1-M3 | Sonnet |
| M4 cave/Nether slice rule | judgment | Opus |
| M5 packaging, install, screenshot runs, README | mechanical | Sonnet |
| Any build failure diagnosis, visual check of screenshots | judgment | Opus |
