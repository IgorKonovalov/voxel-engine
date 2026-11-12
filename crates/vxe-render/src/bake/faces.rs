//! The face classes and masks of a chunk, and the lists of the faces a bake covers. Pure CPU.

use super::*;

/// [`face_classes`]' class of air.
pub(crate) const CLASS_AIR: u8 = 0;

/// [`face_classes`]' class of a material that stops light.
pub(crate) const CLASS_OPAQUE: u8 = 1;

/// [`face_classes`]' class of a translucent material: air to the walk and the shadow rays, a
/// filter to the view.
pub(crate) const CLASS_TRANSLUCENT: u8 = 2;

/// Each material id's class: [`CLASS_AIR`], [`CLASS_OPAQUE`] or [`CLASS_TRANSLUCENT`].
pub fn face_classes(materials: &MaterialTable) -> Vec<u8> {
    let mut t = vec![CLASS_AIR; 1 << 16];
    for (id, m) in materials.rows().iter().enumerate() {
        if id != usize::from(AIR) {
            t[id] = if m.is_translucent() {
                CLASS_TRANSLUCENT
            } else {
                CLASS_OPAQUE
            };
        }
    }
    t
}

/// A chunk's exposed faces, by surface brick in slot order, each brick's keys ascending. A key is
/// `voxel in the brick << 3 | face`, face `2 axis + 1` toward `+axis`. `glass` holds the
/// `(brick slot, key)` of every translucent voxel's face among them, ascending.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChunkFaces {
    pub bricks: Vec<(u16, Vec<u16>)>,
    pub glass: Vec<(u16, u16)>,
}

impl ChunkFaces {
    /// Faces over the chunk's bricks.
    pub(crate) fn faces(&self) -> usize {
        self.bricks.iter().map(|(_, k)| k.len()).sum()
    }
}

pub(super) const NO_VOXEL: [u64; 8] = [0; 8];

pub(super) const ALL_VOXELS: [u64; 8] = [u64::MAX; 8];

/// A brick's opaque voxels and its solid ones (opaque or translucent): bit `x + 8 y` of word `z`.
pub(super) fn class_masks(brick: &Brick, classes: &[u8]) -> Masks {
    match brick {
        Brick::Uniform(m) => uniform_masks(classes[usize::from(*m)]),
        Brick::Full(p) => {
            let mut v = [AIR; BRICK_VOXELS];
            p.decode(&mut v);
            let (mut opaque, mut solid) = (NO_VOXEL, NO_VOXEL);
            for (i, m) in v.iter().enumerate() {
                let c = classes[usize::from(*m)];
                if c == CLASS_OPAQUE {
                    opaque[i / 64] |= 1 << (i % 64);
                }
                if c != CLASS_AIR {
                    solid[i / 64] |= 1 << (i % 64);
                }
            }
            (opaque, solid)
        }
    }
}

/// Whether the voxel beside `l` (in the brick) across `face` is set, in `mask` within the brick or
/// in `side`, the neighbour brick's mask across that face.
pub(super) fn beside_bit(mask: &[u64; 8], side: &[u64; 8], l: [usize; 3], face: usize) -> bool {
    let k = face / 2;
    let mut q = l;
    if face % 2 == 1 {
        if l[k] == 7 {
            q[k] = 0;
            return mask_bit(side, q);
        }
        q[k] += 1;
    } else {
        if l[k] == 0 {
            q[k] = 7;
            return mask_bit(side, q);
        }
        q[k] -= 1;
    }
    mask_bit(mask, q)
}

pub(super) fn mask_bit(mask: &[u64; 8], l: [usize; 3]) -> bool {
    mask[l[2]] >> (l[0] + 8 * l[1]) & 1 != 0
}

/// A brick's opaque and solid masks ([`class_masks`]).
pub(crate) type Masks = ([u64; 8], [u64; 8]);

/// The masks of a uniform brick of class `c`.
pub(super) fn uniform_masks(c: u8) -> Masks {
    match c {
        CLASS_OPAQUE => (ALL_VOXELS, ALL_VOXELS),
        CLASS_TRANSLUCENT => (NO_VOXEL, ALL_VOXELS),
        _ => (NO_VOXEL, NO_VOXEL),
    }
}

/// A uniform brick's class in [`ChunkMasks`]: this marks a brick held by its masks.
pub(super) const MASKED: u8 = u8::MAX;

/// A chunk's brick masks, the listing's decode done ahead of it (on a stream worker, through
/// [`FacePrep`]): a uniform brick by its class, any other by its masks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChunkMasks {
    pub(super) classes: Box<[u8; CHUNK_BRICKS]>,
    pub(super) masked: Vec<(u16, Masks)>,
}

impl ChunkMasks {
    /// `chunk`'s masks under `classes` ([`face_classes`]).
    pub(crate) fn of(chunk: &Chunk, classes: &[u8]) -> Self {
        let mut out = Self {
            classes: Box::new([CLASS_AIR; CHUNK_BRICKS]),
            masked: Vec::new(),
        };
        for (slot, b) in chunk.bricks().iter().enumerate() {
            match b {
                Brick::Uniform(m) => out.classes[slot] = classes[usize::from(*m)],
                Brick::Full(_) => {
                    out.classes[slot] = MASKED;
                    out.masked.push((slot as u16, class_masks(b, classes)));
                }
            }
        }
        out
    }

    /// Brick `slot`'s masks.
    pub(crate) fn get(&self, slot: usize) -> Masks {
        match self.classes[slot] {
            MASKED => {
                let i = self
                    .masked
                    .binary_search_by_key(&(slot as u16), |(s, _)| *s)
                    .expect("a masked brick holds its masks");
                self.masked[i].1
            }
            c => uniform_masks(c),
        }
    }

    /// Bytes held, for the prepared results' cap.
    pub(crate) fn bytes(&self) -> usize {
        CHUNK_BRICKS + self.masked.len() * std::mem::size_of::<(u16, Masks)>()
    }

    /// An upper bound on the brick directions a listing of the chunk finds: six for a masked
    /// brick; for a uniform opaque or translucent brick, its sides toward the chunk's border or
    /// toward a brick that is not uniform opaque (or, for a translucent one, toward one that
    /// is masked or air); none for air.
    pub(crate) fn dirs_bound(&self) -> u64 {
        let mut dirs = 0u64;
        for slot in 0..CHUNK_BRICKS {
            let own = self.classes[slot];
            if own == MASKED {
                dirs += 6;
                continue;
            }
            if own == CLASS_AIR {
                continue;
            }
            let b = vxe_world::brick_of_slot(slot);
            for face in 0..6 {
                let mut n = b;
                n[face / 2] += if face % 2 == 1 { 1 } else { -1 };
                if !(0..8).contains(&n[face / 2]) {
                    dirs += 1;
                    continue;
                }
                let other = self.classes[vxe_world::brick_slot(n)];
                let exposed = match own {
                    CLASS_OPAQUE => other != CLASS_OPAQUE,
                    _ => other == MASKED || other == CLASS_AIR,
                };
                dirs += u64::from(exposed);
            }
        }
        dirs
    }
}

/// The stream workers' hook (ADR-0017, `vxe_world::ChunkPrep`): each full chunk's
/// [`ChunkMasks`] under the material classes it was made with, for [`list_faces_by`].
pub struct FacePrep {
    pub classes: Arc<Vec<u8>>,
}

impl vxe_world::ChunkPrep for FacePrep {
    fn prepare(&self, _coord: ChunkCoord, chunk: &Chunk) -> vxe_world::Prepared {
        let t = stopwatch();
        let masks = ChunkMasks::of(chunk, &self.classes);
        vxe_world::Prepared(Arc::new(PreparedMasks {
            classes: self.classes.clone(),
            masks,
            ns: (seconds_since(t) * 1e9) as u64,
        }))
    }
}

/// What [`FacePrep`] hands back: the masks and the classes they were taken under, and the
/// nanoseconds the worker took over them.
pub(crate) struct PreparedMasks {
    pub classes: Arc<Vec<u8>>,
    pub masks: ChunkMasks,
    pub ns: u64,
}

/// `chunk`'s faces, across brick and chunk borders: an opaque voxel's toward a voxel that is not
/// opaque, and a translucent voxel's toward air (`classes`, [`face_classes`]). The neighbours are
/// in face order (`-x, +x, -y, +y, -z, +z`); across a neighbour that is `None` no ray arrives (an
/// absent chunk is rock to the walk), so no face is listed there.
pub(crate) fn list_chunk_faces(
    chunk: &Chunk,
    neighbours: &[Option<&Chunk>; 6],
    classes: &[u8],
) -> ChunkFaces {
    let masks: Vec<Masks> = chunk
        .bricks()
        .iter()
        .map(|b| class_masks(b, classes))
        .collect();
    list_faces_by(&|slot| masks[slot], &|face, slot| {
        neighbours[face].map(|c| class_masks(c.brick(slot), classes))
    })
}

/// A chunk's faces as [`list_chunk_faces`] lists them, from its bricks' masks (`own`, by slot)
/// and its neighbours' (`beside`, by face and slot; `None` where that neighbour is absent).
pub(crate) fn list_faces_by(
    own: &dyn Fn(usize) -> Masks,
    beside: &dyn Fn(usize, usize) -> Option<Masks>,
) -> ChunkFaces {
    let masks: Vec<Masks> = (0..CHUNK_BRICKS).map(own).collect();
    let mut out = ChunkFaces::default();
    for slot in 0..CHUNK_BRICKS {
        let (opaque, solid) = masks[slot];
        if solid == NO_VOXEL {
            continue;
        }
        let b = brick_of_slot(slot);
        let beside: [Masks; 6] = std::array::from_fn(|face| {
            let (k, d) = (face / 2, if face % 2 == 1 { 1 } else { -1 });
            let mut n = b;
            n[k] += d;
            if (0..8).contains(&n[k]) {
                return masks[brick_slot(n)];
            }
            n[k] = n[k].rem_euclid(8);
            beside(face, brick_slot(n)).unwrap_or((ALL_VOXELS, ALL_VOXELS))
        });
        if opaque == ALL_VOXELS && beside.iter().all(|m| m.0 == ALL_VOXELS) {
            continue;
        }
        let mut keys = Vec::new();
        for i in 0..BRICK_VOXELS {
            let l = [i % 8, i / 8 % 8, i / 64];
            // An opaque voxel's face is closed by an opaque one; a translucent voxel's by any
            // solid one, so a pane lists its two sides toward air and nothing inside it.
            let (own, glass) = if mask_bit(&opaque, l) {
                (&opaque, false)
            } else if mask_bit(&solid, l) {
                (&solid, true)
            } else {
                continue;
            };
            for (face, side) in beside.iter().enumerate() {
                let side = if glass { &side.1 } else { &side.0 };
                if !beside_bit(own, side, l, face) {
                    let key = (i as u16) << 3 | face as u16;
                    keys.push(key);
                    if glass {
                        out.glass.push((slot as u16, key));
                    }
                }
            }
        }
        if !keys.is_empty() {
            out.bricks.push((slot as u16, keys));
        }
    }
    out
}

/// The faces of `coord` in `world`, whose absent neighbours list nothing across.
pub fn list_world_chunk(world: &World, coord: ChunkCoord, classes: &[u8]) -> Option<ChunkFaces> {
    let store = world.store();
    let chunk = store.get(coord)?;
    let neighbour = |face: usize| {
        let mut c = coord.0;
        c[face / 2] += if face % 2 == 1 { 1 } else { -1 };
        store.get(ChunkCoord(c))
    };
    let held: [_; 6] = std::array::from_fn(neighbour);
    let locks: Vec<_> = held
        .iter()
        .map(|h| {
            h.as_ref()
                .map(|c| c.read().unwrap_or_else(std::sync::PoisonError::into_inner))
        })
        .collect();
    let refs: [Option<&Chunk>; 6] = std::array::from_fn(|f| locks[f].as_deref());
    let c = chunk
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Some(list_chunk_faces(&c, &refs, classes))
}

/// A test of a voxel and its face.
pub(crate) type FaceFilter = Arc<dyn Fn([i32; 3], u8) -> bool + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;
    use vxe_world::MaterialId;

    /// An xorshift, seeded.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// A chunk whose bricks mix air, glass, rock and uniform runs by the seed. Material 1 and 2
    /// are opaque, 3 is translucent.
    fn mixed_chunk(seed: u64) -> Chunk {
        let mut rng = Rng(seed | 1);
        let mut c = Chunk::uniform(AIR);
        for slot in 0..CHUNK_BRICKS {
            let brick = match rng.next() % 5 {
                0 => Brick::Uniform(AIR),
                1 => Brick::Uniform(1),
                2 => Brick::Uniform(3),
                _ => {
                    let density = rng.next() % 4;
                    Brick::from_fn(|_| match (rng.next() % 4).max(density) {
                        0 => AIR,
                        1 => 1 as MaterialId,
                        2 => 2,
                        _ => {
                            if rng.next().is_multiple_of(2) {
                                3
                            } else {
                                AIR
                            }
                        }
                    })
                }
            };
            c.set_brick(slot, brick);
        }
        c
    }

    fn material(chunk: &Chunk, l: [i32; 3]) -> MaterialId {
        let b = l.map(|v| v / 8);
        let i = l.map(|v| v % 8);
        chunk
            .brick(brick_slot(b))
            .get((i[0] + 8 * (i[1] + 8 * i[2])) as usize)
    }

    #[test]
    fn every_exposed_face_has_one_key() {
        let mut classes = vec![CLASS_AIR; 1 << 16];
        classes[1] = CLASS_OPAQUE;
        classes[2] = CLASS_OPAQUE;
        classes[3] = CLASS_TRANSLUCENT;
        let chunk = mixed_chunk(0x9E37_79B9_7F4A_7C15);
        let beside: [Chunk; 6] = std::array::from_fn(|f| mixed_chunk(0x1234_5678 + 97 * f as u64));
        let refs: [Option<&Chunk>; 6] = std::array::from_fn(|f| Some(&beside[f]));
        let faces = list_chunk_faces(&chunk, &refs, &classes);
        let mut listed: Vec<(usize, u16)> = Vec::new();
        for (slot, keys) in &faces.bricks {
            assert!(!keys.is_empty(), "brick {slot} listed with no face");
            assert!(
                keys.windows(2).all(|w| w[0] < w[1]),
                "brick {slot} unsorted"
            );
            listed.extend(keys.iter().map(|&k| (usize::from(*slot), k)));
        }
        let mut scan: Vec<(usize, u16)> = Vec::new();
        let mut glass: Vec<(u16, u16)> = Vec::new();
        for z in 0..64 {
            for y in 0..64 {
                for x in 0..64 {
                    let l = [x, y, z];
                    // An opaque voxel's face toward anything not opaque; a translucent voxel's
                    // toward air.
                    let class = classes[usize::from(material(&chunk, l))];
                    if class == CLASS_AIR {
                        continue;
                    }
                    let inner = l.map(|v| v % 8);
                    let key = ((inner[0] + 8 * (inner[1] + 8 * inner[2])) << 3) as u16;
                    for (face, side) in beside.iter().enumerate() {
                        let k = face / 2;
                        let mut n = l;
                        n[k] += if face % 2 == 1 { 1 } else { -1 };
                        let m = if (0..64).contains(&n[k]) {
                            material(&chunk, n)
                        } else {
                            n[k] = n[k].rem_euclid(64);
                            material(side, n)
                        };
                        let open = match class {
                            CLASS_OPAQUE => classes[usize::from(m)] != CLASS_OPAQUE,
                            _ => classes[usize::from(m)] == CLASS_AIR,
                        };
                        if open {
                            let at = (brick_slot(l.map(|v| v / 8)), key | face as u16);
                            scan.push(at);
                            if class == CLASS_TRANSLUCENT {
                                glass.push((at.0 as u16, at.1));
                            }
                        }
                    }
                }
            }
        }
        scan.sort_unstable();
        listed.sort_unstable();
        assert!(!scan.is_empty());
        assert_eq!(
            listed.len(),
            scan.len(),
            "listed {} found {}",
            listed.len(),
            scan.len()
        );
        assert_eq!(listed, scan);
        glass.sort_unstable();
        assert!(!glass.is_empty());
        assert_eq!(faces.glass, glass);
        // The stream workers' path lists the same faces from prepared masks: the chunk's and
        // its neighbours' all prepared, and again with half the neighbours decoded inline.
        let own = ChunkMasks::of(&chunk, &classes);
        let near: [ChunkMasks; 6] = std::array::from_fn(|f| ChunkMasks::of(&beside[f], &classes));
        let prepared = list_faces_by(&|slot| own.get(slot), &|face, slot| {
            Some(near[face].get(slot))
        });
        assert_eq!(prepared, faces, "all prepared");
        let mixed = list_faces_by(&|slot| own.get(slot), &|face, slot| {
            Some(if face % 2 == 0 {
                near[face].get(slot)
            } else {
                class_masks(beside[face].brick(slot), &classes)
            })
        });
        assert_eq!(mixed, faces, "half inline");
        // An absent neighbour lists nothing across, by either path.
        let absent: [Option<&Chunk>; 6] = std::array::from_fn(|f| (f != 3).then(|| &beside[f]));
        let inline = list_chunk_faces(&chunk, &absent, &classes);
        let by_masks = list_faces_by(&|slot| own.get(slot), &|face, slot| {
            (face != 3).then(|| near[face].get(slot))
        });
        assert_eq!(by_masks, inline, "one neighbour absent");
        assert_ne!(inline, faces, "the absent neighbour changes the list");
    }
}
