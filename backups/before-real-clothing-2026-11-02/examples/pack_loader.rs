//! Packs a rendered animation into a file in `assets/` that `src/app/loading.rs` plays.
//! Run it again whenever an animation is re-rendered:
//!
//!     cargo run --release --example pack_loader -- <folder of PNG frames> loader.bin
//!     cargo run --release --example pack_loader -- <folder of PNG frames> intro.bin
//!
//! Frames only need to be as sharp as the size they're shown at, and grey ones only
//! need two channels.
//!
//! The file is a little-endian u32 frame count, one u32 end offset per frame measured
//! from the start of the PNG data, then the PNGs back to back.

use std::path::{Path, PathBuf};

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::imageops::{self, FilterType as Resize};
use image::{ExtendedColorType, ImageBuffer, ImageEncoder, Rgba};

const SIZE: u32 = 384;
/// How far apart a pixel's channels can be and still count as grey.
const GREY_TOLERANCE: f32 = 1.5 / 255.0;

/// Scales with alpha multiplied in. Scaling straight alpha would pull in the color of
/// fully clear pixels, which leaves a fringe around every edge.
fn pack_frame(path: &Path) -> Vec<u8> {
    let mut frame = image::open(path).expect("frame is a readable image").to_rgba32f();
    for pixel in frame.pixels_mut() {
        let a = pixel[3];
        *pixel = Rgba([pixel[0] * a, pixel[1] * a, pixel[2] * a, a]);
    }
    let small: ImageBuffer<Rgba<f32>, Vec<f32>> = imageops::resize(&frame, SIZE, SIZE, Resize::Lanczos3);
    let straight: Vec<[f32; 4]> = small
        .pixels()
        .map(|p| {
            let a = p[3].clamp(0.0, 1.0);
            let color = |c: f32| if a > 0.0 { (c / a).clamp(0.0, 1.0) } else { 0.0 };
            [color(p[0]), color(p[1]), color(p[2]), a]
        })
        .collect();
    let byte = |v: f32| (v * 255.0).round() as u8;
    let grey = straight.iter().all(|p| p[3] < GREY_TOLERANCE || ((p[0] - p[1]).abs() < GREY_TOLERANCE && (p[1] - p[2]).abs() < GREY_TOLERANCE));
    let (pixels, kind): (Vec<u8>, _) = if grey {
        (straight.iter().flat_map(|p| [byte(p[0]), byte(p[3])]).collect(), ExtendedColorType::La8)
    } else {
        (straight.iter().flat_map(|p| p.map(byte)).collect(), ExtendedColorType::Rgba8)
    };
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Best, FilterType::Adaptive)
        .write_image(&pixels, SIZE, SIZE, kind)
        .expect("frame encodes");
    png
}

fn main() {
    let mut args = std::env::args().skip(1);
    let folder = PathBuf::from(args.next().expect("pass the folder of frames"));
    let name = args.next().expect("pass the file to write in assets, such as loader.bin");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&folder)
        .expect("folder is readable")
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "png"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no PNG frames in {}", folder.display());

    let frames: Vec<Vec<u8>> = paths.iter().map(|path| pack_frame(path)).collect();
    let mut out = (frames.len() as u32).to_le_bytes().to_vec();
    let mut end = 0u32;
    for frame in &frames {
        end += frame.len() as u32;
        out.extend(end.to_le_bytes());
    }
    for frame in &frames {
        out.extend(frame);
    }
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join(name);
    std::fs::write(&target, &out).expect("assets folder is writable");
    println!("{} frames, {:.1} MB, {}", frames.len(), out.len() as f32 / 1_048_576.0, target.display());
}
