//! A pinhole camera in voxel units, z up.

use glam::Vec3;

/// Position in voxels (voxel `i` spans `[i, i + 1)`), yaw about +z from +x, pitch toward +z,
/// both in radians, and the vertical field of view in radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub vfov: f32,
}

impl Camera {
    /// Unit forward, right and up vectors.
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let forward = Vec3::new(cp * cy, cp * sy, sp);
        let right = forward.cross(Vec3::Z).normalize_or(Vec3::Y * -1.0);
        let up = right.cross(forward);
        (forward, right, up)
    }
}
