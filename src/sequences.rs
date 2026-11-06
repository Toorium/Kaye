//! Reads KeyframeSequences, which is how Roblox's Animation Editor saves animations, into
//! Kaye's own. This is `export.rs` the other way around, and like it has no interface code.
//!
//! A Keyframe holds a tree of Poses. A Pose's CFrame is a joint's `Transform`, which is what
//! `Bone::pose` stores, and it finds its joint by the name of the part or bone it moves,
//! nested under the Pose of whatever that hangs from.
//!
//! Reading a sequence out of a file needs nothing from the scene. Putting it on a rig comes
//! after, in `load`, since a file of nothing but sequences goes on a rig that's already open.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rbx_dom_weak::{Instance, WeakDom};

use crate::animation::{Animation, CHANNELS, EaseDirection, EaseStyle, Easing, FrameNumber, Key, Track};
use crate::export::pose_name;
use crate::import::{bool_prop, cframe_prop, enum_prop, float_prop};
use crate::math::Xf;
use crate::scene::{BoneId, BoneLink, RigId, Scene};

/// `Enum.PoseEasingStyle`.
const STYLE_CONSTANT: u32 = 1;
const STYLE_ELASTIC: u32 = 2;
const STYLE_CUBIC: u32 = 3;
const STYLE_BOUNCE: u32 = 4;
const STYLE_CUBIC_V2: u32 = 5;
/// `Enum.PoseEasingDirection`.
const DIRECTION_IN: u32 = 0;
const DIRECTION_OUT: u32 = 1;
const DIRECTION_IN_OUT: u32 = 2;
/// The frame rates Roblox animators work in. An empty project takes the first one that puts
/// every key on a whole frame.
const RATES: [f32; 4] = [24.0, 30.0, 60.0, 120.0];
/// How far off a whole frame a key can be and still count as on it, in frames.
const ON_A_FRAME: f32 = 0.02;

/// One Pose that moves something, taken out of its tree.
#[derive(Clone, Debug)]
pub struct PoseRow {
    pub name: String,
    /// The name of the Pose it's nested under.
    pub parent: Option<String>,
    pub pose: Xf,
    style: u32,
    direction: u32,
}

#[derive(Clone, Debug)]
pub struct Keyframe {
    /// In seconds, since the frame rate belongs to the project.
    pub time: f32,
    pub poses: Vec<PoseRow>,
}

#[derive(Clone, Debug)]
pub struct Sequence {
    pub name: String,
    pub looping: bool,
    pub keyframes: Vec<Keyframe>,
}

/// A sequence in a file, and where it was found.
#[derive(Clone, Debug)]
pub struct Found {
    /// The rig it was inside, counting from the file's first. A sequence outside every rig
    /// has `None`, and goes on whichever rig it fits best.
    pub rig: Option<RigId>,
    pub sequence: Sequence,
}

fn children<'a>(dom: &'a WeakDom, instance: &'a Instance) -> impl Iterator<Item = &'a Instance> {
    instance.children().iter().filter_map(|child| dom.get_by_ref(*child))
}

/// A Pose with no weight is only there to reach what's under it, and Roblox doesn't apply it.
fn read_pose(dom: &WeakDom, pose: &Instance, parent: Option<&str>, into: &mut Vec<PoseRow>) {
    if float_prop(pose, "Weight").unwrap_or(1.0) > 0.0 {
        into.push(PoseRow {
            name: pose.name.clone(),
            parent: parent.map(str::to_owned),
            pose: cframe_prop(pose, "CFrame").unwrap_or(Xf::IDENTITY),
            style: enum_prop(pose, "EasingStyle").unwrap_or(0),
            direction: enum_prop(pose, "EasingDirection").unwrap_or(DIRECTION_IN),
        });
    }
    for child in children(dom, pose).filter(|c| c.class == "Pose") {
        read_pose(dom, child, Some(pose.name.as_str()), into);
    }
}

pub fn read(dom: &WeakDom, sequence: &Instance) -> Sequence {
    let keyframes = children(dom, sequence).filter(|c| c.class == "Keyframe").map(|keyframe| {
        let mut poses = Vec::new();
        for pose in children(dom, keyframe).filter(|c| c.class == "Pose") {
            read_pose(dom, pose, None, &mut poses);
        }
        Keyframe { time: float_prop(keyframe, "Time").unwrap_or(0.0).max(0.0), poses }
    });
    Sequence { name: sequence.name.clone(), looping: bool_prop(sequence, "Loop").unwrap_or(true), keyframes: keyframes.collect() }
}

/// What Roblox really does with a Pose's easing, measured in Studio. Its Cubic runs the
/// opposite way to its name, which CubicV2 put right. Nothing else does, including Bounce
/// and Elastic. InOut on those two is a little different from Kaye's, which is left as it is.
fn easing_of(style: u32, direction: u32) -> Easing {
    let (kind, flipped) = match style {
        STYLE_CONSTANT => (EaseStyle::Constant, false),
        STYLE_ELASTIC => (EaseStyle::Elastic, false),
        STYLE_CUBIC => (EaseStyle::Cubic, true),
        STYLE_BOUNCE => (EaseStyle::Bounce, false),
        STYLE_CUBIC_V2 => (EaseStyle::Cubic, false),
        _ => (EaseStyle::Linear, false),
    };
    let direction = match (direction, flipped) {
        (DIRECTION_IN, false) | (DIRECTION_OUT, true) => EaseDirection::In,
        (DIRECTION_OUT, false) | (DIRECTION_IN, true) => EaseDirection::Out,
        _ => EaseDirection::InOut,
    };
    Easing { style: kind, direction, ..Easing::default() }
}

/// The frame a Constant key lets go on, which Roblox decides by its direction: In jumps to
/// the next pose right away, InOut halfway there, and Out holds to the end.
fn jump_frame(direction: u32, frame: FrameNumber, next: FrameNumber) -> FrameNumber {
    match direction {
        DIRECTION_IN => frame + 1,
        DIRECTION_IN_OUT => (frame + next + 1) / 2,
        _ => next,
    }
}

fn key(frame: FrameNumber, pose: Xf, easing: Easing) -> Key {
    Key { frame, pose, value: 1.0, easing, handles: [None; CHANNELS], channels: crate::animation::ALL_CHANNELS }
}

struct Step {
    frame: FrameNumber,
    pose: Xf,
    style: u32,
    direction: u32,
}

/// A bone's keys. Kaye's Constant always holds to the end, so one that lets go sooner gets
/// a key of its own there.
fn track_of(steps: &[Step]) -> Track {
    let mut track = Track::default();
    for (index, step) in steps.iter().enumerate() {
        let easing = easing_of(step.style, step.direction);
        track.keys.push(key(step.frame, step.pose, easing));
        if let Some(next) = steps.get(index + 1).filter(|_| step.style == STYLE_CONSTANT) {
            let jump = jump_frame(step.direction, step.frame, next.frame);
            if jump < next.frame {
                track.keys.push(key(jump, next.pose, easing));
            }
        }
    }
    track
}

/// How a rig's joints are found from the names on its Poses.
struct Joints {
    /// By the name of what a joint hangs from, then the name of what it moves.
    nested: HashMap<(String, String), BoneId>,
    named: HashMap<String, BoneId>,
    parts: BTreeSet<String>,
}

impl Joints {
    fn of(scene: &Scene, rig: RigId) -> Self {
        let mut joints = Self { nested: HashMap::new(), named: HashMap::new(), parts: BTreeSet::new() };
        joints.parts.extend(scene.rigs[rig].parts.iter().map(|part| scene.parts[*part].name.clone()));
        for bone in scene.joints(rig) {
            let name = pose_name(scene, bone);
            let parent = match scene.bones[bone].link {
                BoneLink::Joint { part0, .. } => Some(scene.parts[part0].name.clone()),
                BoneLink::Skinned { parent_bone: Some(parent), .. } => Some(scene.bones[parent].name.clone()),
                BoneLink::Skinned { parent_part, .. } => parent_part.map(|part| scene.parts[part].name.clone()),
                BoneLink::Free { .. } => None,
            };
            if let Some(parent) = parent {
                joints.nested.entry((parent, name.clone())).or_insert(bone);
            }
            joints.named.entry(name).or_insert(bone);
        }
        joints
    }

    fn find(&self, pose: &PoseRow) -> Option<BoneId> {
        let nested = pose.parent.as_ref().and_then(|parent| self.nested.get(&(parent.clone(), pose.name.clone())));
        nested.or_else(|| self.named.get(&pose.name)).copied()
    }
}

/// A sequence's keys on a rig.
struct Placed {
    tracks: BTreeMap<BoneId, Track>,
    end: FrameNumber,
    /// How many of the parts it moves this rig doesn't have.
    unknown: usize,
}

fn place(scene: &Scene, rig: RigId, sequence: &Sequence, fps: f32) -> Placed {
    let joints = Joints::of(scene, rig);
    let mut keyframes: Vec<&Keyframe> = sequence.keyframes.iter().collect();
    keyframes.sort_by(|a, b| a.time.total_cmp(&b.time));
    let mut steps: BTreeMap<BoneId, Vec<Step>> = BTreeMap::new();
    let mut unknown = BTreeSet::new();
    // A range with no length has nowhere to put the playhead.
    let mut end = 1;
    for keyframe in keyframes {
        let frame = (keyframe.time * fps).round() as FrameNumber;
        end = end.max(frame);
        for pose in &keyframe.poses {
            let Some(bone) = joints.find(pose) else {
                // HumanoidRootPart has a Pose and no joint.
                if !joints.parts.contains(&pose.name) {
                    unknown.insert(pose.name.as_str());
                }
                continue;
            };
            let steps = steps.entry(bone).or_default();
            // Keys that round to one frame are as close as Kaye can get, and the later wins.
            if steps.last().is_some_and(|last| last.frame == frame) {
                steps.pop();
            }
            steps.push(Step { frame, pose: pose.pose, style: pose.style, direction: pose.direction });
        }
    }
    Placed { tracks: steps.iter().map(|(bone, steps)| (*bone, track_of(steps))).collect(), end, unknown: unknown.len() }
}

/// The rig a loose sequence belongs on: the one whose joints it names the most of, looking
/// at what came in the same file first. Ties go to the rig in focus, then the earliest.
fn best_rig(scene: &Scene, sequence: &Sequence, first_new_rig: RigId, focus: Option<RigId>) -> Option<RigId> {
    let names: Vec<&PoseRow> = sequence.keyframes.iter().flat_map(|k| &k.poses).collect();
    let came_with_it = first_new_rig < scene.rigs.len();
    let rigs: Vec<RigId> = if came_with_it { (first_new_rig..scene.rigs.len()).collect() } else { focus.into_iter().chain(0..scene.rigs.len()).collect() };
    let mut best = None;
    let mut most = 0;
    for rig in rigs.into_iter().filter(|r| !scene.rigs[*r].removed) {
        let joints = Joints::of(scene, rig);
        let found: BTreeSet<BoneId> = names.iter().filter_map(|pose| joints.find(pose)).collect();
        if found.len() > most {
            most = found.len();
            best = Some(rig);
        }
    }
    best
}

fn fitting_rate(found: &[Found], current: f32) -> f32 {
    let fits = |rate: f32| {
        found.iter().flat_map(|f| &f.sequence.keyframes).all(|keyframe| {
            let at = keyframe.time * rate;
            (at - at.round()).abs() < ON_A_FRAME
        })
    };
    std::iter::once(current).chain(RATES).find(|rate| fits(*rate)).unwrap_or(current)
}

/// Puts everything found in a file on its rig, and returns what the user should know about.
/// A rig that just came in starts out playing its first animation. One already open switches
/// to what was brought in for it, and drops the empty animation every rig starts with.
pub fn load(animation: &mut Animation, scene: &mut Scene, found: Vec<Found>, first_new_rig: RigId, focus: Option<RigId>) -> Vec<String> {
    let mut warnings = Vec::new();
    // A project has one frame rate, and one with no keys yet can still take this file's.
    if animation.keyed_bones().is_empty() {
        animation.fps = fitting_rate(&found, animation.fps);
    }
    let mut switched = BTreeSet::new();
    for Found { rig, sequence } in found {
        let name = &sequence.name;
        let Some(rig) = rig.map(|r| r + first_new_rig).or_else(|| best_rig(scene, &sequence, first_new_rig, focus)) else {
            warnings.push(format!("\"{name}\" doesn't fit any rig here, so it was skipped."));
            continue;
        };
        let placed = place(scene, rig, &sequence, animation.fps);
        let rig_name = &scene.rigs[rig].name;
        if placed.tracks.is_empty() {
            warnings.push(format!("\"{name}\" doesn't move anything on {rig_name}, so it was skipped."));
            continue;
        }
        if placed.unknown > 0 {
            warnings.push(format!("\"{name}\" moves {} parts {rig_name} doesn't have.", placed.unknown));
        }
        let clip = animation.add_keyed_clip(rig, name, placed.end, sequence.looping, placed.tracks);
        if rig < first_new_rig && switched.insert(rig) {
            let old = animation.active_clip(rig);
            animation.switch_clip(scene, clip);
            animation.rest_unkeyed(scene, rig);
            if let Some(old) = old.filter(|old| animation.clips_of(rig).len() == 2 && animation.clips[*old].tracks.is_empty()) {
                animation.remove_clip(scene, old);
            }
        }
    }
    warnings
}
