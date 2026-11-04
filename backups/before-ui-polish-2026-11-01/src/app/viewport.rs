//! The 3D view: its header, input handling, drawing and overlays.

use egui::{Align, Align2, CentralPanel, Color32, FontId, Frame, Layout, Margin, Panel, PointerButton, Pos2, Rect, Sense, Shape, Stroke, Ui, UiBuilder, pos2, vec2};
use glam::{Mat3, Mat4, Quat, Vec3, Vec4};

use super::chrome::icon_button;
use super::icons::{self, Icon};
use super::{FrameShape, KayeApp, Selection, ToastKind, theme};
use crate::camera::{ThroughView, ViewPreset};
use crate::gizmo::{self, AXIS_COLORS, GizmoView, Handle, Space};
use crate::math::ray_box;
use crate::modifiers::{self, IkEnd, ModifierKind};
use crate::render::meshes::MeshKind;
use crate::render::{DrawItem, FrameInput, SceneStyle, ViewportRenderer, linear_color, linear_rgb};
use crate::settings::{Action, Shading, ViewportBackground};
use crate::scene::{BoneId, BoneKind, PartId, PartShape};

const BONE_PICK_DISTANCE: f32 = 8.0;
const NAV_RADIUS: f32 = 34.0;
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
        Panel::top("viewport_header")
            .exact_size(26.0)
            .frame(Frame::new().fill(theme::header()).inner_margin(Margin::symmetric(4, 0)))
            .show(ui, |ui| header(app, ui));
        let rect = ui.available_rect_before_wrap();
        app.viewport_rect = rect;
        scene_view(app, ui, rect, frame);
    });
}

fn header(app: &mut KayeApp, ui: &mut Ui) {
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        for (shading, icon, tip) in [(Shading::Studio, Icon::Studio, "Studio"), (Shading::Shaded, Icon::Shaded, "Shaded")] {
            if icon_button(ui, icon, true, app.settings.viewport.shading == shading, tip).clicked() {
                app.settings.viewport.shading = shading;
            }
        }
        ui.add_space(10.0);
        for (space, icon) in [(Space::Global, Icon::Global), (Space::Local, Icon::Local)] {
            if icon_button(ui, icon, true, app.view.space == space, space.label()).clicked() {
                app.view.space = space;
            }
        }
        ui.add_space(10.0);
        let v = &mut app.view;
        for (flag, icon, tip) in [
            (&mut v.show_parts, Icon::Part, "Parts"),
            (&mut v.show_bones, Icon::Bone, "Bones"),
            (&mut v.bones_on_top, Icon::OnTop, "Bones on Top"),
            (&mut app.settings.viewport.part_colors, Icon::Colors, "Part Colors"),
        ] {
            if icon_button(ui, icon, true, *flag, tip).clicked() {
                *flag = !*flag;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            let (frame_tip, ortho_tip) = (app.tip(Action::FrameSelected), app.tip(Action::ToggleOrtho));
            if icon_button(ui, Icon::Frame, true, false, &frame_tip).clicked() {
                app.frame_selection();
            }
            if icon_button(ui, Icon::Perspective, true, !app.camera.ortho, &ortho_tip).clicked() {
                app.camera.ortho = !app.camera.ortho;
            }
            ui.add_space(10.0);
            let has_camera = app.active_camera().is_some();
            let (through_tip, preview_tip) = (app.tip(Action::CameraView), app.tip(Action::CameraPreview));
            if icon_button(ui, Icon::Target, has_camera, has_camera && app.camera_view.through, &through_tip).clicked() {
                app.toggle_camera_view();
            }
            if icon_button(ui, Icon::Camera, has_camera, has_camera && app.camera_view.preview, &preview_tip).clicked() {
                app.toggle_camera_preview();
            }
        });
    });
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

    let render_state = frame.wgpu_render_state().expect("Kaye needs the wgpu renderer");
    let ppp = ctx.pixels_per_point();
    let input = build_frame(app, rect, ppp);
    let (antialiasing, shadows) = (app.settings.system.antialiasing, app.settings.system.shadow_resolution);
    let Some(renderer) = app.renderer.as_mut() else {
        return;
    };
    renderer.configure(&render_state.device, antialiasing, shadows);
    let texture = renderer.render(render_state, &input);
    let painter = ui.painter_at(rect);
    painter.image(
        texture,
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );

    paint_cameras(app, &painter, rect);
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
        painter.text(
            rect.right_bottom() - vec2(8.0, 6.0),
            Align2::RIGHT_BOTTOM,
            format!("{:.0} fps", 1.0 / dt.max(0.001)),
            FontId::proportional(11.5),
            theme::text_faint(),
        );
        ctx.request_repaint();
    }
    if app.camera.through.is_none() {
        nav_gizmo(app, ui, &painter, rect);
    }
    camera_inset(app, ui, rect, frame);
    paint_toasts(app, &painter, rect);
    drop_overlay(&ctx, &painter, rect);
    if app.scene.is_empty() {
        import_button(app, ui, rect);
    }
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
        if let (Some(handle), Some(bone)) = (*hot, app.selected_bone()) {
            let (op, lock) = handle.session_args(app.view.space);
            app.start_session(bone, op, lock, false, mouse);
            return;
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
    let on_screen = |p: Vec3| app.camera.world_to_screen(p, rect).is_some_and(|s| area.contains(s));
    if app.view.show_bones {
        return (0..scene.bones.len())
            .filter(|id| scene.bone_is_visible(*id) && scene.bones[*id].kind != BoneKind::Camera)
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
    for id in 0..scene.bones.len() {
        let kind = scene.bones[id].kind;
        let shown = if kind == BoneKind::Camera {
            app.camera.through.is_none() || app.active_camera() != Some(id)
        } else {
            app.view.show_bones
        };
        if !shown || !scene.bone_is_visible(id) {
            continue;
        }
        if let Some(d) = gizmo::bone_screen_distance(scene, id, &app.camera, rect, mouse) {
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
        for (id, part) in scene.parts.iter().enumerate() {
            if !scene.part_is_visible(id) {
                continue;
            }
            let inv = part.world.inverse();
            let origin = inv.point(ray.origin);
            let dir = inv.rot * ray.dir;
            if let Some(t) = ray_box(origin, dir, part.size * 0.5) {
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

/// A flat strip from `a` to `b` that always faces the camera. Having no sides or
/// roll of its own, it can't flicker as the rig moves, unlike a thin 3D box.
fn line_strip(camera: &crate::camera::OrbitCamera, a: Vec3, b: Vec3, width: f32) -> Option<Mat4> {
    let span = b - a;
    let length = span.length();
    let y = span.try_normalize()?;
    let toward = if camera.is_ortho() { camera.to_camera() } else { (camera.eye() - (a + b) * 0.5).try_normalize()? };
    let x = y.cross(toward).try_normalize()?;
    let z = x.cross(y);
    Some(Mat4::from_cols(
        (x * width).extend(0.0),
        (y * length).extend(0.0),
        z.extend(0.0),
        a.extend(1.0),
    ))
}

/// The rig's parts, ready to draw. `plain` leaves off every highlight, for a camera
/// showing the scene as it really is.
fn part_items(app: &KayeApp, plain: bool) -> Vec<DrawItem> {
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

    let mut parts = Vec::with_capacity(scene.parts.len());
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
            let rgb = if look.part_colors { p.color } else { clay };
            let edge = if plain {
                plain_edge
            } else if is_selected {
                accent
            } else if is_hovered {
                accent_soft
            } else {
                plain_edge
            };
            let mesh = match p.shape {
                PartShape::Block => MeshKind::Cube,
                PartShape::Ball => MeshKind::Sphere,
                PartShape::Cylinder => MeshKind::Cylinder,
                PartShape::Wedge => MeshKind::Wedge,
                PartShape::CornerWedge => MeshKind::CornerWedge,
            };
            parts.push(DrawItem {
                mesh,
                model: p.world.matrix() * Mat4::from_scale(p.size),
                color: linear_color(rgb, 1.0 - p.transparency),
                edge,
            });
        }
    }
    parts
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
    let (mut links, mut spindles, mut joints) = (Vec::new(), Vec::new(), Vec::new());
    if app.view.show_bones {
        for (id, b) in scene.bones.iter().enumerate() {
            if !scene.bone_is_visible(id) || !b.kind.is_joint() {
                continue;
            }
            let head = b.head();
            let scale = app.camera.world_per_point(head, rect);
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
                model: Mat4::from_scale_rotation_translation(
                    Vec3::new(width, length, width),
                    b.frame.rot * spin,
                    head,
                ),
                color,
                edge: no_edge,
            });
            joints.push(DrawItem {
                mesh: MeshKind::Sphere,
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
                if let Some(model) = line_strip(&app.camera, head, to, LINK_WIDTH * scale) {
                    links.push(DrawItem {
                        mesh: MeshKind::Quad,
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
        parts,
        bones,
        bones_on_top: app.view.bones_on_top,
        style: scene_style(app),
    }
}

/// The scene as a camera sees it: parts only, with nothing that isn't in the game.
fn camera_frame(app: &KayeApp, picture: Rect, ppp: f32, view: ThroughView) -> FrameInput {
    FrameInput {
        size: [(picture.width() * ppp).round() as u32, (picture.height() * ppp).round() as u32],
        view_proj: view.view_proj(picture.aspect_ratio()),
        eye: view.frame.pos,
        grid_y: app.view.show_grid_floor_y,
        grid_fade: 70.0,
        parts: part_items(app, true),
        bones: Vec::new(),
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

fn paint_diamond(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32) {
    let points = vec![center - vec2(0.0, radius), center + vec2(radius, 0.0), center + vec2(0.0, radius), center - vec2(radius, 0.0)];
    painter.add(Shape::closed_line(points, Stroke::new(1.8, color)));
}

/// IK targets as diamonds and poles as circles, each with a dashed line back to the
/// joint it's pulling on.
fn paint_ik_controls(app: &KayeApp, painter: &egui::Painter, rect: Rect) {
    if !app.view.show_bones {
        return;
    }
    let scene = &app.scene;
    let on_screen = |p: Vec3| app.camera.world_to_screen(p, rect);
    for modifier in &scene.modifiers {
        let ModifierKind::Ik(ik) = modifier.kind else {
            continue;
        };
        if !modifier.enabled || !scene.bone_is_visible(ik.target) {
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

fn paint_labels(app: &KayeApp, painter: &egui::Painter, rect: Rect) {
    let at = rect.left_top() + vec2(10.0, 8.0);
    painter.text(at, Align2::LEFT_TOP, app.camera.label(), FontId::proportional(11.5), theme::text_faint());
    if let Some(session) = &app.session {
        painter.text(
            at + vec2(0.0, 16.0),
            Align2::LEFT_TOP,
            session.readout(),
            FontId::proportional(12.5),
            theme::ink(),
        );
    }
}

/// The corner axis widget: click an axis for that view, drag it to orbit.
fn nav_gizmo(app: &mut KayeApp, ui: &mut Ui, painter: &egui::Painter, rect: Rect) {
    let center = rect.right_top() + vec2(-NAV_RADIUS - 12.0, NAV_RADIUS + 10.0);
    let area = Rect::from_center_size(center, egui::Vec2::splat((NAV_RADIUS + 9.0) * 2.0));
    if !rect.intersects(area) {
        return;
    }
    let response = ui.interact(area, ui.scope_id().with("nav_gizmo"), Sense::click_and_drag());
    if response.dragged() {
        app.camera.orbit(response.drag_delta().x, response.drag_delta().y);
    }
    if response.hovered() || response.dragged() {
        painter.circle_filled(center, NAV_RADIUS + 8.0, Color32::from_white_alpha(10));
    }

    let (right, up, toward) = (app.camera.right(), app.camera.up(), app.camera.to_camera());
    struct Ball {
        pos: Pos2,
        depth: f32,
        axis: usize,
        positive: bool,
    }
    let mut balls = Vec::with_capacity(6);
    for (axis, dir) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate() {
        for positive in [true, false] {
            let d = if positive { dir } else { -dir };
            balls.push(Ball {
                pos: center + vec2(d.dot(right), -d.dot(up)) * NAV_RADIUS * 0.84,
                depth: d.dot(toward),
                axis,
                positive,
            });
        }
    }
    balls.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    let pointer = response.hover_pos();
    let mut clicked = None;
    for ball in &balls {
        let color = AXIS_COLORS[ball.axis];
        let radius = if ball.positive { 7.5 } else { 5.0 };
        let is_hot = pointer.is_some_and(|p| (p - ball.pos).length() < radius + 1.0);
        if ball.positive {
            painter.line_segment([center, ball.pos], Stroke::new(1.5, color));
            painter.circle_filled(ball.pos, radius, if is_hot { theme::ink() } else { color });
            painter.text(
                ball.pos,
                Align2::CENTER_CENTER,
                ["X", "Y", "Z"][ball.axis],
                FontId::proportional(10.0),
                theme::window(),
            );
        } else {
            let fill = if is_hot { color } else { color.gamma_multiply(0.25) };
            painter.circle(ball.pos, radius, fill, Stroke::new(1.0, color.gamma_multiply(0.7)));
        }
        if response.clicked() && is_hot {
            clicked = Some(preset_for(ball.axis, ball.positive));
        }
    }
    if let Some(preset) = clicked {
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

fn paint_toasts(app: &KayeApp, painter: &egui::Painter, rect: Rect) {
    let mut y = rect.bottom() - 10.0;
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
        let size = galley.size() + vec2(16.0, 8.0);
        let box_rect = Rect::from_min_size(pos2(rect.left() + 10.0, y - size.y), size);
        painter.rect_filled(box_rect, 2.0, theme::panel_deep());
        painter.galley(box_rect.min + vec2(8.0, 4.0), galley, color);
        y -= size.y + 3.0;
    }
}

fn drop_overlay(ctx: &egui::Context, painter: &egui::Painter, rect: Rect) {
    if ctx.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    painter.rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(2.0, theme::accent()), egui::StrokeKind::Inside);
}

fn import_button(app: &mut KayeApp, ui: &mut Ui, rect: Rect) {
    let button = Rect::from_center_size(rect.center(), vec2(112.0, 28.0));
    let response = ui.interact(button, ui.scope_id().with("import_empty"), Sense::click());
    let fill = if response.hovered() { theme::field_active() } else { theme::field_hover() };
    ui.painter().rect_filled(button, 2.0, fill);
    let icon = Rect::from_center_size(button.left_center() + vec2(16.0, 0.0), egui::Vec2::splat(15.0));
    icons::paint(ui.painter(), icon, Icon::Import, theme::ink());
    ui.painter().text(
        button.left_center() + vec2(29.0, 0.0),
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
        app.delete_camera(camera);
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
    if let Some(bone) = selected.filter(|b| app.scene.bones[*b].kind.is_joint()) {
        let (has_ik, has_unparent) = (app.has_ik(bone), app.has_unparent(bone));
        ui.menu_button("Add Modifier", |ui| super::side::add_modifier_items(app, ui, bone, has_ik, has_unparent));
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
