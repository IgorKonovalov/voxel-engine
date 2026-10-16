//! Packs a SOFA HRIR set into the engine's pack format (ADR-0006).
//!
//! `cargo run -p vxe-audio --example hrir_pack -- [--azimuth-stride N] <SOFA path> <pack path>`
//!
//! Reads the SOFA file at 48 kHz without normalisation. Directions are grouped into rings of
//! equal elevation; within a ring, sorted by azimuth, only the indices divisible by the stride
//! (default 3) are kept, so a ring of one direction (a pole) always survives. Per kept
//! direction, the packer keeps 128
//! taps per ear starting [`PRE_ONSET`] samples before the earlier ear's onset (the first sample
//! whose magnitude reaches a tenth of that ear's peak). The shipped pack is SADIE II subject D2
//! from `D2_HRIR_SOFA.zip`, file `D2_48K_24bit_256tap_FIR_SOFA.sofa`; see `assets/README.md`.
//!
//! Pack format, little-endian: magic `b"VXEH"`, `u32` version 1, `u32` rate, `u32` taps,
//! `u32` direction count; per direction `f32` azimuth and elevation in degrees (azimuth
//! anticlockwise from the front, toward the left; elevation up); then per direction the left
//! ear's taps followed by the right ear's, as `f32`.

use sofar::reader::OpenOptions;

const RATE: u32 = 48_000;
const TAPS: usize = 128;
/// Samples kept before the earlier ear's onset.
const PRE_ONSET: usize = 8;
/// An ear's onset is its first sample at or above this fraction of its peak magnitude.
const ONSET_FRACTION: f32 = 0.1;
/// Default ring azimuth stride: SADIE II D2's 0.9 degree rings become 2.7 degrees.
const DEFAULT_STRIDE: usize = 3;

fn onset(ir: &[f32]) -> usize {
    let peak = ir.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    ir.iter()
        .position(|s| s.abs() >= ONSET_FRACTION * peak)
        .unwrap_or(0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut stride = DEFAULT_STRIDE;
    if let Some(i) = args.iter().position(|a| a == "--azimuth-stride") {
        let v = args.get(i + 1).ok_or("--azimuth-stride needs a value")?;
        stride = v.parse()?;
        if stride == 0 {
            return Err("--azimuth-stride must be at least 1".into());
        }
        args.drain(i..i + 2);
    }
    let [sofa_path, out_path] = args.as_slice() else {
        return Err("usage: hrir_pack [--azimuth-stride N] <SOFA path> <pack path>".into());
    };
    let sofa = OpenOptions::new()
        .sample_rate(RATE as f32)
        .normalized(false)
        .open(sofa_path)?;
    let hrtf = sofa.hrtf();
    if (hrtf.sample_rate() - RATE as f32).abs() > 0.1 {
        return Err(format!("the file is at {} Hz, not {RATE}", hrtf.sample_rate()).into());
    }
    let (m, r, n) = (hrtf.m() as usize, hrtf.r() as usize, hrtf.n() as usize);
    if r != 2 {
        return Err(format!("{r} receivers, not 2").into());
    }
    if hrtf.data_delay.values.iter().any(|&d| d != 0.0) {
        return Err(
            "the file carries per-filter delays; the packer expects them in the IRs".into(),
        );
    }
    let pos = &hrtf.source_position.values;
    let ir = &hrtf.data_ir.values;
    if pos.len() != 3 * m || ir.len() != m * r * n {
        return Err("unexpected array sizes".into());
    }

    // Cartesian after the reader's conversion: x front, y left, z up.
    let angles: Vec<[f32; 2]> = (0..m)
        .map(|i| {
            let [x, y, z] = [pos[3 * i], pos[3 * i + 1], pos[3 * i + 2]];
            [y.atan2(x).to_degrees(), z.atan2(x.hypot(y)).to_degrees()]
        })
        .collect();
    // Rings keyed by elevation in tenths of a degree; members sorted by azimuth in [0, 360).
    let mut rings: std::collections::BTreeMap<i32, Vec<usize>> = Default::default();
    for (i, [_, e]) in angles.iter().enumerate() {
        rings.entry((e * 10.0).round() as i32).or_default().push(i);
    }
    let mut keep = vec![false; m];
    for members in rings.values_mut() {
        members.sort_by(|&a, &b| {
            angles[a][0]
                .rem_euclid(360.0)
                .total_cmp(&angles[b][0].rem_euclid(360.0))
        });
        for (k, &i) in members.iter().enumerate() {
            keep[i] = k % stride == 0;
        }
    }
    let kept = keep.iter().filter(|&&k| k).count();

    let mut dirs = Vec::with_capacity(kept);
    let mut taps = Vec::with_capacity(kept * 2 * TAPS);
    let mut lead = Vec::with_capacity(kept);
    for i in (0..m).filter(|&i| keep[i]) {
        dirs.push(angles[i]);
        let left = &ir[i * 2 * n..i * 2 * n + n];
        let right = &ir[i * 2 * n + n..i * 2 * n + 2 * n];
        let start = onset(left).min(onset(right)).saturating_sub(PRE_ONSET);
        lead.push(start);
        for ear in [left, right] {
            taps.extend((start..start + TAPS).map(|k| ear.get(k).copied().unwrap_or(0.0)));
        }
    }

    let d = dirs.len();
    let mut out = Vec::with_capacity(20 + d * 8 + taps.len() * 4);
    out.extend_from_slice(b"VXEH");
    for v in [1, RATE, TAPS as u32, d as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for [a, e] in &dirs {
        out.extend_from_slice(&a.to_le_bytes());
        out.extend_from_slice(&e.to_le_bytes());
    }
    for t in &taps {
        out.extend_from_slice(&t.to_le_bytes());
    }
    std::fs::write(out_path, &out)?;
    assert_eq!(out.len(), 20 + d * 8 + d * 2 * TAPS * 4);
    println!(
        "{m} directions in {} rings, {n} taps in the file; azimuth stride {stride} keeps {d}; {TAPS} taps from samples {}..={} onward; wrote {} bytes to {out_path}",
        rings.len(),
        lead.iter().min().unwrap_or(&0),
        lead.iter().max().unwrap_or(&0),
        out.len()
    );
    Ok(())
}
