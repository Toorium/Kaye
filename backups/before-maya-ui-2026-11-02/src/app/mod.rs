//! Application state, actions and keyboard shortcuts.

mod chrome;
pub mod graph;
mod icons;
mod loading;
pub mod preferences;
mod side;
pub mod theme;
pub mod timeline;
mod viewport;

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use eframe::egui_wgpu;
use egui::{Key, Modifiers, Rect};

use crate::animation::{Animation, EaseStyle, Easing, HandleType, KeyId, CHANNELS};
use crate::camera::{OrbitCamera, ThroughView, ViewPreset};
use crate::gizmo::{OpKind, Session, Space};
use crate::history::{History, PoseChange};
use crate::import::{self, Imported};
use crate::math::Xf;
use crate::modifiers::{self, ModifierKind};
use crate::project::{self, Project};
use crate::render::ViewportRenderer;
use crate::scene::{BoneId, BoneKind, PartId, RigId, RigState, Scene};
use crate::settings::{Action, GraphicsApi, ImageFit, Settings, Shading, StartupMarker};

/// Roblox's default for a new camera.
const DEFAULT_FOV: f32 = 70.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    None,
    Rig(RigId),
    Bone(BoneId),
    Part(PartId),
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
    Rig,
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
    Bone(BoneId),
    Parts(RigId),
    /// A modifier card, by its bone and whether it's the IK one.
    Modifier(BoneId, bool),
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

/// Something that would lose the current work, once unsaved work has been dealt with.
#[derive(Clone, Debug, PartialEq)]
pub enum Leave {
    Close,
    New,
    /// Opens this project, or asks which one.
    Open(Option<PathBuf>),
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
    /// Where a drag-box selection in the viewport began.
    pub box_select: Option<egui::Pos2>,
    pub hover: Selection,
    /// The outliner owns `hover` this frame, so the viewport must not clear it.
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
    pub about_open: bool,
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
    pub collapsed: HashSet<RowKey>,
    pub panel_edit: Option<(BoneId, Xf)>,
    pub scroll_to_selection: bool,
    pub viewport_rect: Rect,
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
            leaving: None,
            after_save: None,
            closing: false,
            saving: 0,
            save_tx,
            save_rx,
            open_tx,
            open_rx,
            toasts: Vec::new(),
            about_open: false,
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
            collapsed: HashSet::new(),
            panel_edit: None,
            scroll_to_selection: false,
            viewport_rect: Rect::NOTHING,
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
        app.import_paths(&cc.egui_ctx, open);
        app
    }

    pub fn is_importing(&self) -> bool {
        self.importing > 0
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
        self.extra_bones.clear();
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

    /// The bone whose curves the Graph tab shows.
    pub fn curve_bone(&self) -> Option<BoneId> {
        Some(modifiers::control_of(&self.scene, self.selection_bone()?))
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

    /// Adds a bone to the selection, or takes it out if it's already in.
    pub fn toggle_bone(&mut self, bone: BoneId) {
        if self.selected_bone() == Some(bone) {
            let next = self.extra_bones.iter().next().copied();
            match next {
                Some(next) => {
                    self.extra_bones.remove(&next);
                    self.commit_panel_edit();
                    self.selection = Selection::Bone(next);
                }
                None => self.select(Selection::None),
            }
        } else if !self.extra_bones.remove(&bone) {
            match self.selected_bone() {
                Some(_) => {
                    self.extra_bones.insert(bone);
                }
                None => self.select(Selection::Bone(bone)),
            }
        }
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
        let active = self.selected_bone().filter(|b| all.contains(b)).or(all.first().copied());
        self.commit_panel_edit();
        self.selection = active.map_or(Selection::None, Selection::Bone);
        self.extra_bones = all.into_iter().filter(|b| Some(*b) != active).collect();
    }

    /// Starts a drag on the active bone, carrying the rest of the selection along.
    pub fn start_session(&mut self, bone: BoneId, op: OpKind, lock: Option<crate::gizmo::AxisLock>, modal: bool, mouse: egui::Pos2) {
        self.pause();
        let bone = modifiers::control_of(&self.scene, bone);
        let selected = self.controlled_bones();
        self.session = Some(Session::new(&self.scene, bone, op, lock, modal, mouse).with_extras(&self.scene, &selected));
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
                self.animation.set_key_full(bone, frame, b.pose, b.value);
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
            self.edit_rig("Add IK", |scene, _| {
                modifiers::add_ik(scene, bone);
            });
        }
        self.props_tab = PropsTab::Modifiers;
    }

    pub fn add_unparent(&mut self, bone: BoneId) {
        if !self.has_unparent(bone) {
            self.edit_rig("Add Unparent", |scene, _| modifiers::add_unparent(scene, bone));
        }
        self.props_tab = PropsTab::Modifiers;
    }

    pub fn has_ik(&self, bone: BoneId) -> bool {
        self.scene.modifiers.iter().any(|m| m.bone == bone && matches!(m.kind, ModifierKind::Ik(_)))
    }

    pub fn has_unparent(&self, bone: BoneId) -> bool {
        self.scene.modifiers.iter().any(|m| m.bone == bone && matches!(m.kind, ModifierKind::Unparent(_)))
    }

    pub fn remove_modifier(&mut self, index: usize) {
        self.edit_rig("Remove modifier", |scene, animation| {
            for bone in modifiers::remove(scene, index) {
                animation.tracks.remove(&bone);
            }
        });
    }

    fn drop_removed_selection(&mut self) {
        let scene = &self.scene;
        self.extra_bones.retain(|b| !scene.bones[*b].removed);
        if matches!(self.selection, Selection::Bone(b) if scene.bones[b].removed) {
            self.selection = Selection::None;
        }
        let animation = &self.animation;
        self.timeline.selected.retain(|(b, f)| animation.key_at(*b, *f).is_some());
    }

    /// Adds a camera seeing what the viewport sees.
    pub fn add_camera(&mut self) {
        let at = self.camera.free_frame();
        let mut camera = 0;
        self.edit_rig("Add camera", |scene, _| camera = scene.add_camera(at, DEFAULT_FOV));
        self.camera_view.camera = Some(camera);
        self.camera_view.preview = true;
        self.select(Selection::Bone(camera));
        self.scroll_to_selection = true;
    }

    pub fn delete_camera(&mut self, camera: BoneId) {
        self.edit_rig("Delete camera", |scene, animation| {
            scene.bones[camera].removed = true;
            animation.tracks.remove(&camera);
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
        let view = self.camera.free_frame();
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
                Ok(imported) => self.finish_import(imported),
                Err(message) => {
                    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    self.toast(ToastKind::Error, format!("{name}: {message}"));
                }
            }
        }
    }

    fn finish_import(&mut self, imported: Imported) {
        // A rig that's only been opened isn't work yet, since it can just be imported
        // again. Once there's a project file, that file is missing it.
        let nothing_to_lose = self.project_path.is_none() && !self.has_unsaved_work();
        self.scene.append(imported.scene);
        self.scene.evaluate();
        for warning in imported.warnings {
            self.toast(ToastKind::Warning, warning);
        }
        if let Some((min, max)) = self.scene.bounds(None) {
            self.view.show_grid_floor_y = min.y;
            self.camera.frame_bounds(min, max);
        }
        if nothing_to_lose {
            self.saved_work = project::fingerprint(&self.scene, &self.animation);
        }
    }

    pub fn is_saving(&self) -> bool {
        self.saving > 0
    }

    pub fn project_name(&self) -> Option<String> {
        let path = self.project_path.as_ref()?;
        Some(path.file_stem()?.to_string_lossy().into_owned())
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
                self.show_project_name(ctx);
            }
            Leave::Open(Some(path)) => self.open_project_file(ctx, path),
            Leave::Open(None) => self.open_project_dialog(ctx),
        }
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
        let name = self.project_name().or_else(|| self.scene.rigs.first().map(|r| r.name.clone())).unwrap_or_else(|| "Untitled".to_string());
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

    fn poll_saves(&mut self, ctx: &egui::Context) {
        while let Ok(answer) = self.save_rx.try_recv() {
            self.saving = self.saving.saturating_sub(1);
            match answer {
                Some((path, Ok(work))) => {
                    self.saved_work = work;
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
        self.animation.apply(&mut self.scene, self.frame);
        self.saved_work = project::fingerprint(&self.scene, &self.animation);
        self.project_path = Some(path);
        self.show_project_name(ctx);
    }

    pub fn new_scene(&mut self) {
        self.project_path = None;
        self.saved_work = 0;
        self.session = None;
        self.panel_edit = None;
        self.live_edit = None;
        self.camera_view = CameraView::default();
        self.camera.through = None;
        self.scene = Scene::default();
        self.selection = Selection::None;
        self.extra_bones.clear();
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
        self.sync_settings();
    }

    pub fn frame_all(&mut self) {
        if let Some((min, max)) = self.scene.bounds(None) {
            self.camera.frame_bounds(min, max);
        }
    }

    pub fn frame_selection(&mut self) {
        let bounds = match self.selection {
            Selection::None => self.scene.bounds(None),
            Selection::Rig(r) => self.scene.bounds(Some(r)),
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
            let after = self.scene.bones[session.bone].pose;
            if !after.approx_eq(session.start_pose) {
                let label = match session.op {
                    OpKind::Move => "Move bone",
                    OpKind::Rotate => "Rotate bone",
                };
                let mut changes = vec![PoseChange {
                    bone: session.bone,
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
            let unneeded = only_needed && self.animation.sample(c.bone, frame as f32).is_some_and(|p| p.approx_eq(c.after));
            if !unneeded {
                let value = self.scene.bones[c.bone].value;
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
                a.set_key_full(bone, frame, pose, value);
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
                a.set_key_full(bone, frame, pose, value);
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
        self.edit_animation("Change handles", |a| {
            for ((bone, frame), channel) in targets {
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
                    HandleKind::Aligned => track.pin_handles(index, channel, true, smoothing),
                    HandleKind::Free => track.pin_handles(index, channel, false, smoothing),
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
            Selection::Rig(r) => self.scene.bounds(Some(r)).map(|(min, max)| (min + max) * 0.5),
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
        let Some(bone) = self.selected_bone().filter(|b| !modifiers::locked_by_ik(&self.scene, *b)) else {
            return;
        };
        let mouse = ctx
            .input(|i| i.pointer.latest_pos())
            .unwrap_or(self.viewport_rect.center());
        self.start_session(bone, op, None, true, mouse);
    }

    fn pressed(&self, ctx: &egui::Context, action: Action) -> bool {
        consume_combo(ctx, self.settings.keys.get(action), action.repeats())
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // An open menu owns the keyboard, so Esc closes it instead of deselecting.
        let blocked = ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(ctx) || self.prefs.capturing.is_some() || self.leaving.is_some();
        if blocked {
            return;
        }

        // Before Save, which would otherwise be found inside Ctrl+Shift+S on some layouts.
        if self.pressed(ctx, Action::SaveAs) {
            self.save_project_as(ctx);
        }
        if self.pressed(ctx, Action::Save) {
            self.save_project(ctx);
        }
        if self.pressed(ctx, Action::OpenProject) {
            self.request_leave(ctx, Leave::Open(None));
        }
        if self.pressed(ctx, Action::NewProject) {
            self.request_leave(ctx, Leave::New);
        }
        if self.pressed(ctx, Action::Redo) {
            self.redo();
        }
        if self.pressed(ctx, Action::Undo) {
            self.undo();
        }
        if self.pressed(ctx, Action::Import) {
            self.open_import_dialog(ctx);
        }
        if self.pressed(ctx, Action::Preferences) {
            self.prefs_open = !self.prefs_open;
        }
        if self.pressed(ctx, Action::AddCamera) {
            self.add_camera();
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
        if self.pressed(ctx, Action::DeleteSelectedKeys) || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete)) {
            self.delete_selected_keys();
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
        self.poll_renderer();
        self.poll_imports();
        self.poll_saves(&ctx);
        self.poll_opens(&ctx);
        self.poll_background(&ctx);
        self.handle_close_request(&ctx);
        self.outliner_hovering = false;
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        self.import_paths(&ctx, dropped);
        let working = self.renderer.is_none() || self.loads.load(Ordering::SeqCst) > 0;
        self.loader.show(&ctx, working);
        if self.renderer.is_none() {
            return;
        }
        if !self.loader.is_covering() {
            self.handle_shortcuts(&ctx);
        }
        self.tick_playback(&ctx);
        self.toasts.retain(|t| t.until > Instant::now());
        if !self.toasts.is_empty() || self.is_importing() || self.is_saving() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        chrome::top_bar(self, ui);
        chrome::status_bar(self, ui);
        timeline::panel(self, ui);
        chrome::tool_rail(self, ui);
        side::panel(self, ui);
        viewport::panel(self, ui, frame);
        chrome::windows(self, &ctx);
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
        let [r, g, b, a] = theme::window().to_normalized_gamma_f32();
        [r, g, b, a]
    }

    #[cfg(feature = "devtools")]
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(script) = &mut self.script {
            script.step(ctx, raw_input);
        }
    }
}
