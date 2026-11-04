//! The small window that downloads and installs an update, see `crate::update`. It's Kaye's
//! own exe, started with `--update` by the popup's Update button as Kaye closes.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::{Align, Button, CentralPanel, Frame, Layout, Margin, ProgressBar, RichText, vec2};

use super::theme;
use crate::settings::Settings;
use crate::update::{self, Failure, Progress};

pub const SIZE: [f32; 2] = [440.0, 168.0];
const TEXT_SIZE: f32 = 12.5;

#[derive(Clone, Debug, PartialEq)]
enum Stage {
    Checking,
    Downloading(String),
    /// Downloaded, and the Kaye that started this hasn't closed yet.
    Waiting(String),
    Installing(String),
    /// Kaye is starting, so this window goes.
    Finished,
    Failed {
        text: String,
        /// Some files were already swapped in, so this version isn't the one left.
        changed: bool,
    },
}

pub struct Updater {
    exe: Option<PathBuf>,
    parent: Option<u32>,
    stage: Arc<Mutex<Stage>>,
    progress: Arc<Progress>,
    #[cfg(feature = "devtools")]
    script: Option<crate::devscript::DevScript>,
}

impl Updater {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: &Settings, parent: Option<u32>) -> Self {
        theme::apply(&cc.egui_ctx);
        theme::apply_settings(&cc.egui_ctx, &settings.theme, false);
        let mut updater = Self {
            exe: std::env::current_exe().ok(),
            parent,
            stage: Arc::new(Mutex::new(Stage::Checking)),
            progress: Arc::new(Progress::default()),
            #[cfg(feature = "devtools")]
            script: crate::devscript::DevScript::from_var("KAYE_DEV_UPDATER_SCRIPT"),
        };
        updater.start(&cc.egui_ctx);
        updater
    }

    fn set(&self, stage: Stage) {
        *self.stage.lock().unwrap() = stage;
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.set(Stage::Checking);
        self.progress = Arc::new(Progress::default());
        let (stage, progress, ctx, parent) = (self.stage.clone(), self.progress.clone(), ctx.clone(), self.parent);
        let Some(exe) = self.exe.clone() else {
            self.set(Stage::Failed { text: "Kaye couldn't find its own folder.".into(), changed: false });
            return;
        };
        std::thread::spawn(move || {
            let set = |next: Stage| {
                *stage.lock().unwrap() = next;
                ctx.request_repaint();
            };
            let result = run(&exe, parent, &progress, &set);
            // A stopped or failed download is of no more use. After a swap the old exe is in
            // there and still running, so it's left for the new Kaye to delete.
            if let (Err((_, false)), Some(folder)) = (&result, exe.parent()) {
                update::clear_work_folder(folder);
            }
            let next = match result {
                Ok(()) => Stage::Finished,
                // Stopping brings back the Kaye that was there.
                Err((Failure::Cancelled, _)) => match update::launch(&exe, false) {
                    Ok(()) => Stage::Finished,
                    Err(e) => Stage::Failed { text: format!("Kaye didn't start: {e}"), changed: false },
                },
                Err((Failure::Failed(text), changed)) => Stage::Failed { text, changed },
            };
            set(next);
        });
    }

    fn cancel(&self) {
        self.progress.cancel.store(true, Ordering::SeqCst);
    }

    fn cancelling(&self) -> bool {
        self.progress.cancel.load(Ordering::SeqCst)
    }

    fn open_kaye(&self, ctx: &egui::Context) {
        match self.exe.as_deref().map(|exe| update::launch(exe, false)) {
            Some(Ok(())) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Some(Err(e)) => self.set(Stage::Failed { text: format!("Kaye didn't start: {e}"), changed: false }),
            None => {}
        }
    }
}

/// Everything the update does, on the worker thread. A failure says whether any file had
/// been swapped in yet.
fn run(exe: &Path, parent: Option<u32>, progress: &Progress, set: &dyn Fn(Stage)) -> Result<(), (Failure, bool)> {
    let early = |failure: Failure| (failure, false);
    let folder = exe.parent().ok_or_else(|| early(Failure::Failed("Kaye couldn't find its own folder.".into())))?;
    update::clear_work_folder(folder);
    let release = update::fetch_latest(&update::latest_url()).map_err(|e| early(e.into()))?;
    if !update::is_newer(&release.version, update::CURRENT) {
        // Nothing newer after all, so the same Kaye comes back.
        return Err(early(Failure::Cancelled));
    }
    let plan = update::plan(folder, &release).map_err(|e| early(e.into()))?;
    set(Stage::Downloading(release.version.clone()));
    update::download(&plan, folder, progress).map_err(early)?;
    if let Some(pid) = parent {
        if !update::wait_for_exit(pid, Duration::from_secs(1)) {
            set(Stage::Waiting(release.version.clone()));
            // Windows lets an open exe be renamed, so a Kaye that won't close only delays this.
            update::wait_for_exit(pid, Duration::from_secs(60));
        }
    }
    if progress.cancel.load(Ordering::SeqCst) {
        return Err(early(Failure::Cancelled));
    }
    set(Stage::Installing(release.version.clone()));
    update::apply(&plan, folder).map_err(|e| early(e.into()))?;
    update::launch(exe, true).map_err(|e| (Failure::Failed(format!("Kaye was updated to {}, but didn't start: {e}", release.version)), true))
}

impl eframe::App for Updater {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let stage = self.stage.lock().unwrap().clone();
        if stage == Stage::Finished {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // The window's X stops the update the way Cancel does, and waits for it to stop, so
        // files are never left half swapped.
        let busy = !matches!(stage, Stage::Finished | Stage::Failed { .. });
        if busy && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.cancel();
        }
        if matches!(stage, Stage::Checking | Stage::Downloading(_)) {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        let mut retry = false;
        let mut open = false;
        let frame = Frame::new().fill(theme::panel()).inner_margin(Margin { left: 20, right: 20, top: 16, bottom: 14 });
        CentralPanel::default().frame(frame).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let (title, detail) = match &stage {
                Stage::Failed { .. } => ("Couldn't update Kaye".to_string(), String::new()),
                Stage::Checking => ("Updating Kaye".to_string(), "Looking for the update".to_string()),
                Stage::Downloading(v) => (format!("Updating Kaye to {v}"), "Downloading".to_string()),
                Stage::Waiting(v) => (format!("Updating Kaye to {v}"), "Waiting for Kaye to close".to_string()),
                Stage::Installing(v) => (format!("Updating Kaye to {v}"), "Installing".to_string()),
                Stage::Finished => ("Updating Kaye".to_string(), "Starting Kaye".to_string()),
            };
            let detail = if self.cancelling() && busy { "Stopping".to_string() } else { detail };
            ui.label(RichText::new(title).font(theme::strong_font(15.0)).color(theme::ink()));
            ui.add_space(2.0);
            if let Stage::Failed { text, changed } = &stage {
                ui.label(RichText::new(text).size(TEXT_SIZE).color(theme::text()));
                if !changed {
                    ui.label(RichText::new("Nothing was changed, so this version still works.").size(TEXT_SIZE).color(theme::text_dim()));
                }
            } else {
                let (done, total) = (self.progress.done.load(Ordering::SeqCst), self.progress.total.load(Ordering::SeqCst));
                let fraction = match &stage {
                    Stage::Downloading(_) if total > 0 => done as f32 / total as f32,
                    Stage::Waiting(_) | Stage::Installing(_) | Stage::Finished => 1.0,
                    _ => 0.0,
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(detail).size(TEXT_SIZE).color(theme::text()));
                    if matches!(stage, Stage::Downloading(_)) && total > 0 {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(format!("{} of {}", megabytes(done), megabytes(total))).size(TEXT_SIZE).color(theme::text_dim()));
                        });
                    }
                });
                let bar = ProgressBar::new(fraction).desired_height(6.0).fill(theme::accent()).corner_radius(theme::RADIUS);
                ui.add(bar.animate(matches!(stage, Stage::Checking)));
            }
            ui.with_layout(Layout::bottom_up(Align::Max), |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    theme::raised(ui);
                    if matches!(stage, Stage::Failed { .. }) {
                        let again = Button::new(RichText::new("Try Again").color(theme::on_accent())).fill(theme::accent()).min_size(vec2(84.0, 24.0));
                        retry = ui.add(again).clicked();
                        open = ui.add(Button::new("Open Kaye").min_size(vec2(84.0, 24.0))).clicked();
                    } else {
                        // Once files start moving, stopping halfway isn't offered.
                        let can_cancel = matches!(stage, Stage::Checking | Stage::Downloading(_) | Stage::Waiting(_)) && !self.cancelling();
                        if ui.add_enabled(can_cancel, Button::new("Cancel").min_size(vec2(72.0, 24.0))).clicked() {
                            self.cancel();
                        }
                    }
                });
            });
        });
        if retry {
            self.start(&ctx);
        }
        if open {
            self.open_kaye(&ctx);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::panel().to_normalized_gamma_f32()
    }

    #[cfg(feature = "devtools")]
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(script) = &mut self.script {
            script.step(ctx, raw_input);
        }
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}
