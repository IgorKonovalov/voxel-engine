# ADR-0074: An entity carries a mark, drawn in the composite as an outline where the entity changes and a tint of its shaded colour

> **Status:** proposed
> **Amended 2026-11-18 (architect, at Plan 0077's Phase 5 checkpoint):** the hit record's
> instance field is a **part** slot (`INSTANCE_SLOT_BITS` = 9 over `MAX_ENTITY_PARTS` = 512),
> not an entity; an entity's parts take a run of slots (`entities/instances.rs`, `Run`). The
> body's "`MAX_ENTITIES` words indexed by the field" could not find a mark, and its edge test
> would have outlined every part boundary inside a model. The Decision now reads: two words
> per part slot (4 KiB, not 256 bytes) in a marks buffer of its own, the owner (the run's
> first slot) compared for the edge; Alternatives D to F record what was rejected with it.
> The body below is the amended text.
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

The renderer already has most of what an outline needs. ADR-0050 traces entities after the
world march into the same hit record as the world: `entity.wgsl` sets `HIT_ENTITY` and writes
the hit **part's** instance slot (`INSTANCE_SLOT_BITS` = 9, over `MAX_ENTITY_PARTS` = 512
slots) and its generation's low bit from `HIT_INSTANCE_SHIFT`, and `composite.wgsl` binds
`hit_info_in`, so every composited pixel knows which part it shows. It does not know which
entity: an instance is one posed part, an entity's parts take a run of consecutive slots
(`entities/instances.rs`, `Run { first, count }`, moved by compaction with their generation
bumped), and the record's 16 flag bits are full (six flags, the slot and the generation bit),
so the entity cannot ride in the record. The set draws at most `MAX_ENTITIES` (64) entities.
The composite binds no entity buffer (its entity group is `Group::Empty` in
`frame/pipelines.rs`). There is no translucency in `entity.wgsl`: an entity voxel is opaque or
absent, and the world's own translucent layers (ADR-0047) are resolved in the march before the
entity trace.

The need is general. A consumer that previews a placement, selects a creature, or flags one as
hostile or blocked wants the same facility with its own colour; the engine must not know a
ghost from a selection (`CLAUDE.md`: the engine knows no gameplay). And the colours must be
the user's to change (XAG 103's "assign their own color choice"; Satisfactory ships them as
settings), which ADR-0055's render settings already give a home.

## Decision

`vxe-render` gives every **part slot** two words in a **marks buffer** of its own,
`MAX_ENTITY_PARTS` x 2 x `u32` = 4 KiB: the slot's **owner**, its run's first slot (the one
value every part of an entity shares and no part of another has), and its **mark**. A
consumer sets marks by entity, `EntityMirror::set_marks(queue, &[(EntityId, Mark)])` with
`Mark { colour: [f32; 3], outline: bool, tint: f32 }`; the mirror keeps that list, and each
`EntityInstances::write` rewrites the owner and mark of every live slot from the runs and the
list, so a run compaction moves an entity's marks with its slots and an unlisted or unmarked
entity's slots carry no mark. The pair's bits: the owner's 9, the outline's 1, the tint's 8
and the colour's 8 a channel, 42 of 64 (a single word could not hold the 33 the mark alone
needs; the packing is the code's). The buffer is bound to the composite as one more entry of
its own group (`frame/{pipelines,compose,encode}.rs`), not by binding the entity buffer there.

`composite.wgsl` reads it through the hit record: the record's instance field masked to its
slot (the generation bit dropped) indexes the pair. A pixel whose record is an entity hit of a
marked slot mixes its shaded colour toward the mark's colour by `tint`; a pixel whose
**owner** differs from any of its four neighbours' (a non-entity pixel has no owner: an
entity against the world, or against another entity), on either side of that edge, takes the
mark colour of the marked side when `outline` is set. Two parts of one entity share an owner,
so no line is drawn inside a model. The edge is **a change of entity, not of depth**, so two
entities touching keep their edge, and an entity against a wall at its own depth keeps it too.
The line is one internal pixel, drawn before the upscale, so FSR 2 at 1.5
widens and softens it by its factor; if the window finds it smeared, the fallback is the same
neighbour test in the present pass over the hit record sampled at the internal resolution with
the frame's jitter removed, and that is a settings change, not a new mechanism. The colours a
consumer reaches for first are render settings (ADR-0055), the same in every profile: a blue
`[0.35, 0.65, 1.0]` and an orange `[1.0, 0.55, 0.15]`, the pair the colour-blind guidance names
and never green and red. `vxe-render` names neither a ghost nor a preview nor a selection: a
mark is a mark, and the consumer says what it means.

## Consequences

### Positive

- Two words per part slot (4 KiB of VRAM, at most 4 KiB written per instances write, beside
  the instances' own 64 KiB); per composited pixel four `hit_info_in` taps, up to five owner
  reads from a 4 KiB buffer that stays in cache, and a compare. The guess (Plan 0077, "Budgets
  added 2026-11-18") is under 0.1 ms on the RTX 3080 Laptop and under 0.6 ms on the RADV
  RENOIR at 1080p output under FSR 2 at 1.5, on the headroom line of `docs/nfr.md` section 2;
  Phase 6 reads it.
- No new pass, no new target, no new binding group: the composite already has the record, and
  the marks buffer is one more binding in the group it owns.
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
- An entity fully inside another's silhouette at a different depth has no edge pixel, so
  no outline, only the tint. Acceptable for a preview (nothing stands inside a creature);
  a consumer that needs it has the depth in `hit_pos_in` for a later ADR.
- The owner words are derived state rewritten every write, so a mark is only as fresh as the
  last `EntityInstances::write`; a `set_marks` between two writes shows at the next one, which
  is the frame it is called in (the fly-through writes every frame).
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
edge there; the record's instance field, through its owner word, tells entities apart at no
extra cost.

### Alternative D: one mark word per part slot, the mark itself as the edge test (2 KiB)

The dev's smaller cut at the Phase 5 checkpoint: no owner word, and an edge wherever two
neighbours' marks differ. Rejected because the edge becomes a change of **mark**, not of
entity: two touching entities marked alike (a multi-selection, a row of blocked ghosts) lose
their shared edge, which is the flaw Alternative C was rejected for, and the 2 KiB saved is
below the instances' own upload.

### Alternative E: the entity in the hit record

`entity.wgsl` writing an entity index (6 bits for `MAX_ENTITIES`) into the record instead of,
or beside, the part slot, so the composite needs no owner lookup. Rejected because the
record's 16 flag bits are full (`6 + INSTANCE_RECORD_BITS <= 16` in `entities/instances.rs`)
and the part slot with its generation bit is the temporal passes' freshness key (ADR-0050): a
reused slot must differ between two frames, and an entity index does not say which part moved.
Widening the record is a change to every pass that reads it, for a lookup a 4 KiB buffer does.

### Alternative F: the marks inside the entity buffer

The owner and mark words as one more region of the entity mirror's buffer, "beside the
instances" as the first draft said. Rejected because the composite does not bind that buffer
(its entity group is empty) and binding the whole pool, 8 MiB by default, to read 4 KiB adds a
large binding to the pass that draws every pixel; a buffer of its own is a `Res::Buf` in the
group the composite already owns.

## Outcome

_(Added at acceptance if Plan 0077's Phase 6 reading or Phase 8 window falsifies something
above.)_
