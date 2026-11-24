//! Plan 0077 on the software adapter: two models of the entities library drawn from one
//! entity mirror. A closed rock box larger than Plan 0062's (the spider is seven metres
//! across), its table the showcase's plus the walker's rows, under a 192 x 108 frame with no
//! jitter, in the albedo view.

mod support;

use std::path::Path;

use flythrough::cave::ROCK;
use flythrough::entities::{self, library};
use flythrough::{showcase_materials, walker};
use support::bake::rig_on;
use svet_render::glam::Vec3;
use svet_render::{Camera, ViewMode};
use svet_world::{ChunkExtent, EditOp, FIXED_ONE, Fixed3, Mode, UniformSource, World};

const W: u32 = 192;
const H: u32 = 108;

/// The pixel `i`'s unit ray under `c` (the hit pass's `pixel_ray` with no jitter).
fn pixel_dir(c: &Camera, i: usize) -> Vec3 {
    let (f, r, u) = c.basis();
    let t = (c.vfov * 0.5).tan();
    let a = W as f32 / H as f32;
    let (px, py) = ((i % W as usize) as f32, (i / W as usize) as f32);
    let nx = (px + 0.5) / W as f32 * 2.0 - 1.0;
    let ny = (py + 0.5) / H as f32 * 2.0 - 1.0;
    (f + r * (nx * t * a) - u * (ny * t)).normalize()
}

/// The part instance an entity hit names at pixel `i` of the hit records, `None` for any
/// other hit.
fn instance_at(rec: &[[u16; 4]], i: usize) -> Option<u16> {
    let f = rec[i][3];
    (f & 1 != 0 && f & 32 != 0).then_some(f >> 6)
}

/// Plan 0077 Phase 2 done-when 5: the library's walker and spider stand side by side in a lit
/// rock box. At a pixel inside each one's silhouette (its four neighbours on the same entity
/// by the CPU query) the part instance the hit record names is the one `EntitySet::raycast`
/// names for the pixel's ray (the instance table holds the walker's parts and then the
/// spider's, in placement order, so the two pixels name instances of different entities), and the mirror's
/// pool holds the two models' distinct volumes' bytes exactly.
#[test]
fn two_models_draw_from_one_mirror() {
    let table = walker::with_walker(&showcase_materials());
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [3, 2, 1]),
        &UniformSource(ROCK),
        &table,
    );
    world
        .apply(&EditOp::box_(
            Fixed3([2, 2, 2].map(|v: i64| v * FIXED_ONE)),
            Fixed3([190, 126, 62].map(|v: i64| v * FIXED_ONE)),
            Mode::Carve,
        ))
        .expect("the box is resident");
    let Some(mut rig) = rig_on(world, [W, H], None) else {
        return;
    };
    rig.frame.set_lighting(support::lights::box_lighting());
    let mut look = rig.frame.look();
    look.view = ViewMode::Albedo;
    rig.frame.set_look(look);

    let lib = library::scan(&Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities"));
    let mut w = entities::Walkers::default().with_library(lib);
    assert_eq!(w.poll_viewer(0.0, &table), None, "the library builds");
    let index = |w: &entities::Walkers, name: &str| {
        w.library
            .entries
            .iter()
            .position(|e| e.name == name)
            .expect(name)
    };
    let (wi, si) = (index(&w, "walker"), index(&w, "spider"));
    let walker_id = w
        .place_entry(&table, Some(wi), [100.0, 40.0, 2.0], 0.0)
        .expect("walker");
    let spider_id = w
        .place_entry(&table, Some(si), [110.0, 88.0, 2.0], 0.0)
        .expect("spider");
    let camera = Camera {
        position: [40.0, 64.0, 14.0].into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 1.2,
    };
    let eye = camera.position.to_array();
    assert_eq!(
        entities::feed(&mut w, &rig.world, eye, &mut rig.frame, &rig.queue),
        None
    );
    rig.step(&camera);
    let rec = svet_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());

    let origin = Fixed3(eye.map(|v| (v * 256.0).round() as i64));
    let cpu: Vec<Option<(svet_entity::EntityId, u8)>> = (0..rec.len())
        .map(|i| {
            let dir = pixel_dir(&camera, i)
                .to_array()
                .map(|v| (v * 4096.0).round() as i64);
            w.set
                .raycast(origin, dir, 1 << 20)
                .map(|h| (h.entity, h.part))
        })
        .collect();
    let walker_parts = w.set.get(walker_id).expect("walker").0.parts().len() as u16;
    for id in [walker_id, spider_id] {
        // The first pixel whose four neighbours are on the same entity as it.
        let i = (W as usize + 1..rec.len() - W as usize - 1)
            .find(|&i| {
                let on = |j: usize| cpu[j].is_some_and(|(e, _)| e == id);
                on(i)
                    && [i - 1, i + 1, i - W as usize, i + W as usize]
                        .iter()
                        .all(|&j| on(j))
            })
            .unwrap_or_else(|| panic!("{id:?} covers no inner pixel"));
        let (_, part) = cpu[i].expect("on the entity");
        let offset = if id == walker_id { 0 } else { walker_parts };
        assert_eq!(
            instance_at(&rec, i),
            Some(offset + u16::from(part)),
            "{id:?} at pixel ({}, {})",
            i % W as usize,
            i / W as usize
        );
    }
    let bytes = |i: usize| {
        library::pool_bytes(
            &w.library.entries[i]
                .viewer()
                .and_then(|v| v.loaded.as_ref())
                .expect("built")
                .model,
        )
    };
    assert_eq!(
        rig.frame.entity_mirror().pool_used_bytes(),
        bytes(wi) + bytes(si)
    );
}

/// Two library walkers facing the camera from either side of its plane (`y` = 64), in the
/// rock box of [`two_models_draw_from_one_mirror`], and the albedo image and hit records of
/// one frame with `marks` set after the feed.
struct Marked {
    rgb: Vec<[u8; 3]>,
    rec: Vec<[u16; 4]>,
    /// Each walker's run of part slots.
    runs: [(u16, u16); 2],
}

/// The two walkers at `feet` on one rig: a frame drawn with no mark, then one with the first
/// walker marked by `mark`.
fn two_walkers(feet: [[f32; 3]; 2], mark: svet_render::Mark) -> Option<[Marked; 2]> {
    let table = walker::with_walker(&showcase_materials());
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [3, 2, 1]),
        &UniformSource(ROCK),
        &table,
    );
    world
        .apply(&EditOp::box_(
            Fixed3([2, 2, 2].map(|v: i64| v * FIXED_ONE)),
            Fixed3([190, 126, 62].map(|v: i64| v * FIXED_ONE)),
            Mode::Carve,
        ))
        .expect("the box is resident");
    let mut rig = rig_on(world, [W, H], None)?;
    rig.frame.set_lighting(support::lights::box_lighting());
    let mut look = rig.frame.look();
    look.view = ViewMode::Albedo;
    rig.frame.set_look(look);
    let lib = library::scan(&Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities"));
    let mut w = entities::Walkers::default().with_library(lib);
    assert_eq!(w.poll_viewer(0.0, &table), None, "the library builds");
    let wi = w
        .library
        .entries
        .iter()
        .position(|e| e.name == "walker")
        .expect("walker");
    let camera = Camera {
        position: [40.0, 64.0, 14.0].into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 1.2,
    };
    let eye = camera.position.to_array();
    let ids = feet.map(|f| {
        w.place_entry(&table, Some(wi), f, walker::facing(f, eye))
            .expect("walker")
    });
    assert_eq!(
        entities::feed(&mut w, &rig.world, eye, &mut rig.frame, &rig.queue),
        None
    );
    let mut frame = |marks: &[(svet_entity::EntityId, svet_render::Mark)]| {
        rig.frame.entity_mirror_mut().set_marks(&rig.queue, marks);
        rig.step(&camera);
        let rec =
            svet_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
        let rgb = rig
            .display()
            .chunks(4)
            .map(|p| [p[0], p[1], p[2]])
            .collect();
        let m = rig.frame.entity_mirror().instances();
        let runs = ids.map(|id| m.run_of(id).expect("drawn"));
        Marked { rgb, rec, runs }
    };
    let plain = frame(&[]);
    let marked = frame(&[(ids[0], mark)]);
    Some([plain, marked])
}

/// The part slot an entity hit names at pixel `i`, `None` for any other hit.
fn slot_at(rec: &[[u16; 4]], i: usize) -> Option<u16> {
    instance_at(rec, i).map(|v| v & 511)
}

/// Pixel `i`'s four neighbours inside the image.
fn neighbours(i: usize) -> impl Iterator<Item = usize> {
    let (x, y, w, h) = (i % W as usize, i / W as usize, W as usize, H as usize);
    [
        (x > 0).then(|| i - 1),
        (x + 1 < w).then(|| i + 1),
        (y > 0).then(|| i - w),
        (y + 1 < h).then(|| i + w),
    ]
    .into_iter()
    .flatten()
}

/// `a` within `tol` 255ths of `b` in every channel.
fn near(a: [u8; 3], b: [f32; 3], tol: f32) -> bool {
    (0..3).all(|k| (f32::from(a[k]) - b[k] * 255.0).abs() <= tol)
}

/// The mark colour as the marks buffer quantizes it, in 0..1.
fn blue() -> [f32; 3] {
    svet_render::MARK_COLOURS[0].map(|v| (v * 255.0).round() / 255.0)
}

fn mixed(c: [u8; 3], to: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|k| f32::from(c[k]) / 255.0 * (1.0 - t) + to[k] * t)
}

const BLUE_MARK: svet_render::Mark = svet_render::Mark {
    colour: svet_render::MARK_COLOURS[0],
    outline: true,
    tint: 0.35,
};

/// Plan 0077 Phase 6 done-when 1 (ADR-0074): two walkers side by side, the left one marked
/// blue with the outline and a tint of 0.35, in the albedo view. Its centre pixel is the
/// unmarked colour mixed 0.35 toward the blue, within 2/255, as the right walker's mirrored
/// centre pixel mixed the same; every left-walker pixel with a neighbour off it (the world or
/// the other walker) is the blue within 2/255 and no such right-walker pixel is; a left-walker
/// pixel on a boundary between two of its parts, its four neighbours all its own, is the
/// tinted colour, not the blue; with no mark the two walkers read within 1/255 at mirrored
/// pixels.
#[test]
fn a_marked_instance_has_its_outline_and_tint() {
    let feet = [[80.0, 56.0, 2.0], [80.0, 72.0, 2.0]];
    let Some([plain, marked]) = two_walkers(feet, BLUE_MARK) else {
        return;
    };
    assert_eq!(plain.rec, marked.rec, "a mark changes no hit");
    let mirror = |i: usize| i - i % W as usize + (W as usize - 1 - i % W as usize);
    let owner = |rec: &[[u16; 4]], runs: [(u16, u16); 2], i: usize| {
        let s = slot_at(rec, i)?;
        runs.iter().position(|&(f, n)| (f..f + n).contains(&s))
    };
    let on = |e: usize, i: usize| owner(&plain.rec, plain.runs, i) == Some(e);
    let px: Vec<usize> = (0..plain.rec.len()).collect();
    for e in 0..2 {
        assert!(px.iter().any(|&i| on(e, i)), "walker {e} is drawn");
    }
    // Unmarked, the two walkers are mirror images.
    let mut checked = 0;
    for &i in px.iter().filter(|&&i| on(0, i)) {
        let j = mirror(i);
        if !on(1, j) {
            continue;
        }
        let (a, b) = (plain.rgb[i], plain.rgb[j]);
        assert!(
            (0..3).all(|k| a[k].abs_diff(b[k]) <= 1),
            "unmarked pixel {i} {a:?}, mirrored {j} {b:?}"
        );
        checked += 1;
    }
    println!("{checked} mirrored pixels compared unmarked");
    let inner = |e: usize, i: usize| on(e, i) && neighbours(i).all(|j| on(e, j));
    // The centre pixel of each: its inner pixel nearest the mean of its pixels.
    let centre = |e: usize| {
        let all: Vec<usize> = px.iter().copied().filter(|&i| on(e, i)).collect();
        let xy = |i: usize| [(i % W as usize) as f32, (i / W as usize) as f32];
        let n = all.len() as f32;
        let mean = all.iter().fold([0.0; 2], |a, &i| {
            let p = xy(i);
            [a[0] + p[0] / n, a[1] + p[1] / n]
        });
        let d = |i: usize| {
            let p = xy(i);
            (p[0] - mean[0]).powi(2) + (p[1] - mean[1]).powi(2)
        };
        all.iter()
            .copied()
            .filter(|&i| inner(e, i))
            .min_by(|&a, &b| d(a).total_cmp(&d(b)))
            .expect("an inner pixel")
    };
    let (cl, cr) = (centre(0), centre(1));
    let want = mixed(plain.rgb[cl], blue(), 0.35);
    assert!(
        near(marked.rgb[cl], want, 2.0),
        "left centre {:?}, want {want:?}",
        marked.rgb[cl]
    );
    let m = mirror(cl);
    assert!(on(1, m), "the left centre's mirror is on the right walker");
    assert!(
        near(marked.rgb[cl], mixed(plain.rgb[m], blue(), 0.35), 2.0),
        "left centre {:?} against the right walker's mirrored {:?} mixed",
        marked.rgb[cl],
        plain.rgb[m]
    );
    assert_eq!(
        marked.rgb[cr], plain.rgb[cr],
        "the right centre is unmarked"
    );
    // The outline: every edge pixel of the left walker blue, none of the right's.
    let (mut edges, mut right_edges) = (0, 0);
    for &i in &px {
        let e = match owner(&plain.rec, plain.runs, i) {
            Some(e) => e,
            None => continue,
        };
        if !neighbours(i).any(|j| !on(e, j)) {
            continue;
        }
        if e == 0 {
            assert!(
                near(marked.rgb[i], blue(), 2.0),
                "edge pixel {i} {:?}",
                marked.rgb[i]
            );
            edges += 1;
        } else {
            assert!(!near(marked.rgb[i], blue(), 2.0), "right edge pixel {i}");
            assert_eq!(marked.rgb[i], plain.rgb[i], "right edge pixel {i}");
            right_edges += 1;
        }
    }
    assert!(edges > 0 && right_edges > 0);
    // A part boundary inside the left walker: no line, the tint.
    let boundary = px
        .iter()
        .copied()
        .find(|&i| {
            inner(0, i) && neighbours(i).any(|j| slot_at(&plain.rec, j) != slot_at(&plain.rec, i))
        })
        .expect("the walker's parts meet inside its silhouette");
    let want = mixed(plain.rgb[boundary], blue(), 0.35);
    assert!(
        near(marked.rgb[boundary], want, 2.0),
        "part boundary {boundary}: {:?}, want {want:?}",
        marked.rgb[boundary]
    );
    println!(
        "centres {cl} {cr}, {edges} left and {right_edges} right edge pixels, part boundary at {boundary}"
    );
}

/// Plan 0077 Phase 6 done-when 2: the two walkers overlapping on screen (the right one a
/// little behind and toward the left), the first marked: every pixel of either walker with a
/// neighbour on the other is the mark colour (the edge is a change of entity, not of depth).
#[test]
fn marks_hold_two_touching_instances_apart() {
    let feet = [[80.0, 61.0, 2.0], [84.0, 65.0, 2.0]];
    let Some([plain, marked]) = two_walkers(feet, BLUE_MARK) else {
        return;
    };
    let owner = |i: usize| {
        let s = slot_at(&plain.rec, i)?;
        plain
            .runs
            .iter()
            .position(|&(f, n)| (f..f + n).contains(&s))
    };
    let mut shared = 0;
    for i in 0..plain.rec.len() {
        let Some(e) = owner(i) else { continue };
        if neighbours(i).any(|j| owner(j) == Some(1 - e)) {
            assert!(
                near(marked.rgb[i], blue(), 2.0),
                "shared edge pixel {i} of walker {e}: {:?}",
                marked.rgb[i]
            );
            shared += 1;
        }
    }
    println!("{shared} shared edge pixels");
    assert!(shared > 0, "the walkers touch on screen");
}

/// Readings, run by hand (`--run-ignored only -E 'test(=reading::<name>)'`), on the adapter
/// `SVET_TEST_ADAPTER` names (unset: lavapipe). They print and assert nothing.
mod reading {
    use super::*;
    use svet_render::{
        FRAME_PASSES, Fsr2Settings, GpuWorld, MirrorConfig, PassTimer, UpscalerKind, Window,
    };

    /// Plan 0077 Phase 6's reading (ADR-0074): the composite pass's GPU time at 1920 x 1080
    /// output under FSR 2 at 1.5 (1280 x 720 internal), the shaded view, four library walkers
    /// in the rock box, with no mark, one marked and four marked; the median and the p90 of
    /// 64 frames after 16 warm-up frames.
    #[test]
    #[ignore = "reading"]
    fn the_composite_cost_with_marks() {
        let table = walker::with_walker(&showcase_materials());
        let world = World::generate(
            ChunkExtent::new([0, 0, 0], [3, 2, 1]),
            &UniformSource(ROCK),
            &table,
        );
        world
            .apply(&EditOp::box_(
                Fixed3([2, 2, 2].map(|v: i64| v * FIXED_ONE)),
                Fixed3([190, 126, 62].map(|v: i64| v * FIXED_ONE)),
                Mode::Carve,
            ))
            .expect("the box is resident");
        let Some((device, queue, adapter)) = support::reading_device(64 << 20) else {
            return;
        };
        let cfg = MirrorConfig {
            budget_bytes: 64 << 20,
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        };
        let mut gpu = GpuWorld::with_config(&device, cfg);
        for _ in 0..2 {
            support::SyncNow::sync_now(&mut gpu, &world, &device, &queue).expect("fits");
        }
        let mut frame = svet_render::Frame::with_sizes(&device, [1280, 720], [1920, 1080]);
        frame.set_upscaler_kind(&device, UpscalerKind::Fsr2(Fsr2Settings::default()));
        frame.set_lighting(support::lights::box_lighting());
        frame.set_dt(1.0 / 60.0);
        let Some(timer) = PassTimer::with_passes(&device, &queue, FRAME_PASSES.len() as u32) else {
            println!("{adapter}: no timestamp queries");
            return;
        };
        let slot = FRAME_PASSES
            .iter()
            .position(|&n| n == "composite")
            .expect("composite");
        let lib = library::scan(&Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities"));
        let mut w = entities::Walkers::default().with_library(lib);
        assert_eq!(w.poll_viewer(0.0, &table), None, "the library builds");
        let wi = w
            .library
            .entries
            .iter()
            .position(|e| e.name == "walker")
            .expect("walker");
        let camera = Camera {
            position: [40.0, 64.0, 14.0].into(),
            yaw: 0.0,
            pitch: 0.0,
            vfov: 1.2,
        };
        let eye = camera.position.to_array();
        let ids: Vec<_> = [52.0, 60.0, 68.0, 76.0]
            .iter()
            .map(|&y| {
                let f = [80.0, y, 2.0];
                w.place_entry(&table, Some(wi), f, walker::facing(f, eye))
                    .expect("walker")
            })
            .collect();
        for n in [0usize, 1, 4] {
            let marks: Vec<_> = ids[..n].iter().map(|&id| (id, BLUE_MARK)).collect();
            let mut ms = Vec::new();
            for k in 0..80 {
                entities::feed(&mut w, &world, eye, &mut frame, &queue);
                frame.entity_mirror_mut().set_marks(&queue, &marks);
                frame.advance();
                let params = frame.params(&gpu, &camera, &svet_render::LightParams::default());
                let mut enc = device.create_command_encoder(&Default::default());
                frame.encode(&device, &queue, &mut enc, &gpu, &params, Some(&timer));
                timer.resolve(&mut enc);
                queue.submit([enc.finish()]);
                let t = timer.read_all_ms(&device)[slot];
                if k >= 16 {
                    ms.push(t);
                }
            }
            ms.sort_by(f64::total_cmp);
            println!(
                "{adapter}: {n} marked of 4 walkers, composite median {:.4} ms, p90 {:.4} ms (64 frames, 1280 x 720 internal, FSR 2 to 1920 x 1080)",
                ms[32], ms[57]
            );
        }
    }
}
