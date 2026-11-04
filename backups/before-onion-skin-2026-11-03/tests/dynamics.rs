//! Inherit Rotation's influence, Ragdoll and motion paths, on real rigs.

use std::path::Path;

use glam::{Quat, Vec3};
use kaye::animation::Animation;
use kaye::dynamics;
use kaye::import::import_file;
use kaye::math::Xf;
use kaye::modifiers::{self, ModifierKind};
use kaye::posing;
use kaye::scene::{BoneId, PartShape, Scene};

fn sample(name: &str) -> Scene {
    let mut scene = import_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(name)).unwrap().scene;
    scene.evaluate();
    scene.dynamics.floor = scene.bounds(None).unwrap().0.y;
    scene
}

fn bone(scene: &Scene, name: &str) -> BoneId {
    scene.bones.iter().position(|b| b.name == name).unwrap_or_else(|| panic!("no bone {name}"))
}

fn part(scene: &Scene, name: &str) -> usize {
    scene.parts.iter().position(|p| p.name == name).unwrap_or_else(|| panic!("no part {name}"))
}

fn turned_z(angle: f32) -> Xf {
    Xf::new(Quat::from_rotation_z(angle), Vec3::ZERO)
}

fn baked(scene: &mut Scene, animation: &Animation) {
    let physics = dynamics::bake(scene, animation);
    scene.dynamics.set_baked(physics);
}

fn lowest(scene: &Scene) -> f32 {
    scene.bounds(None).unwrap().0.y
}

// Inherit Rotation.

#[test]
fn inherit_rotations_influence_blends_between_following_and_holding() {
    let mut s = sample("R15Rig.rbxm");
    let (waist, neck) = (bone(&s, "Waist"), bone(&s, "Neck"));
    let still = s.bones[neck].frame.rot;
    modifiers::add_inherit(&mut s, neck);
    s.set_bone_pose(waist, turned_z(1.0));
    assert!(s.bones[neck].frame.rot.angle_between(still) < 1e-3, "full influence holds it");

    s.bones[neck].value = 0.0;
    s.evaluate();
    assert!((s.bones[neck].frame.rot.angle_between(still) - 1.0).abs() < 1e-3, "none follows the parent");

    s.bones[neck].value = 0.5;
    s.evaluate();
    assert!((s.bones[neck].frame.rot.angle_between(still) - 0.5).abs() < 1e-3, "half is halfway");
}

#[test]
fn inherit_rotations_influence_can_be_keyed() {
    let mut s = sample("R15Rig.rbxm");
    let (waist, neck) = (bone(&s, "Waist"), bone(&s, "Neck"));
    let still = s.bones[neck].frame.rot;
    modifiers::add_inherit(&mut s, neck);
    s.set_bone_pose(waist, turned_z(1.0));
    let mut a = Animation::default();
    let easing = kaye::animation::Easing { style: kaye::animation::EaseStyle::Linear, ..Default::default() };
    a.default_easing = easing;
    a.set_key_full(neck, 0, Xf::IDENTITY, 0.0);
    a.set_key_full(neck, 10, Xf::IDENTITY, 1.0);
    a.apply(&mut s, 0.0);
    assert!((s.bones[neck].frame.rot.angle_between(still) - 1.0).abs() < 1e-3);
    a.apply(&mut s, 10.0);
    assert!(s.bones[neck].frame.rot.angle_between(still) < 1e-3);
}

// Ragdoll.

#[test]
fn affects_body_off_on_a_torso_leaves_the_limbs_out() {
    let s = sample("R6Rig.rbxm");
    let torso = bone(&s, "RootJoint");
    assert_eq!(dynamics::ragdoll_bones(&s, torso, false), [torso]);
    let r15 = sample("R15Rig.rbxm");
    assert_eq!(dynamics::ragdoll_bones(&r15, bone(&r15, "RightShoulder"), false).len(), 3, "a whole arm");
}

#[test]
fn a_ragdoll_lands_on_an_object_on_its_list() {
    let drop_with = |listed: bool| {
        let (mut s, a, arm) = raised_arm();
        let center = s.parts[part(&s, "Right Arm")].world.pos;
        let shelf = s.add_shape(PartShape::Block, Xf::new(Quat::IDENTITY, center + Vec3::new(0.0, -1.0, 0.0)), Vec3::new(6.0, 1.0, 6.0));
        let rig = s.bones[shelf].rig;
        modifiers::add_ragdoll(&mut s, arm);
        if listed {
            modifiers::add_object(&mut s, 0, rig);
        }
        baked(&mut s, &a);
        a.apply(&mut s, 90.0);
        s.parts[part(&s, "Right Arm")].world.pos.y
    };
    assert!(drop_with(true) > drop_with(false) + 0.2, "{} against {}", drop_with(true), drop_with(false));
}

#[test]
fn a_deleted_shape_comes_off_every_list_and_undo_can_bring_it_back() {
    let mut s = sample("R6Rig.rbxm");
    let arm = bone(&s, "Right Shoulder");
    let shape = s.add_shape(PartShape::Block, Xf::IDENTITY, Vec3::ONE);
    let rig = s.bones[shape].rig;
    modifiers::add_ragdoll(&mut s, arm);
    modifiers::add_object(&mut s, 0, rig);
    let before = s.rig_state();
    modifiers::delete_bones(&mut s, &[shape]);
    assert!(s.rigs[rig].removed);
    let ModifierKind::Ragdoll(ragdoll) = s.modifiers[0].kind else { panic!() };
    assert!(ragdoll.objects.iter().all(Option::is_none));
    s.restore_rig_state(&before);
    assert!(!s.rigs[rig].removed);
}

/// R6 with its right arm held straight out.
fn raised_arm() -> (Scene, Animation, BoneId) {
    let mut s = sample("R6Rig.rbxm");
    let arm = bone(&s, "Right Shoulder");
    let mut a = Animation { end: 90, fps: 30.0, ..Animation::default() };
    a.set_key(arm, 0, turned_z(1.5));
    a.ensure_clips(&s);
    a.apply(&mut s, 0.0);
    (s, a, arm)
}

#[test]
fn a_ragdolled_limb_falls_and_stays_attached() {
    let (mut s, a, arm) = raised_arm();
    let arm_part = part(&s, "Right Arm");
    let height_up = s.parts[arm_part].world.pos.y;
    modifiers::add_ragdoll(&mut s, arm);
    baked(&mut s, &a);
    let held_at = s.bones[arm].joint_base.pos;
    for frame in 0..=90 {
        a.apply(&mut s, frame as f32);
        assert!(s.bones[arm].head().distance(held_at) < 0.02, "the joint stays on the body on frame {frame}: {}", s.bones[arm].head().distance(held_at));
    }
    a.apply(&mut s, 90.0);
    let hanging = s.parts[arm_part].world.pos.y;
    assert!(hanging < height_up - 0.4, "it should have swung down, from {height_up} to {hanging}");
}

#[test]
fn a_ragdolled_limb_leaves_the_rest_of_the_body_alone() {
    let (mut s, a, arm) = raised_arm();
    let torso = part(&s, "Torso");
    let before = s.parts[torso].world;
    modifiers::add_ragdoll(&mut s, arm);
    baked(&mut s, &a);
    a.apply(&mut s, 60.0);
    assert!(s.parts[torso].world.approx_eq(before));
}

#[test]
fn no_gravity_means_nothing_falls() {
    let (mut s, a, arm) = raised_arm();
    let before = s.bones[arm].frame;
    modifiers::add_ragdoll(&mut s, arm);
    let index = s.modifiers.len() - 1;
    if let ModifierKind::Ragdoll(ragdoll) = &mut s.modifiers[index].kind {
        ragdoll.gravity = 0.0;
    }
    baked(&mut s, &a);
    a.apply(&mut s, 60.0);
    assert!(s.bones[arm].frame.rot.angle_between(before.rot) < 0.01, "{}", s.bones[arm].frame.rot.angle_between(before.rot));
}

#[test]
fn ragdolls_influence_blends_back_to_the_keys() {
    let (mut s, a, arm) = raised_arm();
    let keyed = s.bones[arm].frame;
    modifiers::add_ragdoll(&mut s, arm);
    baked(&mut s, &a);
    a.apply(&mut s, 60.0);
    let fallen = s.bones[arm].frame;
    let full = fallen.rot.angle_between(keyed.rot);
    assert!(full > 0.3);
    s.bones[arm].value = 0.5;
    s.evaluate();
    assert!((s.bones[arm].frame.rot.angle_between(keyed.rot) - full * 0.5).abs() < full * 0.1);
    s.bones[arm].value = 0.0;
    s.evaluate();
    assert!(s.bones[arm].frame.rot.angle_between(keyed.rot) < 2e-3);
}

#[test]
fn a_keyed_influence_starts_the_fall_partway_through() {
    let (mut s, mut a, arm) = raised_arm();
    modifiers::add_ragdoll(&mut s, arm);
    a.tracks.remove(&arm);
    let constant = kaye::animation::Easing { style: kaye::animation::EaseStyle::Constant, ..Default::default() };
    a.default_easing = constant;
    a.set_key_full(arm, 0, turned_z(1.5), 0.0);
    a.set_key_full(arm, 30, turned_z(1.5), 1.0);
    a.apply(&mut s, 0.0);
    let keyed = s.bones[arm].frame.rot;
    baked(&mut s, &a);
    a.apply(&mut s, 29.0);
    assert!(s.bones[arm].frame.rot.angle_between(keyed) < 1e-3, "it follows its keys until then");
    a.apply(&mut s, 40.0);
    assert!(s.bones[arm].frame.rot.angle_between(keyed) > 0.1, "then it falls");
}

fn assert_whole_body_lands(file: &str) {
    let mut s = sample(file);
    let floor = s.dynamics.floor;
    let top = s.bounds(None).unwrap().1.y;
    let joints = s.joints(0);
    let first = joints[0];
    let a = Animation { end: 150, fps: 30.0, ..Animation::default() };
    modifiers::add_ragdoll(&mut s, first);
    let index = s.modifiers.len() - 1;
    if let ModifierKind::Ragdoll(ragdoll) = &mut s.modifiers[index].kind {
        ragdoll.affects_body = true;
    }
    baked(&mut s, &a);
    for frame in 0..=150 {
        a.apply(&mut s, frame as f32);
        assert!(lowest(&s) > floor - 0.3, "{file} sank through the floor on frame {frame}: {} under {floor}", lowest(&s));
        for bone in &joints {
            if let Some(parent) = s.bones[*bone].parent {
                let stretch = s.bones[*bone].head().distance(s.bones[*bone].joint_base.pos);
                assert!(stretch < 0.3, "{file}: {} pulled off {} by {stretch} on frame {frame}", s.bones[*bone].name, s.bones[parent].name);
            }
        }
    }
    let highest = s.bounds(None).unwrap().1.y;
    assert!(highest < floor + (top - floor) * 0.6, "{file} should have collapsed: from {top} to {highest}");
    // It settles, and doesn't keep buzzing about on the floor.
    a.apply(&mut s, 149.0);
    let before: Vec<Vec3> = s.bones.iter().map(|b| b.head()).collect();
    a.apply(&mut s, 150.0);
    let moved = s.bones.iter().zip(&before).map(|(b, was)| b.head().distance(*was)).fold(0.0, f32::max);
    assert!(moved < 0.1, "{file} is still moving {moved} studs a frame");
}

#[test]
fn a_whole_r6_body_collapses_onto_the_floor() {
    assert_whole_body_lands("R6Rig.rbxm");
}

#[test]
fn a_whole_r15_body_collapses_onto_the_floor() {
    assert_whole_body_lands("R15Rig.rbxm");
}

#[test]
fn affects_body_off_only_sets_loose_what_hangs_from_it() {
    let s = sample("R15Rig.rbxm");
    let elbow = bone(&s, "RightElbow");
    let whole = s.joints(0).len();
    assert_eq!(dynamics::ragdoll_bones(&s, elbow, false).len(), 2, "the lower arm and the hand");
    assert_eq!(dynamics::ragdoll_bones(&s, elbow, true).len(), whole);
}

#[test]
fn baking_a_ragdoll_to_keys_plays_the_same() {
    let (mut s, a, arm) = raised_arm();
    modifiers::add_ragdoll(&mut s, arm);
    baked(&mut s, &a);
    let arm_part = part(&s, "Right Arm");
    let played: Vec<Xf> = (0..=90)
        .map(|f| {
            a.apply(&mut s, f as f32);
            s.parts[arm_part].world
        })
        .collect();
    let mut baked_animation = a.clone();
    let index = s.modifiers.len() - 1;
    posing::bake(&mut s, &mut baked_animation, &[index]);
    assert!(s.modifiers.is_empty());
    for (frame, was) in played.iter().enumerate() {
        baked_animation.apply(&mut s, frame as f32);
        assert!(s.parts[arm_part].world.pos.distance(was.pos) < 0.01, "frame {frame}");
    }
}

// Motion paths and what starts a replay.

#[test]
fn a_motion_path_follows_the_widest_part_of_the_bone_on_every_frame() {
    let mut s = sample("R15Rig.rbxm");
    let arm = bone(&s, "RightWrist");
    let shoulder = bone(&s, "RightShoulder");
    let mut a = Animation { start: 5, end: 25, ..Animation::default() };
    a.set_key(shoulder, 5, Xf::IDENTITY);
    a.set_key(shoulder, 25, turned_z(1.2));
    let replay = dynamics::replay(s.clone(), a.clone(), vec![arm]);
    assert!(replay.baked.is_none(), "nothing here needs physics");
    assert_eq!(replay.paths.start, 5);
    let points = &replay.paths.points[&arm];
    assert_eq!(points.len(), 21);
    for (index, frame) in (5..=25).enumerate() {
        a.apply(&mut s, frame as f32);
        assert!(points[index].distance(s.bones[arm].widest()) < 1e-4, "frame {frame}");
    }
    assert!(points[0].distance(points[20]) > 1.0, "it's a path, not a point");
}

#[test]
fn the_widest_part_of_a_torso_is_at_the_bottom_of_it() {
    let s = sample("R15Rig.rbxm");
    let torso = &s.bones[bone(&s, "Waist")];
    assert!(torso.tail().y > torso.head().y + 0.5, "the bone runs up the torso");
    assert!(torso.widest().y < (torso.head().y + torso.tail().y) * 0.5, "so the bottom of it is what's wide");
    assert!(torso.widest().distance(torso.head()) < torso.widest().distance(torso.tail()) * 0.1);
}

#[test]
fn a_motion_path_shows_what_physics_did() {
    let (mut s, a, arm) = raised_arm();
    modifiers::add_ragdoll(&mut s, arm);
    let index = s.modifiers.len() - 1;
    if let ModifierKind::Ragdoll(ragdoll) = &mut s.modifiers[index].kind {
        ragdoll.affects_body = true;
    }
    let replay = dynamics::replay(s.clone(), a.clone(), vec![arm]);
    assert!(replay.baked.is_some());
    let points = &replay.paths.points[&arm];
    assert!(points[60].y < points[0].y - 0.5, "the arm comes down with the body");
}

#[test]
fn a_replay_only_runs_again_when_the_work_changes() {
    let mut s = sample("R15Rig.rbxm");
    let arm = bone(&s, "RightShoulder");
    let idle = bone(&s, "LeftShoulder");
    let mut a = Animation::default();
    a.set_key(arm, 0, Xf::IDENTITY);
    a.set_key(arm, 30, turned_z(1.0));
    modifiers::add_ragdoll(&mut s, arm);
    let signature = dynamics::signature(&s, &a, &[]);

    a.apply(&mut s, 15.0);
    assert_eq!(dynamics::signature(&s, &a, &[]), signature, "scrubbing isn't a change");

    assert_ne!(dynamics::signature(&s, &a, &[arm]), signature, "a new path is");
    let mut changed = a.clone();
    changed.set_key(arm, 10, turned_z(0.3));
    assert_ne!(dynamics::signature(&s, &changed, &[]), signature, "a key is");
    let index = s.modifiers.len() - 1;
    if let ModifierKind::Ragdoll(ragdoll) = &mut s.modifiers[index].kind {
        ragdoll.gravity = 10.0;
    }
    let with_gravity = dynamics::signature(&s, &a, &[]);
    assert_ne!(with_gravity, signature, "a setting is");
    s.bones[idle].pose = turned_z(0.2);
    assert_ne!(dynamics::signature(&s, &a, &[]), with_gravity, "and so is posing a bone with no keys");
}

#[test]
fn a_ragdoll_survives_a_save() {
    let (mut s, a, arm) = raised_arm();
    modifiers::add_ragdoll(&mut s, arm);
    let view = kaye::project::View { frame: 0.0, target: Vec3::ZERO, yaw: 0.0, pitch: 0.0, dist: 10.0, ortho: false, floor_y: 0.0, camera: None, preview: false, through: false, motion_paths: vec![arm] };
    let path = std::env::temp_dir().join(format!("kaye-dynamics-{}.kaye", std::process::id()));
    kaye::project::save(&path, &s, &a, &view).unwrap();
    let opened = kaye::project::load(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(opened.scene.modifiers, s.modifiers);
    assert_eq!(opened.view.motion_paths, vec![arm]);
}
