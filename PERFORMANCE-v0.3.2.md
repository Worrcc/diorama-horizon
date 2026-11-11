Diorama 0.3.2 — Pulley results, 2026-11-11

The bench runs the real `.hmod` in the linked Horizon runtime. The fork was not edited.
Its screen budget change arrived during this work; the final run received 4000 µs
with the fullscreen map open, 500 µs during HUD catch-up, and 300 µs when idle.

The original supplied baseline stopped joining after 6000 frames (100 simulated
seconds). To find completion rather than censor it, the bench now allows 24,000
frames (400 seconds). The before artifact has only fullscreen/close fill telemetry
added to the original mod; the scheduler and painting behavior are unchanged.
Both runs use the same final bench fixture and runtime. Times below are simulated
60 fps fill times; frame/init/callback costs are measured wall time on this machine.

| Measurement | Before, extended run | After |
|---|---:|---:|
| Join: HUD box fill | 163 frames / 2.72 s | 49 frames / 0.82 s |
| Join: full mosaic fill | 9378 frames / 156.30 s | 5022 frames / 83.70 s |
| Fullmap-open: visible fill | Not filled at 24,000 frames / 400 s | 180 frames / 3.00 s |
| Fullmap-close: HUD box refill | 178 frames / 2.97 s | 1 frame / 0.02 s |
| Stand: frame average after full fill | 29 µs | 23 µs |
| Fly: frame average / p99 | 291 / 430 µs | 472 / 1018 µs |
| Init | 548 ms | 649 ms |

Raw output: [before](bench/results/0.3.2-pulley-before.txt),
[after](bench/results/0.3.2-pulley-after.txt).
The original supplied run measured a 2.67 s box fill, 288 µs standing frames,
296 / 407 µs flight average/p99, and 444 ms init. Its standing sample was still
painting the unfinished join; the extended comparison above waits for completion
before measuring idle. These are single runs, not a claim about timing variance.

There is a deliberate flight cost increase with the 500 µs catch-up grant, and
p99 still exceeds that soft budget. Initialization also regressed in this run.
Opening's synchronous callback costs 52.9 ms versus 22.7 ms before, including the
temporary cache copy and new image allocation; closing costs 2.7 ms versus 7.2 ms.
The faster visible fill does not eliminate that opening hitch.

The final mod reports metered step averages separately from its conservative
smoothed scheduling estimates. Join averages were read 89.8 µs, prepare 105.7 µs,
set 155.2 µs, paint 129.3 µs per strip row, and upload 130.6 µs/KiB. Fullscreen
opening and normal closing made zero chunk reads; fullscreen painting averaged
6.6 µs per strip row and upload 38.2 µs/KiB because the prepared/shaded HUD cache
was available. The meter saturates at zero, so overrunning step measurements are
lower bounds, rather than independent wall-clock profiles.

Findings and fixes:

- The HUD had finite painting work, not an endless read loop. The extended run
  finished at 156.3 s. Dear first-time shading, one-row steps, and unnecessary
  painting of blank rows kept it busy past the old 100 s cutoff. Fresh resized
  and fully cleared buffers now paint only cached face spans, while still
  uploading their cleared pixels. Idle pending searches also return immediately.
- Fullscreen opening discarded all prepared HUD reads; closing forgot shades,
  reset `since` to zero, and queued every HUD chunk. Opening now copies prepared
  cache entries, both change cursors keep advancing, and closing transfers only
  completed fullscreen updates in the current HUD range. Pending changed chunks
  remain queued. Closing consumes the temporary mosaic, avoiding a third chunk
  copy that otherwise exceeded the host's 64 MiB limit after a resource reload.
- The upload estimator could request less than one strip row forever. A first
  upload now gets one row even when its estimate exceeds the allowance. Both
  upload paths share the 1 MiB frame cap, and row costs use the active mosaic's
  strip width rather than the two-texel constant.
- Paint cost learning counted requested rows rather than rows actually painted
  at dirty interval boundaries. It now counts completed rows, and step learning
  excludes scheduler selection overhead. Rates are rescaled on grant changes;
  detail selection retains its original time threshold across budgets.
- Fill completion could ignore a chunk still in `ready`. It now waits for that
  final commit. Layer/looks changes discard old prepared in-flight data. A looks
  rebuild while the fullscreen mosaic is installed also invalidates the paused
  HUD cache, preserving mandatory re-reads after resource reloads.
- Movement could leave distant pending reads alive after their cached neighbors
  were forgotten. Pending and ready work outside `FORGET` is discarded. Closing
  uses the current player position when transferring chunks; existing cache
  retention limits remain in place.
- Fullscreen rendering performed the cursor's vertical terrain scan before
  checking whether the screen changed. Unchanged frames now skip that scan.
- Hot path changes remove dirty-range allocations, shrink north-neighbor scratch
  storage from 256 to 16 columns, write horizontal texel runs through checked
  slices, and reuse exact dry top colours in a bounded 1024-entry cache. Release
  builds enable LTO across the core/WASM boundary. Colours are not approximated.

Verification passed: 123 core tests, root clippy with warnings denied, bench
clippy with warnings denied, offline `.hmod` build, all eight existing headless
mod scenario scripts (including fullscreen pan/zoom/close and minimap movement),
and the Pulley bench. `BENCH_VERIFY=1` changes a chunk while fullscreen is open,
then reloads pack colours while it is open. Both handoffs match fresh HUD bakes
byte for byte, with zero HUD re-reads on close. The reload case also verifies
that all 625 fullscreen chunks are re-read.

Reproduce with `CARGO_TARGET_DIR` set to this repository's `target-codex`, offline
Cargo enabled, and the artifact at `target/horizon-mod/diorama.hmod`:

```sh
cd bench
BENCH_VERIFY=1 cargo run --release --offline --features pulley -- ../target/horizon-mod/diorama.hmod
```

The telemetry-only before artifact is retained under
`target-codex/horizon-mod/diorama-before-instrumented.hmod` for local comparison.
Desktop live gameplay and physical iPhone testing remain pending. No commit or
push was made; the host change remains owned by the fork work.
