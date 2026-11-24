//! Plan 0083 Phase 2: the fly-through's material file reloaded with no device.

use flythrough::materials::{CASTLE_ROWS, RESTART_WORDS, Reload, Selection, load, reload};
use flythrough::setup::CrispRows;

/// The castle's file text with `edit` applied to the line starting `from`.
fn edited(text: &str, from: &str, to: &str) -> String {
    let n = text.matches(from).count();
    assert_eq!(n, 1, "`{from}` occurs once");
    text.replacen(from, to, 1)
}

/// Done-when 3: a file with rock's albedo changed yields a rewrite of the same row count that
/// marks the rock row changed; a file with a row added, or with the glass row's
/// `translucent` flag removed, yields no rewrite and the banner's `restart` words.
#[test]
fn a_field_change_reloads_and_a_shape_change_waits() {
    let (file, _) = load(None).expect("loads");
    let text = std::fs::read_to_string(flythrough::materials::DEFAULT_PATH).unwrap();
    let selection = Selection::cave_and(&CASTLE_ROWS, &[], true, CrispRows::None);
    // The running table: the castle scene's rows, then rows the start added after them.
    let mut running = selection.compose(&file).unwrap();
    running.push(svet_world::Material::named("walker"));

    let r = reload(&file, &selection, &running, &text);
    let Reload::Rewrite { changed, .. } = &r else {
        panic!("{r:?}")
    };
    assert!(changed.is_empty(), "an unchanged file changes nothing");

    let rock = edited(
        &text,
        "material rock\n  albedo 0.42 0.38 0.34",
        "material rock\n  albedo 0.9 0.1 0.1",
    );
    let r = reload(&file, &selection, &running, &rock);
    let Reload::Rewrite {
        table,
        changed,
        emission,
    } = &r
    else {
        panic!("{r:?}")
    };
    assert_eq!(table.len(), running.len());
    assert_eq!(changed, &[flythrough::cave::ROCK]);
    assert!(emission.is_empty());
    assert_eq!(table.get(1).unwrap().albedo, [0.9, 0.1, 0.1]);
    assert_eq!(table.get(running.len() as u16 - 1).unwrap().name, "walker");
    assert_eq!(r.words(), "materials reloaded");

    let torch = edited(&text, "emission 3.0 1.7 0.6", "emission 6.0 1.7 0.6");
    let r = reload(&file, &selection, &running, &torch);
    let Reload::Rewrite {
        changed, emission, ..
    } = &r
    else {
        panic!("{r:?}")
    };
    let torch_id = 4 + 7;
    assert_eq!(
        (changed.as_slice(), emission.as_slice()),
        (&[torch_id][..], &[torch_id][..])
    );

    let added = format!("{text}material moss\n  albedo 0.1 0.5 0.1\n");
    assert_eq!(reload(&file, &selection, &running, &added), Reload::Restart);
    let clear = edited(&text, "  flags translucent\n", "");
    let r = reload(&file, &selection, &running, &clear);
    assert_eq!(r, Reload::Restart);
    assert_eq!(r.words(), RESTART_WORDS);
    assert!(r.words().contains("restart"));
    // An emitting row that stops emitting changes its class too.
    let dark = edited(&text, "emission 3.0 1.7 0.6", "emission 0 0 0");
    assert_eq!(reload(&file, &selection, &running, &dark), Reload::Restart);
    // A file that does not parse: its error and line.
    let bad = edited(&text, "  albedo 0.42 0.38 0.34", "  color #12");
    let r = reload(&file, &selection, &running, &bad);
    assert!(
        matches!(&r, Reload::Error(e) if e.contains("line") && e.contains("6 hex digits")),
        "{r:?}"
    );
}
