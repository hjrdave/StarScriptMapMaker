//! The StarScript Map Maker desktop app.

use eframe::egui;

#[derive(Default)]
struct App;

impl eframe::App for App {
    fn ui(&mut self, _ui: &mut egui::Ui, _frame: &mut eframe::Frame) {}
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 860.0])
            .with_title("StarScript Map Maker"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "starscript-map-maker",
        options,
        Box::new(|_cc| Ok(Box::new(App))),
    )
}
