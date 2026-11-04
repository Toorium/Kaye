//! Inherit Rotation's influence, React, Ragdoll and motion paths, on real rigs.

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

// React.

/// A rig with React on its right arm, and a block coming from in front of it and on
/// through where that arm is. It reaches the arm at about frame 15, drags it back, and
/// is gone from frame 24 while still touching it, so the arm starts recovering there.
fn arm_and_passing_block(recovery: f32) -> (Scene, Animation, BoneId, BoneId) {
    let mut s = sample("R6Rig.rbxm");
    let arm = bone(&s, "Right Shoulder");
    let arm_part = part(&s, "Right Arm");
    let at = s.parts[arm_part].world.pos;
    let block = s.add_shape(PartShape::Block, Xf::new(Quat::IDENTITY, at + Vec3::new(0.0, 0.0, 6.0)), Vec3::new(1.0, 1.0, 1.0));
    let block_rig = s.bones[block].rig;
    modifiers::add_react(&mut s, arm);
    let index = s.modifiers.len() - 1;
    modifiers::add_object(&mut s, index, block_rig);
    if let ModifierKind::React(react) = &mut s.modifiers[index].kind {
        react.recovery = recovery;
    }
    let mut a = Animation { end: 90, fps: 30.0, ..Animation::default() };
    a.default_easing.style = kaye::animation::EaseStyle::Linear;
    a.set_key(block, 0, Xf::new(Quat::IDENTITY, at + Vec3::new(0.0, 0.0, 6.0)));
    a.set_key(block, 23, Xf::new(Quat::IDENTITY, at + Vec3::new(0.0, 0.0, -0.9)));
    a.set_key(block, 24, Xf::new(Quat::IDENTITY, at + Vec3::new(40.0, 0.0, 0.0)));
    a.ensure_clips(&s);
    baked(&mut s, &a);
    (s, a, arm, block)
}

fn arm_turn(s: &mut Scene, a: &Animation, arm: BoneId, frame: f32) -> f32 {
    a.apply(s, frame);
    s.bones[arm].solved.rot.angle_between(s.bones[arm].pose.rot)
}

#[test]
fn a_limb_with_nothing_to_bump_into_doesnt_move() {
    let mut s = sample("R6Rig.rbxm");
    let arm = bone(&s, "Right Shoulder");
    modifiers::add_react(&mut s, arm);
    let mut a = Animation { end: 20, ..Animation::default() };
    a.set_key(arm, 0, turned_z(0.5));
    baked(&mut s, &a);
    a.apply(&mut s, 10.0);
    assert!(s.bones[arm].solved.rot.angle_between(turned_z(0.5).rot) < 1e-4);
}

#[test]
fn a_limb_gets_pushed_and_stays_attached() {
    let (mut s, a, arm, _) = arm_and_passing_block(0.5);
    let head = |s: &mut Scene, frame: f32| {
        a.apply(s, frame);
        s.bones[arm].head()
    };
    let at_rest = head(&mut s, 0.0);
    let peak = (0..=23).map(|f| arm_turn(&mut s, &a, arm, f as f32)).fold(0.0, f32::max);
    assert!(peak.to_degrees() > 10.0, "the block should have pushed it, got {} degrees", peak.to_degrees());
    for frame in 0..=90 {
        let now = head(&mut s, frame as f32);
        assert!(now.distance(at_rest) < 1e-4, "the joint stays on the body on frame {frame}");
    }
}

#[test]
fn a_pushed_limb_stays_out_of_what_hit_it() {
    let (mut s, a, _, block) = arm_and_passing_block(0.5);
    let arm_part = part(&s, "Right Arm");
    for frame in 0..=23 {
        a.apply(&mut s, frame as f32);
        let block_part = s.bones[block].part.unwrap();
        let (block_at, half) = (s.parts[block_part].world, s.parts[block_part].size * 0.5);
        let (arm_at, arm_size) = (s.parts[arm_part].world, s.parts[arm_part].size);
        // The arm's middle line can come up to its own half width from the block.
        for t in [-0.45f32, -0.3] {
            let point = arm_at.point(Vec3::new(0.0, t * arm_size.y, 0.0));
            let local = block_at.inverse().point(point);
            let inside = local.x.abs() < half.x - 0.05 && local.y.abs() < half.y - 0.05 && local.z.abs() < half.z - 0.05;
            assert!(!inside, "the arm is inside the block on frame {frame}");
        }
    }
}

#[test]
fn a_body_walking_through_a_limb_drags_it_along_and_not_over_the_head() {
    let mut s = sample("R15Rig.rbxm");
    s.append(sample("R15Rig.rbxm"));
    s.evaluate();
    s.dynamics.floor = s.bounds(None).unwrap().0.y;
    s.set_rig_placement(1, Xf::new(Quat::IDENTITY, Vec3::new(2.4, 0.0, -10.0)));
    let joints = s.live_bones(0);
    let shoulder = joints.iter().copied().find(|b| s.bones[*b].name == "RightShoulder").unwrap();
    let wrist = joints.iter().copied().find(|b| s.bones[*b].name == "RightWrist").unwrap();
    let root = s.live_bones(1).into_iter().find(|b| s.bones[*b].name == "Root").unwrap();
    modifiers::add_react(&mut s, shoulder);
    let index = s.modifiers.len() - 1;
    modifiers::add_object(&mut s, index, 1);
    let mut a = Animation { end: 120, fps: 30.0, ..Animation::default() };
    a.default_easing.style = kaye::animation::EaseStyle::Linear;
    a.set_key(root, 0, Xf::IDENTITY);
    a.set_key(root, 60, Xf::new(Quat::IDENTITY, Vec3::new(0.0, 0.0, 20.0)));
    a.ensure_clips(&s);
    baked(&mut s, &a);
    let joint_height = s.bones[shoulder].head().y;
    let mut furthest = 0.0f32;
    for frame in 0..=120 {
        a.apply(&mut s, frame as f32);
        let end = s.bones[wrist].tail();
        assert!(end.y < joint_height + 0.75, "the hand went up to {} over a shoulder at {joint_height} on frame {frame}", end.y);
        furthest = furthest.max(end.z - s.bones[shoulder].head().z);
    }
    assert!(furthest > 1.0, "the walker should have dragged the arm along, by {furthest}");
    a.apply(&mut s, 120.0);
    assert!(s.bones[shoulder].solved.rot.angle_between(s.bones[shoulder].pose.rot) < 0.02, "and it comes back once they've parted");
}

#[test]
fn a_limb_comes_back_smoothly_over_its_recovery_time() {
    let (mut s, a, arm, _) = arm_and_passing_block(1.0);
    // The block is gone from frame 24, so a recovery of one second runs to about frame 54.
    let start = arm_turn(&mut s, &a, arm, 24.0);
    assert!(start.to_degrees() > 5.0, "pushed by then: {} degrees", start.to_degrees());
    let middle = arm_turn(&mut s, &a, arm, 39.0);
    assert!(middle < start && middle > start * 0.02, "partway back: {middle} of {start}");
    assert!(arm_turn(&mut s, &a, arm, 58.0) < start * 0.05, "back by the end of its time");
    assert!(arm_turn(&mut s, &a, arm, 90.0) < 0.01);
    let first_step = start - arm_turn(&mut s, &a, arm, 25.0);
    let second_step = arm_turn(&mut s, &a, arm, 25.0) - arm_turn(&mut s, &a, arm, 26.0);
    assert!(first_step < second_step, "it eases out: {first_step}, then {second_step}");
}

#[test]
fn a_longer_recovery_takes_longer() {
    let quick = {
        let (mut s, a, arm, _) = arm_and_passing_block(0.2);
        arm_turn(&mut s, &a, arm, 33.0)
    };
    let slow = {
        let (mut s, a, arm, _) = arm_and_passing_block(2.0);
        arm_turn(&mut s, &a, arm, 33.0)
    };
    assert!(slow > quick, "after the same time, {slow} should be further out than {quick}");
}

#[test]
fn reacts_influence_scales_it_and_none_turns_it_off() {
    let (mut s, a, arm, _) = arm_and_passing_block(0.5);
    let full = arm_turn(&mut s, &a, arm, 22.0);
    assert!(full > 0.1);
    s.bones[arm].value = 0.5;
    assert!((arm_turn(&mut s, &a, arm, 22.0) - full * 0.5).abs() < full * 0.1, "half influence is about half");
    s.bones[arm].value = 0.0;
    assert!(arm_turn(&mut s, &a, arm, 22.0) < 2e-3);
}

#[test]
fn scrubbing_shows_the_same_as_playing_through() {
    let (mut s, a, arm, _) = arm_and_passing_block(0.5);
    let played: Vec<Quat> = (0..=60)
        .map(|f| {
            a.apply(&mut s, f as f32);
            s.bones[arm].frame.rot
        })
        .collect();
    for frame in [50, 3, 37, 12, 60, 21] {
        a.apply(&mut s, frame as f32);
        assert!(s.bones[arm].frame.rot.angle_between(played[frame]) < 2e-3, "frame {frame}");
    }
}

#[test]
fn removing_a_rig_takes_it_off_the_list() {
    let (mut s, _, _, block) = arm_and_passing_block(0.5);
    let rig = s.bones[block].rig;
    let index = s.modifiers.len() - 1;
    modifiers::delete_rig(&mut s, rig);
    let ModifierKind::React(react) = s.modifiers[index].kind else { panic!() };
    assert!(react.objects.iter().all(Option::is_none));
}

#[test]
fn an_object_goes_on_the_list_once_and_comes_off_again() {
    let mut s = sample("R6Rig.rbxm");
    let arm = bone(&s, "Right Shoulder");
    modifiers::add_react(&mut s, arm);
    modifiers::add_object(&mut s, 0, 3);
    modifiers::add_object(&mut s, 0, 3);
    modifiers::add_object(&mut s, 0, 5);
    let objects = |s: &Scene| match s.modifiers[0].kind {
        ModifierKind::React(react) => react.objects.iter().flatten().copied().collect::<Vec<_>>(),
        _ => panic!(),
    };
    assert_eq!(objects(&s), [3, 5]);
    modifiers::remove_object(&mut s, 0, 3);
    assert_eq!(objects(&s), [5]);
}

/// R6 with React on its right arm and something right beside it, close enough to touch
/// the whole length of the arm from the side.
fn arm_beside(shape: PartShape, size: f32, overlap: f32) -> (Scene, Animation, BoneId) {
    let mut s = sample("R6Rig.rbxm");
    let arm = bone(&s, "Right Shoulder");
    let center = s.parts[part(&s, "Right Arm")].world.pos;
    let at = Xf::new(Quat::IDENTITY, center + Vec3::new(0.5 + size * 0.5 - overlap, 0.0, 0.0));
    let block = s.add_shape(shape, at, Vec3::splat(size));
    let rig = s.bones[block].rig;
    modifiers::add_react(&mut s, arm);
    let index = s.modifiers.len() - 1;
    modifiers::add_object(&mut s, index, rig);
    let a = Animation { end: 10, fps: 30.0, ..Animation::default() };
    baked(&mut s, &a);
    a.apply(&mut s, 10.0);
    (s, a, arm)
}

/// How far the limb's turn is along its own length, in radians.
fn roll_of(s: &Scene, arm: BoneId) -> f32 {
    let turn = s.bones[arm].pose.rot.conjugate() * s.bones[arm].solved.rot;
    turn.to_scaled_axis().dot(Vec3::Y).abs()
}

#[test]
fn a_flat_push_along_the_whole_arm_swings_it_a_little_with_no_roll() {
    let (s, _, arm) = arm_beside(PartShape::Block, 4.0, 0.3);
    let turn = s.bones[arm].solved.rot.angle_between(s.bones[arm].pose.rot).to_degrees();
    assert!(turn > 3.0 && turn < 40.0, "it should swing just far enough, got {turn} degrees");
    assert!(roll_of(&s, arm) < 0.02, "no roll: {}", roll_of(&s, arm));
}

#[test]
fn a_ball_only_pushes_where_it_really_touches() {
    let (s, _, arm) = arm_beside(PartShape::Ball, 3.0, 0.3);
    let turn = s.bones[arm].solved.rot.angle_between(s.bones[arm].pose.rot).to_degrees();
    assert!(turn > 1.0 && turn < 40.0, "got {turn} degrees");
    assert!(roll_of(&s, arm) < 0.02);
    // The corners of a box this size would reach the arm, and a ball doesn't.
    let (clear, _, arm) = arm_beside(PartShape::Ball, 3.0, -0.05);
    assert!(clear.bones[arm].solved.rot.angle_between(clear.bones[arm].pose.rot) < 2e-3);
}

#[test]
fn nothing_moves_with_a_gap() {
    let (s, _, arm) = arm_beside(PartShape::Block, 4.0, -0.05);
    assert!(s.bones[arm].solved.rot.angle_between(s.bones[arm].pose.rot) < 2e-3);
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
fn a_motion_path_follows_the_far_end_of_the_bone_on_every_frame() {
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
        assert!(points[index].distance(s.bones[arm].tail()) < 1e-4, "frame {frame}");
    }
    assert!(points[0].distance(points[20]) > 1.0, "it's a path, not a point");
}

#[test]
fn a_motion_path_shows_what_physics_did() {
    let (mut s, a, arm) = raised_arm();
    modifiers::add_ragdoll(&mut s, arm);
    let replay = dynamics::replay(s.clone(), a.clone(), vec![arm]);
    assert!(replay.baked.is_some());
    let points = &replay.paths.points[&arm];
    assert!(points[60].y < points[0].y - 0.5, "the far end of the arm falls");
}

#[test]
fn a_replay_only_runs_again_when_the_work_changes() {
    let mut s = sample("R15Rig.rbxm");
    let arm = bone(&s, "RightShoulder");
    let idle = bone(&s, "LeftShoulder");
    let mut a = Animation::default();
    a.set_key(arm, 0, Xf::IDENTITY);
    a.set_key(arm, 30, turned_z(1.0));
    modifiers::add_react(&mut s, arm);
    let signature = dynamics::signature(&s, &a, &[]);

    a.apply(&mut s, 15.0);
    assert_eq!(dynamics::signature(&s, &a, &[]), signature, "scrubbing isn't a change");

    assert_ne!(dynamics::signature(&s, &a, &[arm]), signature, "a new path is");
    let mut changed = a.clone();
    changed.set_key(arm, 10, turned_z(0.3));
    assert_ne!(dynamics::signature(&s, &changed, &[]), signature, "a key is");
    let index = s.modifiers.len() - 1;
    if let ModifierKind::React(react) = &mut s.modifiers[index].kind {
        react.recovery = 3.0;
    }
    let with_recovery = dynamics::signature(&s, &a, &[]);
    assert_ne!(with_recovery, signature, "a setting is");
    s.bones[idle].pose = turned_z(0.2);
    assert_ne!(dynamics::signature(&s, &a, &[]), with_recovery, "and so is posing a bone with no keys");
}

#[test]
fn react_and_ragdoll_survive_a_save() {
    let (mut s, a, arm, _) = arm_and_passing_block(0.75);
    modifiers::add_ragdoll(&mut s, arm);
    let view = kaye::project::View { frame: 0.0, target: Vec3::ZERO, yaw: 0.0, pitch: 0.0, dist: 10.0, ortho: false, floor_y: 0.0, camera: None, preview: false, through: false, motion_paths: vec![arm] };
    let path = std::env::temp_dir().join(format!("kaye-dynamics-{}.kaye", std::process::id()));
    kaye::project::save(&path, &s, &a, &view).unwrap();
    let opened = kaye::project::load(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(opened.scene.modifiers, s.modifiers);
    assert_eq!(opened.view.motion_paths, vec![arm]);
}
