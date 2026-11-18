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
/// extent, and building it peaks under 300 MiB, read before a device opens: that peak is the
/// build's, and building every tile reads about 458 000 kB. The whole process's peak, which
/// is the frame's, is printed as a reading.
#[test]
fn castle_gate_golden() {
    use support::*;
    let (world, tiles) = gate_world();
    assert_eq!(tiles, 6);
    if let Some(peak) = peak_rss() {
        println!("castle_gate.png: build peak RSS {peak} B");
        assert!(peak < 300 << 20, "build peak RSS {peak} B");
    }
    let Some(img) = gate_image(&world, None) else {
        return;
    };
    assert_golden("castle_gate.png", &img);
    if let Some(peak) = peak_rss() {
        println!("castle_gate.png: process peak RSS {peak} B");
    }
}

/// The gate pose after 16 frames of history, the headlamp on and the torches lit, read back
/// as RGBA8; `None` without a software adapter. With `profile`, the frame renders at its
/// internal scale through FSR 2 and applies its `Live` and `Settle` fields, the exposure kept
/// at the golden's; its `Rebuild` fields do not reach the fixed mirror.
fn gate_image(world: &vxe_world::World, profile: Option<vxe_render::Profile>) -> Option<Vec<u8>> {
    use support::*;
    let mut rig = LookRig::new(world, None)?;
    if let Some(p) = profile {
        let mut s = vxe_render::RenderSettings::profile(p);
        s.lighting.exposure = vxe_render::Lighting::default().exposure;
        let output = [WIDTH, HEIGHT];
        rig.frame = vxe_render::Frame::with_sizes(&rig.device, s.scale.internal(output), output);
        rig.frame.set_upscaler_kind(
            &rig.device,
            vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
                sharpness: s.fsr2.sharpness,
                ..vxe_render::Fsr2Settings::default()
            }),
        );
        rig.frame.apply_settings(&rig.device, &rig.queue, &s);
    }
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
            .sync_now(world, &rig.device, &rig.queue)
            .expect("mirror fits the adapter");
    }
    // The store the cube faces read: the chunks around the eye's.
    rig.bake_around(world, eye, 1, 64);
    for _ in 0..16 {
        rig.frame.step(
            &rig.device,
            &rig.queue,
            &rig.gpu,
            &camera,
            &vxe_render::LightParams::headlamp(),
        );
    }
    let img = vxe_render::debug::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
    println!(
        "gate ({profile:?}): {} light samples, bins covering 1 %: {}",
        rig.gpu.light_samples(),
        bins_covering(&img, 0.01)
    );
    Some(img)
}

/// `profile_portable_gate.png`: the gate pose in the portable profile, held to the goldens'
/// floor (0.1 % of pixels over 2/255), and differing from the floor profile's image at the
/// same pose in more than that share, so the profile reached the frame. The gate holds the
/// torches and the lamp that portable's light cap and lamp bounce cut.
#[test]
#[ignore = "soak"]
fn portable_profile_golden() {
    use support::*;
    use vxe_render::Profile;
    let (world, _) = gate_world();
    let Some(img) = gate_image(&world, Some(Profile::Portable)) else {
        return;
    };
    assert_golden("profile_portable_gate.png", &img);
    let floor = gate_image(&world, Some(Profile::Floor)).expect("the adapter rendered once");
    let f = fraction_differing(&img, &floor, 2);
    println!(
        "profile_portable_gate.png: {:.4} % of pixels differ from the floor's by more than 2/255",
        f * 100.0
    );
    assert!(
        f > 0.001,
        "portable: {:.4} % differ from the floor",
        f * 100.0
    );
}
