//! The fly-through's menu (ADR-0061): one view over the key tables ([`entries`]), grouped
//! into pages. The toggles ([`crate::toggles::TOGGLES`]) and the actions
//! ([`crate::toggles::ACTIONS`]) are its entries; the fixed keys ([`FIXED_KEYS`]) stay bound
//! and are not entries. `flythrough --keys` prints the tree ([`write_tree`]).

mod entries;
mod filter;
mod page;
mod route;
mod write;

pub use entries::*;
pub use filter::*;
pub use page::*;
pub use route::*;
pub use write::*;

use vxe_app::{KeyCode, PadButton};

/// A page of the menu: the entries of one group, in the order the root page lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Moving,
    Camera,
    World,
    Look,
    Upscaler,
    Lighting,
    Lights,
    Queue,
    Entities,
    Audio,
    Readings,
    Debug,
}

impl Group {
    /// The groups in the root page's order.
    pub const ALL: [Self; 12] = [
        Self::Moving,
        Self::Camera,
        Self::World,
        Self::Look,
        Self::Upscaler,
        Self::Lighting,
        Self::Lights,
        Self::Queue,
        Self::Entities,
        Self::Audio,
        Self::Readings,
        Self::Debug,
    ];

    /// The group's name as its page title and the palette's prefix show it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Moving => "moving",
            Self::Camera => "camera and paths",
            Self::World => "world and sky",
            Self::Look => "look",
            Self::Upscaler => "upscaler",
            Self::Lighting => "lighting",
            Self::Lights => "lights",
            Self::Queue => "light queue",
            Self::Entities => "entities",
            Self::Audio => "audio",
            Self::Readings => "readings",
            Self::Debug => "debug",
        }
    }
}

/// The keys that stay bound and are no entry: movement and look's keys, the menu, the Keys
/// page, the stats overlay and the quit. The fly-through reads them by hand (`fly`,
/// `run::frame`, [`route`]).
pub const FIXED_KEYS: &[KeyCode] = &[
    KeyCode::KeyW,
    KeyCode::KeyA,
    KeyCode::KeyS,
    KeyCode::KeyD,
    KeyCode::Space,
    KeyCode::KeyC,
    KeyCode::ShiftLeft,
    KeyCode::Tab,
    KeyCode::F1,
    KeyCode::F2,
    KeyCode::Escape,
];

/// The fixed binds in words, the Keys page's header block and `--keys`' first group: the
/// binds, then what they do.
pub const FIXED_ROWS: &[(&str, &str)] = &[
    ("W A S D", "move"),
    ("Space C", "up and down"),
    ("Shift", "faster"),
    ("mouse", "look"),
    ("left click / R2", "dig; in placement mode, place the ghost"),
    ("right click / L2", "fill; in placement mode, leave it"),
    ("wheel", "the belt's slot, into placement mode"),
    ("Tab / pad Start", "the menu; type on any page to search"),
    ("F1", "this page, the keys"),
    ("F2 / pad Select", "the stats overlay"),
    ("Esc", "back, close; placing, leave the mode; else quit"),
];

/// The few run flags an entry reads beside the modes: whether the body walks, the audio's
/// switches and mix when the run has audio, and what the run has for the entries that need
/// it (`toggles::Needs`): a camera path, the castle's shots, a model in the viewer, a streamed
/// world, the coarse grid.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RunView {
    pub walking: bool,
    pub audio: Option<AudioView>,
    pub path: bool,
    pub castle: bool,
    pub model: bool,
    /// Whether an entity is placed: the clip entries act on it.
    pub placed: bool,
    pub streaming: bool,
    pub coarse: bool,
}

/// The audio's switches and mix as the audio entries show them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioView {
    pub echoes: bool,
    pub tail: bool,
    pub echo_db: f32,
    pub tail_db: f32,
    pub predelay_ms: f32,
    pub glide: bool,
    pub direct_delay: bool,
    /// The trims' ranges in dB, low and high.
    pub echo_range: (f32, f32),
    pub tail_range: (f32, f32),
}

impl AudioView {
    /// The view of `mix` with the echoes and the tail on or off.
    pub(crate) fn of(mix: &crate::audio::Mix, echoes: bool, tail: bool) -> Self {
        let (e, t) = (mix.echo_range(), mix.tail_range());
        Self {
            echoes,
            tail,
            echo_db: mix.echo_db,
            tail_db: mix.tail_db,
            predelay_ms: mix.predelay_ms,
            glide: mix.glide,
            direct_delay: mix.direct_delay,
            echo_range: (*e.start(), *e.end()),
            tail_range: (*t.start(), *t.end()),
        }
    }
}

/// A key's name as the menu shows it: `F7`, `A`, `1`, `,`, `Backspace`, `Left`. Read from the
/// key's `Debug` name, so this names no key of its own.
pub fn key_name(key: KeyCode) -> String {
    let debug = format!("{key:?}");
    let short = match debug.as_str() {
        "Comma" => ",",
        "Period" => ".",
        "Semicolon" => ";",
        "Quote" => "'",
        "Slash" => "/",
        "Backslash" => "\\",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Minus" => "-",
        "Equal" => "=",
        "Backquote" => "`",
        "Escape" => "Esc",
        "ShiftLeft" => "Shift",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        d => {
            let d = d.strip_prefix("Key").unwrap_or(d);
            return d.strip_prefix("Digit").unwrap_or(d).to_owned();
        }
    };
    short.to_owned()
}

/// A pad button's name as the menu shows it: `pad West`, `pad D-pad up`.
pub fn pad_name(button: PadButton) -> String {
    let debug = format!("{button:?}");
    match debug.strip_prefix("Dpad") {
        Some(dir) => format!("pad D-pad {}", dir.to_lowercase()),
        None => format!("pad {debug}"),
    }
}
