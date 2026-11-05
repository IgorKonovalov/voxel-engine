# Backlog

Ideas, parked questions and owed follow-ups that no plan, ADR or roadmap milestone carries yet.
This is the owner's parking lot: anything said in a session that is "later" lands here, dated,
so it is visible in the tree instead of in an agent's memory.

Rules:

- **One entry per idea**, dated when it was parked, with the context it came from, what it would
  take, and the condition for picking it up. Keep an entry under about ten lines; the long form
  goes in the plan or ADR that eventually takes it.
- **An entry leaves when a plan or ADR takes it.** Replace its body with one line naming the plan
  or ADR, or delete it when the owner drops the idea. The roadmap still owns the milestones; an
  idea that becomes a milestone item moves there.
- **Anyone edits this file**, by hand, in the session the idea comes up. The architect reads it at
  every plan interview and close review and asks which entries are ripe.

## Engine mechanisms

### Chunk-scale world dynamics: a cellular material step and an event scheduler

Parked 2026-11-03 and 2026-11-04. The owner wants Noita or Teardown style dynamics (sand, flowing
liquids, fire, reactions) and "large world events" that run over many chunks. Today the storage
layer is ready and the simulation layer is absent: 64³ chunks behind per-chunk locks, two-tier
streaming residency, every op returning its touched chunks, the coarse grid updated from that
result, and a server tick that only changes the world when an op is applied. Nothing revisits
chunks over time, there is no active set or dirty-chunk scheduler, and collapse (ADR-0008) is
pull-based and per edit.

Feasibility read (2026-11-03): powders, liquids, fire and material-pair reactions fit as one
deterministic op kind, "step N with seed S", through the op log, over resident chunks only, with
a brick checkerboard for order and a per-chunk dirty bitmap as the active set; cost is set by
active voxels, not resident ones, and the GPU mirror's re-upload of every active brick may
starve before the CPU does. Rigid bodies and rotating pieces conflict with ADR-0008 and stay on
the cosmetic client-side layer. Gas as voxels conflicts with M8's separate coarse volume.

What it takes: an ADR that generalises M10's "flowing liquids" item into the cellular step plus a
per-tick chunk scheduler (active set, per-tick budget, resident chunks only; rejected: GPU-side
authoritative simulation, a rigid-body engine), then a plan with a headless proof (a sand column
released over a pit lands as the same pile with the same hash on both CI arms) and a per-tick
reading against a new `nfr.md` line.

Pick up when the lighting base is settled: Plans 0056, 0057 and 0058 closed.

### Per-source indirect light

Parked 2026-10-22 (Plan 0029 live session 11), reaffirmed 2026-11-04 (Plan 0051 verdict). The
owner wants bounce light only from some sources: "fire on a stick or fire in a dungeon" feed the
probes, the headlamp and simple lights are direct only. ADR-0043 shrinks the probe volume to a
lamp-bounce channel and keeps its logic for exactly this later redesign; ADR-0044 gives lights a
fixed list. The missing piece is a per-light and per-emissive-material "feeds the bounce channel"
flag and the channel's redesign. Pick up after Plan 0058 closes and the owner has seen the
steady image.

### Render settings surface and named quality profiles

Parked 2026-10-22. Every mechanism choice (shadow walk, lighting tier, sampler, look, upscaler,
later sky, clouds, far field) should be a documented engine setting a consumer picks, with named
profiles (floor, portable, performance) the roadmap promises. Today `Frame` exposes lighting,
resolution, look and LOD at runtime; construction-time settings (mirror budget, residency) are
separate and live-versus-restart is undocumented. What it takes: a best-practices rule that a new
mechanism lands as a documented setting, and a plan for the settings surface and the profiles.
The 2026-11-03 answer on M10's RADV budget leaned on this plan coming before Plan 0032.

### Far normal range: the owner picked `[1, 2]`

Plan 0055 shipped `[2, 6]` as a provisional pick. Plan 0056's window (2026-11-04) picked `[1, 2]`
by eye, against a headless reading of about 7 times `[2, 6]`'s still jumps under FSR 2, and
named the edge dots left at `[1, 2]` the next aliasing item. Owed: a plan whose first phase
makes `[1, 2]` the default with ADR-0042's Outcome, then the edge dots, and removes the emitter
cap knob (Plan 0056's Followups). Draft it with the antialiasing plan at Plan 0059's Phase 3 verdict;
Plan 0059's reference runs at `[1, 2]` by flag meanwhile.

### The table's edges, if the owner's verdict keeps it

Parked 2026-11-05 at Plan 0057's close. All of this goes if Plan 0059's Phase 13 picks the bake, and
is owed if it picks the table: a frame over `MAX_EDITS` clears the whole table (the vein's `B`
presses, 78 to 87 carves); a cell out of view past the guard's 256-frame window stays stale; a
capped cell can show a blend word from before a neighbour's last change when it enters view; the
parity skip can age out a far cell only an off-parity cell's kernel reads; the walk's slice frames
were never read on their own; Radeon lighting reads over the 6.0 ms line walking, the blend adding
0.64 to 1.38 ms. Pick up at that verdict.

### The lamp channel's faults

Parked 2026-11-05 at Plan 0057's close. `the_lamp_furnace_reads_its_albedo_times_the_direct_mean` takes
its `D` from the shader's output, so it does not defend the albedo: `D` wants an independent reading
of the lamp's irradiance at the rays' hits. Inside probes are traced while the lamp moves
(`probe_update.wgsl`'s `probe_idle` returns on `MOVED_BIT` before its inside check): cost only, unread on
the RTX. `a_lamp_step_moves_the_probes_in_reach_only` checks one probe. Pick up with Plan 0058's Phase 4.

### Slow-pan verdict for FSR 2 (ADR-0034)

The slow pan ADR-0034 named as FSR 2's verdict ran in Plan 0056's window (2026-11-04, steps 2
and 5b), and the owner's word was "FSR 2 at 1.5 stays". Owed: ADR-0034's dated Outcome, written
with the far-normal plan above.

## Tooling and process

### Shader toolchain spike

Parked 2026-10-31. Compare naga's SPIR-V against `spirv-opt -O` on the hit shader (VGPRs,
milliseconds, both adapters), then try wgpu subgroups and `shader-f16` on the walk. Rust versus
C for the CPU side was asked and answered no (GPU about 9 of 10.6 ms, CPU about 1 ms). Run when
no lane holds the GPU.

### `castle_gate_golden`'s RSS bar measures little of the castle

Read 2026-11-05 on 7cd77ad, which keeps the shader compiler out of the peak with a warm-up render in a child process (about 1 002 400 000 B cold and warm; the bar stays at 1 GiB). Of that peak, 655 MiB is shared memory, and the small pillar world's frame reads the same 655 MiB, so the bar mostly measures the frame. The rest is about 222 MiB of heap and 100 MiB of file pages. What makes up the 655 MiB was not read. The bar still fails with `MESA_SHADER_CACHE_DISABLE=true` (1 172 004 864 B), and CI's cold run has not been seen. What it takes: decide what the bar should defend, then read that quantity. Pick up with Plan 0044 or at the first red run from it.

### The gate tier is over its wall-time budget

Read 2026-11-05 on 7cd77ad (`docs/nfr.md` section 7): 1313.4 s, about 22 minutes, loaded, against the 12 min proposal, with the two tests then over the 30 s line already on the soak list. `flythrough::settle` is the largest binary at 338 s. Owed: rule on the four tests that read just over the line in that run (section 7 names them; `castle_gate_golden` only on a cold cache), then either cut the slowest `settle` and `golden` tests or re-set the proposal. Much of `settle`'s time goes with the table at Plan 0059's deletion, so re-read then. `the_lamp_toggle_opens_the_light_window` read 98.0 s on CI's last red run, on an older tree, and reads 18.0 s here, so it stays in the gate; CI's next run shows whether it is still over the 60 s line. Pick up as a dev chore when the owner next wants the gate faster or CI green.

## Later apps

### Web demo as a separate app

Decided 2026-10-16: a WebGPU and wasm demo for blog interactivity is a later, for-fun item and
its own app under `apps/`. The fly-through and the native paths never change or lose performance
for it; any crate support is target-gated. Not before the engine has steady lighting, coloured
materials, a sky and a way to build interesting architecture.

### Antialiasing after the look-first light

Decided 2026-11-05 (owner, Plan 0057's second window): "antializing next", after the emissive-only
reference of the look-first Plan 0059. The owner's fast turn "still has artifacts" (step 1); the
reference runs with no upscaler, so it tells the light's part from the image's. Picks up when
the owner has judged the reference.
