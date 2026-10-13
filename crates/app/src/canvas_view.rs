//! The Canvas panel: hosts `render`'s offscreen Canvas inside egui's frame, with pan and
//! zoom-at-cursor.

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};
use ssmap_core::Map;
use ssmap_render::{CanvasRenderer, CanvasScene, GpuCapabilities, Presenter, View};

/// Space left around the Canvas when it is first fitted, in points.
const FIT_MARGIN: f64 = 32.0;
const MIN_ZOOM: f64 = 0.01;
const MAX_ZOOM: f64 = 64.0;

/// GPU state shared with egui's renderer, created once at startup.
struct CanvasResources {
    renderer: CanvasRenderer,
    presenter: Presenter,
}

/// Creates the Canvas renderer on egui's device and hands it to egui's callback resources.
pub fn install(render_state: &egui_wgpu::RenderState) {
    let capabilities = GpuCapabilities::query(&render_state.adapter);
    let renderer = CanvasRenderer::new(&render_state.device, capabilities);
    // eframe draws egui without MSAA; the Canvas does its own multisampling offscreen.
    let presenter = Presenter::new(&render_state.device, render_state.target_format, 1);
    render_state
        .renderer
        .write()
        .callback_resources
        .insert(CanvasResources {
            renderer,
            presenter,
        });
}

#[derive(Default)]
pub struct CanvasView {
    /// `None` until the first frame, which fits the Canvas to the panel.
    view: Option<View>,
}

impl CanvasView {
    pub fn show(&mut self, ui: &mut egui::Ui, map: &Map) {
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let ppp = f64::from(ui.ctx().pixels_per_point());
        let size = [
            (f64::from(rect.width()) * ppp).round().max(1.0) as u32,
            (f64::from(rect.height()) * ppp).round().max(1.0) as u32,
        ];
        let scene = CanvasScene::new(&map.canvas);
        let view = self
            .view
            .get_or_insert_with(|| View::fit(scene.canvas_size, size, FIT_MARGIN * ppp));
        view.size = size;

        if response.dragged() {
            let delta = response.drag_delta();
            view.pan([f64::from(delta.x) * ppp, f64::from(delta.y) * ppp]);
        }
        if let Some(pointer) = response.hover_pos() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = f64::from(pinch) * (f64::from(scroll) / 200.0).exp();
            let factor = (view.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM) / view.zoom;
            if factor != 1.0 {
                let local = pointer - rect.min;
                view.zoom_at([f64::from(local.x) * ppp, f64::from(local.y) * ppp], factor);
            }
        }

        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            CanvasCallback { scene, view: *view },
        ));
    }
}

struct CanvasCallback {
    scene: CanvasScene,
    view: View,
}

impl egui_wgpu::CallbackTrait for CanvasCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let CanvasResources {
            renderer,
            presenter,
        } = resources
            .get_mut::<CanvasResources>()
            .expect("installed at startup");
        let texture = renderer.render(device, queue, encoder, &self.scene, &self.view);
        presenter.set_source(device, texture);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let resources: &CanvasResources = resources.get().expect("installed at startup");
        resources.presenter.draw(pass);
    }
}
