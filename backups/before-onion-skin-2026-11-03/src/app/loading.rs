//! What covers the app while it isn't ready: the intro once at startup, which plays
//! to its end and holds there until everything's loaded, then a looping animation for
//! any big load after that. A load has to run a moment before the loop shows, since a
//! flash would be worse than the wait.
//!
//! The `.bin` files are made by `examples/pack_loader.rs`. Only the frame on screen is
//! ever decoded, so an animation costs one small texture and nothing while hidden.

use std::time::Instant;

use egui::{Color32, Context, Id, Order, Rect, Sense, TextureHandle, TextureId, TextureOptions, Vec2, pos2};

use super::theme;

/// How long a load has to run before the loop comes up.
const SHOW_AFTER: f32 = 0.3;
/// Once up, it stays this long, so it can't blink on and off.
const MIN_SHOWN: f32 = 0.7;
const FADE: f32 = 0.2;
const FRAMES_PER_SECOND: f32 = 60.0;
/// The most one painted frame can move an animation along. Without this, a slow first
/// frame would skip the start of the intro.
const LONGEST_STEP: f32 = 1.0 / 20.0;

/// One packed animation.
struct Clip {
    pack: &'static [u8],
    /// How big a frame is drawn. Frames are padded, so the art looks smaller than this.
    size: f32,
    looping: bool,
}

static INTRO: Clip = Clip {
    pack: include_bytes!("../../assets/intro.bin"),
    size: 300.0,
    looping: false,
};

static LOOP: Clip = Clip {
    pack: include_bytes!("../../assets/loader.bin"),
    size: 260.0,
    looping: true,
};

/// Twice as wide as it's tall. The popup that shows it decides how big that is.
static CONNECT: Clip = Clip {
    pack: include_bytes!("../../assets/connect.bin"),
    size: 0.0,
    looping: false,
};

impl Clip {
    fn read_u32(&self, at: usize) -> usize {
        u32::from_le_bytes(self.pack[at..at + 4].try_into().expect("four bytes")) as usize
    }

    fn frames(&self) -> usize {
        self.read_u32(0)
    }

    fn seconds(&self) -> f32 {
        self.frames() as f32 / FRAMES_PER_SECOND
    }

    fn frame_at(&self, seconds: f32) -> usize {
        let frame = (seconds * FRAMES_PER_SECOND) as usize;
        if self.looping { frame % self.frames() } else { frame.min(self.frames() - 1) }
    }

    fn decode(&self, index: usize) -> egui::ColorImage {
        let data = 4 + self.frames() * 4;
        let start = if index == 0 { 0 } else { self.read_u32(4 + (index - 1) * 4) };
        let bytes = &self.pack[data + start..data + self.read_u32(4 + index * 4)];
        let frame = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).expect("bundled frame is a valid image");
        let rgba = frame.to_rgba8();
        egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
    }
}

/// Where the screen is, with each stage's start in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    /// App start. Lasts as long as the intro, or until everything's loaded if that's later.
    Intro,
    Hidden,
    /// Something started, and it might finish before it's worth showing anything.
    Waiting(f32),
    Shown(f32),
    Fading(f32),
}

impl Stage {
    /// `played` is how far the intro has got, which a slow start can't cut short.
    fn next(self, now: f32, working: bool, played: f32) -> Stage {
        match self {
            Stage::Intro if !working && played >= INTRO.seconds() => Stage::Fading(now),
            Stage::Hidden if working => Stage::Waiting(now),
            Stage::Waiting(_) if !working => Stage::Hidden,
            Stage::Waiting(since) if now - since >= SHOW_AFTER => Stage::Shown(now),
            Stage::Shown(since) if !working && now - since >= MIN_SHOWN => Stage::Fading(now),
            Stage::Fading(_) if working => Stage::Shown(now),
            Stage::Fading(since) if now - since >= FADE => Stage::Hidden,
            stage => stage,
        }
    }

    /// How solid the screen is, when it's up at all.
    fn opacity(self, now: f32) -> Option<f32> {
        match self {
            Stage::Intro | Stage::Shown(_) => Some(1.0),
            Stage::Fading(since) => Some(1.0 - ((now - since) / FADE).clamp(0.0, 1.0)),
            Stage::Hidden | Stage::Waiting(_) => None,
        }
    }
}

/// The frame showing, so an unchanged one doesn't get decoded again.
struct Shown {
    texture: TextureHandle,
    looping: bool,
    frame: usize,
}

pub struct Loader {
    clock: Instant,
    stage: Stage,
    clip: &'static Clip,
    /// How far into the clip it's played.
    played: f32,
    /// When the last frame was painted, once one has been.
    painted_at: Option<f32>,
    shown: Option<Shown>,
}

impl Loader {
    /// Starts on the intro, since the app has work to do from its very first frame.
    pub fn new() -> Self {
        Self {
            clock: Instant::now(),
            stage: Stage::Intro,
            clip: &INTRO,
            played: 0.0,
            painted_at: None,
            shown: None,
        }
    }

    fn now(&self) -> f32 {
        self.clock.elapsed().as_secs_f32()
    }

    /// Whether the screen is up, which also means it owns the mouse and keyboard.
    pub fn is_covering(&self) -> bool {
        self.stage.opacity(self.now()).is_some()
    }

    /// Moves the screen along, and draws it over everything if it's up.
    pub fn show(&mut self, ctx: &Context, working: bool) {
        let now = self.now();
        let before = self.stage;
        self.stage = before.next(now, working, self.played);
        let Some(opacity) = self.stage.opacity(now) else {
            // Nothing else is drawing frames, and a hidden wait still has to end.
            self.shown = None;
            self.painted_at = None;
            if matches!(self.stage, Stage::Waiting(_)) {
                ctx.request_repaint();
            }
            return;
        };
        if matches!(self.stage, Stage::Shown(since) if since == now) {
            self.clip = &LOOP;
            self.played = 0.0;
        } else if let Some(last) = self.painted_at {
            self.played += (now - last).min(LONGEST_STEP);
        }
        self.painted_at = Some(now);

        let frame = self.clip.frame_at(self.played);
        let looping = self.clip.looping;
        match &mut self.shown {
            Some(shown) if shown.frame == frame && shown.looping == looping => {}
            Some(shown) => {
                shown.texture.set(self.clip.decode(frame), TextureOptions::LINEAR);
                shown.frame = frame;
                shown.looping = looping;
            }
            None => {
                let texture = ctx.load_texture("loading animation", self.clip.decode(frame), TextureOptions::LINEAR);
                self.shown = Some(Shown { texture, looping, frame });
            }
        }
        let area = ctx.content_rect();
        let art = self.shown.as_ref().map(|shown| shown.texture.id());
        let art_size = self.clip.size;
        egui::Area::new(Id::unique("kaye loading screen"))
            .order(Order::Tooltip)
            .fixed_pos(area.min)
            .show(ctx, |ui| {
                // Claims the whole window, so nothing under the cover reacts to the mouse.
                ui.allocate_rect(area, Sense::click_and_drag());
                let painter = ui.painter();
                painter.rect_filled(area, 0.0, theme::cover().gamma_multiply(opacity));
                if let Some(art) = art {
                    let size = Vec2::splat(art_size.min(area.width()).min(area.height()));
                    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                    painter.image(art, Rect::from_center_size(area.center(), size), uv, Color32::WHITE.gamma_multiply(opacity));
                }
            });
        ctx.request_repaint();
    }
}

/// An animation drawn inside a page. It plays once and stays on its last frame.
pub struct PlayOnce {
    clip: &'static Clip,
    clock: Instant,
    played: f32,
    painted_at: Option<f32>,
    shown: Option<(TextureHandle, usize)>,
}

impl PlayOnce {
    pub fn connect() -> Self {
        Self { clip: &CONNECT, clock: Instant::now(), played: 0.0, painted_at: None, shown: None }
    }

    /// Moves along by the time since the last call, and gives the frame to draw.
    pub fn frame(&mut self, ctx: &Context) -> TextureId {
        let now = self.clock.elapsed().as_secs_f32();
        if let Some(last) = self.painted_at {
            self.played += (now - last).min(LONGEST_STEP);
        }
        self.painted_at = Some(now);
        let frame = self.clip.frame_at(self.played);
        if frame + 1 < self.clip.frames() {
            ctx.request_repaint();
        }
        match &mut self.shown {
            Some((texture, showing)) => {
                if *showing != frame {
                    texture.set(self.clip.decode(frame), TextureOptions::LINEAR);
                    *showing = frame;
                }
                texture.id()
            }
            None => {
                let texture = ctx.load_texture("connect animation", self.clip.decode(frame), TextureOptions::LINEAR);
                let id = texture.id();
                self.shown = Some((texture, frame));
                id
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_clip(clip: &Clip) {
        assert!(clip.frames() > 1);
        let first = clip.decode(0);
        for index in 0..clip.frames() {
            assert_eq!(clip.decode(index).size, first.size, "frame {index}");
        }
        // The intro opens on almost nothing, so this looks partway through.
        let middle = clip.decode(clip.frames() / 2);
        assert!(middle.pixels.iter().any(|p| p.a() == 0), "the background has to stay clear");
        assert!(middle.pixels.iter().any(|p| p.a() == 255), "and the art has to be solid");
    }

    #[test]
    fn every_frame_of_every_animation_decodes_at_one_size() {
        check_clip(&INTRO);
        check_clip(&LOOP);
        check_clip(&CONNECT);
    }

    #[test]
    fn the_connect_animation_stops_on_its_last_frame() {
        assert_eq!(CONNECT.frame_at(0.0), 0);
        assert_eq!(CONNECT.frame_at(CONNECT.seconds() * 3.0), CONNECT.frames() - 1);
        let first = CONNECT.decode(0);
        assert_eq!(first.size[0], first.size[1] * 2, "it's twice as wide as it's tall");
    }

    #[test]
    fn the_intro_keeps_its_color_and_the_loop_stays_grey() {
        let rose = INTRO.decode(INTRO.frames() - 1);
        assert!(rose.pixels.iter().filter(|p| p.a() == 255).any(|p| p.r() as i32 - p.b() as i32 > 40));
        let grey = LOOP.decode(0);
        assert!(grey.pixels.iter().filter(|p| p.a() == 255).all(|p| p.r() == p.g() && p.g() == p.b()));
    }

    #[test]
    fn the_intro_plays_once_and_the_loop_wraps() {
        assert_eq!(INTRO.frame_at(0.0), 0);
        assert_eq!(INTRO.frame_at(INTRO.seconds() * 3.0), INTRO.frames() - 1, "it holds on the finished logo");
        assert_eq!(LOOP.frame_at(0.0), 0);
        assert!(LOOP.frame_at(LOOP.seconds() + 0.5) < LOOP.frames() / 2, "it comes back around");
    }

    #[test]
    fn the_intro_finishes_first_and_waits_for_loading() {
        let length = INTRO.seconds();
        assert_eq!(Stage::Intro.next(9.0, false, length - 0.1), Stage::Intro, "loaded early still plays it out");
        assert_eq!(Stage::Intro.next(9.0, true, length + 30.0), Stage::Intro, "a slow load holds on the last frame");
        let fading = Stage::Intro.next(9.0, false, length);
        assert_eq!(fading, Stage::Fading(9.0), "the fade runs on the real clock, whatever the intro's took");
        assert!((fading.opacity(9.0 + FADE * 0.5).unwrap() - 0.5).abs() < 1e-3);
    }

    #[test]
    fn stays_up_for_the_minimum_then_fades_away() {
        let stage = Stage::Shown(0.0);
        assert_eq!(stage.next(MIN_SHOWN - 0.1, false, 0.0), stage, "too soon to leave");
        assert_eq!(stage.next(MIN_SHOWN + 5.0, true, 0.0), stage, "still working");
        let fading = stage.next(MIN_SHOWN, false, 0.0);
        assert_eq!(fading, Stage::Fading(MIN_SHOWN));
        assert!((fading.opacity(MIN_SHOWN + FADE * 0.5).unwrap() - 0.5).abs() < 1e-3);
        assert_eq!(fading.next(MIN_SHOWN + FADE + 0.01, false, 0.0), Stage::Hidden);
        assert_eq!(Stage::Hidden.opacity(0.0), None);
    }

    #[test]
    fn new_work_during_the_fade_brings_it_back() {
        assert_eq!(Stage::Fading(1.0).next(1.05, true, 0.0), Stage::Shown(1.05));
    }

    #[test]
    fn a_load_only_shows_once_it_has_taken_a_while() {
        let waiting = Stage::Hidden.next(1.0, true, 0.0);
        assert_eq!(waiting, Stage::Waiting(1.0));
        assert_eq!(waiting.next(1.0 + SHOW_AFTER * 0.5, false, 0.0), Stage::Hidden, "quick loads never show");
        assert_eq!(waiting.next(1.0 + SHOW_AFTER * 0.5, true, 0.0), waiting, "and nothing shows yet");
        assert_eq!(waiting.next(1.0 + SHOW_AFTER + 0.01, true, 0.0), Stage::Shown(1.0 + SHOW_AFTER + 0.01));
    }
}
