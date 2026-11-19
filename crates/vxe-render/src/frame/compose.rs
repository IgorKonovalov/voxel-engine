//! The WGSL modules the passes are composed from, the prelude of constants Rust defines, and
//! the one list of every composed source ([`shader_sources`]).
//!
//! A pass's source is [`prelude`] then the modules it uses, joined by newlines: `common.wgsl`
//! (the views, `Params`, group 0, the pixel helpers), `mirror.wgsl` (the mirror's decode),
//! `walk.wgsl` (the voxel walk), `hitrec.wgsl` (the hit record and the store's taps),
//! `entity.wgsl` (the entity walk), `field_walk.wgsl` and `hit.wgsl` (the hit pass alone), `emissive.wgsl` (the emissive
//! estimator, after `light.wgsl`), `sky_light.wgsl` and `shade.wgsl` (the light pass's sky
//! terms and its entry, after the sky's modules) and `edits.wgsl` (the edit boxes). A module's identifiers are
//! global to every pass that joins it, so a clash is a naga error, which
//! [`shader_sources`]'s validation finds headless.

use std::fmt::Write as _;

use super::settings::SPLIT_GROUP;
use super::*;
use crate::bake::{
    BRICK_WORDS, SKY_STALE, STORE_FLAGS, STORE_TABLE, TABLE_COARSE, VIS_BAKED, VIS_WORD,
};
use crate::exposure::STATE_HELD;
use crate::field::{BAND_CUBES_BIT, CRISP_COUNT_BIT, SMOOTH_COUNT_BIT};
use crate::gpu_world::{
    ABSENT, CLASS_SHIFT, CLASS_UNIFORM, CLASS_WIDE, EMPTY_BIT, ENTRY_WORD, FULL_BIT,
    HEADER_BAND_BIT, HEADER_BAND_PREFIX_WORD, HEADER_BAND_WORD, HEADER_TRANSLUCENT_WORD, LOD_BIT,
    MATERIAL_MASK, RECORD_MASK, RECORD_WORDS, TRANSLUCENT_BIT, UNIT_MASK, UNIT_WORDS,
};
use crate::probes::{MOVED_BIT, TOUCHED_BIT};
use crate::sky::{SKY_KEY, SKY_MULTISCATTER, SKY_ON, SKY_PREV_KEY, SKY_UNIFORM};
use vxe_world::MaterialFlags;

pub(crate) const COMMON_WGSL: &str = include_str!("../shaders/common.wgsl");
pub(crate) const MIRROR_WGSL: &str = include_str!("../shaders/mirror.wgsl");
pub(crate) const WALK_WGSL: &str = include_str!("../shaders/walk.wgsl");
pub(crate) const HITREC_WGSL: &str = include_str!("../shaders/hitrec.wgsl");
pub(crate) const FIELD_WALK_WGSL: &str = include_str!("../shaders/field_walk.wgsl");
pub(crate) const HIT_WGSL: &str = include_str!("../shaders/hit.wgsl");
pub(crate) const EMISSIVE_WGSL: &str = include_str!("../shaders/emissive.wgsl");
pub(crate) const EDITS_WGSL: &str = include_str!("../shaders/edits.wgsl");
pub(crate) const ENTITY_WGSL: &str = include_str!("../shaders/entity.wgsl");
pub(crate) const SKY_LIGHT_WGSL: &str = include_str!("../shaders/sky_light.wgsl");
pub(crate) const SHADE_WGSL: &str = include_str!("../shaders/shade.wgsl");

/// The hit pass's modules.
pub(crate) const HIT_MODULES: &[&str] = &[
    COMMON_WGSL,
    MIRROR_WGSL,
    WALK_WGSL,
    HITREC_WGSL,
    ENTITY_WGSL,
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
    ("HEADER_TRANSLUCENT_WORD", HEADER_TRANSLUCENT_WORD as u32),
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
    ("MAX_EDITS", MAX_EDITS as u32),
    ("FLAG_SHADOWS", FLAG_SHADOWS),
    ("FLAG_MASKS", FLAG_MASKS),
    ("NORMAL_SHIFT", NORMAL_SHIFT),
    ("VIEW_SHIFT", VIEW_SHIFT),
    ("FLAG_SKIPS", FLAG_SKIPS),
    ("FLAG_CUTAWAY", FLAG_CUTAWAY),
    ("FLAG_FIELD_SPLIT", FLAG_FIELD_SPLIT),
    ("SPLIT_GROUP", SPLIT_GROUP),
    ("LIGHTING_LAMP", LIGHTING_LAMP),
    ("LIGHTING_DIRECT", LIGHTING_DIRECT),
    ("LIGHTING_COARSE_SHADOW", LIGHTING_COARSE_SHADOW),
    ("LIGHTING_GRADIENT", LIGHTING_GRADIENT),
    ("LIGHTING_FILTER_OFF", LIGHTING_FILTER_OFF),
    ("LIGHTING_IDENTITY_TONE_MAP", LIGHTING_IDENTITY_TONE_MAP),
    ("LIGHTING_EMISSION_CHANGED", LIGHTING_EMISSION_CHANGED),
    ("LIGHTING_LIGHTS_CHANGED", LIGHTING_LIGHTS_CHANGED),
    ("LIGHTING_SKY_CHANGED", LIGHTING_SKY_CHANGED),
    ("LIGHTING_INDIRECT", LIGHTING_INDIRECT),
    ("LIGHTING_AUTO_EXPOSURE", LIGHTING_AUTO_EXPOSURE),
    ("LIGHTING_STORE_BY_NORMAL", LIGHTING_STORE_BY_NORMAL),
    ("LIGHTING_FACE_WEIGHTS", LIGHTING_FACE_WEIGHTS),
    ("LIGHTING_YOUNG_EDGES", LIGHTING_YOUNG_EDGES),
    ("LIGHTING_ENTITY_WORLD", LIGHTING_ENTITY_WORLD),
    ("LIGHTING_HOOK_KEY_OFF", LIGHTING_HOOK_KEY_OFF),
    ("LIGHTING_HOOK_ANY_HISTORY", LIGHTING_HOOK_ANY_HISTORY),
    ("LIGHTING_SHADOW_STYLES", LIGHTING_SHADOW_STYLES),
    ("LIGHTING_HOOK_NO_STAIR", LIGHTING_HOOK_NO_STAIR),
    ("LIGHTING_HOOK_SKY_RAY_ALWAYS", LIGHTING_HOOK_SKY_RAY_ALWAYS),
    ("LIGHTING_HOOK_NO_CLAMP", LIGHTING_HOOK_NO_CLAMP),
    ("LIGHTING_PENUMBRA_APPROACH", LIGHTING_PENUMBRA_APPROACH),
    ("LIGHTING_HOOK_SKY_END_IS_SKY", LIGHTING_HOOK_SKY_END_IS_SKY),
    ("LIGHTING_HOOK_KEY_NO_JITTER", LIGHTING_HOOK_KEY_NO_JITTER),
    ("LIGHTING_HOOK_NO_SETTLE", LIGHTING_HOOK_NO_SETTLE),
    ("LIGHTING_SKY_SMOOTHED", LIGHTING_SKY_SMOOTHED),
    // The list of lights (`frame/lights.rs`).
    ("MAX_LIGHTS", MAX_LIGHTS as u32),
    ("LIGHT_ON", Light::ON),
    ("LIGHT_SHADOWS", Light::SHADOWS),
    ("LIGHT_BOUNCE", Light::BOUNCE),
    // The entity mirror (`entities`).
    (
        "ENTITY_HEADER_WORD",
        crate::entities::TREE_HEADER_WORD as u32,
    ),
    ("ENTITY_NODE_WORDS", crate::entities::NODE_WORDS as u32),
    ("ENTITY_LEAF_BIT", crate::entities::LEAF_BIT),
    ("ENTITY_LEAF_MOVED", crate::entities::LEAF_MOVED),
    (
        "ENTITY_INSTANCE_WORD",
        crate::entities::INSTANCE_WORD as u32,
    ),
    (
        "ENTITY_INSTANCE_WORDS",
        crate::entities::INSTANCE_WORDS as u32,
    ),
    ("ENTITY_MOVED", crate::entities::INSTANCE_MOVED),
    // The store (`bake`).
    ("STORE_FLAGS", STORE_FLAGS as u32),
    ("STORE_TABLE", STORE_TABLE as u32),
    ("BRICK_WORDS", BRICK_WORDS as u32),
    ("TABLE_COARSE", TABLE_COARSE),
    ("VIS_WORD", VIS_WORD as u32),
    ("VIS_BAKED", VIS_BAKED),
    ("SKY_STALE", SKY_STALE),
    // The probe slots' flags (`probes`).
    ("TOUCHED_BIT", TOUCHED_BIT),
    ("MOVED_BIT", MOVED_BIT),
    // The exposure pass's modes and state flags (`exposure`).
    ("EX_AUTO", EX_AUTO),
    ("EX_METER", EX_METER),
    ("EX_RESET", EX_RESET),
    ("STATE_HELD", STATE_HELD),
    // The sky (`sky.rs`, `frame/sky.rs`): the uniform's flags and the LUTs' sizes.
    ("SKY_ON", SKY_ON),
    ("SKY_UNIFORM", SKY_UNIFORM),
    ("SKY_MULTISCATTER", SKY_MULTISCATTER),
    ("SKY_KEY", SKY_KEY),
    ("SKY_PREV_KEY", SKY_PREV_KEY),
    ("SKY_TRANSMITTANCE_W", super::sky::TRANSMITTANCE_SIZE[0]),
    ("SKY_TRANSMITTANCE_H", super::sky::TRANSMITTANCE_SIZE[1]),
    ("SKY_MULTISCATTER_W", super::sky::MULTISCATTER_SIZE[0]),
    ("SKY_MULTISCATTER_H", super::sky::MULTISCATTER_SIZE[1]),
    ("SKY_VIEW_W", super::sky::SKY_VIEW_SIZE[0]),
    ("SKY_VIEW_H", super::sky::SKY_VIEW_SIZE[1]),
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
    compose_with(modules, false)
}

/// [`compose`] keeping the walk's default-off arms: the lines from each `// arms:begin` to its
/// `// arms:end` (`walk.wgsl`'s approach probes and absorbing step), which [`compose`] leaves
/// out.
pub(crate) fn compose_arms(modules: &[&str]) -> String {
    compose_with(modules, true)
}

fn compose_with(modules: &[&str], arms: bool) -> String {
    let mut s = prelude();
    for m in modules {
        s.push('\n');
        if arms || !m.contains(ARMS_BEGIN) {
            s.push_str(m);
            continue;
        }
        let mut inside = false;
        for line in m.lines() {
            let t = line.trim_start();
            if t.starts_with(ARMS_BEGIN) {
                inside = true;
            } else if t.starts_with(ARMS_END) {
                inside = false;
            } else if !inside {
                s.push_str(line);
                s.push('\n');
            }
        }
    }
    s
}

/// The markers of a default-off arm's lines in a module ([`compose_arms`]).
const ARMS_BEGIN: &str = "// arms:begin";
const ARMS_END: &str = "// arms:end";

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
        SKY_PASSES
            .iter()
            .map(|(label, modules, entry)| s(label, compose(modules), entry)),
    );
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

    /// The walk's `floor_div` (`mirror.wgsl`) is an arithmetic shift by log2 of the divisor and
    /// `exit_cell`'s cell corner (`walk.wgsl`) a mask; for `v` in -4096..=4096 at every cell
    /// edge the walk uses (1 to 128) both equal `div_euclid` and the float floor division they
    /// replace (`floor(f32(v) / f32(size))`, times `size` for the corner).
    #[test]
    fn the_shift_and_mask_forms_equal_the_floor_division() {
        assert!(MIRROR_WGSL.contains("return a >> vec3<u32>(countTrailingZeros(u32(b)));"));
        assert!(WALK_WGSL.contains("let cell_min = v & vec3<i32>(-size);"));
        for size in [1i32, 2, 4, 8, 16, 32, 64, 128] {
            for v in -4096i32..=4096 {
                let shift = v >> size.trailing_zeros();
                let mask = v & -size;
                let float = (v as f32 / size as f32).floor() as i32;
                assert_eq!(shift, v.div_euclid(size), "{v} / {size}");
                assert_eq!(shift, float, "{v} / {size}");
                assert_eq!(mask, float * size, "{v} at {size}");
            }
        }
    }

    #[test]
    fn the_prelude_holds_the_rust_values() {
        let p = prelude();
        assert!(p.contains("const FULL_BIT: u32 = 2147483648u;"));
        assert!(p.contains("const HEADER_TRANSLUCENT_WORD: u32 = 8u;"));
        assert!(p.contains("const MAX_EDITS: u32 = 64u;"));
        assert!(p.contains("const PAYLOAD_WORD: u32 = 32u;"));
        assert!(p.contains("const FLAG_FIELD_SPLIT: u32 = 524288u;"));
        assert!(p.contains("const MATERIAL_TRANSLUCENT: u32 = 2u;"));
    }

    /// The light pass's default pipelines compile neither default-off arm of the walk (the
    /// approach probes' call, the absorbing step's sum): their sources lack both, which the
    /// arms' variants, the steps view and the bake keep; the markers pair up in `walk.wgsl`.
    #[test]
    fn the_default_light_pipelines_compile_no_arm() {
        use super::super::pipelines::{
            P_LIGHT, P_LIGHT_ARMS, P_LIGHT_BARE, P_LIGHT_BARE_ARMS, P_LIGHT_STEPS,
        };
        let arms = ["approach_cell(o, d, t, exit.t)", "absorb_tau += 0.001"];
        for p in [P_LIGHT, P_LIGHT_BARE] {
            let src = PASSES[p].source();
            for a in arms {
                assert!(!src.contains(a), "{} compiles {a}", PASSES[p].label);
            }
        }
        for p in [P_LIGHT_ARMS, P_LIGHT_BARE_ARMS, P_LIGHT_STEPS] {
            let src = PASSES[p].source();
            for a in arms {
                assert!(src.contains(a), "{} lacks {a}", PASSES[p].label);
            }
        }
        let bake = crate::bake::bake_source();
        assert!(bake.contains(arms[1]), "the bake lacks the absorbing step");
        let (b, e) = (
            WALK_WGSL.matches(ARMS_BEGIN).count(),
            WALK_WGSL.matches(ARMS_END).count(),
        );
        assert_eq!((b, e), (2, 2), "walk.wgsl's arms markers");
    }
}
