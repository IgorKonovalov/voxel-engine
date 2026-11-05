//! `castle_gate.png`: the castle's gate golden on the software adapter (lavapipe), from a
//! world built from only the castle tiles that meet its view. `VXE_BLESS=1` rewrites it.

mod support;

use flythrough::castle;
use vxe_world::VoxelCoord;

/// The gate golden's world: chunks x -1..4, y -4..5, z -1..4 around the spawn's view, built
/// from only the castle tiles that meet them.
fn gate_world() -> (vxe_world::World, usize) {
    let extent = vxe_world::ChunkExtent::new([-1, -4, -1], [4, 5, 4]);
    let (lo, hi) = (extent.min.0.map(|c| c * 64), extent.max.0.map(|c| c * 64));
    let file = castle::castle_vox_where(4, castle::tiles_meeting(lo, hi));
    let tiles = castle::tiles()
        .iter()
        .filter(|t| castle::tiles_meeting(lo, hi)(t))
        .count();
    let src = castle::castle_source(
        1,
        castle::castle_prefabs(&file).expect("every index is mapped"),
    );
    drop(file);
    let table =
        flythrough::castle_materials(&flythrough::with_emissive_ore(&flythrough::cave_materials()));
    (vxe_world::World::generate(extent, &src, &table), tiles)
}

/// The process's peak resident set in bytes from `/proc/self/status`, where there is one.
fn peak_rss() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: u64 = s
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(kb * 1024)
}

/// Set in the child [`warm_the_shader_cache`] spawns: it renders the warm-up and returns.
const WARM_UP: &str = "VXE_CASTLE_WARM_UP";

/// Renders a small world in a child process of this test binary, so Mesa's disk cache holds the
/// frame's shaders before this process opens a device. Lavapipe compiles a shader at its first
/// dispatch and a process keeps the compiler's heap and LLVM's pages for the rest of its life,
/// so only another process keeps them out of this one's peak.
fn warm_the_shader_cache() {
    let exe = std::env::current_exe().expect("the test binary's path");
    let status = std::process::Command::new(exe)
        .args(["--exact", "castle_gate_golden"])
        .env(WARM_UP, "1")
        .stdout(std::process::Stdio::null())
        .status()
        .expect("the warm-up process starts");
    assert!(status.success(), "the warm-up render: {status}");
}

/// `castle_gate.png`: the spawn pose facing the gate, the headlamp on and the torches lit,
/// after 16 frames of history, on lavapipe. The world holds only the 6 tiles that meet its
/// extent, and the process peaks under 1 GB with the shader compiler's share kept out by
/// [`warm_the_shader_cache`]. With Mesa's shader cache disabled there is nothing to warm, the
/// compile lands in this process and the bar fails.
#[test]
fn castle_gate_golden() {
    use support::*;
    if std::env::var_os(WARM_UP).is_some() {
        let world = pillar_world();
        if let Some(mut rig) = LookRig::new(&world, None) {
            let camera = camera_looking_x([8.0, 8.0, 8.0]);
            for _ in 0..2 {
                rig.frame.step(
                    &rig.device,
                    &rig.queue,
                    &rig.gpu,
                    &camera,
                    &vxe_render::LightParams::headlamp(),
                );
            }
            vxe_render::readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
        }
        return;
    }
    if peak_rss().is_some() {
        warm_the_shader_cache();
    }
    let (world, tiles) = gate_world();
    assert_eq!(tiles, 6);
    let Some(mut rig) = LookRig::new(&world, None) else {
        return;
    };
    let (eye, yaw, pitch) = castle::CastleShot::Gate.pose();
    let camera = vxe_render::Camera {
        position: eye.map(|v| v as f32 + 0.5).into(),
        yaw: (yaw as f32).to_radians(),
        pitch: (pitch as f32).to_radians(),
        vfov: 70f32.to_radians(),
    };
    rig.gpu.set_focus(VoxelCoord(eye));
    for _ in 0..4 {
        rig.gpu
            .sync_now(&world, &rig.device, &rig.queue)
            .expect("mirror fits the adapter");
    }
    for _ in 0..16 {
        rig.frame.step(
            &rig.device,
            &rig.queue,
            &rig.gpu,
            &camera,
            &vxe_render::LightParams::headlamp(),
        );
    }
    let img = vxe_render::readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
    println!(
        "castle_gate.png: {} light samples, bins covering 1 %: {}",
        rig.gpu.light_samples(),
        bins_covering(&img, 0.01)
    );
    assert_golden("castle_gate.png", &img);
    if let Some(peak) = peak_rss() {
        println!("castle_gate.png: peak RSS {peak} B");
        assert!(peak < 1 << 30, "peak RSS {peak} B");
    }
}
