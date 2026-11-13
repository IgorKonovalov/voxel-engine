//! The probe volume (ADR-0023) as ADR-0043's lamp-bounce channel, on the software adapter
//! (lavapipe): the gather over preset probes, and the update fed by the headlamp alone.

mod support;

use support::*;
use vxe_render::GpuWorld;
use vxe_render::{LightParams, Lighting, ToneMap};
use vxe_render::{internals::TestProbe, internals::WINDOW_FRAMES};
use vxe_world::*;

/// The lighting of the probe-preset tests: no lamp, no filter, the identity tone map, the
/// channel on over the whole 32-probe volume (no reach cut; the cavity's table has no emissive
/// row). A preset volume (its updates off) is gathered with the lamp off.
fn preset_lighting(indirect: bool) -> Lighting {
    Lighting {
        lamp: false,
        filter: false,
        tone_map: ToneMap::Identity,
        lamp_bounce: indirect,
        lamp_reach: 0.0,
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
    let info = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
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

/// The lamp of [`FloorProbe::wall_lamp`]: at voxel x 30, y 32, z 37 of either box's frame,
/// pointing +x at the lit box's x = 44 wall (the divider of `two_boxes`), 14 voxels away.
const WALL_LAMP: [f32; 3] = [30.0, 32.0, 37.0];

impl FloorProbe {
    fn new(device: &vxe_render::wgpu::Device, x0: i64) -> Self {
        let mut frame = vxe_render::Frame::new(device, 1, 1);
        frame.set_resolution(device, vxe_render::LightingResolution::Full);
        frame.set_lighting(Lighting {
            lamp: false,
            filter: false,
            lamp_bounce: true,
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

    /// The lamp channel's floor probe: lamp on at [`WALL_LAMP`] (a fixed point whatever box the
    /// camera is in), the camera at the floor centre of the box from `x0` looking down, pitched
    /// 5 degrees off the vertical toward +y (yaw 90 degrees, so the camera's right is +x), the
    /// lamp turned 90 degrees onto +x. The floor centre's upper hemisphere sees the lit wall, so
    /// the channel's one bounce reaches it.
    fn wall_lamp(device: &vxe_render::wgpu::Device, x0: i64) -> Self {
        let mut f = Self::new(device, x0);
        f.frame.set_lighting(Lighting {
            lamp: true,
            ..f.frame.lighting()
        });
        f.camera.yaw = std::f32::consts::FRAC_PI_2;
        f.camera.pitch = -std::f32::consts::FRAC_PI_2 + 5f32.to_radians();
        let (_, right, up) = f.camera.basis();
        let to = vxe_render::glam::Vec3::from(WALL_LAMP) - f.camera.position;
        f.light = LightParams {
            offset_right: to.dot(right),
            offset_down: -to.dot(up),
            turn: std::f32::consts::FRAC_PI_2,
            ..LightParams::headlamp()
        };
        f
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
        let e = vxe_render::debug::read_rgba16f(device, queue, self.frame.indirect_irradiance())[0];
        [e[0], e[1], e[2]]
    }
}

/// The lamp furnace: the dull box (albedo 0.5, nothing emissive) under a 1 x 1 frame at the
/// centre of voxel (32, 32, 32), its centre probe `(2, 2, 2)`'s position, with the lamp at the
/// eye lighting every direction (cone 172 to 180 degrees, no shadow ray: from the eye every
/// lit point is visible), the channel over the whole volume, its freeze at the cap.
fn lamp_furnace(device: &vxe_render::wgpu::Device) -> FloorProbe {
    let mut f = FloorProbe::new(device, BOX_MIN[0]);
    f.frame.set_lighting(Lighting {
        lamp: true,
        lamp_reach: 0.0,
        probe_freeze: vxe_render::internals::MAX_AGE,
        ..f.frame.lighting()
    });
    f.camera.position = [32.5, 32.5, 32.5].into();
    f.light = LightParams {
        inner: 3.0,
        outer: std::f32::consts::PI,
        ..LightParams::at_eye()
    };
    f
}

/// `octa_unfold` of `hitrec.wgsl`: the unit direction at octahedral point `p` in `[-1, 1]^2`.
fn octa_unfold(p: [f64; 2]) -> [f64; 3] {
    let mut d = [p[0], p[1], 1.0 - p[0].abs() - p[1].abs()];
    if d[2] < 0.0 {
        let s = [p[0], p[1]].map(|v| if v >= 0.0 { 1.0 } else { -1.0 });
        d = [(1.0 - d[1].abs()) * s[0], (1.0 - d[0].abs()) * s[1], d[2]];
    }
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    d.map(|v| v / l)
}

/// The furnace's `D_t` for each of the 36 interior irradiance texels (row by row), from the
/// box's geometry alone: from the centre probe at `o`, each of `n` spherical Fibonacci
/// directions `w` meets a wall of the box interior `[BOX_MIN, BOX_MAX]` at `p` with inward
/// normal `m`; `E(w)` is `light`'s irradiance there (`Light::irradiance`, the CPU form of the
/// shader's term: `PI * intensity * cone * max(m . l, 0) / d^2` times its window at `floor`).
/// `D_t = sum(max(0, d_t . w) E(w)) / sum(max(0, d_t . w))` with `d_t` the texel's direction:
/// the normalisation the probe blend's `PI sum(L c) / sum(c)` gives for `L = albedo / PI E`,
/// so the texel's fixed point is `albedo D_t`. No value read back from the shader enters it.
fn furnace_reference(light: &vxe_render::Light, floor: f32, o: [f64; 3], n: u32) -> Vec<f64> {
    let (lo, hi) = (BOX_MIN.map(|v| v as f64), BOX_MAX.map(|v| v as f64));
    let golden = std::f64::consts::PI * (3.0 - 5f64.sqrt());
    let samples: Vec<([f64; 3], f64)> = (0..n)
        .map(|i| {
            let z = 1.0 - (2.0 * f64::from(i) + 1.0) / f64::from(n);
            let r = (1.0 - z * z).max(0.0).sqrt();
            let phi = f64::from(i) * golden;
            let w = [r * phi.cos(), r * phi.sin(), z];
            let (t, axis) = (0..3)
                .filter(|&k| w[k] != 0.0)
                .map(|k| {
                    let b = if w[k] > 0.0 { hi[k] } else { lo[k] };
                    ((b - o[k]) / w[k], k)
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .expect("a wall");
            let p: [f32; 3] = std::array::from_fn(|k| (o[k] + w[k] * t) as f32);
            let mut m = [0.0f32; 3];
            m[axis] = if w[axis] > 0.0 { -1.0 } else { 1.0 };
            (w, f64::from(light.irradiance(floor, p, m)))
        })
        .collect();
    (0..36)
        .map(|t| {
            let c = [(t % 6) as f64, (t / 6) as f64].map(|v| (v + 0.5) / 6.0 * 2.0 - 1.0);
            let d = octa_unfold(c);
            let (mut sum, mut wsum) = (0.0, 0.0);
            for (w, e) in &samples {
                let c = (d[0] * w[0] + d[1] * w[1] + d[2] * w[2]).max(0.0);
                sum += c * e;
                wsum += c;
            }
            sum / wsum
        })
        .collect()
}

/// A furnace's reading: after 256 frames (32 visits of the running mean at the defaults) the
/// centre probe `(2, 2, 2)`'s interior texels (red) against `0.5 D_t` of
/// [`furnace_reference`] over 16384 directions; returns the texel mean, `0.5` times the mean
/// `D_t`, and the worst texel's ratio less one.
fn furnace_reading(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    gpu: &GpuWorld,
    f: &mut FloorProbe,
    light: &vxe_render::Light,
    name: &str,
) -> (f64, f64, f64) {
    f.run(device, queue, gpu, 256);
    let slot = f.frame.probes().slot([2, 2, 2]);
    let probe = f.frame.probes().read_probe(device, queue, slot);
    assert!(!probe.inside() && probe.age() >= 30, "age {}", probe.age());
    let m: Vec<f64> = probe
        .interior_irradiance()
        .iter()
        .map(|t| f64::from(t[0]))
        .collect();
    let o = f.frame.probes().probe_position([2, 2, 2]).map(f64::from);
    let floor = f.frame.lighting().light_floor;
    let d = furnace_reference(light, floor, o, 16384);
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let (mm, dm) = (mean(&m), mean(&d));
    let worst = m
        .iter()
        .zip(&d)
        .map(|(a, b)| (a / (0.5 * b) - 1.0).abs())
        .fold(0.0f64, f64::max);
    println!(
        "{name} furnace: centre probe age {} after 256 frames; texel mean {mm:.4}, 0.5 D_ref {:.4} ({:+.2} %); worst texel {:.2} %; D_t {:.3} ..= {:.3}",
        probe.age(),
        0.5 * dm,
        (mm / (0.5 * dm) - 1.0) * 100.0,
        worst * 100.0,
        d.iter().copied().fold(f64::MAX, f64::min),
        d.iter().copied().fold(f64::MIN, f64::max)
    );
    (mm, 0.5 * dm, worst)
}

/// The lamp furnace's fixed point (M2's headless proof, restated for the lamp channel): the
/// probe rays shade a hit with `albedo / PI E_lamp` and nothing else, so after 256 frames the
/// centre probe's texel mean reads within 4 % of `0.5 D_ref` and every texel within 7 % of its
/// `0.5 D_t`, `D_ref` the mean `D_t` of [`furnace_reference`] for the lamp's slot light (a
/// spot whose outer cosine is -1: every direction lit).
#[test]
#[ignore = "soak"]
fn the_lamp_furnace_reads_its_albedo_times_the_direct_mean() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let world = dull_box_world();
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut f = lamp_furnace(&device);
    let lamp = vxe_render::Light::headlamp(&f.camera, &f.light);
    let (mm, half_d, worst) = furnace_reading(&device, &queue, &gpu, &mut f, &lamp, "lamp");
    assert!(half_d > 0.0, "{half_d}");
    assert!((mm / half_d - 1.0).abs() <= 0.04, "{mm} against {half_d}");
    assert!(worst <= 0.07, "worst texel {worst}");
}

/// A flagged list light's furnace: the lamp furnace's box and volume with the lamp off, one
/// white point light of 1200 at the centre probe with `BOUNCE` set (no shadow ray: every wall
/// point is visible from it) and `light_floor` 0; the centre probe's texel mean reads within
/// 4 % of `0.5 D_ref`, [`furnace_reference`]'s mean for that light.
#[test]
#[ignore = "soak"]
fn a_flagged_lights_furnace_reads_its_albedo_times_the_analytic_direct_mean() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let world = dull_box_world();
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut f = lamp_furnace(&device);
    f.frame.set_lighting(Lighting {
        lamp: false,
        light_floor: 0.0,
        ..f.frame.lighting()
    });
    let light = vxe_render::Light {
        flags: vxe_render::Light::ON | vxe_render::Light::BOUNCE,
        ..vxe_render::Light::point([32.5; 3], 1200.0)
    };
    f.frame.set_lights(&[light]);
    let (mm, half_d, worst) = furnace_reading(&device, &queue, &gpu, &mut f, &light, "list light");
    assert!(half_d > 0.0, "{half_d}");
    assert!((mm / half_d - 1.0).abs() <= 0.04, "{mm} against {half_d}");
    println!("worst texel {:.2} % (a reading)", worst * 100.0);
}

/// The channel holds the lamp's bounce and nothing else: in the furnace (its six walls emit
/// `[1, 1, 1]`) under the wall lamp, with the lamp off every probe texel and state word reads
/// exactly 0 over 64 frames (no pass runs); with the lamp on, every probe within the lamp's
/// reach reads, after 64 frames, within 10 % of the same run in the dull box (the walls'
/// emission 0), texel for texel.
#[test]
fn the_channel_holds_the_lamps_bounce_only() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let words = |f: &FloorProbe| -> Vec<f32> {
        let v = f.frame.probes();
        (0..v.count())
            .flat_map(|s| {
                let p = v.read_probe(&device, &queue, s);
                p.irradiance
                    .iter()
                    .flatten()
                    .copied()
                    .chain(std::iter::once(p.state as f32))
                    .collect::<Vec<f32>>()
            })
            .collect()
    };
    let (glow_world, _) = furnace_world();
    let mut glow = GpuWorld::new(&device, glow_world.extent());
    glow.sync_now(&glow_world, &device, &queue).unwrap();
    let mut off = FloorProbe::wall_lamp(&device, BOX_MIN[0]);
    off.frame.set_lighting(Lighting {
        lamp: false,
        ..off.frame.lighting()
    });
    for i in 0..64 {
        off.run(&device, &queue, &glow, 1);
        if i % 16 == 15 {
            assert!(
                words(&off).iter().all(|&w| w == 0.0),
                "lamp off: a probe word moved by frame {}",
                i + 1
            );
            assert_eq!(off.frame.traced_probe_rays(), 0);
        }
    }
    let dull_world = dull_box_world();
    let mut dull = GpuWorld::new(&device, dull_world.extent());
    dull.sync_now(&dull_world, &device, &queue).unwrap();
    let mut lit = FloorProbe::wall_lamp(&device, BOX_MIN[0]);
    let mut dark = FloorProbe::wall_lamp(&device, BOX_MIN[0]);
    lit.run(&device, &queue, &glow, 64);
    dark.run(&device, &queue, &dull, 64);
    let v = lit.frame.probes();
    let origin = v.origin_for(lit.camera.position.to_array());
    let reach = lit.frame.lighting().lamp_reach;
    let lamp = vxe_render::glam::Vec3::from(WALL_LAMP);
    let (mut compared, mut worst, mut lit_texels) = (0, 0.0f32, 0);
    for s in 0..v.count() {
        let w = v.slot_probe(origin, s);
        if (vxe_render::glam::Vec3::from(v.probe_position(w)) - lamp).length() > reach {
            continue;
        }
        let a = v.read_probe(&device, &queue, s);
        let b = dark.frame.probes().read_probe(&device, &queue, s);
        assert_eq!(a.state, b.state, "slot {s}");
        for (x, y) in a.interior_irradiance().iter().zip(b.interior_irradiance()) {
            for k in 0..3 {
                let d = (x[k] - y[k]).abs();
                lit_texels += usize::from(y[k] > 0.0);
                worst = worst.max(if y[k] > 0.0 { d / y[k] } else { d });
                assert!(d <= 0.1 * y[k].abs(), "slot {s}: {x:?} against {y:?}");
            }
        }
        compared += 1;
    }
    println!(
        "lamp off: every word 0 over 64 frames; lamp on: {compared} probes in reach compared, {lit_texels} lit texel channels, worst relative difference {worst}"
    );
    assert!(compared > 0 && lit_texels > 0);
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

/// The two boxes under the wall lamp in the lit box, aimed at the divider: after 256 frames
/// the dark box's floor-centre gather is under 2 % of the lit box's (no leak through the wall:
/// the dark box's probes see no lamp light, and the lit box's are not gathered through it).
#[test]
#[ignore = "soak"]
fn no_light_leaks_through_a_two_voxel_wall() {
    let world = two_boxes();
    let full = world.stats().full_bricks as u64;
    let Some((device, queue)) = software_device(((full + 4096) * 512).max(64 << 20)) else {
        return;
    };
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut lit = FloorProbe::wall_lamp(&device, BOX_MIN[0]);
    let mut dark = FloorProbe::wall_lamp(&device, DARK_X0);
    lit.run(&device, &queue, &gpu, 256);
    dark.run(&device, &queue, &gpu, 256);
    let l = lit.indirect(&device, &queue)[0];
    let d = dark.indirect(&device, &queue)[0];
    println!(
        "lit floor {l:.4}, dark floor {d:.6}: {:.4} % of the lit",
        d / l * 100.0
    );
    assert!(l > 0.0, "{l}");
    assert!(d < 0.02 * l, "{d} against {l}");
}

/// An inside probe's texels are never read by the gather: after 64 frames of the wall lamp in
/// the furnace, with the update off, every probe flagged inside (the chunk's 63 in solid, all
/// within the lamp's reach) is overwritten with irradiance 1000 and the floor centre's indirect
/// irradiance reads exactly what it read before.
#[test]
fn an_inside_probe_s_texels_are_never_gathered() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let (world, _) = furnace_world();
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut f = FloorProbe::wall_lamp(&device, BOX_MIN[0]);
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
    assert!(before[0] > 0.0, "{before:?}");
    assert_eq!(before, after);
}

// ---- Scrolling and edits ----

/// After 16 updated frames at the cavity's centre (the lamp on, the channel over the whole
/// 32-probe volume), a camera jump of 20 probe steps along x (the update off for the jump's
/// frame) leaves the 12 x 32 x 32 probes that stay in the
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
        lamp_bounce: true,
        lamp_reach: 0.0,
        ..Lighting::default()
    });
    let light = LightParams::headlamp();
    for _ in 0..16 {
        rig.frame
            .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
    }
    let probes = rig.frame.probes();
    let before_origin = probes.origin_for(camera.position.to_array());
    let before = vxe_render::debug::read_buffer(
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
    let after = vxe_render::debug::read_buffer(
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
                let slot = probes.slot(w);
                if w[0] < before_origin[0] + 32 {
                    assert_eq!(
                        probe_bytes(&after, slot),
                        probe_bytes(&before, slot),
                        "probe {w:?}"
                    );
                    stayed += 1;
                    live_before += u32::from(probes.age(states[slot as usize], w) > 0);
                } else {
                    let age = probes.age(states[slot as usize], w);
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

/// Slot `slot`'s irradiance tile, visibility tile and state word, as bytes of the 32-probe
/// volume's data buffer read back whole in `data`.
fn probe_bytes(data: &[u8], slot: u32) -> Vec<u8> {
    let count = 32 * 32 * 32;
    let (vis_base, state_base) = (count * 128, count * 128 + count * 256);
    let s = slot as usize;
    let mut out = data[s * 512..(s + 1) * 512].to_vec();
    let v = vis_base * 4 + s * 1024;
    out.extend_from_slice(&data[v..v + 1024]);
    let st = state_base * 4 + s * 4;
    out.extend_from_slice(&data[st..st + 4]);
    out
}

/// Frames after the carve at which the hole test reads the dark box's settled reference: 256
/// probe visits, by which the bounce series has converged (256 frames after the carve it still
/// rises by about 12 %).
const HOLE_SETTLED_FRAMES: usize = 2048;

/// In the two-box scene under the wall lamp (in the lit box, aimed at the divider) after 256
/// frames, a radius-4 carve through the dividing wall, inside the lamp's beam, makes the dark
/// box's floor-centre indirect irradiance reach, within 32 frames, at least 52 % of its
/// settled value (its reading [`HOLE_SETTLED_FRAMES`] frames after the carve) and at least 10
/// times its pre-carve value (the probes the edit box touches refresh at once, the rest in
/// their slices). The 52 % is the probe update with its slices taken by `slot mod slices`
/// (53.1 % on lavapipe) less a point for run-to-run noise, so an update slower to follow a
/// carve fails. Filling the hole back with the wall's rock drops it under 5 % of the lit box's
/// within 64 frames.
#[test]
#[ignore = "soak"]
fn a_hole_in_the_wall_lights_the_dark_box_and_filling_it_darkens_it_again() {
    let world = two_boxes();
    let full = world.stats().full_bricks as u64;
    let Some((device, queue)) = software_device(((full + 4096) * 512).max(64 << 20)) else {
        return;
    };
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut lit = FloorProbe::wall_lamp(&device, BOX_MIN[0]);
    let mut dark = FloorProbe::wall_lamp(&device, DARK_X0);
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

/// The probe settings scale the update's dispatch: in the furnace under the wall lamp, the
/// channel over the whole 32-probe volume (no reach cut), the default (64 rays, one probe in 8)
/// dispatches 4096 x 64 rays a frame after the start's window, 32 rays half that, one in 16
/// half that, and both a quarter; at 32 rays and one in 16 the box's centre probe is live after
/// 16 frames, its texels toward the lit wall above 0.
#[test]
fn the_probe_settings_scale_the_rays_a_frame_traces() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let (world, _) = furnace_world();
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut f = FloorProbe::wall_lamp(&device, BOX_MIN[0]);
    f.frame.set_lighting(Lighting {
        lamp_reach: 0.0,
        ..f.frame.lighting()
    });
    f.run(&device, &queue, &gpu, WINDOW_FRAMES);
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
    let slot = f.frame.probes().slot([2, 2, 2]);
    let probe = f.frame.probes().read_probe(&device, &queue, slot);
    let hi = probe
        .interior_irradiance()
        .iter()
        .flatten()
        .fold(0.0f32, |a, &v| a.max(v));
    println!(
        "at 32 rays, one in 16: centre probe age {}, brightest texel {hi:.4} (toward the lit wall)",
        probe.age()
    );
    assert!(!probe.inside() && probe.age() >= 1);
    assert!(hi > 0.0, "{hi}");
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

/// The dull box under the wall lamp for 256 frames; then the lamp switched off through
/// `Frame::set_lighting` for `frames` frames, and on again for `frames` more: the floor-centre
/// indirect before the switch, and after each switch at each frame the floor-centre indirect,
/// the frame's output pixel (its brightest channel), whether the frame's update ran inside the
/// light-change window, and the probe dispatches the frame recorded.
fn lamp_switch_run(frames: u32) -> Option<(f32, [SwitchSeries; 2])> {
    let world = dull_box_world();
    let (device, queue) = software_device(64 << 20)?;
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut f = FloorProbe::wall_lamp(&device, BOX_MIN[0]);
    let on = f.frame.lighting();
    f.run(&device, &queue, &gpu, 256);
    let f0 = f.indirect(&device, &queue)[0];
    let mut run = |lamp: bool| {
        f.frame.set_lighting(Lighting { lamp, ..on });
        let mut s = SwitchSeries::default();
        for _ in 0..frames {
            f.run(&device, &queue, &gpu, 1);
            s.indirect.push(f.indirect(&device, &queue)[0]);
            let px = vxe_render::debug::read_rgba8(&device, &queue, f.frame.target());
            s.output.push(px[..3].iter().copied().max().unwrap_or(0));
            s.window.push(f.frame.probe_light_window());
            s.dispatches.push(f.frame.probe_dispatches());
        }
        s
    };
    let off = run(false);
    let back = run(true);
    Some((f0, [off, back]))
}

/// One switch's frames in [`lamp_switch_run`].
#[derive(Default)]
struct SwitchSeries {
    indirect: Vec<f32>,
    output: Vec<u8>,
    window: Vec<bool>,
    dispatches: Vec<u32>,
}

/// Lamp off, then on: in the closed dull box (24 x 24 x 12 interior voxels, albedo 0.5, nothing
/// emissive) under the wall lamp at the defaults (64 rays, one probe in 8), the floor-centre
/// indirect `F0` after 256 frames is above 0. With the lamp switched off no probe pass and no
/// gather is recorded and the composite adds no bounce: the floor pixel reads black from the
/// first frame (nothing else lights the box), with no window. Switched back on, the channel fills from zero: the light-change window opens for
/// exactly its 16 frames, and the floor reaches 90 % of `F0` within them.
#[test]
#[ignore = "soak"]
fn switching_the_lamp_off_drops_the_probe_history() {
    let Some((f0, [off, on])) = lamp_switch_run(48) else {
        return;
    };
    let w = WINDOW_FRAMES as usize;
    println!(
        "lamp off: F0 {f0:.5}; off, the output pixel {:?}; back on, frames 1..=24 {:?}; first at 90 %: {:?}",
        &off.output[..8],
        &on.indirect[..24],
        on.indirect
            .iter()
            .position(|&v| v >= 0.9 * f0)
            .map(|i| i + 1)
    );
    assert!(f0 > 0.0, "{f0}");
    assert!(off.output.iter().all(|&v| v == 0), "{:?}", off.output);
    assert!(off.window.iter().all(|&o| !o), "{:?}", off.window);
    assert!(
        off.dispatches.iter().all(|&d| d == 0),
        "{:?}",
        off.dispatches
    );
    assert_eq!(on.window[..w], vec![true; w], "{:?}", on.window);
    assert!(on.window[w..].iter().all(|&o| !o), "{:?}", on.window);
    assert!(
        on.indirect[..w].iter().any(|&v| v >= 0.9 * f0),
        "{:?} against F0 {f0}",
        on.indirect
    );
}

/// The lamp off encodes no probe pass: under the wall lamp with the lamp off for 16 frames, the
/// frame records no probe trace, blend or gather dispatch, traces no ray, and (where the
/// adapter has timestamps) the `probes` and `gather` passes' timers read 0; with the lamp on the
/// same frame records all three.
#[test]
fn the_lamp_off_encodes_no_probe_pass() {
    use vxe_render::FRAME_PASSES;
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let world = dull_box_world();
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut f = FloorProbe::wall_lamp(&device, BOX_MIN[0]);
    let timer = vxe_render::PassTimer::with_passes(&device, &queue, FRAME_PASSES.len() as u32);
    let slot = |name: &str| FRAME_PASSES.iter().position(|&n| n == name).expect(name);
    let (probes, gather) = (slot("probes"), slot("gather"));
    let step = |f: &mut FloorProbe| -> Option<(f64, f64)> {
        f.frame.advance();
        let params = f.frame.params(&gpu, &f.camera, &f.light);
        let mut enc = device.create_command_encoder(&Default::default());
        f.frame
            .encode(&device, &queue, &mut enc, &gpu, &params, timer.as_ref());
        if let Some(t) = &timer {
            t.resolve(&mut enc);
        }
        queue.submit([enc.finish()]);
        timer.as_ref().map(|t| {
            let ms = t.read_all_ms(&device);
            (ms[probes], ms[gather])
        })
    };
    let lit = f.frame.lighting();
    f.frame.set_lighting(Lighting { lamp: false, ..lit });
    let mut off_ms = Vec::new();
    for i in 0..16 {
        off_ms.extend(step(&mut f));
        assert_eq!(f.frame.probe_dispatches(), 0, "frame {}", i + 1);
        assert_eq!(f.frame.traced_probe_rays(), 0, "frame {}", i + 1);
    }
    f.frame.set_lighting(lit);
    let on_ms = step(&mut f);
    let on_dispatches = f.frame.probe_dispatches();
    println!(
        "lamp off: probes and gather ms per frame {off_ms:?}; lamp on: {on_dispatches} dispatches, {on_ms:?} ms"
    );
    assert!(
        off_ms.iter().all(|&(p, g)| p == 0.0 && g == 0.0),
        "{off_ms:?}"
    );
    assert_eq!(on_dispatches, 3, "trace, blend and gather");
}

/// No lamp change, no window: in the furnace under the wall lamp over the whole 32-probe
/// volume (no reach cut), the channel's start opens the light-change window for its 16 frames
/// and the 240 still frames after it open none, nor do 16 frames of camera motion (the lamp
/// moves with the camera) or a change of the candidate count (the probes see no emissive
/// light); a change of the lamp's intensity opens one at the next frame, which traces one
/// probe in 4, and it closes after its 16 frames, one probe in 8 a frame again.
#[test]
#[ignore = "soak"]
fn no_light_change_opens_no_window() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let (world, _) = furnace_world();
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut f = FloorProbe::wall_lamp(&device, BOX_MIN[0]);
    f.frame.set_lighting(Lighting {
        lamp_reach: 0.0,
        ..f.frame.lighting()
    });
    let count = 32 * 32 * 32u64;
    for i in 0..256 {
        f.run(&device, &queue, &gpu, 1);
        assert_eq!(
            f.frame.probe_light_window(),
            i < WINDOW_FRAMES,
            "frame {}",
            i + 1
        );
    }
    for i in 0..16 {
        f.camera.position.x += 0.37;
        f.camera.yaw += 0.05;
        f.run(&device, &queue, &gpu, 1);
        assert!(!f.frame.probe_light_window(), "moving, frame {}", i + 1);
    }
    let l = f.frame.lighting();
    f.frame.set_lighting(Lighting { candidates: 4, ..l });
    f.run(&device, &queue, &gpu, 1);
    assert!(!f.frame.probe_light_window(), "the candidate count");
    f.light.intensity *= 0.5;
    f.run(&device, &queue, &gpu, 1);
    assert!(f.frame.probe_light_window(), "the lamp's intensity");
    assert_eq!(f.frame.traced_probe_rays(), count / 4 * 64);
    f.run(&device, &queue, &gpu, WINDOW_FRAMES);
    assert!(
        !f.frame.probe_light_window(),
        "the window closes after its frames"
    );
    assert_eq!(f.frame.traced_probe_rays(), count / 8 * 64);
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
        lamp_bounce: false,
        ..f.frame.lighting()
    };
    let lamp_part = |f: &FloorProbe| {
        let acc =
            vxe_render::debug::read_float(&device, &queue, f.frame.direct_accumulated())[0][3];
        let raw = vxe_render::debug::read_rgba16f(&device, &queue, f.frame.direct())[0][3];
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

// ---- The bounce flag: list lights in the channel (ADR-0044) ----

/// The long light box of `support::lights` (124 x 60 x 48 interior voxels, no emitter) synced
/// to a mirror on the software adapter.
fn bounce_box() -> Option<(vxe_render::wgpu::Device, vxe_render::wgpu::Queue, GpuWorld)> {
    let world = support::lights::light_box();
    let (device, queue) = software_device(128 << 20)?;
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    Some((device, queue, gpu))
}

/// The bounce box's eye, near its centre, and its 1 x 1 frame looking down: lamp off, filter
/// off, the lamp's bounce on (the channel's switch), the default bounce reach (64 voxels: an
/// 8-probe volume at the 16-voxel spacing), freezing at `freeze` visits.
const BOUNCE_EYE: [f32; 3] = [64.0, 32.0, 26.0];

fn bounce_frame(device: &vxe_render::wgpu::Device, freeze: u32) -> FloorProbe {
    let mut f = FloorProbe::new(device, 52);
    f.frame.set_lighting(Lighting {
        probe_freeze: freeze,
        ..f.frame.lighting()
    });
    f.camera.position = BOUNCE_EYE.into();
    f
}

/// A white point light of 1200 at `pos`, its bounce flag `bounce`.
fn list_light(pos: [f32; 3], bounce: bool) -> vxe_render::Light {
    let l = vxe_render::Light::point(pos, 1200.0);
    vxe_render::Light {
        flags: l.flags | if bounce { vxe_render::Light::BOUNCE } else { 0 },
        ..l
    }
}

/// The world probe each slot of `f`'s volume holds for its eye.
fn slot_probes(f: &FloorProbe) -> Vec<[i32; 3]> {
    let v = f.frame.probes();
    let origin = v.origin_for(f.camera.position.to_array());
    (0..v.count()).map(|s| v.slot_probe(origin, s)).collect()
}

/// An unflagged list light leaves the channel off and a flagged one feeds it: in the bounce
/// box with the lamp off and one list light at the box's centre, with `BOUNCE` clear no probe
/// pass is recorded on any of 64 frames and every probe word reads exactly 0; with `BOUNCE` set
/// the probe nearest the light reads a texel mean above 0 after 16 frames.
#[test]
fn a_flagged_light_feeds_the_channel_and_an_unflagged_one_does_not() {
    let Some((device, queue, gpu)) = bounce_box() else {
        return;
    };
    let mut f = bounce_frame(&device, vxe_render::Lighting::default().probe_freeze);
    let centre = [64.0, 32.0, 26.0];
    f.frame.set_lights(&[list_light(centre, false)]);
    for i in 0..64 {
        f.run(&device, &queue, &gpu, 1);
        assert_eq!(f.frame.probe_dispatches(), 0, "frame {i}: no probe pass");
    }
    let words = support::lights::probe_words(&device, &queue, &f.frame);
    assert!(
        words
            .iter()
            .all(|w| w.state == 0 && w.irradiance.iter().chain(&w.visibility).all(|&x| x == 0)),
        "every probe word 0"
    );
    f.frame.set_lights(&[list_light(centre, true)]);
    f.run(&device, &queue, &gpu, 16);
    assert!(f.frame.probe_dispatches() > 0);
    let v = f.frame.probes();
    let w = [4, 2, 2];
    let p = v.read_probe(&device, &queue, v.slot(w));
    let mean = p
        .interior_irradiance()
        .iter()
        .map(|t| t[0] + t[1] + t[2])
        .sum::<f32>()
        / 36.0;
    println!(
        "probe {w:?} at {:?}: age {}, texel mean {mean}",
        v.probe_position(w),
        p.age()
    );
    assert!(!p.inside() && mean > 0.0, "{mean}");
}

/// The bounce reach cuts the list: a flagged light of 1200 (direct reach 109.5) leaves a probe
/// between 70 and 100 voxels from it (beyond the bounce reach of 64) at 0 and never visited
/// over 64 frames, while the probes within the reach are visited; a flagged light outside the
/// volume by more than the bounce reach records no probe pass.
#[test]
fn a_light_beyond_the_bounce_reach_lists_no_probe() {
    let Some((device, queue, gpu)) = bounce_box() else {
        return;
    };
    let mut f = bounce_frame(&device, vxe_render::Lighting::default().probe_freeze);
    let at = [40.0, 32.0, 26.0];
    f.frame.set_lights(&[list_light(at, true)]);
    f.run(&device, &queue, &gpu, 64);
    let v = f.frame.probes();
    let probes = slot_probes(&f);
    let d = |w: [i32; 3]| support::lights::dist(v.probe_position(w), at);
    let far: Vec<usize> = (0..probes.len())
        .filter(|&s| (70.0..100.0).contains(&d(probes[s])))
        .collect();
    let words = support::lights::probe_words(&device, &queue, &f.frame);
    let near_visited = (0..probes.len())
        .filter(|&s| d(probes[s]) <= 64.0 && words[s].state != 0)
        .count();
    println!(
        "{} probes 70 to 100 voxels from the light; {near_visited} within 64 visited",
        far.len()
    );
    assert!(!far.is_empty() && near_visited > 0);
    for &s in &far {
        assert_eq!(
            words[s].state,
            0,
            "slot {s} at {:.1} never visited",
            d(probes[s])
        );
        assert!(words[s].irradiance.iter().all(|&x| x == 0), "slot {s} at 0");
    }
    // The volume spans 8 probes of 16 voxels from the eye's cell; 70 voxels past its +x side.
    let last = v.probe_position(probes.iter().fold([i32::MIN; 3], |m, w| {
        std::array::from_fn(|k| m[k].max(w[k]))
    }));
    f.frame
        .set_lights(&[list_light([last[0] + 70.0, 32.0, 26.0], true)]);
    for i in 0..4 {
        f.run(&device, &queue, &gpu, 1);
        assert_eq!(f.frame.probe_dispatches(), 0, "frame {i}: no probe pass");
    }
}

/// The bounce box under a flagged light at `a` until frozen at `freeze` visits, then moved to
/// `b` for `slices` frames (every probe's slice once): the words before the move, after
/// `slices` more still frames, and after the move.
type MoveWords = [Vec<support::lights::ProbeWords>; 3];

fn move_run(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    gpu: &GpuWorld,
    f: &mut FloorProbe,
    a: [f32; 3],
    b: [f32; 3],
) -> MoveWords {
    let slices = f.frame.lighting().slices();
    let freeze = f.frame.lighting().probe_freeze;
    f.frame.set_lights(&[list_light(a, true)]);
    // The window's 16 frames (4 visits), then a visit per `slices` frames to the freeze.
    f.run(device, queue, gpu, WINDOW_FRAMES + slices * freeze + 1);
    let frozen = support::lights::probe_words(device, queue, &f.frame);
    f.run(device, queue, gpu, slices);
    let still = support::lights::probe_words(device, queue, &f.frame);
    f.frame.set_lights(&[list_light(b, true)]);
    f.run(device, queue, gpu, slices);
    let moved = support::lights::probe_words(device, queue, &f.frame);
    [frozen, still, moved]
}

/// Checks a [`move_run`]: still, every word is bit-identical; after the move every live probe
/// within the bounce reach of the new position was traced again (its age restarted at 1: a
/// moved probe's fresh mean, or a probe that entered the reach) and every probe beyond the
/// reach of both positions is bit-identical. Returns the counts (live within the new reach,
/// live within the old reach only and how many of those changed, beyond both, of the first
/// how many changed their irradiance, and the inside probes within the new reach and how many
/// of them were traced, their state word rewritten).
fn check_move(f: &FloorProbe, w: &MoveWords, a: [f32; 3], b: [f32; 3]) -> [usize; 7] {
    let [frozen, still, moved] = w;
    assert!(frozen == still, "frozen: every word bit-identical");
    let v = f.frame.probes();
    let probes = slot_probes(f);
    let reach = f.frame.lighting().lamp_reach;
    let mut n = [0; 7];
    for (s, &p) in probes.iter().enumerate() {
        let q = v.probe_position(p);
        let (near_a, near_b) = (
            support::lights::dist(q, a) <= reach,
            support::lights::dist(q, b) <= reach,
        );
        let live = v.live(frozen[s].state, p);
        if near_b && live {
            n[0] += 1;
            assert_eq!(
                v.age(moved[s].state, p),
                1,
                "slot {s} in the new reach restarted its mean"
            );
            n[4] += usize::from(moved[s].irradiance != frozen[s].irradiance);
        } else if near_a && live {
            n[1] += 1;
            n[2] += usize::from(moved[s].irradiance != frozen[s].irradiance);
        } else if near_b && frozen[s].state & 1 != 0 {
            // An inside probe the move listed: MOVED_BIT skips the idle return, so it is traced.
            n[5] += 1;
            n[6] += usize::from(moved[s].state != frozen[s].state);
        } else if !near_a && !near_b {
            n[3] += 1;
            assert_eq!(moved[s], frozen[s], "slot {s} beyond both reaches stayed");
        }
    }
    println!(
        "moved {a:?} -> {b:?}: {} live probes in the new reach restarted, {} of them changed their irradiance; {} live in the old reach only, {} of them changed; {} beyond both unchanged",
        n[0], n[4], n[1], n[2], n[3]
    );
    println!(
        "inside probes in the new reach: {}, traced on the move (state rewritten): {}",
        n[5], n[6]
    );
    assert!(n[0] > 0 && n[3] > 0);
    n
}

/// The gate's half of the freeze: at `probe_freeze` 4 a flagged list light, frozen, moved 10
/// voxels along the box wakes the probes within the bounce reach of its new position within
/// one round of the slices and leaves the probes beyond both reaches bit-identical.
#[test]
fn a_moved_flagged_light_wakes_the_probes_in_reach_only() {
    let Some((device, queue, gpu)) = bounce_box() else {
        return;
    };
    let mut f = bounce_frame(&device, 4);
    let (a, b) = ([30.0, 32.0, 26.0], [40.0, 32.0, 26.0]);
    let w = move_run(&device, &queue, &gpu, &mut f, a, b);
    check_move(&f, &w, a, b);
}

/// The channel freezes when every flagged light is still: at `probe_freeze` 8 the words are
/// bit-identical once frozen, the move wakes the probes in reach only, and once the light is
/// still again they refreeze and hold bit-identical over 64 frames.
#[test]
#[ignore = "soak"]
fn the_channel_freezes_when_every_flagged_light_is_still() {
    let Some((device, queue, gpu)) = bounce_box() else {
        return;
    };
    let mut f = bounce_frame(&device, 8);
    let (a, b) = ([30.0, 32.0, 26.0], [40.0, 32.0, 26.0]);
    let w = move_run(&device, &queue, &gpu, &mut f, a, b);
    check_move(&f, &w, a, b);
    let slices = f.frame.lighting().slices();
    f.run(&device, &queue, &gpu, slices * 8 + 1);
    let refrozen = support::lights::probe_words(&device, &queue, &f.frame);
    f.run(&device, &queue, &gpu, 64);
    assert!(
        support::lights::probe_words(&device, &queue, &f.frame) == refrozen,
        "refrozen: bit-identical over 64 frames"
    );
}

mod reading {
    use super::*;

    /// The bounce's lag behind a light in flight (a reading, no bar): a flagged light of 1200
    /// frozen at (30, 32, 26) in the bounce box at the default freeze and `probe_move_blend`,
    /// then flown 4 voxels a frame along +x for 16 frames to (94, 32, 26); the frames from its
    /// last move until the texel mean of the live probe nearest the end is within 10 % of its
    /// value 256 frames later (frozen).
    #[test]
    #[ignore]
    fn reading_the_bounce_lag_behind_a_light_in_flight() {
        let Some((device, queue, gpu)) = bounce_box() else {
            return;
        };
        let mut f = bounce_frame(&device, vxe_render::Lighting::default().probe_freeze);
        let start = [30.0, 32.0, 26.0];
        f.frame.set_lights(&[list_light(start, true)]);
        f.run(&device, &queue, &gpu, 128);
        let end = [94.0, 32.0, 26.0];
        for i in 1..=16 {
            let x = start[0] + 4.0 * i as f32;
            f.frame.set_lights(&[list_light([x, 32.0, 26.0], true)]);
            f.run(&device, &queue, &gpu, 1);
        }
        let v = f.frame.probes();
        let probes = slot_probes(&f);
        let mean = |f: &FloorProbe, s: u32| {
            let p = f.frame.probes().read_probe(&device, &queue, s);
            p.interior_irradiance()
                .iter()
                .map(|t| t[0] + t[1] + t[2])
                .sum::<f32>()
                / 36.0
        };
        let states = v.read_states(&device, &queue);
        let slot = (0..probes.len())
            .filter(|&s| v.live(states[s], probes[s]))
            .min_by(|&a, &b| {
                let d = |s: usize| support::lights::dist(v.probe_position(probes[s]), end);
                d(a).total_cmp(&d(b))
            })
            .expect("a live probe") as u32;
        let mut series = Vec::new();
        for _ in 0..256 {
            series.push(mean(&f, slot));
            f.run(&device, &queue, &gpu, 1);
        }
        let frozen = mean(&f, slot);
        let lag = series
            .iter()
            .position(|m| (m / frozen - 1.0).abs() <= 0.1)
            .unwrap_or(series.len());
        println!(
            "bounce lag: probe {:?} at {:?} nearest the end; frozen mean {frozen:.4}; first frames {:?}; within 10 % after {lag} frames",
            probes[slot as usize],
            f.frame.probes().probe_position(probes[slot as usize]),
            &series[..12]
        );
    }
}
