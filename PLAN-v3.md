# Diorama -> Horizon, v3: fullscreen map + movable minimap (planner draft, awaiting approval)

State going in: M8 published (7a3825b). Minimap, caves/Nether slice, zoom, settings, waypoints done. The mod is a
WASM client mod on the Horizon-iOS fork, branch `claude/mod-canvas-api` (worktree
`~/Projects/iaf-swarm/mod-canvas-api`, API level 17, SDK pinned at 5082b61b0).

## 1. Why the fork's API has to change first

What a mod can do today (horizon.wit, level 17):
- `ui::open-screen` is widget-only (text, button, item, list stacked down the middle). It can't draw images and gets
  no mouse input.
- `canvas::set-corner` draws images, but only in a HUD box the game stacks into one of 4 corners. It sits under
  screens, so it's not visible while a screen is open, and it takes no offset.
- `input` gives keys (only with no screen open) and the wheel. No pointer events.
- Images: at most 512 x 512 texels per mod. The minimap mosaic already uses all of it (`SIDE = 512`).

So a fullscreen map and free positioning both need new host functions. Everything below is additive at **API level
18**, so level-17 mods keep working.

## 2. Fork API additions (level 18)

In `crates/horizon-mod-sdk/wit/horizon.wit`, `interface canvas`:

1. `set-placed: func(anchor: corner, dx: f32, dy: f32, width: f32, height: f32, shapes: list<shape>)`
   Like `set-corner`, but the box goes at an offset from the anchor corner in GUI px, clamped on screen, and is left
   out of the corner stacking. `set-corner` and `clear-corner` keep working, and either call replaces the other.
2. `open-canvas-screen: func(title: string, shapes: list<shape>)`, `set-canvas-screen: func(shapes: list<shape>)`,
   `close-canvas-screen: func()`
   A full-window screen the mod owns. Shapes are in GUI px from the window's top left, up to 4096. Esc closes it
   (`ui-events::closed`). It pauses single player like other menus. The HUD canvas box is hidden under it.
   `frame` calls keep coming, so the bake keeps going.
3. Per-mod texel budget raised to 1024 x 1024 total (one page cell grows; the page is 4 MiB today, so check it
   fits or grow the page). This gives the full map its own image instead of fighting the minimap for texels.
   **Open question 2**: drop this if you want a smaller fork change.

In `interface ui-events` (for mods that open a canvas screen):

4. `pointer: func(kind: pointer-kind, x: f32, y: f32, button: u32)` with `pointer-kind = down | up | drag | move`,
   and `scroll: func(x: f32, y: f32, steps: f32)`, plus `screen-key: func(key: string)` (the `key.keyboard.*`
   name) for keys while the canvas screen is open. On iOS, touch maps to pointer (one finger = drag, pinch = scroll
   steps) through the fork's existing touch-to-mouse path.

Host side: `runtime/canvas_host.rs` (new calls, limits, level 18 in `api_level.rs` FUNCTIONS),
`game/mod_canvas.rs` (placed box, screen drawing), a new `game/mod_canvas_screen.rs` (screen state, event routing),
`docs/mods.md`, plus tests in `horizon-mods/tests/` (`api_reference`, `sandbox`, a `canvas_guest` screen case).
SDK `src/lib.rs` re-exports the new calls. `cargo-horizon-mod`'s scenario runner gets `pointer`/`scroll` steps so
the mod can be tested headless.

## 3. Mod changes (diorama-horizon)

### 3a. Movable minimap
- Settings: keep **Corner** as the anchor, add **Offset X** / **Offset Y** (stored GUI px, default 0), plus a
  **Move minimap...** button on the settings page.
- "Move minimap..." opens a canvas screen that shows the live minimap where it sits plus a dashed outline. Drag it
  with the mouse or finger. Arrow keys nudge 1 px (Shift: 10). **R** resets. Esc/Done saves. It snaps to the
  nearest corner as the anchor (so it stays put across window sizes) and stores the offset from there.
- The drawing switches from `canvas::set-corner` to `set-placed` when the offset isn't 0 (and on level 18 generally).
  With offset 0 it keeps corner stacking so it doesn't overlap status effects.
- Core: `layout.rs` gets `place(anchor, offset, box, gui) -> rect` with clamping, plus `snap(rect, gui) ->
  (anchor, offset)`, both unit tested.

### 3b. Fullscreen map (Diorama's `MapScreen`, reduced)
- Key **M** (Diorama's default; it was left out in v2) opens it. It's also a button on the waypoint list screen.
- Image: its own 1024 x 1024 (or 512 if Q2 says no) mosaic built by the same `mosaic`/`bake`/`paint` code,
  centred where the map was opened. It covers the loaded chunks at 1-2 texels/block, plus everything baked since
  the screen opened. Until the screen closes, the minimap's mosaic is paused, so there's one bake budget.
- View: pan by dragging, zoom with the wheel/pinch (0.25 to 8 GUI px per block, anchored at the cursor), **C** or
  a corner button to re-centre on the player. Unknown area is dark grey, as on the minimap.
- Overlays: player arrow, waypoint pins with names (hidden ones skipped), death markers, a coordinate + biome line
  for the block under the cursor (inverse projection `zAt` from RULES.md section 1 plus the column's surface y),
  a north compass, and a legend line with the keys.
- Actions: right-click/long-press on the map gives **Add waypoint here** (y = the column's surface). Click a pin
  for Teleport / Delete (same rules as the list screen: teleport only with `command`). A **Waypoints** button
  opens the existing list screen.
- Cave/Nether: the map uses the same slice rule as the minimap at the moment it opened.
- Core: new `core/src/fullmap.rs` (view state, pan/zoom maths, screen <-> world mapping, hit-testing pins), unit
  tested. The WASM side is `wasm/src/fullmap.rs` (screen open/close, events, shapes).

### 3c. Out of scope (still deferred)
Explored-map saving across sessions and fog of war (needs storage of baked tiles; a v4 candidate), PNG export,
drawings/measure, radar, claims.

## 4. Order of work and who builds it

| Step | Repo | What | Builder |
|---|---|---|---|
| A1 | fork | WIT + SDK + host for `set-placed`, level 18, tests | Codex `sol` (judgment in host layout code), Opus review |
| A2 | fork | canvas screen + pointer/scroll/screen-key events, tests, docs | Codex `sol`, Opus review |
| A3 | fork | texel budget 1024^2 (if Q2 yes), scenario runner pointer steps | Codex `luna` |
| A4 | fork | Linux build `--features mods`, run `canvas-test` example and tests | me |
| B1 | mod | layout place/snap + settings + move screen | Codex `luna` (spec is exact), Opus review |
| B2 | mod | `fullmap.rs` core + tests | Codex `sol` |
| B3 | mod | WASM screen, overlays, actions, M key | Codex `sol`, Opus review |
| B4 | mod | scenario tests + desktop screenshots (map opened, panned, zoomed, waypoint added; minimap moved to 4 spots and mid-screen) | me |
| B5 | both | bump SDK pin to the new fork commit, README/RULES section for v3, commit locally | me |

Verification per step: `cargo test` (fork's `horizon-mods`, mod's `diorama-core`), `cargo horizon-mod test`, and
desktop screenshots via horizon-ctl in a scratch HORIZON_HOME (recipe in memory `horizon-ios-port`). iOS needs a
pushed branch for the CI IPA; that push is yours to run (no GitHub auth on this machine).

## 5. Risks
1. **Fork drift:** level 18 lives only on `claude/mod-canvas-api`. A later upstream merge has to carry it. Keeping
   the Horizon-specific drawing in `mod_canvas*.rs` keeps that small.
2. **Screen pause vs bake:** in single player a paused game still draws frames, but `frame` hooks may be gated on
   "not paused". This needs checking in A2. If they are gated, the canvas screen opts out of pausing.
3. **Atlas page size:** 1024^2 texels per mod might not fit the shared 4 MiB mod-image page next to native mods.
   Fallback is a 512 map image at 1 texel/block, which covers about 32 chunks across.
4. **Touch:** pinch-to-scroll may not exist in the fork's touch layer. Fallback is +/- buttons on the map screen.
5. **Cost:** around 1,500-2,200 lines across both repos, with ~4-6 game builds.

## 6. Open questions
1. Movable minimap: is drag-to-place on a "Move minimap" screen plus Offset X/Y buttons what you want, or only
   offset buttons (smaller: no screen needed, so 3a could ship before the fork's screen API)?
2. Raise the per-mod texel budget to 1024^2 for a sharper and larger full map, or keep 512^2?
3. Full map coverage: is loaded/seen-this-session OK for now, with explored-map saving as v4?
