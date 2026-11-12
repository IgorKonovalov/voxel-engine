//! The frame's probe channel: the probe volume and its readings.

use super::*;

impl Frame {
    /// Probe rays the last encode dispatched (update-list entries times
    /// [`Lighting::probe_rays`]); 0 with the channel or the lamp off, or the update off.
    pub fn traced_probe_rays(&self) -> u64 {
        self.traced_probe_rays
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The probe trace, blend and gather dispatches the last encode recorded: 0 with the lamp
    /// off or the channel off.
    pub fn probe_dispatches(&self) -> u32 {
        self.probe_dispatches
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether the last encode's probe update ran inside a light-change window: the channel
    /// starting (the lamp switched on, the channel turned on, the volume rebuilt) or a change of
    /// the lamp's parameters or shadow walk clears every probe's state and opens one for
    /// [`crate::probes::WINDOW_FRAMES`] encoded frames with a probe update; a change inside it
    /// restarts it. In the window the update refreshes one probe in
    /// [`crate::probes::WINDOW_SLICES`] a frame, so the channel fills from zero faster.
    pub fn probe_light_window(&self) -> bool {
        self.probe_light_window
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The probe volume (ADR-0023), built here if no encode with the lamp channel running, no
    /// [`Frame::probes_mut`] and no spacing change has built it yet, at
    /// [`Frame::probe_axis`] probes a side. Until then the frame holds none of its buffers.
    pub fn probes(&self) -> &ProbeVolume {
        self.probes
            .get_or_init(|| ProbeVolume::new(&self.device, self.probe_spacing, self.probe_axis()))
    }

    /// The probe volume's side: the one [`Frame::set_probe_axis`] fixed, or the smallest power
    /// of two whose half covers [`Lighting::lamp_reach`] at the spacing
    /// ([`crate::probes::probes_per_axis`]).
    pub fn probe_axis(&self) -> u32 {
        self.probe_axis_fixed.unwrap_or_else(|| {
            crate::probes::probes_per_axis(self.lighting.lamp_reach, self.probe_spacing)
        })
    }

    /// Fixes the probe volume's side at `axis` probes (a power of two in 4 ..= 32), or with
    /// `None` lets it follow the lamp's reach; a built volume of another side is rebuilt as
    /// [`Frame::set_probe_spacing`] does.
    pub fn set_probe_axis(&mut self, axis: Option<u32>) {
        self.probe_axis_fixed = axis;
        self.rebuild_probes();
    }

    /// Replaces a built probe volume whose spacing or side differs from the frame's, every
    /// probe never traced; the old volume's pinned origin and switches carry over, and it is
    /// dropped before the new one is built.
    pub(super) fn rebuild_probes(&mut self) {
        let (spacing, axis) = (self.probe_spacing, self.probe_axis());
        if self
            .probes
            .get()
            .is_none_or(|v| v.spacing() == spacing && v.axis() == axis)
        {
            return;
        }
        let old = self.probes.take();
        let carried = old.as_ref().map(|o| (o.pinned_origin(), o.updates()));
        drop(old);
        let mut v = ProbeVolume::new(&self.device, spacing, axis);
        if let Some((origin, updates)) = carried {
            v.set_origin(origin);
            v.set_updates(updates);
        }
        self.probe_channel_ran
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let _ = self.probes.set(v);
    }

    /// The probe volume, built here as [`Frame::probes`] does.
    pub fn probes_mut(&mut self) -> &mut ProbeVolume {
        self.probes();
        self.probes.get_mut().expect("built by probes()")
    }

    /// Replaces the probe volume with one of probes `spacing` voxels apart, every probe never
    /// traced, when the spacing changes (its side follows, see [`Frame::probe_axis`]); the old
    /// volume's pinned origin and switches carry over, and it is dropped before the new one is
    /// built. A volume not yet built is built at the new spacing on first use.
    pub fn set_probe_spacing(&mut self, _device: &wgpu::Device, spacing: u32) {
        self.probe_spacing = spacing;
        self.rebuild_probes();
    }

    /// The gather's indirect irradiance on the lighting grid (rgba16float, rgb).
    pub fn indirect_irradiance(&self) -> &wgpu::Texture {
        &self.targets.indirect
    }

    /// Writes the probe passes' uniform for the frame `p`: the volume's origin for its first
    /// view's eye, `updates` entries in the update list, the freeze count, the side, the lamp's
    /// reach (0 for a preset volume) and the moving lamp's blend.
    pub(super) fn write_probe_uniform(
        &self,
        queue: &wgpu::Queue,
        volume: &ProbeVolume,
        p: &Params,
        updates: u32,
    ) {
        let u = ProbeUniform {
            origin: volume.origin_for(p.views[0].origin),
            spacing: volume.spacing(),
            updates,
            rays: self.lighting.rays(),
            steady: self.lighting.probe_steady(),
            axis: volume.axis(),
            reach: if volume.updates() {
                self.lighting.lamp_reach.max(0.0)
            } else {
                0.0
            },
            h_move: self.lighting.probe_move_blend.clamp(0.0, 1.0),
            _pad: [0; 2],
        };
        queue.write_buffer(volume.uniform(), 0, bytemuck::bytes_of(&u));
    }
}
