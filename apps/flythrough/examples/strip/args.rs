//! The strip's flags: one table (`flythrough::cli::table`) the parser reads and `--help` prints.
//! Each row's help is its meaning and default; the example's doc comment has the long form.

use std::path::PathBuf;
use std::str::FromStr;

use flythrough::LookPreset;
use flythrough::cli::table::{Flag, Section, Table, Val};
use vxe_render::{
    AutoExposure, Exposure, LightWeights, LightingResolution, RenderScale, ShadowWalk, StoreRead,
    ViewMode,
};

/// The reading the example runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reading {
    /// The strip along the scripted camera, or a fold, a pan or a drift.
    StripRun,
    /// The hall floor's terraces, on the CPU.
    HallFloorTerraces,
}

/// `--light`: the strip's light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightMode {
    Hero,
    Eye,
    Eye0,
    Baked,
}

impl LightMode {
    pub fn name(self) -> &'static str {
        match self {
            LightMode::Hero => "hero",
            LightMode::Eye => "eye",
            LightMode::Eye0 => "eye0",
            LightMode::Baked => "baked",
        }
    }
}

/// The strip's arguments; each field's default is its flag's.
#[derive(Clone, Debug)]
pub struct Args {
    pub help: bool,
    pub reading: Reading,
    pub adapter: Option<String>,
    pub tag: String,
    /// `--source level`; else the showcase.
    pub level: bool,
    pub shot: Option<String>,
    pub camera: Option<[f32; 5]>,
    pub view: ViewMode,
    pub size: [u32; 2],
    /// `fsr2`, `own` or `off`.
    pub upscaler: &'static str,
    pub look: Option<LookPreset>,
    pub far: Option<[f32; 2]>,
    pub energy: Option<Vec<[u32; 4]>>,
    pub lamp: Option<bool>,
    pub indirect: bool,
    pub filter: bool,
    pub candidates: u32,
    pub shadow: ShadowWalk,
    pub mean: bool,
    pub exposure: Option<Exposure>,
    pub settle: u32,
    pub frames: u32,
    pub every: u32,
    pub speed: f32,
    pub yaw: f32,
    pub lamp_at: Option<u32>,
    pub crop: Option<[u32; 4]>,
    pub flicker: Option<[u32; 2]>,
    pub light: Option<LightMode>,
    pub lamp_power: f32,
    pub bake_radius: i32,
    pub light_radius: Option<i32>,
    pub probe_spacing: Option<u32>,
    pub sharpen: f32,
    pub lighting: Option<LightingResolution>,
    pub store_read: StoreRead,
    pub weights: LightWeights,
    pub jitter_phases: Option<u32>,
    pub dither: bool,
    /// `fold` or `drift`.
    pub read: Option<&'static str>,
    pub phases: Option<u32>,
    pub scale: RenderScale,
    pub shrink: u32,
    pub refs: Vec<PathBuf>,
    pub band_hi: Option<f32>,
    pub rings: Option<[u32; 4]>,
    pub pan: Option<f32>,
    pub dots: bool,
    pub drift_to: u32,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            help: false,
            reading: Reading::StripRun,
            adapter: None,
            tag: "strip".to_owned(),
            level: false,
            shot: None,
            camera: None,
            view: ViewMode::Shaded,
            size: [1920, 1080],
            upscaler: "fsr2",
            look: None,
            far: None,
            energy: None,
            lamp: None,
            indirect: true,
            filter: true,
            candidates: 8,
            shadow: ShadowWalk::Coarse,
            mean: false,
            exposure: None,
            settle: 120,
            frames: 240,
            every: 24,
            speed: 0.0,
            yaw: 0.0,
            lamp_at: None,
            crop: None,
            flicker: None,
            light: None,
            lamp_power: 4000.0,
            bake_radius: 12,
            light_radius: None,
            probe_spacing: None,
            sharpen: 0.5,
            lighting: None,
            store_read: StoreRead::Face,
            weights: LightWeights::Normal,
            jitter_phases: None,
            dither: false,
            read: None,
            phases: None,
            scale: RenderScale::OneHalf,
            shrink: 1,
            refs: Vec::new(),
            band_hi: None,
            rings: None,
            pan: None,
            dots: false,
            drift_to: 8192,
        }
    }
}

impl Args {
    /// The run frames the flicker map averages over: `--flicker`, else the whole run.
    pub fn flicker_window(&self) -> (u32, u32) {
        self.flicker.map_or((1, self.frames), |f| (f[0], f[1]))
    }
}

/// The arguments from `argv` (the program name left out).
pub fn parse(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args::default();
    table().parse_into(argv, &mut args)?;
    args.every = args.every.max(1);
    Ok(args)
}

/// A comma list of `N` numbers.
fn list<const N: usize>(v: &Val<'_>) -> Result<[f32; N], String> {
    let parts: Vec<f32> = v
        .text
        .split(',')
        .map(|p| {
            p.trim()
                .parse()
                .map_err(|_| v.takes(&format!("{N} numbers")))
        })
        .collect::<Result<_, _>>()?;
    parts
        .try_into()
        .map_err(|_| v.takes(&format!("{N} numbers")))
}

/// A crop `x,y,w,h`.
fn crop(v: &Val<'_>) -> Result<[u32; 4], String> {
    Ok(list::<4>(v)?.map(|c| c as u32))
}

/// A row whose value parses as `V`.
fn num<V: FromStr + 'static>(
    names: &'static [&'static str],
    value: &'static str,
    help: &'static str,
    set: fn(&mut Args, V),
) -> Flag<Args>
where
    V::Err: std::fmt::Display,
{
    Flag::value(names, value, help, move |a: &mut Args, v| {
        set(a, v.parse()?);
        Ok(())
    })
}

/// A row taking `on` or `off`.
fn on_off(
    names: &'static [&'static str],
    help: &'static str,
    set: fn(&mut Args, bool),
) -> Flag<Args> {
    Flag::value(names, "on|off", help, move |a: &mut Args, v| {
        set(a, v.on_off()?);
        Ok(())
    })
}

/// A row taking one of an enum's names.
fn named<V: 'static>(
    names: &'static [&'static str],
    value: &'static str,
    help: &'static str,
    from_name: fn(&str) -> Option<V>,
    set: fn(&mut Args, V),
) -> Flag<Args> {
    Flag::value(names, value, help, move |a: &mut Args, v| {
        set(a, v.named(from_name, &value.replace('|', ", "))?);
        Ok(())
    })
}

/// A row taking a crop `x,y,w,h`.
fn crop_row(
    names: &'static [&'static str],
    help: &'static str,
    set: fn(&mut Args, [u32; 4]),
) -> Flag<Args> {
    Flag::value(names, "X,Y,W,H", help, move |a: &mut Args, v| {
        set(a, crop(v)?);
        Ok(())
    })
}

fn run_flags() -> Vec<Flag<Args>> {
    vec![
        Flag::switch(
            &["--help", "-h"],
            "print this help and exit",
            |a: &mut Args| {
                a.help = true;
            },
        ),
        Flag::value(
            &["--reading"],
            "NAME",
            "the reading: strip-run (default; the strip, a fold, a pan or a drift) or hall-floor-terraces (the hall floor's terraces, no device)",
            |a: &mut Args, v| {
                a.reading = v.word(
                    &[
                        ("strip-run", Reading::StripRun),
                        ("hall-floor-terraces", Reading::HallFloorTerraces),
                    ],
                    "strip-run or hall-floor-terraces",
                )?;
                Ok(())
            },
        ),
        Flag::value(
            &["--adapter"],
            "SUBSTRING",
            "the first Vulkan adapter whose name contains it, ignoring case (default, or fallback: lavapipe)",
            |a: &mut Args, v| {
                a.adapter = Some(v.string());
                Ok(())
            },
        ),
        Flag::value(
            &["--tag"],
            "NAME",
            "the output directory's name under renders/strips/ (default strip)",
            |a: &mut Args, v| {
                a.tag = v.string();
                Ok(())
            },
        ),
        Flag::value(
            &["--source"],
            "showcase|level",
            "the world: showcase (default) or level (seed 1, the ore emissive)",
            |a: &mut Args, v| {
                a.level = v.word(&[("showcase", false), ("level", true)], "showcase or level")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--shot"],
            "NAME",
            "hall (default), chasm or crystals on the showcase; vein (default), patch or spawn (the fly-through's start, facing +x) on the level",
            |a: &mut Args, v| {
                a.shot = Some(v.string());
                Ok(())
            },
        ),
        Flag::value(
            &["--camera"],
            "X,Y,Z,YAW,PITCH",
            "voxels and degrees in place of the shot's camera; the shot still names the exposure",
            |a: &mut Args, v| {
                a.camera = Some(list::<5>(v)?);
                Ok(())
            },
        ),
        Flag::value(
            &["--size"],
            "WxH",
            "the output size (default 1920x1080)",
            |a: &mut Args, v| {
                let (w, h) = v.text.split_once('x').ok_or_else(|| v.takes("WxH"))?;
                let parse = |s: &str| s.parse::<u32>().map_err(|_| v.takes("WxH"));
                a.size = [parse(w)?, parse(h)?];
                Ok(())
            },
        ),
    ]
}

/// The run's frame counts, its camera's motion and what it writes.
fn motion_flags() -> Vec<Flag<Args>> {
    vec![
        num(
            &["--settle"],
            "N",
            "frames rendered still before the run (default 120)",
            |a, v| a.settle = v,
        ),
        num(
            &["--frames"],
            "N",
            "the run's frames (default 240)",
            |a, v| a.frames = v,
        ),
        num(
            &["--every"],
            "N",
            "a frame is written every N run frames, from frame 0 (default 24)",
            |a, v| a.every = v,
        ),
        num(
            &["--speed"],
            "V",
            "forward speed in voxels a second (default 0)",
            |a, v| a.speed = v,
        ),
        num(
            &["--yaw"],
            "DEG",
            "yaw rate in degrees a second, positive to the left (default 0)",
            |a, v| a.yaw = v,
        ),
        num(
            &["--lamp-at"],
            "F",
            "the run frame the lamp switches (default: never)",
            |a, v| a.lamp_at = Some(v),
        ),
        crop_row(
            &["--crop"],
            "a crop written beside every frame (default: none)",
            |a, v| a.crop = Some(v),
        ),
        Flag::value(
            &["--flicker"],
            "FROM,TO",
            "run frames the flicker map (the per-pixel mean |dY| between consecutive frames, times 8 as grey) averages over (default: the whole run)",
            |a: &mut Args, v| {
                a.flicker = Some(list::<2>(v)?.map(|f| f as u32));
                Ok(())
            },
        ),
        Flag::value(
            &["--energy"],
            "X,Y,W,H/...",
            "crops separated by /: the last frame's mean horizontal and vertical neighbour |dY| is printed for each",
            |a: &mut Args, v| {
                let crops = v
                    .text
                    .split('/')
                    .map(|c| {
                        crop(&Val {
                            flag: v.flag,
                            text: c,
                        })
                    })
                    .collect::<Result<_, _>>()?;
                a.energy = Some(crops);
                Ok(())
            },
        ),
        on_off(
            &["--mean"],
            "on writes mean.png (and its crop), the run's frames averaged (default off)",
            |a, v| a.mean = v,
        ),
    ]
}

fn frame_flags() -> Vec<Flag<Args>> {
    vec![
        named(
            &["--view"],
            "shaded|normals|albedo|hitmask|steps",
            "the view (default shaded)",
            ViewMode::from_name,
            |a, v| a.view = v,
        ),
        Flag::value(
            &["--upscaler"],
            "fsr2|own|off",
            "fsr2 (default) or own at --scale with sharpening --sharpen, or off at scale 1 with no jitter",
            |a: &mut Args, v| {
                a.upscaler = v.word(
                    &[("fsr2", "fsr2"), ("own", "own"), ("off", "off")],
                    "fsr2, own or off",
                )?;
                Ok(())
            },
        ),
        named(
            &["--scale"],
            "1|1.5|2",
            "the upscalers' internal scale (default 1.5)",
            RenderScale::from_name,
            |a, v| a.scale = v,
        ),
        num(
            &["--sharpen"],
            "S",
            "the upscaler's sharpening (default 0.5)",
            |a, v| a.sharpen = v,
        ),
        num(
            &["--shrink"],
            "K",
            "with --upscaler off, renders K times the size on each edge and box-filters every read frame down to the output size (default 1)",
            |a, v| a.shrink = v,
        ),
        num(
            &["--jitter-phases"],
            "N",
            "FSR 2's jitter phase count (the fold's default follows it)",
            |a, v| a.jitter_phases = Some(v),
        ),
        on_off(
            &["--dither"],
            "FSR 2's tone pass's static one-level dither (default off)",
            |a, v| a.dither = v,
        ),
        Flag::value(
            &["--look"],
            "PRESET",
            "a look preset (default: LookPreset::DEFAULT, far normal [2, 6]; face is the same cubes with the far normal off)",
            |a: &mut Args, v| {
                a.look = Some(v.named(LookPreset::from_name, &LookPreset::names())?);
                Ok(())
            },
        ),
        Flag::value(
            &["--far"],
            "LO,HI",
            "LookParams::far_normal (default: the preset's)",
            |a: &mut Args, v| {
                a.far = Some(list::<2>(v)?);
                Ok(())
            },
        ),
    ]
}

fn light_flags() -> Vec<Flag<Args>> {
    vec![
        Flag::value(
            &["--light"],
            "hero|eye|eye0|baked",
            "hero (the flags here), eye (the eye lamp at --lamp-power with the eye lighting: lamp on, no indirect, no emissive candidates, light radius 0), eye0 (eye with every emission scaled to 0) or baked (the hero's flags over the store, the chunks within --bake-radius baked before the settle); given as hero, eye or eye0 the probe spacing defaults to the hero's 32",
            |a: &mut Args, v| {
                a.light = Some(v.word(
                    &[
                        ("hero", LightMode::Hero),
                        ("eye", LightMode::Eye),
                        ("eye0", LightMode::Eye0),
                        ("baked", LightMode::Baked),
                    ],
                    "hero, eye, eye0 or baked",
                )?);
                Ok(())
            },
        ),
        num(
            &["--lamp-power"],
            "P",
            "the eye lamp's power (default 4000: the hall's dome crop at a mean display luma of 30 to 120 at exposure 8)",
            |a, v| a.lamp_power = v,
        ),
        num(
            &["--bake-radius"],
            "CHUNKS",
            "under --light baked, the chunks baked around the eye's (default 12)",
            |a, v| a.bake_radius = v,
        ),
        num(
            &["--light-radius"],
            "CHUNKS",
            "the mirror's light_radius (default: 0 under eye, else the source's)",
            |a, v| a.light_radius = Some(v),
        ),
        num(
            &["--probe-spacing"],
            "N",
            "the probe volume's spacing",
            |a, v| a.probe_spacing = Some(v),
        ),
        on_off(
            &["--lamp"],
            "the lamp at the start (default off on the showcase, on on the level)",
            |a, v| a.lamp = Some(v),
        ),
        on_off(&["--indirect"], "the lamp's bounce (default on)", |a, v| {
            a.indirect = v
        }),
        on_off(&["--filter"], "the light filter (default on)", |a, v| {
            a.filter = v
        }),
        num(
            &["--candidates"],
            "N",
            "emissive candidates per pixel (default 8)",
            |a, v| a.candidates = v,
        ),
        named(
            &["--shadow"],
            "coarse|full",
            "the shadow walk (default coarse)",
            ShadowWalk::from_name,
            |a, v| a.shadow = v,
        ),
        Flag::value(
            &["--exposure"],
            "auto|E",
            "a number, or auto (default: the shot's or pose's manual value)",
            |a: &mut Args, v| {
                a.exposure = Some(if v.text == "auto" {
                    Exposure::Auto(AutoExposure::eye())
                } else {
                    Exposure::Manual(v.parse().map_err(|_| v.takes("a number or auto"))?)
                });
                Ok(())
            },
        ),
        named(
            &["--lighting"],
            "full|half",
            "the light passes' resolution (default: the frame's, half)",
            LightingResolution::from_name,
            |a, v| a.lighting = Some(v),
        ),
        named(
            &["--store-read"],
            "face|normal",
            "Lighting::store_read, ADR-0049 (default face)",
            StoreRead::from_name,
            |a, v| a.store_read = v,
        ),
        named(
            &["--weights"],
            "normal|face",
            "Lighting::light_weights (default normal)",
            LightWeights::from_name,
            |a, v| a.weights = v,
        ),
    ]
}

fn read_flags() -> Vec<Flag<Args>> {
    vec![
        Flag::value(
            &["--read"],
            "fold|drift",
            "after the settle: fold (--frames still frames folded over --phases: T, L, J, the guard's energy, the jumps by class, the far crops and their maps) or drift (step settle + 1 against step --drift-to)",
            |a: &mut Args, v| {
                a.read = Some(v.word(&[("fold", "fold"), ("drift", "drift")], "fold or drift")?);
                Ok(())
            },
        ),
        num(
            &["--phases"],
            "N",
            "the fold's phases (default: FSR 2's jitter phases, own's JITTER_PHASES, 18 for off)",
            |a, v| a.phases = Some(v),
        ),
        Flag::value(
            &["--ref"],
            "PATH,...",
            "absolute paths to PNGs of the output size: a fold prints its last frame's distance from each, whole, on the far crops and by class",
            |a: &mut Args, v| {
                a.refs = v
                    .text
                    .split(',')
                    .filter(|p| !p.is_empty())
                    .map(PathBuf::from)
                    .collect();
                Ok(())
            },
        ),
        num(
            &["--band-hi"],
            "PX",
            "the far normal's band start in internal pixels for the fold's direction read (default: the look's far_normal[1])",
            |a, v| a.band_hi = Some(v),
        ),
        crop_row(
            &["--rings"],
            "a fold reads the rings on that crop of its last frame and writes zlevel.png, rings-zlevel.png and rings-crop.png",
            |a, v| a.rings = Some(v),
        ),
        num(
            &["--pan"],
            "PX_PER_FRAME",
            "after the settle, --frames frames of a pure yaw to the left at that many display pixels a frame, C read in the centre band with its jumps by class",
            |a, v| a.pan = Some(v),
        ),
        on_off(
            &["--dots"],
            "a pan adds the dot and yellow-dot count and their anchoring (default off)",
            |a, v| a.dots = v,
        ),
        num(
            &["--drift-to"],
            "N",
            "the step --read drift compares against (default 8192)",
            |a, v| a.drift_to = v,
        ),
    ]
}

/// The strip's flag table.
pub fn table() -> Table<Args> {
    Table {
        usage: "strip [FLAGS]: a reading-only strip tool (see the example's doc comment)",
        sections: vec![
            Section {
                title: "Run",
                flags: run_flags(),
            },
            Section {
                title: "Motion and output",
                flags: motion_flags(),
            },
            Section {
                title: "Frame",
                flags: frame_flags(),
            },
            Section {
                title: "Light",
                flags: light_flags(),
            },
            Section {
                title: "Reads",
                flags: read_flags(),
            },
        ],
    }
}
