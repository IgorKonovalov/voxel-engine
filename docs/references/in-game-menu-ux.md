# In-game menus driven by keys and a pad: external sources

An annotated reading list for the fly-through's menu (Plan 0080, ADR-0061), collected
2026-11-17 after the owner's first window: an overlay menu on a text grid with nested groups,
toggles, cycles, numbers and a type-to-search palette, driven by the keyboard and a gamepad,
no mouse pointing. Every entry names what the source says and what we take from it, so a plan
can cite it by section. The figures are the sources' own; the plan's done-whens hold ours.
Sources were read on the web on 2026-11-17 at the URLs given; the Dear ImGui facts are from
the source files on `master` that day, not from its wiki. Where a convention the plan follows
has no written source behind it, the section says so rather than inventing one.

## Placement and size

- **Microsoft, "Xbox Accessibility Guideline 101: Text display".**
  <https://learn.microsoft.com/en-us/gaming/accessibility/xbox-accessibility-guidelines/101>
  Minimum body height of text (descender to ascender, in pixels): "PC/VR: Font size should
  equal or exceed: 18 px at 1080p, 36 px at 4K"; "Console: 26 px at 1080p, 52 px at 4K". Text
  should scale to 200 % without loss; a line of running text at most 80 characters; sentence
  case, left-aligned. What we take: the overlay's 8 x 8 glyph at scale 2 is 16 px, under the
  PC floor, so the menu's glyph scale grows with the window (3 at 1080p and on the laptop's
  2530 x 1378, 4 at 1440p). The panel stays 80 columns wide.
- **Game accessibility guidelines, "Use an easily readable default font size".**
  <https://gameaccessibilityguidelines.com/use-an-easily-readable-default-font-size/>
  "Small text size is a very common complaint amongst people with vision impairments"; cites
  the 10-foot UI floor of 28 px at 1080p and says to treat it as a minimum, not a target. What
  we take: the same direction; our floor is the XAG PC figure, since the testbed is a PC
  window, not a television.
- **Microsoft, "Designing for Xbox and TV".**
  <https://learn.microsoft.com/en-us/windows/apps/design/devices/designing-for-tv> Essential
  UI keeps clear of the outer 5 % of the screen; "it should take no more than six clicks" to
  cross a screen; "Try to put initial focus in the top left region"; information density
  "comparable to what you'd see on a mobile phone, rather than on a desktop". What we take: the
  panel is centred with a margin all round at the sizes where it fits; the selection starts on
  the first row.
- **Dear ImGui, `imgui_demo.cpp`, `ShowExampleAppSimpleOverlay`.**
  <https://github.com/ocornut/imgui/blob/master/imgui_demo.cpp> A readout overlay pins to a
  corner with a 10 px pad, offers the four corners and the centre, takes no focus
  (`NoNav`, `NoFocusOnAppearing`) and sits on a 0.35-alpha background. What we take: the fps
  readout is a corner row that never takes the selection; its corner is the owner's (top right
  asked for).
- **Not found:** a guideline that prescribes a centred panel over a corner one, or a share of
  the screen, for the menu itself. The centring is the owner's call; the sources above bound
  its size from below (text height) and its margins (the safe area).

## Backdrop and legibility

- **Microsoft, "Xbox Accessibility Guideline 102: Contrast".**
  <https://learn.microsoft.com/en-us/gaming/accessibility/xbox-accessibility-guidelines/102>
  Text at least 4.5:1 against its background, large text 3:1, text on inactive elements 3:1,
  a high-contrast mode 7:1; and the rule that matters over a rendered scene: "When text is
  displayed over a non-solid color background, the text contrast ratio should be measured
  between the text and the lowest contrasting area of the background." Recommends "a solid
  background behind any on screen text" or an adjustable-opacity one, and borders or outlines
  where a solid is impossible; Gears 5's subtitles sit on "a semi-opaque background during
  active gameplay". What we take: a dim box behind the panel (black at alpha 0.7), measured
  against the brightest wall the cave shows, which the glyph shadow alone did not clear in the
  owner's window; a grey row for an unavailable entry keeps the 3:1 inactive floor.
- **Game accessibility guidelines, "Provide high contrast between text/UI and background".**
  <https://gameaccessibilityguidelines.com/provide-high-contrast-between-text-ui-and-background/>
  "Ideally place your text and UI elements on a plain high contrast background, or where that
  is not possible, use prominent outlines and shadows"; 4.5:1. Also from the full list:
  "Ensure no essential information is conveyed by a fixed colour alone"
  (<https://gameaccessibilityguidelines.com/full-list/>). What we take: the backdrop over the
  shadow; the `> ` marker beside the yellow selected row and the reason beside a grey row.
- **Microsoft, "Gamepad and remote control interactions".**
  <https://learn.microsoft.com/en-us/windows/apps/design/input/gamepad-and-remote-interactions>
  On Xbox the platform "automatically adds a 'smoke' layer that covers areas outside of the
  popup UI". What we take: the whole-screen dim is a real convention (owner question 10's
  alternative); we prefer the panel-only box because the menu's job is to watch the scene
  change as a row flips.

## Focus and the selected row

- **Microsoft, "Xbox Accessibility Guideline 113: UI focus handling".**
  <https://learn.microsoft.com/en-us/gaming/accessibility/xbox-accessibility-guidelines/113>
  "All UI elements should display a highly visible focus indicator when they receive focus",
  by "borders around the element, increasing the element's size or font weight upon receiving
  focus, or combinations of multiple methods"; the indicator "should always be visible on
  screen" and never rest on an invisible or offscreen element; a dialog opening moves focus to
  its first control. What we take: the selected row is yellow and carries `> `; the page
  scrolls to keep it in view; a page opens with its first row selected.
- **Microsoft, "Xbox Accessibility Guideline 103: Additional channels for visual and audio
  cues".** <https://learn.microsoft.com/en-us/gaming/accessibility/xbox-accessibility-guidelines/103>
  "Color alone should never be used to represent information"; "If a change of color (graying)
  is used to inform the player of the existence and state of a control that isn't available,
  another method is also used." Forza Horizon greys an unavailable row and labels it "Not
  available" with a lock glyph. What we take: grey plus the reason in words, in the row's
  footer and in the notice when `Enter` is pressed on it.
- **W3C, "Understanding Success Criterion 2.4.13: Focus Appearance" (WCAG 2.2).**
  <https://www.w3.org/WAI/WCAG22/Understanding/focus-appearance.html> The focus indicator is
  at least a 2 px perimeter's area and changes with at least 3:1 contrast between focused and
  unfocused. What we take: the marker and the colour change together exceed this on a text
  grid; a reverse-video row is not needed.

## Numbers: how a value is edited

- **Microsoft, "Gamepad and remote control interactions", the engagement model.**
  <https://learn.microsoft.com/en-us/windows/apps/design/input/gamepad-and-remote-interactions>
  "Focus engagement requires the user to press the A/Select button on a gamepad or remote
  control when a UI element has focus in order to interact with it"; a slider defaults to
  engagement on, because otherwise left and right would be trapped by the slider while the
  user tries to move on; stacking controls vertically lets "left and right move the Slider
  handle, as expected", and B disengages. What we take: this is exactly the owner's request
  (`Enter` enters an edit mode, the arrows change the value, `Esc` or `Enter` leaves it), and
  it is the console platform's own model, not an idiosyncrasy. The same page's second half,
  `Left` and `Right` stepping a value in place on a vertical list, was offered and struck by
  the owner (Plan 0080 owner question 9, 2026-11-17): outside the edit mode the arrows only
  move the selection.
- **Dear ImGui, `imgui.cpp` "CONTROLS GUIDE", `imgui.h` `ImGuiKey` comments,
  `imgui_widgets.cpp` `SliderBehaviorT`.** <https://github.com/ocornut/imgui/blob/master/imgui.cpp>,
  <https://github.com/ocornut/imgui/blob/master/imgui.h>,
  <https://github.com/ocornut/imgui/blob/master/imgui_widgets.cpp> Keyboard: "Arrow keys: Move
  through items using directional navigation. Tweak value"; "Enter: Activate item"; "Escape:
  Deactivate item, leave child window, close popup"; Alt and Shift tweak slower and faster.
  Gamepad: D-pad "Move / Tweak", FaceDown "Activate / Open / Toggle", FaceRight "Cancel /
  Close / Exit", L1 "Tweak Slower", R1 "Tweak Faster". A slider tweaks only while active (the
  edit mode); a float steps 1 % of its range a press, an integer within a range of 100 steps
  one unit, fast is x10 and slow x0.1. Repeat: `KeyRepeatDelay = 0.275` s, `KeyRepeatRate =
  0.050` s, with navigation tweaks repeating faster (delay x0.72, rate x0.30). What we take:
  the edit mode's keys match ImGui's (`Enter` activates, `Up`/`Down` tweak, `Esc`
  deactivates; the in-place arrow tweak is not taken, owner question 9); our step is the
  value's own unit (0.1 m for the dig
  radius, 3 dB for a trim, 10 ms for the pre-delay, one iteration), since every number here
  has a natural unit; the repeat delay and rate are ImGui's 275 and 50 ms. Slow and fast
  modifiers are not taken: no number here has a range that needs them.
- **Microsoft, `SystemParametersInfoA` (`SPI_GETKEYBOARDDELAY`, `SPI_GETKEYBOARDSPEED`).**
  <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfoa>
  The OS keyboard repeat ranges from about 250 ms to 1 s of delay and 2.5 to 30 repeats a
  second. What we take: 275 ms and 20 a second sit inside the range every Windows user has
  felt; the menu does its own repeat because `svet_app::Input::pressed` ignores the OS repeat.
- **Microsoft, "Xbox Accessibility Guideline 107: Input".**
  <https://learn.microsoft.com/en-us/gaming/accessibility/xbox-accessibility-guidelines/107>
  "A UI should be navigable by using single, non-simultaneous key presses"; Minecraft's slider
  is the example: "tap A to select once, and then use ... single digital inputs from the D-pad,
  to adjust the slider", against games that make the player hold A while adjusting; "Avoid
  introducing mechanics where a key or button should be held down for an extended period".
  What we take: no chord and no hold to edit; holding an arrow to repeat is a convenience, a
  single press always steps once.
- **Not found:** a written rule for how a numeric row shows its range (`value / max`, arrows
  at the ends). XAG 102's praised examples (Minecraft's "Chat Text Opacity: 79%", Gears
  Tactics' bright knob) show the label, the value and a high-contrast marker together. We show
  `< value >` on the selected row and the range in grey after it.

## Enums and toggles

- **Nielsen Norman Group, Alita Kendrick, "Toggle-Switch Guidelines".**
  <https://www.nngroup.com/articles/toggle-switch-guidelines/> "Toggle switches should take
  immediate effect" with no save step; the label says what the control does when on; state is
  shown by more than colour. What we take: a toggle flips on `Enter` and its row reads the
  new state at once, as the rows already do.
- **Apple, Human Interface Guidelines, "Game controls".**
  <https://developer.apple.com/design/human-interface-guidelines/game-controls> Outside
  gameplay: "A: Activates a control", "Directional pad: Moves selection"; "Map controller
  buttons to expected UI behavior". What we take: `South` activates, the D-pad moves and, on a
  stepped row, tweaks; the same keys the keyboard has.
- **Microsoft, "Xbox Accessibility Guideline 114: UI context".**
  <https://learn.microsoft.com/en-us/gaming/accessibility/xbox-accessibility-guidelines/114>
  Labels sit beside their control; Forza's accessibility menu cycles a row's options with the
  current one shown on the row. What we take: a cycle shows its current state on its row and
  `Enter` is the next state, one press per state, with no edit mode (the owner's call); the
  `<` `>` marker is a number's, while it is being edited.
- **Dear ImGui, `imgui_internal.h`, `ImGuiNavMoveFlags_LoopX` and `WrapX`.**
  <https://github.com/ocornut/imgui/blob/master/imgui_internal.h> Wrapping the selection is an
  opt-in flag, off by default. **Not found:** a guideline on whether a cycle wraps. We wrap a
  cycle (as the keys always have) and clamp a number (a clamp at the range's end is the only
  reading of "no further" a number has).

## Unavailable entries

- **Smashing Magazine, Vitaly Friedman, "Hidden vs. Disabled In UX".**
  <https://smashingmagazine.com/2024/05/hidden-vs-disabled-ux/> "Disable if you want the user
  to know a feature exists but is unavailable"; the test is "Will a given user ever be able to
  interact with this element?" (yes: disable; never: hide); "explain why a feature is disabled
  and also how to re-enable it". What we take: every entry a flag unlocks is greyed with
  `needs --flag`, since the owner can relaunch with the flag; nothing is hidden.
- **Nielsen Norman Group, Huei-Hsin Wang, "Why Disabled Buttons Hurt UX (and How to Fix
  Them)".** <https://www.nngroup.com/videos/why-disabled-buttons-hurt-ux-and-how-to-fix-them/>
  "Disabled buttons often confuse users by appearing clickable but providing no response or
  feedback"; use them sparingly and "clearly explain why the button is disabled". What we take:
  this is the owner's complaint in the general form ("does nothing with no indication"); the
  reason is shown while the row is selected and again when `Enter` is pressed on it.
- **Microsoft, XAG 102 and XAG 103** (above): inactive text keeps 3:1 contrast; greying needs
  a second cue. What we take: `HudColour::Grey` at 0.6 on the backdrop, and the words.
- **Microsoft, "UX checklist for desktop applications" (Windows 7-era).**
  <https://learn.microsoft.com/en-au/windows/win32/uxguide/top-violations> "Disabled controls
  can be hard to use because users literally have to deduce why they are disabled." What we
  take: the same rule from the desktop side; the reason is never left to deduction.

## Feedback when an action happens

- **Nielsen Norman Group, Jakob Nielsen, "Response Times: The 3 Important Limits".**
  <https://www.nngroup.com/articles/response-times-3-important-limits/> "0.1 second: the user
  feels the system is reacting instantaneously"; 1 s keeps the flow of thought; 10 s keeps
  attention. What we take: a row changes in the frame of the press; an action that takes
  longer (the 30-frame readings, half a second at 60 Hz) says at once that it started and
  then what it found.
- **Nielsen Norman Group, "10 Usability Heuristics for User Interface Design" and Aurora
  Harley, "Visibility of System Status".** <https://www.nngroup.com/articles/ten-usability-heuristics/>,
  <https://www.nngroup.com/articles/visibility-system-status/> Feedback "within a reasonable
  amount of time", "ideally, immediately"; "Don't remove items silently." What we take: the
  notice row above the footer holds the last line an action printed, so a result that used to
  go to stdout alone is seen where the key was pressed; a greyed entry stays listed.
- **Apple, HIG "Buttons" and "Game controls".**
  <https://developer.apple.com/design/human-interface-guidelines/buttons> "Always include a
  press state for a custom button. Without a press state, a button can feel unresponsive";
  pair it with sound and haptics. What we take: the notice and the row change are our press
  state; a click sound is not taken (the testbed's audio is a measured subsystem, not UI).
- **Microsoft, XAG 101 and "Xbox Accessibility Guideline 115: Error messages and destructive
  actions".** <https://learn.microsoft.com/en-us/gaming/accessibility/xbox-accessibility-guidelines/115>
  Text settings carry over to overlays; "Make error and warning messages easy to visually
  distinguish from other text". What we take: the notice is yellow, the rows white, an
  unavailable row grey.

## A performance readout while the menu is open

- **Dear ImGui, `imgui_demo.cpp`** (above): the metrics overlay is a corner window with
  `NoNav`, auto-sized, on a 0.35-alpha background, offered in any corner. What we take: one
  readout row at the top right while the menu is open, never selectable, reading the frame
  window the loop already records.
- **Microsoft, XAG 102** (above): HUD elements are assessed for contrast too; The Outer Worlds'
  "bright yellow text against the opaque backgrounds of HUD elements". What we take: the
  readout keeps the glyph shadow; it sits outside the panel, so the owner may ask for a
  backdrop cell under it in the window.
- **Not found:** an authoritative source that top right is the conventional corner for an fps
  counter; Steam's and NVIDIA's overlays offer a corner choice (third-party guides describe
  Steam's four corners; the official page could not be reached). Top right is the owner's
  suggestion and the plan's default.

## The pad's buttons

- **Apple, HIG "Game controls"** (above): "A: Activates a control; B: Cancels an action or
  returns to previous screen; Left shoulder / Right shoulder: Navigates ... to a different
  screen or section; Directional pad: Moves selection; Menu: Opens game settings or pauses
  gameplay"; use the connected controller's own glyphs. What we take: `South` activates,
  `East` goes back, `Start` opens and closes; the shoulders are not taken (one tree, no tabs).
- **Microsoft, "Gamepad and remote control interactions"** (above): the built-in keyboard
  equivalents, "Enter -> A/Select; Escape -> B/Back; Arrow keys -> D-pad"; "If the B button is
  used to go back, then don't show a back button in the UI." What we take: the keyboard and
  pad tables are one table in `route.rs`; no back row in the menu.
- **Dear ImGui, `imgui.h`** (above): FaceDown is "A (Xbox) | B (Switch) | Cross (PS)" and
  `io.ConfigNavSwapGamepadButtons` swaps activate and cancel for a Nintendo layout. What we
  take: the D-pad's up and down tweak a number only inside the edit mode, left and right keep
  their page meaning; `svet_app::PadButton::South` and `East` are positional, so a Switch pad activates on
  its B as the layout expects; a swap setting is not taken (a backlog item if a Switch pad is
  ever plugged in). **Not found:** an official Nintendo statement of the layout; the fact is
  from ImGui's table.
- **Microsoft, XAG 107 and XAG 114** (above): every screen reachable with the same input as
  gameplay, digital and analog alike; bumpers switch tabs consistently where used. What we
  take: the pad reaches every entry the keyboard does, which `the_pad_walks_the_menu` holds.

## Esc and back

- **Dear ImGui, `imgui.cpp` "CONTROLS GUIDE"** (above): "Escape: Deactivate item, leave child
  window, close popup", innermost first. What we take: `Esc` leaves the edit mode, then clears
  the filter, then pops the page, then closes the menu.
- **Apple, HIG "Remotes" and "Game controls".**
  <https://developer.apple.com/design/human-interface-guidelines/remotes> Back opens "the
  parent of the current screen"; with a pause menu open, Back closes it and resumes; during
  play, since Back "can be easy to accidentally press", it opens the pause menu rather than
  leaving the game. What we take: `Esc` with the menu open never ends the run (owner question
  8 of the plan, answered 2026-11-16); closed, it quits as the fly-through always has, a
  testbed convention the path runs and the tester count on.
- **Microsoft, XAG 115** (above): Grounded warns on leaving settings with unapplied changes;
  "Offer players a mechanism to review, confirm, and undo permanent or destructive actions";
  "Don't require button holds for confirmation". What we take: nothing here is destructive and
  every change applies at once, so no apply or discard prompt; the confirmation question is
  Plan 0073's (a settings file).
- **Nielsen Norman Group, Jakob Nielsen, "Confirmation Dialogs Can Prevent User Errors - If
  Not Overused".** <https://www.nngroup.com/articles/confirmation-dialog/> "Do not use
  confirmation dialogs for routine actions"; prefer undo. What we take: none in the menu.
- **Raycast, "Search Bar" (the manual).** <https://manual.raycast.com/search-bar> "Escape:
  Clear search text, or close Raycast if already empty." What we take: the palette's `Esc`
  order, filter first.

## Command palettes

- **Visual Studio Code, "User Interface" and "Tips and Tricks".**
  <https://code.visualstudio.com/docs/getstarted/userinterface>,
  <https://code.visualstudio.com/docs/getstarted/tips-and-tricks> The Command Palette gives
  "access to all functionality within VS Code" from one key; typing filters; Quick Open finds
  any file or symbol by name. The v1.88 notes
  (<https://code.visualstudio.com/updates/v1_88>) describe the "recently used" and "other
  commands" groups. What we take: one key, every entry reachable by its words, which the
  palette does; recency ordering is not taken (tree order keeps the fixture exact; ADR-0061).
- **Visual Studio Code, UX Guidelines, "Quick Picks".**
  <https://code.visualstudio.com/api/ux-guidelines/quick-picks> "Use the description for
  displaying the current items"; brief detail; a placeholder hint in the input; avoid long
  multi-step flows. What we take: a palette row shows its group, its live value and its key;
  the title row shows `search: lam` as the input.
- **Raycast, "Search Bar"** (above): ranking by alias, then title, then keywords, then
  "frecency"; fuzzy strictness is a setting. What we take: not taken; exact substring per word
  (ADR-0061 rejected fuzzy matching for the fixture's sake).
- **Not found:** written rules for highlighting the matched substring (a visible behaviour of
  VS Code and Raycast, not a documented one). Not taken: the overlay colours whole rows.

## No silent no-ops

- **Game accessibility guidelines, "Give a clear indication that interactive elements are
  interactive".** <https://gameaccessibilityguidelines.com/give-a-clear-indication-that-interactive-elements-are-interactive/>
  Use "clear and consistent differences in style, contrast, or additional signifiers". What we
  take: a live row is white, an unavailable one grey with words; the two are never confused.
- **Microsoft, XAG 114** (above): "before activating a button, a player should already be
  aware that the interaction is going to ... change a specific aspect of their settings";
  "Provide context-sensitive help for each element on screen where necessary." What we take:
  the footer names what the keys do on the selected row (a number's `Up Down change`, a grey
  row's reason).
- **Nielsen Norman Group, Page Laubheimer, "Preventing User Errors: Avoiding Unconscious
  Slips".** <https://www.nngroup.com/articles/slips/> Constrain input, give good defaults, warn.
  What we take: a number clamps at its range instead of wrapping to the other end, so a held
  arrow cannot slip from 3.0 m to 0.1 m.
- **The plan's test.** The claim the sources converge on, that every activation answers, is
  held mechanically by Plan 0080 Phase 6's `every_live_entry_answers`: for every entry of the
  tree, `Enter` either changes the run's state, sets the notice, or is refused with a reason.
