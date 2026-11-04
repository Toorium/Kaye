//! Kaye project files: the rigs, keys, modifiers and cameras, plus where the view was
//! left. Like the rig logic, this has no interface code.
//!
//! A project stores the scene itself, not the model it came from, so it opens with no
//! other file around. Anything `Scene::evaluate` can work out is left out and worked
//! out again on load. The undo history isn't kept.

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::animation::{Animation, Easing};
use crate::math::Xf;
use crate::modifiers::ModifierKind;
use crate::scene::{BoneId, BoneLink, Drive, Scene};

pub const EXTENSION: &str = "kaye";
const VERSION: u32 = 1;

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
    let mut project: Project = serde_json::from_slice(bytes).map_err(|e| format!("The project is damaged: {e}"))?;
    check(&project).map_err(|what| format!("The project is damaged: {what}"))?;
    project.scene.evaluate();
    Ok(project)
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
    // What a new key starts with is a preference, not part of the work.
    let animation = Animation { default_easing: Easing::default(), ..animation.clone() };
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
    for r in &scene.rigs {
        r.parts.iter().try_for_each(|p| part(*p))?;
        r.bones.iter().try_for_each(|b| bone(*b))?;
    }
    scene.part_order.iter().try_for_each(|p| part(*p))?;
    scene.skinned_order.iter().chain(&scene.free_order).try_for_each(|b| bone(*b))?;
    for m in &scene.modifiers {
        bone(m.bone)?;
        match m.kind {
            ModifierKind::Ik(ik) => {
                bone(ik.target)?;
                ik.pole.map_or(Ok(()), bone)?;
            }
            ModifierKind::Unparent(u) => u.anchor.map_or(Ok(()), bone)?,
            ModifierKind::LookAt(look) => bone(look.target)?,
            ModifierKind::Copy(copy) => copy.source.map_or(Ok(()), bone)?,
            ModifierKind::Limit(_) => {}
        }
    }
    if animation.active_layer >= animation.layer_count() {
        return Err("no such layer".to_string());
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
