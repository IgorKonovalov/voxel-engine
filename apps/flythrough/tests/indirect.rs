//! Indirect light from the probe volume (ADR-0023), on the software adapter (lavapipe).

mod support;

use support::*;
use vxe_render::GpuWorld;
use vxe_render::probes::{TestProbe, probe_slot};
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
        Self { frame, camera }
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
                .step(device, queue, gpu, &self.camera, &LightParams::headlamp());
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

/// The furnace: in the closed box of 24 x 24 x 12 interior voxels whose six walls emit
/// `[1, 1, 1]` at albedo 0.5, lamp off, the gathered indirect irradiance at the floor's centre
/// after 256 frames is within 10 % of `PI` (direct `PI L_e`, total `2 PI`), and the probe at the
/// box's centre holds within 10 % of `PI` in every interior irradiance texel. Every probe whose
/// position lies in solid reads the inside flag after its first update (8 frames).
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
    for _ in 0..31 {
        f.run(&device, &queue, &gpu, 8);
        series.push(f.indirect(&device, &queue)[0]);
    }
    let e = f.indirect(&device, &queue);
    let pi = std::f32::consts::PI;
    println!(
        "floor centre indirect after 256 frames {e:?} against {pi:.4} ({:+.2} %); every 8 frames {series:?}",
        (e[0] / pi - 1.0) * 100.0
    );
    for c in e {
        assert!((c / pi - 1.0).abs() <= 0.10, "{e:?}");
    }
    let probe = f
        .frame
        .probes()
        .read_probe(&device, &queue, probe_slot([2, 2, 2]));
    let texels = probe.interior_irradiance();
    let (lo, hi) = texels
        .iter()
        .flatten()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    println!(
        "centre probe: age {}, texels {lo:.4} ..= {hi:.4}",
        probe.age()
    );
    for t in &texels {
        for &c in t {
            assert!((c / pi - 1.0).abs() <= 0.10, "{t:?}");
        }
    }
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
