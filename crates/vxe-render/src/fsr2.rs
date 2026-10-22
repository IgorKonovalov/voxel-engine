//! AMD FidelityFX Super Resolution 2.2.1 ported to WGSL (ADR-0034): FSR 2's upscaling passes
//! under `shaders/fsr2/`, fed from the hit buffer by an input pass of ours. [`Fsr2`] holds one
//! frame's FSR resources and pipelines; [`crate::Frame`] runs it when its upscaler is
//! [`UpscalerKind::Fsr2`].
//!
//! The port takes the portable WGSL forms throughout: the FP32 path, no wave operations, no
//! image atomics, core storage formats (each of AMD's 16- and 8-bit formats widened to one at
//! least as precise), and four-tap loads for AMD's bilinear samples.

use crate::marcher::{Dispatch, Params, Upscaler, WORKGROUP};

/// The parts of FSR 2 every FSR pass is compiled after; carries AMD's notice.
pub const FSR2_COMMON_WGSL: &str = include_str!("shaders/fsr2/common.wgsl");
/// FSR 2's accumulate pass, compiled after [`FSR2_COMMON_WGSL`].
pub const FSR2_ACCUMULATE_WGSL: &str = include_str!("shaders/fsr2/accumulate.wgsl");
/// Our input pass (depth, motion vectors, reactive mask), compiled after `march.wgsl`.
pub const FSR2_INPUT_WGSL: &str = include_str!("shaders/fsr2/input.wgsl");
/// Our tone pass (exposure, tone map, display encoding), compiled after `march.wgsl`.
pub const FSR2_TONE_WGSL: &str = include_str!("shaders/fsr2/tone.wgsl");
/// FSR 2's luminance pyramid (two entry points), compiled after [`FSR2_COMMON_WGSL`].
pub const FSR2_LUMINANCE_PYRAMID_WGSL: &str = include_str!("shaders/fsr2/luminance_pyramid.wgsl");
/// FSR 2's reconstruct-previous-depth pass, compiled after [`FSR2_COMMON_WGSL`].
pub const FSR2_RECONSTRUCT_WGSL: &str =
    include_str!("shaders/fsr2/reconstruct_previous_depth.wgsl");
/// FSR 2's depth-clip pass, compiled after [`FSR2_COMMON_WGSL`].
pub const FSR2_DEPTH_CLIP_WGSL: &str = include_str!("shaders/fsr2/depth_clip.wgsl");
/// FSR 2's lock pass, compiled after [`FSR2_COMMON_WGSL`].
pub const FSR2_LOCK_WGSL: &str = include_str!("shaders/fsr2/lock.wgsl");
/// FSR 2's RCAS pass, compiled after [`FSR2_COMMON_WGSL`].
pub const FSR2_RCAS_WGSL: &str = include_str!("shaders/fsr2/rcas.wgsl");

/// The source of an FSR pass: `HDR` (FFX_FSR2_OPTION_HDR_COLOR_INPUT), the common part, then
/// the pass.
pub fn fsr2_source(pass: &str, hdr: bool) -> String {
    format!("const HDR: bool = {hdr};\n{FSR2_COMMON_WGSL}\n{pass}")
}

/// The near plane FSR's depth is taken against, in voxels: below any hit's view depth that
/// matters (a hit nearer than it still gets a finite depth above 1).
pub const NEAR: f32 = 0.01;

/// FSR 2's knobs in the engine ([`UpscalerKind::Fsr2`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fsr2Settings {
    /// RCAS's sharpness, 0 to 1 (FSR's `sharpness`: `2 - 2 sharpness` stops); 0 skips the
    /// pass (FSR's `enableSharpening` off).
    pub sharpness: f32,
    /// The reactive mask's weight on a translucent layer's `1 - transmittance`.
    pub reactive_translucent: f32,
    /// The reactive mask's value where the light at a hit changed.
    pub reactive_light: f32,
    /// FSR's `viewSpaceToMetersFactor`: the world's voxel edge in metres.
    pub metres_per_voxel: f32,
}

impl Default for Fsr2Settings {
    fn default() -> Self {
        Self {
            sharpness: 0.0,
            reactive_translucent: 1.0,
            reactive_light: 1.0,
            metres_per_voxel: 0.1,
        }
    }
}

/// Which upscaler a [`crate::Frame`] runs ([`crate::Frame::set_upscaler_kind`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UpscalerKind {
    /// No upscaler: the jitter off, and at equal sizes the composite's image is the frame's.
    Off,
    /// The upscaler of ADR-0024 with this tuning.
    Own(Upscaler),
    /// FSR 2 with these settings.
    Fsr2(Fsr2Settings),
}

impl UpscalerKind {
    /// The name the fly-through prints and parses.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Own(_) => "own",
            Self::Fsr2(_) => "fsr2",
        }
    }

    pub fn is_fsr2(&self) -> bool {
        matches!(self, Self::Fsr2(_))
    }
}

/// FSR's jitter phase count for a `render_width` image shown `display_width` wide
/// (`ffxFsr2GetJitterPhaseCount`): `int(8 (display / render)^2)`.
pub fn jitter_phases(render_width: u32, display_width: u32) -> u32 {
    let ratio = display_width as f32 / render_width as f32;
    (8.0 * ratio.powi(2)) as u32
}

/// FSR's jitter offset of frame `index` in a sequence of `phases`, in render pixels
/// (`ffxFsr2GetJitterOffset`): Halton (2, 3) at `index % phases + 1`, minus 0.5. FSR's sample
/// of render pixel `p` lies at `p + 0.5 - offset` in the same y-down pixel space our rays use,
/// so a view's [`crate::View::jitter`] is the offset negated on both axes.
pub fn jitter_offset(index: u32, phases: u32) -> [f32; 2] {
    crate::marcher::jitter_at(index, phases)
}

/// `cbFSR2` of AMD's passes plus the luminance pyramid's and RCAS's constants:
/// `Fsr2Constants` in `common.wgsl`, field for field.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Fsr2Constants {
    pub render_size: [i32; 2],
    pub max_render_size: [i32; 2],
    pub display_size: [i32; 2],
    pub input_color_size: [i32; 2],
    pub luma_mip_size: [i32; 2],
    pub luma_mip_level: i32,
    pub frame_index: i32,
    pub device_to_view: [f32; 4],
    pub jitter: [f32; 2],
    pub motion_vector_scale: [f32; 2],
    pub downscale: [f32; 2],
    pub motion_vector_jitter_cancellation: [f32; 2],
    pub pre_exposure: f32,
    pub previous_pre_exposure: f32,
    pub tan_half_fov: f32,
    pub jitter_sequence_length: f32,
    pub delta_time: f32,
    pub dynamic_res_change: f32,
    pub view_space_to_meters: f32,
    pub spd_mips: u32,
    pub spd_work_groups: u32,
    pub rcas_con: f32,
    pub debug: u32,
    pub _pad: u32,
}

/// The shading-change mip of the luminance pyramid (`FFX_FSR2_SHADING_CHANGE_MIP_LEVEL`).
pub const SHADING_CHANGE_MIP: i32 = 4;

/// `Fsr2InputTuning` in `input.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct InputUniform {
    near: f32,
    reactive_translucent: f32,
    reactive_light: f32,
    _pad: f32,
}

/// FSR 2's resources at one pair of sizes; replaced with the frame's targets.
struct Resources {
    render: [u32; 2],
    display: [u32; 2],
    /// The composite's linear colour at the render size (rgba32float: the tone pass of the
    /// composite's own arithmetic reproduces its rgba8 bytes only from f32 radiance).
    colour: wgpu::Texture,
    /// Our input pass's depth (r32float), motion vectors (rg32float) and masks (rgba8unorm:
    /// reactive, transparency and composition, the edit flag).
    depth: wgpu::Texture,
    motion: wgpu::Texture,
    reactive: wgpu::Texture,
    /// The luminance pyramid's shading-change mip and mip 5 (r32float), and its exposure
    /// (rg32float 1 x 1), this frame's and the last.
    luma_mip: wgpu::Texture,
    luma_mip5: wgpu::Texture,
    auto_exposure: [wgpu::Texture; 2],
    /// The reconstructed previous depth, an atomic `u32` per render pixel.
    reconstructed: wgpu::Buffer,
    /// The reconstruct pass's dilated depth (r32float), dilated motion (rg32float, this
    /// frame's and the last) and lock input luma (r32float).
    dilated_depth: wgpu::Texture,
    dilated_motion: [wgpu::Texture; 2],
    lock_luma: wgpu::Texture,
    /// The depth clip's prepared input colour (rgba16float: YCoCg, depth clip) and dilated
    /// reactive masks (rgba8unorm).
    prepared: wgpu::Texture,
    dilated_reactive: wgpu::Texture,
    /// The exposure (rg32float 1 x 1, x).
    exposure: wgpu::Texture,
    /// The internal upscaled colour (rgba16float), lock status (rg32float) and luma history
    /// (rgba8unorm) at the display size, this frame's and the last.
    upscaled: [wgpu::Texture; 2],
    lock_status: [wgpu::Texture; 2],
    luma_history: [wgpu::Texture; 2],
    /// The new locks, an `f32` per display pixel.
    new_locks: wgpu::Buffer,
    /// The accumulate pass's history weight (r32float), at the display size with
    /// [`Fsr2::set_debug`], else 1 x 1.
    weight: wgpu::Texture,
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    format: wgpu::TextureFormat,
    [width, height]: [u32; 2],
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// Bytes per texel of the formats the port uses.
fn texel_bytes(format: wgpu::TextureFormat) -> u64 {
    use wgpu::TextureFormat as F;
    match format {
        F::Rgba8Unorm | F::R32Float => 4,
        F::Rg32Float | F::Rgba16Float => 8,
        F::Rgba32Float => 16,
        f => unreachable!("no FSR target is {f:?}"),
    }
}

fn texture_bytes(t: &wgpu::Texture) -> u64 {
    u64::from(t.width()) * u64::from(t.height()) * texel_bytes(t.format())
}

/// The size of mip `level` of a pyramid whose mip 0 is half of `render` (AMD's scene
/// luminance), at least 1.
fn mip_size(render: [u32; 2], level: u32) -> [u32; 2] {
    render.map(|n| ((n / 2) >> level).max(1))
}

impl Resources {
    fn new(device: &wgpu::Device, render: [u32; 2], display: [u32; 2], debug: bool) -> Self {
        use wgpu::TextureFormat::{R32Float, Rg32Float, Rgba8Unorm, Rgba16Float, Rgba32Float};
        let pair = |label, format, size| [0, 1].map(|_| texture(device, label, format, size));
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let px = |s: [u32; 2]| u64::from(s[0]) * u64::from(s[1]);
        Self {
            render,
            display,
            colour: texture(device, "fsr2 colour", Rgba32Float, render),
            depth: texture(device, "fsr2 depth", R32Float, render),
            motion: texture(device, "fsr2 motion", Rg32Float, render),
            reactive: texture(device, "fsr2 reactive", Rgba8Unorm, render),
            luma_mip: texture(
                device,
                "fsr2 shading-change mip",
                R32Float,
                mip_size(render, SHADING_CHANGE_MIP as u32),
            ),
            luma_mip5: texture(device, "fsr2 luma mip 5", R32Float, mip_size(render, 5)),
            auto_exposure: pair("fsr2 auto exposure", Rg32Float, [1, 1]),
            reconstructed: buffer("fsr2 reconstructed depth", 4 * px(render)),
            dilated_depth: texture(device, "fsr2 dilated depth", R32Float, render),
            dilated_motion: pair("fsr2 dilated motion", Rg32Float, render),
            lock_luma: texture(device, "fsr2 lock luma", R32Float, render),
            prepared: texture(device, "fsr2 prepared colour", Rgba16Float, render),
            dilated_reactive: texture(device, "fsr2 dilated reactive", Rgba8Unorm, render),
            exposure: texture(device, "fsr2 exposure", Rg32Float, [1, 1]),
            upscaled: pair("fsr2 upscaled", Rgba16Float, display),
            lock_status: pair("fsr2 lock status", Rg32Float, display),
            luma_history: pair("fsr2 luma history", Rgba8Unorm, display),
            new_locks: buffer("fsr2 new locks", 4 * px(display)),
            weight: texture(
                device,
                "fsr2 history weight",
                R32Float,
                if debug { display } else { [1, 1] },
            ),
        }
    }

    fn bytes(&self) -> u64 {
        let singles = [
            &self.colour,
            &self.depth,
            &self.motion,
            &self.reactive,
            &self.luma_mip,
            &self.luma_mip5,
            &self.dilated_depth,
            &self.lock_luma,
            &self.prepared,
            &self.dilated_reactive,
            &self.exposure,
            &self.weight,
        ];
        let pairs = [
            &self.auto_exposure,
            &self.dilated_motion,
            &self.upscaled,
            &self.lock_status,
            &self.luma_history,
        ];
        singles.iter().map(|t| texture_bytes(t)).sum::<u64>()
            + pairs
                .iter()
                .map(|p| texture_bytes(&p[0]) + texture_bytes(&p[1]))
                .sum::<u64>()
            + self.new_locks.size()
            + self.reconstructed.size()
    }
}

/// One FSR pass: its group-1 layout (group 2 for ours) and pipeline.
struct Pass {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

/// One frame's FSR 2: its pipelines and resources ([`crate::Frame`] holds it in FSR mode).
pub struct Fsr2 {
    cb_layout: wgpu::BindGroupLayout,
    input: Pass,
    spd_mips: Pass,
    spd_tail: Pass,
    reconstruct: Pass,
    depth_clip: Pass,
    lock: Pass,
    accumulate: Pass,
    rcas: Pass,
    tone: Pass,
    cb: wgpu::Buffer,
    input_tuning: wgpu::Buffer,
    res: Resources,
    settings: Fsr2Settings,
    /// The tone pass reads the composite's colour and FSR's passes are skipped
    /// ([`Fsr2::set_bypass`]).
    bypass: bool,
    /// The accumulate pass writes its history weight ([`Fsr2::set_debug`]).
    debug: bool,
    /// The pre-exposure of the last encoded frame (`previousFramePreExposure`).
    pre_exposure: std::sync::Mutex<f32>,
}

/// The bind groups of one encoded frame.
pub(crate) struct Fsr2Binds {
    cb: wgpu::BindGroup,
    input: wgpu::BindGroup,
    spd_mips: wgpu::BindGroup,
    spd_tail: wgpu::BindGroup,
    reconstruct: wgpu::BindGroup,
    depth_clip: wgpu::BindGroup,
    lock: wgpu::BindGroup,
    accumulate: wgpu::BindGroup,
    /// With sharpening on: RCAS and its workgroups.
    rcas: Option<(wgpu::BindGroup, [u32; 2])>,
    tone: wgpu::BindGroup,
    bypass: bool,
    render_groups: [u32; 2],
    display_groups: [u32; 2],
    spd_groups: [u32; 2],
}

impl Fsr2Binds {
    /// The dispatches of the frame's upscale slot, after the composite, in AMD's order: our
    /// input pass, the luminance pyramid (two dispatches), reconstruct previous depth, depth
    /// clip, lock and accumulate.
    pub(crate) fn upscale<'a>(
        &'a self,
        fsr: &'a Fsr2,
        world: &'a wgpu::BindGroup,
    ) -> Vec<Dispatch<'a>> {
        if self.bypass {
            return Vec::new();
        }
        let fsr_pass = |p: &'a Pass, b: &'a wgpu::BindGroup, workgroups| Dispatch {
            pipeline: &p.pipeline,
            groups: [Some(&self.cb), Some(b), None, None],
            workgroups,
        };
        vec![
            Dispatch {
                pipeline: &fsr.input.pipeline,
                groups: [Some(world), None, Some(&self.input), None],
                workgroups: self.render_groups,
            },
            fsr_pass(&fsr.spd_mips, &self.spd_mips, self.spd_groups),
            fsr_pass(&fsr.spd_tail, &self.spd_tail, [1, 1]),
            fsr_pass(&fsr.reconstruct, &self.reconstruct, self.render_groups),
            fsr_pass(&fsr.depth_clip, &self.depth_clip, self.render_groups),
            fsr_pass(&fsr.lock, &self.lock, self.render_groups),
            fsr_pass(&fsr.accumulate, &self.accumulate, self.display_groups),
        ]
    }

    /// The dispatches of the frame's sharpen slot: RCAS when sharpening is on, then the tone
    /// pass.
    pub(crate) fn sharpen<'a>(
        &'a self,
        fsr: &'a Fsr2,
        world: &'a wgpu::BindGroup,
    ) -> Vec<Dispatch<'a>> {
        let mut d = Vec::new();
        if let Some((b, workgroups)) = self.rcas.as_ref().filter(|_| !self.bypass) {
            d.push(Dispatch {
                pipeline: &fsr.rcas.pipeline,
                groups: [Some(&self.cb), Some(b), None, None],
                workgroups: *workgroups,
            });
        }
        d.push(Dispatch {
            pipeline: &fsr.tone.pipeline,
            groups: [Some(world), None, Some(&self.tone), None],
            workgroups: self.display_groups,
        });
        d
    }
}

fn layout(
    device: &wgpu::Device,
    label: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries,
    })
}

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    }
}

fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    )
}

fn uint_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    )
}

fn store_entry(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
    )
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}

fn rw_buffer_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}

/// A binding of a bind group: a texture's whole view or a buffer.
enum Bind<'a> {
    Tex(&'a wgpu::Texture),
    Buf(&'a wgpu::Buffer),
}

fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    res: &[Bind<'_>],
) -> wgpu::BindGroup {
    let at: Vec<(u32, &Bind<'_>)> = res.iter().enumerate().map(|(i, r)| (i as u32, r)).collect();
    bind_refs(device, layout, &at)
}

/// A bind group of `res` at explicit binding numbers.
fn bind_at(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    res: &[(u32, Bind<'_>)],
) -> wgpu::BindGroup {
    let at: Vec<(u32, &Bind<'_>)> = res.iter().map(|(b, r)| (*b, r)).collect();
    bind_refs(device, layout, &at)
}

fn bind_refs(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    res: &[(u32, &Bind<'_>)],
) -> wgpu::BindGroup {
    let views: Vec<Option<wgpu::TextureView>> = res
        .iter()
        .map(|(_, r)| match r {
            Bind::Tex(t) => Some(t.create_view(&wgpu::TextureViewDescriptor::default())),
            Bind::Buf(_) => None,
        })
        .collect();
    let entries: Vec<wgpu::BindGroupEntry<'_>> = res
        .iter()
        .zip(&views)
        .map(|((binding, r), v)| wgpu::BindGroupEntry {
            binding: *binding,
            resource: match (r, v) {
                (Bind::Buf(b), _) => b.as_entire_binding(),
                (_, Some(v)) => wgpu::BindingResource::TextureView(v),
                (Bind::Tex(_), None) => unreachable!("a texture binding has its view"),
            },
        })
        .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &entries,
    })
}

fn pipeline(
    device: &wgpu::Device,
    label: &str,
    source: String,
    layouts: &[Option<&wgpu::BindGroupLayout>],
    entry: &str,
) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: layouts,
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        module: &module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}

fn ro_buffer_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}

impl Fsr2 {
    /// FSR 2's pipelines and its resources for `render`-sized frames shown at `display`;
    /// `world_layout` is the frame's group 0, which our input and tone passes read.
    pub(crate) fn new(
        device: &wgpu::Device,
        world_layout: &wgpu::BindGroupLayout,
        render: [u32; 2],
        display: [u32; 2],
        settings: Fsr2Settings,
    ) -> Self {
        use wgpu::TextureFormat::{R32Float, Rg32Float, Rgba8Unorm, Rgba16Float};
        let cb_layout = layout(device, "fsr2 constants", &[uniform_entry(0)]);
        // Ours: group 0 is the frame's world, group 2 the pass's.
        let ours = |label: &str,
                    source: &str,
                    entry_point: &str,
                    entries: &[wgpu::BindGroupLayoutEntry]| {
            let l = layout(device, label, entries);
            let p = pipeline(
                device,
                label,
                format!("{}\n{source}", crate::MARCH_WGSL),
                &[Some(world_layout), None, Some(&l)],
                entry_point,
            );
            Pass {
                layout: l,
                pipeline: p,
            }
        };
        // AMD's: group 0 is the constants, group 1 the pass's.
        let amd = |label: &str,
                   source: &str,
                   entry_point: &str,
                   entries: &[wgpu::BindGroupLayoutEntry]| {
            let l = layout(device, label, entries);
            let p = pipeline(
                device,
                label,
                fsr2_source(source, true),
                &[Some(&cb_layout), Some(&l)],
                entry_point,
            );
            Pass {
                layout: l,
                pipeline: p,
            }
        };
        let input = ours(
            "fsr2 input",
            FSR2_INPUT_WGSL,
            "fsr2_input",
            &[
                tex_entry(0),
                uint_entry(1),
                tex_entry(2),
                uniform_entry(3),
                uniform_entry(4),
                uniform_entry(5),
                store_entry(6, R32Float),
                store_entry(7, Rg32Float),
                store_entry(8, Rgba8Unorm),
            ],
        );
        let tone = ours(
            "fsr2 tone",
            FSR2_TONE_WGSL,
            "tone",
            &[tex_entry(0), store_entry(1, Rgba8Unorm)],
        );
        let spd_mips = amd(
            "fsr2 luminance pyramid",
            FSR2_LUMINANCE_PYRAMID_WGSL,
            "spd_mips",
            &[
                tex_entry(0),
                store_entry(1, R32Float),
                store_entry(2, R32Float),
            ],
        );
        let spd_tail = amd(
            "fsr2 luminance pyramid tail",
            FSR2_LUMINANCE_PYRAMID_WGSL,
            "spd_tail",
            &[tex_entry(3), store_entry(4, Rg32Float), tex_entry(5)],
        );
        let reconstruct = amd(
            "fsr2 reconstruct previous depth",
            FSR2_RECONSTRUCT_WGSL,
            "reconstruct_and_dilate",
            &[
                tex_entry(0),
                tex_entry(1),
                tex_entry(2),
                tex_entry(3),
                rw_buffer_entry(4),
                store_entry(5, R32Float),
                store_entry(6, Rg32Float),
                store_entry(7, R32Float),
            ],
        );
        let depth_clip = amd(
            "fsr2 depth clip",
            FSR2_DEPTH_CLIP_WGSL,
            "depth_clip",
            &[
                tex_entry(0),
                tex_entry(1),
                tex_entry(2),
                tex_entry(3),
                tex_entry(4),
                tex_entry(5),
                tex_entry(6),
                tex_entry(7),
                ro_buffer_entry(8),
                store_entry(9, Rgba16Float),
                store_entry(10, Rgba8Unorm),
            ],
        );
        let lock = amd(
            "fsr2 lock",
            FSR2_LOCK_WGSL,
            "compute_lock",
            &[tex_entry(0), rw_buffer_entry(1), rw_buffer_entry(2)],
        );
        let accumulate = amd(
            "fsr2 accumulate",
            FSR2_ACCUMULATE_WGSL,
            "accumulate",
            &[
                tex_entry(0),
                tex_entry(1),
                tex_entry(2),
                tex_entry(3),
                tex_entry(4),
                tex_entry(5),
                tex_entry(6),
                tex_entry(7),
                rw_buffer_entry(8),
                store_entry(9, Rg32Float),
                store_entry(10, Rgba16Float),
                store_entry(11, Rgba8Unorm),
                store_entry(12, R32Float),
            ],
        );
        let rcas = amd(
            "fsr2 rcas",
            FSR2_RCAS_WGSL,
            "rcas",
            &[tex_entry(0), tex_entry(1), store_entry(2, Rgba16Float)],
        );
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Self {
            cb_layout,
            input,
            spd_mips,
            spd_tail,
            reconstruct,
            depth_clip,
            lock,
            accumulate,
            rcas,
            tone,
            cb: buffer("fsr2 constants", size_of::<Fsr2Constants>() as u64),
            input_tuning: buffer("fsr2 input tuning", size_of::<InputUniform>() as u64),
            res: Resources::new(device, render, display, false),
            settings,
            bypass: false,
            debug: false,
            pre_exposure: std::sync::Mutex::new(1.0),
        }
    }

    /// Replaces the resources with ones for `render`-sized frames shown at `display`.
    pub(crate) fn resize(&mut self, device: &wgpu::Device, render: [u32; 2], display: [u32; 2]) {
        if self.res.render != render || self.res.display != display {
            self.res = Resources::new(device, render, display, self.debug);
        }
    }

    pub(crate) fn set_settings(&mut self, settings: Fsr2Settings) {
        self.settings = settings;
    }

    pub fn settings(&self) -> Fsr2Settings {
        self.settings
    }

    /// For tests at equal sizes: the tone pass reads the composite's linear colour directly,
    /// FSR's passes are not dispatched and the rays are not jittered.
    pub fn set_bypass(&mut self, bypass: bool) {
        self.bypass = bypass;
    }

    pub fn bypassed(&self) -> bool {
        self.bypass
    }

    /// For tests: the accumulate pass writes its history weight ([`Fsr2::history_weight`])
    /// from the next frame on; the resources are replaced and the history dropped by the
    /// frame's next size change, so call it before [`crate::Frame::set_sizes`].
    pub fn set_debug(&mut self, device: &wgpu::Device, debug: bool) {
        if debug != self.debug {
            self.debug = debug;
            self.res = Resources::new(device, self.res.render, self.res.display, debug);
        }
    }

    /// Bytes FSR's resources occupy.
    pub fn bytes(&self) -> u64 {
        self.res.bytes()
    }

    /// The internal colour the composite writes in FSR mode.
    pub(crate) fn colour(&self) -> &wgpu::Texture {
        &self.res.colour
    }

    /// The input pass's device depth (r32float, render size).
    pub fn depth(&self) -> &wgpu::Texture {
        &self.res.depth
    }

    /// The input pass's motion vectors (rg32float, render size, UV to the previous frame).
    pub fn motion(&self) -> &wgpu::Texture {
        &self.res.motion
    }

    /// The input pass's masks (rgba8unorm, render size: x reactive, y the
    /// transparency-and-composition mask, z the edit flag).
    pub fn reactive(&self) -> &wgpu::Texture {
        &self.res.reactive
    }

    /// The composite's linear colour FSR reads (rgba32float, render size).
    pub fn colour_in(&self) -> &wgpu::Texture {
        &self.res.colour
    }

    /// The exposure texel FSR reads (rg32float 1 x 1, x).
    pub fn exposure(&self) -> &wgpu::Texture {
        &self.res.exposure
    }

    /// The depth clip's prepared colour (rgba16float, render size: YCoCg, the depth clip in a).
    pub fn prepared(&self) -> &wgpu::Texture {
        &self.res.prepared
    }

    /// The internal upscaled colour copy `i` (rgba16float, display size).
    pub fn upscaled(&self, i: usize) -> &wgpu::Texture {
        &self.res.upscaled[i]
    }

    /// The lock status copy `i` (rg32float, display size: lifetime, temporal luma).
    pub fn lock_status(&self, i: usize) -> &wgpu::Texture {
        &self.res.lock_status[i]
    }

    /// The accumulate pass's history weight of the last encoded frame (r32float, display
    /// size with [`Fsr2::set_debug`]).
    pub fn history_weight(&self) -> &wgpu::Texture {
        &self.res.weight
    }

    /// FSR's constants for a frame of `params` (its first view), `reset` when history is
    /// dropped.
    fn constants(&self, params: &Params, reset: bool) -> Fsr2Constants {
        let render = self.res.render.map(|n| n as i32);
        let display = self.res.display.map(|n| n as i32);
        let v = &params.views[0];
        let tan_h = (v.tan_left + v.tan_right) * 0.5;
        let tan_v = (v.tan_up + v.tan_down) * 0.5;
        let mut prev = self
            .pre_exposure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let pre_exposure = 1.0;
        let [w, h] = self.res.render;
        let c = Fsr2Constants {
            render_size: render,
            max_render_size: render,
            display_size: display,
            input_color_size: render,
            luma_mip_size: render.map(|n| (n as f32 / (2 << SHADING_CHANGE_MIP) as f32) as i32),
            luma_mip_level: SHADING_CHANGE_MIP,
            frame_index: if reset { 0 } else { 1 },
            // setupDeviceDepthToViewSpaceDepthParams for inverted, infinite depth.
            device_to_view: [-f32::EPSILON, NEAR, tan_h, tan_v],
            // The sample of render pixel p lies at p + 0.5 - jitter (see `jitter_offset`).
            jitter: [-v.jitter[0], -v.jitter[1]],
            motion_vector_scale: [1.0, 1.0],
            downscale: [
                render[0] as f32 / display[0] as f32,
                render[1] as f32 / display[1] as f32,
            ],
            motion_vector_jitter_cancellation: [0.0; 2],
            pre_exposure,
            previous_pre_exposure: if reset { pre_exposure } else { *prev },
            tan_half_fov: tan_h,
            jitter_sequence_length: jitter_phases(w, self.res.display[0]) as f32,
            delta_time: 1.0 / 60.0,
            dynamic_res_change: 0.0,
            view_space_to_meters: self.settings.metres_per_voxel,
            // SpdSetup over the whole render rectangle.
            spd_mips: ((w.max(h) as f32).log2().floor() as u32).min(12),
            spd_work_groups: w.div_ceil(64) * h.div_ceil(64),
            // FsrRcasCon of `-2 sharpness + 2` stops.
            rcas_con: (-(2.0 - 2.0 * self.settings.sharpness.clamp(0.0, 1.0))).exp2(),
            debug: u32::from(self.debug),
            _pad: 0,
        };
        *prev = pre_exposure;
        c
    }

    /// Writes this frame's constants and builds its bind groups from the frame's targets.
    pub(crate) fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        params: &Params,
        f: &FrameInputs<'_>,
    ) -> Fsr2Binds {
        let reset = params.history & 2 == 0;
        let c = self.constants(params, reset);
        queue.write_buffer(&self.cb, 0, bytemuck::bytes_of(&c));
        queue.write_buffer(
            &self.input_tuning,
            0,
            bytemuck::bytes_of(&InputUniform {
                near: NEAR,
                reactive_translucent: self.settings.reactive_translucent,
                reactive_light: self.settings.reactive_light,
                _pad: 0.0,
            }),
        );
        let r = &self.res;
        // The exposure seam: the frame's manual exposure, pre-exposure 1 (no eye adaptation).
        queue.write_texture(
            r.exposure.as_image_copy(),
            bytemuck::cast_slice(&[params.exposure, 0.0]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: None,
                rows_per_image: None,
            },
            r.exposure.size(),
        );
        let (w, q) = (f.cur, f.cur ^ 1);
        use Bind::{Buf, Tex};
        let cb = bind(device, &self.cb_layout, &[Buf(&self.cb)]);
        let input = bind(
            device,
            &self.input.layout,
            &[
                Tex(f.hit_pos),
                Tex(f.hit_info),
                Tex(f.layer),
                Buf(&self.input_tuning),
                Buf(f.edits),
                Buf(f.lamp),
                Tex(&r.depth),
                Tex(&r.motion),
                Tex(&r.reactive),
            ],
        );
        let spd_mips = bind(
            device,
            &self.spd_mips.layout,
            &[Tex(&r.colour), Tex(&r.luma_mip), Tex(&r.luma_mip5)],
        );
        let spd_tail = bind_at(
            device,
            &self.spd_tail.layout,
            &[
                (3, Tex(&r.luma_mip5)),
                (4, Tex(&r.auto_exposure[w])),
                (5, Tex(&r.auto_exposure[q])),
            ],
        );
        let reconstruct = bind(
            device,
            &self.reconstruct.layout,
            &[
                Tex(&r.depth),
                Tex(&r.motion),
                Tex(&r.colour),
                Tex(&r.exposure),
                Buf(&r.reconstructed),
                Tex(&r.dilated_depth),
                Tex(&r.dilated_motion[w]),
                Tex(&r.lock_luma),
            ],
        );
        let depth_clip = bind(
            device,
            &self.depth_clip.layout,
            &[
                Tex(&r.depth),
                Tex(&r.motion),
                Tex(&r.colour),
                Tex(&r.reactive),
                Tex(&r.exposure),
                Tex(&r.dilated_depth),
                Tex(&r.dilated_motion[w]),
                Tex(&r.dilated_motion[q]),
                Buf(&r.reconstructed),
                Tex(&r.prepared),
                Tex(&r.dilated_reactive),
            ],
        );
        let lock = bind(
            device,
            &self.lock.layout,
            &[Tex(&r.lock_luma), Buf(&r.new_locks), Buf(&r.reconstructed)],
        );
        let accumulate = bind(
            device,
            &self.accumulate.layout,
            &[
                Tex(&r.exposure),
                Tex(&r.dilated_reactive),
                Tex(&r.dilated_motion[w]),
                Tex(&r.upscaled[q]),
                Tex(&r.lock_status[q]),
                Tex(&r.prepared),
                Tex(&r.luma_mip),
                Tex(&r.luma_history[q]),
                Buf(&r.new_locks),
                Tex(&r.lock_status[w]),
                Tex(&r.upscaled[w]),
                Tex(&r.luma_history[w]),
                Tex(&r.weight),
            ],
        );
        // RCAS writes into last frame's copy of the upscaled colour: the accumulate pass has
        // read it, and the next frame's accumulate pass overwrites it.
        let sharpening = self.settings.sharpness > 0.0;
        let rcas = sharpening.then(|| {
            (
                bind(
                    device,
                    &self.rcas.layout,
                    &[Tex(&r.upscaled[w]), Tex(&r.exposure), Tex(&r.upscaled[q])],
                ),
                r.display.map(|n| n.div_ceil(16)),
            )
        });
        let tone_in = if self.bypass {
            &r.colour
        } else if sharpening {
            &r.upscaled[q]
        } else {
            &r.upscaled[w]
        };
        let tone = bind(device, &self.tone.layout, &[Tex(tone_in), Tex(f.output)]);
        let [rw, rh] = r.render;
        Fsr2Binds {
            cb,
            input,
            spd_mips,
            spd_tail,
            reconstruct,
            depth_clip,
            lock,
            accumulate,
            rcas,
            tone,
            bypass: self.bypass,
            render_groups: r.render.map(|n| n.div_ceil(WORKGROUP)),
            display_groups: r.display.map(|n| n.div_ceil(WORKGROUP)),
            spd_groups: [rw.div_ceil(64), rh.div_ceil(64)],
        }
    }
}

/// The frame's targets FSR reads and writes: this frame's hit buffer and translucent layers,
/// the edit boxes and the upscale pass's tuning uniforms (the older lamp), the frame's rgba8
/// image, and the copy of the double-buffered resources this frame writes (the other is last
/// frame's).
pub(crate) struct FrameInputs<'a> {
    pub hit_pos: &'a wgpu::Texture,
    pub hit_info: &'a wgpu::Texture,
    pub layer: &'a wgpu::Texture,
    pub edits: &'a wgpu::Buffer,
    pub lamp: &'a wgpu::Buffer,
    pub output: &'a wgpu::Texture,
    pub cur: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FSR's phase counts: 8 at scale 1, 18 at 1.5 and 32 at 2; and its first four offsets.
    #[test]
    fn the_jitter_sequence_is_fsrs() {
        assert_eq!(jitter_phases(384, 384), 8);
        assert_eq!(jitter_phases(256, 384), 18);
        assert_eq!(jitter_phases(192, 384), 32);
        assert_eq!(jitter_phases(1280, 1920), 18);
        let want = [
            [0.0, -1.0 / 6.0],
            [-0.25, 1.0 / 6.0],
            [0.25, -7.0 / 18.0],
            [-0.375, -1.0 / 18.0],
        ];
        for (i, w) in want.iter().enumerate() {
            let j = jitter_offset(i as u32, 18);
            for k in 0..2 {
                assert!(
                    (j[k] - w[k]).abs() < 1e-6,
                    "offset {i}: {j:?} against {w:?}"
                );
            }
        }
        assert_eq!(jitter_offset(18, 18), jitter_offset(0, 18));
    }
}
