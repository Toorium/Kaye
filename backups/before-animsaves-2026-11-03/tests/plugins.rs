//! Luau plugins, run on a real rig.

use std::path::Path;

use glam::Vec3;
use kaye::animation::Animation;
use kaye::import::import_file;
use kaye::math::Xf;
use kaye::plugins::{Plugins, Workspace};

fn workspace() -> Workspace {
    let scene = import_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/R15Rig.rbxm")).unwrap().scene;
    Workspace { scene, animation: Animation::default(), rig: Some(0), ..Workspace::default() }
}

fn plugin(source: &str) -> Plugins {
    let mut plugins = Plugins::new().unwrap();
    plugins.load_source("test.luau", source).unwrap();
    plugins
}

#[test]
fn a_plugin_adds_commands() {
    let plugins = plugin(r#"kaye.command("One", function() end) kaye.command("Two", function() end)"#);
    assert_eq!(plugins.command_names(), ["One", "Two"]);
}

#[test]
fn a_command_poses_and_keys_a_bone() {
    let mut plugins = plugin(
        r#"
        kaye.command("Lift", function()
            local knee = kaye.findBone("LeftKnee")
            kaye.setFrame(12)
            kaye.setPose(knee, { y = 2, rx = 90 })
            kaye.setKey(knee)
        end)
        "#,
    );
    let (done, result) = plugins.run(0, workspace());
    result.unwrap();
    let knee = done.scene.bones.iter().position(|b| b.name == "LeftKnee").unwrap();
    assert_eq!(done.frame, 12);
    let key = done.animation.key_at(knee, 12).expect("keyed on frame 12");
    assert_eq!(key.pose.pos, Vec3::new(0.0, 2.0, 0.0));
    let turned = Xf::new(glam::Quat::from_rotation_x(std::f32::consts::FRAC_PI_2), key.pose.pos);
    assert!(key.pose.approx_eq(turned));
}

#[test]
fn a_pose_left_partly_out_keeps_the_rest() {
    let mut plugins = plugin(
        r#"
        kaye.command("Nudge", function()
            for _, bone in kaye.selected() do
                local pose = kaye.getPose(bone)
                kaye.setPose(bone, { x = pose.x + 1 })
            end
        end)
        "#,
    );
    let mut ws = workspace();
    let id = 3;
    ws.scene.bones[id].pose = Xf::new(glam::Quat::from_rotation_y(0.5), Vec3::new(0.0, 1.0, 0.0));
    ws.selected = vec![id];
    let (done, result) = plugins.run(0, ws);
    result.unwrap();
    let pose = done.scene.bones[id].pose;
    assert_eq!(pose.pos, Vec3::new(1.0, 1.0, 0.0));
    assert!((pose.rot.to_euler(glam::EulerRot::YXZ).0 - 0.5).abs() < 1e-3);
}

#[test]
fn a_mistake_in_a_plugin_is_reported_and_not_a_crash() {
    let mut plugins = plugin(r#"kaye.command("Broken", function() kaye.getPose(99999) end)"#);
    let (_, result) = plugins.run(0, workspace());
    assert!(result.unwrap_err().contains("99999"));
    let mut plugins = plugin(r#"kaye.command("Broken", function() local x = nil; x.y = 1 end)"#);
    let (_, result) = plugins.run(0, workspace());
    assert!(result.is_err());
    let mut plugins = Plugins::new().unwrap();
    assert!(plugins.load_source("bad.luau", "this is not luau").unwrap_err().contains("bad.luau"));
}

#[test]
fn a_loop_that_never_ends_is_stopped() {
    let mut plugins = plugin(r#"kaye.command("Forever", function() while true do end end)"#);
    let (done, result) = plugins.run(0, workspace());
    assert!(result.unwrap_err().contains("too long"));
    assert!(!done.scene.bones.is_empty(), "the work comes back regardless");
}

#[test]
fn plugins_cannot_reach_files_or_change_the_language() {
    let mut plugins = plugin(
        r#"
        kaye.command("Snoop", function()
            assert(io == nil, "io")
            assert(os.execute == nil, "execute")
            assert(require == nil, "require")
            assert(loadfile == nil, "loadfile")
            assert(not pcall(function() string.upper = nil end), "the library can't be changed")
        end)
        "#,
    );
    let (_, result) = plugins.run(0, workspace());
    result.unwrap();
}

#[test]
fn a_folder_of_plugins_loads_and_a_bad_one_stops_nothing() {
    let folder = std::env::temp_dir().join(format!("kaye-plugins-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("a.luau"), r#"kaye.command("A", function() end)"#).unwrap();
    std::fs::write(folder.join("b.luau"), "oh no").unwrap();
    std::fs::write(folder.join("c.lua"), r#"kaye.command("C", function() end)"#).unwrap();
    std::fs::write(folder.join("notes.txt"), "not a plugin").unwrap();
    let mut plugins = Plugins::new().unwrap();
    let failures = plugins.load_folder(&folder);
    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("b.luau"));
    assert_eq!(plugins.command_names(), ["A", "C"]);
    std::fs::remove_dir_all(&folder).ok();
}

#[test]
fn the_example_plugin_loads_and_runs() {
    let source = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/plugins/example.luau")).unwrap();
    let mut plugins = Plugins::new().unwrap();
    plugins.load_source("example.luau", &source).unwrap();
    assert_eq!(plugins.command_names(), ["Key Every Bone", "Exaggerate Pose"]);
    let (done, result) = plugins.run(0, workspace());
    result.unwrap();
    assert_eq!(done.animation.tracks.len(), 15);
    assert_eq!(done.messages, ["Keyed 15 bones"]);
}

#[test]
fn a_mirror_call_goes_through_the_same_code_as_the_menu() {
    let mut plugins = plugin(r#"kaye.command("Flip", function() kaye.mirror({ kaye.findBone("LeftElbow") }) end)"#);
    let mut ws = workspace();
    let left = ws.scene.bones.iter().position(|b| b.name == "LeftElbow").unwrap();
    let right = ws.scene.bones.iter().position(|b| b.name == "RightElbow").unwrap();
    ws.scene.bones[left].pose = Xf::new(glam::Quat::from_rotation_x(0.7), Vec3::ZERO);
    ws.scene.evaluate();
    let wanted = kaye::posing::mirror_pose(&ws.scene, &[left])[0];
    let (done, result) = plugins.run(0, ws);
    result.unwrap();
    assert_eq!(wanted.0, right);
    assert!(done.scene.bones[right].pose.approx_eq(wanted.1));
}
