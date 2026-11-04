//! Colors, fonts and widget styling, built from the theme in Preferences.
//!
//! Nothing in the 3D scene reads from here except selection highlights, so a theme
//! can never change how a rig is lit.
//!
//! Six main colors make a theme, and every other color is worked out from them. Each of
//! those others can also be set on its own in Preferences, which `ThemeSettings::colors`
//! keeps by `UiColor::key`. One left alone keeps following the main colors.

use std::sync::RwLock;

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Ui, Vec2, style::{HandleShape, ScrollStyle}};

use crate::settings::{Rgb, ThemeSettings};

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Corner rounding shared by every control.
pub const RADIUS: u8 = 2;
/// Corner rounding of joined groups and menus.
pub const SECTION_RADIUS: u8 = 2;
/// Corner rounding of windows.
pub const WINDOW_RADIUS: u8 = 2;

/// Built-in accents: name, accent, and a lighter accent for hover states. Coral is
/// Kaye's brand color.
pub const PRESETS: [(&str, Rgb, Rgb); 6] = [
    ("Coral", [0xF3, 0x6B, 0x70], [0xFF, 0xA8, 0xA3]),
    ("Ocean", [0x4F, 0xA3, 0xE8], [0x9C, 0xCB, 0xF3]),
    ("Mint", [0x3F, 0xC7, 0xA0], [0x90, 0xE2, 0xCA]),
    ("Amber", [0xE9, 0xA2, 0x3B], [0xF4, 0xCB, 0x8C]),
    ("Violet", [0x9B, 0x7B, 0xEA], [0xC7, 0xB4, 0xF4]),
    ("Graphite", [0xC4, 0xC4, 0xC4], [0xE6, 0xE6, 0xE6]),
];

macro_rules! ui_colors {
    ($($name:ident, $key:literal, $label:literal, $group:literal, $tip:literal;)*) => {
        /// Every interface color past the six main ones, each settable on its own.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum UiColor {
            $($name,)*
        }

        impl UiColor {
            pub const ALL: &[UiColor] = &[$(UiColor::$name,)*];

            /// What it's saved as. Never change one, or saved themes lose that color.
            pub fn key(self) -> &'static str {
                match self {
                    $(UiColor::$name => $key,)*
                }
            }

            pub fn label(self) -> &'static str {
                match self {
                    $(UiColor::$name => $label,)*
                }
            }

            /// Which list it's in under All Colors.
            pub fn group(self) -> &'static str {
                match self {
                    $(UiColor::$name => $group,)*
                }
            }

            /// Where it shows, for someone deciding whether to change it.
            pub fn tip(self) -> &'static str {
                match self {
                    $(UiColor::$name => $tip,)*
                }
            }

            pub fn from_key(key: &str) -> Option<UiColor> {
                match key {
                    $($key => Some(UiColor::$name),)*
                    _ => None,
                }
            }
        }
    };
}

ui_colors! {
    Window, "window", "Borders", "Panels", "The lines between panels, and around windows and menus.";
    PanelDeep, "panel_deep", "Lists", "Panels", "Behind the Workspace, the timeline's tracks and other lists.";
    Section, "section", "Section Headers", "Panels", "The bars at the top of each part of Properties.";
    RowAlt, "row_alt", "Alternate Rows", "Panels", "Every other row in a list, so long lists are easy to follow.";
    ArtBackdrop, "art_backdrop", "Behind Artwork", "Panels", "Behind white pictures, such as the logo and the help art.";
    FieldHover, "field_hover", "Buttons Under Mouse", "Buttons and Fields", "A button while the mouse is over it.";
    FieldActive, "field_active", "Buttons Held", "Buttons and Fields", "A button while it's being clicked.";
    Well, "well", "Value Fields", "Buttons and Fields", "Number boxes, text boxes and drop-downs.";
    WellHover, "well_hover", "Value Fields Under Mouse", "Buttons and Fields", "A number box or drop-down while the mouse is over it.";
    Toggled, "toggled", "Switches On", "Buttons and Fields", "A switch or tool button that's turned on.";
    Ink, "ink", "Strong Text", "Text", "Titles, and text on a colored fill.";
    TextDim, "text_dim", "Labels", "Text", "The names beside settings and values.";
    TextFaint, "text_faint", "Faint Text", "Text", "Hints and other text that stays in the background.";
    OnAccent, "on_accent", "Text on Accent", "Text", "Text and icons on something filled with the accent color.";
    Link, "link", "Links", "Text", "Text that opens a web page.";
    SelectedRow, "selected_row", "Selected Rows", "Selection", "Rows picked in the Workspace and other lists.";
    ActiveRow, "active_row", "Active Row", "Selection", "The row picked last, which the other panels show.";
    AnimatedField, "animated_field", "Animated Values", "Keys", "A value with keys, on a frame between them.";
    KeyedField, "keyed_field", "Keyed Values", "Keys", "A value on a frame where it has a key.";
    Record, "record", "Auto Key On", "Keys", "The Auto Key button while it's recording.";
    Warning, "warning", "Warnings", "Messages", "Messages about something that may need a look.";
    Error, "error", "Errors", "Messages", "Messages about something that went wrong.";
    RigIcon, "rig_icon", "Rigs", "Icons", "The icon of a rig in the Workspace.";
    BoneIcon, "bone_icon", "Bones", "Icons", "The icon of a bone.";
    PartIcon, "part_icon", "Parts and Shapes", "Icons", "The icon of a part or shape.";
    CameraIcon, "camera_icon", "Cameras", "Icons", "The icon of a camera.";
    ModifierIcon, "modifier_icon", "Modifiers", "Icons", "The icon of a modifier.";
    OnionIcon, "onion_icon", "Onion Skins", "Icons", "The icon of an onion skin.";
    Overlay, "overlay", "Text Over the View", "3D View", "Labels and icons drawn over the 3D view.";
    ActivePanel, "active_panel", "Active Panel Outline", "3D View", "The thin line around the 3D view.";
    AxisX, "axis_x", "X Axis", "3D View", "The X handle, the X curve on the Graph tab, and X on the axis gizmo.";
    AxisY, "axis_y", "Y Axis", "3D View", "The Y handle, curve and axis.";
    AxisZ, "axis_z", "Z Axis", "3D View", "The Z handle, curve and axis.";
    HandleHover, "handle_hover", "Handle Under Mouse", "3D View", "A move or rotate handle while the mouse is over it.";
    HandleFree, "handle_free", "Free Handles", "3D View", "The handles that move or turn freely, not along one axis.";
}

#[derive(Clone, Copy)]
struct Colors {
    accent: Color32,
    accent_soft: Color32,
    panel: Color32,
    header: Color32,
    field: Color32,
    text: Color32,
    /// Every other color, by `UiColor`.
    rest: [Color32; UI_COLOR_COUNT],
    opacity: f32,
    tooltips: bool,
}

const UI_COLOR_COUNT: usize = UiColor::ALL.len();

impl Colors {
    fn get(&self, color: UiColor) -> Color32 {
        self.rest[color as usize]
    }
}

const fn opaque(c: Rgb) -> Color32 {
    rgb(c[0], c[1], c[2])
}

static COLORS: RwLock<Option<Colors>> = RwLock::new(None);

fn colors() -> Colors {
    if let Some(c) = *COLORS.read().unwrap() {
        return c;
    }
    let c = build(&ThemeSettings::default(), true);
    *COLORS.write().unwrap() = Some(c);
    c
}

/// A surface panels are filled with, which an image behind the UI can show through.
fn surface(c: Color32, opacity: f32) -> Color32 {
    if opacity >= 0.999 {
        c
    } else {
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (opacity * 255.0) as u8)
    }
}

/// What one of the other colors is right now, set on its own or worked out.
pub fn current(color: UiColor) -> Color32 {
    colors().get(color)
}

pub fn accent() -> Color32 {
    colors().accent
}
pub fn accent_soft() -> Color32 {
    colors().accent_soft
}
pub fn selected_row() -> Color32 {
    colors().get(UiColor::SelectedRow)
}
/// The active item in a list, brighter than others selected with it.
pub fn active_row() -> Color32 {
    colors().get(UiColor::ActiveRow)
}
/// A switch that's on. A full accent fill stays reserved for the current tool.
pub fn toggled() -> Color32 {
    colors().get(UiColor::Toggled)
}
pub fn window() -> Color32 {
    colors().get(UiColor::Window)
}
pub fn border() -> Color32 {
    colors().get(UiColor::Window)
}
pub fn panel_deep() -> Color32 {
    let c = colors();
    surface(c.get(UiColor::PanelDeep), c.opacity)
}
pub fn panel() -> Color32 {
    let c = colors();
    surface(c.panel, c.opacity)
}
pub fn section() -> Color32 {
    let c = colors();
    surface(c.get(UiColor::Section), c.opacity)
}
pub fn row_alt() -> Color32 {
    let c = colors();
    surface(c.get(UiColor::RowAlt), c.opacity)
}
pub fn header() -> Color32 {
    let c = colors();
    surface(c.header, c.opacity)
}
/// Behind the intro and loading animations. It's the panel color, always solid, so the
/// fade into the app has no dark flash.
pub fn cover() -> Color32 {
    colors().panel
}
/// A sunken field holding a value, as opposed to a raised button.
pub fn well() -> Color32 {
    colors().get(UiColor::Well)
}
pub fn field() -> Color32 {
    colors().field
}
pub fn field_hover() -> Color32 {
    colors().get(UiColor::FieldHover)
}
pub fn field_active() -> Color32 {
    colors().get(UiColor::FieldActive)
}
// A field's color says how its value relates to the keys, as in Maya's Channel Box.
/// A field on an animated value, on a frame with no key: dusty pink.
pub fn animated_field() -> Color32 {
    colors().get(UiColor::AnimatedField)
}
/// A field on a frame with a key: red.
pub fn keyed_field() -> Color32 {
    colors().get(UiColor::KeyedField)
}
/// The strongest text, for anything on a colored fill.
pub fn ink() -> Color32 {
    colors().get(UiColor::Ink)
}
pub fn text() -> Color32 {
    colors().text
}
pub fn text_dim() -> Color32 {
    colors().get(UiColor::TextDim)
}
pub fn text_faint() -> Color32 {
    colors().get(UiColor::TextFaint)
}
pub fn tooltips() -> bool {
    colors().tooltips
}
/// Text and icons on a full accent fill, chosen so any accent stays readable.
pub fn on_accent() -> Color32 {
    colors().get(UiColor::OnAccent)
}
// Status colors mean the same thing in every theme.
pub fn warning() -> Color32 {
    colors().get(UiColor::Warning)
}
pub fn error() -> Color32 {
    colors().get(UiColor::Error)
}
// What a thing is reads from its icon's color in every theme.
pub fn rig_icon() -> Color32 {
    colors().get(UiColor::RigIcon)
}
pub fn bone_icon() -> Color32 {
    colors().get(UiColor::BoneIcon)
}
pub fn part_icon() -> Color32 {
    colors().get(UiColor::PartIcon)
}
pub fn camera_icon() -> Color32 {
    colors().get(UiColor::CameraIcon)
}
pub fn modifier_icon() -> Color32 {
    colors().get(UiColor::ModifierIcon)
}
pub fn onion_icon() -> Color32 {
    colors().get(UiColor::OnionIcon)
}
/// The Auto Key button while it's recording.
pub fn record() -> Color32 {
    colors().get(UiColor::Record)
}
/// Text and icons over the 3D view, whose background doesn't follow the theme.
pub fn overlay() -> Color32 {
    colors().get(UiColor::Overlay)
}
/// The thin line around the panel being worked in.
pub fn active_panel() -> Color32 {
    colors().get(UiColor::ActivePanel)
}
/// Behind white artwork, which would vanish on a light theme's panels.
pub fn art_backdrop() -> Color32 {
    colors().get(UiColor::ArtBackdrop)
}

/// Puts plain buttons back on a `ui` that `sunken` has darkened.
pub fn raised(ui: &mut Ui) {
    let c = colors();
    let w = &mut ui.visuals_mut().widgets;
    w.inactive.weak_bg_fill = c.field;
    w.hovered.weak_bg_fill = c.get(UiColor::FieldHover);
    w.active.weak_bg_fill = c.get(UiColor::FieldActive);
    w.open.weak_bg_fill = c.get(UiColor::FieldHover);
}

/// Makes the number fields and drop-downs that follow in this `ui` dark and sunken,
/// which is how Maya tells a value from a button.
pub fn sunken(ui: &mut Ui) {
    let c = colors();
    let w = &mut ui.visuals_mut().widgets;
    w.inactive.weak_bg_fill = c.get(UiColor::Well);
    w.hovered.weak_bg_fill = c.get(UiColor::WellHover);
    w.active.weak_bg_fill = c.get(UiColor::WellHover);
    w.open.weak_bg_fill = c.get(UiColor::WellHover);
}

/// The font family behind `strong_font`.
const STRONG: &str = "strong";

/// Tahoma Bold for titles, since egui's `strong` only changes a text's color.
pub fn strong_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(STRONG.into()))
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let channel = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    rgb(channel(a.r(), b.r()), channel(a.g(), b.g()), channel(a.b(), b.b()))
}

/// A tinted control's color under the mouse.
pub fn lighter(c: Color32) -> Color32 {
    mix(c, Color32::WHITE, 0.08)
}

fn brightness(c: Color32) -> u32 {
    c.r() as u32 + c.g() as u32 + c.b() as u32
}

/// Every color, from the main six, with any set on their own laid over the top.
fn build(theme: &ThemeSettings, tooltips: bool) -> Colors {
    let panel = opaque(theme.panel);
    let field = opaque(theme.field);
    let text = opaque(theme.text);
    let accent = opaque(theme.accent);
    let accent_soft = opaque(theme.accent_soft);
    let contrast = if brightness(panel) > 380 { Color32::BLACK } else { Color32::WHITE };
    // Light panels need far less darkening to read as a step down.
    let light_theme = contrast == Color32::BLACK;
    // Tinted fills have to be light to stay readable under dark text.
    let dark_text = brightness(text) <= 380;
    let panel_deep = mix(panel, Color32::BLACK, if light_theme { 0.07 } else { 0.37 });
    let well = if light_theme { mix(panel, Color32::WHITE, 0.6) } else { panel_deep };
    let luma = 0.2126 * accent.r() as f32 + 0.7152 * accent.g() as f32 + 0.0722 * accent.b() as f32;
    let mut rest = [Color32::BLACK; UI_COLOR_COUNT];
    for color in UiColor::ALL {
        use UiColor::*;
        rest[*color as usize] = match color {
            Window => mix(panel, Color32::BLACK, if light_theme { 0.2 } else { 0.63 }),
            PanelDeep => panel_deep,
            Section => field,
            RowAlt => mix(panel_deep, contrast, 0.022),
            ArtBackdrop => rgb(0x2B, 0x2B, 0x2B),
            FieldHover => mix(field, contrast, 0.09),
            FieldActive => mix(field, contrast, 0.17),
            Well => well,
            WellHover => mix(well, contrast, 0.06),
            Toggled => mix(field, accent, 0.26),
            Ink => mix(text, contrast, 0.6),
            TextDim => mix(text, panel, 0.1),
            TextFaint => mix(text, panel, 0.2),
            OnAccent => if luma > 100.0 { rgb(0x14, 0x14, 0x14) } else { rgb(0xF2, 0xF2, 0xF2) },
            // A light accent is unreadable on light panels, so links go dark there.
            Link => if light_theme { mix(accent, Color32::BLACK, 0.45) } else { accent_soft },
            SelectedRow => mix(panel_deep, accent, 0.3),
            ActiveRow => mix(panel_deep, accent, 0.5),
            // Each keeps at least 4.5:1 against `ink`, which is what's drawn on them.
            AnimatedField => if dark_text { rgb(0xF0, 0xC8, 0xC8) } else { rgb(0x75, 0x47, 0x4B) },
            KeyedField => if dark_text { rgb(0xE8, 0x90, 0x90) } else { rgb(0xA3, 0x3B, 0x3B) },
            Record => rgb(0xD0, 0x3A, 0x3A),
            Warning => rgb(0xE9, 0xB9, 0x4A),
            Error => rgb(0xEF, 0x5B, 0x5B),
            RigIcon => rgb(0xF0, 0xA8, 0x60),
            BoneIcon => rgb(0x8F, 0xCB, 0x8A),
            PartIcon => rgb(0x6C, 0xC4, 0xC4),
            CameraIcon => rgb(0xE6, 0xC8, 0x6A),
            ModifierIcon => rgb(0x7F, 0xB3, 0xEA),
            OnionIcon => rgb(0xC3, 0xA2, 0xF0),
            Overlay => rgb(0xEC, 0xEC, 0xEC),
            ActivePanel => rgb(0xC4, 0xC4, 0xC4),
            AxisX => rgb(232, 72, 84),
            AxisY => rgb(130, 201, 60),
            AxisZ => rgb(64, 148, 235),
            HandleHover => rgb(0xFF, 0xE4, 0x38),
            HandleFree => rgb(0xB4, 0xB4, 0xB4),
        };
    }
    for (key, value) in &theme.colors {
        if let Some(color) = UiColor::from_key(key) {
            rest[color as usize] = opaque(*value);
        }
    }
    Colors { accent, accent_soft, panel, header: opaque(theme.header), field, text, rest, opacity: theme.panel_opacity.clamp(0.0, 1.0), tooltips }
}

/// Rebuilds every color from the theme settings.
pub fn apply_settings(ctx: &egui::Context, theme: &ThemeSettings, tooltips: bool) {
    let c = build(theme, tooltips);
    *COLORS.write().unwrap() = Some(c);
    use UiColor::*;
    crate::gizmo::set_colors([c.get(AxisX), c.get(AxisY), c.get(AxisZ)], c.get(HandleHover), c.get(HandleFree));

    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        v.panel_fill = surface(c.panel, c.opacity);
        v.window_fill = c.panel;
        v.extreme_bg_color = c.get(PanelDeep);
        v.text_edit_bg_color = Some(c.get(Well));
        v.faint_bg_color = c.get(RowAlt);
        v.override_text_color = Some(c.text);
        v.window_stroke = Stroke::new(1.0, c.get(Window));
        v.hyperlink_color = c.get(Link);
        v.selection.bg_fill = c.get(Toggled);
        v.selection.stroke = Stroke::new(1.0, c.accent);
        v.text_cursor.stroke = Stroke::new(1.5, c.accent_soft);

        // Controls are flat fills with no outline, as in Maya.
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = c.panel;
        w.noninteractive.weak_bg_fill = c.panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, c.get(Window));
        w.noninteractive.fg_stroke = Stroke::new(1.0, c.get(TextDim));
        w.inactive.bg_fill = c.field;
        w.inactive.weak_bg_fill = c.field;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, c.text);
        w.hovered.bg_fill = c.get(FieldHover);
        w.hovered.weak_bg_fill = c.get(FieldHover);
        w.hovered.bg_stroke = Stroke::NONE;
        w.hovered.fg_stroke = Stroke::new(1.0, c.get(Ink));
        w.active.bg_fill = c.get(FieldActive);
        w.active.weak_bg_fill = c.get(FieldActive);
        w.active.bg_stroke = Stroke::NONE;
        w.active.fg_stroke = Stroke::new(1.0, c.get(Ink));
        w.open.bg_fill = c.get(FieldHover);
        w.open.weak_bg_fill = c.get(FieldHover);
        w.open.bg_stroke = Stroke::NONE;
        w.open.fg_stroke = Stroke::new(1.0, c.get(Ink));
    });
}

/// Sets up everything that doesn't change with the theme.
pub fn apply(ctx: &egui::Context) {
    load_system_fonts(ctx);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
        ]
        .into();

        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(4.0, 2.0);
        s.button_padding = Vec2::new(6.0, 2.0);
        s.interact_size = Vec2::new(36.0, 20.0);
        s.window_margin = Margin::same(10);
        s.menu_margin = Margin::same(2);
        s.indent = 12.0;
        s.icon_width = 13.0;
        s.icon_width_inner = 8.0;
        s.icon_spacing = 5.0;
        s.scroll = ScrollStyle::solid();
        s.scroll.bar_width = 9.0;
        s.scroll.bar_inner_margin = 1.0;
        s.scroll.bar_outer_margin = 0.0;

        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.window_corner_radius = CornerRadius::same(WINDOW_RADIUS);
        v.menu_corner_radius = CornerRadius::same(SECTION_RADIUS);
        // A hard, short drop, the way a desktop menu sits on a panel.
        v.window_shadow = Shadow { offset: [2, 3], blur: 6, spread: 0, color: Color32::from_black_alpha(90) };
        v.popup_shadow = Shadow { offset: [2, 2], blur: 4, spread: 0, color: Color32::from_black_alpha(90) };
        // The title bar stays the window's color, whichever window is on top.
        v.window_highlight_topmost = false;
        v.slider_trailing_fill = true;
        v.handle_shape = HandleShape::Rect { aspect_ratio: 0.5 };

        let radius = CornerRadius::same(RADIUS);
        let w = &mut v.widgets;
        w.noninteractive.corner_radius = radius;
        w.inactive.corner_radius = radius;
        w.hovered.corner_radius = radius;
        w.hovered.expansion = 0.0;
        w.active.corner_radius = radius;
        w.active.expansion = 0.0;
        w.open.corner_radius = radius;
    });
}

/// Tahoma, chosen by the owner for its desktop-tool look. Kaye still works with the
/// bundled font when a file is missing, and titles fall back to regular.
fn load_system_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut load = |name: &str, path: &str| {
        let Ok(bytes) = std::fs::read(path) else {
            return false;
        };
        // Tahoma's metrics leave it about a pixel high in egui's boxes.
        let tweak = egui::FontTweak { y_offset_factor: 0.06, ..Default::default() };
        fonts.font_data.insert(name.to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes).tweak(tweak)));
        true
    };
    let regular = load("tahoma", "C:/Windows/Fonts/tahoma.ttf");
    let bold = load("tahoma bold", "C:/Windows/Fonts/tahomabd.ttf");
    let family = fonts.families.entry(FontFamily::Proportional).or_default();
    if regular {
        family.insert(0, "tahoma".to_owned());
    }
    let mut strong = family.clone();
    if bold {
        strong.insert(0, "tahoma bold".to_owned());
    }
    fonts.families.insert(FontFamily::Name(STRONG.into()), strong);
    ctx.set_fonts(fonts);
}
