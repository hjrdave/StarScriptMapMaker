//! StarScript Map Maker desktop editor.

// Release builds are GUI apps on Windows: no console window opens beside the editor.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;

fn main() -> eframe::Result {
    // A double-clicked Map file arrives as an argument on Windows and Linux. macOS sends an
    // open-documents event instead, which winit does not expose yet (see docs/packaging.md).
    let launch_file = ssmap_app::launch_file(std::env::args_os().skip(1));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("StarScript Map Maker")
            .with_inner_size([1280.0, 800.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "StarScript Map Maker",
        options,
        Box::new(move |cc| {
            let mut app = ssmap_app::MapMakerApp::new(cc);
            if let Some(path) = launch_file {
                // A file that fails to open leaves a blank Map and an error in the status bar.
                app.open_path(&path);
            }
            Ok(Box::new(app))
        }),
    )
}
