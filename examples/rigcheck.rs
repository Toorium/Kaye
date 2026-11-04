//! Prints how Kaye holds every part of a model: as a root, by a bone, or welded to another
//! part. A part listed as a root that should move with the body, like a hat, means a joint
//! wasn't understood. Takes a model or a `.kaye` project, which shows what opening it gives.
//! `cargo run --example rigcheck -- samples/R15Rig.rbxm`

use kaye::import::import_file;
use kaye::scene::Drive;

fn main() {
    let path = std::env::args().nth(1).expect("a model file");
    // A project shows what opening it gives, after any upgrade.
    let (mut scene, warnings) = if path.ends_with(".kaye") {
        (kaye::project::load(std::path::Path::new(&path)).unwrap().scene, Vec::new())
    } else {
        let imported = import_file(std::path::Path::new(&path)).unwrap();
        (imported.scene, imported.warnings)
    };
    scene.evaluate();
    let scene = &scene;
    for rig in &scene.rigs {
        println!("{} ({:?}): {} bones, {} parts", rig.name, rig.kind, rig.bones.len(), rig.parts.len());
        let roots = rig.parts.iter().filter(|p| matches!(scene.parts[**p].drive, Drive::Root)).count();
        println!("  {roots} roots, shirt {:?}, pants {:?}", rig.shirt, rig.pants);
        for &p in &rig.parts {
            let part = &scene.parts[p];
            let held = match part.drive {
                Drive::Root => "ROOT".to_string(),
                Drive::Bone(b) => format!("bone {}", scene.bones[b].name),
                Drive::Rigid { parent, .. } => format!("welded to {}", scene.parts[parent].name),
            };
            // With no pose, every part should stand where the file saved it.
            let moved = part.world.pos.distance(part.file_cframe.pos);
            let note = if moved > 0.01 { format!("  MOVED {moved:.3} studs from where it was saved") } else { String::new() };
            println!("    {:<24} {held}{note}", part.name);
        }
    }
    for warning in &warnings {
        println!("warning: {warning}");
    }
}
