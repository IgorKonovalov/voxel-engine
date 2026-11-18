# Designer authoring: external sources

An annotated reading list on letting non-programmers build levels and logic through text that a
separate compiler checks, with an editor as a view over the text and an agent as a second author,
collected 2026-11-17 while drafting ADR-0072. Every entry names what it is and what we take from
it, so a plan can cite it by title. Each fact was read on the page cited on that day unless marked
*unverified* (a search digest, a forum quotation or a secondary report); re-read before leaning
on an unverified line. Numbers are the sources' own.

## Graph editors that moved to text

- **Epic Games, "Coding in Unreal Engine: Blueprint vs. C++" (UE5 documentation),
  dev.epicgames.com/documentation/unreal-engine/coding-in-unreal-engine-blueprint-vs-cplusplus.**
  Epic's own comparison: Blueprints "compile into bytecode that executes on a virtual machine",
  the gap "usually insignificant" except for Tick-heavy classes with many instances and no
  threading; C++ "is stored as text, making it easier to diff, merge, and share between
  projects". What we take: the vendor lists text storage as a first-class advantage; the "ten
  times slower" figure in circulation has no primary source (*unverified*) and is not cited.
- **Epic Games, Unreal Engine 5.0 release notes on Blueprint nativization (quoted on the Epic
  forums, 2022; the notes page itself not read).** "Removed underlying framework and support for
  the Blueprint nativization feature". Community reasons (*unverified*): all-or-nothing build
  failures on nodes that could not be nativized, generated code slower than hand-written C++ and
  harder to debug. What we take: a graph-to-native transpiler added later was dropped by the
  largest vendor; a designer language's compiler is the primary path from the first day, every
  construct with a defined lowering and a source position for its errors.
- **Epic Games, "UE Diff Tool in Unreal Engine" (documentation) and SeanAblur, "[Open Source]
  BPMerge", Epic forums, 2026-05-15.** The diff tool exists because "a textual representation
  would not be constructive"; it does not merge. Twelve years on, a third party ships a
  three-way merge that "extracts a semantic IR from the .uasset" with "12 explicit conflict
  rules" and GUID-based rename tracking. What we take: an editor-owned binary format costs a
  semantic IR, conflict rules and rename tracking to recover what `git merge` gives text; names
  in the text are stable identifiers so renames merge.
- **Juan Linietsky, "Godot 4.0 will discontinue Visual Scripting", godotengine.org,
  2022-08-23.** Built for prospective users who asked for Blueprints; "it never gained
  traction", a poll of over 5000 users found 0.5 % on it as their main language. The causes:
  GDScript was easy enough to be the on-ramp, the graph shipped no high-level game features, and
  the docs could not carry it. What we take: a node graph that transcribes a general language
  one to one serves nobody; the visual layer worth having is domain-level (spawn, trigger, door)
  over a text language that is itself learnable.
- **Endless OS Foundation, `godot-block-coding` (GitHub README), 2024.** A block-based,
  high-level, educational layer over Godot 4.3+; its open question "Should blocks generate
  GDScript or be dynamically executed?" and its warning that upgrades may orphan block scripts.
  What we take: blocks lower to the text language so they never become a second truth, and the
  text format is versioned so upgrades do not orphan designer work.
- **Unity, Visual Scripting 1.9 manual: "AOT platforms", "Version control systems", the
  package changelog; roadmap item "Replace serialization stack" (*unverified*, a snippet dated
  2025-04-02).** Graphs name host methods reflectively, so stripping breaks them and an
  `AotStubs.cs` is generated to keep them alive; the serialiser is a third-party library the
  roadmap says is "not supported anymore". A user thread (Unity Discussions, 2024-12 to 2025-04)
  reports graphs that vanished. What we take: avoid a format that binds host functions by
  string and late; compile designer text against a declared interface (the WIT world) so the
  host calls are known statically and the toolchain owns the format.
- **John Popko, "Epic Games will remove Blueprints and Actors from Unreal Engine 6", Inven
  Global, 2026-11-03 (secondary reporting of an Unreal Fest Tokyo keynote).** Sweeney is
  reported saying gameplay logic moves to Verse in UE6, Epic "feeling the pain first" by porting
  its own Blueprints, with assistant tooling among the motives. What we take: reporting, not an
  Epic document; the direction, graphs to text for everyone, matches the rest of this list.

## Verse

- **Peyton Jones, Sweeney, Augustsson, Claessen, Jhala, Shivers, "Beyond Functional
  Programming: The Verse Programming Language", Haskell eXchange, December 2022 (slides).**
  "Learnable as a first language", transactional "from the get-go", "compile time guarantees
  that a module subsumes the API of the previous version"; a functional logic language with an
  effect system; "a creation platform open to all programmers, artists, and designers". No
  visual front end appears in the deck. What we take: Epic's answer for creators is a text
  language whose headline features are stability across versions and rollback on failure, both
  of which an op-log engine can offer a designer language cheaply.
- **Augustsson, Breitner, Claessen, Jhala, Peyton Jones, Shivers, Steele, Sweeney, "The Verse
  Calculus: A Core Calculus for Deterministic Functional Logic Programming", ICFP 2023.**
  Deterministic by construction: multiple values "are returned in a well-specified order",
  conditionals "are driven by failure rather than booleans". What we take: determinism as a
  language property, not an engine afterthought; a designer language has no construct whose
  order or effect is unspecified.
- **Epic Games, "Verse language reference" (UEFN documentation; page body not fetchable,
  *unverified* snippets).** "Failure is control flow"; actions inside a failure context "will
  be undone if a failure occurs". What we take: the concrete shape of rollback for designers, an
  `if` whose body can fail and whose edits are undone. The effect names (`<transacts>`) are
  third-party glossary material, *unverified*.

## Text languages for creators

- **Roblox, "Luau Origins and Evolution" (luau.org, 2022-11-04) and "Why Luau?" (luau.org).**
  Lua chosen for simplicity; "a requirement to still be suitable for beginner programmers guides
  our evolution direction"; gradual typing with a non-strict mode "as helpful as possible for
  programs that are written without type annotations"; autocomplete reworked "to use the same
  type inference engine that the type checking / analysis tools use"; Lua's parser rebuilt
  around an AST for analysis. What we take: one front end (parser and checker) shared by the
  compiler, the linter and the editor from the start, and backward compatibility as a rule. The
  language is open source (MIT, 2021) with `luau-analyze` as a standalone checker and linter, so
  the Lua option in ADR-0072 would reuse it.
- **mlua (GitHub README and docs.rs).** Embeds Lua 5.1 to 5.5, LuaJIT and Luau in a Rust host;
  sandboxing is Luau-only; execution limits differ per flavour (`set_hook` versus Luau's
  `set_interrupt`); "mlua does not provide absolute safety even without using unsafe". What we
  take: `mlua` is an in-process embedding beside the component sandbox, not a replacement for
  it; the roadmap's deferred client-script host, not the designer stack's runtime.
- **Rhai book: "Features", "Safety and Protection", "Maximum Number of Operations",
  "Sandboxing", "Building to WebAssembly" (rhai.rs).** An AST-walking interpreter with operation
  budgets (`set_max_operations`, one operation "roughly equals one expression node"), sandboxed
  by an immutable engine; "use Rhai when compiling to WebAssembly" means the engine runs in
  wasm and interprets scripts there. Determinism is not discussed (*unverified*). What we take:
  a second interpreter inside the component doubles the sandbox; Rhai's value here is as a
  precedent for operation-counted budgets.
- **Rune, Koto, Mun (GitHub READMEs, koto.dev, mun-lang.org).** Rune: a stack VM with hot
  reload; Koto: "still some way from a 1.0"; Mun: "compiled ahead of time", statically typed,
  "every aspect ... designed with hot reloading in mind", LLVM back end, all releases
  pre-releases, the blog's last post the v0.4.0 release of 2022-12-12 (dormant; the v0.5.0 year
  *unverified*). None compiles scripts to wasm. What we take: the one language with the right
  design for us stalled on its maintenance cost, a warning for a purpose-built language.
- **Tuxedo Labs, "Teardown Modding" and "Teardown scripting API (2.2.0)" (teardowngame.com).**
  A mod is a folder: `info.txt`, `main.xml` ("all level content is stored as standard XML"),
  `main.lua`, `vox/` of MagicaVoxel models; scripts take typed parameters declared in the level
  XML (`GetIntParam`, `GetFloatParam`, ...); callbacks `init`, `tick`, `update`, `draw`, split
  into `server.*` and `client.*` in 2.2.0; placed objects are found by tags; the in-game editor
  mirrors the XML and runs the level on F5. The element that attaches a script was not read on a
  documentation page (*unverified*). What we take: the closest voxel precedent is the layered
  text stack of ADR-0072, with typed parameters in the level file and tags as the join between
  placed things and logic; Teardown ships no checker and documents no sandbox, which is the part
  we add.
- **Facepunch, s&box developer documentation: "Code" and "ActionGraph" (sbox.game).** Games
  are C# with a hotload "within milliseconds" and an API whitelist; ActionGraph is a visual layer
  that compiles into the same runtime and exposes C# methods as nodes. What we take: one
  runtime, the visual layer a client of the language; even so a second, simpler event-wiring
  layer was added (*unverified*), so the right split is by task scope.

## Narrative languages: text in, compiler, thin VM, editor as a view

- **Yarn Spinner (GitHub: YarnSpinner, `yarn_spinner.proto`, YarnSpinner-Console, the VS Code
  extension README; docs.yarnspinner.dev `DebugInfo.LineInfo`).** "Easy for writers to use";
  `ysc compile` emits a `.yarnc` program (a versioned protobuf of instructions), a strings table
  and a metadata file; per-instruction `LineNumber` and `CharacterNumber` back to the source;
  the editor runs the full compiler for its red underlines, declares host commands in a
  `.ysls.json` side file, and shows a graph view (whether the graph writes back to text is
  *unverified*). What we take: the shape of ADR-0072's layer 3 exists and works: text source, a
  separate compiler, a versioned artifact with a source map, a thin runtime, and the editor a
  view that runs the same compiler; the host's commands as a data file the compiler, the editor
  and an agent all read.
- **inkle, "ink" (inklestudios.com/ink; GitHub inkle/ink and inkle/inky).** "Markup, not
  programming": text comes first, logic is inserted; `inklecate` compiles to JSON; Inky bundles
  the compiler, highlights errors as you type, and on recompile "fast-forwards" the choices
  already made. What we take: the editor bundles the compiler so writer and build see the same
  diagnostics, and recompile-and-fast-forward is the designer's hot reload, cheap when the
  runtime is deterministic and replayable, which ours is through the op log.

## Level formats

- **Sébastien Bénard, LDtk "JSON overview" (ldtk.io) and "1.1.0 Introducing Super Simple
  Export" (itch.io devlog, 2022-04-01); `docs/JSON_SCHEMA.json` (GitHub deepnight/ldtk).** A
  project is one JSON file (`defs` and `levels`), its documentation and JSON Schema "directly
  generated from latest source code", every field described, computed fields prefixed `__`,
  editor-only sections marked "you can safely ignore", a documented deprecation cycle; the
  simple export is a flat per-level file because of "this moment when you need to write the
  importer". What we take: a schema generated from the Rust types with their doc comments,
  versioned with the format; a rich authoring file and a flat engine-facing artifact (ours is
  the wire encoding).
- **Thorbjørn Lindeijer, Tiled "TMX Map Format" and "JSON Map Format" (doc.mapeditor.org);
  "Reference format files" (Tiled forum, 2022).** Layer data as CSV or base64 with
  compression, a per-file switch; parsers should "ignore unknown elements and attributes (or
  raise a warning)"; `version` and `tiledversion` fields; no official JSON Schema because
  "JSON Schema does not really support inheritance". What we take: the ignore-unknown rule as
  the forward-compatibility contract between a compiler and an editor that evolve apart; tagged
  unions rather than inheritance so a schema can describe the variants.
- **Carter Anderson, "Next Generation Scene / UI System" (bevyengine/bevy discussion #14437,
  2024-07-22) and the Bevy 0.19 release notes (bevy.org, 2026-06-19).** BSN: a Rust-like scene
  syntax where "a BSN expression is a 'patch'" over defaults, scenes inherit and override,
  `.bsn` files carry versioned imports because they lack compile-time context; 0.19 ships the
  `bsn!` macro only, the file loader still owed. What we take: patches over defaults keep files
  and diffs small; a version in the file makes it readable without the compiler's context; and
  two years from proposal to a macro with no loader shows the cost of one syntax serving both a
  Rust macro and a standalone file.

## Rule and chart languages

- **Stephen Lavelle, PuzzleScript documentation, "Rules" (puzzlescript.net).** A game is one
  text file in eight sections; "rules are about pattern replacement" over a grid, matched in
  every orientation, "the only things that are removed are things that are referred to", rules
  emit commands. **Boris the Brave, "PuzzleScript Rules", boristhebrave.com, 2024-06-10 (updated
  2025-10-16)**: the economy of find-and-replace and motion as state, against undefined object
  identity across the arrow, swap rules that leak tags, and games of hundreds of rules that
  "become hard to write and debug"; replacement rules "can't serve as sole source of logic".
  What we take: the smallest logic language non-programmers demonstrably learn maps onto a
  material table (a neighbourhood predicate to an edit op), and a rule layer must carry identity
  across the arrow and sit over a general layer rather than grow into a bad one.
- **Graham Nelson, "Natural Language, Semantic Analysis and Interactive Fiction" (IF Theory
  Reader), read through Ethan Robison's course summary (*secondary*); simvig, "The experience of
  a first-time author in Inform 7", intfiction.org, 2012-08-29.** The defensible part of Inform
  7 is the rulebook model (named rulebooks, rules added, removed and reordered by declaration),
  not the English surface; the first-timer's complaints are "the error messages where very
  general", a minute's compile for a small change, discoverability, and the compiler
  second-guessing intent. What we take: declarative rules over a small world model; never
  pretend the syntax is English; the compiler's messages and turnaround are requirements, and a
  generated index of what the compiler understood is worth shipping.
- **W3C, "State Chart XML (SCXML)", Recommendation 2015-09-01; Erik Mogensen,
  statecharts.dev.** Nested `<state>`, `<parallel>`, `<transition event= cond=>`,
  `<onentry>`/`<onexit>`, a pluggable data model ("null", "ecmascript", "xpath"); the chart is
  standardised, the action language is not. What we take: the chart as the text designers own
  and an editor can draw, with guards and actions as names that resolve to compiled functions;
  not the XML, and no embedded script.
- **Davide Faconti, BehaviorTree.CPP "XML format" and Groot2 (behaviortree.dev).** A tree is
  XML; a `<TreeNodeModel>` block declares each node's ports because "the executor does not
  need it, but Groot does"; since 4.11 generated XSD and Schematron schemas validate structure
  and cross-references; Groot2 edits the same XML the executor loads and monitors a running
  tree over a wire. A byte-identical round trip is *unverified*. What we take: the working
  example of an editor over the runtime's own text, and its price: a machine-readable node model
  exported by the host and a schema for the round trip; our verb table is that model.
- **Media Molecule, Dreams "Logic & Processing" gadgets (docs.indreams.me) and Jack Yarwood,
  "How Media Molecule designed a fun and robust toolset for Dreams", Game Developer,
  2019-06-17.** Counter, Timer, Selector, gates, Calculator, Exclusive Gate ("let only one
  signal through at a time"), microchips as the grouping primitive, advice to name variables
  "wordy and specific ... to avoid accidental conflicts"; logic "certainly did come after"
  sculpting, and the first advice to a creator is "it's really hard". Gadget caps and the phrase
  "logic spaghetti" are *unverified*. What we take: the gadget vocabulary as a designer
  language's standard library, and the observation that wire logic needed grouping, naming
  discipline and budget meters that scopes, modules and a compiler give text for free.
- **Minecraft Wiki, "Function (Java Edition)"; vberlier, beet and bolt (GitHub).** A function is
  a `.mcfunction` text file, one command per line, run by tags in defined order or by
  `/schedule`; parameters arrived only in 1.20.2 as `$(key)` macro lines; no loops or structured
  branching, so compile-to-datapack languages (bolt: "Supercharge Minecraft commands with
  Python") fill the gap. What we take: the largest non-programmer logic community writes a lot
  of plain text when the engine reloads it at once and the nouns are the game's; a format
  without functions, parameters and scope breeds a dozen incompatible compilers, so the designer
  language has them from day one.

## WebAssembly as the boundary

- **Bytecode Alliance, "The WebAssembly Component Model" book, "WIT" chapter
  (component-model.bytecodealliance.org); wit-bindgen (GitHub README).** WIT "defines only
  contracts between components"; packages carry a semver id; WASI 0.2.0 (2024-01-25) is a stable
  pin; wit-bindgen generates guest bindings for Rust, C, C++, C#, Go and MoonBit, and the host
  side is `wasmtime`'s `bindgen!`. What we take: `vxe:logic@0.1.0` is already the stable seam;
  the compiler's only job is to emit a component for it; a new source language gets a component
  cheaply if it lowers to a supported guest language or emits core wasm plus the canonical ABI
  through `wasm-tools`.
- **Bruno Garcia, WASM-4 documentation (wasm4.org).** A fantasy console whose carts are
  `.wasm` files under 64 KB; thirteen-plus languages target it with no per-language work because
  the host owns a tiny ABI. What we take: keep the WIT world small and dull, and the designer
  language stays replaceable.
- **wasvy (docs.rs) and Unvestigate, "How I made my game moddable using Zig and WebAssembly",
  ziggit.dev, 2026-03-20.** Component-model mods for Bevy ("mods can be written in almost any
  language"); a WAMR-hosted mod loader whose author regrets that the runtime "can't time limit
  the execution". What we take: the field converged on wasm as the mod container and not on a
  designer language above it; the pain points named (limits, interface churn) are what fuel and
  a versioned WIT already answer here. No published game-logic DSL with a wasm back end was
  found.
- **chumsky (docs.rs), logos and LALRPOP (GitHub).** chumsky: error recovery ("one run can
  report several errors and still produce a partial syntax tree"), labelled patterns, spans,
  Pratt parsing, a check-only mode, an Ariadne integration; logos: derived lexers with spans;
  LALRPOP: LR(1) grammars with weaker recovery. What we take: a designer language is decided by
  its errors, so a lexer and a recovering parser with spans (logos and chumsky) over a grammar
  generator; the check-only mode is the editor's live validation.

## Agent-friendly tooling

- **The rustc book, "JSON Output" (doc.rust-lang.org/rustc/json.html).** One JSON object per
  line with a `$message_type`, a `code`, a `level`, `spans[]` (file, byte and line/column
  offsets, `is_primary`, `label`, `suggested_replacement`, `suggestion_applicability` up to
  `MachineApplicable`), `children[]`, and `rendered`, the human text carried alongside; parsers
  tolerate new fields. What we take: the diagnostic format to imitate almost literally, so the
  human and the agent read one record.
- **OASIS, "SARIF 2.1.0" (Static Analysis Results Interchange Format), 2020, errata 2023.**
  `runs[]`, `results[]` with rule id, level, message, `physicalLocation`, optional `fix` and
  `fingerprints` for cross-run deduplication; ingested by GitHub code scanning and the rest.
  What we take: an export format for CI annotation and "same error as before"; not a second
  schema for the live loop.
- **miette 7 (docs.rs).** One `Diagnostic` trait (code, severity, labelled spans, help,
  related) with graphical, narratable and JSON reporters. What we take: one diagnostic type,
  many renderers, from the compiler's first error; no formatted strings.
- **Microsoft, VS Code "Contribution Points: jsonValidation"; KDL 2.0 (GitHub kdl-org/kdl).**
  A published schema gives any JSON file validation and completion in the editor by a
  `fileMatch` mapping; KDL has its own young schema language. What we take: whichever syntax
  humans write, a schema the editor and the compiler both validate against is the agent-facing
  truth, and JSON gets that tooling free today.
- **Eyal Kalderon, tower-lsp 0.20 (docs.rs).** One struct implementing `LanguageServer`
  (`initialize`, then opt-in `completion`, `hover`, diagnostics) served over stdio. What we
  take: a minimal server is a few hundred lines over the compiler library; the agent never needs
  it, only the JSON it would show.
- **CoplayDev, "MCP for Unity" tool reference (coplaydev.github.io/unity-mcp, 2026-10).** Fifty
  tools; the ones that matured are `validate_script` with diagnostics, `read_console`, a SHA
  per file for staleness, batching, and async jobs with an id to poll; `manage_tools` exists to
  shrink the list per session. What we take: validate, read the log, hash, batch, job ids; a
  small stable CLI over a sprawling tool catalogue.
- **Siddharth Ahuja, blender-mcp (GitHub README).** Nine tools, of which agents lean on
  `execute_blender_code` (with a "save your work before" ritual and an optional static check
  blocking file and network access) and `look` (viewport, multi-angle contact sheet, one line
  per object). What we take: the compact text summary beside the image, and a sheet of several
  views over one still; the sandbox ritual is what a fuel-metered component makes structural.
- **Godot MCP servers: Coding-Solo/godot-mcp, Erodenn/godot-runtime-mcp (READMEs and a
  listing), a vendor comparison (ziva.sh, 2026-11-16).** `run_project` plus `get_debug_output`;
  a runtime bridge with `take_screenshot` that returns "pixel statistics that flag likely blank
  frames" and `simulate_input` where "each action reports what it hit and what changed"; limits:
  screenshots need a display, output capture depends on who spawned the process, no headless
  mode. What we take: one call that runs a scene with scripted input and returns frame, console
  and errors together; a blank-frame statistic on every still; the report written to a path,
  never scraped from a pipe.
- **chongdashu/unreal-mcp (GitHub README, experimental).** Most of its tools rebuild a text API
  over Blueprint's binary graph (node creation, variables, wiring). What we take: a text-first
  level and logic format makes that machinery unnecessary; the text is the API.
- **Taesiri, Ghildyal, Zadtootaghaj, Barman, Bezemer, "VideoGameQA-Bench", arXiv 2505.15952,
  2025.** A benchmark of game stills and clips for vision-language models over visual unit and
  regression testing, glitch detection and bug reports; results mixed. What we take: whether a
  model sees a regression in a still is measured, not assumed; fix the camera, make the frame
  deterministic, compute the diff numerically, and ask the model to interpret a flagged
  difference image.
- **Wang, Tang, Yuan, Yu, Xie, Bu, "TITAN: Leveraging LLM Agents for Automated Video Game
  Testing", arXiv 2509.22170, 2025.** An agent in eight QA pipelines reads a structured text
  state (location, objectives, vitals in coarse buckets, nearby interactables) and takes
  screenshots only when stuck; its stuck report carries the goal, the actions tried, the
  suspicion and the evidence; 30 % false alarms from imperfect abstraction. What we take: the
  headless run report is text state first (ops, counts, events per tick), stills as evidence,
  and every failure carries its action trace so the report doubles as a replay.
- **Latent Space, "Claude Plays Pokémon" (2025-03).** Each action returned the frame, a copy
  with a coordinate overlay, and a state description read from memory; a navigator tool moved to
  a coordinate because the model "repeatedly tried to walk through walls"; a doormat was taken
  for a dialogue box for hours. What we take: a coordinate overlay on stills meant for an agent,
  authoritative text state beside every image, and high-level verbs over raw inputs; never a
  loop whose only evidence is the picture.
- **Doron Singer, "Part 5: Debug the Spire", DEV Community.** A headless re-implementation
  diverged from the real game; an agent built a harness that ran both on the same seed and
  diffed state after every step, about 100 bugs in eight days, 15 seeds at parity; the quirks
  were uninitialised fields, an extra random draw, and real-time dependencies that never fired
  headless. What we take: "same seed, two builds, diff every step" is the most agent-leverageable
  harness in this list, and we own its preconditions (deterministic `vxe-world`, the op log,
  the native-versus-component equality test); anything that reads time breaks it.
- **Max Polaczuk, "Stop Benchmarking AI Coding Agents on Todo Apps. Make Them Build an MMO.",
  DEV Community.** A 48-hour agent build whose enablers were one deterministic simulation
  shared by client, server and a headless host, no wall clock or `Math.random()`, one interface
  for the headless and windowed paths, scripted parties as regression tests, procedural assets
  over a binary pipeline. What we take: every enabler is a property of the engine, not of the
  agent; scripted bots driving the controller through a level file are the regression test to
  add.
- **Factorio, "Friday Facts #188: Bug, Bug, Desync" (2017-04-28) and the Desync wiki page.**
  Deterministic lockstep; a desync report bundles both saves and the log, "does not state the
  cause", and is a record to diff; heavy mode saves and reloads every tick and dumps the state
  on any change, catching state that lives outside the save. What we take: on a determinism
  failure write both states, the input log and the engine log to one folder and ship the diff
  tool; a heavy-mode test tier (serialise and reload every N ticks, assert bit-identity) for
  state a component is tempted to keep outside the world.
- **Subsecond 0.7 (docs.rs) and Christian Kjær, "Rust hot reloading", codethoughts.io,
  2026-02-07; chamons/game-hotreload-example (GitHub); dexterous_developer 0.3 (docs.rs);
  godotengine/godot PR #85373 (2023-12).** Native hot patching forbids struct layout changes and
  resets thread-locals; the wasmtime example instead serialises the game state, drops the
  instance, instantiates the new component and restores, the host owning window and GPU;
  dexterous_developer serialises reloadable state with a format "so their schemas can evolve";
  Godot's own reload shipped a stale-cache bug and keeps only exported state. What we take: the
  component is the unit of replacement; reload is serialise, swap, deserialise (or replay from
  the log, which needs no state format at all); the surviving state is declared, everything
  else resets, and the reload path is tested, not only the first load.
