//! Free flight: mouse look and WASD movement of a camera in voxel units, z up.

use vxe_app::{Input, KeyCode};
use vxe_render::Camera;
use vxe_render::glam::Vec3;

/// Radians of turn per unit of relative mouse motion.
const LOOK_PER_COUNT: f32 = 0.0025;
/// Pitch stays inside this many radians of the horizon, so the basis never degenerates.
const PITCH_LIMIT: f32 = 1.55;
/// Flight speed in voxels per second (3 m/s), and the factor `Shift` multiplies it by.
const SPEED: f32 = 30.0;
const BOOST: f32 = 4.0;

#[derive(Clone, Copy, Debug)]
pub struct Fly {
    pub camera: Camera,
}

impl Fly {
    pub fn new(camera: Camera) -> Self {
        Self { camera }
    }

    /// Turns by the frame's mouse motion and moves by the held keys over `dt` seconds:
    /// `W`/`S` along the view, `A`/`D` sideways, `Space`/`C` up and down in world z.
    pub fn update(&mut self, input: &Input, dt: f32) {
        let [dx, dy] = input.mouse_delta();
        let c = &mut self.camera;
        c.yaw -= dx as f32 * LOOK_PER_COUNT;
        c.pitch = (c.pitch - dy as f32 * LOOK_PER_COUNT).clamp(-PITCH_LIMIT, PITCH_LIMIT);

        let (forward, right, _) = c.basis();
        let axis = |plus, minus| {
            f32::from(u8::from(input.held(plus))) - f32::from(u8::from(input.held(minus)))
        };
        let wish = forward * axis(KeyCode::KeyW, KeyCode::KeyS)
            + right * axis(KeyCode::KeyD, KeyCode::KeyA)
            + Vec3::Z * axis(KeyCode::Space, KeyCode::KeyC);
        let speed = if input.held(KeyCode::ShiftLeft) {
            SPEED * BOOST
        } else {
            SPEED
        };
        c.position += wish.normalize_or_zero() * speed * dt;
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

    #[test]
    fn pitch_is_clamped_short_of_vertical() {
        let mut f = fly();
        let mut input = Input::default();
        input.add_mouse_delta(0.0, -1.0e6);
        f.update(&input, 0.0);
        assert_eq!(f.camera.pitch, PITCH_LIMIT);
    }
}
