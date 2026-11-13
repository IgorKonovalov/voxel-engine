//! The camera path in the run: the path step and `--report`'s `run.json`.

use crate::app::peak_rss_bytes;
use crate::report::{AdapterLine, RunReport};
use crate::run::Run;
use vxe_app::{Flow, Input};

impl Run {
    /// Steps `--path`'s player by the frame's interval (1/60 s a headless frame, none on the
    /// first frame, whose interval spans the startup): `Some(Exit)` once the run ended,
    /// `Some(Continue)` when the path placed the camera, `None` when the tester flies or there
    /// is no path.
    pub(crate) fn path_step(&mut self, input: &Input, dt: f32) -> Option<Flow> {
        let p = self.player.as_mut()?;
        let step = if self.frame == 0 {
            0.0
        } else if self.args.headless {
            1.0 / 60.0
        } else {
            dt
        };
        let eye = self.fly.camera.position.to_array();
        let pose = p.step(step, crate::path::Controls::read(input), eye);
        if p.ended() {
            return Some(Flow::Exit);
        }
        let pose = pose?;
        let c = &mut self.fly.camera;
        c.position = pose.eye.into();
        c.yaw = pose.yaw_deg.to_radians();
        c.pitch = pose.pitch_deg.to_radians();
        Some(Flow::Continue)
    }

    /// Writes `--report`'s `run.json`: what ran, on which adapter, and the recorder's frame
    /// times; `failed` when the session ended on an error.
    pub(crate) fn write_report(&self) {
        let (Some(dir), Some(p), Some(rec)) = (&self.args.report, &self.player, &self.recorder)
        else {
            return;
        };
        let (_, scale, _) = self.args.upscaler_settings(self.modes.upscaler.mode);
        let sizes = self
            .gfx
            .as_ref()
            .map(|g| (g.frame.output_size(), g.frame.internal_size()));
        let mut r = RunReport {
            version: env!("CARGO_PKG_VERSION"),
            commit: crate::report::COMMIT,
            scene: p.path.scene.name(),
            path: p.path.name,
            mode: p.mode.name(),
            output: sizes.map_or([0, 0], |s| s.0),
            internal: sizes.map_or([0, 0], |s| s.1),
            internal_scale: scale.name(),
            upscaler: self.modes.upscaler.mode.name(),
            lamp: self.args.light.lamp(),
            adapter: self.adapter.as_ref().map(|a| AdapterLine {
                name: a.name.clone(),
                vendor: a.vendor,
                device: a.device,
                device_type: format!("{:?}", a.device_type),
                driver: a.driver.clone(),
                driver_info: a.driver_info.clone(),
                backend: vxe_app::backend_label(a.backend),
            }),
            path_s: p.path.duration_s(),
            frames: 0,
            warmup_frames: 0,
            warmup_s: 0.0,
            wall_ms: None,
            cpu_ms: None,
            gpu_ms: None,
            over_33ms: 0,
            over_100ms: 0,
            peak_rss_bytes: peak_rss_bytes(),
            streaming: None,
            ended: if self.failure.is_some() {
                "failed"
            } else {
                p.ended_word()
            },
        };
        rec.fill(&mut r);
        let file = dir.join("run.json");
        match std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&file, r.to_json())) {
            Ok(()) => println!("report: wrote {}", file.display()),
            Err(e) => eprintln!("flythrough: --report {}: {e}", file.display()),
        }
    }
}
