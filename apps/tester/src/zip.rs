//! A stored-only zip writer: every entry kept as its bytes (method 0), with its CRC-32, a local
//! header before it and a central directory after the last, which Windows Explorer, `unzip` and
//! Python's `zipfile` read. Names are UTF-8 (general-purpose bit 11); no zip64, so an archive
//! and each entry stay under 4 GiB, and no more than 65 535 entries.

use std::io::{self, Write};

/// The DOS date and time every entry carries: 1980-01-01 00:00, the format's epoch. A report's
/// folder name carries the real time.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;
/// Version needed to extract: 2.0, the stored method's.
const VERSION: u16 = 20;
/// General-purpose flag bit 11: the name is UTF-8.
const UTF8: u16 = 1 << 11;

struct Entry {
    name: String,
    crc: u32,
    size: u32,
    offset: u32,
}

/// Writes `files` (name inside the archive, bytes) as a stored zip to `out`.
pub fn write_zip(out: &mut impl Write, files: &[(String, Vec<u8>)]) -> io::Result<()> {
    let too_big = |what: &str| io::Error::new(io::ErrorKind::InvalidInput, what.to_owned());
    if files.len() > usize::from(u16::MAX) {
        return Err(too_big("more than 65535 entries"));
    }
    let mut entries = Vec::with_capacity(files.len());
    let mut offset: u64 = 0;
    for (name, bytes) in files {
        let size = u32::try_from(bytes.len()).map_err(|_| too_big("an entry over 4 GiB"))?;
        let name_len = u16::try_from(name.len()).map_err(|_| too_big("a name over 64 KiB"))?;
        let crc = crc32fast::hash(bytes);
        let at = u32::try_from(offset).map_err(|_| too_big("an archive over 4 GiB"))?;
        let mut h = Vec::with_capacity(30 + name.len());
        h.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        h.extend_from_slice(&VERSION.to_le_bytes());
        h.extend_from_slice(&UTF8.to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes()); // stored
        h.extend_from_slice(&DOS_TIME.to_le_bytes());
        h.extend_from_slice(&DOS_DATE.to_le_bytes());
        h.extend_from_slice(&crc.to_le_bytes());
        h.extend_from_slice(&size.to_le_bytes());
        h.extend_from_slice(&size.to_le_bytes());
        h.extend_from_slice(&name_len.to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes()); // extra
        h.extend_from_slice(name.as_bytes());
        out.write_all(&h)?;
        out.write_all(bytes)?;
        offset += h.len() as u64 + bytes.len() as u64;
        entries.push(Entry {
            name: name.clone(),
            crc,
            size,
            offset: at,
        });
    }
    let dir_at = u32::try_from(offset).map_err(|_| too_big("an archive over 4 GiB"))?;
    let mut dir = Vec::new();
    for e in &entries {
        dir.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        dir.extend_from_slice(&VERSION.to_le_bytes()); // made by: MS-DOS, 2.0
        dir.extend_from_slice(&VERSION.to_le_bytes());
        dir.extend_from_slice(&UTF8.to_le_bytes());
        dir.extend_from_slice(&0u16.to_le_bytes());
        dir.extend_from_slice(&DOS_TIME.to_le_bytes());
        dir.extend_from_slice(&DOS_DATE.to_le_bytes());
        dir.extend_from_slice(&e.crc.to_le_bytes());
        dir.extend_from_slice(&e.size.to_le_bytes());
        dir.extend_from_slice(&e.size.to_le_bytes());
        dir.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
        dir.extend_from_slice(&0u16.to_le_bytes()); // extra
        dir.extend_from_slice(&0u16.to_le_bytes()); // comment
        dir.extend_from_slice(&0u16.to_le_bytes()); // disk
        dir.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        dir.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        dir.extend_from_slice(&e.offset.to_le_bytes());
        dir.extend_from_slice(e.name.as_bytes());
    }
    let dir_len = u32::try_from(dir.len()).map_err(|_| too_big("a directory over 4 GiB"))?;
    let n = entries.len() as u16;
    dir.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    dir.extend_from_slice(&0u16.to_le_bytes());
    dir.extend_from_slice(&0u16.to_le_bytes());
    dir.extend_from_slice(&n.to_le_bytes());
    dir.extend_from_slice(&n.to_le_bytes());
    dir.extend_from_slice(&dir_len.to_le_bytes());
    dir.extend_from_slice(&dir_at.to_le_bytes());
    dir.extend_from_slice(&0u16.to_le_bytes()); // comment
    out.write_all(&dir)
}

/// Every file under `root`, depth first in name order, as (the path relative to `root` with
/// `/` separators, under `prefix/`, and its bytes).
pub fn collect(root: &std::path::Path, prefix: &str) -> io::Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    walk(root, prefix, &mut out)?;
    Ok(out)
}

fn walk(dir: &std::path::Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) -> io::Result<()> {
    let mut names: Vec<_> = std::fs::read_dir(dir)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|e| e.path())
        .collect();
    names.sort();
    for p in names {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let inside = format!("{prefix}/{name}");
        if p.is_dir() {
            walk(&p, &inside, out)?;
        } else {
            out.push((inside, std::fs::read(&p)?));
        }
    }
    Ok(())
}
