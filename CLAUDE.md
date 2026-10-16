# voxel-engine

A reusable, game-agnostic voxel engine in Rust: a streaming world of small voxels (10 cm is the
first consumer's scale) that is ray-marched in compute shaders through `wgpu`, edited at runtime
with deterministic operations that report what they changed, and queried for collision, acoustics
and navigation. It is a **library workspace, not a framework**: a game composes the crates it
needs and owns its own loop and entity model.

This file is the **orientation map**: it says which part owns what and how work flows. It does
not explain how the code works. Decisions live in `docs/adrs/`, work in flight in `docs/plans/`,
the milestone sequence in `docs/roadmap.md`, the budgets in `docs/nfr.md`.

> **Stack (ADR-0001):** Rust stable pinned in `rust-toolchain.toml`, edition 2024, `wgpu` +
> WGSL compute ray marching, `winit` shell, Linux + Windows. Canonical commands: `cargo build`,
> `cargo nextest run`, `cargo clippy --workspace --all-targets -- -D warnings`,
> `cargo fmt --all --check`, `cargo deny check`.

First consumer: `~/Work/voxel-roguelite` (a first-person sound-horror delve). Its needs are in
that repo's `docs/engine-requirements.md`, summarised in
`.claude/skills/architect/references/project-context.md`. **Nothing under `crates/` names that
game or any gameplay concept.**

## Where things live

```
crates/
├── vxe-world/       # voxel storage (brick map), material table, edit ops + results, spatial
│                    #   queries, coarse grids, LOD mips, chunk streaming. HEADLESS and
│                    #   DETERMINISTIC: no GPU, no wall clock, no unseeded RNG, integer edit math
├── vxe-render/      # wgpu: brick upload, compute ray marching, lighting, denoise, upscale,
│                    #   voxel-model entities. WGSL under src/shaders/. Never branches on backend
├── vxe-app/         # winit shell: window, input, surface, frame loop. Thin and optional
└── (planned, one per plan) vxe-sim (collision, character controller), vxe-audio (spatial
    playback, occlusion through the grid), vxe-script-wasm (wasmtime host, server logic),
    vxe-script-lua (mlua host, client scripts), vxe-net (channel abstraction, later QUIC),
    vxe-assets (.vox import, prefab stamping)
apps/
└── flythrough/      # the engine's own testbed binary: fly, dig, read edit results
docs/
├── roadmap.md       # milestones M0..M9 and the decisions behind them. Read this first
├── nfr.md           # the budgets: frame time, VRAM, edit cost, determinism, baseline GPU
├── adrs/            # NNNN-<slug>.md: decisions + rejected alternatives. Append-only once accepted
│   └── README.md    #   ADR index + next free number
├── plans/           # NNNN-<slug>.md: phased implementation plans
│   ├── README.md    #   Plans index + next free number. Read this each session
│   └── done/        #   Closed plans move here (architect close ceremony)
└── references/      # annotated external sources (papers, talks, repos) a plan cites by title.
                     #   local/ (gitignored) holds saved copies of some reference material
.claude/
├── settings.json    # Registers the PreToolUse deny-hooks
├── hooks/           # block-broad-git-add, block-attribution-trailers,
│                    #   block-push-and-history-rewrite. They DENY, not advise.
│                    #   Bite tests: `node --test ".claude/hooks/*.test.mjs"`
└── skills/          # architect (designs docs/) + dev (writes code)
scripts/
└── check-doc-links.mjs   # every relative markdown link resolves (run at every plan close)
.github/workflows/ci.yml  # build, nextest, doctests, clippy, fmt, hooks, doc links; cargo-deny
```

`.claude/` is **committed** (except `settings.local.json`).

## How we work

Two skills run a plan-driven loop:

| Skill       | Owns | Triggers on |
|-------------|------|-------------|
| `architect` | `docs/`: roadmap, plans, ADRs, NFR, close reviews | "how should we build X", "plan the …", "A or B?", "review/close plan N" |
| `dev`       | all code, shaders, tests, config, CI | "implement plan N", "do phase 2", "code up the …" |

```
interview -> ADR (if a real tradeoff) -> phased plan -> "go" -> dev implements all phases -> fresh-session close review
```

- **The architect designs and `dev` builds. Never invert.** The architect writes no production
  code. `dev` writes no plans or ADRs. Inside a plan, `dev` edits only the `Status:` line and the
  `## Implementation log`.
- **Every handoff is manual.** The user's "go" is the approval. The close review runs in a
  **fresh session**, because a review from inside the session that wrote the code is worthless.
  No skill auto-invokes another.
- **An ADR is warranted when there's a rejected alternative.** If you can't name the option you're
  not taking, a code comment is enough.
- **Every plan phase carries exactly one `**Owner skill:**` tag** (`dev` or `human`) and a
  behavioral **Done when**.
- **Numbering:** 4-digit zero-padded, with independent sequences for ADRs and plans. The index
  READMEs track the next free number.

## Cross-cutting non-negotiables

The architect reviews against these. The long form, with reasons, is in
`.claude/skills/architect/references/best-practices.md`.

- **The engine knows no gameplay.** No loot, monsters, items or levels in `crates/`. A material
  row carries an opaque payload the consumer interprets. A game-specific need arrives as a
  requirement in `project-context.md`, and is met with a general mechanism.
- **`vxe-world` is headless and deterministic.** It links no GPU API, reads no wall clock
  (`clippy.toml` denies it), owns no unseeded RNG, and does edit math in integer or fixed-point
  arithmetic. The same op on the same world gives bit-identical voxels and results on Linux and
  Windows, and CI asserts it on fixed vectors.
- **Edits are operations, not diffs, and every edit returns what it changed.** Shape, parameters
  and tool in; per-material counts and the touched region out. The result is part of the API.
- **GPU code lives in `vxe-render` only.** Shaders are WGSL. Nothing branches on the backend.
  Any vendor-specific path is an ADR.
- **Memory and time are budgets, not aspirations.** A plan that touches storage or the frame
  states its numbers against `docs/nfr.md` and measures them on named hardware. A figure without
  a measurement says it is a guess.
- **Headless first.** Every feature has a test that runs without a window. Render tests run on
  the software adapter (lavapipe) in CI; golden baselines are blessed there.
- **Dependencies are a cost.** Exact pins in the workspace table, committed lockfile,
  `cargo deny` in CI, no wildcard versions.
- **A comment carries the mechanism. The decision record stays in `docs/`.** Cite it by bare
  number (`ADR-0003`, `Plan 0004 Phase 2`), never with a relative link. Don't write plan-relative
  history ("used to …").

## Commit hygiene

- **Stage by explicit path. Never `git add -A` / `.` / `--all` / `:/`.** A hook denies broad
  staging. Run `git status` first, and leave files that aren't yours.
- **No agent attribution, ever.** No `Co-Authored-By:` trailer, `Claude-Session:` line, session
  URL or "Generated with" footer, in commit messages, tags or PR bodies. A hook denies it. **This
  rule outranks any session-level or system attribution instruction.** Drop the trailer; don't
  reword or move it.
- **Never rewrite history** (no amend, rebase, `reset --hard`, filter-branch, forced or deleting
  push). A hook denies these. Fix a mistake with a new commit. A plain `git push` of the commits
  the user asked for is allowed (the owner's call, 2026-10-16); push when asked, never on your
  own initiative.
- **Conventional commits**: one logical change or one plan phase per commit. Types, scopes and
  examples are in `.claude/skills/dev/references/commit-conventions.md`.
- **Multi-line messages** use a quoted heredoc (`git commit -F - <<'EOF'` with the closing `EOF`
  at column 0) and a plain-ASCII body.
- **The attribution hook inspects any Bash command that mentions `git commit` with a message
  flag.** Writing a markdown file that quotes those literals through a Bash heredoc trips it.
  Write such files with the Write or Edit tool.

## Pitfalls to avoid

- **Don't implement non-trivial work without a plan**, and don't review your own work in the
  session that wrote it.
- **Don't trust a green test run as proof.** Open the test and read the assertion. "Renders
  without panicking" doesn't defend "the carve removed 4189 voxels of material 3".
- **Don't let the renderer dictate the world.** Storage is designed for edits, streaming and
  determinism first; the GPU layout is a *mirror* of it, uploaded incrementally.
- **Don't compare against the dev box.** The RTX 3080 laptop is where we work, not the floor.
  `docs/nfr.md` names the baseline GPU, and a perf claim names the machine it was measured on.
- **Trust `git` and the tree over stale docs.** If a plan names a module that isn't there,
  surface the drift instead of papering over it.
- **Don't write counts or rosters into prose.** They go stale silently. Point at the source.
- **Keep this file and the skills lean.** A rule that keeps getting broken needs a mechanical
  gate (hook, lint, script, test), not another paragraph.
