//! The entities' still icons on the CPU (Plan 0077 Phase 7, ADR-0075): the committed library's
//! models traced at the fixed view into 64 x 64 RGBA8, hashed to fixed vectors.

use std::path::Path;
use std::sync::Arc;

use flythrough::entities::icon::{ICON_PX, icon};
use flythrough::entities::{self, library};
use flythrough::{cave_materials, showcase_materials, walker};
use vxe_entity::Model;
use vxe_world::MaterialTable;

/// FNV-1a 64 over `bytes`.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The committed library built against `table`: each entry's name and its model, `None` for
/// an entry in error.
fn models(table: &MaterialTable) -> Vec<(String, Option<Arc<Model>>)> {
    let lib = library::scan(&Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities"));
    let mut w = entities::Walkers::default().with_library(lib);
    w.poll_viewer(0.0, table);
    w.library
        .entries
        .iter()
        .map(|e| {
            let m = e
                .viewer()
                .and_then(|v| v.loaded.as_ref())
                .map(|l| l.model.clone());
            (e.name.clone(), m)
        })
        .collect()
}

fn model_of(table: &MaterialTable, name: &str) -> Option<Arc<Model>> {
    models(table)
        .into_iter()
        .find(|(n, _)| n == name)
        .expect(name)
        .1
}

/// The shade of each solid pixel of `px` in 256ths: the pixel's luminance over its material's
/// albedo luminance (`299 r + 587 g + 114 b`), the material found as the table row whose
/// albedo bytes scaled by some shade `s / 256` give the pixel exactly.
fn shades(px: &[u8], table: &MaterialTable) -> Vec<u32> {
    let albedos: Vec<[u32; 3]> = table
        .rows()
        .iter()
        .map(|m| {
            m.albedo
                .map(|a| u32::from((a.clamp(0.0, 1.0) * 255.0).round() as u8))
        })
        .filter(|c| c.iter().any(|&v| v > 0))
        .collect();
    let lum = |c: [u32; 3]| 299 * c[0] + 587 * c[1] + 114 * c[2];
    px.chunks(4)
        .filter(|p| p[3] != 0)
        .map(|p| {
            let p3 = [p[0], p[1], p[2]].map(u32::from);
            let c = albedos
                .iter()
                .find(|c| (0..=256).any(|s| (0..3).all(|k| c[k] * s / 256 == p3[k])))
                .unwrap_or_else(|| panic!("no row shades to {p3:?}"));
            lum(p3) * 256 / lum(*c)
        })
        .collect()
}

/// Plan 0077 Phase 7 done-when 2, amended by Phase 9 (5): the walker's icon (the cave's table
/// with the walker's rows) spans at least 54 of its 64 pixels on its longer axis (the fit is
/// 56), its solid pixels are at least 18 % of the tile (737 of 4096) and it is centred within
/// one pixel; over its solid pixels the mean shade (luminance over the material's, 256ths) is
/// at least 200 and the darkest of the three visible faces (-x, -y, +z under the fixed light:
/// the `walker` row's albedo `[140, 133, 122]` scaled by 198, 210 and 244 256ths) at least
/// 190, the three at least 8 apart; its bytes hash to a fixed vector. The spider in error
/// under the cave's table has no model, so no icon (its built icon is
/// [`the_spider_icon_is_fixed`]'s).
#[test]
fn the_walker_icon_is_fitted_and_fixed() {
    let table = walker::with_walker(&cave_materials());
    let w = model_of(&table, "walker").expect("the walker builds");
    let px = icon(&w, &table).expect("an icon");
    let n = ICON_PX as usize;
    assert_eq!(px.len(), n * n * 4);
    let solid: Vec<(usize, usize)> = (0..n * n)
        .filter(|&i| px[i * 4 + 3] != 0)
        .map(|i| (i % n, i / n))
        .collect();
    let (x0, x1) = (
        solid.iter().map(|p| p.0).min().unwrap(),
        solid.iter().map(|p| p.0).max().unwrap(),
    );
    let (y0, y1) = (
        solid.iter().map(|p| p.1).min().unwrap(),
        solid.iter().map(|p| p.1).max().unwrap(),
    );
    let (sx, sy) = (x1 - x0 + 1, y1 - y0 + 1);
    let shade = shades(&px, &table);
    let mean = shade.iter().sum::<u32>() / shade.len() as u32;
    println!(
        "walker icon spans {sx} x {sy} pixels at ({x0}, {y0}), {} solid ({:.1} %), mean shade \
         {mean}/256",
        solid.len(),
        100.0 * solid.len() as f64 / (n * n) as f64
    );
    assert!(sx.max(sy) >= 54, "{sx} x {sy}");
    assert!(solid.len() >= 737, "{} solid", solid.len());
    // Centred: the margins on each side differ by at most two pixels (one pixel of centre).
    assert!((x0 as i64 - (n - 1 - x1) as i64).abs() <= 2, "x {x0}..{x1}");
    assert!((y0 as i64 - (n - 1 - y1) as i64).abs() <= 2, "y {y0}..{y1}");
    assert!(mean >= 200, "mean shade {mean}/256");
    let body = [140u32, 133, 122];
    let faces = [198u32, 210, 244];
    for s in faces {
        let want: Vec<u8> = body.iter().map(|&c| (c * s / 256) as u8).collect();
        assert!(
            (0..n * n).any(|i| px[i * 4..i * 4 + 3] == want[..]),
            "the face shaded {s}/256: {want:?}"
        );
    }
    let lum = |c: [u32; 3]| 299 * c[0] + 587 * c[1] + 114 * c[2];
    let face_shades = faces.map(|s| lum(body.map(|c| c * s / 256)) * 256 / lum(body));
    println!("walker face shades {face_shades:?}/256");
    assert!(face_shades.iter().all(|&s| s >= 190), "{face_shades:?}");
    assert!(
        face_shades.windows(2).all(|w| w[1] >= w[0] + 8),
        "{face_shades:?}"
    );
    let hash = fnv(&px);
    println!("walker icon hash {hash:#018x}");
    assert_eq!(hash, WALKER_ICON);

    assert_eq!(
        model_of(&table, "spider"),
        None,
        "the spider is in error here"
    );
}

/// Plan 0077 Phase 7 done-when 2, the spider's half: under the showcase's table (with the
/// walker's rows) the spider's icon hashes to its fixed vector.
#[test]
fn the_spider_icon_is_fixed() {
    let show = walker::with_walker(&showcase_materials());
    let spider = model_of(&show, "spider").expect("the spider builds on the showcase");
    let hash = fnv(&icon(&spider, &show).expect("an icon"));
    println!("spider icon hash {hash:#018x}");
    assert_eq!(hash, SPIDER_ICON);
}

const WALKER_ICON: u64 = 0xcd07_06e0_5e4b_9c08;
const SPIDER_ICON: u64 = 0xf1ab_3d3a_97e5_0c96;

/// The icon's CPU time per entry, release build, by wall clock; with `VXE_ICON_DIR` set,
/// each icon's raw RGBA8 bytes written there as `<name>.rgba`. A reading: it prints and
/// asserts nothing.
mod reading {
    use super::*;

    #[test]
    #[ignore = "reading"]
    #[allow(
        clippy::disallowed_methods,
        reason = "a reading of the CPU's time, by wall clock"
    )]
    fn the_icon_time_per_entry() {
        let show = walker::with_walker(&showcase_materials());
        for (name, m) in models(&show) {
            let Some(m) = m else { continue };
            let mut ms = Vec::new();
            for _ in 0..9 {
                let t = std::time::Instant::now();
                std::hint::black_box(icon(&m, &show));
                ms.push(t.elapsed().as_secs_f64() * 1e3);
            }
            ms.sort_by(f64::total_cmp);
            if let (Ok(dir), Some(px)) = (std::env::var("VXE_ICON_DIR"), icon(&m, &show)) {
                let path = Path::new(&dir).join(format!("{name}.rgba"));
                std::fs::write(&path, px).expect("the icon's file");
            }
            println!(
                "{name}: {} parts, icon median {:.2} ms, min {:.2}, max {:.2} (9 runs)",
                m.parts().len(),
                ms[4],
                ms[0],
                ms[8]
            );
        }
    }
}
