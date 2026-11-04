//! The Preferences window. Settings apply as they change and
//! save themselves.

use egui::{Align, Align2, CentralPanel, ComboBox, FontId, Frame, Layout, Margin, Panel, RichText, ScrollArea, Sense, Slider, Ui, Vec2, vec2};

use super::{KayeApp, theme};
use crate::animation::{EaseStyle, HandleType, Smoothing};
use crate::settings::{
    Action, AnimationSettings, GraphicsApi, ImageFit, InterfaceSettings, KeyCombo, KeyMap, LightingSettings, NavigationSettings, Rgb, Shading, SystemSettings,
    ThemeSettings, ViewportBackground, ViewportSettings,
};

const LABEL_WIDTH: f32 = 204.0;
const FIELD_WIDTH: f32 = 230.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrefsTab {
    #[default]
    Interface,
    Themes,
    Viewport,
    Lighting,
    Animation,
    Navigation,
    Keymap,
    System,
}

impl PrefsTab {
    const ALL: [PrefsTab; 8] = [
        PrefsTab::Interface,
        PrefsTab::Themes,
        PrefsTab::Viewport,
        PrefsTab::Lighting,
        PrefsTab::Animation,
        PrefsTab::Navigation,
        PrefsTab::Keymap,
        PrefsTab::System,
    ];

    fn label(self) -> &'static str {
        match self {
            PrefsTab::Interface => "Interface",
            PrefsTab::Themes => "Themes",
            PrefsTab::Viewport => "Viewport",
            PrefsTab::Lighting => "Lighting",
            PrefsTab::Animation => "Animation",
            PrefsTab::Navigation => "Navigation",
            PrefsTab::Keymap => "Keymap",
            PrefsTab::System => "System",
        }
    }
}

#[derive(Default)]
pub struct PrefsState {
    pub tab: PrefsTab,
    /// The action waiting for its new key.
    pub capturing: Option<crate::settings::Action>,
    filter: String,
}

pub fn window(app: &mut KayeApp, ctx: &egui::Context) {
    let mut open = app.prefs_open;
    egui::Window::new("Preferences")
        .open(&mut open)
        .collapsible(false)
        .fixed_size([760.0, 540.0])
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .frame(Frame::window(&ctx.global_style()).inner_margin(Margin::same(0)))
        .show(ctx, |ui| {
            Panel::left("prefs_tabs")
                .exact_size(128.0)
                .resizable(false)
                .frame(
                    // Follows the window's rounded corner, which it would otherwise cover.
                    Frame::new()
                        .fill(theme::panel_deep())
                        .corner_radius(egui::CornerRadius { sw: theme::WINDOW_RADIUS, ..Default::default() })
                        .inner_margin(Margin::symmetric(4, 6)),
                )
                .show(ui, |ui| tabs(app, ui));
            CentralPanel::default().frame(Frame::new().inner_margin(Margin::symmetric(14, 10))).show(ui, |ui| {
                ScrollArea::vertical().auto_shrink(false).id_salt("prefs_body").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    theme::sunken(ui);
                    match app.prefs.tab {
                        PrefsTab::Interface => interface(app, ui),
                        PrefsTab::Themes => themes(app, ui),
                        PrefsTab::Viewport => viewport(app, ui),
                        PrefsTab::Lighting => lighting(app, ui),
                        PrefsTab::Animation => animation(app, ui),
                        PrefsTab::Navigation => navigation(app, ui),
                        PrefsTab::Keymap => keymap(app, ui),
                        PrefsTab::System => system(app, ui),
                    }
                });
            });
        });
    app.prefs_open = open;
    if !open {
        app.prefs.capturing = None;
    }
    capture_key(app, ctx);
}

fn tabs(app: &mut KayeApp, ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = 1.0;
    for tab in PrefsTab::ALL {
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 25.0), Sense::click());
        let active = app.prefs.tab == tab;
        if active {
            ui.painter().rect_filled(rect, theme::RADIUS, theme::active_row());
        } else if response.hovered() {
            ui.painter().rect_filled(rect, theme::RADIUS, theme::header());
        }
        ui.painter().text(
            rect.left_center() + vec2(9.0, 0.0),
            Align2::LEFT_CENTER,
            tab.label(),
            FontId::proportional(12.5),
            if active { theme::ink() } else { theme::text_dim() },
        );
        if response.clicked() {
            app.prefs.tab = tab;
            app.prefs.capturing = None;
        }
    }
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(text).font(theme::strong_font(12.5)).color(theme::text()));
    ui.add_space(1.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, theme::header()));
    ui.add_space(3.0);
}

/// A right-aligned label followed by the control.
fn row<R>(ui: &mut Ui, label: &str, content: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(LABEL_WIDTH, 20.0), Sense::hover());
        ui.painter().text(
            rect.right_center(),
            Align2::RIGHT_CENTER,
            label,
            FontId::proportional(12.0),
            theme::text_dim(),
        );
        ui.add_space(8.0);
        content(ui)
    })
    .inner
}

fn slider(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str) {
    row(ui, label, |ui| {
        ui.spacing_mut().slider_width = FIELD_WIDTH - 54.0;
        ui.add(Slider::new(value, range).suffix(suffix).max_decimals(2));
    });
}

fn toggle(ui: &mut Ui, label: &str, value: &mut bool) {
    row(ui, label, |ui| {
        ui.checkbox(value, "");
    });
}

fn color(ui: &mut Ui, label: &str, value: &mut Rgb) {
    row(ui, label, |ui| {
        egui::color_picker::color_edit_button_srgb(ui, value);
    });
}

/// A row of options side by side, for short choices.
fn choice<T: Copy + PartialEq>(ui: &mut Ui, label: &str, value: &mut T, options: &[(T, &str)]) {
    row(ui, label, |ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        for (option, name) in options {
            if ui.selectable_label(*value == *option, *name).clicked() {
                *value = *option;
            }
        }
    });
}

fn reset(ui: &mut Ui) -> bool {
    ui.add_space(12.0);
    let mut clicked = false;
    row(ui, "", |ui| {
        theme::raised(ui);
        clicked = ui.button("Reset Section").clicked();
    });
    clicked
}

fn interface(app: &mut KayeApp, ui: &mut Ui) {
    let s = &mut app.settings.interface;
    heading(ui, "Display");
    slider(ui, "Resolution Scale", &mut s.ui_scale, 0.75..=2.0, "");
    toggle(ui, "Show Frame Rate", &mut s.show_fps);
    heading(ui, "Editors");
    toggle(ui, "Tooltips", &mut s.tooltips);
    if reset(ui) {
        *s = InterfaceSettings::default();
    }
}

fn themes(app: &mut KayeApp, ui: &mut Ui) {
    let s = &mut app.settings.theme;
    heading(ui, "Theme");
    row(ui, "Preset", |ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for (name, accent, soft) in theme::PRESETS {
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
            let fill = egui::Color32::from_rgb(accent[0], accent[1], accent[2]);
            ui.painter().rect_filled(rect.shrink(2.0), 2.0, fill);
            if s.accent == accent {
                ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(1.5, theme::ink()), egui::StrokeKind::Inside);
            }
            if response.on_hover_text(name).clicked() {
                s.accent = accent;
                s.accent_soft = soft;
            }
        }
    });
    color(ui, "Accent", &mut s.accent);
    color(ui, "Accent Light", &mut s.accent_soft);
    heading(ui, "Main Colors");
    color(ui, "Panels", &mut s.panel);
    color(ui, "Headers", &mut s.header);
    color(ui, "Buttons", &mut s.field);
    color(ui, "Text", &mut s.text);
    all_colors(ui, s);

    heading(ui, "Background Image");
    let mut choose = false;
    let mut remove = false;
    row(ui, "Image", |ui| {
        theme::raised(ui);
        choose = ui.button("Choose...").clicked();
        if let Some(path) = &s.background_image {
            remove = ui.button("Remove").clicked();
            let name = std::path::Path::new(path).file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            ui.add(egui::Label::new(RichText::new(name).color(theme::text_dim())).truncate());
        }
    });
    slider(ui, "Image Opacity", &mut s.image_opacity, 0.0..=1.0, "");
    slider(ui, "Panel Opacity", &mut s.panel_opacity, 0.2..=1.0, "");
    choice(ui, "Fit", &mut s.image_fit, &[(ImageFit::Cover, "Cover"), (ImageFit::Stretch, "Stretch")]);
    if reset(ui) {
        let image = s.background_image.take();
        *s = ThemeSettings {
            background_image: image,
            ..ThemeSettings::default()
        };
    }
    if choose {
        app.choose_background(ui.ctx());
    }
    if remove {
        app.remove_background();
    }
}

/// Every other color, a group at a time. Each follows the main colors until it's changed
/// here, and its Reset button puts it back to following them.
fn all_colors(ui: &mut Ui, s: &mut ThemeSettings) {
    heading(ui, "All Colors");
    row(ui, "", |ui| {
        let note = "These follow the main colors until you change one.";
        ui.label(RichText::new(note).color(theme::text_faint()));
    });
    let mut groups: Vec<&str> = Vec::new();
    for color in theme::UiColor::ALL {
        if !groups.contains(&color.group()) {
            groups.push(color.group());
        }
    }
    for group in groups {
        let changed = theme::UiColor::ALL.iter().filter(|c| c.group() == group && s.colors.contains_key(c.key())).count();
        let title = if changed == 0 { group.to_string() } else { format!("{group} ({changed} changed)") };
        egui::CollapsingHeader::new(RichText::new(title).color(theme::text())).id_salt(("all_colors", group)).default_open(false).show(ui, |ui| {
            for color in theme::UiColor::ALL.iter().filter(|c| c.group() == group) {
                row(ui, color.label(), |ui| {
                    let now = theme::current(*color);
                    let mut value = [now.r(), now.g(), now.b()];
                    if egui::color_picker::color_edit_button_srgb(ui, &mut value).on_hover_text(color.tip()).changed() {
                        s.colors.insert(color.key().to_string(), value);
                    }
                    if s.colors.contains_key(color.key()) {
                        theme::raised(ui);
                        if ui.small_button("Reset").on_hover_text("Follow the main colors again").clicked() {
                            s.colors.remove(color.key());
                        }
                    }
                });
            }
        });
    }
}

fn viewport(app: &mut KayeApp, ui: &mut Ui) {
    let s = &mut app.settings.viewport;
    heading(ui, "Shading");
    choice(ui, "Mode", &mut s.shading, &[(Shading::Studio, "Studio"), (Shading::Shaded, "Shaded")]);
    heading(ui, "Background");
    choice(
        ui,
        "Type",
        &mut s.background,
        &[
            (ViewportBackground::Gradient, "Gradient"),
            (ViewportBackground::Solid, "Solid"),
            (ViewportBackground::UiImage, "Image"),
        ],
    );
    color(ui, "Color", &mut s.background_top);
    color(ui, "Lower Color", &mut s.background_bottom);
    heading(ui, "Grid");
    toggle(ui, "Show Grid", &mut s.show_grid);
    color(ui, "Color", &mut s.grid_color);
    slider(ui, "Opacity", &mut s.grid_opacity, 0.0..=1.0, "");
    heading(ui, "Parts");
    color(ui, "Default Color", &mut s.clay);
    toggle(ui, "Use Part Colors", &mut s.part_colors);
    slider(ui, "Edge Strength", &mut s.part_edges, 0.0..=1.0, "");
    heading(ui, "Bones");
    color(ui, "Bone Color", &mut s.bone_color);
    color(ui, "Line Color", &mut s.link_color);
    slider(ui, "Bone Width", &mut s.bone_width, 2.0..=16.0, "");
    slider(ui, "Joint Size", &mut s.joint_size, 1.0..=10.0, "");
    heading(ui, "Camera and Handles");
    slider(ui, "Field of View", &mut s.field_of_view, 10.0..=90.0, "\u{b0}");
    slider(ui, "Handle Size", &mut s.gizmo_size, 0.5..=2.0, "");
    if reset(ui) {
        *s = ViewportSettings::default();
    }
}

fn lighting(app: &mut KayeApp, ui: &mut Ui) {
    choice(
        ui,
        "Mode",
        &mut app.settings.viewport.shading,
        &[(Shading::Studio, "Studio"), (Shading::Shaded, "Shaded")],
    );
    let s = &mut app.settings.lighting;
    heading(ui, "Sun");
    slider(ui, "Direction", &mut s.sun_azimuth, -180.0..=180.0, "\u{b0}");
    slider(ui, "Height", &mut s.sun_elevation, 5.0..=90.0, "\u{b0}");
    color(ui, "Color", &mut s.sun_color);
    slider(ui, "Brightness", &mut s.sun_strength, 0.0..=2.0, "");
    heading(ui, "Ambient");
    color(ui, "From Above", &mut s.sky_ambient);
    color(ui, "From Below", &mut s.ground_ambient);
    heading(ui, "Shadows");
    toggle(ui, "Shadows", &mut s.shadows);
    toggle(ui, "On Floor", &mut s.floor_shadows);
    slider(ui, "Softness", &mut s.shadow_softness, 0.0..=8.0, "");
    slider(ui, "Darkness", &mut s.shadow_strength, 0.0..=1.0, "");
    if reset(ui) {
        *s = LightingSettings::default();
    }
}

fn animation(app: &mut KayeApp, ui: &mut Ui) {
    let s = &mut app.settings.animation;
    heading(ui, "Keyframes");
    toggle(ui, "Auto Key", &mut s.auto_key);
    toggle(ui, "Key Start Pose on First Edit", &mut s.key_start_pose);
    toggle(ui, "Only Insert Needed", &mut s.only_insert_needed);
    heading(ui, "Curves");
    row(ui, "Default Interpolation", |ui| {
        ComboBox::from_id_salt("default_style")
            .selected_text(s.default_style.label())
            .width(FIELD_WIDTH - 20.0)
            .height(340.0)
            .show_ui(ui, |ui| {
                for style in EaseStyle::ALL {
                    ui.selectable_value(&mut s.default_style, style, style.label());
                }
            });
    });
    row(ui, "Default Handle Type", |ui| {
        ComboBox::from_id_salt("default_handle")
            .selected_text(s.default_handle.label())
            .width(FIELD_WIDTH - 20.0)
            .show_ui(ui, |ui| {
                for handle in HandleType::ALL {
                    ui.selectable_value(&mut s.default_handle, handle, handle.label());
                }
            });
    });
    choice(
        ui,
        "Auto Smoothing",
        &mut s.smoothing,
        &[(Smoothing::None, "None"), (Smoothing::Continuous, "Continuous Acceleration")],
    );
    heading(ui, "Timeline");
    toggle(ui, "Show Seconds", &mut s.show_seconds);
    slider(ui, "Minimum Grid Spacing", &mut s.tick_spacing, 24.0..=120.0, "");
    slider(ui, "Keyframe Size", &mut s.key_size, 3.0..=9.0, "");
    toggle(ui, "Allow Negative Frames", &mut s.allow_negative_frames);
    heading(ui, "Playback");
    slider(ui, "Speed", &mut s.playback_speed, 0.1..=2.0, "x");
    toggle(ui, "Play Every Frame", &mut s.play_every_frame);
    heading(ui, "New Animations");
    slider(ui, "Frame Rate", &mut s.default_fps, 12.0..=120.0, "");
    row(ui, "End Frame", |ui| {
        ui.add_sized([60.0, 20.0], egui::DragValue::new(&mut s.default_end).range(1..=100000));
    });
    toggle(ui, "Loop", &mut s.looping);
    if reset(ui) {
        *s = AnimationSettings::default();
    }
}

fn navigation(app: &mut KayeApp, ui: &mut Ui) {
    let s = &mut app.settings.navigation;
    heading(ui, "Orbit and Pan");
    slider(ui, "Orbit Sensitivity", &mut s.orbit_speed, 0.1..=3.0, "");
    slider(ui, "Pan Sensitivity", &mut s.pan_speed, 0.1..=3.0, "");
    toggle(ui, "Orbit Around Selection", &mut s.orbit_around_selection);
    toggle(ui, "Alt + Left Mouse Orbits", &mut s.alt_left_orbit);
    heading(ui, "Zoom");
    slider(ui, "Zoom Sensitivity", &mut s.zoom_speed, 0.1..=3.0, "");
    toggle(ui, "Invert Zoom", &mut s.invert_zoom);
    toggle(ui, "Zoom to Mouse Position", &mut s.zoom_to_mouse);
    heading(ui, "View Changes");
    toggle(ui, "Smooth View", &mut s.smooth_views);
    slider(ui, "Speed", &mut s.view_speed, 0.25..=3.0, "");
    if reset(ui) {
        *s = NavigationSettings::default();
    }
}

fn keymap(app: &mut KayeApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.prefs.filter).hint_text("Search").desired_width(220.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            theme::raised(ui);
            if ui.button("Reset All").clicked() {
                app.settings.keys = KeyMap::default();
            }
        });
    });
    let filter = app.prefs.filter.trim().to_lowercase();
    let mut section = "";
    for action in Action::ALL.iter().copied() {
        let combo = app.settings.keys.get(action);
        let text = combo.to_string();
        if !filter.is_empty() && !action.label().to_lowercase().contains(&filter) && !text.to_lowercase().contains(&filter) {
            continue;
        }
        if action.section() != section {
            section = action.section();
            heading(ui, section);
        }
        row(ui, action.label(), |ui| {
            let capturing = app.prefs.capturing == Some(action);
            let label = if capturing { "Press a key".to_string() } else { text.clone() };
            let conflict = app.settings.keys.conflicts(action);
            let mut rich = RichText::new(label);
            if conflict {
                rich = rich.color(theme::error());
            }
            let button = egui::Button::new(rich).selected(capturing);
            if ui.add_sized([150.0, 20.0], button).clicked() {
                app.prefs.capturing = Some(action);
            }
            let is_default = combo == action.default_combo();
            theme::raised(ui);
            if ui.add_enabled(!is_default, egui::Button::new("Reset")).clicked() {
                app.settings.keys.set(action, action.default_combo());
            }
        });
    }
}

/// Takes the next key press as the new shortcut. Esc backs out.
fn capture_key(app: &mut KayeApp, ctx: &egui::Context) {
    let Some(action) = app.prefs.capturing else {
        return;
    };
    let pressed = ctx.input_mut(|i| {
        let mut found = None;
        i.events.retain(|event| match event {
            egui::Event::Key { key, pressed: true, modifiers, .. } => {
                found = Some((*key, *modifiers));
                false
            }
            _ => true,
        });
        found
    });
    if let Some((key, modifiers)) = pressed {
        app.prefs.capturing = None;
        if key != egui::Key::Escape {
            app.settings.keys.set(
                action,
                KeyCombo {
                    key,
                    ctrl: modifiers.ctrl || modifiers.command,
                    shift: modifiers.shift,
                    alt: modifiers.alt,
                },
            );
        }
    }
}

fn system(app: &mut KayeApp, ui: &mut Ui) {
    let device = app.gpu_name.clone();
    let running = app.gpu_backend.clone();
    let s = &mut app.settings.system;
    heading(ui, "Graphics");
    row(ui, "Device", |ui| ui.label(RichText::new(device).color(theme::text_dim())));
    row(ui, "Running On", |ui| ui.label(RichText::new(running).color(theme::text_dim())));
    row(ui, "Graphics API (After Restart)", |ui| {
        ComboBox::from_id_salt("graphics_api")
            .selected_text(s.graphics_api.label())
            .width(FIELD_WIDTH - 20.0)
            .show_ui(ui, |ui| {
                for api in GraphicsApi::ALL.into_iter().filter(|a| a.exists_here()) {
                    ui.selectable_value(&mut s.graphics_api, api, api.label());
                }
            });
    });
    heading(ui, "Performance");
    choice(ui, "Anti-Aliasing", &mut s.antialiasing, &[(false, "Off"), (true, "4x")]);
    slider(ui, "Render Scale", &mut s.render_scale, 0.25..=2.0, "");
    choice(
        ui,
        "Shadow Quality",
        &mut s.shadow_resolution,
        &[(1024, "1024"), (2048, "2048"), (4096, "4096")],
    );
    row(ui, "Playback Frame Limit", |ui| {
        ui.add_sized([60.0, 20.0], egui::DragValue::new(&mut s.playback_fps_limit).range(0..=240).custom_formatter(|v, _| {
            if v == 0.0 { "None".to_string() } else { format!("{v:.0}") }
        }));
    });
    toggle(ui, "VSync (After Restart)", &mut s.vsync);
    toggle(ui, "High Performance GPU (After Restart)", &mut s.high_performance_gpu);
    heading(ui, "Memory");
    row(ui, "Undo Steps", |ui| {
        ui.add_sized([60.0, 20.0], egui::DragValue::new(&mut s.undo_steps).range(8..=4096));
    });
    heading(ui, "Roblox");
    let mut get_key = false;
    row(ui, "API Key", |ui| {
        ui.add_sized([FIELD_WIDTH, 20.0], egui::TextEdit::singleline(&mut s.roblox_api_key).password(true));
        theme::raised(ui);
        get_key = ui.button("Get a Key...").clicked();
    });
    if reset(ui) {
        // A key is something someone went and got, not a preference to put back.
        let key = std::mem::take(&mut s.roblox_api_key);
        *s = SystemSettings { roblox_api_key: key, ..SystemSettings::default() };
    }
    if get_key {
        app.connect.ask(&app.settings.system.roblox_api_key);
    }
}
