//! The live queue's own rows: its order, its weights, the withdrawal, the bake thread and the
//! reading switches.

use super::flags::Parsing;
use super::table::Flag;

/// The live queue's order, weights and withdrawal, then its reading switches.
pub(super) fn queue_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::value(
            &["--queue-order"],
            "score|distance",
            "the live queue's order",
            |p: &mut Parsing, v| {
                p.args.bake.queue_distance =
                    v.word(&[("score", false), ("distance", true)], "score or distance")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--queue-weights"],
            "A,R|A,tier",
            "the score's angle and refine weights",
            |p: &mut Parsing, v| {
                let bad = || v.takes("A,R or A,tier");
                let (a, r) = v.text.split_once(',').ok_or_else(bad)?;
                let b = &mut p.args.bake;
                b.queue_angle = a
                    .parse()
                    .ok()
                    .filter(|x: &f32| x.is_finite() && *x >= 0.0)
                    .ok_or_else(bad)?;
                if r == "tier" {
                    b.queue_tier = true;
                } else {
                    b.queue_refine = r
                        .parse()
                        .ok()
                        .filter(|x: &f32| x.is_finite() && *x > 0.0)
                        .ok_or_else(bad)?;
                    b.queue_tier = false;
                }
                Ok(())
            },
        ),
        Flag::value(
            &["--queue-withdraw"],
            "on|off",
            "withdraw the jobs of chunks the camera passed",
            |p: &mut Parsing, v| {
                p.args.bake.queue_withdraw = v.on_off()?;
                Ok(())
            },
        ),
        Flag::value(
            &["--bake-thread"],
            "on|off",
            "run the live queue's sliced steps on the bake thread",
            |p: &mut Parsing, v| {
                p.args.bake.thread = Some(v.on_off()?);
                Ok(())
            },
        ),
        Flag::value(
            &["--bake-depth"],
            "N",
            "the bake thread's submits in flight, 1 to 8",
            |p: &mut Parsing, v| {
                p.args.bake.depth = v
                    .text
                    .parse()
                    .ok()
                    .filter(|d| (1..=8).contains(d))
                    .ok_or_else(|| v.takes("1 to 8"))?;
                Ok(())
            },
        ),
        Flag::value(
            &["--bake-slots"],
            "N",
            "the bake thread's table slots, 16 to 64",
            |p: &mut Parsing, v| {
                p.args.bake.slots = v
                    .text
                    .parse()
                    .ok()
                    .filter(|d| (16..=64).contains(d))
                    .ok_or_else(|| v.takes("16 to 64"))?;
                Ok(())
            },
        ),
        Flag::switch(
            &["--queue-census"],
            "read the queue's view census at every opening, its listing once",
            |p: &mut Parsing| {
                p.args.bake.queue_census = true;
            },
        ),
        Flag::switch(
            &["--queue-rule-reading"],
            "count the jobs opened for chunks the zero rule would answer, and hash the zeroed chunks' blocks and the store's once the queue is empty and at the end (a reading)",
            |p: &mut Parsing| {
                p.args.bake.queue_rule_reading = true;
            },
        ),
        Flag::switch(
            &["--queue-rule-audit"],
            "the zero rule writes no record; compare each job it would answer with its record (a reading; implies --queue-rule-reading)",
            |p: &mut Parsing| {
                p.args.bake.queue_rule_reading = true;
                p.args.bake.queue_rule_audit = true;
            },
        ),
    ]
}
