//! Roblox mesh files of every version, built here byte by byte, and what's done with
//! them. There are no real files to test against until an API key exists, so these
//! follow the community's specification of the format.

use glam::{Mat4, Quat, Vec3};
use kaye::assets::{self, AssetStore, Slot};
use kaye::math::Xf;
use kaye::rbxmesh::{self, RobloxMesh};
use std::sync::Arc;

const POSITIONS: [[f32; 3]; 4] = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 4.0, 0.0], [0.0, 0.0, 6.0]];
const FACES: [[u32; 3]; 2] = [[0, 1, 2], [0, 2, 3]];

fn floats(out: &mut Vec<u8>, values: &[f32]) {
    values.iter().for_each(|v| out.extend(v.to_le_bytes()));
}

fn vertex(out: &mut Vec<u8>, i: usize, size: usize) {
    floats(out, &POSITIONS[i]);
    floats(out, &[0.0, 1.0, 0.0]);
    floats(out, &[i as f32 * 0.25, 0.5]);
    out.extend([0, 0, 0, 127]);
    if size == 40 {
        out.extend([255; 4]);
    }
}

fn vertices(out: &mut Vec<u8>, size: usize) {
    (0..POSITIONS.len()).for_each(|i| vertex(out, i, size));
}

fn faces(out: &mut Vec<u8>, list: &[[u32; 3]]) {
    list.iter().flatten().for_each(|i| out.extend(i.to_le_bytes()));
}

fn u32s(out: &mut Vec<u8>, values: &[u32]) {
    values.iter().for_each(|v| out.extend(v.to_le_bytes()));
}

fn check_shape(mesh: &RobloxMesh) {
    assert_eq!(mesh.positions, POSITIONS.map(Vec3::from_array).to_vec());
    assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
    assert_eq!(mesh.uvs[2], [0.5, 0.5]);
    assert_eq!(mesh.normals[1], Vec3::Y);
}

fn v2(vertex_size: usize) -> Vec<u8> {
    let mut out = b"version 2.00\n".to_vec();
    out.extend(12u16.to_le_bytes());
    out.extend([vertex_size as u8, 12]);
    u32s(&mut out, &[4, 2]);
    vertices(&mut out, vertex_size);
    faces(&mut out, &FACES);
    out
}

/// A mesh with a second level of detail after the first, which must be left out.
fn v3() -> Vec<u8> {
    let mut out = b"version 3.00\n".to_vec();
    out.extend(16u16.to_le_bytes());
    out.extend([40u8, 12]);
    out.extend(4u16.to_le_bytes());
    out.extend(3u16.to_le_bytes());
    u32s(&mut out, &[4, 3]);
    vertices(&mut out, 40);
    faces(&mut out, &FACES);
    faces(&mut out, &[[1, 2, 3]]);
    u32s(&mut out, &[0, 2, 3]);
    out
}

fn bone(out: &mut Vec<u8>, name_start: u32) {
    u32s(out, &[name_start]);
    out.extend([0xFF, 0xFF, 0xFF, 0xFF]);
    floats(out, &[0.0]);
    floats(out, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    floats(out, &[0.0, 0.0, 0.0]);
}

/// One subset of every vertex, whose bones are listed the other way around from the
/// mesh, so reading the place in the subset as the bone itself would go wrong.
fn subset(out: &mut Vec<u8>) {
    u32s(out, &[0, 2, 0, 4, 2]);
    out.extend(1u16.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    (2..26).for_each(|_| out.extend(0xFFFFu16.to_le_bytes()));
}

fn skinning(out: &mut Vec<u8>) {
    // The first two vertices belong to the subset's first bone, which is mesh bone 1.
    for _ in 0..2 {
        out.extend([0, 0, 0, 0, 255, 0, 0, 0]);
    }
    for _ in 0..2 {
        out.extend([1, 0, 0, 0, 255, 0, 0, 0]);
    }
}

fn check_skin(mesh: &RobloxMesh) {
    assert_eq!(mesh.bones, vec!["Upper".to_string(), "Lower".to_string()]);
    assert_eq!(mesh.weights[0].bones[0], 1);
    assert_eq!(mesh.weights[3].bones[0], 0);
    assert_eq!(mesh.weights[0].amounts[0], 255);
}

fn v4(version: &str) -> Vec<u8> {
    let header_size: u16 = if version == "5.00" { 32 } else { 24 };
    let mut out = format!("version {version}\n").into_bytes();
    out.extend(header_size.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    u32s(&mut out, &[4, 3]);
    out.extend(3u16.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    u32s(&mut out, &[12]);
    out.extend(1u16.to_le_bytes());
    out.extend([1, 0]);
    if version == "5.00" {
        u32s(&mut out, &[0, 0]);
    }
    vertices(&mut out, 40);
    skinning(&mut out);
    faces(&mut out, &FACES);
    faces(&mut out, &[[1, 2, 3]]);
    u32s(&mut out, &[0, 2, 3]);
    bone(&mut out, 0);
    bone(&mut out, 6);
    out.extend(b"Upper\0Lower\0");
    subset(&mut out);
    out
}

fn chunk(out: &mut Vec<u8>, name: &[u8; 8], version: u32, data: &[u8]) {
    out.extend(name);
    u32s(out, &[version, data.len() as u32]);
    out.extend(data);
}

fn skin_chunk() -> Vec<u8> {
    let mut data = Vec::new();
    u32s(&mut data, &[4]);
    skinning(&mut data);
    u32s(&mut data, &[2]);
    bone(&mut data, 0);
    bone(&mut data, 6);
    u32s(&mut data, &[12]);
    data.extend(b"Upper\0Lower\0");
    u32s(&mut data, &[1]);
    subset(&mut data);
    data
}

fn lods_chunk() -> Vec<u8> {
    let mut data = vec![0, 0, 1];
    u32s(&mut data, &[2, 0, 2]);
    data
}

fn v6() -> Vec<u8> {
    let mut out = b"version 6.00\n".to_vec();
    let mut core = Vec::new();
    u32s(&mut core, &[4]);
    vertices(&mut core, 40);
    u32s(&mut core, &[2]);
    faces(&mut core, &FACES);
    chunk(&mut out, b"COREMESH", 1, &core);
    chunk(&mut out, b"LODS\0\0\0\0", 1, &lods_chunk());
    chunk(&mut out, b"SKINNING", 1, &skin_chunk());
    chunk(&mut out, b"HSRAVIS\0", 1, &[0; 8]);
    out
}

/// The same mesh through a real Draco encoder, as version 7 stores it.
fn v7() -> Vec<u8> {
    use draco_core::{DataType, EncoderBuffer, EncoderOptions, GeometryAttributeType, Mesh, MeshEncoder, PointAttribute};
    let mut mesh = Mesh::new();
    mesh.set_num_points(4);
    let mut add = |kind: GeometryAttributeType, components: u8, values: Vec<f32>| {
        let mut attribute = PointAttribute::new();
        attribute.init(kind, components, DataType::Float32, false, 4);
        attribute.buffer_mut().write(0, bytemuck::cast_slice(&values));
        mesh.add_attribute(attribute);
    };
    add(GeometryAttributeType::Position, 3, POSITIONS.iter().flatten().copied().collect());
    add(GeometryAttributeType::Normal, 3, [0.0, 1.0, 0.0].repeat(4));
    add(GeometryAttributeType::TexCoord, 2, (0..4).flat_map(|i| [i as f32 * 0.25, 0.5]).collect());
    mesh.set_num_faces(FACES.len());
    for (i, face) in FACES.iter().enumerate() {
        mesh.set_face_from_indices(i, *face);
    }
    let mut buffer = EncoderBuffer::new();
    let mut options = EncoderOptions::default();
    options.set_global_int("encoding_speed", 10);
    options.set_global_int("decoding_speed", 10);
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    encoder.encode(&options, &mut buffer).expect("encodes");
    let stream = buffer.data();

    let mut out = b"version 7.00\n".to_vec();
    let mut core = Vec::new();
    u32s(&mut core, &[stream.len() as u32]);
    core.extend(stream);
    chunk(&mut out, b"COREMESH", 2, &core);
    chunk(&mut out, b"LODS\0\0\0\0", 1, &lods_chunk());
    out
}

#[test]
fn version_1_text_reads_and_1_00_is_halved() {
    let face = "[2,0,0][0,1,0][0,0.25,0][0,2,0][0,1,0][0.5,1,0][0,0,2][0,1,0][1,1,0]";
    let mesh = rbxmesh::parse(format!("version 1.00\n1\n{face}").as_bytes()).unwrap();
    assert_eq!(mesh.positions[0], Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert_eq!(mesh.uvs[0], [0.0, 0.75], "V is flipped");
    let mesh = rbxmesh::parse(format!("version 1.01\n1\n{face}").as_bytes()).unwrap();
    assert_eq!(mesh.positions[0], Vec3::new(2.0, 0.0, 0.0));
}

#[test]
fn version_2_reads_with_or_without_vertex_colors() {
    check_shape(&rbxmesh::parse(&v2(36)).unwrap());
    check_shape(&rbxmesh::parse(&v2(40)).unwrap());
}

#[test]
fn version_3_keeps_only_the_best_level_of_detail() {
    check_shape(&rbxmesh::parse(&v3()).unwrap());
}

#[test]
fn versions_4_and_5_read_with_their_bones() {
    for version in ["4.00", "4.01", "5.00"] {
        let mesh = rbxmesh::parse(&v4(version)).unwrap();
        check_shape(&mesh);
        check_skin(&mesh);
    }
}

#[test]
fn version_6_reads_its_chunks_and_skips_the_rest() {
    let mesh = rbxmesh::parse(&v6()).unwrap();
    check_shape(&mesh);
    check_skin(&mesh);
}

#[test]
fn version_7_decompresses_draco() {
    let mesh = rbxmesh::parse(&v7()).unwrap();
    assert_eq!(mesh.indices.len(), 6);
    assert_eq!(mesh.positions.len(), mesh.normals.len());
    assert_eq!(mesh.positions.len(), mesh.uvs.len());
    let (center, size) = mesh.bounds();
    assert!(center.distance(Vec3::new(1.0, 2.0, 3.0)) < 1e-2, "{center}");
    assert!(size.distance(Vec3::new(2.0, 4.0, 6.0)) < 1e-2, "{size}");
}

#[test]
fn a_cut_short_or_foreign_file_is_an_error_not_a_crash() {
    assert!(rbxmesh::parse(b"not a mesh").is_err());
    assert!(rbxmesh::parse(b"version 9.00\n").is_err());
    for full in [v2(40), v3(), v4("4.00"), v4("5.00"), v6(), v7()] {
        for keep in (0..full.len()).step_by(7) {
            let _ = rbxmesh::parse(&full[..keep]);
        }
    }
}

#[test]
fn a_face_past_the_vertices_is_turned_away() {
    let mut mesh = v2(36);
    let at = mesh.len() - 12;
    mesh[at] = 200;
    assert!(rbxmesh::parse(&mesh).is_err());
}

#[test]
fn asset_ids_are_found_however_they_are_written() {
    for (text, id) in [
        ("rbxassetid://123", Some(123)),
        ("http://www.roblox.com/asset/?id=456", Some(456)),
        ("https://assetdelivery.roblox.com/v1/asset?id=789", Some(789)),
        ("789", Some(789)),
        ("rbxasset://fonts/torso.mesh", None),
        ("rbxassetid://0", None),
        ("", None),
    ] {
        assert_eq!(rbxmesh::asset_id(text), id, "{text}");
    }
}

fn bent(mesh: &RobloxMesh, moves: &[Option<Xf>], still: Xf) -> Vec<Vec3> {
    mesh.skinned(Mat4::IDENTITY, moves, still).iter().map(|v| Vec3::from_array(v.pos)).collect()
}

#[test]
fn skinning_moves_each_vertex_with_its_bone() {
    let mesh = rbxmesh::parse(&v6()).unwrap();
    let up = Xf::new(Quat::IDENTITY, Vec3::Y);
    let right = Xf::new(Quat::IDENTITY, Vec3::X);
    // Vertices 0 and 1 belong to mesh bone 1, and 2 and 3 to mesh bone 0.
    let bent = bent(&mesh, &[Some(up), Some(right)], Xf::IDENTITY);
    assert_eq!(bent[0], Vec3::from_array(POSITIONS[0]) + Vec3::X);
    assert_eq!(bent[2], Vec3::from_array(POSITIONS[2]) + Vec3::Y);
}

#[test]
fn a_bone_not_in_the_rig_leaves_its_vertices_with_the_part() {
    let mesh = rbxmesh::parse(&v6()).unwrap();
    let carried = Xf::new(Quat::IDENTITY, Vec3::Z);
    let bent = bent(&mesh, &[None, None], carried);
    assert_eq!(bent[1], Vec3::from_array(POSITIONS[1]) + Vec3::Z);
}

#[test]
fn weights_blend_between_bones() {
    let mut mesh = rbxmesh::parse(&v6()).unwrap();
    mesh.weights[0].bones = [0, 1, u16::MAX, u16::MAX];
    mesh.weights[0].amounts = [128, 127, 0, 0];
    let bent = bent(&mesh, &[Some(Xf::new(Quat::IDENTITY, Vec3::X)), Some(Xf::new(Quat::IDENTITY, -Vec3::X))], Xf::IDENTITY);
    assert!(bent[0].length() < 0.01, "{}", bent[0]);
}

#[test]
fn skinned_normals_turn_with_the_bone() {
    let mesh = rbxmesh::parse(&v6()).unwrap();
    let turned = Xf::new(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), Vec3::ZERO);
    let out = mesh.skinned(Mat4::IDENTITY, &[Some(turned), Some(turned)], Xf::IDENTITY);
    let normal = Vec3::from_array(out[0].normal);
    assert!(normal.distance(Vec3::NEG_X) < 1e-4, "{normal}");
}

#[test]
fn textures_decode_with_a_full_chain_of_sizes() {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(8, 4, image::Rgba([10, 20, 30, 40])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let texture = assets::decode_texture(&png).unwrap();
    assert_eq!((texture.width, texture.height), (8, 4));
    assert_eq!(texture.levels.len(), 4);
    assert_eq!(texture.levels.last().unwrap(), &vec![10, 20, 30, 40]);
    assert_eq!(texture.levels[1].len(), 4 * 2 * 4);
}

fn store(folder: std::path::PathBuf) -> AssetStore {
    AssetStore::new(Some(folder), String::new(), Arc::new(|| {}))
}

fn wait_for(store: &mut AssetStore, id: u64) -> Slot<RobloxMesh> {
    for _ in 0..500 {
        store.poll();
        let slot = store.mesh(id);
        if !matches!(slot, Slot::Waiting) {
            return slot;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the asset never finished");
}

#[test]
fn real_version_7_files_leave_the_detail_table_empty_and_mean_every_face() {
    let mut file = v7();
    // Real files carry zeros here. Reading that as a range gave a mesh with no faces.
    let at = file.len() - 8;
    file[at..].fill(0);
    assert_eq!(rbxmesh::parse(&file).unwrap().indices.len(), 6);
}

#[test]
fn a_shirt_lies_over_pants_where_it_has_color() {
    let solid = |pixel: [u8; 4]| assets::decode_texture(&png(pixel)).unwrap();
    let both = assets::layered_texture(&solid([255, 0, 0, 0]), &solid([0, 0, 255, 255]));
    assert_eq!(both.levels[0][..4], [0, 0, 255, 255], "clear shirt shows the pants");
    let both = assets::layered_texture(&solid([255, 0, 0, 255]), &solid([0, 0, 255, 255]));
    assert_eq!(both.levels[0][..4], [255, 0, 0, 255], "solid shirt hides them");
}

fn png(pixel: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(2, 2, image::Rgba(pixel)).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}

#[test]
fn a_rigs_shirt_and_pants_are_read() {
    let scene = kaye::import::import_file(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/R6WithAccessories.rbxm")).unwrap().scene;
    assert_eq!(scene.rigs[0].shirt.as_deref().and_then(rbxmesh::asset_id), Some(11112380474));
    assert_eq!(scene.rigs[0].pants.as_deref().and_then(rbxmesh::asset_id), Some(16701819635));
}

#[test]
fn a_cached_asset_needs_no_key_and_no_network() {
    let folder = std::env::temp_dir().join(format!("kaye-cache-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("555"), v6()).unwrap();
    let mut assets = store(folder.clone());
    assert!(matches!(wait_for(&mut assets, 555), Slot::Ready(_)));
    assert!(assets.loaded_mesh(555).is_some());
    std::fs::remove_dir_all(&folder).ok();
}

#[test]
fn with_no_key_an_asset_is_simply_missing() {
    let mut assets = store(std::env::temp_dir().join("kaye-no-such-cache"));
    assert!(matches!(wait_for(&mut assets, 12345), Slot::Missing));
    assert!(assets.is_missing_mesh(12345));
    assert!(!assets.key_refused, "no key isn't a refused key");
}

#[test]
fn a_new_key_tries_missing_assets_again() {
    let mut assets = store(std::env::temp_dir().join("kaye-no-such-cache"));
    wait_for(&mut assets, 777);
    assets.set_key("a key");
    assert!(!assets.is_missing_mesh(777));
}
