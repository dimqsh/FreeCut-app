//! FreeCut desktop application entry point.
#![forbid(unsafe_code)]

mod anim;
mod app;
mod clip_keys;
mod clip_sound;
mod clip_streams;
mod hdr;
mod imports;
mod jobs;
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
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

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
