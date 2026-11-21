//! The live queue's in-view drain in `run.json` and on the `bake queue:` line.

use std::fmt::Write as _;

use super::write_ms;

/// The live queue's in-view drain as `run.json`'s `queue` and the `bake queue:` line read it:
/// the queue's depth, the chunks in view waiting for their light by band and class
/// ([`vxe_render::InViewWaiting`]), the samples they still need, the last second's samples a
/// second, and `drain_s`, the need over that rate (`None` while nothing is drawn and something
/// waits).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QueueDrain {
    pub depth: u64,
    pub waiting: vxe_render::InViewWaiting,
    pub samples_per_s: f64,
}

impl QueueDrain {
    /// The need over the rate ([`vxe_render::InViewWaiting::drain_s`]).
    pub fn drain_s(&self) -> Option<f64> {
        self.waiting.drain_s(self.samples_per_s)
    }

    /// The drain's text on the `bake queue:` line: `in view 6 waiting (near 2 mid 2 far 2; no
    /// record 6 coarse 0), need 1.200e7 samples, drain 4.0 s`, the drain `n/a` while nothing
    /// is drawn and something waits.
    pub fn text(&self) -> String {
        let w = &self.waiting;
        let [near, mid, far] = w.by_band();
        let [blank, coarse] = w.by_class();
        format!(
            "in view {} waiting (near {near} mid {mid} far {far}; no record {blank} coarse {coarse}), need {:.3e} samples, drain {}",
            w.total(),
            w.samples as f64,
            drain_words(self.drain_s()),
        )
    }

    /// `run.json`'s `queue` object.
    pub fn json(&self) -> String {
        let mut o = String::new();
        self.write_json(&mut o);
        o
    }

    pub(super) fn write_json(&self, o: &mut String) {
        let w = &self.waiting;
        let [near, mid, far] = w.by_band();
        let [blank, coarse] = w.by_class();
        let _ = write!(
            o,
            "{{\"depth\": {}, \"in_view_waiting\": {}, \"near\": {near}, \"mid\": {mid}, \"far\": {far}, \"no_record\": {blank}, \"coarse\": {coarse}, \"need_samples\": {}, \"samples_per_s\": ",
            self.depth,
            w.total(),
            w.samples,
        );
        write_ms(o, self.samples_per_s);
        o.push_str(", \"drain_s\": ");
        match self.drain_s() {
            Some(s) => write_ms(o, s),
            None => o.push_str("null"),
        }
        o.push('}');
    }
}

/// A drain's seconds in words: `4.0 s`, or `n/a` for `None`.
pub fn drain_words(s: Option<f64>) -> String {
    s.map_or_else(|| "n/a".to_owned(), |s| format!("{s:.1} s"))
}
