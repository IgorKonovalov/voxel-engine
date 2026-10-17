//! The fly-through's audio left and right, end to end and headless: a camera as the fly-through
//! flies it, the listener `flythrough::listener` builds from it, the shipped HRIR pack, and
//! the mixer's interleaved output (channel 0 left, channel 1 right, the order the device
//! stream is opened with). A source on the camera's right must be louder and earlier in the
//! right channel, the mirror on the left, and front and back must not collapse into one.

use vxe_audio::{Command, Hrtf, Mixer, PlayParams, Sound, VoiceId};
use vxe_render::Camera;
use vxe_render::glam::Vec3;

/// The fly-through's scale.
const MPV: f32 = 0.1;
/// 2 m in voxels.
const DISTANCE: f32 = 20.0;
const FRAMES: usize = 4_800;

/// 5 ms of seeded white noise, then silence: a broadband click with a clear onset.
fn burst() -> Sound {
    let mut state = 0x0006_00a5_u64;
    Sound::new(
        (0..FRAMES)
            .map(|n| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                if n < 240 {
                    ((state >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.8
                } else {
                    0.0
                }
            })
            .collect::<Vec<_>>(),
    )
}

/// Renders the burst at `source` (voxel units) heard by `camera`; returns per channel the
/// energy and the first frame whose magnitude reaches a tenth of the louder channel's peak.
fn render(camera: &Camera, source: Vec3) -> [(f32, usize); 2] {
    let mut m = Mixer::new(Hrtf::sadie2_d2(), MPV);
    m.apply(Command::SetListener(flythrough::listener(camera)));
    m.apply(Command::Play(
        VoiceId(1),
        burst(),
        PlayParams::at(source.to_array()),
    ));
    let mut out = vec![0.0; 2 * FRAMES];
    m.render(&mut out, FRAMES);
    let peak = out.iter().fold(0.0f32, |p, s| p.max(s.abs()));
    [0, 1].map(|c| {
        let ch = || out.iter().skip(c).step_by(2);
        let energy = ch().map(|s| s * s).sum();
        let onset = ch().position(|s| s.abs() >= 0.1 * peak).unwrap_or(FRAMES);
        (energy, onset)
    })
}

fn db(a: f32, b: f32) -> f32 {
    10.0 * (a / b).log10()
}

/// Camera poses: facing +x, turned, looking up and down, from somewhere in a cave.
fn cameras() -> Vec<Camera> {
    [
        (0.0, 0.0),
        (1.0, 0.0),
        (2.5, 0.3),
        (-2.0, -0.4),
        (-0.7, 0.8),
    ]
    .into_iter()
    .map(|(yaw, pitch)| Camera {
        position: Vec3::new(812.5, 403.25, 96.0),
        yaw,
        pitch,
        vfov: 1.0,
    })
    .collect()
}

#[test]
fn a_source_on_the_cameras_right_plays_louder_and_earlier_on_the_right() {
    for cam in cameras() {
        let (_, right, _) = cam.basis();
        let [l, r] = render(&cam, cam.position + right * DISTANCE);
        let level = db(r.0, l.0);
        let lead = l.1 as i64 - r.1 as i64;
        println!(
            "yaw {} pitch {}: right source, right over left {level:.1} dB, right leads by {lead} frames",
            cam.yaw, cam.pitch
        );
        assert!(level >= 10.0, "right over left {level} dB");
        assert!(lead >= 10, "right channel leads by {lead} frames");

        let [l, r] = render(&cam, cam.position - right * DISTANCE);
        let level = db(l.0, r.0);
        let lead = r.1 as i64 - l.1 as i64;
        println!(
            "yaw {} pitch {}: left source, left over right {level:.1} dB, left leads by {lead} frames",
            cam.yaw, cam.pitch
        );
        assert!(level >= 10.0, "left over right {level} dB");
        assert!(lead >= 10, "left channel leads by {lead} frames");
    }
}

#[test]
fn the_screens_right_is_world_minus_y_at_yaw_zero() {
    // Facing +x with +z up, the screen's right (where `D` strafes and the marcher's `+ndc.x`
    // looks) is world -y: a source there plays on the right channel.
    let cam = cameras()[0];
    assert_eq!(cam.basis().1, Vec3::new(0.0, -1.0, 0.0));
    let [l, r] = render(&cam, cam.position - Vec3::Y * DISTANCE);
    assert!(db(r.0, l.0) >= 10.0, "right over left {} dB", db(r.0, l.0));
    assert!(r.1 + 10 <= l.1, "onsets left {} right {}", l.1, r.1);
}

#[test]
fn front_and_back_stay_distinct() {
    for cam in cameras() {
        let (forward, _, _) = cam.basis();
        let [fl, fr] = render(&cam, cam.position + forward * DISTANCE);
        let [bl, br] = render(&cam, cam.position - forward * DISTANCE);
        let front_balance = db(fr.0, fl.0);
        let back_balance = db(br.0, bl.0);
        let front_over_back = db(fl.0 + fr.0, bl.0 + br.0);
        println!(
            "yaw {} pitch {}: front R/L {front_balance:.2} dB, back R/L {back_balance:.2} dB, front over back {front_over_back:.2} dB",
            cam.yaw, cam.pitch
        );
        // Both centred: neither drifts to a side.
        assert!(front_balance.abs() <= 3.0, "front R/L {front_balance} dB");
        assert!(back_balance.abs() <= 3.0, "back R/L {back_balance} dB");
        // The pinna shadows the back: front is the brighter, louder of the two.
        assert!(
            front_over_back >= 3.0,
            "front over back {front_over_back} dB"
        );
    }
}
