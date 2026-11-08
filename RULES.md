# RULES.md: numbers and formulas the port needs

Source: Diorama (MIT, (c) 2026 Dara Adedeji), local clone `/home/worc/Projects/diorama-src`, HEAD `dec36d2`.
Every rule below is read from the Java source, not bytecode, and restated in my own words. Paths are relative to
`src/main/java/dev/diorama/` (written `D/`). Nothing here is copied code. Items I could not verify are marked
**UNKNOWN**. Only numbers and formulas are taken; no assets, colour tables or textures.

---

## 1. Projection (D/Projection.java)

- Camera looks north and down. `sin`/`cos` are of the style pitch (Projection.java:17-18; pitch applied at :33-36).
  Default pitch is 50 degrees (MapStyle.java:62).
- Map column `u = x` (east right). Map row `v = z*sin(pitch) - y*cos(pitch)` (south down, higher blocks go up)
  (Projection.java:56-58, :66-68). Facing NORTH is the default (Projection.java:19); NORTH is the identity turn
  (Facing.java:17, :50-57 `default -> x`). v1 supports north only.
- Inverse: the z where a ray through map row `v` crosses height `y` is `zAt = (v + y*cos) / sin` (Projection.java:71-73).
- Tile constants: `TILE_UNITS = 128` map units per tile edge, `PIXELS_PER_UNIT = 4`, `TILE_PIXELS = 512`,
  `COVERAGE_CELLS = 64` (Projection.java:22-27). The port does not use tiles (mosaic instead); kept for reference.
- A block's position on the map: the minimap uses block centre `(x+0.5, z+0.5)` and the block's own `y` for waypoints
  (waypoint/WaypointOverlay.java:56-57). The viewer is centred at `(u(px,pz), v(px,py,pz))` using the interpolated
  player position (MinimapHud.java:171, :179).

## 2. Face visibility rules (ray trace; the port re-derives them for a column painter)

The Java baker marches a ray per pixel (bake/TileBaker.java:905-1075). Rules that must hold in the port:

- Ray direction per unit t: `y -= sin`, `z -= cos` (TileBaker.java:1013-1014; stepping :913-916). So the ray moves
  north and down. It crosses a y-plane (hits a TOP face, `enteredTop = true`) or a z-plane (hits a SOUTH face,
  `enteredTop = false`), whichever boundary comes first (:1064-1074). Only top and south faces are ever visible.
- The face colour for a top hit comes from the block's UP face, for a south hit from the block's SOUTH face
  (bake/BlockLooks.java:432-433).
- Block kinds (BlockLooks.java:81-88): AIR 0, SOLID 1, WATER 2, LAVA 3, GLASS 4 (see-through), PLANT 5 (no collision).
  - SOLID and LAVA stop the ray (TileBaker.java:994-1039).
  - WATER does not stop it: the first water cell entered is remembered, the ray continues to the floor and the water is
    mixed over it (:1041-1048, :1032-1033). If no floor is found, the water is drawn over black with a 64-block path
    (:1093-1097).
  - PLANT and GLASS do not stop it: each contributes a partial colour overlay. Plant weight 0.4, glass 0.25, each
    scaled by what is still passing: `over += through*w*albedo; through *= (1-w)` (:1049-1059). After the hit,
    the surface is mixed `out = out*through + over*light`, where `light` = luminance(out)/max(0.05, luminance(albedo))
    (:1025-1031).
  - Partial blocks (slabs, stairs, fences): hit only inside their boxes, otherwise treated as AIR for that ray
    (:969-991). v1 may treat them as full cubes; this is a documented look difference.
- Side faces are darker: `SIDE = 0.82` multiplies a non-top face's light (TileBaker.java:80, :1246-1248, :1229).
- Outline: a pixel is darkened when a neighbour pixel is more than `OUTLINE_DEPTH = 1.5` blocks nearer (TileBaker.java:84,
  :882); in cave slices the threshold is `CAVE_OUTLINE_DEPTH = 3` (:85, :891). The darkening factor is
  `1 - style.outline * outlineAmount` (SurfaceArea.java:78-79). Outline fades on floors under water:
  `outlineAmount = exp(-waterDepth / 6)` with `OUTLINE_WATER_FADE = 6` (TileBaker.java:82, :833-834).
- Anti-aliasing: 4 rotated-grid samples per pixel `(0.375,0.125) (0.875,0.375) (0.125,0.625) (0.625,0.875)`,
  or 1 centre sample in Fast quality (TileBaker.java:23-25). The port uses one sample (dropped, see section 10).

## 3. Texture averaging rule (bake/BlockLooks.java:860-913)

Per sprite, over every pixel inside `min(sprite size, image size)`:
1. Skip pixels with alpha < 26.
2. Weight `wa = alpha/255`.
3. Accumulate the stored sRGB channel values (0..255) times `wa`, **not** converted to linear (comment at :892-894
   says linear averaging was tried and looked wrong).
4. Result colour = weighted sum / total weight / 255, per channel. `coverage` = visible pixel count / (w*h).
5. No visible pixels: result is `(0,0,0)` with coverage 0. Image missing: magenta `(1,0,1)` coverage 1 (:867-869,
   :906-912). The port may return magenta for an undecodable sprite.

Per face (one direction, several quads possible), a running average (BlockLooks.java:915-979):
- Each quad contributes its sprite average weighted by that sprite's coverage `w`; zero-coverage quads are skipped
  (:924-928).
- Quads with a tint index go into the **tinted** sum; quads without go into the **untinted** sum (:929-942). The first
  tint index seen is remembered (:930-932).
- `untinted` = sum/weight, `tinted` = sum/weight, each packed as 8-bit sRGB via round(v*255) clamped (:959-978).
- `tintWeight = tintedWeight / (untintedWeight + tintedWeight)` (:967-970).
- Faces used: UP for the top colour, SOUTH for the side colour (:432-433).

## 4. Biome tint rule

- Tint kinds: NONE 0, GRASS 1, FOLIAGE 2, DRY_FOLIAGE 3, WATER 4 (BlockLooks.java:90-94). Blocks with kind NONE may carry
  a fixed `constantTint` (or white if none) (TileBaker.java:1126-1131).
- Albedo in linear light (TileBaker.java:1114-1124), per channel:
  `albedo = lin(base)*(1-w) + lin(tinted)*lin(tint)*w`, where `w` is the face tint weight, `base` the untinted average,
  `tinted` the tinted average, `tint` the column's biome colour for that kind, and `lin` the sRGB-to-linear table.
- Column tint lookup: index `((z&15)<<4)|(x&15)` into per-chunk arrays for grass, foliage, dry foliage, water
  (TileBaker.java:1136-1144). Missing chunk: white (:1133-1135).
- Diorama blends each column's tint over the game's biome blend radius around the column's surface block, per colour
  resolver, averaging RGB over the blend window and skipping unknown cells (bake/BiomeTints.java:65-125). The port
  uses the host's per-column colours unblended: a small look difference near biome edges (RULES section 10).
- sRGB <-> linear tables: `v <= 0.04045 ? v/12.92 : ((v+0.055)/1.055)^2.4` (TileBaker.java:111-115) and the inverse
  `v <= 0.0031308 ? 12.92v : 1.055*v^(1/2.4) - 0.055`, sampled at 4096 steps and rounded to 8 bits (:116-120).
- Optional per-block brightness nudge (`blockVariation`, default 0 so unused) uses a hash of the block position
  (TileBaker.java:1149-1156, :1544-1550). Not needed.

## 5. Lighting and shading (default style, daytime surface)

Default style `MapStyle.DEFAULT` = FLAT with changes (MapStyle.java:48-69):

| Field | Value | Where |
|---|---|---|
| pitchDegrees | 50 | MapStyle.java:62 |
| saturation | 1.35 | :63 |
| exposure | 1.16 | :63 |
| sunTint (r,g,b) | 1.14, 1.0, 0.8 | :63 |
| shadowTint (r,g,b) | 0.74, 0.86, 1.18 | :63 |
| outline | 0.38 | FLAT, :53 |
| blockVariation | 0 | BASELINE, :49 |
| roundedFog | true | :53 (fog only; dropped) |
| clearWater | true | :69 |
| skyOcclusion | 0.85 | :64 |
| sunElevation | 38 degrees (clamped to 25..60) | :65, :45-46 |
| ambient | 0.34 | :66 |
| sunStrength | 0.62 | :66 |
| heightShade | 0.22 | :67 |
| cornerOcclusion | 1.5 | :68 |

Surface pixel light (bake/Lighting.java:45-92):
- `direct = sunStrength * lambert` if lit and not shadowed, else 0; `lambert = 1` for a top, `max(0, sz)/sy` for a
  side (TileBaker.java:1223). Then `direct *= aoSun`.
- `aoSun = max(0, 1 - cornerOcclusion*(1-ao))`, `aoSky = min(ao, aoSun)` (Lighting.java:49-50). AO levels
  `{0.6, 0.74, 0.87, 1.0}` (TileBaker.java:78). (Corner AO is dropped in v1.)
- `ambient = style.ambient * aoSky`, then if skyOcclusion > 0: `ambient *= 1 - occ*(1 - (0.15 + 0.85*sky))`
  where `sky` is open-sky visibility 0..1 (Lighting.java:54-60).
- Colour: `rgb = ambient*shadowTint + direct*sunTint` per channel (Lighting.java:61-65).
- Height shade: `f = 1 + heightShade * clamp((y - 64)/64, -1, 1)`, multiplies rgb (Lighting.java:66-75). For flat water the
  same formula uses `(blockY+1 - 64)/64.0` (TileBaker.java:1289).
- Emission glow: a block with emission level `e` (0..1) raises each channel to at least `0.35 + 0.85*e`
  (Lighting.java:79-84; the same in cave and lid paths).
- Final light is multiplied by `facing` (1 for top, SIDE 0.82 otherwise) (Lighting.java:89-91); pixel linear colour is
  `albedo * light` (TileBaker.java:1232-1234).
- Sun direction: azimuth vector `(-0.878, 0.479)` in (x, z) (west, slightly south) (TileBaker.java:47-48); elevation
  per style. Shadow trace is up to 40 blocks (`SHADOW_DISTANCE`, :99) in 96 steps (`SHADOW_STEPS`, :97). Dropped
  in v1.
- Sky occlusion rays: 2 rings of 6 at elevations 25 and 55 degrees plus one straight up (13 rays), weights `sin(e)` and
  1.5 for the vertical (TileBaker.java:62-77); reach 12 blocks (`SKY_REACH`, :60). Dropped in v1.

## 6. Colour grade (final step) (TileBaker.java:1556-1568)

Input is linear RGB. `l = 0.2126 r + 0.7152 g + 0.0722 b` (:1539-1541). Then per channel
`c' = (l + (c - l)*saturation) * exposure`, clamp to 0..1 (via index `clamp(int(c'*4095), 0, 4095)`), look up the
linear-to-sRGB table, pack. Defaults: saturation 1.35, exposure 1.16.

## 7. Water (clear water on, default) (TileBaker.java:1270-1317)

- Depth = `max(0, tHit - tEntry)`, the path length of the ray through water (:1292), in ray units (not metres of height;
  one unit of t advances `sin` in y and `cos` in z).
- Surface light for the water: `ambient + (shadow ? 0 : sunStrength)`, ambient reduced by sky occlusion as in section 5,
  times the height-shade factor with `(blockY+1-64)/64` (:1280-1290). Cave slices use `Lighting.caveWater`
  (`0.42 + (max(blockLight, 0.8*skyLight)/15)^1.5 * 0.8`, Lighting.java:102-104) times the cave fade.
- With `clearWater`, per channel (water colour `w` linear from the water tint, `peak = max(wr, wg, wb, 1e-4)`):
  - transmission `t_c = exp(-depth * (0.45*(1 - w_c/peak) + 0.16))` (:1298-1301).
  - `deep = light * (0.3 + 0.7*exp(-depth/18))` (:1302).
  - deep colour `= max(0, l + (w_c - l)*1.2) * deep`, with `l` = luminance of the water colour (:1304-1307).
  - result `= floor*t_c + deepColour*(1 - t_c)` (:1308-1310).
- Without clearWater: `a = clamp(1 - exp(-0.2*depth), 0.5, 0.94)`, `out = floor*(1-a) + w*light*a` (:1313-1316).
- Water colour itself comes from the biome water tint through the same albedo path (section 4).
- Lava: noise nudge `1 + 0.3*noise(x,y,z)` on R and B, and `(1 + 0.3 noise)^2` on G (0.1 when under a roof)
  (TileBaker.java:1165-1171). Block position noise = hash with primes 73856093, 19349663, 83492791
  (:1544-1550). Optional.

## 8. Caves, Nether, layers

- Cave slice depth: look `CAVE_DEPTH = 48` blocks below the cut (TileBaker.java:87). `CAVE_ROCK = 10` (:89).
  Fade with depth: brightness is about a third at `CAVE_FADE = 12` blocks (:91), starting after `CAVE_FADE_START = 6`
  blocks below the cut (:93). Wall fade `CAVE_WALL_FADE = 4`, reach 16 (:95-96). Non-depth-look fade
  `1 - min(0.5, (startY - hitY)/60)` (TileBaker.java:1206).
- Cave light (Lighting.java:106-133): `bl = blockLight/15`, `sl = skyLight/15*0.8`, `warm = bl^1.5`, `cool = sl^1.5`,
  base 0.42; `r = (0.42*0.92 + 0.9warm + 0.6cool)*ao*fade`, `g = (0.42*0.96 + 0.75warm + 0.65cool)*ao*fade`,
  `b = (0.42*1.08 + 0.5warm + 0.75cool)*ao*fade`. Soft knee: if peak > 0.5, eased peak = `0.5 + 0.3*(1 - exp(-(peak-0.5)/0.3))`
  (CAVE_KNEE 0.5, CAVE_CEILING 0.8, Lighting.java:13, :20, :118-123). Fog colour `(0.008,0.010,0.015)` blended by
  `slate = 1 - fade` in depth look (:15, TileBaker.java:1209-1213).
- Cave slice for a player at feet height `fy`: `floorDiv(floor(fy) + 7, 4) * 4`, i.e. 2 to 5 blocks above the head
  (MapSession.java:656-658). Follows the player down at once, up only when the cut gets within 1 block of the head
  (`caveLayer <= floor(y)+2`) (MapSession.java:678-684).
- Underground test (MapSession.java:623-637): in a dimension with a ceiling: always true. Otherwise with hysteresis,
  based on eye position: becomes true when not seeing sky and sky light < 5; stays true while not seeing sky and sky
  light < 10.
- Auto layer (MapSession.java:661-671): not underground gives the surface. Underground with `preferSurface` and
  `depthToSky(eye) < MIN_CAVE_DEPTH (8)` gives the surface (a roof). Otherwise the cave slice. `depthToSky`: count
  blocks upward from the eye, at most `MAX_SKY_DEPTH = 32`, to the first cell with sky light 15; returns 33 if none
  (MapSession.java:50-52, :687-696).
- Nether/roof dimension: roof y = `minY + logicalHeight` (MapSession.java:81-82). The roofless sky view starts just
  under the roof, skipping the rock (TileBaker.java:240-265). Because ceiling dimensions are always "underground" and
  `depthToSky` returns 33 there, the Nether always auto-selects a cave slice when `autoCaves` is on. Roof lighting
  constants: `LID_AMBIENT 0.26`, `LID_KEY 0.52`, `LID_KEY_TINT (1.1,1.0,0.88)`, `LID_FILL_TINT (1.0,0.86,0.84)`,
  `LID_GLOW 0.7` (TileBaker.java:28-32; used :1180-1201). Haze: starts `HAZE_START = 30` below the roof, depth 55,
  max 0.72, light 1.6, max luminance 0.025 (TileBaker.java:37-44; :1253-1260 for the mix
  `f = min(0.72, 1 - exp(-below/55))`).
- Cave water/wall specifics and `CaveSight` flood fill (radius 24, 60,000 blocks, CaveSight.java:23-24) are dropped in v1.

## 9. Default settings, sizes, zoom, keys

Settings.java defaults (field lines):
- minimap on (:61); size **96 GUI px** (:64); corner TOP_RIGHT (:65); enlarged 200% (:67); zoom **2 px/block** (:69);
  opacity 100 (:73); compass on (:74); coordinates on (:75); biome on (:76); gameTime OFF (:77); autoCaves on (:80);
  preferSurface on (:82); legibleCaves off (:88); frame CLASSIC (:89); shape SQUARE (:90); outlines on (:148);
  deathMarkers on (:114); bedMarkers on (:116); waypointsOnMap on (:112); waypointDistance 0 (:124); minimap pin size 60%
  (:130); arrowColor 0 (:110); quality FANCY (:161).
- Zoom steps in GUI px per block: `0.1875, 0.25, 0.375, 0.5, 0.75, 1, 1.5, 2, 3, 4` (Settings.java:34). Step is the
  nearest value to the saved number (:297-305), clamped to the table (:307-309). Zoom keys step +1/-1 clamped
  (DioramaClient.java:55). Cave slices and maps without overviews never go below `DETAILED_ZOOM = 0.5`
  (MinimapHud.java:271-274; render/MapViewport.java:31). The plan's six-step subset (0.5, 1, 1.5, 2, 3, 4, default 2,
  with 0.75 omitted) fits the 256-block mosaic; **0.75 is a Diorama step the plan skips**.
- Size range 64..224 step 8 (Settings.java:44-46). Enlarge 125..400% step 25, default 200 (:48-50, :67). Opacity 20..100
  step 5, snapped (:52-54, :473-476).
- Layout (MinimapHud.java): margin 6 GUI px (:42), minimum drawn size `SMALLEST = 64` (:45). Top corner y = `6 + effects`
  (:177); right x = `guiWidth - size - 6` (:176). Status-effect offset: 52 if a harmful effect shows, else 26 if a
  beneficial one, else 0 (:357-373). Fit: top corners shrink to `height - 6 - effects - infoHeight - 6 - 4` and
  `width - 12 - 4` (:283-285); each info line is 10 px (:174); enlarge multiplies size by `enlarged/100` (:99).
- Compass (MinimapHud.java:294-303): letters N E S W at top-centre `(x+size/2, y+1)`, right `(x+size-5, y+size/2-4)`,
  bottom `(x+size/2, y+size-9)`, left `(x+5, y+size/2-4)`; N is `0xFFFF6B6B`, others `0xFFFFFFFF`. Each letter has a
  dark box behind it, colour `0xA0101418`, padded 2 px left/right and 1 px top, 8 px below (:305-309). North-facing
  only in v1.
- Info lines under the map (MinimapHud.java:311-331): coordinates `"x, y, z"` of the block position, then the biome's
  translated name, then the game time. Text colour `0xFFFFFFFF`; the time turns `0xFF9CC4FF` when `isDarkOutside`
  (:52-54, :328).
- Game time (MinimapHud.java:337-350): ticks use the **overworld** clock in all dimensions; `sinceMidnight = ticks + 6000`;
  `day = floorDiv(sinceMidnight, 24000) + 1`; `inDay = floorMod(...)`; `hour = inDay/1000`; `minute = (inDay%1000)*60/1000`;
  12-hour mode maps hour 0 to 12 and shows AM/PM by `hour < 12`. Formats: DAY_24H, DAY_12H (with day), ONLY_24H,
  ONLY_12H (Settings.java:198-204).
- Keys (DioramaClient.java:36-44): new waypoint **B**, waypoint list **U**, zoom in **=**, zoom out **-**; open map **M**
  (dropped); settings, toggle minimap, enlarge, toggle dimension unbound. Key ids use `key.diorama.*`.
- Opacity: layer fade 300 ms and hold 3000 ms exist (MinimapHud.java:50-51), dropped.

## 10. Waypoints

- Palette (Waypoint.java:38-41), 16 ARGB values, in order:
  `E5484D F76B15 FFC53D 46A758 12A594 0090FF 3E63DD 8E4EC6 D6409F FFFFFF B0B4BA 60646C 8D6B4B 99D52A 7CE2FE F0C0D8`
  (all alpha FF). Index names: red, orange, amber, green, teal, sky, indigo, violet, pink, white, silver, slate,
  brown, lime, ice, blossom (waypoint/WaypointColours.java:23-26). The editor also offers 16 dye colours (:28-31)
  (v1 may skip).
- Fields: id, name, initials, icon, x, y, z, dimension (default `minecraft:overworld`), color (default `PALETTE[0]`),
  visible (true), folder, kind (default MANUAL), created (ms) (Waypoint.java:55-70). Kinds: MANUAL, DEATH, TEMPORARY,
  WAYSTONE, BED, NETHER_PORTAL, END_PORTAL, END_GATEWAY (:15-30). v1 uses MANUAL, DEATH, BED.
- Initials: first letter of up to the first two words, uppercase; `"?"` if empty. `MAX_INITIALS = 4` for user edits
  (Waypoint.java:52-53, :139-147).
- New waypoint (Waypoints.java:32-43): name `"Waypoint N"` with `N = existingCount + 1`; colour is a **random** bright
  colour at least 1/6 of the hue wheel away from the previous waypoint's colour, with saturation `0.6 + rand*0.35` and
  value `0.8 + rand*0.2` (waypoint/WaypointColours.java:67-79). The port should pick from `PALETTE` instead to avoid a
  random source (documented difference), skipping the previous colour.
- Distance: `sqrt((x+0.5-px)^2 + (y-py)^2 + (z+0.5-pz)^2)` (Waypoint.java:131-136).
- Near rule: `waypointDistance` 0 means any distance; DEATH, TEMPORARY and BED always show (WaypointStore.java:334-340).
  Distance options `0, 64, 128, 256, 512, 1024, 2048` (Settings.java:42).
- **Death marker.** Diorama adds it from a mixin on `handlePlayerCombatKill` at TAIL, only when the packet's player
  id is the local player's (mixin/DeathPacketMixin.java:17-23). It is called "for every death, instant respawn or not"
  (:12). The marker is at `player.blockPosition()` at that moment, name `"Death HH:mm"` (24-hour local time, pattern
  `HH:mm`, Waypoint.java:49, :97-99), colour `0xFF60646C` (= PALETTE[11], slate), kind DEATH (Waypoints.java:163-176).
  Skipped when `deathMarkers` is off. **At most `DEATHS_KEPT = 3` death markers are kept**: after each add, the oldest by
  `created` are removed until 3 remain (WaypointStore.java:40, :346-352). The plan's trigger ("health dropped to 0")
  is a stand-in because Horizon has no combat-kill packet; **UNKNOWN** whether Horizon exposes anything closer.
- **Bed marker.** Kept per dimension, one at most. Trigger: the local player goes from not sleeping to sleeping, with
  `bedMarkers` on, using the sleeping position (Waypoints.java:115-130). If a BED waypoint exists in that dimension, its
  coordinates are moved; otherwise a new one is added: name `"Bed"`, colour `0xFFE5484D` (= PALETTE[0]), kind BED
  (:146-158). In 26.3 builds, if the block's bed rule says it can never set a spawn (a straw bed), nothing changes
  (:139-145; this check needs the block-state API and is **UNKNOWN** for Horizon). If the world is not ready, retry
  next tick (:125-127). Bed time colour is unrelated (that is the info-line colour).
- Store (waypoint/WaypointStore.java): per world file `.minecraft/diorama/<world>/waypoints.json` (:31, :80), written via
  a temp file and rename (:517); an unreadable file is renamed with a timestamp suffix (:435). The port uses Horizon
  storage keys per world id instead (see PLAN-v2 section 3.3). The exact JSON schema is **UNKNOWN** here (not read);
  only the field names above are verified from `Waypoint.java`.
- Pin layout on the minimap (waypoint/WaypointOverlay.java:38-78):
  - Position: `x = screenX(u(wx+0.5, wz+0.5))`, `y = screenY(v(wx+0.5, wy, wz+0.5))`.
  - Pin size is `Pins.SMALL = 13` on the minimap (`LARGE = 15` fullscreen) (render/Pins.java:22-23), times
    `Pins.scale(waypointMinimapSize%)`, which rounds to whole screen pixels.
  - If the whole standing pin does not fit inside the view (or circle), it becomes an **edge badge** pulled to the edge:
    for a square view, divide the offset `(dx, dy)` from the centre by `reach = max(1, max(|dx|/halfW, |dy|/halfH))`
    where `halfW = max(1, w/2 - inset)`, `halfH = max(1, h/2 - inset)`, `inset = edgeReach * scale`; for a round view,
    `reach = max(1, hypot(dx,dy)/radius)` with `radius = max(1, r - inset)` (WaypointOverlay.java:51-55, :64-66, :73-74).
  - Caret on the badge points along `angle = atan2(py - cy, px - cx)`, snapped to 32 steps (`CARET_STEPS`,
    Pins.java:25, :94-97). Caret length 5 (6 for the large size).
  - `edgeReach` and `fitsStanding` box maths live in render/Pins.java (:114-142); I did not transcribe every box
    constant: **UNKNOWN**; the port can use its own pin size.
- Waypoint pin size setting: 25..200%, default 60 (Settings.java:127-131, :232-236).

## 11. Zoomed view maths for the port (derived, not a Java rule)

Draw the mosaic so the player's projected position `(u, v)` is at the map box centre. One GUI pixel on screen covers
`1 / zoom` map units (zoom in GUI px per block). The Java viewer uses the same centre (MinimapHud.java:179).

---

## 12. What is dropped in v1, and why

Section 1 deferred list, with extra detail from the source read:

| Dropped | Why |
|---|---|
| Fullscreen map (`MapScreen`, `MapViewport`, `MapLevel`) | Needs bigger Horizon UI/image APIs; not worth the Pulley budget. |
| Explored-map save, fog of war, PNG export (`TileStore`, `SavedChunks`, `ExportScreen`, `MapExport`) | Needs file and big-image APIs; v1 only draws what is loaded. |
| Radar and mob faces (`Radar`, `MobFaces`, `ModelFaces`, `TrackedPlayers`) | Needs entity and model data; assets-heavy. |
| Drawings, claims/FTB, Xaero interop, server rules, crafts | Other mods' data and large features outside the minimap. |
| GPU baking (`GpuMesher`, `GpuTileBaker`, `render/gpu/*`, shaders) | Mods have no GPU access; shader files are not ours to copy. |
| Sun shadow rays, sky occlusion rays, corner AO, `RockRays`, `Lights` | Over the 0.3 ms budget on Pulley; replaced by flat ambient plus height shade. |
| 4-sample anti-aliasing | Cost; the port uses one sample per texel. |
| Biome tint blending (`BiomeTints.blend`) | Uses the host's unblended per-column tints; small edge difference. |
| Frame images (`MinimapFrames`) and radar icons | Diorama's assets are never copied; a plain 1 px border is drawn. |
| Layer fade (300 ms) and hold (3000 ms) | Dropped for simplicity. |
| `CaveSight` flood fill, seen-block reveal | 60,000-block spread is too slow in WASM. |
| Turned maps (`Facing` EAST/SOUTH/WEST) | v1 is north-facing only. |
| Waypoint folders, icons, share links, Xaero import, portals, waystones, temporary pins | Outside the v1 waypoint scope. |
| Waypoint edit screen text fields | No text widget in `ui`; naming goes through `/dwp`. |
| Multiple maps per dimension (`Maps`, `MapsScreen`), dimension toggle | Out of scope. |
| Day/night look (`NightLook`, `DayNight`) | Out of scope; the map is a daytime view. |
| Legible cave maps (`legibleCaves`) | Optional extra; off by default in Diorama. |

## 13. Differences found between the plan and the source

1. **Default size is 96 GUI px, not 10% of screen width** (Settings.java:64). The plan's 10% is a deliberate port
   choice; Diorama's own range is 64..224 (Settings.java:44-45).
2. **Default zoom is 2 px/block** (Settings.java:69) and the table has 10 steps including 0.1875, 0.25, 0.375 and
   0.75; the plan's subset skips 0.75.
3. **Death trigger** is the combat-kill packet, not a health check (DeathPacketMixin.java:17).
4. **Death marker colour** is `0xFF60646C`, name `Death HH:mm` (local time); **bed** is `0xFFE5484D` named `Bed`.
5. **New waypoints get a random colour** from an HSV range, not a palette pick (WaypointColours.java:71-79).
6. **Nether**: ceiling dimensions are always "underground" with `autoCaves` on, so the auto layer is always a cave
   slice there; `preferSurface` does not apply (MapSession.java:624-626, :667).
7. The plan says south faces are drawn "from the top down to the south neighbour's top": consistent with the ray rule
   (section 2), but water and plants add overlay rules (section 2) the plan does not list.
8. The default style's `roundedFog` is true, but it only affects fog and is irrelevant to a loaded-chunk minimap.

## 14. How the port applies the colour rules (M4)

Code: `core/src/looks.rs` (sections 3-4), `core/src/style.rs` (sections 5-7), `core/src/paint.rs`, outlines in
`core/src/mosaic.rs` (`shown_at`). Every block colour is averaged at run time from the loaded packs'
textures (`client-world.texture`); no colour table or texture is in the repo. Differences from Diorama:

- The host names one quad per side (its first); a model with no flat up/south quad (crossed plants) gives its
  first face for both. Diorama sums every quad, so a grass block's side lacks its tinted overlay.
- Kinds come from the host's flags (`classify`); leaves are recognised by a block id ending in `_leaves`
  (stand-in for `BlockTags.LEAVES`). A texture the packs lack falls back to the map colour, not magenta.
- Textures are averaged at the size the host holds them (all block sprites resized to one size).
- Light: no corner AO, no sun shadows; sky visibility is the column's sky light / 15. Sun at elevation 38,
  azimuth (-0.878, 0.479), no per-sample jitter.
- Water: the floor is the same column's, straight down, at path `depth / sin(pitch)`; a water column deeper than
  the 12 blocks read has a black floor at the host's depth; a water south face uses an 8-block path.
- Plants and glass above a column tint only its top face. An overhang's face over air takes the block above.
- Outlines: one sample per texel (texels are half a block, Diorama's pixels a quarter), depth
  `-(y/sin + v*cot)` per texel, water path stored in eighths of a block.
- Lava's noise nudge and dry-foliage tints (the host sends no dry-foliage colour) are not applied.
