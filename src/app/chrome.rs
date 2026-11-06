//! Everything around the viewport: menu bar, shelf, tool column and status bar, plus
//! dialogs and the controls every editor shares. It has Maya's look, kept short enough
//! that a new user can read all of it.

use egui::{Align, Align2, Button, Color32, CornerRadius, FontId, Frame, Label, Layout, Margin, Panel, Pos2, RichText, Sense, Shape, Stroke, Ui, Vec2, vec2};

use super::icons::{self, Icon};
use super::theme;
use super::{KayeApp, Leave, Selection, Tool};
use crate::camera::ViewPreset;
use crate::export::{self, Pack};
use crate::modifiers;
use crate::scene::{BoneId, BoneKind, PartShape, RigKind};
use crate::settings::{Action, Shading, ShelfTab};

/// The strip along the top of an editor, holding its tabs or menus.
pub const STRIP_HEIGHT: f32 = 24.0;
const MENU_BAR_HEIGHT: f32 = 24.0;
const SHELF_HEIGHT: f32 = 34.0;
const SHELF_ICON: f32 = 20.0;
const TOOL_COLUMN_WIDTH: f32 = 38.0;
const STATUS_BAR_HEIGHT: f32 = 22.0;

pub fn bar_frame(fill: Color32, x: i8, y: i8) -> Frame {
    Frame::new().fill(fill).inner_margin(Margin::symmetric(x, y))
}

/// The frame of an editor's strip.
pub fn strip_frame() -> Frame {
    bar_frame(theme::panel(), 4, 0)
}

/// A flat text tab. The active one is a lighter block, as in Maya's panel headers.
pub fn editor_tab(ui: &mut Ui, label: &str, active: bool) -> egui::Response {
    let color = if active { theme::ink() } else { theme::text_dim() };
    let galley = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.0), color);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 16.0, 20.0), Sense::click());
    if active {
        ui.painter().rect_filled(rect, 0.0, theme::section());
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::header());
    }
    ui.painter().galley(rect.center() - galley.size() * 0.5, galley, color);
    response
}

/// The icon and icon color for anything stored as a bone.
pub fn bone_icon(kind: BoneKind) -> (Icon, Color32) {
    match kind {
        BoneKind::Camera => (Icon::Camera, theme::camera_icon()),
        BoneKind::IkTarget | BoneKind::IkPole | BoneKind::LookTarget => (Icon::Target, theme::accent_soft()),
        BoneKind::Shape => (Icon::Part, theme::part_icon()),
        _ => (Icon::Bone, theme::bone_icon()),
    }
}

/// A small filled triangle, the marker Maya puts on collapsible frames.
pub fn paint_triangle(painter: &egui::Painter, center: Pos2, open: bool, color: Color32) {
    let points = if open {
        vec![center + vec2(-4.0, -2.5), center + vec2(4.0, -2.5), center + vec2(0.0, 3.0)]
    } else {
        vec![center + vec2(-2.5, -4.0), center + vec2(3.0, 0.0), center + vec2(-2.5, 4.0)]
    };
    painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
}

/// Square icon button. The tooltip names it, for anyone who doesn't know the icon.
pub fn icon_button(ui: &mut Ui, icon: Icon, enabled: bool, active: bool, tip: &str) -> egui::Response {
    let highlight = active.then_some((theme::toggled(), theme::ink()));
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
    painter.rect_filled(rect, theme::RADIUS, fill);
    let color = if let Some((_, color)) = highlight {
        color
    } else if !enabled {
        theme::text_faint().gamma_multiply(0.45)
    } else if response.hovered() {
        theme::ink()
    } else {
        theme::text()
    };
    icons::paint(painter, egui::Rect::from_center_size(rect.center(), Vec2::splat(icon_size)), icon, color);
    tooltip(response, tip)
}

/// An icon button whose on state has its own color, as Auto Key's red does.
pub fn colored_icon_button(ui: &mut Ui, icon: Icon, on: Option<Color32>, tip: &str) -> egui::Response {
    let highlight = on.map(|fill| (fill, theme::ink()));
    sized_icon_button(ui, icon, true, highlight, tip, Vec2::splat(22.0), 15.0)
}

/// The corners of button `index` in a joined row of `count`.
pub fn joined_corners(index: usize, count: usize) -> CornerRadius {
    let r = theme::SECTION_RADIUS;
    CornerRadius {
        nw: if index == 0 { r } else { 0 },
        sw: if index == 0 { r } else { 0 },
        ne: if index + 1 == count { r } else { 0 },
        se: if index + 1 == count { r } else { 0 },
    }
}

/// Rounds the next widgets in this `ui` for their place in a joined row or column.
pub fn round_widgets(ui: &mut Ui, corners: CornerRadius) {
    let w = &mut ui.visuals_mut().widgets;
    w.inactive.corner_radius = corners;
    w.hovered.corner_radius = corners;
    w.active.corner_radius = corners;
}

/// Adds a tooltip, unless they're turned off in Preferences.
pub fn tooltip(response: egui::Response, text: &str) -> egui::Response {
    if theme::tooltips() { response.on_hover_text(text) } else { response }
}

/// A thin etched line between groups of controls.
pub fn divider(ui: &mut Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, height), Sense::hover());
    let x = ui.painter().round_to_pixel_center(rect.center().x);
    ui.painter().vline(x, rect.y_range(), Stroke::new(1.0, theme::window()));
    ui.painter().vline(x + 1.0, rect.y_range(), Stroke::new(1.0, theme::field()));
}

/// Menu rows light up in the accent, as Maya's do in blue.
pub fn menu_rows(ui: &mut Ui) {
    ui.set_min_width(190.0);
    let visuals = ui.visuals_mut();
    visuals.override_text_color = None;
    let w = &mut visuals.widgets;
    for state in [&mut w.hovered, &mut w.active, &mut w.open] {
        state.weak_bg_fill = theme::active_row();
        state.bg_fill = theme::active_row();
    }
}

/// A menu title that opens rows styled by `menu_rows`.
pub fn menu(ui: &mut Ui, title: &str, content: impl FnOnce(&mut Ui)) {
    ui.menu_button(title, |ui| {
        menu_rows(ui);
        content(ui);
    });
}

/// A menu entry with its shortcut right-aligned. Returns true when clicked.
pub fn item(ui: &mut Ui, text: &str, shortcut: &str, enabled: bool) -> bool {
    let clicked = ui
        .add_enabled(enabled, Button::new(text).shortcut_text(RichText::new(shortcut).color(theme::text_faint())))
        .clicked();
    if clicked {
        ui.close();
    }
    clicked
}

pub fn menu_bar(app: &mut KayeApp, ui: &mut Ui) {
    Panel::top("menu_bar")
        .exact_size(MENU_BAR_HEIGHT)
        .frame(bar_frame(theme::panel(), 6, 0))
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                logo(ui);
                ui.add_space(2.0);
                // MenuBar claims the whole row, so the rest of the bar lives inside it.
                egui::MenuBar::new().ui(ui, |ui| {
                    file_menu(app, ui);
                    // The rest work on what's in the viewport, which the home page hides.
                    if !app.home.open {
                        edit_menu(app, ui);
                        create_menu(app, ui);
                        animation_menu(app, ui);
                        plugins_menu(app, ui);
                    }
                    help_menu(app, ui);
                    if app.home.open {
                        return;
                    }
                    super::rig_mode::mode_tabs(app, ui);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(4.0);
                        let title = app.project_name().unwrap_or_else(|| "Untitled".to_string());
                        ui.label(RichText::new(title).color(theme::text_dim()));
                        if app.is_importing() || app.is_saving() {
                            ui.spinner();
                        }
                    });
                });
            });
        });
}

fn logo(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
    icons::paint_logo(ui.painter(), rect, theme::ink());
}

fn file_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "File", |ui| {
        let editing = !app.home.open;
        if editing {
            if item(ui, "Home", "", true) {
                app.go_home();
            }
            ui.separator();
        }
        if item(ui, "New", &app.key_text(Action::NewProject), true) {
            app.request_leave(ui.ctx(), Leave::New);
        }
        if item(ui, "Open Project...", &app.key_text(Action::OpenProject), true) {
            app.request_leave(ui.ctx(), Leave::Open(None));
        }
        ui.separator();
        if item(ui, "Save", &app.key_text(Action::Save), editing) {
            app.save_project(ui.ctx());
        }
        if item(ui, "Save As...", &app.key_text(Action::SaveAs), editing) {
            app.save_project_as(ui.ctx());
        }
        ui.separator();
        if item(ui, "Import Rig...", &app.key_text(Action::Import), true) {
            app.open_import_dialog(ui.ctx());
        }
        ui.separator();
        ui.add_enabled_ui(editing && app.export_rig().is_some(), |ui| {
            menu(ui, "Export", |ui| {
                menu(ui, &format!(".{}", export::EXTENSION), |ui| {
                    if item(ui, "Rig", "", true) {
                        app.export_animation(Pack::Rig);
                    }
                    if item(ui, "Keyframe", "", true) {
                        app.export_animation(Pack::Keyframe);
                    }
                    if item(ui, "Current Keyframe", "", true) {
                        app.export_animation(Pack::CurrentKeyframe);
                    }
                });
            });
        });
        if item(ui, "Bake Animation", "", editing && !app.scene.modifiers.is_empty()) {
            app.bake_all_modifiers();
        }
        ui.separator();
        if item(ui, "Quit", "", true) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    });
}

fn edit_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "Edit", |ui| {
        if item(ui, "Undo", &app.key_text(Action::Undo), app.history.can_undo()) {
            app.undo();
        }
        if item(ui, "Redo", &app.key_text(Action::Redo), app.history.can_redo()) {
            app.redo();
        }
        ui.separator();
        if item(ui, "Select All Bones", "", app.focused_rig().is_some()) {
            app.select_all_bones();
        }
        if item(ui, "Deselect All", &app.key_text(Action::Deselect), app.selection != Selection::None) {
            app.select(Selection::None);
        }
        ui.separator();
        let can_copy = !app.key_targets().is_empty();
        if item(ui, "Copy Pose", &app.key_text(Action::CopyPose), can_copy) {
            app.copy_pose(ui.ctx());
        }
        if item(ui, "Paste Pose", &app.key_text(Action::PastePose), !app.pose_clipboard.is_empty()) {
            app.paste_pose();
        }
        let can_paste = if app.pasting_keys { !app.key_clipboard.is_empty() } else { !app.pose_clipboard.is_empty() };
        if item(ui, "Paste Mirrored", &app.key_text(Action::PasteMirrored), can_paste) {
            app.paste_mirrored();
        }
        if item(ui, "Mirror Pose", &app.key_text(Action::MirrorPose), can_copy) {
            app.mirror_pose();
        }
        ui.separator();
        if item(ui, "Delete", &app.key_text(Action::Delete), app.selection != Selection::None) {
            app.delete_selection();
        }
        ui.separator();
        let has_bone = app.selected_bone().is_some() || app.movable_rig().is_some();
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
        if item(ui, "Preferences...", &app.key_text(Action::Preferences), true) {
            app.prefs_open = true;
        }
    });
}

fn create_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "Create", |ui| {
        if item(ui, "Camera", &app.key_text(Action::AddCamera), true) {
            app.add_camera();
        }
        ui.separator();
        shape_items(app, ui);
        ui.separator();
        if item(ui, "Reference...", "", true) {
            super::references::pick_files(app, ui.ctx());
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

fn shape_items(app: &mut KayeApp, ui: &mut Ui) {
    for shape in PartShape::ALL {
        if item(ui, shape.label(), "", true) {
            app.add_shape(shape);
        }
    }
    ui.separator();
    if item(ui, "Baseplate", "", true) {
        app.add_baseplate();
    }
}

fn animation_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "Animation", |ui| {
        let can_key = !app.key_targets().is_empty();
        if item(ui, "Set Key", &app.key_text(Action::InsertKey), can_key) {
            app.insert_key();
        }
        if item(ui, "Delete Key", &app.key_text(Action::DeleteKey), can_key) {
            app.delete_keys_at_frame();
        }
        ui.checkbox(&mut app.settings.animation.auto_key, "Auto Key");
        ui.separator();
        let has_keys = !app.timeline.selected.is_empty();
        if item(ui, "Duplicate Selected Keys", &app.key_text(Action::DuplicateKeys), has_keys) {
            app.duplicate_selected_keys();
        }
        if item(ui, "Delete Selected Keys", &app.key_text(Action::DeleteSelectedKeys), has_keys) {
            app.delete_selected_keys();
        }
        if item(ui, "Copy Selected Keys", &app.key_text(Action::CopyPose), has_keys) {
            app.copy_keys(ui.ctx());
        }
        if item(ui, "Paste Keys", &app.key_text(Action::PastePose), !app.key_clipboard.is_empty()) {
            app.paste_keys();
        }
        ui.separator();
        if item(ui, if app.playing { "Pause" } else { "Play" }, &app.key_text(Action::PlayPause), true) {
            app.toggle_play();
        }
        ui.checkbox(&mut app.animation.looping, "Loop");
    });
}

fn plugins_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "Plugins", |ui| {
        let commands = app.plugin_commands();
        for (index, name) in commands.iter().enumerate() {
            if item(ui, name, "", true) {
                app.run_plugin(index);
            }
        }
        if !commands.is_empty() {
            ui.separator();
        }
        if item(ui, "Reload Plugins", "", true) {
            app.reload_plugins();
        }
        if item(ui, "Open Plugins Folder", "", crate::settings::plugins_folder().is_some()) {
            if let Some(folder) = crate::settings::plugins_folder() {
                let _ = std::fs::create_dir_all(&folder);
                let _ = std::process::Command::new("explorer").arg(folder).spawn();
            }
        }
    });
}

fn help_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "Help", |ui| {
        if item(ui, "Keyboard Shortcuts", "", true) {
            app.prefs_open = true;
            app.prefs.tab = super::preferences::PrefsTab::Keymap;
        }
        if item(ui, "About Kaye", "", true) {
            app.about_open = true;
        }
    });
}

/// In the 3D view's own menu row, as in Maya.
pub fn view_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "View", |ui| {
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
        if item(ui, "Fullscreen", &app.key_text(Action::Fullscreen), true) {
            app.toggle_fullscreen(ui.ctx());
        }
    });
}

pub fn shading_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "Shading", |ui| {
        let current = app.settings.viewport.shading;
        for (shading, label) in [(Shading::Studio, "Studio"), (Shading::Shaded, "Shaded")] {
            let shortcut = app.key_text(Action::ToggleShading);
            let button = Button::new(label).selected(current == shading).shortcut_text(RichText::new(shortcut).color(theme::text_faint()));
            if ui.add(button).clicked() {
                app.settings.viewport.shading = shading;
                ui.close();
            }
        }
        ui.separator();
        ui.checkbox(&mut app.settings.viewport.part_colors, "Part Colors");
        ui.checkbox(&mut app.settings.lighting.shadows, "Shadows");
    });
}

pub fn show_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "Show", |ui| {
        ui.checkbox(&mut app.view.show_parts, "Parts");
        ui.checkbox(&mut app.view.show_bones, "Bones");
        ui.checkbox(&mut app.view.bones_on_top, "Bones on Top");
        ui.checkbox(&mut app.settings.viewport.show_grid, "Grid");
    });
}

/// Tabs of icons, one for each kind of work, as Maya's shelf has, named by their
/// tooltips. Undo, Redo and Preferences stay on the right whichever tab is open. Copy
/// and paste aren't here, since everyone uses their keys, saving and opening are in the
/// File menu, and playback lives with the timeline.
pub fn shelf(app: &mut KayeApp, ui: &mut Ui) {
    Panel::top("shelf").exact_size(SHELF_HEIGHT).frame(bar_frame(theme::header(), 6, 2)).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            if app.mode == super::rig_mode::Mode::Rigging {
                shelf_tab(ui, "Rigging", true);
                shelf_gap(ui);
                super::rig_mode::tools(app, ui);
            } else {
                for tab in ShelfTab::ALL {
                    if shelf_tab(ui, tab.label(), app.settings.interface.shelf_tab == tab).clicked() {
                        app.settings.interface.shelf_tab = tab;
                    }
                }
                shelf_gap(ui);
                match app.settings.interface.shelf_tab {
                    ShelfTab::Animation => animation_tools(app, ui),
                    ShelfTab::Modifiers => modifier_tools(app, ui),
                    ShelfTab::Objects => object_tools(app, ui),
                    ShelfTab::View => view_tools(app, ui),
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let prefs_tip = app.tip(Action::Preferences);
                if shelf_button(ui, Icon::Settings, theme::text(), true, app.prefs_open, &prefs_tip).clicked() {
                    app.prefs_open = !app.prefs_open;
                }
                shelf_gap(ui);
                let (undo_tip, redo_tip) = (app.tip(Action::Undo), app.tip(Action::Redo));
                if shelf_button(ui, Icon::Redo, theme::text(), app.history.can_redo(), false, &redo_tip).clicked() {
                    app.redo();
                }
                if shelf_button(ui, Icon::Undo, theme::text(), app.history.can_undo(), false, &undo_tip).clicked() {
                    app.undo();
                }
            });
        });
    });
}

/// A flat text tab on the shelf. The open one is a lighter block.
fn shelf_tab(ui: &mut Ui, label: &str, active: bool) -> egui::Response {
    let font = FontId::proportional(12.0);
    let width = ui.painter().layout_no_wrap(label.to_string(), font.clone(), theme::text()).size().x;
    let (rect, response) = ui.allocate_exact_size(vec2(width + 16.0, 22.0), Sense::click());
    if active {
        ui.painter().rect_filled(rect, theme::RADIUS, theme::section());
    } else if response.hovered() {
        ui.painter().rect_filled(rect, theme::RADIUS, theme::panel());
    }
    let color = if active || response.hovered() { theme::ink() } else { theme::text_dim() };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, color);
    ui.painter().galley(rect.center() - galley.size() * 0.5, galley, color);
    response
}

/// Keys, clearing and mirroring poses, and seeing motion over time.
fn animation_tools(app: &mut KayeApp, ui: &mut Ui) {
    let tint = theme::rig_icon();
    let can_key = !app.key_targets().is_empty();
    if shelf_button(ui, Icon::Key, tint, can_key, false, &app.tip(Action::InsertKey)).clicked() {
        app.insert_key();
    }
    if shelf_button(ui, Icon::Eraser, tint, can_key, false, &app.tip(Action::DeleteKey)).clicked() {
        app.delete_keys_at_frame();
    }
    shelf_gap(ui);
    let posable = app.selected_bone().is_some() || app.movable_rig().is_some();
    if shelf_button(ui, Icon::Reset, tint, posable, false, "Clear Pose").clicked() {
        app.reset_selected(true, true);
    }
    if shelf_button(ui, Icon::ClearPosition, tint, posable, false, &app.tip(Action::ClearPosition)).clicked() {
        app.reset_selected(true, false);
    }
    if shelf_button(ui, Icon::ClearRotation, tint, posable, false, &app.tip(Action::ClearRotation)).clicked() {
        app.reset_selected(false, true);
    }
    if shelf_button(ui, Icon::Mirror, tint, can_key, false, &app.tip(Action::MirrorPose)).clicked() {
        app.mirror_pose();
    }
    shelf_gap(ui);
    // Each of these adds the first time and takes it away the next, lit while it's there.
    let (can_add, can_delete) = (app.can_add_onion(), app.can_delete_onion());
    let onion_tip = if can_add { app.tip(Action::AddOnionSkin) } else { app.tip(Action::DeleteOnionSkin) };
    if shelf_button(ui, Icon::Onion, theme::onion_icon(), can_add || can_delete, !can_add && can_delete, &onion_tip).clicked() {
        if can_add {
            app.add_onion_skins();
        } else {
            app.delete_onion_skins();
        }
    }
    let bones = app.selected_bones();
    let all_have_path = !bones.is_empty() && bones.iter().all(|b| app.motion_paths.contains(b));
    let path_tip = if all_have_path { "Clear Motion Path" } else { "Create Motion Path" };
    if shelf_button(ui, Icon::MotionPath, tint, app.selected_bone().is_some(), all_have_path, path_tip).clicked() {
        if all_have_path {
            app.clear_motion_paths();
        } else {
            app.create_motion_paths();
        }
    }
}

/// Every modifier, for the selected bone, then baking them all into keys.
fn modifier_tools(app: &mut KayeApp, ui: &mut Ui) {
    let tint = theme::bone_icon();
    let bone = app.modifier_bone();
    let has_ik = bone.is_some_and(|b| app.has_ik(b));
    let has_unparent = bone.is_some_and(|b| app.has_unparent(b));
    let ik_on = bone.is_some_and(|b| app.ik_is_on(b));
    let unparent_on = bone.is_some_and(|b| app.unparent_is_on(b));
    // Once a bone has one of these, the button switches it, leaving the limb put.
    let ik_tip = if !has_ik { "Inverse Kinematics" } else if ik_on { "Switch to FK" } else { "Switch to IK" };
    if shelf_button(ui, Icon::Chain, tint, bone.is_some(), ik_on, ik_tip).clicked() {
        if let Some(bone) = bone {
            if has_ik {
                app.switch_ik(bone, !ik_on);
            } else {
                app.add_ik(bone);
            }
        }
    }
    let unparent_tip = if has_unparent && unparent_on { "Parent" } else { "Unparent" };
    if shelf_button(ui, Icon::Unparent, tint, bone.is_some(), unparent_on, unparent_tip).clicked() {
        if let Some(bone) = bone {
            if has_unparent {
                app.switch_unparent(bone, !unparent_on);
            } else {
                app.add_unparent(bone);
            }
        }
    }
    // The rest can only be added once, as in the Add Modifier menu.
    let add: [(Icon, &str, u8, fn(&mut KayeApp, BoneId)); 5] = [
        (Icon::Target, "Look At", 2, KayeApp::add_look_at),
        (Icon::Copy, "Copy Bone", 3, KayeApp::add_copy),
        (Icon::Limit, "Limit Rotation", 4, KayeApp::add_limit),
        (Icon::Rotate, "Inherit Rotation", 5, KayeApp::add_inherit),
        (Icon::Ragdoll, "Ragdoll", modifiers::RAGDOLL_KIND, KayeApp::add_ragdoll),
    ];
    for (icon, tip, kind, add) in add {
        let free = bone.filter(|b| !app.has_modifier(*b, kind));
        if shelf_button(ui, icon, tint, free.is_some(), false, tip).clicked() {
            if let Some(bone) = free {
                add(app, bone);
            }
        }
    }
    shelf_gap(ui);
    if shelf_button(ui, Icon::Bake, tint, !app.scene.modifiers.is_empty(), false, "Bake Animation").clicked() {
        app.bake_all_modifiers();
    }
}

/// Cameras, shapes and folders to add, and deleting what's selected.
fn object_tools(app: &mut KayeApp, ui: &mut Ui) {
    if shelf_button(ui, Icon::Camera, theme::camera_icon(), true, false, &app.tip(Action::AddCamera)).clicked() {
        app.add_camera();
    }
    shelf_gap(ui);
    for shape in PartShape::ALL {
        let icon = match shape {
            PartShape::Block => Icon::Part,
            PartShape::Ball => Icon::Sphere,
            PartShape::Cylinder => Icon::Cylinder,
            PartShape::Wedge => Icon::Wedge,
            PartShape::CornerWedge => Icon::CornerWedge,
        };
        if shelf_button(ui, icon, theme::part_icon(), true, false, shape.label()).clicked() {
            app.add_shape(shape);
        }
    }
    if shelf_button(ui, Icon::Baseplate, theme::part_icon(), true, false, "Baseplate").clicked() {
        app.add_baseplate();
    }
    shelf_gap(ui);
    if shelf_button(ui, Icon::Image, theme::text(), true, false, "Reference").clicked() {
        super::references::pick_files(app, ui.ctx());
    }
    if shelf_button(ui, Icon::NewFolder, theme::text(), true, false, "New Folder").clicked() {
        let parent = app.selected_folder();
        app.new_folder(parent);
    }
    if shelf_button(ui, Icon::Trash, theme::text(), app.selection != Selection::None, false, &app.tip(Action::Delete)).clicked() {
        app.delete_selection();
    }
}

/// Framing, views, what's drawn, and looking through a camera.
fn view_tools(app: &mut KayeApp, ui: &mut Ui) {
    let tint = theme::text();
    if shelf_button(ui, Icon::Frame, tint, true, false, &app.tip(Action::FrameAll)).clicked() {
        app.frame_all();
    }
    if shelf_button(ui, Icon::ZoomIn, tint, app.selection != Selection::None, false, &app.tip(Action::FrameSelected)).clicked() {
        app.frame_selection();
    }
    let views = shelf_button(ui, Icon::Views, tint, true, false, "Views");
    egui::Popup::menu(&views).show(|ui| {
        menu_rows(ui);
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
        ui.checkbox(&mut app.camera.ortho, "Orthographic");
    });
    shelf_gap(ui);
    let shaded = app.settings.viewport.shading == Shading::Shaded;
    if shelf_button(ui, Icon::Sun, tint, true, shaded, "Shaded").clicked() {
        app.settings.viewport.shading = if shaded { Shading::Studio } else { Shading::Shaded };
    }
    if shelf_button(ui, Icon::Rig, tint, true, app.view.show_parts, "Show Parts").clicked() {
        app.view.show_parts = !app.view.show_parts;
    }
    if shelf_button(ui, Icon::Bone, tint, true, app.view.show_bones, "Show Bones").clicked() {
        app.view.show_bones = !app.view.show_bones;
    }
    if shelf_button(ui, Icon::Grid, tint, true, app.settings.viewport.show_grid, "Show Grid").clicked() {
        app.settings.viewport.show_grid = !app.settings.viewport.show_grid;
    }
    shelf_gap(ui);
    let has_camera = app.active_camera().is_some();
    if shelf_button(ui, Icon::Aperture, theme::camera_icon(), has_camera, app.camera_view.through, &app.tip(Action::CameraView)).clicked() {
        app.toggle_camera_view();
    }
    if shelf_button(ui, Icon::PictureInPicture, theme::camera_icon(), has_camera, app.camera_view.preview, &app.tip(Action::CameraPreview)).clicked() {
        app.toggle_camera_preview();
    }
}

/// A colored icon. The color says what family it belongs to.
pub(super) fn shelf_button(ui: &mut Ui, icon: Icon, tint: Color32, enabled: bool, on: bool, tip: &str) -> egui::Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(32.0, SHELF_HEIGHT - 4.0), sense);
    let fill = if on {
        theme::toggled()
    } else if enabled && response.is_pointer_button_down_on() {
        theme::field_active()
    } else if enabled && response.hovered() {
        theme::field()
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, theme::RADIUS, fill);
    let color = if enabled { tint } else { theme::text_faint().gamma_multiply(0.45) };
    icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), Vec2::splat(SHELF_ICON)), icon, color);
    tooltip(response, tip)
}

pub(super) fn shelf_gap(ui: &mut Ui) {
    ui.add_space(3.0);
    divider(ui, SHELF_HEIGHT - 14.0);
    ui.add_space(3.0);
}

/// Select, Move and Rotate, where Maya keeps them.
pub fn tool_column(app: &mut KayeApp, ui: &mut Ui) {
    Panel::left("tool_column").exact_size(TOOL_COLUMN_WIDTH).resizable(false).frame(bar_frame(theme::panel(), 4, 6)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (tool, icon, action) in [
            (Tool::Select, Icon::Select, Action::ToolSelect),
            (Tool::Move, Icon::Move, Action::ToolMove),
            (Tool::Rotate, Icon::Rotate, Action::ToolRotate),
        ] {
            let tip = app.tip(action);
            // The current tool is the one thing on screen that gets a full accent fill.
            let highlight = (app.tool == tool).then_some((theme::accent(), theme::on_accent()));
            if sized_icon_button(ui, icon, true, highlight, &tip, Vec2::splat(30.0), 19.0).clicked() {
                app.tool = tool;
            }
        }
    });
}

pub fn status_bar(app: &mut KayeApp, ui: &mut Ui) {
    Panel::bottom("status_bar").exact_size(STATUS_BAR_HEIGHT).frame(bar_frame(theme::panel(), 8, 0)).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            let s = &app.scene;
            let (icon, tint, path) = match app.selection {
                Selection::Bone(b) if s.rigs[s.bones[b].rig].kind.is_single() => {
                    let (icon, tint) = bone_icon(s.bones[b].kind);
                    (Some(icon), tint, s.bones[b].name.clone())
                }
                Selection::Bone(b) => (Some(Icon::Bone), theme::bone_icon(), format!("{}  /  {}", s.rigs[s.bones[b].rig].name, s.bones[b].name)),
                Selection::Part(p) => (Some(Icon::Part), theme::part_icon(), format!("{}  /  {}", s.rigs[s.parts[p].rig].name, s.parts[p].name)),
                Selection::Rig(r) if s.rigs[r].kind == RigKind::Camera => (Some(Icon::Part), theme::part_icon(), format!("{}  /  Root", s.rigs[r].name)),
                Selection::Rig(r) => (Some(Icon::Rig), theme::rig_icon(), s.rigs[r].name.clone()),
                Selection::Folder(f) => (Some(Icon::Folder), theme::text_dim(), s.folders[f].name.clone()),
                Selection::Onion(r) => (Some(Icon::Onion), theme::onion_icon(), format!("{}  /  Onion Skin", s.rigs[r].name)),
                Selection::Attachment(a) => {
                    let part = &s.parts[s.attachments[a].part];
                    (Some(Icon::Attachment), theme::bone_icon(), format!("{}  /  {}  /  {}", s.rigs[part.rig].name, part.name, s.attachments[a].name))
                }
                Selection::None => (None, theme::text_dim(), String::new()),
            };
            if let Some(icon) = icon {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(13.0), Sense::hover());
                icons::paint(ui.painter(), rect, icon, tint);
            }
            ui.add(Label::new(RichText::new(path).small().color(theme::text_dim())).truncate());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).small().color(theme::text_faint()));
                // After Later, the update stays a click away until Kaye closes.
                if let Some(version) = app.update.waiting().map(str::to_string) {
                    divider(ui, 12.0);
                    let link = Button::new(RichText::new(format!("Update to {version}")).small().color(theme::accent())).frame(false);
                    if ui.add(link).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        app.update.reopen();
                    }
                }
                let counts = app.counts();
                if counts.parts > 0 || counts.bones > 0 {
                    divider(ui, 12.0);
                    ui.label(RichText::new(format!("{} parts", counts.parts)).small().color(theme::text_dim()));
                    divider(ui, 12.0);
                    ui.label(RichText::new(format!("{} bones", counts.bones)).small().color(theme::text_dim()));
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
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            ui.label(RichText::new(concat!("Kaye ", env!("CARGO_PKG_VERSION"))).font(theme::strong_font(13.0)));
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
        ui.label(RichText::new(format!("Save changes to {name}?")).font(theme::strong_font(12.5)));
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add(Button::new(RichText::new("Save").color(theme::on_accent())).fill(theme::accent())).clicked() {
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
