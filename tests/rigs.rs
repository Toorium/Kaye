//! Runs the importer and pose math against real rigs exported from Studio.

use std::path::Path;

use glam::{Quat, Vec3};
use kaye::animation::Animation;
use kaye::camera::OrbitCamera;
use kaye::gizmo::{AxisLock, MAX_WEIGHT, OpKind, Session, Space, Weights};
use kaye::history::{History, PoseChange};
use kaye::import::{import_bytes, import_file};
use kaye::math::Xf;
use kaye::scene::{BoneKind, Drive, RigKind, Scene};

fn load(name: &str) -> Scene {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(name);
    let imported = import_file(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(imported.warnings.is_empty(), "{name}: {:?}", imported.warnings);
    imported.scene
}

fn bone(s: &Scene, name: &str) -> usize {
    s.bones
        .iter()
        .position(|b| b.name == name)
        .unwrap_or_else(|| panic!("no bone {name}"))
}

fn part(s: &Scene, name: &str) -> usize {
    s.parts
        .iter()
        .position(|p| p.name == name)
        .unwrap_or_else(|| panic!("no part {name}"))
}

/// Solving joints from the root must land every part where Studio saved it.
fn assert_rest_matches_file(s: &Scene) {
    for p in &s.parts {
        let moved = p.world.pos.distance(p.file_cframe.pos);
        assert!(moved < 0.02, "{} moved {moved} studs at rest", p.name);
        assert!(
            p.world.rot.dot(p.file_cframe.rot).abs() > 0.999,
            "{} rotated at rest",
            p.name
        );
    }
}

#[test]
fn r6_rig_structure() {
    let s = load("R6Rig.rbxm");
    assert_eq!(s.rigs.len(), 1);
    assert_eq!(s.rigs[0].kind, RigKind::R6);
    assert_eq!(s.bones.len(), 6);
    assert_eq!(s.parts.len(), 7);
    assert!(s.bones.iter().all(|b| b.kind == BoneKind::Motor6D));
    assert_eq!(s.parts[part(&s, "HumanoidRootPart")].drive, Drive::Root);
    assert!(s.parts[part(&s, "HumanoidRootPart")].is_helper());

    let root = bone(&s, "RootJoint");
    assert_eq!(s.bones[root].parent, None);
    for child in ["Neck", "Left Shoulder", "Right Shoulder", "Left Hip", "Right Hip"] {
        assert_eq!(s.bones[bone(&s, child)].parent, Some(root), "{child}");
    }
    assert_rest_matches_file(&s);
}

#[test]
fn r15_rig_structure() {
    let s = load("R15Rig.rbxm");
    assert_eq!(s.rigs[0].kind, RigKind::R15);
    assert_eq!(s.bones.len(), 15);
    assert_eq!(s.parts.len(), 16);
    assert_eq!(s.bones[bone(&s, "Root")].parent, None);
    assert_eq!(s.bones[bone(&s, "Waist")].parent, Some(bone(&s, "Root")));
    assert_eq!(s.bones[bone(&s, "LeftElbow")].parent, Some(bone(&s, "LeftShoulder")));
    assert_eq!(s.bones[bone(&s, "RightAnkle")].parent, Some(bone(&s, "RightKnee")));
    let mesh_parts = s.parts.iter().filter(|p| p.mesh_asset.is_some()).count();
    assert!(mesh_parts >= 14, "only {mesh_parts} parts kept a mesh reference");
    assert_rest_matches_file(&s);
}

#[test]
fn accessories_are_welded_and_follow_their_body_part() {
    let mut s = load("R6WithAccessories.rbxm");
    assert_eq!(s.rigs[0].kind, RigKind::R6);
    assert_eq!(s.bones.len(), 6, "welds must not become bones");
    assert_rest_matches_file(&s);

    let neck = bone(&s, "Neck");
    let head = part(&s, "Head");
    let handles: Vec<usize> = (0..s.parts.len())
        .filter(|p| s.parts[*p].name == "Handle" && s.part_owner(*p) == Some(neck))
        .collect();
    assert!(!handles.is_empty(), "no accessory follows the neck");
    let offsets: Vec<f32> = handles
        .iter()
        .map(|h| s.parts[*h].world.pos.distance(s.parts[head].world.pos))
        .collect();

    let head_before = s.parts[head].world;
    s.set_bone_pose(neck, Xf::new(Quat::from_rotation_y(1.0), Vec3::ZERO));
    assert!(!s.parts[head].world.approx_eq(head_before), "head did not move");
    for (h, offset) in handles.iter().zip(offsets) {
        let now = s.parts[*h].world.pos.distance(s.parts[head].world.pos);
        assert!((now - offset).abs() < 1e-3, "accessory drifted from the head");
    }
}

#[test]
fn rotating_a_joint_pivots_around_it_and_carries_children() {
    let mut s = load("R15Rig.rbxm");
    let shoulder = bone(&s, "RightShoulder");
    let pivot = s.bones[shoulder].head();
    let hand = part(&s, "RightHand");
    let hand_before = s.parts[hand].world.pos;
    let torso_before = s.parts[part(&s, "UpperTorso")].world;

    let frame = s.bones[shoulder].frame;
    let turned = Xf::new(Quat::from_rotation_z(1.2) * frame.rot, frame.pos);
    s.set_bone_frame(shoulder, turned);

    assert!(s.bones[shoulder].head().distance(pivot) < 1e-4, "pivot moved");
    assert!(s.parts[hand].world.pos.distance(hand_before) > 0.5, "hand did not follow");
    assert!(s.parts[part(&s, "UpperTorso")].world.approx_eq(torso_before), "parent moved");

    // Distance across the rotated chain stays rigid.
    let d0 = hand_before.distance(pivot);
    let d1 = s.parts[hand].world.pos.distance(pivot);
    assert!((d0 - d1).abs() < 1e-3);

    // The stored pose reproduces the same frame, which is what keyframes will save.
    let got = s.bones[shoulder].base * s.bones[shoulder].pose;
    assert!(got.approx_eq(turned));

    s.set_bone_pose(shoulder, Xf::IDENTITY);
    assert_rest_matches_file(&s);
}

#[test]
fn moving_a_joint_translates_it_exactly() {
    let mut s = load("R6Rig.rbxm");
    let neck = bone(&s, "Neck");
    let frame = s.bones[neck].frame;
    let target = Xf::new(frame.rot, frame.pos + Vec3::new(0.5, 1.0, -0.25));
    let head = part(&s, "Head");
    let before = s.parts[head].world.pos;
    s.set_bone_frame(neck, target);
    let moved = s.parts[head].world.pos - before;
    assert!(moved.distance(Vec3::new(0.5, 1.0, -0.25)) < 1e-3, "{moved}");
}

#[test]
fn undo_and_redo_restore_poses() {
    let mut s = load("R6Rig.rbxm");
    let mut h = History::default();
    let neck = bone(&s, "Neck");
    let after = Xf::new(Quat::from_rotation_x(0.7), Vec3::ZERO);
    let rest = s.parts[part(&s, "Head")].world;
    s.set_bone_pose(neck, after);
    let posed = s.parts[part(&s, "Head")].world;
    h.push(
        "Rotate",
        vec![PoseChange {
            bone: neck,
            before: Xf::IDENTITY,
            after,
        }],
    );

    let mut animation = Animation::default();
    h.undo(&mut s, &mut animation).unwrap();
    assert!(s.parts[part(&s, "Head")].world.approx_eq(rest));
    h.redo(&mut s, &mut animation).unwrap();
    assert!(s.parts[part(&s, "Head")].world.approx_eq(posed));
}

#[test]
fn bones_have_visible_length_and_point_down_limbs() {
    for file in ["R6Rig.rbxm", "R15Rig.rbxm", "R6WithAccessories.rbxm"] {
        let s = load(file);
        for b in &s.bones {
            assert!(b.tail_local.length() >= 0.19, "{file}: {} is too short", b.name);
        }
    }
    let s = load("R6Rig.rbxm");
    let shoulder = &s.bones[bone(&s, "Right Shoulder")];
    assert!(shoulder.tail().y < shoulder.head().y - 0.5, "arm bone should run down the arm");
}

#[test]
fn wires_attach_where_a_skeleton_would_and_stay_there() {
    let mut s = load("R6Rig.rbxm");
    let (neck, shoulder, hip) = (bone(&s, "Neck"), bone(&s, "Left Shoulder"), bone(&s, "Left Hip"));
    assert!(s.bones[neck].link_to_parent_tail, "neck sits on the top of the torso bone");
    assert!(s.bones[shoulder].link_to_parent_tail, "shoulders hang from the top");
    assert!(!s.bones[hip].link_to_parent_tail, "hips hang from the torso's center");

    // Posing moves the bones, but must never change where a wire attaches.
    let root = bone(&s, "RootJoint");
    s.set_bone_pose(root, Xf::new(Quat::from_rotation_x(2.5), Vec3::new(1.0, 2.0, 3.0)));
    assert!(s.bones[shoulder].link_to_parent_tail);
    assert!(!s.bones[hip].link_to_parent_tail);
}

#[test]
fn appending_a_second_file_keeps_both_rigs_working() {
    let mut s = load("R6Rig.rbxm");
    let (parts, bones) = (s.parts.len(), s.bones.len());
    let second = load("R15Rig.rbxm");
    let elbow = bones + bone(&second, "LeftElbow");
    s.append(second);
    s.evaluate();
    assert_eq!(s.rigs.len(), 2);
    assert_eq!(s.parts.len(), parts + 16);
    assert_rest_matches_file(&s);

    assert_eq!(s.bones[elbow].rig, 1);
    let hand = part(&s, "LeftHand");
    let before = s.parts[hand].world.pos;
    s.set_bone_pose(elbow, Xf::new(Quat::from_rotation_x(1.0), Vec3::ZERO));
    assert!(s.parts[hand].world.pos.distance(before) > 0.2);
}

#[test]
fn rejects_files_that_are_not_roblox_models() {
    assert!(import_bytes(b"not a model", "x.rbxm").is_err());
    assert!(import_bytes(b"<roblox!broken", "x.rbxm").is_err());
}

#[test]
fn opens_a_binary_file_even_with_the_rblxm_extension() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/R6Rig.rbxm");
    let bytes = std::fs::read(path).unwrap();
    let s = import_bytes(&bytes, "rig.rblxm").unwrap().scene;
    assert_eq!(s.bones.len(), 6);
}

fn test_rect() -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 700.0))
}

#[test]
fn dragging_several_bones_turns_each_around_its_own_joint() {
    let mut s = load("R15Rig.rbxm");
    let (left, right) = (bone(&s, "LeftShoulder"), bone(&s, "RightShoulder"));
    let (left_pivot, right_pivot) = (s.bones[left].head(), s.bones[right].head());
    let camera = OrbitCamera::default();
    let (left_pose, right_pose) = (s.bones[left].pose, s.bones[right].pose);

    let start = egui::pos2(600.0, 200.0);
    let mut session = Session::new(&s, left, OpKind::Rotate, None, true, start).with_extras(&s, &[left, right]);
    assert_eq!(session.extras.len(), 1);
    session.update(&mut s, &camera, test_rect(), egui::pos2(450.0, 120.0));

    assert!(s.bones[left].head().distance(left_pivot) < 1e-3, "left pivot held");
    assert!(s.bones[right].head().distance(right_pivot) < 1e-3, "right pivot held");
    assert!(!s.bones[left].pose.approx_eq(left_pose), "left turned");
    assert!(!s.bones[right].pose.approx_eq(right_pose), "right turned");
    let (left_turn, right_turn) = (
        s.bones[left].frame.rot * Quat::IDENTITY,
        s.bones[right].frame.rot * Quat::IDENTITY,
    );
    assert!(left_turn.angle_between(right_turn) < 1e-3, "both shoulders start square, so they stay matched");

    session.cancel(&mut s);
    assert!(s.bones[left].pose.approx_eq(left_pose));
    assert!(s.bones[right].pose.approx_eq(right_pose));
}

#[test]
fn a_selected_child_follows_its_parent_instead_of_moving_twice() {
    let s = load("R15Rig.rbxm");
    let (shoulder, elbow, wrist, hip) = (
        bone(&s, "LeftShoulder"),
        bone(&s, "LeftElbow"),
        bone(&s, "LeftWrist"),
        bone(&s, "LeftHip"),
    );
    let session = Session::new(&s, shoulder, OpKind::Rotate, None, true, egui::pos2(0.0, 0.0))
        .with_extras(&s, &[shoulder, elbow, wrist, hip]);
    let extras: Vec<usize> = session.extras.iter().map(|e| e.0).collect();
    assert_eq!(extras, [hip], "descendants follow the parent, unrelated bones move");

    // The same holds when the active bone is the child.
    let session = Session::new(&s, wrist, OpKind::Rotate, None, true, egui::pos2(0.0, 0.0)).with_extras(&s, &[wrist, shoulder]);
    assert!(session.extras.is_empty());
}

/// Runs one G or R drag on the right elbow, a quarter turn around it on screen, and
/// gives back where the bone began and ended.
fn drag_with(weights: Weights, op: OpKind, lock: Option<AxisLock>, trackball: bool) -> (Xf, Xf, Session) {
    let mut s = load("R15Rig.rbxm");
    let elbow = bone(&s, "RightElbow");
    let before = s.bones[elbow].frame;
    let mut camera = OrbitCamera::default();
    camera.target = before.pos;
    let center = camera.world_to_screen(before.pos, test_rect()).unwrap();
    let start = center + egui::vec2(120.0, 0.0);
    let mut session = Session::new(&s, elbow, op, lock, true, start).with_weights(weights);
    if trackball {
        session.press_rotate();
    }
    session.update(&mut s, &camera, test_rect(), center + egui::vec2(0.0, -120.0));
    (before, s.bones[elbow].frame, session)
}

fn turn(before: Xf, after: Xf) -> f32 {
    before.rot.angle_between(after.rot)
}

#[test]
fn grab_weight_moves_a_bone_part_of_the_way() {
    let half = Weights { grab: 0.5, ..Weights::default() };
    for lock in [None, Some(AxisLock { axis: 0, space: Space::Global })] {
        let (before, full, _) = drag_with(Weights::default(), OpKind::Move, lock, false);
        let (_, weighted, session) = drag_with(half, OpKind::Move, lock, false);
        let (full, weighted) = (full.pos - before.pos, weighted.pos - before.pos);
        assert!(full.length() > 0.5, "the drag moved the bone {full}");
        assert!(weighted.distance(full * 0.5) < 1e-3, "{lock:?}: {weighted} is not half of {full}");
        assert!((session.offset.length() - weighted.length()).abs() < 1e-3, "the readout shows the real move");
    }
}

#[test]
fn rotate_weights_turn_a_bone_part_of_the_way() {
    let quarter = Weights { rotate: 0.75, free_rotate: 0.75, ..Weights::default() };
    for trackball in [false, true] {
        let (before, full, _) = drag_with(Weights::default(), OpKind::Rotate, None, trackball);
        let (_, weighted, session) = drag_with(quarter, OpKind::Rotate, None, trackball);
        let (full, weighted) = (turn(before, full), turn(before, weighted));
        assert!(full > 0.1, "the drag turned the bone {full}");
        assert!((weighted - full * 0.25).abs() < 2e-3, "trackball {trackball}: {weighted} is not a quarter of {full}");
        if !trackball {
            assert!((session.angle.abs() - weighted).abs() < 2e-3, "the readout shows the real turn");
        }
    }
}

#[test]
fn each_weight_only_slows_its_own_drag() {
    let grab_only = Weights { grab: 0.9, ..Weights::default() };
    let (before, full, _) = drag_with(Weights::default(), OpKind::Rotate, None, false);
    let (_, same, _) = drag_with(grab_only, OpKind::Rotate, None, false);
    assert!((turn(before, full) - turn(before, same)).abs() < 2e-3);
}

#[test]
fn a_full_weight_still_lets_a_bone_move() {
    let (before, full, _) = drag_with(Weights::default(), OpKind::Move, None, false);
    let (_, heaviest, _) = drag_with(Weights { grab: 5.0, ..Weights::default() }, OpKind::Move, None, false);
    let (full, heaviest) = ((full.pos - before.pos).length(), (heaviest.pos - before.pos).length());
    assert!((heaviest - full * (1.0 - MAX_WEIGHT)).abs() < 1e-3, "{heaviest} against {full}");
}
