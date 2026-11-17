//! The models the placed entities need, kept in an entity pool: the frame's mirror
//! ([`MirrorPool`]) or the bookkeeping alone in a test, with the free, retry and reset path of
//! a full or fragmented pool ([`sync_models`]), and one frame's feed ([`feed`]).

use vxe_entity::Model;
use vxe_render::{EntityUploadError, ModelHandle};
use vxe_world::{Fixed3, MaterialTable};

use super::Walkers;

/// What [`sync_models`] needs of an entity pool: the frame's mirror with its queue
/// ([`MirrorPool`]), or the bookkeeping alone ([`vxe_render::internals::VolumePool`]) in a test.
pub trait ModelPool {
    fn upload(
        &mut self,
        model: &Model,
        table: &MaterialTable,
    ) -> Result<ModelHandle, EntityUploadError>;
    /// Frees `handle`'s model; a handle the pool does not know is ignored.
    fn free(&mut self, handle: ModelHandle);
    fn clear(&mut self);
    fn used_bytes(&self) -> u64;
    fn total_bytes(&self) -> u64;
}

impl ModelPool for vxe_render::internals::VolumePool {
    fn upload(
        &mut self,
        model: &Model,
        table: &MaterialTable,
    ) -> Result<ModelHandle, EntityUploadError> {
        vxe_render::internals::VolumePool::upload(self, model, table).map(|u| u.handle)
    }

    fn free(&mut self, handle: ModelHandle) {
        let _ = self.free_model(handle);
    }

    fn clear(&mut self) {
        vxe_render::internals::VolumePool::clear(self);
    }

    fn used_bytes(&self) -> u64 {
        vxe_render::internals::VolumePool::used_bytes(self)
    }

    fn total_bytes(&self) -> u64 {
        vxe_render::internals::VolumePool::total_bytes(self)
    }
}

/// The frame's entity mirror and the queue its uploads are written through.
pub struct MirrorPool<'a> {
    pub mirror: &'a mut vxe_render::EntityMirror,
    pub queue: &'a vxe_render::wgpu::Queue,
}

impl ModelPool for MirrorPool<'_> {
    fn upload(
        &mut self,
        model: &Model,
        table: &MaterialTable,
    ) -> Result<ModelHandle, EntityUploadError> {
        self.mirror.upload_model(self.queue, model, table)
    }

    fn free(&mut self, handle: ModelHandle) {
        let _ = self.mirror.free_model(handle);
    }

    fn clear(&mut self) {
        self.mirror.clear();
    }

    fn used_bytes(&self) -> u64 {
        self.mirror.pool_used_bytes()
    }

    fn total_bytes(&self) -> u64 {
        self.mirror.pool_total_bytes()
    }
}

/// The banner's words for a pool [`sync_models`] cleared and filled again.
pub const POOL_RESET: &str = "entity pool reset";

/// Whether `e` is a refusal a cleared pool may not repeat: the free bytes hold the model, no
/// hole does.
fn fragmented(e: &EntityUploadError) -> bool {
    matches!(*e, EntityUploadError::PoolFull { needed, free, largest }
        if largest < needed && needed <= free)
}

/// Uploads `model` into `pool`; on `PoolFull` frees `stale` (the viewer's replaced model, when
/// there is one) and tries once more.
fn upload_or_free_stale(
    pool: &mut impl ModelPool,
    model: &Model,
    table: &MaterialTable,
    stale: &mut Option<ModelHandle>,
) -> Result<ModelHandle, EntityUploadError> {
    match pool.upload(model, table) {
        Err(EntityUploadError::PoolFull { .. }) if stale.is_some() => {
            pool.free(stale.take().expect("checked"));
            pool.upload(model, table)
        }
        r => r,
    }
}

/// One pass of [`sync_models`]: the built-in walker's model and the viewer's uploaded when
/// needed, the viewer's replaced model freed once its new one is in. The first refusal and
/// whose it was (`true` the viewer's).
fn upload_needed(
    walkers: &mut Walkers,
    pool: &mut impl ModelPool,
    table: &MaterialTable,
) -> Result<(), (bool, EntityUploadError)> {
    let mut no_stale = None;
    if walkers.handle.is_none()
        && walkers.placed.iter().any(|p| !p.rig)
        && let Some(model) = walkers.model(table)
    {
        let stale = walkers
            .viewer
            .as_mut()
            .map_or(&mut no_stale, |v| &mut v.stale);
        walkers.handle =
            Some(upload_or_free_stale(pool, &model, table, stale).map_err(|e| (false, e))?);
    }
    if let Some(v) = &mut walkers.viewer
        && v.handle.is_none()
        && let Some(l) = &v.loaded
    {
        let model = l.model.clone();
        v.handle =
            Some(upload_or_free_stale(pool, &model, table, &mut v.stale).map_err(|e| (true, e))?);
    }
    if let Some(v) = &mut walkers.viewer
        && v.handle.is_some()
        && let Some(s) = v.stale.take()
    {
        pool.free(s);
    }
    Ok(())
}

/// The models the placed walkers need, in `pool`: the built-in walker's on first need, the
/// viewer's after a build or a reload, then the viewer's replaced model freed. A `PoolFull`
/// first frees the replaced model and retries; one that stands with the free bytes enough and
/// no hole large enough clears the pool and uploads every model still placed again (the
/// banner's [`POOL_RESET`]). A refusal after that keeps the viewer's error on screen (or prints
/// the walker's). The pool's figures are read for the HUD row. The banner's words when there
/// are any.
pub fn sync_models(
    walkers: &mut Walkers,
    pool: &mut impl ModelPool,
    table: &MaterialTable,
) -> Option<String> {
    let mut words = None;
    let mut r = upload_needed(walkers, pool, table);
    if let Err((_, e)) = &r
        && fragmented(e)
    {
        pool.clear();
        walkers.handle = None;
        if let Some(v) = &mut walkers.viewer {
            (v.handle, v.stale) = (None, None);
        }
        words = Some(POOL_RESET.to_owned());
        r = upload_needed(walkers, pool, table);
    }
    match r {
        Ok(()) => {}
        Err((false, e)) => println!("walker: {e}"),
        Err((true, e)) => {
            if let Some(v) = &mut walkers.viewer {
                // Tried with no model: the next try follows a reload.
                let err = format!("model {}: {e}", v.name);
                v.loaded = None;
                v.error = Some(e.to_string());
                words = Some(err);
            }
        }
    }
    (walkers.pool_used, walkers.pool_total) = (pool.used_bytes(), pool.total_bytes());
    words
}

/// One frame's walkers into `frame`: the pending ring placed around `eye` on `world`'s floor,
/// the models synced ([`sync_models`]), and the frame's entities set while any walker is
/// placed, was drawn the frame before, or the mirror still walks a ghost leaf; the drawn counts copied back for the HUD. The
/// banner's words from the sync, when there are any.
pub fn feed(
    walkers: &mut Walkers,
    world: &vxe_world::World,
    eye: [f32; 3],
    frame: &mut vxe_render::Frame,
    queue: &vxe_render::wgpu::Queue,
) -> Option<String> {
    if walkers.ring > 0 {
        let floor_below = |p: [f32; 3], search: i64| {
            let o = Fixed3(p.map(|v| (v * 256.0).round() as i64));
            let hit = world
                .raycast(o, [0, 0, -256], search * 256)
                .ok()
                .flatten()?;
            Some(hit.t as f32 / 256.0)
        };
        walkers.place_ring(world.materials(), eye, floor_below);
    }
    // A removal leaves a ghost leaf until the next write, so the frames keep writing while the
    // mirror still walks one.
    if walkers.placed.is_empty()
        && walkers.drawn == 0
        && walkers.over == 0
        && !frame.entity_mirror().walks_entities()
    {
        return None;
    }
    let words = sync_models(
        walkers,
        &mut MirrorPool {
            mirror: frame.entity_mirror_mut(),
            queue,
        },
        world.materials(),
    );
    walkers.refresh_displays();
    frame.set_entity_displays(
        queue,
        walkers.displays().iter().map(|(id, h, d)| (*id, *h, d)),
    );
    let m = frame.entity_mirror();
    (walkers.drawn, walkers.parts, walkers.over) =
        (m.entities_drawn(), m.parts_drawn(), m.entities_over_cap());
    words
}
