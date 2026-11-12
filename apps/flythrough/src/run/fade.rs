//! The fade reading (`--fade-reading`): the screen centre's light by source while the lamp
//! goes off and on.

use crate::percentile;
use vxe_app::Gpu;
use vxe_render::{Frame, wgpu};

/// `--fade-reading`'s digs along the camera ray, one a frame from the first.
pub(crate) const FADE_DIGS: u32 = 5;

/// Frames the light settles in before each switch: the lamp off (the ore's light alone, the
/// baseline), then on.
pub(crate) const FADE_SETTLE: u32 = 256;

/// Frames read after the lamp goes off.
pub(crate) const FADE_TAIL: u32 = 200;

/// Frames the baseline and the lamp-on values average over, the last of each settle.
pub(crate) const FADE_MEAN: usize = 32;

/// A source whose lamp-on value exceeds its baseline by no more than this has no fade to
/// read.
pub(crate) const FADE_NO_LAMP: f64 = 1e-4;

/// The frame `--fade-reading` exits at.
pub(crate) const FADE_END: u32 = FADE_DIGS + 2 * FADE_SETTLE + FADE_TAIL;

/// Lighting pixels on each edge of the block around the screen centre a sample averages.
pub(crate) const FADE_BLOCK: u32 = 8;

/// The frames after the switch the bar is read over: under 10 % from FADE_BY through
/// FADE_THROUGH.
pub(crate) const FADE_BY: usize = 24;

pub(crate) const FADE_THROUGH: usize = 160;

/// Whether `--fade-reading`'s lamp is on at frame `frame`.
pub(crate) fn fade_lamp(frame: u32) -> bool {
    (FADE_DIGS + FADE_SETTLE..FADE_DIGS + 2 * FADE_SETTLE).contains(&frame)
}

/// One frame of `--fade-reading`: luminances averaged over the FADE_BLOCK square of lighting
/// pixels at the screen centre (the composite's output over the same square). `direct` is
/// the filter's irradiance (the light pass through its temporal history and the a-trous
/// filter, lamp and emissive), `lamp` and `emissive` the light pass's own estimate of each
/// before any history, `indirect` the probe gather's (which runs no filter); the composite
/// shades a hit with `albedo (direct + indirect) / PI`, so `lit`, their sum, is the wall's
/// radiance over its albedo. `output` is the composite's display-encoded output in [0, 1].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FadeSample {
    pub(crate) lit: f64,
    pub(crate) direct: f64,
    pub(crate) lamp: f64,
    pub(crate) emissive: f64,
    pub(crate) indirect: f64,
    pub(crate) output: f64,
    pub(crate) gpu_ms: f64,
}

impl FadeSample {
    pub(crate) fn read(gpu: &Gpu, frame: &Frame, gpu_ms: f64) -> Self {
        let lum = |p: [f32; 4]| {
            0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])
        };
        let block = |t: &wgpu::Texture, edge: u32| {
            let (w, h) = (t.width(), t.height());
            let x0 = (w / 2).saturating_sub(edge / 2);
            let y0 = (h / 2).saturating_sub(edge / 2);
            move |i: u32| ((y0 + i / edge).min(h - 1) * w + (x0 + i % edge).min(w - 1)) as usize
        };
        let mean = |v: &[[f32; 4]], t: &wgpu::Texture, edge: u32, f: &dyn Fn([f32; 4]) -> f64| {
            let at = block(t, edge);
            (0..edge * edge).map(|i| f(v[at(i)])).sum::<f64>() / f64::from(edge * edge)
        };
        let (dev, q) = (&gpu.device, &gpu.queue);
        let filtered = vxe_render::debug::read_rgba16f(dev, q, frame.filtered());
        let raw = vxe_render::debug::read_rgba16f(dev, q, frame.irradiance());
        let indirect = vxe_render::debug::read_rgba16f(dev, q, frame.indirect_irradiance());
        let out_tex = frame.target();
        let out: Vec<[f32; 4]> = vxe_render::debug::read_rgba8(dev, q, out_tex)
            .chunks(4)
            .map(|p| [0, 1, 2, 3].map(|k| f32::from(p[k]) / 255.0))
            .collect();
        let scale = (out_tex.width() / frame.filtered().width()).max(1);
        let direct = mean(&filtered, frame.filtered(), FADE_BLOCK, &lum);
        let indirect = mean(&indirect, frame.indirect_irradiance(), FADE_BLOCK, &lum);
        Self {
            lit: direct + indirect,
            direct,
            lamp: mean(&raw, frame.irradiance(), FADE_BLOCK, &|p| f64::from(p[3])),
            emissive: mean(&raw, frame.irradiance(), FADE_BLOCK, &lum),
            indirect,
            output: mean(&out, out_tex, FADE_BLOCK * scale, &lum),
            gpu_ms,
        }
    }
}

/// One source of a [`FadeSample`], read out of it.
pub(crate) type FadeSource = fn(&FadeSample) -> f64;

/// `--fade-reading`'s report: for each source of [`FadeSample`], its baseline (the mean of
/// the last FADE_MEAN frames before the lamp first goes on), its lamp-on value (the last
/// FADE_MEAN with it on), the first frame after the switch whose excess over the baseline is
/// under 10 % of the lamp-on excess, the frames from FADE_BY through FADE_THROUGH over it, and
/// the first 32 frames of the tail.
pub(crate) fn fade_report(samples: &[FadeSample]) -> Vec<String> {
    let settle = FADE_SETTLE as usize;
    if samples.len() < 2 * settle + FADE_THROUGH {
        return vec![format!(
            "fade: {} samples, too few for a reading",
            samples.len()
        )];
    }
    let mut tail_ms: Vec<f64> = samples[2 * settle..].iter().map(|s| s.gpu_ms).collect();
    let ms = percentile(&mut tail_ms, 50.0);
    let mut lines = vec![format!(
        "fade: the lamp off at frame {} after {} frames on; the frame's GPU time {ms:.3} ms (median of the tail)",
        FADE_DIGS + 2 * FADE_SETTLE,
        FADE_SETTLE
    )];
    let sources: [(&str, FadeSource); 6] = [
        ("composite (direct + indirect)", |s| s.lit),
        ("direct (filtered)", |s| s.direct),
        ("lamp (light pass)", |s| s.lamp),
        ("emissive (light pass)", |s| s.emissive),
        ("indirect (gather)", |s| s.indirect),
        ("output (display-encoded)", |s| s.output),
    ];
    let avg = |r: &[FadeSample], f: FadeSource| r.iter().map(f).sum::<f64>() / r.len() as f64;
    for (name, f) in sources {
        let base = avg(&samples[settle - FADE_MEAN..settle], f);
        let on = avg(&samples[2 * settle - FADE_MEAN..2 * settle], f);
        let tail: Vec<f64> = samples[2 * settle..].iter().map(f).collect();
        if on - base <= FADE_NO_LAMP {
            lines.push(format!(
                "fade: {name}: off {base:.5}, on {on:.5}; the lamp adds nothing here"
            ));
            continue;
        }
        let bar = 0.1 * (on - base);
        let under = |v: f64| v - base < bar;
        let first = tail.iter().position(|&v| under(v)).map(|i| i + 1);
        let over: Vec<usize> = (FADE_BY..=FADE_THROUGH)
            .filter(|&n| !under(tail[n - 1]))
            .collect();
        let first_text = first.map_or("not reached".to_owned(), |n| {
            format!("{n} ({:.0} ms)", n as f64 * ms)
        });
        lines.push(format!(
            "fade: {name}: off {base:.5}, on {on:.5}; first frame under 10 %: {first_text}; frames {FADE_BY}..={FADE_THROUGH} over it: {}; frames 1..=32 {:?}",
            if over.is_empty() {
                "none".to_owned()
            } else {
                format!("{} ({:?})", over.len(), &over[..over.len().min(8)])
            },
            tail[..32].iter().map(|v| (v * 1e4).round() / 1e4).collect::<Vec<_>>()
        ));
    }
    lines
}
