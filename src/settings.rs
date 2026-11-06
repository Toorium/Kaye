//! Everything in Preferences, saved as JSON between launches. Unknown or missing
//! values fall back to defaults, so old settings files keep working.

use std::collections::BTreeMap;
use std::path::PathBuf;

use egui::Key;
use serde::{Deserialize, Serialize};

use crate::animation::{EaseStyle, HandleType, Smoothing};

pub type Rgb = [u8; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shading {
    Studio,
    Shaded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewportBackground {
    Gradient,
    Solid,
    UiImage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFit {
    Cover,
    Stretch,
}

/// Which graphics system draws Kaye. Auto lets the driver setup decide.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphicsApi {
    #[default]
    Auto,
    Direct3D12,
    Vulkan,
    OpenGl,
    Metal,
}

impl GraphicsApi {
    pub const ALL: [GraphicsApi; 5] = [GraphicsApi::Auto, GraphicsApi::Direct3D12, GraphicsApi::Vulkan, GraphicsApi::OpenGl, GraphicsApi::Metal];

    pub fn label(self) -> &'static str {
        match self {
            GraphicsApi::Auto => "Auto",
            GraphicsApi::Direct3D12 => "Direct3D 12",
            GraphicsApi::Vulkan => "Vulkan",
            GraphicsApi::OpenGl => "OpenGL",
            GraphicsApi::Metal => "Metal",
        }
    }

    /// Whether this operating system has it at all.
    pub fn exists_here(self) -> bool {
        match self {
            GraphicsApi::Auto | GraphicsApi::OpenGl => true,
            GraphicsApi::Direct3D12 => cfg!(windows),
            GraphicsApi::Vulkan => !cfg!(target_os = "macos"),
            GraphicsApi::Metal => cfg!(target_os = "macos"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InterfaceSettings {
    pub ui_scale: f32,
    pub tooltips: bool,
    pub show_fps: bool,
}

impl Default for InterfaceSettings {
    fn default() -> Self {
        Self {
            ui_scale: 1.0,
            tooltips: true,
            show_fps: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeSettings {
    pub accent: Rgb,
    pub accent_soft: Rgb,
    pub panel: Rgb,
    pub header: Rgb,
    pub field: Rgb,
    pub text: Rgb,
    pub background_image: Option<String>,
    pub image_opacity: f32,
    pub panel_opacity: f32,
    pub image_fit: ImageFit,
    /// Interface colors set on their own in All Colors, by `theme::UiColor::key`. Any not
    /// here follow the main colors above.
    pub colors: BTreeMap<String, Rgb>,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            accent: [0xF3, 0x6B, 0x70],
            accent_soft: [0xFF, 0xA8, 0xA3],
            panel: [0x44, 0x44, 0x44],
            header: [0x4B, 0x4B, 0x4B],
            field: [0x5D, 0x5D, 0x5D],
            text: [0xC8, 0xC8, 0xC8],
            background_image: None,
            image_opacity: 1.0,
            panel_opacity: 1.0,
            image_fit: ImageFit::Cover,
            colors: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewportSettings {
    pub shading: Shading,
    pub background: ViewportBackground,
    pub background_top: Rgb,
    pub background_bottom: Rgb,
    pub show_grid: bool,
    pub grid_color: Rgb,
    pub grid_opacity: f32,
    pub clay: Rgb,
    pub part_colors: bool,
    pub part_edges: f32,
    pub bone_color: Rgb,
    pub link_color: Rgb,
    pub bone_width: f32,
    pub joint_size: f32,
    pub gizmo_size: f32,
    pub field_of_view: f32,
}

impl Default for ViewportSettings {
    fn default() -> Self {
        Self {
            shading: Shading::Studio,
            background: ViewportBackground::Solid,
            background_top: [0x3D, 0x3D, 0x3D],
            background_bottom: [0x2F, 0x2F, 0x2F],
            show_grid: true,
            grid_color: [0x8D, 0x8D, 0x8D],
            grid_opacity: 1.0,
            clay: [0x75, 0x75, 0x75],
            // On, so a file's textures and materials show without hunting for a switch.
            part_colors: true,
            part_edges: 0.3,
            bone_color: [0xF2, 0xF2, 0xF2],
            link_color: [0xB8, 0xB8, 0xB8],
            bone_width: 7.0,
            joint_size: 3.6,
            gizmo_size: 1.0,
            field_of_view: 34.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LightingSettings {
    pub sun_azimuth: f32,
    pub sun_elevation: f32,
    pub sun_color: Rgb,
    pub sun_strength: f32,
    pub sky_ambient: Rgb,
    pub ground_ambient: Rgb,
    pub shadows: bool,
    pub shadow_softness: f32,
    pub shadow_strength: f32,
    pub floor_shadows: bool,
}

impl Default for LightingSettings {
    /// Modeled on Roblox's default daytime look: a strong sun, a generous flat
    /// ambient, and soft shadows that never go fully black.
    fn default() -> Self {
        Self {
            sun_azimuth: 30.0,
            sun_elevation: 58.0,
            sun_color: [0xFF, 0xF8, 0xEC],
            sun_strength: 0.72,
            sky_ambient: [0x9C, 0xA4, 0xB0],
            ground_ambient: [0x72, 0x70, 0x6C],
            shadows: true,
            shadow_softness: 2.5,
            shadow_strength: 0.45,
            floor_shadows: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnimationSettings {
    pub auto_key: bool,
    pub key_start_pose: bool,
    pub only_insert_needed: bool,
    pub default_style: EaseStyle,
    pub default_handle: HandleType,
    pub smoothing: Smoothing,
    pub show_seconds: bool,
    pub tick_spacing: f32,
    pub key_size: f32,
    pub allow_negative_frames: bool,
    pub default_fps: f32,
    pub default_end: i32,
    pub looping: bool,
    pub playback_speed: f32,
    pub play_every_frame: bool,
    /// How far behind the mouse G, R and R R leave a bone, from 0 (exactly with it) to
    /// `gizmo::MAX_WEIGHT`. See `gizmo::Weights`.
    pub grab_weight: f32,
    pub rotate_weight: f32,
    pub free_rotate_weight: f32,
    /// Each animation keeps its own frame rate and modifiers. Like smoothing, it's really
    /// the open project's, in `Animation::separate`, and this follows it.
    pub individual_properties: bool,
}

impl Default for AnimationSettings {
    fn default() -> Self {
        Self {
            auto_key: true,
            key_start_pose: true,
            only_insert_needed: false,
            default_style: EaseStyle::Bezier,
            default_handle: HandleType::AutoClamped,
            smoothing: Smoothing::Continuous,
            show_seconds: false,
            tick_spacing: 80.0,
            key_size: 6.0,
            allow_negative_frames: false,
            default_fps: 30.0,
            default_end: 120,
            looping: true,
            playback_speed: 1.0,
            play_every_frame: false,
            grab_weight: 0.0,
            rotate_weight: 0.0,
            free_rotate_weight: 0.0,
            individual_properties: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavigationSettings {
    pub orbit_speed: f32,
    pub zoom_speed: f32,
    pub pan_speed: f32,
    pub invert_zoom: bool,
    pub zoom_to_mouse: bool,
    pub orbit_around_selection: bool,
    pub smooth_views: bool,
    pub view_speed: f32,
    pub alt_left_orbit: bool,
}

impl Default for NavigationSettings {
    fn default() -> Self {
        Self {
            orbit_speed: 1.0,
            zoom_speed: 1.0,
            pan_speed: 1.0,
            invert_zoom: false,
            zoom_to_mouse: false,
            orbit_around_selection: false,
            smooth_views: true,
            view_speed: 1.0,
            alt_left_orbit: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemSettings {
    pub antialiasing: bool,
    pub render_scale: f32,
    pub shadow_resolution: u32,
    pub vsync: bool,
    pub high_performance_gpu: bool,
    pub graphics_api: GraphicsApi,
    pub playback_fps_limit: u32,
    pub undo_steps: usize,
    /// An Open Cloud key with the `legacy-asset:manage` scope, for real meshes.
    pub roblox_api_key: String,
}

impl Default for SystemSettings {
    fn default() -> Self {
        Self {
            antialiasing: true,
            render_scale: 1.0,
            shadow_resolution: 2048,
            vsync: true,
            high_performance_gpu: true,
            graphics_api: GraphicsApi::Auto,
            playback_fps_limit: 0,
            undo_steps: 256,
            roblox_api_key: String::new(),
        }
    }
}

/// Where downloaded meshes and textures are kept, beside the settings.
pub fn assets_folder() -> Option<PathBuf> {
    Some(settings_path()?.with_file_name("assets"))
}

/// Where someone puts plugins, beside the settings.
pub fn plugins_folder() -> Option<PathBuf> {
    Some(settings_path()?.with_file_name("plugins"))
}

/// The list of files opened lately, beside the settings.
pub fn recent_list() -> Option<PathBuf> {
    Some(settings_path()?.with_file_name("recent.json"))
}

/// Where copies of work in progress are kept, beside the settings.
pub fn backups_folder() -> Option<PathBuf> {
    Some(settings_path()?.with_file_name("backups"))
}

/// Where the home page's pictures of files are kept, beside the settings.
pub fn thumbnails_folder() -> Option<PathBuf> {
    Some(settings_path()?.with_file_name("thumbnails"))
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Which default look this file has been upgraded to. Files from before this
    /// field read as 0.
    #[serde(default)]
    pub look: u32,
    pub interface: InterfaceSettings,
    pub theme: ThemeSettings,
    pub viewport: ViewportSettings,
    pub lighting: LightingSettings,
    pub animation: AnimationSettings,
    pub navigation: NavigationSettings,
    pub system: SystemSettings,
    pub keys: KeyMap,
}

fn settings_path() -> Option<PathBuf> {
    // Scripted test runs keep away from the real settings.
    #[cfg(feature = "devtools")]
    if let Some(path) = std::env::var_os("KAYE_SETTINGS") {
        return Some(PathBuf::from(path));
    }
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Kaye").join("settings.json"))
}

/// Exists while Kaye is starting up. Finding it on launch means the last start died
/// with the chosen graphics API, which can't be changed from a window that won't open.
pub struct StartupMarker(Option<PathBuf>);

impl StartupMarker {
    pub fn path() -> Self {
        Self(settings_path().map(|p| p.with_file_name("starting")))
    }

    pub fn left_behind(&self) -> bool {
        self.0.as_ref().is_some_and(|p| p.exists())
    }

    pub fn set(&self) {
        if let Some(path) = &self.0 {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, "");
        }
    }

    pub fn clear(&self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The default look this build ships with.
const CURRENT_LOOK: u32 = 3;

/// Moves a value to the new default only if it's still the old one.
fn follow_default<T: PartialEq>(value: &mut T, old: T, new: T) {
    if *value == old {
        *value = new;
    }
}

impl Settings {
    pub fn load() -> Self {
        let mut settings = settings_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
            .unwrap_or_default()
            .upgraded();
        if key_kept_securely() {
            let in_file = std::mem::take(&mut settings.system.roblox_api_key);
            if in_file.is_empty() {
                settings.system.roblox_api_key = crate::secrets::load_key().unwrap_or_default();
            } else {
                // Older versions wrote the key into the file. Saving moves it out.
                settings.system.roblox_api_key = in_file;
                settings.save();
            }
        }
        settings
    }

    /// Every save writes all values, so a saved file holds the old defaults as if
    /// they'd been chosen. This moves those to the current look and leaves anything
    /// the user really changed alone.
    pub fn upgraded(mut self) -> Self {
        if self.look < 1 {
            let theme = &mut self.theme;
            follow_default(&mut theme.panel, [0x20, 0x20, 0x20], [0x2B, 0x2B, 0x2B]);
            follow_default(&mut theme.header, [0x2B, 0x2B, 0x2B], [0x33, 0x33, 0x33]);
            follow_default(&mut theme.field, [0x38, 0x38, 0x38], [0x4E, 0x4E, 0x4E]);
            follow_default(&mut theme.text, [0xF2, 0xF2, 0xF2], [0xE6, 0xE6, 0xE6]);
            let view = &mut self.viewport;
            follow_default(&mut view.background_top, [0x2B, 0x2B, 0x2B], [0x42, 0x42, 0x42]);
            follow_default(&mut view.background_bottom, [0x21, 0x21, 0x21], [0x2F, 0x2F, 0x2F]);
            let new_animation = AnimationSettings::default();
            follow_default(&mut self.animation.tick_spacing, 48.0, new_animation.tick_spacing);
            follow_default(&mut self.animation.key_size, 5.0, new_animation.key_size);
        }
        if self.look < 2 {
            // Maya's interface, around Blender's dark viewport.
            let (theme, new_theme) = (&mut self.theme, ThemeSettings::default());
            follow_default(&mut theme.panel, [0x2B, 0x2B, 0x2B], new_theme.panel);
            follow_default(&mut theme.header, [0x33, 0x33, 0x33], new_theme.header);
            follow_default(&mut theme.field, [0x4E, 0x4E, 0x4E], new_theme.field);
            follow_default(&mut theme.text, [0xE6, 0xE6, 0xE6], new_theme.text);
            let (view, new_view) = (&mut self.viewport, ViewportSettings::default());
            follow_default(&mut view.background, ViewportBackground::Gradient, new_view.background);
            follow_default(&mut view.background_top, [0x42, 0x42, 0x42], new_view.background_top);
        }
        if self.look < 3 {
            // Part Colors was off by default, which hid every texture on a new install.
            follow_default(&mut self.viewport.part_colors, false, true);
        }
        self.look = CURRENT_LOOK;
        self
    }

    /// Losing this isn't worth interrupting anyone over, since the settings still
    /// apply for the current session.
    pub fn save(&self) {
        let Some(path) = settings_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut written = self.clone();
        if key_kept_securely() {
            let key = &self.system.roblox_api_key;
            let stored = crate::secrets::load_key().unwrap_or_default() == *key || crate::secrets::save_key(key);
            // If Windows won't hold it, the file keeps it, so the key isn't lost.
            if stored {
                written.system.roblox_api_key.clear();
            }
        }
        if let Ok(text) = serde_json::to_string_pretty(&written) {
            let _ = std::fs::write(path, text);
        }
    }
}

/// Whether the API key lives in the system's secure store rather than the settings file.
/// Scripted test runs keep theirs in their own settings file, away from the real key.
fn key_kept_securely() -> bool {
    #[cfg(feature = "devtools")]
    if std::env::var_os("KAYE_SETTINGS").is_some() {
        return false;
    }
    crate::secrets::available()
}

/// A key with its modifiers, stored as text like `Ctrl+Shift+Z`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct KeyCombo {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyCombo {
    pub fn parse(text: &str) -> Option<Self> {
        let (mut ctrl, mut shift, mut alt) = (false, false, false);
        let mut rest = text;
        loop {
            if let Some(after) = rest.strip_prefix("Ctrl+") {
                ctrl = true;
                rest = after;
            } else if let Some(after) = rest.strip_prefix("Shift+") {
                shift = true;
                rest = after;
            } else if let Some(after) = rest.strip_prefix("Alt+") {
                alt = true;
                rest = after;
            } else {
                break;
            }
        }
        Some(Self {
            key: Key::from_name(rest)?,
            ctrl,
            shift,
            alt,
        })
    }

    pub fn matches(&self, modifiers: egui::Modifiers) -> bool {
        (modifiers.ctrl || modifiers.command) == self.ctrl && modifiers.shift == self.shift && modifiers.alt == self.alt
    }
}

impl std::fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.ctrl {
            write!(f, "Ctrl+")?;
        }
        if self.shift {
            write!(f, "Shift+")?;
        }
        if self.alt {
            write!(f, "Alt+")?;
        }
        write!(f, "{}", self.key.name())
    }
}

impl From<KeyCombo> for String {
    fn from(combo: KeyCombo) -> String {
        combo.to_string()
    }
}

impl TryFrom<String> for KeyCombo {
    type Error = String;
    fn try_from(text: String) -> Result<Self, String> {
        KeyCombo::parse(&text).ok_or(text)
    }
}

macro_rules! actions {
    ($($variant:ident: $section:literal, $label:literal, $default:literal, $repeats:literal;)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub enum Action { $($variant,)* }

        impl Action {
            pub const ALL: &[Action] = &[$(Action::$variant,)*];

            pub fn section(self) -> &'static str {
                match self { $(Action::$variant => $section,)* }
            }

            pub fn label(self) -> &'static str {
                match self { $(Action::$variant => $label,)* }
            }

            pub fn default_combo(self) -> KeyCombo {
                let text = match self { $(Action::$variant => $default,)* };
                KeyCombo::parse(text).expect("built-in shortcut is valid")
            }

            /// Holding the key keeps triggering this action.
            pub fn repeats(self) -> bool {
                match self { $(Action::$variant => $repeats,)* }
            }
        }
    };
}

actions! {
    Undo: "General", "Undo", "Ctrl+Z", true;
    Redo: "General", "Redo", "Ctrl+Shift+Z", true;
    NewProject: "General", "New", "Ctrl+N", false;
    OpenProject: "General", "Open Project", "Ctrl+O", false;
    Save: "General", "Save", "Ctrl+S", false;
    SaveAs: "General", "Save As", "Ctrl+Shift+S", false;
    Import: "General", "Import Rig", "Ctrl+I", false;
    Preferences: "General", "Preferences", "Ctrl+Comma", false;
    Deselect: "General", "Deselect All", "Alt+A", false;
    AddCamera: "General", "Add Camera", "Shift+A", false;
    Delete: "General", "Delete", "Delete", false;
    CopyPose: "Posing", "Copy Pose, Keys", "Ctrl+C", false;
    PastePose: "Posing", "Paste Pose, Keys", "Ctrl+V", false;
    PasteMirrored: "Posing", "Paste Mirrored", "Ctrl+Shift+V", false;
    MirrorPose: "Posing", "Mirror Pose, Keys", "Ctrl+M", false;
    ToolSelect: "Tools", "Select Tool", "Q", false;
    ToolMove: "Tools", "Move Tool", "W", false;
    ToolRotate: "Tools", "Rotate Tool", "E", false;
    Grab: "Posing", "Grab", "G", false;
    Rotate: "Posing", "Rotate, Free Rotate", "R", false;
    AxisX: "Posing", "Lock X", "X", false;
    AxisY: "Posing", "Lock Y", "Y", false;
    AxisZ: "Posing", "Lock Z", "Z", false;
    Confirm: "Posing", "Confirm", "Enter", false;
    Cancel: "Posing", "Cancel", "Escape", false;
    ClearPosition: "Posing", "Clear Position", "Alt+G", false;
    ClearRotation: "Posing", "Clear Rotation", "Alt+R", false;
    PlayPause: "Animation", "Play, Pause", "Space", false;
    FrameBack: "Animation", "Previous Frame", "ArrowLeft", true;
    FrameForward: "Animation", "Next Frame", "ArrowRight", true;
    JumpStart: "Animation", "Jump to Start", "Shift+ArrowLeft", false;
    JumpEnd: "Animation", "Jump to End", "Shift+ArrowRight", false;
    PrevKey: "Animation", "Previous Keyframe", "ArrowDown", true;
    NextKey: "Animation", "Next Keyframe", "ArrowUp", true;
    InsertKey: "Animation", "Insert Keyframe", "I", false;
    DeleteKey: "Animation", "Delete Keyframe", "Alt+I", false;
    DeleteSelectedKeys: "Animation", "Delete Selected Keys", "X", false;
    DuplicateKeys: "Animation", "Duplicate Selected Keys", "Shift+D", false;
    AddOnionSkin: "Animation", "Add Onion Skin", "Shift+O", false;
    DeleteOnionSkin: "Animation", "Delete Onion Skin", "Alt+O", false;
    FrameSelected: "View", "Frame Selected", "F", false;
    FrameAll: "View", "Frame All", "Home", false;
    ToggleShading: "View", "Studio, Shaded", "Shift+Z", false;
    ToggleOrtho: "View", "Orthographic", "5", false;
    ViewFront: "View", "Front View", "1", false;
    ViewBack: "View", "Back View", "Ctrl+1", false;
    ViewRight: "View", "Right View", "3", false;
    ViewLeft: "View", "Left View", "Ctrl+3", false;
    ViewTop: "View", "Top View", "7", false;
    ViewBottom: "View", "Bottom View", "Ctrl+7", false;
    CameraView: "View", "Camera View", "0", false;
    CameraPreview: "View", "Camera Preview", "Ctrl+0", false;
}

/// Only stores what someone changed. Everything else follows the defaults.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyMap(pub BTreeMap<Action, KeyCombo>);

impl KeyMap {
    pub fn get(&self, action: Action) -> KeyCombo {
        self.0.get(&action).copied().unwrap_or_else(|| action.default_combo())
    }

    pub fn set(&mut self, action: Action, combo: KeyCombo) {
        if combo == action.default_combo() {
            self.0.remove(&action);
        } else {
            self.0.insert(action, combo);
        }
    }

    /// Whether another action in a group active at the same time uses this combo.
    /// Posing keys only run during a drag, and everything else only outside one,
    /// so `X` can mean both Lock X and Delete Keys without clashing.
    pub fn conflicts(&self, action: Action) -> bool {
        let combo = self.get(action);
        let modal = |a: Action| matches!(a, Action::AxisX | Action::AxisY | Action::AxisZ | Action::Confirm);
        Action::ALL
            .iter()
            .any(|other| *other != action && self.get(*other) == combo && modal(*other) == modal(action))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_shortcut_parses_and_round_trips() {
        for action in Action::ALL {
            let combo = action.default_combo();
            assert_eq!(KeyCombo::parse(&combo.to_string()), Some(combo), "{action:?}");
        }
    }

    #[test]
    fn default_shortcuts_have_no_conflicts() {
        let keys = KeyMap::default();
        for action in Action::ALL {
            assert!(!keys.conflicts(*action), "{action:?} conflicts");
        }
    }

    #[test]
    fn rebinding_is_saved_and_detects_conflicts() {
        let mut settings = Settings::default();
        settings.keys.set(Action::Grab, KeyCombo::parse("Ctrl+T").unwrap());
        settings.keys.set(Action::ToolSelect, KeyCombo::parse("R").unwrap());
        assert!(settings.keys.conflicts(Action::Rotate));
        assert!(settings.keys.conflicts(Action::ToolSelect));

        let text = serde_json::to_string(&settings).unwrap();
        let loaded: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(loaded, settings);
        assert_eq!(loaded.keys.get(Action::Grab).to_string(), "Ctrl+T");

        settings.keys.set(Action::Grab, Action::Grab.default_combo());
        assert!(!settings.keys.0.contains_key(&Action::Grab), "defaults aren't stored");
    }

    #[test]
    fn missing_or_old_settings_fall_back_to_defaults() {
        let partial: Settings = serde_json::from_str(r#"{"interface":{"ui_scale":1.5},"unknown":1}"#).unwrap();
        assert_eq!(partial.interface.ui_scale, 1.5);
        assert!(partial.interface.tooltips);
        assert_eq!(partial.viewport, ViewportSettings::default());
    }

    #[test]
    fn modifiers_must_match_exactly() {
        let combo = KeyCombo::parse("Shift+D").unwrap();
        assert!(combo.matches(egui::Modifiers::SHIFT));
        assert!(!combo.matches(egui::Modifiers::NONE));
        assert!(!combo.matches(egui::Modifiers::SHIFT | egui::Modifiers::CTRL));
    }
}
