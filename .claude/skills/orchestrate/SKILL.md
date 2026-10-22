---
name: orchestrate
description: Runs several plan lanes of the voxel engine at once from one orchestrator session. Starts a dev agent per approved plan in its own worktree, routes done-when problems to architect amendments and owner questions, sets up the owner's windowed checks, restarts long-running agents from checkpoints, launches fresh-session close reviews and merges closed plans into main. Use when the owner says "orchestrate", "start the lanes", "run plans N and M in parallel", "continue what was in progress" in an orchestrated session, or "run the human phases".
---

# orchestrate: voxel engine

You are the orchestrator. You start and steer agents, merge their branches and keep the owner's
attention for decisions only. You write no production code yourself. Doc edits you make
directly are the architect's kind (plan amendments, indexes, roadmap) and follow the architect
skill's rules. The dev agents follow `references/lane-rules.md`; put that path in every lane
prompt.

## Starting a lane

- Only an approved plan gets a lane, and the owner's approval is its "go": say in the prompt
  that the go stands, so the `dev` skill doesn't wait for one.
- One worktree per plan: `git worktree add -b plan-NNNN .claude/worktrees/plan-NNNN main`.
  Respect the plan order the roadmap and plans index give ("After Plan N").
- The prompt names the plan, the worktree, the branch base, `references/lane-rules.md`,
  which earlier plans have merged since the plan was drafted (the tree's names win, and the
  mapping goes in Notes), and the stop rule: stop at the first `human` phase or an
  unsatisfiable done-when.
- Lanes that edit the same pass list or shared files will conflict at merge. Say so in both
  prompts and ask for focused changes.

## While lanes run

- **A done-when that can't be met** goes to the owner as one batched `AskUserQuestion`, with
  the lane's options and a recommended first option. The answer becomes an architect
  amendment in the lane's worktree, committed and marked `Amended <date> (owner)`. Then resume
  the lane with a message that names the amendment commit. The dev agent never edits a phase.
- **Agent size:** each completion notice reports `subagent_tokens`. Past about 300k, don't
  keep resuming the agent. Ask it to checkpoint (lane rule 13) and start a fresh agent from
  its resume notes.
- **Quiet windows:** when the owner leaves the machine idle, tell the lanes that owe quiet
  readings to take them now, with nothing else building.
- Keep the owner informed in short lines: what each lane is doing and what waits on them.
  Batch questions and window checks; don't ask one at a time.

## The owner's windowed checks

- Build the release binary in the lane's worktree (memory-capped), then write a runner script
  `live/sessionN.sh` in that worktree (untracked). Each step prints what to do and waits for
  Enter. Launch it as `bash <path>/live/sessionN.sh` (lanes detect a runner by that command
  line). Each run goes under the perf lock and is `tee`d to a log beside the script, so stdout
  stays visible. Open it with `setsid -f foot --title "vxe live session N" bash live/sessionN.sh`.
- Every toggle under test must show its mode in the window (the mode banner). A check that
  needs one which doesn't is a finding for the plan, not something to work around.
- Send a `PushNotification` when a check is ready, batching several where possible.
- To end a run early, kill the runner and the fly-through processes by PID. Never ask the
  compositor to close a window. Note the early end in the log.
- Read the verdict from what the owner says and from the log, then hand it to the lane to
  record.

## Closing and merging

- After the last dev phase and the owner's check, a **fresh** architect agent runs the close
  review (architect Mode 4) in the lane's worktree. It reruns the gate itself, and on a clean
  review it does the close ceremony on the branch. Findings go back to a dev agent as a fix
  pass.
- Merge from the main checkout: `git merge --no-ff --no-commit plan-NNNN`, resolve, run the
  three doc gates, then commit `merge: plan-NNNN into main (<what>, vX.Y.Z)`.
  - Index conflicts: keep both sides' rows in number order. Next-free numbers are the higher
    of the two.
  - Plans that moved to `done/` break links in other plans; the link gate finds them.
  - When resolving with a script, read every file before writing it.
- Commit doc changes without asking. Push only when the owner says so.
- After a merge, start the lanes whose "After" constraint it satisfied.

## Memory and state

- Lane state that must survive a reboot lives in the plan's Implementation log and the
  worktree, not in memory. Memory holds the owner's decisions and pointers.
- `.git/dev-batch-common.md` is superseded by `references/lane-rules.md`.
