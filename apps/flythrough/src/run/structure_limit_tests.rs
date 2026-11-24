//! The run's tests of a structure the scale refuses (Plan 0083 Phase 8, from window 83d): a box
//! over the world's edit limit, or a scale over 1 of the generated castle, places nothing, and
//! the window says why (the banner's row at the click, the belt's name row while the slot is
//! selected), not stdout only.

use std::sync::Arc;

use super::structure_tests::{
    EYE, click, enter_slot, frame_keys, look, notice, rock_box, run_in, stamps, structure,
    world_table,
};
use super::*;
use crate::entities::belt::{self, BELT_ROWS};
use crate::structures as st;
use svet_assets::VoxScene;
use svet_render::HudColour;
use svet_world::WorldLimits;

/// The belt drawn on an 80 x 45 grid at `now_ms`: its name row, the row's colour, and whether
/// a red span starts at `col` on it.
fn name_row(run: &Run, now_ms: f64, col: Option<usize>) -> (String, HudColour, bool) {
    let (mut text, mut colours, mut paint) = (String::new(), Vec::new(), belt::Paint::default());
    belt::append_belt(
        &mut text,
        &mut colours,
        (80, 45),
        &run.modes,
        now_ms,
        crate::hud::BANNER_MS,
        &mut paint,
    );
    let row = 45 - BELT_ROWS;
    let line = text.lines().nth(row).unwrap_or("").to_owned();
    let red = col.is_some_and(|c| {
        let at = line.len() - line.trim_start().len() + c;
        paint
            .spans
            .iter()
            .any(|(b, k)| *k == HudColour::Red && b.row as usize == row && b.col as usize == at)
    });
    (line.trim().to_owned(), colours[row], red)
}

/// A scene of no voxels whose box is `size` voxels across: a stand-in for the castle's.
fn empty_scene(size: [i32; 3]) -> VoxScene {
    VoxScene {
        prefabs: Vec::new(),
        placements: Vec::new(),
        layers: Vec::new(),
        bounds: ([0; 3], size.map(|v| v - 1)),
    }
}

/// Done-when of Plan 0083 Phase 8's 83d addition: in a world whose edits span at most 100
/// voxels an axis, the chapel (48 x 32 x 40, a 10-voxel margin each side) places at scale 1
/// and is refused at scale 2: the click submits no op; the notice and the banner's row over
/// the view read `chapel x2: 96 x 64 x 80 voxels, over the edit limit of 100`, and so does the
/// HUD's text; the belt's name row reads the same in grey with the reason in red, and stays
/// after the name row's banner time. The castle's box (600 x 600 x 500) is refused at scale 2
/// by the default limit of 1024 (`castle x2: 1200 x 1200 x 1000 voxels, ...`), and at scale 1
/// fits; with a limit that its box at scale 2 fits, the castle, which has no file to scale, is
/// still refused (`castle: scale 1 only`).
#[test]
fn a_structure_over_the_edit_limit_is_refused_on_the_hud() {
    let limits = WorldLimits {
        edit_axis_max: 100,
        ..WorldLimits::DEFAULT
    };
    let mut run = run_in(rock_box(&world_table()).with_limits(limits));
    look(&mut run, EYE, -40.0);
    enter_slot(&mut run, 2);
    let chapel = structure(&run, "chapel");
    assert_eq!(run.modes.walkers.belt.structure(), Some(chapel));
    assert_eq!(run.modes.edit_axis_max, 100);
    assert_eq!(st::words::refusal(&run.modes, chapel), None);

    run.modes.structure_scale = 2;
    let words = "chapel x2: 96 x 64 x 80 voxels, over the edit limit of 100";
    assert_eq!(
        st::words::refusal(&run.modes, chapel).as_deref(),
        Some(words)
    );
    frame_keys(&mut run, &click());
    run.tick_server();
    assert!(stamps(&run).is_empty());
    assert_eq!(run.net.as_ref().unwrap().server.ops().entries().len(), 0);
    assert_eq!(notice(&run), Some(words));
    let banner = run.banner().expect("a banner");
    assert!(banner.lines().any(|l| l == words), "{banner}");
    run.fill_hud(None);
    assert!(run.hud.text.lines().any(|l| l == words), "{}", run.hud.text);

    let late = run.modes.walkers.belt.changed_at.expect("a change") + 2.0 * crate::hud::BANNER_MS;
    let reason = words.find(": ").map(|c| c + 2);
    let (row, colour, red) = name_row(&run, late, reason);
    assert_eq!(row, words);
    assert_eq!(colour, HudColour::Grey);
    assert!(red, "the reason in red");
    run.modes.structure_scale = 1;
    assert_eq!(name_row(&run, late, None).0, "");

    let castle = structure(&run, st::CASTLE_NAME);
    let e = &mut run.modes.structures.entries[castle];
    e.source = Ok(st::Source::Castle(st::Castle::Built(Arc::new(
        empty_scene([600, 600, 500]),
    ))));
    let limit = WorldLimits::DEFAULT.edit_axis_max;
    assert_eq!(e.refusal(1, limit), None);
    assert_eq!(
        e.refusal(2, limit).as_deref(),
        Some("castle x2: 1200 x 1200 x 1000 voxels, over the edit limit of 1024")
    );
    assert_eq!(e.refusal(2, 4096).as_deref(), Some("castle: scale 1 only"));
}
