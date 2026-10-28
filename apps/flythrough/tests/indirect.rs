//! Indirect light from the probe volume (ADR-0023), on the software adapter (lavapipe).

mod support;

use support::*;
use vxe_render::GpuWorld;
use vxe_render::probes::{PROBE_COUNT, TestProbe, WINDOW_FRAMES, probe_slot};
use vxe_render::{LightParams, Lighting, ToneMap, readback};
use vxe_world::*;

/// The lighting of the probe-preset tests: no lamp, no filter, the identity tone map, indirect
/// light on (the cavity's table has no emissive row).
fn preset_lighting(indirect: bool) -> Lighting {
    Lighting {
        lamp: false,
        filter: false,
        tone_map: ToneMap::Identity,
        indirect,
        ..Lighting::default()
    }
}

/// The display byte of linear `c` under the identity tone map.
fn encoded(c: f32) -> u8 {
    (c.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8
}

/// Renders the cavity with every probe preset by `f`, returning the image and the hit
/// records.
fn render_preset(
    rig: &mut LookRig,
    indirect: bool,
    f: impl Fn([i32; 3]) -> TestProbe,
) -> (Vec<u8>, Vec<[u16; 4]>) {
    let camera = cavity_camera();
    rig.frame.set_lighting(preset_lighting(indirect));
    rig.frame.probes_mut().set_updates(false);
    let origin = rig.frame.probes().origin_for(camera.position.to_array());
    rig.frame
        .probes()
        .write_test_irradiance(&rig.queue, origin, f);
    let rgba = rig.render(
        &camera,
        &LightParams::headlamp(),
        vxe_render::LookParams::default(),
    );
    let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    (rgba, info)
}

/// The largest difference, over every pixel, between the image's rgb and `want` of the
/// pixel's hit (material id, or `None` on a miss); and the hit pixels counted.
fn worst_error(
    rgba: &[u8],
    info: &[[u16; 4]],
    want: impl Fn(Option<u16>) -> [u8; 3],
) -> (u8, usize) {
    let mut worst = 0u8;
    let mut hits = 0;
    for (i, rec) in info.iter().enumerate() {
        let hit = rec[3] & 1 != 0;
        hits += usize::from(hit);
        let w = want(hit.then_some(rec[2]));
        for k in 0..3 {
            worst = worst.max(rgba[4 * i + k].abs_diff(w[k]));
        }
    }
    (worst, hits)
}

/// On the cavity at 320 x 180 with the lamp off, no emissive material, the filter off and the
/// identity tone map: every probe preset to irradiance `[1, 1, 1]`, visibility clear, makes
/// every hit pixel read `albedo / PI` (the composite's Lambertian `albedo E / PI`) within
/// 2/255, and preset to `PI` its albedo within 2/255; every miss pixel reads 0; with indirect
/// light off every pixel reads 0.
#[test]
fn a_constant_probe_lights_every_hit_with_its_albedo() {
    let world = cavity_world();
    let Some(mut rig) = LookRig::new(&world, None) else {
        return;
    };
    let albedo = look_table().get(3).expect("row 3").albedo;
    let pi = std::f32::consts::PI;
    for (value, label) in [(1.0, "1"), (pi, "pi")] {
        let (rgba, info) = render_preset(&mut rig, true, |_| TestProbe {
            irradiance: [value; 3],
            inside: false,
        });
        let (worst, hits) = worst_error(&rgba, &info, |m| match m {
            Some(3) => albedo.map(|a| encoded(a * value / pi)),
            Some(m) => panic!("material {m}"),
            None => [0; 3],
        });
        println!("probes at {label}: {hits} hit pixels, worst error {worst}/255");
        assert!(hits > 0);
        assert!(worst <= 2, "{worst}");
    }
    let (rgba, info) = render_preset(&mut rig, false, |_| TestProbe {
        irradiance: [pi; 3],
        inside: false,
    });
    let (worst, _) = worst_error(&rgba, &info, |_| [0; 3]);
    println!("indirect off: worst pixel {worst}/255");
    assert_eq!(worst, 0);
}

/// A probe preset to the inside flag contributes nothing: with every probe of one plane of
/// the volume crossing the cavity inside rock and preset to irradiance 50, and every other at
/// `PI`, every hit pixel still reads its albedo within 2/255; with every probe inside, every
/// pixel reads 0.
#[test]
fn an_inside_probe_contributes_nothing() {
    let world = cavity_world();
    let Some(mut rig) = LookRig::new(&world, None) else {
        return;
    };
    let albedo = look_table().get(3).expect("row 3").albedo;
    let pi = std::f32::consts::PI;
    // World probe x = 1 sits at voxel x = 16.5, inside the radius-40 cavity.
    let (rgba, info) = render_preset(&mut rig, true, |w| {
        let inside = w[0] == 1;
        TestProbe {
            irradiance: if inside { [50.0; 3] } else { [pi; 3] },
            inside,
        }
    });
    let (worst, hits) = worst_error(&rgba, &info, |m| match m {
        Some(_) => albedo.map(encoded),
        None => [0; 3],
    });
    println!("one plane inside: {hits} hit pixels, worst error {worst}/255");
    assert!(worst <= 2, "{worst}");
    let (rgba, info) = render_preset(&mut rig, true, |_| TestProbe {
        irradiance: [pi; 3],
        inside: true,
    });
    let (worst, _) = worst_error(&rgba, &info, |_| [0; 3]);
    println!("every probe inside: worst pixel {worst}/255");
    assert_eq!(worst, 0);
}

// ---- The update pass (furnace, leak, inside flag) ----

/// Material rows: `glow` (albedo 0.5, emission `[1, 1, 1]`) and `dull` (albedo 0.5, dark).
fn furnace_table() -> (MaterialTable, MaterialId, MaterialId) {
    let mut t = MaterialTable::new();
    let glow = t.push(Material {
        albedo: [0.5; 3],
        emission: [1.0; 3],
        ..Material::named("glow")
    });
    let dull = t.push(Material {
        albedo: [0.5; 3],
        ..Material::named("dull")
    });
    (t, glow, dull)
}

fn box_op(min: [i64; 3], max: [i64; 3], mode: Mode) -> EditOp {
    EditOp::box_(
        Fixed3(min.map(|v| v * FIXED_ONE)),
        Fixed3(max.map(|v| v * FIXED_ONE)),
        mode,
    )
}

/// The furnace box's interior: 24 x 24 x 12 voxels centred on voxel corner (32, 32, 32), so
/// world probe (2, 2, 2) at the centre of voxel (32, 32, 32) is its centre probe.
const BOX_MIN: [i64; 3] = [20, 20, 26];
const BOX_MAX: [i64; 3] = [44, 44, 38];

/// One chunk of solid `glow` with the furnace box carved: its six walls are emissive.
fn furnace_world() -> (World, MaterialId) {
    let (table, glow, _) = furnace_table();
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &UniformSource(glow),
        &table,
    );
    world.apply(&box_op(BOX_MIN, BOX_MAX, Mode::Carve)).unwrap();
    (world, glow)
}

/// A 1 x 1 frame looking straight down at the floor centre of the box whose interior spans
/// x in `[x0, x0 + 24)`, at full lighting resolution, lamp off, filter off, indirect on.
struct FloorProbe {
    frame: vxe_render::Frame,
    camera: vxe_render::Camera,
    light: LightParams,
}

impl FloorProbe {
    fn new(device: &vxe_render::wgpu::Device, x0: i64) -> Self {
        let mut frame = vxe_render::Frame::new(device, 1, 1);
        frame.set_resolution(device, vxe_render::LightingResolution::Full);
        frame.set_lighting(Lighting {
            lamp: false,
            filter: false,
            indirect: true,
            ..Lighting::default()
        });
        let camera = vxe_render::Camera {
            position: [x0 as f32 + 12.0, 32.0, 37.0].into(),
            yaw: 0.0,
            pitch: -std::f32::consts::FRAC_PI_2,
            vfov: 0.5,
        };
        Self {
            frame,
            camera,
            light: LightParams::headlamp(),
        }
    }

    /// Steps `n` frames.
    fn run(
        &mut self,
        device: &vxe_render::wgpu::Device,
        queue: &vxe_render::wgpu::Queue,
        gpu: &GpuWorld,
        n: u32,
    ) {
        for _ in 0..n {
            self.frame
                .step(device, queue, gpu, &self.camera, &self.light);
        }
    }

    /// The gathered indirect irradiance at the floor centre (rgb).
    fn indirect(
        &self,
        device: &vxe_render::wgpu::Device,
        queue: &vxe_render::wgpu::Queue,
    ) -> [f32; 3] {
        let e = readback::read_rgba16f(device, queue, self.frame.indirect_irradiance())[0];
        [e[0], e[1], e[2]]
    }
}

/// The probe rays' own direct mean `D` at their hits in the furnace, and its standard error:
/// with the centre probe's irradiance tile zeroed before every frame, its rays' probe term at
/// their hits is 0 (the box holds one live probe), so a settled visit leaves
/// `(1 - HYSTERESIS) x albedo x D_t` in texel `t`. `D` is the mean of the texels over that
/// factor across the interior texels of `visits` visits; the error is the standard deviation
/// of the per-visit means over the square root of the visits.
fn furnace_direct_mean(
    f: &mut FloorProbe,
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    gpu: &GpuWorld,
    visits: usize,
) -> (f64, f64) {
    let slot = probe_slot([2, 2, 2]);
    let scale = f64::from(1.0 - vxe_render::probes::HYSTERESIS) * 0.5;
    let mut means = Vec::new();
    while means.len() < visits {
        f.frame.probes().write_test_tile(queue, slot, [0.0; 3]);
        f.run(device, queue, gpu, 1);
        let probe = f.frame.probes().read_probe(device, queue, slot);
        assert!(
            probe.age() > vxe_render::probes::YOUNG_UPDATES,
            "{}",
            probe.age()
        );
        let texels = probe.interior_irradiance();
        if texels.iter().all(|t| t[0] == 0.0) {
            continue;
        }
        let sum: f64 = texels.iter().map(|t| f64::from(t[0]) / scale).sum();
        means.push(sum / texels.len() as f64);
    }
    let n = means.len() as f64;
    let d = means.iter().sum::<f64>() / n;
    let var = means.iter().map(|m| (m - d) * (m - d)).sum::<f64>() / (n - 1.0);
    (d, (var / n).sqrt())
}

/// The furnace: in the closed box of 24 x 24 x 12 interior voxels whose six walls emit
/// `[1, 1, 1]` at albedo 0.5, lamp off, the probe update converges to its fixed point within
/// 256 frames. The fixed point is the probe rays' own direct mean `D` at their hits
/// ([`furnace_direct_mean`], over 256 visits, its standard error at most 0.5 % of `D`): a probe
/// reads `0.5 (D + E)`, so `E = D`, which the firefly clamp of the emissive estimate puts
/// about 8.5 % under `PI L_e = PI` here (printed, not asserted). After 256 frames every
/// channel of the floor centre's gathered indirect irradiance and every interior texel of
/// the box's centre probe lie within 7 % of `D`, and the centre probe's texel mean within 4 %.
/// Every probe whose position lies in solid reads the inside flag after its first update (8
/// frames).
#[test]
fn the_furnace_box_reads_pi_of_indirect_irradiance() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let (world, _) = furnace_world();
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut f = FloorProbe::new(&device, BOX_MIN[0]);
    let origin = f.frame.probes().origin_for(f.camera.position.to_array());
    f.run(&device, &queue, &gpu, 8);
    let states = f.frame.probes().read_states(&device, &queue);
    let mut inside = 0;
    for &w in &[[1, 2, 2], [2, 2, 1], [3, 3, 3], [0, 0, 0], [2, 2, 3]] {
        let s = states[probe_slot(w) as usize];
        assert_eq!(s & 1, 1, "probe {w:?} in solid: state {s:#x}");
        assert!((s >> 1) & 127 >= 1, "probe {w:?} traced");
        inside += 1;
    }
    let centre = states[probe_slot([2, 2, 2]) as usize];
    assert_eq!(centre & 1, 0, "the centre probe is live: {centre:#x}");
    println!("after 8 frames: {inside} solid probes flagged inside, origin {origin:?}");
    let mut series = Vec::new();
    let mut direct = Vec::new();
    for _ in 0..31 {
        for _ in 0..8 {
            f.run(&device, &queue, &gpu, 1);
            direct.push(f64::from(
                readback::read_rgba16f(&device, &queue, f.frame.irradiance())[0][0],
            ));
        }
        series.push(f.indirect(&device, &queue)[0]);
    }
    let pi = std::f64::consts::PI;
    // The light pass's direct estimate at the floor-centre pixel (filter off: its emissive
    // part); it sees almost none of the clamp the probe rays' hits near the box's edges do.
    let direct_mean = direct.iter().sum::<f64>() / direct.len() as f64;
    println!(
        "the light pass's floor-centre direct over 248 frames: mean {direct_mean:.4} ({:+.2} % of pi)",
        (direct_mean / pi - 1.0) * 100.0
    );
    let e = f.indirect(&device, &queue).map(f64::from);
    let probe = f
        .frame
        .probes()
        .read_probe(&device, &queue, probe_slot([2, 2, 2]));
    let texels: Vec<f64> = probe
        .interior_irradiance()
        .iter()
        .flatten()
        .map(|&v| f64::from(v))
        .collect();
    let (lo, hi) = texels
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let mean = texels.iter().sum::<f64>() / texels.len() as f64;
    let (d, err) = furnace_direct_mean(&mut f, &device, &queue, &gpu, 256);
    let pct = |v: f64, of: f64| (v / of - 1.0) * 100.0;
    println!(
        "the probe rays' direct mean D {d:.4} (standard error {:.3} %, {:+.2} % of pi)",
        err / d * 100.0,
        pct(d, pi)
    );
    println!(
        "floor centre after 256 frames {e:?}: {:+.2} % of D, {:+.2} % of pi; every 8 frames {series:?}",
        pct(e[0], d),
        pct(e[0], pi)
    );
    println!(
        "centre probe: age {}, texels {lo:.4} ..= {hi:.4} ({:+.2} ..= {:+.2} % of D), mean {mean:.4} ({:+.2} % of D, {:+.2} % of pi)",
        probe.age(),
        pct(lo, d),
        pct(hi, d),
        pct(mean, d),
        pct(mean, pi)
    );
    assert!(err <= 0.005 * d, "D {d} error {err}");
    for c in e {
        assert!((c / d - 1.0).abs() <= 0.07, "floor {e:?} against D {d}");
    }
    for &c in &texels {
        assert!((c / d - 1.0).abs() <= 0.07, "texel {c} against D {d}");
    }
    assert!(
        (mean / d - 1.0).abs() <= 0.04,
        "texel mean {mean} against D {d}"
    );
}

/// Two boxes side by side, separated by a 2-voxel wall, only the first with emissive walls:
/// after 256 frames the dark box's floor-centre indirect irradiance is under 2 % of the lit
/// box's (no leak through the wall).
fn two_boxes() -> World {
    let (table, glow, dull) = furnace_table();
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 1, 1]),
        &UniformSource(dull),
        &table,
    );
    // The lit box: a shell of glow around BOX_MIN..BOX_MAX, x up to 44 inclusive.
    let shell_min = BOX_MIN.map(|v| v - 1);
    let shell_max = BOX_MAX.map(|v| v + 1);
    world
        .apply(&box_op(
            shell_min,
            shell_max,
            Mode::Replace {
                from: dull,
                to: glow,
            },
        ))
        .unwrap();
    world.apply(&box_op(BOX_MIN, BOX_MAX, Mode::Carve)).unwrap();
    // The dark box: x in [46, 70), behind the glow shell's x = 44 and the dull x = 45.
    let dark_min = [DARK_X0, BOX_MIN[1], BOX_MIN[2]];
    let dark_max = [DARK_X0 + 24, BOX_MAX[1], BOX_MAX[2]];
    world
        .apply(&box_op(dark_min, dark_max, Mode::Carve))
        .unwrap();
    world
}

/// The dark box's lowest interior x.
const DARK_X0: i64 = 46;

#[test]
fn no_light_leaks_through_a_two_voxel_wall() {
    let world = two_boxes();
    let full = world.stats().full_bricks as u64;
    let Some((device, queue)) = software_device(((full + 4096) * 512).max(64 << 20)) else {
        return;
    };
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut lit = FloorProbe::new(&device, BOX_MIN[0]);
    let mut dark = FloorProbe::new(&device, DARK_X0);
    lit.run(&device, &queue, &gpu, 256);
    dark.run(&device, &queue, &gpu, 256);
    let l = lit.indirect(&device, &queue)[0];
    let d = dark.indirect(&device, &queue)[0];
    println!(
        "lit floor {l:.4}, dark floor {d:.6}: {:.4} % of the lit",
        d / l * 100.0
    );
    assert!(l > 0.5, "{l}");
    assert!(d < 0.02 * l, "{d} against {l}");
}

/// An inside probe's texels are never read by the gather: after the furnace converges, with
/// the update off, every probe flagged inside (the chunk's 63 in solid) is overwritten with irradiance 1000 and the
/// floor centre's indirect irradiance reads exactly what it read before.
#[test]
fn an_inside_probe_s_texels_are_never_gathered() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let (world, _) = furnace_world();
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut f = FloorProbe::new(&device, BOX_MIN[0]);
    f.run(&device, &queue, &gpu, 64);
    f.frame.probes_mut().set_updates(false);
    f.run(&device, &queue, &gpu, 1);
    let before = f.indirect(&device, &queue);
    let states = f.frame.probes().read_states(&device, &queue);
    let poked: Vec<u32> = (0..states.len() as u32)
        .filter(|&s| states[s as usize] & 1 == 1)
        .collect();
    for &s in &poked {
        f.frame.probes().write_test_tile(&queue, s, [1000.0; 3]);
    }
    f.run(&device, &queue, &gpu, 1);
    let after = f.indirect(&device, &queue);
    println!(
        "{} inside probes set to 1000: floor centre {before:?} -> {after:?}",
        poked.len()
    );
    // The chunk's 4 x 4 x 4 probes (voxels 0.5 to 48.5) less the box's centre probe.
    assert_eq!(poked.len(), 63);
    assert!(before[0] > 1.0, "{before:?}");
    assert_eq!(before, after);
}

// ---- Scrolling and edits ----

/// After 16 updated frames at the cavity's centre, a camera jump of 20 probe steps along x
/// (the update off for the jump's frame) leaves the 12 x 32 x 32 probes that stay in the
/// volume texel for texel as they were, at the same slots, and every one of the 20 x 32 x 32
/// probes that enter reads age 0.
#[test]
fn a_scroll_keeps_the_probes_that_stay_and_enters_new_ones_at_age_zero() {
    let world = cavity_world();
    let Some(mut rig) = LookRig::new(&world, None) else {
        return;
    };
    let mut camera = cavity_camera();
    rig.frame.set_lighting(Lighting {
        indirect: true,
        ..Lighting::default()
    });
    let light = LightParams::headlamp();
    for _ in 0..16 {
        rig.frame
            .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
    }
    let probes = rig.frame.probes();
    let before_origin = probes.origin_for(camera.position.to_array());
    let before = readback::read_buffer(
        &rig.device,
        &rig.queue,
        probes.data(),
        0,
        probes.data().size(),
    );
    let spacing = probes.spacing() as f32;
    camera.position.x += 20.0 * spacing;
    rig.frame.probes_mut().set_updates(false);
    rig.frame
        .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
    let probes = rig.frame.probes();
    let origin = probes.origin_for(camera.position.to_array());
    assert_eq!(
        origin,
        [before_origin[0] + 20, before_origin[1], before_origin[2]]
    );
    let states = probes.read_states(&rig.device, &rig.queue);
    let after = readback::read_buffer(
        &rig.device,
        &rig.queue,
        probes.data(),
        0,
        probes.data().size(),
    );
    let (mut stayed, mut entered, mut live_before) = (0, 0, 0);
    for z in 0..32 {
        for y in 0..32 {
            for x in 0..32 {
                let w = [origin[0] + x, origin[1] + y, origin[2] + z];
                let slot = probe_slot(w);
                if w[0] < before_origin[0] + 32 {
                    assert_eq!(
                        probe_bytes(&after, slot),
                        probe_bytes(&before, slot),
                        "probe {w:?}"
                    );
                    stayed += 1;
                    live_before +=
                        u32::from(vxe_render::ProbeVolume::age(states[slot as usize], w) > 0);
                } else {
                    let age = vxe_render::ProbeVolume::age(states[slot as usize], w);
                    assert_eq!(age, 0, "entering probe {w:?}");
                    entered += 1;
                }
            }
        }
    }
    println!("{stayed} probes stayed ({live_before} traced), {entered} entered at age 0");
    assert_eq!((stayed, entered), (12 * 32 * 32, 20 * 32 * 32));
    assert!(live_before > 0);
}

/// Slot `slot`'s irradiance tile, visibility tile and state word, as bytes of the data buffer
/// read back whole in `data`.
fn probe_bytes(data: &[u8], slot: u32) -> Vec<u8> {
    use vxe_render::probes::{STATE_BASE, VIS_BASE};
    let s = slot as usize;
    let mut out = data[s * 512..(s + 1) * 512].to_vec();
    let v = VIS_BASE as usize * 4 + s * 1024;
    out.extend_from_slice(&data[v..v + 1024]);
    let st = STATE_BASE as usize * 4 + s * 4;
    out.extend_from_slice(&data[st..st + 4]);
    out
}

/// Frames after the carve at which the hole test reads the dark box's settled reference: 256
/// probe visits, by which the bounce series has converged (256 frames after the carve it still
/// rises by about 12 %).
const HOLE_SETTLED_FRAMES: usize = 2048;

/// In the two-box scene after 256 frames, a radius-4 carve through the dividing wall makes the
/// dark box's floor-centre indirect irradiance reach, within 32 frames, at least 52 % of its
/// settled value (its reading [`HOLE_SETTLED_FRAMES`] frames after the carve) and at least 10
/// times its pre-carve value (the probes the edit box touches refresh at once, the rest in
/// their slices). The 52 % is the probe update with its slices taken by `slot mod slices`
/// (53.1 % on lavapipe) less a point for run-to-run noise, so an update slower to follow a
/// carve fails. Filling the hole back with the wall's rock drops it under 5 % of the lit box's
/// within 64 frames.
#[test]
fn a_hole_in_the_wall_lights_the_dark_box_and_filling_it_darkens_it_again() {
    let world = two_boxes();
    let full = world.stats().full_bricks as u64;
    let Some((device, queue)) = software_device(((full + 4096) * 512).max(64 << 20)) else {
        return;
    };
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut lit = FloorProbe::new(&device, BOX_MIN[0]);
    let mut dark = FloorProbe::new(&device, DARK_X0);
    lit.run(&device, &queue, &gpu, 256);
    dark.run(&device, &queue, &gpu, 256);
    let d0 = dark.indirect(&device, &queue)[0];
    let edit = |mode: Mode, lit: &mut FloorProbe, dark: &mut FloorProbe, gpu: &mut GpuWorld| {
        let r = world
            .apply(&EditOp::sphere(
                centre_of_voxel([45, 32, 31]),
                4 * FIXED_ONE,
                mode,
            ))
            .unwrap();
        let (min, max) = r.aabb.expect("the edit changes voxels");
        gpu.sync_now(&world, &device, &queue).unwrap();
        lit.frame.invalidate(min.0, max.0);
        dark.frame.invalidate(min.0, max.0);
        (min, max)
    };
    let dull = furnace_table().2;
    let aabb = edit(Mode::Carve, &mut lit, &mut dark, &mut gpu);
    let mut opened = Vec::new();
    for frame in 1..=HOLE_SETTLED_FRAMES {
        lit.run(&device, &queue, &gpu, 1);
        dark.run(&device, &queue, &gpu, 1);
        if frame <= 32 || frame % 256 == 0 {
            opened.push((frame, dark.indirect(&device, &queue)[0]));
        }
    }
    let settled = opened.last().expect("frames ran").1;
    let at_32 = opened[31].1;
    println!(
        "dark {d0:.5} before; carve {aabb:?}: frame 32 {at_32:.5} = {:.1} % of settled {settled:.5} (frame {HOLE_SETTLED_FRAMES}), {:.1} x the pre-carve; frame 256 {:.5} ({:.1} %); frames 1 to 32, then every 256: {:?}",
        at_32 / settled * 100.0,
        at_32 / d0,
        opened[32].1,
        at_32 / opened[32].1 * 100.0,
        opened.iter().map(|&(_, v)| v).collect::<Vec<_>>()
    );
    assert!(at_32 >= 0.52 * settled, "{at_32} against settled {settled}");
    assert!(at_32 >= 10.0 * d0, "{at_32} against pre-carve {d0}");
    edit(Mode::Fill(dull), &mut lit, &mut dark, &mut gpu);
    let mut closed = Vec::new();
    for _ in 0..64 {
        lit.run(&device, &queue, &gpu, 1);
        dark.run(&device, &queue, &gpu, 1);
        closed.push(dark.indirect(&device, &queue)[0] / lit.indirect(&device, &queue)[0]);
    }
    println!("filled: dark / lit over 64 frames {closed:?}");
    assert!(closed[63] < 0.05, "{closed:?}");
}

/// The probe settings scale the update's dispatch: in the furnace, the default (64 rays, one
/// probe in 8) dispatches 4096 x 64 rays a frame, 32 rays half that, one in 16 half that, and
/// both a quarter; at 32 rays and one in 16 the box's centre probe is live after 16 frames.
#[test]
fn the_probe_settings_scale_the_rays_a_frame_traces() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let (world, _) = furnace_world();
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut f = FloorProbe::new(&device, BOX_MIN[0]);
    let base = f.frame.lighting();
    let mut traced = Vec::new();
    for (rays, slices, want) in [
        (64, 8, 4096 * 64),
        (32, 8, 4096 * 32),
        (64, 16, 2048 * 64),
        (32, 16, 2048 * 32),
    ] {
        f.frame.set_lighting(Lighting {
            probe_rays: rays,
            probe_slices: slices,
            ..base
        });
        f.run(&device, &queue, &gpu, 1);
        let n = f.frame.traced_probe_rays();
        println!("{rays} rays, one in {slices}: {n} rays a frame");
        assert_eq!(n, want);
        traced.push(n);
    }
    assert_eq!(traced[1] * 2, traced[0]);
    assert_eq!(traced[2] * 2, traced[0]);
    assert_eq!(traced[3] * 4, traced[0]);
    f.run(&device, &queue, &gpu, 16);
    let probe = f
        .frame
        .probes()
        .read_probe(&device, &queue, probe_slot([2, 2, 2]));
    let lo = probe
        .interior_irradiance()
        .iter()
        .flatten()
        .fold(f32::MAX, |a, &v| a.min(v));
    println!(
        "at 32 rays, one in 16: centre probe age {}, lowest texel {lo:.4}",
        probe.age()
    );
    assert!(!probe.inside() && probe.age() >= 1);
    assert!(lo > 0.5, "{lo}");
}

// ---- Light changes (the light-change window) ----

/// One chunk of solid `dull` (albedo 0.5, no emission) with the furnace box carved, and a
/// table with no emissive row.
fn dull_box_world() -> World {
    let mut table = MaterialTable::new();
    let dull = table.push(Material {
        albedo: [0.5; 3],
        ..Material::named("dull")
    });
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &UniformSource(dull),
        &table,
    );
    world.apply(&box_op(BOX_MIN, BOX_MAX, Mode::Carve)).unwrap();
    world
}

/// The first frame (1-based) of `series` under `bar`.
fn first_under(series: &[f32], bar: f32) -> Option<usize> {
    series.iter().position(|&v| v < bar).map(|i| i + 1)
}

/// The dull box lit by the lamp from its centre, looking at the floor, for 256 frames; then
/// the lamp switched off through `Frame::set_lighting` and `frames` more frames: the
/// floor-centre indirect before the switch and at each frame after it, and whether each
/// frame's update ran inside the light-change window.
fn lamp_off_fade(restarts: bool, frames: u32) -> Option<(f32, Vec<f32>, Vec<bool>)> {
    let world = dull_box_world();
    let (device, queue) = software_device(64 << 20)?;
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut f = FloorProbe::new(&device, BOX_MIN[0]);
    f.camera.position.z = 32.0;
    f.frame.probes_mut().set_light_restarts(restarts);
    let on = Lighting {
        lamp: true,
        ..f.frame.lighting()
    };
    f.frame.set_lighting(on);
    f.run(&device, &queue, &gpu, 256);
    let f0 = f.indirect(&device, &queue)[0];
    f.frame.set_lighting(Lighting { lamp: false, ..on });
    let mut series = Vec::new();
    let mut window = Vec::new();
    for _ in 0..frames {
        f.run(&device, &queue, &gpu, 1);
        series.push(f.indirect(&device, &queue)[0]);
        window.push(f.frame.probe_light_window());
    }
    Some((f0, series, window))
}

/// Lamp off: in the closed dull box (24 x 24 x 12 interior voxels, albedo 0.5, nothing
/// emissive) lit by the lamp from its centre at the defaults (64 rays, one probe in 8), the
/// floor-centre indirect `F0` after 256 frames is above 0; with the lamp switched off it is
/// under 10 % of `F0` at every frame from frame 24 after the switch on. The switch opens the
/// light-change window for exactly its 16 frames, which trace one probe in 4 a frame.
#[test]
fn switching_the_lamp_off_drops_the_probe_history() {
    let Some((f0, series, window)) = lamp_off_fade(true, 160) else {
        return;
    };
    let first = first_under(&series, 0.1 * f0);
    println!(
        "lamp off: F0 {f0:.5}; first frame under 10 %: {first:?}; frames 1..=48 {:?}",
        &series[..48]
    );
    assert!(f0 > 0.0, "{f0}");
    let w = WINDOW_FRAMES as usize;
    assert_eq!(window[..w], vec![true; w], "{window:?}");
    assert!(window[w..].iter().all(|&w| !w), "{window:?}");
    for (i, &v) in series.iter().enumerate().skip(23) {
        assert!(v < 0.1 * f0, "frame {}: {v} against F0 {f0}", i + 1);
    }
}

/// The comparison for the lamp-off fade: the same switch with the light-change restart off,
/// the probes fading at their settled hysteresis. Prints the first frame under 10 % of `F0`
/// in 1600 frames; ignored by default (about 1900 frames on the software adapter).
#[test]
#[ignore = "a reading for the log, not a bar"]
fn the_lamp_off_fade_without_the_window() {
    let Some((f0, series, window)) = lamp_off_fade(false, 1600) else {
        return;
    };
    let first = first_under(&series, 0.1 * f0);
    println!(
        "lamp off, restarts off: F0 {f0:.5}; first frame under 10 %: {first:?} in {} frames; every 100: {:?}",
        series.len(),
        series.iter().step_by(100).collect::<Vec<_>>()
    );
    assert!(window.iter().all(|&w| !w));
}

/// Emission step: in the furnace after 256 frames (floor-centre indirect `F0` near pi),
/// `set_emission_scale(glow, 0)` brings the floor-centre indirect under 10 % of `F0` at every
/// frame from frame 24 after the step on; stepping the scale back to 1 brings it to at least
/// 90 % of `F0` by frame 64.
#[test]
fn an_emission_step_drops_the_probe_history() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let (world, glow) = furnace_world();
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut f = FloorProbe::new(&device, BOX_MIN[0]);
    f.run(&device, &queue, &gpu, 256);
    let f0 = f.indirect(&device, &queue)[0];
    f.frame.set_emission_scale(&queue, glow, 0.0);
    let mut off = Vec::new();
    for _ in 0..96 {
        f.run(&device, &queue, &gpu, 1);
        off.push(f.indirect(&device, &queue)[0]);
    }
    f.frame.set_emission_scale(&queue, glow, 1.0);
    let mut on = Vec::new();
    for _ in 0..64 {
        f.run(&device, &queue, &gpu, 1);
        on.push(f.indirect(&device, &queue)[0]);
    }
    println!(
        "emission 0: F0 {f0:.4}; first frame under 10 %: {:?}; frames 1..=64 {:?}",
        first_under(&off, 0.1 * f0),
        &off[..64]
    );
    println!(
        "emission back to 1: frame 64 {:.4} = {:.1} % of F0; first frame at 90 %: {:?}",
        on[63],
        on[63] / f0 * 100.0,
        on.iter().position(|&v| v >= 0.9 * f0).map(|i| i + 1)
    );
    for (i, &v) in off.iter().enumerate().skip(23) {
        assert!(v < 0.1 * f0, "frame {}: {v} against F0 {f0}", i + 1);
    }
    assert!(on[63] >= 0.9 * f0, "{} against F0 {f0}", on[63]);
}

/// No signal, no change: in the furnace, 256 frames with the lamp, its parameters and the
/// scales unchanged open no light-change window, nor do 16 frames of camera motion; a change
/// of the lamp's intensity or of the candidate count opens one at the next frame, which
/// traces one probe in 4, and it closes after its 16 frames.
#[test]
fn no_light_change_opens_no_window() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let (world, _) = furnace_world();
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut f = FloorProbe::new(&device, BOX_MIN[0]);
    for i in 0..256 {
        f.run(&device, &queue, &gpu, 1);
        assert!(!f.frame.probe_light_window(), "frame {}", i + 1);
    }
    for i in 0..16 {
        f.camera.position.x += 0.37;
        f.camera.yaw += 0.05;
        f.run(&device, &queue, &gpu, 1);
        assert!(!f.frame.probe_light_window(), "moving, frame {}", i + 1);
    }
    f.light.intensity *= 0.5;
    f.run(&device, &queue, &gpu, 1);
    assert!(f.frame.probe_light_window(), "the lamp's intensity");
    assert_eq!(f.frame.traced_probe_rays(), u64::from(PROBE_COUNT / 4) * 64);
    f.run(&device, &queue, &gpu, WINDOW_FRAMES);
    assert!(
        !f.frame.probe_light_window(),
        "the window closes after its frames"
    );
    assert_eq!(f.frame.traced_probe_rays(), u64::from(PROBE_COUNT / 8) * 64);
    let l = f.frame.lighting();
    f.frame.set_lighting(Lighting { candidates: 4, ..l });
    f.run(&device, &queue, &gpu, 1);
    assert!(f.frame.probe_light_window(), "the candidate count");
}

/// Lamp switch, the direct light's history: in the dull box lit by the lamp from its centre
/// for 64 frames (indirect off, the filter on), the temporal pass's lamp part at the floor
/// centre is above 0; on the frame the lamp goes off it reads 0 and stays 0, and on the frame
/// it comes back on it reads that frame's light-pass lamp estimate exactly, not a blend with
/// the dark history.
#[test]
fn a_lamp_switch_drops_the_lamp_s_direct_history() {
    let world = dull_box_world();
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut f = FloorProbe::new(&device, BOX_MIN[0]);
    f.camera.position.z = 32.0;
    let on = Lighting {
        lamp: true,
        filter: true,
        indirect: false,
        ..f.frame.lighting()
    };
    let lamp_part = |f: &FloorProbe| {
        let acc = readback::read_float(&device, &queue, f.frame.accumulated())[0][3];
        let raw = readback::read_rgba16f(&device, &queue, f.frame.irradiance())[0][3];
        (acc, raw)
    };
    f.frame.set_lighting(on);
    f.run(&device, &queue, &gpu, 64);
    let (settled, _) = lamp_part(&f);
    f.frame.set_lighting(Lighting { lamp: false, ..on });
    let mut off = Vec::new();
    for _ in 0..4 {
        f.run(&device, &queue, &gpu, 1);
        off.push(lamp_part(&f).0);
    }
    f.frame.set_lighting(on);
    f.run(&device, &queue, &gpu, 1);
    let (back, raw) = lamp_part(&f);
    println!("lamp part settled {settled:.4}; off {off:?}; back on {back:.4} (estimate {raw:.4})");
    assert!(settled > 0.0, "{settled}");
    assert_eq!(off, vec![0.0; 4]);
    assert!(raw > 0.0 && back == raw, "{back} against {raw}");
}

/// The vein box: the dull box with an 8 x 8 patch of `glow` in its -x wall (the wall voxels
/// at x = 19, y and z from 28 and 27), its table's glow emitting `[1, 1, 1]`.
fn vein_box_world() -> (World, MaterialId) {
    let (table, glow, dull) = furnace_table();
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &UniformSource(dull),
        &table,
    );
    world
        .apply(&box_op(
            [19, 28, 27],
            [20, 36, 35],
            Mode::Replace {
                from: dull,
                to: glow,
            },
        ))
        .unwrap();
    world.apply(&box_op(BOX_MIN, BOX_MAX, Mode::Carve)).unwrap();
    (world, glow)
}

/// The pulse's period in frames (1.5 s at 60 frames a second) and the frames a pulse run
/// steps.
const PULSE_PERIOD: u32 = 90;
const PULSE_FRAMES: u32 = 256;

/// How a pulse run sets the vein's emission each frame.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pulse {
    /// The scale stays 1.
    Off,
    /// The pixels' scale pulses between 0.25 and 1.75 about 1; the probes' stays at 1.
    AtMean,
    /// Both scales pulse (`set_emission_scale`).
    Everywhere,
}

/// The floor beside the vein (x = 24) after each of PULSE_FRAMES frames with the lamp off and
/// the vein pulsing as `pulse` says: the gathered indirect and the temporal pass's emissive
/// direct irradiance (luminance), and whether a light-change window was open.
fn pulse_run(pulse: Pulse) -> Option<(Vec<f64>, Vec<f64>, Vec<bool>)> {
    let (world, glow) = vein_box_world();
    let (device, queue) = software_device(64 << 20)?;
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut f = FloorProbe::new(&device, 12);
    let lum = |e: [f32; 4]| {
        0.2126 * f64::from(e[0]) + 0.7152 * f64::from(e[1]) + 0.0722 * f64::from(e[2])
    };
    let (mut indirect, mut direct, mut window) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..PULSE_FRAMES {
        let s = 1.0 + 0.75 * (i as f32 / PULSE_PERIOD as f32 * std::f32::consts::TAU).sin();
        match pulse {
            Pulse::Off => f.frame.set_emission_scale(&queue, glow, 1.0),
            Pulse::AtMean => f.frame.set_emission_scales(&queue, glow, s, 1.0),
            Pulse::Everywhere => f.frame.set_emission_scale(&queue, glow, s),
        }
        f.run(&device, &queue, &gpu, 1);
        let e = f.indirect(&device, &queue);
        indirect.push(lum([e[0], e[1], e[2], 0.0]));
        direct.push(lum(readback::read_float(
            &device,
            &queue,
            f.frame.accumulated(),
        )[0]));
        window.push(f.frame.probe_light_window());
    }
    Some((indirect, direct, window))
}

/// The mean frame-to-frame change of `v` over its second half.
fn mean_step(v: &[f64]) -> f64 {
    let h = &v[v.len() / 2..];
    h.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (h.len() - 1) as f64
}

/// Max over min of `v` over its last pulse period.
fn swing(v: &[f64]) -> f64 {
    let p = &v[v.len() - PULSE_PERIOD as usize..];
    let (lo, hi) = p
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
    hi / lo
}

/// The pulse feeds the probes at its mean: in the vein box with the camera still for 256
/// frames, a vein whose pixel scale pulses while its probe scale holds at the pulse's mean
/// opens no light-change window; the floor beside it reads a gathered indirect whose
/// frame-to-frame change is no more than with the pulse off plus 1 % of that run's mean; and
/// its direct light's max-to-min ratio over the last pulse period matches, within 5 %, the run
/// whose probes pulse too.
#[test]
fn a_pulse_at_its_mean_leaves_the_probes_still() {
    let Some((ind_off, _, _)) = pulse_run(Pulse::Off) else {
        return;
    };
    let Some((ind_mean, dir_mean, window)) = pulse_run(Pulse::AtMean) else {
        return;
    };
    let Some((ind_all, dir_all, window_all)) = pulse_run(Pulse::Everywhere) else {
        return;
    };
    let half = &ind_off[ind_off.len() / 2..];
    let mean_off = half.iter().sum::<f64>() / half.len() as f64;
    let (step_off, step_mean, step_all) = (
        mean_step(&ind_off),
        mean_step(&ind_mean),
        mean_step(&ind_all),
    );
    let (swing_mean, swing_all) = (swing(&dir_mean), swing(&dir_all));
    println!(
        "indirect beside the vein: mean {mean_off:.5} with the pulse off; frame-to-frame change {step_off:.6} off, {step_mean:.6} at the mean, {step_all:.6} everywhere (windows open {} of {PULSE_FRAMES} frames)",
        window_all.iter().filter(|&&w| w).count()
    );
    println!(
        "direct swing over the last period: {swing_mean:.4} at the mean, {swing_all:.4} everywhere"
    );
    assert!(window.iter().all(|&w| !w), "{window:?}");
    assert!(mean_off > 0.0, "{mean_off}");
    assert!(
        step_mean <= step_off + 0.01 * mean_off,
        "{step_mean} against {step_off} + 1 % of {mean_off}"
    );
    assert!(swing_mean > 2.0, "{swing_mean}");
    assert!(
        (swing_mean / swing_all - 1.0).abs() <= 0.05,
        "{swing_mean} against {swing_all}"
    );
}
