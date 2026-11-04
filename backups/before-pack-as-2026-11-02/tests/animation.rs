//! Keyframes, easing and playback, run against a real Studio rig where a rig matters.

use std::collections::HashSet;
use std::path::Path;

use glam::{Quat, Vec3};
use kaye::animation::{Animation, EaseDirection, EaseStyle, Easing, HandleType, Smoothing, Track};
use kaye::history::{History, PoseChange};
use kaye::import::import_file;
use kaye::math::Xf;
use kaye::scene::Scene;

fn rig() -> Scene {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/R6Rig.rbxm");
    import_file(&path).unwrap().scene
}

fn bone(s: &Scene, name: &str) -> usize {
    s.bones.iter().position(|b| b.name == name).unwrap()
}

fn turned(angle: f32) -> Xf {
    Xf::new(Quat::from_rotation_z(angle), Vec3::ZERO)
}

fn linear() -> Easing {
    Easing {
        style: EaseStyle::Linear,
        ..Easing::default()
    }
}

fn angle_of(pose: Xf) -> f32 {
    pose.rot.to_euler(glam::EulerRot::XYZ).2
}

#[test]
fn every_easing_starts_at_zero_and_ends_at_one() {
    for style in EaseStyle::ALL {
        for direction in EaseDirection::ALL {
            let e = Easing {
                style,
                direction,
                ..Easing::default()
            };
            assert!(e.apply(0.0).abs() < 1e-3, "{style:?} {direction:?} at 0: {}", e.apply(0.0));
            assert!((e.apply(1.0) - 1.0).abs() < 1e-3, "{style:?} {direction:?} at 1: {}", e.apply(1.0));
        }
    }
}

#[test]
fn easing_shapes_are_correct() {
    let e = |style, direction| Easing {
        style,
        direction,
        ..Easing::default()
    };
    // In starts slow, Out starts fast, In Out is symmetric.
    assert!(e(EaseStyle::Cubic, EaseDirection::In).apply(0.25) < 0.25);
    assert!(e(EaseStyle::Cubic, EaseDirection::Out).apply(0.25) > 0.25);
    assert!((e(EaseStyle::Cubic, EaseDirection::InOut).apply(0.5) - 0.5).abs() < 1e-4);
    // Constant holds until the next key.
    assert_eq!(e(EaseStyle::Constant, EaseDirection::In).apply(0.99), 0.0);
    // Back overshoots past its target before settling.
    let back = e(EaseStyle::Back, EaseDirection::Out);
    assert!((0..100).any(|i| back.apply(i as f32 / 100.0) > 1.0));
}

#[test]
fn default_bezier_is_smooth_and_handles_change_it() {
    let default = Easing::default();
    assert_eq!(default.style, EaseStyle::Bezier);
    assert!((default.apply(0.5) - 0.5).abs() < 1e-3);
    assert!(default.apply(0.2) < 0.2, "starts slowly");
    assert!(default.apply(0.8) > 0.8, "stops slowly");

    let mut last = 0.0;
    for i in 1..=100 {
        let v = default.apply(i as f32 / 100.0);
        assert!(v >= last, "should never move backward");
        last = v;
    }

    // Custom handles on the diagonal make a straight line.
    let straight = Easing {
        style: EaseStyle::Custom,
        handles: [0.25, 0.25, 0.75, 0.75],
        ..default
    };
    assert!((straight.apply(0.3) - 0.3).abs() < 1e-3);
}

#[test]
fn track_stays_sorted_and_holds_past_its_ends() {
    let mut track = Track::default();
    track.set_key(20, turned(0.4));
    track.set_key(0, turned(0.0));
    track.set_key(10, turned(0.2));
    let frames: Vec<i32> = track.keys.iter().map(|k| k.frame).collect();
    assert_eq!(frames, [0, 10, 20]);

    track.set_key(10, turned(0.3));
    assert_eq!(track.keys.len(), 3, "same frame replaces");
    assert!((angle_of(track.sample(-5.0).unwrap())).abs() < 1e-5);
    assert!((angle_of(track.sample(99.0).unwrap()) - 0.4).abs() < 1e-4);
    assert!(Track::default().sample(1.0).is_none());
}

#[test]
fn sampling_interpolates_between_keys() {
    let mut track = Track::default();
    track.set_key(0, Xf::new(Quat::IDENTITY, Vec3::ZERO));
    track.set_key(10, Xf::new(Quat::from_rotation_z(1.0), Vec3::new(2.0, 0.0, 0.0)));
    track.keys[0].easing = linear();
    let mid = track.sample(5.0).unwrap();
    assert!((angle_of(mid) - 0.5).abs() < 1e-4);
    assert!(mid.pos.distance(Vec3::new(1.0, 0.0, 0.0)) < 1e-4);
    // A key's easing only affects the motion leaving it.
    track.keys[0].easing.style = EaseStyle::Constant;
    assert!(angle_of(track.sample(9.9).unwrap()).abs() < 1e-4);
    assert!((angle_of(track.sample(10.0).unwrap()) - 1.0).abs() < 1e-4);
}

#[test]
fn rotation_takes_the_short_way_around() {
    let mut track = Track::default();
    let a = Quat::from_rotation_y(0.1);
    // The same rotation as a small turn, written with every sign flipped.
    let b = -Quat::from_rotation_y(0.3);
    track.set_key(0, Xf::new(a, Vec3::ZERO));
    track.set_key(10, Xf::new(b, Vec3::ZERO));
    track.keys[0].easing = linear();
    let mid = track.sample(5.0).unwrap();
    assert!(mid.rot.angle_between(Quat::from_rotation_y(0.2)) < 1e-3);
}

#[test]
fn applying_an_animation_moves_the_rig_at_each_frame() {
    let mut scene = rig();
    let neck = bone(&scene, "Neck");
    let shoulder = bone(&scene, "Right Shoulder");
    let shoulder_pose = turned(0.25);
    scene.set_bone_pose(shoulder, shoulder_pose);

    let mut anim = Animation::default();
    anim.set_key(neck, 0, turned(0.0));
    anim.set_key(neck, 30, turned(1.0));
    anim.tracks.get_mut(&neck).unwrap().keys[0].easing = linear();

    anim.apply(&mut scene, 15.0);
    assert!((angle_of(scene.bones[neck].pose) - 0.5).abs() < 1e-3);
    let head = scene.parts.iter().position(|p| p.name == "Head").unwrap();
    let at_mid = scene.parts[head].world;
    anim.apply(&mut scene, 30.0);
    assert!(!scene.parts[head].world.approx_eq(at_mid), "world positions must update");
    assert!(scene.bones[shoulder].pose.approx_eq(shoulder_pose), "unkeyed bones stay put");
}

#[test]
fn first_edit_partway_along_keeps_where_the_bone_was() {
    let mut anim = Animation::default();
    let before = turned(0.1);
    anim.record_edit(3, 30, before, turned(0.9), 1.0, true);
    let frames: Vec<i32> = anim.tracks[&3].keys.iter().map(|k| k.frame).collect();
    assert_eq!(frames, [0, 30]);
    assert!(anim.key_at(3, 0).unwrap().pose.approx_eq(before));

    // Later edits only touch their own frame.
    anim.record_edit(3, 60, turned(0.9), turned(0.5), 1.0, true);
    assert_eq!(anim.tracks[&3].keys.len(), 3);
    // A first edit on the start frame needs only one key.
    anim.record_edit(4, 0, before, turned(0.2), 1.0, true);
    // It can also be turned off.
    anim.record_edit(5, 30, before, turned(0.2), 1.0, false);
    assert_eq!(anim.tracks[&5].keys.len(), 1);
    assert_eq!(anim.tracks[&4].keys.len(), 1);
}

#[test]
fn moving_keys_shifts_them_and_replaces_what_they_land_on() {
    let mut anim = Animation::default();
    anim.set_key(1, 0, turned(0.0));
    anim.set_key(1, 10, turned(0.1));
    anim.set_key(1, 20, turned(0.2));
    anim.set_key(2, 10, turned(0.3));

    let keys: HashSet<_> = [(1, 10), (2, 10)].into_iter().collect();
    let moved = anim.moved(&keys, 10);
    assert!(moved.key_at(1, 10).is_none());
    assert!(moved.key_at(2, 20).is_some());
    let landed = moved.key_at(1, 20).unwrap();
    assert!((angle_of(landed.pose) - 0.1).abs() < 1e-4, "moved key wins");
    assert_eq!(moved.tracks[&1].keys.len(), 2);
    assert_eq!(anim.tracks[&1].keys.len(), 3, "the original is untouched");
}

#[test]
fn removing_the_last_key_removes_the_track() {
    let mut anim = Animation::default();
    anim.set_key(1, 5, turned(0.0));
    anim.remove_key(1, 5);
    assert!(anim.tracks.is_empty());
    anim.remove_key(9, 9);
}

#[test]
fn playback_loops_or_stops_at_the_end() {
    let mut anim = Animation {
        start: 10,
        end: 40,
        fps: 30.0,
        looping: true,
        ..Animation::default()
    };
    let (frame, playing) = anim.advance(10.0, 0.5);
    assert_eq!((frame, playing), (25.0, true));
    let (frame, playing) = anim.advance(39.0, 0.1);
    assert!(playing);
    assert!((frame - 12.0).abs() < 1e-3, "wrapped to {frame}");
    let (frame, _) = anim.advance(0.0, 0.0);
    assert_eq!(frame, 10.0, "starts inside the range");

    anim.looping = false;
    assert_eq!(anim.advance(39.0, 1.0), (40.0, false));
}

#[test]
fn undo_restores_keys_and_poses_together() {
    let mut scene = rig();
    let neck = bone(&scene, "Neck");
    let mut anim = Animation::default();
    let mut history = History::default();

    let before_anim = anim.clone();
    let after_pose = turned(0.6);
    scene.set_bone_pose(neck, after_pose);
    anim.record_edit(neck, 0, Xf::IDENTITY, after_pose, 1.0, true);
    history.push_edit(
        "Rotate",
        vec![PoseChange {
            bone: neck,
            before: Xf::IDENTITY,
            after: after_pose,
        }],
        before_anim,
        anim.clone(),
    );

    history.undo(&mut scene, &mut anim).unwrap();
    assert!(anim.tracks.is_empty());
    assert!(scene.bones[neck].pose.approx_eq(Xf::IDENTITY));

    history.redo(&mut scene, &mut anim).unwrap();
    assert!(anim.key_at(neck, 0).is_some());
    assert!(scene.bones[neck].pose.approx_eq(after_pose));
}

/// Height on a position curve, which reads more directly than a rotation angle.
fn smooth_track(keys: &[(i32, f32)], handle_type: HandleType) -> Track {
    let mut track = Track::default();
    for (frame, y) in keys {
        track.set_key_with(
            *frame,
            Xf::new(Quat::IDENTITY, Vec3::new(0.0, *y, 0.0)),
            Easing {
                handle_type,
                ..Easing::default()
            },
        );
    }
    track
}

fn height(track: &Track, frame: f32, smoothing: Smoothing) -> f32 {
    track.sample_with(frame, smoothing).unwrap().pos.y
}

fn speed(track: &Track, frame: f32, smoothing: Smoothing) -> f32 {
    (height(track, frame + 0.05, smoothing) - height(track, frame - 0.05, smoothing)) / 0.1
}

#[test]
fn bezier_keys_pass_through_exactly_and_ends_come_to_rest() {
    for smoothing in [Smoothing::None, Smoothing::Continuous] {
        let track = smooth_track(&[(0, 0.0), (10, 2.0), (20, 5.0), (30, 6.0)], HandleType::AutoClamped);
        for (frame, y) in [(0.0, 0.0), (10.0, 2.0), (20.0, 5.0), (30.0, 6.0)] {
            assert!((height(&track, frame, smoothing) - y).abs() < 1e-3, "{smoothing:?} at {frame}");
        }
        assert!(speed(&track, 0.05, smoothing).abs() < 0.1, "starts at rest");
        assert!(speed(&track, 29.95, smoothing).abs() < 0.1, "ends at rest");
    }
}

#[test]
fn motion_flows_through_a_middle_key_instead_of_stopping() {
    // This is the difference from per-span easing, which would stop dead at frame 10.
    for smoothing in [Smoothing::None, Smoothing::Continuous] {
        let track = smooth_track(&[(0, 0.0), (10, 5.0), (20, 10.0)], HandleType::AutoClamped);
        let at_key = speed(&track, 10.0, smoothing);
        assert!(at_key > 0.3, "{smoothing:?} slowed to {at_key} at the key");
        let (before, after) = (speed(&track, 9.9, smoothing), speed(&track, 10.1, smoothing));
        assert!((before - after).abs() < 0.1, "{smoothing:?} kinked: {before} then {after}");
    }
}

#[test]
fn clamped_curves_stay_flat_on_peaks_and_never_overshoot() {
    let track = smooth_track(&[(0, 0.0), (10, 5.0), (20, 1.0), (30, 1.2), (40, 8.0)], HandleType::AutoClamped);
    for smoothing in [Smoothing::None, Smoothing::Continuous] {
        assert!(speed(&track, 10.0, smoothing).abs() < 0.2, "peak should be flat");
        assert!(speed(&track, 20.0, smoothing).abs() < 0.2, "valley should be flat");
        for i in 0..=400 {
            let y = height(&track, i as f32 * 0.1, smoothing);
            assert!((-0.001..=8.001).contains(&y), "{smoothing:?} overshot to {y}");
            if (0..=100).contains(&i) {
                assert!(y <= 5.001, "{smoothing:?} went past the peak: {y}");
            }
            if (100..=200).contains(&i) {
                assert!(y >= 0.999, "{smoothing:?} dipped below the valley: {y}");
            }
        }
    }
}

#[test]
fn unclamped_auto_is_allowed_to_overshoot_and_vector_is_straight() {
    let keys = [(0, 0.0), (10, 5.0), (20, 1.0), (30, 0.0)];
    let auto = smooth_track(&keys, HandleType::Auto);
    assert!(speed(&auto, 10.0, Smoothing::None).abs() > 0.03, "auto doesn't flatten peaks");

    let vector = smooth_track(&[(0, 0.0), (10, 5.0), (20, 5.0)], HandleType::Vector);
    assert!((height(&vector, 5.0, Smoothing::None) - 2.5).abs() < 1e-3, "straight between keys");
    assert!(speed(&vector, 9.0, Smoothing::None) > 0.4);
    assert!(speed(&vector, 11.0, Smoothing::None).abs() < 1e-3, "sharp corner at the key");
}

#[test]
fn continuous_smoothing_keeps_acceleration_continuous() {
    // Uneven spacing is what makes plain auto handles jump in acceleration.
    let track = smooth_track(&[(0, 0.0), (4, 1.0), (20, 8.0), (26, 10.0), (40, 18.0)], HandleType::Auto);
    let accel = |frame: f32, smoothing| (speed(&track, frame + 0.2, smoothing) - speed(&track, frame - 0.2, smoothing)) / 0.4;
    let jump = |frame: f32, smoothing| (accel(frame + 0.25, smoothing) - accel(frame - 0.25, smoothing)).abs();
    assert!(
        jump(20.0, Smoothing::Continuous) < jump(20.0, Smoothing::None) * 0.5,
        "{} vs {}",
        jump(20.0, Smoothing::Continuous),
        jump(20.0, Smoothing::None)
    );
}

#[test]
fn smoothed_rotation_takes_the_short_way_and_stays_valid() {
    let mut track = Track::default();
    // Written with flipped signs, as rotations can legitimately be.
    track.set_key(0, Xf::new(Quat::from_rotation_y(0.0), Vec3::ZERO));
    track.set_key(10, Xf::new(-Quat::from_rotation_y(0.4), Vec3::ZERO));
    track.set_key(20, Xf::new(Quat::from_rotation_y(0.8), Vec3::ZERO));
    for smoothing in [Smoothing::None, Smoothing::Continuous] {
        let mut last = 0.0;
        for i in 0..=200 {
            let pose = track.sample_with(i as f32 * 0.1, smoothing).unwrap();
            assert!((pose.rot.length() - 1.0).abs() < 1e-3);
            let angle = pose.rot.angle_between(Quat::IDENTITY);
            assert!(angle <= 0.801, "swung the long way: {angle}");
            assert!(angle + 1e-3 >= last, "went backward");
            last = angle;
        }
    }
}

#[test]
fn other_styles_ignore_curves_and_new_keys_use_the_defaults() {
    let mut anim = Animation {
        default_easing: Easing {
            style: EaseStyle::Linear,
            ..Easing::default()
        },
        ..Animation::default()
    };
    anim.set_key(1, 0, Xf::new(Quat::IDENTITY, Vec3::ZERO));
    anim.set_key(1, 10, Xf::new(Quat::IDENTITY, Vec3::Y * 4.0));
    assert_eq!(anim.key_at(1, 0).unwrap().easing.style, EaseStyle::Linear);
    assert!((anim.sample(1, 5.0).unwrap().pos.y - 2.0).abs() < 1e-3);
    assert!(anim.sample(2, 5.0).is_none());
}

