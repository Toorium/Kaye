//! Animations saved inside a model file, in the shape Studio's Animation Editor leaves
//! them. The easing tests hold numbers measured from a real Animator in Studio.

use std::path::Path;

use glam::{Quat, Vec3};
use kaye::animation::{Animation, EaseStyle};
use kaye::export;
use kaye::import::{import_bytes, import_dom, import_file};
use kaye::math::Xf;
use kaye::scene::{BoneId, BoneLink, RigId, Scene};
use kaye::sequences;
use rbx_dom_weak::{InstanceBuilder, WeakDom};
use rbx_types::{CFrame, Enum, Matrix3, Ref, Variant, Vector3};

const LINEAR: u32 = 0;
const CONSTANT: u32 = 1;
const ELASTIC: u32 = 2;
const CUBIC: u32 = 3;
const BOUNCE: u32 = 4;
const CUBIC_V2: u32 = 5;
const IN: u32 = 0;
const OUT: u32 = 1;
const IN_OUT: u32 = 2;

fn sample_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(name)
}

fn sample_dom(name: &str) -> WeakDom {
    rbx_binary::from_reader(&std::fs::read(sample_path(name)).unwrap()[..]).unwrap()
}

/// The rig at the top of a sample file.
fn model(dom: &WeakDom) -> Ref {
    dom.root().children()[0]
}

fn along_x(x: f32) -> CFrame {
    CFrame::new(Vector3::new(x, 0.0, 0.0), Matrix3::identity())
}

fn add_pose(dom: &mut WeakDom, parent: Ref, name: &str, x: f32, weight: f32, easing: (u32, u32)) -> Ref {
    let pose = InstanceBuilder::new("Pose")
        .with_name(name)
        .with_property("CFrame", Variant::CFrame(along_x(x)))
        .with_property("EasingStyle", Variant::Enum(Enum::from_u32(easing.0)))
        .with_property("EasingDirection", Variant::Enum(Enum::from_u32(easing.1)))
        .with_property("Weight", Variant::Float32(weight));
    dom.insert(parent, pose)
}

/// A KeyframeSequence moving one thing, reached through `chain`, as Studio nests them. What's
/// on the way has no weight, as it doesn't when nothing keyed it.
fn add_sequence(dom: &mut WeakDom, parent: Ref, name: &str, chain: &[&str], keys: &[(f32, f32)], easing: (u32, u32)) -> Ref {
    let sequence = dom.insert(parent, InstanceBuilder::new("KeyframeSequence").with_name(name).with_property("Loop", Variant::Bool(false)));
    for (time, x) in keys {
        let keyframe = dom.insert(sequence, InstanceBuilder::new("Keyframe").with_property("Time", Variant::Float32(*time)));
        let mut at = keyframe;
        for (index, step) in chain.iter().enumerate() {
            let moved = index + 1 == chain.len();
            at = add_pose(dom, at, step, if moved { *x } else { 0.0 }, if moved { 1.0 } else { 0.0 }, easing);
        }
    }
    sequence
}

fn add_anim_saves(dom: &mut WeakDom) -> Ref {
    let top = model(dom);
    dom.insert(top, InstanceBuilder::new("ObjectValue").with_name("AnimSaves"))
}

const R15_ARM: [&str; 4] = ["HumanoidRootPart", "LowerTorso", "UpperTorso", "RightUpperArm"];
const R6_ARM: [&str; 3] = ["HumanoidRootPart", "Torso", "Right Arm"];

fn load_dom(dom: &WeakDom) -> (Scene, Animation, Vec<String>) {
    let imported = import_dom(dom, "test.rbxm").unwrap();
    let (mut scene, mut animation) = (imported.scene, Animation::default());
    let mut warnings = imported.warnings;
    warnings.extend(sequences::load(&mut animation, &mut scene, imported.sequences, 0, None));
    animation.ensure_clips(&scene);
    (scene, animation, warnings)
}

fn moving(scene: &Scene, rig: RigId, part: &str) -> BoneId {
    let moves_it = |bone: &BoneId| matches!(scene.bones[*bone].link, BoneLink::Joint { part1, .. } if scene.parts[part1].name == part);
    scene.joints(rig).into_iter().find(moves_it).unwrap_or_else(|| panic!("nothing moves {part}"))
}

fn names(animation: &Animation, rig: RigId) -> Vec<String> {
    animation.clips_of(rig).into_iter().map(|clip| animation.clips[clip].name.clone()).collect()
}

fn x_at(animation: &Animation, bone: BoneId, frame: f32) -> f32 {
    animation.sample(bone, frame).unwrap().pos.x
}

#[test]
fn a_rigs_anim_saves_become_its_animations() {
    let mut dom = sample_dom("R15Rig.rbxm");
    let saves = add_anim_saves(&mut dom);
    add_sequence(&mut dom, saves, "Walk", &R15_ARM, &[(0.0, 0.0), (0.5, 1.0), (1.0, 3.0)], (LINEAR, IN));
    add_sequence(&mut dom, saves, "Idle", &R15_ARM, &[(0.0, 2.0), (2.0, 4.0)], (LINEAR, IN));
    let mut bytes = Vec::new();
    rbx_binary::to_writer(&mut bytes, &dom, dom.root().children()).unwrap();

    let imported = import_bytes(&bytes, "rig.rbxm").unwrap();
    assert_eq!(imported.sequences.len(), 2);
    let (mut scene, mut animation) = (imported.scene, Animation::default());
    let warnings = sequences::load(&mut animation, &mut scene, imported.sequences, 0, None);
    animation.ensure_clips(&scene);
    assert!(warnings.is_empty(), "{warnings:?}");

    assert_eq!(names(&animation, 0), ["Walk", "Idle"], "the rig has no empty animation besides these");
    let arm = moving(&scene, 0, "RightUpperArm");
    let (walk, idle) = (0, 1);
    assert!(animation.clips[walk].active, "it starts out playing the first");
    assert_eq!((animation.clips[walk].start, animation.clips[walk].end, animation.clips[walk].looping), (0, 30, false));
    assert_eq!(animation.clips[idle].end, 60);
    animation.apply(&mut scene, 15.0);
    assert!(scene.bones[arm].pose.approx_eq(Xf::new(Quat::IDENTITY, Vec3::X)));
    assert!((x_at(&animation, arm, 22.5) - 2.0).abs() < 1e-3, "between keys it's as linear as Studio's");
    assert_eq!(animation.tracks.len(), 1, "only what was keyed has a track");

    animation.set_active_clip(&scene, idle);
    assert!((x_at(&animation, arm, 30.0) - 3.0).abs() < 1e-3, "the other one is there too");
}

/// `samples/StudioAnimSaves.rbxm` came out of Studio itself, with real poses in it from an
/// animation made there. These are what Studio said the Head's poses were.
#[test]
fn a_file_studio_wrote_loads_with_every_rig_and_animation() {
    let imported = import_file(&sample_path("StudioAnimSaves.rbxm")).unwrap();
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    let (mut scene, mut animation) = (imported.scene, Animation::default());
    let warnings = sequences::load(&mut animation, &mut scene, imported.sequences, 0, None);
    animation.ensure_clips(&scene);
    assert!(warnings.is_empty(), "{warnings:?}");

    assert_eq!(scene.rigs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["RigOne", "RigTwo"]);
    assert_eq!(names(&animation, 0), ["Wave", "Nod"]);
    assert_eq!(names(&animation, 1), ["Turn"]);
    assert_eq!(animation.fps, 24.0, "keys a quarter of a second apart sit on whole frames at 24");
    assert_eq!(animation.clips.iter().map(|c| c.looping).collect::<Vec<_>>(), [true, false, true]);
    assert_eq!(animation.clips.iter().map(|c| c.end).collect::<Vec<_>>(), [18, 6, 6]);

    let check = |animation: &Animation, neck: BoneId, times: &[(f32, [f32; 3])]| {
        for (frame, look) in times {
            let pose = animation.sample(neck, *frame).unwrap();
            assert!(pose.pos.distance(Vec3::new(0.0082, -0.0090, -0.0205)) < 1e-3, "frame {frame}: {:?}", pose.pos);
            let got = pose.rot * Vec3::NEG_Z;
            assert!(got.distance(Vec3::from(*look)) < 1e-3, "frame {frame}: {got:?}");
        }
    };
    let (neck_one, neck_two) = (moving(&scene, 0, "Head"), moving(&scene, 1, "Head"));
    check(&animation, neck_one, &[(0.0, [-0.0744, 0.2074, -0.9754]), (6.0, [-0.0734, 0.2087, -0.9752]), (12.0, [-0.0640, 0.2206, -0.9733]), (18.0, [-0.0405, 0.2485, -0.9678])]);
    check(&animation, neck_two, &[(0.0, [-0.0564, 0.2299, -0.9716]), (6.0, [-0.0314, 0.2587, -0.9655])]);
    animation.set_active_clip(&scene, 1);
    check(&animation, neck_one, &[(0.0, [-0.0742, 0.2076, -0.9754]), (6.0, [-0.0699, 0.2132, -0.9745])]);
    assert_eq!(animation.tracks.keys().filter(|bone| scene.bones[**bone].rig == 0).count(), 1, "the poses on the way to the Head have no weight");
}

#[test]
fn every_rig_in_a_file_keeps_its_own_animations() {
    let mut r6 = sample_dom("R6Rig.rbxm");
    let mut r15 = sample_dom("R15Rig.rbxm");
    let saves = add_anim_saves(&mut r6);
    add_sequence(&mut r6, saves, "Wave", &R6_ARM, &[(0.0, 0.0), (1.0, 1.0)], (LINEAR, IN));
    let saves = add_anim_saves(&mut r15);
    add_sequence(&mut r15, saves, "Run", &R15_ARM, &[(0.0, 0.0), (1.0, 2.0)], (LINEAR, IN));
    add_sequence(&mut r15, saves, "Jump", &R15_ARM, &[(0.0, 0.0), (0.5, 3.0)], (LINEAR, IN));

    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let folder = dom.insert(root, InstanceBuilder::new("Folder").with_name("Rigs"));
    for (mut from, name) in [(r6, "First"), (r15, "Second")] {
        let top = model(&from);
        from.get_by_ref_mut(top).unwrap().name = name.to_string();
        from.transfer(top, &mut dom, folder);
    }

    let (scene, mut animation, warnings) = load_dom(&dom);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(scene.rigs.len(), 2);
    assert_eq!(names(&animation, 0), ["Wave"]);
    assert_eq!(names(&animation, 1), ["Run", "Jump"]);
    let (arm_a, arm_b) = (moving(&scene, 0, "Right Arm"), moving(&scene, 1, "RightUpperArm"));
    assert!((x_at(&animation, arm_a, 30.0) - 1.0).abs() < 1e-3);
    assert!((x_at(&animation, arm_b, 30.0) - 2.0).abs() < 1e-3, "both rigs play at once");
    animation.set_active_clip(&scene, 2);
    assert!((x_at(&animation, arm_b, 15.0) - 3.0).abs() < 1e-3);
}

#[test]
fn a_file_added_to_an_open_scene_keys_the_rig_it_brought() {
    let mut scene = import_file(&sample_path("R6Rig.rbxm")).unwrap().scene;
    let mut animation = Animation::default();
    animation.ensure_clips(&scene);
    let old_arm = moving(&scene, 0, "Right Arm");
    animation.set_key(old_arm, 0, Xf::new(Quat::IDENTITY, Vec3::Y));
    animation.set_key(old_arm, 9, Xf::new(Quat::IDENTITY, Vec3::Z));

    let mut dom = sample_dom("R15Rig.rbxm");
    let saves = add_anim_saves(&mut dom);
    add_sequence(&mut dom, saves, "Run", &R15_ARM, &[(0.0, 0.0), (1.0, 2.0)], (LINEAR, IN));
    let imported = import_dom(&dom, "second.rbxm").unwrap();
    let first_new_rig = scene.rigs.len();
    scene.append(imported.scene);
    let warnings = sequences::load(&mut animation, &mut scene, imported.sequences, first_new_rig, Some(0));
    assert!(warnings.is_empty(), "{warnings:?}");

    assert_eq!(names(&animation, 0), ["Animation"], "the rig that was open is left alone");
    assert_eq!(names(&animation, first_new_rig), ["Run"]);
    let new_arm = moving(&scene, first_new_rig, "RightUpperArm");
    assert!((x_at(&animation, new_arm, 30.0) - 2.0).abs() < 1e-3);
    assert_eq!(animation.tracks[&old_arm].keys.len(), 2);
}

#[test]
fn what_kaye_exports_comes_back_the_same() {
    let scene = import_file(&sample_path("R15Rig.rbxm")).unwrap().scene;
    let (arm, leg) = (moving(&scene, 0, "RightUpperArm"), moving(&scene, 0, "LeftUpperLeg"));
    let mut animation = Animation { end: 40, ..Animation::default() };
    animation.ensure_clips(&scene);
    animation.clips[0].name = "Wave".to_string();
    animation.show_clip(0);
    animation.set_key(arm, 0, Xf::IDENTITY);
    animation.set_key(arm, 40, Xf::new(Quat::from_rotation_z(1.2), Vec3::new(0.0, 0.2, 0.0)));
    let second = animation.add_clip(0);
    animation.clips[second].name = "Kick".to_string();
    animation.set_active_clip(&scene, second);
    animation.set_key(leg, 0, Xf::IDENTITY);
    animation.set_key(leg, 20, Xf::new(Quat::from_rotation_x(-0.8), Vec3::ZERO));
    animation.clips[second].end = 20;
    animation.clips[second].looping = false;

    let dom = export::model_with_animations(&scene, &animation, 0).unwrap();
    let (loaded, read_back, warnings) = load_dom(&dom);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(names(&read_back, 0), ["Wave", "Kick"]);
    assert_eq!(read_back.clips[0].end, 40);
    assert!(read_back.clips[0].looping);
    assert_eq!((read_back.clips[1].end, read_back.clips[1].looping), (20, false));

    let wanted = animation.playing_clip(&mut scene.clone(), 0);
    for frame in [0, 7, 20, 33, 40] {
        let (made, came_back) = (wanted.sample(arm, frame as f32).unwrap(), read_back.sample(moving(&loaded, 0, "RightUpperArm"), frame as f32).unwrap());
        assert!(made.pos.distance(came_back.pos) < 1e-3, "frame {frame}");
        assert!(made.rot.angle_between(came_back.rot) < 3e-3, "frame {frame}");
    }
}

/// What a real Animator did a quarter, half and three quarters of the way between two
/// keys, 10 studs apart, in Studio on 2026-11-03.
const MEASURED: [((u32, u32), [f32; 3]); 11] = [
    ((LINEAR, IN), [2.5, 5.0, 7.5]),
    ((CUBIC, IN), [5.78, 8.75, 9.84]),
    ((CUBIC, OUT), [0.16, 1.25, 4.22]),
    ((CUBIC, IN_OUT), [0.62, 5.0, 9.38]),
    ((CUBIC_V2, IN), [0.16, 1.25, 4.22]),
    ((CUBIC_V2, OUT), [5.78, 8.75, 9.84]),
    ((CUBIC_V2, IN_OUT), [0.62, 5.0, 9.38]),
    ((BOUNCE, IN), [0.27, 2.34, 5.27]),
    ((BOUNCE, OUT), [4.73, 7.66, 9.73]),
    ((ELASTIC, IN), [-0.06, -0.16, 0.88]),
    ((ELASTIC, OUT), [9.12, 10.16, 10.06]),
];

#[test]
fn easing_plays_the_way_it_does_in_studio() {
    for (easing, wanted) in MEASURED {
        let mut dom = sample_dom("R6Rig.rbxm");
        let saves = add_anim_saves(&mut dom);
        add_sequence(&mut dom, saves, "Ease", &R6_ARM, &[(0.0, 0.0), (1.0, 10.0)], easing);
        let (scene, animation, _) = load_dom(&dom);
        let arm = moving(&scene, 0, "Right Arm");
        for (share, wanted) in [0.25, 0.5, 0.75].into_iter().zip(wanted) {
            let got = x_at(&animation, arm, share * 30.0);
            assert!((got - wanted).abs() < 0.02, "{easing:?} at {share}: Studio had {wanted}, Kaye has {got}");
        }
    }
}

#[test]
fn the_easing_on_a_span_is_the_one_on_its_first_key() {
    let mut dom = sample_dom("R6Rig.rbxm");
    let saves = add_anim_saves(&mut dom);
    add_sequence(&mut dom, saves, "Ease", &R6_ARM, &[(0.0, 0.0)], (CUBIC, IN));
    let sequence = add_sequence(&mut dom, saves, "Second", &R6_ARM, &[(1.0, 10.0)], (LINEAR, IN));
    let keyframe = dom.get_by_ref(sequence).unwrap().children()[0];
    let first = dom.get_by_ref(saves).unwrap().children()[0];
    dom.transfer_within(keyframe, first);
    dom.destroy(sequence);

    let (scene, animation, _) = load_dom(&dom);
    let arm = moving(&scene, 0, "Right Arm");
    assert!((x_at(&animation, arm, 7.5) - 5.78).abs() < 0.02, "Studio had 5.78 here");
}

#[test]
fn constant_keys_let_go_when_they_do_in_studio() {
    let run = |direction: u32| {
        let mut dom = sample_dom("R6Rig.rbxm");
        let saves = add_anim_saves(&mut dom);
        add_sequence(&mut dom, saves, "Step", &R6_ARM, &[(0.0, 0.0), (1.0, 10.0), (2.0, 20.0)], (CONSTANT, direction));
        let (scene, animation, _) = load_dom(&dom);
        let arm = moving(&scene, 0, "Right Arm");
        assert!(animation.tracks[&arm].keys.iter().all(|k| k.easing.style == EaseStyle::Constant));
        [1.0, 15.0, 29.0, 31.0, 45.0, 59.0].map(|frame| x_at(&animation, arm, frame))
    };
    assert_eq!(run(IN), [10.0, 10.0, 10.0, 20.0, 20.0, 20.0], "In jumps right away");
    assert_eq!(run(OUT), [0.0, 0.0, 0.0, 10.0, 10.0, 10.0], "Out holds to the next key");
    assert_eq!(run(IN_OUT), [0.0, 10.0, 10.0, 10.0, 20.0, 20.0], "InOut lets go halfway");
}

#[test]
fn a_loose_sequence_goes_on_the_rig_it_fits() {
    let mut r6 = sample_dom("R6Rig.rbxm");
    let mut r15 = sample_dom("R15Rig.rbxm");
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let top = model(&r6);
    r6.transfer(top, &mut dom, root);
    let top = model(&r15);
    r15.transfer(top, &mut dom, root);
    add_sequence(&mut dom, root, "ForR15", &R15_ARM, &[(0.0, 0.0), (1.0, 2.0)], (LINEAR, IN));
    add_sequence(&mut dom, root, "ForR6", &R6_ARM, &[(0.0, 0.0), (1.0, 2.0)], (LINEAR, IN));

    let (_, animation, warnings) = load_dom(&dom);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(names(&animation, 0), ["ForR6"]);
    assert_eq!(names(&animation, 1), ["ForR15"]);
}

#[test]
fn a_file_of_only_sequences_goes_on_the_open_rig_and_replaces_its_empty_animation() {
    let mut scene = import_file(&sample_path("R15Rig.rbxm")).unwrap().scene;
    scene.append(import_file(&sample_path("R15Rig.rbxm")).unwrap().scene);
    let mut animation = Animation::default();
    animation.ensure_clips(&scene);

    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    add_sequence(&mut dom, root, "Run", &R15_ARM, &[(0.0, 0.0), (1.0, 2.0)], (LINEAR, IN));
    let imported = import_dom(&dom, "only.rbxm").unwrap();
    assert!(imported.scene.parts.is_empty());
    let rigs = scene.rigs.len();
    scene.append(imported.scene);
    let warnings = sequences::load(&mut animation, &mut scene, imported.sequences, rigs, Some(1));

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(names(&animation, 0), ["Animation"]);
    assert_eq!(names(&animation, 1), ["Run"], "it goes to the rig in focus, which plays it");
    assert!(animation.clips[animation.active_clip(1).unwrap()].name == "Run");
    assert!(animation.tracks.contains_key(&moving(&scene, 1, "RightUpperArm")));
}

#[test]
fn an_animation_with_no_rig_to_go_on_is_a_warning() {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    add_sequence(&mut dom, root, "Run", &R15_ARM, &[(0.0, 0.0)], (LINEAR, IN));
    let (scene, animation, warnings) = load_dom(&dom);
    assert!(scene.is_empty());
    assert!(animation.clips.is_empty());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("Run"));
}

#[test]
fn a_file_with_nothing_useful_is_still_an_error() {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    dom.insert(root, InstanceBuilder::new("Folder"));
    assert!(import_dom(&dom, "empty.rbxm").is_err());
}

#[test]
fn parts_the_rig_doesnt_have_are_a_warning_and_the_rest_still_loads() {
    let mut dom = sample_dom("R6Rig.rbxm");
    let saves = add_anim_saves(&mut dom);
    let sequence = add_sequence(&mut dom, saves, "Tail", &R6_ARM, &[(0.0, 0.0), (1.0, 1.0)], (LINEAR, IN));
    let keyframe = dom.get_by_ref(sequence).unwrap().children()[0];
    add_pose(&mut dom, keyframe, "Tail", 1.0, 1.0, (LINEAR, IN));
    let (_, animation, warnings) = load_dom(&dom);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("Tail"));
    assert_eq!(names(&animation, 0), ["Tail"]);
}

#[test]
fn a_sequence_that_moves_nothing_here_is_skipped_with_a_warning() {
    let mut dom = sample_dom("R6Rig.rbxm");
    let saves = add_anim_saves(&mut dom);
    add_sequence(&mut dom, saves, "Other", &["Somewhere", "Else"], &[(0.0, 0.0), (1.0, 1.0)], (LINEAR, IN));
    let (_, animation, warnings) = load_dom(&dom);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(names(&animation, 0), ["Animation"], "the rig still gets its empty one");
}

#[test]
fn two_animations_of_one_name_both_stay() {
    let mut dom = sample_dom("R6Rig.rbxm");
    let saves = add_anim_saves(&mut dom);
    add_sequence(&mut dom, saves, "Walk", &R6_ARM, &[(0.0, 0.0), (1.0, 1.0)], (LINEAR, IN));
    add_sequence(&mut dom, saves, "Walk", &R6_ARM, &[(0.0, 0.0), (1.0, 2.0)], (LINEAR, IN));
    let (_, animation, _) = load_dom(&dom);
    assert_eq!(names(&animation, 0), ["Walk", "Walk 2"]);
}

#[test]
fn an_empty_project_takes_the_frame_rate_that_fits() {
    let sixtieths = [(0.0, 0.0), (1.0 / 60.0, 1.0), (2.0 / 60.0, 2.0), (1.0, 3.0)];
    let mut dom = sample_dom("R6Rig.rbxm");
    let saves = add_anim_saves(&mut dom);
    add_sequence(&mut dom, saves, "Quick", &R6_ARM, &sixtieths, (LINEAR, IN));
    let (scene, animation, _) = load_dom(&dom);
    assert_eq!(animation.fps, 60.0);
    let arm = moving(&scene, 0, "Right Arm");
    assert_eq!(animation.tracks[&arm].keys.iter().map(|k| k.frame).collect::<Vec<_>>(), [0, 1, 2, 60], "no key was lost");
    assert_eq!(animation.clips[0].end, 60);

    let thirtieths = [(0.0, 0.0), (0.5, 1.0)];
    let mut dom = sample_dom("R6Rig.rbxm");
    let saves = add_anim_saves(&mut dom);
    add_sequence(&mut dom, saves, "Plain", &R6_ARM, &thirtieths, (LINEAR, IN));
    assert_eq!(load_dom(&dom).1.fps, 30.0, "30 already fits, so it stays");
}

#[test]
fn a_project_with_keys_keeps_its_frame_rate() {
    let mut scene = import_file(&sample_path("R6Rig.rbxm")).unwrap().scene;
    let mut animation = Animation::default();
    animation.ensure_clips(&scene);
    animation.set_key(moving(&scene, 0, "Left Arm"), 0, Xf::IDENTITY);

    let mut dom = sample_dom("R6Rig.rbxm");
    let saves = add_anim_saves(&mut dom);
    add_sequence(&mut dom, saves, "Quick", &R6_ARM, &[(0.0, 0.0), (1.0 / 60.0, 1.0), (2.0 / 60.0, 2.0)], (LINEAR, IN));
    let imported = import_dom(&dom, "quick.rbxm").unwrap();
    let rigs = scene.rigs.len();
    scene.append(imported.scene);
    sequences::load(&mut animation, &mut scene, imported.sequences, rigs, None);
    assert_eq!(animation.fps, 30.0);
    let arm = moving(&scene, rigs, "Right Arm");
    assert_eq!(animation.tracks[&arm].keys.iter().map(|k| k.frame).collect::<Vec<_>>(), [0, 1], "keys rounded to one frame become one");
}

#[test]
fn a_curve_animation_says_why_it_was_left_out() {
    let mut dom = sample_dom("R6Rig.rbxm");
    let saves = add_anim_saves(&mut dom);
    dom.insert(saves, InstanceBuilder::new("CurveAnimation").with_name("Smooth"));
    let (_, _, warnings) = load_dom(&dom);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("Smooth"));
}

#[test]
fn rigs_with_nothing_saved_import_as_they_always_did() {
    let imported = import_file(&sample_path("R15Rig.rbxm")).unwrap();
    assert!(imported.sequences.is_empty());
}
