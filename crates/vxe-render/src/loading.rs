//! Loading's progress signal (ADR-0059): [`LoadProgress`], app-named stages each with a weight,
//! whose overall fraction never moves back.

/// One stage of loading: its name, its share of the bar, and its count when it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadStage {
    pub name: &'static str,
    /// The stage's share of the bar, relative to the other stages' weights.
    pub weight: f32,
    /// Units done, of `of` when the stage knows its count.
    pub done: u64,
    /// The stage's count; `None` when it has none, and then it fills only when it ends.
    pub of: Option<u64>,
}

/// Ordered loading stages and the fraction they add up to. The current stage fills its weight
/// by its count; a stage that ends fills all of it. [`LoadProgress::fraction`] is the largest
/// fraction reported so far, so a count that drops (a chunk withdrawn) holds the bar.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadProgress {
    stages: Vec<LoadStage>,
    current: usize,
    shown: f32,
}

impl LoadProgress {
    /// Stages of these names and weights, in order, at the start of the first.
    pub fn new(stages: &[(&'static str, f32)]) -> Self {
        Self {
            stages: stages
                .iter()
                .map(|&(name, weight)| LoadStage {
                    name,
                    weight: weight.max(0.0),
                    done: 0,
                    of: None,
                })
                .collect(),
            current: 0,
            shown: 0.0,
        }
    }

    /// The current stage's count: `done` of `of`.
    pub fn set(&mut self, done: u64, of: Option<u64>) {
        if let Some(s) = self.stages.get_mut(self.current) {
            s.done = done;
            s.of = of;
        }
        self.update();
    }

    /// Ends the current stage and starts the next; past the last, loading has ended.
    pub fn next(&mut self) {
        self.current = (self.current + 1).min(self.stages.len());
        self.update();
    }

    /// Moves to the stage named `name` if it lies ahead, ending every stage before it; a name
    /// behind or unknown is ignored.
    pub fn enter(&mut self, name: &str) {
        if let Some(i) = self.stages.iter().position(|s| s.name == name)
            && i > self.current
        {
            self.current = i;
            self.update();
        }
    }

    /// Ends every stage.
    pub fn finish(&mut self) {
        self.current = self.stages.len();
        self.update();
    }

    /// Whether every stage has ended.
    pub fn finished(&self) -> bool {
        self.current >= self.stages.len()
    }

    /// The overall fraction, 0 to 1: never below a value it reported before.
    pub fn fraction(&self) -> f32 {
        self.shown
    }

    /// The current stage; the last once loading has ended. `None` without stages.
    pub fn stage(&self) -> Option<&LoadStage> {
        self.stages.get(self.current).or_else(|| self.stages.last())
    }

    /// The stages, in order.
    pub fn stages(&self) -> &[LoadStage] {
        &self.stages
    }

    fn update(&mut self) {
        let total: f32 = self.stages.iter().map(|s| s.weight).sum();
        let raw = if self.finished() || total <= 0.0 {
            if self.finished() { 1.0 } else { 0.0 }
        } else {
            let before: f32 = self.stages[..self.current].iter().map(|s| s.weight).sum();
            let s = &self.stages[self.current];
            let part = match s.of {
                Some(of) if of > 0 => (s.done as f32 / of as f32).clamp(0.0, 1.0),
                _ => 0.0,
            };
            (before + part * s.weight) / total
        };
        self.shown = self.shown.max(raw.clamp(0.0, 1.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    /// Stages weighted 1, 2 and 7 read 0.1 once the first ends and 0.3 once the second does;
    /// a count that drops holds the bar where it was.
    #[test]
    fn the_bar_never_moves_back() {
        let mut p = LoadProgress::new(&[("a", 1.0), ("b", 2.0), ("c", 7.0)]);
        assert_eq!(p.fraction(), 0.0);
        p.next();
        assert!(close(p.fraction(), 0.1), "{}", p.fraction());
        p.set(1, Some(2));
        assert!(close(p.fraction(), 0.2), "{}", p.fraction());
        p.next();
        assert!(close(p.fraction(), 0.3), "{}", p.fraction());
        p.set(5, Some(10));
        assert!(close(p.fraction(), 0.65), "{}", p.fraction());
        p.set(4, Some(10));
        assert!(close(p.fraction(), 0.65), "a withdrawn chunk holds the bar");
        p.set(4, Some(20));
        assert!(close(p.fraction(), 0.65), "a grown count holds the bar");
        p.finish();
        assert_eq!(p.fraction(), 1.0);
        assert!(p.finished());
        assert_eq!(p.stage().map(|s| s.name), Some("c"));
    }

    /// A stage fills its weight by its count: 3 of 4 of the first of two equal stages is
    /// 0.375; a stage without a count fills nothing until it ends.
    #[test]
    fn an_unweighted_stage_fills_by_its_count() {
        let mut p = LoadProgress::new(&[("a", 1.0), ("b", 1.0)]);
        p.set(3, Some(4));
        assert!(close(p.fraction(), 0.375), "{}", p.fraction());
        p.next();
        p.set(7, None);
        assert!(close(p.fraction(), 0.5), "{}", p.fraction());
        p.enter("a");
        assert_eq!(
            p.stage().map(|s| s.name),
            Some("b"),
            "a stage behind is ignored"
        );
    }
}
