//! Keys that only set some channels, as Blender keys one axis at a time, and Smooth Loop,
//! where the curves wrap from the last key into the first.

use std::path::Path;

use glam::{EulerRot, Quat, Vec3};
use kaye::animation::{ALL_CHANNELS, Animation, Easing, Smoothing, Track, Wrap, changed_channels};
use kaye::import::import_file;
use kaye::math::Xf;
use kaye::scene::Scene;

const POS_X: usize = 0;
const ROT_Y: usize = 4;

fn rig() -> Scene {
    import_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/R15Rig.rbxm")).unwrap().scene
}

fn turned_y(degrees: f32) -> Quat {
    Quat::from_rotation_y(degrees.to_radians())
}

fn angle_y(rot: Quat) -> f32 {
    rot.to_euler(EulerRot::XYZ).1.to_degrees()
}

#[test]
fn only_what_changed_counts_as_changed() {
    let before = (Xf::new(Quat::IDENTITY, Vec3::new(1.0, 2.0, 3.0)), 1.0);
    let turned = (Xf::new(turned_y(30.0), Vec3::new(1.0, 2.0, 3.0)), 1.0);
    assert_eq!(changed_channels(before, turned), 1 << ROT_Y);
    let moved = (Xf::new(Quat::IDENTITY, Vec3::new(1.0, 2.5, 3.0)), 1.0);
    assert_eq!(changed_channels(before, moved), 1 << 1);
    let both = (Xf::new(Quat::IDENTITY, Vec3::new(4.0, 2.0, 0.0)), 0.5);
    assert_eq!(changed_channels(before, both), 1 | 1 << 2 | 1 << 6);
    assert_eq!(changed_channels(before, before), 0);
}

#[test]
fn a_channel_no_key_sets_keeps_the_bones_own_value() {
    let mut track = Track::default();
    let only_y = 1 << ROT_Y;
    track.set_key_channels(0, Xf::new(Quat::IDENTITY, Vec3::ZERO), 1.0, Easing::default(), only_y);
    track.set_key_channels(10, Xf::new(turned_y(40.0), Vec3::ZERO), 1.0, Easing::default(), only_y);
    let own = (Xf::new(Quat::IDENTITY, Vec3::new(3.0, -1.0, 2.0)), 0.25);
    let (pose, value) = track.sample_onto(5.0, Smoothing::None, own).unwrap();
    assert!((pose.pos - own.0.pos).length() < 1e-5, "position stays where the bone has it: {:?}", pose.pos);
    assert!((value - 0.25).abs() < 1e-6);
    assert!((angle_y(pose.rot) - 20.0).abs() < 0.5, "halfway round Y: {}", angle_y(pose.rot));
}

#[test]
fn a_channels_curve_only_runs_through_keys_that_set_it() {
    let mut track = Track::default();
    track.set_key_full(0, Xf::new(Quat::IDENTITY, Vec3::ZERO), 1.0, Easing::default());
    // This key only turns Y, so Position X runs straight past it.
    track.set_key_channels(10, Xf::new(turned_y(50.0), Vec3::new(99.0, 0.0, 0.0)), 1.0, Easing::default(), 1 << ROT_Y);
    track.set_key_full(20, Xf::new(Quat::IDENTITY, Vec3::new(10.0, 0.0, 0.0)), 1.0, Easing::default());
    let (pose, _) = track.sample_full(10.0, Smoothing::None).unwrap();
    assert!((pose.pos.x - 5.0).abs() < 1e-3, "halfway from the first to the last: {}", pose.pos.x);
    assert!((angle_y(pose.rot) - 50.0).abs() < 1e-2, "but Y does stop on it");
    let curves = track.curves(Smoothing::None);
    assert!((curves.points()[POS_X][1].value - 5.0).abs() < 1e-3, "the key sits on the X curve, not at 99");
    let drawn = curves.at(10.0).unwrap();
    assert!((drawn[POS_X] - pose.pos.x).abs() < 1e-4 && (drawn[ROT_Y] - 50.0).abs() < 1e-2, "the Graph tab draws what plays");
}

#[test]
fn keys_that_set_everything_play_exactly_as_before() {
    let mut whole = Track::default();
    let mut marked = Track::default();
    let poses = [(0, Xf::IDENTITY), (8, Xf::new(Quat::from_rotation_x(1.1) * turned_y(20.0), Vec3::new(0.0, 1.0, 0.5))), (20, Xf::new(Quat::from_rotation_z(-0.4), Vec3::new(2.0, 0.0, 0.0)))];
    for (frame, pose) in poses {
        whole.set_key_full(frame, pose, 1.0, Easing::default());
        marked.set_key_channels(frame, pose, 1.0, Easing::default(), ALL_CHANNELS);
    }
    for frame in 0..=20 {
        let (a, b) = (whole.sample(frame as f32).unwrap(), marked.sample(frame as f32).unwrap());
        assert!(a.approx_eq(b), "frame {frame}");
    }
}

#[test]
fn auto_key_only_keys_the_channels_that_moved() {
    let scene = rig();
    let bone = scene.bones.iter().position(|b| b.name == "RightShoulder").unwrap();
    let mut animation = Animation::default();
    let before = Xf::IDENTITY;
    animation.record_edit(bone, 10, before, Xf::new(turned_y(25.0), Vec3::ZERO), 1.0, true);
    let track = &animation.tracks[&bone];
    assert_eq!(track.keys.len(), 2, "the start frame keeps where it was");
    assert!(track.keys.iter().all(|k| k.channels == 1 << ROT_Y), "{:?}", track.keys.iter().map(|k| k.channels).collect::<Vec<_>>());
    animation.record_edit(bone, 10, Xf::new(turned_y(25.0), Vec3::ZERO), Xf::new(turned_y(25.0), Vec3::new(0.0, 0.0, 1.0)), 1.0, true);
    assert_eq!(animation.tracks[&bone].key_at(10).unwrap().channels, 1 << ROT_Y | 1 << 2, "a key already there takes on the new channel too");
    animation.key_shown(bone, 10, Xf::IDENTITY, 1.0);
    assert!(animation.tracks[&bone].key_at(10).unwrap().sets_all(), "Insert Keyframe keys everything");
}

#[test]
fn older_keys_with_no_channels_set_everything_and_whole_keys_save_without_them() {
    let mut track = Track::default();
    track.set_key(4, Xf::new(turned_y(10.0), Vec3::ONE));
    let text = serde_json::to_string(&track).unwrap();
    assert!(!text.contains("channels"), "{text}");
    let back: Track = serde_json::from_str(&text).unwrap();
    assert!(back.keys[0].sets_all());
    track.set_key_channels(8, Xf::IDENTITY, 1.0, Easing::default(), 1 << POS_X);
    let back: Track = serde_json::from_str(&serde_json::to_string(&track).unwrap()).unwrap();
    assert_eq!(back.keys[1].channels, 1 << POS_X);
}

// Smooth Loop.

/// Position X rising 5, 10, then back to 0, over a 15 frame loop.
fn rising() -> Track {
    let mut track = Track::default();
    for (frame, x) in [(0, 5.0), (5, 10.0), (10, 0.0)] {
        track.set_key(frame, Xf::new(Quat::IDENTITY, Vec3::new(x, 0.0, 0.0)));
    }
    track
}

fn x_at(track: &Track, frame: f32, wrap: Option<Wrap>) -> f32 {
    track.sample_looped(frame, Smoothing::None, (Xf::IDENTITY, 1.0), wrap).unwrap().0.pos.x
}

#[test]
fn a_smooth_loop_eases_from_the_last_key_back_into_the_first() {
    let track = rising();
    let wrap = Some(Wrap { start: 0, end: 15 });
    assert!((x_at(&track, 12.5, None) - 0.0).abs() < 1e-5, "without it the last key holds");
    let wrapped = x_at(&track, 12.5, wrap);
    assert!(wrapped > 0.5 && wrapped < 4.5, "with it, on its way back to 5: {wrapped}");
    assert!((x_at(&track, 15.0, wrap) - 5.0).abs() < 1e-3, "and arrives as the loop starts again");
}

#[test]
fn a_smooth_loop_has_no_corner_at_the_join() {
    let track = rising();
    let wrap = Some(Wrap { start: 0, end: 15 });
    let slope = |frame: f32, wrap| (x_at(&track, frame + 0.01, wrap) - x_at(&track, frame - 0.01, wrap)) / 0.02;
    assert!(slope(0.01, None).abs() < 0.2, "a plain first key starts flat");
    let leaving = (x_at(&track, 0.02, wrap) - x_at(&track, 0.0, wrap)) / 0.02;
    let arriving = (x_at(&track, 15.0, wrap) - x_at(&track, 14.98, wrap)) / 0.02;
    assert!(leaving > 0.3, "the first key's handles see the last key: {leaving}");
    assert!((leaving - arriving).abs() < 0.1, "the same slope both sides of the join: {leaving} vs {arriving}");
}

#[test]
fn a_last_key_a_loop_after_the_first_is_the_same_moment() {
    let mut track = rising();
    track.set_key(15, Xf::new(Quat::IDENTITY, Vec3::new(5.0, 0.0, 0.0)));
    let wrap = Some(Wrap { start: 0, end: 15 });
    let without_it = rising();
    for frame in [0.0, 2.0, 7.5, 12.0, 14.0, 15.0] {
        assert!((x_at(&track, frame, wrap) - x_at(&without_it, frame, wrap)).abs() < 1e-3, "frame {frame}");
    }
}

#[test]
fn smooth_loop_only_wraps_with_loop_on() {
    let mut animation = Animation { end: 15, ..Animation::default() };
    let scene = rig();
    animation.ensure_clips(&scene);
    animation.show_clip(animation.active_clip(0).unwrap());
    assert_eq!(animation.wrap_of(0), None, "off by default");
    animation.smooth_loop = true;
    assert_eq!(animation.wrap_of(0), Some(Wrap { start: 0, end: 15 }));
    animation.looping = false;
    assert_eq!(animation.wrap_of(0), None, "needs Loop too");
}

#[test]
fn the_graph_draws_a_smooth_loop_as_it_plays() {
    let track = rising();
    let wrap = Some(Wrap { start: 0, end: 15 });
    let curves = track.curves_looped(Smoothing::None, wrap);
    for frame in [0.5, 4.0, 11.0, 13.5] {
        assert!((curves.at(frame).unwrap()[POS_X] - x_at(&track, frame, wrap)).abs() < 1e-4, "frame {frame}");
    }
    let plain = track.curves(Smoothing::None);
    assert!(curves.points()[POS_X][0].right[1] > plain.points()[POS_X][0].right[1] + 0.1, "the first key's handle tips up toward the join");
}
