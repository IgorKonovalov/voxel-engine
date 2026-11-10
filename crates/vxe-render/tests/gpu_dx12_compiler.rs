//! The DX12 backend compiles shaders with the statically linked DXC, never FXC (ADR-0035).
//!
//! `wgpu-hal` names the compiler it picked at debug level when it makes the DX12 instance.
//! The test captures that line from an instance made with the default options, so the build's
//! `static-dxc` feature and `Dx12Compiler::Auto` decide, with no environment variable read.
//! The feature links DXC only on x86_64 and i686 Windows MSVC, so the test exists only there.
#![cfg(all(windows, target_env = "msvc", not(target_arch = "aarch64")))]

use std::sync::Mutex;

use vxe_render::wgpu;

/// Every message `wgpu_hal` logged while the test ran.
static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target().starts_with("wgpu_hal")
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            LINES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

static CAPTURE: Capture = Capture;

#[test]
fn the_dx12_instance_compiles_with_static_dxc() {
    log::set_logger(&CAPTURE).expect("the only logger in this binary");
    log::set_max_level(log::LevelFilter::Debug);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    drop(instance);
    let lines = LINES.lock().unwrap_or_else(|e| e.into_inner());
    let picked: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("Using ") && l.ends_with(" for shader compilation"))
        .collect();
    assert_eq!(
        picked,
        ["Using static DXC for shader compilation"],
        "wgpu_hal logged: {lines:?}"
    );
}
