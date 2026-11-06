//! Individual Animation Properties: with it on, each animation keeps its own frame rate and
//! modifiers, and switching to it brings them back. It's off by default, and then
//! animations share them as before. Start and end are each animation's own either way.

use std::path::Path;

use glam::{Quat, Vec3};
use kaye::animation::Animation;
use kaye::export;
use kaye::import::import_file;
use kaye::math::Xf;
use kaye::modifiers::{self, ModifierKind};
use kaye::project::{self, View};
use kaye::scene::{BoneId, Scene};
use rbx_dom_weak::{Instance, WeakDom};
use rbx_types::Variant;
use ustr::ustr;

fn sample(name: &str) -> Scene {
    let mut scene = import_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(name)).unwrap().scene;
    scene.evaluate();
    scene
}

fn bone_named(scene: &Scene, rig: usize, name: &str) -> BoneId {
    scene.live_bones(rig).into_iter().find(|b| scene.bones[*b].name == name).unwrap_or_else(|| panic!("no bone {name}"))
}

fn has_ik(scene: &Scene, bone: BoneId) -> bool {
    scene.modifiers.iter().any(|m| m.bone == bone && matches!(m.kind, ModifierKind::Ik(_)))
}

fn has_look_at(scene: &Scene, bone: BoneId) -> bool {
    scene.modifiers.iter().any(|m| m.bone == bone && matches!(m.kind, ModifierKind::LookAt(_)))
}

fn view() -> View {
    View { frame: 0.0, target: Vec3::ZERO, yaw: 0.0, pitch: 0.0, dist: 10.0, ortho: false, floor_y: 0.0, camera: None, preview: false, through: false, motion_paths: Vec::new(), references: Vec::new() }
}

fn temp_file(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("kaye-individual-{}-{:?}-{name}", std::process::id(), std::thread::current().id()))
}

/// Plays another of the rig's animations, and shows it, as picking it in the list does.
fn go_to(a: &mut Animation, scene: &mut Scene, clip: usize) {
    a.switch_clip(scene, clip);
    let rig = a.clips[clip].rig;
    a.rest_unkeyed(scene, rig);
    a.show_clip(clip);
    a.apply(scene, 0.0);
}

/// The R15 sample's first animation, called Walk and showing, 0 to 30 at 30 fps.
fn walk(scene: &Scene, separate: bool) -> (Animation, usize) {
    let mut a = Animation { end: 30, ..Animation::default() };
    a.ensure_clips(scene);
    let walk = a.active_clip(0).unwrap();
    a.clips[walk].name = "Walk".to_string();
    a.show_clip(walk);
    a.set_separate(scene, separate);
    (a, walk)
}

fn new_animation(a: &mut Animation, scene: &mut Scene, name: &str) -> usize {
    let clip = a.add_clip(0);
    a.clips[clip].name = name.to_string();
    go_to(a, scene, clip);
    clip
}

#[test]
fn off_by_default_so_animations_share_frame_rate_and_modifiers() {
    let mut scene = sample("R15Rig.rbxm");
    let foot = bone_named(&scene, 0, "RightAnkle");
    assert!(!Animation::default().separate);
    let (mut a, walk) = walk(&scene, false);
    let idle = new_animation(&mut a, &mut scene, "Idle");
    modifiers::add_ik(&mut scene, foot);
    a.fps = 60.0;
    a.end = 90;
    go_to(&mut a, &mut scene, walk);
    assert_eq!(a.fps, 60.0, "one frame rate");
    assert!(has_ik(&scene, foot), "one set of modifiers");
    assert_eq!(a.end, 30, "but each animation still has its own range");
    go_to(&mut a, &mut scene, idle);
    assert_eq!(a.end, 90);
}

#[test]
fn a_new_animation_starts_as_a_copy_of_the_one_showing() {
    let mut scene = sample("R15Rig.rbxm");
    let foot = bone_named(&scene, 0, "RightAnkle");
    let (mut a, _) = walk(&scene, true);
    let target = modifiers::add_ik(&mut scene, foot);
    a.set_key(target, 0, Xf::IDENTITY);
    a.fps = 60.0;
    a.end = 45;
    new_animation(&mut a, &mut scene, "Run");
    assert_eq!((a.fps, a.end), (60.0, 45));
    assert!(has_ik(&scene, foot) && !scene.bones[target].removed, "it has the same IK");
    assert!(a.key_at(target, 0).is_none(), "but no keys");
}

#[test]
fn each_animation_keeps_its_own_frame_rate() {
    let mut scene = sample("R15Rig.rbxm");
    let (mut a, walk) = walk(&scene, true);
    let idle = new_animation(&mut a, &mut scene, "Idle");
    a.fps = 24.0;
    go_to(&mut a, &mut scene, walk);
    assert_eq!(a.fps, 30.0);
    a.fps = 60.0;
    go_to(&mut a, &mut scene, idle);
    assert_eq!(a.fps, 24.0);
    go_to(&mut a, &mut scene, walk);
    assert_eq!(a.fps, 60.0);
}

#[test]
fn each_animation_keeps_its_own_modifiers() {
    let mut scene = sample("R15Rig.rbxm");
    let (foot, neck) = (bone_named(&scene, 0, "RightAnkle"), bone_named(&scene, 0, "Neck"));
    let (mut a, walk) = walk(&scene, true);
    let idle = new_animation(&mut a, &mut scene, "Idle");
    go_to(&mut a, &mut scene, walk);
    let target = modifiers::add_ik(&mut scene, foot);
    modifiers::set_follow(&mut scene, target, Some(neck));
    let follows = scene.bones[target].link;
    a.set_key(target, 0, Xf::IDENTITY);
    a.set_key(target, 10, Xf::new(Quat::IDENTITY, Vec3::new(0.3, 0.6, -0.4)));

    go_to(&mut a, &mut scene, idle);
    assert!(!has_ik(&scene, foot), "Idle never had IK");
    assert!(scene.bones[target].removed, "so its target is hidden");
    let look = modifiers::add_look_at(&mut scene, neck);

    go_to(&mut a, &mut scene, walk);
    assert!(has_ik(&scene, foot) && !scene.bones[target].removed, "Walk's IK came back");
    assert_eq!(scene.bones[target].link, follows, "following what it followed");
    assert!(a.key_at(target, 10).is_some(), "with its keys");
    assert!(!has_look_at(&scene, neck) && scene.bones[look].removed, "Idle's Look At stays with Idle");

    go_to(&mut a, &mut scene, idle);
    assert!(has_look_at(&scene, neck) && !scene.bones[look].removed);
    assert!(!has_ik(&scene, foot));
}

#[test]
fn removing_a_modifier_only_takes_it_from_that_animation() {
    let mut scene = sample("R15Rig.rbxm");
    let foot = bone_named(&scene, 0, "RightAnkle");
    let (mut a, walk) = walk(&scene, true);
    let target = modifiers::add_ik(&mut scene, foot);
    a.set_key(target, 0, Xf::IDENTITY);
    new_animation(&mut a, &mut scene, "Idle");
    a.set_key(target, 5, Xf::new(Quat::IDENTITY, Vec3::Y));
    let index = scene.modifiers.iter().position(|m| m.bone == foot).unwrap();
    for gone in modifiers::remove(&mut scene, index) {
        a.forget_gone(&scene, gone);
    }
    assert!(a.key_at(target, 5).is_none());

    go_to(&mut a, &mut scene, walk);
    assert!(has_ik(&scene, foot) && !scene.bones[target].removed, "Walk still has it");
    assert!(a.key_at(target, 0).is_some(), "and its keys");
}

#[test]
fn deleting_a_bone_takes_its_modifiers_from_every_animation() {
    let mut scene = sample("R15Rig.rbxm");
    let foot = bone_named(&scene, 0, "RightAnkle");
    let (mut a, walk) = walk(&scene, true);
    let target = modifiers::add_ik(&mut scene, foot);
    a.set_key(target, 0, Xf::IDENTITY);
    new_animation(&mut a, &mut scene, "Idle");
    a.set_key(target, 5, Xf::IDENTITY);
    for gone in modifiers::delete_bones(&mut scene, &[foot]) {
        a.forget_gone(&scene, gone);
    }
    go_to(&mut a, &mut scene, walk);
    assert!(!scene.modifiers.iter().any(|m| m.bone == foot), "Walk's IK went with the foot");
    assert!(scene.bones[target].removed, "and its target didn't come back");
    assert!(!a.keyed_bones().contains(&target), "nor did any of its keys stay behind");
}

#[test]
fn turning_it_on_starts_every_animation_from_what_is_there() {
    let mut scene = sample("R15Rig.rbxm");
    let foot = bone_named(&scene, 0, "RightAnkle");
    let (mut a, walk) = walk(&scene, false);
    let idle = new_animation(&mut a, &mut scene, "Idle");
    modifiers::add_ik(&mut scene, foot);
    a.fps = 60.0;
    a.set_separate(&scene, true);
    go_to(&mut a, &mut scene, walk);
    assert_eq!(a.fps, 60.0);
    assert!(has_ik(&scene, foot));
    a.fps = 24.0;
    go_to(&mut a, &mut scene, idle);
    assert_eq!(a.fps, 60.0, "and from then on they go their own ways");
}

#[test]
fn turning_it_off_shares_what_is_showing() {
    let mut scene = sample("R15Rig.rbxm");
    let foot = bone_named(&scene, 0, "RightAnkle");
    let (mut a, walk) = walk(&scene, true);
    let idle = new_animation(&mut a, &mut scene, "Idle");
    go_to(&mut a, &mut scene, walk);
    modifiers::add_ik(&mut scene, foot);
    a.fps = 60.0;
    go_to(&mut a, &mut scene, idle);
    a.set_separate(&scene, false);
    assert!(a.clips.iter().all(|c| c.fps.is_none() && c.setup.is_none()));
    go_to(&mut a, &mut scene, walk);
    assert_eq!(a.fps, 30.0, "Idle's frame rate is everyone's now");
    assert!(!has_ik(&scene, foot), "and so are its modifiers");
}

fn sequences(dom: &WeakDom) -> Vec<&Instance> {
    dom.root().children().iter().filter_map(|c| dom.get_by_ref(*c)).collect()
}

fn named<'a>(dom: &'a WeakDom, name: &str) -> &'a Instance {
    sequences(dom).into_iter().find(|s| s.name == name).unwrap_or_else(|| panic!("no {name}"))
}

fn last_time(dom: &WeakDom, sequence: &Instance) -> f32 {
    let keyframes = sequence.children().iter().filter_map(|c| dom.get_by_ref(*c));
    keyframes
        .map(|k| match k.properties.get(&ustr("Time")) {
            Some(Variant::Float32(t)) => *t,
            other => panic!("a keyframe's Time is {other:?}"),
        })
        .fold(0.0, f32::max)
}

fn pose_names(dom: &WeakDom, inst: &Instance, into: &mut Vec<String>) {
    for child in inst.children().iter().filter_map(|c| dom.get_by_ref(*c)) {
        into.push(child.name.clone());
        pose_names(dom, child, into);
    }
}

#[test]
fn an_animation_exports_with_its_own_frame_rate_and_modifiers_while_another_plays() {
    let mut scene = sample("R15Rig.rbxm");
    let (foot, arm) = (bone_named(&scene, 0, "RightAnkle"), bone_named(&scene, 0, "LeftShoulder"));
    let (mut a, _) = walk(&scene, true);
    let target = modifiers::add_ik(&mut scene, foot);
    a.set_key(target, 0, Xf::IDENTITY);
    a.set_key(target, 30, Xf::new(Quat::IDENTITY, Vec3::new(0.3, 0.6, -0.4)));
    a.fps = 60.0;
    let idle = new_animation(&mut a, &mut scene, "Idle");
    let index = scene.modifiers.iter().position(|m| m.bone == foot).unwrap();
    for gone in modifiers::remove(&mut scene, index) {
        a.forget_gone(&scene, gone);
    }
    a.fps = 30.0;
    a.set_key(arm, 0, Xf::IDENTITY);
    a.set_key(arm, 30, Xf::new(Quat::from_rotation_x(1.0), Vec3::ZERO));
    assert_eq!(a.active_clip(0), Some(idle), "Walk is waiting, not playing");

    let dom = export::all_keyframe_sequences(&scene, &a, 0).unwrap();
    let (walk_out, idle_out) = (named(&dom, "Walk"), named(&dom, "Idle"));
    assert!((last_time(&dom, walk_out) - 0.5).abs() < 1e-4, "30 frames at 60 fps");
    assert!((last_time(&dom, idle_out) - 1.0).abs() < 1e-4, "30 frames at 30 fps");
    let (mut walk_poses, mut idle_poses) = (Vec::new(), Vec::new());
    pose_names(&dom, walk_out, &mut walk_poses);
    pose_names(&dom, idle_out, &mut idle_poses);
    assert!(walk_poses.iter().any(|n| n == "RightUpperLeg"), "Walk's IK bends the leg: {walk_poses:?}");
    assert!(!idle_poses.iter().any(|n| n == "RightUpperLeg"), "Idle has no IK: {idle_poses:?}");
}

#[test]
fn a_project_keeps_each_animations_own_properties() {
    let mut scene = sample("R15Rig.rbxm");
    let foot = bone_named(&scene, 0, "RightAnkle");
    let (mut a, walk) = walk(&scene, true);
    modifiers::add_ik(&mut scene, foot);
    a.fps = 60.0;
    let idle = new_animation(&mut a, &mut scene, "Idle");
    let index = scene.modifiers.iter().position(|m| m.bone == foot).unwrap();
    for gone in modifiers::remove(&mut scene, index) {
        a.forget_gone(&scene, gone);
    }
    a.fps = 24.0;
    let path = temp_file("save.kaye");
    project::save(&path, &scene, &a, &view()).unwrap();
    let opened = project::load(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(opened.animation, a);
    let (mut scene, mut a) = (opened.scene, opened.animation);
    assert!(a.separate);
    go_to(&mut a, &mut scene, walk);
    assert_eq!(a.fps, 60.0);
    assert!(has_ik(&scene, foot));
    go_to(&mut a, &mut scene, idle);
    assert_eq!(a.fps, 24.0);
    assert!(!has_ik(&scene, foot));
}

#[test]
fn a_file_whose_own_modifiers_point_nowhere_is_refused() {
    let mut scene = sample("R15Rig.rbxm");
    let foot = bone_named(&scene, 0, "RightAnkle");
    let (mut a, _) = walk(&scene, true);
    modifiers::add_ik(&mut scene, foot);
    new_animation(&mut a, &mut scene, "Idle");
    let path = temp_file("damaged.kaye");
    project::save(&path, &scene, &a, &view()).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    json["animation"]["clips"][0]["setup"]["modifiers"][0]["bone"] = 99999.into();
    std::fs::write(&path, json.to_string()).unwrap();
    let refused = project::load(&path).unwrap_err();
    std::fs::remove_file(&path).ok();
    assert!(refused.contains("damaged"), "{refused}");
}

#[test]
fn looking_at_another_rig_with_its_own_frame_rate_isnt_unsaved_work() {
    let mut scene = sample("R15Rig.rbxm");
    scene.append(sample("R15Rig.rbxm"));
    scene.evaluate();
    let mut a = Animation::default();
    a.ensure_clips(&scene);
    a.show_clip(a.active_clip(0).unwrap());
    a.set_separate(&scene, true);
    a.fps = 60.0;
    let saved = project::fingerprint(&scene, &a);
    a.show_clip(a.active_clip(1).unwrap());
    assert_eq!(a.fps, 30.0, "the other rig's animation has its own");
    assert_eq!(project::fingerprint(&scene, &a), saved);
    a.fps = 24.0;
    assert_ne!(project::fingerprint(&scene, &a), saved, "changing it does count");
}
