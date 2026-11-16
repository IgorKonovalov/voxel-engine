# ADR-0068: An entity's variants are recipes in its model: a volume per part and a palette per variant, picked per instance by one index

> **Status:** proposed
> **Date:** 2026-11-16
> **Related plan(s):** [Plan 0085](../plans/0085-entity-variants-holding-clips-and-an-entity-stamped-into-the-world.md)

## Context

The owner asked (2026-11-16) how an entity changes state: "the spider is healthy, then damaged,
then dead - is it 3 separate entities or one with modes?" The engine knows no gameplay, so
"healthy" and "damaged" are the game's words; what the engine owes is a mechanism by which one
entity, keeping its identity, its pose, its clips and its history on screen, can look and hit
differently: a part gone (a torn-off leg), a part's volume replaced (a broken leg), a material
replaced on every voxel that carries it (a charred shell).

Four facts shape the choice. ADR-0050 draws a model as one GPU instance per part, 128 B, and the
instance names its volume by a word offset into the pool (word 24); the volume's header names
its 256-entry palette of material ids, which is shared by every instance of every model that
uses that prefab (Plan 0062's dedupe by content id). The record has one spare word (31) and
thirty free bits in its flags word (26); Plan 0078 takes word 25 for the slot and its
generation. The headless query (`EntitySet::raycast`) walks every part of an entity's model with
no per-part state and reads the material straight from the prefab, and it is the server's
authority on what a ray hit, so whatever the picture does the query must do the same. And a
model's part count is what Plan 0078's stable instance slots and the temporal history key on: a
change that alters the part count costs the entity its slots and its pixels a frame of history.

## Decision

A **variant** is a recipe declared in the model (and so in the `.rig`): for each part, hidden or
drawn, and which volume it draws, its own or one of the model's **alternates** (extra volumes
from the same `.vox`, with a pivot, at the part's detail); and a list of **material remaps**
`(from, to)` applied to every voxel of every part. An entity carries one `u8`, its variant index
(0 is the base model), set at runtime (`EntitySet::set_variant`). The part count never changes:
a hidden part keeps its index, its pose and (under Plan 0078) its instance slot, flagged hidden
in the instance's flags word so the walk skips it and its box leaves the entity's box. A swap
changes the instance's volume word. A remap is resolved **at upload**, not at the pixel: for each
variant and each volume whose palette holds a remapped material, the mirror writes one more
256-word palette with the substitution made, and the instance's spare word names the palette it
reads; the base variant names the volume's own. The headless query applies the same recipe: it
skips a hidden part, walks the chosen volume and maps the hit's material through the remap list,
so the server and the picture name the same part and the same material. At variant 0 every
result is bit-identical to today's.

## Consequences

### Positive
- One entity through every state: its `EntityId`, pose, playback, slots and screen history
  survive a variant change; the change costs one `u8` on the CPU and a rewrite of the instances
  the frame writes anyway.
- The instance record stays 128 B; the per-pixel cost of a variant is one flags read per part
  visited (the hidden check) and the same palette read as today from another address.
- Variants compose: a hidden leg, a swapped leg and a charred shell are one variant or three,
  as the author likes, with no combinatorial set of models in the pool.
- The remapped material is a real material row, so an emissive remap glows and a charred one
  reads through the same table the world reads; the headless hit reports the remapped id, which
  is what a game's rules want to know.

### Negative
- **Pool bytes per variant.** Each alternate is a full volume, and each remapping variant costs
  1024 B per volume whose palette it touches: a fourteen-part model with twelve distinct
  volumes pays about 12 KiB for every variant that remaps a shared material. Plan 0085 reads it
  on the spider.
- **The last spare instance word is spent.** The next per-instance need grows the record past
  128 B or packs into word 26's free bits.
- **A remap set is fixed at upload.** A runtime remap the `.rig` did not declare needs a new
  variant, a re-upload of the model and `free_model` of the old (ADR-0060); Alternative C would
  have allowed it for a per-pixel cost.
- **An alternate must fit the part's frame.** Its grid shares the part's pivot (or states its
  own) and detail; an author who draws a stump in a different box places its pivot by hand.
- A hidden part still occupies an instance slot and still gets a pose sampled for it, so the
  CPU composition cost of a model does not fall when parts are hidden.

## Alternatives considered

### Alternative A: a model per state, swapped by `EntitySet::replace`
The game uploads `spider`, `spider_damaged` and `spider_dead` as three models and swaps them
under one `EntityId` (Plan 0078's `replace`). No new engine concept. It lost because the pool
pays every part again per state (the fourteen-part spider about 225 KiB a copy against about
45 KiB for its variants), the states multiply (a torn leg times a charred shell is four
models), a swap that changes the part count loses the slots and a frame of history, and the
hit record of the frame before names volumes the swap freed. It stays the route for a change
that is a different model (a chrysalis, a pile of parts), which Plan 0085 documents.

### Alternative B: a palette per instance
Every part instance carries its own 1024 B palette in the pool, written at upload and rewritten
on a remap. The simplest shader. It lost on bytes and churn: 512 instances times 1 KiB is
512 KiB reserved for a feature most instances never use, a remap rewrites a palette per part at
runtime through the queue, and two walkers of the same model stop sharing anything.

### Alternative C: remap pairs resolved at the pixel
The instance names a short list of `(from, to)` pairs in the pool and `entity_part` loops over
them after the palette read. Arbitrary runtime remaps with no upload. It lost because it adds a
loop and up to eight compares to every entity hit and every entity shadow hit on the slow
adapter, where Plan 0062 read 64 walkers at +32 to 34 ms already, for a flexibility the `.rig`
can declare up front.

### Alternative D: hide a part by not writing its instance
The frame writes fewer instances and a shorter leaf run, no shader change. It lost because Plan
0078 gives an entity a contiguous run of slots sized by its part count, keyed by `(entity,
part)` for the frame before's transform and the history; a run that shrinks and grows as parts
hide and show would reallocate slots and bump generations on every change, costing the history
the flag keeps for free.

## Outcome

_(Added at acceptance if Plan 0085 falsifies something above.)_
