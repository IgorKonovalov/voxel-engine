//! The stored zip: three files (empty, 1 MiB, a UTF-8 name) read back byte-equal through a
//! central-directory reader here and through Python's `zipfile`, CRCs matching.

use std::path::PathBuf;

use vxe_tester::zip::write_zip;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vxe-tester-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch folder");
    d
}

fn files() -> Vec<(String, Vec<u8>)> {
    let mib: Vec<u8> = (0..1u32 << 20)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    vec![
        ("report/empty.txt".to_owned(), Vec::new()),
        ("report/runs/01-cave-bench/log.bin".to_owned(), mib),
        (
            "report/räksmörgås-日本.json".to_owned(),
            "{\"ok\": true}\n".as_bytes().to_vec(),
        ),
    ]
}

fn u16_at(b: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes([b[at], b[at + 1]]))
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Reads every entry through the end record and the central directory: (name, bytes, the CRC
/// the directory gives), checking each local header agrees.
fn read_zip(z: &[u8]) -> Vec<(String, Vec<u8>, u32)> {
    let end = z.len() - 22;
    assert_eq!(u32_at(z, end), 0x0605_4b50, "end of central directory");
    let n = u16_at(z, end + 10);
    let mut at = u32_at(z, end + 16) as usize;
    let mut out = Vec::new();
    for _ in 0..n {
        assert_eq!(u32_at(z, at), 0x0201_4b50, "central header");
        assert_eq!(u16_at(z, at + 8) & (1 << 11), 1 << 11, "UTF-8 flag");
        assert_eq!(u16_at(z, at + 10), 0, "stored");
        let crc = u32_at(z, at + 16);
        let size = u32_at(z, at + 20) as usize;
        assert_eq!(u32_at(z, at + 24) as usize, size, "stored sizes agree");
        let name_len = u16_at(z, at + 28);
        let extra = u16_at(z, at + 30);
        let comment = u16_at(z, at + 32);
        let local = u32_at(z, at + 42) as usize;
        let name = String::from_utf8(z[at + 46..at + 46 + name_len].to_vec()).expect("UTF-8");
        assert_eq!(u32_at(z, local), 0x0403_4b50, "local header of {name}");
        assert_eq!(u32_at(z, local + 14), crc, "local CRC of {name}");
        let data = local + 30 + u16_at(z, local + 26) + u16_at(z, local + 28);
        out.push((name, z[data..data + size].to_vec(), crc));
        at += 46 + name_len + extra + comment;
    }
    out
}

#[test]
fn a_stored_zip_reads_back_byte_equal() {
    let files = files();
    let mut z = Vec::new();
    write_zip(&mut z, &files).expect("zip");
    let read = read_zip(&z);
    assert_eq!(read.len(), 3);
    for ((name, bytes), (rname, rbytes, crc)) in files.iter().zip(&read) {
        assert_eq!(name, rname);
        assert_eq!(bytes, rbytes, "{name}");
        assert_eq!(*crc, crc32fast::hash(bytes), "{name}");
    }
    assert_eq!(read[0].2, 0, "the empty file's CRC");
}

/// Python's `zipfile` opens it, `testzip` finds every CRC good, and each entry's bytes equal
/// the file written beside it.
#[test]
fn python_zipfile_reads_it() {
    let python = ["python3", "python"].into_iter().find(|p| {
        std::process::Command::new(p)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    });
    let Some(python) = python else {
        eprintln!("skipped: no python3 on this machine");
        return;
    };
    let dir = scratch("zip");
    let files = files();
    let mut z = Vec::new();
    write_zip(&mut z, &files).expect("zip");
    let zip_path = dir.join("report.zip");
    std::fs::write(&zip_path, &z).expect("write the zip");
    for (i, (_, bytes)) in files.iter().enumerate() {
        std::fs::write(dir.join(format!("expect{i}")), bytes).expect("write an expected file");
    }
    let script = r#"
import sys, zipfile, os
d = sys.argv[1]
names = [l.rstrip("\n") for l in open(os.path.join(d, "names.txt"), encoding="utf-8")]
with zipfile.ZipFile(os.path.join(d, "report.zip")) as z:
    assert z.testzip() is None, "a bad CRC"
    assert z.namelist() == names, (z.namelist(), names)
    for i, n in enumerate(names):
        with open(os.path.join(d, "expect%d" % i), "rb") as f:
            assert z.read(n) == f.read(), n
print("zipfile ok", len(names))
"#;
    let names: String = files.iter().map(|(n, _)| format!("{n}\n")).collect();
    std::fs::write(dir.join("names.txt"), names).expect("names");
    let out = std::process::Command::new(python)
        .arg("-I")
        .arg("-c")
        .arg(script)
        .arg(&dir)
        .output()
        .expect("python runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "zipfile: {stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout.trim(), "zipfile ok 3");
    let _ = std::fs::remove_dir_all(&dir);
}
