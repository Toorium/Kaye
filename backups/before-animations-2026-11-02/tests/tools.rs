//! Switching IK and Unparent without a jump, spawned shapes, poses, baking, layers,
//! limits and the smaller modifiers, on real exports where one exists.

use std::path::Path;

use glam::{Quat, Vec3};
use kaye::animation::Animation;
use kaye::history::{History, PoseChange};
use kaye::import::import_file;
use kaye::math::Xf;
use kaye::modifiers::{self, IkEnd, ModifierKind};
use kaye::posing;
use kaye::project::{self, View};
use kaye::scene::{BoneId, Drive, PartShape, Scene};

fn rig(file: &str) -> Scene {
    import_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(file)).unwrap().scene
}

fn bone(s: &Scene, name: &str) -> BoneId {
    s.bones.iter().position(|b| b.name == name).unwrap_or_else(|| panic!("no bone {name}"))
}

fn place(s: &mut Scene, control: BoneId, at: Vec3) {
    let rot = s.bones[control].frame.rot;
    s.set_bone_frame(control, Xf::new(rot, at));
}

fn turned(x: f32) -> Xf {
    Xf::new(Quat::from_rotation_x(x), Vec3::ZERO)
}

fn assert_parts_within(before: &[Xf], s: &Scene, what: &str, studs: f32, facing: f32) {
    for (id, was) in before.iter().enumerate() {
        let now = s.parts[id].world;
        assert!(was.pos.distance(now.pos) < studs, "{what}: {} moved {} studs", s.parts[id].name, was.pos.distance(now.pos));
        assert!(was.rot.dot(now.rot).abs() > facing, "{what}: {} turned", s.parts[id].name);
    }
}

fn assert_same_parts(before: &[Xf], s: &Scene, what: &str) {
    assert_parts_within(before, s, what, 1e-3, 0.99999);
}

fn parts(s: &Scene) -> Vec<Xf> {
    s.parts.iter().map(|p| p.world).collect()
}

fn ik_on(s: &mut Scene, tip: &str) -> (usize, BoneId) {
    let tip = bone(s, tip);
    let target = modifiers::add_ik(s, tip);
    (s.modifiers.len() - 1, target)
}

// Switching IK and FK.

#[test]
fn switching_a_posed_limb_to_fk_and_back_moves_nothing() {
    for (file, tips) in [("R15Rig.rbxm", ["LeftAnkle", "RightWrist"]), ("R6Rig.rbxm", ["Left Hip", "Right Shoulder"])] {
        for tip in tips {
            let mut s = rig(file);
            let (index, target) = ik_on(&mut s, tip);
            let at = s.bones[target].head();
            place(&mut s, target, at + Vec3::new(0.4, 0.7, -0.5));
            let posed = parts(&s);
            modifiers::switch_ik(&mut s, index, false);
            assert!(!modifiers::is_on(&s, index));
            assert_same_parts(&posed, &s, &format!("{file} {tip} to FK"));
            modifiers::switch_ik(&mut s, index, true);
            assert!(modifiers::is_on(&s, index));
            assert_same_parts(&posed, &s, &format!("{file} {tip} back to IK"));
        }
    }
}

#[test]
fn a_limb_keeps_going_from_where_it_was_switched() {
    let mut s = rig("R15Rig.rbxm");
    let (index, target) = ik_on(&mut s, "LeftAnkle");
    let at = s.bones[target].head();
    place(&mut s, target, at + Vec3::new(0.0, 0.9, -0.6));
    modifiers::switch_ik(&mut s, index, false);
    // Moving the body now mustn't drag the limb back toward its target.
    let lower = bone(&s, "LeftKnee");
    let before = s.bones[lower].frame;
    let knee_pose = s.bones[lower].pose;
    s.set_bone_pose(lower, knee_pose * turned(0.3));
    assert!(s.bones[lower].frame.rot.dot(before.rot).abs() < 0.9999, "FK should move it now");
}

#[test]
fn a_limb_bent_by_hand_switches_to_ik_where_it_is() {
    for sideways in [false, true] {
        let mut s = rig("R15Rig.rbxm");
        let (index, _) = ik_on(&mut s, "LeftAnkle");
        modifiers::switch_ik(&mut s, index, false);
        let (hip, knee) = (bone(&s, "LeftHip"), bone(&s, "LeftKnee"));
        s.bones[hip].pose = Xf::new(Quat::from_rotation_y(0.5) * Quat::from_rotation_x(0.7), Vec3::ZERO);
        // The second one folds the knee off to the side, against the way IK was set up.
        s.bones[knee].pose = if sideways { Xf::new(Quat::from_rotation_z(0.8), Vec3::ZERO) } else { turned(-1.1) };
        s.evaluate();
        let posed = parts(&s);
        modifiers::switch_ik(&mut s, index, true);
        assert_same_parts(&posed, &s, "bent by hand, to IK");
    }
}

#[test]
fn switching_keeps_a_pole_and_twist_in_step() {
    let mut s = rig("R15Rig.rbxm");
    let (index, _) = ik_on(&mut s, "RightAnkle");
    modifiers::add_pole(&mut s, index);
    if let ModifierKind::Ik(ik) = &mut s.modifiers[index].kind {
        ik.twist = 25.0;
    }
    s.evaluate();
    modifiers::switch_ik(&mut s, index, false);
    let (hip, knee) = (bone(&s, "RightHip"), bone(&s, "RightKnee"));
    s.bones[hip].pose = Xf::new(Quat::from_rotation_y(-0.4) * Quat::from_rotation_x(0.9), Vec3::ZERO);
    s.bones[knee].pose = turned(-0.9);
    s.evaluate();
    let posed = parts(&s);
    modifiers::switch_ik(&mut s, index, true);
    assert_same_parts(&posed, &s, "pole and twist");
}

#[test]
fn switching_works_on_a_long_chain() {
    let mut s = rig("R15Rig.rbxm");
    let wrist = bone(&s, "LeftWrist");
    let target = modifiers::add_ik(&mut s, wrist);
    let index = s.modifiers.len() - 1;
    if let ModifierKind::Ik(ik) = &mut s.modifiers[index].kind {
        ik.chain = 3;
    }
    let at = s.bones[target].head();
    place(&mut s, target, at + Vec3::new(0.3, 0.6, -0.4));
    let posed = parts(&s);
    modifiers::switch_ik(&mut s, index, false);
    assert_same_parts(&posed, &s, "long chain to FK");
    modifiers::switch_ik(&mut s, index, true);
    assert_same_parts(&posed, &s, "long chain to IK");
}

#[test]
fn switching_from_a_blend_keeps_the_blend() {
    let mut s = rig("R15Rig.rbxm");
    let (index, target) = ik_on(&mut s, "LeftWrist");
    let at = s.bones[target].head();
    place(&mut s, target, at + Vec3::new(0.0, 1.0, -0.5));
    s.bones[target].value = 0.4;
    s.evaluate();
    let posed = parts(&s);
    modifiers::switch_ik(&mut s, index, false);
    assert_same_parts(&posed, &s, "from 0.4 to FK");
    let mut t = rig("R15Rig.rbxm");
    let (index, target) = ik_on(&mut t, "LeftWrist");
    let at = t.bones[target].head();
    place(&mut t, target, at + Vec3::new(0.0, 1.0, -0.5));
    t.bones[target].value = 0.7;
    t.evaluate();
    let posed = parts(&t);
    modifiers::switch_ik(&mut t, index, true);
    assert_same_parts(&posed, &t, "from 0.7 to IK");
}

#[test]
fn a_switch_on_a_hidden_modifier_turns_it_back_on() {
    let mut s = rig("R15Rig.rbxm");
    let (index, _) = ik_on(&mut s, "LeftAnkle");
    s.modifiers[index].enabled = false;
    s.evaluate();
    let posed = parts(&s);
    modifiers::switch_ik(&mut s, index, true);
    assert!(s.modifiers[index].enabled);
    assert_same_parts(&posed, &s, "hidden to IK");
}

#[test]
fn an_off_target_has_nothing_left_to_grab() {
    let mut s = rig("R15Rig.rbxm");
    let (index, target) = ik_on(&mut s, "LeftAnkle");
    let ankle = bone(&s, "LeftAnkle");
    assert_eq!(modifiers::control_of(&s, ankle), target);
    assert!(!modifiers::control_is_idle(&s, target));
    modifiers::switch_ik(&mut s, index, false);
    assert_eq!(modifiers::control_of(&s, ankle), ankle);
    assert!(modifiers::control_is_idle(&s, target));
    assert!(!modifiers::locked_by_ik(&s, bone(&s, "LeftKnee")));
}

// Switching Parent and Unparent.

#[test]
fn unparenting_and_parenting_a_posed_bone_moves_nothing() {
    for (file, name, parent) in [("R15Rig.rbxm", "LeftElbow", "LeftShoulder"), ("R6Rig.rbxm", "Left Shoulder", "RootJoint")] {
        let mut s = rig(file);
        let id = bone(&s, name);
        modifiers::add_unparent(&mut s, id);
        let index = s.modifiers.len() - 1;
        modifiers::switch_unparent(&mut s, index, false);
        s.bones[id].pose = turned(0.6);
        let up = bone(&s, parent);
        s.bones[up].pose = turned(-0.5);
        s.evaluate();
        let posed = parts(&s);

        modifiers::switch_unparent(&mut s, index, true);
        assert!(modifiers::is_on(&s, index));
        assert_same_parts(&posed, &s, "to Unparent");

        // It's let go: the parent can move without it.
        let frame = s.bones[id].frame;
        s.set_bone_pose(up, turned(0.9));
        assert!(frame.pos.distance(s.bones[id].frame.pos) < 1e-3, "{name} followed its parent");
        let moved = parts(&s);

        modifiers::switch_unparent(&mut s, index, false);
        assert!(!modifiers::is_on(&s, index));
        assert_same_parts(&moved, &s, "back to Parent");

        // It's taken back: the parent carries it again.
        let frame = s.bones[id].frame;
        s.set_bone_pose(up, turned(0.2));
        assert!(frame.pos.distance(s.bones[id].frame.pos) > 1e-2, "{name} stayed behind");
    }
}

#[test]
fn a_part_release_switches_cleanly_too() {
    let mut s = rig("R15Rig.rbxm");
    let id = bone(&s, "LeftElbow");
    modifiers::add_unparent(&mut s, id);
    let index = s.modifiers.len() - 1;
    if let ModifierKind::Unparent(u) = &mut s.modifiers[index].kind {
        u.position = false;
    }
    let up = bone(&s, "LeftShoulder");
    s.set_bone_pose(up, turned(0.7));
    let posed = parts(&s);
    modifiers::switch_unparent(&mut s, index, false);
    assert_same_parts(&posed, &s, "rotation only, to Parent");
    modifiers::switch_unparent(&mut s, index, true);
    assert_same_parts(&posed, &s, "rotation only, back");
}

// How a switch is keyed.

#[test]
fn a_keyed_switch_is_one_step_and_leaves_the_frames_before_alone() {
    let mut a = Animation::default();
    let pose = |x: f32| Xf::new(Quat::IDENTITY, Vec3::new(x, 0.0, 0.0));
    a.set_key_full(0, 0, pose(0.0), 1.0);
    a.set_key_full(0, 40, pose(4.0), 1.0);
    let before = a.sample(0, 29.0).unwrap();
    a.key_step(0, 30, pose(3.0), 1.0, pose(9.0), 0.0);
    let held = a.tracks[&0].key_at(29).expect("held on the frame before");
    assert!(held.pose.pos.distance(before.pos) < 1e-4);
    assert_eq!(a.sample(0, 29.5).unwrap().pos, held.pose.pos, "no drift inside the last frame");
    assert_eq!(a.sample(0, 30.0).unwrap().pos, Vec3::new(9.0, 0.0, 0.0));
    assert_eq!(a.tracks[&0].key_at(30).unwrap().value, 0.0);
}

#[test]
fn a_switch_on_the_first_frame_needs_nothing_before_it() {
    let mut a = Animation::default();
    a.key_step(3, 0, Xf::IDENTITY, 1.0, turned(1.0), 0.0);
    assert_eq!(a.tracks[&3].keys.len(), 1);
}

#[test]
fn later_keys_carry_a_switch_as_far_as_the_next_one() {
    let mut a = Animation::default();
    for (frame, value) in [(0, 1.0), (10, 1.0), (20, 1.0), (30, 0.0), (40, 0.0)] {
        a.set_key_full(0, frame, turned(0.1), value);
    }
    let change = Xf::new(Quat::IDENTITY, Vec3::X);
    a.carry_switch(0, 5, 1.0, 0.0, change);
    assert_eq!(a.tracks[&0].key_at(0).unwrap().value, 1.0, "before the switch");
    assert_eq!(a.tracks[&0].key_at(10).unwrap().value, 0.0);
    assert_eq!(a.tracks[&0].key_at(20).unwrap().pose.pos, Vec3::X);
    assert_eq!(a.tracks[&0].key_at(30).unwrap().pose.pos, Vec3::ZERO, "past the next switch");
}

#[test]
fn a_switch_to_fk_stays_off_through_the_targets_later_keys() {
    let mut a = Animation::default();
    let target = 0;
    a.set_key_full(target, 0, Xf::IDENTITY, 1.0);
    a.set_key_full(target, 40, Xf::IDENTITY, 1.0);
    a.key_step(target, 20, Xf::IDENTITY, 1.0, Xf::IDENTITY, 0.0);
    a.carry_switch(target, 20, 1.0, 0.0, Xf::IDENTITY);
    assert_eq!(a.tracks[&target].sample_full(30.0, a.smoothing).unwrap().1, 0.0);
    assert!((a.tracks[&target].sample_full(10.0, a.smoothing).unwrap().1 - 1.0).abs() < 1e-4);
}

#[test]
fn a_keyed_ik_switch_plays_back_on_both_sides() {
    let mut s = rig("R15Rig.rbxm");
    let mut a = Animation::default();
    let (index, target) = ik_on(&mut s, "LeftAnkle");
    let at = s.bones[target].head();
    a.set_key_full(target, 0, s.bones[target].pose, 1.0);
    place(&mut s, target, at + Vec3::new(0.0, 1.2, -0.7));
    a.set_key_full(target, 20, s.bones[target].pose, 1.0);
    a.apply(&mut s, 12.0);
    let ik_frame = parts(&s);
    a.apply(&mut s, 20.0);
    let switch_frame = parts(&s);

    // What the app does when the switch is clicked with Auto Key on.
    let old: Vec<(Xf, f32)> = s.bones.iter().map(|b| (b.pose, b.value)).collect();
    modifiers::switch_ik(&mut s, index, false);
    for (id, (pose, value)) in old.into_iter().enumerate() {
        if !s.bones[id].pose.approx_eq(pose) || s.bones[id].value != value {
            let (new_pose, new_value) = (s.bones[id].pose, s.bones[id].value);
            a.key_step(id, 20, pose, value, new_pose, new_value);
        }
    }
    a.apply(&mut s, 20.0);
    assert_same_parts(&switch_frame, &s, "on the switch frame");
    // Any new key reshapes the automatic curve into it by a hair, as inserting one always does.
    a.apply(&mut s, 12.0);
    assert_parts_within(&ik_frame, &s, "before the switch", 0.01, 0.9999);
    a.apply(&mut s, 60.0);
    assert_same_parts(&switch_frame, &s, "held after the switch");
}

// Spawned shapes.

#[test]
fn every_shape_spawns_as_something_to_move_and_key() {
    for shape in PartShape::ALL {
        let mut s = Scene::default();
        let at = Xf::new(Quat::IDENTITY, Vec3::new(1.0, 2.0, 3.0));
        let id = s.add_shape(shape, at, Vec3::splat(2.0));
        s.evaluate();
        let part = s.bones[id].part.unwrap();
        assert_eq!(s.parts[part].shape, shape);
        assert_eq!(s.parts[part].drive, Drive::Bone(id));
        assert_eq!(s.part_owner(part), Some(id));
        assert_eq!(s.parts[part].world.pos, at.pos);
        assert!(!s.bones[id].kind.is_joint(), "an export must skip shapes");
        s.set_bone_frame(id, Xf::new(Quat::from_rotation_y(0.5), Vec3::new(4.0, 0.0, 0.0)));
        assert_eq!(s.parts[part].world.pos, Vec3::new(4.0, 0.0, 0.0));
    }
}

#[test]
fn shapes_get_their_own_names() {
    let mut s = Scene::default();
    let names: Vec<String> = [PartShape::Block, PartShape::Block, PartShape::Ball]
        .into_iter()
        .map(|shape| {
            let id = s.add_shape(shape, Xf::IDENTITY, Vec3::ONE);
            s.bones[id].name.clone()
        })
        .collect();
    assert_eq!(names, ["Block", "Block 2", "Sphere"]);
}

#[test]
fn a_shape_can_be_held_by_a_rig() {
    let mut s = rig("R15Rig.rbxm");
    let at = s.bones[bone(&s, "RightWrist")].frame;
    let shape = s.add_shape(PartShape::Cylinder, at, Vec3::ONE);
    s.evaluate();
    let hand = bone(&s, "RightWrist");
    modifiers::set_follow(&mut s, shape, Some(hand));
    let part = s.bones[shape].part.unwrap();
    assert!(s.parts[part].world.pos.distance(at.pos) < 1e-3, "attaching moved it");
    let elbow = bone(&s, "RightElbow");
    s.set_bone_pose(elbow, turned(-1.0));
    assert!(s.parts[part].world.pos.distance(s.bones[hand].frame.pos) < 1e-2, "it should stay in the hand");
}

#[test]
fn undoing_a_shape_takes_it_away_and_redo_brings_it_back() {
    let mut s = Scene::default();
    let mut animation = Animation::default();
    let mut history = History::default();
    let before = (s.rig_state(), animation.clone());
    let id = s.add_shape(PartShape::Block, Xf::IDENTITY, Vec3::ONE);
    let after = (s.rig_state(), animation.clone());
    history.push_rig("Add shape", before, after);
    let part = s.bones[id].part.unwrap();
    s.evaluate();
    assert!(s.part_is_visible(part));
    history.undo(&mut s, &mut animation);
    assert!(!s.part_is_visible(part));
    assert_eq!(s.live_parts(), 0);
    history.redo(&mut s, &mut animation);
    assert!(s.part_is_visible(part));
}

#[test]
fn a_shapes_size_and_color_undo() {
    let mut s = Scene::default();
    let id = s.add_shape(PartShape::Block, Xf::IDENTITY, Vec3::ONE);
    let part = s.bones[id].part.unwrap();
    let state = s.rig_state();
    s.parts[part].size = Vec3::splat(5.0);
    s.parts[part].color = [1.0, 0.0, 0.0];
    s.restore_rig_state(&state);
    assert_eq!(s.parts[part].size, Vec3::ONE);
    assert_ne!(s.parts[part].color, [1.0, 0.0, 0.0]);
}

#[test]
fn shapes_layers_and_new_modifiers_survive_a_save() {
    let dir = std::env::temp_dir().join(format!("kaye-tools-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tools.kaye");
    let mut s = rig("R15Rig.rbxm");
    let shape = s.add_shape(PartShape::Wedge, Xf::IDENTITY, Vec3::new(1.0, 2.0, 3.0));
    let neck = bone(&s, "Neck");
    modifiers::add_look_at(&mut s, neck);
    let waist = bone(&s, "Waist");
    modifiers::add_copy(&mut s, waist);
    let index = s.modifiers.len() - 1;
    let root = bone(&s, "Root");
    modifiers::set_copy_source(&mut s, index, Some(root));
    let knee = bone(&s, "LeftKnee");
    modifiers::add_limit(&mut s, knee);
    let mut a = Animation::default();
    a.set_key(shape, 0, Xf::IDENTITY);
    a.add_layer();
    a.set_key(neck, 5, turned(0.4));
    a.layers[1].weight = 0.5;
    s.evaluate();
    let view = View { frame: 0.0, target: Vec3::ZERO, yaw: 0.0, pitch: 0.0, dist: 10.0, ortho: false, floor_y: 0.0, camera: None, preview: false, through: false };
    project::save(&path, &s, &a, &view).unwrap();
    let opened = project::load(&path).unwrap();
    assert_eq!(opened.animation, a);
    assert_eq!(opened.scene.modifiers, s.modifiers);
    let part = opened.scene.bones[shape].part.unwrap();
    assert_eq!(opened.scene.parts[part].size, Vec3::new(1.0, 2.0, 3.0));
    std::fs::remove_dir_all(&dir).ok();
}

// Copying and pasting.

#[test]
fn a_pose_pastes_back_onto_the_bones_it_came_from() {
    let mut s = rig("R15Rig.rbxm");
    let arm = [bone(&s, "LeftShoulder"), bone(&s, "LeftElbow")];
    s.bones[arm[0]].pose = turned(0.5);
    s.bones[arm[1]].pose = turned(-0.8);
    let copied = posing::copy_pose(&s, &arm);
    s.bones[arm[0]].pose = Xf::IDENTITY;
    s.bones[arm[1]].pose = Xf::IDENTITY;
    let poses = posing::paste_pose(&s, &copied, 0);
    assert_eq!(poses, vec![(arm[0], turned(0.5)), (arm[1], turned(-0.8))]);
}

#[test]
fn a_pose_finds_the_same_bones_on_another_rig() {
    let mut s = rig("R15Rig.rbxm");
    s.append(rig("R15Rig.rbxm"));
    s.evaluate();
    let first = s.rigs[0].bones.iter().copied().find(|b| s.bones[*b].name == "LeftKnee").unwrap();
    let second = s.rigs[1].bones.iter().copied().find(|b| s.bones[*b].name == "LeftKnee").unwrap();
    s.bones[first].pose = turned(0.7);
    let copied = posing::copy_pose(&s, &[first]);
    assert_eq!(posing::paste_pose(&s, &copied, 1), vec![(second, turned(0.7))]);
}

// Mirroring.

#[test]
fn names_find_the_other_side() {
    for (a, b) in [("LeftUpperArm", "RightUpperArm"), ("Right Hip", "Left Hip"), ("foot.L", "foot.R"), ("hand_r", "hand_l"), ("LeftAnkle IK", "RightAnkle IK")] {
        assert_eq!(posing::mirrored_name(a).as_deref(), Some(b));
        assert_eq!(posing::mirrored_name(b).as_deref(), Some(a));
    }
    assert_eq!(posing::mirrored_name("Neck"), None);
    assert_eq!(posing::mirrored_name("Root"), None);
}

/// Where a part would be seen in a mirror down the middle of the rig.
fn reflected(p: Vec3, center: f32) -> Vec3 {
    Vec3::new(center * 2.0 - p.x, p.y, p.z)
}

#[test]
fn a_mirrored_arm_lands_where_the_mirror_says_on_either_rig() {
    for (file, left, right, center) in [
        ("R15Rig.rbxm", ["LeftShoulder", "LeftElbow", "LeftWrist"].as_slice(), ["RightShoulder", "RightElbow", "RightWrist"].as_slice(), "HumanoidRootPart"),
        ("R6Rig.rbxm", ["Left Shoulder"].as_slice(), ["Right Shoulder"].as_slice(), "HumanoidRootPart"),
    ] {
        let mut s = rig(file);
        let root_x = s.parts.iter().find(|p| p.name == center).unwrap().world.pos.x;
        for (i, name) in left.iter().enumerate() {
            let id = bone(&s, name);
            s.bones[id].pose = Xf::new(Quat::from_euler(glam::EulerRot::XYZ, 0.4 + i as f32 * 0.2, -0.3, 0.5), Vec3::ZERO);
        }
        s.evaluate();
        let wanted: Vec<(BoneId, Vec3)> = left.iter().zip(right).map(|(l, r)| (bone(&s, r), reflected(s.bones[bone(&s, l)].tail(), root_x))).collect();
        let lefts: Vec<BoneId> = left.iter().map(|n| bone(&s, n)).collect();
        for (id, pose) in posing::mirror_pose(&s, &lefts) {
            s.bones[id].pose = pose;
        }
        s.evaluate();
        for (id, at) in wanted {
            let landed = s.bones[id].tail();
            assert!(landed.distance(at) < 0.05, "{file}: {} landed {} from its mirror image", s.bones[id].name, landed.distance(at));
        }
    }
}

#[test]
fn mirroring_a_whole_rig_twice_gets_it_back() {
    for file in ["R15Rig.rbxm", "R6Rig.rbxm"] {
        let mut s = rig(file);
        let all = s.joints(0);
        for (i, id) in all.iter().enumerate() {
            s.bones[*id].pose = Xf::new(Quat::from_rotation_y(i as f32 * 0.1) * Quat::from_rotation_z(0.2), Vec3::new(0.0, 0.05 * i as f32, 0.0));
        }
        s.evaluate();
        let posed = parts(&s);
        for _ in 0..2 {
            for (id, pose) in posing::mirror_pose(&s, &all) {
                s.bones[id].pose = pose;
            }
            s.evaluate();
        }
        assert_same_parts(&posed, &s, "mirrored twice");
    }
}

#[test]
fn a_center_bone_mirrors_onto_itself() {
    let mut s = rig("R15Rig.rbxm");
    let neck = bone(&s, "Neck");
    s.bones[neck].pose = Xf::new(Quat::from_rotation_y(0.6), Vec3::ZERO);
    s.evaluate();
    let looked = s.bones[neck].frame.rot * Vec3::NEG_Z;
    let (id, pose) = posing::mirror_pose(&s, &[neck])[0];
    assert_eq!(id, neck);
    s.bones[neck].pose = pose;
    s.evaluate();
    let now = s.bones[neck].frame.rot * Vec3::NEG_Z;
    assert!((now.x + looked.x).abs() < 1e-3 && (now.z - looked.z).abs() < 1e-3, "the head should turn the other way");
}

#[test]
fn ik_targets_mirror_across_too() {
    let mut s = rig("R15Rig.rbxm");
    let (_, left) = ik_on(&mut s, "LeftAnkle");
    let (_, right) = ik_on(&mut s, "RightAnkle");
    let at = s.bones[left].head();
    place(&mut s, left, at + Vec3::new(0.3, 1.0, -0.6));
    let root_x = s.parts.iter().find(|p| p.name == "HumanoidRootPart").unwrap().world.pos.x;
    let wanted = reflected(s.bones[left].head(), root_x);
    let poses = posing::mirror_pose(&s, &[left]);
    assert_eq!(poses[0].0, right);
    s.set_bone_pose(right, poses[0].1);
    assert!(s.bones[right].head().distance(wanted) < 1e-3);
}

// Baking.

#[test]
fn baked_ik_plays_the_same_with_nothing_solving_it() {
    for (file, tip) in [("R15Rig.rbxm", "LeftAnkle"), ("R6Rig.rbxm", "Left Hip")] {
        let mut s = rig(file);
        let mut a = Animation { end: 24, ..Animation::default() };
        let (index, target) = ik_on(&mut s, tip);
        let at = s.bones[target].head();
        a.set_key_full(target, 0, s.bones[target].pose, 1.0);
        place(&mut s, target, at + Vec3::new(0.4, 1.0, -0.8));
        a.set_key_full(target, 24, s.bones[target].pose, 1.0);
        let samples: Vec<Vec<Xf>> = (0..=24)
            .map(|f| {
                a.apply(&mut s, f as f32);
                parts(&s)
            })
            .collect();
        posing::bake(&mut s, &mut a, &[index]);
        assert!(s.modifiers.is_empty());
        assert!(!a.tracks.contains_key(&target), "the target's keys go with it");
        for (frame, shown) in samples.iter().enumerate() {
            a.apply(&mut s, frame as f32);
            assert_same_parts(shown, &s, &format!("{file} frame {frame}"));
        }
    }
}

#[test]
fn baked_unparent_plays_the_same() {
    let mut s = rig("R15Rig.rbxm");
    let mut a = Animation { end: 12, ..Animation::default() };
    let elbow = bone(&s, "LeftElbow");
    modifiers::add_unparent(&mut s, elbow);
    let shoulder = bone(&s, "LeftShoulder");
    a.set_key(shoulder, 0, Xf::IDENTITY);
    a.set_key(shoulder, 12, turned(1.0));
    let samples: Vec<Vec<Xf>> = (0..=12)
        .map(|f| {
            a.apply(&mut s, f as f32);
            parts(&s)
        })
        .collect();
    posing::bake(&mut s, &mut a, &[0]);
    assert!(s.modifiers.is_empty());
    for (frame, shown) in samples.iter().enumerate() {
        a.apply(&mut s, frame as f32);
        assert_same_parts(shown, &s, &format!("frame {frame}"));
    }
}

#[test]
fn baking_leaves_keys_outside_the_range() {
    let mut s = rig("R15Rig.rbxm");
    let mut a = Animation { end: 10, ..Animation::default() };
    let (index, _) = ik_on(&mut s, "LeftAnkle");
    let knee = bone(&s, "LeftKnee");
    a.set_key(knee, 50, turned(0.2));
    a.set_key(knee, 5, turned(0.1));
    posing::bake(&mut s, &mut a, &[index]);
    assert!(a.key_at(knee, 50).is_some());
    assert_eq!(a.tracks[&knee].keys.len(), 12, "frames 0 to 10, and the one outside");
}

// Layers.

#[test]
fn a_layer_adds_to_what_is_under_it() {
    let mut s = rig("R15Rig.rbxm");
    let knee = bone(&s, "LeftKnee");
    let mut a = Animation::default();
    a.set_key(knee, 0, turned(0.5));
    a.add_layer();
    assert_eq!(a.active_layer, 1);
    assert!(a.tracks.is_empty(), "a new layer starts with no keys");
    a.set_key(knee, 0, turned(0.2));
    a.apply(&mut s, 0.0);
    assert!(s.bones[knee].pose.approx_eq(turned(0.7)), "{:?}", s.bones[knee].pose);
}

#[test]
fn a_layers_weight_and_mute_scale_it() {
    let mut s = rig("R15Rig.rbxm");
    let knee = bone(&s, "LeftKnee");
    let mut a = Animation::default();
    a.set_key(knee, 0, turned(0.5));
    a.add_layer();
    a.set_key(knee, 0, turned(0.4));
    a.layers[1].weight = 0.5;
    a.apply(&mut s, 0.0);
    assert!(s.bones[knee].pose.approx_eq(turned(0.7)));
    a.layers[1].muted = true;
    a.apply(&mut s, 0.0);
    assert!(s.bones[knee].pose.approx_eq(turned(0.5)));
}

#[test]
fn a_pose_keyed_on_a_layer_shows_exactly_as_posed() {
    let mut s = rig("R15Rig.rbxm");
    let hip = bone(&s, "LeftHip");
    let mut a = Animation::default();
    a.set_key(hip, 0, turned(0.5));
    a.add_layer();
    a.layers[1].weight = 0.5;
    let wanted = Xf::new(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(0.9), Vec3::new(0.1, 0.0, 0.0));
    a.record_edit(hip, 0, turned(0.5), wanted, 1.0, false);
    a.apply(&mut s, 0.0);
    assert!(s.bones[hip].pose.pos.distance(wanted.pos) < 1e-4);
    assert!(s.bones[hip].pose.rot.dot(wanted.rot).abs() > 0.99999);
}

#[test]
fn editing_under_a_layer_still_shows_exactly_as_posed() {
    let mut s = rig("R15Rig.rbxm");
    let hip = bone(&s, "LeftHip");
    let mut a = Animation::default();
    a.set_key(hip, 0, Xf::IDENTITY);
    a.add_layer();
    a.set_key(hip, 0, Xf::new(Quat::from_rotation_y(0.3), Vec3::ZERO));
    a.set_active_layer(0);
    let wanted = Xf::new(Quat::from_rotation_x(0.9), Vec3::ZERO);
    a.record_edit(hip, 0, Xf::IDENTITY, wanted, 1.0, false);
    a.apply(&mut s, 0.0);
    assert!(s.bones[hip].pose.rot.dot(wanted.rot).abs() > 0.99999);
}

#[test]
fn a_bone_only_keyed_on_a_layer_keeps_its_pose_underneath() {
    let mut s = rig("R15Rig.rbxm");
    let hip = bone(&s, "LeftHip");
    s.set_bone_pose(hip, turned(0.6));
    let mut a = Animation::default();
    a.add_layer();
    a.record_edit(hip, 0, turned(0.6), turned(0.9), 1.0, false);
    for _ in 0..3 {
        a.apply(&mut s, 0.0);
    }
    assert!(s.bones[hip].pose.approx_eq(turned(0.9)), "applying must not add up");
    a.remove_layer(1);
    a.apply(&mut s, 0.0);
    assert!(s.bones[hip].pose.approx_eq(turned(0.6)), "the pose under the layer should be left");
}

/// Where everything sits on every frame, to compare before and after a merge.
fn played(s: &Scene, a: &Animation) -> Vec<Vec<Xf>> {
    let mut s = s.clone();
    (a.start..=a.end)
        .map(|frame| {
            a.apply(&mut s, frame as f32);
            s.parts.iter().map(|p| p.world).collect()
        })
        .collect()
}

fn assert_plays_the_same(s: &Scene, before: &[Vec<Xf>], a: &Animation, what: &str) {
    for (frame, (was, now)) in before.iter().zip(played(s, a)).enumerate() {
        for (part, (was, now)) in was.iter().zip(&now).enumerate() {
            assert!(was.pos.distance(now.pos) < 0.05, "{what}: {} moved {} on frame {frame}", s.parts[part].name, was.pos.distance(now.pos));
            assert!(was.rot.angle_between(now.rot).to_degrees() < 0.5, "{what}: {} turned on frame {frame}", s.parts[part].name);
        }
    }
}

fn layered_wave(s: &Scene) -> Animation {
    let (shoulder, elbow) = (bone(s, "RightShoulder"), bone(s, "RightElbow"));
    let mut a = Animation { end: 40, ..Animation::default() };
    a.set_key(shoulder, 0, Xf::IDENTITY);
    a.set_key(shoulder, 20, turned(1.2));
    a.set_key(shoulder, 40, Xf::IDENTITY);
    a.add_layer();
    a.set_key(shoulder, 7, turned(0.4));
    a.set_key(shoulder, 31, turned(-0.3));
    a.set_key(elbow, 12, Xf::new(Quat::from_rotation_z(0.6), Vec3::new(0.0, 0.1, 0.0)));
    a
}

#[test]
fn merging_a_layer_down_plays_the_same() {
    let s = rig("R15Rig.rbxm");
    let mut a = layered_wave(&s);
    let before = played(&s, &a);
    a.merge_down(1);
    assert!(a.layers.is_empty(), "back to a single layer");
    assert_eq!(a.active_layer, 0);
    assert_plays_the_same(&s, &before, &a, "merged");
    let shoulder = bone(&s, "RightShoulder");
    for frame in [0, 7, 20, 31, 40] {
        assert!(a.key_at(shoulder, frame).is_some(), "keeps the key on frame {frame}");
    }
    assert!(a.tracks[&shoulder].keys.len() < 41, "not one key per frame: {}", a.tracks[&shoulder].keys.len());
}

#[test]
fn merging_takes_the_weight_into_the_keys() {
    let s = rig("R15Rig.rbxm");
    let mut a = layered_wave(&s);
    a.layers[1].weight = 0.35;
    let before = played(&s, &a);
    a.merge_down(1);
    assert_plays_the_same(&s, &before, &a, "weighted");
}

#[test]
fn merging_a_hidden_layer_loses_it() {
    let s = rig("R15Rig.rbxm");
    let mut a = layered_wave(&s);
    a.layers[1].muted = true;
    let before = played(&s, &a);
    a.merge_down(1);
    assert_plays_the_same(&s, &before, &a, "hidden");
}

#[test]
fn merging_between_upper_layers_leaves_the_others_working() {
    let s = rig("R15Rig.rbxm");
    let (shoulder, waist) = (bone(&s, "RightShoulder"), bone(&s, "Waist"));
    let mut a = layered_wave(&s);
    a.layers[1].weight = 0.6;
    a.add_layer();
    a.set_key(shoulder, 15, turned(0.25));
    a.set_key(waist, 15, Xf::new(Quat::from_rotation_y(0.2), Vec3::ZERO));
    a.add_layer();
    a.set_key(shoulder, 25, turned(0.15));
    a.layers[3].weight = 0.5;
    assert_eq!(a.active_layer, 3);
    let before = played(&s, &a);
    a.merge_down(2);
    assert_eq!(a.layers.len(), 3);
    assert_eq!(a.active_layer, 2, "the top layer stays active");
    assert_eq!(a.layers[1].weight, 1.0, "the weight went into the keys");
    assert_plays_the_same(&s, &before, &a, "middle");
    a.merge_down(2);
    assert_plays_the_same(&s, &before, &a, "top");
    a.merge_down(1);
    assert!(a.layers.is_empty());
    assert_plays_the_same(&s, &before, &a, "all");
}

#[test]
fn merging_the_active_layer_lands_on_the_one_under_it() {
    let s = rig("R15Rig.rbxm");
    let mut a = layered_wave(&s);
    a.add_layer();
    a.set_key(bone(&s, "RightShoulder"), 3, turned(0.1));
    let before = played(&s, &a);
    a.merge_down(2);
    assert_eq!(a.active_layer, 1);
    assert_plays_the_same(&s, &before, &a, "active");
    assert!(a.tracks.contains_key(&bone(&s, "RightShoulder")), "its keys are the active ones now");
}

#[test]
fn merging_keeps_easing_where_it_was_set() {
    let s = rig("R15Rig.rbxm");
    let shoulder = bone(&s, "RightShoulder");
    let mut a = layered_wave(&s);
    a.set_active_layer(0);
    let easing = kaye::animation::Easing { style: kaye::animation::EaseStyle::Constant, ..Default::default() };
    a.tracks.get_mut(&shoulder).unwrap().keys[1].easing = easing;
    a.set_active_layer(1);
    let before = played(&s, &a);
    a.merge_down(1);
    assert_eq!(a.key_at(shoulder, 20).unwrap().easing.style, kaye::animation::EaseStyle::Constant);
    assert_plays_the_same(&s, &before, &a, "constant");
}

#[test]
fn the_base_layer_cant_be_merged_into_nothing() {
    let s = rig("R15Rig.rbxm");
    let mut a = layered_wave(&s);
    let before = a.clone();
    a.merge_down(0);
    a.merge_down(5);
    assert_eq!(a, before);
}

#[test]
fn removing_layers_leaves_the_base_keys() {
    let mut a = Animation::default();
    a.set_key(0, 0, turned(0.1));
    a.add_layer();
    a.add_layer();
    a.set_key(0, 1, turned(0.2));
    a.remove_layer(2);
    assert_eq!(a.active_layer, 1);
    a.remove_layer(1);
    assert_eq!(a.active_layer, 0);
    assert!(a.layers.is_empty());
    assert!(a.key_at(0, 0).is_some());
}

#[test]
fn switching_layers_keeps_each_ones_keys() {
    let mut a = Animation::default();
    a.set_key(0, 0, turned(0.1));
    a.add_layer();
    a.set_key(0, 7, turned(0.2));
    a.set_active_layer(0);
    assert!(a.key_at(0, 0).is_some() && a.key_at(0, 7).is_none());
    a.set_active_layer(1);
    assert!(a.key_at(0, 7).is_some() && a.key_at(0, 0).is_none());
}

#[test]
fn forgetting_a_bone_reaches_every_layer() {
    let mut a = Animation::default();
    a.set_key(4, 0, Xf::IDENTITY);
    a.add_layer();
    a.set_key(4, 0, Xf::IDENTITY);
    a.forget_bone(4);
    assert!(a.keyed_bones().is_empty());
}

// Limits.

#[test]
fn a_knee_never_swings_around_behind_a_limited_leg() {
    let mut s = rig("R15Rig.rbxm");
    let (_, target) = ik_on(&mut s, "LeftAnkle");
    let hip = s.bones[bone(&s, "LeftHip")].head();
    let knee = bone(&s, "LeftKnee");
    let reach = s.bones[target].head().distance(hip) * 0.8;
    let mut last: Option<f32> = None;
    // The target sweeps from below the hip, out in front, and up to hip height, which
    // is what flipped a leg with no limit.
    for step in 0..=60 {
        let angle = step as f32 / 60.0 * 1.7;
        place(&mut s, target, hip + Vec3::new(0.0, -angle.cos(), -angle.sin()) * reach);
        let at = s.bones[knee].head();
        let line = (s.bones[target].head() - hip).normalize();
        let off = (at - hip) - line * (at - hip).dot(line);
        // Measured against the way the leg faces at this step, forward and up.
        let forward = Vec3::new(0.0, angle.sin(), -angle.cos());
        assert!(off.dot(forward) > -1e-3, "the knee went behind at step {step}");
        if let Some(last) = last {
            assert!((off.length() - last).abs() < 0.2, "a jump at step {step}");
        }
        last = Some(off.length());
    }
}

#[test]
fn a_limit_stops_a_joint_folding_too_far() {
    let mut s = rig("R15Rig.rbxm");
    let (index, target) = ik_on(&mut s, "LeftAnkle");
    if let ModifierKind::Ik(ik) = &mut s.modifiers[index].kind {
        ik.limit = Some(90.0);
    }
    let hip = s.bones[bone(&s, "LeftHip")].head();
    place(&mut s, target, hip + Vec3::new(0.0, -0.1, 0.0));
    let (a, b, c) = (hip, s.bones[bone(&s, "LeftKnee")].head(), s.bones[bone(&s, "LeftAnkle")].head());
    let fold = (b - a).angle_between(c - b).to_degrees();
    assert!(fold <= 90.5, "folded {fold} degrees");
}

#[test]
fn a_limited_long_chain_stays_within_it() {
    let mut s = rig("R15Rig.rbxm");
    let wrist = bone(&s, "LeftWrist");
    let target = modifiers::add_ik(&mut s, wrist);
    let index = s.modifiers.len() - 1;
    if let ModifierKind::Ik(ik) = &mut s.modifiers[index].kind {
        ik.chain = 3;
        ik.limit = Some(70.0);
    }
    let chain = {
        let ModifierKind::Ik(ik) = s.modifiers[index].kind else { panic!() };
        modifiers::chain_of(&s, wrist, &ik)
    };
    let at = s.bones[chain[0]].head();
    place(&mut s, target, at + Vec3::new(0.3, 0.2, -0.1));
    let mut joints: Vec<Vec3> = chain.iter().map(|b| s.bones[*b].head()).collect();
    joints.push(s.bones[wrist].head());
    for w in joints.windows(3) {
        let fold = (w[1] - w[0]).angle_between(w[2] - w[1]).to_degrees();
        assert!(fold <= 71.0, "folded {fold}");
    }
}

#[test]
fn older_ik_with_no_limit_acts_as_it_did() {
    let mut s = rig("R15Rig.rbxm");
    let (index, target) = ik_on(&mut s, "LeftAnkle");
    if let ModifierKind::Ik(ik) = &mut s.modifiers[index].kind {
        ik.limit = None;
    }
    let hip = s.bones[bone(&s, "LeftHip")].head();
    place(&mut s, target, hip + Vec3::new(0.0, -0.3, 0.0));
    let end = s.bones[bone(&s, "LeftAnkle")].head();
    assert!(end.distance(hip + Vec3::new(0.0, -0.3, 0.0)) < 0.5, "with no limit it gets as close as the bones allow");
}

#[test]
fn a_new_limb_gets_a_limit_and_one_bone_has_no_use_for_it() {
    let mut s = rig("R15Rig.rbxm");
    let (index, _) = ik_on(&mut s, "LeftAnkle");
    let ModifierKind::Ik(ik) = s.modifiers[index].kind else { panic!() };
    assert_eq!(ik.end, IkEnd::Joint);
    assert_eq!(ik.limit, Some(150.0));
}

// The smaller modifiers.

#[test]
fn adding_look_at_moves_nothing_and_then_it_follows() {
    let mut s = rig("R15Rig.rbxm");
    let neck = bone(&s, "Neck");
    let still = parts(&s);
    let target = modifiers::add_look_at(&mut s, neck);
    assert_same_parts(&still, &s, "adding Look At");
    assert_eq!(modifiers::control_of(&s, neck), target);
    assert_eq!(modifiers::owner_of(&s, target), neck);
    let head = s.bones[neck].head();
    place(&mut s, target, head + Vec3::new(3.0, 1.0, -2.0));
    let ModifierKind::LookAt(look) = s.modifiers[0].kind else { panic!() };
    let facing = s.bones[neck].frame.rot * look.axis;
    let wanted = (s.bones[target].head() - head).normalize();
    assert!(facing.normalize().dot(wanted) > 0.9999);
    assert!(s.bones[neck].head().distance(head) < 1e-4, "it only turns");
}

#[test]
fn look_at_plays_back_as_a_real_joint() {
    let mut s = rig("R15Rig.rbxm");
    let neck = bone(&s, "Neck");
    let target = modifiers::add_look_at(&mut s, neck);
    let head = s.bones[neck].head();
    place(&mut s, target, head + Vec3::new(2.0, 0.0, -2.0));
    let mut plain = s.clone();
    plain.modifiers.clear();
    for (i, b) in s.bones.iter().enumerate() {
        plain.bones[i].pose = b.output();
    }
    plain.evaluate();
    assert_same_parts(&parts(&s), &plain, "after export");
}

#[test]
fn copy_bone_keeps_its_distance_and_follows() {
    let mut s = rig("R15Rig.rbxm");
    let (left, right) = (bone(&s, "LeftElbow"), bone(&s, "RightElbow"));
    let still = parts(&s);
    modifiers::add_copy(&mut s, right);
    modifiers::set_copy_source(&mut s, 0, Some(left));
    assert_same_parts(&still, &s, "picking a source");
    let rest = s.bones[right].frame.rot;
    s.set_bone_pose(left, turned(0.8));
    let turn = s.bones[right].frame.rot * rest.inverse();
    assert!(turn.dot(Quat::IDENTITY).abs() < 0.99, "it should have turned with the source");
}

#[test]
fn copy_bone_with_nothing_picked_does_nothing() {
    let mut s = rig("R15Rig.rbxm");
    let elbow = bone(&s, "RightElbow");
    let still = parts(&s);
    modifiers::add_copy(&mut s, elbow);
    assert_same_parts(&still, &s, "nothing picked");
}

#[test]
fn limit_rotation_stops_a_bone_at_its_range() {
    let mut s = rig("R15Rig.rbxm");
    let knee = bone(&s, "LeftKnee");
    modifiers::add_limit(&mut s, knee);
    let ModifierKind::Limit(limit) = &mut s.modifiers[0].kind else { panic!() };
    limit.axes[0] = true;
    limit.min[0] = -30.0;
    limit.max[0] = 10.0;
    s.set_bone_pose(knee, turned(90f32.to_radians()));
    assert!(s.bones[knee].pose.approx_eq(turned(90f32.to_radians())), "the key stays what was keyed");
    let angle = s.bones[knee].solved.rot.to_euler(glam::EulerRot::XYZ).0.to_degrees();
    assert!((angle - 10.0).abs() < 0.01, "stopped at {angle}");
    assert!((s.bones[knee].output().rot.to_euler(glam::EulerRot::XYZ).0.to_degrees() - 10.0).abs() < 0.01);
}

#[test]
fn limit_rotation_goes_after_ik() {
    let mut s = rig("R15Rig.rbxm");
    let (_, target) = ik_on(&mut s, "LeftAnkle");
    let knee = bone(&s, "LeftKnee");
    modifiers::add_limit(&mut s, knee);
    let ModifierKind::Limit(limit) = &mut s.modifiers[1].kind else { panic!() };
    limit.axes[0] = true;
    limit.min[0] = -5.0;
    limit.max[0] = 5.0;
    let at = s.bones[target].head();
    place(&mut s, target, at + Vec3::new(0.0, 1.0, -0.5));
    let angle = s.bones[knee].solved.rot.to_euler(glam::EulerRot::XYZ).0.to_degrees();
    assert!(angle.abs() <= 5.01, "IK bent it to {angle}");
}

#[test]
fn a_look_at_target_goes_with_its_modifier() {
    let mut s = rig("R15Rig.rbxm");
    let neck = bone(&s, "Neck");
    let target = modifiers::add_look_at(&mut s, neck);
    let gone = modifiers::remove(&mut s, 0);
    assert_eq!(gone, vec![target]);
    assert!(s.bones[target].removed);
}

#[test]
fn undo_puts_back_poses_rewritten_by_a_switch() {
    let mut s = rig("R15Rig.rbxm");
    let mut a = Animation::default();
    let (index, target) = ik_on(&mut s, "LeftAnkle");
    let at = s.bones[target].head();
    place(&mut s, target, at + Vec3::new(0.0, 1.0, -0.6));
    let posed = parts(&s);
    let before = (s.rig_state(), a.clone());
    let old: Vec<Xf> = s.bones.iter().map(|b| b.pose).collect();
    modifiers::switch_ik(&mut s, index, false);
    let changes = old.iter().enumerate().filter(|(i, p)| !s.bones[*i].pose.approx_eq(**p)).map(|(bone, before)| PoseChange { bone, before: *before, after: s.bones[bone].pose }).collect();
    let after = (s.rig_state(), a.clone());
    let mut history = History::default();
    history.push_all("Switch to FK", changes, before, after);
    history.undo(&mut s, &mut a);
    assert!(modifiers::is_on(&s, index));
    assert_same_parts(&posed, &s, "undone");
    let knee = bone(&s, "LeftKnee");
    assert!(s.bones[knee].pose.approx_eq(Xf::IDENTITY), "the pose goes back as well");
    history.redo(&mut s, &mut a);
    assert!(!modifiers::is_on(&s, index));
    assert_same_parts(&posed, &s, "redone");
}
