# Plan NNNN readings log

> Written by `dev`. Raw facts only: readings with machine, adapter, resolution, scene, load and
> commit; gate output counts and wall times; attempts and what each read; resume notes for the
> next agent. No conclusions. The plan's `## Implementation log` keeps one row per phase and the
> close block, and points here.

## Phase 1: <name>

- **Gate (<commit>):** `node scripts/test-changed.mjs --profile quick` N run, N passed, M s;
  clippy 0; fmt 0.
- **Reading (<figure>, <commit>, <machine and adapter>, <scene>, exclusive perf lock, 1-minute
  load X):** value.

## Phase 2: <name>

…

### Resume notes (<date>, delete when Phase N lands)

_(Only while a lane is checkpointed: what is done with SHAs, what is left, the gate state at
the tip, readings and attempts so far, gotchas. The next agent reads this section and the
current phase above, not the whole file.)_
