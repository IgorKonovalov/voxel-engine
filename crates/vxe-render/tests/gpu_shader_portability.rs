//! Shader constructs that validate under naga and run on Vulkan but fail to compile on one of
//! the backends we ship, caught headless on any host.

use vxe_render::wgpu::naga::{self, Block, Expression, Statement, TypeInner};

/// Every shader the renderer compiles: the composed compute sources from the renderer's own
/// list (`internals::shader_sources`, built from the pass tables and the bake's composer), and
/// every `.wgsl` file under `src/shaders/` (subdirectories included) whose text no composed
/// source holds, read alone (the HUD's and the present pass's render sources). A new module
/// that no pass composes is read alone too, and fails to parse without its prelude, so no
/// shader file is missed.
fn shaders() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vxe_render::internals::shader_sources()
        .into_iter()
        .map(|s| (format!("{} ({})", s.label, s.entry), s.source))
        .collect();
    let mut files = Vec::new();
    shader_files(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders"),
        &mut files,
    );
    assert!(
        files.len() > 20,
        "the walk finds the shader files: {files:?}"
    );
    let mut alone = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path).expect("shader reads");
        if !out.iter().any(|(_, source)| source.contains(text.trim())) {
            alone.push((path.display().to_string(), text));
        }
    }
    out.extend(alone);
    out
}

/// Every `.wgsl` file under `dir`, recursively, in a fixed order.
fn shader_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("shaders dir reads")
        .map(|e| e.expect("dir entry").path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            shader_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "wgsl") {
            out.push(p);
        }
    }
}

/// The files read alone are exactly the two render sources that join no module: every other
/// shader file is held by a composed source.
#[test]
fn every_shader_file_is_composed_or_read_alone() {
    let composed = vxe_render::internals::shader_sources().len();
    let alone: Vec<String> = shaders()
        .into_iter()
        .skip(composed)
        .map(|(name, _)| {
            std::path::Path::new(&name)
                .file_name()
                .expect("a file name")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(alone, ["hud.wgsl", "present.wgsl"]);
}

/// FXC, the D3D12 HLSL compiler, rejects a store to a vector component chosen at run time
/// (`v[k] = x`) with "array reference cannot be used as an l-value; not natively
/// addressable". Every shader writes such a component through a whole-vector `select`.
#[test]
fn no_shader_stores_through_a_runtime_vector_index() {
    let mut offenders = Vec::new();
    for (name, source) in shaders() {
        let module =
            naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{name}: {e}"));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let functions = module
            .functions
            .iter()
            .map(|(handle, f)| (f, &info[handle]))
            .chain(
                module
                    .entry_points
                    .iter()
                    .enumerate()
                    .map(|(i, ep)| (&ep.function, info.get_entry_point(i))),
            );
        for (function, function_info) in functions {
            let mut stores = Vec::new();
            collect_stores(&function.body, &mut stores);
            for pointer in stores {
                let Expression::Access { base, .. } = function.expressions[pointer] else {
                    continue;
                };
                let is_vector = match *function_info[base].ty.inner_with(&module.types) {
                    TypeInner::Pointer { base, .. } => {
                        matches!(module.types[base].inner, TypeInner::Vector { .. })
                    }
                    TypeInner::ValuePointer { size, .. } => size.is_some(),
                    _ => false,
                };
                if is_vector {
                    offenders.push(format!(
                        "{name}: fn {}",
                        function.name.as_deref().unwrap_or("?")
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "runtime-indexed vector component stores: {offenders:?}"
    );
}

fn collect_stores(block: &Block, out: &mut Vec<naga::Handle<Expression>>) {
    for statement in block.iter() {
        match statement {
            Statement::Store { pointer, .. } => out.push(*pointer),
            Statement::Block(inner) => collect_stores(inner, out),
            Statement::If { accept, reject, .. } => {
                collect_stores(accept, out);
                collect_stores(reject, out);
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    collect_stores(&case.body, out);
                }
            }
            Statement::Loop {
                body, continuing, ..
            } => {
                collect_stores(body, out);
                collect_stores(continuing, out);
            }
            _ => {}
        }
    }
}
