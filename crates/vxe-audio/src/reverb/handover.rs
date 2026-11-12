//! The tail's two networks, which one is fed, how a change of room moves between them, and the
//! pre-delay line in front of them.

use super::network::{Network, SPARE_SETS, SpareLines};
use super::{
    ENERGY_BANDS, PRE_RING, REVERB_LINES, REVERB_MAX_DELAY_FRAMES, RING, ReverbClears, ReverbLine,
    ReverbNetwork, ReverbSettings, TAIL_HANDOVER_RATIO, TailLines,
};
use crate::command::RoomTargets;
use crate::{BLOCK, RATE, SMOOTHING_SECONDS};

/// The tail: two networks, one fed, and how a change of room moves between them.
pub(crate) struct Reverb {
    nets: [Network; 2],
    /// The network the input goes to.
    fed: usize,
    /// The input crossfade from the other network to `fed`, 0 to 1; 1 when done.
    xfade: f32,
    /// A room waiting for the other network to fade out and clear.
    pending: Option<RoomTargets>,
    settings: ReverbSettings,
    /// `cos w` at the centre of each of the [`ENERGY_BANDS`] bands.
    band_cos: [f32; ENERGY_BANDS],
    pre: PreDelay,
    spare: SpareLines,
}

/// The pre-delay line and its crossfade between two read points (module comment).
struct PreDelay {
    ring: Vec<f32>,
    write: usize,
    /// The read point being left and the one being taken, frames back.
    from: usize,
    to: usize,
    /// Crossfade progress from `from` to `to`, 0 to 1; 1 when done.
    x: f32,
    /// A pre-delay waiting for the crossfade in progress.
    pending: Option<usize>,
    /// One block of the delayed input.
    out: Vec<f32>,
}

impl PreDelay {
    fn new() -> Self {
        Self {
            ring: vec![0.0; PRE_RING],
            write: 0,
            from: 0,
            to: 0,
            x: 1.0,
            pending: None,
            out: vec![0.0; BLOCK],
        }
    }

    /// Asks for a read point `frames` back.
    fn set(&mut self, frames: usize) {
        if self.x < 1.0 {
            self.pending = (frames != self.to).then_some(frames);
        } else if frames != self.to {
            self.from = self.to;
            self.to = frames;
            self.x = 0.0;
        }
    }

    /// Writes `input` into the line and returns it delayed.
    fn process(&mut self, input: &[f32]) -> &[f32] {
        let n = input.len();
        if self.x >= 1.0 && self.pending.is_none() && self.to == 0 {
            // No pre-delay: the line still records, so a later one has history to read.
            for &v in input {
                self.ring[self.write] = v;
                self.write = if self.write + 1 == PRE_RING {
                    0
                } else {
                    self.write + 1
                };
            }
            self.out[..n].copy_from_slice(input);
            return &self.out[..n];
        }
        let step = 1.0 / (SMOOTHING_SECONDS * RATE as f32);
        let back = |w: usize, d: usize| if w >= d { w - d } else { w + PRE_RING - d };
        for (j, &v) in input.iter().enumerate() {
            self.ring[self.write] = v;
            let a = self.ring[back(self.write, self.from)];
            let b = self.ring[back(self.write, self.to)];
            if self.x < 1.0 {
                self.x = (self.x + step).min(1.0);
            }
            self.out[j] = a + (b - a) * self.x;
            self.write = if self.write + 1 == PRE_RING {
                0
            } else {
                self.write + 1
            };
            if self.x >= 1.0
                && let Some(next) = self.pending.take()
            {
                self.from = self.to;
                self.to = next;
                self.x = 0.0;
            }
        }
        &self.out[..n]
    }
}

impl Reverb {
    pub fn new() -> Self {
        Self {
            nets: [Network::new(), Network::new()],
            fed: 0,
            xfade: 1.0,
            pending: None,
            settings: ReverbSettings::default(),
            band_cos: std::array::from_fn(|k| {
                (std::f32::consts::PI * (k as f32 + 0.5) / ENERGY_BANDS as f32).cos()
            }),
            pre: PreDelay::new(),
            spare: SpareLines::new(),
        }
    }

    /// Bytes the lines hold: both networks' and the spare set.
    pub fn line_bytes() -> usize {
        (2 + SPARE_SETS) * REVERB_LINES * RING * 4
    }

    /// How the clears ran so far.
    pub fn clears(&self) -> ReverbClears {
        self.spare.stats
    }

    /// Bytes the pre-delay line holds.
    pub fn predelay_bytes() -> usize {
        PRE_RING * 4
    }

    pub fn set_settings(&mut self, settings: ReverbSettings) {
        self.settings = settings;
        self.pre.set(settings.predelay_frames());
        for net in &mut self.nets {
            net.t60_target = net.t60_target.min(settings.max_t60_s);
            net.take_room_level(net.room_mfp, &settings);
        }
    }

    /// Takes a room (module comment, "A change of room").
    pub fn set_room(&mut self, room: RoomTargets) {
        let settings = self.settings;
        let fed = &mut self.nets[self.fed];
        if !fed.started {
            fed.set_at_once(room, &settings, 0.0);
            return;
        }
        match settings.lines {
            TailLines::Glide => {
                self.pending = None;
                fed.set_in_place(room, &settings, true);
            }
            TailLines::Handover => {
                let (a, b) = (fed.mfp, room.mean_free_path_m.max(0.0));
                let within = (a == 0.0 && b == 0.0)
                    || (a > 0.0 && b > 0.0 && a.max(b) <= TAIL_HANDOVER_RATIO * a.min(b));
                if within {
                    self.pending = None;
                    fed.set_in_place(room, &settings, false);
                } else {
                    self.pending = Some(room);
                    let other = &mut self.nets[1 - self.fed];
                    if other.active && other.fade.is_none() {
                        other.fade = Some(1.0);
                    }
                    self.take_pending();
                }
            }
        }
    }

    /// Hands the pending room to the other network if it is silent and not parked: it takes the
    /// room's lengths and decay at once and the input starts crossfading to it.
    fn take_pending(&mut self) {
        let other = 1 - self.fed;
        if self.nets[other].active || self.pending.is_none() {
            return;
        }
        self.nets[other].clear(&mut self.spare);
        if self.nets[other].parked() {
            return;
        }
        let Some(room) = self.pending.take() else {
            return;
        };
        let settings = self.settings;
        let net = &mut self.nets[other];
        // Its input weight starts at 0, so its input gain starts at its target.
        net.set_at_once(room, &settings, 0.0);
        net.b = net.input_gain(net.t60, &settings, &self.band_cos);
        self.fed = other;
        self.xfade = 0.0;
    }

    /// Moves the fed network's line `i` current length by `frames` at once, within the ring.
    pub fn jump(&mut self, i: usize, frames: f32) {
        let net = &mut self.nets[self.fed];
        net.length[i] = (net.length[i] + frames).clamp(1.0, REVERB_MAX_DELAY_FRAMES as f32);
    }

    /// The fed network's current and target length per line.
    pub fn lines(&self) -> [ReverbLine; REVERB_LINES] {
        let net = &self.nets[self.fed];
        std::array::from_fn(|i| ReverbLine {
            length: net.length[i],
            target: net.target[i],
        })
    }

    /// Both networks, for tests.
    pub fn networks(&self) -> [ReverbNetwork; 2] {
        let weights = self.weights(self.xfade);
        std::array::from_fn(|k| {
            let net = &self.nets[k];
            ReverbNetwork {
                lines: std::array::from_fn(|i| ReverbLine {
                    length: net.length[i],
                    target: net.target[i],
                }),
                holds_signal: net.holds_signal(),
                fed: k == self.fed,
                input: weights[k],
            }
        })
    }

    /// Each network's input weight at crossfade position `x`: equal power, `sin` for the fed
    /// network and `cos` for the other.
    fn weights(&self, x: f32) -> [f32; 2] {
        let (fed, other) = if x >= 1.0 {
            (1.0, 0.0)
        } else if x <= 0.0 {
            (0.0, 1.0)
        } else {
            (std::f32::consts::FRAC_PI_2 * x).sin_cos()
        };
        let mut w = [0.0; 2];
        w[self.fed] = fed;
        w[1 - self.fed] = other;
        w
    }

    /// Runs one block: `input` (the voices' summed sends, `n` frames) in, both ears of both
    /// networks added to interleaved `out`.
    pub fn render(&mut self, input: &[f32], out: &mut [f32]) {
        self.take_pending();
        let n = input.len();
        let x0 = self.xfade;
        let x1 = (x0 + n as f32 / (SMOOTHING_SECONDS * RATE as f32)).min(1.0);
        self.xfade = x1;
        let (w0, w1) = (self.weights(x0), self.weights(x1));
        let input = self.pre.process(input);
        for (k, net) in self.nets.iter_mut().enumerate() {
            if net.render(input, out, (w0[k], w1[k]), &self.settings, &self.band_cos) {
                net.clear(&mut self.spare);
            }
        }
        self.spare.end_block();
        if self.spare.has_clean()
            && let Some(net) = self.nets.iter_mut().find(|n| n.parked())
        {
            net.clear(&mut self.spare);
        }
    }
}
