# ADR-0058: The apps keep render settings in a TOML file in the user's config directory, layered over a profile through an app-side field table

> **Status:** proposed
> **Date:** 2026-11-12
> **Related plan(s):** [Plan 0073](../plans/0073-a-settings-menu-and-file-for-the-fly-through-and-the-tester.md)

## Context

Plan 0069 gives `vxe-render` one settings surface: `RenderSettings`, its leaf table
`RenderSettings::FIELDS` with an apply class per leaf (`Live`, `Settle`, `Rebuild`; the
2026-11-12 amendment removes `Delete` rows), and the named profiles. ADR-0055 rejected a
string-keyed settings file in the engine: the consumer serialises the struct itself. The owner
answered on 2026-11-12 that the fly-through and the tester get an in-window menu and a settings
file, the engine stays file-free, the file is TOML through `serde` and `toml` pinned in the apps
only, unknown keys warn and missing keys take the profile's value. The owner's standing rule of
the same day makes every cheap exploration mode a render setting with its picked default (Plan
0067's bake budget `frame:1.5|second:N`, its missing-light fallback, full-first radius and reach),
so the field list grows with every plan.

Four things are still open once the format is fixed. How the app maps a TOML key to a field of a
struct the engine will not derive `Deserialize` on. Where the file lives, given that the tester
ships as an archive a tester unpacks anywhere and replaces with each release, and runs the
fly-through as a child whose working directory is the report folder (Plan 0068). Which runs read
the file, given that the goldens and the plans' readings run the fly-through headless on the dev
box, where a settings file will exist. Whether the menu writes the file, and when.

## Decision

The fly-through's library holds the settings layer, which the tester reuses through its existing
dependency on `flythrough`. A **field table** names one row per `RenderSettings::FIELDS` leaf, with
its kind (bool, integer range, float steps, choice list, or text parsed by the same function as
the leaf's command-line flag), its flag name, and a getter and a setter over `&mut RenderSettings`.
A CPU test fails when a `FIELDS` leaf has no row. TOML keys are the leaf names as nested tables
under `[render]`, beside a top-level `profile`. The effective settings are built as **the
profile < the file < the flags < the in-window edits**: the base profile is the one named by the
highest layer that names one (`Floor` when none does), and each layer's edits apply over it in
that order, so a missing key keeps the profile's value. The file lives in the **user's config
directory**, `$XDG_CONFIG_HOME` (or `$HOME/.config`) on Linux and `%APPDATA%` on Windows, under
`vxe-flythrough/settings.toml` or `vxe-tester/settings.toml`, found from the environment with no
new dependency. **A windowed run reads it; a headless run or a `--tester` child reads only a file
named by `--config`**, and `--no-config` reads none. **The menu writes the file only on its save
row**, with the profile and the file's and the window's edits that differ from the profile, never
the flags. The tester writes each run's effective settings into the run's folder and passes that
file to the child with `--config`, so the report carries what each run used.

## Consequences

### Positive
- The engine gains nothing: no dependency, no string-keyed API, ADR-0055 stands as written.
- A new `FIELDS` leaf fails the row test until it has a row, so every setting is in the file and
  the menu as soon as it exists, which is the owner's rule for exploration modes made mechanical.
- A non-scalar setting has one syntax: the file's string is the flag's argument, parsed once.
- The goldens and the readings do not depend on the dev box's file, because headless runs never
  read the default path.
- A tester report holds each run's settings, so a reading names the settings it was taken at.
- The file survives a new tester archive and does not need a writable install folder.

### Negative
- The field table restates per leaf what the struct already knows (its type and its path). It
  is kept honest by the row test and a round-trip test, not by the compiler.
- `serde` and `toml` enter the workspace table, the first file-format dependency of the apps
  (the fly-through's `run.json` is written by hand). `cargo deny` covers them.
- Saving rewrites the file: a comment the owner wrote by hand is lost (the header says so).
- A setting renamed by a later plan (Plan 0071 renames types with deprecated re-exports) turns
  a saved key into an unknown key that warns until the file is saved again.
- A file in the config directory is out of sight; the menu's footer and `--settings` print its
  path.

## Alternatives considered

### Alternative A: name-keyed access in `vxe-render` (`RenderSettings::get` and `set` by leaf name)

The engine would carry the getters and setters by name, with a small value enum, and both the
apps and a game's options screen would reuse them. It lost because it is the string-keyed
surface ADR-0055 kept out of the engine, it moves type errors to run time inside the engine's
API, and the owner decided the consumer owns serialisation. The apps' table can move into the
engine later by a superseding ADR if a second consumer writes the same table.

### Alternative B: `serde` derives on `RenderSettings` behind a `vxe-render` feature

The file would map onto the struct with no table. It lost on the owner's answer: the dependency
stays in the apps. It would also tie the file's keys to Rust field names and give no place for
the flag syntax, the menu's steps or the choice lists.

### Alternative C: the file beside the executable, or in the working directory

It is easy to find and goes with the archive. It lost because each tester release is a fresh
folder, so the settings vanish with the old one; an install folder may be read-only; and the
tester's children run with the report folder as their working directory, so a relative file
would be a different file per run.

### Alternative D: the menu saves on every change, or never

Autosave would persist an exploratory key press or a menu step the owner did not mean to keep,
and a file that changes under every toggle cannot be compared between runs. A menu that never
writes leaves the owner editing TOML by hand for every setting worth keeping. An explicit save
row costs one line on the menu.

## Outcome

_(Added only at acceptance if implementation falsified something above.)_
