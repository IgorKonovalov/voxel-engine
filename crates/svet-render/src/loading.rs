//! Loading's pieces (ADR-0059): [`LoadProgress`], app-named stages each with a weight, whose
//! overall fraction never moves back; [`EasedBar`], the bar's shown value; and
//! [`LoadingScreen`], the engine's default loading frame on the text overlay.

use crate::hud::{HudColour, HudOverlay};

/// One stage of loading: its name, its share of the bar, and its count when it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadStage {
    pub name: &'static str,
    /// The stage's share of the bar, relative to the other stages' weights.
    pub weight: f32,
    /// Units done, of `of` when the stage knows its count.
    pub done: u64,
    /// The stage's count; `None` when it has none, and then it fills only when it ends.
    pub of: Option<u64>,
}

/// Ordered loading stages and the fraction they add up to. The current stage fills its weight
/// by its count; a stage that ends fills all of it. [`LoadProgress::fraction`] is the largest
/// fraction reported so far, so a count that drops (a chunk withdrawn) holds the bar.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadProgress {
    stages: Vec<LoadStage>,
    current: usize,
    shown: f32,
}

impl LoadProgress {
    /// Stages of these names and weights, in order, at the start of the first.
    pub fn new(stages: &[(&'static str, f32)]) -> Self {
        Self {
            stages: stages
                .iter()
                .map(|&(name, weight)| LoadStage {
                    name,
                    weight: weight.max(0.0),
                    done: 0,
                    of: None,
                })
                .collect(),
            current: 0,
            shown: 0.0,
        }
    }

    /// The current stage's count: `done` of `of`.
    pub fn set(&mut self, done: u64, of: Option<u64>) {
        if let Some(s) = self.stages.get_mut(self.current) {
            s.done = done;
            s.of = of;
        }
        self.update();
    }

    /// Ends the current stage and starts the next; past the last, loading has ended.
    pub fn next(&mut self) {
        self.current = (self.current + 1).min(self.stages.len());
        self.update();
    }

    /// Moves to the stage named `name` if it lies ahead, ending every stage before it; a name
    /// behind or unknown is ignored.
    pub fn enter(&mut self, name: &str) {
        if let Some(i) = self.stages.iter().position(|s| s.name == name)
            && i > self.current
        {
            self.current = i;
            self.update();
        }
    }

    /// Ends every stage.
    pub fn finish(&mut self) {
        self.current = self.stages.len();
        self.update();
    }

    /// Whether every stage has ended.
    pub fn finished(&self) -> bool {
        self.current >= self.stages.len()
    }

    /// The overall fraction, 0 to 1: never below a value it reported before.
    pub fn fraction(&self) -> f32 {
        self.shown
    }

    /// The current stage; the last once loading has ended. `None` without stages.
    pub fn stage(&self) -> Option<&LoadStage> {
        self.stages.get(self.current).or_else(|| self.stages.last())
    }

    /// The stages, in order.
    pub fn stages(&self) -> &[LoadStage] {
        &self.stages
    }

    fn update(&mut self) {
        let total: f32 = self.stages.iter().map(|s| s.weight).sum();
        let raw = if self.finished() || total <= 0.0 {
            if self.finished() { 1.0 } else { 0.0 }
        } else {
            let before: f32 = self.stages[..self.current].iter().map(|s| s.weight).sum();
            let s = &self.stages[self.current];
            let part = match s.of {
                Some(of) if of > 0 => (s.done as f32 / of as f32).clamp(0.0, 1.0),
                _ => 0.0,
            };
            (before + part * s.weight) / total
        };
        self.shown = self.shown.max(raw.clamp(0.0, 1.0));
    }
}

/// The loading frame's shader: one fullscreen pass for the background, the turning cube and
/// the bar.
pub const LOADING_WGSL: &str = include_str!("shaders/loading.wgsl");

/// What one loading frame shows. Time comes in as `seconds`, so a frame is a function of its
/// inputs.
#[derive(Clone, Copy, Debug)]
pub struct LoadingView<'a> {
    /// The first row: the scene, or what a game names its load.
    pub title: &'a str,
    /// The current stage's name.
    pub stage: &'a str,
    /// The stage's count or note (`31 / 49 chunks`); empty for none.
    pub detail: &'a str,
    /// The bar's fill, 0 to 1 (an [`EasedBar`]'s value, or a [`LoadProgress`]'s fraction).
    pub progress: f32,
    /// Seconds since loading began: the cube's turn and the elapsed row.
    pub seconds: f32,
    /// The frame's opacity over what the target holds: 1 while loading, falling to 0 over the
    /// fade into the first world frames. Below 1 the text rows are not drawn.
    pub alpha: f32,
}

/// The time constant the eased bar approaches its target with, in seconds.
pub const BAR_EASE_S: f32 = 0.15;

/// The bar's shown value: it approaches the target fraction with [`BAR_EASE_S`], never
/// exceeds the target, never moves back, and snaps to 1 when loading ends.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EasedBar {
    shown: f32,
}

impl EasedBar {
    /// Advances the shown value toward `target` over `dt` seconds and returns it.
    pub fn update(&mut self, target: f32, dt: f32) -> f32 {
        let target = target.clamp(0.0, 1.0);
        if target > self.shown {
            let k = 1.0 - (-dt.max(0.0) / BAR_EASE_S).exp();
            let next = self.shown + (target - self.shown) * k;
            // Within a thousandth the bar lands on its target.
            self.shown = if target - next < 1e-3 {
                target
            } else {
                next.min(target)
            };
        }
        self.shown
    }

    /// Loading has ended: the bar shows full.
    pub fn snap(&mut self) -> f32 {
        self.shown = 1.0;
        self.shown
    }

    pub fn value(&self) -> f32 {
        self.shown
    }
}

/// The uniform block of `loading.wgsl`, field for field.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    size: [f32; 2],
    seconds: f32,
    progress: f32,
    bar: [f32; 4],
    alpha: f32,
    border: f32,
    cube_px: f32,
    cube_y: f32,
}

/// The engine's default loading frame (ADR-0059): a background, a 3 x 3 x 3 voxel cube turning
/// about a tilted axis, a bordered progress bar, then the text rows on a [`HudOverlay`] it
/// owns. A game that wants its own look draws its own frame from the same [`LoadProgress`].
pub struct LoadingScreen {
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    params: wgpu::Buffer,
    hud: HudOverlay,
    /// The rows' text, reused between frames.
    text: String,
}

/// The bar's outer rect in target pixels for a `size` target, `[x0, y0, x1, y1]`: half the
/// width, centred, at 70 % of the height, 2 % of the height tall (at least 12 px), and its
/// border in pixels.
pub fn loading_bar_rect(size: [u32; 2]) -> ([f32; 4], f32) {
    let [w, h] = [size[0] as f32, size[1] as f32];
    let border = (h / 360.0).round().max(1.0);
    let bar_h = (h * 0.02).round().max(12.0);
    let x0 = (w * 0.25).round();
    let y0 = (h * 0.70).round();
    ([x0, y0, w - x0, y0 + bar_h], border)
}

/// The bar's inner rect (inside the border and a gap of one border width) for a `size` target.
pub fn loading_bar_inner(size: [u32; 2]) -> [f32; 4] {
    let (r, b) = loading_bar_rect(size);
    [
        r[0] + 2.0 * b,
        r[1] + 2.0 * b,
        r[2] - 2.0 * b,
        r[3] - 2.0 * b,
    ]
}

/// Where a `size` target's loading frame puts its parts, in target pixels: a function of the
/// size alone, so every loading frame drawn at one size (the first after the device and each
/// stage's) is the same picture, the cube and the bar centred across the width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoadingLayout {
    /// The bar's outer rect, `[x0, y0, x1, y1]` ([`loading_bar_rect`]).
    pub bar: [f32; 4],
    /// The bar's border width.
    pub border: f32,
    /// The cube's centre: half the width, at 45 % of the height.
    pub cube_centre: [f32; 2],
    /// The cube's bounding radius on screen: 9 % of the height, at least 16 px.
    pub cube_px: f32,
}

/// The layout of a `size` target's loading frame.
pub fn loading_layout(size: [u32; 2]) -> LoadingLayout {
    let (bar, border) = loading_bar_rect(size);
    let [w, h] = [size[0] as f32, size[1] as f32];
    LoadingLayout {
        bar,
        border,
        cube_centre: [0.5 * w, (h * 0.45).round()],
        cube_px: (h * 0.09).round().max(16.0),
    }
}

impl LoadingScreen {
    /// A loading frame for targets of `format`, its rows drawn at the overlay's `scale`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, scale: u32) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("loading.wgsl"),
            source: wgpu::ShaderSource::Wgsl(LOADING_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("loading"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("loading"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("loading"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("loading params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("loading"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            }],
        });
        Self {
            pipeline,
            bind,
            params,
            hud: HudOverlay::new(device, format, scale),
            text: String::with_capacity(256),
        }
    }

    /// The overlay's CPU allocations so far ([`HudOverlay::allocations`]); the text buffer
    /// is reserved at creation.
    pub fn allocations(&self) -> u32 {
        self.hud.allocations()
    }

    /// Records the loading frame onto `target`, a `size` target: the pass over what it holds
    /// (opaque at `alpha` 1), then at `alpha` 1 the rows: the title, the stage and its detail,
    /// and the seconds.
    pub fn encode(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: [u32; 2],
        view: &LoadingView<'_>,
    ) {
        self.encode_timed(queue, encoder, target, size, view, None);
    }

    /// [`Self::encode`], writing the loading pass's start and end timestamps when given.
    pub fn encode_timed(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: [u32; 2],
        view: &LoadingView<'_>,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        if size[0] == 0 || size[1] == 0 {
            return;
        }
        let layout = loading_layout(size);
        let params = Params {
            size: [size[0] as f32, size[1] as f32],
            seconds: view.seconds,
            progress: view.progress.clamp(0.0, 1.0),
            bar: layout.bar,
            alpha: view.alpha.clamp(0.0, 1.0),
            border: layout.border,
            cube_px: layout.cube_px,
            cube_y: layout.cube_centre[1],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("loading"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                timestamp_writes: timestamps,
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        if view.alpha < 1.0 {
            return;
        }
        use std::fmt::Write as _;
        self.text.clear();
        let _ = writeln!(self.text, "{}", view.title);
        if view.detail.is_empty() {
            let _ = writeln!(self.text, "{}", view.stage);
        } else {
            let _ = writeln!(self.text, "{}: {}", view.stage, view.detail);
        }
        let _ = write!(
            self.text,
            "{:.0} %  {:.1} s",
            view.progress.clamp(0.0, 1.0) * 100.0,
            view.seconds
        );
        self.hud
            .set_lines(queue, &self.text, &[HudColour::Yellow, HudColour::White]);
        self.hud.encode(encoder, target, size[0], size[1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_wgsl_validates_through_naga() {
        let module = wgpu::naga::front::wgsl::parse_str(LOADING_WGSL).expect("loading.wgsl parses");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .expect("loading.wgsl validates");
    }

    /// Fed 0.2, 0.2, then 0.6 at 16 ms steps, the shown value never decreases, never exceeds
    /// the target, and is within 0.01 of 0.6 by 1.0 s; `snap` shows 1.
    #[test]
    fn the_eased_bar_is_monotone_and_reaches_the_target() {
        let mut bar = EasedBar::default();
        let mut last = 0.0;
        let dt = 0.016;
        let mut t = 0.0;
        for i in 0..63 {
            let target = if i < 2 { 0.2 } else { 0.6 };
            let v = bar.update(target, dt);
            t += dt;
            assert!(v >= last, "moved back at {t} s: {last} -> {v}");
            assert!(v <= target, "passed its target at {t} s: {v} > {target}");
            last = v;
        }
        assert!(t <= 1.01);
        assert!((0.6 - last).abs() <= 0.01, "{last} at {t} s");
        assert_eq!(bar.update(0.3, dt), last, "a lower target holds the bar");
        assert_eq!(bar.snap(), 1.0);
    }

    /// At 1280 x 720 (the window's first size) and 2048 x 1124 (the size a tiling compositor
    /// gave it on a 2560 x 1440 screen at scale 1.25), the cube's centre is half the width
    /// and the bar's rect is symmetric about it, both inside the target.
    #[test]
    fn the_loading_layout_is_centred_at_two_sizes() {
        for size in [[1280, 720], [2048, 1124]] {
            let l = loading_layout(size);
            let [w, h] = [size[0] as f32, size[1] as f32];
            assert_eq!(l.cube_centre[0], w / 2.0, "{size:?}");
            assert_eq!(
                l.bar[0] + l.bar[2],
                w,
                "the bar centred at {size:?}: {:?}",
                l.bar
            );
            assert!(l.bar[0] > 0.0 && l.bar[3] < h, "{size:?}: {:?}", l.bar);
            assert!(
                l.cube_centre[1] - l.cube_px > 0.0 && l.cube_centre[1] + l.cube_px < l.bar[1],
                "the cube above the bar at {size:?}: {l:?}"
            );
        }
        assert_ne!(loading_layout([1280, 720]), loading_layout([2048, 1124]));
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    /// Stages weighted 1, 2 and 7 read 0.1 once the first ends and 0.3 once the second does;
    /// a count that drops holds the bar where it was.
    #[test]
    fn the_bar_never_moves_back() {
        let mut p = LoadProgress::new(&[("a", 1.0), ("b", 2.0), ("c", 7.0)]);
        assert_eq!(p.fraction(), 0.0);
        p.next();
        assert!(close(p.fraction(), 0.1), "{}", p.fraction());
        p.set(1, Some(2));
        assert!(close(p.fraction(), 0.2), "{}", p.fraction());
        p.next();
        assert!(close(p.fraction(), 0.3), "{}", p.fraction());
        p.set(5, Some(10));
        assert!(close(p.fraction(), 0.65), "{}", p.fraction());
        p.set(4, Some(10));
        assert!(close(p.fraction(), 0.65), "a withdrawn chunk holds the bar");
        p.set(4, Some(20));
        assert!(close(p.fraction(), 0.65), "a grown count holds the bar");
        p.finish();
        assert_eq!(p.fraction(), 1.0);
        assert!(p.finished());
        assert_eq!(p.stage().map(|s| s.name), Some("c"));
    }

    /// A stage fills its weight by its count: 3 of 4 of the first of two equal stages is
    /// 0.375; a stage without a count fills nothing until it ends.
    #[test]
    fn an_unweighted_stage_fills_by_its_count() {
        let mut p = LoadProgress::new(&[("a", 1.0), ("b", 1.0)]);
        p.set(3, Some(4));
        assert!(close(p.fraction(), 0.375), "{}", p.fraction());
        p.next();
        p.set(7, None);
        assert!(close(p.fraction(), 0.5), "{}", p.fraction());
        p.enter("a");
        assert_eq!(
            p.stage().map(|s| s.name),
            Some("b"),
            "a stage behind is ignored"
        );
    }
}
