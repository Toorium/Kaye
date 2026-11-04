//! Writes an animation as a Roblox KeyframeSequence, in a `.rbxm` file Studio can insert.
//! Like the rig logic, this has no interface code.
//!
//! Roblox has no Bezier handles, IK, Unparent or physics, and only some of Kaye's easings,
//! so the animation is baked: one Linear pose per frame, of `Bone::output`, which is what
//! the real joint needs to land where Kaye drew it. A Pose's CFrame is a joint's
//! `Transform`, and it finds its joint by the name of the part or bone it moves, nested
//! under the Pose of whatever that hangs from.
//!
//! It can come packed a few ways: the rig's own model with all its animations saved inside
//! it, see `model_with_animations`, or just KeyframeSequences, all of them or the one playing.

use std::collections::BTreeSet;
use std::path::Path;

use rbx_dom_weak::{InstanceBuilder, WeakDom};
use rbx_types::{CFrame, Enum, Matrix3, Ref, Variant, Vector3};
use ustr::ustr;

use crate::animation::Animation;
use crate::dynamics;
use crate::math::Xf;
use crate::scene::{BoneId, BoneLink, RigId, Scene};

pub const EXTENSION: &str = "rbxm";

/// What an exported animation comes packed as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pack {
    /// The rig's model, with all its animations in its AnimSaves.
    Rig,
    /// A KeyframeSequence for each of its animations.
    Keyframe,
    /// A KeyframeSequence for the animation playing.
    CurrentKeyframe,
}

/// Where Roblox's Animation Editor keeps a rig's animations.
const ANIM_SAVES: &str = "AnimSaves";
/// `Enum.AnimationPriority.Action`, Roblox's own default for a new animation.
const PRIORITY_ACTION: u32 = 2;
/// `Enum.PoseEasingStyle.Linear`.
const EASE_LINEAR: u32 = 0;
const EASE_IN: u32 = 0;
/// Close enough to no change at all to leave a joint out of the animation.
const STILL: f32 = 1e-4;

fn cframe(xf: Xf) -> CFrame {
    let m = glam::Mat3::from_quat(xf.rot);
    CFrame::new(
        Vector3::new(xf.pos.x, xf.pos.y, xf.pos.z),
        // rbx_types stores the rotation by rows; glam keeps columns.
        Matrix3::new(Vector3::new(m.x_axis.x, m.y_axis.x, m.z_axis.x), Vector3::new(m.x_axis.y, m.y_axis.y, m.z_axis.y), Vector3::new(m.x_axis.z, m.y_axis.z, m.z_axis.z)),
    )
}

/// One node of the Pose tree each Keyframe holds.
struct PoseNode {
    name: String,
    /// `None` for something only there to reach what hangs from it, like HumanoidRootPart.
    /// It gets no weight, so it can't hold down an animation under this one.
    bone: Option<BoneId>,
    children: Vec<PoseNode>,
}

/// What a joint moves goes by that part's name, and a skinned bone by its own.
fn pose_name(scene: &Scene, bone: BoneId) -> String {
    match scene.bones[bone].link {
        BoneLink::Joint { part1, .. } => scene.parts[part1].name.clone(),
        _ => scene.bones[bone].name.clone(),
    }
}

/// The part a chain of joints starts from, when nothing above it moves.
fn root_name(scene: &Scene, bone: BoneId) -> Option<String> {
    match scene.bones[bone].link {
        BoneLink::Joint { part0, .. } => Some(scene.parts[part0].name.clone()),
        BoneLink::Skinned { parent_part, .. } => parent_part.map(|part| scene.parts[part].name.clone()),
        BoneLink::Free { .. } => None,
    }
}

fn node_for(scene: &Scene, bone: BoneId, wanted: &BTreeSet<BoneId>) -> PoseNode {
    let children = scene.bones[bone].children.iter().copied().filter(|c| wanted.contains(c) || has_wanted_below(scene, *c, wanted));
    let children = children.map(|c| node_for(scene, c, wanted)).collect();
    PoseNode { name: pose_name(scene, bone), bone: wanted.contains(&bone).then_some(bone), children }
}

fn has_wanted_below(scene: &Scene, bone: BoneId, wanted: &BTreeSet<BoneId>) -> bool {
    scene.bones[bone].children.iter().any(|c| wanted.contains(c) || has_wanted_below(scene, *c, wanted))
}

fn pose_tree(scene: &Scene, joints: &[BoneId], wanted: &BTreeSet<BoneId>) -> Vec<PoseNode> {
    let mut roots: Vec<PoseNode> = Vec::new();
    for &bone in joints {
        let top = scene.bones[bone].parent.is_none_or(|p| !joints.contains(&p));
        if !top || !(wanted.contains(&bone) || has_wanted_below(scene, bone, wanted)) {
            continue;
        }
        let node = node_for(scene, bone, wanted);
        match root_name(scene, bone) {
            Some(name) => match roots.iter_mut().find(|r| r.name == name) {
                Some(root) => root.children.push(node),
                None => roots.push(PoseNode { name, bone: None, children: vec![node] }),
            },
            None => roots.push(node),
        }
    }
    roots
}

fn insert_pose(dom: &mut WeakDom, parent: Ref, node: &PoseNode, poses: &[Xf]) {
    let (pose, weight) = node.bone.map_or((Xf::IDENTITY, 0.0), |bone| (poses[bone], 1.0));
    let builder = InstanceBuilder::new("Pose")
        .with_name(node.name.clone())
        .with_property("CFrame", Variant::CFrame(cframe(pose)))
        .with_property("EasingStyle", Variant::Enum(Enum::from_u32(EASE_LINEAR)))
        .with_property("EasingDirection", Variant::Enum(Enum::from_u32(EASE_IN)))
        .with_property("Weight", Variant::Float32(weight));
    let inserted = dom.insert(parent, builder);
    for child in &node.children {
        insert_pose(dom, inserted, child, poses);
    }
}

/// Builds the KeyframeSequence, which ends up the one thing at the top of the file.
pub fn keyframe_sequence(scene: &Scene, animation: &Animation, rig: RigId) -> Result<WeakDom, String> {
    let joints = scene.joints(rig);
    if joints.is_empty() {
        return Err("This rig has no joints to animate.".to_string());
    }
    let mut playing = scene.clone();
    playing.dynamics.set_baked(dynamics::bake(scene, animation));
    let (start, end) = (animation.start, animation.end.max(animation.start));
    let keyed: BTreeSet<BoneId> = animation.tracks.keys().copied().collect();
    let mut frames: Vec<Vec<Xf>> = Vec::with_capacity((end - start + 1) as usize);
    let mut wanted: BTreeSet<BoneId> = joints.iter().copied().filter(|b| keyed.contains(b)).collect();
    for frame in start..=end {
        animation.apply(&mut playing, frame as f32);
        let poses: Vec<Xf> = playing.bones.iter().map(|b| b.output()).collect();
        for &bone in &joints {
            if poses[bone].pos.length() > STILL || poses[bone].rot.angle_between(glam::Quat::IDENTITY) > STILL {
                wanted.insert(bone);
            }
        }
        frames.push(poses);
    }
    if wanted.is_empty() {
        return Err("Nothing moves in this animation yet.".to_string());
    }
    let tree = pose_tree(scene, &joints, &wanted);

    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let sequence = InstanceBuilder::new("KeyframeSequence")
        .with_name(animation.active_clip(rig).map_or_else(|| scene.rigs[rig].name.clone(), |clip| animation.clips[clip].name.clone()))
        .with_property("Loop", Variant::Bool(animation.looping))
        .with_property("Priority", Variant::Enum(Enum::from_u32(PRIORITY_ACTION)))
        .with_property("AuthoredHipHeight", Variant::Float32(2.0));
    let sequence = dom.insert(root, sequence);
    for (i, poses) in frames.iter().enumerate() {
        let time = i as f32 / animation.fps;
        let keyframe = dom.insert(sequence, InstanceBuilder::new("Keyframe").with_name("Keyframe").with_property("Time", Variant::Float32(time)));
        for node in &tree {
            insert_pose(&mut dom, keyframe, node, poses);
        }
    }
    Ok(dom)
}

/// Moves the rig's parts to where it was placed in Kaye. Everything else in a model is
/// relative to its parts, apart from where a Model keeps its pivot.
fn apply_placement(dom: &mut WeakDom, placement: Xf) {
    let refs: Vec<Ref> = dom.descendants().map(|i| i.referent()).collect();
    for r in refs {
        let Some(inst) = dom.get_by_ref_mut(r) else {
            continue;
        };
        let is_part = inst.properties.contains_key(&ustr("Size"));
        if let (true, Some(Variant::CFrame(at))) = (is_part, inst.properties.get_mut(&ustr("CFrame"))) {
            *at = cframe(placement * Xf::from_cframe(at));
        }
        if let Some(Variant::OptionalCFrame(Some(at))) = inst.properties.get_mut(&ustr("WorldPivotData")) {
            *at = cframe(placement * Xf::from_cframe(at));
        }
    }
}

/// A KeyframeSequence for each of a rig's animations that has anything in it. Each is played
/// with its own range and with this rig's other poses out of the way. Other rigs keep
/// playing what they're playing, since that's what a React would bump into.
fn every_sequence(scene: &Scene, animation: &Animation, rig: RigId) -> Result<Vec<WeakDom>, String> {
    let mut animation = animation.clone();
    animation.ensure_clips(scene);
    let mut sequences = Vec::new();
    let mut first_problem = None;
    let current = animation.active_clip(rig);
    for clip in animation.clips_of(rig) {
        let playing = animation.playing_clip(scene, clip);
        let mut world = scene.clone();
        if Some(clip) != current {
            playing.rest_unkeyed(&mut world, rig);
        }
        match keyframe_sequence(&world, &playing, rig) {
            Ok(dom) => sequences.push(dom),
            Err(problem) => first_problem = first_problem.or(Some(problem)),
        }
    }
    match (sequences.is_empty(), first_problem) {
        (true, Some(problem)) => Err(problem),
        _ => Ok(sequences),
    }
}

/// Every one of a rig's animations, as KeyframeSequences side by side in one file.
pub fn all_keyframe_sequences(scene: &Scene, animation: &Animation, rig: RigId) -> Result<WeakDom, String> {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    for mut saved in every_sequence(scene, animation, rig)? {
        let sequence = saved.root().children()[0];
        saved.transfer(sequence, &mut dom, root);
    }
    Ok(dom)
}

/// Builds the rig's own model with all its animations saved inside it, the way Roblox's
/// Animation Editor keeps them: KeyframeSequences in an ObjectValue called AnimSaves.
pub fn model_with_animations(scene: &Scene, animation: &Animation, rig: RigId) -> Result<WeakDom, String> {
    let sequences = every_sequence(scene, animation, rig)?;
    let Some(original) = &scene.rigs[rig].model else {
        return Err("This project doesn't have the rig's original model. Import the rig again, then animate that one.".to_string());
    };
    let mut dom = rbx_binary::from_reader(original.as_slice()).map_err(|e| e.to_string())?;
    if !scene.rigs[rig].placement.approx_eq(Xf::IDENTITY) {
        apply_placement(&mut dom, scene.rigs[rig].placement);
    }

    // A file of loose parts has no model to hold anything, and Studio's Animation Editor
    // wants a Model, so those get one around them.
    let top: Vec<Ref> = dom.root().children().to_vec();
    let model = match top.as_slice() {
        [only] if dom.get_by_ref(*only).is_some_and(|i| i.class == "Model") => *only,
        _ => {
            let root = dom.root_ref();
            let model = dom.insert(root, InstanceBuilder::new("Model").with_name(scene.rigs[rig].name.clone()));
            for r in top {
                dom.transfer_within(r, model);
            }
            model
        }
    };

    let existing = dom.get_by_ref(model).and_then(|m| m.children().iter().copied().find(|c| dom.get_by_ref(*c).is_some_and(|i| i.class == "ObjectValue" && i.name == ANIM_SAVES)));
    let saves = existing.unwrap_or_else(|| dom.insert(model, InstanceBuilder::new("ObjectValue").with_name(ANIM_SAVES)));
    for mut saved in sequences {
        let sequence = saved.root().children()[0];
        let name = saved.get_by_ref(sequence).map_or_else(String::new, |i| i.name.clone());
        // Saving under a name that's taken replaces it, as Studio does.
        let same_name: Vec<Ref> = dom.get_by_ref(saves).map_or_else(Vec::new, |s| s.children().iter().copied().filter(|c| dom.get_by_ref(*c).is_some_and(|i| i.name == name)).collect());
        for r in same_name {
            dom.destroy(r);
        }
        saved.transfer(sequence, &mut dom, saves);
    }
    Ok(dom)
}

/// Writes just the KeyframeSequence for the animation playing.
pub fn write_animation(path: &Path, scene: &Scene, animation: &Animation, rig: RigId) -> Result<(), String> {
    write_file(path, &keyframe_sequence(scene, animation, rig)?)
}

/// Writes a KeyframeSequence for each of the rig's animations.
pub fn write_all_animations(path: &Path, scene: &Scene, animation: &Animation, rig: RigId) -> Result<(), String> {
    write_file(path, &all_keyframe_sequences(scene, animation, rig)?)
}

/// Writes the rig's model with all its animations inside it.
pub fn write_model(path: &Path, scene: &Scene, animation: &Animation, rig: RigId) -> Result<(), String> {
    write_file(path, &model_with_animations(scene, animation, rig)?)
}

/// Writes the file, only replacing an old one once the new one is all there.
fn write_file(path: &Path, dom: &WeakDom) -> Result<(), String> {
    let mut bytes = Vec::new();
    rbx_binary::to_writer(&mut bytes, dom, dom.root().children()).map_err(|e| e.to_string())?;
    let temporary = path.with_added_extension("tmp");
    let written = std::fs::write(&temporary, &bytes).and_then(|()| std::fs::rename(&temporary, path));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    Ok(())
}

/// The path with `.rbxm` on the end, for a name typed without it.
pub fn with_extension(path: std::path::PathBuf) -> std::path::PathBuf {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION)) { path } else { path.with_added_extension(EXTENSION) }
}
