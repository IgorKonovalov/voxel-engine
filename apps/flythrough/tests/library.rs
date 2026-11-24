//! The entities library with no device: the scan of a library directory (entries in name
//! order, an entry in error kept with its error, a directory with no rig a note), the
//! committed library built against the showcase's and the cave's tables, `--model`'s entry
//! first, and `flythrough --library`'s lines.

use std::path::{Path, PathBuf};

use flythrough::entities::library::{self, LINE_BYTES, Library, Source, scan};
use flythrough::viewer::Viewer;
use flythrough::{cave_materials, showcase_materials, walker};
use svet_world::MaterialTable;

fn committed() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities")
}

fn cave_table() -> MaterialTable {
    walker::with_walker(&cave_materials())
}

fn showcase_table() -> MaterialTable {
    walker::with_walker(&showcase_materials())
}

/// The committed walker's rig, its `.vox` line naming `<name>.vox`, with line `n` (1-based)
/// replaced by `line` when given.
fn walker_rig(name: &str, line: Option<(usize, &str)>) -> String {
    let text = std::fs::read_to_string(committed().join("walker/walker.rig")).expect("reads");
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let vox = lines
        .iter()
        .position(|l| l == "vox walker.vox")
        .expect("the vox line");
    lines[vox] = format!("vox {name}.vox");
    if let Some((n, l)) = line {
        lines[n - 1] = l.to_owned();
    }
    lines.join("\n") + "\n"
}

/// A library under the target's scratch directory: `a` valid, `b` broken on line 3, `c/`
/// with no rig, `d` naming a material no table has.
fn scratch_library(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("library-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    let entry = |name: &str, rig: Option<String>| {
        let d = dir.join(name);
        std::fs::create_dir_all(&d).expect("dir");
        std::fs::copy(
            committed().join("walker/walker.vox"),
            d.join(format!("{name}.vox")),
        )
        .expect("vox copied");
        if let Some(r) = rig {
            std::fs::write(d.join(format!("{name}.rig")), r).expect("rig written");
        }
    };
    entry(
        "d",
        Some(walker_rig("d", Some((8, "material 1 nosuchrow")))),
    );
    entry("b", Some(walker_rig("b", Some((3, "bogus 1 2 3")))));
    entry("a", Some(walker_rig("a", None)));
    entry("c", None);
    dir
}

fn names(lib: &Library) -> Vec<&str> {
    lib.entries.iter().map(|e| e.name.as_str()).collect()
}

/// Phase 1 done-when 1: `a`, `b` and `d` in name order; `a` builds, `b` carries `rig line 3`,
/// `d` an error quoting the material's name; `c` is no entry and a note names it.
#[test]
fn the_library_lists_each_rig_directory_in_name_order() {
    let mut lib = scan(&scratch_library("order"));
    assert_eq!(names(&lib), ["a", "b", "d"]);
    assert_eq!(lib.build(&cave_table()).len(), 1, "d's build fails");
    let errors: Vec<Option<&str>> = lib.entries.iter().map(|e| e.error()).collect();
    assert_eq!(errors[0], None, "a builds");
    let b = errors[1].expect("b is in error");
    assert!(b.starts_with("rig line 3: "), "{b}");
    let d = errors[2].expect("d is in error");
    assert!(d.contains("`nosuchrow`"), "{d}");
    assert!(
        lib.notes.iter().any(|n| n.contains("c/ holds no c.rig")),
        "{:?}",
        lib.notes
    );
    assert_eq!(lib.selected, 0);
}

/// Phase 1 done-when 2: every committed entry builds against the showcase's table, the
/// walker and the spider among them; under the cave's table, which lacks the showcase's
/// glowing rows, the spider is an entry in error naming `crystal` and the walker builds.
#[test]
fn the_committed_library_builds() {
    let mut lib = scan(&committed());
    assert!(lib.notes.is_empty(), "{:?}", lib.notes);
    let n = names(&lib);
    assert!(n.contains(&"walker") && n.contains(&"spider"), "{n:?}");
    assert_eq!(lib.build(&showcase_table()), Vec::<String>::new());
    for e in &lib.entries {
        assert_eq!(e.error(), None, "{}", e.name);
        let l = e.viewer().and_then(|v| v.loaded.as_ref()).expect("built");
        assert!(!l.model.parts().is_empty(), "{}", e.name);
    }

    let mut cave = scan(&committed());
    let errors = cave.build(&cave_table());
    let error = |name: &str| {
        cave.entries
            .iter()
            .find(|e| e.name == name)
            .and_then(|e| e.error())
            .map(str::to_owned)
    };
    let spider = error("spider").expect("the cave has no crystal row");
    assert!(spider.contains("unknown material `crystal`"), "{spider}");
    assert_eq!(error("walker"), None);
    assert_eq!(errors.len(), 1, "{errors:?}");
    // The spider sorts first, so the selection moves to the walker, which builds.
    assert_eq!(cave.entries[cave.selected].name, "walker");
}

/// Phase 1 done-when 3: `--model x.rig` over a library of `a` lists `x` then `a`, `x`
/// selected; a library that does not exist gives the built-in walker and a note.
#[test]
fn the_model_flag_comes_first() {
    let dir = scratch_library("model");
    for b in ["b", "c", "d"] {
        std::fs::remove_dir_all(dir.join(b)).expect("removed");
    }
    let x = dir.join("x.rig");
    std::fs::write(&x, walker_rig("a/a", None)).expect("x written");
    let lib = scan(&dir).with_model(Viewer::open(&x, None).expect("x opens"));
    assert_eq!(names(&lib), ["x", "a"]);
    assert_eq!(lib.selected, 0);
    assert!(lib.entries[0].flagged && !lib.entries[1].flagged);
    assert!(lib.flagged().is_some_and(|v| v.name == "x"));

    let none = scan(&dir.join("no-such-library"));
    assert_eq!(names(&none), [library::BUILTIN_NAME]);
    assert!(matches!(none.entries[0].source, Ok(Source::Builtin)));
    assert_eq!(none.notes.len(), 1);
    assert!(
        none.notes[0].contains("no-such-library"),
        "{:?}",
        none.notes
    );
}

/// Phase 1 done-when 4: `flythrough --library --library-dir DIR` parses to the early exit
/// and its text holds one line per entry, then the note, each at most 80 bytes: `a`'s with
/// its parts, size, bytes and clips, `b`'s and `d`'s with their errors.
#[test]
fn the_library_flag_prints_the_entries() {
    let dir = scratch_library("flag");
    let argv = ["--library", "--library-dir", dir.to_str().expect("utf-8")];
    let args = flythrough::cli::parse_args_from(argv.map(str::to_owned)).expect("parses");
    assert!(args.library);
    let text = library::library_text(&args);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    assert!(lines.iter().all(|l| l.len() <= LINE_BYTES), "{text}");
    assert!(lines[0].starts_with("a "), "{text}");
    assert!(
        lines[0].contains(" 6 parts") && lines[0].contains("clips idle* walk turn"),
        "{text}"
    );
    assert!(
        lines[1].starts_with("b ") && lines[1].contains("error: rig line 3"),
        "{text}"
    );
    assert!(
        lines[2].starts_with("d ") && lines[2].contains("nosuchrow"),
        "{text}"
    );
    assert!(lines[3].contains("c/ holds no c.rig"), "{text}");
}

/// Plan 0077 Phase 9 (4): `--model` naming the committed library's own walker rig (by a path
/// that canonicalises to the scan's) with the default library gives one `walker` entry, the
/// flagged one and first in `--library`'s lines, and one `spider` entry, no third.
#[test]
fn a_model_rig_from_the_library_is_one_entry() {
    let rig = committed().join("walker/../walker/walker.rig");
    let argv = ["--library", "--model", rig.to_str().expect("utf-8")];
    let args = flythrough::cli::parse_args_from(argv.map(str::to_owned)).expect("parses");
    let text = library::library_text(&args);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].starts_with("walker "), "{text}");
    assert!(lines[1].starts_with("spider "), "{text}");

    let lib = library::of_args(&args);
    assert_eq!(names(&lib), ["spider", "walker"], "the scan's places");
    assert!(lib.entries[1].flagged && !lib.entries[0].flagged);
    assert_eq!(lib.selected, 1);
    assert_eq!(lib.order(), [1, 0]);
    assert!(lib.flagged().is_some_and(|v| v.rig_path == rig));
}
