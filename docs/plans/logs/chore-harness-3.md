# chore-harness-3: readings and resume notes

### Resume notes (2026-11-25, delete when piece A lands)

**Done (wip commit on `chore-harness-3`):** piece A's code. `apps/flythrough/src/inert.rs`
holds `INERT_WHEN` (row, menu entry, short reason, `holds`, `live_under`), `inert_rows` and the
`Inert` bitset that `RunView::inert` carries (`Run::run_view` fills it from `live_settings`).
`Entry::shown` appends `(inert: <reason>)`; `write.rs` prints it; `Run::menu_act` and
`toggle_keys` say the marked line in the notice. CPU tests in `src/run/inert_tests.rs` (mark on
and off per condition, the table's entry names derived by activating every entry, the notice).
`tests/gpu_settings_rows.rs` reads the table instead of its own `Needs`/`Reading` entries.
`docs/flythrough.md` has the paragraph. Quick tier 406/406, clippy, fmt and the check gates
pass.

**Blocked: the GPU walk fails, and the failure is real.** Readings (lavapipe, exclusive perf
lock, load 13 to 19, indicative):

- The base test at `adc3f59a` proves its `Needs` rows in INERT's order (sky_rays 272,
  sky_history 22, key_history 1933, penumbra 18525, key_cap 1106, sky_down_skip 1048,
  key_far_mix 1508) and passes.
- The same comparison in the table's order (penumbra, key_cap, key_history, sky_rays,
  sky_history, sky_down_skip, key_far_mix): key_history 5 and sky_down_skip 0. The walk's
  renders are not independent: `render`'s resets (history, exposure, frame index, lamp
  channel) do not clear what the key ray and the light history keep, so a row's "live"
  count depended on the previous row's render.
- Both images after a warm render under `on` (`render(on); ref = render(on); render(s)`):
  penumbra 18527, key_cap 5, key_history 0, sky_rays 272, sky_history 22, sky_down_skip 0,
  key_far_mix 426; and `render(on)` after `render(s)` differs from `ref` by 1139 pixels
  (penumbra) and 638 (key_far_mix): the leak persists.

So key_cap, key_history and sky_down_skip are not shown live under key far ray / the light
history by a fair comparison; the backlog's "each live under key far ray" read the leak.

**Left:** the owner's (orchestrator's) pick of the options in the agent's report; then
re-read the walk, adjust the table, commit A for real; then piece B (notes below).

**B notes (not started):** no flag writes a still today. The headless target
(`svet_app::run_headless`) is RENDER_ATTACHMENT only; the frame's output is readable with
`svet_render::debug::read_rgba8(&device, &queue, gfx.frame.target())` (the HUD overlay is not
in it). `png` is a dev-dependency of the fly-through only: write a binary PPM from the binary
and let the node script encode PNG with `node:zlib` (node 26 has `zlib.crc32`). Text: the
mode rows from `Modes::write_rows` and the banner from `Run::compose_banner`. The exit point
is `frame.rs` where `args.frames` ends the run (`print_exit_readings`). Plan 0098's
rehearsals used GPU test rigs (`dig_line::reading::dig_timeline` with `VXE_LINE_PNG`,
`docs/plans/logs/0098.md` "The rehearsal for Phase 18"), not the binary.
