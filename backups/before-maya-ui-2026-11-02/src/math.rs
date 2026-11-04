//! Rigid transforms. Mirrors Roblox's CFrame: a rotation plus a position, with
//! `a * b` meaning "b expressed in a's space".

use glam::{Mat3, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Xf {
    pub rot: Quat,
    pub pos: Vec3,
}

impl Default for Xf {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Xf {
    pub const IDENTITY: Xf = Xf {
        rot: Quat::IDENTITY,
        pos: Vec3::ZERO,
    };

    pub fn new(rot: Quat, pos: Vec3) -> Self {
        Self { rot, pos }
    }

    pub fn from_cframe(c: &rbx_types::CFrame) -> Self {
        // rbx_types stores the rotation by rows; glam wants columns.
        let o = &c.orientation;
        let m = Mat3::from_cols(
            Vec3::new(o.x.x, o.y.x, o.z.x),
            Vec3::new(o.x.y, o.y.y, o.z.y),
            Vec3::new(o.x.z, o.y.z, o.z.z),
        );
        Self {
            rot: Quat::from_mat3(&orthonormalize(m)).normalize(),
            pos: Vec3::new(c.position.x, c.position.y, c.position.z),
        }
    }

    pub fn inverse(self) -> Self {
        let rot = self.rot.conjugate();
        Self {
            rot,
            pos: rot * -self.pos,
        }
    }

    pub fn point(self, p: Vec3) -> Vec3 {
        self.rot * p + self.pos
    }

    pub fn matrix(self) -> Mat4 {
        Mat4::from_rotation_translation(self.rot, self.pos)
    }

    /// True when both transforms match closely enough that an edit between them
    /// is not worth an undo step.
    pub fn approx_eq(self, other: Xf) -> bool {
        self.pos.distance_squared(other.pos) < 1e-10
            && self.rot.dot(other.rot).abs() > 1.0 - 1e-7
    }
}

impl std::ops::Mul for Xf {
    type Output = Xf;
    fn mul(self, rhs: Xf) -> Xf {
        Xf {
            rot: (self.rot * rhs.rot).normalize(),
            pos: self.rot * rhs.pos + self.pos,
        }
    }
}

/// File rotations occasionally drift from exact orthonormal due to float storage.
fn orthonormalize(m: Mat3) -> Mat3 {
    let x = m.x_axis.try_normalize().unwrap_or(Vec3::X);
    let y = (m.y_axis - x * x.dot(m.y_axis))
        .try_normalize()
        .unwrap_or(Vec3::Y);
    Mat3::from_cols(x, y, x.cross(y))
}

/// Closest parameter `t` on the line `a + t*dir` to the ray `o + s*d`.
pub fn closest_on_line(a: Vec3, dir: Vec3, o: Vec3, d: Vec3) -> Option<f32> {
    let w = a - o;
    let b = dir.dot(d);
    let denom = dir.dot(dir) * d.dot(d) - b * b;
    if denom.abs() < 1e-6 {
        return None;
    }
    Some((b * d.dot(w) - d.dot(d) * dir.dot(w)) / denom)
}

/// Ray versus box given in local space, returning the entry distance.
pub fn ray_box(origin: Vec3, dir: Vec3, half: Vec3) -> Option<f32> {
    let mut tmin = f32::NEG_INFINITY;
    let mut tmax = f32::INFINITY;
    for i in 0..3 {
        if dir[i].abs() < 1e-7 {
            if origin[i].abs() > half[i] {
                return None;
            }
            continue;
        }
        let t1 = (-half[i] - origin[i]) / dir[i];
        let t2 = (half[i] - origin[i]) / dir[i];
        tmin = tmin.max(t1.min(t2));
        tmax = tmax.min(t1.max(t2));
    }
    (tmax >= tmin.max(0.0)).then_some(tmin.max(0.0))
}
