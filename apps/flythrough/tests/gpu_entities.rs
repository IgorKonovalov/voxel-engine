//! Plan 0062's entities on the software adapter (ADR-0050): voxel-model parts drawn in the
//! hit pass after the world march, and the caps of the entity mirror. A closed rock box, its
//! table the cave's plus the walker's rows and a red row, under a 192 x 108 frame with no
//! jitter. `VXE_BLESS=1` rewrites the goldens here.

mod support;

use std::sync::Arc;

use flythrough::cave::ROCK;
use flythrough::cave_materials;
use flythrough::walker;
use support::bake::{Rig, rig_on};
use vxe_entity::{EntityId, EntitySet, Model, Part, Pose, Quat};
use vxe_render::glam::Vec3;
use vxe_render::{Camera, EntityMirror, EntityUploadError, ModelHandle, ViewMode};
use vxe_world::{
    ChunkExtent, EditOp, FIXED_ONE, Fixed3, Material, MaterialId, MaterialTable, Mode, Prefab,
    UniformSource, World,
};

const W: u32 = 192;
const H: u32 = 108;

/// The part's red: albedo (0.8, 0.2, 0.2).
const RED: [f32; 3] = [0.8, 0.2, 0.2];

/// The cave's rows, the walker's two and a red row.
fn table() -> (MaterialTable, MaterialId) {
    let mut t = walker::with_walker(&cave_materials());
    let red = t.push(Material {
        albedo: RED,
        ..Material::named("red")
    });
    (t, red)
}

/// A closed rock box over chunks `[0, 2) x [0, 1) x [0, 1)`, hollow over `[2, 126) x [2, 62) x
/// [2, 50)`, of `table`'s rows.
fn entity_box(table: &MaterialTable) -> World {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 1, 1]),
        &UniformSource(ROCK),
        table,
    );
    world
        .apply(&EditOp::box_(
            Fixed3([2, 2, 2].map(|v: i64| v * FIXED_ONE)),
            Fixed3([126, 62, 50].map(|v: i64| v * FIXED_ONE)),
            Mode::Carve,
        ))
        .expect("the box is resident");
    world
}

/// The box under a 192 x 108 frame (no jitter, full lighting grid, lamp off) and the red row.
fn rig() -> Option<(Rig, MaterialId)> {
    let (t, red) = table();
    let mut rig = rig_on(entity_box(&t), [W, H], None)?;
    rig.frame.set_lighting(support::lights::box_lighting());
    Some((rig, red))
}

fn albedo_view(rig: &mut Rig) {
    let mut look = rig.frame.look();
    look.view = ViewMode::Albedo;
    rig.frame.set_look(look);
}

/// A one-part model: a solid prefab of `size` and `m`, its pivot at its centre, at `detail`.
fn block(size: [u32; 3], m: MaterialId, detail: u8) -> Arc<Model> {
    let n = (size[0] * size[1] * size[2]) as usize;
    let part = Part {
        volume: Arc::new(Prefab::new(size, vec![m; n], None)),
        parent: None,
        pivot: size.map(|s| (s * 128) as i32),
        detail,
    };
    Arc::new(Model::new(vec![part]).expect("one part"))
}

/// `model` posed with its root (a one-part model's pivot) at `at` voxels, turned `q`.
fn posed(model: &Model, at: [f32; 3], q: Quat) -> Pose {
    let mut pose = Pose::rest(model);
    pose.root = Fixed3(at.map(|v| (v * 256.0).round() as i64));
    pose.root_q = q;
    pose
}

/// Uploads each model to the rig's frame and draws `entities` (a model and its pose each):
/// returns the set and the frame's list.
fn draw(
    rig: &mut Rig,
    entities: &[(Arc<Model>, Pose)],
) -> (EntitySet, Vec<(EntityId, ModelHandle)>) {
    let mut set = EntitySet::new();
    let mut list = Vec::new();
    for (model, pose) in entities {
        let h = rig
            .frame
            .entity_mirror_mut()
            .upload_model(&rig.queue, model, rig.world.materials())
            .expect("the pool has room");
        let id = set
            .insert(model.clone(), pose.clone())
            .expect("the pose fits");
        list.push((id, h));
    }
    rig.frame.set_entities(&rig.queue, &set, &list);
    (set, list)
}

/// The pixel the point `p` projects to under `c` on the 192 x 108 target.
fn pixel_of(c: &Camera, p: [f32; 3]) -> usize {
    let (f, r, u) = c.basis();
    let d = Vec3::from(p) - c.position;
    let z = d.dot(f);
    let t = (c.vfov * 0.5).tan();
    let a = W as f32 / H as f32;
    let x = d.dot(r) / z / (t * a);
    let y = d.dot(u) / z / t;
    let px = ((x + 1.0) * 0.5 * W as f32) as usize;
    let py = ((1.0 - y) * 0.5 * H as f32) as usize;
    py * W as usize + px
}

/// The unit ray of pixel `i` under `c` (the hit pass's `pixel_ray` with no jitter).
fn pixel_dir(c: &Camera, i: usize) -> Vec3 {
    let (f, r, u) = c.basis();
    let t = (c.vfov * 0.5).tan();
    let a = W as f32 / H as f32;
    let (px, py) = ((i % W as usize) as f32, (i / W as usize) as f32);
    let nx = (px + 0.5) / W as f32 * 2.0 - 1.0;
    let ny = (py + 0.5) / H as f32 * 2.0 - 1.0;
    (f + r * (nx * t * a) - u * (ny * t)).normalize()
}

/// The target's rgb at pixel `i` as unorm bytes.
fn rgb(img: &[u8], i: usize) -> [u8; 3] {
    [img[4 * i], img[4 * i + 1], img[4 * i + 2]]
}

fn unorm(c: [f32; 3]) -> [u8; 3] {
    c.map(|v| (v * 255.0).round() as u8)
}

/// Asserts `got` within one unorm step of `want` per channel.
fn near(name: &str, got: [u8; 3], want: [u8; 3]) {
    let ok = (0..3).all(|k| got[k].abs_diff(want[k]) <= 1);
    assert!(ok, "{name}: {got:?} against {want:?}");
}

/// The instance an entity hit names at pixel `i` of the hit records, `None` for any other hit.
fn instance_at(rec: &[[u16; 4]], i: usize) -> Option<u16> {
    let f = rec[i][3];
    (f & 1 != 0 && f & 32 != 0).then_some(f >> 6)
}

/// Looking down the box's length toward +x from x 86, level, at height 26.
fn level_camera(x: f32, z: f32) -> Camera {
    Camera {
        position: [x, 32.0, z].into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 1.2,
    }
}

/// Phase 2 done-when 1: in the albedo view an 8-voxel red cube 20 voxels in front of the
/// camera (its near face at x 106) and the far wall 40 voxels away (x 126): the centre pixel
/// reads the part's albedo and its hit distance 20 within 0.01 voxel; the pixel whose ray
/// passes 12 voxels to the side at the part's depth reads the wall's.
#[test]
fn an_entity_hides_the_wall_behind_it() {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    albedo_view(&mut rig);
    let camera = level_camera(86.0, 26.0);
    draw(
        &mut rig,
        &[(
            block([8; 3], red, 1),
            posed(&block([8; 3], red, 1), [110.0, 32.0, 26.0], Quat::IDENTITY),
        )],
    );
    rig.step(&camera);
    let img = rig.display();
    let pos = vxe_render::debug::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions());
    let centre = pixel_of(&camera, [106.0, 32.0, 26.0]);
    near("centre", rgb(&img, centre), unorm(RED));
    let t = pos[centre][3];
    println!("centre: t {t}, hit {:?}", &pos[centre][..3]);
    assert!((t - 20.0).abs() <= 0.01, "the centre's hit distance {t}");
    let side = pixel_of(&camera, [106.0, 20.0, 26.0]);
    let rock = cave_materials().get(ROCK).expect("rock").albedo;
    near("12 aside", rgb(&img, side), unorm(rock));
    assert!(
        (pos[side][0] - 126.0).abs() < 1e-3,
        "the side pixel's hit {:?}",
        pos[side]
    );
}

/// Phase 2 done-when 2: a world slab between the camera and the part (x 96..98) gives the
/// slab's albedo at the centre pixel.
#[test]
fn a_world_voxel_in_front_hides_the_entity() {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    support::bake::fill(&rig.world, [96, 28, 22], [98, 36, 30], ROCK);
    for _ in 0..2 {
        support::SyncNow::sync_now(&mut rig.gpu, &rig.world, &rig.device, &rig.queue)
            .expect("fits");
    }
    albedo_view(&mut rig);
    let camera = level_camera(86.0, 26.0);
    let cube = block([8; 3], red, 1);
    draw(
        &mut rig,
        &[(
            cube.clone(),
            posed(&cube, [110.0, 32.0, 26.0], Quat::IDENTITY),
        )],
    );
    rig.step(&camera);
    let img = rig.display();
    let rec = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let centre = pixel_of(&camera, [106.0, 32.0, 26.0]);
    let rock = cave_materials().get(ROCK).expect("rock").albedo;
    near("centre, behind the slab", rgb(&img, centre), unorm(rock));
    assert_eq!(
        instance_at(&rec, centre),
        None,
        "the slab's hit is the world's"
    );
}

/// Phase 2 done-when 3: a 16 x 4 x 4 bar across the view (its long axis along y) reads the bar
/// 6 voxels right of centre and the wall 6 voxels above; turned 90 degrees about the view axis
/// (x), the two swap.
#[test]
fn a_turned_bar_shows_its_turned_silhouette() {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    albedo_view(&mut rig);
    let camera = level_camera(86.0, 26.0);
    let bar = block([4, 16, 4], red, 1);
    let rock = unorm(cave_materials().get(ROCK).expect("rock").albedo);
    // Camera right is -y; the bar's near face is at x 106.
    let right = pixel_of(&camera, [106.0, 26.0, 26.0]);
    let above = pixel_of(&camera, [106.0, 32.0, 32.0]);
    let quarter = walker::turn(0, std::f32::consts::FRAC_PI_2);
    for (name, q, at_right, at_above) in [
        ("level", Quat::IDENTITY, unorm(RED), rock),
        ("turned", quarter, rock, unorm(RED)),
    ] {
        draw(
            &mut rig,
            &[(bar.clone(), posed(&bar, [108.0, 32.0, 26.0], q))],
        );
        rig.frame.reset_history();
        rig.step(&camera);
        let img = rig.display();
        near(&format!("{name}: 6 right"), rgb(&img, right), at_right);
        near(&format!("{name}: 6 above"), rgb(&img, above), at_above);
    }
}

/// Phase 2 done-when 4: the walker in a tilted pose; over an 8 x 8 grid of pixels spanning the
/// pixels it covers, the part the hit record names equals `EntitySet::raycast`'s part for the
/// pixel's ray (no entity on both counts as equal), on every pixel whose four neighbours name
/// the same part; the others are counted as silhouette pixels and skipped.
#[test]
fn the_picture_and_the_query_name_the_same_part() {
    let Some((mut rig, _)) = rig() else {
        return;
    };
    albedo_view(&mut rig);
    let camera = level_camera(84.0, 9.0);
    let (body, eye) = walker::rows(rig.world.materials()).expect("the walker's rows");
    let model = Arc::new(walker::model(body, eye));
    let mut pose = walker::pose([104.0, 32.0, 2.0], 0.5, 0.4);
    pose.root_q = walker::turn(0, 0.35) * pose.root_q;
    let (set, _) = draw(&mut rig, &[(model, pose)]);
    rig.step(&camera);
    let rec = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let gpu: Vec<Option<u16>> = (0..rec.len()).map(|i| instance_at(&rec, i)).collect();
    let covered: Vec<usize> = (0..gpu.len()).filter(|&i| gpu[i].is_some()).collect();
    assert!(
        covered.len() > 200,
        "the walker covers {} pixels",
        covered.len()
    );
    let (xs, ys): (Vec<usize>, Vec<usize>) = covered
        .iter()
        .map(|&i| (i % W as usize, i / W as usize))
        .unzip();
    let (x0, x1) = (*xs.iter().min().unwrap(), *xs.iter().max().unwrap());
    let (y0, y1) = (*ys.iter().min().unwrap(), *ys.iter().max().unwrap());
    let origin = Fixed3(
        camera
            .position
            .to_array()
            .map(|v| (v * 256.0).round() as i64),
    );
    let (mut checked, mut skipped, mut on_entity) = (0, 0, 0);
    for gy in 0..8 {
        for gx in 0..8 {
            let x = x0 + (x1 - x0) * (2 * gx + 1) / 16;
            let y = y0 + (y1 - y0) * (2 * gy + 1) / 16;
            let i = y * W as usize + x;
            let n = [i - 1, i + 1, i - W as usize, i + W as usize];
            if n.iter().any(|&j| gpu[j] != gpu[i]) {
                skipped += 1;
                continue;
            }
            let d = pixel_dir(&camera, i);
            let dir = d.to_array().map(|v| (v * 4096.0).round() as i64);
            let cpu = set.raycast(origin, dir, 1 << 20).map(|h| u16::from(h.part));
            assert_eq!(gpu[i], cpu, "pixel ({x}, {y})");
            checked += 1;
            on_entity += usize::from(cpu.is_some());
        }
    }
    println!(
        "{checked} pixels agree ({on_entity} on the walker), {skipped} silhouette pixels skipped"
    );
    assert!(on_entity >= 16, "{on_entity} grid pixels on the walker");
}

/// Phase 2 done-when 6: the caps count what they drop. 65 one-part entities draw 64 and count 1
/// over; 40 walkers (160 parts) all draw; 129 four-part entities draw 64 (256 parts) and count
/// 65 over; 40 sixteen-part entities draw 32 (512 parts) and count 8 over. A pool of 4096 bytes
/// refuses the 16-voxel cube part (5672 bytes) with `PoolFull`.
#[test]
fn the_caps_count_what_they_drop() {
    let Some((device, queue)) = support::software_device(64 << 20) else {
        return;
    };
    let (t, red) = table();
    let (body, eye) = walker::rows(&t).expect("the walker's rows");
    let one = Arc::new(Prefab::new([1; 3], vec![red], None));
    let parts = |n: usize| {
        let p = |_| Part {
            volume: one.clone(),
            parent: None,
            pivot: [128; 3],
            detail: 1,
        };
        Arc::new(Model::new((0..n).map(p).collect()).expect("parts"))
    };
    let walker = Arc::new(walker::model(body, eye));
    for (name, model, count, drawn, drawn_parts, over) in [
        ("one-part", parts(1), 65, 64, 64, 1),
        ("walkers", walker.clone(), 40, 40, 160, 0),
        ("four-part", parts(4), 129, 64, 256, 65),
        ("sixteen-part", parts(16), 40, 32, 512, 8),
    ] {
        let mut mirror = EntityMirror::new(&device, 1 << 20);
        let h = mirror.upload_model(&queue, &model, &t).expect("room");
        let mut set = EntitySet::new();
        let list: Vec<_> = (0..count)
            .map(|i| {
                let pose = Pose {
                    root: Fixed3([i64::from(i) * 4096, 0, 0]),
                    ..Pose::rest(&model)
                };
                (set.insert(model.clone(), pose).expect("fits"), h)
            })
            .collect();
        mirror.set_entities(&queue, &set, &list);
        let got = (
            mirror.entities_drawn(),
            mirror.parts_drawn(),
            mirror.entities_over_cap(),
        );
        println!("{name}: {count} given, drawn, parts, over {got:?}");
        assert_eq!(got, (drawn, drawn_parts, over), "{name}");
    }
    let mut small = EntityMirror::new(&device, 4096);
    let cube = block([16; 3], red, 1);
    let err = small.upload_model(&queue, &cube, &t).expect_err("too big");
    assert_eq!(
        err,
        EntityUploadError::PoolFull {
            needed: 5672,
            free: 4096
        }
    );
    assert_eq!(small.pool_used_bytes(), 0, "nothing written");
}

/// Phase 2 done-when 7: the walker at rest on the floor, facing the camera, in the albedo
/// view at 192 x 108, against the golden blessed on lavapipe: at most 0.1 % of pixels differ
/// by more than 2/255 in a channel (the harness's floor).
#[test]
fn walker_albedo_golden() {
    let Some((mut rig, _)) = rig() else {
        return;
    };
    albedo_view(&mut rig);
    let camera = level_camera(84.0, 9.0);
    let (body, eye) = walker::rows(rig.world.materials()).expect("the walker's rows");
    let model = Arc::new(walker::model(body, eye));
    let feet = [104.0, 32.0, 2.0];
    let pose = walker::pose(feet, walker::facing(feet, camera.position.to_array()), 0.0);
    draw(&mut rig, &[(model, pose)]);
    rig.step(&camera);
    assert_golden_sized("walker_albedo.png", &rig.display());
}

/// [`support::assert_golden`]'s rule at 192 x 108.
fn assert_golden_sized(name: &str, rgba: &[u8]) {
    let path = support::golden_path(name);
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        support::write_png(&path, rgba, W, H);
        println!("blessed {path:?}");
        return;
    }
    let (golden, w, h) = support::read_png(&path);
    assert_eq!((w, h), (W, H));
    let f = support::fraction_differing(rgba, &golden, 2);
    println!(
        "{name}: {:.4} % of pixels differ by more than 2/255",
        f * 100.0
    );
    assert!(f <= 0.001, "{name}: {:.4} % of pixels differ", f * 100.0);
}

// ---- Phase 3: light, shadow and emission ----

use vxe_render::{Light, LightingResolution};

/// ADR-0044's term apart from the renderer: `PI * intensity * max(n . l, 0) / d^2` times the
/// window `(1 - (d / r)^4)^2`, `r = sqrt(intensity / floor)`, at `p` with normal `n`.
fn analytic(light: &Light, floor: f32, p: [f32; 3], n: [f32; 3]) -> f32 {
    let l: [f32; 3] = std::array::from_fn(|k| light.pos[k] - p[k]);
    let d2: f32 = l.iter().map(|v| v * v).sum();
    let d = d2.sqrt();
    let cos = ((0..3).map(|k| l[k] * n[k]).sum::<f32>() / d).max(0.0);
    let r = (light.intensity / floor).sqrt();
    let window = (1.0 - (d / r).powi(4)).max(0.0).powi(2);
    std::f32::consts::PI * light.intensity * cos / d2 * window
}

/// Asserts `got` within `tol` (relative) of `want`.
fn within(name: &str, got: f32, want: f32, tol: f32) {
    println!(
        "{name}: {got:.4} against {want:.4} ({:+.3} %)",
        (got / want - 1.0) * 100.0
    );
    assert!(
        (got / want - 1.0).abs() <= tol,
        "{name}: {got} against {want}"
    );
}

/// The last frame's hits: (pixel, point, record) for every pixel that hit.
fn hits(rig: &Rig) -> Vec<(usize, [f32; 3], [u16; 4])> {
    let pos = vxe_render::debug::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions());
    let rec = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    (0..pos.len())
        .filter(|&i| rec[i][3] & 1 != 0)
        .map(|i| (i, [pos[i][0], pos[i][1], pos[i][2]], rec[i]))
        .collect()
}

/// The hit nearest `p` among those `keep` takes: its pixel and point.
fn nearest(
    hits: &[(usize, [f32; 3], [u16; 4])],
    p: [f32; 3],
    keep: impl Fn(&[u16; 4]) -> bool,
) -> (usize, [f32; 3]) {
    let d = |q: [f32; 3]| (0..3).map(|k| (q[k] - p[k]).powi(2)).sum::<f32>();
    hits.iter()
        .filter(|h| keep(&h.2))
        .min_by(|a, b| d(a.1).total_cmp(&d(b.1)))
        .map(|h| (h.0, h.1))
        .expect("a hit near the point")
}

fn world_hit(r: &[u16; 4]) -> bool {
    r[3] & 32 == 0
}

fn entity_hit(k: u16) -> impl Fn(&[u16; 4]) -> bool {
    move |r| r[3] & 32 != 0 && r[3] >> 6 == k
}

/// The direct target's luminance (`a`) per pixel.
fn direct(rig: &Rig) -> Vec<f32> {
    vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.direct())
        .iter()
        .map(|p| p[3])
        .collect()
}

/// A 16 x 16 x 2 slab of `m` (x, y wide, z thick), its pivot at `pivot` (model voxels), detail 1.
fn slab(m: MaterialId, pivot: [i32; 3]) -> Arc<Model> {
    let part = Part {
        volume: Arc::new(Prefab::new([16, 16, 2], vec![m; 512], None)),
        parent: None,
        pivot: pivot.map(|v| v * 256),
        detail: 1,
    };
    Arc::new(Model::new(vec![part]).expect("one part"))
}

/// From y 6, 8 voxels over the floor, looking along +y and a little down: under a slab at
/// heights 9 to 11 the floor below it shows.
fn under_camera() -> Camera {
    Camera {
        position: [64.0, 6.0, 8.0].into(),
        yaw: std::f32::consts::FRAC_PI_2,
        pitch: -0.25,
        vfov: 1.2,
    }
}

/// Phase 3 done-when 1: the lit box's light of 1200 20 voxels above the floor at (64, 32), and a
/// 16 x 16 x 2 slab part level between heights 9 and 11 centred under it: the floor under the
/// light and 10 voxels aside read 0 direct (the shadow's edge is at 17.8), 20 voxels aside its
/// term within 2 %; with `SHADOWS` clear all three read their term within 2 %.
#[test]
fn an_entity_casts_a_hard_shadow_on_the_floor() {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    let floor = rig.frame.lighting().light_floor;
    let light = Light::point([64.0, 32.0, 22.0], 1200.0);
    let s = slab(red, [8, 8, 1]);
    draw(
        &mut rig,
        &[(s.clone(), posed(&s, [64.0, 32.0, 10.0], Quat::IDENTITY))],
    );
    let camera = under_camera();
    for shadows in [true, false] {
        let flags = if shadows {
            Light::ON | Light::SHADOWS
        } else {
            Light::ON
        };
        rig.frame.set_lights(&[Light { flags, ..light }]);
        rig.frame.reset_history();
        rig.step(&camera);
        let hits = hits(&rig);
        let e = direct(&rig);
        for (name, x, dark) in [
            ("under", 64.0, true),
            ("10 aside", 74.0, true),
            ("20 aside", 84.0, false),
        ] {
            let (i, p) = nearest(&hits, [x, 32.0, 2.0], world_hit);
            assert!(
                (p[0] - x).abs() < 1.0 && (p[2] - 2.0).abs() < 1e-3,
                "{name}: {p:?}"
            );
            let want = analytic(&light, floor, p, [0.0, 0.0, 1.0]);
            if shadows && dark {
                println!("{name}: {} in the shadow at {p:?}", e[i]);
                assert_eq!(e[i], 0.0, "{name} at {p:?}");
            } else {
                within(&format!("{name}, shadows {shadows}"), e[i], want, 0.02);
            }
        }
    }
}

/// Phase 3 done-when 2: the light 22 voxels above the floor, a 16 x 16 x 2 slab resting on it
/// (top face 20 below the light): the top face's centre reads 9.40 within 2 %; turned 45
/// degrees about a level axis (x) through that centre, the same point reads 6.65 within 2 %.
#[test]
fn an_entity_face_is_lit_by_its_own_normal() {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    let floor = rig.frame.lighting().light_floor;
    let light = Light::point([64.0, 32.0, 24.0], 1200.0);
    rig.frame.set_lights(&[light]);
    let s = slab(red, [8, 8, 2]);
    // The camera placed so that pixel (96, 54)'s ray passes through the top face's centre.
    let centre = [64.0, 32.0, 4.0];
    let mut camera = Camera {
        position: [0.0; 3].into(),
        yaw: 0.0,
        pitch: -0.7,
        vfov: 1.2,
    };
    let px = 54 * W as usize + 96;
    camera.position = Vec3::from(centre) - pixel_dir(&camera, px) * 28.0;
    let (sn, cn) = std::f32::consts::FRAC_PI_4.sin_cos();
    for (name, q, n, want) in [
        ("level", Quat::IDENTITY, [0.0, 0.0, 1.0], 9.40),
        (
            "turned 45",
            walker::turn(0, std::f32::consts::FRAC_PI_4),
            [0.0, -sn, cn],
            6.65,
        ),
    ] {
        draw(&mut rig, &[(s.clone(), posed(&s, [64.0, 32.0, 4.0], q))]);
        rig.frame.reset_history();
        rig.step(&camera);
        let (i, p) = nearest(&hits(&rig), [64.0, 32.0, 4.0], entity_hit(0));
        let d = (0..3)
            .map(|k| (p[k] - [64.0, 32.0, 4.0][k]).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!(d < 0.6, "{name}: the pixel's hit {p:?}");
        let e = direct(&rig)[i];
        within(
            &format!("{name}, the term at the hit"),
            e,
            analytic(&light, floor, p, n),
            0.02,
        );
        within(&format!("{name}, the plan's figure"), e, want, 0.02);
    }
}

/// Phase 3 done-when 3: a second slab 4 voxels under the first (the light 20 above the floor,
/// the first slab at heights 9 to 11, the second 3 to 5): the second's top centre reads 0, and so
/// does the first's bottom face.
#[test]
fn an_entity_shadows_another_and_itself() {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    rig.frame
        .set_lights(&[Light::point([64.0, 32.0, 22.0], 1200.0)]);
    let s = slab(red, [8, 8, 1]);
    draw(
        &mut rig,
        &[
            (s.clone(), posed(&s, [64.0, 32.0, 10.0], Quat::IDENTITY)),
            (s.clone(), posed(&s, [64.0, 32.0, 4.0], Quat::IDENTITY)),
        ],
    );
    let camera = Camera {
        position: [64.0, 6.0, 7.0].into(),
        yaw: std::f32::consts::FRAC_PI_2,
        pitch: 0.0,
        vfov: 1.2,
    };
    rig.step(&camera);
    let hits = hits(&rig);
    let e = direct(&rig);
    let lower = nearest(&hits, [64.0, 30.0, 5.0], entity_hit(1));
    let upper = nearest(&hits, [64.0, 30.0, 9.0], entity_hit(0));
    for (name, (i, p), z) in [("lower top", lower, 5.0), ("upper bottom", upper, 9.0)] {
        println!("{name}: {} at {p:?}", e[i]);
        assert!(
            (p[2] - z).abs() < 1e-3 && (p[1] - 30.0).abs() < 2.0,
            "{name}: {p:?}"
        );
        assert_eq!(e[i], 0.0, "{name}");
    }
}

/// Phase 3 done-when 4: with no light, a 4-voxel cube of an emissive row (albedo 0, so only its
/// emission shows) as an entity and the same as world voxels at the same distance, either side
/// of the view: the two face centres read the same output within 1/255 a channel.
#[test]
fn an_entity_emitter_shows_as_a_world_emitter_does() {
    let (mut t, _) = table();
    let glow = t.push(Material {
        albedo: [0.0; 3],
        emission: [0.6, 0.3, 0.1],
        ..Material::named("glow")
    });
    let Some(mut rig) = rig_on(entity_box(&t), [W, H], None) else {
        return;
    };
    rig.frame.set_lighting(support::lights::box_lighting());
    support::bake::fill(&rig.world, [56, 40, 24], [60, 44, 28], glow);
    for _ in 0..2 {
        support::SyncNow::sync_now(&mut rig.gpu, &rig.world, &rig.device, &rig.queue)
            .expect("fits");
    }
    let cube = block([4; 3], glow, 1);
    draw(
        &mut rig,
        &[(
            cube.clone(),
            posed(&cube, [70.0, 42.0, 26.0], Quat::IDENTITY),
        )],
    );
    let camera = Camera {
        position: [64.0, 10.0, 26.0].into(),
        yaw: std::f32::consts::FRAC_PI_2,
        pitch: 0.0,
        vfov: 1.2,
    };
    rig.step(&camera);
    let img = rig.display();
    let world = rgb(&img, pixel_of(&camera, [58.0, 40.0, 26.0]));
    let entity = rgb(&img, pixel_of(&camera, [70.0, 40.0, 26.0]));
    println!("world {world:?}, entity {entity:?}");
    assert!(
        world.iter().any(|&c| c > 20),
        "the world emitter shows: {world:?}"
    );
    near("the entity emitter against the world's", entity, world);
}

/// Phase 3 done-when 5: with a part between a bouncing list light and a wall, every probe texel
/// and every baked store value is bit-identical to the run without the part, over 32 frames.
#[test]
fn the_probes_and_the_bake_see_no_entity() {
    let run = |with: bool| {
        let (mut rig, red) = rig()?;
        rig.frame.set_lighting(vxe_render::Lighting {
            lamp_bounce: true,
            ..support::lights::box_lighting()
        });
        let light = Light {
            flags: Light::ON | Light::SHADOWS | Light::BOUNCE,
            ..Light::point([40.0, 32.0, 20.0], 1200.0)
        };
        rig.frame.set_lights(&[light]);
        if with {
            let cube = block([8; 3], red, 1);
            draw(
                &mut rig,
                &[(
                    cube.clone(),
                    posed(&cube, [30.0, 32.0, 20.0], Quat::IDENTITY),
                )],
            );
        }
        let camera = level_camera(60.0, 20.0);
        let camera = Camera {
            yaw: std::f32::consts::PI,
            ..camera
        };
        for _ in 0..32 {
            rig.step(&camera);
        }
        let probes = support::lights::probe_words(&rig.device, &rig.queue, &rig.frame);
        rig.bake(&support::bake::test_counts(16));
        Some((probes, rig.blocks()))
    };
    let (Some(without), Some(with)) = (run(false), run(true)) else {
        return;
    };
    assert!(!without.0.is_empty(), "the probe volume runs");
    assert!(!without.1.is_empty(), "the store holds blocks");
    assert!(
        without.0 == with.0,
        "the probe texels differ with the entity"
    );
    assert!(
        without.1 == with.1,
        "the baked store differs with the entity"
    );
}

/// Phase 3 done-when 6: the walker in mid-stride on the lit box's floor, its eyes' light off and
/// the box's light of 1200 above and behind it, so its shadow falls on the floor before it; the
/// half lighting tier, 192 x 108, against the golden blessed once on lavapipe.
#[test]
fn walker_shadow_golden() {
    let Some((mut rig, _)) = rig() else {
        return;
    };
    rig.frame
        .set_resolution(&rig.device, LightingResolution::Half);
    rig.frame
        .set_lights(&[Light::point([112.0, 32.0, 22.0], 1200.0)]);
    let camera = Camera {
        position: [80.0, 32.0, 14.0].into(),
        yaw: 0.0,
        pitch: -0.35,
        vfov: 1.2,
    };
    let (body, eye) = walker::rows(rig.world.materials()).expect("the walker's rows");
    let model = Arc::new(walker::model(body, eye));
    let feet = [100.0, 32.0, 2.0];
    let pose = walker::pose(feet, walker::facing(feet, camera.position.to_array()), 0.5);
    draw(&mut rig, &[(model, pose)]);
    for _ in 0..8 {
        rig.step(&camera);
    }
    assert_golden_sized("walker_shadow.png", &rig.display());
}

/// Readings, run by hand (`--run-ignored only -E 'test(=reading::<name>)'`), on the adapter
/// `VXE_TEST_ADAPTER` names (unset: lavapipe). They print and assert nothing.
mod reading {
    use super::*;
    use vxe_render::{FRAME_PASSES, GpuWorld, MirrorConfig, PassTimer, Window};

    /// [`rig_on`]'s rig on the reading adapter, and the adapter's name.
    fn reading_rig(world: World) -> Option<(Rig, String)> {
        let (device, queue, name) = support::reading_device(64 << 20)?;
        let base = MirrorConfig::new(Window::Fixed(world.extent()));
        let cfg = MirrorConfig {
            budget_bytes: 64 << 20,
            ..base
        };
        let mut gpu = GpuWorld::with_config(&device, cfg);
        for _ in 0..2 {
            support::SyncNow::sync_now(&mut gpu, &world, &device, &queue).expect("fits");
        }
        let mut frame = support::bake::new_frame(&device, [W, H]);
        frame.set_lighting(support::lights::box_lighting());
        Some((
            Rig {
                device,
                queue,
                gpu,
                world,
                frame,
                cfg,
            },
            name,
        ))
    }

    fn walker_at(rig: &Rig, feet: [f32; 3], eye: [f32; 3]) -> (Arc<Model>, Pose) {
        let (body, e) = walker::rows(rig.world.materials()).expect("the walker's rows");
        let pose = walker::pose(feet, walker::facing(feet, eye), 0.0);
        (Arc::new(walker::model(body, e)), pose)
    }

    /// Phase 3 done-when 7, the fallback: a still walker beside a world emitter (a 4 x 4 x 8
    /// block of a warm emissive row) with no list light and the lamp off; at the walker's
    /// torso front facing the camera, the temporal pass's accumulated emissive part (its
    /// luminance) at frame 64 and its frame-to-frame rms over the 64 frames.
    #[test]
    #[ignore = "reading"]
    fn the_fallback_on_a_still_walker() {
        let (mut t, _) = table();
        let glow = t.push(Material {
            emission: [4.0, 2.5, 1.0],
            ..Material::named("glow")
        });
        let world = entity_box(&t);
        support::bake::fill(&world, [96, 25, 2], [100, 29, 10], glow);
        let Some((mut rig, adapter)) = reading_rig(world) else {
            return;
        };
        let camera = level_camera(84.0, 9.0);
        let entity = walker_at(&rig, [104.0, 32.0, 2.0], camera.position.to_array());
        draw(&mut rig, &[entity]);
        let mut series = Vec::new();
        let mut px = None;
        for _ in 0..64 {
            rig.step(&camera);
            let i = *px
                .get_or_insert_with(|| nearest(&hits(&rig), [102.0, 32.0, 7.0], entity_hit(0)).0);
            let a =
                vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.accumulated())[i];
            series.push(0.2126 * a[0] + 0.7152 * a[1] + 0.0722 * a[2]);
        }
        let diffs: Vec<f32> = series.windows(2).map(|w| w[1] - w[0]).collect();
        let rms = (diffs.iter().map(|d| d * d).sum::<f32>() / diffs.len() as f32).sqrt();
        println!(
            "{adapter}: the fallback at the torso's front, frame 64 {:.4}, mean of the last 32 {:.4}, frame-to-frame rms {rms:.4} ({:.2} % of the value)",
            series[63],
            series[32..].iter().sum::<f32>() / 32.0,
            100.0 * rms / series[63].max(1e-6)
        );
    }

    /// Phase 3 done-when 7, the cost: the light pass's time over 64 frames on the lit box (a
    /// light of 1200 at (64, 32, 22)) with 0 and 4 walkers standing around under the light, the
    /// median and the p90 in ms.
    #[test]
    #[ignore = "reading"]
    fn the_light_pass_cost_with_walkers() {
        let (t, _) = table();
        let Some((mut rig, adapter)) = reading_rig(entity_box(&t)) else {
            return;
        };
        let Some(timer) =
            PassTimer::with_passes(&rig.device, &rig.queue, FRAME_PASSES.len() as u32)
        else {
            println!("{adapter}: no timestamp queries");
            return;
        };
        let slot = FRAME_PASSES
            .iter()
            .position(|&n| n == "light")
            .expect("light");
        rig.frame
            .set_lights(&[Light::point([64.0, 32.0, 22.0], 1200.0)]);
        let camera = Camera {
            position: [40.0, 32.0, 30.0].into(),
            yaw: 0.0,
            pitch: -0.6,
            vfov: 1.2,
        };
        for n in [0usize, 4] {
            let entities: Vec<_> = (0..n)
                .map(|k| {
                    let a = std::f32::consts::TAU * k as f32 / 4.0;
                    let feet = [64.0 + 8.0 * a.cos(), 32.0 + 8.0 * a.sin(), 2.0];
                    walker_at(&rig, feet, camera.position.to_array())
                })
                .collect();
            draw(&mut rig, &entities);
            let mut ms = Vec::new();
            for _ in 0..64 {
                rig.frame.advance();
                let params =
                    rig.frame
                        .params(&rig.gpu, &camera, &vxe_render::LightParams::headlamp());
                let mut enc = rig.device.create_command_encoder(&Default::default());
                rig.frame.encode(
                    &rig.device,
                    &rig.queue,
                    &mut enc,
                    &rig.gpu,
                    &params,
                    Some(&timer),
                );
                timer.resolve(&mut enc);
                rig.queue.submit([enc.finish()]);
                ms.push(timer.read_all_ms(&rig.device)[slot]);
            }
            ms.sort_by(f64::total_cmp);
            println!(
                "{adapter}: {n} walkers, light pass median {:.3} ms, p90 {:.3} ms (64 frames, {W} x {H})",
                ms[32], ms[57]
            );
        }
    }
}
