//! Orbit camera. Roblox space is right-handed and Y-up, and characters face -Z,
//! so the Front view looks from -Z toward +Z.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use egui::{Pos2, Rect, pos2};
use glam::camera::rh::proj::directx;
use glam::{Mat4, Quat, Vec3, Vec4Swizzles};

use crate::math::Xf;

/// Looking through a camera in the scene. While set, the orbit camera answers every
/// question with this view, so handles and picking keep working through it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThroughView {
    pub frame: Xf,
    /// Vertical field of view, in radians.
    pub fov: f32,
}

impl ThroughView {
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        directx::perspective(self.fov, aspect, 0.05, 8000.0) * self.frame.matrix().inverse()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewPreset {
    Front,
    Back,
    Right,
    Left,
    Top,
    Bottom,
}

impl ViewPreset {
    pub fn label(self) -> &'static str {
        match self {
            ViewPreset::Front => "Front",
            ViewPreset::Back => "Back",
            ViewPreset::Right => "Right",
            ViewPreset::Left => "Left",
            ViewPreset::Top => "Top",
            ViewPreset::Bottom => "Bottom",
        }
    }

    pub fn angles(self) -> (f32, f32) {
        match self {
            ViewPreset::Front => (PI, 0.0),
            ViewPreset::Back => (0.0, 0.0),
            ViewPreset::Right => (FRAC_PI_2, 0.0),
            ViewPreset::Left => (-FRAC_PI_2, 0.0),
            ViewPreset::Top => (PI, -FRAC_PI_2),
            ViewPreset::Bottom => (PI, FRAC_PI_2),
        }
    }
}

#[derive(Clone, Debug)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub ortho: bool,
    anim: Option<(f32, f32)>,
    /// The preset this camera is resting on, shown in the viewport label.
    pub preset: Option<ViewPreset>,
    /// Vertical field of view, in radians.
    pub fov: f32,
    pub smooth_views: bool,
    pub view_speed: f32,
    pub through: Option<ThroughView>,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 2.5, 0.0),
            yaw: PI - 0.55,
            pitch: -0.28,
            dist: 14.0,
            ortho: false,
            anim: None,
            preset: None,
            fov: 0.6,
            smooth_views: true,
            view_speed: 1.0,
            through: None,
        }
    }
}

/// A ray in world space.
#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl OrbitCamera {
    pub fn rotation(&self) -> Quat {
        match self.through {
            Some(view) => view.frame.rot,
            None => Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch),
        }
    }

    pub fn eye(&self) -> Vec3 {
        match self.through {
            Some(view) => view.frame.pos,
            None => self.target + self.rotation() * Vec3::Z * self.dist,
        }
    }

    /// Where the orbit view is, even while looking through a camera.
    pub fn free_frame(&self) -> Xf {
        let rot = Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch);
        Xf::new(rot, self.target + rot * Vec3::Z * self.dist)
    }

    /// Cameras in the scene are always perspective.
    pub fn is_ortho(&self) -> bool {
        self.ortho && self.through.is_none()
    }

    fn view_fov(&self) -> f32 {
        self.through.map_or(self.fov, |view| view.fov)
    }

    /// Direction from the scene toward the camera.
    pub fn to_camera(&self) -> Vec3 {
        self.rotation() * Vec3::Z
    }

    pub fn right(&self) -> Vec3 {
        self.rotation() * Vec3::X
    }

    pub fn up(&self) -> Vec3 {
        self.rotation() * Vec3::Y
    }

    pub fn view(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.rotation(), self.eye()).inverse()
    }

    pub fn projection(&self, aspect: f32) -> Mat4 {
        if self.is_ortho() {
            let h = self.half_height_at_target();
            directx::orthographic(-h * aspect, h * aspect, -h, h, -2000.0, 6000.0)
        } else {
            let near = (self.dist * 0.01).max(0.01);
            directx::perspective(self.view_fov(), aspect, near, (self.dist * 200.0).max(8000.0))
        }
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }

    fn half_height_at_target(&self) -> f32 {
        self.dist * (self.fov * 0.5).tan()
    }

    /// World units covered by one screen point at `point`'s depth.
    pub fn world_per_point(&self, point: Vec3, rect: Rect) -> f32 {
        let height = rect.height().max(1.0);
        if self.is_ortho() {
            return 2.0 * self.half_height_at_target() / height;
        }
        let depth = (point - self.eye()).dot(-self.to_camera()).max(0.01);
        2.0 * depth * (self.view_fov() * 0.5).tan() / height
    }

    pub fn world_to_screen(&self, p: Vec3, rect: Rect) -> Option<Pos2> {
        let clip = self.view_proj(rect.aspect_ratio()) * p.extend(1.0);
        if clip.w <= 1e-5 {
            return None;
        }
        let ndc = clip.xyz() / clip.w;
        Some(pos2(
            rect.left() + (ndc.x * 0.5 + 0.5) * rect.width(),
            rect.top() + (0.5 - ndc.y * 0.5) * rect.height(),
        ))
    }

    pub fn screen_ray(&self, pos: Pos2, rect: Rect) -> Ray {
        let nx = (pos.x - rect.left()) / rect.width() * 2.0 - 1.0;
        let ny = 1.0 - (pos.y - rect.top()) / rect.height() * 2.0;
        let h = self.half_height_at_target();
        let aspect = rect.aspect_ratio();
        if self.is_ortho() {
            let dir = -self.to_camera();
            let origin = self.target + self.right() * (nx * h * aspect) + self.up() * (ny * h)
                - dir * 4000.0;
            Ray { origin, dir }
        } else {
            let tan = (self.view_fov() * 0.5).tan();
            let dir = (self.rotation() * Vec3::new(nx * tan * aspect, ny * tan, -1.0)).normalize();
            Ray {
                origin: self.eye(),
                dir,
            }
        }
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.anim = None;
        self.preset = None;
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch - dy * 0.008).clamp(-FRAC_PI_2, FRAC_PI_2);
    }

    /// Orbits while keeping `pivot` fixed on screen, rather than the view's center.
    pub fn orbit_about(&mut self, dx: f32, dy: f32, pivot: Vec3) {
        let before = self.rotation();
        self.orbit(dx, dy);
        let turn = self.rotation() * before.inverse();
        self.target = pivot + turn * (self.target - pivot);
    }

    /// Zooms toward what's under the mouse instead of the view's center.
    pub fn zoom_at(&mut self, factor: f32, mouse: Pos2, rect: Rect) {
        let ray = self.screen_ray(mouse, rect);
        let normal = self.to_camera();
        let denom = ray.dir.dot(normal);
        if denom.abs() > 1e-5 {
            let hit = ray.origin + ray.dir * ((self.target - ray.origin).dot(normal) / denom);
            self.target = hit + (self.target - hit) * factor;
        }
        self.zoom(factor);
    }

    pub fn pan(&mut self, dx: f32, dy: f32, rect: Rect) {
        let wpp = self.world_per_point(self.target, rect);
        self.target += (self.up() * dy - self.right() * dx) * wpp;
    }

    pub fn zoom(&mut self, factor: f32) {
        self.dist = (self.dist * factor).clamp(0.2, 20000.0);
    }

    pub fn frame_bounds(&mut self, min: Vec3, max: Vec3) {
        self.target = (min + max) * 0.5;
        let radius = (max - min).length() * 0.5;
        self.dist = (radius.max(0.5) / (self.fov * 0.5).sin() * 1.15).max(1.0);
    }

    pub fn set_preset(&mut self, preset: ViewPreset) {
        let (yaw, pitch) = preset.angles();
        // Travel the short way around.
        let delta = (yaw - self.yaw).rem_euclid(TAU);
        let yaw = if delta > PI { self.yaw + delta - TAU } else { self.yaw + delta };
        self.preset = Some(preset);
        if self.smooth_views {
            self.anim = Some((yaw, pitch));
        } else {
            self.yaw = yaw;
            self.pitch = pitch;
            self.anim = None;
        }
    }

    /// Advances a view change. Returns true while still moving.
    pub fn tick(&mut self, dt: f32) -> bool {
        let Some((yaw, pitch)) = self.anim else {
            return false;
        };
        let k = 1.0 - (-dt * 18.0 * self.view_speed).exp();
        self.yaw += (yaw - self.yaw) * k;
        self.pitch += (pitch - self.pitch) * k;
        if (yaw - self.yaw).abs() + (pitch - self.pitch).abs() < 0.001 {
            self.yaw = yaw;
            self.pitch = pitch;
            self.anim = None;
            return false;
        }
        true
    }

    pub fn label(&self) -> String {
        if self.through.is_some() {
            return "Camera View".to_string();
        }
        let projection = if self.ortho { "Orthographic" } else { "Perspective" };
        match self.preset {
            Some(p) => format!("{} {}", p.label(), projection),
            None => projection.to_string(),
        }
    }
}
