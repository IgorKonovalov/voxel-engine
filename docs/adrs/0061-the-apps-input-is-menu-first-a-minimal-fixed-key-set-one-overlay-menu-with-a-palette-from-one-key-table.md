# ADR-0061: The apps' input is menu-first: a minimal fixed key set, one overlay menu with a search palette, and one key table that drives the menu, the palette, the help and the shortcuts

> **Status:** proposed
> **Date:** 2026-11-16
> **Related plan(s):** [Plan 0080](../plans/0080-the-fly-throughs-menu-one-overlay-a-palette-and-the-minimal-key-set.md)
> (builds it); [Plan 0073](../plans/0073-a-settings-menu-and-file-for-the-fly-through-and-the-tester.md),
> [Plan 0076](../plans/0076-a-key-help-overlay-for-the-fly-through-from-the-key-table.md) and
> [Plan 0077](../plans/0077-an-entity-picker-in-the-fly-through-choose-a-model-from-the-library-and-place-it.md)
> (amended onto it)

## Context

The fly-through is the engine's testbed and the tester's child (ADR-0054). Every mechanism a
plan adds gets a key: `docs/flythrough.md` lists them, and on main plus the `plan-0062` and
`plan-0063` branches the run reads every letter, every digit, `F1` to `F12`, `Tab`, `Backspace`,
`Space`, `Esc`, `Shift`, the main block's punctuation, both mouse buttons, the wheel, the arrows
and a pad. The owner, 2026-11-16: "about keys - instead of filling all keyboard with everything we
have in place - and I already can't remember what key does what - we should have a menu system,
settings overlay etc - and keep using keys to miminal. I think there shoud be some best practices
for interfaces like this".

Three approved plans each answer part of this with a panel of their own. Plan 0076 draws a key
help panel from the key tables (`F1`). Plan 0077 draws an entity picker (`` ` ``) and introduces
a `ListMenu` with its own key routing. Plan 0073 draws a settings menu over Plan 0069's
`RenderSettings` (its key reopened, since `Tab` is bound) with a third routing rule, and the
tester's launcher already has a fourth menu (`apps/tester/src/menu.rs`: rows, a selection,
`Menu::key`, `Menu::write`). Each decides on its own which keys it takes while open and what
happens to mouse look, the dig click and the wheel. The toggles are already one table
(`toggles::TOGGLES`: press, name, HUD row, a getter in words, a flip), and the owner's standing
rule since Plan 0010 is that every toggle shows its mode in the window.

What the apps have to draw with: `vxe_render::HudOverlay` (ADR-0009), a grid of 8 x 8 glyphs at
an integer scale, 80 x 45 cells at 1280 x 720, coloured rows, no widgets. What they have to read
with: `vxe_app::Input`, which reports key presses by `KeyCode`, clicks, the wheel, mouse deltas
and a pad, and reports no cursor position and no text events. The cursor is locked for mouse
look. No UI library is in the lockfile.

## Decision

The fly-through and the tester are **menu-first**. A **fixed key set** stays bound and is not in
any menu: moving and looking (`W` `A` `S` `D`, `Space`, `C`, `Shift`, the mouse), the dig and the
fill (the clicks), the dig radius (the wheel), the menu key, `F1` (help), `F2` (the stats
overlay) and `Esc`. **Everything else is an entry** of **one menu component** in the fly-through's
library, which the tester reuses as it reuses the rest of `flythrough`: a shallow tree of pages
(a page per group of the key table), each entry drawn with its **live value** from the same
getter its HUD row uses, its **shortcut** beside it where it has one, and a **palette**: typing
on any page filters every entry of the tree by its words, flat, so a forgotten key is found by
name. **One input-capture rule**, one function the tests drive: while the menu is open the
arrows, `Enter`, `Backspace`, `Esc`, the printable keys (into the filter), the wheel and the
clicks are the menu's, and the fixed movement and look keys, the function keys and the pad's
sticks pass through; the dig click and the wheel's own use pause; `Esc` with the menu closed
still ends the run. **The key table is the single source**: the menu's tree, the palette's
words, the help view (`F1` opens the menu on a flat page of every entry and its binds) and the
shortcut labels are generated from it, `flythrough --keys` prints it, and a CPU test fails when
a key is read in the source and not in the table, or an entry is unreachable from the menu. A
shortcut is an optional accelerator on an entry, never the only way to it; a plan that adds a
mode adds an entry and may add a shortcut, so the key set does not grow by default. The
component is **apps-only**: nothing under `crates/` names a key, an entry or a menu, and
`vxe-render`'s overlay gains at most a general drawing facility (a backdrop or an inverse-video
cell bit) if the owner's window asks for one.

## Consequences

### Positive
- A mechanism is reachable by name from the window without a key to remember, and its value is
  on screen where it is changed, which is the owner's standing rule made structural.
- One component, one routing function, one test of it. Plans 0073, 0076 and 0077 become pages
  and views of it instead of three panels with three rules.
- The help cannot drift from the keys and the menu cannot miss a toggle: both are the table, and
  the source scan and the reachability test bite on a lane that adds a key by hand.
- The keyboard empties. A plan that needs a shortcut takes one from a free key with the owner's
  go, instead of from the last punctuation key left.
- No new dependency, no `wgpu` version coupling, pixel-exact goldens with the menu open.
- A pad navigates the same tree with the same routing (`Start` opens, the D-pad moves, `South`
  activates, `East` goes back), so the pad gains every entry at once.

### Negative
- Fewer shortcuts means more presses for what the hand did in one: a debug toggle flipped many
  times in a session is now `Tab`, a few letters, `Enter`. The palette keeps this to about four
  presses; the entries the owner uses most keep a shortcut (Plan 0080's table, the owner's
  picks).
- Text input is `KeyCode`-based: the filter takes ASCII letters, digits and space from the
  physical key, not the layout's character. Right for the dev box's layout; wrong for a layout
  where `KeyQ` is not `q`. Accepted for a testbed; `vxe-app` can report text events later.
- No pointing: `vxe-app` reports no cursor position, so the menu is keys, wheel and clicks. The
  cell grid makes a hit test trivial once a cursor position exists (a backlog entry).
- The component is app code the first consumer cannot reuse without copying it. If a game asks
  for it, a later ADR lifts it into a `vxe-ui` crate; the engine does not grow it now.
- The tester's launcher menu moves onto the component (Plan 0073 Phase 5), a rewrite of working
  code for the sake of one routing rule.

## Alternatives considered

### Alternative A: keep binding keys, with Plan 0076's help panel as the remedy

The cheapest path: every mechanism keeps a key and `F1` lists them. It lost on the owner's
words: the keyboard is already full, and a list of about a hundred bindings is a reference, not
a memory aid. It also leaves `Tab`, `Backspace` and `ArrowUp` as the last keys for Plans 0073
and 0077 to fight over.

### Alternative B: an immediate-mode GUI library (`egui` through `egui-wgpu` and `egui-winit`, or `imgui-rs`)

Widgets, sliders, text fields, proportional text and mouse pointing for free, and the tree and
palette are a few dozen lines. Weighed honestly: `egui` brings `epaint`, `emath`, `ecolor`,
`ahash` and the two integration crates, each of which pins a `wgpu` and a `winit` major on its
own release train; our `wgpu` is `=30.0.1` and `winit` `=0.30.13`, so a `wgpu` bump waits for
the integration crate or the integration crate holds the bump (ADR-0009 read the lag at weeks
to months). `imgui-rs` adds a C++ build and the same binding lag. Both draw anti-aliased
triangles in a pass of their own, so a golden with the menu open is no longer a bit-exact
comparison, and both want a cursor position, which `vxe-app` would gain anyway. Against that,
what the menu draws is rows of text with one filter field, which the overlay already draws. It
lost on the version coupling, the one cost no pin removes. If the menu outgrows text rows
(sliders, plots), the library goes into an optional app-side crate, never into `vxe-render`.

### Alternative C: `F1` help only, no menu

Plan 0076 as approved and nothing more. It lost because help does not change a value: the hand
still returns to the keyboard, and the keyboard is the problem.

### Alternative D: a console of typed commands (Quake-style)

A text line, commands by name, no tree. It gives the same reach as the palette with no list to
read: a command must be known to be typed, which is the owner's complaint restated. It also
shares the palette's `KeyCode` text limit. The palette keeps the console's one virtue, typing to
reach anything, and shows the matches as it goes.

### Alternative E: each plan its own panel (the approved drafts of Plans 0073, 0076 and 0077)

Three panels, three key-routing rules, three ideas of what mouse look does while a panel is
open, and the tester's fourth. It lost on the count: the second rule is already a bug surface,
and the window has one overlay.

## Outcome

_(Added only at acceptance if implementation falsified something above.)_
