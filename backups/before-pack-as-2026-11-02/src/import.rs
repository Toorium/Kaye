//! Turns a Roblox model file into a [`Scene`].
//!
//! The format is detected from the file's contents: binary `.rbxm` starts with
//! `<roblox!`, XML `.rbxmx` with `<roblox`. Extensions are not trusted, so a file
//! named `.rblxm` opens the same way.

use std::collections::{HashMap, VecDeque};
use std::path::Path;

use glam::Vec3;
use rbx_dom_weak::{Instance, WeakDom};
use rbx_types::{Ref, Variant};
use ustr::ustr;

use crate::math::Xf;
use crate::scene::*;

#[derive(Debug)]
pub struct Imported {
    pub scene: Scene,
    /// Things the user should know were skipped or not understood.
    pub warnings: Vec<String>,
}

pub fn import_file(path: &Path) -> Result<Imported, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Could not read the file: {e}"))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "model".into());
    import_bytes(&bytes, &name)
}

pub fn import_bytes(bytes: &[u8], file_name: &str) -> Result<Imported, String> {
    let dom = if bytes.starts_with(b"<roblox!") {
        rbx_binary::from_reader(bytes).map_err(|e| format!("Not a valid .rbxm file: {e}"))?
    } else if bytes.starts_with(b"<roblox") {
        rbx_xml::from_reader_default(bytes).map_err(|e| format!("Not a valid .rbxmx file: {e}"))?
    } else {
        return Err("This is not a Roblox model file. Export a .rbxm or .rbxmx from Studio.".into());
    };
    import_dom(&dom, file_name)
}

pub fn import_dom(dom: &WeakDom, file_name: &str) -> Result<Imported, String> {
    let mut groups = Vec::new();
    let mut loose = Vec::new();
    for child in dom.root().children() {
        collect_groups(dom, *child, &mut groups, &mut loose);
    }
    let stem = file_name.rsplit_once('.').map_or(file_name, |(s, _)| s);
    if !loose.is_empty() {
        groups.push(Group {
            name: stem.to_string(),
            roots: loose,
        });
    }

    let mut out = Imported {
        scene: Scene::default(),
        warnings: Vec::new(),
    };
    for group in groups {
        let (scene, mut warnings) = build_rig(dom, &group, file_name);
        out.scene.append(scene);
        out.warnings.append(&mut warnings);
    }
    if out.scene.parts.is_empty() {
        return Err("The file opened, but it has no parts or rigs in it.".into());
    }
    out.scene.evaluate();
    out.scene.compute_tails();
    Ok(out)
}

struct Group {
    name: String,
    roots: Vec<Ref>,
}

fn is_part(inst: &Instance) -> bool {
    inst.class != "Terrain" && cframe_prop(inst, "CFrame").is_some() && vec3_prop(inst, "Size").is_some()
}

/// A group is the outermost container that directly holds parts or a Humanoid.
/// Containers that only hold other models are walked so each rig stays separate.
fn collect_groups(dom: &WeakDom, r: Ref, groups: &mut Vec<Group>, loose: &mut Vec<Ref>) {
    let Some(inst) = dom.get_by_ref(r) else {
        return;
    };
    if is_part(inst) {
        loose.push(r);
        return;
    }
    let holds_rig = inst.children().iter().any(|c| {
        dom.get_by_ref(*c)
            .is_some_and(|c| is_part(c) || c.class == "Humanoid")
    });
    if holds_rig {
        groups.push(Group {
            name: inst.name.clone(),
            roots: vec![r],
        });
        return;
    }
    for child in inst.children() {
        collect_groups(dom, *child, groups, loose);
    }
}

fn descendants(dom: &WeakDom, roots: &[Ref]) -> Vec<Ref> {
    let mut out = Vec::new();
    let mut stack: Vec<Ref> = roots.iter().rev().copied().collect();
    while let Some(r) = stack.pop() {
        if let Some(inst) = dom.get_by_ref(r) {
            out.push(r);
            stack.extend(inst.children().iter().rev().copied());
        }
    }
    out
}

fn prop<'a>(inst: &'a Instance, name: &str) -> Option<&'a Variant> {
    inst.properties.get(&ustr(name))
}

fn cframe_prop(inst: &Instance, name: &str) -> Option<Xf> {
    match prop(inst, name)? {
        Variant::CFrame(c) => Some(Xf::from_cframe(c)),
        _ => None,
    }
}

fn vec3_prop(inst: &Instance, name: &str) -> Option<Vec3> {
    match prop(inst, name)? {
        Variant::Vector3(v) => Some(Vec3::new(v.x, v.y, v.z)),
        _ => None,
    }
}

fn ref_prop(inst: &Instance, name: &str) -> Option<Ref> {
    match prop(inst, name)? {
        Variant::Ref(r) if r.is_some() => Some(*r),
        _ => None,
    }
}

fn float_prop(inst: &Instance, name: &str) -> Option<f32> {
    match prop(inst, name)? {
        Variant::Float32(v) => Some(*v),
        Variant::Float64(v) => Some(*v as f32),
        _ => None,
    }
}

fn enum_prop(inst: &Instance, name: &str) -> Option<u32> {
    match prop(inst, name)? {
        Variant::Enum(e) => Some(e.to_u32()),
        _ => None,
    }
}

fn number_prop(inst: &Instance, name: &str) -> Option<i64> {
    match prop(inst, name)? {
        Variant::Int64(v) => Some(*v),
        Variant::Int32(v) => Some(*v as i64),
        _ => None,
    }
}

fn color_prop(inst: &Instance) -> [f32; 3] {
    let as_rgb = |v: &Variant| match v {
        Variant::Color3uint8(c) => Some([c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0]),
        Variant::Color3(c) => Some([c.r, c.g, c.b]),
        _ => None,
    };
    ["Color", "Color3uint8"]
        .iter()
        .find_map(|n| prop(inst, n).and_then(as_rgb))
        .unwrap_or([0.64, 0.64, 0.65])
}

/// Newer files store `MeshContent`; older ones store `MeshId` (and the texture
/// equivalents). Returns the first non-empty asset reference.
fn content_prop(inst: &Instance, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        let text = match prop(inst, name)? {
            Variant::Content(c) => c.as_uri()?.to_string(),
            Variant::ContentId(c) => c.as_str().to_string(),
            Variant::String(s) => s.clone(),
            _ => return None,
        };
        (!text.is_empty()).then_some(text)
    })
}

const MESH_PROPS: &[&str] = &["MeshContent", "MeshId"];
const TEXTURE_PROPS: &[&str] = &["TextureContent", "TextureID", "TextureId"];
/// PBR color from a SurfaceAppearance, which replaces a mesh's plain texture.
const COLOR_MAP_PROPS: &[&str] = &["ColorMapContent", "ColorMap"];
const DECAL_PROPS: &[&str] = &["TextureContent", "Texture"];
/// `Enum.NormalId.Front`, the face of a part a Decal usually sits on.
const FRONT_FACE: u32 = 5;
/// `Enum.MeshType.FileMesh`. Every other type is a shape Roblox draws itself.
const FILE_MESH: u32 = 5;
const CHARACTER_MESH_PARTS: [&str; 6] = ["Head", "Torso", "Left Arm", "Right Arm", "Left Leg", "Right Leg"];

/// The shape a SpecialMesh asks for, if it isn't a file. Ramps and prisms have no
/// shape of their own here, so they stay blocks.
fn built_shape(mesh: &Instance) -> Option<BuiltShape> {
    if mesh.class != "SpecialMesh" {
        return None;
    }
    match enum_prop(mesh, "MeshType")? {
        FILE_MESH => None,
        0 => Some(BuiltShape::Head),
        2 => Some(BuiltShape::Wedge),
        3 => Some(BuiltShape::Sphere),
        4 => Some(BuiltShape::Cylinder),
        11 => Some(BuiltShape::CornerWedge),
        _ => Some(BuiltShape::Block),
    }
}

fn part_shape(inst: &Instance) -> PartShape {
    match inst.class.as_str() {
        "WedgePart" => return PartShape::Wedge,
        "CornerWedgePart" => return PartShape::CornerWedge,
        _ => {}
    }
    match enum_prop(inst, "Shape") {
        Some(0) => PartShape::Ball,
        Some(2) => PartShape::Cylinder,
        Some(3) => PartShape::Wedge,
        Some(4) => PartShape::CornerWedge,
        _ => PartShape::Block,
    }
}

enum JointClass {
    Bone(BoneKind),
    Rigid,
}

fn joint_class(class: &str) -> Option<JointClass> {
    Some(match class {
        "Motor6D" => JointClass::Bone(BoneKind::Motor6D),
        "Motor" => JointClass::Bone(BoneKind::LegacyMotor),
        "AnimationConstraint" => JointClass::Bone(BoneKind::AnimationConstraint),
        "Weld" | "ManualWeld" | "ManualGlue" | "Snap" | "Glue" | "Rotate" | "RotateP" | "RotateV"
        | "WeldConstraint" | "RigidConstraint" => JointClass::Rigid,
        _ => return None,
    })
}

struct JointInfo {
    name: String,
    kind: Option<BoneKind>,
    part0: PartId,
    part1: PartId,
    c0: Xf,
    c1: Xf,
}

fn build_rig(dom: &WeakDom, group: &Group, file_name: &str) -> (Scene, Vec<String>) {
    let mut warnings = Vec::new();
    let refs = descendants(dom, &group.roots);
    let mut part_ids: HashMap<Ref, PartId> = HashMap::new();
    let mut parts: Vec<Part> = Vec::new();

    for r in &refs {
        let inst = dom.get_by_ref(*r).unwrap();
        if !is_part(inst) {
            continue;
        }
        let mut mesh_asset = content_prop(inst, MESH_PROPS);
        let mut texture_asset = content_prop(inst, TEXTURE_PROPS);
        let mut mesh_fit = MeshFit::Size;
        for child in inst.children() {
            let Some(child) = dom.get_by_ref(*child) else {
                continue;
            };
            if matches!(child.class.as_str(), "SpecialMesh" | "FileMesh") {
                if mesh_asset.is_none() {
                    let scale = vec3_prop(child, "Scale").unwrap_or(Vec3::ONE);
                    let offset = vec3_prop(child, "Offset").unwrap_or(Vec3::ZERO);
                    mesh_fit = match built_shape(child) {
                        Some(shape) => MeshFit::Built { shape, scale, offset },
                        None => MeshFit::Scaled { scale, offset },
                    };
                    // Roblox ignores a leftover file on a mesh set to one of its own shapes.
                    if built_shape(child).is_none() {
                        mesh_asset = content_prop(child, MESH_PROPS);
                    }
                }
                texture_asset = texture_asset.or_else(|| content_prop(child, TEXTURE_PROPS));
            }
            if child.class == "SurfaceAppearance" {
                texture_asset = content_prop(child, COLOR_MAP_PROPS).or(texture_asset);
            }
        }
        if matches!(mesh_fit, MeshFit::Built { shape: BuiltShape::Head, .. }) && texture_asset.is_none() {
            texture_asset = face_on(dom, inst);
        }
        let file_cframe = cframe_prop(inst, "CFrame").unwrap();
        part_ids.insert(*r, parts.len());
        parts.push(Part {
            name: inst.name.clone(),
            class: inst.class.to_string(),
            rig: 0,
            size: vec3_prop(inst, "Size").unwrap(),
            color: color_prop(inst),
            transparency: float_prop(inst, "Transparency").unwrap_or(0.0),
            shape: part_shape(inst),
            mesh_asset,
            texture_asset,
            mesh_fit,
            file_cframe,
            world: file_cframe,
            drive: Drive::Root,
            visible: true,
        });
    }

    wear_character_meshes(dom, &refs, &mut parts);

    let mut joints: Vec<JointInfo> = Vec::new();
    for r in &refs {
        let inst = dom.get_by_ref(*r).unwrap();
        let Some(class) = joint_class(inst.class.as_str()) else {
            continue;
        };
        let (a, b, c0, c1) = if inst.class == "AnimationConstraint" {
            match attachment_link(dom, inst, &part_ids) {
                Some(link) => link,
                None => continue,
            }
        } else {
            let a = ref_prop(inst, "Part0").and_then(|x| part_ids.get(&x).copied());
            let b = ref_prop(inst, "Part1").and_then(|x| part_ids.get(&x).copied());
            let (Some(a), Some(b)) = (a, b) else {
                if matches!(class, JointClass::Bone(_)) {
                    warnings.push(format!(
                        "{}: joint \"{}\" is missing a part and was skipped.",
                        group.name, inst.name
                    ));
                }
                continue;
            };
            if inst.class == "WeldConstraint" || inst.class == "RigidConstraint" {
                // These store no offsets; the parts' saved positions define them.
                let rel = parts[a].file_cframe.inverse() * parts[b].file_cframe;
                (a, b, rel, Xf::IDENTITY)
            } else {
                (
                    a,
                    b,
                    cframe_prop(inst, "C0").unwrap_or(Xf::IDENTITY),
                    cframe_prop(inst, "C1").unwrap_or(Xf::IDENTITY),
                )
            }
        };
        if a == b {
            continue;
        }
        joints.push(JointInfo {
            name: inst.name.clone(),
            kind: match class {
                JointClass::Bone(k) => Some(k),
                JointClass::Rigid => None,
            },
            part0: a,
            part1: b,
            c0,
            c1,
        });
    }

    let primary = group
        .roots
        .first()
        .and_then(|r| dom.get_by_ref(*r))
        .and_then(|m| ref_prop(m, "PrimaryPart"))
        .and_then(|r| part_ids.get(&r).copied());

    let mut scene = Scene {
        parts,
        ..Scene::default()
    };
    solve_hierarchy(&mut scene, &joints, primary, &group.name, &mut warnings);
    add_skinned_bones(dom, &refs, &part_ids, &mut scene);

    for b in 0..scene.bones.len() {
        if let Some(p) = scene.bones[b].parent {
            scene.bones[p].children.push(b);
        }
    }

    let kind = detect_kind(dom, &refs, &scene);
    let clothing = |class: &str, props: &[&str]| refs.iter().filter_map(|r| dom.get_by_ref(*r)).find(|i| i.class == class).and_then(|i| content_prop(i, props));
    let (shirt, pants) = (clothing("Shirt", &["ShirtTemplateContent", "ShirtTemplate"]), clothing("Pants", &["PantsTemplateContent", "PantsTemplate"]));
    scene.rigs.push(Rig {
        name: group.name.clone(),
        kind,
        source_file: file_name.to_string(),
        parts: (0..scene.parts.len()).collect(),
        bones: (0..scene.bones.len()).collect(),
        visible: true,
        shirt,
        pants,
    });
    (scene, warnings)
}

/// The picture on a part's front, for a head with no texture of its own.
fn face_on(dom: &WeakDom, part: &Instance) -> Option<String> {
    let decals = part.children().iter().filter_map(|c| dom.get_by_ref(*c)).filter(|c| c.class == "Decal");
    let mut on_front = decals.filter(|d| enum_prop(d, "Face").is_none_or(|face| face == FRONT_FACE));
    on_front.find_map(|d| content_prop(d, DECAL_PROPS))
}

/// R6 packages swap a body part's shape with a CharacterMesh, which names its assets by
/// number. A mesh set on the part itself wins.
fn wear_character_meshes(dom: &WeakDom, refs: &[Ref], parts: &mut [Part]) {
    for inst in refs.iter().filter_map(|r| dom.get_by_ref(*r)).filter(|i| i.class == "CharacterMesh") {
        let Some(name) = enum_prop(inst, "BodyPart").and_then(|i| CHARACTER_MESH_PARTS.get(i as usize)) else {
            continue;
        };
        let Some(mesh) = number_prop(inst, "MeshId").filter(|id| *id > 0) else {
            continue;
        };
        let Some(part) = parts.iter_mut().find(|p| p.name == *name && p.mesh_asset.is_none()) else {
            continue;
        };
        part.mesh_asset = Some(format!("rbxassetid://{mesh}"));
        part.mesh_fit = MeshFit::Scaled { scale: Vec3::ONE, offset: Vec3::ZERO };
        if part.texture_asset.is_none() {
            part.texture_asset = number_prop(inst, "BaseTextureId").filter(|id| *id > 0).map(|id| format!("rbxassetid://{id}"));
        }
    }
}

fn attachment_link(
    dom: &WeakDom,
    inst: &Instance,
    part_ids: &HashMap<Ref, PartId>,
) -> Option<(PartId, PartId, Xf, Xf)> {
    let side = |name: &str| -> Option<(PartId, Xf)> {
        let att = dom.get_by_ref(ref_prop(inst, name)?)?;
        let part = *part_ids.get(&att.parent())?;
        Some((part, cframe_prop(att, "CFrame").unwrap_or(Xf::IDENTITY)))
    };
    let (a, c0) = side("Attachment0")?;
    let (b, c1) = side("Attachment1")?;
    Some((a, b, c0, c1))
}

/// Decides how each part gets positioned. Animated joints always run Part0 to
/// Part1, because that is the direction `Transform` is defined in. Rigid joints
/// can be walked either way, since accessories weld Handle to body part.
fn solve_hierarchy(
    scene: &mut Scene,
    joints: &[JointInfo],
    primary: Option<PartId>,
    rig_name: &str,
    warnings: &mut Vec<String>,
) {
    let n = scene.parts.len();
    let mut driven_by: Vec<Option<usize>> = vec![None; n];
    for (i, j) in joints.iter().enumerate() {
        if j.kind.is_none() {
            continue;
        }
        if driven_by[j.part1].is_some() {
            warnings.push(format!(
                "{rig_name}: \"{}\" is driven by more than one joint, so \"{}\" was skipped.",
                scene.parts[j.part1].name, j.name
            ));
        } else {
            driven_by[j.part1] = Some(i);
        }
    }
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut rigid: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, j) in joints.iter().enumerate() {
        if j.kind.is_some() {
            if driven_by[j.part1] == Some(i) {
                children[j.part0].push(i);
            }
        } else {
            rigid[j.part0].push(i);
            rigid[j.part1].push(i);
        }
    }

    // Start from the real root so a welded accessory is never treated as the
    // root of the character it is attached to.
    let mut candidates: Vec<PartId> = Vec::new();
    candidates.extend((0..n).filter(|p| scene.parts[*p].name == "HumanoidRootPart"));
    candidates.extend(primary);
    candidates.extend((0..n).filter(|p| driven_by[*p].is_none() && !children[*p].is_empty()));
    candidates.extend((0..n).filter(|p| driven_by[*p].is_none()));
    // Anything left is in a joint cycle.
    candidates.extend(0..n);

    let mut placed = vec![false; n];
    for root in candidates {
        if placed[root] {
            continue;
        }
        placed[root] = true;
        scene.parts[root].drive = Drive::Root;
        scene.part_order.push(root);
        let mut queue = VecDeque::from([root]);
        while let Some(p) = queue.pop_front() {
            for &i in &children[p] {
                let j = &joints[i];
                if placed[j.part1] {
                    warnings.push(format!(
                        "{rig_name}: joint \"{}\" closes a loop and was skipped.",
                        j.name
                    ));
                    continue;
                }
                placed[j.part1] = true;
                let parent = scene.parts[j.part0]
                    .drive_bone()
                    .or_else(|| owner_of(scene, j.part0));
                let id = scene.bones.len();
                scene.bones.push(Bone::new(
                    j.name.clone(),
                    j.kind.unwrap(),
                    BoneLink::Joint {
                        part0: j.part0,
                        part1: j.part1,
                        c0: j.c0,
                        c1: j.c1,
                    },
                    parent,
                ));
                scene.parts[j.part1].drive = Drive::Bone(id);
                scene.part_order.push(j.part1);
                queue.push_back(j.part1);
            }
            for &i in &rigid[p] {
                let j = &joints[i];
                let forward = j.c0 * j.c1.inverse();
                let (other, offset) = if j.part0 == p {
                    (j.part1, forward)
                } else {
                    (j.part0, forward.inverse())
                };
                if placed[other] {
                    continue;
                }
                placed[other] = true;
                scene.parts[other].drive = Drive::Rigid { parent: p, offset };
                scene.part_order.push(other);
                queue.push_back(other);
            }
        }
    }
}

impl Part {
    pub(crate) fn drive_bone(&self) -> Option<BoneId> {
        match self.drive {
            Drive::Bone(b) => Some(b),
            _ => None,
        }
    }
}

fn owner_of(scene: &Scene, part: PartId) -> Option<BoneId> {
    scene.part_owner(part)
}

fn add_skinned_bones(
    dom: &WeakDom,
    refs: &[Ref],
    part_ids: &HashMap<Ref, PartId>,
    scene: &mut Scene,
) {
    // `refs` is depth-first, so a parent Bone is always registered first.
    let mut bone_ids: HashMap<Ref, BoneId> = HashMap::new();
    for r in refs {
        let inst = dom.get_by_ref(*r).unwrap();
        if inst.class != "Bone" {
            continue;
        }
        let parent_ref = inst.parent();
        let parent_bone = bone_ids.get(&parent_ref).copied();
        let parent_part = part_ids.get(&parent_ref).copied();
        let parent = parent_bone.or_else(|| parent_part.and_then(|p| scene.part_owner(p)));
        let id = scene.bones.len();
        scene.bones.push(Bone::new(
            inst.name.clone(),
            BoneKind::Skinned,
            BoneLink::Skinned {
                parent_part,
                parent_bone,
                local: cframe_prop(inst, "CFrame").unwrap_or(Xf::IDENTITY),
            },
            parent,
        ));
        scene.skinned_order.push(id);
        bone_ids.insert(*r, id);
    }
}

fn detect_kind(dom: &WeakDom, refs: &[Ref], scene: &Scene) -> RigKind {
    if scene.bones.is_empty() {
        return RigKind::Static;
    }
    let has_part = |name: &str| scene.parts.iter().any(|p| p.name == name);
    let humanoid = refs
        .iter()
        .filter_map(|r| dom.get_by_ref(*r))
        .find(|i| i.class == "Humanoid");
    let rig_type = humanoid.and_then(|h| enum_prop(h, "RigType"));
    let is_r15 = rig_type == Some(1) || (humanoid.is_some() && has_part("UpperTorso"));
    if is_r15 {
        let rthro = humanoid.is_some_and(|h| {
            h.children().iter().filter_map(|c| dom.get_by_ref(*c)).any(|c| {
                c.name == "BodyTypeScale" && float_prop(c, "Value").is_some_and(|v| v > 0.01)
            })
        });
        return if rthro { RigKind::Rthro } else { RigKind::R15 };
    }
    if rig_type == Some(0) && has_part("Torso") {
        return RigKind::R6;
    }
    if scene.bones.iter().any(|b| b.kind == BoneKind::Skinned) {
        return RigKind::Skinned;
    }
    RigKind::Custom
}
