//! The menu's entries: one view over [`TOGGLES`] and [`ACTIONS`], their words and binds, the
//! tree `--keys` prints, and the source scan that holds the tables to the keys the code reads.

use super::{FIXED_KEYS, FIXED_ROWS, Group, RunView, key_name, pad_name};
use crate::hud::Modes;
use crate::toggles::{ACTIONS, Action, Bind, Press, TOGGLES, Toggle};
use std::fmt::Write as _;
use vxe_app::KeyCode;

/// What an entry is: a toggle's row or an action's.
#[derive(Clone, Copy)]
pub enum EntryKind {
    Toggle(&'static Toggle),
    Action(&'static Action),
}

/// One entry of the menu: its label, its page and the table row behind it.
#[derive(Clone, Copy)]
pub struct Entry {
    pub label: &'static str,
    pub group: Group,
    pub kind: EntryKind,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.label, self.group.name())
    }
}

impl Entry {
    /// The entry's mode in words: a toggle's `get`, an action's mode, or none for an action
    /// with no mode.
    pub fn value(&self, m: &Modes, v: &RunView) -> Option<String> {
        match self.kind {
            EntryKind::Toggle(t) => Some((t.get)(m)),
            EntryKind::Action(a) => a.mode.map(|f| f(m, v)),
        }
    }

    /// The words the entry's row shows: its mode in words when they begin with its label,
    /// else `label: words`; the label alone for an action with no mode.
    pub fn words(&self, m: &Modes, v: &RunView) -> String {
        match self.value(m, v) {
            Some(w) if w.starts_with(self.label) => w,
            Some(w) => format!("{}: {w}", self.label),
            None => self.label.to_owned(),
        }
    }

    /// The entry's binds in words, `F7`, `X / pad West`, `wheel / pad D-pad up down`; empty
    /// for a menu-only entry.
    pub fn keys(&self) -> String {
        let names: Vec<String> = match self.kind {
            EntryKind::Toggle(t) => match t.press {
                Press::Key(k, pad) => std::iter::once(key_name(k))
                    .chain(pad.map(pad_name))
                    .collect(),
                Press::Scroll => vec!["wheel".to_owned(), "pad D-pad up down".to_owned()],
                Press::Menu => Vec::new(),
            },
            EntryKind::Action(a) => a
                .binds
                .iter()
                .map(|b| match *b {
                    Bind::Key(k) => key_name(k),
                    Bind::Pad(p) => pad_name(p),
                })
                .collect(),
        };
        names.join(" / ")
    }
}

/// Every toggle and action as an entry, in the tree's order: the groups in [`Group::ALL`]'s
/// order, each its toggles in [`TOGGLES`]' order and then its actions in [`ACTIONS`]'.
pub fn entries() -> impl Iterator<Item = Entry> {
    Group::ALL.into_iter().flat_map(group_entries)
}

/// The entries of `group`, in the tree's order.
pub fn group_entries(group: Group) -> impl Iterator<Item = Entry> {
    let toggles = TOGGLES
        .iter()
        .filter(move |t| t.group == group)
        .map(|t| Entry {
            label: t.name,
            group: t.group,
            kind: EntryKind::Toggle(t),
        });
    let actions = ACTIONS
        .iter()
        .filter(move |a| a.group == group)
        .map(|a| Entry {
            label: a.label,
            group: a.group,
            kind: EntryKind::Action(a),
        });
    toggles.chain(actions)
}

/// The tree `--keys` prints at `m` and `v`: the fixed keys (what they do, then the keys in
/// the key column), then a header per group and a row per entry, its label, its mode in words (`-` for none) and its binds (`menu` for
/// none).
pub fn write_tree(m: &Modes, v: &RunView, out: &mut String) {
    let _ = writeln!(out, "fixed");
    for (keys, does) in FIXED_ROWS {
        let _ = writeln!(out, "  {does:<67} {keys}");
    }
    for g in Group::ALL {
        let _ = writeln!(out, "{}", g.name());
        for e in group_entries(g) {
            let value = e.value(m, v).unwrap_or_else(|| "-".to_owned());
            let keys = e.keys();
            let keys = if keys.is_empty() { "menu" } else { &keys };
            let _ = writeln!(out, "  {:<22} {value:<44} {keys}", e.label);
        }
    }
}

/// `flythrough --keys`' text: the tree at the modes and run flags `args` start with.
pub fn keys_text(args: &crate::cli::Args) -> String {
    let m = args.start_modes(0.0);
    let v = RunView {
        walking: args.walk,
        audio: args.audio.on.then(|| {
            let mix = args.audio.mix;
            super::AudioView {
                echoes: true,
                tail: true,
                echo_db: mix.echo_db,
                tail_db: mix.tail_db,
                predelay_ms: mix.predelay_ms,
                glide: mix.glide,
                direct_delay: mix.direct_delay,
            }
        }),
    };
    let mut out = String::new();
    write_tree(&m, &v, &mut out);
    out
}

/// Every key the tables and the fixed set bind.
pub fn bound_keys() -> Vec<KeyCode> {
    let toggles = TOGGLES.iter().filter_map(|t| match t.press {
        Press::Key(k, _) => Some(k),
        Press::Scroll | Press::Menu => None,
    });
    let actions = ACTIONS
        .iter()
        .flat_map(|a| a.binds)
        .filter_map(|b| match b {
            Bind::Key(k) => Some(*k),
            Bind::Pad(_) => None,
        });
    toggles
        .chain(actions)
        .chain(FIXED_KEYS.iter().copied())
        .collect()
}

/// The `Debug` names of [`bound_keys`].
pub fn bound_key_names() -> Vec<String> {
    bound_keys().into_iter().map(|k| format!("{k:?}")).collect()
}

/// The marker lines around a stretch of source whose `KeyCode`s are the open menu's text and
/// navigation, not binds (the palette's typed keys, the arrows); [`unlisted_keys`] skips the
/// lines between them.
pub const TYPED_BEGIN: &str = "keys:typed-begin";
pub const TYPED_END: &str = "keys:typed-end";

/// The `KeyCode::<Name>` tokens in `source` whose name no table binds, in the order read; a
/// plain token scan that skips the lines between [`TYPED_BEGIN`] and [`TYPED_END`].
pub fn unlisted_keys(source: &str) -> Vec<String> {
    let bound = bound_key_names();
    let mut out = Vec::new();
    let mut typed = false;
    for line in source.lines() {
        if line.contains(TYPED_BEGIN) {
            typed = true;
        } else if line.contains(TYPED_END) {
            typed = false;
        }
        if typed {
            continue;
        }
        for (i, _) in line.match_indices("KeyCode::") {
            let name: String = line[i + "KeyCode::".len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if name.starts_with(|c: char| c.is_ascii_uppercase()) && !bound.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}
