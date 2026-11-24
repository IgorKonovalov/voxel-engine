//! The light placement's tests: the indexes and the write walk's stop against the scan and
//! the walk of every wanted chunk.

use super::*;

/// One run of the recorded sequence on [`Ranges`], by the index or (`linear`) by the walk
/// and the scan: per step the placed ranges, the free ranges and the high-water mark, the
/// buffer's words (each placed sample holds its chunk's tag, a released one 0), and the
/// placements and evictions.
fn replay(linear: bool) -> (Vec<String>, Vec<u32>, u64, u64) {
    const SAMPLES: u32 = 24_000;
    let mut r = Ranges::new(SAMPLES);
    r.linear = linear;
    let mut words = vec![0u32; SAMPLES as usize];
    let mut st = SyncStats::default();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut rand = move |m: u64| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) % m
    };
    let tag = |c: ChunkCoord| ((c.0[0] + 64) as u32) << 16 | ((c.0[2] + 64) as u32) << 4 | 1;
    let mut log = Vec::new();
    let (mut placed, mut evictions) = (0u64, 0u64);
    for step in 0..2_600u64 {
        let focus = ChunkCoord([(step / 40) as i32, 0, 0]);
        let c = ChunkCoord([
            focus.0[0] + rand(13) as i32 - 6,
            rand(3) as i32 - 1,
            rand(13) as i32 - 6,
        ]);
        if rand(10) == 0 {
            // A dropped chunk: forgotten now, freed at the next sync.
            if let Some((s, n)) = r.forget(c) {
                words[s as usize..(s + n) as usize].fill(0);
                r.free_range((s, n));
            }
            continue;
        }
        if let Some((s, n)) = r.release(c) {
            words[s as usize..(s + n) as usize].fill(0);
        }
        let n = 1 + rand(400) as u32;
        let mut evicted = Vec::new();
        let at = r.place(c, n, c.distance2(focus), focus, &mut evicted, &mut st);
        for (_, (s, n)) in &evicted {
            words[*s as usize..(*s + *n) as usize].fill(0);
        }
        evictions += evicted.len() as u64;
        if let Some(s) = at {
            words[s as usize..(s + n) as usize].fill(tag(c));
            placed += 1;
        }
        let mut held: Vec<_> = r.placed().collect();
        held.sort_unstable_by_key(|&(c, _)| c.0);
        log.push(format!(
            "{step}: {at:?} evicted {evicted:?} held {held:?} free {:?} high {}",
            r.free_ranges(),
            r.high()
        ));
    }
    (log, words, placed, evictions)
}

/// Plan 0096 Phase 7 bar 1: a recorded sequence of placements, releases and drops with a
/// moving focus (2 600 steps, at least 2 000 placements and 300 evictions) gives the same
/// ranges, free ranges and buffer words at every step through the first-fit index and the
/// victims' index as through the walk of every free range and the scan of every placed
/// chunk.
#[test]
fn the_indexed_placement_writes_the_same_bytes_as_the_scan() {
    let (walk, walk_words, placed, evictions) = replay(true);
    let (index, index_words, ..) = replay(false);
    println!("{placed} placements, {evictions} evictions");
    assert!(
        placed >= 2_000 && evictions >= 300,
        "{placed} placed, {evictions} evicted"
    );
    for (a, b) in walk.iter().zip(&index) {
        assert_eq!(a, b);
    }
    assert_eq!(walk.len(), index.len());
    assert_eq!(walk_words, index_words);

    // The refusing phase (Plan 0096 Phase 7b): write walks over a buffer of about a tenth
    // of the demand, through the indexes and the stop at the first refusal against the
    // scan and the walk of every wanted chunk.
    let full = refusing_replay(true, true);
    let ended = refusing_replay(false, false);
    println!(
        "refusing: {} placed, {} refused ({} without a place call), {} evicted, {} rewrites \
         after a refusal, {} / {} place calls, demand {} in {} samples",
        ended.placed,
        ended.refused,
        ended.skipped,
        ended.evictions,
        ended.rewrites_after_refusal,
        ended.places,
        full.places,
        ended.demand,
        REFUSING_SAMPLES,
    );
    assert!(
        ended.refused > ended.placed && ended.evictions > 0,
        "{} refused, {} placed, {} evicted",
        ended.refused,
        ended.placed,
        ended.evictions
    );
    assert!(ended.demand >= 8 * u64::from(REFUSING_SAMPLES));
    assert!(ended.rewrites_after_refusal > 0 && ended.skipped > 0);
    assert!(ended.places < full.places);
    assert_eq!(full.skipped, 0);
    assert_eq!(full.refused, ended.refused);
    for (a, b) in full.log.iter().zip(&ended.log) {
        assert_eq!(a, b);
    }
    assert_eq!(full.log.len(), ended.log.len());
    assert_eq!(full.words, ended.words);
}

/// [`WriteWalk`] over [`Ranges`] and a word per sample, as a sync's walk runs on the
/// mirror: each chunk's thinned count in `lists` (0 for empty lists), the dirty and waiting
/// sets, and the place calls and refusals counted. With `full` every count fits, so the walk
/// tries every wanted chunk (the reference).
struct Mock {
    r: Ranges,
    lists: HashMap<ChunkCoord, u32>,
    dirty: HashSet<ChunkCoord>,
    wait: HashSet<ChunkCoord>,
    words: Vec<u32>,
    focus: ChunkCoord,
    st: SyncStats,
    full: bool,
    places: u64,
    placed: u64,
    refused: u64,
    skipped: u64,
    evictions: u64,
    refused_this_walk: bool,
    rewrites_after_refusal: u64,
}

fn tag(c: ChunkCoord) -> u32 {
    ((c.0[0] + 64) as u32) << 16 | ((c.0[1] + 64) as u32) << 8 | ((c.0[2] + 64) as u32) << 1 | 1
}

impl Mock {
    fn new(samples: u32, linear: bool, full: bool) -> Self {
        let mut r = Ranges::new(samples);
        r.linear = linear;
        Self {
            r,
            lists: HashMap::new(),
            dirty: HashSet::new(),
            wait: HashSet::new(),
            words: vec![0; samples as usize],
            focus: ChunkCoord([0; 3]),
            st: SyncStats::default(),
            full,
            places: 0,
            placed: 0,
            refused: 0,
            skipped: 0,
            evictions: 0,
            refused_this_walk: false,
            rewrites_after_refusal: 0,
        }
    }

    fn release(&mut self, c: ChunkCoord) -> bool {
        let Some((s, n)) = self.r.release(c) else {
            return false;
        };
        self.words[s as usize..(s + n) as usize].fill(0);
        true
    }

    /// One sync's walks, as `place_lights` runs them: ranges beyond `radius + 1` released,
    /// then the write walk over the chunks within `radius` that hold no range or are dirty.
    fn sync(&mut self, radius: i64, budget: u64) {
        let f = self.focus;
        let mut far: Vec<ChunkCoord> = self
            .r
            .placed()
            .map(|(c, _)| c)
            .filter(|c| c.distance2(f) > (radius + 1) * (radius + 1))
            .collect();
        far.sort_unstable_by_key(|c| c.0);
        for c in far {
            self.release(c);
        }
        let mut wanted: Vec<Wanted> = self
            .lists
            .iter()
            .filter(|(c, _)| c.distance2(f) <= radius * radius)
            .map(|(c, &n)| (*c, n, self.r.get(*c).is_some()))
            .filter(|&(c, _, placed)| !placed || self.dirty.contains(&c))
            .map(|(c, n, placed)| Wanted {
                d2: c.distance2(f),
                coord: c,
                placed,
                n,
            })
            .collect();
        wanted.sort_unstable_by_key(|w| (w.d2, w.coord.0));
        self.refused_this_walk = false;
        write_walk(&wanted, budget, self);
    }

    /// The placed ranges, free ranges, high-water mark, waiting and dirty chunks, the
    /// refusals and a hash of the words.
    fn state(&self) -> String {
        let mut held: Vec<_> = self.r.placed().collect();
        held.sort_unstable_by_key(|&(c, _)| c.0);
        let mut wait: Vec<_> = self.wait.iter().map(|c| c.0).collect();
        wait.sort_unstable();
        let mut dirty: Vec<_> = self.dirty.iter().map(|c| c.0).collect();
        dirty.sort_unstable();
        let hash = self.words.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, &w| {
            (h ^ u64::from(w)).wrapping_mul(0x100_0000_01B3)
        });
        format!(
            "held {held:?} free {:?} high {} wait {wait:?} dirty {dirty:?} refused {} \
             words {hash:x}",
            self.r.free_ranges(),
            self.r.high(),
            self.refused,
        )
    }
}

impl WriteWalk for Mock {
    fn write(&mut self, coord: ChunkCoord, d2: i64) -> Option<u64> {
        let rewrite = self.release(coord);
        let Some(&n) = self.lists.get(&coord) else {
            return Some(0);
        };
        if n == 0 {
            self.lists.remove(&coord);
            return Some(0);
        }
        if rewrite && self.refused_this_walk {
            self.rewrites_after_refusal += 1;
        }
        self.places += 1;
        let mut evicted = Vec::new();
        let at = self
            .r
            .place(coord, n, d2, self.focus, &mut evicted, &mut self.st);
        for (v, (s, len)) in evicted {
            self.words[s as usize..(s + len) as usize].fill(0);
            self.wait.insert(v);
            self.evictions += 1;
        }
        let Some(s) = at else {
            self.refused += 1;
            self.refused_this_walk = true;
            return None;
        };
        self.words[s as usize..(s + n) as usize].fill(tag(coord));
        self.placed += 1;
        Some(u64::from(n))
    }

    fn fits(&self, n: u32) -> bool {
        self.full || self.r.fits(n)
    }

    fn refuse(&mut self, _: ChunkCoord) {
        self.refused += 1;
        self.skipped += 1;
    }

    fn wait(&mut self, coord: ChunkCoord) {
        self.wait.insert(coord);
    }

    fn walked(&mut self, coord: ChunkCoord) {
        self.dirty.remove(&coord);
        self.wait.remove(&coord);
    }
}

/// The refusing phase's buffer: about a tenth of the demand within the walk's radius.
const REFUSING_SAMPLES: u32 = 1_600;

/// What [`refusing_replay`] returns: the per-sync states, the final words and the counts.
struct Refusing {
    log: Vec<String>,
    words: Vec<u32>,
    places: u64,
    placed: u64,
    refused: u64,
    skipped: u64,
    evictions: u64,
    rewrites_after_refusal: u64,
    demand: u64,
}

/// 600 syncs over a field of chunks (a thinned count of 1 to 400 each, one in 32 with
/// empty lists) with the focus moving a chunk every 4 syncs, the walk's radius 3, a budget
/// of 3 000 samples a sync: before each sync, 3 placed chunks and 1 other chunk get new
/// counts and are marked dirty.
fn refusing_replay(linear: bool, full: bool) -> Refusing {
    let mut m = Mock::new(REFUSING_SAMPLES, linear, full);
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut rand = move |k: u64| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) % k
    };
    let count = |rand: &mut dyn FnMut(u64) -> u64| {
        if rand(32) == 0 {
            0
        } else {
            1 + rand(400) as u32
        }
    };
    for x in -4..160 {
        for y in -1..=1 {
            for z in -4..=4 {
                let n = count(&mut rand);
                m.lists.insert(ChunkCoord([x, y, z]), n);
            }
        }
    }
    let demand: u64 = m
        .lists
        .iter()
        .filter(|(c, _)| c.distance2(ChunkCoord([0; 3])) <= 9)
        .map(|(_, &n)| u64::from(n))
        .sum();
    let mut log = Vec::new();
    for step in 0..600 {
        m.focus = ChunkCoord([step / 4, 0, 0]);
        let mut held: Vec<ChunkCoord> = m.r.placed().map(|(c, _)| c).collect();
        held.sort_unstable_by_key(|c| c.0);
        for _ in 0..3 {
            if held.is_empty() {
                break;
            }
            let c = held[rand(held.len() as u64) as usize];
            let n = count(&mut rand);
            m.lists.insert(c, n);
            m.dirty.insert(c);
        }
        let c = ChunkCoord([
            m.focus.0[0] + rand(7) as i32 - 3,
            rand(3) as i32 - 1,
            rand(7) as i32 - 3,
        ]);
        let n = count(&mut rand);
        m.lists.insert(c, n);
        m.dirty.insert(c);
        m.sync(3, 3_000);
        log.push(format!("{step}: {}", m.state()));
    }
    Refusing {
        log,
        words: m.words,
        places: m.places,
        placed: m.placed,
        refused: m.refused,
        skipped: m.skipped,
        evictions: m.evictions,
        rewrites_after_refusal: m.rewrites_after_refusal,
        demand,
    }
}

/// Places `n` samples of `c` on the mock's buffer outside a walk.
fn preplace(m: &mut Mock, c: ChunkCoord, n: u32) {
    m.lists.insert(c, n);
    let mut evicted = Vec::new();
    let at =
        m.r.place(c, n, c.distance2(m.focus), m.focus, &mut evicted, &mut m.st);
    assert!(at.is_some() && evicted.is_empty());
}

/// One walk (radius 12) on a full buffer of 1 000 samples with no free run: five placed
/// chunks of 200 samples each between 11 and 13 chunks out, ten wanted chunks of 200 at 1
/// to 10 chunks along +z (with `dirty`, the fifth far chunk's place taken by a placed chunk
/// at the fifth wanted chunk's distance, ordered after it, rewritten at `dirty` samples).
/// With `u6_small` the sixth wanted chunk takes 100 samples. The place calls, refusals,
/// refusals without a place call, and the end state.
fn full_buffer_walk(full: bool, dirty: Option<u32>, u6_small: bool) -> (u64, u64, u64, String) {
    let mut m = Mock::new(1_000, false, full);
    let far = [[0, 0, 11], [1, 0, 11], [0, 0, 12], [1, 0, 12], [0, 1, 12]].map(ChunkCoord);
    let d = ChunkCoord([5, 0, 0]);
    for (i, c) in far.iter().enumerate() {
        // With a dirty chunk, it takes the last far chunk's place.
        if dirty.is_some() && i == 4 {
            preplace(&mut m, d, 200);
        } else {
            preplace(&mut m, *c, 200);
        }
    }
    assert!(!m.r.fits(1));
    for i in 1..=10 {
        let n = if u6_small && i == 6 { 100 } else { 200 };
        m.lists.insert(ChunkCoord([0, 0, i]), n);
    }
    if let Some(n) = dirty {
        m.lists.insert(d, n);
        m.dirty.insert(d);
    }
    m.sync(12, u64::MAX);
    let state = m.state();
    (m.places, m.refused, m.skipped, state)
}

/// Plan 0096 Phase 7b bar 2: on a full buffer with no free run, a walk of N = 10 wanted
/// chunks whose first K fit (each by evicting a far chunk) calls `place` K + 1 times and
/// counts N - K refused; a dirty placed chunk after the refusal is rewritten (one more call)
/// and the walk places again only a chunk that the run its rewrite left holds. Each walk's end
/// state equals the walk of every wanted chunk.
#[test]
fn a_refused_walk_ends_at_the_first_refusal_it_cannot_undo() {
    let (places, refused, skipped, state) = full_buffer_walk(false, None, false);
    let (all, all_refused, _, all_state) = full_buffer_walk(true, None, false);
    // K = 5: five far chunks to evict.
    assert_eq!((places, refused, skipped), (5 + 1, 10 - 5, 10 - 5 - 1));
    assert_eq!((all, all_refused), (10, 5));
    assert_eq!(state, all_state);

    // A dirty placed chunk at the fifth's distance, rewritten at its size: K = 4 (four far
    // chunks), the fifth refused, the rewrite fits back in its own range and leaves no run,
    // so the sixth to tenth are refused without a place call.
    let (places, refused, skipped, state) = full_buffer_walk(false, Some(200), false);
    let (all, _, _, all_state) = full_buffer_walk(true, Some(200), false);
    assert_eq!((places, refused, skipped), (4 + 1 + 1, 10 - 4, 10 - 4 - 1));
    assert_eq!(all, 11);
    assert_eq!(state, all_state);

    // Rewritten at 100 samples: its release leaves a run of 100, so the walk places again
    // the sixth (100 samples), which fits it; the seventh to tenth (200) are refused without a
    // place call.
    let (places, refused, skipped, state) = full_buffer_walk(false, Some(100), true);
    let (all, _, _, all_state) = full_buffer_walk(true, Some(100), true);
    assert_eq!((places, refused, skipped), (4 + 1 + 1 + 1, 10 - 4 - 1, 4));
    assert_eq!(all, 11);
    assert_eq!(state, all_state);
}
