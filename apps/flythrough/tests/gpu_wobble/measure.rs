//! Plan 0104 Phase 4: the two candidate measures and candidate 7's readings, scored on the CPU
//! (no device) from a clip's decoded frames (`ffmpeg` to raw RGB), its side file's exposure
//! and the `.keys` of its track rendered again (`keys.rs`). Nothing is asserted.
//!
//! - **Luma over exposure:** a display pixel's sRGB decoded to linear, Rec. 709 weights, over
//!   the exposure the frame was drawn with (the side file's value after the frame before: the
//!   side file holds the value the next frame uses).
//! - **Windows:** each segment (the fly, the stop's hold, the turn, the hold after) cut into
//!   55-frame windows from its start, a remainder under 30 frames joined to the window before;
//!   the turn (40 frames) one window. Extra windows a clip names are scored beside them.
//! - **A unit's series in a window:** its frames present (at least [`MIN_FRAMES`]; gaps hold the
//!   last value), its mean luma over exposure (candidate 1) and its lit share, the share of its
//!   samples over the mid luma between the clip's lit and shadowed levels (the medians of the
//!   samples the rig reads sunlit and shadowed, every 5th frame; candidate 2).
//! - **Candidate 1:** the series as a contrast against its mean, its least-squares line taken
//!   out, its DFT's power between 1 Hz (the lowest bin over it; the turn's 40 frames start at
//!   1.375 Hz) and 15 Hz, as an RMS amplitude.
//! - **Candidate 2:** the share of frame pairs a quarter second (14 frames) apart whose lit
//!   shares differ by over a tenth.
//! - **Dots (a reading beside candidate 1):** the share of frames where the unit's mean leaves
//!   its window median by over a tenth and is back within two frames.
//! - **Maps:** a unit's score lands in the 64-pixel tile of its samples' centroid each frame,
//!   weighted by its display pixels that frame over the window's frames; a tile's value is over
//!   its own area, a window's total over the picture's. A moment's map is the frame-weighted
//!   sum of its windows'.
//! - **Line (a):** the region holds at least twice its share of the scored tiles among the top
//!   tenth by score. Line (b) compares the clip totals (the frame-weighted mean of the windows').
//! - **Candidate 7:** the key slot's source at each sample whose lighting pixel faces the sun
//!   (as Phase 4 step 2's split compared): its maps, the
//!   source-0 ratio in the region against the picture, the slot's mean change a frame on units
//!   by source, and source 0's share in candidate 1's top tenth.
//!
//! `SVET_MEASURE=<clip>[,<clip>..]` (`T-a`, `T-b`, `C-a`, `C-b`, `S`), or `control` for `S`'s
//! exposure control. Output under `live/p4/`.

use std::collections::HashMap;
use std::io::Read;

use super::keys::STRIDE;
use super::*;

const W: usize = 2530;
const H: usize = 1378;
const TILE: usize = 64;
const TW: usize = W.div_ceil(TILE);
const TH: usize = H.div_ceil(TILE);
const MIN_FRAMES: usize = 30;
const PAIR: usize = 14;
const SW: usize = W.div_ceil(STRIDE as usize);
const SH: usize = H.div_ceil(STRIDE as usize);
const NS: usize = SW * SH;
const HOLD: usize = 165;
const TURN: usize = 40;

#[derive(Clone, Copy)]
enum Region {
    TopThird,
    TopLeftNinth,
    /// Tiles whose samples are at least 5 % sunlit leaf over the moment.
    SunlitLeaf,
    /// Tiles whose samples are at least 5 % sunlit and not leaf over the moment.
    SunlitOther,
}

struct Clip {
    name: &'static str,
    mkv: PathBuf,
    side: PathBuf,
    keys: PathBuf,
    fly: usize,
    frames: usize,
    artifact: char,
    region: Region,
    moment: (usize, usize),
    /// The frame a map is laid over: his screenshot's, else the moment's middle.
    shot: usize,
    extras: Vec<(usize, usize)>,
}

fn live() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../live")
}

fn clip_of(name: &str) -> Clip {
    let a = live().join("anchor");
    let p4 = live().join("p4");
    let (fly, artifact, region, moment, shot) = match name {
        "T-a" | "T-b" => (275, 'A', Region::TopThird, (110, 220), 203),
        "C-a" => (102, 'B', Region::TopLeftNinth, (55, 102), 73),
        "C-b" => (102, 'B', Region::SunlitLeaf, (267, 307), 287),
        "S" => (193, 'A', Region::SunlitOther, (358, 398), 378),
        other => panic!("clip {other}"),
    };
    let frames = fly + 2 * HOLD + TURN;
    let mut extras = vec![(fly, fly + 40), (fly + HOLD + TURN, fly + HOLD + TURN + 40)];
    if name == "S" {
        extras.push((398, 441));
    }
    Clip {
        name: match name {
            "T-a" => "T-a",
            "T-b" => "T-b",
            "C-a" => "C-a",
            "C-b" => "C-b",
            _ => "S",
        },
        mkv: a.join(format!("{name}.mkv")),
        side: a.join(format!("{name}.txt")),
        keys: p4.join(format!("{name}-k.keys")),
        fly,
        frames,
        artifact,
        region,
        moment,
        shot,
        extras,
    }
}

/// The exposure each frame was drawn with.
fn exposures(side: &Path, frames: usize) -> Vec<f32> {
    let txt = std::fs::read_to_string(side).unwrap_or_else(|e| panic!("{side:?}: {e}"));
    let after: Vec<f32> = txt
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("frame"))
        .map(|l| {
            l.split_whitespace()
                .nth(6)
                .expect("exposure")
                .parse()
                .expect("a float")
        })
        .collect();
    assert!(after.len() >= frames, "{side:?}: {} frames", after.len());
    (0..frames).map(|i| after[i.saturating_sub(1)]).collect()
}

/// Frames of `mkv`, RGB, in order.
struct Decoder {
    child: std::process::Child,
    out: std::process::ChildStdout,
}

impl Decoder {
    fn open(mkv: &Path) -> Self {
        let mut child = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(mkv)
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("ffmpeg starts");
        let out = child.stdout.take().expect("stdout");
        Self { child, out }
    }
    fn next(&mut self, buf: &mut [u8]) -> bool {
        self.out.read_exact(buf).is_ok()
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One frame of `mkv` by index.
fn frame_of(mkv: &Path, n: usize) -> Vec<u8> {
    let out = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(mkv)
        .args(["-vf", &format!("select=eq(n\\,{n})"), "-vframes", "1"])
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .expect("ffmpeg");
    assert_eq!(out.stdout.len(), W * H * 3, "{mkv:?} frame {n}");
    out.stdout
}

/// One frame's record of the `.keys`.
struct KeyFrame {
    ids: Vec<u32>,
    flags: Vec<u8>,
    vals: Vec<u8>,
}

fn read_keys(r: &mut impl Read) -> Option<KeyFrame> {
    let mut b = vec![0u8; NS * 4];
    r.read_exact(&mut b).ok()?;
    let ids = b
        .chunks(4)
        .map(|c| u32::from_le_bytes(c.try_into().expect("4")))
        .collect();
    let mut flags = vec![0u8; NS];
    r.read_exact(&mut flags).ok()?;
    let mut vals = vec![0u8; NS];
    r.read_exact(&mut vals).ok()?;
    Some(KeyFrame { ids, flags, vals })
}

fn lin_lut() -> [f32; 256] {
    std::array::from_fn(|i| {
        let c = i as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    })
}

/// The linear luma of display pixel `(x, y)` of `rgb`.
fn luma(rgb: &[u8], lut: &[f32; 256], x: usize, y: usize) -> f32 {
    let p = 3 * (y * W + x);
    0.2126 * lut[rgb[p] as usize]
        + 0.7152 * lut[rgb[p + 1] as usize]
        + 0.0722 * lut[rgb[p + 2] as usize]
}

fn sample_xy(s: usize) -> (usize, usize) {
    ((s % SW) * STRIDE as usize, (s / SW) * STRIDE as usize)
}

fn tile_of(x: f32, y: f32) -> usize {
    let tx = ((x as usize) / TILE).min(TW - 1);
    let ty = ((y as usize) / TILE).min(TH - 1);
    ty * TW + tx
}

/// A unit's sums in one frame.
#[derive(Default, Clone, Copy)]
struct Acc {
    lum: f32,
    n: u32,
    lit: u32,
    sx: f32,
    sy: f32,
    /// The slot's value sum and count at sources 0 and 2.
    s0: f32,
    n0: u32,
    s2: f32,
    n2: u32,
}

/// A unit's entry in one window's frame.
#[derive(Clone, Copy)]
struct E {
    f: u16,
    lum: f32,
    lit: f32,
    n: u16,
    tile: u16,
    k0: f32,
    k2: f32,
}

/// The per-tile sample counts of one window.
#[derive(Clone)]
struct Counts {
    all: Vec<f64>,
    /// Samples whose lighting pixel faces the sun (sunlit or shadowed, the pixels the key slot
    /// is read at); `src` and the slot's changes count only these.
    lighting: Vec<f64>,
    src: [Vec<f64>; 5],
    sunlit_leaf: Vec<f64>,
    sunlit_other: Vec<f64>,
}

impl Counts {
    fn new() -> Self {
        let z = vec![0.0; TW * TH];
        Self {
            all: z.clone(),
            lighting: z.clone(),
            src: std::array::from_fn(|_| z.clone()),
            sunlit_leaf: z.clone(),
            sunlit_other: z,
        }
    }
    fn add(&mut self, o: &Counts, w: f64) {
        let add =
            |a: &mut Vec<f64>, b: &Vec<f64>| a.iter_mut().zip(b).for_each(|(x, y)| *x += w * y);
        add(&mut self.all, &o.all);
        add(&mut self.lighting, &o.lighting);
        for k in 0..5 {
            add(&mut self.src[k], &o.src[k]);
        }
        add(&mut self.sunlit_leaf, &o.sunlit_leaf);
        add(&mut self.sunlit_other, &o.sunlit_other);
    }
}

/// One window's accumulation and its result.
struct Win {
    range: (usize, usize),
    units: HashMap<u32, Vec<E>>,
    counts: Counts,
}

/// A window's scored maps (a tile's value over its own samples) and totals.
#[derive(Clone)]
struct Maps {
    range: (usize, usize),
    c1: Vec<f64>,
    c2: Vec<f64>,
    dots: Vec<f64>,
    counts: Counts,
    /// The slot's mean change a frame on units at source 0 and 2, and the unit-frames read.
    dk0: (f64, usize),
    dk2: (f64, usize),
    units: usize,
}

impl Maps {
    fn total(v: &[f64], counts: &Counts) -> f64 {
        let a: f64 = counts.all.iter().sum();
        v.iter().zip(&counts.all).map(|(x, n)| x * n).sum::<f64>() / a.max(1.0)
    }
}

fn windows(c: &Clip) -> Vec<(usize, usize)> {
    let segs = [
        (0, c.fly),
        (c.fly, c.fly + HOLD),
        (c.fly + HOLD, c.fly + HOLD + TURN),
        (c.fly + HOLD + TURN, c.frames),
    ];
    let mut v = Vec::new();
    for (a, b) in segs {
        let mut s = a;
        while s < b {
            let e = if b - s < 55 + MIN_FRAMES { b } else { s + 55 };
            v.push((s, e));
            s = e;
        }
    }
    v
}

/// The DFT band power's RMS amplitude of `x` (contrast, line out) between 1 and 15 Hz.
fn band_rms(x: &[f32]) -> f64 {
    let n = x.len();
    let nf = n as f64;
    let m = x.iter().map(|&v| f64::from(v)).sum::<f64>() / nf;
    if m <= 1e-9 {
        return 0.0;
    }
    let c: Vec<f64> = x.iter().map(|&v| f64::from(v) / m - 1.0).collect();
    let tm = (nf - 1.0) / 2.0;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (i, v) in c.iter().enumerate() {
        sxy += (i as f64 - tm) * v;
        sxx += (i as f64 - tm).powi(2);
    }
    let slope = sxy / sxx.max(1e-9);
    let mean_c = c.iter().sum::<f64>() / nf;
    let r: Vec<f64> = c
        .iter()
        .enumerate()
        .map(|(i, v)| v - mean_c - slope * (i as f64 - tm))
        .collect();
    let mut p = 0.0;
    for k in 1..=n / 2 {
        let f = k as f64 * 55.0 / nf;
        let lo = if n <= TURN { 1.3 } else { 1.0 };
        if f < lo || f > 15.0 {
            continue;
        }
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in r.iter().enumerate() {
            let a = std::f64::consts::TAU * (k * i) as f64 / nf;
            re += v * a.cos();
            im -= v * a.sin();
        }
        let both = if 2 * k == n { 1.0 } else { 2.0 };
        p += both * (re * re + im * im) / (nf * nf);
    }
    p.sqrt()
}

fn score(w: Win) -> Maps {
    let len = w.range.1 - w.range.0;
    let mut c1 = vec![0.0; TW * TH];
    let mut c2 = vec![0.0; TW * TH];
    let mut dots = vec![0.0; TW * TH];
    let (mut dk0, mut nk0, mut dk2, mut nk2) = (0.0, 0usize, 0.0, 0usize);
    let px = f64::from(STRIDE * STRIDE);
    let mut scored = 0;
    for es in w.units.values() {
        // The slot's change a frame, by source, on consecutive frames.
        for p in es.windows(2) {
            if p[1].f != p[0].f + 1 {
                continue;
            }
            if p[0].k0 >= 0.0 && p[1].k0 >= 0.0 {
                dk0 += f64::from((p[1].k0 - p[0].k0).abs());
                nk0 += 1;
            }
            if p[0].k2 >= 0.0 && p[1].k2 >= 0.0 {
                dk2 += f64::from((p[1].k2 - p[0].k2).abs());
                nk2 += 1;
            }
        }
        if es.len() < MIN_FRAMES.min(len) {
            continue;
        }
        scored += 1;
        // The series over the window, gaps holding the last value.
        let mut lum = vec![0.0f32; len];
        let mut lit = vec![0.0f32; len];
        let mut have = vec![false; len];
        for e in es {
            let i = usize::from(e.f) - w.range.0;
            lum[i] = e.lum;
            lit[i] = e.lit;
            have[i] = true;
        }
        let first = have.iter().position(|&h| h).expect("an entry");
        for i in 0..len {
            if !have[i] {
                let j = if i < first { first } else { i - 1 };
                lum[i] = lum[j];
                lit[i] = lit[j];
            }
        }
        let s1 = band_rms(&lum);
        let pairs: Vec<bool> = (0..len.saturating_sub(PAIR))
            .filter(|&i| have[i] && have[i + PAIR])
            .map(|i| (lit[i] - lit[i + PAIR]).abs() > 0.1)
            .collect();
        let s2 = if pairs.is_empty() {
            0.0
        } else {
            pairs.iter().filter(|&&b| b).count() as f64 / pairs.len() as f64
        };
        let mut sorted = lum.clone();
        sorted.sort_by(f32::total_cmp);
        let med = sorted[len / 2];
        let off = |i: usize| (lum[i] - med).abs() > 0.1 * med;
        let ev = (1..len)
            .filter(|&i| {
                off(i)
                    && !off(i - 1)
                    && ((i + 1 < len && !off(i + 1)) || (i + 2 < len && !off(i + 2)))
            })
            .count() as f64
            / len as f64;
        for e in es {
            let wgt = f64::from(e.n) * px / len as f64;
            let t = usize::from(e.tile);
            c1[t] += s1 * wgt;
            c2[t] += s2 * wgt;
            dots[t] += ev * wgt;
        }
    }
    let per = |v: &mut Vec<f64>| {
        for (t, x) in v.iter_mut().enumerate() {
            let a = w.counts.all[t] * px / len as f64;
            *x = if a > 0.0 { *x / a } else { 0.0 };
        }
    };
    per(&mut c1);
    per(&mut c2);
    per(&mut dots);
    Maps {
        range: w.range,
        c1,
        c2,
        dots,
        counts: w.counts,
        dk0: (dk0 / nk0.max(1) as f64, nk0),
        dk2: (dk2 / nk2.max(1) as f64, nk2),
        units: scored,
    }
}

/// The clip's lit and shadowed levels (luma over exposure): the medians of the samples the rig
/// reads sunlit and shadowed, every 5th frame, every 7th sample.
fn levels(c: &Clip, ex: &[f32], lut: &[f32; 256]) -> (f32, f32) {
    let mut dec = Decoder::open(&c.mkv);
    let mut keys = std::io::BufReader::new(std::fs::File::open(&c.keys).expect("keys"));
    let mut rgb = vec![0u8; W * H * 3];
    let (mut lit, mut sh) = (Vec::new(), Vec::new());
    for (i, &e) in ex.iter().enumerate().take(c.frames) {
        if !dec.next(&mut rgb) {
            break;
        }
        let k = read_keys(&mut keys).expect("a keys frame");
        if i % 5 != 0 {
            continue;
        }
        for s in (0..NS).step_by(7) {
            let st = (k.flags[s] >> 3) & 3;
            if k.ids[s] == 0 || st > 1 {
                continue;
            }
            let (x, y) = sample_xy(s);
            let v = luma(&rgb, lut, x, y) / e;
            if st == 0 { lit.push(v) } else { sh.push(v) }
        }
    }
    let med = |v: &mut Vec<f32>| {
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(0.0)
    };
    (med(&mut lit), med(&mut sh))
}

/// Every window's maps of clip `c`, the extras after the segments'.
fn run(c: &Clip, mid: f32, ex: &[f32], lut: &[f32; 256]) -> Vec<Maps> {
    let mut ranges = windows(c);
    ranges.extend(c.extras.iter().copied());
    let mut wins: Vec<Option<Win>> = ranges
        .iter()
        .map(|&r| {
            Some(Win {
                range: r,
                units: HashMap::new(),
                counts: Counts::new(),
            })
        })
        .collect();
    let mut out: Vec<Option<Maps>> = vec![None; ranges.len()];
    let mut dec = Decoder::open(&c.mkv);
    let mut keys = std::io::BufReader::new(std::fs::File::open(&c.keys).expect("keys"));
    let mut rgb = vec![0u8; W * H * 3];
    for (i, &e) in ex.iter().enumerate().take(c.frames) {
        assert!(dec.next(&mut rgb), "{:?}: frame {i}", c.mkv);
        let k = read_keys(&mut keys).expect("a keys frame");
        let mut frame: HashMap<u32, Acc> = HashMap::with_capacity(1 << 17);
        let mut counts = Counts::new();
        for s in 0..NS {
            let (x, y) = sample_xy(s);
            let t = tile_of(x as f32, y as f32);
            counts.all[t] += 1.0;
            let fl = k.flags[s];
            let st = (fl >> 3) & 3;
            let src = usize::from(fl & 7).min(4);
            if st <= 1 {
                counts.lighting[t] += 1.0;
                counts.src[src][t] += 1.0;
                if st == 0 {
                    if fl & 32 != 0 {
                        counts.sunlit_leaf[t] += 1.0;
                    } else {
                        counts.sunlit_other[t] += 1.0;
                    }
                }
            }
            if k.ids[s] == 0 {
                continue;
            }
            let v = luma(&rgb, lut, x, y) / e;
            let a = frame.entry(k.ids[s]).or_default();
            a.lum += v;
            a.n += 1;
            a.lit += u32::from(v > mid);
            a.sx += x as f32;
            a.sy += y as f32;
            let kv = f32::from(k.vals[s]) / 255.0;
            if st <= 1 && src == 0 {
                a.s0 += kv;
                a.n0 += 1;
            } else if st <= 1 && src == 2 {
                a.s2 += kv;
                a.n2 += 1;
            }
        }
        for (wi, slot) in wins.iter_mut().enumerate() {
            let Some(w) = slot else { continue };
            if i < w.range.0 || i >= w.range.1 {
                continue;
            }
            w.counts.add(&counts, 1.0);
            for (&id, a) in &frame {
                let n = a.n as f32;
                w.units.entry(id).or_default().push(E {
                    f: i as u16,
                    lum: a.lum / n,
                    lit: a.lit as f32 / n,
                    n: a.n.min(u32::from(u16::MAX)) as u16,
                    tile: tile_of(a.sx / n, a.sy / n) as u16,
                    k0: if a.n0 > 0 { a.s0 / a.n0 as f32 } else { -1.0 },
                    k2: if a.n2 > 0 { a.s2 / a.n2 as f32 } else { -1.0 },
                });
            }
            if i + 1 == w.range.1 {
                out[wi] = Some(score(slot.take().expect("a window")));
            }
        }
    }
    out.into_iter()
        .map(|m| m.expect("every window scored"))
        .collect()
}

/// The frame-weighted sum of the windows inside `r`.
fn moment(maps: &[Maps], r: (usize, usize), nseg: usize) -> Maps {
    let mut m = Maps {
        range: r,
        c1: vec![0.0; TW * TH],
        c2: vec![0.0; TW * TH],
        dots: vec![0.0; TW * TH],
        counts: Counts::new(),
        dk0: (0.0, 0),
        dk2: (0.0, 0),
        units: 0,
    };
    let inside: Vec<&Maps> = maps[..nseg]
        .iter()
        .filter(|w| w.range.0 >= r.0 && w.range.1 <= r.1)
        .collect();
    assert!(!inside.is_empty(), "no window inside {r:?}");
    let tot: f64 = inside.iter().map(|w| (w.range.1 - w.range.0) as f64).sum();
    let (mut d0, mut d2) = (0.0, 0.0);
    for w in &inside {
        let f = (w.range.1 - w.range.0) as f64 / tot;
        for t in 0..TW * TH {
            m.c1[t] += f * w.c1[t];
            m.c2[t] += f * w.c2[t];
            m.dots[t] += f * w.dots[t];
        }
        m.counts.add(&w.counts, 1.0);
        d0 += w.dk0.0 * w.dk0.1 as f64;
        d2 += w.dk2.0 * w.dk2.1 as f64;
        m.dk0.1 += w.dk0.1;
        m.dk2.1 += w.dk2.1;
        m.units += w.units;
    }
    m.dk0.0 = d0 / m.dk0.1.max(1) as f64;
    m.dk2.0 = d2 / m.dk2.1.max(1) as f64;
    m
}

fn region_tiles(region: Region, counts: &Counts) -> Vec<bool> {
    (0..TW * TH)
        .map(|t| {
            let (cx, cy) = (
                ((t % TW) * TILE + TILE / 2) as f64,
                ((t / TW) * TILE + TILE / 2) as f64,
            );
            let all = counts.all[t].max(1.0);
            match region {
                Region::TopThird => cy < H as f64 / 3.0,
                Region::TopLeftNinth => cy < H as f64 / 3.0 && cx < W as f64 / 3.0,
                Region::SunlitLeaf => counts.sunlit_leaf[t] / all >= 0.05,
                Region::SunlitOther => counts.sunlit_other[t] / all >= 0.05,
            }
        })
        .collect()
}

/// The top tenth of the scored tiles of `v` (value over 0) by value.
fn top_tenth(v: &[f64]) -> (Vec<usize>, Vec<usize>) {
    let mut scored: Vec<usize> = (0..v.len()).filter(|&t| v[t] > 0.0).collect();
    scored.sort_by(|&a, &b| v[b].total_cmp(&v[a]));
    let n = scored.len().div_ceil(10);
    (scored[..n].to_vec(), scored)
}

/// Line (a): the region's share of the top tenth and of the scored tiles.
fn line_a(v: &[f64], region: &[bool]) -> (f64, f64, bool) {
    let (top, scored) = top_tenth(v);
    if scored.is_empty() {
        return (0.0, 0.0, false);
    }
    let share = |s: &[usize]| s.iter().filter(|&&t| region[t]).count() as f64 / s.len() as f64;
    let (st, ss) = (share(&top), share(&scored));
    (st, ss, ss > 0.0 && st >= 2.0 * ss)
}

/// A map over `rgb` at half size: the frame dimmed, tiles tinted by `v` over its largest, the
/// top tenth outlined white, the region's tiles outlined cyan.
fn map_png(path: &Path, rgb: &[u8], v: Option<&[f64]>, region: &[bool], src: Option<&KeyFrame>) {
    let (w, h) = (W / 2, H / 2);
    let mut out = vec![255u8; w * h * 4];
    let max = v.map_or(1.0, |v| v.iter().copied().fold(0.0, f64::max).max(1e-12));
    let top = v.map(|v| top_tenth(v).0).unwrap_or_default();
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (2 * x, 2 * y);
            let p = 3 * (dy * W + dx);
            let t = tile_of(dx as f32, dy as f32);
            let mut c = [0, 1, 2].map(|k| f64::from(rgb[p + k]) * 0.45);
            if let Some(v) = v {
                let a = (v[t] / max).sqrt();
                // Black, red, yellow, white by score.
                let heat = [
                    (3.0 * a).min(1.0),
                    (3.0 * a - 1.0).clamp(0.0, 1.0),
                    (3.0 * a - 2.0).clamp(0.0, 1.0),
                ];
                for k in 0..3 {
                    c[k] = c[k] * (1.0 - 0.6 * a) + 255.0 * heat[k] * 0.6 * a.max(0.0);
                }
            }
            if let Some(kf) = src {
                let s = (y.min(SH - 1)) * SW + x.min(SW - 1);
                let fl = kf.flags[s];
                let col = if (fl >> 3) & 3 == 3 {
                    None
                } else {
                    Some(match fl & 7 {
                        0 => [255.0, 60.0, 40.0],
                        1 => [255.0, 220.0, 0.0],
                        2 => [90.0, 160.0, 255.0],
                        3 => [0.0, 255.0, 120.0],
                        _ => [200.0, 0.0, 255.0],
                    })
                };
                if let Some(col) = col {
                    for k in 0..3 {
                        c[k] = c[k] * 0.5 + col[k] * 0.5;
                    }
                }
            }
            let (ex, ey) = (
                dx % TILE < 2 || dx % TILE >= TILE - 2,
                dy % TILE < 2 || dy % TILE >= TILE - 2,
            );
            let edge = ex || ey;
            // The region's outline: a region tile's side whose neighbour is outside it.
            let (tx, ty) = (t % TW, t / TW);
            let outside = |nx: isize, ny: isize| {
                nx < 0
                    || ny < 0
                    || nx >= TW as isize
                    || ny >= TH as isize
                    || !region[ny as usize * TW + nx as usize]
            };
            let (ix, iy) = (tx as isize, ty as isize);
            let rim = region[t]
                && ((dx % TILE < 4 && outside(ix - 1, iy))
                    || (dx % TILE >= TILE - 4 && outside(ix + 1, iy))
                    || (dy % TILE < 4 && outside(ix, iy - 1))
                    || (dy % TILE >= TILE - 4 && outside(ix, iy + 1)));
            if rim {
                c = [0.0, 255.0, 255.0];
            } else if edge && top.contains(&t) {
                c = [255.0; 3];
            }
            let o = 4 * (y * w + x);
            for k in 0..3 {
                out[o + k] = c[k].clamp(0.0, 255.0) as u8;
            }
        }
    }
    support::write_png(&path.to_path_buf(), &out, w as u32, h as u32);
}

/// The keys frame `n` of `c`.
fn keys_frame(c: &Clip, n: usize) -> KeyFrame {
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::open(&c.keys).expect("keys");
    f.seek(SeekFrom::Start((n * NS * 6) as u64)).expect("seek");
    read_keys(&mut f).expect("the keys frame")
}

fn measure(name: &str) {
    let c = clip_of(name);
    let t0 = std::time::Instant::now();
    let lut = lin_lut();
    let ex = exposures(&c.side, c.frames);
    let (lit, sh) = levels(&c, &ex, &lut);
    let mid = 0.5 * (lit + sh);
    println!(
        "{}: levels (luma over exposure) lit {lit:.4} shadowed {sh:.4} mid {mid:.4}",
        c.name
    );
    let maps = run(&c, mid, &ex, &lut);
    let nseg = windows(&c).len();
    println!("{}: scored in {:.0} s", c.name, t0.elapsed().as_secs_f64());
    // Totals a window and the clip's (frame-weighted, segment windows only).
    let mut clip_tot = [0.0; 3];
    for (k, w) in maps.iter().enumerate() {
        let tc = [&w.c1, &w.c2, &w.dots].map(|v| Maps::total(v, &w.counts));
        let lighting: f64 = w.counts.lighting.iter().sum::<f64>().max(1.0);
        let src: Vec<String> = (0..5)
            .map(|s| {
                format!(
                    "{:.1}",
                    100.0 * w.counts.src[s].iter().sum::<f64>() / lighting
                )
            })
            .collect();
        println!(
            "{} {} frames {}..{}: c1 {:.5} c2 {:.5} dots {:.5}; units scored {}; slot sources 0/1/2/3/4 % {}; slot change a frame src0 {:.4} ({}) src2 {:.4} ({})",
            c.name,
            if k < nseg { "window" } else { "extra" },
            w.range.0,
            w.range.1,
            tc[0],
            tc[1],
            tc[2],
            w.units,
            src.join("/"),
            w.dk0.0,
            w.dk0.1,
            w.dk2.0,
            w.dk2.1
        );
        if k < nseg {
            for j in 0..3 {
                clip_tot[j] += tc[j] * (w.range.1 - w.range.0) as f64 / c.frames as f64;
            }
        }
    }
    println!(
        "{} clip totals: c1 {:.5} c2 {:.5} dots {:.5}",
        c.name, clip_tot[0], clip_tot[1], clip_tot[2]
    );
    let m = moment(&maps, c.moment, nseg);
    let region = region_tiles(c.region, &m.counts);
    let nreg = region.iter().filter(|&&r| r).count();
    for (lbl, v) in [("c1", &m.c1), ("c2", &m.c2), ("dots", &m.dots)] {
        let (st, ss, holds) = line_a(v, &region);
        println!(
            "{} moment {:?} ({} region tiles) {lbl}: total {:.5}; region's share of the top tenth {:.3} against of the scored tiles {:.3}: line (a) {}",
            c.name,
            c.moment,
            nreg,
            Maps::total(v, &m.counts),
            st,
            ss,
            if holds { "holds" } else { "fails" }
        );
    }
    // Candidate 7 (2): source 0's share of the lighting samples in the region against the picture's.
    let sum = |v: &[f64], sel: &dyn Fn(usize) -> bool| {
        (0..TW * TH).filter(|&t| sel(t)).map(|t| v[t]).sum::<f64>()
    };
    let in_r = |t: usize| region[t];
    let any = |_t: usize| true;
    let r_share = sum(&m.counts.src[0], &in_r) / sum(&m.counts.lighting, &in_r).max(1.0);
    let p_share = sum(&m.counts.src[0], &any) / sum(&m.counts.lighting, &any).max(1.0);
    println!(
        "{} c7 (2): source 0 in the region {:.4} of its lighting samples, in the picture {:.4}: ratio {:.2}",
        c.name,
        r_share,
        p_share,
        if p_share > 0.0 {
            r_share / p_share
        } else {
            f64::NAN
        }
    );
    println!(
        "{} c7 (3) moment: slot change a frame on units, src0 {:.4} ({} unit-frames), src2 {:.4} ({})",
        c.name, m.dk0.0, m.dk0.1, m.dk2.0, m.dk2.1
    );
    let (top1, _) = top_tenth(&m.c1);
    let top_set = |t: usize| top1.contains(&t);
    println!(
        "{} c7 (4): of candidate 1's top tenth ({} tiles), source 0 holds {:.4} of their lighting samples",
        c.name,
        top1.len(),
        sum(&m.counts.src[0], &top_set) / sum(&m.counts.lighting, &top_set).max(1.0)
    );
    // Maps over the frame he looked at (or the moment's middle).
    let rgb = frame_of(&c.mkv, c.shot);
    let kf = keys_frame(&c, c.shot);
    let dir = live().join("p4");
    for (lbl, v) in [("c1", &m.c1), ("c2", &m.c2), ("dots", &m.dots)] {
        map_png(
            &dir.join(format!("map-{}-{lbl}.png", c.name)),
            &rgb,
            Some(v),
            &region,
            None,
        );
    }
    map_png(
        &dir.join(format!("map-{}-source.png", c.name)),
        &rgb,
        None,
        &region,
        Some(&kf),
    );
    // `S`'s window after the turn (398 to 440), over his screenshot's frame (413): a reading.
    if let Some(w) = maps[nseg..].iter().find(|w| w.range == (398, 441)) {
        let rgb = frame_of(&c.mkv, 413);
        let kf = keys_frame(&c, 413);
        let none = vec![false; TW * TH];
        for (lbl, v) in [("c1", &w.c1), ("c2", &w.c2), ("dots", &w.dots)] {
            map_png(
                &dir.join(format!("map-{}-398-440-{lbl}.png", c.name)),
                &rgb,
                Some(v),
                &none,
                None,
            );
        }
        map_png(
            &dir.join(format!("map-{}-398-440-source.png", c.name)),
            &rgb,
            None,
            &none,
            Some(&kf),
        );
    }
    // The rendered-again frame against his at the same index.
    let again = frame_of(&dir.join(format!("{}-k.mkv", c.name)), c.shot);
    let d: f64 = rgb
        .iter()
        .zip(&again)
        .map(|(a, b)| f64::from(a.abs_diff(*b)))
        .sum::<f64>()
        / rgb.len() as f64;
    println!(
        "{} frame {}: his clip against the render again, mean abs difference {d:.3} levels",
        c.name, c.shot
    );
    println!(
        "{}: artifact {}; done in {:.0} s",
        c.name,
        c.artifact,
        t0.elapsed().as_secs_f64()
    );
}

/// `S`'s control: frames 398 and 440 differenced, pinned and not; the difference on the
/// outlines (pixels whose grey gradient at the first frame is over 24 levels) and elsewhere.
/// Beside it the hold's late pair 500 and 542 (42 frames apart too), the change that remains
/// once the hold has settled.
fn control() {
    let p4 = live().join("p4");
    let a = live().join("anchor");
    let grey = |rgb: &[u8]| -> Vec<f32> {
        rgb.chunks(3)
            .map(|p| 0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2]))
            .collect()
    };
    for ((lbl, mkv), (f0, f1)) in [
        ("his clip", a.join("S.mkv")),
        ("rendered again", p4.join("S-k.mkv")),
        ("pinned at 397", p4.join("S-pin.mkv")),
    ]
    .into_iter()
    .flat_map(|c| [(c.clone(), (398, 440)), (c, (500, 542))])
    {
        let g0 = grey(&frame_of(&mkv, f0));
        let g1 = grey(&frame_of(&mkv, f1));
        let (mut eo, mut no, mut ef, mut nf, mut big_o, mut big_f) = (0.0, 0, 0.0, 0, 0, 0);
        let mut png = vec![255u8; (W / 2) * (H / 2) * 4];
        for y in 1..H - 1 {
            for x in 1..W - 1 {
                let i = y * W + x;
                let gx = g0[i + 1] - g0[i - 1];
                let gy = g0[i + W] - g0[i - W];
                let d = (g1[i] - g0[i]).abs();
                if (gx * gx + gy * gy).sqrt() > 24.0 {
                    eo += f64::from(d);
                    no += 1;
                    big_o += usize::from(d > 8.0);
                } else {
                    ef += f64::from(d);
                    nf += 1;
                    big_f += usize::from(d > 8.0);
                }
                if x % 2 == 0 && y % 2 == 0 && y / 2 < H / 2 && x / 2 < W / 2 {
                    let o = 4 * ((y / 2) * (W / 2) + x / 2);
                    let v = (d * 16.0).min(255.0) as u8;
                    png[o..o + 3].copy_from_slice(&[v, v, v]);
                }
            }
        }
        let stem = lbl.replace(' ', "-");
        support::write_png(
            &p4.join(format!("control-{stem}-{f0}-{f1}-x16.png")),
            &png,
            (W / 2) as u32,
            (H / 2) as u32,
        );
        println!(
            "control {lbl}: {f0} against {f1}, outline pixels {no}: mean {:.3} levels, over 8 levels {big_o}; elsewhere {nf}: mean {:.3}, over 8 levels {big_f}",
            eo / no.max(1) as f64,
            ef / nf.max(1) as f64
        );
    }
}

mod reading {
    use super::*;

    /// `SVET_MEASURE` (module doc).
    #[test]
    #[ignore = "reading"]
    fn the_measures() {
        let var = std::env::var("SVET_MEASURE").unwrap_or_else(|_| "T-a".into());
        for name in var.split(',') {
            if name == "control" {
                control();
            } else {
                measure(name);
            }
        }
    }
}
