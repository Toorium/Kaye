//! The popup that asks for a Roblox Open Cloud key. It comes up at launch for as long as
//! there isn't one, since rigs stay grey boxes without it.

use egui::{Align, Button, CornerRadius, FontId, Frame, Id, Key, Layout, Margin, Modal, Rect, RichText, ScrollArea, Sense, TextEdit, Ui, Vec2, pos2, vec2};

use super::loading::PlayOnce;
use super::{KayeApp, theme};

/// Where Roblox makes API keys, on the API Keys tab.
const API_KEYS_PAGE: &str = "https://create.roblox.com/dashboard/credentials?activeTab=ApiKeysTab";

const WHY: &str = "Your rigs use meshes and textures that are stored on Roblox, and Kaye needs an Open Cloud API key to download them. \
    Without one, rigs still open, but their parts show up as plain grey boxes. \
    The key stays on this computer, and it only takes a minute to make.";

const WIDTH: f32 = 500.0;
const TEXT_SIZE: f32 = 12.5;
/// The tallest the animation gets, and what it shrinks to in a small window.
const ART_HEIGHT: (f32, f32) = (96.0, 220.0);
/// How much of the window everything but the animation can take.
const OTHER_CONTENT: f32 = 372.0;
/// The title, key field and buttons, which always have to fit.
const FIXED_CONTENT: f32 = 130.0;

pub struct Connect {
    open: bool,
    /// What's typed so far, which only becomes the key once it's saved.
    key: String,
    art: PlayOnce,
}

impl Connect {
    pub fn new(key: &str) -> Self {
        Self { open: key.trim().is_empty(), key: key.to_string(), art: PlayOnce::connect() }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Brings it back from Preferences, with the animation from the start.
    pub fn ask(&mut self, key: &str) {
        *self = Self { open: true, ..Self::new(key) };
    }
}

pub fn window(app: &mut KayeApp, ctx: &egui::Context) {
    if !app.connect.open {
        return;
    }
    let art = app.connect.art.frame(ctx);
    let window_height = ctx.content_rect().height();
    let art_height = (window_height - OTHER_CONTENT).clamp(ART_HEIGHT.0, ART_HEIGHT.1);
    let text_height = (window_height - art_height - FIXED_CONTENT).max(40.0);
    let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
    let mut save = false;
    // Clicking outside is ignored, since coming back from the browser can be that click.
    Modal::new(Id::unique("connect_to_roblox")).frame(Frame::window(&ctx.global_style()).inner_margin(Margin::ZERO)).show(ctx, |ui| {
        ui.set_width(WIDTH);
        let (band, _) = ui.allocate_exact_size(vec2(WIDTH, art_height), Sense::hover());
        let top = theme::WINDOW_RADIUS;
        ui.painter().rect_filled(band, CornerRadius { nw: top, ne: top, sw: 0, se: 0 }, theme::ART_BACKDROP);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        ui.painter().image(art, Rect::from_center_size(band.center(), vec2(art_height * 2.0, art_height)), uv, egui::Color32::WHITE);
        Frame::new().inner_margin(Margin { left: 20, right: 20, top: 14, bottom: 14 }).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.label(RichText::new("Connect Kaye to Roblox").font(theme::strong_font(15.0)).color(theme::ink()));
            ui.add_space(2.0);
            // A small window or a large UI scale scrolls this, and keeps the buttons in reach.
            ScrollArea::vertical().max_height(text_height).auto_shrink([false, true]).show(ui, |ui| {
                ui.label(RichText::new(WHY).size(TEXT_SIZE).color(theme::text()));
                ui.add_space(8.0);
                steps(ui);
            });
            ui.add_space(6.0);
            theme::sunken(ui);
            let field = ui.add(TextEdit::singleline(&mut app.connect.key).password(true).hint_text("Paste your key here").desired_width(f32::INFINITY));
            let has_key = !app.connect.key.trim().is_empty();
            save = has_key && field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                theme::raised(ui);
                let button = Button::new(RichText::new("Save Key").color(theme::on_accent())).fill(theme::accent()).min_size(vec2(84.0, 24.0));
                save |= ui.add_enabled(has_key, button).clicked();
                close |= ui.add(Button::new("Later").min_size(vec2(64.0, 24.0))).clicked();
            });
        });
    });
    if save {
        app.settings.system.roblox_api_key = app.connect.key.trim().to_string();
    }
    if save || close {
        app.connect.close();
    }
}

/// How to make a key. The names in bold are what Roblox's page calls them.
fn steps(ui: &mut Ui) {
    step(ui, 1, |ui| {
        ui.hyperlink_to("Open the API Keys page", API_KEYS_PAGE);
        text(ui, " and sign in. Use your own account, because a group's key won't work.");
    });
    step(ui, 2, |ui| {
        text(ui, "Click ");
        name(ui, "Create API Key");
        text(ui, " and name it Kaye.");
    });
    step(ui, 3, |ui| {
        text(ui, "Under ");
        name(ui, "Access Permissions");
        text(ui, ", pick ");
        name(ui, "legacy-asset");
        text(ui, ", then pick ");
        name(ui, "manage");
        text(ui, " under ");
        name(ui, "Select Operations");
        text(ui, ".");
    });
    step(ui, 4, |ui| {
        text(ui, "Click ");
        name(ui, "Save & Generate key");
        text(ui, ", then copy the key.");
    });
    step(ui, 5, |ui| text(ui, "Paste it below."));
}

fn step(ui: &mut Ui, number: u32, content: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(16.0, 16.0), Sense::hover());
        ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, format!("{number}."), FontId::proportional(TEXT_SIZE), theme::text_dim());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.spacing_mut().interact_size.y = 16.0;
            ui.style_mut().override_font_id = Some(FontId::proportional(TEXT_SIZE));
            content(ui);
        });
    });
}

fn text(ui: &mut Ui, words: &str) {
    ui.label(RichText::new(words).color(theme::text()));
}

fn name(ui: &mut Ui, words: &str) {
    ui.label(RichText::new(words).font(theme::strong_font(TEXT_SIZE)).color(theme::ink()));
}
