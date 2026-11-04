//! Every way Roblox joins one part to another, built in code: joint instances, constraints
//! between parts or attachments, attachments on bones, and accessories with no joint.
//! Whatever holds a part, it has to stand where the file saved it and move with its limb.

use glam::{Quat, Vec3};
use kaye::import::import_dom;
use kaye::math::Xf;
use kaye::scene::{BoneKind, Drive, Scene};
use rbx_dom_weak::{InstanceBuilder, WeakDom};
use rbx_types::{CFrame, Matrix3, Ref, Variant, Vector3};
use ustr::ustr;

fn at(x: f32, y: f32, z: f32) -> CFrame {
    CFrame::new(Vector3::new(x, y, z), Matrix3::identity())
}

fn part(name: &str, position: CFrame) -> InstanceBuilder {
    InstanceBuilder::new("Part").with_name(name).with_property("CFrame", position).with_property("Size", Vector3::new(1.0, 2.0, 1.0))
}

fn set(dom: &mut WeakDom, on: Ref, name: &str, value: Variant) {
    dom.get_by_ref_mut(on).unwrap().properties.insert(ustr(name), value);
}

fn attachment(dom: &mut WeakDom, on: Ref, name: &str, spot: CFrame) -> Ref {
    dom.insert(on, InstanceBuilder::new("Attachment").with_name(name).with_property("CFrame", spot))
}

/// A joint instance, or `WeldConstraint`, naming two parts.
fn join_parts(dom: &mut WeakDom, class: &str, a: Ref, b: Ref, c0: CFrame, c1: CFrame) -> Ref {
    let joint = dom.insert(a, InstanceBuilder::new(class).with_name(class).with_property("C0", c0).with_property("C1", c1));
    set(dom, joint, "Part0", Variant::Ref(a));
    set(dom, joint, "Part1", Variant::Ref(b));
    joint
}

/// A constraint joining two attachments.
fn join_attachments(dom: &mut WeakDom, class: &str, on: Ref, a: Ref, b: Ref) -> Ref {
    let joint = dom.insert(on, InstanceBuilder::new(class).with_name(class));
    set(dom, joint, "Attachment0", Variant::Ref(a));
    set(dom, joint, "Attachment1", Variant::Ref(b));
    joint
}

/// A Base with an Arm on a Motor6D called Shoulder, above it.
fn body() -> (WeakDom, Ref, Ref, Ref) {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let model = dom.insert(root, InstanceBuilder::new("Model").with_name("Rig"));
    let base = dom.insert(model, part("Base", at(0.0, 0.0, 0.0)));
    let arm = dom.insert(model, part("Arm", at(0.0, 2.0, 0.0)));
    join_parts(&mut dom, "Motor6D", base, arm, at(0.0, 1.0, 0.0), at(0.0, -1.0, 0.0));
    let shoulder = dom.get_by_ref(base).unwrap().children().last().copied().unwrap();
    dom.get_by_ref_mut(shoulder).unwrap().name = "Shoulder".into();
    (dom, model, base, arm)
}

fn scene_of(dom: &WeakDom) -> Scene {
    let imported = import_dom(dom, "test.rbxm").unwrap();
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    imported.scene
}

fn part_id(s: &Scene, name: &str) -> usize {
    s.parts.iter().position(|p| p.name == name).unwrap_or_else(|| panic!("no part {name}"))
}

fn bone(s: &Scene, name: &str) -> usize {
    s.bones.iter().position(|b| b.name == name).unwrap_or_else(|| panic!("no bone {name}"))
}

/// The part stands where it was saved, and moves with the arm when the shoulder does.
fn follows_the_arm(s: &mut Scene, name: &str, saved: Vec3) {
    let id = part_id(s, name);
    assert!(matches!(s.parts[id].drive, Drive::Rigid { .. }), "{name} is held by {:?}", s.parts[id].drive);
    assert!(s.parts[id].world.pos.distance(saved) < 1e-4, "{name} stands at {}", s.parts[id].world.pos);
    s.set_bone_pose(bone(s, "Shoulder"), Xf::new(Quat::IDENTITY, Vec3::new(0.0, 0.0, 3.0)));
    assert!(s.parts[id].world.pos.distance(saved + Vec3::new(0.0, 0.0, 3.0)) < 1e-4, "{name} didn't follow the arm");
}

#[test]
fn every_joint_instance_holds_its_part() {
    for class in ["Weld", "ManualWeld", "Snap", "Glue", "ManualGlue", "Rotate", "RotateP", "RotateV"] {
        let (mut dom, model, _, arm) = body();
        let thing = dom.insert(model, part("Thing", at(2.0, 2.0, 0.0)));
        join_parts(&mut dom, class, arm, thing, at(1.0, 0.0, 0.0), at(-1.0, 0.0, 0.0));
        follows_the_arm(&mut scene_of(&dom), "Thing", Vec3::new(2.0, 2.0, 0.0));
    }
}

#[test]
fn a_weld_constraint_keeps_the_saved_spot() {
    let (mut dom, model, _, arm) = body();
    let thing = dom.insert(model, part("Thing", at(2.0, 2.5, 0.0)));
    join_parts(&mut dom, "WeldConstraint", thing, arm, at(0.0, 0.0, 0.0), at(0.0, 0.0, 0.0));
    follows_the_arm(&mut scene_of(&dom), "Thing", Vec3::new(2.0, 2.5, 0.0));
}

/// What new Roblox accessories are held by, between the Handle's attachment and the body's.
#[test]
fn a_rigid_constraint_holds_by_its_attachments() {
    let (mut dom, model, _, arm) = body();
    let thing = dom.insert(model, part("Thing", at(2.0, 2.0, 0.0)));
    let on_thing = attachment(&mut dom, thing, "A", at(-1.0, 0.0, 0.0));
    let on_arm = attachment(&mut dom, arm, "A", at(1.0, 0.0, 0.0));
    join_attachments(&mut dom, "RigidConstraint", thing, on_thing, on_arm);
    follows_the_arm(&mut scene_of(&dom), "Thing", Vec3::new(2.0, 2.0, 0.0));
}

#[test]
fn physics_constraints_keep_the_part_where_it_was_saved() {
    for class in [
        "BallSocketConstraint", "HingeConstraint", "PrismaticConstraint", "CylindricalConstraint", "UniversalConstraint", "RodConstraint",
        "RopeConstraint", "SpringConstraint", "PlaneConstraint", "SlidingBallConstraint", "AlignPosition", "AlignOrientation",
    ] {
        let (mut dom, model, _, arm) = body();
        // The attachments say nothing about where it hangs, so the saved spot is used.
        let thing = dom.insert(model, part("Thing", at(0.5, -1.0, 0.25)));
        let on_thing = attachment(&mut dom, thing, "A", at(0.0, 1.0, 0.0));
        let on_arm = attachment(&mut dom, arm, "A", at(0.0, -1.0, 0.0));
        join_attachments(&mut dom, class, arm, on_arm, on_thing);
        follows_the_arm(&mut scene_of(&dom), "Thing", Vec3::new(0.5, -1.0, 0.25));
    }
}

#[test]
fn a_velocity_motor_is_a_bone() {
    let (mut dom, model, _, arm) = body();
    let hand = dom.insert(model, part("Hand", at(0.0, 4.0, 0.0)));
    join_parts(&mut dom, "VelocityMotor", arm, hand, at(0.0, 1.0, 0.0), at(0.0, -1.0, 0.0));
    let s = scene_of(&dom);
    let motor = bone(&s, "VelocityMotor");
    assert_eq!(s.bones[motor].kind, BoneKind::LegacyMotor);
    assert_eq!(s.bones[motor].parent, Some(bone(&s, "Shoulder")));
}

#[test]
fn a_turned_off_joint_holds_nothing() {
    let (mut dom, model, _, arm) = body();
    let thing = dom.insert(model, part("Thing", at(2.0, 2.0, 0.0)));
    let weld = join_parts(&mut dom, "Weld", arm, thing, at(1.0, 0.0, 0.0), at(-1.0, 0.0, 0.0));
    set(&mut dom, weld, "Enabled", Variant::Bool(false));
    let s = scene_of(&dom);
    assert!(matches!(s.parts[part_id(&s, "Thing")].drive, Drive::Root));
}

#[test]
fn an_attachment_on_a_bone_is_found_through_it() {
    let (mut dom, model, _, arm) = body();
    let spine = dom.insert(arm, InstanceBuilder::new("Bone").with_name("Spine").with_property("CFrame", at(0.0, 1.0, 0.0)));
    let on_bone = attachment(&mut dom, spine, "Grip", at(1.0, 0.0, 0.0));
    // The bone is 1 up the arm and the attachment 1 across it, so the thing hangs at (1, 3).
    let thing = dom.insert(model, part("Thing", at(1.0, 3.0, 0.0)));
    let on_thing = attachment(&mut dom, thing, "Grip", at(0.0, 0.0, 0.0));
    join_attachments(&mut dom, "RigidConstraint", thing, on_thing, on_bone);
    let s = scene_of(&dom);
    let thing = part_id(&s, "Thing");
    assert!(!matches!(s.parts[thing].drive, Drive::Root));
    assert!(s.parts[thing].world.pos.distance(Vec3::new(1.0, 3.0, 0.0)) < 1e-4);
}

/// New R15 rigs pair each AnimationConstraint with a BallSocketConstraint on the same parts.
#[test]
fn a_ball_socket_beside_an_animated_joint_leaves_the_limb_animated() {
    let (mut dom, _, base, arm) = body();
    let on_base = attachment(&mut dom, base, "A", at(0.0, 1.0, 0.0));
    let on_arm = attachment(&mut dom, arm, "A", at(0.0, -1.0, 0.0));
    join_attachments(&mut dom, "BallSocketConstraint", base, on_base, on_arm);
    let s = scene_of(&dom);
    assert_eq!(s.parts[part_id(&s, "Arm")].drive, Drive::Bone(bone(&s, "Shoulder")));
}

fn accessory(dom: &mut WeakDom, model: Ref, saved: CFrame) -> Ref {
    let worn = dom.insert(model, InstanceBuilder::new("Accessory").with_name("Hat"));
    dom.insert(worn, part("Handle", saved))
}

#[test]
fn an_accessory_with_no_joint_goes_on_by_its_attachment_name() {
    let (mut dom, model, _, arm) = body();
    attachment(&mut dom, arm, "HatAttachment", at(0.0, 1.0, 0.0));
    // Saved somewhere else entirely, as an accessory that was never worn can be.
    let handle = accessory(&mut dom, model, at(50.0, 0.0, 0.0));
    attachment(&mut dom, handle, "HatAttachment", at(0.0, -0.5, 0.0));
    let mut s = scene_of(&dom);
    follows_the_arm(&mut s, "Handle", Vec3::new(0.0, 3.5, 0.0));
}

#[test]
fn an_old_hat_with_no_attachment_stays_on_the_head() {
    let (mut dom, model, _, arm) = body();
    dom.get_by_ref_mut(arm).unwrap().name = "Head".into();
    accessory(&mut dom, model, at(0.0, 3.4, 0.0));
    let s = scene_of(&dom);
    let handle = part_id(&s, "Handle");
    assert!(matches!(s.parts[handle].drive, Drive::Rigid { parent, .. } if parent == part_id(&s, "Head")));
    assert!(s.parts[handle].world.pos.distance(Vec3::new(0.0, 3.4, 0.0)) < 1e-4);
}

/// A project saved before Kaye read RigidConstraints holds the hat as a part standing on its
/// own. Opening it reads the rig's kept model again and puts the hat back on the arm.
#[test]
fn an_old_project_gets_its_loose_accessory_attached() {
    let (mut dom, model, _, arm) = body();
    let thing = dom.insert(model, part("Thing", at(2.0, 2.0, 0.0)));
    let on_thing = attachment(&mut dom, thing, "A", at(-1.0, 0.0, 0.0));
    let on_arm = attachment(&mut dom, arm, "A", at(1.0, 0.0, 0.0));
    join_attachments(&mut dom, "RigidConstraint", thing, on_thing, on_arm);
    let mut s = scene_of(&dom);
    // What the importer made before: a root, placed first.
    let thing = part_id(&s, "Thing");
    s.parts[thing].drive = Drive::Root;
    s.part_order.retain(|p| *p != thing);
    s.part_order.insert(0, thing);

    let mut animation = kaye::animation::Animation::default();
    animation.ensure_clips(&s);
    let view = kaye::project::View {
        frame: 0.0,
        target: Vec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        dist: 10.0,
        ortho: false,
        floor_y: 0.0,
        camera: None,
        preview: false,
        through: false,
        motion_paths: Vec::new(),
    };
    let path = std::env::temp_dir().join(format!("kaye-old-hat-{}.kaye", std::process::id()));
    kaye::project::save(&path, &s, &animation, &view).unwrap();
    let mut opened = kaye::project::load(&path).unwrap().scene;
    let _ = std::fs::remove_file(&path);

    let arm = part_id(&opened, "Arm");
    assert!(matches!(opened.parts[thing].drive, Drive::Rigid { parent, .. } if parent == arm));
    let order = |p: usize| opened.part_order.iter().position(|o| *o == p).unwrap();
    assert!(order(arm) < order(thing), "the hat is placed after the arm it hangs from");
    follows_the_arm(&mut opened, "Thing", Vec3::new(2.0, 2.0, 0.0));
}
