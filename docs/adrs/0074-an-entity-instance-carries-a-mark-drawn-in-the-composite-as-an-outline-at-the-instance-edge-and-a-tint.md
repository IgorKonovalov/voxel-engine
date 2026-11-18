# ADR-0074: An entity instance carries a mark, drawn in the composite as an outline where the instance changes and a tint of its shaded colour

> **Status:** proposed
> **Date:** 2026-11-18
> **Related plan(s):** [Plan 0077](../plans/0077-an-entity-picker-in-the-fly-through-choose-a-model-from-the-library-and-place-it.md) (Phase 6)

## Context

Plan 0077's first owner window (2026-11-18) found that the fly-through's placement preview,
"a normal instance in the set, drawn and lit as any other" under the plan's original decision,
cannot be told from a placed entity. The owner asked for "some form of border around" it, and
answered question 12 with an outline **and** a tint: at a distance where a one-pixel line
vanishes the tint still reads, and up close the line reads as a border. The pair also follows
Xbox Accessibility Guideline 103 (colour alone never carries information; the outline is a
shape cue) and Satisfactory's hologram, whose invalid state carries an outline and a fill at
once (`docs/references/entity-placement-ux.md`, "The ghost", "Outlines", "Colour").

The renderer already has what an outline needs. ADR-0050 traces entities after the world march
into the same hit record as the world: `hitrec.wgsl` sets `HIT_ENTITY` and writes the
instance's slot (`INSTANCE_SLOT_BITS` = 9) and its generation's low bit from
`HIT_INSTANCE_SHIFT`, and `composite.wgsl` binds `hit_info_in`, so every composited pixel knows
which instance it shows. The set holds `MAX_ENTITIES` (64) instance slots. There is no
translucency in `entity.wgsl`: an entity voxel is opaque or absent, and the world's own
translucent layers (ADR-0047) are resolved in the march before the entity trace.

The need is general. A consumer that previews a placement, selects a creature, or flags one as
hostile or blocked wants the same facility with its own colour; the engine must not know a
ghost from a selection (`CLAUDE.md`: the engine knows no gameplay). And the colours must be
the user's to change (XAG 103's "assign their own color choice"; Satisfactory ships them as
settings), which ADR-0055's render settings already give a home.

## Decision

`vxe-render` gives every entity instance slot a **mark word**, written by
`EntityMirror::set_marks(queue, &[(EntityId, Mark)])` with `Mark { colour: [f32; 3], outline:
bool, tint: f32 }`, into a region of `MAX_ENTITIES` words beside the instances (an unlisted
instance is unmarked; the colour is quantised to 8 bits a channel and the tint to 8 bits, so
one word holds a mark). `composite.wgsl` reads it through the hit record: a pixel whose record
is an entity hit of a marked slot mixes its shaded colour toward the mark's colour by `tint`;
a pixel whose record's instance field differs from any of its four neighbours' (an entity
against the world, or against another instance), on either side of that edge, takes the mark
colour of the marked side when `outline` is set. The edge is **a change of instance, not of
depth**, so two instances touching keep their edge, and an instance against a wall at its own
depth keeps it too. The line is one internal pixel, drawn before the upscale, so FSR 2 at 1.5
widens and softens it by its factor; if the window finds it smeared, the fallback is the same
neighbour test in the present pass over the hit record sampled at the internal resolution with
the frame's jitter removed, and that is a settings change, not a new mechanism. The colours a
consumer reaches for first are render settings (ADR-0055), the same in every profile: a blue
`[0.35, 0.65, 1.0]` and an orange `[1.0, 0.55, 0.15]`, the pair the colour-blind guidance names
and never green and red. `vxe-render` names neither a ghost nor a preview nor a selection: a
mark is a mark, and the consumer says what it means.

## Consequences

### Positive

- One word per instance slot (256 bytes), four `hit_info_in` taps and a compare per
  composited pixel. The guess (Plan 0077, "Budgets added 2026-11-18") is under 0.1 ms on the
  RTX 3080 Laptop and under 0.6 ms on the RADV RENOIR at 1080p output under FSR 2 at 1.5, on
  the headroom line of `docs/nfr.md` section 2; Phase 6 reads it.
- No new pass, no new target, no new binding group: the composite already has the record.
- Several marks of several colours in one frame (a ghost in blue, a blocked ghost in orange, a
  selected entity in a consumer's colour), as a stencil index gives an editor's multi-colour
  outline, and no depth-mask flaw where two marked instances overlap.
- The marked instance is still a real instance: lit and shadowed where it stands, so a
  preview shows what the placement will look like.

### Negative

- The line's width is the internal pixel, so it is one output pixel at internal scale 1 and
  about 1.5 at 1.5, softened by the upscaler's history; a width in output pixels at any scale
  is exactly what the jump flood buys and this does not. The owner judges it in Plan 0077
  Phase 8; the present-pass fallback is named above.
- An instance fully inside another's silhouette at a different depth has no edge pixel, so
  no outline, only the tint. Acceptable for a preview (nothing stands inside a creature);
  a consumer that needs it has the depth in `hit_pos_in` for a later ADR.
- The mark is cosmetic and client-side: nothing of it enters the op log or the entity set.
  A consumer that wants a mark to mean something keeps the meaning on its side.
- `composite.wgsl` grows by a branch every pixel takes (the entity test on the record) even
  when nothing is marked; the reading above says what it costs.

## Alternatives considered

### Alternative A: a jump-flood outline

A silhouette mask of the marked instances, a jump flood over it (log2 of the width in passes)
and a distance read, giving an exact anti-aliased outline of any width in output pixels (Ben
Golus; Ameye's "really wide outlines, at a very reasonable performance cost"). Rejected
because a ghost's border needs one or two pixels, which the neighbour test gives in the pass
that already has the record, and the flood costs several passes and two targets for a width
we do not want.

### Alternative B: a translucent ghost

The preview drawn see-through, as Valheim's and Space Engineers' ghosts read on screen.
Rejected because `entity.wgsl` has no translucent voxels (an entity hit is opaque), so it
would need a second trace through the entity for what is behind, and it would read against
the world's own translucent layers (ADR-0047), which are resolved before the entity trace; the
owner asked for a border, and a translucent body carries no shape cue at a distance.

### Alternative C: an outline from a depth mask

The classic custom-depth post process (Looman): a pixel with no depth whose neighbour has one
is the line. Rejected because a depth-only mask cannot separate two marked objects that
overlap or touch, and a marked instance standing against a wall at its own depth loses its
edge there; the record's instance field tells instances apart at no extra cost.

## Outcome

_(Added at acceptance if Plan 0077's Phase 6 reading or Phase 8 window falsifies something
above.)_
