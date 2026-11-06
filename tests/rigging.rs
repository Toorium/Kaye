//! The Rigging tab's logic: joining parts with Motor6Ds, moving joints between parts, pivots,
//! moving parts at rest, attachments, undo, and writing all of it back into the model.

use std::path::Path;

use glam::{Quat, Vec3};
use kaye::animation::Animation;
use kaye::export;
use kaye::import::{import_bytes, import_dom, import_file};
use kaye::math::Xf;
use kaye::rigging;
use kaye::scene::{BoneKind, BoneLink, Drive, RigKind, Scene};
use rbx_dom_weak::{InstanceBuilder, WeakDom};
use rbx_types::{CFrame, Matrix3, Variant, Vector3};
use ustr::ustr;

fn sample(name: &str) -> Scene {
    let mut scene = import_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(name)).unwrap().scene;
    scene.at_rest.0 = true;
    scene.evaluate();
    scene
}

fn part(s: &Scene, name: &str) -> usize {
    s.parts.iter().position(|p| p.name == name && !p.removed).unwrap_or_else(|| panic!("no part {name}"))
}

fn bone(s: &Scene, name: &str) -> usize {
    s.bones.iter().position(|b| b.name == name && !b.removed).unwrap_or_else(|| panic!("no bone {name}"))
}

fn at(x: f32, y: f32, z: f32) -> CFrame {
    CFrame::new(Vector3::new(x, y, z), Matrix3::identity())
}

/// A model of three loose parts, with no joints at all: a Body with a Head above it and an Arm
/// beside it.
fn loose_parts() -> Vec<u8> {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let model = dom.insert(root, InstanceBuilder::new("Model").with_name("Figure"));
    for (name, spot) in [("Body", at(0.0, 0.0, 0.0)), ("Head", at(0.0, 2.0, 0.0)), ("Arm", at(1.5, 0.0, 0.0))] {
        dom.insert(model, InstanceBuilder::new("Part").with_name(name).with_property("CFrame", spot).with_property("Size", Vector3::new(1.0, 2.0, 1.0)));
    }
    let mut bytes = Vec::new();
    rbx_binary::to_writer(&mut bytes, &dom, &[model]).unwrap();
    bytes
}

fn at_rest(mut scene: Scene) -> Scene {
    scene.at_rest.0 = true;
    scene.evaluate();
    scene
}

#[test]
fn import_reads_attachments_and_finds_each_joints_instance() {
    let s = sample("R15Rig.rbxm");
    assert!(s.rigs[0].prepared);
    assert!(s.attachments.iter().any(|a| a.name == "RightShoulderRigAttachment" && s.parts[a.part].name == "RightUpperArm"));
    for b in s.joints(0) {
        assert!(s.bones[b].source.is_some(), "{} has no instance", s.bones[b].name);
    }
}

#[test]
fn joining_loose_parts_makes_motor6ds_that_turn_at_the_child() {
    let mut s = at_rest(import_bytes(&loose_parts(), "figure.rbxm").unwrap().scene);
    assert_eq!(s.rigs[0].kind, RigKind::Static);
    assert!(s.rigs[0].model.is_some(), "a small model keeps its file so it can be rigged");
    let (body, head, arm) = (part(&s, "Body"), part(&s, "Head"), part(&s, "Arm"));
    let (head_at, arm_at) = (s.parts[head].world, s.parts[arm].world);
    let joints = rigging::join(&mut s, body, &[head, arm]).unwrap();
    assert_eq!(joints.len(), 2);
    assert_eq!(s.rigs[0].kind, RigKind::Custom);
    assert!(s.parts[head].world.approx_eq(head_at) && s.parts[arm].world.approx_eq(arm_at), "joining moves nothing");
    let shoulder = joints[1];
    assert_eq!(s.bones[shoulder].kind, BoneKind::Motor6D);
    assert!(rigging::pivot(&s, shoulder).unwrap().pos.distance(arm_at.pos) < 1e-5, "the pivot starts in the arm's middle");
    // Posed, the arm turns about its own middle.
    s.at_rest.0 = false;
    s.set_bone_pose(shoulder, Xf::new(Quat::from_rotation_z(1.0), Vec3::ZERO));
    assert!(s.parts[arm].world.pos.distance(arm_at.pos) < 1e-5);
}

#[test]
fn joining_refuses_a_loop() {
    let mut s = sample("R15Rig.rbxm");
    let (torso, hand) = (part(&s, "UpperTorso"), part(&s, "RightHand"));
    assert!(rigging::join(&mut s, hand, &[torso]).is_err());
}

#[test]
fn joining_a_jointed_part_moves_its_joint_and_keeps_it() {
    let mut s = sample("R15Rig.rbxm");
    let elbow = bone(&s, "RightElbow");
    let (left, lower) = (part(&s, "LeftUpperArm"), part(&s, "RightLowerArm"));
    let lower_at = s.parts[lower].world;
    let pivot = rigging::pivot(&s, elbow).unwrap();
    let joints = rigging::join(&mut s, left, &[lower]).unwrap();
    assert_eq!(joints, [elbow], "the same joint, so its keys still fit");
    assert!(matches!(s.bones[elbow].link, BoneLink::Joint { part0, .. } if part0 == left));
    assert_eq!(s.bones[elbow].parent, Some(bone(&s, "LeftShoulder")));
    assert!(s.parts[lower].world.approx_eq(lower_at));
    assert!(rigging::pivot(&s, elbow).unwrap().approx_eq(pivot), "the pivot stays on the lower arm");
}

#[test]
fn moving_a_pivot_moves_no_part() {
    let mut s = sample("R15Rig.rbxm");
    let elbow = bone(&s, "RightElbow");
    let worlds: Vec<Xf> = s.parts.iter().map(|p| p.world).collect();
    let moved = Xf::new(Quat::from_rotation_y(0.4), rigging::pivot(&s, elbow).unwrap().pos + Vec3::new(0.0, -0.3, 0.2));
    rigging::set_pivot(&mut s, elbow, moved);
    assert!(rigging::pivot(&s, elbow).unwrap().approx_eq(moved));
    for (p, was) in s.parts.iter().zip(&worlds) {
        assert!(p.world.approx_eq(*was), "{} moved", p.name);
    }
    assert!(s.rigs[0].rigged);
}

#[test]
fn moving_a_part_brings_what_hangs_below() {
    let mut s = sample("R15Rig.rbxm");
    let (upper, hand, torso) = (part(&s, "RightUpperArm"), part(&s, "RightHand"), part(&s, "UpperTorso"));
    let (hand_at, torso_at) = (s.parts[hand].world, s.parts[torso].world);
    let shift = Vec3::new(0.5, 0.25, 0.0);
    let target = Xf::new(s.parts[upper].world.rot, s.parts[upper].world.pos + shift);
    rigging::set_part(&mut s, upper, target);
    assert!(s.parts[upper].world.approx_eq(target));
    assert!(s.parts[hand].world.pos.distance(hand_at.pos + shift) < 1e-4, "the hand came along");
    assert!(s.parts[torso].world.approx_eq(torso_at), "what holds it didn't move");
}

#[test]
fn attachments_follow_their_part_and_can_be_moved() {
    let mut s = sample("R6Rig.rbxm");
    let arm = part(&s, "Right Arm");
    let index = rigging::add_attachment(&mut s, arm);
    let second = rigging::add_attachment(&mut s, arm);
    assert_eq!((s.attachments[index].name.as_str(), s.attachments[second].name.as_str()), ("Attachment", "Attachment2"));
    let spot = Xf::new(Quat::IDENTITY, s.parts[arm].world.pos + Vec3::new(0.0, -1.0, 0.0));
    rigging::set_attachment_frame(&mut s, index, spot);
    assert!(rigging::attachment_frame(&s, index).approx_eq(spot));
    assert!(s.attachments[index].local.pos.distance(Vec3::new(0.0, -1.0, 0.0)) < 1e-5);
}

#[test]
fn rigging_undoes_through_rig_state() {
    let mut s = at_rest(import_bytes(&loose_parts(), "figure.rbxm").unwrap().scene);
    let before = s.rig_state();
    let (body, arm) = (part(&s, "Body"), part(&s, "Arm"));
    let joint = rigging::join(&mut s, body, &[arm]).unwrap()[0];
    rigging::add_attachment(&mut s, arm);
    assert_ne!(s.rig_state(), before);
    s.restore_rig_state(&before);
    s.evaluate();
    assert!(s.bones[joint].removed);
    assert_eq!(s.parts[arm].drive, Drive::Root);
    assert!(s.attachments.is_empty());
    assert!(!s.rigs[0].rigged);
}

#[test]
fn at_rest_shows_joints_unposed_and_leaves_poses_alone() {
    let mut s = sample("R15Rig.rbxm");
    let elbow = bone(&s, "RightElbow");
    let hand = part(&s, "RightHand");
    let rest = s.parts[hand].world;
    s.at_rest.0 = false;
    s.set_bone_pose(elbow, Xf::new(Quat::from_rotation_x(1.2), Vec3::ZERO));
    let posed = s.parts[hand].world;
    assert!(!posed.approx_eq(rest));
    s.at_rest.0 = true;
    s.evaluate();
    assert!(s.parts[hand].world.approx_eq(rest));
    s.at_rest.0 = false;
    s.evaluate();
    assert!(s.parts[hand].world.approx_eq(posed), "the pose was kept");
}

/// Exports the rig's model and imports it again.
fn round_trip(s: &Scene) -> Scene {
    let bytes = export::model_file(s, &Animation::default(), 0).unwrap();
    let mut back = import_bytes(&bytes, "back.rbxm").unwrap().scene;
    back.at_rest.0 = true;
    back.evaluate();
    back
}

#[test]
fn a_rigged_model_exports_its_new_joints() {
    let mut s = at_rest(import_bytes(&loose_parts(), "figure.rbxm").unwrap().scene);
    let (body, head, arm) = (part(&s, "Body"), part(&s, "Head"), part(&s, "Arm"));
    let joints = rigging::join(&mut s, body, &[head, arm]).unwrap();
    let pivot = Xf::new(Quat::IDENTITY, Vec3::new(1.0, 1.0, 0.0));
    rigging::set_pivot(&mut s, joints[1], pivot);
    let attachment = rigging::add_attachment(&mut s, head);
    rigging::set_attachment_frame(&mut s, attachment, Xf::new(Quat::IDENTITY, Vec3::new(0.0, 2.5, 0.0)));
    let back = round_trip(&s);
    assert_eq!(back.rigs[0].kind, RigKind::Custom);
    let shoulder = bone(&back, "Arm");
    assert_eq!(back.bones[shoulder].kind, BoneKind::Motor6D);
    assert!(rigging::pivot(&back, shoulder).unwrap().approx_eq(pivot), "the pivot came back");
    for name in ["Body", "Head", "Arm"] {
        assert!(back.parts[part(&back, name)].world.approx_eq(s.parts[part(&s, name)].world), "{name} moved");
    }
    let a = back.attachments.iter().find(|a| a.name == "Attachment").expect("the attachment was saved");
    assert_eq!(back.parts[a.part].name, "Head");
    assert!(a.local.pos.distance(Vec3::new(0.0, 0.5, 0.0)) < 1e-5);
}

#[test]
fn a_deleted_joint_and_attachment_leave_the_model() {
    let mut s = sample("R15Rig.rbxm");
    let elbow = bone(&s, "RightElbow");
    kaye::modifiers::delete_bones(&mut s, &[elbow]);
    let shoulder_attachment = s.attachments.iter().position(|a| a.name == "RightShoulderRigAttachment").unwrap();
    s.attachments[shoulder_attachment].removed = true;
    s.rigs[0].rigged = true;
    let back = round_trip(&s);
    assert!(!back.bones.iter().any(|b| b.name == "RightElbow"), "the Motor6D went");
    assert!(back.bones.iter().any(|b| b.name == "RightWrist"), "the rest stayed");
    let lower = part(&back, "RightLowerArm");
    assert_eq!(back.parts[lower].drive, Drive::Root, "the lower arm is on its own");
    // R15 has one on each side of the shoulder, and only one went.
    let count = |scene: &Scene| scene.attachments.iter().filter(|a| a.name == "RightShoulderRigAttachment" && !a.removed).count();
    assert_eq!(count(&back), count(&s));
    assert_eq!(count(&back), 1);
    assert_eq!(back.parts.iter().filter(|p| !p.removed).count(), s.parts.iter().filter(|p| !p.removed).count());
}

#[test]
fn moving_a_joint_to_another_part_replaces_the_files_weld() {
    // A Handle welded to a Body, which then gets a Motor6D to the Head instead.
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let model = dom.insert(root, InstanceBuilder::new("Model").with_name("Figure"));
    let body = dom.insert(model, InstanceBuilder::new("Part").with_name("Body").with_property("CFrame", at(0.0, 0.0, 0.0)).with_property("Size", Vector3::new(1.0, 2.0, 1.0)));
    let head = dom.insert(model, InstanceBuilder::new("Part").with_name("Head").with_property("CFrame", at(0.0, 2.0, 0.0)).with_property("Size", Vector3::new(1.0, 1.0, 1.0)));
    let handle = dom.insert(model, InstanceBuilder::new("Part").with_name("Handle").with_property("CFrame", at(0.0, 3.0, 0.0)).with_property("Size", Vector3::new(1.0, 1.0, 1.0)));
    for (a, b, name) in [(body, head, "Neck"), (body, handle, "HandleWeld")] {
        let class = if name == "Neck" { "Motor6D" } else { "Weld" };
        let joint = dom.insert(a, InstanceBuilder::new(class).with_name(name).with_property("C0", at(0.0, 0.0, 0.0)).with_property("C1", at(0.0, 0.0, 0.0)));
        let inst = dom.get_by_ref_mut(joint).unwrap();
        inst.properties.insert(ustr("Part0"), Variant::Ref(a));
        inst.properties.insert(ustr("Part1"), Variant::Ref(b));
        inst.properties.insert(ustr("C1"), Variant::CFrame(dom_offset(name)));
    }
    let mut s = at_rest(import_dom(&dom, "figure.rbxm").unwrap().scene);
    let (head_id, handle_id) = (part(&s, "Head"), part(&s, "Handle"));
    assert!(matches!(s.parts[handle_id].drive, Drive::Rigid { .. }));
    rigging::join(&mut s, head_id, &[handle_id]).unwrap();
    let back = round_trip(&s);
    let handle_back = part(&back, "Handle");
    assert!(matches!(back.parts[handle_back].drive, Drive::Bone(b) if back.bones[b].kind == BoneKind::Motor6D), "{:?}", back.parts[handle_back].drive);
    assert!(back.parts[handle_back].world.approx_eq(s.parts[handle_id].world));
}

/// Offsets that put each part where its CFrame says.
fn dom_offset(name: &str) -> CFrame {
    match name {
        "Neck" => at(0.0, -2.0, 0.0),
        _ => at(0.0, -3.0, 0.0),
    }
}

#[test]
fn a_copy_never_shows_at_rest() {
    // Exports, playback and replays work on copies, which must play the animation even with
    // the Rigging tab open.
    let mut s = sample("R15Rig.rbxm");
    let elbow = bone(&s, "RightElbow");
    s.bones[elbow].pose = Xf::new(Quat::from_rotation_x(1.0), Vec3::ZERO);
    let mut copy = s.clone();
    assert!(s.at_rest.0 && !copy.at_rest.0);
    copy.evaluate();
    s.evaluate();
    let hand = part(&s, "RightHand");
    assert!(!copy.parts[hand].world.approx_eq(s.parts[hand].world), "the copy is posed");
}
