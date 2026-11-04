//! Animations written as a Roblox KeyframeSequence. Files are read back and played through
//! plain Roblox joint math, with nothing of Kaye's solving them.

use std::collections::HashMap;
use std::path::Path;

use glam::{Quat, Vec3};
use kaye::animation::Animation;
use kaye::export;
use kaye::import::import_file;
use kaye::math::Xf;
use kaye::modifiers;
use kaye::scene::{BoneId, BoneKind, BoneLink, Scene};
use rbx_dom_weak::{Instance, WeakDom};
use rbx_types::Variant;
use ustr::ustr;

fn sample(name: &str) -> Scene {
    import_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(name)).unwrap().scene
}

fn bone_named(scene: &Scene, name: &str) -> BoneId {
    scene.bones.iter().position(|b| b.name == name).unwrap_or_else(|| panic!("no bone {name}"))
}

fn write_and_read(scene: &Scene, animation: &Animation) -> WeakDom {
    let path = std::env::temp_dir().join(format!("kaye-export-{}-{:?}.rbxm", std::process::id(), std::thread::current().id()));
    export::write_animation(&path, scene, animation, 0).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).ok();
    rbx_binary::from_reader(&bytes[..]).unwrap()
}

fn children<'a>(dom: &'a WeakDom, inst: &'a Instance) -> impl Iterator<Item = &'a Instance> {
    inst.children().iter().filter_map(|c| dom.get_by_ref(*c))
}

fn sequence(dom: &WeakDom) -> &Instance {
    let mut top = children(dom, dom.root());
    let found = top.next().unwrap();
    assert!(top.next().is_none(), "one thing at the top of the file");
    assert_eq!(found.class, "KeyframeSequence");
    found
}

fn keyframes<'a>(dom: &'a WeakDom, sequence: &'a Instance) -> Vec<&'a Instance> {
    let mut frames: Vec<&Instance> = children(dom, sequence).collect();
    frames.iter().for_each(|f| assert_eq!(f.class, "Keyframe"));
    frames.sort_by(|a, b| time_of(a).total_cmp(&time_of(b)));
    frames
}

fn time_of(keyframe: &Instance) -> f32 {
    match keyframe.properties.get(&ustr("Time")) {
        Some(Variant::Float32(t)) => *t,
        other => panic!("no time: {other:?}"),
    }
}

fn weight_of(pose: &Instance) -> f32 {
    match pose.properties.get(&ustr("Weight")) {
        Some(Variant::Float32(w)) => *w,
        other => panic!("no weight: {other:?}"),
    }
}

fn cframe_of(pose: &Instance) -> Xf {
    match pose.properties.get(&ustr("CFrame")) {
        Some(Variant::CFrame(c)) => Xf::from_cframe(c),
        other => panic!("no cframe: {other:?}"),
    }
}

/// Every pose in a keyframe: its transform, and the name of the pose it's nested under.
fn poses_of(dom: &WeakDom, keyframe: &Instance) -> HashMap<String, (Xf, f32, String)> {
    fn walk(dom: &WeakDom, pose: &Instance, parent: &str, out: &mut HashMap<String, (Xf, f32, String)>) {
        assert_eq!(pose.class, "Pose");
        out.insert(pose.name.clone(), (cframe_of(pose), weight_of(pose), parent.to_string()));
        children(dom, pose).for_each(|c| walk(dom, c, &pose.name, out));
    }
    let mut out = HashMap::new();
    children(dom, keyframe).for_each(|p| walk(dom, p, "", &mut out));
    out
}

/// Plays one keyframe's poses on a bare copy of the rig using only Roblox's own formula,
/// `Part1 = Part0 * C0 * Transform * C1:Inverse()`, and returns where each part lands.
fn play(scene: &Scene, poses: &HashMap<String, (Xf, f32, String)>) -> Vec<Xf> {
    let mut world: Vec<Xf> = scene.parts.iter().map(|p| p.file_cframe).collect();
    for &part in &scene.part_order {
        if let kaye::scene::Drive::Bone(bone) = scene.parts[part].drive {
            let BoneLink::Joint { part0, part1, c0, c1 } = scene.bones[bone].link else {
                continue;
            };
            let transform = poses.get(&scene.parts[part1].name).map_or(Xf::IDENTITY, |p| p.0);
            world[part1] = world[part0] * c0 * transform * c1.inverse();
        }
    }
    world
}

fn waving(scene: &Scene, bone: BoneId) -> Animation {
    let mut animation = Animation { end: 20, ..Animation::default() };
    animation.set_key(bone, 0, Xf::IDENTITY);
    animation.set_key(bone, 10, Xf::new(Quat::from_rotation_x(1.2), Vec3::new(0.0, 0.2, 0.0)));
    animation.set_key(bone, 20, Xf::IDENTITY);
    assert!(scene.bones[bone].kind.is_joint());
    animation
}

#[test]
fn the_file_holds_one_keyframe_per_frame_at_the_right_times() {
    let scene = sample("R15Rig.rbxm");
    let animation = Animation { fps: 30.0, ..waving(&scene, bone_named(&scene, "RightShoulder")) };
    let dom = write_and_read(&scene, &animation);
    let sequence = sequence(&dom);
    assert_eq!(sequence.properties.get(&ustr("Loop")), Some(&Variant::Bool(true)));
    let frames = keyframes(&dom, sequence);
    assert_eq!(frames.len(), 21);
    assert!((time_of(frames[0])).abs() < 1e-6);
    assert!((time_of(frames[15]) - 0.5).abs() < 1e-5);
    assert!((time_of(frames[20]) - 20.0 / 30.0).abs() < 1e-5);
}

#[test]
fn time_starts_at_zero_even_when_the_animation_does_not() {
    let scene = sample("R15Rig.rbxm");
    let bone = bone_named(&scene, "RightShoulder");
    let mut animation = Animation { start: 10, end: 20, ..Animation::default() };
    animation.set_key(bone, 10, Xf::new(Quat::from_rotation_x(0.5), Vec3::ZERO));
    let dom = write_and_read(&scene, &animation);
    let frames = keyframes(&dom, sequence(&dom));
    assert_eq!(frames.len(), 11);
    assert_eq!(time_of(frames[0]), 0.0);
}

#[test]
fn poses_nest_the_way_the_rig_hangs_together() {
    let scene = sample("R15Rig.rbxm");
    let dom = write_and_read(&scene, &waving(&scene, bone_named(&scene, "RightWrist")));
    let frames = keyframes(&dom, sequence(&dom));
    let poses = poses_of(&dom, frames[0]);
    let parent = |name: &str| poses[name].2.as_str();
    assert_eq!(parent("HumanoidRootPart"), "");
    assert_eq!(parent("LowerTorso"), "HumanoidRootPart");
    assert_eq!(parent("UpperTorso"), "LowerTorso");
    assert_eq!(parent("RightUpperArm"), "UpperTorso");
    assert_eq!(parent("RightLowerArm"), "RightUpperArm");
    assert_eq!(parent("RightHand"), "RightLowerArm");
    assert_eq!(poses["RightHand"].1, 1.0, "what moves counts");
    assert_eq!(poses["UpperTorso"].1, 0.0, "what's only on the way there doesn't");
    assert_eq!(poses["HumanoidRootPart"].1, 0.0);
    assert!(!poses.contains_key("LeftUpperArm"), "still joints stay out, so other animations can use them");
}

#[test]
fn r6_nests_under_the_root_too() {
    let scene = sample("R6Rig.rbxm");
    let dom = write_and_read(&scene, &waving(&scene, bone_named(&scene, "Right Shoulder")));
    let frames = keyframes(&dom, sequence(&dom));
    let poses = poses_of(&dom, frames[0]);
    assert_eq!(poses["Torso"].2, "HumanoidRootPart");
    assert_eq!(poses["Right Arm"].2, "Torso");
    assert_eq!(poses["Right Arm"].1, 1.0);
}

fn assert_plays_as_drawn(scene: &Scene, animation: &Animation) {
    let dom = write_and_read(scene, animation);
    let frames = keyframes(&dom, sequence(&dom));
    let mut drawn = scene.clone();
    for (i, keyframe) in frames.iter().enumerate() {
        animation.apply(&mut drawn, (animation.start + i as i32) as f32);
        let played = play(&drawn, &poses_of(&dom, keyframe));
        for (part, world) in played.iter().enumerate() {
            let expected = drawn.parts[part].world;
            assert!(world.pos.distance(expected.pos) < 2e-3, "{} on frame {i}: {} against {}", drawn.parts[part].name, world.pos, expected.pos);
            assert!(world.rot.angle_between(expected.rot) < 2e-3, "{} on frame {i}", drawn.parts[part].name);
        }
    }
}

#[test]
fn keys_play_back_in_roblox_exactly_where_they_were_drawn() {
    let scene = sample("R15Rig.rbxm");
    let mut animation = waving(&scene, bone_named(&scene, "RightShoulder"));
    animation.set_key(bone_named(&scene, "Waist"), 5, Xf::new(Quat::from_rotation_y(0.4), Vec3::ZERO));
    assert_plays_as_drawn(&scene, &animation);
}

#[test]
fn r6_keys_play_back_too() {
    let scene = sample("R6Rig.rbxm");
    assert_plays_as_drawn(&scene, &waving(&scene, bone_named(&scene, "Left Hip")));
}

#[test]
fn ik_plays_back_with_nothing_solving_it() {
    let mut scene = sample("R15Rig.rbxm");
    let foot = bone_named(&scene, "RightAnkle");
    let target = modifiers::add_ik(&mut scene, foot);
    let mut animation = Animation { end: 10, ..Animation::default() };
    animation.set_key(target, 0, Xf::IDENTITY);
    animation.set_key(target, 10, Xf::new(Quat::IDENTITY, Vec3::new(0.3, 0.6, -0.4)));
    assert_plays_as_drawn(&scene, &animation);
    let dom = write_and_read(&scene, &animation);
    let names: Vec<String> = poses_of(&dom, keyframes(&dom, sequence(&dom))[10]).into_keys().collect();
    assert!(names.iter().any(|n| n == "RightUpperLeg"), "the chain is in the file: {names:?}");
    assert!(!names.iter().any(|n| n.contains("Target")), "the target isn't: {names:?}");
}

#[test]
fn layers_come_down_to_one_pose() {
    let scene = sample("R15Rig.rbxm");
    let bone = bone_named(&scene, "RightShoulder");
    let mut animation = waving(&scene, bone);
    animation.add_layer();
    animation.set_key(bone, 5, Xf::new(Quat::from_rotation_z(0.3), Vec3::ZERO));
    assert_plays_as_drawn(&scene, &animation);
}

#[test]
fn only_real_joints_go_in_the_file() {
    let mut scene = sample("R15Rig.rbxm");
    let camera = scene.add_control("Camera".to_string(), BoneKind::Camera, 0, Xf::IDENTITY, 70.0);
    let bone = bone_named(&scene, "RightShoulder");
    let mut animation = waving(&scene, bone);
    animation.set_key(camera, 3, Xf::new(Quat::IDENTITY, Vec3::X));
    let dom = write_and_read(&scene, &animation);
    let poses = poses_of(&dom, keyframes(&dom, sequence(&dom))[3]);
    assert!(!poses.contains_key("Camera"));
}

#[test]
fn an_animation_with_nothing_moving_says_so() {
    let scene = sample("R15Rig.rbxm");
    let path = std::env::temp_dir().join(format!("kaye-export-empty-{}.rbxm", std::process::id()));
    let result = export::write_animation(&path, &scene, &Animation::default(), 0);
    assert!(result.is_err());
    assert!(!path.exists(), "no file left behind");
}

#[test]
fn a_rig_with_no_joints_says_so() {
    let mut scene = sample("R15Rig.rbxm");
    scene.rigs.push(kaye::scene::Rig { name: "Empty".to_string(), kind: kaye::scene::RigKind::Static, source_file: String::new(), parts: Vec::new(), bones: Vec::new(), visible: true, shirt: None, pants: None });
    let rig = scene.rigs.len() - 1;
    assert!(export::keyframe_sequence(&scene, &Animation::default(), rig).is_err());
}

#[test]
fn a_typed_name_gets_the_extension() {
    assert_eq!(export::with_extension("walk".into()), std::path::PathBuf::from("walk.rbxm"));
    assert_eq!(export::with_extension("walk.RBXM".into()), std::path::PathBuf::from("walk.RBXM"));
}
