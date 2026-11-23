use super::*;
use crate::bake::faces::{CLASS_TRANSLUCENT, list_faces_by};
use vxe_world::{AIR, Brick, Chunk, MaterialId};

const ROCK: MaterialId = 1;
const GLASS: MaterialId = 3;

fn classes() -> Vec<u8> {
    let mut c = vec![CLASS_AIR; 1 << 16];
    c[usize::from(ROCK)] = CLASS_OPAQUE;
    c[usize::from(GLASS)] = CLASS_TRANSLUCENT;
    c
}

/// A chunk whose voxel at chunk-local `l` is `f(l)`.
fn chunk_by(f: impl Fn([i32; 3]) -> MaterialId) -> Chunk {
    let mut c = Chunk::uniform(AIR);
    for slot in 0..CHUNK_BRICKS {
        let b = brick_of_slot(slot);
        c.set_brick(
            slot,
            Brick::from_fn(|l| f([0, 1, 2].map(|k| b[k] * 8 + l[k]))),
        );
    }
    c
}

/// Whether `own` lists no face with `beside` its neighbours (`None` read as air), by the
/// listing itself.
fn listed_none(own: &ChunkMasks, beside: &[Option<ChunkMasks>; 6]) -> bool {
    let air = ([0u64; 8], [0u64; 8]);
    list_faces_by(&|slot| own.get(slot), &|face, slot| {
        Some(beside[face].as_ref().map_or(air, |m| m.get(slot)))
    })
    .bricks
    .is_empty()
}

/// The verdict and the listing agree on a chunk and its neighbours, every neighbour present and
/// with each one absent in turn; returns the verdict with all present.
fn agree(own: &Chunk, beside: &[Chunk; 6]) -> bool {
    let cl = classes();
    let m = ChunkMasks::of(own, &cl);
    let near: [ChunkMasks; 6] = std::array::from_fn(|f| ChunkMasks::of(&beside[f], &cl));
    let shells: [Shell; 6] = std::array::from_fn(|f| Shell::of(&near[f]));
    let shell = Shell::of(&m);
    let all = shell.sealed(|f| Some(&shells[f]));
    assert_eq!(
        all,
        listed_none(&m, &std::array::from_fn(|f| Some(near[f].clone()))),
        "all present"
    );
    for gone in 0..6 {
        let v = shell.sealed(|f| (f != gone).then(|| &shells[f]));
        let l = listed_none(
            &m,
            &std::array::from_fn(|f| (f != gone).then(|| near[f].clone())),
        );
        assert_eq!(v, l, "neighbour {gone} absent");
    }
    all
}

fn rock() -> Chunk {
    Chunk::uniform(ROCK)
}

/// The no-face verdict from the shells (a chunk's inner faces and its six border layers against
/// its neighbours') lists the same as the listing itself across structured chunks: rock in rock,
/// air, a ground at several heights, a hole on each border layer, a glass seam, glass in glass.
#[test]
fn the_shell_verdict_agrees_with_the_listing() {
    let rocks: [Chunk; 6] = std::array::from_fn(|_| rock());
    assert!(agree(&rock(), &rocks), "rock in rock is sealed");
    assert!(agree(&Chunk::uniform(AIR), &rocks), "air is sealed");
    let airs: [Chunk; 6] = std::array::from_fn(|_| Chunk::uniform(AIR));
    assert!(!agree(&rock(), &airs), "rock in air lists faces");
    for h in [1, 31, 63] {
        let ground = chunk_by(|l| if l[2] < h { ROCK } else { AIR });
        assert!(!agree(&ground, &rocks), "a ground at {h} lists its top");
    }
    // Rock with one air voxel on a border layer: an inner face whatever the neighbours.
    for (face, at) in [
        (0, [0, 17, 40]),
        (1, [63, 5, 9]),
        (2, [33, 0, 63]),
        (3, [12, 63, 0]),
        (4, [63, 63, 0]),
        (5, [0, 0, 63]),
    ] {
        let holed = chunk_by(|l| if l == at { AIR } else { ROCK });
        assert!(!agree(&holed, &rocks), "a hole on layer {face}");
        // The same hole in the neighbour across that face opens the rock chunk's border.
        let mut near: [Chunk; 6] = std::array::from_fn(|_| rock());
        let mut o = at;
        o[face / 2] = 63 - o[face / 2];
        near[face] = chunk_by(|l| if l == o { AIR } else { ROCK });
        assert!(!agree(&rock(), &near), "a hole across face {face}");
    }
    // Rock beside a neighbour whose touching layer is glass: rock faces glass.
    let mut seam: [Chunk; 6] = std::array::from_fn(|_| rock());
    seam[3] = chunk_by(|l| if l[1] == 0 { GLASS } else { ROCK });
    assert!(!agree(&rock(), &seam), "rock against glass");
    // Glass in glass and glass against rock list nothing; glass against air does.
    let glass = Chunk::uniform(GLASS);
    let glasses: [Chunk; 6] = std::array::from_fn(|_| Chunk::uniform(GLASS));
    assert!(agree(&glass, &glasses), "glass in glass");
    assert!(agree(&glass, &rocks), "glass in rock");
    assert!(!agree(&glass, &airs), "glass in air");
    // A chunk of rock with a glass core: the glass touches only rock and glass.
    let core = chunk_by(|l| {
        if l.iter().all(|&v| (20..40).contains(&v)) {
            GLASS
        } else {
            ROCK
        }
    });
    assert!(
        !agree(&core, &rocks),
        "rock against its glass core lists faces"
    );
}

fn shell_of(c: &Chunk) -> Shell {
    Shell::of(&ChunkMasks::of(c, &classes()))
}

/// The shells' order: a rock chunk is not sealed while a neighbour is missing (open), turns
/// sealed in the verdicts of the arrival that closes its last border, opens again when a
/// neighbour arrives with a hole on the touching layer, and an edit's forget clears the
/// verdicts near it at a later point of the order.
#[test]
fn a_neighbours_arrival_seals_or_opens_a_chunk_in_order() {
    let mut s = Shells::default();
    let c = [5, 5, 5];
    let v = s.admit(c, shell_of(&rock()));
    assert_eq!(v.len(), 1);
    assert!(!v[0].sealed, "every neighbour missing reads open");
    let mut last = v[0].seq;
    for f in 0..6 {
        let v = s.admit(beside(c, f), shell_of(&rock()));
        assert!(v[0].seq > last);
        last = v[0].seq;
        let on_c: Vec<&Verdict> = v.iter().filter(|x| x.c == c).collect();
        if f < 5 {
            assert!(on_c.is_empty(), "no change on {c:?} at arrival {f}");
        } else {
            assert_eq!(on_c.len(), 1, "the last border closes");
            assert!(on_c[0].sealed);
        }
    }
    // +z arrives again with an air voxel against `c`: `c` opens.
    let holed = chunk_by(|l| if l == [7, 7, 0] { AIR } else { ROCK });
    let v = s.admit(beside(c, 5), shell_of(&holed));
    let on_c: Vec<&Verdict> = v.iter().filter(|x| x.c == c).collect();
    assert_eq!(on_c.len(), 1);
    assert!(!on_c[0].sealed, "the hole opens the border");
    // Closed again, then an edit forgets `c`'s shell: a neighbour's arrival reads `c` absent
    // and emits no verdict on it.
    s.admit(beside(c, 5), shell_of(&rock()));
    let at = s.forget(&[c], &[c, beside(c, 0)]);
    assert!(at > last);
    let v = s.admit(beside(c, 1), shell_of(&rock()));
    assert!(
        v.iter().all(|x| x.c != c),
        "a forgotten shell has no verdict"
    );
    assert!(v.iter().all(|x| x.seq > at));
    s.prune(|k| k != beside(c, 1));
    assert_eq!(s.len(), 5);
}

/// The shells of a uniform chunk hold no layer bits; a mixed border layer holds its 1 KB.
#[test]
fn a_uniform_chunks_shell_holds_no_layer_bytes() {
    let mut s = Shells::default();
    s.admit([0, 0, 0], shell_of(&rock()));
    s.admit([1, 0, 0], shell_of(&Chunk::uniform(AIR)));
    let base = s.bytes();
    assert_eq!(base, 2 * std::mem::size_of::<Shell>());
    let ground = chunk_by(|l| if l[2] < 30 { ROCK } else { AIR });
    s.admit([2, 0, 0], shell_of(&ground));
    // Four side layers are mixed; the bottom is rock and the top air.
    assert_eq!(s.bytes() - base, std::mem::size_of::<Shell>() + 4 * 1024);
}
