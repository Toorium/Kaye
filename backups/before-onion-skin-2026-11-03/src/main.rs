#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use eframe::egui;
use eframe::wgpu::{Backends, Instance, InstanceDescriptor};
use kaye::settings::{GraphicsApi, StartupMarker};

fn backends_for(api: GraphicsApi) -> Option<Backends> {
    match api {
        GraphicsApi::Auto => None,
        GraphicsApi::Direct3D12 => Some(Backends::DX12),
        GraphicsApi::Vulkan => Some(Backends::VULKAN),
        GraphicsApi::OpenGl => Some(Backends::GL),
        GraphicsApi::Metal => Some(Backends::METAL),
    }
}

/// True when this machine has a graphics adapter for these backends.
fn has_adapter(backends: Backends) -> bool {
    let mut descriptor = InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backends;
    !pollster::block_on(Instance::new(descriptor).enumerate_adapters(backends)).is_empty()
}

/// What Auto uses when there's a clear first pick. Asking every API at once loads each one's
/// driver, and a bad Vulkan driver can hang or crash right there, before Kaye has a window to
/// say anything in. Direct3D 12 comes with Windows, so it's tried by itself first.
fn auto_backends() -> Option<Backends> {
    (cfg!(windows) && has_adapter(Backends::DX12)).then_some(Backends::DX12)
}

fn main() -> eframe::Result {
    // Files passed on the command line open on launch, which is also how Windows hands
    // over a double-clicked project.
    let open: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    // Only a real build, so a test run can't point `.kaye` files at itself.
    #[cfg(all(windows, not(debug_assertions), not(feature = "devtools")))]
    kaye::association::register();

    let mut settings = kaye::settings::Settings::load();
    let marker = StartupMarker::path();
    let chosen = backends_for(settings.system.graphics_api);
    // The choice can only be changed from inside Kaye, so a bad one must never keep
    // the window from opening. Auto takes over for good, and the app says so.
    let usable = chosen.filter(|b| !marker.left_behind() && has_adapter(*b));
    let graphics_failed = (chosen.is_some() && usable.is_none()).then_some(settings.system.graphics_api);
    if graphics_failed.is_some() {
        settings.system.graphics_api = GraphicsApi::Auto;
        settings.save();
    }
    marker.set();

    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    wgpu_options.surface.present_mode = if settings.system.vsync {
        eframe::wgpu::PresentMode::AutoVsync
    } else {
        eframe::wgpu::PresentMode::AutoNoVsync
    };
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        setup.power_preference = if settings.system.high_performance_gpu {
            eframe::wgpu::PowerPreference::HighPerformance
        } else {
            eframe::wgpu::PowerPreference::LowPower
        };
        if let Some(backends) = usable.or_else(auto_backends) {
            setup.instance_descriptor.backends = backends;
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Kaye")
            .with_icon(kaye::app::window_icon())
            .with_inner_size(kaye::app::WINDOW_SIZE)
            .with_min_inner_size([900.0, 560.0])
            .with_drag_and_drop(true)
            // Scripted test runs should not take focus from whatever is in front.
            .with_active(!cfg!(feature = "devtools")),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options,
        ..Default::default()
    };
    eframe::run_native(
        "Kaye",
        options,
        Box::new(move |cc| Ok(Box::new(kaye::app::KayeApp::new(cc, open, settings, graphics_failed)))),
    )
}
