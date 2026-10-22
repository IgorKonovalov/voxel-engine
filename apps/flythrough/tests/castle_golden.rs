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

/// `castle_gate.png`: the spawn pose facing the gate, the headlamp on and the torches lit,
/// after 16 frames of history, on lavapipe. The world holds only the 6 tiles that meet its
/// extent, and the process peaks under 1 GB.
#[test]
fn castle_gate_golden() {
    use support::*;
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
