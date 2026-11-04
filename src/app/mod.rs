//! Application state, actions and keyboard shortcuts.

mod chrome;
pub mod connect;
mod dragout;
pub mod graph;
mod home;
mod icons;
mod loading;
pub mod preferences;
mod side;
pub mod theme;
pub mod timeline;
mod update_popup;
pub mod updater;
mod viewport;

use std::cell::RefCell;
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use eframe::egui_wgpu;
use egui::Rect;

use crate::animation::{Animation, EaseStyle, Easing, HandleType, KeyId, CHANNELS};
use crate::assets::AssetStore;
use crate::camera::{OrbitCamera, ThroughView, ViewPreset};
use crate::dynamics::{self, MotionPaths, Replay};
use crate::export;
use crate::gizmo::{Moving, OpKind, Session, Space};
use crate::history::{History, PoseChange};
use crate::import::{self, Imported};
use crate::library;
use crate::math::Xf;
use crate::modifiers::{self, ModifierKind};
use crate::onion::{self, OnionSkin};
use crate::plugins::{Plugins, Workspace};
use crate::posing::{self, KeyCopy, PoseCopy};
use crate::project::{self, Project};
use crate::render::ViewportRenderer;
use crate::scene::{BoneId, BoneKind, FolderId, PartId, PartShape, RestPose, RigId, RigKind, RigState, Scene};
use crate::sequences;
use crate::settings::{Action, GraphicsApi, ImageFit, Settings, Shading, StartupMarker};

/// How big the window opens, where the screen has the room.
pub const WINDOW_SIZE: [f32; 2] = [1600.0, 900.0];
/// Roblox's default for a new camera.
const DEFAULT_FOV: f32 = 70.0;
/// A new shape's size in studs, about as wide as a character.
const SHAPE_SIZE: f32 = 2.0;
/// How often changed work gets a backup.
const BACKUP_EVERY: Duration = Duration::from_secs(300);
/// Frames to wait before taking a file's picture, so the view has settled.
const THUMBNAIL_WAIT: u8 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    None,
    Rig(RigId),
    Bone(BoneId),
    Part(PartId),
    Folder(FolderId),
    /// A rig's onion skin, from its row in the Workspace.
    Onion(RigId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Move,
    Rotate,
}

impl Tool {
    pub fn op(self) -> Option<OpKind> {
        match self {
            Tool::Select => None,
            Tool::Move => Some(OpKind::Move),
            Tool::Rotate => Some(OpKind::Rotate),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropsTab {
    Item,
    Modifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BottomTab {
    Timeline,
    Graph,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RowKey {
    Rig(RigId),
    Folder(FolderId),
    Bone(BoneId),
    Parts(RigId),
    /// A modifier card, by its bone and which kind it is.
    Modifier(BoneId, u8),
}

/// The shapes a camera's frame can be previewed in. Roblox players bring any window
/// shape, so this belongs to the preview and not the camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameShape {
    Wide,
    Standard,
    Ultrawide,
    Square,
    Phone,
}

impl FrameShape {
    pub const ALL: [FrameShape; 5] = [FrameShape::Wide, FrameShape::Standard, FrameShape::Ultrawide, FrameShape::Square, FrameShape::Phone];

    pub fn label(self) -> &'static str {
        match self {
            FrameShape::Wide => "16:9",
            FrameShape::Standard => "4:3",
            FrameShape::Ultrawide => "21:9",
            FrameShape::Square => "1:1",
            FrameShape::Phone => "9:16",
        }
    }

    pub fn ratio(self) -> f32 {
        match self {
            FrameShape::Wide => 16.0 / 9.0,
            FrameShape::Standard => 4.0 / 3.0,
            FrameShape::Ultrawide => 21.0 / 9.0,
            FrameShape::Square => 1.0,
            FrameShape::Phone => 9.0 / 16.0,
        }
    }
}

pub struct CameraView {
    /// The camera the preview and Camera View look through.
    pub camera: Option<BoneId>,
    /// The small picture in the viewport's corner.
    pub preview: bool,
    /// Looking through the camera in the whole viewport.
    pub through: bool,
    pub shape: FrameShape,
    pub inset_width: f32,
    /// How far the picture has been dragged from the bottom right corner.
    pub inset_offset: egui::Vec2,
}

impl Default for CameraView {
    fn default() -> Self {
        Self {
            camera: None,
            preview: false,
            through: false,
            shape: FrameShape::Wide,
            inset_width: 300.0,
            inset_offset: egui::Vec2::ZERO,
        }
    }
}

/// A folder being renamed in the Workspace.
pub struct Rename {
    pub folder: FolderId,
    pub text: String,
    /// The field still has to take the keyboard, on the frame it first shows.
    pub focus: bool,
}

/// A slider or field being changed. It becomes one undo step when it settles.
struct LiveEdit {
    label: String,
    before: (RigState, Animation),
    /// The bone whose extra number is changing, which Auto Key may need to key.
    keyed_value: Option<BoneId>,
}

/// What the Handle Type menu can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleKind {
    Auto(HandleType),
    Aligned,
    Free,
}

pub struct ViewSettings {
    pub show_parts: bool,
    pub show_bones: bool,
    pub bones_on_top: bool,
    pub space: Space,
    pub show_grid_floor_y: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Warning,
    Error,
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub until: Instant,
}

type ImportResult = (PathBuf, Result<Imported, String>);
type BackgroundResult = (String, Result<egui::ColorImage, String>);
/// Where a project went and what it held, or `None` when the dialog was cancelled.
type SaveResult = Option<(PathBuf, Result<u64, String>)>;
type OpenResult = (PathBuf, Result<Project, String>);
type ReplayResult = (u64, Replay);

/// Everything the onion skin copies depend on, apart from the keys.
#[derive(Clone, PartialEq)]
struct OnionInputs {
    frame: i32,
    state: RigState,
    /// Poses the keys don't set. Played keys change poses every frame, so while playing
    /// only these count.
    poses: Vec<(Xf, f32)>,
    replay: Option<u64>,
}

/// The copies last worked out, kept until something they depend on changes. Orbiting or
/// sitting still then costs nothing.
#[derive(Default)]
struct OnionCache {
    inputs: Option<OnionInputs>,
    /// Shares its keys with the real animation, so comparing is cheap until one changes.
    animation: Option<Animation>,
    ghosts: Vec<onion::Ghost>,
}

/// Something that would lose the current work, once unsaved work has been dealt with.
#[derive(Clone, Debug, PartialEq)]
pub enum Leave {
    Close,
    New,
    /// Opens this project or rig, or asks which project.
    Open(Option<PathBuf>),
    /// Closes so the updater can replace Kaye's files.
    Update,
}

/// What's left in the scene, for the status bar and the Statistics section.
#[derive(Clone, Copy, Debug)]
pub struct SceneCounts {
    pub parts: usize,
    /// Real joints, leaving out cameras, shapes and targets.
    pub bones: usize,
    /// How long the lists were when counted, so a scene that grew is counted again.
    of: (usize, usize),
}

/// Counts as loading for as long as it's alive, even if the thread holding it panics.
struct LoadGuard(Arc<AtomicUsize>);

impl LoadGuard {
    fn new(loads: &Arc<AtomicUsize>) -> Self {
        loads.fetch_add(1, Ordering::SeqCst);
        Self(loads.clone())
    }
}

impl Drop for LoadGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct KayeApp {
    pub scene: Scene,
    pub selection: Selection,
    /// Bones selected along with the active one in `selection`.
    pub extra_bones: BTreeSet<BoneId>,
    /// Rigs and folders selected in the Workspace along with the active one. Cameras and
    /// shapes are listed by their bone, so they're in `extra_bones`.
    pub extra_rigs: BTreeSet<RigId>,
    pub extra_folders: BTreeSet<FolderId>,
    /// The Workspace row a Shift click selects from.
    pub outliner_anchor: Option<Selection>,
    /// Where a drag-box selection in the viewport began.
    pub box_select: Option<egui::Pos2>,
    pub hover: Selection,
    /// The Workspace owns `hover` this frame, so the viewport must not clear it.
    pub outliner_hovering: bool,
    pub history: History,
    pub camera: OrbitCamera,
    pub view: ViewSettings,
    pub tool: Tool,
    pub session: Option<Session>,
    /// A modal confirm happens on press, so its release must not also select.
    swallow_click: bool,
    /// Built on another thread, since shaders can take a while. Nothing but the
    /// loading screen draws until it arrives.
    renderer: Option<ViewportRenderer>,
    renderer_rx: Receiver<ViewportRenderer>,
    loader: loading::Loader,
    /// How many things are loading right now, which picking a file doesn't count as.
    loads: Arc<AtomicUsize>,
    /// Made the first time a camera preview is shown, since most sessions won't need it.
    preview_renderer: Option<ViewportRenderer>,
    pub camera_view: CameraView,
    live_edit: Option<LiveEdit>,
    /// Frames left before this launch counts as having started safely.
    startup_frames: u8,
    import_tx: Sender<ImportResult>,
    import_rx: Receiver<ImportResult>,
    importing: usize,
    /// The file this project was last saved to or opened from.
    project_path: Option<PathBuf>,
    /// `project::fingerprint` of the work as last saved.
    saved_work: u64,
    /// `project::fingerprint` of the work in the last backup.
    backed_up: u64,
    last_backup: Instant,
    /// The file whose picture is wanted, and how many frames are left to wait.
    thumbnail_due: Option<(PathBuf, u8)>,
    /// Asking what to do about unsaved work before doing this.
    pub leaving: Option<Leave>,
    /// A save is under way, and this comes after it.
    after_save: Option<Leave>,
    /// Has already been dealt with, so the window can really go.
    closing: bool,
    saving: usize,
    save_tx: Sender<SaveResult>,
    save_rx: Receiver<SaveResult>,
    open_tx: Sender<OpenResult>,
    open_rx: Receiver<OpenResult>,
    pub toasts: Vec<Toast>,
    pub home: home::Home,
    pub connect: connect::Connect,
    pub update: update_popup::UpdatePopup,
    pub about_open: bool,
    /// The window has been checked against the screen it opened on.
    fitted_to_screen: bool,
    pub prefs_open: bool,
    pub prefs: preferences::PrefsState,
    pub settings: Settings,
    /// What's currently applied and saved, for spotting changes.
    applied: Settings,
    pub gpu_name: String,
    pub gpu_backend: String,
    background: Option<egui::TextureHandle>,
    background_tx: Sender<BackgroundResult>,
    background_rx: Receiver<BackgroundResult>,
    /// A right click that cancelled a drag shouldn't also open a menu.
    pub suppress_menu: bool,
    pub animation: Animation,
    /// The time being shown. Fractional only during playback.
    pub frame: f32,
    pub playing: bool,
    pub timeline: timeline::TimelineState,
    pub props_tab: PropsTab,
    pub bottom_tab: BottomTab,
    pub outliner_filter: String,
    /// A folder whose name is being typed in its row.
    pub renaming: Option<Rename>,
    /// A rig being dragged out of the window, see `dragout.rs`.
    drag_out: dragout::DragOut,
    pub collapsed: HashSet<RowKey>,
    pub panel_edit: Option<(BoneId, Xf)>,
    /// The last pose copied, with Ctrl+C or from a menu.
    pub pose_clipboard: PoseCopy,
    /// The last keys copied, with Ctrl+C or from a menu.
    pub key_clipboard: KeyCopy,
    /// Keys were copied after the last pose, so they're what Ctrl+V pastes.
    pasting_keys: bool,
    /// The timeline's selection last frame, for telling when it changes.
    keys_seen: HashSet<(BoneId, i32)>,
    /// Keys were picked more recently than anything else, so Delete and Ctrl+C mean them.
    keys_picked_last: bool,
    pub assets: AssetStore,
    /// Where everything sits with no pose, which skinned meshes are bound to. Kept
    /// because working it out costs a whole evaluation.
    rest_pose: RefCell<Option<Rc<RestPose>>>,
    /// Roblox's refusal of the key has been shown, so it isn't said on every asset.
    key_warned: bool,
    plugins: Option<Plugins>,
    pub scroll_to_selection: bool,
    pub viewport_rect: Rect,
    /// The bones showing a motion path.
    pub motion_paths: Vec<BoneId>,
    /// Where those paths run, from the last replay.
    pub motion_path_points: MotionPaths,
    /// `dynamics::signature` of what the last finished replay was given.
    replay_signature: Option<u64>,
    /// One is running on another thread, and any change waits for it to finish.
    replaying: bool,
    replay_tx: Sender<ReplayResult>,
    replay_rx: Receiver<ReplayResult>,
    /// Where each onion skin's copies stand, see `sync_onion`.
    onion: OnionCache,
    /// How many parts and bones are left. The status bar shows it every frame, and counting a
    /// big model means a whole pass over it, so it's kept until something is added or taken out.
    counts: std::cell::Cell<Option<SceneCounts>>,
    /// The 3D view's list of parts from the last frame, handed back to be filled again.
    spare_parts: RefCell<Vec<crate::render::DrawItem>>,
    /// The parts with a mesh or texture to fetch, and how many parts there were when listed.
    pub(crate) asset_parts: Option<(usize, Vec<PartId>)>,
    #[cfg(feature = "devtools")]
    script: Option<crate::devscript::DevScript>,
}

impl KayeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Vec<PathBuf>, settings: Settings, graphics_failed: Option<GraphicsApi>) -> Self {
        theme::apply(&cc.egui_ctx);
        theme::apply_settings(&cc.egui_ctx, &settings.theme, settings.interface.tooltips);
        cc.egui_ctx.set_zoom_factor(settings.interface.ui_scale);
        let render_state: &egui_wgpu::RenderState = cc
            .wgpu_render_state
            .as_ref()
            .expect("Kaye needs the wgpu renderer");
        let (import_tx, import_rx) = channel();
        let (background_tx, background_rx) = channel();
        let (renderer_tx, renderer_rx) = channel();
        let (save_tx, save_rx) = channel();
        let (open_tx, open_rx) = channel();
        let (replay_tx, replay_rx) = channel();
        let wake = cc.egui_ctx.clone();
        let mut assets = AssetStore::new(crate::settings::assets_folder(), settings.system.roblox_api_key.clone(), Arc::new(move || wake.request_repaint()));
        assets.provide(crate::materials::GRID_ID, crate::materials::baseplate_grid());
        let (device, repaint) = (render_state.device.clone(), cc.egui_ctx.clone());
        let (antialiasing, shadows) = (settings.system.antialiasing, settings.system.shadow_resolution);
        std::thread::spawn(move || {
            let _ = renderer_tx.send(ViewportRenderer::new(&device, antialiasing, shadows));
            repaint.request_repaint();
        });
        let mut app = Self {
            scene: Scene::default(),
            selection: Selection::None,
            extra_bones: BTreeSet::new(),
            extra_rigs: BTreeSet::new(),
            extra_folders: BTreeSet::new(),
            outliner_anchor: None,
            box_select: None,
            hover: Selection::None,
            outliner_hovering: false,
            history: History::default(),
            camera: OrbitCamera::default(),
            view: ViewSettings {
                show_parts: true,
                show_bones: true,
                bones_on_top: true,
                space: Space::Global,
                show_grid_floor_y: 0.0,
            },
            // Rotating is most of what posing a Roblox rig is, so clicking a
            // bone should immediately show something to grab.
            tool: Tool::Rotate,
            session: None,
            swallow_click: false,
            renderer: None,
            renderer_rx,
            loader: loading::Loader::new(),
            loads: Arc::new(AtomicUsize::new(0)),
            preview_renderer: None,
            camera_view: CameraView::default(),
            live_edit: None,
            startup_frames: 3,
            import_tx,
            import_rx,
            importing: 0,
            project_path: None,
            saved_work: 0,
            backed_up: 0,
            last_backup: Instant::now(),
            thumbnail_due: None,
            leaving: None,
            after_save: None,
            closing: false,
            saving: 0,
            save_tx,
            save_rx,
            open_tx,
            open_rx,
            toasts: Vec::new(),
            home: home::Home::new(),
            connect: connect::Connect::new(&settings.system.roblox_api_key),
            update: update_popup::UpdatePopup::start(&cc.egui_ctx),
            about_open: false,
            fitted_to_screen: false,
            prefs_open: false,
            prefs: preferences::PrefsState::default(),
            gpu_name: render_state.adapter.get_info().name,
            gpu_backend: backend_label(render_state.adapter.get_info().backend),
            background: None,
            background_tx,
            background_rx,
            suppress_menu: false,
            animation: new_animation(&settings),
            applied: settings.clone(),
            settings,
            frame: 0.0,
            playing: false,
            timeline: timeline::TimelineState::default(),
            props_tab: PropsTab::Item,
            bottom_tab: BottomTab::Timeline,
            outliner_filter: String::new(),
            renaming: None,
            drag_out: dragout::DragOut::default(),
            collapsed: HashSet::new(),
            panel_edit: None,
            pose_clipboard: PoseCopy::new(),
            key_clipboard: KeyCopy::new(),
            pasting_keys: false,
            keys_seen: HashSet::new(),
            keys_picked_last: false,
            assets,
            rest_pose: RefCell::new(None),
            key_warned: false,
            plugins: None,
            scroll_to_selection: false,
            viewport_rect: Rect::NOTHING,
            motion_paths: Vec::new(),
            motion_path_points: MotionPaths::default(),
            replay_signature: None,
            replaying: false,
            replay_tx,
            replay_rx,
            onion: OnionCache::default(),
            counts: std::cell::Cell::new(None),
            spare_parts: RefCell::new(Vec::new()),
            asset_parts: None,
            #[cfg(feature = "devtools")]
            script: crate::devscript::DevScript::from_env(),
        };
        // A scripted run has to keep going when another window covers this one.
        #[cfg(feature = "devtools")]
        if app.script.is_some() {
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                loop {
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
                    ctx.request_repaint();
                    std::thread::sleep(Duration::from_millis(8));
                }
            });
        }
        app.sync_settings();
        if let Some(path) = app.settings.theme.background_image.clone() {
            app.load_background(&cc.egui_ctx, path);
        }
        if let Some(api) = graphics_failed {
            app.toast(ToastKind::Warning, format!("{} couldn't start, so the graphics API went back to Auto.", api.label()));
        }
        app.reload_plugins();
        app.import_paths(&cc.egui_ctx, open);
        app
    }

    /// Starts fresh, so a plugin that's been edited or removed doesn't linger.
    pub fn reload_plugins(&mut self) {
        match Plugins::new() {
            Ok(mut plugins) => {
                if let Some(folder) = crate::settings::plugins_folder() {
                    for failure in plugins.load_folder(&folder) {
                        self.toast(ToastKind::Error, failure);
                    }
                }
                self.plugins = Some(plugins);
            }
            Err(error) => self.toast(ToastKind::Error, format!("Plugins couldn't start: {error}")),
        }
    }

    pub fn plugin_commands(&self) -> Vec<String> {
        self.plugins.as_ref().map_or_else(Vec::new, Plugins::command_names)
    }

    /// Runs a plugin's command as one undo step. If it fails, nothing it did stays.
    pub fn run_plugin(&mut self, command: usize) {
        let Some(mut plugins) = self.plugins.take() else {
            return;
        };
        // A plugin can take anything out, so what's left gets counted again.
        self.counts.set(None);
        self.pause();
        self.settle_edits();
        let before = (self.scene.rig_state(), self.animation.clone());
        let old_poses: Vec<Xf> = self.scene.bones.iter().map(|b| b.pose).collect();
        let selected = self.key_targets();
        let rig = self.focused_rig();
        let workspace = Workspace {
            scene: std::mem::take(&mut self.scene),
            animation: std::mem::take(&mut self.animation),
            selected,
            rig,
            frame: self.whole_frame(),
            messages: Vec::new(),
        };
        let (workspace, result) = plugins.run(command, workspace);
        self.plugins = Some(plugins);
        self.scene = workspace.scene;
        self.animation = workspace.animation;
        for message in workspace.messages {
            self.toast(ToastKind::Warning, message);
        }
        if let Err(error) = result {
            self.toast(ToastKind::Error, error);
            self.scene.restore_rig_state(&before.0);
            self.animation = before.1;
            for (bone, pose) in old_poses.into_iter().enumerate() {
                self.scene.bones[bone].pose = pose;
            }
            self.animation.apply(&mut self.scene, self.frame);
            return;
        }
        self.frame = workspace.frame as f32;
        let changes: Vec<PoseChange> = old_poses
            .into_iter()
            .enumerate()
            .filter(|(bone, pose)| !self.scene.bones[*bone].pose.approx_eq(*pose))
            .map(|(bone, pose)| PoseChange { bone, before: pose, after: self.scene.bones[bone].pose })
            .collect();
        let after = (self.scene.rig_state(), self.animation.clone());
        if !changes.is_empty() || after != before {
            self.history.push_all("Run plugin", changes, before, after);
        }
        self.prune_selected_keys();
    }

    pub fn is_importing(&self) -> bool {
        self.importing > 0
    }

    /// The 3D view's renderer has arrived, so the editor can draw.
    pub fn has_renderer(&self) -> bool {
        self.renderer.is_some()
    }

    /// How many parts and bones are left in the scene. Anything that deletes or brings back
    /// goes through `drop_removed_selection`, which forgets the count, and anything added
    /// makes the lists longer.
    pub fn counts(&self) -> SceneCounts {
        let sizes = (self.scene.parts.len(), self.scene.bones.len());
        if let Some(counts) = self.counts.get().filter(|c| c.of == sizes) {
            return counts;
        }
        let scene = &self.scene;
        let counts = SceneCounts {
            parts: scene.live_parts(),
            bones: scene.bones.iter().filter(|b| b.kind.is_joint() && !b.removed).count(),
            of: (scene.parts.len(), scene.bones.len()),
        };
        self.counts.set(Some(counts));
        counts
    }

    /// The rest pose, worked out again only when the scene has grown.
    pub fn rest_pose(&self) -> Rc<RestPose> {
        let mut kept = self.rest_pose.borrow_mut();
        let fits = kept.as_ref().is_some_and(|r| r.bones.len() == self.scene.bones.len() && r.parts.len() == self.scene.parts.len());
        if !fits {
            *kept = Some(Rc::new(self.scene.rest_pose()));
        }
        kept.clone().expect("just filled in")
    }

    fn poll_assets(&mut self) {
        self.assets.set_key(&self.settings.system.roblox_api_key);
        self.assets.poll();
        if !self.assets.key_refused {
            self.key_warned = false;
        } else if !self.key_warned {
            self.key_warned = true;
            self.toast(ToastKind::Warning, "Roblox refused the API key, so meshes stay as boxes.");
        }
    }

    pub fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        self.toasts.push(Toast {
            text: text.into(),
            kind,
            until: Instant::now() + Duration::from_secs(8),
        });
    }

    pub fn selected_bone(&self) -> Option<BoneId> {
        match self.selection {
            Selection::Bone(b) => Some(b),
            _ => None,
        }
    }

    pub fn select(&mut self, selection: Selection) {
        if selection != self.selection {
            self.commit_panel_edit();
            self.commit_live_edit();
        }
        self.selection = selection;
        self.clear_extras();
        self.keys_picked_last = false;
        self.outliner_anchor = (selection != Selection::None).then_some(selection);
        if let Selection::Bone(b) = selection {
            if self.scene.bones[b].kind == BoneKind::Camera {
                self.camera_view.camera = Some(b);
            }
        }
    }

    /// The bone a selection stands for, reaching through a selected part.
    fn selection_bone(&self) -> Option<BoneId> {
        match self.selection {
            Selection::Bone(b) => Some(b),
            Selection::Part(p) => self.scene.part_owner(p),
            _ => None,
        }
    }

    /// The joint the Modifiers tab works on. A target stands for the bone it drives.
    pub fn modifier_bone(&self) -> Option<BoneId> {
        let owner = modifiers::owner_of(&self.scene, self.selection_bone()?);
        self.scene.bones[owner].kind.is_joint().then_some(owner)
    }

    /// The bones whose curves the Graph tab shows, with the active one first.
    pub fn curve_bones(&self) -> Vec<BoneId> {
        let mut bones = Vec::new();
        for bone in self.selection_bone().into_iter().chain(self.extra_bones.iter().copied()) {
            let control = modifiers::control_of(&self.scene, bone);
            if !bones.contains(&control) {
                bones.push(control);
            }
        }
        bones
    }

    /// Every selected bone, with the active one first.
    pub fn selected_bones(&self) -> Vec<BoneId> {
        let mut bones: Vec<BoneId> = self.selected_bone().into_iter().collect();
        bones.extend(self.extra_bones.iter().copied());
        bones
    }

    pub fn is_bone_selected(&self, bone: BoneId) -> bool {
        self.selected_bone() == Some(bone) || self.extra_bones.contains(&bone)
    }

    /// Whether this is the active selection or one of the extras with it.
    pub fn is_selected(&self, selection: Selection) -> bool {
        match selection {
            Selection::None => false,
            Selection::Bone(b) => self.is_bone_selected(b),
            Selection::Rig(r) => self.selection == selection || self.extra_rigs.contains(&r),
            Selection::Folder(f) => self.selection == selection || self.extra_folders.contains(&f),
            Selection::Part(_) | Selection::Onion(_) => self.selection == selection,
        }
    }

    fn clear_extras(&mut self) {
        self.extra_bones.clear();
        self.extra_rigs.clear();
        self.extra_folders.clear();
    }

    /// Everything selected that isn't the active one.
    fn extras(&self) -> Vec<Selection> {
        let bones = self.extra_bones.iter().map(|b| Selection::Bone(*b));
        let rigs = self.extra_rigs.iter().map(|r| Selection::Rig(*r));
        bones.chain(rigs).chain(self.extra_folders.iter().map(|f| Selection::Folder(*f))).collect()
    }

    /// Adds to the extras. A part can only be selected alone.
    fn add_extra(&mut self, selection: Selection) -> bool {
        match selection {
            Selection::Bone(b) => self.extra_bones.insert(b),
            Selection::Rig(r) => self.extra_rigs.insert(r),
            Selection::Folder(f) => self.extra_folders.insert(f),
            Selection::Part(_) | Selection::Onion(_) | Selection::None => false,
        }
    }

    fn remove_extra(&mut self, selection: Selection) -> bool {
        match selection {
            Selection::Bone(b) => self.extra_bones.remove(&b),
            Selection::Rig(r) => self.extra_rigs.remove(&r),
            Selection::Folder(f) => self.extra_folders.remove(&f),
            Selection::Part(_) | Selection::Onion(_) | Selection::None => false,
        }
    }

    /// Hands being active to the next thing selected, with the active one gone.
    fn promote_extra(&mut self) {
        let next = self.extras().into_iter().next();
        self.commit_panel_edit();
        match next {
            Some(next) => {
                self.remove_extra(next);
                self.selection = next;
            }
            None => self.selection = Selection::None,
        }
    }

    /// Adds a bone to the selection, or takes it out if it's already in.
    pub fn toggle_bone(&mut self, bone: BoneId) {
        self.toggle(Selection::Bone(bone));
    }

    /// Adds anything in the Workspace to the selection, or takes it out if it's already
    /// in, as Ctrl does in Roblox Studio's Explorer.
    pub fn toggle(&mut self, selection: Selection) {
        self.keys_picked_last = false;
        if selection == Selection::None {
            return;
        }
        self.outliner_anchor = Some(selection);
        if self.selection == selection {
            self.promote_extra();
        } else if !self.remove_extra(selection) {
            let alone = matches!(selection, Selection::Part(_) | Selection::Onion(_)) || matches!(self.selection, Selection::Part(_) | Selection::Onion(_) | Selection::None);
            if alone || !self.add_extra(selection) {
                self.select(selection);
            }
        }
    }

    /// Selects a run of Workspace rows, as Shift does in Roblox Studio's Explorer, with
    /// `active` the one clicked. Adding keeps what was selected.
    pub fn select_run(&mut self, run: Vec<Selection>, active: Selection, add: bool) {
        let anchor = self.outliner_anchor;
        if !add {
            self.select(active);
        } else if !self.is_selected(active) {
            self.toggle(active);
        }
        if self.selection != active && self.remove_extra(active) {
            let previous = self.selection;
            self.commit_panel_edit();
            self.selection = active;
            self.add_extra(previous);
        }
        for selection in run {
            if selection != self.selection {
                self.add_extra(selection);
            }
        }
        // The next Shift click runs from the same row, as Explorer does.
        self.outliner_anchor = anchor;
        self.keys_picked_last = false;
    }

    /// What dragging out of the Workspace takes along: each rig, camera and shape selected,
    /// and each folder, leaving out anything already inside a folder that's going.
    pub fn selected_items(&self) -> Vec<export::Item> {
        let scene = &self.scene;
        let mut items: Vec<export::Item> = Vec::new();
        for selection in std::iter::once(self.selection).chain(self.extras()) {
            let item = match selection {
                Selection::Rig(r) => Some(export::Item::Rig(r)),
                Selection::Bone(b) if scene.rigs[scene.bones[b].rig].kind.is_single() => Some(export::Item::Rig(scene.bones[b].rig)),
                Selection::Folder(f) => Some(export::Item::Folder(f)),
                _ => None,
            };
            if let Some(item) = item.filter(|i| !items.contains(i)) {
                items.push(item);
            }
        }
        let folders: Vec<FolderId> = items.iter().filter_map(|i| if let export::Item::Folder(f) = i { Some(*f) } else { None }).collect();
        let inside = |within: Option<FolderId>| within.is_some_and(|w| folders.iter().any(|f| scene.folder_is_within(w, *f)));
        items.retain(|item| match *item {
            export::Item::Rig(r) => !inside(scene.rig_folder(r)),
            export::Item::Folder(f) => !inside(scene.folders[f].parent),
        });
        items
    }

    /// Applies a box selection. Adding keeps what was selected; otherwise the box
    /// replaces it, and the active bone stays active if the box caught it.
    pub fn select_bones(&mut self, bones: Vec<BoneId>, add: bool) {
        let mut all = if add { self.selected_bones() } else { Vec::new() };
        for bone in bones {
            if !all.contains(&bone) {
                all.push(bone);
            }
        }
        self.commit_panel_edit();
        self.keys_picked_last = false;
        // A rig or folder picked in the Workspace stays active when the box adds to it.
        if add && matches!(self.selection, Selection::Rig(_) | Selection::Folder(_)) {
            self.extra_bones = all.into_iter().collect();
            return;
        }
        let active = self.selected_bone().filter(|b| all.contains(b)).or(all.first().copied());
        if !add {
            self.extra_rigs.clear();
            self.extra_folders.clear();
        }
        self.selection = active.map_or(Selection::None, Selection::Bone);
        self.extra_bones = all.into_iter().filter(|b| Some(*b) != active).collect();
    }

    /// The rig the selection is in, or the first rig when nothing's selected.
    pub fn focused_rig(&self) -> Option<RigId> {
        match self.selection {
            Selection::Bone(b) => Some(self.scene.bones[b].rig),
            Selection::Part(p) => Some(self.scene.parts[p].rig),
            Selection::Rig(r) | Selection::Onion(r) => Some(r),
            Selection::Folder(f) => {
                let inside = self.scene.rigs_in_folder(f).into_iter().find(|r| !self.scene.rigs[*r].kind.is_single());
                inside.or_else(|| self.scene.rigs.iter().position(|r| !r.kind.is_single() && !r.removed))
            }
            Selection::None => self.scene.rigs.iter().position(|r| !r.kind.is_single() && !r.removed),
        }
    }

    /// The rig a drag would move, when a whole rig is what's selected.
    pub fn movable_rig(&self) -> Option<RigId> {
        match self.selection {
            // A camera's rig is its root, which moves the camera's whole path.
            Selection::Rig(r) if self.scene.rigs[r].kind != RigKind::Shape => Some(r),
            _ => None,
        }
    }

    pub fn select_all_bones(&mut self) {
        if let Some(rig) = self.focused_rig() {
            let bones = self.scene.joints(rig);
            self.select_bones(bones, false);
        }
    }

    /// Starts a drag on the active bone, carrying the rest of the selection along.
    pub fn start_session(&mut self, bone: BoneId, op: OpKind, lock: Option<crate::gizmo::AxisLock>, modal: bool, mouse: egui::Pos2) {
        self.pause();
        let bone = modifiers::control_of(&self.scene, bone);
        let selected = self.controlled_bones();
        self.session = Some(Session::new(&self.scene, bone, op, lock, modal, mouse).with_extras(&self.scene, &selected));
    }

    /// Starts a drag of a whole rig, which is left out of the animation.
    pub fn start_rig_session(&mut self, rig: RigId, op: OpKind, lock: Option<crate::gizmo::AxisLock>, modal: bool, mouse: egui::Pos2) {
        self.pause();
        self.session = Some(Session::for_rig(&self.scene, rig, op, lock, modal, mouse));
    }

    /// The selected bones, with any under IK swapped for their targets.
    fn controlled_bones(&self) -> Vec<BoneId> {
        let mut bones: Vec<BoneId> = Vec::new();
        for bone in self.selected_bones() {
            let control = modifiers::control_of(&self.scene, bone);
            if !bones.contains(&control) {
                bones.push(control);
            }
        }
        bones
    }

    /// Makes a change to modifiers or added objects as one undo step.
    pub fn edit_rig(&mut self, label: &str, edit: impl FnOnce(&mut Scene, &mut Animation)) {
        self.commit_live_edit();
        self.commit_panel_edit();
        let before = (self.scene.rig_state(), self.animation.clone());
        edit(&mut self.scene, &mut self.animation);
        self.animation.apply(&mut self.scene, self.frame);
        let after = (self.scene.rig_state(), self.animation.clone());
        if after != before {
            self.history.push_rig(label, before, after);
        }
        self.drop_removed_selection();
    }

    /// Makes a change that rewrites poses as well as modifiers, as one undo step. With
    /// Auto Key on, every bone it changed is keyed as a single step on this frame, so
    /// the frames before stay as they were.
    fn edit_switch(&mut self, label: &str, edit: impl FnOnce(&mut Scene, &mut Animation)) {
        self.pause();
        self.commit_live_edit();
        self.commit_panel_edit();
        let before = (self.scene.rig_state(), self.animation.clone());
        let old: Vec<(Xf, f32)> = self.scene.bones.iter().map(|b| (b.pose, b.value)).collect();
        edit(&mut self.scene, &mut self.animation);
        let frame = self.whole_frame();
        let mut changes = Vec::new();
        for (bone, (pose, value)) in old.into_iter().enumerate() {
            let now = &self.scene.bones[bone];
            let (new_pose, new_value) = (now.pose, now.value);
            if new_pose.approx_eq(pose) && new_value == value {
                continue;
            }
            changes.push(PoseChange { bone, before: pose, after: new_pose });
            if self.settings.animation.auto_key {
                self.animation.key_step(bone, frame, pose, value, new_pose, new_value);
            }
        }
        if self.settings.animation.auto_key {
            self.animation.apply(&mut self.scene, self.frame);
        }
        let after = (self.scene.rig_state(), self.animation.clone());
        self.history.push_all(label, changes, before, after);
        self.prune_selected_keys();
    }

    fn modifier_of(&self, bone: BoneId, kind: u8) -> Option<usize> {
        self.scene.modifiers.iter().position(|m| m.bone == bone && m.kind.index() == kind)
    }

    pub fn ik_is_on(&self, bone: BoneId) -> bool {
        self.modifier_of(bone, 0).is_some_and(|i| modifiers::is_on(&self.scene, i))
    }

    pub fn unparent_is_on(&self, bone: BoneId) -> bool {
        self.modifier_of(bone, 1).is_some_and(|i| modifiers::is_on(&self.scene, i))
    }

    /// Hands a limb to IK or FK, leaving it exactly where it is.
    pub fn switch_ik(&mut self, bone: BoneId, on: bool) {
        let Some(index) = self.modifier_of(bone, 0).filter(|i| modifiers::is_on(&self.scene, *i) != on) else {
            return;
        };
        let label = if on { "Switch to IK" } else { "Switch to FK" };
        let (frame, auto_key) = (self.whole_frame(), self.settings.animation.auto_key);
        self.edit_switch(label, |scene, animation| {
            let ModifierKind::Ik(ik) = scene.modifiers[index].kind else {
                return;
            };
            let old_value = scene.bones[ik.target].value;
            modifiers::switch_ik(scene, index, on);
            // The target's later keys were made for the old setting, and would ease back.
            if auto_key {
                animation.carry_switch(ik.target, frame, old_value, scene.bones[ik.target].value, Xf::IDENTITY);
            }
        });
    }

    /// Lets a bone go of its parent or takes it back, leaving it exactly where it is.
    pub fn switch_unparent(&mut self, bone: BoneId, on: bool) {
        let Some(index) = self.modifier_of(bone, 1).filter(|i| modifiers::is_on(&self.scene, *i) != on) else {
            return;
        };
        let label = if on { "Unparent" } else { "Parent" };
        let (old_value, frame, auto_key) = (self.scene.bones[bone].value, self.whole_frame(), self.settings.animation.auto_key);
        self.edit_switch(label, |scene, animation| {
            let change = modifiers::switch_unparent(scene, index, on);
            if auto_key {
                animation.carry_switch(bone, frame, old_value, scene.bones[bone].value, change);
            }
        });
    }

    /// Turns a modifier's result into plain keys, and takes it off.
    pub fn bake_modifier(&mut self, index: usize) {
        self.bake(vec![index]);
    }

    pub fn bake_all_modifiers(&mut self) {
        self.bake((0..self.scene.modifiers.len()).collect());
    }

    /// The rig an export would write: the one in focus, or the first that has joints. A
    /// camera or shape in focus goes as a rig of its own, see `export::single_model`.
    pub fn export_rig(&self) -> Option<RigId> {
        let has_joints = |rig: &RigId| !self.scene.joints(*rig).is_empty();
        let selected = match self.selection {
            Selection::Bone(b) => Some(self.scene.bones[b].rig),
            Selection::Rig(r) => Some(r),
            _ => None,
        };
        let single = selected.filter(|r| self.scene.rigs[*r].kind.is_single() && !self.scene.live_bones(*r).is_empty());
        single.or_else(|| self.focused_rig().filter(has_joints)).or_else(|| (0..self.scene.rigs.len()).find(has_joints))
    }

    pub fn export_animation(&mut self, pack: export::Pack) {
        let Some(rig) = self.export_rig() else {
            return;
        };
        self.pause();
        let name = self.scene.rigs[rig].name.clone();
        let Some(path) = pick_export_path(&name).map(export::with_extension) else {
            return;
        };
        self.animation.store_range();
        let written = match pack {
            export::Pack::Rig => export::write_model(&path, &self.scene, &self.animation, rig),
            export::Pack::Keyframe => export::write_all_animations(&path, &self.scene, &self.animation, rig),
            export::Pack::CurrentKeyframe => {
                let current = self.animation.active_clip(rig).map_or_else(|| self.animation.clone(), |clip| self.animation.playing_clip(&self.scene, clip));
                export::write_animation(&path, &self.scene, &current, rig)
            }
        };
        if let Err(message) = written {
            self.toast(ToastKind::Error, format!("{name} couldn't be exported: {message}"));
        }
    }

    fn bake(&mut self, indices: Vec<usize>) {
        if indices.is_empty() {
            return;
        }
        let frame = self.frame;
        self.pause();
        self.edit_rig("Bake to keys", |scene, animation| {
            posing::bake(scene, animation, &indices);
            animation.apply(scene, frame);
        });
    }

    /// Whether a new modifier should only act from here on, leaving keys already made
    /// before this frame alone.
    fn adds_partway(&self, bone: BoneId) -> bool {
        self.settings.animation.auto_key && self.whole_frame() > self.animation.start && self.animation.tracks.contains_key(&bone)
    }

    pub fn copy_pose(&mut self, ctx: &egui::Context) {
        let bones = self.key_targets();
        if !bones.is_empty() {
            self.pose_clipboard = posing::copy_pose(&self.scene, &bones);
            self.pasting_keys = false;
            // egui only reports Ctrl+V while there's text to paste, so this has to leave some.
            ctx.copy_text(format!("Kaye pose: {} bones", bones.len()));
        }
    }

    pub fn paste_pose(&mut self) {
        if let Some(rig) = self.focused_rig() {
            self.pause();
            let poses = posing::paste_pose(&self.scene, &self.pose_clipboard, rig);
            self.apply_poses("Paste pose", poses);
        }
    }

    /// Whether Delete and Ctrl+C mean the selected keys and not the selected object:
    /// the mouse is over them, or they're the last thing picked, or there's nothing else.
    fn keys_in_focus(&self) -> bool {
        !self.timeline.selected.is_empty() && (self.timeline.hovered || self.keys_picked_last || self.selection == Selection::None)
    }

    pub fn copy_keys(&mut self, ctx: &egui::Context) {
        let keys: Vec<KeyId> = self.timeline.selected.iter().copied().collect();
        self.key_clipboard = posing::copy_keys(&self.scene, &self.animation, &keys);
        self.pasting_keys = true;
        // egui only reports Ctrl+V while there's text to paste, so this has to leave some.
        ctx.copy_text(format!("Kaye keys: {}", keys.len()));
    }

    /// Pastes the copied keys so the earliest lands on the current frame, and selects them.
    pub fn paste_keys(&mut self) {
        self.pause();
        let (frame, rig) = (self.whole_frame(), self.focused_rig());
        let mut animation = self.animation.clone();
        let pasted = posing::paste_keys(&self.scene, &mut animation, &self.key_clipboard, rig, frame);
        self.timeline.selected = pasted.into_iter().collect();
        self.edit_animation("Paste keys", |a| *a = animation);
    }

    /// Pastes what was copied last, pose or keys, mirrored onto the other side of the rig.
    pub fn paste_mirrored(&mut self) {
        if self.pasting_keys {
            self.paste_keys_mirrored();
        } else {
            self.paste_pose_mirrored();
        }
    }

    /// Pastes the copied pose onto the other side of the rig, as Blender's Paste X-Flipped.
    pub fn paste_pose_mirrored(&mut self) {
        if let Some(rig) = self.focused_rig() {
            self.pause();
            let poses = posing::paste_mirrored(&self.scene, &self.pose_clipboard, rig);
            self.apply_poses("Paste mirrored pose", poses);
        }
    }

    /// Pastes the copied keys at the current frame as `paste_keys` does, but onto the bones on
    /// the other side, mirrored. The keys on the side they came from stay as they were.
    pub fn paste_keys_mirrored(&mut self) {
        self.pause();
        let (frame, rig) = (self.whole_frame(), self.focused_rig());
        let mut pasted_on = self.animation.clone();
        let pasted = posing::paste_keys(&self.scene, &mut pasted_on, &self.key_clipboard, rig, frame);
        let mirrored = posing::mirror_keys(&self.scene, &pasted_on, &pasted);
        self.insert_mirrored("Paste mirrored keys", mirrored);
    }

    /// Mirrors the selected keys onto the other side of the rig, on the same frames.
    pub fn mirror_selected_keys(&mut self) {
        let keys: Vec<KeyId> = self.timeline.selected.iter().copied().collect();
        if keys.is_empty() {
            return;
        }
        self.pause();
        let mirrored = posing::mirror_keys(&self.scene, &self.animation, &keys);
        self.insert_mirrored("Mirror keys", mirrored);
    }

    fn insert_mirrored(&mut self, label: &str, keys: Vec<(BoneId, crate::animation::Key)>) {
        self.timeline.selected = keys.iter().map(|(bone, key)| (*bone, key.frame)).collect();
        self.edit_animation(label, |a| {
            for (bone, key) in keys {
                a.tracks.entry(bone).or_default().insert(key);
            }
        });
    }

    /// Mirrors the selection across the rig's middle, onto the bones on the other side.
    pub fn mirror_pose(&mut self) {
        self.pause();
        let poses = posing::mirror_pose(&self.scene, &self.key_targets());
        self.apply_poses("Mirror pose", poses);
    }

    fn start_live_edit(&mut self, label: &str) {
        if self.live_edit.is_none() {
            self.pause();
            self.live_edit = Some(LiveEdit {
                label: label.to_string(),
                before: (self.scene.rig_state(), self.animation.clone()),
                keyed_value: None,
            });
        }
    }

    /// Changes a modifier setting as it's dragged. The undo step comes when it settles.
    pub fn touch_rig(&mut self, label: &str, edit: impl FnOnce(&mut Scene)) {
        self.start_live_edit(label);
        edit(&mut self.scene);
        self.scene.evaluate();
    }

    /// Changes a bone's extra number as it's dragged.
    pub fn touch_value(&mut self, bone: BoneId, value: f32) {
        self.touch_rig("Change value", |scene| scene.bones[bone].value = value);
        if let Some(edit) = &mut self.live_edit {
            edit.keyed_value = Some(bone);
        }
    }

    /// Changes keys from a typed field as it's dragged.
    pub fn touch_keys(&mut self, label: &str, edit: impl FnOnce(&mut Animation)) {
        self.start_live_edit(label);
        edit(&mut self.animation);
        self.animation.apply(&mut self.scene, self.frame);
    }

    fn commit_live_edit(&mut self) {
        let Some(edit) = self.live_edit.take() else {
            return;
        };
        // Auto Key only follows a number that's already animated, so moving a slider
        // once doesn't quietly start a track.
        if let Some(bone) = edit.keyed_value {
            if self.settings.animation.auto_key && self.animation.tracks.contains_key(&bone) {
                let (frame, b) = (self.whole_frame(), &self.scene.bones[bone]);
                self.animation.key_shown(bone, frame, b.pose, b.value);
            }
        }
        let after = (self.scene.rig_state(), self.animation.clone());
        if after != edit.before {
            self.history.push_rig(edit.label, edit.before, after);
        }
    }

    /// An edit lasts until the mouse is up and nothing is being typed.
    fn settle_live_edit(&mut self, ctx: &egui::Context) {
        let busy = ctx.input(|i| i.pointer.any_down()) || ctx.memory(|m| m.focused().is_some());
        if !busy {
            self.commit_live_edit();
        }
    }

    pub fn add_ik(&mut self, bone: BoneId) {
        if !self.has_ik(bone) {
            let partway = self.adds_partway(bone);
            let frame = self.whole_frame();
            self.edit_rig("Add IK", |scene, animation| {
                let target = modifiers::add_ik(scene, bone);
                if partway {
                    let pose = scene.bones[target].pose;
                    animation.key_step(target, frame, pose, 0.0, pose, 1.0);
                }
            });
        }
        self.props_tab = PropsTab::Modifiers;
    }

    pub fn add_unparent(&mut self, bone: BoneId) {
        if !self.has_unparent(bone) {
            let partway = self.adds_partway(bone);
            let frame = self.whole_frame();
            self.edit_rig("Add Unparent", |scene, animation| {
                modifiers::add_unparent(scene, bone);
                if partway {
                    let pose = scene.bones[bone].pose;
                    animation.set_value_before(bone, frame, 0.0);
                    animation.key_step(bone, frame, pose, 0.0, pose, 1.0);
                }
            });
        }
        self.props_tab = PropsTab::Modifiers;
    }

    pub fn add_look_at(&mut self, bone: BoneId) {
        if !self.has_modifier(bone, 2) {
            self.edit_rig("Add Look At", |scene, _| {
                modifiers::add_look_at(scene, bone);
            });
        }
        self.props_tab = PropsTab::Modifiers;
    }

    pub fn add_copy(&mut self, bone: BoneId) {
        if !self.has_modifier(bone, 3) {
            self.edit_rig("Add Copy Bone", |scene, _| modifiers::add_copy(scene, bone));
        }
        self.props_tab = PropsTab::Modifiers;
    }

    pub fn add_limit(&mut self, bone: BoneId) {
        if !self.has_modifier(bone, 4) {
            self.edit_rig("Add Limit Rotation", |scene, _| modifiers::add_limit(scene, bone));
        }
        self.props_tab = PropsTab::Modifiers;
    }

    pub fn add_inherit(&mut self, bone: BoneId) {
        if !self.has_modifier(bone, 5) {
            self.edit_rig("Add Inherit Rotation", |scene, _| modifiers::add_inherit(scene, bone));
        }
        self.props_tab = PropsTab::Modifiers;
    }

    pub fn add_ragdoll(&mut self, bone: BoneId) {
        if !self.has_modifier(bone, modifiers::RAGDOLL_KIND) {
            self.edit_rig("Add Ragdoll", |scene, _| modifiers::add_ragdoll(scene, bone));
        }
        self.props_tab = PropsTab::Modifiers;
    }

    /// Shows a motion path for every selected bone.
    pub fn create_motion_paths(&mut self) {
        for bone in self.selected_bones() {
            if !self.motion_paths.contains(&bone) {
                self.motion_paths.push(bone);
            }
        }
    }

    pub fn clear_motion_paths(&mut self) {
        let selected = self.selected_bones();
        self.motion_paths.retain(|bone| !selected.contains(bone));
    }

    /// Every rig the selection reaches that can have an onion skin: rigs, and the rigs of
    /// selected bones and parts, cameras left out.
    pub fn onion_rigs(&self) -> Vec<RigId> {
        let scene = &self.scene;
        let mut rigs: Vec<RigId> = Vec::new();
        let picked = std::iter::once(self.selection).chain(self.extras());
        for selection in picked {
            let rig = match selection {
                Selection::Rig(r) | Selection::Onion(r) => Some(r),
                Selection::Bone(b) => Some(scene.bones[b].rig),
                Selection::Part(p) => Some(scene.parts[p].rig),
                Selection::Folder(_) | Selection::None => None,
            };
            if let Some(rig) = rig.filter(|r| onion::can_have(scene, *r) && !rigs.contains(r)) {
                rigs.push(rig);
            }
        }
        rigs
    }

    /// Whether Add Onion Skin would add one: some rig in the selection doesn't have one yet.
    pub fn can_add_onion(&self) -> bool {
        self.onion_rigs().iter().any(|r| self.scene.rigs[*r].onion.is_none())
    }

    /// Whether Delete Onion Skin would take one away.
    pub fn can_delete_onion(&self) -> bool {
        self.onion_rigs().iter().any(|r| self.scene.rigs[*r].onion.is_some())
    }

    /// Gives every selected rig without one an onion skin, as one undo step. The selection
    /// stays put, so posing carries on, and the rig's row opens to show the new one.
    pub fn add_onion_skins(&mut self) {
        let rigs: Vec<RigId> = self.onion_rigs().into_iter().filter(|r| self.scene.rigs[*r].onion.is_none()).collect();
        if rigs.is_empty() {
            return;
        }
        let label = if rigs.len() == 1 { "Add onion skin" } else { "Add onion skins" };
        self.edit_rig(label, |scene, _| {
            for &rig in &rigs {
                scene.rigs[rig].onion = Some(OnionSkin::default());
            }
        });
        for rig in rigs {
            self.collapsed.remove(&RowKey::Rig(rig));
        }
    }

    /// Takes away the onion skin of every selected rig, as one undo step.
    pub fn delete_onion_skins(&mut self) {
        let rigs: Vec<RigId> = self.onion_rigs().into_iter().filter(|r| self.scene.rigs[*r].onion.is_some()).collect();
        if rigs.is_empty() {
            return;
        }
        let label = if rigs.len() == 1 { "Delete onion skin" } else { "Delete onion skins" };
        self.edit_rig(label, |scene, _| {
            for &rig in &rigs {
                scene.rigs[rig].onion = None;
            }
        });
    }

    /// Works the onion skin copies out again when something they depend on has changed.
    /// Runs every frame the 3D view draws, and costs a comparison when nothing has.
    fn sync_onion(&mut self) {
        if !self.scene.rigs.iter().any(|r| r.onion.is_some() && !r.removed) {
            self.onion = OnionCache::default();
            return;
        }
        let animation = &self.animation;
        let playing = self.playing;
        let inputs = OnionInputs {
            frame: self.whole_frame(),
            state: self.scene.rig_state(),
            poses: self.scene.bones.iter().enumerate().filter(|(id, _)| !playing || !animation.tracks.contains_key(id)).map(|(_, b)| (b.pose, b.value)).collect(),
            replay: self.replay_signature,
        };
        if self.onion.inputs.as_ref() == Some(&inputs) && self.onion.animation.as_ref() == Some(&self.animation) {
            return;
        }
        self.onion.ghosts = onion::ghosts(&self.scene, &self.animation, inputs.frame);
        self.onion.inputs = Some(inputs);
        self.onion.animation = Some(self.animation.clone());
    }

    /// Makes an animation the one its rig plays, as one undo step. Anything that animation
    /// doesn't key goes back to rest, so the last one's pose doesn't show through.
    pub fn switch_animation(&mut self, clip: usize) {
        self.change_animations("Switch animation", |scene, animation| {
            animation.set_active_clip(scene, clip);
            let rig = animation.clips[clip].rig;
            animation.rest_unkeyed(scene, rig);
        });
    }

    pub fn new_animation(&mut self, rig: RigId) {
        self.change_animations("New animation", |scene, animation| {
            let clip = animation.add_clip(rig);
            animation.set_active_clip(scene, clip);
            animation.rest_unkeyed(scene, rig);
        });
    }

    /// Adds a copy and goes on working on the copy, as Blender's button does.
    pub fn duplicate_animation(&mut self, clip: usize) {
        self.change_animations("Duplicate animation", |scene, animation| {
            let copy = animation.duplicate_clip(scene, clip);
            animation.set_active_clip(scene, copy);
        });
    }

    pub fn delete_animation(&mut self, clip: usize) {
        self.change_animations("Delete animation", |scene, animation| {
            let rig = animation.clips[clip].rig;
            if animation.remove_clip(scene, clip) {
                animation.rest_unkeyed(scene, rig);
            }
        });
    }

    /// Settles a typed name once it's done being typed, so no two of a rig's match.
    pub fn finish_animation_name(&mut self, clip: usize) {
        let details = &self.animation.clips[clip];
        let name = self.animation.unused_clip_name(details.rig, &details.name.clone(), Some(clip));
        if name != self.animation.clips[clip].name {
            self.touch_keys("Rename animation", |a| a.clips[clip].name = name);
        }
    }

    /// A change to which animations there are, as one undo step with any poses it moves.
    fn change_animations(&mut self, label: &str, change: impl FnOnce(&mut Scene, &mut Animation)) {
        self.pause();
        self.commit_live_edit();
        self.commit_panel_edit();
        self.animation.store_range();
        let before = (self.scene.rig_state(), self.animation.clone());
        let old: Vec<Xf> = self.scene.bones.iter().map(|b| b.pose).collect();
        change(&mut self.scene, &mut self.animation);
        self.sync_animations();
        self.animation.apply(&mut self.scene, self.frame);
        let changes = old.into_iter().enumerate().filter(|(bone, pose)| !self.scene.bones[*bone].pose.approx_eq(*pose)).map(|(bone, before)| PoseChange { bone, before, after: self.scene.bones[bone].pose }).collect();
        let after = (self.scene.rig_state(), self.animation.clone());
        self.history.push_all(label, changes, before, after);
        self.timeline.selected.clear();
        self.prune_selected_keys();
    }

    pub fn has_modifier(&self, bone: BoneId, kind: u8) -> bool {
        self.modifier_of(bone, kind).is_some()
    }

    pub fn has_ik(&self, bone: BoneId) -> bool {
        self.has_modifier(bone, 0)
    }

    pub fn has_unparent(&self, bone: BoneId) -> bool {
        self.has_modifier(bone, 1)
    }

    pub fn remove_modifier(&mut self, index: usize) {
        self.edit_rig("Remove modifier", |scene, animation| {
            for bone in modifiers::remove(scene, index) {
                animation.forget_bone(bone);
            }
        });
    }

    fn drop_removed_selection(&mut self) {
        self.counts.set(None);
        let scene = &self.scene;
        self.extra_bones.retain(|b| !scene.bones[*b].removed);
        self.extra_rigs.retain(|r| !scene.rigs[*r].removed);
        self.extra_folders.retain(|f| scene.folder_is_live(*f));
        let gone = match self.selection {
            Selection::Bone(b) => scene.bones[b].removed,
            Selection::Part(p) => scene.part_is_removed(p),
            Selection::Rig(r) => scene.rigs[r].removed,
            Selection::Folder(f) => !scene.folder_is_live(f),
            Selection::Onion(r) => scene.rigs[r].removed || scene.rigs[r].onion.is_none(),
            Selection::None => false,
        };
        if gone {
            self.promote_extra();
        }
        let animation = &self.animation;
        self.timeline.selected.retain(|(b, f)| animation.key_at(*b, *f).is_some());
    }

    /// The folder something new goes in: the one selected, as Roblox Studio inserts into
    /// what's selected.
    fn selected_folder(&self) -> Option<FolderId> {
        match self.selection {
            Selection::Folder(f) => Some(f),
            _ => None,
        }
    }

    /// Adds a folder to the Workspace and starts naming it, as a file manager does.
    pub fn new_folder(&mut self, parent: Option<FolderId>) {
        let mut folder = 0;
        self.edit_rig("New folder", |scene, _| folder = scene.add_folder(parent));
        if let Some(parent) = parent {
            self.collapsed.remove(&RowKey::Folder(parent));
        }
        self.select(Selection::Folder(folder));
        self.start_rename(folder);
        self.scroll_to_selection = true;
    }

    /// Turns a folder's row into a field for typing its name.
    pub fn start_rename(&mut self, folder: FolderId) {
        let text = self.scene.folders[folder].name.clone();
        self.renaming = Some(Rename { folder, text, focus: true });
        self.scroll_to_selection = true;
    }

    pub fn rename_folder(&mut self, folder: FolderId, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() || name == self.scene.folders[folder].name {
            return;
        }
        self.edit_rig("Rename folder", |scene, _| scene.folders[folder].name = name);
    }

    /// Lists rigs, cameras, shapes and folders in a folder, or at the top with `None`, as
    /// one undo step. Only the list changes, so nothing in the scene moves. A folder can't
    /// go inside itself, and stays where it is.
    pub fn move_items_to(&mut self, items: &[export::Item], folder: Option<FolderId>) {
        let scene = &self.scene;
        let moves = |item: &export::Item| match *item {
            export::Item::Rig(r) => scene.rig_folder(r) != folder,
            export::Item::Folder(f) => scene.folders[f].parent != folder && !folder.is_some_and(|i| scene.folder_is_within(i, f)),
        };
        let items: Vec<export::Item> = items.iter().copied().filter(moves).collect();
        if items.is_empty() {
            return;
        }
        self.edit_rig("Move to folder", |scene, _| {
            for item in items {
                match item {
                    export::Item::Rig(r) => scene.rigs[r].folder = folder,
                    export::Item::Folder(f) => {
                        scene.move_folder(f, folder);
                    }
                }
            }
        });
        if let Some(folder) = folder {
            self.collapsed.remove(&RowKey::Folder(folder));
        }
    }

    /// Adds a camera seeing what the viewport sees.
    pub fn add_camera(&mut self) {
        self.add_camera_in(self.selected_folder());
    }

    /// Adds a camera seeing what the viewport sees, listed in `folder`.
    pub fn add_camera_in(&mut self, folder: Option<FolderId>) {
        let at = self.camera.free_frame();
        let mut camera = 0;
        self.edit_rig("Add camera", |scene, _| {
            camera = scene.add_camera(at, DEFAULT_FOV);
            let rig = scene.bones[camera].rig;
            scene.rigs[rig].folder = folder;
        });
        if let Some(folder) = folder {
            self.collapsed.remove(&RowKey::Folder(folder));
        }
        self.camera_view.camera = Some(camera);
        self.camera_view.preview = true;
        self.select(Selection::Bone(camera));
        self.scroll_to_selection = true;
    }

    /// Spawns a shape on the floor beside what's there, seen from where the view is,
    /// so it doesn't land inside a rig.
    pub fn add_shape(&mut self, shape: PartShape) {
        self.add_shape_in(shape, self.selected_folder());
    }

    /// Spawns a shape as `add_shape` does, listed in `folder`.
    pub fn add_shape_in(&mut self, shape: PartShape, folder: Option<FolderId>) {
        let size = glam::Vec3::splat(SHAPE_SIZE);
        let at = self.spawn_spot(size);
        self.spawn(folder, |scene| scene.add_shape(shape, at, size));
    }

    /// Where something `size` big lands: on the floor beside what's there, seen from
    /// where the view is, so it doesn't land inside a rig.
    fn spawn_spot(&self, size: glam::Vec3) -> Xf {
        let center = self.camera.target;
        let side = (self.camera.right() * glam::Vec3::new(1.0, 0.0, 1.0)).try_normalize().unwrap_or(glam::Vec3::X);
        let reach = size.x.max(size.z) * 0.5 + SHAPE_SIZE * 0.5;
        let past = self.scene.bounds(None).map_or(0.0, |(min, max)| {
            let corners = [min, glam::Vec3::new(max.x, 0.0, min.z), glam::Vec3::new(min.x, 0.0, max.z), max];
            corners.into_iter().map(|c| (c - center).dot(side)).fold(0.0, f32::max) + reach
        });
        let spot = center + side * past;
        Xf::new(glam::Quat::IDENTITY, glam::Vec3::new(spot.x, self.view.show_grid_floor_y + size.y * 0.5, spot.z))
    }

    /// Adds something with one undo step, lists it in `folder` and selects it.
    fn spawn(&mut self, folder: Option<FolderId>, add: impl FnOnce(&mut Scene) -> BoneId) {
        let mut bone = 0;
        self.edit_rig("Add shape", |scene, _| {
            bone = add(scene);
            let rig = scene.bones[bone].rig;
            scene.rigs[rig].folder = folder;
        });
        if let Some(folder) = folder {
            self.collapsed.remove(&RowKey::Folder(folder));
        }
        self.select(Selection::Bone(bone));
        self.scroll_to_selection = true;
    }

    /// Adds Studio's Baseplate, as a shape.
    pub fn add_baseplate(&mut self) {
        self.add_baseplate_in(self.selected_folder());
    }

    /// The size, color and grid of Studio's Baseplate template, with its top as the floor,
    /// under everything, as in a new Studio place. It starts locked, so clicks pass
    /// through it to clear the selection.
    pub fn add_baseplate_in(&mut self, folder: Option<FolderId>) {
        use crate::scene::{Decal, Face, Tiling};
        let size = glam::Vec3::new(2048.0, 16.0, 2048.0);
        let at = Xf::new(glam::Quat::IDENTITY, glam::Vec3::new(0.0, self.view.show_grid_floor_y - size.y * 0.5, 0.0));
        let grid = Decal {
            texture: crate::materials::BASEPLATE_TEXTURE.to_string(),
            face: Face::Top,
            color: [0.0; 3],
            transparency: 0.8,
            tiling: Some(Tiling { studs_u: 8.0, studs_v: 8.0, offset_u: 0.0, offset_v: 0.0 }),
        };
        self.spawn(folder, |scene| {
            let bone = scene.add_shape(PartShape::Block, at, size);
            let rig = scene.bones[bone].rig;
            let part = scene.bones[bone].part.expect("a shape has a part");
            scene.rigs[rig].name = "Baseplate".to_string();
            scene.bones[bone].name = "Baseplate".to_string();
            let p = &mut scene.parts[part];
            (p.name, p.color, p.locked) = ("Baseplate".to_string(), [91.0 / 255.0; 3], true);
            p.decals = vec![grid];
            bone
        });
    }

    /// Deletes a bone or anything else listed in the Workspace, along with its keys.
    pub fn delete_object(&mut self, bone: BoneId) {
        self.edit_rig("Delete", |scene, animation| {
            for gone in modifiers::delete_bones(scene, &[bone]) {
                animation.forget_bone(gone);
            }
        });
    }

    /// Deletes what's selected in the Workspace or the viewport.
    pub fn delete_selection(&mut self) {
        let (selection, bones) = (self.selection, self.selected_bones());
        if selection == Selection::None {
            return;
        }
        let rigs: Vec<RigId> = std::iter::once(selection).filter_map(|s| if let Selection::Rig(r) = s { Some(r) } else { None }).chain(self.extra_rigs.iter().copied()).collect();
        let folders: Vec<FolderId> = std::iter::once(selection).filter_map(|s| if let Selection::Folder(f) = s { Some(f) } else { None }).chain(self.extra_folders.iter().copied()).collect();
        if let Selection::Onion(rig) = selection {
            self.edit_rig("Delete onion skin", |scene, _| scene.rigs[rig].onion = None);
            return;
        }
        self.pause();
        self.edit_rig("Delete", |scene, animation| {
            let mut gone = if bones.is_empty() { Vec::new() } else { modifiers::delete_bones(scene, &bones) };
            if let Selection::Part(part) = selection {
                gone.extend(modifiers::delete_part(scene, part));
            }
            for &rig in &rigs {
                gone.extend(modifiers::delete_rig(scene, rig));
            }
            for &folder in &folders {
                gone.extend(modifiers::delete_folder(scene, folder));
            }
            for bone in gone {
                animation.forget_bone(bone);
            }
        });
    }

    /// The camera being looked through, falling back to any other if it was deleted.
    pub fn active_camera(&self) -> Option<BoneId> {
        self.camera_view
            .camera
            .filter(|c| !self.scene.bones[*c].removed)
            .or_else(|| self.scene.cameras().next())
    }

    /// Moves a camera to what the viewport is showing.
    pub fn camera_to_view(&mut self, camera: BoneId) {
        // The pose is relative to the camera's root, or to the bone it follows.
        let view = self.scene.bones[camera].base.inverse() * self.camera.free_frame();
        self.apply_poses("Align camera", vec![(camera, view)]);
    }

    pub fn toggle_camera_view(&mut self) {
        self.camera_view.through = !self.camera_view.through && self.active_camera().is_some();
    }

    pub fn toggle_camera_preview(&mut self) {
        self.camera_view.preview = !self.camera_view.preview && self.active_camera().is_some();
    }

    /// Keeps the viewport looking through the camera as it animates.
    fn sync_camera_view(&mut self, rect: Rect) {
        self.camera.through = self.active_camera().filter(|_| self.camera_view.through).map(|camera| {
            let bone = &self.scene.bones[camera];
            // A tall viewport shows the frame shorter than itself, so the view has to
            // open up for the frame to keep the camera's true field of view.
            let frame_height = (rect.width() / self.camera_view.shape.ratio()).min(rect.height()).max(1.0);
            let tan = (bone.value.clamp(1.0, 120.0).to_radians() * 0.5).tan() * rect.height() / frame_height;
            ThroughView { frame: bone.frame, fov: tan.atan() * 2.0 }
        });
    }


    pub fn open_import_dialog(&mut self, ctx: &egui::Context) {
        let tx = self.import_tx.clone();
        let repaint = ctx.clone();
        let loads = self.loads.clone();
        self.importing += 1;
        std::thread::spawn(move || {
            let picked = rfd::FileDialog::new()
                .set_title("Import a Roblox rig")
                .add_filter("Roblox model", &["rbxm", "rbxmx", "rblxm"])
                .add_filter("All files", &["*"])
                .pick_files()
                .unwrap_or_default();
            let cancelled = picked.is_empty();
            let _load = (!cancelled).then(|| LoadGuard::new(&loads));
            repaint.request_repaint();
            for path in picked {
                let result = import::import_file(&path);
                let _ = tx.send((path, result));
            }
            // An empty path tells the app this dialog is finished.
            let _ = tx.send((PathBuf::new(), Err(if cancelled { String::new() } else { "\0".into() })));
            repaint.request_repaint();
        });
    }

    pub fn import_paths(&mut self, ctx: &egui::Context, mut paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        // A project replaces the scene, so anything imported alongside it would be lost.
        if let Some(at) = paths.iter().position(|p| project::is_project(p)) {
            let path = paths.remove(at);
            if !paths.is_empty() {
                self.toast(ToastKind::Warning, "A project opens on its own, so the other files weren't imported.");
            }
            self.request_leave(ctx, Leave::Open(Some(path)));
            return;
        }
        let tx = self.import_tx.clone();
        let repaint = ctx.clone();
        let load = LoadGuard::new(&self.loads);
        self.importing += 1;
        std::thread::spawn(move || {
            let _load = load;
            for path in paths {
                let result = import::import_file(&path);
                let _ = tx.send((path, result));
            }
            let _ = tx.send((PathBuf::new(), Err("\0".into())));
            repaint.request_repaint();
        });
    }

    fn poll_renderer(&mut self) {
        if self.renderer.is_some() {
            return;
        }
        match self.renderer_rx.try_recv() {
            Ok(renderer) => self.renderer = Some(renderer),
            Err(TryRecvError::Empty) => {}
            // Going down here leaves the startup marker, so the next launch uses Auto.
            Err(TryRecvError::Disconnected) => panic!("the viewport renderer couldn't start"),
        }
    }

    fn poll_imports(&mut self) {
        while let Ok((path, result)) = self.import_rx.try_recv() {
            if path.as_os_str().is_empty() {
                self.importing = self.importing.saturating_sub(1);
                continue;
            }
            match result {
                Ok(imported) => {
                    self.finish_import(imported);
                    self.remember_file(&path, false);
                }
                Err(message) => {
                    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    self.toast(ToastKind::Error, format!("{name}: {message}"));
                }
            }
        }
    }

    /// Adds a file to the home page's list, and asks for a picture of it if `new_picture`
    /// says the old one is out of date, or it hasn't got one.
    fn remember_file(&mut self, path: &Path, new_picture: bool) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(list) = crate::settings::recent_list() {
            library::remember(&list, &path, library::now());
        }
        let no_picture = crate::settings::thumbnails_folder().is_some_and(|folder| !library::thumbnail_path(&folder, &path).exists());
        if new_picture || no_picture {
            self.thumbnail_due = Some((path, THUMBNAIL_WAIT));
        }
    }

    /// Takes a file's picture from the 3D view, once nothing's in front of it.
    fn take_thumbnail(&mut self, ctx: &egui::Context) {
        let blocked = self.home.open || self.loader.is_covering() || self.connect.is_open() || self.update.is_showing() || self.prefs_open || self.about_open || self.leaving.is_some();
        let Some((_, frames)) = self.thumbnail_due.as_mut().filter(|_| !blocked) else {
            return;
        };
        ctx.request_repaint();
        if *frames > 0 {
            *frames -= 1;
            return;
        }
        let Some((file, _)) = self.thumbnail_due.take() else {
            return;
        };
        let Some(folder) = crate::settings::thumbnails_folder() else {
            return;
        };
        let target = library::thumbnail_path(&folder, &file);
        let (view, scale) = (self.viewport_rect, ctx.pixels_per_point());
        ctx.request_screenshot(move |shot| {
            std::thread::spawn(move || {
                let bytes: Vec<u8> = shot.pixels.iter().flat_map(|c| c.to_array()).collect();
                let Some(whole) = image::RgbaImage::from_raw(shot.size[0] as u32, shot.size[1] as u32, bytes) else {
                    return;
                };
                let pixels = |points: f32| (points * scale).round() as u32;
                let view = image::imageops::crop_imm(&whole, pixels(view.left()), pixels(view.top()), pixels(view.width()), pixels(view.height())).to_image();
                // A file with no picture only shows its icon, which is fine.
                let _ = library::write_thumbnail(&view, &target);
            });
        });
    }

    /// Writes a copy of the work every so often, if it's changed since the last one. A
    /// crash can't take more than this long's worth, even with nothing ever saved.
    fn keep_backup(&mut self) {
        let settled = self.session.is_none() && self.live_edit.is_none() && self.panel_edit.is_none();
        if !settled || self.last_backup.elapsed() < backup_interval() || self.scene.is_empty() {
            return;
        }
        self.last_backup = Instant::now();
        let work = project::fingerprint(&self.scene, &self.animation);
        let Some(folder) = crate::settings::backups_folder().filter(|_| work != self.backed_up) else {
            return;
        };
        self.backed_up = work;
        let name = self.work_name();
        let (scene, animation, view) = (self.scene.clone(), self.animation.clone(), self.project_view());
        std::thread::spawn(move || {
            let _ = std::fs::create_dir_all(&folder);
            let path = library::backup_path(&folder, &name, library::now());
            if project::save(&path, &scene, &animation, &view).is_ok() {
                library::prune_backups(&folder);
            }
        });
    }

    /// A window that opens bigger than the screen leaves things out of reach, like the buttons
    /// on the Connect to Roblox popup. The size of the screen isn't known until it's open.
    fn fit_to_screen(&mut self, ctx: &egui::Context) {
        if self.fitted_to_screen {
            return;
        }
        let Some(screen) = ctx.input(|i| i.viewport().monitor_size) else {
            return;
        };
        self.fitted_to_screen = true;
        if screen.x < WINDOW_SIZE[0] || screen.y < WINDOW_SIZE[1] {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
    }

    /// Back to the home page. The work stays as it is, ready to go back to.
    pub fn go_home(&mut self) {
        self.pause();
        self.settle_edits();
        self.home.show();
    }

    fn finish_import(&mut self, imported: Imported) {
        // A rig that's only been opened isn't work yet, since it can just be imported
        // again. Once there's a project file, that file is missing it.
        let nothing_to_lose = self.project_path.is_none() && !self.has_unsaved_work();
        let (first_new_rig, focus) = (self.scene.rigs.len(), self.focused_rig());
        let has_animations = !imported.sequences.is_empty();
        self.scene.append(imported.scene);
        self.scene.evaluate();
        let mut warnings = imported.warnings;
        warnings.extend(sequences::load(&mut self.animation, &mut self.scene, imported.sequences, first_new_rig, focus));
        self.sync_animations();
        if has_animations {
            // Undo steps hold whole animations, and the ones so far don't have these.
            self.history.clear();
            self.animation.apply(&mut self.scene, self.frame);
        }
        for warning in warnings {
            self.toast(ToastKind::Warning, warning);
        }
        if let Some((min, max)) = self.scene.bounds(None) {
            self.view.show_grid_floor_y = min.y;
            self.camera.frame_bounds(min, max);
        }
        if nothing_to_lose {
            self.saved_work = project::fingerprint(&self.scene, &self.animation);
            self.backed_up = self.saved_work;
        }
        // A file of nothing but animations leaves nothing to show if no rig is open.
        self.home.open &= self.scene.is_empty();
    }

    pub fn is_saving(&self) -> bool {
        self.saving > 0
    }

    pub fn project_name(&self) -> Option<String> {
        let path = self.project_path.as_ref()?;
        Some(path.file_stem()?.to_string_lossy().into_owned())
    }

    /// What the work goes by, with or without a file of its own.
    fn work_name(&self) -> String {
        self.project_name().or_else(|| self.scene.rigs.first().map(|r| r.name.clone())).unwrap_or_else(|| "Untitled".to_string())
    }

    fn show_project_name(&self, ctx: &egui::Context) {
        let title = self.project_name().map_or_else(|| "Kaye".to_string(), |name| format!("{name} - Kaye"));
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }

    /// Finishes anything still being changed, so it counts as part of the work.
    fn settle_edits(&mut self) {
        self.cancel_session();
        self.commit_panel_edit();
        self.commit_live_edit();
    }

    pub fn has_unsaved_work(&self) -> bool {
        let empty = self.scene.parts.is_empty() && self.scene.bones.is_empty() && self.animation.tracks.is_empty();
        !empty && project::fingerprint(&self.scene, &self.animation) != self.saved_work
    }

    /// Closing, a new scene or opening a project all lose the current work, so they
    /// come through here to ask first.
    pub fn request_leave(&mut self, ctx: &egui::Context, leave: Leave) {
        if self.leaving.is_some() {
            return;
        }
        // The answer depends on how that save turns out.
        if self.is_saving() {
            self.after_save = Some(leave);
            return;
        }
        self.settle_edits();
        if self.has_unsaved_work() {
            self.leaving = Some(leave);
        } else {
            self.leave_now(ctx, leave);
        }
    }

    pub fn leave_now(&mut self, ctx: &egui::Context, leave: Leave) {
        match leave {
            Leave::Close => self.close_without_asking(ctx),
            Leave::New => {
                self.new_scene();
                self.home.open = false;
                self.show_project_name(ctx);
            }
            Leave::Open(Some(path)) if project::is_project(&path) => self.open_project_file(ctx, path),
            // A rig picked from the home page starts fresh, as a project does.
            Leave::Open(Some(path)) => {
                self.new_scene();
                self.show_project_name(ctx);
                self.import_paths(ctx, vec![path]);
            }
            Leave::Open(None) => self.open_project_dialog(ctx),
            Leave::Update => match crate::update::start_updater() {
                Ok(()) => self.close_without_asking(ctx),
                Err(error) => self.toast(ToastKind::Error, format!("The updater didn't start: {error}")),
            },
        }
    }

    /// The updater started this Kaye, so what it left behind can go.
    pub fn finish_update(&mut self) {
        crate::update::clean_up_after_update();
    }

    pub fn close_without_asking(&mut self, ctx: &egui::Context) {
        self.closing = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// The window's X button and File > Quit both end up here.
    fn handle_close_request(&mut self, ctx: &egui::Context) {
        #[cfg(feature = "devtools")]
        if self.script.as_ref().is_some_and(|s| s.quitting) {
            return;
        }
        if !self.closing && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_leave(ctx, Leave::Close);
        }
    }

    pub fn save_then(&mut self, ctx: &egui::Context, leave: Leave) {
        self.after_save = Some(leave);
        self.save_project(ctx);
    }

    pub fn save_project(&mut self, ctx: &egui::Context) {
        self.start_save(ctx, self.project_path.clone());
    }

    pub fn save_project_as(&mut self, ctx: &egui::Context) {
        self.start_save(ctx, None);
    }

    /// Saves a copy of the work on another thread, so edits made meanwhile simply
    /// count as unsaved. With no path, it asks for one.
    fn start_save(&mut self, ctx: &egui::Context, path: Option<PathBuf>) {
        self.settle_edits();
        let (scene, animation, view) = (self.scene.clone(), self.animation.clone(), self.project_view());
        let name = self.work_name();
        let (tx, repaint) = (self.save_tx.clone(), ctx.clone());
        self.saving += 1;
        std::thread::spawn(move || {
            let answer = path.or_else(|| pick_save_path(&name)).map(project::with_extension).map(|path| {
                let saved = project::save(&path, &scene, &animation, &view).map(|()| project::fingerprint(&scene, &animation));
                (path, saved)
            });
            let _ = tx.send(answer);
            repaint.request_repaint();
        });
    }

    /// Every rig has its animations, and the range showing belongs to the rig in focus.
    /// A camera or a spawned shape doesn't take over the range, since it's usually there
    /// to go along with a character's animation.
    fn sync_animations(&mut self) {
        self.animation.ensure_clips(&self.scene);
        let shows = self.focused_rig().filter(|rig| !self.scene.rigs[*rig].kind.is_single());
        if let Some(clip) = shows.and_then(|rig| self.animation.active_clip(rig)) {
            self.animation.show_clip(clip);
        }
    }

    /// Runs Ragdoll and motion paths again whenever what they depend on has changed.
    /// It happens on another thread, since a long animation takes a moment to play through.
    fn sync_replay(&mut self, ctx: &egui::Context) {
        while let Ok((signature, replay)) = self.replay_rx.try_recv() {
            self.replaying = false;
            self.replay_signature = Some(signature);
            self.scene.dynamics.set_baked(replay.baked);
            self.motion_path_points = replay.paths;
            self.scene.evaluate();
        }
        self.scene.dynamics.floor = self.view.show_grid_floor_y;
        let gone = |bone: &BoneId| self.scene.bones.get(*bone).is_none_or(|b| b.removed);
        if self.motion_paths.iter().any(gone) {
            let scene = &self.scene;
            self.motion_paths.retain(|bone| scene.bones.get(*bone).is_some_and(|b| !b.removed));
        }
        if self.motion_paths.is_empty() && !dynamics::has_dynamics(&self.scene) {
            if self.scene.dynamics.has_baked() {
                self.scene.dynamics.set_baked(None);
                self.scene.evaluate();
            }
            self.motion_path_points = MotionPaths::default();
            self.replay_signature = None;
            return;
        }
        if self.replaying {
            return;
        }
        let signature = dynamics::signature(&self.scene, &self.animation, &self.motion_paths);
        if self.replay_signature == Some(signature) {
            return;
        }
        self.replaying = true;
        let (scene, animation, bones) = (self.scene.clone(), self.animation.clone(), self.motion_paths.clone());
        let (tx, repaint) = (self.replay_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send((signature, dynamics::replay(scene, animation, bones)));
            repaint.request_repaint();
        });
    }

    fn poll_saves(&mut self, ctx: &egui::Context) {
        while let Ok(answer) = self.save_rx.try_recv() {
            self.saving = self.saving.saturating_sub(1);
            match answer {
                Some((path, Ok(work))) => {
                    self.saved_work = work;
                    self.remember_file(&path, true);
                    self.project_path = Some(path);
                    self.show_project_name(ctx);
                    if let Some(leave) = self.after_save.take().filter(|_| !self.is_saving()) {
                        self.request_leave(ctx, leave);
                    }
                }
                Some((path, Err(message))) => {
                    self.after_save = None;
                    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    self.toast(ToastKind::Error, format!("{name} couldn't be saved: {message}"));
                }
                None => self.after_save = None,
            }
        }
    }

    fn project_view(&self) -> project::View {
        project::View {
            frame: self.frame,
            target: self.camera.target,
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
            dist: self.camera.dist,
            ortho: self.camera.ortho,
            floor_y: self.view.show_grid_floor_y,
            camera: self.camera_view.camera,
            preview: self.camera_view.preview,
            through: self.camera_view.through,
            motion_paths: self.motion_paths.clone(),
        }
    }

    fn open_project_dialog(&mut self, ctx: &egui::Context) {
        let (tx, repaint, loads) = (self.open_tx.clone(), ctx.clone(), self.loads.clone());
        std::thread::spawn(move || {
            let picked = rfd::FileDialog::new().set_title("Open a Kaye project").add_filter("Kaye project", &[project::EXTENSION]).pick_file();
            if let Some(path) = picked {
                let _load = LoadGuard::new(&loads);
                repaint.request_repaint();
                let result = project::load(&path);
                let _ = tx.send((path, result));
                repaint.request_repaint();
            }
        });
    }

    fn open_project_file(&mut self, ctx: &egui::Context, path: PathBuf) {
        let (tx, repaint) = (self.open_tx.clone(), ctx.clone());
        let load = LoadGuard::new(&self.loads);
        std::thread::spawn(move || {
            let _load = load;
            let result = project::load(&path);
            let _ = tx.send((path, result));
            repaint.request_repaint();
        });
    }

    fn poll_opens(&mut self, ctx: &egui::Context) {
        while let Ok((path, result)) = self.open_rx.try_recv() {
            match result {
                Ok(project) => self.finish_open(ctx, path, project),
                Err(message) => {
                    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    self.toast(ToastKind::Error, format!("{name}: {message}"));
                }
            }
        }
    }

    fn finish_open(&mut self, ctx: &egui::Context, path: PathBuf, project: Project) {
        self.new_scene();
        *self.rest_pose.borrow_mut() = None;
        let Project { scene, animation, view } = project;
        self.scene = scene;
        // Smoothing changes how the keys play, so it comes from the project and
        // Preferences follows. What new keys start with stays a preference.
        self.settings.animation.smoothing = animation.smoothing;
        self.animation = Animation { default_easing: self.animation.default_easing, ..animation };
        self.frame = view.frame;
        self.camera.target = view.target;
        self.camera.yaw = view.yaw;
        self.camera.pitch = view.pitch;
        self.camera.dist = view.dist;
        self.camera.ortho = view.ortho;
        self.view.show_grid_floor_y = view.floor_y;
        self.camera_view.camera = view.camera;
        self.camera_view.preview = view.preview;
        self.camera_view.through = view.through;
        self.motion_paths = view.motion_paths;
        self.sync_animations();
        self.animation.apply(&mut self.scene, self.frame);
        self.saved_work = project::fingerprint(&self.scene, &self.animation);
        self.backed_up = self.saved_work;
        // A backup opens as unsaved work. Saving onto it would put the work somewhere
        // that gets cleaned out, so saving asks where it goes.
        if crate::settings::backups_folder().is_some_and(|folder| library::is_backup(&folder, &path)) {
            self.saved_work = 0;
        } else {
            self.remember_file(&path, false);
            self.project_path = Some(path);
        }
        self.show_project_name(ctx);
        self.home.open = false;
    }

    pub fn new_scene(&mut self) {
        self.counts.set(None);
        self.renaming = None;
        self.asset_parts = None;
        *self.rest_pose.borrow_mut() = None;
        self.project_path = None;
        self.saved_work = 0;
        self.backed_up = 0;
        self.thumbnail_due = None;
        self.session = None;
        self.panel_edit = None;
        self.live_edit = None;
        self.camera_view = CameraView::default();
        self.camera.through = None;
        self.scene = Scene::default();
        self.selection = Selection::None;
        self.clear_extras();
        self.box_select = None;
        self.hover = Selection::None;
        self.history.clear();
        self.collapsed.clear();
        self.animation = new_animation(&self.settings);
        self.frame = 0.0;
        self.playing = false;
        self.timeline = timeline::TimelineState::default();
        self.camera = OrbitCamera::default();
        self.view.show_grid_floor_y = 0.0;
        self.motion_paths.clear();
        self.motion_path_points = MotionPaths::default();
        self.replay_signature = None;
        self.sync_settings();
    }

    pub fn frame_all(&mut self) {
        if let Some((min, max)) = self.scene.bounds(None) {
            self.camera.frame_bounds(min, max);
        }
    }

    /// What everything in a folder covers together.
    fn folder_bounds(&self, folder: FolderId) -> Option<(glam::Vec3, glam::Vec3)> {
        self.scene.rigs_in_folder(folder).into_iter().filter_map(|r| self.scene.bounds(Some(r))).reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
    }

    pub fn frame_selection(&mut self) {
        let bounds = match self.selection {
            Selection::None => self.scene.bounds(None),
            // A camera's root, with the camera it holds.
            Selection::Rig(r) if self.scene.rigs[r].kind == RigKind::Camera => {
                let root = self.scene.rigs[r].placement.pos;
                let lens = self.scene.rigs[r].bones.first().map_or(root, |c| self.scene.bones[*c].frame.pos);
                let pad = glam::Vec3::splat(1.0);
                Some((root.min(lens) - pad, root.max(lens) + pad))
            }
            Selection::Rig(r) | Selection::Onion(r) => self.scene.bounds(Some(r)),
            Selection::Folder(f) => self.folder_bounds(f),
            Selection::Bone(_) => {
                let pad = glam::Vec3::splat(0.6);
                self.selected_bones().iter().fold(None, |bounds: Option<(glam::Vec3, glam::Vec3)>, b| {
                    let bone = &self.scene.bones[*b];
                    let low = bone.head().min(bone.tail()) - pad;
                    let high = bone.head().max(bone.tail()) + pad;
                    Some(bounds.map_or((low, high), |(a, c)| (a.min(low), c.max(high))))
                })
            }
            Selection::Part(p) => {
                let part = &self.scene.parts[p];
                let half = glam::Vec3::splat(part.size.length() * 0.5);
                Some((part.world.pos - half, part.world.pos + half))
            }
        };
        if let Some((min, max)) = bounds {
            self.camera.frame_bounds(min, max);
        }
    }

    pub fn commit_session(&mut self) {
        if let Some(session) = self.session.take() {
            let bone = match session.moving {
                Moving::Bone(bone) => bone,
                Moving::Rig(rig) => {
                    // Only an undo step, with nothing for Auto Key to see.
                    let moved = self.scene.rigs[rig].placement;
                    self.scene.rigs[rig].placement = session.start_pose;
                    self.edit_rig("Move rig", |scene, _| scene.set_rig_placement(rig, moved));
                    return;
                }
            };
            let after = self.scene.bones[bone].pose;
            if !after.approx_eq(session.start_pose) {
                let label = match session.op {
                    OpKind::Move => "Move bone",
                    OpKind::Rotate => "Rotate bone",
                };
                let mut changes = vec![PoseChange {
                    bone,
                    before: session.start_pose,
                    after,
                }];
                changes.extend(session.extras.iter().map(|(bone, before, _)| PoseChange {
                    bone: *bone,
                    before: *before,
                    after: self.scene.bones[*bone].pose,
                }));
                self.record_edit(label, changes);
            }
        }
    }

    pub fn cancel_session(&mut self) {
        if let Some(session) = self.session.take() {
            session.cancel(&mut self.scene);
        }
    }

    pub fn commit_panel_edit(&mut self) {
        if let Some((bone, before)) = self.panel_edit.take() {
            let after = self.scene.bones[bone].pose;
            if !after.approx_eq(before) {
                self.record_edit("Edit bone", vec![PoseChange { bone, before, after }]);
            }
        }
    }

    /// Writes new poses for a set of bones as one undo step.
    pub fn apply_poses(&mut self, label: &str, poses: Vec<(BoneId, Xf)>) {
        let changes: Vec<PoseChange> = poses
            .into_iter()
            .filter(|(bone, after)| !self.scene.bones[*bone].pose.approx_eq(*after))
            .map(|(bone, after)| PoseChange {
                bone,
                before: self.scene.bones[bone].pose,
                after,
            })
            .collect();
        for c in &changes {
            self.scene.bones[c.bone].pose = c.after;
        }
        self.scene.evaluate();
        self.record_edit(label, changes);
    }

    pub fn reset_selected(&mut self, position: bool, rotation: bool) {
        if let Some(rig) = self.movable_rig() {
            let now = self.scene.rigs[rig].placement;
            let placement = Xf::new(if rotation { glam::Quat::IDENTITY } else { now.rot }, if position { glam::Vec3::ZERO } else { now.pos });
            self.edit_rig("Move rig", |scene, _| scene.set_rig_placement(rig, placement));
            return;
        }
        let poses = self
            .controlled_bones()
            .into_iter()
            .map(|b| {
                let pose = self.scene.bones[b].pose;
                let after = Xf::new(
                    if rotation { glam::Quat::IDENTITY } else { pose.rot },
                    if position { glam::Vec3::ZERO } else { pose.pos },
                );
                (b, after)
            })
            .collect();
        self.apply_poses("Reset bone", poses);
    }

    pub fn reset_all_poses(&mut self) {
        let poses = (0..self.scene.bones.len()).filter(|b| self.scene.bones[*b].kind.is_joint()).map(|b| (b, Xf::IDENTITY)).collect();
        self.apply_poses("Reset pose", poses);
    }

    pub fn undo(&mut self) {
        self.cancel_session();
        self.commit_panel_edit();
        self.commit_live_edit();
        self.history.undo(&mut self.scene, &mut self.animation);
        self.after_history();
    }

    pub fn redo(&mut self) {
        self.cancel_session();
        self.commit_panel_edit();
        self.commit_live_edit();
        self.history.redo(&mut self.scene, &mut self.animation);
        self.after_history();
    }

    fn after_history(&mut self) {
        self.animation.apply(&mut self.scene, self.frame);
        self.prune_selected_keys();
        self.drop_removed_selection();
    }

    fn prune_selected_keys(&mut self) {
        let animation = &self.animation;
        self.timeline.selected.retain(|(b, f)| animation.key_at(*b, *f).is_some());
        graph::prune(self);
    }

    /// Stores a finished pose edit in the undo history, and on the timeline when
    /// Auto Key is on.
    fn record_edit(&mut self, label: &str, changes: Vec<PoseChange>) {
        if changes.is_empty() {
            return;
        }
        let keys = &self.settings.animation;
        if !keys.auto_key {
            self.history.push(label, changes);
            return;
        }
        let (key_start_pose, only_needed) = (keys.key_start_pose, keys.only_insert_needed);
        let before = self.animation.clone();
        let frame = self.whole_frame();
        for c in &changes {
            // Skip a key that wouldn't change the curve, as Blender's option does.
            let value = self.scene.bones[c.bone].value;
            let unneeded = only_needed && self.animation.sample(c.bone, frame as f32).is_some_and(|p| p.approx_eq(c.after));
            if !unneeded {
                self.animation.record_edit(c.bone, frame, c.before, c.after, value, key_start_pose);
            }
        }
        self.history.push_edit(label, changes, before, self.animation.clone());
    }

    /// Runs a change to the keys as one undo step, then updates the rig to match.
    pub fn edit_animation(&mut self, label: &str, edit: impl FnOnce(&mut Animation)) {
        let before = self.animation.clone();
        edit(&mut self.animation);
        if self.animation != before {
            self.history.push_edit(label, Vec::new(), before, self.animation.clone());
            self.animation.apply(&mut self.scene, self.frame);
        }
        self.prune_selected_keys();
    }

    pub fn whole_frame(&self) -> i32 {
        self.frame.round() as i32
    }

    pub fn set_frame(&mut self, frame: f32) {
        self.commit_panel_edit();
        self.commit_live_edit();
        self.frame = frame;
        self.animation.apply(&mut self.scene, frame);
    }

    /// Stops playback and settles on a whole frame, ready for editing.
    pub fn pause(&mut self) {
        if self.playing {
            self.playing = false;
            self.set_frame(self.frame.round());
        }
    }

    pub fn toggle_play(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.commit_panel_edit();
            self.cancel_session();
            if self.frame >= self.animation.end as f32 {
                self.set_frame(self.animation.start as f32);
            }
            self.playing = true;
        }
    }

    fn tick_playback(&mut self, ctx: &egui::Context) {
        if !self.playing {
            return;
        }
        let keys = &self.settings.animation;
        let seconds = if keys.play_every_frame {
            1.0 / self.animation.fps
        } else {
            ctx.input(|i| i.stable_dt).min(0.1) * keys.playback_speed
        };
        let (frame, playing) = self.animation.advance(self.frame, seconds);
        self.playing = playing;
        self.set_frame(if playing { frame } else { frame.round() });
        match self.settings.system.playback_fps_limit {
            0 => ctx.request_repaint(),
            limit => ctx.request_repaint_after(Duration::from_secs_f32(1.0 / limit as f32)),
        }
    }

    pub fn step_frame(&mut self, delta: i32) {
        self.pause();
        self.set_frame((self.whole_frame() + delta) as f32);
    }

    /// Jumps to the closest key in a direction, on the selection when there is one.
    pub fn jump_key(&mut self, forward: bool) {
        self.pause();
        let targets = self.key_targets();
        let bones: Vec<BoneId> = if targets.is_empty() { self.animation.tracks.keys().copied().collect() } else { targets };
        let frames = self.animation.key_frames(&bones);
        let now = self.whole_frame();
        let next = if forward {
            frames.range(now + 1..).next().copied()
        } else {
            frames.range(..now).next_back().copied()
        };
        if let Some(frame) = next {
            self.set_frame(frame as f32);
        }
    }

    /// The bones a keyframe command acts on.
    pub fn key_targets(&self) -> Vec<BoneId> {
        match self.selection {
            Selection::Bone(_) => self.controlled_bones(),
            Selection::Rig(r) => self.scene.live_bones(r),
            _ => Vec::new(),
        }
    }

    pub fn insert_key(&mut self) {
        self.pause();
        let frame = self.whole_frame();
        let poses: Vec<(BoneId, Xf, f32)> = self
            .key_targets()
            .into_iter()
            .map(|b| (b, self.scene.bones[b].pose, self.scene.bones[b].value))
            .collect();
        self.edit_animation("Insert keyframe", |a| {
            for (bone, pose, value) in poses {
                a.key_shown(bone, frame, pose, value);
            }
        });
    }

    /// The diamond beside an animated number: keys it here, or removes that key.
    pub fn toggle_key(&mut self, bone: BoneId) {
        self.pause();
        let frame = self.whole_frame();
        let (pose, value) = (self.scene.bones[bone].pose, self.scene.bones[bone].value);
        self.edit_animation("Toggle keyframe", |a| {
            if a.key_at(bone, frame).is_some() {
                a.remove_key(bone, frame);
            } else {
                a.key_shown(bone, frame, pose, value);
            }
        });
    }

    pub fn delete_keys_at_frame(&mut self) {
        self.pause();
        let frame = self.whole_frame();
        let bones = self.key_targets();
        self.edit_animation("Delete keyframe", |a| {
            for bone in bones {
                a.remove_key(bone, frame);
            }
        });
    }

    pub fn delete_selected_keys(&mut self) {
        let keys: Vec<KeyId> = self.timeline.selected.iter().copied().collect();
        self.edit_animation("Delete keys", |a| {
            for (bone, frame) in keys {
                a.remove_key(bone, frame);
            }
        });
    }

    /// Takes one channel out of the selected keys, leaving the rest of each key. A key with
    /// no channel left goes.
    pub fn delete_selected_channel(&mut self, channel: usize) {
        let keys: Vec<KeyId> = self.timeline.selected.iter().copied().collect();
        self.edit_animation("Delete channel keys", |a| {
            for (bone, frame) in keys {
                let Some(track) = a.tracks.get_mut(&bone) else {
                    continue;
                };
                let Some(index) = track.index(frame).filter(|i| track.keys[*i].sets(channel)) else {
                    continue;
                };
                track.keys[index].channels &= !(1 << channel);
                if track.keys[index].channels == 0 {
                    a.remove_key(bone, frame);
                }
            }
        });
    }

    /// Copies the selected keys so the earliest lands on the current frame, and
    /// selects the copies.
    pub fn duplicate_selected_keys(&mut self) {
        let Some(earliest) = self.timeline.selected.iter().map(|k| k.1).min() else {
            return;
        };
        self.pause();
        let delta = self.whole_frame() - earliest;
        let copies: Vec<(BoneId, crate::animation::Key)> = self
            .timeline
            .selected
            .iter()
            .filter_map(|(b, f)| self.animation.key_at(*b, *f).map(|k| (*b, crate::animation::Key { frame: f + delta, ..*k })))
            .collect();
        self.timeline.selected = copies.iter().map(|(b, k)| (*b, k.frame)).collect();
        self.edit_animation("Duplicate keys", |a| {
            for (bone, key) in copies {
                a.tracks.entry(bone).or_default().insert(key);
            }
        });
    }

    /// The keys an easing change applies to: the selected keys, otherwise the key
    /// on the selected bone that the current motion is leaving.
    pub fn easing_targets(&self) -> Vec<KeyId> {
        if !self.timeline.selected.is_empty() {
            let mut keys: Vec<KeyId> = self.timeline.selected.iter().copied().collect();
            keys.sort_unstable();
            return keys;
        }
        self.selected_bone()
            .and_then(|b| {
                let track = self.animation.tracks.get(&b)?;
                let key = track.key_before(self.frame).or(track.keys.first())?;
                Some(vec![(b, key.frame)])
            })
            .unwrap_or_default()
    }

    pub fn modify_easing(&mut self, keys: &[KeyId], change: impl Fn(&mut Easing)) {
        self.edit_animation("Change easing", |a| {
            for (bone, frame) in keys {
                if let Some(key) = a.tracks.get_mut(bone).and_then(|t| t.keys.iter_mut().find(|k| k.frame == *frame)) {
                    change(&mut key.easing);
                }
            }
        });
    }

    /// What a Handle Type change applies to: the channels picked on the Graph tab,
    /// otherwise every channel of the selected keys.
    fn handle_targets(&self) -> Vec<(KeyId, usize)> {
        if self.bottom_tab == BottomTab::Graph {
            let picked = graph::picked_channels(self);
            if !picked.is_empty() {
                return picked;
            }
        }
        self.easing_targets().into_iter().flat_map(|key| (0..CHANNELS).map(move |c| (key, c))).collect()
    }

    pub fn set_handle_kind(&mut self, kind: HandleKind) {
        let targets = self.handle_targets();
        let smoothing = self.animation.smoothing;
        let wraps: Vec<_> = targets.iter().map(|((bone, _), _)| self.animation.wrap_of(self.scene.bones[*bone].rig)).collect();
        self.edit_animation("Change handles", |a| {
            for (((bone, frame), channel), wrap) in targets.into_iter().zip(wraps) {
                let Some(track) = a.tracks.get_mut(&bone) else {
                    continue;
                };
                let Some(index) = track.index(frame) else {
                    continue;
                };
                track.keys[index].easing.style = EaseStyle::Bezier;
                match kind {
                    HandleKind::Auto(handle) => {
                        track.keys[index].easing.handle_type = handle;
                        track.keys[index].handles[channel] = None;
                    }
                    HandleKind::Aligned => track.pin_handles(index, channel, true, smoothing, wrap),
                    HandleKind::Free => track.pin_handles(index, channel, false, smoothing, wrap),
                }
            }
        });
    }

    /// A shortcut as shown in menus.
    pub fn key_text(&self, action: Action) -> String {
        self.settings.keys.get(action).to_string()
    }

    /// An action's name and shortcut, for tooltips.
    pub fn tip(&self, action: Action) -> String {
        format!("{}  {}", action.label(), self.key_text(action))
    }

    pub fn toggle_shading(&mut self) {
        let shading = &mut self.settings.viewport.shading;
        *shading = match shading {
            Shading::Studio => Shading::Shaded,
            Shading::Shaded => Shading::Studio,
        };
    }

    /// Where the camera orbits around when that's set to follow the selection.
    pub fn selection_center(&self) -> Option<glam::Vec3> {
        match self.selection {
            Selection::Bone(b) => Some(self.scene.bones[b].head()),
            Selection::Part(p) => Some(self.scene.parts[p].world.pos),
            Selection::Rig(r) | Selection::Onion(r) => self.scene.bounds(Some(r)).map(|(min, max)| (min + max) * 0.5),
            Selection::Folder(f) => self.folder_bounds(f).map(|(min, max)| (min + max) * 0.5),
            Selection::None => None,
        }
    }

    pub fn has_background(&self) -> bool {
        self.background.is_some()
    }

    /// Decodes on another thread, since a large image takes a moment.
    fn load_background(&self, ctx: &egui::Context, path: String) {
        let tx = self.background_tx.clone();
        let repaint = ctx.clone();
        let load = LoadGuard::new(&self.loads);
        std::thread::spawn(move || {
            let _load = load;
            let result = decode_image(&path);
            let _ = tx.send((path, result));
            repaint.request_repaint();
        });
    }

    pub fn choose_background(&self, ctx: &egui::Context) {
        let tx = self.background_tx.clone();
        let repaint = ctx.clone();
        let loads = self.loads.clone();
        std::thread::spawn(move || {
            let picked = rfd::FileDialog::new()
                .set_title("Choose a background image")
                .add_filter("Image", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
                .pick_file();
            if let Some(path) = picked {
                let _load = LoadGuard::new(&loads);
                repaint.request_repaint();
                let path = path.to_string_lossy().into_owned();
                let result = decode_image(&path);
                let _ = tx.send((path, result));
                repaint.request_repaint();
            }
        });
    }

    pub fn remove_background(&mut self) {
        self.background = None;
        self.settings.theme.background_image = None;
        self.settings.theme.panel_opacity = 1.0;
    }

    fn poll_background(&mut self, ctx: &egui::Context) {
        while let Ok((path, result)) = self.background_rx.try_recv() {
            match result {
                Ok(image) => {
                    self.background = Some(ctx.load_texture("ui background", image, egui::TextureOptions::LINEAR));
                    // An opaque UI would hide the image they just picked.
                    if self.settings.theme.background_image.is_none() && self.settings.theme.panel_opacity > 0.9 {
                        self.settings.theme.panel_opacity = 0.82;
                    }
                    self.settings.theme.background_image = Some(path);
                }
                Err(message) => self.toast(ToastKind::Error, format!("{path}: {message}")),
            }
        }
    }

    /// Paints behind every panel.
    fn paint_background(&self, ui: &egui::Ui) {
        let rect = ui.ctx().content_rect();
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, theme::window());
        let Some(texture) = &self.background else {
            return;
        };
        let theme_settings = &self.settings.theme;
        let size = texture.size_vec2();
        let uv = match theme_settings.image_fit {
            ImageFit::Stretch => Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            // Crop whichever way the image overflows the window, keeping it centered.
            ImageFit::Cover => {
                let window_aspect = rect.width() / rect.height().max(1.0);
                let image_aspect = size.x / size.y.max(1.0);
                let (w, h) = if image_aspect > window_aspect {
                    (window_aspect / image_aspect, 1.0)
                } else {
                    (1.0, image_aspect / window_aspect)
                };
                Rect::from_center_size(egui::pos2(0.5, 0.5), egui::vec2(w, h))
            }
        };
        let alpha = (theme_settings.image_opacity.clamp(0.0, 1.0) * 255.0) as u8;
        painter.image(texture.id(), rect, uv, egui::Color32::from_white_alpha(alpha));
    }

    /// Pushes Preferences out to everything that reads them.
    fn sync_settings(&mut self) {
        let s = &self.settings;
        self.camera.fov = s.viewport.field_of_view.to_radians();
        self.camera.smooth_views = s.navigation.smooth_views;
        self.camera.view_speed = s.navigation.view_speed;
        self.history.limit = s.system.undo_steps;
        self.animation.smoothing = s.animation.smoothing;
        self.animation.default_easing = Easing {
            style: s.animation.default_style,
            handle_type: s.animation.default_handle,
            ..Easing::default()
        };
    }

    fn apply_settings_changes(&mut self, ctx: &egui::Context) {
        if self.settings == self.applied {
            return;
        }
        if self.settings.theme != self.applied.theme || self.settings.interface.tooltips != self.applied.interface.tooltips {
            theme::apply_settings(ctx, &self.settings.theme, self.settings.interface.tooltips);
        }
        // Rescaling while dragging its own slider would move the slider under the mouse.
        let dragging = ctx.input(|i| i.pointer.any_down());
        if self.settings.interface.ui_scale != ctx.zoom_factor() && !dragging {
            ctx.set_zoom_factor(self.settings.interface.ui_scale);
        }
        let smoothing_changed = self.settings.animation.smoothing != self.animation.smoothing;
        self.sync_settings();
        if smoothing_changed {
            self.animation.apply(&mut self.scene, self.frame);
        }
        if !dragging {
            self.settings.save();
            self.applied = self.settings.clone();
        }
    }

    fn begin_modal(&mut self, ctx: &egui::Context, op: OpKind) {
        let mouse = ctx
            .input(|i| i.pointer.latest_pos())
            .unwrap_or(self.viewport_rect.center());
        if let Some(rig) = self.movable_rig() {
            self.start_rig_session(rig, op, None, true, mouse);
        } else if let Some(bone) = self.selected_bone().filter(|b| !modifiers::locked_by_ik(&self.scene, *b)) {
            self.start_session(bone, op, None, true, mouse);
        }
    }

    fn pressed(&self, ctx: &egui::Context, action: Action) -> bool {
        consume_combo(ctx, self.settings.keys.get(action), action.repeats())
    }

    /// Ctrl+C and Ctrl+V reach egui as clipboard events and not as keys, so those count too.
    fn clipboard_shortcut(&self, ctx: &egui::Context, action: Action) -> bool {
        let default = self.settings.keys.get(action) == action.default_combo();
        let event = default
            && ctx.input_mut(|i| {
                let before = i.events.len();
                // A picture on the system clipboard arrives as its own kind of paste.
                i.events.retain(|event| !matches!((action, event), (Action::CopyPose, egui::Event::Copy) | (Action::PastePose, egui::Event::Paste(_) | egui::Event::PasteImage(_))));
                i.events.len() != before
            });
        self.pressed(ctx, action) || event
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // An open menu owns the keyboard, so Esc closes it instead of deselecting.
        let blocked = ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(ctx) || self.prefs.capturing.is_some() || self.leaving.is_some() || self.connect.is_open() || self.update.is_showing();
        if blocked {
            return;
        }

        if self.pressed(ctx, Action::OpenProject) {
            self.request_leave(ctx, Leave::Open(None));
        }
        if self.pressed(ctx, Action::NewProject) {
            self.request_leave(ctx, Leave::New);
        }
        if self.pressed(ctx, Action::Import) {
            self.open_import_dialog(ctx);
        }
        if self.pressed(ctx, Action::Preferences) {
            self.prefs_open = !self.prefs_open;
        }
        // The rest work on what's in the viewport, which the home page hides.
        if self.home.open {
            return;
        }

        // Before Save, which would otherwise be found inside Ctrl+Shift+S on some layouts.
        if self.pressed(ctx, Action::SaveAs) {
            self.save_project_as(ctx);
        }
        if self.pressed(ctx, Action::Save) {
            self.save_project(ctx);
        }
        if self.pressed(ctx, Action::Redo) {
            self.redo();
        }
        if self.pressed(ctx, Action::Undo) {
            self.undo();
        }
        if self.pressed(ctx, Action::AddCamera) {
            self.add_camera();
        }
        if self.clipboard_shortcut(ctx, Action::CopyPose) {
            if self.keys_in_focus() {
                self.copy_keys(ctx);
            } else {
                self.copy_pose(ctx);
            }
        }
        // Ctrl+Shift+V reaches egui as a paste with Shift held, the same way Ctrl+V does.
        let shift = ctx.input(|i| i.modifiers.shift);
        let mirrored_is_default = self.settings.keys.get(Action::PasteMirrored) == Action::PasteMirrored.default_combo();
        let paste_mirrored = self.pressed(ctx, Action::PasteMirrored);
        let paste = self.clipboard_shortcut(ctx, Action::PastePose);
        if paste_mirrored || (paste && shift && mirrored_is_default) {
            self.paste_mirrored();
        } else if paste {
            if self.pasting_keys {
                self.paste_keys();
            } else {
                self.paste_pose();
            }
        }
        if self.pressed(ctx, Action::MirrorPose) {
            if self.keys_in_focus() {
                self.mirror_selected_keys();
            } else {
                self.mirror_pose();
            }
        }
        if self.pressed(ctx, Action::CameraView) {
            self.toggle_camera_view();
        }
        if self.pressed(ctx, Action::CameraPreview) {
            self.toggle_camera_preview();
        }

        if self.session.is_some() {
            for (i, action) in [Action::AxisX, Action::AxisY, Action::AxisZ].into_iter().enumerate() {
                if self.pressed(ctx, action) {
                    if let Some(session) = &mut self.session {
                        session.press_axis(i);
                    }
                }
            }
            if self.pressed(ctx, Action::Rotate) {
                if let Some(session) = &mut self.session {
                    session.press_rotate();
                }
            }
            if self.pressed(ctx, Action::Cancel) {
                self.cancel_session();
            } else if self.pressed(ctx, Action::Confirm) {
                self.commit_session();
            }
            return;
        }

        if self.pressed(ctx, Action::ClearPosition) {
            self.reset_selected(true, false);
        }
        if self.pressed(ctx, Action::ClearRotation) {
            self.reset_selected(false, true);
        }
        if self.pressed(ctx, Action::DeleteKey) {
            self.delete_keys_at_frame();
        }
        if self.pressed(ctx, Action::DuplicateKeys) {
            self.duplicate_selected_keys();
        }
        if self.pressed(ctx, Action::AddOnionSkin) {
            self.add_onion_skins();
        }
        if self.pressed(ctx, Action::DeleteOnionSkin) {
            self.delete_onion_skins();
        }
        if self.pressed(ctx, Action::Deselect) {
            self.select(Selection::None);
        }
        if self.pressed(ctx, Action::Cancel) {
            if self.box_select.take().is_some() {
                // The box was what Esc was aimed at.
            } else if timeline::cancel_drag(self) {
                // The drag was what Esc was aimed at.
            } else if self.playing {
                self.pause();
            } else if self.prefs_open {
                self.prefs_open = false;
            } else {
                self.timeline.selected.clear();
                self.select(Selection::None);
            }
        }
        if self.pressed(ctx, Action::PlayPause) {
            self.toggle_play();
        }
        if self.pressed(ctx, Action::InsertKey) {
            self.insert_key();
        }
        if self.pressed(ctx, Action::DeleteSelectedKeys) {
            self.delete_selected_keys();
        }
        if self.pressed(ctx, Action::Delete) {
            if self.keys_in_focus() {
                self.delete_selected_keys();
            } else if !self.timeline.hovered {
                self.delete_selection();
            }
        }
        if self.pressed(ctx, Action::JumpStart) {
            self.pause();
            self.set_frame(self.animation.start as f32);
        }
        if self.pressed(ctx, Action::JumpEnd) {
            self.pause();
            self.set_frame(self.animation.end as f32);
        }
        if self.pressed(ctx, Action::FrameBack) {
            self.step_frame(-1);
        }
        if self.pressed(ctx, Action::FrameForward) {
            self.step_frame(1);
        }
        if self.pressed(ctx, Action::NextKey) {
            self.jump_key(true);
        }
        if self.pressed(ctx, Action::PrevKey) {
            self.jump_key(false);
        }
        if self.pressed(ctx, Action::Grab) {
            self.begin_modal(ctx, OpKind::Move);
        }
        if self.pressed(ctx, Action::Rotate) {
            self.begin_modal(ctx, OpKind::Rotate);
        }
        if self.pressed(ctx, Action::ToolSelect) {
            self.tool = Tool::Select;
        }
        if self.pressed(ctx, Action::ToolMove) {
            self.tool = Tool::Move;
        }
        if self.pressed(ctx, Action::ToolRotate) {
            self.tool = Tool::Rotate;
        }
        if self.pressed(ctx, Action::FrameSelected) {
            self.frame_selection();
        }
        if self.pressed(ctx, Action::FrameAll) {
            if self.timeline.hovered {
                timeline::fit_view(self);
            } else {
                self.frame_all();
            }
        }
        if self.pressed(ctx, Action::ToggleOrtho) {
            self.camera.ortho = !self.camera.ortho;
        }
        if self.pressed(ctx, Action::ToggleShading) {
            self.toggle_shading();
        }
        for (action, preset) in [
            (Action::ViewBack, ViewPreset::Back),
            (Action::ViewLeft, ViewPreset::Left),
            (Action::ViewBottom, ViewPreset::Bottom),
            (Action::ViewFront, ViewPreset::Front),
            (Action::ViewRight, ViewPreset::Right),
            (Action::ViewTop, ViewPreset::Top),
        ] {
            if self.pressed(ctx, action) {
                self.camera_view.through = false;
                self.camera.set_preset(preset);
            }
        }
    }
}

fn backend_label(backend: eframe::wgpu::Backend) -> String {
    use eframe::wgpu::Backend;
    match backend {
        Backend::Dx12 => GraphicsApi::Direct3D12.label().to_string(),
        Backend::Vulkan => GraphicsApi::Vulkan.label().to_string(),
        Backend::Gl => GraphicsApi::OpenGl.label().to_string(),
        Backend::Metal => GraphicsApi::Metal.label().to_string(),
        other => other.to_str().to_string(),
    }
}

/// Kaye's logo for the title bar and taskbar.
pub fn window_icon() -> egui::IconData {
    let (rgba, width, height) = icons::logo_full();
    egui::IconData { rgba, width, height }
}

fn backup_interval() -> Duration {
    #[cfg(feature = "devtools")]
    if let Some(seconds) = std::env::var("KAYE_DEV_BACKUP_EVERY").ok().and_then(|text| text.parse().ok()) {
        return Duration::from_secs(seconds);
    }
    BACKUP_EVERY
}

fn pick_save_path(name: &str) -> Option<PathBuf> {
    #[cfg(feature = "devtools")]
    if let Some(path) = std::env::var_os("KAYE_DEV_SAVE_AS") {
        return Some(path.into());
    }
    rfd::FileDialog::new()
        .set_title("Save Kaye project")
        .add_filter("Kaye project", &[project::EXTENSION])
        .set_file_name(format!("{name}.{}", project::EXTENSION))
        .save_file()
}

fn pick_export_path(name: &str) -> Option<PathBuf> {
    #[cfg(feature = "devtools")]
    if let Some(path) = std::env::var_os("KAYE_DEV_EXPORT_AS") {
        return Some(path.into());
    }
    rfd::FileDialog::new()
        .set_title("Export animation")
        .add_filter("Roblox model", &[export::EXTENSION])
        .set_file_name(format!("{name}.{}", export::EXTENSION))
        .save_file()
}

/// A new animation set up from Preferences.
fn new_animation(settings: &Settings) -> Animation {
    Animation {
        end: settings.animation.default_end,
        fps: settings.animation.default_fps,
        looping: settings.animation.looping,
        ..Animation::default()
    }
}

/// Loads an image for the UI background. Anything bigger than a large monitor is
/// scaled down, since it would only cost graphics memory.
fn decode_image(path: &str) -> Result<egui::ColorImage, String> {
    let image = image::open(path).map_err(|e| e.to_string())?;
    let image = if image.width() > 2560 || image.height() > 2560 {
        image.thumbnail(2560, 2560)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}

/// Takes a key press matching this shortcut out of the event queue.
fn consume_combo(ctx: &egui::Context, combo: crate::settings::KeyCombo, repeats: bool) -> bool {
    ctx.input_mut(|i| {
        let mut hit = false;
        i.events.retain(|event| match event {
            egui::Event::Key { key, pressed: true, repeat, modifiers, .. } if *key == combo.key && combo.matches(*modifiers) => {
                hit |= repeats || !*repeat;
                false
            }
            _ => true,
        });
        hit
    })
}

impl eframe::App for KayeApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.paint_background(ui);
        self.fit_to_screen(&ctx);
        self.poll_renderer();
        self.poll_imports();
        self.poll_saves(&ctx);
        self.poll_opens(&ctx);
        self.poll_background(&ctx);
        self.poll_assets();
        self.handle_close_request(&ctx);
        self.outliner_hovering = false;
        // A rig dragged out and let go over Kaye itself isn't a new file to bring in.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !dragout::is_drag_file(p)).collect());
        self.import_paths(&ctx, dropped);
        let working = self.renderer.is_none() || self.loads.load(Ordering::SeqCst) > 0;
        self.loader.show(&ctx, working);
        if self.renderer.is_none() {
            return;
        }
        self.sync_animations();
        self.sync_replay(&ctx);
        self.keep_backup();
        if !self.loader.is_covering() {
            self.handle_shortcuts(&ctx);
        }
        self.tick_playback(&ctx);
        self.toasts.retain(|t| t.until > Instant::now());
        if !self.toasts.is_empty() || self.is_importing() || self.is_saving() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Panels take their space in this order, so the bars and the timeline run the
        // full width of the window.
        chrome::menu_bar(self, ui);
        if self.home.open {
            home::panel(self, ui);
        } else {
            chrome::shelf(self, ui);
            chrome::status_bar(self, ui);
            timeline::panel(self, ui);
            if self.timeline.selected != self.keys_seen {
                self.keys_picked_last = !self.timeline.selected.is_empty();
                self.keys_seen = self.timeline.selected.clone();
            }
            chrome::tool_column(self, ui);
            side::panel(self, ui);
            viewport::panel(self, ui, frame);
            dragout::check(self, &ctx, frame);
        }
        chrome::windows(self, &ctx);
        if !self.loader.is_covering() {
            connect::window(self, &ctx);
        }
        update_popup::window(self, &ctx);
        self.take_thumbnail(&ctx);
        self.apply_settings_changes(&ctx);
        self.settle_live_edit(&ctx);
        if self.startup_frames > 0 {
            self.startup_frames -= 1;
            if self.startup_frames == 0 {
                StartupMarker::path().clear();
            }
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let [r, g, b, a] = theme::cover().to_normalized_gamma_f32();
        [r, g, b, a]
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        dragout::finish(self, raw_input);
        #[cfg(feature = "devtools")]
        if let Some(script) = &mut self.script {
            script.step(_ctx, raw_input);
        }
    }
}
