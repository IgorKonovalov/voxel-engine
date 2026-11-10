# 0066: Test cost: a 10 s soak line, and the gpu binaries a phase names

> **Status:** in-progress (owner go 2026-11-10; side lane beside Plan 0060)
> **Created:** 2026-11-10
> **Milestone:** none (harness chore; `docs/nfr.md` section 7)
> **Related ADRs:** none. The rejected alternatives (below, Decision) are a process call, recorded
> here and in `docs/nfr.md` section 7.
> **Source:** the owner, 2026-11-10: "it feels like the most time goes towards tests". The session's
> readings: over the 14 days to 2026-11-10, test files took 51.9k changed lines against 33.0k in
> `src/` (`git log --numstat`, `crates/` and `apps/`), led by the flythrough harnesses (`settle.rs`
> 11.9k, `bake.rs` 9.8k, `look.rs` 5.4k); a phase's gpu tier read 787.6 to 1374.6 s; the gate read
> 850.2 s at Plan 0059's close, over its 12 min proposal, with no test over the 30 s line.
> **Starts after:** nothing. It edits no file under `crates/` but test attributes, and no render
> source, so it runs beside Plan 0060 as a side lane.

## TL;DR

The gate is slow because the GPU tests run one at a time on lavapipe and many of them take 10 to
30 s each, not because one test is huge. This plan lowers the soak line from 30 s to 10 s on
lavapipe, marks every gate test over it `#[ignore = "soak"]`, and gives
`scripts/test-changed.mjs` a `--binaries` option so a phase runs only the `gpu_*` binaries it
names or touched. The gate drops under 12 min; a phase's GPU check drops from 13 to 23 min to the
few binaries it touches. The process half (exploration mode for look-driven plans, the soak run at
close) is already written into the skills and Plans 0032 to 0037, 0058 and 0061 on 2026-11-10.

## Context & problem

- `.config/nextest.toml` puts every `gpu_*` binary in the `lavapipe` group at one thread, so the
  gate's wall time is about the serial sum of the GPU tests plus the build. Plan 0059's close read
  850.2 s with a serial test sum of 20.0 min (`docs/nfr.md` section 7).
- `test-changed.mjs --profile gpu` selects `rdeps(<changed package>)`. `apps/flythrough` depends on
  every crate, so any render change runs every flythrough GPU binary.
- The soak line (30 s) catches no test today, so marking soak by it cannot cut the gate.

## Decision

1. **The soak line on the dev reference's lavapipe is 10 s** (CI's Linux arm keeps 60 s). Every
   gate test over it is born or becomes `#[ignore = "soak"]`. The soak tier runs weekly in CI and,
   narrowed to the plan's crates, in each close review.
2. **A phase runs the gpu binaries it names**: `test-changed.mjs --profile gpu --binaries a,b`
   narrows the gpu tier to `binary(=a) | binary(=b)` and to every `tests/gpu_*.rs` the change
   touched. The whole gpu tier runs at a plan's tip and in the close review.

Rejected (owner, 2026-11-10): **re-cutting the slow GPU tests to smaller renders** (keeps them in
the gate, but costs a plan of test rewrites and moves their goldens); **a golden-only gate** (every
other flythrough GPU test to soak, which drops the CPU-visible bars too); **no GPU test per
phase** (a render phase would reach the tip blind).

## Implementation phases

### Phase 1: `--binaries`, the 10 s line in the scripts

- **Owner skill:** dev
- **Files touched:** `scripts/test-changed.mjs`, `scripts/test-times.mjs`,
  `scripts/check-test-tiers.mjs` (only if it states the line), `scripts/fixtures/` and its
  README (a fixture for the new option, if the script's self-test uses fixtures),
  `.config/nextest.toml` (the header comment's line), `CLAUDE.md` (the `test-changed.mjs` row,
  only if its wording no longer fits), `docs/plans/logs/0066.md`.
- **Done when:**
  - `node scripts/test-changed.mjs --profile gpu --binaries gpu_look --dry-run` with a change
    under `crates/vxe-render/src/` prints a filter that selects `binary(=gpu_look)` and no other
    gpu binary; with a change to `apps/flythrough/tests/gpu_queue.rs` as well it also selects
    `binary(=gpu_queue)`. `--binaries` under `--profile quick` is a usage error (exit 2). An
    unknown binary name is a usage error that names it.
  - `node scripts/test-times.mjs <log>` uses a 10 s line by default and names a 12.0 s test in a
    seeded log as over it.
  - The nextest header comment and every script comment that states the line say 10 s on lavapipe,
    60 s on CI.

### Phase 2: Mark the tests over the line, and read the gate

- **Owner skill:** dev
- **Files touched:** the test files of the tests over the line (attribute only:
  `#[ignore = "soak"]`, no body change), `docs/nfr.md` section 7 (the reading),
  `docs/plans/logs/0066.md`.
- **Done when:**
  - A quiet gate run under the perf lock (`cargo nextest run --workspace --profile gate
    --status-level pass`, built tree, the load recorded) and `node scripts/test-times.mjs` on its
    log name the tests over 10 s; each gets `#[ignore = "soak"]`. The readings log lists every
    moved test with its time, so the owner sees what left the gate.
  - A second quiet gate run reads **under 720 s** wall on the dev reference, and `test-times.mjs`
    names no test over the line. If it reads over 720 s, stop and report the residual and its
    largest binaries; don't lower the line further (that is the owner's call).
  - One `--profile soak --run-ignored only` run under the lock passes every moved test (a one-off
    the owner approved with this plan, 2026-11-10), its wall time recorded.
  - `node scripts/check-test-tiers.mjs` exits 0.

## Budgets

- `docs/nfr.md` section 7: the gate under 12 min (the reading replaces the proposal at close); the
  gpu tier per phase, read once at Phase 2 with `--binaries` naming one binary, as a reading.
- No frame, memory or edit figure moves.

## Risks & open questions

- **Coverage.** Goldens over 10 s (`castle_gate_golden` read 11.4 to 13.3 s warm) leave the gate
  for the weekly soak and the close review's narrowed soak run. A break between those runs
  surfaces later than today. The moved list in the readings log is the owner's check.
- **Load.** A test near 10 s reads over it under load; the line is read quiet, as section 7 says.
  A test that flips across the line between quiet runs stays in the gate.

## What this plan does NOT do

- Rewrite or speed up any test body (the rejected re-cut).
- Change CI's 60 s line or the `ci` profiles' timeouts.
- Touch the process rules for exploration phases; those landed in the skills and the plans on
  2026-11-10.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`. Readings, gate counts, attempts, sweeps and
> resume notes go to the plan's readings log, `docs/plans/logs/0066.md` (started from
> `templates/readings-log.md`), and this section links it once it exists.

Readings log: [logs/0066.md](logs/0066.md).

| phase | owner | state | commit |
|---|---|---|---|
| 1: `--binaries`, the 10 s line in the scripts | dev | done; `CLAUDE.md` canonical-commands line names `--binaries`; fixture `scripts/fixtures/test-times/` | committed with this row |
| 2: Mark the tests over the line, and read the gate | dev | not started | |

### Notes

### Close triggers

## Followups
