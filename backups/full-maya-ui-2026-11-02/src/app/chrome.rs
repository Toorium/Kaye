//! Everything around the viewport, laid out the way Maya lays it out: menu bar, status
//! line, shelf, toolbox and command line. Also dialogs, and the controls every editor
//! shares.

use egui::{Align, Align2, Button, Color32, ComboBox, CornerRadius, Frame, Label, Layout, Margin, Panel, Pos2, RichText, Sense, Shape, Stroke, TextEdit, Ui, Vec2, vec2};

use super::icons::{self, Icon};
use super::theme;
use super::{KayeApp, Leave, Selection, Tool};
use crate::camera::ViewPreset;
use crate::gizmo::Space;
use crate::scene::{BoneKind, RigKind};
use crate::settings::{Action, PanelLayout, Shading, ShelfTab};

/// The strip along the top of an editor, holding its tabs or menus.
pub const STRIP_HEIGHT: f32 = 21.0;
const MENU_BAR_HEIGHT: f32 = 24.0;
const STATUS_LINE_HEIGHT: f32 = 30.0;
const SHELF_TAB_HEIGHT: f32 = 19.0;
const SHELF_BUTTON: f32 = 36.0;
const SHELF_ICON: f32 = 25.0;
const TOOLBOX_WIDTH: f32 = 38.0;
const COMMAND_LINE_HEIGHT: f32 = 24.0;

fn bar_frame(fill: Color32, x: i8, y: i8) -> Frame {
    Frame::new().fill(fill).inner_margin(Margin::symmetric(x, y))
}

/// The frame of an editor's strip.
pub fn strip_frame() -> Frame {
    bar_frame(theme::panel(), 4, 0)
}

/// One row of flat menus across the top of an editor.
pub fn menu_strip(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    strip_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(STRIP_HEIGHT);
        ui.horizontal_centered(|ui| {
            egui::MenuBar::new().ui(ui, content);
        });
    });
}

/// A flat text tab. The active one is a lighter block, as in Maya's panel headers.
pub fn editor_tab(ui: &mut Ui, label: &str, active: bool) -> egui::Response {
    let color = if active { theme::ink() } else { theme::text_dim() };
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(12.0), color);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 16.0, STRIP_HEIGHT - 2.0), Sense::click());
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
        BoneKind::Camera => (Icon::Camera, theme::CAMERA_ICON),
        BoneKind::IkTarget | BoneKind::IkPole => (Icon::Target, theme::accent_soft()),
        _ => (Icon::Bone, theme::BONE_ICON),
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

/// A larger flat button, for the status line.
pub fn wide_icon_button(ui: &mut Ui, icon: Icon, enabled: bool, active: bool, tip: &str) -> egui::Response {
    let highlight = active.then_some((theme::toggled(), theme::ink()));
    sized_icon_button(ui, icon, enabled, highlight, tip, vec2(25.0, 24.0), 17.0)
}

/// A button whose on state has its own color, as Auto Key's red does.
pub fn colored_icon_button(ui: &mut Ui, icon: Icon, on: Option<Color32>, tip: &str) -> egui::Response {
    let highlight = on.map(|fill| (fill, theme::ink()));
    sized_icon_button(ui, icon, true, highlight, tip, Vec2::splat(22.0), 15.0)
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
                    edit_menu(app, ui);
                    create_menu(app, ui);
                    select_menu(app, ui);
                    key_menu(app, ui);
                    playback_menu(app, ui);
                    rig_menu(app, ui);
                    display_menu(app, ui);
                    windows_menu(app, ui);
                    help_menu(app, ui);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(2.0);
                        // Where Maya names its workspace. Kaye has one layout, so this names the project.
                        let title = app.project_name().unwrap_or_else(|| "Untitled".to_string());
                        Frame::new().fill(theme::well()).inner_margin(Margin::symmetric(6, 2)).show(ui, |ui| {
                            ui.set_width(150.0);
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.add(Label::new(RichText::new(title).color(theme::text_dim())).truncate());
                            });
                        });
                        ui.label(RichText::new("Project :").color(theme::text_faint()));
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
    menu(ui, "Edit", |ui| {
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
    });
}

fn select_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "Select", |ui| {
        let has_rig = app.focused_rig().is_some();
        if item(ui, "All Bones", "", has_rig) {
            app.select_all_bones();
        }
        if item(ui, "Deselect All", &app.key_text(Action::Deselect), app.selection != Selection::None) {
            app.select(Selection::None);
        }
        if item(ui, "Invert", "", has_rig) {
            app.invert_selection();
        }
        ui.separator();
        if item(ui, "Hierarchy", "", app.selected_bone().is_some()) {
            app.select_hierarchy();
        }
    });
}

fn key_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "Key", |ui| {
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
    });
}

fn playback_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "Playback", |ui| {
        if item(ui, if app.playing { "Pause" } else { "Play" }, &app.key_text(Action::PlayPause), true) {
            app.toggle_play();
        }
        ui.checkbox(&mut app.animation.looping, "Loop");
        ui.separator();
        if item(ui, "Next Frame", &app.key_text(Action::FrameForward), true) {
            app.step_frame(1);
        }
        if item(ui, "Previous Frame", &app.key_text(Action::FrameBack), true) {
            app.step_frame(-1);
        }
        let has_keys = !app.animation.tracks.is_empty();
        if item(ui, "Next Key", &app.key_text(Action::NextKey), has_keys) {
            app.jump_key(true);
        }
        if item(ui, "Previous Key", &app.key_text(Action::PrevKey), has_keys) {
            app.jump_key(false);
        }
        ui.separator();
        if item(ui, "Go to Start", &app.key_text(Action::JumpStart), true) {
            app.pause();
            app.set_frame(app.animation.start as f32);
        }
        if item(ui, "Go to End", &app.key_text(Action::JumpEnd), true) {
            app.pause();
            app.set_frame(app.animation.end as f32);
        }
    });
}

fn rig_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "Rig", |ui| {
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
        item(ui, "Mirror Pose", "", false);
    });
}

fn display_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "Display", |ui| {
        ui.checkbox(&mut app.view.show_parts, "Parts");
        ui.checkbox(&mut app.view.show_bones, "Bones");
        ui.checkbox(&mut app.view.bones_on_top, "Bones on Top");
        ui.checkbox(&mut app.settings.viewport.show_grid, "Grid");
    });
}

fn windows_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "Windows", |ui| {
        let interface = &mut app.settings.interface;
        ui.checkbox(&mut interface.show_workspace, "Workspace");
        ui.checkbox(&mut interface.show_channel_box, "Channel Box");
        ui.checkbox(&mut interface.show_shelf, "Shelf");
        ui.separator();
        if item(ui, "Dope Sheet", "", true) {
            app.set_layout(PanelLayout::DopeSheet);
        }
        if item(ui, "Graph Editor", "", true) {
            app.set_layout(PanelLayout::Graph);
        }
        let has_camera = app.active_camera().is_some();
        if item(ui, "Camera Preview", &app.key_text(Action::CameraPreview), has_camera) {
            app.toggle_camera_preview();
        }
        ui.separator();
        if item(ui, "Preferences...", &app.key_text(Action::Preferences), true) {
            app.prefs_open = true;
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
    });
}

pub fn lighting_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "Lighting", |ui| {
        ui.checkbox(&mut app.settings.lighting.shadows, "Shadows");
        ui.checkbox(&mut app.settings.lighting.floor_shadows, "Floor Shadows");
        ui.separator();
        if item(ui, "Lighting Settings...", "", true) {
            app.prefs_open = true;
            app.prefs.tab = super::preferences::PrefsTab::Lighting;
        }
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

pub fn panels_menu(app: &mut KayeApp, ui: &mut Ui) {
    menu(ui, "Panels", |ui| {
        let current = app.settings.interface.layout;
        for (layout, label) in [(PanelLayout::Single, "Single View"), (PanelLayout::DopeSheet, "View and Dope Sheet"), (PanelLayout::Graph, "View and Graph Editor")] {
            if ui.add(Button::new(label).selected(current == layout)).clicked() {
                app.set_layout(layout);
                ui.close();
            }
        }
    });
}

pub fn status_line(app: &mut KayeApp, ui: &mut Ui) {
    Panel::top("status_line").exact_size(STATUS_LINE_HEIGHT).frame(bar_frame(theme::panel(), 6, 0)).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            theme::sunken(ui);
            let mut space = app.view.space;
            ComboBox::from_id_salt("transform_space").selected_text(space.label()).width(92.0).show_ui(ui, |ui| {
                for option in [Space::Global, Space::Local] {
                    ui.selectable_value(&mut space, option, option.label());
                }
            });
            app.view.space = space;
            divider(ui, 20.0);

            let (new_tip, open_tip, save_tip, import_tip) = (app.tip(Action::NewProject), app.tip(Action::OpenProject), app.tip(Action::Save), app.tip(Action::Import));
            if wide_icon_button(ui, Icon::New, true, false, &new_tip).clicked() {
                app.request_leave(ui.ctx(), Leave::New);
            }
            if wide_icon_button(ui, Icon::Open, true, false, &open_tip).clicked() {
                app.request_leave(ui.ctx(), Leave::Open(None));
            }
            if wide_icon_button(ui, Icon::Save, true, false, &save_tip).clicked() {
                app.save_project(ui.ctx());
            }
            if wide_icon_button(ui, Icon::Import, true, false, &import_tip).clicked() {
                app.open_import_dialog(ui.ctx());
            }
            divider(ui, 20.0);

            let (undo_tip, redo_tip) = (app.tip(Action::Undo), app.tip(Action::Redo));
            if wide_icon_button(ui, Icon::Undo, app.history.can_undo(), false, &undo_tip).clicked() {
                app.undo();
            }
            if wide_icon_button(ui, Icon::Redo, app.history.can_redo(), false, &redo_tip).clicked() {
                app.redo();
            }
            divider(ui, 20.0);

            // Selection masks: what a click in the 3D view can pick.
            if wide_icon_button(ui, Icon::Bone, true, app.view.pick_bones, "Select Bones").clicked() {
                app.view.pick_bones ^= true;
            }
            if wide_icon_button(ui, Icon::Part, true, app.view.pick_parts, "Select Parts").clicked() {
                app.view.pick_parts ^= true;
            }
            divider(ui, 20.0);

            let has_bone = app.selected_bone().is_some();
            let (position_tip, rotation_tip) = (app.tip(Action::ClearPosition), app.tip(Action::ClearRotation));
            if wide_icon_button(ui, Icon::Move, has_bone, false, &position_tip).clicked() {
                app.reset_selected(true, false);
            }
            if wide_icon_button(ui, Icon::Rotate, has_bone, false, &rotation_tip).clicked() {
                app.reset_selected(false, true);
            }
            if wide_icon_button(ui, Icon::Reset, has_bone, false, "Clear Pose").clicked() {
                app.reset_selected(true, true);
            }
            divider(ui, 20.0);

            ui.add_space(4.0);
            ui.add_enabled(false, Label::new("Mirror: Off"));
            ui.add_space(4.0);

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let prefs_tip = app.tip(Action::Preferences);
                if wide_icon_button(ui, Icon::Settings, true, app.prefs_open, &prefs_tip).clicked() {
                    app.prefs_open = !app.prefs_open;
                }
                divider(ui, 20.0);
                let interface = &mut app.settings.interface;
                if wide_icon_button(ui, Icon::Channels, true, interface.show_channel_box, "Channel Box").clicked() {
                    interface.show_channel_box ^= true;
                }
                if wide_icon_button(ui, Icon::Workspace, true, interface.show_workspace, "Workspace").clicked() {
                    interface.show_workspace ^= true;
                }
                if wide_icon_button(ui, Icon::OnTop, true, interface.show_shelf, "Shelf").clicked() {
                    interface.show_shelf ^= true;
                }
                divider(ui, 20.0);
                let rigs: Vec<&crate::scene::Rig> = app.scene.rigs.iter().filter(|r| r.kind != RigKind::Camera).collect();
                if let [rig] = rigs.as_slice() {
                    ui.add_space(4.0);
                    ui.label(RichText::new(rig.kind.label()).small().color(theme::text_faint()));
                    ui.add_space(4.0);
                    ui.add(Label::new(RichText::new(&rig.name).color(theme::text_dim())).truncate());
                }
            });
        });
    });
}

pub fn shelf(app: &mut KayeApp, ui: &mut Ui) {
    if !app.settings.interface.show_shelf {
        return;
    }
    Panel::top("shelf").exact_size(SHELF_TAB_HEIGHT + SHELF_BUTTON + 4.0).frame(Frame::new().fill(theme::panel())).show(ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        bar_frame(theme::panel(), 4, 0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(SHELF_TAB_HEIGHT);
            ui.horizontal_centered(|ui| {
                for tab in ShelfTab::ALL {
                    if shelf_tab(ui, tab.label(), app.settings.interface.shelf == tab).clicked() {
                        app.settings.interface.shelf = tab;
                    }
                }
            });
        });
        bar_frame(theme::header(), 6, 2).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(SHELF_BUTTON);
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                match app.settings.interface.shelf {
                    ShelfTab::Animation => animation_shelf(app, ui),
                    ShelfTab::Rigging => rigging_shelf(app, ui),
                    ShelfTab::Cameras => cameras_shelf(app, ui),
                    ShelfTab::Display => display_shelf(app, ui),
                    ShelfTab::Roblox => roblox_shelf(app, ui),
                }
            });
        });
    });
}

fn shelf_tab(ui: &mut Ui, label: &str, active: bool) -> egui::Response {
    let color = if active { theme::ink() } else { theme::text_dim() };
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(11.5), color);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 22.0, SHELF_TAB_HEIGHT), Sense::click());
    if active {
        ui.painter().rect_filled(rect, 0.0, theme::header());
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::trough());
    }
    ui.painter().galley(rect.center() - galley.size() * 0.5, galley, color);
    response
}

/// A big colored button on the shelf. The color says what family it belongs to.
fn shelf_button(ui: &mut Ui, icon: Icon, tint: Color32, enabled: bool, on: bool, tip: &str) -> egui::Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(SHELF_BUTTON), sense);
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

fn shelf_gap(ui: &mut Ui) {
    ui.add_space(3.0);
    divider(ui, SHELF_BUTTON - 6.0);
    ui.add_space(3.0);
}

fn animation_shelf(app: &mut KayeApp, ui: &mut Ui) {
    let can_key = !app.key_targets().is_empty();
    let (set_tip, delete_tip) = (app.tip(Action::InsertKey), app.tip(Action::DeleteKey));
    if shelf_button(ui, Icon::SetKey, theme::RECORD, can_key, false, &set_tip).clicked() {
        app.insert_key();
    }
    if shelf_button(ui, Icon::KeyOutline, theme::RECORD, can_key, false, &delete_tip).clicked() {
        app.delete_keys_at_frame();
    }
    let auto_key = app.settings.animation.auto_key;
    if shelf_button(ui, Icon::AutoKey, theme::RECORD, true, auto_key, "Auto Key").clicked() {
        app.settings.animation.auto_key = !auto_key;
    }
    shelf_gap(ui);
    let has_keys = !app.animation.tracks.is_empty();
    let (prev_tip, next_tip) = (app.tip(Action::PrevKey), app.tip(Action::NextKey));
    if shelf_button(ui, Icon::PrevKey, theme::CAMERA_ICON, has_keys, false, &prev_tip).clicked() {
        app.jump_key(false);
    }
    if shelf_button(ui, Icon::NextKey, theme::CAMERA_ICON, has_keys, false, &next_tip).clicked() {
        app.jump_key(true);
    }
    shelf_gap(ui);
    let has_selected = !app.timeline.selected.is_empty();
    let (copy_tip, trash_tip) = (app.tip(Action::DuplicateKeys), app.tip(Action::DeleteSelectedKeys));
    if shelf_button(ui, Icon::Copy, theme::MODIFIER_ICON, has_selected, false, &copy_tip).clicked() {
        app.duplicate_selected_keys();
    }
    if shelf_button(ui, Icon::Trash, theme::MODIFIER_ICON, has_selected, false, &trash_tip).clicked() {
        app.delete_selected_keys();
    }
    shelf_gap(ui);
    let looping = app.animation.looping;
    if shelf_button(ui, Icon::Loop, theme::BONE_ICON, true, looping, "Loop").clicked() {
        app.animation.looping = !looping;
    }
    let play_tip = app.tip(Action::PlayPause);
    let icon = if app.playing { Icon::Pause } else { Icon::Play };
    if shelf_button(ui, icon, theme::BONE_ICON, true, app.playing, &play_tip).clicked() {
        app.toggle_play();
    }
}

fn rigging_shelf(app: &mut KayeApp, ui: &mut Ui) {
    let bone = app.modifier_bone();
    let has_ik = bone.is_some_and(|b| app.has_ik(b));
    let has_unparent = bone.is_some_and(|b| app.has_unparent(b));
    if shelf_button(ui, Icon::Chain, theme::MODIFIER_ICON, bone.is_some(), has_ik, "Inverse Kinematics").clicked() {
        if let Some(bone) = bone {
            app.add_ik(bone);
        }
    }
    if shelf_button(ui, Icon::Unparent, theme::MODIFIER_ICON, bone.is_some(), has_unparent, "Unparent").clicked() {
        if let Some(bone) = bone {
            app.add_unparent(bone);
        }
    }
    let _ = shelf_button(ui, Icon::Mirror, theme::MODIFIER_ICON, false, false, "Mirror Pose");
    shelf_gap(ui);
    let has_rig = app.focused_rig().is_some();
    if shelf_button(ui, Icon::Hierarchy, theme::BONE_ICON, app.selected_bone().is_some(), false, "Select Hierarchy").clicked() {
        app.select_hierarchy();
    }
    if shelf_button(ui, Icon::Rig, theme::RIG_ICON, has_rig, false, "Select All Bones").clicked() {
        app.select_all_bones();
    }
    shelf_gap(ui);
    if shelf_button(ui, Icon::Reset, theme::RIG_ICON, !app.scene.bones.is_empty(), false, "Clear All Poses").clicked() {
        app.reset_all_poses();
    }
}

fn cameras_shelf(app: &mut KayeApp, ui: &mut Ui) {
    let add_tip = app.tip(Action::AddCamera);
    if shelf_button(ui, Icon::Camera, theme::CAMERA_ICON, true, false, &add_tip).clicked() {
        app.add_camera();
    }
    let camera = app.active_camera();
    let has_camera = camera.is_some();
    let (view_tip, preview_tip) = (app.tip(Action::CameraView), app.tip(Action::CameraPreview));
    if shelf_button(ui, Icon::Target, theme::CAMERA_ICON, has_camera, has_camera && app.camera_view.through, &view_tip).clicked() {
        app.toggle_camera_view();
    }
    if shelf_button(ui, Icon::Frame, theme::CAMERA_ICON, has_camera, has_camera && app.camera_view.preview, &preview_tip).clicked() {
        app.toggle_camera_preview();
    }
    if shelf_button(ui, Icon::Move, theme::CAMERA_ICON, has_camera, false, "Align to View").clicked() {
        if let Some(camera) = camera {
            app.camera_to_view(camera);
        }
    }
    shelf_gap(ui);
    let (all_tip, selected_tip) = (app.tip(Action::FrameAll), app.tip(Action::FrameSelected));
    if shelf_button(ui, Icon::FrameAll, theme::MODIFIER_ICON, true, false, &all_tip).clicked() {
        app.frame_all();
    }
    if shelf_button(ui, Icon::Frame, theme::MODIFIER_ICON, true, false, &selected_tip).clicked() {
        app.frame_selection();
    }
    let ortho_tip = app.tip(Action::ToggleOrtho);
    if shelf_button(ui, Icon::Perspective, theme::MODIFIER_ICON, true, app.camera.ortho, &ortho_tip).clicked() {
        app.camera.ortho ^= true;
    }
}

fn display_shelf(app: &mut KayeApp, ui: &mut Ui) {
    let shading = app.settings.viewport.shading;
    for (mode, icon, tip) in [(Shading::Studio, Icon::Studio, "Studio"), (Shading::Shaded, Icon::Shaded, "Shaded")] {
        if shelf_button(ui, icon, theme::CAMERA_ICON, true, shading == mode, tip).clicked() {
            app.settings.viewport.shading = mode;
        }
    }
    shelf_gap(ui);
    let v = &mut app.view;
    if shelf_button(ui, Icon::Part, theme::PART_ICON, true, v.show_parts, "Parts").clicked() {
        v.show_parts ^= true;
    }
    if shelf_button(ui, Icon::Bone, theme::BONE_ICON, true, v.show_bones, "Bones").clicked() {
        v.show_bones ^= true;
    }
    if shelf_button(ui, Icon::OnTop, theme::BONE_ICON, true, v.bones_on_top, "Bones on Top").clicked() {
        v.bones_on_top ^= true;
    }
    shelf_gap(ui);
    let look = &mut app.settings.viewport;
    if shelf_button(ui, Icon::Colors, theme::RIG_ICON, true, look.part_colors, "Part Colors").clicked() {
        look.part_colors ^= true;
    }
    if shelf_button(ui, Icon::Grid, theme::text(), true, look.show_grid, "Grid").clicked() {
        look.show_grid ^= true;
    }
}

fn roblox_shelf(app: &mut KayeApp, ui: &mut Ui) {
    let import_tip = app.tip(Action::Import);
    if shelf_button(ui, Icon::Import, theme::MODIFIER_ICON, true, false, &import_tip).clicked() {
        app.open_import_dialog(ui.ctx());
    }
    let _ = shelf_button(ui, Icon::Export, theme::MODIFIER_ICON, false, false, "Export Rig");
    let _ = shelf_button(ui, Icon::Export, theme::RECORD, false, false, "Export Animation");
    shelf_gap(ui);
    let _ = shelf_button(ui, Icon::OnTop, theme::RIG_ICON, false, false, "Bake Animation");
}

pub fn toolbox(app: &mut KayeApp, ui: &mut Ui) {
    Panel::left("toolbox").exact_size(TOOLBOX_WIDTH).resizable(false).frame(bar_frame(theme::panel(), 4, 6)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (tool, icon, action) in [
            (Tool::Select, Icon::Select, Action::ToolSelect),
            (Tool::Move, Icon::Move, Action::ToolMove),
            (Tool::Rotate, Icon::Rotate, Action::ToolRotate),
        ] {
            let tip = app.tip(action);
            if tool_button(ui, icon, app.tool == tool, &tip).clicked() {
                app.tool = tool;
            }
        }
        toolbox_divider(ui);
        // The quick layouts, which Maya keeps at the foot of the same column.
        let layout = app.settings.interface.layout;
        for (choice, icon, tip) in [
            (PanelLayout::Single, Icon::LayoutSingle, "Single View"),
            (PanelLayout::DopeSheet, Icon::LayoutRows, "View and Dope Sheet"),
            (PanelLayout::Graph, Icon::LayoutGraph, "View and Graph Editor"),
        ] {
            if layout_button(ui, icon, layout == choice, tip).clicked() {
                app.set_layout(choice);
            }
        }
    });
}

fn tool_button(ui: &mut Ui, icon: Icon, active: bool, tip: &str) -> egui::Response {
    // The current tool is the one thing on screen that gets a full accent fill.
    let highlight = active.then_some((theme::accent(), theme::on_accent()));
    sized_icon_button(ui, icon, true, highlight, tip, Vec2::splat(30.0), 19.0)
}

fn layout_button(ui: &mut Ui, icon: Icon, active: bool, tip: &str) -> egui::Response {
    let highlight = active.then_some((theme::toggled(), theme::ink()));
    sized_icon_button(ui, icon, true, highlight, tip, Vec2::splat(30.0), 17.0)
}

fn toolbox_divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(30.0, 9.0), Sense::hover());
    let y = ui.painter().round_to_pixel_center(rect.center().y);
    ui.painter().hline(rect.x_range().shrink(4.0), y, Stroke::new(1.0, theme::window()));
    ui.painter().hline(rect.x_range().shrink(4.0), y + 1.0, Stroke::new(1.0, theme::field()));
}

/// Where Maya has its MEL line. It stays off until Luau plugins exist.
pub fn command_line(app: &mut KayeApp, ui: &mut Ui) {
    Panel::bottom("command_line").exact_size(COMMAND_LINE_HEIGHT).frame(bar_frame(theme::panel(), 6, 0)).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new("Luau").small().color(theme::text_dim()));
            let mut nothing = String::new();
            ui.add_enabled(false, TextEdit::singleline(&mut nothing).desired_width(320.0).margin(Margin::symmetric(4, 1)));
            divider(ui, 16.0);
            let s = &app.scene;
            let (icon, tint, path) = match app.selection {
                Selection::Bone(b) if s.bones[b].kind == BoneKind::Camera => (Some(Icon::Camera), theme::CAMERA_ICON, s.bones[b].name.clone()),
                Selection::Bone(b) => (Some(Icon::Bone), theme::BONE_ICON, format!("{}  /  {}", s.rigs[s.bones[b].rig].name, s.bones[b].name)),
                Selection::Part(p) => (Some(Icon::Part), theme::PART_ICON, format!("{}  /  {}", s.rigs[s.parts[p].rig].name, s.parts[p].name)),
                Selection::Rig(r) => (Some(Icon::Rig), theme::RIG_ICON, s.rigs[r].name.clone()),
                Selection::None => (None, theme::text_dim(), String::new()),
            };
            if let Some(icon) = icon {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(13.0), Sense::hover());
                icons::paint(ui.painter(), rect, icon, tint);
            }
            ui.add(Label::new(RichText::new(path).small().color(theme::text_dim())).truncate());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).small().color(theme::text_faint()));
                if !s.is_empty() {
                    divider(ui, 12.0);
                    ui.label(RichText::new(format!("{} parts", s.parts.len())).small().color(theme::text_dim()));
                    divider(ui, 12.0);
                    let joints = s.bones.iter().filter(|b| b.kind.is_joint()).count();
                    ui.label(RichText::new(format!("{joints} bones")).small().color(theme::text_dim()));
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
