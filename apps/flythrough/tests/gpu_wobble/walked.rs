//! Plan 0100 Phase 5b step 1, on Phase 5's CPU pass: the port's mismatches placed; the
//! march's row with its breaks, its ramp's faces and the new dark; the texels a face reads
//! under the march; the record's rows. Nothing here renders; [`read`] returns its lines.

use super::*;

/// The GPU's key value against the CPU horizon at a lighting pixel's hit, off by over 2/255.
pub(super) struct Mismatch {
    pub p: [f32; 3],
    pub nf: [f32; 3],
    pub gpu: f32,
    pub cpu: f32,
    /// The key slot's source (`support::defects::key_slot`: 0 the mean, 1 a record, 2 the
    /// horizon, 3 a landing's fade, 4 none).
    pub src: usize,
}

/// The column map's cell edge, in voxels (`support::cover`'s `COLUMN_CELL`).
const MAP_CELL: f32 = 2.0;

/// The map cell that decides the horizon at `p` on a face of normal `n` toward the key light
/// `l`, as a distance bin: the point lifted by `SEED_LIFT` along the face's normal, "own cell"
/// where its own column tops it, else the cell of the steepest elevation over the lifted point
/// walked in half-cell steps along the sun's azimuth to 512 voxels, binned by its distance;
/// "none above" where no cell rises over the point.
fn decider(cols: &Columns, p: [f32; 3], n: [f32; 3], l: [f32; 3]) -> &'static str {
    let q = [0, 1, 2].map(|k| p[k] + n[k] * support::cover::SEED_LIFT);
    let cell = |x: f32, y: f32| [(x / MAP_CELL).floor() as i32, (y / MAP_CELL).floor() as i32];
    let own = cols.cell_height(cell(q[0], q[1]));
    if own < 1.0e38 && own > q[2] {
        return "own cell";
    }
    let flat = (l[0] * l[0] + l[1] * l[1]).sqrt().max(1.0e-6);
    let u = [l[0] / flat, l[1] / flat];
    let (mut best, mut at) = (0.0f32, None);
    let mut t = MAP_CELL * 0.5;
    while t <= 512.0 {
        let h = cols.cell_height(cell(q[0] + u[0] * t, q[1] + u[1] * t));
        if h > 1.0e38 {
            break;
        }
        let e = (h - q[2]) / t;
        if e > best {
            (best, at) = (e, Some(t));
        }
        t += MAP_CELL * 0.5;
    }
    match at {
        None => "none above",
        Some(t) if t < 8.0 => "t 0..8",
        Some(t) if t < 32.0 => "t 8..32",
        Some(t) if t < 128.0 => "t 32..128",
        Some(_) => "t 128..512",
    }
}

pub(super) struct Input<'a> {
    pub faces: &'a [([i32; 3], u8)],
    pub rows: &'a [FaceRow],
    pub px_face: &'a [Option<usize>],
    pub h_hit: &'a [f32],
    pub blend: &'a [Option<f32>],
    pub cols: &'a Columns,
    pub l: [f32; 3],
    pub radius: f32,
    pub eye: [f32; 3],
    pub coarse: &'a std::collections::HashSet<[i32; 3]>,
    pub mismatches: &'a [Mismatch],
}

/// The texels the march reads at each face's centre.
fn reads(i: &Input<'_>) -> Vec<u32> {
    let row = |&(v, face): &([i32; 3], u8)| {
        let nf = face_normal(face);
        let c = [0, 1, 2].map(|k| v[k] as f32 + 0.5 + 0.5 * nf[k]);
        i.cols.horizon_counted(c, nf, i.l, i.radius).1
    };
    let per = i.faces.len().div_ceil(6).max(1);
    std::thread::scope(|s| {
        let hs: Vec<_> = i
            .faces
            .chunks(per)
            .map(|part| s.spawn(move || part.iter().map(row).collect::<Vec<_>>()))
            .collect();
        hs.into_iter()
            .flat_map(|h| h.join().expect("a face thread"))
            .collect()
    })
}

/// The lines.
pub(super) fn read(i: &Input<'_>) -> String {
    let mut out = String::from("Phase 5b\n");
    out.push_str(&format!("  the map {} texels\n", i.cols.map.len()));
    let reads = reads(i);
    let mut px_of = vec![0usize; i.rows.len()];
    for f in i.px_face.iter().flatten() {
        px_of[*f] += 1;
    }
    let lit_px: usize = px_of.iter().sum();

    out.push_str(&format!(
        "  march ({} key-lit faces, {lit_px} px): breaks (march 1, reference under 0.5, centre ray blocked) faces / px / share of key-lit px; ramp faces; new dark (march under 0.5, exact 1, reference 0.5 or more) faces / px\n",
        i.rows.len()
    ));
    let mut breaks_march = std::collections::HashSet::new();
    {
        let (mut bf, mut bp, mut ramp, mut df, mut dp) = (0, 0, 0, 0, 0);
        for (k, r) in i.rows.iter().enumerate() {
            let v = r.horizon;
            if v >= 1.0 && r.reference < 0.5 && r.disc <= 0.0 {
                bf += 1;
                bp += px_of[k];
                breaks_march.insert(k);
            }
            ramp += usize::from(v > 0.0 && v < 1.0);
            if v < 0.5 && r.exact >= 1.0 && r.reference >= 0.5 {
                df += 1;
                dp += px_of[k];
            }
        }
        out.push_str(&format!(
            "    march              breaks {bf} / {bp} / {:.2} %; ramp {ramp}; new dark {df} / {dp}\n",
            100.0 * bp as f64 / lit_px.max(1) as f64
        ));
    }

    // The texels the march reads at a face, lit (exact 1) and shadowed apart.
    out.push_str("  texels read a face (mean / p99 / largest):\n");
    {
        let mut line = format!("    {:<20}", "march");
        for (cl, lit) in [("lit", true), ("shadowed", false)] {
            let mut v: Vec<f32> = i
                .rows
                .iter()
                .zip(&reads)
                .filter(|(r, _)| (r.exact >= 1.0) == lit)
                .map(|(_, n)| *n as f32)
                .collect();
            let mean = v.iter().map(|x| f64::from(*x)).sum::<f64>() / v.len().max(1) as f64;
            line.push_str(&format!(
                " {cl} {mean:.1} / {} / {} ({} faces);",
                quantile(&mut v, 0.99),
                quantile(&mut v, 1.0),
                v.len()
            ));
        }
        out.push_str(&line);
        out.push('\n');
    }

    // The port's mismatches, placed.
    let index: std::collections::HashMap<([i32; 3], u8), usize> =
        i.faces.iter().enumerate().map(|(k, f)| (*f, k)).collect();
    let mut by = std::collections::BTreeMap::<String, usize>::new();
    for m in i.mismatches {
        let a = (0..3).find(|&k| m.nf[k] != 0.0).unwrap_or(0);
        let mut v = m.p.map(|x| x.floor() as i32);
        v[a] = m.p[a].round() as i32 - i32::from(m.nf[a] > 0.0);
        let face = (2 * a) as u8 + u8::from(m.nf[a] > 0.0);
        let d = ((0..3).map(|k| (m.p[k] - i.eye[k]).powi(2)).sum::<f32>()).sqrt();
        let dist = match d {
            x if x < 32.0 => "t 0..32",
            x if x < 128.0 => "t 32..128",
            x if x < 512.0 => "t 128..512",
            _ => "t 512 on",
        };
        let class = |x: f32| {
            if x >= 1.0 {
                "1"
            } else if x <= 0.0 {
                "0"
            } else {
                "ramp"
            }
        };
        let face_at = index.get(&(v, face));
        for key in [
            format!("by distance {dist}"),
            format!("cpu {} gpu {}", class(m.cpu), class(m.gpu)),
            format!(
                "on a {} block",
                if i.coarse.contains(&v.map(|x| x >> 6)) {
                    "coarse"
                } else {
                    "full"
                }
            ),
            format!(
                "face {}",
                match face_at {
                    None => "not among the key-lit faces",
                    Some(k) if breaks_march.contains(k) => "in march's breaks",
                    Some(_) => "outside the breaks",
                }
            ),
            format!("gpu {} cpu", if m.gpu > m.cpu { "over" } else { "under" }),
            format!(
                "slot {}",
                ["mean", "record", "horizon", "fade", "none"][m.src.min(4)]
            ),
            format!("decider {}", decider(i.cols, m.p, m.nf, i.l)),
            format!(
                "slot {} on a {} block",
                ["mean", "record", "horizon", "fade", "none"][m.src.min(4)],
                if i.coarse.contains(&v.map(|x| x >> 6)) {
                    "coarse"
                } else {
                    "full"
                }
            ),
        ] {
            *by.entry(key).or_default() += 1;
        }
    }
    out.push_str(&format!(
        "  the port's mismatches ({} lighting px):",
        i.mismatches.len()
    ));
    for (k, n) in &by {
        out.push_str(&format!(" {k}: {n};"));
    }
    out.push('\n');

    // The record's rows, all and by coarse and full blocks.
    for (part, on) in [
        ("all", None),
        ("full blocks", Some(false)),
        ("coarse blocks", Some(true)),
    ] {
        let keep =
            |k: usize| on.is_none_or(|c| i.coarse.contains(&i.faces[k].0.map(|x| x >> 6)) == c);
        let mut added = [[0.0f64; 4]; 2];
        let mut n_dark = 0usize;
        let mut over = [0.0f64; 6];
        let mut e_ref = 0.0f64;
        let mut n = 0usize;
        for (p, f) in i.px_face.iter().enumerate() {
            let Some(f) = *f else { continue };
            if !keep(f) {
                continue;
            }
            let r = &i.rows[f];
            n += 1;
            let c = candidates(r, i.h_hit[p], i.blend[p]);
            e_ref += f64::from(r.reference);
            for k in 0..6 {
                over[k] += f64::from((c[k] - r.reference).max(0.0));
            }
            if r.reference <= 0.0 {
                n_dark += 1;
                for (j, x) in [c[4], c[5]].into_iter().enumerate() {
                    added[j][0] += f64::from(x - r.horizon);
                    for (b, bar) in [0.02, 0.1, 0.5].iter().enumerate() {
                        added[j][b + 1] += f64::from(u8::from(x - r.horizon > *bar));
                    }
                }
            }
        }
        if n == 0 {
            continue;
        }
        out.push_str(&format!(
            "  record, {part}: {n} key-lit px, {n_dark} truly shadowed\n"
        ));
        for (j, name) in ["own cell", "cell blend"].iter().enumerate() {
            let d = n_dark.max(1) as f64;
            out.push_str(&format!(
                "    {name}: adds on the truly shadowed a mean {:.4}; over 0.02 on {:.2} %, 0.1 {:.2} %, 0.5 {:.2} %\n",
                added[j][0] / d,
                100.0 * added[j][1] / d,
                100.0 * added[j][2] / d,
                100.0 * added[j][3] / d
            ));
        }
        out.push_str(&format!(
            "    light over the reference, summed (px-equivalents; the reference's own sum {e_ref:.0}):"
        ));
        for (name, x) in CANDIDATES.iter().zip(over) {
            out.push_str(&format!(" {name} {x:.0};"));
        }
        out.push('\n');
        // The own cell against the reference pooled over the cell's visible faces of its
        // direction, by pixels.
        let mut pool = std::collections::HashMap::<([i32; 3], u8), (f64, usize, f32)>::new();
        for f in i.px_face.iter().flatten() {
            if !keep(*f) {
                continue;
            }
            let r = &i.rows[*f];
            let Some(own) = r.own else { continue };
            let (v, face) = i.faces[*f];
            let e = pool
                .entry((v.map(|x| x >> 3), face))
                .or_insert((0.0, 0, own));
            e.0 += f64::from(r.reference);
            e.1 += 1;
        }
        let mut diffs: Vec<f32> = pool
            .values()
            .map(|(s, n, own)| own - (*s / *n as f64) as f32)
            .collect();
        let m = diffs.len().max(1) as f64;
        let signed = diffs.iter().map(|x| f64::from(*x)).sum::<f64>() / m;
        let mut abs: Vec<f32> = diffs.iter().map(|x| x.abs()).collect();
        let mean_abs = abs.iter().map(|x| f64::from(*x)).sum::<f64>() / m;
        let o25 = abs.iter().filter(|x| **x > 0.25).count() as f64 / m;
        let o50 = abs.iter().filter(|x| **x > 0.5).count() as f64 / m;
        out.push_str(&format!(
            "    own cell against the pooled reference: {} cell-directions; mean signed {signed:+.4}, mean |d| {mean_abs:.4}, p99 |d| {:.3}; |d| over 0.25 on {:.2} %, 0.5 on {:.2} %\n",
            diffs.len(),
            quantile(&mut abs, 0.99),
            100.0 * o25,
            100.0 * o50
        ));
        diffs.clear();
    }
    out
}
