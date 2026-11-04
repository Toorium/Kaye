//! Writes a project with one block of every material on a Baseplate, to look at materials
//! in the real app. `cargo run --example material_board -- board.kaye`

use glam::{Quat, Vec3};
use kaye::animation::Animation;
use kaye::materials::{self, Material};
use kaye::math::Xf;
use kaye::project::{self, View};
use kaye::scene::{Decal, Face, PartShape, Scene, Tiling};

fn main() {
    let out = std::env::args().nth(1).expect("an output path");
    let mut scene = Scene::default();
    let colors: &[(Material, [u8; 3])] = &[
        (Material::Grass, [75, 151, 75]),
        (Material::LeafyGrass, [75, 151, 75]),
        (Material::Brick, [196, 40, 28]),
        (Material::WoodPlanks, [160, 95, 53]),
        (Material::Wood, [160, 95, 53]),
        (Material::Sand, [215, 197, 154]),
        (Material::Sandstone, [215, 197, 154]),
        (Material::Snow, [248, 248, 248]),
        (Material::Ice, [175, 221, 255]),
        (Material::Glacier, [175, 221, 255]),
        (Material::Neon, [255, 0, 191]),
        (Material::CrackedLava, [99, 95, 98]),
        (Material::CorrodedMetal, [124, 92, 70]),
        (Material::Mud, [124, 92, 70]),
        (Material::Ground, [124, 92, 70]),
        (Material::Cardboard, [204, 142, 105]),
        (Material::ClayRoofTiles, [204, 142, 105]),
        (Material::Carpet, [116, 134, 157]),
        (Material::Fabric, [116, 134, 157]),
        (Material::Leather, [105, 64, 40]),
    ];
    let columns = 9;
    for (i, material) in Material::ALL.iter().enumerate() {
        let (col, row) = ((i % columns) as f32, (i / columns) as f32);
        let at = Xf::new(Quat::IDENTITY, Vec3::new(col * 4.5 - 18.0, 1.5, row * 4.5 - 10.0));
        let bone = scene.add_shape(PartShape::Block, at, Vec3::splat(3.0));
        let part = scene.bones[bone].part.unwrap();
        let rgb = colors.iter().find(|(m, _)| m == material).map_or([163, 162, 165], |(_, c)| *c);
        scene.parts[part].color = rgb.map(|c| c as f32 / 255.0);
        scene.parts[part].material = *material;
        scene.parts[part].name = material.label().to_string();
        let rig = scene.bones[bone].rig;
        scene.rigs[rig].name = material.label().to_string();
    }
    let base = scene.add_shape(PartShape::Block, Xf::new(Quat::IDENTITY, Vec3::new(0.0, -8.0, 0.0)), Vec3::new(2048.0, 16.0, 2048.0));
    let part = scene.bones[base].part.unwrap();
    scene.parts[part].color = [91.0 / 255.0; 3];
    scene.parts[part].locked = true;
    scene.parts[part].decals = vec![Decal {
        texture: materials::BASEPLATE_TEXTURE.to_string(),
        face: Face::Top,
        color: [0.0; 3],
        transparency: 0.8,
        tiling: Some(Tiling { studs_u: 8.0, studs_v: 8.0, offset_u: 0.0, offset_v: 0.0 }),
    }];
    let mut animation = Animation::default();
    animation.ensure_clips(&scene);
    let view = View { frame: 0.0, target: Vec3::new(2.0, 0.0, -2.0), yaw: 0.35, pitch: -0.5, dist: 30.0, ortho: false, floor_y: 0.0, camera: None, preview: false, through: false, motion_paths: Vec::new() };
    project::save(std::path::Path::new(&out), &scene, &animation, &view).unwrap();
    println!("wrote {out}");
}
