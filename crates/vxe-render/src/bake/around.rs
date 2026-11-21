//! The sky over the camera's floor (ADR-0081): the `+z` sky visibility of the first baked
//! record with a `+z` face in the brick column under the eye, read from the store's CPU block
//! words, the camera's own chunk summary out of reach. Pure CPU; no `vxe-world` read.

use super::*;

/// Cells the walk goes down past the eye's own: the eye's cell and this many under it are
/// read (ADR-0081's `AROUND_REACH`, a guess).
pub const AROUND_REACH: i32 = 8;

/// The `+z` face's index in face order (`-x, +x, -y, +y, -z, +z`).
const FACE_UP: usize = 5;

/// Which answered the sky over the floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AroundSource {
    /// A brick record's cell in the column under the eye.
    Record,
    /// The eye's chunk summary: no record with a `+z` face within [`AROUND_REACH`].
    Summary,
}

impl AroundSource {
    pub fn name(self) -> &'static str {
        match self {
            Self::Record => "record",
            Self::Summary => "summary",
        }
    }
}

/// The record of world cell `c` (cells of `cell` voxels, 8 or 4) in `block`, its chunk's words
/// with the directory first: `None` where its brick has no record. The directory arithmetic
/// of `temporal.wgsl`'s `store_cell_record`.
pub(crate) fn cell_record(block: &[u32], cell: u32, c: [i32; 3]) -> Option<&[u32]> {
    let per = if cell == 4 { 1 } else { 0 };
    let b = c.map(|v| v >> per);
    let s = b.map(|v| v & 7);
    let dir = *block.get((s[0] + 8 * (s[1] + 8 * s[2])) as usize)?;
    if dir == 0 {
        return None;
    }
    let n = 1i32 << per;
    let l = c.map(|v| v & (n - 1));
    let sub = (l[0] + n * (l[1] + n * l[2])) as usize;
    let cells = (n * n * n) as usize;
    let first = (dir >> 11) as usize + BRICK_WORDS * sub;
    let first = first.checked_sub(BRICK_WORDS * cells)?;
    block.get(first..first + BRICK_WORDS)
}

/// The sky over the floor under `eye` (voxels) over blocks by chunk (`block`), summaries by
/// chunk (`summary`) and the records' cell in voxels: from the eye's cell down, the eye's and
/// [`AROUND_REACH`] cells under it, the first record whose `+z` direction is baked under a sky
/// answers with its visibility (a [`SKY_STALE`] one with its old value); else the eye's chunk
/// summary's `+z` visibility; else `None`.
pub(crate) fn floor_sky<'a>(
    block: impl Fn([i32; 3]) -> Option<&'a [u32]>,
    summary: impl Fn([i32; 3]) -> Option<&'a [u32; BRICK_WORDS]>,
    cell: u32,
    eye: [f32; 3],
) -> Option<(f32, AroundSource)> {
    if !eye.iter().all(|v| v.is_finite()) {
        return None;
    }
    let v = eye.map(|x| x.floor() as i32);
    let shift = cell.trailing_zeros();
    let c0 = v.map(|x| x >> shift);
    // Cells of a chunk a side: 64 voxels over the cell.
    let chunk_shift = 6 - shift;
    for k in 0..=AROUND_REACH {
        let c = [c0[0], c0[1], c0[2] - k];
        let Some(words) = block(c.map(|x| x >> chunk_shift)) else {
            continue;
        };
        if let Some(f) = cell_record(words, cell, c).and_then(|r| record_visibility(r, FACE_UP)) {
            return Some((f, AroundSource::Record));
        }
    }
    let chunk = v.map(|x| x >> 6);
    summary(chunk)
        .and_then(|s| record_visibility(s, FACE_UP))
        .map(|f| (f, AroundSource::Summary))
}

/// [`floor_sky`] over `store`'s CPU words and summaries (ADR-0081): the sky share over the
/// camera's floor at `eye` (voxels), and which answered.
pub fn sky_over_floor(store: &Store, eye: [f32; 3]) -> Option<(f32, AroundSource)> {
    floor_sky(
        |c| store.words.get(&c).map(Vec::as_slice),
        |c| store.light.summary(c),
        store.cell,
        eye,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A block of one brick at slot `slot` whose cells (cell `cell`) hold `records`, cell order.
    fn block(slot: usize, records: &[[u32; BRICK_WORDS]]) -> Vec<u32> {
        let mut b = vec![0u32; BLOCK_HEAD_WORDS as usize];
        for r in records {
            b.extend_from_slice(r);
        }
        b[slot] = (b.len() as u32) << 11;
        b
    }

    /// A record whose cell has the faces of `mask` and sky visibilities `vis`.
    fn record(mask: u32, vis: [f32; 6]) -> [u32; BRICK_WORDS] {
        let mut r = [0u32; BRICK_WORDS];
        r[6] = 3 | mask << 16;
        r[VIS_WORD..VIS_WORD + 2].copy_from_slice(&visibility_words(vis));
        r
    }

    fn read(
        blocks: &HashMap<[i32; 3], Vec<u32>>,
        summaries: &HashMap<[i32; 3], [u32; BRICK_WORDS]>,
        cell: u32,
        eye: [f32; 3],
    ) -> Option<(f32, AroundSource)> {
        floor_sky(
            |c| blocks.get(&c).map(Vec::as_slice),
            |c| summaries.get(&c),
            cell,
            eye,
        )
    }

    /// Plan 0097 Phase 1 bar 1: on hand-built blocks (cells of 8 voxels), an eye 17 voxels
    /// over a floor cell with a `+z` face at visibility 0.12 reads 0.12 within 1/255 from the
    /// record, a [`SKY_STALE`] mark keeps the value, with the floor's record missing the chunk
    /// summary's 0.5 answers, with the floor 9 cells down and no summary `None`, and a cell with
    /// faces but no `+z` face is walked past. The same at cells of 4 voxels.
    #[test]
    fn the_floor_record_under_the_eye() {
        let up = 1 << 5;
        let floor = record(up | 1, [0.9, 0.0, 0.0, 0.0, 0.0, 0.12]);
        // Brick (1, 2, 0) of chunk (0, 0, 0): its cell spans z 0..8; the eye 17 voxels over
        // the cell's top face at z 8.
        let slot = 1 + 8 * 2;
        let eye = [12.5, 20.5, 25.0];
        let mut blocks = HashMap::from([([0, 0, 0], block(slot, &[floor]))]);
        let mut summaries = HashMap::new();
        let near = |r: Option<(f32, AroundSource)>, f: f32, s: AroundSource| {
            let (v, src) = r.expect("an answer");
            assert!((v - f).abs() <= 1.0 / 255.0, "{v} against {f}");
            assert_eq!(src, s);
        };
        near(
            read(&blocks, &summaries, 8, eye),
            0.12,
            AroundSource::Record,
        );
        // Stale: the old value.
        let mut stale = floor;
        stale[VIS_WORD + 1] |= SKY_STALE;
        blocks.insert([0, 0, 0], block(slot, &[stale]));
        assert!(record_sky_stale(&stale));
        near(
            read(&blocks, &summaries, 8, eye),
            0.12,
            AroundSource::Record,
        );
        // A cell with faces but no `+z` face over the floor is walked past: brick (1, 2, 1).
        let wall = record(1 | 2, [0.4, 0.4, 0.0, 0.0, 0.0, 0.0]);
        let mut two = block(slot, &[floor]);
        let at = two.len() as u32;
        two.extend_from_slice(&wall);
        two[slot + 64] = (at + BRICK_WORDS as u32) << 11;
        blocks.insert([0, 0, 0], two);
        near(
            read(&blocks, &summaries, 8, eye),
            0.12,
            AroundSource::Record,
        );
        // A `+z` face baked with no sky is walked past too.
        let mut unbaked = floor;
        unbaked[VIS_WORD + 1] &= !VIS_BAKED;
        blocks.insert([0, 0, 0], block(slot, &[unbaked]));
        assert_eq!(read(&blocks, &summaries, 8, eye), None);
        // The floor's record missing: the chunk summary.
        blocks.insert([0, 0, 0], block(slot + 1, &[floor]));
        summaries.insert([0, 0, 0], record(up, [0.0, 0.0, 0.0, 0.0, 0.0, 0.5]));
        near(
            read(&blocks, &summaries, 8, eye),
            0.5,
            AroundSource::Summary,
        );
        // The floor 9 cells down (z 0..8 under an eye in cell 9) and no summary.
        blocks.insert([0, 0, 0], block(slot, &[floor]));
        summaries.clear();
        assert_eq!(read(&blocks, &summaries, 8, [12.5, 20.5, 72.0]), None);
        // 8 cells down is within reach; across the chunk boundary in z.
        let r = read(&blocks, &summaries, 8, [12.5, 20.5, 64.0 + 2.0]);
        near(r, 0.12, AroundSource::Record);
        // Cells of 4: eight cells a brick; the floor at cell (1, 0, 1) of brick (1, 2, 0)
        // (voxels x 12..16, y 16..20, z 4..8), every other cell without a `+z` face.
        let side = record(1, [0.3, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let mut cells = [side; 8];
        cells[1 + 4] = floor;
        blocks.insert([0, 0, 0], block(slot, &cells));
        near(
            read(&blocks, &summaries, 4, [13.0, 17.0, 25.0]),
            0.12,
            AroundSource::Record,
        );
        assert_eq!(read(&blocks, &summaries, 4, [9.0, 17.0, 25.0]), None);
    }
}
