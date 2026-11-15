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
