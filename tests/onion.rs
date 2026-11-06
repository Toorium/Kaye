//! Onion skins: which frames get a copy, where the copies stand, undo, and saving.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use glam::{Quat, Vec3};
use kaye::animation::Animation;
use kaye::import::import_file;
use kaye::math::Xf;
use kaye::onion::{self, OnionMode, OnionSkin, Side};
use kaye::project::{self, View};
use kaye::scene::{BoneId, PartShape, RigId, Scene};

fn sample(name: &str) -> Scene {
    import_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(name)).unwrap().scene
}

fn bone_named(scene: &Scene, name: &str) -> BoneId {
    scene.bones.iter().position(|b| b.name == name).unwrap_or_else(|| panic!("no bone {name}"))
}

fn keys(frames: &[i32]) -> BTreeSet<i32> {
    frames.iter().copied().collect()
}

fn skin(mode: OnionMode, before: u8, after: u8) -> OnionSkin {
    OnionSkin { mode, before, after, ..OnionSkin::default() }
}

/// R15 with its right arm keyed up at 0, down at 20, and forward at 40.
fn waving() -> (Scene, Animation, BoneId) {
    let mut scene = sample("R15Rig.rbxm");
    let shoulder = bone_named(&scene, "RightShoulder");
    let mut animation = Animation { end: 40, ..Animation::default() };
    for (frame, angle) in [(0, 0.0), (20, 90.0_f32), (40, -60.0)] {
        animation.set_key(shoulder, frame, Xf::new(Quat::from_rotation_z(angle.to_radians()), Vec3::ZERO));
    }
    animation.apply(&mut scene, 10.0);
    (scene, animation, shoulder)
}

fn rig_of(scene: &Scene, bone: BoneId) -> RigId {
    scene.bones[bone].rig
}

#[test]
fn keyframes_mode_shows_the_nearest_keys_either_side() {
    let at = onion::ghost_frames(&skin(OnionMode::Keys, 2, 1), &keys(&[0, 10, 20, 30]), 15, (0, 30));
    let frames: Vec<(i32, Side)> = at.iter().map(|(f, s, _)| (*f, *s)).collect();
    assert_eq!(frames, [(10, Side::Before), (0, Side::Before), (20, Side::After)]);
    // The nearest one on each side is the strongest, and the next fades.
    assert_eq!(at[0].2, 1.0);
    assert!(at[1].2 < at[0].2 && at[1].2 > 0.0);
    assert_eq!(at[2].2, 1.0);
}

#[test]
fn a_key_on_the_frame_showing_gets_no_copy() {
    let at = onion::ghost_frames(&skin(OnionMode::Keys, 1, 1), &keys(&[0, 10, 20]), 10, (0, 20));
    let frames: Vec<i32> = at.iter().map(|(f, _, _)| *f).collect();
    assert_eq!(frames, [0, 20]);
}

#[test]
fn frames_mode_steps_out_and_stays_in_the_range() {
    let onion = OnionSkin { step: 5, ..skin(OnionMode::Frames, 2, 2) };
    let at = onion::ghost_frames(&onion, &keys(&[]), 15, (0, 20));
    let frames: Vec<(i32, Side)> = at.iter().map(|(f, s, _)| (*f, *s)).collect();
    assert_eq!(frames, [(10, Side::Before), (5, Side::Before), (20, Side::After)], "25 is past the end");
}

#[test]
fn frames_outside_the_range_and_odd_settings_dont_break() {
    for mode in OnionMode::ALL {
        for frame in [-50, 0, 7, 120, 500] {
            let wild = OnionSkin { before: 200, after: 200, step: 0, opacity: f32::NAN, ..skin(mode, 0, 0) };
            let at = onion::ghost_frames(&wild, &keys(&[0, 5, 100]), frame, (0, 120));
            assert!(at.len() <= 2 * onion::MAX_GHOSTS as usize);
            assert!(at.iter().all(|(f, _, s)| (0..=120).contains(f) && *f != frame && *s > 0.0 && *s <= 1.0));
        }
        // A range typed backward still works.
        let at = onion::ghost_frames(&skin(mode, 1, 1), &keys(&[0, 10]), 5, (10, 0));
        assert!(!at.is_empty());
    }
    assert!(OnionSkin { opacity: f32::NAN, ..OnionSkin::default() }.clamped().opacity.is_finite());
}

#[test]
fn copies_stand_where_the_rig_is_on_their_frames() {
    let (mut scene, animation, shoulder) = waving();
    let rig = rig_of(&scene, shoulder);
    scene.rigs[rig].onion = Some(skin(OnionMode::Keys, 1, 1));
    let ghosts = onion::ghosts(&scene, &animation, 10);
    assert_eq!(ghosts.len(), 2);
    for ghost in &ghosts {
        let mut played = scene.clone();
        animation.apply(&mut played, ghost.frame as f32);
        assert_eq!(ghost.rig, rig);
        assert_eq!(ghost.frame, if ghost.side == Side::Before { 0 } else { 20 });
        for (index, part) in scene.rigs[rig].parts.iter().enumerate() {
            let (copy, real) = (ghost.parts[index], played.parts[*part].world);
            assert!(copy.pos.distance(real.pos) < 1e-4 && copy.rot.angle_between(real.rot) < 2e-3, "part {index} at frame {}", ghost.frame);
        }
        let at = scene.rigs[rig].bones.iter().position(|b| *b == shoulder).unwrap();
        assert!(ghost.bones[at].rot.angle_between(played.bones[shoulder].frame.rot) < 2e-3);
    }
    // The arm really is somewhere else in each copy, and the scene itself didn't move.
    let hand = scene.parts.iter().position(|p| p.name == "RightHand").unwrap();
    let index = scene.rigs[rig].parts.iter().position(|p| *p == hand).unwrap();
    let shown = scene.parts[hand].world.pos;
    assert!(ghosts.iter().all(|g| g.parts[index].pos.distance(shown) > 0.3));
    let mut again = scene.clone();
    animation.apply(&mut again, 10.0);
    assert!(again.parts[hand].world.pos.distance(shown) < 1e-5);
}

#[test]
fn a_hidden_skin_a_camera_and_a_rig_without_one_get_no_copies() {
    let (mut scene, animation, shoulder) = waving();
    let rig = rig_of(&scene, shoulder);
    assert!(onion::ghosts(&scene, &animation, 10).is_empty(), "nothing has one yet");
    scene.rigs[rig].onion = Some(OnionSkin { shown: false, ..OnionSkin::default() });
    assert!(onion::ghosts(&scene, &animation, 10).is_empty());

    let camera = scene.add_camera(Xf::IDENTITY, 70.0);
    let camera_rig = scene.bones[camera].rig;
    assert!(!onion::can_have(&scene, camera_rig));
    scene.rigs[camera_rig].onion = Some(OnionSkin::default());
    assert!(onion::ghosts(&scene, &animation, 10).is_empty(), "a camera has no parts to copy");
    assert!(onion::can_have(&scene, rig));
}

#[test]
fn a_spawned_shape_can_have_one() {
    let mut scene = Scene::default();
    let ball = scene.add_shape(PartShape::Ball, Xf::IDENTITY, Vec3::splat(2.0));
    let rig = scene.bones[ball].rig;
    let mut animation = Animation { end: 30, ..Animation::default() };
    animation.set_key(ball, 0, Xf::new(Quat::IDENTITY, Vec3::new(0.0, 1.0, 0.0)));
    animation.set_key(ball, 30, Xf::new(Quat::IDENTITY, Vec3::new(10.0, 1.0, 0.0)));
    scene.rigs[rig].onion = Some(OnionSkin { step: 10, ..skin(OnionMode::Frames, 1, 1) });
    animation.apply(&mut scene, 15.0);
    let ghosts = onion::ghosts(&scene, &animation, 15);
    let spots: Vec<(i32, f32)> = ghosts.iter().map(|g| (g.frame, g.parts[0].pos.x)).collect();
    assert_eq!(spots.len(), 2);
    assert_eq!(spots[0].0, 5);
    assert_eq!(spots[1].0, 25);
    assert!(spots[0].1 < scene.parts[0].world.pos.x && spots[1].1 > scene.parts[0].world.pos.x);
}

#[test]
fn undo_brings_an_onion_skin_back_and_takes_it_away() {
    let (mut scene, _, shoulder) = waving();
    let rig = rig_of(&scene, shoulder);
    let without = scene.rig_state();
    scene.rigs[rig].onion = Some(OnionSkin { before: 3, ..OnionSkin::default() });
    let with = scene.rig_state();
    assert_ne!(with, without, "adding one is a change undo sees");
    scene.restore_rig_state(&without);
    assert_eq!(scene.rigs[rig].onion, None);
    scene.restore_rig_state(&with);
    assert_eq!(scene.rigs[rig].onion.map(|o| o.before), Some(3));
}

#[test]
fn an_onion_skin_is_saved_and_older_projects_open_without_one() {
    let (mut scene, animation, shoulder) = waving();
    let rig = rig_of(&scene, shoulder);
    let folder = std::env::temp_dir().join(format!("kaye-test-{}-onion", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path: PathBuf = folder.join("onion.kaye");
    let view = View {
        frame: 10.0,
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
        references: Vec::new(),
    };

    project::save(&path, &scene, &animation, &view).unwrap();
    let plain = std::fs::read_to_string(&path).unwrap();
    assert!(!plain.contains("onion"), "a rig with none writes nothing for it");
    assert!(project::load(&path).unwrap().scene.rigs.iter().all(|r| r.onion.is_none()));

    let before = project::fingerprint(&scene, &animation);
    let wanted = OnionSkin { mode: OnionMode::Frames, before: 2, after: 3, step: 4, opacity: 0.6, before_color: [1, 2, 3], after_color: [4, 5, 6], shown: false };
    scene.rigs[rig].onion = Some(wanted);
    assert_ne!(project::fingerprint(&scene, &animation), before, "adding one is unsaved work");
    project::save(&path, &scene, &animation, &view).unwrap();
    let loaded = project::load(&path).unwrap();
    assert_eq!(loaded.scene.rigs[rig].onion, Some(wanted));

    // A file that only names some of the settings fills in the rest.
    let text = std::fs::read_to_string(&path).unwrap();
    let start = text.find("\"onion\":{").unwrap();
    let end = start + text[start..].find('}').unwrap() + 1;
    let partial = format!("{}\"onion\":{{\"before\":5}}{}", &text[..start], &text[end..]);
    std::fs::write(&path, partial).unwrap();
    let loaded = project::load(&path).unwrap();
    assert_eq!(loaded.scene.rigs[rig].onion, Some(OnionSkin { before: 5, ..OnionSkin::default() }));
    let _ = std::fs::remove_dir_all(&folder);
}
