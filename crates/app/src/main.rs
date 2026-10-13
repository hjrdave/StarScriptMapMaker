//! StarScript Map Maker desktop editor.

mod canvas_view;

use eframe::egui;
use ssmap_core::{Canvas, DistanceUnit, Map, Scale};

use crate::canvas_view::CanvasView;

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
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

struct App {
    map: Map,
    canvas_view: CanvasView,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let render_state = cc
            .wgpu_render_state
            .as_ref()
            .expect("the app runs on the wgpu renderer");
        canvas_view::install(render_state);
        // Until the New Map dialog exists, every session starts with a blank 4096x3072 Map.
        let map = Map::new(Canvas {
            width: 4096,
            height: 3072,
            scale: Scale {
                distance_per_unit: 0.25,
                unit: DistanceUnit::Miles,
            },
        });
        Self {
            map,
            canvas_view: CanvasView::default(),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.canvas_view.show(ui, &self.map));
    }
}
