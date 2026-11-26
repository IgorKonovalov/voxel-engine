//! The live queue's in-view drain in `run.json` and on the `bake queue:` line.

use std::fmt::Write as _;

use super::write_ms;

/// The live queue's in-view drain as `run.json`'s `queue` and the `bake queue:` line read it:
/// the queue's depth, the chunks in view waiting for their light by band and class
/// ([`svet_render::InViewWaiting`]), the samples they still need, the last second's samples a
/// second, and `drain_s`, the need over that rate (`None` while nothing is drawn and something
/// waits). Under `--queue-rule-reading`, the jobs the rule would have answered
/// ([`svet_render::RuleMissed`]; `None` while the reading has counted nothing). Under
/// `--queue-rule-audit`, the blocks compared with the rule's zero record and those that differ
/// by case ([`svet_render::RuleAudit`]; `None` while the audit has compared nothing).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QueueDrain {
    pub depth: u64,
    pub waiting: svet_render::InViewWaiting,
    pub samples_per_s: f64,
    pub rule_missed: Option<svet_render::RuleMissed>,
    pub rule_audit: Option<svet_render::RuleAudit>,
}

impl QueueDrain {
    /// The need over the rate ([`svet_render::InViewWaiting::drain_s`]).
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
        let mut t = format!(
            "in view {} waiting (near {near} mid {mid} far {far}; no record {blank} coarse {coarse}), need {:.3e} samples, drain {}",
            w.total(),
            w.samples as f64,
            drain_words(self.drain_s()),
        );
        if let Some(r) = &self.rule_missed {
            t.push_str(&rule_missed_text(r));
        }
        if let Some(a) = &self.rule_audit {
            t.push_str(&rule_audit_text(a));
        }
        t
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
        if let Some(r) = &self.rule_missed {
            let list = |v: &[u64; 3]| format!("[{}, {}, {}]", v[0], v[1], v[2]);
            let _ = write!(
                o,
                ", \"rule_missed\": {{\"jobs\": {}, \"samples\": {}, \"before_jobs\": {}, \"before_samples\": {}, \"opened\": [{}, {}], \"opened_before\": [{}, {}], \"landed_at\": {}}}",
                list(&r.jobs),
                list(&r.samples),
                list(&r.before_jobs),
                list(&r.before_samples),
                r.opened[0],
                r.opened[1],
                r.opened_before[0],
                r.opened_before[1],
                r.landed_at
                    .map_or_else(|| "null".to_owned(), |s| s.to_string()),
            );
        }
        if let Some(a) = &self.rule_audit {
            let list = |v: &[u64; 4]| format!("[{}, {}, {}, {}]", v[0], v[1], v[2], v[3]);
            let first: Vec<String> = a
                .kept()
                .iter()
                .map(|d| {
                    format!(
                        "{{\"chunk\": [{}, {}, {}], \"case\": \"{}\", \"word\": {}, \"job\": {}, \"rule\": {}}}",
                        d.chunk[0],
                        d.chunk[1],
                        d.chunk[2],
                        AUDIT_CASES[usize::from(d.case)],
                        d.word,
                        d.job,
                        d.rule,
                    )
                })
                .collect();
            let _ = write!(
                o,
                ", \"rule_audit\": {{\"cases\": [\"full\", \"provisional\", \"partial\", \"class1\"], \"compared\": {}, \"differ\": {}, \"tier_masked\": {}, \"first\": [{}]}}",
                list(&a.compared),
                list(&a.differ),
                a.tier_masked,
                first.join(", "),
            );
        }
        o.push('}');
    }
}

/// The audit's cases by index ([`svet_render::RuleAudit`]).
pub const AUDIT_CASES: [&str; 4] = ["full", "provisional", "partial", "class1"];

/// The rule audit's text on the `bake queue:` line: the blocks compared with the zero record
/// and those that differ, by case, the tier-masked count, and the first that differs (chunk,
/// case, word index, the job's word and the rule's).
pub fn rule_audit_text(a: &svet_render::RuleAudit) -> String {
    let mut t = String::from("; rule audit compared/differ");
    for (k, name) in AUDIT_CASES.iter().enumerate() {
        let _ = write!(t, " {name} {}/{}", a.compared[k], a.differ[k]);
    }
    let _ = write!(t, ", tier masked {}", a.tier_masked);
    if let Some(d) = a.kept().first() {
        let _ = write!(
            t,
            ", first differ chunk {:?} {} word {} job {:#010x} rule {:#010x}",
            d.chunk,
            AUDIT_CASES[usize::from(d.case)],
            d.word,
            d.job,
            d.rule,
        );
    }
    t
}

/// The rule reading's text on the `bake queue:` line: the jobs (and samples) the rule would
/// have answered by exclusion (provisional, partial, class 1), whole and before the in-view
/// landing in view, against every job opened and those opened before the landing.
pub fn rule_missed_text(r: &svet_render::RuleMissed) -> String {
    let [sj, ss] = r.before_shares();
    format!(
        "; rule missed jobs prov/partial/class1 {}/{}/{} ({:.3e}/{:.3e}/{:.3e} samples), before the in-view landing in view {}/{}/{} ({:.3e}/{:.3e}/{:.3e}), of {} jobs {:.3e} samples opened, {} jobs {:.3e} before the landing ({:.1} % of jobs, {:.1} % of samples), landed at step {}",
        r.jobs[0],
        r.jobs[1],
        r.jobs[2],
        r.samples[0] as f64,
        r.samples[1] as f64,
        r.samples[2] as f64,
        r.before_jobs[0],
        r.before_jobs[1],
        r.before_jobs[2],
        r.before_samples[0] as f64,
        r.before_samples[1] as f64,
        r.before_samples[2] as f64,
        r.opened[0],
        r.opened[1] as f64,
        r.opened_before[0],
        r.opened_before[1] as f64,
        sj * 100.0,
        ss * 100.0,
        r.landed_at
            .map_or_else(|| "-".to_owned(), |s| s.to_string()),
    )
}

/// A drain's seconds in words: `4.0 s`, or `n/a` for `None`.
pub fn drain_words(s: Option<f64>) -> String {
    s.map_or_else(|| "n/a".to_owned(), |s| format!("{s:.1} s"))
}
