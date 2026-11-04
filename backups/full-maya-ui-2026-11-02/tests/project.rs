//! Saving and opening projects, on the real sample rigs.

use std::path::{Path, PathBuf};

use glam::{Quat, Vec3};
use kaye::animation::{Animation, Handles, Smoothing};
use kaye::import::import_file;
use kaye::math::Xf;
use kaye::modifiers;
use kaye::project::{self, View};
use kaye::scene::Scene;

fn rig(file: &str) -> Scene {
    import_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(file)).unwrap().scene
}

fn bone(s: &Scene, name: &str) -> usize {
    s.bones.iter().position(|b| b.name == name).unwrap_or_else(|| panic!("no bone {name}"))
}

/// A file in its own folder, which goes away with the test.
struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let folder = std::env::temp_dir().join(format!("kaye-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        Self(folder)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn view() -> View {
    View {
        frame: 12.5,
        target: Vec3::new(1.0, 2.0, 3.0),
        yaw: 0.5,
        pitch: -0.25,
        dist: 9.0,
        ortho: true,
        floor_y: -3.0,
        camera: None,
        preview: false,
        through: false,
    }
}

/// A rig with everything a project has to keep: IK, a pole, Unparent, a camera, keys
/// with hand-set handles, and a hidden part.
fn busy_scene() -> (Scene, Animation) {
    let mut scene = rig("R15Rig.rbxm");
    let foot = bone(&scene, "LeftAnkle");
    let target = modifiers::add_ik(&mut scene, foot);
    modifiers::add_pole(&mut scene, 0);
    let hand = bone(&scene, "RightWrist");
    modifiers::add_unparent(&mut scene, hand);
    let camera = scene.add_camera(Xf::new(Quat::from_rotation_y(1.0), Vec3::new(0.0, 5.0, 12.0)), 55.0);
    scene.parts[1].visible = false;
    scene.evaluate();

    let mut animation = Animation { end: 200, fps: 24.0, looping: false, smoothing: Smoothing::Continuous, ..Animation::default() };
    let shoulder = bone(&scene, "RightShoulder");
    for (frame, angle) in [(0, 0.0f32), (10, 0.7), (25, -0.4), (40, 0.2)] {
        animation.set_key(shoulder, frame, Xf::new(Quat::from_rotation_x(angle), Vec3::ZERO));
        animation.set_key_full(camera, frame, Xf::new(Quat::IDENTITY, Vec3::splat(angle)), 40.0 + angle);
    }
    animation.set_key(target, 0, Xf::IDENTITY);
    animation.set_key(target, 30, Xf::new(Quat::IDENTITY, Vec3::new(0.3, 0.8, -0.2)));
    animation.tracks.get_mut(&shoulder).unwrap().keys[1].handles[3] = Some(Handles { left: [-2.0, 1.5], right: [3.0, -1.0], aligned: false });
    animation.apply(&mut scene, 17.5);
    (scene, animation)
}

#[test]
fn a_project_comes_back_exactly() {
    let temp = Temp::new("exact");
    let path = temp.file("busy.kaye");
    let (scene, animation) = busy_scene();
    project::save(&path, &scene, &animation, &view()).unwrap();
    let loaded = project::load(&path).unwrap();

    assert_eq!(loaded.animation, animation);
    assert_eq!(loaded.view, view());
    assert_eq!(loaded.scene.modifiers, scene.modifiers);
    assert_eq!(loaded.scene.rig_state(), scene.rig_state());
    assert_eq!(loaded.scene.parts.len(), scene.parts.len());
    assert!(!loaded.scene.parts[1].visible);

    // Everything left out of the file gets worked out to the same place.
    for (a, b) in loaded.scene.bones.iter().zip(&scene.bones) {
        assert_eq!(a.pose, b.pose, "{}", a.name);
        assert_eq!(a.name, b.name);
        assert_eq!(a.tail_local, b.tail_local);
        assert!(a.frame.pos.distance(b.frame.pos) < 1e-5, "{}", a.name);
        assert!(a.frame.rot.dot(b.frame.rot).abs() > 0.99999, "{}", a.name);
    }
    for (a, b) in loaded.scene.parts.iter().zip(&scene.parts) {
        assert!(a.world.pos.distance(b.world.pos) < 1e-5, "{}", a.name);
    }
}

#[test]
fn every_sample_rig_survives_a_save() {
    let temp = Temp::new("samples");
    for file in ["R6Rig.rbxm", "R15Rig.rbxm", "R6WithAccessories.rbxm"] {
        let path = temp.file("rig.kaye");
        let scene = rig(file);
        project::save(&path, &scene, &Animation::default(), &view()).unwrap();
        let loaded = project::load(&path).unwrap();
        assert_eq!(loaded.scene.parts.len(), scene.parts.len(), "{file}");
        assert_eq!(project::fingerprint(&loaded.scene, &loaded.animation), project::fingerprint(&scene, &Animation::default()), "{file}");
    }
}

#[test]
fn a_loaded_project_still_animates_and_edits() {
    let temp = Temp::new("live");
    let path = temp.file("live.kaye");
    let (scene, animation) = busy_scene();
    project::save(&path, &scene, &animation, &view()).unwrap();
    let mut loaded = project::load(&path).unwrap();

    let mut original = scene.clone();
    for time in [0.0, 3.3, 10.0, 17.5, 32.25, 39.9] {
        animation.apply(&mut original, time);
        loaded.animation.apply(&mut loaded.scene, time);
        for (a, b) in loaded.scene.parts.iter().zip(&original.parts) {
            assert!(a.world.pos.distance(b.world.pos) < 1e-4, "{} at {time}", a.name);
        }
    }
    let shoulder = bone(&loaded.scene, "RightShoulder");
    loaded.animation.set_key(shoulder, 5, Xf::IDENTITY);
    assert_eq!(loaded.animation.tracks[&shoulder].keys.len(), 5);
}

#[test]
fn only_real_changes_count_as_unsaved() {
    let (mut scene, animation) = busy_scene();
    let saved = project::fingerprint(&scene, &animation);

    // Looking at another frame only moves poses on animated bones.
    animation.apply(&mut scene, 33.0);
    assert_eq!(project::fingerprint(&scene, &animation), saved, "scrubbing isn't work");

    // A preference for what new keys start with isn't either.
    let mut other = animation.clone();
    other.default_easing.style = kaye::animation::EaseStyle::Linear;
    assert_eq!(project::fingerprint(&scene, &other), saved);

    let mut keyed = animation.clone();
    keyed.set_key(bone(&scene, "RightShoulder"), 7, Xf::IDENTITY);
    assert_ne!(project::fingerprint(&scene, &keyed), saved, "a new key");

    let mut timing = animation.clone();
    timing.fps = 60.0;
    assert_ne!(project::fingerprint(&scene, &timing), saved, "the frame rate");

    let mut posed = scene.clone();
    let unkeyed = bone(&scene, "Neck");
    posed.bones[unkeyed].pose = Xf::new(Quat::from_rotation_x(0.3), Vec3::ZERO);
    assert_ne!(project::fingerprint(&posed, &animation), saved, "a pose with no keys");

    let mut hidden = scene.clone();
    hidden.parts[2].visible = false;
    assert_ne!(project::fingerprint(&hidden, &animation), saved, "hiding a part");

    let mut removed = scene.clone();
    modifiers::remove(&mut removed, 0);
    assert_ne!(project::fingerprint(&removed, &animation), saved, "removing a modifier");
}

#[test]
fn a_failed_save_leaves_the_old_file_alone() {
    let temp = Temp::new("failed");
    let path = temp.file("work.kaye");
    let (scene, animation) = busy_scene();
    project::save(&path, &scene, &animation, &view()).unwrap();
    let good = std::fs::read(&path).unwrap();

    // A number JSON can't hold makes a file that couldn't be opened again.
    let mut broken = scene.clone();
    broken.parts[0].transparency = f32::NAN;
    assert!(project::save(&path, &broken, &animation, &view()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), good);
    assert_eq!(std::fs::read_dir(&temp.0).unwrap().count(), 1, "no temporary file left behind");

    // Nor can a folder that isn't there be written to.
    let nowhere = temp.file("missing").join("work.kaye");
    assert!(project::save(&nowhere, &scene, &animation, &view()).is_err());
}

#[test]
fn bad_files_give_a_message_and_not_a_crash() {
    let temp = Temp::new("bad");
    let open = |name: &str, text: &str| {
        let path = temp.file(name);
        std::fs::write(&path, text).unwrap();
        project::load(&path).unwrap_err()
    };
    assert!(open("a.kaye", "not json at all").contains("isn't a Kaye project"));
    assert!(open("b.kaye", "{}").contains("isn't a Kaye project"));
    assert!(open("c.kaye", r#"{"kaye_project": 999}"#).contains("newer version"));
    assert!(open("d.kaye", r#"{"kaye_project": 1}"#).contains("damaged"));
    assert!(project::load(&temp.file("never-written.kaye")).is_err());

    // Saved fine, then damaged by hand: ids pointing nowhere and a loop of parents.
    let path = temp.file("good.kaye");
    let (scene, animation) = busy_scene();
    let tamper = |change: fn(&mut serde_json::Value)| {
        project::save(&path, &scene, &animation, &view()).unwrap();
        let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        change(&mut json);
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        project::load(&path).unwrap_err()
    };
    assert!(tamper(|j| j["scene"]["bones"][1]["parent"] = 99999.into()).contains("damaged"));
    assert!(tamper(|j| j["scene"]["modifiers"][0]["bone"] = 99999.into()).contains("no bone"));
    assert!(tamper(|j| j["scene"]["part_order"][0] = 99999.into()).contains("no part"));
    assert!(tamper(|j| j["scene"]["bones"][0]["parent"] = 0.into()).contains("damaged"));
    assert!(tamper(|j| j["animation"]["fps"] = 0.into()).contains("frame rate"));
    assert!(tamper(|j| j["view"]["camera"] = 99999.into()).contains("no bone"));
}

/// A project saved before Lean existed opens with the default, and a set value is kept.
#[test]
fn lean_is_saved_and_older_projects_still_open() {
    let temp = Temp::new("lean");
    let path = temp.file("lean.kaye");
    let (mut scene, animation) = busy_scene();
    let hand = bone(&scene, "RightShoulder");
    modifiers::add_ik(&mut scene, hand);
    let index = scene.modifiers.len() - 1;
    if let modifiers::ModifierKind::Ik(ik) = &mut scene.modifiers[index].kind {
        ik.lean = 1.25;
    }
    project::save(&path, &scene, &animation, &view()).unwrap();
    assert_eq!(project::load(&path).unwrap().scene.modifiers, scene.modifiers);

    let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    json["scene"]["modifiers"][index]["kind"]["Ik"].as_object_mut().unwrap().remove("lean").unwrap();
    std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    let older = project::load(&path).unwrap();
    let modifiers::ModifierKind::Ik(ik) = older.scene.modifiers[index].kind else { panic!() };
    assert_eq!(ik.lean, 0.5);
}

#[test]
fn names_get_the_right_extension() {
    assert_eq!(project::with_extension("walk".into()), PathBuf::from("walk.kaye"));
    assert_eq!(project::with_extension("walk.kaye".into()), PathBuf::from("walk.kaye"));
    assert_eq!(project::with_extension("walk.v2".into()), PathBuf::from("walk.v2.kaye"));
    assert!(project::is_project(Path::new("WALK.KAYE")));
    assert!(!project::is_project(Path::new("walk.rbxm")));
}
