//! The column map (ADR-0070's amendment of 2026-11-18): for each column of `COLUMN_CELL` x
//! `COLUMN_CELL` voxels over the window's footprint, the highest opaque voxel's `z` among the
//! resident chunks of the window, or [`COLUMN_NONE`] where the column holds none. The lighting
//! rays read it to decide whether a ray that ended without a hit reached the sky.
//!
//! Each mirrored chunk keeps a *sheet*: per cell of its 64 x 64 footprint, the highest opaque
//! voxel's local `z` (0..64), or `SHEET_NONE`. A full chunk's sheet is its voxels' exactly; a
//! LOD-only chunk's comes from its level-1 mips, a mip cell of two voxels counting as its top
//! voxel, so it is at most one voxel high. A chunk's upload or drop marks its column dirty, and
//! [`ColumnMap::flush`] writes each dirty column's tile, the max over the sheets of the
//! window's chunks in that column, into an `r32sint` texture of `(W_x 32) x (W_y 32)` texels.
//! The texture is addressed like the chunk table: a column's tile sits at its chunk coordinate
//! wrapped by the window size, so a moving window reuses the tiles of the columns it leaves;
//! a tile whose wrapped slot now names another column is written again at the next flush.

use std::collections::{HashMap, HashSet};

use vxe_world::{Brick, ChunkCoord};

/// A map cell's edge in voxels.
pub const COLUMN_CELL: i32 = 2;
/// A chunk's footprint in map cells, per axis.
pub const COLUMN_CELLS: i32 = 64 / COLUMN_CELL;
/// A map texel of a column that holds no opaque voxel among the resident chunks.
pub const COLUMN_NONE: i32 = i32::MIN;

const SHEET_CELLS: usize = (COLUMN_CELLS * COLUMN_CELLS) as usize;
const SHEET_NONE: u8 = 0xFF;

/// A chunk's sheet: per cell, x fastest, the highest opaque voxel's local `z`.
type Sheet = Box<[u8; SHEET_CELLS]>;

pub(super) struct ColumnMap {
    texture: wgpu::Texture,
    /// The window's size in chunks along x and y: the tiles per axis.
    tiles: [i32; 2],
    /// The sheets of the mirrored chunks that hold an opaque voxel.
    sheets: HashMap<ChunkCoord, Sheet>,
    /// Columns (chunk x, y) whose tile is written again at the next flush.
    dirty: HashSet<[i32; 2]>,
    /// Per tile slot, the column its texels hold.
    held: Vec<Option<[i32; 2]>>,
    /// Tiles written since the map was made.
    pub(super) tiles_written: u64,
}

impl ColumnMap {
    pub(super) fn new(device: &wgpu::Device, window_size: [i32; 3]) -> Self {
        let tiles = [window_size[0].max(1), window_size[1].max(1)];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("column map"),
            size: wgpu::Extent3d {
                width: (tiles[0] * COLUMN_CELLS) as u32,
                height: (tiles[1] * COLUMN_CELLS) as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Sint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Self {
            texture,
            tiles,
            sheets: HashMap::new(),
            dirty: HashSet::new(),
            held: vec![None; (tiles[0] * tiles[1]) as usize],
            tiles_written: 0,
        }
    }

    pub(super) fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// The texture's bytes.
    pub(super) fn bytes(&self) -> u64 {
        u64::from(self.texture.width()) * u64::from(self.texture.height()) * 4
    }

    /// Sets `coord`'s sheet from its bricks: `bricks(i)` is brick `i` (x + 8 y + 64 z) of a
    /// full chunk, or with `mips` mip-brick `i` (x + 4 y + 16 z) of its level 1; `layers`
    /// gives a brick's opaque bits, layer `z` in word `z` with bit `x + 8 y`.
    pub(super) fn set_sheet<'a>(
        &mut self,
        coord: ChunkCoord,
        mips: bool,
        bricks: impl Fn(usize) -> &'a Brick,
        layers: impl Fn(&Brick) -> [u64; 8],
    ) {
        let sheet = if mips {
            sheet_of(4, 2, bricks, layers)
        } else {
            sheet_of(8, 1, bricks, layers)
        };
        let old = match sheet {
            Some(s) => self.sheets.insert(coord, s),
            None => self.sheets.remove(&coord),
        };
        if old.as_deref() != self.sheets.get(&coord).map(|s| &**s) {
            self.dirty.insert([coord.0[0], coord.0[1]]);
        }
    }

    /// Forgets `coord`'s sheet.
    pub(super) fn remove(&mut self, coord: ChunkCoord) {
        if self.sheets.remove(&coord).is_some() {
            self.dirty.insert([coord.0[0], coord.0[1]]);
        }
    }

    /// The column top of map cell `cell` (in map cells, absolute) over the window's chunks
    /// `z_range`, from the sheets.
    fn top(&self, cell: [i32; 2], z_range: std::ops::Range<i32>) -> i32 {
        let col = cell.map(|v| v.div_euclid(COLUMN_CELLS));
        let i = cell[0].rem_euclid(COLUMN_CELLS) + COLUMN_CELLS * cell[1].rem_euclid(COLUMN_CELLS);
        z_range
            .rev()
            .find_map(|z| {
                let s = self.sheets.get(&ChunkCoord([col[0], col[1], z]))?;
                let t = s[i as usize];
                (t != SHEET_NONE).then(|| z * 64 + i32::from(t))
            })
            .unwrap_or(COLUMN_NONE)
    }

    /// The top of the column holding voxel column `(x, y)` as the map holds it once flushed,
    /// from the CPU's sheets.
    pub(super) fn top_at(&self, x: i32, y: i32, z_range: std::ops::Range<i32>) -> i32 {
        self.top([x, y].map(|v| v.div_euclid(COLUMN_CELL)), z_range)
    }

    /// Writes the tiles of the dirty columns inside the window, and of every tile slot that
    /// now names another column. Returns the tiles written.
    pub(super) fn flush(&mut self, queue: &wgpu::Queue, min: [i32; 3], size: [i32; 3]) -> usize {
        let t = self.tiles;
        for y in min[1]..min[1] + t[1] {
            for x in min[0]..min[0] + t[0] {
                let s = self.slot([x, y]);
                if self.held[s] != Some([x, y]) {
                    self.dirty.insert([x, y]);
                }
            }
        }
        if self.dirty.is_empty() {
            return 0;
        }
        let z_range = min[2]..min[2] + size[2].max(1);
        let mut texels = vec![COLUMN_NONE; SHEET_CELLS];
        let mut n = 0;
        let dirty: Vec<[i32; 2]> = self.dirty.drain().collect();
        for c in dirty {
            if (0..2).any(|k| c[k] < min[k] || c[k] >= min[k] + t[k]) {
                continue;
            }
            for j in 0..COLUMN_CELLS {
                for i in 0..COLUMN_CELLS {
                    texels[(i + COLUMN_CELLS * j) as usize] = self.top(
                        [c[0] * COLUMN_CELLS + i, c[1] * COLUMN_CELLS + j],
                        z_range.clone(),
                    );
                }
            }
            let s = self.slot(c);
            self.held[s] = Some(c);
            let origin = [0, 1].map(|k| (c[k].rem_euclid(t[k]) * COLUMN_CELLS) as u32);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: origin[0],
                        y: origin[1],
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&texels),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some((COLUMN_CELLS * 4) as u32),
                    rows_per_image: Some(COLUMN_CELLS as u32),
                },
                wgpu::Extent3d {
                    width: COLUMN_CELLS as u32,
                    height: COLUMN_CELLS as u32,
                    depth_or_array_layers: 1,
                },
            );
            n += 1;
        }
        self.tiles_written += n as u64;
        n
    }

    fn slot(&self, c: [i32; 2]) -> usize {
        let t = self.tiles;
        (c[0].rem_euclid(t[0]) + t[0] * c[1].rem_euclid(t[1])) as usize
    }
}

impl super::GpuWorld {
    /// Sets `coord`'s column sheet from what its upload wrote: `chunk`'s bricks for a full
    /// chunk, else the level-1 mips of `src`.
    pub(super) fn note_columns(
        &mut self,
        coord: ChunkCoord,
        src: &super::window::Src,
        chunk: Option<&vxe_world::Chunk>,
    ) {
        let mats = &self.mats;
        let layers = |b: &Brick| opaque_layers(b, &mats.opaque, |p| mats.glass_of(p));
        match (src, chunk) {
            (_, Some(ch)) => {
                let bricks = ch.bricks();
                self.columns.set_sheet(coord, false, |i| &bricks[i], layers);
            }
            (super::window::Src::Lod(mips), None) => {
                let l1 = mips.level(1);
                self.columns.set_sheet(coord, true, |i| &l1[i], layers);
            }
            (super::window::Src::Full(_), None) => {}
        }
    }
}

/// Whether chunk column `(cx, cy)` over the window's chunks `z` is final: no chunk of it is
/// `pending` upload, and each one `wanted` names is `mirrored`.
pub(crate) fn column_is_final(
    cx: i32,
    cy: i32,
    z: std::ops::Range<i32>,
    pending: impl Fn(ChunkCoord) -> bool,
    mirrored: impl Fn(ChunkCoord) -> bool,
    wanted: Option<&dyn Fn(ChunkCoord) -> bool>,
) -> bool {
    z.into_iter().all(|z| {
        let c = ChunkCoord([cx, cy, z]);
        !pending(c) && wanted.is_none_or(|w| !w(c) || mirrored(c))
    })
}

/// A chunk's sheet from `e`^3 bricks of `cell`-voxel cells (`e` 8 and `cell` 1 for a full
/// chunk, 4 and 2 for level 1's mips), each brick's opaque layers from `layers`; `None` when
/// no voxel is opaque. A cell of `cell` voxels counts as solid up to its top voxel.
fn sheet_of<'a>(
    e: usize,
    cell: i32,
    bricks: impl Fn(usize) -> &'a Brick,
    layers: impl Fn(&Brick) -> [u64; 8],
) -> Option<Sheet> {
    let mut sheet: Sheet = Box::new([SHEET_NONE; SHEET_CELLS]);
    let mut any = false;
    for by in 0..e {
        for bx in 0..e {
            // The brick column's cells already topped, bit x + 8 y.
            let mut covered = 0u64;
            for bz in (0..e).rev() {
                if covered == u64::MAX {
                    break;
                }
                let b = bricks(bx + e * (by + e * bz));
                let l = layers(b);
                for z in (0..8).rev() {
                    let mut new = l[z] & !covered;
                    if new == 0 {
                        continue;
                    }
                    covered |= new;
                    any = true;
                    let top = ((bz * 8 + z) as i32 + 1) * cell - 1;
                    while new != 0 {
                        let bit = new.trailing_zeros() as usize;
                        new &= new - 1;
                        let (x, y) = (bx * 8 + bit % 8, by * 8 + bit / 8);
                        // Brick cell x spans voxels x cell..(x + 1) cell, inside one map cell.
                        let (mx, my) =
                            (x as i32 * cell / COLUMN_CELL, y as i32 * cell / COLUMN_CELL);
                        let k = (mx + COLUMN_CELLS * my) as usize;
                        let t = top as u8;
                        if sheet[k] == SHEET_NONE || sheet[k] < t {
                            sheet[k] = t;
                        }
                    }
                }
            }
        }
    }
    any.then_some(sheet)
}

/// A brick's opaque layers: a uniform brick's whole or nothing by `opaque`, a full brick's
/// packed mask less its translucent voxels (`glass`).
pub(super) fn opaque_layers(
    b: &Brick,
    opaque: &[bool],
    glass: impl Fn(&vxe_world::packed::Packed) -> [u64; 8],
) -> [u64; 8] {
    match b {
        Brick::Uniform(m) => {
            if opaque[usize::from(*m)] {
                [u64::MAX; 8]
            } else {
                [0; 8]
            }
        }
        Brick::Full(p) => {
            let g = glass(p);
            let m = p.mask();
            [0, 1, 2, 3, 4, 5, 6, 7].map(|z| m[z] & !g[z])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full chunk's sheet is the highest opaque voxel per two-voxel cell; a level-1 mip
    /// cell counts to its top voxel.
    #[test]
    fn a_sheet_is_the_top_of_its_opaque_voxels() {
        let solid = Brick::Uniform(1);
        let air = Brick::Uniform(0);
        let opaque = [false, true];
        // Bricks below z 3 solid, and one voxel column (x 5, y 9) up to local z 40.
        let tall = Brick::from_fn(|p| if p[0] == 5 && p[1] == 1 { 1 } else { 0 });
        let bricks: Vec<Brick> = (0..512)
            .map(|i| {
                let (bx, by, bz) = (i % 8, (i / 8) % 8, i / 64);
                if bz < 3 {
                    solid.clone()
                } else if bx == 0 && by == 1 && bz <= 4 {
                    tall.clone()
                } else {
                    air.clone()
                }
            })
            .collect();
        let layers = |b: &Brick| opaque_layers(b, &opaque, |_| [0; 8]);
        let s = sheet_of(8, 1, |i| &bricks[i], layers).expect("solid");
        assert_eq!(s[0], 23);
        assert_eq!(s[(5 / 2 + COLUMN_CELLS * (9 / 2)) as usize], 39);
        assert_eq!(
            s[(4 / 2 + COLUMN_CELLS * (9 / 2)) as usize],
            39,
            "x 4 and 5 share a cell"
        );
        assert_eq!(s[(6 / 2 + COLUMN_CELLS * (9 / 2)) as usize], 23);
        let mips: Vec<Brick> = (0..64)
            .map(|i| {
                if i / 16 == 0 {
                    solid.clone()
                } else {
                    air.clone()
                }
            })
            .collect();
        let m = sheet_of(4, 2, |i| &mips[i], layers).expect("solid");
        assert!(
            m.iter().all(|&t| t == 15),
            "level 1 mip cells 0..8 reach voxel 15"
        );
        assert!(sheet_of(8, 1, |_| &air, layers).is_none());
    }
}
