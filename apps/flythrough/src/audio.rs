//! The fly-through's audio: the output, the mix and its trims, the microphone and the room
//! probe, and the sounds it plays.

use crate::app::ms_since;
use crate::cave_materials;
use crate::now;
use std::time::Instant;
use vxe_audio::{
    Acoustics, BlockClock, Command, Hrtf, MixSettings, Mixer, Output, PlayParams, ReverbSettings,
    RoomEstimate, RoomTargets, Sound, TailLines, VoiceId,
};
use vxe_world::MaterialTable;
use vxe_world::hash::SplitMix64;

/// Metres per voxel, for the audio mixer's distances.
pub(crate) const METRES_PER_VOXEL: f32 = 0.1;

/// The knock: 50 ms of seeded noise under an exponential decay of 10 ms time constant.
pub(crate) const KNOCK_SEED: u64 = 0x0006_0001;

pub(crate) const KNOCK_SECONDS: f32 = 0.05;

pub(crate) const KNOCK_DECAY_SECONDS: f32 = 0.01;

/// The hums `J` plays in turn, one note per press so each source can be told apart by ear:
/// an A minor pentatonic from A3 to D5, rounded to whole hertz so one second holds a whole
/// number of cycles and loops cleanly.
pub(crate) const HUM_HZ: [u32; 8] = [220, 262, 294, 330, 392, 440, 523, 587];

pub(crate) const HUM_AMPLITUDE: f32 = 0.3;

/// The acoustic triple (absorption per metre, transmission, resonance) the probes give every
/// solid cave row; the cave's own table leaves it zero, which would block all sound.
pub(crate) const CAVE_ACOUSTIC: [f32; 3] = [0.3, 0.5, 0.0];

/// SplitMix64 noise in `[-1, 1)` under an exponential decay: the knock `H` plays.
pub(crate) fn knock() -> Sound {
    Sound::new(knock_samples())
}

/// The `T` pulse: one knock at the start of each second, silence after it, so a looping pulse
/// leaves gaps in which the echoes off far walls can be heard on their own.
pub(crate) fn pulse() -> Sound {
    let mut samples = knock_samples();
    samples.resize(vxe_audio::RATE as usize, 0.0);
    Sound::new(samples)
}

/// The knock's samples.
pub(crate) fn knock_samples() -> Vec<f32> {
    let mut rng = SplitMix64(KNOCK_SEED);
    let len = (KNOCK_SECONDS * vxe_audio::RATE as f32) as usize;
    let samples: Vec<f32> = (0..len)
        .map(|n| {
            let noise = (rng.next_u64() >> 40) as f32 / (1u64 << 23) as f32 - 1.0;
            let t = n as f32 / vxe_audio::RATE as f32;
            0.8 * noise * (-t / KNOCK_DECAY_SECONDS).exp()
        })
        .collect();
    samples
}

/// The cave's material table with [`CAVE_ACOUSTIC`] on every row but air, its absorption
/// replaced by `absorption` (`--absorption`).
pub(crate) fn acoustic_table(absorption: f32) -> MaterialTable {
    let mut t = MaterialTable::new();
    for row in cave_materials().rows().iter().skip(1) {
        t.push(vxe_world::Material {
            acoustic: [absorption, CAVE_ACOUSTIC[1], CAVE_ACOUSTIC[2]],
            ..row.clone()
        });
    }
    t
}

/// The room line: `room: V <m3> S <m2> mfp <m> alpha <a> T60 <s> energy <dB>`.
pub(crate) fn room_line(r: &RoomEstimate) -> String {
    format!(
        "room: V {:.1} m3 S {:.1} m2 mfp {:.3} m alpha {:.4} T60 {:.3} s energy {:+.1} dB",
        r.volume_m3,
        r.surface_m2,
        r.mean_free_path_m,
        r.absorption,
        r.t60_s,
        10.0 * r.energy.max(1e-12).log10()
    )
}

/// `--audio-debug`'s line for each room sent to the mixer:
/// `room sent: T60 <s> mfp <m> energy <dB>`.
pub(crate) fn room_sent_line(r: &RoomTargets) -> String {
    format!(
        "room sent: T60 {:.3} s mfp {:.3} m energy {:+.1} dB",
        r.t60_s,
        r.mean_free_path_m,
        10.0 * r.energy.max(1e-12).log10()
    )
}

/// The balance the keys and flags set over the acoustics: trims over the mixer's room curves
/// (or, with `--flat-mix`, over flat curves), and the room the curves read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Mix {
    /// The echo trim, dB.
    pub(crate) echo_db: f32,
    /// The tail trim, dB.
    pub(crate) tail_db: f32,
    /// The tail's pre-delay, milliseconds.
    pub(crate) predelay_ms: f32,
    /// The tail's lines glide instead of handing over.
    pub(crate) glide: bool,
    /// The direct path's propagation delay.
    pub(crate) direct_delay: bool,
    /// `--flat-mix`: the trims over [`MixSettings::flat`] instead of the room curves.
    pub(crate) flat: bool,
    /// The mean free path of the latest room sent, metres.
    pub(crate) room: Option<f32>,
    /// The room the last printed curve values were read at.
    pub(crate) printed_room: Option<f32>,
}

impl Mix {
    /// The trims' range over the room curves.
    pub(crate) const TRIM_DB: std::ops::RangeInclusive<f32> = -24.0..=12.0;
    /// With `--flat-mix` the absolute ranges, so a flat balance replays as it was heard.
    pub(crate) const FLAT_ECHO_DB: std::ops::RangeInclusive<f32> = -12.0..=30.0;
    pub(crate) const FLAT_TAIL_DB: std::ops::RangeInclusive<f32> = -30.0..=6.0;
    pub(crate) const PREDELAY_MS: std::ops::RangeInclusive<f32> = 0.0..=200.0;
    /// A room that moves either curve by this much prints the mix line again.
    pub(crate) const REPRINT_DB: f32 = 0.5;

    pub(crate) fn echo_range(&self) -> std::ops::RangeInclusive<f32> {
        if self.flat {
            Self::FLAT_ECHO_DB
        } else {
            Self::TRIM_DB
        }
    }

    pub(crate) fn tail_range(&self) -> std::ops::RangeInclusive<f32> {
        if self.flat {
            Self::FLAT_TAIL_DB
        } else {
            Self::TRIM_DB
        }
    }

    /// `flag`'s value `v` as a finite number; [`Mix::check`] holds it to its range once every
    /// flag is read.
    pub(crate) fn parse(flag: &str, v: &str) -> Result<f32, String> {
        v.parse::<f32>()
            .ok()
            .filter(|x| x.is_finite())
            .ok_or_else(|| format!("{flag} takes a number, not {v}"))
    }

    /// Each flag's value within its range for this mix's mode.
    pub(crate) fn check(&self) -> Result<(), String> {
        for (flag, v, r) in [
            ("--echo-db", self.echo_db, self.echo_range()),
            ("--tail-db", self.tail_db, self.tail_range()),
            ("--predelay-ms", self.predelay_ms, Self::PREDELAY_MS),
        ] {
            if !r.contains(&v) {
                return Err(format!(
                    "{flag} takes {} to {}, not {v}",
                    r.start(),
                    r.end()
                ));
            }
        }
        Ok(())
    }

    /// Each value moved by its key's step and held in its range.
    pub(crate) fn step(self, echo_db: f32, tail_db: f32, predelay_ms: f32) -> Self {
        let clamp = |x: f32, r: std::ops::RangeInclusive<f32>| x.clamp(*r.start(), *r.end());
        Self {
            echo_db: clamp(self.echo_db + echo_db, self.echo_range()),
            tail_db: clamp(self.tail_db + tail_db, self.tail_range()),
            predelay_ms: clamp(self.predelay_ms + predelay_ms, Self::PREDELAY_MS),
            ..self
        }
    }

    pub(crate) fn settings(&self) -> MixSettings {
        let base = if self.flat {
            MixSettings::flat()
        } else {
            MixSettings::default()
        };
        MixSettings {
            echo_gain: 10f32.powf(self.echo_db / 20.0),
            direct_delay: self.direct_delay,
            reverb: ReverbSettings {
                level: 10f32.powf(self.tail_db / 20.0),
                predelay_s: self.predelay_ms / 1e3,
                lines: if self.glide {
                    TailLines::Glide
                } else {
                    TailLines::Handover
                },
                ..base.reverb
            },
            ..base
        }
    }

    /// The echo and tail curves' values at `room` (no room reads the curves' last knot).
    pub(crate) fn curve_db(&self, room: Option<f32>) -> (f32, f32) {
        let s = self.settings();
        let mfp = room.unwrap_or(f32::NAN);
        (s.echo_by_room.db_at(mfp), s.reverb.level_by_room.db_at(mfp))
    }

    /// Takes the mean free path of a room sent to the mixer; true when either curve moved by
    /// [`Mix::REPRINT_DB`] or more since the last printed line (which this then becomes).
    pub(crate) fn take_room(&mut self, mfp: f32) -> bool {
        self.room = Some(mfp);
        if self.flat {
            return false;
        }
        let (e0, t0) = self.curve_db(self.printed_room);
        let (e1, t1) = self.curve_db(self.room);
        let moved = (e1 - e0).abs() >= Self::REPRINT_DB || (t1 - t0).abs() >= Self::REPRINT_DB;
        if moved {
            self.printed_room = self.room;
        }
        moved
    }

    /// `mix: echoes <trim> dB (room <curve> dB at mfp <m> m), tail <trim> dB (room <curve> dB),
    /// pre-delay <ms>, tail lines <mode>, direct delay <on|off>`; `room flat` with
    /// `--flat-mix`, and `before any room` until a room arrives.
    pub(crate) fn line(&self) -> String {
        let (echo_room, tail_room) = if self.flat {
            ("room flat".to_owned(), "room flat".to_owned())
        } else {
            let (e, t) = self.curve_db(self.room);
            let at = self.room.map_or_else(
                || "before any room".to_owned(),
                |m| format!("at mfp {m:.3} m"),
            );
            (format!("room {e:+.1} dB {at}"), format!("room {t:+.1} dB"))
        };
        format!(
            "mix: echoes {:+.0} dB ({echo_room}), tail {:+.0} dB ({tail_room}), pre-delay {:.0} \
             ms, tail lines {}, direct delay {}",
            self.echo_db,
            self.tail_db,
            self.predelay_ms,
            if self.glide { "glide" } else { "handover" },
            if self.direct_delay { "on" } else { "off" }
        )
    }
}

/// The voice the microphone plays as. The output numbers its voices up from 0, so this one
/// is never handed out in a session.
pub(crate) const MIC_VOICE: VoiceId = VoiceId(u32::MAX);

/// `--mic`: the input stream and whether its period has been printed.
pub(crate) struct Mic {
    pub(crate) input: vxe_audio::Input,
    pub(crate) period_printed: bool,
    pub(crate) stream_errors: u64,
}

/// `--room-line` without an output device: the room fan on a block clock that never
/// advances, its commands dropped.
pub(crate) struct RoomProbe {
    pub(crate) acoustics: Acoustics,
    pub(crate) table: MaterialTable,
    /// The balance a mixer would take, for the mix line's room curves.
    pub(crate) mix: Mix,
}

impl RoomProbe {
    pub(crate) fn new(absorption: f32, mix: Mix) -> Self {
        Self {
            acoustics: Acoustics::new(BlockClock::default()),
            table: acoustic_table(absorption),
            mix,
        }
    }
}

/// One second of a `J` hum at `hz`.
pub(crate) fn hum(hz: u32) -> Sound {
    let rate = vxe_audio::RATE as f32;
    Sound::new(
        (0..vxe_audio::RATE)
            .map(|n| HUM_AMPLITUDE * (std::f32::consts::TAU * hz as f32 * n as f32 / rate).sin())
            .collect::<Vec<_>>(),
    )
}

/// The audio of `--audio`: the output stream, the sounds the keys play and the acoustics
/// probing them.
pub(crate) struct Audio {
    pub(crate) output: Output,
    pub(crate) knock: Sound,
    /// The looping knock `T` plays.
    pub(crate) pulse: Sound,
    /// One hum per [`HUM_HZ`] note.
    pub(crate) hums: Vec<Sound>,
    /// The note the next `J` plays, an index into [`HUM_HZ`].
    pub(crate) next_hum: usize,
    /// Where the last left click dug, in voxel units.
    pub(crate) dig_point: Option<[f32; 3]>,
    pub(crate) acoustics: Acoustics,
    /// The cave's rows with [`CAVE_ACOUSTIC`] on every solid one.
    pub(crate) table: MaterialTable,
    /// The voices playing and where, in voxel units.
    pub(crate) voices: Vec<(VoiceId, [f32; 3])>,
    /// Stream errors (underruns among them) already reported.
    pub(crate) stream_errors: u64,
    /// `--audio-debug`: print the callback's readings each second.
    pub(crate) debug: bool,
    /// When the callback's readings were last taken.
    pub(crate) stats_at: Instant,
    /// The longest callback (the mixer's render of one device buffer, timed inside the
    /// callback) over the last whole second, in milliseconds.
    pub(crate) callback_max_ms: f32,
    /// Wall time of the last `Acoustics::update`, in milliseconds.
    pub(crate) update_ms: f64,
    /// `--room-line`: print the room line whenever a fan completes.
    pub(crate) room_line: bool,
    pub(crate) mic: Option<Mic>,
    /// Fans discarded in a row so far (the listener inside a solid), for `--audio-debug`.
    pub(crate) discarded_run: u64,
    /// The balance sent to the mixer.
    pub(crate) mix: Mix,
}

/// An environment variable naming an HRIR pack to use instead of the shipped one, for
/// listening comparisons.
pub(crate) const HRIR_ENV: &str = "FLYTHROUGH_HRIR";

/// Seconds of output `--audio-debug` keeps, and where it writes them.
pub(crate) const AUDIO_DEBUG_SECONDS: f32 = 60.0;

pub(crate) const AUDIO_DEBUG_WAV: &str = "flythrough-audio-debug.wav";

impl Drop for Audio {
    fn drop(&mut self) {
        if !self.debug {
            return;
        }
        let samples = self.output.take_recording();
        match std::fs::write(AUDIO_DEBUG_WAV, wav_f32_stereo(&samples)) {
            Ok(()) => println!(
                "audio-debug: wrote {:.1} s of the device's output to {AUDIO_DEBUG_WAV}",
                samples.len() as f32 / 2.0 / vxe_audio::RATE as f32
            ),
            Err(e) => println!("audio-debug: {AUDIO_DEBUG_WAV}: {e}"),
        }
    }
}

/// A 32-bit float stereo WAV file of interleaved `samples` at the mixer's rate.
pub(crate) fn wav_f32_stereo(samples: &[f32]) -> Vec<u8> {
    let data = (samples.len() * 4) as u32;
    let rate = vxe_audio::RATE;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 8).to_le_bytes());
    b.extend_from_slice(&8u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for &x in samples {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b
}

impl Audio {
    /// Plays the sound `name` names (`knock`, `hum` or `pulse`) at the dig point with `params`.
    pub(crate) fn play(&mut self, name: &str, params: impl Fn([f32; 3]) -> PlayParams) {
        let Some(p) = self.dig_point else {
            println!("{name}: no dig point yet; left-click first");
            return;
        };
        let sound = if name == "hum" {
            let i = self.next_hum;
            self.next_hum = (i + 1) % self.hums.len();
            println!(
                "hum: note {} of {}, {} Hz, at [{:.1}, {:.1}, {:.1}]",
                i + 1,
                self.hums.len(),
                HUM_HZ[i],
                p[0],
                p[1],
                p[2]
            );
            &self.hums[i]
        } else if name == "pulse" {
            &self.pulse
        } else {
            &self.knock
        };
        match self.output.play(sound, params(p)) {
            Ok(id) => self.voices.push((id, p)),
            Err(e) => println!("{name}: {e}"),
        }
    }

    /// Opens the default device; prints why when it cannot. `debug` meters and records the
    /// callback; `absorption` is the cave row's; `mic` also opens the default input device and
    /// plays it at the listener with its direct path off; `room_line` prints each room.
    pub(crate) fn open(
        debug: bool,
        absorption: f32,
        mic: bool,
        room_line: bool,
        mix: Mix,
    ) -> Option<Self> {
        let t = now();
        let hrtf = match std::env::var_os(HRIR_ENV) {
            Some(path) => match std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| Hrtf::load(&b).map_err(|e| e.to_string()))
            {
                Ok(h) => {
                    println!(
                        "audio: HRIR pack from {HRIR_ENV}={}",
                        path.to_string_lossy()
                    );
                    std::sync::Arc::new(h)
                }
                Err(e) => {
                    println!(
                        "audio: {HRIR_ENV}={}: {e}; using the shipped pack",
                        path.to_string_lossy()
                    );
                    Hrtf::sadie2_d2()
                }
            },
            None => Hrtf::sadie2_d2(),
        };
        println!(
            "audio: HRIR pack loaded in {:.1} ms ({} directions, {} taps)",
            ms_since(t),
            hrtf.len(),
            hrtf.taps()
        );
        let mut mixer = Mixer::new(hrtf, METRES_PER_VOXEL);
        mixer.set_mix(mix.settings());
        let acoustics = Acoustics::new(mixer.clock());
        match Output::open(mixer) {
            Ok(output) => {
                println!("audio: {}", output.device());
                if debug {
                    output.record(AUDIO_DEBUG_SECONDS);
                }
                let mic = if mic { open_mic(&output) } else { None };
                let mut voices = Vec::new();
                if mic.is_some() {
                    voices.push((MIC_VOICE, [0.0; 3]));
                }
                Some(Self {
                    output,
                    knock: knock(),
                    pulse: pulse(),
                    hums: HUM_HZ.iter().map(|&hz| hum(hz)).collect(),
                    next_hum: 0,
                    dig_point: None,
                    acoustics,
                    table: acoustic_table(absorption),
                    voices,
                    stream_errors: 0,
                    debug,
                    stats_at: now(),
                    callback_max_ms: 0.0,
                    update_ms: 0.0,
                    room_line,
                    mic,
                    discarded_run: 0,
                    mix,
                })
            }
            Err(e) => {
                println!("audio: off, {e}");
                None
            }
        }
    }
}

/// Opens the default input device into a stream played at the listener with its direct path
/// off; prints the device, or why it could not.
pub(crate) fn open_mic(output: &Output) -> Option<Mic> {
    println!("mic: wear headphones: from speakers the microphone hears the tail and feeds it back");
    let (writer, reader) = vxe_audio::Stream::new();
    match vxe_audio::Input::open(writer) {
        Ok(input) => {
            println!("mic: {}", input.device());
            let params = PlayParams {
                direct: false,
                ..PlayParams::at([0.0; 3])
            };
            if let Err(e) = output.send(Command::PlayStream(MIC_VOICE, reader, params)) {
                println!("mic: off, {e}");
                return None;
            }
            Some(Mic {
                input,
                period_printed: false,
                stream_errors: 0,
            })
        }
        Err(e) => {
            println!("mic: off, {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use vxe_audio::{MixSettings, TailLines};

    use crate::cli::parse_args_from;
    use crate::test_support::argv;

    /// The balance flags parse within their ranges, set the mixer's settings and print the
    /// mix line; the keys' steps stop at the ranges' ends (the trims at -24 and +12 dB over
    /// the room curves, the absolute ranges under `--flat-mix`).
    #[test]
    fn the_balance_flags_and_steps() {
        let a = parse_args_from(argv(&[
            "--room",
            "12",
            "--echo-db",
            "6",
            "--tail-db",
            "-9",
            "--predelay-ms",
            "30",
            "--tail-glide",
            "--direct-delay",
        ]))
        .expect("parses");
        assert_eq!(
            a.audio.mix.line(),
            "mix: echoes +6 dB (room +6.0 dB before any room), tail -9 dB (room +0.0 dB), \
             pre-delay 30 ms, tail lines glide, direct delay on"
        );
        let s = a.audio.mix.settings();
        assert!((s.echo_gain - 1.9953).abs() < 1e-4);
        assert!((s.reverb.level - 0.3548).abs() < 1e-4);
        assert_eq!(s.reverb.predelay_frames(), 1_440);
        assert_eq!(s.reverb.lines, TailLines::Glide);
        assert!(s.direct_delay);
        assert_eq!(s.echo_by_room, MixSettings::default().echo_by_room);
        assert_eq!(Mix::default().settings(), MixSettings::default());

        let flat = parse_args_from(argv(&["--flat-mix", "--echo-db", "9"])).expect("parses");
        assert_eq!(
            flat.audio.mix.line(),
            "mix: echoes +9 dB (room flat), tail +0 dB (room flat), pre-delay 0 ms, tail lines \
             handover, direct delay off"
        );
        assert_eq!(
            flat.audio.mix.settings(),
            MixSettings {
                echo_gain: 10f32.powf(9.0 / 20.0),
                ..MixSettings::flat()
            }
        );
        // The Phase 12 picks replay under --flat-mix; over the curves the trims stop at +12.
        assert!(parse_args_from(argv(&["--flat-mix", "--echo-db", "30"])).is_ok());
        assert!(parse_args_from(argv(&["--echo-db", "30", "--flat-mix"])).is_ok());
        for bad in [
            &["--echo-db", "13"][..],
            &["--tail-db", "-25"],
            &["--echo-db", "-25"],
            &["--tail-db", "13"],
            &["--flat-mix", "--echo-db", "31"],
            &["--flat-mix", "--tail-db", "7"],
            &["--predelay-ms", "-1"],
            &["--predelay-ms", "x"],
            &["--echo-db", "inf"],
        ] {
            assert!(parse_args_from(argv(bad)).is_err(), "{bad:?}");
        }
        let top = Mix::default().step(40.0, 40.0, 400.0);
        assert_eq!(
            (top.echo_db, top.tail_db, top.predelay_ms),
            (12.0, 12.0, 200.0)
        );
        let bottom = Mix::default().step(-40.0, -40.0, -10.0);
        assert_eq!(
            (bottom.echo_db, bottom.tail_db, bottom.predelay_ms),
            (-24.0, -24.0, 0.0)
        );
        let flat_top = flat.audio.mix.step(40.0, 40.0, 0.0);
        assert_eq!((flat_top.echo_db, flat_top.tail_db), (30.0, 6.0));
        let flat_bottom = flat.audio.mix.step(-40.0, -40.0, 0.0);
        assert_eq!((flat_bottom.echo_db, flat_bottom.tail_db), (-12.0, -30.0));
    }

    /// A room prints the mix line again when it moves either curve by 0.5 dB or more since the
    /// last printed line; under `--flat-mix` never.
    #[test]
    fn a_room_reprints_the_mix_line_when_a_curve_moves() {
        let mut mix = Mix::default();
        assert!(
            mix.take_room(2.666),
            "no room (+6 dB) to the 4 m room (+30 dB)"
        );
        assert_eq!(
            mix.line(),
            "mix: echoes +0 dB (room +30.0 dB at mfp 2.666 m), tail +0 dB (room -15.0 dB), \
             pre-delay 0 ms, tail lines handover, direct delay off"
        );
        assert!(!mix.take_room(2.6), "both curves hold below the first knot");
        assert!(mix.take_room(7.999));
        assert!((mix.curve_db(mix.room).0 - 15.0).abs() < 0.01);
        assert!(!mix.take_room(8.1), "+15.0 to +14.9 dB");
        assert!(
            mix.take_room(10.0),
            "+15.0 to +14.4 dB since the printed line"
        );
        let mut flat = Mix {
            flat: true,
            ..Mix::default()
        };
        assert!(!flat.take_room(2.666));
        assert_eq!(flat.room, Some(2.666));
    }
}
