//! Headless benchmarks of everything that doesn't need a window: import, pose
//! evaluation, IK, animation sampling and undo. Results go to stdout as a table.
//! Usage: cargo run --release --example perf -- [out.csv]
//!
//! For real frame times, see `examples/frametime.rs`.

use std::fmt::Write as _;
use std::hint::black_box;
use std::time::{Duration, Instant};

use glam::{Quat, Vec3};
use kaye::animation::{Animation, EaseStyle, Easing, Key, Smoothing, Track};
use kaye::history::History;
use kaye::import::{import_bytes, import_dom, import_file};
use kaye::math::Xf;
use kaye::modifiers;
use kaye::scene::{BoneId, Scene};
use rbx_dom_weak::{InstanceBuilder, WeakDom};
use rbx_types::{CFrame, Matrix3, Ref, Variant, Vector3};
use ustr::ustr;

const BUDGET: Duration = Duration::from_millis(600);
const FRAME_60: f64 = 1000.0 / 60.0;

struct Report {
    csv: String,
}

impl Report {
    /// Runs `work` until the budget is spent, and reports the median call.
    fn time<T>(&mut self, group: &str, name: &str, mut work: impl FnMut() -> T) -> f64 {
        let mut samples = Vec::new();
        let started = Instant::now();
        while samples.len() < 20 || (started.elapsed() < BUDGET && samples.len() < 200_000) {
            let at = Instant::now();
            black_box(work());
            samples.push(at.elapsed().as_secs_f64() * 1000.0);
            if started.elapsed() > BUDGET * 10 {
                break;
            }
        }
        samples.sort_by(f64::total_cmp);
        let (median, high) = (samples[samples.len() / 2], samples[samples.len() * 99 / 100]);
        println!(
            "{group:<12} {name:<44} {median:>11.4} ms  p99 {high:>11.4} ms  {:>7.2}% of a 60 fps frame  (n={})",
            median / FRAME_60 * 100.0,
            samples.len()
        );
        let _ = writeln!(self.csv, "{group},\"{name}\",{median:.5},{high:.5},{}", samples.len());
        median
    }
}

fn at(x: f32, y: f32, z: f32) -> CFrame {
    CFrame::new(Vector3::new(x, y, z), Matrix3::identity())
}

fn part(name: String, position: CFrame) -> InstanceBuilder {
    InstanceBuilder::new("Part")
        .with_name(name)
        .with_property("CFrame", position)
        .with_property("Size", Vector3::new(1.0, 1.0, 1.0))
}

fn set_ref(dom: &mut WeakDom, on: Ref, name: &str, to: Ref) {
    dom.get_by_ref_mut(on).unwrap().properties.insert(ustr(name), Variant::Ref(to));
}

/// A rig of `count` bones as a tree, `arms` chains out from the root part. Long
/// chains are what custom creatures and tails look like.
fn tree_rig(count: usize, arms: usize) -> WeakDom {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let model = dom.insert(root, InstanceBuilder::new("Model").with_name("Rig"));
    let trunk = dom.insert(model, part("Root".to_string(), at(0.0, 5.0, 0.0)));
    let mut tips = vec![trunk; arms];
    for i in 0..count {
        let arm = i % arms;
        let angle = arm as f32 / arms as f32 * std::f32::consts::TAU;
        let (x, z) = (angle.cos(), angle.sin());
        let step = (i / arms + 1) as f32;
        let next = dom.insert(model, part(format!("Link{i}"), at(x * step, 5.0, z * step)));
        let joint = dom.insert(
            tips[arm],
            InstanceBuilder::new("Motor6D")
                .with_name(format!("Joint{i}"))
                .with_property("C0", at(x, 0.0, z))
                .with_property("C1", at(0.0, 0.0, 0.0)),
        );
        set_ref(&mut dom, joint, "Part0", tips[arm]);
        set_ref(&mut dom, joint, "Part1", next);
        tips[arm] = next;
    }
    dom
}

/// A static model of unjoined parts, like a map or a prop pack.
fn static_model(count: usize) -> WeakDom {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let model = dom.insert(root, InstanceBuilder::new("Model").with_name("Map"));
    let side = (count as f32).sqrt().ceil() as usize;
    for i in 0..count {
        dom.insert(model, part(format!("Part{i}"), at((i % side) as f32 * 2.0, 0.5, (i / side) as f32 * 2.0)));
    }
    dom
}

fn model_refs(dom: &WeakDom) -> Vec<Ref> {
    dom.root().children().to_vec()
}

fn binary(dom: &WeakDom) -> Vec<u8> {
    let mut out = Vec::new();
    rbx_binary::to_writer(&mut out, dom, &model_refs(dom)).unwrap();
    out
}

fn xml(dom: &WeakDom) -> Vec<u8> {
    let mut out = Vec::new();
    rbx_xml::to_writer_default(&mut out, dom, &model_refs(dom)).unwrap();
    out
}

/// A smooth, different swing on every bone, so nothing gets a cheap path by accident.
fn swing(index: usize, time: f32) -> Xf {
    let phase = index as f32 * 0.7 + time * std::f32::consts::TAU;
    Xf::new(
        Quat::from_euler(glam::EulerRot::XYZ, phase.sin() * 0.6, (phase * 0.5).cos() * 0.3, (phase * 1.5).sin() * 0.2),
        Vec3::new(0.0, (phase * 2.0).sin() * 0.05, 0.0),
    )
}

/// `keys` evenly spaced keys on each bone, across a 120 frame animation.
fn keyed(bones: &[BoneId], keys: usize, style: EaseStyle, smoothing: Smoothing) -> Animation {
    let mut animation = Animation {
        smoothing,
        default_easing: Easing { style, ..Easing::default() },
        end: 120.max(keys as i32),
        ..Animation::default()
    };
    let length = animation.end;
    for (index, bone) in bones.iter().enumerate() {
        for key in 0..keys {
            let frame = key as i32 * length / (keys as i32 - 1).max(1);
            animation.set_key(*bone, frame, swing(index, key as f32 / keys as f32));
        }
    }
    animation
}

fn joints(scene: &Scene) -> Vec<BoneId> {
    (0..scene.bones.len()).filter(|b| scene.bones[*b].kind.is_joint()).collect()
}

fn named(scene: &Scene, name: &str) -> BoneId {
    scene.bones.iter().position(|b| b.name == name).unwrap_or_else(|| panic!("no bone {name}"))
}

fn sample(name: &str) -> Scene {
    import_file(format!("samples/{name}.rbxm").as_ref()).unwrap().scene
}

fn with_limb_ik(mut scene: Scene, poles: bool) -> Scene {
    for (index, bone) in ["LeftAnkle", "RightAnkle", "LeftWrist", "RightWrist"].iter().enumerate() {
        let bone = named(&scene, bone);
        modifiers::add_ik(&mut scene, bone);
        if poles {
            modifiers::add_pole(&mut scene, index);
        }
    }
    scene
}

/// `rigs` copies of the R15 rig in one scene, each with IK and poles on all four limbs.
fn crowd(rigs: usize, ik: bool) -> Scene {
    let mut scene = Scene::default();
    for _ in 0..rigs {
        scene.append(sample("R15Rig"));
    }
    scene.evaluate();
    if ik {
        for rig in 0..rigs {
            for limb in ["LeftAnkle", "RightAnkle", "LeftWrist", "RightWrist"] {
                let bone = scene.rigs[rig].bones.iter().copied().find(|b| scene.bones[*b].name == limb).unwrap();
                modifiers::add_ik(&mut scene, bone);
                let index = scene.modifiers.len() - 1;
                modifiers::add_pole(&mut scene, index);
            }
        }
    }
    scene
}

/// Private bytes of this process, for measuring what undo really holds.
#[cfg(windows)]
fn private_bytes() -> usize {
    #[repr(C)]
    #[derive(Default)]
    struct Counters([usize; 10]);
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, size: u32) -> i32;
    }
    let mut c = Counters::default();
    c.0[0] = size_of::<Counters>();
    // Safe: `c` is live and the size matches it. The struct starts with two u32s,
    // which fit the first usize, and `PagefileUsage` is the eighth field after them.
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, size_of::<Counters>() as u32) };
    c.0[8]
}

#[cfg(not(windows))]
fn private_bytes() -> usize {
    0
}

/// What a full undo history really holds. Needs its own run, and every history stays
/// alive, because the heap hands freed memory back out without growing.
fn undo_memory() {
    let mut kept = Vec::new();
    for (label, count, keys) in [("R15, 9 keys/bone", 15, 9), ("R15, 30 keys/bone", 15, 30), ("R15, 120 keys/bone", 15, 120), ("200 bones, 120 keys/bone", 200, 120)] {
        let ids: Vec<BoneId> = (0..count).collect();
        let mut live = keyed(&ids, keys, EaseStyle::Bezier, Smoothing::Continuous);
        let held = private_bytes();
        let mut history = History::default();
        for edit in 0..300 {
            let before = live.clone();
            live.set_key(0, edit % 120, swing(0, 0.1));
            history.push_edit("Edit", Vec::new(), before, live.clone());
        }
        println!("{label}: a full undo history holds {:.0} MB", private_bytes().saturating_sub(held) as f64 / 1e6);
        kept.push(history);
    }
}

/// Writes the generated models that `frametime` opens.
fn write_models(dir: &str) {
    std::fs::create_dir_all(dir).unwrap();
    for count in [1_000, 10_000, 60_000] {
        std::fs::write(format!("{dir}/parts_{count}.rbxm"), binary(&static_model(count))).unwrap();
    }
    for count in [100, 200, 1_000, 5_000] {
        std::fs::write(format!("{dir}/bones_{count}.rbxm"), binary(&tree_rig(count, 8))).unwrap();
    }
}

/// IK on the last bones of the rig's chains, with a chain of `chain` bones.
fn with_chain_ik(mut scene: Scene, arms: usize, chain: usize) -> Scene {
    let count = joints(&scene).len();
    for tip in count - arms..count {
        modifiers::add_ik(&mut scene, tip);
        let last = scene.modifiers.len() - 1;
        if let modifiers::ModifierKind::Ik(ik) = &mut scene.modifiers[last].kind {
            ik.chain = chain;
        }
    }
    scene.evaluate();
    scene
}

fn main() {
    let mut r = Report { csv: String::from("group,name,median_ms,p99_ms,samples\n") };
    let out = std::env::args().nth(1);
    if out.as_deref() == Some("--write-models") {
        write_models(&std::env::args().nth(2).expect("a folder for the models"));
        return;
    }
    if out.as_deref() == Some("--undo-memory") {
        undo_memory();
        return;
    }

    println!("Sizes: Key {} bytes, Bone {} bytes, Part {} bytes", size_of::<Key>(), size_of::<kaye::scene::Bone>(), size_of::<kaye::scene::Part>());

    // Import: file parsing plus building the rig.
    for name in ["R6Rig", "R15Rig", "R6WithAccessories"] {
        let path = format!("samples/{name}.rbxm");
        r.time("import", &format!("{name}.rbxm"), || import_file(path.as_ref()).unwrap());
    }
    for count in [1_000, 10_000, 60_000] {
        let dom = static_model(count);
        let (rbxm, rbxmx) = (binary(&dom), xml(&dom));
        println!("  {count} parts: rbxm {} KB, rbxmx {} KB", rbxm.len() / 1024, rbxmx.len() / 1024);
        r.time("import", &format!("static {count} parts, parse+build rbxm"), || import_bytes(&rbxm, "m.rbxm").unwrap());
        r.time("import", &format!("static {count} parts, parse+build rbxmx"), || import_bytes(&rbxmx, "m.rbxmx").unwrap());
        r.time("import", &format!("static {count} parts, build only"), || import_dom(&dom, "m.rbxm").unwrap());
    }
    for count in [100, 1_000, 5_000] {
        let dom = tree_rig(count, 8);
        let rbxm = binary(&dom);
        r.time("import", &format!("rig {count} bones, parse+build rbxm"), || import_bytes(&rbxm, "m.rbxm").unwrap());
    }

    // Evaluate runs every frame during playback and every mouse move during a drag.
    for (name, mut scene) in [
        ("R6", sample("R6Rig")),
        ("R15", sample("R15Rig")),
        ("R15 + 4 IK", with_limb_ik(sample("R15Rig"), false)),
        ("R15 + 4 IK + poles", with_limb_ik(sample("R15Rig"), true)),
    ] {
        r.time("evaluate", name, || scene.evaluate());
    }
    {
        let mut scene = sample("R15Rig");
        for bone in joints(&scene) {
            modifiers::add_unparent(&mut scene, bone);
        }
        r.time("evaluate", "R15 + Unparent on all 15 bones", || scene.evaluate());
    }
    for count in [100, 1_000, 5_000] {
        let imported = |n| import_dom(&tree_rig(n, 8), "m.rbxm").unwrap().scene;
        let mut plain = imported(count);
        r.time("evaluate", &format!("{count} bones"), || plain.evaluate());
        let mut solved = with_chain_ik(imported(count), 8, 4);
        r.time("evaluate", &format!("{count} bones + 8 IK chains of 4"), || solved.evaluate());
    }
    for rigs in [2, 5, 10, 20, 50] {
        let mut plain = crowd(rigs, false);
        r.time("evaluate", &format!("{rigs} R15 rigs in one scene"), || plain.evaluate());
        let mut solved = crowd(rigs, true);
        r.time("evaluate", &format!("{rigs} R15 rigs, each with 4 IK + poles"), || solved.evaluate());
    }

    // Sampling at a fractional frame, which is what playback does.
    let scene = sample("R15Rig");
    let bones = joints(&scene);
    for keys in [5, 9, 30, 120] {
        for (name, style, smoothing) in [
            ("Bezier, Continuous (default)", EaseStyle::Bezier, Smoothing::Continuous),
            ("Bezier, no smoothing", EaseStyle::Bezier, Smoothing::None),
            ("Linear", EaseStyle::Linear, Smoothing::None),
        ] {
            let animation = keyed(&bones, keys, style, smoothing);
            let mut scene = scene.clone();
            let mut time = 0.0;
            r.time("apply", &format!("R15 {keys} keys/bone, {name}"), || {
                time = (time + 0.37) % 120.0;
                animation.apply(&mut scene, time);
            });
        }
    }
    let rig = import_dom(&tree_rig(200, 8), "m.rbxm").unwrap().scene;
    let many = joints(&rig);
    for keys in [30, 120, 600] {
        let animation = keyed(&many, keys, EaseStyle::Bezier, Smoothing::Continuous);
        let mut scene = rig.clone();
        let mut time = 0.0;
        r.time("apply", &format!("200 bones, {keys} keys/bone, default"), || {
            time = (time + 0.37) % animation.end as f32;
            animation.apply(&mut scene, time);
        });
    }

    // What the Graph tab and every sample above spend their time on.
    for keys in [9, 120, 600] {
        let track: Track = keyed(&bones, keys, EaseStyle::Bezier, Smoothing::Continuous).tracks.remove(&bones[0]).unwrap();
        r.time("curves", &format!("Track::curves, {keys} keys"), || {
            let curves = track.curves(Smoothing::Continuous);
            curves.points[0].len()
        });
        let curves = track.curves(Smoothing::Continuous);
        r.time("curves", &format!("Graph tab, 6 curves at 900 px, {keys} keys"), || {
            let mut sum = 0.0;
            for x in 0..900 {
                if let Some(values) = curves.at(x as f32 / 900.0 * 120.0) {
                    sum += values[0];
                }
            }
            sum
        });
    }

    // Undo keeps a whole copy of the animation on each side of every key edit.
    for (label, count, keys) in [("R15, 9 keys/bone", 15, 9), ("R15, 120 keys/bone", 15, 120), ("200 bones, 120 keys/bone", 200, 120)] {
        let ids: Vec<BoneId> = (0..count).collect();
        let animation = keyed(&ids, keys, EaseStyle::Bezier, Smoothing::Continuous);
        let bytes = count * keys * size_of::<Key>();
        r.time("undo", &format!("clone animation, {label}"), || animation.clone());
        r.time("undo", &format!("compare animations, {label}"), || animation == animation.clone());
        println!(
            "  {label}: one copy holds at least {:.1} MB of keys, so 256 key edits hold {:.0} MB",
            bytes as f64 / 1e6,
            bytes as f64 * 2.0 * 256.0 / 1e6
        );
        let mut history = History::default();
        let mut scene = Scene::default();
        let mut live = animation.clone();
        let mut frame = 0;
        r.time("undo", &format!("edit, then undo, {label}"), || {
            frame = (frame + 1) % 120;
            let before = live.clone();
            live.set_key(0, frame, swing(0, 0.1));
            history.push_edit("Edit", Vec::new(), before, live.clone());
            history.undo(&mut scene, &mut live);
        });
    }

    if let Some(path) = out {
        std::fs::write(path, r.csv).unwrap();
    }
}
