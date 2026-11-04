//! The Graph Editor: every channel of the selected bone as a curve to grab. Keys move
//! in time and value, and handles shape how motion speeds up and slows down around
//! them. Everything dragged here can also be typed into the inspector on the right.

use std::collections::HashSet;

use egui::{Align2, Color32, ComboBox, DragValue, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2};

use super::icons::{self, Icon};
use super::timeline::{KEY_PICK, Layout2, ROW_HEIGHT, easing_editor, key_marker};
use super::{HandleKind, KayeApp, theme};
use crate::animation::{Animation, CHANNEL_NAMES, CHANNELS, EaseStyle, FrameNumber, HandleType, KeyId, Track, VALUE_CHANNEL, angles_near};
use crate::gizmo::AXIS_COLORS;
use crate::scene::{BoneId, BoneKind};

pub const CHANNEL_WIDTH: f32 = 176.0;
pub const INSPECTOR_WIDTH: f32 = 204.0;
const VALUE_TICK_SPACING: f32 = 30.0;
const HANDLE_RADIUS: f32 = 3.5;
const CURVE_COLUMN: f32 = 2.0;
/// Curves leave this much room above and below when fitted.
const FIT_MARGIN: f32 = 0.12;
const MIN_SPAN: f32 = 0.05;
const MAX_SPAN: f32 = 100_000.0;

/// Which part of a key on a curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Grip {
    Key,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Point {
    pub frame: FrameNumber,
    pub channel: usize,
    pub grip: Grip,
}

impl Point {
    fn key(frame: FrameNumber, channel: usize) -> Self {
        Self { frame, channel, grip: Grip::Key }
    }
}

enum GraphDrag {
    Keys {
        start: Pos2,
        before: Animation,
        points: Vec<Point>,
        moved: (FrameNumber, f32),
    },
    Handle {
        point: Point,
        before: Animation,
    },
    Box {
        anchor: Pos2,
        base: HashSet<Point>,
    },
}

pub struct GraphState {
    /// The values at the plot's bottom and top.
    low: f32,
    high: f32,
    hidden: [bool; CHANNELS],
    /// Every point selected on the bone being shown.
    pub selected: HashSet<Point>,
    /// The point the inspector shows.
    active: Option<Point>,
    drag: Option<GraphDrag>,
    /// The bone the view was last set up for.
    bone: Option<BoneId>,
}

impl Default for GraphState {
    fn default() -> Self {
        Self {
            low: -90.0,
            high: 90.0,
            hidden: [false; CHANNELS],
            selected: HashSet::new(),
            active: None,
            drag: None,
            bone: None,
        }
    }
}

/// Maps between values and the plot's height.
#[derive(Clone, Copy)]
struct Plot {
    rect: Rect,
    low: f32,
    high: f32,
}

impl Plot {
    fn new(app: &KayeApp, layout: &Layout2) -> Self {
        Self { rect: layout.area, low: app.timeline.graph.low, high: app.timeline.graph.high }
    }

    fn y(&self, value: f32) -> f32 {
        self.rect.bottom() - (value - self.low) / (self.high - self.low) * self.rect.height()
    }

    fn value_at(&self, y: f32) -> f32 {
        self.low + (self.rect.bottom() - y) / self.rect.height() * (self.high - self.low)
    }
}

fn channel_color(channel: usize) -> Color32 {
    match channel {
        VALUE_CHANNEL => theme::ink(),
        c => AXIS_COLORS[c % 3],
    }
}

/// Bones with an extra number have a seventh channel.
fn value_name(app: &KayeApp, bone: BoneId) -> Option<&'static str> {
    match app.scene.bones[bone].kind {
        BoneKind::Camera => Some("Field of View"),
        BoneKind::IkTarget => Some("Influence"),
        _ => app.has_unparent(bone).then_some("Influence"),
    }
}

fn channel_name(app: &KayeApp, bone: BoneId, channel: usize) -> &'static str {
    match CHANNEL_NAMES.get(channel) {
        Some(name) => name,
        None => value_name(app, bone).unwrap_or(""),
    }
}

fn channel_count(app: &KayeApp, bone: BoneId) -> usize {
    if value_name(app, bone).is_some() { CHANNELS } else { VALUE_CHANNEL }
}

fn visible_channels(app: &KayeApp, bone: BoneId) -> Vec<usize> {
    (0..channel_count(app, bone)).filter(|c| !app.timeline.graph.hidden[*c]).collect()
}

/// A bone's channels at this moment, from its live pose.
fn live_channels(app: &KayeApp, bone: BoneId) -> [f32; CHANNELS] {
    let b = &app.scene.bones[bone];
    let (p, r) = (b.pose.pos, angles_near(b.pose.rot, [0.0; 3]));
    [p.x, p.y, p.z, r[0], r[1], r[2], b.value]
}

/// A key's left handle only matters if the span arriving at it is a curve, and its
/// right handle only if the span leaving it is.
fn handle_shown(track: &Track, index: usize, grip: Grip) -> bool {
    match grip {
        Grip::Key => true,
        Grip::Left => index > 0 && track.keys[index - 1].easing.style == EaseStyle::Bezier,
        Grip::Right => index + 1 < track.keys.len() && track.keys[index].easing.style == EaseStyle::Bezier,
    }
}

/// A point on screen, and the key it belongs to.
struct Marker {
    point: Point,
    pos: Pos2,
    key: Pos2,
}

/// Every point that can be grabbed: all keys, plus the handles of selected keys.
fn markers(app: &KayeApp, bone: BoneId, layout: &Layout2, plot: &Plot) -> Vec<Marker> {
    let Some(track) = app.animation.tracks.get(&bone) else {
        return Vec::new();
    };
    let curves = track.curves(app.animation.smoothing);
    let selected = &app.timeline.graph.selected;
    let mut out = Vec::new();
    for channel in visible_channels(app, bone) {
        for (index, key) in track.keys.iter().enumerate() {
            let point = &curves.points[channel][index];
            let at = |offset: [f32; 2]| pos2(layout.x(key.frame as f32 + offset[0]), plot.y(point.value + offset[1]));
            let pos = at([0.0, 0.0]);
            let open = [Grip::Key, Grip::Left, Grip::Right].iter().any(|grip| selected.contains(&Point { frame: key.frame, channel, grip: *grip }));
            if open {
                for (grip, offset) in [(Grip::Left, point.left), (Grip::Right, point.right)] {
                    if handle_shown(track, index, grip) {
                        out.push(Marker { point: Point { frame: key.frame, channel, grip }, pos: at(offset), key: pos });
                    }
                }
            }
            out.push(Marker { point: Point::key(key.frame, channel), pos, key: pos });
        }
    }
    out
}

/// What's under the mouse. Handles go first, since they sit on top.
fn hit(app: &KayeApp, bone: BoneId, layout: &Layout2, plot: &Plot, pos: Pos2) -> Option<Point> {
    let all = markers(app, bone, layout, plot);
    let closest = |handles: bool| {
        all.iter()
            .filter(|m| (m.point.grip != Grip::Key) == handles)
            .map(|m| (m.point, (m.pos - pos).length()))
            .filter(|(_, d)| *d <= KEY_PICK)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(point, _)| point)
    };
    closest(true).or_else(|| closest(false))
}

/// The timeline's selection follows the graph's, so its menus and Delete act on it.
fn sync_keys(app: &mut KayeApp, bone: BoneId) {
    app.timeline.selected = app.timeline.graph.selected.iter().map(|p| (bone, p.frame)).collect();
}

/// Sets the plot's height to fit what's visible.
pub fn fit_values(app: &mut KayeApp) {
    let Some(bone) = app.curve_bone() else {
        return;
    };
    let channels = visible_channels(app, bone);
    let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
    let mut include = |value: f32| {
        low = low.min(value);
        high = high.max(value);
    };
    match app.animation.tracks.get(&bone) {
        Some(track) => {
            let curves = track.curves(app.animation.smoothing);
            for channel in &channels {
                for (index, point) in curves.points[*channel].iter().enumerate() {
                    include(point.value);
                    for (grip, offset) in [(Grip::Left, point.left), (Grip::Right, point.right)] {
                        if handle_shown(track, index, grip) {
                            include(point.value + offset[1]);
                        }
                    }
                }
            }
        }
        None => {
            let live = live_channels(app, bone);
            channels.iter().for_each(|c| include(live[*c]));
        }
    }
    if !low.is_finite() {
        return;
    }
    let span = (high - low).max(2.0);
    let middle = (high + low) * 0.5;
    let graph = &mut app.timeline.graph;
    graph.low = middle - span * (0.5 + FIT_MARGIN);
    graph.high = middle + span * (0.5 + FIT_MARGIN);
}

/// Switching to the tab brings along keys picked on the timeline.
pub fn opened(app: &mut KayeApp) {
    let Some(bone) = app.curve_bone() else {
        return;
    };
    let picked: Vec<FrameNumber> = app.timeline.selected.iter().filter(|(b, _)| *b == bone).map(|(_, f)| *f).collect();
    let channels = visible_channels(app, bone);
    let graph = &mut app.timeline.graph;
    graph.selected = picked.iter().flat_map(|f| channels.iter().map(|c| Point::key(*f, *c))).collect();
    graph.active = None;
    fit_values(app);
}

/// Drops points whose keys have gone, after undo for example.
pub fn prune(app: &mut KayeApp) {
    let Some(bone) = app.timeline.graph.bone else {
        return;
    };
    let track = app.animation.tracks.get(&bone);
    let exists = |frame: FrameNumber| track.is_some_and(|t| t.key_at(frame).is_some());
    let graph = &mut app.timeline.graph;
    graph.selected.retain(|p| exists(p.frame));
    if graph.active.is_some_and(|p| !exists(p.frame)) {
        graph.active = None;
    }
}

/// The channels picked on the graph, for Handle Type.
pub fn picked_channels(app: &KayeApp) -> Vec<(KeyId, usize)> {
    let Some(bone) = app.timeline.graph.bone else {
        return Vec::new();
    };
    let mut picked: Vec<(KeyId, usize)> = app.timeline.graph.selected.iter().map(|p| ((bone, p.frame), p.channel)).collect();
    picked.sort_unstable();
    picked.dedup();
    picked
}

/// Puts a graph drag back as it was. Returns true when there was one to cancel.
pub fn cancel_drag(app: &mut KayeApp) -> bool {
    match app.timeline.graph.drag.take() {
        Some(GraphDrag::Keys { before, points, moved, .. }) => {
            app.animation = before;
            let graph = &mut app.timeline.graph;
            graph.selected = points.into_iter().collect();
            graph.active = graph.active.map(|p| Point { frame: p.frame - moved.0, ..p });
            app.animation.apply(&mut app.scene, app.frame);
            if let Some(bone) = app.timeline.graph.bone {
                sync_keys(app, bone);
            }
            true
        }
        Some(GraphDrag::Handle { before, .. }) => {
            app.animation = before;
            app.animation.apply(&mut app.scene, app.frame);
            true
        }
        Some(GraphDrag::Box { base, .. }) => {
            app.timeline.graph.selected = base;
            true
        }
        None => false,
    }
}

/// The keys a right click means, selecting the point it landed on.
pub(super) fn keys_at(app: &mut KayeApp, layout: &Layout2, pos: Pos2) -> Vec<KeyId> {
    let Some(bone) = app.curve_bone() else {
        return Vec::new();
    };
    let plot = Plot::new(app, layout);
    let Some(point) = hit(app, bone, layout, &plot, pos) else {
        return Vec::new();
    };
    let graph = &mut app.timeline.graph;
    if !graph.selected.contains(&point) {
        graph.selected = [point].into_iter().collect();
    }
    graph.active = Some(point);
    sync_keys(app, bone);
    vec![(bone, point.frame)]
}

/// Zooming and panning the value axis. Time is shared with the timeline.
pub(super) fn navigate(app: &mut KayeApp, ui: &mut Ui, layout: &Layout2, response: &egui::Response) {
    let (zoom, delta) = ui.input(|i| (i.zoom_delta(), i.pointer.delta()));
    let graph = &mut app.timeline.graph;
    if let Some(pos) = response.hover_pos().filter(|p| layout.area.contains(*p)) {
        if zoom != 1.0 {
            let plot = Plot { rect: layout.area, low: graph.low, high: graph.high };
            let at = plot.value_at(pos.y);
            let span = ((graph.high - graph.low) / zoom).clamp(MIN_SPAN, MAX_SPAN);
            let share = (at - graph.low) / (graph.high - graph.low);
            graph.low = at - span * share;
            graph.high = graph.low + span;
        }
    }
    if response.dragged_by(egui::PointerButton::Middle) {
        let shift = delta.y * (graph.high - graph.low) / layout.area.height().max(1.0);
        graph.low += shift;
        graph.high += shift;
    }
}

pub(super) fn input(app: &mut KayeApp, ui: &mut Ui, layout: &Layout2, response: &egui::Response) {
    let Some(bone) = app.curve_bone() else {
        app.timeline.graph.bone = None;
        return;
    };
    if app.timeline.graph.bone != Some(bone) {
        let graph = &mut app.timeline.graph;
        graph.bone = Some(bone);
        graph.selected.clear();
        graph.active = None;
        graph.drag = None;
        fit_values(app);
    }
    let plot = Plot::new(app, layout);
    let (pressed, released, mouse, modifiers) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_released(), i.pointer.latest_pos(), i.modifiers));

    if pressed && app.timeline.graph.drag.is_none() {
        if let Some(pos) = response.hover_pos().filter(|p| layout.area.contains(*p)) {
            press(app, bone, layout, &plot, pos, modifiers.shift);
        }
    }
    let Some(mouse) = mouse else {
        return;
    };
    match &app.timeline.graph.drag {
        Some(GraphDrag::Keys { .. }) => drag_keys(app, bone, layout, &plot, mouse, modifiers.shift),
        Some(GraphDrag::Handle { .. }) => drag_handle(app, bone, layout, &plot, mouse, modifiers.alt),
        Some(GraphDrag::Box { .. }) => drag_box(app, bone, layout, &plot, mouse),
        None => {}
    }
    if released {
        release(app, bone);
    }
}

fn press(app: &mut KayeApp, bone: BoneId, layout: &Layout2, plot: &Plot, pos: Pos2, shift: bool) {
    let found = hit(app, bone, layout, plot, pos);
    let Some(point) = found else {
        let base = if shift { app.timeline.graph.selected.clone() } else { HashSet::new() };
        let graph = &mut app.timeline.graph;
        graph.selected = base.clone();
        graph.active = None;
        graph.drag = Some(GraphDrag::Box { anchor: pos, base });
        sync_keys(app, bone);
        return;
    };
    let before = app.animation.clone();
    app.pause();
    let graph = &mut app.timeline.graph;
    if point.grip != Grip::Key {
        graph.selected.insert(point);
        graph.active = Some(point);
        graph.drag = Some(GraphDrag::Handle { point, before });
        return;
    }
    if shift && graph.selected.remove(&point) {
        graph.active = None;
        sync_keys(app, bone);
        return;
    }
    if !shift && !graph.selected.contains(&point) {
        graph.selected.clear();
    }
    graph.selected.insert(point);
    graph.active = Some(point);
    let points: Vec<Point> = graph.selected.iter().copied().filter(|p| p.grip == Grip::Key).collect();
    graph.drag = Some(GraphDrag::Keys { start: pos, before, points, moved: (0, 0.0) });
    sync_keys(app, bone);
}

fn drag_keys(app: &mut KayeApp, bone: BoneId, layout: &Layout2, plot: &Plot, mouse: Pos2, lock: bool) {
    let Some(GraphDrag::Keys { start, before, points, moved }) = &app.timeline.graph.drag else {
        return;
    };
    let (start, previous) = (*start, *moved);
    let mut frames = (layout.frame_at(mouse.x) - layout.frame_at(start.x)).round() as FrameNumber;
    let mut value = plot.value_at(mouse.y) - plot.value_at(start.y);
    // Shift holds a key to whichever direction it has moved furthest.
    if lock {
        if (mouse.x - start.x).abs() >= (mouse.y - start.y).abs() {
            value = 0.0;
        } else {
            frames = 0;
        }
    }
    if !app.settings.animation.allow_negative_frames {
        frames = frames.max(-points.iter().map(|p| p.frame).min().unwrap_or(0));
    }
    if (frames, value) == previous {
        return;
    }
    let keys: HashSet<KeyId> = points.iter().map(|p| (bone, p.frame)).collect();
    let mut next = if frames != 0 { before.moved(&keys, frames) } else { before.clone() };
    let starts = before.tracks.get(&bone).map(|t| (t.key_channels(), t.clone()));
    if let (Some((values, old)), Some(track)) = (starts, next.tracks.get_mut(&bone)) {
        for p in points {
            if let (Some(from), Some(to)) = (old.index(p.frame), track.index(p.frame + frames)) {
                track.set_channel(to, p.channel, values[from][p.channel] + value);
            }
        }
    }
    app.animation = next;
    let shifted = |p: &Point| Point { frame: p.frame + frames - previous.0, ..*p };
    let graph = &mut app.timeline.graph;
    graph.selected = graph.selected.iter().map(shifted).collect();
    graph.active = graph.active.as_ref().map(shifted);
    if let Some(GraphDrag::Keys { moved, .. }) = &mut graph.drag {
        *moved = (frames, value);
    }
    sync_keys(app, bone);
    app.animation.apply(&mut app.scene, app.frame);
}

fn drag_handle(app: &mut KayeApp, bone: BoneId, layout: &Layout2, plot: &Plot, mouse: Pos2, break_handle: bool) {
    let Some(GraphDrag::Handle { point, before }) = &app.timeline.graph.drag else {
        return;
    };
    let point = *point;
    let mut next = before.clone();
    if let Some(track) = next.tracks.get_mut(&bone) {
        if let Some(index) = track.index(point.frame) {
            let value = track.key_channels()[index][point.channel];
            let offset = [layout.frame_at(mouse.x) - point.frame as f32, plot.value_at(mouse.y) - value];
            // Alt breaks a handle free of its partner, and it stays free after that.
            let aligned = !break_handle && track.handle_mode(index, point.channel).unwrap_or(true);
            track.set_handle(index, point.channel, point.grip == Grip::Right, offset, aligned, app.animation.smoothing);
        }
    }
    app.animation = next;
    app.animation.apply(&mut app.scene, app.frame);
}

fn drag_box(app: &mut KayeApp, bone: BoneId, layout: &Layout2, plot: &Plot, mouse: Pos2) {
    let Some(GraphDrag::Box { anchor, base }) = &app.timeline.graph.drag else {
        return;
    };
    let area = Rect::from_two_pos(*anchor, mouse);
    let mut selected = base.clone();
    for marker in markers(app, bone, layout, plot) {
        if marker.point.grip == Grip::Key && area.contains(marker.pos) {
            selected.insert(marker.point);
        }
    }
    app.timeline.graph.selected = selected;
    sync_keys(app, bone);
}

fn release(app: &mut KayeApp, bone: BoneId) {
    match app.timeline.graph.drag.take() {
        Some(GraphDrag::Keys { before, .. }) | Some(GraphDrag::Handle { before, .. }) => {
            if before != app.animation {
                app.history.push_edit("Edit curve", Vec::new(), before, app.animation.clone());
            }
            sync_keys(app, bone);
        }
        _ => {}
    }
}

/// How far apart the value lines sit, so they stay readable at any zoom.
fn value_step(plot: &Plot) -> f32 {
    let wanted = (plot.high - plot.low) * VALUE_TICK_SPACING / plot.rect.height().max(1.0);
    let scale = 10f32.powf(wanted.log10().floor());
    [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * scale).find(|s| *s >= wanted).unwrap_or(scale * 10.0)
}

fn paint_value_grid(painter: &egui::Painter, plot: &Plot) {
    let step = value_step(plot);
    let first = (plot.low / step).ceil() as i32;
    let last = (plot.high / step).floor() as i32;
    for i in first..=last {
        let value = i as f32 * step;
        let y = plot.y(value);
        let alpha = if i == 0 { 30 } else { 9 };
        painter.hline(plot.rect.x_range(), y, Stroke::new(1.0, Color32::from_white_alpha(alpha)));
        let text = if step < 1.0 { format!("{value:.2}") } else { format!("{value:.0}") };
        painter.text(pos2(plot.rect.left() + 4.0, y - 1.0), Align2::LEFT_BOTTOM, text, FontId::proportional(10.0), theme::text_faint());
    }
}

/// Position is dashed and rotation is solid, so the two can be told apart with no
/// help from color. Each curve also carries its name at the plot's edge.
fn paint_curve(painter: &egui::Painter, points: Vec<Pos2>, channel: usize, strong: bool) {
    let color = channel_color(channel);
    let width = if strong { 2.2 } else { 1.5 };
    if channel < 3 {
        painter.extend(Shape::dashed_line(&points, Stroke::new(width, color), 7.0, 4.0));
    } else {
        painter.add(Shape::line(points, Stroke::new(width, color)));
    }
}

pub(super) fn paint(app: &KayeApp, ui: &Ui, layout: &Layout2) {
    let plot = Plot::new(app, layout);
    let painter = ui.painter_at(layout.area);
    paint_value_grid(&painter, &plot);
    let Some(bone) = app.timeline.graph.bone else {
        return;
    };
    let Some(track) = app.animation.tracks.get(&bone) else {
        return;
    };
    let curves = track.curves(app.animation.smoothing);
    let channels = visible_channels(app, bone);
    let columns = (plot.rect.width() / CURVE_COLUMN) as usize;
    let mut lines: Vec<Vec<Pos2>> = vec![Vec::with_capacity(columns + 1); channels.len()];
    for column in 0..=columns {
        let x = plot.rect.left() + column as f32 * CURVE_COLUMN;
        if let Some(values) = curves.at(layout.frame_at(x)) {
            for (line, channel) in lines.iter_mut().zip(&channels) {
                line.push(pos2(x, plot.y(values[*channel])));
            }
        }
    }
    let graph = &app.timeline.graph;
    let mut labels: Vec<f32> = Vec::new();
    for (line, channel) in lines.into_iter().zip(&channels) {
        let strong = graph.active.is_some_and(|p| p.channel == *channel);
        if let Some(end) = line.last().copied() {
            // Curves that end together would stack their names into a smudge.
            if !labels.iter().any(|y| (y - end.y).abs() < 11.0) && plot.rect.y_range().contains(end.y) {
                labels.push(end.y);
                let name = match *channel {
                    VALUE_CHANNEL => "V".to_string(),
                    c => format!("{}{}", if c < 3 { "P" } else { "R" }, ["X", "Y", "Z"][c % 3]),
                };
                painter.text(end + vec2(-3.0, -1.0), Align2::RIGHT_BOTTOM, name, FontId::proportional(10.0), channel_color(*channel));
            }
        }
        paint_curve(&painter, line, *channel, strong);
    }
    let all = markers(app, bone, layout, &plot);
    for marker in all.iter().filter(|m| m.point.grip != Grip::Key) {
        let selected = graph.active == Some(marker.point);
        painter.line_segment([marker.key, marker.pos], Stroke::new(1.0, theme::text_faint()));
        let fill = if selected { theme::accent() } else { theme::ink() };
        painter.circle(marker.pos, HANDLE_RADIUS, fill, Stroke::new(1.0, theme::window()));
    }
    let size = app.settings.animation.key_size;
    for marker in all.iter().filter(|m| m.point.grip == Grip::Key) {
        let selected = graph.selected.contains(&marker.point);
        let fill = if selected { theme::accent() } else { channel_color(marker.point.channel) };
        let square = track.key_at(marker.point.frame).is_some_and(|k| k.easing.style == EaseStyle::Constant);
        key_marker(&painter, marker.pos, size, square, fill);
    }
    if let (Some(GraphDrag::Box { anchor, .. }), Some(mouse)) = (&graph.drag, ui.input(|i| i.pointer.latest_pos())) {
        let area = Rect::from_two_pos(*anchor, mouse).intersect(layout.area);
        painter.rect_filled(area, 0.0, theme::accent().gamma_multiply(0.14));
        painter.rect_stroke(area, 0.0, Stroke::new(1.0, theme::accent()), StrokeKind::Inside);
    }
}

/// The channel list on the left and the inspector on the right. Drawn after the
/// plot, so the plot's contents can't paint over them.
pub(super) fn side_panels(app: &mut KayeApp, ui: &mut Ui, rect: Rect, layout: &Layout2) {
    ui.painter().rect_filled(layout.side, 0.0, theme::panel());
    let inspector = Rect::from_min_max(pos2(layout.area.right(), rect.top()), rect.max);
    ui.painter().rect_filled(inspector, 0.0, theme::panel());
    ui.painter().vline(inspector.left(), inspector.y_range(), Stroke::new(1.0, theme::border()));
    if let Some(bone) = app.timeline.graph.bone {
        channel_list(app, ui, layout, bone);
        inspect(app, ui, inspector, bone);
    }
}

fn channel_list(app: &mut KayeApp, ui: &mut Ui, layout: &Layout2, bone: BoneId) {
    let live = match app.animation.tracks.get(&bone).and_then(|t| t.curves(app.animation.smoothing).at(app.frame)) {
        Some(values) => values,
        None => live_channels(app, bone),
    };
    let list = layout.side;
    let painter = ui.painter_at(list);
    let count = channel_count(app, bone);
    for channel in 0..count {
        let y = list.top() + (channel as f32 + 0.5) * ROW_HEIGHT - app.timeline.scroll;
        let row = Rect::from_center_size(pos2(list.center().x, y), vec2(list.width(), ROW_HEIGHT));
        let response = ui.interact(row.intersect(list), ui.scope_id().with(("graph_channel", channel)), Sense::click());
        let hidden = app.timeline.graph.hidden[channel];
        if response.hovered() {
            painter.rect_filled(row, 0.0, theme::header());
        } else if channel % 2 == 0 {
            painter.rect_filled(row, 0.0, theme::row_alt());
        }
        let eye = Rect::from_center_size(pos2(list.left() + 12.0, y), Vec2::splat(13.0));
        let (icon, tint) = if hidden { (Icon::EyeOff, theme::text_faint()) } else { (Icon::Eye, theme::text_dim()) };
        icons::paint(&painter, eye, icon, tint);

        let swatch = [pos2(list.left() + 23.0, y), pos2(list.left() + 37.0, y)];
        let color = if hidden { theme::text_faint() } else { channel_color(channel) };
        if channel < 3 {
            painter.extend(Shape::dashed_line(&swatch, Stroke::new(2.0, color), 4.0, 2.0));
        } else {
            painter.line_segment(swatch, Stroke::new(2.0, color));
        }
        let text = if hidden { theme::text_faint() } else { theme::text() };
        painter.text(pos2(list.left() + 42.0, y), Align2::LEFT_CENTER, channel_name(app, bone, channel), FontId::proportional(11.5), text);
        painter.text(pos2(list.right() - 6.0, y), Align2::RIGHT_CENTER, format!("{:.2}", live[channel]), FontId::proportional(11.0), theme::text_dim());

        if response.clicked() {
            let on_eye = response.interact_pointer_pos().is_some_and(|p| p.x < list.left() + 20.0);
            let graph = &mut app.timeline.graph;
            if on_eye {
                graph.hidden[channel] ^= true;
            } else {
                // Clicking a name shows only that curve, and clicking again brings the rest back.
                let solo = (0..count).all(|c| graph.hidden[c] == (c != channel));
                for c in 0..count {
                    graph.hidden[c] = !solo && c != channel;
                }
            }
            graph.selected.retain(|p| !graph.hidden[p.channel]);
            fit_values(app);
        }
    }
}

fn field_row(ui: &mut Ui, label: &str, content: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let (rect, _) = ui.allocate_exact_size(vec2(42.0, 20.0), Sense::hover());
        ui.painter().text(rect.right_center(), Align2::RIGHT_CENTER, label, FontId::proportional(11.5), theme::text_dim());
        ui.add_space(3.0);
        content(ui);
    });
}

fn number(ui: &mut Ui, value: &mut f32, width: f32, speed: f64) -> bool {
    ui.add_sized([width, 20.0], DragValue::new(value).speed(speed).max_decimals(3)).changed()
}

/// The typed side of everything draggable on the plot.
fn inspect(app: &mut KayeApp, ui: &mut Ui, area: Rect, bone: BoneId) {
    let inner = area.shrink2(vec2(8.0, 6.0));
    let active = app.timeline.graph.active.and_then(|point| {
        let track = app.animation.tracks.get(&bone)?;
        let index = track.index(point.frame)?;
        let style = track.keys[index].easing.style;
        Some((point, index, style))
    });
    let Some((point, index, style)) = active else {
        easing_editor(app, ui, inner);
        return;
    };
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
    let ui = &mut child;
    ui.spacing_mut().item_spacing.y = 2.0;
    let name = channel_name(app, bone, point.channel);
    ui.label(egui::RichText::new(name).size(12.0).strong().color(channel_color(point.channel)));

    let Some(track) = app.animation.tracks.get(&bone) else {
        return;
    };
    let smoothing = app.animation.smoothing;
    let curves = track.curves(smoothing);
    let shown = curves.points[point.channel][index];
    let has = [handle_shown(track, index, Grip::Left), handle_shown(track, index, Grip::Right)];
    let mode = track.handle_mode(index, point.channel);
    let auto = track.keys[index].easing.handle_type;

    let mut frame = point.frame;
    let mut value = shown.value;
    let (mut left, mut right) = (
        [point.frame as f32 + shown.left[0], shown.value + shown.left[1]],
        [point.frame as f32 + shown.right[0], shown.value + shown.right[1]],
    );

    let mut new_frame = None;
    let mut new_value = None;
    let mut new_handle = None;
    field_row(ui, "Frame", |ui| {
        if ui.add_sized([60.0, 20.0], DragValue::new(&mut frame).speed(0.25)).changed() {
            new_frame = Some(frame);
        }
    });
    field_row(ui, "Value", |ui| {
        if number(ui, &mut value, 60.0, 0.1) {
            new_value = Some(value);
        }
    });

    if style != EaseStyle::Bezier {
        let plot = Rect::from_min_max(pos2(inner.left(), ui.cursor().top() + 2.0), inner.max);
        easing_editor(app, ui, plot);
    } else {
        field_row(ui, "Handles", |ui| {
            let label = match mode {
                Some(true) => "Aligned",
                Some(false) => "Free",
                None => auto.label(),
            };
            ComboBox::from_id_salt("graph_handle_kind").selected_text(label).width(126.0).show_ui(ui, |ui| {
                for handle in HandleType::ALL {
                    if ui.selectable_label(mode.is_none() && auto == handle, handle.label()).clicked() {
                        app.set_handle_kind(HandleKind::Auto(handle));
                    }
                }
                if ui.selectable_label(mode == Some(true), "Aligned").clicked() {
                    app.set_handle_kind(HandleKind::Aligned);
                }
                if ui.selectable_label(mode == Some(false), "Free").clicked() {
                    app.set_handle_kind(HandleKind::Free);
                }
            });
        });
        for (label, handle, shown, right_side) in [("Left", &mut left, has[0], false), ("Right", &mut right, has[1], true)] {
            if shown {
                field_row(ui, label, |ui| {
                    let first = number(ui, &mut handle[0], 62.0, 0.1);
                    let second = number(ui, &mut handle[1], 62.0, 0.1);
                    if first || second {
                        new_handle = Some((right_side, *handle));
                    }
                });
            }
        }
    }

    if let Some(frame) = new_frame.filter(|f| *f != point.frame) {
        let free = app.animation.key_at(bone, frame).is_none();
        if free {
            let key: HashSet<KeyId> = [(bone, point.frame)].into_iter().collect();
            app.touch_keys("Edit key", |a| *a = a.moved(&key, frame - point.frame));
            let graph = &mut app.timeline.graph;
            graph.selected = graph.selected.iter().map(|p| if p.frame == point.frame { Point { frame, ..*p } } else { *p }).collect();
            graph.active = Some(Point { frame, ..point });
            sync_keys(app, bone);
        }
    }
    if let Some(value) = new_value {
        app.touch_keys("Edit key", |a| {
            if let Some(track) = a.tracks.get_mut(&bone) {
                track.set_channel(index, point.channel, value);
            }
        });
    }
    if let Some((right_side, handle)) = new_handle {
        let offset = [handle[0] - point.frame as f32, handle[1] - shown.value];
        let aligned = mode.unwrap_or(true);
        app.touch_keys("Edit handle", |a| {
            if let Some(track) = a.tracks.get_mut(&bone) {
                track.set_handle(index, point.channel, right_side, offset, aligned, smoothing);
            }
        });
    }
}
