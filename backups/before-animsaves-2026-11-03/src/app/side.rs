//! The column beside the 3D view: the Workspace on top, which lists everything in the
//! scene, and Properties and Modifiers under it, laid out like Roblox Studio.

use egui::{Align2, CentralPanel, Color32, ComboBox, DragValue, FontId, Frame, Label, Margin, Panel, Rect, RichText, ScrollArea, Sense, Slider, SliderClamping, Stroke, TextEdit, Ui, Vec2, collapsing_header::CollapsingState, vec2};
use glam::{EulerRot, Quat, Vec3};

use super::chrome::{self, icon_button};
use super::icons::{self, Icon};
use super::{KayeApp, PropsTab, RowKey, Selection, theme};
use crate::animation::{CHANNEL_NAMES, angles_near};
use crate::math::Xf;
use crate::modifiers::{self, CopyBone, Ik, IkEnd, Limit, LookAt, Modifier, ModifierKind, Ragdoll, React, Unparent};
use crate::scene::{BoneId, BoneKind, BoneLink, Drive, PartId, RigId, RigKind};
use crate::settings::Action;

const ROW_HEIGHT: f32 = 20.0;
const INDENT: f32 = 14.0;
const CHANNEL_HEIGHT: f32 = 20.0;
const LABEL_WIDTH: f32 = 74.0;
/// Widest a field of view can go before a picture falls apart.
const MAX_FOV: f32 = 120.0;

pub fn panel(app: &mut KayeApp, ui: &mut Ui) {
    Panel::right("side")
        .default_size(300.0)
        .size_range(220.0..=520.0)
        // The two sections sit on one fill, since filling each would double up on
        // transparency when an image is behind the UI.
        .frame(Frame::new().fill(theme::panel()))
        .show(ui, |ui| {
            // A share of the column, so a short window or a big interface scale still
            // leaves room for Properties.
            let height = (ui.available_height() * 0.44).max(110.0);
            Panel::top("workspace")
                .resizable(true)
                .default_size(height)
                .size_range(110.0..=640.0)
                .frame(Frame::new())
                .show(ui, |ui| outliner(app, ui));
            CentralPanel::default().frame(Frame::new()).show(ui, |ui| properties(app, ui));
        });
}

enum RowKind {
    Rig(RigId),
    Bone(BoneId),
    PartsFolder,
    Part(PartId),
}

struct Row {
    depth: usize,
    kind: RowKind,
    key: Option<RowKey>,
    expandable: bool,
}

fn flatten(app: &KayeApp) -> Vec<Row> {
    let mut rows = Vec::new();
    let filter = app.outliner_filter.trim().to_lowercase();
    if !filter.is_empty() {
        for (id, b) in app.scene.bones.iter().enumerate() {
            if !b.removed && b.name.to_lowercase().contains(&filter) {
                rows.push(Row { depth: 0, kind: RowKind::Bone(id), key: None, expandable: false });
            }
        }
        for (id, p) in app.scene.parts.iter().enumerate() {
            // A spawned shape is listed by its bone, as a camera is.
            if app.scene.rigs[p.rig].kind != RigKind::Shape && !app.scene.part_is_removed(id) && p.name.to_lowercase().contains(&filter) {
                rows.push(Row { depth: 0, kind: RowKind::Part(id), key: None, expandable: false });
            }
        }
        return rows;
    }
    for (rig_id, rig) in app.scene.rigs.iter().enumerate() {
        if rig.removed {
            continue;
        }
        // A camera or shape is one thing, not a rig with a bone in it.
        if rig.kind.is_single() {
            if let Some(object) = rig.bones.first().filter(|b| !app.scene.bones[**b].removed) {
                rows.push(Row { depth: 0, kind: RowKind::Bone(*object), key: None, expandable: false });
            }
            continue;
        }
        let rig_key = RowKey::Rig(rig_id);
        rows.push(Row { depth: 0, kind: RowKind::Rig(rig_id), key: Some(rig_key), expandable: true });
        if app.collapsed.contains(&rig_key) {
            continue;
        }
        for &b in &rig.bones {
            if app.scene.bones[b].parent.is_none() && !app.scene.bones[b].removed {
                push_bone(app, b, 1, &mut rows);
            }
        }
        // Collapsed by default: the bones are what people are here for.
        let parts_key = RowKey::Parts(rig_id);
        rows.push(Row { depth: 1, kind: RowKind::PartsFolder, key: Some(parts_key), expandable: true });
        if app.collapsed.contains(&parts_key) {
            for &p in rig.parts.iter().filter(|p| !app.scene.part_is_removed(**p)) {
                rows.push(Row { depth: 2, kind: RowKind::Part(p), key: None, expandable: false });
            }
        }
    }
    rows
}

fn push_bone(app: &KayeApp, bone: BoneId, depth: usize, rows: &mut Vec<Row>) {
    let key = RowKey::Bone(bone);
    let children: Vec<BoneId> = app.scene.bones[bone].children.iter().copied().filter(|c| !app.scene.bones[*c].removed).collect();
    rows.push(Row { depth, kind: RowKind::Bone(bone), key: Some(key), expandable: !children.is_empty() });
    if !app.collapsed.contains(&key) {
        for c in children {
            push_bone(app, c, depth + 1, rows);
        }
    }
}

fn outliner(app: &mut KayeApp, ui: &mut Ui) {
    chrome::strip_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(chrome::STRIP_HEIGHT);
        ui.horizontal_centered(|ui| {
            chrome::editor_tab(ui, "Workspace", true);
            ui.add_space(4.0);
            let search = ui.add(
                TextEdit::singleline(&mut app.outliner_filter)
                    .hint_text("Search")
                    .margin(Margin { left: 21, right: 5, top: 2, bottom: 2 })
                    .desired_width(f32::INFINITY),
            );
            let icon = Rect::from_center_size(search.rect.left_center() + vec2(11.0, 0.0), Vec2::splat(12.0));
            icons::paint(ui.painter(), icon, Icon::Search, theme::text_faint());
        });
    });

    let rows = flatten(app);
    Frame::new().fill(theme::panel_deep()).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        // Only the rows on screen get built. A model can have tens of thousands.
        ScrollArea::vertical().auto_shrink(false).id_salt("outliner_rows").show_rows(ui, ROW_HEIGHT, rows.len(), |ui, shown| {
            if app.scroll_to_selection {
                // The row may be off screen and not built, so its place has to be worked out.
                if let Some(index) = rows.iter().rposition(|row| row_is_selected(app, row)) {
                    let top = ui.max_rect().top() + (index as f32 - shown.start as f32) * ROW_HEIGHT;
                    ui.scroll_to_rect(Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=top + ROW_HEIGHT), None);
                }
            }
            for index in shown {
                outliner_row(app, ui, &rows[index]);
            }
        });
    });
    app.scroll_to_selection = false;
}

fn row_selection(row: &Row) -> Selection {
    match row.kind {
        RowKind::Rig(r) => Selection::Rig(r),
        RowKind::Bone(b) => Selection::Bone(b),
        RowKind::PartsFolder => Selection::None,
        RowKind::Part(p) => Selection::Part(p),
    }
}

fn row_is_selected(app: &KayeApp, row: &Row) -> bool {
    match row.kind {
        RowKind::Bone(b) => app.is_bone_selected(b),
        _ => row_selection(row) != Selection::None && app.selection == row_selection(row),
    }
}

/// The small boxed plus or minus on a row with children, as in Maya's outliner.
fn paint_expander(painter: &egui::Painter, center: egui::Pos2, open: bool) {
    let (x, y) = (painter.round_to_pixel_center(center.x), painter.round_to_pixel_center(center.y));
    let stroke = Stroke::new(1.0, theme::text_faint());
    painter.rect_stroke(Rect::from_center_size(egui::pos2(x, y), Vec2::splat(9.0)), 0.0, stroke, egui::StrokeKind::Middle);
    painter.hline((x - 2.5)..=(x + 2.5), y, stroke);
    if !open {
        painter.vline(x, (y - 2.5)..=(y + 2.5), stroke);
    }
}

fn outliner_row(app: &mut KayeApp, ui: &mut Ui, row: &Row) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let selection = row_selection(row);
    let (name, trailing, icon, tint) = match row.kind {
        RowKind::Rig(r) => {
            let rig = &app.scene.rigs[r];
            (rig.name.clone(), Some(rig.kind.label().to_string()), Icon::Rig, theme::RIG_ICON)
        }
        RowKind::Bone(b) => {
            let bone = &app.scene.bones[b];
            let (icon, tint) = chrome::bone_icon(bone.kind);
            // Named in words, so what a bone has on it doesn't rest on color.
            let on_bone: Vec<&str> = app.scene.modifiers.iter().filter(|m| m.bone == b).map(Modifier::short_label).collect();
            (bone.name.clone(), (!on_bone.is_empty()).then(|| on_bone.join(", ")), icon, tint)
        }
        RowKind::PartsFolder => ("Parts".to_string(), None, Icon::Folder, theme::text_dim()),
        RowKind::Part(p) => (app.scene.parts[p].name.clone(), None, Icon::Part, theme::PART_ICON),
    };
    let is_selected = row_is_selected(app, row);
    let is_active = selection != Selection::None && app.selection == selection;
    let is_hovered = selection != Selection::None && app.hover == selection;

    let indent = 4.0 + row.depth as f32 * INDENT;
    let expander = rect.left_center() + vec2(indent + 6.0, 0.0);
    let expander_rect = Rect::from_center_size(expander, Vec2::splat(11.0));
    let icon_rect = Rect::from_center_size(rect.left_center() + vec2(indent + 21.0, 0.0), Vec2::splat(14.0));
    let eye_rect = Rect::from_center_size(rect.right_center() - vec2(14.0, 0.0), Vec2::splat(14.0));

    let visible = match row.kind {
        RowKind::Rig(r) => Some(app.scene.rigs[r].visible),
        RowKind::Part(p) => Some(app.scene.parts[p].visible),
        RowKind::Bone(b) if app.scene.rigs[app.scene.bones[b].rig].kind.is_single() => Some(app.scene.rigs[app.scene.bones[b].rig].visible),
        _ => None,
    };

    let painter = ui.painter();
    if is_active {
        painter.rect_filled(rect, 0.0, theme::active_row());
    } else if is_selected {
        painter.rect_filled(rect, 0.0, theme::selected_row());
    } else if response.hovered() || is_hovered {
        painter.rect_filled(rect, 0.0, theme::header());
    }
    // Parts start collapsed, so that folder's stored flag means "open".
    let open = match row.key {
        Some(key @ RowKey::Parts(_)) => app.collapsed.contains(&key),
        Some(key) => !app.collapsed.contains(&key),
        None => false,
    };
    if row.expandable {
        paint_expander(painter, expander, open);
    }
    icons::paint(painter, icon_rect, icon, tint);
    let text_at = icon_rect.right_center() + vec2(5.0, 0.0);
    let color = if is_selected { theme::ink() } else { theme::text() };
    let galley = painter.layout_no_wrap(name, FontId::proportional(12.0), color);
    let text_width = galley.size().x;
    painter.galley(text_at - vec2(0.0, galley.size().y * 0.5), galley, color);
    if let Some(trailing) = trailing {
        painter.text(
            text_at + vec2(text_width + 7.0, 0.0),
            Align2::LEFT_CENTER,
            trailing,
            FontId::proportional(11.0),
            if is_selected { theme::ink() } else { theme::text_faint() },
        );
    }
    if let Some(visible) = visible {
        let (icon, color) = if visible { (Icon::Eye, theme::text_dim()) } else { (Icon::EyeOff, theme::text_faint()) };
        icons::paint(painter, eye_rect, icon, color);
    }

    if response.hovered() {
        app.hover = selection;
        app.outliner_hovering = true;
    }
    if response.secondary_clicked() && selection != Selection::None && !is_selected {
        app.select(selection);
    }
    response.context_menu(|ui| {
        chrome::menu_rows(ui);
        row_menu(app, ui, &row.kind);
    });
    if response.clicked() {
        let at = response.interact_pointer_pos().unwrap_or(rect.center());
        if visible.is_some() && eye_rect.expand(3.0).contains(at) {
            match row.kind {
                RowKind::Rig(r) => app.scene.rigs[r].visible ^= true,
                RowKind::Part(p) => app.scene.parts[p].visible ^= true,
                RowKind::Bone(b) => {
                    let rig = app.scene.bones[b].rig;
                    app.scene.rigs[rig].visible ^= true;
                }
                RowKind::PartsFolder => {}
            }
        } else if row.expandable && (expander_rect.expand(4.0).contains(at) || matches!(row.kind, RowKind::PartsFolder)) {
            if let Some(key) = row.key {
                if !app.collapsed.remove(&key) {
                    app.collapsed.insert(key);
                }
            }
        } else if let (RowKind::Bone(b), true) = (&row.kind, ui.input(|i| i.modifiers.shift || i.modifiers.ctrl)) {
            app.toggle_bone(*b);
        } else if selection != Selection::None {
            app.select(selection);
        }
    }
}

fn row_menu(app: &mut KayeApp, ui: &mut Ui, kind: &RowKind) {
    match kind {
        RowKind::Bone(_) => super::viewport::bone_menu(app, ui),
        RowKind::Rig(r) => {
            if ui.button("Insert Keyframes").clicked() {
                app.insert_key();
                ui.close();
            }
            if ui.button("Delete Keyframes on This Frame").clicked() {
                app.delete_keys_at_frame();
                ui.close();
            }
            if ui.button("Clear Pose").clicked() {
                let poses = app.scene.rigs[*r].bones.iter().map(|b| (*b, Xf::IDENTITY)).collect();
                app.apply_poses("Clear rig pose", poses);
                ui.close();
            }
            ui.separator();
            if ui.button("Frame").clicked() {
                app.frame_selection();
                ui.close();
            }
            let visible = app.scene.rigs[*r].visible;
            if ui.button(if visible { "Hide" } else { "Show" }).clicked() {
                app.scene.rigs[*r].visible = !visible;
                ui.close();
            }
            delete_row(app, ui);
        }
        RowKind::Part(p) => {
            if ui.button("Frame").clicked() {
                app.frame_selection();
                ui.close();
            }
            let visible = app.scene.parts[*p].visible;
            if ui.button(if visible { "Hide" } else { "Show" }).clicked() {
                app.scene.parts[*p].visible = !visible;
                ui.close();
            }
            delete_row(app, ui);
        }
        RowKind::PartsFolder => {}
    }
}

fn delete_row(app: &mut KayeApp, ui: &mut Ui) {
    ui.separator();
    if ui.add(egui::Button::new("Delete").shortcut_text(app.key_text(Action::Delete))).clicked() {
        app.delete_selection();
        ui.close();
    }
}

/// Under the Workspace, as Properties sits under Explorer in Roblox Studio.
fn properties(app: &mut KayeApp, ui: &mut Ui) {
    // Cameras and an empty selection have no modifiers, and a blank tab says nothing.
    let shown = if app.modifier_bone().is_some() { app.props_tab } else { PropsTab::Item };
    chrome::strip_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(chrome::STRIP_HEIGHT);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            for (tab, label) in [(PropsTab::Item, "Properties"), (PropsTab::Modifiers, "Modifiers")] {
                if chrome::editor_tab(ui, label, shown == tab).clicked() {
                    app.props_tab = tab;
                }
            }
        });
    });
    ScrollArea::vertical().auto_shrink(false).id_salt("properties").show(ui, |ui| {
        Frame::new().inner_margin(Margin::symmetric(4, 4)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match shown {
                PropsTab::Item => selected_tab(app, ui),
                PropsTab::Modifiers => modifiers_tab(app, ui),
            }
        });
    });
}

fn selected_tab(app: &mut KayeApp, ui: &mut Ui) {
    match app.selection {
        Selection::Bone(b) => bone_properties(app, ui, b),
        Selection::Part(p) => part_properties(app, ui, p),
        Selection::Rig(r) => rig_properties(app, ui, r),
        Selection::None => scene_properties(app, ui),
    }
}

/// With nothing selected, the panel describes the whole scene instead of sitting empty.
fn scene_properties(app: &mut KayeApp, ui: &mut Ui) {
    let s = &app.scene;
    section(ui, "Statistics", |ui| {
        let rigs = s.rigs.iter().filter(|r| !r.kind.is_single() && !r.removed).count();
        info(ui, "Rigs", &rigs.to_string());
        info(ui, "Bones", &s.bones.iter().filter(|b| b.kind.is_joint() && !b.removed).count().to_string());
        info(ui, "Parts", &s.live_parts().to_string());
        info(ui, "Cameras", &s.cameras().count().to_string());
    });
    let a = &app.animation;
    section(ui, "Animation", |ui| {
        info(ui, "Frames", &format!("{} to {}", a.start, a.end));
        info(ui, "Frame Rate", &format!("{} fps", a.fps));
        info(ui, "Keys", &a.tracks.values().map(|t| t.keys.len()).sum::<usize>().to_string());
    });
}

/// What the panel is showing: an icon, the name, what kind of thing it is, and any
/// buttons that act on the whole thing.
fn title_row(ui: &mut Ui, icon: Icon, tint: Color32, title: &str, kind: &str, buttons: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
        icons::paint(ui.painter(), rect, icon, tint);
        ui.add(Label::new(RichText::new(title).font(theme::strong_font(12.0)).color(theme::ink())).truncate());
        ui.label(RichText::new(kind).small().color(theme::text_faint()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), buttons);
    });
    ui.add_space(4.0);
}

/// A collapsible frame, as in Maya's Attribute Editor. Sections with the same title
/// share whether they're open, so moving between bones doesn't reset them.
fn section(ui: &mut Ui, title: &str, content: impl FnOnce(&mut Ui)) {
    let id = ui.make_persistent_id(("section", title));
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, true);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 21.0), Sense::click());
    ui.painter().rect_filled(rect, 0.0, if response.hovered() { theme::field_hover() } else { theme::section() });
    chrome::paint_triangle(ui.painter(), rect.left_center() + vec2(11.0, 0.0), state.is_open(), theme::ink());
    ui.painter().text(rect.left_center() + vec2(22.0, 0.0), Align2::LEFT_CENTER, title, theme::strong_font(11.5), theme::ink());
    if response.clicked() {
        state.toggle(ui);
    }
    state.show_body_unindented(ui, |ui| {
        Frame::new().inner_margin(Margin { left: 4, right: 6, top: 4, bottom: 6 }).show(ui, content);
    });
    state.store(ui.ctx());
    ui.add_space(2.0);
}

/// One label-and-value line in the shared two-column layout.
fn info(ui: &mut Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(LABEL_WIDTH, 18.0), Sense::hover());
        ui.painter().text(rect.right_center(), Align2::RIGHT_CENTER, label, FontId::proportional(12.0), theme::text_dim());
        ui.add_space(4.0);
        ui.add(Label::new(value).truncate());
    });
}

/// A number's text, showing only the digits it needs, as Maya's Channel Box does.
fn channel_field(value: &mut f32, speed: f64) -> DragValue<'_> {
    DragValue::new(value).speed(speed).custom_formatter(|value, _| {
        let text = format!("{value:.3}");
        let text = text.trim_end_matches('0').trim_end_matches('.');
        if text == "-0" { "0".to_string() } else { text.to_string() }
    })
}

/// One pose number: its name right-aligned against a dark field, as in Maya's Channel
/// Box. `tint` colors it by how the value relates to the keys.
fn channel_row(ui: &mut Ui, label: &str, field: DragValue<'_>, tint: Option<Color32>) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.spacing_mut().interact_size.y = CHANNEL_HEIGHT;
        ui.spacing_mut().button_padding.y = 1.0;
        theme::sunken(ui);
        if let Some(tint) = tint {
            let visuals = ui.visuals_mut();
            visuals.override_text_color = Some(theme::ink());
            visuals.widgets.inactive.weak_bg_fill = tint;
            visuals.widgets.hovered.weak_bg_fill = theme::lighter(tint);
        }
        let width = (ui.available_width() * 0.5).clamp(90.0, 160.0);
        let (rect, _) = ui.allocate_exact_size(vec2((ui.available_width() - width - 6.0).max(0.0), CHANNEL_HEIGHT), Sense::hover());
        ui.painter().text(rect.right_center(), Align2::RIGHT_CENTER, label, FontId::proportional(12.0), theme::text());
        ui.add_space(2.0);
        ui.add_sized([width, CHANNEL_HEIGHT], field)
    })
    .inner
}

fn bone_properties(app: &mut KayeApp, ui: &mut Ui, id: BoneId) {
    let bone = &app.scene.bones[id];
    let frame = app.whole_frame();
    let control = modifiers::control_of(&app.scene, id);
    let keyed = app.animation.key_at(control, frame).is_some();
    let tint = if keyed {
        Some(theme::keyed_field())
    } else if app.animation.tracks.contains_key(&control) {
        Some(theme::animated_field())
    } else {
        None
    };
    let mut toggle_key = false;
    let (icon, icon_tint) = chrome::bone_icon(bone.kind);
    title_row(ui, icon, icon_tint, &bone.name, bone.kind.label(), |ui| {
        let icon = if keyed { Icon::Key } else { Icon::KeyOutline };
        let tip = if keyed { app.tip(Action::DeleteKey) } else { app.tip(Action::InsertKey) };
        toggle_key = icon_button(ui, icon, true, keyed, &tip).clicked();
    });
    if toggle_key {
        if keyed {
            app.delete_keys_at_frame();
        } else {
            app.insert_key();
        }
    }

    let pose = app.scene.bones[id].pose;
    let mut pos = pose.pos.to_array();
    // The same angles the Graph tab shows. Tiny float noise around zero would
    // otherwise display as "-0.0".
    let mut rot = angles_near(pose.rot, [0.0; 3]).map(|degrees| if degrees.abs() < 0.0005 { 0.0 } else { degrees });
    let rot_before = rot;
    let mut responses = Vec::with_capacity(6);

    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        for i in 0..3 {
            responses.push(channel_row(ui, CHANNEL_NAMES[i], channel_field(&mut pos[i], 0.01), tint));
        }
        for i in 0..3 {
            responses.push(channel_row(ui, CHANNEL_NAMES[i + 3], channel_field(&mut rot[i], 0.5), tint));
        }
    });
    ui.add_space(6.0);

    let started = responses.iter().any(|r| r.drag_started() || r.gained_focus());
    let changed = responses.iter().any(|r| r.changed());
    let stopped = responses.iter().any(|r| r.drag_stopped() || r.lost_focus());
    if (started || changed) && app.panel_edit.is_none() {
        app.pause();
        app.panel_edit = Some((id, app.scene.bones[id].pose));
    }
    if changed {
        // Converting back only when rotation really changed stops a position
        // edit from nudging the rotation through Euler rounding.
        let new_rot = if rot == rot_before {
            pose.rot
        } else {
            Quat::from_euler(EulerRot::XYZ, rot[0].to_radians(), rot[1].to_radians(), rot[2].to_radians())
        };
        app.scene.set_bone_pose(id, Xf::new(new_rot, Vec3::from_array(pos)));
    }
    if stopped {
        app.commit_panel_edit();
    }

    match app.scene.bones[id].kind {
        BoneKind::Camera => camera_section(app, ui, id),
        BoneKind::IkTarget => section(ui, "Inverse Kinematics", |ui| value_row(app, ui, id, "Influence", 0.0..=1.0, "")),
        BoneKind::LookTarget => section(ui, "Look At", |ui| value_row(app, ui, id, "Influence", 0.0..=1.0, "")),
        BoneKind::Shape => shape_section(app, ui, id),
        BoneKind::IkPole => {}
        _ => {
            let bone = &app.scene.bones[id];
            let scene = &app.scene;
            section(ui, "Joint", |ui| match bone.link {
                BoneLink::Joint { part0, part1, .. } => {
                    info(ui, "Part0", &scene.parts[part0].name);
                    info(ui, "Part1", &scene.parts[part1].name);
                }
                BoneLink::Skinned { parent_part, parent_bone, .. } => {
                    let parent = parent_bone
                        .map(|b| scene.bones[b].name.as_str())
                        .or_else(|| parent_part.map(|p| scene.parts[p].name.as_str()))
                        .unwrap_or("-");
                    info(ui, "Parent", parent);
                }
                BoneLink::Free { .. } => {}
            });
        }
    }
}

/// A row in the Modifiers tab and camera settings: a label, then its control.
fn prop_row(ui: &mut Ui, label: &str, content: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (rect, _) = ui.allocate_exact_size(vec2(LABEL_WIDTH, 20.0), Sense::hover());
        ui.painter().text(rect.right_center(), Align2::RIGHT_CENTER, label, FontId::proportional(12.0), theme::text_dim());
        ui.add_space(2.0);
        content(ui);
    });
}

/// A button filling the rest of its row.
fn wide_button(ui: &mut Ui, text: impl Into<String>) -> egui::Response {
    let width = ui.available_width();
    ui.add(egui::Button::new(text.into()).truncate().min_size(vec2(width, 20.0)))
}

/// A whole number or angle filling the rest of its row.
fn wide_number<'a>(ui: &mut Ui, field: egui::DragValue<'a>) -> egui::Response {
    let width = ui.available_width();
    ui.scope(|ui| {
        theme::sunken(ui);
        ui.add_sized([width, 20.0], field)
    })
    .inner
}

/// An animated number, with the diamond that keys it on this frame.
fn value_row(app: &mut KayeApp, ui: &mut Ui, bone: BoneId, label: &str, range: std::ops::RangeInclusive<f32>, suffix: &str) {
    let mut value = app.scene.bones[bone].value;
    let keyed = app.animation.key_at(bone, app.whole_frame()).is_some();
    let mut changed = false;
    let mut toggle = false;
    prop_row(ui, label, |ui| {
        ui.spacing_mut().slider_width = (ui.available_width() - 76.0).max(30.0);
        // The default clamping rounds the existing value every frame and calls that a
        // change, so an animated number would drop a key on every frame scrubbed to.
        let slider = Slider::new(&mut value, range).suffix(suffix).max_decimals(2).clamping(SliderClamping::Edits);
        theme::sunken(ui);
        changed = ui.add(slider).changed();
        let icon = if keyed { Icon::Key } else { Icon::KeyOutline };
        toggle = icon_button(ui, icon, true, keyed, "Keyframe").clicked();
    });
    if changed {
        app.touch_value(bone, value);
    }
    if toggle {
        app.toggle_key(bone);
    }
}

fn camera_section(app: &mut KayeApp, ui: &mut Ui, id: BoneId) {
    section(ui, "Camera", |ui| {
        value_row(app, ui, id, "Field of View", 1.0..=MAX_FOV, "\u{b0}");
        prop_row(ui, "Frame", |ui| {
            let mut shape = app.camera_view.shape;
            theme::sunken(ui);
            ComboBox::from_id_salt("camera_shape").selected_text(shape.label()).width(ui.available_width() - 6.0).show_ui(ui, |ui| {
                for option in super::FrameShape::ALL {
                    ui.selectable_value(&mut shape, option, option.label());
                }
            });
            app.camera_view.shape = shape;
        });
        prop_row(ui, "", |ui| {
            if wide_button(ui, "Align to View").clicked() {
                app.camera_to_view(id);
            }
        });
    });
}

/// What can be changed about a spawned shape.
fn shape_section(app: &mut KayeApp, ui: &mut Ui, id: BoneId) {
    let Some(part) = app.scene.bones[id].part else {
        return;
    };
    section(ui, "Shape", |ui| {
        let mut size = app.scene.parts[part].size.to_array();
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (i, axis) in ["Size X", "Size Y", "Size Z"].into_iter().enumerate() {
                let field = channel_field(&mut size[i], 0.05).range(0.05..=2048.0);
                if channel_row(ui, axis, field, None).changed() {
                    app.touch_rig("Change shape", |scene| scene.parts[part].size = Vec3::from_array(size));
                }
            }
        });
        ui.add_space(3.0);
        prop_row(ui, "Color", |ui| {
            let mut color = app.scene.parts[part].color;
            if ui.color_edit_button_rgb(&mut color).changed() {
                app.touch_rig("Change shape", |scene| scene.parts[part].color = color);
            }
        });
        prop_row(ui, "Transparency", |ui| {
            let mut transparency = app.scene.parts[part].transparency;
            ui.spacing_mut().slider_width = (ui.available_width() - 50.0).max(30.0);
            theme::sunken(ui);
            // Fully see-through would make it a helper part, which can't be clicked.
            if ui.add(Slider::new(&mut transparency, 0.0..=0.95).clamping(SliderClamping::Edits).max_decimals(2)).changed() {
                app.touch_rig("Change shape", |scene| scene.parts[part].transparency = transparency);
            }
        });
        let BoneLink::Free { follow, .. } = app.scene.bones[id].link else {
            return;
        };
        prop_row(ui, "Follow", |ui| {
            let choices = (0..app.scene.bones.len()).filter(|b| app.scene.bones[*b].kind.is_joint() && !app.scene.bones[*b].removed).collect();
            if let Some(choice) = follow_combo(app, ui, id, follow, choices, "World") {
                app.edit_rig("Change follow", |scene, _| modifiers::set_follow(scene, id, choice));
            }
        });
    });
}

/// `rbxassetid://123` and `http://www.roblox.com/asset/?id=123` both show as `123`.
fn asset_label(asset: Option<&str>) -> &str {
    let Some(asset) = asset else {
        return "-";
    };
    match asset.rsplit(|c: char| !c.is_ascii_digit()).next() {
        Some(id) if !id.is_empty() => id,
        _ => asset,
    }
}

fn part_properties(app: &mut KayeApp, ui: &mut Ui, id: PartId) {
    let part = &app.scene.parts[id];
    title_row(ui, Icon::Part, theme::PART_ICON, &part.name, &part.class, |_| {});
    let s = part.size;
    section(ui, "Part", |ui| {
        info(ui, "Size", &format!("{:.2}, {:.2}, {:.2}", s.x, s.y, s.z));
        match part.drive {
            Drive::Root => {}
            Drive::Bone(b) => info(ui, "Bone", &app.scene.bones[b].name),
            Drive::Rigid { parent, .. } => info(ui, "Welded to", &app.scene.parts[parent].name),
        }
    });
    let missing = part.mesh_asset.as_deref().and_then(crate::rbxmesh::asset_id).is_some_and(|id| app.assets.is_missing_mesh(id));
    section(ui, "Assets", |ui| {
        let mesh = asset_label(part.mesh_asset.as_deref());
        info(ui, "Mesh", &if missing { format!("{mesh} (unavailable)") } else { mesh.to_string() });
        info(ui, "Texture", asset_label(part.texture_asset.as_deref()));
    });
}

fn rig_properties(app: &mut KayeApp, ui: &mut Ui, id: RigId) {
    let bones = app.scene.joints(id);
    let rig = &app.scene.rigs[id];
    if rig.kind.is_single() {
        return;
    }
    let any_posed = bones.iter().any(|b| !app.scene.bones[*b].pose.approx_eq(Xf::IDENTITY));
    let mut clear = false;
    title_row(ui, Icon::Rig, theme::RIG_ICON, &rig.name, rig.kind.label(), |ui| {
        clear = icon_button(ui, Icon::Reset, any_posed, false, "Clear Rig Pose").clicked();
    });
    section(ui, "Rig", |ui| {
        info(ui, "File", &rig.source_file);
        info(ui, "Bones", &bones.len().to_string());
        info(ui, "Parts", &rig.parts.iter().filter(|p| !app.scene.part_is_removed(**p)).count().to_string());
    });
    if clear {
        let poses = bones.into_iter().map(|b| (b, Xf::IDENTITY)).collect();
        app.apply_poses("Clear rig pose", poses);
    }
    placement_section(app, ui, id);
}

/// Where the whole rig stands. It's an undo step, and it stays out of the animation.
fn placement_section(app: &mut KayeApp, ui: &mut Ui, id: RigId) {
    section(ui, "Placement", |ui| {
        let placement = app.scene.rigs[id].placement;
        let mut pos = placement.pos.to_array();
        let mut rot = angles_near(placement.rot, [0.0; 3]).map(|degrees| if degrees.abs() < 0.0005 { 0.0 } else { degrees });
        let rot_before = rot;
        let mut changed = false;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for i in 0..3 {
                changed |= channel_row(ui, CHANNEL_NAMES[i], channel_field(&mut pos[i], 0.05), None).changed();
            }
            for i in 0..3 {
                changed |= channel_row(ui, CHANNEL_NAMES[i + 3], channel_field(&mut rot[i], 0.5), None).changed();
            }
        });
        if changed {
            let mut moved = Xf::new(placement.rot, Vec3::from_array(pos));
            if rot != rot_before {
                // Turns on the part it stands on, the same as dragging the handles.
                let pivot = app.scene.rig_frame(id).pos;
                moved.rot = Quat::from_euler(EulerRot::XYZ, rot[0].to_radians(), rot[1].to_radians(), rot[2].to_radians());
                moved.pos = pivot + (moved.rot * placement.rot.inverse()) * (moved.pos - pivot);
            }
            app.touch_rig("Move rig", |scene| scene.rigs[id].placement = moved);
        }
    });
}

fn modifiers_tab(app: &mut KayeApp, ui: &mut Ui) {
    let Some(bone) = app.modifier_bone() else {
        return;
    };
    let (icon, tint) = chrome::bone_icon(app.scene.bones[bone].kind);
    title_row(ui, icon, tint, &app.scene.bones[bone].name.clone(), app.scene.bones[bone].kind.label(), |_| {});
    let add = egui::Button::new("Add Modifier").min_size(vec2(ui.available_width(), 22.0));
    egui::containers::menu::MenuButton::from_button(add).ui(ui, |ui| {
        chrome::menu_rows(ui);
        add_modifier_items(app, ui, bone);
    });
    ui.add_space(4.0);
    let cards: Vec<usize> = (0..app.scene.modifiers.len()).filter(|i| app.scene.modifiers[*i].bone == bone).collect();
    // Cards are read before any can be removed, so a removal waits until they're done.
    let mut remove = None;
    for index in cards {
        if modifier_card(app, ui, index) {
            remove = Some(index);
        }
        ui.add_space(2.0);
    }
    if let Some(index) = remove {
        app.remove_modifier(index);
    }
}

/// The entries of the Add Modifier menu, shared with the bone's right-click menu.
pub fn add_modifier_items(app: &mut KayeApp, ui: &mut Ui, bone: BoneId) {
    let add: [(&str, u8, fn(&mut KayeApp, BoneId)); 8] = [
        ("Inverse Kinematics", 0, KayeApp::add_ik),
        ("Unparent", 1, KayeApp::add_unparent),
        ("Look At", 2, KayeApp::add_look_at),
        ("Copy Bone", 3, KayeApp::add_copy),
        ("Limit Rotation", 4, KayeApp::add_limit),
        ("Inherit Rotation", 5, KayeApp::add_inherit),
        ("React", modifiers::REACT_KIND, KayeApp::add_react),
        ("Ragdoll", modifiers::RAGDOLL_KIND, KayeApp::add_ragdoll),
    ];
    for (label, kind, add) in add {
        if ui.add_enabled(!app.has_modifier(bone, kind), egui::Button::new(label)).clicked() {
            add(app, bone);
            ui.close();
        }
    }
}

/// Draws one modifier as a collapsible frame. Returns true when its remove button was
/// clicked.
fn modifier_card(app: &mut KayeApp, ui: &mut Ui, index: usize) -> bool {
    let modifier = app.scene.modifiers[index];
    let key = RowKey::Modifier(modifier.bone, modifier.kind.index());
    let open = !app.collapsed.contains(&key);
    let mut remove = false;
    let mut bake = false;
    Frame::new().fill(theme::section()).inner_margin(Margin::symmetric(3, 0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
            chrome::paint_triangle(ui.painter(), rect.center(), open, theme::ink());
            if response.clicked() && !app.collapsed.remove(&key) {
                app.collapsed.insert(key);
            }
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
            let icon = match modifier.kind {
                ModifierKind::Ik(_) => Icon::Chain,
                ModifierKind::Unparent(_) => Icon::Unparent,
                ModifierKind::LookAt(_) => Icon::Target,
                ModifierKind::Copy(_) => Icon::Copy,
                ModifierKind::Limit(_) => Icon::Limit,
                ModifierKind::Inherit(_) => Icon::Rotate,
                ModifierKind::React(_) => Icon::React,
                ModifierKind::Ragdoll(_) => Icon::Ragdoll,
            };
            icons::paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(14.0)), icon, theme::MODIFIER_ICON);
            ui.add(Label::new(RichText::new(modifier.label()).font(theme::strong_font(11.5)).color(theme::ink())).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                remove = icon_button(ui, Icon::Close, true, false, "Remove").clicked();
                let (eye, tip) = if modifier.enabled { (Icon::Eye, "Hide Effect") } else { (Icon::EyeOff, "Show Effect") };
                if icon_button(ui, eye, true, modifier.enabled, tip).clicked() {
                    let enabled = !modifier.enabled;
                    app.touch_rig("Toggle modifier", |scene| scene.modifiers[index].enabled = enabled);
                }
            });
        });
    });
    if open {
        Frame::new().inner_margin(Margin { left: 4, right: 6, top: 4, bottom: 6 }).show(ui, |ui| {
            ui.add_enabled_ui(modifier.enabled, |ui| match modifier.kind {
                ModifierKind::Ik(ik) => ik_properties(app, ui, index, modifier.bone, ik),
                ModifierKind::Unparent(unparent) => unparent_properties(app, ui, index, modifier.bone, unparent),
                ModifierKind::LookAt(look) => look_at_properties(app, ui, modifier.bone, look),
                ModifierKind::Copy(copy) => copy_properties(app, ui, index, modifier.bone, copy),
                ModifierKind::Limit(limit) => limit_properties(app, ui, index, limit),
                ModifierKind::Inherit(_) => value_row(app, ui, modifier.bone, "Influence", 0.0..=1.0, ""),
                ModifierKind::React(react) => react_properties(app, ui, index, modifier.bone, react),
                ModifierKind::Ragdoll(ragdoll) => ragdoll_properties(app, ui, index, modifier.bone, ragdoll),
            });
            let bakes = !matches!(modifier.kind, ModifierKind::Limit(_) | ModifierKind::Inherit(_) | ModifierKind::React(_) | ModifierKind::Ragdoll(_));
            if bakes && ui.is_enabled() {
                prop_row(ui, "", |ui| {
                    if wide_button(ui, "Bake to Keys").clicked() {
                        bake = true;
                    }
                });
            }
        });
    }
    if bake {
        app.bake_modifier(index);
    }
    remove
}

/// Two joined buttons, for a switch between two ways of driving a bone. Returns which
/// one was clicked, when it isn't the one already set.
fn switch_row(ui: &mut Ui, on: bool, labels: [&str; 2]) -> Option<bool> {
    let mut picked = None;
    prop_row(ui, "Mode", |ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        let width = (ui.available_width() - 1.0) / 2.0;
        for (i, (label, state)) in labels.into_iter().zip([true, false]).enumerate() {
            let corners = chrome::joined_corners(i, 2);
            let button = ui.scope(|ui| {
                chrome::round_widgets(ui, corners);
                let text = RichText::new(label).color(theme::ink());
                ui.add(egui::Button::new(text).selected(on == state).corner_radius(corners).min_size(vec2(width, 20.0)))
            });
            if button.inner.clicked() && on != state {
                picked = Some(state);
            }
        }
    });
    picked
}

fn with_ik(scene: &mut crate::scene::Scene, index: usize, change: impl FnOnce(&mut Ik)) {
    if let ModifierKind::Ik(ik) = &mut scene.modifiers[index].kind {
        change(ik);
    }
}

fn with_unparent(scene: &mut crate::scene::Scene, index: usize, change: impl FnOnce(&mut Unparent)) {
    if let ModifierKind::Unparent(unparent) = &mut scene.modifiers[index].kind {
        change(unparent);
    }
}

fn with_copy(scene: &mut crate::scene::Scene, index: usize, change: impl FnOnce(&mut CopyBone)) {
    if let ModifierKind::Copy(copy) = &mut scene.modifiers[index].kind {
        change(copy);
    }
}

fn with_react(scene: &mut crate::scene::Scene, index: usize, change: impl FnOnce(&mut React)) {
    if let ModifierKind::React(react) = &mut scene.modifiers[index].kind {
        change(react);
    }
}

fn with_ragdoll(scene: &mut crate::scene::Scene, index: usize, change: impl FnOnce(&mut Ragdoll)) {
    if let ModifierKind::Ragdoll(ragdoll) = &mut scene.modifiers[index].kind {
        change(ragdoll);
    }
}

fn with_limit(scene: &mut crate::scene::Scene, index: usize, change: impl FnOnce(&mut Limit)) {
    if let ModifierKind::Limit(limit) = &mut scene.modifiers[index].kind {
        change(limit);
    }
}

/// A drop-down of what something can move with. Returns the choice when one is made.
fn follow_combo(app: &KayeApp, ui: &mut Ui, id: BoneId, current: Option<BoneId>, choices: Vec<BoneId>, nothing: &str) -> Option<Option<BoneId>> {
    let shown = current.map_or(nothing.to_string(), |a| app.scene.bones[a].name.clone());
    let mut picked = None;
    theme::sunken(ui);
    ComboBox::from_id_salt(("follow", id)).selected_text(shown).truncate().width(ui.available_width() - 6.0).show_ui(ui, |ui| {
        if ui.selectable_label(current.is_none(), nothing).clicked() {
            picked = Some(None);
        }
        for choice in choices {
            if ui.selectable_label(current == Some(choice), app.scene.bones[choice].name.as_str()).clicked() {
                picked = Some(Some(choice));
            }
        }
    });
    picked
}

/// What an IK control moves with. Anything the chain itself moves would chase its
/// own tail, so those aren't offered.
fn control_follow_row(app: &mut KayeApp, ui: &mut Ui, control: BoneId, root: BoneId) {
    let BoneLink::Free { follow, .. } = app.scene.bones[control].link else {
        return;
    };
    prop_row(ui, "Follow", |ui| {
        if let Some(choice) = follow_combo(app, ui, control, follow, anchor_choices(app, root), "World") {
            app.edit_rig("Change follow", |scene, _| modifiers::set_follow(scene, control, choice));
        }
    });
}

fn ik_properties(app: &mut KayeApp, ui: &mut Ui, index: usize, bone: BoneId, ik: Ik) {
    let root = modifiers::chain_of(&app.scene, bone, &ik).first().copied().unwrap_or(bone);
    if let Some(on) = switch_row(ui, app.ik_is_on(bone), ["IK", "FK"]) {
        app.switch_ik(bone, on);
    }
    value_row(app, ui, ik.target, "Influence", 0.0..=1.0, "");
    prop_row(ui, "Target", |ui| {
        let name = app.scene.bones[ik.target].name.clone();
        if wide_button(ui, name).clicked() {
            app.select(Selection::Bone(ik.target));
            app.scroll_to_selection = true;
        }
    });
    control_follow_row(app, ui, ik.target, root);
    prop_row(ui, "End", |ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        let width = (ui.available_width() - 1.0) / IkEnd::ALL.len() as f32;
        for (i, end) in IkEnd::ALL.into_iter().enumerate() {
            let button = ui.scope(|ui| {
                chrome::round_widgets(ui, chrome::joined_corners(i, IkEnd::ALL.len()));
                ui.add(egui::Button::new(end.label()).selected(ik.end == end).corner_radius(chrome::joined_corners(i, IkEnd::ALL.len())).min_size(vec2(width, 20.0)))
            });
            if button.inner.clicked() && ik.end != end {
                let longest = modifiers::longest_chain(&app.scene, bone, end);
                app.touch_rig("Change IK", |scene| {
                    with_ik(scene, index, |ik| {
                        ik.end = end;
                        ik.chain = ik.chain.min(longest);
                    });
                    modifiers::snap_target(scene, index);
                });
            }
        }
    });
    prop_row(ui, "Chain", |ui| {
        let longest = modifiers::longest_chain(&app.scene, bone, ik.end);
        let mut chain = ik.chain;
        if wide_number(ui, egui::DragValue::new(&mut chain).range(1..=longest).speed(0.05).suffix(" bones")).changed() {
            app.touch_rig("Change IK", |scene| with_ik(scene, index, |ik| ik.chain = chain));
        }
    });
    if ik.end == IkEnd::Joint {
        prop_row(ui, "Rotation", |ui| {
            let mut matched = ik.match_rotation;
            if ui.checkbox(&mut matched, "Match Target").changed() {
                app.touch_rig("Change IK", |scene| with_ik(scene, index, |ik| ik.match_rotation = matched));
            }
        });
    }
    prop_row(ui, "Pole", |ui| {
        let mut used = ik.pole.is_some();
        if ui.checkbox(&mut used, "").changed() {
            app.edit_rig("Change IK pole", |scene, animation| {
                if used {
                    modifiers::add_pole(scene, index);
                } else if let Some(pole) = modifiers::remove_pole(scene, index) {
                    animation.forget_bone(pole);
                }
            });
        }
        if let Some(pole) = ik.pole {
            let name = app.scene.bones[pole].name.clone();
            if wide_button(ui, name).clicked() {
                app.select(Selection::Bone(pole));
                app.scroll_to_selection = true;
            }
        }
    });
    if let Some(pole) = ik.pole {
        control_follow_row(app, ui, pole, root);
    }
    prop_row(ui, if ik.pole.is_some() { "Pole Angle" } else { "Twist" }, |ui| {
        let mut twist = ik.twist;
        if wide_number(ui, egui::DragValue::new(&mut twist).range(-180.0..=180.0).speed(0.5).suffix("\u{b0}").max_decimals(1)).changed() {
            app.touch_rig("Change IK", |scene| with_ik(scene, index, |ik| ik.twist = twist));
        }
    });
    if !ik.is_one_bone() && ik.chain > 1 {
        prop_row(ui, "Bend Limit", |ui| {
            let mut limited = ik.limit.is_some();
            if ui.checkbox(&mut limited, "").changed() {
                app.touch_rig("Change IK", |scene| with_ik(scene, index, |ik| ik.limit = limited.then_some(150.0)));
            }
            if let Some(mut limit) = ik.limit {
                if wide_number(ui, egui::DragValue::new(&mut limit).range(10.0..=180.0).speed(0.5).suffix("\u{b0}").max_decimals(1)).changed() {
                    app.touch_rig("Change IK", |scene| with_ik(scene, index, |ik| ik.limit = Some(limit)));
                }
            }
        });
    }
    if ik.is_one_bone() {
        prop_row(ui, "Sensitivity", |ui| {
            let mut lean = ik.lean;
            theme::sunken(ui);
            if ui.add(Slider::new(&mut lean, 0.0..=modifiers::MAX_LEAN).clamping(SliderClamping::Edits).fixed_decimals(2)).changed() {
                app.touch_rig("Change IK", |scene| with_ik(scene, index, |ik| ik.lean = lean));
            }
        });
    }
    prop_row(ui, "", |ui| {
        if wide_button(ui, "Snap Target to Bone").clicked() {
            let pose_before = app.scene.bones[ik.target].pose;
            modifiers::snap_target(&mut app.scene, index);
            let after = app.scene.bones[ik.target].pose;
            app.scene.bones[ik.target].pose = pose_before;
            app.apply_poses("Snap IK target", vec![(ik.target, after)]);
        }
    });
}

fn react_properties(app: &mut KayeApp, ui: &mut Ui, index: usize, bone: BoneId, react: React) {
    value_row(app, ui, bone, "Influence", 0.0..=1.0, "");
    prop_row(ui, "Recovery", |ui| {
        let mut recovery = react.recovery;
        ui.spacing_mut().slider_width = (ui.available_width() - 56.0).max(30.0);
        theme::sunken(ui);
        if ui.add(Slider::new(&mut recovery, 0.0..=5.0).suffix(" s").clamping(SliderClamping::Edits).max_decimals(2)).changed() {
            app.touch_rig("Change React", |scene| with_react(scene, index, |r| r.recovery = recovery.max(0.0)));
        }
    });
    object_rows(app, ui, index, bone, react.objects);
}

/// The list of what a React or Ragdoll bumps into, and the button that adds to it.
fn object_rows(app: &mut KayeApp, ui: &mut Ui, index: usize, bone: BoneId, objects: [Option<RigId>; modifiers::MAX_REACT_OBJECTS]) {
    let own_rig = app.scene.bones[bone].rig;
    // Older projects can still have a rig left behind by a deleted shape.
    let is_there = |rig: RigId| !app.scene.rigs[rig].removed && !(app.scene.rigs[rig].kind.is_single() && app.scene.live_bones(rig).is_empty());
    let others: Vec<RigId> = (0..app.scene.rigs.len()).filter(|r| *r != own_rig && is_there(*r) && !objects.contains(&Some(*r))).collect();
    prop_row(ui, "Objects", |ui| {
        let has_room = objects.iter().any(Option::is_none);
        let add = egui::Button::new("Add Object").min_size(vec2(ui.available_width() - 6.0, 20.0));
        ui.add_enabled_ui(has_room && !others.is_empty(), |ui| {
            egui::containers::menu::MenuButton::from_button(add).ui(ui, |ui| {
                chrome::menu_rows(ui);
                for rig in &others {
                    if ui.button(app.scene.rigs[*rig].name.as_str()).clicked() {
                        app.edit_rig("Change objects", |scene, _| modifiers::add_object(scene, index, *rig));
                        ui.close();
                    }
                }
            });
        });
    });
    for object in objects.iter().flatten().copied() {
        prop_row(ui, "", |ui| {
            let (icon, tint) = if app.scene.rigs[object].kind.is_single() { (Icon::Part, theme::PART_ICON) } else { (Icon::Rig, theme::RIG_ICON) };
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            icons::paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(13.0)), icon, tint);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                if icon_button(ui, Icon::Close, true, false, "Remove").clicked() {
                    app.edit_rig("Change objects", |scene, _| modifiers::remove_object(scene, index, object));
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(Label::new(app.scene.rigs[object].name.as_str()).truncate());
                });
            });
        });
    }
}

fn ragdoll_properties(app: &mut KayeApp, ui: &mut Ui, index: usize, bone: BoneId, ragdoll: Ragdoll) {
    prop_row(ui, "Gravity", |ui| {
        let mut gravity = ragdoll.gravity;
        ui.spacing_mut().slider_width = (ui.available_width() - 56.0).max(30.0);
        theme::sunken(ui);
        if ui.add(Slider::new(&mut gravity, 0.0..=500.0).clamping(SliderClamping::Edits).max_decimals(1)).changed() {
            app.touch_rig("Change Ragdoll", |scene| with_ragdoll(scene, index, |r| r.gravity = gravity.max(0.0)));
        }
    });
    value_row(app, ui, bone, "Influence", 0.0..=1.0, "");
    prop_row(ui, "", |ui| {
        let mut whole = ragdoll.affects_body;
        if ui.checkbox(&mut whole, "Affects Body").changed() {
            app.touch_rig("Change Ragdoll", |scene| with_ragdoll(scene, index, |r| r.affects_body = whole));
        }
    });
    object_rows(app, ui, index, bone, ragdoll.objects);
}

/// A bone can follow anything in its rig except itself and what hangs from it.
fn anchor_choices(app: &KayeApp, bone: BoneId) -> Vec<BoneId> {
    let hangs_from_it = |mut at: BoneId| loop {
        if at == bone {
            return true;
        }
        match app.scene.bones[at].parent {
            Some(parent) => at = parent,
            None => return false,
        }
    };
    app.scene.joints(app.scene.bones[bone].rig).into_iter().filter(|b| !hangs_from_it(*b)).collect()
}

fn look_at_properties(app: &mut KayeApp, ui: &mut Ui, bone: BoneId, look: LookAt) {
    value_row(app, ui, look.target, "Influence", 0.0..=1.0, "");
    prop_row(ui, "Target", |ui| {
        let name = app.scene.bones[look.target].name.clone();
        if wide_button(ui, name).clicked() {
            app.select(Selection::Bone(look.target));
            app.scroll_to_selection = true;
        }
    });
    control_follow_row(app, ui, look.target, bone);
}

fn copy_properties(app: &mut KayeApp, ui: &mut Ui, index: usize, bone: BoneId, copy: CopyBone) {
    prop_row(ui, "Source", |ui| {
        if let Some(source) = follow_combo(app, ui, bone, copy.source, anchor_choices(app, bone), "None") {
            app.edit_rig("Change Copy Bone", |scene, _| modifiers::set_copy_source(scene, index, source));
        }
    });
    prop_row(ui, "Copy", |ui| {
        let (mut position, mut rotation) = (copy.position, copy.rotation);
        if ui.checkbox(&mut position, "Position").changed() {
            app.touch_rig("Change Copy Bone", |scene| with_copy(scene, index, |c| c.position = position));
        }
        if ui.checkbox(&mut rotation, "Rotation").changed() {
            app.touch_rig("Change Copy Bone", |scene| with_copy(scene, index, |c| c.rotation = rotation));
        }
    });
    prop_row(ui, "Amount", |ui| {
        let mut amount = copy.amount;
        ui.spacing_mut().slider_width = (ui.available_width() - 50.0).max(30.0);
        theme::sunken(ui);
        if ui.add(Slider::new(&mut amount, 0.0..=1.0).clamping(SliderClamping::Edits).max_decimals(2)).changed() {
            app.touch_rig("Change Copy Bone", |scene| with_copy(scene, index, |c| c.amount = amount));
        }
    });
}

fn limit_properties(app: &mut KayeApp, ui: &mut Ui, index: usize, limit: Limit) {
    for (axis, name) in ["Rotation X", "Rotation Y", "Rotation Z"].into_iter().enumerate() {
        prop_row(ui, name, |ui| {
            let mut used = limit.axes[axis];
            if ui.checkbox(&mut used, "").changed() {
                app.touch_rig("Change Limit", |scene| with_limit(scene, index, |l| l.axes[axis] = used));
            }
            let (mut low, mut high) = (limit.min[axis], limit.max[axis]);
            let width = ((ui.available_width() - 6.0) * 0.5).max(30.0);
            theme::sunken(ui);
            ui.add_enabled_ui(used, |ui| {
                if ui.add_sized([width, 20.0], DragValue::new(&mut low).range(-360.0..=high).speed(0.5).suffix("\u{b0}").max_decimals(1)).changed() {
                    app.touch_rig("Change Limit", |scene| with_limit(scene, index, |l| l.min[axis] = low));
                }
                if ui.add_sized([width, 20.0], DragValue::new(&mut high).range(low..=360.0).speed(0.5).suffix("\u{b0}").max_decimals(1)).changed() {
                    app.touch_rig("Change Limit", |scene| with_limit(scene, index, |l| l.max[axis] = high));
                }
            });
        });
    }
}

fn unparent_properties(app: &mut KayeApp, ui: &mut Ui, index: usize, bone: BoneId, unparent: Unparent) {
    if let Some(on) = switch_row(ui, app.unparent_is_on(bone), ["Unparent", "Parent"]) {
        app.switch_unparent(bone, on);
    }
    value_row(app, ui, bone, "Influence", 0.0..=1.0, "");
    prop_row(ui, "Release", |ui| {
        let (mut position, mut rotation) = (unparent.position, unparent.rotation);
        if ui.checkbox(&mut position, "Position").changed() {
            app.touch_rig("Change Unparent", |scene| with_unparent(scene, index, |u| u.position = position));
        }
        if ui.checkbox(&mut rotation, "Rotation").changed() {
            app.touch_rig("Change Unparent", |scene| with_unparent(scene, index, |u| u.rotation = rotation));
        }
    });
    prop_row(ui, "Follow", |ui| {
        if let Some(anchor) = follow_combo(app, ui, bone, unparent.anchor, anchor_choices(app, bone), "World") {
            app.edit_rig("Change Unparent", |scene, _| modifiers::set_anchor(scene, index, anchor));
        }
    });
    prop_row(ui, "", |ui| {
        if wide_button(ui, "Hold Here").clicked() {
            app.edit_rig("Hold bone", |scene, _| modifiers::hold_here(scene, index));
        }
    });
}
