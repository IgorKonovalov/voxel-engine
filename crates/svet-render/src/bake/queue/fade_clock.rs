//! A fade's clock (ADR-0083): a fade moves by the frames elapsed since its block was placed, read
//! from the frame number a step runs at, not by the steps it was given. In lockstep a step runs
//! every frame and the two agree; a bake step that comes late takes one larger move.

use super::*;

impl Fade {
    /// Moves the fade to frame `now`: the frames shown since its placement at `start`, the
    /// placement's own frame counting as the first, at most `frames`. Never moves back. Returns
    /// whether it has ended (its target shown).
    pub(super) fn advance(&mut self, now: u64) -> bool {
        let elapsed = now.saturating_sub(self.start).saturating_add(1);
        let at = u32::try_from(elapsed).unwrap_or(u32::MAX).min(self.frames);
        self.frame = self.frame.max(at);
        self.frame >= self.frames
    }
}

/// Whether `c`'s block runs a dig's fade ([`Fade::dig`]): ring 2 of a split chunk waits it out
/// ([`BakeQueue::open_digs`]). A landing fade of another kind does not hold a dig.
pub(super) fn dig_fading(fades: &BTreeMap<[i32; 3], Fade>, c: &[i32; 3]) -> bool {
    fades.get(c).is_some_and(|f| f.dig)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One brick's record at the block's start (`dir` points past it), its cells' key words
    /// baked with bytes `v`.
    fn block(v: u8) -> Vec<u32> {
        let record = record_words(BRICK_CELL);
        let mut b = vec![0u32; CHUNK_BRICKS + record];
        b[0] = ((CHUNK_BRICKS + record) as u32) << 11 | 1;
        for cell in (CHUNK_BRICKS..CHUNK_BRICKS + record).step_by(BRICK_WORDS) {
            let k = cell + KEY_WORD;
            b[k] = u32::from_le_bytes([v; 4]);
            b[k + 1] = KEY_BAKED | u32::from(v) | u32::from(v) << 8;
        }
        b
    }

    fn fade(start: u64, frames: u32, from: Vec<u32>, dig: bool) -> Fade {
        Fade {
            first: 0,
            from,
            start,
            frame: 0,
            frames,
            vis: false,
            dig,
        }
    }

    /// Done when (4) of Plan 0099 Phase 2: a 16-frame fade whose bake steps arrive at frames 1,
    /// 2, 5, 9 and 16: every shown word lies between its old and new value, never moves back,
    /// moves at most the elapsed frames' share of the change plus one unit of rounding, and
    /// equals the new value at frame 16.
    #[test]
    fn a_fade_is_monotone_when_steps_are_skipped() {
        let record = record_words(BRICK_CELL);
        let (old, new) = (10u8, 250u8);
        let (from, to) = (block(old), block(new));
        // Placed in the step at frame 1, whose fades stage shows its first frame: before it the
        // pool showed `from`, the fade's frame 0.
        let mut f = fade(1, FADE, from.clone(), false);
        let key = CHUNK_BRICKS + KEY_WORD;
        let byte = |w: u32| (w & 255) as i32;
        let mut last = i32::from(old);
        let mut last_frame = 0u64;
        let mut shown = Vec::new();
        for now in [1u64, 2, 5, 9, 16] {
            f.advance(now);
            let t = f.frame as f32 / f.frames as f32;
            fade_mix(&mut shown, &f.from, &to, t, record, f.vis);
            let v = byte(shown[key]);
            assert!(
                (i32::from(old)..=i32::from(new)).contains(&v),
                "frame {now}: {v} outside {old}..={new}"
            );
            assert!(v >= last, "frame {now}: moved back from {last} to {v}");
            // The elapsed frames' share of the change, plus one unit of rounding.
            let share =
                (i32::from(new) - i32::from(old)) * (now - last_frame) as i32 / FADE as i32 + 1;
            assert!(
                v - last <= share,
                "frame {now}: moved {} over {} frames, more than {share}",
                v - last,
                now - last_frame
            );
            last = v;
            last_frame = now;
        }
        assert_eq!(f.frame, f.frames);
        assert_eq!(shown, to, "the target at frame 16");
    }

    /// Done when (8) of Plan 0099 Phase 2: ring 1 of a split chunk placed at frame 10, bake
    /// steps at frames 11, 13 and 14: ring 2 opens at neither 11 nor 13 and opens at 14 (the
    /// openings run before the step's fades stage, which drops an ended fade). A chunk under a
    /// 16-frame landing fade opens its dig's bricks in the first step.
    #[test]
    fn ring_two_waits_out_a_dig_fade_whose_steps_are_skipped() {
        let c = [1, 2, 3];
        let mut fades = BTreeMap::new();
        // Placed at frame 10, whose fades stage shows its first frame.
        let mut f = fade(10, DIG_FADE, block(0), true);
        f.advance(10);
        fades.insert(c, f);
        let mut opened = Vec::new();
        for now in [11u64, 13, 14] {
            // The opening, then the fades stage.
            if !dig_fading(&fades, &c) {
                opened.push(now);
            }
            if fades.get_mut(&c).is_some_and(|f| f.advance(now)) {
                fades.remove(&c);
            }
        }
        assert_eq!(opened.first(), Some(&14), "ring 2 opened at {opened:?}");
        // A landing fade, not a dig's: no wait.
        let mut fades = BTreeMap::new();
        fades.insert(c, fade(0, FADE, block(0), false));
        assert!(!dig_fading(&fades, &c));
    }
}
