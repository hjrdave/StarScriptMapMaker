//! The Canvas panel: hosts `render`'s offscreen Canvas inside egui's frame, with pan and
//! zoom-at-cursor.

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};
use ssmap_core::Map;
use ssmap_render::{
    CanvasRenderer, CanvasScene, GpuCapabilities, Presenter, RgbaImage, Shapes, View,
    render_to_image,
};

use crate::editor::Editor;
use crate::handles::{self, Bounds};
use crate::tools::{self, CanvasInput, ToolState};

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

/// Renders the whole Canvas, `longest_side` pixels on its longer side, as for the preview
/// stored in a Map file. Uses the same renderer as the screen.
pub fn render_whole_canvas(
    render_state: &egui_wgpu::RenderState,
    map: &Map,
    longest_side: u32,
) -> Option<RgbaImage> {
    let mut egui_renderer = render_state.renderer.write();
    let resources = egui_renderer
        .callback_resources
        .get_mut::<CanvasResources>()?;
    let scene = CanvasScene::from_map(map);
    let view = View::whole_canvas(scene.canvas_size, longest_side);
    Some(render_to_image(
        &render_state.device,
        &render_state.queue,
        &mut resources.renderer,
        &scene,
        &view,
    ))
}

pub struct CanvasView {
    /// `None` until the first frame, which fits the Canvas to the panel.
    view: Option<View>,
    /// False without a wgpu device (as in UI tests); the Canvas is then a flat placeholder.
    gpu: bool,
    tools: ToolState,
    /// Where the panel was last drawn, in points, and the pixels per point then.
    panel: Option<(egui::Rect, f64)>,
    /// The fills last built, and the Map revision they show.
    shapes: Option<(u64, Shapes)>,
}

impl CanvasView {
    pub fn new(gpu: bool) -> Self {
        Self {
            view: None,
            gpu,
            tools: ToolState::default(),
            panel: None,
            shapes: None,
        }
    }

    /// The screen position of a Canvas point, as last drawn.
    pub fn to_screen(&self, point: [f64; 2]) -> Option<egui::Pos2> {
        let (rect, ppp) = self.panel?;
        Some(handles::to_screen(
            self.view.as_ref()?,
            rect.min,
            ppp,
            point,
        ))
    }

    /// Starts afresh for another Map: fits it to the panel on the next frame and forgets
    /// what was drawn and dragged.
    pub fn refit(&mut self) {
        self.view = None;
        self.tools = ToolState::default();
        self.shapes = None;
    }

    /// The Map's fills, built again only when the Map has changed.
    fn shapes(&mut self, map: &Map) -> Shapes {
        match &self.shapes {
            Some((revision, shapes)) if *revision == map.revision() => shapes.clone(),
            _ => {
                let shapes = Shapes::from_map(map);
                self.shapes = Some((map.revision(), shapes.clone()));
                shapes
            }
        }
    }

    /// Zoom as a share of 1:1, in points rather than physical pixels.
    pub fn zoom(&self, pixels_per_point: f64) -> Option<f64> {
        self.view.map(|view| view.zoom / pixels_per_point)
    }

    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let ppp = f64::from(ui.ctx().pixels_per_point());
        let size = [
            (f64::from(rect.width()) * ppp).round().max(1.0) as u32,
            (f64::from(rect.height()) * ppp).round().max(1.0) as u32,
        ];
        let mut scene = CanvasScene::new(&editor.map.canvas);
        let view = self
            .view
            .get_or_insert_with(|| View::fit(scene.canvas_size, size, FIT_MARGIN * ppp));
        view.size = size;

        tools::canvas_input(
            editor,
            view,
            &mut self.tools,
            &CanvasInput {
                response: &response,
                pixels_per_point: ppp,
            },
        );
        if let Some(pointer) = response.hover_pos() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = f64::from(pinch) * (f64::from(scroll) / 200.0).exp();
            let factor = (view.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM) / view.zoom;
            if factor != 1.0 {
                let local = pointer - rect.min;
                view.zoom_at([f64::from(local.x) * ppp, f64::from(local.y) * ppp], factor);
            }
        }

        let view = *view;
        self.panel = Some((rect, ppp));
        // After the tools, so a drag shows on the frame it happens.
        scene.shapes = self.shapes(&editor.map);
        let painter = ui.painter_at(rect);
        if self.gpu {
            painter.add(egui_wgpu::Callback::new_paint_callback(
                rect,
                CanvasCallback { scene, view },
            ));
        } else {
            paint_placeholder(&painter, rect, &scene, &view, ppp);
        }
        if let Some(bounds) = Bounds::of_selection(&editor.map, editor.selection()) {
            handles::paint(&painter, &bounds, &view, rect.min, ppp);
        }
    }
}

/// The pasteboard and a flat Canvas, for when there is no GPU to render with.
fn paint_placeholder(
    painter: &egui::Painter,
    rect: egui::Rect,
    scene: &CanvasScene,
    view: &View,
    ppp: f64,
) {
    let color = |c: ssmap_render::Color| egui::Color32::from_rgb(c.r, c.g, c.b);
    painter.rect_filled(rect, 0.0, color(scene.pasteboard));
    let corner = |p: [f64; 2]| {
        rect.min
            + egui::vec2(
                ((p[0] - view.origin[0]) * view.zoom / ppp) as f32,
                ((p[1] - view.origin[1]) * view.zoom / ppp) as f32,
            )
    };
    let canvas = egui::Rect::from_two_pos(corner([0.0, 0.0]), corner(scene.canvas_size));
    painter.rect_filled(canvas, 0.0, color(scene.backdrop.water));
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
