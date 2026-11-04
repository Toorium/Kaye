//! Interactive transforms: the on-screen handles and the shared drag logic used
//! by both the handles and Blender-style G/R.

use std::f32::consts::{PI, TAU};

use egui::{Color32, Painter, Pos2, Rect, Stroke};
use glam::{Quat, Vec3};

use crate::camera::{OrbitCamera, Ray};
use crate::math::{Xf, closest_on_line};
use crate::scene::{BoneId, RigId, Scene};

pub const AXIS_COLORS: [Color32; 3] = [
    Color32::from_rgb(232, 72, 84),
    Color32::from_rgb(130, 201, 60),
    Color32::from_rgb(64, 148, 235),
];
const HOT: Color32 = Color32::from_rgb(0xFF, 0xE4, 0x38);
const AXIS_NAMES: [&str; 3] = ["X", "Y", "Z"];

const MOVE_LENGTH: f32 = 84.0;
const RING_RADIUS: f32 = 76.0;
const VIEW_RING_RADIUS: f32 = 90.0;
const TRACKBALL_SPEED: f32 = 0.009;
const PICK_DISTANCE: f32 = 9.0;
const RING_SEGMENTS: usize = 72;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    Global,
    Local,
}

impl Space {
    pub fn label(self) -> &'static str {
        match self {
            Space::Global => "Global",
            Space::Local => "Local",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    Move,
    Rotate,
}

/// An axis constraint, kept as an index so it can be shown and toggled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxisLock {
    pub axis: usize,
    pub space: Space,
}

/// What a drag moves. Moving a bone is posing it, and moving a rig is only placing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moving {
    Bone(BoneId),
    Rig(RigId),
}

/// One drag from start to confirm or cancel.
#[derive(Clone, Debug)]
pub struct Session {
    pub moving: Moving,
    pub op: OpKind,
    pub lock: Option<AxisLock>,
    /// Modal sessions (G/R) end on a click; handle drags end on release.
    pub modal: bool,
    /// The bone's pose, or the rig's placement.
    pub start_pose: Xf,
    pub start_frame: Xf,
    start_mouse: Pos2,
    last_angle: f32,
    pub angle: f32,
    pub offset: Vec3,
    /// Pressing R again turns a rotate into a free tumble, like Blender.
    pub trackball: bool,
    last_mouse: Pos2,
    current: Xf,
    /// Where the main bone was when the drag began, however many times the
    /// rotate mode has switched since.
    origin: Xf,
    /// Other selected bones that move along: bone, starting pose, starting frame.
    pub extras: Vec<(BoneId, Xf, Xf)>,
}

impl Session {
    pub fn new(scene: &Scene, bone: BoneId, op: OpKind, lock: Option<AxisLock>, modal: bool, mouse: Pos2) -> Self {
        let b = &scene.bones[bone];
        Self::starting(Moving::Bone(bone), b.pose, b.frame, op, lock, modal, mouse)
    }

    /// A drag of a whole rig, around the part it stands on.
    pub fn for_rig(scene: &Scene, rig: RigId, op: OpKind, lock: Option<AxisLock>, modal: bool, mouse: Pos2) -> Self {
        Self::starting(Moving::Rig(rig), scene.rigs[rig].placement, scene.rig_frame(rig), op, lock, modal, mouse)
    }

    fn starting(moving: Moving, start_pose: Xf, frame: Xf, op: OpKind, lock: Option<AxisLock>, modal: bool, mouse: Pos2) -> Self {
        Self {
            moving,
            op,
            lock,
            modal,
            start_pose,
            start_frame: frame,
            start_mouse: mouse,
            last_angle: f32::NAN,
            angle: 0.0,
            offset: Vec3::ZERO,
            trackball: false,
            last_mouse: mouse,
            current: frame,
            origin: frame,
            extras: Vec::new(),
        }
    }

    /// Adds other selected bones to the drag. A bone whose parent is also moving
    /// already follows it, and moving it again would apply the change twice.
    pub fn with_extras(mut self, scene: &Scene, selected: &[BoneId]) -> Self {
        let Moving::Bone(main) = self.moving else {
            return self;
        };
        let moving: Vec<BoneId> = selected.iter().copied().filter(|b| *b != main).collect();
        let related = |a: BoneId, b: BoneId| {
            let mut at = scene.bones[a].parent;
            while let Some(p) = at {
                if p == b {
                    return true;
                }
                at = scene.bones[p].parent;
            }
            false
        };
        self.extras = moving
            .iter()
            .copied()
            .filter(|b| !related(*b, main) && !related(main, *b))
            .filter(|b| !moving.iter().any(|other| related(*b, *other)))
            .map(|b| (b, scene.bones[b].pose, scene.bones[b].frame))
            .collect();
        self
    }

    fn axis_dir(&self, lock: AxisLock) -> Vec3 {
        axis_vector(lock.axis, lock.space, self.start_frame)
    }

    /// Applies the current mouse position to the bone.
    pub fn update(&mut self, scene: &mut Scene, camera: &OrbitCamera, rect: Rect, mouse: Pos2) {
        let pivot = self.start_frame.pos;
        let frame = match self.op {
            OpKind::Move => {
                let start = camera.screen_ray(self.start_mouse, rect);
                let now = camera.screen_ray(mouse, rect);
                self.offset = match self.lock {
                    Some(lock) => {
                        let dir = self.axis_dir(lock);
                        match (
                            closest_on_line(pivot, dir, start.origin, start.dir),
                            closest_on_line(pivot, dir, now.origin, now.dir),
                        ) {
                            (Some(a), Some(b)) => dir * (b - a).clamp(-5000.0, 5000.0),
                            _ => self.offset,
                        }
                    }
                    None => {
                        let n = camera.to_camera();
                        match (ray_plane(start, pivot, n), ray_plane(now, pivot, n)) {
                            (Some(a), Some(b)) => b - a,
                            _ => self.offset,
                        }
                    }
                };
                Xf::new(self.start_frame.rot, pivot + self.offset)
            }
            OpKind::Rotate if self.trackball => {
                // Rolls the bone like a ball under the cursor, going by how far the mouse is
                // from where it began. Adding up each step instead leaves a twist behind
                // after a circle, so bringing the mouse back wouldn't bring the bone back.
                let d = (mouse - self.start_mouse) * TRACKBALL_SPEED;
                let turn = Quat::from_scaled_axis(camera.up() * d.x + camera.right() * d.y);
                Xf::new((turn * self.start_frame.rot).normalize(), pivot)
            }
            OpKind::Rotate => {
                let center = camera.world_to_screen(pivot, rect).unwrap_or(rect.center());
                let d = mouse - center;
                let screen_angle = (-d.y).atan2(d.x);
                if self.last_angle.is_nan() {
                    let d0 = self.start_mouse - center;
                    self.last_angle = (-d0.y).atan2(d0.x);
                }
                // Unwrapped, so spinning past 180 degrees keeps going.
                let mut step = screen_angle - self.last_angle;
                if step > PI {
                    step -= TAU;
                } else if step < -PI {
                    step += TAU;
                }
                self.last_angle = screen_angle;
                self.angle += step;
                let to_camera = camera.to_camera();
                // Counter-clockwise on screen is positive around an axis aimed at the viewer.
                let (axis, sign) = match self.lock {
                    Some(lock) => {
                        let a = self.axis_dir(lock);
                        (a, if a.dot(to_camera) >= 0.0 { 1.0 } else { -1.0 })
                    }
                    None => (to_camera, 1.0),
                };
                Xf::new(
                    (Quat::from_axis_angle(axis, self.angle * sign) * self.start_frame.rot).normalize(),
                    pivot,
                )
            }
        };
        self.last_mouse = mouse;
        self.current = frame;
        let bone = match self.moving {
            Moving::Bone(bone) => bone,
            Moving::Rig(rig) => {
                scene.set_rig_placement(rig, frame * self.origin.inverse() * self.start_pose);
                return;
            }
        };
        // Each extra bone turns the same way around its own joint, and moves as far.
        let turn = frame.rot * self.origin.rot.inverse();
        let shift = frame.pos - self.origin.pos;
        for (bone, _, start) in &self.extras {
            let target = Xf::new((turn * start.rot).normalize(), start.pos + shift);
            scene.bones[*bone].pose = scene.bones[*bone].base.inverse() * target;
        }
        scene.set_bone_frame(bone, frame);
    }

    /// Carries on from the current pose after switching rotate modes.
    fn rebase(&mut self) {
        self.start_frame = self.current;
        self.start_mouse = self.last_mouse;
        self.last_angle = f32::NAN;
        self.angle = 0.0;
    }

    /// R during a rotate switches between view rotation and free rotation.
    pub fn press_rotate(&mut self) {
        if self.op == OpKind::Rotate {
            self.rebase();
            self.lock = None;
            self.trackball = !self.trackball;
        }
    }

    /// Blender's X, X, X cycle: global axis, then local axis, then free.
    pub fn press_axis(&mut self, axis: usize) {
        if self.trackball {
            self.rebase();
            self.trackball = false;
        }
        self.lock = match self.lock {
            Some(l) if l.axis == axis && l.space == Space::Global => Some(AxisLock {
                axis,
                space: Space::Local,
            }),
            Some(l) if l.axis == axis => None,
            _ => Some(AxisLock {
                axis,
                space: Space::Global,
            }),
        };
    }

    pub fn cancel(&self, scene: &mut Scene) {
        for (bone, pose, _) in &self.extras {
            scene.bones[*bone].pose = *pose;
        }
        match self.moving {
            Moving::Bone(bone) => scene.set_bone_pose(bone, self.start_pose),
            Moving::Rig(rig) => scene.set_rig_placement(rig, self.start_pose),
        }
    }

    /// Short text for the viewport while dragging.
    pub fn readout(&self) -> String {
        let axis = match self.lock {
            Some(l) => format!(
                " along {} ({})",
                AXIS_NAMES[l.axis],
                l.space.label().to_lowercase()
            ),
            None => String::new(),
        };
        match self.op {
            OpKind::Move => format!("Move {:.3} studs{axis}", self.offset.length()),
            OpKind::Rotate if self.trackball => "Free Rotate".to_string(),
            OpKind::Rotate => format!("Rotate {:.1}\u{b0}{axis}", self.angle.to_degrees()),
        }
    }
}

fn axis_vector(axis: usize, space: Space, frame: Xf) -> Vec3 {
    let v = [Vec3::X, Vec3::Y, Vec3::Z][axis];
    match space {
        Space::Global => v,
        Space::Local => frame.rot * v,
    }
}

fn ray_plane(ray: Ray, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let denom = ray.dir.dot(normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    Some(ray.origin + ray.dir * ((point - ray.origin).dot(normal) / denom))
}

/// A handle on the gizmo under the mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    MoveAxis(usize),
    MoveFree,
    RotateAxis(usize),
    RotateView,
}

impl Handle {
    pub fn session_args(self, space: Space) -> (OpKind, Option<AxisLock>) {
        match self {
            Handle::MoveAxis(axis) => (OpKind::Move, Some(AxisLock { axis, space })),
            Handle::MoveFree => (OpKind::Move, None),
            Handle::RotateAxis(axis) => (OpKind::Rotate, Some(AxisLock { axis, space })),
            Handle::RotateView => (OpKind::Rotate, None),
        }
    }
}

fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-4)).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

/// The screen-space polyline of a ring, with a flag for the half facing the viewer.
fn ring_points(camera: &OrbitCamera, rect: Rect, center: Vec3, axis: Vec3, radius: f32) -> Vec<(Pos2, bool)> {
    let u = axis.any_orthonormal_vector();
    let v = axis.cross(u);
    let to_camera = if camera.is_ortho() {
        camera.to_camera()
    } else {
        (camera.eye() - center).normalize()
    };
    // A ring nearly facing the viewer has no meaningful back half. Off-center
    // perspective would otherwise hide half of a ring that looks fully round.
    let face_on = axis.dot(to_camera).abs() > 0.8;
    (0..=RING_SEGMENTS)
        .filter_map(|i| {
            let a = TAU * i as f32 / RING_SEGMENTS as f32;
            let dir = u * a.cos() + v * a.sin();
            let facing = face_on || dir.dot(to_camera) > -0.02;
            camera.world_to_screen(center + dir * radius, rect).map(|s| (s, facing))
        })
        .collect()
}

pub struct GizmoView<'a> {
    pub camera: &'a OrbitCamera,
    pub rect: Rect,
    pub frame: Xf,
    pub space: Space,
    pub op: OpKind,
    /// Multiplies every handle's on-screen size.
    pub size: f32,
}

impl GizmoView<'_> {
    fn axis(&self, i: usize) -> Vec3 {
        axis_vector(i, self.space, self.frame)
    }

    fn world_scale(&self) -> f32 {
        self.camera.world_per_point(self.frame.pos, self.rect) * self.size
    }

    pub fn hit_test(&self, mouse: Pos2) -> Option<Handle> {
        let center = self.camera.world_to_screen(self.frame.pos, self.rect)?;
        let scale = self.world_scale();
        match self.op {
            OpKind::Move => {
                if (mouse - center).length() < 11.0 {
                    return Some(Handle::MoveFree);
                }
                let mut best = None;
                let mut best_dist = PICK_DISTANCE;
                for i in 0..3 {
                    let end = self.frame.pos + self.axis(i) * MOVE_LENGTH * scale;
                    if let Some(end) = self.camera.world_to_screen(end, self.rect) {
                        let d = segment_distance(mouse, center, end);
                        if d < best_dist {
                            best_dist = d;
                            best = Some(Handle::MoveAxis(i));
                        }
                    }
                }
                best
            }
            OpKind::Rotate => {
                let mut best = None;
                let mut best_dist = PICK_DISTANCE;
                for i in 0..3 {
                    let pts = ring_points(self.camera, self.rect, self.frame.pos, self.axis(i), RING_RADIUS * scale);
                    for w in pts.windows(2) {
                        if w[0].1 && w[1].1 {
                            let d = segment_distance(mouse, w[0].0, w[1].0);
                            if d < best_dist {
                                best_dist = d;
                                best = Some(Handle::RotateAxis(i));
                            }
                        }
                    }
                }
                if best.is_none() {
                    let d = ((mouse - center).length() - VIEW_RING_RADIUS * self.size).abs();
                    if d < PICK_DISTANCE {
                        return Some(Handle::RotateView);
                    }
                }
                best
            }
        }
    }

    pub fn paint(&self, painter: &Painter, hot: Option<Handle>) {
        let Some(center) = self.camera.world_to_screen(self.frame.pos, self.rect) else {
            return;
        };
        match self.op {
            OpKind::Move => self.paint_move(painter, center, hot),
            OpKind::Rotate => self.paint_rotate(painter, center, hot),
        }
    }

    fn paint_move(&self, painter: &Painter, center: Pos2, hot: Option<Handle>) {
        let scale = self.world_scale();
        let mut order = [0, 1, 2];
        // Farthest arrow first so the nearest stays on top.
        order.sort_by(|a, b| {
            let (a, b) = (self.axis(*a).dot(self.camera.to_camera()), self.axis(*b).dot(self.camera.to_camera()));
            a.total_cmp(&b)
        });
        for i in order {
            let color = if hot == Some(Handle::MoveAxis(i)) { HOT } else { AXIS_COLORS[i] };
            let end = self.frame.pos + self.axis(i) * MOVE_LENGTH * scale;
            let Some(end) = self.camera.world_to_screen(end, self.rect) else {
                continue;
            };
            painter.line_segment([center, end], Stroke::new(1.8, color));
            let dir = (end - center).normalized();
            if dir.length() > 0.5 {
                let side = dir.rot90() * 4.5;
                let tip = end + dir * 12.0;
                painter.add(egui::Shape::convex_polygon(
                    vec![tip, end + side, end - side],
                    color,
                    Stroke::NONE,
                ));
            }
        }
        let color = if hot == Some(Handle::MoveFree) { HOT } else { Color32::from_rgb(0xB4, 0xB4, 0xB4) };
        painter.circle_stroke(center, 5.0, Stroke::new(1.5, color));
    }

    fn paint_rotate(&self, painter: &Painter, center: Pos2, hot: Option<Handle>) {
        let scale = self.world_scale();
        for i in 0..3 {
            let color = if hot == Some(Handle::RotateAxis(i)) { HOT } else { AXIS_COLORS[i] };
            let width = if hot == Some(Handle::RotateAxis(i)) { 2.6 } else { 1.8 };
            let pts = ring_points(self.camera, self.rect, self.frame.pos, self.axis(i), RING_RADIUS * scale);
            for w in pts.windows(2) {
                let stroke = if w[0].1 && w[1].1 {
                    Stroke::new(width, color)
                } else {
                    Stroke::new(1.0, color.gamma_multiply(0.18))
                };
                painter.line_segment([w[0].0, w[1].0], stroke);
            }
        }
        let color = if hot == Some(Handle::RotateView) { HOT } else { Color32::from_rgb(0xB4, 0xB4, 0xB4) };
        painter.circle_stroke(center, VIEW_RING_RADIUS * self.size, Stroke::new(1.2, color));
            }
}

/// Draws the guide shown during a drag: an infinite-looking line on the locked
/// axis, or a rubber band from the pivot to the mouse when rotating.
pub fn paint_session_guides(painter: &Painter, session: &Session, camera: &OrbitCamera, rect: Rect, mouse: Pos2) {
    let pivot = session.start_frame.pos;
    let Some(center) = camera.world_to_screen(pivot, rect) else {
        return;
    };
    if let Some(lock) = session.lock {
        let dir = session.axis_dir(lock);
        let reach = camera.world_per_point(pivot, rect) * 4000.0;
        if session.op == OpKind::Move {
            if let (Some(a), Some(b)) = (
                camera.world_to_screen(pivot - dir * reach, rect),
                camera.world_to_screen(pivot + dir * reach, rect),
            ) {
                painter.line_segment([a, b], Stroke::new(1.5, AXIS_COLORS[lock.axis]));
            }
        } else {
            let radius = camera.world_per_point(pivot, rect) * RING_RADIUS;
            let pts = ring_points(camera, rect, pivot, dir, radius);
            for w in pts.windows(2) {
                painter.line_segment([w[0].0, w[1].0], Stroke::new(1.8, AXIS_COLORS[lock.axis]));
            }
        }
    }
    if session.trackball {
        painter.circle_stroke(center, VIEW_RING_RADIUS, Stroke::new(1.2, Color32::from_white_alpha(120)));
    } else if session.op == OpKind::Rotate {
        painter.line_segment([center, mouse], Stroke::new(1.0, Color32::from_white_alpha(90)));
    }
}

/// Screen distance from a point to a bone's drawn segment, for picking.
pub fn bone_screen_distance(scene: &Scene, bone: BoneId, camera: &OrbitCamera, rect: Rect, mouse: Pos2) -> Option<f32> {
    let b = &scene.bones[bone];
    let a = camera.world_to_screen(b.head(), rect)?;
    let c = camera.world_to_screen(b.tail(), rect)?;
    Some(segment_distance(mouse, a, c))
}

