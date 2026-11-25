//! The render settings as one value (ADR-0055): a frame's own settings applied back change
//! nothing, a `Settle` field applies on the next frame and restarts the history once, a `Live`
//! field keeps it, and a `Rebuild` field waits for the caller.

use svet_render::{
    ApplyClass, Camera, Frame, GpuWorld, LightParams, RenderScale, RenderSettings, UpscalerChoice,
    glam::Vec3, wgpu,
};
use svet_world::*;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    svet_render::test_support::any_device(16 << 20, wgpu::Features::empty())
}

/// A hand-built box: one chunk, rock walls one brick thick around air, a rock pillar in the
/// middle, mirrored.
fn mirrored(device: &wgpu::Device, queue: &wgpu::Queue) -> GpuWorld {
    struct BoxRoom;
    impl WorldSource for BoxRoom {
        fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
            let base = coord.min_voxel().0;
            for slot in 0..CHUNK_BRICKS {
                let b = brick_of_slot(slot);
                out.set_brick(
                    slot,
                    Brick::from_fn(|l| {
                        let v: [i32; 3] =
                            std::array::from_fn(|k| base[k] + b[k] * BRICK_EDGE + l[k]);
                        let wall = v.iter().any(|&c| !(4..60).contains(&c));
                        let pillar = (28..36).contains(&v[0]) && (28..36).contains(&v[1]);
                        if wall || pillar { 1 } else { AIR }
                    }),
                );
            }
        }
    }
    let mut t = MaterialTable::new();
    t.push(Material::named("rock"));
    let world = World::generate(ChunkExtent::new([0, 0, 0], [1, 1, 1]), &BoxRoom, &t);
    let mut gpu = GpuWorld::new(device, world.extent());
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    gpu.sync(&world, queue, &mut enc).expect("sync");
    queue.submit([enc.finish()]);
    gpu
}

fn camera() -> Camera {
    Camera {
        position: Vec3::new(12.0, 12.0, 30.0),
        yaw: 0.7,
        pitch: -0.2,
        vfov: 1.2,
    }
}

const SIZE: [u32; 2] = [192, 108];

/// Steps `n` frames and returns the last one's image.
fn frames(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &mut Frame,
    gpu: &GpuWorld,
    n: usize,
) -> Vec<Vec<u8>> {
    (0..n)
        .map(|_| {
            frame.step(device, queue, gpu, &camera(), &LightParams::default());
            svet_render::debug::read_rgba8(device, queue, frame.target())
        })
        .collect()
}

/// A default frame at 192 x 108 over the box: applying its own settings returns an empty
/// change, and its next 8 frames' bytes equal those of a frame never applied to.
#[test]
fn applying_the_frames_own_settings_changes_nothing() {
    let Some((device, queue)) = device() else {
        return;
    };
    let gpu = mirrored(&device, &queue);
    let mut plain = Frame::new(&device, SIZE[0], SIZE[1]);
    let mut applied = Frame::new(&device, SIZE[0], SIZE[1]);
    let own = applied.settings();
    let change = applied.apply_settings(&device, &queue, &own);
    assert!(change.is_empty(), "{change:?}");
    assert_eq!(applied.settings(), own);
    let a = frames(&device, &queue, &mut plain, &gpu, 8);
    let b = frames(&device, &queue, &mut applied, &gpu, 8);
    for (i, (a, b)) in a.iter().zip(&b).enumerate() {
        assert!(a == b, "frame {i} differs");
    }
}

/// Under FSR 2, changing the scale from 1.5 to 2 is reported as `Settle`, and
/// `settings().scale` reads 2 after the next frame; changing the mirror budget is reported in
/// `rebuild`, and `settings()` still reads the old budget.
#[test]
fn a_scale_change_settles_and_a_budget_change_waits_for_a_rebuild() {
    let Some((device, queue)) = device() else {
        return;
    };
    let gpu = mirrored(&device, &queue);
    let internal = RenderScale::OneHalf.internal(SIZE);
    let mut frame = Frame::with_sizes(&device, internal, SIZE);
    frame.set_upscaler_kind(
        &device,
        svet_render::UpscalerKind::Fsr2(svet_render::Fsr2Settings::default()),
    );
    assert_eq!(frame.settings().scale, RenderScale::OneHalf);
    frames(&device, &queue, &mut frame, &gpu, 2);
    let mut s = frame.settings();
    s.scale = RenderScale::Two;
    let budget = s.build.mirror_budget_bytes;
    s.build.mirror_budget_bytes = budget / 2;
    let change = frame.apply_settings(&device, &queue, &s);
    assert_eq!(change.settled, ["scale"]);
    assert_eq!(change.rebuild, ["build.mirror_budget_bytes"]);
    assert!(change.live.is_empty(), "{change:?}");
    frames(&device, &queue, &mut frame, &gpu, 1);
    assert_eq!(frame.settings().scale, RenderScale::Two);
    assert_eq!(frame.internal_size(), RenderScale::Two.internal(SIZE));
    assert_eq!(frame.settings().build.mirror_budget_bytes, budget);
}

/// For each `Live` and `Settle` row of `FIELDS` whose name `rows` keeps, on `frame` warmed 4
/// frames: the row's test value (`RenderSettings::with_test_value`) applied alone leaves
/// `history_restarts()` unchanged for `Live` and raises it by exactly 1 for `Settle`; the
/// frame's own settings are applied back and one frame stepped after each. With the upscaler
/// off the frame renders at its output size, so two rows apply as the clamp
/// (`RenderSettings::clamped`) says: the scale off FSR 2 changes nothing and restarts nothing,
/// and turning FSR 2 off at a scale other than 1 names the scale and the upscaler and
/// restarts the history once.
fn assert_classes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &mut Frame,
    gpu: &GpuWorld,
    rows: impl Fn(&str) -> bool,
) -> usize {
    frames(device, queue, frame, gpu, 4);
    let base = frame.settings();
    let mut read = 0;
    for &(name, class) in RenderSettings::FIELDS {
        if class == ApplyClass::Rebuild || !rows(name) {
            continue;
        }
        let s = base.with_test_value(name).expect("a test value");
        let before = frame.history_restarts();
        let change = frame.apply_settings(device, queue, &s);
        let after = frame.history_restarts();
        if name == "scale" && base.upscaler == UpscalerChoice::Off {
            assert!(change.is_empty(), "the scale off FSR 2: {change:?}");
            assert_eq!(after, before, "the scale off FSR 2 restarted the history");
            assert_eq!(frame.internal_size(), frame.output_size());
            assert_eq!(frame.settings().scale, RenderScale::One);
        } else {
            let want = u64::from(class == ApplyClass::Settle);
            assert_eq!(after - before, want, "{name} ({class:?}): {change:?}");
            let named = match class {
                ApplyClass::Live => &change.live,
                _ => &change.settled,
            };
            if name == "upscaler"
                && s.upscaler == UpscalerChoice::Off
                && base.scale != RenderScale::One
            {
                assert_eq!(named, &["scale", "upscaler"], "{change:?}");
                assert_eq!(frame.internal_size(), frame.output_size());
            } else {
                assert_eq!(named, &[name], "{change:?}");
            }
        }
        frames(device, queue, frame, gpu, 1);
        frame.apply_settings(device, queue, &base);
        frames(device, queue, frame, gpu, 1);
        read += 1;
    }
    read
}

/// Every `Live` and `Settle` row on a default frame at 64 x 36 (no upscaler).
#[test]
fn every_live_field_keeps_the_history_and_every_settle_field_restarts_it() {
    let Some((device, queue)) = device() else {
        return;
    };
    let gpu = mirrored(&device, &queue);
    let mut frame = Frame::new(&device, 64, 36);
    let read = assert_classes(&device, &queue, &mut frame, &gpu, |_| true);
    let rows = RenderSettings::FIELDS
        .iter()
        .filter(|(_, c)| *c != ApplyClass::Rebuild)
        .count();
    assert_eq!(read, rows);
}

/// The upscaler's rows again on a frame running FSR 2 at scale 1.5 for a 64 x 36 output: FSR
/// 2's knobs keep its history, the scale and the upscaler restart it.
#[test]
fn under_fsr2_its_knobs_keep_the_history_and_the_scale_restarts_it() {
    let Some((device, queue)) = device() else {
        return;
    };
    let gpu = mirrored(&device, &queue);
    let mut frame = Frame::new(&device, 64, 36);
    let mut s = frame.settings();
    s.upscaler = UpscalerChoice::Fsr2;
    s.scale = RenderScale::OneHalf;
    frame.apply_settings(&device, &queue, &s);
    assert!(frame.fsr2().is_some());
    let read = assert_classes(&device, &queue, &mut frame, &gpu, |n| {
        n.starts_with("fsr2.") || ["scale", "upscaler"].contains(&n)
    });
    assert_eq!(read, 8);
}
