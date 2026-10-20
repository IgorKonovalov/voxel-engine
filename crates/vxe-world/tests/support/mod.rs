//! Shared fixtures for the integration tests.
#![allow(dead_code, reason = "shared across test binaries; each uses a subset")]

use vxe_world::*;

/// Rows 0 (air), 1..=5 (plain), with 4 flagged indestructible.
pub fn test_table() -> MaterialTable {
    let mut t = MaterialTable::new();
    for i in 1..=5u16 {
        let mut m = Material::named(format!("m{i}"));
        if i == 4 {
            m.flags = MaterialFlags::INDESTRUCTIBLE;
        }
        assert_eq!(t.push(m), i);
    }
    t
}

/// Chunks `[-2, 2)³`.
pub fn test_extent() -> ChunkExtent {
    ChunkExtent::new([-2, -2, -2], [2, 2, 2])
}

/// `UniformSource(3)` over chunks `[-2, 2)³`.
pub fn solid3_world() -> World {
    World::generate(test_extent(), &UniformSource(3), &test_table())
}

/// A source evaluated per voxel from world voxel coordinates; bricks collapse when uniform.
pub struct FnSource<F>(pub F);

impl<F: Fn(VoxelCoord) -> MaterialId> WorldSource for FnSource<F> {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            out.set_brick(
                slot,
                Brick::from_fn(|l| {
                    (self.0)(VoxelCoord(
                        [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE + l[k]),
                    ))
                }),
            );
        }
    }
}

/// The edit bench's cubes world: `material = 1` where
/// `((x >> 4) + (y >> 4) + (z >> 4)).rem_euclid(3) != 0`, else air, over chunks `[-2, 2)³`.
/// The value is constant over each 16³ cube and the cubes align with the 8³ bricks, so every
/// brick is generated `Uniform`.
pub fn cubes_world() -> World {
    World::generate(test_extent(), &CubesSource, &test_table())
}

/// The source of [`cubes_world`], evaluated once per brick.
pub struct CubesSource;

impl WorldSource for CubesSource {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            let v = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE);
            let m = if ((v[0] >> 4) + (v[1] >> 4) + (v[2] >> 4)).rem_euclid(3) != 0 {
                1
            } else {
                AIR
            };
            out.set_brick(slot, Brick::Uniform(m));
        }
    }
}

/// A per-voxel closure standing in for the fly-through's cave (which `vxe-world` cannot depend
/// on): each brick is air, rock (1) or mixed by a hash of its brick coordinate, and a mixed
/// brick draws each voxel from air, 1 and 2 by a hash of the voxel. Chunks hold `Uniform` and
/// `Full` bricks, and every value is a pure integer function of the voxel coordinate. Cheap
/// enough per voxel for unoptimized test builds, unlike the cave's three noise octaves.
pub fn cave_like_source() -> FnSource<impl Fn(VoxelCoord) -> MaterialId + Send + Sync + 'static> {
    FnSource(
        |v: VoxelCoord| match hash::hash3(0x5EED_CAFE, v.0.map(|c| c >> 3)) % 16 {
            0..4 => AIR,
            4..12 => 1,
            _ => (hash::hash3(0x0EE0_0EE0, v.0) % 3) as MaterialId,
        },
    )
}

/// FNV-1a 64 over the chunks `coords` of `world` (each must be resident), in the given
/// order, laid out as [`world_hash`] lays out every chunk.
pub fn chunks_hash(world: &World, coords: &[ChunkCoord]) -> u64 {
    let mut h = Fnv1a::default();
    for c in coords {
        for v in c.0 {
            h.write(&v.to_le_bytes());
        }
        let chunk = world
            .store()
            .get(*c)
            .unwrap_or_else(|| panic!("{c:?} resident"));
        let chunk = chunk.read().unwrap();
        for b in chunk.bricks() {
            for i in 0..BRICK_VOXELS {
                h.write(&b.get(i).to_le_bytes());
            }
        }
    }
    h.0
}

/// Chunk coordinates within squared distance `r2` of `centre`, ascending.
pub fn ball(centre: [i32; 3], r2: i32) -> Vec<ChunkCoord> {
    let r = (1..).find(|r| r * r > r2).unwrap();
    let mut v = Vec::new();
    for x in -r..=r {
        for y in -r..=r {
            for z in -r..=r {
                if x * x + y * y + z * z <= r2 {
                    v.push(ChunkCoord([centre[0] + x, centre[1] + y, centre[2] + z]));
                }
            }
        }
    }
    v.sort_unstable();
    v
}

/// FNV-1a 64 over the resident voxels: chunks ascending (coordinate as three little-endian
/// `i32`), bricks in slot order, voxels x-fastest as little-endian `u16`. A `Uniform` brick
/// hashes as its 512 expanded values, so representation never moves the hash.
pub fn world_hash(world: &World) -> u64 {
    let mut h = Fnv1a::default();
    for c in world.store().coords() {
        for v in c.0 {
            h.write(&v.to_le_bytes());
        }
        let chunk = world.store().get(c).unwrap();
        let chunk = chunk.read().unwrap();
        for b in chunk.bricks() {
            for i in 0..BRICK_VOXELS {
                h.write(&b.get(i).to_le_bytes());
            }
        }
    }
    h.0
}

/// FNV-1a, 64-bit.
pub struct Fnv1a(pub u64);

impl Default for Fnv1a {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv1a {
    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// SplitMix64: a seeded, platform-independent stream for test inputs.
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `lo..=hi`.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() % (hi - lo + 1) as u64) as i64
    }
}

/// The solid material-3 world after the radius-10 carve at the origin.
pub fn carved_world() -> World {
    let world = solid3_world();
    world
        .apply(&EditOp::sphere(
            centre_of_voxel([0, 0, 0]),
            10 * FIXED_ONE,
            Mode::Carve,
        ))
        .unwrap();
    world
}

/// What [`reference_apply`] says `World::apply` must do with one op.
#[derive(Debug)]
pub struct Expected {
    /// The result or error `World::apply` must return.
    pub result: Result<EditResult, EditError>,
    /// The op's inclusive voxel bounds; `None` for an empty shape.
    pub bounds: Option<([i64; 3], [i64; 3])>,
    /// The material of every voxel in `bounds` after the op, x fastest, then y, then z;
    /// `None` for a voxel in a non-resident chunk.
    pub after: Vec<Option<MaterialId>>,
}

/// The material a voxel of `old` becomes under `mode`, or `None` when it is unchanged, read
/// straight from the material table.
fn reference_target(mode: Mode, old: MaterialId, table: &MaterialTable) -> Option<MaterialId> {
    let new = match mode {
        Mode::Carve => {
            let indestructible = table
                .get(old)
                .is_some_and(|m| m.flags.contains(MaterialFlags::INDESTRUCTIBLE));
            (old != AIR && !indestructible).then_some(AIR)
        }
        Mode::Fill(m) => (old == AIR).then_some(m),
        Mode::Replace { from, to } => (old == from).then_some(to),
    }?;
    (new != old).then_some(new)
}

/// A brute-force model of `World::apply`: every voxel centre in the shape's voxel bounds is
/// tested with `Shape::contains`, the mode is applied by a direct `MaterialTable` lookup, and
/// the op fails on the least non-resident chunk holding an inside centre. Reads `world` one
/// voxel at a time and never calls `World::apply`.
pub fn reference_apply(world: &World, op: &EditOp) -> Expected {
    let Some((lo, hi)) = op.shape.voxel_bounds() else {
        return Expected {
            result: Ok(EditResult::default()),
            bounds: None,
            after: Vec::new(),
        };
    };
    let table = world.materials();
    let mut missing: Option<ChunkCoord> = None;
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut counts = std::collections::BTreeMap::<MaterialId, u32>::new();
    let mut touched = std::collections::BTreeSet::<ChunkCoord>::new();
    let mut aabb: Option<([i64; 3], [i64; 3])> = None;
    for z in lo[2]..=hi[2] {
        for y in lo[1]..=hi[1] {
            for x in lo[0]..=hi[0] {
                let v = [x, y, z];
                let vc = VoxelCoord(v.map(|c| i32::try_from(c).expect("voxel fits i32")));
                let old = world.material_at(vc);
                before.push(old);
                let inside = op.shape.contains(v.map(|c| c * FIXED_ONE + FIXED_HALF));
                let new = match (inside, old) {
                    (false, _) => old,
                    (true, None) => {
                        let c = vc.chunk();
                        missing = Some(missing.map_or(c, |m| m.min(c)));
                        None
                    }
                    (true, Some(o)) => match reference_target(op.mode, o, table) {
                        None => Some(o),
                        Some(n) => {
                            *counts.entry(o).or_default() += 1;
                            touched.insert(vc.chunk());
                            aabb = Some(match aabb {
                                None => (v, v),
                                Some((a, b)) => (
                                    [0, 1, 2].map(|k| a[k].min(v[k])),
                                    [0, 1, 2].map(|k| b[k].max(v[k])),
                                ),
                            });
                            Some(n)
                        }
                    },
                };
                after.push(new);
            }
        }
    }
    let to_voxel = |v: [i64; 3]| VoxelCoord(v.map(|c| i32::try_from(c).expect("fits i32")));
    let (result, after) = match missing {
        Some(c) => (Err(EditError::NotResident(c)), before),
        None => (
            Ok(EditResult {
                changed_from: counts.into_iter().collect(),
                touched: touched.into_iter().collect(),
                aabb: aabb.map(|(a, b)| (to_voxel(a), to_voxel(b))),
            }),
            after,
        ),
    };
    Expected {
        result,
        bounds: Some((lo, hi)),
        after,
    }
}

/// The material of every voxel in `lo..=hi`, x fastest, as [`Expected::after`] lays it out.
pub fn materials_in(world: &World, lo: [i64; 3], hi: [i64; 3]) -> Vec<Option<MaterialId>> {
    let mut out = Vec::new();
    for z in lo[2]..=hi[2] {
        for y in lo[1]..=hi[1] {
            for x in lo[0]..=hi[0] {
                out.push(world.material_at(VoxelCoord(
                    [x, y, z].map(|c| i32::try_from(c).expect("voxel fits i32")),
                )));
            }
        }
    }
    out
}
