//! The Canvas panel: hosts `render`'s offscreen Canvas inside egui's frame, with pan and
//! zoom-at-cursor.

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};
use ssmap_core::Command;
use ssmap_render::{
    CanvasRenderer, CanvasScene, ExportError, ExportJob, ExportStep, GpuCapabilities, Presenter,
    Quality, RgbaImage, SpriteStore, Suitability, TessellationCache, View, render_to_image,
};

use crate::canvas_controls::{self, BannerAction};
use crate::editor::Editor;
use crate::handles::{self, Bounds};
use crate::tessellation::BackgroundTessellation;
use crate::tools::{self, ToolId, ToolState};
use crate::zoom::{self, FRAME_MARGIN as FIT_MARGIN, MAX_ZOOM, MIN_ZOOM, Zoom};

/// How long the view must hold still, in seconds, before a pan or zoom counts as finished and
/// the Canvas redraws at full quality.
const GESTURE_SETTLE: f64 = 0.15;

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
    // Each tab brings its own tessellation cache, which the renderer switches to when it draws.
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
    cache: Option<&TessellationCache>,
    longest_side: u32,
) -> Option<RgbaImage> {
    let mut egui_renderer = render_state.renderer.write();
    let resources = egui_renderer
        .callback_resources
        .get_mut::<CanvasResources>()?;
    if let Some(cache) = cache {
        resources.renderer.use_tessellation_cache(cache);
    }
    let mut scene = CanvasScene::new(&editor.map.canvas).with_content(editor.scene_content());
    // The preview is of the Canvas alone, so off-Canvas Objects stay out of it.
    scene.off_canvas_objects = false;
    let view = View::whole_canvas_at(scene.canvas_origin, scene.canvas_size, longest_side);
    // A failed readback leaves the Map without a preview rather than crashing.
    render_to_image(
        &render_state.device,
        &render_state.queue,
        &mut resources.renderer,
        &scene,
        &view,
    )
    .ok()
}

/// Advances `job` by one step on the same renderer as the screen. `None` without a GPU.
pub fn step_export(
    render_state: &egui_wgpu::RenderState,
    editor: &mut Editor,
    cache: Option<&TessellationCache>,
    job: &mut ExportJob,
) -> Option<Result<ExportStep, ExportError>> {
    let mut egui_renderer = render_state.renderer.write();
    let resources = egui_renderer
        .callback_resources
        .get_mut::<CanvasResources>()?;
    if let Some(cache) = cache {
        resources.renderer.use_tessellation_cache(cache);
    }
    Some(editor.step_export(
        job,
        &render_state.device,
        &render_state.queue,
        &mut resources.renderer,
    ))
}

/// Gives `view` a new target size, keeping the Canvas point at the middle of the target where
/// it is, so the Map does not jump when a panel opens, closes or resizes beside it.
fn resize_keeping_centre(view: &mut View, size: [u32; 2]) {
    if view.size == size {
        return;
    }
    let centre = view.to_canvas([f64::from(view.size[0]) / 2.0, f64::from(view.size[1]) / 2.0]);
    view.size = size;
    view.origin = [
        centre[0] - f64::from(size[0]) / (2.0 * view.zoom),
        centre[1] - f64::from(size[1]) / (2.0 * view.zoom),
    ];
}

pub struct CanvasView {
    /// `None` until the first frame, which fits the Canvas to the panel.
    view: Option<View>,
    /// False without a wgpu device (as in UI tests); the Canvas is then a flat placeholder.
    gpu: bool,
    /// The tessellated Objects of this tab's Map, kept while another tab is shown.
    cache: Option<TessellationCache>,
    /// Tessellates Objects on a thread of its own after a Map opens.
    tessellation: BackgroundTessellation,
    tools: ToolState,
    /// Where the panel was last drawn, in points, and the pixels per point then.
    panel: Option<(egui::Rect, f64)>,
    /// The origin and zoom of the last frame, to tell when the view moves.
    last_view: Option<([f64; 2], f64)>,
    /// Until this time (egui's clock, in seconds) a pan or zoom is still in progress.
    gesture_until: f64,
    /// A zoom asked for by a shortcut, applied on the next frame.
    pending_zoom: Option<Zoom>,
    /// The zoom menu's percentage text.
    zoom_field: String,
    /// Where the right-click that opened the Canvas menu landed, in Canvas units: Paste puts
    /// the pasted items' middle there.
    menu_point: Option<[f64; 2]>,
}

impl CanvasView {
    /// `cache` is this tab's tessellation cache; `None` without a wgpu device.
    pub fn new(cache: Option<TessellationCache>) -> Self {
        Self {
            view: None,
            gpu: cache.is_some(),
            cache: cache.clone(),
            tessellation: BackgroundTessellation::new(cache),
            tools: ToolState::default(),
            panel: None,
            last_view: None,
            gesture_until: 0.0,
            pending_zoom: None,
            zoom_field: String::new(),
            menu_point: None,
        }
    }

    /// Zooms on the next frame, as from a shortcut.
    pub fn request_zoom(&mut self, zoom: Zoom) {
        self.pending_zoom = Some(zoom);
    }

    /// This tab's tessellation cache; `None` without a wgpu device.
    pub fn cache(&self) -> Option<&TessellationCache> {
        self.cache.as_ref()
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

    /// The Canvas point under `pointer` (screen points), when it is over the Canvas as last
    /// drawn.
    pub fn canvas_point(&self, pointer: egui::Pos2) -> Option<[f64; 2]> {
        let (rect, ppp) = self.panel?;
        if !rect.contains(pointer) {
            return None;
        }
        let local = pointer - rect.min;
        Some(
            self.view?
                .to_canvas([f64::from(local.x) * ppp, f64::from(local.y) * ppp]),
        )
    }

    /// Where Paste puts the middle of what it pastes: the Canvas point under `pointer` (the
    /// last cursor position) when it is over the Canvas, else the middle of what is visible.
    /// Uses this tab's own zoom and pan.
    pub fn paste_point(&self, pointer: Option<egui::Pos2>) -> Option<[f64; 2]> {
        pointer
            .and_then(|pointer| self.canvas_point(pointer))
            .or_else(|| {
                self.view
                    .as_ref()
                    .map(|view| view.to_canvas(zoom::centre(view)))
            })
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
            View::fit_at(editor.map.canvas.origin, canvas, size, FIT_MARGIN * ppp)
        });
        resize_keeping_centre(view, size);

        // Before the tools, so a click on the control is not also a click on the Canvas.
        let requested = canvas_controls::zoom_control(
            ui.ctx(),
            rect,
            view.zoom / ppp * 100.0,
            !editor.selection().is_empty(),
            &mut self.zoom_field,
        )
        .or(self.pending_zoom.take());
        if let Some(requested) = requested {
            let canvas = &editor.map.canvas;
            let canvas = [f64::from(canvas.width), f64::from(canvas.height)];
            let bounds = if requested == Zoom::Selection {
                Bounds::of_selection(editor)
            } else {
                None
            };
            zoom::apply(view, requested, ppp, canvas, bounds);
        }

        tools::canvas_input(editor, view, &mut self.tools, &response, ppp);
        Self::context_menu(&mut self.menu_point, &response, editor, view, rect, ppp);
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
            // Before the callback is queued, so this frame already leaves out Objects that
            // the worker has yet to finish.
            let ctx = ui.ctx().clone();
            self.tessellation
                .update(&scene.content, editor.outlines_rebuilding(), move || {
                    ctx.request_repaint()
                });
            painter.add(egui_wgpu::Callback::new_paint_callback(
                rect,
                CanvasCallback {
                    cache: self.cache.clone(),
                    scene,
                    view,
                    ctx: ui.ctx().clone(),
                },
            ));
        } else {
            paint_placeholder(&painter, rect, &scene, &view, ppp);
        }
        if editor.node_edit().is_none()
            && let Some(bounds) = Bounds::of_selection(editor)
        {
            let rotatable = tools::selection_has_label(editor);
            let select = crate::theme::palette(ui.ctx()).select;
            handles::paint(&painter, &bounds, &view, rect.min, ppp, rotatable, select);
        }
        paint_missing_names(&painter, editor, &view, rect.min, ppp);
        tools::paint_overlay(
            &painter,
            editor,
            &view,
            &mut self.tools,
            rect.min,
            response.hover_pos(),
            ppp,
        );
        if let Some(text) = self.tools.canvas_hud(editor)
            && let Some(pointer) = ui.input(|i| i.pointer.latest_pos())
        {
            let palette = crate::theme::palette(ui.ctx());
            egui::Area::new(egui::Id::new("canvas_hud"))
                .order(egui::Order::Tooltip)
                .interactable(false)
                .fixed_pos(pointer + egui::vec2(14.0, 14.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::NONE
                        .fill(palette.bg_raised)
                        .stroke(egui::Stroke::new(1.0, palette.border_strong))
                        .corner_radius(crate::theme::RADIUS_BUTTON)
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(text)
                                        .size(crate::theme::CAPTION)
                                        .color(palette.text_primary),
                                )
                                .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        });
                });
        }
        self.node_banner(ui, editor, rect);
        self.text_box(ui, editor, &view, rect.min, ppp);
    }

    /// The right-click menu of the Select tool. A right-click on something not yet selected
    /// selects it first; the items are the Edit menu's Commands, nothing more.
    fn context_menu(
        menu_point: &mut Option<[f64; 2]>,
        response: &egui::Response,
        editor: &mut Editor,
        view: &View,
        rect: egui::Rect,
        ppp: f64,
    ) {
        if editor.active_tool != ToolId::Select || editor.node_edit().is_some() {
            return;
        }
        if response.secondary_clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let local = pos - rect.min;
            let point = view.to_canvas([f64::from(local.x) * ppp, f64::from(local.y) * ppp]);
            *menu_point = Some(point);
            if let Some(id) = editor.pick(point)
                && !editor.is_selected(id)
            {
                editor.select(vec![id]);
            }
        }
        let at = *menu_point;
        response.context_menu(|ui| {
            let has_selection = !editor.selection().is_empty();
            if ui
                .add_enabled(has_selection, egui::Button::new("Copy"))
                .clicked()
            {
                editor.copy_selection();
                ui.close();
            }
            if ui
                .add_enabled(has_selection, egui::Button::new("Cut"))
                .clicked()
            {
                editor.cut_selection();
                ui.close();
            }
            if ui
                .add_enabled(editor.has_clipboard(), egui::Button::new("Paste"))
                .clicked()
            {
                let centre = at.unwrap_or_else(|| view.to_canvas(zoom::centre(view)));
                editor.paste(centre);
                ui.close();
            }
            if ui
                .add_enabled(has_selection, egui::Button::new("Group"))
                .clicked()
            {
                editor.group_selection();
                ui.close();
            }
            if ui
                .add_enabled(has_selection, egui::Button::new("Delete"))
                .clicked()
            {
                editor.delete_selection();
                ui.close();
            }
        });
    }

    /// The banner shown while nodes are edited. Cancel drops a gesture in progress or else
    /// leaves node editing, as Escape does; Done leaves it, as Return does.
    fn node_banner(&mut self, ui: &egui::Ui, editor: &mut Editor, rect: egui::Rect) {
        if editor.active_tool != ToolId::Select {
            return;
        }
        let Some(edit) = editor.node_edit() else {
            return;
        };
        let selected = edit.nodes.len();
        let total = editor.edited_shape().iter().sum();
        match canvas_controls::node_banner(ui.ctx(), rect, selected, total) {
            Some(BannerAction::Cancel) => {
                if !editor.cancel_open_gesture() {
                    editor.leave_node_edit();
                }
                self.tools.clear_node_gestures(editor);
            }
            Some(BannerAction::Done) => {
                editor.leave_node_edit();
                self.tools.clear_node_gestures(editor);
            }
            None => {}
        }
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
            if let Some(editing) = self.tools.editing.take() {
                editing.finish(editor);
            }
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
            editing.run(
                editor,
                Command::SetLabel {
                    id,
                    label: Box::new(ssmap_core::Label { text, ..label }),
                },
            );
        }
        if response.lost_focus() {
            if editing.text.is_empty() {
                editing.run(editor, Command::Delete { ids: vec![id] });
                editor.select(Vec::new());
            }
            if let Some(editing) = self.tools.editing.take() {
                editing.finish(editor);
            }
        }
    }
}

/// Writes the Asset's name on the box drawn for each Stamp whose Asset is not installed.
fn paint_missing_names(
    painter: &egui::Painter,
    editor: &Editor,
    view: &View,
    origin: egui::Pos2,
    ppp: f64,
) {
    for (position, scale, name) in editor.missing_stamps() {
        let center = handles::to_screen(view, origin, ppp, position);
        let width = (ssmap_render::PLACEHOLDER_SIZE[0] * scale * view.zoom / ppp) as f32;
        let galley = painter.layout(
            name,
            egui::FontId::proportional(11.0),
            egui::Color32::from_gray(40),
            width.max(1.0),
        );
        painter.galley(center - galley.size() / 2.0, galley, egui::Color32::BLACK);
    }
}

/// The off-Canvas and a flat Canvas, for when there is no GPU to render with.
fn paint_placeholder(
    painter: &egui::Painter,
    rect: egui::Rect,
    scene: &CanvasScene,
    view: &View,
    ppp: f64,
) {
    let color = |c: ssmap_render::Color| egui::Color32::from_rgb(c.r, c.g, c.b);
    painter.rect_filled(rect, 0.0, color(scene.off_canvas));
    let corner = |p: [f64; 2]| {
        rect.min
            + egui::vec2(
                ((p[0] - view.origin[0]) * view.zoom / ppp) as f32,
                ((p[1] - view.origin[1]) * view.zoom / ppp) as f32,
            )
    };
    let canvas = egui::Rect::from_two_pos(
        corner(scene.canvas_origin),
        corner([
            scene.canvas_origin[0] + scene.canvas_size[0],
            scene.canvas_origin[1] + scene.canvas_size[1],
        ]),
    );
    painter.rect_filled(canvas, 0.0, color(scene.backdrop.water));
}

struct CanvasCallback {
    /// The shown tab's cache, which the renderer draws from this frame.
    cache: Option<TessellationCache>,
    scene: CanvasScene,
    view: View,
    /// Asked for another frame when this one left finished results to apply.
    ctx: egui::Context,
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
        if let Some(cache) = &self.cache {
            renderer.use_tessellation_cache(cache);
        }
        let texture = renderer.render(device, queue, encoder, &self.scene, &self.view);
        presenter.set_source(device, texture);
        if renderer.apply_pending() {
            self.ctx.request_repaint();
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use ssmap_core::{Canvas, DistanceUnit, Id, Map, MultiPolygon, NodeRef, Polygon, Scale};

    fn centre(view: &View) -> [f64; 2] {
        view.to_canvas([f64::from(view.size[0]) / 2.0, f64::from(view.size[1]) / 2.0])
    }

    #[test]
    fn the_canvas_centre_is_unchanged_when_the_layers_panel_collapses() {
        let ppp = 2.0;
        let window = 1440.0_f32;
        let shell = crate::shell::Shell::default();
        let width_with = shell.layout(window).canvas_width(window);
        let mut collapsed = shell.clone();
        collapsed.set_layers_collapsed(true);
        let width_without = collapsed.layout(window).canvas_width(window);
        assert_ne!(width_with, width_without);

        let pixels = |points: f32| (f64::from(points) * ppp) as u32;
        let mut view = View::fit([4096.0, 3072.0], [pixels(width_with), 1200], 64.0);
        view.zoom_at([300.0, 200.0], 2.5);
        let before = centre(&view);
        resize_keeping_centre(&mut view, [pixels(width_without), 1200]);
        let after = centre(&view);
        assert!((before[0] - after[0]).abs() < 1e-9 && (before[1] - after[1]).abs() < 1e-9);
        assert_eq!(view.size[0], pixels(width_without));
    }

    /// A Canvas view and an Editor holding one square Landmass in node-edit mode with its
    /// first node selected.
    fn harness() -> Harness<'static, (CanvasView, Editor)> {
        let mut editor = Editor::new(Map::new(Canvas::new(
            100,
            100,
            Scale::new(1.0, DistanceUnit::Miles),
        )));
        let layer = editor.new_layer().unwrap();
        let id = Id::new();
        editor
            .run(Command::CreateLandmass {
                id,
                parent: layer,
                index: None,
                geometry: MultiPolygon(vec![Polygon {
                    exterior: vec![[10.0, 10.0], [60.0, 10.0], [60.0, 60.0], [10.0, 60.0]],
                    holes: Vec::new(),
                }]),
                fill: None,
            })
            .unwrap();
        editor.enter_node_edit(id);
        editor.select_nodes(vec![NodeRef {
            part: 0,
            ring: 0,
            index: 0,
        }]);
        let mut harness = Harness::new_ui_state(
            |ui, (view, editor)| view.show(ui, editor),
            (CanvasView::new(None), editor),
        );
        harness.run();
        harness
    }

    #[test]
    fn the_banner_counts_the_selected_nodes() {
        let harness = harness();
        harness.get_by_label("Editing points · 1 of 4 selected");
    }

    #[test]
    fn done_leaves_node_edit() {
        let mut harness = harness();
        harness.get_by_label("Done").click();
        harness.run();
        assert!(harness.state().1.node_edit().is_none());
        assert!(harness.query_by_label("Done").is_none());
    }

    #[test]
    fn cancel_drops_a_gesture_then_leaves_node_edit() {
        let mut harness = harness();
        let gesture = harness.state_mut().1.begin_gesture("Move Points");
        harness.get_by_label("Cancel").click();
        harness.run();
        // The button rolled it back, so the token is stale.
        assert!(!harness.state_mut().1.cancel_gesture(gesture));
        assert!(!harness.state().1.gesture_open());
        assert!(harness.state().1.node_edit().is_some());
        harness.get_by_label("Cancel").click();
        harness.run();
        assert!(harness.state().1.node_edit().is_none());
    }

    #[test]
    fn overlays_do_not_change_what_an_export_draws() {
        // Selection, handles and the node-edit banner are egui shapes over the Canvas; the
        // scene an export renders is built from the Map alone and stays the same build.
        let mut harness = harness();
        let editing = harness.state_mut().1.scene_content();
        harness.state_mut().1.leave_node_edit();
        harness.state_mut().1.select(Vec::new());
        harness.run();
        let plain = harness.state_mut().1.scene_content();
        assert_eq!(format!("{editing:?}"), format!("{plain:?}"));
    }

    #[test]
    fn the_zoom_buttons_step_the_zoom() {
        let mut harness = harness();
        let before = harness.state().0.view.unwrap().zoom;
        harness.get_by_label("+").click();
        harness.run();
        let after = harness.state().0.view.unwrap().zoom;
        assert!((after / before - zoom::STEP).abs() < 1e-9);
        harness.get_by_label("−").click();
        harness.run();
        assert!((harness.state().0.view.unwrap().zoom - before).abs() < 1e-9);
    }
}
