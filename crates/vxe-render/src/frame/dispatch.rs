//! A frame pass's dispatches, held in a fixed array: a frame records them with no allocation.

use super::ATROUS_ITERATIONS;

/// One dispatch of a frame pass: its pipeline, the bind groups at indices 0 to 3, and the
/// workgroups in x and y.
#[derive(Clone, Copy)]
pub(crate) struct Dispatch<'a> {
    pub pipeline: &'a wgpu::ComputePipeline,
    pub groups: [Option<&'a wgpu::BindGroup>; 4],
    pub workgroups: [u32; 2],
    /// Dispatched with the workgroup counts this buffer holds at this byte offset instead.
    pub indirect: Option<(&'a wgpu::Buffer, u64)>,
}

impl Dispatch<'_> {
    /// Sets the pipeline and the bind groups on `pass` and dispatches.
    pub(super) fn record(&self, pass: &mut wgpu::ComputePass<'_>) {
        pass.set_pipeline(self.pipeline);
        for (g, bind) in self.groups.iter().enumerate() {
            if let Some(b) = bind {
                pass.set_bind_group(g as u32, *b, &[]);
            }
        }
        match self.indirect {
            Some((args, offset)) => pass.dispatch_workgroups_indirect(args, offset),
            None => pass.dispatch_workgroups(self.workgroups[0], self.workgroups[1], 1),
        }
    }
}

/// Dispatches one pass records at most: the variance pass and every a-trous iteration.
pub(crate) const PASS_DISPATCHES: usize = 1 + ATROUS_ITERATIONS;

/// The dispatches of one frame pass, in order, at most [`PASS_DISPATCHES`].
#[derive(Clone, Copy, Default)]
pub(crate) struct Dispatches<'a> {
    list: [Option<Dispatch<'a>>; PASS_DISPATCHES],
    len: usize,
}

impl<'a> Dispatches<'a> {
    /// The pass of `d` alone.
    pub(crate) fn one(d: Dispatch<'a>) -> Self {
        let mut out = Self::default();
        out.push(d);
        out
    }

    /// Appends `d`; past [`PASS_DISPATCHES`] it panics, an invariant of the frame's passes.
    pub(crate) fn push(&mut self, d: Dispatch<'a>) {
        self.list[self.len] = Some(d);
        self.len += 1;
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Dispatch<'a>> {
        self.list[..self.len].iter().flatten()
    }
}
