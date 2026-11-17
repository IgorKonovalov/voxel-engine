//! Plan 0077 Phase 3 with no run: the entities page as the menu writes it from the library,
//! and the keys the menu takes for it.

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
/// (the committed spider) under the target's scratch directory, built against the showcase's
/// table with the walker's rows.
fn library_abs() -> library::Library {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("picker-rows");
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

/// Phase 3 done-when: `a`, `b` (in error) and `spider`, `spider` selected on its second
/// clip: the page's text and colours are the fixture's, `b`'s row grey with its error after
/// its name, the clip row a cycle naming `idle (2/3)`, and no row past 80 columns or 45 rows.
#[test]
fn the_picker_draws_its_rows() {
    let mut lib = library_abs();
    lib.selected = 2;
    let v = lib.entries[2].viewer_mut().expect("a rig");
    assert_eq!(v.cycle(1), "clip idle");
    let m = modes_with(lib);
    let mut menu = Menu::default();
    menu.open_entities(m.walkers.library.selected);
    let (mut text, mut colours) = (String::new(), Vec::new());
    menu.write_with(&m, &RunView::default(), None, &mut text, &mut colours);
    let rows: Vec<&str> = text.lines().collect();
    assert!(rows.len() <= MENU_ROWS && rows.iter().all(|r| r.len() <= MENU_COLS));
    let want: Vec<(&str, HudColour)> = FIXTURE.to_vec();
    let have: Vec<(&str, HudColour)> = rows.iter().copied().zip(colours.iter().copied()).collect();
    assert_eq!(have, want, "{text}");
}

/// The page of `the_picker_draws_its_rows`: the title, the three entries (`b` grey with its
/// error, `spider` selected), the clip row, the `entities` group's entries (the clip and
/// remove entries grey with no entity placed), a blank notice row and the footer.
const FIXTURE: [(&str, HudColour); 13] = [
    ("menu  >  entities", HudColour::White),
    ("  a           6 parts  0.6 x 0.3 x 1.5 m", HudColour::White),
    (
        "  b          rig line 3: unknown statement `bogus`",
        HudColour::Grey,
    ),
    (
        "> spider     14 parts  6.9 x 4.0 x 2.3 m",
        HudColour::Yellow,
    ),
    ("  clip  idle  (2/3)", HudColour::White),
    (
        "  remove aimed entity  needs a placed entity                Delete",
        HudColour::Grey,
    ),
    ("  walkers walk off", HudColour::White),
    ("  eye lights on", HudColour::White),
    (
        "  clip before  needs a placed entity                        Left",
        HudColour::Grey,
    ),
    (
        "  clip after  needs a placed entity                         Right",
        HudColour::Grey,
    ),
    (
        "  clip pause  needs a placed entity                         Down",
        HudColour::Grey,
    ),
    ("", HudColour::White),
    (
        "Enter places  ` places, closes  Delete removes  Esc closes",
        HudColour::White,
    ),
];

/// The route with the entities page: `` ` `` is the menu's while open (it shows the page, and
/// on the page places and closes) and passes while closed (its action opens the page);
/// `Delete` passes either way, so the remove key acts on the page and off it.
#[test]
fn the_backquote_and_delete_route() {
    let bq = RouteInput::Key(KeyCode::Backquote);
    let del = RouteInput::Key(KeyCode::Delete);
    assert!(taken(bq, true).menu && !taken(bq, true).pass);
    assert!(taken(bq, false).pass && !taken(bq, false).menu);
    assert!(taken(del, true).pass && !taken(del, true).menu);
    let m = modes_with(library_abs());
    let mut menu = Menu::default();
    menu.open_at(PageId::Keys);
    let mut input = Input::default();
    input.key_down(KeyCode::Backquote);
    let routed = step(&mut menu, &input, &m, &RunView::default());
    assert!(routed.acts.is_empty());
    assert_eq!(menu.page(), Some(PageId::Entities));
    let routed = step(&mut menu, &input, &m, &RunView::default());
    assert!(
        matches!(
            routed.acts[..],
            [Activated::Place {
                entry: 0,
                close: true
            }]
        ),
        "{:?}",
        routed.acts
    );
}
