//! Pictures, GIFs and videos for reference windows. Each is read into frames on another
//! thread, shrunk and kept compressed, so a whole clip fits in memory and scrubbing is
//! instant. Videos go through Windows' own Media Foundation, so MP4, MOV, WMV, AVI and MKV
//! work with nothing extra installed. No interface code.

use std::path::Path;

use image::{ImageFormat, RgbImage, RgbaImage};

/// Frames are shrunk to fit this many pixels on their longest side.
pub const VIDEO_SIDE: u32 = 960;
const PICTURE_SIDE: u32 = 2048;
/// A clip stops reading once its frames take this much memory, and says how far it got.
const MEMORY_LIMIT: usize = 768 * 1024 * 1024;
const JPEG_QUALITY: u8 = 88;

pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "wmv", "avi", "mkv", "webm", "3gp"];
pub const PICTURE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp", "webp"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Picture,
    Gif,
    Video,
}

impl MediaKind {
    pub fn of(path: &Path) -> Option<MediaKind> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        if extension == "gif" {
            Some(MediaKind::Gif)
        } else if PICTURE_EXTENSIONS.contains(&extension.as_str()) {
            Some(MediaKind::Picture)
        } else if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
            Some(MediaKind::Video)
        } else {
            None
        }
    }
}

/// Every extension a reference can be made from, for the file picker.
pub fn extensions() -> Vec<&'static str> {
    PICTURE_EXTENSIONS.iter().chain(["gif"].iter()).chain(VIDEO_EXTENSIONS).copied().collect()
}

/// A frame's pixels, kept small.
pub enum Pixels {
    /// Straight RGBA, for pictures and GIFs, which can be see-through.
    Rgba(Vec<u8>),
    /// A video frame, which never is.
    Jpeg(Vec<u8>),
}

impl Pixels {
    pub fn bytes(&self) -> usize {
        match self {
            Pixels::Rgba(data) | Pixels::Jpeg(data) => data.len(),
        }
    }

    /// RGBA, one byte per channel.
    pub fn rgba(&self) -> Option<Vec<u8>> {
        match self {
            Pixels::Rgba(data) => Some(data.clone()),
            Pixels::Jpeg(data) => Some(image::load_from_memory_with_format(data, ImageFormat::Jpeg).ok()?.to_rgba8().into_raw()),
        }
    }
}

/// One picture of a clip and when it shows, in seconds from the clip's start.
pub struct Frame {
    pub time: f64,
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
}

/// What the reading thread reports, in order: `Started`, then frames, then `Done` or
/// `Failed`.
pub enum Loaded {
    /// `duration` is 0 for a picture, and `fps` is the clip's own rate.
    Started { duration: f64, fps: f32 },
    Frame(Frame),
    /// Stopped early, at this many seconds, because the clip wouldn't fit in memory.
    Cut(f64),
    Done,
    Failed(String),
}

/// Reads a file, handing each part to `out` as it's ready. `out` returns false to stop.
pub fn read(path: &Path, out: &mut dyn FnMut(Loaded) -> bool) {
    let result = match MediaKind::of(path) {
        Some(MediaKind::Picture) => read_picture(path, out),
        Some(MediaKind::Gif) => read_gif(path, out),
        Some(MediaKind::Video) => read_video(path, out),
        None => Err("Kaye can't show this kind of file.".to_string()),
    };
    match result {
        Ok(()) => {
            out(Loaded::Done);
        }
        Err(error) => {
            out(Loaded::Failed(error));
        }
    }
}

/// The size that fits `side`, keeping the shape. Never bigger than it was.
fn fit(width: u32, height: u32, side: u32) -> (u32, u32) {
    let longest = width.max(height).max(1);
    if longest <= side {
        return (width.max(1), height.max(1));
    }
    let scale = side as f32 / longest as f32;
    (((width as f32 * scale).round() as u32).max(1), ((height as f32 * scale).round() as u32).max(1))
}

fn shrink_rgba(image: RgbaImage, side: u32) -> RgbaImage {
    let (w, h) = fit(image.width(), image.height(), side);
    if (w, h) == image.dimensions() { image } else { image::imageops::thumbnail(&image, w, h) }
}

fn rgba_frame(time: f64, image: RgbaImage) -> Frame {
    let (width, height) = image.dimensions();
    Frame { time, width, height, pixels: Pixels::Rgba(image.into_raw()) }
}

fn read_picture(path: &Path, out: &mut dyn FnMut(Loaded) -> bool) -> Result<(), String> {
    let image = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    if out(Loaded::Started { duration: 0.0, fps: 0.0 }) {
        out(Loaded::Frame(rgba_frame(0.0, shrink_rgba(image, PICTURE_SIDE))));
    }
    Ok(())
}

fn read_gif(path: &Path, out: &mut dyn FnMut(Loaded) -> bool) -> Result<(), String> {
    use image::AnimationDecoder;
    let file = std::io::BufReader::new(std::fs::File::open(path).map_err(|e| e.to_string())?);
    let decoder = image::codecs::gif::GifDecoder::new(file).map_err(|e| e.to_string())?;
    let frames = decoder.into_frames().collect_frames().map_err(|e| e.to_string())?;
    // Browsers treat a delay under 20 ms as 100 ms, and GIFs are made to look right there.
    let delays: Vec<f64> = frames
        .iter()
        .map(|f| {
            let (n, d) = f.delay().numer_denom_ms();
            let ms = n as f64 / d.max(1) as f64;
            if ms < 20.0 { 0.1 } else { ms / 1000.0 }
        })
        .collect();
    let duration: f64 = delays.iter().sum();
    let fps = if duration > 0.0 { (frames.len() as f64 / duration) as f32 } else { 10.0 };
    if !out(Loaded::Started { duration, fps }) {
        return Ok(());
    }
    let mut time = 0.0;
    for (frame, delay) in frames.into_iter().zip(delays) {
        if !out(Loaded::Frame(rgba_frame(time, shrink_rgba(frame.into_buffer(), VIDEO_SIDE)))) {
            return Ok(());
        }
        time += delay;
    }
    Ok(())
}

/// Turns a video frame upright and small, then compresses it.
fn video_frame(time: f64, image: RgbImage, rotation: u32) -> Option<Frame> {
    let image = match rotation {
        90 => image::imageops::rotate90(&image),
        180 => image::imageops::rotate180(&image),
        270 => image::imageops::rotate270(&image),
        _ => image,
    };
    let (w, h) = fit(image.width(), image.height(), VIDEO_SIDE);
    let image = if (w, h) == image.dimensions() { image } else { image::imageops::thumbnail(&image, w, h) };
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .encode(image.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .ok()?;
    Some(Frame { time, width: w, height: h, pixels: Pixels::Jpeg(jpeg) })
}

#[cfg(not(windows))]
fn read_video(_path: &Path, _out: &mut dyn FnMut(Loaded) -> bool) -> Result<(), String> {
    Err("Videos only open on Windows for now.".to_string())
}

#[cfg(windows)]
fn read_video(path: &Path, out: &mut dyn FnMut(Loaded) -> bool) -> Result<(), String> {
    windows_video::read(path, out)
}

#[cfg(windows)]
mod windows_video {
    use std::path::Path;

    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, mpsc};

    use image::RgbImage;
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
    use windows::core::HSTRING;

    use super::{Frame, Loaded, MEMORY_LIMIT, VIDEO_SIDE, video_frame};

    /// How a decoded frame's bytes are laid out.
    struct Layout {
        width: u32,
        height: u32,
        /// Bytes from one row to the next. Negative means the rows run bottom up.
        stride: i32,
        /// The part of the frame meant to be seen. Decoders pad 1080 rows to 1088.
        shown: (u32, u32, u32, u32),
    }

    impl Layout {
        unsafe fn of(kind: &IMFMediaType) -> Result<Self, String> {
            unsafe {
                let size = kind.GetUINT64(&MF_MT_FRAME_SIZE).map_err(|e| e.message())?;
                let (width, height) = ((size >> 32) as u32, size as u32);
                let stride = kind.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| s as i32).unwrap_or(width as i32 * 4);
                let mut area = [0u8; 16];
                let shown = match kind.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, &mut area, None) {
                    Ok(()) => {
                        let area: MFVideoArea = std::ptr::read_unaligned(area.as_ptr().cast());
                        let x = area.OffsetX.value.max(0) as u32;
                        let y = area.OffsetY.value.max(0) as u32;
                        let w = (area.Area.cx.max(1) as u32).min(width.saturating_sub(x));
                        let h = (area.Area.cy.max(1) as u32).min(height.saturating_sub(y));
                        if w == 0 || h == 0 { (0, 0, width, height) } else { (x, y, w, h) }
                    }
                    Err(_) => (0, 0, width, height),
                };
                Ok(Self { width, height, stride, shown })
            }
        }

        /// The shown part of a frame of blue, green, red and an unused byte, shrunk by a whole
        /// number of times while it's read, so a 1080p frame comes out at 960 by 540 in one
        /// pass. Each pixel is the average of the block it stands for.
        fn picture(&self, bytes: &[u8], side: u32) -> Option<RgbImage> {
            let row = self.stride.unsigned_abs() as usize;
            if row < self.width as usize * 4 || bytes.len() < row * self.height as usize {
                return None;
            }
            let (x0, y0, w, h) = self.shown;
            let step = (w.max(h) / side.max(1)).max(1);
            let (out_w, out_h) = ((w / step).max(1) as usize, (h / step).max(1) as usize);
            let mut rgb = vec![0u8; out_w * out_h * 3];
            let mut sums = vec![0u32; out_w * 3];
            let block = step * step;
            for oy in 0..out_h {
                sums.fill(0);
                for dy in 0..step {
                    let y = y0 + oy as u32 * step + dy;
                    let source = if self.stride < 0 { self.height - 1 - y } else { y } as usize;
                    let line = &bytes[source * row..source * row + self.width as usize * 4];
                    for (ox, sum) in sums.chunks_exact_mut(3).enumerate() {
                        let start = (x0 + ox as u32 * step) as usize * 4;
                        for pixel in line[start..start + step as usize * 4].chunks_exact(4) {
                            sum[0] += pixel[2] as u32;
                            sum[1] += pixel[1] as u32;
                            sum[2] += pixel[0] as u32;
                        }
                    }
                }
                for (out, sum) in rgb[oy * out_w * 3..(oy + 1) * out_w * 3].iter_mut().zip(&sums) {
                    *out = (sum / block) as u8;
                }
            }
            RgbImage::from_raw(out_w as u32, out_h as u32, rgb)
        }
    }

    pub fn read(path: &Path, out: &mut dyn FnMut(Loaded) -> bool) -> Result<(), String> {
        unsafe {
            // Already set up on this thread is fine too.
            let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            let result = match MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                Ok(()) => {
                    let result = read_frames(path, out);
                    let _ = MFShutdown();
                    result
                }
                Err(e) => Err(e.message()),
            };
            if com {
                CoUninitialize();
            }
            result
        }
    }

    /// Hands frames on in the order they were read, whichever helper finishes first.
    struct InOrder {
        next: usize,
        waiting: BTreeMap<usize, Option<Frame>>,
        memory: usize,
        stopped: bool,
    }

    impl InOrder {
        /// Passes on everything that's ready. Returns false once `out` has had enough.
        fn pass(&mut self, out: &mut dyn FnMut(Loaded) -> bool) -> bool {
            while !self.stopped {
                let Some(frame) = self.waiting.remove(&self.next) else {
                    break;
                };
                self.next += 1;
                let Some(frame) = frame else {
                    continue;
                };
                self.memory += frame.pixels.bytes();
                let time = frame.time;
                if !out(Loaded::Frame(frame)) {
                    self.stopped = true;
                } else if self.memory > MEMORY_LIMIT {
                    out(Loaded::Cut(time));
                    self.stopped = true;
                }
            }
            !self.stopped
        }
    }

    unsafe fn read_frames(path: &Path, out: &mut dyn FnMut(Loaded) -> bool) -> Result<(), String> {
        unsafe {
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 1).map_err(|e| e.message())?;
            let attributes = attributes.ok_or("Media Foundation didn't start.")?;
            // Lets the reader turn any video into plain RGB.
            attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1).map_err(|e| e.message())?;
            let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), &attributes)
                .map_err(|_| "Windows can't play this video. It may need a codec, such as the VP9 or HEVC extension from the Microsoft Store.".to_string())?;
            let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
            reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false).map_err(|e| e.message())?;
            reader.SetStreamSelection(video, true).map_err(|_| "This file has no video in it.".to_string())?;
            let wanted = MFCreateMediaType().map_err(|e| e.message())?;
            wanted.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|e| e.message())?;
            wanted.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).map_err(|e| e.message())?;
            reader.SetCurrentMediaType(video, None, &wanted).map_err(|_| "Windows can't turn this video into pictures.".to_string())?;
            let mut layout = Layout::of(&reader.GetCurrentMediaType(video).map_err(|e| e.message())?)?;
            // Phones film sideways and note how to turn it.
            let rotation = reader.GetNativeMediaType(video, 0).and_then(|t| t.GetUINT32(&MF_MT_VIDEO_ROTATION)).unwrap_or(0);
            let fps = reader
                .GetCurrentMediaType(video)
                .and_then(|t| t.GetUINT64(&MF_MT_FRAME_RATE))
                .map(|r| (r >> 32) as f64 / (r as u32).max(1) as f64)
                .unwrap_or(30.0) as f32;
            let duration = reader
                .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
                .map(|value| value.Anonymous.Anonymous.Anonymous.uhVal as f64 / 1e7)
                .unwrap_or(0.0);
            if !out(Loaded::Started { duration, fps: if fps.is_finite() && fps > 0.0 { fps } else { 30.0 } }) {
                return Ok(());
            }
            // Media Foundation decodes on this thread, and helpers shrink and compress, which
            // takes as long again.
            let helpers = std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(2).clamp(1, 3));
            let (jobs, job_queue) = mpsc::sync_channel::<(usize, f64, RgbImage)>(helpers * 2);
            let job_queue = Arc::new(Mutex::new(job_queue));
            let (finished, results) = mpsc::channel::<(usize, Option<Frame>)>();
            let mut order = InOrder { next: 0, waiting: BTreeMap::new(), memory: 0, stopped: false };
            std::thread::scope(|scope| {
                for _ in 0..helpers {
                    let (queue, finished) = (job_queue.clone(), finished.clone());
                    scope.spawn(move || {
                        loop {
                            let job = queue.lock().map_err(|_| ()).and_then(|q| q.recv().map_err(|_| ()));
                            let Ok((index, time, picture)) = job else {
                                break;
                            };
                            if finished.send((index, video_frame(time, picture, rotation))).is_err() {
                                break;
                            }
                        }
                    });
                }
                drop(finished);
                let mut read = || -> Result<(), String> {
                    let (mut first, mut count) = (None, 0usize);
                    loop {
                        let (mut flags, mut time, mut sample) = (0u32, 0i64, None);
                        reader.ReadSample(video, 0, None, Some(&mut flags), Some(&mut time), Some(&mut sample)).map_err(|e| e.message())?;
                        if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
                            layout = Layout::of(&reader.GetCurrentMediaType(video).map_err(|e| e.message())?)?;
                        }
                        if let Some(sample) = sample {
                            let buffer = sample.ConvertToContiguousBuffer().map_err(|e| e.message())?;
                            let (mut data, mut length) = (std::ptr::null_mut(), 0u32);
                            buffer.Lock(&mut data, None, Some(&mut length)).map_err(|e| e.message())?;
                            let picture = layout.picture(std::slice::from_raw_parts(data, length as usize), VIDEO_SIDE);
                            let _ = buffer.Unlock();
                            let start = *first.get_or_insert(time);
                            let seconds = (time - start) as f64 / 1e7;
                            match picture {
                                Some(picture) => {
                                    if jobs.send((count, seconds, picture)).is_err() {
                                        return Ok(());
                                    }
                                }
                                None => {
                                    order.waiting.insert(count, None);
                                }
                            }
                            count += 1;
                        }
                        while let Ok((index, frame)) = results.try_recv() {
                            order.waiting.insert(index, frame);
                        }
                        if !order.pass(out) || flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                            return Ok(());
                        }
                    }
                };
                let result = read();
                drop(jobs);
                // The helpers finish what they hold, then the rest goes out in order.
                for (index, frame) in results.iter() {
                    order.waiting.insert(index, frame);
                    order.pass(out);
                }
                result
            })
        }
    }
}
