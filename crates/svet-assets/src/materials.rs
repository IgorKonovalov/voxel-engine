//! A material table as a text file in the `.rig` line grammar (ADR-0065).
//!
//! ```text
//! # a comment runs to the end of its line
//! material rock               # opens a row; the name is the rest of the line
//!   color #6b615a             # sRGB, decoded exactly to linear albedo
//!   roughness 0.9
//!   acoustic 0.1 0.6 0        # absorption, transmission, resonance
//! material castle glass
//!   albedo 0.25 0.35 0.85     # linear
//!   flags translucent
//!   opacity 0.4
//! material ore
//!   emission 2.0 1.2 0.5
//! ```
//!
//! A `material` line opens a row; the statements under it, until the next `material`, set its
//! fields: `color #rrggbb` or `albedo r g b`, `roughness`, `emission r g b`, `opacity`,
//! `hardness`, `acoustic A T R`, `payload <u64>` and `flags` with any of `indestructible`,
//! `translucent` and `crisp`. A field left out keeps `Material::named`'s default. Rows take
//! ids in file order from 1; air is row 0 and is not written. Every error names its line.

use std::fmt::Write as _;

use svet_world::{Material, MaterialFlags, MaterialTable};

use crate::rows::srgb_to_linear;

/// A material file's error: its line (1-based) and what is wrong, quoting the token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialFileError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for MaterialFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "materials line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for MaterialFileError {}

fn err(line: usize, message: impl Into<String>) -> MaterialFileError {
    MaterialFileError {
        line,
        message: message.into(),
    }
}

/// The flag words and their bits, in the order [`MaterialFile::canonical`] writes them.
const FLAGS: [(&str, MaterialFlags); 3] = [
    ("indestructible", MaterialFlags::INDESTRUCTIBLE),
    ("translucent", MaterialFlags::TRANSLUCENT),
    ("crisp", MaterialFlags::CRISP),
];

/// A parsed material file: its rows in file order, which take ids 1, 2, ... in a table.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialFile {
    pub rows: Vec<Material>,
}

fn floats<const N: usize>(
    line: usize,
    kw: &str,
    toks: &[&str],
) -> Result<[f32; N], MaterialFileError> {
    if toks.len() != N {
        return Err(err(
            line,
            format!("`{kw}` takes {N} number(s), found {}", toks.len()),
        ));
    }
    let mut out = [0.0; N];
    for (o, t) in out.iter_mut().zip(toks) {
        *o = t
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| err(line, format!("`{t}` is not a number")))?;
    }
    Ok(out)
}

fn colour(line: usize, toks: &[&str]) -> Result<[f32; 3], MaterialFileError> {
    let bad = || err(line, "`color` takes #rrggbb: 6 hex digits");
    let [tok] = toks else { return Err(bad()) };
    let hex = tok.strip_prefix('#').ok_or_else(bad)?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).expect("checked hex");
    Ok([0, 2, 4].map(|i| srgb_to_linear(byte(i))))
}

/// `raw` without its comment: from the first `#`, except the `#` that opens a `color` line's
/// colour.
fn strip_comment(raw: &str) -> &str {
    let t = raw.trim_start();
    if let Some(rest) = t.strip_prefix("color")
        && rest.starts_with(char::is_whitespace)
    {
        let r = rest.trim_start();
        if r.starts_with('#') {
            let at = raw.len() - r.len() + 1;
            return raw[at..].find('#').map_or(raw, |i| &raw[..at + i]);
        }
    }
    raw.split_once('#').map_or(raw, |(b, _)| b)
}

impl MaterialFile {
    /// Parses a material file. A field before any `material`, an unknown statement or flag, a
    /// malformed number or colour, an empty name or two rows of one name is an error naming
    /// its line.
    pub fn parse(text: &str) -> Result<Self, MaterialFileError> {
        let mut rows: Vec<Material> = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            let body = strip_comment(raw);
            let toks: Vec<&str> = body.split_ascii_whitespace().collect();
            let Some((&kw, args)) = toks.split_first() else {
                continue;
            };
            if kw == "material" {
                let name = args.join(" ");
                if name.is_empty() {
                    return Err(err(line, "`material` needs a name"));
                }
                if rows.iter().any(|m| m.name == name) {
                    return Err(err(line, format!("material `{name}` named twice")));
                }
                rows.push(Material::named(name));
                continue;
            }
            let Some(m) = rows.last_mut() else {
                return Err(err(line, format!("`{kw}` before any `material`")));
            };
            match kw {
                "color" => m.albedo = colour(line, args)?,
                "albedo" => m.albedo = floats(line, kw, args)?,
                "roughness" => [m.roughness] = floats(line, kw, args)?,
                "emission" => m.emission = floats(line, kw, args)?,
                "opacity" => [m.opacity] = floats(line, kw, args)?,
                "hardness" => [m.hardness] = floats(line, kw, args)?,
                "acoustic" => m.acoustic = floats(line, kw, args)?,
                "payload" => {
                    let [t] = args else {
                        return Err(err(line, "`payload` takes one whole number"));
                    };
                    m.payload = t
                        .parse()
                        .map_err(|_| err(line, format!("`{t}` is not a whole number")))?;
                }
                "flags" => {
                    let mut f = MaterialFlags::empty();
                    for t in args {
                        let (_, bit) = FLAGS.iter().find(|(n, _)| n == t).ok_or_else(|| {
                            err(
                                line,
                                format!(
                                    "unknown flag `{t}`: one of indestructible, translucent, crisp"
                                ),
                            )
                        })?;
                        f = f | *bit;
                    }
                    m.flags = f;
                }
                _ => return Err(err(line, format!("unknown statement `{kw}`"))),
            }
        }
        Ok(Self { rows })
    }

    /// The file's rows as a table: air, then the rows from id 1 in file order.
    pub fn table(&self) -> MaterialTable {
        self.rows.iter().cloned().collect()
    }

    /// The file of `table`'s rows past air, as [`MaterialFile::table`] reads it back.
    pub fn of_table(table: &MaterialTable) -> Self {
        Self {
            rows: table.rows().iter().skip(1).cloned().collect(),
        }
    }

    /// The row named `name`, with its id in [`MaterialFile::table`].
    pub fn row(&self, name: &str) -> Option<(u16, &Material)> {
        self.rows
            .iter()
            .position(|m| m.name == name)
            .map(|i| (i as u16 + 1, &self.rows[i]))
    }

    /// The file in canonical form: each row's `material` line, then each field that differs
    /// from `Material::named`'s default on its own indented line, colours as linear `albedo`,
    /// numbers in Rust's shortest form that parses back to the same `f32`. Comments are not
    /// kept; [`MaterialFile::parse`] reads it back to an equal file.
    pub fn canonical(&self) -> String {
        let mut o = String::new();
        let d = Material::named("");
        let three = |o: &mut String, kw: &str, v: [f32; 3]| {
            let _ = writeln!(o, "  {kw} {:?} {:?} {:?}", v[0], v[1], v[2]);
        };
        for m in &self.rows {
            let _ = writeln!(o, "material {}", m.name);
            if m.albedo != d.albedo {
                three(&mut o, "albedo", m.albedo);
            }
            if m.roughness != d.roughness {
                let _ = writeln!(o, "  roughness {:?}", m.roughness);
            }
            if m.emission != d.emission {
                three(&mut o, "emission", m.emission);
            }
            if m.hardness != d.hardness {
                let _ = writeln!(o, "  hardness {:?}", m.hardness);
            }
            if m.flags != d.flags {
                let names: Vec<&str> = FLAGS
                    .iter()
                    .filter(|(_, b)| m.flags.contains(*b))
                    .map(|(n, _)| *n)
                    .collect();
                let _ = writeln!(o, "  flags {}", names.join(" "));
            }
            if m.acoustic != d.acoustic {
                three(&mut o, "acoustic", m.acoustic);
            }
            if m.payload != d.payload {
                let _ = writeln!(o, "  payload {}", m.payload);
            }
            if m.opacity != d.opacity {
                let _ = writeln!(o, "  opacity {:?}", m.opacity);
            }
        }
        o
    }
}
