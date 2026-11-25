//! Write lists (ADR-0083): the store's buffer writes as the bake half makes them, applied by the
//! render half once a frame, whole or not at all, under a byte cap.
//!
//! A **unit** is the writes that must land in one frame together: a block's words with its table
//! word, a fade's shown words with the block they show, an eviction's table word with its
//! summaries. A list holds whole units. Lists are applied in the order they were made, so a later
//! write of a word always lands after an earlier one. An edit's list (a dig's sky-stale marks,
//! its held and provisional blocks) is never carried: it is applied in the frame it reaches the
//! render half, and every list made before it goes with it, whatever the frame's cap.

use std::collections::VecDeque;

/// One buffer write into the pool: its byte offset, its bytes, and its kind ([`super::W_POOL`]
/// and its neighbours) for the step's byte tally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Write {
    pub(crate) offset: u64,
    pub(crate) data: Vec<u8>,
    pub(crate) kind: usize,
    /// The chunk whose landing word `data` holds, and where ([`Landing`]).
    pub(crate) landing: Option<Landing>,
}

/// A block write's landing word: its chunk, the word's index into the write's data (in words),
/// and whether the write is the chunk's first block. The render half writes the frame a first
/// block's list is applied in, plus one, there ([`Lists::apply`]), so the shader's landing fade
/// starts at the frame the block is first read. Any other block write of the chunk carries the
/// bake half's copy of the word, which is stale until the word written reaches it: the render
/// half writes its own word there until then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Landing {
    pub(crate) chunk: [i32; 3],
    pub(crate) at: usize,
    pub(crate) first: bool,
}

/// Ordered writes the render half applies whole in one frame.
#[derive(Clone, Debug, Default)]
pub(crate) struct WriteList {
    /// The newest message's frame the bake half had read when it made the list
    /// ([`super::FrameMessage::frame`]).
    pub(crate) frame: u64,
    pub(crate) writes: Vec<Write>,
    pub(crate) bytes: usize,
    /// An edit's list (or a unit over the cap sent whole): never carried; the lists made before
    /// it are applied with it.
    pub(crate) edit: bool,
}

impl WriteList {
    pub(crate) fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }
}

/// Where the render half's writes go: the queue and the pool in the frame, a recorder in a test.
pub(crate) trait Applier {
    fn write(&mut self, offset: u64, data: &[u8], kind: usize);
}

/// A [`WriteList`] over the frame's cap: refused at its making, never split.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OverCap {
    pub(crate) bytes: usize,
    pub(crate) cap: usize,
}

/// The bake half's list maker: units appended whole, a list closed when the next unit would
/// take it over the cap, and the lists given in the order they were made. With no cap
/// (lockstep's default) a step makes one list, or up to three around an edit.
#[derive(Debug, Default)]
pub(crate) struct ListMaker {
    pub(crate) cap: Option<usize>,
    /// The message's frame the lists are named by ([`WriteList::frame`]).
    pub(crate) frame: u64,
    /// The unit being written ([`ListMaker::seal`] closes it).
    unit: Vec<Write>,
    unit_bytes: usize,
    /// The open list: the landings', or the edit's while [`ListMaker::editing`] (one list,
    /// whatever its size).
    current: WriteList,
    pub(crate) done: Vec<WriteList>,
    /// Whether writes go to an edit's list or to the landings'. A unit sealed after it changes
    /// closes the open list first, so the edit's list sits between the landings made before
    /// and after it.
    pub(crate) editing: bool,
    /// Units over the cap the maker refused (a block larger than a frame's cap), kept for the
    /// caller: [`ListMaker::force`] sends each as a list of its own that is never carried.
    pub(crate) refused: Vec<Vec<Write>>,
}

impl ListMaker {
    pub(crate) fn new(cap: Option<usize>, frame: u64) -> Self {
        Self {
            cap,
            frame,
            ..Self::default()
        }
    }

    /// Appends one write to the open unit.
    pub(crate) fn push(&mut self, w: Write) {
        self.unit_bytes += w.data.len();
        self.unit.push(w);
    }

    /// Closes the open unit: an edit's joins the edit's list; a landing's joins the current list,
    /// which is closed first if the unit would take it over the cap. A unit alone over the cap
    /// is refused ([`OverCap`]) and kept in [`ListMaker::refused`]; never split.
    pub(crate) fn seal(&mut self) -> Result<(), OverCap> {
        if self.unit.is_empty() {
            return Ok(());
        }
        let bytes = std::mem::take(&mut self.unit_bytes);
        let unit = std::mem::take(&mut self.unit);
        if self.editing != self.current.edit {
            self.close();
        }
        if !self.editing
            && let Some(cap) = self.cap
        {
            if bytes > cap {
                self.refused.push(unit);
                return Err(OverCap { bytes, cap });
            }
            if self.current.bytes + bytes > cap {
                self.close();
            }
        }
        self.current.bytes += bytes;
        self.current.writes.extend(unit);
        Ok(())
    }

    /// Sends the refused units whole, each as a list of its own in its place in the order (the
    /// store's books already hold them, so they cannot wait or drop): never carried.
    pub(crate) fn force(&mut self) {
        for unit in std::mem::take(&mut self.refused) {
            self.close();
            self.current.bytes = unit.iter().map(|w| w.data.len()).sum::<usize>();
            self.current.writes = unit;
            self.current.edit = true;
            self.close();
        }
    }

    fn close(&mut self) {
        if !self.current.is_empty() {
            let mut l = std::mem::take(&mut self.current);
            l.frame = self.frame;
            self.done.push(l);
        }
        self.current.edit = self.editing;
    }

    /// The step's lists in the order made: the landings before an edit, the edit's (flagged),
    /// the landings after.
    pub(crate) fn finish(&mut self) -> Vec<WriteList> {
        // An unsealed unit is a caller's slip; it is sealed whole here rather than lost.
        if self.seal().is_err() {
            self.force();
        }
        self.close();
        self.editing = false;
        self.current.edit = false;
        std::mem::take(&mut self.done)
    }
}

/// The render half's lists waiting to apply, in the order they were made.
#[derive(Debug, Default)]
pub(crate) struct Lists {
    /// Bytes a frame applies at most; `None` applies every list the frame it arrives.
    pub(crate) cap: Option<usize>,
    pub(crate) waiting: VecDeque<WriteList>,
    /// Lists applied in a later frame than the first they could have been.
    pub(crate) carried: u64,
    /// The bytes the last [`Lists::apply`] applied over its cap: an edit's list and the lists
    /// made before it go whatever the cap (0 within it).
    pub(crate) over: usize,
    /// The landing words written that the bake half's copy may not hold yet, by chunk: the word,
    /// and the message's frame that carried it to the bake half (`None`: not sent yet).
    pub(crate) unconfirmed: std::collections::HashMap<[i32; 3], (u32, Option<u64>)>,
    /// The newest message's frame a list applied so far was made from ([`WriteList::frame`]).
    pub(crate) newest: u64,
}

impl Lists {
    /// Takes a step's lists, in the order given. A landing list over the cap is refused at its
    /// making, never split.
    pub(crate) fn take(&mut self, lists: Vec<WriteList>) -> Result<(), OverCap> {
        for l in lists {
            if let Some(cap) = self.cap
                && !l.edit
                && l.bytes > cap
            {
                return Err(OverCap {
                    bytes: l.bytes,
                    cap,
                });
            }
            self.waiting.push_back(l);
        }
        Ok(())
    }

    /// Applies whole lists in order: every list up to and with the last edit's list waiting,
    /// whatever the cap, then landing lists while the next fits in what the cap leaves. `frame`
    /// is the frame applying them: a first block's landing word reads `frame + 1`. Returns the
    /// bytes applied and the landing words written, by chunk; [`Lists::over`] reads the bytes
    /// past the cap.
    pub(crate) fn apply(
        &mut self,
        frame: u64,
        out: &mut dyn Applier,
    ) -> (usize, Vec<([i32; 3], u32)>) {
        let mut bytes = 0usize;
        let mut landed = Vec::new();
        // The lists that go whatever the cap: up to and with the last edit's.
        let mut forced = self
            .waiting
            .iter()
            .rposition(|l| l.edit)
            .map_or(0, |k| k + 1);
        while let Some(l) = self.waiting.front() {
            let fits = forced > 0 || self.cap.is_none_or(|cap| bytes + l.bytes <= cap);
            if !fits {
                break;
            }
            forced = forced.saturating_sub(1);
            let l = self.waiting.pop_front().expect("front above");
            bytes += l.bytes;
            self.newest = self.newest.max(l.frame);
            // The bake half had read the words sent up to the list's message.
            self.unconfirmed
                .retain(|_, (_, sent)| sent.is_none_or(|f| f > l.frame));
            apply_list(l, frame, out, &mut landed, &mut self.unconfirmed);
        }
        self.over = self.cap.map_or(0, |cap| bytes.saturating_sub(cap));
        self.carried += self.waiting.len() as u64;
        (bytes, landed)
    }

    /// The landing words not sent to the bake half, for the message of `frame`
    /// ([`super::FrameMessage::landed`]).
    pub(crate) fn report(&mut self, frame: u64) -> Vec<([i32; 3], u32)> {
        let mut out = Vec::new();
        for (c, (w, sent)) in &mut self.unconfirmed {
            if sent.is_none() {
                *sent = Some(frame);
                out.push((*c, *w));
            }
        }
        out.sort_unstable();
        out
    }

    /// Every word written is in the bake half's copy (lockstep: the step takes them at apply).
    pub(crate) fn confirm_all(&mut self) {
        self.unconfirmed.clear();
    }
}

fn apply_list(
    l: WriteList,
    frame: u64,
    out: &mut dyn Applier,
    landed: &mut Vec<([i32; 3], u32)>,
    unconfirmed: &mut std::collections::HashMap<[i32; 3], (u32, Option<u64>)>,
) {
    let word = u32::try_from(frame + 1).unwrap_or(u32::MAX);
    for mut w in l.writes {
        if let Some(Landing { chunk, at, first }) = w.landing {
            let put = if first {
                landed.push((chunk, word));
                unconfirmed.insert(chunk, (word, None));
                Some(word)
            } else {
                unconfirmed.get(&chunk).map(|(w, _)| *w)
            };
            if let Some(v) = put
                && let Some(d) = w.data.get_mut(at * 4..at * 4 + 4)
            {
                d.copy_from_slice(&v.to_le_bytes());
            }
        }
        out.write(w.offset, &w.data, w.kind);
    }
}

/// The frame's applier: `wgpu` writes into the pool, tallied as the step's `render` share.
pub(crate) struct PoolApplier<'a> {
    pub(crate) queue: &'a wgpu::Queue,
    pub(crate) pool: &'a wgpu::Buffer,
}

impl Applier for PoolApplier<'_> {
    fn write(&mut self, offset: u64, data: &[u8], kind: usize) {
        super::write(self.queue, self.pool, offset, data, kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records every write with the frame it was applied in.
    #[derive(Default)]
    struct Recorder {
        frame: u64,
        log: Vec<(u64, u64, usize)>,
    }

    impl Applier for Recorder {
        fn write(&mut self, offset: u64, data: &[u8], _kind: usize) {
            self.log.push((self.frame, offset, data.len()));
        }
    }

    /// A landing as the store makes it: a block's words, then its table word.
    fn landing(m: &mut ListMaker, block_at: u64, block_bytes: usize, table_at: u64) {
        m.push(Write {
            offset: block_at,
            data: vec![0; block_bytes],
            kind: 0,
            landing: None,
        });
        m.push(Write {
            offset: table_at,
            data: vec![0; 4],
            kind: 1,
            landing: None,
        });
        m.seal().expect("under the cap");
    }

    /// Done when (1) of Plan 0099 Phase 2: three lists of about 40 KiB under a 64 KiB cap apply
    /// one a frame over three frames, in order; no frame holds a block's words without its
    /// table word; a single list over the cap is refused at its making, not split.
    #[test]
    fn a_landing_is_applied_whole_or_not_at_all() {
        const KIB: usize = 1024;
        let cap = 64 * KIB;
        let mut m = ListMaker::new(Some(cap), 1);
        // Three landings of 40 KiB with their table words: each its own list under 64 KiB.
        for (k, at) in [(0u64, 100_000u64), (1, 200_000), (2, 300_000)] {
            landing(&mut m, at, 40 * KIB - 4, 16 + k * 4);
        }
        let lists = m.finish();
        assert_eq!(lists.len(), 3);
        assert!(lists.iter().all(|l| l.bytes == 40 * KIB && !l.edit));
        let mut q = Lists {
            cap: Some(cap),
            ..Lists::default()
        };
        q.take(lists).expect("each under the cap");
        let mut rec = Recorder::default();
        for frame in 1..=4 {
            rec.frame = frame;
            let (bytes, _) = q.apply(frame, &mut rec);
            assert!(bytes <= cap, "frame {frame} applied {bytes} bytes");
        }
        // One list a frame, in order: block then its table word, in the same frame.
        let expect = [
            (1, 100_000, 40 * KIB - 4),
            (1, 16, 4),
            (2, 200_000, 40 * KIB - 4),
            (2, 20, 4),
            (3, 300_000, 40 * KIB - 4),
            (3, 24, 4),
        ];
        assert_eq!(rec.log, expect.map(|(f, o, n)| (f as u64, o as u64, n)));
        for f in 1..=3u64 {
            let blocks = rec.log.iter().filter(|e| e.0 == f && e.2 > 4).count();
            let tables = rec.log.iter().filter(|e| e.0 == f && e.2 == 4).count();
            assert_eq!(blocks, tables, "frame {f}: a block without its table word");
        }
        // A single unit over the cap is refused when it is made, not split across lists.
        let mut m = ListMaker::new(Some(cap), 2);
        m.push(Write {
            offset: 0,
            data: vec![0; 70 * KIB],
            kind: 0,
            landing: None,
        });
        assert_eq!(
            m.seal(),
            Err(OverCap {
                bytes: 70 * KIB,
                cap
            })
        );
        assert!(m.finish().is_empty());
        // And a list over the cap is refused at the render half's door.
        let big = WriteList {
            frame: 3,
            writes: vec![],
            bytes: 70 * KIB,
            edit: false,
        };
        assert!(q.take(vec![big]).is_err());
    }

    /// Done when (9) of Plan 0099 Phase 2: a first block whose list was made at frame 5 and
    /// applied at frame 7 holds 8 at its landing word; the word is reported for the bake half's
    /// copy, which a later refinement keeps.
    #[test]
    fn a_first_blocks_landing_word_is_the_frame_it_is_applied_in() {
        let mut m = ListMaker::new(None, 5);
        m.push(Write {
            offset: 400,
            // Word 2 is the landing word, made at frame 5 as 6.
            data: [0u32, 0, 6, 0]
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .collect(),
            kind: 0,
            landing: Some(Landing {
                chunk: [1, 2, 3],
                at: 2,
                first: true,
            }),
        });
        m.seal().expect("no cap");
        let mut q = Lists::default();
        q.take(m.finish()).expect("no cap");
        struct Words(Vec<u32>);
        impl Applier for Words {
            fn write(&mut self, _offset: u64, data: &[u8], _kind: usize) {
                self.0 = bytemuck::pod_collect_to_vec(data);
            }
        }
        let mut w = Words(Vec::new());
        // Made at frame 5, first applied at frame 7 (the render half did not apply at 5 or 6).
        let (_, landed) = q.apply(7, &mut w);
        assert_eq!(w.0, vec![0, 0, 8, 0]);
        assert_eq!(landed, vec![([1, 2, 3], 8)]);
        // A refinement keeps its block's word: its write carries no landing mark, so the word
        // the bake half copied from the patched block (8) is what it writes at frame 9.
        let mut m = ListMaker::new(None, 9);
        m.push(Write {
            offset: 400,
            data: [0u32, 0, 8, 1]
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .collect(),
            kind: 0,
            landing: None,
        });
        m.seal().expect("no cap");
        q.take(m.finish()).expect("no cap");
        let (_, landed) = q.apply(9, &mut w);
        assert_eq!(w.0[2], 8);
        assert!(landed.is_empty());
    }

    /// A byte-addressed pool the writes land in, in the order applied.
    #[derive(Default)]
    struct Pool(Vec<u8>);

    impl Applier for Pool {
        fn write(&mut self, offset: u64, data: &[u8], _kind: usize) {
            let at = offset as usize;
            if self.0.len() < at + data.len() {
                self.0.resize(at + data.len(), 0);
            }
            self.0[at..at + data.len()].copy_from_slice(data);
        }
    }

    impl Pool {
        fn word(&self, at: u64) -> u32 {
            let at = at as usize;
            u32::from_le_bytes(self.0[at..at + 4].try_into().expect("4 bytes"))
        }
    }

    /// A write of `words` at byte `offset`.
    fn words_at(offset: u64, words: &[u32], kind: usize) -> Write {
        Write {
            offset,
            data: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
            kind,
            landing: None,
        }
    }

    /// One unit's list made from message `frame`: a landing's, or an edit's when `edit`.
    fn made(frame: u64, edit: bool, unit: Vec<Write>) -> Vec<WriteList> {
        let mut m = ListMaker::new(Some(1 << 20), frame);
        m.editing = edit;
        for w in unit {
            m.push(w);
        }
        m.seal().expect("under the maker's cap");
        m.finish()
    }

    /// A list of `bytes` with no writes: what the cap counts.
    fn sized(frame: u64, bytes: usize, edit: bool) -> WriteList {
        WriteList {
            frame,
            writes: vec![],
            bytes,
            edit,
        }
    }

    /// The edit's list and the two landings made before it apply in its frame whatever the cap
    /// (660 bytes over a cap of 100); the landing made after it waits.
    #[test]
    fn an_edit_list_is_never_carried() {
        let cap = 100;
        let mut q = Lists {
            cap: Some(cap),
            ..Lists::default()
        };
        q.take(vec![sized(1, 80, false), sized(1, 80, false)])
            .expect("under");
        q.take(vec![sized(2, 500, true), sized(2, 80, false)])
            .expect("an edit's is never refused");
        let mut rec = Recorder::default();
        let (bytes, _) = q.apply(2, &mut rec);
        assert_eq!(bytes, 660, "the two landings ahead and the edit's 500");
        assert_eq!(q.over, 560);
        assert_eq!(q.waiting.len(), 1, "none behind it");
        let (bytes, _) = q.apply(3, &mut rec);
        assert_eq!(bytes, 80);
        assert_eq!(q.over, 0);
    }

    /// Done when (1) of Plan 0099 Phase 3c (the recording applier). A fade's show of a chunk's
    /// place made from message 5 and an edit's held block at the same place made from message
    /// 6, taken in one frame under a cap that fits neither: the place holds the edit's block.
    /// A landing (block at P, table word to P) from message 5 and the edit's block at Q with
    /// its table word: the table reads Q. 2 MiB of landings ahead of the edit under a 1 MiB
    /// cap: all applied in that frame, 1 MiB over; two landings made after the edit wait.
    #[test]
    fn an_edits_list_lands_after_the_writes_made_before_it() {
        const P: u64 = 4096;
        const Q: u64 = 8192;
        const TABLE: u64 = 64;
        let shown = [7u32; 16];
        let held = [9u32; 16];
        let mut q = Lists {
            cap: Some(68),
            ..Lists::default()
        };
        q.take(made(5, false, vec![words_at(P, &shown, 0)]))
            .expect("under");
        q.take(made(6, true, vec![words_at(P, &held, 0)]))
            .expect("an edit's");
        let mut pool = Pool::default();
        q.apply(7, &mut pool);
        assert!(q.waiting.is_empty(), "both in the edit's frame");
        assert_eq!(pool.word(P), 9, "the place holds the edit's block");
        assert_eq!(pool.word(P + 60), 9);

        // A landing pointing the table at P, then the edit's block at Q pointing it at Q.
        let mut q = Lists {
            cap: Some(68),
            ..Lists::default()
        };
        let p_word = u32::try_from(P / 4).expect("small");
        let q_word = u32::try_from(Q / 4).expect("small");
        q.take(made(
            5,
            false,
            vec![words_at(P, &shown, 0), words_at(TABLE, &[p_word], 1)],
        ))
        .expect("under");
        q.take(made(
            6,
            true,
            vec![words_at(Q, &held, 0), words_at(TABLE, &[q_word], 1)],
        ))
        .expect("an edit's");
        let mut pool = Pool::default();
        q.apply(7, &mut pool);
        assert!(q.waiting.is_empty());
        assert_eq!(pool.word(TABLE), q_word, "the table reads the edit's place");
        assert_eq!(pool.word(Q), 9);

        // 2 MiB of landings ahead of a 512-byte edit under a 1 MiB cap.
        const MIB: usize = 1 << 20;
        let mut q = Lists {
            cap: Some(MIB),
            ..Lists::default()
        };
        let landing = (2 * MIB - 512) / 4;
        q.take((0..4).map(|_| sized(5, landing, false)).collect())
            .expect("each under the cap");
        q.take(vec![sized(6, 512, true)]).expect("an edit's");
        q.take(vec![sized(7, 100, false)]).expect("under");
        q.take(vec![sized(8, 100, false)]).expect("under");
        let mut rec = Recorder::default();
        let (bytes, _) = q.apply(8, &mut rec);
        assert_eq!(bytes, 2 * MIB, "every list up to and with the edit's");
        assert_eq!(q.over, MIB, "1 MiB over the cap");
        assert_eq!(q.waiting.len(), 2, "the landings made after the edit wait");
        let (bytes, _) = q.apply(9, &mut rec);
        assert_eq!((bytes, q.over), (200, 0), "and apply in the next frame");
    }

    /// Done when (2) of Plan 0099 Phase 3c. A table write made before the box, the edit's write
    /// of the same word, a landing after: three lists in that order, the word the edit's. A
    /// unit over the maker's cap after them is sent whole as a list of its own, in its place.
    #[test]
    fn a_steps_lists_keep_the_order_they_were_made() {
        const TABLE: u64 = 64;
        let mut m = ListMaker::new(Some(256), 3);
        m.push(words_at(TABLE, &[1], 1));
        m.seal().expect("under");
        m.editing = true;
        m.push(words_at(TABLE, &[2], 1));
        m.push(words_at(1024, &[2; 8], 0));
        m.seal().expect("an edit's");
        m.editing = false;
        m.push(words_at(2048, &[3; 8], 0));
        m.seal().expect("under");
        m.push(words_at(4096, &[4; 128], 0));
        assert!(m.seal().is_err(), "512 bytes over the 256 cap");
        m.force();
        let lists = m.finish();
        let shape: Vec<(bool, usize)> = lists.iter().map(|l| (l.edit, l.bytes)).collect();
        assert_eq!(
            shape,
            [(false, 4), (true, 36), (false, 32), (true, 512)],
            "the order made, the unit over the cap whole and never carried"
        );
        let mut q = Lists {
            cap: Some(256),
            ..Lists::default()
        };
        q.take(lists).expect("the landings under the cap");
        let mut pool = Pool::default();
        let (bytes, _) = q.apply(3, &mut pool);
        assert_eq!(bytes, 584);
        assert_eq!(pool.word(TABLE), 2, "the edit's word over the earlier one");
        assert_eq!(pool.word(2048), 3);
    }

    /// Done when (9) of Plan 0099 Phase 3a: a first block's list made from message 5 and a
    /// fade's `show` of that chunk made from message 6 (which carries the bake half's copy of
    /// the word, stale until the word written reaches it), applied in frames 8 and 9: the
    /// landing word reads 9 after each. A list made after the bake half had the word (message
    /// 10 or later, the word sent with message 9) is written as made.
    #[test]
    fn a_fade_ahead_of_its_landing_list_keeps_the_applied_word() {
        const C: [i32; 3] = [4, 0, -1];
        const AT: usize = 1;
        let block = |head: u32, first: bool| Write {
            offset: 800,
            data: [7u32, head, 7]
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .collect(),
            kind: 0,
            landing: Some(Landing {
                chunk: C,
                at: AT,
                first,
            }),
        };
        let word = |p: &Pool| p.word(804);
        let list = |frame: u64, w: Write| {
            let mut m = ListMaker::new(Some(1 << 20), frame);
            m.push(w);
            m.seal().expect("under the cap");
            m.finish()
        };
        let mut q = Lists {
            cap: Some(1 << 20),
            ..Lists::default()
        };
        // Made from message 5 (the first block, the bake half's guess 6) and 6 (the fade's
        // show, the copy's word still 6).
        q.take(list(5, block(6, true))).expect("under");
        q.take(list(6, block(6, false))).expect("under");
        let mut pool = Pool::default();
        // Frame 8 applies the first; the frame cap lets the second wait to frame 9.
        q.cap = Some(12);
        let (_, landed) = q.apply(8, &mut pool);
        assert_eq!(landed, vec![(C, 9)]);
        assert_eq!(word(&pool), 9, "after frame 8");
        // Frame 9's message carries the word to the bake half; the show made from message 6
        // is applied in frame 9 and keeps 9.
        assert_eq!(q.report(9), vec![(C, 9)]);
        let (_, landed) = q.apply(9, &mut pool);
        assert!(landed.is_empty());
        assert_eq!(word(&pool), 9, "after frame 9");
        // A list made from message 10 read the word: what it carries is written as made.
        q.take(list(10, block(9, false))).expect("under");
        q.apply(10, &mut pool);
        assert_eq!(word(&pool), 9);
        assert!(q.unconfirmed.is_empty(), "the word is the bake half's");
        q.take(list(11, block(3, false))).expect("under");
        q.apply(11, &mut pool);
        assert_eq!(word(&pool), 3, "a later list's word is the bake half's own");
    }
}
