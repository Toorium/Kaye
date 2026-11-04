//! Reports what Kaye reads from every mesh in a folder, for when a real asset looks wrong.
//! Run with: cargo run --example meshcheck -- <assets folder>

use kaye::rbxmesh;

fn main() {
    let folder = std::env::args().nth(1).expect("give a folder of assets");
    for entry in std::fs::read_dir(folder).expect("readable folder").flatten() {
        let bytes = std::fs::read(entry.path()).expect("readable file");
        if !bytes.starts_with(b"version") {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let version = String::from_utf8_lossy(&bytes[..12]).into_owned();
        println!("{name} [{version}] {} bytes", bytes.len());
        print_chunks(&bytes);
        match rbxmesh::parse(&bytes) {
            Ok(mesh) => {
                let (center, size) = mesh.bounds();
                let finite = mesh.positions.iter().all(|p| p.is_finite());
                let named = mesh.weights.iter().filter(|w| w.amounts[0] > 0).count();
                println!(
                    "  ok: {} vertices, {} triangles, center {center:.3}, size {size:.3}, finite {finite}, {} bones, {} weights ({named} used)",
                    mesh.positions.len(),
                    mesh.indices.len() / 3,
                    mesh.bones.len(),
                    mesh.weights.len()
                );
                println!("  bones: {:?}", mesh.bones.iter().take(8).collect::<Vec<_>>());
            }
            Err(error) => println!("  FAILED: {error}"),
        }
    }
}

/// Names the chunks of a version 6 or 7 file.
fn print_chunks(bytes: &[u8]) {
    if !bytes.starts_with(b"version 6") && !bytes.starts_with(b"version 7") {
        return;
    }
    let mut at = 13;
    while at + 16 <= bytes.len() {
        let name = String::from_utf8_lossy(&bytes[at..at + 8]).replace('\0', "");
        let word = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        let (version, size) = (word(at + 8), word(at + 12) as usize);
        println!("  chunk {name} v{version}, {size} bytes");
        at += 16 + size;
    }
}
