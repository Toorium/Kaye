//! Materials, Decals and Textures: reading them from a model, keeping them in a project,
//! sending them back out, and where their pictures come from.

use glam::{Quat, Vec3};
use kaye::animation::Animation;
use kaye::assets::decode_material_map;
use kaye::export;
use kaye::import::{import_bytes, import_dom};
use kaye::materials::{self, Material};
use kaye::math::Xf;
use kaye::project::{self, View};
use kaye::scene::{Decal, Face, PartShape, Scene, Tiling};
use kaye::settings::Settings;
use rbx_dom_weak::{InstanceBuilder, WeakDom};
use rbx_types::{CFrame, Color3, ContentId, Enum, Matrix3, Variant, Vector3};
use ustr::ustr;

fn at(x: f32, y: f32, z: f32) -> CFrame {
    CFrame::new(Vector3::new(x, y, z), Matrix3::identity())
}

/// A two-part rig: a brick torso with a Decal on its front, and a grass arm with a
/// Texture on its top.
fn dressed_rig() -> WeakDom {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let model = dom.insert(root, InstanceBuilder::new("Model").with_name("Rig"));
    let torso = dom.insert(
        model,
        InstanceBuilder::new("Part")
            .with_name("Torso")
            .with_property("CFrame", at(0.0, 3.0, 0.0))
            .with_property("Size", Vector3::new(2.0, 2.0, 1.0))
            .with_property("Material", Variant::Enum(Enum::from_u32(848))),
    );
    dom.insert(
        torso,
        InstanceBuilder::new("Decal")
            .with_property("Texture", Variant::ContentId(ContentId::from("rbxassetid://123".to_string())))
            .with_property("Face", Variant::Enum(Enum::from_u32(5)))
            .with_property("Color3", Variant::Color3(Color3::new(1.0, 0.5, 0.25)))
            .with_property("Transparency", Variant::Float32(0.25)),
    );
    let arm = dom.insert(
        model,
        InstanceBuilder::new("Part")
            .with_name("Arm")
            .with_property("CFrame", at(1.5, 3.0, 0.0))
            .with_property("Size", Vector3::new(1.0, 2.0, 1.0))
            .with_property("Material", Variant::Enum(Enum::from_u32(1280))),
    );
    dom.insert(
        arm,
        InstanceBuilder::new("Texture")
            .with_property("Texture", Variant::ContentId(ContentId::from(materials::BASEPLATE_TEXTURE.to_string())))
            .with_property("Face", Variant::Enum(Enum::from_u32(1)))
            .with_property("StudsPerTileU", Variant::Float32(8.0))
            .with_property("StudsPerTileV", Variant::Float32(4.0))
            .with_property("OffsetStudsU", Variant::Float32(1.0)),
    );
    let joint = dom.insert(torso, InstanceBuilder::new("Motor6D").with_name("Shoulder").with_property("C0", at(1.5, 0.0, 0.0)).with_property("C1", at(0.0, 0.0, 0.0)));
    let joint = dom.get_by_ref_mut(joint).unwrap();
    joint.properties.insert(ustr("Part0"), Variant::Ref(torso));
    joint.properties.insert(ustr("Part1"), Variant::Ref(arm));
    dom
}

fn part_named<'a>(scene: &'a Scene, name: &str) -> &'a kaye::scene::Part {
    scene.parts.iter().find(|p| p.name == name).unwrap()
}

#[test]
fn every_material_keeps_its_roblox_value() {
    for material in Material::ALL {
        assert_eq!(Material::from_roblox(material.roblox_value()), *material, "{}", material.label());
    }
    // Terrain's own materials, and anything newer, show as plain plastic.
    assert_eq!(Material::from_roblox(2048), Material::Plastic);
    assert_eq!(Material::from_roblox(99999), Material::Plastic);
    assert_eq!(Material::ALL.len(), 43);
}

#[test]
fn every_material_roblox_publishes_has_its_pictures_and_a_slot() {
    let drawn_by_roblox = [Material::Plastic, Material::SmoothPlastic, Material::Neon, Material::ForceField];
    for material in Material::ALL {
        let maps = material.maps();
        assert_eq!(maps.is_none(), drawn_by_roblox.contains(material), "{}", material.label());
        assert_eq!(material.layer().is_some(), maps.is_some());
        if let Some((color, normal)) = maps {
            assert!(materials::is_public(color) && materials::is_public(normal));
            assert_ne!(color, normal);
        }
    }
    let slots: Vec<u32> = materials::textured().iter().map(|m| m.layer().unwrap()).collect();
    assert_eq!(slots, (0..slots.len() as u32).collect::<Vec<_>>(), "slots are packed in order");
    assert!(!materials::is_public(123), "only Roblox's own published pictures skip the key");
}

#[test]
fn pictures_roblox_keeps_to_itself_have_stand_ins() {
    assert_eq!(materials::stand_in(materials::BASEPLATE_TEXTURE), Some(materials::GRID_ID));
    assert_eq!(materials::stand_in("http://www.roblox.com/asset/?id=6372755229"), Some(materials::GRID_ID));
    assert_eq!(materials::stand_in("rbxasset://textures/SpawnLocation.png"), None, "no spawn point, by request");
    assert_eq!(materials::stand_in("rbxassetid://123"), None);
    for picture in [materials::baseplate_grid()] {
        assert_eq!(picture.levels[0].len() as u32, picture.width * picture.height * 4);
        assert_eq!(picture.levels.last().unwrap().len(), 4, "shrinks all the way to one pixel");
    }
}

#[test]
fn a_material_picture_fills_its_slot_exactly_and_keeps_untinted_color_far_away() {
    // Half the picture is clear, which in a material picture means "don't tint here", not
    // "nothing here". Its smaller sizes must keep that color instead of going black.
    let mut pixels = Vec::new();
    for y in 0..64u32 {
        for _ in 0..64u32 {
            pixels.extend(if y < 32 { [200, 100, 50, 255] } else { [100, 200, 50, 0] });
        }
    }
    let mut png = Vec::new();
    image::RgbaImage::from_raw(64, 64, pixels).unwrap().write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let map = decode_material_map(&png, 32).unwrap();
    assert_eq!((map.width, map.height), (32, 32));
    let smallest = map.levels.last().unwrap();
    assert!(smallest[0] > 120 && smallest[1] > 120, "both halves count: {smallest:?}");
    assert!((100..=155).contains(&smallest[3]), "alpha averages too: {smallest:?}");
}

#[test]
fn materials_decals_and_textures_come_in_with_a_model() {
    let scene = import_dom(&dressed_rig(), "dressed.rbxm").unwrap().scene;
    let torso = part_named(&scene, "Torso");
    assert_eq!(torso.material, Material::Brick);
    assert_eq!(torso.decals.len(), 1);
    let decal = &torso.decals[0];
    assert_eq!((decal.texture.as_str(), decal.face, decal.tiling), ("rbxassetid://123", Face::Front, None));
    assert_eq!(decal.color, [1.0, 0.5, 0.25]);
    assert_eq!(decal.transparency, 0.25);
    assert!(!torso.locked, "Studio's Locked isn't read, so limbs stay clickable");

    let arm = part_named(&scene, "Arm");
    assert_eq!(arm.material, Material::Grass);
    let texture = &arm.decals[0];
    assert_eq!(texture.face, Face::Top);
    assert_eq!(texture.tiling, Some(Tiling { studs_u: 8.0, studs_v: 4.0, offset_u: 1.0, offset_v: 0.0 }));
}

#[test]
fn every_face_lays_a_picture_flat_and_facing_out() {
    for value in 0..6 {
        let face = Face::from_roblox(value);
        assert_eq!(face.roblox_value(), value);
        let (out, right, down) = face.axes();
        // The picture's right and up, crossed, point out of the face, or it'd be culled.
        assert!(right.cross(-down).abs_diff_eq(out, 1e-6), "{face:?}");
    }
}

fn decorated_block(scene: &mut Scene) -> usize {
    let bone = scene.add_shape(PartShape::Block, Xf::new(Quat::IDENTITY, Vec3::new(0.0, 0.5, 0.0)), Vec3::new(12.0, 1.0, 12.0));
    let part = scene.bones[bone].part.unwrap();
    let p = &mut scene.parts[part];
    p.material = Material::DiamondPlate;
    p.locked = true;
    p.decals = vec![Decal { texture: "rbxassetid://123".to_string(), face: Face::Top, color: [1.0; 3], transparency: 0.0, tiling: None }];
    scene.evaluate();
    bone
}

#[test]
fn materials_decals_and_locks_survive_a_save() {
    let mut scene = Scene::default();
    let bone = decorated_block(&mut scene);
    let part = scene.bones[bone].part.unwrap();
    let path = std::env::temp_dir().join(format!("kaye-materials-{}.kaye", std::process::id()));
    let view = View { frame: 0.0, target: Vec3::ZERO, yaw: 0.0, pitch: 0.0, dist: 10.0, ortho: false, floor_y: 0.0, camera: None, preview: false, through: false, motion_paths: Vec::new(), references: Vec::new() };
    project::save(&path, &scene, &Animation::default(), &view).unwrap();
    let opened = project::load(&path).unwrap();
    std::fs::remove_file(&path).ok();
    let p = &opened.scene.parts[part];
    assert_eq!((p.material, p.locked), (Material::DiamondPlate, true));
    assert_eq!(p.decals, scene.parts[part].decals);
}

#[test]
fn a_shape_goes_out_with_its_material_lock_and_decals() {
    let mut scene = Scene::default();
    let bone = decorated_block(&mut scene);
    let rig = scene.bones[bone].rig;
    let (bytes, left_out) = export::items_file(&scene, &Animation::default(), &[export::Item::Rig(rig)]).unwrap();
    assert!(left_out.is_empty(), "{left_out:?}");
    let dom: WeakDom = rbx_binary::from_reader(&bytes[..]).unwrap();
    let part = dom.descendants().find(|i| i.name == "Block" && i.class == "Part").expect("the shape's part");
    assert_eq!(part.properties.get(&ustr("Material")), Some(&Variant::Enum(Enum::from_u32(1056))));
    assert_eq!(part.properties.get(&ustr("Locked")), Some(&Variant::Bool(true)));
    let decal = part.children().iter().map(|c| dom.get_by_ref(*c).unwrap()).find(|c| c.class == "Decal").expect("the decal");
    assert_eq!(decal.properties.get(&ustr("Face")), Some(&Variant::Enum(Enum::from_u32(1))));

    // And reading the file back gives the same thing.
    let back = import_bytes(&bytes, "block.rbxm").unwrap().scene;
    let p = back.parts.iter().find(|p| p.material == Material::DiamondPlate).expect("the shape came back");
    assert_eq!(p.decals.len(), 1);
    assert_eq!(p.decals[0].texture, "rbxassetid://123");
    assert_eq!(p.decals[0].face, Face::Top);
}

#[test]
fn ground_the_size_of_a_baseplate_stays_out_of_framing_and_shadows() {
    let mut scene = Scene::default();
    let cube = scene.add_shape(PartShape::Block, Xf::new(Quat::IDENTITY, Vec3::new(0.0, 1.0, 0.0)), Vec3::splat(2.0));
    let base = scene.add_shape(PartShape::Block, Xf::new(Quat::IDENTITY, Vec3::new(0.0, -8.0, 0.0)), Vec3::new(2048.0, 16.0, 2048.0));
    scene.evaluate();
    let (min, max) = scene.bounds(None).unwrap();
    assert!(max.x - min.x < 10.0, "only the cube counts: {min} to {max}");
    // Asked about on its own, the baseplate still has its size, for framing it.
    let (min, max) = scene.bounds(Some(scene.bones[base].rig)).unwrap();
    assert!(max.x - min.x > 2000.0);
    let _ = cube;
}

#[test]
fn part_colors_turn_on_for_settings_saved_with_the_old_default() {
    let old: Settings = serde_json::from_str(r#"{"look":2,"viewport":{"part_colors":false}}"#).unwrap();
    assert!(old.clone().upgraded().viewport.part_colors, "textures were hidden on every new install");
    assert!(Settings::default().viewport.part_colors);
    // Once upgraded, turning it off on purpose sticks.
    let mut chosen = old.upgraded();
    chosen.viewport.part_colors = false;
    let text = serde_json::to_string(&chosen).unwrap();
    assert!(!serde_json::from_str::<Settings>(&text).unwrap().upgraded().viewport.part_colors);
}

#[test]
fn colors_set_on_their_own_are_kept_by_name() {
    let text = r#"{"theme":{"colors":{"rig_icon":[255,0,0],"no_such_color":[1,2,3]}}}"#;
    let settings: Settings = serde_json::from_str(text).unwrap();
    assert_eq!(settings.theme.colors.get("rig_icon"), Some(&[255, 0, 0]));
    // A name a newer Kaye saved doesn't spoil the rest of the file.
    assert_eq!(settings.theme.colors.len(), 2);
    assert_eq!(settings.viewport, Settings::default().viewport);
}
