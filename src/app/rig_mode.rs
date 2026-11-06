//! The Rigging tab. The rig shows at rest, parts are picked in order and joined with Enter,
//! and the same handles, G and R move parts, pivots and attachments. It has its own shelf,
//! Workspace and Properties, and no timeline. The logic is in `crate::rigging`.

use egui::{Align, Align2, ComboBox, FontId, Frame, Layout, Margin, Rect, ScrollArea, Sense, TextEdit, Ui, UiBuilder, Vec2, pos2, vec2};
use glam::{EulerRot, Quat, Vec3};

use super::chrome::{self, shelf_button, shelf_gap};
use super::icons::{self, Icon};
use super::side::{channel_field, channel_row, info, prop_row, section, title_row};
use super::{KayeApp, Selection, ToastKind, theme};
use crate::animation::{Animation, angles_near};
use crate::gizmo::{AxisLock, Moving, OpKind, Session};
use crate::math::Xf;
use crate::modifiers;
use crate::rigging;
use crate::scene::{BoneId, BoneLink, Drive, PartId, RigId, RigState, Scene};
use crate::settings::Action;

const ROW_HEIGHT: f32 = 20.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Animation,
    Rigging,
}

#[derive(Default)]
pub struct Rigging {
    /// Parts picked, in the order they were clicked. The first is what Join hangs the
    /// others from.
    pub picks: Vec<PartId>,
    /// The rig before a drag or a typed change, for its undo step.
    before: Option<(RigState, Animation)>,
}

impl KayeApp {
    pub fn set_mode(&mut self, mode: Mode) {
        if self.mode == mode {
            return;
        }
        self.cancel_session();
        self.pause();
        self.box_select = None;
        self.mode = mode;
        // Parts, joints' pivots and attachments are picked differently in each tab.
        self.select(Selection::None);
        self.rigging.picks.clear();
        self.sync_mode();
    }

    /// Keeps the scene at rest while the Rigging tab is open, including after a project
    /// opens or a rig comes in.
    pub(super) fn sync_mode(&mut self) {
        let rigging = self.mode == Mode::Rigging;
        if rigging {
            for rig in 0..self.scene.rigs.len() {
                rigging::prepare(&mut self.scene, rig);
            }
            // Picks follow the selection, which the Workspace and other places set too.
            match self.selection {
                Selection::Part(p) if !self.rigging.picks.contains(&p) => self.rigging.picks = vec![p],
                Selection::Part(_) => {}
                _ => self.rigging.picks.clear(),
            }
            let scene = &self.scene;
            self.rigging.picks.retain(|p| !scene.part_is_removed(*p));
        }
        if self.scene.at_rest.0 != rigging {
            self.scene.at_rest.0 = rigging;
            if rigging {
                self.scene.evaluate();
            } else {
                self.animation.apply(&mut self.scene, self.frame);
            }
        }
    }

    /// A click in the Rigging tab. Ctrl or Shift adds a part to the picks or takes it out,
    /// keeping the order they were picked in.
    pub(super) fn rig_click(&mut self, picked: Selection, adding: bool) {
        match picked {
            Selection::Part(part) if adding => {
                let picks = &mut self.rigging.picks;
                if !matches!(self.selection, Selection::Part(_)) {
                    picks.clear();
                }
                match picks.iter().position(|p| *p == part) {
                    Some(at) => {
                        picks.remove(at);
                    }
                    None => picks.push(part),
                }
                self.selection = picks.last().map_or(Selection::None, |p| Selection::Part(*p));
            }
            _ if adding => {}
            other => {
                self.select(other);
                self.rigging.picks = match other {
                    Selection::Part(p) => vec![p],
                    _ => Vec::new(),
                };
            }
        }
    }

    /// Joins every picked part after the first to the first, with a Motor6D each.
    pub fn join_picked(&mut self) {
        let picks = self.rigging.picks.clone();
        let Some((&parent, children)) = picks.split_first() else {
            return;
        };
        if children.is_empty() {
            self.toast(ToastKind::Warning, "Pick two or more parts. The first one is what the others join to.");
            return;
        }
        let mut result = Ok(Vec::new());
        self.edit_rig("Join", |scene, _| result = rigging::join(scene, parent, children));
        if let Err(error) = result {
            self.toast(ToastKind::Error, error);
        }
    }

    pub fn add_attachment(&mut self) {
        let Selection::Part(part) = self.selection else {
            return;
        };
        let mut index = 0;
        self.edit_rig("Add attachment", |scene, _| index = rigging::add_attachment(scene, part));
        self.select(Selection::Attachment(index));
    }

    /// Takes a part's joint out, leaving the part where it is.
    pub fn release_part(&mut self, part: PartId) {
        self.edit_rig("Delete joint", |scene, animation| release(scene, animation, part));
    }

    /// Gives a part a new parent, or none.
    pub fn set_parent(&mut self, part: PartId, parent: Option<PartId>) {
        let mut result = Ok(Vec::new());
        self.edit_rig("Change parent", |scene, animation| match parent {
            Some(parent) => result = rigging::join(scene, parent, &[part]),
            None => release(scene, animation, part),
        });
        if let Err(error) = result {
            self.toast(ToastKind::Error, error);
        }
    }

    /// Delete in the Rigging tab: a joint, the picked parts, or an attachment. A part's own
    /// joints go with it, and parts jointed to it stay.
    pub fn rig_delete(&mut self) {
        let picks = self.rigging.picks.clone();
        match self.selection {
            Selection::Bone(bone) if matches!(self.scene.bones[bone].link, BoneLink::Joint { .. }) => {
                let mut bones: Vec<BoneId> = vec![bone];
                bones.extend(self.extra_bones.iter().copied());
                self.edit_rig("Delete joint", |scene, animation| {
                    for gone in modifiers::delete_bones(scene, &bones) {
                        animation.forget_gone(scene, gone);
                    }
                });
            }
            Selection::Part(_) => {
                self.edit_rig("Delete part", |scene, animation| {
                    for part in picks {
                        let mut gone = match rigging::joint_of(scene, part) {
                            Some(joint) => modifiers::delete_bones(scene, &[joint]),
                            None => Vec::new(),
                        };
                        gone.extend(rigging::delete_part(scene, part));
                        for bone in gone {
                            animation.forget_gone(scene, bone);
                        }
                    }
                });
                self.rigging.picks.clear();
            }
            Selection::Attachment(index) => {
                self.edit_rig("Delete attachment", |scene, _| {
                    scene.attachments[index].removed = true;
                    scene.rigs[scene.parts[scene.attachments[index].part].rig].rigged = true;
                });
            }
            Selection::None => {}
            _ => self.delete_selection(),
        }
    }

    /// What the handles, G and R move in the Rigging tab, and what else comes along.
    pub(super) fn rig_moving(&self) -> Option<(Moving, Vec<Moving>)> {
        match self.selection {
            Selection::Part(part) => Some((Moving::Part(part), self.rigging.picks.iter().map(|p| Moving::Part(*p)).collect())),
            Selection::Bone(bone) if matches!(self.scene.bones[bone].link, BoneLink::Joint { .. }) => {
                let others = self.extra_bones.iter().filter(|b| matches!(self.scene.bones[**b].link, BoneLink::Joint { .. })).map(|b| Moving::Pivot(*b)).collect();
                Some((Moving::Pivot(bone), others))
            }
            Selection::Attachment(index) => Some((Moving::Attachment(index), Vec::new())),
            _ => None,
        }
    }

    pub(super) fn start_rig_item_session(&mut self, op: OpKind, lock: Option<AxisLock>, modal: bool, mouse: egui::Pos2) {
        let Some((main, others)) = self.rig_moving() else {
            return;
        };
        self.rigging.before = Some((self.scene.rig_state(), self.animation.clone()));
        self.session = Some(Session::for_rigging(&self.scene, main, &others, op, lock, modal, mouse));
    }

    /// Ends a drag or a typed change as one undo step.
    pub(super) fn finish_rig_edit(&mut self, label: &str) {
        if let Some(before) = self.rigging.before.take() {
            let after = (self.scene.rig_state(), self.animation.clone());
            if after != before {
                self.history.push_rig(label, before, after);
            }
        }
    }

    pub(super) fn drop_rig_edit(&mut self) {
        self.rigging.before = None;
    }

    fn begin_rig_edit(&mut self) {
        if self.rigging.before.is_none() {
            self.rigging.before = Some((self.scene.rig_state(), self.animation.clone()));
        }
    }

    pub(super) fn rig_shortcuts(&mut self, ctx: &egui::Context) {
        if self.pressed(ctx, Action::JoinParts) {
            self.join_picked();
        }
        if self.pressed(ctx, Action::AddAttachment) {
            self.add_attachment();
        }
    }
}

/// Lets a part go of what holds it, keeping it where it is.
fn release(scene: &mut Scene, animation: &mut Animation, part: PartId) {
    match scene.parts[part].drive {
        Drive::Bone(bone) => {
            for gone in modifiers::delete_bones(scene, &[bone]) {
                animation.forget_gone(scene, gone);
            }
        }
        Drive::Rigid { .. } => {
            let rig = scene.parts[part].rig;
            rigging::prepare(scene, rig);
            let world = scene.parts[part].world;
            scene.parts[part].drive = Drive::Root;
            scene.parts[part].file_cframe = scene.rigs[rig].placement.inverse() * world;
            scene.rigs[rig].rigged = true;
            scene.evaluate();
        }
        Drive::Root => {}
    }
}

/// The Animation and Rigging tabs, in the middle of the menu bar.
pub fn mode_tabs(app: &mut KayeApp, ui: &mut Ui) {
    const TABS: [(Mode, &str); 2] = [(Mode::Animation, "Animation"), (Mode::Rigging, "Rigging")];
    // The same widths `editor_tab` gives them, with the gap between.
    let font = FontId::proportional(12.0);
    let width: f32 = TABS.iter().map(|(_, label)| ui.painter().layout_no_wrap(label.to_string(), font.clone(), theme::text()).size().x + 17.0).sum();
    let row = ui.max_rect();
    let middle = ui.ctx().content_rect().center().x;
    let place = Rect::from_min_size(pos2(middle - width * 0.5, row.top()), vec2(width, row.height()));
    let mut tabs = ui.new_child(UiBuilder::new().max_rect(place).layout(Layout::left_to_right(Align::Center)));
    tabs.spacing_mut().item_spacing.x = 1.0;
    for (mode, label) in TABS {
        if chrome::editor_tab(&mut tabs, label, app.mode == mode).clicked() {
            app.set_mode(mode);
        }
    }
}

/// The shelf's tools in the Rigging tab.
pub fn tools(app: &mut KayeApp, ui: &mut Ui) {
    let picks = app.rigging.picks.len();
    let joint = match app.selection {
        Selection::Part(p) => !matches!(app.scene.parts[p].drive, Drive::Root),
        Selection::Bone(b) => matches!(app.scene.bones[b].link, BoneLink::Joint { .. }),
        _ => false,
    };
    if shelf_button(ui, Icon::Chain, theme::bone_icon(), picks >= 2, false, &app.tip(Action::JoinParts)).clicked() {
        app.join_picked();
    }
    if shelf_button(ui, Icon::Unparent, theme::bone_icon(), joint, false, "Delete Joint").clicked() {
        match app.selection {
            Selection::Part(p) => app.release_part(p),
            Selection::Bone(_) => app.rig_delete(),
            _ => {}
        }
    }
    shelf_gap(ui);
    let part = matches!(app.selection, Selection::Part(_));
    if shelf_button(ui, Icon::Attachment, theme::bone_icon(), part, false, &app.tip(Action::AddAttachment)).clicked() {
        app.add_attachment();
    }
    shelf_gap(ui);
    if shelf_button(ui, Icon::Trash, theme::text(), app.selection != Selection::None, false, &app.tip(Action::Delete)).clicked() {
        app.rig_delete();
    }
}

/// One line of the Rigging tab's Workspace.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Rig(RigId),
    Part(PartId),
    Joint(BoneId),
    Attachment(usize),
}

impl Row {
    fn selection(self) -> Selection {
        match self {
            Row::Rig(r) => Selection::Rig(r),
            Row::Part(p) => Selection::Part(p),
            Row::Joint(b) => Selection::Bone(b),
            Row::Attachment(a) => Selection::Attachment(a),
        }
    }
}

/// Every rig's parts, as Studio's Explorer lists them, each with the joint that holds it and
/// its attachments under it.
fn rows(scene: &Scene) -> Vec<(usize, Row)> {
    let mut rows = Vec::new();
    for (rig_id, rig) in scene.rigs.iter().enumerate() {
        if rig.removed || rig.kind.is_single() {
            continue;
        }
        rows.push((0, Row::Rig(rig_id)));
        for &part in &rig.parts {
            if scene.part_is_removed(part) {
                continue;
            }
            rows.push((1, Row::Part(part)));
            if let Some(joint) = rigging::joint_of(scene, part) {
                rows.push((2, Row::Joint(joint)));
            }
            for (index, a) in scene.attachments.iter().enumerate() {
                if a.part == part && !a.removed {
                    rows.push((2, Row::Attachment(index)));
                }
            }
        }
    }
    rows
}

/// The Rigging tab's Workspace.
pub fn workspace(app: &mut KayeApp, ui: &mut Ui) {
    chrome::strip_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(chrome::STRIP_HEIGHT);
        ui.horizontal_centered(|ui| {
            chrome::editor_tab(ui, "Workspace", true);
        });
    });
    let rows = rows(&app.scene);
    let (ctrl, shift) = ui.input(|i| (i.modifiers.ctrl, i.modifiers.shift));
    ScrollArea::vertical().auto_shrink(false).id_salt("rig_workspace").show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
        for &(depth, row) in &rows[range] {
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
            let selected = match row {
                Row::Part(p) => app.rigging.picks.contains(&p),
                other => app.is_selected(other.selection()),
            };
            let painter = ui.painter_at(rect);
            if selected {
                painter.rect_filled(rect, 0.0, theme::selected_row());
            } else if response.hovered() {
                painter.rect_filled(rect, 0.0, theme::field_hover());
            }
            // The part the others join to.
            if let Row::Part(p) = row {
                if app.rigging.picks.len() > 1 && app.rigging.picks.first() == Some(&p) {
                    painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, theme::camera_icon());
                }
            }
            let x = rect.left() + 8.0 + depth as f32 * 16.0;
            let (icon, tint, name, kind) = match row {
                Row::Rig(r) => (Icon::Rig, theme::rig_icon(), app.scene.rigs[r].name.clone(), String::new()),
                Row::Part(p) => (Icon::Part, theme::part_icon(), app.scene.parts[p].name.clone(), String::new()),
                Row::Joint(b) => {
                    let parent = match app.scene.bones[b].link {
                        BoneLink::Joint { part0, .. } => app.scene.parts[part0].name.clone(),
                        _ => String::new(),
                    };
                    (Icon::Bone, theme::bone_icon(), app.scene.bones[b].name.clone(), format!("{}  {parent}", app.scene.bones[b].kind.label()))
                }
                Row::Attachment(a) => (Icon::Attachment, theme::bone_icon(), app.scene.attachments[a].name.clone(), String::new()),
            };
            icons::paint(&painter, Rect::from_center_size(pos2(x + 7.0, rect.center().y), Vec2::splat(13.0)), icon, tint);
            let text = if selected { theme::ink() } else { theme::text() };
            let galley = painter.layout_no_wrap(name, FontId::proportional(12.0), text);
            let width = galley.size().x;
            painter.galley(pos2(x + 19.0, rect.center().y - galley.size().y * 0.5), galley, text);
            if !kind.is_empty() {
                painter.text(pos2(x + 25.0 + width, rect.center().y), Align2::LEFT_CENTER, kind, FontId::proportional(10.5), theme::text_faint());
            }
            if response.hovered() {
                app.hover = row.selection();
                app.outliner_hovering = true;
            }
            if response.clicked() {
                app.rig_click(row.selection(), ctrl || shift);
            }
        }
    });
}

/// The Rigging tab's Properties.
pub fn properties(app: &mut KayeApp, ui: &mut Ui) {
    chrome::strip_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(chrome::STRIP_HEIGHT);
        ui.horizontal_centered(|ui| {
            chrome::editor_tab(ui, "Properties", true);
        });
    });
    ScrollArea::vertical().auto_shrink(false).id_salt("rig_properties").show(ui, |ui| {
        Frame::new().inner_margin(Margin::symmetric(4, 4)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match app.selection {
                Selection::Part(p) => part_properties(app, ui, p),
                Selection::Bone(b) if matches!(app.scene.bones[b].link, BoneLink::Joint { .. }) => joint_properties(app, ui, b),
                Selection::Attachment(a) => attachment_properties(app, ui, a),
                Selection::Rig(r) => rig_properties(app, ui, r),
                _ => {}
            }
        });
    });
}

fn part_properties(app: &mut KayeApp, ui: &mut Ui, part: PartId) {
    let p = &app.scene.parts[part];
    title_row(ui, Icon::Part, theme::part_icon(), &p.name.clone(), &p.class.clone(), |_| {});
    section(ui, "Joint", |ui| {
        parent_row(app, ui, part);
        if let Some(joint) = rigging::joint_of(&app.scene, part) {
            name_row(app, ui, Selection::Bone(joint));
            info(ui, "Type", app.scene.bones[joint].kind.label());
        } else if let Drive::Rigid { .. } = app.scene.parts[part].drive {
            info(ui, "Type", "Weld");
        }
    });
    section(ui, "Position", |ui| {
        let frame = app.scene.parts[part].world;
        frame_rows(app, ui, frame, "Move part", |scene, frame| rigging::set_part(scene, part, frame));
    });
}

fn joint_properties(app: &mut KayeApp, ui: &mut Ui, joint: BoneId) {
    let bone = &app.scene.bones[joint];
    let (icon, tint) = chrome::bone_icon(bone.kind);
    title_row(ui, icon, tint, &bone.name.clone(), bone.kind.label(), |_| {});
    let BoneLink::Joint { part1, .. } = app.scene.bones[joint].link else {
        return;
    };
    section(ui, "Joint", |ui| {
        name_row(app, ui, Selection::Bone(joint));
        parent_row(app, ui, part1);
        info(ui, "Part1", &app.scene.parts[part1].name);
    });
    section(ui, "Pivot", |ui| {
        let Some(frame) = rigging::pivot(&app.scene, joint) else {
            return;
        };
        frame_rows(app, ui, frame, "Move pivot", |scene, frame| rigging::set_pivot(scene, joint, frame));
    });
}

fn attachment_properties(app: &mut KayeApp, ui: &mut Ui, index: usize) {
    let a = &app.scene.attachments[index];
    title_row(ui, Icon::Attachment, theme::bone_icon(), &a.name.clone(), "Attachment", |_| {});
    section(ui, "Attachment", |ui| {
        name_row(app, ui, Selection::Attachment(index));
        info(ui, "Part", &app.scene.parts[app.scene.attachments[index].part].name);
    });
    // Relative to its part, as Studio's Position and Orientation are.
    section(ui, "Position", |ui| {
        let local = app.scene.attachments[index].local;
        frame_rows(app, ui, local, "Move attachment", |scene, local| {
            scene.attachments[index].local = local;
            let rig = scene.parts[scene.attachments[index].part].rig;
            scene.rigs[rig].rigged = true;
        });
    });
}

fn rig_properties(app: &mut KayeApp, ui: &mut Ui, rig: RigId) {
    let scene = &app.scene;
    let r = &scene.rigs[rig];
    title_row(ui, Icon::Rig, theme::rig_icon(), &r.name, r.kind.label(), |_| {});
    let parts = r.parts.iter().filter(|p| !scene.part_is_removed(**p)).count();
    let joints = scene.joints(rig).len();
    let attachments = scene.attachments.iter().enumerate().filter(|(i, a)| scene.parts[a.part].rig == rig && rigging::attachment_is_live(scene, *i)).count();
    section(ui, "Rig", |ui| {
        info(ui, "Parts", &parts.to_string());
        info(ui, "Joints", &joints.to_string());
        info(ui, "Attachments", &attachments.to_string());
    });
}

/// What holds a part, picked from the rig's other parts. Parts that hang from it can't be
/// picked, since that would make a loop.
fn parent_row(app: &mut KayeApp, ui: &mut Ui, part: PartId) {
    let scene = &app.scene;
    let current = rigging::holder(scene, part);
    let rig = scene.parts[part].rig;
    let choices: Vec<PartId> = scene.rigs[rig].parts.iter().copied().filter(|p| *p != part && !scene.part_is_removed(*p) && !rigging::hangs_from(scene, *p, part)).collect();
    let mut picked = None;
    prop_row(ui, "Parent", |ui| {
        theme::sunken(ui);
        let shown = current.map_or("None".to_string(), |p| scene.parts[p].name.clone());
        ComboBox::from_id_salt(("rig_parent", part)).selected_text(shown).width(ui.available_width() - 4.0).height(300.0).show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), "None").clicked() {
                picked = Some(None);
            }
            for choice in &choices {
                if ui.selectable_label(current == Some(*choice), &scene.parts[*choice].name).clicked() {
                    picked = Some(Some(*choice));
                }
            }
        });
    });
    if let Some(parent) = picked.filter(|p| *p != current) {
        app.set_parent(part, parent);
    }
}

/// A name typed in place, which becomes one undo step when the field is left.
fn name_row(app: &mut KayeApp, ui: &mut Ui, of: Selection) {
    let mut name = match of {
        Selection::Bone(b) => app.scene.bones[b].name.clone(),
        Selection::Attachment(a) => app.scene.attachments[a].name.clone(),
        _ => return,
    };
    let mut response = None;
    prop_row(ui, "Name", |ui| {
        theme::sunken(ui);
        response = Some(ui.add(TextEdit::singleline(&mut name).desired_width(ui.available_width() - 4.0)));
    });
    let Some(response) = response else {
        return;
    };
    if response.changed() {
        app.begin_rig_edit();
        match of {
            Selection::Bone(b) => {
                app.scene.bones[b].name = name;
                let rig = app.scene.bones[b].rig;
                app.scene.rigs[rig].rigged = true;
            }
            Selection::Attachment(a) => {
                app.scene.attachments[a].name = name;
                let rig = app.scene.parts[app.scene.attachments[a].part].rig;
                app.scene.rigs[rig].rigged = true;
            }
            _ => {}
        }
    }
    if response.lost_focus() {
        app.finish_rig_edit("Rename");
    }
}

/// Position and rotation fields for a frame, changed live and kept as one undo step.
fn frame_rows(app: &mut KayeApp, ui: &mut Ui, frame: Xf, label: &str, apply: impl FnOnce(&mut Scene, Xf)) {
    let mut pos = frame.pos.to_array();
    let mut rot = angles_near(frame.rot, [0.0; 3]).map(|d| if d.abs() < 0.0005 { 0.0 } else { d });
    let rot_before = rot;
    let mut responses = Vec::with_capacity(6);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        for (i, name) in ["Position X", "Position Y", "Position Z"].iter().enumerate() {
            responses.push(channel_row(ui, name, channel_field(&mut pos[i], 0.01), None));
        }
        for (i, name) in ["Rotation X", "Rotation Y", "Rotation Z"].iter().enumerate() {
            responses.push(channel_row(ui, name, channel_field(&mut rot[i], 0.5), None));
        }
    });
    let started = responses.iter().any(|r| r.drag_started() || r.gained_focus());
    let changed = responses.iter().any(|r| r.changed());
    let stopped = responses.iter().any(|r| r.drag_stopped() || r.lost_focus());
    if started || changed {
        app.begin_rig_edit();
    }
    if changed {
        let rot = if rot == rot_before { frame.rot } else { Quat::from_euler(EulerRot::XYZ, rot[0].to_radians(), rot[1].to_radians(), rot[2].to_radians()) };
        apply(&mut app.scene, Xf::new(rot, Vec3::from_array(pos)));
    }
    if stopped {
        app.finish_rig_edit(label);
    }
}
