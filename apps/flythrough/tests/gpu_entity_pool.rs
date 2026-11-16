//! Plan 0078 on the software adapter (ADR-0060): the entity pool freed and refilled under the
//! frame. Plan 0062's closed rock box, its table the cave's plus the walker's rows and a red
//! row, under a 192 x 108 frame with no jitter.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use flythrough::cave::ROCK;
use flythrough::viewer::{Loaded, Viewer};
use flythrough::{cave_materials, walker};
use support::bake::{Rig, rig_on};
use vxe_assets::{VoxText, encode};
use vxe_entity::{Model, Part, Pose};
use vxe_render::glam::Vec3;
use vxe_render::{Camera, EntityMirror, ViewMode};
use vxe_world::{
    ChunkExtent, EditOp, FIXED_ONE, Fixed3, Material, MaterialId, MaterialTable, Mode, Prefab,
    UniformSource, World,
};

const W: u32 = 192;
const H: u32 = 108;

/// The red row's albedo.
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
/// [2, 50)`.
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

/// Looking toward +x from `x`, level, at height `z`.
fn level_camera(x: f32, z: f32) -> Camera {
    Camera {
        position: [x, 32.0, z].into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 1.2,
    }
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

/// The bytes `model`'s distinct volumes take in the pool.
fn model_bytes(model: &Model) -> u64 {
    let mut ids: Vec<u64> = Vec::new();
    let mut sum = 0;
    for p in model.parts() {
        if !ids.contains(&p.volume.id()) {
            ids.push(p.volume.id());
            sum += EntityMirror::volume_bytes(&p.volume);
        }
    }
    sum
}

fn assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities/walker")
}

/// The sample walker's `.vox` with its torso `extra` layers taller: each extra layer a copy of
/// the torso's first.
fn walker_vox(extra: usize) -> Vec<u8> {
    let text = std::fs::read_to_string(assets().join("walker.voxtext")).expect("voxtext reads");
    let at = text.find("model torso\n").expect("a torso") + "model torso\n".len();
    let layer: String = text[at..]
        .lines()
        .take(7)
        .map(|l| format!("{l}\n"))
        .collect();
    assert!(layer.starts_with("layer\n"), "{layer}");
    let mut grown = text[..at].to_owned();
    for _ in 0..extra {
        grown.push_str(&layer);
    }
    grown.push_str(&text[at..]);
    encode(&VoxText::parse(&grown).expect("parses").to_vox_file())
}

/// Plan 0078 Phase 2 done-when 3: twenty saves of the sample walker's `.vox`, its torso
/// alternating 12 and 13 layers tall, each polled, synced and drawn: after every frame the
/// pool holds exactly the live model's bytes (its distinct volumes' `volume_bytes`), and on
/// the twentieth frame the centre pixel of the albedo view reads the torso's albedo.
#[test]
fn twenty_reloads_hold_the_pool_at_one_model() {
    let Some((mut rig, _)) = rig() else {
        return;
    };
    albedo_view(&mut rig);
    let t = rig.world.materials().clone();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("entity-pool-reload");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::copy(assets().join("walker.rig"), dir.join("walker.rig")).expect("rig copied");
    let save = |extra: usize, secs: u64| {
        let path = dir.join("walker.vox");
        std::fs::write(&path, walker_vox(extra)).expect("saved");
        let f = std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("opens");
        f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .expect("stamped");
    };
    save(0, 1);
    let viewer = Viewer::open(&dir.join("walker.rig"), None).expect("opens");
    let mut w = walker::Walkers::default().with_viewer(Some(viewer));
    let camera = level_camera(86.0, 11.0);
    // Feet on the floor 20 voxels ahead, facing the camera.
    assert!(w.place(&t, [106.0, 32.0, 2.0], [86.0, 32.0, 11.0]));
    let eye = camera.position.to_array();
    let mut sizes = std::collections::BTreeSet::new();
    for k in 0..=20u64 {
        if k > 0 {
            save((k % 2) as usize, 10 + k);
            let words = w.poll_viewer(k as f64 * 2.0, &t);
            assert_eq!(words.as_deref(), Some("reloaded"), "save {k}");
        } else {
            assert_eq!(w.poll_viewer(0.0, &t), None);
        }
        let words = walker::feed(&mut w, &rig.world, eye, &mut rig.frame, &rig.queue);
        assert_eq!(words, None, "save {k}");
        rig.step(&camera);
        let live = model_bytes(
            &w.viewer
                .as_ref()
                .expect("viewer")
                .loaded
                .as_ref()
                .expect("built")
                .model,
        );
        sizes.insert(live);
        let used = rig.frame.entity_mirror().pool_used_bytes();
        println!("save {k}: pool {used} B, the live model {live} B");
        assert_eq!(used, live, "save {k}");
        assert_eq!(rig.frame.entity_mirror().entities_drawn(), 1, "save {k}");
    }
    assert_eq!(sizes.len(), 2, "two torso sizes: {sizes:?}");
    let img = rig.display();
    let body = t
        .rows()
        .iter()
        .find(|m| m.name == walker::BODY_ROW)
        .expect("the body row")
        .albedo;
    let centre = pixel_of(&camera, [106.0, 32.0, 11.0]);
    near("the torso", rgb(&img, centre), unorm(body));
}

/// Plan 0078 Phase 2 done-when 4: a pool of 13 984 B (the walker twice and a 16-voxel cube
/// once) holding a walker-sized filler at word 0, the viewer's one-voxel model after it and
/// the built-in walker after that; the filler freed. The viewer's reload to a 16-voxel cube
/// is refused, freeing the old model leaves a 5224 B hole under the 5672 B the cube needs
/// with 9828 B free, so `feed` resets: the banner reads `entity pool reset`, the cube is
/// uploaded, and both placed walkers are drawn on the next frame with none over.
#[test]
fn a_fragmented_pool_resets_and_the_walkers_stay() {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    let t = rig.world.materials().clone();
    let (body, eye) = walker::rows(&t).expect("the walker's rows");
    let mut mirror = EntityMirror::new(&rig.device, 13_984);
    // The walker with its rows swapped: the built-in walker's shape, none of its prefabs.
    let filler = mirror
        .upload_model(&rig.queue, &walker::model(eye, body), &t)
        .expect("room");
    let old = mirror
        .upload_model(&rig.queue, &block([1; 3], red, 1), &t)
        .expect("room");
    rig.frame.set_entity_mirror(mirror);

    let camera = level_camera(86.0, 11.0);
    let eye_at = camera.position.to_array();
    let mut w = walker::Walkers::default();
    assert!(w.place(&t, [106.0, 26.0, 2.0], eye_at));
    assert!(w.place(&t, [106.0, 38.0, 2.0], eye_at));
    assert_eq!(
        walker::feed(&mut w, &rig.world, eye_at, &mut rig.frame, &rig.queue),
        None
    );
    rig.step(&camera);
    assert_eq!((w.drawn, w.over), (2, 0));
    assert_eq!(
        rig.frame
            .entity_mirror()
            .pool()
            .part_words(w.handle.expect("uploaded")),
        Some(vec![1306, 1733, 2059, 2059]),
        "the walker after the filler (1039 words) and the one-voxel model (267)"
    );
    rig.frame
        .entity_mirror_mut()
        .free_model(filler)
        .expect("live");

    let mut viewer = Viewer::open(&assets().join("walker.rig"), None).expect("opens");
    assert_eq!(viewer.build_pending(&t), None);
    let built = viewer.loaded.clone().expect("built");
    let cube = block([16; 3], red, 1);
    viewer.loaded = Some(Loaded {
        model: cube.clone(),
        part_names: vec!["cube".into()],
        rest: Pose::rest(&cube),
        clips: Vec::new(),
        rate: built.rate,
    });
    (viewer.handle, viewer.stale) = (None, Some(old));
    w.viewer = Some(viewer);
    let words = walker::feed(&mut w, &rig.world, eye_at, &mut rig.frame, &rig.queue);
    assert_eq!(words.as_deref(), Some(walker::POOL_RESET));
    let v = w.viewer.as_ref().expect("viewer");
    assert!(v.handle.is_some(), "the cube is uploaded");
    assert_eq!(v.stale, None);
    let m = rig.frame.entity_mirror();
    assert_eq!(m.pool_used_bytes(), 4156 + 5672);
    rig.step(&camera);
    let m = rig.frame.entity_mirror();
    assert_eq!((m.entities_drawn(), m.entities_over_cap()), (2, 0));
    assert_eq!((w.drawn, w.over), (2, 0));
}

// ---- Phase 3: stable instance slots, generations and ghost leaves ----

use vxe_entity::{EntityId, EntitySet};
use vxe_render::{Light, ModelHandle};

/// `model` posed with its root at `at` voxels, unturned.
fn posed(model: &Model, at: [f32; 3]) -> Pose {
    let mut pose = Pose::rest(model);
    pose.root = Fixed3(at.map(|v| (v * 256.0).round() as i64));
    pose
}

/// Uploads `model` to the rig's mirror and places it at `pose` in `set`.
fn place(
    rig: &mut Rig,
    set: &mut EntitySet,
    model: &Arc<Model>,
    pose: Pose,
) -> (EntityId, ModelHandle) {
    let h = rig
        .frame
        .entity_mirror_mut()
        .upload_model(&rig.queue, model, rig.world.materials())
        .expect("the pool has room");
    (set.insert(model.clone(), pose).expect("fits"), h)
}

/// A buffer the target's rgba8 rows are copied into, row pitch 256-byte aligned.
fn readback_buffer(rig: &Rig) -> vxe_render::wgpu::Buffer {
    rig.device
        .create_buffer(&vxe_render::wgpu::BufferDescriptor {
            label: Some("target readback"),
            size: u64::from(W * 4 * H),
            usage: vxe_render::wgpu::BufferUsages::COPY_DST
                | vxe_render::wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
}

/// Submits a copy of the frame's target into `buf`, without waiting.
fn copy_target(rig: &Rig, buf: &vxe_render::wgpu::Buffer) {
    use vxe_render::wgpu;
    let mut enc = rig
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        rig.frame.target().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                // 192 * 4 = 768 bytes, a multiple of 256.
                bytes_per_row: Some(W * 4),
                rows_per_image: Some(H),
            },
        },
        rig.frame.target().size(),
    );
    rig.queue.submit([enc.finish()]);
}

/// Maps `buf` and returns its bytes, waiting for the device.
fn read_buffer(rig: &Rig, buf: &vxe_render::wgpu::Buffer) -> Vec<u8> {
    let slice = buf.slice(..);
    slice.map_async(vxe_render::wgpu::MapMode::Read, |r| r.expect("maps"));
    rig.device
        .poll(vxe_render::wgpu::PollType::wait_indefinitely())
        .expect("the device finishes");
    let bytes = slice.get_mapped_range().expect("mapped").to_vec();
    buf.unmap();
    bytes
}

/// Plan 0078 Phase 3 done-when 6: a red 8-voxel cube 20 voxels before the camera on the box,
/// frame 1 submitted and its target's copy submitted; with no wait, the model freed and a
/// green cube of the same size uploaded into the same pool word and frame 2 submitted: frame
/// 1's centre pixel reads red and frame 2's green, each within 1/255 a channel. The queue
/// orders the green cube's write after frame 1 and its copy.
#[test]
fn a_reused_pool_slot_shows_the_new_model_and_the_old_frame_the_old() {
    let (mut t, red) = table();
    let green_rgb = [0.2, 0.8, 0.2];
    let green = t.push(Material {
        albedo: green_rgb,
        ..Material::named("green")
    });
    let Some(mut rig) = rig_on(entity_box(&t), [W, H], None) else {
        return;
    };
    rig.frame.set_lighting(support::lights::box_lighting());
    albedo_view(&mut rig);
    let camera = level_camera(86.0, 26.0);
    let (rcube, gcube) = (block([8; 3], red, 1), block([8; 3], green, 1));
    let mut set = EntitySet::new();
    let (id, h) = place(
        &mut rig,
        &mut set,
        &rcube,
        posed(&rcube, [110.0, 32.0, 26.0]),
    );
    let word = rig
        .frame
        .entity_mirror()
        .pool()
        .part_words(h)
        .expect("live");
    rig.frame.set_entities(&rig.queue, &set, &[(id, h)]);
    rig.step(&camera);
    rig.step(&camera);
    let (one, two) = (readback_buffer(&rig), readback_buffer(&rig));
    copy_target(&rig, &one);
    rig.frame.entity_mirror_mut().free_model(h).expect("live");
    let g = rig
        .frame
        .entity_mirror_mut()
        .upload_model(&rig.queue, &gcube, &t)
        .expect("room");
    assert_eq!(
        rig.frame.entity_mirror().pool().part_words(g),
        Some(word.clone()),
        "the same pool word"
    );
    set.replace(id, gcube.clone(), posed(&gcube, [110.0, 32.0, 26.0]))
        .expect("fits");
    rig.frame.set_entities(&rig.queue, &set, &[(id, g)]);
    rig.step(&camera);
    copy_target(&rig, &two);
    let centre = pixel_of(&camera, [106.0, 32.0, 26.0]);
    near("frame 1", rgb(&read_buffer(&rig, &one), centre), unorm(RED));
    near(
        "frame 2",
        rgb(&read_buffer(&rig, &two), centre),
        unorm(green_rgb),
    );
}

/// The hit records' instance field (flags bits 6..15) at pixel `i`, when it is an entity hit.
fn instance_field(rig: &Rig, i: usize) -> Option<u16> {
    let rec = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let f = rec[i][3];
    (f & 1 != 0 && f & 32 != 0).then_some(f >> 6)
}

/// The temporal pass's accumulated direct part's luminance at pixel `i`.
fn direct_acc_at(rig: &Rig, i: usize) -> f32 {
    vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.direct_accumulated())[i][3]
}

/// A 16 x 16 x 2 slab of `m`, its pivot at its top centre, detail 1.
fn slab_part(m: MaterialId) -> Part {
    Part {
        volume: Arc::new(Prefab::new([16, 16, 2], vec![m; 512], None)),
        parent: None,
        pivot: [8 * 256, 8 * 256, 2 * 256],
        detail: 1,
    }
}

/// The camera whose pixel (96, 54) looks at `p` from 28 voxels, pitched down 0.7.
fn looking_at(p: [f32; 3]) -> (Camera, usize) {
    let mut camera = Camera {
        position: [0.0; 3].into(),
        yaw: 0.0,
        pitch: -0.7,
        vfov: 1.2,
    };
    let px = 54 * W as usize + 96;
    let (f, r, u) = camera.basis();
    let t = (camera.vfov * 0.5).tan();
    let a = W as f32 / H as f32;
    let nx = (96.5 / W as f32) * 2.0 - 1.0;
    let ny = (54.5 / H as f32) * 2.0 - 1.0;
    let d = (f + r * (nx * t * a) - u * (ny * t)).normalize();
    camera.position = Vec3::from(p) - d * 28.0;
    (camera, px)
}

/// Plan 0078 Phase 3 done-when 7: on the lit box (a light of 1200 at height 24), entity A, a
/// slab whose top face (height 4) reads 9.40, is removed and entity B placed the same frame:
/// B's first part is the same slab of the same material in the same place, taking A's slot,
/// and its second a slab at heights 14 to 16 that shadows the first's top. On that frame the
/// pixel looking at the top's centre reads under 2 % of 9.40 in the accumulated direct part,
/// and its hit record's instance field holds the same slot as the frame before's under the
/// other generation bit.
#[test]
fn history_does_not_cross_a_reused_instance_slot() {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    rig.frame
        .set_lights(&[Light::point([64.0, 32.0, 24.0], 1200.0)]);
    let top = [64.0, 32.0, 4.0];
    let (camera, px) = looking_at(top);
    let a = Arc::new(Model::new(vec![slab_part(red)]).expect("one part"));
    let b = Arc::new(Model::new(vec![slab_part(red), slab_part(red)]).expect("two parts"));
    let mut set = EntitySet::new();
    let (ia, ha) = place(&mut rig, &mut set, &a, posed(&a, top));
    for _ in 0..8 {
        rig.frame.set_entities(&rig.queue, &set, &[(ia, ha)]);
        rig.step(&camera);
    }
    let lit = direct_acc_at(&rig, px);
    println!("A's top: {lit}");
    assert!((lit / 9.40 - 1.0).abs() <= 0.02, "A's top reads {lit}");
    let before = instance_field(&rig, px).expect("A at the pixel");
    set.remove(ia);
    let mut pose = posed(&b, top);
    pose.parts[1].t = [0, 0, 12 * 256];
    let (ib, hb) = place(&mut rig, &mut set, &b, pose);
    rig.frame.set_entities(&rig.queue, &set, &[(ib, hb)]);
    rig.step(&camera);
    let dark = direct_acc_at(&rig, px);
    let after = instance_field(&rig, px).expect("B at the pixel");
    println!("B's top: {dark}; the instance field {before:#x} then {after:#x}");
    assert!(dark < 0.02 * 9.40, "B's top reads {dark}");
    assert_eq!(before & 511, after & 511, "the same slot");
    assert_ne!(before >> 9, after >> 9, "the other generation bit");
    assert_eq!(
        rig.frame.entity_mirror().instances().run_of(ib),
        Some((0, 2))
    );
}

/// Plan 0078 Phase 3 done-when 8: Plan 0062 Phase 3's shadow rig (a light of 1200 at height
/// 22, a 16 x 16 x 2 slab at heights 9 to 11 under it, the floor under it reading 0). On the
/// first frame after the slab is removed, the floor pixel under it reads its term within 2 %
/// in the accumulated direct part (the ghost leaf marked it fresh); the write after holds no
/// ghost.
#[test]
fn a_removed_entity_takes_its_shadow_with_it() {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    let light = Light::point([64.0, 32.0, 22.0], 1200.0);
    rig.frame.set_lights(&[light]);
    let camera = Camera {
        position: [64.0, 6.0, 8.0].into(),
        yaw: std::f32::consts::FRAC_PI_2,
        pitch: -0.25,
        vfov: 1.2,
    };
    let s = Arc::new(Model::new(vec![slab_part(red)]).expect("one part"));
    let mut set = EntitySet::new();
    let (id, h) = place(&mut rig, &mut set, &s, posed(&s, [64.0, 32.0, 11.0]));
    for _ in 0..8 {
        rig.frame.set_entities(&rig.queue, &set, &[(id, h)]);
        rig.step(&camera);
    }
    let pos = vxe_render::debug::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions());
    let under = (0..pos.len())
        .filter(|&i| (pos[i][2] - 2.0).abs() < 1e-3)
        .min_by(|&i, &j| {
            let d = |k: usize| (pos[k][0] - 64.0).powi(2) + (pos[k][1] - 32.0).powi(2);
            d(i).total_cmp(&d(j))
        })
        .expect("the floor shows under the slab");
    let p = [pos[under][0], pos[under][1], pos[under][2]];
    assert!(
        (p[0] - 64.0).abs() < 1.0 && (p[1] - 32.0).abs() < 1.0,
        "{p:?}"
    );
    let shadowed = direct_acc_at(&rig, under);
    println!("under the slab at {p:?}: {shadowed}");
    assert_eq!(shadowed, 0.0);
    rig.frame.set_entities(&rig.queue, &set, &[]);
    rig.step(&camera);
    let m = rig.frame.entity_mirror();
    assert_eq!(m.parts_drawn(), 0);
    assert_eq!(m.instances().leaves().count(), 1, "the slab's ghost");
    let lit = direct_acc_at(&rig, under);
    let l: [f32; 3] = std::array::from_fn(|k| light.pos[k] - p[k]);
    let d2: f32 = l.iter().map(|v| v * v).sum();
    let r2 = light.intensity / rig.frame.lighting().light_floor;
    let window = (1.0 - (d2 / r2).powi(2)).max(0.0).powi(2);
    let want = std::f32::consts::PI * light.intensity * (l[2] / d2.sqrt()) / d2 * window;
    println!("the first frame without it: {lit} against {want}");
    assert!((lit / want - 1.0).abs() <= 0.02, "{lit} against {want}");
    rig.frame.set_entities(&rig.queue, &set, &[]);
    rig.step(&camera);
    assert_eq!(rig.frame.entity_mirror().instances().leaves().count(), 0);
}

/// The display's pixels where the two images differ by more than `tol` in a channel.
fn differing(a: &[u8], b: &[u8], tol: u8) -> Vec<usize> {
    (0..a.len() / 4)
        .filter(|&i| (0..3).any(|k| a[4 * i + k].abs_diff(b[4 * i + k]) > tol))
        .collect()
}

/// Plan 0078 Phase 3 done-when 9: a still walker on the lit box, no jitter, settled. `replace`
/// with a model of the same voxels (new prefabs of equal content share the volumes; a new
/// handle): every pixel of the next frame equals the frame before within 1/255. `replace`
/// with the torso's material changed: pixels of the torso (slot 0) change and none of the
/// legs' (slots 2 and 3) do.
#[test]
fn a_reload_keeps_the_picture_steady() {
    let Some((mut rig, red)) = rig() else {
        return;
    };
    let t = rig.world.materials().clone();
    let (body, eye) = walker::rows(&t).expect("the walker's rows");
    rig.frame
        .set_lights(&[Light::point([100.0, 32.0, 30.0], 1200.0)]);
    let walker_model = Arc::new(walker::model(body, eye));
    let camera = level_camera(86.0, 9.0);
    let feet = [106.0, 32.0, 2.0];
    let pose = walker::pose(feet, walker::facing(feet, camera.position.to_array()), 0.0);
    let mut set = EntitySet::new();
    let (id, h) = place(&mut rig, &mut set, &walker_model, pose.clone());
    for _ in 0..24 {
        rig.frame.set_entities(&rig.queue, &set, &[(id, h)]);
        rig.step(&camera);
    }
    let settled = rig.display();
    rig.frame.set_entities(&rig.queue, &set, &[(id, h)]);
    rig.step(&camera);
    let n = rig.display();
    let still = differing(&settled, &n, 1).len();
    println!("frame to frame with nothing replaced: {still} pixels differ by more than 1/255");

    let same = Arc::new(walker::model(body, eye));
    let h2 = rig
        .frame
        .entity_mirror_mut()
        .upload_model(&rig.queue, &same, &t)
        .expect("room");
    assert_ne!(h2, h);
    set.replace(id, same.clone(), pose.clone()).expect("fits");
    rig.frame.set_entities(&rig.queue, &set, &[(id, h2)]);
    rig.step(&camera);
    let n1 = rig.display();
    let moved = differing(&n, &n1, 1);
    println!(
        "after a replace of the same voxels: {} pixels differ",
        moved.len()
    );
    assert!(moved.is_empty(), "{} pixels differ", moved.len());

    let mut parts = same.parts().to_vec();
    parts[0].volume = Arc::new(Prefab::new(
        walker::TORSO,
        vec![red; (walker::TORSO[0] * walker::TORSO[1] * walker::TORSO[2]) as usize],
        None,
    ));
    let red_torso = Arc::new(Model::new(parts).expect("the walker's parts"));
    let h3 = rig
        .frame
        .entity_mirror_mut()
        .upload_model(&rig.queue, &red_torso, &t)
        .expect("room");
    set.replace(id, red_torso, pose).expect("fits");
    rig.frame.set_entities(&rig.queue, &set, &[(id, h3)]);
    rig.step(&camera);
    let n2 = rig.display();
    let rec = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let slot = |i: usize| {
        let f = rec[i][3];
        (f & 1 != 0 && f & 32 != 0).then_some((f >> 6) & 511)
    };
    let changed = differing(&n1, &n2, 1);
    let torso = changed.iter().filter(|&&i| slot(i) == Some(0)).count();
    let legs: Vec<_> = changed
        .iter()
        .filter(|&&i| matches!(slot(i), Some(2 | 3)))
        .collect();
    println!(
        "the red torso: {} pixels differ, {torso} of them the torso's, {} the legs'",
        changed.len(),
        legs.len()
    );
    for &&i in &legs {
        let (x, y) = ((i % W as usize) as i64, (i / W as usize) as i64);
        let mut near_torso = i64::MAX;
        for j in 0..(W * H) as usize {
            if slot(j) == Some(0) {
                let (u, v) = ((j % W as usize) as i64, (j / W as usize) as i64);
                near_torso = near_torso.min((u - x).abs().max((v - y).abs()));
            }
        }
        println!(
            "leg pixel ({x}, {y}): {:?} then {:?}, the nearest torso pixel {near_torso} away",
            &n1[4 * i..4 * i + 3],
            &n2[4 * i..4 * i + 3]
        );
    }
    assert!(torso > 0, "the torso's pixels change");
    assert!(legs.is_empty(), "{} leg pixels change", legs.len());
}
