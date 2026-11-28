# ADR-0090: Every look feature has a floor form and a rich form, and a rich profile holds its own frame budget on a named card

> **Status:** proposed (draft awaiting the owner's approval)
> **Date:** 2026-11-28
> **Related plan(s):** [Plan 0112](../plans/0112-the-rich-profile-and-the-voxels-colour-in-layers.md)
> (builds the profile and its budget lines); the amendments of 2026-11-28 to Plans
> [0033](../plans/0033-far-field-clipmap-to-a-kilometre.md),
> [0034](../plans/0034-water-surface-waves-reflection-refraction-underwater-fog.md),
> [0035](../plans/0035-aerial-perspective-and-volumetric-clouds-with-cloud-shadows.md),
> [0036](../plans/0036-weather-rain-snow-wind-wet-surfaces-snow-cover-lightning.md),
> [0037](../plans/0037-night-sky-stars-moon-phases-sky-bodies-and-aurora.md) and
> [0081](../plans/0081-a-surface-detail-layer-blades-caps-and-pebbles-marched-above-styled-faces-swayed-by-wind.md);
> Plans [0113](../plans/0113-overgrowth-by-shape-a-seeded-decoration-pass-over-generated-chunks.md),
> [0114](../plans/0114-the-ruins-demo-world-canyons-a-lake-and-overgrown-ruins-in-apps.md) and
> [0115](../plans/0115-vegetation-forms-clumps-vines-and-flowers-as-voxels-with-a-rich-detail-form.md)

Drafted on Opus 2026-11-28 (Fable exhausted for the week); a Fable second pass reviews and fixes
this draft after the weekly reset (owner).

## Context

The owner named a target look on 2026-11-28: overgrown voxel ruins in canyons, a long hazy
view, dense foliage, reflective water (`docs/references/local/wbk-voxel-2026/`, wbk's Vulkan
renderer, 4K at about 130 fps on an RTX 5090). He asked how to reach it when our baseline is
thinner: "make everything that we can do now, haze and reflections and water, greenery etc,
and then maybe increase performance budget?"

The floor (`docs/nfr.md` section 1) is a 2019 mid-range desktop card, GTX 1660 / RX 5600 class,
6 GB, no RT hardware, 60 fps at 1080p output with FSR 2 at internal scale 1.5, held as a split
of 16.6 ms (section 2). The baseline card is not acquired; the floor's figures are bracketed
by the RTX 3080 Laptop above it and the Cezanne iGPU below it. The RTX reads a Floor frame of
about 2.9 to 6.8 ms today on the testbed's scenes, so on the card the owner works and judges
on, two thirds of the frame is unspent. ADR-0055 gave four named profiles (Floor, Portable,
Performance, High). None of them spends more on the look: Performance and High change only the
internal scale.

The M10 plans (0033 to 0037, 0081) each carry one form of their feature and a set of switches,
and each states its cost against the Floor lines. The target look's costly traits (foliage as
dense small-voxel clumps, traced water reflections, a full-rate cloud march) do not fit the
floor's 2.6 ms of headroom together. Either they are cut to the floor, the floor rises, or a
profile above the floor gets a budget of its own.

## Decision

The floor's 16.6 ms budget and its card stay as they are. **Every look feature from Plan 0033
on ships in two forms:** a **floor form**, the default, which fits the Floor lines and is the
one the Floor and (where it fits, else off) the Portable profile apply; and a **rich form**,
which a new fifth profile, provisionally `Profile::Rich`, applies. The rich profile has its
**own frame budget on a named card**: 16.6 ms at 1080p output with FSR 2 at internal scale
1.5, read on the **RTX 3080 Laptop** (the card on hand, so every reading is a measurement, not
a projection), split as Plan 0112 sets it in `docs/nfr.md` section 2. Each look plan states,
in its Budgets section, a **cost per profile** table: the form each profile applies and its
reading against that profile's line. Three rules bound the rich form:

1. **A profile never changes the world.** Forms differ only in `svet-render` and in what the
   renderer draws of the same voxels. Generation output, the op log, queries, collision and
   audio are identical under every profile; a generation-time pass (Plan 0113) has no profile.
2. **A rich form passes the same stillness rules as the floor form** (ADR-0076, the owner's
   "stable image first"); it is judged in the same owner window as the floor form, and a rich
   form that cannot hold still is not shipped as a default of any profile.
3. **The rich form is a setting, not a branch.** It is one or more `RenderSettings` fields
   whose values the rich profile picks (ADR-0055); a consumer may raise one feature alone on a
   Floor base. Nothing chooses a form by adapter.

A feature whose floor form is "off" (for example the traced reflection, whose floor form is the
sky's reflection) states so in its table; a feature with one form that already fits the floor
(for example aerial perspective's volume) states the same form twice.

## Consequences

### Positive
- The target look gets a home that is measured on hardware we have, without moving the floor
  under the first consumer or the tester's slow-adapter readings.
- Cheap features are built first and reach every profile; costly ones arrive with their cheap
  form beside them, so no plan ends with a look only the rich profile can show.
- "Then maybe increase performance budget" becomes a decision with numbers: the rich profile's
  readings say what raising the floor would buy, before anyone raises it.
- The cost per profile table makes the product's cost visible per plan, which ADR-0055's
  profile rows began.

### Negative
- Two forms per feature is more code, more settings rows and a window that judges both, roughly
  a third more work per look plan (a guess).
- The rich profile's card is a laptop part in a dev box: its readings vary with power and
  thermals, and it is not a card a player buys. Readings name the power state, as section 2's
  RTX readings do.
- A rich form that the floor never runs can rot: Plan 0110 and later perf work measure the
  floor first. The rich profile's lines are checked at each look plan's close, not in CI.
- Rule 1 rules out a denser world for the rich profile (more trees, more clumps placed); the
  rich profile can add only drawn detail on the same voxels.

## Alternatives considered

### Alternative A: raise the floor

Move the floor to a newer card (RTX 3060 / RX 6600 class) or relax it to 30 fps, so every
feature has more room. It lost because the floor is the first consumer's player base and the
tester's fixed point: every budget line in `docs/nfr.md` since M1, and every reading
bracketed by two adapters, would be re-based at once, the slow adapter would fall further
below it, and the owner asked to build what fits now and then decide. The rich profile's
readings are the evidence a later raise would need; this ADR keeps that door open.

### Alternative B: one form per feature, cut to the floor

Each feature ships only what fits the floor (sky reflection only, foliage as world voxels only,
clouds at the floor's update rate), and the surplus of faster cards goes to internal resolution
through the High profile. It lost because the target look's traits that carry it (traced
reflections of the cliffs, dense foliage detail) would never be built, and a sharper image of
the floor's look is not the look the owner named.

### Alternative C: an adaptive quality that fills the frame on any card

The renderer measures its frame and raises or lowers each feature's form to fill a target. It
lost for the reasons ADR-0055 rejected adapting at run time: a form that moves frame to frame
moves the image, against "stable image first", and no reading would name what a profile costs.

## Owner answers (2026-11-28)

The owner answered the draft's eight questions, the recommended option in each: the card is the
RTX 3080 Laptop; the Rich budget is 16.6 ms at 1080p output with FSR 2 at 1.5 (the split stays a
guess until read); a profile never changes the world; `Rich` is a profile of its own, not folded
into `High`; Floor's water reflects the sky with a screen-space arm tried in the window; the demo
world lives in `apps/ruins`; Plan 0035's haze brief runs before Plan 0033's render half; Plan
0033 reads a fourth level for Rich. The ADR stays proposed until the Fable second pass and the
owner's go on the plans that carry it.

## Outcome

_(Added only at acceptance if implementation falsified something above.)_
