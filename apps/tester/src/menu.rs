//! The launcher's menu: four choices (scene, mode, output, internal scale) and five actions,
//! driven by the arrow keys and Enter, or a number key that selects and activates its row. The
//! menu holds no window; [`Menu::key`] turns a key into a [`Command`] and [`Menu::write`] lays
//! the rows out as the text overlay draws them.

use flythrough::path::{PathMode, Scene};
use vxe_app::KeyCode;
use vxe_render::HudColour;

use crate::run::{Output, RunSpec, Scale};

/// What a menu key asks the launcher to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Run the chosen scene in the chosen mode.
    Run(RunSpec),
    /// Run every scene's benchmark in order at the chosen quality.
    RunAll(Output, Scale),
    SaveReport,
    OpenFolder,
    Quit,
}

/// The menu's rows, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Scene,
    Mode,
    Output,
    Scale,
    Run,
    RunAll,
    Save,
    Open,
    Quit,
}

const ROWS: [Row; 9] = [
    Row::Scene,
    Row::Mode,
    Row::Output,
    Row::Scale,
    Row::Run,
    Row::RunAll,
    Row::Save,
    Row::Open,
    Row::Quit,
];

const MODES: [PathMode; 2] = [PathMode::Bench, PathMode::Tour];

/// The menu's state: the selected row and each choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Menu {
    selected: usize,
    pub scene: Scene,
    pub mode: PathMode,
    pub output: Output,
    pub scale: Scale,
}

impl Default for Menu {
    /// The showcase, benchmark, 1280 x 720 at the fly-through's default scale of 1.5; the run
    /// row selected.
    fn default() -> Self {
        Self {
            selected: 4,
            scene: Scene::Showcase,
            mode: PathMode::Bench,
            output: Output::Hd720,
            scale: Scale::OneHalf,
        }
    }
}

/// The element after (or before, `back`) `v` in `all`, wrapping.
fn cycle<T: Copy + PartialEq>(all: &[T], v: T, back: bool) -> T {
    let i = all.iter().position(|&x| x == v).unwrap_or(0);
    let n = all.len();
    all[if back { (i + n - 1) % n } else { (i + 1) % n }]
}

impl Menu {
    /// The run the choices make.
    pub fn spec(&self) -> RunSpec {
        RunSpec {
            scene: self.scene,
            mode: self.mode,
            output: self.output,
            scale: self.scale,
        }
    }

    /// Takes a pressed key: `Up`/`Down` move the selection, `Left`/`Right` change a choice,
    /// `Enter` changes a choice or activates an action, `1` to `9` select a row and do as
    /// `Enter`, `Esc` quits.
    pub fn key(&mut self, key: KeyCode) -> Option<Command> {
        let n = ROWS.len();
        match key {
            KeyCode::ArrowUp => self.selected = (self.selected + n - 1) % n,
            KeyCode::ArrowDown => self.selected = (self.selected + 1) % n,
            KeyCode::ArrowLeft => self.change(true),
            KeyCode::ArrowRight => self.change(false),
            KeyCode::Enter | KeyCode::NumpadEnter => return self.activate(),
            KeyCode::Escape => return Some(Command::Quit),
            k => {
                let digits = [
                    KeyCode::Digit1,
                    KeyCode::Digit2,
                    KeyCode::Digit3,
                    KeyCode::Digit4,
                    KeyCode::Digit5,
                    KeyCode::Digit6,
                    KeyCode::Digit7,
                    KeyCode::Digit8,
                    KeyCode::Digit9,
                ];
                if let Some(i) = digits.iter().position(|&d| d == k) {
                    self.selected = i;
                    return self.activate();
                }
            }
        }
        None
    }

    fn change(&mut self, back: bool) {
        match ROWS[self.selected] {
            Row::Scene => self.scene = cycle(&Scene::ALL, self.scene, back),
            Row::Mode => self.mode = cycle(&MODES, self.mode, back),
            Row::Output => self.output = cycle(&Output::ALL, self.output, back),
            Row::Scale => self.scale = cycle(&Scale::ALL, self.scale, back),
            _ => {}
        }
    }

    fn activate(&mut self) -> Option<Command> {
        match ROWS[self.selected] {
            Row::Scene | Row::Mode | Row::Output | Row::Scale => {
                self.change(false);
                None
            }
            Row::Run => Some(Command::Run(self.spec())),
            Row::RunAll => Some(Command::RunAll(self.output, self.scale)),
            Row::Save => Some(Command::SaveReport),
            Row::Open => Some(Command::OpenFolder),
            Row::Quit => Some(Command::Quit),
        }
    }

    /// Appends the menu's rows to `text`, one a line, with their colours: the selected row
    /// yellow.
    pub fn write(&self, text: &mut String, colours: &mut Vec<HudColour>) {
        for (i, row) in ROWS.iter().enumerate() {
            let label = match row {
                Row::Scene => format!("scene    < {} >", self.scene.name()),
                Row::Mode => format!(
                    "mode     < {} >",
                    match self.mode {
                        PathMode::Bench => "benchmark",
                        PathMode::Tour => "tour",
                    }
                ),
                Row::Output => format!("output   < {} >", self.output.name()),
                Row::Scale => format!("internal scale < {} >", self.scale.name()),
                Row::Run => "run".to_owned(),
                Row::RunAll => "run all benchmarks".to_owned(),
                Row::Save => "save report".to_owned(),
                Row::Open => "open report folder".to_owned(),
                Row::Quit => "quit".to_owned(),
            };
            let mark = if i == self.selected { '>' } else { ' ' };
            text.push_str(&format!("{mark} {} {label}\n", i + 1));
            colours.push(if i == self.selected {
                HudColour::Yellow
            } else {
                HudColour::White
            });
        }
    }
}
