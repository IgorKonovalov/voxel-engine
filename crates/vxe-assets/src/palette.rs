//! MagicaVoxel's default palette, used when a file has no `RGBA` chunk.
//!
//! The spec ("MagicaVoxel .vox File Format", section 8, `default_palette`) lists 256 values as
//! `0xAABBGGRR`. They follow a fixed pattern, generated here instead of transcribed: index 0
//! is empty; indices 1 to 215 are the 6 × 6 × 6 cube of channel levels `ff, cc, 99, 66, 33,
//! 00` with blue changing fastest, then green, then red, black left out; indices 216 to 255
//! are ten-step ramps `ee, dd, bb, aa, 88, 77, 55, 44, 22, 11` of red, green, blue and grey.

/// Channel levels of the colour cube, in palette order.
const CUBE: [u8; 6] = [0xff, 0xcc, 0x99, 0x66, 0x33, 0x00];
/// Levels of the four ramps, in palette order.
const RAMP: [u8; 10] = [0xee, 0xdd, 0xbb, 0xaa, 0x88, 0x77, 0x55, 0x44, 0x22, 0x11];

/// MagicaVoxel's default palette as `[r, g, b, a]` per index; index 0 is `[0, 0, 0, 0]`.
pub const DEFAULT_PALETTE: [[u8; 4]; 256] = default_palette();

const fn default_palette() -> [[u8; 4]; 256] {
    let mut p = [[0u8; 4]; 256];
    let mut i = 1;
    let mut r = 0;
    while r < 6 {
        let mut g = 0;
        while g < 6 {
            let mut b = 0;
            while b < 6 {
                if !(r == 5 && g == 5 && b == 5) {
                    p[i] = [CUBE[r], CUBE[g], CUBE[b], 0xff];
                    i += 1;
                }
                b += 1;
            }
            g += 1;
        }
        r += 1;
    }
    // Red, green, blue, then grey (all three channels).
    let mut ch = 0;
    while ch < 4 {
        let mut k = 0;
        while k < 10 {
            let v = RAMP[k];
            p[i] = match ch {
                0 => [v, 0, 0, 0xff],
                1 => [0, v, 0, 0xff],
                2 => [0, 0, v, 0xff],
                _ => [v, v, v, 0xff],
            };
            i += 1;
            k += 1;
        }
        ch += 1;
    }
    p
}
