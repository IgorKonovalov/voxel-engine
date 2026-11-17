# ADR-0071: The engine takes a name: one word that is the project, the crate prefix, the repository and the archive

> **Status:** proposed (the owner picks the word from the shortlist below; the Decision's first
> sentence is filled in before acceptance)
> **Date:** 2026-11-17
> **Related plan(s):** a rename plan, drafted after the pick (none yet)

## Context

The project is called `voxel-engine` and its crates `vxe-*`. Both are descriptors, not names.
GitHub lists 1394 repositories named `voxel-engine` (search `voxel-engine in:name`, 2026-11-17),
the largest `max-mapper/voxel-engine` at 1315 stars; a reader who hears the name cannot find
us. `vxe` fares worse: on GitHub and npm it is the Vue table library `vxe-table` (8635 stars)
and its family (`vxe-pc-ui`, `vxe-table-plugin-*`), so every search for our prefix lands in a
spreadsheet widget. The `vxe`, `vxe-world` and `vxe-render` names are free on crates.io
(checked 2026-11-17), which only says nobody else wanted them.

The rename is cheapest now. Nothing of ours is on crates.io. The first release is v0.54.0
(2026-11-15), the current one v0.59.0, and the tester archives ship as
`vxe-tester-<version>-<os>-x86_64` to a handful of testers. There is one consumer
(`~/Work/voxel-roguelite`, path dependencies), no docs site and no second consumer. Each of
those, once it exists, pins the name harder: a published crate cannot be renamed, only
re-published and deprecated; a docs site's URLs are in other people's bookmarks; a second
consumer has `vxe_world::` in its code.

What the name must carry is the engine, not the first game. The engine is honest small voxels in
carved rock, deterministic edit operations that report what they changed, light that holds
still, and sound probed through the same material table. Its first consumer is a sound-horror
delve, and `crates/` names no gameplay concept, so neither does the name: no "delve", "dungeon",
"crypt" or "horror" word, and nothing that only makes sense once the game is known.

The cost is measured on `main` at e7b7b609 (2026-11-17). `vxe` appears 5260 times in 451
tracked files (250 `.rs`, 168 `.md`, 22 `.toml`, 3 `.sh`, 2 `.mjs`, 2 `.yml`, 2 `.voxtext`, 1
`.wit`, 1 `.wgsl`, 1 `.txt`), and there are 84 distinct `VXE_*` environment variables. Most of
it is a mechanical substitution. The parts that are API are the environment variables (a
tester's scripts, `docs/flythrough.md`, the `check-test-tiers.mjs` gate that counts `VXE_*`
names per test file), the WIT package `vxe:logic@0.1.0` (`crates/vxe-script-wasm/wit/logic.wit`,
which every logic component names), the window title `"vxe"` in `vxe-app`'s shell, the archive
and artifact names in `.github/workflows/release.yml` and `scripts/package-tester.sh`, the
`repository` field in `Cargo.toml`, the README badge and the regenerate commands under the
hero image.

Two external rules bound the mechanics. **crates.io** forbids reservation: its policy lists,
among the content it removes, a crate that "exists only to reserve a name for a prolonged period
of time (often called 'name squatting') without having any genuine functionality, purpose, or
significant development activity on the corresponding repository", and says the team "may delete
crates from the registry that do not comply", in most cases after giving "the author the chance
to justify the purpose of the crate" (https://crates.io/policies, read from the page source
2026-11-17). A publish can be undone by its owner within 72 hours, or later while the crate has
one owner, under 1000 downloads a month and no dependants. So a placeholder crate is against the
rules, and a real `0.x` publish is the only way to hold a name; the same rule is also the lever
against a placeholder someone else holds. **GitHub** redirects a renamed repository: "all
existing information, with the exception of project site URLs, is automatically redirected to
the new name", and `git clone`, `fetch` and `push` against the old location "continue to
function"; it does not redirect a GitHub Pages project URL or an action referenced by `uses:`,
and the redirect dies if a new repository takes the old name
(https://docs.github.com/en/repositories/creating-and-managing-repositories/renaming-a-repository,
2026-11-17). Neither exception bites us today: there is no Pages site and no reusable action.

## Decision

We give the engine one word, and that word is the project name, the crate prefix, the
repository name, the environment-variable prefix, the WIT package namespace and the tester
archive's name, changed together in one plan. **The word is the owner's pick from the
shortlist below; the recommendation is `karst`, with `doline` as the runner-up.** Until the
pick, the project keeps `voxel-engine` and `vxe-`.

The criteria, in the order they were applied:

1. **Says the engine.** Rock, hollows, carving, light or sound; not the first game and not a
   gameplay word.
2. **One or two syllables, five or six letters at most, no diacritics**, spelled the way it
   sounds, and pronounced about the same in English, German, Russian, Spanish and French. It is
   typed as a crate prefix thousands of times.
3. **Free as a prefix on crates.io**: `<name>-world`, `-render`, `-app`, `-sim`, `-assets`,
   `-net`, `-script-wasm`, `-audio`, `-entity` all unregistered. The bare `<name>` matters less
   (the engine ships no umbrella crate) but a bare crate in the same field counts against it.
4. **No engine, renderer, game library or notable software product of that name.** A small game
   or a company in another industry is noted, not disqualifying; a trademark in another class is
   not a conflict for an open-source library.
5. **No unfortunate meaning** in a major language.
6. **A domain is a hint only.** `.dev` and `.org` are registered for almost every real word; a
   free `.io` or `.rs` is a bonus.

The shape after the pick, with `karst` as the worked example:

| Today | After |
|---|---|
| `IgorKonovalov/voxel-engine` | `IgorKonovalov/karst`, the old URL redirecting |
| `crates/vxe-world`, `vxe_world::` | `crates/karst-world`, `karst_world::` |
| `VXE_COMMIT`, `VXE_*` | `KARST_COMMIT`, `KARST_*`, a clean cut, no aliases |
| `package vxe:logic@0.1.0` | `package karst:logic@0.1.0`, bumped with the components |
| `vxe-tester-0.59.0-linux-x86_64.tar.gz` | `karst-tester-0.60.0-linux-x86_64.tar.gz` from the next tag |
| `apps/flythrough`, `flythrough` | unchanged: the testbed keeps its own name |
| window title `vxe` | `karst` |

**What the rename plan covers**, in phases a `dev` lane can run in a day, each a commit:

1. **The workspace**: directory moves (`git mv`), `Cargo.toml` package names and path
   dependencies, `use` paths, the WIT package and the guest components, the environment
   variables and the gates that count them, the window title; `cargo build`, the gate tier and
   `scripts/build-guests.sh` green. This is the phase that touches 250 Rust files; it is one
   substitution and a rebuild, not a redesign.
2. **The repository**: the GitHub rename (an owner step), the `repository` field, the badge and
   every `github.com/IgorKonovalov/voxel-engine` URL, the consumer's remotes and path
   dependencies in `~/Work/voxel-roguelite`, the local worktrees' remotes.
3. **The release**: archive and artifact names in `release.yml` and `package-tester.sh`,
   `dist/README-TESTER.txt`, the release title; old releases keep their `vxe-tester-*` files
   and are not re-published.
4. **The documents**: `README.md`, `CLAUDE.md`, `docs/roadmap.md`, `docs/nfr.md`,
   `docs/flythrough.md`, the active plans and this index. **Accepted ADRs and closed plans are
   not rewritten**: they are records, and `vxe-` in them is the prefix the engine had when they
   were written. One sentence at the top of `docs/adrs/README.md` and `docs/plans/README.md`
   says so.
5. **Harness**: `.claude/` skills, hooks and settings, in their own repository.
6. **The version**: a minor bump at the plan's close, since a crate rename is a breaking change
   in `0.x`; the CHANGELOG entry names the old and new prefixes once.

**crates.io.** We publish nothing to reserve the name: a placeholder is against the policy
quoted above. If the owner wants the prefix held, the way to hold it is to publish the real
crates at their `0.x` version, which brings docs.rs builds, semver expectations and a
`cargo publish` step per crate into every close; the recommendation is to defer that until a
second consumer asks, and accept the risk that someone registers a `<name>-world` meanwhile
(low: the `-world` and `-render` forms of every one of ninety checked words were free).

**A docs site is not this decision**, but its timing follows from it: the rename goes first, so
no URL is published under the old name. Rustdoc to GitHub Pages fits right after the
`cargo doc -p vxe-render` warnings chore (`docs/backlog.md`), and a guide site after M10 or a
second consumer; both are backlog entries with their own ADR (Starlight vs mdBook) when the
owner asks.

## Consequences

### Positive

- A name a reader can search for, say aloud and type as a prefix; `karst` returns caves and
  limestone, not a Vue table.
- One word everywhere: repo, crates, env vars, archive. No "the project is X but the crates are
  `vxe-`" sentence in the README.
- Done before the first publish, before a docs site and before a second consumer, which is the
  only time the rename is a day's work and nobody else's.

### Negative

- A day of `dev` work plus the owner's GitHub rename, and a noisy commit that touches 450 files;
  `git blame` on moved files needs `--follow`.
- Every tester who scripted `VXE_*` variables or bookmarked the old archive name re-learns the
  prefix once. The release notes carry one line for it.
- Historical records keep the old prefix forever (by design), so a reader of ADR-0002 meets
  `vxe-world` and needs the index's sentence to map it.
- The bare `karst` crate is not ours (see the shortlist); if its holder ships a volume renderer
  under it, two volumetric projects share a word on crates.io. `doline` and `saxum` avoid this
  at the price of a less known word.
- The name is picked from words that are free today; a check is a snapshot, and the ADR's
  acceptance re-runs the crates.io sweep for the picked word on the day of the rename.

## Alternatives considered

### Alternative A: keep `voxel-engine` and `vxe-`

Zero cost, the crate names are free, and the testers know `vxe-tester`. It lost because the
name is a category, not a project: 1394 repositories share it, the prefix is owned in every
search index by a Vue table library, and both only get more expensive to leave. The option
stays open for the owner, and this ADR records why it is not recommended.

### Alternative B: a project name with the `vxe-` crates kept

Half the work (no code moves, no env-var cut) and a name for the README and the repository. It
lost because it defers the expensive half to the worst moment: the crates get published or
imported by a second consumer under `vxe-`, and then the two names are permanent. `wgpu`/`gfx`,
`bevy`/`bevy_*` and `fyrox`/`fyrox-*` all use one word; nobody chooses two.

### Alternative C: a descriptive compound

`voxelmarch`, `brickmarch`, `raymarch-*`: honest about the renderer, free on crates.io. Lost
because it names one crate's mechanism (the march) and not the storage, the edits, the
determinism or the sound; it is long as a prefix (`voxelmarch-script-wasm`); and the march may
not be the renderer forever (the field surface is already an opt-in look, ADR-0041).

### Alternative D: the other shortlisted words

Each is a real alternative with its own evidence in the table below; none lost on a rule, only
on the ordering of criteria 1 and 4. The owner may pick any of them, and this ADR is accepted
with that word.

## Appendix A: the shortlist, with evidence

All checks on 2026-11-17. crates.io through `https://crates.io/api/v1/crates/<name>` (404 is
free); the nine prefix forms `<name>-world` … `<name>-entity` were swept for the first four names
and the `-world` and `-render` forms for all. GitHub through `search/repositories` with
`<name> in:name`, sorted by stars. Domains through `https://rdap.org/domain/<name>.<tld>` (404
is unregistered; a 429 means the check was rate-limited and is reported as unverified).
Meanings from Wiktionary and Wikipedia. "Collisions" lists what a search for the word plus
"engine", "software", "game" or "company" returned; absence of a hit is weak evidence.

| Name | Meaning, sound | crates.io | GitHub | Collisions | Domains | Verdict |
|---|---|---|---|---|---|---|
| **karst** | Landscape of caves, sinkholes and underground rivers dissolved out of limestone; the word is German from Slovene *kras*, written `Karst` in German, `karst` in French and Spanish, карст in Russian; one syllable, no ambiguity | bare **taken**: `0.0.0-reserved`, "A library for high-performance rendering of massive volumetric images", published 2024-09-15, no repository, 1224 downloads; owner LDeakin, whose GitHub carries a Vulkan volume renderer (`VkVolume`, 35 stars). **All nine prefix forms free.** | 703 repos; top `karstnet/karstnet` (22, Python karst networks), `aparis69/Karst-Synthesis` (20, a CGF paper's code). No engine | Two small games: *Karst* (3D first-person cave puzzle, itch.io and Steam, indie) and *KARST: Speed the Void* (web speedrun, beta 2026); Karst Games LLC (mobile puzzles, Missouri). No software product | `.dev` and `.org` registered, `.io` and `.rs` free | **Recommended.** Best fit to criterion 1, perfect on 2 and 5; the bare crate is a two-year placeholder in an adjacent field (scientific volume imaging), contestable under the squatting policy or simply left alone |
| **doline** | A hollow in karst where the roof gave way: a sinkhole; from Slavic *dolina* "valley", German `Doline`, French `doline`, Spanish `dolina`; English "DOH-leen" or "doh-LEEN" | bare **free**; all nine prefix forms free | 73 repos, none above 3 stars (`rok/dolines`, a map of Slovenian dolines) | Nothing found for software, engine, game or company | `.dev`, `.io`, `.rs` free; `.org` registered | **Runner-up.** Karst's family with no collision at all; the collapse ops make it apt; less known than karst, and the pronunciation forks |
| **saxum** | Latin "a rock, a large rough stone" (Lewis & Short); "SAK-sum"; no modern-language meaning | bare **free**; all nine prefix forms free | 44 repos, none above 1 star (a student marble-puzzle game, a Mexican landing page); GitHub organisation `Saxum` exists, empty | A 2014/15 student game at RWTH Aachen; trademarks held by a vineyard, a PR firm, a real-estate partnership and an engineering contractor, all outside software | `.dev` and `.org` registered, `.io` and `.rs` free | Clean coinage-like option; says only "stone", nothing of hollows, light or sound; reads close to "Saxon" |
| **grotto** | A small cave, natural or made; Italian *grotta*, from Greek *kryptē*; every European language has a cognate; "GROT-oh" | bare **free**; all nine prefix forms free | 487 repos; top a 2D platformer asset pack (33), `GrottoCenter` (a caving database, 19) | *Grotto* (a narrative game published by Digerati), Grotto (an Avalanche indie-game platform with a runtime SDK, 2026), Secret Grotto Game Engine (SourceForge, SFML), `jacksery/grotto` (a hobby OpenGL engine) | `.dev` and `.org` registered, `.io` and `.rs` free | The friendliest word on the list; loses on criterion 4 (two hobby engines and a game platform) and reads soft for a stone engine |
| **grike** | A deep fissure between the blocks of a limestone pavement (British English, /ɡɹaɪk/); appears in *Ulysses* | bare **free**; `-world` and `-render` free | 6 repos, none above 0 stars | Nothing found | `.dev`, `.io`, `.org`, `.rs` all free | The only word with every namespace free; obscure outside Britain and reads close to "gripe" |
| **kaiku** | Finnish "echo" ("KY-koo"); also a Marimekko print | bare **free**; `-world` and `-render` free | 126 repos; top `oamaok/kaiku` (11, a JSX UI framework), two music players | Kaiku Health (Helsinki digital-oncology software, part of Elekta since 2020), Kaiku Acoustics (hand-built headphones, Kuopio), a vocal ensemble | `.dev` and `.org` registered, `.io` and `.rs` free | The sound angle; loses on criterion 4 twice (a software company and an audio brand) and says nothing of rock |
| **granum** | Latin "grain"; also a chloroplast structure in biology; "GRAH-num" | bare **free**; `-world` and `-render` free | 66 repos, none above 3 stars | An Android developer "Granum" (training and CRM apps); nothing in graphics | `.dev` and `.org` registered, `.io` and `.rs` free | Voxels as grains is apt, but "granum" says biology to anyone who met it in school |
| **mattock** | A digging tool with an adze blade; "MAT-uk" | bare **free**; `-world` and `-render` free | 65 repos; a GitHub user `mattock` with 50 repos; `MattockFS` (forensics) | An archived Chia plotting manager (`mccxiv/mattock`) | `.dev` and `.org` registered, `.io` and `.rs` free | The carve angle in one tool; seven letters, and it names the tool, not the rock |

Sources: crates.io API responses (`/api/v1/crates/karst`, `/karst/owners`, `/adit`, `/hewn`,
…) and the GitHub REST search, both run 2026-11-17 and summarised above;
https://en.wikipedia.org/wiki/Karst; https://en.wiktionary.org/wiki/doline;
https://en.wiktionary.org/wiki/saxum; https://en.wiktionary.org/wiki/grike;
https://xerithh.itch.io/karst; https://rawg.io/games/karst; https://jeffersoncitymag.com/building-your-own-adventure/
(Karst Games); https://nordiclifescience.org/qa-kaiku-health-from-startup-to-acquisition/;
https://village.ai/company/kaiku-acoustics; https://www.mobygames.com/game/174314/grotto/;
https://egamers.io/avalanche-indie-game-dev-platform-grotto-releases-runtime-sdk/;
https://sourceforge.net/projects/sgge/; https://github.com/mccxiv/mattock;
https://trademark.justia.com/owners/saxum-inc-2608391.

Not verified: trademark registers outside the US aggregator hits; whether any shortlisted word
is a brand in Chinese, Japanese or Arabic markets; `.com` availability (not checked, assumed
taken for every real word).

## Appendix B: the candidate pool

Ninety words were checked against crates.io (bare, `-world`, `-render`) and GitHub on
2026-11-17; a web search followed for the ones that survived. A word was cut for one reason,
given here. "taken" quotes the crate's description.

**Voxel, volume, cell, grain.** `vxe` (free on crates.io; `vxe-table`, 8635 stars, owns the
word on GitHub and npm), `voxa` (free; Voxa Discord client 189 stars, VoxaAI framework),
`voxl` (taken: "a command-line tool for working with voxels"; ModalAI VOXL drones; `tembo/voxl`
1739 stars), `voxum` (free; sounds like "vacuum"), `vexel` (taken: linear algebra; also a
pixel-art term), `voxen` (free; VoxEngine TTS), `voksel` (free; `pannapudi/vokselis`, a
wgpu volume renderer, and three voxel tools of that name), `grain` (taken; the Grain
language, 3468 stars), `granum` (shortlisted), `grit` (taken; GritQL 4626 stars), `tessera`
(taken; Tesseract OCR owns every search), `tessel` (taken; the Tessel board), `kybos` (Greek
"cube, die"; free; pronunciation forks KEE/KY), `brikk` (free; GitHub organisation `brikk`, a
data company), `cubit` (taken; Flutter's Cubit), `lattice` (taken; Lattice HR and
semiconductors), `cella` (taken: a reserved kv crate), `cellar` (taken: a password tool).

**Light and marching.** `glim` (taken: "simple ray tracing library"), `glint` (taken; also
Microsoft Glint), `wick` (taken; John Wick owns the search), `candela` (taken), `lumen`
(taken; Lumen PHP), `ember` (taken; Ember.js), `umbra` (taken; Umbra occlusion culling, now
Unity), `tallow` (free; a miner's candle fat, unglamorous), `lucerna` (Latin "lamp"; free;
`Federicoand98/Lucerna` is a GPU path tracer), `lambent` (taken: "a ray / path tracer built on
wgpu"), `phos` (taken). Cut before the registry: `lux`, `glow`, `gleam` (a language),
`glimmer` (Ember's VM), `lucent`, `penumbra` (Frictional Games), `lantern`, `stride` (a C#
engine).

**Stone, earth, mining, carving.** `adit` (a mine's horizontal entrance; **taken 2026-11-14**:
"placeholder for an in-development IPC library", a textbook squat; also Adit dental software
and `AditGame/Adit`, a Dwarf-Fortress-like voxel game), `stope` (taken: an S3 store; a mining
software term, Maptek Vulcan), `lode` (taken), `seam` (taken), `quarry` (taken), `mattock`
(shortlisted), `auger` (taken), `hewn` (**taken: "a simple game engine built for educational
purposes"**), `knap` (taken; silent k), `spall` (taken; the Spall profiler, 487 stars),
`clast` (taken), `karst` (shortlisted), `doline` (shortlisted), `cenote` (Maya sinkhole;
free; Cenote Labs, Atlassian apps; `Cenote-Taker` virome tool), `grike` (shortlisted), `aven`
(taken; Aven fintech), `grotto` (shortlisted), `cairn` (taken; `oritera/Cairn` 5271 stars),
`scree` (taken; every "screen" repo), `marl` (taken; Marlin firmware, Google `marl`), `shale`
(taken; Shale Ruby), `flint` (taken; Microsoft `flint-chart` 4581 stars), `chert` (taken; and
чёрт in Russian), `basalt` (taken: "a window/ui framework built upon vulkan"; vkBasalt), `silex`
(taken; Silex PHP), `saxum` (shortlisted), `lithic` (taken; Lithic card-issuing API), `lithos`
(taken), `petra` (taken; Petra wallet), `lapis` (taken; Lapis Lua), `breccia` (free;
pronunciation BRETCH-ia), `flowstone` (free; nine letters), `sinter` (taken), `crag` (taken;
CRAG retrieval papers), `gulch` (free; Galt's Gulch), `scarp` (taken), `cleft` (taken; medical
sense), `carbide` (taken; a Rust GUI library), `kivi` (Finnish "stone"; taken; sounds like
Kivy), `kallio` (Finnish "bedrock"; free; Kalliope assistant), `steen` (Dutch "stone"; free;
a surname), `luola` (Finnish "cave"; free; the Luola cave-flying game), `louhos` (Finnish
"quarry"; free; GitHub organisation `louhos`). Cut before the registry: `geode` (Geometry Dash
mod loader, Apache Geode), `menhir` (OCaml parser generator), `dolmen` (a 2022 game),
`obsidian`, `slate`, `granite`, `quartz`, `chisel` (Scala HDL), `spade` (a Rust HDL),
`delf`/`delve` (the consumer's genre word), `stollen` (German for adit, and a cake), `kobold`
(a monster).

**Determinism, clockwork.** `pawl` (taken), `ratchet` (taken; Ratchet PHP), `lockstep` (taken;
LockstepFramework 1541 stars), `tick` (taken; 81k downloads). Cut before the registry:
`clockwork`, `cog`, `verge`, `ledger`, `loom`, `tally`: every one a product.

**Sound, echo.** `kaiku` (shortlisted), `thrum` (taken: a terminal email client; a haptic
meditation app), `knell` (taken), `peal` (taken), `echo` (taken; Echo Go framework 32807
stars), `resonance` (free; Resonance Audio, Google's spatial-audio SDK, 544 stars: a direct
collision for an engine with HRTF audio), `timbre` (taken: an audio-effects crate). Cut before
the registry: `reverb`, `sonar`, `rumble`, `hum`, `knocker`.

**Myth and other languages.** `ymir` (Norse giant whose bones became the mountains; taken;
two game engines named Ymir, a Sega Saturn emulator at 1126 stars), `erebus` (taken; red-team
tooling), `chthon` (free; hard to spell), `bucca` (Cornish mine spirit; free; "buccaneer"
noise), `tellus` (taken), `brokkr` (Norse smith; taken: a task queue), `svarog` (Slavic
smith-god; taken: an SVG library; two Svarog games and a Svarog Studios). Cut before the
registry: `vulcan` (Vulkan), `wayland` (the display protocol), `gaia`, `nyx`, `sampo` (a
Finnish financial group), `durin`, `mimir`.

## Outcome

_(Added at acceptance: the picked word, the day's re-run of the crates.io sweep for it, and the
rename plan's number.)_
