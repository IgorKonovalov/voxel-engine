//! Every row of the render settings changes the image under the fly-through's defaults, or is
//! listed inert with its reason (lavapipe): one frame over one small world that holds what the
//! rows act on (the sun's light and shadow, a sheltered part the headlamp lights, a baked half
//! and an unbaked half, translucent panes, an emitter, list lights), each row flipped from the
//! defaults to its test value (`RenderSettings::with_test_value`) and rendered from a reset.

mod support;

use flythrough::inert::INERT_WHEN;
use svet_render::{Camera, Frame, GpuWorld, Light, LightParams, RenderSettings, wgpu};
use svet_world::{AIR, ChunkExtent, FnSource, Material, MaterialFlags, World};

/// The output size: FSR 2 at the defaults' 1.5 renders 128 x 72 into it.
const OUT: [u32; 2] = [192, 108];
/// Frames each render steps from its reset.
const FRAMES: usize = 8;
/// Pixels a row's flip changes at the least to count as changing the image: 0.1 % of the
/// output's.
const MIN_PIXELS: usize = (OUT[0] * OUT[1] / 1000) as usize;

/// Why a row's flip changes under `MIN_PIXELS` pixels of the walk's image, for what the run
/// cannot tell from its settings. A row inert under the settings that hold it is
/// `flythrough::inert::INERT_WHEN`'s, which the window reads too.
#[derive(Clone, Copy, Debug)]
enum Why {
    /// It acts on the live queue (when light lands, what it costs), which the walk replaces
    /// with a whole bake: a queue's wall-clock slice is not deterministic.
    Queue,
    /// A `Rebuild` row: the frame leaves it to the caller, who recreates the mirror, the
    /// streamer or the entity pool; the walk rebuilds nothing.
    Rebuild(&'static str),
    /// Inert in every arrangement the walk tried, what it needs not found: a finding
    /// (`docs/backlog.md`, the harness chore's entry).
    Open(&'static str),
    /// It acts on what the walk's world and still camera do not hold (motion, a dig, an
    /// entity, more frames).
    Absent(&'static str),
}

impl Why {
    /// The reason in words.
    fn words(self) -> &'static str {
        match self {
            Why::Rebuild(w) | Why::Open(w) | Why::Absent(w) => w,
            Why::Queue => "the live queue",
        }
    }
}

/// The rows whose flip changes the walk's image in under `MIN_PIXELS` pixels, and why. A row
/// listed here whose flip changes more fails the walk, so the list cannot hold a live row.
const INERT: &[(&str, Why)] = &[
    (
        "lighting.history",
        Why::Absent("a change of light under a light history, which the gradient kills"),
    ),
    (
        "lighting.probe_slices",
        Why::Absent("frames past the lamp channel's first fill, which refreshes every probe"),
    ),
    (
        "lighting.probe_freeze",
        Why::Absent("a probe's visits past the walk's frames"),
    ),
    ("lighting.probe_move_blend", Why::Absent("a moving lamp")),
    (
        "lighting.bake_pool_bytes",
        Why::Rebuild("the store's pool, allocated by the first bake"),
    ),
    ("lighting.landing_fade", Why::Queue),
    ("lighting.source_fade", Why::Queue),
    ("lighting.column_wait", Why::Queue),
    ("lighting.dig_hold", Why::Queue),
    (
        "lighting.key_cap",
        Why::Absent("a key ray past its test value's 1536 voxels: the world is 128 long"),
    ),
    (
        "lighting.key_history",
        Why::Absent("frames past its test value's 64: in 8 the key mean counts under 64 samples"),
    ),
    (
        "lighting.sky_down_skip",
        Why::Open("no pixel under key far ray, key far baked or the light history, or both"),
    ),
    ("lighting.entity_light", Why::Absent("an entity in view")),
    (
        "lighting.candidates",
        Why::Open("no pixel here, under the defaults or with the light history on"),
    ),
    ("lighting.key_step", Why::Queue),
    (
        "look.tuning.face_mix",
        Why::Absent("the field look, whose mirror holds the field's band slots (a rebuild)"),
    ),
    (
        "field_split",
        Why::Absent("the field look, whose mirror holds the field's band slots (a rebuild)"),
    ),
    ("lod.r_full", Why::Absent("chunks past 16 from the camera")),
    ("lod.forced_level", Why::Absent("LOD mips in the mirror")),
    (
        "fsr2.reactive_light",
        Why::Absent("a light that changes between frames"),
    ),
    ("fsr2.metres_per_voxel", Why::Absent("motion")),
    (
        "fsr2.jitter_phases",
        Why::Absent("frames past the first 16 phases, which both sequences share"),
    ),
    ("bake.budget", Why::Queue),
    ("bake.coarse_samples", Why::Queue),
    ("bake.near", Why::Queue),
    ("bake.full_first", Why::Queue),
    ("bake.reach", Why::Queue),
    ("bake.edit_ms", Why::Queue),
    ("bake.dig_reach", Why::Queue),
    ("bake.tiers", Why::Queue),
    ("bake.drain", Why::Queue),
    (
        "build.mirror_budget_bytes",
        Why::Rebuild("the mirror's reservation"),
    ),
    (
        "build.record_chunks",
        Why::Rebuild("the mirror's chunk records"),
    ),
    (
        "build.light_samples",
        Why::Rebuild("the mirror's light-sample buffer"),
    ),
    (
        "build.light_chunk_cap",
        Why::Rebuild("the light samples a chunk keeps"),
    ),
    (
        "build.light_radius",
        Why::Rebuild("the chunks whose emitters light the frame"),
    ),
    (
        "build.light_sync_samples",
        Why::Rebuild("the light samples a sync writes"),
    ),
    (
        "build.sync_budget_bytes",
        Why::Rebuild("the bytes a sync writes"),
    ),
    (
        "build.sync_budget_time",
        Why::Rebuild("the time a sync spends"),
    ),
    (
        "build.field_surface",
        Why::Rebuild("the mirror's field band slots"),
    ),
    ("build.entity_pool_bytes", Why::Rebuild("the entity pool")),
    ("build.residency", Why::Rebuild("the streamer's radii")),
    (
        "build.workers",
        Why::Rebuild("the streamer's generation threads"),
    ),
    (
        "output_hint",
        Why::Rebuild("a size the consumer reads; the engine never applies it"),
    ),
    ("mark_colours", Why::Absent("a marked entity")),
];

/// The walk's world and its view.
struct Fixture {
    device: wgpu::Device,
    queue: wgpu::Queue,
    gpu: GpuWorld,
    world: World,
    camera: Camera,
}

/// Two chunks along +x: rock to z 8 under a rock shelter (a roof at z 28 and three walls,
/// open toward +x) in the first, grass to z 8 and a rock pillar in the second; three glass
/// panes, a flight of steps (a riser every third voxel, for the far normal) and an emissive ore
/// block inside the shelter, another ore block outside; the camera inside looking out through
/// the opening, slightly down.
fn fixture() -> Option<Fixture> {
    use support::SyncNow;
    let mut table = flythrough::with_emissive_ore(&flythrough::outdoor_materials());
    let glass = table.push(Material {
        albedo: [0.2, 0.5, 0.9],
        flags: MaterialFlags::TRANSLUCENT,
        opacity: 0.4,
        ..Material::named("glass")
    });
    let (rock, ore, grass) = (
        flythrough::cave::ROCK,
        flythrough::cave::ORE,
        flythrough::outdoor::GRASS,
    );
    let src = FnSource(move |[x, y, z]: [i32; 3]| {
        let within = |lo: i32, hi: i32, c: i32| (lo..hi).contains(&c);
        if z < 8 {
            return if x < 40 { rock } else { grass };
        }
        let shelter = x < 40 && z < 30;
        if shelter && (z >= 28 || x < 3 || !within(3, 61, y)) {
            return rock;
        }
        if within(10, 38, x) && within(36, 50, y) && z < 8 + (x - 10) / 3 {
            return rock;
        }
        if [18, 22, 26].contains(&x) && within(12, 24, y) && within(8, 20, z) {
            return glass;
        }
        if within(30, 33, x) && within(52, 56, y) && within(8, 12, z)
            || within(48, 51, x) && within(36, 40, y) && within(8, 12, z)
        {
            return ore;
        }
        if within(70, 74, x) && within(30, 34, y) && z < 40 {
            return rock;
        }
        AIR
    });
    let world = World::generate(ChunkExtent::new([0, 0, 0], [2, 1, 1]), &src, &table);
    let (device, queue) = support::software_device(256 << 20)?;
    let cfg = svet_render::MirrorConfig {
        budget_bytes: 128 << 20,
        ..svet_render::MirrorConfig::new(svet_render::Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    gpu.set_focus(svet_world::VoxelCoord([10, 32, 14]));
    while gpu
        .sync_now(&world, &device, &queue)
        .expect("the mirror fits")
        .pending_chunks
        > 0
    {}
    let camera = flythrough::camera_looking_at([8.5, 32.5, 15.5], [60.0, 32.5, 10.0]);
    Some(Fixture {
        device,
        queue,
        gpu,
        world,
        camera,
    })
}

/// The fly-through's settings for an outdoor run with no other flag.
fn defaults() -> RenderSettings {
    let argv = ["--source", "outdoor"].map(String::from);
    flythrough::cli::parse_args_from(argv)
        .expect("the flags parse")
        .render_settings()
}

/// The sun 35 degrees up, east of south-east, behind the pillar.
fn sky() -> svet_render::sky::Sky {
    flythrough::daylight::Clock {
        fixed_sun: Some([35.0, 100.0]),
        ..flythrough::daylight::Clock::default()
    }
    .sky(8.0)
}

/// Five point lights along the shelter's open side: more than the portable profile's cap.
fn lights() -> Vec<Light> {
    (0..5)
        .map(|i| Light::point([34.0, 8.0 + 10.0 * i as f32, 20.0], 200.0))
        .collect()
}

/// A frame of `OUT` under `s` with the sky, the lights and the outdoor styles set, its store
/// baked over the first chunk alone (the shelter), so both a baked and an unbaked face are in
/// view, and a dig into the baked floor after it.
fn new_frame(fx: &mut Fixture, s: &RenderSettings) -> Frame {
    let mut f = Frame::new(&fx.device, OUT[0], OUT[1]);
    f.apply_settings(&fx.device, &fx.queue, s);
    f.set_material_styles(
        &fx.queue,
        &flythrough::outdoor_styles(flythrough::LeafShadow::Voxel),
    );
    f.set_sky(Some(sky()));
    f.set_lights(&lights());
    f.set_dt(1.0 / 60.0);
    f.bake_until(
        &fx.device,
        &fx.queue,
        &mut fx.gpu,
        &fx.world,
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &svet_render::BakeSettings::default(),
        &mut |_| {},
    )
    .expect("the store fits");
    // A dig into the baked floor after the bake, never re-baked: its faces hold no value.
    let carve = svet_world::EditOp::sphere(
        svet_world::Fixed3([16, 28, 8].map(|v| v * svet_world::FIXED_ONE)),
        4 * svet_world::FIXED_ONE,
        svet_world::Mode::Carve,
    );
    let dug = fx.world.apply(&carve).expect("the dig is resident");
    while support::SyncNow::sync_now(&mut fx.gpu, &fx.world, &fx.device, &fx.queue)
        .expect("the mirror fits")
        .pending_chunks
        > 0
    {}
    let (lo, hi) = dug.aabb.expect("the dig carved rock");
    f.invalidate(lo.0, hi.0);
    f
}

/// `FRAMES` frames under `s` from a reset: the frame's temporal state as a new frame holds it
/// (`Frame::restart_as_new`: the history with the light lists the upscaler's light-change test
/// reads, the exposure the first frame's passes use, the frame index) and the lamp channel (a
/// frame with the bounce off over a volume rebuilt at another spacing); the last frame's image.
/// `Frame::reset_history` and `Frame::reset_exposure` alone are not such a reset: a row's
/// count would depend on the rows rendered before it.
fn render(fx: &Fixture, f: &mut Frame, s: &RenderSettings) -> Vec<u8> {
    let mut other = s.clone();
    other.probe_spacing = if s.probe_spacing == 8 { 16 } else { 8 };
    other.lighting.lamp_bounce = false;
    f.apply_settings(&fx.device, &fx.queue, &other);
    f.step(
        &fx.device,
        &fx.queue,
        &fx.gpu,
        &fx.camera,
        &LightParams::headlamp(),
    );
    f.apply_settings(&fx.device, &fx.queue, s);
    f.restart_as_new(&fx.queue);
    for _ in 0..FRAMES {
        f.step(
            &fx.device,
            &fx.queue,
            &fx.gpu,
            &fx.camera,
            &LightParams::headlamp(),
        );
    }
    svet_render::debug::read_rgba8(&fx.device, &fx.queue, f.target())
}

/// Pixels whose colour differs at all.
fn differing(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(p, q)| p[..3] != q[..3])
        .count()
}

/// Every row of `RenderSettings::FIELDS` (the rows Plan 0069's tests walk), flipped alone from
/// the fly-through's outdoor defaults: its image differs from the defaults' in at least
/// `MIN_PIXELS` pixels, or the row is listed inert with its reason and changes fewer: in
/// `INERT`, or in `flythrough::inert::INERT_WHEN` with its condition holding under the
/// defaults. A table row with settings it acts under (`live_under`) changes at least
/// `MIN_PIXELS` pixels there, where its condition does not hold, so the window's mark and
/// the walk cannot disagree. The defaults' image repeats to the byte from a reset, before and
/// after the walk, and each `live_under` image after its row's (key far ray, the light
/// history), so a difference is the row's and not the order of the walk's.
///
/// About 16 s on lavapipe (16.0 s at a load of 2.5), over the 10 s soak line and the gate's only test of the rule, so
/// it runs whenever the crate changes rather than in the gate. A new row fails it until it
/// changes the image here or joins `INERT` with its reason.
#[test]
#[ignore = "sole: every settings row changes the image or is listed inert"]
#[allow(clippy::disallowed_methods, reason = "the walk prints its own time")]
fn every_settings_row_changes_the_image_or_is_listed_inert() {
    let t = std::time::Instant::now();
    let Some(mut fx) = fixture() else { return };
    let base = defaults();
    let mut f = new_frame(&mut fx, &base);
    println!(
        "reading: fixture and bake {:.1} s",
        t.elapsed().as_secs_f64()
    );
    let a = render(&fx, &mut f, &base);
    assert_eq!(
        differing(&a, &render(&fx, &mut f, &base)),
        0,
        "the defaults repeat"
    );
    let (mut weak, mut live_inert) = (Vec::new(), Vec::new());
    for &(name, _) in RenderSettings::FIELDS {
        let s = base.with_test_value(name).expect("a test value");
        let d = differing(&a, &render(&fx, &mut f, &s));
        let listed = INERT
            .iter()
            .find(|(n, _)| *n == name)
            .map(|&(_, w)| w.words())
            .or_else(|| {
                INERT_WHEN
                    .iter()
                    .find(|r| r.row == name && (r.holds)(&base))
                    .map(|r| r.reason)
            });
        println!("reading: {name:32} {d:6} {}", listed.unwrap_or(""));
        match listed {
            Some(_) if d >= MIN_PIXELS => live_inert.push((name, d)),
            None if d < MIN_PIXELS => weak.push((name, d)),
            _ => {}
        }
    }
    assert_eq!(
        differing(&a, &render(&fx, &mut f, &base)),
        0,
        "the defaults repeat after"
    );
    let mut unproven = Vec::new();
    for r in INERT_WHEN {
        assert!(
            (r.holds)(&base),
            "{} is not inert under the defaults",
            r.row
        );
        assert!(
            INERT.iter().all(|(n, _)| *n != r.row),
            "{} is listed twice",
            r.row
        );
        let Some(live) = r.live_under else { continue };
        let mut on = base.clone();
        live(&mut on);
        assert!(!(r.holds)(&on), "{} holds under its live_under", r.row);
        let s = on.with_test_value(r.row).expect("a test value");
        let reference = render(&fx, &mut f, &on);
        let d = differing(&reference, &render(&fx, &mut f, &s));
        println!("reading: {:32} {d:6} under its live_under", r.row);
        assert_eq!(
            differing(&reference, &render(&fx, &mut f, &on)),
            0,
            "{}: the image under its live_under repeats after the row's",
            r.row
        );
        if d < MIN_PIXELS {
            unproven.push((r.row, d));
        }
    }
    println!("reading: walk {:.1} s", t.elapsed().as_secs_f64());
    assert!(
        weak.is_empty(),
        "rows that change under {MIN_PIXELS} pixels and are not listed inert: {weak:?}"
    );
    assert!(
        live_inert.is_empty(),
        "rows listed inert that change {MIN_PIXELS} pixels or more: {live_inert:?}"
    );
    assert!(
        unproven.is_empty(),
        "rows that change under {MIN_PIXELS} pixels under their live_under too: {unproven:?}"
    );
    for n in INERT
        .iter()
        .map(|(n, _)| *n)
        .chain(INERT_WHEN.iter().map(|r| r.row))
    {
        assert!(RenderSettings::class_of(n).is_some(), "{n} is no row");
    }
}
