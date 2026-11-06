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
//!
//! A camera or spawned shape has no joint for an animation to turn, so it goes as a small
//! rig of its own: a root part where its root is, like a character's HumanoidRootPart,
//! and a part for the object, joined by a Motor6D. A camera's part is one stud across and
//! faces the way the camera looks. See `single_model`.

use std::collections::BTreeSet;
use std::path::Path;

use rbx_dom_weak::{InstanceBuilder, WeakDom};
use rbx_types::{Attributes, CFrame, Color3, ContentId, Enum, Matrix3, Ref, Variant, Vector3};
use ustr::ustr;

use crate::animation::Animation;
use crate::dynamics;
use crate::materials::Material;
use crate::math::Xf;
use crate::scene::{BoneId, BoneLink, FolderId, PartShape, RigId, RigKind, Scene};

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
/// The part a camera or shape's rig stands on, named as a character's is, since that's
/// what Studio's Animation Editor looks for.
pub const ROOT_PART: &str = "HumanoidRootPart";
/// A camera's own part, which a script can copy the view from.
pub const CAMERA_PART: &str = "Camera";
/// The joint between the root and the object.
const ROOT_JOINT: &str = "RootJoint";
/// The camera's field of view, as an attribute on its part. A Roblox animation can't
/// change it, so this is where it starts.
pub const FOV_ATTRIBUTE: &str = "FieldOfView";
const CAMERA_COLOR: Color3 = Color3 { r: 0.26, g: 0.55, b: 0.95 };

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
pub(crate) fn pose_name(scene: &Scene, bone: BoneId) -> String {
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
    if scene.rigs[rig].kind.is_single() {
        let moves = single_moves(scene, animation, rig)?;
        let shown = animation.active_clip(rig);
        let clip = moves.clips.iter().find(|c| Some(c.clip) == shown).ok_or("Nothing moves in this animation yet.")?;
        return Ok(single_sequence(&moves, clip, animation.fps));
    }
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
/// playing what they're playing, since that's what a Ragdoll would land on.
fn every_sequence(scene: &Scene, animation: &Animation, rig: RigId) -> Result<Vec<WeakDom>, String> {
    if scene.rigs[rig].kind.is_single() {
        let moves = single_moves(scene, animation, rig)?;
        if moves.clips.is_empty() {
            return Err("Nothing moves in this animation yet.".to_string());
        }
        return Ok(moves.clips.iter().map(|clip| single_sequence(&moves, clip, animation.fps)).collect());
    }
    let mut animation = animation.clone();
    animation.ensure_clips(scene);
    let mut sequences = Vec::new();
    let mut first_problem = None;
    let current = animation.active_clip(rig);
    for clip in animation.clips_of(rig) {
        let mut world = scene.clone();
        let playing = animation.playing_clip(&mut world, clip);
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
    if scene.rigs[rig].kind.is_single() {
        let moves = single_moves(scene, animation, rig)?;
        if moves.clips.is_empty() {
            return Err("Nothing moves in this animation yet.".to_string());
        }
        return Ok(single_model(scene, &moves, animation.fps));
    }
    model_with(scene, rig, every_sequence(scene, animation, rig)?)
}

/// The rig's model with whatever animations it has. A rig with nothing animated yet still
/// comes through as just its model, which is what dragging it into Studio wants.
fn model_or_bare(scene: &Scene, animation: &Animation, rig: RigId) -> Result<WeakDom, String> {
    if scene.rigs[rig].kind.is_single() {
        return Ok(single_model(scene, &single_moves(scene, animation, rig)?, animation.fps));
    }
    model_with(scene, rig, every_sequence(scene, animation, rig).unwrap_or_default())
}

/// The rig's model with whatever animations it has, as the bytes of a `.rbxm`.
pub fn model_file(scene: &Scene, animation: &Animation, rig: RigId) -> Result<Vec<u8>, String> {
    to_bytes(&model_or_bare(scene, animation, rig)?)
}

/// Something picked in the Workspace to go out together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    /// A rig, camera or shape.
    Rig(RigId),
    /// A folder, with everything in it.
    Folder(FolderId),
}

/// Everything in `items` as one `.rbxm`, side by side at the top of the file, so Studio
/// inserts them all at once and they keep where they stand from each other. A folder
/// becomes a Folder holding what's in it. Also returns a line for each rig that had to be
/// left out, and fails only when nothing could come.
pub fn items_file(scene: &Scene, animation: &Animation, items: &[Item]) -> Result<(Vec<u8>, Vec<String>), String> {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let mut left_out = Vec::new();
    let mut added = 0;
    for &item in items {
        added += add_item(scene, animation, item, &mut dom, root, &mut left_out);
    }
    if added == 0 {
        return Err(left_out.first().cloned().unwrap_or_else(|| "There's nothing in it to send.".to_string()));
    }
    Ok((to_bytes(&dom)?, left_out))
}

/// Puts one item under `parent`, returning how many rigs went in.
fn add_item(scene: &Scene, animation: &Animation, item: Item, dom: &mut WeakDom, parent: Ref, left_out: &mut Vec<String>) -> usize {
    match item {
        Item::Rig(rig) => match model_or_bare(scene, animation, rig) {
            Ok(mut model) => {
                for top in model.root().children().to_vec() {
                    model.transfer(top, dom, parent);
                }
                1
            }
            Err(problem) => {
                left_out.push(format!("{}: {problem}", scene.rigs[rig].name));
                0
            }
        },
        Item::Folder(folder) => {
            let inside = dom.insert(parent, InstanceBuilder::new("Folder").with_name(scene.folders[folder].name.clone()));
            let folders = (0..scene.folders.len()).filter(|f| scene.folders[*f].parent == Some(folder) && !scene.folders[*f].removed);
            let rigs = (0..scene.rigs.len()).filter(|r| !scene.rigs[*r].removed && scene.rig_folder(*r) == Some(folder));
            let items: Vec<Item> = folders.map(Item::Folder).chain(rigs.map(Item::Rig)).collect();
            items.into_iter().map(|item| add_item(scene, animation, item, dom, inside, left_out)).sum()
        }
    }
}

/// A camera or shape's animations, baked relative to its root.
struct SingleMoves {
    rig: RigId,
    object: BoneId,
    /// What the object's part is called, which its Pose has to match.
    part: String,
    /// Where the object's part sits on its root with no animation playing: where the
    /// first animation starts, so it shows in place in Studio before anything plays.
    rest: Xf,
    /// The camera's field of view at rest.
    value: f32,
    clips: Vec<SingleClip>,
}

struct SingleClip {
    clip: usize,
    name: String,
    looping: bool,
    /// Relative to the root, one per frame.
    frames: Vec<Xf>,
}

fn single_moves(scene: &Scene, animation: &Animation, rig: RigId) -> Result<SingleMoves, String> {
    let object = scene.rigs[rig].bones.first().copied().filter(|b| !scene.bones[*b].removed).ok_or("There's nothing in it to send.")?;
    let root = scene.rigs[rig].placement.inverse();
    let mut animation = animation.clone();
    animation.ensure_clips(scene);
    let mut clips = Vec::new();
    let mut value = None;
    for clip in animation.clips_of(rig) {
        let mut world = scene.clone();
        let playing = animation.playing_clip(&mut world, clip);
        world.dynamics.set_baked(dynamics::bake(&world, &playing));
        let mut frames = Vec::new();
        for frame in playing.start..=playing.end.max(playing.start) {
            playing.apply(&mut world, frame as f32);
            frames.push(root * world.bones[object].frame);
            value.get_or_insert(world.bones[object].value);
        }
        let moves = frames.iter().any(|f| f.pos.distance(frames[0].pos) > STILL || f.rot.angle_between(frames[0].rot) > STILL);
        if moves {
            clips.push(SingleClip { clip, name: animation.clips[clip].name.clone(), looping: playing.looping, frames });
        }
    }
    let rest = clips.first().map_or_else(|| root * scene.bones[object].frame, |c| c.frames[0]);
    let value = if clips.is_empty() { scene.bones[object].value } else { value.unwrap_or(scene.bones[object].value) };
    let part = match scene.rigs[rig].kind {
        RigKind::Camera => CAMERA_PART.to_string(),
        _ => scene.bones[object].name.clone(),
    };
    Ok(SingleMoves { rig, object, part, rest, value, clips })
}

/// A camera or shape as a rig Studio can animate: a model holding an anchored root part
/// where its root is, the object's own part joined to it by a Motor6D, an
/// AnimationController, and its animations in AnimSaves. A camera's part is one stud
/// across, facing the way it looks, with its field of view as an attribute.
fn single_model(scene: &Scene, moves: &SingleMoves, fps: f32) -> WeakDom {
    let rig = &scene.rigs[moves.rig];
    let root_at = rig.placement;
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let top = dom.root_ref();
    // "Camera" alone would hide Workspace's own camera from scripts looking it up by name.
    let name = if rig.kind == RigKind::Camera { format!("{} Rig", rig.name) } else { rig.name.clone() };
    let model = dom.insert(top, InstanceBuilder::new("Model").with_name(name).with_property("WorldPivotData", Variant::OptionalCFrame(Some(cframe(root_at)))));
    let root = dom.insert(
        model,
        InstanceBuilder::new("Part")
            .with_name(ROOT_PART)
            .with_property("CFrame", Variant::CFrame(cframe(root_at)))
            .with_property("Size", Variant::Vector3(Vector3::new(1.0, 1.0, 1.0)))
            .with_property("Transparency", Variant::Float32(1.0))
            .with_property("Anchored", Variant::Bool(true))
            .with_property("CanCollide", Variant::Bool(false))
            .with_property("CanQuery", Variant::Bool(false))
            .with_property("CanTouch", Variant::Bool(false)),
    );
    let at = Variant::CFrame(cframe(root_at * moves.rest));
    let part = match rig.kind {
        RigKind::Camera => InstanceBuilder::new("Part")
            .with_property("Size", Variant::Vector3(Vector3::new(1.0, 1.0, 1.0)))
            .with_property("Color", Variant::Color3(CAMERA_COLOR))
            .with_property("Attributes", Variant::Attributes(Attributes::new().with(FOV_ATTRIBUTE, Variant::Float64(moves.value as f64)))),
        _ => {
            let shape = scene.bones[moves.object].part.map(|p| &scene.parts[p]);
            let (class, size, color, transparency, kind) = shape.map_or(("Part", glam::Vec3::ONE, [0.64, 0.64, 0.65], 0.0, PartShape::Block), |p| (p.class.as_str(), p.size, p.color, p.transparency, p.shape));
            let material = shape.map_or(Material::Plastic, |p| p.material);
            let locked = shape.is_some_and(|p| p.locked);
            let builder = InstanceBuilder::new(class)
                .with_property("Size", Variant::Vector3(Vector3::new(size.x, size.y, size.z)))
                .with_property("Color", Variant::Color3(Color3::new(color[0], color[1], color[2])))
                .with_property("Transparency", Variant::Float32(transparency))
                .with_property("Material", Variant::Enum(Enum::from_u32(material.roblox_value())))
                .with_property("Locked", Variant::Bool(locked));
            match kind {
                // `Enum.PartType`.
                PartShape::Ball => builder.with_property("Shape", Variant::Enum(Enum::from_u32(0))),
                PartShape::Cylinder => builder.with_property("Shape", Variant::Enum(Enum::from_u32(2))),
                _ => builder,
            }
        }
    };
    let part = dom.insert(
        model,
        part.with_name(moves.part.clone())
            .with_property("CFrame", at)
            .with_property("Anchored", Variant::Bool(false))
            .with_property("CanCollide", Variant::Bool(false))
            .with_property("Massless", Variant::Bool(true)),
    );
    let decals = scene.bones[moves.object].part.map_or(&[][..], |p| &scene.parts[p].decals[..]);
    for decal in decals {
        let [r, g, b] = decal.color;
        let mut builder = InstanceBuilder::new(if decal.tiling.is_some() { "Texture" } else { "Decal" })
            .with_property("Texture", Variant::ContentId(ContentId::from(decal.texture.clone())))
            .with_property("Face", Variant::Enum(Enum::from_u32(decal.face.roblox_value())))
            .with_property("Color3", Variant::Color3(Color3::new(r, g, b)))
            .with_property("Transparency", Variant::Float32(decal.transparency));
        if let Some(tiling) = decal.tiling {
            builder = builder
                .with_property("StudsPerTileU", Variant::Float32(tiling.studs_u))
                .with_property("StudsPerTileV", Variant::Float32(tiling.studs_v))
                .with_property("OffsetStudsU", Variant::Float32(tiling.offset_u))
                .with_property("OffsetStudsV", Variant::Float32(tiling.offset_v));
        }
        dom.insert(part, builder);
    }
    // Part1 = Part0 * C0 * Transform * C1:Inverse(), so with C0 at the rest spot an
    // animation's Transform is how far the object has moved from there.
    dom.insert(
        root,
        InstanceBuilder::new("Motor6D")
            .with_name(ROOT_JOINT)
            .with_property("C0", Variant::CFrame(cframe(moves.rest)))
            .with_property("C1", Variant::CFrame(cframe(Xf::IDENTITY)))
            .with_property("Part0", Variant::Ref(root))
            .with_property("Part1", Variant::Ref(part)),
    );
    if let Some(m) = dom.get_by_ref_mut(model) {
        m.properties.insert(ustr("PrimaryPart"), Variant::Ref(root));
    }
    let controller = dom.insert(model, InstanceBuilder::new("AnimationController").with_name("AnimationController"));
    dom.insert(controller, InstanceBuilder::new("Animator").with_name("Animator"));
    if !moves.clips.is_empty() {
        let saves = dom.insert(model, InstanceBuilder::new("ObjectValue").with_name(ANIM_SAVES));
        for clip in &moves.clips {
            let mut saved = single_sequence(moves, clip, fps);
            let sequence = saved.root().children()[0];
            saved.transfer(sequence, &mut dom, saves);
        }
    }
    dom
}

fn single_sequence(moves: &SingleMoves, clip: &SingleClip, fps: f32) -> WeakDom {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let sequence = InstanceBuilder::new("KeyframeSequence")
        .with_name(clip.name.clone())
        .with_property("Loop", Variant::Bool(clip.looping))
        .with_property("Priority", Variant::Enum(Enum::from_u32(PRIORITY_ACTION)))
        .with_property("AuthoredHipHeight", Variant::Float32(2.0));
    let sequence = dom.insert(root, sequence);
    let rest = moves.rest.inverse();
    let tree = PoseNode { name: ROOT_PART.to_string(), bone: None, children: vec![PoseNode { name: moves.part.clone(), bone: Some(0), children: Vec::new() }] };
    for (i, frame) in clip.frames.iter().enumerate() {
        let keyframe = dom.insert(sequence, InstanceBuilder::new("Keyframe").with_name("Keyframe").with_property("Time", Variant::Float32(i as f32 / fps)));
        insert_pose(&mut dom, keyframe, &tree, &[rest * *frame]);
    }
    dom
}

fn model_with(scene: &Scene, rig: RigId, sequences: Vec<WeakDom>) -> Result<WeakDom, String> {
    if scene.rigs[rig].kind.is_single() {
        return Err("A camera or shape is built by single_model.".to_string());
    }
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
    let bytes = to_bytes(dom)?;
    let temporary = path.with_added_extension("tmp");
    let written = std::fs::write(&temporary, &bytes).and_then(|()| std::fs::rename(&temporary, path));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    Ok(())
}

fn to_bytes(dom: &WeakDom) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    rbx_binary::to_writer(&mut bytes, dom, dom.root().children()).map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// The path with `.rbxm` on the end, for a name typed without it.
pub fn with_extension(path: std::path::PathBuf) -> std::path::PathBuf {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION)) { path } else { path.with_added_extension(EXTENSION) }
}
