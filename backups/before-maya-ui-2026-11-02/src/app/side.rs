//! Right-hand column: the outliner on top and properties underneath.

use egui::{Align2, CentralPanel, Color32, ComboBox, CornerRadius, FontId, Frame, Label, Margin, Panel, Rect, RichText, ScrollArea, Sense, Slider, SliderClamping, Stroke, Ui, Vec2, collapsing_header::CollapsingState, vec2};
use glam::{EulerRot, Quat, Vec3};

use super::chrome::{self, icon_button};
use super::icons::{self, Icon};
use super::{KayeApp, PropsTab, RowKey, Selection, theme};
use crate::animation::angles_near;
use crate::gizmo::AXIS_COLORS;
use crate::math::Xf;
use crate::modifiers::{self, Ik, IkEnd, Modifier, ModifierKind, Unparent};
use crate::scene::{BoneId, BoneKind, BoneLink, Drive, PartId, RigId, RigKind};

const ROW_HEIGHT: f32 = 20.0;
const LABEL_WIDTH: f32 = 74.0;
/// Widest a field of view can go before a picture falls apart.
const MAX_FOV: f32 = 120.0;

pub fn panel(app: &mut KayeApp, ui: &mut Ui) {
    Panel::right("side")
        .default_size(310.0)
        .size_range(240.0..=520.0)
        // The two sections fill themselves. Filling here too would double up on
        // transparency when an image is behind the UI.
        .frame(Frame::new())
        .show(ui, |ui| {
            Panel::top("outliner")
                .resizable(true)
                .default_size(320.0)
                .size_range(110.0..=700.0)
                .frame(Frame::new().fill(theme::panel_deep()))
                .show(ui, |ui| outliner(app, ui));
            CentralPanel::default()
                .frame(Frame::new().fill(theme::panel()))
                .show(ui, |ui| properties(app, ui));
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
            if p.name.to_lowercase().contains(&filter) {
                rows.push(Row { depth: 0, kind: RowKind::Part(id), key: None, expandable: false });
            }
        }
        return rows;
    }
    for (rig_id, rig) in app.scene.rigs.iter().enumerate() {
        // A camera is one thing, not a rig with a bone in it.
        if rig.kind == RigKind::Camera {
            if let Some(camera) = rig.bones.first().filter(|b| !app.scene.bones[**b].removed) {
                rows.push(Row { depth: 0, kind: RowKind::Bone(*camera), key: None, expandable: false });
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
            for &p in &rig.parts {
                rows.push(Row { depth: 2, kind: RowKind::Part(p), key: None, expandable: false });
            }
        }
    }
    rows
}

fn push_bone(app: &KayeApp, bone: BoneId, depth: usize, rows: &mut Vec<Row>) {
    let key = RowKey::Bone(bone);
    let children = &app.scene.bones[bone].children;
    rows.push(Row { depth, kind: RowKind::Bone(bone), key: Some(key), expandable: !children.is_empty() });
    if !app.collapsed.contains(&key) {
        for &c in children {
            push_bone(app, c, depth + 1, rows);
        }
    }
}

fn outliner(app: &mut KayeApp, ui: &mut Ui) {
    chrome::strip_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(chrome::STRIP_HEIGHT);
        ui.horizontal_centered(|ui| {
            chrome::editor_tab(ui, "Outliner", true, theme::panel_deep());
            ui.add_space(6.0);
            let search = ui.add(
                egui::TextEdit::singleline(&mut app.outliner_filter)
                    .hint_text("Search")
                    .margin(Margin { left: 22, right: 5, top: 2, bottom: 2 })
                    .desired_width(f32::INFINITY),
            );
            let icon = Rect::from_center_size(search.rect.left_center() + vec2(12.0, 0.0), Vec2::splat(13.0));
            icons::paint(ui.painter(), icon, Icon::Search, theme::text_faint());
        });
    });

    let rows = flatten(app);
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
            outliner_row(app, ui, &rows[index], index);
        }
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

fn outliner_row(app: &mut KayeApp, ui: &mut Ui, row: &Row, index: usize) {
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

    let indent = 4.0 + row.depth as f32 * 12.0;
    let chevron_rect = Rect::from_center_size(rect.left_center() + vec2(indent + 6.0, 0.0), Vec2::splat(11.0));
    let icon_rect = Rect::from_center_size(rect.left_center() + vec2(indent + 21.0, 0.0), Vec2::splat(14.0));
    let eye_rect = Rect::from_center_size(rect.right_center() - vec2(14.0, 0.0), Vec2::splat(14.0));

    let visible = match row.kind {
        RowKind::Rig(r) => Some(app.scene.rigs[r].visible),
        RowKind::Part(p) => Some(app.scene.parts[p].visible),
        RowKind::Bone(b) if app.scene.bones[b].kind == BoneKind::Camera => Some(app.scene.rigs[app.scene.bones[b].rig].visible),
        _ => None,
    };

    let painter = ui.painter();
    if is_active {
        painter.rect_filled(rect, 0.0, theme::active_row());
    } else if is_selected {
        painter.rect_filled(rect, 0.0, theme::selected_row());
    } else if response.hovered() || is_hovered {
        painter.rect_filled(rect, 0.0, theme::header());
    } else if index % 2 == 1 {
        painter.rect_filled(rect, 0.0, theme::row_alt());
    }
    // A line for each level above this row, which runs on through its siblings.
    for level in 0..row.depth {
        let x = rect.left() + 10.0 + level as f32 * 12.0;
        painter.vline(x, rect.y_range(), Stroke::new(1.0, theme::guide()));
    }
    // Parts start collapsed, so that folder's stored flag means "open".
    let open = match row.key {
        Some(key @ RowKey::Parts(_)) => app.collapsed.contains(&key),
        Some(key) => !app.collapsed.contains(&key),
        None => false,
    };
    if row.expandable {
        let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
        icons::paint(painter, chevron_rect, chevron, theme::text_faint());
    }
    icons::paint(painter, icon_rect, icon, tint);
    let text_at = icon_rect.right_center() + vec2(5.0, 0.0);
    let galley = painter.layout_no_wrap(name, FontId::proportional(12.5), theme::text());
    let text_width = galley.size().x;
    painter.galley(text_at - vec2(0.0, galley.size().y * 0.5), galley, theme::text());
    if let Some(trailing) = trailing {
        painter.text(
            text_at + vec2(text_width + 7.0, 0.0),
            Align2::LEFT_CENTER,
            trailing,
            FontId::proportional(11.0),
            theme::text_faint(),
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
    response.context_menu(|ui| row_menu(app, ui, &row.kind));
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
        } else if row.expandable && (chevron_rect.expand(4.0).contains(at) || matches!(row.kind, RowKind::PartsFolder)) {
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
        }
        RowKind::PartsFolder => {}
    }
}

fn properties(app: &mut KayeApp, ui: &mut Ui) {
    Panel::top("props_strip").exact_size(chrome::STRIP_HEIGHT).frame(chrome::strip_frame()).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            chrome::editor_tab(ui, "Properties", true, theme::panel());
        });
    });
    Panel::left("props_tabs")
        .exact_size(30.0)
        .resizable(false)
        .frame(Frame::new().fill(theme::panel_deep()).inner_margin(Margin { left: 3, right: 0, top: 4, bottom: 0 }))
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (tab, icon, tint, tip) in [
                (PropsTab::Item, Icon::Bone, theme::BONE_ICON, "Selected"),
                (PropsTab::Rig, Icon::Rig, theme::RIG_ICON, "Rig"),
                (PropsTab::Modifiers, Icon::Modifiers, theme::MODIFIER_ICON, "Modifiers"),
            ] {
                let (rect, response) = ui.allocate_exact_size(vec2(27.0, 27.0), Sense::click());
                let active = app.props_tab == tab;
                let left = CornerRadius { nw: theme::SECTION_RADIUS, sw: theme::SECTION_RADIUS, ne: 0, se: 0 };
                if active {
                    ui.painter().rect_filled(rect, left, theme::panel());
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, left, theme::header());
                }
                let color = if active || response.hovered() { tint } else { theme::text_faint() };
                icons::paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(16.0)), icon, color);
                if chrome::tooltip(response, tip).clicked() {
                    app.props_tab = tab;
                }
            }
        });
    CentralPanel::default().frame(Frame::new()).show(ui, |ui| {
        ScrollArea::vertical().auto_shrink(false).id_salt("props").show(ui, |ui| {
            Frame::new().inner_margin(Margin::symmetric(6, 6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                match app.props_tab {
                    PropsTab::Item => selected_tab(app, ui),
                    PropsTab::Rig => rig_tab(app, ui),
                    PropsTab::Modifiers => modifiers_tab(app, ui),
                }
            });
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
    title_row(ui, Icon::Folder, theme::text_dim(), "Scene", "", |_| {});
    let s = &app.scene;
    section(ui, "Statistics", |ui| {
        let rigs = s.rigs.iter().filter(|r| r.kind != RigKind::Camera).count();
        info(ui, "Rigs", &rigs.to_string());
        info(ui, "Bones", &s.bones.iter().filter(|b| b.kind.is_joint()).count().to_string());
        info(ui, "Parts", &s.parts.len().to_string());
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
        ui.add_space(4.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        icons::paint(ui.painter(), rect, icon, tint);
        ui.add_space(1.0);
        ui.add(Label::new(RichText::new(title).font(theme::strong_font(13.5))).truncate());
        ui.label(RichText::new(kind).small().color(theme::text_faint()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), buttons);
    });
    ui.add_space(5.0);
}

/// A collapsible panel, as in Blender's properties editor. Sections with the same
/// title share whether they're open, so moving between bones doesn't reset them.
fn section(ui: &mut Ui, title: &str, content: impl FnOnce(&mut Ui)) {
    let id = ui.make_persistent_id(("section", title));
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, true);
    Frame::new().fill(theme::section()).corner_radius(theme::SECTION_RADIUS).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(rect, theme::SECTION_RADIUS, theme::header());
        }
        let chevron = if state.is_open() { Icon::ChevronDown } else { Icon::ChevronRight };
        let chevron_rect = Rect::from_center_size(rect.left_center() + vec2(12.0, 0.0), Vec2::splat(11.0));
        icons::paint(ui.painter(), chevron_rect, chevron, theme::text_dim());
        ui.painter().text(rect.left_center() + vec2(23.0, 0.0), Align2::LEFT_CENTER, title, theme::strong_font(12.0), theme::text());
        if response.clicked() {
            state.toggle(ui);
        }
        state.show_body_unindented(ui, |ui| {
            Frame::new().inner_margin(Margin { left: 4, right: 6, top: 0, bottom: 7 }).show(ui, content);
        });
    });
    state.store(ui.ctx());
    ui.add_space(4.0);
}

/// One label-and-value line in the shared two-column layout.
fn info(ui: &mut Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(LABEL_WIDTH, 18.0), Sense::hover());
        ui.painter().text(
            rect.right_center(),
            Align2::RIGHT_CENTER,
            label,
            FontId::proportional(12.0),
            theme::text_dim(),
        );
        ui.add_space(4.0);
        ui.add(Label::new(value).truncate());
    });
}

/// One number of a three-part value. The three join into one block, and `tint` colors
/// them by how the value relates to the keys.
#[allow(clippy::too_many_arguments)]
fn axis_row(ui: &mut Ui, label: &str, axis: usize, value: &mut f32, speed: f64, suffix: &str, decimals: usize, tint: Option<Color32>) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        ui.spacing_mut().interact_size.y = 20.0;
        let r = theme::SECTION_RADIUS;
        chrome::round_widgets(
            ui,
            CornerRadius {
                nw: if axis == 0 { r } else { 0 },
                ne: if axis == 0 { r } else { 0 },
                sw: if axis == 2 { r } else { 0 },
                se: if axis == 2 { r } else { 0 },
            },
        );
        if let Some(tint) = tint {
            let w = &mut ui.visuals_mut().widgets;
            w.inactive.weak_bg_fill = tint;
            w.hovered.weak_bg_fill = theme::lighter(tint);
        }
        let (rect, _) = ui.allocate_exact_size(vec2(LABEL_WIDTH - 11.0, 20.0), Sense::hover());
        ui.painter().text(
            rect.right_center(),
            Align2::RIGHT_CENTER,
            label,
            FontId::proportional(12.0),
            theme::text_dim(),
        );
        let (rect, _) = ui.allocate_exact_size(vec2(11.0, 20.0), Sense::hover());
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            ["X", "Y", "Z"][axis],
            FontId::proportional(11.5),
            AXIS_COLORS[axis],
        );
        // Exactly the row's height, so the three fields meet with only a hairline between.
        ui.spacing_mut().button_padding.y = 1.0;
        let width = ui.available_width();
        ui.add_sized(
            [width, 20.0],
            egui::DragValue::new(value)
                .speed(speed)
                .suffix(suffix)
                .min_decimals(decimals)
                .max_decimals(3),
        )
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
        let tip = if keyed { app.tip(crate::settings::Action::DeleteKey) } else { app.tip(crate::settings::Action::InsertKey) };
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

    section(ui, "Transform", |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        for i in 0..3 {
            let label = if i == 0 { "Position" } else { "" };
            responses.push(axis_row(ui, label, i, &mut pos[i], 0.01, "", 2, tint));
        }
        ui.add_space(6.0);
        for i in 0..3 {
            let label = if i == 0 { "Rotation" } else { "" };
            responses.push(axis_row(ui, label, i, &mut rot[i], 0.5, "\u{b0}", 1, tint));
        }
    });

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

/// A button filling the rest of its row, as controls do in Blender's properties.
fn wide_button(ui: &mut Ui, text: impl Into<String>) -> egui::Response {
    let width = ui.available_width();
    ui.add(egui::Button::new(text.into()).truncate().min_size(vec2(width, 22.0)))
}

/// A whole number or angle filling the rest of its row.
fn wide_number<'a>(ui: &mut Ui, field: egui::DragValue<'a>) -> egui::Response {
    let width = ui.available_width();
    ui.add_sized([width, 22.0], field)
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
    section(ui, "Assets", |ui| {
        info(ui, "Mesh", asset_label(part.mesh_asset.as_deref()));
        info(ui, "Texture", asset_label(part.texture_asset.as_deref()));
    });
}

fn rig_properties(app: &mut KayeApp, ui: &mut Ui, id: RigId) {
    let bones = app.scene.joints(id);
    let rig = &app.scene.rigs[id];
    if rig.kind == RigKind::Camera {
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
        info(ui, "Parts", &rig.parts.len().to_string());
    });
    if clear {
        let poses = bones.into_iter().map(|b| (b, Xf::IDENTITY)).collect();
        app.apply_poses("Clear rig pose", poses);
    }
}

fn rig_tab(app: &mut KayeApp, ui: &mut Ui) {
    let rig = match app.selection {
        Selection::Rig(r) => Some(r),
        Selection::Bone(b) => Some(app.scene.bones[b].rig),
        Selection::Part(p) => Some(app.scene.parts[p].rig),
        Selection::None => (!app.scene.rigs.is_empty()).then_some(0),
    };
    if let Some(r) = rig {
        rig_properties(app, ui, r);
    }
}

fn modifiers_tab(app: &mut KayeApp, ui: &mut Ui) {
    let Some(bone) = app.modifier_bone() else {
        return;
    };
    let (icon, tint) = chrome::bone_icon(app.scene.bones[bone].kind);
    title_row(ui, icon, tint, &app.scene.bones[bone].name.clone(), app.scene.bones[bone].kind.label(), |_| {});
    let (has_ik, has_unparent) = (app.has_ik(bone), app.has_unparent(bone));
    let add = egui::Button::new("Add Modifier").min_size(vec2(ui.available_width(), 24.0));
    egui::containers::menu::MenuButton::from_button(add).ui(ui, |ui| add_modifier_items(app, ui, bone, has_ik, has_unparent));
    ui.add_space(5.0);
    let cards: Vec<usize> = (0..app.scene.modifiers.len()).filter(|i| app.scene.modifiers[*i].bone == bone).collect();
    // Cards are read before any can be removed, so a removal waits until they're done.
    let mut remove = None;
    for index in cards {
        if modifier_card(app, ui, index) {
            remove = Some(index);
        }
        ui.add_space(4.0);
    }
    if let Some(index) = remove {
        app.remove_modifier(index);
    }
}

/// The entries of the Add Modifier menu, shared with the bone's right-click menu.
pub fn add_modifier_items(app: &mut KayeApp, ui: &mut Ui, bone: BoneId, has_ik: bool, has_unparent: bool) {
    if ui.add_enabled(!has_ik, egui::Button::new("Inverse Kinematics")).clicked() {
        app.add_ik(bone);
        ui.close();
    }
    if ui.add_enabled(!has_unparent, egui::Button::new("Unparent")).clicked() {
        app.add_unparent(bone);
        ui.close();
    }
    ui.add_enabled(false, egui::Button::new("Mirror Pose"));
    ui.add_enabled(false, egui::Button::new("Constraints"));
}

/// Draws one modifier. Returns true when its remove button was clicked.
fn modifier_card(app: &mut KayeApp, ui: &mut Ui, index: usize) -> bool {
    let modifier = app.scene.modifiers[index];
    let is_ik = matches!(modifier.kind, ModifierKind::Ik(_));
    let key = RowKey::Modifier(modifier.bone, is_ik);
    let open = !app.collapsed.contains(&key);
    let mut remove = false;
    Frame::new().fill(theme::section()).corner_radius(theme::SECTION_RADIUS).inner_margin(Margin { left: 3, right: 4, top: 3, bottom: 6 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
            let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
            icons::paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(11.0)), chevron, theme::text_dim());
            if response.clicked() && !app.collapsed.remove(&key) {
                app.collapsed.insert(key);
            }
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
            let icon = if is_ik { Icon::Chain } else { Icon::Unparent };
            icons::paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(14.0)), icon, theme::MODIFIER_ICON);
            ui.add(Label::new(RichText::new(modifier.label()).font(theme::strong_font(12.0))).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                remove = icon_button(ui, Icon::Close, true, false, "Remove").clicked();
                let (eye, tip) = if modifier.enabled { (Icon::Eye, "Turn Off") } else { (Icon::EyeOff, "Turn On") };
                if icon_button(ui, eye, true, modifier.enabled, tip).clicked() {
                    let enabled = !modifier.enabled;
                    app.touch_rig("Toggle modifier", |scene| scene.modifiers[index].enabled = enabled);
                }
            });
        });
        if open {
            ui.add_space(3.0);
            ui.add_enabled_ui(modifier.enabled, |ui| match modifier.kind {
                ModifierKind::Ik(ik) => ik_properties(app, ui, index, modifier.bone, ik),
                ModifierKind::Unparent(unparent) => unparent_properties(app, ui, index, modifier.bone, unparent),
            });
        }
    });
    remove
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

/// A drop-down of what something can move with. Returns the choice when one is made.
fn follow_combo(app: &KayeApp, ui: &mut Ui, id: BoneId, current: Option<BoneId>, choices: Vec<BoneId>) -> Option<Option<BoneId>> {
    let shown = current.map_or("World".to_string(), |a| app.scene.bones[a].name.clone());
    let mut picked = None;
    ComboBox::from_id_salt(("follow", id)).selected_text(shown).truncate().width(ui.available_width() - 6.0).show_ui(ui, |ui| {
        if ui.selectable_label(current.is_none(), "World").clicked() {
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
        if let Some(choice) = follow_combo(app, ui, control, follow, anchor_choices(app, root)) {
            app.edit_rig("Change follow", |scene, _| modifiers::set_follow(scene, control, choice));
        }
    });
}

fn ik_properties(app: &mut KayeApp, ui: &mut Ui, index: usize, bone: BoneId, ik: Ik) {
    let root = modifiers::chain_of(&app.scene, bone, &ik).first().copied().unwrap_or(bone);
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
                ui.add(egui::Button::new(end.label()).selected(ik.end == end).corner_radius(chrome::joined_corners(i, IkEnd::ALL.len())).min_size(vec2(width, 22.0)))
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
                    animation.tracks.remove(&pole);
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
    if ik.is_one_bone() {
        prop_row(ui, "Sensitivity", |ui| {
            let mut lean = ik.lean;
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

fn unparent_properties(app: &mut KayeApp, ui: &mut Ui, index: usize, bone: BoneId, unparent: Unparent) {
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
        if let Some(anchor) = follow_combo(app, ui, bone, unparent.anchor, anchor_choices(app, bone)) {
            app.edit_rig("Change Unparent", |scene, _| modifiers::set_anchor(scene, index, anchor));
        }
    });
    prop_row(ui, "", |ui| {
        if wide_button(ui, "Hold Here").clicked() {
            app.edit_rig("Hold bone", |scene, _| modifiers::hold_here(scene, index));
        }
    });
}
