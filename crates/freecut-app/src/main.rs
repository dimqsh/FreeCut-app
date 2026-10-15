//! FreeCut desktop application entry point.
#![forbid(unsafe_code)]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod anim;
mod app;
mod clip_keys;
mod clip_sound;
mod clip_streams;
mod hdr;
mod imports;
mod jobs;
mod looks;
mod luts;
mod posters;
mod preview;
mod prop_keys;
mod recent;
mod recorder;
mod session;
mod settings;
mod skim;
mod sound;
mod theme;
mod thumbs;
mod timeline_commands;
mod ui;
mod waveform;
mod widgets;
mod workspace;

use anyhow::anyhow;

fn main() -> anyhow::Result<()> {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("FreeCut")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([960.0, 600.0]);
    if cfg!(target_os = "macos") {
        // The content runs under a transparent title bar; the top bar
        // keeps room for the window buttons and drags the window
        // (ui-concept §3.5). Other platforms keep native decorations.
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_title_shown(false)
            .with_titlebar_shown(false);
    }
    let mut options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        if std::env::args_os().any(|arg| arg == "--software") {
            #[cfg(windows)]
            {
                // WARP is provided by Windows. FXC avoids an additional DXC
                // runtime and does not require a hardware graphics driver.
                setup.instance_descriptor.backends = eframe::wgpu::Backends::DX12;
                setup
                    .instance_descriptor
                    .backend_options
                    .dx12
                    .shader_compiler = eframe::wgpu::Dx12Compiler::Fxc;
            }
            eprintln!(
                "FreeCut resource budget: {:?}",
                freecut_media::resources::current()
            );
            setup.native_adapter_selector = Some(std::sync::Arc::new(|adapters, surface| {
                adapters.iter().find(|adapter| {
                    adapter.get_info().device_type == eframe::wgpu::DeviceType::Cpu
                        && surface.is_none_or(|surface| adapter.is_surface_supported(surface))
                }).cloned().inspect(|adapter| {
                    eprintln!("FreeCut software UI: {}", adapter.get_info().name);
                }).ok_or_else(|| {
                    "CPU rendering needs a software graphics driver (Mesa lavapipe on Linux or WARP on Windows); no CPU adapter can present this window".to_owned()
                })
            }));
        }
        // The UI only draws textured triangles. Don't require desktop GPU
        // compute/storage limits or 8192px textures from older adapters.
        setup.device_descriptor = std::sync::Arc::new(|adapter| eframe::wgpu::DeviceDescriptor {
            label: Some("FreeCut UI"),
            required_limits: eframe::wgpu::Limits {
                max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d.min(8192),
                ..eframe::wgpu::Limits::downlevel_webgl2_defaults()
            },
            ..Default::default()
        });
    }

    eframe::run_native(
        "FreeCut",
        options,
        Box::new(|cc| Ok(Box::new(app::FreeCutApp::new(cc)))),
    )
    .map_err(|err| anyhow!("failed to start the FreeCut window: {err}"))
}

#[cfg(test)]
mod e2e;
#[cfg(test)]
mod snapshots;
