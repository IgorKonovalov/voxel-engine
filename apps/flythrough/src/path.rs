//! Camera paths: a keyframed flight through a scene, the same on every machine, which the
//! fly-through plays as a benchmark (`--path NAME`, once, timed by the wall clock) and the
//! tester build's launcher starts per scene (ADR-0054).
//!
//! A path is a list of [`Keyframe`]s `(t_s, eye, yaw, pitch)`. The eye between two keyframes
//! is a centripetal Catmull-Rom spline (Barry and Goldman's pyramid, knots spaced by the square
//! root of the distance), which neither overshoots nor loops on unevenly spaced keys; the
//! segment's share of time maps linearly onto its knot interval. Yaw and pitch are a monotone
//! cubic (Fritsch and Butland's tangents) over the keyframe times, the yaw unwrapped key to key
//! along the shortest arc, so a turn from 350 to 10 degrees passes through 0, and no angle
//! reverses inside a segment whose keys move one way.
//!
//! Sampling at a keyframe's time returns that keyframe exactly; before the first time the first,
//! after the last the last. Every path is laid against the scene's world at [`PATH_SEED`], and
//! the air test (`tests/path.rs`) checks every keyframe and every quarter-second sample against
//! the scene's source, so a generator change that moves rock into a path fails the gate.

use vxe_app::{Axis, Input, KeyCode, MouseButton, PadButton};

/// The world seed every path is laid against: the fly-through's default `--seed`.
pub const PATH_SEED: u64 = 1;

/// The first seconds of a benchmark run, excluded from its percentiles (streaming and the
/// light's first passes settle) and reported on their own.
pub const WARMUP_S: f32 = 2.0;

/// One keyframe: the time from the path's start in seconds, the eye in voxels (z up), the
/// view's yaw (0 looks along +x, 90 along +y) and pitch (up positive) in degrees, and whether
/// the camera cuts to it: a cut key starts a new leg, the camera holding the previous key's pose
/// until the cut key's time and no spline joining the two (a room no 1 m wide passage reaches).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe {
    pub t_s: f32,
    pub eye: [f32; 3],
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub cut: bool,
}

/// Shorthand for the path tables: a key the spline flies to.
pub const fn key(t_s: f32, eye: [f32; 3], yaw_deg: f32, pitch_deg: f32) -> Keyframe {
    Keyframe {
        t_s,
        eye,
        yaw_deg,
        pitch_deg,
        cut: false,
    }
}

/// A key the camera cuts to (see [`Keyframe::cut`]).
pub const fn cut(t_s: f32, eye: [f32; 3], yaw_deg: f32, pitch_deg: f32) -> Keyframe {
    Keyframe {
        cut: true,
        ..key(t_s, eye, yaw_deg, pitch_deg)
    }
}

/// The scene a path flies through, and the fly-through's flags that build it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    /// The streamed cave (`--source cave`, the default).
    Cave,
    /// The showcase hall (`--source showcase`).
    Showcase,
    /// The castle in the streamed cave's hall (`--castle`).
    Castle,
}

impl Scene {
    pub const ALL: [Self; 3] = [Self::Cave, Self::Showcase, Self::Castle];

    pub fn name(self) -> &'static str {
        match self {
            Self::Cave => "cave",
            Self::Showcase => "showcase",
            Self::Castle => "castle",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    /// The fly-through's flags that build this scene.
    pub fn flags(self) -> &'static [&'static str] {
        match self {
            Self::Cave => &["--source", "cave"],
            Self::Showcase => &["--source", "showcase"],
            Self::Castle => &["--castle"],
        }
    }
}

/// A named path through one scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPath {
    pub name: &'static str,
    pub scene: Scene,
    pub keys: &'static [Keyframe],
}

/// The camera a path places at one time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathPose {
    pub eye: [f32; 3],
    /// Unwrapped: a path that turns twice round reads 720.
    pub yaw_deg: f32,
    pub pitch_deg: f32,
}

/// The showcase hall: from its `hall` shot down to the floor, east past the chasm's rim, round
/// south by the crystal clusters and back up over the hall to the start, at about 3 m/s. The
/// keys are an air route found on a 6-voxel lattice with 2.2 m of clearance, smoothed and thinned
/// to the farthest key in a clear straight line.
pub const SHOWCASE: CameraPath = CameraPath {
    name: "showcase",
    scene: Scene::Showcase,
    keys: SHOWCASE_KEYS,
};

const SHOWCASE_KEYS: &[Keyframe] = &[
    key(0.0, [189.5, 387.5, 219.5], -39.0, -20.0),
    key(5.3, [315.5, 321.5, 147.5], -16.0, 0.0),
    key(5.6, [321.5, 327.5, 147.5], -20.0, 0.0),
    key(9.3, [423.5, 285.5, 153.5], 8.0, 0.0),
    key(13.8, [549.5, 339.5, 156.0], 0.0, 1.0),
    key(16.3, [621.5, 351.5, 159.5], -35.0, -2.0),
    key(21.6, [585.5, 199.5, 134.0], 179.0, 8.0),
    key(23.7, [525.5, 185.0, 152.5], 138.0, 11.0),
    key(28.9, [417.5, 282.0, 210.0], 162.0, -1.0),
    key(32.1, [327.5, 315.5, 201.5], 168.0, -13.0),
    key(37.4, [189.5, 387.5, 219.5], 135.0, 0.0),
];

/// The streamed cave from its spawn pocket (`UNBOUNDED_SPAWN`) about 140 m along +x, down
/// through the caverns to 31 m under the spawn and up again, crossing some twenty chunk
/// borders, so the streamer loads new chunks the whole way. Laid like [`SHOWCASE`], to a goal
/// 1400 voxels along +x.
pub const CAVE: CameraPath = CameraPath {
    name: "cave",
    scene: Scene::Cave,
    keys: CAVE_KEYS,
};

const CAVE_KEYS: &[Keyframe] = &[
    key(0.0, [33.5, 33.5, 33.5], 122.0, 25.0),
    key(1.7, [10.0, 65.5, 63.5], 112.0, 25.0),
    key(5.9, [-15.0, 147.5, 159.5], 43.0, 20.0),
    key(8.1, [17.0, 189.0, 199.0], 32.0, 10.0),
    key(8.4, [22.0, 194.0, 203.0], 32.0, 8.0),
    key(8.9, [27.5, 201.5, 213.5], 30.0, 5.0),
    key(9.2, [33.5, 207.5, 219.5], 30.0, 3.0),
    key(14.5, [159.5, 291.5, 261.5], 8.0, -16.0),
    key(15.0, [171.5, 303.5, 261.5], -4.0, -20.0),
    key(16.0, [189.5, 321.5, 249.5], -27.0, -20.0),
    key(16.6, [207.5, 315.5, 249.5], -33.0, -23.0),
    key(17.0, [219.5, 315.5, 243.5], -38.0, -23.0),
    key(17.3, [225.5, 315.5, 237.5], -41.0, -22.0),
    key(18.9, [255.5, 297.5, 207.5], -45.0, -18.0),
    key(24.1, [357.5, 201.0, 138.0], -10.0, -4.0),
    key(25.1, [381.5, 189.5, 153.5], 12.0, -12.0),
    key(25.4, [387.5, 183.5, 153.5], 16.0, -13.0),
    key(28.3, [465.5, 207.5, 123.5], 27.0, -25.0),
    key(32.5, [549.5, 249.5, 39.5], 9.0, -14.0),
    key(36.3, [657.5, 267.5, 3.5], -27.0, -2.0),
    key(39.0, [729.5, 237.5, -8.5], -45.0, -7.0),
    key(39.3, [735.5, 231.5, -2.5], -45.0, -10.0),
    key(40.2, [753.5, 213.5, -2.5], -43.0, -15.0),
    key(41.9, [789.5, 177.5, -8.5], -30.0, -24.0),
    key(45.4, [855.5, 127.5, -74.0], 34.0, -8.0),
    key(48.5, [933.5, 177.5, -92.5], 30.0, -25.0),
    key(49.2, [945.5, 189.5, -104.5], 20.0, -25.0),
    key(49.6, [951.5, 195.5, -110.5], 12.0, -25.0),
    key(51.7, [993.5, 219.5, -152.5], -24.0, -25.0),
    key(53.1, [1023.5, 225.5, -182.5], -34.0, -23.0),
    key(58.3, [1131.5, 170.5, -280.0], 34.0, 5.0),
    key(60.8, [1191.5, 215.0, -273.0], 13.0, 5.0),
    key(61.4, [1197.5, 225.5, -260.5], 4.0, -2.0),
    key(61.7, [1203.5, 231.5, -260.5], 4.0, 0.0),
    key(65.2, [1305.5, 255.5, -248.5], -32.0, 16.0),
    key(65.6, [1317.5, 255.5, -248.5], -36.0, 18.0),
    key(68.4, [1371.5, 213.5, -200.5], -24.0, 6.0),
    key(70.0, [1407.5, 189.5, -182.5], -11.0, -7.0),
    key(71.0, [1437.5, 183.5, -188.5], -16.0, 4.0),
];

/// The castle in the streamed cave's hall, its four shots in order: from the gate (the spawn)
/// up and south over the approach; a cut into the keep, west along the nave; a cut to the
/// lancet outside, then west and up along the keep's south side; a cut to the dungeon's
/// corridor, panning along the cells. No passage into the keep or down the stair is 1 m clear on
/// every side (the stair house's doorway is 13 voxels wide), so the legs join by cuts.
pub const CASTLE: CameraPath = CameraPath {
    name: "castle",
    scene: Scene::Castle,
    keys: CASTLE_KEYS,
};

const CASTLE_KEYS: &[Keyframe] = &[
    key(0.0, [32.5, 32.5, 32.5], 0.0, 0.0),
    key(1.5, [32.5, 32.5, 32.5], 0.0, 0.0),
    key(6.7, [99.5, -56.5, 141.5], -45.0, -10.0),
    cut(8.0, [602.5, 32.5, 112.5], 180.0, 12.0),
    key(13.0, [453.5, 15.5, 93.5], 180.0, 5.0),
    key(14.1, [435.5, 33.5, 111.5], 180.0, 5.0),
    cut(15.5, [492.5, -107.5, 112.5], 90.0, 0.0),
    key(17.0, [492.5, -107.5, 112.5], 90.0, 0.0),
    key(22.3, [339.5, -148.0, 117.5], 150.0, 10.0),
    key(24.1, [303.5, -128.5, 153.5], 135.0, 0.0),
    cut(25.5, [392.5, 182.5, -68.5], -25.0, 0.0),
    key(31.5, [392.5, 182.5, -68.5], 25.0, 0.0),
];

/// Every path, by name.
pub const PATHS: [CameraPath; 3] = [CAVE, SHOWCASE, CASTLE];

impl CameraPath {
    /// The path named `name`.
    pub fn named(name: &str) -> Option<&'static CameraPath> {
        PATHS.iter().find(|p| p.name == name)
    }

    /// The paths' names, comma-separated, for an error message.
    pub fn names() -> String {
        PATHS.iter().map(|p| p.name).collect::<Vec<_>>().join(", ")
    }

    /// The last keyframe's time.
    pub fn duration_s(&self) -> f32 {
        self.keys.last().map_or(0.0, |k| k.t_s)
    }

    /// The camera at `t_s` seconds from the start, clamped to the path's span; between a key and
    /// a cut key after it, the first key's pose.
    pub fn sample(&self, t_s: f32) -> PathPose {
        let keys = self.keys;
        let n = keys.len();
        let first = keys[0];
        if n == 1 || t_s <= first.t_s {
            return pose_of(&first);
        }
        let last = keys[n - 1];
        if t_s >= last.t_s {
            let mut p = pose_of(&last);
            p.yaw_deg = unwrapped_yaw(keys, n - 1);
            return p;
        }
        // The segment [i, i + 1] holding t_s: keys[i].t_s <= t_s < keys[i + 1].t_s.
        let i = keys.partition_point(|k| k.t_s <= t_s) - 1;
        let (a, b) = (keys[i], keys[i + 1]);
        if t_s == a.t_s {
            let mut p = pose_of(&a);
            p.yaw_deg = unwrapped_yaw(keys, i);
            return p;
        }
        if b.cut {
            let mut p = pose_of(&a);
            p.yaw_deg = unwrapped_yaw(keys, i);
            return p;
        }
        let u = (t_s - a.t_s) / (b.t_s - a.t_s);
        let p0 = if i == 0 || a.cut {
            phantom(a.eye, b.eye)
        } else {
            keys[i - 1].eye
        };
        let p3 = if i + 2 < n && !keys[i + 2].cut {
            keys[i + 2].eye
        } else {
            phantom(b.eye, a.eye)
        };
        let eye = centripetal(p0, a.eye, b.eye, p3, u);
        let yaw = |k: usize| unwrapped_yaw(keys, k);
        let pitch = |k: usize| keys[k].pitch_deg;
        PathPose {
            eye,
            yaw_deg: monotone(keys, i, t_s, yaw),
            pitch_deg: monotone(keys, i, t_s, pitch),
        }
    }

    /// The time in `[0, duration]` whose eye is nearest `eye`, searched every 50 ms.
    pub fn nearest_time(&self, eye: [f32; 3]) -> f32 {
        let end = self.duration_s();
        let steps = (end / 0.05).ceil() as u32;
        let mut best = (f32::INFINITY, 0.0);
        for s in 0..=steps {
            let t = (s as f32 * 0.05).min(end);
            let d = dist2(self.sample(t).eye, eye);
            if d < best.0 {
                best = (d, t);
            }
        }
        best.1
    }
}

fn pose_of(k: &Keyframe) -> PathPose {
    PathPose {
        eye: k.eye,
        yaw_deg: k.yaw_deg,
        pitch_deg: k.pitch_deg,
    }
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|k| (a[k] - b[k]) * (a[k] - b[k])).sum()
}

/// The point past `end` mirrored from `inner`: `2 end - inner`, the end tangent of a spline
/// that keeps its last segment's direction.
fn phantom(end: [f32; 3], inner: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|k| 2.0 * end[k] - inner[k])
}

/// Key `k`'s yaw unwrapped from key 0 along the shortest arc between neighbours.
fn unwrapped_yaw(keys: &[Keyframe], k: usize) -> f32 {
    let mut yaw = keys[0].yaw_deg;
    for w in keys[..=k].windows(2) {
        yaw += shortest_arc(w[1].yaw_deg - w[0].yaw_deg);
    }
    yaw
}

/// `d` wrapped into `(-180, 180]`.
fn shortest_arc(d: f32) -> f32 {
    let r = d.rem_euclid(360.0);
    if r > 180.0 { r - 360.0 } else { r }
}

/// Barry and Goldman's pyramid for the centripetal Catmull-Rom segment between `p1` and `p2`
/// at `u` in `[0, 1]`.
fn centripetal(p0: [f32; 3], p1: [f32; 3], p2: [f32; 3], p3: [f32; 3], u: f32) -> [f32; 3] {
    // Knot spacing: the square root of the distance, kept off zero for repeated points.
    let step = |a: [f32; 3], b: [f32; 3]| dist2(a, b).sqrt().sqrt().max(1e-4);
    let t0 = 0.0;
    let t1 = t0 + step(p0, p1);
    let t2 = t1 + step(p1, p2);
    let t3 = t2 + step(p2, p3);
    let t = t1 + u * (t2 - t1);
    let lerp = |a: [f32; 3], b: [f32; 3], ta: f32, tb: f32| {
        let w = (t - ta) / (tb - ta);
        [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * w)
    };
    let a1 = lerp(p0, p1, t0, t1);
    let a2 = lerp(p1, p2, t1, t2);
    let a3 = lerp(p2, p3, t2, t3);
    let b1 = lerp(a1, a2, t0, t2);
    let b2 = lerp(a2, a3, t1, t3);
    lerp(b1, b2, t1, t2)
}

/// A monotone cubic through `value(k)` at the keys' times, sampled at `t_s` inside segment
/// `[i, i + 1]`: Fritsch and Butland's tangents, zero where the secants change sign, so the
/// curve never passes a key's value inside a segment.
fn monotone(keys: &[Keyframe], i: usize, t_s: f32, value: impl Fn(usize) -> f32) -> f32 {
    let n = keys.len();
    let secant = |k: usize| (value(k + 1) - value(k)) / (keys[k + 1].t_s - keys[k].t_s);
    let tangent = |k: usize| -> f32 {
        // A leg's first and last keys take their one segment's secant.
        if k == 0 || keys[k].cut {
            return secant(k);
        }
        if k == n - 1 || keys[k + 1].cut {
            return secant(k - 1);
        }
        let (d0, d1) = (secant(k - 1), secant(k));
        if d0 == 0.0 || d1 == 0.0 || (d0 > 0.0) != (d1 > 0.0) {
            return 0.0;
        }
        let h0 = keys[k].t_s - keys[k - 1].t_s;
        let h1 = keys[k + 1].t_s - keys[k].t_s;
        3.0 * (h0 + h1) / ((2.0 * h1 + h0) / d0 + (h1 + 2.0 * h0) / d1)
    };
    let h = keys[i + 1].t_s - keys[i].t_s;
    let s = (t_s - keys[i].t_s) / h;
    let (y0, y1) = (value(i), value(i + 1));
    let (m0, m1) = (tangent(i) * h, tangent(i + 1) * h);
    let s2 = s * s;
    let s3 = s2 * s;
    (2.0 * s3 - 3.0 * s2 + 1.0) * y0
        + (s3 - 2.0 * s2 + s) * m0
        + (-2.0 * s3 + 3.0 * s2) * y1
        + (s3 - s2) * m1
}

/// How a path plays: once as a benchmark, or looping as a tour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathMode {
    /// Once, by wall time; the run ends at the path's end.
    Bench,
    /// Looping, until the tester takes the camera.
    Tour,
}

impl PathMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Bench => "bench",
            Self::Tour => "tour",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Bench, Self::Tour]
            .into_iter()
            .find(|m| m.name() == name)
    }
}

/// Who drives the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drive {
    /// The path, at `t_s` seconds into it.
    Path { t_s: f32 },
    /// The tester, in free flight.
    Free,
    /// The run was ended before the benchmark path's end.
    Aborted,
    /// The run ended: the benchmark path's end, or `Esc` in a tour.
    Done,
}

/// The inputs a path player reads in one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    /// A move key or the move stick.
    pub moved: bool,
    /// Mouse motion or the look stick.
    pub looked: bool,
    /// A dig (the left click or `R2`).
    pub dug: bool,
    /// `T`: resume the tour.
    pub resume: bool,
    /// `Esc`: end the run.
    pub escape: bool,
}

impl Controls {
    /// This frame's controls from the window's input: the move keys (`W A S D`, `Space`, `C`)
    /// or the pad's move stick and `R1`/`L1`; mouse motion or the look stick; the left click or
    /// `R2` past half its travel; `T`; `Esc`.
    pub fn read(input: &Input) -> Self {
        let stick = |x: Axis, y: Axis| input.axis(x).hypot(input.axis(y)) > DEAD_ZONE;
        let [dx, dy] = input.mouse_delta();
        Self {
            moved: [
                KeyCode::KeyW,
                KeyCode::KeyA,
                KeyCode::KeyS,
                KeyCode::KeyD,
                KeyCode::Space,
                KeyCode::KeyC,
            ]
            .into_iter()
            .any(|k| input.held(k))
                || stick(Axis::MoveX, Axis::MoveY)
                || input.pad_held(PadButton::R1)
                || input.pad_held(PadButton::L1),
            looked: dx != 0.0 || dy != 0.0 || stick(Axis::LookX, Axis::LookY),
            dug: input.clicked(MouseButton::Left) || input.axis(Axis::TriggerR) > 0.5,
            resume: input.pressed(KeyCode::KeyT),
            escape: input.pressed(KeyCode::Escape),
        }
    }
}

/// A stick deflection under this reads as no input (the fly-through's own dead zone).
const DEAD_ZONE: f32 = 0.15;

/// A path played as a benchmark or a tour: its drive, and the seconds since the run's first
/// frame, which the report splits into the warm-up and the measured span.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathPlayer {
    pub path: &'static CameraPath,
    pub mode: PathMode,
    pub drive: Drive,
    /// Seconds since the first frame.
    pub clock_s: f32,
}

impl PathPlayer {
    pub fn new(path: &'static CameraPath, mode: PathMode) -> Self {
        Self {
            path,
            mode,
            drive: Drive::Path { t_s: 0.0 },
            clock_s: 0.0,
        }
    }

    /// Advances by `dt` seconds with this frame's `controls`, the camera's eye at `eye`, and
    /// returns the pose the path sets this frame (`None` while the tester flies or after the
    /// run ended). A benchmark reads only `Esc`.
    pub fn step(&mut self, dt: f32, controls: Controls, eye: [f32; 3]) -> Option<PathPose> {
        self.clock_s += dt;
        let end = self.path.duration_s();
        self.drive = match (self.drive, self.mode) {
            (Drive::Aborted | Drive::Done, _) => self.drive,
            (Drive::Path { .. }, PathMode::Bench) if controls.escape => Drive::Aborted,
            (_, PathMode::Tour) if controls.escape => Drive::Done,
            (Drive::Path { t_s }, PathMode::Bench) => {
                let t = t_s + dt;
                if t >= end {
                    Drive::Done
                } else {
                    Drive::Path { t_s: t }
                }
            }
            (Drive::Path { .. }, PathMode::Tour)
                if controls.moved || controls.looked || controls.dug =>
            {
                Drive::Free
            }
            (Drive::Path { t_s }, PathMode::Tour) => Drive::Path {
                t_s: if end > 0.0 { (t_s + dt) % end } else { 0.0 },
            },
            (Drive::Free, _) if controls.resume => Drive::Path {
                t_s: self.path.nearest_time(eye),
            },
            (Drive::Free, _) => Drive::Free,
        };
        match self.drive {
            Drive::Path { t_s } => Some(self.path.sample(t_s)),
            _ => None,
        }
    }

    /// Whether the run is over.
    pub fn ended(&self) -> bool {
        matches!(self.drive, Drive::Aborted | Drive::Done)
    }

    /// The report's `ended`: `aborted` for a benchmark stopped before its end, else
    /// `completed`.
    pub fn ended_word(&self) -> &'static str {
        match (self.mode, self.drive) {
            (PathMode::Bench, Drive::Done) | (PathMode::Tour, _) => "completed",
            (PathMode::Bench, _) => "aborted",
        }
    }
}

/// An input event of the fly-through's window, as the tester mode's filter sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TesterInput {
    Key(KeyCode),
    /// Mouse motion (or the look stick).
    MouseLook,
    Click(MouseButton),
}

/// What an input does under `--tester`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TesterAction {
    Move,
    Look,
    Dig,
    /// `F1`: the help panel.
    Help,
    /// `T`: resume the tour.
    Resume,
    /// `Esc`: end the run.
    End,
}

/// The keys `--tester` moves the camera with: `W A S D`, `Space` and `C` (up and down), `Shift`
/// (faster).
pub const TESTER_MOVE_KEYS: [KeyCode; 7] = [
    KeyCode::KeyW,
    KeyCode::KeyA,
    KeyCode::KeyS,
    KeyCode::KeyD,
    KeyCode::Space,
    KeyCode::KeyC,
    KeyCode::ShiftLeft,
];

/// The tester mode's filter: movement, look, the left click's dig, `F1`, `T` and `Esc` map to
/// their action; every other key and button (the debug keys among them) to none.
pub fn tester_action(input: TesterInput) -> Option<TesterAction> {
    match input {
        TesterInput::Key(k) if TESTER_MOVE_KEYS.contains(&k) => Some(TesterAction::Move),
        TesterInput::Key(KeyCode::F1) => Some(TesterAction::Help),
        TesterInput::Key(KeyCode::KeyT) => Some(TesterAction::Resume),
        TesterInput::Key(KeyCode::Escape) => Some(TesterAction::End),
        TesterInput::Key(_) => None,
        TesterInput::MouseLook => Some(TesterAction::Look),
        TesterInput::Click(MouseButton::Left) => Some(TesterAction::Dig),
        TesterInput::Click(_) => None,
    }
}
