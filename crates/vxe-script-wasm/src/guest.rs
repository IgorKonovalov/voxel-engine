//! The guest side: a logic crate built as a component exports the `logic` world with
//! [`export_logic!`](crate::export_logic), which forwards `init` and `tick` to its
//! [`Logic`] with an [`ImportHost`] that calls the host's imports.
//!
//! ```ignore
//! vxe_script_wasm::export_logic!(my_logic::MyLogic);
//! ```

use vxe_net::{ClientInput, Host, Logic};
use vxe_world::wire::encode_op;
use vxe_world::{
    CellCoord, EditError, Face, Fixed3, Hit, MaterialId, Op, OpResult, Region, VoxelCoord,
};

/// The generated bindings of the `logic` world.
pub mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "logic",
        pub_export_macro: true,
        export_macro_name: "export_logic_world",
        default_bindings_module: "vxe_script_wasm::guest::bindings",
    });
}

use bindings::vxe::logic::host as wit;

/// A [`Host`] whose every call is one of the `host` imports.
#[derive(Clone, Copy, Debug, Default)]
pub struct ImportHost;

fn wit_voxel(v: VoxelCoord) -> wit::Voxel {
    let [x, y, z] = v.0;
    wit::Voxel { x, y, z }
}

fn wit_fixed3(p: Fixed3) -> wit::Fixed3 {
    let [x, y, z] = p.0;
    wit::Fixed3 { x, y, z }
}

fn face(f: wit::Face) -> Face {
    match f {
        wit::Face::NegX => Face::NegX,
        wit::Face::PosX => Face::PosX,
        wit::Face::NegY => Face::NegY,
        wit::Face::PosY => Face::PosY,
        wit::Face::NegZ => Face::NegZ,
        wit::Face::PosZ => Face::PosZ,
    }
}

impl Host for ImportHost {
    fn apply(&mut self, op: Op) -> Result<OpResult, EditError> {
        let mut bytes = Vec::new();
        encode_op(&mut bytes, &op);
        let answer = wit::apply(&bytes);
        // The host decodes what `encode_op` wrote and answers in `encode_answer`'s format;
        // anything else is a host bug, and the panic traps.
        crate::decode_answer(&answer)
            .expect("the host's apply answer decodes")
            .expect("the host decodes an encoded op")
    }

    fn material_at(&self, v: VoxelCoord) -> Option<MaterialId> {
        wit::material_at(wit_voxel(v))
    }

    fn raycast(&self, origin: Fixed3, dir: Fixed3, max: i64) -> Option<Hit> {
        wit::raycast(wit_fixed3(origin), wit_fixed3(dir), max).map(|h| Hit {
            voxel: VoxelCoord([h.voxel.x, h.voxel.y, h.voxel.z]),
            face: h.face.map(face),
            t: h.t,
            material: h.material,
        })
    }

    fn count_in_box(&self, lo: VoxelCoord, hi: VoxelCoord) -> Vec<(MaterialId, u64)> {
        wit::count_in_box(wit_voxel(lo), wit_voxel(hi))
    }

    fn unsupported(&self, changed: &[CellCoord]) -> Vec<Region> {
        let changed: Vec<wit::Cell> = changed
            .iter()
            .map(|c| {
                let [x, y, z] = c.0;
                wit::Cell { x, y, z }
            })
            .collect();
        wit::unsupported(&changed)
            .into_iter()
            .map(|g| Region {
                cells: g
                    .cells
                    .into_iter()
                    .map(|c| CellCoord([c.x, c.y, c.z]))
                    .collect(),
                counts: g.counts,
                aabb: (voxel(g.lo), voxel(g.hi)),
            })
            .collect()
    }
}

fn voxel(v: wit::Voxel) -> VoxelCoord {
    VoxelCoord([v.x, v.y, v.z])
}

/// The export's `init`: `logic.init` on the imports.
pub fn init(logic: &mut impl Logic, seed: u64) {
    logic.init(&mut ImportHost, seed);
}

/// The export's `tick`: `logic.tick` on the imports, its broadcast returned.
pub fn tick(logic: &mut impl Logic, tick: u64, inputs: Vec<bindings::ClientInput>) -> Vec<u8> {
    let inputs: Vec<ClientInput> = inputs
        .into_iter()
        .map(|i| ClientInput {
            client: i.client,
            bytes: i.bytes,
        })
        .collect();
    logic.tick(&mut ImportHost, tick, &inputs).broadcast
}

/// Exports the `logic` world for a [`Logic`] type that is `Default`: one instance, made on
/// first use, receives every `init` and `tick`.
#[macro_export]
macro_rules! export_logic {
    ($logic:ty) => {
        const _: () = {
            struct Component;

            ::std::thread_local! {
                static LOGIC: ::core::cell::RefCell<$logic> =
                    ::core::cell::RefCell::new(<$logic as ::core::default::Default>::default());
            }

            impl $crate::guest::bindings::Guest for Component {
                fn init(seed: u64) {
                    LOGIC.with_borrow_mut(|l| $crate::guest::init(l, seed));
                }

                fn tick(
                    tick: u64,
                    inputs: ::std::vec::Vec<$crate::guest::bindings::ClientInput>,
                ) -> ::std::vec::Vec<u8> {
                    LOGIC.with_borrow_mut(|l| $crate::guest::tick(l, tick, inputs))
                }
            }

            $crate::guest::bindings::export_logic_world!(Component with_types_in $crate::guest::bindings);
        };
    };
}
