//! A pinhole camera in voxel units, z up, and the views a frame renders.

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

/// One view of a frame, as the shaders read it: an eye at `origin` with an orthonormal basis,
/// the tangents of its four half-angles (left, right, up, down; an asymmetric frustum has
/// unequal pairs), and the rectangle of the internal target it fills (`x, y, w, h` in
/// pixels). The ray of pixel `(px, py)` inside the rectangle, with
/// `u = (px - x + 0.5 + jitter.x) / w` and `v = (py - y + 0.5 + jitter.y) / h`, is
/// `fwd + right (-tan_left + u (tan_left + tan_right)) + up (tan_up - v (tan_up + tan_down))`.
/// `aspect` changes no ray beyond rounding: the shader takes horizontal tangents in units of
/// the mean vertical tangent times it, so views sharing it round alike (see `pixel_ray` in
/// `march.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct View {
    pub origin: [f32; 3],
    pub tan_left: f32,
    pub right: [f32; 3],
    pub tan_right: f32,
    pub up: [f32; 3],
    pub tan_up: f32,
    pub fwd: [f32; 3],
    pub tan_down: f32,
    pub rect: [u32; 4],
    pub aspect: f32,
    /// `(tan_left + tan_right) / unit` and `tan_left / unit`, `unit` the mean vertical
    /// tangent times `aspect`: set by the constructors.
    pub unit_scale: f32,
    pub unit_offset: f32,
    pub _pad: f32,
    /// The sub-pixel offset of every ray of the view, in internal pixels, each component in
    /// `(-0.5, 0.5)`; zero by the constructors (`Frame::params_for` sets it per frame when
    /// the frame jitters).
    pub jitter: [f32; 2],
    pub _pad2: [f32; 2],
}

impl View {
    /// The symmetric view of `camera` over `rect`: vertical half-angle `vfov / 2`, horizontal
    /// tangent scaled by the rectangle's aspect.
    pub fn of_camera(camera: &Camera, rect: [u32; 4]) -> Self {
        let t = (camera.vfov * 0.5).tan();
        let aspect = rect[2] as f32 / rect[3] as f32;
        Self::with_tangents(camera, [t * aspect, t * aspect, t, t], aspect, rect)
    }

    /// `camera`'s eye and basis with explicit tangents `[left, right, up, down]`, and the
    /// `aspect` its horizontal tangents are taken in units of.
    pub fn with_tangents(camera: &Camera, tangents: [f32; 4], aspect: f32, rect: [u32; 4]) -> Self {
        let (fwd, right, up) = camera.basis();
        let [tan_left, tan_right, tan_up, tan_down] = tangents;
        let unit = (tan_up + tan_down) * 0.5 * aspect;
        Self {
            origin: camera.position.to_array(),
            tan_left,
            right: right.to_array(),
            tan_right,
            up: up.to_array(),
            tan_up,
            fwd: fwd.to_array(),
            tan_down,
            rect,
            aspect,
            unit_scale: (tan_left + tan_right) / unit,
            unit_offset: tan_left / unit,
            _pad: 0.0,
            jitter: [0.0; 2],
            _pad2: [0.0; 2],
        }
    }
}
