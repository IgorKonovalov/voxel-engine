# The fly-through: every key and flag

The fly-through (`apps/flythrough`) is the engine's testbed. This page lists every key it binds
and every flag it reads. The sources of truth are the code: `flythrough --help` prints one row
per flag from the same table the parser reads (`apps/flythrough/src/cli/table.rs`), the keys
are two tables in `apps/flythrough/src/toggles.rs` (`flythrough --keys` prints them), and the long-form notes are the
`flythrough::cli` module docs. When this page and `--help` disagree, `--help` is right and this
page is stale.

```sh
cargo run --release -p flythrough                       # the streamed cave
cargo run --release -p flythrough -- --level            # the bounded level
cargo run --release -p flythrough -- --source showcase  # the showcase cavern
cargo run --release -p flythrough -- --help             # every flag
```

Every toggle prints its mode to stdout and shows it in the window: the lighting, look,
upscaler and queue toggles raise a banner for 3 s, the menu's row shows it, and the stats
overlay (`F2`) carries the same rows.

## Keys

The keys are few (ADR-0061): a fixed set, a shortcut on a handful of entries, and everything
else in the menu. `Tab` opens the menu: a page per group, each entry with its live value and
its key where it keeps one; `Enter` flips or runs the selected entry, `Esc` goes back and
closes. Typing on any page searches the whole tree (`lam` finds the lamp, its bounce and the
missing light). `F1` opens the Keys page: every entry with its binds. `flythrough --keys`
prints the same tree with every entry's start value and key, from the tables in
`apps/flythrough/src/toggles.rs`; this section is that output's fixed and shortcut rows.

### In the menu

The menu draws as a panel of 80 x 45 cells at the largest whole glyph scale that fits the
window (2 at 1280 x 720, 3 at 1920 x 1080), centred, over a dimmed box behind its rows; the
window's top-right corner reads `fps <n>  frame <median> ms p99 <ms>` while it is open.

- **A switch or a cycle** (the lamp, the missing light, the view): `Enter` flips it or steps
  it to its next state; its row shows the new value.
- **A number** (the dig radius, which has no key, the queue's angle weight, the echo and tail trims, the
  pre-delay): `Enter` edits it. The row reads `dig radius  < 0.5 m >  0.1..3.0 m`: the value
  between `<` and `>`, its range after. `Up` and `Down` (the pad's D-pad) step it, held they
  repeat after 275 ms every 50 ms, and it stops at its range's ends; `Enter` or `Esc` (the
  pad's `South` or `East`) keep the value and leave the edit.
- **A grey row** is an entry the run cannot act on, with the reason on its own row after its
  value, on its page and in the search (`needs --audio`, `needs --pan`, `needs --path`,
  `needs --castle`, `needs --model`, `needs --level`, `needs --sky on`,
  `needs --source outdoor`); `Enter` on it changes nothing and says
  the reason. A group whose entries
  all need the same flag is grey on the root with the reason beside it. `--keys` prints
  `unavailable: <reason>` on those rows.
- **The notice row**, above the footer, holds the last line an action printed for 3 s, in
  yellow: `readings: sampling 30 frames` and then the frame line, the logic's `stamped: ...`,
  a refused entry's reason.

### Fixed

| Key | Does |
|-----|------|
| Mouse | Look |
| `W` `A` `S` `D` | Fly (or walk, in walk mode) |
| `Space` / `C` | Rise / sink; walking, `Space` jumps, or climbs a ledge with a standable top within 1.2 m |
| `Shift` | Faster |
| Left click | Dig at the crosshair; the edit result prints to the terminal. In placement mode: place the ghost |
| `E` | In placement mode: place the ghost, as the left click does; out of it nothing (in the menu it types) |
| Right click | Fill at the crosshair. In placement mode: leave it |
| Wheel | Enter placement mode at the belt's selected slot; in the mode, step the slot (down the next, wrapping at nine) |
| `1`..`9` | While the belt is shown (placement mode, and the chooser over it): the belt's slots, `4` included; `1`, `4` and `6` then drop, remove and fire nothing. Out of the mode they keep their shortcuts below and no digit enters the mode |
| `Tab` | The menu; type on any page to search |
| `F1` | The Keys page (with `--tester`: the help panel) |
| `F2` | Toggle the stats overlay |
| `Esc` | Clear the search, go back a page, close the menu; with the menu closed, leave placement mode, else ask before quitting: the banner reads `Esc again to quit` for 2 s, a second `Esc` inside that quits, any other key, a click or a pad button cancels (on a path run, end it so its report says how; under `--tester` and while loading the first `Esc` quits) |

While the menu is open the arrows, `Enter`, `Backspace`, `Esc`, the letters, digits and
punctuation, the wheel and the clicks are the menu's: the dig, the fill and the belt
pause, and a letter or digit shortcut types instead of acting. The movement keys still move
(their press also types), the mouse still looks, and the function keys still act, the open
row showing the new value.

### Shortcuts, by the menu's groups

| Group | Key | Does |
|-------|-----|------|
| moving | `G` | Toggle fly and walk mode (bounded worlds only) |
| camera and paths | `T` | On a `--path ... --path-mode tour` run: resume the path from its nearest point after a key took the camera |
| upscaler | `F10` | Cycle the upscaler: off, own, fsr2 |
| upscaler | `F4` | The next render profile: floor, portable, performance, high (`--profile`) |
| lighting | `F7` | Toggle the headlamp |
| lights | `1` | Drop a point light of the picked colour (intensity 600) 2 voxels off the looked-at surface (20 voxels ahead on a miss); the banner names `lights <n>/16`. Sixteen slots, the headlamp's among them while it is on; a drop into a full list prints a line and adds nothing |
| lights | `4` | Remove the newest light or walker, whichever was added last (in placement mode `4` is the belt's slot 4) |
| lights | `6` | Fire a point light of the picked colour (intensity 600) along the view ray at 20 m/s; it stops 2 voxels short of the first surface (or after 40 m) and stays there as a dropped light |
| entities | `` ` `` | `choose an entity`: the chooser (also the `entities` page's first row). Four rows per library entry beside its icon: a blank row, its name, parts and size on the cursor's row (an entry in error: its name grey, its error red), then its clips (the one a placement starts on marked `*`) and the belt's slot that holds it. `Enter` (or the left click, or the pad's `South`, or `` ` ``) on an entry puts it in the belt's selected slot (or selects the slot already holding it), closes the menu and enters placement mode; a digit `1`..`9` puts it in that slot and the chooser stays open; letters search the menu as on any page |
| entities | `R` | `turn the ghost`: 22.5 degrees about the up axis per press, kept until the mode ends; on a structure's slot a quarter turn (90 degrees) |
| entities | `Delete` | Remove the placed entity under the crosshair, the menu open or not, in placement mode or not |
| entities | `Left`, `Right` | The aimed (else the newest) placed entity's clip before and after (menu closed); in placement mode, the ghost's |
| entities | `Down` | Pause or play that entity's clip (menu closed); in placement mode, the ghost's |
| structures | `F` | `snap`: a structure's box on the ground under its footprint or free where the aim points (`ground`, `free`; the pad's D-pad up with the menu closed); the notice says the new snap, the HUD row ends `snap ground` or `snap free` and the belt's digits row ends with `free` in yellow while free |
| readings | `Enter` | Sample the next frames and print frame-time readings (menu closed) |
| debug | `F9` | Cycle the view: shaded, normals, hitmask, steps, cracks, albedo |
| debug | `X` | Cycle the cutaway: auto, on, off |

Everything else is an entry of the menu (`Tab`) with no key: the dig radius (`moving`), the
look presets and far normal,
the sharpening and pan, the lighting modes but the lamp, the lights' carry, colour and bounce
flag, the light queue's order, weights, withdrawal, budget, full-first, missing light and
reach, the wander (`wander` under `entities`: every placed body wanders, the built-in walkers
walk their circle) and the eye lights, the audio's sounds and mix, the bench, the stamp,
the crumble, the coarse print, the castle shot and the shadow ray, the `structures` group
(`choose a structure`: the structures' chooser, four rows per structure beside its icon, its
name, placements and size, then the palette rows its file added and its slot; `Enter`, a
digit and `` ` `` act as on the entities' chooser; `clear box` on or off [on], `structure
scale` 1 to 4 [1] and `snap` (`F`) ground or free [ground], shown on the HUD's `clear box on,
structure scale 1, snap ground` row), and the world and sky page:
the time of day (a number, an hour a step, held between 00:00 and 23:59) and the day cycle
(`--day-length`) and the sky rays (1 to 4) under a sky, the next world source (`--source`'s
order: cave, level, showcase, outdoor, and back, rebuilt in process with that source's start
settings; the window, the frame, the clock and the menu stay; a source that refuses the run's
other flags is passed over), and on the outdoor hills the next `--trees` (none, sparse, forest,
and back), rebuilt in process as the source is. Under `--loading-screen on` a rebuild shows the
loading screen at once, naming what loads (`loading outdoor, trees sparse`, `switching from
cave`), and loads behind it as start-up does; the run then comes back with the menu and the
switch's notice.

The HUD's entities row reads `entities <n>/64 parts <m>/512 over <k> pool <used>/<total> KB`
(then `tick <us>`, the movers' CPU a tick as the median over the last second, once a body has
ticked, and `wander` while the wander is on): the entity pool's bytes in use, rounded up to
whole KB, against its size. A library entry whose rig has a `body` line moves at a 60 Hz tick
(at most 8 ticks a frame), wander on or off: a ground body (the walker) stands on the floor and
falls when it is dug away, a surface body (the spider) clings to the face it touches, wall or
ceiling, and falls and lands when that is dug away. With the wander on (`wander on` in the
banner) each body picks a new heading every 5 to 15 s, mostly within a quarter turn, and walks or crawls at 1 or 1.5 m/s, its
`walk` clip kept in step with the distance and `idle` played when it stands still; with it off
each stands where it is on the clip the chooser gave it. A `--model` reload frees the replaced model, so the figure holds still over
saves that keep the model's size. When an upload is refused although the pool's free bytes
would hold it (no hole is large enough), the pool is cleared and every placed model uploaded
again, and the banner reads `entity pool reset`; a `--model` model refused after that keeps
its error on screen in red until a reload builds.

### Placement mode

The belt is nine slots, `1`..`9`, drawn centred along the bottom of the window in placement
mode and under the open chooser (its frame on the selected slot; a chooser digit moves the
frame to the slot it fills), over a dimmed box: each slot's digit and its entry's name, the selected slot
framed by `+`, `-` and `|` (the frame's top and bottom rows yellow), and the selected entry's
name above the belt for 3 s after the selection changes (`slot 5 empty  (choose with `)` for
an empty slot, an entry in error with its name grey and its error red). The belt starts filled with the library
in the chooser's order with the walker first (`--model`'s rig first, then the walker, then the
entries that build, then those in error, each in name order); a slot keeps its entry for the
run, and the chooser's digits change it: a digit on an empty slot moves the entry there, on an
occupied slot swaps the two (the same digit on the old slot swaps them back; an entry from no
slot moves the holder to the next empty slot, or off a full belt), and the notice says both
moves (`slot 4 walker, slot 1 castle`). The wheel, the pad's D-pad left and right or the chooser's `Enter` enter the mode from the
view, never a digit; in the mode every digit `1`..`9` selects its slot (`Delete` removes the
aimed entity). Opening the menu, `Esc`, the right click, `L2` or the pad's `East` leave it.

In the mode the menu is closed and the camera free. The selected slot's entry stands at the aim
as the ghost, a real entity lit where it will stand, facing the camera, turned by `R`: at the
camera ray's first world hit within 30 m, never nearer than 3 m or 1.5 times the model's
longest extent (the walker at 3 m, the spider at about 10.4 m), the feet on the hit when it is
a floor, and under a wall or a ceiling hit moved back by half the model's footprint and dropped
to the floor (up to 10 m below). A surface body (the spider) stands on the face it is aimed at
instead, wall or ceiling, its legs to the face, facing up the face (`R` turns it about the
face's normal), and a click attaches it there. With no hit, or a hit nearer than that distance,
the ghost stands at that distance along the view, dropped to the floor. The left click (or `R2`, or `E`) places
it there on its clip and turn (`placed spider #3 (rear)`) and the mode stays, a new ghost at
the aim; the dig and the fill rest while the mode is on. An empty slot, an entry in error or a
full set (64) stands no ghost and the banner says why. Under `--tester` there is no belt and no
mode.

The belt also holds the structures (`apps/flythrough/assets/structures`, then the generated
castle), filled after the entities: walker, spider, chapel, castle, slot 5 empty (the
spider's rows join every source's table at the start, `entities: 2 entries, +3 rows`). A
structure's ghost is a coarse copy of the structure inside a frame of its box at the scale and
turn, marked as one entity: the copy is the structure voxel for voxel while it is at most 64
voxels a side, else one voxel for each cube of `k` voxels (the least `k` that fits: 10 for the
castle), and the frame is its twelve edges, or its eight corners for a box over 64 voxels a
side. It writes no voxel. Its name row is the structure's size
(`chapel  4.8 x 3.2 x 4.0 m`, `  x2` after it at scale 2). The box's near side is at the
crosshair's first world hit within 30 m (on a wall or a ceiling, dropped to the floor below),
or the view's 3 m point dropped to the floor with no hit; its footprint grows away from the
camera along the view, so the camera is never inside it. With `snap ground` (the default) its
base is the lowest of a lattice of columns over the footprint (up to 8 x 8, corners included),
each cast down from just over that point: on a slope the uphill side stands in the cut
`clear box` makes, and a column that finds no ground within 10 m caps the base 10 m under the
point. With `snap free` (`F`) the box stands where the crosshair's first hit is, on any face
and with no drop, or 30 m along the view in the air, its base at that point. `R`
turns it a quarter turn. The left click (or `E`) places it as logged stamps through the server: with
`clear box` on, a stamp of written air over the box and 1 m on its sides and above, then a
stamp per placement; a part of the box not resident is written as its chunks arrive. The
notice reads `placed chapel 4.8 x 3.2 x 4.0 m: 5 ops, 4 chunks now, 0 pending`, and the mode
stays. The castle builds on a worker the first time its slot is selected (`castle: building`
in grey until it arrives; its icon and its ghost arrive with it). A box past the edit limit
(1024 voxels a side with its 1 m margin each side) is refused: the click places nothing and
the notice over the view reads `castle x2: 1200 x 1200 x 1000 voxels, over the edit limit
of 1024`, and while that slot is selected at that scale the belt's name row stays on screen with
the same words in grey, the reason in red. The castle (60 x 60 x 50 m) is past the limit at
every scale over 1; were it not, it would read `castle: scale 1 only`, having no file to scale. A
structure larger than the cave's rooms stands in its cleared box with the cut rock around it:
the fly-through has no notion of a room, so judge a large structure on the outdoor hills
(`--source outdoor --time 12:00`).

### Gamepad (`--pad on`, the default in a window)

| Input | Does |
|-------|------|
| Left stick | Move, at a speed set by its deflection |
| Right stick | Look |
| `L1` / `R1` | Sink / rise |
| `R3` | Latch boost |
| `South` | Walking: jump; in the menu: `Enter` |
| `R2` / `L2` | Dig / fill; in placement mode, place the ghost / leave the mode |
| D-pad up | The structures' `snap`, ground or free (menu closed); in the menu: move the selection up, or step the number being edited |
| D-pad down | In the menu: move the selection down, or step the number being edited |
| D-pad left / right | The belt's slot before / after, entering placement mode (`belt before`, `belt next`); in the menu: back a page / open |
| `North` | Toggle walk mode |
| `West` | Cycle the cutaway; in placement mode, turn the ghost (the cutaway is then the menu's) |
| `East` | Knock (with `--audio`); in placement mode, leave it; in the menu: back |
| `Start` | Open or close the menu |
| `Select` | Toggle the stats overlay |

### Tester mode (`--tester`)

Only movement, look, the left click's dig, `F1` (a help panel), `T` and `Esc` act. Every other
key is off, the stats overlay among them.

## Flags

Grouped as `--help` groups them. A default in brackets is the code's.

### Run

| Flag | Does |
|------|------|
| `--help`, `-h` | Print every flag and exit |
| `--keys` | Print the keys and the menu's tree at the start modes, and exit |
| `--library` | Print the entities library (each entry's parts, size in voxels, pool bytes and clips, the first marked `*`, or its error) and exit |
| `--check-args` | Parse the arguments, print `arguments ok` and exit, opening nothing |
| `--seed N` | The world's seed [1] |
| `--render WxH` | Fix the frame's output resolution; the window scales it |
| `--frames N` | Exit after `N` frames, printing the readings (`Enter`) over the last ones |
| `--pace free\|compositor` | Wait for the compositor's frame callback before each present or not [free] |
| `--bench N` | Send `N` bench inputs on the first frame |
| `--dig-at F,..` | Dig at each of these frames |
| `--bench-at F,..` | A bench input at each of these frames |
| `--sync-budget MIB` | The mirror's upload bytes a frame |
| `--sync-ms MS` | The mirror's upload time a frame [none; 0 is none] |
| `--workers N` | The streamer's generation workers, at least 1 [half the logical cores] |
| `--residency R_FULL,R_LOD` | The streamed full and LOD radii in chunks, hysteresis 1 [16,24] |
| `--mirror-budget MIB` | The VRAM the mirror reserves at startup [3072 streamed; on the level what it needs plus dig headroom]. While streaming, the full radius drops a chunk after 30 frames with the mirror's pressure over 0.9 and returns after 300 frames under 0.7, down to 6 |

### Session

| Flag | Does |
|------|------|
| `--autopilot` | Dig, fly 300 m out and back through the streamed cave, print the streaming readings |
| `--headless` | Render the same frames offscreen, with no window |
| `--adapter SUBSTRING` | Use the first adapter whose name contains it |
| `--hud` | Start with the stats overlay shown |
| `--census` | Print the brick census on exit |
| `--save PATH` | Write the session's log file on exit and print the world's hash (with `--level`) |
| `--replay PATH` | Re-simulate a saved session before the first frame |
| `--logic native\|wasm[=PATH]` | The server's logic: native [default], or a component (needs `--features wasm` and `scripts/build-guests.sh`) |
| `--path cave\|showcase\|castle\|outdoor` | Fly that scene's camera path (the scene's own flags and seed 1 required; `outdoor` needs `--source outdoor` with the `outside` spawn, the sparse trees and the default leaf fill, and takes any `--sun` or `--time`) |
| `--path-mode bench\|tour` | Fly the path once by wall time with the lamp off, then exit [bench]; or loop it |
| `--report DIR` | Write `DIR/run.json` at exit (with `--path`) |
| `--tester` | Only movement, look, the dig, `F1`, `T` and `Esc` act |
| `--pad on\|off` | Read a gamepad in a window [on; never headless or under the autopilot] |

### World

| Flag | Does |
|------|------|
| `--source cave\|level\|showcase\|outdoor` | The world: the streamed noise cave [default], the bounded level, the showcase cavern (a bounded 72 m hall lit by four ores; the lamp off, light radius 19 and its `hall` shot unless told otherwise; no `--save`, `--replay`, `--vault`, `--autopilot`, `--room` or `--castle`), or the streamed outdoor hills with a cave mouth under a sky (no `--level`, `--castle`, `--vault`, `--room`, a rig, `--save` or `--replay`). The banner's line starts `source NAME` |
| `--spawn outside\|inside\|forest` | Where `--source outdoor` starts: 30 m before the cave mouth facing it, 2 m up [default], 3 m inside the tunnel facing out, or 64 m north of the mouth facing north, 2 m up, 10 degrees down |
| `--trees none\|sparse\|forest` | Trees on the outdoor hills: about 9 % or 94 % of the 48-voxel cells of a 3072-voxel box between the mouth and the forest spawn; the banner's source reads `outdoor, trees NAME` [sparse] |
| `--leaf-fill N` | The canopy voxels kept, of 256, 1 to 256 [64] |
| `--leaf-shadow brick\|voxel\|absorb` | How a canopy shadows the sun and the sky: as its 8^3 bricks (a light's shadow ray stops where it enters a brick holding leaves, so no fleck shows through), voxel by voxel, or as a transmittance (the key light's ray and the sky rays pass the leaves, attenuated by `exp(-0.15 x)` over the path `x` in voxels through them); the banner reads `leaf shadow brick` or `leaf shadow absorb` with a sky set and names nothing for `voxel` [voxel] |
| `--sun EL,AZ` | Fix the sun at this elevation and azimuth in degrees (clockwise from north, 180 south), the moon opposite; the menu's `time` and `day cycle` leave it, and the banner reads `sun EL/AZ fixed` in place of the time |
| `--time HH:MM` | The time of day the sky starts at, latitude 45 at an equinox [12:00]; the banner names it while a sky is set |
| `--day-length S` | Seconds for a whole day while the cycle runs (the menu's `day cycle`) [1200] |
| `--sun-radius DEG` | The sun's angular radius, 0.05 to 10 degrees: a wider disc softens every sun shadow (the real sun's is 0.2665) [3] |
| `--moon-radius DEG` | The moon's angular radius, 0.05 to 10 degrees: its disc in the sky and its shadows' softness [3] |
| `--sky-rays N` | Cosine-weighted sky rays a lighting pixel casts where no baked sky visibility is fresh, 0 (bake only) to 4; the banner names the count [1] |
| `--sky-history N` | Frames the sky rays' running mean (the visibility pair) holds at most, 1 to 1024 [64] |
| `--key-history N` | Frames the key light's visibility mean (the pair's `g`) holds at most, 1 to 1024 [256]; the banner names it when not the default |
| `--key-far baked\|ray` | The key light's visibility where a voxel is small on screen (ADR-0079): the store's baked value per brick direction over the sun's disc, with no ray and no screen history, the band blended or stepped by `--key-far-mix` and a near pixel's restarted mean seeded from it; or the per-pixel disc ray everywhere; the banner reads `key far ray` or `key far baked, key far mix step` with a sky set (nothing for the default), and the menu's lighting page switches both (`key far`, `key far mix`) [baked] |
| `--key-far-mix blend\|step` | Inside the far normal's band under `--key-far baked`: the ray's mean blended to the store's value by the band's weight, or the store's from the weight's half [blend] |
| `--penumbra disc\|approach` | The key light's penumbra: one shadow ray a frame toward a random point of the sun's or moon's disc, or (a spike, Plan 0086 Phase 7) one toward its centre that scores how close it passes the rock beside it, the same every frame and on the lit side of an edge only; the banner reads `penumbra approach` with a sky set [disc] |
| `--sky-normal face\|smoothed` | The normal a cube face's baked sky light reads its visibilities by: the face's own (the far normal's blend far off), or the 27-count gradient's at every distance, so a tread and a riser of a one-voxel stair read the slope's blend; the surface and the sun keep the face normal; the banner reads `sky normal face` with a sky set [smoothed] |
| `--sky-bounce B` | The baked sky light's bounce floor, 0 to 1: this share of a face's visibility is its brick's six visibilities' mean weighted toward the normal (no ray); the banner reads `sky bounce B` with a sky set [0] |
| `--sky-anchor N\|none` | While a sky is set, the auto exposure opens at most `N` EV (-4 to 12) over the exposure that puts a sunlit horizontal face of albedo 0.5 at mid-grey (the sky's and the key light's irradiance on it, each frame), so a noon tunnel stays dark and the night still opens; the banner reads `sky anchor N` (`sky anchor none`; nothing for the default) and the exposure row `ceiling +X EV (anchor N)`. `none` keeps the constant ceiling [4] |
| `--column-wait on\|off` | `on`: the live queue bakes a chunk once every column over its reach holds all the chunks the streamer will hold (the sky rule's tops are final); `off`: once none of those chunks waits to upload, and a chunk baked under a column not yet final bakes again once when it is (faster first light, a provisional sky meanwhile). The banner names `column wait on` [off] |
| `--dump-black` | Once a second while the store's black count is over 0, print the lighting pixels with no record value grouped by the chunk they hit: its distance in chunks, whether it holds a block, its queue tracks, the chunks that hold its columns from final and whether its reach crosses the full radius (the queue's own test, as `--reach partial` reads it). Counts the store's reads; the banner names `dump black` [off] |
| `--exposure-max EV` | The auto exposure's constant ceiling in EV, -16 to 8; the exposure row names it as `ceiling +X EV` [+2 for `--source outdoor`, else +4] |
| `--exposure-rate EV/s` | The most the auto exposure moves toward its target a second, 0 to 64, `0` for no cap (ADR-0076 rule 5); a frame steps at most `EV/s` times its own time, and no further than a 15 fps frame's; the banner reads `exposure rate N EV/s` when not the default [3.75: 1/16 EV a frame at 60 Hz] |
| `--sky on\|off` | A sky over the world [on for `--source outdoor`, else off] |
| `--switch-at F,..` | Switch the source at these frames, as the menu's `source` entry does, and print the spawn's hash at the end |
| `--spawn-hash` | Print the voxel hash of the chunks within 2 of the spawn's chunk at the end |
| `--level` | The bounded level (as `--source level`) |
| `--thin-rig` | A bounded scene of thin features for the field look |
| `--seam-rig` | A bounded scene of seams for the field look |
| `--room METRES` | A closed cube of air of that edge, 1 to 100 m |
| `--shot hall\|chasm\|crystals` | With the showcase: the shot the camera starts at [hall] |
| `--pose vein\|patch` | With the level: the settle rig's still camera |
| `--walk` | Start in walk mode (on a bounded world) |
| `--glide V` | Move along the heading at `V` voxels a second |
| `--vault` | A 16-voxel room with an indestructible shell 10 m along +x from the spawn |
| `--castle` | A generated castle in the streamed cave's hall |
| `--castle-out PATH` | Also write the castle's `.vox` (implies `--castle`) |
| `--castle-shot gate\|nave\|window\|dungeon` | The castle pose to start at (implies `--castle`) |
| `--crisp none\|rock\|all` | The table rows marked crisp, which the field look draws as cubes [none] |

### Look

| Flag | Does |
|------|------|
| `--look PRESET` | The starting preset: `face` (honest cubes with the far normal; named here, with it off), `field` (the smooth surface, opt-in), `trilinear8`, `gradient27` [face] |
| `--face-mix K` | The preset's face mix, 0 to 0.5 |
| `--far-normal LO,HI` | Face mode's far normal band in pixels a voxel [2,6] |
| `--band-census` | Print the field surface's band census |
| `--view VIEW` | `shaded`, `normals`, `hitmask`, `steps`, `cracks` or `albedo` [shaded] |
| `--translucent-layers N` | Translucent voxels composited before the hit [4; 0 ignores them] |
| `--cutaway auto\|on\|off` | The cutaway view's start state [auto: on while the camera is in rock] |
| `--internal-scale 1\|1.5\|2` | The output size over the internal size [1.5] |
| `--jitter on\|off` | The primary rays' sub-pixel jitter (selects the `own` upscaler) |
| `--sharpen S` | The upscaler's sharpening, 0 to 1 [0.5] |
| `--upscaler off\|own\|fsr2` | The upscaler [fsr2, or own with `--jitter`, or off at scale 1 but under `--profile high`] |
| `--pan DEG_PER_S` | Turn the camera at a constant yaw rate (`.` stops and starts it) |
| `--jitter-phases N` | FSR 2's jitter phase count, 1 to 1024 |
| `--dither on\|off` | FSR 2's static one-level dither [off] |

### Lighting

| Flag | Does |
|------|------|
| `--lamp on\|off` | The headlamp [on; off on the showcase and a `--path` benchmark] |
| `--lamp-bounce on\|off` (alias `--indirect`) | The lamp's bounce from the probe volume, ADR-0043 [on] |
| `--shadow-walk full\|coarse` | The walk shadow rays take [coarse] |
| `--store-read face\|normal` | How a cube face reads the store, ADR-0049 [normal] |
| `--light-weights normal\|face` | The normal the light's filters compare [face] |
| `--young-filter luma\|edges` | The a-trous weights at a pixel younger than 4 frames: all, or plane and normal only (`Delete`) [luma] |
| `--entity-light own\|world` | An entity's emissive light: its own sample, or the baked light of the world faces around it (`Insert`) [world] |
| `--filter on\|off` | The a-trous filter runs [on] |
| `--light-radius N` | Chunks whose emitters light the frame, 0 to 64 [4; 19 on the showcase] |
| `--history edits\|gradient` | What kills the temporal history [edits] |
| `--lighting full\|half` | The lighting passes' resolution [half] |
| `--probe-spacing N` | Voxels between probes, 4 to 64 [16] |
| `--probe-rays N` | Rays each updated probe traces, 1 to 64 [64] |
| `--probe-slices N` | The probes refreshed a frame, one in N: 8, 16 or 32 [8] |
| `--emissive-ore on\|off` | The ore glows and lights the cave [on] |
| `--lamp-toggle N` | Switch the lamp every `N` frames |
| `--entities N` | Start with `N` of the library's selected entry (voxel-model entities) on a 3 m ring around the spawn [0] |
| `--entities-walk on\|off` | The wander starts, as the menu's `wander` sets it: every placed body wanders, the built-in walkers walk their circle [off] |
| `--model PATH` | View a `.rig` and the `.vox` it names (the library is `apps/flythrough/assets/entities/<name>/<name>.rig`, [entities.md](entities.md)): it is the library's first entry, selected, so `` ` `` and `--entities N` place it (a rig of the library itself is that entry, not a second one); a saved change to either file reloads its instances only |
| `--clip NAME` | The `--model` clip to play first [the rig's first] |
| `--materials PATH` | The material table file [`apps/flythrough/assets/materials.txt`; the built-in copy when that is missing]. Each scene takes its rows by name; while the window is open a saved change of colour, roughness, opacity or emission shows within a second (`materials reloaded`, the emissive chunks re-baked), a row added, removed or renamed or a class changed (`flags`, emitting or not) waits for a restart (`materials: shape changed, restart`), and an error names its line. A `--model` whose `.rig` has no `material` line for a palette entry takes the entry's colour and MagicaVoxel material as a row (`vox #rrggbb ...`, the HUD's `rows +N`) |
| `--vox PATH[@x,y,z][*k]` | Place a MagicaVoxel scene in the streamed cave at the start ([structures](../apps/flythrough/assets/structures/README.md)): its palette as material rows (glass, glow, metal), hidden layers left out, mirrored groups mirrored, `k` world voxels per `.vox` voxel per axis (1 to 4) [1]; its box's minimum corner at `x,y,z`, or 3 m ahead of the spawn, its base 1.6 m below it |
| `--vox-clear on\|off` | Carve `--vox`'s box, and 1 m on its sides and above, to air first [on] |
| `--vox-op` | Place `--vox` as logged ops instead of through the cave's source: a stamp of written air over the box (with `--vox-clear on`) and a stamp per placement, submitted on the first tick (a chunk not resident is written when it arrives), so the two paths can be compared |
| `--structures-dir DIR` | The structures library: every `DIR/<name>/<name>.vox` is a belt item after the entities, then the castle; each file's palette rows join the world's table at the start (`structures: N entries, +R rows`) [`apps/flythrough/assets/structures`] |
| `--library-dir DIR` | The entities library: every `DIR/<name>/<name>.rig` is an entry, in name order; `--model`'s rig comes first. A missing or empty library gives the built-in walker [`apps/flythrough/assets/entities`] |
| `--fade-reading` | Headless: dig, settle, switch the lamp off and print how the light fades |
| `--exposure auto\|E` | Adapt like an eye, or a fixed exposure [auto] |
| `--adaptation eye\|symmetric\|instant` | The auto exposure's preset [eye] |
| `--adapt LIGHT,CONE,ROD,FLOOR` | The preset's time constants and cone floor |
| `--metering centre\|average` | How the meter weights the frame |

### Light store and queue

| Flag | Does |
|------|------|
| `--bake-slice MS` | The queue's slice a frame, 0.1 to 100 ms [1.5] |
| `--bake-budget frame:MS\|second:MS,CAP` | The queue's budget, ADR-0052 |
| `--coarse N` | The coarse pass's samples a brick direction, 0 to 4096 [256] |
| `--near R` | Chunks baked in full before the first frame, 0 to 8 [2] |
| `--full-first off\|N` | The queue's full-first radius after the pre-roll [`--near`'s] |
| `--missing-light black\|neighbour\|parent\|chain` | What a face with no baked value reads, ADR-0053 [chain] |
| `--reach full\|partial` | Whether a chunk waits for its whole light reach [full] |
| `--queue-order score\|distance` | The live queue's order |
| `--queue-weights A,R\|A,tier` | The score's angle and refine weights |
| `--queue-withdraw on\|off` | Withdraw the jobs of chunks the camera passed |
| `--queue-census` | Read the queue's view census at every opening |
| `--edit-ms MS` | The slice's growth while a dig waits, 0 to 100 ms [3] |
| `--dig-reach on\|off` | A dig re-bakes the chunks within the light radius [on] |
| `--read-missing` | Read the store's no-value count while a dig waits |

### Audio

| Flag | Does |
|------|------|
| `--audio` | Open the default output device |
| `--audio-debug` | `--audio` plus the callback's readings and a recording |
| `--room-line` | Print the room line whenever a fan completes |
| `--absorption A` | The cave row's absorption per metre, 0 to 1 |
| `--mic` | Play the input device at the listener (implies `--audio`) |
| `--echo-db DB` | The echoes' trim |
| `--tail-db DB` | The tail's trim |
| `--predelay-ms MS` | The tail's pre-delay |
| `--tail-glide` | The tail's lines glide instead of handing over |
| `--flat-mix` | Both room curves flat at 0 dB |
| `--direct-delay` | The direct path's propagation delay |

### Loading

| Flag | Does |
|------|------|
| `--loading-screen on\|off` | Step the start-up (the entity pipelines, the level's upload or the streamed near residency, the light pre-roll, the field's fill) from the frame loop behind loading frames: a turning voxel cube, a bar that only moves forward, the scene, the stage and its count; the menu's in-process rebuilds (`source`, `trees`, `--switch-at`) load the same way; off runs it all before the first frame [on in a window, off headless] |
| `--loading-fade MS` | Fade the loading frame out over the first world frames; 0 cuts [250] |

With the screen on, the window's first frame is a still loading frame (the background, the
scene and an empty bar), drawn as soon as the device exists and before the frame and the
mirror are created; the turning loading frames follow. The castle's prefabs build beside the
window and the device, not before them.

`Esc` or the window's close during loading ends the run through the ordinary teardown; a
benchmark so ended reports `ended: "aborted"` with 0 frames. Under `--tester` the loading
frame's title is the path's scene and mode (`castle - benchmark`). `run.json` is schema 2:
`load_ms` (device ready to the first world frame) and `load_stages` (`[{name, ms}]`, each
stage's step time).

### Render settings

| Flag | Does |
|------|------|
| `--settings` | Print every render setting the flags give (`vxe_render::RenderSettings`, ADR-0055), one line per field: its name, its apply class (`live`, `settle`, `rebuild`) and its value; then exit |
| `--profile floor\|portable\|performance\|high` | The render profile (`vxe_render::Profile`) the other flags edit, whatever their order: floor (1080p, FSR 2 at 1.5), portable (720p, FSR 2 at 1.5, fewer layers and lights, no lamp bounce, a smaller residency), performance (the window's size, FSR 2 at 2) or high (1080p, FSR 2 at 1; not for the slow adapter); its output hint sizes a headless run with no `--render`. The HUD's first mode row reads `profile: portable +1` with one field edited [floor] |
| `--light-cap N` | Slots of the list of lights shaded, the headlamp's included, 1 to 16 [the profile's] |

`F4` (or the menu's `profile` entry, on the upscaler page) cycles the profiles in the window:
the next one's translucent layers, lamp bounce, light cap, internal scale and bake budget apply
from the next frame, and the profile row names the fields that wait for a restart with
`--profile` (`restart for: lod.r_full, build.mirror_budget_bytes, build.residency`).

### Diagnostics

| Flag | Does |
|------|------|
| `--load-slice MS` | The start-up step time a loading frame spends before it draws [25] |
| `--quit-at-loading-frame N` | Act as `Esc` at loading frame `N`, a test's way to end a run while it loads |

### Removed flags

Flags from earlier plans (`--steady`, `--sampler`, `--emissive-history` and others from Plan
0057; `--light`, `--preload` and others from Plan 0059) are refused with a message that names
the plan. `--help` lists them.

## Recording clips

Two scripts turn a model's clip into a GIF or MP4 for articles and the README. The models and
how to make one are in [entities.md](entities.md).

`scripts/record-clip.sh` renders headlessly: it builds the fly-through's `clip` example
(release, in a 6G memory-capped scope), renders numbered PNGs under the GPU lock into
`target/clip/frames/<rig>-<clip>/`, and encodes them with ffmpeg. Frame `n` shows the clip at
tick `n * rate / fps`, so a recording is the same on every run.

```sh
scripts/record-clip.sh                                    # the spider's first clip, a GIF
scripts/record-clip.sh --format both --width 800 -- --clip rear
scripts/record-clip.sh -- --rig apps/flythrough/assets/entities/walker/walker.rig \
  --clip walk --ahead 70 --orbit 24,3,-10 --aim 9          # the walker, close up
scripts/record-clip.sh --input target/clip/frames/spider-walk --crop 960:540:160:90
```

| Option | Does |
|--------|------|
| `--out BASE` | The outputs' path without extension [`target/clip/<rig>-<clip>`] |
| `--format gif\|mp4\|both` | A looping GIF (two-pass palette, ordered dither), an H.264 MP4 (yuv420p, faststart), or both [gif] |
| `--width W` | The encoded width, the height in proportion [640] |
| `--fps N` | Frames a second, rendered and encoded [25] |
| `--crop W:H:X:Y` | Crop the frames, in rendered pixels, before scaling |
| `--colors N` | The GIF palette's colours [192] |
| `--crf N` | The MP4's quality, lower is better [20] |
| `--input PATH` | Encode a directory of `frame-*.png` or a video instead of rendering |
| `--no-build` | Run the example as already built |

Everything after `--` goes to the example; `cargo run --release -p flythrough --example clip --
--help` lists its flags. The ones that frame a recording:

| Flag | Does |
|------|------|
| `--rig PATH`, `--clip NAME` | The model and its clip [the spider; the rig's first clip] |
| `--source`, `--shot`, `--seed` | The world, as the hero's [the showcase's `hall`] |
| `--place X,Y,Z` | The model's feet voxel [the floor `--ahead D` voxels (80) along the shot's heading] |
| `--facing DEG` | The model's yaw from facing -y [towards the shot's eye] |
| `--orbit DIST,HEIGHT,ANGLE` | The camera `DIST` voxels from the feet across the floor, `HEIGHT` over them, `ANGLE` degrees round from the model's front [60,12,10] |
| `--aim H` | The orbit looks at the point `H` voxels over the feet [14] |
| `--at X,Y,Z`, `--yaw`, `--pitch` | A fixed camera instead of the orbit |
| `--exposure auto\|E` | The exposure [a manual 10] |
| `--size WxH` | The rendered size [1280x720] |
| `--loops N`, `--seconds S` | One or more passes of the clip, or a time [1 loop] |
| `--settle N`, `--warmup N` | Frames drawn at each pose before it is kept, and before the first [16, 64] |

A loop is seamless when its ticks fill whole frames (`ticks * fps / rate` whole); the example
says so when they do not. The lighting is the hero's: lamp and bounce off, probes at 32.

`scripts/record-screen.sh` is the fallback for a scene that needs hands on the controls: start
the fly-through in a window yourself, then run it. It counts down in desktop notifications that
expire after 1.5 s, the last at `--quiet` seconds (3) before the recording starts so that none
is on screen at its first frame, dismisses any others (`makoctl`, when installed), records a
monitor with `gpu-screen-recorder` for `--seconds` (10), stops it with SIGINT and hands the
video to `record-clip.sh --input`. It never closes, moves or focuses a window. `--monitor NAME`
picks the monitor (the first `gpu-screen-recorder --list-capture-options` names by default),
`--countdown N` the countdown with the quiet gap in it (6), `--keep-raw` keeps the recording,
`--dry-run` prints the schedule and the commands and records nothing, and `--format`,
`--width`, `--fps`, `--crop` (in screen pixels, `W:H:X:Y`), `--colors` and `--crf` go to the
encode.
