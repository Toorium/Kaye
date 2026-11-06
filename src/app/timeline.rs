//! Bottom panel: the timeline for keys, and the frame that hosts the graph tab.

use std::collections::HashSet;

use egui::{Align, Align2, Button, Color32, DragValue, FontId, Frame, Layout, Panel, PointerButton, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use super::chrome::{self, colored_icon_button, icon_button};
use super::graph;
use super::references;
use super::icons::{self, Icon};
use super::kept::{Kept, Signature};
use super::{BottomTab, HandleKind, KayeApp, Selection, theme};
use crate::animation::{Animation, EaseDirection, EaseStyle, FrameNumber, HandleType, Key, KeyId};
use crate::reference::{Edge, Reference};
use crate::scene::BoneId;
use crate::settings::Action;

const TRACK_LIST_WIDTH: f32 = 170.0;
const ANIMATION_POPUP_WIDTH: f32 = 240.0;
/// How many rows of the animation list show before it scrolls, and the name field's width.
const ANIMATION_LIST_HEIGHT: f32 = 180.0;
const ANIMATION_NAME_WIDTH: f32 = 130.0;
const HEADER_HEIGHT: f32 = 30.0;
const RULER_HEIGHT: f32 = 22.0;
pub(super) const ROW_HEIGHT: f32 = 20.0;
pub(super) const KEY_PICK: f32 = 7.0;
/// The strip at the left of a bone's row that opens its channels.
const EXPANDER_WIDTH: f32 = 15.0;
/// The eye at the right of a reference's row.
const EYE_WIDTH: f32 = 22.0;
/// How close to a strip's end a drag trims it instead of moving it.
const STRIP_EDGE: f32 = 6.0;

pub struct TimelineState {
    view_start: f32,
    view_end: f32,
    pub(super) scroll: f32,
    pub selected: HashSet<KeyId>,
    drag: Option<Drag>,
    easing_before: Option<Animation>,
    menu: MenuTarget,
    /// The selected bone the track list last scrolled to.
    shown_bone: Option<BoneId>,
    /// The pointer was over this panel last frame, so Home fits this view.
    pub hovered: bool,
    pub graph: graph::GraphState,
    /// What's typed in the animation list's search field.
    animation_search: String,
    /// Bones whose Position and Rotation rows are open under them.
    pub open_channels: HashSet<BoneId>,
    /// The channel row a right click landed on, for taking just that channel out of keys.
    menu_channel: Option<usize>,
    /// The keys as last drawn, see `kept.rs`.
    kept: Kept,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            view_start: -6.0,
            view_end: 126.0,
            scroll: 0.0,
            selected: HashSet::new(),
            drag: None,
            easing_before: None,
            menu: MenuTarget::Empty(0),
            shown_bone: None,
            hovered: false,
            graph: graph::GraphState::default(),
            animation_search: String::new(),
            open_channels: HashSet::new(),
            menu_channel: None,
            kept: Kept::default(),
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
    /// A reference strip, moved whole or by one end. `grab` is the frame the mouse took it at.
    Strip {
        id: u64,
        edge: Option<Edge>,
        grab: f64,
        before: Reference,
    },
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
        Some(Drag::BoxSelect { base, .. }) => {
            app.timeline.selected = base;
            true
        }
        Some(Drag::Strip { before, .. }) => {
            if let Some(reference) = app.scene.references.iter_mut().find(|r| r.id == before.id) {
                *reference = before;
            }
            true
        }
        Some(Drag::Scrub) => true,
        None => false,
    }
}

/// Fits the whole animation in the view.
pub fn fit_view(app: &mut KayeApp) {
    let span = (app.animation.end - app.animation.start).max(1) as f32;
    app.timeline.view_start = app.animation.start as f32 - span * 0.05;
    app.timeline.view_end = app.animation.end as f32 + span * 0.05;
    if app.bottom_tab == BottomTab::Graph {
        graph::fit_values(app);
    }
}

pub fn panel(app: &mut KayeApp, ui: &mut Ui) {
    // Curves need the height keys don't, so each tab keeps its own.
    let (id, height) = match app.bottom_tab {
        BottomTab::Timeline => ("timeline", 210.0),
        BottomTab::Graph => ("graph", 310.0),
    };
    Panel::bottom(id)
        .resizable(true)
        .default_size(height)
        .size_range(80.0..=620.0)
        .frame(Frame::new().fill(theme::panel()))
        .show(ui, |ui| {
            header(app, ui);
            let rect = ui.available_rect_before_wrap();
            body(app, ui, rect);
            app.timeline.hovered = ui.rect_contains_pointer(ui.max_rect());
        });
}

fn number_field(ui: &mut Ui, value: &mut i32, width: f32, prefix: &str) -> bool {
    ui.scope(|ui| {
        theme::sunken(ui);
        ui.add_sized([width, 22.0], DragValue::new(value).speed(0.25).prefix(prefix)).changed()
    })
    .inner
}

fn header(app: &mut KayeApp, ui: &mut Ui) {
    chrome::strip_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(HEADER_HEIGHT);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            for (kind, label) in [(BottomTab::Timeline, "Timeline"), (BottomTab::Graph, "Graph")] {
                if chrome::editor_tab(ui, label, app.bottom_tab == kind).clicked() && app.bottom_tab != kind {
                    app.bottom_tab = kind;
                    // Each tab has its own list down the side, which shouldn't inherit
                    // how far the other one was scrolled.
                    app.timeline.scroll = 0.0;
                    app.timeline.shown_bone = None;
                    if kind == BottomTab::Graph {
                        graph::opened(app);
                    }
                }
            }
            ui.add_space(10.0);
            key_controls(app, ui);
            ui.add_space(10.0);
            animation_picker(app, ui);
            // The transport sits in the middle of the editor, as in Blender, unless the
            // window is too narrow to leave room for what's on either side.
            let width = TRANSPORT_BUTTONS as f32 * 23.0;
            let middle = ui.max_rect().center().x;
            let gap = middle - width * 0.5 - ui.cursor().left();
            let room = middle + width * 0.5 + RANGE_FIELDS_WIDTH < ui.max_rect().right();
            ui.add_space(if gap > 12.0 && room { gap } else { 12.0 });
            transport(app, ui);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                ui.add_space(2.0);
                range_fields(app, ui);
            });
        });
    });
}

fn key_controls(app: &mut KayeApp, ui: &mut Ui) {
    let auto_key = app.settings.animation.auto_key;
    if colored_icon_button(ui, Icon::AutoKey, auto_key.then_some(theme::record()), "Auto Key").clicked() {
        app.settings.animation.auto_key = !auto_key;
    }
    let can_key = !app.key_targets().is_empty();
    if icon_button(ui, Icon::Key, can_key, false, &app.tip(Action::InsertKey)).clicked() {
        app.insert_key();
    }
    if icon_button(ui, Icon::Loop, true, app.animation.looping, "Loop").clicked() {
        app.animation.looping = !app.animation.looping;
        app.animation.apply(&mut app.scene, app.frame);
    }
    // Only means anything while it loops.
    let looping = app.animation.looping;
    if icon_button(ui, Icon::SmoothLoop, looping, looping && app.animation.smooth_loop, "Smooth Loop").clicked() {
        app.edit_animation("Smooth Loop", |a| a.smooth_loop = !a.smooth_loop);
    }
}

/// The rig's animations, as Blender's Action Editor has them: a list to pick from, the
/// name of the one playing, and buttons for making and removing them. It goes by the rig
/// in focus, so nothing here asks which rig.
fn animation_picker(app: &mut KayeApp, ui: &mut Ui) {
    let rig = app.focused_rig();
    let clips = rig.map_or_else(Vec::new, |rig| app.animation.clips_of(rig));
    let active = rig.and_then(|rig| app.animation.active_clip(rig));
    let (rect, button) = ui.allocate_exact_size(vec2(36.0, 22.0), Sense::click());
    let fill = if button.hovered() { theme::field_hover() } else { theme::field() };
    ui.painter().rect_filled(rect, theme::RADIUS, fill);
    icons::paint(ui.painter(), Rect::from_center_size(rect.left_center() + vec2(12.0, 0.0), Vec2::splat(14.0)), Icon::Animations, theme::ink());
    chrome::paint_triangle(ui.painter(), rect.right_center() - vec2(9.0, 0.0), true, theme::ink());
    let button = chrome::tooltip(button, "Animations");
    egui::Popup::menu(&button).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        chrome::menu_rows(ui);
        ui.set_width(ANIMATION_POPUP_WIDTH);
        let wanted = app.timeline.animation_search.to_lowercase();
        let mut picked = None;
        egui::ScrollArea::vertical().max_height(ANIMATION_LIST_HEIGHT).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for clip in &clips {
                let name = &app.animation.clips[*clip].name;
                if !wanted.is_empty() && !name.to_lowercase().contains(&wanted) {
                    continue;
                }
                let text = egui::RichText::new(name.as_str()).color(theme::ink());
                if ui.add(Button::new(text).selected(active == Some(*clip)).min_size(vec2(ui.available_width(), 22.0))).clicked() {
                    picked = Some(*clip);
                }
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            icons::paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(13.0)), Icon::Search, theme::text_dim());
            theme::sunken(ui);
            ui.add(egui::TextEdit::singleline(&mut app.timeline.animation_search).desired_width(f32::INFINITY));
        });
        if let Some(clip) = picked {
            app.switch_animation(clip);
            app.timeline.animation_search.clear();
            ui.close();
        }
    });
    let Some(active) = active else {
        ui.add_enabled_ui(false, |ui| {
            ui.add_sized([ANIMATION_NAME_WIDTH, 22.0], Button::new(""));
        });
        return;
    };
    let mut name = app.animation.clips[active].name.clone();
    let field = ui.scope(|ui| {
        theme::sunken(ui);
        ui.add_sized([ANIMATION_NAME_WIDTH, 22.0], egui::TextEdit::singleline(&mut name).margin(vec2(5.0, 3.0)))
    });
    if field.inner.changed() {
        app.touch_keys("Rename animation", |a| a.clips[active].name = name);
    }
    // Names only have to be different once typing is done, or "Walk" couldn't pass through
    // "W" while another animation has that name.
    if field.inner.lost_focus() {
        app.finish_animation_name(active);
    }
    let rig = app.animation.clips[active].rig;
    ui.add_space(2.0);
    if icon_button(ui, Icon::Plus, true, false, "New Animation").clicked() {
        app.new_animation(rig);
    }
    if icon_button(ui, Icon::Copy, true, false, "Duplicate Animation").clicked() {
        app.duplicate_animation(active);
    }
    if icon_button(ui, Icon::Close, clips.len() > 1, false, "Delete Animation").clicked() {
        app.delete_animation(active);
    }
}

const TRANSPORT_BUTTONS: usize = 7;
/// Roughly how wide the fields on the right of the header are.
const RANGE_FIELDS_WIDTH: f32 = 300.0;

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

/// Laid out right to left. Each field carries its own label, as Blender's do.
fn range_fields(app: &mut KayeApp, ui: &mut Ui) {
    let mut fps = app.animation.fps as i32;
    if number_field(ui, &mut fps, 62.0, "FPS ") {
        app.animation.fps = fps.clamp(1, 240) as f32;
    }
    ui.add_space(10.0);
    let mut end = app.animation.end;
    if number_field(ui, &mut end, 76.0, "End ") {
        app.animation.end = end.max(app.animation.start + 1);
    }
    let mut start = app.animation.start;
    if number_field(ui, &mut start, 76.0, "Start ") {
        app.animation.start = start.min(app.animation.end - 1);
    }
    ui.add_space(10.0);
    let mut frame = app.whole_frame();
    if number_field(ui, &mut frame, 70.0, "Frame ") {
        app.pause();
        app.set_frame(frame as f32);
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

/// One row of the track list.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TrackRow {
    /// Every key in the rig.
    Summary,
    Bone(BoneId),
    /// One of a bone's channels, open under it, with only the keys that set it.
    Channel(BoneId, usize),
    /// A reference's strip, by its place in `Scene::references`.
    Reference(usize),
}

/// The rows the track list shows, top to bottom: the summary, then each bone with any
/// channels open under it.
fn track_rows(app: &KayeApp, bones: &[BoneId]) -> Vec<TrackRow> {
    let mut rows = vec![TrackRow::Summary];
    rows.extend((0..app.scene.references.len()).map(TrackRow::Reference));
    for &bone in bones {
        rows.push(TrackRow::Bone(bone));
        if app.timeline.open_channels.contains(&bone) && app.animation.tracks.contains_key(&bone) {
            rows.extend((0..graph::channel_count(app, bone)).map(|c| TrackRow::Channel(bone, c)));
        }
    }
    rows
}

/// The rig whose bones the panel lists.
fn focused_bones(app: &KayeApp) -> Vec<BoneId> {
    app.focused_rig().map_or_else(Vec::new, |r| app.scene.live_bones(r))
}

/// Brings the selected bone's track into view, once each time the selection changes.
fn reveal_selected_track(app: &mut KayeApp, layout: &Layout2, bones: &[BoneId]) {
    let selected = app.selected_bone();
    if selected == app.timeline.shown_bone {
        return;
    }
    app.timeline.shown_bone = selected;
    let rows = track_rows(app, bones);
    let Some(row) = selected.and_then(|bone| rows.iter().position(|r| *r == TrackRow::Bone(bone))) else {
        return;
    };
    let top = row as f32 * ROW_HEIGHT;
    let height = layout.area.height();
    if top < app.timeline.scroll || top + ROW_HEIGHT > app.timeline.scroll + height {
        app.timeline.scroll = (top + ROW_HEIGHT * 0.5 - height * 0.5).max(0.0);
    }
}

fn body(app: &mut KayeApp, ui: &mut Ui, rect: Rect) {
    let (side_width, right) = match app.bottom_tab {
        BottomTab::Timeline => (TRACK_LIST_WIDTH, rect.right()),
        BottomTab::Graph => (graph::CHANNEL_WIDTH, rect.right() - graph::INSPECTOR_WIDTH),
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
    if app.bottom_tab == BottomTab::Graph {
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

    match app.bottom_tab {
        BottomTab::Timeline => {
            let bones = focused_bones(app);
            reveal_selected_track(app, &layout, &bones);
            key_input(app, ui, &layout, &response, &bones);
            paint_tracks(app, ui, &layout, &bones);
        }
        BottomTab::Graph => {
            graph::input(app, ui, &layout, &response);
            graph::paint(app, ui, &layout);
        }
    }
    paint_ruler(&painter, &layout);
    paint_playhead(app, &painter, &layout);
    if app.bottom_tab == BottomTab::Graph {
        graph::side_panels(app, ui, rect, &layout);
    }
    // The corner above the track list, so nothing scrolls under it.
    ui.painter().rect_filled(Rect::from_min_size(layout.side.min, vec2(layout.side.width(), RULER_HEIGHT)), 0.0, theme::panel());
    context_menus(app, &layout, &response);
    if let Some(Drag::BoxSelect { anchor, .. }) = &app.timeline.drag {
        if let Some(mouse) = ui.input(|i| i.pointer.latest_pos()) {
            let area = Rect::from_two_pos(*anchor, mouse).intersect(layout.area);
            painter.rect_filled(area, 0.0, theme::accent().gamma_multiply(0.14));
            painter.rect_stroke(area, 0.0, Stroke::new(1.0, theme::accent()), StrokeKind::Inside);
        }
    }
}

/// Scrubbing, zooming and panning, shared by both tabs.
fn navigate(app: &mut KayeApp, ui: &mut Ui, layout: &Layout2, response: &egui::Response) {
    let (pressed, released, mouse, scroll, delta) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_released(),
            i.pointer.latest_pos(),
            i.smooth_scroll_delta.y + i.smooth_scroll_delta.x,
            i.pointer.delta(),
        )
    });
    let span = app.timeline.view_end - app.timeline.view_start;
    if let Some(pos) = response.hover_pos() {
        if scroll != 0.0 && pos.x <= layout.ruler.right() {
            if layout.side.contains(pos) {
                app.timeline.scroll = (app.timeline.scroll - scroll).max(0.0);
            } else {
                let at = layout.frame_at(pos.x);
                let factor = (-scroll * 0.003).exp();
                let new_span = span * factor;
                if (8.0..=100_000.0).contains(&new_span) {
                    app.timeline.view_start = at + (app.timeline.view_start - at) * factor;
                    app.timeline.view_end = at + (app.timeline.view_end - at) * factor;
                }
            }
        }
        if pressed && layout.ruler.contains(pos) {
            app.pause();
            app.timeline.drag = Some(Drag::Scrub);
        }
    }
    if response.dragged_by(PointerButton::Middle) {
        let shift = -delta.x * span / layout.area.width().max(1.0);
        app.timeline.view_start += shift;
        app.timeline.view_end += shift;
    }
    if matches!(app.timeline.drag, Some(Drag::Scrub)) {
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
    let rows = track_rows(app, bones);
    let Some(row) = hit_row(app, layout, pos.y, rows.len()).map(|r| rows[r]) else {
        return Vec::new();
    };
    let closest = |frames: &mut dyn Iterator<Item = FrameNumber>| {
        frames
            .map(|f| (f, (layout.x(f as f32) - pos.x).abs()))
            .filter(|(_, d)| *d <= KEY_PICK)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(f, _)| f)
    };
    let (bone, channel) = match row {
        TrackRow::Summary => (None, None),
        TrackRow::Bone(bone) => (Some(bone), None),
        TrackRow::Channel(bone, channel) => (Some(bone), Some(channel)),
        TrackRow::Reference(_) => return Vec::new(),
    };
    let Some(bone) = bone else {
        let frames = app.animation.key_frames(bones);
        let Some(frame) = closest(&mut frames.into_iter()) else {
            return Vec::new();
        };
        return bones
            .iter()
            .filter(|b| app.animation.key_at(**b, frame).is_some())
            .map(|b| (*b, frame))
            .collect();
    };
    let Some(track) = app.animation.tracks.get(&bone) else {
        return Vec::new();
    };
    closest(&mut track.keys.iter().filter(|k| channel.is_none_or(|c| k.sets(c))).map(|k| k.frame))
        .map(|f| vec![(bone, f)])
        .unwrap_or_default()
}

fn key_input(app: &mut KayeApp, ui: &mut Ui, layout: &Layout2, response: &egui::Response, bones: &[BoneId]) {
    let (pressed, released, mouse, shift) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_released(),
            i.pointer.latest_pos(),
            // Ctrl adds to a selection everywhere else in Kaye, so it does here too.
            i.modifiers.shift || i.modifiers.ctrl,
        )
    });

    // A strip's ends can be dragged, so the mouse says so over them.
    if app.timeline.drag.is_none() {
        if let Some(pos) = response.hover_pos() {
            if let Some((_, edge)) = strip_at(app, layout, bones, pos) {
                ui.ctx().set_cursor_icon(if edge.is_some() { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::Grab });
            }
        }
    }

    if pressed && app.timeline.drag.is_none() {
        if let Some(pos) = response.hover_pos() {
            if let Some((index, edge)) = strip_at(app, layout, bones, pos) {
                app.pause();
                let before = app.scene.references[index].clone();
                app.timeline.drag = Some(Drag::Strip { id: before.id, edge, grab: layout.frame_at(pos.x) as f64, before });
            } else if layout.area.contains(pos) {
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
                let rows = track_rows(app, bones);
                match hit_row(app, layout, pos.y, rows.len()).map(|r| rows[r]) {
                    Some(TrackRow::Summary) => {
                        if let Some(b) = bones.first() {
                            app.select(Selection::Rig(app.scene.bones[*b].rig));
                        }
                    }
                    Some(TrackRow::Bone(bone)) if pos.x < layout.side.left() + EXPANDER_WIDTH && app.animation.tracks.contains_key(&bone) => {
                        if !app.timeline.open_channels.remove(&bone) {
                            app.timeline.open_channels.insert(bone);
                        }
                    }
                    Some(TrackRow::Bone(bone) | TrackRow::Channel(bone, _)) => {
                        app.select(Selection::Bone(bone));
                        app.scroll_to_selection = true;
                    }
                    // The eye at the right shows and hides the window, and the rest of the
                    // row brings it up.
                    Some(TrackRow::Reference(index)) => {
                        let id = app.scene.references[index].id;
                        if pos.x > layout.side.right() - EYE_WIDTH && app.references.is_open(id) {
                            app.references.hide(id);
                        } else {
                            app.references.show(id);
                        }
                    }
                    None => {}
                }
            }
        }
    }

    let Some(mouse) = mouse else {
        return;
    };
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
            // The summary row stands for every key on its frames, as a click on it does.
            let summary_y = row_y(app, layout, 0);
            let on_summary = summary_y >= area.top() && summary_y <= area.bottom();
            for (row, kind) in track_rows(app, bones).into_iter().enumerate().skip(1) {
                let y = row_y(app, layout, row);
                if !on_summary && (y < area.top() || y > area.bottom()) {
                    continue;
                }
                let (TrackRow::Bone(bone) | TrackRow::Channel(bone, _)) = kind else {
                    continue;
                };
                let channel = if let TrackRow::Channel(_, c) = kind { Some(c) } else { None };
                if let Some(track) = app.animation.tracks.get(&bone) {
                    for key in track.keys.iter().filter(|k| on_summary || channel.is_none_or(|c| k.sets(c))) {
                        let x = layout.x(key.frame as f32);
                        if x >= area.left() && x <= area.right() {
                            selected.insert((bone, key.frame));
                        }
                    }
                }
            }
            app.timeline.selected = selected;
            if released {
                app.timeline.drag = None;
            }
        }
        Some(Drag::Strip { id, edge, grab, before }) => {
            let (id, edge, grab) = (*id, *edge, *grab);
            let mut moved = before.clone();
            let rate = references::rate(app, &moved);
            // Whole frames, so a strip lines up with keys.
            let at = layout.frame_at(mouse.x) as f64;
            match edge {
                None => moved.move_by((at - grab).round()),
                Some(edge) => moved.drag_edge(edge, at.round(), rate),
            }
            if let Some(reference) = app.scene.references.iter_mut().find(|r| r.id == id) {
                *reference = moved;
            }
            if released {
                app.timeline.drag = None;
            }
        }
        _ => {}
    }
}

/// The reference strip under the mouse, and which end of it if it's near one.
fn strip_at(app: &KayeApp, layout: &Layout2, bones: &[BoneId], pos: Pos2) -> Option<(usize, Option<Edge>)> {
    if !layout.area.contains(pos) {
        return None;
    }
    let rows = track_rows(app, bones);
    let Some(TrackRow::Reference(index)) = hit_row(app, layout, pos.y, rows.len()).map(|r| rows[r]) else {
        return None;
    };
    let reference = &app.scene.references[index];
    let (start, end) = reference.span(references::rate(app, reference));
    let (left, right) = (layout.x(start as f32), layout.x(end as f32));
    // A short strip still keeps some middle to grab.
    let reach = STRIP_EDGE.min((right - left) * 0.3);
    if (pos.x - left).abs() <= reach {
        Some((index, Some(Edge::Start)))
    } else if (pos.x - right).abs() <= reach {
        Some((index, Some(Edge::End)))
    } else if pos.x > left && pos.x < right {
        Some((index, None))
    } else {
        None
    }
}

/// A key's diamond, or a square for one that holds.
pub(super) fn key_shape(shapes: &mut Vec<Shape>, center: Pos2, radius: f32, square: bool, fill: Color32) {
    let outline = Stroke::new(1.0, theme::window());
    if square {
        let rect = Rect::from_center_size(center, Vec2::splat(radius * 1.6));
        shapes.push(Shape::rect_filled(rect, 0.0, fill));
        shapes.push(Shape::rect_stroke(rect, 0.0, outline, StrokeKind::Outside));
    } else {
        let points = vec![
            center - vec2(0.0, radius),
            center + vec2(radius, 0.0),
            center + vec2(0.0, radius),
            center - vec2(radius, 0.0),
        ];
        shapes.push(Shape::convex_polygon(points, fill, outline));
    }
}

fn paint_tracks(app: &mut KayeApp, ui: &Ui, layout: &Layout2, bones: &[BoneId]) {
    let list = Rect::from_min_max(pos2(layout.side.left(), layout.area.top()), layout.side.max);
    ui.painter().rect_filled(layout.side, 0.0, theme::panel());
    let list_painter = ui.painter_at(list);
    let key_painter = ui.painter_at(layout.area);
    let full = list.union(layout.area);
    let row_painter = ui.painter_at(full);
    let rows = track_rows(app, bones);

    // The summary row sits first and collects every key in the rig.
    let center = row_y(app, layout, 0);
    let row_rect = |y: f32| Rect::from_center_size(pos2(full.center().x, y), vec2(full.width(), ROW_HEIGHT));
    row_painter.rect_filled(row_rect(center), 0.0, theme::header());
    list_painter.text(pos2(list.left() + 8.0, center), Align2::LEFT_CENTER, "Summary", theme::strong_font(11.5), theme::text());

    let mut shaded = false;
    for (row, kind) in rows.iter().copied().enumerate().skip(1) {
        let y = row_y(app, layout, row);
        let on_screen = y + ROW_HEIGHT >= full.top() && y - ROW_HEIGHT <= full.bottom();
        let bone = match kind {
            TrackRow::Bone(bone) => {
                shaded = !shaded;
                bone
            }
            TrackRow::Channel(bone, channel) => {
                if on_screen {
                    paint_channel_name(app, &list_painter, list, y, bone, channel);
                }
                continue;
            }
            TrackRow::Reference(index) => {
                if on_screen {
                    row_painter.rect_filled(row_rect(y), 0.0, Color32::from_black_alpha(28));
                    paint_reference_row(app, &list_painter, &key_painter, layout, list, y, index);
                }
                continue;
            }
            TrackRow::Summary => continue,
        };
        if !on_screen {
            continue;
        }
        let selected = app.is_bone_selected(bone);
        if selected {
            row_painter.rect_filled(row_rect(y), 0.0, theme::selected_row());
        } else if shaded {
            row_painter.rect_filled(row_rect(y), 0.0, Color32::from_white_alpha(4));
        }
        if app.animation.tracks.contains_key(&bone) {
            let open = app.timeline.open_channels.contains(&bone);
            chrome::paint_triangle(&list_painter, pos2(list.left() + 8.0, y), open, if selected { theme::ink() } else { theme::text_dim() });
        }
        let (icon, tint) = chrome::bone_icon(app.scene.bones[bone].kind);
        icons::paint(&list_painter, Rect::from_center_size(pos2(list.left() + 22.0, y), Vec2::splat(13.0)), icon, tint);
        list_painter.text(
            pos2(list.left() + 33.0, y),
            Align2::LEFT_CENTER,
            &app.scene.bones[bone].name,
            FontId::proportional(12.0),
            if selected { theme::ink() } else { theme::text_dim() },
        );
    }

    // Keys and the bars between them only change when keys, the selection or the view do,
    // so playing back reuses them.
    let signature = keys_signature(app, layout, bones, &rows);
    let mut kept = std::mem::take(&mut app.timeline.kept);
    let shape = kept.shape(ui.ctx(), signature, |shapes| key_shapes(app, layout, bones, &rows, shapes));
    app.timeline.kept = kept;
    key_painter.add(shape);
}

/// Everything the keys' drawing depends on.
fn keys_signature(app: &KayeApp, layout: &Layout2, bones: &[BoneId], rows: &[TrackRow]) -> u64 {
    let mut s = Signature::new();
    s.add_rect(layout.area);
    s.add_f32(layout.start);
    s.add_f32(layout.end);
    s.add_f32(app.timeline.scroll);
    s.add_f32(app.settings.animation.key_size);
    for color in [theme::accent(), theme::ink(), theme::text(), theme::text_dim(), theme::window()] {
        s.add_color(color);
    }
    for row in rows {
        match *row {
            TrackRow::Summary => s.add(0u8),
            TrackRow::Bone(bone) => s.add((1u8, bone)),
            TrackRow::Channel(bone, channel) => s.add((2u8, bone, channel)),
            TrackRow::Reference(index) => s.add((3u8, index)),
        }
    }
    for bone in bones {
        s.add(app.animation.tracks.get(bone).map_or(0, |t| t.keys.generation()));
    }
    s.add_set(app.timeline.selected.iter());
    s.finish()
}

/// The keys of a track that can show between these frames, plus one on each side so a bar
/// running off the edge still draws.
fn keys_in_view<'a>(keys: &'a [Key], layout: &Layout2, pad: f32) -> &'a [Key] {
    let first = layout.frame_at(layout.area.left() - pad).floor() as FrameNumber;
    let last = layout.frame_at(layout.area.right() + pad).ceil() as FrameNumber;
    let start = keys.partition_point(|k| k.frame < first).saturating_sub(1);
    let end = (keys.partition_point(|k| k.frame <= last) + 1).min(keys.len());
    &keys[start..end.max(start)]
}

/// Leaves out a marker landing on the one just drawn, which would be hidden under it anyway.
/// Only happens once whole frames are narrower than a pixel.
struct Markers {
    last: Option<(f32, Color32, bool)>,
}

impl Markers {
    fn new() -> Self {
        Self { last: None }
    }

    fn add(&mut self, shapes: &mut Vec<Shape>, center: Pos2, radius: f32, square: bool, fill: Color32) {
        if self.last.is_some_and(|(x, f, s)| (center.x - x).abs() < 0.5 && f == fill && s == square) {
            return;
        }
        self.last = Some((center.x, fill, square));
        key_shape(shapes, center, radius, square, fill);
    }
}

/// Bars between keys, with neighbors of the same kind joined into one line.
fn add_bars(shapes: &mut Vec<Shape>, bars: impl Iterator<Item = (f32, f32, Stroke)>, y: f32) {
    let mut run: Option<(f32, f32, Stroke)> = None;
    for (a, b, stroke) in bars {
        run = match run {
            Some((start, end, s)) if s == stroke && (a - end).abs() < 0.01 => Some((start, b, s)),
            Some((start, end, s)) => {
                shapes.push(Shape::hline(start..=end, y, s));
                Some((a, b, stroke))
            }
            None => Some((a, b, stroke)),
        };
    }
    if let Some((start, end, s)) = run {
        shapes.push(Shape::hline(start..=end, y, s));
    }
}

fn key_shapes(app: &KayeApp, layout: &Layout2, bones: &[BoneId], rows: &[TrackRow], shapes: &mut Vec<Shape>) {
    let size = app.settings.animation.key_size;
    let pad = size * 2.0;
    let area = layout.area;
    // Every keyed frame on screen, and whether all its keys are picked.
    let center = row_y(app, layout, 0);
    let mut frames: std::collections::BTreeMap<FrameNumber, bool> = std::collections::BTreeMap::new();
    for bone in bones {
        let Some(track) = app.animation.tracks.get(bone) else {
            continue;
        };
        for key in keys_in_view(&track.keys, layout, pad) {
            *frames.entry(key.frame).or_insert(true) &= app.timeline.selected.contains(&(*bone, key.frame));
        }
    }
    let mut markers = Markers::new();
    for (frame, selected) in frames {
        let fill = if selected { theme::accent() } else { theme::text_dim() };
        markers.add(shapes, pos2(layout.x(frame as f32), center), size - 1.0, false, fill);
    }

    for (row, kind) in rows.iter().copied().enumerate().skip(1) {
        let y = row_y(app, layout, row);
        if y + ROW_HEIGHT < area.top() || y - ROW_HEIGHT > area.bottom() {
            continue;
        }
        let (bone, channel) = match kind {
            TrackRow::Bone(bone) => (bone, None),
            TrackRow::Channel(bone, channel) => (bone, Some(channel)),
            TrackRow::Summary | TrackRow::Reference(_) => continue,
        };
        let Some(track) = app.animation.tracks.get(&bone) else {
            continue;
        };
        let keys = keys_in_view(&track.keys, layout, pad);
        let mut markers = Markers::new();
        match channel {
            None => {
                let bars = keys.windows(2).map(|pair| {
                    // A solid bar means the pose holds still between these keys.
                    let still = pair[0].pose.approx_eq(pair[1].pose) || pair[0].easing.style == EaseStyle::Constant;
                    let width = if still { 3.0 } else { 1.0 };
                    (layout.x(pair[0].frame as f32), layout.x(pair[1].frame as f32), Stroke::new(width, Color32::from_white_alpha(28)))
                });
                add_bars(shapes, bars, y);
                for key in keys {
                    let fill = if app.timeline.selected.contains(&(bone, key.frame)) { theme::accent() } else { theme::ink() };
                    markers.add(shapes, pos2(layout.x(key.frame as f32), y), size, key.easing.style == EaseStyle::Constant, fill);
                }
            }
            Some(channel) => {
                // The channel's keys are only the ones that set it, so a bar can skip keys
                // that don't, and has to start from the last one before the view.
                let all = &track.keys[..];
                let first = keys.first().map_or(0, |k| all.partition_point(|a| a.frame < k.frame));
                let before = all[..first].iter().rev().find(|k| k.sets(channel));
                let last = keys.last().map_or(0, |k| all.partition_point(|a| a.frame <= k.frame));
                let after = all[last..].iter().find(|k| k.sets(channel));
                let set: Vec<&Key> = before.into_iter().chain(keys.iter().filter(|k| k.sets(channel))).chain(after).collect();
                let bars = set.windows(2).map(|pair| (layout.x(pair[0].frame as f32), layout.x(pair[1].frame as f32), Stroke::new(1.0, Color32::from_white_alpha(20))));
                add_bars(shapes, bars, y);
                for key in set {
                    let fill = if app.timeline.selected.contains(&(bone, key.frame)) { theme::accent() } else { theme::text() };
                    markers.add(shapes, pos2(layout.x(key.frame as f32), y), size - 1.0, key.easing.style == EaseStyle::Constant, fill);
                }
            }
        }
    }
}

/// A reference's row: its name and eye on the left, and its strip on the timeline.
fn paint_reference_row(app: &KayeApp, list_painter: &egui::Painter, key_painter: &egui::Painter, layout: &Layout2, list: Rect, y: f32, index: usize) {
    let reference = &app.scene.references[index];
    let open = app.references.is_open(reference.id);
    let picture = reference.is_picture() && crate::media::MediaKind::of(std::path::Path::new(&reference.path)) == Some(crate::media::MediaKind::Picture);
    let icon = if picture { Icon::Image } else { Icon::Animations };
    icons::paint(list_painter, Rect::from_center_size(pos2(list.left() + 22.0, y), Vec2::splat(13.0)), icon, theme::camera_icon());
    list_painter.text(pos2(list.left() + 33.0, y), Align2::LEFT_CENTER, &reference.name, FontId::proportional(12.0), theme::text_dim());
    let eye = Rect::from_center_size(pos2(list.right() - EYE_WIDTH * 0.5, y), Vec2::splat(13.0));
    icons::paint(list_painter, eye, if open { Icon::Eye } else { Icon::EyeOff }, theme::text_dim());

    let (start, end) = reference.span(references::rate(app, reference));
    let strip = Rect::from_min_max(pos2(layout.x(start as f32), y - ROW_HEIGHT * 0.5 + 2.0), pos2(layout.x(end as f32), y + ROW_HEIGHT * 0.5 - 2.0));
    let color = theme::camera_icon();
    let dragged = matches!(&app.timeline.drag, Some(Drag::Strip { id, .. }) if *id == reference.id);
    key_painter.rect_filled(strip, 3.0, color.gamma_multiply(if dragged { 0.55 } else { 0.35 }));
    key_painter.rect_stroke(strip, 3.0, Stroke::new(1.0, color), StrokeKind::Inside);
    // Grips at both ends say they can be dragged.
    for x in [strip.left() + 3.5, strip.right() - 3.5] {
        if strip.width() > 14.0 {
            key_painter.vline(x, strip.shrink2(vec2(0.0, 3.0)).y_range(), Stroke::new(1.0, color));
        }
    }
    if let Some(progress) = app.references.loading(reference.id) {
        let bar = Rect::from_min_size(strip.left_bottom() - vec2(0.0, 2.0), vec2(strip.width() * progress, 2.0));
        key_painter.rect_filled(bar, 0.0, theme::ink());
    }
    // Only clipped at the sides, since Tahoma sits a little low in its line.
    let text_area = Rect::from_x_y_ranges(strip.shrink2(vec2(8.0, 0.0)).x_range(), (y - ROW_HEIGHT * 0.5)..=(y + ROW_HEIGHT * 0.5)).intersect(layout.area);
    if text_area.width() > 8.0 {
        key_painter.with_clip_rect(text_area).text(pos2(text_area.left(), y - 1.0), Align2::LEFT_CENTER, &reference.name, FontId::proportional(11.0), theme::ink());
    }
}

/// One channel open under its bone: its name beside the keys that set it.
fn paint_channel_name(app: &KayeApp, list_painter: &egui::Painter, list: Rect, y: f32, bone: BoneId, channel: usize) {
    list_painter.hline(list.left() + 30.0..=list.right(), y - ROW_HEIGHT * 0.5, Stroke::new(1.0, Color32::from_white_alpha(6)));
    let swatch = Rect::from_center_size(pos2(list.left() + 36.0, y), vec2(3.0, 11.0));
    list_painter.rect_filled(swatch, 1.0, graph::channel_color(channel));
    list_painter.text(pos2(list.left() + 44.0, y), Align2::LEFT_CENTER, graph::channel_name(app, bone, channel), FontId::proportional(11.5), theme::text_dim());
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
    Reference(u64),
}

fn reference_menu(app: &mut KayeApp, ui: &mut Ui, id: u64) {
    let open = app.references.is_open(id);
    if chrome::item(ui, if open { "Hide Window" } else { "Show Window" }, "", true) {
        if open {
            app.references.hide(id);
        } else {
            app.references.show(id);
        }
    }
    if chrome::item(ui, "Start Here", "", true) {
        // Lines the strip's start up with the frame showing.
        if let Some(reference) = app.scene.references.iter_mut().find(|r| r.id == id) {
            let rate = reference.rate(app.animation.fps, app.settings.animation.match_reference_fps);
            let (start, _) = reference.span(rate);
            reference.move_by(app.frame.round() as f64 - start);
        }
    }
    ui.separator();
    if chrome::item(ui, "Delete Reference", "", true) {
        references::delete(app, id);
    }
}

fn context_menus(app: &mut KayeApp, layout: &Layout2, response: &egui::Response) {
    if response.secondary_clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let frame = layout.frame_at(pos.x).round() as FrameNumber;
            let rows = track_rows(app, &focused_bones(app));
            let on_reference = match hit_row(app, layout, pos.y, rows.len()).map(|r| rows[r]) {
                Some(TrackRow::Reference(index)) if app.bottom_tab == BottomTab::Timeline && !layout.ruler.contains(pos) => Some(app.scene.references[index].id),
                _ => None,
            };
            app.timeline.menu = if layout.ruler.contains(pos) {
                MenuTarget::Ruler(frame)
            } else if let Some(id) = on_reference {
                MenuTarget::Reference(id)
            } else {
                let hit = match app.bottom_tab {
                    BottomTab::Timeline => keys_at(app, layout, &focused_bones(app), pos),
                    BottomTab::Graph => graph::keys_at(app, layout, pos),
                };
                let rows = track_rows(app, &focused_bones(app));
                app.timeline.menu_channel = match (app.bottom_tab, hit_row(app, layout, pos.y, rows.len()).map(|r| rows[r])) {
                    (BottomTab::Timeline, Some(TrackRow::Channel(_, channel))) => Some(channel),
                    _ => None,
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
    response.context_menu(|ui| {
        chrome::menu_rows(ui);
        match app.timeline.menu {
            MenuTarget::Keys => key_menu(app, ui),
            MenuTarget::Empty(frame) => empty_menu(app, ui, frame),
            MenuTarget::Ruler(frame) => ruler_menu(app, ui, frame),
            MenuTarget::Reference(id) => reference_menu(app, ui, id),
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
    if ui.add(Button::new("Copy").shortcut_text(app.key_text(Action::CopyPose))).clicked() {
        app.copy_keys(ui.ctx());
        ui.close();
    }
    if ui.add_enabled(!app.key_clipboard.is_empty(), Button::new("Paste to Playhead").shortcut_text(app.key_text(Action::PastePose))).clicked() {
        app.paste_keys();
        ui.close();
    }
    if ui.add_enabled(!app.key_clipboard.is_empty(), Button::new("Paste Mirrored to Playhead").shortcut_text(app.key_text(Action::PasteMirrored))).clicked() {
        app.paste_keys_mirrored();
        ui.close();
    }
    if ui.add(Button::new("Mirror").shortcut_text(app.key_text(Action::MirrorPose))).clicked() {
        app.mirror_selected_keys();
        ui.close();
    }
    if ui.add(Button::new("Delete").shortcut_text(app.key_text(Action::DeleteSelectedKeys))).clicked() {
        app.delete_selected_keys();
        ui.close();
    }
    if let Some(channel) = app.timeline.menu_channel {
        let bone = targets.first().map(|(b, _)| *b);
        let name = bone.map_or("", |b| graph::channel_name(app, b, channel));
        if ui.button(format!("Delete {name} Only")).clicked() {
            app.delete_selected_channel(channel);
            ui.close();
        }
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
    if ui.add_enabled(!app.key_clipboard.is_empty(), Button::new("Paste Keys Here").shortcut_text(app.key_text(Action::PastePose))).clicked() {
        app.pause();
        app.set_frame(frame as f32);
        app.paste_keys();
        ui.close();
    }
    if ui.add_enabled(!app.key_clipboard.is_empty(), Button::new("Paste Mirrored Keys Here").shortcut_text(app.key_text(Action::PasteMirrored))).clicked() {
        app.pause();
        app.set_frame(frame as f32);
        app.paste_keys_mirrored();
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
