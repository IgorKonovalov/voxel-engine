# Lane rules for parallel dev agents

Every dev agent in an orchestrated session reads this file before its first command. Several
plan lanes run at once, each in its own worktree at
`/home/igor/Work/voxel-engine/.claude/worktrees/plan-NNNN` on branch `plan-NNNN`.

## Scope

1. Work only inside your worktree. Never touch the main checkout or another lane's worktree.
   Never merge, push or switch branches; the orchestrator merges. The one exception: when the
   orchestrator asks, merge main into your own branch (`git merge --no-ff main`), resolve,
   gate the merged tree with the full tier (`cargo nextest run --workspace --profile full
   --no-fail-fast`, under the perf lock) and commit it as `merge: main into plan-NNNN (...)`.
2. Inside a plan you edit only the `Status:` line and the `## Implementation log` (the `dev`
   skill's rule). A done-when you cannot meet is a question, not an edit: stop and report it
   with options. Never weaken an assertion to make a bar pass.
2a. **Pre-approved test re-derivation (owner, 2026-10-28).** An existing test that reads a
    transient may move its reference to where the quantity has settled, without stopping.
    Examples: a "settled" value at a frame count that a long series shows is still converging,
    or a golden rendered before its history settles. The conditions:
    - The claim and the bar stay as written; only the reference frame count or settled value
      moves.
    - A long series (run until the change per visit is under the bar's tolerance) shows where
      the quantity settles. The new count comes from that series or from the blend arithmetic,
      not from what makes the test pass.
    - The test is read on the base and the tip, in its old and new form, and all four readings
      go in the log, with the commit that changes the test.
    - The doc comment states the reference and where it comes from.

    Anything that loosens a bar still stops for the owner. That includes a wider tolerance, a
    lower ratio, a longer lag, a dropped assertion, or a bar re-derived against a different
    quantity.
3. Keep anything that must survive a reboot in the worktree (committed, or under the
   untracked `live/`), never only in `/tmp` or a scratchpad.

## Machine load

4. Cap build parallelism: `export CARGO_BUILD_JOBS=4` for every cargo command, and run
   `--profile full` nextest passes with `--test-threads 2`. `CARGO_BUILD_JOBS` caps the build;
   nextest's `-j` is `--test-threads`, not build jobs. The tiers are in `.config/nextest.toml`:
   `--profile quick` is the phase gate, `--profile gpu` adds the render tests, `--profile full`
   is everything (the default profile is the full suite too).
5. Every cargo build, test, bench or fly-through run goes inside a memory-capped scope, so a
   runaway dies alone (the 2026-10-20 oomd kill took every session at once):

       systemd-run --user --scope --quiet -p MemoryMax=6G -p MemorySwapMax=1G -- <command>

   If a command dies to the cap, report its name and peak. Don't raise the cap yourself.
6. GPU work is serialized across lanes by one exclusive lock. Anything that touches the GPU or
   measures time (render and golden tests, `vxe-render` tests, headless fly-through renders,
   any bench, any timing reading, a `--profile gpu` or `--profile full` nextest run) runs
   under it, with the lock outside the scope. A `--profile quick` run touches no GPU and takes
   no lock:

       flock /home/igor/Work/voxel-engine/.git/vxe-perf.lock systemd-run --user --scope --quiet -p MemoryMax=6G -p MemorySwapMax=1G -- <command>

   Build first, outside the lock, and keep the hold short. The owner's windowed checks take the
   same lock run by run, and the lock is free between runs, so a GPU job started then blocks
   the owner's next run. While a live-session runner is up (`pgrep -f '^bash .*live/session[0-9]+\.sh'`
   finds it; a looser pattern also matches the shell running the check), take no GPU or timing work at all; poll at most once a minute, and keep doing CPU work.
7. Record "exclusive perf lock" and the 1-minute load average with every reading. A reading
   taken at a load above 2 is labelled loaded (indicative), not quiet.

## Readings

8. From Plan 0028 on, a render plan's GPU readings are taken on both adapters and labelled:
   the NVIDIA RTX 3080 Laptop (the dev reference) and the AMD Cezanne iGPU on RADV (the slow
   adapter of `docs/nfr.md` §1). Use `--adapter` where the branch has it; otherwise
   `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json` leaves the Vulkan loader only
   RADV. Say in the log how the adapter was selected.
9. The wasm tests need `scripts/build-guests.sh` run once in a fresh worktree.
9c. **Iterate on the RTX, prove on lavapipe (owner, 2026-10-28).** Exploratory render runs are
    sweeps, attribution runs, variant comparisons and long series. Run them on the RTX 3080
    Laptop, which is far faster than lavapipe, and label the readings with the adapter. Every
    done-when reading, golden comparison or bless, and gate run stays on lavapipe, which is
    CI's adapter and where the goldens were blessed. A sweep's winner is re-read on lavapipe
    before its commit. The render tests hard-code `force_fallback_adapter: true` (2026-10-28),
    so until a test-support switch exists, this applies only to scratch tests and fly-through
    runs that take `--adapter`. Golden assertions never run against the RTX.

## Context budget

9a. Read your own plan in full, but not the large shared docs: `docs/nfr.md` (~110 KB),
    `docs/roadmap.md` (~55 KB), `CHANGELOG.md`, other plans. `grep -n` for the section you need
    and read only that range.
9b. Keep tool output short. Cargo and nextest output is the main waste:
    - builds: `cargo build -q`; clippy: `--message-format short`;
    - nextest: `--status-level fail --final-status-level fail` (failures and the summary only);
    - anything longer: `set -o pipefail; <command> 2>&1 | tee live/<name>.log | tail -n 40`, then
      `grep` the log for what you need rather than reading it whole.

## The owner

10. Never open a window, never run anything windowed, and never ask the compositor
    (`hyprctl`) to close or focus a window; a hook denies the last. A windowed, headphone or
    controller check is a `human` phase: stop there and give the exact commands.

## Commits

11. Stage by explicit path; separate `git add` and `git commit` calls; a quoted heredoc
    message with a plain-ASCII body; no attribution trailer of any kind; no amend, rebase or
    reset. Conventional commits, one phase per commit (`.claude/skills/dev/references/commit-conventions.md`).

## Stopping

12. When you stop (plan done, human phase reached, a plan-is-wrong question, or the
    orchestrator asked for a checkpoint), your final message gives: the branch tip SHA, phases
    done, the gate state with counts, the budget readings, and any owner question verbatim
    with options, plus the exact commands for the owner's human phase.
13. On a checkpoint request: let the running command finish, release the GPU lock, commit any
    code as `wip(...)`, and write `### Resume notes (<date>, delete when Phase N lands)` in the
    plan's Implementation log (what is done with SHAs, what is left, readings and attempts so
    far, gate state, gotchas). A fresh agent resumes from those notes.
