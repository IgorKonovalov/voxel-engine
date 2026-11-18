# Placing things in a first-person view: build modes, hotbars, ghosts and thumbnails

An annotated reading list for the fly-through's entity placement (Plan 0077, amended
2026-11-18 after the owner's first window): a chooser with a small picture per model, a belt of
slots at the screen's edge, a placement mode in which the camera moves freely and a marked
ghost of the model stands at the aimed surface, keys to confirm, cancel and turn, and a pad.
Every entry names what the source says and what we take from it, so the plan cites it by
section. The figures are the sources' own; the plan's done-whens hold ours. Sources were read
on the web on 2026-11-18 at the URLs given. Several first-party pages were unreachable that
day (Fandom wikis answered 402, `valheim.wiki.gg` 401, EA Help timed out, Medium 403); where a
convention is known only from a mod page that describes the vanilla game, or from a search
snippet, the entry says so, and the last section lists what could not be verified rather than
inventing it.

## The belt: a hotbar of slots

- **Minecraft Wiki, "Inventory".** <https://minecraft.wiki/w/Inventory> "The inventory
  consists of 4 armor slots, 27 storage slots, 9 hotbar slots, and an off-hand slot." "Items
  in the hotbar slots can be selected using the keyboard keys (1-9) or by moving the Scroll
  Wheel." While the inventory grid is open, "pressing (by default) 1-9 switches the item in
  that inventory slot with the item in that slot of the hotbar." What we take: nine slots, the
  wheel and the digits pick one, and a digit pressed over an entry in the chooser puts it into
  that slot without dragging (there is no cursor here, so this is the one assignment path).
- **Minecraft Wiki, "Heads-up display".** <https://minecraft.wiki/w/Heads-up_display> The
  hotbar "consists of the bottom nine slots within the player's inventory." "When the player
  switches an item in hand by hotbar scrolling, its name is shown above the experience level
  for a short period of time." The selected slot's frame is drawn in the page's images, not
  described in its text. What we take: the belt sits at the bottom centre; the selected slot's
  name shows above it for a moment when the selection changes (`hud::BANNER_MS`), so a wheel
  step is read without looking at the icons.
- **Minecraft Wiki, "Creative inventory".** <https://minecraft.wiki/w/Creative_inventory>
  "The Creative inventory screen is sorted into 11 sections"; "Players can search for the
  items they want to use with the search bar at the top or in several tabs." "Pressing a
  number key while hovering over an item instantly places one full stack of that item into the
  hotbar slot that corresponds with the number." What we take: the chooser is the full
  catalogue with a search (the menu's palette filter already types), the belt is the working
  set; a digit on the selected entry fills that slot and keeps the chooser open. Tabs are not
  taken: the library has one kind of thing.
- **Minecraft Wiki, "Controls".** <https://minecraft.wiki/w/Controls> Hotbar Slot 1-9
  "Selects the appropriate hotbar item."; Scroll Up: "Move the selected hotbar slot one to the
  left."; Pick Block is the middle mouse button; on Bedrock with a controller "Cycle Item
  Left" and "Cycle Item Right" are LB and RB. What we take: the wheel steps the slot one at a
  time and wraps. The pad's bumpers are the shell's sink and rise here, so the belt takes the
  D-pad's left and right instead (the dig radius left the D-pad, owner 2026-11-18). A "pick"
  of the aimed entity into the belt is not taken now.
- **Terraria Wiki, "Hotbar".** <https://terraria.wiki.gg/wiki/Hotbar> "A single row of ten,
  each of which corresponds to the 0-9 keys." "Pressing a slot's matching number key will
  cause it to be highlighted." "Highlighted item can be shifted by scrolling with the mouse
  wheel." Its history notes "Hotbar now shows the name of the selected item." What we take:
  the name beside the slot was added as a fix, so we draw it from the start; ten slots on
  `1`..`0` is the alternative to nine, and nine is enough for a library of a few models.
- **Satisfactory Wiki, "Hotbar" and "Controls".** <https://satisfactory.wiki.gg/wiki/Hotbar>,
  <https://satisfactory.wiki.gg/wiki/Controls> "There are 10 hotbars." A slot is assigned by
  pressing "a number (1-0) on your keyboard" with the build menu open; `Alt` + a slot number
  is "Save Hologram to Hotbar", `Alt` + scroll "Cycle To Next Hotbar", and on a controller
  LB/L1 "Cycle To Next Hotbar". Patch 0.8.2.0 "Added a way to save the current hologram into
  the hotbar." What we take: one belt; several belts are a mechanism for a hundred buildables,
  not for a library of a few. Saving the ghost with its turn is not taken (the ghost faces the
  camera, turned by a key, below).
- **Space Engineers Wiki, "Xbox Controls".** <https://spaceengineers.wiki.gg/wiki/Xbox_Controls>
  "open the build menu": press the left stick; categories on LT and RT; "select a block":
  "Use either stick and press RB to confirm"; "Add block to build planner": "Press X on a
  block". What we take: the pad reaches the chooser through the menu (`Start`, the `entities`
  group, `choose`), as it reaches every page; `South` on an entry does what `Enter` does.
- **Not found:** an authoritative UX source on hotbar design (slot count, always visible or
  only in a mode, the frame's style). The games above agree on nine or ten slots, digits plus
  the wheel, a highlighted frame and a transient name; we take those four and judge the rest in
  the window.

## The ghost: how a placement preview looks

- **Satisfactory Wiki, "Build Gun".** <https://satisfactory.wiki.gg/wiki/Build_Gun> The gun
  "projects a hologram showing where the buildable structure will be placed"; "Valid
  placements appear blue, hard clearance appears red, and soft clearance appears yellow",
  "highlighting key features such as Conveyor and Pipeline inputs, outputs and power
  connectors." What we take: the ghost's state is a colour (valid, blocked, a warning), and
  the valid colour is blue, not green; we have two states (can place, cannot) and name the
  reason in words beside the colour.
- **Satisfactory Wiki, "Build clearance".** <https://satisfactory.wiki.gg/wiki/Build_clearance>
  Captions: "valid Build Gun building placement hologram (default blue color)", "invalid Build
  Gun building placement hologram (default red color)"; "Hologram Color for both Soft
  Clearance and Hard Clearance can be set in Settings"; a soft block shows the words
  "Overlapping other's clearance, clipping may occur". What we take: the mark's colours are
  settings, not constants (a render setting per ADR-0055, so a consumer or a colour-blind
  owner changes them); a refusal shows its words in the notice row.
- **Satisfactory modding docs, "What is a Buildable Hologram?".**
  <https://docs.ficsit.app/satisfactory-modding/latest/Development/Satisfactory/BuildableHolograms.html>
  "Hologram code becomes active the moment the buildable is picked from the build menu." "It
  is possible to show the red outline and the red hologram while placing the buildable." What
  we take: the ghost appears the moment a slot is picked, not on a further key; an invalid
  ghost carries two signifiers at once (the outline and the fill), which is also XAG 103's rule
  below.
- **Space Engineers Wiki, "Block Placement Mode".**
  <https://spaceengineers.wiki.gg/wiki/Block_Placement_Mode> "you see a ghost preview of the
  selected block and you can click to place the block in the world." The ghost's colours are
  not stated on the wiki's pages. What we take: the word "ghost" and the click.
- **Minecraft Wiki, "Interaction range".** <https://minecraft.wiki/w/Interaction_range> "When
  within the reach distance, targeted blocks are highlighted"; "this manifests as an
  interaction box wireframe around the block"; the "Outline Selection" video option, when
  off, "replaces the wireframe with a white highlight." What we take: Minecraft marks the
  target, not a ghost of the thing placed; the owner wants the ghost (the model is not a unit
  cube), with the outline as the mark. The wireframe-or-highlight pair is the outline-or-tint
  choice in owner question 1.
- **Valheim, from mod pages describing the vanilla game** (the wikis were unreachable).
  Hardwire99, "FlipIt", <https://thunderstore.io/c/valheim/p/Hardwire99/FlipIt/>: "Scroll
  wheel - rotate on Y (vanilla, 22.5 degrees per step)". Enigma Prime, "Enigma's Snug Fit",
  <https://thunderstore.io/c/valheim/p/Enigma_Prime/Enigmas_Snug_Fit/>: "The changes are
  applied to the placement ghost before the game checks snapping and placing"; "The piece
  keeps snapping to what it is pointed at." What we take: the pipeline order, ghost transform
  then snap then validity, is the one `stand_preview` follows; the ghost's translucency and
  colours in Valheim could not be verified and are not cited.
- **Not found:** a transparency figure for any game's ghost; The Sims' green and red
  footprint, Fortnite's blueprint colours, Teardown's editor and Enshrouded's keys reached
  only as search snippets. A translucent ghost would need translucent entity voxels in the
  tracer (`entity.wgsl` has none); the outline and the tint need no such path, which decides
  the mechanism as much as the convention does.

## Where the ghost stands: distance and snapping

- **Minecraft Wiki, "Interaction range"** (above). "The default block interaction range is
  4.5 blocks in all other game modes and 5 blocks in Creative mode"; on Bedrock "the default
  interaction range is 5 blocks for keyboard, mouse, or controller inputs", and for touch "the
  reach extends to 12 blocks in Creative mode and 6 blocks in other modes." What we take: a
  reach is a stated figure, longer in an editing mode than in play; our dig reach is 10 m
  (`REACH`, 100 voxels), and the placement reach is longer (the plan's figure), since a 7 m
  spider cannot be aimed at 4 m.
- **Satisfactory Wiki, "Build Gun"** (above). "The construction and dismantle range is 100
  meters (12.5 foundation lengths)." Holding Left Ctrl shows guidelines; "Green snap
  guidelines will be visible to help align to inputs/outputs." What we take: a long reach in a
  third-person-scale builder; snapping as a held modifier is not taken (nothing to snap a
  creature to but the floor).
- **Space Engineers Wiki, "Key Bindings" and "Block Placement Mode".**
  <https://spaceengineers.wiki.gg/wiki/Key_Bindings> "Change block preview distance": Ctrl +
  scroll (Creative Mode). The B key toggles "between free placement mode, local-grid
  alignment mode, and gravity-aligned block placement". What we take: the ghost's distance
  can be the player's to push in and out; it is owner question 2's alternative to a rule from
  the model's size, since our wheel picks the slot.
- **Unity Manual, "Position GameObjects".**
  <https://docs.unity.com/en-us/engine/current/manual/working-with-scenes/scenes-manage-gameobjects/positioning-game-objects>
  "Surface snapping: Snap the GameObject to the intersection of any Collider." Hold
  Shift+Ctrl, "Drag the GameObject on to another GameObject with a Collider." What we take:
  the pivot goes to the hit point on the aimed surface; our pivot is the model's feet.
- **Unreal Engine docs, "Actor Snapping".**
  <https://dev.epicgames.com/documentation/en-us/unreal-engine/actor-snapping-in-unreal-engine>
  "Surface snapping makes Actors align to the floor or another surface." "If you select an
  Actor and press the End key, the Actor snaps to the nearest surface." A Surface Offset:
  "Modify this value to configure the distance between the Actor and the surface it snaps
  to." Rotate to Surface Normal: "If enabled, Actors will automatically rotate to match the
  alignment of the surface they snap to." What we take: when the aim hits a wall or a ceiling
  the ghost is dropped to the floor below the hit (a downward ray), as "drop to floor" does;
  aligning to the normal is not taken (a creature stands upright; Plan 0079's surface mover
  owns anything that climbs).
- **Godot docs, "Introduction to 3D".**
  <https://docs.godotengine.org/en/stable/tutorials/3d/introduction_to_3d.html> The Transform
  menu "Snaps an object to a solid floor." "Snapping can also temporarily be activated using
  Ctrl while performing the action." What we take: the same floor snap, a third editor
  agreeing.
- **Not found:** Valheim's build range and its 20 m workbench radius, Rust's build distance
  (its wiki page names no figure), a written rule for a minimum placement distance in any
  game. Our minimum (the ghost no nearer than a distance set by the model's size) is the
  owner's complaint turned into a rule, judged in the window.

## The keys: confirm, cancel, turn, repeat, undo

- **Satisfactory Wiki, "Controls"** (above). Q "Activates Build Mode and opens Build Menu";
  Left Button "Place buildable"; scroll up and down "Rotate Hologram Right" and "Rotate
  Hologram Left"; H "Lock Hologram" ("Using Left Button will place buildable/blueprint,
  pressing H again will reset the origin"); the arrows nudge, "Holding Ctrl will nudge in 0.5m
  steps"; R "Change Build Mode"; F "Activates Dismantle Mode". What we take: the left click
  places and the mode stays (the hologram is still there after a placement); a turn key; no
  lock and nudge (one more sub-mode than a testbed needs).
- **Minecraft Wiki, "Controls"** (above). Attack/Destroy on the left button: "Destroy blocks
  (hold down); Attack entity (click once)."; Use Item/Place Block on the right: "Place blocks,
  summon entities with spawn eggs, toggle levers/doors (click once)". No rotate key. What we
  take: the one game with the left-destroys, right-places split, which is the fly-through's
  dig and fill today. Owner question 3 chooses between it and the builders' left-places.
- **Space Engineers Wiki, "Key Bindings" and "Building"** (above). "Rotate block preview:
  vertical axis +" Page Down, "vertical axis -" Delete, horizontal axis Home and End, depth
  axis Insert and Page Up; "Press R key (or press the same toolbar slot number again) to toggle
  between large or small size before placing the block." What we take: three axes on six keys
  is for blocks; a creature turns about one axis, so one key with a step. Pressing the slot's
  own digit again as a toggle is noted: our second press of the selected slot's digit leaves
  the mode (owner question 4's alternative).
- **Valheim, "FlipIt"** (above): "Scroll wheel - rotate on Y (vanilla, 22.5 degrees per
  step)". What we take: the step. Our wheel is the belt's, so the turn is a key (`R`, which is
  free), in 45 degree steps: a model faces the camera by default and 45 degrees is the
  smallest step the eye reads on a voxel model at a distance; 22.5 is the alternative.
- **Satisfactory Wiki, "Controls"**, the controller half. LB/L1 "Build Menu"; RT/R2 "Build"
  ("Place buildable"); B/Circle "Unequip Build Gun" (in the menu, "Close"); D-pad right and
  left (hold) "Rotate Hologram Right" and "Left"; RT/R2 long press "Hologram Lock"; D-pad
  down "Change/Switch Build Mode". What we take: the pad places on the trigger that digs
  (`R2`) and leaves the mode on `East`; the D-pad's left and right are the belt's slots (the
  owner freed the D-pad from the dig radius), so the turn goes to `West` (the cutaway's
  button, menu-only in the mode) or stays on the keyboard; owner question 5.
- **Space Engineers Wiki, "Xbox Controls"** (above): "rotate block before placing": "D-left
  and D-right"; confirm on RB. What we take: a second shipped mapping with the D-pad turning;
  ours gives the D-pad to the slots and keeps one button for the turn.
- **Not found:** The Sims' `,` and `.` turn, its Shift repeat and Ctrl+Z; Fortnite's piece
  keys and its R; Valheim's pad map; Enshrouded's R, X and Y (search snippets only). Undo is a
  removal here (`Delete` the aimed, `4` the newest), as the plan already has; Teardown's author
  on undo (<https://blog.voxagon.se/>, "Undo for lazy programmers", 2018) is about editors with
  a document, which the fly-through is not.

## The chooser and its pictures

- **Unreal Engine docs, "Placing Actors".**
  <https://dev.epicgames.com/documentation/en-us/unreal-engine/placing-actors-in-unreal-engine>
  "The Place Actors panel will open at the left-hand side of the Unreal Editor window."
  "Left-click the Asset, then drag it into the Level Viewport and drop it at the location
  where you want to place it." What we take: a panel of thumbnails is the editors' convention
  for choosing a thing to place; dragging needs a cursor we do not have (backlog, "Mouse
  pointing"), so the chooser hands the pick to the belt and the belt to the aim.
- **Unity Scripting API, "AssetPreview".**
  <https://docs.unity.com/en-us/engine/current/script-reference/unityeditor/assetpreview>
  "Returns a rendered preview texture for an asset. This is the large image shown in the
  Project window grid view." "Previews load asynchronously, so poll this and repaint when it
  finishes." "Set the asset preview cache to a size that can hold all visible previews on the
  screen at once." What we take: a picture is made once and kept; it arrives later than the
  row, which shows its words meanwhile.
- **Unreal Engine docs, "Content Browser Settings".**
  <https://dev.epicgames.com/documentation/en-us/unreal-engine/content-browser-settings-in-unreal-engine>
  Real-Time Thumbnails: "If enabled, Asset thumbnails are rendered in real time." Five sizes,
  Tiny to Huge. Thumbnail Edit Mode: "you can adjust the thumbnails of 3D Assets by
  left-clicking and dragging inside the thumbnail." What we take: a live picture is an
  opt-in, the still is the default (owner question 6); a per-model camera angle is not taken,
  one fixed view for all.
- **Godot docs, "EditorResourcePreview" and "EditorResourcePreviewGenerator"; the editor's
  settings source.** <https://docs.godotengine.org/en/stable/classes/class_editorresourcepreview.html>,
  <https://docs.godotengine.org/en/stable/classes/class_editorresourcepreviewgenerator.html>,
  <https://raw.githubusercontent.com/godotengine/godot/master/editor/settings/editor_settings.cpp>
  "Once the preview is ready, the receiver's receiver_func will be called." The generator
  "is always called from a thread (not the main thread)." Defaults:
  `docks/filesystem/thumbnail_size` 64 (range 32 to 224 in steps of 16),
  `filesystem/file_dialog/thumbnail_size` 64 (32 to 128). What we take: 64 pixels is the
  working size three editors converge near; the picture is made off the frame thread and
  delivered when done.
- **Minecraft's `block.json` (the vanilla assets).**
  <https://raw.githubusercontent.com/InventivetalentDev/minecraft-assets/1.21/assets/minecraft/models/block/block.json>
  `"gui": { "rotation": [ 30, 225, 0 ], "scale": [ 0.625, 0.625, 0.625 ] }`. What we take:
  the hotbar icon's fixed view, 30 degrees down and 225 degrees round (a three-quarter view
  showing the top and two sides), is the one our icon renderer uses; the model is fitted to
  the tile at about 5/8 of its width.
- **Not found:** Unity's and Unreal's thumbnail pixel sizes in a first-party page; a written
  recommendation on orthographic against perspective or on neutral lighting. We take
  orthographic (a voxel model's edges stay parallel; the icon is a diagram) and a fixed
  three-light Lambert shading from the hit normal, labelled a choice, not a convention.

## Outlines: how a marked object is drawn

- **Alexander Ameye, "5 ways to draw an outline".** <https://ameye.dev/notes/rendering-outlines/>
  Edge detection: "This full-screen pass draws lines by detecting discontinuities in the
  scene". Jump flood: "The main advantage of this technique is that it can render really wide
  outlines, at a very reasonable performance cost." Blurred buffer: "can have a bigger impact
  on performance compared to other methods"; "the silhouette buffer can be scaled down before
  blurring." Vertex extrusion: "a width unit 1 will correspond with exactly 1 pixel on the
  screen." What we take: an edge detect over an id buffer costs one pass of neighbour taps and
  gives a one-pixel line; the jump flood (several passes) buys width we do not need; the
  extrusion needs a rasteriser we do not have.
- **Tom Looman, "Unreal Engine 4 Custom Depth".** <https://tomlooman.com/unreal-engine-custom-depth/>
  "we can perform a simple depth comparison by sampling the neighboring pixels in our
  post-process." "If a neighbor has depth info but the pixel itself does not - we color it to
  our outline color." "Because the buffer is filled with all objects using Custom Depth it
  will not work when having multiple overlapped meshes." What we take: the classic
  mechanism, and its known flaw: a depth-only mask cannot separate two marked objects that
  overlap. Our hit record carries the instance's slot (`HIT_ENTITY`, `HIT_INSTANCE_SHIFT`), so
  the edge is a change of instance, not of depth, and two instances touching still get their
  edge.
- **Tom Looman, "Multi-color Outline Post Process".**
  <https://tomlooman.com/unreal-engine-outline-multi-color-post-process/> "Custom Stencil is
  disabled by default"; a stencil index "assign within 1-255 range"; the multi-colour material
  costs "144" instructions against "97". What we take: a per-instance mark word (a colour and
  a style) is the stencil index's equivalent; the ghost and a selected placed entity can carry
  different colours in the same pass.
- **Ben Golus, the jump flood outline shader (gist; the article was unreachable).**
  <https://gist.githubusercontent.com/bgolus/a18c1a3fc9af2d73cc19169a809eb195/raw> "// sample
  silhouette texture for sobel", "// jump samples", "// distance is already in pixels, so
  this is already perfectly anti-aliased!" What we take: the jump flood's gain is an exact,
  anti-aliased width at any size; a one- or two-pixel line from a neighbour test is enough for
  a ghost's border, and it is the rejected alternative the ADR names.
- **Unity Manual, "Gizmos menu".**
  <https://docs.unity.com/en-us/engine/current/manual/unity-editor/editor-windows-views-reference/gizmos-and-handles/gizmos-menu>
  "Check Selection Outline to display selected GameObjects with a colored outline, and their
  children with a different colored outline. By default, Unity highlights the selected
  GameObject in orange, and child GameObjects in blue." What we take: an outline is how an
  editor marks the selected thing; orange is its colour, and a second colour marks a second
  role.
- **Godot's `editor_settings.cpp` and Blender's default theme.**
  <https://raw.githubusercontent.com/godotengine/godot/master/editor/settings/editor_settings.cpp>:
  `editors/3d/selection_box_color` `Color(1.0, 0.5, 0)`, `editors/3d/active_selection_box_color`
  `Color(1.5, 0.75, 0, 1.0)`.
  <https://raw.githubusercontent.com/blender/blender/main/release/datafiles/userdef/userdef_default_theme.c>:
  `.select = RGBA(0xed5700ff)`, `.active = RGBA(0xffa028ff)`. What we take: three editors
  mark a selection in orange, two of them with a brighter orange for the active one. The
  ghost is not a selection, so it takes the builders' blue (above) and leaves orange for a
  selected placed entity if a later plan selects one; a blocked ghost is orange here, the
  colour-blind-safe pair below.
- **Not found:** a first-party statement that an outline is 1 to 3 pixels and scales with the
  resolution. Ameye's "width unit 1 ... exactly 1 pixel" is the nearest. We draw one output
  pixel at 1080p and two at 1440p and above, and the window judges.

## Colour, and never colour alone

- **Microsoft, "Xbox Accessibility Guideline 103: Additional channels for visual and audio
  cues".** <https://learn.microsoft.com/en-us/gaming/accessibility/xbox-accessibility-guidelines/103>
  "Color alone should never be used to represent information." "If color reliance is
  unavoidable, providing players the option to assign their own color choice for individual
  game elements is important." The failure example: "when enemy characters are outlined in
  red, ally characters are outlined in green". What we take: the ghost differs from a placed
  entity by the outline (a shape cue), not by colour alone; a blocked ghost changes colour and
  the notice row says why; the colours are render settings.
- **Microsoft, "Xbox Accessibility Guideline 113: UI focus handling".**
  <https://learn.microsoft.com/en-us/gaming/accessibility/xbox-accessibility-guidelines/113>
  "All UI elements should display a highly visible focus indicator when they receive focus",
  "borders around the element, increasing the element's size or font weight upon receiving
  focus, or combinations of multiple methods". What we take: the belt's selected slot has a
  frame and its name; the chooser's selected entry keeps the menu's `> ` and yellow.
- **Game accessibility guidelines, "Ensure no essential information is conveyed by a colour
  alone".** <https://gameaccessibilityguidelines.com/ensure-no-essential-information-is-conveyed-by-a-colour-alone/>
  "Difficulty perceiving red or green in particular is very common, affecting around 8-10% of
  males"; "Orange Vs blue works for the three most common three types." What we take: the
  ghost's two states are blue (can place) and orange (cannot), not green and red; Satisfactory
  ships blue for valid already.
- **Satisfactory Wiki, "Build clearance"** (above): the hologram colours are settings. What
  we take: the same, through the render settings and their profiles (ADR-0055).

## Developer material

- **GDC Vault, Billy Bramer (Epic), "Under Construction: Crafting Fortnite's Building
  System" (GDC 2013).** <https://www.gdcvault.com/play/1017937/Under-Construction-Crafting-Fortnite-s>
  The abstract: "attention will be paid to the complicated task of allowing players to build
  rapidly and accurately" without "hampering their architectural creativity in the process."
  The talk itself is paywalled. What we take: the two goals, fast and accurate, are the
  owner's two complaints (the panel in the way; the ghost too close to aim).
- **Not found:** a Valheim, Satisfactory or Teardown developer post on build-mode UX; Celia
  Hodent's "Developing a UX Mindset on Fortnite" (GDC Vault 1025826) surfaced but was not
  read.

## Could not be verified on 2026-11-18

Valheim's ghost colours, build range and pad map; Space Engineers' grey and red; The Sims 4's
keys and footprint; Fortnite's piece keys and colours; Teardown's editor placement; Besiege,
Rust's range, Planet Coaster, Enshrouded; Godot's Page Down floor snap (in a pull request, not
the manual); Unity's and Unreal's thumbnail pixel sizes; Unreal's selection colour page; Ben
Golus's article text; any transparency figure for a ghost; a hotbar design guideline. None of
these is cited as fact above.
