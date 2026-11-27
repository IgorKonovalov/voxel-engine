use super::*;
use svet_world::{Chunk, Material, MaterialFlags as RowFlags};

/// Materials 1 to 24 by `(id - 1) mod 4`: emissive opaque, opaque, translucent, emissive
/// translucent; 0 is air.
fn table() -> MaterialTable {
    let mut t = MaterialTable::new();
    for id in 1..=24u16 {
        let mut m = Material::named(format!("m{id}"));
        let kind = (id - 1) % 4;
        if kind == 0 || kind == 3 {
            m.emission = [1.0, 0.5, 0.25];
        }
        if kind >= 2 {
            m.flags = RowFlags::TRANSLUCENT;
        }
        assert_eq!(t.push(m), id);
    }
    t
}

/// A seeded brick: one of the uniform kinds (air, opaque, translucent, emissive), or a `Full`
/// brick of 1 to 24 materials as noise over air, sometimes through a slab of one material
/// (runs of solid voxels, emitters buried inside), as `Brick::from_fn` encodes it.
fn brick(next: &mut impl FnMut(u64) -> u64) -> Brick {
    match next(8) {
        0 | 4 => return Brick::Uniform(AIR),
        1 => return Brick::Uniform(2 + 4 * next(6) as MaterialId),
        2 => return Brick::Uniform(3 + 4 * next(6) as MaterialId),
        3 => return Brick::Uniform(1 + 4 * next(6) as MaterialId),
        _ => {}
    }
    let n = match next(5) {
        0 => 1,
        1 => 2,
        2 => 3 + next(2) as usize,
        3 => 5 + next(12) as usize,
        _ => 17 + next(8) as usize,
    };
    let mut ids: Vec<MaterialId> = (1..=24).collect();
    for i in 0..n {
        let j = i + next((24 - i) as u64) as usize;
        ids.swap(i, j);
    }
    let mats = &ids[..n];
    let air = next(4);
    let slab = next(3) == 0;
    let (axis, cut) = (next(3) as usize, 1 + next(7) as i32);
    // Every material appears at least once, so the class follows from `n`.
    let mut first = 0usize;
    Brick::from_fn(|l| {
        if first < n {
            first += 1;
            return mats[first - 1];
        }
        if slab && l[axis] < cut {
            return mats[0];
        }
        if next(4) < air {
            AIR
        } else {
            mats[next(n as u64) as usize]
        }
    })
}

/// 64 seeded chunks, each brick of them [`brick`]: each brick's image words equal
/// `fill_slot`'s output padded to its bin, its bin equals `bin_of(slot_units)`, its entry the
/// upload's entry less the unit; the image's per-bin counts and bytes equal [`need_of`]'s for
/// a new record with no band; its emitter lists equal [`scan_emitters`]' per brick, slot by
/// slot. Between them the chunks hold every packed class, uniform air, opaque and translucent
/// bricks, bricks with a translucent voxel, and exposed emitters on brick faces and inside.
#[test]
fn an_image_holds_the_inline_paths_words_sizes_and_emitters() {
    let table = table();
    let mut flags = MaterialFlags::new();
    flags.read(&table);
    let mut z = 0x0107_0002_u64;
    let mut next = |n: u64| {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        svet_world::hash::mix64(z) % n
    };
    let mut classes = [0u32; 5];
    let (mut uni_air, mut uni_opaque, mut uni_glass, mut glassy) = (0, 0, 0, 0);
    let (mut on_face, mut inside, mut emitter_chunks) = (0, 0, 0);
    for c in 0..64 {
        let coord = ChunkCoord([0, 1, 2].map(|_| next(64) as i32 - 32));
        let mut chunk = Chunk::uniform(AIR);
        for i in 0..CHUNK_BRICKS {
            chunk.set_brick(i, brick(&mut next));
        }
        let image = SlotImage::of(coord, &chunk, &flags);
        assert_eq!(image.key, flags.key);

        let mut at = 0;
        let mut want = Vec::new();
        let mut emits = false;
        for (i, b) in chunk.bricks().iter().enumerate() {
            match b {
                Brick::Full(p) => {
                    let flag = flags.fill_slot(p, None, &mut want);
                    let k = bin_of(slot_units(p, |m| table.is_translucent(m)));
                    assert_eq!(usize::from(image.bins[i]), k, "chunk {c} brick {i} bin");
                    want.resize(BINS[k] as usize * UNIT_WORDS, u32::MAX);
                    let got = &image.words[at..at + want.len()];
                    assert_eq!(got, &want[..], "chunk {c} brick {i} words");
                    at += want.len();
                    assert_eq!(
                        image.entries[i],
                        FULL_BIT | flag | class_code(p.class()) << CLASS_SHIFT,
                        "chunk {c} brick {i} entry"
                    );
                    classes[class_code(p.class()) as usize - 1] += 1;
                    glassy += usize::from(flag != 0);
                    emits |= materials_of(p, |m| table.rows()[usize::from(m)].emission[0] > 0.0);
                }
                Brick::Uniform(m) => {
                    assert_eq!(image.bins[i], NO_BIN, "chunk {c} brick {i} bin");
                    let glass = table.is_translucent(*m);
                    let want = u32::from(*m) | if glass { TRANSLUCENT_BIT } else { 0 };
                    assert_eq!(image.entries[i], want, "chunk {c} brick {i} entry");
                    match (*m == AIR, glass) {
                        (true, _) => uni_air += 1,
                        (false, true) => uni_glass += 1,
                        (false, false) => uni_opaque += 1,
                    }
                }
            }
        }
        assert_eq!(
            at,
            image.words.len(),
            "chunk {c}: no words past the last slot"
        );

        let bricks = Bricks::Chunk(&chunk);
        let need = need_of(
            &flags,
            &bricks,
            &upload_set(CHUNK_BRICKS, true, false, |_| false),
            &[None; CHUNK_BRICKS],
            None::<fn(usize) -> u32>,
            |_| 0,
            |_| None,
        );
        assert_eq!((image.new, image.bytes), need, "chunk {c} need");

        assert_eq!(image.emits, emits, "chunk {c} emits");
        let mut lists = Vec::new();
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            let neighbour = |k: usize, dir: i32| {
                let mut n = b;
                n[k] += dir;
                (0..CHUNK_EDGE_BRICKS)
                    .contains(&n[k])
                    .then(|| chunk.brick(brick_slot(n)))
            };
            let found = scan_emitters(
                coord,
                slot,
                chunk.brick(slot),
                &flags.emissive,
                &flags.opaque,
                neighbour,
            );
            if let Some(found) = found.filter(|f| !f.is_empty()) {
                for s in &found {
                    let l = s.voxel.map(|v| v.rem_euclid(BRICK_EDGE));
                    if l.iter().any(|&v| v == 0 || v == BRICK_EDGE - 1) {
                        on_face += 1;
                    } else {
                        inside += 1;
                    }
                }
                lists.push((slot as u16, found));
            }
        }
        emitter_chunks += usize::from(!lists.is_empty());
        assert_eq!(image.lights, lists, "chunk {c} emitters");
    }
    assert!(classes.iter().all(|&n| n > 0), "classes {classes:?}");
    assert!(
        uni_air > 0 && uni_opaque > 0 && uni_glass > 0 && glassy > 0,
        "uniform air {uni_air}, opaque {uni_opaque}, translucent {uni_glass}; {glassy} glassy"
    );
    assert!(
        emitter_chunks == 64 && on_face > 0 && inside > 0,
        "{emitter_chunks} chunks with emitters, {on_face} on brick faces, {inside} inside"
    );
}

/// The key follows the flags: a table that differs in one row's translucency keys apart.
#[test]
fn the_key_names_the_flags() {
    let a = table();
    let mut rows: Vec<Material> = a.rows()[1..].to_vec();
    rows[4].flags = RowFlags::TRANSLUCENT;
    let b: MaterialTable = rows.into_iter().collect();
    assert_eq!(b.len(), a.len());
    let (mut fa, mut fb, mut fa2) = (
        MaterialFlags::new(),
        MaterialFlags::new(),
        MaterialFlags::new(),
    );
    fa.read(&a);
    fa2.read(&a);
    fb.read(&b);
    assert_eq!(fa.key, fa2.key);
    assert_ne!(fa.key, fb.key);
    assert_ne!(fa.key, MaterialFlags::new().key);
}
