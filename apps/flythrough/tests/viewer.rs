//! Plan 0063 Phase 3: the model viewer with no device. The sample walker's files in step, the
//! reload step, the HUD row and the clip keys' words, and `--model`'s startup errors.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use flythrough::viewer::Viewer;
use flythrough::{cave, cave_materials, entities, walker};
use vxe_assets::{Rig, VoxText, encode};
use vxe_entity::{blend, sample};
use vxe_world::MaterialTable;

fn assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities/walker")
}

fn table() -> MaterialTable {
    walker::with_walker(&cave_materials())
}

/// The sample walker opened and built against the fly-through's table.
fn built() -> Viewer {
    let mut v = Viewer::open(&assets().join("walker.rig"), None).expect("walker.rig opens");
    assert_eq!(v.build_pending(&table()), None, "walker.rig builds");
    v
}

/// Done-when 2: `walker.voxtext` converts to the committed `walker.vox`'s bytes; `walker.rig`
/// builds against it with the fly-through's material names; every part with an `at` has that
/// place in the built rest (at its parent's detail, 1/256 world voxel); at tick 0 of `idle`
/// the parts it does not key equal their rest; and a placed instance before any step holds
/// the clip's tick 0 from that rest.
#[test]
fn the_sample_assets_are_in_step() {
    let text = std::fs::read_to_string(assets().join("walker.voxtext")).expect("voxtext reads");
    let vt = VoxText::parse(&text).expect("voxtext parses");
    let committed = std::fs::read(assets().join("walker.vox")).expect("walker.vox reads");
    assert!(
        encode(&vt.to_vox_file()) == committed,
        "walker.vox is not walker.voxtext's: regenerate it with the voxtext example"
    );

    let v = built();
    let l = v.loaded.as_ref().expect("built");
    let rig_text = std::fs::read_to_string(assets().join("walker.rig")).expect("rig reads");
    let rig = Rig::parse(&rig_text).expect("rig parses");
    let names: Vec<String> = rig.parts.iter().map(|p| p.name.clone()).collect();
    let index = |name: &str| l.part_names.iter().position(|n| n == name).expect("named");
    let mut placed_parts = 0;
    for p in &rig.parts {
        let detail = p.parent.as_ref().map_or(p.detail, |n| {
            rig.parts
                .iter()
                .find(|q| &q.name == n)
                .expect("parent")
                .detail
        });
        let want = p.at.map(|c| c / i32::from(detail));
        assert!(
            p.at.iter().all(|c| c % i32::from(detail) == 0),
            "{}",
            p.name
        );
        assert_eq!(
            l.rest.parts[index(&p.name)].t,
            want,
            "{}'s rest place",
            p.name
        );
        placed_parts += usize::from(p.at != [0; 3]);
    }
    assert_eq!(placed_parts, names.len(), "every sample part has an `at`");
    // The named places, 1/256 world voxel: two model voxels to a world voxel.
    assert_eq!(l.rest.parts[index("torso")].t, [0, 0, 1536]);
    assert_eq!(l.rest.parts[index("head")].t, [0, 0, 1536]);
    assert_eq!(l.rest.parts[index("leg_l")].t, [-256, 0, 0]);
    assert_eq!(l.rest.parts[index("arm_r")].t, [640, 0, 1536]);

    let (name, idle) = &l.clips[0];
    assert_eq!(name, "idle");
    let at0 = sample(&l.model, &l.rest, idle, 0);
    let keyed: Vec<usize> = idle.tracks().iter().map(|t| usize::from(t.part)).collect();
    let mut untracked = 0;
    for k in 0..l.model.parts().len() {
        if !keyed.contains(&k) {
            assert_eq!(at0.parts[k], l.rest.parts[k], "part {k} at idle's tick 0");
            untracked += 1;
        }
    }
    assert_eq!(untracked, 3, "idle keys the head and the arms only");

    let mut walkers = entities::Walkers::default().with_viewer(Some(built()));
    assert!(walkers.place(&table(), [10.0, 0.0, 2.0], [0.0, 0.0, 2.0]));
    let p = walkers.placed[0].clone();
    assert_eq!(p.entry, Some(0));
    let (model, pose) = walkers.set.get(p.id).expect("placed");
    assert!(Arc::ptr_eq(model, &l.model) || **model == *l.model);
    assert_eq!(
        pose.parts, at0.parts,
        "placed at idle's tick 0 from the rest"
    );
    assert_eq!(pose.root.0, [2560, 0, 512]);
}

/// The sample rig with line `n` (1-based) replaced by `line`.
fn with_line(text: &str, n: usize, line: &str) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    lines[n - 1] = line;
    lines.join("\n") + "\n"
}

/// Done-when 3, the step: a valid change swaps the model and its clips; line 9 broken keeps
/// the old model and reports `rig line 9`.
#[test]
fn a_changed_rig_reloads_and_a_broken_one_keeps_the_old() {
    let rig = std::fs::read_to_string(assets().join("walker.rig")).expect("rig reads");
    let vox = std::fs::read(assets().join("walker.vox")).expect("vox reads");
    let mut v = built();
    let old = v.loaded.clone().expect("built");
    let narrower = rig.replace("leg_l rot x 45", "leg_l rot x 30");
    assert_ne!(narrower, rig);
    v.reload(&narrower, &vox, &table())
        .expect("a narrower swing builds");
    let new = v.loaded.clone().expect("built");
    assert!(!Arc::ptr_eq(&old.model, &new.model), "the model is swapped");
    assert_ne!(old.clips[1].1, new.clips[1].1, "walk's keys changed");

    assert!(
        rig.lines()
            .nth(8)
            .is_some_and(|l| l.starts_with("material 2"))
    );
    let broken = with_line(&rig, 9, "material 2");
    let e = v
        .reload(&broken, &vox, &table())
        .expect_err("line 9 is broken");
    assert!(e.starts_with("rig line 9: "), "{e}");
    let kept = v.loaded.clone().expect("still built");
    assert!(Arc::ptr_eq(&kept.model, &new.model), "the old model stays");
    assert_eq!(kept.clips[1].1, new.clips[1].1);
}

/// Done-when 3 through the files: a placed instance survives a reload on the new model, the
/// banner reads `reloaded`, and a broken save reports its line and keeps the model.
#[test]
fn a_saved_rig_reloads_the_placed_instances() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("viewer-reload");
    std::fs::create_dir_all(&dir).expect("dir");
    let rig = std::fs::read_to_string(assets().join("walker.rig")).expect("rig reads");
    std::fs::write(dir.join("walker.rig"), &rig).expect("rig written");
    std::fs::copy(assets().join("walker.vox"), dir.join("walker.vox")).expect("vox copied");
    // Each save stamped `secs` after the epoch: distinct from the copy's and each other's.
    let save = |text: &str, secs: u64| {
        std::fs::write(dir.join("walker.rig"), text).expect("saved");
        let f = std::fs::File::options()
            .write(true)
            .open(dir.join("walker.rig"))
            .expect("opens");
        f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .expect("stamped");
    };
    let viewer = Viewer::open(&dir.join("walker.rig"), Some("walk".into())).expect("opens");
    let mut w = entities::Walkers::default().with_viewer(Some(viewer));
    let t = table();
    assert_eq!(w.poll_viewer(0.0, &t), None, "built, nothing changed");
    assert!(w.place(&t, [0.0, 0.0, 2.0], [5.0, 0.0, 2.0]));
    let first = w.set.get(w.placed[0].id).expect("placed").0.clone();

    save(&rig.replace("leg_l rot x 45", "leg_l rot x 30"), 10);
    assert_eq!(w.poll_viewer(0.5, &t), None, "polled at most once a second");
    assert_eq!(w.poll_viewer(1.0, &t).as_deref(), Some("reloaded"));
    let v = w.viewer().expect("viewer");
    assert_eq!(v.clip_name(), Some("walk"), "the playing clip is kept");
    let now = v.loaded.as_ref().expect("built").model.clone();
    assert!(!Arc::ptr_eq(&first, &now));
    assert_eq!(w.placed.len(), 1);
    let (held, _) = w.set.get(w.placed[0].id).expect("placed again");
    assert!(Arc::ptr_eq(held, &now), "the instance holds the new model");

    save(&with_line(&rig, 9, "material 2"), 20);
    let words = w.poll_viewer(2.0, &t).expect("tried");
    assert!(words.starts_with("rig line 9: "), "{words}");
    let v = w.viewer().expect("viewer");
    assert!(Arc::ptr_eq(&v.loaded.as_ref().expect("kept").model, &now));
}

/// Done-when 5: the HUD row names the model, the clip and its tick; a cycle names the clip;
/// the pause shows in the row and holds the tick.
#[test]
fn the_viewer_row_shows_the_clip_and_the_pause() {
    let mut fresh = Viewer::open(&assets().join("walker.rig"), None).expect("opens");
    let mut row = String::new();
    fresh.write_row(&mut row);
    assert_eq!(row, "model walker not built");
    assert_eq!(fresh.build_pending(&table()), None);

    let row = |v: &Viewer| {
        let mut out = String::new();
        v.write_row(&mut out);
        out
    };
    let mut v = built();
    assert_eq!(row(&v), "model walker clip idle 0/60");
    v.advance(0.5);
    assert_eq!(row(&v), "model walker clip idle 15/60");
    assert_eq!(v.cycle(1), "clip walk");
    assert_eq!(row(&v), "model walker clip walk 0/30");
    v.advance(1.25);
    // 37.5 ticks into a looped 30-tick clip.
    assert_eq!(row(&v), "model walker clip walk 7/30");
    assert_eq!(v.toggle_pause(), "paused");
    v.advance(1.0);
    assert_eq!(row(&v), "model walker clip walk 7/30 paused");
    assert_eq!(v.toggle_pause(), "playing");
    assert_eq!(v.cycle(1), "clip turn");
    assert_eq!(v.cycle(1), "clip idle", "the cycle wraps");
    assert_eq!(v.cycle(-1), "clip turn", "and back");
    let (a, b, alpha) = v.poses().expect("built");
    assert_eq!((a.parts.len(), b.parts.len(), alpha), (6, 6, 0.0));
}

/// A clip change fades into the new clip over `CLIP_FADE_TICKS`: right after the change the
/// drawn pose is the one drawn just before it (blended between its two ticks at 1/256), with
/// no fraction left; half way it is the blend of that pose and the new clip's; from the fade's
/// end on it is the new clip's from its tick 0. A change during the fade starts from the
/// blend drawn then.
#[test]
fn a_clip_change_fades_into_the_new_clip() {
    use flythrough::viewer::CLIP_FADE_TICKS;
    assert_eq!(CLIP_FADE_TICKS, 8);
    let mut v = built();
    let l = v.loaded.clone().expect("built");
    let (idle, walk) = (l.clips[0].1.clone(), l.clips[1].1.clone());
    // 15.5 ticks into idle at 30 a second.
    v.advance(0.5 + 0.5 / 30.0);
    let (a, b, alpha) = v.poses().expect("built");
    assert!((alpha - 0.5).abs() < 1e-3, "{alpha}");
    assert_eq!(a, sample(&l.model, &l.rest, &idle, 15));
    let drawn = blend(&a, &b, 128);
    assert_eq!(v.cycle(1), "clip walk");
    let (a, _, alpha) = v.poses().expect("built");
    assert_eq!((a, alpha), (drawn.clone(), 0.0), "no jump at the change");
    v.advance(4.5 / 30.0);
    let (a, _, _) = v.poses().expect("built");
    assert_eq!(a, blend(&drawn, &sample(&l.model, &l.rest, &walk, 4), 128));
    assert_ne!(a, sample(&l.model, &l.rest, &walk, 4));
    v.advance(4.0 / 30.0);
    let (a, b, _) = v.poses().expect("built");
    assert_eq!(a, sample(&l.model, &l.rest, &walk, 8), "the fade is over");
    assert_eq!(b, sample(&l.model, &l.rest, &walk, 9));

    // A change half way through a fade starts from the blend drawn then.
    v.cycle(-1);
    v.advance(4.0 / 30.0);
    let (a, b, alpha) = v.poses().expect("built");
    let shown = blend(&a, &b, (alpha * 256.0).round() as u16);
    v.cycle(1);
    assert_eq!(v.poses().expect("built").0, shown);
}

/// Done-when 5: a missing `--model` file and a `--clip` the rig lacks are errors that name
/// the path.
#[test]
fn a_missing_model_is_an_error_that_names_it() {
    let missing = assets().join("no-such.rig");
    let e = Viewer::open(&missing, None).expect_err("missing");
    assert!(e.contains(&missing.display().to_string()), "{e}");
    let e = Viewer::open(&assets().join("walker.rig"), Some("run".into())).expect_err("no run");
    assert!(
        e.contains("walker.rig") && e.contains("no clip `run`"),
        "{e}"
    );
    assert!(Viewer::open(&assets().join("walker.rig"), Some("walk".into())).is_ok());
}

/// Plan 0078 Phase 2 done-when 1: after an upload, a valid reload moves the handle to `stale`
/// and leaves `handle` `None`; a broken reload leaves both as they were; the sync step uploads
/// the new model, then frees the stale one (the pool holds the new model's bytes alone, and
/// the old handle names nothing).
#[test]
fn a_reload_hands_back_the_old_handle() {
    use flythrough::entities::sync_models;
    use vxe_render::internals::VolumePool;
    use vxe_render::{DEFAULT_ENTITY_POOL_BYTES, EntityMirror, EntityUploadError};

    let rig = std::fs::read_to_string(assets().join("walker.rig")).expect("rig reads");
    let vox = std::fs::read(assets().join("walker.vox")).expect("vox reads");
    let t = table();
    let bytes = |m: &vxe_entity::Model| {
        let mut ids: Vec<u64> = Vec::new();
        let mut sum = 0;
        for p in m.parts() {
            if !ids.contains(&p.volume.id()) {
                ids.push(p.volume.id());
                sum += EntityMirror::volume_bytes(&p.volume);
            }
        }
        sum
    };
    let mut w = entities::Walkers::default().with_viewer(Some(built()));
    assert!(w.place(&t, [0.0, 0.0, 2.0], [5.0, 0.0, 2.0]));
    let mut pool = VolumePool::new(DEFAULT_ENTITY_POOL_BYTES);
    assert_eq!(sync_models(&mut w, &mut pool, &t), None);
    let v = w.viewer_mut().expect("viewer");
    let old = v.handle.expect("uploaded");
    assert_eq!(v.stale, None);

    // A changed palette entry for the eyes' index: every part holding an eye is a new prefab.
    let changed = rig.replace("material 185 walker_eye", "material 185 walker");
    assert_ne!(changed, rig);
    v.reload(&changed, &vox, &t).expect("builds");
    assert_eq!((v.handle, v.stale), (None, Some(old)));
    let broken = with_line(&rig, 9, "material 2");
    v.reload(&broken, &vox, &t).expect_err("line 9 is broken");
    assert_eq!((v.handle, v.stale), (None, Some(old)));
    let live = bytes(&v.loaded.as_ref().expect("built").model);

    assert_eq!(sync_models(&mut w, &mut pool, &t), None);
    let v = w.viewer().expect("viewer");
    let new = v.handle.expect("uploaded");
    assert_ne!(new, old);
    assert_eq!(v.stale, None, "freed after the upload");
    assert_eq!(pool.free_model(old), Err(EntityUploadError::UnknownModel));
    assert_eq!(pool.used_bytes(), live);
}

/// Readings, run by hand (`--run-ignored only -E 'test(=reading::<name>)'`, under the perf
/// lock, `--release` for the figure). They print and assert nothing.
mod reading {
    use std::sync::Arc;

    use vxe_entity::{Clip, Key, Model, Part, Pose, Quat, Track, sample};
    use vxe_render::DisplayPose;
    use vxe_world::{Fixed3, Prefab};

    /// Plan 0078 Phase 2's reading: the CPU time of a reload's pool step for the sample walker,
    /// `free_model` of the old model plus the upload of the new (the bookkeeping and the
    /// volumes' words, no device: the mirror's `write_buffer` copy is not in it); the median of
    /// 1000, in ms. The two models differ in the parts that hold an eye.
    #[test]
    #[ignore]
    fn the_reload_pool_step_cost() {
        use vxe_render::DEFAULT_ENTITY_POOL_BYTES;
        use vxe_render::internals::VolumePool;

        let dir = super::assets();
        let rig = std::fs::read_to_string(dir.join("walker.rig")).expect("rig reads");
        let vox = std::fs::read(dir.join("walker.vox")).expect("vox reads");
        let t = super::table();
        let build = |text: &str| {
            let mut v = super::built();
            v.reload(text, &vox, &t).expect("builds");
            v.loaded.expect("built").model
        };
        let models = [
            build(&rig),
            build(&rig.replace("material 185 walker_eye", "material 185 walker")),
        ];
        let mut pool = VolumePool::new(DEFAULT_ENTITY_POOL_BYTES);
        let mut live = pool.upload(&models[0], &t).expect("room").handle;
        let mut ms = Vec::new();
        for k in 1..=1000usize {
            let t0 = flythrough::now();
            let next = pool.upload(&models[k % 2], &t).expect("room").handle;
            pool.free_model(live).expect("live");
            ms.push(flythrough::now().duration_since(t0).as_secs_f64() * 1e3);
            live = next;
        }
        ms.sort_by(f64::total_cmp);
        println!(
            "reload pool step, median of 1000: {:.4} ms (min {:.4}, max {:.4})",
            ms[500], ms[0], ms[999]
        );
    }

    /// Done-when 6: `sample` at two ticks and `DisplayPose::between` for 64 entities of 16 parts
    /// (a chain, each part on the one before, every part keyed), each entity at its own tick;
    /// the median of 200 frames of all 64, in ms.
    #[test]
    #[ignore]
    fn the_sample_and_compose_cost_at_64_sixteen_part_entities() {
        let parts: Vec<Part> = (0..16)
            .map(|k| Part {
                volume: Arc::new(Prefab::new([4; 3], vec![1; 64], None)),
                parent: (k > 0).then(|| k as u8 - 1),
                pivot: [512, 512, 0],
                detail: 2,
            })
            .collect();
        let model = Model::new(parts).expect("sixteen parts");
        let mut rest = Pose::rest(&model);
        for p in rest.parts.iter_mut().skip(1) {
            p.t = [0, 0, 512];
        }
        let turn = |s: i16| Quat([16000, s, 0, 0]).normalised();
        let tracks = (0..16u8)
            .map(|part| Track {
                part,
                keys: vec![
                    Key {
                        tick: 0,
                        t: [0; 3],
                        q: turn(3000),
                    },
                    Key {
                        tick: 20,
                        t: [0, 0, 64],
                        q: turn(-3000),
                    },
                    Key {
                        tick: 40,
                        t: [0; 3],
                        q: turn(3000),
                    },
                ],
            })
            .collect();
        let clip = Clip::new(&model, 40, true, tracks).expect("the clip");
        // Per frame: all 64 entities' two samples, then all 64 compositions, timed apart.
        let (mut sampling, mut composing) = (Vec::new(), Vec::new());
        let mut kept = 0usize;
        for frame in 0..200u64 {
            let t0 = flythrough::now();
            let poses: Vec<(Pose, Pose)> = (0..64u64)
                .map(|e| {
                    let tick = frame + 7 * e;
                    let mut a = sample(&model, &rest, &clip, tick);
                    let mut b = sample(&model, &rest, &clip, tick + 1);
                    a.root = Fixed3([e as i64 * 4096, 0, 0]);
                    b.root = a.root;
                    (a, b)
                })
                .collect();
            let t1 = flythrough::now();
            for (a, b) in &poses {
                kept += DisplayPose::between(&model, a, b, 0.5).expect("fits").len();
            }
            let t2 = flythrough::now();
            sampling.push(t1.duration_since(t0).as_secs_f64() * 1e3);
            composing.push(t2.duration_since(t1).as_secs_f64() * 1e3);
        }
        for v in [&mut sampling, &mut composing] {
            v.sort_by(f64::total_cmp);
        }
        println!(
            "64 x 16 parts a frame, median of 200: sample x2 {:.4} ms (min {:.4}), \
             DisplayPose::between {:.4} ms (min {:.4}) ({kept} parts)",
            sampling[100], sampling[0], composing[100], composing[0]
        );
    }
}

/// The committed library's walker and spider copied under the target's scratch directory
/// (`tag` names it), and the showcase's table with the walker's rows they build against.
fn library_copy(tag: &str) -> (PathBuf, MaterialTable) {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("library-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    for name in ["walker", "spider"] {
        std::fs::create_dir_all(dir.join(name)).expect("dir");
        for ext in ["rig", "vox"] {
            let f = format!("{name}/{name}.{ext}");
            std::fs::copy(from.join(&f), dir.join(&f)).expect("copied");
        }
    }
    (dir, walker::with_walker(&flythrough::showcase_materials()))
}

/// The index of the library entry `name`.
fn entry(w: &entities::Walkers, name: &str) -> usize {
    w.library
        .entries
        .iter()
        .position(|e| e.name == name)
        .expect("the entry")
}

/// Plan 0077 Phase 2 done-when 1: two instances of the library's walker, one placed on
/// `idle` and one on `walk`, started together: half a second on (15 ticks at 30 a second) each
/// stands in its own clip's pose at tick 15, placed at its feet, exactly, and their legs'
/// poses differ. The walker has a body, so its pose is its mover's, set at the movers' tick
/// with the wander off: on a floor whose top is at the feet it stands where it was placed.
#[test]
fn each_entity_plays_its_own_clip() {
    let lib =
        entities::library::scan(&Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities"));
    let t = table();
    let mut w = entities::Walkers::default().with_library(lib);
    w.poll_viewer(0.0, &t);
    let i = entry(&w, "walker");
    w.library.selected = i;
    let a = w
        .place_entry(&t, Some(i), [0.0, 0.0, 2.0], 0.0)
        .expect("placed");
    let v = w.library.entries[i].viewer_mut().expect("a rig");
    assert_eq!(
        v.cycle(1),
        "clip walk",
        "the entry's clip for the next placement"
    );
    let b = w
        .place_entry(&t, Some(i), [30.0, 0.0, 2.0], 0.0)
        .expect("placed");
    let floor = vxe_world::World::generate(
        vxe_world::ChunkExtent::new([-1, -1, -1], [1, 1, 1]),
        &vxe_world::FnSource(|p: [i32; 3]| if p[2] < 2 { cave::ROCK } else { 0 }),
        &t,
    );
    for _ in 0..30 {
        w.step(1.0 / 60.0);
        w.tick_movers(1.0 / 60.0, &floor);
    }
    let l = w.library.entries[i]
        .viewer()
        .and_then(|v| v.loaded.clone())
        .expect("built");
    let (idle, walk) = (&l.clips[0].1, &l.clips[1].1);
    let want_a =
        flythrough::viewer::placed(&sample(&l.model, &l.rest, idle, 15), [0.0, 0.0, 2.0], 0.0);
    let want_b =
        flythrough::viewer::placed(&sample(&l.model, &l.rest, walk, 15), [30.0, 0.0, 2.0], 0.0);
    let (pa, pb) = (&w.set.get(a).expect("a").1, &w.set.get(b).expect("b").1);
    assert_eq!(**pa, want_a);
    assert_eq!(**pb, want_b);
    let leg = l
        .part_names
        .iter()
        .position(|n| n == "leg_l")
        .expect("leg_l");
    assert_ne!(pa.parts[leg], pb.parts[leg]);
}

/// Plan 0077 Phase 2 done-when 2: with a walker and a spider placed from one library, a saved
/// change to the walker's rig moves the walker onto the new model under the same id, and
/// leaves the spider's id, model and pose as they were. Phase 9 (4): the same with `--model`
/// naming the library's own walker rig, merged into its one entry (two entries, the walker's
/// viewer `--model`'s), whose reload is the one viewer's (said as `--model`'s, `reloaded`).
#[test]
fn a_reload_moves_only_its_own_entry() {
    for merged in [false, true] {
        let (dir, t) = library_copy(if merged { "reload-merged" } else { "reload" });
        let rig_path = dir.join("walker/walker.rig");
        let mut lib = entities::library::scan(&dir);
        if merged {
            lib = lib.with_model(Viewer::open(&rig_path, None).expect("opens"));
            assert_eq!(lib.entries.len(), 2);
        }
        let mut w = entities::Walkers::default().with_library(lib);
        assert_eq!(w.poll_viewer(0.0, &t), None, "both build");
        let (wi, si) = (entry(&w, "walker"), entry(&w, "spider"));
        assert_eq!(w.library.entries[wi].flagged, merged);
        let walker_id = w
            .place_entry(&t, Some(wi), [0.0, 0.0, 2.0], 0.0)
            .expect("walker");
        let spider_id = w
            .place_entry(&t, Some(si), [90.0, 0.0, 2.0], 0.0)
            .expect("spider");
        let old_walker = w.set.get(walker_id).expect("walker").0.clone();
        let (spider_model, spider_pose) = {
            let (m, p) = w.set.get(spider_id).expect("spider");
            (m.clone(), p.clone())
        };
        let text = std::fs::read_to_string(&rig_path).expect("reads");
        std::fs::write(&rig_path, text.replace("leg_l rot x 45", "leg_l rot x 30")).expect("saved");
        std::fs::File::options()
            .write(true)
            .open(&rig_path)
            .expect("opens")
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(10))
            .expect("stamped");
        // `--model`'s viewer says its reload as `--model` always has, unprefixed.
        let said = if merged {
            "reloaded"
        } else {
            "walker: reloaded"
        };
        assert_eq!(w.poll_viewer(1.0, &t).as_deref(), Some(said));
        let (new_walker, _) = w.set.get(walker_id).expect("the same id");
        assert!(
            !Arc::ptr_eq(new_walker, &old_walker),
            "the walker's model is new"
        );
        let (m, p) = w.set.get(spider_id).expect("the spider's id");
        assert!(Arc::ptr_eq(m, &spider_model));
        assert_eq!(*p, spider_pose);
        assert_eq!(w.placed.len(), 2);
    }
}

/// Plan 0077 Phase 2 done-when 3: with the crosshair's ray (the entity set's raycast, as the
/// frame reads it) on the second of two walkers, the clip key changes its clip and not the
/// first's; with the ray on none, the newest's.
#[test]
fn the_clip_keys_follow_the_aim() {
    use vxe_world::Fixed3;
    let lib =
        entities::library::scan(&Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities"));
    let t = table();
    let mut w = entities::Walkers::default().with_library(lib);
    w.poll_viewer(0.0, &t);
    let i = entry(&w, "walker");
    for x in [0.0, 30.0, 60.0] {
        w.place_entry(&t, Some(i), [x, 0.0, 2.0], 0.0)
            .expect("placed");
    }
    // Looking along +y at the second's torso from 20 voxels in front of it.
    let origin = Fixed3([30 * 256, -20 * 256, 10 * 256]);
    w.aimed = w
        .set
        .raycast(origin, [0, 4096, 0], 1 << 20)
        .map(|h| h.entity);
    assert_eq!(w.aimed, Some(w.placed[1].id));
    let clips = |w: &entities::Walkers| w.placed.iter().map(|p| p.play.clip).collect::<Vec<_>>();
    assert_eq!(w.cycle_clip(1), "walker #2 clip walk");
    assert_eq!(clips(&w), [0, 1, 0]);
    w.aimed = w
        .set
        .raycast(origin, [0, -4096, 0], 1 << 20)
        .map(|h| h.entity);
    assert_eq!(w.aimed, None);
    assert_eq!(w.cycle_clip(-1), "walker #3 clip turn");
    assert_eq!(clips(&w), [0, 1, 2]);
}

/// Plan 0077 Phase 2 done-when 4: a pool that holds the walker and not the spider: the spider
/// placed and synced leaves the set and the placements as they were before it, and the
/// banner's words name `spider`. A pool whose free bytes hold the spider in two holes, none
/// large enough, is reset first (`entity pool reset`) and then holds both.
#[test]
fn a_full_pool_refuses_one_entry_and_keeps_the_rest() {
    use flythrough::entities::library::pool_bytes;
    use flythrough::entities::sync_models;
    use vxe_render::internals::VolumePool;
    use vxe_world::Prefab;

    let (dir, t) = library_copy("pool");
    let fresh = || {
        let mut w = entities::Walkers::default().with_library(entities::library::scan(&dir));
        w.poll_viewer(0.0, &t);
        w
    };
    let mut w = fresh();
    let (wi, si) = (entry(&w, "walker"), entry(&w, "spider"));
    let bytes = |w: &entities::Walkers, i: usize| {
        pool_bytes(
            &w.library.entries[i]
                .viewer()
                .unwrap()
                .loaded
                .as_ref()
                .unwrap()
                .model,
        )
    };
    let (wb, sb) = (bytes(&w, wi), bytes(&w, si));
    assert!(wb < sb, "{wb} {sb}");

    let mut pool = VolumePool::new(wb + sb / 2);
    w.place_entry(&t, Some(wi), [0.0, 0.0, 2.0], 0.0)
        .expect("walker");
    w.newest.push(true);
    assert_eq!(sync_models(&mut w, &mut pool, &t), None);
    let before = (w.placed.clone(), w.set.len(), w.newest.clone());
    w.place_entry(&t, Some(si), [90.0, 0.0, 2.0], 0.0)
        .expect("spider placed");
    w.newest.push(true);
    let words = sync_models(&mut w, &mut pool, &t).expect("the refusal's words");
    assert!(words.starts_with("spider: "), "{words}");
    assert_eq!((w.placed.clone(), w.set.len(), w.newest.clone()), before);
    assert_eq!(pool.used_bytes(), wb);

    // A filler a little over half the spider's bytes ahead of the walker, freed: the free bytes
    // hold the spider, in two holes that each are smaller.
    let mut w = fresh();
    let red = vxe_world::MaterialId::try_from(1usize).unwrap();
    let mut n = 1;
    let filler = loop {
        let p = Prefab::new([n; 3], vec![red; (n * n * n) as usize], None);
        if 2 * vxe_render::EntityMirror::volume_bytes(&p) > sb {
            break p;
        }
        n += 1;
    };
    let fb = vxe_render::EntityMirror::volume_bytes(&filler);
    let filler = vxe_entity::Model::new(vec![vxe_entity::Part {
        volume: Arc::new(filler),
        parent: None,
        pivot: [0; 3],
        detail: 1,
    }])
    .expect("one part");
    let tail = sb - fb / 2;
    let mut pool = VolumePool::new(fb + wb + tail);
    let h = pool.upload(&filler, &t).expect("room").handle;
    w.place_entry(&t, Some(wi), [0.0, 0.0, 2.0], 0.0)
        .expect("walker");
    assert_eq!(sync_models(&mut w, &mut pool, &t), None);
    pool.free_model(h).expect("freed");
    w.place_entry(&t, Some(si), [90.0, 0.0, 2.0], 0.0)
        .expect("spider");
    assert_eq!(
        sync_models(&mut w, &mut pool, &t).as_deref(),
        Some(entities::POOL_RESET)
    );
    assert_eq!(w.placed.len(), 2);
    assert_eq!(pool.used_bytes(), wb + sb);
}
