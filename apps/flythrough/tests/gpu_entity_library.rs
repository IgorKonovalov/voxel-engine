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
use vxe_render::glam::Vec3;
use vxe_render::{Camera, ViewMode};
use vxe_world::{ChunkExtent, EditOp, FIXED_ONE, Fixed3, Mode, UniformSource, World};

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
    let rec = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());

    let origin = Fixed3(eye.map(|v| (v * 256.0).round() as i64));
    let cpu: Vec<Option<(vxe_entity::EntityId, u8)>> = (0..rec.len())
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
