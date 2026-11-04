//! Everything around the viewport: top bar, tool rail, status bar and dialogs.

use egui::{Align, Button, Color32, Frame, Label, Layout, Margin, Panel, RichText, Sense, Stroke, Ui, Vec2, vec2};

use super::icons::{self, Icon};
use super::theme;
use super::{KayeApp, Leave, Selection, Tool};
use crate::camera::ViewPreset;
use crate::scene::{BoneKind, RigKind};
use crate::settings::Action;

fn bar_frame(fill: Color32, x: i8, y: i8) -> Frame {
    Frame::new().fill(fill).inner_margin(Margin::symmetric(x, y))
}

/// Square icon button. The tooltip names it, for anyone who doesn't know the icon.
pub fn icon_button(ui: &mut Ui, icon: Icon, enabled: bool, active: bool, tip: &str) -> egui::Response {
    let highlight = active.then_some((theme::field_active(), theme::ink()));
    sized_icon_button(ui, icon, enabled, highlight, tip, Vec2::splat(22.0), 15.0)
}

/// `highlight` is the fill and icon color for an active button.
fn sized_icon_button(ui: &mut Ui, icon: Icon, enabled: bool, highlight: Option<(Color32, Color32)>, tip: &str, size: Vec2, icon_size: f32) -> egui::Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let painter = ui.painter();
    let fill = if let Some((fill, _)) = highlight {
        fill
    } else if enabled && response.is_pointer_button_down_on() {
        theme::field_active()
    } else if enabled && response.hovered() {
        theme::field_hover()
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect, 2.0, fill);
    let color = if let Some((_, color)) = highlight {
        color
    } else if !enabled {
        theme::text_faint()
    } else if response.hovered() {
        theme::ink()
    } else {
        theme::text_dim()
    };
    icons::paint(painter, egui::Rect::from_center_size(rect.center(), Vec2::splat(icon_size)), icon, color);
    tooltip(response, tip)
}

/// Adds a tooltip, unless they're turned off in Preferences.
pub fn tooltip(response: egui::Response, text: &str) -> egui::Response {
    if theme::tooltips() { response.on_hover_text(text) } else { response }
}

pub fn top_bar(app: &mut KayeApp, ui: &mut Ui) {
    Panel::top("top_bar")
        .exact_size(28.0)
        .frame(bar_frame(theme::panel_deep(), 6, 0))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                logo(ui);
                ui.add_space(2.0);
                // MenuBar claims the whole row, so the rest of the bar lives inside it.
                egui::MenuBar::new().ui(ui, |ui| {
                    file_menu(app, ui);
                    edit_menu(app, ui);
                    add_menu(app, ui);
                    view_menu(app, ui);
                    help_menu(app, ui);
                    ui.add_space(8.0);
                    let (import_tip, undo_tip, redo_tip) = (app.tip(Action::Import), app.tip(Action::Undo), app.tip(Action::Redo));
                    if icon_button(ui, Icon::Import, true, false, &import_tip).clicked() {
                        app.open_import_dialog(ui.ctx());
                    }
                    if icon_button(ui, Icon::Undo, app.history.can_undo(), false, &undo_tip).clicked() {
                        app.undo();
                    }
                    if icon_button(ui, Icon::Redo, app.history.can_redo(), false, &redo_tip).clicked() {
                        app.redo();
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(4.0);
                        let prefs_tip = app.tip(Action::Preferences);
                        if icon_button(ui, Icon::Settings, true, app.prefs_open, &prefs_tip).clicked() {
                            app.prefs_open = !app.prefs_open;
                        }
                        ui.add_space(6.0);
                        if app.is_importing() || app.is_saving() {
                            ui.spinner();
                        }
                        let rigs: Vec<&crate::scene::Rig> = app.scene.rigs.iter().filter(|r| r.kind != RigKind::Camera).collect();
                        if let [rig] = rigs.as_slice() {
                            ui.label(RichText::new(rig.kind.label()).small().color(theme::text_faint()));
                            ui.label(RichText::new(&rig.name).color(theme::text_dim()));
                        }
                    });
                });
            });
        });
}

fn logo(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
    icons::paint_logo(ui.painter(), rect, theme::ink());
}

/// A menu entry with its shortcut right-aligned. Returns true when clicked.
fn item(ui: &mut Ui, text: &str, shortcut: &str, enabled: bool) -> bool {
    let clicked = ui
        .add_enabled(enabled, Button::new(text).shortcut_text(RichText::new(shortcut).color(theme::text_faint())))
        .clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn file_menu(app: &mut KayeApp, ui: &mut Ui) {
    ui.menu_button("File", |ui| {
        if item(ui, "New", &app.key_text(Action::NewProject), true) {
            app.request_leave(ui.ctx(), Leave::New);
        }
        if item(ui, "Open Project...", &app.key_text(Action::OpenProject), true) {
            app.request_leave(ui.ctx(), Leave::Open(None));
        }
        ui.separator();
        if item(ui, "Save", &app.key_text(Action::Save), true) {
            app.save_project(ui.ctx());
        }
        if item(ui, "Save As...", &app.key_text(Action::SaveAs), true) {
            app.save_project_as(ui.ctx());
        }
        ui.separator();
        if item(ui, "Import Rig...", &app.key_text(Action::Import), true) {
            app.open_import_dialog(ui.ctx());
        }
        ui.separator();
        item(ui, "Export Rig...", "", false);
        item(ui, "Export Animation...", "", false);
        item(ui, "Bake Animation", "", false);
        ui.separator();
        if item(ui, "Quit", "", true) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    });
}

fn edit_menu(app: &mut KayeApp, ui: &mut Ui) {
    ui.menu_button("Edit", |ui| {
        if item(ui, "Undo", &app.key_text(Action::Undo), app.history.can_undo()) {
            app.undo();
        }
        if item(ui, "Redo", &app.key_text(Action::Redo), app.history.can_redo()) {
            app.redo();
        }
        ui.separator();
        let has_bone = app.selected_bone().is_some();
        if item(ui, "Clear Position", &app.key_text(Action::ClearPosition), has_bone) {
            app.reset_selected(true, false);
        }
        if item(ui, "Clear Rotation", &app.key_text(Action::ClearRotation), has_bone) {
            app.reset_selected(false, true);
        }
        if item(ui, "Clear All Poses", "", !app.scene.bones.is_empty()) {
            app.reset_all_poses();
        }
        ui.separator();
        let can_key = !app.key_targets().is_empty();
        if item(ui, "Insert Keyframe", &app.key_text(Action::InsertKey), can_key) {
            app.insert_key();
        }
        if item(ui, "Delete Keyframe", &app.key_text(Action::DeleteKey), can_key) {
            app.delete_keys_at_frame();
        }
        let has_keys = !app.timeline.selected.is_empty();
        if item(ui, "Duplicate Selected Keys", &app.key_text(Action::DuplicateKeys), has_keys) {
            app.duplicate_selected_keys();
        }
        if item(ui, "Delete Selected Keys", &app.key_text(Action::DeleteSelectedKeys), has_keys) {
            app.delete_selected_keys();
        }
        ui.separator();
        if item(ui, "Preferences...", &app.key_text(Action::Preferences), true) {
            app.prefs_open = true;
        }
    });
}

fn add_menu(app: &mut KayeApp, ui: &mut Ui) {
    ui.menu_button("Add", |ui| {
        if item(ui, "Camera", &app.key_text(Action::AddCamera), true) {
            app.add_camera();
        }
        ui.separator();
        let bone = app.modifier_bone();
        let has_ik = bone.is_some_and(|b| app.has_ik(b));
        let has_unparent = bone.is_some_and(|b| app.has_unparent(b));
        if item(ui, "Inverse Kinematics", "", bone.is_some() && !has_ik) {
            if let Some(bone) = bone {
                app.add_ik(bone);
            }
        }
        if item(ui, "Unparent", "", bone.is_some() && !has_unparent) {
            if let Some(bone) = bone {
                app.add_unparent(bone);
            }
        }
    });
}

fn view_menu(app: &mut KayeApp, ui: &mut Ui) {
    ui.menu_button("View", |ui| {
        if item(ui, "Frame All", &app.key_text(Action::FrameAll), true) {
            app.frame_all();
        }
        if item(ui, "Frame Selected", &app.key_text(Action::FrameSelected), true) {
            app.frame_selection();
        }
        ui.separator();
        for (preset, action) in [
            (ViewPreset::Front, Action::ViewFront),
            (ViewPreset::Back, Action::ViewBack),
            (ViewPreset::Right, Action::ViewRight),
            (ViewPreset::Left, Action::ViewLeft),
            (ViewPreset::Top, Action::ViewTop),
            (ViewPreset::Bottom, Action::ViewBottom),
        ] {
            if item(ui, preset.label(), &app.key_text(action), true) {
                app.camera_view.through = false;
                app.camera.set_preset(preset);
            }
        }
        ui.separator();
        let has_camera = app.active_camera().is_some();
        if item(ui, "Camera View", &app.key_text(Action::CameraView), has_camera) {
            app.toggle_camera_view();
        }
        if item(ui, "Camera Preview", &app.key_text(Action::CameraPreview), has_camera) {
            app.toggle_camera_preview();
        }
        ui.separator();
        ui.checkbox(&mut app.camera.ortho, "Orthographic");
        let mut shaded = app.settings.viewport.shading == crate::settings::Shading::Shaded;
        if ui.checkbox(&mut shaded, "Shaded").changed() {
            app.toggle_shading();
        }
        ui.checkbox(&mut app.view.show_parts, "Parts");
        ui.checkbox(&mut app.view.show_bones, "Bones");
        ui.checkbox(&mut app.view.bones_on_top, "Bones on Top");
        ui.checkbox(&mut app.settings.viewport.part_colors, "Part Colors");
    });
}

fn help_menu(app: &mut KayeApp, ui: &mut Ui) {
    ui.menu_button("Help", |ui| {
        if item(ui, "Keyboard Shortcuts", "", true) {
            app.prefs_open = true;
            app.prefs.tab = super::preferences::PrefsTab::Keymap;
        }
        if item(ui, "About Kaye", "", true) {
            app.about_open = true;
        }
    });
}

pub fn tool_rail(app: &mut KayeApp, ui: &mut Ui) {
    Panel::left("tool_rail")
        .exact_size(36.0)
        .resizable(false)
        .frame(bar_frame(theme::panel(), 3, 4))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (tool, icon, action) in [
                (Tool::Select, Icon::Select, Action::ToolSelect),
                (Tool::Move, Icon::Move, Action::ToolMove),
                (Tool::Rotate, Icon::Rotate, Action::ToolRotate),
            ] {
                let tip = app.tip(action);
                if rail_button(ui, icon, app.tool == tool, true, &tip).clicked() {
                    app.tool = tool;
                }
            }
            rail_divider(ui);
            let has_bone = app.selected_bone().is_some();
            if rail_button(ui, Icon::Reset, false, has_bone, "Clear Pose").clicked() {
                app.reset_selected(true, true);
            }
            rail_divider(ui);
            let bone = app.modifier_bone();
            if rail_button(ui, Icon::Chain, false, bone.is_some(), "IK").clicked() {
                if let Some(bone) = bone {
                    app.add_ik(bone);
                }
            }
            if rail_button(ui, Icon::Unparent, false, bone.is_some(), "Unparent").clicked() {
                if let Some(bone) = bone {
                    app.add_unparent(bone);
                }
            }
            let _ = rail_button(ui, Icon::Mirror, false, false, "Mirror Pose");
            let camera_tip = app.tip(Action::AddCamera);
            if rail_button(ui, Icon::Camera, false, true, &camera_tip).clicked() {
                app.add_camera();
            }
            let can_key = !app.key_targets().is_empty();
            let key_tip = app.tip(Action::InsertKey);
            if rail_button(ui, Icon::Key, false, can_key, &key_tip).clicked() {
                app.insert_key();
            }
        });
}

fn rail_button(ui: &mut Ui, icon: Icon, active: bool, enabled: bool, tip: &str) -> egui::Response {
    // The current tool is the one thing on screen that gets a full accent fill.
    let highlight = active.then_some((theme::accent(), theme::on_accent()));
    sized_icon_button(ui, icon, enabled, highlight, tip, Vec2::splat(30.0), 18.0)
}

fn rail_divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(30.0, 9.0), Sense::hover());
    ui.painter().hline(rect.x_range().shrink(5.0), rect.center().y, Stroke::new(1.0, theme::field()));
}

pub fn status_bar(app: &mut KayeApp, ui: &mut Ui) {
    Panel::bottom("status_bar")
        .exact_size(20.0)
        .frame(bar_frame(theme::panel_deep(), 8, 0))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let s = &app.scene;
                let path = match app.selection {
                    Selection::Bone(b) if s.bones[b].kind == BoneKind::Camera => s.bones[b].name.clone(),
                    Selection::Bone(b) => format!("{}  /  {}", s.rigs[s.bones[b].rig].name, s.bones[b].name),
                    Selection::Part(p) => format!("{}  /  {}", s.rigs[s.parts[p].rig].name, s.parts[p].name),
                    Selection::Rig(r) => s.rigs[r].name.clone(),
                    Selection::None => String::new(),
                };
                ui.add(Label::new(RichText::new(path).small().color(theme::text_dim())).truncate());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(env!("CARGO_PKG_VERSION")).small().color(theme::text_faint()));
                    ui.add_space(10.0);
                    if !s.is_empty() {
                        let joints = s.bones.iter().filter(|b| b.kind.is_joint()).count();
                        ui.label(
                            RichText::new(format!("{joints} bones   {} parts", s.parts.len()))
                                .small()
                                .color(theme::text_faint()),
                        );
                    }
                });
            });
        });
}

pub fn windows(app: &mut KayeApp, ctx: &egui::Context) {
    let mut about_open = app.about_open;
    egui::Window::new("About")
        .open(&mut about_open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            ui.label(RichText::new(concat!("Kaye ", env!("CARGO_PKG_VERSION"))).strong());
            ui.label(RichText::new("Icons: Phosphor (MIT)").small().color(theme::text_dim()));
        });
    app.about_open = about_open;

    super::preferences::window(app, ctx);
    unsaved_dialog(app, ctx);
}

fn unsaved_dialog(app: &mut KayeApp, ctx: &egui::Context) {
    let Some(leave) = app.leaving.clone() else {
        return;
    };
    let name = app.project_name().unwrap_or_else(|| "this project".to_string());
    let modal = egui::Modal::new(egui::Id::unique("unsaved_work")).show(ctx, |ui| {
        ui.set_width(320.0);
        ui.label(RichText::new(format!("Save changes to {name}?")).strong());
        ui.add_space(10.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add(Button::new("Save").fill(theme::accent())).clicked() {
                app.leaving = None;
                app.save_then(ctx, leave.clone());
            }
            if ui.button("Don't Save").clicked() {
                app.leaving = None;
                app.leave_now(ctx, leave.clone());
            }
            if ui.button("Cancel").clicked() {
                app.leaving = None;
            }
        });
    });
    if modal.should_close() {
        app.leaving = None;
    }
}
