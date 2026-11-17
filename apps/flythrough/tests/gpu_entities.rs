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

/// Plan 0062 Phase 6d: [`vxe_render::Frame::prepare_entities`] builds every pipeline a frame
/// that draws an entity uses, so the first frame that draws a part creates none, and
/// `Frame::new` builds none of them itself. The process's compute-pipeline count
/// (`debug::compute_pipelines_created`, which every compute pipeline of `vxe-render` passes):
/// the call adds exactly the five it owns (the hit pass's three `ENTITIES`-true pipelines, the
/// entity light pass and the no-entity light pass), none of which the frame's creation built.
/// Then a frame with no entity, and the 8-voxel cube of `an_entity_hides_the_wall_behind_it`
/// drawn on the next two: the count is the same as after the call, and the part shows (one
/// part drawn; the centre pixel's hit is the entity's).
#[test]
fn the_first_frame_with_an_entity_builds_no_pipeline() {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    let made = vxe_render::debug::compute_pipelines_created();
    rig.frame.prepare_entities(&rig.device);
    let prepared = vxe_render::debug::compute_pipelines_created();
    assert_eq!(
        prepared - made,
        5,
        "the call builds the entity set and both light pipelines, none built by the frame"
    );
    let camera = level_camera(86.0, 26.0);
    rig.step(&camera);
    let cube = block([8; 3], red, 1);
    draw(
        &mut rig,
        &[(
            cube.clone(),
            posed(&cube, [110.0, 32.0, 26.0], Quat::IDENTITY),
        )],
    );
    rig.step(&camera);
    rig.step(&camera);
    let after = vxe_render::debug::compute_pipelines_created();
    println!(
        "compute pipelines: {made} at the frame's creation, {prepared} after the call, \
         {after} after the part's frames"
    );
    assert_eq!(after, prepared, "pipelines built after the call");
    assert_eq!(rig.frame.entity_mirror().parts_drawn(), 1);
    let rec = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let centre = pixel_of(&camera, [106.0, 32.0, 26.0]);
    assert!(
        entity_hit(0)(&rec[centre]),
        "the centre pixel's record {:?}",
        rec[centre]
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
            free: 4096,
            largest: 4096,
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

/// Plan 0063 Phase 3 done-when 4: the sample `walker.rig` at tick 10 of `walk`, drawn between
/// no two ticks (`DisplayPose::of`), in [`walker_shadow_golden`]'s lit box and view.
#[test]
fn walker_walk_golden() {
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
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities/walker/walker.rig");
    let mut viewer = flythrough::viewer::Viewer::open(&path, Some("walk".into())).expect("opens");
    assert_eq!(viewer.build_pending(rig.world.materials()), None);
    let l = viewer.loaded.clone().expect("built");
    let walk = &l.clips.iter().find(|(n, _)| n == "walk").expect("walk").1;
    let feet = [100.0, 32.0, 2.0];
    let yaw = walker::facing(feet, camera.position.to_array());
    let at10 = vxe_entity::sample(&l.model, &l.rest, walk, 10);
    let pose = flythrough::viewer::placed(&at10, feet, yaw);
    draw(&mut rig, &[(l.model.clone(), pose)]);
    for _ in 0..8 {
        rig.step(&camera);
    }
    assert_golden_sized("walker_walk.png", &rig.display());
}

/// Plan 0062's default (`Lighting::default().entity_light` is `EntityLight::World`): an
/// entity's emissive part is the baked light of the nearest world face behind it of its
/// direction. The box with its ceiling lowered to height 24 and a glowing plate (emission 4)
/// set into it over `[48, 80) x [16, 48)` at heights 24 to 26, so the plate's only face in the
/// air is its -z face at height 24; baked at 64 samples a brick direction; lamp off and no
/// list light, so the temporal pass writes the store's value with no history. An 8-voxel red
/// cube (identity turn) centred at (64, 32, 12), seen from (56, 32, 23) 41 degrees down.
///
/// The expected face, from the geometry: the cube's top is the plane z 16, its normal +z (face
/// 5 of the store). The rule reads the voxel half a voxel behind the hit, z 15, in brick z 1
/// (voxels 8 to 15); the box holds no +z face between the floor and the ceiling, so that
/// brick's cells hold no value of face 5 and the rule steps one cell (8 voxels) against the
/// normal, to z 7 in brick z 0, whose cells hold the floor's +z faces (rock at z 1 under the
/// air from z 2). So at a top-face pixel whose hit is (x, y, 16) the expected value is
/// [`support::bake::brick_value`] of voxel (floor x, floor y, 7), face 5, from the store's
/// records read back after the bake (the 2 x 2 bilinear in the face's plane that
/// `store_brick_value` takes), times exposure 1. The cube's top is axis-aligned, so the shading
/// normal is the face's and no normal mix applies.
///
/// Asserts, at every top-face pixel (at least 100: the 8 x 8 top, 10.6 voxels from the eye,
/// spans about 68 x 44 pixels at 108 rows over 1.2 rad): (a) the accumulated rgb equals the
/// expected value within one half-precision step (`max(want / 1024, 1 / 16384)`, as
/// `a_baked_view_is_final_on_its_first_frame` holds a face's read), and the value is lit (each
/// channel over 0); (b) frames 2 to 8 of the still view are bit-identical to frame 1 (no random
/// stream reaches the pixel); (c) the light pass's emissive sample (the irradiance target's rgb)
/// is 0 at every one of them; under `EntityLight::Own` its mean over them is over 0 in each
/// channel and fewer than half of them read 0 (a pixel's draw is random and may find no
/// candidate: 87 of 2718 on lavapipe at the commit; under `World` all of them read 0, so a
/// half separates the two modes with room either side).
#[test]
fn an_entity_reads_the_baked_light_of_the_world_face_behind_it() {
    assert_eq!(
        vxe_render::Lighting::default().entity_light,
        vxe_render::EntityLight::World
    );
    let (mut t, red) = table();
    let glow = t.push(Material {
        albedo: [0.0; 3],
        emission: [4.0, 4.0, 4.0],
        ..Material::named("glow")
    });
    let world = entity_box(&t);
    // A fill writes air only: the plate first, then the rock around and above it.
    support::bake::fill(&world, [48, 16, 24], [80, 48, 26], glow);
    support::bake::fill(&world, [2, 2, 24], [126, 62, 50], ROCK);
    assert_eq!(world.material_at([60, 30, 25]), Some(glow));
    let Some(mut rig) = rig_on(world, [W, H], None) else {
        return;
    };
    rig.frame.set_lighting(support::lights::box_lighting());
    assert_eq!(
        rig.frame.lighting().entity_light,
        vxe_render::EntityLight::World
    );
    rig.bake(&support::bake::test_counts(64));
    let cells = rig.cells();
    let cube = block([8; 3], red, 1);
    draw(
        &mut rig,
        &[(
            cube.clone(),
            posed(&cube, [64.0, 32.0, 12.0], Quat::IDENTITY),
        )],
    );
    let camera = Camera {
        position: [56.0, 32.0, 23.0].into(),
        yaw: 0.0,
        pitch: -0.72,
        vfov: 1.2,
    };
    rig.step(&camera);
    let top: Vec<(usize, [f32; 3])> = hits(&rig)
        .into_iter()
        .filter(|h| entity_hit(0)(&h.2) && (h.1[2] - 16.0).abs() < 1e-3)
        .map(|h| (h.0, h.1))
        .collect();
    assert!(top.len() >= 100, "{} top-face pixels", top.len());
    let first = rig.accum();
    let irradiance = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.irradiance());
    let (mut lo, mut hi) = (f32::MAX, 0f32);
    for &(i, p) in &top {
        let v = [p[0].floor() as i32, p[1].floor() as i32, 7];
        let want = support::bake::brick_value(&cells, v, 5).expect("the floor's brick holds +z");
        for k in 0..3 {
            assert!(want[k] > 0.0, "the floor under the plate is lit: {want:?}");
            let step = (want[k] / 1024.0).max(1.0 / 16384.0);
            assert!(
                (first[i][k] - want[k]).abs() <= step,
                "pixel {i} at {p:?} channel {k}: {} against {}",
                first[i][k],
                want[k]
            );
        }
        assert_eq!(
            [irradiance[i][0], irradiance[i][1], irradiance[i][2]],
            [0.0; 3],
            "the light pass's emissive sample at pixel {i}"
        );
        lo = lo.min(want[1]);
        hi = hi.max(want[1]);
    }
    println!(
        "{} top-face pixels read the floor's value, green {lo:.5} to {hi:.5}",
        top.len()
    );
    for n in 2..=8 {
        rig.step(&camera);
        assert!(rig.accum() == first, "frame {n} against frame 1");
    }

    rig.frame.set_lighting(vxe_render::Lighting {
        entity_light: vxe_render::EntityLight::Own,
        ..rig.frame.lighting()
    });
    rig.step(&camera);
    let own = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.irradiance());
    let dark = top
        .iter()
        .filter(|&&(i, _)| own[i][..3].iter().any(|&c| c <= 0.0))
        .count();
    let mean: [f32; 3] = std::array::from_fn(|k| {
        top.iter().map(|&(i, _)| own[i][k]).sum::<f32>() / top.len() as f32
    });
    println!(
        "under own: {dark} of {} top-face pixels drew no light, mean {mean:?}",
        top.len()
    );
    assert!(mean.iter().all(|&c| c > 0.0), "under own: mean {mean:?}");
    assert!(
        2 * dark < top.len(),
        "under own {dark} of {} top-face pixels drew no light",
        top.len()
    );
}

// ---- Phase 4: motion ----

/// The motion rig's light: 1200, 20 voxels above the floor at (64, 32).
fn motion_light() -> Light {
    Light::point([64.0, 32.0, 22.0], 1200.0)
}

/// From over the box's +x end, looking back along -x and down: the floor from x 2 to about
/// 116 under the light, and the part's path.
fn motion_camera() -> Camera {
    Camera {
        position: [124.0, 32.0, 46.0].into(),
        yaw: std::f32::consts::PI,
        pitch: -0.8,
        vfov: 1.2,
    }
}

/// The moving part's centre on move frame `k` (0 the still start): an 8-voxel cube 6 voxels
/// above the floor (heights 8 to 16), from x 52 along +x, 4 voxels a frame.
fn part_centre(k: usize) -> [f32; 3] {
    [52.0 + 4.0 * k as f32, 32.0, 12.0]
}

/// The world box of an 8-voxel cube centred at `c`, grown by `m` voxels on every side.
fn cube_box(c: [f32; 3], m: f32) -> [[f32; 3]; 2] {
    [c.map(|v| v - 4.0 - m), c.map(|v| v + 4.0 + m)]
}

/// Whether the segment from `a` to `b` meets box `bx` (a slab test).
fn segment_meets(a: [f32; 3], b: [f32; 3], bx: [[f32; 3]; 2]) -> bool {
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for k in 0..3 {
        let d = b[k] - a[k];
        if d.abs() < 1e-9 {
            if a[k] < bx[0][k] || a[k] > bx[1][k] {
                return false;
            }
            continue;
        }
        let (u, v) = ((bx[0][k] - a[k]) / d, (bx[1][k] - a[k]) / d);
        t0 = t0.max(u.min(v));
        t1 = t1.min(u.max(v));
    }
    t0 <= t1
}

/// Whether the floor point `p`'s shadow ray (from half a voxel above it, as the light pass
/// starts it) to `light` meets the 8-voxel cube at `c` grown by `m`.
fn floor_shadowed(p: [f32; 3], light: &Light, c: [f32; 3], m: f32) -> bool {
    segment_meets([p[0], p[1], p[2] + 0.5], light.pos, cube_box(c, m))
}

fn on_floor(p: [f32; 3]) -> bool {
    (p[2] - 2.0).abs() < 1e-3
}

/// The temporal pass's accumulated direct part's luminance (`a`) per pixel.
fn direct_acc(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    frame: &vxe_render::Frame,
) -> Vec<f32> {
    vxe_render::debug::read_float(device, queue, frame.direct_accumulated())
        .iter()
        .map(|p| p[3])
        .collect()
}

/// The light pass's fresh bit (its irradiance's `a`) per pixel.
fn fresh(rig: &Rig) -> Vec<bool> {
    vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.irradiance())
        .iter()
        .map(|p| p[3] > 0.5)
        .collect()
}

/// Uploads `model` to `frame`'s mirror and returns a set holding it posed `pose`, its id
/// and handle.
fn place(
    rig: &Rig,
    frame: &mut vxe_render::Frame,
    model: &Arc<Model>,
    pose: Pose,
) -> (EntitySet, EntityId, ModelHandle) {
    let h = frame
        .entity_mirror_mut()
        .upload_model(&rig.queue, model, rig.world.materials())
        .expect("the pool has room");
    let mut set = EntitySet::new();
    let id = set.insert(model.clone(), pose).expect("the pose fits");
    (set, id, h)
}

/// Phase 4 done-when 1: on the lit box, an 8-voxel cube part 6 voxels above the floor, still
/// for 32 frames, then moved 4 voxels a frame along +x for 16 frames. On every frame of the
/// move, read from the temporal pass's accumulated direct part: (a) every floor pixel whose
/// shadow ray met the part one frame before and misses it now (by half a voxel either way)
/// reads its analytic term within 2 %; (b) every pixel that showed the part one frame before
/// and shows the world now, outside the part's shadow, reads the value of a run without the
/// entity within 2 %; (c) the pixel nearest the centre of the part's top face, followed with
/// it (within 1.5 voxels: a pixel spans about one there), reads its analytic term at its hit
/// within 2 %.
#[test]
fn a_moving_entity_leaves_no_ghost_and_no_stale_shadow() {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    let light = motion_light();
    let floor = rig.frame.lighting().light_floor;
    rig.frame.set_lights(&[light]);
    let camera = motion_camera();
    // The run without the entity: its direct part is still, so two frames settle it.
    rig.step(&camera);
    rig.step(&camera);
    let reference = direct_acc(&rig.device, &rig.queue, &rig.frame);
    let cube = block([8; 3], red, 1);
    let mut frame = std::mem::replace(
        &mut rig.frame,
        support::bake::new_frame(&rig.device, [W, H]),
    );
    let (mut set, id, h) = place(
        &rig,
        &mut frame,
        &cube,
        posed(&cube, part_centre(0), Quat::IDENTITY),
    );
    rig.frame = frame;
    for _ in 0..32 {
        rig.frame.set_entities(&rig.queue, &set, &[(id, h)]);
        rig.step(&camera);
    }
    let mut prev_hits = hits(&rig);
    let (mut a_count, mut b_count) = (0usize, 0usize);
    let (mut a_worst, mut b_worst) = (0f32, 0f32);
    for k in 1..=16 {
        set.set_pose(id, posed(&cube, part_centre(k), Quat::IDENTITY))
            .expect("the pose fits");
        rig.frame.set_entities(&rig.queue, &set, &[(id, h)]);
        rig.step(&camera);
        assert!(rig.frame.entities_moved(), "move frame {k}");
        let now_hits = hits(&rig);
        let e = direct_acc(&rig.device, &rig.queue, &rig.frame);
        let (was, now) = (part_centre(k - 1), part_centre(k));
        let mut was_entity = vec![false; (W * H) as usize];
        for h in &prev_hits {
            was_entity[h.0] = h.2[3] & 32 != 0;
        }
        for &(i, p, r) in &now_hits {
            if !world_hit(&r) {
                continue;
            }
            if on_floor(p)
                && floor_shadowed(p, &light, was, -0.5)
                && !floor_shadowed(p, &light, now, 0.5)
            {
                a_count += 1;
                let want = analytic(&light, floor, p, [0.0, 0.0, 1.0]);
                a_worst = a_worst.max((e[i] / want - 1.0).abs());
                assert!(
                    (e[i] / want - 1.0).abs() <= 0.02,
                    "(a) move frame {k}: the floor at {p:?} reads {} against {want}",
                    e[i]
                );
            }
            if was_entity[i] && !segment_meets(p, light.pos, cube_box(now, 1.0)) {
                b_count += 1;
                b_worst = b_worst.max((e[i] / reference[i] - 1.0).abs());
                assert!(
                    (e[i] / reference[i] - 1.0).abs() <= 0.02,
                    "(b) move frame {k}: the world at {p:?} reads {} against {} without the entity",
                    e[i],
                    reference[i]
                );
            }
        }
        let top = [now[0], now[1], 16.0];
        let (i, p) = nearest(&now_hits, top, entity_hit(0));
        let d = (0..3).map(|j| (p[j] - top[j]).powi(2)).sum::<f32>().sqrt();
        // A pixel spans about a voxel on the top face from the camera.
        assert!(d < 1.5, "(c) move frame {k}: the top's pixel hit {p:?}");
        within(
            &format!("(c) move frame {k}"),
            e[i],
            analytic(&light, floor, p, [0.0, 0.0, 1.0]),
            0.02,
        );
        prev_hits = now_hits;
    }
    println!(
        "(a) {a_count} floor pixels left by the shadow, worst {:.3} %; (b) {b_count} pixels left by the part, worst {:.3} %",
        100.0 * a_worst,
        100.0 * b_worst
    );
    assert!(a_count > 100 && b_count > 50, "({a_count}, {b_count})");
}

/// Phase 4 done-when 2: over 32 still frames (the part set every frame) `entities_moved()` is
/// false and no pixel is fresh; on the first frame of a 4-voxel move every floor pixel inside
/// the old or the new shadow (by half a voxel) is fresh, and no floor pixel 40 voxels or more
/// from both shadows is.
#[test]
fn a_still_entity_marks_nothing_fresh() {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    let light = motion_light();
    rig.frame.set_lights(&[light]);
    let camera = motion_camera();
    let cube = block([8; 3], red, 1);
    let mut frame = std::mem::replace(
        &mut rig.frame,
        support::bake::new_frame(&rig.device, [W, H]),
    );
    let (mut set, id, h) = place(
        &rig,
        &mut frame,
        &cube,
        posed(&cube, part_centre(0), Quat::IDENTITY),
    );
    rig.frame = frame;
    for f in 0..32 {
        rig.frame.set_entities(&rig.queue, &set, &[(id, h)]);
        rig.step(&camera);
        assert!(!rig.frame.entities_moved(), "still frame {f}");
        let n = fresh(&rig).iter().filter(|&&b| b).count();
        assert_eq!(n, 0, "still frame {f}: {n} fresh pixels");
    }
    set.set_pose(id, posed(&cube, part_centre(1), Quat::IDENTITY))
        .expect("the pose fits");
    rig.frame.set_entities(&rig.queue, &set, &[(id, h)]);
    rig.step(&camera);
    assert!(rig.frame.entities_moved());
    let marks = fresh(&rig);
    let floor: Vec<(usize, [f32; 3])> = hits(&rig)
        .into_iter()
        .filter(|h| world_hit(&h.2) && on_floor(h.1))
        .map(|h| (h.0, h.1))
        .collect();
    let shadowed: Vec<[f32; 3]> = floor
        .iter()
        .filter(|(_, p)| {
            floor_shadowed(*p, &light, part_centre(0), -0.5)
                || floor_shadowed(*p, &light, part_centre(1), -0.5)
        })
        .map(|h| h.1)
        .collect();
    let mut inside = 0;
    for &(i, p) in &floor {
        if floor_shadowed(p, &light, part_centre(0), -0.5)
            || floor_shadowed(p, &light, part_centre(1), -0.5)
        {
            inside += 1;
            assert!(marks[i], "the shadowed floor at {p:?} is not fresh");
        }
    }
    let mut far = 0;
    for &(i, p) in &floor {
        let d = shadowed
            .iter()
            .map(|q| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt())
            .fold(f32::MAX, f32::min);
        if d >= 40.0 {
            far += 1;
            assert!(
                !marks[i],
                "the floor at {p:?}, {d} from the shadows, is fresh"
            );
        }
    }
    println!("{inside} floor pixels in the shadows, {far} 40 voxels or more from them");
    assert!(inside > 100 && far > 100, "({inside}, {far})");
}

/// The FSR 2 motion vectors at every hit of `frame`'s last frame in internal pixels, and the
/// expected motion of each hit whose point was at `before(p)` the frame before: where that
/// point projects through `view` less the jitter `j`, less the pixel's centre.
fn motion_and_want(
    rig: &Rig,
    frame: &vxe_render::Frame,
    view: &vxe_render::View,
    j: [f32; 2],
    before: impl Fn(usize, [f32; 3], [u16; 4]) -> Option<[f32; 3]>,
) -> Vec<(usize, [f64; 2], [f64; 2])> {
    let size = frame.internal_size();
    let motion = support::rigs::read_f32_texels(
        &rig.device,
        &rig.queue,
        frame.fsr2().expect("FSR 2").motion(),
    );
    let pos = vxe_render::debug::read_rgba32f(&rig.device, &rig.queue, frame.hit_positions());
    let rec = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, frame.hit_records());
    let mut out = Vec::new();
    for i in 0..pos.len() {
        if rec[i][3] & 1 == 0 {
            continue;
        }
        let p = [pos[i][0], pos[i][1], pos[i][2]];
        let Some(b) = before(i, p, rec[i]) else {
            continue;
        };
        let Some(q) = support::rigs::project(view, b) else {
            continue;
        };
        let (x, y) = (i as u32 % size[0], i as u32 / size[0]);
        let got = [0, 1].map(|k| f64::from(motion[2 * i + k]) * f64::from(size[k]));
        let want = [0, 1].map(|k| q[k] - f64::from(j[k]) - (f64::from([x, y][k]) + 0.5));
        out.push((i, got, want));
    }
    out
}

/// Phase 4 done-when 3 (written here, beside the entity rigs, not in `gpu_fsr2.rs`): under FSR
/// 2 at 1.5 with a still camera, an 8-voxel cube moved 4 voxels sideways in one frame: the
/// motion vector at the pixel nearest its near face's centre equals the projected
/// displacement within 0.1 internal pixel, and at every world pixel it is 0 exactly; a 16 x 4
/// x 4 bar turning 10 degrees a frame about the view axis through its centre: the vector at
/// the hit nearest its tip matches the tip's projected displacement within 0.25 pixel.
#[test]
fn fsr2_motion_follows_a_moving_part() {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    let camera = level_camera(86.0, 26.0);
    let view = vxe_render::View::of_camera(&camera, [0, 0, 128, 72]);
    let light = vxe_render::LightParams::headlamp();
    let jitter = |rig: &mut Rig| rig.frame.params(&rig.gpu, &camera, &light).views[0].jitter;

    let cube = block([8; 3], red, 1);
    // Both parts run on the rig's frame, set up afresh for each: a new frame's pipelines cost
    // more than the runs.
    reset_frame(&mut rig, [128, 72], Some(support::rigs::fsr2_kind()));
    let (mut set, id, h) = place_on(
        &mut rig,
        &cube,
        posed(&cube, [110.0, 32.0, 26.0], Quat::IDENTITY),
    );
    for _ in 0..3 {
        rig.frame.set_entities(&rig.queue, &set, &[(id, h)]);
        support::lights::step(&mut rig, &camera);
    }
    set.set_pose(id, posed(&cube, [110.0, 28.0, 26.0], Quat::IDENTITY))
        .expect("the pose fits");
    rig.frame.set_entities(&rig.queue, &set, &[(id, h)]);
    support::lights::step(&mut rig, &camera);
    let j = jitter(&mut rig);
    let all = motion_and_want(&rig, &rig.frame, &view, j, |_, p, r| {
        Some(if r[3] & 32 != 0 {
            [p[0], p[1] + 4.0, p[2]]
        } else {
            p
        })
    });
    let pos = vxe_render::debug::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions());
    let rec = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let face = [106.0, 28.0, 26.0];
    let (centre, _) = all
        .iter()
        .filter(|m| rec[m.0][3] & 32 != 0)
        .map(|m| {
            let d = (0..3).map(|k| (pos[m.0][k] - face[k]).powi(2)).sum::<f32>();
            (m, d)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("the part shows");
    let err = (0..2)
        .map(|k| (centre.1[k] - centre.2[k]).abs())
        .fold(0.0, f64::max);
    println!(
        "the cube's centre: motion {:?} against {:?} internal pixels",
        centre.1, centre.2
    );
    assert!(err <= 0.1, "off by {err} internal pixels");
    let world: Vec<_> = all.iter().filter(|m| rec[m.0][3] & 32 == 0).collect();
    assert!(world.len() > 1000, "{} world pixels", world.len());
    for m in &world {
        assert_eq!(m.1, [0.0, 0.0], "a world pixel's motion at {:?}", pos[m.0]);
    }

    let bar = block([4, 16, 4], red, 1);
    reset_frame(&mut rig, [128, 72], Some(support::rigs::fsr2_kind()));
    let c = [110.0f32, 32.0, 26.0];
    let step = 10f32.to_radians();
    let (mut set, id, h) = place_on(&mut rig, &bar, posed(&bar, c, Quat::IDENTITY));
    for k in 0..4 {
        set.set_pose(id, posed(&bar, c, walker::turn(0, step * k as f32)))
            .expect("the pose fits");
        rig.frame.set_entities(&rig.queue, &set, &[(id, h)]);
        support::lights::step(&mut rig, &camera);
    }
    let j = jitter(&mut rig);
    // The frame before's point: turned back 10 degrees about x through the centre.
    let (s, co) = (-step).sin_cos();
    let back = |p: [f32; 3]| {
        let (y, z) = (p[1] - c[1], p[2] - c[2]);
        [p[0], c[1] + y * co - z * s, c[2] + y * s + z * co]
    };
    let all = motion_and_want(&rig, &rig.frame, &view, j, |_, p, r| {
        (r[3] & 32 != 0).then(|| back(p))
    });
    let pos = vxe_render::debug::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions());
    // The tip: 7.5 voxels along the bar's long axis (y turned 30 degrees about x) on its near
    // face (x 108).
    let a = 3.0 * step;
    let tip = [108.0, c[1] + 7.5 * a.cos(), c[2] + 7.5 * a.sin()];
    let (m, d) = all
        .iter()
        .map(|m| {
            (
                m,
                (0..3)
                    .map(|k| (pos[m.0][k] - tip[k]).powi(2))
                    .sum::<f32>()
                    .sqrt(),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("the bar shows");
    let err = (0..2).map(|k| (m.1[k] - m.2[k]).abs()).fold(0.0, f64::max);
    println!(
        "the bar's tip ({d:.2} voxels from it): motion {:?} against {:?} internal pixels",
        m.1, m.2
    );
    assert!(d < 1.0, "the tip's nearest hit is {d} voxels off");
    assert!(err <= 0.25, "off by {err} internal pixels");
}

/// The mean output luminance (0..255) over the 5 x 5 output pixels around `at`.
fn luma_around(rig: &Rig, frame: &vxe_render::Frame, at: [usize; 2]) -> f64 {
    let img = vxe_render::debug::read_rgba8(&rig.device, &rig.queue, frame.target());
    let mut s = 0.0;
    for y in at[1] - 2..=at[1] + 2 {
        for x in at[0] - 2..=at[0] + 2 {
            let p = &img[4 * (y * W as usize + x)..];
            s += 0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2]);
        }
    }
    s / 25.0
}

/// Sets `f` up as `support::lights::box_frame` makes a frame (no jitter, full tier, the box's
/// lighting) under `kind`, `internal` pixels for a 192 x 108 output, with no entity drawn and
/// no list light. `set_sizes` replaces every target and drops the history, so the frame starts
/// afresh as a new one does; only its pipelines (and FSR 2's, once made) are kept, which is
/// what a new frame costs on lavapipe (a few seconds each). The tests that reset run on the
/// rig's own frame, so they make no frame beyond it.
fn reset_frame(rig: &mut Rig, internal: [u32; 2], kind: Option<vxe_render::UpscalerKind>) {
    let (device, f) = (&rig.device, &mut rig.frame);
    f.set_entities(&rig.queue, &EntitySet::new(), &[]);
    f.set_lights(&[]);
    f.set_jitter(device, false);
    f.set_upscaler_kind(device, kind.unwrap_or(vxe_render::UpscalerKind::Off));
    f.set_resolution(device, LightingResolution::Full);
    f.set_sizes(device, internal, [W, H]);
    f.set_lighting(support::lights::box_lighting());
    f.set_dt(1.0 / 60.0);
    f.reset_exposure();
}

/// The rig's frame set up for the trail ([`reset_frame`]): FSR 2 or our upscaler at 1.5
/// (`kind`), or with none the frame at the output size.
fn trail_frame(rig: &mut Rig, kind: Option<vxe_render::UpscalerKind>) {
    let internal = if kind.is_some() { [128, 72] } else { [W, H] };
    reset_frame(rig, internal, kind);
}

/// [`place`] on the rig's own frame.
fn place_on(rig: &mut Rig, model: &Arc<Model>, pose: Pose) -> (EntitySet, EntityId, ModelHandle) {
    let h = rig
        .frame
        .entity_mirror_mut()
        .upload_model(&rig.queue, model, rig.world.materials())
        .expect("the pool has room");
    let mut set = EntitySet::new();
    let id = set.insert(model.clone(), pose).expect("the pose fits");
    (set, id, h)
}

/// The trail behind done-when 1's move under `kind`, at output pixel `at` (where the part's
/// start centre shows): the luminance there with the part held at its start (32 frames), and
/// settled with no entity (32 frames); then the move of `moves` frames from a 32-frame still
/// start and 16 frames after it, the distance from the no-entity luminance on each frame from
/// the first on which the part and its shadow have left the pixel (`clear`). Returns the
/// frames from `clear` until that distance is under a tenth of the step (the settle rule of
/// `fsr2_keeps_pace_with_a_sweeping_headlamp_and_a_lamp_step`), and the step. Both runs use
/// the rig's frame, set up afresh for each ([`trail_frame`]).
fn trail_after_move(
    rig: &mut Rig,
    red: MaterialId,
    kind: Option<vxe_render::UpscalerKind>,
    at: [usize; 2],
    moves: usize,
    clear: usize,
) -> (Option<usize>, f64) {
    let camera = motion_camera();
    let cube = block([8; 3], red, 1);
    let mut run = |with: bool| {
        trail_frame(rig, kind);
        rig.frame.set_lights(&[motion_light()]);
        let placed =
            with.then(|| place_on(rig, &cube, posed(&cube, part_centre(0), Quat::IDENTITY)));
        for _ in 0..32 {
            if let Some((set, id, h)) = &placed {
                rig.frame.set_entities(&rig.queue, set, &[(*id, *h)]);
            }
            support::lights::step(rig, &camera);
        }
        (luma_around(rig, &rig.frame, at), placed)
    };
    let (reference, _) = run(false);
    let (held, placed) = run(true);
    let (mut set, id, h) = placed.expect("placed");
    let mut series = Vec::new();
    for k in 1..=moves + 16 {
        let c = part_centre(k.min(moves));
        set.set_pose(id, posed(&cube, c, Quat::IDENTITY))
            .expect("the pose fits");
        rig.frame.set_entities(&rig.queue, &set, &[(id, h)]);
        support::lights::step(rig, &camera);
        if k >= clear {
            series.push((luma_around(rig, &rig.frame, at) - reference).abs());
        }
    }
    let step = (held - reference).abs();
    println!(
        "{:?}: at {at:?} held {held:.1}, without the entity {reference:.1}/255; from move frame {clear}: {:.2?}",
        kind.map(|k| k.name()),
        series
    );
    (support::rigs::settle_frames(&series, step), step)
}

/// The output pixel where the part's start centre shows, and the first move frame on which the
/// part and its shadow have left the floor point that pixel's ray reaches (`trail_after_move`):
/// the ray from the camera through (52, 32, 12) meets the floor at x 30.8, which the part's
/// shadow leaves at move frame 2 and the part itself at move frame 4.
fn trail_pixel() -> ([usize; 2], usize) {
    let i = pixel_of(&motion_camera(), part_centre(0));
    ([i % W as usize, i / W as usize], 4)
}

/// Phase 4 done-when 4 (written here, beside the entity rigs, not in `gpu_fsr2.rs`): after done-
/// when 1's move, the output at the pixel the part left reaches a tenth of the step from the
/// run without the entity under FSR 2 at 1.5 in at most 2 frames more than the frame at the
/// output size takes (`fsr2_keeps_pace_with_a_sweeping_headlamp_and_a_lamp_step`'s rule); the
/// same under our upscaler at 1.5. The trails' lengths are printed. The gate's variant moves 8
/// frames; `fsr2_leaves_no_trail_behind_a_moving_entity_over_16_frames` moves 16.
#[test]
fn fsr2_leaves_no_trail_behind_a_moving_entity() {
    trail_rule(8);
}

/// Phase 4 done-when 4 with done-when 1's 16-frame move.
#[test]
#[ignore = "soak"]
fn fsr2_leaves_no_trail_behind_a_moving_entity_over_16_frames() {
    trail_rule(16);
}

fn trail_rule(moves: usize) {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    let (at, clear) = trail_pixel();
    // Every run on the rig's frame: a new frame's pipelines cost more than a run's frames.
    let (native, _) = trail_after_move(&mut rig, red, None, at, moves, clear);
    let native = native.expect("the frame at the output size settles");
    for kind in [
        support::rigs::fsr2_kind(),
        vxe_render::UpscalerKind::Own(vxe_render::Upscaler::default()),
    ] {
        let (s, step) = trail_after_move(&mut rig, red, Some(kind), at, moves, clear);
        println!(
            "{}: the trail is {s:?} frames against {native} at the output size (step {step:.1}/255)",
            kind.name()
        );
        assert!(step > 5.0, "the part changes the pixel: step {step}");
        let s = s.unwrap_or_else(|| panic!("{} did not settle", kind.name()));
        assert!(s <= native + 2, "{}: {s} against {native} + 2", kind.name());
    }
}

/// Phase 4 done-when 5: two parts of the same albedo passing through each other along x, one
/// a voxel above the other (tops at heights 12 and 13), under a light at height 12.5 off to the
/// side: the lower top is lit, the upper top faces away. On every frame of the pass, every
/// pixel of the upper top reads (the temporal pass's accumulated direct part) under 2 % of the
/// lit top's mean.
#[test]
fn history_does_not_cross_between_instances() {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    let light = Light::point([64.0, 52.0, 12.5], 1200.0);
    rig.frame.set_lights(&[light]);
    let camera = Camera {
        position: [64.0, 32.0, 46.0].into(),
        yaw: 0.0,
        pitch: -1.45,
        vfov: 1.2,
    };
    let cube = block([8; 3], red, 1);
    let mut frame = std::mem::replace(
        &mut rig.frame,
        support::bake::new_frame(&rig.device, [W, H]),
    );
    frame.set_lights(&[light]);
    let at = |k: i32| {
        (
            [40.0 + 2.0 * k as f32, 32.0, 8.0],
            [88.0 - 2.0 * k as f32, 32.0, 9.0],
        )
    };
    let (lo, hi) = at(0);
    let h = frame
        .entity_mirror_mut()
        .upload_model(&rig.queue, &cube, rig.world.materials())
        .expect("room");
    let mut set = EntitySet::new();
    let a = set
        .insert(cube.clone(), posed(&cube, lo, Quat::IDENTITY))
        .expect("fits");
    let b = set
        .insert(cube.clone(), posed(&cube, hi, Quat::IDENTITY))
        .expect("fits");
    rig.frame = frame;
    let mut frames = 0;
    for k in 0..=24 {
        let (lo, hi) = at(k);
        set.set_pose(a, posed(&cube, lo, Quat::IDENTITY))
            .expect("fits");
        set.set_pose(b, posed(&cube, hi, Quat::IDENTITY))
            .expect("fits");
        rig.frame.set_entities(&rig.queue, &set, &[(a, h), (b, h)]);
        rig.step(&camera);
        let e = direct(&rig);
        let acc = direct_acc(&rig.device, &rig.queue, &rig.frame);
        let hs = hits(&rig);
        let top = |k: u16, z: f32| {
            hs.iter()
                .filter(move |h| entity_hit(k)(&h.2) && (h.1[2] - z).abs() < 1e-3)
                .map(|h| h.0)
                .collect::<Vec<_>>()
        };
        let (lit, unlit) = (top(0, 12.0), top(1, 13.0));
        if lit.is_empty() || unlit.is_empty() {
            continue;
        }
        frames += 1;
        let mean = lit.iter().map(|&i| e[i]).sum::<f32>() / lit.len() as f32;
        assert!(mean > 0.0, "frame {k}: the lower top is lit");
        let worst = unlit.iter().map(|&i| acc[i]).fold(0.0f32, f32::max);
        assert!(
            worst < 0.02 * mean,
            "frame {k}: the upper top reads {worst} against the lit top's {mean}"
        );
    }
    println!("{frames} frames with both tops in view");
    assert!(frames >= 12, "{frames} frames");
}

// ---- Phase 5: the costs and the caps ----

/// Phase 5 done-when 1: the entity mirror's buffer is 8 458 240 bytes at the default pool (the
/// tree 4096, the instances 65 536, the pool 8 MiB), and the frame's inventory (`frame_bytes`)
/// holds it: a frame whose mirror is swapped for one with no pool reads exactly the pool less.
#[test]
fn entity_bytes_are_in_the_inventory() {
    let Some((device, _queue)) = support::software_device(64 << 20) else {
        return;
    };
    let mut frame = vxe_render::Frame::with_sizes(&device, [W, H], [W, H]);
    assert_eq!(frame.entity_mirror().bytes(), 8_458_240);
    let with = frame.frame_bytes();
    frame.set_entity_mirror(EntityMirror::new(&device, 0));
    assert_eq!(frame.entity_mirror().bytes(), 4096 + 65_536);
    let without = frame.frame_bytes();
    println!(
        "frame_bytes at {W} x {H}: {with} B with the default mirror, {without} B with no pool"
    );
    assert_eq!(with - without, 8 << 20);
}

// The readings' module, `reading`, kept in its own file for the file-size cap.
include!("gpu_entities/reading.rs");
