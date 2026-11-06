//! Clothing on R15, heads and shapes Roblox draws itself, and CharacterMesh. The atlas
//! layout is checked against rectangles read from Roblox's own compositing meshes.

use std::path::Path;

use glam::Vec3;
use kaye::assets::{self, TextureData};
use kaye::clothing::BodyAtlas;
use kaye::import::{import_dom, import_file};
use kaye::math::ray_triangle;
use kaye::rbxmesh::{self, RobloxMesh};
use kaye::render::meshes::{self, MeshKind};
use kaye::scene::{BuiltShape, MeshFit, Scene};
use rbx_dom_weak::{InstanceBuilder, WeakDom};
use rbx_types::{CFrame, Enum, Matrix3, Variant, Vector3};

const WIDTH: u32 = 585;
const HEIGHT: u32 = 559;

fn sample(name: &str) -> Scene {
    import_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(name)).unwrap().scene
}

/// A template where each pixel says where it is: red and green hold x, blue holds y.
fn map_template() -> TextureData {
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            pixels.extend([(x / 256) as u8, (x % 256) as u8, (y / 3) as u8, 255]);
        }
    }
    TextureData { width: WIDTH, height: HEIGHT, levels: vec![pixels] }
}

fn solid(pixel: [u8; 4]) -> TextureData {
    let pixels = pixel.iter().copied().cycle().take((WIDTH * HEIGHT * 4) as usize).collect();
    TextureData { width: WIDTH, height: HEIGHT, levels: vec![pixels] }
}

/// The atlas pixel at a spot counted up from its bottom edge, as Roblox's meshes count it.
fn atlas_pixel(texture: &TextureData, x: u32, up: u32) -> [u8; 4] {
    let row = texture.height - 1 - up;
    let at = ((row * texture.width + x) * 4) as usize;
    texture.levels[0][at..at + 4].try_into().unwrap()
}

/// Which template pixel an atlas pixel came from, as (x, y to within 3).
fn came_from(texture: &TextureData, x: u32, up: u32) -> (u32, u32) {
    let [high, low, y, alpha] = atlas_pixel(texture, x, up);
    assert_eq!(alpha, 255, "nothing was drawn at {x}, {up}");
    (high as u32 * 256 + low as u32, y as u32 * 3)
}

fn assert_from(texture: &TextureData, x: u32, up: u32, want: (u32, u32)) {
    let got = came_from(texture, x, up);
    assert!(got.0.abs_diff(want.0) <= 1 && got.1.abs_diff(want.1) <= 3, "atlas {x}, {up} came from {got:?}, wanted {want:?}");
}

#[test]
fn the_left_arm_atlas_matches_roblox_layout() {
    let atlas = BodyAtlas::LeftArm.build(Some(&map_template()), None);
    assert_eq!((atlas.width, atlas.height), (264, 284));
    // Upper and lower arm, each side, from the template's 112 rows above the hand.
    assert_from(&atlas, 2 + 32, 106 + 56, (506 + 32, 467 - 56));
    assert_from(&atlas, 66 + 32, 106 + 56, (308 + 32, 467 - 56));
    assert_from(&atlas, 130 + 32, 106 + 56, (374 + 32, 467 - 56));
    assert_from(&atlas, 194 + 32, 106 + 56, (440 + 32, 467 - 56));
    // The hand takes the last 16 rows, with its back apart from the others.
    assert_from(&atlas, 66 + 32, 50 + 8, (308 + 32, 483 - 8));
    assert_from(&atlas, 2 + 32, 30 + 8, (440 + 32, 483 - 8));
    // The top is flipped across, the underside isn't.
    assert_from(&atlas, 2 + 8, 218 + 32, (308 + 56, 353 - 32));
    assert_from(&atlas, 198 + 8, 2 + 32, (308 + 8, 549 - 32));
}

#[test]
fn the_right_arm_atlas_matches_roblox_layout() {
    let atlas = BodyAtlas::RightArm.build(Some(&map_template()), None);
    assert_from(&atlas, 2 + 32, 106 + 56, (151 + 32, 467 - 56));
    assert_from(&atlas, 66 + 32, 106 + 56, (217 + 32, 467 - 56));
    assert_from(&atlas, 130 + 32, 106 + 56, (19 + 32, 467 - 56));
    assert_from(&atlas, 194 + 32, 106 + 56, (85 + 32, 467 - 56));
}

#[test]
fn the_torso_atlas_matches_roblox_layout() {
    let atlas = BodyAtlas::Torso.build(Some(&map_template()), None);
    assert_eq!((atlas.width, atlas.height), (388, 272));
    assert_from(&atlas, 2 + 64, 70 + 64, (231 + 64, 202 - 64));
    assert_from(&atlas, 130 + 32, 70 + 64, (361 + 32, 202 - 64));
    assert_from(&atlas, 194 + 64, 70 + 64, (427 + 64, 202 - 64));
    assert_from(&atlas, 322 + 32, 70 + 64, (165 + 32, 202 - 64));
    assert_from(&atlas, 2 + 64, 198 + 32, (231 + 64, 72 - 32));
    assert_from(&atlas, 2 + 64, 6 + 32, (231 + 64, 268 - 32));
}

#[test]
fn panels_bleed_past_their_edges_so_a_seam_finds_no_gap() {
    let atlas = BodyAtlas::Torso.build(Some(&solid([10, 20, 30, 255])), None);
    assert_eq!(atlas_pixel(&atlas, 1, 100)[3], 255, "the bleed left of the front");
    assert_eq!(atlas_pixel(&atlas, 300, 250)[3], 0, "the empty corner stays clear");
}

#[test]
fn each_atlas_only_wears_what_covers_it() {
    let (shirt, pants) = (solid([255, 0, 0, 255]), solid([0, 0, 255, 255]));
    let spot = (2 + 32, 106 + 56);
    let arm = BodyAtlas::LeftArm.build(Some(&shirt), Some(&pants));
    assert_eq!(atlas_pixel(&arm, spot.0, spot.1), [255, 0, 0, 255], "an arm shows the shirt");
    let leg = BodyAtlas::LeftLeg.build(Some(&shirt), Some(&pants));
    assert_eq!(atlas_pixel(&leg, spot.0, spot.1), [0, 0, 255, 255], "a leg shows the pants");
    let torso = BodyAtlas::Torso.build(Some(&shirt), Some(&pants));
    assert_eq!(atlas_pixel(&torso, 66, 134), [255, 0, 0, 255], "a solid shirt hides the pants");
    let open = BodyAtlas::Torso.build(Some(&solid([255, 0, 0, 0])), Some(&pants));
    assert_eq!(atlas_pixel(&open, 66, 134), [0, 0, 255, 255], "a clear shirt shows them");
}

#[test]
fn every_r15_body_part_has_an_atlas() {
    for rig_part in sample("R15Rig.rbxm").parts.iter().filter(|p| p.mesh_asset.is_some()) {
        assert!(BodyAtlas::of_part(&rig_part.name).is_some(), "{}", rig_part.name);
    }
    assert_eq!(BodyAtlas::of_part("Torso"), None, "R6 stays on the template");
    assert_eq!(BodyAtlas::of_part("Handle"), None);
    assert_eq!(BodyAtlas::of_part("UpperTorso"), BodyAtlas::of_part("LowerTorso"), "the torso is one atlas");
}

#[test]
fn an_atlas_is_redrawn_when_the_other_clothing_arrives() {
    assert_ne!(BodyAtlas::Torso.id(Some(1), None), BodyAtlas::Torso.id(Some(1), Some(2)));
    assert_ne!(BodyAtlas::LeftArm.id(Some(1), None), BodyAtlas::RightArm.id(Some(1), None));
    assert!(BodyAtlas::Torso.id(Some(1), None) >= 1 << 63, "clear of real asset ids");
}

#[test]
fn clear_pixels_do_not_darken_smaller_sizes() {
    let mut bytes = Vec::new();
    let mut image = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 0]));
    image.put_pixel(0, 0, image::Rgba([200, 100, 50, 255]));
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    let texture = assets::decode_texture(&bytes).unwrap();
    let small = &texture.levels[1];
    assert_eq!(small[..3], [200, 100, 50], "the color survives");
    assert_eq!(small[3], 63, "and it covers a quarter");
}

#[test]
fn a_plain_rigs_head_is_the_classic_shape_wearing_its_face() {
    for name in ["R6Rig.rbxm", "R15Rig.rbxm"] {
        let scene = sample(name);
        let head = scene.parts.iter().find(|p| p.name == "Head").unwrap();
        assert!(matches!(head.mesh_fit, MeshFit::Built { shape: BuiltShape::Head, .. }), "{name}");
        assert_eq!(head.mesh_asset, None);
        assert_eq!(head.texture_asset.as_deref(), Some("rbxasset://textures/face.png"));
        // A 2 by 1 by 1 part at Roblox's usual 1.25 is as round as it's tall.
        let (center, size) = head.drawn_box();
        assert_eq!(center, Vec3::ZERO);
        assert!((size - Vec3::splat(1.25)).length() < 1e-5, "{size}");
    }
}

#[test]
fn a_head_with_its_own_mesh_stays_that_mesh() {
    let scene = sample("R6WithAccessories.rbxm");
    let head = scene.parts.iter().find(|p| p.name == "Head").unwrap();
    assert!(matches!(head.mesh_fit, MeshFit::Scaled { .. }));
    assert_eq!(head.mesh_asset.as_deref().and_then(rbxmesh::asset_id), Some(75121822706551));
    assert_eq!(head.texture_asset.as_deref().and_then(rbxmesh::asset_id), Some(83021747312619), "its texture, not the face decal");
}

#[test]
fn the_head_mesh_fills_a_unit_box_and_shows_a_face_only_on_its_front() {
    let (data, ranges) = meshes::build_all();
    let range = ranges.iter().find(|(kind, _)| *kind == MeshKind::Head).unwrap().1;
    assert!(range.index_count > 0);
    let indices = &data.indices[range.first_index as usize..(range.first_index + range.index_count) as usize];
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    let mut low = Vec3::splat(f32::INFINITY);
    for vertex in indices.iter().map(|i| &data.vertices[*i as usize]) {
        let p = Vec3::from_array(vertex.pos);
        (low, high) = (low.min(p), high.max(p));
        assert!((Vec3::from_array(vertex.normal).length() - 1.0).abs() < 1e-3);
        if p.z > 1e-4 {
            assert_eq!(vertex.uv, [0.01, 0.01], "the back of the head stays clear");
        } else if p.z < -1e-4 {
            assert!((vertex.uv[0] - (0.5 - p.x)).abs() < 1e-5 && (vertex.uv[1] - (0.5 - p.y)).abs() < 1e-5);
        }
    }
    assert!((high - Vec3::splat(0.5)).length() < 1e-5 && (low + Vec3::splat(0.5)).length() < 1e-5, "{low} {high}");
    // Every triangle faces out, or the head would draw inside out.
    for t in indices.chunks_exact(3) {
        let p = |i: usize| Vec3::from_array(data.vertices[t[i] as usize].pos);
        let out = (p(1) - p(0)).cross(p(2) - p(0));
        assert!(out.dot((p(0) + p(1) + p(2)) / 3.0) > 0.0);
    }
}

/// A rig with one part and whatever's asked for inside it, as a file would hold it.
fn dom_with(part_name: &str, size: Vector3, extras: Vec<InstanceBuilder>, beside: Vec<InstanceBuilder>) -> WeakDom {
    let identity = CFrame::new(Vector3::new(0.0, 0.0, 0.0), Matrix3::identity());
    let part = InstanceBuilder::new("Part").with_name(part_name).with_property("CFrame", Variant::CFrame(identity)).with_property("Size", Variant::Vector3(size)).with_children(extras);
    let rig = InstanceBuilder::new("Model").with_name("Rig").with_child(part).with_children(beside);
    WeakDom::new(InstanceBuilder::new("DataModel").with_child(rig))
}

fn special_mesh(mesh_type: u32) -> InstanceBuilder {
    InstanceBuilder::new("SpecialMesh")
        .with_property("MeshType", Variant::Enum(Enum::from_u32(mesh_type)))
        .with_property("Scale", Variant::Vector3(Vector3::new(2.0, 3.0, 4.0)))
        .with_property("Offset", Variant::Vector3(Vector3::new(0.0, 1.0, 0.0)))
}

#[test]
fn a_specialmesh_shape_scales_with_its_part() {
    for (mesh_type, shape) in [(3, BuiltShape::Sphere), (4, BuiltShape::Cylinder), (2, BuiltShape::Wedge), (6, BuiltShape::Block), (11, BuiltShape::CornerWedge), (9, BuiltShape::Block)] {
        let dom = dom_with("Thing", Vector3::new(1.0, 2.0, 1.0), vec![special_mesh(mesh_type)], vec![]);
        let scene = import_dom(&dom, "t.rbxm").unwrap().scene;
        let part = &scene.parts[0];
        assert!(matches!(part.mesh_fit, MeshFit::Built { shape: found, .. } if found == shape), "type {mesh_type}");
        let (center, size) = part.drawn_box();
        assert_eq!(center, Vec3::Y);
        assert_eq!(size, Vec3::new(2.0, 6.0, 4.0));
    }
}

#[test]
fn a_leftover_file_on_a_specialmesh_set_to_a_shape_is_ignored() {
    let mesh = special_mesh(0).with_property("MeshContent", Variant::String("rbxassetid://99".to_string()));
    let dom = dom_with("Head", Vector3::new(2.0, 1.0, 1.0), vec![mesh], vec![]);
    assert_eq!(import_dom(&dom, "t.rbxm").unwrap().scene.parts[0].mesh_asset, None);
}

#[test]
fn a_face_on_another_side_is_not_a_face() {
    let decal = |face: u32| InstanceBuilder::new("Decal").with_property("Face", Variant::Enum(Enum::from_u32(face))).with_property("TextureContent", Variant::String("rbxassetid://5".to_string()));
    let dom = dom_with("Head", Vector3::new(2.0, 1.0, 1.0), vec![special_mesh(0), decal(2)], vec![]);
    assert_eq!(import_dom(&dom, "t.rbxm").unwrap().scene.parts[0].texture_asset, None);
    let dom = dom_with("Head", Vector3::new(2.0, 1.0, 1.0), vec![special_mesh(0), decal(5)], vec![]);
    assert_eq!(import_dom(&dom, "t.rbxm").unwrap().scene.parts[0].texture_asset.as_deref(), Some("rbxassetid://5"));
}

fn character_mesh(body_part: u32, mesh: i64, base: i64) -> InstanceBuilder {
    InstanceBuilder::new("CharacterMesh")
        .with_property("BodyPart", Variant::Enum(Enum::from_u32(body_part)))
        .with_property("MeshId", Variant::Int64(mesh))
        .with_property("BaseTextureId", Variant::Int64(base))
}

#[test]
fn a_character_mesh_goes_on_the_body_part_it_names() {
    let dom = dom_with("Left Arm", Vector3::new(1.0, 2.0, 1.0), vec![], vec![character_mesh(2, 123, 456), character_mesh(3, 789, 0)]);
    let part = &import_dom(&dom, "t.rbxm").unwrap().scene.parts[0];
    assert_eq!(part.mesh_asset.as_deref().and_then(rbxmesh::asset_id), Some(123));
    assert_eq!(part.texture_asset.as_deref().and_then(rbxmesh::asset_id), Some(456));
    assert_eq!(part.mesh_fit, MeshFit::Scaled { scale: Vec3::ONE, offset: Vec3::ZERO });
}

#[test]
fn an_empty_character_mesh_changes_nothing() {
    let dom = dom_with("Left Arm", Vector3::new(1.0, 2.0, 1.0), vec![], vec![character_mesh(2, 0, 0)]);
    let part = &import_dom(&dom, "t.rbxm").unwrap().scene.parts[0];
    assert_eq!(part.mesh_asset, None);
    assert_eq!(part.mesh_fit, MeshFit::Size);
}

#[test]
fn shipped_files_have_to_stay_inside_robloxs_folder() {
    assert_eq!(rbxmesh::shipped_file("rbxasset://textures/face.png"), Some("textures/face.png"));
    assert_eq!(rbxmesh::shipped_file("rbxasset://../../secret.png"), None);
    assert_eq!(rbxmesh::shipped_file("rbxasset://textures/../../secret.png"), None);
    assert_eq!(rbxmesh::shipped_file("rbxasset:///etc/passwd"), None);
    assert_eq!(rbxmesh::shipped_file("rbxasset://C:/Windows/win.ini"), None);
    assert_eq!(rbxmesh::shipped_file("rbxassetid://5"), None);
    assert_ne!(assets::shipped_id("textures/face.png"), assets::shipped_id("textures/other.png"));
}

#[test]
fn an_older_projects_head_becomes_the_classic_one() {
    let mut scene = sample("R6Rig.rbxm");
    let head = scene.parts.iter().position(|p| p.name == "Head").unwrap();
    // What Kaye used to make of this head: a scale, no file, and no face.
    let scale = Vec3::splat(1.25);
    scene.parts[head].mesh_fit = MeshFit::Scaled { scale, offset: Vec3::ZERO };
    scene.parts[head].texture_asset = None;
    let path = std::env::temp_dir().join(format!("kaye-old-head-{}.kaye", std::process::id()));
    let view = kaye::project::View { frame: 0.0, target: Vec3::ZERO, yaw: 0.0, pitch: 0.0, dist: 10.0, ortho: false, floor_y: 0.0, camera: None, preview: false, through: false, motion_paths: Vec::new(), references: Vec::new() };
    kaye::project::save(&path, &scene, &kaye::animation::Animation::default(), &view).unwrap();
    let opened = kaye::project::load(&path).unwrap();
    std::fs::remove_file(&path).ok();
    let head = &opened.scene.parts[head];
    assert_eq!(head.mesh_fit, MeshFit::Built { shape: BuiltShape::Head, scale, offset: Vec3::ZERO });
    assert_eq!(head.texture_asset.as_deref(), Some("rbxasset://textures/face.png"));
}

fn tent() -> RobloxMesh {
    RobloxMesh {
        positions: vec![Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, -1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)],
        normals: vec![Vec3::Z; 3],
        uvs: vec![[0.0; 2]; 3],
        indices: vec![0, 1, 2],
        ..RobloxMesh::default()
    }
}

#[test]
fn a_ray_hits_a_mesh_on_its_surface_and_not_in_its_box() {
    let mesh = tent();
    let hit = mesh.ray_hit(Vec3::new(0.0, 0.0, -5.0), Vec3::Z).unwrap();
    assert!((hit - 5.0).abs() < 1e-5);
    assert_eq!(mesh.ray_hit(Vec3::new(0.9, 0.9, -5.0), Vec3::Z), None, "the box's corner is empty");
    assert_eq!(mesh.ray_hit(Vec3::new(0.0, 0.0, 5.0), Vec3::Z), None, "behind the ray");
    assert!(mesh.ray_hit(Vec3::new(0.0, 0.0, 5.0), -Vec3::Z).is_some(), "from either side");
}

#[test]
fn a_bent_mesh_is_hit_where_it_went() {
    let mesh = tent();
    let mut bent = mesh.vertices();
    bent.iter_mut().for_each(|v| v.pos[0] += 10.0);
    assert!(mesh.ray_hit_bent(&bent, Vec3::new(10.0, 0.0, -5.0), Vec3::Z).is_some());
    assert_eq!(mesh.ray_hit_bent(&bent, Vec3::new(0.0, 0.0, -5.0), Vec3::Z), None);
}

#[test]
fn a_ray_along_a_triangle_misses_it() {
    let flat = [Vec3::ZERO, Vec3::X, Vec3::Y];
    assert_eq!(ray_triangle(Vec3::new(-1.0, 0.2, 0.0), Vec3::X, flat), None);
}

/// A body package's own texture with a shirt atlas over it, as Roblox shows one. The atlas
/// is smaller, so the result keeps the body's size and stretches the atlas over it.
#[test]
fn clothes_go_over_a_body_texture_at_its_size() {
    // The body: 4 by 4, all bone white.
    let body = TextureData { width: 4, height: 4, levels: vec![[230, 220, 200, 255].repeat(16)] };
    // The shirt: 2 by 2, red on the left half and clear on the right.
    let shirt = TextureData { width: 2, height: 2, levels: vec![[[200, 0, 0, 255], [0, 0, 0, 0], [200, 0, 0, 255], [0, 0, 0, 0]].concat()] };
    let worn = assets::layered_texture(&shirt, &body);
    assert_eq!((worn.width, worn.height), (4, 4));
    let pixel = |x: usize, y: usize| &worn.levels[0][(y * 4 + x) * 4..(y * 4 + x) * 4 + 4];
    assert_eq!(pixel(0, 1), [200, 0, 0, 255], "the shirt covers the body");
    assert_eq!(pixel(3, 1), [230, 220, 200, 255], "the body shows where the shirt is clear");
}

/// Roblox shows a SurfaceAppearance instead of clothing, so its texture is marked.
#[test]
fn a_surface_appearance_texture_is_marked() {
    let appearance = InstanceBuilder::new("SurfaceAppearance").with_property("ColorMapContent", Variant::Content(rbx_types::Content::from_uri("rbxassetid://7")));
    let plain = dom_with("LeftHand", Vector3::new(1.0, 1.0, 1.0), vec![], vec![]);
    let pbr = dom_with("LeftHand", Vector3::new(1.0, 1.0, 1.0), vec![appearance], vec![]);
    let hand = |dom: &WeakDom| import_dom(dom, "test.rbxm").unwrap().scene.parts[0].clone();
    assert!(!hand(&plain).surface_appearance);
    let hand = hand(&pbr);
    assert!(hand.surface_appearance);
    assert_eq!(hand.texture_asset.as_deref(), Some("rbxassetid://7"));
}
