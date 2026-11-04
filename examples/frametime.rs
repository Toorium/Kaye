//! Measures real frames in the real app, with vsync off. One run is one scenario, in
//! its own process, so memory and GPU state can't leak between scenarios.
//!
//! Usage: cargo run --release --features devtools --example frametime -- [options]
//!   --name <text>        label for the result
//!   --api <auto|dx12|vulkan|gl>
//!   --file <path>        model to import, through the real import path
//!   --copies <n>         import it this many times
//!   --keys <n>           keys per bone across the animation
//!   --frames <n>         animation length
//!   --linear             Linear easing, instead of the default Bezier
//!   --ik                 IK on the four R15 limbs, with poles
//!   --select <bone>      select this bone, which opens its panels and handles
//!   --shaded             Shaded mode with shadows
//!   --play               play the animation
//!   --graph              show the Graph tab
//!   --preview            add a camera and show Camera Preview
//!   --parts-open         open the Parts folder in the outliner
//!   --orbit              hold the middle mouse button and circle the mouse
//!   --rotate             press R on the selected bone and circle the mouse
//!   --no-aa              turn antialiasing off
//!   --scale <x>          render scale
//!   --idle               let the app repaint only when it asks to
//!   --secs <n>           how long to measure, default 6
//!   --shot <file.png>    save a picture at the end of the run
//!   --out <file>         append the result as one JSON line
//!
//! Needs `devtools`, because without it `KAYE_SETTINGS` is ignored and the run would
//! share the owner's real settings folder. `Cargo.toml` enforces that.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eframe::egui;
use eframe::wgpu::{Backends, PowerPreference, PresentMode};
use egui::{Event, PointerButton, Pos2, pos2};
use glam::{Quat, Vec3};
use kaye::animation::{EaseStyle, Easing};
use kaye::app::{BottomTab, KayeApp, RowKey, Selection};
use kaye::math::Xf;
use kaye::modifiers;
use kaye::scene::BoneId;
use kaye::settings::{Settings, Shading};
use serde_json::json;

const WARM_UP: Duration = Duration::from_millis(1500);
/// The intro's 2.5 seconds and its fade, with a little room.
const INTRO_OVER: Duration = Duration::from_millis(3200);
/// A slow scenario still gets enough frames to mean something.
const MIN_FRAMES: usize = 60;
const LONGEST_MEASURE: Duration = Duration::from_secs(30);

#[derive(Default, Clone)]
struct Options {
    name: String,
    api: String,
    file: Option<PathBuf>,
    copies: usize,
    keys: usize,
    frames: i32,
    linear: bool,
    ik: bool,
    select: Option<String>,
    shaded: bool,
    play: bool,
    graph: bool,
    preview: bool,
    parts_open: bool,
    orbit: bool,
    rotate: bool,
    no_aa: bool,
    scale: f32,
    idle: bool,
    secs: f32,
    shot: Option<String>,
    out: Option<String>,
}

fn parse() -> Options {
    let mut o = Options { api: "auto".into(), copies: 1, frames: 120, scale: 1.0, secs: 6.0, ..Options::default() };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--name" => o.name = value(),
            "--api" => o.api = value(),
            "--file" => o.file = Some(value().into()),
            "--copies" => o.copies = value().parse().unwrap(),
            "--keys" => o.keys = value().parse().unwrap(),
            "--frames" => o.frames = value().parse().unwrap(),
            "--select" => o.select = Some(value()),
            "--scale" => o.scale = value().parse().unwrap(),
            "--secs" => o.secs = value().parse().unwrap(),
            "--shot" => o.shot = Some(value()),
            "--out" => o.out = Some(value()),
            "--linear" => o.linear = true,
            "--ik" => o.ik = true,
            "--shaded" => o.shaded = true,
            "--play" => o.play = true,
            "--graph" => o.graph = true,
            "--preview" => o.preview = true,
            "--parts-open" => o.parts_open = true,
            "--orbit" => o.orbit = true,
            "--rotate" => o.rotate = true,
            "--no-aa" => o.no_aa = true,
            "--idle" => o.idle = true,
            other => panic!("unknown option {other}"),
        }
    }
    o
}

/// CPU time and memory of this process, straight from Windows.
#[cfg(windows)]
mod system {
    #[repr(C)]
    #[derive(Default)]
    struct FileTime(u32, u32);

    #[repr(C)]
    #[derive(Default)]
    struct MemoryCounters {
        size: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged: usize,
        quota_paged: usize,
        quota_peak_nonpaged: usize,
        quota_nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }

    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetProcessTimes(process: isize, a: *mut FileTime, b: *mut FileTime, c: *mut FileTime, d: *mut FileTime) -> i32;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut MemoryCounters, size: u32) -> i32;
    }

    /// Seconds of CPU this process has used, across all its threads.
    pub fn cpu_seconds() -> f64 {
        let (mut a, mut b, mut kernel, mut user) = Default::default();
        // Safe: the out pointers are live locals and the handle is a pseudo handle.
        unsafe { GetProcessTimes(GetCurrentProcess(), &mut a, &mut b, &mut kernel, &mut user) };
        let ticks = |t: &FileTime| ((t.1 as u64) << 32 | t.0 as u64) as f64;
        (ticks(&kernel) + ticks(&user)) / 1e7
    }

    /// Working set now, peak working set, and private bytes, in megabytes.
    pub fn memory_mb() -> (f64, f64, f64) {
        let mut m = MemoryCounters { size: size_of::<MemoryCounters>() as u32, ..Default::default() };
        // Safe: `m` is live and `size` matches it.
        unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut m, m.size) };
        let mb = |bytes: usize| bytes as f64 / 1e6;
        (mb(m.working_set), mb(m.peak_working_set), mb(m.pagefile))
    }
}

fn percentile(sorted: &[f64], at: f64) -> f64 {
    sorted.get(((sorted.len() - 1) as f64 * at) as usize).copied().unwrap_or(0.0)
}

fn stats(values: &[f64]) -> serde_json::Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    json!({
        "mean": values.iter().sum::<f64>() / values.len().max(1) as f64,
        "p50": percentile(&sorted, 0.50),
        "p95": percentile(&sorted, 0.95),
        "p99": percentile(&sorted, 0.99),
        "max": sorted.last().copied().unwrap_or(0.0),
    })
}

#[derive(PartialEq)]
enum Stage {
    /// Waiting for the renderer and the intro.
    Starting,
    Importing,
    WarmingUp(Instant),
    Measuring(Instant),
    Shot,
    Done,
}

struct Harness {
    app: KayeApp,
    o: Options,
    launched: Instant,
    stage: Stage,
    first_ui: Option<f64>,
    ready: Option<f64>,
    import_seconds: f64,
    import_started: Option<Instant>,
    frame_ms: Vec<f64>,
    ui_ms: Vec<f64>,
    cpu_ms: Vec<f64>,
    intro_ms: Vec<f64>,
    last_frame: Option<Instant>,
    cpu_at_start: f64,
    working_set_at_start: f64,
    orbit_angle: f32,
    rotating: bool,
    /// Set when an idle measurement's time is up, since an idle app won't call back.
    idle_over: Arc<AtomicBool>,
    shot_saved: Arc<AtomicBool>,
}

fn swing(index: usize, time: f32) -> Xf {
    let phase = index as f32 * 0.7 + time * std::f32::consts::TAU;
    Xf::new(
        Quat::from_euler(glam::EulerRot::XYZ, phase.sin() * 0.6, (phase * 0.5).cos() * 0.3, (phase * 1.5).sin() * 0.2),
        Vec3::new(0.0, (phase * 2.0).sin() * 0.05, 0.0),
    )
}

impl Harness {
    fn is_ready(&self) -> bool {
        // Kaye starts on the home page, so the viewport can't be what says it's ready.
        self.app.has_renderer()
    }

    fn build_scene(&mut self) {
        let app = &mut self.app;
        // The throwaway settings have no key, and the popup asking for one would be measured.
        app.connect.close();
        app.frame_all();
        let joints: Vec<BoneId> = (0..app.scene.bones.len()).filter(|b| app.scene.bones[*b].kind.is_joint()).collect();
        if self.o.ik {
            let rigs = app.scene.rigs.len();
            for rig in 0..rigs {
                for name in ["LeftAnkle", "RightAnkle", "LeftWrist", "RightWrist"] {
                    let bone = app.scene.rigs[rig].bones.iter().copied().find(|b| app.scene.bones[*b].name == name);
                    if let Some(bone) = bone {
                        app.add_ik(bone);
                        let index = app.scene.modifiers.len() - 1;
                        modifiers::add_pole(&mut app.scene, index);
                    }
                }
            }
        }
        if self.o.linear {
            app.animation.default_easing = Easing { style: EaseStyle::Linear, ..Easing::default() };
        }
        app.animation.end = self.o.frames;
        if self.o.keys > 0 {
            let (keys, length) = (self.o.keys as i32, self.o.frames);
            for (index, bone) in joints.iter().enumerate() {
                for key in 0..keys {
                    let frame = key * length / (keys - 1).max(1);
                    app.animation.set_key(*bone, frame, swing(index, key as f32 / keys as f32));
                }
            }
            app.animation.apply(&mut app.scene, 0.0);
        }
        if let Some(name) = &self.o.select {
            let bone = app.scene.bones.iter().position(|b| &b.name == name).unwrap_or_else(|| panic!("no bone {name}"));
            app.select(Selection::Bone(bone));
        }
        if self.o.graph {
            app.bottom_tab = BottomTab::Graph;
        }
        if self.o.preview {
            app.add_camera();
            app.camera_view.preview = true;
            // A new camera gets selected, which would shrink the timeline to one row
            // and make this look cheaper than the run it's compared with.
            app.select(Selection::None);
        }
        if self.o.parts_open {
            for rig in 0..app.scene.rigs.len() {
                app.collapsed.insert(RowKey::Parts(rig));
            }
        }
        if self.o.play {
            app.toggle_play();
        }
    }

    fn write_result(&self, measured: f64) {
        let (working, peak, private) = system::memory_mb();
        let cpu = (system::cpu_seconds() - self.cpu_at_start) / measured * 100.0;
        let result = json!({
            "name": self.o.name,
            "api": self.o.api,
            "gpu": format!("{} ({})", self.app.gpu_name, self.app.gpu_backend),
            "frames": self.frame_ms.len(),
            "fps": self.frame_ms.len() as f64 / measured,
            "frame_ms": stats(&self.frame_ms),
            "ui_ms": stats(&self.ui_ms),
            "cpu_ms": stats(&self.cpu_ms),
            "intro_ui_ms": stats(&self.intro_ms),
            "process_cpu_percent_of_one_core": cpu,
            "working_set_start_mb": self.working_set_at_start,
            "working_set_mb": working,
            "peak_working_set_mb": peak,
            "private_mb": private,
            "first_ui_s": self.first_ui,
            "ready_s": self.ready,
            "import_s": self.import_seconds,
            "bones": self.app.scene.bones.len(),
            "parts": self.app.scene.parts.len(),
            "modifiers": self.app.scene.modifiers.len(),
            "tracks": self.app.animation.tracks.len(),
        });
        println!("{result}");
        if let Some(path) = &self.o.out {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
            writeln!(file, "{result}").unwrap();
        }
    }
}

impl eframe::App for Harness {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        self.first_ui.get_or_insert(self.launched.elapsed().as_secs_f64());
        let began = Instant::now();
        self.app.ui(ui, frame);
        let ui_ms = began.elapsed().as_secs_f64() * 1000.0;
        let continuous = !self.o.idle || !matches!(self.stage, Stage::Measuring(_));
        if continuous {
            ctx.request_repaint();
            // Painting has to carry on if some other window ends up over this one.
            // This counts as a repaint request too, so idle runs have to skip it.
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
        }

        match self.stage {
            Stage::Starting => {
                self.intro_ms.push(ui_ms);
                if self.ready.is_none() && self.is_ready() {
                    self.ready = Some(self.launched.elapsed().as_secs_f64());
                }
                // The renderer arrives well before the intro ends, and the intro
                // would be what gets measured.
                if self.ready.is_some() && self.launched.elapsed() >= INTRO_OVER {
                    if let Some(file) = self.o.file.clone() {
                        self.import_started = Some(Instant::now());
                        self.app.import_paths(&ctx, vec![file; self.o.copies]);
                        self.stage = Stage::Importing;
                    } else {
                        self.build_scene();
                        self.stage = Stage::WarmingUp(Instant::now());
                    }
                }
            }
            Stage::Importing => {
                if !self.app.is_importing() {
                    self.import_seconds = self.import_started.map_or(0.0, |t| t.elapsed().as_secs_f64());
                    self.build_scene();
                    self.stage = Stage::WarmingUp(Instant::now());
                }
            }
            Stage::WarmingUp(since) => {
                if since.elapsed() >= WARM_UP {
                    self.cpu_at_start = system::cpu_seconds();
                    self.working_set_at_start = system::memory_mb().0;
                    self.last_frame = None;
                    self.stage = Stage::Measuring(Instant::now());
                    if self.o.idle {
                        let (over, ctx, secs) = (self.idle_over.clone(), ctx.clone(), self.o.secs);
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_secs_f32(secs));
                            over.store(true, Ordering::SeqCst);
                            ctx.request_repaint();
                        });
                    }
                }
            }
            Stage::Measuring(since) => {
                if let Some(last) = self.last_frame {
                    self.frame_ms.push(now.duration_since(last).as_secs_f64() * 1000.0);
                    self.ui_ms.push(ui_ms);
                    self.cpu_ms.push(frame.info().cpu_usage.unwrap_or(0.0) as f64 * 1000.0);
                }
                self.last_frame = Some(now);
                let elapsed = since.elapsed();
                let timed_out = elapsed.as_secs_f32() >= self.o.secs;
                let enough = self.o.idle || self.frame_ms.len() >= MIN_FRAMES;
                let too_long = elapsed >= LONGEST_MEASURE.max(Duration::from_secs_f32(self.o.secs + 5.0));
                if (self.idle_over.load(Ordering::SeqCst) || (!self.o.idle && timed_out && enough)) || too_long {
                    self.write_result(elapsed.as_secs_f64());
                    self.stage = Stage::Shot;
                    if let Some(path) = self.o.shot.clone() {
                        let (saved, repaint) = (self.shot_saved.clone(), ctx.clone());
                        ctx.request_screenshot(move |image| {
                            let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                            image::save_buffer(&path, &bytes, image.size[0] as u32, image.size[1] as u32, image::ColorType::Rgba8).expect("could not save picture");
                            saved.store(true, Ordering::SeqCst);
                            repaint.request_repaint();
                        });
                    } else {
                        self.shot_saved.store(true, Ordering::SeqCst);
                    }
                }
            }
            Stage::Shot => {
                if self.shot_saved.load(Ordering::SeqCst) {
                    self.stage = Stage::Done;
                    self.app.close_without_asking(&ctx);
                }
            }
            Stage::Done => {}
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.app.clear_color(visuals)
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        // The real mouse doesn't count, so a passing cursor can't change a result.
        raw.events.retain(|e| !matches!(e, Event::PointerMoved(_) | Event::PointerButton { .. } | Event::PointerGone | Event::MouseWheel { .. }));
        let moving = self.o.orbit || self.o.rotate;
        if !moving || !matches!(self.stage, Stage::WarmingUp(_) | Stage::Measuring(_)) {
            return;
        }
        let center = self.app.viewport_rect.center();
        let at = |angle: f32| -> Pos2 { pos2(center.x + angle.cos() * 180.0, center.y + angle.sin() * 120.0) };
        if self.orbit_angle == 0.0 {
            raw.events.push(Event::PointerMoved(at(0.0)));
            if self.o.orbit {
                raw.events.push(Event::PointerButton { pos: at(0.0), button: PointerButton::Middle, pressed: true, modifiers: egui::Modifiers::NONE });
            }
        } else if self.o.rotate && !self.rotating {
            // A frame later, so the app has seen where the mouse is.
            self.rotating = true;
            for pressed in [true, false] {
                raw.events.push(Event::Key { key: egui::Key::R, physical_key: Some(egui::Key::R), pressed, repeat: false, modifiers: egui::Modifiers::NONE });
            }
        }
        self.orbit_angle += 0.05;
        raw.events.push(Event::PointerMoved(at(self.orbit_angle)));
    }
}

fn main() -> eframe::Result {
    let o = parse();
    let launched = Instant::now();
    assert!(std::env::var_os("KAYE_SETTINGS").is_some(), "set KAYE_SETTINGS to a throwaway file");
    assert!(o.name.len() > 0, "--name is needed");

    // Defaults, so the owner's preferences can't change a result.
    let mut settings = Settings::default();
    settings.system.vsync = false;
    settings.system.antialiasing = !o.no_aa;
    settings.system.render_scale = o.scale;
    if o.shaded {
        settings.viewport.shading = Shading::Shaded;
    }

    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    wgpu_options.surface.present_mode = PresentMode::AutoNoVsync;
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        setup.power_preference = PowerPreference::HighPerformance;
        match o.api.as_str() {
            "auto" => {}
            "dx12" => setup.instance_descriptor.backends = Backends::DX12,
            "vulkan" => setup.instance_descriptor.backends = Backends::VULKAN,
            "gl" => setup.instance_descriptor.backends = Backends::GL,
            other => panic!("unknown api {other}"),
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Kaye performance run")
            .with_inner_size([1600.0, 900.0])
            .with_position([20.0, 20.0])
            // Stays on top so the compositor can't throttle a covered window, and
            // doesn't take focus from whatever the owner is doing.
            .with_always_on_top()
            .with_active(false),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options,
        ..Default::default()
    };
    eframe::run_native(
        "Kaye performance run",
        options,
        Box::new(move |cc| {
            let mut app = KayeApp::new(cc, Vec::new(), settings, None);
            app.connect.close();
            Ok(Box::new(Harness {
                app,
                o,
                launched,
                stage: Stage::Starting,
                first_ui: None,
                ready: None,
                import_seconds: 0.0,
                import_started: None,
                frame_ms: Vec::new(),
                ui_ms: Vec::new(),
                cpu_ms: Vec::new(),
                intro_ms: Vec::new(),
                last_frame: None,
                cpu_at_start: 0.0,
                working_set_at_start: 0.0,
                orbit_angle: 0.0,
                rotating: false,
                idle_over: Arc::new(AtomicBool::new(false)),
                shot_saved: Arc::new(AtomicBool::new(false)),
            }))
        }),
    )
}
