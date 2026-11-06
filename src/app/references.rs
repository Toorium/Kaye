//! Reference windows: a picture, GIF or video in a window of its own, showing the moment
//! the timeline is on. Each can float over Kaye or pop out into a real window for a second
//! screen. Clips are read by `crate::media` on another thread, and each window decodes the
//! frame it needs on a thread of its own, so scrubbing never waits on a picture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::Arc;

use egui::{Align2, Color32, ColorImage, FontId, Frame, Id, Margin, Rect, RichText, Sense, TextureHandle, TextureOptions, Ui, Vec2, ViewportBuilder, ViewportId, pos2, vec2};

use super::chrome;
use super::icons::{self, Icon};
use super::{KayeApp, ToastKind, theme};
use crate::media::{self, Loaded, MediaKind};
use crate::reference::{Reference, WindowPlace, next_id};

const HEADER: f32 = 24.0;
/// A new window's width, with its height from the picture's shape.
const NEW_WIDTH: f32 = 360.0;

/// The windows and what each is showing.
#[derive(Default)]
pub struct References {
    players: HashMap<u64, Player>,
    places: HashMap<u64, Place>,
    /// Files picked in the dialog, on their way back from its thread.
    picked: Option<Receiver<Vec<PathBuf>>>,
}

#[derive(Clone, Copy)]
struct Place {
    rect: Rect,
    open: bool,
    popped: bool,
    /// Bring the window to the front next frame.
    raise: bool,
    /// Just added, so its window takes the picture's shape once the first frame is in.
    fresh: bool,
}

impl References {
    /// Where every window is, for saving with the view.
    pub fn places(&self) -> Vec<WindowPlace> {
        let mut places: Vec<WindowPlace> = self
            .places
            .iter()
            .map(|(id, p)| WindowPlace { id: *id, rect: [p.rect.min.x, p.rect.min.y, p.rect.width(), p.rect.height()], open: p.open, popped: p.popped })
            .collect();
        places.sort_by_key(|p| p.id);
        places
    }

    /// Puts windows back where a project left them.
    pub fn restore(&mut self, places: &[WindowPlace]) {
        self.clear();
        for p in places {
            let rect = Rect::from_min_size(pos2(p.rect[0], p.rect[1]), vec2(p.rect[2].max(120.0), p.rect[3].max(90.0)));
            self.places.insert(p.id, Place { rect, open: p.open, popped: p.popped, raise: false, fresh: false });
        }
    }

    pub fn clear(&mut self) {
        self.players.clear();
        self.places.clear();
    }

    pub fn is_open(&self, id: u64) -> bool {
        self.places.get(&id).is_some_and(|p| p.open)
    }

    /// Opens a reference's window, or brings it to the front if it's open.
    pub fn show(&mut self, id: u64) {
        let place = self.places.entry(id).or_insert_with(|| new_place(id, None));
        place.open = true;
        place.raise = true;
    }

    pub fn hide(&mut self, id: u64) {
        if let Some(place) = self.places.get_mut(&id) {
            place.open = false;
        }
    }

    /// A small picture of the frame showing, for the timeline's strip.
    pub fn texture(&self, id: u64) -> Option<&TextureHandle> {
        self.players.get(&id)?.texture.as_ref()
    }

    /// How much of a clip has been read, from 0 to 1, while it's still reading.
    pub fn loading(&self, id: u64) -> Option<f32> {
        let player = self.players.get(&id)?;
        (!player.done && player.error.is_none()).then(|| player.progress())
    }
}

/// Where a new window opens: down and to the right of the last one, so they don't stack.
fn new_place(id: u64, aspect: Option<f32>) -> Place {
    let step = (id % 6) as f32 * 28.0;
    let height = NEW_WIDTH / aspect.unwrap_or(16.0 / 9.0).clamp(0.3, 4.0) + HEADER;
    Place { rect: Rect::from_min_size(pos2(80.0 + step, 120.0 + step), vec2(NEW_WIDTH, height)), open: true, popped: false, raise: true, fresh: true }
}

/// One reference's clip as it's read, and the frame its window shows.
struct Player {
    path: String,
    loading: Option<Receiver<Loaded>>,
    /// Stops the reading thread when the reference goes.
    stop: Arc<AtomicBool>,
    frames: Vec<Arc<media::Frame>>,
    duration: f64,
    fps: f32,
    done: bool,
    error: Option<String>,
    cut: Option<f64>,
    decode: Sender<(usize, Arc<media::Frame>)>,
    decoded: Receiver<(usize, ColorImage)>,
    texture: Option<TextureHandle>,
    shown: Option<usize>,
    asked: Option<usize>,
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Player {
    fn start(ctx: &egui::Context, path: &str) -> Self {
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (file, halt, repaint) = (PathBuf::from(path), stop.clone(), ctx.clone());
        std::thread::spawn(move || {
            media::read(&file, &mut |loaded| {
                let sent = tx.send(loaded).is_ok();
                repaint.request_repaint();
                sent && !halt.load(Ordering::Relaxed)
            });
        });
        // Decodes only the newest frame asked for, skipping any it fell behind on.
        let (decode, jobs) = channel::<(usize, Arc<media::Frame>)>();
        let (done, decoded) = channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            while let Ok(mut job) = jobs.recv() {
                while let Ok(newer) = jobs.try_recv() {
                    job = newer;
                }
                let (index, frame) = job;
                if let Some(rgba) = frame.pixels.rgba() {
                    let image = ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &rgba);
                    if done.send((index, image)).is_err() {
                        break;
                    }
                    repaint.request_repaint();
                }
            }
        });
        Self {
            path: path.to_string(),
            loading: Some(rx),
            stop,
            frames: Vec::new(),
            duration: 0.0,
            fps: 0.0,
            done: false,
            error: None,
            cut: None,
            decode,
            decoded,
            texture: None,
            shown: None,
            asked: None,
        }
    }

    fn progress(&self) -> f32 {
        match self.frames.last() {
            Some(last) if self.duration > 0.0 => (last.time / self.duration).clamp(0.0, 1.0) as f32,
            _ => 0.0,
        }
    }

    /// The picture's width over its height, once a frame has come in.
    fn aspect(&self) -> Option<f32> {
        self.frames.first().map(|f| f.width as f32 / f.height.max(1) as f32)
    }

    /// The last frame that starts at or before `time`.
    fn frame_at(&self, time: f64) -> Option<usize> {
        if self.frames.is_empty() {
            return None;
        }
        Some(self.frames.partition_point(|f| f.time <= time + 1e-6).saturating_sub(1))
    }

    /// Takes in what the reading thread has found. Returns what the reference should now say
    /// about the clip's length and rate.
    fn poll(&mut self) {
        let Some(rx) = &self.loading else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(Loaded::Started { duration, fps }) => {
                    self.duration = duration;
                    self.fps = fps;
                }
                Ok(Loaded::Frame(frame)) => self.frames.push(Arc::new(frame)),
                Ok(Loaded::Cut(at)) => self.cut = Some(at),
                Ok(Loaded::Done) => {
                    self.finish();
                    break;
                }
                Ok(Loaded::Failed(error)) => {
                    self.error = Some(error);
                    self.loading = None;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.finish();
                    break;
                }
            }
        }
    }

    /// The real length and rate come from the frames themselves, since a file's own
    /// numbers can be off, as WebM's frame rate often is.
    fn finish(&mut self) {
        self.loading = None;
        self.done = true;
        if self.frames.len() >= 2 {
            let last = self.frames[self.frames.len() - 1].time;
            if last > 0.0 {
                self.fps = ((self.frames.len() - 1) as f64 / last) as f32;
                self.duration = last + 1.0 / self.fps as f64;
            }
        } else if self.frames.len() == 1 && self.duration > 0.0 {
            // A GIF of one frame is a picture.
            self.duration = 0.0;
        }
        if let Some(cut) = self.cut {
            self.duration = self.duration.min(cut);
        }
    }

    /// Asks for the frame at `time`, and puts any finished one on screen.
    fn want(&mut self, ctx: &egui::Context, id: u64, time: f64) {
        while let Ok((index, image)) = self.decoded.try_recv() {
            match &mut self.texture {
                Some(texture) => texture.set(image, TextureOptions::LINEAR),
                None => self.texture = Some(ctx.load_texture(format!("reference-{id}"), image, TextureOptions::LINEAR)),
            }
            self.shown = Some(index);
        }
        let Some(index) = self.frame_at(time) else {
            return;
        };
        if self.asked != Some(index) {
            self.asked = Some(index);
            let _ = self.decode.send((index, self.frames[index].clone()));
        }
    }
}

/// Opens the file picker for new references.
pub fn pick_files(app: &mut KayeApp, ctx: &egui::Context) {
    let (tx, rx) = channel();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        let picked = rfd::FileDialog::new()
            .set_title("Add a reference")
            .add_filter("Pictures and videos", &media::extensions())
            .add_filter("All files", &["*"])
            .pick_files()
            .unwrap_or_default();
        let _ = tx.send(picked);
        repaint.request_repaint();
    });
    app.references.picked = Some(rx);
}

/// Whether a dropped file makes a reference rather than being imported.
pub fn is_reference_file(path: &Path) -> bool {
    MediaKind::of(path).is_some()
}

/// Adds a reference starting at the animation's first frame, and opens its window.
pub fn add(app: &mut KayeApp, path: &Path) {
    let name = path.file_stem().map_or_else(|| "Reference".to_string(), |s| s.to_string_lossy().into_owned());
    let id = next_id(&app.scene.references);
    let reference = Reference::new(id, path.to_string_lossy().into_owned(), name, app.animation.start as f64);
    app.scene.references.push(reference);
    app.references.places.insert(id, new_place(id, None));
}

pub fn delete(app: &mut KayeApp, id: u64) {
    app.scene.references.retain(|r| r.id != id);
    app.references.players.remove(&id);
    app.references.places.remove(&id);
}

/// Timeline frames per second of clip, from Preferences.
pub fn rate(app: &KayeApp, reference: &Reference) -> f64 {
    reference.rate(app.animation.fps, app.settings.animation.match_reference_fps)
}

/// Runs every frame: takes in picked files, reading progress and decoded frames, and asks for
/// the frame each open window needs.
pub fn update(app: &mut KayeApp, ctx: &egui::Context) {
    if let Some(rx) = &app.references.picked {
        match rx.try_recv() {
            Ok(paths) => {
                app.references.picked = None;
                for path in paths {
                    add(app, &path);
                }
            }
            Err(TryRecvError::Disconnected) => app.references.picked = None,
            Err(TryRecvError::Empty) => {}
        }
    }
    let live: Vec<u64> = app.scene.references.iter().map(|r| r.id).collect();
    app.references.players.retain(|id, _| live.contains(id));
    let frame = app.frame as f64;
    let length = (app.animation.end - app.animation.start).max(1) as f64;
    for index in 0..app.scene.references.len() {
        let id = app.scene.references[index].id;
        let path = app.scene.references[index].path.clone();
        let player = app.references.players.entry(id).or_insert_with(|| Player::start(ctx, &path));
        if player.path != path {
            *player = Player::start(ctx, &path);
        }
        let was_loading = player.loading.is_some();
        player.poll();
        let (duration, fps, failed) = (player.duration, player.fps, player.error.clone());
        let started = player.duration > 0.0 || !player.frames.is_empty();
        let aspect = player.aspect();
        if was_loading {
            if let Some(error) = failed {
                let name = app.scene.references[index].name.clone();
                app.toast(ToastKind::Error, format!("{name}: {error}"));
            }
        }
        // Only write what really changed, since anything written counts as unsaved work.
        let reference = &mut app.scene.references[index];
        if started && ((reference.duration - duration).abs() > 1e-4 || (reference.fps - fps).abs() > 1e-3) {
            reference.duration = duration;
            reference.fps = fps;
            if !reference.is_picture() {
                reference.trim_out = reference.trim_out.min(duration);
            }
        }
        if started && reference.trim_out <= reference.trim_in && (reference.is_picture() || duration > 0.0) {
            let rate = reference.rate(app.animation.fps, app.settings.animation.match_reference_fps);
            reference.fill_length(length, rate);
        }
        // A new window takes the picture's shape once its first frame is in.
        if let (Some(place), Some(aspect)) = (app.references.places.get_mut(&id), aspect) {
            if place.fresh {
                place.rect = Rect::from_min_size(place.rect.min, vec2(NEW_WIDTH, NEW_WIDTH / aspect.clamp(0.3, 4.0) + HEADER));
                place.fresh = false;
            }
        }
        let reference = &app.scene.references[index];
        let rate = rate(app, reference);
        let time = reference.time_at(frame, rate);
        let open = app.references.is_open(id);
        let player = app.references.players.get_mut(&id).expect("made above");
        if open || player.texture.is_none() {
            player.want(ctx, id, time);
        }
    }
}

/// Draws every open window. Called after the panels, so the windows float over them.
pub fn windows(app: &mut KayeApp, ctx: &egui::Context) {
    for index in 0..app.scene.references.len() {
        let id = app.scene.references[index].id;
        let Some(mut place) = app.references.places.get(&id).copied() else {
            continue;
        };
        if !place.open {
            continue;
        }
        // A new window waits a moment for its first frame, so it opens in the picture's shape.
        let waiting = app.references.players.get(&id).is_none_or(|p| p.frames.is_empty() && p.error.is_none());
        if place.fresh && waiting {
            continue;
        }
        if place.popped {
            pop_out(app, ctx, index, &mut place);
        } else {
            floating(app, ctx, index, &mut place);
        }
        app.references.places.insert(id, place);
    }
}

fn floating(app: &mut KayeApp, ctx: &egui::Context, index: usize, place: &mut Place) {
    let reference = &app.scene.references[index];
    let frame = Frame::window(&ctx.global_style()).inner_margin(Margin::same(4));
    // The path is part of the id, so another project's reference with the same number
    // doesn't open where this one was.
    let shown = egui::Window::new(reference.name.clone())
        .id(Id::unique(("reference", reference.id, &reference.path)))
        .title_bar(false)
        .resizable(true)
        .frame(frame)
        .min_size(vec2(140.0, 90.0))
        .default_pos(place.rect.min)
        .default_size(place.rect.size())
        .constrain_to(ctx.content_rect())
        .show(ctx, |ui| contents(app, ui, index, place));
    if let Some(shown) = shown {
        place.rect = shown.response.rect;
        if std::mem::take(&mut place.raise) {
            ctx.move_to_top(shown.response.layer_id);
        }
    }
}

fn pop_out(app: &mut KayeApp, ctx: &egui::Context, index: usize, place: &mut Place) {
    let reference = &app.scene.references[index];
    let viewport = ViewportId::from_hash_of(("reference", reference.id, &reference.path));
    let builder = ViewportBuilder::default()
        .with_title(format!("{} - Kaye", reference.name))
        .with_inner_size(place.rect.size())
        .with_min_inner_size([160.0, 120.0]);
    let mut closed = false;
    ctx.show_viewport_immediate(viewport, builder, |ui, _class| {
        let panel = Frame::new().fill(theme::panel()).inner_margin(Margin::same(4));
        egui::CentralPanel::default().frame(panel).show(ui, |ui| {
            contents(app, ui, index, place);
        });
        // Its own window's X puts it back inside Kaye rather than losing it.
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            closed = true;
        }
        if let Some(inner) = ui.ctx().input(|i| i.viewport().inner_rect) {
            place.rect = Rect::from_min_size(place.rect.min, inner.size());
        }
    });
    if closed {
        place.popped = false;
    }
}

/// The header with the name and buttons, then the picture fitted to what's left.
fn contents(app: &mut KayeApp, ui: &mut Ui, index: usize, place: &mut Place) {
    let reference = &app.scene.references[index];
    let id = reference.id;
    let name = reference.name.clone();
    let picture = reference.is_picture() && MediaKind::of(Path::new(&reference.path)) == Some(MediaKind::Picture);
    ui.horizontal(|ui| {
        ui.set_height(HEADER - 4.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        icons::paint(ui.painter(), rect, if picture { Icon::Image } else { Icon::Animations }, theme::text_dim());
        ui.label(RichText::new(&name).color(theme::text()));
        if let Some(progress) = app.references.loading(id) {
            ui.label(RichText::new(format!("{:.0}%", progress * 100.0)).color(theme::text_faint()));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            if chrome::icon_button(ui, Icon::Close, true, false, "Close").clicked() {
                place.open = false;
            }
            let (icon, tip) = if place.popped { (Icon::PopIn, "Back into Kaye") } else { (Icon::PopOut, "Own Window") };
            if chrome::icon_button(ui, icon, true, false, tip).clicked() {
                place.popped = !place.popped;
            }
        });
    });
    let area = ui.available_rect_before_wrap();
    let (area, _) = ui.allocate_exact_size(area.size().max(vec2(1.0, 1.0)), Sense::hover());
    let painter = ui.painter_at(area);
    painter.rect_filled(area, 2.0, Color32::from_gray(18));
    let player = app.references.players.get(&id);
    match player.and_then(|p| p.texture.as_ref().map(|t| (p, t))) {
        Some((_, texture)) => {
            let size = texture.size_vec2();
            let scale = (area.width() / size.x).min(area.height() / size.y);
            let shown = Rect::from_center_size(area.center(), size * scale);
            painter.image(texture.id(), shown, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            let text = match player.and_then(|p| p.error.clone()) {
                Some(error) => error,
                None => "Loading".to_string(),
            };
            let galley = painter.layout(text, FontId::proportional(12.0), theme::text_dim(), area.width() - 16.0);
            painter.galley(area.center() - galley.size() * 0.5, galley, theme::text_dim());
        }
    }
    if let Some(cut) = player.and_then(|p| p.cut) {
        let text = format!("Only the first {cut:.0} seconds fit in memory");
        painter.text(area.left_bottom() + vec2(6.0, -6.0), Align2::LEFT_BOTTOM, text, FontId::proportional(11.0), theme::warning());
    }
}
