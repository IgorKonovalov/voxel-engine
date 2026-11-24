//! The render settings' tests: the fields, the profiles and the apply classes.

use super::*;

/// Every leaf of the pretty `Debug` text with no stops.
fn all_leaves(s: &RenderSettings) -> Vec<String> {
    debug_leaves(&format!("{s:#?}"), &[])
        .into_iter()
        .map(|(n, _)| n)
        .collect()
}

/// The field names `FIELDS` lists are the leaves of `RenderSettings`'s `Debug` output,
/// each once: every leaf is a field or lies under exactly one (a struct-like enum
/// variant's fields, such as the bake budget's `ms`, or the residency's radii), and every
/// field holds a leaf. Read on the default settings and on settings whose enums take their
/// other variants.
#[test]
fn the_fields_are_the_leaves_of_the_debug_output_each_once() {
    let names: Vec<&str> = RenderSettings::FIELDS.iter().map(|&(n, _)| n).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "a field is listed twice");
    let mut other = RenderSettings::default();
    other.bake.budget = Budget::PerSecond {
        ms_per_s: 2.0,
        cap: 1.0,
    };
    other.metering = Metering::Average;
    other.lighting.exposure = crate::exposure::Exposure::Auto(Default::default());
    other.output_hint = Some([1280, 720]);
    other.build.sync_budget_time = Some(std::time::Duration::from_millis(3));
    for s in [RenderSettings::default(), other] {
        let leaves = all_leaves(&s);
        for leaf in &leaves {
            let under: Vec<&&str> = names
                .iter()
                .filter(|n| leaf == *n || leaf.starts_with(&format!("{n}.")))
                .collect();
            assert_eq!(under.len(), 1, "leaf {leaf} lies under {under:?}");
        }
        for n in &names {
            assert!(
                leaves
                    .iter()
                    .any(|l| l == n || l.starts_with(&format!("{n}."))),
                "field {n} holds no leaf of {leaves:?}"
            );
        }
        assert_eq!(s.table().len(), names.len());
        assert!(s.table().iter().all(|(_, _, v)| !v.is_empty()));
    }
}

/// The default settings are the floor's, and the other profiles differ from it in the
/// rows of `RenderSettings::profile`'s table alone.
#[test]
fn the_default_is_the_floor_and_the_profiles_differ_in_their_rows() {
    let floor = RenderSettings::profile(Profile::Floor);
    assert_eq!(RenderSettings::default(), floor);
    assert_eq!(
        floor.changed_fields(&RenderSettings::profile(Profile::Portable)),
        [
            "lighting.lamp_bounce",
            "look.translucent_layers",
            "lod.r_full",
            "light_cap",
            "bake.budget",
            "bake.tiers",
            "bake.drain",
            "build.mirror_budget_bytes",
            "build.residency",
            "output_hint",
        ]
    );
    assert_eq!(
        floor.changed_fields(&RenderSettings::profile(Profile::Performance)),
        ["scale", "output_hint"]
    );
    assert_eq!(
        floor.changed_fields(&RenderSettings::profile(Profile::High)),
        ["scale"]
    );
    for p in Profile::ALL {
        assert_eq!(Profile::from_name(p.name()), Some(p));
    }
}

/// Each profile's values equal the owner's picks field by field, and it differs from the
/// floor in no field outside them.
#[test]
fn the_profiles_hold_the_owners_picks() {
    assert_eq!(
        Profile::ALL,
        [
            Profile::Floor,
            Profile::Portable,
            Profile::Performance,
            Profile::High
        ]
    );
    let floor = RenderSettings::profile(Profile::Floor);
    for (i, p) in Profile::ALL.into_iter().enumerate() {
        let s = RenderSettings::profile(p);
        let table = s.table();
        for (field, values) in RenderSettings::PROFILE_PICKS {
            let (_, _, v) = table
                .iter()
                .find(|(n, _, _)| n == field)
                .unwrap_or_else(|| panic!("no row {field}"));
            assert_eq!(v, values[i], "{} {field}", p.name());
        }
        for f in floor.changed_fields(&s) {
            assert!(
                RenderSettings::PROFILE_PICKS.iter().any(|(n, _)| *n == f),
                "{} differs from the floor in {f}, not a pick",
                p.name()
            );
        }
    }
}

/// Plan 0098 Phase 4 (ADR-0082, window W2's picks): the library's lighting by default takes the
/// five picks, no light history, the key light's far from the column map's horizon stepped in
/// at the band's half and the disc penumbra, with the arms not picked off (the landing's two
/// fades, the far's wide light, the far's light history); and every named profile holds the
/// five, the far normal's band 4..8 among them.
#[test]
fn every_profile_holds_the_calm_picks() {
    use crate::frame::{KeyFar, KeyFarMix, Penumbra};
    let picks = |l: &Lighting| (l.light_history, l.key_far, l.key_far_mix, l.penumbra);
    let calm = (false, KeyFar::Column, KeyFarMix::Step, Penumbra::Disc);
    let l = Lighting::default();
    assert_eq!(picks(&l), calm);
    assert_eq!(
        (
            l.landing_fade,
            l.source_fade,
            l.far_light_wide,
            l.light_history_far
        ),
        (false, false, false, false)
    );
    assert_eq!(DEFAULT_FAR_NORMAL, [4.0, 8.0]);
    for p in Profile::ALL {
        let s = RenderSettings::profile(p);
        assert_eq!(picks(&s.lighting), calm, "{}", p.name());
        assert_eq!(s.look.far_normal, DEFAULT_FAR_NORMAL, "{}", p.name());
    }
}

/// Each field's test value changes that field alone.
#[test]
fn each_test_value_changes_its_field_alone() {
    let base = RenderSettings::default();
    for &(name, _) in RenderSettings::FIELDS {
        let s = base.with_test_value(name).expect("a test value");
        assert_eq!(base.changed_fields(&s), [name]);
    }
    assert!(base.with_test_value("no.such").is_none());
}

/// The `pub fn set_` names in the crate's source files, read at test time.
fn setters_in_source() -> Vec<String> {
    fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).expect("a source dir") {
            let p = e.expect("an entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&p).expect("a source file");
                for (i, _) in text.match_indices("pub fn set_") {
                    let name: String = text[i + "pub fn ".len()..]
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    // The search's own literal ends at `set_`.
                    if name.len() > "set_".len() {
                        out.push(name);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut out,
    );
    out.sort_unstable();
    out.dedup();
    out
}

/// Every `pub fn set_` of the crate is the setter of a field or listed as not a setting;
/// every listed setter exists, and each field it names is a row of `FIELDS`.
#[test]
fn every_setter_is_a_setting_or_an_input() {
    let found = setters_in_source();
    assert!(
        found.len() > 30,
        "only {} setters found: {found:?}",
        found.len()
    );
    let listed: Vec<&str> = RenderSettings::SETTERS
        .iter()
        .map(|&(n, _)| n)
        .chain(RenderSettings::NOT_SETTINGS.iter().map(|&(n, _)| n))
        .collect();
    for setter in &found {
        assert!(
            listed.contains(&setter.as_str()),
            "pub fn {setter} is neither a setting's setter (RenderSettings::SETTERS) nor \
             listed in RenderSettings::NOT_SETTINGS"
        );
    }
    for n in &listed {
        assert!(
            found.iter().any(|f| f == n),
            "{n} is listed but not in the source"
        );
    }
    for &(setter, fields) in RenderSettings::SETTERS {
        for f in fields {
            assert!(
                RenderSettings::FIELDS
                    .iter()
                    .any(|(n, _)| n == f || (f.ends_with('.') && n.starts_with(f))),
                "{setter} names {f}, no field"
            );
        }
    }
}

/// A changed leaf is named by its field, an enum's variant change too; equal settings
/// change nothing.
#[test]
fn a_changed_value_names_its_field() {
    let a = RenderSettings::default();
    assert!(a.changed_fields(&a.clone()).is_empty());
    let mut b = a.clone();
    b.look.translucent_layers = 2;
    b.bake.budget = Budget::PerSecond {
        ms_per_s: 2.0,
        cap: 1.0,
    };
    b.build.residency.r_full = 12;
    assert_eq!(
        a.changed_fields(&b),
        ["look.translucent_layers", "bake.budget", "build.residency"]
    );
    let row = |s: &RenderSettings, n: &str| {
        s.table()
            .into_iter()
            .find(|(f, _, _)| *f == n)
            .expect("a row")
            .2
    };
    assert_eq!(row(&b, "look.translucent_layers"), "2");
    assert_eq!(
        row(&b, "bake.budget"),
        "PerSecond { ms_per_s: 2.0, cap: 1.0 }"
    );
    assert_eq!(row(&a, "output_hint"), "Some([1920, 1080])");
}
