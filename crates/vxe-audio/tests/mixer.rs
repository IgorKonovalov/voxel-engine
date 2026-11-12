//! Headless mixer renders: distance gain, direction, looping, ending, and no allocation per
//! block. Everything runs through the HRTF path except the hard-right panning check, which
//! holds the constant-power panner to its 20 dB.

use std::f32::consts::SQRT_2;
use std::sync::Arc;

use support::{tail_rms, tone};
use vxe_audio::*;

mod support;

const MPV: f32 = 0.1;
const FRAMES: usize = 24_000;
const TAIL: usize = 4_800;

/// `m` with both room curves flat.
fn flat(mut m: Mixer) -> Mixer {
    m.set_mix(MixSettings::flat());
    m
}

/// The HRTF path's mixer over the shipped pack.
fn mixer() -> Mixer {
    let mut m = Mixer::new(Hrtf::sadie2_d2(), MPV);
    m.set_mix(MixSettings::flat());
    m
}

/// `|H(freq)|` of measurement `i`'s left and right impulse responses: the gain a steady tone
/// of `freq` gets through them.
fn gain_at(hrtf: &Arc<Hrtf>, i: usize, freq: f32) -> [f32; 2] {
    hrtf.impulse(i).map(|h| {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (n, &x) in h.iter().enumerate() {
            let w = 2.0 * std::f64::consts::PI * f64::from(freq) * n as f64 / f64::from(RATE);
            re += f64::from(x) * w.cos();
            im -= f64::from(x) * w.sin();
        }
        re.hypot(im) as f32
    })
}

/// 0.5 s of a 1 kHz tone of amplitude 0.5 at `position` (voxels), listener at the origin
/// facing +x with +z up.
fn render_at(mut m: Mixer, position: [f32; 3]) -> [f32; 2] {
    m.apply(Command::Play(
        VoiceId(1),
        tone(1000.0, 0.5, FRAMES),
        PlayParams::at(position),
    ));
    let mut out = vec![0.0; 2 * FRAMES];
    m.render(&mut out, FRAMES);
    tail_rms(&out, TAIL)
}

fn assert_within(actual: f32, expected: f32, rel: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= rel * expected,
        "{what}: {actual} vs {expected} (tolerance {rel})"
    );
}

#[test]
fn distance_gain_in_front() {
    // The tone's RMS through the front measurement's two responses, read from the pack.
    let hrtf = Hrtf::sadie2_d2();
    let front = gain_at(&hrtf, hrtf.nearest([1.0, 0.0, 0.0]), 1000.0);
    println!(
        "front-direction gain at 1 kHz: left {}, right {}",
        front[0], front[1]
    );
    let one_m = render_at(mixer(), [10.0, 0.0, 0.0]);
    for (c, rms) in one_m.iter().enumerate() {
        assert_within(
            *rms,
            0.5 / SQRT_2 * front[c],
            0.01,
            &format!("1 m, channel {c}"),
        );
    }
    let two_m = render_at(mixer(), [20.0, 0.0, 0.0]);
    for (c, rms) in two_m.iter().enumerate() {
        assert_within(
            *rms,
            0.5 / SQRT_2 * front[c] / 2.0,
            0.01,
            &format!("2 m, channel {c}"),
        );
    }
    let half_m = render_at(mixer(), [5.0, 0.0, 0.0]);
    for (c, rms) in half_m.iter().enumerate() {
        assert_within(*rms, one_m[c], 0.01, &format!("0.5 m, channel {c}"));
    }
}

#[test]
fn the_panner_at_centre_gives_each_channel_half_the_power() {
    let centre = 0.5 / SQRT_2 * (1.0 / SQRT_2);
    for (c, rms) in render_at(flat(Mixer::panning(MPV)), [10.0, 0.0, 0.0])
        .iter()
        .enumerate()
    {
        assert_within(*rms, centre, 0.01, &format!("panned 1 m, channel {c}"));
    }
}

#[test]
fn a_source_on_the_right_is_20_db_louder_on_the_right_when_panned() {
    // Facing +x with +z up, the listener's right is -y.
    let [l, r] = render_at(flat(Mixer::panning(MPV)), [0.0, -10.0, 0.0]);
    let db = 20.0 * (r / l.max(1e-12)).log10();
    assert!(db >= 20.0, "right over left {db} dB (left {l}, right {r})");
}

fn max_step(out: &[f32]) -> f32 {
    [0, 1]
        .map(|c| {
            let ch: Vec<f32> = out.iter().skip(c).step_by(2).copied().collect();
            ch.windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0, f32::max)
        })
        .into_iter()
        .fold(0.0, f32::max)
}

#[test]
fn a_looping_voice_wraps_without_a_discontinuity() {
    // Ten periods of 480 Hz fill the 1000 samples exactly, so the loop is seamless.
    let short = tone(480.0, 0.5, 1000);
    let long = tone(480.0, 0.5, 48_000);
    let render = |sound: Sound, looping: bool| {
        let mut m = mixer();
        m.apply(Command::Play(
            VoiceId(7),
            sound,
            PlayParams {
                position: [10.0, 0.0, 0.0],
                gain: 1.0,
                looping,
                direct: true,
            },
        ));
        let mut out = vec![0.0; 2 * 48_000];
        m.render(&mut out, 48_000);
        (out, m.is_playing(VoiceId(7)))
    };
    let (looped, playing) = render(short, true);
    let (reference, _) = render(long, false);
    assert!(playing, "a looping voice keeps playing");
    let step = max_step(&looped);
    let own = max_step(&reference);
    assert!(
        step <= own * 1.001,
        "loop step {step} vs the tone's own {own}"
    );
    assert!(own > 0.0);
}

#[test]
fn a_one_shot_voice_ends_and_reports_finished() {
    let mut m = mixer();
    m.apply(Command::Play(
        VoiceId(3),
        tone(480.0, 0.5, 1000),
        PlayParams::at([10.0, 0.0, 0.0]),
    ));
    let mut out = vec![0.0; 2 * 2048];
    m.render(&mut out, 2048);
    let mut finished = Vec::new();
    m.drain_finished(|id| finished.push(id));
    assert_eq!(finished, [VoiceId(3)]);
    assert!(!m.is_playing(VoiceId(3)));
    assert_eq!(m.voice_count(), 0);
    // Silence once the sound's 1000 samples have left the 128-tap filter.
    let taps = Hrtf::sadie2_d2().taps();
    assert!(out[2 * (1000 + taps - 1)..].iter().all(|&s| s == 0.0));
}

/// `MAX_TAPS` taps for voice `voice` in block `block`: a new set whenever `block` changes.
fn moving_taps(voice: u32, block: u32) -> AcousticTargets {
    let mut taps = [Tap::default(); MAX_TAPS];
    for (k, t) in taps.iter_mut().enumerate() {
        *t = Tap {
            delay_frames: 200 + 500 * k as u32 + 3 * voice + 11 * block,
            gain: 0.02 + 0.001 * block as f32,
        };
    }
    AcousticTargets {
        gain: 0.8,
        cutoff_hz: 2_000.0,
        taps,
        send: 0.8,
    }
}

/// While counting, every block turns the listener (every voice crossfades directions), moves
/// every voice's 16 tap targets (every voice's taps glide), keeps the 2 kHz low-pass on,
/// and one block stops a voice and plays another into its pooled buffers.
#[test]
fn sixty_four_voices_render_a_block_without_allocating() {
    let mut m = mixer();
    let sound = tone(440.0, 0.1, 48_000);
    let params = |i: u32| {
        let a = i as f32 * 0.1;
        PlayParams {
            position: [30.0 * a.cos(), 30.0 * a.sin(), 0.0],
            gain: 1.0,
            looping: true,
            direct: true,
        }
    };
    for i in 0..64u32 {
        m.apply(Command::Play(VoiceId(i), sound.clone(), params(i)));
        m.apply(Command::SetAcoustics(VoiceId(i), moving_taps(i, 0)));
    }
    let mut out = vec![0.0; 2 * BLOCK];
    m.render(&mut out, BLOCK);
    // The replacement voice's sound, cloned before counting.
    let mut replacement = Some(sound.clone());
    let ((), counts) = support::count(|| {
        for block in 1..=16u32 {
            let (s, c) = (block as f32 * 0.5).sin_cos();
            m.apply(Command::SetListener(Listener {
                forward: [c, s, 0.0],
                ..Listener::default()
            }));
            if block == 8 {
                m.apply(Command::Stop(VoiceId(5)));
                if let Some(sound) = replacement.take() {
                    m.apply(Command::Play(VoiceId(64), sound, params(5)));
                }
            }
            for i in (0..=64u32).filter(|&i| i != 5) {
                m.apply(Command::SetAcoustics(VoiceId(i), moving_taps(i, block)));
            }
            m.render(&mut out, BLOCK);
        }
    });
    assert_eq!(counts, support::Counts::NONE);
    assert_eq!(m.voice_count(), 64);
    assert!(!m.is_playing(VoiceId(5)));
    assert!(m.is_playing(VoiceId(64)));
    assert!(out.iter().all(|s| s.is_finite()));
}

/// Smoothers heading for 0 settle on exact zeros instead of decaying into subnormals: a
/// looping tone turned down to gain 0 reads own gain and gain of exactly 0 after 10 s, and
/// a stream voice that played a quarter second and then went silent, behind an occluder
/// (the low-pass engaged), reads a low-pass state of exactly 0.
#[test]
fn smoothers_heading_for_zero_settle_on_exact_zeros() {
    let mut m = flat(Mixer::panning(MPV));
    m.apply(Command::Play(
        VoiceId(1),
        tone(440.0, 0.5, RATE as usize),
        PlayParams {
            looping: true,
            ..PlayParams::at([20.0, 0.0, 0.0])
        },
    ));
    let (mut writer, reader) = Stream::new();
    let quarter = tone(440.0, 0.5, RATE as usize / 4);
    let mut pushed = 0;
    while pushed < quarter.len() {
        let n = writer.push(&quarter.samples()[pushed..]);
        assert!(n > 0, "the stream holds a quarter second");
        pushed += n;
    }
    m.apply(Command::PlayStream(
        VoiceId(2),
        reader,
        PlayParams::at([0.0, 20.0, 0.0]),
    ));
    let occluded = AcousticTargets {
        gain: 0.5,
        cutoff_hz: 500.0,
        ..AcousticTargets::default()
    };
    m.apply(Command::SetAcoustics(VoiceId(2), occluded));
    let mut out = vec![0.0; 2 * RATE as usize / 2];
    m.render(&mut out, RATE as usize / 2);
    m.apply(Command::SetGain(VoiceId(1), 0.0));
    for _ in 0..20 {
        m.render(&mut out, RATE as usize / 2);
    }
    let [own, _, gain, _, _] = m.voice_smoothers(VoiceId(1)).unwrap();
    assert_eq!((own, gain), (0.0, 0.0), "own gain {own:e}, gain {gain:e}");
    let [_, _, _, _, lowpass] = m.voice_smoothers(VoiceId(2)).unwrap();
    assert_eq!(lowpass, 0.0, "low-pass state {lowpass:e}");
}

/// A voice whose sound the game has dropped holds the last reference to its samples. From
/// the `Play` to its end the callback neither allocates nor frees; the samples are freed when
/// the game thread recycles.
#[test]
fn a_voice_whose_sound_the_game_dropped_ends_without_a_free_on_the_callback() {
    let mut m = mixer();
    let recycler = m.take_recycler().expect("a new mixer holds its recycler");
    let game = tone(440.0, 0.1, 4 * BLOCK);
    let play = Command::Play(VoiceId(1), game.clone(), PlayParams::at([10.0, 0.0, 0.0]));
    drop(game);
    let mut out = vec![0.0; 2 * BLOCK];
    let (blocks, counts) = support::count(|| {
        m.apply(play);
        let mut blocks = 0;
        while m.is_playing(VoiceId(1)) && blocks < 64 {
            m.render(&mut out, BLOCK);
            blocks += 1;
        }
        blocks
    });
    println!("ended after {blocks} blocks: callback {counts:?}");
    assert!(!m.is_playing(VoiceId(1)));
    assert_eq!(counts, support::Counts::NONE);
    let (taken, game_side) = support::count(|| recycler.recycle());
    println!("recycled {taken}: game thread {game_side:?}");
    assert_eq!(taken, 1);
    assert_eq!(
        game_side.frees, 1,
        "the sound's samples, freed on the game thread"
    );
}

/// A stopped voice's sound, whose last reference the voice held, is not freed on the
/// callback either; nor is the sound of a `Play` for an id already playing, which is dropped.
#[test]
fn a_stopped_voice_and_a_dropped_play_free_nothing_on_the_callback() {
    let mut m = mixer();
    let recycler = m.take_recycler().expect("a new mixer holds its recycler");
    let looping = PlayParams {
        looping: true,
        ..PlayParams::at([10.0, 0.0, 0.0])
    };
    let first = Command::Play(VoiceId(1), tone(440.0, 0.1, BLOCK), looping);
    let again = Command::Play(VoiceId(1), tone(550.0, 0.1, BLOCK), looping);
    let mut out = vec![0.0; 2 * BLOCK];
    let ((), counts) = support::count(|| {
        m.apply(first);
        m.render(&mut out, BLOCK);
        m.apply(again);
        m.render(&mut out, BLOCK);
        m.apply(Command::Stop(VoiceId(1)));
        m.render(&mut out, BLOCK);
    });
    assert_eq!(counts, support::Counts::NONE);
    assert!(!m.is_playing(VoiceId(1)));
    let (taken, game_side) = support::count(|| recycler.recycle());
    assert_eq!(taken, 2);
    assert_eq!(
        game_side.frees, 2,
        "both sounds' samples, freed on the game thread"
    );
}

/// With the game thread not recycling, the channel to it fills: [`MAX_VOICES`] voices end
/// into it, and the next `Play`s find no buffers and are held on the callback, freeing
/// nothing. Once the game recycles, the buffers come back cleared, the held sounds go out on
/// the next block, and a new voice plays.
#[test]
fn a_full_recycling_channel_holds_on_the_callback_and_frees_nothing() {
    let mut m = mixer();
    let recycler = m.take_recycler().expect("a new mixer holds its recycler");
    let mut out = vec![0.0; 2 * BLOCK];
    let plays: Vec<_> = (0..MAX_VOICES as u32 + 8)
        .map(|i| {
            Command::Play(
                VoiceId(i),
                tone(400.0 + i as f32, 0.1, BLOCK),
                PlayParams::at([10.0, 0.0, 0.0]),
            )
        })
        .collect();
    let mut plays = plays.into_iter();
    let ((), counts) = support::count(|| {
        for cmd in plays.by_ref().take(MAX_VOICES) {
            m.apply(cmd);
        }
        for _ in 0..8 {
            m.render(&mut out, BLOCK);
        }
        assert_eq!(m.voice_count(), 0, "every voice ended");
        // No buffers left: these eight are retired at once and held, the channel full.
        for cmd in plays.by_ref() {
            m.apply(cmd);
        }
        assert_eq!(m.voice_count(), 0);
        m.render(&mut out, BLOCK);
    });
    assert_eq!(counts, support::Counts::NONE);
    let (taken, game_side) = support::count(|| recycler.recycle());
    assert_eq!(taken, MAX_VOICES);
    assert_eq!(game_side.frees, MAX_VOICES as u64);
    let replay = Command::Play(
        VoiceId(999),
        tone(300.0, 0.1, BLOCK),
        PlayParams::at([10.0, 0.0, 0.0]),
    );
    let ((), counts) = support::count(|| {
        m.render(&mut out, BLOCK);
        m.apply(replay);
        m.render(&mut out, BLOCK);
    });
    assert_eq!(counts, support::Counts::NONE);
    assert!(
        m.is_playing(VoiceId(999)),
        "a recycled buffer took the new voice"
    );
    let (taken, game_side) = support::count(|| recycler.recycle());
    assert_eq!(taken, 8, "the held sounds, sent once the channel had room");
    assert_eq!(game_side.frees, 8);
}

/// A burst larger than every hold: [`MAX_VOICES`] looping voices play, then one block gets 200
/// `Play`s (no buffers left, so each is dropped) and [`MAX_VOICES`] `Stop`s. The dropped
/// `Play`s fill the channel and the source hold; the stopped voices still wait in the voice
/// hold, so the callback frees nothing and loses no buffers. After the game thread recycles
/// between blocks, [`MAX_VOICES`] new `Play`s start [`MAX_VOICES`] voices.
#[test]
fn a_burst_past_every_hold_keeps_every_voice_buffer_and_frees_nothing() {
    let mut m = mixer();
    let recycler = m.take_recycler().expect("a new mixer holds its recycler");
    let looping = PlayParams {
        looping: true,
        ..PlayParams::at([10.0, 0.0, 0.0])
    };
    // One sound the test keeps, so a dropped `Play`'s source is never its last reference.
    let sound = tone(440.0, 0.1, BLOCK);
    let n = MAX_VOICES as u32;
    let mut first: Vec<_> = (0..n)
        .map(|i| Command::Play(VoiceId(i), sound.clone(), looping))
        .collect();
    let mut burst: Vec<_> = (n..n + 200)
        .map(|i| Command::Play(VoiceId(i), sound.clone(), looping))
        .collect();
    let mut again: Vec<_> = (1000..1000 + n)
        .map(|i| Command::Play(VoiceId(i), sound.clone(), looping))
        .collect();
    let mut out = vec![0.0; 2 * BLOCK];
    let ((), counts) = support::count(|| {
        for cmd in first.drain(..) {
            m.apply(cmd);
        }
        m.render(&mut out, BLOCK);
        assert_eq!(m.voice_count(), MAX_VOICES);
        for cmd in burst.drain(..) {
            m.apply(cmd);
        }
        for i in 0..n {
            m.apply(Command::Stop(VoiceId(i)));
        }
        m.render(&mut out, BLOCK);
        assert_eq!(m.voice_count(), 0);
    });
    assert_eq!(counts.frees, 0, "callback {counts:?}");
    for _ in 0..4 {
        recycler.recycle();
        let ((), counts) = support::count(|| m.render(&mut out, BLOCK));
        assert_eq!(counts.frees, 0, "callback {counts:?}");
    }
    recycler.recycle();
    let ((), counts) = support::count(|| {
        for cmd in again.drain(..) {
            m.apply(cmd);
        }
        m.render(&mut out, BLOCK);
    });
    assert_eq!(counts.frees, 0, "callback {counts:?}");
    assert_eq!(
        m.voice_count(),
        MAX_VOICES,
        "every buffer set came back to the pool"
    );
}

/// With the game's end dropped, the voices that end keep their buffers on the mixer: the next
/// [`MAX_VOICES`] `Play`s start [`MAX_VOICES`] voices again.
#[test]
fn a_dropped_recycler_keeps_every_voice_buffer_on_the_mixer() {
    let mut m = mixer();
    drop(m.take_recycler());
    let sound = tone(440.0, 0.1, BLOCK);
    let mut out = vec![0.0; 2 * BLOCK];
    for round in 0..2u32 {
        for i in 0..MAX_VOICES as u32 {
            m.apply(Command::Play(
                VoiceId(round * 1000 + i),
                sound.clone(),
                PlayParams::at([10.0, 0.0, 0.0]),
            ));
        }
        assert_eq!(m.voice_count(), MAX_VOICES, "round {round}");
        for _ in 0..8 {
            m.render(&mut out, BLOCK);
        }
        assert_eq!(m.voice_count(), 0, "every voice ended");
    }
}
