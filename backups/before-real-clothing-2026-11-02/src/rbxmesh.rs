//! Reads Roblox mesh files, every version from 1.00 to 7.00, and bends skinned ones.
//! Like the rig logic, this has no interface code.
//!
//! The format is internal to Roblox, so this follows the community's specification,
//! not an official one. Version 1 is text. Versions 2 to 5 are one fixed layout that
//! grows. Versions 6 and 7 are a list of named chunks, and 7's geometry is compressed
//! with Draco. Only the most detailed level of each mesh is kept.

use glam::{Mat3, Mat4, Vec3};

use crate::math::Xf;
use crate::render::meshes::Vertex;

const BONE_SIZE: usize = 60;
const SUBSET_SIZE: usize = 72;
const SKINNING_SIZE: usize = 8;
const SUBSET_BONES: usize = 26;
/// More than any real mesh, and it stops a damaged count from asking for all of memory.
const MOST_ELEMENTS: usize = 50_000_000;
/// A real mesh has no box edges to outline, so this tells the shader to light its rim
/// when selected instead.
const NO_EDGES: [f32; 3] = [2.0; 3];

/// Which bones move one vertex, and how much. Weights add up to about 255.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Weights {
    pub bones: [u16; 4],
    pub amounts: [u8; 4],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RobloxMesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// The names of the bones `weights` points at.
    pub bones: Vec<String>,
    /// One per vertex on a skinned mesh, otherwise empty.
    pub weights: Vec<Weights>,
}

impl RobloxMesh {
    /// The middle of the mesh and how big it is, which is what a MeshPart's Size scales.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let mut low = Vec3::splat(f32::INFINITY);
        let mut high = Vec3::splat(f32::NEG_INFINITY);
        for p in &self.positions {
            low = low.min(*p);
            high = high.max(*p);
        }
        if self.positions.is_empty() {
            return (Vec3::ZERO, Vec3::ONE);
        }
        ((low + high) * 0.5, (high - low).max(Vec3::splat(1e-4)))
    }

    /// The mesh in the renderer's format.
    pub fn vertices(&self) -> Vec<Vertex> {
        (0..self.positions.len())
            .map(|i| Vertex { pos: self.positions[i].into(), normal: self.normals[i].into(), edge: NO_EDGES, uv: self.uvs[i] })
            .collect()
    }

    /// Bends the mesh to where its bones are now. `fit` places a vertex at rest, `moves`
    /// is how far each of `bones` has moved from rest, and `still` is how far the part
    /// has, for vertices or shares of them that belong to no bone.
    pub fn skinned(&self, fit: Mat4, moves: &[Option<Xf>], still: Xf) -> Vec<Vertex> {
        let turn = Mat3::from_mat4(fit);
        let scale_of = |axis: Vec3| axis.length_squared().max(1e-8);
        let squares = Vec3::new(scale_of(turn.x_axis), scale_of(turn.y_axis), scale_of(turn.z_axis));
        (0..self.positions.len())
            .map(|i| {
                let rest = fit.transform_point3(self.positions[i]);
                let normal = (turn * (self.normals[i] / squares)).normalize_or_zero();
                let weights = self.weights.get(i).copied().unwrap_or_default();
                let (mut pos, mut turned, mut left) = (Vec3::ZERO, Vec3::ZERO, 1.0f32);
                for (bone, amount) in weights.bones.into_iter().zip(weights.amounts) {
                    if amount == 0 {
                        continue;
                    }
                    if let Some(Some(moved)) = moves.get(bone as usize) {
                        let share = amount as f32 / 255.0;
                        pos += moved.point(rest) * share;
                        turned += (moved.rot * normal) * share;
                        left -= share;
                    }
                }
                if left > 1e-3 {
                    pos += still.point(rest) * left;
                    turned += (still.rot * normal) * left;
                }
                Vertex { pos: pos.into(), normal: turned.normalize_or_zero().into(), edge: NO_EDGES, uv: self.uvs[i] }
            })
            .collect()
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(count).filter(|end| *end <= self.bytes.len()).ok_or("The mesh file ends too soon.")?;
        let bytes = &self.bytes[self.at..end];
        self.at = end;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        Ok(self.take(N)?.try_into().expect("took exactly N bytes"))
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    fn vec3(&mut self) -> Result<Vec3, String> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }

    fn count(&mut self) -> Result<usize, String> {
        let count = self.u32()? as usize;
        if count > MOST_ELEMENTS {
            return Err("The mesh file says it holds more than it could.".to_string());
        }
        Ok(count)
    }

    fn done(&self) -> bool {
        self.at >= self.bytes.len()
    }
}

#[derive(Default)]
struct Geometry {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    uvs: Vec<[f32; 2]>,
    faces: Vec<[u32; 3]>,
}

impl Geometry {
    fn read_vertices(&mut self, r: &mut Reader, count: usize, size: usize) -> Result<(), String> {
        if size < 36 {
            return Err("The mesh file's vertices are too small to read.".to_string());
        }
        self.positions.reserve(count);
        self.normals.reserve(count);
        self.uvs.reserve(count);
        for _ in 0..count {
            let mut vertex = Reader::new(r.take(size)?);
            self.positions.push(vertex.vec3()?);
            self.normals.push(vertex.vec3()?);
            self.uvs.push([vertex.f32()?, vertex.f32()?]);
        }
        Ok(())
    }

    fn read_faces(&mut self, r: &mut Reader, count: usize, size: usize) -> Result<(), String> {
        if size < 12 {
            return Err("The mesh file's faces are too small to read.".to_string());
        }
        self.faces.reserve(count);
        for _ in 0..count {
            let mut face = Reader::new(r.take(size)?);
            self.faces.push([face.u32()?, face.u32()?, face.u32()?]);
        }
        Ok(())
    }

    /// Keeps the first level of detail. All levels share one list of faces, and the
    /// offsets say where each one starts.
    fn finish(self, lods: &[u32], bones: Vec<String>, weights: Vec<Weights>) -> Result<RobloxMesh, String> {
        // Real version 7 files leave the table as zeros, which means all the faces.
        let listed = lods.len() >= 2 && lods[1] > lods[0];
        let end = if listed { (lods[1] as usize).min(self.faces.len()) } else { self.faces.len() };
        let start = if listed { (lods[0] as usize).min(end) } else { 0 };
        let vertices = self.positions.len() as u32;
        let indices: Vec<u32> = self.faces[start..end].iter().flatten().copied().collect();
        if indices.iter().any(|i| *i >= vertices) {
            return Err("A face in the mesh file points at a vertex that isn't there.".to_string());
        }
        Ok(RobloxMesh { positions: self.positions, normals: self.normals, uvs: self.uvs, indices, bones, weights })
    }
}

/// Bone names, each picked out by where it starts in one block of text.
fn bone_names(starts: &[u32], names: &[u8]) -> Vec<String> {
    starts
        .iter()
        .map(|start| {
            let rest = names.get(*start as usize..).unwrap_or(&[]);
            let end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
            String::from_utf8_lossy(&rest[..end]).into_owned()
        })
        .collect()
}

fn read_bones(r: &mut Reader, count: usize) -> Result<Vec<u32>, String> {
    (0..count)
        .map(|_| {
            let mut bone = Reader::new(r.take(BONE_SIZE)?);
            bone.u32()
        })
        .collect()
}

struct Subset {
    first: usize,
    count: usize,
    bones: [u16; SUBSET_BONES],
}

fn read_subsets(r: &mut Reader, count: usize) -> Result<Vec<Subset>, String> {
    (0..count)
        .map(|_| {
            let mut subset = Reader::new(r.take(SUBSET_SIZE)?);
            let (_faces_begin, _faces_length) = (subset.u32()?, subset.u32()?);
            let (first, count) = (subset.u32()? as usize, subset.u32()? as usize);
            let _bone_count = subset.u32()?;
            let mut bones = [0; SUBSET_BONES];
            for bone in &mut bones {
                *bone = subset.u16()?;
            }
            Ok(Subset { first, count, bones })
        })
        .collect()
}

/// A vertex names its bones by their place in its subset, so that's turned into which
/// bone of the whole mesh they are.
fn resolve_weights(raw: &[([u8; 4], [u8; 4])], subsets: &[Subset]) -> Vec<Weights> {
    let mut weights: Vec<Weights> = raw.iter().map(|(_, amounts)| Weights { bones: [u16::MAX; 4], amounts: *amounts }).collect();
    for subset in subsets {
        let end = subset.first.saturating_add(subset.count).min(raw.len());
        for vertex in subset.first.min(end)..end {
            for slot in 0..4 {
                if let Some(bone) = subset.bones.get(raw[vertex].0[slot] as usize) {
                    weights[vertex].bones[slot] = *bone;
                }
            }
        }
    }
    weights
}

fn read_skinning(r: &mut Reader, count: usize) -> Result<Vec<([u8; 4], [u8; 4])>, String> {
    (0..count)
        .map(|_| {
            let bytes = r.take(SKINNING_SIZE)?;
            Ok(([bytes[0], bytes[1], bytes[2], bytes[3]], [bytes[4], bytes[5], bytes[6], bytes[7]]))
        })
        .collect()
}

fn read_lods(r: &mut Reader, count: usize) -> Result<Vec<u32>, String> {
    (0..count).map(|_| r.u32()).collect()
}

/// Text: one line of faces, each as nine bracketed triples.
fn read_text(body: &str, half: bool) -> Result<RobloxMesh, String> {
    let (count, numbers) = body.split_once('\n').ok_or("The mesh file ends too soon.")?;
    let count: usize = count.trim().parse().map_err(|_| "The mesh file's face count isn't a number.".to_string())?;
    let values: Vec<f32> = numbers.split(['[', ']', ',']).filter(|t| !t.trim().is_empty()).filter_map(|t| t.trim().parse().ok()).collect();
    if count > MOST_ELEMENTS || values.len() < count * 27 {
        return Err("The mesh file ends too soon.".to_string());
    }
    let scale = if half { 0.5 } else { 1.0 };
    let mut mesh = RobloxMesh::default();
    for vertex in values.chunks_exact(9).take(count * 3) {
        mesh.positions.push(Vec3::new(vertex[0], vertex[1], vertex[2]) * scale);
        mesh.normals.push(Vec3::new(vertex[3], vertex[4], vertex[5]));
        mesh.uvs.push([vertex[6], 1.0 - vertex[7]]);
    }
    mesh.indices = (0..mesh.positions.len() as u32).collect();
    Ok(mesh)
}

/// Versions 2 and 3.
fn read_simple(bytes: &[u8], with_lods: bool) -> Result<RobloxMesh, String> {
    let mut r = Reader::new(bytes);
    let header = r.u16()? as usize;
    let (vertex_size, face_size) = (r.u8()? as usize, r.u8()? as usize);
    let (lod_size, lod_count) = if with_lods { (r.u16()? as usize, r.u16()? as usize) } else { (4, 0) };
    let (vertices, faces) = (r.count()?, r.count()?);
    r.at = header;
    let mut geometry = Geometry::default();
    geometry.read_vertices(&mut r, vertices, vertex_size)?;
    geometry.read_faces(&mut r, faces, face_size)?;
    let lods = if lod_size == 4 { read_lods(&mut r, lod_count)? } else { Vec::new() };
    geometry.finish(&lods, Vec::new(), Vec::new())
}

/// Versions 4 and 5, whose header only differs in what comes after everything here.
fn read_skinned(bytes: &[u8]) -> Result<RobloxMesh, String> {
    let mut r = Reader::new(bytes);
    let header = r.u16()? as usize;
    let _lod_type = r.u16()?;
    let (vertices, faces) = (r.count()?, r.count()?);
    let (lod_count, bone_count) = (r.u16()? as usize, r.u16()? as usize);
    let name_bytes = r.count()?;
    let subset_count = r.u16()? as usize;
    r.at = header;
    let mut geometry = Geometry::default();
    geometry.read_vertices(&mut r, vertices, 40)?;
    let raw = if bone_count > 0 { read_skinning(&mut r, vertices)? } else { Vec::new() };
    geometry.read_faces(&mut r, faces, 12)?;
    let lods = read_lods(&mut r, lod_count)?;
    let starts = read_bones(&mut r, bone_count)?;
    let names = r.take(name_bytes)?;
    let subsets = read_subsets(&mut r, subset_count)?;
    geometry.finish(&lods, bone_names(&starts, names), resolve_weights(&raw, &subsets))
}

/// Version 7's compressed geometry.
fn read_draco(bytes: &[u8]) -> Result<Geometry, String> {
    let mut buffer = draco_core::DecoderBuffer::new(bytes);
    let mut mesh = draco_core::Mesh::new();
    draco_core::MeshDecoder::new().decode(&mut buffer, &mut mesh).map_err(|e| format!("The mesh's compressed data couldn't be read: {e:?}"))?;
    let points = mesh.num_points();
    let channel = |id: u32, components: usize| -> Result<Vec<f32>, String> {
        let attribute = mesh.attribute_by_unique_id(id).or_else(|| mesh.try_attribute(id as i32).ok()).ok_or("The mesh's compressed data is missing something.")?;
        Ok(attribute.read_f32s(points, components))
    };
    let (positions, normals, uvs) = (channel(0, 3)?, channel(1, 3)?, channel(2, 2)?);
    let triples = |values: Vec<f32>| values.chunks_exact(3).map(Vec3::from_slice).collect::<Vec<Vec3>>();
    let faces = mesh.faces().iter().map(|face| face.map(|point| point.0)).collect();
    Ok(Geometry { positions: triples(positions), normals: triples(normals), uvs: uvs.chunks_exact(2).map(|uv| [uv[0], uv[1]]).collect(), faces })
}

/// Versions 6 and 7: named chunks, read until the file runs out.
fn read_chunks(bytes: &[u8]) -> Result<RobloxMesh, String> {
    let mut r = Reader::new(bytes);
    let mut geometry = Geometry::default();
    let (mut lods, mut raw, mut starts, mut names, mut subsets) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    while !r.done() {
        let name = r.array::<8>()?;
        let version = r.u32()?;
        let size = r.count()?;
        let mut chunk = Reader::new(r.take(size)?);
        match (&name, version) {
            (b"COREMESH", 1) => {
                let vertices = chunk.count()?;
                geometry.read_vertices(&mut chunk, vertices, 40)?;
                let faces = chunk.count()?;
                geometry.read_faces(&mut chunk, faces, 12)?;
            }
            (b"COREMESH", 2) => {
                let size = chunk.count()?;
                geometry = read_draco(chunk.take(size)?)?;
            }
            (b"LODS\0\0\0\0", 1) => {
                let (_lod_type, _high) = (chunk.u16()?, chunk.u8()?);
                let count = chunk.count()?;
                lods = read_lods(&mut chunk, count)?;
            }
            (b"SKINNING", 1) => {
                let count = chunk.count()?;
                raw = read_skinning(&mut chunk, count)?;
                let bones = chunk.count()?;
                starts = read_bones(&mut chunk, bones)?;
                let size = chunk.count()?;
                names = chunk.take(size)?.to_vec();
                let count = chunk.count()?;
                subsets = read_subsets(&mut chunk, count)?;
            }
            _ => {}
        }
    }
    if geometry.positions.is_empty() {
        return Err("The mesh file has no geometry in it.".to_string());
    }
    geometry.finish(&lods, bone_names(&starts, &names), resolve_weights(&raw, &subsets))
}

pub fn parse(bytes: &[u8]) -> Result<RobloxMesh, String> {
    let line = bytes.iter().take(32).position(|b| *b == b'\n').ok_or("This isn't a Roblox mesh file.")?;
    let header = std::str::from_utf8(&bytes[..line]).map_err(|_| "This isn't a Roblox mesh file.".to_string())?;
    let version = header.trim().strip_prefix("version ").ok_or("This isn't a Roblox mesh file.")?;
    let body = &bytes[line + 1..];
    match version.split('.').next() {
        Some("1") => read_text(&String::from_utf8_lossy(body), version == "1.00"),
        Some("2") => read_simple(body, false),
        Some("3") => read_simple(body, true),
        Some("4" | "5") => read_skinned(body),
        Some("6" | "7") => read_chunks(body),
        _ => Err(format!("Mesh version {version} isn't one Kaye can read yet.")),
    }
}

/// The asset id in any of the ways Roblox writes one, such as `rbxassetid://123` and
/// `http://www.roblox.com/asset/?id=123`. Files that ship with Roblox have none.
pub fn asset_id(text: &str) -> Option<u64> {
    if text.starts_with("rbxasset://") {
        return None;
    }
    let digits = text.rsplit(|c: char| !c.is_ascii_digit()).next()?;
    digits.parse().ok().filter(|id| *id > 0)
}
