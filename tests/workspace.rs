//! Free rotation, moving whole rigs, deleting from the Workspace, Inherit Rotation, and
//! copying and pasting keys.

use std::path::Path;

use glam::{Quat, Vec3};
use kaye::animation::Animation;
use kaye::camera::OrbitCamera;
use kaye::export;
use kaye::gizmo::{OpKind, Session};
use kaye::import::import_file;
use kaye::math::Xf;
use kaye::modifiers;
use kaye::posing;
use kaye::project;
use kaye::scene::{BoneId, BoneLink, PartShape, Scene};

fn sample(name: &str) -> Scene {
    import_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(name)).unwrap().scene
}

fn bone_named(scene: &Scene, name: &str) -> BoneId {
    scene.bones.iter().position(|b| b.name == name).unwrap_or_else(|| panic!("no bone {name}"))
}

fn view() -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 700.0))
}

fn free_rotate(scene: &Scene, bone: BoneId, start: egui::Pos2) -> Session {
    let mut session = Session::new(scene, bone, OpKind::Rotate, None, true, start);
    session.press_rotate();
    session
}

/// A circle that starts and ends at `start`.
fn circle(start: egui::Pos2, radius: f32, steps: usize) -> impl Iterator<Item = egui::Pos2> {
    let center = start - egui::vec2(radius, 0.0);
    (1..=steps).map(move |i| {
        let angle = std::f32::consts::TAU * i as f32 / steps as f32;
        center + egui::vec2(angle.cos(), angle.sin()) * radius
    })
}

#[test]
fn a_full_circle_of_the_mouse_puts_the_bone_back() {
    let mut scene = sample("R15Rig.rbxm");
    let bone = bone_named(&scene, "RightShoulder");
    let camera = OrbitCamera::default();
    let start = egui::pos2(600.0, 300.0);
    let before = scene.bones[bone].frame;
    let mut session = free_rotate(&scene, bone, start);
    for radius in [40.0, 120.0, 260.0] {
        for mouse in circle(start, radius, 240) {
            session.update(&mut scene, &camera, view(), mouse);
        }
        let after = scene.bones[bone].frame;
        assert!(after.rot.angle_between(before.rot) < 1e-3, "off by {} after a circle of {radius}", after.rot.angle_between(before.rot));
        assert!(after.pos.distance(before.pos) < 1e-3);
    }
}

#[test]
fn free_rotation_only_depends_on_where_the_mouse_is() {
    let mut scene = sample("R15Rig.rbxm");
    let bone = bone_named(&scene, "RightShoulder");
    let camera = OrbitCamera::default();
    let start = egui::pos2(600.0, 300.0);
    let end = egui::pos2(720.0, 390.0);
    let mut session = free_rotate(&scene, bone, start);
    session.update(&mut scene, &camera, view(), end);
    let direct = scene.bones[bone].frame.rot;
    for mouse in [egui::pos2(100.0, 50.0), egui::pos2(900.0, 650.0), egui::pos2(300.0, 600.0), end] {
        session.update(&mut scene, &camera, view(), mouse);
    }
    assert!(scene.bones[bone].frame.rot.angle_between(direct) < 1e-4);
}

#[test]
fn free_rotation_turns_the_way_the_mouse_goes() {
    let mut scene = sample("R15Rig.rbxm");
    let bone = bone_named(&scene, "RightShoulder");
    let camera = OrbitCamera::default();
    let start = egui::pos2(600.0, 300.0);
    let front = scene.bones[bone].frame.rot.inverse() * camera.to_camera();
    let mut session = free_rotate(&scene, bone, start);
    session.update(&mut scene, &camera, view(), start + egui::vec2(60.0, 0.0));
    let moved = scene.bones[bone].frame.rot * front;
    assert!(moved.dot(camera.right()) > 0.1, "the side facing the viewer rolls right");
}

#[test]
fn moving_a_rig_leaves_its_poses_and_export_alone() {
    let mut scene = sample("R15Rig.rbxm");
    let bone = bone_named(&scene, "RightShoulder");
    let mut animation = Animation { end: 10, ..Animation::default() };
    animation.set_key(bone, 0, Xf::IDENTITY);
    animation.set_key(bone, 10, Xf::new(Quat::from_rotation_x(1.0), Vec3::ZERO));
    animation.apply(&mut scene, 5.0);
    let before: Vec<Xf> = scene.bones.iter().map(|b| b.output()).collect();
    let (hand, poses) = (scene.bones[bone].frame.pos, scene.bones[bone].pose);
    let shift = Vec3::new(5.0, 1.0, -3.0);

    scene.set_rig_placement(0, Xf::new(Quat::IDENTITY, shift));

    assert_eq!(scene.bones[bone].pose, poses);
    assert!(scene.bones[bone].frame.pos.distance(hand + shift) < 1e-4);
    for (now, was) in scene.bones.iter().map(|b| b.output()).zip(before) {
        assert!(now.pos.distance(was.pos) < 1e-4 && now.rot.angle_between(was.rot) < 1e-4);
    }
}

#[test]
fn dragging_a_rig_moves_it_without_touching_a_pose_and_cancel_undoes_it() {
    let mut scene = sample("R15Rig.rbxm");
    let root_was = scene.rig_frame(0);
    let camera = OrbitCamera::default();
    let start = egui::pos2(600.0, 300.0);
    let mut session = Session::for_rig(&scene, 0, OpKind::Move, None, true, start);
    session.update(&mut scene, &camera, view(), start + egui::vec2(120.0, 40.0));
    assert!(scene.rig_frame(0).pos.distance(root_was.pos) > 0.5, "it moved");
    assert!(scene.bones.iter().all(|b| b.pose == Xf::IDENTITY), "no pose changed");
    session.cancel(&mut scene);
    assert_eq!(scene.rigs[0].placement, Xf::IDENTITY);
    assert!(scene.rig_frame(0).pos.distance(root_was.pos) < 1e-5);
}

#[test]
fn turning_a_rig_keeps_its_handles_where_they_are() {
    let mut scene = sample("R15Rig.rbxm");
    let pivot = scene.rig_frame(0).pos;
    let camera = OrbitCamera::default();
    let start = egui::pos2(900.0, 300.0);
    let mut session = Session::for_rig(&scene, 0, OpKind::Rotate, None, true, start);
    session.update(&mut scene, &camera, view(), egui::pos2(900.0, 500.0));
    assert!(scene.rigs[0].placement.rot.angle_between(Quat::IDENTITY) > 0.01);
    assert!(scene.rig_frame(0).pos.distance(pivot) < 1e-4);
}

#[test]
fn ik_and_unparent_come_along_with_a_moved_rig() {
    let mut scene = sample("R15Rig.rbxm");
    let foot = bone_named(&scene, "RightAnkle");
    let held = bone_named(&scene, "LeftShoulder");
    modifiers::add_ik(&mut scene, foot);
    modifiers::add_unparent(&mut scene, held);
    let shift = Vec3::new(4.0, 0.0, 2.0);
    let (foot_was, held_was) = (scene.bones[foot].frame.pos, scene.bones[held].frame.pos);
    scene.set_rig_placement(0, Xf::new(Quat::IDENTITY, shift));
    assert!(scene.bones[foot].frame.pos.distance(foot_was + shift) < 1e-3);
    assert!(scene.bones[held].frame.pos.distance(held_was + shift) < 1e-3);

    // And something added once it's moved doesn't jump.
    let hand = bone_named(&scene, "RightWrist");
    let hand_was = scene.bones[hand].frame;
    modifiers::add_ik(&mut scene, hand);
    assert!(scene.bones[hand].frame.pos.distance(hand_was.pos) < 1e-3);
}

#[test]
fn deleting_a_joint_keeps_its_limb_where_it_was() {
    let mut scene = sample("R15Rig.rbxm");
    let elbow = bone_named(&scene, "RightElbow");
    let wrist = bone_named(&scene, "RightWrist");
    let shoulder = bone_named(&scene, "RightShoulder");
    let lower = scene.parts.iter().position(|p| p.name == "RightLowerArm").unwrap();
    let hand = scene.parts.iter().position(|p| p.name == "RightHand").unwrap();
    let (lower_at, hand_at) = (scene.parts[lower].world, scene.parts[hand].world);
    let parts_were = scene.live_parts();
    let gone = modifiers::delete_bones(&mut scene, &[elbow]);
    assert_eq!(gone, [elbow], "only the joint goes");
    assert!(!scene.bones[wrist].removed && !scene.bones[shoulder].removed);
    assert_eq!(scene.live_parts(), parts_were, "every part stays");
    assert!(scene.part_is_visible(lower) && scene.part_is_visible(hand));
    assert!(scene.parts[lower].world.approx_eq(lower_at) && scene.parts[hand].world.approx_eq(hand_at), "nothing moved");
    assert_eq!(scene.bones[wrist].parent, None, "the wrist now hangs from a part on its own");
    // Turning the shoulder no longer takes the lower arm along.
    scene.bones[shoulder].pose = Xf::new(Quat::from_rotation_x(1.0), Vec3::ZERO);
    scene.evaluate();
    assert!(scene.parts[lower].world.approx_eq(lower_at));
}

#[test]
fn deleting_comes_back_with_undo_state() {
    let mut scene = sample("R15Rig.rbxm");
    let elbow = bone_named(&scene, "RightElbow");
    let before = scene.rig_state();
    modifiers::delete_bones(&mut scene, &[elbow]);
    assert_ne!(scene.rig_state(), before);
    scene.restore_rig_state(&before);
    scene.evaluate();
    assert!(!scene.bones[elbow].removed);
}

#[test]
fn deleting_a_bone_takes_its_modifiers_and_their_targets() {
    let mut scene = sample("R15Rig.rbxm");
    let foot = bone_named(&scene, "RightAnkle");
    let target = modifiers::add_ik(&mut scene, foot);
    assert_eq!(scene.modifiers.len(), 1);
    let gone = modifiers::delete_bones(&mut scene, &[foot]);
    assert!(scene.modifiers.is_empty());
    assert!(gone.contains(&target) && scene.bones[target].removed);
}

#[test]
fn deleting_a_target_takes_its_modifier_and_leaves_the_bone() {
    let mut scene = sample("R15Rig.rbxm");
    let foot = bone_named(&scene, "RightAnkle");
    let target = modifiers::add_ik(&mut scene, foot);
    modifiers::delete_bones(&mut scene, &[target]);
    assert!(scene.modifiers.is_empty());
    assert!(scene.bones[target].removed && !scene.bones[foot].removed);
}

#[test]
fn what_followed_a_deleted_bone_stays_where_it_was() {
    let mut scene = sample("R15Rig.rbxm");
    let hand = bone_named(&scene, "RightWrist");
    let shape = scene.add_shape(PartShape::Block, Xf::new(Quat::IDENTITY, Vec3::new(3.0, 3.0, 3.0)), Vec3::ONE);
    scene.evaluate();
    modifiers::set_follow(&mut scene, shape, Some(hand));
    let at = scene.bones[shape].frame.pos;
    modifiers::delete_bones(&mut scene, &[hand]);
    assert!(!scene.bones[shape].removed);
    assert!(matches!(scene.bones[shape].link, BoneLink::Free { follow: None, .. }));
    assert!(scene.bones[shape].frame.pos.distance(at) < 1e-4);
}

#[test]
fn deleting_a_rig_or_a_welded_part() {
    let mut scene = sample("R6WithAccessories.rbxm");
    let welded = scene.parts.iter().position(|p| matches!(p.drive, kaye::scene::Drive::Rigid { .. }) && !p.is_helper()).expect("an accessory");
    let parts_were = scene.live_parts();
    let gone = modifiers::delete_part(&mut scene, welded);
    assert!(gone.is_empty());
    assert!(scene.live_parts() < parts_were);
    assert!(!scene.part_is_visible(welded));

    let gone = modifiers::delete_rig(&mut scene, 0);
    assert!(!gone.is_empty());
    assert!(scene.joints(0).is_empty());
    assert_eq!(scene.live_parts(), 0);
    assert!(scene.is_empty());
}

#[test]
fn inherit_rotation_keeps_a_head_looking_the_same_way() {
    let mut scene = sample("R15Rig.rbxm");
    let (neck, waist) = (bone_named(&scene, "Neck"), bone_named(&scene, "Waist"));
    let looking = scene.bones[neck].frame;
    modifiers::add_inherit(&mut scene, neck);
    assert!(scene.bones[neck].frame.rot.angle_between(looking.rot) < 1e-5, "adding it moves nothing");

    scene.set_bone_pose(waist, Xf::new(Quat::from_rotation_x(0.8) * Quat::from_rotation_y(0.5), Vec3::ZERO));
    let frame = scene.bones[neck].frame;
    assert!(frame.rot.angle_between(looking.rot) < 1e-4, "still looks the same way");
    assert!(frame.pos.distance(looking.pos) > 0.1, "but moved with the body");

    // Turning the head itself still works, on top of that.
    scene.set_bone_pose(neck, Xf::new(Quat::from_rotation_y(0.6), Vec3::ZERO));
    assert!((scene.bones[neck].frame.rot.angle_between(looking.rot) - 0.6).abs() < 1e-3);
}

#[test]
fn inherit_rotation_can_be_hidden_and_turns_with_a_moved_rig() {
    let mut scene = sample("R15Rig.rbxm");
    let (neck, waist) = (bone_named(&scene, "Neck"), bone_named(&scene, "Waist"));
    modifiers::add_inherit(&mut scene, neck);
    let looking = scene.bones[neck].frame.rot;
    let turned = Quat::from_rotation_y(1.0);

    scene.set_rig_placement(0, Xf::new(turned, Vec3::ZERO));
    assert!(scene.bones[neck].frame.rot.angle_between(turned * looking) < 1e-4);
    scene.set_rig_placement(0, Xf::IDENTITY);

    scene.set_bone_pose(waist, Xf::new(Quat::from_rotation_x(0.8), Vec3::ZERO));
    scene.modifiers[0].enabled = false;
    scene.evaluate();
    assert!(scene.bones[neck].frame.rot.angle_between(looking) > 0.5, "with it hidden the head follows");
}

#[test]
fn inherit_rotation_exports_and_bakes_to_the_same_motion() {
    let mut scene = sample("R15Rig.rbxm");
    let (neck, waist) = (bone_named(&scene, "Neck"), bone_named(&scene, "Waist"));
    modifiers::add_inherit(&mut scene, neck);
    let mut animation = Animation { end: 10, ..Animation::default() };
    animation.set_key(waist, 0, Xf::IDENTITY);
    animation.set_key(waist, 10, Xf::new(Quat::from_rotation_x(0.9), Vec3::ZERO));

    animation.apply(&mut scene, 10.0);
    let held = scene.bones[neck].frame.rot;
    let output = scene.bones[neck].output();
    assert!(output.rot.angle_between(Quat::IDENTITY) > 0.5, "the real joint has to turn back against the body");

    let index = scene.modifiers.iter().position(|m| m.bone == neck).unwrap();
    posing::bake(&mut scene, &mut animation, &[index]);
    assert!(scene.modifiers.is_empty());
    animation.apply(&mut scene, 10.0);
    assert!(scene.bones[neck].frame.rot.angle_between(held) < 1e-3, "baked keys land in the same place");
}

#[test]
fn pasted_keys_land_with_the_earliest_on_the_current_frame() {
    let scene = sample("R15Rig.rbxm");
    let (arm, leg) = (bone_named(&scene, "RightShoulder"), bone_named(&scene, "LeftHip"));
    let pose = Xf::new(Quat::from_rotation_x(1.0), Vec3::ZERO);
    let mut animation = Animation::default();
    animation.set_key(arm, 10, pose);
    animation.set_key(leg, 14, pose);

    let copied = posing::copy_keys(&scene, &animation, &[(arm, 10), (leg, 14)]);
    let pasted = posing::paste_keys(&scene, &mut animation, &copied, Some(0), 40);
    assert_eq!(pasted.len(), 2);
    assert!(pasted.contains(&(arm, 40)) && pasted.contains(&(leg, 44)));
    assert!(animation.key_at(arm, 40).unwrap().pose.approx_eq(pose));
    assert!(animation.key_at(arm, 10).is_some(), "the originals stay");
}

#[test]
fn pasted_keys_find_the_same_bone_on_another_rig_and_keep_a_cameras_keys() {
    let mut scene = sample("R15Rig.rbxm");
    scene.append(sample("R15Rig.rbxm").clone());
    scene.evaluate();
    let first = bone_named(&scene, "RightShoulder");
    let second = scene.joints(1).into_iter().find(|b| scene.bones[*b].name == "RightShoulder").unwrap();
    let camera = scene.add_camera(Xf::IDENTITY, 70.0);
    let mut animation = Animation::default();
    animation.set_key(first, 5, Xf::IDENTITY);
    animation.set_key(camera, 5, Xf::IDENTITY);
    let copied = posing::copy_keys(&scene, &animation, &[(first, 5), (camera, 5)]);
    let pasted = posing::paste_keys(&scene, &mut animation, &copied, Some(1), 9);
    assert!(pasted.contains(&(second, 9)), "same name on the rig in focus");
    assert!(pasted.contains(&(camera, 9)), "a camera isn't on any rig, so it stays itself");
}

#[test]
fn nothing_copied_pastes_nothing() {
    let scene = sample("R6Rig.rbxm");
    let mut animation = Animation::default();
    assert!(posing::paste_keys(&scene, &mut animation, &[], Some(0), 0).is_empty());
}

#[test]
fn placement_deleted_parts_and_inherit_survive_saving() {
    let mut scene = sample("R6WithAccessories.rbxm");
    let neck = bone_named(&scene, "Neck");
    let welded = scene.parts.iter().position(|p| matches!(p.drive, kaye::scene::Drive::Rigid { .. }) && !p.is_helper()).unwrap();
    let placement = Xf::new(Quat::from_rotation_y(0.4), Vec3::new(1.0, 2.0, 3.0));
    scene.set_rig_placement(0, placement);
    modifiers::add_inherit(&mut scene, neck);
    modifiers::delete_part(&mut scene, welded);
    let view = project::View { frame: 0.0, target: Vec3::ZERO, yaw: 0.0, pitch: 0.0, dist: 10.0, ortho: false, floor_y: 0.0, camera: None, preview: false, through: false, motion_paths: Vec::new(), references: Vec::new() };
    let path = std::env::temp_dir().join(format!("kaye-workspace-{}.kaye", std::process::id()));
    project::save(&path, &scene, &Animation::default(), &view).unwrap();
    let loaded = project::load(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(loaded.scene.rigs[0].placement, placement);
    assert!(loaded.scene.parts[welded].removed);
    assert_eq!(loaded.scene.modifiers.len(), 1);
    assert_eq!(loaded.scene.rigs[0].model.as_ref().map(|m| m.len()), scene.rigs[0].model.as_ref().map(|m| m.len()));
    assert!(export::model_with_animations(&loaded.scene, &Animation::default(), 0).is_err(), "nothing moves yet");
}

#[test]
fn moving_a_rig_counts_as_unsaved_work() {
    let mut scene = sample("R6Rig.rbxm");
    let animation = Animation::default();
    let before = project::fingerprint(&scene, &animation);
    scene.set_rig_placement(0, Xf::new(Quat::IDENTITY, Vec3::X));
    assert_ne!(project::fingerprint(&scene, &animation), before);
}
