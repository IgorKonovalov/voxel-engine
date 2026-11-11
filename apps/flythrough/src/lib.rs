//! The fly-through's reusable parts: the cave world source its binary, tests and examples
//! share, its server logic (the `flythrough-logic` crate, as [`logic`]) and the choice of its
//! native or hosted build, save replay, the streaming defaults, and the small measurement
//! helpers its readings use. The engine ships no terrain; this cave is the testbed's own.

pub mod castle;
pub mod cave;
pub mod hud;
pub mod path;
pub mod report;
pub mod showcase;
pub mod thin_rig;
pub use flythrough_logic as logic;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use vxe_audio::Listener;
use vxe_net::{Logic, Server};
use vxe_render::{Camera, NormalMode, SurfaceTuning};
use vxe_world::hash::Fnv1a;
use vxe_world::{
    BRICK_VOXELS, Brick, Chunk, ChunkCoord, ChunkExtent, ChunkMips, LogFile, Material,
    MaterialFlags, MaterialId, MaterialTable, OpLog, Residency, WireError, World,
};

use cave::CaveSource;
use logic::DigLogic;

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
pub const STREAM_MIRROR_BUDGET: u64 = vxe_render::gpu_world::DEFAULT_BUDGET_BYTES;

/// `Full` bricks of headroom beyond the bounded level's own, so digging has room: 64 Ki
/// bricks, each at the largest slot the level's rock takes (6 units), and with the field a
/// band slot of the most units ([`vxe_render::field::MAX_BAND_UNITS`]) beside each.
pub const POOL_HEADROOM_BRICKS: u64 = 65_536;

/// The pool units the bounded level `world` needs: each bin's `Full` bricks in whole pages
/// of that bin, and when `field` each band brick's band slot and each uniform band brick's
/// slot in pages of theirs (the band as [`band_census`] reads it), plus the headroom.
pub fn level_pool_units(world: &vxe_world::World, field: bool) -> u64 {
    use vxe_render::field::{MAX_BAND_UNITS, band_slot_units};
    use vxe_render::gpu_world::{
        BIN_COUNT, BINS, PAGE_UNITS, band_bin_of, bin_of, slot_units, slots_per_page,
    };
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
    vxe_render::gpu_world::budget_for(level_pool_units(world, field), chunks)
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

/// The rubble a collapsing region leaves when `R` has the fly-through's logic remove
/// unsupported regions instead of dropping them. The cave never generates it.
pub use logic::RUBBLE;

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

/// A named look of the window: a normal mode and the surface tuning. `--look` starts in one, `N` cycles them in [`LookPreset::ALL`] order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookPreset {
    pub name: &'static str,
    pub normal_mode: NormalMode,
    pub tuning: SurfaceTuning,
    /// `LookParams::far_normal`, `[0.0, 0.0]` off.
    pub far_normal: [f32; 2],
}

impl LookPreset {
    /// A preset of `mode` with no face mix.
    const fn of(name: &'static str, normal_mode: NormalMode) -> Self {
        Self {
            name,
            normal_mode,
            tuning: SurfaceTuning { face_mix: 0.0 },
            far_normal: [0.0; 2],
        }
    }

    pub const FACE: Self = Self::of("face", NormalMode::Face);
    /// The field surface (ADR-0041, `NormalMode::Contour`), an opt-in look; a material marked
    /// `MaterialFlags::CRISP` in the table renders as cubes in it. It needs a mirror with the
    /// field's band slots.
    pub const FIELD: Self = Self::of("field", NormalMode::Contour);
    pub const TRILINEAR8: Self = Self::of("trilinear8", NormalMode::Trilinear8);
    pub const GRADIENT27: Self = Self::of("gradient27", NormalMode::Gradient27);

    /// `N`'s order: face, the field, then the two diagnostic normal modes.
    pub const ALL: [Self; 4] = [Self::FACE, Self::FIELD, Self::TRILINEAR8, Self::GRADIENT27];

    /// The look the fly-through starts in without `--look`: honest cubes (ADR-0041) with the
    /// far normal at `[2, 6]`, the owner's provisional pick. `--look face` is the same cubes
    /// with the far normal off.
    pub const DEFAULT: Self = Self {
        far_normal: [2.0, 6.0],
        ..Self::FACE
    };

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name == name)
    }

    /// The names `--look` takes, comma-separated.
    pub fn names() -> String {
        Self::ALL.map(|p| p.name).join(", ")
    }

    /// The next preset in [`LookPreset::ALL`] order, wrapping, skipping the field unless
    /// `contour` is set (a mirror with the field's band slots). A preset not in the list
    /// (one with a part overridden) continues from the first of its normal mode.
    pub fn next(self, contour: bool) -> Self {
        let i = Self::ALL
            .iter()
            .position(|p| *p == self)
            .or_else(|| {
                Self::ALL
                    .iter()
                    .position(|p| p.normal_mode == self.normal_mode)
            })
            .unwrap_or(0);
        (1..=Self::ALL.len())
            .map(|k| Self::ALL[(i + k) % Self::ALL.len()])
            .find(|p| contour || p.normal_mode != NormalMode::Contour)
            .unwrap_or(Self::FACE)
    }

    /// The parts beyond the normal mode: `face mix 0`.
    pub fn parts(&self) -> String {
        format!("face mix {}", self.tuning.face_mix)
    }

    /// The far normal in words: `far normal 2..6` in face mode, the one mode that reads it, and
    /// `far normal off` in the others or with the knob off.
    pub fn far_normal_words(normal_mode: NormalMode, far_normal: [f32; 2]) -> String {
        let [lo, hi] = far_normal;
        if normal_mode == NormalMode::Face && hi > lo {
            format!("far normal {lo}..{hi}")
        } else {
            "far normal off".to_owned()
        }
    }
}

/// The field surface's band over a set of chunks (ADR-0040 Alternative B): a voxel is in the
/// band when its 27-count, the solid voxels of the 3 x 3 x 3 block around it with itself, is
/// neither 0 nor 27; a brick is a band brick when it holds a band voxel. Solid is not air
/// ([`Brick::solid_bits`]), a translucent voxel included, as for the field pass. The bytes
/// are a band slot's: packed, one band-mask unit plus `ceil(n / 64)` units of `u8` counts for
/// `n` band voxels, plus two header units for a uniform brick (which has no slot today);
/// dense, nine units per band brick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BandCensus {
    /// Band voxels that are solid.
    pub solid_voxels: u64,
    /// Band voxels that are air.
    pub air_voxels: u64,
    /// Band bricks that are `Full`.
    pub full_bricks: u64,
    /// `Full` bricks with a surface voxel (a solid voxel with an air voxel among its 26
    /// neighbours).
    pub surface_full_bricks: u64,
    /// `Full` bricks with a surface voxel that are band bricks.
    pub surface_full_band_bricks: u64,
    /// Band bricks that are uniform and solid.
    pub solid_bricks: u64,
    /// Band bricks that are uniform air.
    pub air_bricks: u64,
    pub packed_bytes: u64,
    pub dense_bytes: u64,
}

impl BandCensus {
    pub fn bricks(&self) -> u64 {
        self.full_bricks + self.solid_bricks + self.air_bricks
    }
}

/// The 27-counts of the 512 voxels of brick `slot` of the chunk at the centre of `around`
/// (the 27 chunks around it, index `(dx + 1) + 3 (dy + 1) + 9 (dz + 1)`, `None` read as air),
/// voxel order `x + 8 y + 64 z`, and the brick's surface (its solid voxels with an air face
/// neighbour, bit `i % 64` of word `i / 64`); `None` when the brick and its 26 neighbours are
/// all uniform and all solid or all air (every count 27 or 0, no surface).
pub fn brick_counts(
    around: &[Option<&Chunk>; 27],
    slot: usize,
) -> Option<([u8; BRICK_VOXELS], [u64; 8])> {
    let l = vxe_world::brick_of_slot(slot);
    // The 27 bricks around, as their solid bits; `kinds` says whether all are one uniform kind.
    let mut bits = [[0u64; 8]; 27];
    let (mut any_solid, mut any_air, mut any_full) = (false, false, false);
    for (k, out) in bits.iter_mut().enumerate() {
        let d = [k as i32 % 3 - 1, (k as i32 / 3) % 3 - 1, k as i32 / 9 - 1];
        let b = [0, 1, 2].map(|a| l[a] + d[a]);
        let cd = b.map(|v| v.div_euclid(8));
        let ci = ((cd[0] + 1) + 3 * (cd[1] + 1) + 9 * (cd[2] + 1)) as usize;
        let brick = around[ci].map(|c| c.brick(vxe_world::brick_slot(b.map(|v| v.rem_euclid(8)))));
        match brick {
            None | Some(Brick::Uniform(vxe_world::AIR)) => any_air = true,
            Some(Brick::Uniform(_)) => {
                any_solid = true;
                *out = [u64::MAX; 8];
            }
            Some(b @ Brick::Full(_)) => {
                any_full = true;
                *out = b.solid_bits();
            }
        }
    }
    if !any_full && !(any_solid && any_air) {
        return None;
    }
    // The occupancy of the brick and a one-voxel shell, 10^3, cell c at voxel c - 1.
    let mut occ = [0u8; 1000];
    for z in 0..10i32 {
        for y in 0..10i32 {
            for x in 0..10i32 {
                let v = [x - 1, y - 1, z - 1];
                let d = v.map(|c| c.div_euclid(8));
                let k = ((d[0] + 1) + 3 * (d[1] + 1) + 9 * (d[2] + 1)) as usize;
                let lv = v.map(|c| c.rem_euclid(8));
                let on = (bits[k][lv[2] as usize] >> (lv[0] + 8 * lv[1])) & 1;
                occ[(x + 10 * (y + 10 * z)) as usize] = on as u8;
            }
        }
    }
    // Separable box sums: along x (8 x 10 x 10), y (8 x 8 x 10), z (8 x 8 x 8).
    let mut sx = [0u8; 800];
    for z in 0..10 {
        for y in 0..10 {
            for x in 0..8 {
                let i = x + 10 * (y + 10 * z);
                sx[x + 8 * (y + 10 * z)] = occ[i] + occ[i + 1] + occ[i + 2];
            }
        }
    }
    let mut sy = [0u8; 640];
    for z in 0..10 {
        for y in 0..8 {
            for x in 0..8 {
                let i = x + 8 * (y + 10 * z);
                sy[x + 8 * (y + 8 * z)] = sx[i] + sx[i + 8] + sx[i + 16];
            }
        }
    }
    let mut out = [0u8; BRICK_VOXELS];
    let mut surface = [0u64; 8];
    for (i, c) in out.iter_mut().enumerate() {
        *c = sy[i] + sy[i + 64] + sy[i + 128];
        let (x, y, z) = (i % 8 + 1, (i / 8) % 8 + 1, i / 64 + 1);
        let at = |x: usize, y: usize, z: usize| occ[x + 10 * (y + 10 * z)];
        if at(x, y, z) == 1
            && (at(x - 1, y, z)
                & at(x + 1, y, z)
                & at(x, y - 1, z)
                & at(x, y + 1, z)
                & at(x, y, z - 1)
                & at(x, y, z + 1))
                == 0
        {
            surface[i / 64] |= 1 << (i % 64);
        }
    }
    Some((out, surface))
}

/// The chunks around `c` that `world` holds, for [`brick_counts`]; the guards are read locks.
fn around_guards(world: &World, c: ChunkCoord) -> Vec<Option<Arc<std::sync::RwLock<Chunk>>>> {
    (0..27)
        .map(|k| {
            let d = [k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1];
            world
                .store()
                .get(ChunkCoord([0, 1, 2].map(|a| c.0[a] + d[a])))
        })
        .collect()
}

/// [`BandCensus`] over the bricks of `chunks` in `world`; a voxel of a chunk `world` does not
/// hold reads as air.
pub fn band_census(world: &World, chunks: &[ChunkCoord]) -> BandCensus {
    let mut cs = BandCensus::default();
    for &c in chunks {
        let arcs = around_guards(world, c);
        let guards: Vec<_> = arcs
            .iter()
            .map(|a| {
                a.as_ref()
                    .map(|a| a.read().unwrap_or_else(std::sync::PoisonError::into_inner))
            })
            .collect();
        let around: [Option<&Chunk>; 27] = std::array::from_fn(|k| guards[k].as_deref());
        let Some(centre) = around[13] else {
            continue;
        };
        for slot in 0..vxe_world::CHUNK_BRICKS {
            let Some((counts, surface)) = brick_counts(&around, slot) else {
                continue;
            };
            let brick = centre.brick(slot);
            let solid = brick.solid_bits();
            let surface = surface.iter().any(|&w| w != 0);
            let mut n = 0u64;
            for (i, &k) in counts.iter().enumerate() {
                let on = (solid[i / 64] >> (i % 64)) & 1 != 0;
                if k > 0 && k < 27 {
                    n += 1;
                    if on {
                        cs.solid_voxels += 1;
                    } else {
                        cs.air_voxels += 1;
                    }
                }
            }
            let full = brick.is_full();
            if full && surface {
                cs.surface_full_bricks += 1;
                if n > 0 {
                    cs.surface_full_band_bricks += 1;
                }
            }
            if n == 0 {
                continue;
            }
            match brick {
                Brick::Full(_) => cs.full_bricks += 1,
                Brick::Uniform(vxe_world::AIR) => cs.air_bricks += 1,
                Brick::Uniform(_) => cs.solid_bricks += 1,
            }
            let header = if full { 0 } else { 2 };
            cs.packed_bytes += 64 * (1 + n.div_ceil(64) + header);
            cs.dense_bytes += 64 * 9;
        }
    }
    cs
}

/// FNV-1a 64 over the voxels of the resident chunks `coords`, in the given order: each as its
/// coordinate (three little-endian `i32`), then its bricks in slot order with voxels x-fastest
/// as little-endian `u16`, a `Uniform` brick expanded. `None` if one is not resident.
pub fn chunks_hash(world: &World, coords: &[ChunkCoord]) -> Option<u64> {
    let mut h = Fnv1a::default();
    for c in coords {
        for v in c.0 {
            h.write(&v.to_le_bytes());
        }
        let chunk = world.store().get(*c)?;
        let chunk = chunk
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for b in chunk.bricks() {
            for i in 0..BRICK_VOXELS {
                h.write(&b.get(i).to_le_bytes());
            }
        }
    }
    Some(h.finish())
}

impl CaveSource {
    /// The cave's generation version, written into a save's header. A save made under
    /// another version names other voxels and does not replay.
    pub const VERSION: u32 = 1;
}

/// Where `scripts/build-guests.sh` places the logic component, relative to the workspace root.
pub const DEFAULT_COMPONENT: &str = "target/guests/flythrough-logic.wasm";

/// Why `--logic wasm` is refused by a build without the `wasm` feature.
pub const WASM_NEEDS_FEATURE: &str =
    "--logic wasm needs a build with --features wasm (cargo run -p flythrough --features wasm)";

/// Which build of [`DigLogic`] the server runs: `--logic native` or `--logic wasm[=path]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LogicChoice {
    /// [`DigLogic`] linked into the binary.
    #[default]
    Native,
    /// The `flythrough-logic` component at this path, hosted by `vxe-script-wasm`.
    Wasm(PathBuf),
}

impl LogicChoice {
    /// Parses `native`, `wasm` (the component at [`DEFAULT_COMPONENT`]) or `wasm=PATH`.
    pub fn parse(v: &str) -> Result<Self, String> {
        match v.split_once('=') {
            None if v == "native" => Ok(Self::Native),
            None if v == "wasm" => Ok(Self::Wasm(DEFAULT_COMPONENT.into())),
            Some(("wasm", path)) if !path.is_empty() => Ok(Self::Wasm(path.into())),
            _ => Err(format!("--logic takes native, wasm or wasm=PATH, not {v}")),
        }
    }

    /// A fresh logic of this choice. For [`LogicChoice::Wasm`], loads and compiles the
    /// component under the default `WasmConfig`; fails with the load error, or with
    /// [`WASM_NEEDS_FEATURE`] in a build without the `wasm` feature.
    pub fn load(&self) -> Result<Box<dyn Logic>, String> {
        match self {
            Self::Native => Ok(Box::new(DigLogic::default())),
            Self::Wasm(path) => load_component(path),
        }
    }
}

impl std::fmt::Display for LogicChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native => write!(f, "native"),
            Self::Wasm(path) => write!(f, "wasm ({})", path.display()),
        }
    }
}

#[cfg(feature = "wasm")]
fn load_component(path: &Path) -> Result<Box<dyn Logic>, String> {
    let config = vxe_script_wasm::WasmConfig::default();
    match vxe_script_wasm::WasmLogic::load(path, &config) {
        Ok(logic) => Ok(Box::new(logic)),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(not(feature = "wasm"))]
fn load_component(_path: &Path) -> Result<Box<dyn Logic>, String> {
    Err(WASM_NEEDS_FEATURE.to_owned())
}

/// How [`replay_file`] rebuilds a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Applies the op log.
    Ops,
    /// Re-simulates the input log through a fresh [`DigLogic`], with the op log for the
    /// boundaries of directly submitted ops.
    Inputs,
}

/// Why a save did not replay.
#[derive(Debug)]
pub enum ReplayError {
    Io(std::io::Error),
    Wire(WireError),
    /// The save was written by another [`CaveSource::VERSION`].
    SourceVersion {
        file: u32,
        ours: u32,
    },
    /// The save's extent is not the one the caller replays (the bounded level's, for the
    /// binary); checked before anything is generated.
    Extent {
        file: ChunkExtent,
    },
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Wire(e) => write!(f, "{e}"),
            Self::SourceVersion { file, ours } => {
                write!(f, "saved with cave version {file}, this build is {ours}")
            }
            Self::Extent { file } => write!(
                f,
                "saved over chunks {:?}..{:?}, not the level's extent",
                file.min.0, file.max.0
            ),
        }
    }
}

impl std::error::Error for ReplayError {}

/// Reads the save at `path` and rebuilds its world (see [`replay_log`]) with
/// [`cave_materials`], refusing a save whose extent is not `extent`. Returns the world and
/// its voxel hash.
pub fn replay_file(
    path: impl AsRef<std::path::Path>,
    mode: Mode,
    extent: ChunkExtent,
) -> Result<(World, u64), ReplayError> {
    replay_file_with(path, mode, extent, Box::new(DigLogic::default()))
}

/// [`replay_file`] with `logic` re-simulating the inputs under [`Mode::Inputs`] (unused under
/// [`Mode::Ops`]): a fresh [`DigLogic`], native or hosted.
pub fn replay_file_with(
    path: impl AsRef<std::path::Path>,
    mode: Mode,
    extent: ChunkExtent,
    logic: Box<dyn Logic>,
) -> Result<(World, u64), ReplayError> {
    let file = read_log_file(path)?;
    let (world, _) = replay_log_with(&file, mode, extent, &cave_materials(), logic)?;
    let hash = world.voxel_hash();
    Ok((world, hash))
}

/// Reads and decodes the save at `path`.
pub fn read_log_file(path: impl AsRef<std::path::Path>) -> Result<LogFile, ReplayError> {
    let mut f = std::fs::File::open(path).map_err(ReplayError::Io)?;
    LogFile::read_from(&mut f).map_err(ReplayError::Wire)
}

/// Rebuilds a save's world: the bounded cave of its seed over its extent, which must equal
/// `extent` (so a corrupt header cannot ask for an unbounded world), generated with `table`,
/// then its op log replayed ([`Mode::Ops`]) or its input log re-simulated through
/// a fresh [`DigLogic`] seeded with the save's seed ([`Mode::Inputs`]). Returns the world and
/// the op log it was rebuilt from: the save's, or the one re-simulation produced.
pub fn replay_log(
    file: &LogFile,
    mode: Mode,
    extent: ChunkExtent,
    table: &MaterialTable,
) -> Result<(World, OpLog), ReplayError> {
    replay_log_with(file, mode, extent, table, Box::new(DigLogic::default()))
}

/// [`replay_log`] with `logic` re-simulating the inputs under [`Mode::Inputs`] (unused under
/// [`Mode::Ops`]): a fresh [`DigLogic`], native or hosted, initialised with the save's seed.
pub fn replay_log_with(
    file: &LogFile,
    mode: Mode,
    extent: ChunkExtent,
    table: &MaterialTable,
    logic: Box<dyn Logic>,
) -> Result<(World, OpLog), ReplayError> {
    replay_log_on(file, mode, extent, table, logic, false)
}

/// [`replay_log_with`] on the cave generated with the `--vault` room when `vault` is set. The
/// save's header does not record the room (its placement at generation is not an op), so a
/// session saved with `--vault` replays to its world only with `vault` set.
pub fn replay_log_on(
    file: &LogFile,
    mode: Mode,
    extent: ChunkExtent,
    table: &MaterialTable,
    logic: Box<dyn Logic>,
    vault: bool,
) -> Result<(World, OpLog), ReplayError> {
    if file.extent != extent {
        return Err(ReplayError::Extent { file: file.extent });
    }
    if file.source_version != CaveSource::VERSION {
        return Err(ReplayError::SourceVersion {
            file: file.source_version,
            ours: CaveSource::VERSION,
        });
    }
    let source = CaveSource::new(file.seed, file.extent);
    let world = if vault {
        World::generate(file.extent, &source.with_vault(), table)
    } else {
        World::generate(file.extent, &source, table)
    };
    match mode {
        Mode::Ops => {
            world.replay(&file.ops, &file.prefabs);
            Ok((world, file.ops.clone()))
        }
        Mode::Inputs => {
            let world = Arc::new(world);
            let ops = Server::resimulate(
                world.clone(),
                logic,
                file.seed,
                &file.prefabs,
                &file.ops,
                &file.inputs,
            );
            let world = Arc::into_inner(world).expect("the re-simulation's server is dropped");
            Ok((world, ops))
        }
    }
}

/// RAM a resident full chunk holds: its slot array and the class boxes of its `Full` bricks.
pub fn chunk_ram(chunk: &Chunk) -> u64 {
    let payload: usize = chunk.bricks().iter().map(Brick::payload_bytes).sum();
    (size_of::<Chunk>() + payload) as u64
}

/// RAM a LOD-only chunk holds: its 73 mip-brick slots and the class boxes of its `Full` ones.
pub fn mips_ram(mips: &ChunkMips) -> u64 {
    let slots: usize = vxe_world::MIP_BRICKS.iter().sum();
    let payload: usize = (1..=vxe_world::MIP_LEVELS)
        .flat_map(|n| mips.level(n))
        .map(Brick::payload_bytes)
        .sum();
    (size_of::<ChunkMips>() + slots * size_of::<Brick>() + payload) as u64
}

/// The `p`-th percentile (0 to 100) of `v` by nearest rank; `NaN` when empty. Sorts `v`.
pub fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * v.len() as f64).ceil() as usize;
    v[rank.clamp(1, v.len()) - 1]
}

/// Scroll lines banked into whole steps: smooth or touchpad scrolling delivers a fraction of
/// a line per frame, which rounding per frame would turn into no step at all.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollSteps {
    carry: f32,
}

impl ScrollSteps {
    /// Adds a frame's scroll in lines and returns the whole steps banked, rounded toward
    /// zero; the fraction stays for the next frame.
    pub fn add(&mut self, delta: f32) -> i64 {
        self.carry += delta;
        let steps = self.carry.trunc();
        self.carry -= steps;
        steps as i64
    }

    /// The fraction of a step banked.
    pub fn carry(&self) -> f32 {
        self.carry
    }

    /// Drops the banked fraction.
    pub fn clear(&mut self) {
        self.carry = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_steps_bank_fractions_and_step_toward_zero() {
        let mut s = ScrollSteps::default();
        let quarters: Vec<i64> = [0.25; 4].iter().map(|&d| s.add(d)).collect();
        assert_eq!(quarters, [0, 0, 0, 1]);
        assert_eq!(s.carry(), 0.0);
        assert_eq!([s.add(0.3), s.add(-0.3)], [0, 0]);
        assert_eq!(s.add(2.0), 2);
        let mut s = ScrollSteps::default();
        assert_eq!([s.add(-0.6), s.add(-0.6)], [0, -1]);
        assert!((s.carry() + 0.2).abs() < 1e-6, "carry {}", s.carry());
    }

    #[test]
    fn percentiles_by_nearest_rank() {
        let mut v: Vec<f64> = (1..=100).rev().map(f64::from).collect();
        assert_eq!(percentile(&mut v, 50.0), 50.0);
        assert_eq!(percentile(&mut v, 99.0), 99.0);
        assert_eq!(percentile(&mut v, 100.0), 100.0);
        assert_eq!(percentile(&mut [3.0], 95.0), 3.0);
        assert!(percentile(&mut [], 50.0).is_nan());
    }

    #[test]
    fn logic_choice_parses_native_wasm_and_a_path() {
        assert_eq!(LogicChoice::parse("native"), Ok(LogicChoice::Native));
        assert_eq!(
            LogicChoice::parse("wasm"),
            Ok(LogicChoice::Wasm(DEFAULT_COMPONENT.into()))
        );
        assert_eq!(
            LogicChoice::parse("wasm=a/b.wasm"),
            Ok(LogicChoice::Wasm("a/b.wasm".into()))
        );
        for bad in ["", "wasm=", "lua", "native=x"] {
            assert!(LogicChoice::parse(bad).is_err(), "{bad:?}");
        }
    }

    /// Without the `wasm` feature a hosted logic is refused with a message naming it.
    #[cfg(not(feature = "wasm"))]
    #[test]
    fn a_build_without_the_wasm_feature_refuses_the_component() {
        let e = LogicChoice::Wasm(DEFAULT_COMPONENT.into())
            .load()
            .err()
            .expect("refused");
        assert!(e.contains("--features wasm"), "{e}");
        assert!(LogicChoice::Native.load().is_ok());
    }

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
