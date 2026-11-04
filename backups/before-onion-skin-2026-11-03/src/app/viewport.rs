//! The 3D view: its header, input handling, drawing and overlays.

use egui::{Align, Align2, CentralPanel, Color32, ComboBox, FontId, Frame, Layout, Panel, PointerButton, Pos2, Rect, Sense, Shape, Stroke, Ui, UiBuilder, Vec2, pos2, vec2};
use std::collections::HashMap;
use std::sync::Arc;

use glam::{Mat3, Mat4, Quat, Vec3, Vec4};

use super::chrome::{self, icon_button};
use super::icons::{self, Icon};
use super::{FrameShape, KayeApp, Selection, ToastKind, theme};
use crate::camera::{ThroughView, ViewPreset};
use crate::gizmo::{self, AXIS_COLORS, GizmoView, Handle, Space};
use crate::assets::TextureData;
use crate::clothing::BodyAtlas;
use crate::math::{Xf, ray_box};
use crate::modifiers::{self, IkEnd, ModifierKind};
use crate::rbxmesh::{self, RobloxMesh};
use crate::render::meshes::MeshKind;
use crate::render::{DrawItem, FrameInput, SceneStyle, SkinnedMesh, ViewportRenderer, linear_color, linear_rgb};
use crate::settings::{Action, Shading, ViewportBackground};
use crate::scene::{BoneId, BoneKind, BuiltShape, MeshFit, Part, PartId, PartShape, RigId, RigKind};

const BONE_PICK_DISTANCE: f32 = 8.0;
/// How far the axis balls in the corner sit from its middle, as Blender spaces them.
const AXIS_REACH: f32 = 32.0;
const AXIS_BALL: f32 = 9.0;
const AXIS_MARGIN: f32 = 8.0;
/// Near-black, as Blender letters them. White falls well short of 4.5:1 on the green.
const AXIS_INK: Color32 = Color32::from_rgb(0x12, 0x12, 0x12);
/// How far the drawn camera reaches in front of its lens, in studs.
const CAMERA_WIRE_DEPTH: f32 = 1.4;
const INSET_HEADER: f32 = 22.0;
const INSET_MARGIN: f32 = 10.0;
const INSET_MIN_WIDTH: f32 = 160.0;

// Bones are overlays, so their thickness is set in screen points. They stay slim
// up close and readable from far away.
const LINK_WIDTH: f32 = 1.6;

fn linear(color: Color32) -> Vec4 {
    let [r, g, b, _] = color.to_array();
    linear_color([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0], 1.0)
}

pub fn panel(app: &mut KayeApp, ui: &mut Ui, frame: &mut eframe::Frame) {
    // With the image background, the viewport shows what's behind it.
    let backdrop = if app.settings.viewport.background == ViewportBackground::UiImage && app.has_background() {
        Color32::TRANSPARENT
    } else {
        theme::window()
    };
    CentralPanel::default().frame(Frame::new().fill(backdrop)).show(ui, |ui| {
        Panel::top("viewport_header").exact_size(chrome::STRIP_HEIGHT).frame(chrome::strip_frame()).show(ui, |ui| header(app, ui));
        let rect = ui.available_rect_before_wrap();
        app.viewport_rect = rect;
        scene_view(app, ui, rect, frame);
    });
}

/// The 3D view's own menus, as on a Maya panel, and what affects how it's handled.
fn header(app: &mut KayeApp, ui: &mut Ui) {
    ui.horizontal_centered(|ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            chrome::view_menu(app, ui);
            chrome::shading_menu(app, ui);
            chrome::show_menu(app, ui);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                view_drag(app, ui, Icon::Pan, "Pan", false);
                view_drag(app, ui, Icon::Search, "Zoom", true);
                ui.add_space(8.0);
                ui.visuals_mut().button_frame = true;
                theme::sunken(ui);
                let mut space = app.view.space;
                ComboBox::from_id_salt("transform_space").selected_text(space.label()).width(76.0).show_ui(ui, |ui| {
                    for option in [Space::Global, Space::Local] {
                        ui.selectable_value(&mut space, option, option.label());
                    }
                });
                app.view.space = space;
            });
        });
    });
}

/// A button that's dragged to move the view, for anyone without a middle mouse button.
fn view_drag(app: &mut KayeApp, ui: &mut Ui, icon: Icon, tip: &str, zoom: bool) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::drag());
    let active = response.hovered() || response.dragged();
    if active {
        ui.painter().rect_filled(rect, theme::RADIUS, theme::field_hover());
        ui.ctx().set_cursor_icon(if zoom { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::Grab });
    }
    let color = if active { theme::ink() } else { theme::text() };
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(15.0)), icon, color);
    if response.dragged() {
        let nav = &app.settings.navigation;
        let (delta, zoom_speed, pan_speed) = (response.drag_delta(), nav.zoom_speed, nav.pan_speed);
        // Moving the view means leaving the camera, as in Blender.
        app.camera_view.through = false;
        app.camera.through = None;
        if zoom {
            app.camera.zoom((delta.y * 0.008 * zoom_speed).exp());
        } else {
            app.camera.pan(delta.x * pan_speed, delta.y * pan_speed, app.viewport_rect);
        }
    }
    chrome::tooltip(response, tip);
}

fn scene_view(app: &mut KayeApp, ui: &mut Ui, rect: Rect, frame: &mut eframe::Frame) {
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let ctx = ui.ctx().clone();
    let (dt, mouse, delta, modifiers, scroll) = ctx.input(|i| {
        (
            i.stable_dt,
            i.pointer.latest_pos(),
            i.pointer.delta(),
            i.modifiers,
            i.smooth_scroll_delta.y,
        )
    });
    let hovered = response.hovered();

    app.sync_camera_view(rect);
    camera_input(app, &response, delta, modifiers, scroll, hovered, rect, mouse);
    if app.camera.tick(dt) {
        ctx.request_repaint();
    }

    let mut hot = None;
    if let Some(mouse) = mouse {
        pose_input(app, &ctx, &response, mouse, modifiers, rect, &mut hot);
    }

    viewport_menu(app, &response, rect, mouse);
    want_assets(app);

    let render_state = frame.wgpu_render_state().expect("Kaye needs the wgpu renderer");
    let ppp = ctx.pixels_per_point();
    let input = build_frame(app, rect, ppp);
    let (antialiasing, shadows) = (app.settings.system.antialiasing, app.settings.system.shadow_resolution);
    let Some(renderer) = app.renderer.as_mut() else {
        return;
    };
    renderer.configure(&render_state.device, antialiasing, shadows);
    let texture = renderer.render(render_state, &input);
    // Kept to fill again next frame. A big model's list is megabytes.
    *app.spare_parts.borrow_mut() = input.parts;
    let painter = ui.painter_at(rect);
    painter.image(
        texture,
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );

    paint_cameras(app, &painter, rect);
    paint_camera_roots(app, &painter, rect);
    paint_motion_paths(app, &painter, rect);
    paint_ik_controls(app, &painter, rect);
    paint_camera_frame(app, &painter, rect);
    paint_gizmo(app, &painter, rect, hot, mouse);
    if let (Some(anchor), Some(mouse)) = (app.box_select, mouse) {
        let area = Rect::from_two_pos(anchor, mouse);
        painter.rect_filled(area, 0.0, theme::accent().gamma_multiply(0.14));
        painter.rect_stroke(area, 0.0, Stroke::new(1.0, theme::accent()), egui::StrokeKind::Inside);
    }
    paint_labels(app, &painter, rect);
    if app.settings.interface.show_fps {
        let text = format!("{:.0} fps", 1.0 / dt.max(0.001));
        overlay_text(&painter, rect.right_bottom() - vec2(52.0, 20.0), text, FontId::proportional(11.5), theme::OVERLAY);
        ctx.request_repaint();
    }
    if app.camera.through.is_none() {
        axis_gizmo(app, ui, &painter, rect);
    }
    camera_inset(app, ui, rect, frame);
    paint_toasts(app, &painter, rect);
    drop_overlay(&ctx, &painter, rect);
    if app.scene.is_empty() {
        import_button(app, ui, rect);
    }
    // Maya outlines the panel being worked in.
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, theme::ACTIVE_PANEL), egui::StrokeKind::Inside);
}

#[allow(clippy::too_many_arguments)]
fn camera_input(
    app: &mut KayeApp,
    response: &egui::Response,
    delta: egui::Vec2,
    modifiers: egui::Modifiers,
    scroll: f32,
    hovered: bool,
    rect: Rect,
    mouse: Option<Pos2>,
) {
    let nav = app.settings.navigation.clone();
    let middle = response.dragged_by(PointerButton::Middle);
    let alt_left = nav.alt_left_orbit && modifiers.alt && response.dragged_by(PointerButton::Primary) && app.session.is_none();
    // Moving the view means leaving the camera, as in Blender.
    if app.camera.through.is_some() && (middle || alt_left || (hovered && scroll != 0.0)) {
        app.camera_view.through = false;
        app.camera.through = None;
    }
    if middle || alt_left {
        if modifiers.shift {
            app.camera.pan(delta.x * nav.pan_speed, delta.y * nav.pan_speed, rect);
        } else if modifiers.ctrl {
            app.camera.zoom((delta.y * 0.008 * nav.zoom_speed).exp());
        } else {
            let (dx, dy) = (delta.x * nav.orbit_speed, delta.y * nav.orbit_speed);
            match app.selection_center().filter(|_| nav.orbit_around_selection) {
                Some(pivot) => app.camera.orbit_about(dx, dy, pivot),
                None => app.camera.orbit(dx, dy),
            }
        }
    }
    if hovered && scroll != 0.0 {
        let direction = if nav.invert_zoom { 1.0 } else { -1.0 };
        let factor = (direction * scroll * 0.0035 * nav.zoom_speed).exp();
        match mouse.filter(|_| nav.zoom_to_mouse) {
            Some(mouse) => app.camera.zoom_at(factor, mouse, rect),
            None => app.camera.zoom(factor),
        }
    }
}

fn gizmo_view(app: &KayeApp, rect: Rect) -> Option<GizmoView<'_>> {
    let op = app.tool.op()?;
    if let Some(rig) = app.movable_rig() {
        let shown = app.scene.rigs[rig].visible;
        return shown.then(|| GizmoView {
            camera: &app.camera,
            rect,
            frame: app.scene.rig_frame(rig),
            space: app.view.space,
            op,
            size: app.settings.viewport.gizmo_size,
        });
    }
    let selected = app.selected_bone()?;
    // Nothing done by hand can show on a bone that IK fully decides. A bone at the
    // end of a chain hands its handles to its target instead.
    if modifiers::locked_by_ik(&app.scene, selected) {
        return None;
    }
    let bone = modifiers::control_of(&app.scene, selected);
    if !app.scene.bone_is_visible(bone) {
        return None;
    }
    Some(GizmoView {
        camera: &app.camera,
        rect,
        frame: app.scene.bones[bone].frame,
        space: app.view.space,
        op,
        size: app.settings.viewport.gizmo_size,
    })
}

#[allow(clippy::too_many_arguments)]
fn pose_input(
    app: &mut KayeApp,
    ctx: &egui::Context,
    response: &egui::Response,
    mouse: Pos2,
    modifiers: egui::Modifiers,
    rect: Rect,
    hot: &mut Option<Handle>,
) {
    let (pressed, released, secondary) = ctx.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_released(),
            i.pointer.secondary_pressed(),
        )
    });
    let inside = rect.contains(mouse) && response.hovered();

    if let Some(session) = &mut app.session {
        app.hover = Selection::None;
        session.update(&mut app.scene, &app.camera, rect, mouse);
        let modal = session.modal;
        ctx.request_repaint();
        if secondary {
            app.cancel_session();
            app.swallow_click = false;
            // This right click was for cancelling, not for opening a menu.
            app.suppress_menu = true;
        } else if (modal && pressed) || (!modal && released) {
            app.commit_session();
            app.swallow_click = true;
        }
        return;
    }

    // A drag box in progress owns the mouse until it's released.
    if let Some(anchor) = app.box_select {
        app.hover = Selection::None;
        ctx.request_repaint();
        if released {
            app.box_select = None;
            let bones = bones_in_box(app, rect, Rect::from_two_pos(anchor, mouse));
            app.select_bones(bones, modifiers.shift || modifiers.ctrl);
            app.scroll_to_selection = true;
        }
        return;
    }

    if response.clicked() {
        if app.swallow_click {
            app.swallow_click = false;
        } else if !modifiers.alt {
            let picked = pick(app, rect, mouse);
            app.scroll_to_selection = true;
            match picked {
                Selection::Bone(bone) if modifiers.shift || modifiers.ctrl => app.toggle_bone(bone),
                _ if modifiers.shift || modifiers.ctrl => {}
                other => app.select(other),
            }
        }
        return;
    }
    if released {
        app.swallow_click = false;
    }

    if !inside {
        if !app.outliner_hovering {
            app.hover = Selection::None;
        }
        return;
    }
    if let Some(view) = gizmo_view(app, rect) {
        *hot = view.hit_test(mouse);
    }
    if pressed && !(modifiers.alt && app.settings.navigation.alt_left_orbit) {
        if let Some(handle) = *hot {
            let (op, lock) = handle.session_args(app.view.space);
            if let Some(rig) = app.movable_rig() {
                app.start_rig_session(rig, op, lock, false, mouse);
                return;
            }
            if let Some(bone) = app.selected_bone() {
                app.start_session(bone, op, lock, false, mouse);
                return;
            }
        }
    }
    let orbiting = modifiers.alt && app.settings.navigation.alt_left_orbit;
    if response.drag_started_by(PointerButton::Primary) && !orbiting {
        app.box_select = ctx.input(|i| i.pointer.press_origin()).or(Some(mouse));
        return;
    }
    app.hover = if hot.is_some() { Selection::None } else { pick(app, rect, mouse) };
}

/// The bones a drag box catches: any whose drawn line touches it. With bones
/// hidden, parts stand in for the bones that move them.
fn bones_in_box(app: &KayeApp, rect: Rect, area: Rect) -> Vec<BoneId> {
    let scene = &app.scene;
    let to_screen = app.camera.projector(rect);
    let on_screen = |p: Vec3| to_screen(p).is_some_and(|s| area.contains(s));
    if app.view.show_bones {
        return (0..scene.bones.len())
            .filter(|id| scene.bone_is_visible(*id) && scene.bones[*id].kind != BoneKind::Camera && !modifiers::control_is_idle(scene, *id))
            .filter(|id| {
                let bone = &scene.bones[*id];
                let (head, tail) = (bone.head(), bone.tail());
                (0..=8).any(|i| on_screen(head.lerp(tail, i as f32 / 8.0)))
            })
            .collect();
    }
    let mut bones: Vec<BoneId> = (0..scene.parts.len())
        .filter(|id| scene.part_is_visible(*id) && on_screen(scene.parts[*id].world.pos))
        .filter_map(|id| scene.part_owner(id))
        .collect();
    bones.sort_unstable();
    bones.dedup();
    bones
}

/// What is under the mouse. Bones win because they are drawn over the parts.
pub fn pick(app: &KayeApp, rect: Rect, mouse: Pos2) -> Selection {
    let scene = &app.scene;
    let mut best: Option<(BoneId, f32)> = None;
    let to_screen = app.camera.projector(rect);
    // A root only shows with its camera picked, so it goes ahead of everything else.
    for (rig, _) in shown_roots(app) {
        if to_screen(scene.rigs[rig].placement.pos).is_some_and(|at| at.distance(mouse) < BONE_PICK_DISTANCE) {
            return Selection::Rig(rig);
        }
    }
    for id in 0..scene.bones.len() {
        let kind = scene.bones[id].kind;
        let shown = match kind {
            BoneKind::Camera => app.camera.through.is_none() || app.active_camera() != Some(id),
            // Clicked on its part, like anything else solid.
            BoneKind::Shape => false,
            // Hidden while its IK is off, so a click can't land on something unseen.
            _ => app.view.show_bones && !modifiers::control_is_idle(scene, id),
        };
        if !shown || !scene.bone_is_visible(id) {
            continue;
        }
        if let Some(d) = gizmo::bone_screen_distance(scene, id, &to_screen, mouse) {
            // Controls sit right on top of joints, and would be unreachable otherwise.
            let d = if kind.is_joint() { d } else { d - BONE_PICK_DISTANCE };
            if d < BONE_PICK_DISTANCE && best.is_none_or(|(_, b)| d < b) {
                best = Some((id, d));
            }
        }
    }
    if let Some((id, _)) = best {
        return Selection::Bone(id);
    }
    if app.view.show_parts {
        let ray = app.camera.screen_ray(mouse, rect);
        let mut best: Option<(PartId, f32)> = None;
        let mut moved = HashMap::new();
        for id in 0..scene.parts.len() {
            if !scene.part_is_visible(id) {
                continue;
            }
            if let Some(t) = part_ray_hit(app, id, &ray, &mut moved) {
                if best.is_none_or(|(_, b)| t < b) {
                    best = Some((id, t));
                }
            }
        }
        if let Some((part, _)) = best {
            // Clicking an arm means "the arm's bone".
            return match scene.part_owner(part) {
                Some(bone) => Selection::Bone(bone),
                None => Selection::Part(part),
            };
        }
    }
    Selection::None
}

/// Where lines are seen from: a direction for an orthographic view, or the eye. Worked out
/// once a frame, since every bone has a line.
#[derive(Clone, Copy)]
enum LineView {
    Ortho(Vec3),
    Eye(Vec3),
}

impl LineView {
    fn of(camera: &crate::camera::OrbitCamera) -> Self {
        if camera.is_ortho() { LineView::Ortho(camera.to_camera()) } else { LineView::Eye(camera.eye()) }
    }
}

/// A flat strip from `a` to `b` that always faces the camera. Having no sides or
/// roll of its own, it can't flicker as the rig moves, unlike a thin 3D box.
fn line_strip(view: LineView, a: Vec3, b: Vec3, width: f32) -> Option<Mat4> {
    let span = b - a;
    let length = span.length();
    let y = span.try_normalize()?;
    let toward = match view {
        LineView::Ortho(toward) => toward,
        LineView::Eye(eye) => (eye - (a + b) * 0.5).try_normalize()?,
    };
    let x = y.cross(toward).try_normalize()?;
    let z = x.cross(y);
    Some(Mat4::from_cols(
        (x * width).extend(0.0),
        (y * length).extend(0.0),
        z.extend(0.0),
        a.extend(1.0),
    ))
}

/// Asks for every mesh and texture on show. Each one only ever starts once.
fn want_assets(app: &mut KayeApp) {
    let colors = app.settings.viewport.part_colors;
    // Most parts in a big model have nothing to fetch, and a part's assets never change once
    // it's in, so the ones that do are only listed again when parts are added.
    let count = app.scene.parts.len();
    if app.asset_parts.as_ref().is_none_or(|(of, _)| *of != count) {
        let parts = &app.scene.parts;
        let list = (0..count).filter(|p| parts[*p].mesh_asset.is_some() || parts[*p].texture_asset.is_some()).collect();
        app.asset_parts = Some((count, list));
    }
    let Some((_, with_assets)) = &app.asset_parts else {
        return;
    };
    for &id in with_assets {
        if !app.scene.part_is_visible(id) {
            continue;
        }
        let part = &app.scene.parts[id];
        if let Some(mesh) = part.mesh_asset.as_deref().and_then(rbxmesh::asset_id) {
            app.assets.mesh(mesh);
        }
        if let Some(texture) = part.texture_asset.as_deref().filter(|_| colors) {
            app.assets.texture_named(texture);
        }
    }
    if !colors {
        return;
    }
    for rig in 0..app.scene.rigs.len() {
        let (shirt, pants) = rig_clothing(app, rig);
        if let Some(id) = shirt {
            app.assets.texture(id);
        }
        if let Some(id) = pants {
            app.assets.texture(id);
        }
        if let (Some(shirt), Some(pants)) = (shirt, pants) {
            app.assets.layered(shirt, pants);
        }
        if shirt.is_some() || pants.is_some() {
            for atlas in [BodyAtlas::Torso, BodyAtlas::RightArm, BodyAtlas::LeftArm, BodyAtlas::RightLeg, BodyAtlas::LeftLeg] {
                app.assets.dress(atlas, shirt, pants);
            }
        }
    }
}

fn rig_clothing(app: &KayeApp, rig: RigId) -> (Option<u64>, Option<u64>) {
    let rig = &app.scene.rigs[rig];
    (rig.shirt.as_deref().and_then(rbxmesh::asset_id), rig.pants.as_deref().and_then(rbxmesh::asset_id))
}

/// What a model calls a texture, as the id the store keeps it under.
fn texture_id(text: &str) -> Option<u64> {
    rbxmesh::asset_id(text).or_else(|| rbxmesh::shipped_file(text).map(crate::assets::shipped_id))
}

/// Which side of the body a part is on, and what it wears, for a part drawn as a box.
/// Covers R6 and R15 names.
fn body_piece(name: &str) -> Option<(MeshKind, bool, bool)> {
    let side = if name.starts_with("Left") { MeshKind::ClothedLeftLimb } else { MeshKind::ClothedRightLimb };
    match name {
        "Torso" | "LowerTorso" | "UpperTorso" => Some((MeshKind::ClothedTorso, true, true)),
        _ if name.ends_with("Arm") || name.ends_with("Hand") => Some((side, true, false)),
        _ if name.ends_with("Leg") || name.ends_with("Foot") => Some((side, false, true)),
        _ => None,
    }
}

/// The texture a body part gets from what the character's wearing. The torso can show
/// both, with the shirt over the pants.
fn worn_texture(app: &KayeApp, rig: RigId, wears_shirt: bool, wears_pants: bool) -> Option<u64> {
    let (shirt, pants) = rig_clothing(app, rig);
    let (shirt, pants) = (shirt.filter(|_| wears_shirt), pants.filter(|_| wears_pants));
    let both = shirt.zip(pants).map(|(s, p)| s ^ p.rotate_left(17) | 1 << 63);
    both.filter(|id| app.assets.loaded_texture(*id).is_some()).or(shirt.filter(|id| app.assets.loaded_texture(*id).is_some())).or(pants.filter(|id| app.assets.loaded_texture(*id).is_some()))
}

/// What the parts need from the renderer this frame.
#[derive(Default)]
struct PartDraws {
    items: Vec<DrawItem>,
    meshes: Vec<(u64, Arc<RobloxMesh>)>,
    textures: Vec<(u64, Arc<TextureData>)>,
    skinned: Vec<SkinnedMesh>,
}

/// Places a mesh's own space inside its part.
fn mesh_fit(part: &Part, mesh: &RobloxMesh) -> Mat4 {
    match part.mesh_fit {
        MeshFit::Size => {
            let (center, size) = mesh.bounds();
            Mat4::from_scale(part.size / size) * Mat4::from_translation(-center)
        }
        MeshFit::Scaled { scale, offset } | MeshFit::Built { scale, offset, .. } => Mat4::from_translation(offset) * Mat4::from_scale(scale),
    }
}

/// How far along a ray a part is hit. A real mesh is tried by its triangles, so a click
/// through a gap in it reaches what's behind. Anything else goes by the box it's drawn in.
fn part_ray_hit(app: &KayeApp, id: PartId, ray: &crate::camera::Ray, moved: &mut HashMap<RigId, HashMap<String, Xf>>) -> Option<f32> {
    let part = &app.scene.parts[id];
    let inverse = part.world.inverse();
    let (origin, dir) = (inverse.point(ray.origin), inverse.rot * ray.dir);
    let Some((_, mesh)) = part_mesh(app, part) else {
        let (center, size) = part.drawn_box();
        return ray_box(origin - center, dir, size * 0.5);
    };
    let fit = mesh_fit(part, mesh);
    let moves = (!mesh.weights.is_empty()).then(|| bone_moves(app, part.rig, mesh, moved)).flatten();
    if let Some(moves) = moves {
        let rest = app.rest_pose().parts[id];
        let bent = mesh.skinned(rest.matrix() * fit, &moves, part.world * rest.inverse());
        return mesh.ray_hit_bent(&bent, ray.origin, ray.dir);
    }
    let to_mesh = fit.inverse();
    mesh.ray_hit(to_mesh.transform_point3(origin), to_mesh.transform_vector3(dir))
}

fn part_mesh<'a>(app: &'a KayeApp, part: &Part) -> Option<(u64, &'a Arc<RobloxMesh>)> {
    let id = part.mesh_asset.as_deref().and_then(rbxmesh::asset_id)?;
    app.assets.loaded_mesh(id).map(|mesh| (id, mesh))
}

/// How far each of a mesh's bones has moved from rest, found by name among the rig's
/// bones, or its parts for a mesh skinned to a character's body. Returns nothing if
/// none of them are in the rig, which leaves the mesh rigid.
fn bone_moves(app: &KayeApp, rig: RigId, mesh: &RobloxMesh, names: &mut HashMap<RigId, HashMap<String, Xf>>) -> Option<Vec<Option<Xf>>> {
    let scene = &app.scene;
    let moved = names.entry(rig).or_insert_with(|| {
        let rest = app.rest_pose();
        let mut moved = HashMap::new();
        for id in &scene.rigs[rig].parts {
            moved.insert(scene.parts[*id].name.clone(), scene.parts[*id].world * rest.parts[*id].inverse());
        }
        for id in &scene.rigs[rig].bones {
            if scene.bones[*id].kind == BoneKind::Skinned {
                moved.insert(scene.bones[*id].name.clone(), scene.bones[*id].frame * rest.bones[*id].inverse());
            }
        }
        moved
    });
    let moves: Vec<Option<Xf>> = mesh.bones.iter().map(|name| moved.get(name).copied()).collect();
    moves.iter().any(Option::is_some).then_some(moves)
}

/// The rig's parts, ready to draw. `plain` leaves off every highlight, for a camera
/// showing the scene as it really is.
fn part_items(app: &KayeApp, plain: bool) -> PartDraws {
    let scene = &app.scene;
    let hovered_bone = match app.hover {
        Selection::Bone(b) => Some(b),
        _ => None,
    };
    let selected_part = match app.selection {
        Selection::Part(p) => Some(p),
        _ => None,
    };
    let selected_rig = match app.selection {
        Selection::Rig(r) => Some(r),
        _ => None,
    };
    let accent = linear(theme::accent());
    let accent_soft = linear(theme::accent_soft());
    let look = &app.settings.viewport;
    let shaded = look.shading == Shading::Shaded;
    // Roblox parts have no outlines, so the shaded mode leaves them off.
    let plain_edge = Vec4::new(0.0, 0.0, 0.0, if shaded || plain { 0.0 } else { look.part_edges });
    let clay = look.clay.map(|c| c as f32 / 255.0);

    let mut draws = PartDraws::default();
    let mut bones_by_rig = HashMap::new();
    let mut parts = std::mem::take(&mut *app.spare_parts.borrow_mut());
    parts.clear();
    parts.reserve(scene.parts.len());
    let mut last_color: Option<([f32; 3], f32, Vec4)> = None;
    if app.view.show_parts || plain {
        for (id, p) in scene.parts.iter().enumerate() {
            if !scene.part_is_visible(id) {
                continue;
            }
            let owner = scene.part_owner(id);
            let is_selected = owner.is_some_and(|b| app.is_bone_selected(b))
                || selected_part == Some(id)
                || selected_rig == Some(p.rig);
            let is_hovered = hovered_bone.is_some_and(|b| owner == Some(b));
            // A spawned shape's color was picked on purpose, so it always shows.
            let rgb = if look.part_colors || scene.rigs[p.rig].kind == RigKind::Shape { p.color } else { clay };
            let edge = if plain {
                plain_edge
            } else if is_selected {
                accent
            } else if is_hovered {
                accent_soft
            } else {
                plain_edge
            };
            // Neighboring parts usually share a color, and converting one costs three powers.
            let color = match last_color {
                Some((was, alpha, color)) if was == rgb && alpha == p.transparency => color,
                _ => {
                    let color = linear_color(rgb, 1.0 - p.transparency);
                    last_color = Some((rgb, p.transparency, color));
                    color
                }
            };
            let asset = part_mesh(app, p);
            let worn = look.part_colors.then(|| body_piece(&p.name)).flatten();
            let own = p.texture_asset.as_deref().filter(|_| look.part_colors).and_then(texture_id);
            // A real body mesh's UVs point into an atlas, not at the template. An R6 mesh
            // has an atlas of its own that isn't drawn yet, so it wears nothing.
            let dressed = match asset {
                Some(_) if look.part_colors => BodyAtlas::of_part(&p.name).and_then(|atlas| {
                    let (shirt, pants) = rig_clothing(app, p.rig);
                    app.assets.dressed(atlas, shirt, pants)
                }),
                Some(_) => None,
                None => worn.and_then(|(_, shirt, pants)| worn_texture(app, p.rig, shirt, pants)),
            };
            let texture = own.filter(|t| app.assets.loaded_texture(*t).is_some()).or(dressed);
            let texture = texture.and_then(|t| app.assets.loaded_texture(t).map(|data| (t, data)));
            if let Some((id, data)) = texture {
                if !draws.textures.iter().any(|(t, _)| *t == id) {
                    draws.textures.push((id, data.clone()));
                }
            }
            let texture = texture.map(|(id, _)| id);
            if let Some((asset, mesh)) = asset {
                if !draws.meshes.iter().any(|(m, _)| *m == asset) {
                    draws.meshes.push((asset, mesh.clone()));
                }
                // A real mesh only lights up when it's picked out, so it isn't left
                // looking dirty the way an outline on a curved surface does.
                let edge = if edge == plain_edge { Vec4::ZERO } else { edge };
                let moves = (!mesh.weights.is_empty()).then(|| bone_moves(app, p.rig, mesh, &mut bones_by_rig)).flatten();
                let fit = mesh_fit(p, mesh);
                if let Some(moves) = moves {
                    let rest = app.rest_pose().parts[id];
                    let vertices = mesh.skinned(rest.matrix() * fit, &moves, p.world * rest.inverse());
                    parts.push(DrawItem { mesh: MeshKind::Skinned(draws.skinned.len()), texture, model: Mat4::IDENTITY, color, edge });
                    draws.skinned.push(SkinnedMesh { asset, vertices });
                } else {
                    parts.push(DrawItem { mesh: MeshKind::Asset(asset), texture, model: p.world.matrix() * fit, color, edge });
                }
                continue;
            }
            let mesh = match (p.mesh_fit, p.shape) {
                (MeshFit::Built { shape, .. }, _) => match shape {
                    BuiltShape::Head => MeshKind::Head,
                    BuiltShape::Block => MeshKind::Cube,
                    BuiltShape::Sphere => MeshKind::Sphere,
                    BuiltShape::Cylinder => MeshKind::Cylinder,
                    BuiltShape::Wedge => MeshKind::Wedge,
                    BuiltShape::CornerWedge => MeshKind::CornerWedge,
                },
                (_, PartShape::Block) if texture.is_some() => worn.map_or(MeshKind::Cube, |(kind, _, _)| kind),
                (_, PartShape::Block) => MeshKind::Cube,
                (_, PartShape::Ball) => MeshKind::Sphere,
                (_, PartShape::Cylinder) => MeshKind::Cylinder,
                (_, PartShape::Wedge) => MeshKind::Wedge,
                (_, PartShape::CornerWedge) => MeshKind::CornerWedge,
            };
            // Rounded like a real mesh, so it only lights up when it's picked out too.
            let edge = if mesh == MeshKind::Head && edge == plain_edge { Vec4::ZERO } else { edge };
            let (center, size) = p.drawn_box();
            let model = Mat4::from_scale_rotation_translation(size, p.world.rot, p.world.point(center));
            parts.push(DrawItem { mesh, texture, model, color, edge });
        }
    }
    draws.items = parts;
    draws
}

fn build_frame(app: &KayeApp, rect: Rect, ppp: f32) -> FrameInput {
    let scene = &app.scene;
    let selected = app.selected_bone();
    let hovered_bone = match app.hover {
        Selection::Bone(b) => Some(b),
        _ => None,
    };
    let accent = linear(theme::accent());
    let accent_soft = linear(theme::accent_soft());
    let look = app.settings.viewport.clone();
    let bone_color = linear_rgb(look.bone_color).extend(1.0);
    let link_color = linear_rgb(look.link_color).extend(1.0);
    let no_edge = Vec4::ZERO;
    let parts = part_items(app, false);

    // Links first, then bones, then joints, so each covers the end of the last.
    let point_scale = app.camera.point_scale(rect);
    let line_view = LineView::of(&app.camera);
    // Every joint is the same size on screen, so one choice covers them all.
    let joint_pixels = look.joint_size * 2.0 * 1.3 * ppp * app.settings.system.render_scale;
    let joint_mesh = if joint_pixels <= crate::render::meshes::JOINT_MAX_PIXELS { MeshKind::Joint } else { MeshKind::Sphere };
    let (mut links, mut spindles, mut joints) = (Vec::new(), Vec::new(), Vec::new());
    if app.view.show_bones {
        for (id, b) in scene.bones.iter().enumerate() {
            if !scene.bone_is_visible(id) || !b.kind.is_joint() {
                continue;
            }
            let head = b.head();
            let scale = point_scale(head);
            let is_selected = app.is_bone_selected(id);
            let color = if selected == Some(id) {
                accent
            } else if is_selected {
                // Selected along with the active bone, which keeps the full accent.
                accent_soft
            } else if hovered_bone == Some(id) {
                accent_soft
            } else {
                bone_color
            };

            let length = b.tail_local.length().max(1e-3);
            // Built from the bone's own fixed direction, so it turns rigidly with
            // the joint and never rolls on its own.
            let y_local = b.tail_local / length;
            let x_local = y_local.any_orthonormal_vector();
            let spin = Quat::from_mat3(&Mat3::from_cols(x_local, y_local, x_local.cross(y_local)));
            let width = look.bone_width * scale * if is_selected { 1.3 } else { 1.0 };
            spindles.push(DrawItem {
                mesh: MeshKind::Bone,
                texture: None,
                model: Mat4::from_scale_rotation_translation(
                    Vec3::new(width, length, width),
                    b.frame.rot * spin,
                    head,
                ),
                color,
                edge: no_edge,
            });
            joints.push(DrawItem {
                mesh: joint_mesh,
                texture: None,
                model: Mat4::from_scale_rotation_translation(
                    Vec3::splat(look.joint_size * 2.0 * scale * if is_selected { 1.3 } else { 1.0 }),
                    Quat::IDENTITY,
                    head,
                ),
                color,
                edge: no_edge,
            });

            // A thin line back to the parent shows the hierarchy where bones
            // don't touch, such as clavicles and hips. An unparented bone loses
            // its line, which is how you can see it has let go.
            if let Some(parent) = b.parent.filter(|_| !modifiers::is_unparented(scene, id)) {
                let parent_bone = &scene.bones[parent];
                let to = if b.link_to_parent_tail { parent_bone.tail() } else { parent_bone.head() };
                if let Some(model) = line_strip(line_view, head, to, LINK_WIDTH * scale) {
                    links.push(DrawItem {
                        mesh: MeshKind::Quad,
                        texture: None,
                        model,
                        color: link_color,
                        edge: no_edge,
                    });
                }
            }
        }
    }
    let mut bones = links;
    bones.append(&mut spindles);
    bones.append(&mut joints);

    FrameInput {
        size: [
            (rect.width() * ppp * app.settings.system.render_scale).round() as u32,
            (rect.height() * ppp * app.settings.system.render_scale).round() as u32,
        ],
        view_proj: app.camera.view_proj(rect.aspect_ratio()),
        eye: app.camera.eye(),
        grid_y: app.view.show_grid_floor_y,
        grid_fade: (app.camera.dist * 7.0).max(70.0),
        parts: parts.items,
        bones,
        meshes: parts.meshes,
        textures: parts.textures,
        skinned: parts.skinned,
        bones_on_top: app.view.bones_on_top,
        style: scene_style(app),
    }
}

/// The scene as a camera sees it: parts only, with nothing that isn't in the game.
fn camera_frame(app: &KayeApp, picture: Rect, ppp: f32, view: ThroughView) -> FrameInput {
    let parts = part_items(app, true);
    FrameInput {
        size: [(picture.width() * ppp).round() as u32, (picture.height() * ppp).round() as u32],
        view_proj: view.view_proj(picture.aspect_ratio()),
        eye: view.frame.pos,
        grid_y: app.view.show_grid_floor_y,
        grid_fade: 70.0,
        parts: parts.items,
        bones: Vec::new(),
        meshes: parts.meshes,
        textures: parts.textures,
        skinned: parts.skinned,
        bones_on_top: false,
        style: scene_style(app),
    }
}

/// Where a camera's view is pointing, as the preview needs it.
fn view_of(app: &KayeApp, camera: BoneId) -> ThroughView {
    let bone = &app.scene.bones[camera];
    ThroughView { frame: bone.frame, fov: bone.value.clamp(1.0, 120.0).to_radians() }
}

/// The picture in the corner, showing what the camera sees on this frame.
fn camera_inset(app: &mut KayeApp, ui: &mut Ui, rect: Rect, frame: &mut eframe::Frame) {
    let Some(camera) = app.active_camera().filter(|_| app.camera_view.preview && !app.camera_view.through) else {
        return;
    };
    let shape = app.camera_view.shape;
    let width = app.camera_view.inset_width.clamp(INSET_MIN_WIDTH, (rect.width() - INSET_MARGIN * 2.0).max(INSET_MIN_WIDTH));
    let height = (width / shape.ratio()).min(rect.height() - INSET_HEADER - INSET_MARGIN * 2.0).max(40.0);
    let width = (height * shape.ratio()).min(width);
    let size = vec2(width, height + INSET_HEADER);
    let corner = rect.right_bottom() - size - vec2(INSET_MARGIN, INSET_MARGIN) + app.camera_view.inset_offset;
    let min = pos2(corner.x.clamp(rect.left(), (rect.right() - size.x).max(rect.left())), corner.y.clamp(rect.top(), (rect.bottom() - size.y).max(rect.top())));
    let panel = Rect::from_min_size(min, size);
    let header = Rect::from_min_size(panel.min, vec2(width, INSET_HEADER));
    let picture = Rect::from_min_max(pos2(panel.left(), header.bottom()), panel.max);

    // Claims the whole panel first, so clicks here don't fall through to the rig.
    ui.interact(panel, ui.scope_id().with("camera_inset"), Sense::click());
    let drag = ui.interact(header, ui.scope_id().with("camera_inset_move"), Sense::click_and_drag());
    if drag.dragged() {
        app.camera_view.inset_offset += drag.drag_delta();
    }
    let grip = Rect::from_min_size(panel.min - vec2(4.0, 4.0), egui::Vec2::splat(12.0));
    let resize = ui.interact(grip, ui.scope_id().with("camera_inset_size"), Sense::drag());
    if resize.dragged() {
        app.camera_view.inset_width = (width - resize.drag_delta().x).max(INSET_MIN_WIDTH);
    }

    let view = view_of(app, camera);
    let input = camera_frame(app, picture, ui.ctx().pixels_per_point(), view);
    let render_state = frame.wgpu_render_state().expect("Kaye needs the wgpu renderer");
    let (antialiasing, shadows) = (app.settings.system.antialiasing, app.settings.system.shadow_resolution);
    let renderer = app.preview_renderer.get_or_insert_with(|| ViewportRenderer::new(&render_state.device, antialiasing, shadows));
    renderer.configure(&render_state.device, antialiasing, shadows);
    let texture = renderer.render(render_state, &input);
    *app.spare_parts.borrow_mut() = input.parts;

    let painter = ui.painter_at(rect);
    painter.rect_filled(header, 0.0, theme::header());
    painter.image(texture, picture, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    painter.rect_stroke(panel, 0.0, Stroke::new(1.0, theme::border()), egui::StrokeKind::Outside);
    if resize.hovered() || resize.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
    }

    let mut bar = ui.new_child(UiBuilder::new().max_rect(header.shrink2(vec2(5.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
    bar.spacing_mut().item_spacing.x = 1.0;
    let name = app.scene.bones[camera].name.clone();
    bar.add(egui::Label::new(egui::RichText::new(name).size(11.5).color(theme::text_dim())).truncate().selectable(false));
    bar.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if icon_button(ui, Icon::Close, true, false, "Close").clicked() {
            app.camera_view.preview = false;
        }
        if icon_button(ui, Icon::Frame, true, false, &app.tip(Action::CameraView)).clicked() {
            app.camera_view.through = true;
        }
        let next = FrameShape::ALL[(FrameShape::ALL.iter().position(|s| *s == shape).unwrap_or(0) + 1) % FrameShape::ALL.len()];
        if ui.add(egui::Button::new(egui::RichText::new(shape.label()).size(11.0)).frame(false)).clicked() {
            app.camera_view.shape = next;
        }
    });
}

/// Shades everything outside the camera's frame while looking through it.
fn paint_camera_frame(app: &KayeApp, painter: &egui::Painter, rect: Rect) {
    if app.camera.through.is_none() {
        return;
    }
    let ratio = app.camera_view.shape.ratio();
    let height = (rect.width() / ratio).min(rect.height());
    let inner = Rect::from_center_size(rect.center(), vec2(height * ratio, height));
    let shade = Color32::from_black_alpha(120);
    painter.rect_filled(Rect::from_min_max(rect.min, pos2(rect.right(), inner.top())), 0.0, shade);
    painter.rect_filled(Rect::from_min_max(pos2(rect.left(), inner.bottom()), rect.max), 0.0, shade);
    painter.rect_filled(Rect::from_min_max(pos2(rect.left(), inner.top()), pos2(inner.left(), inner.bottom())), 0.0, shade);
    painter.rect_filled(Rect::from_min_max(pos2(inner.right(), inner.top()), pos2(rect.right(), inner.bottom())), 0.0, shade);
    painter.rect_stroke(inner, 0.0, Stroke::new(1.0, theme::text_dim()), egui::StrokeKind::Outside);
}

/// Scene colors come from Preferences, and only selection follows the theme.
fn overlay_color(app: &KayeApp, bone: BoneId) -> Color32 {
    let [r, g, b] = app.settings.viewport.bone_color;
    if app.selected_bone() == Some(bone) {
        theme::accent()
    } else if app.is_bone_selected(bone) || app.hover == Selection::Bone(bone) {
        theme::accent_soft()
    } else {
        Color32::from_rgb(r, g, b)
    }
}

/// A wire shape for each camera: a box for the frame, lines back to the lens, and a
/// small triangle on top so it's clear which way is up.
fn paint_cameras(app: &KayeApp, painter: &egui::Painter, rect: Rect) {
    let ratio = app.camera_view.shape.ratio();
    for id in app.scene.cameras() {
        if !app.scene.bone_is_visible(id) || (app.camera.through.is_some() && app.active_camera() == Some(id)) {
            continue;
        }
        let bone = &app.scene.bones[id];
        let half_height = CAMERA_WIRE_DEPTH * (bone.value.clamp(1.0, 120.0).to_radians() * 0.5).tan();
        let half_width = half_height * ratio;
        let at = |x: f32, y: f32, z: f32| app.camera.world_to_screen(bone.frame.point(Vec3::new(x, y, z)), rect);
        let corners = [
            at(-half_width, -half_height, -CAMERA_WIRE_DEPTH),
            at(half_width, -half_height, -CAMERA_WIRE_DEPTH),
            at(half_width, half_height, -CAMERA_WIRE_DEPTH),
            at(-half_width, half_height, -CAMERA_WIRE_DEPTH),
        ];
        let (Some(lens), [Some(a), Some(b), Some(c), Some(d)]) = (at(0.0, 0.0, 0.0), corners) else {
            continue;
        };
        let stroke = Stroke::new(if app.is_bone_selected(id) { 1.8 } else { 1.2 }, overlay_color(app, id));
        for (from, to) in [(a, b), (b, c), (c, d), (d, a), (lens, a), (lens, b), (lens, c), (lens, d)] {
            painter.line_segment([from, to], stroke);
        }
        if let Some(top) = at(0.0, half_height * 1.35, -CAMERA_WIRE_DEPTH) {
            let left = at(-half_width * 0.4, half_height * 1.05, -CAMERA_WIRE_DEPTH).unwrap_or(c);
            let right = at(half_width * 0.4, half_height * 1.05, -CAMERA_WIRE_DEPTH).unwrap_or(d);
            painter.add(Shape::convex_polygon(vec![top, right, left], stroke.color, Stroke::NONE));
        }
    }
}

/// The cameras whose root is drawn: any picked, or whose root is picked or under the
/// mouse. Drawn for every camera, roots would crowd the view.
fn shown_roots(app: &KayeApp) -> Vec<(RigId, BoneId)> {
    let scene = &app.scene;
    scene
        .cameras()
        .map(|camera| (scene.bones[camera].rig, camera))
        .filter(|(rig, camera)| scene.bone_is_visible(*camera) && (app.is_bone_selected(*camera) || app.is_selected(Selection::Rig(*rig)) || app.hover == Selection::Rig(*rig)))
        .collect()
}

/// A camera's root, where it stands in Roblox: a one-stud square on the root's floor, and
/// a dashed line up to the lens.
fn paint_camera_roots(app: &KayeApp, painter: &egui::Painter, rect: Rect) {
    let [r, g, b] = app.settings.viewport.bone_color;
    for (rig, camera) in shown_roots(app) {
        let root = app.scene.rigs[rig].placement;
        let color = if app.selection == Selection::Rig(rig) {
            theme::accent()
        } else if app.is_selected(Selection::Rig(rig)) || app.hover == Selection::Rig(rig) {
            theme::accent_soft()
        } else {
            Color32::from_rgb(r, g, b)
        };
        let at = |x: f32, z: f32| app.camera.world_to_screen(root.point(Vec3::new(x, 0.0, z)), rect);
        let corners = [at(-0.5, -0.5), at(0.5, -0.5), at(0.5, 0.5), at(-0.5, 0.5)];
        if let [Some(a), Some(b), Some(c), Some(d)] = corners {
            painter.add(Shape::closed_line(vec![a, b, c, d], Stroke::new(1.6, color)));
        }
        let (lens, middle) = (app.camera.world_to_screen(app.scene.bones[camera].frame.pos, rect), app.camera.world_to_screen(root.pos, rect));
        if let (Some(lens), Some(middle)) = (lens, middle) {
            painter.extend(Shape::dashed_line(&[middle, lens], Stroke::new(1.0, color.gamma_multiply(0.7)), 5.0, 4.0));
            painter.circle_filled(middle, 2.5, color);
        }
    }
}

fn paint_diamond(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32) {
    let points = vec![center - vec2(0.0, radius), center + vec2(radius, 0.0), center + vec2(0.0, radius), center - vec2(radius, 0.0)];
    painter.add(Shape::closed_line(points, Stroke::new(1.8, color)));
}

/// Where each bone with a motion path goes over the animation: a line through every frame,
/// a dot on each keyed one, and a ring on the frame showing. The stretch already played
/// is dimmer, as in Blender.
fn paint_motion_paths(app: &KayeApp, painter: &egui::Painter, rect: Rect) {
    let paths = &app.motion_path_points;
    let now = app.frame - paths.start as f32;
    let color = theme::accent();
    let to_screen = app.camera.projector(rect);
    for (bone, points) in &paths.points {
        if !app.motion_paths.contains(bone) || !app.scene.bone_is_visible(*bone) {
            continue;
        }
        let on_screen: Vec<Option<Pos2>> = points.iter().map(|p| to_screen(*p)).collect();
        for (index, ends) in on_screen.windows(2).enumerate() {
            if let [Some(a), Some(b)] = ends {
                let played = (index as f32) < now;
                painter.line_segment([*a, *b], Stroke::new(1.5, if played { color.gamma_multiply(0.45) } else { color }));
            }
        }
        for at in on_screen.iter().flatten() {
            painter.circle_filled(*at, 1.3, color);
        }
        let control = modifiers::control_of(&app.scene, *bone);
        for frame in app.animation.key_frames(&[control]) {
            if let Some(Some(at)) = usize::try_from(frame - paths.start).ok().and_then(|i| on_screen.get(i)) {
                painter.circle_filled(*at, 3.5, theme::OVERLAY);
            }
        }
        if let Some(Some(at)) = usize::try_from(now.round() as i32).ok().and_then(|i| on_screen.get(i)) {
            painter.circle_stroke(*at, 5.0, Stroke::new(1.5, theme::OVERLAY));
        }
    }
}

/// IK targets as diamonds and poles as circles, each with a dashed line back to the
/// joint it's pulling on.
fn paint_ik_controls(app: &KayeApp, painter: &egui::Painter, rect: Rect) {
    if !app.view.show_bones {
        return;
    }
    let scene = &app.scene;
    let to_screen = app.camera.projector(rect);
    let on_screen = |p: Vec3| to_screen(p);
    for modifier in &scene.modifiers {
        if let ModifierKind::LookAt(look) = modifier.kind {
            if modifier.enabled && scene.bone_is_visible(look.target) && !modifiers::control_is_idle(scene, look.target) {
                let color = overlay_color(app, look.target);
                if let (Some(a), Some(b)) = (on_screen(scene.bones[modifier.bone].head()), on_screen(scene.bones[look.target].head())) {
                    painter.extend(Shape::dashed_line(&[a, b], Stroke::new(1.0, color), 5.0, 4.0));
                    paint_diamond(painter, b, 7.0, color);
                }
            }
        }
        let ModifierKind::Ik(ik) = modifier.kind else {
            continue;
        };
        if !modifier.enabled || !scene.bone_is_visible(ik.target) || modifiers::control_is_idle(scene, ik.target) {
            continue;
        }
        let bone = &scene.bones[modifier.bone];
        let end = if ik.end == IkEnd::Joint { bone.head() } else { bone.tail() };
        let target = scene.bones[ik.target].head();
        let color = overlay_color(app, ik.target);
        if let (Some(a), Some(b)) = (on_screen(end), on_screen(target)) {
            if (a - b).length() > 2.0 {
                painter.extend(Shape::dashed_line(&[a, b], Stroke::new(1.0, color), 5.0, 4.0));
            }
            paint_diamond(painter, b, 7.0, color);
        }
        if let Some(pole) = ik.pole {
            let chain = modifiers::chain_of(scene, modifier.bone, &ik);
            let middle = chain.get(chain.len() / 2).map(|b| scene.bones[*b].head());
            let color = overlay_color(app, pole);
            if let (Some(a), Some(b)) = (middle.and_then(on_screen), on_screen(scene.bones[pole].head())) {
                painter.extend(Shape::dashed_line(&[a, b], Stroke::new(1.0, color), 5.0, 4.0));
                painter.circle_stroke(b, 5.0, Stroke::new(1.8, color));
            }
        }
    }
}

/// Everything about the scene's look, taken from Preferences and never the theme.
fn scene_style(app: &KayeApp) -> SceneStyle {
    let look = &app.settings.viewport;
    let light = &app.settings.lighting;
    let shaded = look.shading == Shading::Shaded;
    let (azimuth, elevation) = (light.sun_azimuth.to_radians(), light.sun_elevation.to_radians());
    let sun_dir = Vec3::new(azimuth.sin() * elevation.cos(), elevation.sin(), -azimuth.cos() * elevation.cos());
    let (matrix, lift) = if shaded && light.shadows {
        shadow_view(app, sun_dir)
    } else {
        (None, 0.0)
    };
    let top = linear_rgb(look.background_top);
    let bottom = if look.background == ViewportBackground::Solid { top } else { linear_rgb(look.background_bottom) };
    SceneStyle {
        shaded,
        shadow_matrix: matrix,
        sun_dir,
        sun_color: linear_rgb(light.sun_color) * light.sun_strength,
        sky: linear_rgb(light.sky_ambient),
        ground: linear_rgb(light.ground_ambient),
        shadow_softness: light.shadow_softness,
        shadow_strength: light.shadow_strength,
        floor_shadows: light.floor_shadows,
        shadow_lift: lift,
        background_top: top,
        background_bottom: bottom,
        background_alpha: if look.background == ViewportBackground::UiImage && app.has_background() { 0.0 } else { 1.0 },
        grid_color: linear_rgb(look.grid_color),
        grid_opacity: if look.show_grid { look.grid_opacity } else { 0.0 },
    }
}

/// The sun's view, fitted around the rig and where its shadow lands on the floor.
/// Also returns how far to lift receivers off themselves to avoid shadow acne.
fn shadow_view(app: &KayeApp, sun_dir: Vec3) -> (Option<Mat4>, f32) {
    let Some((min, max)) = app.scene.bounds(None) else {
        return (None, 0.0);
    };
    let floor = app.view.show_grid_floor_y;
    let (mut low, mut high) = (min, max);
    for x in [min.x, max.x] {
        for y in [min.y, max.y] {
            for z in [min.z, max.z] {
                let corner = Vec3::new(x, y, z);
                let along = (corner.y - floor) / sun_dir.y.max(0.05);
                let landed = corner - sun_dir * along;
                low = low.min(landed);
                high = high.max(landed);
            }
        }
    }
    let center = (low + high) * 0.5;
    let radius = (high - low).length() * 0.5 + 1.0;
    let z = sun_dir;
    let x = Vec3::Y.cross(z).try_normalize().unwrap_or(Vec3::X);
    let y = z.cross(x);
    let eye = center + z * radius * 2.0;
    let view = Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), eye.extend(1.0)).inverse();
    let projection = glam::camera::rh::proj::directx::orthographic(-radius, radius, -radius, radius, 0.0, radius * 4.0);
    let texel = radius * 2.0 / app.settings.system.shadow_resolution.max(1) as f32;
    (Some(projection * view), texel * 1.5)
}

fn paint_gizmo(app: &KayeApp, painter: &egui::Painter, rect: Rect, hot: Option<Handle>, mouse: Option<Pos2>) {
    if let Some(session) = &app.session {
        if let Some(mouse) = mouse {
            gizmo::paint_session_guides(painter, session, &app.camera, rect, mouse);
        }
        return;
    }
    if let Some(view) = gizmo_view(app, rect) {
        view.paint(painter, hot);
    }
}

/// Text over the 3D view, with a shadow so it reads on any background or rig.
fn overlay_text(painter: &egui::Painter, at: Pos2, text: String, font: FontId, color: Color32) {
    let galley = painter.layout_no_wrap(text, font, color);
    painter.galley_with_override_text_color(at + vec2(1.0, 1.0), galley.clone(), Color32::from_black_alpha(170));
    painter.galley(at, galley, color);
}

/// The view's name along the bottom edge, where Maya puts it, and a running readout
/// while something is being moved.
fn paint_labels(app: &KayeApp, painter: &egui::Painter, rect: Rect) {
    let through = app.active_camera().filter(|_| app.camera.through.is_some());
    let name = through.map_or_else(|| app.camera.label().map(str::to_string), |camera| Some(app.scene.bones[camera].name.clone()));
    if let Some(name) = name {
        let width = painter.layout_no_wrap(name.clone(), FontId::proportional(12.0), theme::OVERLAY).size().x;
        overlay_text(painter, pos2(rect.center().x - width * 0.5, rect.bottom() - 21.0), name, FontId::proportional(12.0), theme::OVERLAY);
    }
    if let Some(session) = &app.session {
        overlay_text(painter, rect.left_top() + vec2(10.0, 8.0), session.readout(), FontId::proportional(12.0), theme::OVERLAY);
    }
}

/// One end of an axis on the corner gizmo.
struct AxisBall {
    axis: usize,
    positive: bool,
    at: Pos2,
    /// How much it faces the viewer, from -1 to 1.
    facing: f32,
}

/// The axes in the top right corner, drawn as Blender draws them: a solid ball with a
/// letter on each positive end, and a hollow one on each negative end. Click a ball for
/// that view, and click it again for the opposite one, or drag to orbit.
fn axis_gizmo(app: &mut KayeApp, ui: &mut Ui, painter: &egui::Painter, rect: Rect) {
    let outer = AXIS_REACH + AXIS_BALL;
    let center = rect.right_top() + vec2(-(outer + AXIS_MARGIN), outer + AXIS_MARGIN);
    let area = Rect::from_center_size(center, Vec2::splat(outer * 2.0));
    if !rect.contains_rect(area) {
        return;
    }
    let response = ui.interact(area, ui.scope_id().with("axis_gizmo"), Sense::click_and_drag());
    let free = app.session.is_none();
    if response.dragged() && free {
        app.camera.orbit(response.drag_delta().x, response.drag_delta().y);
    }

    let (right, up, toward) = (app.camera.right(), app.camera.up(), app.camera.to_camera());
    let mut balls: Vec<AxisBall> = [Vec3::X, Vec3::Y, Vec3::Z]
        .into_iter()
        .enumerate()
        .flat_map(|(axis, dir)| [(axis, true, dir), (axis, false, -dir)])
        .map(|(axis, positive, dir)| AxisBall { axis, positive, at: center + vec2(dir.dot(right), -dir.dot(up)) * AXIS_REACH, facing: dir.dot(toward) })
        .collect();
    balls.sort_by(|a, b| a.facing.total_cmp(&b.facing));

    // The nearest ball under the mouse, since the far ones are drawn behind it.
    let pointer = response.hover_pos();
    let hot = balls.iter().rposition(|b| pointer.is_some_and(|p| (p - b.at).length() <= AXIS_BALL));
    if free && (response.hovered() || response.dragged()) {
        painter.circle_filled(center, outer, Color32::from_white_alpha(18));
    }
    for (index, ball) in balls.iter().enumerate() {
        let is_hot = free && hot == Some(index);
        // Farther balls sink back into the dark behind them.
        let [r, g, b, _] = AXIS_COLORS[ball.axis].to_array();
        let depth = 0.72 + 0.28 * (ball.facing + 1.0) * 0.5;
        let shade = |c: u8| (c as f32 * depth) as u8;
        let color = Color32::from_rgb(shade(r), shade(g), shade(b));
        if ball.positive {
            painter.line_segment([center, ball.at], Stroke::new(2.0, color));
        }
        let name = ["X", "Y", "Z"][ball.axis];
        let font = theme::strong_font(11.0);
        if ball.positive || is_hot {
            let fill = if is_hot { theme::lighter(color) } else { color };
            painter.circle_filled(ball.at, AXIS_BALL, fill);
            let text = if ball.positive { name.to_string() } else { format!("-{name}") };
            painter.text(ball.at, Align2::CENTER_CENTER, text, font, AXIS_INK);
        } else {
            painter.circle_filled(ball.at, AXIS_BALL, color.gamma_multiply(0.22));
            painter.circle_stroke(ball.at, AXIS_BALL - 0.75, Stroke::new(1.5, color));
        }
    }
    if let Some(ball) = hot.filter(|_| response.clicked() && free).map(|index| &balls[index]) {
        let preset = preset_for(ball.axis, ball.positive);
        let preset = if app.camera.preset == Some(preset) { preset_for(ball.axis, !ball.positive) } else { preset };
        app.camera.set_preset(preset);
    }
}

/// The view that puts this axis end facing the viewer.
fn preset_for(axis: usize, positive: bool) -> ViewPreset {
    match (axis, positive) {
        (0, true) => ViewPreset::Right,
        (0, false) => ViewPreset::Left,
        (1, true) => ViewPreset::Top,
        (1, false) => ViewPreset::Bottom,
        (_, true) => ViewPreset::Back,
        (_, false) => ViewPreset::Front,
    }
}

pub fn paint_toasts(app: &KayeApp, painter: &egui::Painter, rect: Rect) {
    let mut y = rect.bottom() - 12.0;
    for toast in app.toasts.iter().rev().take(5) {
        let color = match toast.kind {
            ToastKind::Warning => theme::WARNING,
            ToastKind::Error => theme::ERROR,
        };
        let galley = painter.layout(
            toast.text.clone(),
            FontId::proportional(12.5),
            color,
            (rect.width() - 60.0).max(120.0),
        );
        let size = galley.size() + vec2(20.0, 10.0);
        let box_rect = Rect::from_min_size(pos2(rect.left() + 10.0, y - size.y), size);
        painter.rect_filled(box_rect, theme::SECTION_RADIUS, theme::panel());
        painter.rect_stroke(box_rect, theme::SECTION_RADIUS, Stroke::new(1.0, color.gamma_multiply(0.5)), egui::StrokeKind::Inside);
        painter.galley(box_rect.min + vec2(10.0, 5.0), galley, color);
        y -= size.y + 4.0;
    }
}

pub fn drop_overlay(ctx: &egui::Context, painter: &egui::Painter, rect: Rect) {
    if ctx.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    painter.rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(2.0, theme::accent()), egui::StrokeKind::Inside);
}

fn import_button(app: &mut KayeApp, ui: &mut Ui, rect: Rect) {
    let button = Rect::from_center_size(rect.center(), vec2(120.0, 32.0));
    let response = ui.interact(button, ui.scope_id().with("import_empty"), Sense::click());
    let fill = if response.hovered() { theme::field_active() } else { theme::field_hover() };
    ui.painter().rect_filled(button, theme::SECTION_RADIUS, fill);
    let icon = Rect::from_center_size(button.left_center() + vec2(18.0, 0.0), egui::Vec2::splat(15.0));
    icons::paint(ui.painter(), icon, Icon::Import, theme::ink());
    ui.painter().text(
        button.left_center() + vec2(33.0, 0.0),
        Align2::LEFT_CENTER,
        "Import Rig",
        FontId::proportional(12.5),
        theme::ink(),
    );
    if response.clicked() {
        app.open_import_dialog(ui.ctx());
    }
}

/// Right click in the viewport acts on whatever is under the mouse.
fn viewport_menu(app: &mut KayeApp, response: &egui::Response, rect: Rect, mouse: Option<Pos2>) {
    if app.session.is_some() {
        return;
    }
    if app.suppress_menu {
        if response.ctx.input(|i| !i.pointer.secondary_down()) {
            app.suppress_menu = false;
        }
        return;
    }
    if response.secondary_clicked() {
        if let Some(mouse) = mouse {
            let picked = pick(app, rect, mouse);
            // Right clicking inside a selection keeps it, so the menu acts on all of it.
            let inside = matches!(picked, Selection::Bone(b) if app.is_bone_selected(b));
            if !inside {
                app.scroll_to_selection = true;
                app.select(picked);
            }
        }
    }
    response.context_menu(|ui| {
        chrome::menu_rows(ui);
        if app.selected_bone().is_some() {
            bone_menu(app, ui);
            return;
        }
        if ui.button("Frame All").clicked() {
            app.frame_all();
            ui.close();
        }
        for preset in [ViewPreset::Front, ViewPreset::Right, ViewPreset::Top] {
            if ui.button(format!("{} View", preset.label())).clicked() {
                app.camera.set_preset(preset);
                ui.close();
            }
        }
    });
}

fn camera_menu(app: &mut KayeApp, ui: &mut Ui, camera: BoneId) {
    if ui.add(egui::Button::new("Camera View").shortcut_text(app.key_text(Action::CameraView))).clicked() {
        app.camera_view.camera = Some(camera);
        app.toggle_camera_view();
        ui.close();
    }
    if ui.button("Align to View").clicked() {
        app.camera_to_view(camera);
        ui.close();
    }
    ui.separator();
    if ui.button("Delete").clicked() {
        app.delete_object(camera);
        ui.close();
    }
    ui.separator();
}

/// Actions for the selected bone, shared by the viewport and the outliner.
pub fn bone_menu(app: &mut KayeApp, ui: &mut Ui) {
    let selected = app.selected_bone();
    if let Some(camera) = selected.filter(|b| app.scene.bones[*b].kind == BoneKind::Camera) {
        camera_menu(app, ui, camera);
    }
    if let Some(shape) = selected.filter(|b| app.scene.bones[*b].kind == BoneKind::Shape) {
        if ui.button("Delete").clicked() {
            app.delete_object(shape);
            ui.close();
        }
        ui.separator();
    }
    if selected.is_some_and(|b| app.scene.bones[b].kind.is_joint() || app.scene.bones[b].kind.is_control()) {
        if ui.add(egui::Button::new("Delete").shortcut_text(app.key_text(Action::Delete))).clicked() {
            app.delete_selection();
            ui.close();
            return;
        }
        ui.separator();
    }
    if selected.is_some() {
        let bones = app.selected_bones();
        let has_path = bones.iter().any(|b| app.motion_paths.contains(b));
        let all_have_path = bones.iter().all(|b| app.motion_paths.contains(b));
        if ui.add_enabled(!all_have_path, egui::Button::new("Create Motion Path")).clicked() {
            app.create_motion_paths();
            ui.close();
        }
        if ui.add_enabled(has_path, egui::Button::new("Clear Motion Path")).clicked() {
            app.clear_motion_paths();
            ui.close();
        }
        ui.separator();
    }
    if let Some(bone) = selected.filter(|b| app.scene.bones[*b].kind.is_joint()) {
        chrome::menu(ui, "Add Modifier", |ui| super::side::add_modifier_items(app, ui, bone));
        if app.has_ik(bone) {
            let on = app.ik_is_on(bone);
            if ui.button(if on { "Switch to FK" } else { "Switch to IK" }).clicked() {
                app.switch_ik(bone, !on);
                ui.close();
            }
        }
        if app.has_unparent(bone) {
            let on = app.unparent_is_on(bone);
            if ui.button(if on { "Parent" } else { "Unparent" }).clicked() {
                app.switch_unparent(bone, !on);
                ui.close();
            }
        }
        ui.separator();
        if ui.add(egui::Button::new("Copy Pose").shortcut_text(app.key_text(Action::CopyPose))).clicked() {
            app.copy_pose(ui.ctx());
            ui.close();
        }
        if ui.add_enabled(!app.pose_clipboard.is_empty(), egui::Button::new("Paste Pose").shortcut_text(app.key_text(Action::PastePose))).clicked() {
            app.paste_pose();
            ui.close();
        }
        if ui.add_enabled(!app.pose_clipboard.is_empty(), egui::Button::new("Paste Mirrored Pose").shortcut_text(app.key_text(Action::PasteMirrored))).clicked() {
            app.paste_pose_mirrored();
            ui.close();
        }
        if ui.add(egui::Button::new("Mirror Pose").shortcut_text(app.key_text(Action::MirrorPose))).clicked() {
            app.mirror_pose();
            ui.close();
        }
        ui.separator();
    }
    let frame = app.whole_frame();
    let keyed = app.selected_bone().is_some_and(|b| app.animation.key_at(b, frame).is_some());
    if ui.add(egui::Button::new("Insert Keyframe").shortcut_text(app.key_text(Action::InsertKey))).clicked() {
        app.insert_key();
        ui.close();
    }
    if ui.add_enabled(keyed, egui::Button::new("Delete Keyframe").shortcut_text(app.key_text(Action::DeleteKey))).clicked() {
        app.delete_keys_at_frame();
        ui.close();
    }
    ui.separator();
    if ui.add(egui::Button::new("Clear Position").shortcut_text(app.key_text(Action::ClearPosition))).clicked() {
        app.reset_selected(true, false);
        ui.close();
    }
    if ui.add(egui::Button::new("Clear Rotation").shortcut_text(app.key_text(Action::ClearRotation))).clicked() {
        app.reset_selected(false, true);
        ui.close();
    }
    if ui.button("Clear Pose").clicked() {
        app.reset_selected(true, true);
        ui.close();
    }
    ui.separator();
    if ui.add(egui::Button::new("Frame").shortcut_text(app.key_text(Action::FrameSelected))).clicked() {
        app.frame_selection();
        ui.close();
    }
}
