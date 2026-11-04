//! Ragdoll: the modifier that depends on what already happened, so it can't be worked out
//! from one frame alone. Like the rig logic, this has no interface code.
//!
//! `bake` plays the whole animation once on a copy of the scene, stepping the physics
//! frame by frame, and keeps each result. `Scene::evaluate` then only has to look up the
//! frame showing, so scrubbing to any frame gives the same answer as playing up to it, and
//! an export carries the result through `Bone::output` like any other modifier.
//!
//! How strongly it acts is `Bone::value`, so turning a Ragdoll's influence on with a key
//! is how a character starts falling partway through an animation. Until then it follows
//! its keys, and it falls with the speed it had.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::Hasher;
use std::sync::Arc;

use glam::{Quat, Vec3};

use crate::animation::{Animation, FrameNumber};
use crate::math::Xf;
use crate::modifiers::{self, Modifier, ModifierKind};
use crate::scene::{BoneId, BoneLink, BuiltShape, MeshFit, PartShape, RigId, Scene};

/// Physics steps per frame. Small steps are what keep joints from stretching.
const SUBSTEPS: usize = 8;
/// How much less a moving object's side and top faces count than the one it's leading with.
const AGAINST_MOTION: f32 = 4.0;
/// How far a ragdoll is pushed out of something in one step, so starting inside it
/// doesn't fire the body off.
const DEEPEST_PUSH: f32 = 0.2;
/// How quickly a ragdoll loses its spin and speed, per second.
const SPIN_DAMPING: f32 = 1.5;
const SPEED_DAMPING: f32 = 0.1;
/// How much a body sliding on the floor is held back, as a share of how hard it's pressed.
const FRICTION: f32 = 0.8;

/// A bone whose pose comes from physics, since nothing the keys say matters to something
/// falling.
#[derive(Clone, Copy, Debug)]
struct Effect {
    bone: BoneId,
    /// The bone holding the modifier, whose value says how strongly this acts.
    owner: BoneId,
}

/// Every result for every frame.
#[derive(Debug)]
pub struct Baked {
    start: FrameNumber,
    effects: Vec<Effect>,
    frames: Vec<Vec<Xf>>,
}

/// What `Scene::evaluate` needs to show physics. Never saved, since it can be worked out.
#[derive(Clone, Debug, Default)]
pub struct Dynamics {
    /// Where the ground is. Ragdolls land on it.
    pub floor: f32,
    /// The frame showing.
    pub(crate) frame: f32,
    baked: Option<Arc<Baked>>,
    /// The results so far, while `bake` is still working them out.
    live: Option<Vec<(Effect, Xf)>>,
}

impl Dynamics {
    pub fn set_baked(&mut self, baked: Option<Arc<Baked>>) {
        self.baked = baked;
    }

    pub fn has_baked(&self) -> bool {
        self.baked.is_some()
    }
}

fn lerp(a: Xf, b: Xf, t: f32) -> Xf {
    Xf::new(a.rot.slerp(b.rot, t).normalize(), a.pos.lerp(b.pos, t))
}

/// The results for this point in time.
fn results(scene: &Scene) -> Vec<(Effect, Xf)> {
    if let Some(live) = &scene.dynamics.live {
        return live.clone();
    }
    let Some(baked) = &scene.dynamics.baked else {
        return Vec::new();
    };
    let Some(last) = baked.frames.len().checked_sub(1) else {
        return Vec::new();
    };
    let at = (scene.dynamics.frame - baked.start as f32).clamp(0.0, last as f32);
    let (first, t) = (at.floor() as usize, at.fract());
    let (a, b) = (&baked.frames[first], &baked.frames[(first + 1).min(last)]);
    baked.effects.iter().enumerate().map(|(i, effect)| (*effect, lerp(a[i], b[i], t))).collect()
}

/// Puts the physics results on top of what the keys and other modifiers said.
pub(crate) fn apply(scene: &mut Scene) {
    let entries = results(scene);
    if entries.is_empty() {
        return;
    }
    let mut changed = false;
    for (effect, result) in entries {
        let found = scene.modifiers.iter().find(|m| m.bone == effect.owner && is_dynamic(m));
        let amount = found.map_or(0.0, |m| modifiers::influence(scene, m));
        if amount <= 0.0 {
            continue;
        }
        // A replay of a project that's since been closed can still be arriving.
        let Some(bone) = scene.bones.get_mut(effect.bone) else {
            continue;
        };
        bone.solved = lerp(bone.solved, result, amount);
        changed = true;
    }
    if changed {
        scene.place_all();
    }
}

fn is_dynamic(modifier: &Modifier) -> bool {
    matches!(modifier.kind, ModifierKind::Ragdoll(_))
}

pub fn has_dynamics(scene: &Scene) -> bool {
    scene.modifiers.iter().any(is_dynamic)
}

/// A box something can bump into.
#[derive(Clone, Copy)]
struct Collider {
    at: Xf,
    half: Vec3,
    /// Round, with the smallest of `half` for a radius. Everything else is treated as a box.
    ball: bool,
}

fn part_box(scene: &Scene, part: usize) -> Collider {
    let p = &scene.parts[part];
    let (offset, size) = p.drawn_box();
    let ball = p.shape == PartShape::Ball || matches!(p.mesh_fit, MeshFit::Built { shape: BuiltShape::Sphere, .. });
    Collider { at: p.world * Xf::new(Quat::IDENTITY, offset), half: size * 0.5, ball }
}

/// Everything solid in a rig. A hidden rig is still there, but helpers such as
/// HumanoidRootPart were never meant to be touched.
fn rig_colliders(scene: &Scene, rig: RigId, into: &mut Vec<Collider>) {
    for part in &scene.rigs[rig].parts {
        if !scene.parts[*part].is_helper() && !scene.part_is_removed(*part) {
            into.push(part_box(scene, *part));
        }
    }
}

fn subtree(scene: &Scene, bone: BoneId, into: &mut Vec<BoneId>) {
    if scene.bones[bone].removed {
        return;
    }
    into.push(bone);
    for child in &scene.bones[bone].children {
        subtree(scene, *child, into);
    }
}

/// Which bones a Ragdoll sets loose, parents always before children. Without Affects Body
/// that's only the limb: the bone and the chain below it, stopping at a hub. On a torso,
/// that's just the torso, and the other limbs stay as they are.
pub fn ragdoll_bones(scene: &Scene, bone: BoneId, affects_body: bool) -> Vec<BoneId> {
    if !affects_body {
        let mut bones = vec![bone];
        let mut at = bone;
        loop {
            let below: Vec<BoneId> = scene.bones[at].children.iter().copied().filter(|b| !scene.bones[*b].removed).collect();
            let [only] = below.as_slice() else {
                return bones;
            };
            bones.push(*only);
            at = *only;
        }
    }
    let rig = scene.bones[bone].rig;
    let joints = scene.joints(rig);
    let mut bones = Vec::with_capacity(joints.len());
    for joint in &joints {
        if scene.bones[*joint].parent.is_none_or(|p| !joints.contains(&p)) {
            subtree(scene, *joint, &mut bones);
        }
    }
    bones
}

fn depth(scene: &Scene, mut bone: BoneId) -> usize {
    let mut count = 0;
    while let Some(parent) = scene.bones[bone].parent {
        bone = parent;
        count += 1;
    }
    count
}

// Contacts.

/// Points all over a box, in its own space: corners, edges and faces.
fn box_points(half: Vec3) -> Vec<Vec3> {
    let mut points = Vec::with_capacity(26);
    for x in [-1.0, 0.0, 1.0] {
        for y in [-1.0, 0.0, 1.0] {
            for z in [-1.0, 0.0, 1.0] {
                if x != 0.0 || y != 0.0 || z != 0.0 {
                    points.push(Vec3::new(x, y, z) * half);
                }
            }
        }
    }
    points
}

/// How far a box reaches from its middle along a direction.
fn reach_along(at: Xf, half: Vec3, way: Vec3) -> f32 {
    let local = at.rot.conjugate() * way;
    local.abs().dot(half)
}

/// The one way a box has to go to get out of another, so every point on it is pushed
/// alike and a flat push moves it flat. None when they don't overlap. Deep inside
/// something on the move, the way out is the way that's going, since that's being dragged
/// along. The shortest way could send a limb out the top of a body walking past.
fn way_out(at: Xf, half: Vec3, object: &Collider, earlier: &Collider) -> Option<Vec3> {
    let apart = at.pos - object.at.pos;
    let axes = [at.rot * Vec3::X, at.rot * Vec3::Y, at.rot * Vec3::Z, object.at.rot * Vec3::X, object.at.rot * Vec3::Y, object.at.rot * Vec3::Z];
    let moved = (object.at.pos - earlier.at.pos).try_normalize();
    let (mut best, mut least, mut score) = (Vec3::Y, f32::INFINITY, f32::INFINITY);
    for axis in axes {
        let overlap = reach_along(at, half, axis) + reach_along(object.at, object.half, axis) - apart.dot(axis).abs();
        if overlap <= 0.0 {
            return None;
        }
        let way = if apart.dot(axis) < 0.0 { -axis } else { axis };
        // Something on the move pushes ahead of itself, so a face it isn't leading with
        // only wins for a light brush. Otherwise the top of a leg walking past lifts an arm.
        let against = moved.map_or(0.0, |m| 1.0 - m.dot(way).max(0.0));
        let weighed = overlap * (1.0 + AGAINST_MOTION * against);
        least = least.min(overlap);
        if weighed < score {
            score = weighed;
            best = way;
        }
    }
    Some(moved.filter(|_| least > half.min_element()).unwrap_or(best))
}

/// Where a box and an object overlap: each point, which way it's pushed and how far.
fn contacts(at: Xf, half: Vec3, object: &Collider, earlier: &Collider, into: &mut Vec<(Vec3, Vec3, f32)>) {
    if at.pos.distance(object.at.pos) > half.length() + object.half.length() {
        return;
    }
    if object.ball {
        let (center, radius) = (object.at.pos, object.half.min_element());
        // The nearest point counts too, since a ball can touch between the others.
        let nearest = at.point(at.inverse().point(center).clamp(-half, half));
        for point in box_points(half).into_iter().map(|p| at.point(p)).chain([nearest]) {
            let off = point - center;
            let distance = off.length();
            if distance < radius {
                let way = off.try_normalize().or((at.pos - center).try_normalize()).unwrap_or(Vec3::Y);
                into.push((point, way, radius - distance));
            }
        }
        return;
    }
    let Some(way) = way_out(at, half, object, earlier) else {
        return;
    };
    let face = object.at.pos.dot(way) + reach_along(object.at, object.half, way);
    let inverse = object.at.inverse();
    for point in box_points(half).into_iter().map(|p| at.point(p)) {
        let local = inverse.point(point);
        if local.abs().cmple(object.half).all() {
            into.push((point, way, face - point.dot(way)));
        }
    }
    // A corner of the object can poke into a face between those points.
    let back = at.pos.dot(way) - reach_along(at, half, way);
    let inverse = at.inverse();
    for corner in box_points(object.half).into_iter().map(|p| object.at.point(p)) {
        if inverse.point(corner).abs().cmple(half).all() {
            into.push((corner, way, corner.dot(way) - back));
        }
    }
}

// Ragdoll. Each bone is a rigid body, joined up by position constraints solved in small
// steps, which is a simple way to stay stable with limbs of very different weights.

#[derive(Clone, Copy, PartialEq)]
enum Link {
    /// Nothing holds it up, as for the top of a whole body falling.
    Free,
    /// Its joint stays where the keys put it, for a limb still on a body that isn't falling.
    Held,
    Body(usize),
}

struct Body {
    bone: BoneId,
    link: Link,
    /// The middle of its weight, in the bone's space.
    middle: Vec3,
    inverse_mass: f32,
    inverse_inertia: f32,
    corners: [Vec3; 8],
    /// Its box, in the bone's space.
    shape: Xf,
    half: Vec3,
    /// Where its joint sits on what it hangs from, in that bone's space.
    anchor: Vec3,
    /// Where its base sits on what it hangs from, which doesn't change.
    hung: Xf,
    /// Its turn at rest, relative to what it hangs from.
    rest_turn: Quat,
    limit: f32,
    /// The middle of its weight in the world.
    pos: Vec3,
    rot: Quat,
    velocity: Vec3,
    spin: Vec3,
    start_pos: Vec3,
    start_rot: Quat,
    /// What the keys say, on the last frame and this one.
    keyed: Xf,
    keyed_base: Xf,
}

impl Body {
    fn origin(&self) -> Vec3 {
        self.pos - self.rot * self.middle
    }

    fn frame(&self) -> Xf {
        Xf::new(self.rot, self.origin())
    }

    fn set_frame(&mut self, frame: Xf) {
        self.rot = frame.rot;
        self.pos = frame.point(self.middle);
    }

    /// Moves it by a push at a point `lever` away from the middle of its weight.
    fn push(&mut self, impulse: Vec3, lever: Vec3) {
        self.pos += impulse * self.inverse_mass;
        self.turn_by(lever.cross(impulse) * self.inverse_inertia);
    }

    fn turn_by(&mut self, amount: Vec3) {
        let change = Quat::from_xyzw(amount.x, amount.y, amount.z, 0.0) * self.rot;
        self.rot = Quat::from_xyzw(self.rot.x + change.x * 0.5, self.rot.y + change.y * 0.5, self.rot.z + change.z * 0.5, self.rot.w + change.w * 0.5).normalize();
    }

    /// How hard it is to move at a point, along a direction.
    fn give(&self, lever: Vec3, way: Vec3) -> f32 {
        self.inverse_mass + lever.cross(way).length_squared() * self.inverse_inertia
    }
}

/// How far a joint can turn from rest, in degrees. Goes by Roblox's joint names, so
/// anything else gets the loose default.
fn joint_limit(name: &str) -> f32 {
    let has = |word: &str| name.contains(word);
    if has("Neck") || has("Wrist") || has("Ankle") {
        50.0
    } else if has("Waist") {
        40.0
    } else if has("Elbow") || has("Knee") {
        120.0
    } else {
        110.0
    }
}

fn new_body(scene: &Scene, rest: &crate::scene::RestPose, bone: BoneId, link: Link) -> Body {
    let b = &scene.bones[bone];
    let (middle, half, turn) = match b.link {
        BoneLink::Joint { part1, c1, .. } => {
            let part = &scene.parts[part1];
            let (offset, size) = part.drawn_box();
            let held = c1.inverse() * Xf::new(Quat::IDENTITY, offset);
            (held.pos, size * 0.5, held.rot)
        }
        _ => {
            let reach = b.tail_local.length().max(0.2);
            let thick = reach * 0.15;
            let along = b.tail_local.try_normalize().unwrap_or(Vec3::Y);
            (b.tail_local * 0.5, Vec3::new(thick, reach * 0.5, thick), Quat::from_rotation_arc(Vec3::Y, along))
        }
    };
    let half = half.max(Vec3::splat(0.05));
    let size = half * 2.0;
    let mass = (size.x * size.y * size.z).clamp(0.2, 20.0);
    let inertia = mass * size.length_squared() / 12.0;
    let corners = std::array::from_fn(|i| {
        let sign = |bit: usize| if i & (1 << bit) == 0 { -1.0 } else { 1.0 };
        middle + turn * Vec3::new(half.x * sign(0), half.y * sign(1), half.z * sign(2))
    });
    let parent_frame = match link {
        Link::Body(_) => b.parent.map(|p| rest.bones[p]),
        _ => None,
    };
    let rest_frame = rest.bones[bone];
    let hung = parent_frame.map_or(Xf::IDENTITY, |parent| parent.inverse() * rest_frame);
    Body {
        bone,
        link,
        middle,
        inverse_mass: 1.0 / mass,
        inverse_inertia: 1.0 / inertia,
        corners,
        shape: Xf::new(turn, middle),
        half,
        anchor: hung.pos,
        hung,
        rest_turn: hung.rot,
        limit: joint_limit(&b.name).to_radians(),
        pos: Vec3::ZERO,
        rot: Quat::IDENTITY,
        velocity: Vec3::ZERO,
        spin: Vec3::ZERO,
        start_pos: Vec3::ZERO,
        start_rot: Quat::IDENTITY,
        keyed: b.frame,
        keyed_base: b.base,
    }
}

/// One Ragdoll as it plays.
struct Ragdoll {
    owner: BoneId,
    gravity: f32,
    /// Going by its own physics, and not following the keys for now.
    loose: bool,
    bodies: Vec<Body>,
    started: bool,
    /// The rigs and shapes it lands on, besides the floor.
    objects: Vec<RigId>,
    /// Where those were on the last frame and this one.
    boxes_was: Vec<Collider>,
    boxes_now: Vec<Collider>,
}

impl Ragdoll {
    /// How far this body's turn is from rest, relative to what it hangs from.
    fn over_limit(&self, index: usize, held_turn: Quat) -> Option<(Vec3, f32)> {
        let body = &self.bodies[index];
        let parent = match body.link {
            Link::Body(p) => self.bodies[p].rot,
            Link::Held => held_turn,
            Link::Free => return None,
        };
        let off = parent.conjugate() * body.rot * body.rest_turn.conjugate();
        let (axis, angle) = off.normalize().to_axis_angle();
        let angle = if angle > std::f32::consts::PI { angle - std::f32::consts::TAU } else { angle };
        (angle.abs() > body.limit).then(|| (parent * (axis * angle.signum()), angle.abs() - body.limit))
    }

    /// Lets loose from what the keys say right now, keeping the joints where they are.
    fn let_loose(&mut self) {
        self.loose = true;
        for index in 0..self.bodies.len() {
            let Link::Body(parent) = self.bodies[index].link else {
                continue;
            };
            let keyed = self.bodies[index].keyed;
            let parent = self.bodies[parent].keyed;
            let body = &mut self.bodies[index];
            body.anchor = parent.inverse().point(keyed.pos);
            let turned = parent.rot.conjugate() * keyed.rot * body.rest_turn.conjugate();
            // Posed past its limit already, it shouldn't snap back the moment it's let go.
            let angle = turned.normalize().to_axis_angle().1;
            let angle = if angle > std::f32::consts::PI { std::f32::consts::TAU - angle } else { angle };
            body.limit = body.limit.max(angle + 0.05);
        }
    }

    fn follow_keys(&mut self, dt: f32) {
        for body in &mut self.bodies {
            let was = body.frame();
            body.set_frame(body.keyed);
            if self.started {
                body.velocity = (body.pos - was.point(body.middle)) / dt;
                let change = (body.rot * was.rot.conjugate()).normalize();
                let change = if change.w < 0.0 { -change } else { change };
                body.spin = Vec3::new(change.x, change.y, change.z) * 2.0 / dt;
            }
        }
    }

    fn substep(&mut self, h: f32, t: f32, was: &[Xf], floor: f32) {
        for body in &mut self.bodies {
            body.start_pos = body.pos;
            body.start_rot = body.rot;
            body.velocity.y -= self.gravity * h;
            body.pos += body.velocity * h;
            body.turn_by(body.spin * h);
        }
        for index in 0..self.bodies.len() {
            match self.bodies[index].link {
                Link::Free => {}
                Link::Held => {
                    let goal = was[index].pos.lerp(self.bodies[index].keyed.pos, t);
                    let body = &mut self.bodies[index];
                    let lever = body.origin() - body.pos;
                    let gap = goal - body.origin();
                    if let Some(way) = gap.try_normalize() {
                        let push = way * (gap.length() / body.give(lever, way));
                        body.push(push, lever);
                    }
                }
                Link::Body(parent) => {
                    let (hold, parent_lever) = {
                        let p = &self.bodies[parent];
                        let hold = p.frame().point(self.bodies[index].anchor);
                        (hold, hold - p.pos)
                    };
                    let body = &self.bodies[index];
                    let lever = body.origin() - body.pos;
                    let gap = hold - body.origin();
                    if let Some(way) = gap.try_normalize() {
                        let push = way * (gap.length() / (body.give(lever, way) + self.bodies[parent].give(parent_lever, way)));
                        self.bodies[index].push(push, lever);
                        self.bodies[parent].push(-push, parent_lever);
                    }
                }
            }
            let held_turn = self.bodies[index].keyed_base.rot;
            if let Some((axis, excess)) = self.over_limit(index, held_turn) {
                let own = self.bodies[index].inverse_inertia;
                let other = match self.bodies[index].link {
                    Link::Body(parent) => self.bodies[parent].inverse_inertia,
                    _ => 0.0,
                };
                let share = own / (own + other);
                self.bodies[index].turn_by(axis * (-excess * share));
                if let Link::Body(parent) = self.bodies[index].link {
                    self.bodies[parent].turn_by(axis * (excess * (1.0 - share)));
                }
            }
        }
        for body in &mut self.bodies {
            for corner in body.corners {
                let point = body.pos + body.rot * (corner - body.middle);
                let depth = floor - point.y;
                if depth <= 0.0 {
                    continue;
                }
                let lever = point - body.pos;
                body.push(Vec3::Y * (depth / body.give(lever, Vec3::Y)), lever);
                // Friction, from how far this corner slid over the floor during this step.
                let start = body.start_pos + body.start_rot * (corner - body.middle);
                let after = body.pos + body.rot * (corner - body.middle);
                let slid = Vec3::new(after.x - start.x, 0.0, after.z - start.z);
                if let Some(way) = slid.try_normalize() {
                    let held = slid.length().min(FRICTION * depth);
                    let lever = after - body.pos;
                    body.push(way * (-held / body.give(lever, way)), lever);
                }
            }
        }
        if !self.boxes_now.is_empty() {
            let boxes_at = |t: f32| -> Vec<Collider> { self.boxes_was.iter().zip(&self.boxes_now).map(|(a, b)| Collider { at: lerp(a.at, b.at, t), ..*b }).collect() };
            let (objects, earlier) = (boxes_at(t), boxes_at((t - 1.0 / SUBSTEPS as f32).max(0.0)));
            for body in &mut self.bodies {
                let mut found = Vec::new();
                for (object, before) in objects.iter().zip(&earlier) {
                    contacts(body.frame() * body.shape, body.half, object, before, &mut found);
                }
                for (point, way, depth) in found {
                    let lever = point - body.pos;
                    body.push(way * (depth.min(DEEPEST_PUSH) / body.give(lever, way)), lever);
                }
            }
        }
        for body in &mut self.bodies {
            body.velocity = (body.pos - body.start_pos) / h * (-SPEED_DAMPING * h).exp();
            let change = (body.rot * body.start_rot.conjugate()).normalize();
            let change = if change.w < 0.0 { -change } else { change };
            body.spin = Vec3::new(change.x, change.y, change.z) * (2.0 / h) * (-SPIN_DAMPING * h).exp();
        }
    }

    fn step(&mut self, world: &Scene, dt: f32) -> Vec<Xf> {
        let was: Vec<Xf> = self.bodies.iter().map(|b| if self.started { b.keyed } else { world.bones[b.bone].frame }).collect();
        for body in &mut self.bodies {
            body.keyed = world.bones[body.bone].frame;
            body.keyed_base = world.bones[body.bone].base;
            if !self.started {
                body.set_frame(body.keyed);
            }
        }
        let mut boxes = Vec::new();
        for rig in &self.objects {
            if !world.rigs[*rig].removed {
                rig_colliders(world, *rig, &mut boxes);
            }
        }
        self.boxes_was = if self.boxes_now.len() == boxes.len() { std::mem::take(&mut self.boxes_now) } else { boxes.clone() };
        self.boxes_now = boxes;
        let awake = world.modifiers.iter().find(|m| m.bone == self.owner && matches!(m.kind, ModifierKind::Ragdoll(_))).is_some_and(|m| modifiers::influence(world, m) > 0.0);
        if !awake {
            self.loose = false;
        } else if !self.loose {
            self.let_loose();
        }
        if self.loose {
            let h = dt / SUBSTEPS as f32;
            for index in 1..=SUBSTEPS {
                self.substep(h, index as f32 / SUBSTEPS as f32, &was, world.dynamics.floor);
            }
        } else {
            self.follow_keys(dt);
        }
        self.started = true;
        // A pose is relative to the base, which hangs from the parent as it really landed.
        let mut frames: Vec<Xf> = Vec::with_capacity(self.bodies.len());
        let mut out = Vec::with_capacity(self.bodies.len());
        for body in &self.bodies {
            let frame = if self.loose { body.frame() } else { body.keyed };
            let base = match body.link {
                Link::Body(parent) => frames[parent] * body.hung,
                _ => body.keyed_base,
            };
            frames.push(frame);
            out.push(base.inverse() * frame);
        }
        out
    }
}

fn new_ragdolls(scene: &Scene) -> Vec<Ragdoll> {
    let mut wanted: Vec<(BoneId, f32, bool, Vec<RigId>)> = scene
        .modifiers
        .iter()
        .filter_map(|m| match m.kind {
            ModifierKind::Ragdoll(ragdoll) if !scene.bones[m.bone].removed => Some((m.bone, ragdoll.gravity, ragdoll.affects_body, ragdoll.objects.iter().flatten().copied().collect())),
            _ => None,
        })
        .collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    // A whole body takes in any limb, and a limb takes in any bone below it.
    wanted.sort_by_key(|(bone, _, whole, _)| (!*whole, depth(scene, *bone)));
    let rest = scene.rest_pose();
    let mut claimed: BTreeSet<BoneId> = BTreeSet::new();
    let mut ragdolls = Vec::new();
    for (owner, gravity, whole, objects) in wanted {
        let bones: Vec<BoneId> = ragdoll_bones(scene, owner, whole).into_iter().filter(|b| !claimed.contains(b)).collect();
        if bones.is_empty() {
            continue;
        }
        claimed.extend(bones.iter().copied());
        let mut bodies: Vec<Body> = Vec::with_capacity(bones.len());
        for bone in &bones {
            let parent = scene.bones[*bone].parent.and_then(|p| bones.iter().position(|b| *b == p));
            let link = match parent {
                Some(parent) => Link::Body(parent),
                None if whole => Link::Free,
                None => Link::Held,
            };
            bodies.push(new_body(scene, &rest, *bone, link));
        }
        ragdolls.push(Ragdoll { owner, gravity, loose: false, bodies, started: false, objects, boxes_was: Vec::new(), boxes_now: Vec::new() });
    }
    ragdolls
}

/// Plays the animation once and keeps what Ragdoll does on every frame. Returns `None` when
/// nothing in the scene needs it.
pub fn bake(scene: &Scene, animation: &Animation) -> Option<Arc<Baked>> {
    if !has_dynamics(scene) {
        return None;
    }
    let mut world = scene.clone();
    world.dynamics.baked = None;
    let mut ragdolls = new_ragdolls(scene);
    let mut effects = Vec::new();
    for ragdoll in &ragdolls {
        effects.extend(ragdoll.bodies.iter().map(|b| Effect { bone: b.bone, owner: ragdoll.owner }));
    }

    let (start, end) = (animation.start, animation.end.max(animation.start));
    let dt = 1.0 / animation.fps;
    let mut frames = Vec::with_capacity((end - start + 1) as usize);
    for frame in start..=end {
        // First what the keys and the other modifiers say, with no physics at all.
        world.dynamics.live = Some(Vec::new());
        animation.apply(&mut world, frame as f32);
        let mut poses: Vec<Xf> = Vec::with_capacity(effects.len());
        for ragdoll in &mut ragdolls {
            poses.extend(ragdoll.step(&world, dt));
        }
        frames.push(poses);
    }
    Some(Arc::new(Baked { start, effects, frames }))
}

/// What `bake` found, along with where each motion path runs.
pub struct Replay {
    pub baked: Option<Arc<Baked>>,
    pub paths: MotionPaths,
}

/// Where the widest part of each bone goes, on each frame of the animation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MotionPaths {
    pub start: FrameNumber,
    pub points: BTreeMap<BoneId, Vec<Vec3>>,
}

/// Plays the animation through, for what can't be worked out from one frame. This is the
/// slow part, so it's given its own copies and can run on another thread.
pub fn replay(mut scene: Scene, animation: Animation, path_bones: Vec<BoneId>) -> Replay {
    let baked = bake(&scene, &animation);
    let mut paths = MotionPaths { start: animation.start, points: BTreeMap::new() };
    if !path_bones.is_empty() {
        scene.dynamics.baked = baked.clone();
        scene.dynamics.live = None;
        for bone in &path_bones {
            paths.points.insert(*bone, Vec::new());
        }
        for frame in animation.start..=animation.end.max(animation.start) {
            animation.apply(&mut scene, frame as f32);
            for bone in &path_bones {
                if let (Some(points), Some(b)) = (paths.points.get_mut(bone), scene.bones.get(*bone)) {
                    points.push(b.widest());
                }
            }
        }
    }
    Replay { baked, paths }
}

struct HashWriter(DefaultHasher);

impl std::io::Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.write(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Stands for everything a replay depends on, so a new one only runs when it would come
/// out different. It compares the work itself, so no edit can forget to ask for one.
pub fn signature(scene: &Scene, animation: &Animation, path_bones: &[BoneId]) -> u64 {
    let unkeyed: Vec<(BoneId, Xf, f32)> = scene.bones.iter().enumerate().filter(|(id, _)| !animation.tracks.contains_key(id)).map(|(id, b)| (id, b.pose, b.value)).collect();
    let links: Vec<(BoneLink, bool)> = scene.bones.iter().map(|b| (b.link, b.removed)).collect();
    let parts: Vec<(Vec3, f32, bool)> = scene.parts.iter().map(|p| (p.size, p.transparency, p.removed)).collect();
    let rigs: Vec<(Xf, bool)> = scene.rigs.iter().map(|r| (r.placement, r.removed)).collect();
    let mut writer = HashWriter(DefaultHasher::new());
    let work = (&scene.modifiers, &animation.tracks, (animation.start, animation.end, animation.fps), (animation.smoothing, scene.dynamics.floor, path_bones), (unkeyed, links, parts, rigs));
    serde_json::to_writer(&mut writer, &work).expect("this can always be written");
    writer.0.finish()
}
