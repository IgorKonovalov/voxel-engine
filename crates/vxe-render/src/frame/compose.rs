//! The WGSL modules the passes are composed from, the prelude of constants Rust defines, and
//! the one list of every composed source ([`shader_sources`]).
//!
//! A pass's source is [`prelude`] then the modules it uses, joined by newlines: `common.wgsl`
//! (the views, `Params`, group 0, the pixel helpers), `mirror.wgsl` (the mirror's decode),
//! `walk.wgsl` (the voxel walk), `hitrec.wgsl` (the hit record and the store's taps),
//! `field_walk.wgsl` and `hit.wgsl` (the hit pass alone), `emissive.wgsl` (the emissive
//! estimator, after `light.wgsl`) and `edits.wgsl` (the edit boxes). A module's identifiers are
//! global to every pass that joins it, so a clash is a naga error, which
//! [`shader_sources`]'s validation finds headless.

use std::fmt::Write as _;

use super::settings::SPLIT_GROUP;
use super::*;
use crate::bake::{BRICK_WORDS, STORE_FLAGS, STORE_TABLE, TABLE_COARSE};
use crate::exposure::STATE_HELD;
use crate::field::{BAND_CUBES_BIT, CRISP_COUNT_BIT, SMOOTH_COUNT_BIT};
use crate::gpu_world::{
    ABSENT, CLASS_SHIFT, CLASS_UNIFORM, CLASS_WIDE, EMPTY_BIT, ENTRY_WORD, FULL_BIT,
    HEADER_BAND_BIT, HEADER_BAND_PREFIX_WORD, HEADER_BAND_WORD, HEADER_TRANSLUCENT_WORD, LOD_BIT,
    MATERIAL_MASK, RECORD_MASK, RECORD_WORDS, TRANSLUCENT_BIT, UNIT_MASK, UNIT_WORDS,
};
use crate::probes::{MOVED_BIT, TOUCHED_BIT};
use vxe_world::MaterialFlags;

pub(crate) const COMMON_WGSL: &str = include_str!("../shaders/common.wgsl");
pub(crate) const MIRROR_WGSL: &str = include_str!("../shaders/mirror.wgsl");
pub(crate) const WALK_WGSL: &str = include_str!("../shaders/walk.wgsl");
pub(crate) const HITREC_WGSL: &str = include_str!("../shaders/hitrec.wgsl");
pub(crate) const FIELD_WALK_WGSL: &str = include_str!("../shaders/field_walk.wgsl");
pub(crate) const HIT_WGSL: &str = include_str!("../shaders/hit.wgsl");
pub(crate) const EMISSIVE_WGSL: &str = include_str!("../shaders/emissive.wgsl");
pub(crate) const EDITS_WGSL: &str = include_str!("../shaders/edits.wgsl");

/// The hit pass's modules.
pub(crate) const HIT_MODULES: &[&str] = &[
    COMMON_WGSL,
    MIRROR_WGSL,
    WALK_WGSL,
    HITREC_WGSL,
    FIELD_WALK_WGSL,
    HIT_WGSL,
];

/// The constants the shaders share with Rust, by their WGSL names, every one a `u32`. Each is
/// written into [`prelude`] from its Rust definition, so no shader spells one as a literal.
/// The slot words are words of a slot from its first unit: the opaque line is unit 0, the
/// header unit 1 and the payload from unit 2.
pub(crate) const PRELUDE: &[(&str, u32)] = &[
    // The mirror's layout (`gpu_world`).
    ("ABSENT", ABSENT),
    ("FULL_BIT", FULL_BIT),
    ("LOD_BIT", LOD_BIT),
    ("EMPTY_BIT", EMPTY_BIT),
    ("RECORD_MASK", RECORD_MASK),
    ("TRANSLUCENT_BIT", TRANSLUCENT_BIT),
    ("MATERIAL_MASK", MATERIAL_MASK),
    ("UNIT_MASK", UNIT_MASK),
    ("CLASS_SHIFT", CLASS_SHIFT),
    ("CLASS_WIDE", CLASS_WIDE),
    ("CLASS_UNIFORM", CLASS_UNIFORM),
    ("UNIT_WORDS", UNIT_WORDS as u32),
    ("RECORD_WORDS", RECORD_WORDS as u32),
    ("ENTRY_WORD", ENTRY_WORD as u32),
    ("HEADER_WORD", UNIT_WORDS as u32),
    (
        "HEADER_TRANSLUCENT_WORD",
        (UNIT_WORDS + HEADER_TRANSLUCENT_WORD) as u32,
    ),
    ("PAYLOAD_WORD", 2 * UNIT_WORDS as u32),
    ("HEADER_BAND_WORD", HEADER_BAND_WORD as u32),
    ("HEADER_BAND_BIT", HEADER_BAND_BIT),
    ("HEADER_BAND_PREFIX_WORD", HEADER_BAND_PREFIX_WORD as u32),
    ("MATERIAL_TRANSLUCENT", MaterialFlags::TRANSLUCENT.0),
    ("MATERIAL_CRISP", MaterialFlags::CRISP.0),
    // The field's band bits (`field`).
    ("BAND_CUBES_BIT", BAND_CUBES_BIT),
    ("CRISP_COUNT_BIT", CRISP_COUNT_BIT as u32),
    ("SMOOTH_COUNT_BIT", SMOOTH_COUNT_BIT as u32),
    // `Params.flags` and `Params.lighting` (`frame/settings.rs`).
    ("MAX_VIEWS", MAX_VIEWS as u32),
    ("FLAG_SHADOWS", FLAG_SHADOWS),
    ("FLAG_MASKS", FLAG_MASKS),
    ("NORMAL_SHIFT", NORMAL_SHIFT),
    ("VIEW_SHIFT", VIEW_SHIFT),
    ("FLAG_SKIPS", FLAG_SKIPS),
    ("FLAG_CUTAWAY", FLAG_CUTAWAY),
    ("FLAG_FIELD_SPLIT", FLAG_FIELD_SPLIT),
    ("SPLIT_GROUP", SPLIT_GROUP),
    ("LIGHTING_LAMP", LIGHTING_LAMP),
    ("LIGHTING_COARSE_SHADOW", LIGHTING_COARSE_SHADOW),
    ("LIGHTING_GRADIENT", LIGHTING_GRADIENT),
    ("LIGHTING_FILTER_OFF", LIGHTING_FILTER_OFF),
    ("LIGHTING_IDENTITY_TONE_MAP", LIGHTING_IDENTITY_TONE_MAP),
    ("LIGHTING_EMISSION_CHANGED", LIGHTING_EMISSION_CHANGED),
    ("LIGHTING_LAMP_CHANGED", LIGHTING_LAMP_CHANGED),
    ("LIGHTING_INDIRECT", LIGHTING_INDIRECT),
    ("LIGHTING_AUTO_EXPOSURE", LIGHTING_AUTO_EXPOSURE),
    ("LIGHTING_STORE_BY_NORMAL", LIGHTING_STORE_BY_NORMAL),
    ("LIGHTING_FACE_WEIGHTS", LIGHTING_FACE_WEIGHTS),
    // The store (`bake`).
    ("STORE_FLAGS", STORE_FLAGS as u32),
    ("STORE_TABLE", STORE_TABLE as u32),
    ("BRICK_WORDS", BRICK_WORDS as u32),
    ("TABLE_COARSE", TABLE_COARSE),
    // The probe slots' flags (`probes`).
    ("TOUCHED_BIT", TOUCHED_BIT),
    ("MOVED_BIT", MOVED_BIT),
    // The exposure pass's modes and state flags (`exposure`).
    ("EX_AUTO", EX_AUTO),
    ("EX_METER", EX_METER),
    ("EX_RESET", EX_RESET),
    ("STATE_HELD", STATE_HELD),
];

/// [`PRELUDE`] as WGSL, one `const` a line.
pub(crate) fn prelude() -> String {
    let mut s = String::from("// The constants Rust defines (frame/compose.rs).\n");
    for (name, value) in PRELUDE {
        let _ = writeln!(s, "const {name}: u32 = {value}u;");
    }
    s
}

/// A pass's source: [`prelude`] then `modules`, joined by newlines.
pub(crate) fn compose(modules: &[&str]) -> String {
    let mut s = prelude();
    for m in modules {
        s.push('\n');
        s.push_str(m);
    }
    s
}

/// One compute pipeline's source as the renderer composes it.
pub struct ShaderSource {
    /// The pipeline's label.
    pub label: &'static str,
    /// The composed WGSL.
    pub source: String,
    /// The entry point.
    pub entry: &'static str,
}

/// Every compute source the renderer compiles: the hit pass's two entry points, each of
/// [`PASSES`], the composite's FSR 2 variant, FSR 2's passes (the accumulate pass in both its
/// variants), the bake, the field pass and the exposure pass, each composed as its pipeline
/// composes it. The HUD's and the present pass's render sources join no module and are not
/// listed.
pub fn shader_sources() -> Vec<ShaderSource> {
    let s = |label, source, entry| ShaderSource {
        label,
        source,
        entry,
    };
    let mut out = vec![
        s("hit", compose(HIT_MODULES), "main"),
        s("hit field resume", compose(HIT_MODULES), "field_resume"),
    ];
    out.extend(PASSES.iter().map(|p| s(p.label, p.source(), p.entry)));
    out.push(s("composite hdr", composite_hdr_compose(), "composite"));
    out.extend(
        crate::fsr2::FSR2_PIPES
            .iter()
            .map(|p| s(p.label, p.composed(), p.entry)),
    );
    out.push(s(
        "fsr2 accumulate (LDR)",
        crate::fsr2::fsr2_source(crate::fsr2::FSR2_ACCUMULATE_WGSL, false),
        "accumulate",
    ));
    out.push(s("bake", crate::bake::bake_source(), "bake_round"));
    out.push(s("field", crate::field::field_source(), "field_main"));
    out.push(s(
        "exposure",
        crate::exposure::exposure_source(),
        "exposure",
    ));
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::*;

    /// Every `.wgsl` file under `src/shaders/`, by its path from there, and its text.
    fn shader_files() -> BTreeMap<String, String> {
        fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, String>) {
            for e in std::fs::read_dir(dir).expect("shaders dir reads") {
                let p = e.expect("dir entry").path();
                if p.is_dir() {
                    walk(&p, root, out);
                } else if p.extension().is_some_and(|x| x == "wgsl") {
                    let rel = p.strip_prefix(root).expect("under root");
                    let text = std::fs::read_to_string(&p).expect("shader reads");
                    out.insert(rel.to_string_lossy().replace('\\', "/"), text);
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders");
        let mut out = BTreeMap::new();
        walk(&root, &root, &mut out);
        out
    }

    /// The functions defined in `text` that are not entry points (no stage attribute on the
    /// line before or on their own line).
    fn helper_fns(text: &str) -> Vec<String> {
        let lines: Vec<&str> = text.lines().collect();
        let mut out = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            let Some(rest) = l.strip_prefix("fn ") else {
                continue;
            };
            let stage = |s: &str| {
                s.contains("@compute") || s.contains("@vertex") || s.contains("@fragment")
            };
            if i > 0 && stage(lines[i - 1]) {
                continue;
            }
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            out.push(name);
        }
        out
    }

    fn count(source: &str, needle: &str) -> usize {
        source.lines().filter(|l| l.starts_with(needle)).count()
    }

    /// Every composed source parses and validates under naga.
    #[test]
    fn every_composed_source_validates() {
        let mut failed = Vec::new();
        for ShaderSource { label, source, .. } in shader_sources() {
            let module = match wgpu::naga::front::wgsl::parse_str(&source) {
                Ok(m) => m,
                Err(e) => {
                    failed.push(format!("{label}: {}", e.emit_to_string(&source)));
                    continue;
                }
            };
            if let Err(e) = wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::default(),
            )
            .validate(&module)
            {
                failed.push(format!("{label}: {e:?}"));
            }
        }
        assert!(failed.is_empty(), "{}", failed.join("\n"));
    }

    #[test]
    fn the_composed_sharpen_and_present_sources_hold_no_walk() {
        let sharpen = PASSES
            .iter()
            .find(|p| p.label == "sharpen")
            .expect("a sharpen pass")
            .source();
        for (label, source) in [
            ("sharpen", sharpen.as_str()),
            ("present", crate::present::PRESENT_WGSL),
        ] {
            assert_eq!(count(source, "fn descend("), 0, "{label} holds the walk");
        }
    }

    #[test]
    fn the_emissive_estimator_is_defined_once() {
        let files = shader_files();
        for needle in ["fn draw_candidates(", "struct Reservoir {"] {
            let homes: Vec<&String> = files
                .iter()
                .filter(|(_, t)| count(t, needle) > 0)
                .map(|(p, _)| p)
                .collect();
            assert_eq!(
                homes,
                ["emissive.wgsl"],
                "`{needle}` is defined only in emissive.wgsl"
            );
        }
        let light = PASSES
            .iter()
            .find(|p| p.label == "light")
            .expect("a light pass")
            .source();
        for (label, source) in [("light", light), ("bake", crate::bake::bake_source())] {
            for needle in ["fn draw_candidates(", "struct Reservoir {"] {
                assert_eq!(count(&source, needle), 1, "{label} holds `{needle}` once");
            }
        }
    }

    #[test]
    fn no_wgsl_function_is_defined_in_two_shader_files() {
        let mut homes: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (path, text) in shader_files() {
            for name in helper_fns(&text) {
                homes.entry(name).or_default().push(path.clone());
            }
        }
        let twice: Vec<_> = homes.into_iter().filter(|(_, h)| h.len() > 1).collect();
        assert!(twice.is_empty(), "defined in more than one file: {twice:?}");
    }

    #[test]
    fn no_constant_rust_defines_is_a_wgsl_literal() {
        for (path, text) in shader_files() {
            for (name, _) in PRELUDE {
                let decl = format!("const {name}:");
                assert!(
                    !text.lines().any(|l| l.trim_start().starts_with(&decl)),
                    "{path} writes {name} as a literal; the prelude defines it"
                );
            }
        }
        let names: std::collections::BTreeSet<_> = PRELUDE.iter().map(|(n, _)| n).collect();
        assert_eq!(names.len(), PRELUDE.len(), "each prelude name once");
    }

    #[test]
    fn the_prelude_holds_the_rust_values() {
        let p = prelude();
        assert!(p.contains("const FULL_BIT: u32 = 2147483648u;"));
        assert!(p.contains("const HEADER_TRANSLUCENT_WORD: u32 = 24u;"));
        assert!(p.contains("const PAYLOAD_WORD: u32 = 32u;"));
        assert!(p.contains("const FLAG_FIELD_SPLIT: u32 = 524288u;"));
        assert!(p.contains("const MATERIAL_TRANSLUCENT: u32 = 2u;"));
    }
}
