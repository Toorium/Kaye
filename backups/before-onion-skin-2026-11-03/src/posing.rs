//! Things that work on whole poses: copying and pasting, mirroring, and baking what
//! modifiers do into plain keys. Like the rig logic, this has no interface code.

use glam::{Quat, Vec3};

use crate::animation::{Animation, EaseStyle, Easing, FrameNumber, Key, KeyId};
use crate::dynamics;
use crate::math::Xf;
use crate::modifiers::{self, ModifierKind};
use crate::scene::{BoneId, Drive, RigId, Scene};

/// One bone's pose on the clipboard. The name is what finds it on another rig.
#[derive(Clone, Debug)]
pub struct CopiedBone {
    id: BoneId,
    rig: RigId,
    name: String,
    pose: Xf,
}

pub type PoseCopy = Vec<CopiedBone>;

pub fn copy_pose(scene: &Scene, bones: &[BoneId]) -> PoseCopy {
    bones
        .iter()
        .map(|id| {
            let bone = &scene.bones[*id];
            CopiedBone { id: *id, rig: bone.rig, name: bone.name.clone(), pose: bone.pose }
        })
        .collect()
}

/// The poses a paste onto this rig would write. It lands on the same bones in the rig
/// it came from, and on bones with the same name in any other.
pub fn paste_pose(scene: &Scene, copied: &[CopiedBone], rig: RigId) -> Vec<(BoneId, Xf)> {
    let bones = scene.live_bones(rig);
    copied
        .iter()
        .filter_map(|c| {
            let same = (c.rig == rig && bones.contains(&c.id)).then_some(c.id);
            let found = same.or_else(|| bones.iter().copied().find(|b| scene.bones[*b].name == c.name));
            found.map(|bone| (bone, c.pose))
        })
        .collect()
}

/// One key on the clipboard. Like a copied pose, the name finds the bone on another rig.
#[derive(Clone, Debug)]
pub struct CopiedKey {
    id: BoneId,
    name: String,
    key: Key,
}

pub type KeyCopy = Vec<CopiedKey>;

pub fn copy_keys(scene: &Scene, animation: &Animation, keys: &[KeyId]) -> KeyCopy {
    keys.iter()
        .filter_map(|(id, frame)| {
            let key = animation.key_at(*id, *frame)?;
            Some(CopiedKey { id: *id, name: scene.bones[*id].name.clone(), key: *key })
        })
        .collect()
}

/// Writes copied keys so the earliest lands on `frame`, and returns where they went. They
/// land on a bone of the same name in the rig in focus, and otherwise on the bone they
/// came from, so a camera's keys aren't lost by having a character selected.
pub fn paste_keys(scene: &Scene, animation: &mut Animation, copied: &[CopiedKey], rig: Option<RigId>, frame: FrameNumber) -> Vec<KeyId> {
    let Some(earliest) = copied.iter().map(|c| c.key.frame).min() else {
        return Vec::new();
    };
    let delta = frame - earliest;
    let focused = rig.map_or_else(Vec::new, |rig| scene.live_bones(rig));
    let mut pasted = Vec::with_capacity(copied.len());
    for c in copied {
        let named = (!focused.contains(&c.id)).then(|| focused.iter().copied().find(|b| scene.bones[*b].name == c.name)).flatten();
        let bone = named.unwrap_or(c.id);
        if !scene.bones[bone].removed {
            let key = Key { frame: c.key.frame + delta, ..c.key };
            animation.tracks.entry(bone).or_default().insert(key);
            pasted.push((bone, key.frame));
        }
    }
    pasted
}

/// The name of the same bone on the other side, if it says which side it's on.
pub fn mirrored_name(name: &str) -> Option<String> {
    for (a, b) in [("Left", "Right"), ("left", "right"), ("LEFT", "RIGHT")] {
        if name.contains(a) {
            return Some(name.replace(a, b));
        }
        if name.contains(b) {
            return Some(name.replace(b, a));
        }
    }
    let swap = |c: char| match c {
        'L' => Some('R'),
        'R' => Some('L'),
        'l' => Some('r'),
        'r' => Some('l'),
        _ => None,
    };
    // Blender's habit of ending names in .L or _R.
    let mut ending = name.chars().rev();
    let (last, mark) = (ending.next()?, ending.next()?);
    if matches!(mark, '.' | '_' | '-') {
        let stem = &name[..name.len() - last.len_utf8()];
        return swap(last).map(|other| format!("{stem}{other}"));
    }
    None
}

/// A rotation seen in a mirror across the character's middle.
fn mirror_rotation(q: Quat) -> Quat {
    Quat::from_xyzw(q.x, -q.y, -q.z, q.w)
}

fn mirror_vector(v: Vec3) -> Vec3 {
    Vec3::new(-v.x, v.y, v.z)
}

/// Where the rig stands and how it's turned, which the mirror is measured from.
fn rig_space(scene: &Scene, rig: RigId) -> Xf {
    let standing_on = scene.rigs[rig].parts.iter().find(|p| scene.parts[**p].drive == Drive::Root);
    standing_on.map_or(Xf::IDENTITY, |p| scene.parts[*p].world)
}

/// The poses that mirror these bones across their rig's middle. Each bone's pose lands
/// on the bone on the other side, so selecting one arm copies it across and selecting
/// both swaps them. Bones down the middle mirror onto themselves.
///
/// A pose is local to its joint, and the two sides of a Roblox rig rarely share axes.
/// So each pose is turned into the change it makes at rest, seen from the rig. That
/// is what's mirrored, and it goes back through the rest pose of the other bone.
pub fn mirror_pose(scene: &Scene, bones: &[BoneId]) -> Vec<(BoneId, Xf)> {
    mirror_pairs(scene, bones).into_iter().map(|(_, target, pose)| (target, pose)).collect()
}

/// `mirror_pose`, keeping which bone each pose came from.
fn mirror_pairs(scene: &Scene, bones: &[BoneId]) -> Vec<(BoneId, BoneId, Xf)> {
    let mut rest = scene.clone();
    rest.modifiers.clear();
    rest.bones.iter_mut().for_each(|b| b.pose = Xf::IDENTITY);
    rest.evaluate();
    let partner = |bone: BoneId| {
        let b = &scene.bones[bone];
        let name = mirrored_name(&b.name)?;
        scene.live_bones(b.rig).into_iter().find(|other| scene.bones[*other].name == name)
    };
    let mut out = Vec::new();
    let mut controls = Vec::new();
    for &bone in bones {
        let b = &scene.bones[bone];
        let target = partner(bone).or(b.kind.is_joint().then_some(bone));
        let Some(target) = target else {
            continue;
        };
        if !b.kind.is_joint() {
            controls.push((bone, target));
            continue;
        }
        let space = rig_space(&rest, b.rig).rot.inverse();
        let (from, to) = (space * rest.bones[bone].base.rot, space * rest.bones[target].base.rot);
        let turned = mirror_rotation(from * b.pose.rot * from.inverse());
        let moved = mirror_vector(from * b.pose.pos);
        out.push((bone, target, Xf::new((to.inverse() * turned * to).normalize(), to.inverse() * moved)));
    }
    if controls.is_empty() {
        return out;
    }
    // Controls follow bones that have just moved, so these go from where those land.
    let mut mirrored = scene.clone();
    for (_, bone, pose) in &out {
        mirrored.bones[*bone].pose = *pose;
    }
    mirrored.evaluate();
    for (bone, target) in controls {
        let b = &scene.bones[bone];
        let space = rig_space(&rest, b.rig);
        let local = space.inverse() * b.frame;
        let (from, to) = (space.rot.inverse() * rest.bones[bone].frame.rot, space.rot.inverse() * rest.bones[target].frame.rot);
        let turned = mirror_rotation(local.rot * from.inverse()) * to;
        let wanted = space * Xf::new(turned.normalize(), mirror_vector(local.pos));
        out.push((bone, target, mirrored.bones[target].base.inverse() * wanted));
    }
    out
}

/// The poses a mirrored paste onto this rig would write: what a plain paste would, then
/// mirrored across the rig, as Blender's Paste X-Flipped does. Copying the left arm and
/// pasting it mirrored poses the right arm.
pub fn paste_mirrored(scene: &Scene, copied: &[CopiedBone], rig: RigId) -> Vec<(BoneId, Xf)> {
    let pasted = paste_pose(scene, copied, rig);
    let mut posed = scene.clone();
    for (bone, pose) in &pasted {
        posed.bones[*bone].pose = *pose;
    }
    posed.evaluate();
    let bones: Vec<BoneId> = pasted.iter().map(|(bone, _)| *bone).collect();
    mirror_pose(&posed, &bones)
}

/// Keys mirrored across the rig onto the bones on the other side, each on the frame of the
/// key it came from. Picking one arm's keys copies them across, picking both swaps them,
/// and keys down the middle mirror onto themselves. Easing comes along, but handles set by
/// hand don't, since the two sides' axes rarely match.
pub fn mirror_keys(scene: &Scene, animation: &Animation, keys: &[KeyId]) -> Vec<(BoneId, Key)> {
    let mut frames: Vec<FrameNumber> = keys.iter().map(|(_, frame)| *frame).collect();
    frames.sort_unstable();
    frames.dedup();
    let mut out = Vec::new();
    for frame in frames {
        let bones: Vec<BoneId> = keys.iter().filter(|(_, f)| *f == frame).map(|(bone, _)| *bone).collect();
        // Everything as it stands on that frame, so controls mirror from where their bones are.
        let mut posed = scene.clone();
        animation.apply(&mut posed, frame as f32);
        posed.evaluate();
        for (from, target, pose) in mirror_pairs(&posed, &bones) {
            if let Some(key) = animation.key_at(from, frame) {
                out.push((target, Key { pose, handles: Default::default(), ..*key }));
            }
        }
    }
    out
}

/// Turns what these modifiers do into plain keys on every frame of the animation, then
/// takes them off, so the result plays with nothing solving it.
pub fn bake(scene: &mut Scene, animation: &mut Animation, indices: &[usize]) {
    let physics = dynamics::bake(scene, animation);
    scene.dynamics.set_baked(physics);
    let mut baked: Vec<(BoneId, bool)> = Vec::new();
    for index in indices {
        let unparent = matches!(scene.modifiers[*index].kind, ModifierKind::Unparent(_) | ModifierKind::Inherit(_) | ModifierKind::Ragdoll(_));
        for bone in modifiers::driven_bones(scene, *index) {
            if !baked.iter().any(|(b, _)| *b == bone) {
                baked.push((bone, unparent));
            }
        }
    }
    let (start, end) = (animation.start, animation.end);
    let mut keys: Vec<(BoneId, FrameNumber, Xf, f32)> = Vec::with_capacity(baked.len() * (end - start + 1).max(0) as usize);
    for frame in start..=end {
        animation.apply(scene, frame as f32);
        for (id, unparent) in &baked {
            let bone = &scene.bones[*id];
            // These results are only true of the bone's real joint. IK's is a pose.
            let pose = if *unparent { bone.output() } else { bone.solved };
            keys.push((*id, frame, pose, bone.value));
        }
    }
    let mut order = indices.to_vec();
    order.sort_unstable();
    for index in order.into_iter().rev() {
        for gone in modifiers::remove(scene, index) {
            animation.forget_bone(gone);
        }
    }
    for (bone, _) in &baked {
        if let Some(track) = animation.tracks.get_mut(bone) {
            track.keys.retain(|k| k.frame < start || k.frame > end);
        }
    }
    let default_easing = animation.default_easing;
    animation.default_easing = Easing { style: EaseStyle::Linear, ..default_easing };
    for (bone, frame, pose, value) in keys {
        animation.key_shown(bone, frame, pose, value);
    }
    animation.default_easing = default_easing;
}
