//! Shader constructs that validate under naga and run on Vulkan but fail to compile on one of
//! the backends we ship, caught headless on any host.

use vxe_render::wgpu::naga::{self, Block, Expression, Statement, TypeInner};

const SHADERS: [(&str, &str); 4] = [
    ("march.wgsl", include_str!("../src/shaders/march.wgsl")),
    ("attrib.wgsl", include_str!("../src/shaders/attrib.wgsl")),
    ("present.wgsl", include_str!("../src/shaders/present.wgsl")),
    ("hud.wgsl", include_str!("../src/shaders/hud.wgsl")),
];

/// FXC, the D3D12 HLSL compiler, rejects a store to a vector component chosen at run time
/// (`v[k] = x`) with "array reference cannot be used as an l-value; not natively
/// addressable". Every shader writes such a component through a whole-vector `select`.
#[test]
fn no_shader_stores_through_a_runtime_vector_index() {
    let mut offenders = Vec::new();
    for (name, source) in SHADERS {
        let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{name}: {e}"));
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
