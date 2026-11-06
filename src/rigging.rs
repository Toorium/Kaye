//! Rigging: joining parts with Motor6Ds, moving where joints turn, moving parts, and
//! Roblox Attachments. Everything here works on the rig at rest, the way the Rigging tab
//! shows it (`Scene::at_rest`), and goes through `Scene::rig_state` for undo. No interface
//! code.
//!
//! A joint holds its second part with `Part1 = Part0 * C0 * C1:Inverse()` at rest, so its
//! pivot sits at `Part0 * C0`, which is also `Part1 * C1`. Moving a pivot rewrites both
//! offsets so neither part moves. Moving a part rewrites the offset that ties it to what holds
//! it, so the pivot moves with it and everything below follows.
//!
//! Rig exports read the rig's original model again and write these changes into it, so
//! nothing Kaye doesn't understand is lost. `prepare` notes which instance in the model each
//! joint and attachment came from, counting every instance in order.

use std::collections::HashSet;

use rbx_dom_weak::WeakDom;
use rbx_dom_weak::types::Ref;

use crate::math::Xf;
use crate::scene::{Attachment, Bone, BoneId, BoneKind, BoneLink, Drive, PartId, RigId, RigKind, Scene};

/// The part that holds this one, through a joint or a weld.
pub fn holder(scene: &Scene, part: PartId) -> Option<PartId> {
    match scene.parts[part].drive {
        Drive::Root => None,
        Drive::Bone(b) => match scene.bones[b].link {
            BoneLink::Joint { part0, .. } => Some(part0),
            _ => None,
        },
        Drive::Rigid { parent, .. } => Some(parent),
    }
}

/// Whether `part` hangs from `above`, however far up.
pub fn hangs_from(scene: &Scene, part: PartId, above: PartId) -> bool {
    let mut at = holder(scene, part);
    for _ in 0..=scene.parts.len() {
        match at {
            Some(p) if p == above => return true,
            Some(p) => at = holder(scene, p),
            None => return false,
        }
    }
    false
}

/// The joint that holds a part, when one does.
pub fn joint_of(scene: &Scene, part: PartId) -> Option<BoneId> {
    match scene.parts[part].drive {
        Drive::Bone(b) if matches!(scene.bones[b].link, BoneLink::Joint { .. }) && !scene.bones[b].removed => Some(b),
        _ => None,
    }
}

/// Where a joint turns: the shared frame of `Part0 * C0` and `Part1 * C1`.
pub fn pivot(scene: &Scene, bone: BoneId) -> Option<Xf> {
    match scene.bones[bone].link {
        BoneLink::Joint { part0, c0, .. } => Some(scene.parts[part0].world * c0),
        _ => None,
    }
}

/// Every instance under the model's top, in the order the model lists them, which is how
/// `Bone::source` and `Attachment::source` count.
pub fn model_order(dom: &WeakDom) -> Vec<Ref> {
    let mut out = Vec::new();
    let mut stack: Vec<Ref> = dom.root().children().iter().rev().copied().collect();
    while let Some(r) = stack.pop() {
        if let Some(inst) = dom.get_by_ref(r) {
            out.push(r);
            stack.extend(inst.children().iter().rev().copied());
        }
    }
    out
}

/// Whether an instance is a part, as import decides.
pub fn is_part(dom: &WeakDom, r: Ref) -> bool {
    dom.get_by_ref(r).is_some_and(|i| {
        i.class != "Terrain" && i.properties.contains_key(&ustr::ustr("CFrame")) && i.properties.contains_key(&ustr::ustr("Size"))
    })
}

/// A rig's parts in the model, in the same order as `Rig::parts`.
pub fn model_parts(dom: &WeakDom, order: &[Ref]) -> Vec<Ref> {
    order.iter().copied().filter(|r| is_part(dom, *r)).collect()
}

/// Gets a rig ready for rigging, once: notes what holds each part as the file had it, reads
/// its Attachments, and matches each joint to its instance in the model. Projects from before
/// rigging get this the first time the Rigging tab opens.
pub fn prepare(scene: &mut Scene, rig: RigId) {
    if scene.rigs[rig].prepared || scene.rigs[rig].kind.is_single() {
        return;
    }
    scene.rigs[rig].prepared = true;
    for part in scene.rigs[rig].parts.clone() {
        scene.parts[part].file_parent = holder(scene, part);
    }
    let Some(model) = scene.rigs[rig].model.clone() else {
        return;
    };
    let Ok(dom) = rbx_binary::from_reader(model.as_slice()) else {
        return;
    };
    let order = model_order(&dom);
    let parts = model_parts(&dom, &order);
    if parts.len() != scene.rigs[rig].parts.len() {
        return;
    }
    let part_of = |r: Ref| parts.iter().position(|p| *p == r).map(|i| scene.rigs[rig].parts[i]);
    let mut attachments = Vec::new();
    for (index, r) in order.iter().enumerate() {
        let Some(inst) = dom.get_by_ref(*r) else {
            continue;
        };
        if inst.class == "Attachment" {
            let Some(part) = part_of(inst.parent()) else {
                continue;
            };
            let local = crate::import::cframe_prop(inst, "CFrame").unwrap_or(Xf::IDENTITY);
            attachments.push(Attachment { name: inst.name.clone(), part, local, removed: false, source: Some(index) });
        }
    }
    scene.attachments.extend(attachments);
    // A joint is its instance when they join the same two parts with the same name.
    let mut taken = HashSet::new();
    for bone in scene.rigs[rig].bones.clone() {
        let BoneLink::Joint { part0, part1, .. } = scene.bones[bone].link else {
            continue;
        };
        let name = scene.bones[bone].name.clone();
        let found = order.iter().enumerate().find(|(index, r)| {
            !taken.contains(index)
                && dom.get_by_ref(**r).is_some_and(|inst| {
                    inst.name == name && joint_parts(&dom, inst).is_some_and(|(a, b)| part_of(a) == Some(part0) && part_of(b) == Some(part1))
                })
        });
        if let Some((index, _)) = found {
            taken.insert(index);
            scene.bones[bone].source = Some(index);
        }
    }
}

/// The two parts a joint instance joins, from `Part0` and `Part1`, or from its attachments'
/// parts for a constraint.
pub fn joint_parts(dom: &WeakDom, inst: &rbx_dom_weak::Instance) -> Option<(Ref, Ref)> {
    use rbx_dom_weak::types::Variant;
    let reference = |name: &str| match inst.properties.get(&ustr::ustr(name)) {
        Some(Variant::Ref(r)) if r.is_some() => Some(*r),
        _ => None,
    };
    if let (Some(a), Some(b)) = (reference("Part0"), reference("Part1")) {
        return Some((a, b));
    }
    let a = dom.get_by_ref(reference("Attachment0")?)?.parent();
    let b = dom.get_by_ref(reference("Attachment1")?)?.parent();
    Some((a, b))
}

/// Joins each child to `parent` with a Motor6D. The pivot starts at the child's middle. A
/// child already on a joint is moved onto `parent`, keeping its pivot and its keys.
pub fn join(scene: &mut Scene, parent: PartId, children: &[PartId]) -> Result<Vec<BoneId>, String> {
    let rig = scene.parts[parent].rig;
    for &child in children {
        if child == parent {
            continue;
        }
        if scene.parts[child].rig != rig {
            return Err("Only parts from the same model can be joined.".to_string());
        }
        if hangs_from(scene, parent, child) {
            return Err(format!("{} already hangs from {}, so joining them would make a loop.", scene.parts[parent].name, scene.parts[child].name));
        }
    }
    prepare(scene, rig);
    let mut joints = Vec::new();
    for &child in children {
        if child == parent {
            continue;
        }
        let (wp, wc) = (scene.parts[parent].world, scene.parts[child].world);
        match joint_of(scene, child) {
            Some(bone) => {
                if let BoneLink::Joint { part0, c0, c1, .. } = &mut scene.bones[bone].link {
                    *part0 = parent;
                    *c0 = wp.inverse() * wc * *c1;
                }
                joints.push(bone);
            }
            None => {
                let link = BoneLink::Joint { part0: parent, part1: child, c0: wp.inverse() * wc, c1: Xf::IDENTITY };
                let mut bone = Bone::new(scene.parts[child].name.clone(), BoneKind::Motor6D, link, None);
                bone.rig = rig;
                let id = scene.bones.len();
                scene.bones.push(bone);
                scene.rigs[rig].bones.push(id);
                scene.parts[child].drive = Drive::Bone(id);
                joints.push(id);
            }
        }
    }
    if scene.rigs[rig].kind == RigKind::Static {
        scene.rigs[rig].kind = RigKind::Custom;
    }
    scene.rigs[rig].rigged = true;
    rebuild(scene, rig);
    Ok(joints)
}

/// Takes a joint out and leaves its part standing where it is now, on its own. Whatever
/// hangs from that part stays on it.
pub fn detach(scene: &mut Scene, bone: BoneId) {
    let BoneLink::Joint { part1, .. } = scene.bones[bone].link else {
        return;
    };
    let rig = scene.bones[bone].rig;
    prepare(scene, rig);
    let world = scene.parts[part1].world;
    scene.bones[bone].removed = true;
    if scene.parts[part1].drive == Drive::Bone(bone) {
        scene.parts[part1].drive = Drive::Root;
        scene.parts[part1].file_cframe = scene.rigs[rig].placement.inverse() * world;
    }
    scene.rigs[rig].rigged = true;
    rebuild(scene, rig);
}

/// Deletes one part and the joint holding it. Parts jointed to it stay, standing on their
/// own. Returns the joints that went, so their keys can go too.
pub fn delete_part(scene: &mut Scene, part: PartId) -> Vec<BoneId> {
    let rig = scene.parts[part].rig;
    prepare(scene, rig);
    let mut gone = Vec::new();
    let below: Vec<BoneId> = (0..scene.bones.len())
        .filter(|b| !scene.bones[*b].removed && matches!(scene.bones[*b].link, BoneLink::Joint { part0, .. } if part0 == part))
        .collect();
    for bone in below {
        detach(scene, bone);
    }
    if let Some(bone) = joint_of(scene, part) {
        scene.bones[bone].removed = true;
        gone.push(bone);
    }
    scene.parts[part].removed = true;
    scene.rigs[rig].rigged = true;
    rebuild(scene, rig);
    gone
}

/// Moves where a joint turns without moving either part.
pub fn set_pivot(scene: &mut Scene, bone: BoneId, world: Xf) {
    let BoneLink::Joint { part0, part1, .. } = scene.bones[bone].link else {
        return;
    };
    let (w0, w1) = (scene.parts[part0].world, scene.parts[part1].world);
    if let BoneLink::Joint { c0, c1, .. } = &mut scene.bones[bone].link {
        *c0 = w0.inverse() * world;
        *c1 = w1.inverse() * world;
    }
    scene.rigs[scene.bones[bone].rig].rigged = true;
    scene.evaluate();
}

/// Moves a part at rest. Its joint's pivot comes with it, and everything below follows.
pub fn set_part(scene: &mut Scene, part: PartId, world: Xf) {
    let rig = scene.parts[part].rig;
    match scene.parts[part].drive {
        Drive::Root => scene.parts[part].file_cframe = scene.rigs[rig].placement.inverse() * world,
        Drive::Bone(bone) => {
            let BoneLink::Joint { part0, .. } = scene.bones[bone].link else {
                return;
            };
            let w0 = scene.parts[part0].world;
            if let BoneLink::Joint { c0, c1, .. } = &mut scene.bones[bone].link {
                *c0 = w0.inverse() * world * *c1;
            }
        }
        Drive::Rigid { parent, .. } => {
            let w = scene.parts[parent].world;
            scene.parts[part].drive = Drive::Rigid { parent, offset: w.inverse() * world };
        }
    }
    scene.rigs[rig].rigged = true;
    scene.evaluate();
}

/// Adds an Attachment at the middle of a part, named so it doesn't clash on that part.
pub fn add_attachment(scene: &mut Scene, part: PartId) -> usize {
    prepare(scene, scene.parts[part].rig);
    let taken = |name: &str| scene.attachments.iter().any(|a| !a.removed && a.part == part && a.name == name);
    let mut name = "Attachment".to_string();
    let mut number = 1;
    while taken(&name) {
        number += 1;
        name = format!("Attachment{number}");
    }
    scene.attachments.push(Attachment { name, part, local: Xf::IDENTITY, removed: false, source: None });
    scene.rigs[scene.parts[part].rig].rigged = true;
    scene.attachments.len() - 1
}

pub fn attachment_frame(scene: &Scene, index: usize) -> Xf {
    let a = &scene.attachments[index];
    scene.parts[a.part].world * a.local
}

pub fn set_attachment_frame(scene: &mut Scene, index: usize, world: Xf) {
    let part = scene.attachments[index].part;
    scene.attachments[index].local = scene.parts[part].world.inverse() * world;
    scene.rigs[scene.parts[part].rig].rigged = true;
}

/// Whether an attachment is still there, with its part.
pub fn attachment_is_live(scene: &Scene, index: usize) -> bool {
    let a = &scene.attachments[index];
    !a.removed && !scene.part_is_removed(a.part)
}

/// Sorts parts so each comes after what holds it, gives each joint the bone above it as its
/// parent, and works out drawn lengths again, after joints change.
fn rebuild(scene: &mut Scene, rig: RigId) {
    let in_order: HashSet<PartId> = scene.part_order.iter().copied().collect();
    let mut placed = HashSet::new();
    let mut order = Vec::with_capacity(scene.part_order.len());
    for part in scene.part_order.clone() {
        let mut chain = vec![part];
        let mut at = holder(scene, part);
        while let Some(p) = at {
            if placed.contains(&p) || chain.contains(&p) || !in_order.contains(&p) {
                break;
            }
            chain.push(p);
            at = holder(scene, p);
        }
        for p in chain.into_iter().rev() {
            if placed.insert(p) {
                order.push(p);
            }
        }
    }
    scene.part_order = order;
    for bone in scene.rigs[rig].bones.clone() {
        if let BoneLink::Joint { part0, .. } = scene.bones[bone].link {
            scene.bones[bone].parent = scene.part_owner(part0).filter(|p| *p != bone);
        }
    }
    let bones = scene.rigs[rig].bones.clone();
    for &bone in &bones {
        scene.bones[bone].children.clear();
    }
    for &bone in &bones {
        if let Some(parent) = scene.bones[bone].parent {
            scene.bones[parent].children.push(bone);
        }
    }
    scene.evaluate();
    // Drawn lengths are worked out at rest, and only for this rig, since cameras and
    // controls have their own.
    let mut rest = scene.clone();
    rest.at_rest.0 = true;
    rest.evaluate();
    rest.compute_tails();
    for &bone in &bones {
        scene.bones[bone].tail_local = rest.bones[bone].tail_local;
        scene.bones[bone].link_to_parent_tail = rest.bones[bone].link_to_parent_tail;
    }
}

/// Writes a rig's rigging into its model, read again from `Rig::model`: every part where it
/// stands at rest, each joint's parts and offsets, joints and attachments added or deleted in
/// Kaye, and parts deleted. A file's other joint that held a part now held another way, such
/// as a weld replaced by a new Motor6D, goes too, so the two don't fight.
pub fn write_into(dom: &mut WeakDom, scene: &Scene, rig: RigId) -> Result<(), String> {
    use rbx_dom_weak::InstanceBuilder;
    use rbx_dom_weak::types::Variant;
    use ustr::ustr;

    use crate::export::cframe;

    let order = model_order(dom);
    let parts = model_parts(dom, &order);
    let ours = &scene.rigs[rig].parts;
    if parts.len() != ours.len() {
        return Err("This rig's model no longer matches its parts, so its rigging can't be saved into it. Import the rig again.".to_string());
    }
    let part_ref = |part: PartId| ours.iter().position(|p| *p == part).map(|i| parts[i]);
    let rest = scene.rest_pose();
    let mut doomed: Vec<Ref> = Vec::new();

    for (index, &part) in ours.iter().enumerate() {
        if scene.parts[part].removed {
            doomed.push(parts[index]);
        } else if let Some(inst) = dom.get_by_ref_mut(parts[index]) {
            inst.properties.insert(ustr("CFrame"), Variant::CFrame(cframe(rest.parts[part])));
        }
    }

    let sources: HashSet<usize> = scene.rigs[rig].bones.iter().filter_map(|b| scene.bones[*b].source).collect();
    for &bone in &scene.rigs[rig].bones {
        let b = &scene.bones[bone];
        let BoneLink::Joint { part0, part1, c0, c1 } = b.link else {
            continue;
        };
        let (Some(r0), Some(r1)) = (part_ref(part0), part_ref(part1)) else {
            continue;
        };
        match b.source.and_then(|i| order.get(i).copied()) {
            Some(r) if b.removed => doomed.push(r),
            Some(r) => {
                let attachments = match dom.get_by_ref(r) {
                    Some(inst) if inst.class == "AnimationConstraint" => {
                        let at = |name: &str| match inst.properties.get(&ustr(name)) {
                            Some(Variant::Ref(a)) if a.is_some() => Some(*a),
                            _ => None,
                        };
                        Some((at("Attachment0"), at("Attachment1")))
                    }
                    _ => None,
                };
                match attachments {
                    // An AnimationConstraint keeps its offsets in its two attachments.
                    Some((a0, a1)) => {
                        for (attachment, part, offset) in [(a0, r0, c0), (a1, r1, c1)] {
                            if let Some(inst) = attachment.and_then(|a| dom.get_by_ref_mut(a)).filter(|i| i.parent() == part) {
                                inst.properties.insert(ustr("CFrame"), Variant::CFrame(cframe(offset)));
                            }
                        }
                    }
                    None => {
                        if let Some(inst) = dom.get_by_ref_mut(r) {
                            inst.name = b.name.clone();
                            inst.properties.insert(ustr("Part0"), Variant::Ref(r0));
                            inst.properties.insert(ustr("Part1"), Variant::Ref(r1));
                            inst.properties.insert(ustr("C0"), Variant::CFrame(cframe(c0)));
                            inst.properties.insert(ustr("C1"), Variant::CFrame(cframe(c1)));
                        }
                    }
                }
            }
            None if b.removed => {}
            // Inside the part it moves, where R15 rigs keep theirs.
            None => {
                let joint = InstanceBuilder::new("Motor6D")
                    .with_name(b.name.clone())
                    .with_property("Part0", Variant::Ref(r0))
                    .with_property("Part1", Variant::Ref(r1))
                    .with_property("C0", Variant::CFrame(cframe(c0)))
                    .with_property("C1", Variant::CFrame(cframe(c1)));
                dom.insert(r1, joint);
            }
        }
    }

    // The file's own joint to a part's old holder, when the part is held another way now.
    for &part in ours {
        let (now, then) = (holder(scene, part), scene.parts[part].file_parent);
        let (Some(old), true) = (then, now != then) else {
            continue;
        };
        let (Some(a), Some(b)) = (part_ref(part), part_ref(old)) else {
            continue;
        };
        for (index, r) in order.iter().enumerate() {
            if sources.contains(&index) {
                continue;
            }
            let Some(inst) = dom.get_by_ref(*r) else {
                continue;
            };
            if crate::import::is_joint_class(&inst.class) && joint_parts(dom, inst).is_some_and(|(x, y)| (x == a && y == b) || (x == b && y == a)) {
                doomed.push(*r);
            }
        }
    }

    for attachment in scene.attachments.iter().filter(|a| scene.parts[a.part].rig == rig) {
        let Some(on) = part_ref(attachment.part) else {
            continue;
        };
        match attachment.source.and_then(|i| order.get(i).copied()) {
            Some(r) if attachment.removed => doomed.push(r),
            Some(r) => {
                if let Some(inst) = dom.get_by_ref_mut(r) {
                    inst.name = attachment.name.clone();
                    inst.properties.insert(ustr("CFrame"), Variant::CFrame(cframe(attachment.local)));
                }
            }
            None if attachment.removed => {}
            None => {
                let made = InstanceBuilder::new("Attachment").with_name(attachment.name.clone()).with_property("CFrame", Variant::CFrame(cframe(attachment.local)));
                dom.insert(on, made);
            }
        }
    }

    for r in doomed {
        if dom.get_by_ref(r).is_some() {
            dom.destroy(r);
        }
    }
    Ok(())
}
