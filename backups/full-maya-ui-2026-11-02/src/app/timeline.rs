//! Time along the bottom of the window, as in Maya: the time slider and the range
//! slider under it. Also the Dope Sheet, and the frame that hosts the Graph Editor,
//! which share the middle of the window with the 3D view.

use std::collections::{BTreeSet, HashSet};

use egui::{Align, Align2, Button, Color32, ComboBox, DragValue, FontId, Frame, Layout, Margin, Panel, PointerButton, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2};

use super::chrome::{self, colored_icon_button, icon_button};
use super::graph;
use super::icons::{self, Icon};
use super::{HandleKind, KayeApp, Selection, theme};
use crate::animation::{Animation, EaseDirection, EaseStyle, FrameNumber, HandleType, KeyId};
use crate::scene::BoneId;
use crate::settings::{Action, PanelLayout};

const TRACK_LIST_WIDTH: f32 = 170.0;
const RULER_HEIGHT: f32 = 22.0;
const TIME_SLIDER_HEIGHT: f32 = 32.0;
const RANGE_SLIDER_HEIGHT: f32 = 27.0;
/// The current frame and the transport, at the right of the time slider.
const TRANSPORT_WIDTH: f32 = 240.0;
const RANGE_GRIP: f32 = 7.0;
const MIN_VIEW_SPAN: f32 = 8.0;
pub(super) const ROW_HEIGHT: f32 = 20.0;
pub(super) const KEY_PICK: f32 = 7.0;

pub struct TimelineState {
    view_start: f32,
    view_end: f32,
    pub(super) scroll: f32,
    pub selected: HashSet<KeyId>,
    drag: Option<Drag>,
    easing_before: Option<Animation>,
    menu: MenuTarget,
    bar_grip: BarGrip,
    /// The selected bone the Dope Sheet last scrolled to.
    pub(super) shown_bone: Option<BoneId>,
    /// The pointer was over this panel last frame, so Home fits this view.
    pub hovered: bool,
    pub graph: graph::GraphState,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            view_start: -0.5,
            view_end: 120.5,
            scroll: 0.0,
            selected: HashSet::new(),
            drag: None,
            easing_before: None,
            menu: MenuTarget::Empty(0),
            bar_grip: BarGrip::Whole,
            shown_bone: None,
            hovered: false,
            graph: graph::GraphState::default(),
        }
    }
}

enum Drag {
    Scrub,
    MoveKeys {
        grab: FrameNumber,
        before: Animation,
        keys: HashSet<KeyId>,
        delta: FrameNumber,
    },
    BoxSelect {
        anchor: Pos2,
        base: HashSet<KeyId>,
    },
    /// Shift dragging along the time slider, which picks a stretch of time.
    RangeSelect {
        anchor: f32,
        base: HashSet<KeyId>,
    },
}

/// Which part of the range slider's bar is being dragged.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BarGrip {
    Start,
    End,
    Whole,
}

/// Puts a key drag back as it was. Returns true when there was one to cancel.
pub fn cancel_drag(app: &mut KayeApp) -> bool {
    if graph::cancel_drag(app) {
        return true;
    }
    let drag = app.timeline.drag.take();
    match drag {
        Some(Drag::MoveKeys { before, keys, .. }) => {
            app.animation = before;
            app.timeline.selected = keys;
            app.animation.apply(&mut app.scene, app.frame);
            true
        }
        Some(Drag::BoxSelect { base, .. } | Drag::RangeSelect { base, .. }) => {
            app.timeline.selected = base;
            true
        }
        Some(Drag::Scrub) => true,
        None => false,
    }
}

/// Fits the whole animation in the view.
pub fn fit_view(app: &mut KayeApp) {
    let span = (app.animation.end - app.animation.start).max(1) as f32;
    let margin = (span * 0.004).max(0.5);
    app.timeline.view_start = app.animation.start as f32 - margin;
    app.timeline.view_end = app.animation.end as f32 + margin;
    if app.settings.interface.layout == PanelLayout::Graph {
        graph::fit_values(app);
    }
}

pub fn time_slider(app: &mut KayeApp, ui: &mut Ui) {
    Panel::bottom("time_slider").exact_size(TIME_SLIDER_HEIGHT).frame(Frame::new().fill(theme::panel())).show(ui, |ui| {
        let full = ui.max_rect();
        app.timeline.hovered = ui.rect_contains_pointer(full);
        let split = full.right() - TRANSPORT_WIDTH;
        slider(app, ui, Rect::from_min_max(full.min, pos2(split, full.bottom())));
        let controls = Rect::from_min_max(pos2(split, full.top()), full.max).shrink2(vec2(6.0, 0.0));
        let mut bar = ui.new_child(UiBuilder::new().max_rect(controls).layout(Layout::left_to_right(Align::Center)));
        bar.spacing_mut().item_spacing.x = 1.0;
        let mut frame = app.whole_frame();
        if number_field(&mut bar, &mut frame, 58.0) {
            app.pause();
            app.set_frame(frame as f32);
        }
        bar.add_space(5.0);
        transport(app, &mut bar);
    });
}

pub fn range_slider(app: &mut KayeApp, ui: &mut Ui) {
    Panel::bottom("range_slider")
        .exact_size(RANGE_SLIDER_HEIGHT)
        .frame(Frame::new().fill(theme::panel()).inner_margin(Margin::symmetric(6, 0)))
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                let mut start = app.animation.start;
                if number_field(ui, &mut start, 52.0) {
                    app.animation.start = start.min(app.animation.end - 1);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    let prefs_tip = app.tip(Action::Preferences);
                    if icon_button(ui, Icon::Settings, true, app.prefs_open, &prefs_tip).clicked() {
                        app.prefs_open = !app.prefs_open;
                    }
                    let auto_key = app.settings.animation.auto_key;
                    if colored_icon_button(ui, Icon::SetKey, auto_key.then_some(theme::RECORD), "Auto Key").clicked() {
                        app.settings.animation.auto_key = !auto_key;
                    }
                    if icon_button(ui, Icon::Loop, true, app.animation.looping, "Loop").clicked() {
                        app.animation.looping = !app.animation.looping;
                    }
                    chrome::divider(ui, 16.0);
                    let mut fps = app.animation.fps as i32;
                    ui.scope(|ui| {
                        theme::sunken(ui);
                        if ui.add_sized([62.0, 20.0], DragValue::new(&mut fps).speed(0.25).suffix(" fps")).changed() {
                            app.animation.fps = fps.clamp(1, 240) as f32;
                        }
                    });
                    ui.add_space(3.0);
                    // Animation layers are on the way, so the control is already in its place.
                    ui.add_enabled_ui(false, |ui| {
                        theme::sunken(ui);
                        ComboBox::from_id_salt("anim_layer").selected_text("No Anim Layer").width(104.0).show_ui(ui, |_| {});
                    });
                    ui.add_space(3.0);
                    let mut end = app.animation.end;
                    if number_field(ui, &mut end, 52.0) {
                        app.animation.end = end.max(app.animation.start + 1);
                    }
                    ui.add_space(2.0);
                    range_bar(app, ui);
                });
            });
        });
}

/// The bar between the start and end fields. It shows which stretch the time slider
/// is showing: drag it to move along, and drag its ends to zoom.
fn range_bar(app: &mut KayeApp, ui: &mut Ui) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click_and_drag());
    let (first, last) = (app.animation.start as f32, app.animation.end as f32);
    let span = (last - first).max(1.0);
    let x = |frame: f32| rect.left() + ((frame - first) / span).clamp(0.0, 1.0) * rect.width();
    let (view_start, view_end) = (app.timeline.view_start, app.timeline.view_end);
    let thumb = Rect::from_x_y_ranges(x(view_start)..=x(view_end).max(x(view_start) + RANGE_GRIP * 2.0), rect.y_range());

    if response.drag_started() {
        // A drag only starts once the pointer has moved, so what counts is where it was pressed.
        let at = ui.input(|i| i.pointer.press_origin()).map_or(thumb.center().x, |p| p.x);
        app.timeline.bar_grip = if (at - thumb.left()).abs() <= RANGE_GRIP {
            BarGrip::Start
        } else if (at - thumb.right()).abs() <= RANGE_GRIP {
            BarGrip::End
        } else {
            BarGrip::Whole
        };
    }
    if response.dragged() {
        let shift = response.drag_delta().x / rect.width().max(1.0) * span;
        let view = &mut app.timeline;
        match view.bar_grip {
            BarGrip::Start => view.view_start = (view.view_start + shift).min(view.view_end - MIN_VIEW_SPAN),
            BarGrip::End => view.view_end = (view.view_end + shift).max(view.view_start + MIN_VIEW_SPAN),
            BarGrip::Whole => {
                // It stops at the ends of the animation, and can always come back in from past them.
                let lowest = (first - 0.5 - view.view_start).min(0.0);
                let highest = (last + 0.5 - view.view_end).max(0.0);
                let shift = shift.clamp(lowest, highest);
                view.view_start += shift;
                view.view_end += shift;
            }
        }
    }
    if response.double_clicked() {
        fit_view(app);
    }
    let near_end = response.hover_pos().is_some_and(|p| (p.x - thumb.left()).abs() <= RANGE_GRIP || (p.x - thumb.right()).abs() <= RANGE_GRIP);
    if near_end || (response.dragged() && app.timeline.bar_grip != BarGrip::Whole) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }

    let painter = ui.painter();
    painter.rect_filled(rect, theme::RADIUS, theme::well());
    let fill = if response.hovered() || response.dragged() { theme::field_hover() } else { theme::field() };
    painter.rect_filled(thumb, theme::RADIUS, fill);
    let font = FontId::proportional(11.0);
    painter.text(thumb.left_center() + vec2(5.0, 0.0), Align2::LEFT_CENTER, (view_start.ceil() as i32).to_string(), font.clone(), theme::text());
    if thumb.width() > 60.0 {
        painter.text(thumb.right_center() - vec2(5.0, 0.0), Align2::RIGHT_CENTER, (view_end.floor() as i32).to_string(), font, theme::text());
    }
}

/// The Dope Sheet or Graph Editor under the 3D view, when the layout has one.
pub fn editor(app: &mut KayeApp, ui: &mut Ui) {
    // Curves need the height keys don't, so each keeps its own.
    let (id, height) = match app.settings.interface.layout {
        PanelLayout::Single => return,
        PanelLayout::DopeSheet => ("dope_sheet", 200.0),
        PanelLayout::Graph => ("graph_editor", 320.0),
    };
    Panel::bottom(id)
        .resizable(true)
        .default_size(height)
        .size_range(100.0..=620.0)
        .frame(Frame::new().fill(theme::panel()))
        .show(ui, |ui| {
            header(app, ui);
            let rect = ui.available_rect_before_wrap();
            body(app, ui, rect);
            app.timeline.hovered |= ui.rect_contains_pointer(ui.max_rect());
        });
}

fn number_field(ui: &mut Ui, value: &mut i32, width: f32) -> bool {
    ui.scope(|ui| {
        theme::sunken(ui);
        ui.add_sized([width, 20.0], DragValue::new(value).speed(0.25)).changed()
    })
    .inner
}

fn header(app: &mut KayeApp, ui: &mut Ui) {
    chrome::strip_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(chrome::STRIP_HEIGHT);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            for (kind, label) in [(PanelLayout::DopeSheet, "Dope Sheet"), (PanelLayout::Graph, "Graph Editor")] {
                if chrome::editor_tab(ui, label, app.settings.interface.layout == kind).clicked() {
                    app.set_layout(kind);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, Icon::Close, true, false, "Close").clicked() {
                    app.set_layout(PanelLayout::Single);
                }
            });
        });
    });
}

fn transport(app: &mut KayeApp, ui: &mut Ui) {
    let has_keys = !app.animation.tracks.is_empty();
    if icon_button(ui, Icon::SkipStart, true, false, &app.tip(Action::JumpStart)).clicked() {
        app.pause();
        app.set_frame(app.animation.start as f32);
    }
    if icon_button(ui, Icon::PrevKey, has_keys, false, &app.tip(Action::PrevKey)).clicked() {
        app.jump_key(false);
    }
    if icon_button(ui, Icon::StepBack, true, false, &app.tip(Action::FrameBack)).clicked() {
        app.step_frame(-1);
    }
    let icon = if app.playing { Icon::Pause } else { Icon::Play };
    if icon_button(ui, icon, true, app.playing, &app.tip(Action::PlayPause)).clicked() {
        app.toggle_play();
    }
    if icon_button(ui, Icon::StepForward, true, false, &app.tip(Action::FrameForward)).clicked() {
        app.step_frame(1);
    }
    if icon_button(ui, Icon::NextKey, has_keys, false, &app.tip(Action::NextKey)).clicked() {
        app.jump_key(true);
    }
    if icon_button(ui, Icon::SkipEnd, true, false, &app.tip(Action::JumpEnd)).clicked() {
        app.pause();
        app.set_frame(app.animation.end as f32);
    }
}

/// The view's mapping between frames and screen position.
#[derive(Clone, Copy)]
pub(super) struct Layout2 {
    pub(super) side: Rect,
    pub(super) ruler: Rect,
    pub(super) area: Rect,
    pub(super) start: f32,
    pub(super) end: f32,
    pub(super) tick_spacing: f32,
    /// Set to label the ruler in seconds at this frame rate.
    pub(super) seconds_at: Option<f32>,
}

impl Layout2 {
    pub(super) fn x(&self, frame: f32) -> f32 {
        self.area.left() + (frame - self.start) / (self.end - self.start) * self.area.width()
    }

    pub(super) fn frame_at(&self, x: f32) -> f32 {
        self.start + (x - self.area.left()) / self.area.width() * (self.end - self.start)
    }
}

/// The bones the Dope Sheet lists.
fn focused_bones(app: &KayeApp) -> Vec<BoneId> {
    app.focused_rig().map_or_else(Vec::new, |r| app.scene.live_bones(r))
}

/// The bones whose keys show on the time slider: the selection when there is one.
fn slider_bones(app: &KayeApp) -> Vec<BoneId> {
    let targets = app.key_targets();
    if targets.is_empty() { focused_bones(app) } else { targets }
}

fn slider_layout(app: &KayeApp, rect: Rect) -> Layout2 {
    Layout2 {
        side: Rect::from_min_max(rect.left_top(), rect.left_bottom()),
        ruler: rect,
        area: rect,
        start: app.timeline.view_start,
        end: app.timeline.view_end,
        // Maya labels its slider more often than an editor needs.
        tick_spacing: app.settings.animation.tick_spacing * 0.6,
        seconds_at: app.settings.animation.show_seconds.then_some(app.animation.fps),
    }
}

fn slider(app: &mut KayeApp, ui: &mut Ui, rect: Rect) {
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    zoom_and_pan(app, ui, &slider_layout(app, rect), &response);
    // Navigation may have changed the view, so read it again.
    let layout = slider_layout(app, rect);
    let bones = slider_bones(app);
    slider_input(app, ui, &layout, &response, &bones);
    follow_scrub(app, ui, &layout);
    paint_slider(app, ui, &layout, &bones);
    if response.secondary_clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let hit = slider_keys_at(app, &layout, &bones, pos);
            app.timeline.menu = if hit.is_empty() {
                MenuTarget::Ruler(layout.frame_at(pos.x).round() as FrameNumber)
            } else {
                if !hit.iter().all(|k| app.timeline.selected.contains(k)) {
                    app.timeline.selected = hit.into_iter().collect();
                }
                MenuTarget::Keys
            };
        }
    }
    open_menu(app, &response);
}

/// Every key on the frame under the pointer, among the bones the slider is showing.
fn slider_keys_at(app: &KayeApp, layout: &Layout2, bones: &[BoneId], pos: Pos2) -> Vec<KeyId> {
    let closest = app
        .animation
        .key_frames(bones)
        .into_iter()
        .map(|f| (f, (layout.x(f as f32) - pos.x).abs()))
        .filter(|(_, d)| *d <= KEY_PICK)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(f, _)| f);
    let Some(frame) = closest else {
        return Vec::new();
    };
    bones.iter().filter(|b| app.animation.key_at(**b, frame).is_some()).map(|b| (*b, frame)).collect()
}

/// As in Maya, a press scrubs, Shift drags out a stretch of time, and a press on keys
/// already selected moves them.
fn slider_input(app: &mut KayeApp, ui: &mut Ui, layout: &Layout2, response: &egui::Response, bones: &[BoneId]) {
    let (pressed, released, mouse, shift) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_released(), i.pointer.latest_pos(), i.modifiers.shift));
    if pressed && app.timeline.drag.is_none() {
        if let Some(pos) = response.hover_pos() {
            let hit = slider_keys_at(app, layout, bones, pos);
            if shift {
                app.timeline.drag = Some(Drag::RangeSelect { anchor: pos.x, base: app.timeline.selected.clone() });
            } else if !hit.is_empty() && hit.iter().all(|k| app.timeline.selected.contains(k)) {
                app.pause();
                app.timeline.drag = Some(Drag::MoveKeys {
                    grab: hit[0].1,
                    before: app.animation.clone(),
                    keys: app.timeline.selected.clone(),
                    delta: 0,
                });
            } else {
                app.timeline.selected.clear();
                app.pause();
                app.timeline.drag = Some(Drag::Scrub);
            }
        }
    }
    if let Some(mouse) = mouse {
        drag_keys(app, layout, bones, mouse, released);
    }
}

fn paint_slider(app: &KayeApp, ui: &Ui, layout: &Layout2, bones: &[BoneId]) {
    let painter = ui.painter_at(layout.area);
    let area = layout.area;
    painter.rect_filled(area, 0.0, theme::trough());
    paint_range_shade(app, &painter, layout);

    let step = tick_step(layout);
    let minor = (step / 5).max(1);
    let first = (layout.start / minor as f32).floor() as i32;
    let last = (layout.end / minor as f32).ceil() as i32;
    let bottom = area.bottom();
    for i in first..=last {
        let frame = i * minor;
        let x = painter.round_to_pixel_center(layout.x(frame as f32));
        if frame % step == 0 {
            painter.vline(x, (area.top() + 3.0)..=bottom, Stroke::new(1.0, theme::text_faint().gamma_multiply(0.7)));
            let label = match layout.seconds_at {
                Some(fps) => format!("{:.2}s", frame as f32 / fps),
                None => frame.to_string(),
            };
            painter.text(pos2(x + 4.0, area.center().y + 3.0), Align2::LEFT_CENTER, label, FontId::proportional(11.0), theme::text_dim());
        } else {
            painter.vline(x, (bottom - 5.0)..=bottom, Stroke::new(1.0, theme::text_faint().gamma_multiply(0.45)));
        }
    }

    // A stretch of selected keys shows as one block, which is what gets dragged.
    let picked: BTreeSet<FrameNumber> = app.timeline.selected.iter().filter(|k| bones.contains(&k.0)).map(|k| k.1).collect();
    if let (Some(first), Some(last)) = (picked.first(), picked.last()) {
        let band = Rect::from_x_y_ranges(layout.x(*first as f32)..=layout.x((*last + 1) as f32), area.y_range());
        painter.rect_filled(band, 0.0, theme::RECORD.gamma_multiply(0.4));
    }
    if let (Some(Drag::RangeSelect { anchor, .. }), Some(mouse)) = (&app.timeline.drag, ui.input(|i| i.pointer.latest_pos())) {
        let band = Rect::from_x_y_ranges(anchor.min(mouse.x)..=anchor.max(mouse.x), area.y_range());
        painter.rect_filled(band, 0.0, theme::RECORD.gamma_multiply(0.25));
    }
    for frame in app.animation.key_frames(bones) {
        let x = painter.round_to_pixel_center(layout.x(frame as f32));
        let color = if picked.contains(&frame) { theme::accent_soft() } else { theme::RECORD };
        painter.vline(x, (area.top() + 2.0)..=bottom, Stroke::new(2.0, color));
    }

    // One frame wide, as Maya's marker is, and never too thin to find.
    let width = (area.width() / (layout.end - layout.start)).max(3.0);
    let x = layout.x(app.frame);
    if x + width >= area.left() && x <= area.right() {
        let block = Rect::from_x_y_ranges(x..=x + width, area.y_range());
        painter.rect_filled(block, 0.0, theme::accent().gamma_multiply(0.55));
        painter.vline(x, area.y_range(), Stroke::new(1.0, theme::accent()));
        let text = painter.layout_no_wrap(app.whole_frame().to_string(), theme::strong_font(11.0), theme::ink());
        painter.galley(pos2(x + width + 3.0, area.top() + 1.0), text, theme::ink());
    }
}

fn body(app: &mut KayeApp, ui: &mut Ui, rect: Rect) {
    let graph_open = app.settings.interface.layout == PanelLayout::Graph;
    let (side_width, right) = if graph_open {
        (graph::CHANNEL_WIDTH, rect.right() - graph::INSPECTOR_WIDTH)
    } else {
        (TRACK_LIST_WIDTH, rect.right())
    };
    let side = Rect::from_min_max(rect.min, pos2(rect.left() + side_width, rect.bottom()));
    let layout = Layout2 {
        side,
        ruler: Rect::from_min_max(pos2(side.right(), rect.top()), pos2(right, rect.top() + RULER_HEIGHT)),
        area: Rect::from_min_max(pos2(side.right(), rect.top() + RULER_HEIGHT), pos2(right, rect.bottom())),
        start: app.timeline.view_start,
        end: app.timeline.view_end,
        tick_spacing: app.settings.animation.tick_spacing,
        seconds_at: app.settings.animation.show_seconds.then_some(app.animation.fps),
    };
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    navigate(app, ui, &layout, &response);
    if graph_open {
        graph::navigate(app, ui, &layout, &response);
    }
    // Navigation may have changed the view, so read it again for drawing.
    let layout = Layout2 {
        start: app.timeline.view_start,
        end: app.timeline.view_end,
        ..layout
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::panel_deep());
    paint_range_shade(app, &painter, &layout);
    paint_grid(&painter, &layout);

    if graph_open {
        graph::input(app, ui, &layout, &response);
        graph::paint(app, ui, &layout);
    } else {
        let bones = focused_bones(app);
        reveal_selected_track(app, &layout, &bones);
        key_input(app, ui, &layout, &response, &bones);
        paint_tracks(app, ui, &layout, &bones);
    }
    paint_ruler(&painter, &layout);
    paint_playhead(app, &painter, &layout);
    if graph_open {
        graph::side_panels(app, ui, rect, &layout);
    }
    context_menus(app, &layout, &response);
    if let Some(Drag::BoxSelect { anchor, .. }) = &app.timeline.drag {
        if let Some(mouse) = ui.input(|i| i.pointer.latest_pos()) {
            let area = Rect::from_two_pos(*anchor, mouse).intersect(layout.area);
            painter.rect_filled(area, 0.0, theme::accent().gamma_multiply(0.14));
            painter.rect_stroke(area, 0.0, Stroke::new(1.0, theme::accent()), StrokeKind::Inside);
        }
    }
}

/// Brings the selected bone's track into view, once each time the selection changes.
fn reveal_selected_track(app: &mut KayeApp, layout: &Layout2, bones: &[BoneId]) {
    let selected = app.selected_bone();
    if selected == app.timeline.shown_bone {
        return;
    }
    app.timeline.shown_bone = selected;
    let Some(row) = selected.and_then(|bone| bones.iter().position(|b| *b == bone)) else {
        return;
    };
    // Row 0 is the summary.
    let top = (row + 1) as f32 * ROW_HEIGHT;
    let height = layout.area.height();
    if top < app.timeline.scroll || top + ROW_HEIGHT > app.timeline.scroll + height {
        app.timeline.scroll = (top + ROW_HEIGHT * 0.5 - height * 0.5).max(0.0);
    }
}

/// Zooming and panning, shared by the slider and both editors.
fn zoom_and_pan(app: &mut KayeApp, ui: &mut Ui, layout: &Layout2, response: &egui::Response) {
    let (scroll, delta) = ui.input(|i| (i.smooth_scroll_delta.y + i.smooth_scroll_delta.x, i.pointer.delta()));
    let span = app.timeline.view_end - app.timeline.view_start;
    if let Some(pos) = response.hover_pos() {
        if scroll != 0.0 && pos.x <= layout.ruler.right() {
            if layout.side.contains(pos) {
                app.timeline.scroll = (app.timeline.scroll - scroll).max(0.0);
            } else {
                let at = layout.frame_at(pos.x);
                let factor = (-scroll * 0.003).exp();
                let new_span = span * factor;
                if (MIN_VIEW_SPAN..=100_000.0).contains(&new_span) {
                    app.timeline.view_start = at + (app.timeline.view_start - at) * factor;
                    app.timeline.view_end = at + (app.timeline.view_end - at) * factor;
                }
            }
        }
    }
    if response.dragged_by(PointerButton::Middle) {
        let shift = -delta.x * span / layout.area.width().max(1.0);
        app.timeline.view_start += shift;
        app.timeline.view_end += shift;
    }
}

/// Moves the current frame with the pointer while a scrub is going.
fn follow_scrub(app: &mut KayeApp, ui: &mut Ui, layout: &Layout2) {
    if !matches!(app.timeline.drag, Some(Drag::Scrub)) {
        return;
    }
    let (released, mouse) = ui.input(|i| (i.pointer.primary_released(), i.pointer.latest_pos()));
    if let Some(mouse) = mouse {
        let frame = layout.frame_at(mouse.x).round();
        if frame != app.frame {
            app.set_frame(frame);
        }
    }
    if released {
        app.timeline.drag = None;
    }
}

/// Scrubbing from the ruler, zooming and panning, shared by both editors.
fn navigate(app: &mut KayeApp, ui: &mut Ui, layout: &Layout2, response: &egui::Response) {
    zoom_and_pan(app, ui, layout, response);
    if ui.input(|i| i.pointer.primary_pressed()) && response.hover_pos().is_some_and(|pos| layout.ruler.contains(pos)) {
        app.pause();
        app.timeline.drag = Some(Drag::Scrub);
    }
    follow_scrub(app, ui, layout);
}

/// Frames between ruler ticks, so labels stay readable at any zoom.
fn tick_step(layout: &Layout2) -> i32 {
    let per_frame = layout.area.width() / (layout.end - layout.start);
    [1, 2, 5, 10, 20, 30, 60, 120, 300, 600, 1200, 3000, 6000, 12000]
        .into_iter()
        .find(|s| *s as f32 * per_frame >= layout.tick_spacing)
        .unwrap_or(30000)
}

fn visible_ticks(layout: &Layout2) -> impl Iterator<Item = i32> {
    let step = tick_step(layout);
    let first = (layout.start / step as f32).floor() as i32;
    let last = (layout.end / step as f32).ceil() as i32;
    (first..=last).map(move |i| i * step)
}

fn paint_range_shade(app: &KayeApp, painter: &egui::Painter, layout: &Layout2) {
    let shade = Color32::from_black_alpha(70);
    let left = layout.x(app.animation.start as f32).clamp(layout.area.left(), layout.area.right());
    let right = layout.x(app.animation.end as f32).clamp(layout.area.left(), layout.area.right());
    let top = layout.area.top();
    let bottom = layout.area.bottom();
    painter.rect_filled(Rect::from_min_max(pos2(layout.area.left(), top), pos2(left, bottom)), 0.0, shade);
    painter.rect_filled(Rect::from_min_max(pos2(right, top), pos2(layout.area.right(), bottom)), 0.0, shade);
}

fn paint_grid(painter: &egui::Painter, layout: &Layout2) {
    for frame in visible_ticks(layout) {
        let x = layout.x(frame as f32);
        if x >= layout.area.left() {
            painter.vline(x, layout.area.y_range(), Stroke::new(1.0, Color32::from_white_alpha(9)));
        }
    }
}

fn paint_ruler(painter: &egui::Painter, layout: &Layout2) {
    painter.rect_filled(layout.ruler, 0.0, theme::panel());
    let step = tick_step(layout);
    let bottom = layout.ruler.bottom();
    let minor = (step / 5).max(1);
    let first = (layout.start / minor as f32).floor() as i32;
    let last = (layout.end / minor as f32).ceil() as i32;
    for i in first..=last {
        let frame = i * minor;
        let x = layout.x(frame as f32);
        if x < layout.ruler.left() {
            continue;
        }
        let major = frame % step == 0;
        painter.vline(x, (bottom - if major { 6.0 } else { 3.0 })..=bottom, Stroke::new(1.0, theme::text_faint()));
        if major {
            painter.text(
                pos2(x, layout.ruler.top() + 2.0),
                Align2::CENTER_TOP,
                match layout.seconds_at {
                    Some(fps) => format!("{:.2}s", frame as f32 / fps),
                    None => frame.to_string(),
                },
                FontId::proportional(11.0),
                theme::text_dim(),
            );
        }
    }
    painter.hline(layout.ruler.x_range(), bottom, Stroke::new(1.0, theme::border()));
    painter.line_segment(
        [layout.side.right_top(), layout.side.right_bottom()],
        Stroke::new(1.0, theme::border()),
    );
}

fn paint_playhead(app: &KayeApp, painter: &egui::Painter, layout: &Layout2) {
    let x = layout.x(app.frame);
    if x < layout.ruler.left() || x > layout.ruler.right() {
        return;
    }
    painter.vline(x, layout.ruler.top()..=layout.area.bottom(), Stroke::new(1.5, theme::accent()));
    let text = app.whole_frame().to_string();
    let galley = painter.layout_no_wrap(text, theme::strong_font(11.0), theme::on_accent());
    let flag = Rect::from_center_size(
        pos2(x, layout.ruler.top() + 9.0),
        vec2((galley.size().x + 10.0).max(18.0), 16.0),
    );
    painter.rect_filled(flag, theme::RADIUS, theme::accent());
    let on_accent = theme::on_accent();
    painter.galley(flag.center() - galley.size() * 0.5, galley, on_accent);
}

/// The vertical center of a row, where row 0 is the summary.
fn row_y(app: &KayeApp, layout: &Layout2, row: usize) -> f32 {
    layout.area.top() + (row as f32 + 0.5) * ROW_HEIGHT - app.timeline.scroll
}

fn hit_row(app: &KayeApp, layout: &Layout2, y: f32, rows: usize) -> Option<usize> {
    let row = ((y - layout.area.top() + app.timeline.scroll) / ROW_HEIGHT).floor();
    (row >= 0.0 && (row as usize) < rows).then_some(row as usize)
}

/// The keys a click at this position would pick. The summary row stands for every
/// key on its frame.
fn keys_at(app: &KayeApp, layout: &Layout2, bones: &[BoneId], pos: Pos2) -> Vec<KeyId> {
    let Some(row) = hit_row(app, layout, pos.y, bones.len() + 1) else {
        return Vec::new();
    };
    let closest = |frames: &mut dyn Iterator<Item = FrameNumber>| {
        frames
            .map(|f| (f, (layout.x(f as f32) - pos.x).abs()))
            .filter(|(_, d)| *d <= KEY_PICK)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(f, _)| f)
    };
    if row == 0 {
        let frames = app.animation.key_frames(bones);
        let Some(frame) = closest(&mut frames.into_iter()) else {
            return Vec::new();
        };
        return bones
            .iter()
            .filter(|b| app.animation.key_at(**b, frame).is_some())
            .map(|b| (*b, frame))
            .collect();
    }
    let bone = bones[row - 1];
    let Some(track) = app.animation.tracks.get(&bone) else {
        return Vec::new();
    };
    closest(&mut track.keys.iter().map(|k| k.frame))
        .map(|f| vec![(bone, f)])
        .unwrap_or_default()
}

fn key_input(app: &mut KayeApp, ui: &mut Ui, layout: &Layout2, response: &egui::Response, bones: &[BoneId]) {
    let (pressed, released, mouse, shift) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_released(),
            i.pointer.latest_pos(),
            i.modifiers.shift,
        )
    });

    if pressed && app.timeline.drag.is_none() {
        if let Some(pos) = response.hover_pos() {
            if layout.area.contains(pos) {
                let hit = keys_at(app, layout, bones, pos);
                if hit.is_empty() {
                    let base = if shift { app.timeline.selected.clone() } else { HashSet::new() };
                    app.timeline.selected = base.clone();
                    app.timeline.drag = Some(Drag::BoxSelect { anchor: pos, base });
                } else {
                    let all_selected = hit.iter().all(|k| app.timeline.selected.contains(k));
                    if shift && all_selected {
                        for k in &hit {
                            app.timeline.selected.remove(k);
                        }
                    } else {
                        if !shift && !all_selected {
                            app.timeline.selected.clear();
                        }
                        app.timeline.selected.extend(hit.iter().copied());
                        app.pause();
                        app.timeline.drag = Some(Drag::MoveKeys {
                            grab: hit[0].1,
                            before: app.animation.clone(),
                            keys: app.timeline.selected.clone(),
                            delta: 0,
                        });
                    }
                }
            } else if layout.side.contains(pos) {
                if let Some(row) = hit_row(app, layout, pos.y, bones.len() + 1) {
                    if row == 0 {
                        if let Some(b) = bones.first() {
                            app.select(Selection::Rig(app.scene.bones[*b].rig));
                        }
                    } else {
                        app.select(Selection::Bone(bones[row - 1]));
                        app.scroll_to_selection = true;
                    }
                }
            }
        }
    }

    if let Some(mouse) = mouse {
        drag_keys(app, layout, bones, mouse, released);
    }
}

/// Carries on whichever key drag is going.
fn drag_keys(app: &mut KayeApp, layout: &Layout2, bones: &[BoneId], mouse: Pos2, released: bool) {
    match &mut app.timeline.drag {
        Some(Drag::MoveKeys { grab, before, keys, delta }) => {
            let earliest = keys.iter().map(|k| k.1).min().unwrap_or(0);
            // Keys stop at frame zero instead of running into negative time.
            let lowest = if app.settings.animation.allow_negative_frames { FrameNumber::MIN } else { -earliest };
            let wanted = (layout.frame_at(mouse.x).round() as FrameNumber - *grab).max(lowest);
            if wanted != *delta {
                *delta = wanted;
                let moved = before.moved(keys, wanted);
                let selected = keys.iter().map(|(b, f)| (*b, f + wanted)).collect();
                app.animation = moved;
                app.timeline.selected = selected;
                app.animation.apply(&mut app.scene, app.frame);
            }
            if released {
                if let Some(Drag::MoveKeys { before, delta, .. }) = app.timeline.drag.take() {
                    if delta != 0 {
                        app.history.push_edit("Move keys", Vec::new(), before, app.animation.clone());
                    }
                }
            }
        }
        Some(Drag::BoxSelect { anchor, base }) => {
            let area = Rect::from_two_pos(*anchor, mouse);
            let mut selected = base.clone();
            for (row, bone) in bones.iter().enumerate() {
                let y = row_y(app, layout, row + 1);
                if y < area.top() || y > area.bottom() {
                    continue;
                }
                if let Some(track) = app.animation.tracks.get(bone) {
                    for key in &track.keys {
                        let x = layout.x(key.frame as f32);
                        if x >= area.left() && x <= area.right() {
                            selected.insert((*bone, key.frame));
                        }
                    }
                }
            }
            app.timeline.selected = selected;
            if released {
                app.timeline.drag = None;
            }
        }
        Some(Drag::RangeSelect { anchor, base }) => {
            let (low, high) = (anchor.min(mouse.x), anchor.max(mouse.x));
            let mut selected = base.clone();
            for bone in bones {
                if let Some(track) = app.animation.tracks.get(bone) {
                    let inside = track.keys.iter().filter(|k| (low..=high).contains(&layout.x(k.frame as f32)));
                    selected.extend(inside.map(|k| (*bone, k.frame)));
                }
            }
            app.timeline.selected = selected;
            if released {
                app.timeline.drag = None;
            }
        }
        _ => {}
    }
}

pub(super) fn key_marker(painter: &egui::Painter, center: Pos2, radius: f32, square: bool, fill: Color32) {
    let outline = Stroke::new(1.0, theme::window());
    if square {
        let rect = Rect::from_center_size(center, Vec2::splat(radius * 1.6));
        painter.rect_filled(rect, 0.0, fill);
        painter.rect_stroke(rect, 0.0, outline, StrokeKind::Outside);
    } else {
        let points = vec![
            center - vec2(0.0, radius),
            center + vec2(radius, 0.0),
            center + vec2(0.0, radius),
            center - vec2(radius, 0.0),
        ];
        painter.add(Shape::convex_polygon(points, fill, outline));
    }
}

/// A key in the Dope Sheet: an upright tick, as in Maya, and a wider block for a held
/// pose.
fn key_tick(painter: &egui::Painter, center: Pos2, size: f32, wide: bool, fill: Color32) {
    let rect = Rect::from_center_size(center, vec2(if wide { size * 1.5 } else { size * 0.75 }, size * 2.2));
    painter.rect_filled(rect, 0.0, fill);
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, theme::window()), StrokeKind::Outside);
}

fn paint_tracks(app: &KayeApp, ui: &Ui, layout: &Layout2, bones: &[BoneId]) {
    let list = Rect::from_min_max(pos2(layout.side.left(), layout.area.top()), layout.side.max);
    ui.painter().rect_filled(layout.side, 0.0, theme::panel());
    let list_painter = ui.painter_at(list);
    let key_painter = ui.painter_at(layout.area);
    let full = list.union(layout.area);
    let row_painter = ui.painter_at(full);

    // The summary row sits first and collects every key in the rig.
    let center = row_y(app, layout, 0);
    let row_rect = |y: f32| Rect::from_center_size(pos2(full.center().x, y), vec2(full.width(), ROW_HEIGHT));
    row_painter.rect_filled(row_rect(center), 0.0, theme::header());
    list_painter.text(pos2(list.left() + 8.0, center), Align2::LEFT_CENTER, "Dope Sheet Summary", theme::strong_font(11.0), theme::ink());
    for frame in app.animation.key_frames(bones) {
        let on_frame: Vec<KeyId> = bones
            .iter()
            .filter(|b| app.animation.key_at(**b, frame).is_some())
            .map(|b| (*b, frame))
            .collect();
        let selected = on_frame.iter().all(|k| app.timeline.selected.contains(k));
        let fill = if selected { theme::accent() } else { theme::text_dim() };
        key_tick(&key_painter, pos2(layout.x(frame as f32), center), app.settings.animation.key_size - 1.0, false, fill);
    }

    for (i, &bone) in bones.iter().enumerate() {
        let y = row_y(app, layout, i + 1);
        if y + ROW_HEIGHT < full.top() || y - ROW_HEIGHT > full.bottom() {
            continue;
        }
        let selected = app.is_bone_selected(bone);
        if selected {
            row_painter.rect_filled(row_rect(y), 0.0, theme::selected_row());
        } else if i % 2 == 0 {
            row_painter.rect_filled(row_rect(y), 0.0, Color32::from_white_alpha(4));
        }
        let (icon, tint) = chrome::bone_icon(app.scene.bones[bone].kind);
        icons::paint(&list_painter, Rect::from_center_size(pos2(list.left() + 15.0, y), Vec2::splat(13.0)), icon, tint);
        list_painter.text(
            pos2(list.left() + 26.0, y),
            Align2::LEFT_CENTER,
            &app.scene.bones[bone].name,
            FontId::proportional(12.0),
            if selected { theme::ink() } else { theme::text_dim() },
        );
        let Some(track) = app.animation.tracks.get(&bone) else {
            continue;
        };
        for pair in track.keys.windows(2) {
            let (a, b) = (layout.x(pair[0].frame as f32), layout.x(pair[1].frame as f32));
            // A solid bar means the pose holds still between these keys.
            let still = pair[0].pose.approx_eq(pair[1].pose) || pair[0].easing.style == EaseStyle::Constant;
            let stroke = if still {
                Stroke::new(3.0, Color32::from_white_alpha(28))
            } else {
                Stroke::new(1.0, Color32::from_white_alpha(28))
            };
            key_painter.hline(a..=b, y, stroke);
        }
        for key in &track.keys {
            let is_selected = app.timeline.selected.contains(&(bone, key.frame));
            let fill = if is_selected { theme::accent() } else { theme::ink() };
            let held = key.easing.style == EaseStyle::Constant;
            key_tick(&key_painter, pos2(layout.x(key.frame as f32), y), app.settings.animation.key_size, held, fill);
        }
    }
}

/// Shows and edits how a span eases, inside `area`.
pub(super) fn easing_editor(app: &mut KayeApp, ui: &mut Ui, area: Rect) {
    let targets = app.easing_targets();
    let Some(easing) = targets.first().and_then(|(b, f)| app.animation.key_at(*b, *f)).map(|k| k.easing) else {
        return;
    };
    // Square plot with room above and below, since Back and Elastic overshoot.
    let size = (area.height() - 14.0).min(area.width() - 20.0).max(20.0);
    let plot = Rect::from_center_size(area.center(), Vec2::splat(size));
    let to_screen = |x: f32, y: f32| {
        pos2(
            plot.left() + x * plot.width(),
            plot.bottom() - (y + 0.25) / 1.5 * plot.height(),
        )
    };
    let from_screen = |p: Pos2| {
        (
            ((p.x - plot.left()) / plot.width()).clamp(0.0, 1.0),
            ((plot.bottom() - p.y) / plot.height() * 1.5 - 0.25).clamp(-0.25, 1.25),
        )
    };
    let painter = ui.painter_at(area);
    painter.rect_filled(plot, 2.0, theme::panel_deep());
    let (zero, one) = (to_screen(0.0, 0.0), to_screen(1.0, 1.0));
    let faint = Stroke::new(1.0, theme::field());
    painter.hline(plot.x_range(), zero.y, faint);
    painter.hline(plot.x_range(), one.y, faint);
    painter.vline(zero.x, plot.y_range(), faint);
    painter.vline(one.x, plot.y_range(), faint);
    painter.line_segment([zero, one], Stroke::new(1.0, Color32::from_white_alpha(14)));

    if easing.style == EaseStyle::Custom {
        let [x1, y1, x2, y2] = easing.handles;
        let (p1, p2) = (to_screen(x1, y1), to_screen(x2, y2));
        let arm = Stroke::new(1.0, theme::text_faint());
        painter.line_segment([zero, p1], arm);
        painter.line_segment([one, p2], arm);
    }
    let curve: Vec<Pos2> = (0..=80)
        .map(|i| {
            let t = i as f32 / 80.0;
            to_screen(t, easing.apply(t).clamp(-0.25, 1.25))
        })
        .collect();
    painter.add(Shape::line(curve, Stroke::new(1.8, theme::accent())));

    if easing.style != EaseStyle::Custom {
        return;
    }
    let [x1, y1, x2, y2] = easing.handles;
    let mut handles = [(x1, y1), (x2, y2)];
    let mut changed = false;
    let mut started = false;
    let mut stopped = false;
    for (i, handle) in handles.iter_mut().enumerate() {
        let center = to_screen(handle.0, handle.1);
        let hit = Rect::from_center_size(center, Vec2::splat(16.0));
        let response = ui.interact(hit, ui.scope_id().with(("bezier_handle", i)), Sense::drag());
        started |= response.drag_started();
        stopped |= response.drag_stopped();
        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                *handle = from_screen(pos);
                changed = true;
            }
        }
        let fill = if response.hovered() || response.dragged() { theme::accent_soft() } else { theme::ink() };
        painter.circle(center, 4.0, fill, Stroke::new(1.0, theme::window()));
    }
    if started && app.timeline.easing_before.is_none() {
        app.timeline.easing_before = Some(app.animation.clone());
    }
    if changed {
        let handles = [handles[0].0, handles[0].1, handles[1].0, handles[1].1];
        for (bone, frame) in &targets {
            if let Some(track) = app.animation.tracks.get_mut(bone) {
                if let Some(key) = track.keys.iter_mut().find(|k| k.frame == *frame) {
                    key.easing.handles = handles;
                }
            }
        }
        app.animation.apply(&mut app.scene, app.frame);
    }
    if stopped {
        if let Some(before) = app.timeline.easing_before.take() {
            if before != app.animation {
                app.history.push_edit("Edit easing", Vec::new(), before, app.animation.clone());
            }
        }
    }
}

/// What a right click landed on, remembered while its menu stays open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuTarget {
    Keys,
    Empty(FrameNumber),
    Ruler(FrameNumber),
}

fn context_menus(app: &mut KayeApp, layout: &Layout2, response: &egui::Response) {
    if response.secondary_clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let frame = layout.frame_at(pos.x).round() as FrameNumber;
            app.timeline.menu = if layout.ruler.contains(pos) {
                MenuTarget::Ruler(frame)
            } else {
                let hit = if app.settings.interface.layout == PanelLayout::Graph {
                    graph::keys_at(app, layout, pos)
                } else {
                    keys_at(app, layout, &focused_bones(app), pos)
                };
                if hit.is_empty() {
                    MenuTarget::Empty(frame)
                } else {
                    // Right clicking an unselected key means that key, as elsewhere.
                    if !hit.iter().all(|k| app.timeline.selected.contains(k)) {
                        app.timeline.selected = hit.into_iter().collect();
                    }
                    MenuTarget::Keys
                }
            };
        }
    }
    open_menu(app, response);
}

fn open_menu(app: &mut KayeApp, response: &egui::Response) {
    response.context_menu(|ui| {
        chrome::menu_rows(ui);
        match app.timeline.menu {
            MenuTarget::Keys => key_menu(app, ui),
            MenuTarget::Empty(frame) => empty_menu(app, ui, frame),
            MenuTarget::Ruler(frame) => ruler_menu(app, ui, frame),
        }
    });
}

fn pick(ui: &mut Ui, label: &str, selected: bool) -> bool {
    let clicked = ui.add(Button::new(label).selected(selected)).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn key_menu(app: &mut KayeApp, ui: &mut Ui) {
    let targets = app.easing_targets();
    let current = targets.first().and_then(|(b, f)| app.animation.key_at(*b, *f)).map(|k| k.easing).unwrap_or_default();

    chrome::menu(ui, "Interpolation", |ui| {
        for style in [EaseStyle::Bezier, EaseStyle::Linear, EaseStyle::Constant, EaseStyle::Custom] {
            if pick(ui, style.label(), current.style == style) {
                app.modify_easing(&targets, |e| e.style = style);
            }
        }
    });
    chrome::menu(ui, "Easing", |ui| {
        for style in EaseStyle::ALL.into_iter().filter(|s| s.has_direction()) {
            chrome::menu(ui, style.label(), |ui| {
                for direction in EaseDirection::ALL {
                    if pick(ui, direction.label(), current.style == style && current.direction == direction) {
                        app.modify_easing(&targets, |e| {
                            e.style = style;
                            e.direction = direction;
                        });
                    }
                }
            });
        }
    });
    chrome::menu(ui, "Handle Type", |ui| {
        for handle in HandleType::ALL {
            if pick(ui, handle.label(), current.handle_type == handle) {
                app.set_handle_kind(HandleKind::Auto(handle));
            }
        }
        ui.separator();
        if pick(ui, "Aligned", false) {
            app.set_handle_kind(HandleKind::Aligned);
        }
        if pick(ui, "Free", false) {
            app.set_handle_kind(HandleKind::Free);
        }
    });
    ui.separator();
    if ui.add(Button::new("Jump to Key")).clicked() {
        if let Some((_, frame)) = targets.first() {
            app.pause();
            app.set_frame(*frame as f32);
        }
        ui.close();
    }
    if ui.add(Button::new("Duplicate to Playhead").shortcut_text(app.key_text(Action::DuplicateKeys))).clicked() {
        app.duplicate_selected_keys();
        ui.close();
    }
    if ui.add(Button::new("Delete").shortcut_text(app.key_text(Action::DeleteSelectedKeys))).clicked() {
        app.delete_selected_keys();
        ui.close();
    }
}

fn empty_menu(app: &mut KayeApp, ui: &mut Ui, frame: FrameNumber) {
    let can_key = !app.key_targets().is_empty();
    if ui.add_enabled(can_key, Button::new("Insert Keyframe Here")).clicked() {
        app.pause();
        app.set_frame(frame as f32);
        app.insert_key();
        ui.close();
    }
    if ui.button("Select All Keys").clicked() {
        let bones = focused_bones(app);
        app.timeline.selected = bones
            .iter()
            .filter_map(|b| app.animation.tracks.get(b).map(|t| (b, t)))
            .flat_map(|(b, t)| t.keys.iter().map(|k| (*b, k.frame)))
            .collect();
        ui.close();
    }
    ui.separator();
    if ui.button("Fit View").clicked() {
        fit_view(app);
        ui.close();
    }
}

fn ruler_menu(app: &mut KayeApp, ui: &mut Ui, frame: FrameNumber) {
    if ui.button("Jump Here").clicked() {
        app.pause();
        app.set_frame(frame as f32);
        ui.close();
    }
    ui.separator();
    if ui.add_enabled(frame < app.animation.end, Button::new("Set Start Here")).clicked() {
        app.animation.start = frame;
        ui.close();
    }
    if ui.add_enabled(frame > app.animation.start, Button::new("Set End Here")).clicked() {
        app.animation.end = frame;
        ui.close();
    }
    ui.separator();
    if ui.button("Fit View").clicked() {
        fit_view(app);
        ui.close();
    }
}
