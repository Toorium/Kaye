//! IK and Unparent, on real R6 and R15 exports and on custom rigs built in code.

use std::path::Path;

use glam::{Quat, Vec3};
use kaye::animation::Animation;
use kaye::history::History;
use kaye::import::{import_dom, import_file};
use kaye::math::Xf;
use kaye::modifiers::{self, IkEnd, ModifierKind};
use kaye::scene::{BoneKind, BoneLink, Scene};
use rbx_dom_weak::{InstanceBuilder, WeakDom};
use rbx_types::{CFrame, Matrix3, Ref, Variant, Vector3};
use ustr::ustr;

fn rig(file: &str) -> Scene {
    import_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(file)).unwrap().scene
}

fn bone(s: &Scene, name: &str) -> usize {
    s.bones.iter().position(|b| b.name == name).unwrap_or_else(|| panic!("no bone {name}"))
}

fn place(s: &mut Scene, control: usize, at: Vec3) {
    let rot = s.bones[control].frame.rot;
    s.set_bone_frame(control, Xf::new(rot, at));
}

fn turned(angle: f32) -> Xf {
    Xf::new(Quat::from_rotation_x(angle), Vec3::ZERO)
}

/// Every part where only `Transform` on each real joint would put it, which is all
/// Roblox can play back.
fn played_in_roblox(scene: &Scene) -> Vec<Xf> {
    let mut plain = scene.clone();
    plain.modifiers.clear();
    for (i, b) in scene.bones.iter().enumerate() {
        plain.bones[i].pose = b.output();
    }
    plain.evaluate();
    plain.parts.iter().map(|p| p.world).collect()
}

fn assert_plays_back(scene: &Scene) {
    for (id, played) in played_in_roblox(scene).into_iter().enumerate() {
        let shown = scene.parts[id].world;
        assert!(shown.pos.distance(played.pos) < 1e-2, "{} is at {} here but {} in Roblox", scene.parts[id].name, shown.pos, played.pos);
        assert!(shown.rot.dot(played.rot).abs() > 0.9995, "{} turned differently in Roblox", scene.parts[id].name);
    }
}

// IK only places the rig it's solving, which has to change nothing.

fn crowd(rigs: usize) -> Scene {
    let mut scene = Scene::default();
    for _ in 0..rigs {
        scene.append(rig("R15Rig.rbxm"));
    }
    scene.evaluate();
    for r in 0..rigs {
        for limb in ["LeftAnkle", "RightAnkle", "LeftWrist", "RightWrist"] {
            let tip = scene.rigs[r].bones.iter().copied().find(|b| scene.bones[*b].name == limb).unwrap();
            modifiers::add_ik(&mut scene, tip);
            let index = scene.modifiers.len() - 1;
            modifiers::add_pole(&mut scene, index);
        }
    }
    scene
}

/// Placing everything again finds nothing left out of date, and every rig matches one
/// that's alone in its scene.
#[test]
fn rigs_solve_the_same_side_by_side() {
    let mut alone = crowd(1);
    let foot = alone.modifiers[0];
    let ModifierKind::Ik(ik) = foot.kind else { panic!() };
    let moved = alone.bones[ik.target].head() + Vec3::new(0.4, 0.5, -0.3);
    place(&mut alone, ik.target, moved);

    let mut scene = crowd(3);
    let size = alone.bones.len();
    let bones_in_rig = scene.rigs[0].bones.len();
    assert_eq!(size, bones_in_rig);
    for r in 0..3 {
        let target = scene.rigs[r].bones[ik.target];
        place(&mut scene, target, moved);
    }
    for r in 0..3 {
        for (at, bone) in scene.rigs[r].bones.iter().enumerate() {
            let (a, b) = (scene.bones[*bone].frame, alone.bones[alone.rigs[0].bones[at]].frame);
            assert!(a.pos.distance(b.pos) < 1e-4 && a.rot.dot(b.rot).abs() > 0.99999, "{} in rig {r}", scene.bones[*bone].name);
        }
        for (at, part) in scene.rigs[r].parts.iter().enumerate() {
            let (a, b) = (scene.parts[*part].world, alone.parts[alone.rigs[0].parts[at]].world);
            assert!(a.pos.distance(b.pos) < 1e-4, "{} in rig {r}", scene.parts[*part].name);
        }
    }
    assert_plays_back(&scene);
}

/// A camera has a rig of its own with no IK to place it again, so it only sees the IK
/// result on a bone it follows if solving places more than the bone's own rig.
#[test]
fn something_following_another_rigs_ik_still_follows() {
    let mut scene = crowd(1);
    let ModifierKind::Ik(ik) = scene.modifiers[0].kind else { panic!() };
    let thigh = modifiers::chain_of(&scene, scene.modifiers[0].bone, &ik)[0];

    let camera = scene.add_camera(Xf::IDENTITY, 70.0);
    scene.bones[camera].pose = Xf::IDENTITY;
    modifiers::set_follow(&mut scene, camera, Some(thigh));
    let held = scene.bones[thigh].frame.inverse() * scene.bones[camera].frame;

    let moved = scene.bones[ik.target].head() + Vec3::new(0.5, 0.6, -0.4);
    place(&mut scene, ik.target, moved);
    let expected = scene.bones[thigh].frame * held;
    let at = scene.bones[camera].frame;
    assert!(at.pos.distance(expected.pos) < 1e-3, "at {} but should be at {}", at.pos, expected.pos);
}

/// An Unparent modifier added straight onto `Scene::modifiers`, as undo and loading do,
/// still counts once the scene is evaluated.
#[test]
fn unparent_restored_by_undo_still_holds() {
    let mut scene = rig("R15Rig.rbxm");
    let hand = bone(&scene, "RightWrist");
    let before = scene.rig_state();
    modifiers::add_unparent(&mut scene, hand);
    let after = scene.rig_state();
    let held = scene.bones[hand].frame;

    scene.restore_rig_state(&before);
    scene.evaluate();
    scene.restore_rig_state(&after);
    let shoulder = bone(&scene, "RightShoulder");
    scene.bones[shoulder].pose = turned(0.9);
    scene.evaluate();
    assert!(scene.bones[hand].frame.pos.distance(held.pos) < 1e-4, "the hand followed its parent");
}

// Custom rigs, built the way a Motor6D or skinned rig arrives from Studio.

fn at(y: f32) -> CFrame {
    CFrame::new(Vector3::new(0.0, y, 0.0), Matrix3::identity())
}

/// A rope of parts hanging down from an anchored root, each joined by a Motor6D.
fn rope(links: usize) -> Scene {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let model = dom.insert(root, InstanceBuilder::new("Model").with_name("Rope"));
    let part = |name: String, y: f32| {
        InstanceBuilder::new("Part")
            .with_name(name)
            .with_property("CFrame", at(y))
            .with_property("Size", Vector3::new(0.5, 1.0, 0.5))
    };
    let mut previous = dom.insert(model, part("Anchor".to_string(), 20.0));
    let mut parts: Vec<Ref> = Vec::new();
    for i in 0..links {
        let link = dom.insert(model, part(format!("Link{i}"), 19.0 - i as f32));
        let joint = dom.insert(
            previous,
            InstanceBuilder::new("Motor6D")
                .with_name(format!("Joint{i}"))
                .with_property("C0", at(-0.5))
                .with_property("C1", at(0.5)),
        );
        for (name, to) in [("Part0", previous), ("Part1", link)] {
            dom.get_by_ref_mut(joint).unwrap().properties.insert(ustr(name), Variant::Ref(to));
        }
        parts.push(link);
        previous = link;
    }
    let imported = import_dom(&dom, "rope.rbxm").unwrap();
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    imported.scene
}

/// A tail of skinned `Bone` instances inside one mesh.
fn skinned_tail(links: usize) -> Scene {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let model = dom.insert(root, InstanceBuilder::new("Model").with_name("Tail"));
    let body = dom.insert(
        model,
        InstanceBuilder::new("MeshPart")
            .with_name("Body")
            .with_property("CFrame", at(10.0))
            .with_property("Size", Vector3::new(1.0, 1.0, 1.0)),
    );
    let mut parent = body;
    for i in 0..links {
        parent = dom.insert(parent, InstanceBuilder::new("Bone").with_name(format!("Tail{i}")).with_property("CFrame", at(-1.0)));
    }
    import_dom(&dom, "tail.rbxm").unwrap().scene
}

#[test]
fn r15_leg_reaches_its_target_and_the_knee_bends_forward() {
    let mut s = rig("R15Rig.rbxm");
    let (hip, knee, ankle) = (bone(&s, "RightHip"), bone(&s, "RightKnee"), bone(&s, "RightAnkle"));
    let foot = s.bones[ankle].head();
    let (knee_rest, keyed) = (s.bones[knee].head(), s.bones[knee].pose);
    let thigh = s.bones[hip].head().distance(knee_rest);
    let shin = knee_rest.distance(foot);

    let target = modifiers::add_ik(&mut s, ankle);
    let ModifierKind::Ik(ik) = s.modifiers[0].kind else { panic!() };
    assert_eq!((ik.end, ik.chain), (IkEnd::Joint, 2), "a foot bends the knee and hip");
    assert!(s.bones[ankle].head().distance(foot) < 1e-3, "adding IK moved the foot");
    assert!(s.bones[knee].head().distance(knee_rest) < 0.1, "adding IK moved the knee");

    let goal = foot + Vec3::new(0.0, 0.9, -0.2);
    place(&mut s, target, goal);
    assert!(s.bones[ankle].head().distance(goal) < 1e-3, "foot ended {} from the target", s.bones[ankle].head().distance(goal));
    assert!(s.bones[knee].head().z < knee_rest.z - 0.3, "knee should come forward, toward -Z: {}", s.bones[knee].head());
    assert!((s.bones[hip].head().distance(s.bones[knee].head()) - thigh).abs() < 1e-3, "the thigh stretched");
    assert!((s.bones[knee].head().distance(s.bones[ankle].head()) - shin).abs() < 1e-3, "the shin stretched");
    assert!(s.bones[knee].pose.approx_eq(keyed), "IK must leave the keyed pose alone");
    assert_plays_back(&s);
}

#[test]
fn r15_arm_keeps_its_built_in_bend_and_every_limb_works() {
    let mut s = rig("R15Rig.rbxm");
    let (elbow, wrist) = (bone(&s, "LeftElbow"), bone(&s, "LeftWrist"));
    let hand = s.bones[wrist].head();
    let rest = s.bones[elbow].head();
    let target = modifiers::add_ik(&mut s, wrist);
    assert!(s.bones[elbow].head().distance(rest) < 1e-3, "adding IK moved the elbow");
    place(&mut s, target, hand + Vec3::new(0.0, 0.8, -0.3));
    assert!(s.bones[elbow].head().x < rest.x - 0.1, "the left elbow should push outward: {}", s.bones[elbow].head());

    for end in ["RightWrist", "LeftAnkle", "RightAnkle"] {
        let end = bone(&s, end);
        let start = s.bones[end].head();
        let target = modifiers::add_ik(&mut s, end);
        let goal = start + Vec3::new(0.2, 0.5, -0.2);
        place(&mut s, target, goal);
        assert!(s.bones[end].head().distance(goal) < 1e-3, "{} missed", s.bones[end].name);
    }
    assert_plays_back(&s);
}

#[test]
fn a_planted_foot_keeps_the_targets_rotation_while_the_body_moves() {
    let mut s = rig("R15Rig.rbxm");
    let ankle = bone(&s, "LeftAnkle");
    let (foot, spin) = (s.bones[ankle].head(), s.bones[ankle].frame.rot);
    modifiers::add_ik(&mut s, ankle);

    // Crouching: the whole body drops, and the foot has to stay where it is.
    s.set_bone_pose(bone(&s, "Root"), Xf::new(Quat::IDENTITY, Vec3::new(0.0, -0.8, 0.0)));
    assert!(s.bones[ankle].head().distance(foot) < 1e-3, "the foot slid");
    assert!(s.bones[ankle].frame.rot.dot(spin).abs() > 0.9999, "the foot tilted");
    assert_plays_back(&s);
}

#[test]
fn r6_limbs_are_one_bone_so_they_aim_at_the_target() {
    let mut s = rig("R6Rig.rbxm");
    let shoulder = bone(&s, "Right Shoulder");
    let pivot = s.bones[shoulder].head();
    let length = s.bones[shoulder].tail().distance(pivot);

    let target = modifiers::add_ik(&mut s, shoulder);
    let ModifierKind::Ik(ik) = s.modifiers[0].kind else { panic!() };
    assert_eq!((ik.end, ik.chain), (IkEnd::Tip, 1), "R6 has no elbow to bend");
    assert!(s.bones[target].head().distance(s.bones[shoulder].tail()) < 1e-3, "the target starts on the hand");
    assert!(!modifiers::locked_by_ik(&s, shoulder), "the arm has to stay grabbable");
    assert_eq!(modifiers::control_of(&s, shoulder), target, "and grabbing it moves the target");

    let goal = pivot + Vec3::new(1.0, 1.0, -1.0).normalize() * length;
    place(&mut s, target, goal);
    assert!(s.bones[shoulder].tail().distance(goal) < 1e-3, "the arm should reach a point it can touch");
    assert!(s.bones[shoulder].head().distance(pivot) < 1e-3, "the shoulder has to stay in its socket");

    // Out of reach, it points straight at the target and can't stretch.
    let far = pivot + Vec3::new(-3.0, 2.0, 1.0) * 4.0;
    place(&mut s, target, far);
    let aim = (s.bones[shoulder].tail() - pivot).normalize();
    assert!(aim.dot((far - pivot).normalize()) > 0.9999, "aim was off: {aim}");
    assert!((s.bones[shoulder].tail().distance(pivot) - length).abs() < 1e-3);

    for limb in ["Left Shoulder", "Left Hip", "Right Hip"] {
        let limb = bone(&s, limb);
        let (pivot, reach) = (s.bones[limb].head(), s.bones[limb].tail().distance(s.bones[limb].head()));
        let target = modifiers::add_ik(&mut s, limb);
        let goal = pivot + Vec3::new(0.3, -0.7, -0.5).normalize() * reach;
        place(&mut s, target, goal);
        assert!(s.bones[limb].tail().distance(goal) < 1e-3, "{} missed", s.bones[limb].name);
    }
    assert_plays_back(&s);
}

/// Lowering the body with IK on the legs: the feet stay exactly where they were, all the
/// way down and past the point where the hip reaches the foot, and never jump.
#[test]
fn r6_legs_stay_planted_as_the_body_lowers() {
    let mut s = rig("R6Rig.rbxm");
    let root = bone(&s, "RootJoint");
    let legs = [bone(&s, "Left Hip"), bone(&s, "Right Hip")];
    let targets = legs.map(|leg| modifiers::add_ik(&mut s, leg));
    let planted = targets.map(|t| s.bones[t].head());
    let start_turn = legs.map(|leg| s.bones[leg].frame.rot);

    let mut previous: Vec<Xf> = legs.iter().map(|l| s.bones[*l].frame).collect();
    // Past 4 studs the hip is a whole leg below the foot, which is out of reach.
    for step in 1..=40 {
        let drop = step as f32 * 0.1;
        let world = s.bones[root].base.rot.inverse() * Vec3::new(0.0, -drop, 0.0);
        s.bones[root].pose = Xf::new(Quat::IDENTITY, world);
        s.evaluate();
        for (i, leg) in legs.iter().enumerate() {
            let tail = s.bones[*leg].tail();
            assert!(tail.distance(planted[i]) < 1e-3, "foot {i} left its target by {} at drop {drop}", tail.distance(planted[i]));
            // Down to a little past the hip reaching the foot, which is further than a
            // real crouch goes, the leg has to stay upright and not swing about.
            let now = s.bones[*leg].frame;
            if drop <= 2.5 {
                assert!(now.rot.dot(previous[i].rot).abs() > 0.99, "leg {i} jumped at drop {drop}");
                assert!(now.rot.dot(start_turn[i]).abs() > 0.9, "leg {i} tipped over at drop {drop}");
            }
            previous[i] = now;
        }
    }
    assert_plays_back(&s);
}

/// Lifting a foot straight up leans the leg forward, as an R15 knee goes, and not out
/// to the side, even though the hip joint sits off to the side of the leg.
#[test]
fn a_lifted_r6_foot_leans_the_leg_forward() {
    for (hip, pole) in [("Left Hip", false), ("Right Hip", false), ("Left Hip", true)] {
        let mut s = rig("R6Rig.rbxm");
        let leg = bone(&s, hip);
        let root_part = s.parts.iter().find(|p| p.name == "HumanoidRootPart").unwrap().world;
        let (forward, side) = (root_part.rot * Vec3::NEG_Z, root_part.rot * Vec3::X);
        let at_rest = s.bones[leg].tail() - s.bones[leg].head();
        let target = modifiers::add_ik(&mut s, leg);
        if pole {
            modifiers::add_pole(&mut s, 0);
        }
        let goal = s.bones[target].head() + Vec3::Y * 1.0;
        place(&mut s, target, goal);

        let tail = s.bones[leg].tail();
        assert!(tail.distance(goal) < 1e-3, "{hip}: the foot left its target");
        let now = s.bones[leg].tail() - s.bones[leg].head();
        assert!(now.dot(forward) > 0.3, "{hip}, pole {pole}: only leaned {} forward", now.dot(forward));
        assert!((now.dot(side) - at_rest.dot(side)).abs() < 0.1, "{hip}, pole {pole}: went sideways by {}", now.dot(side) - at_rest.dot(side));
        assert_plays_back(&s);
    }
}

/// Lean sets how hard the leg tips forward, and the foot stays planted at any setting.
#[test]
fn lean_sets_how_far_a_one_bone_limb_tips() {
    let tipped = |lean: f32| {
        let mut s = rig("R6Rig.rbxm");
        let leg = bone(&s, "Left Hip");
        let forward = s.parts.iter().find(|p| p.name == "HumanoidRootPart").unwrap().world.rot * Vec3::NEG_Z;
        let target = modifiers::add_ik(&mut s, leg);
        if let ModifierKind::Ik(ik) = &mut s.modifiers[0].kind {
            ik.lean = lean;
        }
        let goal = s.bones[target].head() + Vec3::Y;
        place(&mut s, target, goal);
        assert!(s.bones[leg].tail().distance(goal) < 1e-3, "lean {lean}: the foot left its target");
        assert_plays_back(&s);
        (s.bones[leg].tail() - s.bones[leg].head()).dot(forward)
    };
    let (none, some, lots) = (tipped(0.0), tipped(0.5), tipped(2.0));
    assert!(none.abs() < 0.05, "no lean stays upright, got {none}");
    assert!(none < some && some < lots, "{none}, {some}, {lots}");
}

/// A target moved across the limb still swings it that way, and one out of reach
/// straight above still raises it, which is how an arm gets lifted.
#[test]
fn one_bone_limbs_still_swing_and_raise() {
    let mut s = rig("R6Rig.rbxm");
    let arm = bone(&s, "Right Shoulder");
    let joint = s.bones[arm].head();
    let length = joint.distance(s.bones[arm].tail());
    let target = modifiers::add_ik(&mut s, arm);
    let rest = s.bones[target].head();

    let goal = rest + Vec3::X * 1.0 * (rest.x - joint.x).signum();
    place(&mut s, target, goal);
    assert!(s.bones[arm].tail().distance(goal) < 1e-3 || s.bones[arm].head().distance(joint) < 1e-3);

    place(&mut s, target, joint + Vec3::Y * length * 1.5);
    assert!(s.bones[arm].head().distance(joint) < 1e-4);
    assert!((s.bones[arm].tail() - (joint + Vec3::Y * length)).length() < 1e-3, "the arm points straight up");
}

/// Adding IK moves nothing, a leg at full stretch still only points, and a part-way
/// influence blends back to the keyed pose.
#[test]
fn one_bone_limbs_blend_and_keep_their_length_out_of_reach() {
    let mut s = rig("R6Rig.rbxm");
    let leg = bone(&s, "Left Hip");
    let (joint, foot) = (s.bones[leg].head(), s.bones[leg].tail());
    let target = modifiers::add_ik(&mut s, leg);
    assert!(s.bones[leg].head().distance(joint) < 1e-4 && s.bones[leg].tail().distance(foot) < 1e-4, "adding IK moved the leg");

    let length = joint.distance(foot);
    place(&mut s, target, joint + Vec3::new(length * 2.0, 0.0, 0.0));
    assert!(s.bones[leg].head().distance(joint) < 1e-4, "out of reach, the joint stays put");
    assert!((s.bones[leg].tail() - (joint + Vec3::X * length)).length() < 1e-3, "and the leg points at the target");

    place(&mut s, target, foot + Vec3::Y * 0.5);
    assert!(s.bones[leg].tail().distance(foot + Vec3::Y * 0.5) < 1e-3);
    s.bones[target].value = 0.0;
    s.evaluate();
    assert!(s.bones[leg].tail().distance(foot) < 1e-3, "at no influence it's back where it was keyed");
    s.bones[target].value = 0.5;
    s.evaluate();
    let halfway = s.bones[leg].tail().y;
    assert!(halfway > foot.y + 0.1 && halfway < foot.y + 0.4, "halfway is between the two, got {}", halfway - foot.y);
    assert_plays_back(&s);
}

#[test]
fn r6_can_also_bend_through_the_torso_with_a_longer_chain() {
    let mut s = rig("R6Rig.rbxm");
    let shoulder = bone(&s, "Right Shoulder");
    assert_eq!(modifiers::longest_chain(&s, shoulder, IkEnd::Tip), 2);
    let target = modifiers::add_ik(&mut s, shoulder);
    if let ModifierKind::Ik(ik) = &mut s.modifiers[0].kind {
        ik.chain = 2;
    }
    let goal = s.bones[shoulder].tail() + Vec3::new(0.3, 0.5, -0.4);
    place(&mut s, target, goal);
    assert!(s.bones[shoulder].tail().distance(goal) < 1e-3);
    assert_plays_back(&s);
}

#[test]
fn custom_motor6d_chains_of_any_length_reach_the_target() {
    for links in [2, 3, 5, 8] {
        let mut s = rope(links);
        let tip = bone(&s, &format!("Joint{}", links - 1));
        let (root, start) = (s.bones[0].head(), s.bones[tip].head());
        let target = modifiers::add_ik(&mut s, tip);
        let ModifierKind::Ik(ik) = s.modifiers[0].kind else { panic!() };
        assert_eq!(ik.chain, links - 1, "every link above the tip bends");
        assert!(s.bones[tip].head().distance(start) < 1e-3, "{links} links: adding IK moved the tip");
        assert!(s.bones[links / 2].head().x.abs() < 1e-3, "{links} links: adding IK bent a straight chain");
        let lengths: Vec<f32> = (0..links - 1).map(|i| s.bones[i].head().distance(s.bones[i + 1].head())).collect();

        // One link can only swing around its full length. Longer chains can pull in.
        let reach = (links - 1) as f32 * if links == 2 { 1.0 } else { 0.8 };
        let goal = root + Vec3::new(0.6, -0.8, 0.0) * reach;
        place(&mut s, target, goal);
        assert!(s.bones[tip].head().distance(goal) < 1e-2, "{links} links: ended {} away", s.bones[tip].head().distance(goal));
        for i in 0..links - 1 {
            let now = s.bones[i].head().distance(s.bones[i + 1].head());
            assert!((now - lengths[i]).abs() < 1e-3, "{links} links: link {i} stretched");
        }
        assert_plays_back(&s);
    }
}

#[test]
fn custom_rigs_can_pick_any_chain_and_either_end() {
    let mut s = rope(6);
    let tip = bone(&s, "Joint5");
    assert_eq!(modifiers::longest_chain(&s, tip, IkEnd::Joint), 5);
    assert_eq!(modifiers::longest_chain(&s, tip, IkEnd::Tip), 6);
    let target = modifiers::add_ik(&mut s, tip);
    if let ModifierKind::Ik(ik) = &mut s.modifiers[0].kind {
        ik.chain = 3;
    }
    let held = s.bones[bone(&s, "Joint1")].head();
    let goal = s.bones[tip].head() + Vec3::new(1.2, 0.8, 0.0);
    place(&mut s, target, goal);
    assert!(s.bones[bone(&s, "Joint1")].head().distance(held) < 1e-3, "bones above the chain stay put");

    if let ModifierKind::Ik(ik) = &mut s.modifiers[0].kind {
        ik.end = IkEnd::Tip;
    }
    s.evaluate();
    assert!(s.bones[tip].tail().distance(s.bones[target].head()) < 1e-2, "the tip of the bone reaches instead");
    assert_plays_back(&s);
}

#[test]
fn skinned_bone_chains_solve_too() {
    let mut s = skinned_tail(5);
    let tip = bone(&s, "Tail4");
    assert_eq!(s.bones[tip].kind, BoneKind::Skinned);
    let target = modifiers::add_ik(&mut s, tip);
    let goal = s.bones[tip].head() + Vec3::new(1.5, 1.0, 0.5);
    place(&mut s, target, goal);
    assert!(s.bones[tip].head().distance(goal) < 1e-2, "ended {} away", s.bones[tip].head().distance(goal));
}

#[test]
fn a_pole_swings_the_knee_around_and_twist_does_too() {
    let mut s = rig("R15Rig.rbxm");
    let (knee, ankle) = (bone(&s, "RightKnee"), bone(&s, "RightAnkle"));
    let foot = s.bones[ankle].head();
    let target = modifiers::add_ik(&mut s, ankle);
    place(&mut s, target, foot + Vec3::new(0.0, 0.8, 0.0));
    let forward = s.bones[knee].head();

    if let ModifierKind::Ik(ik) = &mut s.modifiers[0].kind {
        ik.twist = 90.0;
    }
    s.evaluate();
    let twisted = s.bones[knee].head();
    assert!(twisted.distance(forward) > 0.3, "twist should swing the knee");
    assert!(s.bones[ankle].head().distance(foot + Vec3::new(0.0, 0.8, 0.0)) < 1e-3, "and the foot stays on target");

    if let ModifierKind::Ik(ik) = &mut s.modifiers[0].kind {
        ik.twist = 0.0;
    }
    s.evaluate();
    modifiers::add_pole(&mut s, 0);
    assert!(s.bones[knee].head().distance(forward) < 1e-3, "adding a pole changes nothing yet");
    let ModifierKind::Ik(ik) = s.modifiers[0].kind else { panic!() };
    let pole = ik.pole.unwrap();
    let side = s.bones[knee].head() + Vec3::new(5.0, 0.0, 0.0);
    place(&mut s, pole, side);
    assert!(s.bones[knee].head().x > forward.x + 0.3, "the knee should follow the pole");
}

#[test]
fn a_pole_moves_with_the_body_so_the_knee_doesnt_flip_when_it_turns() {
    let mut s = rig("R15Rig.rbxm");
    let (root, knee, ankle) = (bone(&s, "Root"), bone(&s, "RightKnee"), bone(&s, "RightAnkle"));
    let target = modifiers::add_ik(&mut s, ankle);
    let lifted = s.bones[ankle].head() + Vec3::new(0.0, 0.8, 0.0);
    place(&mut s, target, lifted);
    modifiers::add_pole(&mut s, 0);
    let ModifierKind::Ik(ik) = s.modifiers[0].kind else { panic!() };
    let pole = ik.pole.unwrap();
    assert!(matches!(s.bones[pole].link, BoneLink::Free { follow: Some(b), .. } if b == root), "a new pole follows the bone the chain hangs from");

    // Turn the whole character around, taking the foot's target along.
    modifiers::set_follow(&mut s, target, Some(root));
    let before = s.bones[root].frame.inverse().point(s.bones[knee].head());
    s.set_bone_pose(root, Xf::new(Quat::from_rotation_y(2.6), Vec3::new(3.0, 0.0, -2.0)));
    let after = s.bones[root].frame.inverse().point(s.bones[knee].head());
    assert!(before.distance(after) < 1e-3, "the knee should turn with the body, not flip: {before} then {after}");
    assert_plays_back(&s);
}

#[test]
fn a_pole_left_in_the_world_stays_there() {
    let mut s = rig("R15Rig.rbxm");
    let (root, ankle) = (bone(&s, "Root"), bone(&s, "RightAnkle"));
    modifiers::add_ik(&mut s, ankle);
    modifiers::add_pole(&mut s, 0);
    let ModifierKind::Ik(ik) = s.modifiers[0].kind else { panic!() };
    let pole = ik.pole.unwrap();
    let spot = s.bones[pole].head();
    modifiers::set_follow(&mut s, pole, None);
    assert!(s.bones[pole].head().distance(spot) < 1e-3, "changing what it follows mustn't move it");
    s.set_bone_pose(root, Xf::new(Quat::IDENTITY, Vec3::new(5.0, 0.0, 0.0)));
    assert!(s.bones[pole].head().distance(spot) < 1e-3);
}

#[test]
fn a_following_target_can_still_be_moved_by_hand_and_keyed() {
    let mut s = rig("R15Rig.rbxm");
    let (root, wrist) = (bone(&s, "Root"), bone(&s, "RightWrist"));
    let target = modifiers::add_ik(&mut s, wrist);
    modifiers::set_follow(&mut s, target, Some(root));
    let goal = s.bones[target].head() + Vec3::new(0.2, 0.6, -0.3);
    place(&mut s, target, goal);
    assert!(s.bones[wrist].head().distance(goal) < 1e-3, "the handles still land where they're dragged");

    let shift = Vec3::new(1.0, 0.0, 0.0);
    s.set_bone_pose(root, Xf::new(Quat::IDENTITY, shift));
    assert!(s.bones[wrist].head().distance(goal + shift) < 1e-3, "and the hand comes along with the body");
    assert_plays_back(&s);
}

#[test]
fn undo_puts_back_what_a_control_followed() {
    let mut s = rig("R15Rig.rbxm");
    let (root, ankle) = (bone(&s, "Root"), bone(&s, "RightAnkle"));
    let target = modifiers::add_ik(&mut s, ankle);
    let link = s.bones[target].link;
    let mut animation = Animation::default();
    let mut history = History::default();

    let before = (s.rig_state(), animation.clone());
    modifiers::set_follow(&mut s, target, Some(root));
    history.push_rig("Change follow", before, (s.rig_state(), animation.clone()));
    assert_ne!(s.bones[target].link, link);

    history.undo(&mut s, &mut animation).unwrap();
    assert_eq!(s.bones[target].link, link);
    history.redo(&mut s, &mut animation).unwrap();
    assert!(matches!(s.bones[target].link, BoneLink::Free { follow: Some(b), .. } if b == root));
}

#[test]
fn poles_work_on_custom_chains_of_any_length() {
    for links in [3, 4, 7] {
        let mut s = rope(links);
        let tip = bone(&s, &format!("Joint{}", links - 1));
        let root = s.bones[0].head();
        let target = modifiers::add_ik(&mut s, tip);
        place(&mut s, target, root + Vec3::new(0.0, -0.7, 0.0) * (links - 1) as f32);
        modifiers::add_pole(&mut s, 0);
        let ModifierKind::Ik(ik) = s.modifiers[0].kind else { panic!() };
        let pole = ik.pole.unwrap();
        let middle = links / 2;

        for side in [Vec3::X, Vec3::NEG_X, Vec3::Z] {
            let at = root + Vec3::new(0.0, -2.0, 0.0) + side * 30.0;
            place(&mut s, pole, at);
            let sideways = (s.bones[middle].head() - root).dot(side);
            assert!(sideways > 0.1, "{links} links: the chain should bend toward {side}, got {sideways}");
            assert!(s.bones[tip].head().distance(s.bones[target].head()) < 1e-2, "{links} links: and still reach");
        }
    }
}

#[test]
fn influence_blends_between_ik_and_fk_and_can_be_animated() {
    let mut s = rig("R15Rig.rbxm");
    let (knee, ankle) = (bone(&s, "RightKnee"), bone(&s, "RightAnkle"));
    let fk_foot = s.bones[ankle].head();
    let target = modifiers::add_ik(&mut s, ankle);
    let goal = fk_foot + Vec3::new(0.0, 1.0, -0.4);
    place(&mut s, target, goal);

    s.bones[target].value = 0.0;
    s.evaluate();
    assert!(s.bones[ankle].head().distance(fk_foot) < 1e-3, "no influence is plain FK");
    assert!(s.bones[knee].solved.approx_eq(s.bones[knee].pose));

    s.bones[target].value = 0.5;
    s.evaluate();
    let halfway = s.bones[ankle].head();
    assert!(halfway.distance(fk_foot) > 0.1 && halfway.distance(goal) > 0.1, "half should land between: {halfway}");

    let mut animation = Animation::default();
    let pose = s.bones[target].pose;
    animation.set_key_full(target, 0, pose, 1.0);
    animation.set_key_full(target, 30, pose, 0.0);
    animation.apply(&mut s, 0.0);
    assert!(s.bones[ankle].head().distance(goal) < 1e-3, "IK at the first key");
    animation.apply(&mut s, 30.0);
    assert!(s.bones[ankle].head().distance(fk_foot) < 1e-3, "FK at the last key");
    animation.apply(&mut s, 15.0);
    assert!((s.bones[target].value - 0.5).abs() < 1e-3, "influence is on a curve");
}

#[test]
fn turning_a_modifier_off_gets_the_original_animation_back() {
    let mut s = rig("R15Rig.rbxm");
    let ankle = bone(&s, "RightAnkle");
    let before: Vec<Xf> = s.parts.iter().map(|p| p.world).collect();
    let target = modifiers::add_ik(&mut s, ankle);
    let at = s.bones[ankle].head() + Vec3::Y;
    place(&mut s, target, at);
    s.modifiers[0].enabled = false;
    s.evaluate();
    for (part, world) in s.parts.iter().zip(before) {
        assert!(part.world.pos.distance(world.pos) < 1e-4, "{} stayed moved", part.name);
    }
}

#[test]
fn unparented_legs_stay_put_when_the_torso_moves() {
    let mut s = rig("R15Rig.rbxm");
    let (root, waist, hip) = (bone(&s, "Root"), bone(&s, "Waist"), bone(&s, "RightHip"));
    let leg_part = s.parts.iter().position(|p| p.name == "RightUpperLeg").unwrap();
    let other_leg = s.parts.iter().position(|p| p.name == "LeftUpperLeg").unwrap();
    let (leg, other) = (s.parts[leg_part].world, s.parts[other_leg].world);

    modifiers::add_unparent(&mut s, hip);
    assert!(s.parts[leg_part].world.pos.distance(leg.pos) < 1e-4, "adding it moved the leg");

    let moved = Xf::new(Quat::from_rotation_y(0.9), Vec3::new(0.6, 0.4, -0.3));
    s.set_bone_pose(root, moved);
    s.set_bone_pose(waist, turned(0.4));
    assert!(s.parts[leg_part].world.pos.distance(leg.pos) < 1e-4, "the unparented leg followed the torso");
    assert!(s.parts[leg_part].world.rot.dot(leg.rot).abs() > 0.9999, "the unparented leg turned with the torso");
    assert!(s.parts[other_leg].world.pos.distance(other.pos) > 0.3, "the other leg still follows");
    assert_plays_back(&s);
}

#[test]
fn unparent_works_on_r6_and_its_own_pose_still_animates() {
    let mut s = rig("R6Rig.rbxm");
    let (root, hip) = (bone(&s, "RootJoint"), bone(&s, "Left Hip"));
    let pivot = s.bones[hip].frame.pos;
    modifiers::add_unparent(&mut s, hip);
    s.set_bone_pose(root, Xf::new(Quat::from_rotation_y(1.0), Vec3::new(0.0, 0.5, 0.0)));
    assert!(s.bones[hip].frame.pos.distance(pivot) < 1e-4);

    // Moving it by hand keeps working, relative to where it's held.
    let frame = Xf::new(Quat::from_rotation_x(0.5), pivot + Vec3::Y);
    s.set_bone_frame(hip, frame);
    assert!(s.bones[hip].frame.approx_eq(frame));
    assert_plays_back(&s);
}

#[test]
fn unparent_settings_change_what_it_lets_go_of() {
    let mut s = rig("R6Rig.rbxm");
    let (root, hip) = (bone(&s, "RootJoint"), bone(&s, "Left Hip"));
    let start = s.bones[hip].frame;
    modifiers::add_unparent(&mut s, hip);
    let body = Xf::new(Quat::from_rotation_y(1.0), Vec3::new(1.0, 0.0, 0.0));
    s.set_bone_pose(root, body);

    if let ModifierKind::Unparent(u) = &mut s.modifiers[0].kind {
        u.rotation = false;
    }
    s.evaluate();
    assert!(s.bones[hip].frame.pos.distance(start.pos) < 1e-4, "still holds position");
    assert!(s.bones[hip].frame.rot.dot(start.rot).abs() < 0.99, "but turns with its parent");

    if let ModifierKind::Unparent(u) = &mut s.modifiers[0].kind {
        u.rotation = true;
    }
    s.bones[hip].value = 0.5;
    s.evaluate();
    let follows = s.bones[hip].joint_base.pos;
    let halfway = s.bones[hip].frame.pos;
    assert!(halfway.distance((start.pos + follows) * 0.5) < 1e-4, "half influence lands halfway");
    assert_plays_back(&s);
}

#[test]
fn an_unparented_bone_can_follow_a_different_bone_without_jumping() {
    let mut s = rig("R15Rig.rbxm");
    let (wrist, head) = (bone(&s, "RightWrist"), bone(&s, "Neck"));
    let start = s.bones[wrist].frame;
    modifiers::add_unparent(&mut s, wrist);
    modifiers::set_anchor(&mut s, 0, Some(head));
    assert!(s.bones[wrist].frame.pos.distance(start.pos) < 1e-4, "changing the anchor moved it");

    let shift = Vec3::new(0.0, 0.0, 0.7);
    s.set_bone_pose(head, Xf::new(Quat::IDENTITY, shift));
    assert!(s.bones[wrist].frame.pos.distance(start.pos + shift) < 1e-3, "it should follow the new bone");
    assert_plays_back(&s);

    modifiers::hold_here(&mut s, 0);
    assert!(s.bones[wrist].frame.pos.distance(s.bones[wrist].joint_base.pos) < 1e-4, "hold where the joint is now");
}

#[test]
fn undo_restores_modifiers_and_takes_added_controls_back_out() {
    let mut s = rig("R15Rig.rbxm");
    let ankle = bone(&s, "RightAnkle");
    let mut animation = Animation::default();
    let mut history = History::default();

    let before = (s.rig_state(), animation.clone());
    let target = modifiers::add_ik(&mut s, ankle);
    animation.set_key_full(target, 0, s.bones[target].pose, 1.0);
    history.push_rig("Add IK", before, (s.rig_state(), animation.clone()));
    assert!(!s.bones[target].removed);

    history.undo(&mut s, &mut animation).unwrap();
    assert!(s.modifiers.is_empty());
    assert!(s.bones[target].removed, "the target leaves with its modifier");
    assert!(animation.tracks.is_empty());
    assert!(!s.live_bones(0).contains(&target));

    history.redo(&mut s, &mut animation).unwrap();
    assert_eq!(s.modifiers.len(), 1);
    assert!(!s.bones[target].removed, "and comes back with it");
}

#[test]
fn removing_ik_takes_its_controls_and_grabbing_goes_back_to_the_bone() {
    let mut s = rig("R15Rig.rbxm");
    let (knee, ankle) = (bone(&s, "RightKnee"), bone(&s, "RightAnkle"));
    let target = modifiers::add_ik(&mut s, ankle);
    modifiers::add_pole(&mut s, 0);
    assert_eq!(modifiers::control_of(&s, ankle), target, "grabbing the foot grabs its target");
    assert_eq!(modifiers::owner_of(&s, target), ankle);
    assert!(modifiers::locked_by_ik(&s, knee), "the knee can't be posed by hand under full IK");

    s.bones[target].value = 0.0;
    s.evaluate();
    assert_eq!(modifiers::control_of(&s, ankle), ankle, "at zero influence it's a plain bone again");
    assert!(!modifiers::locked_by_ik(&s, knee));
    s.bones[target].value = 1.0;

    let gone = modifiers::remove(&mut s, 0);
    assert_eq!(gone.len(), 2);
    assert!(gone.iter().all(|b| s.bones[*b].removed));
    assert_eq!(modifiers::control_of(&s, ankle), ankle);
}

#[test]
fn targets_stay_out_of_the_rigs_bounds_and_joint_counts() {
    let mut s = rig("R15Rig.rbxm");
    let joints = s.joints(0).len();
    let bounds = s.bounds(None).unwrap();
    let ankle = bone(&s, "RightAnkle");
    let target = modifiers::add_ik(&mut s, ankle);
    s.set_bone_pose(target, Xf::new(Quat::IDENTITY, Vec3::splat(500.0)));
    assert_eq!(s.joints(0).len(), joints);
    assert_eq!(s.live_bones(0).len(), joints + 1);
    let now = s.bounds(None).unwrap();
    assert!(now.1.distance(bounds.1) < 10.0, "a far off target shouldn't stretch the view");
}
