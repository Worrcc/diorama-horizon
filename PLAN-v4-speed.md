# Diorama v0.3.2: faster map fill on iPhone (approved 2026-11-11, incl. step 4)

The phone runs mods on Pulley (an interpreter). Baseline (bench, Pulley, `~/Projects/iaf-swarm/dio-bench-base.txt`):
init 444 ms; join: box filled in 2.67 s, the whole 512 mosaic **not filled after 100 s**; frame avg 278-296 µs
(the 300 µs budget is spent every frame, even standing still).

Goal: the minimap fills faster, the fullscreen map opens with the minimap's area already drawn and fills the rest
fast, and closing the fullscreen map no longer makes the minimap re-read everything.

1. **Measure.** Bench scenarios for the fullscreen map: open after join (frames until the screen's area is
   filled), close (frames until the box is filled again). Log per-step costs (read, prepare, set, paint row,
   upload). Find why the full mosaic never finishes and why standing still uses the whole budget (re-read loop,
   too-dear steps, chunks never done?).
2. **Share chunk reads.** On open, the fullscreen mosaic takes the HUD mosaic's cached chunks (copies) so only
   painting is left for them. While it is open the HUD keeps its change tracking (`changes` since) and gets the
   chunks the fullscreen map read; closing no longer re-reads every HUD chunk (only changed ones; a resource
   reload still rebuilds everything).
3. **Catch-up budget (mod).** Request 500 µs (host max) while there is a backlog (pending/ready/painting/
   uploading) or the fullscreen map is open; 300 µs when idle.
4. **Host: bigger budget for a mod's open canvas screen (fork).** `request-frame-budget` may grant up to
   `SCREEN_BUDGET_MAX` (4000 µs) while that mod's canvas screen is open; falls back to `FRAME_BUDGET_MAX` (500)
   when it closes. Respect the iOS heat governor if it scales budgets. Behaviour change only, no WIT change;
   document it (docs/mods.md + WIT doc comment text). Fork branch `claude/mousetweaks-api`. Diorama asks 4000 µs
   while its fullscreen map is open.
5. **Fix what step 1 finds** (hot paths in read/set/paint on Pulley, wasted re-reads).
6. Bench before/after on Pulley; desktop live test (minimap, open/close fullscreen map, move); version 0.3.2.
