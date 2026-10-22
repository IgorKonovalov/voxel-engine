//! `--castle`: a gothic castle generated at startup as a multi-model MagicaVoxel scene, read
//! back through `vxe-assets`' parser and palette map, and stamped into the unbounded cave by a
//! [`StampedSource`] over a [`HallSource`].
//!
//! The castle is drawn in its own frame `(u, v, w)`, whose origin is the world voxel
//! [`ORIGIN`]: `u = x - 132`, `v = y + 268`, `w = z - 12`, with `w = 0` on the hall's floor.
//! Its box is `u` and `v` in `0..600` and `w` in `-100..400`, split into [`tiles`] of at most
//! 256 per axis (a `.vox` model's limit). Each tile is a model whose voxels are palette
//! indices: [`STONE`] to [`TORCH`] are the castle's materials, and [`EXPLICIT_AIR`] is air the
//! dungeon carves. Above the floor index 0 is "leave the world alone" (the hall is already
//! air); in the dungeon tiles too, so rock the dungeon does not touch stays.
//!
//! Every voxel's index is a pure integer function of its coordinate: a list of parts, each a
//! box and a rule, evaluated in order with the last part that claims a voxel winning. The file
//! lists tiles in [`tiles`] order and each tile's voxels with `x` fastest, so the bytes are
//! the same however many threads build the tiles. Nothing here uses floating point.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use vxe_assets::{Node, Slot, Unmapped, VoxFile, VoxMap, VoxModel};
use vxe_world::{MaterialId, Prefab, Rot, StampedSource, VoxelCoord};

use crate::cave::HallSource;

/// The world voxel at castle-local `(0, 0, 0)`.
pub const ORIGIN: [i32; 3] = [132, -268, 12];
/// The castle box's minimum corner, castle-local.
pub const BOX_LO: [i32; 3] = [0, 0, -100];
/// The castle box's exclusive maximum corner, castle-local.
pub const BOX_HI: [i32; 3] = [600, 600, 400];

/// Palette indices of the generated `.vox`.
pub const STONE: u8 = 1;
/// Darker stone for ribs, tracery and merlons.
pub const TRIM: u8 = 2;
pub const SLATE: u8 = 3;
pub const WOOD: u8 = 4;
pub const IRON: u8 = 5;
pub const GLASS: u8 = 6;
pub const TORCH: u8 = 7;
/// Air written explicitly: the dungeon's rooms, carved out of the plinth.
pub const EXPLICIT_AIR: u8 = 8;

/// The material ids the castle's rows take, appended to the cave's table by
/// `flythrough::castle_materials`: palette index `i` in `1..=7` maps to `MATERIAL_BASE + i - 1`.
pub const MATERIAL_BASE: MaterialId = 5;

/// The material id palette index `i` (`1..=7`) maps to.
pub const fn material(i: u8) -> MaterialId {
    MATERIAL_BASE + i as MaterialId - 1
}

/// The palette written into the `.vox`: index 1 to 8 as the castle's colours, the rest
/// MagicaVoxel's default.
const PALETTE_COLOURS: [[u8; 4]; 8] = [
    [140, 135, 128, 255], // stone
    [92, 89, 86, 255],    // trim
    [51, 56, 69, 255],    // slate
    [102, 69, 38, 255],   // wood
    [46, 46, 51, 255],    // iron
    [64, 90, 217, 255],   // glass
    [255, 179, 77, 255],  // torch
    [255, 0, 255, 255],   // explicit air, a colour no material has
];

/// One tile of the castle box: its castle-local minimum corner and its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub lo: [i32; 3],
    pub size: [u32; 3],
}

impl Tile {
    /// Whether this is a dungeon tile (below the floor).
    pub fn dungeon(&self) -> bool {
        self.lo[2] < 0
    }

    /// Voxels in the tile.
    pub fn len(&self) -> usize {
        self.size.iter().map(|&s| s as usize).product()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The tiles whose box meets the world voxel box `lo..hi` (maximum exclusive).
pub fn tiles_meeting(lo: [i32; 3], hi: [i32; 3]) -> impl Fn(&Tile) -> bool {
    move |t| {
        (0..3).all(|k| {
            let a = ORIGIN[k] + t.lo[k];
            a < hi[k] && lo[k] < a + t.size[k] as i32
        })
    }
}

/// The 27 tiles: `u` and `v` split at 256, 256, 88; `w` at the dungeon's 100 below the floor,
/// then 256 and 144 above it. Ordered by layer (dungeon, lower, upper), then `v`, then `u`.
pub fn tiles() -> Vec<Tile> {
    const UV: [(i32, u32); 3] = [(0, 256), (256, 256), (512, 88)];
    const W: [(i32, u32); 3] = [(-100, 100), (0, 256), (256, 144)];
    let mut out = Vec::with_capacity(27);
    for (w, sw) in W {
        for (v, sv) in UV {
            for (u, su) in UV {
                out.push(Tile {
                    lo: [u, v, w],
                    size: [su, sv, sw],
                });
            }
        }
    }
    out
}

/// A part of the castle: a castle-local box (minimum inclusive, maximum exclusive) and the
/// index its rule gives each voxel inside it, or `None` to leave the voxel to earlier parts.
struct Part {
    lo: [i32; 3],
    hi: [i32; 3],
    rule: Box<dyn Fn(i32, i32, i32) -> Option<u8> + Send + Sync>,
}

impl Part {
    fn new(
        lo: [i32; 3],
        hi: [i32; 3],
        rule: impl Fn(i32, i32, i32) -> Option<u8> + Send + Sync + 'static,
    ) -> Self {
        Self {
            lo,
            hi,
            rule: Box::new(rule),
        }
    }

    /// A box filled with one index.
    fn fill(lo: [i32; 3], hi: [i32; 3], index: u8) -> Self {
        Self::new(lo, hi, move |_, _, _| Some(index))
    }

    fn meets(&self, lo: [i32; 3], hi: [i32; 3]) -> bool {
        (0..3).all(|k| self.lo[k] < hi[k] && lo[k] < self.hi[k])
    }
}

/// `a * a + b * b` in `i64`.
fn d2(a: i32, b: i32) -> i64 {
    let (a, b) = (i64::from(a), i64::from(b));
    a * a + b * b
}

/// A two-centred pointed arch standing on `h = 0` over the `2 * half` voxels `d` in
/// `-half..half` across its span: true inside the opening. Each side is an arc of radius `2 *
/// half` centred on the springing line at the opposite jamb, so the apex rises `half * sqrt(3)`
/// above the springing line, which sits that far below `height`. Evaluated at voxel centres in
/// doubled coordinates, so the opening is symmetric about the span's centre line.
fn pointed_arch(d: i32, h: i32, half: i32, height: i32) -> bool {
    let x = 2 * d + 1;
    if h < 0 || x.abs() >= 2 * half {
        return false;
    }
    let spring = height - half * 1732 / 1000;
    if h < spring {
        return true;
    }
    let y = 2 * (h - spring) + 1;
    let r2 = 16 * i64::from(half) * i64::from(half);
    d2(x + 2 * half, y) < r2 && d2(x - 2 * half, y) < r2
}

/// A merlon's pattern along a parapet: solid for the first 6 of every 12 voxels of `s`.
fn merlon(s: i32) -> bool {
    s.rem_euclid(12) < 6
}

/// A round tower's merlons: a checker of 6-voxel cells around the ring.
fn ring_merlon(du: i32, dv: i32) -> bool {
    (du.div_euclid(6) + dv.div_euclid(6)).rem_euclid(2) == 0
}

/// The octagon's distance from its centre, times 1000: the largest of `|a|`, `|b|` and
/// `(|a| + |b|) / sqrt(2)`, so `oct(a, b) <= 1000 r` is a regular octagon of inradius `r`.
fn oct(a: i32, b: i32) -> i64 {
    let (a, b) = (i64::from(a.abs()), i64::from(b.abs()));
    (1000 * a).max(1000 * b).max(707 * (a + b))
}

/// Which sixteenth of a turn `(du, dv)` lies in, counter-clockwise from `+u`: 0 to 15, sector
/// `s` spanning `[45 s / 2, 45 (s + 1) / 2)` degrees; 0 at the centre. Integer only: the
/// point is turned into the first quadrant and compared against the tangent of 45 / 2
/// degrees, `4142 / 10000`.
fn sector16(du: i32, dv: i32) -> i32 {
    let (q, x, y) = if du > 0 && dv >= 0 {
        (0, du, dv)
    } else if du <= 0 && dv > 0 {
        (1, dv, -du)
    } else if du < 0 && dv <= 0 {
        (2, -du, -dv)
    } else if du >= 0 && dv < 0 {
        (3, -dv, du)
    } else {
        return 0;
    };
    let (x, y) = (i64::from(x), i64::from(y));
    let sub = if y * 10_000 < x * 4142 {
        0
    } else if y < x {
        1
    } else if y * 4142 < x * 10_000 {
        2
    } else {
        3
    };
    4 * q + sub
}

/// The four curtain walls: outer faces 60 in from the box on every side, 20 thick, 12 m tall,
/// merlons on their outer 4 voxels with a wall walk behind them.
fn curtain_walls() -> Vec<Part> {
    let merlons = |lo: [i32; 3], hi: [i32; 3], along_u: bool| {
        Part::new(lo, hi, move |u, v, _| {
            merlon(if along_u { u } else { v } - 60).then_some(TRIM)
        })
    };
    vec![
        Part::fill([60, 60, 0], [80, 540, 120], STONE),
        Part::fill([520, 60, 0], [540, 540, 120], STONE),
        Part::fill([60, 60, 0], [540, 80, 120], STONE),
        Part::fill([60, 520, 0], [540, 540, 120], STONE),
        merlons([60, 60, 120], [64, 540, 128], false),
        merlons([536, 60, 120], [540, 540, 128], false),
        merlons([60, 60, 120], [540, 64, 128], true),
        merlons([60, 536, 120], [540, 540, 128], true),
    ]
}

/// A corner tower centred at `(cu, cv)`: radius 50, walls 15 thick, 20 m tall, four arrow
/// slits at two heights, a doorway toward the courtyard (`toward` is the courtyard's
/// direction on each axis), a trim cornice and a conical slate roof to `w = 280`.
fn corner_tower(cu: i32, cv: i32, toward: [i32; 2]) -> Vec<Part> {
    const R: i32 = 50;
    const INNER: i32 = 35;
    const TOP: i32 = 200;
    const ROOF_R: i32 = 56;
    const ROOF_TOP: i32 = 280;
    let lo = [cu - ROOF_R, cv - ROOF_R, 0];
    let hi = [cu + ROOF_R + 1, cv + ROOF_R + 1, ROOF_TOP];
    vec![
        Part::new(
            [cu - R, cv - R, 0],
            [cu + R, cv + R, TOP],
            move |u, v, w| {
                let (du, dv) = (u - cu, v - cv);
                let r2 = d2(du, dv);
                if r2 >= i64::from(R * R) {
                    return None;
                }
                if r2 < i64::from(INNER * INNER) {
                    return Some(0);
                }
                let slit = (60..80).contains(&w) || (130..150).contains(&w);
                if slit && (du.abs() <= 1 || dv.abs() <= 1) {
                    return Some(0);
                }
                // The doorway: on the courtyard's diagonal, 12 wide and 3 m tall.
                let (a, b) = (du * toward[0], dv * toward[1]);
                if a > 0 && b > 0 && (a - b).abs() < 9 && w < 30 {
                    return Some(0);
                }
                Some(STONE)
            },
        ),
        Part::new(lo, [hi[0], hi[1], TOP], move |u, v, w| {
            let r2 = d2(u - cu, v - cv);
            (w >= TOP - 4 && r2 >= i64::from(R * R) && r2 < 53 * 53).then_some(TRIM)
        }),
        Part::new([lo[0], lo[1], TOP], hi, move |u, v, w| {
            let r = ROOF_R * (ROOF_TOP - w) / (ROOF_TOP - TOP);
            let r2 = d2(u - cu, v - cv);
            let inner = (r - 4).max(0);
            (r2 <= i64::from(r * r) && (r < 4 || r2 >= i64::from(inner * inner))).then_some(SLATE)
        }),
    ]
}

/// The gatehouse on the `u = 60` face at `v = 300`: a block over the passage, two flanking
/// towers of radius 35 and 16 m centred 60 to each side, merlons on both, a pointed arch 40
/// wide and 60 tall through the block and the wall, an iron portcullis half raised, and a
/// torch on each side of the arch.
fn gatehouse() -> Vec<Part> {
    const CV: i32 = 300;
    let mut parts = vec![
        Part::fill([45, 260, 0], [95, 340, 150], STONE),
        Part::new([45, 260, 150], [95, 340, 158], |u, v, _| {
            let rim = !(49..91).contains(&u) || !(264..336).contains(&v);
            (rim && merlon(u + v)).then_some(TRIM)
        }),
    ];
    for cv in [CV - 60, CV + 60] {
        let cu = 60;
        parts.push(Part::new(
            [cu - 35, cv - 35, 0],
            [cu + 35, cv + 35, 168],
            move |u, v, w| {
                let (du, dv) = (u - cu, v - cv);
                let r2 = d2(du, dv);
                if r2 >= 35 * 35 {
                    return None;
                }
                if w >= 160 {
                    return (r2 >= 30 * 30 && ring_merlon(du, dv)).then_some(TRIM);
                }
                if (80..100).contains(&w) && dv.abs() <= 1 && du < 0 {
                    return Some(0);
                }
                Some(STONE)
            },
        ));
    }
    // The passage through the block and the wall.
    parts.push(Part::new([40, 280, 0], [100, 320, 60], |_, v, w| {
        pointed_arch(v - CV, w, 20, 60).then_some(0)
    }));
    // The portcullis: 2-voxel iron bars on a 6-voxel grid, raised to half the arch.
    parts.push(Part::new([47, 280, 30], [49, 320, 60], |_, v, w| {
        let bar = (v - 280).rem_euclid(6) < 2 || w.rem_euclid(8) < 2;
        (pointed_arch(v - CV, w, 20, 60) && bar).then_some(IRON)
    }));
    for v in GATE_TORCH_V {
        parts.push(torch([43, v, 40]));
    }
    parts
}

/// The `v` of each gate torch's lower corner; the torches stand at `u` 43..45, `w` 40..43,
/// against the gatehouse's face.
pub const GATE_TORCH_V: [i32; 2] = [275, 323];

/// A torch: a 2 x 2 x 3 block of torch material with its minimum corner at `lo`.
fn torch(lo: [i32; 3]) -> Part {
    Part::fill(lo, [lo[0] + 2, lo[1] + 2, lo[2] + 3], TORCH)
}

/// The nave's walls, castle-local: `u` 260..500, `v` 220..380, 12 thick.
const NAVE_LO: [i32; 2] = [260, 220];
const NAVE_HI: [i32; 2] = [500, 380];
const NAVE_WALL: i32 = 12;
/// The eaves and the ridge of the nave's roof.
const EAVES: i32 = 200;
const RIDGE: i32 = 300;
/// Half the roof's span across `v`: the eaves overhang the walls by 6.
const ROOF_HALF: i32 = 86;
/// The nave's long axis, `v = 300`.
const NAVE_V: i32 = 300;
/// The vault's springing line and its apex.
const SPRING: i32 = 140;
const APEX: i32 = 190;
/// A vault bay's length along `u`; bays start at `u = 260 + 40 k`.
const BAY: i32 = 40;

/// The underside of the nave's roof at `v`: a straight pitch from the eaves to the ridge.
fn roof_h(v: i32) -> i32 {
    EAVES + (ROOF_HALF - (v - NAVE_V).abs()) * (RIDGE - EAVES) / ROOF_HALF
}

/// The vault's surface at bay position `t` (0 to 39 along `u`) and `dv` across the nave: a
/// groin of the nave's barrel across `v` (half span 68) and the bay's cross barrel across `u`
/// (half span 20), each a parabola from the springing line to the apex; the higher wins.
fn vault_h(t: i32, dv: i32) -> i32 {
    let du = t - BAY / 2;
    let rise = APEX - SPRING;
    let hu = APEX - rise * du * du / (20 * 20);
    let hv = APEX - rise * dv * dv / (68 * 68);
    hu.max(hv)
}

/// Whether bay position `t`, `dv` lies on a rib: the transverse rib at the bay's boundary,
/// the ridge rib along the nave's axis, or a diagonal rib along a groin, each 3 voxels wide.
fn on_rib(t: i32, dv: i32) -> bool {
    let du = t - BAY / 2;
    t <= 1 || t == BAY - 1 || dv.abs() <= 1 || (du.abs() * 68 - dv.abs() * 20).abs() <= 102
}

/// The chapel keep: the nave's walls and gables, its quadripartite rib vault with wall
/// shafts, buttresses every bay on both long walls, a lancet per bay, the west door with its
/// leaves open, the rose window over it, the pitched slate roof, a torch per bay, and the
/// square west tower with its octagonal spire.
fn keep() -> Vec<Part> {
    let inner_lo = NAVE_LO.map(|c| c + NAVE_WALL);
    let inner_hi = NAVE_HI.map(|c| c - NAVE_WALL);
    let mut p = Vec::new();
    // Buttresses: 10 wide at each bay boundary, 24 out to 10 m and 14 out to 17 m.
    for k in 0..=6 {
        let c = NAVE_LO[0] + BAY * k;
        for (lo, hi) in [(196, 220), (380, 404)] {
            p.push(Part::new(
                [c - 5, lo, 0],
                [c + 5, hi, 170],
                move |_, v, w| {
                    let depth = if lo < NAVE_V { 220 - v } else { v - 379 };
                    (depth <= if w < 100 { 24 } else { 14 }).then_some(STONE)
                },
            ));
        }
    }
    p.push(Part::new(
        [NAVE_LO[0], NAVE_LO[1], 0],
        [NAVE_HI[0], NAVE_HI[1], RIDGE + 4],
        move |u, v, w| {
            let inside =
                (inner_lo[0]..inner_hi[0]).contains(&u) && (inner_lo[1]..inner_hi[1]).contains(&v);
            if !inside {
                return (w < roof_h(v)).then_some(STONE);
            }
            let t = (u - NAVE_LO[0]).rem_euclid(BAY);
            let dv = v - NAVE_V;
            if w < SPRING {
                // Wall shafts: 3 x 3 trim columns under each transverse rib.
                let shaft =
                    (t <= 1 || t == BAY - 1) && (v < inner_lo[1] + 3 || v >= inner_hi[1] - 3);
                return Some(if shaft { TRIM } else { 0 });
            }
            let h = vault_h(t, dv);
            Some(if w < h - 3 {
                0
            } else if w < h {
                if on_rib(t, dv) { TRIM } else { 0 }
            } else if w < h + 4 {
                STONE
            } else {
                0
            })
        },
    ));
    // Lancets: 20 wide and 80 tall from 6 m, centred in each bay, glass at mid-wall behind a
    // 2-voxel trim mullion.
    for (wall, glass) in [(NAVE_LO[1], NAVE_LO[1] + 5), (inner_hi[1], inner_hi[1] + 5)] {
        p.push(Part::new(
            [270, wall, 60],
            [490, wall + NAVE_WALL, 140],
            move |u, v, w| {
                let centre = NAVE_LO[0] + BAY / 2 + BAY * (u - NAVE_LO[0]).div_euclid(BAY);
                let d = u - centre;
                if !pointed_arch(d, w - 60, 10, 80) {
                    return None;
                }
                Some(if d == -1 || d == 0 {
                    TRIM
                } else if (glass..glass + 2).contains(&v) {
                    GLASS
                } else {
                    0
                })
            },
        ));
    }
    // The west door, 30 x 50, its two wooden leaves standing open against the nave.
    p.push(Part::new([260, 285, 0], [272, 315, 50], |_, v, w| {
        pointed_arch(v - NAVE_V, w, 15, 50).then_some(0)
    }));
    for v in [285, 313] {
        p.push(Part::fill([272, v, 0], [286, v + 2, 42], WOOD));
    }
    // The rose window: radius 30 at w = 150 on the west wall; an outer ring, a middle ring,
    // a hub and eight spokes of trim, glass between them at mid-wall.
    p.push(Part::new([260, 270, 120], [272, 330, 180], |u, v, w| {
        let (dv, dw) = (v - NAVE_V, w - ROSE_W);
        let r2 = d2(dv, dw);
        if r2 >= 30 * 30 {
            return None;
        }
        let tracery = !(5 * 5..27 * 27).contains(&r2)
            || (15 * 15..17 * 17).contains(&r2)
            || dv.abs() <= 1
            || dw.abs() <= 1
            || (dv - dw).abs() <= 1
            || (dv + dw).abs() <= 1;
        Some(if tracery {
            TRIM
        } else if (265..267).contains(&u) {
            GLASS
        } else {
            0
        })
    }));
    p.push(Part::new(
        [NAVE_LO[0] - 6, NAVE_V - ROOF_HALF, EAVES],
        [NAVE_HI[0] + 6, NAVE_V + ROOF_HALF + 1, RIDGE + 4],
        |_, v, w| {
            let h = roof_h(v);
            (w >= h && w < h + 4).then_some(SLATE)
        },
    ));
    for k in 0..6 {
        p.push(torch([NAVE_LO[0] + BAY / 2 + BAY * k - 1, inner_lo[1], 40]));
    }
    // The west tower: 40 x 40 beside the west front, walls 6 thick, slits, a parapet of
    // merlons at 28 m, and an octagonal slate spire from there to 38 m with a trim finial.
    p.push(Part::new([250, 180, 0], [290, 220, 288], |u, v, w| {
        if w >= 280 {
            let rim = !(253..287).contains(&u) || !(183..217).contains(&v);
            return (rim && merlon(u + v)).then_some(TRIM);
        }
        if (256..284).contains(&u) && (186..214).contains(&v) {
            return Some(0);
        }
        let slit = (120..140).contains(&w) || (200..220).contains(&w);
        if slit && ((u - 270).abs() <= 1 || (v - 200).abs() <= 1) {
            return Some(0);
        }
        Some(STONE)
    }));
    p.push(Part::new([252, 182, 280], [288, 218, 388], |u, v, w| {
        let (a, b) = (u - 270, v - 200);
        if w >= 380 {
            return (a.abs() <= 1 && b.abs() <= 1).then_some(TRIM);
        }
        let r = 17 * (380 - w) / 100;
        let o = oct(a, b);
        (o <= 1000 * i64::from(r) && (r < 3 || o > 1000 * i64::from(r - 3))).then_some(SLATE)
    }));
    p
}

/// The rose window's centre height.
pub const ROSE_W: i32 = 150;

/// The spiral stair's axis.
pub const STAIR: [i32; 2] = [200, 450];
/// The dungeon's floor: 4 voxels above the dungeon tiles' bottom, so the lining fits under it.
pub const FLOOR: i32 = -96;
/// The cells' lower `u` corners; each cell is 30 x 30 x 30, one on each side of the corridor.
pub const CELLS: [i32; 4] = [245, 280, 405, 440];
/// The `v` of the bars in front of the cells south and north of the corridor.
pub const BARS_V: [i32; 2] = [437, 462];

/// Inside a barrel-vaulted passage 20 wide and 30 tall at `d` across it (`-10..10`) and `w`:
/// walls to 2 m, then a half circle of radius 10.
fn barrel(d: i32, w: i32) -> bool {
    w < FLOOR + 20 || d2(2 * d + 1, 2 * (w - FLOOR - 20) + 1) < 400
}

/// The crypt's groin vault over its 30-voxel bays: the higher of two parabolas across `u` and
/// across `v` from the springing line at about `w = -66` to the apex at `w = -46`.
fn crypt_h(u: i32, v: i32) -> i32 {
    let h = |c: i32| {
        let x = 2 * c.rem_euclid(30) - 29;
        -46 - 20 * x * x / 900
    };
    h(u - 300).max(h(v - 240))
}

/// The dungeon, all below the floor: a 4-voxel stone lining around every room, then the rooms'
/// air, then what stands in them. The stair shaft of radius 25 under the stair house; 48
/// steps, a 2-voxel rise per sixteenth of a turn, three turns around an open well of radius 6;
/// the barrel-vaulted corridor from the stair's foot along `u` and a cross corridor along `v`
/// to the crypt; eight barred cells; the crypt, `u` 300..420 and `v` 240..360, 5 m tall, its
/// groin vault on nine pillars; and a torch every 80 voxels.
fn dungeon() -> Vec<Part> {
    const TOP: i32 = FLOOR + 30;
    const L: i32 = 4;
    let [su, sv] = STAIR;
    let mut p = vec![
        Part::new(
            [su - 29, sv - 29, FLOOR - L],
            [su + 29, sv + 29, 0],
            move |u, v, _| (d2(u - su, v - sv) < 29 * 29).then_some(STONE),
        ),
        Part::fill(
            [200 - L, 440 - L, FLOOR - L],
            [520 + L, 460 + L, TOP + L],
            STONE,
        ),
        Part::fill(
            [350 - L, 350 - L, FLOOR - L],
            [370 + L, 540 + L, TOP + L],
            STONE,
        ),
        Part::fill(
            [300 - L, 240 - L, FLOOR - L],
            [420 + L, 360 + L, -46 + L],
            STONE,
        ),
    ];
    for u0 in CELLS {
        for v0 in [410, 460] {
            p.push(Part::fill(
                [u0 - L, v0 - L, FLOOR - L],
                [u0 + 30 + L, v0 + 30 + L, TOP + L],
                STONE,
            ));
        }
    }
    p.push(Part::new(
        [su - 25, sv - 25, FLOOR],
        [su + 25, sv + 25, 0],
        move |u, v, _| (d2(u - su, v - sv) < 25 * 25).then_some(EXPLICIT_AIR),
    ));
    p.push(Part::new([200, 440, FLOOR], [520, 460, TOP], |_, v, w| {
        barrel(v - 450, w).then_some(EXPLICIT_AIR)
    }));
    p.push(Part::new([350, 350, FLOOR], [370, 540, TOP], |u, _, w| {
        barrel(u - 360, w).then_some(EXPLICIT_AIR)
    }));
    for u0 in CELLS {
        for v0 in [410, 460] {
            p.push(Part::fill(
                [u0, v0, FLOOR],
                [u0 + 30, v0 + 30, TOP],
                EXPLICIT_AIR,
            ));
        }
    }
    p.push(Part::new([300, 240, FLOOR], [420, 360, -46], |u, v, w| {
        (w < crypt_h(u, v)).then_some(EXPLICIT_AIR)
    }));
    // The steps: step k (0 to 47) fills its sixteenth of a turn from radius 6 to 25, 6 deep
    // under its tread at w = -2 k.
    p.push(Part::new(
        [su - 25, sv - 25, FLOOR - L],
        [su + 25, sv + 25, 0],
        move |u, v, w| {
            let (du, dv) = (u - su, v - sv);
            let r2 = d2(du, dv);
            if !(36..625).contains(&r2) {
                return None;
            }
            let s = sector16(du, dv);
            (s..48)
                .step_by(16)
                .any(|k| (-2 * k - 6..-2 * k).contains(&w))
                .then_some(STONE)
        },
    ));
    for pu in [330, 360, 390] {
        for pv in [270, 300, 330] {
            p.push(Part::new(
                [pu - 8, pv - 8, FLOOR],
                [pu + 8, pv + 8, -46],
                move |u, v, w| {
                    let r2 = d2(u - pu, v - pv);
                    if (-70..-66).contains(&w) && r2 < 64 {
                        Some(TRIM)
                    } else {
                        (r2 < 36).then_some(STONE)
                    }
                },
            ));
        }
    }
    // Bars 1 voxel thick at a 3-voxel pitch and two cross bars, a 6-voxel door gap in each.
    for u0 in CELLS {
        for v in BARS_V {
            p.push(Part::new(
                [u0, v, FLOOR],
                [u0 + 30, v + 1, TOP],
                move |u, _, w| {
                    let x = u - u0;
                    if (12..18).contains(&x) {
                        return None;
                    }
                    (x % 3 == 1 || w == FLOOR + 16 || w == FLOOR + 26).then_some(IRON)
                },
            ));
        }
    }
    for u in [240, 320, 400, 480] {
        p.push(torch([u - 1, 440, FLOOR + 16]));
    }
    p.push(torch([350, 500, FLOOR + 16]));
    for lo in [
        [300, 299, FLOOR + 16],
        [418, 299, FLOOR + 16],
        [359, 240, FLOOR + 16],
    ] {
        p.push(torch(lo));
    }
    p
}

/// The stair house over the shaft: a round wall of radius 31 and 4 m, 6 thick, a doorway
/// facing the gate, and a conical slate roof to 64 voxels.
fn stair_house() -> Vec<Part> {
    let [su, sv] = STAIR;
    vec![Part::new(
        [su - 34, sv - 34, 0],
        [su + 35, sv + 35, 64],
        move |u, v, w| {
            let (du, dv) = (u - su, v - sv);
            let r2 = d2(du, dv);
            if w >= 40 {
                let r = 34 * (64 - w) / 24;
                let inner = (r - 3).max(0);
                return (r2 <= i64::from(r * r) && (r < 3 || r2 >= i64::from(inner * inner)))
                    .then_some(SLATE);
            }
            if r2 >= 31 * 31 {
                return None;
            }
            if r2 < 25 * 25 || (du < 0 && dv.abs() < 7 && w < 25) {
                return Some(0);
            }
            Some(STONE)
        },
    )]
}

/// Every part of the castle, in evaluation order.
fn parts() -> Vec<Part> {
    let mut p = curtain_walls();
    p.extend(corner_tower(60, 60, [1, 1]));
    p.extend(corner_tower(60, 540, [1, -1]));
    p.extend(corner_tower(540, 60, [-1, 1]));
    p.extend(corner_tower(540, 540, [-1, -1]));
    p.extend(gatehouse());
    p.extend(keep());
    p.extend(stair_house());
    p.extend(dungeon());
    p
}

/// The index at castle-local `(u, v, w)` from `parts`, the last claim winning.
fn index_at(parts: &[&Part], u: i32, v: i32, w: i32) -> u8 {
    let mut idx = 0;
    for p in parts {
        if (p.lo[2]..p.hi[2]).contains(&w)
            && let Some(i) = (p.rule)(u, v, w)
        {
            idx = i;
        }
    }
    idx
}

/// The tile's model: its voxels with a non-zero index, `x` fastest, then `y`, then `z`.
fn tile_model(all: &[Part], tile: Tile) -> VoxModel {
    let hi = [0, 1, 2].map(|k| tile.lo[k] + tile.size[k] as i32);
    let parts: Vec<&Part> = all.iter().filter(|p| p.meets(tile.lo, hi)).collect();
    let [sx, sy, _] = tile.size.map(|s| s as usize);
    // The parts whose box covers each column, in evaluation order.
    let mut columns: Vec<Vec<&Part>> = Vec::with_capacity(sx * sy);
    for y in 0..sy as i32 {
        for x in 0..sx as i32 {
            let (u, v) = (tile.lo[0] + x, tile.lo[1] + y);
            columns.push(
                parts
                    .iter()
                    .copied()
                    .filter(|p| (p.lo[0]..p.hi[0]).contains(&u) && (p.lo[1]..p.hi[1]).contains(&v))
                    .collect(),
            );
        }
    }
    let mut voxels = Vec::new();
    for z in 0..tile.size[2] {
        let w = tile.lo[2] + z as i32;
        for y in 0..tile.size[1] {
            for x in 0..tile.size[0] {
                let col = &columns[x as usize + sx * y as usize];
                if col.is_empty() {
                    continue;
                }
                let i = index_at(col, tile.lo[0] + x as i32, tile.lo[1] + y as i32, w);
                if i != 0 {
                    voxels.push([x as u8, y as u8, z as u8, i]);
                }
            }
        }
    }
    VoxModel {
        size: tile.size,
        voxels,
    }
}

/// The castle as a `.vox` scene: one model per tile that lists any voxel, in [`tiles`] order,
/// under a root transform and a group, each model behind its own transform whose `_t` puts
/// the model's centre (`lo + size / 2`) at its castle-local place. Tiles are built on up to
/// `threads` threads; the result does not depend on the count.
pub fn castle_vox_on(threads: usize) -> VoxFile {
    castle_vox_where(threads, |_| true)
}

/// [`castle_vox_on`] built from only the tiles `keep` accepts: a view that needs part of the
/// castle builds and holds only that part.
pub fn castle_vox_where(threads: usize, keep: impl Fn(&Tile) -> bool) -> VoxFile {
    let parts = parts();
    let tiles: Vec<Tile> = tiles().into_iter().filter(|t| keep(t)).collect();
    let next = AtomicUsize::new(0);
    let mut built: Vec<Option<VoxModel>> = vec![None; tiles.len()];
    std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads.clamp(1, tiles.len().max(1)))
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&t) = tiles.get(k) else { break };
                        out.push((k, tile_model(&parts, t)));
                    }
                    out
                })
            })
            .collect();
        for h in workers {
            for (k, m) in h.join().expect("a castle tile worker panicked") {
                built[k] = Some(m);
            }
        }
    });
    let mut models = Vec::new();
    let mut placed = Vec::new();
    for (t, m) in tiles.iter().zip(built) {
        let m = m.expect("every tile is built");
        if !m.voxels.is_empty() {
            placed.push(*t);
            models.push(m);
        }
    }
    let mut nodes = std::collections::BTreeMap::new();
    nodes.insert(
        0,
        Node::Transform {
            child: 1,
            translation: [0; 3],
            rot: Rot::IDENTITY,
        },
    );
    nodes.insert(
        1,
        Node::Group {
            children: (0..models.len() as u32).map(|k| 2 + 2 * k).collect(),
        },
    );
    for (k, t) in placed.iter().enumerate() {
        let id = 2 + 2 * k as u32;
        nodes.insert(
            id,
            Node::Transform {
                child: id + 1,
                translation: [0, 1, 2].map(|a| t.lo[a] + (t.size[a] / 2) as i32),
                rot: Rot::IDENTITY,
            },
        );
        nodes.insert(
            id + 1,
            Node::Shape {
                models: vec![k as u32],
            },
        );
    }
    let mut palette = vxe_assets::DEFAULT_PALETTE;
    palette[1..=8].copy_from_slice(&PALETTE_COLOURS);
    VoxFile {
        version: 200,
        models,
        palette,
        nodes,
        imap_present: false,
    }
}

/// [`castle_vox_on`] on the machine's available threads.
pub fn castle_vox() -> VoxFile {
    castle_vox_on(std::thread::available_parallelism().map_or(1, usize::from))
}

/// How many voxels of each palette index the file's models list.
pub fn index_counts(file: &VoxFile) -> [u64; 256] {
    let mut n = [0u64; 256];
    for m in &file.models {
        for v in &m.voxels {
            n[usize::from(v[3])] += 1;
        }
    }
    n
}

/// The map for the tiles above the floor: index 0 leaves the hall's air alone, 1 to 7 are the
/// castle's materials. Index 8 is left unset, so an explicit air voxel above the floor is an
/// error.
pub fn above_map() -> VoxMap {
    let mut m = VoxMap::new();
    m.set(0, Slot::Skip);
    for i in STONE..=TORCH {
        m.set(i, Slot::Material(material(i)));
    }
    m
}

/// The map for the dungeon's tiles: [`above_map`] with index 8 as air.
pub fn dungeon_map() -> VoxMap {
    let mut m = above_map();
    m.set(EXPLICIT_AIR, Slot::Air);
    m
}

/// The castle's placements in the world: each placed model as a prefab through
/// [`dungeon_map`] when its box lies below the floor and [`above_map`] otherwise, at the
/// world corner [`ORIGIN`] plus the placement's minimum corner, in placement order.
pub fn castle_prefabs(file: &VoxFile) -> Result<Vec<(Arc<Prefab>, VoxelCoord, Rot)>, Unmapped> {
    let (above, dungeon) = (above_map(), dungeon_map());
    file.placements()
        .into_iter()
        .map(|p| {
            let m = &file.models[p.model];
            let corner = p.min_corner(p.rot.apply_size(m.size)).0;
            let map = if corner[2] < 0 { &dungeon } else { &above };
            let prefab = map.to_prefab(m)?;
            let at = VoxelCoord([0, 1, 2].map(|k| ORIGIN[k] + corner[k]));
            Ok((Arc::new(prefab), at, p.rot))
        })
        .collect()
}

/// The hall in the cave of `seed` with the castle's placements written over it. Each `Arc` is
/// moved in, so a prefab's unrotated copy is freed as its rotated one is made.
pub fn castle_source(
    seed: u64,
    placements: Vec<(Arc<Prefab>, VoxelCoord, Rot)>,
) -> StampedSource<HallSource> {
    StampedSource::new(HallSource::new(seed), placements)
}

/// The bytes the kept prefabs hold: per prefab of `n` voxels, `2 n` of material and
/// `8 * ceil(n / 64)` of mask.
pub fn prefab_bytes<'a>(prefabs: impl IntoIterator<Item = &'a Prefab>) -> u64 {
    prefabs
        .into_iter()
        .map(|p| {
            let n = p.len() as u64;
            2 * n + 8 * n.div_ceil(64)
        })
        .sum()
}

/// A named camera pose in the castle, for `--castle-shot` and the shot key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastleShot {
    /// The spawn pose, facing the gate across the hall.
    Gate,
    /// Inside the keep, looking west along the vault at the rose window.
    Nave,
    /// Outside, 6 m from a lancet and its buttresses.
    Window,
    /// In the dungeon's corridor, looking along the cells.
    Dungeon,
}

impl CastleShot {
    pub const ALL: [Self; 4] = [Self::Gate, Self::Nave, Self::Window, Self::Dungeon];

    pub fn name(self) -> &'static str {
        match self {
            Self::Gate => "gate",
            Self::Nave => "nave",
            Self::Window => "window",
            Self::Dungeon => "dungeon",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    /// The next shot, wrapping after the last.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&s| s == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// The eye's world voxel (the camera sits at its centre), the yaw (0 looks along +x, 90
    /// along +y) and the pitch (up positive), in whole degrees.
    pub fn pose(self) -> ([i32; 3], i32, i32) {
        let at = |l: [i32; 3]| [0, 1, 2].map(|k| ORIGIN[k] + l[k]);
        match self {
            Self::Gate => (crate::cave::UNBOUNDED_SPAWN.0, 0, 0),
            Self::Nave => (at([470, NAVE_V, 100]), 180, 12),
            Self::Window => (at([360, 160, 100]), 90, 0),
            Self::Dungeon => (at([260, 450, FLOOR + 15]), 0, 0),
        }
    }
}
