//! A run's report (`--report DIR` writes `DIR/run.json`): what the run was, on which adapter,
//! and its frame times as nearest-rank percentiles, the warm-up apart. Written with `std` only;
//! [`json`] is the minimal reader the tester build's launcher reads it back with.
//!
//! The recorder's series are allocated once, for the path's length at 240 frames a second;
//! past that (a tour left running) each series keeps its newest frames as a ring, so the frame
//! path never allocates.

use std::fmt::Write as _;

use crate::path::WARMUP_S;

/// The `schema` field: bumped when a field changes meaning or goes away.
pub const SCHEMA: u32 = 1;

/// Frames a second the recorder's series are sized for.
pub const SIZED_FPS: f32 = 240.0;

/// A wall frame over this is a hitch the report counts (`over_33ms`): under 30 frames a
/// second.
pub const OVER_33_MS: f64 = 33.3;
/// A wall frame over this is a stall the report counts (`over_100ms`).
pub const OVER_100_MS: f64 = 100.0;

/// The commit the binary was built from: `VXE_COMMIT` at build time, `unknown` without it.
pub const COMMIT: &str = match option_env!("VXE_COMMIT") {
    Some(c) => c,
    None => "unknown",
};

/// Nearest-rank percentiles of one series, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Percentiles {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

/// The `p`-th percentile (0 to 100] of `sorted` (ascending, not empty) by nearest rank: the
/// value at rank `ceil(p / 100 * n)`, counted from 1.
pub fn nearest_rank(sorted: &[f64], p: f64) -> f64 {
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

impl Percentiles {
    /// The percentiles of `v`, sorted in place; `None` when it is empty.
    pub fn of(v: &mut [f64]) -> Option<Self> {
        if v.is_empty() {
            return None;
        }
        v.sort_by(f64::total_cmp);
        Some(Self {
            p50: nearest_rank(v, 50.0),
            p95: nearest_rank(v, 95.0),
            p99: nearest_rank(v, 99.0),
            max: nearest_rank(v, 100.0),
        })
    }
}

/// The adapter a run drew on, as `wgpu` names it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AdapterLine {
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    pub device_type: String,
    pub driver: String,
    pub driver_info: String,
    pub backend: String,
}

/// One run's report, the contents of `run.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct RunReport {
    pub version: &'static str,
    pub commit: &'static str,
    pub scene: &'static str,
    pub path: &'static str,
    pub mode: &'static str,
    pub output: [u32; 2],
    pub internal: [u32; 2],
    pub internal_scale: &'static str,
    pub upscaler: &'static str,
    pub adapter: Option<AdapterLine>,
    /// The path's length in seconds.
    pub path_s: f32,
    /// Frames measured: the run's frames after the warm-up.
    pub frames: u32,
    pub warmup_frames: u32,
    pub warmup_s: f32,
    pub wall_ms: Option<Percentiles>,
    pub cpu_ms: Option<Percentiles>,
    /// `None` without the adapter's timestamp queries.
    pub gpu_ms: Option<Percentiles>,
    pub over_33ms: u32,
    pub over_100ms: u32,
    pub peak_rss_bytes: Option<u64>,
    /// `completed` or `aborted`; `failed` when the session ended on an error.
    pub ended: &'static str,
}

/// Appends `s` to `out` as a JSON string, quoted, with `"`, `\` and control characters
/// escaped.
pub fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A JSON number for `v`: three decimals, `null` when not finite.
fn write_ms(out: &mut String, v: f64) {
    if v.is_finite() {
        let _ = write!(out, "{v:.3}");
    } else {
        out.push_str("null");
    }
}

fn write_percentiles(out: &mut String, p: Option<Percentiles>) {
    match p {
        None => out.push_str("null"),
        Some(p) => {
            out.push_str("{\"p50\": ");
            write_ms(out, p.p50);
            out.push_str(", \"p95\": ");
            write_ms(out, p.p95);
            out.push_str(", \"p99\": ");
            write_ms(out, p.p99);
            out.push_str(", \"max\": ");
            write_ms(out, p.max);
            out.push('}');
        }
    }
}

impl RunReport {
    /// The report as a JSON object, one field a line.
    pub fn to_json(&self) -> String {
        let mut o = String::new();
        let field = |o: &mut String, name: &str| {
            if o.len() > 2 {
                o.push_str(",\n");
            }
            o.push_str("  ");
            write_json_string(o, name);
            o.push_str(": ");
        };
        o.push_str("{\n");
        field(&mut o, "schema");
        let _ = write!(o, "{SCHEMA}");
        for (name, v) in [
            ("version", self.version),
            ("commit", self.commit),
            ("scene", self.scene),
            ("path", self.path),
            ("mode", self.mode),
        ] {
            field(&mut o, name);
            write_json_string(&mut o, v);
        }
        field(&mut o, "output");
        let _ = write!(o, "[{}, {}]", self.output[0], self.output[1]);
        field(&mut o, "internal");
        let _ = write!(o, "[{}, {}]", self.internal[0], self.internal[1]);
        field(&mut o, "internal_scale");
        write_json_string(&mut o, self.internal_scale);
        field(&mut o, "upscaler");
        write_json_string(&mut o, self.upscaler);
        field(&mut o, "adapter");
        match &self.adapter {
            None => o.push_str("null"),
            Some(a) => {
                o.push_str("{\"name\": ");
                write_json_string(&mut o, &a.name);
                let _ = write!(o, ", \"vendor\": {}, \"device\": {}", a.vendor, a.device);
                for (name, v) in [
                    ("device_type", &a.device_type),
                    ("driver", &a.driver),
                    ("driver_info", &a.driver_info),
                    ("backend", &a.backend),
                ] {
                    o.push_str(", ");
                    write_json_string(&mut o, name);
                    o.push_str(": ");
                    write_json_string(&mut o, v);
                }
                o.push('}');
            }
        }
        field(&mut o, "path_s");
        write_ms(&mut o, f64::from(self.path_s));
        field(&mut o, "frames");
        let _ = write!(o, "{}", self.frames);
        field(&mut o, "warmup_frames");
        let _ = write!(o, "{}", self.warmup_frames);
        field(&mut o, "warmup_s");
        write_ms(&mut o, f64::from(self.warmup_s));
        for (name, p) in [
            ("wall_ms", self.wall_ms),
            ("cpu_ms", self.cpu_ms),
            ("gpu_ms", self.gpu_ms),
        ] {
            field(&mut o, name);
            write_percentiles(&mut o, p);
        }
        field(&mut o, "over_33ms");
        let _ = write!(o, "{}", self.over_33ms);
        field(&mut o, "over_100ms");
        let _ = write!(o, "{}", self.over_100ms);
        field(&mut o, "peak_rss_bytes");
        match self.peak_rss_bytes {
            Some(b) => {
                let _ = write!(o, "{b}");
            }
            None => o.push_str("null"),
        }
        field(&mut o, "ended");
        write_json_string(&mut o, self.ended);
        o.push_str("\n}\n");
        o
    }
}

/// A series kept in a buffer allocated once: appended to until full, then a ring over its
/// newest values.
#[derive(Clone, Debug, Default)]
struct Series {
    v: Vec<f64>,
    next: usize,
}

impl Series {
    fn with_capacity(n: usize) -> Self {
        Self {
            v: Vec::with_capacity(n.max(1)),
            next: 0,
        }
    }

    fn push(&mut self, x: f64) {
        if self.v.len() < self.v.capacity() {
            self.v.push(x);
        } else {
            self.v[self.next] = x;
            self.next = (self.next + 1) % self.v.len();
        }
    }
}

/// The frame times of a path run, the warm-up apart.
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    wall: Series,
    cpu: Series,
    gpu: Series,
    /// Every measured frame's count of hitches, including those the ring dropped.
    frames: u32,
    over_33: u32,
    over_100: u32,
    warmup_frames: u32,
    warmup_s: f32,
}

impl Recorder {
    /// A recorder sized for a path of `path_s` seconds at [`SIZED_FPS`].
    pub fn for_path(path_s: f32) -> Self {
        let n = ((path_s.max(1.0) + 1.0) * SIZED_FPS) as usize;
        Self {
            wall: Series::with_capacity(n),
            cpu: Series::with_capacity(n),
            gpu: Series::with_capacity(n),
            ..Self::default()
        }
    }

    /// Records a frame that ended `clock_s` seconds into the run: its wall time since the frame
    /// before, its render-thread CPU time and, when a timer read one, its GPU time. A frame
    /// whose clock is inside [`WARMUP_S`] counts toward the warm-up only.
    pub fn push(&mut self, clock_s: f32, wall_ms: f64, cpu_ms: f64, gpu_ms: Option<f64>) {
        if clock_s < WARMUP_S {
            self.warmup_frames += 1;
            self.warmup_s = clock_s;
            return;
        }
        self.frames += 1;
        self.wall.push(wall_ms);
        self.cpu.push(cpu_ms);
        if let Some(g) = gpu_ms {
            self.gpu.push(g);
        }
        self.over_33 += u32::from(wall_ms > OVER_33_MS);
        self.over_100 += u32::from(wall_ms > OVER_100_MS);
    }

    /// Fills `r`'s frame fields from the recorded frames.
    pub fn fill(&self, r: &mut RunReport) {
        r.frames = self.frames;
        r.warmup_frames = self.warmup_frames;
        r.warmup_s = self.warmup_s;
        r.wall_ms = Percentiles::of(&mut self.wall.v.clone());
        r.cpu_ms = Percentiles::of(&mut self.cpu.v.clone());
        r.gpu_ms = Percentiles::of(&mut self.gpu.v.clone());
        r.over_33ms = self.over_33;
        r.over_100ms = self.over_100;
    }
}

/// A minimal JSON reader: enough for `run.json` and `system.json` (objects, arrays, strings
/// with the escapes [`write_json_string`] writes plus `\uXXXX`, numbers, `true`, `false`,
/// `null`).
pub mod json {
    /// A parsed JSON value; an object keeps its fields in order.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Value {
        Null,
        Bool(bool),
        Num(f64),
        Str(String),
        Arr(Vec<Value>),
        Obj(Vec<(String, Value)>),
    }

    impl Value {
        /// Field `key` of an object.
        pub fn get(&self, key: &str) -> Option<&Value> {
            match self {
                Value::Obj(f) => f.iter().find(|(k, _)| k == key).map(|(_, v)| v),
                _ => None,
            }
        }

        pub fn as_f64(&self) -> Option<f64> {
            match self {
                Value::Num(n) => Some(*n),
                _ => None,
            }
        }

        pub fn as_str(&self) -> Option<&str> {
            match self {
                Value::Str(s) => Some(s),
                _ => None,
            }
        }

        pub fn as_array(&self) -> Option<&[Value]> {
            match self {
                Value::Arr(a) => Some(a),
                _ => None,
            }
        }
    }

    /// Parses one JSON value filling all of `s` but whitespace.
    pub fn parse(s: &str) -> Result<Value, String> {
        let mut p = Parser {
            b: s.as_bytes(),
            i: 0,
        };
        let v = p.value()?;
        p.ws();
        if p.i != p.b.len() {
            return Err(format!("trailing bytes at {}", p.i));
        }
        Ok(v)
    }

    struct Parser<'a> {
        b: &'a [u8],
        i: usize,
    }

    impl Parser<'_> {
        fn ws(&mut self) {
            while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
        }

        fn peek(&mut self) -> Option<u8> {
            self.ws();
            self.b.get(self.i).copied()
        }

        fn expect(&mut self, c: u8) -> Result<(), String> {
            if self.peek() == Some(c) {
                self.i += 1;
                Ok(())
            } else {
                Err(format!("expected {:?} at {}", c as char, self.i))
            }
        }

        fn word(&mut self, w: &str, v: Value) -> Result<Value, String> {
            if self.b[self.i..].starts_with(w.as_bytes()) {
                self.i += w.len();
                Ok(v)
            } else {
                Err(format!("bad literal at {}", self.i))
            }
        }

        fn value(&mut self) -> Result<Value, String> {
            match self.peek() {
                Some(b'{') => {
                    self.i += 1;
                    let mut fields = Vec::new();
                    if self.peek() == Some(b'}') {
                        self.i += 1;
                        return Ok(Value::Obj(fields));
                    }
                    loop {
                        let k = self.string()?;
                        self.expect(b':')?;
                        fields.push((k, self.value()?));
                        match self.peek() {
                            Some(b',') => self.i += 1,
                            Some(b'}') => {
                                self.i += 1;
                                return Ok(Value::Obj(fields));
                            }
                            _ => return Err(format!("expected , or }} at {}", self.i)),
                        }
                    }
                }
                Some(b'[') => {
                    self.i += 1;
                    let mut items = Vec::new();
                    if self.peek() == Some(b']') {
                        self.i += 1;
                        return Ok(Value::Arr(items));
                    }
                    loop {
                        items.push(self.value()?);
                        match self.peek() {
                            Some(b',') => self.i += 1,
                            Some(b']') => {
                                self.i += 1;
                                return Ok(Value::Arr(items));
                            }
                            _ => return Err(format!("expected , or ] at {}", self.i)),
                        }
                    }
                }
                Some(b'"') => Ok(Value::Str(self.string()?)),
                Some(b't') => self.word("true", Value::Bool(true)),
                Some(b'f') => self.word("false", Value::Bool(false)),
                Some(b'n') => self.word("null", Value::Null),
                Some(c) if c == b'-' || c.is_ascii_digit() => {
                    let from = self.i;
                    while self.i < self.b.len()
                        && matches!(
                            self.b[self.i],
                            b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
                        )
                    {
                        self.i += 1;
                    }
                    let text =
                        std::str::from_utf8(&self.b[from..self.i]).map_err(|e| e.to_string())?;
                    text.parse()
                        .map(Value::Num)
                        .map_err(|e| format!("number {text:?}: {e}"))
                }
                _ => Err(format!("unexpected byte at {}", self.i)),
            }
        }

        fn string(&mut self) -> Result<String, String> {
            self.expect(b'"')?;
            let mut out: Vec<u8> = Vec::new();
            loop {
                let Some(&c) = self.b.get(self.i) else {
                    return Err("unterminated string".to_owned());
                };
                self.i += 1;
                match c {
                    b'"' => break,
                    b'\\' => {
                        let Some(&e) = self.b.get(self.i) else {
                            return Err("unterminated escape".to_owned());
                        };
                        self.i += 1;
                        match e {
                            b'"' => out.push(b'"'),
                            b'\\' => out.push(b'\\'),
                            b'/' => out.push(b'/'),
                            b'n' => out.push(b'\n'),
                            b'r' => out.push(b'\r'),
                            b't' => out.push(b'\t'),
                            b'b' => out.push(8),
                            b'f' => out.push(12),
                            b'u' => {
                                let hex = self
                                    .b
                                    .get(self.i..self.i + 4)
                                    .and_then(|h| std::str::from_utf8(h).ok())
                                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                                    .ok_or("bad \\u escape")?;
                                self.i += 4;
                                let ch = char::from_u32(hex).unwrap_or('\u{fffd}');
                                let mut buf = [0; 4];
                                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                            }
                            _ => return Err(format!("bad escape at {}", self.i)),
                        }
                    }
                    c => out.push(c),
                }
            }
            String::from_utf8(out).map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::json::{Value, parse};
    use super::*;

    fn report() -> RunReport {
        RunReport {
            version: "0.0.0",
            commit: "abc",
            scene: "showcase",
            path: "showcase",
            mode: "bench",
            output: [1280, 720],
            internal: [853, 480],
            internal_scale: "1.5",
            upscaler: "fsr2",
            adapter: Some(AdapterLine {
                name: "Test \"GPU\" \\ v2\nline".to_owned(),
                vendor: 0x10de,
                device: 0x24dd,
                device_type: "DiscreteGpu".to_owned(),
                driver: "drv".to_owned(),
                driver_info: "1.2.3".to_owned(),
                backend: "Vulkan".to_owned(),
            }),
            path_s: 37.4,
            frames: 0,
            warmup_frames: 0,
            warmup_s: 0.0,
            wall_ms: None,
            cpu_ms: None,
            gpu_ms: None,
            over_33ms: 0,
            over_100ms: 0,
            peak_rss_bytes: Some(123_456_789),
            ended: "completed",
        }
    }

    /// 100 measured frames whose wall times are 1 to 100 ms in a shuffled order
    /// (`(37 k mod 100) + 1` is a permutation), after 3 warm-up frames of 500 ms. Nearest rank over 1..=100:
    /// p50 is the 50th value, 50 ms; p95 95; p99 99; max 100. Over 33.3 ms: 34..=100, 67
    /// frames; over 100 ms: none (100 is not over).
    #[test]
    fn the_report_reads_nearest_rank_percentiles_and_hitches() {
        let mut rec = Recorder::for_path(30.0);
        for k in 0..3 {
            rec.push(0.5 * k as f32, 500.0, 1.0, None);
        }
        for k in 0..100u32 {
            let wall = f64::from((37 * k) % 100 + 1);
            rec.push(3.0 + k as f32 * 0.01, wall, wall / 2.0, Some(wall / 4.0));
        }
        let mut r = report();
        rec.fill(&mut r);
        assert_eq!(r.frames, 100);
        assert_eq!(r.warmup_frames, 3);
        assert_eq!(r.warmup_s, 1.0);
        let wall = r.wall_ms.expect("wall");
        assert_eq!(
            [wall.p50, wall.p95, wall.p99, wall.max],
            [50.0, 95.0, 99.0, 100.0]
        );
        let gpu = r.gpu_ms.expect("gpu");
        assert_eq!([gpu.p50, gpu.max], [12.5, 25.0]);
        assert_eq!(r.over_33ms, 67);
        assert_eq!(r.over_100ms, 0);

        let v = parse(&r.to_json()).expect("the report parses");
        let num = |path: &[&str]| {
            path.iter()
                .try_fold(&v, |v, k| v.get(k))
                .and_then(Value::as_f64)
                .unwrap_or_else(|| panic!("{path:?} is not a number"))
        };
        assert_eq!(num(&["schema"]), 1.0);
        assert_eq!(num(&["wall_ms", "p50"]), 50.0);
        assert_eq!(num(&["wall_ms", "p95"]), 95.0);
        assert_eq!(num(&["wall_ms", "p99"]), 99.0);
        assert_eq!(num(&["wall_ms", "max"]), 100.0);
        assert_eq!(num(&["cpu_ms", "p50"]), 25.0);
        assert_eq!(num(&["over_33ms"]), 67.0);
        assert_eq!(num(&["over_100ms"]), 0.0);
        assert_eq!(num(&["frames"]), 100.0);
        assert_eq!(num(&["warmup_frames"]), 3.0);
        assert_eq!(num(&["peak_rss_bytes"]), 123_456_789.0);
        assert_eq!(num(&["adapter", "vendor"]), f64::from(0x10de_u32));
        assert_eq!(v.get("scene").and_then(Value::as_str), Some("showcase"));
        assert_eq!(v.get("ended").and_then(Value::as_str), Some("completed"));
        assert_eq!(
            v.get("output")
                .and_then(Value::as_array)
                .map(<[Value]>::len),
            Some(2)
        );
        assert_eq!(
            v.get("adapter")
                .and_then(|a| a.get("name"))
                .and_then(Value::as_str),
            Some("Test \"GPU\" \\ v2\nline")
        );
    }

    #[test]
    fn a_string_with_a_quote_a_backslash_and_a_newline_is_escaped() {
        let mut out = String::new();
        write_json_string(&mut out, "a\"b\\c\nd\u{1}");
        assert_eq!(out, r#""a\"b\\c\nd\u0001""#);
        assert_eq!(parse(&out), Ok(Value::Str("a\"b\\c\nd\u{1}".to_owned())));
    }

    #[test]
    fn missing_series_and_memory_are_null() {
        let r = report();
        let v = parse(&r.to_json()).expect("parses");
        assert_eq!(v.get("gpu_ms"), Some(&Value::Null));
        let r = RunReport {
            peak_rss_bytes: None,
            adapter: None,
            ..report()
        };
        let v = parse(&r.to_json()).expect("parses");
        assert_eq!(v.get("peak_rss_bytes"), Some(&Value::Null));
        assert_eq!(v.get("adapter"), Some(&Value::Null));
    }

    /// Past its capacity a series keeps its newest frames, and the hitch counts keep every
    /// frame.
    #[test]
    fn a_full_recorder_keeps_its_newest_frames() {
        let mut rec = Recorder::for_path(1.0);
        let cap = rec.wall.v.capacity();
        for k in 0..(cap + 10) {
            rec.push(5.0, if k < 10 { 1000.0 } else { 1.0 }, 1.0, None);
        }
        let mut r = report();
        rec.fill(&mut r);
        assert_eq!(r.frames as usize, cap + 10);
        assert_eq!(r.over_100ms, 10);
        assert_eq!(r.wall_ms.map(|p| p.max), Some(1.0));
        assert_eq!(rec.wall.v.capacity(), cap);
    }
}
