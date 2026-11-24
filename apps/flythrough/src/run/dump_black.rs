//! `--dump-black`: the lighting pixels with no record value, by the chunk they hit.

use crate::run::Run;
use svet_app::Gpu;
use svet_render::GpuWorld;
use svet_world::ChunkCoord;

/// Which record a lighting pixel's hit reads in the store: `None` where its voxel's brick has a
/// record, else whether its chunk holds a block (the brick then lacks a record).
fn hit_without_record(store: &svet_render::Store, hit: [f32; 4]) -> Option<(ChunkCoord, bool)> {
    let v = [0, 1, 2].map(|k| hit[k].floor() as i32);
    let c = ChunkCoord(v.map(|x| x.div_euclid(64)));
    let Some(words) = store.block_words(c) else {
        return Some((c, false));
    };
    let slot = svet_world::brick_slot(v.map(|x| x.rem_euclid(64) / 8));
    (words.get(slot).is_none_or(|&d| d == 0)).then_some((c, true))
}

/// `--dump-black`'s lines: the lighting pixels whose hit (`hits`, the frame's
/// `light_hit_positions` read back) reads no record value in `frame`'s store (the store's
/// `missing`, of which the chain leaves `black` unlit), grouped by the chunk they hit, nearest
/// the eye (`view.eye`) first, each with the chunk's distance in chunks, whether it holds a
/// block, its queue tracks, the mirror's state, whether its reach crosses the full radius by
/// the queue's test ([`svet_render::reach_crosses`]) and
/// the chunks that hold its columns from final ([`svet_render::finality_blockers`]). A header
/// line first.
pub fn black_dump_lines(
    mirror: &GpuWorld,
    frame: &svet_render::Frame,
    hits: &[[f32; 4]],
    view: &svet_render::QueueView<'_>,
    at_frame: u32,
    black: u32,
) -> Vec<String> {
    let store = frame.store();
    let mut by = std::collections::BTreeMap::<[i32; 3], (u32, bool)>::new();
    for h in hits.iter().filter(|h| h[3] < 1.0e30) {
        if let Some((c, block)) = hit_without_record(store, *h) {
            by.entry(c.0).or_insert((0, block)).0 += 1;
        }
    }
    let p = view.eye;
    let e = p.map(|v| (v.floor() as i32).div_euclid(64));
    let lr = mirror.light_radius();
    let mut rows: Vec<_> = by
        .into_iter()
        .map(|(c, (n, block))| {
            let d = (0..3)
                .map(|k| f64::from(c[k] - e[k]).powi(2))
                .sum::<f64>()
                .sqrt();
            (d, c, n, block)
        })
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = vec![format!(
        "dump black: frame {at_frame}, eye x {:.1} y {:.1} z {:.1} (chunk {} {} {}), black {black}, {} pixels with no record value over {} chunks",
        p[0],
        p[1],
        p[2],
        e[0],
        e[1],
        e[2],
        rows.iter().map(|r| r.2).sum::<u32>(),
        rows.len()
    )];
    let q = frame.bake_queue();
    for (d, c, n, block) in rows {
        let cc = ChunkCoord(c);
        let blockers = svet_render::finality_blockers(mirror, cc, lr, view);
        let crosses = view.full_radius.map_or("no streamer".to_owned(), |r| {
            svet_render::reach_crosses(cc, e, lr, r).to_string()
        });
        out.push(format!(
            "dump black: chunk {} {} {}: {n} px, {d:.1} chunks out, block {}, tracks {:?}, mirrored {} lod {} pending {}, reach crosses the full radius (the queue's test) {crosses}, {} blockers {:?}",
            c[0],
            c[1],
            c[2],
            if block {
                "yes (brick without a record)"
            } else {
                "no"
            },
            q.map(|q| q.tracks(cc)).unwrap_or_default(),
            mirror.is_mirrored(cc),
            mirror.is_lod(cc),
            mirror.is_pending(cc),
            blockers.len(),
            blockers
                .iter()
                .take(4)
                .map(|(b, pending)| (b.0, *pending))
                .collect::<Vec<_>>(),
        ));
    }
    out
}

impl Run {
    /// `--dump-black`: [`black_dump_lines`] printed, the hit positions read back blocking, so
    /// called at most once a second and only under the flag.
    pub(crate) fn dump_black(&mut self, gpu: &Gpu, black: u32) {
        let Some(gfx) = self.gfx.as_ref() else {
            return;
        };
        let hits = svet_render::debug::read_rgba32f(
            &gpu.device,
            &gpu.queue,
            gfx.frame.light_hit_positions(),
        );
        let stream = self.stream.as_ref();
        let resident_fn = |c: ChunkCoord| {
            stream.is_some_and(|s| s.streamer.tier(c) == Some(svet_world::Tier::Full))
        };
        let view = svet_render::QueueView {
            eye: self.fly.camera.position.to_array(),
            forward: self.fly.camera.basis().0.to_array(),
            resident: stream
                .is_some()
                .then_some(&resident_fn as &dyn Fn(ChunkCoord) -> bool),
            samples: None,
            interval_s: 0.0,
            hold: None,
            half_angle: 1.0,
            frustum: None,
            full_radius: stream.map(|s| s.rule.r_full),
            lod_radius: stream.map(|s| {
                let r = s.streamer.config().residency;
                r.r_lod.max(r.r_full)
            }),
            focal_px: None,
        };
        for l in black_dump_lines(&gfx.mirror, &gfx.frame, &hits, &view, self.frame, black) {
            println!("{l}");
        }
    }
}
