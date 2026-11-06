//! Kaye project files: the rigs, keys, modifiers and cameras, plus where the view was
//! left. Like the rig logic, this has no interface code.
//!
//! A project stores the scene itself, so it opens with no other file around. Each rig's
//! original model comes along inside it, for a model export to write back out. Anything `Scene::evaluate` can work out is left out and worked
//! out again on load. The undo history isn't kept.

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::animation::{Animation, Easing};
use crate::math::Xf;
use crate::modifiers::{Modifier, ModifierKind};
use crate::scene::{BoneId, BoneLink, BuiltShape, Drive, MeshFit, Scene};

pub const EXTENSION: &str = "kaye";
/// 2 added a list of animations for each rig, and dropped layers.
const VERSION: u32 = 2;

/// Where the view was left, which isn't part of the work itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub frame: f32,
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub ortho: bool,
    pub floor_y: f32,
    pub camera: Option<BoneId>,
    pub preview: bool,
    pub through: bool,
    /// The bones showing a motion path.
    #[serde(default)]
    pub motion_paths: Vec<BoneId>,
    /// Where each reference's window was left.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<crate::reference::WindowPlace>,
}

#[derive(Serialize)]
struct Outgoing<'a> {
    kaye_project: u32,
    scene: &'a Scene,
    animation: &'a Animation,
    view: &'a View,
}

#[derive(Deserialize)]
struct Header {
    kaye_project: Option<u32>,
}

/// A project that has been read and checked.
#[derive(Debug, Deserialize)]
pub struct Project {
    pub scene: Scene,
    pub animation: Animation,
    pub view: View,
}

pub fn is_project(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// The path with `.kaye` on the end, for a name typed without it.
pub fn with_extension(path: PathBuf) -> PathBuf {
    if is_project(&path) { path } else { path.with_added_extension(EXTENSION) }
}

/// Writes a project. The old file is only replaced once the new one is known to open,
/// so a failed save can't cost anything that was already saved.
pub fn save(path: &Path, scene: &Scene, animation: &Animation, view: &View) -> Result<(), String> {
    let bytes = serde_json::to_vec(&Outgoing { kaye_project: VERSION, scene, animation, view }).map_err(|e| e.to_string())?;
    parse(&bytes)?;
    let temporary = path.with_added_extension("tmp");
    let written = write_through(&temporary, &bytes).and_then(|()| std::fs::rename(&temporary, path));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    Ok(())
}

fn write_through(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

pub fn load(path: &Path) -> Result<Project, String> {
    parse(&std::fs::read(path).map_err(|e| e.to_string())?)
}

fn parse(bytes: &[u8]) -> Result<Project, String> {
    let header: Header = serde_json::from_slice(bytes).map_err(|_| "This isn't a Kaye project.".to_string())?;
    match header.kaye_project {
        None => return Err("This isn't a Kaye project.".to_string()),
        Some(version) if version > VERSION => return Err("This project was saved by a newer version of Kaye.".to_string()),
        Some(_) => {}
    }
    let mut project = read_project(bytes).map_err(|e| format!("The project is damaged: {e}"))?;
    if project.animation.active_layer >= project.animation.layers.len().max(1) {
        return Err("The project is damaged: no such layer".to_string());
    }
    project.animation.flatten_layers();
    check(&project).map_err(|what| format!("The project is damaged: {what}"))?;
    project.animation.ensure_clips(&project.scene);
    upgrade_heads(&mut project.scene);
    upgrade_joints(&mut project.scene);
    project.scene.evaluate();
    Ok(project)
}

/// Projects saved before Kaye read every kind of joint can hold parts left standing on their
/// own, such as hats held by a RigidConstraint, and know nothing of SurfaceAppearances. Each
/// rig's original model is kept in the project, so it's read again: a part that was left
/// standing is attached as it is now, and the SurfaceAppearance mark is copied. Nothing else
/// changes. Parts are matched in order and by name, so a rig that doesn't line up is left
/// alone. Only rigs that could be affected are read, since reading costs time on every open.
fn upgrade_joints(scene: &mut Scene) {
    let mut attached = false;
    for r in 0..scene.rigs.len() {
        let Some(model) = scene.rigs[r].model.clone() else { continue };
        let ours = scene.rigs[r].parts.clone();
        let roots = ours.iter().filter(|p| matches!(scene.parts[**p].drive, Drive::Root)).count();
        let dressed = scene.rigs[r].shirt.is_some() || scene.rigs[r].pants.is_some();
        if roots < 2 && !(dressed && ours.iter().any(|p| scene.parts[*p].texture_asset.is_some())) {
            continue;
        }
        let Ok(fresh) = crate::import::import_bytes(&model, "model.rbxm") else { continue };
        let Some(theirs) = fresh.scene.rigs.first().map(|rig| rig.parts.clone()) else { continue };
        let lines_up = theirs.len() == ours.len() && theirs.iter().zip(&ours).all(|(t, o)| fresh.scene.parts[*t].name == scene.parts[*o].name);
        if !lines_up {
            continue;
        }
        for (&mine, &read) in ours.iter().zip(&theirs) {
            scene.parts[mine].surface_appearance = fresh.scene.parts[read].surface_appearance;
            if let (Drive::Root, Drive::Rigid { parent, offset }) = (scene.parts[mine].drive, fresh.scene.parts[read].drive) {
                let Some(at) = theirs.iter().position(|p| *p == parent) else { continue };
                scene.parts[mine].drive = Drive::Rigid { parent: ours[at], offset };
                attached = true;
            }
        }
    }
    if attached {
        reorder_parts(scene);
    }
}

/// Puts every part after the one it hangs from, keeping the order otherwise.
fn reorder_parts(scene: &mut Scene) {
    let holder = |p: usize| match scene.parts[p].drive {
        Drive::Root => None,
        Drive::Rigid { parent, .. } => Some(parent),
        Drive::Bone(b) => match scene.bones[b].link {
            BoneLink::Joint { part0, .. } => Some(part0),
            _ => None,
        },
    };
    let mut listed = vec![false; scene.parts.len()];
    for p in &scene.part_order {
        listed[*p] = true;
    }
    let mut placed = vec![false; scene.parts.len()];
    let mut order = Vec::with_capacity(scene.part_order.len());
    let mut left = scene.part_order.clone();
    while !left.is_empty() {
        let before = left.len();
        left.retain(|&p| {
            let ready = holder(p).is_none_or(|h| placed[h] || !listed[h]);
            if ready {
                placed[p] = true;
                order.push(p);
            }
            !ready
        });
        // A loop would never finish, so whatever's left keeps its order.
        if left.len() == before {
            order.append(&mut left);
        }
    }
    scene.part_order = order;
}

/// React was taken out of Kaye. A project still holding one opens without it, and the rest
/// of the file is read as usual.
fn read_project(bytes: &[u8]) -> Result<Project, serde_json::Error> {
    const RETIRED: &[u8] = b"\"React\"";
    if !bytes.windows(RETIRED.len()).any(|w| w == RETIRED) {
        return serde_json::from_slice(bytes);
    }
    let mut value: serde_json::Value = serde_json::from_slice(bytes)?;
    if let Some(modifiers) = value.pointer_mut("/scene/modifiers").and_then(serde_json::Value::as_array_mut) {
        modifiers.retain(|m| m.pointer("/kind/React").is_none());
    }
    serde_json::from_value(value)
}

/// Projects from before Kaye knew Roblox's built-in shapes hold a head as a mesh with a
/// scale and no file. Real heads like that were all the classic shape, wearing the classic face.
fn upgrade_heads(scene: &mut Scene) {
    for part in &mut scene.parts {
        if let MeshFit::Scaled { scale, offset } = part.mesh_fit {
            if part.name == "Head" && part.mesh_asset.is_none() {
                part.mesh_fit = MeshFit::Built { shape: BuiltShape::Head, scale, offset };
                part.texture_asset.get_or_insert_with(|| "rbxasset://textures/face.png".to_string());
            }
        }
    }
}

/// Stands for all the work in a scene, so comparing two says whether anything changed.
/// Poses on animated bones only say what frame is showing, so they don't count.
pub fn fingerprint(scene: &Scene, animation: &Animation) -> u64 {
    let mut scene = scene.clone();
    for bone in animation.keyed_bones() {
        if let Some(bone) = scene.bones.get_mut(bone) {
            bone.pose = Xf::IDENTITY;
            bone.value = 0.0;
        }
    }
    // What a new key starts with is a preference, not part of the work. Which animation's
    // range is showing goes by what's selected, so it doesn't count either.
    let mut animation = Animation { default_easing: Easing::default(), ..animation.clone() };
    animation.ensure_clips(&scene);
    if animation.shown.is_some() {
        animation.store_range();
        (animation.start, animation.end, animation.looping, animation.shown) = (0, 0, false, None);
        // With its own frame rate, the showing one's is on its animation now.
        if animation.separate {
            animation.fps = 0.0;
        }
    }
    let mut writer = HashWriter(DefaultHasher::new());
    serde_json::to_writer(&mut writer, &(&scene, &animation)).expect("a scene can always be written");
    writer.0.finish()
}

struct HashWriter(DefaultHasher);

impl std::io::Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.write(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Every id has to point at something, and bones have to form trees. The rest of Kaye
/// counts on both, so a damaged file has to stop here and not crash the app later.
fn check(project: &Project) -> Result<(), String> {
    let Project { scene, animation, view } = project;
    let (parts, bones, rigs) = (scene.parts.len(), scene.bones.len(), scene.rigs.len());
    let part = |id: usize| if id < parts { Ok(()) } else { Err(format!("no part {id}")) };
    let bone = |id: usize| if id < bones { Ok(()) } else { Err(format!("no bone {id}")) };
    let rig = |id: usize| if id < rigs { Ok(()) } else { Err(format!("no rig {id}")) };

    for p in &scene.parts {
        rig(p.rig)?;
        match p.drive {
            Drive::Root => {}
            Drive::Bone(b) => bone(b)?,
            Drive::Rigid { parent, .. } => part(parent)?,
        }
    }
    for (id, b) in scene.bones.iter().enumerate() {
        rig(b.rig)?;
        b.parent.map_or(Ok(()), bone)?;
        b.part.map_or(Ok(()), part)?;
        match b.link {
            BoneLink::Joint { part0, part1, .. } => {
                part(part0)?;
                part(part1)?;
            }
            BoneLink::Skinned { parent_part, parent_bone, .. } => {
                parent_part.map_or(Ok(()), part)?;
                parent_bone.map_or(Ok(()), bone)?;
            }
            BoneLink::Free { follow, .. } => follow.map_or(Ok(()), bone)?,
        }
        for child in &b.children {
            bone(*child)?;
            if scene.bones[*child].parent != Some(id) {
                return Err(format!("bone {child} is lost from its parent"));
            }
        }
    }
    for id in 0..bones {
        let (mut at, mut steps) = (scene.bones[id].parent, 0);
        while let Some(parent) = at {
            steps += 1;
            if steps > bones {
                return Err(format!("bone {id} is its own parent"));
            }
            at = scene.bones[parent].parent;
        }
    }
    let folder = |id: usize| if id < scene.folders.len() { Ok(()) } else { Err(format!("no folder {id}")) };
    for r in &scene.rigs {
        r.parts.iter().try_for_each(|p| part(*p))?;
        r.bones.iter().try_for_each(|b| bone(*b))?;
        r.folder.map_or(Ok(()), folder)?;
    }
    for (id, f) in scene.folders.iter().enumerate() {
        f.parent.map_or(Ok(()), folder)?;
        if f.parent.is_some_and(|parent| scene.folder_is_within(parent, id)) {
            return Err(format!("folder {id} is inside itself"));
        }
    }
    scene.part_order.iter().try_for_each(|p| part(*p))?;
    scene.skinned_order.iter().chain(&scene.free_order).try_for_each(|b| bone(*b))?;
    let modifier = |m: &Modifier| -> Result<(), String> {
        bone(m.bone)?;
        match m.kind {
            ModifierKind::Ik(ik) => {
                bone(ik.target)?;
                ik.pole.map_or(Ok(()), bone)
            }
            ModifierKind::Unparent(u) => u.anchor.map_or(Ok(()), bone),
            ModifierKind::LookAt(look) => bone(look.target),
            ModifierKind::Copy(copy) => copy.source.map_or(Ok(()), bone),
            ModifierKind::Ragdoll(ragdoll) => ragdoll.objects.iter().flatten().try_for_each(|r| rig(*r)),
            ModifierKind::Limit(_) | ModifierKind::Inherit(_) => Ok(()),
        }
    };
    scene.modifiers.iter().try_for_each(modifier)?;
    for (index, clip) in animation.clips.iter().enumerate() {
        rig(clip.rig)?;
        if clip.fps.is_some_and(|fps| !(fps > 0.0 && fps.is_finite())) {
            return Err(format!("animation {index}'s frame rate isn't a usable number"));
        }
        if let Some(setup) = &clip.setup {
            setup.modifiers.iter().try_for_each(modifier)?;
            for (id, link) in &setup.controls {
                bone(*id)?;
                if let BoneLink::Free { follow, .. } = link {
                    follow.map_or(Ok(()), bone)?;
                }
            }
        }
        let playing = animation.clips.iter().filter(|c| c.rig == clip.rig && c.active).count();
        if playing != 1 {
            return Err(format!("rig {} has {playing} animations playing", clip.rig));
        }
        if clip.active && !clip.tracks.is_empty() {
            return Err(format!("animation {index} has its keys in two places"));
        }
    }
    if animation.shown.is_some_and(|i| i >= animation.clips.len()) {
        return Err("no such animation".to_string());
    }
    for (id, track) in animation.all_tracks() {
        bone(*id)?;
        if !track.keys.windows(2).all(|w| w[0].frame < w[1].frame) {
            return Err(format!("the keys on bone {id} are out of order"));
        }
    }
    if !(animation.fps > 0.0 && animation.fps.is_finite()) {
        return Err("the frame rate isn't a usable number".to_string());
    }
    view.camera.map_or(Ok(()), bone)
}
