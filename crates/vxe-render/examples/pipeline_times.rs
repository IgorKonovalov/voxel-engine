//! Times the creation of every compute pipeline the renderer builds, one row per pass, on the
//! adapter the environment picks.
//!
//! The instance and adapter honour `WGPU_BACKEND`, `WGPU_ADAPTER_NAME` and
//! `WGPU_DX12_COMPILER`, so one binary reads Vulkan on Linux and DX12 under FXC or DXC on
//! Windows. Each pass is a shader module plus a compute pipeline with an automatic layout,
//! built from the same concatenated source and entry point the renderer uses; the timed span
//! runs from `create_shader_module` through `create_compute_pipeline` and a `device.poll`
//! wait. Each pass is created three times on one fresh device: `cold` is the first creation,
//! `min` the fastest of the three (a driver-side pipeline cache can make the later two cheap).
//!
//! `--hlsl <dir>` also writes naga's HLSL of each pass for Shader Model 5.1 (what FXC
//! compiles) and 6.0 (what DXC compiles) as `<pass>.sm51.hlsl` and `<pass>.sm60.hlsl`, and
//! `entries.txt` with each pass's HLSL entry point name, so the Microsoft compilers can be
//! timed outside wgpu.
//!
//! ```text
//! cargo run -p vxe-render --release --example pipeline_times [-- --hlsl <dir>]
//! ```

use std::time::Instant;

use vxe_render::wgpu::{self, naga};
use vxe_render::{
    ATROUS_WGSL, ATTRIB_WGSL, COMPOSITE_WGSL, GRADIENT_WGSL, LIGHT_WGSL, MARCH_WGSL, SHARPEN_WGSL,
    TEMPORAL_WGSL, UPSCALE_WGSL,
};

/// One compute pipeline the renderer creates: its source as compiled and its entry point.
struct Pass {
    name: &'static str,
    source: String,
    entry: &'static str,
}

/// Every compute pass, each source concatenated exactly as the renderer does it. The list
/// mirrors `AttribPass::new` in `attrib.rs` and `Marcher::new` and `Frame::with_sizes` in
/// `marcher.rs`; a pass added there belongs here too.
fn passes() -> Vec<Pass> {
    let after_march = |rest: &[&str]| {
        let mut s = MARCH_WGSL.to_owned();
        for part in rest {
            s.push('\n');
            s.push_str(part);
        }
        s
    };
    let pass = |name, source, entry| Pass {
        name,
        source,
        entry,
    };
    vec![
        pass("attrib", ATTRIB_WGSL.to_owned(), "main"),
        pass("march", MARCH_WGSL.to_owned(), "main"),
        pass("light", after_march(&[LIGHT_WGSL]), "shade"),
        pass("temporal", after_march(&[TEMPORAL_WGSL]), "temporal"),
        pass(
            "gradient",
            after_march(&[LIGHT_WGSL, GRADIENT_WGSL]),
            "gradient",
        ),
        pass("downsample", after_march(&[COMPOSITE_WGSL]), "downsample"),
        pass("variance", after_march(&[ATROUS_WGSL]), "variance"),
        pass("atrous", after_march(&[ATROUS_WGSL]), "atrous"),
        pass("upscale", after_march(&[UPSCALE_WGSL]), "upscale"),
        pass("sharpen", after_march(&[SHARPEN_WGSL]), "sharpen_main"),
        pass("composite", after_march(&[COMPOSITE_WGSL]), "composite"),
    ]
}

/// Creations per pass on the one device.
const REPEATS: usize = 3;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut hlsl_dir = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--hlsl" => hlsl_dir = Some(args.next().expect("--hlsl takes a directory")),
            other => panic!("unknown argument {other:?}; the only one is --hlsl <dir>"),
        }
    }
    let passes = passes();
    if let Some(dir) = &hlsl_dir {
        write_hlsl(&passes, std::path::Path::new(dir));
    }

    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
        &instance, None,
    ))
    .expect("no adapter");
    let info = adapter.get_info();
    let compiler = std::env::var("WGPU_DX12_COMPILER").unwrap_or_else(|_| "auto".to_owned());
    let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: vxe_render::limits_for(&adapter, 16 << 20),
        ..Default::default()
    }))
    .expect("no device");

    println!(
        "adapter: {} ({:?}), WGPU_DX12_COMPILER={compiler}",
        info.name, info.backend
    );
    println!(
        "{:<11} {:>6} {:>10} {:>10}",
        "pass", "lines", "cold ms", "min ms"
    );
    let (mut cold_sum, mut min_sum) = (0.0, 0.0);
    for pass in &passes {
        let mut times = [0.0f64; REPEATS];
        for t in &mut times {
            *t = create_once(&device, pass);
        }
        let min = times.iter().copied().fold(f64::INFINITY, f64::min);
        cold_sum += times[0];
        min_sum += min;
        println!(
            "{:<11} {:>6} {:>10.1} {:>10.1}",
            pass.name,
            pass.source.lines().count(),
            times[0],
            min
        );
    }
    println!(
        "{:<11} {:>6} {:>10.1} {:>10.1}",
        "total", "", cold_sum, min_sum
    );
}

/// Creates `pass`'s module and pipeline once and returns the milliseconds it took, panicking
/// with wgpu's message if either is invalid.
#[allow(
    clippy::disallowed_methods,
    reason = "the example measures pipeline creation in wall time; it is no simulation code"
)]
fn create_once(device: &wgpu::Device, pass: &Pass) -> f64 {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let start = Instant::now();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(pass.name),
        source: wgpu::ShaderSource::Wgsl(pass.source.as_str().into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(pass.name),
        layout: None,
        module: &module,
        entry_point: Some(pass.entry),
        compilation_options: Default::default(),
        cache: None,
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("device poll");
    let ms = start.elapsed().as_secs_f64() * 1e3;
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("{}: {e}", pass.name);
    }
    drop(pipeline);
    ms
}

/// Writes each pass's HLSL for Shader Models 5.1 and 6.0 into `dir`, and `entries.txt` with
/// one `<pass> <hlsl entry point>` line per pass.
fn write_hlsl(passes: &[Pass], dir: &std::path::Path) {
    std::fs::create_dir_all(dir).expect("create the --hlsl directory");
    let mut entries = String::new();
    for pass in passes {
        let module = naga::front::wgsl::parse_str(&pass.source)
            .unwrap_or_else(|e| panic!("{}: {}", pass.name, e.emit_to_string(&pass.source)));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}: {e:?}", pass.name));
        let pipeline = naga::back::hlsl::PipelineOptions {
            entry_point: Some((naga::ShaderStage::Compute, pass.entry.to_owned())),
        };
        let mut entry_name = None;
        for (model, tag) in [
            (naga::back::hlsl::ShaderModel::V5_1, "sm51"),
            (naga::back::hlsl::ShaderModel::V6_0, "sm60"),
        ] {
            let options = naga::back::hlsl::Options {
                shader_model: model,
                ..Default::default()
            };
            let mut out = String::new();
            let reflection = naga::back::hlsl::Writer::new(&mut out, &options, &pipeline)
                .write(&module, &info, None)
                .unwrap_or_else(|e| panic!("{} {tag}: {e}", pass.name));
            entry_name = reflection
                .entry_point_names
                .into_iter()
                .find_map(Result::ok);
            std::fs::write(dir.join(format!("{}.{tag}.hlsl", pass.name)), out)
                .expect("write the HLSL");
        }
        let entry_name = entry_name.unwrap_or_else(|| panic!("{}: no entry point", pass.name));
        entries.push_str(&format!("{} {entry_name}\n", pass.name));
    }
    std::fs::write(dir.join("entries.txt"), entries).expect("write entries.txt");
}
