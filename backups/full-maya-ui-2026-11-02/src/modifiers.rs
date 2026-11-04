//! Modifiers change where a bone ends up without touching its keys, so turning one
//! off always gets the original animation back. Like the rig logic, this has no
//! interface code.
//!
//! Roblox only understands one `Transform` per joint, so everything here has to come
//! down to that. `Bone::pose` stays what was keyed, and `Bone::output` is what an
//! export needs to write to reproduce the result.
//!
//! - Unparent runs while the rig is placed, by swapping the base a bone hangs from.
//! - IK runs afterward and writes `Bone::solved`, then the rig is placed again.
//!
//! How strongly each one acts is `Bone::value`, on the IK target or on the unparented
//! bone. That makes it animated like anything else, which is how IK and FK get
//! blended over time.

use std::f32::consts::PI;

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::math::Xf;
use crate::scene::{BoneId, BoneKind, BoneLink, Drive, Placement, Scene};

const FABRIK_PASSES: usize = 24;
const FABRIK_TOLERANCE: f32 = 1e-3;
/// How far a straight chain leans toward its bend, as a share of its length. FABRIK
/// can't bend a perfectly straight chain, because every step stays on the line.
const CHAIN_LEAN: f32 = 0.12;
/// How far off the straight line a chain's middle has to sit, as a share of the
/// chain's reach, to count as bent.
const CLEAR_BEND: f32 = 0.08;
/// How far a one-bone limb leans to start with, as a share of how far a real knee
/// would stick out. Much more and the top of the limb comes out of the back of the body.
const DEFAULT_LEAN: f32 = 0.5;
pub const MAX_LEAN: f32 = 2.0;

fn default_lean() -> f32 {
    DEFAULT_LEAN
}

/// Which point on the modifier's bone the chain carries to the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IkEnd {
    /// The bone's own pivot. The chain is its parents, so a foot stays a foot.
    Joint,
    /// The far end of the bone, which bends with the chain. R6 limbs need this,
    /// since each is one bone.
    Tip,
}

impl IkEnd {
    pub const ALL: [IkEnd; 2] = [IkEnd::Joint, IkEnd::Tip];

    pub fn label(self) -> &'static str {
        match self {
            IkEnd::Joint => "Joint",
            IkEnd::Tip => "Tip",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ik {
    pub target: BoneId,
    pub pole: Option<BoneId>,
    /// How many bones bend.
    pub chain: usize,
    pub end: IkEnd,
    /// The bone takes the target's rotation, so a planted foot stays flat.
    pub match_rotation: bool,
    /// Turns the bend around the line from the chain's root to the target, in degrees.
    pub twist: f32,
    /// Which way the chain bends with no pole, in the space of its root's base.
    pub bend: Vec3,
    /// How hard a one-bone limb leans that way as its target comes closer.
    #[serde(default = "default_lean")]
    pub lean: f32,
}

impl Ik {
    /// One solid bone with its far end on the target, which is what `lean` acts on.
    pub fn is_one_bone(&self) -> bool {
        self.chain == 1 && self.end == IkEnd::Tip
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Unparent {
    pub position: bool,
    pub rotation: bool,
    /// A bone to follow in place of the real parent. `None` stays still in the world.
    pub anchor: Option<BoneId>,
    /// Where the bone is held, relative to the anchor.
    pub hold: Xf,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ModifierKind {
    Ik(Ik),
    Unparent(Unparent),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Modifier {
    pub bone: BoneId,
    pub enabled: bool,
    pub kind: ModifierKind,
}

impl Modifier {
    pub fn label(&self) -> &'static str {
        match self.kind {
            ModifierKind::Ik(_) => "Inverse Kinematics",
            ModifierKind::Unparent(_) => "Unparent",
        }
    }

    /// Short name for tight spots such as outliner rows.
    pub fn short_label(&self) -> &'static str {
        match self.kind {
            ModifierKind::Ik(_) => "IK",
            ModifierKind::Unparent(_) => "Unparent",
        }
    }

    /// The bone whose `value` says how strongly this acts.
    pub fn influence_bone(&self) -> BoneId {
        match self.kind {
            ModifierKind::Ik(ik) => ik.target,
            ModifierKind::Unparent(_) => self.bone,
        }
    }
}

fn influence(scene: &Scene, modifier: &Modifier) -> f32 {
    if modifier.enabled { scene.bones[modifier.influence_bone()].value.clamp(0.0, 1.0) } else { 0.0 }
}

/// Bones above `bone` that belong only to its limb, nearest first. Stops at a hub,
/// such as the torso, that other limbs hang from too.
fn limb_above(scene: &Scene, bone: BoneId) -> Vec<BoneId> {
    let mut limb = Vec::new();
    let mut at = scene.bones[bone].parent;
    while let Some(b) = at {
        if scene.bones[b].children.iter().filter(|c| !scene.bones[**c].removed).count() != 1 {
            break;
        }
        limb.push(b);
        at = scene.bones[b].parent;
    }
    limb
}

/// The bones that bend, from the root of the chain to its end.
pub fn chain_of(scene: &Scene, tip: BoneId, ik: &Ik) -> Vec<BoneId> {
    let mut at = match ik.end {
        IkEnd::Joint => scene.bones[tip].parent,
        IkEnd::Tip => Some(tip),
    };
    let mut chain = Vec::with_capacity(ik.chain);
    while let Some(b) = at {
        if chain.len() == ik.chain {
            break;
        }
        chain.push(b);
        at = scene.bones[b].parent;
    }
    chain.reverse();
    chain
}

/// The longest chain this bone can have.
pub fn longest_chain(scene: &Scene, tip: BoneId, end: IkEnd) -> usize {
    let mut count = usize::from(end == IkEnd::Tip);
    let mut at = scene.bones[tip].parent;
    while let Some(b) = at {
        count += 1;
        at = scene.bones[b].parent;
    }
    count.max(1)
}

/// Where the chain is trying to reach from, right now.
fn effector(scene: &Scene, tip: BoneId, end: IkEnd) -> Vec3 {
    match end {
        IkEnd::Joint => scene.bones[tip].head(),
        IkEnd::Tip => scene.bones[tip].tail(),
    }
}

/// For limbs built straight, legs bend forward and arms bend back. Which is which
/// comes from height, so it works on rigs with unfamiliar names. Characters face -Z,
/// as in Roblox, relative to the part the rig stands on.
fn default_bend(scene: &Scene, root: BoneId) -> Vec3 {
    let bone = &scene.bones[root];
    let rig = &scene.rigs[bone.rig];
    let standing_on = rig.parts.iter().find(|p| scene.parts[**p].drive == Drive::Root);
    let turn = standing_on.map_or(Quat::IDENTITY, |p| scene.parts[*p].world.rot);
    let middle = scene.bounds(Some(bone.rig)).map_or(bone.head().y, |(min, max)| (min.y + max.y) * 0.5);
    let forward = turn * Vec3::NEG_Z;
    let world = if bone.head().y < middle { forward } else { -forward };
    bone.base.rot.inverse() * world
}

/// The way a chain is already bent, by posing or by how the rig is built. R15 arms
/// come bent outward at the shoulder. Keeping that bend is what lets IK go on with
/// nothing moving. Tiny offsets don't count, since a leg built almost straight can
/// lean slightly the wrong way.
fn current_bend(scene: &Scene, tip: BoneId, ik: &Ik, chain: &[BoneId]) -> Option<Vec3> {
    let root = scene.bones[*chain.first()?].head();
    let middle = scene.bones[chain[chain.len() / 2]].head();
    let span = effector(scene, tip, ik.end) - root;
    let toward = span.try_normalize()?;
    let along = middle - root;
    let bend = along - toward * along.dot(toward);
    (chain.len() > 1 && bend.length() > span.length() * CLEAR_BEND).then(|| bend.normalize())
}

/// Adds IK to a bone with a target where its chain ends now, so nothing jumps.
/// Returns the new target.
pub fn add_ik(scene: &mut Scene, tip: BoneId) -> BoneId {
    let limb = limb_above(scene, tip);
    let (end, chain) = if limb.is_empty() { (IkEnd::Tip, 1) } else { (IkEnd::Joint, limb.len()) };
    let rest = Xf::new(scene.bones[tip].frame.rot, effector(scene, tip, end));
    let (name, rig) = (format!("{} IK", scene.bones[tip].name), scene.bones[tip].rig);
    let target = scene.add_control(name, BoneKind::IkTarget, rig, rest, 1.0);
    let mut ik = Ik {
        target,
        pole: None,
        chain,
        end,
        match_rotation: end == IkEnd::Joint,
        twist: 0.0,
        bend: Vec3::Z,
        lean: DEFAULT_LEAN,
    };
    let bones = chain_of(scene, tip, &ik);
    if let Some(root) = bones.first().copied() {
        ik.bend = match current_bend(scene, tip, &ik, &bones) {
            Some(world) => scene.bones[root].base.rot.inverse() * world,
            None => default_bend(scene, root),
        };
    }
    scene.modifiers.push(Modifier { bone: tip, enabled: true, kind: ModifierKind::Ik(ik) });
    scene.evaluate();
    target
}

/// Changes what a target or pole moves with, without moving it now.
pub fn set_follow(scene: &mut Scene, control: BoneId, follow: Option<BoneId>) {
    let held = scene.bones[control].base;
    let space = follow.map_or(Xf::IDENTITY, |b| scene.bones[b].frame);
    scene.bones[control].link = BoneLink::Free { rest: space.inverse() * held, follow };
    scene.evaluate();
}

/// Adds a pole out along the way the chain already bends, so nothing moves yet. It
/// follows the bone the chain hangs from, since a pole left behind when the body
/// turns would swing the knee around to chase it.
pub fn add_pole(scene: &mut Scene, index: usize) {
    let modifier = scene.modifiers[index];
    let ModifierKind::Ik(ik) = modifier.kind else {
        return;
    };
    let Some(root) = chain_of(scene, modifier.bone, &ik).first().copied() else {
        return;
    };
    let start = scene.bones[root].head();
    let goal = scene.bones[ik.target].head();
    let toward = (goal - start).try_normalize().unwrap_or(Vec3::NEG_Y);
    let bend = square_to(scene.bones[root].base.rot * ik.bend, toward);
    let reach = start.distance(goal).max(1.0);
    let at = start + toward * reach * 0.5 + bend * reach;
    let name = format!("{} Pole", scene.bones[modifier.bone].name);
    let rig = scene.bones[modifier.bone].rig;
    let pole = scene.add_control(name, BoneKind::IkPole, rig, Xf::new(Quat::IDENTITY, at), 1.0);
    if let ModifierKind::Ik(ik) = &mut scene.modifiers[index].kind {
        ik.pole = Some(pole);
    }
    scene.evaluate();
    let parent = scene.bones[root].parent;
    set_follow(scene, pole, parent);
}

/// Lets go of a bone where it stands, so turning this on doesn't move anything.
pub fn add_unparent(scene: &mut Scene, bone: BoneId) {
    let hold = scene.bones[bone].base;
    scene.bones[bone].value = 1.0;
    scene.modifiers.push(Modifier {
        bone,
        enabled: true,
        kind: ModifierKind::Unparent(Unparent { position: true, rotation: true, anchor: None, hold }),
    });
    scene.evaluate();
}

/// Takes a modifier off, along with the controls that belong to it.
pub fn remove(scene: &mut Scene, index: usize) -> Vec<BoneId> {
    let modifier = scene.modifiers.remove(index);
    let mut gone = Vec::new();
    if let ModifierKind::Ik(ik) = modifier.kind {
        gone.push(ik.target);
        gone.extend(ik.pole);
    }
    for bone in &gone {
        scene.bones[*bone].removed = true;
    }
    scene.evaluate();
    gone
}

pub fn remove_pole(scene: &mut Scene, index: usize) -> Option<BoneId> {
    let ModifierKind::Ik(ik) = &mut scene.modifiers[index].kind else {
        return None;
    };
    let pole = ik.pole.take()?;
    scene.bones[pole].removed = true;
    scene.evaluate();
    Some(pole)
}

/// Changes what an unparented bone follows, without moving it.
pub fn set_anchor(scene: &mut Scene, index: usize, anchor: Option<BoneId>) {
    let bone = scene.modifiers[index].bone;
    let held = scene.bones[bone].base;
    let space = anchor.map_or(Xf::IDENTITY, |a| scene.bones[a].frame);
    if let ModifierKind::Unparent(u) = &mut scene.modifiers[index].kind {
        u.anchor = anchor;
        u.hold = space.inverse() * held;
    }
    scene.evaluate();
}

/// Holds an unparented bone wherever it would be right now, following its parent.
pub fn hold_here(scene: &mut Scene, index: usize) {
    let bone = scene.modifiers[index].bone;
    let joint_base = scene.bones[bone].joint_base;
    if let ModifierKind::Unparent(u) = &mut scene.modifiers[index].kind {
        let space = u.anchor.map_or(Xf::IDENTITY, |a| scene.bones[a].frame);
        u.hold = space.inverse() * joint_base;
    }
    scene.evaluate();
}

/// Moves an IK target onto the end of its chain, for switching to IK without a jump.
pub fn snap_target(scene: &mut Scene, index: usize) {
    let modifier = scene.modifiers[index];
    let ModifierKind::Ik(ik) = modifier.kind else {
        return;
    };
    let frame = Xf::new(scene.bones[modifier.bone].frame.rot, effector(scene, modifier.bone, ik.end));
    scene.set_bone_frame(ik.target, frame);
}

/// The bone an edit of `bone` should really land on. Grabbing a foot under IK means
/// moving its target, since that's what the foot is following.
pub fn control_of(scene: &Scene, bone: BoneId) -> BoneId {
    scene
        .modifiers
        .iter()
        .find_map(|m| match m.kind {
            ModifierKind::Ik(ik) if m.bone == bone && influence(scene, m) > 0.0 => Some(ik.target),
            _ => None,
        })
        .unwrap_or(bone)
}

/// The bone whose modifiers apply to this selection. A target stands for its owner.
pub fn owner_of(scene: &Scene, bone: BoneId) -> BoneId {
    scene
        .modifiers
        .iter()
        .find_map(|m| match m.kind {
            ModifierKind::Ik(ik) if ik.target == bone || ik.pole == Some(bone) => Some(m.bone),
            _ => None,
        })
        .unwrap_or(bone)
}

/// True when IK fully decides this bone, so moving it by hand can't do anything. The
/// bone holding the modifier never counts, even when it bends with the chain as an
/// R6 limb does, because grabbing that one moves the target.
pub fn locked_by_ik(scene: &Scene, bone: BoneId) -> bool {
    scene.modifiers.iter().any(|m| match m.kind {
        ModifierKind::Ik(ik) => m.bone != bone && influence(scene, m) >= 0.999 && chain_of(scene, m.bone, &ik).contains(&bone),
        ModifierKind::Unparent(_) => false,
    })
}

/// True when a bone has fully let go of its parent.
pub fn is_unparented(scene: &Scene, bone: BoneId) -> bool {
    scene.modifiers.iter().any(|m| match m.kind {
        ModifierKind::Unparent(u) => m.bone == bone && u.position && u.rotation && influence(scene, m) >= 0.999,
        ModifierKind::Ik(_) => false,
    })
}

pub(crate) fn has_anchors(scene: &Scene) -> bool {
    scene.modifiers.iter().any(|m| matches!(m.kind, ModifierKind::Unparent(Unparent { anchor: Some(_), .. })))
}

/// Where a bone hangs from once Unparent has had its say.
pub(crate) fn held_base(scene: &Scene, bone: BoneId, joint_base: Xf) -> Xf {
    if !scene.bones[bone].unparented {
        return joint_base;
    }
    let mut base = joint_base;
    for modifier in &scene.modifiers {
        let ModifierKind::Unparent(u) = modifier.kind else {
            continue;
        };
        if modifier.bone != bone {
            continue;
        }
        let amount = influence(scene, modifier);
        let held = u.anchor.map_or(u.hold, |a| scene.bones[a].frame * u.hold);
        if u.position {
            base.pos = base.pos.lerp(held.pos, amount);
        }
        if u.rotation {
            base.rot = base.rot.slerp(held.rot, amount).normalize();
        }
    }
    base
}

/// Runs every IK modifier. Chains nearer the root go first, so a spine moves before
/// the arm hanging from it.
pub(crate) fn solve(scene: &mut Scene) {
    let depth = |scene: &Scene, mut bone: BoneId| {
        let mut count = 0;
        while let Some(parent) = scene.bones[bone].parent {
            bone = parent;
            count += 1;
        }
        count
    };
    let mut order: Vec<usize> = (0..scene.modifiers.len()).filter(|i| matches!(scene.modifiers[*i].kind, ModifierKind::Ik(_))).collect();
    order.sort_by_key(|i| depth(scene, scene.modifiers[*i].bone));
    if order.is_empty() {
        return;
    }
    // A chain only moves its own rig, so that's all that needs placing again.
    let placements = if crosses_rigs(scene) { Vec::new() } else { scene.rig_placements() };
    for index in order {
        let modifier = scene.modifiers[index];
        if let ModifierKind::Ik(ik) = modifier.kind {
            let amount = influence(scene, &modifier);
            if amount > 0.0 {
                let placement = placements.get(scene.bones[modifier.bone].rig);
                solve_ik(scene, modifier.bone, &ik, amount, placement);
            }
        }
    }
}

/// True when something hangs from a bone in another rig, which means moving one rig
/// can move another.
fn crosses_rigs(scene: &Scene) -> bool {
    let rig = |bone: BoneId| scene.bones[bone].rig;
    let follows = scene.bones.iter().any(|b| matches!(b.link, BoneLink::Free { follow: Some(f), .. } if rig(f) != b.rig));
    follows
        || scene.modifiers.iter().any(|m| match m.kind {
            ModifierKind::Ik(ik) => rig(ik.target) != rig(m.bone) || ik.pole.is_some_and(|p| rig(p) != rig(m.bone)),
            ModifierKind::Unparent(u) => u.anchor.is_some_and(|a| rig(a) != rig(m.bone)),
        })
}

/// Places the rig being solved, or everything when there's no plan for just that rig.
fn place_again(scene: &mut Scene, placement: Option<&Placement>) {
    match placement {
        Some(order) => scene.place(order),
        None => scene.place_all(),
    }
}

fn solve_ik(scene: &mut Scene, tip: BoneId, ik: &Ik, amount: f32, placement: Option<&Placement>) {
    let chain = chain_of(scene, tip, ik);
    let Some(root) = chain.first().copied() else {
        return;
    };
    let target = scene.bones[ik.target].frame;
    let start: Vec<(BoneId, Xf)> = chain.iter().chain([&tip]).map(|b| (*b, scene.bones[*b].solved)).collect();
    if ik.is_one_bone() {
        plant_one_bone(scene, tip, ik, target.pos);
        place_again(scene, placement);
        if amount < 1.0 {
            let (bone, fk) = start[0];
            let solved = &mut scene.bones[bone].solved;
            *solved = Xf::new(fk.rot.slerp(solved.rot, amount).normalize(), fk.pos.lerp(solved.pos, amount));
            place_again(scene, placement);
        }
        return;
    }

    let mut joints: Vec<Vec3> = chain.iter().map(|b| scene.bones[*b].head()).collect();
    joints.push(effector(scene, tip, ik.end));
    let toward = (target.pos - joints[0]).try_normalize().unwrap_or(Vec3::NEG_Y);
    let aim = match ik.pole {
        Some(pole) => scene.bones[pole].head() - joints[0],
        None => scene.bones[root].base.rot * ik.bend,
    };
    let bend = Quat::from_axis_angle(toward, ik.twist.to_radians()) * square_to(aim, toward);
    let goal = reach(&joints, target.pos, toward, bend);

    // Turn each bone in order, from the root out, reading where the next point has
    // actually landed. Bones often sit off each other's axes, so it has to be read.
    for (i, bone) in chain.iter().enumerate() {
        let head = scene.bones[*bone].head();
        let end = chain.get(i + 1).map_or_else(|| effector(scene, tip, ik.end), |next| scene.bones[*next].head());
        if let (Some(from), Some(to)) = ((end - head).try_normalize(), (goal[i + 1] - head).try_normalize()) {
            let mut turn = Quat::from_rotation_arc(from, to);
            if chain.len() == 1 {
                turn = Quat::from_axis_angle(to, ik.twist.to_radians()) * turn;
            }
            let frame = Xf::new((turn * scene.bones[*bone].frame.rot).normalize(), head);
            scene.bones[*bone].solved = scene.bones[*bone].base.inverse() * frame;
            place_again(scene, placement);
        }
    }
    if ik.end == IkEnd::Joint && ik.match_rotation {
        let bone = &mut scene.bones[tip];
        let frame = Xf::new(target.rot, bone.head());
        bone.solved = bone.base.inverse() * frame;
        place_again(scene, placement);
    }
    if amount < 1.0 {
        for (bone, fk) in start {
            let solved = &mut scene.bones[bone].solved;
            *solved = Xf::new(fk.rot.slerp(solved.rot, amount).normalize(), fk.pos.lerp(solved.pos, amount));
        }
        place_again(scene, placement);
    }
}

/// An R6 limb is one solid bone with nothing to bend. Out of reach, it points at the
/// target. Closer than its length, it slides into the body so its end stays on the
/// target, which `Transform` allows, and leans the way a knee or elbow would bend.
/// Only moves across the limb swing it. A joint usually sits off to one side of the
/// limb, so aiming from the joint would tip the limb sideways as its end came up.
fn plant_one_bone(scene: &mut Scene, tip: BoneId, ik: &Ik, target: Vec3) {
    let aim_at = ik.pole.map(|pole| scene.bones[pole].head());
    let bone = &scene.bones[tip];
    let (head, end) = (bone.head(), bone.tail());
    let Some(keyed) = (end - head).try_normalize() else {
        return;
    };
    let (length, offset) = (head.distance(end), target - head);
    let spare = length - offset.length();
    let along = bone.frame.rot * long_axis(bone.tail_local);
    let wanted = aim_at.map_or(bone.base.rot * ik.bend, |pole| pole - head);
    let bend = Quat::from_axis_angle(along, ik.twist.to_radians()) * square_to(wanted, along);
    let across = (target - end) - along * (target - end).dot(along);
    let lean = (length * length - offset.length_squared()).max(0.0).sqrt() * ik.lean;
    let close = (end - head) + across + bend * lean;
    // Blended in over the last stretch of reach, so nothing jumps at full stretch.
    let aim = offset.lerp(close, (spare / length * 4.0).clamp(0.0, 1.0)).try_normalize().unwrap_or(keyed);
    let turn = Quat::from_axis_angle(aim, ik.twist.to_radians()) * Quat::from_rotation_arc(keyed, aim);
    let rot = (turn * bone.frame.rot).normalize();
    let pos = if spare > 0.0 { target - rot * bone.tail_local } else { head };
    let solved = bone.base.inverse() * Xf::new(rot, pos);
    scene.bones[tip].solved = solved;
}

/// The bone's own axis nearest the way it points.
fn long_axis(tail_local: Vec3) -> Vec3 {
    let size = tail_local.abs();
    if size.y >= size.x && size.y >= size.z {
        Vec3::Y * tail_local.y.signum()
    } else if size.x >= size.z {
        Vec3::X * tail_local.x.signum()
    } else {
        Vec3::Z * tail_local.z.signum()
    }
}

/// The part of `aim` at a right angle to `axis`, as a unit direction.
fn square_to(aim: Vec3, axis: Vec3) -> Vec3 {
    (aim - axis * aim.dot(axis)).try_normalize().unwrap_or_else(|| axis.any_orthonormal_vector())
}

/// Where each point of a chain should end up, for chains of any length.
fn reach(joints: &[Vec3], target: Vec3, toward: Vec3, bend: Vec3) -> Vec<Vec3> {
    match joints {
        [root, end] => vec![*root, *root + toward * root.distance(*end)],
        [root, middle, end] => two_bones(*root, root.distance(*middle), middle.distance(*end), target, toward, bend),
        _ => fabrik(joints, target, toward, bend),
    }
}

/// The exact answer for two bones, from the law of cosines. Limbs are nearly always
/// this, and unlike the general solver it can't drift between frames.
fn two_bones(root: Vec3, upper: f32, lower: f32, target: Vec3, toward: Vec3, bend: Vec3) -> Vec<Vec3> {
    let distance = root.distance(target).clamp((upper - lower).abs().max(1e-4), (upper + lower).max(1e-4));
    let cos = ((upper * upper + distance * distance - lower * lower) / (2.0 * upper * distance).max(1e-6)).clamp(-1.0, 1.0);
    let sin = (1.0 - cos * cos).sqrt();
    vec![root, root + toward * (upper * cos) + bend * (upper * sin), root + toward * distance]
}

fn fabrik(joints: &[Vec3], target: Vec3, toward: Vec3, bend: Vec3) -> Vec<Vec3> {
    let lengths: Vec<f32> = joints.windows(2).map(|w| w[0].distance(w[1])).collect();
    let total: f32 = lengths.iter().sum();
    let last = joints.len() - 1;
    let root = joints[0];
    // At full stretch there's only one answer, and solving for it would bend the
    // chain a little. Going straight there means a straight chain stays straight.
    if root.distance(target) >= total - FABRIK_TOLERANCE {
        let mut along = 0.0;
        let mut points = vec![root];
        for length in lengths {
            along += length;
            points.push(root + toward * along);
        }
        return points;
    }
    let mut points = joints.to_vec();
    for (i, point) in points.iter_mut().enumerate() {
        *point += bend * total * CHAIN_LEAN * (PI * i as f32 / last as f32).sin();
    }
    for _ in 0..FABRIK_PASSES {
        points[last] = target;
        for i in (0..last).rev() {
            let dir = (points[i] - points[i + 1]).try_normalize().unwrap_or(bend);
            points[i] = points[i + 1] + dir * lengths[i];
        }
        points[0] = root;
        for i in 0..last {
            let dir = (points[i + 1] - points[i]).try_normalize().unwrap_or(bend);
            points[i + 1] = points[i] + dir * lengths[i];
        }
        if points[last].distance(target) < FABRIK_TOLERANCE {
            break;
        }
    }
    points
}
