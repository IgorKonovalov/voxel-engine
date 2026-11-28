//! The bake half's shadow of the mirror (ADR-0083): what the queue reads of the mirror, kept
//! from the notes the render half sends a frame, so the bake half holds no reference to it.
//!
//! The render half reads the mirror once a frame ([`notes`]): each chunk the mirror's change
//! list names, with its state now (held full, LOD-only, waiting to upload); the column of each
//! such chunk inside the window with its lowest top (a window move, in z too, is no different:
//! the column map holds the window's sheets only, dropped in the sync that moves the window,
//! which notes each dropped chunk); and each chunk the mirror's light change list names
//! ([`GpuWorld::take_light_changes`]: every site that changes a chunk's light membership, lists,
//! range or dirty mark notes it) with whether it holds lights, an exposed emitter and a current
//! range. The shadow answers the same questions the mirror does ([`Shadow::is_mirrored`] and
//! its neighbours) from those notes. An untracked list (the first frame, an overflow) reads the
//! whole, of the chunks or of the lights alone.

use super::*;
use crate::gpu_world::{boxes_within, column_is_final};

/// What the bake half reads of a chunk with lights: whether it holds an exposed emitter, and
/// whether its light-sample range is placed and clean.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LightNote {
    pub(crate) emitter: bool,
    pub(crate) current: bool,
}

/// A chunk's mirror state as the render half read it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ChunkNote {
    pub(crate) mirrored: bool,
    pub(crate) lod: bool,
    pub(crate) pending: bool,
}

/// Light notes by chunk, each once; `None` for a chunk that holds no lights.
pub(crate) type LightDelta = Vec<([i32; 3], Option<LightNote>)>;

/// One frame's notes, render half to bake half.
#[derive(Clone, Debug, Default)]
pub(crate) struct ShadowNotes {
    pub(crate) window: Option<ChunkExtent>,
    /// The window of the notes before these (the render half's last), whose columns these
    /// notes' move left; merged notes ([`ShadowNotes::then`]) carry those columns themselves.
    pub(crate) from_window: Option<ChunkExtent>,
    pub(crate) light_radius: i32,
    /// Every chunk the mirror's change list named, each once, with its state now; `None` when
    /// the list went untracked (the first frame, an overflow): `whole` holds every chunk then.
    pub(crate) changed: Option<Vec<ChunkCoord>>,
    pub(crate) chunks: Vec<([i32; 3], ChunkNote)>,
    /// The chunk columns inside the window a change touched (every one in the window when
    /// `whole`), with the lowest top the column map holds over each. A move of the window's z
    /// range adds none: the column map holds the sheets of the window's chunks only, each
    /// dropped (and its chunk noted) in the sync that moves the window, so a column's lowest top
    /// changes only with a sheet set or removed, and each of those notes its chunk.
    pub(crate) columns: Vec<([i32; 2], i32)>,
    /// The chunks whose light note changed, each once, with the note now (`None`: the chunk
    /// holds no lights); every chunk with lights when `lights_whole`. `None` when no light
    /// note changed.
    pub(crate) lights: Option<LightDelta>,
    /// `lights` holds every chunk with lights and replaces the shadow's.
    pub(crate) lights_whole: bool,
    /// The notes replace the shadow's whole state.
    pub(crate) whole: bool,
}

impl ShadowNotes {
    /// These notes followed by `newer`, as one: taking the result leaves the shadow as taking
    /// the two in order does. The changes append (each chunk once), the newer window, radius and
    /// lights win, and newer notes that replace the whole state replace these.
    pub(crate) fn then(mut self, newer: ShadowNotes) -> ShadowNotes {
        if newer.whole {
            return newer;
        }
        self.changed = match (self.changed, newer.changed) {
            (Some(mut a), Some(b)) => {
                let seen: std::collections::HashSet<ChunkCoord> = a.iter().copied().collect();
                a.extend(b.into_iter().filter(|c| !seen.contains(c)));
                Some(a)
            }
            _ => None,
        };
        // The columns a window move of these notes or of the newer ones left were dropped from
        // the mirror's column map with their chunks, and no note names them: they read empty,
        // so a later window of the merge that holds them again (a move out and back while the
        // bake half stalled) answers what the mirror does, unless a newer note names them.
        for (a, b) in [(self.from_window, self.window), (self.window, newer.window)] {
            if let (Some(a), Some(b)) = (a, b)
                && a != b
            {
                newest(&mut self.columns, columns_left(a, b));
            }
        }
        self.from_window = newer.window;
        self.window = newer.window;
        self.light_radius = newer.light_radius;
        // Each chunk's and column's newest state, once: a stalled bake half's inbox stays within
        // the chunks and columns sent.
        newest(&mut self.chunks, newer.chunks);
        newest(&mut self.columns, newer.columns);
        if let Some(b) = newer.lights {
            match &mut self.lights {
                // A whole list stays whole with the newer notes in it: the shadow clears, then
                // takes each note, a `None` removing.
                Some(a) if !newer.lights_whole => newest(a, b),
                have => {
                    *have = Some(b);
                    self.lights_whole = newer.lights_whole;
                }
            }
        }
        self
    }
}

/// The columns of window `a` outside window `b`, each read empty.
fn columns_left(a: ChunkExtent, b: ChunkExtent) -> Vec<([i32; 2], i32)> {
    let inside = |k: &[i32; 2]| (0..2).all(|i| k[i] >= b.min.0[i] && k[i] < b.max.0[i]);
    (a.min.0[1]..a.max.0[1])
        .flat_map(|y| (a.min.0[0]..a.max.0[0]).map(move |x| [x, y]))
        .filter(|k| !inside(k))
        .map(|k| (k, crate::gpu_world::COLUMN_NONE))
        .collect()
}

/// `newer`'s entries into `have`: a key already there takes the newer value in its place, a new
/// key goes at the end.
fn newest<K: Copy + Eq + std::hash::Hash, V>(have: &mut Vec<(K, V)>, newer: Vec<(K, V)>) {
    let mut at: std::collections::HashMap<K, usize> =
        have.iter().enumerate().map(|(i, (k, _))| (*k, i)).collect();
    for (k, v) in newer {
        match at.get(&k) {
            Some(&i) => have[i].1 = v,
            None => {
                at.insert(k, have.len());
                have.push((k, v));
            }
        }
    }
}

/// The mirror's reads the render half makes its notes from: [`GpuWorld`] in the frame, a model
/// in the tests.
pub(crate) trait MirrorRead {
    fn window(&self) -> ChunkExtent;
    fn light_radius(&self) -> i32;
    fn chunk_note(&self, c: ChunkCoord) -> ChunkNote;
    /// Every chunk the mirror holds a record of or waits to upload.
    fn chunks(&self) -> Vec<ChunkCoord>;
    fn column_lowest(&self, cx: i32, cy: i32) -> i32;
    /// `c`'s light note; `None` when it holds no lights.
    fn light_note(&self, c: ChunkCoord) -> Option<LightNote>;
    /// Every chunk with lights and its note.
    fn light_notes(&self) -> Vec<([i32; 3], LightNote)>;
}

/// The render half's reading of the mirror for this frame's message. `changed` is the mirror's
/// change list and `light_changed` its light change list (`None` untracked).
pub(crate) fn notes(
    m: &impl MirrorRead,
    changed: Option<Vec<ChunkCoord>>,
    light_changed: Option<Vec<ChunkCoord>>,
    last_window: Option<ChunkExtent>,
) -> ShadowNotes {
    let window = m.window();
    // A window move is a delta like any other frame's: the mirror notes every chunk it drops
    // and queues, so the change list names what the move touched. Only an untracked list (the
    // first frame, an overflow) or no notes sent yet reads the whole mirror.
    let whole = changed.is_none() || last_window.is_none();
    let (lights, lights_whole) = light_part(m, whole, light_changed);
    let all_columns = || {
        let size = window.size();
        let mut cols = Vec::new();
        for y in window.min.0[1]..window.min.0[1] + size[1] {
            for x in window.min.0[0]..window.min.0[0] + size[0] {
                cols.push(([x, y], m.column_lowest(x, y)));
            }
        }
        cols
    };
    let (chunks, columns) = if whole {
        let chunks: Vec<_> = m
            .chunks()
            .into_iter()
            .map(|c| (c.0, m.chunk_note(c)))
            .collect();
        (chunks, all_columns())
    } else {
        let ch = changed.as_deref().unwrap_or_default();
        let mut seen = ChunkSet::default();
        let mut cols = std::collections::BTreeMap::new();
        let mut chunks = Vec::with_capacity(ch.len());
        for c in ch {
            if seen.insert(c.0) {
                chunks.push((c.0, m.chunk_note(*c)));
                // A column outside the window holds no sheet, and the shadow answers it
                // `COLUMN_NONE` as the mirror does once `take` has pruned it.
                let col = [c.0[0], c.0[1]];
                if (0..2).all(|a| col[a] >= window.min.0[a] && col[a] < window.max.0[a]) {
                    cols.entry(col)
                        .or_insert_with(|| m.column_lowest(col[0], col[1]));
                }
            }
        }
        (chunks, cols.into_iter().collect())
    };
    ShadowNotes {
        window: Some(window),
        from_window: last_window,
        light_radius: m.light_radius(),
        changed,
        chunks,
        columns,
        lights,
        lights_whole,
        whole,
    }
}

/// The light half of a frame's notes: the notes of the chunks `light_changed` names, each once
/// (`None` when it names none), or every chunk with lights when the notes are `whole` or the
/// list went untracked; and whether they are the whole.
fn light_part(
    m: &impl MirrorRead,
    whole: bool,
    light_changed: Option<Vec<ChunkCoord>>,
) -> (Option<LightDelta>, bool) {
    match light_changed {
        Some(mut ch) if !whole => {
            if ch.is_empty() {
                return (None, false);
            }
            ch.sort_unstable_by_key(|c| c.0);
            ch.dedup();
            (
                Some(ch.into_iter().map(|c| (c.0, m.light_note(c))).collect()),
                false,
            )
        }
        _ => (
            Some(
                m.light_notes()
                    .into_iter()
                    .map(|(c, l)| (c, Some(l)))
                    .collect(),
            ),
            true,
        ),
    }
}

/// The bake half's copy of the mirror's state ([`ShadowNotes`]).
#[derive(Debug, Default)]
pub(crate) struct Shadow {
    window: Option<ChunkExtent>,
    light_radius: i32,
    chunks: ChunkMap<ChunkNote>,
    mirrored: usize,
    pending: usize,
    columns: std::collections::HashMap<[i32; 2], i32>,
    lights: ChunkMap<LightNote>,
}

impl Shadow {
    /// Takes a frame's notes.
    pub(crate) fn take(&mut self, n: &ShadowNotes) {
        if n.whole {
            self.chunks.clear();
            self.columns.clear();
            self.mirrored = 0;
            self.pending = 0;
        } else if let (Some(new), true) = (n.window, n.window != self.window) {
            // A moved window's leaving columns go: the map holds the window's columns only, so
            // it stays bounded however far the window travels.
            let (lo, size) = (new.min.0, new.size());
            self.columns
                .retain(|k, _| (0..2).all(|a| k[a] >= lo[a] && k[a] < lo[a] + size[a]));
        }
        self.window = n.window;
        self.light_radius = n.light_radius;
        for (c, s) in &n.chunks {
            let was = if *s == ChunkNote::default() {
                self.chunks.remove(c)
            } else {
                self.chunks.insert(*c, *s)
            }
            .unwrap_or_default();
            self.mirrored = self.mirrored + usize::from(s.mirrored) - usize::from(was.mirrored);
            self.pending = self.pending + usize::from(s.pending) - usize::from(was.pending);
        }
        // Only the window's columns: notes merged by `then` may carry an older window's.
        let inside = |k: &[i32; 2]| {
            n.window
                .is_none_or(|w| (0..2).all(|a| k[a] >= w.min.0[a] && k[a] < w.max.0[a]))
        };
        for (k, v) in n.columns.iter().filter(|(k, _)| inside(k)) {
            self.columns.insert(*k, *v);
        }
        self.take_lights(n);
    }

    fn take_lights(&mut self, n: &ShadowNotes) {
        let Some(l) = &n.lights else {
            return;
        };
        if n.lights_whole {
            self.lights.clear();
        }
        for (c, note) in l {
            match note {
                Some(note) => self.lights.insert(*c, *note),
                None => self.lights.remove(c),
            };
        }
    }

    fn note(&self, c: ChunkCoord) -> ChunkNote {
        self.chunks.get(&c.0).copied().unwrap_or_default()
    }

    pub(crate) fn window(&self) -> ChunkExtent {
        self.window.expect("a shadow that took its first notes")
    }

    pub(crate) fn light_radius(&self) -> i32 {
        self.light_radius
    }

    pub(crate) fn is_mirrored(&self, c: ChunkCoord) -> bool {
        self.note(c).mirrored
    }

    pub(crate) fn is_lod(&self, c: ChunkCoord) -> bool {
        let n = self.note(c);
        n.mirrored && n.lod
    }

    pub(crate) fn is_pending(&self, c: ChunkCoord) -> bool {
        self.note(c).pending
    }

    pub(crate) fn mirrored_chunks(&self) -> usize {
        self.mirrored
    }

    pub(crate) fn pending_chunks(&self) -> usize {
        self.pending
    }

    /// The chunks held full, in no order.
    pub(crate) fn full_chunks(&self) -> impl Iterator<Item = ChunkCoord> + '_ {
        self.chunks
            .iter()
            .filter(|(_, n)| n.mirrored && !n.lod)
            .map(|(c, _)| ChunkCoord(*c))
    }

    /// [`GpuWorld::column_lowest`] as last noted.
    pub(crate) fn column_lowest(&self, cx: i32, cy: i32) -> i32 {
        self.columns
            .get(&[cx, cy])
            .copied()
            .unwrap_or(crate::gpu_world::COLUMN_NONE)
    }

    /// [`GpuWorld::column_final`] on the shadow.
    pub(crate) fn column_final(
        &self,
        cx: i32,
        cy: i32,
        wanted: Option<&dyn Fn(ChunkCoord) -> bool>,
    ) -> bool {
        let w = self.window();
        let (min, size) = (w.min.0[2], w.size()[2].max(1));
        column_is_final(
            cx,
            cy,
            min..min + size,
            |c| self.is_pending(c),
            |c| self.is_mirrored(c),
            wanted,
        )
    }

    /// [`GpuWorld::emitters_near`] on the shadow.
    pub(crate) fn emitters_near(&self, coord: ChunkCoord, r: i32) -> bool {
        self.lights
            .iter()
            .any(|(c, l)| l.emitter && boxes_within(ChunkCoord(*c), coord, r))
    }

    /// [`GpuWorld::lights_ready_near`] on the shadow.
    pub(crate) fn lights_ready_near(&self, coord: ChunkCoord, r: i32) -> bool {
        self.lights
            .iter()
            .all(|(c, l)| l.current || !boxes_within(ChunkCoord(*c), coord, r))
    }
}

/// A shadow of a mirror's lights fed its light change list's deltas, checked against one fed
/// the whole light notes: a test's view of what the bake half reads. [`LightNotesProbe::take`]
/// takes the mirror's light changes, so nothing else (a frame's bake) may read that mirror.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct LightNotesProbe {
    shadow: Shadow,
}

impl LightNotesProbe {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes `gpu`'s light changes into the shadow, as a frame's notes do: the chunks the delta
    /// named, or `None` when the list was untracked and the whole was read.
    pub fn take(&mut self, gpu: &mut GpuWorld) -> Option<usize> {
        let changed = gpu.take_light_changes();
        let (lights, lights_whole) = light_part(&*gpu, false, changed);
        let named = (!lights_whole).then(|| lights.as_ref().map_or(0, Vec::len));
        self.shadow.take_lights(&ShadowNotes {
            lights,
            lights_whole,
            ..ShadowNotes::default()
        });
        named
    }

    /// Every disagreement over the chunks of `at` at each reach of `reaches`: the shadow's
    /// `emitters_near` and `lights_ready_near` against a shadow fed `gpu`'s whole light notes,
    /// and against `gpu`'s own `emitters_near` and `lights_ready_near_uncached`.
    pub fn mismatches(&self, gpu: &GpuWorld, at: ChunkExtent, reaches: &[i32]) -> Vec<String> {
        let mut whole = Shadow::default();
        let (lights, lights_whole) = light_part(gpu, true, None);
        whole.take_lights(&ShadowNotes {
            lights,
            lights_whole,
            ..ShadowNotes::default()
        });
        let mut out = Vec::new();
        if whole.lights != self.shadow.lights {
            out.push(format!(
                "the held notes: {} against the whole's {}",
                self.shadow.lights.len(),
                whole.lights.len()
            ));
        }
        for x in at.min.0[0]..at.max.0[0] {
            for y in at.min.0[1]..at.max.0[1] {
                for z in at.min.0[2]..at.max.0[2] {
                    let c = ChunkCoord([x, y, z]);
                    for &r in reaches {
                        let near = self.shadow.emitters_near(c, r);
                        let ready = self.shadow.lights_ready_near(c, r);
                        let want = [
                            (whole.emitters_near(c, r), whole.lights_ready_near(c, r)),
                            (
                                gpu.emitters_near(c, r),
                                gpu.lights_ready_near_uncached(c, r),
                            ),
                        ];
                        for (by, w) in ["whole", "mirror"].iter().zip(want) {
                            if (near, ready) != w {
                                out.push(format!(
                                    "{:?} at {r}: emitters near, ready {:?} against the {by}'s {w:?}",
                                    c.0,
                                    (near, ready)
                                ));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// The chunks with lights the shadow holds.
    pub fn len(&self) -> usize {
        self.shadow.lights.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shadow.lights.is_empty()
    }
}

impl MirrorRead for GpuWorld {
    fn window(&self) -> ChunkExtent {
        GpuWorld::window(self)
    }

    fn light_radius(&self) -> i32 {
        GpuWorld::light_radius(self)
    }

    fn chunk_note(&self, c: ChunkCoord) -> ChunkNote {
        ChunkNote {
            mirrored: self.is_mirrored(c),
            lod: self.is_lod(c),
            pending: self.is_pending(c),
        }
    }

    fn chunks(&self) -> Vec<ChunkCoord> {
        self.noted_chunks()
    }

    fn column_lowest(&self, cx: i32, cy: i32) -> i32 {
        GpuWorld::column_lowest(self, cx, cy)
    }

    fn light_note(&self, c: ChunkCoord) -> Option<LightNote> {
        GpuWorld::light_note(self, c)
    }

    fn light_notes(&self) -> Vec<([i32; 3], LightNote)> {
        GpuWorld::light_notes(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A mirror model: chunk states, a top a chunk, lights, and the two change lists a mirror
    /// keeps.
    #[derive(Default)]
    struct Model {
        window: Option<ChunkExtent>,
        chunks: BTreeMap<[i32; 3], (ChunkNote, i32)>,
        lights: BTreeMap<[i32; 3], LightNote>,
        changed: Vec<ChunkCoord>,
        light_changed: Vec<ChunkCoord>,
    }

    impl MirrorRead for Model {
        fn window(&self) -> ChunkExtent {
            self.window.expect("set")
        }
        fn light_radius(&self) -> i32 {
            2
        }
        fn chunk_note(&self, c: ChunkCoord) -> ChunkNote {
            self.chunks.get(&c.0).map(|s| s.0).unwrap_or_default()
        }
        fn chunks(&self) -> Vec<ChunkCoord> {
            self.chunks.keys().map(|c| ChunkCoord(*c)).collect()
        }
        /// Over every held chunk of the column, as the column map's sheets are: the model, like
        /// the mirror, drops a chunk the window leaves in the move itself, so no z range applies.
        fn column_lowest(&self, cx: i32, cy: i32) -> i32 {
            self.chunks
                .iter()
                .filter(|(c, s)| c[0] == cx && c[1] == cy && s.0.mirrored)
                .map(|(_, s)| s.1)
                .min()
                .unwrap_or(crate::gpu_world::COLUMN_NONE)
        }
        fn light_note(&self, c: ChunkCoord) -> Option<LightNote> {
            self.lights.get(&c.0).copied()
        }
        fn light_notes(&self) -> Vec<([i32; 3], LightNote)> {
            self.lights.iter().map(|(c, l)| (*c, *l)).collect()
        }
    }

    /// A seeded generator (a 64-bit LCG): the sequence is the same run to run.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % n
        }
    }

    fn extent(min: [i32; 3]) -> ChunkExtent {
        ChunkExtent::new(min, [min[0] + 8, min[1] + 8, min[2] + 4])
    }

    /// Done when (3) of Plan 0099 Phase 2, as amended, and of Plan 0111 Phase 6a'': over a
    /// recorded sequence of 2 000 mirror changes (arrivals, LOD swaps, evictions, window moves,
    /// uploads queued and done, digs that lower a column's top; light ranges written or moved,
    /// dirtied or evicted, dropped, and lists emptied, each sent as a delta of the chunks the
    /// light change list names), the shadow's held-full, LOD-only, pending, column-final,
    /// lowest-top, emitters-near and lights-ready answers equal the mirror's after every
    /// message. Every ninth message is three frames' notes merged by `then` (a stalled bake
    /// half), and every thirteenth frame's light list is untracked (an overflow: the lights
    /// read whole while the chunks stay a delta).
    #[test]
    fn the_shadow_membership_answers_as_the_mirror_does() {
        let mut rng = Lcg(0x0099);
        let mut m = Model {
            window: Some(extent([0, 0, 0])),
            ..Model::default()
        };
        let mut shadow = Shadow::default();
        let mut last_window = None;
        let mut changes = 0usize;
        let mut kinds = [0usize; 10];
        let (mut frame, mut messages) = (0u64, 0u64);
        let (mut deltas, mut light_wholes, mut stalls) = (0usize, 0usize, 0usize);
        while changes < 2_000 {
            messages += 1;
            let stall = if messages % 9 == 4 { 3 } else { 1 };
            stalls += usize::from(stall > 1);
            let mut inbox: Option<ShadowNotes> = None;
            for _ in 0..stall {
                frame += 1;
                for _ in 0..1 + rng.next(12) {
                    let w = m.window();
                    let c = [0, 1, 2].map(|k| w.min.0[k] + rng.next(w.size()[k] as u64) as i32);
                    let kind = rng.next(10) as usize;
                    kinds[kind] += 1;
                    changes += 1;
                    if (5..9).contains(&kind) {
                        light_change(&mut m, &mut rng, c, kind);
                        m.light_changed.push(ChunkCoord(c));
                        continue;
                    }
                    let s = m.chunks.entry(c).or_insert((ChunkNote::default(), 0));
                    match kind {
                        // An arrival: queued, then (another frame) uploaded full.
                        0 => s.0.pending = true,
                        1 => {
                            s.0 = ChunkNote {
                                mirrored: true,
                                lod: false,
                                pending: false,
                            };
                            s.1 = 64 * c[2] + rng.next(64) as i32;
                        }
                        // A LOD swap.
                        2 => s.0.lod = !s.0.lod && s.0.mirrored,
                        // An eviction: its lights go with it.
                        3 => {
                            m.chunks.remove(&c);
                            if m.lights.remove(&c).is_some() {
                                m.light_changed.push(ChunkCoord(c));
                            }
                        }
                        // A dig: the chunk re-uploads with a lower top.
                        4 => {
                            s.0.pending = !s.0.pending;
                            s.1 -= 1 + rng.next(8) as i32;
                        }
                        // A window move: everything outside drops, lights too.
                        _ => {
                            let o = [0, 1, 2].map(|_| rng.next(3) as i32 - 1);
                            let nw = extent([0, 1, 2].map(|k| w.min.0[k] + o[k]));
                            m.window = Some(nw);
                            let size = nw.size();
                            let inside = |c: &[i32; 3]| {
                                (0..3).all(|k| (0..size[k]).contains(&(c[k] - nw.min.0[k])))
                            };
                            // The mirror notes every chunk a move drops.
                            let gone: Vec<[i32; 3]> =
                                m.chunks.keys().filter(|c| !inside(c)).copied().collect();
                            m.changed.extend(gone.iter().map(|c| ChunkCoord(*c)));
                            m.chunks.retain(|c, _| inside(c));
                            let dark: Vec<[i32; 3]> =
                                m.lights.keys().filter(|c| !inside(c)).copied().collect();
                            m.light_changed.extend(dark.iter().map(|c| ChunkCoord(*c)));
                            m.lights.retain(|c, _| inside(c));
                            continue;
                        }
                    }
                    if m.chunks
                        .get(&c)
                        .is_some_and(|s| s.0 == ChunkNote::default())
                    {
                        m.chunks.remove(&c);
                    }
                    m.changed.push(ChunkCoord(c));
                }
                // The first frame's lists are untracked, as the mirror's are.
                let changed = std::mem::take(&mut m.changed);
                let light_changed = std::mem::take(&mut m.light_changed);
                let tracked = frame > 1 && frame % 13 != 0;
                let n = notes(
                    &m,
                    (frame > 1).then_some(changed),
                    tracked.then_some(light_changed),
                    last_window,
                );
                deltas += usize::from(n.lights.is_some() && !n.lights_whole);
                light_wholes += usize::from(frame > 1 && n.lights_whole && !n.whole);
                last_window = n.window;
                inbox = Some(match inbox {
                    None => n,
                    Some(a) => a.then(n),
                });
            }
            shadow.take(&inbox.expect("a frame's notes"));
            check(&m, &shadow, frame);
        }
        assert!(kinds.iter().all(|&k| k > 100), "every kind ran: {kinds:?}");
        assert!(
            deltas > 100 && light_wholes > 10 && stalls > 20,
            "light deltas {deltas}, untracked {light_wholes}, stalls {stalls}"
        );
    }

    /// One light change of the model's chunk `c`: 5 a range written or moved, 6 dirtied or
    /// evicted, 7 the lights dropped, 8 the lists emptied (held, no emitter, not current: a
    /// note equal to the default that still counts as not ready).
    fn light_change(m: &mut Model, rng: &mut Lcg, c: [i32; 3], kind: usize) {
        match kind {
            5 => {
                let l = m.lights.entry(c).or_default();
                l.emitter = rng.next(4) != 0;
                l.current = true;
            }
            6 => {
                let emitter = rng.next(2) == 0;
                m.lights
                    .entry(c)
                    .or_insert(LightNote {
                        emitter,
                        current: false,
                    })
                    .current = false;
            }
            7 => {
                m.lights.remove(&c);
            }
            _ => {
                m.lights.insert(c, LightNote::default());
            }
        }
    }

    /// Done when of Plan 0111 Phases 6a and 6a': over a model mirror of a 16 x 16 x 8 window
    /// whose world holds chunks at chunk z 1 to 7 only (nothing below the window's first layer
    /// or above its last, as a cave's window has empty layers), a window move by one chunk in
    /// x, then in x and z, then a jump clear of the window, each sends notes that read `whole`
    /// false and name only chunks the change list named; the columns noted are the named
    /// chunks' columns inside the window and no other, on the z move too (fewer than the
    /// window's every column); the shadow that took them answers held, LOD-only and pending for
    /// every chunk of the old and new windows, the counts and the lowest top of every column
    /// exactly as a shadow fed whole notes of the same mirror does, and the first frame's notes
    /// read `whole` true. Each move is followed by a frame that uploads part of the entering
    /// slab, read the same way.
    #[test]
    fn a_window_move_sends_a_delta_that_reads_as_the_whole() {
        const SIZE: [i32; 3] = [16, 16, 8];
        let ext = |min: [i32; 3]| ChunkExtent::new(min, [0, 1, 2].map(|k| min[k] + SIZE[k]));
        let mut rng = Lcg(0x0111);
        // A chunk's state as the mirror holds it: a tenth absent, the rest full, LOD-only,
        // waiting, or full and waiting a re-upload.
        let state = |rng: &mut Lcg, c: [i32; 3]| -> Option<(ChunkNote, i32)> {
            let top = 64 * c[2] + rng.next(64) as i32;
            let (mirrored, lod, pending) = match rng.next(10) {
                0 => return None,
                1..=4 => (true, false, false),
                5 | 6 => (true, true, false),
                7 | 8 => (false, false, true),
                _ => (true, false, true),
            };
            Some((
                ChunkNote {
                    mirrored,
                    lod,
                    pending,
                },
                top,
            ))
        };
        let mut m = Model {
            window: Some(ext([0, 0, 0])),
            ..Model::default()
        };
        let w0 = m.window();
        for x in 0..SIZE[0] {
            for y in 0..SIZE[1] {
                for z in 1..SIZE[2] {
                    let c = [x, y, z];
                    if let Some(s) = state(&mut rng, c) {
                        m.chunks.insert(c, s);
                    }
                }
            }
        }
        assert!(m.chunks.len() > 1_500, "{} chunks", m.chunks.len());
        let mut last_window = None;
        let mut shadow = Shadow::default();
        let first = notes(&m, None, None, last_window);
        assert!(first.whole, "the first frame's notes read the whole mirror");
        last_window = first.window;
        shadow.take(&first);
        same_as_whole(&m, &shadow, w0, "first frame");
        let jump = [41, -23, 5];
        for (offset, name) in [([1, 0, 0], "x"), ([1, 0, 1], "z"), (jump, "jump")] {
            let old = m.window();
            let new = ext([0, 1, 2].map(|k| old.min.0[k] + offset[k]));
            // The mirror's move: every held or waiting chunk leaving is dropped and noted; every
            // chunk entering that the world holds is queued and noted.
            m.window = Some(new);
            let gone: Vec<[i32; 3]> = m
                .chunks
                .keys()
                .filter(|c| !new.contains(ChunkCoord(**c)))
                .copied()
                .collect();
            for c in &gone {
                m.chunks.remove(c);
                m.changed.push(ChunkCoord(*c));
            }
            let mut entering = Vec::new();
            let entered = (new.min.0[0]..new.max.0[0]).flat_map(|x| {
                (new.min.0[1]..new.max.0[1]).flat_map(move |y| {
                    (new.min.0[2]..new.max.0[2]).map(move |z| ChunkCoord([x, y, z]))
                })
            });
            for c in entered.filter(|c| !old.contains(*c)) {
                // The world holds nothing outside chunk z 1 to 7.
                if (1..SIZE[2]).contains(&c.0[2]) && rng.next(10) != 0 {
                    m.chunks.insert(
                        c.0,
                        (
                            ChunkNote {
                                pending: true,
                                ..ChunkNote::default()
                            },
                            0,
                        ),
                    );
                    m.changed.push(c);
                    entering.push(c.0);
                }
            }
            for (frame, at) in [("move", old), ("arrivals", new)] {
                if frame == "arrivals" {
                    // Half the entering slab uploads, full or LOD-only.
                    for c in entering.iter().step_by(2) {
                        let lod = rng.next(3) == 0;
                        let top = 64 * c[2] + rng.next(64) as i32;
                        let s = (
                            ChunkNote {
                                mirrored: true,
                                lod,
                                pending: false,
                            },
                            top,
                        );
                        m.chunks.insert(*c, s);
                        m.changed.push(ChunkCoord(*c));
                    }
                }
                let changed = std::mem::take(&mut m.changed);
                let named: std::collections::HashSet<[i32; 3]> =
                    changed.iter().map(|c| c.0).collect();
                let n = notes(&m, Some(changed), Some(Vec::new()), last_window);
                let at_name = format!("{name} {frame}");
                assert!(!n.whole, "{at_name}: a delta");
                assert!(n.chunks.len() <= named.len(), "{at_name}: chunks");
                assert!(
                    n.chunks.iter().all(|(c, _)| named.contains(c)),
                    "{at_name}: every chunk noted was named"
                );
                // Every move alike, z too: the columns noted are the named chunks' columns
                // inside the window, each once, and no other.
                let mut cols: Vec<[i32; 2]> = n.columns.iter().map(|(k, _)| *k).collect();
                cols.sort_unstable();
                let mut want: Vec<[i32; 2]> = named
                    .iter()
                    .map(|c| [c[0], c[1]])
                    .filter(|k| (0..2).all(|a| k[a] >= new.min.0[a] && k[a] < new.max.0[a]))
                    .collect();
                want.sort_unstable();
                want.dedup();
                assert_eq!(cols, want, "{at_name}: the named chunks' columns");
                if name == "z" {
                    let window_columns = (SIZE[0] * SIZE[1]) as usize;
                    assert!(
                        cols.len() < window_columns,
                        "{at_name}: {} columns, not the window's {window_columns}",
                        cols.len()
                    );
                }
                last_window = n.window;
                shadow.take(&n);
                same_as_whole(&m, &shadow, at, &at_name);
                same_as_whole(&m, &shadow, new, &at_name);
            }
        }
    }

    /// `s` answers as a shadow fed whole notes of `m` does, for every chunk and column of `at`.
    fn same_as_whole(m: &Model, s: &Shadow, at: ChunkExtent, name: &str) {
        let mut oracle = Shadow::default();
        oracle.take(&notes(m, None, None, None));
        assert_eq!(
            s.mirrored_chunks(),
            oracle.mirrored_chunks(),
            "{name}: held"
        );
        assert_eq!(
            s.pending_chunks(),
            oracle.pending_chunks(),
            "{name}: pending"
        );
        let sorted = |s: &Shadow| {
            let mut v: Vec<_> = s.full_chunks().map(|c| c.0).collect();
            v.sort_unstable();
            v
        };
        assert_eq!(sorted(s), sorted(&oracle), "{name}: full chunks");
        for x in at.min.0[0]..at.max.0[0] {
            for y in at.min.0[1]..at.max.0[1] {
                assert_eq!(
                    s.column_lowest(x, y),
                    oracle.column_lowest(x, y),
                    "{name}: column {x} {y}"
                );
                for z in at.min.0[2]..at.max.0[2] {
                    let c = ChunkCoord([x, y, z]);
                    let a = (s.is_mirrored(c), s.is_lod(c), s.is_pending(c));
                    let b = (
                        oracle.is_mirrored(c),
                        oracle.is_lod(c),
                        oracle.is_pending(c),
                    );
                    assert_eq!(a, b, "{name}: chunk {x} {y} {z}");
                }
            }
        }
    }

    fn check(m: &Model, s: &Shadow, frame: u64) {
        let w = m.window();
        let held = |c: ChunkCoord| (c.0[0] + c.0[1]) % 2 == 0;
        let mut full_m: Vec<_> = m
            .chunks
            .iter()
            .filter(|(_, s)| s.0.mirrored && !s.0.lod)
            .map(|(c, _)| *c)
            .collect();
        let mut full_s: Vec<_> = s.full_chunks().map(|c| c.0).collect();
        full_m.sort_unstable();
        full_s.sort_unstable();
        assert_eq!(full_m, full_s, "frame {frame}: full chunks");
        let count = |f: fn(&ChunkNote) -> bool| m.chunks.values().filter(|s| f(&s.0)).count();
        assert_eq!(s.mirrored_chunks(), count(|n| n.mirrored), "frame {frame}");
        assert_eq!(s.pending_chunks(), count(|n| n.pending), "frame {frame}");
        for z in w.min.0[2] - 1..w.min.0[2] + w.size()[2] + 1 {
            for y in w.min.0[1] - 1..w.min.0[1] + w.size()[1] + 1 {
                for x in w.min.0[0] - 1..w.min.0[0] + w.size()[0] + 1 {
                    let c = ChunkCoord([x, y, z]);
                    let n = m.chunk_note(c);
                    let at = format!("frame {frame}, chunk {:?}", c.0);
                    assert_eq!(s.is_mirrored(c), n.mirrored, "{at}: held");
                    assert_eq!(s.is_lod(c), n.mirrored && n.lod, "{at}: LOD-only");
                    assert_eq!(s.is_pending(c), n.pending, "{at}: pending");
                    let model_near = |r: i32| {
                        m.lights
                            .iter()
                            .any(|(k, l)| l.emitter && boxes_within(ChunkCoord(*k), c, r))
                    };
                    let model_ready = |r: i32| {
                        m.lights
                            .iter()
                            .all(|(k, l)| l.current || !boxes_within(ChunkCoord(*k), c, r))
                    };
                    for r in [1, 2] {
                        assert_eq!(s.emitters_near(c, r), model_near(r), "{at}: emitters");
                        assert_eq!(s.lights_ready_near(c, r), model_ready(r), "{at}: lights");
                    }
                }
            }
        }
        for y in w.min.0[1]..w.min.0[1] + w.size()[1] {
            for x in w.min.0[0]..w.min.0[0] + w.size()[0] {
                let at = format!("frame {frame}, column {x} {y}");
                assert_eq!(s.column_lowest(x, y), m.column_lowest(x, y), "{at}: lowest");
                let (z0, zn) = (w.min.0[2], w.size()[2]);
                for wanted in [None, Some(&held as &dyn Fn(ChunkCoord) -> bool)] {
                    let mf = column_is_final(
                        x,
                        y,
                        z0..z0 + zn,
                        |c| m.chunk_note(c).pending,
                        |c| m.chunk_note(c).mirrored,
                        wanted,
                    );
                    assert_eq!(s.column_final(x, y, wanted), mf, "{at}: final");
                }
            }
        }
    }
}
