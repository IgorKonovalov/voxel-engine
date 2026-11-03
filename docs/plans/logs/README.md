# Plan readings logs

One file per plan, `NNNN.md`, holding what a plan's `## Implementation log` used to carry in
bulk: the per-run readings tables, sweeps and attempts, the gate output counts, and the
resume notes a checkpointed lane leaves for the next agent. The plan itself keeps one log row
per phase (state, commit, deviations) and the close block, so an agent that starts on a plan
reads the plan in full and, from here, only the latest `### Resume notes` and the current
phase's section.

The file is started by the first dev phase that has a reading to record, from
`.claude/skills/architect/references/templates/readings-log.md`, and linked from the plan's
Implementation log. It stays in this folder when the plan moves to `done/`; the close review
reads it for the budget readings it checks against `docs/nfr.md`.

Plans before 0055 keep their readings inline; nothing is moved.
