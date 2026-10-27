//! A seeded cave: integer value noise thresholded into rock and air, ore veins from a second
//! noise, an indestructible shell around the extent, and an air pocket at the spawn point.
//! The unbounded cave ([`CaveSource::unbounded`]) is the same noise everywhere, with no extent
//! and no shell, and its spawn pocket at the fixed voxel [`UNBOUNDED_SPAWN`].
//!
//! Every value is a pure integer function of the seed and the voxel coordinate, so chunks can
//! be generated in any order on any thread and on any platform with identical bytes.
//!
//! Density is the sum of two octaves, caverns at cell 64 and detail at cell 16 (one eighth
//! weight); ore veins come from a third noise at cell 16. All lattices align with bricks, so inside one brick each octave is a trilinear function of one cell, and a
//! trilinear function over a box takes its extremes at the box's corners. A brick whose eight
//! corner densities all lie beyond `MARGIN` on one side of a threshold is therefore uniform
//! on that side and is emitted without evaluating its voxels. `MARGIN` covers the integer
//! rounding in `ValueNoise::sample` (under one unit per octave, plus the detail shift).

use std::sync::Arc;

use vxe_world::hash::ValueNoise;
use vxe_world::{
    AIR, BRICK_EDGE, Brick, CHUNK_BRICKS, CHUNK_EDGE, Chunk, ChunkCoord, ChunkExtent, MaterialId,
    Prefab, Rot, StampedSource, VoxelCoord, WorldSource, brick_of_slot,
};

pub const ROCK: MaterialId = 1;
pub const ORE: MaterialId = 2;
pub const SHELL: MaterialId = 3;

/// Density above which a voxel is air.
const AIR_ABOVE: i64 = 6000;
/// Ore-noise value above which rock is ore.
const ORE_ABOVE: i64 = 28000;
/// The detail octave is divided by `1 << DETAIL_SHIFT` before it is added to the caverns.
const DETAIL_SHIFT: u32 = 3;
/// Rounding slack for the per-brick early-out.
const MARGIN: i64 = 4;
/// Voxels of indestructible shell inside each face of the extent.
const SHELL_THICKNESS: i32 = 2;
/// Radius of the spawn pocket, in voxels (3 m at 10 cm).
pub const SPAWN_POCKET_RADIUS: i64 = 30;
/// Chunks of the level the fly-through generates: 1280 x 1280 x 512 voxels.
pub const LEVEL_CHUNKS: [i32; 3] = [20, 20, 8];
/// The spawn voxel of the unbounded cave: the centre of chunk `[0, 0, 0]`.
pub const UNBOUNDED_SPAWN: VoxelCoord = VoxelCoord([32, 32, 32]);

/// Edge of the `--vault` room, in voxels (160 cm at 10 cm).
pub const VAULT_EDGE: u32 = 16;
/// How far along +x from the spawn the `--vault` room's minimum corner sits (10 m).
pub const VAULT_AHEAD: i32 = 100;

/// The `--vault` room: a [`VAULT_EDGE`] cube of [`SHELL`] around air.
pub fn vault() -> Prefab {
    Prefab::vault(VAULT_EDGE, SHELL)
}

/// The cave over a bounded extent of chunks, or unbounded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaveSource {
    pub seed: u64,
    /// The bounded cave's extent; empty for the unbounded cave.
    pub extent: ChunkExtent,
    /// Whether the extent's faces carry the indestructible shell.
    pub bounded: bool,
}

impl CaveSource {
    /// The cave over `extent`, inside a shell, with its spawn pocket at the extent's centre.
    pub fn new(seed: u64, extent: ChunkExtent) -> Self {
        Self {
            seed,
            extent,
            bounded: true,
        }
    }

    /// The cave everywhere: no extent, no shell, the spawn pocket at [`UNBOUNDED_SPAWN`].
    pub fn unbounded(seed: u64) -> Self {
        Self {
            seed,
            extent: ChunkExtent::new([0, 0, 0], [0, 0, 0]),
            bounded: false,
        }
    }

    /// The voxel where the spawn pocket is carved: the centre of the extent, or
    /// [`UNBOUNDED_SPAWN`].
    pub fn spawn(&self) -> VoxelCoord {
        if !self.bounded {
            return UNBOUNDED_SPAWN;
        }
        VoxelCoord(
            [0, 1, 2].map(|k| (self.extent.min.0[k] + self.extent.max.0[k]) * CHUNK_EDGE / 2),
        )
    }

    /// The `--vault` room's minimum corner: [`VAULT_AHEAD`] voxels along +x from the spawn,
    /// clamped on the bounded cave so the whole room lies inside the extent.
    pub fn vault_origin(&self) -> VoxelCoord {
        let s = self.spawn().0;
        let mut o = [s[0] + VAULT_AHEAD, s[1], s[2]];
        if self.bounded {
            let edge = VAULT_EDGE as i32;
            for (k, c) in o.iter_mut().enumerate() {
                let lo = self.extent.min.0[k] * CHUNK_EDGE;
                let hi = self.extent.max.0[k] * CHUNK_EDGE - edge;
                *c = (*c).min(hi).max(lo);
            }
        }
        VoxelCoord(o)
    }

    /// This cave with the `--vault` room placed at [`CaveSource::vault_origin`].
    pub fn with_vault(self) -> StampedSource<CaveSource> {
        let origin = self.vault_origin();
        StampedSource::new(self, vec![(Arc::new(vault()), origin, Rot::IDENTITY)])
    }

    fn density_noise(&self) -> (ValueNoise, ValueNoise) {
        (
            ValueNoise::new(self.seed, 6),
            ValueNoise::new(self.seed ^ 0xD3A5_17C0_0000_0001, 4),
        )
    }

    fn ore_noise(&self) -> ValueNoise {
        ValueNoise::new(self.seed ^ 0x0EE0_0EE0_0000_0002, 4)
    }

    fn density(&self, p: [i32; 3]) -> i64 {
        let (a, b) = self.density_noise();
        a.sample(p) + (b.sample(p) >> DETAIL_SHIFT)
    }

    /// Rock, ore or air from the noise alone, before shell and pocket.
    fn base(&self, p: [i32; 3]) -> MaterialId {
        if self.density(p) > AIR_ABOVE {
            AIR
        } else if self.ore_noise().sample(p) > ORE_ABOVE {
            ORE
        } else {
            ROCK
        }
    }

    fn in_shell(&self, p: [i32; 3]) -> bool {
        self.bounded
            && (0..3).any(|k| {
                p[k] < self.extent.min.0[k] * CHUNK_EDGE + SHELL_THICKNESS
                    || p[k] >= self.extent.max.0[k] * CHUNK_EDGE - SHELL_THICKNESS
            })
    }

    fn in_pocket(&self, p: [i32; 3]) -> bool {
        let s = self.spawn().0;
        let d2: i64 = (0..3)
            .map(|k| (i64::from(p[k]) - i64::from(s[k])).pow(2))
            .sum();
        d2 <= SPAWN_POCKET_RADIUS * SPAWN_POCKET_RADIUS
    }

    /// The final material of voxel `p`.
    pub fn material(&self, p: [i32; 3]) -> MaterialId {
        if self.in_shell(p) {
            SHELL
        } else if self.in_pocket(p) {
            AIR
        } else {
            self.base(p)
        }
    }

    /// The brick with minimum voxel `b`: uniform from its corners when they allow it, else
    /// evaluated per voxel.
    pub fn brick(&self, b: [i32; 3]) -> Brick {
        let hi = b.map(|v| v + BRICK_EDGE - 1);
        let near_shell = self.bounded
            && (0..3).any(|k| {
                b[k] < self.extent.min.0[k] * CHUNK_EDGE + SHELL_THICKNESS
                    || hi[k] >= self.extent.max.0[k] * CHUNK_EDGE - SHELL_THICKNESS
            });
        let s = self.spawn().0;
        let pocket_near: i64 = (0..3)
            .map(|k| (i64::from(s[k].clamp(b[k], hi[k])) - i64::from(s[k])).pow(2))
            .sum();
        let near_pocket = pocket_near <= SPAWN_POCKET_RADIUS * SPAWN_POCKET_RADIUS;
        if !near_shell
            && !near_pocket
            && let Some(m) = self.uniform_from_corners(b, hi)
        {
            return Brick::Uniform(m);
        }
        // Every octave's lattice cell is at least a brick and brick-aligned, so one set of
        // corners per noise serves all 512 voxels.
        let (dn, dd) = self.density_noise();
        let on = self.ore_noise();
        let (dc, ddc, oc) = (
            dn.corners(dn.cell_of(b)),
            dd.corners(dd.cell_of(b)),
            on.corners(on.cell_of(b)),
        );
        Brick::from_fn(|l| {
            let p = [0, 1, 2].map(|k| b[k] + l[k]);
            if self.in_shell(p) {
                SHELL
            } else if self.in_pocket(p)
                || dn.interpolate(&dc, p) + (dd.interpolate(&ddc, p) >> DETAIL_SHIFT) > AIR_ABOVE
            {
                AIR
            } else if on.interpolate(&oc, p) > ORE_ABOVE {
                ORE
            } else {
                ROCK
            }
        })
    }

    /// The material of every voxel in `lo..=hi` when the corner values prove it uniform.
    fn uniform_from_corners(&self, lo: [i32; 3], hi: [i32; 3]) -> Option<MaterialId> {
        let corners: [[i32; 3]; 8] =
            std::array::from_fn(|i| [0, 1, 2].map(|k| if i >> k & 1 == 0 { lo[k] } else { hi[k] }));
        let d = corners.map(|c| self.density(c));
        if d.iter().all(|&v| v > AIR_ABOVE + MARGIN) {
            return Some(AIR);
        }
        if !d.iter().all(|&v| v <= AIR_ABOVE - MARGIN) {
            return None;
        }
        let ore = self.ore_noise();
        corners
            .iter()
            .all(|&c| ore.sample(c) <= ORE_ABOVE - MARGIN)
            .then_some(ROCK)
    }
}

/// The `--castle` hall's air before its noise, as voxel half-open ranges per axis: 80 x 80 x
/// 48 m around the castle's box, its floor at `z = 12`.
pub const HALL_LO: [i32; 3] = [-18, -368, 12];
/// The exclusive upper corner of the hall's air before its noise.
pub const HALL_HI: [i32; 3] = [782, 432, 492];
/// The most voxels the hall's noise pushes a wall or the ceiling outward.
pub const HALL_NOISE: i32 = 16;
/// The bottom of the rock plinth under the hall, inclusive; the plinth's top is the floor.
pub const PLINTH_BOTTOM: i32 = -138;

/// `--castle`'s base: the unbounded cave with a hall of air carved in it and a plinth of solid
/// rock under the hall's floor, so a building stamped on the floor stands on rock and what it
/// carves below the floor is carved from rock, not from the cave's voids.
///
/// The hall is the box [`HALL_LO`]..[`HALL_HI`] with each of its four walls and its ceiling
/// pushed outward by 0 to [`HALL_NOISE`] voxels of value noise (one 2D field per face, sampled
/// on a 32-voxel lattice); the floor is flat. The plinth is solid [`ROCK`] for `z` in
/// [`PLINTH_BOTTOM`]..`HALL_LO.z` under the hall's footprint grown by [`HALL_NOISE`], and wins
/// over the cave's spawn pocket. Everywhere else the cave's own voxels. Pure integer, like
/// [`CaveSource`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HallSource {
    pub cave: CaveSource,
}

impl HallSource {
    /// The hall in the unbounded cave of `seed`.
    pub fn new(seed: u64) -> Self {
        Self {
            cave: CaveSource::unbounded(seed),
        }
    }

    /// How far face `face` (0 and 1: low and high x; 2 and 3: low and high y; 4: the
    /// ceiling) is pushed outward at the face's coordinates `(a, b)`: 0 to [`HALL_NOISE`].
    fn push(&self, face: i32, a: i32, b: i32) -> i32 {
        let n = ValueNoise::new(self.cave.seed ^ 0x4A11_0000_0000_0000, 5);
        let s = n.sample([a, b, face * 4096]);
        (((s + 32768) * i64::from(HALL_NOISE + 1)) >> 16) as i32
    }

    /// Whether voxel `p` is the hall's air.
    pub fn in_hall(&self, p: [i32; 3]) -> bool {
        let [x, y, z] = p;
        let outer = |k: usize, c: i32| c >= HALL_LO[k] - HALL_NOISE && c < HALL_HI[k] + HALL_NOISE;
        if z < HALL_LO[2] || !outer(0, x) || !outer(1, y) || !outer(2, z) {
            return false;
        }
        x >= HALL_LO[0] - self.push(0, y, z)
            && x < HALL_HI[0] + self.push(1, y, z)
            && y >= HALL_LO[1] - self.push(2, x, z)
            && y < HALL_HI[1] + self.push(3, x, z)
            && z < HALL_HI[2] + self.push(4, x, y)
    }

    fn in_plinth(&self, p: [i32; 3]) -> bool {
        (PLINTH_BOTTOM..HALL_LO[2]).contains(&p[2])
            && (0..2).all(|k| p[k] >= HALL_LO[k] - HALL_NOISE && p[k] < HALL_HI[k] + HALL_NOISE)
    }

    /// The final material of voxel `p`.
    pub fn material(&self, p: [i32; 3]) -> MaterialId {
        if self.in_plinth(p) {
            ROCK
        } else if self.in_hall(p) {
            AIR
        } else {
            self.cave.material(p)
        }
    }

    /// The brick with minimum voxel `b`: air inside the hall's noiseless box, rock inside the
    /// plinth, the cave's brick clear of both, else evaluated per voxel.
    pub fn brick(&self, b: [i32; 3]) -> Brick {
        let hi = b.map(|v| v + BRICK_EDGE - 1);
        let within = |lo: [i32; 3], top: [i32; 3]| (0..3).all(|k| b[k] >= lo[k] && hi[k] < top[k]);
        let g = HALL_NOISE;
        if within(HALL_LO, HALL_HI) {
            return Brick::Uniform(AIR);
        }
        let foot_lo = [HALL_LO[0] - g, HALL_LO[1] - g, PLINTH_BOTTOM];
        let foot_hi = [HALL_HI[0] + g, HALL_HI[1] + g, HALL_LO[2]];
        if within(foot_lo, foot_hi) {
            return Brick::Uniform(ROCK);
        }
        let outer_hi = [HALL_HI[0] + g, HALL_HI[1] + g, HALL_HI[2] + g];
        let clear = (0..3).any(|k| hi[k] < foot_lo[k] || b[k] >= outer_hi[k]);
        if clear {
            return self.cave.brick(b);
        }
        Brick::from_fn(|l| self.material([0, 1, 2].map(|k| b[k] + l[k])))
    }
}

impl WorldSource for HallSource {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            let b = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE);
            out.set_brick(slot, self.brick(b));
        }
    }
}

/// Voxels of rock around a [`RoomSource`]'s air: 2 m at 10 cm.
pub const ROOM_ROCK: i32 = 20;

/// `--room`: a closed cube of air `edge` voxels on a side inside [`ROOM_ROCK`] voxels of
/// [`ROCK`] on every side, the extent the smallest whole number of chunks holding both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoomSource {
    pub edge: i32,
    pub extent: ChunkExtent,
}

impl RoomSource {
    pub fn new(edge: i32) -> Self {
        let edge = edge.max(1);
        let chunks = (edge + 2 * ROOM_ROCK + CHUNK_EDGE - 1) / CHUNK_EDGE;
        Self {
            edge,
            extent: ChunkExtent::new([0, 0, 0], [chunks; 3]),
        }
    }

    /// The air's centre voxel.
    pub fn spawn(&self) -> VoxelCoord {
        VoxelCoord([ROOM_ROCK + self.edge / 2; 3])
    }

    /// Whether voxel coordinate `c` lies inside the air on one axis.
    fn inside(&self, c: i32) -> bool {
        (ROOM_ROCK..ROOM_ROCK + self.edge).contains(&c)
    }

    pub fn material(&self, p: [i32; 3]) -> MaterialId {
        if p.iter().all(|&c| self.inside(c)) {
            AIR
        } else {
            ROCK
        }
    }
}

impl WorldSource for RoomSource {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            let lo = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE);
            let hi = lo.map(|v| v + BRICK_EDGE - 1);
            // A brick wholly inside the air on every axis is air; one wholly outside on any
            // axis is rock.
            let brick = if (0..3).all(|k| self.inside(lo[k]) && self.inside(hi[k])) {
                Brick::Uniform(AIR)
            } else if (0..3).any(|k| hi[k] < ROOM_ROCK || lo[k] >= ROOM_ROCK + self.edge) {
                Brick::Uniform(ROCK)
            } else {
                Brick::from_fn(|l| self.material([0, 1, 2].map(|k| lo[k] + l[k])))
            };
            out.set_brick(slot, brick);
        }
    }
}

impl WorldSource for CaveSource {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            let b = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE);
            out.set_brick(slot, self.brick(b));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cave(seed: u64) -> CaveSource {
        CaveSource::new(seed, ChunkExtent::new([0, 0, 0], [4, 4, 4]))
    }

    /// The early-out never disagrees with evaluating every voxel.
    #[test]
    fn corner_early_out_matches_per_voxel_evaluation() {
        let c = cave(0x5EED_0001);
        let mut uniform = 0;
        for bz in 0..12 {
            for by in 0..12 {
                for bx in 0..12 {
                    let b = [bx, by, bz].map(|v| 8 + v * BRICK_EDGE);
                    let fast = c.brick(b);
                    let slow = Brick::from_fn(|l| c.material([0, 1, 2].map(|k| b[k] + l[k])));
                    assert_eq!(fast, slow, "brick at {b:?}");
                    uniform += i32::from(!fast.is_full());
                }
            }
        }
        // Most of the sample is uniform, so the early-out is doing work.
        assert!(uniform > 12 * 12 * 12 / 2, "{uniform}");
    }

    #[test]
    fn the_unbounded_cave_has_no_shell_and_a_fixed_pocket() {
        let bounded = cave(1);
        let open = CaveSource::unbounded(1);
        assert_eq!(open.spawn(), UNBOUNDED_SPAWN);
        assert_eq!(open.material([32, 32, 32]), AIR);
        assert_eq!(open.material([32 + 30, 32, 32]), AIR);
        // Where the bounded cave has its shell, the unbounded one has rock, ore or air.
        for p in [[0, 100, 100], [255, 100, 100], [-5000, 77, 1 << 20]] {
            assert_ne!(open.material(p), SHELL, "{p:?}");
        }
        assert_eq!(bounded.material([0, 100, 100]), SHELL);
        // Away from both pockets and the shell, both caves are the same noise.
        for p in [[10, 200, 40], [200, 20, 220], [100, 100, 10]] {
            assert_eq!(open.material(p), bounded.material(p), "{p:?}");
        }
        // The early-out agrees with per-voxel evaluation without a shell too.
        for b in [[-64, 0, 0], [1024, -512, 64], [24, 24, 24]] {
            let fast = open.brick(b);
            let slow = Brick::from_fn(|l| open.material([0, 1, 2].map(|k| b[k] + l[k])));
            assert_eq!(fast, slow, "brick at {b:?}");
        }
    }

    /// `--vault` puts the room 100 voxels along +x from the spawn and changes nothing else.
    #[test]
    fn the_vault_sits_ahead_of_the_spawn_inside_the_extent() {
        let c = cave(1);
        assert_eq!(c.vault_origin(), VoxelCoord([228, 128, 128]));
        let v = c.with_vault();
        // Chunk [3, 2, 2] holds the whole room (x 228..=243, y and z 128..=143).
        let coord = ChunkCoord([3, 2, 2]);
        let (mut plain, mut vaulted) = (Chunk::default(), Chunk::default());
        c.chunk(coord, &mut plain);
        v.chunk(coord, &mut vaulted);
        let at = |ch: &Chunk, p: [i32; 3]| {
            let p = VoxelCoord(p);
            ch.brick(p.brick_slot()).get(p.index_in_brick())
        };
        assert_eq!(at(&vaulted, [228, 128, 128]), SHELL);
        assert_eq!(at(&vaulted, [243, 143, 143]), SHELL);
        assert_eq!(at(&vaulted, [235, 128, 135]), SHELL);
        assert_eq!(at(&vaulted, [235, 135, 135]), AIR);
        let mut differ = 0;
        for z in 128..192 {
            for y in 128..192 {
                for x in 192..256 {
                    let p = [x, y, z];
                    let inside =
                        (0..3).all(|k| p[k] >= [228, 128, 128][k] && p[k] <= [243, 143, 143][k]);
                    if inside {
                        differ += i32::from(at(&plain, p) != at(&vaulted, p));
                    } else {
                        assert_eq!(at(&plain, p), at(&vaulted, p), "{p:?}");
                    }
                }
            }
        }
        assert!(differ > 0);
        // Other chunks are the plain cave's.
        let (mut a, mut b) = (Chunk::default(), Chunk::default());
        c.chunk(ChunkCoord([2, 2, 2]), &mut a);
        v.chunk(ChunkCoord([2, 2, 2]), &mut b);
        assert_eq!(a, b);

        // On a small extent the room is clamped inside it: x at most 128 - 16.
        let small = CaveSource::new(1, ChunkExtent::new([0, 0, 0], [2, 2, 2]));
        assert_eq!(small.vault_origin(), VoxelCoord([112, 64, 64]));
        // Unbounded: no clamp.
        assert_eq!(
            CaveSource::unbounded(1).vault_origin(),
            VoxelCoord([132, 32, 32])
        );
    }

    /// The hall is air inside its noiseless box and rock on its plinth, its walls move
    /// outward only, and the brick early-outs agree with the voxels around every face.
    #[test]
    fn the_hall_is_air_over_a_rock_plinth_and_its_bricks_match_its_voxels() {
        let h = HallSource::new(1);
        assert_eq!(h.material([32, 32, 32]), AIR, "the spawn");
        assert_eq!(h.material([32, 32, 0]), ROCK, "under the floor");
        assert_eq!(
            h.material([32, 32, 11]),
            ROCK,
            "the pocket's lower half stays rock"
        );
        assert_eq!(h.material([-18, -368, 12]), AIR);
        assert_eq!(h.material([781, 431, 491]), AIR);
        assert_eq!(h.material([400, 32, PLINTH_BOTTOM]), ROCK);
        // Past the noise on every side the cave's own voxels.
        for p in [
            [-35, 32, 100],
            [798, 32, 100],
            [400, -385, 100],
            [400, 448, 100],
            [400, 32, 508],
        ] {
            assert_eq!(h.material(p), h.cave.material(p), "{p:?}");
        }
        // The noise reaches past the box somewhere on each face.
        let pushed = |f: i32| (0..800).step_by(7).any(|a| h.push(f, a - 300, 100) > 0);
        assert!((0..5).all(pushed));
        for b in [
            [-24, 24, 24],
            [-40, -376, 8],
            [776, 424, 488],
            [400, 32, 488],
            [400, 440, 200],
            [-40, 32, -144],
            [0, 0, 0],
            [800, 0, 100],
        ] {
            let fast = h.brick(b);
            let slow = Brick::from_fn(|l| h.material([0, 1, 2].map(|k| b[k] + l[k])));
            assert_eq!(fast, slow, "brick at {b:?}");
        }
    }

    #[test]
    fn a_room_is_a_closed_cube_of_air_in_two_metres_of_rock() {
        let r = RoomSource::new(120);
        // 120 + 40 voxels fit in 3 chunks.
        assert_eq!(r.extent, ChunkExtent::new([0, 0, 0], [3, 3, 3]));
        assert_eq!(r.spawn(), VoxelCoord([80, 80, 80]));
        assert_eq!(r.material([20, 20, 20]), AIR);
        assert_eq!(r.material([139, 139, 139]), AIR);
        assert_eq!(r.material([19, 80, 80]), ROCK);
        assert_eq!(r.material([140, 80, 80]), ROCK);
        // The brick early-outs agree with the voxels.
        for c in [[0, 0, 0], [0, 1, 1], [1, 1, 1], [2, 2, 2], [1, 2, 0]] {
            let coord = ChunkCoord(c);
            let mut chunk = Chunk::default();
            r.chunk(coord, &mut chunk);
            let base = coord.min_voxel().0;
            for slot in 0..CHUNK_BRICKS {
                let b = brick_of_slot(slot);
                let lo = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE);
                let slow = Brick::from_fn(|l| r.material([0, 1, 2].map(|k| lo[k] + l[k])));
                assert_eq!(*chunk.brick(slot), slow, "brick {lo:?}");
            }
        }
    }

    #[test]
    fn shell_is_indestructible_and_the_pocket_is_air() {
        let c = cave(1);
        assert!(crate::cave_materials().is_indestructible(SHELL));
        assert_eq!(c.material([0, 100, 100]), SHELL);
        assert_eq!(c.material([255, 100, 100]), SHELL);
        assert_eq!(c.material([128, 128, 128]), AIR);
        assert_eq!(c.material([128 + 30, 128, 128]), AIR);
        assert_eq!(c.spawn(), VoxelCoord([128, 128, 128]));
    }
}
