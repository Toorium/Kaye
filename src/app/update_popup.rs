//! The "New update" popup, which comes up when GitHub has a newer Kaye than this one. Update
//! closes Kaye the way the window's X does, asking about unsaved work first, then the updater
//! takes over. See `crate::update`.

use std::sync::mpsc::{Receiver, TryRecvError, channel};

use egui::{Align, Button, Frame, Id, Key, Layout, Margin, Modal, RichText, ScrollArea, vec2};

use super::{KayeApp, Leave, theme};
use crate::update::{self, Release};

const WIDTH: f32 = 380.0;
const TEXT_SIZE: f32 = 12.5;
/// Updating means taking the newer version under this license, so the popup shows it and the
/// button says so.
pub const LICENSE: &str = include_str!("../../LICENSE");

#[derive(Default)]
pub struct UpdatePopup {
    /// The check runs on another thread from launch.
    check: Option<Receiver<Option<Release>>>,
    found: Option<Release>,
    /// Later was clicked. The status bar still offers the update until Kaye closes.
    dismissed: bool,
    showing: bool,
}

impl UpdatePopup {
    pub fn start(ctx: &egui::Context) -> Self {
        if !update::checks_on_launch() {
            return Self::default();
        }
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(update::check());
            repaint.request_repaint();
        });
        Self { check: Some(rx), ..Self::default() }
    }

    fn poll(&mut self) {
        let Some(check) = &self.check else { return };
        match check.try_recv() {
            Ok(found) => {
                self.found = found;
                self.check = None;
            }
            Err(TryRecvError::Disconnected) => self.check = None,
            Err(TryRecvError::Empty) => {}
        }
    }

    pub fn is_showing(&self) -> bool {
        self.showing
    }

    /// The newer version, once Later has put the popup away.
    pub fn waiting(&self) -> Option<&str> {
        self.found.as_ref().filter(|_| self.dismissed).map(|r| r.version.as_str())
    }

    pub fn reopen(&mut self) {
        self.dismissed = false;
    }
}

pub fn window(app: &mut KayeApp, ctx: &egui::Context) {
    app.update.poll();
    // It waits its turn behind the Roblox key popup and any question about unsaved work.
    let blocked = app.connect.is_open() || app.leaving.is_some() || app.loader.is_covering();
    let was_showing = app.update.showing;
    app.update.showing = app.update.found.is_some() && !app.update.dismissed && !blocked;
    let Some(release) = app.update.found.as_ref().filter(|_| app.update.showing) else {
        return;
    };
    let (version, notes) = (release.version.clone(), release.notes.trim().to_string());
    // An Escape that just closed the key popup isn't meant for this one.
    let mut later = was_showing && ctx.input(|i| i.key_pressed(Key::Escape));
    let mut install = false;
    // Clicking outside is ignored, so a stray click can't decide for someone.
    let frame = Frame::window(&ctx.global_style()).inner_margin(Margin { left: 20, right: 20, top: 16, bottom: 14 });
    Modal::new(Id::unique("new_update")).frame(frame).show(ctx, |ui| {
        ui.set_width(WIDTH);
        ui.spacing_mut().item_spacing.y = 5.0;
        ui.label(RichText::new("New update").font(theme::strong_font(15.0)).color(theme::ink()));
        ui.add_space(2.0);
        ui.label(RichText::new(format!("Kaye {version} is out. You have {}.", update::CURRENT)).size(TEXT_SIZE).color(theme::text()));
        if !notes.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new("What's new").font(theme::strong_font(TEXT_SIZE)).color(theme::ink()));
            ScrollArea::vertical().max_height(140.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.label(RichText::new(&notes).size(TEXT_SIZE).color(theme::text()));
            });
        }
        ui.add_space(4.0);
        ui.label(RichText::new("License").font(theme::strong_font(TEXT_SIZE)).color(theme::ink()));
        Frame::new().fill(theme::well()).inner_margin(Margin::same(6)).corner_radius(3).show(ui, |ui| {
            ScrollArea::vertical().id_salt("update_license").max_height(110.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.label(RichText::new(LICENSE.trim()).size(11.5).color(theme::text()));
            });
        });
        ui.add_space(4.0);
        let how = "Kaye will close, download the update and open again. Your projects, settings and backups aren't touched.";
        ui.label(RichText::new(how).size(TEXT_SIZE).color(theme::text_dim()));
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            theme::raised(ui);
            let button = Button::new(RichText::new("Accept & Update").color(theme::on_accent())).fill(theme::accent()).min_size(vec2(120.0, 24.0));
            install = ui.add(button).clicked();
            later |= ui.add(Button::new("Later").min_size(vec2(64.0, 24.0))).clicked();
        });
    });
    if install || later {
        app.update.dismissed = true;
        app.update.showing = false;
    }
    if install {
        app.request_leave(ctx, Leave::Update);
    }
}
