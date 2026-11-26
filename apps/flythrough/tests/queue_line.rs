//! The live queue's in-view drain readout: the census of the chunks in view waiting for their
//! light by band, the `bake queue:` line's text and `run.json`'s `queue` object. CPU only.

use flythrough::report::QueueDrain;
use flythrough::report::json::{Value, parse};
use svet_render::{Band, Bands, QueueView, in_view_census};

/// The fly-through's vertical field, 70 degrees.
const VFOV: f32 = 70.0 * std::f32::consts::PI / 180.0;

/// A view from the centre of chunk (0, 0, 0) looking down +x, at 1080p.
fn view() -> QueueView {
    QueueView {
        eye: [32.0, 32.0, 32.0],
        forward: [1.0, 0.0, 0.0],
        resident: None,
        samples: None,
        interval_s: 0.0,
        hold: None,
        half_angle: 35f32.to_radians(),
        frustum: None,
        full_radius: None,
        lod_radius: None,
        focal_px: Some(Bands::focal_px(1080, VFOV)),
    }
}

/// A synthetic view of 12 chunks down +x at 1080p and the far-normal band `[2, 6]` (f about
/// 771 px, so a voxel spans 6 px at 128.5 voxels and 2 px at 385.6): 4 near (nearest point 32
/// to 101 voxels), 4 mid (160 to 352), 4 far (416 to 608); in each band 2 without a record and
/// 2 holding a full one, each waiting chunk needing 1e6 samples. At 2e6 samples a second the
/// line reads `in view 6 waiting (near 2 mid 2 far 2; ...)` and a drain of 6e6 / 2e6 = 3.0 s,
/// and `run.json`'s `queue` carries the same counts and `drain_s`. A chunk behind the camera
/// with no record is not in view and not counted.
#[test]
fn the_queue_line_reports_in_view_waiting_by_band_and_a_drain_projection() {
    let v = view();
    let bands = Bands {
        focal_px: v.focal_px.expect("set"),
        lo: 2.0,
        hi: 6.0,
    };
    let near = [[1, 0, 0], [2, 0, 0], [1, 1, 0], [2, 1, 0]];
    let mid = [[3, 0, 0], [4, 0, 0], [5, 0, 0], [6, 0, 0]];
    let far = [[7, 0, 0], [8, 0, 0], [9, 0, 0], [10, 0, 0]];
    for (set, band) in [(near, Band::Near), (mid, Band::Mid), (far, Band::Far)] {
        for c in set {
            assert_eq!(bands.of_chunk(c, v.eye), band, "{c:?}");
        }
    }
    let mut chunks = Vec::new();
    for set in [near, mid, far] {
        for (i, c) in set.into_iter().enumerate() {
            // The first two of each band hold no record; the other two a full one.
            let has = u8::from(i >= 2);
            chunks.push((c, has, false, 1_000_000u64));
        }
    }
    assert_eq!(chunks.len(), 12);
    chunks.push(([-3, 0, 0], 0, false, 1_000_000));
    let waiting = in_view_census(&v, 2, [2.0, 6.0], chunks);
    assert_eq!(waiting.chunks, [[2, 0], [2, 0], [2, 0]]);
    assert_eq!(waiting.samples, 6_000_000);
    let drain = QueueDrain {
        depth: 13,
        waiting,
        samples_per_s: 2.0e6,
        rule_missed: None,
        rule_audit: None,
    };
    assert_eq!(drain.drain_s(), Some(3.0));
    let text = drain.text();
    assert!(
        text.starts_with("in view 6 waiting (near 2 mid 2 far 2; no record 6 coarse 0)"),
        "{text}"
    );
    assert!(
        text.ends_with("need 6.000e6 samples, drain 3.0 s"),
        "{text}"
    );
    let json = parse(&drain.json()).expect("parses");
    let num = |k: &str| match json.get(k) {
        Some(Value::Num(n)) => *n,
        other => panic!("{k}: {other:?}"),
    };
    assert_eq!(num("in_view_waiting"), 6.0);
    assert_eq!([num("near"), num("mid"), num("far")], [2.0, 2.0, 2.0]);
    assert_eq!([num("no_record"), num("coarse")], [6.0, 0.0]);
    assert_eq!(num("need_samples"), 6.0e6);
    assert_eq!(num("drain_s"), 3.0);
    assert_eq!(num("depth"), 13.0);
    // Nothing drawn while chunks wait: no projection.
    let idle = QueueDrain {
        samples_per_s: 0.0,
        ..drain
    };
    assert!(idle.text().ends_with("drain n/a"), "{}", idle.text());
    assert_eq!(
        parse(&idle.json()).expect("parses").get("drain_s"),
        Some(&Value::Null)
    );
    // The rule reading: 30 of 120 jobs before the landing (25 %), 1.0e6 of 8.0e6 samples.
    let r = svet_render::RuleMissed {
        jobs: [40, 5, 2],
        samples: [4_000_000, 500_000, 200_000],
        before_jobs: [25, 4, 1],
        before_samples: [800_000, 150_000, 50_000],
        opened: [300, 20_000_000],
        opened_before: [120, 8_000_000],
        landed_at: Some(900),
    };
    assert_eq!(r.before_shares(), [0.25, 0.125]);
    let read = QueueDrain {
        rule_missed: Some(r),
        ..drain
    };
    let text = read.text();
    assert!(
        text.contains("rule missed jobs prov/partial/class1 40/5/2")
            && text.contains("(25.0 % of jobs, 12.5 % of samples), landed at step 900"),
        "{text}"
    );
    assert!(
        read.json()
            .contains("\"rule_missed\": {\"jobs\": [40, 5, 2]"),
        "{}",
        read.json()
    );

    // The rule audit (Plan 0101 Phase 1): compared and differ by case, full / provisional /
    // partial / class 1, on the line and in `run.json`, with the first block that differs.
    let mut a = svet_render::RuleAudit {
        compared: [12, 7, 30, 2],
        differ: [0, 0, 1, 0],
        tier_masked: 4,
        ..Default::default()
    };
    a.first[0] = svet_render::AuditDiffer {
        chunk: [3, -2, 1],
        case: 2,
        word: 77,
        job: 5,
        rule: 0,
    };
    a.kept = 1;
    let audited = QueueDrain {
        rule_audit: Some(a),
        ..read
    };
    let text = audited.text();
    assert!(
        text.contains(
            "rule audit compared/differ full 12/0 provisional 7/0 partial 30/1 class1 2/0, tier masked 4, first differ chunk [3, -2, 1] partial word 77"
        ),
        "{text}"
    );
    let json = parse(&audited.json()).expect("parses");
    let audit = json.get("rule_audit").expect("the audit's object");
    let nums = |k: &str| match audit.get(k) {
        Some(Value::Arr(v)) => v
            .iter()
            .map(|x| match x {
                Value::Num(n) => *n,
                other => panic!("{k}: {other:?}"),
            })
            .collect::<Vec<f64>>(),
        other => panic!("{k}: {other:?}"),
    };
    assert_eq!(nums("compared"), [12.0, 7.0, 30.0, 2.0]);
    assert_eq!(nums("differ"), [0.0, 0.0, 1.0, 0.0]);
    let Some(Value::Arr(first)) = audit.get("first") else {
        panic!("first: {audit:?}");
    };
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].get("word"), Some(&Value::Num(77.0)));
    assert_eq!(first[0].get("case"), Some(&Value::Str("partial".into())));
    // No audit: no object.
    assert!(read.json().contains("rule_missed") && !read.json().contains("rule_audit"));
}
