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
