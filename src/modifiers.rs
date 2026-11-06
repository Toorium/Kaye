//! Modifiers change where a bone ends up without touching its keys, so turning one
//! off always gets the original animation back. Like the rig logic, this has no
//! interface code.
//!
//! Roblox only understands one `Transform` per joint, so everything here has to come
//! down to that. `Bone::pose` stays what was keyed, and `Bone::output` is what an
//! export needs to write to reproduce the result.
//!
//! - Unparent and Inherit Rotation run while the rig is placed, by swapping the base a
//!   bone hangs from.
//! - IK, Look At and Copy Bone run afterward and write `Bone::solved`, then the rig is
//!   placed again. Ragdoll goes next, from what `dynamics.rs` worked out, and Limit
//!   Rotation goes last, since it has the final word on a bone.
//!
//! How strongly most of these act is `Bone::value`, on the target or on the bone itself.
//! That makes it animated like anything else, which is how IK and FK get blended over
//! time. Unparent, Inherit Rotation and Ragdoll on one bone share that one number.
//!
//! The switches are the one exception to leaving keys alone. `switch_ik` and
//! `switch_unparent` hand a limb from one way of being driven to the other with nothing
//! moving, and that means rewriting its pose.

use std::f32::consts::PI;

use glam::{EulerRot, Mat3, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::animation::angles_near;
use crate::dynamics;
use crate::math::Xf;
use crate::scene::{BoneId, BoneKind, BoneLink, Drive, FolderId, PartId, Placement, RigId, Scene};

const FABRIK_PASSES: usize = 24;
const FABRIK_TOLERANCE: f32 = 1e-3;
/// How far a straight chain leans toward its bend, as a share of its length. FABRIK
/// can't bend a perfectly straight chain, because every step stays on the line.
const CHAIN_LEAN: f32 = 0.12;
/// How far off the straight line a chain's middle has to sit, as a share of the
/// chain's reach, to count as bent.
const CLEAR_BEND: f32 = 0.08;
/// How far a one-bone limb leans to start with, as a share of how far a real knee
/// would stick out. Much more and the top of the limb comes out of the side of the
/// body it leans toward.
const DEFAULT_LEAN: f32 = 0.5;
pub const MAX_LEAN: f32 = 2.0;
/// How far a new chain's knees and elbows can fold, in degrees.
const DEFAULT_FOLD: f32 = 150.0;
/// How far ahead a new Look At target starts, in studs.
const LOOK_DISTANCE: f32 = 4.0;
/// How many objects one Ragdoll can land on. A fixed number keeps a modifier plain data.
pub const MAX_OBJECTS: usize = 8;
/// Roblox's default, in studs per second squared.
pub const DEFAULT_GRAVITY: f32 = 196.2;
pub const RAGDOLL_KIND: u8 = 6;

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
    /// The most a joint in the chain can fold, in degrees. Having a limit also turns
    /// the bend along with the limb, so a knee can never swing around to the back.
    #[serde(default)]
    pub limit: Option<f32>,
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

/// Keeps a bone pointed at a target.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LookAt {
    pub target: BoneId,
    /// The side of the bone that does the looking, in the bone's own space.
    pub axis: Vec3,
}

/// Makes a bone copy where another bone is.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CopyBone {
    pub source: Option<BoneId>,
    pub position: bool,
    pub rotation: bool,
    /// How strongly it copies.
    pub amount: f32,
    /// Where this bone sits relative to the source, so picking one moves nothing.
    pub offset: Xf,
}

/// Stops a bone turning past a range, in degrees around its own axes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Limit {
    pub axes: [bool; 3],
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// Keeps a bone looking the same way however its parents turn, while it still moves
/// with them. Blender's Inherit Rotation, turned off. A head is what this is for.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeldRotation {
    /// The way the bone hangs, relative to where the rig has been placed.
    pub held: Quat,
}

/// Hands a limb, or the whole body, over to gravity.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ragdoll {
    /// In studs per second squared.
    pub gravity: f32,
    /// The whole rig falls, and not only this limb.
    pub affects_body: bool,
    /// The rigs and shapes it lands on, besides the floor.
    #[serde(default)]
    pub objects: [Option<RigId>; MAX_OBJECTS],
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ModifierKind {
    Ik(Ik),
    Unparent(Unparent),
    LookAt(LookAt),
    Copy(CopyBone),
    Limit(Limit),
    Inherit(HeldRotation),
    Ragdoll(Ragdoll),
}

impl ModifierKind {
    /// Which kind this is, for telling apart the one of each a bone can have.
    pub fn index(&self) -> u8 {
        match self {
            ModifierKind::Ik(_) => 0,
            ModifierKind::Unparent(_) => 1,
            ModifierKind::LookAt(_) => 2,
            ModifierKind::Copy(_) => 3,
            ModifierKind::Limit(_) => 4,
            ModifierKind::Inherit(_) => 5,
            ModifierKind::Ragdoll(_) => RAGDOLL_KIND,
        }
    }
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
            ModifierKind::LookAt(_) => "Look At",
            ModifierKind::Copy(_) => "Copy Bone",
            ModifierKind::Limit(_) => "Limit Rotation",
            ModifierKind::Inherit(_) => "Inherit Rotation",
            ModifierKind::Ragdoll(_) => "Ragdoll",
        }
    }

    /// Short name for tight spots such as outliner rows.
    pub fn short_label(&self) -> &'static str {
        match self.kind {
            ModifierKind::Ik(_) => "IK",
            ModifierKind::Unparent(_) => "Unparent",
            ModifierKind::LookAt(_) => "Look At",
            ModifierKind::Copy(_) => "Copy",
            ModifierKind::Limit(_) => "Limit",
            ModifierKind::Inherit(_) => "Inherit",
            ModifierKind::Ragdoll(_) => "Ragdoll",
        }
    }

    /// The bone whose `value` says how strongly this acts, for the kinds animated that way.
    pub fn influence_bone(&self) -> Option<BoneId> {
        match self.kind {
            ModifierKind::Ik(ik) => Some(ik.target),
            ModifierKind::LookAt(look) => Some(look.target),
            ModifierKind::Unparent(_) | ModifierKind::Inherit(_) | ModifierKind::Ragdoll(_) => Some(self.bone),
            ModifierKind::Copy(_) | ModifierKind::Limit(_) => None,
        }
    }
}

pub(crate) fn influence(scene: &Scene, modifier: &Modifier) -> f32 {
    if !modifier.enabled {
        return 0.0;
    }
    match (modifier.influence_bone(), modifier.kind) {
        (Some(bone), _) => scene.bones[bone].value.clamp(0.0, 1.0),
        (None, ModifierKind::Copy(copy)) => copy.amount.clamp(0.0, 1.0),
        (None, _) => 1.0,
    }
}

/// True when this modifier is the one in charge, for showing which way a switch is set.
pub fn is_on(scene: &Scene, index: usize) -> bool {
    influence(scene, &scene.modifiers[index]) >= 0.5
}

/// True for a target or pole whose IK or Look At isn't doing anything right now.
pub fn control_is_idle(scene: &Scene, bone: BoneId) -> bool {
    let owner = scene.modifiers.iter().find(|m| match m.kind {
        ModifierKind::Ik(ik) => ik.target == bone || ik.pole == Some(bone),
        ModifierKind::LookAt(look) => look.target == bone,
        _ => false,
    });
    owner.is_some_and(|m| influence(scene, m) <= 0.0)
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
        // Knees and elbows. A tail or a rope needs to curl any way it likes.
        limit: (chain == 2).then_some(DEFAULT_FOLD),
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
    let space = follow.map_or(placement_of(scene, control), |b| scene.bones[b].frame);
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
    let bone = &scene.bones[root];
    let goal = scene.bones[ik.target].head();
    // A one-bone limb turns on its own length, as it lies before IK moves it, so its
    // pole has to sit square to that for nothing to move.
    let (start, toward) = if ik.is_one_bone() {
        let keyed = bone.base * bone.pose;
        (keyed.pos, keyed.rot * long_axis(bone.tail_local))
    } else {
        (bone.head(), (goal - bone.head()).try_normalize().unwrap_or(Vec3::NEG_Y))
    };
    let chain = chain_of(scene, modifier.bone, &ik);
    let along = scene.bones[chain[chain.len() / 2]].head() - start;
    let off_line = along - toward * along.dot(toward);
    // The way it's bending now, which a limit may have carried away from where it began.
    let wanted = if !ik.is_one_bone() && chain.len() > 1 && off_line.length() > 1e-3 { off_line } else { bone.base.rot * ik.bend };
    let bend = square_to(wanted, toward);
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

/// Where a bone's rig has been moved to. What's held still in the world is held relative
/// to this, so moving the rig brings it along.
fn placement_of(scene: &Scene, bone: BoneId) -> Xf {
    scene.rigs[scene.bones[bone].rig].placement
}

/// A new modifier starts at full influence, unless the bone's number already means
/// something to another modifier on it.
fn start_influence(scene: &mut Scene, bone: BoneId) {
    if !scene.modifiers.iter().any(|m| m.bone == bone && m.influence_bone() == Some(bone)) {
        scene.bones[bone].value = 1.0;
    }
}

/// Stops a bone taking on its parents' turns, from the way it hangs now, so adding this
/// doesn't move anything.
pub fn add_inherit(scene: &mut Scene, bone: BoneId) {
    start_influence(scene, bone);
    let held = placement_of(scene, bone).rot.inverse() * scene.bones[bone].base.rot;
    scene.modifiers.push(Modifier { bone, enabled: true, kind: ModifierKind::Inherit(HeldRotation { held }) });
    scene.evaluate();
}

/// Lets go of a bone where it stands, so turning this on doesn't move anything.
pub fn add_unparent(scene: &mut Scene, bone: BoneId) {
    let hold = placement_of(scene, bone).inverse() * scene.bones[bone].base;
    scene.bones[bone].value = 1.0;
    scene.modifiers.push(Modifier {
        bone,
        enabled: true,
        kind: ModifierKind::Unparent(Unparent { position: true, rotation: true, anchor: None, hold }),
    });
    scene.evaluate();
}

/// The way the character on this bone's rig faces, as Roblox has it.
fn rig_forward(scene: &Scene, bone: BoneId) -> Vec3 {
    let rig = &scene.rigs[scene.bones[bone].rig];
    let standing_on = rig.parts.iter().find(|p| scene.parts[**p].drive == Drive::Root);
    standing_on.map_or(Quat::IDENTITY, |p| scene.parts[*p].world.rot) * Vec3::NEG_Z
}

/// Adds Look At with a target straight ahead of the bone, so nothing moves yet.
/// Returns the new target.
pub fn add_look_at(scene: &mut Scene, bone: BoneId) -> BoneId {
    let forward = rig_forward(scene, bone);
    let at = scene.bones[bone].head() + forward * LOOK_DISTANCE;
    let axis = scene.bones[bone].frame.rot.inverse() * forward;
    let (name, rig) = (format!("{} Look At", scene.bones[bone].name), scene.bones[bone].rig);
    let target = scene.add_control(name, BoneKind::LookTarget, rig, Xf::new(Quat::IDENTITY, at), 1.0);
    scene.modifiers.push(Modifier { bone, enabled: true, kind: ModifierKind::LookAt(LookAt { target, axis }) });
    scene.evaluate();
    target
}

/// Adds Copy Bone, with nothing picked to copy yet.
pub fn add_copy(scene: &mut Scene, bone: BoneId) {
    let copy = CopyBone { source: None, position: false, rotation: true, amount: 1.0, offset: Xf::IDENTITY };
    scene.modifiers.push(Modifier { bone, enabled: true, kind: ModifierKind::Copy(copy) });
    scene.evaluate();
}

/// Picks what Copy Bone copies, keeping the distance between the two bones as it is.
pub fn set_copy_source(scene: &mut Scene, index: usize, source: Option<BoneId>) {
    let bone = scene.bones[scene.modifiers[index].bone].frame;
    let offset = source.map_or(Xf::IDENTITY, |s| scene.bones[s].frame.inverse() * bone);
    if let ModifierKind::Copy(copy) = &mut scene.modifiers[index].kind {
        copy.source = source;
        copy.offset = offset;
    }
    scene.evaluate();
}

/// Adds Limit Rotation with the bone's current angles as the middle of each range.
pub fn add_limit(scene: &mut Scene, bone: BoneId) {
    let angles = angles_near(scene.bones[bone].solved.rot, [0.0; 3]);
    let limit = Limit { axes: [false; 3], min: angles.map(|a| a - 45.0), max: angles.map(|a| a + 45.0) };
    scene.modifiers.push(Modifier { bone, enabled: true, kind: ModifierKind::Limit(limit) });
    scene.evaluate();
}

/// Adds Ragdoll for just this bone and what hangs from it.
pub fn add_ragdoll(scene: &mut Scene, bone: BoneId) {
    start_influence(scene, bone);
    let ragdoll = Ragdoll { gravity: DEFAULT_GRAVITY, affects_body: false, objects: [None; MAX_OBJECTS] };
    scene.modifiers.push(Modifier { bone, enabled: true, kind: ModifierKind::Ragdoll(ragdoll) });
    scene.evaluate();
}

/// What a Ragdoll lands on.
fn objects_of(modifier: &mut Modifier) -> Option<&mut [Option<RigId>; MAX_OBJECTS]> {
    match &mut modifier.kind {
        ModifierKind::Ragdoll(ragdoll) => Some(&mut ragdoll.objects),
        _ => None,
    }
}

/// Gives a Ragdoll another object to land on. Does nothing if it has it or has no room.
pub fn add_object(scene: &mut Scene, index: usize, object: RigId) {
    if let Some(objects) = objects_of(&mut scene.modifiers[index]) {
        if !objects.contains(&Some(object)) {
            if let Some(free) = objects.iter_mut().find(|o| o.is_none()) {
                *free = Some(object);
            }
        }
    }
}

pub fn remove_object(scene: &mut Scene, index: usize, object: RigId) {
    if let Some(objects) = objects_of(&mut scene.modifiers[index]) {
        let kept: Vec<RigId> = objects.iter().flatten().copied().filter(|o| *o != object).collect();
        *objects = std::array::from_fn(|i| kept.get(i).copied());
    }
}

/// Takes a modifier off, along with the controls that belong to it.
pub fn remove(scene: &mut Scene, index: usize) -> Vec<BoneId> {
    let modifier = scene.modifiers.remove(index);
    let mut gone = Vec::new();
    if let ModifierKind::Ik(ik) = modifier.kind {
        gone.push(ik.target);
        gone.extend(ik.pole);
    }
    if let ModifierKind::LookAt(look) = modifier.kind {
        gone.push(look.target);
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

/// One animation's own modifiers, kept while another of its rig's animations plays. Only
/// used with Individual Animation Properties on, see `Animation::separate`. The targets and
/// poles stay in the scene the whole time, hidden while an animation without them plays,
/// so their ids and keys still match.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Setup {
    pub modifiers: Vec<Modifier>,
    /// The targets and poles these use, with what each one follows.
    pub controls: Vec<(BoneId, BoneLink)>,
}

impl Setup {
    /// Lets go of a deleted bone. Modifiers that can't work without it go, and so do
    /// their targets, which are returned so their keys can go too.
    pub fn forget(&mut self, bone: BoneId) -> Vec<BoneId> {
        self.modifiers.retain(|m| m.bone != bone && !needed_controls(m).contains(&bone));
        for m in &mut self.modifiers {
            let_go_of(m, |b| b != bone);
        }
        let used: Vec<BoneId> = self.modifiers.iter().flat_map(controls_of).collect();
        let gone = self.controls.iter().map(|(b, _)| *b).filter(|b| !used.contains(b)).collect();
        self.controls.retain(|(b, _)| used.contains(b));
        gone
    }
}

/// The controls a modifier can't work without.
fn needed_controls(modifier: &Modifier) -> Vec<BoneId> {
    match modifier.kind {
        ModifierKind::Ik(ik) => vec![ik.target],
        ModifierKind::LookAt(look) => vec![look.target],
        _ => Vec::new(),
    }
}

/// Every control a modifier uses, its pole included.
fn controls_of(modifier: &Modifier) -> Vec<BoneId> {
    let mut controls = needed_controls(modifier);
    if let ModifierKind::Ik(Ik { pole: Some(pole), .. }) = modifier.kind {
        controls.push(pole);
    }
    controls
}

/// Drops whatever a modifier only points at, such as a pole or a bone to copy, when
/// `kept` says it's gone.
fn let_go_of(modifier: &mut Modifier, kept: impl Fn(BoneId) -> bool) {
    match &mut modifier.kind {
        ModifierKind::Ik(ik) => ik.pole = ik.pole.filter(|p| kept(*p)),
        ModifierKind::Copy(copy) => copy.source = copy.source.filter(|s| kept(*s)),
        ModifierKind::Unparent(unparent) => unparent.anchor = unparent.anchor.filter(|a| kept(*a)),
        _ => {}
    }
}

/// The modifiers on this rig's bones right now, and the targets and poles they use.
pub fn setup_of(scene: &Scene, rig: RigId) -> Setup {
    let modifiers = scene.modifiers.iter().filter(|m| scene.bones[m.bone].rig == rig).copied().collect();
    let controls = scene.rigs[rig]
        .bones
        .iter()
        .map(|b| (*b, &scene.bones[*b]))
        .filter(|(_, b)| b.kind.is_control() && !b.removed)
        .map(|(id, b)| (id, b.link))
        .collect();
    Setup { modifiers, controls }
}

/// Puts an animation's own modifiers on its rig in place of the ones there now. Anything
/// they point at that's been deleted since is let go, so an old setup can't bring back a
/// bone that's gone.
pub fn use_setup(scene: &mut Scene, rig: RigId, setup: &Setup) {
    for id in scene.rigs[rig].bones.clone() {
        if scene.bones[id].kind.is_control() {
            scene.bones[id].removed = true;
        }
    }
    for (id, link) in &setup.controls {
        if let Some(bone) = scene.bones.get_mut(*id).filter(|b| b.rig == rig && b.kind.is_control()) {
            bone.removed = false;
            bone.link = *link;
        }
    }
    let bones = &scene.bones;
    scene.modifiers.retain(|m| bones[m.bone].rig != rig);
    let live = |b: BoneId| scene.bones.get(b).is_some_and(|b| !b.removed);
    let mut kept = Vec::with_capacity(setup.modifiers.len());
    for modifier in &setup.modifiers {
        if !live(modifier.bone) || scene.bones[modifier.bone].rig != rig || !needed_controls(modifier).into_iter().all(live) {
            continue;
        }
        let mut modifier = *modifier;
        let_go_of(&mut modifier, live);
        if let ModifierKind::Ragdoll(ragdoll) = &mut modifier.kind {
            let landing: Vec<RigId> = ragdoll.objects.iter().flatten().copied().filter(|r| scene.rigs.get(*r).is_some_and(|r| !r.removed)).collect();
            ragdoll.objects = std::array::from_fn(|i| landing.get(i).copied());
        }
        kept.push(modifier);
    }
    let used: Vec<BoneId> = kept.iter().flat_map(controls_of).collect();
    for (id, _) in &setup.controls {
        if !used.contains(id) {
            if let Some(bone) = scene.bones.get_mut(*id).filter(|b| b.rig == rig) {
                bone.removed = true;
            }
        }
    }
    scene.modifiers.extend(kept);
}

fn bones_below(scene: &Scene, bone: BoneId, into: &mut Vec<BoneId>) {
    into.push(bone);
    for child in &scene.bones[bone].children {
        bones_below(scene, *child, into);
    }
}

/// Deletes bones from the scene, and returns everything that went, so its keys can go too.
/// A joint takes what hangs from it, since nothing would hold that up. Deleting a target
/// takes its modifier, which can't work without it, and anything left that followed one
/// of these stays where it is.
pub fn delete_bones(scene: &mut Scene, bones: &[BoneId]) -> Vec<BoneId> {
    let mut gone = Vec::new();
    for &bone in bones {
        if scene.bones[bone].removed {
            continue;
        }
        let owner = scene.modifiers.iter().position(|m| match m.kind {
            ModifierKind::Ik(ik) => ik.target == bone,
            ModifierKind::LookAt(look) => look.target == bone,
            _ => false,
        });
        let pole = scene.modifiers.iter().position(|m| matches!(m.kind, ModifierKind::Ik(ik) if ik.pole == Some(bone)));
        if let Some(index) = owner {
            gone.extend(remove(scene, index));
        } else if let Some(index) = pole {
            gone.extend(remove_pole(scene, index));
        } else if scene.bones[bone].kind.is_joint() {
            let mut below = Vec::new();
            bones_below(scene, bone, &mut below);
            for b in below {
                scene.bones[b].removed = true;
                gone.push(b);
            }
        } else {
            scene.bones[bone].removed = true;
            gone.push(bone);
        }
    }
    while let Some(index) = scene.modifiers.iter().position(|m| gone.contains(&m.bone)) {
        gone.extend(remove(scene, index));
    }
    for index in 0..scene.modifiers.len() {
        match scene.modifiers[index].kind {
            ModifierKind::Unparent(Unparent { anchor: Some(anchor), .. }) if gone.contains(&anchor) => set_anchor(scene, index, None),
            ModifierKind::Copy(CopyBone { source: Some(source), .. }) if gone.contains(&source) => set_copy_source(scene, index, None),
            _ => {}
        }
    }
    // A camera or shape is its one bone, so with that gone, so is its rig.
    let rigs: Vec<RigId> = gone.iter().map(|b| scene.bones[*b].rig).collect();
    for rig in rigs {
        if scene.rigs[rig].kind.is_single() && scene.live_bones(rig).is_empty() {
            scene.rigs[rig].removed = true;
            for index in 0..scene.modifiers.len() {
                remove_object(scene, index, rig);
            }
        }
    }
    for control in 0..scene.bones.len() {
        if matches!(scene.bones[control].link, BoneLink::Free { follow: Some(f), .. } if gone.contains(&f)) && !scene.bones[control].removed {
            set_follow(scene, control, None);
        }
    }
    scene.evaluate();
    gone
}

/// Deletes a whole rig, or whatever else was spawned as one.
pub fn delete_rig(scene: &mut Scene, rig: RigId) -> Vec<BoneId> {
    let bones = scene.live_bones(rig);
    let gone = delete_bones(scene, &bones);
    for index in 0..scene.modifiers.len() {
        remove_object(scene, index, rig);
    }
    scene.rigs[rig].removed = true;
    scene.evaluate();
    gone
}

/// Deletes a folder with everything in it, as Roblox Studio does. The folders inside go
/// with it, since a folder only counts as there while the ones holding it are.
pub fn delete_folder(scene: &mut Scene, folder: FolderId) -> Vec<BoneId> {
    let mut gone = Vec::new();
    for rig in scene.rigs_in_folder(folder) {
        gone.extend(delete_rig(scene, rig));
    }
    scene.folders[folder].removed = true;
    gone
}

/// Deletes a part. One a joint moves goes with its joint, one welded on goes by itself,
/// and with the part a rig stands on gone, there's no rig left.
pub fn delete_part(scene: &mut Scene, part: PartId) -> Vec<BoneId> {
    match scene.parts[part].drive {
        Drive::Bone(bone) => delete_bones(scene, &[bone]),
        Drive::Rigid { .. } => {
            scene.parts[part].removed = true;
            Vec::new()
        }
        Drive::Root => delete_rig(scene, scene.parts[part].rig),
    }
}

/// Changes what an unparented bone follows, without moving it.
pub fn set_anchor(scene: &mut Scene, index: usize, anchor: Option<BoneId>) {
    let bone = scene.modifiers[index].bone;
    let held = scene.bones[bone].base;
    let space = anchor.map_or(placement_of(scene, bone), |a| scene.bones[a].frame);
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
    let placement = placement_of(scene, bone);
    if let ModifierKind::Unparent(u) = &mut scene.modifiers[index].kind {
        let space = u.anchor.map_or(placement, |a| scene.bones[a].frame);
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

/// Every bone a modifier's result lands on, chain and all.
pub fn driven_bones(scene: &Scene, index: usize) -> Vec<BoneId> {
    let modifier = scene.modifiers[index];
    if let ModifierKind::Ragdoll(ragdoll) = modifier.kind {
        return dynamics::ragdoll_bones(scene, modifier.bone, ragdoll.affects_body);
    }
    let ModifierKind::Ik(ik) = modifier.kind else {
        return vec![modifier.bone];
    };
    let mut bones = chain_of(scene, modifier.bone, &ik);
    if !bones.contains(&modifier.bone) {
        bones.push(modifier.bone);
    }
    bones
}

/// What's showing becomes the pose, so nothing is lost when the solver stops.
fn freeze(scene: &mut Scene, bones: &[BoneId]) {
    for bone in bones {
        let bone = &mut scene.bones[*bone];
        bone.pose = bone.solved;
    }
}

/// How far each joint in a chain is folded right now, in degrees, at most.
fn deepest_fold(joints: &[Vec3]) -> f32 {
    let fold = |w: &[Vec3]| (w[1] - w[0]).angle_between(w[2] - w[1]).to_degrees();
    joints.windows(3).map(fold).filter(|a| a.is_finite()).fold(0.0, f32::max)
}

/// Switches a limb between IK and FK with nothing moving. Going to FK, what the solver
/// worked out becomes the pose. Going to IK, the target and pole come to the limb, and
/// the way it's bent right now is what IK carries on with.
pub fn switch_ik(scene: &mut Scene, index: usize, on: bool) {
    let modifier = scene.modifiers[index];
    let ModifierKind::Ik(ik) = modifier.kind else {
        return;
    };
    let bones = driven_bones(scene, index);
    freeze(scene, &bones);
    scene.modifiers[index].enabled = true;
    scene.bones[ik.target].value = 0.0;
    scene.evaluate();
    if on {
        let tip = &scene.bones[modifier.bone];
        let target = Xf::new(tip.frame.rot, effector(scene, modifier.bone, ik.end));
        scene.set_bone_frame(ik.target, target);
        carry_on_from_here(scene, index);
        scene.bones[ik.target].value = 1.0;
    }
    scene.evaluate();
}

/// Sets up a chain's bend and pole to agree with how it lies now.
fn carry_on_from_here(scene: &mut Scene, index: usize) {
    let modifier = scene.modifiers[index];
    let ModifierKind::Ik(ik) = modifier.kind else {
        return;
    };
    let chain = chain_of(scene, modifier.bone, &ik);
    let Some(root) = chain.first().copied() else {
        return;
    };
    let mut joints: Vec<Vec3> = chain.iter().map(|b| scene.bones[*b].head()).collect();
    joints.push(effector(scene, modifier.bone, ik.end));
    let frame = scene.bones[root].frame;
    let line = (joints[chain.len()] - joints[0]).try_normalize().unwrap_or(Vec3::NEG_Y);
    let mut plane = square_to(frame.rot * ik.bend, line);
    let along = joints[chain.len() / 2] - joints[0];
    let off_line = along - line * along.dot(line);
    let bent = chain.len() > 1 && off_line.length() > 1e-3;
    if bent {
        plane = off_line.normalize();
    }
    let keeps_bend = !bent || plane.dot(square_to(frame.rot * ik.bend, line)) > 0.99999;
    let fold = deepest_fold(&joints);
    let mut twist = ik.twist;
    if let ModifierKind::Ik(ik) = &mut scene.modifiers[index].kind {
        if !keeps_bend {
            ik.bend = frame.rot.inverse() * plane;
        }
        if let Some(limit) = &mut ik.limit {
            *limit = limit.max(fold);
        }
        // With no pole, a twist turns the limb, and nothing else could keep it still.
        if ik.pole.is_none() {
            ik.twist = 0.0;
            twist = 0.0;
        }
    }
    if let Some(pole) = ik.pole {
        let (start, toward, plane) = if ik.is_one_bone() {
            let bone = &scene.bones[root];
            let along = bone.frame.rot * long_axis(bone.tail_local);
            (bone.head(), along, square_to(bone.base.rot * ik.bend, along))
        } else {
            (joints[0], line, plane)
        };
        let reach = start.distance(scene.bones[ik.target].head()).max(1.0);
        let turned = Quat::from_axis_angle(toward, -twist.to_radians()) * plane;
        let at = start + toward * reach * 0.5 + turned * reach;
        scene.set_bone_frame(pole, Xf::new(scene.bones[pole].frame.rot, at));
    }
}

/// Switches a bone between Parent and Unparent with nothing moving, by working out the
/// pose that lands in the same spot from the new base. Returns how the bone's other keys
/// need to change to carry on from the new base, see `Animation::carry_switch`.
pub fn switch_unparent(scene: &mut Scene, index: usize, on: bool) -> Xf {
    let bone = scene.modifiers[index].bone;
    let (frame, old_base) = (scene.bones[bone].frame, scene.bones[bone].base);
    scene.modifiers[index].enabled = true;
    scene.bones[bone].value = if on { 1.0 } else { 0.0 };
    scene.evaluate();
    let new_base = scene.bones[bone].base;
    scene.bones[bone].pose = new_base.inverse() * frame;
    scene.evaluate();
    new_base.inverse() * old_base
}

/// The bone an edit of `bone` should really land on. Grabbing a foot under IK means
/// moving its target, since that's what the foot is following.
pub fn control_of(scene: &Scene, bone: BoneId) -> BoneId {
    scene
        .modifiers
        .iter()
        .find_map(|m| match m.kind {
            ModifierKind::Ik(ik) if m.bone == bone && influence(scene, m) > 0.0 => Some(ik.target),
            ModifierKind::LookAt(look) if m.bone == bone && influence(scene, m) > 0.0 => Some(look.target),
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
            ModifierKind::LookAt(look) if look.target == bone => Some(m.bone),
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
        _ => false,
    })
}

/// True when a bone has fully let go of its parent.
pub fn is_unparented(scene: &Scene, bone: BoneId) -> bool {
    scene.modifiers.iter().any(|m| match m.kind {
        ModifierKind::Unparent(u) => m.bone == bone && u.position && u.rotation && influence(scene, m) >= 0.999,
        _ => false,
    })
}

pub(crate) fn has_anchors(scene: &Scene) -> bool {
    scene.modifiers.iter().any(|m| matches!(m.kind, ModifierKind::Unparent(Unparent { anchor: Some(_), .. })))
}

/// Where a bone hangs from once Unparent and Inherit Rotation have had their say.
pub(crate) fn held_base(scene: &Scene, bone: BoneId, joint_base: Xf) -> Xf {
    if !scene.bones[bone].rebased {
        return joint_base;
    }
    let placement = placement_of(scene, bone);
    let mut base = joint_base;
    for modifier in &scene.modifiers {
        if modifier.bone != bone {
            continue;
        }
        let amount = influence(scene, modifier);
        match modifier.kind {
            ModifierKind::Unparent(u) => {
                let held = u.anchor.map_or(placement * u.hold, |a| scene.bones[a].frame * u.hold);
                if u.position {
                    base.pos = base.pos.lerp(held.pos, amount);
                }
                if u.rotation {
                    base.rot = base.rot.slerp(held.rot, amount).normalize();
                }
            }
            ModifierKind::Inherit(inherit) => base.rot = base.rot.slerp(placement.rot * inherit.held, amount).normalize(),
            _ => {}
        }
    }
    base
}

/// Runs every modifier that works on a placed rig. Those nearer the root go first, so
/// a spine moves before the arm hanging from it, and limits go after everything.
pub(crate) fn solve(scene: &mut Scene) {
    let depth = |scene: &Scene, mut bone: BoneId| {
        let mut count = 0;
        while let Some(parent) = scene.bones[bone].parent {
            bone = parent;
            count += 1;
        }
        count
    };
    let handled_elsewhere = |kind: ModifierKind| matches!(kind, ModifierKind::Unparent(_) | ModifierKind::Inherit(_) | ModifierKind::Ragdoll(_));
    let mut order: Vec<usize> = (0..scene.modifiers.len()).filter(|i| !handled_elsewhere(scene.modifiers[*i].kind)).collect();
    order.sort_by_key(|i| (matches!(scene.modifiers[*i].kind, ModifierKind::Limit(_)), depth(scene, scene.modifiers[*i].bone)));
    if order.is_empty() {
        dynamics::apply(scene);
        return;
    }
    // A chain only moves its own rig, so that's all that needs placing again.
    let placements = if crosses_rigs(scene) { Vec::new() } else { scene.rig_placements() };
    let mut physics_done = false;
    for index in order {
        let modifier = scene.modifiers[index];
        if !physics_done && matches!(modifier.kind, ModifierKind::Limit(_)) {
            dynamics::apply(scene);
            physics_done = true;
        }
        let amount = influence(scene, &modifier);
        if amount <= 0.0 {
            continue;
        }
        let placement = placements.get(scene.bones[modifier.bone].rig);
        match modifier.kind {
            ModifierKind::Ik(ik) => solve_ik(scene, modifier.bone, &ik, amount, placement),
            ModifierKind::LookAt(look) => solve_look_at(scene, modifier.bone, &look, amount, placement),
            ModifierKind::Copy(copy) => solve_copy(scene, modifier.bone, &copy, amount, placement),
            ModifierKind::Limit(limit) => solve_limit(scene, modifier.bone, &limit, placement),
            ModifierKind::Unparent(_) | ModifierKind::Inherit(_) | ModifierKind::Ragdoll(_) => {}
        }
    }
    if !physics_done {
        dynamics::apply(scene);
    }
}

/// Moves a bone part of the way to where a modifier wants it.
fn settle(scene: &mut Scene, bone: BoneId, wanted: Xf, amount: f32, placement: Option<&Placement>) {
    let b = &mut scene.bones[bone];
    let goal = b.base.inverse() * wanted;
    b.solved = Xf::new(b.solved.rot.slerp(goal.rot, amount).normalize(), b.solved.pos.lerp(goal.pos, amount));
    place_again(scene, placement);
}

fn solve_look_at(scene: &mut Scene, bone: BoneId, look: &LookAt, amount: f32, placement: Option<&Placement>) {
    let b = &scene.bones[bone];
    let Some(toward) = (scene.bones[look.target].head() - b.head()).try_normalize() else {
        return;
    };
    let turn = Quat::from_rotation_arc((b.frame.rot * look.axis).normalize(), toward);
    let wanted = Xf::new((turn * b.frame.rot).normalize(), b.head());
    settle(scene, bone, wanted, amount, placement);
}

fn solve_copy(scene: &mut Scene, bone: BoneId, copy: &CopyBone, amount: f32, placement: Option<&Placement>) {
    let Some(source) = copy.source else {
        return;
    };
    let from = scene.bones[source].frame * copy.offset;
    let frame = scene.bones[bone].frame;
    let wanted = Xf::new(if copy.rotation { from.rot } else { frame.rot }, if copy.position { from.pos } else { frame.pos });
    settle(scene, bone, wanted, amount, placement);
}

fn solve_limit(scene: &mut Scene, bone: BoneId, limit: &Limit, placement: Option<&Placement>) {
    let b = &mut scene.bones[bone];
    let angles = angles_near(b.solved.rot, [0.0; 3]);
    let held: [f32; 3] = std::array::from_fn(|i| if limit.axes[i] { angles[i].clamp(limit.min[i], limit.max[i]) } else { angles[i] });
    if held != angles {
        let [x, y, z] = held.map(f32::to_radians);
        b.solved.rot = Quat::from_euler(EulerRot::XYZ, x, y, z).normalize();
        place_again(scene, placement);
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
            ModifierKind::LookAt(look) => rig(look.target) != rig(m.bone),
            ModifierKind::Copy(copy) => copy.source.is_some_and(|s| rig(s) != rig(m.bone)),
            ModifierKind::Limit(_) | ModifierKind::Inherit(_) | ModifierKind::Ragdoll(_) => false,
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
    let line = (joints[chain.len()] - joints[0]).try_normalize().unwrap_or(toward);
    // The bend turns with the bone it starts from, so a limb posed by hand keeps its
    // knee where it was.
    let fold = square_to(scene.bones[root].frame.rot * ik.bend, line);
    let aim = match ik.pole {
        Some(pole) => scene.bones[pole].head() - joints[0],
        // Carried along as the limb swings, the bend can't be crossed by the target.
        None if ik.limit.is_some() => Quat::from_rotation_arc(line, toward) * fold,
        None => scene.bones[root].frame.rot * ik.bend,
    };
    let bend = Quat::from_axis_angle(toward, ik.twist.to_radians()) * square_to(aim, toward);
    let goal = reach(&joints, target.pos, toward, bend, ik.limit);

    // A pole or twist turns the whole chain to face its way, which leaves each bone
    // only folding in that plane, as a knee does. Swinging the bones straight to the
    // new bend would leave the limb facing forward with its knee folding sideways.
    if ik.pole.is_some() || ik.twist != 0.0 {
        let bone = &mut scene.bones[root];
        let face = match ik.pole {
            Some(_) => carry(line, fold, toward, bend),
            // Starts from nothing, so a small twist is a small change.
            None => Quat::from_axis_angle(line, ik.twist.to_radians()),
        };
        let frame = Xf::new((face * bone.frame.rot).normalize(), bone.head());
        bone.solved = bone.base.inverse() * frame;
        place_again(scene, placement);
    }

    // Turn each bone in order, from the root out, reading where the next point has
    // actually landed. Bones often sit off each other's axes, so it has to be read.
    for (i, bone) in chain.iter().enumerate() {
        let head = scene.bones[*bone].head();
        let end = chain.get(i + 1).map_or_else(|| effector(scene, tip, ik.end), |next| scene.bones[*next].head());
        if let (Some(from), Some(to)) = ((end - head).try_normalize(), (goal[i + 1] - head).try_normalize()) {
            let turn = Quat::from_rotation_arc(from, to);
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
/// target, which `Transform` allows, and its middle goes the way a knee or elbow would
/// stick out. Its end is held on the target, so that means aiming it the other way.
/// Only moves across the limb swing it. A joint usually sits off to one side of the
/// limb, so aiming from the joint would tip the limb sideways as its end came up.
/// A pole or twist turns the limb on its own length, which is why it's placed by its
/// far end: the joint is what has to give way, so the limb stays in its place.
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
    let rest_bend = bone.base.rot * ik.bend;
    let wanted = aim_at.map_or(rest_bend, |pole| pole - head);
    let bend = Quat::from_axis_angle(along, ik.twist.to_radians()) * square_to(wanted, along);
    let across = (target - end) - along * (target - end).dot(along);
    let lean = (length * length - offset.length_squared()).max(0.0).sqrt() * ik.lean;
    let close = (end - head) + across - bend * lean;
    // Blended in over the last stretch of reach, so nothing jumps at full stretch.
    let aim = offset.lerp(close, (spare / length * 4.0).clamp(0.0, 1.0)).try_normalize().unwrap_or(keyed);
    let face = carry(along, square_to(rest_bend, along), along, bend);
    let rot = (Quat::from_rotation_arc(keyed, aim) * face * bone.frame.rot).normalize();
    let far_end = if spare > 0.0 { target } else { head + aim * length };
    let solved = bone.base.inverse() * Xf::new(rot, far_end - rot * bone.tail_local);
    scene.bones[tip].solved = solved;
}

/// The bone's own axis nearest the way it points.
pub(crate) fn long_axis(tail_local: Vec3) -> Vec3 {
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

/// The turn that carries one line, and the way it bends, onto another. Each bend has
/// to be a unit direction square to its line.
fn carry(from: Vec3, from_bend: Vec3, to: Vec3, to_bend: Vec3) -> Quat {
    let frame = |line: Vec3, bend: Vec3| Quat::from_mat3(&Mat3::from_cols(line, bend, line.cross(bend)));
    frame(to, to_bend) * frame(from, from_bend).inverse()
}

/// Where each point of a chain should end up, for chains of any length.
fn reach(joints: &[Vec3], target: Vec3, toward: Vec3, bend: Vec3, limit: Option<f32>) -> Vec<Vec3> {
    match joints {
        [root, end] => vec![*root, *root + toward * root.distance(*end)],
        [root, middle, end] => two_bones(*root, root.distance(*middle), middle.distance(*end), target, toward, bend, limit),
        _ => fabrik(joints, target, toward, bend, limit),
    }
}

/// The exact answer for two bones, from the law of cosines. Limbs are nearly always
/// this, and unlike the general solver it can't drift between frames. A limit on the
/// fold is a limit on how close the end can come to the root.
fn two_bones(root: Vec3, upper: f32, lower: f32, target: Vec3, toward: Vec3, bend: Vec3, limit: Option<f32>) -> Vec<Vec3> {
    let closest = limit.map_or(0.0, |fold| {
        let inside = (180.0 - fold).clamp(0.0, 180.0).to_radians();
        (upper * upper + lower * lower - 2.0 * upper * lower * inside.cos()).max(0.0).sqrt()
    });
    let shortest = (upper - lower).abs().max(1e-4).max(closest);
    let distance = root.distance(target).clamp(shortest, (upper + lower).max(shortest));
    let cos = ((upper * upper + distance * distance - lower * lower) / (2.0 * upper * distance).max(1e-6)).clamp(-1.0, 1.0);
    let sin = (1.0 - cos * cos).sqrt();
    vec![root, root + toward * (upper * cos) + bend * (upper * sin), root + toward * distance]
}

/// Keeps a joint folded forward, and by no more than the limit, in radians.
fn within_limit(before: Vec3, after: Vec3, hinge: Vec3, limit: f32) -> Vec3 {
    let folded = before.cross(after).dot(hinge).atan2(before.dot(after));
    if (0.0..=limit).contains(&folded) {
        return after;
    }
    Quat::from_axis_angle(hinge, folded.clamp(0.0, limit)) * before
}

fn fabrik(joints: &[Vec3], target: Vec3, toward: Vec3, bend: Vec3, limit: Option<f32>) -> Vec<Vec3> {
    let lengths: Vec<f32> = joints.windows(2).map(|w| w[0].distance(w[1])).collect();
    let total: f32 = lengths.iter().sum();
    let last = joints.len() - 1;
    let root = joints[0];
    // Already there, which is what lets a switch to IK leave a long chain alone.
    if joints[last].distance(target) < FABRIK_TOLERANCE {
        return joints.to_vec();
    }
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
    let hinge = bend.cross(toward);
    let limit = limit.map(f32::to_radians);
    // Leaning eases in with how far there is to go, so a target just leaving the end
    // of the chain doesn't make it jump.
    let lean = total * CHAIN_LEAN * (joints[last].distance(target) / total * 4.0).min(1.0);
    let mut points = joints.to_vec();
    for (i, point) in points.iter_mut().enumerate() {
        *point += bend * lean * (PI * i as f32 / last as f32).sin();
    }
    for _ in 0..FABRIK_PASSES {
        points[last] = target;
        for i in (0..last).rev() {
            let dir = (points[i] - points[i + 1]).try_normalize().unwrap_or(bend);
            points[i] = points[i + 1] + dir * lengths[i];
        }
        points[0] = root;
        let mut before = None;
        for i in 0..last {
            let mut dir = (points[i + 1] - points[i]).try_normalize().unwrap_or(bend);
            if let (Some(before), Some(limit)) = (before, limit) {
                dir = within_limit(before, dir, hinge, limit);
            }
            points[i + 1] = points[i] + dir * lengths[i];
            before = Some(dir);
        }
        if points[last].distance(target) < FABRIK_TOLERANCE {
            break;
        }
    }
    points
}
