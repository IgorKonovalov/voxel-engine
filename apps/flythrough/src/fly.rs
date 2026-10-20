//! Free flight and walking: mouse look and WASD movement of a camera in voxel units, z up, or
//! a gamepad's sticks through the same `Input`. Flight moves the camera freely; walking moves
//! a character body through the world with the `vxe-sim` controller at a fixed 60 Hz tick and
//! puts the camera at its eye.
//!
//! A stick passes a radial dead zone of [`DEAD_ZONE`] and a squared response: deflection `m`
//! reads `((m - DEAD_ZONE) / (1 - DEAD_ZONE))^2` along the stick's direction, so the move
//! stick sets the speed and the look stick the turn rate.

use vxe_app::{Axis, Input, KeyCode, PadButton};
use vxe_render::Camera;
use vxe_render::glam::{Vec2, Vec3};
use vxe_sim::{CharacterController, CharacterState, ControllerParams, MoveInput, overlaps};
use vxe_world::{FIXED_ONE, Fixed3, World};

/// Radians of turn per unit of relative mouse motion.
const LOOK_PER_COUNT: f32 = 0.0025;
/// Pitch stays inside this many radians of the horizon, so the basis never degenerates.
const PITCH_LIMIT: f32 = 1.55;
/// Flight speed in voxels per second (3 m/s), and the factor `Shift` multiplies it by.
const SPEED: f32 = 30.0;
const BOOST: f32 = 4.0;
/// A stick deflection under this reads 0.
pub const DEAD_ZONE: f32 = 0.15;
/// Radians per second the look stick turns at full deflection.
const LOOK_RATE: f32 = 2.5;

#[derive(Clone, Copy, Debug)]
pub struct Fly {
    pub camera: Camera,
    /// `R3` latched boost; it lets go once the move stick returns inside the dead zone.
    boost: bool,
}

impl Fly {
    pub fn new(camera: Camera) -> Self {
        Self {
            camera,
            boost: false,
        }
    }

    /// Turns by the frame's mouse motion and look stick and moves over `dt` seconds by the
    /// held keys and the move stick: `W`/`S` or the stick's y along the view, `A`/`D` or its x
    /// sideways, `Space`/`C` or `R1`/`L1` up and down in world z. Keys move at [`SPEED`], the
    /// stick at `SPEED` times its response; `Shift` held or `R3` latched multiplies by
    /// [`BOOST`].
    pub fn update(&mut self, input: &Input, dt: f32) {
        look(&mut self.camera, input, dt);
        if input.pad_pressed(PadButton::R3) {
            self.boost = true;
        } else if stick(input, Axis::MoveX, Axis::MoveY) == Vec2::ZERO {
            self.boost = false;
        }
        let c = &mut self.camera;
        let (forward, right, _) = c.basis();
        let up = axis(input, KeyCode::Space, KeyCode::KeyC)
            + f32::from(u8::from(input.pad_held(PadButton::R1)))
            - f32::from(u8::from(input.pad_held(PadButton::L1)));
        let wish = forward * move_axis(input, KeyCode::KeyW, KeyCode::KeyS, Axis::MoveY)
            + right * move_axis(input, KeyCode::KeyD, KeyCode::KeyA, Axis::MoveX)
            + Vec3::Z * up.clamp(-1.0, 1.0);
        let speed = if input.held(KeyCode::ShiftLeft) || self.boost {
            SPEED * BOOST
        } else {
            SPEED
        };
        c.position += wish.clamp_length_max(1.0) * speed * dt;
    }
}

/// Turns `camera` by the frame's mouse motion and, over `dt` seconds, the look stick.
fn look(camera: &mut Camera, input: &Input, dt: f32) {
    let [dx, dy] = input.mouse_delta();
    let s = stick(input, Axis::LookX, Axis::LookY) * LOOK_RATE * dt;
    camera.yaw -= dx as f32 * LOOK_PER_COUNT + s.x;
    camera.pitch =
        (camera.pitch - dy as f32 * LOOK_PER_COUNT + s.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
}

/// `+1`, `-1` or `0` from a pair of held keys.
fn axis(input: &Input, plus: KeyCode, minus: KeyCode) -> f32 {
    f32::from(u8::from(input.held(plus))) - f32::from(u8::from(input.held(minus)))
}

/// The stick of `x` and `y` after the radial dead zone and the squared response; its length
/// is at most 1.
fn stick(input: &Input, x: Axis, y: Axis) -> Vec2 {
    let v = Vec2::new(input.axis(x), input.axis(y));
    let m = v.length();
    if m <= DEAD_ZONE {
        return Vec2::ZERO;
    }
    let r = ((m.min(1.0) - DEAD_ZONE) / (1.0 - DEAD_ZONE)).powi(2);
    v / m * r
}

/// The key pair plus the stick component `a` (after the stick's dead zone and response),
/// clamped to `[-1, 1]`.
pub fn move_axis(input: &Input, plus: KeyCode, minus: KeyCode, a: Axis) -> f32 {
    let s = match a {
        Axis::MoveX => stick(input, Axis::MoveX, Axis::MoveY).x,
        Axis::MoveY => stick(input, Axis::MoveX, Axis::MoveY).y,
        Axis::LookX => stick(input, Axis::LookX, Axis::LookY).x,
        Axis::LookY => stick(input, Axis::LookX, Axis::LookY).y,
        Axis::TriggerL | Axis::TriggerR => input.axis(a),
    };
    (axis(input, plus, minus) + s).clamp(-1.0, 1.0)
}

/// Controller ticks per second; parameters below are per tick.
pub const TICK_HZ: f32 = 60.0;
/// At most this many ticks catch up in one frame; a longer stall drops time.
const MAX_TICKS_PER_FRAME: u32 = 8;
/// The eye above the foot: 1.7 m.
const EYE: f32 = 17.0;
/// Walking speed: 128 fixed units (0.5 voxel) per tick, 3 m/s at 60 Hz.
const WALK_WISH: f32 = 128.0;

/// The body and limits of walk mode (Plan 0004's test parameters): a 0.6 m by 1.8 m cylinder
/// that steps 0.4 m, climbs ledges up to 1.2 m and jumps about 0.6 m.
pub fn walk_params() -> ControllerParams {
    ControllerParams {
        radius: 768,
        height: 4608,
        step_height: 1024,
        climb_height: 3072,
        climb_speed: 64,
        gravity: -8,
        max_fall: 1024,
        jump_speed: 160,
        ground_probe: 2,
    }
}

/// Walk mode: a character body moved by the controller, the camera at its eye.
#[derive(Clone, Copy, Debug)]
pub struct Walk {
    pub controller: CharacterController,
    pub state: CharacterState,
    /// Seconds not yet spent on ticks.
    pending: f32,
    /// The last tick found the body embedded, so `walk: embedded` prints once per episode.
    embedded: bool,
}

impl Walk {
    /// A body whose eye is at the camera, or `None` when it would overlap the world there.
    pub fn at_camera(world: &World, camera: &Camera) -> Option<Self> {
        let controller = CharacterController::new(walk_params());
        let p = camera.position - Vec3::Z * EYE;
        let foot = p.to_array().map(|v| (v * FIXED_ONE as f32).floor() as i64);
        if overlaps(world, &controller.body(foot)) {
            return None;
        }
        Some(Self {
            controller,
            state: CharacterState::new(Fixed3(foot)),
            pending: 0.0,
            embedded: false,
        })
    }

    /// The tick input from held keys and the move stick: `W`/`S`/`A`/`D` walk along the
    /// camera's heading at 3 m/s, the stick at 3 m/s times its response; `Space` or `South`
    /// jumps and, held against a wall with a standable top within 1.2 m, climbs it.
    pub fn input(input: &Input, camera: &Camera) -> MoveInput {
        let (forward, right, _) = camera.basis();
        let f = Vec2::new(forward.x, forward.y).normalize_or_zero();
        let r = Vec2::new(right.x, right.y).normalize_or_zero();
        let w = (f * move_axis(input, KeyCode::KeyW, KeyCode::KeyS, Axis::MoveY)
            + r * move_axis(input, KeyCode::KeyD, KeyCode::KeyA, Axis::MoveX))
        .clamp_length_max(1.0)
            * WALK_WISH;
        let space = input.held(KeyCode::Space) || input.pad_held(PadButton::South);
        MoveInput {
            wish: [w.x.round() as i64, w.y.round() as i64],
            jump: space,
            climb: space,
        }
    }

    /// Turns the camera, runs the ticks `dt` seconds owe, and puts the camera at the eye.
    /// Returns the state-change lines to print: `walk: grounded`, `walk: stepped`,
    /// `walk: climbing`, `walk: climbed`, `walk: lifted out of solid`, `walk: embedded`.
    pub fn update(
        &mut self,
        world: &World,
        input: &Input,
        camera: &mut Camera,
        dt: f32,
    ) -> Vec<&'static str> {
        look(camera, input, dt);
        self.pending += dt;
        let step = 1.0 / TICK_HZ;
        let mut ticks = 0;
        let mut lines = Vec::new();
        while self.pending >= step && ticks < MAX_TICKS_PER_FRAME {
            self.pending -= step;
            ticks += 1;
            lines.extend(self.tick(world, &Self::input(input, camera)));
        }
        if ticks == MAX_TICKS_PER_FRAME {
            self.pending = 0.0;
        }
        camera.position = self.eye();
        lines
    }

    /// One controller tick, and the state-change lines it caused.
    pub fn tick(&mut self, world: &World, input: &MoveInput) -> Vec<&'static str> {
        let (was_grounded, was_climbing) = (self.state.grounded, self.state.is_climbing());
        let r = self.controller.tick(world, &mut self.state, input);
        let mut lines = Vec::new();
        if r.lifted != Fixed3::default() {
            lines.push("walk: lifted out of solid");
        }
        if r.embedded && !self.embedded {
            lines.push("walk: embedded");
        }
        self.embedded = r.embedded;
        if r.stepped {
            lines.push("walk: stepped");
        }
        if self.state.is_climbing() && !was_climbing {
            lines.push("walk: climbing");
        }
        if r.climbed {
            lines.push("walk: climbed");
        }
        if self.state.grounded && !was_grounded {
            lines.push("walk: grounded");
        }
        lines
    }

    /// The eye position in voxels.
    pub fn eye(&self) -> Vec3 {
        Vec3::from_array(self.state.foot.0.map(|v| v as f32 / FIXED_ONE as f32)) + Vec3::Z * EYE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fly() -> Fly {
        Fly::new(Camera {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            vfov: 1.0,
        })
    }

    #[test]
    fn w_moves_along_the_view_at_three_metres_per_second() {
        let mut f = fly();
        let mut input = Input::default();
        input.key_down(KeyCode::KeyW);
        f.update(&input, 0.5);
        assert!((f.camera.position - Vec3::new(15.0, 0.0, 0.0)).length() < 1e-4);
    }

    /// The camera's travel along +x (its heading at yaw 0) after one 0.1 s update.
    fn fly_distance(f: &mut Fly, input: &Input) -> f32 {
        let x0 = f.camera.position.x;
        f.update(input, 0.1);
        f.camera.position.x - x0
    }

    /// The move stick at full deflection moves as the key pair does, 3.0 voxels in 0.1 s;
    /// inside the dead zone it moves nothing; at half deflection it moves
    /// `3.0 x ((0.5 - 0.15) / 0.85)^2`; latched boost moves 12.0; `W` plus the stick clamps
    /// to the key pair's 3.0.
    #[test]
    fn the_move_stick_sets_the_flight_speed_by_its_deflection() {
        let at = |v: f32| {
            let mut input = Input::default();
            input.set_axis(Axis::MoveY, v);
            fly_distance(&mut fly(), &input)
        };
        assert!((at(1.0) - 3.0).abs() < 1e-4, "{}", at(1.0));
        assert_eq!(at(0.1), 0.0);
        assert!((at(0.5) - 0.5086).abs() < 1e-3, "{}", at(0.5));

        let mut f = fly();
        let mut input = Input::default();
        input.set_axis(Axis::MoveY, 1.0);
        input.pad_down(PadButton::R3);
        assert!((fly_distance(&mut f, &input) - 12.0).abs() < 1e-4);
        input.end_frame();
        assert!(
            (fly_distance(&mut f, &input) - 12.0).abs() < 1e-4,
            "latched"
        );
        input.set_axis(Axis::MoveY, 0.0);
        assert_eq!(fly_distance(&mut f, &input), 0.0);
        input.set_axis(Axis::MoveY, 1.0);
        assert!(
            (fly_distance(&mut f, &input) - 3.0).abs() < 1e-4,
            "the latch let go in the dead zone"
        );

        let mut input = Input::default();
        input.key_down(KeyCode::KeyW);
        input.set_axis(Axis::MoveY, 1.0);
        assert!((fly_distance(&mut fly(), &input) - 3.0).abs() < 1e-4);
    }

    /// `R1` rises and `L1` sinks at the key pair's speed.
    #[test]
    fn r1_and_l1_fly_up_and_down() {
        let mut f = fly();
        let mut input = Input::default();
        input.pad_down(PadButton::R1);
        f.update(&input, 0.1);
        assert!((f.camera.position.z - 3.0).abs() < 1e-4);
        input.pad_up(PadButton::R1);
        input.pad_down(PadButton::L1);
        f.update(&input, 0.2);
        assert!((f.camera.position.z + 3.0).abs() < 1e-4);
    }

    /// The look stick at full deflection turns 2.5 rad/s, 0.25 rad in 0.1 s, rightward
    /// (yaw falls, as the mouse turns right), and adds to the frame's mouse motion; up
    /// raises the pitch.
    #[test]
    fn the_look_stick_turns_at_its_rate_and_adds_to_the_mouse() {
        let mut f = fly();
        let mut input = Input::default();
        input.set_axis(Axis::LookX, 1.0);
        f.update(&input, 0.1);
        assert!((f.camera.yaw + 0.25).abs() < 1e-6, "{}", f.camera.yaw);

        let mut f = fly();
        input.add_mouse_delta(100.0, 0.0);
        f.update(&input, 0.1);
        assert!((f.camera.yaw + 0.25 + 100.0 * LOOK_PER_COUNT).abs() < 1e-6);

        let mut f = fly();
        let mut input = Input::default();
        input.set_axis(Axis::LookY, 1.0);
        f.update(&input, 0.1);
        assert!((f.camera.pitch - 0.25).abs() < 1e-6);
    }

    /// `Walk::input` takes the stick as it takes the keys: `MoveY` 1.0 wishes what `W` held
    /// does, and `South` held jumps and climbs as `Space` does.
    #[test]
    fn walk_input_takes_the_stick_and_south_like_w_and_space() {
        let cam = camera_at(Vec3::ZERO, 0.0);
        let mut keys = Input::default();
        keys.key_down(KeyCode::KeyW);
        let mut pad = Input::default();
        pad.set_axis(Axis::MoveY, 1.0);
        let k = Walk::input(&keys, &cam);
        let p = Walk::input(&pad, &cam);
        assert_eq!(k.wish, [128, 0]);
        assert_eq!(p.wish, k.wish);
        assert!(!p.jump);
        pad.pad_down(PadButton::South);
        let p = Walk::input(&pad, &cam);
        assert!(p.jump && p.climb);
    }

    #[test]
    fn mouse_right_turns_right_and_mouse_down_looks_down() {
        let mut f = fly();
        let mut input = Input::default();
        input.add_mouse_delta(100.0, 100.0);
        f.update(&input, 0.0);
        let (forward, _, _) = f.camera.basis();
        assert!(
            forward.y < 0.0,
            "turned toward -y, the camera's right at yaw 0"
        );
        assert!(forward.z < 0.0, "looks down");
    }

    /// A floor of material 1 at `z < 0` over chunks `[-1, 1)³`.
    fn floor_world() -> World {
        let mut t = vxe_world::MaterialTable::new();
        let rock = t.push(vxe_world::Material::named("rock"));
        struct Floor(u16);
        impl vxe_world::WorldSource for Floor {
            fn chunk(&self, c: vxe_world::ChunkCoord, out: &mut vxe_world::Chunk) {
                if c.0[2] < 0 {
                    for slot in 0..vxe_world::CHUNK_BRICKS {
                        out.set_brick(slot, vxe_world::Brick::Uniform(self.0));
                    }
                }
            }
        }
        World::generate(
            vxe_world::ChunkExtent::new([-1, -1, -1], [1, 1, 1]),
            &Floor(rock),
            &t,
        )
    }

    fn camera_at(p: Vec3, pitch: f32) -> Camera {
        Camera {
            position: p,
            yaw: 0.0,
            pitch,
            vfov: 1.0,
        }
    }

    #[test]
    fn walking_with_w_moves_three_metres_per_second() {
        let world = floor_world();
        let mut cam = camera_at(Vec3::new(0.5, 0.5, 20.0), 0.0);
        let mut walk = Walk::at_camera(&world, &cam).unwrap();
        let mut input = Input::default();
        // Fall to the floor from 0.3 m up.
        for _ in 0..30 {
            walk.update(&world, &input, &mut cam, 1.0 / 60.0 + 1e-6);
        }
        assert!(walk.state.grounded);
        assert_eq!(walk.state.foot.0[2], 0);
        let x0 = walk.state.foot.0[0];
        input.key_down(KeyCode::KeyW);
        for _ in 0..60 {
            walk.update(&world, &input, &mut cam, 1.0 / 60.0 + 1e-6);
        }
        // 60 ticks at 128 units: 30 voxels, 3 m.
        assert_eq!(walk.state.foot.0[0] - x0, 60 * 128);
        assert!((cam.position.z - EYE).abs() < 1e-4);
    }

    /// The hand-driven check scripted: dig a 1 m hole under the feet, fall in, then hold
    /// forward and `Space` until the body climbs out.
    #[test]
    fn a_body_falls_into_a_dug_hole_and_climbs_out_with_w_and_space() {
        use vxe_world::{EditOp, Mode};
        let world = floor_world();
        let mut walk = Walk {
            controller: CharacterController::new(walk_params()),
            state: CharacterState::new(Fixed3([128, 128, 0])),
            pending: 0.0,
            embedded: false,
        };
        let idle = MoveInput::default();
        for _ in 0..5 {
            walk.tick(&world, &idle);
        }
        assert!(walk.state.grounded);
        // A 1 m (10 voxel) deep, 1 m wide hole under the feet.
        let r = world
            .apply(&EditOp::box_(
                Fixed3([-5 * 256, -5 * 256, -10 * 256]),
                Fixed3([5 * 256, 5 * 256, 0]),
                Mode::Carve,
            ))
            .unwrap();
        assert_eq!(r.changed_from, [(1, 1000)]);
        let mut lines = Vec::new();
        for _ in 0..120 {
            lines.extend(walk.tick(&world, &idle));
        }
        assert_eq!(walk.state.foot.0[2], -10 * 256, "fell to the hole's floor");
        assert!(walk.state.grounded);
        let mut cam = camera_at(Vec3::ZERO, 0.0);
        let mut input = Input::default();
        input.key_down(KeyCode::KeyW);
        input.key_down(KeyCode::Space);
        let mut climbed = false;
        for _ in 0..240 {
            let step = walk.tick(&world, &Walk::input(&input, &cam));
            climbed |= step.contains(&"walk: climbed");
            lines.extend(step);
            cam.position = walk.eye();
            if climbed && walk.state.grounded {
                break;
            }
        }
        assert!(climbed, "{lines:?}");
        assert!(walk.state.foot.0[2] >= 0 && walk.state.foot.0[0] >= 5 * 256);
        assert!(lines.contains(&"walk: climbing"));
        assert!(lines.contains(&"walk: grounded"));
    }

    /// The owner's walk check, scripted: in a 1 m deep trench, facing its long wall 30
    /// degrees off head-on, `W` plus `Space` climbs out. The wish slides along the wall, so
    /// the climb has to begin without the body being blocked.
    #[test]
    fn w_and_space_climb_out_of_a_trench_facing_its_wall_at_an_angle() {
        use vxe_world::{EditOp, Mode};
        let world = floor_world();
        world
            .apply(&EditOp::box_(
                Fixed3([-10 * 256, -60 * 256, -10 * 256]),
                Fixed3([10 * 256, 60 * 256, 0]),
                Mode::Carve,
            ))
            .unwrap();
        let mut walk = Walk {
            controller: CharacterController::new(walk_params()),
            state: CharacterState::new(Fixed3([128, 128, -10 * 256])),
            pending: 0.0,
            embedded: false,
        };
        let mut cam = Camera {
            yaw: 30f32.to_radians(),
            ..camera_at(walk.eye(), 0.0)
        };
        let mut input = Input::default();
        input.key_down(KeyCode::KeyW);
        input.key_down(KeyCode::Space);
        let mut lines = Vec::new();
        for _ in 0..180 {
            lines.extend(walk.tick(&world, &Walk::input(&input, &cam)));
            cam.position = walk.eye();
            if lines.contains(&"walk: climbed") {
                break;
            }
        }
        assert!(lines.contains(&"walk: climbing"), "{lines:?}");
        assert!(lines.contains(&"walk: climbed"), "{lines:?}");
        // `Space` held hops the body off the rim on the tick it climbs out.
        assert!(walk.state.foot.0[2] >= 0, "out of the trench");
        assert!(walk.state.foot.0[0] >= 10 * 256 - walk_params().radius);
    }

    /// Random walks on the seed-1 level with `Space` held on a third of them and a carve near
    /// the body before a quarter of them: after every tick the body does not overlap the
    /// world, it never stays put for 30 ticks at a pose where no sweep (16 horizontal
    /// directions, up, down) moves it, and some walk climbs a ledge. Before the sweep left
    /// out boxes a move does not close on, the body froze in mid-air beside a voxel edge
    /// within the first few walks of every seed tried. Every 50th walk fills rock over the
    /// body's feet first, which the next tick lifts it out of.
    #[test]
    fn random_walks_on_the_level_never_freeze_or_overlap() {
        use flythrough::cave::{CaveSource, LEVEL_CHUNKS};
        use vxe_sim::sweep;
        let source = CaveSource::new(1, vxe_world::ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
        let world = World::generate(source.extent, &source, &flythrough::cave_materials());
        let c = CharacterController::new(walk_params());
        let mut st = CharacterState::new(Fixed3(source.spawn().0.map(|v| i64::from(v) * 256)));
        let mut rng = 1u64;
        let mut next = || {
            rng = rng
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            rng >> 33
        };
        let dirs: Vec<[i64; 3]> = (0..16)
            .map(|k| {
                let a = k as f32 * std::f32::consts::TAU / 16.0;
                [(a.cos() * 128.0) as i64, (a.sin() * 128.0) as i64, 0]
            })
            .chain([[0, 0, 256], [0, 0, -256]])
            .collect();
        let (mut still, mut climbs, mut ticks, mut digs, mut lifts) = (0, 0, 0, 0, 0);
        for walk in 0..800 {
            if walk % 50 == 0 {
                let f = st.foot.0;
                let r = world.apply(&vxe_world::EditOp::sphere(
                    Fixed3([f[0], f[1], f[2] + 256]),
                    768,
                    vxe_world::Mode::Fill(flythrough::cave::ROCK),
                ));
                assert!(r.is_ok_and(|r| !r.changed_from.is_empty()), "walk {walk}");
                assert!(overlaps(&world, &c.body(f)), "walk {walk}");
            }
            let a = (next() % 3600) as f32 / 3600.0 * std::f32::consts::TAU;
            let wish = [
                (a.cos() * 128.0).round() as i64,
                (a.sin() * 128.0).round() as i64,
            ];
            let space = next() % 3 == 0;
            if next() % 4 == 0 {
                // A dig near the body, as the left click makes: up to 3 m off in plan.
                let f = st.foot.0;
                let mut off = |span: u64, lo: i64| (next() % span) as i64 - lo;
                let at = [
                    f[0] + off(60, 30) * 256,
                    f[1] + off(60, 30) * 256,
                    f[2] + off(30, 10) * 256,
                ];
                let radius = (5 + next() % 11) as i64 * 256;
                let r = world.apply(&vxe_world::EditOp::sphere(
                    Fixed3(at),
                    radius,
                    vxe_world::Mode::Carve,
                ));
                digs += u32::from(r.is_ok());
            }
            for _ in 0..20 + next() % 100 {
                let input = MoveInput {
                    wish,
                    jump: space,
                    climb: space,
                };
                let r = c.tick(&world, &mut st, &input);
                ticks += 1;
                climbs += u32::from(r.climbed);
                lifts += u32::from(r.lifted != Fixed3::default());
                assert!(!r.embedded, "walk {walk}: embedded at {:?}", st.foot.0);
                let f = st.foot.0;
                assert!(
                    !overlaps(&world, &c.body(f)),
                    "walk {walk}: overlapping at {f:?}"
                );
                let m = r.moved.0;
                still = if m[0] * m[0] + m[1] * m[1] + m[2] * m[2] < 4 && !st.is_climbing() {
                    still + 1
                } else {
                    0
                };
                if still == 30 {
                    let free = dirs.iter().any(|d| {
                        let s = sweep(&world, &c.body(f), Fixed3(*d)).moved.0;
                        s[0] * s[0] + s[1] * s[1] + s[2] * s[2] >= 16 * 16
                    });
                    assert!(free, "walk {walk}: frozen at {f:?}, vz {}", st.vz);
                }
            }
        }
        assert!(ticks > 40_000);
        assert!(digs > 100, "{digs} carves");
        assert!(climbs > 0, "no climb in {ticks} ticks");
        assert_eq!(lifts, 16);
    }

    #[test]
    fn pitch_is_clamped_short_of_vertical() {
        let mut f = fly();
        let mut input = Input::default();
        input.add_mouse_delta(0.0, -1.0e6);
        f.update(&input, 0.0);
        assert_eq!(f.camera.pitch, PITCH_LIMIT);
    }
}
