//! The sky the game sets each frame (ADR-0026): the atmosphere, the sun and the moon, and the
//! mapping from the world's voxels to altitude. The engine holds no clock: the game turns its
//! own time into the bodies' directions, with [`body_direction`] if it likes.
//!
//! World axes are +x east, +y north, +z up. Illuminance and radiance are in the frame's
//! radiance units, the units a material's emission is in.

use bytemuck::{Pod, Zeroable};

/// The atmosphere's parameters (Hillaire 2020's Earth defaults). Coefficients are per metre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Atmosphere {
    pub bottom_radius_km: f32,
    pub top_radius_km: f32,
    pub rayleigh_scattering: [f32; 3],
    pub rayleigh_scale_height_km: f32,
    pub mie_scattering: [f32; 3],
    pub mie_extinction: [f32; 3],
    pub mie_scale_height_km: f32,
    /// The Mie phase function's asymmetry (Cornette-Shanks).
    pub mie_g: f32,
    /// Ozone's absorption at the peak of its tent profile (10 to 40 km, peak at 25 km).
    pub ozone_absorption: [f32; 3],
    /// The planet's albedo, which the multiple-scattering term bounces off the ground.
    pub ground_albedo: [f32; 3],
}

impl Default for Atmosphere {
    fn default() -> Self {
        Self {
            bottom_radius_km: 6360.0,
            top_radius_km: 6460.0,
            rayleigh_scattering: [5.802e-6, 13.558e-6, 33.1e-6],
            rayleigh_scale_height_km: 8.0,
            mie_scattering: [3.996e-6; 3],
            mie_extinction: [4.40e-6; 3],
            mie_scale_height_km: 1.2,
            mie_g: 0.8,
            ozone_absorption: [0.650e-6, 1.881e-6, 0.085e-6],
            ground_albedo: [0.3; 3],
        }
    }
}

/// The real sun's angular radius: 0.2665 degrees.
pub const SUN_ANGULAR_RADIUS: f32 = 4.651e-3;
/// The moon's angular radius: 0.2595 degrees.
pub const MOON_ANGULAR_RADIUS: f32 = 4.53e-3;
/// The sun's top-of-atmosphere illuminance in frame radiance units: a sunlit ground of albedo
/// 0.3 reads about 200 times a glowing ore (a guess the owner tunes).
pub const SUN_ILLUMINANCE: f32 = 3000.0;
/// A full moon's illuminance over the sun's.
pub const MOON_OVER_SUN: f32 = 1.0 / 400_000.0;

/// A light in the sky: a unit direction toward it in world axes, its top-of-atmosphere
/// illuminance per channel, and its angular radius in radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub direction: [f32; 3],
    pub illuminance: [f32; 3],
    pub angular_radius: f32,
}

impl Body {
    /// The sun toward `direction` at [`SUN_ILLUMINANCE`] and its real radius.
    pub fn sun(direction: [f32; 3]) -> Self {
        Self {
            direction,
            illuminance: [SUN_ILLUMINANCE; 3],
            angular_radius: SUN_ANGULAR_RADIUS,
        }
    }

    /// A full moon toward `direction` at [`MOON_OVER_SUN`] of [`SUN_ILLUMINANCE`].
    pub fn moon(direction: [f32; 3]) -> Self {
        Self {
            direction,
            illuminance: [SUN_ILLUMINANCE * MOON_OVER_SUN; 3],
            angular_radius: MOON_ANGULAR_RADIUS,
        }
    }

    /// The body's elevation above the horizon, in radians.
    pub fn elevation(&self) -> f32 {
        let [x, y, z] = self.direction;
        z.atan2((x * x + y * y).sqrt())
    }

    /// The solid angle of its disc, in steradians.
    pub fn solid_angle(&self) -> f32 {
        disc_solid_angle(self.angular_radius)
    }
}

/// The solid angle of a disc of angular radius `r` radians: `2 pi (1 - cos r)`, in f64 so a
/// small radius keeps its digits.
pub fn disc_solid_angle(r: f32) -> f32 {
    (2.0 * std::f64::consts::PI * (1.0 - f64::from(r).cos())) as f32
}

/// The sky of a frame ([`crate::Frame::set_sky`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sky {
    pub atmosphere: Atmosphere,
    pub sun: Body,
    pub moon: Body,
    /// The world voxel z of altitude `base_altitude_m`.
    pub altitude_zero_z: f32,
    /// The consumer's scale: metres per voxel.
    pub metres_per_voxel: f32,
    /// The altitude of the plane `z = altitude_zero_z` above the planet's surface, in metres.
    pub base_altitude_m: f32,
    /// A test sky: a constant radiance above the horizon and 0 below it, no discs and no
    /// directional light; the atmosphere and bodies are then unused.
    pub uniform: Option<[f32; 3]>,
}

impl Sky {
    /// The default atmosphere with `sun` and `moon`, altitude 0 at z = 0, 10 cm voxels.
    pub fn new(sun: Body, moon: Body) -> Self {
        Self {
            atmosphere: Atmosphere::default(),
            sun,
            moon,
            altitude_zero_z: 0.0,
            metres_per_voxel: 0.1,
            base_altitude_m: 0.0,
            uniform: None,
        }
    }

    /// The test sky of radiance `l` above the horizon ([`Sky::uniform`]'s field).
    pub fn uniform(l: f32) -> Self {
        let below = Body {
            direction: [0.0, 0.0, -1.0],
            illuminance: [0.0; 3],
            angular_radius: SUN_ANGULAR_RADIUS,
        };
        Self {
            uniform: Some([l; 3]),
            ..Self::new(below, below)
        }
    }

    /// The scale the sky-view LUT stores radiance at: one over the larger body's largest
    /// channel, so the table holds values near 1 by day and by night.
    pub(crate) fn view_scale(&self) -> f32 {
        let m = self
            .sun
            .illuminance
            .iter()
            .chain(&self.moon.illuminance)
            .fold(0.0f32, |a, &b| a.max(b));
        if m > 0.0 { 1.0 / m } else { 1.0 }
    }
}

/// The unit direction toward a body at `hour_angle` (radians, 0 at its culmination, positive
/// after it, toward the west), `declination` and an observer's `latitude` (radians), in world
/// axes (+x east, +y north, +z up).
pub fn body_direction(hour_angle: f32, declination: f32, latitude: f32) -> [f32; 3] {
    let (h, d, l) = (
        f64::from(hour_angle),
        f64::from(declination),
        f64::from(latitude),
    );
    let east = -d.cos() * h.sin();
    let north = d.sin() * l.cos() - d.cos() * h.cos() * l.sin();
    let up = l.sin() * d.sin() + l.cos() * d.cos() * h.cos();
    let n = (east * east + north * north + up * up).sqrt();
    [(east / n) as f32, (north / n) as f32, (up / n) as f32]
}

/// `sky.wgsl`'s `SkyUniform` flag: a sky is set.
pub(crate) const SKY_ON: u32 = 1;
/// The sky is [`Sky::uniform`]'s.
pub(crate) const SKY_UNIFORM: u32 = 2;
/// The sky-view pass adds the multiple-scattering LUT.
pub(crate) const SKY_MULTISCATTER: u32 = 4;

/// The sky's uniform as `sky.wgsl` declares it: the atmosphere per kilometre, the bodies, the
/// view scale, the altitude mapping and the camera's altitude.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub(crate) struct SkyUniform {
    pub rayleigh_scattering: [f32; 3],
    pub bottom_radius_km: f32,
    pub mie_scattering: [f32; 3],
    pub top_radius_km: f32,
    pub mie_extinction: [f32; 3],
    pub rayleigh_scale_height_km: f32,
    pub ozone_absorption: [f32; 3],
    pub mie_scale_height_km: f32,
    pub ground_albedo: [f32; 3],
    pub mie_g: f32,
    pub sun_dir: [f32; 3],
    pub sun_radius: f32,
    pub sun_e: [f32; 3],
    pub flags: u32,
    pub moon_dir: [f32; 3],
    pub moon_radius: f32,
    pub moon_e: [f32; 3],
    pub view_scale: f32,
    pub uniform_l: [f32; 3],
    pub altitude_zero_z: f32,
    pub metres_per_voxel: f32,
    pub base_altitude_m: f32,
    /// The camera's altitude above the planet's surface in kilometres, which the sky-view
    /// LUT is built for.
    pub camera_alt_km: f32,
    pub _pad: f32,
}

impl SkyUniform {
    /// `sky`'s uniform, `None` the frame's off state, the camera at world z `camera_z`.
    pub(crate) fn of(sky: Option<&Sky>, camera_z: f32, multiscatter: bool) -> Self {
        let Some(s) = sky else {
            return Self::default();
        };
        let a = &s.atmosphere;
        let km = |v: [f32; 3]| v.map(|c| c * 1000.0);
        let mut flags = SKY_ON;
        if s.uniform.is_some() {
            flags |= SKY_UNIFORM;
        }
        if multiscatter {
            flags |= SKY_MULTISCATTER;
        }
        Self {
            rayleigh_scattering: km(a.rayleigh_scattering),
            bottom_radius_km: a.bottom_radius_km,
            mie_scattering: km(a.mie_scattering),
            top_radius_km: a.top_radius_km,
            mie_extinction: km(a.mie_extinction),
            rayleigh_scale_height_km: a.rayleigh_scale_height_km,
            ozone_absorption: km(a.ozone_absorption),
            mie_scale_height_km: a.mie_scale_height_km,
            ground_albedo: a.ground_albedo,
            mie_g: a.mie_g,
            sun_dir: s.sun.direction,
            sun_radius: s.sun.angular_radius,
            sun_e: s.sun.illuminance,
            flags,
            moon_dir: s.moon.direction,
            moon_radius: s.moon.angular_radius,
            moon_e: s.moon.illuminance,
            view_scale: s.view_scale(),
            uniform_l: s.uniform.unwrap_or([0.0; 3]),
            altitude_zero_z: s.altitude_zero_z,
            metres_per_voxel: s.metres_per_voxel,
            base_altitude_m: s.base_altitude_m,
            camera_alt_km: ((camera_z - s.altitude_zero_z) * s.metres_per_voxel
                + s.base_altitude_m)
                / 1000.0,
            _pad: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deg(d: f32) -> f32 {
        d.to_radians()
    }

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= tol)
    }

    #[test]
    fn the_sun_at_culmination_on_the_equator_is_at_the_zenith() {
        let d = body_direction(0.0, 0.0, 0.0);
        assert!(close(d, [0.0, 0.0, 1.0], 1e-6), "{d:?}");
    }

    #[test]
    fn at_latitude_45_noon_is_45_degrees_up_in_the_south() {
        let d = body_direction(0.0, 0.0, deg(45.0));
        let b = Body::sun(d);
        assert!((b.elevation() - deg(45.0)).abs() < 1e-6, "{d:?}");
        assert!(d[0].abs() < 1e-6 && d[1] < 0.0, "toward the south: {d:?}");
    }

    #[test]
    fn hour_angle_90_sets_the_sun_in_the_west() {
        let d = body_direction(deg(90.0), 0.0, deg(45.0));
        assert!(Body::sun(d).elevation().abs() < 1e-6, "{d:?}");
        assert!((d[0] + 1.0).abs() < 1e-6, "toward the west: {d:?}");
    }

    #[test]
    fn a_quarter_to_six_is_2_65_degrees_up() {
        // 17:45 at 15 degrees an hour: sin(el) = cos 45 cos 86.25 = 0.0463.
        let d = body_direction(deg(86.25), 0.0, deg(45.0));
        let el = Body::sun(d).elevation().to_degrees();
        assert!((el - 2.65).abs() < 0.01, "{el}");
    }

    #[test]
    fn the_real_sun_disc_holds_6_80e_5_steradians() {
        let w = Body::sun([0.0, 0.0, 1.0]).solid_angle();
        assert!((w / 6.80e-5 - 1.0).abs() < 0.005, "{w}");
    }

    #[test]
    fn no_sky_is_the_zero_uniform_and_a_sky_sets_its_flag() {
        assert_eq!(SkyUniform::of(None, 0.0, true), SkyUniform::default());
        let s = Sky::new(Body::sun([0.0, 0.0, 1.0]), Body::moon([0.0, 0.0, -1.0]));
        let u = SkyUniform::of(Some(&s), 100.0, false);
        assert_eq!(u.flags, SKY_ON);
        assert!((u.camera_alt_km - 0.01).abs() < 1e-7, "{}", u.camera_alt_km);
        assert!((u.view_scale - 1.0 / SUN_ILLUMINANCE).abs() < 1e-9);
        assert_eq!(
            SkyUniform::of(Some(&Sky::uniform(0.5)), 0.0, false).flags,
            SKY_ON | SKY_UNIFORM
        );
    }
}
