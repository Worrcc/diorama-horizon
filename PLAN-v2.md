# Diorama -> Horizon, plan v2: a WASM client mod that runs on desktop and iOS

Planner draft (Opus), 2026-11-08. Replaces PLAN.md's route (a desktop-only `.hnmod` that took over
`Game::update_minimap`); PLAN.md stays as the record of v1. Nothing has been built yet.

Source: Diorama (github.com/SunkenInTime/minecraft-orthogonal-map, local clone `/home/worc/Projects/diorama-src`,
HEAD `dec36d2`), MIT, (c) 2026 Dara Adedeji. This is an unofficial port.

---

## 0. What I found that changes the plan

1. **Client WASM mods already run on iOS.** Your premise was that the iOS fork runs only WASM *server* mods. That's
   not right. The fork's runtime (`crates/horizon-mods/src/runtime.rs`, `engine()`, about lines 585-600) runs **client**
   mods too, on Pulley (`config.target("pulley64")`, `signals_based_traps(false)`, `memory_reservation(0)`), and
   `ios/HANDOFF.md` records **Block Info** (`mods/official/block-info`, a client mod) working on the device.
   Client mods also already have a HUD API, in the experimental capabilities `render`, `ui` and `input`
   (`docs/api/mods-reference.md`, sections `render`, `ui`, `input`). Its functions: `render.set-hud`/`set-corner` (fill,
   quad, GUI sprite, item, text shapes), `gui-size`, `text-width`, `frame-stats`, `set-minimap` (Horizon's own
   map-item minimap), `ui.set-settings` (a page in Options > Mods), `ui.open-screen` (list and button widgets) and
   `input.register-key` (rebindable keys). The fork's official **Minimap** is already a WASM mod built this way
   (`mods/official/minimap/src/lib.rs`, 135 lines).
2. **What's missing is narrower than "a HUD API".** A WASM mod can't (a) draw its own pixels: 1024 fill shapes can't
   paint a 128x128 picture, and there's no image shape; (b) read the client world: `world` is for server mods only
   (`get-block` as a string per block, `docs/api/mods-reference.md` `world`), and `game.raycast` returns one block;
   (c) read block textures or a block-state table; (d) get a world id to keep waypoints per world; (e) get a frame
   budget bigger than the 0.1 ms target, or find out how much budget it has left. So "the client HUD API" in this plan
   adds two new interfaces, `canvas` and `client-world`, plus a few functions.
3. **Upstream Horizon deleted the WASM runtime.** At `2ebf00a` (the selected launcher build `nightly-2026-11-07-2ebf00a`),
   `crates/horizon-mods/src` has only `api_level, content, index, lib, manifest, modpack` (no `runtime.rs`),
   `crates/horizon-mod-sdk` is gone, and `docs/mods.md` lists just two mod kinds, content `.hmod` and native `.hnmod`.
   (Checked through `gh api` at that commit. The commits that removed it are 0c9a49390 and e7614d219, per memory
   `horizon-ios-upstream-merge`.) **So a WASM HUD API can't land in upstream.** It lands in the fork only. On desktop,
   the WASM mod runs in the fork's desktop build (`horizon-client --features mods`, which already builds on Linux), not
   in the launcher's upstream nightly.
4. **Upstream 2ebf00a already has native helpers for everything Diorama needs.** `docs/native-mods.md` at 2ebf00a lists:
   - `native_mod_image` (API 355): a mod's RGBA image in the 1024x1024 HUD atlas slot `horizon/modimage`.
   - Client world snapshots, `native_client_chunks` and `native_client_chunks_changed`
     (`game/native_client_world.rs`), plus `native_client_biomes` with per-column grass, foliage and water colours.
   - `native_block_states()`, `native_client_resources().texture(name)` and `native_block_sprites` (API 380), which
     give a resource-pack-aware block table and raw texture pixels ("a face's colour on a map ... averaged as the mod
     likes").
   - `native_client_state()` (screen, F3, world id) and `native_gui_render_post` (API 410).

   So a native desktop front-end on upstream no longer needs PLAN.md's fragile `update_minimap` takeover. I designed
   the new WASM interfaces to mirror these helpers, so the two front-ends can share one core crate (section 3).
5. **The 2ebf00a gap is large.** It is 2,263 commits ahead of `66f3981` (the fork's last merge,
   branch `claude/ios-horizon-66f3981`) and 14,327 ahead of `fbd7288` (the current pin of perf_plus and xiaoxiang_core).

### Feasibility verdicts

| Requirement | Verdict | Why |
|---|---|---|
| Runs on iOS | **Yes** | Client WASM mods already run on Pulley on the device (Block Info). Diorama becomes a client `.hmod`. |
| Client HUD API | **Yes, fork only** | It extends the fork's existing `render`/`ui`/`input`. Upstream has no WASM runtime to extend (finding 3). |
| World read from WASM | **Yes, with a new `client-world` interface** | The host has the client `World` (what the window shows, on servers too). The existing server `world` (one string per block) is too slow and server-side only. The new interface reads whole chunks as compact columns (section 2.3). |
| Average resource-pack colours | **Yes** | The mod gets **raw texture pixels** (`client-world.texture`) plus a block-state table naming each state's top and side sprite and its tint source. It averages them itself, with Diorama's `BlockLooks.average` rule (alpha >= 26, alpha-weighted sRGB mean, `bake/BlockLooks.java` 860-912), in `init` or spread over frames. Nothing is copied: the colours come from whatever pack the player has loaded. **Open check (M1):** whether the fork at 66f3981 keeps a CPU-side tree of decoded textures, as upstream's `client_resources` does. If not, the host computes per-sprite averages while it stitches the atlas. |
| 0.3 ms/frame bake on Pulley | **Feasible with a heightmap-column baker, not Diorama's ray tracer** | Pulley is an interpreter. Expect it to be 5-20x slower than Cranelift; measure in M7. A column baker costs about 50k WASM instructions per chunk, so on Pulley 0.3 ms is roughly 0.5-2 chunks per frame (30-120 per second): the visible map fills nearest-first in about 1-3 s, and the full mosaic in 3-8 s. Diorama's per-pixel ray tracer (`bake/TileBaker.java`: 4 samples per pixel, `SHADOW_STEPS = 96`, 13 sky rays, `SKY_REACH = 12`) is far over budget on Pulley. Shadows and occlusion get cheap heightmap stand-ins, or wait for v1.x. |
| Waypoints | **Yes, most of it** | Add/remove/list, the death marker, pins on the minimap and edge badges all fit the existing `ui`/`input`/`storage` plus `canvas`. Typing a name has no widget (`ui` has text, button, item and list widgets only), so naming goes through `/dwp add <name>` and `/dwp rename`. In-world pins (`WaypointHud`) need the camera: an optional `game.camera()` function, M6b. |
| Move perf_plus/xiaoxiang_core to 2ebf00a | **Yes** | Every injection target still exists by name at 2ebf00a (checked: `Renderer::render` `gfx.rs:7397`, `in_frustum` `gfx/resident.rs:367`, `ParticleEngine::tick/clear`, `BlockEntityRenderer::sign_text` `:2310`, `Streaming::receive_generated` `:1362`, `finish_consuming`, `Game::break_block` `game.rs:6545`, `entity_geometry` `:11631`, `native_tick/world_lifecycle/key/hud/command/mods`). Binding types may have drifted, which only `horizon-mixins plan` will show (section 6). |
| Publish on GitHub | **Yes** | `Worrcc/diorama-horizon` is free (checked). MIT allows it, keeping the notice. |

---

## 1. Scope of v1 (the WASM mod)

In v1:
- The angled minimap in the top right corner, about **10% of the screen width**, square or round.
- Oblique projection with Diorama's default pitch (50°). The view always faces north.
- Top faces and south faces, colours averaged from the resource pack, biome tints, face shading, height shade,
  outlines, clear-water depth tint, and Diorama's colour grade.
- The cave slice underground and the Nether roof slice (auto, `preferSurface`).
- Zoom steps, toggle and enlarge keys.
- Compass letters (fixed N/E/S/W, since the map doesn't turn), and the coordinates, biome and optional game time lines
  under the map.
- Settings in Horizon's per-key style.
- Waypoints: manual add/remove/rename/recolour/hide, a death marker (`DEATHS_KEPT = 3`), a bed marker if cheap, pins on
  the minimap with edge badges, and a waypoint list screen.

Not in v1 (needs more Horizon API, or isn't worth the budget on Pulley):
- The fullscreen map (`MapScreen`, `MapViewport`, `MapLevel`).
- The explored-map save, fog of war and PNG export (`TileStore`, `SavedChunks`, `ExportScreen`).
- Radar and mob faces, drawings, claims and FTB, Xaero interop and server rules, crafts.
- GPU baking, sun shadow rays and sky occlusion rays.
- Folders and icons, and chat `[Add]` links. `/tp` to a waypoint is a v1 button only if `command` is granted (Q4).

Optional in v1 (M6b): in-world waypoint pins, after `game.camera()`.

---

## 2. The Horizon API (fork only)

### 2.1 Where it lands

- **Base branch:** `claude/ios-horizon-66f3981` (worktree `~/Projects/iaf-swarm/ios-update`, the fork merged with
  upstream 66f3981, not yet pushed). It has `horizon-client-render/src/minimap.rs`, `game/extras.rs` (whose
  `minimap_wanted` already falls back to `mod_minimap()`), the native `native_mod_image` helper
  (`game/native_hooks.rs:8937`) and the atlas slot `MOD_IMAGE_SPRITE = "horizon/modimage"`
  (`horizon-client-render/src/hud.rs:147`, reserved at about line 856 only while a native mod is loaded).
  The branch you named, `claude/launcher-mods-shaders-controller-xfv3kk`, is older (before the merge). New work goes
  on a branch `claude/mod-canvas-api` cut from the merged branch. **Q1 confirms this.**
- **Upstream:** no WASM change is possible (finding 3). For upstream, the plan offers the optional native front-end
  (section 3.4) using the existing 2ebf00a helpers. Proposing anything upstream is out of scope.
- **Versioning:** the fork's own scheme. The manifest's `api = "0.1"` line stays (client mods). Every new function
  gets **API level 17** (the fork's `LEVEL` is 16, `crates/horizon-mods/src/api_level.rs:10`). A mod that imports them
  is refused on an older Horizon with "made for a newer Horizon (it uses canvas.set-corner): update Horizon"
  (`api_level::too_new`). The new capability is experimental, so the manifest lists it in `experimental = [...]`, as
  `render` already is.
- **Backwards compatibility (the rule that drives the design):**
  - **Existing WIT types are not changed.** Adding a case to `render.shape` or a field to `events.registration`
    changes the type every old mod imported, and wasmtime's import type check would then refuse all of them.
  - New things go in **new interfaces** (`canvas`, `client-world`) that reuse the old records through `use`, and in
    **new functions** on existing interfaces (`game.world-id`). Old components import a subset and still link.
  - No new manifest field either: `Manifest` is `#[serde(deny_unknown_fields)]` (`manifest.rs:33`), so an older Horizon
    would say "unknown field" instead of "update Horizon". The frame budget is requested by a function call instead.

### 2.2 `canvas` (capability `render`, experimental)

Draws the mod's own pixels on the HUD, in a corner box or anywhere.

```wit
interface canvas {
    use render.{corner, rect, point, fill-shape, quad-shape, sprite-shape, text-shape};
    type image = u32;
    /// A region of the shared mod-image atlas page, `width` x `height` texels (each side 1..=512; at most
    /// 1 MiB of texels per mod). Starts transparent. Err when the page is full.
    create-image: func(width: u32, height: u32) -> result<image, string>;
    /// Replaces texels `at`..`at+size` of the image with `rgba` (size.w*size.h*4 bytes, straight alpha).
    /// Uploaded before the next frame. At most 512 KiB per mod per frame: Err past it, and nothing queued.
    update-image: func(image: image, x: u32, y: u32, width: u32, height: u32, rgba: list<u8>) -> result<_, string>;
    drop-image: func(image: image);
    record image-shape {
        image: image,
        src: rect,          // texels of the image
        dst: rect,          // GUI pixels
        tint: u32,          // 0xAARRGGBB, 0xFFFFFFFF as is (opacity setting)
        round: bool,        // clipped to the circle inscribed in dst (a 48-triangle fan with matching UVs)
    }
    variant shape { image(image-shape), fill(fill-shape), quad(quad-shape), sprite(sprite-shape), text(text-shape) }
    /// As render.set-corner, with images: a width x height box placed in `corner`, stacked with the game's
    /// overlays, other mods' panels and the status effect rows; hidden with F1; the top left gives way to F3.
    set-corner: func(corner: corner, width: f32, height: f32, shapes: list<shape>);
    clear-corner: func();
    /// Asks for a per-frame CPU budget in microseconds for this mod's `frame` hook (default 100, at most 500).
    /// Returns what was granted. Shown on the Mods page.
    request-frame-budget: func(micros: u32) -> u32;
    /// Fuel left in the current hook call, scaled so the granted budget starts at about 1_000_000 units.
    /// A mod that loops "while work-left() > reserve" keeps to its budget without a clock.
    work-left: func() -> u64;
}
```

Host design:
- **Image page.** Reserve `horizon/modimage` (1024x1024, 4 MiB) when a native mod **or a running WASM mod that
  imports `canvas`** is loaded (`hud.rs`, the line 856 condition). Hand out sub-rectangles with a shelf allocator,
  per mod, freed when the mod stops. Uploads become `horizon_client::font::Patch`es in `Game::map_patches`, the same
  path `update_minimap` and `native_mod_image` use (`game/extras.rs`).
- **Drawing.** `HudBatch::rect_uv` for square images. For round ones, `HudBatch::quad` with UVs as a fan, the same
  batch calls `extras_hud::draw_minimap` uses (`crates/horizon-client/src/extras_hud.rs:59-86`). Placed with
  `extras_hud::Corners::place`.
- **Budget.** The mod has no clock, by design (`docs/mods.md`, "Design goals"). The host turns the granted µs into
  fuel per call with a feedback controller:
  - `fuel_next = clamp(fuel_now * budget / measured_time, 50k, FUEL_FRAME)`.
  - It is calibrated per platform automatically, since Pulley burns more time per fuel unit than Cranelift.
  - `work-left` is `store.get_fuel()` rescaled.
  - `FUEL_FRAME` (10 M, `runtime.rs:124`) stays the hard cap. Running out still stops the mod. The soft budget can't
    trap it, because the mod stops itself.
  - `Entry::cost` already tracks average call time, which is the data the Mods page shows.
- **Limits:** 1024 shapes per corner box (as `set-hud`), 1 MiB of images per mod, 512 KiB uploaded per frame, and
  budgets up to 500 µs. Over-limit calls return Err or are clipped. They never crash the game.

### 2.3 `client-world` (new capability `world-read`, experimental)

The player reads: "May see the blocks, biomes and light your game has loaded around you". It covers what F3 and any
minimap see on a server, and nothing the server didn't send. This is the "world-read" interface `docs/mods.md` already
lists under "Planned interfaces" ("blocks and biomes around the player ... minimaps, schematics").

```wit
interface client-world {
    record dimension-info { id: string, min-y: s32, height: u32, ceiling: bool }   // ceiling: the Nether's roof
    record face { sprite: u32, tint: tint-source }                                 // sprite indexes `sprites`
    enum tint-source { none, grass, foliage, dry-foliage, water, constant }
    record block-table {
        version: u64,                 // changes when resources reload (rebuild colours then)
        blocks: list<string>,         // "minecraft:grass_block", one per block
        sprites: list<string>,        // "block/grass_block_top", texture names
        state-block: list<u16>,       // per state id -> index into blocks
        state-flags: list<u16>,       // air, invisible, water, lava, waterlogged, glass-like, plant (no collision),
                                      // full-top, emission<<8
        state-top: list<face>, state-side: list<face>, // what the model draws up and south (first quad layer)
        constant-tint: list<u32>,     // for tint-source constant (lily pad, ...)
        map-color: list<u32>,         // MapColor as 0xRRGGBB, a fallback colour
    }
    record texture { width: u32, height: u32, rgba: list<u8> }   // first frame of an animated texture
    variant layer { surface, below(s32) }   // below(y): the floor under y, as Horizon's minimap Layer::Below
    record chunk-columns {
        revision: u64,          // the chunk's change counter; equal means unchanged
        tops: list<s16>,        // 256 columns, x fastest: the top block's y in this layer (i16::MIN: none)
        states: list<u32>,      // 256 x depth state ids, from the top down
        water: list<u8>,        // water depth over the floor, 0..=255
        light: list<u8>,        // sky<<4 | block light in front of the top face
        tints: list<u32>,       // 256 x 3: grass, foliage, water at the column (as drawn, unblended)
    }
    dimension: func() -> option<dimension-info>;
    block-table: func() -> block-table;                 // ~29k states; meant for init or after `version` changes
    texture: func(name: string) -> option<texture>;     // from the loaded packs, a higher pack over a lower one
    /// Chunks around the player, at most 16 a call, depth 1..=16 states per column. None for a chunk not loaded.
    columns: func(chunks: list<tuple<s32, s32>>, layer: layer, depth: u8) -> list<option<chunk-columns>>;
    /// Chunks whose revision moved since `since`, plus loaded and unloaded ones, within `radius` chunks of the
    /// player (at most 4096). Returns the current counter for the next call.
    changes: func(since: u64, radius: u32) -> tuple<u64, list<tuple<s32, s32>>>;
    light: func(x: s32, y: s32, z: s32) -> option<u8>;  // sky<<4|block, as Horizon's minimap checks it
    biome: func(x: s32, y: s32, z: s32) -> option<string>;
}
```

Additions to `game` (capability `game-read`):
- `world-id: func() -> option<string>`: `sp:<save folder>` or `mp:<address>`, as upstream's `NativeWorldId`.
- Optional (M6b): `camera: func() -> option<camera>` with position, yaw, pitch, fov and aspect, for in-world pins.

Host design:
- **A lent view.** `trait ClientWorldView` in `runtime.rs`, lent for the duration of client `tick`/`frame`/event
  calls the way `WorldView` is lent for server mods (`runtime.rs:455-500`, `Mods::lend_world`). The game implements it
  in `game/mods.rs` over `self.world`, `self.res.facts()` (`BlockFacts`: fluid, air, sturdy top, `map_color`, as
  `minimap::Palette::new` reads them, `horizon-client-render/src/minimap.rs:48`), the block sprite table
  (`game/native_block_sprites.rs`) and the loaded texture tree. Nothing is copied unless asked for. `columns` walks
  each column down from the section top, like `minimap.rs`'s scan does today.
- **Revisions.** A per-chunk counter, bumped wherever the client world writes a section or a chunk
  loads or unloads. Find the hook point the built-in minimap uses for "rescanned after block changes" (`minimap.rs`,
  `RESCAN`) and reuse it. This is a judgment step for M1.
- **Cost.** Host work counts against the mod's budget: the controller times the whole call, host functions included.
  `columns` is about 16 chunks x 256 columns x depth, natively, which is tens of µs.
- **Checks.** Indexes, counts and sizes are checked. A bad chunk list returns Err. It reads only the client world,
  so the same code serves single player, LAN and remote servers.

### 2.4 Files to change in the fork (branch `claude/mod-canvas-api`)

| File | Change |
|---|---|
| `crates/horizon-mod-sdk/wit/horizon.wit` | `interface canvas`, `interface client-world`, `game.world-id`. Add both interfaces to `world horizon-ui-mod` and to the host world `render-host` (or a new `canvas-host`/`world-read-host` pair, like `recipes-host`). |
| `crates/horizon-mod-sdk/src/` | Typed wrappers (`canvas`, `client_world`). No change to the `Mod` trait. |
| `crates/horizon-mods/src/manifest.rs` | `Capability::WorldRead` (`"world-read"` -> interface `client-world`, experimental, its player text). Interface `canvas` maps to `Capability::Render`. |
| `crates/horizon-mods/src/api_level.rs` | The new functions at level 17, and `LEVEL = 17`. |
| `crates/horizon-mods/src/runtime.rs` | Host impls: image allocator and upload queue, `request-frame-budget`, `work-left`, the per-mod fuel controller in `each_client`/`call` for `frame`, the `ClientWorldView` slot, `add_to_linker` for the new host worlds. A `force-pulley` cargo feature that widens the two `target_os = "ios"` cfgs, so Pulley can be tested on Linux (`ios/HANDOFF.md` describes doing this by hand). |
| `crates/horizon-mods/src/testing.rs`, `tests/sandbox.rs`, `tests/api_reference.rs` | Script columns and textures for `cargo horizon-mod test`. Sandbox tests: over-budget, over-quota and bad-index calls. Regenerate `docs/api/mods-reference.md` (`HORIZON_WRITE_DOCS=1`). |
| `crates/horizon-client/src/game/mods.rs` | Implement and lend `ClientWorldView`; drain image uploads into `map_patches`; time frame calls for the controller. |
| `crates/horizon-client/src/game/mod_render.rs` | Draw canvas corner boxes (image shapes, round fans) beside `draw_shapes`. |
| `crates/horizon-client-render/src/hud.rs` | Reserve the mod-image slot for canvas mods too. |
| `crates/horizon-client/src/game/extras.rs` | Stack the canvas boxes in `draw_extras` with the other corner panels (`Corners`). |
| `docs/mods.md`, `docs/api/mods.md` (changelog, "Not there yet"), `docs/api/stability.md` | Document level 17. |
| `.github/workflows/ios.yml` | Optional: build `diorama.hmod` beside `block_info.hmod` (lines 48-55). |
| `ios/` | No code change expected: Pulley and `--features mods` already exist (`ios/build.sh:65`). Run `ios/scripts/check-ios.sh` to type-check. |

About 1,200-1,800 lines of Horizon changes, including tests and docs.

---

## 3. The mod's architecture

### 3.1 Repo layout (`/home/worc/Projects/diorama-horizon`, published as `Worrcc/diorama-horizon`)

```
Cargo.toml                 workspace: core, wasm (and native/ later)
LICENSE                    MIT: Diorama's notice (c) 2026 Dara Adedeji, plus the port's (c) 2026 Worrcc
README.md                  unofficial port, credits, what's in or out, install for desktop (fork build) and iOS
RULES.md                   constants and formulas from the Java source, cited by file:line
PLAN.md, PLAN-v2.md, NEXT.md
core/  (crate diorama-core, no_std + alloc, no dependencies)
  src/projection.rs        Projection: u = x, v = z*sin - y*cos, depth t; pitch from the style
  src/style.rs             MapStyle DEFAULT/FLAT numbers, colour grade, SRGB<->linear tables
  src/looks.rs             BlockLooks subset: kind/tint per state, average(rgba) rule, tint application
  src/bake.rs              column baker: chunk columns -> per-texel colour + depth into a mosaic
  src/mosaic.rs            the 512x512 mosaic around the player, dirty rects, recentring
  src/cave.rs              auto layer (surface / below(head)), Nether roof, preferSurface rule
  src/settings.rs          per-key values, parse/format, defaults
  src/waypoints.rs         Waypoint, store (serialise), death/bed rules, layout and edge clamping
  tests/                   pure Rust tests (host target): projection, face visibility, averaging, layout
wasm/  (crate diorama, cdylib, horizon-mod-sdk)
  horizon-mod.toml         id diorama, side client, api 0.1, capabilities and experimental
  src/lib.rs               Mod impl: init/frame/tick/command/ui hooks -> core
  tests/*.toml             cargo horizon-mod test scripts
native/ (optional, section 3.4)
```

The core crate's tests run on the host with plain `cargo test`, with no game and no build of Horizon, so most of the
baker and waypoint logic is checked quickly.

### 3.2 What runs where (per frame and per tick)

- **`init`:**
  - Read settings (per-key storage).
  - `request-frame-budget(300)`, `create-image(512, 512)`.
  - `block-table()`. Compute averaged colours for each sprite (`texture` per distinct sprite, about 1,000 decodes, a
    few M fuel: in `init`'s 2 G, or spread over the first frames if Pulley is slow).
  - `register-key` for the keys; `set-settings`.
- **`tick` (20/s):**
  - Player, dimension and `world-id` change: swap the waypoint set, clear tiles.
  - Health dropped to 0: death marker at the last position.
  - Pick the layer (surface or below(head)); `changes(since, radius)` marks dirty chunks.
- **`frame`, `while work-left() > reserve`:**
  - Bake the nearest dirty chunk (`columns` for 1-4 chunks at a time, then bake into the mosaic).
  - Then recompose only dirty mosaic rects: `update-image` sub-rects.
  - Then `canvas.set-corner(TopRight, s, s + info, shapes)`, only when something moved: the image shape (src rect
    centred on the player's projected feet `(x, z*sin - y*cos)`, round per setting, opacity as tint), the player arrow
    (quad), compass letters (text), waypoint pins and edge badges (quad + text), and the info lines (text).
  - `s = round(0.10 * gui_width)` clamped to 48..224 GUI px (Diorama's `MIN_SIZE` 64 to `MAX_SIZE` 224; Q3).
- **Mosaic:** 512x512 texels at 2 texels per block (256 blocks across). It recentres by `memmove` plus one full upload
  when the player is within 48 blocks of an edge. Zoom only changes the src rect: 0.5, 1, 1.5, 2, 3, 4 GUI px per block,
  a subset of Diorama's `Settings.ZOOMS` that fits a 256-block mosaic.
- **Memory:** about 1 MiB mosaic, plus a depth buffer (512x512 i16, 0.5 MiB), plus about 5 KiB of column records per
  cached chunk (around 1,024 chunks is about 5 MiB). That's well under the 64 MiB cap (`runtime.rs:119`).

### 3.3 Java -> Rust mapping for v1

| Diorama (Java, cited) | v1 Rust (core/wasm) | Notes |
|---|---|---|
| `Projection.java` (u, v, `zAt`, TILE_UNITS 128, PIXELS_PER_UNIT 4) | `projection.rs` | North-facing only (`Facing.NORTH`). Turned maps come later. |
| `MapStyle.java` `DEFAULT` (pitch 50, saturation 1.35, exposure 1.16, sun/shadow tints, heightShade 0.22, outline 0.38, clearWater) | `style.rs` | Copy only the numbers. `skyOcclusion`, `sunElevation` and `cornerOcclusion` get cheap stand-ins or are dropped (RULES.md says which). |
| `bake/TileBaker.java` (SIDE 0.82, OUTLINE_DEPTH 1.5, CAVE_DEPTH 48, CAVE_FADE 12, CAVE_FADE_START 6, CAVE_WALL_FADE 4, LID_* , HAZE_*) | `bake.rs` | **Rewritten** as a column painter with a z-buffer on depth t, not a per-pixel ray trace. Top face 2x2 texels. South face from the top down to the south neighbour's top, coloured per state from `states[]`. Overhangs (leaves over air) are drawn as columns: a documented look difference. |
| `bake/GpuMesher`, `GpuTileBaker`, `render/gpu/*`, `*.glsl` | none | No GPU access for mods. |
| `bake/BlockLooks.java` (kinds AIR/SOLID/WATER/LAVA/GLASS/PLANT, TINT_*, top/side + tinted parts, `average` 860-912) | `looks.rs` | From `client-world.block-table` + `texture`. Averaging done in WASM. |
| `bake/BiomeTints.java`, `TintProbe` | `looks.rs` tint step | Uses the `tints` per column (unblended; Diorama blends, a small edge difference). |
| `bake/Lighting.java`, `RockRays`, `light/Lights` | `bake.rs` shading | Face factors, height shade, block light glow in caves (`LID_GLOW`). Optional heightmap shadow along `SUN_AZIMUTH` (cheap, M5 stretch). |
| `CaveSight.java`, `MapSession.autoLayer` (`preferSurface`, `MIN_CAVE_DEPTH`) | `cave.rs` | Layer from `light(head)` and depth to sky. No seen-blocks flood fill (60k-block spread is too slow on Pulley); the slice shows the floor under the head. |
| `MinimapHud.java` (MARGIN 6, the effects offset, the compass, info lines, round/square, opacity, layer fade 300 ms) | `wasm/lib.rs` + `canvas.set-corner` | Corner stacking and the effects offset come from the host (`Corners`). The fade between slices is dropped in v1. |
| `render/MinimapFrames.java` + `assets/.../textures` frames | none | Diorama's frame PNGs are never copied. A plain 1 px border drawn with fill shapes. |
| `render/Markers.java` (player arrow) | quad shape | Own geometry. |
| `Settings.java` (minimap*, compass, coordinates, biome, gameTime, autoCaves, preferSurface, waypoints*, deathMarkers) | `settings.rs` | Per-key storage, section 4. |
| `DioramaClient.java` keys (B new waypoint, U list, = / - zoom, toggle, enlarge unbound) | `input.register-key` | Same defaults. `key.diorama.*` names. M (fullscreen map) left out. |
| `waypoint/Waypoint.java` (Kind MANUAL/DEATH/BED, PALETTE, initials, MAX_INITIALS 4) | `waypoints.rs` | Kinds TEMPORARY/WAYSTONE/portals left out. |
| `waypoint/WaypointStore.java` (per world JSON, DEATHS_KEPT 3) | storage key `wp-<hash>` per `world-id` | Same fields, Diorama-compatible JSON where cheap. |
| `waypoint/Waypoints.java` (`atPlayer`, `died`, bed) | `/dwp add [name]`, key B, the death rule in `tick` | Bed only if a sleeping signal exists (Q5). |
| `waypoint/WaypointOverlay.java` (pins, edge badges with carets) | quad + text shapes | Layout and clamping in `waypoints.rs`. |
| `waypoint/WaypointListScreen.java` | `ui.open-screen`: a list widget (name, distance, dim) plus buttons Hide/Show, Colour, Delete, Teleport (if `command`), Close | No text field, no folders. |
| `waypoint/WaypointHud.java` | optional M6b with `game.camera()` | |

### 3.4 Optional: a native front-end for upstream desktop (not in v1 unless Q2 says so)

`native/` would be a horizon-mixins bundle pinned to 2ebf00a with `"hooks": N` (no internal targets). `diorama-core`
goes in as the bundle's own `[[crate]]`: it has no dependencies, which a mod crate needs, per `docs/native-mods.md` at
2ebf00a, "The mod's crate". The glue maps:
- `native_client_chunks` -> columns
- `native_block_states` + `native_client_resources().texture` -> the block table
- `native_mod_image` + `native_sprite` -> the canvas
- `native_client_state().world` -> world id
- `native_key`, `native_hud`, `native_mods` and `native_settings` -> the rest

About 400-600 lines and one native build at 2ebf00a, shared with perf_plus and xiaoxiang_core. This is the only way to
see Diorama in the launcher's own upstream build.

---

## 4. Settings in Horizon's per-key style

The same pattern as the official Minimap mod (`mods/official/minimap/src/lib.rs`):
- One `storage` key per setting, as plain text (`corner = top_right`, `zoom = 2`).
- Read in `init`, with unknown values falling back to the default.
- `ui.set-settings` shows one cycling button per setting, labelled "Name: Value".
- `ui_select` advances the value, saves, applies and refreshes the page; `ui_settings_opened` refreshes the page.
- Keys show as Key Binds rows on the same page automatically (`input.register-key`, `docs/mods.md` `input`).

| Key | Values (default first) | From Diorama |
|---|---|---|
| `minimap` | `true`/`false` | `Settings.minimap` |
| `corner` | `top_right`, `top_left`, `bottom_left`, `bottom_right` | `minimapCorner` |
| `size` | `10` (% of screen width), `8`, `12`, `15`, `20` | the requested 10%; Diorama's default is 96 px |
| `shape` | `square`, `round` | `minimapShape` |
| `zoom` | `2`, `3`, `4`, `0.5`, `1`, `1.5` (px per block) | `minimapPixelsPerBlock` = 2 |
| `opacity` | `100`, `80`, `60`, `40` | `minimapOpacity` |
| `auto_caves`, `prefer_surface` | `true` | `autoCaves`, `preferSurface` |
| `compass`, `coordinates`, `biome` | `true` | same |
| `game_time` | `off`, `clock` | `gameTime` |
| `style` | `default`, `flat` | `MapStyle.DEFAULT/FLAT` |
| `waypoints_on_map`, `death_markers` | `true` | same |
| `waypoint_distance` | `0` (any), 64...2048 | `WAYPOINT_DISTANCES` |

That's 16 keys. The waypoint files take one key per world, inside the storage limit of 64 files and 4 MiB
(`docs/mods.md` `storage`), so about 45 worlds. Past that, the oldest world's waypoints are dropped, with a chat note.

---

## 5. Milestones

Each milestone ends with a verification. **O** = judgment work for Opus; **S** = mechanical work for Sonnet, or for
Luna (Codex) per your CLAUDE.md delegation rule. Building and pushing happen only after you approve this plan.
Pushing the fork, which starts the iOS CI, needs a separate yes each time.

- **M0 - Rules + scaffold (S, 0.5 session).**
  - Write `RULES.md` from the Java files cited in section 3.3, numbers with file:line, plus a list of what is dropped.
  - Create the workspace with `core` and `wasm` crates (stubs) and the LICENSE/README skeleton.
  - **Verify:** `cargo test -p diorama-core` (stubs) and `cargo build --target wasm32-unknown-unknown -p diorama`
    compile.
- **M1 - The Horizon API skeleton (O, 2 sessions).**
  - Do the fork changes in section 2.4. The open checks are settled here: the texture source in the fork, the chunk
    revision hook point, and the drawing order with other corner panels.
  - Add an SDK example `examples/canvas-test`: a 64x64 gradient image in a round corner box, plus a `columns` read
    that prints the top block under the player.
  - **Verify:** `cargo test -p horizon-mods --features runtime,pack` (sandbox + api_reference); `cargo horizon-mod
    test` on the example; `--features force-pulley` release sandbox test (budget controller converges to ±20% of
    300 µs); `ios/scripts/check-ios.sh` type-check.
- **M2 - A flat test pattern on desktop and iOS (S, 0.5-1 session).**
  - Diorama `wasm` draws a projected checkerboard plus the arrow at 10% width, top right.
  - **Verify (desktop):** fork Linux build `--features mods`, `horizon-ctl launch --offscreen --dev-mod diorama.hmod`,
    then a screenshot plus an ffmpeg crop of the corner.
  - **Verify (iOS):** push the branch (**ask first**), the iOS CI builds the IPA plus `diorama.hmod`; you install it
    and send a screenshot or `latest.log`.
- **M3 - Projection + column baker (O, 1.5 sessions).**
  - Build top and south faces, the z-buffer, mosaic, dirty rects, `changes` polling, and recentring.
  - Core unit tests: projection maths against the Java formulas, face visibility on hand-made columns (a step, a
    tower, an overhang), and the depth order.
  - **Verify:** a screenshot in a test world with a placed tower and a staircase. Place a block and the map updates
    within 1 s.
- **M4 - Colours from the resource pack (O, 1 session).**
  - Block table, texture averaging (Diorama's rule), biome tints, water depth tint, colour grade, height shade,
    outlines.
  - Core test: averaging a known 16x16 RGBA gives the Java result (worked by hand).
  - **Verify:** screenshots of plains, forest, ocean and desert. Switching to a different resource pack changes the map
    colours after the reload (the `version` changes). A grep of the repo shows no PNGs and no colour tables copied from
    Diorama or Mojang.
- **M5 - Caves, Nether, zoom, keys, settings, info lines (S with O review, 1 session).**
  - **Verify:** zoom key screenshots at each step; underground and Nether teleports show the slice; settings survive a
    restart (`horizon-mods/.horizon-mod-data/diorama/` keys); F1 hides the map; F3 frees the top left.
- **M6 - Waypoints (O for layout, S for screens/commands, 1.5-2 sessions).**
  - `/dwp add|del|rename|color|list`, key B/U, the death marker, pins and edge badges, the list screen, per-world
    storage.
  - Core tests for layout and clamping, and for serialising.
  - **Verify:** add 3 waypoints, fly away and check the edge badges point right; die and check the death pin; leave
    and rejoin to check they persist; open another world to check they are kept separate.
  - **M6b (optional):** `game.camera()` and in-world pins.
- **M7 - Performance (O, 0.5-1 session).**
  - Log the frame-call time per frame (the Mods page cost, `Entry::cost`), desktop Cranelift and `force-pulley`,
    flying at sprint speed for 60 s.
  - **Verify:** the bake stays within 0.3 ms per frame amortized, and no frame goes over 0.5 ms. Fill-in time for the
    visible map is under 3 s on Pulley. An iOS device run with an FPS reading next to Block Info. If Pulley misses:
    halve the texels per block (1 per block), shrink the mosaic, or decide whether a worker-thread mod call is worth a
    v1.x API.
- **M8 - Publish (S, 0.5 session).**
  - Section 7. **Verify:** a fresh clone builds `diorama.hmod` with the documented commands; README links work; the
    repo contains no Diorama assets.

---

## 6. Moving perf_plus and xiaoxiang_core to 2ebf00a

- **Change:** in each `~/horizon-mods/<mod>/horizon-mixin.toml`, set `[compatibility] horizon_revision =
  "2ebf00a0281c592f79191e76fce0e7d522c6f425"`. Leave the mixins pin as is, unless the launcher's horizon-mixins at
  2ebf00a needs a newer one: the build uses the newest of the pins (`docs/native-mods.md` at 2ebf00a, "Pinned to a
  Nightly").
- **Check before building:** run `horizon-mixins plan` (dry run) for each bundle against the 2ebf00a source. That
  catches drifted binding types. Every target exists by name (section 0), but the bindings' types are spelled out in
  the toml (for example `&mut Vec<crate::game::native_hooks::NativeMod>`).
- **Likely edits:**
  - perf_plus hooks render internals (`Renderer::render`, `in_frustum`, `ParticleEngine`, `sign_text`). In 14k
    commits their arguments probably changed. Expect 1-3 binding fixes.
  - xiaoxiang_core mostly uses named `native_*` hook points, which are stable. The risk is in `finish_consuming`,
    `break_block` and `Streaming::receive_generated`.
- **Rebuild cost:**
  - The launcher fetches the 2ebf00a source (the cache has only `fbd7288`).
  - One full native build at a new commit: estimated 15-30 min. `native/target` is shared, so the dependencies are
    partly reused, but the workspace crates compile fully. After that, about 3-5 min per edit (the last incremental
    build took 2 m 39 s, `native/build.log`).
  - Then a smoke test of each mod, about 10 min.
  - In total: about 0.5-1 session, mostly waiting.
- **Order:** do this before anything else that builds natively (the optional native Diorama front-end), so all
  native bundles share the one pin.

---

## 7. Publishing

- **Repo:** `Worrcc/diorama-horizon`, public, after M2 at the earliest (**Q6**). Push only on your yes.
- **Licence:**
  - `LICENSE` holds the MIT text with both lines: `Copyright (c) 2026 Dara Adedeji` (Diorama's notice, kept verbatim)
    and `Copyright (c) 2026 Worrcc (Horizon port)`.
  - The Horizon SDK is a dependency under the fork's own licence: check that `LICENSE` and `THIRD_PARTY_NOTICES.md`
    in the fork don't need a notice in the mod repo.
- **README:**
  - The first line says "**Unofficial** port of Diorama by Dara Adedeji (github.com/SunkenInTime/minecraft-orthogonal-map,
    Modrinth `diorama-map`) to Horizon. Not affiliated with or endorsed by the Diorama author or Mojang."
  - Then what's in and out (section 1), the requirements (the Horizon-iOS fork build with API level 17; it does not
    run in upstream Horizon nightlies), install steps for desktop and iOS, and credits.
- **Excluded:** every file under Diorama's `src/main/resources/assets/` (icon, lang, shaders, frame and icon
  textures), `docs/minimap-frames`, `docs/radar-icons`, Java sources, and any Mojang texture.
  - `.gitignore` covers `work/`, `target/` and `dist/*.hmod` (release assets go on GitHub Releases).
  - Check before publishing: `git ls-files | grep -Ei '\.(png|ogg|json|glsl|fsh|vsh)$'` must print nothing (or only
    our own `tests/*.toml` fixtures).
  - Colours: the code holds **no colour tables**. Every block colour is computed at run time from the player's loaded
    packs. The only fixed colours are Diorama's style constants (grade, tints) and the waypoint `PALETTE`, which are
    numbers in MIT code, credited in RULES.md.
- **Fork side:** the API changes go to `Worrcc/Horizon-iOS-worc` as a branch, merged into the iOS line on your call.

---

## 8. Risks

1. **Pulley speed** is the biggest risk. If Pulley is at the slow end (about 20x), 0.3 ms buys about half a chunk per
   frame, and the map fills slowly while flying.
   - Mitigations, in order: 1 texel per block, a smaller mosaic, baking only the visible map rect first, and
     reading the most compact `columns` (depth 4).
   - Last resort: a v1.x "background call" API (a second Store on a worker thread). That is a large runtime change.
2. **The fork drifts from upstream.** The new host code sits on the 66f3981 fork. A later merge to 2ebf00a or newer
   (2,263+ commits) will touch `game/mods.rs`, `mod_render.rs` and the hud.
   - Mitigation: keep the host code in a few new files (`game/mod_canvas.rs`, `game/mod_client_world.rs`) behind small
     adapters. After that merge, swap their internals for upstream's `native_client_world`/`block_states`/
     `client_resources` helpers.
3. **The look differs from Diorama.** The column painter has no sun shadows, no sky occlusion and no textured faces, and
   overhangs are drawn as columns. It will look like "Diorama-lite", not Diorama's painted map.
4. **iOS input.** Touch controls have no mod-key buttons (no mod keys in `game/touch.rs`, `touch_controls.rs`). On a
   phone without a keyboard or controller, zoom, add-waypoint and the list need the settings page or `/dwp` in chat.
5. **The texture source in the fork.** It is uncertain whether 66f3981 keeps decoded textures on the CPU. If not, M1 adds
   averaging while the atlas is stitched (the same result; the host does it).
6. **Storage quota.** 64 files per mod limits per-world waypoint files to about 45 worlds.
7. **Native pin move.** Bindings may break in perf_plus after 14k commits. That costs a few rebuilds of 3-5 min each.

## 9. Open questions (please answer before building)

1. **Base branch:** build the API on `claude/ios-horizon-66f3981` (merged with upstream, not pushed yet) rather than
   `claude/launcher-mods-shaders-controller-xfv3kk`? Recommended: yes.
2. **Desktop route:** is the fork's desktop build enough for desktop (the WASM mod everywhere), or do you also want the
   native front-end (section 3.4) so Diorama runs in the launcher's upstream 2ebf00a build? That adds about 1-1.5
   sessions.
3. **Size:** a strict 10% of screen width gives about 43-48 GUI px on a 1080p desktop at GUI scale 4, smaller than
   Diorama's 64 px minimum. Keep 10% with a 48 px floor, or use Diorama's 96 px default on desktop?
4. **Teleport button:** grant capability `command` so the waypoint list can run `/tp` (shown to the player as "run
   commands as you")? Or leave it out?
5. **Bed marker:** it needs a "sleeping at" signal that client mods don't have. Add `game.sleeping-pos()` (one more
   level-17 function), or skip the bed marker in v1?
6. **When to make the repo public:** after M2 (it works on both platforms), or only after M8?

## 10. Effort

| Part | Sessions |
|---|---|
| M0 rules + scaffold | 0.5 |
| M1 Horizon API (fork) | 2 |
| M2 test pattern, desktop + iOS | 0.5-1 |
| M3 projection + baker | 1.5 |
| M4 colours | 1 |
| M5 caves/zoom/keys/settings | 1 |
| M6 waypoints (+M6b optional) | 1.5-2 (+0.5) |
| M7 performance | 0.5-1 |
| M8 publish | 0.5 |
| Pin move perf_plus + xiaoxiang_core | 0.5-1 |
| **Total v1** | **about 9.5-11.5 sessions** |
| Optional native front-end for upstream (Q2) | +1-1.5 |
