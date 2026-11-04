//! Editing curves on the Graph tab, and cameras as animated objects.

use std::collections::HashSet;
use std::path::Path;

use glam::{EulerRot, Quat, Vec3};
use kaye::animation::{Animation, CHANNELS, EaseDirection, EaseStyle, Easing, Handles, Smoothing, Track, VALUE_CHANNEL, angles_near};
use kaye::history::History;
use kaye::import::import_file;
use kaye::math::Xf;
use kaye::scene::{BoneKind, Scene};
use kaye::settings::{GraphicsApi, Settings};

const ROT_X: usize = 3;
const ROT_Y: usize = 4;

fn rig() -> Scene {
    import_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/R15Rig.rbxm")).unwrap().scene
}

/// Curves are worked out once and kept, so every way of changing keys has to drop them.
#[test]
fn kept_curves_follow_every_change_to_the_keys() {
    let smoothing = Smoothing::Continuous;
    let fresh = |track: &Track| {
        let copy: Vec<_> = track.keys.iter().copied().collect();
        let mut rebuilt = Track::default();
        copy.into_iter().for_each(|key| rebuilt.insert(key));
        rebuilt.curves(smoothing).points[ROT_X].clone()
    };
    let mut track = track_of(&[(0, 0.0), (10, 40.0), (20, -10.0), (30, 25.0)]);
    assert_eq!(track.curves(smoothing).points[ROT_X], fresh(&track));

    track.set_key(15, Xf::new(glam::Quat::from_rotation_x(0.5), glam::Vec3::ZERO));
    assert_eq!(track.curves(smoothing).points[ROT_X], fresh(&track), "after adding a key");
    track.remove(20);
    assert_eq!(track.curves(smoothing).points[ROT_X], fresh(&track), "after removing a key");
    track.set_channel(1, ROT_X, 70.0);
    assert_eq!(track.curves(smoothing).points[ROT_X], fresh(&track), "after a typed value");
    track.keys[1].frame = 12;
    assert_eq!(track.curves(smoothing).points[ROT_X], fresh(&track), "after a direct change");
    track.set_handle(1, ROT_X, true, [2.0, 9.0], false, smoothing);
    assert_eq!(track.curves(smoothing).points[ROT_X], fresh(&track), "after dragging a handle");
}

#[test]
fn kept_curves_stay_separate_per_smoothing_and_per_copy() {
    let mut track = track_of(&[(0, 0.0), (10, 40.0), (20, 35.0), (30, 90.0)]);
    let (none, continuous) = (track.curves(Smoothing::None).points[ROT_X].clone(), track.curves(Smoothing::Continuous).points[ROT_X].clone());
    assert_ne!(none, continuous);

    let copy = track.clone();
    track.set_channel(1, ROT_X, -20.0);
    assert_eq!(copy.curves(Smoothing::None).points[ROT_X], none, "a copy keeps the curves of its own keys");
    assert_ne!(track.curves(Smoothing::None).points[ROT_X], none);
    assert_eq!(copy.keys[1].frame, 10);
}

/// Undo relies on this: a copy of an animation shares what an edit didn't touch.
#[test]
fn copies_share_keys_until_one_changes() {
    let mut animation = Animation::default();
    for bone in 0..3 {
        animation.set_key(bone, 0, Xf::IDENTITY);
        animation.set_key(bone, 10, Xf::IDENTITY);
    }
    let copy = animation.clone();
    let same = |a: &Animation, b: &Animation, bone: usize| std::ptr::eq(a.tracks[&bone].keys.as_ptr(), b.tracks[&bone].keys.as_ptr());
    assert!((0..3).all(|bone| same(&animation, &copy, bone)));

    animation.set_key(1, 5, Xf::IDENTITY);
    assert!(same(&animation, &copy, 0) && same(&animation, &copy, 2), "untouched bones are still shared");
    assert!(!same(&animation, &copy, 1));
    assert_eq!(copy.tracks[&1].keys.len(), 2, "the copy didn't change");
    assert_eq!(animation.tracks[&1].keys.len(), 3);
}

fn track_of(keys: &[(i32, f32)]) -> Track {
    let mut track = Track::default();
    for (frame, x) in keys {
        track.set_key(*frame, Xf::new(Quat::from_rotation_x(x.to_radians()), Vec3::new(0.0, *x * 0.01, 0.0)));
    }
    track
}

fn slope(track: &Track, channel: usize, frame: f32, smoothing: Smoothing) -> f32 {
    let curves = track.curves(smoothing);
    (curves.at(frame + 0.05).unwrap()[channel] - curves.at(frame - 0.05).unwrap()[channel]) / 0.1
}

#[test]
fn both_ways_of_writing_a_rotation_are_the_same_rotation() {
    let q = Quat::from_euler(EulerRot::XYZ, 0.7, 1.9, -0.4);
    for near in [[0.0, 0.0, 0.0], [170.0, 60.0, 150.0], [-300.0, 800.0, 40.0]] {
        let a = angles_near(q, near);
        let back = Quat::from_euler(EulerRot::XYZ, a[0].to_radians(), a[1].to_radians(), a[2].to_radians());
        assert!(back.dot(q).abs() > 0.99999, "{a:?} near {near:?}");
    }
    // Past 90 degrees on Y is where a plain conversion flips the other two axes.
    let tilted = Quat::from_euler(EulerRot::XYZ, 0.0, 120f32.to_radians(), 0.0);
    let a = angles_near(tilted, [0.0, 110.0, 0.0]);
    assert!((a[1] - 120.0).abs() < 1e-2 && a[0].abs() < 1e-2 && a[2].abs() < 1e-2, "{a:?}");
}

#[test]
fn the_drawn_curve_is_exactly_what_plays() {
    let mut track = Track::default();
    track.set_key(0, Xf::new(Quat::from_rotation_y(0.2), Vec3::ZERO));
    track.set_key(10, Xf::new(Quat::from_euler(EulerRot::XYZ, 0.9, -0.4, 0.5), Vec3::new(1.0, 2.0, 0.0)));
    track.set_key(30, Xf::new(Quat::from_euler(EulerRot::XYZ, -0.6, 0.8, 1.2), Vec3::new(-1.0, 0.0, 3.0)));
    track.set_key(45, Xf::new(Quat::from_rotation_z(0.3), Vec3::ZERO));
    track.keys[1].easing = Easing { style: EaseStyle::Elastic, direction: EaseDirection::Out, ..Easing::default() };
    track.keys[2].easing = Easing { style: EaseStyle::Linear, ..Easing::default() };
    track.set_handle(0, ROT_X, true, [3.0, 40.0], true, Smoothing::Continuous);

    let curves = track.curves(Smoothing::Continuous);
    for i in -20..=500 {
        let frame = i as f32 * 0.1;
        let drawn = curves.at(frame).unwrap();
        let played = track.sample_with(frame, Smoothing::Continuous).unwrap();
        assert!(played.pos.distance(Vec3::new(drawn[0], drawn[1], drawn[2])) < 1e-3, "position at {frame}");
        let rot = Quat::from_euler(EulerRot::XYZ, drawn[3].to_radians(), drawn[4].to_radians(), drawn[5].to_radians());
        assert!(rot.dot(played.rot).abs() > 0.99995, "rotation at {frame}: {drawn:?}");
    }
}

#[test]
fn curves_pass_through_every_key_and_stay_continuous_past_180() {
    let track = track_of(&[(0, 0.0), (10, 120.0), (20, 240.0), (30, 359.0), (40, 480.0)]);
    let curves = track.curves(Smoothing::Continuous);
    for (frame, angle) in [(0.0, 0.0), (10.0, 120.0), (20.0, 240.0), (30.0, 359.0), (40.0, 480.0)] {
        assert!((curves.at(frame).unwrap()[ROT_X] - angle).abs() < 0.01, "at {frame}");
    }
    let mut last = curves.at(0.0).unwrap()[ROT_X];
    for i in 1..=400 {
        let now = curves.at(i as f32 * 0.1).unwrap()[ROT_X];
        assert!((now - last).abs() < 8.0, "jumped from {last} to {now}");
        last = now;
    }
}

#[test]
fn moving_a_key_on_one_channel_leaves_the_others_alone() {
    let mut track = Track::default();
    let pose = Xf::new(Quat::from_euler(EulerRot::XYZ, 0.4, 0.5, 0.6), Vec3::new(1.0, 2.0, 3.0));
    track.set_key(0, pose);
    track.set_key(10, pose);
    let before = track.key_channels()[1];
    track.set_channel(1, ROT_Y, 55.0);
    let after = track.key_channels()[1];
    assert!((after[ROT_Y] - 55.0).abs() < 1e-2, "got {}", after[ROT_Y]);
    for channel in (0..CHANNELS).filter(|c| *c != ROT_Y) {
        assert!((after[channel] - before[channel]).abs() < 1e-2, "channel {channel} moved");
    }
    track.set_channel(1, 1, 9.0);
    assert_eq!(track.keys[1].pose.pos.y, 9.0);
    track.set_channel(1, VALUE_CHANNEL, 0.25);
    assert_eq!(track.keys[1].value, 0.25);
}

#[test]
fn rotation_keys_cant_be_dragged_past_the_short_way() {
    let mut track = track_of(&[(0, 0.0), (10, 0.0)]);
    track.set_channel(1, ROT_X, 300.0);
    let value = track.key_channels()[1][ROT_X];
    assert!((value - 179.0).abs() < 0.1, "stopped at {value}");
}

#[test]
fn dragging_a_handle_changes_how_motion_speeds_up() {
    let smoothing = Smoothing::Continuous;
    let mut track = track_of(&[(0, 0.0), (20, 90.0)]);
    assert!(slope(&track, ROT_X, 0.1, smoothing).abs() < 1.0, "clamped curves start at rest");

    // Pull the right handle up, so it leaves fast.
    track.set_handle(0, ROT_X, true, [5.0, 40.0], true, smoothing);
    assert!(slope(&track, ROT_X, 0.2, smoothing) > 4.0, "started at {}", slope(&track, ROT_X, 0.2, smoothing));
    let curves = track.curves(smoothing);
    assert!((curves.at(0.0).unwrap()[ROT_X]).abs() < 1e-3, "the key itself doesn't move");
    assert!((curves.at(20.0).unwrap()[ROT_X] - 90.0).abs() < 1e-2);

    let other = track.keys[0].handles[ROT_Y];
    assert!(other.is_none(), "other channels keep their automatic handles");
}

#[test]
fn aligned_handles_stay_smooth_and_broken_ones_make_a_corner() {
    let smoothing = Smoothing::None;
    let mut track = track_of(&[(0, 0.0), (10, 45.0), (20, 90.0)]);
    track.set_handle(1, ROT_X, true, [4.0, 30.0], true, smoothing);
    let (before, after) = (slope(&track, ROT_X, 9.9, smoothing), slope(&track, ROT_X, 10.1, smoothing));
    assert!((before - after).abs() < 2.0, "aligned should be smooth: {before} then {after}");

    track.set_handle(1, ROT_X, true, [4.0, -10.0], false, smoothing);
    let (before, after) = (slope(&track, ROT_X, 9.9, smoothing), slope(&track, ROT_X, 10.1, smoothing));
    assert!(before > 0.0 && after < 0.0, "free handles can make a corner: {before} then {after}");
    assert_eq!(track.handle_mode(1, ROT_X), Some(false));
}

#[test]
fn a_handle_can_never_make_time_run_backward() {
    let smoothing = Smoothing::Continuous;
    let mut track = track_of(&[(0, 0.0), (10, 45.0)]);
    track.set_handle(0, ROT_X, true, [500.0, 900.0], false, smoothing);
    track.set_handle(1, ROT_X, false, [-500.0, -900.0], false, smoothing);
    let curves = track.curves(smoothing);
    let mut last = curves.at(0.0).unwrap()[ROT_X];
    for i in 1..=100 {
        let now = curves.at(i as f32 * 0.1).unwrap()[ROT_X];
        assert!(now.is_finite() && (now - last).abs() < 60.0, "jumped to {now}");
        last = now;
    }
    // A handle dragged across its key stays on its own side.
    track.set_handle(0, ROT_X, true, [-5.0, 10.0], false, smoothing);
    assert!(track.keys[0].handles[ROT_X].unwrap().right[0] > 0.0);
    track.set_handle(1, ROT_X, false, [5.0, 10.0], false, smoothing);
    assert!(track.keys[1].handles[ROT_X].unwrap().left[0] < 0.0);
}

#[test]
fn automatic_neighbors_work_around_a_handle_set_by_hand() {
    let smoothing = Smoothing::Continuous;
    let mut plain = track_of(&[(0, 0.0), (10, 10.0), (20, 20.0), (30, 30.0)]);
    let before = plain.curves(smoothing).points[ROT_X][2];
    plain.keys[1].handles[ROT_X] = Some(Handles { left: [-3.0, 0.0], right: [3.0, 0.0], aligned: true });
    let after = plain.curves(smoothing).points[ROT_X][2];
    assert!((before.right[1] - after.right[1]).abs() > 1e-3, "smoothing should react to the fixed key");
}

#[test]
fn pinning_handles_keeps_the_curve_exactly_as_it_was() {
    let smoothing = Smoothing::Continuous;
    let mut track = track_of(&[(0, 0.0), (10, 30.0), (20, 25.0), (35, 90.0)]);
    let before: Vec<f32> = (0..350).map(|i| track.curves(smoothing).at(i as f32 * 0.1).unwrap()[ROT_X]).collect();
    for index in 0..track.keys.len() {
        track.pin_handles(index, ROT_X, true, smoothing);
    }
    for (i, value) in before.iter().enumerate() {
        let now = track.curves(smoothing).at(i as f32 * 0.1).unwrap()[ROT_X];
        assert!((now - value).abs() < 0.05, "frame {}: {value} became {now}", i as f32 * 0.1);
    }
}

#[test]
fn handles_travel_with_a_key_when_it_moves() {
    let mut animation = Animation::default();
    animation.set_key(1, 0, Xf::IDENTITY);
    animation.set_key(1, 20, Xf::IDENTITY);
    let set = Handles { left: [-2.0, 1.0], right: [2.0, -1.0], aligned: true };
    animation.tracks.get_mut(&1).unwrap().keys[1].handles[ROT_X] = Some(set);
    let keys: HashSet<_> = [(1, 20)].into_iter().collect();
    let moved = animation.moved(&keys, 10);
    assert_eq!(moved.key_at(1, 30).unwrap().handles[ROT_X], Some(set));
}

#[test]
fn undo_puts_a_curve_edit_back() {
    let mut scene = rig();
    let mut animation = Animation::default();
    animation.set_key(0, 0, Xf::IDENTITY);
    animation.set_key(0, 10, Xf::new(Quat::from_rotation_x(1.0), Vec3::ZERO));
    let mut history = History::default();

    let before = animation.clone();
    let track = animation.tracks.get_mut(&0).unwrap();
    track.set_channel(1, ROT_X, 20.0);
    track.set_handle(0, ROT_X, true, [3.0, 9.0], true, Smoothing::None);
    history.push_edit("Edit curve", Vec::new(), before.clone(), animation.clone());

    history.undo(&mut scene, &mut animation).unwrap();
    assert_eq!(animation, before);
}

#[test]
fn cameras_are_keyed_like_bones_and_their_lens_animates() {
    let mut scene = rig();
    let joints = scene.joints(0).len();
    let bounds = scene.bounds(None).unwrap();
    let start = Xf::new(Quat::from_rotation_y(0.5), Vec3::new(50.0, 40.0, 30.0));
    let camera = scene.add_camera(start, 70.0);
    scene.evaluate();
    assert_eq!(scene.bones[camera].kind, BoneKind::Camera);
    assert!(scene.bones[camera].frame.approx_eq(start), "its pose is where it is in the world");
    assert_eq!(scene.joints(0).len(), joints, "the rig doesn't gain a bone");
    assert_eq!(scene.cameras().collect::<Vec<_>>(), [camera]);
    let framed = scene.bounds(None).unwrap();
    assert!(framed.1.distance(bounds.1) < 1e-3, "Frame All shouldn't fly out to the camera");

    let mut animation = Animation::default();
    let end = Xf::new(Quat::IDENTITY, Vec3::new(0.0, 5.0, 20.0));
    animation.set_key_full(camera, 0, start, 70.0);
    animation.set_key_full(camera, 30, end, 30.0);
    animation.apply(&mut scene, 30.0);
    assert!(scene.bones[camera].frame.pos.distance(end.pos) < 1e-3);
    assert_eq!(scene.bones[camera].value, 30.0);
    animation.apply(&mut scene, 15.0);
    let fov = scene.bones[camera].value;
    assert!(fov > 30.0 && fov < 70.0, "the field of view should be between keys: {fov}");
    let curves = animation.tracks[&camera].curves(animation.smoothing);
    assert!((curves.at(15.0).unwrap()[VALUE_CHANNEL] - fov).abs() < 1e-3, "and on the Graph tab");
}

#[test]
fn an_empty_scene_with_a_camera_still_offers_to_import() {
    let mut scene = Scene::default();
    scene.add_camera(Xf::IDENTITY, 70.0);
    assert!(scene.is_empty());
    let imported = rig();
    scene.append(imported);
    scene.evaluate();
    assert!(!scene.is_empty());
    assert_eq!(scene.cameras().count(), 1, "importing keeps the camera");
    assert_eq!(scene.bones[scene.cameras().next().unwrap()].rig, 0);
    assert!(scene.joints(1).len() > 10);
}

#[test]
fn undo_takes_a_new_camera_back_out_and_each_gets_its_own_name() {
    let mut scene = rig();
    let mut animation = Animation::default();
    let mut history = History::default();
    let before = (scene.rig_state(), animation.clone());
    let first = scene.add_camera(Xf::IDENTITY, 70.0);
    history.push_rig("Add camera", before, (scene.rig_state(), animation.clone()));
    let second = scene.add_camera(Xf::IDENTITY, 70.0);
    assert_eq!(scene.bones[first].name, "Camera");
    assert_eq!(scene.bones[second].name, "Camera 2");

    history.undo(&mut scene, &mut animation).unwrap();
    assert!(scene.bones[first].removed && scene.bones[second].removed);
    assert_eq!(scene.cameras().count(), 0);
    history.redo(&mut scene, &mut animation).unwrap();
    assert!(!scene.bones[first].removed);
}

#[test]
fn old_settings_files_load_with_automatic_graphics() {
    let old: Settings = serde_json::from_str(r#"{"system":{"vsync":false}}"#).unwrap();
    assert_eq!(old.system.graphics_api, GraphicsApi::Auto);
    assert!(!old.system.vsync);

    let mut settings = Settings::default();
    settings.system.graphics_api = GraphicsApi::Vulkan;
    let text = serde_json::to_string(&settings).unwrap();
    assert_eq!(serde_json::from_str::<Settings>(&text).unwrap().system.graphics_api, GraphicsApi::Vulkan);
    assert!(GraphicsApi::Auto.exists_here() && GraphicsApi::OpenGl.exists_here());
}

#[test]
fn old_settings_files_follow_the_new_look_but_keep_real_choices() {
    // A file saved before the look changed holds the old defaults, plus one color the
    // user really picked.
    let old: Settings = serde_json::from_str(
        r#"{"theme":{"panel":[32,32,32],"header":[43,43,43],"field":[10,20,30],"text":[242,242,242],"accent":[1,2,3]},
            "viewport":{"background":"Solid","background_top":[43,43,43],"background_bottom":[33,33,33]},
            "animation":{"tick_spacing":48.0}}"#,
    )
    .unwrap();
    assert_eq!(old.look, 0);
    let new = old.upgraded();
    let defaults = Settings::default();
    assert_eq!(new.theme.panel, defaults.theme.panel);
    assert_eq!(new.theme.header, defaults.theme.header);
    assert_eq!(new.theme.text, defaults.theme.text);
    assert_eq!(new.viewport.background_top, defaults.viewport.background_top);
    assert_eq!(new.viewport.background_bottom, defaults.viewport.background_bottom);
    assert_eq!(new.animation.tick_spacing, defaults.animation.tick_spacing);
    assert_eq!(new.theme.field, [10, 20, 30]);
    assert_eq!(new.theme.accent, [1, 2, 3]);

    // Once upgraded, picking an old default color on purpose has to stick.
    let mut chosen = new;
    chosen.theme.panel = [32, 32, 32];
    let text = serde_json::to_string(&chosen).unwrap();
    let reloaded = serde_json::from_str::<Settings>(&text).unwrap().upgraded();
    assert_eq!(reloaded.theme.panel, [32, 32, 32]);
}
