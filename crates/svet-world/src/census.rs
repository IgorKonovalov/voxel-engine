//! A census of materials per brick (ADR-0012): how many `Full` bricks hold one, two, three to
//! four, five to sixteen, 17 to 256 or more non-air materials, and the payload bytes each
//! packed layout would take for them.
//!
//! Three payload totals are kept for the `Full` bricks: today's 512 raw ids (1024 B each); the
//! class ladder of `packed.rs` (72, 136, 200, 352 B for 1, 2, 3 to 4 and 5 to 16 materials,
//! 1024 B above); and exact planes (an 8-byte header, the 64-byte mask, 64 bytes per
//! `ceil(log2 n)` planes and 2 bytes per palette entry; 1096 B above 256 materials, sixteen
//! planes of raw ids). Slots are counted separately, 16 B each, `Uniform` or not.

use crate::brick::Brick;
use crate::material::{AIR, MaterialFlags, MaterialId, MaterialTable};
use crate::store::{World, read};

/// The non-air voxels of `brick`, one `u64` per z-layer (bit `x + 8 y`): a `Full` brick's
/// mask, all set for a uniform solid brick, clear for uniform air.
pub fn solid_mask(brick: &Brick) -> [u64; 8] {
    match brick {
        Brick::Uniform(m) => [if *m == AIR { 0 } else { u64::MAX }; 8],
        Brick::Full(p) => p.mask(),
    }
}

/// The *chunk-local surface* of a brick whose non-air voxels are `solid` (layer-major as
/// [`solid_mask`]): its non-air voxels with at least one of the six face neighbours air, the
/// neighbours across the brick's faces read from `beside[k][d]`, the solid mask of the brick
/// next to it on axis `k` toward minus (`d` 0) or plus (`d` 1). `None` there (the brick's face
/// is a face of its chunk) counts as air, so every voxel on that face is surface: no chunk
/// past the brick's own is read.
pub fn surface_mask(solid: &[u64; 8], beside: &[[Option<[u64; 8]>; 2]; 3]) -> [u64; 8] {
    // Bits of the x = 0 and x = 7 columns, and of the y = 0 and y = 7 rows, of one layer.
    const X0: u64 = 0x0101_0101_0101_0101;
    const X7: u64 = X0 << 7;
    const Y0: u64 = 0xFF;
    const Y7: u64 = Y0 << 56;
    let side = |k: usize, d: usize, z: usize| beside[k][d].map_or(0, |m| m[z]);
    let mut out = [0u64; 8];
    for z in 0..8 {
        let m = solid[z];
        // Each word is set where the neighbour on that side is solid.
        let xm = ((m << 1) & !X0) | ((side(0, 0, z) & X7) >> 7);
        let xp = ((m >> 1) & !X7) | ((side(0, 1, z) & X0) << 7);
        let ym = (m << 8) | ((side(1, 0, z) & Y7) >> 56);
        let yp = (m >> 8) | ((side(1, 1, z) & Y0) << 56);
        let zm = if z > 0 { solid[z - 1] } else { side(2, 0, 7) };
        let zp = if z < 7 { solid[z + 1] } else { side(2, 1, 0) };
        out[z] = m & !(xm & xp & ym & yp & zm & zp);
    }
    out
}

/// Counts over a set of bricks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct BrickCensus {
    pub uniform: u64,
    /// `Full` bricks by their count `n` of non-air materials, in the bins of
    /// [`BrickCensus::BINS`].
    pub full_by_materials: [u64; 6],
    /// Bricks holding at least one voxel of a `TRANSLUCENT` material.
    pub translucent_any: u64,
    /// Bricks with a non-air voxel whose non-air voxels are all `TRANSLUCENT`.
    pub translucent_only: u64,
    /// 1024 B per `Full` brick.
    pub payload_today: u64,
    /// The class ladder's bytes per `Full` brick, by bin.
    pub payload_ladder: u64,
    /// `8 + 64 + 64 ceil(log2 n) + 2 n` per `Full` brick; 1096 above 256 materials.
    pub payload_exact_planes: u64,
    /// Brick slots, `Uniform` and `Full`.
    pub slots: u64,
}

impl BrickCensus {
    /// The bins of `full_by_materials`, by `n`.
    pub const BINS: [&'static str; 6] = ["1", "2", "3-4", "5-16", "17-256", ">256"];
    /// The ladder's payload bytes per `Full` brick in each bin.
    pub const LADDER_BYTES: [u64; 6] = [72, 136, 200, 352, 1024, 1024];
    /// Today's payload bytes per `Full` brick.
    pub const TODAY_BYTES: u64 = 1024;
    /// Bytes per brick slot (`size_of::<Brick>()`).
    pub const SLOT_BYTES: u64 = 16;

    /// The bin of a `Full` brick holding `n >= 1` non-air materials.
    pub const fn bin(n: usize) -> usize {
        match n {
            0 | 1 => 0,
            2 => 1,
            3 | 4 => 2,
            5..=16 => 3,
            17..=256 => 4,
            _ => 5,
        }
    }

    /// Exact-plane payload bytes of a `Full` brick holding `n >= 1` non-air materials.
    pub const fn exact_bytes(n: usize) -> u64 {
        if n > 256 {
            return 1096;
        }
        let planes = (n as u64).next_power_of_two().trailing_zeros() as u64;
        8 + 64 + 64 * planes + 2 * n as u64
    }

    /// `Full` bricks.
    pub fn full(&self) -> u64 {
        self.full_by_materials.iter().sum()
    }

    /// Slot bytes.
    pub fn slot_bytes(&self) -> u64 {
        self.slots * Self::SLOT_BYTES
    }

    /// Adds one brick.
    pub(crate) fn add(&mut self, brick: &Brick, table: &MaterialTable, scratch: &mut Distinct) {
        let translucent = |m: MaterialId| table.flags(m).contains(MaterialFlags::TRANSLUCENT);
        self.slots += 1;
        // Non-air materials, and how many of them are translucent.
        let (solid, glass) = match brick {
            Brick::Uniform(m) => {
                self.uniform += 1;
                let solid = usize::from(*m != AIR);
                (solid, solid * usize::from(translucent(*m)))
            }
            Brick::Full(p) => {
                let mut v = [AIR; crate::coords::BRICK_VOXELS];
                p.decode(&mut v);
                let ids = scratch.of(&v[..]);
                let glass = ids.iter().filter(|&&m| translucent(m)).count();
                let n = ids.len();
                let b = Self::bin(n);
                self.full_by_materials[b] += 1;
                self.payload_today += Self::TODAY_BYTES;
                self.payload_ladder += Self::LADDER_BYTES[b];
                self.payload_exact_planes += Self::exact_bytes(n);
                (n, glass)
            }
        };
        if glass > 0 {
            self.translucent_any += 1;
            if glass == solid {
                self.translucent_only += 1;
            }
        }
    }

    /// Adds `other`'s counts.
    pub fn merge(&mut self, other: &Self) {
        self.uniform += other.uniform;
        for (a, b) in self
            .full_by_materials
            .iter_mut()
            .zip(other.full_by_materials)
        {
            *a += b;
        }
        self.translucent_any += other.translucent_any;
        self.translucent_only += other.translucent_only;
        self.payload_today += other.payload_today;
        self.payload_ladder += other.payload_ladder;
        self.payload_exact_planes += other.payload_exact_planes;
        self.slots += other.slots;
    }

    /// The bin holding the most `Full` bricks (the lower bin on a tie); `None` with none.
    pub fn majority_bin(&self) -> Option<usize> {
        let max = *self.full_by_materials.iter().max()?;
        (max > 0).then(|| {
            self.full_by_materials
                .iter()
                .position(|&c| c == max)
                .unwrap_or(0)
        })
    }
}

fn mib(b: u64) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

/// Two lines: the bins and translucent counts, then the payload totals and the slot bytes.
impl std::fmt::Display for BrickCensus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "uniform {}, full {} by materials",
            self.uniform,
            self.full()
        )?;
        for (name, n) in Self::BINS.iter().zip(self.full_by_materials) {
            write!(f, " [{name}: {n}]")?;
        }
        writeln!(
            f,
            "; translucent any {}, only {}",
            self.translucent_any, self.translucent_only
        )?;
        write!(
            f,
            "payload today {} B ({:.1} MiB), ladder {} B ({:.1} MiB), exact {} B ({:.1} MiB); \
             slots {} x {} B = {} B ({:.1} MiB)",
            self.payload_today,
            mib(self.payload_today),
            self.payload_ladder,
            mib(self.payload_ladder),
            self.payload_exact_planes,
            mib(self.payload_exact_planes),
            self.slots,
            Self::SLOT_BYTES,
            self.slot_bytes(),
            mib(self.slot_bytes())
        )
    }
}

/// The census of a world: its resident chunks' bricks and its LOD-only chunks' mip-bricks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WorldCensus {
    pub bricks: BrickCensus,
    pub mip_bricks: BrickCensus,
}

impl WorldCensus {
    /// Bricks and mip-bricks together.
    pub fn total(&self) -> BrickCensus {
        let mut t = self.bricks;
        t.merge(&self.mip_bricks);
        t
    }
}

/// The bricks, the mip-bricks, and the projected resident payload plus slots under each
/// layout.
impl std::fmt::Display for WorldCensus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let indent = |s: String| s.replace('\n', "\n                   ");
        writeln!(f, "census bricks:     {}", indent(self.bricks.to_string()))?;
        writeln!(
            f,
            "census mip-bricks: {}",
            indent(self.mip_bricks.to_string())
        )?;
        let t = self.total();
        let s = t.slot_bytes();
        write!(
            f,
            "census resident:   payload + slots: today {} B ({:.1} MiB), ladder {} B \
             ({:.1} MiB), exact {} B ({:.1} MiB)",
            t.payload_today + s,
            mib(t.payload_today + s),
            t.payload_ladder + s,
            mib(t.payload_ladder + s),
            t.payload_exact_planes + s,
            mib(t.payload_exact_planes + s)
        )
    }
}

/// Scratch for listing a brick's distinct non-air ids: a bit per id over the whole `u16`
/// range, cleared after each brick through the list it filled.
pub(crate) struct Distinct {
    seen: Vec<u64>,
    ids: Vec<MaterialId>,
}

impl Default for Distinct {
    fn default() -> Self {
        Self {
            seen: vec![0; (usize::from(MaterialId::MAX) + 1) / 64],
            ids: Vec::with_capacity(64),
        }
    }
}

impl Distinct {
    /// The distinct non-air ids of `voxels`, in first-appearance order.
    pub(crate) fn of(&mut self, voxels: &[MaterialId]) -> &[MaterialId] {
        for &m in &self.ids {
            self.seen[usize::from(m) / 64] = 0;
        }
        self.ids.clear();
        let mut last = AIR;
        for &m in voxels {
            if m == AIR || m == last {
                continue;
            }
            last = m;
            let (w, b) = (usize::from(m) / 64, m % 64);
            if self.seen[w] >> b & 1 == 0 {
                self.seen[w] |= 1 << b;
                self.ids.push(m);
            }
        }
        &self.ids
    }
}

impl World {
    /// The census of every resident chunk's bricks and every LOD-only chunk's mip-bricks,
    /// read one chunk lock at a time.
    pub fn brick_census(&self) -> WorldCensus {
        let mut c = WorldCensus::default();
        let mut scratch = Distinct::default();
        for coord in self.store.coords() {
            let Some(chunk) = self.store.get(coord) else {
                continue;
            };
            let chunk = read(&chunk);
            for b in chunk.bricks() {
                c.bricks.add(b, &self.materials, &mut scratch);
            }
        }
        for coord in self.lod_coords() {
            let Some(mips) = self.lod(coord) else {
                continue;
            };
            for n in 1..=crate::MIP_LEVELS {
                for b in mips.level(n) {
                    c.mip_bricks.add(b, &self.materials, &mut scratch);
                }
            }
        }
        c
    }
}
