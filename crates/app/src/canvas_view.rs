//! The Canvas panel: hosts `render`'s offscreen Canvas inside egui's frame, with pan and
//! zoom-at-cursor.

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};
use ssmap_core::Command;
use ssmap_render::{
    CanvasRenderer, CanvasScene, GpuCapabilities, Presenter, Quality, RgbaImage, SpriteStore,
    Suitability, View, render_to_image,
};

use crate::editor::Editor;
use crate::handles::{self, Bounds};
use crate::tools::{self, CanvasInput, ToolState};

/// Space left around the Canvas when it is first fitted, in points.
const FIT_MARGIN: f64 = 32.0;
/// How long the view must hold still, in seconds, before a pan or zoom counts as finished and
/// the Canvas redraws at full quality.
const GESTURE_SETTLE: f64 = 0.15;
const MIN_ZOOM: f64 = 0.01;
const MAX_ZOOM: f64 = 64.0;

/// GPU state shared with egui's renderer, created once at startup.
struct CanvasResources {
    renderer: CanvasRenderer,
    presenter: Presenter,
}

/// Creates the Canvas renderer on egui's device, drawing Stamps from `sprites`, and hands it to
/// egui's callback resources. Returns false, leaving the Canvas as a placeholder, when the
/// adapter is below the hardware floor. Called once per run, so a software-adapter warning
/// shows once.
pub fn install(render_state: &egui_wgpu::RenderState, sprites: SpriteStore) -> bool {
    let capabilities = GpuCapabilities::query(&render_state.adapter);
    match capabilities.suitability() {
        Suitability::Ok => {}
        Suitability::SoftwareWarning(message) => eprintln!("Warning: {message}"),
        Suitability::BelowFloor(message) => {
            eprintln!("Error: {message}");
            return false;
        }
    }
    let mut renderer = CanvasRenderer::new(&render_state.device, capabilities);
    renderer.set_sprites(sprites);
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
    true
}

/// Renders the whole Canvas, `longest_side` pixels on its longer side, as for the preview
/// stored in a Map file. Uses the same renderer as the screen.
pub fn render_whole_canvas(
    render_state: &egui_wgpu::RenderState,
    editor: &mut Editor,
    longest_side: u32,
) -> Option<RgbaImage> {
    let mut egui_renderer = render_state.renderer.write();
    let resources = egui_renderer
        .callback_resources
        .get_mut::<CanvasResources>()?;
    let scene = CanvasScene::new(&editor.map.canvas).with_content(editor.scene_content());
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
    /// The origin and zoom of the last frame, to tell when the view moves.
    last_view: Option<([f64; 2], f64)>,
    /// Until this time (egui's clock, in seconds) a pan or zoom is still in progress.
    gesture_until: f64,
}

impl CanvasView {
    pub fn new(gpu: bool) -> Self {
        Self {
            view: None,
            gpu,
            tools: ToolState::default(),
            panel: None,
            last_view: None,
            gesture_until: 0.0,
        }
    }

    /// Whether a Label's text box is open.
    pub fn editing_text(&self) -> bool {
        self.tools.editing.is_some()
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
        self.last_view = None;
        self.gesture_until = 0.0;
        self.tools = ToolState::default();
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
        let view = self.view.get_or_insert_with(|| {
            let canvas = &editor.map.canvas;
            let canvas = [f64::from(canvas.width), f64::from(canvas.height)];
            View::fit(canvas, size, FIT_MARGIN * ppp)
        });
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
        // While the view is moving, draw without Shore effects; once it has held still, ask
        // for one more frame, which draws everything.
        let now = ui.input(|i| i.time);
        if self
            .last_view
            .is_some_and(|last| last != (view.origin, view.zoom))
        {
            self.gesture_until = now + GESTURE_SETTLE;
        }
        self.last_view = Some((view.origin, view.zoom));
        let quality = if now < self.gesture_until {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(
                    self.gesture_until - now,
                ));
            Quality::Interactive
        } else {
            Quality::Full
        };
        // After input, so this frame shows this frame's edits.
        let scene = CanvasScene::new(&editor.map.canvas)
            .with_content(editor.scene_content())
            .with_quality(quality);
        let painter = ui.painter_at(rect);
        if self.gpu {
            painter.add(egui_wgpu::Callback::new_paint_callback(
                rect,
                CanvasCallback { scene, view },
            ));
        } else {
            paint_placeholder(&painter, rect, &scene, &view, ppp);
        }
        if editor.node_edit().is_none()
            && let Some(bounds) = Bounds::of_selection(editor)
        {
            let rotatable = tools::selection_has_label(editor);
            handles::paint(&painter, &bounds, &view, rect.min, ppp, rotatable);
        }
        tools::paint_overlay(
            &painter,
            editor,
            &view,
            &mut self.tools,
            rect.min,
            response.hover_pos(),
            ppp,
        );
        self.text_box(ui, editor, &view, rect.min, ppp);
    }

    /// The box a Label's text is typed in, just below the Label so its real render shows
    /// live above. Clicking away or Escape commits; emptying it deletes the Label.
    fn text_box(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        view: &View,
        origin: egui::Pos2,
        ppp: f64,
    ) {
        let Some(editing) = &mut self.tools.editing else {
            return;
        };
        let id = editing.id;
        let (Some((shown, layout)), Some(label)) =
            (editor.label_layout(id), editor.label(id).cloned())
        else {
            // Undone or deleted meanwhile.
            self.tools.editing = None;
            editor.end_gesture();
            return;
        };
        let bounds = Bounds::around(&layout.corners(&shown)).expect("four corners");
        let at = handles::to_screen(view, origin, ppp, [bounds.min[0], bounds.max[1]]);
        let text_id = egui::Id::new("label_text").with(id);
        let mut output = None;
        egui::Area::new(egui::Id::new("label_text_box"))
            .fixed_pos(at + egui::vec2(0.0, 6.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    output = Some(
                        egui::TextEdit::multiline(&mut editing.text)
                            .id(text_id)
                            .desired_rows(1)
                            .desired_width(220.0)
                            .hint_text("Label text")
                            .show(ui),
                    );
                });
            });
        let Some(mut output) = output else { return };
        let response = output.response.response.clone();
        if editing.just_opened {
            // A new Area's first frame only sizes it, so keep asking until the box has the
            // keyboard, then select the text so typing replaces it.
            if response.has_focus() {
                editing.just_opened = false;
                output
                    .state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::select_all(&output.galley)));
                output.state.store(ui.ctx(), text_id);
            } else {
                response.request_focus();
            }
            ui.ctx().request_repaint();
            return;
        }
        if response.changed() && !editing.text.is_empty() && editing.text != label.text {
            let text = editing.text.clone();
            editor.gesture(Command::SetLabel {
                id,
                label: Box::new(ssmap_core::Label { text, ..label }),
            });
        }
        if response.lost_focus() {
            if editing.text.is_empty() {
                editor.gesture(Command::Delete { ids: vec![id] });
                editor.select(Vec::new());
            }
            self.tools.editing = None;
            editor.end_gesture();
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
