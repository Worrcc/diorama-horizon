# Diorama 0.4: load even faster on iPhone (approved 2026-11-11: all three steps, storage up to 1 GB)

Pulley per-chunk costs at 0.3.2: read ~110 µs, prepare ~130 µs, set ~155-190 µs, paint ~130 µs per strip row,
upload ~130 µs/KiB; budget 300/500/4000 µs. Bench results: bench/results/0.3.2-pulley-after.txt.

## Host (fork `claude/mousetweaks-api`, worktree ~/Projects/iaf-swarm/mousetweaks-api, API level 23)
H1. **Per-mod storage quota.** `horizon-mod.toml` may declare `storage-mb = N` (1..=1024; default and
    minimum behaviour = today's 4 MiB / 64 keys). With a declared quota: total bytes N MiB, keys up to 65536
    (`[A-Za-z0-9._-]` keys as today), same 1 MiB written per hook call. The check output / install prompt says
    "may save up to N MB to disk". Listing/deleting stays cheap (no full directory scan per `set` on big stores:
    keep a running total). Diorama declares 1024.
H2. **Native chunk prepare** (`client-world`, level 23). The mod registers a per-block-state class table once
    (and again when its looks rebuild): for each state, "see-through" (Diorama: Plant | Glass | Air) or solid.
    A new call returns chunks like `columns` but already prepared as Diorama's `Painter::prepare` does: for each
    column, skip the leading see-through blocks within `depth` (k = first solid index; k == 0 or none found:
    unchanged), shift the column's states up by k (pad with the last state), lower the top by k, and return the
    skipped states (only for columns with k > 0) so the mod can compute their overlay colours. Also look at the
    read cost itself (list lifting into guest memory): pack fixed-size arrays as `list<u8>`/`list<u32>` blobs if
    that is cheaper. Older mods keep using `columns`.

## Mod (~/Projects/diorama-horizon)
M1. **Cheaper uploads:** keep texels in upload-ready RGBA bytes and send whole-width strips (contiguous), so the
    host copies natively; no per-texel loop in the guest at upload time.
M2. **Saved explored map:** tiles of baked texels (fixed world-aligned tiles, e.g. 256x256 texels at the
    minimap's texels-per-block, per world id (`game.world-id`) + dimension (+ cave layer if applicable)),
    compressed cheaply (RLE or similar; cost on Pulley matters), written a little per frame (<= 1 MiB per hook) as
    tiles change and on leaving; LRU-evicted under the declared quota (1 GiB, keep a safety margin). On join: show
    saved tiles at once in the minimap, then bake loaded chunks over them as usual. The fullscreen map shows saved
    tiles for explored areas beyond the loaded chunks. A pack/looks rebuild marks saved tiles stale (shown until
    re-baked, or dropped if colours changed: choose and document).
M3. **Use H2** when the host has level 23 (fall back to `columns` + `prepare` otherwise); identical pixels (tests).
M4. Bench (Pulley): join fresh, rejoin with saved tiles (time to box filled), fullscreen open, fly; version 0.4.0.

Then: desktop live test, push fork (new IPA), release diorama + new modpack, update memory.
