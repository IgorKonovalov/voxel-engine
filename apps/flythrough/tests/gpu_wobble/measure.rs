//! The stepping edge's readings (Plan 0104): candidate 2 and candidate 1 with its dots, scored
//! on the CPU (no device) from a clip's decoded frames (`ffmpeg` to raw RGB), its side file's
//! exposure and the `.keys` of its track rendered again (`keys.rs`). Candidate 2 on held units
//! is a reading, not a measure: it ordered one pair the owner ranked. Nothing is asserted.
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
//! - **Held units:** a hold window (the camera still) is scored with each sample's unit taken
//!   from the window's first frame for the whole window (the luma each frame's own), so a
//!   sample whose jittered internal hit crosses a unit's border stays in its unit. A moving
//!   window's units are read a frame.
//! - Beside every window, the picture's own change: the mean absolute grey difference of
//!   consecutive frames and the share of pixels over 8 levels; beside candidate 1, its mean
//!   over the units whose samples are mostly sunlit leaf over the window (the sunlit-leaf row).
//!
//! `SVET_MEASURE=<clip>[,<clip>..]` (`T-a`, `T-b`, `C-a`, `C-b`, `S`), or `control` for `S`'s
//! exposure control (frames 398 and 440 differenced on the outlines and elsewhere). With both
//! `T` clips, line (b) at T and the two `T` moment maps of candidate 2 drawn on one shared
//! scale. Reads `live/anchor/<clip>.mkv` and `.txt` and `live/p4/<clip>-k.keys` and
//! `-k.mkv` (`SVET_CLIP=<subject>:<arm>:<clip>-k` with the `keys` option renders both again);
//! output under `live/p4/`, a map file that exists not drawn again.

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
    Some(KeyFrame { ids, flags })
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
    /// Samples sunlit leaf.
    leaf: u32,
}

/// A unit's entry in one window's frame.
#[derive(Clone, Copy)]
struct E {
    f: u16,
    lum: f32,
    lit: f32,
    n: u16,
    leaf: u16,
    tile: u16,
}

/// The per-tile sample counts of one window.
#[derive(Clone)]
struct Counts {
    all: Vec<f64>,
    sunlit_leaf: Vec<f64>,
    sunlit_other: Vec<f64>,
}

impl Counts {
    fn new() -> Self {
        let z = vec![0.0; TW * TH];
        Self {
            all: z.clone(),
            sunlit_leaf: z.clone(),
            sunlit_other: z,
        }
    }
    fn add(&mut self, o: &Counts, w: f64) {
        let add =
            |a: &mut Vec<f64>, b: &Vec<f64>| a.iter_mut().zip(b).for_each(|(x, y)| *x += w * y);
        add(&mut self.all, &o.all);
        add(&mut self.sunlit_leaf, &o.sunlit_leaf);
        add(&mut self.sunlit_other, &o.sunlit_other);
    }
}

/// One window's accumulation and its result.
struct Win {
    range: (usize, usize),
    units: HashMap<u32, Vec<E>>,
    counts: Counts,
    /// Held units: each sample's unit from the window's first frame (set at that frame).
    held: bool,
    ids: Option<Vec<u32>>,
    /// The picture's change: the sums of consecutive frames' mean absolute grey difference and
    /// share of pixels over 8 levels, and the pairs read.
    diff: [f64; 3],
}

/// A window's scored maps (a tile's value over its own samples) and totals.
#[derive(Clone)]
struct Maps {
    range: (usize, usize),
    c1: Vec<f64>,
    c2: Vec<f64>,
    dots: Vec<f64>,
    counts: Counts,
    units: usize,
    held: bool,
    diff: [f64; 3],
    /// Candidate 1 over the units mostly sunlit leaf: the sum of score times pixel-frames, and
    /// the pixel-frames.
    leaf: (f64, f64),
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
    let px = f64::from(STRIDE * STRIDE);
    let mut scored = 0;
    let mut leaf = (0.0, 0.0);
    for es in w.units.values() {
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
        let (nl, na) = es.iter().fold((0u32, 0u32), |(l, a), e| {
            (l + u32::from(e.leaf), a + u32::from(e.n))
        });
        for e in es {
            let wgt = f64::from(e.n) * px / len as f64;
            let t = usize::from(e.tile);
            c1[t] += s1 * wgt;
            c2[t] += s2 * wgt;
            dots[t] += ev * wgt;
            if 2 * nl > na {
                leaf.0 += s1 * wgt;
                leaf.1 += wgt;
            }
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
        units: scored,
        held: w.held,
        diff: w.diff,
        leaf,
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

/// One frame's units read with `ids`: each unit's sums over its samples.
fn accumulate(
    ids: &[u32],
    k: &KeyFrame,
    rgb: &[u8],
    lut: &[f32; 256],
    e: f32,
    mid: f32,
) -> HashMap<u32, Acc> {
    let mut frame: HashMap<u32, Acc> = HashMap::with_capacity(1 << 17);
    for (s, &id) in ids.iter().enumerate() {
        if id == 0 {
            continue;
        }
        let (x, y) = sample_xy(s);
        let fl = k.flags[s];
        let st = (fl >> 3) & 3;
        let v = luma(rgb, lut, x, y) / e;
        let a = frame.entry(id).or_default();
        a.lum += v;
        a.n += 1;
        a.lit += u32::from(v > mid);
        a.leaf += u32::from(st == 0 && fl & 32 != 0);
        a.sx += x as f32;
        a.sy += y as f32;
    }
    frame
}

/// The grey levels of `rgb` (Rec. 601 weights, as `control`).
fn grey(rgb: &[u8]) -> Vec<f32> {
    rgb.chunks(3)
        .map(|p| 0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2]))
        .collect()
}

/// Whether window `r` of `c` lies inside a hold (the camera still).
fn is_hold(c: &Clip, r: (usize, usize)) -> bool {
    let h1 = (c.fly, c.fly + HOLD);
    let h2 = (c.fly + HOLD + TURN, c.frames);
    [h1, h2].iter().any(|h| r.0 >= h.0 && r.1 <= h.1)
}

/// Every window's maps of clip `c`, the extras after the segments'; a hold window scored with
/// held units.
fn run(c: &Clip, mid: f32, ex: &[f32], lut: &[f32; 256]) -> Vec<Maps> {
    let mut ranges = windows(c);
    ranges.extend(c.extras.iter().copied());
    let specs: Vec<((usize, usize), bool)> = ranges.iter().map(|&r| (r, is_hold(c, r))).collect();
    let mut wins: Vec<Option<Win>> = specs
        .iter()
        .map(|&(r, held)| {
            Some(Win {
                range: r,
                units: HashMap::new(),
                counts: Counts::new(),
                held,
                ids: None,
                diff: [0.0; 3],
            })
        })
        .collect();
    let mut out: Vec<Option<Maps>> = vec![None; specs.len()];
    let mut dec = Decoder::open(&c.mkv);
    let mut keys = std::io::BufReader::new(std::fs::File::open(&c.keys).expect("keys"));
    let mut rgb = vec![0u8; W * H * 3];
    let mut prev: Option<Vec<f32>> = None;
    for (i, &e) in ex.iter().enumerate().take(c.frames) {
        assert!(dec.next(&mut rgb), "{:?}: frame {i}", c.mkv);
        let k = read_keys(&mut keys).expect("a keys frame");
        let g = grey(&rgb);
        let d = prev.as_ref().map(|p| {
            let (mut sum, mut big) = (0.0f64, 0usize);
            for (a, b) in p.iter().zip(&g) {
                let x = (a - b).abs();
                sum += f64::from(x);
                big += usize::from(x > 8.0);
            }
            (sum / g.len() as f64, big as f64 / g.len() as f64)
        });
        prev = Some(g);
        let mut counts = Counts::new();
        for s in 0..NS {
            let (x, y) = sample_xy(s);
            let t = tile_of(x as f32, y as f32);
            counts.all[t] += 1.0;
            let fl = k.flags[s];
            if (fl >> 3) & 3 == 0 {
                if fl & 32 != 0 {
                    counts.sunlit_leaf[t] += 1.0;
                } else {
                    counts.sunlit_other[t] += 1.0;
                }
            }
        }
        let frame = accumulate(&k.ids, &k, &rgb, lut, e, mid);
        // Held units' frames, one a distinct window start.
        let mut held: HashMap<usize, HashMap<u32, Acc>> = HashMap::new();
        for (wi, slot) in wins.iter_mut().enumerate() {
            let Some(w) = slot else { continue };
            if i < w.range.0 || i >= w.range.1 {
                continue;
            }
            w.counts.add(&counts, 1.0);
            if i > w.range.0
                && let Some((m, b)) = d
            {
                w.diff[0] += m;
                w.diff[1] += b;
                w.diff[2] += 1.0;
            }
            let accs = if w.held {
                if i == w.range.0 {
                    w.ids = Some(k.ids.clone());
                }
                let ids = w.ids.as_ref().expect("the first frame's units");
                &*held
                    .entry(w.range.0)
                    .or_insert_with(|| accumulate(ids, &k, &rgb, lut, e, mid))
            } else {
                &frame
            };
            for (&id, a) in accs {
                let n = a.n as f32;
                w.units.entry(id).or_default().push(E {
                    f: i as u16,
                    lum: a.lum / n,
                    lit: a.lit as f32 / n,
                    n: a.n.min(u32::from(u16::MAX)) as u16,
                    leaf: a.leaf.min(u32::from(u16::MAX)) as u16,
                    tile: tile_of(a.sx / n, a.sy / n) as u16,
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
        units: 0,
        held: false,
        diff: [0.0; 3],
        leaf: (0.0, 0.0),
    };
    let inside: Vec<&Maps> = maps[..nseg]
        .iter()
        .filter(|w| w.range.0 >= r.0 && w.range.1 <= r.1)
        .collect();
    assert!(!inside.is_empty(), "no window inside {r:?}");
    let tot: f64 = inside.iter().map(|w| (w.range.1 - w.range.0) as f64).sum();
    for w in &inside {
        let f = (w.range.1 - w.range.0) as f64 / tot;
        for t in 0..TW * TH {
            m.c1[t] += f * w.c1[t];
            m.c2[t] += f * w.c2[t];
            m.dots[t] += f * w.dots[t];
        }
        m.counts.add(&w.counts, 1.0);
        m.units += w.units;
    }
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

/// A map over `rgb` at half size: the frame dimmed, tiles tinted by `v` over `scale` (else its
/// largest), the top tenth outlined white, the region's tiles outlined cyan. Not drawn where
/// `path` exists.
fn map_png(path: &Path, rgb: &[u8], v: Option<&[f64]>, region: &[bool], scale: Option<f64>) {
    if path.exists() {
        return;
    }
    let (w, h) = (W / 2, H / 2);
    let mut out = vec![255u8; w * h * 4];
    let max = scale
        .unwrap_or_else(|| v.map_or(1.0, |v| v.iter().copied().fold(0.0, f64::max).max(1e-12)));
    let top = v.map(|v| top_tenth(v).0).unwrap_or_default();
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (2 * x, 2 * y);
            let p = 3 * (dy * W + dx);
            let t = tile_of(dx as f32, dy as f32);
            let mut c = [0, 1, 2].map(|k| f64::from(rgb[p + k]) * 0.45);
            if let Some(v) = v {
                let a = (v[t] / max).min(1.0).sqrt();
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

/// What [`measure`] hands back for the pair's readings.
struct Measured {
    /// Candidate 2's clip total.
    c2: f64,
    moment_c2: Vec<f64>,
    region: Vec<bool>,
    shot: usize,
    mkv: PathBuf,
}

fn measure(name: &str) -> Measured {
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
    // Totals a window and the clip's (frame-weighted, segment windows only); a hold window's
    // units held from its first frame.
    let mut clip_tot = [0.0; 3];
    let mut leaf_tot = 0.0;
    let pct = |x: f64| 100.0 * x;
    // A window's display pixels a frame (the scale of `Maps::leaf`'s pixel-frames).
    let area = |w: &Maps| {
        w.counts.all.iter().sum::<f64>() * f64::from(STRIDE * STRIDE)
            / (w.range.1 - w.range.0) as f64
    };
    for (k, w) in maps.iter().enumerate() {
        let tc = [&w.c1, &w.c2, &w.dots].map(|v| Maps::total(v, &w.counts));
        let pd = w.diff[2].max(1.0);
        println!(
            "{} {} {} frames {}..{}: c1 {:.5} c2 {:.5} dots {:.5}; units scored {}; picture a frame {:.4} levels, {:.4} % over 8; c1 on sunlit-leaf units {:.5} ({:.1} % of the scored pixel-frames)",
            c.name,
            if k < nseg { "window" } else { "extra" },
            if w.held { "hold, held units" } else { "moving" },
            w.range.0,
            w.range.1,
            tc[0],
            tc[1],
            tc[2],
            w.units,
            w.diff[0] / pd,
            pct(w.diff[1] / pd),
            w.leaf.0 / w.leaf.1.max(1e-12),
            pct(w.leaf.1 / area(w)),
        );
        if k < nseg {
            let f = (w.range.1 - w.range.0) as f64 / c.frames as f64;
            for j in 0..3 {
                clip_tot[j] += tc[j] * f;
            }
            leaf_tot += f * w.leaf.0 / w.leaf.1.max(1e-12);
        }
    }
    println!(
        "{} clip totals, held units in the holds: c1 {:.5} c2 {:.5} dots {:.5}",
        c.name, clip_tot[0], clip_tot[1], clip_tot[2]
    );
    println!(
        "{} c1 on sunlit-leaf units, clip, held units in the holds: {:.5}",
        c.name, leaf_tot
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
    // Maps over the frame he looked at (or the moment's middle).
    let rgb = frame_of(&c.mkv, c.shot);
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
    // `S`'s window after the turn (398 to 440), over his screenshot's frame (413): a reading.
    if let Some(w) = maps[nseg..].iter().find(|w| w.range == (398, 441)) {
        let rgb = frame_of(&c.mkv, 413);
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
    Measured {
        c2: clip_tot[1],
        moment_c2: m.c2.clone(),
        region,
        shot: c.shot,
        mkv: c.mkv.clone(),
    }
}

/// Line (b) at T on candidate 2 (`T a` over `T b`), held units in the holds, and the two
/// moment maps of candidate 2 on one shared scale (the larger of the two maxima).
fn pair_t(a: &Measured, b: &Measured) {
    println!(
        "T line (b) c2, held units in the holds: T-a {:.5} against T-b {:.5}: {}",
        a.c2,
        b.c2,
        if a.c2 > b.c2 {
            "his order (holds)"
        } else {
            "the other way (fails)"
        }
    );
    let max = |v: &[f64]| v.iter().copied().fold(0.0, f64::max);
    let scale = max(&a.moment_c2).max(max(&b.moment_c2)).max(1e-12);
    println!(
        "T moment c2 maxima: T-a {:.5}, T-b {:.5}; shared scale {scale:.5}",
        max(&a.moment_c2),
        max(&b.moment_c2)
    );
    let dir = live().join("p4");
    for (m, stem) in [(a, "T-a"), (b, "T-b")] {
        map_png(
            &dir.join(format!("map-{stem}-c2-shared.png")),
            &frame_of(&m.mkv, m.shot),
            Some(&m.moment_c2),
            &m.region,
            Some(scale),
        );
    }
}

/// `S`'s control: frames 398 and 440 differenced, pinned and not; the difference on the
/// outlines (pixels whose grey gradient at the first frame is over 24 levels) and elsewhere.
/// Beside it the hold's late pair 500 and 542 (42 frames apart too), the change that remains
/// once the hold has settled.
fn control() {
    let p4 = live().join("p4");
    let a = live().join("anchor");
    // Phase 5b's render (no upscaler at the default arm's internal size, pinned at 397) where
    // it has been made.
    let native = p4.join("S-native-pin.mkv");
    let mut files = vec![
        ("his clip", a.join("S.mkv")),
        ("rendered again", p4.join("S-k.mkv")),
        ("pinned at 397", p4.join("S-pin.mkv")),
    ];
    if native.exists() {
        files.push(("no upscaler pinned at 397", native));
    }
    for ((lbl, mkv), (f0, f1)) in files
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
        let out = p4.join(format!("control-{stem}-{f0}-{f1}-x16.png"));
        if !out.exists() {
            support::write_png(&out, &png, (W / 2) as u32, (H / 2) as u32);
        }
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
        let mut got: HashMap<String, Measured> = HashMap::new();
        for name in var.split(',') {
            if name == "control" {
                control();
            } else {
                got.insert(name.to_owned(), measure(name));
            }
        }
        if let (Some(a), Some(b)) = (got.get("T-a"), got.get("T-b")) {
            pair_t(a, b);
        }
    }
}
