//! The Rust structs and constants the shaders mirror, compared against the WGSL as naga lays it
//! out: every uniform block (and each struct it nests) has the span and member offsets of the
//! Rust struct that fills it, and a constant a shader file spells as a literal under the name of
//! a Rust constant holds that constant's value.

use std::collections::{BTreeMap, BTreeSet};
use std::mem::{offset_of, size_of};
use std::path::Path;

use wgpu::naga::{self, TypeInner};

/// What fills a WGSL struct from Rust: its byte size and, by WGSL member name, the offset of
/// the Rust field that member reads. An empty `fields` is a block written as raw words (its
/// span is still compared); a member named `_` plus anything is padding and is not listed.
struct Mirror {
    rust: &'static str,
    size: usize,
    fields: Vec<(&'static str, usize)>,
}

/// `mirror!(Type { wgsl_member, wgsl_member = rust_field, .. })`.
macro_rules! mirror {
    ($t:ty { $($w:ident $(= $r:ident)?),* $(,)? }) => {
        Mirror {
            rust: stringify!($t),
            size: size_of::<$t>(),
            fields: vec![$((stringify!($w), mirror!(@off $t, $w $(, $r)?))),*],
        }
    };
    (@off $t:ty, $w:ident) => { offset_of!($t, $w) };
    (@off $t:ty, $w:ident, $r:ident) => { offset_of!($t, $r) };
}

/// The Rust side of WGSL struct `name` in the source labelled `label`, or `None` for a struct
/// no Rust struct fills.
fn mirror_of(label: &str, name: &str) -> Option<Mirror> {
    use crate::frame::{EditsUniform, Light, LightsUniform, UpscaleUniform};
    let raw = |rust, size| Mirror {
        rust,
        size,
        fields: Vec::new(),
    };
    Some(match (label, name) {
        ("field", "Params") => mirror!(crate::field::Params {
            window_min,
            units_per_shard,
            window_size,
            items,
            groups_x
        }),
        ("hud.wgsl", "Params") => mirror!(crate::hud::Params {
            scale,
            sample_count,
            ceiling_ms,
            line_px
        }),
        (_, "Params") => frame_params(),
        (_, "View") => mirror!(crate::camera::View {
            origin,
            tan_left,
            right,
            tan_right,
            up,
            tan_up,
            fwd,
            tan_down,
            rect,
            aspect,
            unit_scale,
            unit_offset,
            jitter
        }),
        (_, "Light") => mirror!(Light {
            pos,
            intensity,
            dir,
            cos_inner,
            colour,
            cos_outer,
            flags,
            floor = _pad
        }),
        (_, "Lights") => mirror!(LightsUniform {
            l,
            old,
            count,
            old_count,
            changed,
            lamp
        }),
        (_, "ExposureState") => mirror!(crate::exposure::ExposureState {
            current,
            previous,
            adapted_log2,
            metered_log2,
            frame,
            flags,
            ceiling
        }),
        (_, "ExUniform") => mirror!(crate::exposure::ExposureUniform {
            tiles,
            view_count,
            mode,
            rects,
            manual,
            strength,
            sigma,
            dt,
            key,
            min_e = min,
            max_e = max,
            tau_light,
            tau_cone,
            tau_rod,
            floor_log2,
            anchor
        }),
        (_, "ProbeParams") => mirror!(crate::probes::ProbeUniform {
            origin,
            spacing,
            updates,
            rays,
            steady,
            axis,
            reach,
            h_move
        }),
        (_, "SkyUniform") => mirror!(crate::sky::SkyUniform {
            rayleigh_scattering,
            bottom_radius = bottom_radius_km,
            mie_scattering,
            top_radius = top_radius_km,
            mie_extinction,
            rayleigh_scale_height = rayleigh_scale_height_km,
            ozone_absorption,
            mie_scale_height = mie_scale_height_km,
            ground_albedo,
            mie_g,
            sun_dir,
            sun_radius,
            sun_e,
            flags,
            moon_dir,
            moon_radius,
            moon_e,
            view_scale,
            uniform_l,
            altitude_zero_z,
            metres_per_voxel,
            base_altitude_m,
            camera_alt = camera_alt_km,
            sky_rays,
            key_dir,
            key_radius,
            key_e,
            sky_history,
            prev_key_dir,
            sky_scale,
            prev_key_e,
            prev_sky_scale,
            key_history
        }),
        (_, "Edits") => mirror!(EditsUniform { count, boxes }),
        (_, "UpTuning" | "InLamp") => mirror!(UpscaleUniform {
            width,
            alpha,
            alpha_moving,
            clip_gamma,
            w_mean,
            change_rel,
            change_abs
        }),
        (_, "Fsr2InputTuning") => mirror!(crate::fsr2::InputUniform {
            near,
            reactive_translucent,
            reactive_light
        }),
        (_, "Fsr2Constants") => fsr2_constants(),
        // Written as words: the a-trous strides (`[stride, last, 0, 0]`), the sharpen pass's
        // `[con, 0, 0, 0]` and the bake's job.
        (_, "Step") => raw("[u32; 4]", size_of::<[u32; 4]>()),
        (_, "Sharpen") => raw("[f32; 4]", size_of::<[f32; 4]>()),
        (_, "BakeJob") => raw("bake::JOB_BYTES", crate::bake::JOB_BYTES as usize),
        _ => return None,
    })
}

/// `Params` in `common.wgsl`.
fn frame_params() -> Mirror {
    use crate::frame::Params;
    // `lamp_pose` and `lamp` are the CPU's (WGSL pads them), and `far_normal` is WGSL's
    // `far_lo` and `far_hi`.
    let mut m = mirror!(Params {
        views,
        view_count,
        translucent_layers,
        frame,
        lighting,
        window_min,
        face_mix,
        window_size,
        size,
        light_floor,
        flags,
        units_per_shard,
        forced_level,
        lod1_r2,
        lod2_r2,
        exposure,
        candidates,
        light_table_base,
        history,
        prev_views,
        light_size,
        light_scale,
        light_offset,
        prev_light_offset,
        output_size,
        sky_bounce,
        dither
    });
    let far = offset_of!(Params, far_normal);
    m.fields.extend([("far_lo", far), ("far_hi", far + 4)]);
    let cam = offset_of!(Params, cam_chunk);
    m.fields.extend([
        ("cam_chunk_x", cam),
        ("cam_chunk_y", cam + 4),
        ("cam_chunk_z", cam + 8),
    ]);
    m
}

/// `Fsr2Constants` in `fsr2/common.wgsl`.
fn fsr2_constants() -> Mirror {
    mirror!(crate::fsr2::Fsr2Constants {
        render_size,
        max_render_size,
        display_size,
        input_color_size,
        luma_mip_size,
        luma_mip_level,
        frame_index,
        device_to_view,
        jitter,
        motion_vector_scale,
        downscale,
        motion_vector_jitter_cancellation,
        pre_exposure,
        previous_pre_exposure,
        tan_half_fov,
        jitter_sequence_length,
        delta_time,
        dynamic_res_change,
        view_space_to_meters,
        spd_mips,
        spd_work_groups,
        rcas_con,
        debug,
        tone
    })
}

/// Every source the renderer compiles, by label: the composed compute sources and the HUD's
/// and the present pass's render sources.
fn sources() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = crate::frame::shader_sources()
        .into_iter()
        .map(|s| (s.label.to_owned(), s.source))
        .collect();
    out.push(("hud.wgsl".to_owned(), crate::hud::HUD_WGSL.to_owned()));
    out.push((
        "present.wgsl".to_owned(),
        crate::present::PRESENT_WGSL.to_owned(),
    ));
    out
}

/// The struct types a uniform global of `module` reads, with the struct types they nest.
fn uniform_structs(module: &naga::Module) -> BTreeSet<naga::Handle<naga::Type>> {
    fn nest(
        module: &naga::Module,
        ty: naga::Handle<naga::Type>,
        out: &mut BTreeSet<naga::Handle<naga::Type>>,
    ) {
        match &module.types[ty].inner {
            TypeInner::Struct { members, .. } => {
                if out.insert(ty) {
                    for m in members {
                        nest(module, m.ty, out);
                    }
                }
            }
            TypeInner::Array { base, .. } => nest(module, *base, out),
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    for (_, g) in module.global_variables.iter() {
        if g.space == naga::AddressSpace::Uniform {
            nest(module, g.ty, &mut out);
        }
    }
    out
}

/// The layout mismatches of every uniform struct of `module` (from the source `label`).
fn layout_errors(label: &str, module: &naga::Module, errors: &mut BTreeSet<String>) {
    let mut layouter = naga::proc::Layouter::default();
    layouter
        .update(module.to_ctx())
        .unwrap_or_else(|e| panic!("{label}: layout: {e:?}"));
    for ty in uniform_structs(module) {
        let t = &module.types[ty];
        let name = t.name.as_deref().unwrap_or("?");
        let TypeInner::Struct { members, span } = &t.inner else {
            continue;
        };
        let Some(m) = mirror_of(label, name) else {
            errors.insert(format!(
                "{label}: uniform struct {name} has no Rust mirror listed"
            ));
            continue;
        };
        if layouter[ty].size != *span {
            errors.insert(format!(
                "{label}: {name}: naga's layouter disagrees with the span"
            ));
        }
        if *span as usize != m.size {
            errors.insert(format!(
                "{label}: {name} spans {span} bytes, {} is {}",
                m.rust, m.size
            ));
        }
        if m.fields.is_empty() {
            continue;
        }
        let rust: BTreeMap<&str, usize> = m.fields.iter().copied().collect();
        for member in members {
            let w = member.name.as_deref().unwrap_or("?");
            if w.starts_with('_') {
                continue;
            }
            match rust.get(w) {
                None => {
                    errors.insert(format!("{label}: {name}.{w} is not listed for {}", m.rust));
                }
                Some(&off) if off != member.offset as usize => {
                    errors.insert(format!(
                        "{label}: {name}.{w} is at {} in WGSL and {off} in {}",
                        member.offset, m.rust
                    ));
                }
                Some(_) => {}
            }
        }
        for w in rust.keys() {
            if !members.iter().any(|mm| mm.name.as_deref() == Some(w)) {
                errors.insert(format!("{label}: {name} has no member {w}"));
            }
        }
    }
}

#[test]
fn every_uniform_struct_has_the_layout_of_the_rust_struct_that_fills_it() {
    let mut errors = BTreeSet::new();
    for (label, source) in sources() {
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&source)));
        layout_errors(&label, &module, &mut errors);
    }
    assert!(
        errors.is_empty(),
        "{}",
        errors.into_iter().collect::<Vec<_>>().join("\n")
    );
}

/// Every file of `dir` with extension `ext`, recursively, by path from `dir`, and its text.
fn files(dir: &Path, ext: &str) -> BTreeMap<String, String> {
    fn walk(dir: &Path, root: &Path, ext: &str, out: &mut BTreeMap<String, String>) {
        for e in std::fs::read_dir(dir).expect("the directory reads") {
            let p = e.expect("a directory entry").path();
            if p.is_dir() {
                walk(&p, root, ext, out);
            } else if p.extension().is_some_and(|x| x == ext) {
                let rel = p.strip_prefix(root).expect("under the root");
                let text = std::fs::read_to_string(&p).expect("the file reads");
                out.insert(rel.to_string_lossy().replace('\\', "/"), text);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, ext, &mut out);
    out
}

/// The name of each `const NAME:` declared at the start of a line of `text`, with its
/// visibility.
fn const_names(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim_start();
            let l = l
                .strip_prefix("pub(crate) ")
                .or_else(|| l.strip_prefix("pub(super) "))
                .or_else(|| l.strip_prefix("pub "))
                .unwrap_or(l);
            let rest = l.strip_prefix("const ")?;
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            rest[name.len()..].starts_with(':').then_some(name)
        })
        .collect()
}

/// The Rust values of the constants a shader also declares as a literal outside the prelude.
/// A shader that spells a Rust constant must be listed here; better, it moves into the
/// prelude.
fn rust_value(name: &str) -> Option<f64> {
    Some(match name {
        "BAKE_LANES" => f64::from(crate::bake::BAKE_LANES),
        "BAKE_ROW" => f64::from(crate::bake::BAKE_ROW),
        "COLUMN_CELL" => f64::from(crate::gpu_world::COLUMN_CELL),
        "COLUMN_CELLS" => f64::from(crate::gpu_world::COLUMN_CELLS),
        "EMISSION_ROWS" => crate::frame::EMISSION_ROWS as f64,
        "FIRST" => f64::from(crate::font8x8::FIRST),
        "LAST" => f64::from(crate::font8x8::LAST),
        "GRADIENT_STRATUM" => f64::from(crate::frame::GRADIENT_STRATUM),
        "GRID_COLS" => f64::from(crate::hud::GRID_COLS),
        "GRID_ROWS" => f64::from(crate::hud::GRID_ROWS),
        _ => return None,
    })
}

#[test]
fn a_constant_named_on_both_sides_has_one_value() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let rust_files = files(&src, "rs");
    let rust: BTreeSet<String> = rust_files.values().flat_map(|t| const_names(t)).collect();
    let mut errors = Vec::new();
    // The prelude passes each same-named Rust constant through as it is.
    let compose = &rust_files["frame/compose.rs"];
    for (name, _) in crate::frame::PRELUDE {
        if !rust.contains(*name) {
            continue;
        }
        let at = compose
            .find(&format!("(\"{name}\","))
            .unwrap_or_else(|| panic!("the prelude's entry {name} is in compose.rs"));
        let rest = &compose[at + name.len() + 4..];
        let expr: String = rest[..rest.find(')').expect("the entry closes")]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if expr != *name && expr != format!("{name} as u32") {
            errors.push(format!(
                "the prelude writes {name} as `{expr}`, not the Rust constant {name}"
            ));
        }
    }
    // A literal in a shader file under a Rust constant's name holds that constant's value.
    let prelude: BTreeSet<&str> = crate::frame::PRELUDE.iter().map(|(n, _)| *n).collect();
    let mut seen = BTreeSet::new();
    for (label, source) in sources() {
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&source)));
        for (_, c) in module.constants.iter() {
            let Some(name) = c.name.as_deref() else {
                continue;
            };
            if !rust.contains(name) || prelude.contains(name) || !seen.insert(name.to_owned()) {
                continue;
            }
            let Some(want) = rust_value(name) else {
                errors.push(format!("{label} declares {name}, a Rust constant; list it"));
                continue;
            };
            let got = match module.global_expressions[c.init] {
                naga::Expression::Literal(naga::Literal::U32(v)) => f64::from(v),
                naga::Expression::Literal(naga::Literal::I32(v)) => f64::from(v),
                naga::Expression::Literal(naga::Literal::F32(v)) => f64::from(v),
                ref e => panic!("{label}: {name} is {e:?}, not a literal"),
            };
            if got != want {
                errors.push(format!("{label}: {name} is {got}, Rust's is {want}"));
            }
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
