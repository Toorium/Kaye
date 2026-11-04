//! Gives the installer Kaye's icon and tells Windows how to run it, and works out the logo for
//! the wizard's side panel. Windows only draws plain pixels, so the picture is decoded here
//! and the program doesn't need a PNG reader.

use std::path::Path;

/// Keep these the same as `MARK_SIZE` and `BANNER_COLOR` in `src/wizard.rs`.
const MARK_SIZE: u32 = 240;
const BANNER_COLOR: [u8; 3] = [0xF3, 0x6B, 0x70];

fn write_mark() {
    let mark = image::open("../assets/logo_mark.png").expect("the logo is there").to_rgba8();
    let mark = image::imageops::resize(&mark, MARK_SIZE, MARK_SIZE, image::imageops::FilterType::Lanczos3);
    let mut pixels = Vec::with_capacity((MARK_SIZE * MARK_SIZE * 4) as usize);
    for pixel in mark.pixels() {
        let solid = pixel[3] as f32 / 255.0;
        let blended = BANNER_COLOR.map(|channel| (channel as f32 + (255.0 - channel as f32) * solid).round() as u8);
        // Windows keeps its colors in the other order.
        pixels.extend_from_slice(&[blended[2], blended[1], blended[0], 0]);
    }
    let out = std::env::var("OUT_DIR").expect("cargo sets this");
    std::fs::write(Path::new(&out).join("mark.bgra"), pixels).expect("the build folder can be written to");
}

fn main() {
    println!("cargo:rerun-if-changed=../assets/kaye.ico");
    println!("cargo:rerun-if-changed=../assets/logo_mark.png");
    println!("cargo:rerun-if-changed=setup.manifest");
    write_mark();
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../assets/kaye.ico");
    resource.set_manifest_file("setup.manifest");
    resource.set("FileDescription", "Kaye Setup");
    resource.set("ProductName", "Kaye");
    // A missing resource compiler shouldn't stop it building.
    if let Err(error) = resource.compile() {
        println!("cargo:warning=Couldn't embed the icon: {error}");
    }
}
