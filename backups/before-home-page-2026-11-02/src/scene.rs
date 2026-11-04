//! The in-memory rig document: parts, bones and how a pose moves them.
//!
//! Roblox joints follow `Part1 = Part0 * C0 * Transform * C1:Inverse()`, and
//! skinned bones follow `Bone = Parent * CFrame * Transform`. `Bone::pose` is
//! that `Transform`, the only thing an animation changes. It is what keyframes
//! store. Modifiers run on top of it, see `modifiers.rs`.
//!
//! Cameras, IK targets and spawned shapes are bones too, with no joint. Being bones
//! means selecting, moving, keying, undo and the Graph tab all work on them with no
//! extra code. Anything that means a real joint checks `BoneKind::is_joint`.

use std::sync::Arc;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::dynamics::Dynamics;
use crate::math::Xf;
use crate::modifiers::{self, Modifier, ModifierKind};

pub type PartId = usize;
pub type BoneId = usize;
pub type RigId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PartShape {
    Block,
    Ball,
    Cylinder,
    Wedge,
    CornerWedge,
}

impl PartShape {
    pub const ALL: [PartShape; 5] = [PartShape::Block, PartShape::Ball, PartShape::Cylinder, PartShape::Wedge, PartShape::CornerWedge];

    pub fn label(self) -> &'static str {
        match self {
            PartShape::Block => "Block",
            PartShape::Ball => "Sphere",
            PartShape::Cylinder => "Cylinder",
            PartShape::Wedge => "Wedge",
            PartShape::CornerWedge => "Corner Wedge",
        }
    }

    /// The Roblox class a part of this shape would be.
    fn class(self) -> &'static str {
        match self {
            PartShape::Wedge => "WedgePart",
            PartShape::CornerWedge => "CornerWedgePart",
            _ => "Part",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Part {
    pub name: String,
    pub class: String,
    pub rig: RigId,
    pub size: Vec3,
    pub color: [f32; 3],
    pub transparency: f32,
    pub shape: PartShape,
    /// Asset references kept for when real meshes and textures are downloaded.
    pub mesh_asset: Option<String>,
    pub texture_asset: Option<String>,
    #[serde(default)]
    pub mesh_fit: MeshFit,
    pub file_cframe: Xf,
    #[serde(skip)]
    pub world: Xf,
    pub drive: Drive,
    pub visible: bool,
    /// Deleted on its own, for a welded part. Ids have to stay stable and undo can bring
    /// it back, so it stays in the list.
    #[serde(default)]
    pub removed: bool,
}

impl Part {
    /// Fully transparent parts such as HumanoidRootPart are helpers, not visuals.
    pub fn is_helper(&self) -> bool {
        self.transparency >= 0.999
    }

    /// Where the middle of what's drawn sits in the part, and how big it is. A SpecialMesh's
    /// shape scales with the part, except that a classic head stays as round as it's tall,
    /// on the narrower side. That's why a head part 2 studs wide doesn't look it.
    pub fn drawn_box(&self) -> (Vec3, Vec3) {
        match self.mesh_fit {
            MeshFit::Built { shape: BuiltShape::Head, scale, offset } => {
                let round = self.size.x.min(self.size.z);
                (offset, Vec3::new(round, self.size.y, round) * scale)
            }
            MeshFit::Built { scale, offset, .. } => (offset, self.size * scale),
            _ => (Vec3::ZERO, self.size),
        }
    }
}

/// A shape Roblox draws itself, with no mesh file behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuiltShape {
    Head,
    Block,
    Sphere,
    Cylinder,
    Wedge,
    CornerWedge,
}

/// How a mesh sits in its part.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum MeshFit {
    /// A MeshPart, whose Size stretches the mesh's own bounds to fit.
    #[default]
    Size,
    /// A SpecialMesh with a file, which ignores Size and has its own scale and offset.
    Scaled { scale: Vec3, offset: Vec3 },
    /// A SpecialMesh naming one of Roblox's own shapes.
    Built { shape: BuiltShape, scale: Vec3, offset: Vec3 },
}

/// What positions a part each time the pose is evaluated.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Drive {
    /// Not attached to anything earlier in the rig; stays at its file position.
    Root,
    /// Part1 of this bone's joint.
    Bone(BoneId),
    /// Rigidly attached through a Weld, WeldConstraint and so on.
    Rigid { parent: PartId, offset: Xf },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoneKind {
    Motor6D,
    LegacyMotor,
    AnimationConstraint,
    Skinned,
    Camera,
    IkTarget,
    IkPole,
    LookTarget,
    /// A shape spawned in Kaye. The bone moves it, and `Bone::part` is what it moves.
    Shape,
}

impl BoneKind {
    pub fn label(self) -> &'static str {
        match self {
            BoneKind::Motor6D => "Motor6D",
            BoneKind::LegacyMotor => "Motor",
            BoneKind::AnimationConstraint => "AnimationConstraint",
            BoneKind::Skinned => "Bone",
            BoneKind::Camera => "Camera",
            BoneKind::IkTarget => "IK Target",
            BoneKind::IkPole => "IK Pole",
            BoneKind::LookTarget => "Look At Target",
            BoneKind::Shape => "Shape",
        }
    }

    /// A real joint in the rig, as opposed to something Kaye added around it.
    pub fn is_joint(self) -> bool {
        !matches!(self, BoneKind::Camera | BoneKind::IkTarget | BoneKind::IkPole | BoneKind::LookTarget | BoneKind::Shape)
    }

    pub fn is_control(self) -> bool {
        matches!(self, BoneKind::IkTarget | BoneKind::IkPole | BoneKind::LookTarget)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum BoneLink {
    Joint {
        part0: PartId,
        part1: PartId,
        c0: Xf,
        c1: Xf,
    },
    Skinned {
        parent_part: Option<PartId>,
        parent_bone: Option<BoneId>,
        local: Xf,
    },
    /// Cameras, IK controls and shapes, which hang from nothing unless told to follow
    /// a bone. `rest` is relative to that bone when there is one.
    Free { rest: Xf, follow: Option<BoneId> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bone {
    pub name: String,
    pub kind: BoneKind,
    pub rig: RigId,
    pub link: BoneLink,
    pub parent: Option<BoneId>,
    pub children: Vec<BoneId>,
    /// The animated Transform, relative to `base`.
    pub pose: Xf,
    /// `pose` after IK. This is what actually places the bone.
    #[serde(skip)]
    pub solved: Xf,
    /// One extra animated number: a camera's field of view, an IK target's
    /// influence, or an Unparent modifier's influence.
    pub value: f32,
    /// World frame with no pose applied. Depends only on the bone's ancestors,
    /// and on an Unparent modifier if it has one.
    #[serde(skip)]
    pub base: Xf,
    /// Where the joint itself would put `base`, ignoring Unparent.
    #[serde(skip)]
    pub joint_base: Xf,
    /// World frame with the pose applied. Its origin is the pivot.
    #[serde(skip)]
    pub frame: Xf,
    /// Where the drawn bone ends, in `frame` space.
    pub tail_local: Vec3,
    /// The drawn line to the parent attaches at its tail, not its head. Decided
    /// once at rest, so posing can't make the line jump between the two.
    pub link_to_parent_tail: bool,
    /// Taken out of the scene. Ids have to stay stable and undo can bring it back,
    /// so it stays in the list.
    pub removed: bool,
    /// Has an Unparent or Inherit Rotation modifier. Set by `evaluate`, so placing a bone
    /// without one doesn't have to look through every modifier.
    #[serde(skip)]
    pub(crate) rebased: bool,
    /// The part this bone carries around, for a spawned shape.
    #[serde(default)]
    pub part: Option<PartId>,
}

impl Bone {
    pub fn new(name: String, kind: BoneKind, link: BoneLink, parent: Option<BoneId>) -> Self {
        Self {
            name,
            kind,
            rig: 0,
            link,
            parent,
            children: Vec::new(),
            pose: Xf::IDENTITY,
            solved: Xf::IDENTITY,
            value: 1.0,
            base: Xf::IDENTITY,
            joint_base: Xf::IDENTITY,
            frame: Xf::IDENTITY,
            tail_local: Vec3::Y,
            link_to_parent_tail: true,
            removed: false,
            rebased: false,
            part: None,
        }
    }

    pub fn head(&self) -> Vec3 {
        self.frame.pos
    }

    pub fn tail(&self) -> Vec3 {
        self.frame.point(self.tail_local)
    }

    /// The Transform Roblox needs on the real joint to put the bone where modifiers
    /// did. An export has to write this, not `pose`.
    pub fn output(&self) -> Xf {
        self.joint_base.inverse() * self.frame
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RigKind {
    R6,
    R15,
    Rthro,
    Skinned,
    Custom,
    Static,
    /// Holds one camera, which the outliner shows as a single row.
    Camera,
    /// Holds one spawned shape, also a single row.
    Shape,
}

impl RigKind {
    pub fn label(self) -> &'static str {
        match self {
            RigKind::R6 => "R6",
            RigKind::R15 => "R15",
            RigKind::Rthro => "Rthro",
            RigKind::Skinned => "Skinned",
            RigKind::Custom => "Custom rig",
            RigKind::Static => "Static model",
            RigKind::Camera => "Camera",
            RigKind::Shape => "Shape",
        }
    }

    /// One object, not a rig with bones in it.
    pub fn is_single(self) -> bool {
        matches!(self, RigKind::Camera | RigKind::Shape)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rig {
    pub name: String,
    pub kind: RigKind,
    pub source_file: String,
    pub parts: Vec<PartId>,
    pub bones: Vec<BoneId>,
    pub visible: bool,
    /// The templates of the character's Shirt and Pants.
    #[serde(default)]
    pub shirt: Option<String>,
    #[serde(default)]
    pub pants: Option<String>,
    /// Where the whole rig has been moved to. It's never keyed and never exported in an
    /// animation, since a pose only ever says how a joint turns.
    #[serde(default)]
    pub placement: Xf,
    #[serde(default)]
    pub removed: bool,
    /// The rig as it came, as a `.rbxm`, which a model export writes back out. Projects
    /// saved before Kaye kept this have none.
    #[serde(default, with = "model_bytes", skip_serializing_if = "Option::is_none")]
    pub model: Option<Arc<Vec<u8>>>,
}

/// A rig's original model as text, which is far smaller in a project file than a list of
/// numbers. Shared, so copying a scene doesn't copy the model.
mod model_bytes {
    use std::sync::Arc;

    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(model: &Option<Arc<Vec<u8>>>, out: S) -> Result<S::Ok, S::Error> {
        match model {
            Some(bytes) => out.serialize_str(&STANDARD.encode(bytes.as_slice())),
            None => out.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(input: D) -> Result<Option<Arc<Vec<u8>>>, D::Error> {
        let text = Option::<String>::deserialize(input)?;
        text.map(|text| STANDARD.decode(text).map(Arc::new).map_err(serde::de::Error::custom)).transpose()
    }
}

/// The whole scene with no pose applied.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RestPose {
    pub bones: Vec<Xf>,
    pub parts: Vec<Xf>,
}

/// Everything undo has to put back that isn't a pose or a key.
#[derive(Clone, Debug, PartialEq)]
pub struct RigState {
    modifiers: Vec<Modifier>,
    values: Vec<f32>,
    removed: Vec<bool>,
    links: Vec<BoneLink>,
    /// What can be changed about each spawned shape.
    shapes: Vec<(PartId, Vec3, [f32; 3], f32)>,
    parts_removed: Vec<bool>,
    rigs_removed: Vec<bool>,
    placements: Vec<Xf>,
}

/// The order to place some of a scene in. Parents always come before children.
#[derive(Clone, Debug, Default)]
pub(crate) struct Placement {
    free: Vec<BoneId>,
    parts: Vec<PartId>,
    skinned: Vec<BoneId>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Scene {
    pub parts: Vec<Part>,
    pub bones: Vec<Bone>,
    pub rigs: Vec<Rig>,
    /// Parents always come before children.
    pub part_order: Vec<PartId>,
    pub skinned_order: Vec<BoneId>,
    /// Cameras and IK targets, placed first because nothing moves them.
    pub free_order: Vec<BoneId>,
    pub modifiers: Vec<Modifier>,
    /// What React and Ragdoll came to on every frame, see `dynamics.rs`.
    #[serde(skip)]
    pub dynamics: Dynamics,
}

impl Scene {
    pub fn is_empty(&self) -> bool {
        self.live_parts() == 0 && !self.bones.iter().any(|b| b.kind.is_joint() && !b.removed)
    }

    /// Recomputes every world transform from the current poses, then runs modifiers.
    pub fn evaluate(&mut self) {
        for bone in &mut self.bones {
            bone.solved = bone.pose;
            bone.rebased = false;
        }
        for modifier in &self.modifiers {
            if matches!(modifier.kind, ModifierKind::Unparent(_) | ModifierKind::Inherit(_)) {
                self.bones[modifier.bone].rebased = true;
            }
        }
        self.place_all();
        // An Unparent anchor can sit later in the order than the bone that uses it.
        if modifiers::has_anchors(self) {
            self.place_all();
        }
        modifiers::solve(self);
    }

    /// Works out every world transform from `Bone::solved`.
    pub(crate) fn place_all(&mut self) {
        let order = Placement {
            free: std::mem::take(&mut self.free_order),
            parts: std::mem::take(&mut self.part_order),
            skinned: std::mem::take(&mut self.skinned_order),
        };
        self.place(&order);
        self.free_order = order.free;
        self.part_order = order.parts;
        self.skinned_order = order.skinned;
    }

    /// Each rig's own placing order. Placing just one rig only gives the right answer
    /// when nothing in another rig hangs from it, see `modifiers::crosses_rigs`.
    pub(crate) fn rig_placements(&self) -> Vec<Placement> {
        let mut rigs = vec![Placement::default(); self.rigs.len()];
        for id in &self.free_order {
            rigs[self.bones[*id].rig].free.push(*id);
        }
        for id in &self.part_order {
            rigs[self.parts[*id].rig].parts.push(*id);
        }
        for id in &self.skinned_order {
            rigs[self.bones[*id].rig].skinned.push(*id);
        }
        rigs
    }

    pub(crate) fn place(&mut self, order: &Placement) {
        for &id in &order.free {
            if let BoneLink::Free { rest, follow: None } = self.bones[id].link {
                self.place_bone(id, self.rigs[self.bones[id].rig].placement * rest);
            }
        }
        for &id in &order.parts {
            match self.parts[id].drive {
                Drive::Root => self.parts[id].world = self.rigs[self.parts[id].rig].placement * self.parts[id].file_cframe,
                Drive::Rigid { parent, offset } => {
                    self.parts[id].world = self.parts[parent].world * offset;
                }
                Drive::Bone(b) => {
                    let BoneLink::Joint { part0, c0, c1, .. } = self.bones[b].link else {
                        continue;
                    };
                    self.place_bone(b, self.parts[part0].world * c0);
                    self.parts[id].world = self.bones[b].frame * c1.inverse();
                }
            }
        }
        for &id in &order.skinned {
            let BoneLink::Skinned {
                parent_part,
                parent_bone,
                local,
            } = self.bones[id].link
            else {
                continue;
            };
            let parent = match (parent_bone, parent_part) {
                (Some(b), _) => self.bones[b].frame,
                (None, Some(p)) => self.parts[p].world,
                (None, None) => self.rigs[self.bones[id].rig].placement,
            };
            self.place_bone(id, parent * local);
        }
        // Controls that follow a bone need it placed first.
        for &id in &order.free {
            if let BoneLink::Free { rest, follow: Some(bone) } = self.bones[id].link {
                self.place_bone(id, self.bones[bone].frame * rest);
            }
        }
    }

    fn place_bone(&mut self, id: BoneId, joint_base: Xf) {
        let base = modifiers::held_base(self, id, joint_base);
        let bone = &mut self.bones[id];
        bone.joint_base = joint_base;
        bone.base = base;
        bone.frame = base * bone.solved;
        if let Some(part) = bone.part {
            self.parts[part].world = self.bones[id].frame;
        }
    }

    /// Moves a bone so its world frame becomes `frame`, then re-evaluates.
    pub fn set_bone_frame(&mut self, bone: BoneId, frame: Xf) {
        let pose = self.bones[bone].base.inverse() * frame;
        self.set_bone_pose(bone, pose);
    }

    pub fn set_bone_pose(&mut self, bone: BoneId, pose: Xf) {
        self.bones[bone].pose = pose;
        self.evaluate();
    }

    /// Moves a whole rig. Keys and poses don't change, so none of it reaches the animation.
    pub fn set_rig_placement(&mut self, rig: RigId, placement: Xf) {
        self.rigs[rig].placement = placement;
        self.evaluate();
    }

    /// Where a rig's handles go: on the part it stands on, turned the way the rig is.
    pub fn rig_frame(&self, rig: RigId) -> Xf {
        let placement = self.rigs[rig].placement;
        let standing_on = self.rigs[rig].parts.iter().find(|p| self.parts[**p].drive == Drive::Root);
        let middle = || self.bounds(Some(rig)).map(|(min, max)| (min + max) * 0.5);
        let pos = standing_on.map(|p| self.parts[*p].world.pos).or_else(middle).unwrap_or(placement.pos);
        Xf::new(placement.rot, pos)
    }
    /// The bone a part follows, walking up through rigid attachments.
    pub fn part_owner(&self, mut part: PartId) -> Option<BoneId> {
        for _ in 0..self.parts.len() + 1 {
            match self.parts[part].drive {
                Drive::Root => return None,
                Drive::Bone(b) => return Some(b),
                Drive::Rigid { parent, .. } => part = parent,
            }
        }
        None
    }

    pub fn bone_is_visible(&self, bone: BoneId) -> bool {
        !self.bones[bone].removed && self.rigs[self.bones[bone].rig].visible
    }

    /// The bones of a rig that haven't been taken out.
    pub fn live_bones(&self, rig: RigId) -> Vec<BoneId> {
        self.rigs[rig].bones.iter().copied().filter(|b| !self.bones[*b].removed).collect()
    }

    /// The rig's real joints, leaving out the targets Kaye added.
    pub fn joints(&self, rig: RigId) -> Vec<BoneId> {
        self.live_bones(rig).into_iter().filter(|b| self.bones[*b].kind.is_joint()).collect()
    }

    /// Every camera still in the scene.
    pub fn cameras(&self) -> impl Iterator<Item = BoneId> + '_ {
        (0..self.bones.len()).filter(|b| self.bones[*b].kind == BoneKind::Camera && !self.bones[*b].removed)
    }

    /// Adds something with no joint to a rig. `rest` is where it sits with no pose.
    pub fn add_control(&mut self, name: String, kind: BoneKind, rig: RigId, rest: Xf, value: f32) -> BoneId {
        let id = self.bones.len();
        let rest = self.rigs[rig].placement.inverse() * rest;
        let mut bone = Bone::new(name, kind, BoneLink::Free { rest, follow: None }, None);
        bone.rig = rig;
        bone.value = value;
        bone.tail_local = Vec3::ZERO;
        self.bones.push(bone);
        self.rigs[rig].bones.push(id);
        self.free_order.push(id);
        id
    }

    /// Adds a camera at `at`. Its pose is its world position, which is what someone
    /// typing in numbers expects.
    pub fn add_camera(&mut self, at: Xf, fov: f32) -> BoneId {
        let number = self.bones.iter().filter(|b| b.kind == BoneKind::Camera).count() + 1;
        let name = if number == 1 { "Camera".to_string() } else { format!("Camera {number}") };
        let rig = self.rigs.len();
        self.rigs.push(Rig {
            name: name.clone(),
            kind: RigKind::Camera,
            source_file: String::new(),
            parts: Vec::new(),
            bones: Vec::new(),
            visible: true,
            shirt: None,
            pants: None,
            placement: Xf::IDENTITY,
            removed: false,
            model: None,
        });
        let id = self.add_control(name, BoneKind::Camera, rig, Xf::IDENTITY, fov);
        self.bones[id].pose = at;
        // Picks up along the view line, not just at the lens.
        self.bones[id].tail_local = Vec3::new(0.0, 0.0, -1.4);
        id
    }

    /// Spawns a shape in a rig of its own, the way a camera has one. Its pose is its
    /// world position, which is what someone typing in numbers expects.
    pub fn add_shape(&mut self, shape: PartShape, at: Xf, size: Vec3) -> BoneId {
        let same = self.bones.iter().filter(|b| !b.removed).filter_map(|b| b.part).filter(|p| self.parts[*p].shape == shape).count();
        let name = if same == 0 { shape.label().to_string() } else { format!("{} {}", shape.label(), same + 1) };
        let rig = self.rigs.len();
        let part = self.parts.len();
        self.rigs.push(Rig {
            name: name.clone(),
            kind: RigKind::Shape,
            source_file: String::new(),
            parts: vec![part],
            bones: Vec::new(),
            visible: true,
            shirt: None,
            pants: None,
            placement: Xf::IDENTITY,
            removed: false,
            model: None,
        });
        let id = self.add_control(name.clone(), BoneKind::Shape, rig, Xf::IDENTITY, 1.0);
        self.bones[id].pose = at;
        self.bones[id].part = Some(part);
        self.parts.push(Part {
            name,
            class: shape.class().to_string(),
            rig,
            size,
            color: DEFAULT_PART_COLOR,
            transparency: 0.0,
            shape,
            mesh_asset: None,
            texture_asset: None,
            mesh_fit: MeshFit::Size,
            file_cframe: Xf::IDENTITY,
            world: at,
            drive: Drive::Bone(id),
            visible: true,
            removed: false,
        });
        id
    }

    pub fn rig_state(&self) -> RigState {
        RigState {
            modifiers: self.modifiers.clone(),
            values: self.bones.iter().map(|b| b.value).collect(),
            removed: self.bones.iter().map(|b| b.removed).collect(),
            links: self.bones.iter().map(|b| b.link).collect(),
            shapes: self
                .bones
                .iter()
                .filter_map(|b| b.part)
                .map(|id| {
                    let part = &self.parts[id];
                    (id, part.size, part.color, part.transparency)
                })
                .collect(),
            parts_removed: self.parts.iter().map(|p| p.removed).collect(),
            rigs_removed: self.rigs.iter().map(|r| r.removed).collect(),
            placements: self.rigs.iter().map(|r| r.placement).collect(),
        }
    }

    /// Bones added after this state was taken didn't exist yet, so they go away.
    pub fn restore_rig_state(&mut self, state: &RigState) {
        self.modifiers = state.modifiers.clone();
        for (id, bone) in self.bones.iter_mut().enumerate() {
            bone.removed = state.removed.get(id).copied().unwrap_or(true);
            if let Some(value) = state.values.get(id) {
                bone.value = *value;
            }
            if let Some(link) = state.links.get(id) {
                bone.link = *link;
            }
        }
        for (id, size, color, transparency) in &state.shapes {
            let part = &mut self.parts[*id];
            part.size = *size;
            part.color = *color;
            part.transparency = *transparency;
        }
        for (part, removed) in self.parts.iter_mut().zip(&state.parts_removed) {
            part.removed = *removed;
        }
        for (id, rig) in self.rigs.iter_mut().enumerate() {
            rig.removed = state.rigs_removed.get(id).copied().unwrap_or(true);
            if let Some(placement) = state.placements.get(id) {
                rig.placement = *placement;
            }
        }
    }

    /// Whether a part has gone, on its own or with the bone or rig that carries it.
    pub fn part_is_removed(&self, mut part: PartId) -> bool {
        for _ in 0..self.parts.len() + 1 {
            let p = &self.parts[part];
            if p.removed || self.rigs[p.rig].removed {
                return true;
            }
            match p.drive {
                Drive::Root => return false,
                Drive::Bone(b) => return self.bones[b].removed,
                Drive::Rigid { parent, .. } => part = parent,
            }
        }
        false
    }

    pub fn part_is_visible(&self, part: PartId) -> bool {
        let p = &self.parts[part];
        p.visible && self.rigs[p.rig].visible && !p.is_helper() && !self.part_is_removed(part)
    }

    /// How many parts are still in the scene. A deleted part stays in the list.
    pub fn live_parts(&self) -> usize {
        (0..self.parts.len()).filter(|p| !self.part_is_removed(*p)).count()
    }

    /// Bounds of what is drawn, optionally limited to one rig.
    pub fn bounds(&self, rig: Option<RigId>) -> Option<(Vec3, Vec3)> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut any = false;
        for (id, p) in self.parts.iter().enumerate() {
            if rig.is_some_and(|r| r != p.rig) || !self.part_is_visible(id) {
                continue;
            }
            for corner in BOX_CORNERS {
                let c = p.world.point(corner * p.size * 0.5);
                min = min.min(c);
                max = max.max(c);
            }
            any = true;
        }
        for b in &self.bones {
            if rig.is_some_and(|r| r != b.rig) || !self.rigs[b.rig].visible || !b.kind.is_joint() || b.removed {
                continue;
            }
            for c in [b.head(), b.tail()] {
                min = min.min(c);
                max = max.max(c);
            }
            any = true;
        }
        any.then_some((min, max))
    }

    /// Where every bone and part sits with no pose at all, which a skinned mesh is
    /// bound to. Costs a whole evaluation, so anyone calling it often should keep it.
    pub fn rest_pose(&self) -> RestPose {
        let mut rest = self.clone();
        rest.modifiers.clear();
        rest.bones.iter_mut().for_each(|b| b.pose = Xf::IDENTITY);
        rest.evaluate();
        RestPose { bones: rest.bones.iter().map(|b| b.frame).collect(), parts: rest.parts.iter().map(|p| p.world).collect() }
    }

    /// Adds another imported scene, shifting its ids to fit after the current ones.
    pub fn append(&mut self, mut other: Scene) {
        let (po, bo, ro) = (self.parts.len(), self.bones.len(), self.rigs.len());
        for p in &mut other.parts {
            p.rig += ro;
            match &mut p.drive {
                Drive::Root => {}
                Drive::Bone(b) => *b += bo,
                Drive::Rigid { parent, .. } => *parent += po,
            }
        }
        for b in &mut other.bones {
            b.rig += ro;
            b.parent = b.parent.map(|x| x + bo);
            b.children.iter_mut().for_each(|c| *c += bo);
            match &mut b.link {
                BoneLink::Joint { part0, part1, .. } => {
                    *part0 += po;
                    *part1 += po;
                }
                BoneLink::Skinned {
                    parent_part,
                    parent_bone,
                    ..
                } => {
                    *parent_part = parent_part.map(|x| x + po);
                    *parent_bone = parent_bone.map(|x| x + bo);
                }
                BoneLink::Free { .. } => {}
            }
        }
        for r in &mut other.rigs {
            r.parts.iter_mut().for_each(|x| *x += po);
            r.bones.iter_mut().for_each(|x| *x += bo);
        }
        self.parts.append(&mut other.parts);
        self.bones.append(&mut other.bones);
        self.rigs.append(&mut other.rigs);
        self.part_order
            .extend(other.part_order.into_iter().map(|x| x + po));
        self.skinned_order
            .extend(other.skinned_order.into_iter().map(|x| x + bo));
    }

    /// Works out where each drawn bone ends. Runs once, on the rest pose.
    pub fn compute_tails(&mut self) {
        for id in 0..self.bones.len() {
            let tail = self.rest_tail_world(id);
            let bone = &mut self.bones[id];
            let mut local = bone.frame.inverse().point(tail);
            if local.length() < MIN_BONE_LENGTH {
                local = local.try_normalize().unwrap_or(Vec3::Y) * MIN_BONE_LENGTH;
            }
            bone.tail_local = local;
        }
        for id in 0..self.bones.len() {
            if let Some(parent) = self.bones[id].parent {
                let head = self.bones[id].head();
                let (p_head, p_tail) = (self.bones[parent].head(), self.bones[parent].tail());
                // R6 shoulders are exactly as far from the torso bone's head as its tail,
                // so ties have to go one way on purpose, not by rounding noise.
                self.bones[id].link_to_parent_tail = p_tail.distance_squared(head) <= p_head.distance_squared(head) + 1e-3;
            }
        }
    }

    fn rest_tail_world(&self, id: BoneId) -> Vec3 {
        let bone = &self.bones[id];
        let head = bone.head();
        let part = match bone.link {
            BoneLink::Joint { part1, .. } => Some(part1),
            BoneLink::Skinned { .. } | BoneLink::Free { .. } => None,
        };
        let part_extent = part.map_or(0.6, |p| self.parts[p].size.max_element());

        if !bone.children.is_empty() {
            let mean = bone
                .children
                .iter()
                .map(|c| self.bones[*c].head())
                .sum::<Vec3>()
                / bone.children.len() as f32;
            // A hub like a torso has children pulling in opposite directions, so
            // their average lands on the head and tells us nothing.
            if head.distance(mean) > part_extent * 0.25 {
                return mean;
            }
        }
        match part {
            Some(p) => part_far_end(&self.parts[p], head),
            None => {
                let dir = bone
                    .parent
                    .map(|p| self.bones[p].head())
                    .and_then(|p| (head - p).try_normalize())
                    .unwrap_or(Vec3::Y);
                head + dir * 0.4
            }
        }
    }
}

pub const MIN_BONE_LENGTH: f32 = 0.2;
/// Roblox's Medium stone grey.
const DEFAULT_PART_COLOR: [f32; 3] = [0.64, 0.64, 0.65];

const BOX_CORNERS: [Vec3; 8] = [
    Vec3::new(-1.0, -1.0, -1.0),
    Vec3::new(1.0, -1.0, -1.0),
    Vec3::new(-1.0, 1.0, -1.0),
    Vec3::new(1.0, 1.0, -1.0),
    Vec3::new(-1.0, -1.0, 1.0),
    Vec3::new(1.0, -1.0, 1.0),
    Vec3::new(-1.0, 1.0, 1.0),
    Vec3::new(1.0, 1.0, 1.0),
];

/// The center of the part's face opposite the joint, so a leaf bone runs down a
/// limb instead of cutting out through its side. The axis is whichever the joint
/// sits furthest along, favoring Y because R6 shoulders are offset on X and Y
/// equally and limbs hang vertically. A centered joint points up.
fn part_far_end(part: &Part, head: Vec3) -> Vec3 {
    let local = part.world.inverse().point(head);
    let reach = Vec3::new(local.x.abs(), local.y.abs() * 1.25, local.z.abs());
    let axis = if reach.y >= reach.x && reach.y >= reach.z {
        1
    } else if reach.x >= reach.z {
        0
    } else {
        2
    };
    let sign = if local[axis] > 0.01 { -1.0 } else { 1.0 };
    let mut end = Vec3::ZERO;
    end[axis] = part.size[axis] * 0.5 * sign;
    part.world.point(end)
}
