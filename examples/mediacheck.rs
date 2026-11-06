//! Reads a picture, GIF or video the way a reference window does, and prints what came
//! out. Pass a folder to save the first and last frames there as PNGs.
//!
//! Usage: cargo run --example mediacheck -- <file> [folder]

use std::path::PathBuf;
use std::time::Instant;

use kaye::media::{self, Loaded};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().expect("give a file"));
    let folder = args.next().map(PathBuf::from);
    let started = Instant::now();
    let mut frames = Vec::new();
    media::read(&path, &mut |loaded| {
        match loaded {
            Loaded::Started { duration, fps } => println!("started: {duration:.3} s at {fps:.3} fps"),
            Loaded::Frame(frame) => frames.push(frame),
            Loaded::Cut(at) => println!("cut at {at:.3} s"),
            Loaded::Done => println!("done"),
            Loaded::Failed(error) => println!("failed: {error}"),
        }
        true
    });
    let bytes: usize = frames.iter().map(|f| f.pixels.bytes()).sum();
    println!("{} frames in {:.2} s, {:.1} MB kept", frames.len(), started.elapsed().as_secs_f64(), bytes as f64 / 1e6);
    if let (Some(first), Some(last)) = (frames.first(), frames.last()) {
        println!("first at {:.3} s, {}x{}; last at {:.3} s, {}x{}", first.time, first.width, first.height, last.time, last.width, last.height);
        if let Some(folder) = folder {
            for (name, frame) in [("first.png", first), ("last.png", last)] {
                let rgba = frame.pixels.rgba().expect("frame decodes");
                image::save_buffer(folder.join(name), &rgba, frame.width, frame.height, image::ExtendedColorType::Rgba8).unwrap();
            }
        }
    }
}
