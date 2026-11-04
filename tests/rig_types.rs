//! Rig types the sample files don't cover, built in code. These check Kaye's own
//! reading of each structure, not compatibility with files exported from Studio.

use std::path::Path;

use glam::{Quat, Vec3};
use kaye::import::{import_bytes, import_dom};
use kaye::math::Xf;
use kaye::scene::{BoneKind, Drive, RigKind, Scene};
use rbx_dom_weak::{InstanceBuilder, WeakDom};
use rbx_types::{CFrame, Matrix3, Ref, Variant, Vector3};
use ustr::ustr;

fn at(x: f32, y: f32, z: f32) -> CFrame {
    CFrame::new(Vector3::new(x, y, z), Matrix3::identity())
}

fn part(name: &str, class: &str, position: CFrame) -> InstanceBuilder {
    InstanceBuilder::new(class)
        .with_name(name)
        .with_property("CFrame", position)
        .with_property("Size", Vector3::new(1.0, 2.0, 1.0))
}

fn set_ref(dom: &mut WeakDom, on: Ref, name: &str, to: Ref) {
    dom.get_by_ref_mut(on)
        .unwrap()
        .properties
        .insert(ustr(name), Variant::Ref(to));
}

fn joint(dom: &mut WeakDom, class: &str, name: &str, a: Ref, b: Ref, c0: CFrame, c1: CFrame) -> Ref {
    let j = dom.insert(
        a,
        InstanceBuilder::new(class)
            .with_name(name)
            .with_property("C0", c0)
            .with_property("C1", c1),
    );
    set_ref(dom, j, "Part0", a);
    set_ref(dom, j, "Part1", b);
    j
}

fn new_model(dom: &mut WeakDom, parent: Ref, name: &str) -> Ref {
    dom.insert(parent, InstanceBuilder::new("Model").with_name(name))
}

fn scene_of(dom: &WeakDom) -> Scene {
    let imported = import_dom(dom, "test.rbxm").unwrap();
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    imported.scene
}

fn bone(s: &Scene, name: &str) -> usize {
    s.bones.iter().position(|b| b.name == name).unwrap()
}

fn part_id(s: &Scene, name: &str) -> usize {
    s.parts.iter().position(|p| p.name == name).unwrap()
}

#[test]
fn skinned_bones_nest_and_inherit_transforms() {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let model = new_model(&mut dom, root, "Dragon");
    let body = dom.insert(model, part("Body", "MeshPart", at(0.0, 5.0, 0.0)));
    let spine = dom.insert(
        body,
        InstanceBuilder::new("Bone").with_name("Spine").with_property("CFrame", at(0.0, 1.0, 0.0)),
    );
    dom.insert(
        spine,
        InstanceBuilder::new("Bone").with_name("Tail").with_property("CFrame", at(0.0, 2.0, 0.0)),
    );

    let mut s = scene_of(&dom);
    assert_eq!(s.rigs[0].kind, RigKind::Skinned);
    let (spine, tail) = (bone(&s, "Spine"), bone(&s, "Tail"));
    assert_eq!(s.bones[spine].kind, BoneKind::Skinned);
    assert_eq!(s.bones[tail].parent, Some(spine));
    assert!(s.bones[spine].head().distance(Vec3::new(0.0, 6.0, 0.0)) < 1e-4);
    assert!(s.bones[tail].head().distance(Vec3::new(0.0, 8.0, 0.0)) < 1e-4);

    // A quarter turn about Z swings the child from +Y to -X around the parent.
    s.set_bone_pose(spine, Xf::new(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), Vec3::ZERO));
    assert!(s.bones[tail].head().distance(Vec3::new(-2.0, 6.0, 0.0)) < 1e-4);
}

#[test]
fn custom_motor6d_rig_with_unusual_part_names() {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let model = new_model(&mut dom, root, "Crane");
    let base = dom.insert(model, part("Base", "Part", at(0.0, 0.0, 0.0)));
    let arm = dom.insert(model, part("Arm", "Part", at(0.0, 3.0, 0.0)));
    joint(&mut dom, "Motor6D", "Pivot", base, arm, at(0.0, 1.0, 0.0), at(0.0, -2.0, 0.0));

    let s = scene_of(&dom);
    assert_eq!(s.rigs[0].kind, RigKind::Custom);
    assert_eq!(s.parts[part_id(&s, "Base")].drive, Drive::Root);
    assert!(s.parts[part_id(&s, "Arm")].world.pos.distance(Vec3::new(0.0, 3.0, 0.0)) < 1e-4);
}

#[test]
fn legacy_motor_and_animation_constraint_become_bones() {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let model = new_model(&mut dom, root, "Mixed");
    let a = dom.insert(model, part("A", "Part", at(0.0, 0.0, 0.0)));
    let b = dom.insert(model, part("B", "Part", at(0.0, 2.0, 0.0)));
    let c = dom.insert(model, part("C", "Part", at(0.0, 4.0, 0.0)));
    joint(&mut dom, "Motor", "OldMotor", a, b, at(0.0, 1.0, 0.0), at(0.0, -1.0, 0.0));

    let on_b = dom.insert(
        b,
        InstanceBuilder::new("Attachment").with_property("CFrame", at(0.0, 1.0, 0.0)),
    );
    let on_c = dom.insert(
        c,
        InstanceBuilder::new("Attachment").with_property("CFrame", at(0.0, -1.0, 0.0)),
    );
    let constraint = dom.insert(b, InstanceBuilder::new("AnimationConstraint").with_name("Anim"));
    set_ref(&mut dom, constraint, "Attachment0", on_b);
    set_ref(&mut dom, constraint, "Attachment1", on_c);

    let mut s = scene_of(&dom);
    assert_eq!(s.bones[bone(&s, "OldMotor")].kind, BoneKind::LegacyMotor);
    let anim = bone(&s, "Anim");
    assert_eq!(s.bones[anim].kind, BoneKind::AnimationConstraint);
    assert_eq!(s.bones[anim].parent, Some(bone(&s, "OldMotor")));
    assert!(s.parts[part_id(&s, "C")].world.pos.distance(Vec3::new(0.0, 4.0, 0.0)) < 1e-4);

    s.set_bone_pose(anim, Xf::new(Quat::IDENTITY, Vec3::new(1.0, 0.0, 0.0)));
    assert!(s.parts[part_id(&s, "C")].world.pos.distance(Vec3::new(1.0, 4.0, 0.0)) < 1e-4);
}

#[test]
fn weld_constraint_keeps_saved_offset_and_follows_the_bone() {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let model = new_model(&mut dom, root, "Sword");
    let hip = dom.insert(model, part("Hip", "Part", at(0.0, 0.0, 0.0)));
    let hand = dom.insert(model, part("Hand", "Part", at(0.0, 2.0, 0.0)));
    let blade = dom.insert(model, part("Blade", "Part", at(3.0, 2.0, 0.0)));
    joint(&mut dom, "Motor6D", "Grip", hip, hand, at(0.0, 1.0, 0.0), at(0.0, -1.0, 0.0));
    let weld = dom.insert(hand, InstanceBuilder::new("WeldConstraint"));
    set_ref(&mut dom, weld, "Part0", blade);
    set_ref(&mut dom, weld, "Part1", hand);

    let mut s = scene_of(&dom);
    let blade = part_id(&s, "Blade");
    assert!(matches!(s.parts[blade].drive, Drive::Rigid { .. }));
    assert!(s.parts[blade].world.pos.distance(Vec3::new(3.0, 2.0, 0.0)) < 1e-4);
    assert_eq!(s.part_owner(blade), Some(bone(&s, "Grip")));

    s.set_bone_pose(bone(&s, "Grip"), Xf::new(Quat::IDENTITY, Vec3::new(0.0, 0.0, 2.0)));
    assert!(s.parts[blade].world.pos.distance(Vec3::new(3.0, 2.0, 2.0)) < 1e-4);
}

#[test]
fn every_part_shape_and_a_broken_joint_are_handled() {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let model = new_model(&mut dom, root, "Shapes");
    let a = dom.insert(model, part("Wedge", "WedgePart", at(0.0, 0.0, 0.0)));
    dom.insert(model, part("Corner", "CornerWedgePart", at(2.0, 0.0, 0.0)));
    // A Motor6D with nothing in Part1 must warn, not fail the import.
    let broken = dom.insert(a, InstanceBuilder::new("Motor6D").with_name("Loose"));
    set_ref(&mut dom, broken, "Part0", a);

    let imported = import_dom(&dom, "test.rbxm").unwrap();
    assert_eq!(imported.warnings.len(), 1);
    assert!(imported.warnings[0].contains("Loose"));
    assert_eq!(imported.scene.rigs[0].kind, RigKind::Static);
    assert_eq!(imported.scene.parts.len(), 2);
}

#[test]
fn folder_of_models_keeps_each_rig_separate() {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let folder = dom.insert(root, InstanceBuilder::new("Folder").with_name("Characters"));
    for name in ["One", "Two"] {
        let model = new_model(&mut dom, folder, name);
        let a = dom.insert(model, part("A", "Part", at(0.0, 0.0, 0.0)));
        let b = dom.insert(model, part("B", "Part", at(0.0, 2.0, 0.0)));
        joint(&mut dom, "Motor6D", "J", a, b, at(0.0, 1.0, 0.0), at(0.0, -1.0, 0.0));
    }
    let s = scene_of(&dom);
    let names: Vec<&str> = s.rigs.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["One", "Two"]);
    assert_eq!(s.bones.len(), 2);
    assert_eq!(s.bones[1].rig, 1);
}

/// Converts a real Studio file to XML, then imports that like an `.rbxmx`.
#[test]
fn xml_models_import_the_same_as_binary() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/R15Rig.rbxm");
    let bytes = std::fs::read(path).unwrap();
    let dom = rbx_binary::from_reader(&bytes[..]).unwrap();
    let mut xml = Vec::new();
    rbx_xml::to_writer_default(&mut xml, &dom, dom.root().children()).unwrap();

    let s = import_bytes(&xml, "R15Rig.rbxmx").unwrap().scene;
    assert_eq!(s.rigs[0].kind, RigKind::R15);
    assert_eq!(s.bones.len(), 15);
    assert_eq!(s.parts.len(), 16);
    for p in &s.parts {
        assert!(p.world.pos.distance(p.file_cframe.pos) < 0.02, "{}", p.name);
    }
}
