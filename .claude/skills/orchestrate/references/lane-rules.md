# Lane rules for parallel dev agents

Every dev agent in an orchestrated session reads this file before its first command. Several
plan lanes run at once, each in its own worktree at
`/home/igor/Work/voxel-engine/.claude/worktrees/plan-NNNN` on branch `plan-NNNN`.

## Scope

1. Work only inside your worktree. Never touch the main checkout or another lane's worktree.
   Never merge, push or switch branches; the orchestrator merges.
2. Inside a plan you edit only the `Status:` line and the `## Implementation log` (the `dev`
   skill's rule). A done-when you cannot meet is a question, not an edit: stop and report it
   with options. Never weaken an assertion to make a bar pass.
3. Keep anything that must survive a reboot in the worktree (committed, or under the
   untracked `live/`), never only in `/tmp` or a scratchpad.

## Machine load

4. Cap build parallelism: `export CARGO_BUILD_JOBS=4` for every cargo command, and run full
   nextest passes with `--test-threads 2`.
5. Every cargo build, test, bench or fly-through run goes inside a memory-capped scope, so a
   runaway dies alone (the 2026-10-20 oomd kill took every session at once):

       systemd-run --user --scope --quiet -p MemoryMax=6G -p MemorySwapMax=1G -- <command>

   If a command dies to the cap, report its name and peak. Don't raise the cap yourself.
6. GPU work is serialized across lanes by one exclusive lock. Anything that touches the GPU or
   measures time (render and golden tests, `vxe-render` tests, headless fly-through renders,
   any bench, any timing reading, a full-workspace nextest run) runs under it, with the lock
   outside the scope:

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
