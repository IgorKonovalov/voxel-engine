//! Plan 0077 with no run: the chooser (the entities page) as the menu writes it from the
//! library and the belt, and the keys the menu takes for it.

use std::path::{Path, PathBuf};

use flythrough::entities::{self, library};
use flythrough::hud::Modes;
use flythrough::hud::menu::{
    Activated, MENU_COLS, MENU_ROWS, Menu, PageId, RouteInput, RunView, step, taken,
};
use flythrough::walker;
use vxe_app::{Input, KeyCode};
use vxe_render::{HudColour, Lighting, LightingResolution};

/// A library of `a` (the committed walker), `b` (the walker broken on line 3) and `spider`
/// (the committed spider) under the target's scratch directory (`picker-rows-<tag>`, one per
/// test, since nextest runs them at once), built against the showcase's table with the
/// walker's rows.
fn library_abs(tag: &str) -> library::Library {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("picker-rows-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    let rig = std::fs::read_to_string(from.join("walker/walker.rig")).expect("reads");
    for (name, text) in [
        ("a", rig.replace("vox walker.vox", "vox a.vox")),
        ("b", {
            let mut l: Vec<&str> = rig.lines().collect();
            l[2] = "bogus 1 2 3";
            l.join("\n").replace("vox walker.vox", "vox b.vox") + "\n"
        }),
    ] {
        std::fs::create_dir_all(dir.join(name)).expect("dir");
        std::fs::write(dir.join(format!("{name}/{name}.rig")), text).expect("rig");
        std::fs::copy(
            from.join("walker/walker.vox"),
            dir.join(format!("{name}/{name}.vox")),
        )
        .expect("vox");
    }
    std::fs::create_dir_all(dir.join("spider")).expect("dir");
    for ext in ["rig", "vox"] {
        let f = format!("spider/spider.{ext}");
        std::fs::copy(from.join(&f), dir.join(&f)).expect("copied");
    }
    let mut lib = library::scan(&dir);
    lib.build(&walker::with_walker(&flythrough::showcase_materials()));
    lib
}

fn modes_with(lib: library::Library) -> Modes {
    let mut m = Modes::new(Lighting::default(), LightingResolution::Half, 0.0);
    m.walkers = entities::Walkers::default().with_library(lib);
    m
}

/// Phase 5, amended by Phase 9 (3): `a`, `b` (in error) and `spider`, the belt filled in the
/// chooser's order (`a`, `spider`, then `b` in error), `spider` selected on its second clip:
/// each entry takes a block of four rows, its 4 x 4 cell tile over all four from the third
/// cell (`a`'s and `spider`'s once their icons are made; `b` has none), its words (with the
/// `>` cell when selected) on the block's second row, its clips with the playing one marked
/// and its slot on the third, and no words beside the tile on the first and fourth; `b` grey
/// with its error after its name, the error a red span; no row past 80 columns or 45 rows.
#[test]
fn the_picker_draws_its_rows() {
    let mut lib = library_abs("rows");
    let v = lib.entries[2].viewer_mut().expect("a rig");
    assert_eq!(v.cycle(1), "clip idle");
    let mut m = modes_with(lib);
    let order = m.walkers.library.order();
    assert_eq!(order, [0, 2, 1]);
    m.walkers.belt.fill(&order);
    m.walkers
        .refresh_icons(&walker::with_walker(&flythrough::showcase_materials()));
    m.walkers.icons.wait();
    let mut menu = Menu::default();
    menu.open_entities(1);
    let (mut text, mut colours) = (String::new(), Vec::new());
    let mut paint = entities::belt::Paint::default();
    menu.write_with(
        &m,
        &RunView::default(),
        None,
        &mut text,
        &mut colours,
        &mut paint,
    );
    let rows: Vec<&str> = text.lines().collect();
    assert!(rows.len() <= MENU_ROWS && rows.iter().all(|r| r.len() <= MENU_COLS));
    let want: Vec<(&str, HudColour)> = FIXTURE.to_vec();
    let have: Vec<(&str, HudColour)> = rows.iter().copied().zip(colours.iter().copied()).collect();
    assert_eq!(have, want, "{text}");
    let tile = |row, index| vxe_render::HudTile { col: 2, row, index };
    assert_eq!(
        paint.tiles,
        [tile(1, 0), tile(5, 2)],
        "a block's first row, the third cell"
    );
    // `b`'s reason, after its name padded to ten cells, is a red span over the rest of its
    // grey words row (Phase 9 (6)).
    let b = rows[10];
    let reason = vxe_render::HudBox {
        col: 19,
        row: 10,
        cols: (b.len() - 19) as u32,
        rows: 1,
    };
    assert_eq!(&b[19..], "rig line 3: unknown statement `bogus`");
    assert_eq!(paint.spans, [(reason, HudColour::Red)]);
    for block in [1, 5, 9] {
        assert!(rows[block].is_empty() && rows[block + 3].is_empty());
        assert!(rows[block + 1].len() > 8 && rows[block + 2].len() > 8);
        assert!(rows[block + 1][2..8].trim().is_empty(), "the tile's cells");
    }
    assert!(rows[6].starts_with("> "), "the cursor on spider's words");
}

/// The colour code of each cell of `text`'s rows (`colours` per row) with `paint` laid over
/// them, as the overlay writes the cell words; a cell past its row's end reads its row's
/// colour only when a span reaches it.
fn cell_colours(text: &str, colours: &[HudColour], paint: &entities::belt::Paint) -> Vec<Vec<u32>> {
    let cols = vxe_render::HUD_GRID_COLS as usize;
    let rows = text.split('\n').count();
    let mut cells = vec![0u32; rows.max(45) * cols];
    for (r, line) in text.split('\n').enumerate() {
        for (c, b) in line.bytes().enumerate().take(cols) {
            cells[r * cols + c] = u32::from(b) | (colours[r] as u32) << 8;
        }
    }
    paint.with(&[]).apply(&mut cells);
    cells
        .chunks(cols)
        .map(|r| r.iter().map(|w| (w >> 8) & 0xff).collect())
        .collect()
}

/// Phase 9 (6): under the cave's table (the walker's rows added) the spider is in error: in
/// the chooser its name cells are grey and the cells of its reason on the same row red (a
/// span), the rest of the row's cells (the blank past it) untouched; its `--library` line
/// still reads `error: ...`; and the belt's name row for its slot reads the name grey and the
/// reason red the same way.
#[test]
fn an_entry_in_error_reads_its_reason_red() {
    let table = walker::with_walker(&flythrough::cave_materials());
    let mut lib = library::scan(Path::new(library::DEFAULT_DIR));
    lib.build(&table);
    let spider = lib
        .entries
        .iter()
        .position(|e| e.name == "spider")
        .expect("spider");
    let err = lib.entries[spider].error().expect("in error").to_owned();
    assert!(err.contains("crystal"), "{err}");
    let line = library::entry_line(&lib.entries[spider], &table);
    assert!(line.starts_with("spider     error: "), "{line}");
    let mut m = modes_with(lib);
    let order = m.walkers.library.order();
    m.walkers.belt.fill(&order);
    let mut menu = Menu::default();
    menu.open_entities(0);
    let (mut text, mut colours) = (String::new(), Vec::new());
    let mut paint = entities::belt::Paint::default();
    menu.write_with(
        &m,
        &RunView::default(),
        None,
        &mut text,
        &mut colours,
        &mut paint,
    );
    let rows: Vec<&str> = text.split('\n').collect();
    let r = rows
        .iter()
        .position(|l| l.trim_start().starts_with("spider "))
        .expect("the spider's row");
    let at = rows[r].find(&err[..err.len().min(20)]).expect("the reason");
    let cells = cell_colours(&text, &colours, &paint);
    let (grey, red) = (HudColour::Grey as u32, HudColour::Red as u32);
    for (c, ch) in rows[r].char_indices() {
        let want = if c >= at { red } else { grey };
        assert_eq!(cells[r][c], want, "cell {c} {ch:?} of {:?}", rows[r]);
    }
    assert!(
        cells[r][rows[r].len()..].iter().all(|&c| c == 0),
        "past the row"
    );
    assert!(
        cells
            .iter()
            .enumerate()
            .all(|(i, row)| i == r || !row.contains(&red))
    );

    // The belt's name row with the spider's slot selected.
    let slot = m.walkers.belt.slot_of(spider).expect("a slot");
    m.walkers.belt.select(slot, 0.0);
    let (mut text, mut colours) = (String::new(), Vec::new());
    let mut paint = entities::belt::Paint::default();
    entities::belt::append_belt(
        &mut text,
        &mut colours,
        (80, 45),
        &m.walkers,
        0.0,
        flythrough::hud::BANNER_MS,
        &mut paint,
    );
    let first = 45 - entities::belt::BELT_ROWS;
    let name_row = text.split('\n').nth(first).expect("the name row");
    assert_eq!(name_row.trim_start(), format!("spider: {err}"));
    let at = name_row
        .find(&err[..err.len().min(20)])
        .expect("the reason");
    let start = name_row.len() - name_row.trim_start().len();
    let cells = cell_colours(&text, &colours, &paint);
    for (c, &cell) in cells[first]
        .iter()
        .enumerate()
        .take(name_row.len())
        .skip(start)
    {
        let want = if c >= at { red } else { grey };
        assert_eq!(cell, want, "belt cell {c} of {name_row:?}");
    }
}

/// The page of `the_picker_draws_its_rows`: the title, the three entries of four rows each
/// (`spider` selected, `b` grey), a blank notice row and the footer.
const FIXTURE: [(&str, HudColour); 15] = [
    ("menu  >  entities", HudColour::White),
    ("", HudColour::White),
    (
        "        a           6 parts  0.6 x 0.3 x 1.5 m",
        HudColour::White,
    ),
    ("        clips idle* walk turn   slot 1", HudColour::White),
    ("", HudColour::White),
    ("", HudColour::Yellow),
    (
        ">       spider     14 parts  6.9 x 4.0 x 2.3 m",
        HudColour::Yellow,
    ),
    ("        clips walk idle* rear   slot 2", HudColour::Yellow),
    ("", HudColour::Yellow),
    ("", HudColour::Grey),
    (
        "        b          rig line 3: unknown statement `bogus`",
        HudColour::Grey,
    ),
    ("        no clip   slot 3", HudColour::Grey),
    ("", HudColour::Grey),
    ("", HudColour::White),
    (
        "Enter places it  1-9 into a slot  Esc closes  type to search",
        HudColour::White,
    ),
];

/// The route with the chooser: `` ` `` is the menu's while open (it shows the page, and on the
/// page hands the selected entry to placement mode) and passes while closed (its action opens
/// the page); `Delete` passes either way, so the remove key acts on the page and off it; a
/// digit on the page puts the selected entry in that slot.
#[test]
fn the_backquote_and_delete_route() {
    let bq = RouteInput::Key(KeyCode::Backquote);
    let del = RouteInput::Key(KeyCode::Delete);
    assert!(taken(bq, true).menu && !taken(bq, true).pass);
    assert!(taken(bq, false).pass && !taken(bq, false).menu);
    assert!(taken(del, true).pass && !taken(del, true).menu);
    let m = modes_with(library_abs("route"));
    let mut menu = Menu::default();
    menu.open_at(PageId::Keys);
    let mut input = Input::default();
    input.key_down(KeyCode::Backquote);
    let routed = step(&mut menu, &input, &m, &RunView::default());
    assert!(routed.acts.is_empty());
    assert_eq!(menu.page(), Some(PageId::Entities));
    let routed = step(&mut menu, &input, &m, &RunView::default());
    assert!(
        matches!(routed.acts[..], [Activated::Choose { entry: 0 }]),
        "{:?}",
        routed.acts
    );
    menu.open_entities(1);
    let mut input = Input::default();
    input.key_down(KeyCode::Digit7);
    let routed = step(&mut menu, &input, &m, &RunView::default());
    assert!(
        matches!(routed.acts[..], [Activated::Assign { entry: 2, slot: 6 }]),
        "{:?}",
        routed.acts
    );
    assert_eq!(menu.page(), Some(PageId::Entities));
    assert_eq!(menu.filter(), "");
}
