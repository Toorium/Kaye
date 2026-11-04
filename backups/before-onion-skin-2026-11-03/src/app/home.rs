//! The page Kaye opens on: start something new, or pick up a recent file or a backup.
//! The layout follows Photoshop's home screen, in Kaye's own colors.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{Align, CentralPanel, Color32, ColorImage, CursorIcon, FontId, Frame, Galley, Layout, Margin, Panel, Rect, Response, RichText, ScrollArea, Sense, TextureHandle, TextureId, TextureOptions, Ui, Vec2, pos2, vec2};

use super::icons::{self, Icon};
use super::{KayeApp, Leave, chrome, theme, viewport};
use crate::library::{self, Backup, Recent};
use crate::settings::{self, Action};

const HEADER_HEIGHT: f32 = 44.0;
const SIDEBAR_WIDTH: f32 = 216.0;
const ROW_HEIGHT: f32 = 30.0;
const CARD_WIDTH: f32 = 178.0;
const CARD_PICTURE: f32 = 134.0;
const CARD_HEIGHT: f32 = CARD_PICTURE + 42.0;
const CARD_GAP: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Recent,
    Backups,
}

pub struct Home {
    /// Starts open, and the first thing opened or made takes it away.
    pub open: bool,
    section: Section,
    recent: Vec<Recent>,
    backups: Vec<Backup>,
    /// Pictures of files, loaded the first time they're drawn. `None` means there isn't one.
    pictures: HashMap<PathBuf, Option<TextureHandle>>,
}

impl Home {
    pub fn new() -> Self {
        let mut home = Self { open: true, section: Section::Recent, recent: Vec::new(), backups: Vec::new(), pictures: HashMap::new() };
        home.refresh();
        home
    }

    pub fn show(&mut self) {
        self.open = true;
        self.section = Section::Recent;
        self.refresh();
    }

    /// The lists change while the page is away, and a saved file may have a new picture.
    fn refresh(&mut self) {
        self.recent = settings::recent_list().map_or_else(Vec::new, |list| library::recent_files(&list));
        self.backups = settings::backups_folder().map_or_else(Vec::new, |folder| library::backups(&folder));
        self.pictures.clear();
    }

    fn picture(&mut self, ctx: &egui::Context, file: &Path) -> Option<TextureId> {
        let picture = self.pictures.entry(file.to_path_buf()).or_insert_with(|| {
            let path = library::thumbnail_path(&settings::thumbnails_folder()?, file);
            let image = image::open(path).ok()?.to_rgba8();
            let image = ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.as_raw());
            Some(ctx.load_texture("file thumbnail", image, TextureOptions::LINEAR))
        });
        picture.as_ref().map(TextureHandle::id)
    }
}

pub fn panel(app: &mut KayeApp, ui: &mut Ui) {
    header(app, ui);
    Panel::left("home_sidebar")
        .exact_size(SIDEBAR_WIDTH)
        .resizable(false)
        .frame(Frame::new().fill(theme::panel()).inner_margin(Margin::symmetric(12, 18)))
        .show(ui, |ui| sidebar(app, ui));
    CentralPanel::default().frame(Frame::new().fill(theme::panel_deep())).show(ui, |ui| {
        let area = ui.max_rect();
        let picked = Frame::new()
            .inner_margin(Margin { left: 40, right: 24, top: 26, bottom: 0 })
            .show(ui, |ui| match app.home.section {
                Section::Recent => recent_section(app, ui),
                Section::Backups => backups_section(app, ui),
            })
            .inner;
        if let Some(path) = picked {
            app.request_leave(ui.ctx(), Leave::Open(Some(path)));
        }
        viewport::paint_toasts(app, ui.painter(), area);
        viewport::drop_overlay(ui.ctx(), ui.painter(), area);
    });
}

fn header(app: &mut KayeApp, ui: &mut Ui) {
    Panel::top("home_header").exact_size(HEADER_HEIGHT).frame(chrome::bar_frame(theme::header(), 12, 0)).show(ui, |ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let tip = app.tip(Action::Preferences);
            if chrome::icon_button(ui, Icon::Settings, true, app.prefs_open, &tip).clicked() {
                app.prefs_open = !app.prefs_open;
            }
        });
    });
}

fn sidebar(app: &mut KayeApp, ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = 2.0;
    let ctx = ui.ctx().clone();
    if new_button(ui).clicked() {
        app.request_leave(&ctx, Leave::New);
    }
    ui.add_space(6.0);
    if side_row(ui, Icon::Folder, "Open", false).clicked() {
        app.request_leave(&ctx, Leave::Open(None));
    }
    if side_row(ui, Icon::Import, "Import Rig", false).clicked() {
        app.open_import_dialog(&ctx);
    }
    if !app.scene.is_empty() && side_row(ui, Icon::StepBack, "Back to Project", false).clicked() {
        app.home.open = false;
    }
    ui.add_space(10.0);
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(line.x_range(), line.center().y, egui::Stroke::new(1.0, theme::field()));
    ui.add_space(10.0);
    if side_row(ui, Icon::Home, "Home", app.home.section == Section::Recent).clicked() {
        app.home.section = Section::Recent;
    }
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(RichText::new("FILES").small().color(theme::text_faint()));
    });
    ui.add_space(4.0);
    if side_row(ui, Icon::Reset, "Backups", app.home.section == Section::Backups).clicked() {
        app.home.section = Section::Backups;
    }
}

/// The one colored control on the page, as New file is in the reference.
fn new_button(ui: &mut Ui) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT + 2.0), Sense::click());
    let fill = if response.hovered() { theme::lighter(theme::accent()) } else { theme::accent() };
    let painter = ui.painter();
    painter.rect_filled(rect, theme::RADIUS, fill);
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, "New", theme::strong_font(13.0), theme::on_accent());
    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn side_row(ui: &mut Ui, icon: Icon, label: &str, active: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, theme::RADIUS, theme::active_row());
    } else if response.hovered() {
        painter.rect_filled(rect, theme::RADIUS, theme::header());
    }
    let color = if active || response.hovered() { theme::ink() } else { theme::text() };
    icons::paint(painter, Rect::from_center_size(rect.left_center() + vec2(18.0, 0.0), Vec2::splat(16.0)), icon, color);
    painter.text(rect.left_center() + vec2(36.0, 0.0), egui::Align2::LEFT_CENTER, label, FontId::proportional(13.0), color);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(theme::strong_font(15.0)).color(theme::ink()));
    ui.add_space(14.0);
}

fn nothing_here(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.5).color(theme::text_dim()));
}

/// One line of text cut off with an ellipsis once it's wider than `width`.
fn one_line(ui: &Ui, text: String, font: FontId, color: Color32, width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple(text, font, color, width);
    job.wrap = TextWrapping::truncate_at_width(width);
    ui.painter().layout_job(job)
}

fn recent_section(app: &mut KayeApp, ui: &mut Ui) -> Option<PathBuf> {
    heading(ui, "Recent");
    if app.home.recent.is_empty() {
        nothing_here(ui, "No recent files");
        return None;
    }
    let mut picked = None;
    let now = library::now();
    let files = app.home.recent.clone();
    ScrollArea::vertical().auto_shrink(false).id_salt("home_recent").show(ui, |ui| {
        let columns = (((ui.available_width() + CARD_GAP) / (CARD_WIDTH + CARD_GAP)) as usize).max(1);
        for row in files.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = CARD_GAP;
                for file in row {
                    let picture = app.home.picture(ui.ctx(), &file.path);
                    if card(ui, file, picture, now).clicked() {
                        picked = Some(file.path.clone());
                    }
                }
            });
            ui.add_space(20.0);
        }
    });
    picked
}

fn card(ui: &mut Ui, file: &Recent, picture: Option<TextureId>, now: u64) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(CARD_WIDTH, CARD_HEIGHT), Sense::click());
    let tile = Rect::from_min_size(rect.min, vec2(CARD_WIDTH, CARD_PICTURE));
    let hovered = response.hovered();
    let painter = ui.painter();
    painter.rect_filled(tile, theme::RADIUS, if hovered { theme::header() } else { theme::panel() });
    if let Some(picture) = picture {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        painter.image(picture, tile.shrink(4.0), uv, Color32::WHITE);
    } else if file.is_project() {
        icons::paint_logo(painter, Rect::from_center_size(tile.center(), Vec2::splat(44.0)), theme::text_faint());
    } else {
        icons::paint(painter, Rect::from_center_size(tile.center(), Vec2::splat(44.0)), Icon::Rig, theme::RIG_ICON);
    }
    let name_color = if hovered { theme::ink() } else { theme::text() };
    let name = one_line(ui, file.name(), FontId::proportional(12.5), name_color, CARD_WIDTH);
    ui.painter().galley(tile.left_bottom() + vec2(0.0, 8.0), name, name_color);
    let when = library::time_ago(file.opened, now);
    ui.painter().text(tile.left_bottom() + vec2(0.0, 26.0), egui::Align2::LEFT_TOP, when, FontId::proportional(11.0), theme::text_faint());
    chrome::tooltip(response, &file.path.to_string_lossy()).on_hover_cursor(CursorIcon::PointingHand)
}

fn backups_section(app: &mut KayeApp, ui: &mut Ui) -> Option<PathBuf> {
    heading(ui, "Backups");
    if app.home.backups.is_empty() {
        nothing_here(ui, "No backups yet");
        return None;
    }
    let mut picked = None;
    let now = library::now();
    ScrollArea::vertical().auto_shrink(false).id_salt("home_backups").show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, backup) in app.home.backups.iter().enumerate() {
            if backup_row(ui, backup, index, now).clicked() {
                picked = Some(backup.path.clone());
            }
        }
    });
    picked
}

fn backup_row(ui: &mut Ui, backup: &Backup, index: usize, now: u64) -> Response {
    const WHEN_FROM_RIGHT: f32 = 110.0;
    let width = ui.available_width() - 16.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    let hovered = response.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, theme::RADIUS, theme::header());
    } else if index % 2 == 1 {
        painter.rect_filled(rect, theme::RADIUS, theme::row_alt());
    }
    let color = if hovered { theme::ink() } else { theme::text() };
    icons::paint(painter, Rect::from_center_size(rect.left_center() + vec2(16.0, 0.0), Vec2::splat(15.0)), Icon::File, theme::text_dim());
    let room = (rect.width() - WHEN_FROM_RIGHT - 170.0).max(60.0);
    let name = one_line(ui, backup.name.clone(), FontId::proportional(12.5), color, room);
    let at = rect.left_center() + vec2(34.0, -name.size().y * 0.5);
    let painter = ui.painter();
    painter.galley(at, name, color);
    let dim = FontId::proportional(12.0);
    painter.text(rect.right_center() - vec2(WHEN_FROM_RIGHT, 0.0), egui::Align2::RIGHT_CENTER, library::time_ago(backup.made, now), dim.clone(), theme::text_dim());
    painter.text(rect.right_center() - vec2(12.0, 0.0), egui::Align2::RIGHT_CENTER, library::size_text(backup.size), dim, theme::text_faint());
    response.on_hover_cursor(CursorIcon::PointingHand)
}
