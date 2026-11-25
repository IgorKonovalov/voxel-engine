//! The encode's filter and output bind groups: the a-trous passes', the composite's and the
//! gradient's.

use super::*;

/// The composite's pipeline and the output passes' bind groups.
pub(super) struct OutputBinds<'a> {
    pub(super) composite: &'a wgpu::ComputePipeline,
    pub(super) composite_bind: wgpu::BindGroup,
    pub(super) gradient_bind: wgpu::BindGroup,
}

impl Frame {
    /// The filter's bind groups: whether it runs, the variance pass's and each a-trous
    /// iteration's.
    pub(super) fn filter_binds(
        &self,
        device: &wgpu::Device,
    ) -> (bool, wgpu::BindGroup, Vec<wgpu::BindGroup>) {
        let t = &self.targets;
        let c = self.cur;
        let filtering = self.state.lighting.filter && self.state.look.view.shades();
        let variance_bind = self.binds.io_at(
            device,
            self.pipes.layout(P_VARIANCE),
            &[0, 1, 2, 3, 4, 9, 10, 11, 12],
            &[
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
                Res::Tex(&t.accum[c]),
                Res::Tex(&t.moments[c]),
                Res::Tex(&t.filter[0]),
                Res::Tex(&t.direct_accum[c]),
                Res::Tex(&t.vis),
                Res::Tex(&t.pair[c]),
                Res::Buf(&self.sky.irradiance),
            ],
        );
        let n = self.state.lighting.atrous_count();
        let atrous_binds: Vec<wgpu::BindGroup> = (0..n)
            .map(|i| {
                self.binds.io_at(
                    device,
                    self.pipes.layout(P_ATROUS),
                    &[0, 1, 2, 5, 6, 7, 8, 10],
                    &[
                        Res::Tex(t.lit_pos(c)),
                        Res::Tex(t.lit_info(c)),
                        Res::Tex(&t.filter[i % 2]),
                        Res::Tex(&t.filter[(i + 1) % 2]),
                        Res::Buf(&self.state.atrous_steps[i][usize::from(i + 1 == n)]),
                        Res::Tex(&t.accum[c]),
                        Res::Tex(&t.moments[c]),
                        Res::Tex(&t.vis),
                    ],
                )
            })
            .collect();
        (filtering, variance_bind, atrous_binds)
    }

    /// The bind groups of the composite (and its pipeline, FSR 2's variant in FSR mode) and
    /// the gradient.
    pub(super) fn output_binds(&self, device: &wgpu::Device, filtering: bool) -> OutputBinds<'_> {
        let t = &self.targets;
        let (c, q) = (self.cur, self.cur ^ 1);
        let lit = if filtering {
            &t.filter[self.state.lighting.atrous_count() % 2]
        } else {
            &t.accum[c]
        };
        // In FSR mode the composite writes FSR's colour input at the internal size.
        let composite_out = self.up.fsr.as_ref().map_or(&t.output, Fsr2::colour);
        // FSR 2's composite stores linear radiance (`composite_hdr`).
        let (composite_layout, composite) = self
            .up
            .composite_hdr
            .as_ref()
            .filter(|_| self.up.fsr.is_some())
            .map_or(
                (
                    self.pipes.layout(P_COMPOSITE),
                    self.pipes.pipeline(device, P_COMPOSITE),
                ),
                |(l, p)| (l, p),
            );
        let composite_bind = self.binds.io(
            device,
            composite_layout,
            &[
                Res::Tex(&t.hit_pos[c]),
                Res::Tex(&t.hit_info[c]),
                Res::Tex(&t.layer),
                Res::Tex(lit),
                Res::Tex(composite_out),
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
                Res::Tex(&t.indirect),
                Res::Buf(&t.tiles),
                Res::Tex(&t.transmit),
                Res::Tex(&t.direct_accum[c]),
                Res::Tex(&t.vis),
                Res::Tex(&t.pair[c]),
                Res::Buf(&self.sky.irradiance),
                Res::Buf(self.entities.marks_buffer()),
            ],
        );
        let gradient_bind = self.binds.io(
            device,
            self.pipes.layout(P_GRADIENT),
            &[
                Res::Tex(t.lit_pos(q)),
                Res::Tex(t.lit_info(q)),
                Res::Tex(&t.direct),
                Res::Tex(&t.strata),
            ],
        );
        // The gradient pass runs in a shaded view for the gradient mode's lambda.
        OutputBinds {
            composite,
            composite_bind,
            gradient_bind,
        }
    }
}
