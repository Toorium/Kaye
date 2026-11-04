//! Several animations in one project: each rig has its own list, plays one of them, and
//! can write out all of them.

use std::path::Path;

use glam::{Quat, Vec3};
use kaye::animation::Animation;
use kaye::export;
use kaye::import::import_file;
use kaye::math::Xf;
use kaye::project::{self, View};
use kaye::scene::{BoneId, Scene};
use rbx_dom_weak::{Instance, WeakDom};

fn sample(name: &str) -> Scene {
    let mut scene = import_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(name)).unwrap().scene;
    scene.evaluate();
    scene
}

/// Two characters in one scene.
fn two_rigs() -> Scene {
    let mut scene = sample("R15Rig.rbxm");
    scene.append(sample("R15Rig.rbxm"));
    scene.evaluate();
    scene
}

fn bone_named(scene: &Scene, rig: usize, name: &str) -> BoneId {
    scene.live_bones(rig).into_iter().find(|b| scene.bones[*b].name == name).unwrap_or_else(|| panic!("no bone {name}"))
}

fn turned(angle: f32) -> Xf {
    Xf::new(Quat::from_rotation_x(angle), Vec3::ZERO)
}

fn view() -> View {
    View { frame: 0.0, target: Vec3::ZERO, yaw: 0.0, pitch: 0.0, dist: 10.0, ortho: false, floor_y: 0.0, camera: None, preview: false, through: false, motion_paths: Vec::new() }
}

fn temp_file(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("kaye-animations-{}-{:?}-{name}", std::process::id(), std::thread::current().id()))
}

fn children<'a>(dom: &'a WeakDom, inst: &'a Instance) -> impl Iterator<Item = &'a Instance> {
    inst.children().iter().filter_map(|c| dom.get_by_ref(*c))
}

#[test]
fn every_rig_starts_with_one_animation_holding_its_keys() {
    let scene = two_rigs();
    let mut a = Animation::default();
    let arm = bone_named(&scene, 0, "RightShoulder");
    a.set_key(arm, 0, turned(0.3));
    a.ensure_clips(&scene);
    assert_eq!(a.clips_of(0).len(), 1);
    assert_eq!(a.clips_of(1).len(), 1);
    assert_eq!(a.clips[a.active_clip(0).unwrap()].name, "Animation");
    assert!(a.key_at(arm, 0).is_some(), "the keys it had are the ones playing");
    let before = a.clone();
    a.ensure_clips(&scene);
    assert_eq!(a, before, "asking again changes nothing");
}

#[test]
fn switching_animation_swaps_the_keys_of_that_rig_only() {
    let scene = two_rigs();
    let (mine, theirs) = (bone_named(&scene, 0, "RightShoulder"), bone_named(&scene, 1, "RightShoulder"));
    let mut a = Animation::default();
    a.set_key(mine, 0, turned(0.3));
    a.set_key(theirs, 0, turned(0.6));
    a.ensure_clips(&scene);
    let walk = a.active_clip(0).unwrap();
    let run = a.add_clip(0);
    a.set_active_clip(&scene, run);
    assert_eq!(a.active_clip(0), Some(run));
    assert!(a.key_at(mine, 0).is_none(), "the new one starts empty");
    assert!(a.key_at(theirs, 0).is_some(), "the other rig keeps playing what it was");
    a.set_key(mine, 5, turned(1.0));
    a.set_active_clip(&scene, walk);
    assert!(a.key_at(mine, 0).is_some() && a.key_at(mine, 5).is_none());
    a.set_active_clip(&scene, run);
    assert!(a.key_at(mine, 5).is_some() && a.key_at(mine, 0).is_none(), "each keeps its own keys");
}

#[test]
fn each_animation_has_its_own_range() {
    let scene = two_rigs();
    let mut a = Animation { start: 0, end: 30, ..Animation::default() };
    a.ensure_clips(&scene);
    let first = a.active_clip(0).unwrap();
    a.show_clip(first);
    let second = a.add_clip(0);
    a.show_clip(second);
    a.end = 90;
    a.looping = false;
    a.show_clip(first);
    assert_eq!((a.start, a.end, a.looping), (0, 30, true));
    a.show_clip(second);
    assert_eq!((a.start, a.end, a.looping), (0, 90, false));
}

#[test]
fn names_stay_different_within_a_rig() {
    let scene = two_rigs();
    let mut a = Animation::default();
    a.ensure_clips(&scene);
    let second = a.add_clip(0);
    assert_eq!(a.clips[second].name, "Animation 2");
    let third = a.add_clip(0);
    assert_eq!(a.clips[third].name, "Animation 3");
    let other_rig = a.add_clip(1);
    assert_eq!(a.clips[other_rig].name, "Animation 2", "another rig has its own names");
    assert_eq!(a.unused_clip_name(0, "Animation 2", None), "Animation 4", "a number counts up, and doesn't pile up");
    assert_eq!(a.unused_clip_name(0, "Animation 2", Some(second)), "Animation 2", "its own name is free to it");
    assert_eq!(a.unused_clip_name(0, "  ", Some(second)), "Animation 2", "a blank name isn't a name");
}

#[test]
fn a_duplicate_is_a_separate_copy() {
    let scene = two_rigs();
    let arm = bone_named(&scene, 0, "RightShoulder");
    let other = bone_named(&scene, 1, "RightShoulder");
    let mut a = Animation { end: 40, ..Animation::default() };
    a.set_key(arm, 0, turned(0.3));
    a.set_key(other, 0, turned(0.4));
    a.ensure_clips(&scene);
    let first = a.active_clip(0).unwrap();
    let copy = a.duplicate_clip(&scene, first);
    assert_eq!(a.clips[copy].name, "Animation 2");
    assert_eq!(a.clips[copy].end, 40);
    assert!(a.clips[copy].tracks.contains_key(&arm));
    assert!(!a.clips[copy].tracks.contains_key(&other), "only that rig's keys");
    a.set_active_clip(&scene, copy);
    a.set_key(arm, 9, turned(0.9));
    a.set_active_clip(&scene, first);
    assert!(a.key_at(arm, 9).is_none(), "changing the copy leaves the original");
}

#[test]
fn deleting_hands_over_to_another_and_a_rig_keeps_its_last() {
    let scene = two_rigs();
    let arm = bone_named(&scene, 0, "RightShoulder");
    let mut a = Animation::default();
    a.set_key(arm, 0, turned(0.3));
    a.ensure_clips(&scene);
    let first = a.active_clip(0).unwrap();
    assert!(!a.remove_clip(&scene, first), "the last one stays");
    let second = a.add_clip(0);
    a.set_active_clip(&scene, second);
    a.set_key(arm, 8, turned(0.8));
    a.show_clip(second);
    assert!(a.remove_clip(&scene, second));
    assert!(a.key_at(arm, 8).is_none(), "its keys went with it");
    assert!(a.key_at(arm, 0).is_some(), "the other one took over");
    assert_eq!(a.clips_of(0).len(), 1);
    assert!(a.shown.is_none_or(|i| i < a.clips.len()));
}

#[test]
fn removing_a_clip_keeps_later_ones_pointing_right() {
    let scene = two_rigs();
    let mut a = Animation::default();
    a.ensure_clips(&scene);
    let middle = a.add_clip(0);
    let last = a.add_clip(1);
    a.show_clip(last);
    assert!(a.remove_clip(&scene, middle));
    assert_eq!(a.shown, Some(last - 1));
    assert_eq!(a.clips[a.shown.unwrap()].rig, 1);
}

#[test]
fn forgetting_a_bone_reaches_every_animation() {
    let scene = two_rigs();
    let arm = bone_named(&scene, 0, "RightShoulder");
    let mut a = Animation::default();
    a.set_key(arm, 0, turned(0.3));
    a.ensure_clips(&scene);
    let second = a.add_clip(0);
    a.set_active_clip(&scene, second);
    a.set_key(arm, 4, turned(0.4));
    a.forget_bone(arm);
    assert!(a.keyed_bones().is_empty());
}

#[test]
fn bones_an_animation_doesnt_key_go_back_to_rest() {
    let mut scene = two_rigs();
    let arm = bone_named(&scene, 0, "RightShoulder");
    let camera = scene.add_camera(Xf::new(Quat::IDENTITY, Vec3::new(1.0, 2.0, 3.0)), 70.0);
    let mut a = Animation::default();
    a.set_key(arm, 0, turned(0.5));
    a.apply(&mut scene, 0.0);
    a.ensure_clips(&scene);
    let other = a.add_clip(0);
    a.set_active_clip(&scene, other);
    a.rest_unkeyed(&mut scene, 0);
    assert!(scene.bones[arm].pose.approx_eq(Xf::IDENTITY));
    let rig = scene.bones[camera].rig;
    a.rest_unkeyed(&mut scene, rig);
    assert_eq!(scene.bones[camera].pose.pos, Vec3::new(1.0, 2.0, 3.0), "a camera stays where it was put");
}

#[test]
fn all_animations_survive_a_save() {
    let scene = two_rigs();
    let arm = bone_named(&scene, 0, "RightShoulder");
    let mut a = Animation { end: 30, ..Animation::default() };
    a.set_key(arm, 0, turned(0.3));
    a.ensure_clips(&scene);
    let walk = a.active_clip(0).unwrap();
    a.clips[walk].name = "Walk".to_string();
    a.show_clip(walk);
    let run = a.duplicate_clip(&scene, walk);
    a.clips[run].name = "Run".to_string();
    let path = temp_file("save.kaye");
    project::save(&path, &scene, &a, &view()).unwrap();
    let opened = project::load(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(opened.animation, a);
    let names: Vec<&str> = opened.animation.clips_of(0).into_iter().map(|c| opened.animation.clips[c].name.as_str()).collect();
    assert_eq!(names, ["Walk", "Run"]);
}

#[test]
fn a_file_with_clips_in_a_bad_state_is_refused() {
    let scene = sample("R15Rig.rbxm");
    let mut a = Animation::default();
    a.ensure_clips(&scene);
    let path = temp_file("damaged.kaye");
    project::save(&path, &scene, &a, &view()).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    json["animation"]["clips"][0]["rig"] = 99.into();
    std::fs::write(&path, json.to_string()).unwrap();
    assert!(project::load(&path).unwrap_err().contains("damaged"));

    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    json["animation"]["clips"][0]["active"] = false.into();
    std::fs::write(&path, json.to_string()).unwrap();
    assert!(project::load(&path).unwrap_err().contains("damaged"), "a rig has to be playing one");
    std::fs::remove_file(&path).ok();
}

#[test]
fn an_older_file_with_layers_opens_as_one_animation() {
    let scene = sample("R15Rig.rbxm");
    let arm = bone_named(&scene, 0, "RightShoulder");
    let mut a = Animation::default();
    a.set_key(arm, 0, turned(0.5));
    let path = temp_file("older.kaye");
    project::save(&path, &scene, &a, &view()).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    json["kaye_project"] = 1.into();
    let base = json["animation"]["tracks"].clone();
    json["animation"]["tracks"] = serde_json::json!({});
    json["animation"]["layers"] = serde_json::json!([
        { "name": "Base", "weight": 1.0, "muted": false, "tracks": base },
        { "name": "Layer 1", "weight": 1.0, "muted": false, "tracks": {} },
    ]);
    let mut change = kaye::animation::Track::default();
    change.set_key(0, turned(0.25));
    json["animation"]["tracks"][arm.to_string()] = serde_json::to_value(&change).unwrap();
    json["animation"]["active_layer"] = 1.into();
    json["animation"].as_object_mut().unwrap().remove("clips");
    json["animation"].as_object_mut().unwrap().remove("shown");
    std::fs::write(&path, json.to_string()).unwrap();

    let mut opened = project::load(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert!(opened.animation.layers.is_empty());
    assert_eq!(opened.animation.clips_of(0).len(), 1);
    opened.animation.apply(&mut opened.scene, 0.0);
    assert!(opened.scene.bones[arm].pose.rot.angle_between(turned(0.75).rot) < 1e-3, "both layers are in the keys");
}

#[test]
fn looking_at_another_rig_doesnt_count_as_unsaved_work() {
    let scene = two_rigs();
    let mut a = Animation::default();
    a.ensure_clips(&scene);
    a.show_clip(a.active_clip(0).unwrap());
    let saved = project::fingerprint(&scene, &a);
    a.show_clip(a.active_clip(1).unwrap());
    assert_eq!(project::fingerprint(&scene, &a), saved);

    a.end += 5;
    assert_ne!(project::fingerprint(&scene, &a), saved, "changing a range does");
    a.end -= 5;
    let first = a.active_clip(0).unwrap();
    a.clips[first].name = "Walk".to_string();
    assert_ne!(project::fingerprint(&scene, &a), saved, "and so does a new name");
}

/// A rig with a Walk that moves one arm and a longer Wave that moves the other.
fn walk_and_wave(scene: &Scene) -> (Animation, BoneId, BoneId) {
    let (right, left) = (bone_named(scene, 0, "RightShoulder"), bone_named(scene, 0, "LeftShoulder"));
    let mut a = Animation { end: 10, ..Animation::default() };
    a.set_key(right, 0, Xf::IDENTITY);
    a.set_key(right, 10, turned(1.0));
    a.ensure_clips(scene);
    let walk = a.active_clip(0).unwrap();
    a.clips[walk].name = "Walk".to_string();
    a.show_clip(walk);
    let wave = a.add_clip(0);
    a.clips[wave].name = "Wave".to_string();
    a.set_active_clip(scene, wave);
    a.show_clip(wave);
    a.end = 20;
    a.set_key(left, 0, Xf::IDENTITY);
    a.set_key(left, 20, turned(-1.0));
    (a, right, left)
}

fn keyframes_of(dom: &WeakDom, sequence: &Instance) -> usize {
    children(dom, sequence).filter(|c| c.class == "Keyframe").count()
}

#[test]
fn a_rig_export_holds_every_animation_by_name() {
    let scene = sample("R15Rig.rbxm");
    let (a, _, _) = walk_and_wave(&scene);
    let dom = export::model_with_animations(&scene, &a, 0).unwrap();
    let model = children(&dom, dom.root()).next().unwrap();
    let saves = children(&dom, model).find(|c| c.name == "AnimSaves").unwrap();
    let mut found: Vec<(String, usize)> = children(&dom, saves).map(|s| (s.name.clone(), keyframes_of(&dom, s))).collect();
    found.sort();
    assert_eq!(found, [("Walk".to_string(), 11), ("Wave".to_string(), 21)], "each at its own length");
}

#[test]
fn a_keyframe_export_holds_every_animation_side_by_side() {
    let scene = sample("R15Rig.rbxm");
    let (a, _, _) = walk_and_wave(&scene);
    let dom = export::all_keyframe_sequences(&scene, &a, 0).unwrap();
    let mut names: Vec<&str> = children(&dom, dom.root()).inspect(|s| assert_eq!(s.class, "KeyframeSequence")).map(|s| s.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["Walk", "Wave"]);
}

#[test]
fn an_exported_animation_only_has_its_own_motion() {
    let mut scene = sample("R15Rig.rbxm");
    let (a, right, _) = walk_and_wave(&scene);
    // What's showing is Wave. Walk's pose must not leak into it, or Wave's into Walk.
    scene.bones[right].pose = turned(0.7);
    let dom = export::all_keyframe_sequences(&scene, &a, 0).unwrap();
    let walk = children(&dom, dom.root()).find(|s| s.name == "Walk").unwrap();
    let names = |sequence: &Instance| -> Vec<String> {
        let keyframe = children(&dom, sequence).next().unwrap();
        let mut stack: Vec<&Instance> = children(&dom, keyframe).collect();
        let mut names = Vec::new();
        while let Some(pose) = stack.pop() {
            names.push(pose.name.clone());
            stack.extend(children(&dom, pose));
        }
        names
    };
    assert!(names(walk).iter().any(|n| n == "RightUpperArm"));
    assert!(!names(walk).iter().any(|n| n == "LeftUpperArm"), "Wave's arm isn't in Walk");
}

#[test]
fn an_empty_animation_is_left_out_unless_all_of_them_are() {
    let scene = sample("R15Rig.rbxm");
    let (mut a, _, _) = walk_and_wave(&scene);
    a.add_clip(0);
    let dom = export::all_keyframe_sequences(&scene, &a, 0).unwrap();
    assert_eq!(children(&dom, dom.root()).count(), 2);

    let mut nothing = Animation::default();
    nothing.ensure_clips(&scene);
    assert!(export::all_keyframe_sequences(&scene, &nothing, 0).is_err());
}

#[test]
fn the_current_keyframe_export_is_only_the_one_playing() {
    let scene = sample("R15Rig.rbxm");
    let (a, _, _) = walk_and_wave(&scene);
    let dom = export::keyframe_sequence(&scene, &a, 0).unwrap();
    let only: Vec<&Instance> = children(&dom, dom.root()).collect();
    assert_eq!(only.len(), 1);
    assert_eq!(only[0].name, "Wave");
    assert_eq!(keyframes_of(&dom, only[0]), 21);
}
