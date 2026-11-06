//! Reference clips: where they sit on the timeline, trimming, matching the frame rate,
//! saving, and reading pictures and GIFs.

use std::path::PathBuf;

use kaye::media::{self, Loaded, MediaKind};
use kaye::reference::{Edge, Reference};

fn clip(duration: f64, fps: f32) -> Reference {
    let mut r = Reference::new(1, "clip.mp4".into(), "clip".into(), 0.0);
    r.duration = duration;
    r.fps = fps;
    r.fill_length(120.0, 30.0);
    r
}

#[test]
fn a_new_clip_covers_its_whole_length() {
    let r = clip(4.0, 30.0);
    assert_eq!((r.trim_in, r.trim_out), (0.0, 4.0));
    assert_eq!(r.span(30.0), (0.0, 120.0));
}

#[test]
fn matching_the_frame_rate_plays_at_real_speed() {
    // A 24 fps clip in a 30 fps animation.
    let r = clip(2.0, 24.0);
    let real = r.rate(30.0, true);
    assert_eq!(real, 30.0);
    assert_eq!(r.span(real).1, 60.0, "two seconds take two seconds of timeline");
    assert!((r.time_at(15.0, real) - 0.5).abs() < 1e-9);
    // Off, each of its frames takes one timeline frame, so it plays slower.
    let each = r.rate(30.0, false);
    assert_eq!(each, 24.0);
    assert_eq!(r.span(each).1, 48.0);
    assert!((r.time_at(12.0, each) - 0.5).abs() < 1e-9);
}

#[test]
fn outside_its_strip_a_clip_holds_its_ends() {
    let mut r = clip(4.0, 30.0);
    r.drag_edge(Edge::Start, 30.0, 30.0);
    r.drag_edge(Edge::End, 90.0, 30.0);
    assert!((r.time_at(0.0, 30.0) - 1.0).abs() < 1e-9);
    assert!((r.time_at(200.0, 30.0) - 3.0).abs() < 1e-9);
}

#[test]
fn trimming_keeps_the_clip_still_under_the_strip() {
    let mut r = clip(4.0, 30.0);
    r.move_by(10.0);
    r.drag_edge(Edge::Start, 40.0, 30.0);
    assert_eq!(r.offset, 10.0, "trimming doesn't slide the clip");
    assert!((r.trim_in - 1.0).abs() < 1e-9);
    assert!((r.time_at(40.0, 30.0) - 1.0).abs() < 1e-9, "the frame at the new start is the one that was there");
}

#[test]
fn trimming_stops_at_the_clip_and_at_one_frame() {
    let mut r = clip(4.0, 30.0);
    r.drag_edge(Edge::Start, -50.0, 30.0);
    assert_eq!(r.trim_in, 0.0, "nothing before the clip's start");
    r.drag_edge(Edge::End, 500.0, 30.0);
    assert_eq!(r.trim_out, 4.0, "nothing past its end");
    r.drag_edge(Edge::End, -10.0, 30.0);
    let (start, end) = r.span(30.0);
    assert!((end - start - 1.0).abs() < 1e-9, "a frame is the shortest");
}

#[test]
fn a_picture_lasts_as_long_as_its_strip() {
    let mut r = Reference::new(1, "pose.png".into(), "pose".into(), 0.0);
    r.fill_length(120.0, 30.0);
    assert!(r.is_picture());
    assert_eq!(r.span(30.0), (0.0, 120.0));
    r.drag_edge(Edge::End, 300.0, 30.0);
    assert_eq!(r.span(30.0).1, 300.0, "a picture has no end of its own");
    r.drag_edge(Edge::Start, 50.0, 30.0);
    let (start, end) = r.span(30.0);
    assert!((start - 50.0).abs() < 1e-9 && (end - 300.0).abs() < 1e-9, "its start moves without moving its end: {start} {end}");
}

fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("kaye-references-test-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    folder.join(name)
}

fn read_all(path: &std::path::Path) -> (f64, f32, Vec<media::Frame>, bool) {
    let (mut duration, mut fps, mut frames, mut done) = (0.0, 0.0, Vec::new(), false);
    media::read(path, &mut |loaded| {
        match loaded {
            Loaded::Started { duration: d, fps: f } => (duration, fps) = (d, f),
            Loaded::Frame(frame) => frames.push(frame),
            Loaded::Done => done = true,
            Loaded::Failed(error) => panic!("{error}"),
            Loaded::Cut(_) => {}
        }
        true
    });
    (duration, fps, frames, done)
}

#[test]
fn reads_a_picture_as_one_frame() {
    let path = scratch("pose.png");
    let image = image::RgbaImage::from_fn(40, 20, |x, _| image::Rgba([x as u8 * 6, 0, 0, 255]));
    image.save(&path).unwrap();
    let (duration, _, frames, done) = read_all(&path);
    assert!(done);
    assert_eq!(duration, 0.0);
    assert_eq!(frames.len(), 1);
    assert_eq!((frames[0].width, frames[0].height), (40, 20));
    let rgba = frames[0].pixels.rgba().unwrap();
    assert_eq!(&rgba[4 * 39..4 * 40], &[234, 0, 0, 255]);
}

#[test]
fn reads_a_gif_with_its_timing() {
    let path = scratch("walk.gif");
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = image::codecs::gif::GifEncoder::new(file);
        for shade in [0u8, 80, 160, 240] {
            let frame = image::RgbaImage::from_pixel(16, 16, image::Rgba([shade, shade, shade, 255]));
            let delay = image::Delay::from_numer_denom_ms(250, 1);
            encoder.encode_frame(image::Frame::from_parts(frame, 0, 0, delay)).unwrap();
        }
    }
    assert_eq!(MediaKind::of(&path), Some(MediaKind::Gif));
    let (duration, fps, frames, done) = read_all(&path);
    assert!(done);
    assert_eq!(frames.len(), 4);
    assert!((duration - 1.0).abs() < 1e-6, "{duration}");
    assert!((fps - 4.0).abs() < 1e-3, "{fps}");
    let times: Vec<f64> = frames.iter().map(|f| f.time).collect();
    assert_eq!(times, [0.0, 0.25, 0.5, 0.75]);
}

#[test]
fn a_missing_file_says_so() {
    let mut failed = None;
    media::read(&scratch("nothing.mp4"), &mut |loaded| {
        if let Loaded::Failed(error) = loaded {
            failed = Some(error);
        }
        true
    });
    assert!(failed.is_some());
}

#[test]
fn references_save_with_the_project() {
    let mut scene = kaye::scene::Scene::default();
    scene.references.push(clip(3.0, 24.0));
    let json = serde_json::to_string(&scene).unwrap();
    let back: kaye::scene::Scene = serde_json::from_str(&json).unwrap();
    assert_eq!(back.references, scene.references);
    // A scene with none leaves the field out, so older Kaye opens it unchanged.
    let empty = serde_json::to_string(&kaye::scene::Scene::default()).unwrap();
    assert!(!empty.contains("references"));
}
