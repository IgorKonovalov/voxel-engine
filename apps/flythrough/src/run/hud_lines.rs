//! The stats HUD's and the exit report's free line formatters: probe, field, band, pool, belt,
//! shadow-ray, steps and look lines, and the banner's row colours.

use crate::app::ms_since;
use crate::now;
use crate::run::hud_feed::SAMPLES;
use crate::{LookPreset, percentile};
use svet_render::debug::StepHistogram;
use svet_render::{BELT_CHUNK_BYTES, FRAME_PASSES, GpuWorld, HudColour, LookParams, ShadowWalk};

/// The probe volume's bytes inside `Frame::frame_bytes`, by part.
pub(crate) fn probe_bytes_line(p: &svet_render::ProbeVolume) -> String {
    format!(
        "probe volume {} B in frame_bytes (atlases {} B, state {} B, update list, rays and uniform {} B), spacing {} voxels",
        p.bytes(),
        p.atlas_bytes(),
        p.state_bytes(),
        p.bytes() - p.atlas_bytes() - p.state_bytes(),
        p.spacing()
    )
}

/// The probe update and gather passes' GPU time in a frame's `FRAME_PASSES` times.
pub(crate) fn probe_passes_ms(passes: &[f64]) -> f64 {
    FRAME_PASSES
        .iter()
        .zip(passes)
        .filter(|(name, _)| matches!(**name, "probes" | "gather"))
        .map(|(_, ms)| ms)
        .sum()
}

/// `--lamp-toggle`'s exit line: the probes plus gather median of the last [`SAMPLES`] frames
/// outside and inside a light-change window.
pub(crate) fn window_line(every: u32, ms: &[Vec<f64>; 2]) -> String {
    let tail = |v: &Vec<f64>| {
        let mut t = v[v.len().saturating_sub(SAMPLES)..].to_vec();
        if t.is_empty() {
            "n/a".to_owned()
        } else {
            format!("{:.3} ms", percentile(&mut t, 50.0))
        }
    };
    format!(
        "lamp toggled every {every} frames: probes + gather median of the last {SAMPLES} outside a light-change window {} ({} frames), inside {} ({} frames)",
        tail(&ms[0]),
        ms[0].len(),
        tail(&ms[1]),
        ms[1].len()
    )
}

/// The field surface's band slots in the mirror: the band bricks holding one and their units'
/// bytes (each slot's whole bin, a uniform band brick's own slot included), inside the
/// mirror's live bytes.
pub(crate) fn field_line(mirror: &GpuWorld) -> String {
    format!(
        "field band: {} band bricks, {} B of band slots, mirror {} B live, {} B allocated",
        mirror.band_bricks(),
        mirror.band_units() * svet_render::internals::UNIT_BYTES,
        mirror.mirror_bytes(),
        mirror.allocated_bytes()
    )
}

/// The band census (`flythrough::band_census`) over the chunks `mirror` holds full, against
/// the mirror's live bytes.
pub(crate) fn band_census_line(world: &svet_world::World, mirror: &GpuWorld) -> String {
    let chunks: Vec<svet_world::ChunkCoord> = world
        .store()
        .coords()
        .into_iter()
        .filter(|&c| mirror.is_mirrored(c) && !mirror.is_lod(c))
        .collect();
    let t = now();
    let cs = crate::band_census(world, &chunks);
    format!(
        "band census over {} full chunks in {:.0} ms: band voxels {} solid, {} air; band bricks {} Full, {} uniform solid, {} uniform air ({} in all); Full bricks with a surface voxel {}, of them band {}; bytes packed {}, dense {}; mirror {} B live",
        chunks.len(),
        ms_since(t),
        cs.solid_voxels,
        cs.air_voxels,
        cs.full_bricks,
        cs.solid_bricks,
        cs.air_bricks,
        cs.bricks(),
        cs.surface_full_bricks,
        cs.surface_full_band_bricks,
        cs.packed_bytes,
        cs.dense_bytes,
        mirror.mirror_bytes()
    )
}

/// The pool's bins at a moment: per bin with a page, its slot size, live slots, spare units
/// (free slots and the rest of its cursor's page) and pages, then the spare units over every
/// bin against the live units, e.g. `bins: 2u 10 live 6 spare 1p, 3u 4 live 0 spare 1p;
/// spare 6 of 32 live units (18.75 %)`.
pub(crate) fn bins_line(mirror: &GpuWorld) -> String {
    let bins: Vec<String> = mirror
        .bin_stats()
        .iter()
        .filter(|b| b.pages > 0)
        .map(|b| {
            format!(
                "{}u {} live {} spare {}p",
                b.units,
                b.live,
                b.spare * u64::from(b.units),
                b.pages
            )
        })
        .collect();
    let (spare, live) = (mirror.spare_units(), mirror.live_units());
    format!(
        "bins: {}; spare {spare} of {live} live units ({:.2} %)",
        bins.join(", "),
        100.0 * spare as f64 / live.max(1) as f64
    )
}

/// `field pass: 383608 band slots in 12.345 ms GPU (initial fill)`.
pub(crate) fn field_pass_line(what: &str, (n, ms): (usize, Option<f64>)) -> String {
    let ms = ms.map_or("n/a (no TIMESTAMP_QUERY)".to_owned(), |ms| {
        format!("{ms:.3} ms")
    });
    format!("field pass: {n} band slots in {ms} GPU ({what})")
}

/// The mirror's staging belt: its chunks, the host bytes they map at the chunk size, and how
/// often a burst replaced it.
pub(crate) fn belt_line(mirror: &GpuWorld) -> String {
    let n = mirror.belt_chunks();
    format!(
        "staging belt {n} chunks ({} B mapped), {} trims",
        n as u64 * BELT_CHUNK_BYTES,
        mirror.belt_trims()
    )
}

/// The steps view's reading of the lamp shadow rays, from the light pass's counters (one
/// texel per hit, alpha 1): `shadow rays (full walk, 1 234 567 rays): steps mean 8.12, loads
/// mean 20.31, p99 64, pool reads mean 3.10`.
pub(crate) fn shadow_steps_line(texels: &[[f32; 4]], walk: ShadowWalk) -> String {
    let rays: Vec<&[f32; 4]> = texels.iter().filter(|t| t[3] > 0.5).collect();
    let n = rays.len().max(1) as f64;
    let mean = |k: usize| rays.iter().map(|t| f64::from(t[k])).sum::<f64>() / n;
    let mut loads: Vec<f64> = rays.iter().map(|t| f64::from(t[1])).collect();
    format!(
        "shadow rays ({} walk, {} pixels cast, {:.3} a casting pixel): steps mean {:.2}, loads mean {:.2}, p99 {:.0}, pool reads mean {:.2}",
        walk.name(),
        rays.len(),
        mean(3),
        mean(0),
        mean(1),
        percentile(&mut loads, 99.0),
        mean(2)
    )
}

/// The key light's shadow ray in the steps view, from the light pass's direct target (one
/// texel per hit, alpha 1 where the ray was cast): the pixels that cast it, its mean steps,
/// p99 steps, mean storage loads and mean pool reads; `None` where no pixel cast it.
pub(crate) fn key_ray_steps(texels: &[[f32; 4]]) -> Option<(usize, [f64; 4])> {
    let rays: Vec<&[f32; 4]> = texels.iter().filter(|t| t[3] > 0.5).collect();
    if rays.is_empty() {
        return None;
    }
    let n = rays.len() as f64;
    let mean = |k: usize| rays.iter().map(|t| f64::from(t[k])).sum::<f64>() / n;
    let mut steps: Vec<f64> = rays.iter().map(|t| f64::from(t[0])).collect();
    Some((
        rays.len(),
        [mean(0), percentile(&mut steps, 99.0), mean(1), mean(2)],
    ))
}

/// The steps view's reading of the key light's shadow ray ([`key_ray_steps`]): `key light ray
/// (coarse walk, 1 234 567 pixels cast): steps mean 8.12, p99 64, loads mean 20.31, pool reads
/// mean 3.10`, or `key light ray: none cast`.
pub(crate) fn key_steps_line(texels: &[[f32; 4]], walk: ShadowWalk) -> String {
    match key_ray_steps(texels) {
        Some((n, [mean, p99, loads, pool])) => format!(
            "key light ray ({} walk, {n} pixels cast): steps mean {mean:.2}, p99 {p99:.0}, loads mean {loads:.2}, pool reads mean {pool:.2}",
            walk.name()
        ),
        None => "key light ray: none cast".to_owned(),
    }
}

/// The steps view's reading of one frame: `steps per ray (2073600 rays): mean 41.27, p99 133,
/// block-max mean 88.10; loads mean 12.40, p99 61; pool reads mean 3.02, p99 18`. Counts clamp
/// at 255 as the view does; the block-max mean is over 8 × 8 pixel blocks of each block's
/// maximum steps; loads are the ray's 32-bit storage loads from the mirror, pool reads the
/// pool loads among them.
pub(crate) fn steps_line(h: &StepHistogram) -> String {
    format!(
        "steps per ray ({} rays): mean {:.2}, p99 {}, block-max mean {:.2}; loads mean {:.2}, p99 {}; pool reads mean {:.2}, p99 {}",
        h.rays(),
        h.mean_steps(),
        h.p99_steps(),
        h.block_max_mean(),
        h.mean_loads(),
        h.p99_loads(),
        h.mean_pool_reads(),
        h.p99_pool_reads()
    )
}

/// The preset's name and parts, the far normal and the view: `look field (normals contour,
/// face mix 0, far normal off), view shaded`.
pub(crate) fn look_line(look: &LookParams, preset: &LookPreset) -> String {
    format!(
        "look {} (normals {}, {}, {}), view {}",
        preset.name,
        look.normal_mode.name(),
        preset.parts(),
        LookPreset::far_normal_words(look.normal_mode, look.far_normal),
        look.view.name()
    )
}

/// A banner row's colour: red for the model viewer's error row, yellow otherwise.
pub(crate) fn banner_colour(line: &str) -> HudColour {
    if crate::viewer::is_error_row(line) {
        HudColour::Red
    } else {
        HudColour::Yellow
    }
}

/// Each of `banner`'s rows' colours into `colours`, cleared first ([`banner_colour`]).
pub(crate) fn banner_colours(banner: &str, colours: &mut Vec<HudColour>) {
    colours.clear();
    colours.extend(banner.lines().map(banner_colour));
}
