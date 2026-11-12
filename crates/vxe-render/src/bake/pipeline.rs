//! The bake's settings, its pipeline and the shader source it composes.

use super::*;

/// The sampler, compiled after [`crate::MARCH_WGSL`] and [`crate::LIGHT_WGSL`].
pub(crate) const BAKE_WGSL: &str = include_str!("../shaders/bake.wgsl");

/// Samples a face takes between two looks at its stop.
pub const BAKE_ROUND: u32 = 1024;

/// Entries (one round of one face) one submit's working buffer holds at most.
pub(crate) const BAKE_BATCH_FACES: usize = 65_536;

/// Jobs open at once, each with a slot of the tables buffer.
pub(crate) const OPEN_JOBS: usize = 256;

/// The bytes of the job uniform (`BakeJob` in `bake.wgsl`).
pub(super) const JOB_BYTES: u64 = 32;

/// A slot's 16-byte words (`TABLE_SLOT` in `bake.wgsl`): the candidate table (a header, then
/// [`LIGHT_TABLE_ENTRIES`] entries), then the job's chunk.
pub(super) const TABLE_SLOT_WORDS: u64 = 2 + LIGHT_TABLE_ENTRIES as u64;

pub(super) const TABLE_SLOT_BYTES: u64 = TABLE_SLOT_WORDS * 16;

const _: () = assert!(TABLE_SLOT_WORDS == 1026, "TABLE_SLOT in bake.wgsl");

/// Samples one submit holds at most, so the driver sees short dispatches with a wait between.
pub(crate) const SUBMIT_SAMPLES: u64 = 10_000_000;

/// The bake's settings. The default is the brick level the owner picked (Plan 0059 Phase 7b):
/// cells of [`BRICK_CELL`] voxels, [`QUEUE_BRICK_SAMPLES`] a brick direction,
/// [`KEPT_CANDIDATES`] candidates, the clamp as [`KEPT_CLAMP`].
#[derive(Clone)]
pub struct BakeSettings {
    /// Emissive candidates a sample resamples.
    pub candidates: u32,
    /// The estimate is clamped at the light pass's `FIREFLY_CLAMP`.
    pub clamp: bool,
    /// The random sequences' seed.
    pub seed: u32,
    /// Samples between two looks at a face's stop.
    pub round: u32,
    /// The stop's standard error, in display levels.
    pub se: f32,
    /// Entries (one round of one face) a submit holds at most; `SUBMIT_SAMPLES / round` and
    /// [`BAKE_BATCH_FACES`] cap it too.
    pub batch_faces: usize,
    /// Rounds one face takes in a submit at most; 0 is as many as the submit's room allows.
    pub submit_rounds: u32,
    /// A submit holds one job's faces only.
    pub jobs_alone: bool,
    /// The chunks in descending coordinate order, for the test that a result ignores the order.
    pub reversed: bool,
    /// [`BakeReport::faces`] holds every face.
    pub keep_faces: bool,
    /// Sample counts at which [`FaceOutcome::snapshots`] take the running means.
    pub snapshots: Vec<u32>,
    /// Bakes only the faces this passes (a reading's sphere); the others stay 0.
    pub filter: Option<FaceFilter>,
    /// Times each submit's dispatch on the GPU into [`BakeReport::dispatch_seconds`] (a reading's
    /// cost rows).
    pub timer: Option<Arc<crate::frame::PassTimer>>,
    /// The brick records' cell in voxels: 8 (the brick) or 4.
    pub cell: u32,
    /// Samples a brick direction draws, spread over its faces (at least one a face).
    pub brick_samples: u32,
}

/// Candidates a sample resamples in the reference: the arm the count reading keeps.
pub(crate) const KEPT_CANDIDATES: u32 = 32;

/// Whether the reference clamps a sample: the arm the count reading keeps.
pub(crate) const KEPT_CLAMP: bool = false;

impl Default for BakeSettings {
    fn default() -> Self {
        Self {
            candidates: KEPT_CANDIDATES,
            clamp: KEPT_CLAMP,
            seed: 0,
            round: BAKE_ROUND,
            se: BAKE_SE,
            batch_faces: BAKE_BATCH_FACES,
            submit_rounds: 0,
            jobs_alone: false,
            reversed: false,
            keep_faces: false,
            snapshots: Vec::new(),
            filter: None,
            timer: None,
            cell: BRICK_CELL,
            brick_samples: QUEUE_BRICK_SAMPLES,
        }
    }
}

/// The compute pipeline and the buffers of a bake, built on the first bake.
pub(crate) struct BakePipeline {
    pub(super) pipeline: wgpu::ComputePipeline,
    pub(super) job: wgpu::Buffer,
    pub(super) faces: wgpu::Buffer,
    /// [`OPEN_JOBS`] slots of [`TABLE_SLOT_BYTES`]: an open job's candidate table and chunk.
    pub(super) tables: wgpu::Buffer,
    /// Group 3 over the three buffers above, which the pipeline never replaces.
    pub(super) bind: wgpu::BindGroup,
}

/// The light pass's table read, which the bake's module reads from the face's slot instead.
pub(super) const TABLE_READ: &str =
    "return bitcast<vec4<u32>>(light_samples[params.light_table_base + i]);";

/// The LOD walk's camera chunk, which the bake's module takes from the face's job instead.
pub(super) const LOD_ORIGIN: &str =
    "let d = chunk - floor_div(vec3<i32>(floor(params.views[0].origin)), 64);";

pub(super) fn replace_once(source: &str, from: &str, to: &str) -> String {
    assert_eq!(
        source.matches(from).count(),
        1,
        "the bake's module replaces `{from}` once"
    );
    source.replace(from, to)
}

/// The bake's module: march.wgsl and light.wgsl, with the candidate table and the LOD walk's
/// camera chunk read from the face's job slot (`bake_table_slot`, `bake_origin_chunk`), so one
/// dispatch serves faces of many jobs; then bake.wgsl.
pub(crate) fn bake_source() -> String {
    let march = replace_once(
        crate::MARCH_WGSL,
        LOD_ORIGIN,
        "let d = chunk - bake_origin_chunk();",
    );
    let light = replace_once(crate::LIGHT_WGSL, TABLE_READ, "return bake_table_slot(i);");
    format!("{march}\n{light}\n{BAKE_WGSL}")
}

impl BakePipeline {
    pub(crate) fn new(device: &wgpu::Device, world_layout: &wgpu::BindGroupLayout) -> Self {
        let layout = io_layout(device, "bake", &[Io::Uniform, Io::Storage, Io::ReadStorage]);
        let job = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bake job"),
            size: JOB_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tables = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bake job tables"),
            size: OPEN_JOBS as u64 * TABLE_SLOT_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let faces = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bake faces"),
            size: BAKE_BATCH_FACES as u64 * FACE_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pipeline = compute_pipeline_with(
            device,
            "bake",
            bake_source(),
            &[Some(world_layout), None, None, Some(&layout)],
            "bake_round",
            &[("RAND_HASHED", 1.0), ("EMISSION_UNIT", 1.0)],
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bake"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: job.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: faces.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tables.as_entire_binding(),
                },
            ],
        });
        Self {
            pipeline,
            job,
            faces,
            tables,
            bind,
        }
    }

    /// Group 3 of a bake's dispatch: the job uniform, the working buffer and the table slots.
    pub(crate) fn bind(&self) -> &wgpu::BindGroup {
        &self.bind
    }
}

/// The candidate table of the job at `coord` (ADR-0025, centred on the job's chunk): the chunks
/// holding a range whose box lies within `light_radius` voxels of the job's box, the nearest
/// [`LIGHT_TABLE_ENTRIES`], in chunk-coordinate order, each weighing its power over
/// `max(d^2, 64^2)`, `d` the distance between the boxes.
pub(crate) fn job_table(world: &GpuWorld, coord: ChunkCoord) -> LightTableUniform {
    let reach = i64::from(world.light_radius()) * 64;
    let lo = coord.0.map(|v| i64::from(v) * 64);
    let gap2 = |c: ChunkCoord| -> i64 {
        (0..3)
            .map(|k| {
                let o = i64::from(c.0[k]) * 64;
                let gap = (o - (lo[k] + 64)).max(lo[k] - (o + 64)).max(0);
                gap * gap
            })
            .sum()
    };
    let mut near: Vec<(i64, ChunkCoord, u32, u32, f32)> = world
        .light_chunks()
        .map(|(c, start, len, power)| (gap2(c), c, start, len, power))
        .filter(|&(d2, ..)| d2 < reach * reach)
        .collect();
    near.sort_unstable_by_key(|&(d2, c, ..)| (d2, c.0));
    near.truncate(LIGHT_TABLE_ENTRIES);
    near.sort_unstable_by_key(|&(_, c, ..)| c.0);
    let mut t = LightTableUniform {
        count: near.len() as u32,
        total: 0.0,
        _pad: [0; 2],
        chunks: [[0; 4]; LIGHT_TABLE_ENTRIES],
    };
    let mut cdf = 0f32;
    for (i, &(d2, _, start, len, power)) in near.iter().enumerate() {
        cdf += power / (d2.max(64 * 64) as f32);
        t.chunks[i] = [start, len, cdf.to_bits(), 0];
    }
    t.total = cdf;
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bake's module, with the table and LOD reads replaced, parses and validates.
    #[test]
    fn the_bake_module_reads_the_entrys_job() {
        let source = bake_source();
        assert!(!source.contains(TABLE_READ) && !source.contains(LOD_ORIGIN));
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("the bake module validates: {e:?}"));
    }
}
