//! StarScript Map Maker desktop editor.

use eframe::egui;

fn main() -> eframe::Result {
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
        Box::new(|cc| Ok(Box::new(ssmap_app::MapMakerApp::new(cc)))),
    )
}
