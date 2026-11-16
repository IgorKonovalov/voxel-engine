//! The fly-through's menu (`flythrough::hud::menu`): its entries over the key tables, the
//! source scan that holds the tables to the keys the code reads, and the tree `--keys`
//! prints. CPU only, no device.

use flythrough::cli::parse_args_from;
use flythrough::hud::menu::{Group, entries, keys_text, unlisted_keys};
use flythrough::toggles::{ACTIONS, TOGGLES};

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}

/// The entries' `(label, group)` pairs cover every `TOGGLES` row and every `ACTIONS` row once
/// each, and no entry has an empty label or a group the root page does not list.
#[test]
fn every_toggle_and_action_is_an_entry() {
    let pairs: Vec<(&str, Group)> = entries().map(|e| (e.label, e.group)).collect();
    let mut want: Vec<(&str, Group)> = TOGGLES.iter().map(|t| (t.name, t.group)).collect();
    want.extend(ACTIONS.iter().map(|a| (a.label, a.group)));
    assert_eq!(pairs.len(), want.len(), "{pairs:?}");
    for w in &want {
        let n = pairs.iter().filter(|p| *p == w).count();
        assert_eq!(n, 1, "{w:?} is an entry {n} times");
    }
    for (label, group) in &pairs {
        assert!(!label.is_empty(), "an entry of {group:?} has no label");
        assert!(Group::ALL.contains(group), "{label}: {group:?}");
    }
}

/// Every `KeyCode::<Name>` token in the `.rs` files under `apps/flythrough/src/` names a key
/// `TOGGLES`, `ACTIONS` or `FIXED_KEYS` binds; and the scan bites: fed a read of `ArrowUp`,
/// which nothing binds, it reports `ArrowUp`.
#[test]
fn the_source_reads_no_key_the_tables_miss() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).expect("the source directory reads") {
            let p = e.expect("an entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(files.len() > 20, "{files:?}");
    for f in &files {
        let text = std::fs::read_to_string(f).expect("the file reads");
        let missed = unlisted_keys(&text);
        assert!(missed.is_empty(), "{}: {missed:?}", f.display());
    }
    assert_eq!(
        unlisted_keys("if input.pressed(KeyCode::ArrowUp) {}"),
        ["ArrowUp"]
    );
    assert_eq!(
        unlisted_keys("// keys:typed-begin\nKeyCode::ArrowUp,\n// keys:typed-end\n"),
        Vec::<String>::new()
    );
}

/// The `--keys` tree's row for `label` under the group header `group`.
fn tree_row<'a>(tree: &'a str, group: &str, label: &str) -> &'a str {
    let mut in_group = false;
    for line in tree.lines() {
        if !line.starts_with(' ') {
            in_group = line == group;
        } else if in_group && line[2..].split("  ").next() == Some(label) {
            return line;
        }
    }
    panic!("no {label} under {group}:\n{tree}");
}

/// `--keys` at the start modes prints `lamp on` under `lighting` with `F7` in its key column;
/// with `--lamp off` it prints `lamp off`. Every group's header is a line of its own.
#[test]
fn the_tree_prints_live_values() {
    let start = parse_args_from(argv(&["--keys"])).expect("--keys parses");
    assert!(start.keys);
    let tree = keys_text(&start);
    let row = tree_row(&tree, "lighting", "lamp");
    let cols: Vec<&str> = row.split("  ").filter(|c| !c.trim().is_empty()).collect();
    assert_eq!(
        cols.iter().map(|c| c.trim()).collect::<Vec<_>>(),
        ["lamp", "lamp on", "F7"],
        "{row}"
    );
    for g in Group::ALL {
        assert!(tree.lines().any(|l| l == g.name()), "{g:?}:\n{tree}");
    }
    let off = keys_text(&parse_args_from(argv(&["--keys", "--lamp", "off"])).unwrap());
    let row = tree_row(&off, "lighting", "lamp");
    assert!(row.contains(" lamp off "), "{row}");
    assert!(row.trim_end().ends_with(" F7"), "{row}");
}

/// `flythrough --keys` exits before a device: the parser sets `keys` (which `main` reads
/// beside `--help`, before the window or a device), and a run without it does not.
#[test]
fn keys_is_an_early_exit_beside_help() {
    let a = parse_args_from(argv(&["--keys"])).unwrap();
    assert!(a.keys && !a.help);
    assert!(!parse_args_from(argv(&[])).unwrap().keys);
    let main = include_str!("../src/main.rs");
    let keys_at = main.find("if args.keys").expect("main reads --keys");
    let device_at = main.find("setup::flythrough").expect("main starts the app");
    assert!(keys_at < device_at, "--keys exits before the app starts");
}
