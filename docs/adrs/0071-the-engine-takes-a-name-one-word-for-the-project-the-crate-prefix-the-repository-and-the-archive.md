# ADR-0071: The engine takes a name: one word that is the project, the crate prefix, the repository and the archive

> **Status:** accepted (owner, 2026-11-17: "for renaming, I think svet is the best")
> **Date:** 2026-11-17
> **Related plan(s):** [Plan 0087](../plans/0087-the-rename-to-svet-crates-env-vars-wit-package-archives-docs-and-the-repository.md),
> the rename (draft, awaits the owner's go)

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

**The engine is `svet`**: Slavic *svet* / свет, "light", and in the same languages "the world"
(Russian *весь свет*, Czech *svět*, Slovene and Serbo-Croatian *svet*); one syllable, four
letters, said the same in every language the criteria name. The owner picked it on 2026-11-17
from the three shortlists below, in his words: "for renaming, I think svet is the best". The
one word is the project name, the crate prefix (`svet-world`, `svet-render`, `svet-app`,
`svet-sim`, `svet-assets`, `svet-net`, `svet-script-wasm`, `svet-audio`, `svet-entity`), the
repository (`IgorKonovalov/svet`), the environment-variable prefix (`SVET_*`), the WIT package
namespace (`svet:logic`) and the tester archive's name (`svet-tester-<version>-<os>-x86_64`),
changed together in Plan 0087. Until that plan merges, the tree keeps `voxel-engine` and
`vxe-`.

What `svet` costs, stated plainly. It is one letter from `svelte`, and a web developer's eye
will read it so; it is a near-homophone of Swedish and Norwegian *svett*, "sweat"; `svet.dev`,
`svet.io`, `svet.org` and `svet.rs` were all registered on the day of the check, so **no domain
claim is made** and none is planned; the word belongs to Slavic readers, who will hear "light"
or "world" where others hear a coinage. On crates.io the bare `svet` and all nine prefix
forms were free as of the check (2026-11-17); a check is a snapshot, and Plan 0087's first
phase re-runs it on the rename day before a file moves.

Three sweeps fed the shortlists. The first (Appendix A) worked from rock, hollows and carving;
the second (Appendix C), asked for by the owner, from light, ray marching, entities, cave
animals, spiders and rock types; the third (Appendix D), also asked for, from famous caves,
mountains and karst features, with eight names the owner marked and `antrum`. The
architect's recommendation was `karst`, with `svet` the runner-up; the owner weighed
`svet`'s reach (the only word checked that says both "light" and "world") and its clean
registry above `karst`'s better-known meaning and its held bare crate. The other clean
alternatives were `naica` (the crystal cave; one small game shares it), `antrum` (Latin
"cave"; a film and a sinus share it), `doline` and `zarevo` (nobody has touched either).

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

The shape after the rename:

| Today | After |
|---|---|
| `IgorKonovalov/voxel-engine` | `IgorKonovalov/svet`, the old URL redirecting |
| `crates/vxe-world`, `vxe_world::` | `crates/svet-world`, `svet_world::` |
| `VXE_COMMIT`, `VXE_*` | `SVET_COMMIT`, `SVET_*`, a clean cut, no aliases |
| `package vxe:logic@0.1.0` | `package svet:logic@0.1.0`; the components are rebuilt |
| `vxe-tester-0.59.0-linux-x86_64.tar.gz` | `svet-tester-<next minor>-linux-x86_64.tar.gz` from the next tag |
| `apps/flythrough`, `flythrough` | unchanged: the testbed keeps its own name |
| window title `vxe` | `svet` |

**What the rename plan covers** is Plan 0087's business: the workspace, the environment
variables and the WIT package, the tester and its archive, the documents and the harness, the
owner's GitHub rename, a minor version bump, and a final grep. The four smaller choices below
are **the architect's recommendations, taken into the plan as written; the owner overturns any
of them by saying so**, and the plan's phase changes with it:

1. **crates.io: nothing is published until a second consumer asks** (recommended). A
   placeholder is against the policy quoted above. If the owner wants the prefix held, the way
   to hold it is to publish the real crates at their `0.x` version, which brings docs.rs builds,
   semver expectations and a `cargo publish` step per crate into every close; deferring accepts
   the risk that someone registers `svet-world` meanwhile (low: the `-world` and `-render`
   forms of every one of the words checked across three sweeps were free). **Plan 0087 re-runs
   the crates.io check for `svet` and its nine forms on the rename day**, before the first
   `git mv`; a form taken by then is reported to the owner before the lane goes on.
2. **The environment variables are cut clean: `VXE_*` becomes `SVET_*`, no aliases**
   (recommended). An alias layer would read both names in every consumer of the variable
   (the fly-through, the tester, `check-test-tiers.mjs`, CI) for a handful of testers'
   scripts; the release notes carry one line for the change instead. The 84 distinct names
   on `main` at 778fac63 are substituted, and the gate that counts `VXE_*` names per test
   file counts `SVET_*`.
3. **Accepted ADRs, closed plans, their readings logs, the dated reviews and the CHANGELOG's
   entries are not rewritten** (recommended). They are records, and `vxe-` in them is the
   prefix the engine had when they were written; rewriting them would make `git blame` on a
   decision lie about its date. One sentence at the top of `docs/adrs/README.md` and of
   `docs/plans/README.md` maps the old prefix to the new. Proposed ADRs and active plans are
   not records yet and are rewritten with the rest of the documents (a proposed ADR whose
   file name carries `vxe` is moved, and its links repaired).
4. **Old releases stay as they are, and no repository is kept under the old name**
   (recommended). The `vxe-tester-*` archives on the tagged releases are not re-published;
   GitHub's redirect carries the old clone URL, and it lives as long as no new repository
   takes `IgorKonovalov/voxel-engine`, so none is made.

**A docs site is not this decision**, but its timing follows from it: the rename goes first, so
no URL is published under the old name. Rustdoc to GitHub Pages fits right after the
`cargo doc -p vxe-render` warnings chore (`docs/backlog.md`), and a guide site after M10 or a
second consumer; both are backlog entries with their own ADR (Starlight vs mdBook) when the
owner asks.

## Consequences

### Positive

- A name a reader can search for, say aloud and type as a prefix; `svet` is four letters and
  one syllable, and a search for `svet-world` or `svet-render` returns nothing of anyone
  else's, not a Vue table.
- The word says what the engine is after: light that holds still, in a world. No other word
  checked carried both.
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
- `svet` is one letter from `svelte` in the eyes of a web developer, and a near-homophone of
  Swedish and Norwegian "svett" (sweat); neither is the word, both will be heard. Outside the
  Slavic languages the word means nothing until the README says so.
- Every `svet.*` domain checked is registered, so the project has no domain and the
  repository URL is its address.
- The name is picked from words that were free on 2026-11-17; a check is a snapshot, and Plan
  0087 re-runs the crates.io check for `svet` on the rename day.

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

Each is a real alternative with its own evidence in the tables below (Appendices A, C and D);
none lost on a rule, only on the ordering of criteria 1 and 4. `karst`, the architect's
recommendation, lost to the owner's weighing: its bare crate is held by a volume-imaging
placeholder, and `svet` says more of the engine (light, and the world) in fewer letters. The
rest are recorded so that a later reader knows they were seen.

## Appendix A: the shortlist, with evidence

All checks on 2026-11-17. crates.io through `https://crates.io/api/v1/crates/<name>` (404 is
free); the nine prefix forms `<name>-world` … `<name>-entity` were swept for the first four names
and the `-world` and `-render` forms for all. GitHub through `search/repositories` with
`<name> in:name`, sorted by stars. Domains: `.dev` and `.org` through
`https://rdap.org/domain/<name>.<tld>` following its redirect to the registry (200 registered,
404 free), `.io` through `https://rdap.identitydigital.services/rdap/domain/<name>.io`, `.rs`
through `whois -h whois.rnids.rs`; the first pass read rdap.org's bare 404 as "free" for `.io`
and `.rs`, where it means "no RDAP server known" (it answers 404 for `google.io`), so every
domain cell in this document was re-run with the corrected method on 2026-11-17.
Meanings from Wiktionary and Wikipedia. "Collisions" lists what a search for the word plus
"engine", "software", "game" or "company" returned; absence of a hit is weak evidence.

| Name | Meaning, sound | crates.io | GitHub | Collisions | Domains | Verdict |
|---|---|---|---|---|---|---|
| **karst** | Landscape of caves, sinkholes and underground rivers dissolved out of limestone; the word is German from Slovene *kras*, written `Karst` in German, `karst` in French and Spanish, карст in Russian; one syllable, no ambiguity | bare **taken**: `0.0.0-reserved`, "A library for high-performance rendering of massive volumetric images", published 2024-09-15, no repository, 1224 downloads; owner LDeakin, whose GitHub carries a Vulkan volume renderer (`VkVolume`, 35 stars). **All nine prefix forms free.** | 703 repos; top `karstnet/karstnet` (22, Python karst networks), `aparis69/Karst-Synthesis` (20, a CGF paper's code). No engine | Two small games: *Karst* (3D first-person cave puzzle, itch.io and Steam, indie) and *KARST: Speed the Void* (web speedrun, beta 2026); Karst Games LLC (mobile puzzles, Missouri). No software product | `.dev`, `.io`, `.org` registered; `.rs` free | **Recommended.** Best fit to criterion 1, perfect on 2 and 5; the bare crate is a two-year placeholder in an adjacent field (scientific volume imaging), contestable under the squatting policy or simply left alone |
| **doline** | A hollow in karst where the roof gave way: a sinkhole; from Slavic *dolina* "valley", German `Doline`, French `doline`, Spanish `dolina`; English "DOH-leen" or "doh-LEEN" | bare **free**; all nine prefix forms free | 73 repos, none above 3 stars (`rok/dolines`, a map of Slovenian dolines) | Nothing found for software, engine, game or company | `.dev`, `.io`, `.rs` free; `.org` registered | **Third overall** (runner-up of this sweep). Karst's family with no collision at all; the collapse ops make it apt; less known than karst, and the pronunciation forks |
| **saxum** | Latin "a rock, a large rough stone" (Lewis & Short); "SAK-sum"; no modern-language meaning | bare **free**; all nine prefix forms free | 44 repos, none above 1 star (a student marble-puzzle game, a Mexican landing page); GitHub organisation `Saxum` exists, empty | A 2014/15 student game at RWTH Aachen; trademarks held by a vineyard, a PR firm, a real-estate partnership and an engineering contractor, all outside software | `.dev`, `.io`, `.org` registered; `.rs` free | Clean coinage-like option; says only "stone", nothing of hollows, light or sound; reads close to "Saxon" |
| **grotto** | A small cave, natural or made; Italian *grotta*, from Greek *kryptē*; every European language has a cognate; "GROT-oh" | bare **free**; all nine prefix forms free | 487 repos; top a 2D platformer asset pack (33), `GrottoCenter` (a caving database, 19) | *Grotto* (a narrative game published by Digerati), Grotto (an Avalanche indie-game platform with a runtime SDK, 2026), Secret Grotto Game Engine (SourceForge, SFML), `jacksery/grotto` (a hobby OpenGL engine) | `.dev`, `.io`, `.org` registered; `.rs` free | The friendliest word on the list; loses on criterion 4 (two hobby engines and a game platform) and reads soft for a stone engine |
| **grike** | A deep fissure between the blocks of a limestone pavement (British English, /ɡɹaɪk/); appears in *Ulysses* | bare **free**; `-world` and `-render` free | 6 repos, none above 0 stars | Nothing found | `.dev`, `.org`, `.rs` free; `.io` registered | Free everywhere but `.io`; obscure outside Britain and reads close to "gripe" |
| **kaiku** | Finnish "echo" ("KY-koo"); also a Marimekko print | bare **free**; `-world` and `-render` free | 126 repos; top `oamaok/kaiku` (11, a JSX UI framework), two music players | Kaiku Health (Helsinki digital-oncology software, part of Elekta since 2020), Kaiku Acoustics (hand-built headphones, Kuopio), a vocal ensemble | `.dev`, `.io`, `.org` registered; `.rs` free | The sound angle; loses on criterion 4 twice (a software company and an audio brand) and says nothing of rock |
| **granum** | Latin "grain"; also a chloroplast structure in biology; "GRAH-num" | bare **free**; `-world` and `-render` free | 66 repos, none above 3 stars | An Android developer "Granum" (training and CRM apps); nothing in graphics | `.dev`, `.io`, `.org`, `.rs` all registered | Voxels as grains is apt, but "granum" says biology to anyone who met it in school |
| **mattock** | A digging tool with an adze blade; "MAT-uk" | bare **free**; `-world` and `-render` free | 65 repos; a GitHub user `mattock` with 50 repos; `MattockFS` (forensics) | An archived Chia plotting manager (`mccxiv/mattock`) | `.dev`, `.io`, `.org` registered; `.rs` free | The carve angle in one tool; seven letters, and it names the tool, not the rock |

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

## Appendix C: the second sweep: light, rays, entities, animals, spiders, rocks

Asked for by the owner on 2026-11-17 after the first shortlist: "something related to light,
ray tracing, entities", then "animals, spiders, types of rocks". The same bar as Appendix A,
run the same day: 199 more words through crates.io (bare, `-world`, `-render`) and the GitHub
search, the survivors through the nine prefix forms, `rdap.org` and a web search. One finding
shapes this whole sweep: **the light and ray words are the most crowded namespace on crates.io
and GitHub.** `lambent`, `glim`, `lueur` ("shadowy game and graphics library"), `luma` ("lazy
GPU arrays on wgpu"), `lumo` ("CPU based rendering engine"), `valo` ("a 2D render engine on
wgpu", published 2026-11-17), `albedo`, `radiance`, `irradiance`, `photon`, `lumen`, `umbra`,
`penumbra`, `glint`, `wick`, `candela`, `ember`, `phos`, `beam`, `flare`, `halo`, `nimbus`,
`corona`, `candle`, `taper`, `dusk`, `dawn` (a reserved placeholder since 2019) are all
registered; so are `golem` ("a library for graphics programming"), `karakuri` ("an automata
framework"), `homunculus` ("3D modeling library"), `anima`, `animus`, `automaton`, `puppet`,
`marionette`, `eidolon`, `tulpa` (a 2026-09 placeholder) and `theia`; and of the rocks and
minerals, 37 of the 49 checked, among them `feldspar` (a 2015 placeholder, "Nothing Yet"; on GitHub
`bonsairobo/feldspar` is "procedural voxel worlds for Bevy"), `hematite` (the Piston Minecraft clone), `basalt` (a
Vulkan UI), `granite` (also Themaister's Vulkan renderer on GitHub), `pyrite` (also an N64
engine), `galena`, `gneiss`, `schist`, `obsidian`, `slate`, `quartz`, `onyx`, `marble`,
`agate`, `geode`, `tuff`, `gabbro`, `scoria`, `kyanite`, `rutile`, `skarn`, `tephra`,
`moraine`, `syenite` (a placeholder) and `spinel`. Among spiders and cave animals, every web
or crawling word belongs to a crawler (`anansi` is a Rust web framework, `ragno`, `kumo` and
`recluse` are Rust crawlers, `pholcus` a Go one; `gossamer`, `weft`, `skein`, `clew`, `weaver`,
`spinneret`, `nema`, `filum` are taken), `arachne` is a Nix framework crate, `portia` was
published on 2026-11-08 and `luciola` on 2026-11-16, `talpa`, `tyto`, `strix`, `noctua`, `firefly`,
`glowworm`, `owl`, `newt`, `mole`, `moth`, `badger`, `wombat` and `pangolin` are all gone, and
`olm` is unsearchable next to AllenAI's OLMo (6737 stars) and olmocr (19829).

The finalists. Every one has the bare crate and all nine prefix forms free on crates.io
(swept 2026-11-17); the "Domains" column is `.dev/.io/.org/.rs` by Appendix A's corrected method, and the
GitHub user of the same name is noted when it exists.

| Name | Angle; meaning, sound | GitHub | Collisions | Domains | Verdict |
|---|---|---|---|---|---|
| **svet** | Light. Slavic *svet* / свет: "light", and in the same languages "the world" (Russian *весь свет*, Czech *svět*, Slovene and Serbo-Croatian *svet*); one syllable, four letters, said the same everywhere | 6216 repos match, almost all Svelte-adjacent (`svetch`); none is a product; user `svet` (12 repos) | None found for software, engine or game. One letter from `svelte`; near-homophone of Swedish and Norwegian *svett*, "sweat" | `.dev`, `.io`, `.org`, `.rs` all registered | **Recommended in this sweep, runner-up overall.** The only word checked that says both "light" and "world"; clean registry; the price is the svelte/svett echo and a word Slavic readers own |
| **zarevo** | Light. Russian зарево: the glow of a distant fire or of dawn on the sky; "za-RE-vo", three syllables, no diacritics | 11 repos, none above 1 star; GitHub organisation `zarevo` exists, empty | None found | `.dev`, `.io`, `.rs` free; `.org` registered | **Runner-up in this sweep.** The look the engine is after (a steady glow in the dark) in one word, every namespace empty; meaning opaque outside Slavic languages, and the longest word on either list |
| **aktis** | Ray. Greek ἀκτίς, "ray, beam" (modern *aktina*); "AK-tis" | 49 repos; `cfr/aktis` (a toy path tracer, 3 stars), `timlittle/aktis` (a raycasting game engine, 1 star) | Czech software firm Actis (actis.cz) by sound only; no product | `.dev`, `.io`, `.org` registered; `.rs` free | The ray-tracing word that is still free; says the renderer and nothing else; two one-person hobby tracers already picked it, which shows the fit and the risk |
| **candor** | Light. Latin *candor*, "dazzling whiteness, radiance", whence English "candour/candor": frankness. Light and honest cubes in one word; "KAN-dor" | 625 repos; `indutny/candor` (an experimental language VM, 176 stars) | Candor Technology (mortgage AI), Candor (YC W25), CandorIQ: business software, not graphics. The US/UK spelling fork | `.dev`, `.io`, `.org` registered; `.rs` free | The strongest double meaning after `svet`; loses on being a common English word (unsearchable) owned by several companies |
| **raylith** | Ray + stone. Coinage from *ray* and Greek *lithos*; "RAY-lith" | 3 repos, a student's; user `raylith` exists (3 repos) | None found. One letter-pair from `raylib`, the C game library, which a reader will see | `.dev`, `.io`, `.org`, `.rs` all free | Says exactly what the renderer does (rays through rock), every namespace free, no meaning to misread; a coinage, and the `raylib` shadow is real |
| **haniwa** | Entities. 埴輪, the hollow clay figures that stood guard on Japanese tombs: cylinders, simple faces, holes for eyes; "HA-ni-wa" | 79 repos, none above 1 star | A six-year-old itch.io game *Haniwa Face*; Animal Crossing's gyroids are called haniwa in the Japanese versions, so Japanese players know the word | `.dev`, `.io`, `.rs` free; `.org` registered | The entity angle's best word: clay figures that stand in the world is what `vxe-entity` ships; says nothing of light or rock, and names one crate's concern |
| **myotis** | Animals. The mouse-eared bats, the largest bat genus: cave roosts, night, echolocation (the engine probes sound through the same voxels); "my-OH-tis" | 25 repos; `biafra23/myotis` (a wallet engine, 37 stars) | None found | `.io`, `.rs` free; `.dev`, `.org` registered | Cave, dark and echo in one animal; an English ear hears *myositis*, and a Latin genus name reads as biology |
| **iktomi** | Spiders. The Lakota spider trickster, a shape-shifter and culture hero; "ik-TOH-mi" | 20 repos; `SmartTeleMax/iktomi` (a Python web library, 24 stars) | None found | `.dev`, `.rs` free; `.io`, `.org` registered | The spider angle's free myth name (`anansi` and `arachne` are taken); a sacred figure of a living people, which the owner should weigh before using it as a brand |
| **pumice** | Rocks. The light, porous volcanic rock that floats; "PUM-iss" | 79 repos; `zksecurity/pumice` (a prover port, 20 stars) | None found; the everyday association is the foot stone | `.dev`, `.io`, `.org` registered; `.rs` free | The one common rock name still free with a clean picture; the association is bathroom, not cave |

Also free and checked, but not finalists: `lumbre` (Spanish "firelight"; empty), `blask`
(Polish "glare"; a Flask blog engine), `blesk` (Czech "lightning", Russian "gleam"; BleskOS, and
the largest Czech tabloid), `gloaming` (Scots "dusk"; eight letters), `nitor` (Latin "sheen";
Nitor Infotech), `strahl` (German "ray"; `StuckiSimon/strahl` is a WebGPU path tracer),
`selas` (Greek "glow"; `schuttejoe/Selas` is an offline renderer, 109 stars), `solas` (Irish
"light"; a Minecraft shader pack and `Grokmoo/sulis`, a Rust RPG, next door), `luch` (Russian
"ray"; unreadable in Latin letters), `geisli` (Icelandic "ray"; a consulting firm), `kiir`
(Estonian "ray"; odd spelling), `onibi` and `hitodama` (Japanese ghost-lights; a developer and
a game of those names), `foxfire` (reads as Firefox), `lumbra` and `umbrix` (coinages;
`lumbra` reads as *lumbar*), `lumarch` and `marchstone` (coinages; weaker than `raylith`),
`raymarch`, `marcher`, `raycast` and `dda` (free but generic; `MarbleMarcher` and Cataclysm-DDA
own the searches), `xoanon` (Greek, an archaic carved image; six letters, "ZOH-a-non"; empty,
and the carve-plus-entity meaning is good, but nobody can say it), `ushabti` and `shabti`
(Egyptian "answerer" figurines that animate to work; `shabti` has a 196-star security repo),
`tanagra` (Greek figurines; a Vulkan binding generator), `dogu` (Japanese figurines; a test
automation company), `poppet`, `effigy` (Effigy Softworks, a 266-star asset generator),
`domovoi` (Slavic house spirit; an AWS Lambda tool, 178 stars), `nisse` and `tomte` (Christmas
games), `argil` (potter's clay; Argilla noise), `galatea` and `enkidu` (clay made alive; both
noisy), `belenus` and `yarilo` (gods of light; empty but seven and six letters nobody places),
`spalax` and `zokor` (blind burrowing rodents; obscure), `fossor` (Latin "digger"; an archived
LinkedIn tool), `noctule` and `photinus` (a bat and a firefly genus; seven and eight letters),
`aranea`, `araneus`, `argiope`, `hogna`, `maratus`, `dysdera`, `atypus`, `ummidia`,
`salticus`, `mygale`, `lycosa` (spider genera and names; `lycosa` was a Razer keyboard,
`ummidia` the trapdoor spider is the aptest and the least sayable), `huntsman` (a web spider
and a chemicals group), `dragline` (eight letters), `diorite`, `andesite`, `rhyolite`,
`dolomite`, `halite`, `olivine`, `porphyry`, `perlite`, `eclogite`, `augite`, `phonolite`
(rocks; `halite` and `perlite` have 1000-plus-star namesakes, `olivine` a Labs, `phonolite`,
the rock that rings when struck, is the aptest and nine letters long), `troglobite` and
`gloaming` (length).

**How the second sweep compares with `karst` and `doline`.** On criterion 1 `svet` and
`candor` reach further than `karst` (light and world; light and honesty), `zarevo` and
`raylith` about as far, the rest less. On criterion 2 `svet` wins outright, `aktis` and
`karst` tie, `zarevo` and `raylith` trail. On criterion 3 every finalist here beats `karst`,
whose bare crate is held. On criterion 4 `karst`, `svet`, `zarevo`, `raylith` and `haniwa`
are clean; `candor` and `iktomi` are not. On criterion 5 `karst`, `zarevo`, `aktis`, `haniwa`
and `pumice` are clean; `svet` has the svelte/svett echo, `myotis` the *myositis* one. Summed
with the criteria in their stated order, `karst` stays first on the strength of 1, 2 and 5;
`svet` takes the runner-up place from `doline` because it says more and its registry is
cleaner; `zarevo` is the pick for an owner who wants a word nobody else has touched.

Sources for this appendix: the crates.io and GitHub sweeps of 2026-11-17 as above;
https://en.wiktionary.org/wiki/свет, https://en.wiktionary.org/wiki/svet,
https://en.wiktionary.org/wiki/зарево, https://en.wiktionary.org/wiki/ἀκτίς,
https://en.wiktionary.org/wiki/candor#Latin, https://en.wikipedia.org/wiki/Haniwa,
https://en.wikipedia.org/wiki/Myotis, https://en.wikipedia.org/wiki/Iktomi,
https://en.wikipedia.org/wiki/Pumice; https://github.com/cfr/aktis;
https://github.com/timlittle/aktis; https://github.com/indutny/candor;
https://www.ycombinator.com/companies/candor; https://craft.co/candor-technology;
https://lib.rs/crates/wisp-rs; https://docs.rs/crate/lueur/latest;
https://github.com/StuckiSimon/strahl; https://github.com/schuttejoe/Selas;
https://github.com/bonsairobo/feldspar; https://github.com/Themaister/Granite;
https://github.com/HailToDodongo/pyrite64. Not verified: trademark registers for any finalist
(a US aggregator search was run only for `candor`, which has several owners in other
classes); the Japanese market's use of `haniwa` as a brand; Lakota views on `iktomi` as a
product name.

## Appendix D: the third sweep: famous caves, mountains, karst features, mythic places

Asked for by the owner on 2026-11-17 ("names of some famous caves or mountains can be an idea
as well"), with eight names he marked from Wikipedia's cave lists and `antrum` added after.
Same bar, same day: 171 words through crates.io (bare, `-world`, `-render`) and the GitHub
search, 22 survivors through the other seven prefix forms (all free), the owner's names through
all ten, domains with the corrected method of Appendix A, and a web search per finalist.

**What the famous names taught.** Every famous mountain is a brand or a crate: `teide` is a
columnar computation engine on crates.io (TeideDB, 2026-02, all four domains taken), `hekla` an
event-sourcing runtime updated 2026-11-04 (and a 367-star Hacker News app), `katla` a data-flow
generator, `etna` a 2019 placeholder beside a 880-star time-series library and a 2D game engine,
`triglav` a networking tool and an insurer, `orda` an AoE4 client, `mammoth` a web server
beside mammoth.js (6345 stars), `eiger` 350 repos, `kibo` a 3987-star shadcn registry, `zagros`
a Zig server and an OS; the sacred ones (Kailash, Uluru, Aoraki, Tongariro, Meru, Athos) and
the ones that are a death toll (Huascarán, Marmolada, Kitum, Mossdale, Tham Luang) were cut
before the registry. The caves fared better: the registries are free for almost every one,
and the collisions are tourism brands, not software.

| Name | What it is; sound | crates.io | GitHub; collisions | Domains | Meanings, sensitivity | Verdict |
|---|---|---|---|---|---|---|
| **naica** | Cueva de los Cristales in the Naica mine, Chihuahua: 12 m selenite crystals 300 m down, found 2000, flooded since Peñoles stopped the pumps in 2015; "NAI-ka", five letters, two syllables, the same in all five languages | bare and all nine free | 45 repos, none relevant. *Naica Online*, a free-to-play 8-bit MMORPG (FAT SLU, Steam Early Access 2025); NAICA, a French consulting cooperative | `.dev`, `.io`, `.rs` free; `.org` registered | Tarahumara "shady place" (the cave site's claim, unverified); a working mine, nothing sacred, no tragedy | **First from this angle.** The cleanest registries of any famous cave and a renderer's image; one small game; says nothing until the cave is known |
| **antrum** | Latin "cave, cavern, hollow, tomb", from Greek *antron*; English anatomy "a bodily cavity with bony walls" (maxillary antrum, gastric antrum); "AN-trum", six letters | bare and all nine free | 40 repos, hackathon projects of 1–4 stars. Antrum / AntrumX, an indoor-air-quality startup (antrum.com, 2020); *Antrum: The Deadliest Film Ever Made* (2018, a cursed-film horror mockumentary) | `.io`, `.rs` free; `.dev`, `.org` registered | Doctors hear sinus and stomach; Spanish and Italian *antro* is colloquially a seedy dive; the film is the one tie to the first consumer's genre | **Second.** The Latin generic for cave, as `karst` is the generic for the landscape; free where it matters; the anatomy and the film are the drag |
| **fingal** | Fingal's Cave, Staffa: a 70 m sea cave in hexagonal basalt columns, "the cave of melody", Mendelssohn's *Hebrides* overture (1829); "FING-gal" | bare and all nine free | 75 repos (an iOS theming tool, a geophysics package); no product | `.io`, `.rs` free; `.dev`, `.org` registered (Fingal County Council) | An Irish county and the Ossianic hero. **Russian colloquial фингал is a black eye** (Wiktionary) | The best story found (sound through columnar stone); the Russian meaning decides it, criterion 5 |
| **postojna** | Postojnska jama, Slovenia: 24 km show cave since 1818, the olm, the cave railway; run by Postojnska jama d.d. as "Postojna Cave Park"; "po-STOY-na", eight letters, three syllables | bare and all nine free | 3 repos; no software or product | `.dev`, `.io`, `.rs` free; `.org` registered | A town's name; nothing sacred; EN, FR and ES readers say the *j* as J | The most famous cave word that is free everywhere; fails criterion 2 and names one company's attraction |
| **ponor** | Karst: the opening where a stream sinks underground (swallow hole); South Slavic, from Proto-Slavic *nora* "pit, abyss"; "PO-nor", five letters | bare and all nine free | 176 repos, all substring hits on Ponorogo (an Indonesian regency); nothing for software or game | `.dev`, `.io` free; `.rs`, `.org` registered | Serbian/Croatian also "abyss"; Czech *ponor* a ship's draught; Russian понор the karst term; no unfortunate sense | `doline`'s sibling, as apt and as free, more obscure in English; does not beat it |
| **luray** | Luray Caverns, Virginia: the Great Stalacpipe Organ, solenoid mallets on tuned stalactites over 3.5 acres; "loo-RAY", five letters | bare and all nine free | 23 repos (`LuRay-NGN`, a 1-star hobby engine). LURAY CAVERNS, US trademark reg. 0988086 (1974, renewed 2024, class 41), Luray Caverns Corporation | `.dev`, `.io`, `.rs` free; `.org` registered | A Virginia town; contains "ray" | Stone that is played, in one word; a live trademark on an attraction and a town name dilute it |
| **jameo** | Canarian word for the hollow left where a lava-tube roof fell in; Jameos del Agua, Lanzarote, Manrique's concert hall inside the tube; five letters | bare and all nine free | 27 repos, nothing | `.dev`, `.io`, `.rs` free; `.org` registered | Spanish "HA-meh-o", read "JAY-mee-oh" elsewhere: the fork criterion 2 forbids | Honest fit (a hollow, acoustics), unsayable outside Spain |
| **kazbek** | Mount Kazbek / Mkinvartsveri, 5054 m: Amirani, the Georgian Prometheus, chained in the Betlemi cave at 4000 m, later a hermitage; six letters | bare and all nine free | 141 repos, personal names (a common Caucasian given name). *Kazbek*, the Soviet elite cigarette brand (defunct) | `.io`, `.rs`, `.org` free; `.dev` registered | A sacred mountain; the Russian exonym for a Georgian peak | The best mountain-with-a-cave myth, carried down by the exonym, the brand and the first name |

**The owner's eight names, same bar.** `akkado` (Akkadō, 安家洞, Japan's longest limestone cave,
23.7 km; all ten crate forms and all four domains free; 90 GitHub repos, every one Akka.NET's
`akkadotnet-*`, so the search is owned; three syllables, "ah-KAH-doh"; the cleanest of the
eight after `antrum`, not a finalist). `torres` (Gruta das Torres, Pico, Portugal's longest lava
tube; all ten crate forms free, but 15,212 repos of surname, Miguel Torres S.A.'s 260 US marks
including TORRES SMART WINE in software classes, `.dev`/`.io`/`.org` registered; cut). **Noon's
Hole** (Fermanagh; earlier "Sumera", abyss; renamed in the 1820s after Dominick Noone, murdered
and thrown into the shaft, the killer never found; cut, the name exists because of a killing).
`smoo` (Smoo Cave, Durness; Norse *smuga* "hiding place"; **Australian vulgar slang for the
vagina**, Green's Dictionary of Slang; cut). **La Mouthe** (Les Eyzies; Rivière's 1899 red-sandstone
fat lamp with an engraved ibex, the first Palaeolithic lamp recognised as such, MAN 50 295; all
crate forms and domains free for `mouthe` and `lamouthe`; cut on shape, "mouth-e"/"moot"; the
best light-in-a-cave story found). `spy` (Grotte de Spy, the 1886 Neanderthals; bare crate taken,
35k repos, every domain taken; cut). `tavrida` (a cave found in 2018 under the roadbed of the
Russian state "Tavrida" highway in annexed Crimea, opened 2022; Tavrida is the imperial name of
Crimea, Tavrida.ART a Rosmolodezh youth forum, Tavrida Electric a Russian switchgear group;
cut on sensitivity). **Shan-Koba** (Crimean Mesolithic type site; *koba* is Crimean Tatar for
"cave", a fine generic, but "Koba" was Stalin's party name, the site is in annexed Crimea and the
word is eight letters with a hyphen; cut).

**Other cuts, one line each.** `movile` (the chemosynthetic cave; Movile is the Brazilian iFood
holding), `mulu` (MULU Corp, classes 42/35, registered 2026-02), `waitomo` (a Māori place-name
owned by the glowworm-cave tour brands), `lascaux` (silent x; a Swiss paint brand since 1963 with
a US mark; `lascaux.io` taken), `chauvet` (stage lighting), `pelion` (Arm's IoT platform),
`drach` (Cuevas del Drach; GitHub prefix owned by drachtio, 339 stars; a dragon word), `staffa`
(reads "staff"), `doolin` (Doolin Cave and the music village; a GitHub user's fifty repos,
`doolin.dev`/`.io` taken), `ushba` (Dali's mountain; Svan "mountain of misfortune" per travel
sources, a Pakistani given name, three domains taken), `vihren`, `pelmo`, `krafla` (clean, say
nothing of hollows; a `vihren-dev` account, the Pelmodoro app, `krafla.io` taken), `tsingy`
(Malagasy "where one cannot walk barefoot", clean registries, the *ts-* onset forks), `tiankeng`
(eight letters), `ogof` (Welsh "cave"; `.dev`/`.io` taken, "OG-ov" vs "OH-gof"), `jeita`, `kungur`
(free everywhere, flat), `yordas` (an infant-eating giant), `sonora` (a 1.35 M-download audio
crate), `acheron`, `alum`, `duat`, `ossa`, `stromboli` (taken), `jama` (Japanese 邪魔,
"nuisance"), `lerna`, `noctis`, `tharsis`, `scylla`, `erebor`, `moria` (products or Tolkien
marks), `denali`, `tahoma` (a truck trim and a font).

**How the third sweep compares.** None of it beats `karst` on criterion 1: karst says caves and
limestone to every reader, `naica` says crystals to those who know the cave, `antrum` says cave to
Latinists and sinus to doctors. On criterion 3 `naica` and `antrum` are cleaner than `karst` (bare
free, no adjacent placeholder); on criterion 4 each carries one small collision (an MMORPG; a film
and an air-quality startup) against karst's two small games. `doline` stays the collision-free
option; `antrum` is its nearest rival from this sweep, `ponor` a more obscure sibling.

Sources for this appendix: the crates.io, GitHub and domain sweeps of 2026-11-17 (scripts
and results kept by the sweeper, not in the repository);
https://en.wikipedia.org/wiki/Cave_of_the_Crystals; https://showcaves.com/english/mx/caves/Naica.html;
https://store.steampowered.com/app/3562850; https://en.wiktionary.org/wiki/antrum;
https://en.wikipedia.org/wiki/Antrum_(film); https://brandfetch.com/antrum.com;
https://en.wikipedia.org/wiki/Fingal%27s_Cave; https://en.wiktionary.org/wiki/фингал;
https://www.postojnska-jama.eu/en/about-the-management/; https://en.wikipedia.org/wiki/Ponor;
https://en.wiktionary.org/wiki/ponor; https://en.wikipedia.org/wiki/Luray_Caverns;
https://trademarks.justia.com/owners/luray-caverns-corporation-132618; https://www.rae.es/tdhle/jameo;
https://en.wikipedia.org/wiki/Kazbek; https://en.wikipedia.org/wiki/Kazbek_(cigarettes);
https://en.wikipedia.org/wiki/Akkadō; https://en.wikipedia.org/wiki/Gruta_das_Torres;
https://trademarks.justia.com/owners/miguel-torres-s-a-2472142; https://en.wikipedia.org/wiki/Noon%27s_Hole;
https://www2.habitas.org.uk/escr/summary.php?item=1161; https://greensdictofslang.com/entry/vlkyw4a;
https://donsmaps.com/lamouthe.html; https://en.wikipedia.org/wiki/Spy_Cave;
https://tass.com/society/1012874; https://www.cbinsights.com/company/tavrida-electric;
https://en.wikipedia.org/wiki/Kizil-Koba_culture; https://en.wikipedia.org/wiki/Movile_(company);
https://trademark.justia.com/owners/mulu-inc-2519692; https://lascaux.ch/en/about-us;
https://en.wikipedia.org/wiki/Hekla; https://crates.io/crates/teide; https://crates.io/crates/hekla.
Not verified: EUIPO and WIPO registers; `.com` for any word; brands in Chinese, Japanese or
Arabic markets; the Tarahumara etymology of Naica and the Svan one of Ushba.

## Outcome

**2026-11-17, accepted with `svet`** (owner: "for renaming, I think svet is the best"). On the
day of the pick the bare `svet` and `svet-world`, `svet-render`, `svet-app`, `svet-sim`,
`svet-assets`, `svet-net`, `svet-script-wasm`, `svet-audio` and `svet-entity` were free on
crates.io (Appendix C's sweep); the four domains were registered. The rename is Plan 0087
(draft, awaits the owner's go), which runs when no lane is open. Its scope re-measured on
`main` at 778fac63 the same day, case-insensitive: `vxe` appears 5130 times in 466 tracked
files; of those, 2310 occurrences in 137 files are the records this ADR keeps (every file
under `docs/adrs/`, `docs/plans/done/`, `docs/plans/logs/` and `CHANGELOG.md`; the proposed
ADRs among them, about 55 occurrences in 10 files, are rewritten) and 2820 occurrences in 329
files are the live tree the plan substitutes (257 `.rs`, the active plans and `docs/`, 22
`.toml` and the lockfile, the three workflows, the scripts, `dist/`, two `.voxtext` headers,
the WIT file, one WGSL comment). `VXE_*` names: 84 distinct. The harness repository under
`.claude/` carries 41 more in 9 files. The first consumer's checkout (`~/Work/voxel-roguelite`)
holds no `vxe` at all today, so there is nothing to update there.
