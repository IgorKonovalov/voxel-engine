//! The sharpness metrics of the surface-look harness (`tests/sharpness.rs`), on the shaded
//! output's luminance `Y = 0.2126 R + 0.7152 G + 0.0722 B` of the display-encoded bytes over
//! 255, and the axis fraction of `tests/look.rs`.

use super::{HEIGHT, WIDTH, decode_normal};

/// Box width in pixels of the fine band at the wall's distance: the odd count nearest 0.4
/// voxel at 13.5 px a voxel (`0.4 x 13.5 = 5.4`).
pub const BOX_FINE: usize = 5;
/// Box width in pixels of the mid band's low edge: the odd count nearest 1.3 voxel
/// (`1.3 x 13.5 = 17.6`).
pub const BOX_MID: usize = 17;

/// The metric window: the central 160 x 90 pixels, `[x0, x1) x [y0, y1)`.
pub const WINDOW: [usize; 4] = [80, 240, 45, 135];

/// Pixels of one voxel at the edge scene's 10-voxel distance: `128.5 / 10`.
pub const EDGE_PX_PER_VOXEL: f64 = 12.85;

/// The luminance of each pixel of a `WIDTH` x `HEIGHT` RGBA8 image, in `[0, 1]`.
pub fn luminance(rgba: &[u8]) -> Vec<f64> {
    rgba.chunks(4)
        .map(|p| {
            (0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])) / 255.0
        })
        .collect()
}

/// A square box blur of width `a` (odd) over the `WIDTH` x `HEIGHT` image `y`, the edge
/// pixels repeated past the border.
pub fn box_blur(y: &[f64], a: usize) -> Vec<f64> {
    assert!(a % 2 == 1, "box width {a} is odd");
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let r = (a / 2) as isize;
    let at = |v: &[f64], x: isize, yy: isize| {
        let x = x.clamp(0, w as isize - 1) as usize;
        let yy = yy.clamp(0, h as isize - 1) as usize;
        v[yy * w + x]
    };
    let mut rows = vec![0.0; w * h];
    for yy in 0..h as isize {
        for x in 0..w as isize {
            rows[yy as usize * w + x as usize] =
                (-r..=r).map(|d| at(y, x + d, yy)).sum::<f64>() / a as f64;
        }
    }
    let mut out = vec![0.0; w * h];
    for yy in 0..h as isize {
        for x in 0..w as isize {
            out[yy as usize * w + x as usize] =
                (-r..=r).map(|d| at(&rows, x, yy + d)).sum::<f64>() / a as f64;
        }
    }
    out
}

/// `sqrt(mean(band^2)) / mean(Y)` over [`WINDOW`].
fn band_contrast(y: &[f64], band: impl Fn(usize) -> f64) -> f64 {
    let w = WIDTH as usize;
    let [x0, x1, y0, y1] = WINDOW;
    let (mut sq, mut sum, mut n) = (0.0, 0.0, 0.0);
    for yy in y0..y1 {
        for x in x0..x1 {
            let i = yy * w + x;
            sq += band(i).powi(2);
            sum += y[i];
            n += 1.0;
        }
    }
    (sq / n).sqrt() / (sum / n)
}

/// Fine-band contrast `S_fine`: `HF = Y - box_5(Y)`, `sqrt(mean(HF^2)) / mean(Y)` over the
/// window.
pub fn s_fine(rgba: &[u8]) -> f64 {
    let y = luminance(rgba);
    let a = box_blur(&y, BOX_FINE);
    band_contrast(&y, |i| y[i] - a[i])
}

/// Mid-band contrast `S_mid`: the same with `box_5(Y) - box_17(Y)`.
pub fn s_mid(rgba: &[u8]) -> f64 {
    let y = luminance(rgba);
    let a = box_blur(&y, BOX_FINE);
    let b = box_blur(&y, BOX_MID);
    band_contrast(&y, |i| a[i] - b[i])
}

/// The left and right plateau columns of the edge scene: the columns whose centres lie 15 to
/// 20 px from the centre line (`WIDTH / 2`, between columns 159 and 160).
fn plateau_columns() -> [std::ops::RangeInclusive<usize>; 2] {
    let c = WIDTH as usize / 2;
    [c - 20..=c - 16, c + 15..=c + 19]
}

/// The edge scene's signal at each pixel of a `view = Normals` render: the angle in degrees of
/// the normal's horizontal part from -x toward -y, `atan2(-n.y, -n.x)`, so the -x face reads
/// 0, the -y face 90 and a bevel between them in between. Unlike the shaded luminance it is
/// flat across each face (no lamp distance or cone) and monotonic across a bevel.
pub fn edge_angle(rgba: &[u8]) -> Vec<f64> {
    rgba.chunks(4)
        .map(|p| {
            let n = decode_normal(p);
            f64::from((-n.y).atan2(-n.x)).to_degrees()
        })
        .collect()
}

/// The column position (pixel centres at `x + 0.5`) where `v`, scanned from column `from`
/// rightward, first reaches `level`, linearly interpolated between pixel centres.
fn crossing(v: &[f64], from: usize, to: usize, level: f64) -> Option<f64> {
    if v[from] >= level {
        return Some(from as f64 + 0.5);
    }
    (from + 1..to).find(|&x| v[x] >= level).map(|x| {
        let (a, b) = (v[x - 1], v[x]);
        x as f64 - 0.5 + (level - a) / (b - a)
    })
}

/// The edge spread of one row of the signal `s`: the plateaus are its means over the
/// [`plateau_columns`], and the spread is the width over which it rises from 10 % to 90 % of
/// the way between them, scanned left to right from 15 px left of the centre line to 15 px
/// right of it. `None` for a row whose plateaus are within a degree of each other or whose
/// levels are not both reached.
pub fn row_spread(s: &[f64], row: usize) -> Option<f64> {
    let w = WIDTH as usize;
    let c = w / 2;
    let line = &s[row * w..(row + 1) * w];
    let [l, r] = plateau_columns().map(|cols| {
        let n = cols.clone().count() as f64;
        cols.map(|x| line[x]).sum::<f64>() / n
    });
    if (r - l).abs() < 1.0 {
        return None;
    }
    let v: Vec<f64> = line.iter().map(|&y| (y - l) / (r - l)).collect();
    let (from, to) = (c - 15, c + 15);
    let lo = crossing(&v, from, to, 0.1)?;
    let hi = crossing(&v, from, to, 0.9)?;
    Some(hi - lo)
}

/// Edge spread `E` in pixels of a `view = Normals` render of the edge scene: the median of
/// [`row_spread`] of its [`edge_angle`] over the central 60 rows, and the rows that had a
/// step; NaN when none had.
pub fn spread_of(normals: &[u8]) -> (f64, usize) {
    let s = edge_angle(normals);
    let h = HEIGHT as usize;
    let mut spreads: Vec<f64> = (h / 2 - 30..h / 2 + 30)
        .filter_map(|row| row_spread(&s, row))
        .collect();
    if spreads.is_empty() {
        return (f64::NAN, 0);
    }
    spreads.sort_by(f64::total_cmp);
    let n = spreads.len();
    let median = if n % 2 == 1 {
        spreads[n / 2]
    } else {
        0.5 * (spreads[n / 2 - 1] + spreads[n / 2])
    };
    (median, n)
}

/// The share of the hit pixels of a `view = Normals` render whose decoded normal lies within
/// 5 degrees of a coordinate axis, a miss (a decoded length under 0.5) not counted.
pub fn axis_fraction(rgba: &[u8]) -> f64 {
    let cos5 = 5f32.to_radians().cos();
    let (mut hits, mut axial) = (0usize, 0usize);
    for p in rgba.chunks(4) {
        let n = decode_normal(p);
        if n.length() < 0.5 {
            continue;
        }
        hits += 1;
        let n = n.normalize();
        if n.x.abs().max(n.y.abs()).max(n.z.abs()) >= cos5 {
            axial += 1;
        }
    }
    axial as f64 / hits.max(1) as f64
}

/// The largest per-channel difference between two RGBA8 images, in 1/255.
pub fn max_channel_diff(a: &[u8], b: &[u8]) -> u8 {
    a.chunks(4)
        .zip(b.chunks(4))
        .map(|(p, q)| (0..3).map(|k| p[k].abs_diff(q[k])).max().unwrap_or(0))
        .max()
        .unwrap_or(0)
}

/// `images` side by side, left to right, as one `n WIDTH` x `HEIGHT` RGBA8 image.
pub fn side_by_side(images: &[&[u8]]) -> Vec<u8> {
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let mut out = Vec::with_capacity(images.len() * w * h * 4);
    for row in 0..h {
        for img in images {
            out.extend_from_slice(&img[row * w * 4..(row + 1) * w * 4]);
        }
    }
    out
}

/// `target/look-sharpness/`, created: where the side-by-side images go for reading by eye.
pub fn out_dir() -> std::path::PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target")
        });
    let dir = target.join("look-sharpness");
    std::fs::create_dir_all(&dir).expect("create target/look-sharpness");
    dir
}

/// Writes an `n WIDTH` x `HEIGHT` image to `path`.
pub fn write_wide_png(path: &std::path::Path, rgba: &[u8], n: u32) {
    let file = std::fs::File::create(path).expect("create side-by-side image");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), n * WIDTH, HEIGHT);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .expect("png header")
        .write_image_data(rgba)
        .expect("png data");
}
