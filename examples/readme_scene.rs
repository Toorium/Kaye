//! Writes the project the README's screenshots are taken from: the R15 sample mid-run on a
//! Baseplate, with a looping run keyed on its arms and legs, and an onion skin if asked.
//! `cargo run --example readme_scene -- scene.kaye [--onion]`

use std::path::Path;

use glam::{Quat, Vec3};
use kaye::animation::Animation;
use kaye::import::import_file;
use kaye::materials;
use kaye::math::Xf;
use kaye::onion::{OnionMode, OnionSkin};
use kaye::project::{self, View};
use kaye::scene::{BoneId, Decal, Face, PartShape, Scene, Tiling};

fn bone_named(scene: &Scene, name: &str) -> BoneId {
    scene.bones.iter().position(|b| b.name == name).unwrap_or_else(|| panic!("no bone {name}"))
}

/// A turn in degrees about the joint's own X, Y and Z.
fn turn(x: f32, y: f32, z: f32) -> Xf {
    Xf::new(Quat::from_euler(glam::EulerRot::XYZ, x.to_radians(), y.to_radians(), z.to_radians()), Vec3::ZERO)
}

fn main() {
    let out = std::env::args().nth(1).expect("an output path");
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples");
    let mut scene = import_file(&samples.join("R15Rig.rbxm")).unwrap().scene;
    scene.evaluate();
    let (low, high) = scene.bounds(None).unwrap();

    // A run cycle: each limb swings one way at 0 and 30, and the other way at 15.
    let mut animation = Animation { end: 30, ..Animation::default() };
    let swing = |a: f32| [(0, a), (15, -a), (30, a)];
    let keys: &[(&str, [(i32, f32); 3], fn(f32) -> Xf)] = &[
        // Each arm swings against the leg on its own side. A shoulder turns forward on +X,
        // and a hip on -X.
        ("RightShoulder", swing(-50.0), |a| turn(a, 0.0, 8.0)),
        ("LeftShoulder", swing(50.0), |a| turn(a, 0.0, -8.0)),
        ("RightElbow", swing(35.0), |a| turn(35.0 + a.abs() * 0.4, 0.0, 0.0)),
        ("LeftElbow", swing(35.0), |a| turn(35.0 + a.abs() * 0.4, 0.0, 0.0)),
        ("RightHip", swing(-40.0), |a| turn(a, 0.0, 0.0)),
        ("LeftHip", swing(40.0), |a| turn(a, 0.0, 0.0)),
        // Knees bend back on +X, most on the leg behind.
        ("RightKnee", swing(-40.0), |a| turn(15.0 + a.max(0.0) * 1.4, 0.0, 0.0)),
        ("LeftKnee", swing(40.0), |a| turn(15.0 + a.max(0.0) * 1.4, 0.0, 0.0)),
        ("Waist", swing(8.0), |a| turn(-6.0, a, 0.0)),
        ("Neck", swing(-6.0), |a| turn(4.0, a, 0.0)),
    ];
    for (name, frames, pose) in keys {
        let bone = bone_named(&scene, name);
        for (frame, angle) in frames {
            animation.set_key(bone, *frame, pose(*angle));
        }
    }
    animation.ensure_clips(&scene);

    // The sample wears Roblox's template clothing, labeled FRONT and R, so it gets the
    // classic body colors instead.
    let rig = scene.bones[bone_named(&scene, "RightShoulder")].rig;
    scene.rigs[rig].shirt = None;
    scene.rigs[rig].pants = None;
    for part in scene.parts.iter_mut().filter(|p| p.rig == rig) {
        let rgb: [u8; 3] = if part.name.contains("Torso") {
            [13, 105, 172]
        } else if part.name.contains("Leg") || part.name.contains("Foot") {
            [164, 189, 71]
        } else {
            [245, 205, 48]
        };
        part.color = rgb.map(|c| c as f32 / 255.0);
    }
    // Copies of a run in place would all stand on top of each other, so with an onion skin
    // the rig also runs forward.
    if std::env::args().any(|a| a == "--onion") {
        scene.rigs[rig].onion = Some(OnionSkin { mode: OnionMode::Frames, before: 2, after: 2, step: 4, opacity: 0.35, ..OnionSkin::default() });
        let root = bone_named(&scene, "Root");
        for frame in [0, 15, 30] {
            animation.set_key(root, frame, Xf::new(Quat::IDENTITY, Vec3::new(0.0, 0.0, -0.45 * frame as f32)));
        }
    }

    let center = (low + high) * 0.5;
    let base = scene.add_shape(PartShape::Block, Xf::new(Quat::IDENTITY, Vec3::new(center.x, low.y - 8.0, center.z)), Vec3::new(2048.0, 16.0, 2048.0));
    let part = scene.bones[base].part.unwrap();
    scene.parts[part].color = [91.0 / 255.0; 3];
    scene.parts[part].locked = true;
    scene.parts[part].name = "Baseplate".to_string();
    scene.rigs[scene.bones[base].rig].name = "Baseplate".to_string();
    scene.parts[part].decals = vec![Decal {
        texture: materials::BASEPLATE_TEXTURE.to_string(),
        face: Face::Top,
        color: [0.0; 3],
        transparency: 0.8,
        tiling: Some(Tiling { studs_u: 8.0, studs_v: 8.0, offset_u: 0.0, offset_v: 0.0 }),
    }];
    animation.ensure_clips(&scene);

    // Three quarters from the front, or from the side at frame 10 for the onion skin, with
    // `--yaw` to try another angle.
    let onion = scene.rigs[rig].onion.is_some();
    let args: Vec<String> = std::env::args().collect();
    let yaw = args.iter().position(|a| a == "--yaw").and_then(|i| args.get(i + 1)?.parse().ok());
    let view = View {
        frame: if onion { 10.0 } else { 3.0 },
        target: if onion { center + Vec3::new(0.0, 0.0, -4.5) } else { center },
        yaw: yaw.unwrap_or(if onion { std::f32::consts::FRAC_PI_2 + 0.35 } else { std::f32::consts::PI - 0.6 }),
        pitch: -0.18,
        dist: (high - low).length() * if onion { 2.6 } else { 1.85 },
        ortho: false,
        floor_y: low.y,
        camera: None,
        preview: false,
        through: false,
        motion_paths: Vec::new(),
        references: Vec::new(),
    };
    project::save(Path::new(&out), &scene, &animation, &view).unwrap();
    println!("wrote {out}");
}
