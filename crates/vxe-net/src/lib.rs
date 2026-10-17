//! The simulation split (ADR-0007, ADR-0010): a server that owns the authoritative world and
//! its op log, a byte channel to clients, the game's logic behind a trait, and clients that
//! either share the server's world or own a copy and check every result against the server's.
//!
//! The server reads no clock: a tick is a counter the caller advances by calling
//! [`Server::tick`]. Clients send opaque input bytes; once per tick the server drains them,
//! hands them to the [`Logic`], applies the ops the logic requests through a [`Host`], logs
//! the inputs and the applied ops, and announces every result. Ops may also be submitted
//! directly on the host side with [`Server::submit`]; they apply before the logic's tick.
//!
//! ```
//! use std::sync::Arc;
//! use vxe_net::*;
//! use vxe_world::*;
//!
//! let mut table = MaterialTable::new();
//! let rock = table.push(Material::named("rock"));
//! let extent = ChunkExtent::new([-1, -1, -1], [1, 1, 1]);
//! let world = Arc::new(World::generate(extent, &UniformSource(rock), &table));
//! let mut server = Server::new(world.clone(), Box::new(NoLogic), 0);
//! let (near, far) = MemChannel::pair();
//! server.attach(far);
//! let mut client = Client::shared(world, near);
//! server.submit(EditOp::sphere(centre_of_voxel([0, 0, 0]), 2 * FIXED_ONE, Mode::Carve));
//! server.tick();
//! let events = client.poll();
//! assert!(matches!(&events[..], [Event::Applied { seq: 0, result, .. }] if result.changed_from == [(rock, 33)]));
//! ```

mod channel;
mod client;
mod logic;
mod message;
mod server;

pub use channel::{Channel, Endpoint, MemChannel};
pub use client::{Client, ClientWorld, Event};
pub use logic::{ClientInput, Host, Logic, NoLogic, TickOutput};
pub use message::{MSG_APPLIED, MSG_EVENT, MSG_INPUT, MSG_REFUSED, Message};
pub use server::{Server, TickReport};
