//! [`WorldHost`]: a [`Host`] straight on a world, answering every query from it. The server's
//! tick host reads through one; a logic can also be driven by one without a server.

use vxe_world::{
    Anchors, CellChange, CellCoord, CoarseGrid, EditError, Fixed3, Hit, MaterialId, Op, OpResult,
    PrefabRegistry, Region, SupportParams, VoxelCoord, World, find_unsupported_with_stats,
};

use crate::logic::Host;
use crate::server::max_changed_cells;

/// What the support query reads besides the world: the coarse grid, the parameters and the
/// explicit anchors.
#[derive(Clone, Copy, Debug)]
struct SupportView<'a> {
    grid: &'a CoarseGrid,
    params: &'a SupportParams,
    anchors: &'a [CellCoord],
}

/// A [`Host`] on a world. `apply` applies the op to the world with the host's prefabs (none
/// unless [`WorldHost::with_prefabs`]) and keeps no log and no grid current. The support query
/// runs only with [`WorldHost::with_support`]; without it `unsupported` reads no regions.
#[derive(Clone, Copy, Debug)]
pub struct WorldHost<'a> {
    world: &'a World,
    prefabs: Option<&'a PrefabRegistry>,
    support: Option<SupportView<'a>>,
}

impl<'a> WorldHost<'a> {
    /// A host on `world` with no prefabs and no support query.
    pub fn new(world: &'a World) -> Self {
        Self {
            world,
            prefabs: None,
            support: None,
        }
    }

    /// The same host, applying `Stamp` ops from `prefabs`.
    pub fn with_prefabs(self, prefabs: &'a PrefabRegistry) -> Self {
        Self {
            prefabs: Some(prefabs),
            ..self
        }
    }

    /// The same host, answering the support query from `grid` with `params` and the explicit
    /// `anchors`. The grid is read as it stands: the caller keeps it current.
    pub fn with_support(
        self,
        grid: &'a CoarseGrid,
        params: &'a SupportParams,
        anchors: &'a [CellCoord],
    ) -> Self {
        Self {
            support: Some(SupportView {
                grid,
                params,
                anchors,
            }),
            ..self
        }
    }

    /// The world this host answers from.
    pub fn world(&self) -> &'a World {
        self.world
    }
}

impl Host for WorldHost<'_> {
    fn apply(&mut self, op: Op) -> Result<OpResult, EditError> {
        match self.prefabs {
            Some(p) => op.apply(self.world, p),
            None => op.apply(self.world, &PrefabRegistry::new()),
        }
    }

    fn material_at(&self, v: VoxelCoord) -> Option<MaterialId> {
        self.world.material_at(v)
    }

    /// A ray the world refuses as out of range reads as no hit.
    fn raycast(&self, origin: Fixed3, dir: Fixed3, max: i64) -> Option<Hit> {
        self.world.raycast(origin, dir.0, max).ok().flatten()
    }

    fn raycast_with_steps(&self, origin: Fixed3, dir: Fixed3, max: i64) -> (Option<Hit>, u64) {
        self.world
            .raycast_with_stats(origin, dir.0, max)
            .unwrap_or((None, 0))
    }

    /// A box the world refuses as out of range reads as an empty count.
    fn count_in_box(&self, lo: VoxelCoord, hi: VoxelCoord) -> Vec<(MaterialId, u64)> {
        self.world.count_in_box(lo, hi).unwrap_or_default()
    }

    fn unsupported(&self, changed: &[CellCoord]) -> Vec<Region> {
        self.unsupported_with_steps(changed).0
    }

    /// No regions and 0 steps without [`WorldHost::with_support`]. A `changed` list longer
    /// than one in-range edit can change (see [`max_changed_cells`]) reads as no regions, and
    /// the query does not run.
    fn unsupported_with_steps(&self, changed: &[CellCoord]) -> (Vec<Region>, u64) {
        let Some(s) = self.support else {
            return (Vec::new(), 0);
        };
        if changed.len() as u64 > max_changed_cells(self.world) {
            return (Vec::new(), 0);
        }
        // The query reads only each change's cell; the grid holds the cells' current state.
        let changes: Vec<CellChange> = changed
            .iter()
            .map(|&c| {
                let (chunk, slot) = c.split();
                let now = s.grid.cell(chunk, slot).unwrap_or_default();
                CellChange {
                    chunk,
                    slot: slot as u16,
                    before: now,
                    after: now,
                }
            })
            .collect();
        let anchors = Anchors {
            explicit: s.anchors,
        };
        find_unsupported_with_stats(self.world, s.grid, s.params, &anchors, &changes)
    }
}
