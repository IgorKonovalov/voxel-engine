# ADR-0031: Weather is cosmetic client state stepped by the game's ticks, and rain, snow, wetness and snow cover are rendered against a fall map traced through the mirror, never written as voxels

> **Status:** proposed
> **Date:** 2026-10-22
> **Related plan(s):** [Plan 0036](../plans/0036-weather-rain-snow-wind-wet-surfaces-snow-cover-lightning.md)

## Context

The owner asked for full dynamic weather (2026-10-22): rain, snow, wind, wet surfaces, snow
cover and lightning, driven by the game through a small API, with time of day and weather as
parameters the game sets. Two constraints frame it. `vxe-world` is headless and deterministic
and reads no wall clock (CLAUDE.md); the outdoor plans of this batch keep sky and weather on
the rendering side, cosmetic and client-side unless a plan argues otherwise. And the first
consumer is underground: the one property weather must get right is that a cave stays dry.
Rain inside a cave mouth, or snow on a floor under an overhang, is the failure a player sees
first.

So every weather effect needs the same answer to one question: is this point reached by what
falls from the sky? A particle under a ledge must vanish, a floor under a roof must stay dry,
a cliff face on the windward side must get wet. The answer depends on the world's geometry,
which changes at runtime (digging opens a roof, collapse closes one), and on the wind, which
slants the fall. The world lives twice: authoritative in `vxe-world` (CPU, 10 cm voxels, a
per-brick coarse grid at 0.8 m) and mirrored in `vxe-render` (the GPU brick pool, walked by the
full walk at about 0.9 ms per million rays at 1080p on the dev box, Plan 0015).

Snow cover could also be matter: voxels of a snow material stamped onto exposed tops. That
would make it collidable and diggable, but every snowfall would be an edit through the op log
on every exposed chunk in the view.

## Decision

Weather is cosmetic client state. `vxe-render`'s `weather` module holds a `WeatherState` the
game sets as a target: rain and snow rates, a horizontal wind vector, wetness and snow-cover
levels, the cloud coverage it hands to Plan 0035's clouds, and a lightning rate. A `Weather`
value approaches that target linearly over a tick count the game names. The game advances it
with `Weather::step(ticks)`, and the engine reads no clock. Lightning strikes are scheduled from
an explicit seed and the tick number by an integer hash, and the game can also force a strike.
`step` reports each strike as a `WeatherEvent::Lightning` with its direction, distance and
intensity. Thunder is the game's sound, so no weather type names `vxe-audio`.

The renderer answers "reached by what falls?" with a **fall map**. The map is a camera-centred
1024 x 1024 grid at 0.2 m, covering 204.8 m, in tiles of one chunk column. A compute pass traces
one ray per texel through the far field and the mirror along the quantized fall direction, and
stores the height of the first solid voxel it meets. Only dirty tiles are re-traced: those an
edit box or a streamed chunk touched, those that scrolled in, and all of them, into a second map,
when the quantized wind slant changes. Particles, wetness, puddles, snow cover and the lightning
flash all compare a point's height against this map. Precipitation is drawn as stateless
particles: their positions are a hash of their index plus the wind-integrated fall offset,
wrapped in a world-aligned box around the camera. They are drawn after the upscaler, depth-tested
against the hit buffer, and dropped where the map says covered. Wetness, puddles and snow cover
are terms in the composite on the exposed faces. Weather never writes a voxel. A game that needs
weather in gameplay (a sheltered check, snow that blocks, thunder that alerts) owns that state
in its own deterministic logic and sends the targets and strikes to its clients. It queries
`vxe-world` for gameplay facts and never reads the fall map, which is a cosmetic GPU artifact.

## Consequences

### Positive
- One occlusion source serves all the effects, so rain, wetness and snow agree about where the
  roof ends. Its edges are 0.2 m texels compared with a 2 x 2 filter, smooth enough for the
  owner's "no Lego" bar.
- Edits and streaming cost almost nothing: an edit marks the tiles under its box, and the next
  frame re-traces up to 64 tiles (65 536 rays, a projected 0.06 ms on the dev box).
- Particles need no buffer and no simulation. The same seed and tick give the same image, so
  goldens are stable and two clients with one seed see the same flashes at the same ticks.
- `vxe-world`, the op log, edit persistence and the network are untouched. Weather adds no
  bytes to a save and no ops to a log.

### Negative
- The map sees only what the mirror and the far field hold. Above the far field's vertical
  extent nothing is known, so a column whose trace starts in air reads as open sky. A deep cave
  under an air pocket above that extent would get rain. The game can set the rates to zero
  where it knows better.
- The map is 2.5D. A point counts as exposed if nothing lies above it along the fall line, so
  rain does not blow sideways into a cave mouth beyond the slant. The slant is capped at
  `tan = 0.625` (32 degrees) and quantized to steps of 1/8, so a gust's exact angle is lost. A
  slant change re-traces the whole map over 16 frames at the default cap.
- Snow cover has no thickness. It recolours and roughens faces but cannot round a step or bury
  a pebble, and nothing collides with it.
- The flash has no shadow ray toward the strike. It lights what the fall map calls exposed,
  shaded by the normal, so a hill between the strike and a valley does not shade the valley.
- 16 MiB of VRAM (two maps of `rg32float`) for an effect a cave-only game may never switch on.
  It is allocated only when weather is enabled.

## Alternatives considered

### Alternative A: snow and wetness as voxels in `vxe-world`
A snowfall stamps snow-material voxels onto exposed tops, and wetness is a per-voxel flag.
Accumulated snow becomes real terrain. It lost because it puts a cosmetic effect into the
authoritative, deterministic, logged world. Every exposed chunk in a 200 m view becomes an
edited chunk, which pushes the edit-persistence RAM line (`nfr.md` section 3) and the op log
and the mirror's upload line (section 2's 1.0 ms streaming line, already a recorded miss) to
carry weather. A 10 cm voxel is also too coarse for a few centimetres of snow, which is the
look the owner rejects. A game that wants drifts as terrain can stamp them with ops itself.

### Alternative B: an exposure height map on the CPU from `vxe-world`'s coarse grid
Derive each column's top occupied height from the per-brick coarse grid (Plan 0004) and upload
it. It is deterministic and could serve gameplay. It lost on resolution and reach. A coarse cell
is 0.8 m, so drip lines become a 0.8 m staircase. The grid is maintained only where the
consumer builds it and holds no LOD-only chunk. Refining it to voxels on the CPU means walking
up to 64 voxels per column across about a million columns per rebuild on the render thread.

### Alternative C: a per-pixel and per-particle ray toward the sky
Trace one more ray up the fall direction from every hit pixel, and from every particle. It is
exact, with no map, no tiles and no slant quantization. It lost on cost and noise: one more
full-walk ray per pixel is about 1.9 ms at 1080p on the dev box (2.07 M rays at 0.9 ms per
million) against the map's 0.06 ms per frame. Its result would also need denoising, or would
shimmer at drip lines.

### Alternative D: screen-space rain layers
Scroll rain textures on a few depth layers around the camera (the cone-and-layers technique of
Tatarchuk's 2006 rain work), occluded per pixel by a depth lookup. It is cheap and has a fixed
cost. It lost because snowflakes on flat layers show no parallax when the camera moves, and
occlusion is only correct per layer, not per flake. That fails exactly at a cave mouth, the
scene this ADR is for. Stateless particles cost about as little and keep every flake in the
world.
