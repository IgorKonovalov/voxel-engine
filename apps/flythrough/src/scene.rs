//! The scenes' fixed parts: the rigs' poses and cameras, the material tables, and the
//! residency and mirror budgets the streamed and bounded worlds run under.

use crate::castle;
use crate::cave;
use crate::census::{around_guards, brick_counts};
use crate::logic::RUBBLE;
use crate::showcase;
use crate::thin_rig;
use vxe_audio::Listener;
use vxe_render::Camera;
use vxe_world::{Chunk, Material, MaterialFlags, MaterialId, MaterialTable, Residency};

/// A still camera on the bounded level (seed 1), with the manual exposure the stillness and
/// settle rigs pin it at. `--level --pose NAME` starts the fly-through there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub name: &'static str,
    pub position: [f32; 3],
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub exposure: f32,
}

/// Facing the wall where the lamp-off patches were first counted. The exposure is the eye's
/// adapted value after 600 frames with the lamp on and 900 off at 192 x 108.
pub const PATCH_POSE: Pose = Pose {
    name: "patch",
    position: [676.2, 514.8, 193.1],
    yaw_deg: 59.0,
    pitch_deg: -9.7,
    exposure: 10.9624,
};

/// Out from an ore vein along its normal, looking at it, at its eye's adapted value, read the
/// same way.
pub const VEIN_POSE: Pose = Pose {
    name: "vein",
    position: [727.5, 644.3, 208.5],
    yaw_deg: 153.4,
    pitch_deg: 0.0,
    exposure: VEIN_EXPOSURE,
};

/// `VEIN_POSE`'s adapted exposure.
pub const VEIN_EXPOSURE: f32 = 5.3720;

/// Far from the spawn, in chunk [18, 15, 3], where the owner judged the brick level at the
/// position the HUD read (1191.5 1013.9 240.4); the heading is the flight's from `VEIN_POSE`, out
/// along the cave, a little up toward the wall and its ceiling lamp. Not a rig's pose.
pub const FAR_POSE: Pose = Pose {
    name: "far",
    position: [1191.5, 1013.9, 240.4],
    yaw_deg: 38.6,
    pitch_deg: 10.0,
    exposure: VEIN_EXPOSURE,
};

/// The poses the rigs read, by name.
pub const POSES: [Pose; 2] = [VEIN_POSE, PATCH_POSE];

/// The poses `--pose` takes: the rigs' and [`FAR_POSE`].
/// The start of the settle rig's flight to `VEIN_POSE` (`VEIN_FLIGHT` in its support), facing
/// along it: 180 voxels to the vein pose, which `--glide 30` flies in 360 frames. Not a rig's pose.
pub const VEIN_FLIGHT_POSE: Pose = Pose {
    name: "vein-flight",
    position: [882.7914, 733.9575, 192.81197],
    yaw_deg: 210.0,
    pitch_deg: 5.0,
    exposure: VEIN_EXPOSURE,
};

pub const START_POSES: [Pose; 4] = [VEIN_POSE, PATCH_POSE, FAR_POSE, VEIN_FLIGHT_POSE];

impl Pose {
    /// The pose `--pose` names `name`, if any.
    pub fn named(name: &str) -> Option<Pose> {
        START_POSES.into_iter().find(|p| p.name == name)
    }

    pub fn camera(&self) -> Camera {
        Camera {
            position: self.position.into(),
            yaw: self.yaw_deg.to_radians(),
            pitch: self.pitch_deg.to_radians(),
            vfov: 70f32.to_radians(),
        }
    }
}

/// The fly-through's residency defaults (Plan 0002): full detail to 16 chunks (102 m), mips
/// to 24 (154 m), one chunk of hysteresis.
pub const RESIDENCY: Residency = Residency {
    r_full: 16,
    r_lod: 24,
    hysteresis: 1,
};

/// The side of the mirror window that holds a residency: `2 (r_lod + h) + 1` chunks.
pub fn window_side(r: Residency) -> i32 {
    2 * (r.r_lod.max(r.r_full) + r.hysteresis) + 1
}

/// Mirror pressure under which the fly-through's full radius returns a chunk (ADR-0013). The
/// streamed mirror's records are sized so their occupancy stays under it.
pub const PRESSURE_LOW: f32 = 0.7;

/// Records the streamed mirror reserves for residency `r`: the chunks one focus can keep,
/// `chunks_in_ball(max(r_lod, r_full) + h)`, over [`PRESSURE_LOW`] (as `* 10 / 7`, rounded
/// up), so the record term of the pressure stays under the restore line while the streamer
/// keeps its radius. 93 239 at the defaults.
pub fn stream_record_chunks(r: Residency) -> u64 {
    let ball = vxe_world::chunks_in_ball(r.r_lod.max(r.r_full) + r.hysteresis);
    (ball * 10).div_ceil(7)
}

/// The mirror budget of the streamed cave unless `--mirror-budget` sets one: the world line
/// of `nfr.md` §3, 3 GiB, of which the residency's records take their share.
pub const STREAM_MIRROR_BUDGET: u64 = vxe_render::DEFAULT_BUDGET_BYTES;

/// `Full` bricks of headroom beyond the bounded level's own, so digging has room: 64 Ki
/// bricks, each at the largest slot the level's rock takes (6 units), and with the field a
/// band slot of the most units ([`vxe_render::internals::MAX_BAND_UNITS`]) beside each.
pub const POOL_HEADROOM_BRICKS: u64 = 65_536;

/// The pool units the bounded level `world` needs: each bin's `Full` bricks in whole pages
/// of that bin, and when `field` each band brick's band slot and each uniform band brick's
/// slot in pages of theirs (the band as [`band_census`] reads it), plus the headroom.
pub fn level_pool_units(world: &vxe_world::World, field: bool) -> u64 {
    use vxe_render::{
        internals::BIN_COUNT, internals::BINS, internals::PAGE_UNITS, internals::band_bin_of,
        internals::bin_of, internals::slot_units, internals::slots_per_page,
    };
    use vxe_render::{internals::MAX_BAND_UNITS, internals::band_slot_units};
    let table = world.materials();
    let translucent = |m| {
        table
            .flags(m)
            .contains(vxe_world::MaterialFlags::TRANSLUCENT)
    };
    let mut bins = [0u64; BIN_COUNT];
    for c in world.store().coords() {
        let arcs = if field {
            around_guards(world, c)
        } else {
            let mut v = vec![None; 27];
            v[13] = world.store().get(c);
            v
        };
        let guards: Vec<_> = arcs
            .iter()
            .map(|a| {
                a.as_ref()
                    .map(|a| a.read().unwrap_or_else(std::sync::PoisonError::into_inner))
            })
            .collect();
        let around: [Option<&Chunk>; 27] = std::array::from_fn(|k| guards[k].as_deref());
        let Some(chunk) = around[13] else {
            continue;
        };
        for (i, b) in chunk.bricks().iter().enumerate() {
            if let vxe_world::Brick::Full(p) = b {
                bins[bin_of(slot_units(p, translucent))] += 1;
            }
            if !field {
                continue;
            }
            let Some((counts, _)) = brick_counts(&around, i) else {
                continue;
            };
            let n = counts.iter().filter(|&&k| k > 0 && k < 27).count() as u32;
            if n == 0 {
                continue;
            }
            bins[band_bin_of(band_slot_units(n))] += 1;
            if let vxe_world::Brick::Uniform(m) = b {
                bins[bin_of(2 + u32::from(translucent(*m)))] += 1;
            }
        }
    }
    let pages: u64 = bins
        .iter()
        .enumerate()
        .map(|(k, &n)| n.div_ceil(u64::from(slots_per_page(k))))
        .sum();
    let per = u64::from(BINS[bin_of(6)]) + if field { u64::from(MAX_BAND_UNITS) } else { 0 };
    pages * u64::from(PAGE_UNITS) + POOL_HEADROOM_BRICKS * per
}

/// The mirror budget of the bounded level unless `--mirror-budget` sets one: the pool
/// [`level_pool_units`] needs and a record for each of its `chunks`, sized from the world it
/// loads rather than the streamed default.
pub fn level_mirror_budget(world: &vxe_world::World, chunks: u64, field: bool) -> u64 {
    vxe_render::internals::budget_for(level_pool_units(world, field), chunks)
}

/// The audio listener for `camera`: its position (voxel units), its forward and its up. The
/// mixer derives the listener's right as `forward x up`, which is the camera's right
/// (`forward x Z`), so a source on the right of the screen plays on the right channel.
pub fn listener(camera: &Camera) -> Listener {
    let (forward, _, up) = camera.basis();
    Listener {
        position: camera.position.to_array(),
        forward: forward.to_array(),
        up: up.to_array(),
    }
}

/// The material the [`thin_rig`] scene's branches, rods and sheets are made of, after the cave's
/// rows: the rig's "wood".
pub const THIN_RIG_WOOD: MaterialId = 5;

/// `table` (the cave's rows) with the thin rig's wood appended at [`THIN_RIG_WOOD`], a crisp
/// row ([`MaterialFlags::CRISP`]): the field surface draws its voxels as cubes.
pub fn with_wood(table: &MaterialTable) -> MaterialTable {
    with_wood_flags(table, MaterialFlags::CRISP)
}

/// [`with_wood`] with the wood's row carrying `flags` (`MaterialFlags::empty()` for smooth wood).
pub fn with_wood_flags(table: &MaterialTable, flags: MaterialFlags) -> MaterialTable {
    let mut t = table.clone();
    let wood = Material {
        albedo: [0.45, 0.30, 0.16],
        flags,
        ..Material::named("rig wood")
    };
    assert_eq!(
        t.push(wood),
        THIN_RIG_WOOD,
        "the wood follows the cave's rows"
    );
    t
}

/// A camera at `eye` looking at `target` (voxel units), 70 degrees vertical field of view.
pub fn camera_looking_at(eye: [f32; 3], target: [f32; 3]) -> Camera {
    let d = [0, 1, 2].map(|k| target[k] - eye[k]);
    let len = d.iter().map(|v| v * v).sum::<f32>().sqrt();
    Camera {
        position: eye.into(),
        yaw: d[1].atan2(d[0]),
        pitch: (d[2] / len).asin(),
        vfov: 70f32.to_radians(),
    }
}

/// The seam rig's camera: at its spawn's centre, looking +y head-on at the wall's inlay.
pub fn seam_rig_camera() -> Camera {
    let eye = thin_rig::SeamRig {
        crisp: THIN_RIG_WOOD,
    }
    .spawn()
    .0
    .map(|v| v as f32 + 0.5);
    camera_looking_at(eye, [eye[0], thin_rig::WALL_Y[0] as f32, eye[2]])
}

/// The thin rig's camera: on -y in front of the trunk, at its spawn's centre, looking +y and
/// slightly down at the features' middle.
pub fn thin_rig_camera() -> Camera {
    let eye = thin_rig::ThinRig {
        wood: THIN_RIG_WOOD,
    }
    .spawn()
    .0;
    let p = eye.map(|v| v as f32 + 0.5);
    let target = [64.5f32, 64.5, 90.0];
    let d = [0, 1, 2].map(|k| target[k] - p[k]);
    let len = d.iter().map(|v| v * v).sum::<f32>().sqrt();
    Camera {
        position: p.into(),
        yaw: d[1].atan2(d[0]),
        pitch: (d[2] / len).asin(),
        vfov: 70f32.to_radians(),
    }
}

/// The cave's table: air, rock, ore, the indestructible shell (at the ids `cave` uses), and
/// [`RUBBLE`]. It lives here rather than in `cave.rs` so that file stays free of floating point.
pub fn cave_materials() -> MaterialTable {
    let mut t = MaterialTable::new();
    let row = |name: &str, albedo: [f32; 3]| Material {
        albedo,
        ..Material::named(name)
    };
    assert_eq!(t.push(row("rock", [0.42, 0.38, 0.34])), cave::ROCK);
    assert_eq!(t.push(row("ore", [0.72, 0.45, 0.20])), cave::ORE);
    let mut shell = row("shell", [0.12, 0.12, 0.14]);
    shell.flags = MaterialFlags::INDESTRUCTIBLE;
    assert_eq!(t.push(shell), cave::SHELL);
    assert_eq!(t.push(row("rubble", [0.36, 0.33, 0.30])), RUBBLE);
    t
}

/// `table` (the cave's four rows after air) with the castle's seven rows appended at
/// `castle::MATERIAL_BASE` on, in palette order: stone, trim (darker stone for ribs, tracery
/// and merlons), slate, wood and iron, opaque; glass, translucent and tinted blue; and torch,
/// emitting a warm light. None is indestructible, so a dig into the castle is an ordinary edit.
pub fn castle_materials(table: &MaterialTable) -> MaterialTable {
    let mut t = table.clone();
    assert_eq!(
        t.len(),
        usize::from(castle::MATERIAL_BASE),
        "the castle's rows follow the cave's"
    );
    let row = |name: &str, albedo: [f32; 3]| Material {
        albedo,
        ..Material::named(name)
    };
    let rows = [
        row("castle stone", [0.55, 0.53, 0.50]),
        row("castle trim", [0.36, 0.35, 0.34]),
        row("castle slate", [0.20, 0.22, 0.27]),
        row("castle wood", [0.40, 0.27, 0.15]),
        Material {
            roughness: 0.5,
            ..row("castle iron", [0.18, 0.18, 0.20])
        },
        Material {
            flags: MaterialFlags::TRANSLUCENT,
            opacity: 0.6,
            ..row("castle glass", [0.25, 0.35, 0.85])
        },
        Material {
            emission: [3.0, 1.7, 0.6],
            ..row("castle torch", [1.0, 0.7, 0.3])
        },
    ];
    for (i, m) in (castle::STONE..=castle::TORCH).zip(rows) {
        assert_eq!(t.push(m), castle::material(i));
    }
    t
}

/// The ore's emission under `--emissive-ore on`: an orange glow, radiance per channel.
pub const ORE_EMISSION: [f32; 3] = [1.6, 0.9, 0.35];

/// `table` with the ore row emitting [`ORE_EMISSION`]: the fly-through's table unless
/// `--emissive-ore off`.
pub fn with_emissive_ore(table: &MaterialTable) -> MaterialTable {
    let mut t = MaterialTable::new();
    for (id, row) in table.rows().iter().enumerate().skip(1) {
        let mut m = row.clone();
        if id == usize::from(cave::ORE) {
            m.emission = ORE_EMISSION;
        }
        assert_eq!(usize::from(t.push(m)), id);
    }
    t
}

/// The showcase's crystal emission: cyan, radiance per channel.
pub const CRYSTAL_EMISSION: [f32; 3] = [0.35, 1.6, 2.0];

/// The showcase's amethyst emission: a dim violet.
pub const AMETHYST_EMISSION: [f32; 3] = [0.6, 0.2, 1.1];

/// The showcase's ember emission: deep red.
pub const EMBER_EMISSION: [f32; 3] = [2.4, 0.3, 0.08];

/// `table` (the cave's five rows, air included) with the showcase's three emissive rows
/// appended at `showcase::CRYSTAL`, `AMETHYST` and `EMBER`, each with rock's acoustics. The
/// showcase's amber is the table's ore row as it stands.
pub fn with_showcase_rows(table: &MaterialTable) -> MaterialTable {
    let mut t = table.clone();
    assert_eq!(
        t.len(),
        usize::from(showcase::CRYSTAL),
        "the showcase's rows follow the cave's"
    );
    let acoustic = t.get(cave::ROCK).map_or([0.0; 3], |r| r.acoustic);
    let row = |name: &str, albedo: [f32; 3], emission: [f32; 3]| Material {
        albedo,
        emission,
        acoustic,
        ..Material::named(name)
    };
    let rows = [
        (
            showcase::CRYSTAL,
            row("crystal", [0.35, 0.75, 0.85], CRYSTAL_EMISSION),
        ),
        (
            showcase::AMETHYST,
            row("amethyst", [0.45, 0.25, 0.60], AMETHYST_EMISSION),
        ),
        (
            showcase::EMBER,
            row("ember", [0.60, 0.22, 0.10], EMBER_EMISSION),
        ),
    ];
    for (id, m) in rows {
        assert_eq!(t.push(m), id);
    }
    t
}

/// The showcase's table: [`cave_materials`] with the ore emitting [`ORE_EMISSION`] (the
/// amber) and the three emissive rows of [`with_showcase_rows`].
pub fn showcase_materials() -> MaterialTable {
    with_showcase_rows(&with_emissive_ore(&cave_materials()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_streamed_mirror_reserves_records_for_the_residency_ball_over_the_restore_line() {
        assert_eq!(vxe_world::chunks_in_ball(25), 65_267);
        let n = stream_record_chunks(RESIDENCY);
        assert_eq!(n, 93_239);
        assert_eq!(n * 2624, 244_659_136);
        // The smallest count whose occupancy at the full ball stays at or under 0.7.
        assert!(65_267 * 10 <= n * 7 && 65_267 * 10 > (n - 1) * 7);
        assert!((PRESSURE_LOW - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn the_default_window_holds_the_default_residency() {
        assert_eq!(window_side(RESIDENCY), 51);
    }
}
