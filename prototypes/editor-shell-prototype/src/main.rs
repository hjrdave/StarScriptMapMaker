//! PROTOTYPE - throwaway code. Do not build on this.
//!
//! Question (map ticket "Prototype: Figma-style three-panel editor shell"):
//! does a Figma-style layout feel right for a map editor, and can egui + a custom
//! wgpu canvas produce it?
//!
//! Three structurally different layout variants, switchable with the floating
//! bottom bar or the Left/Right arrow keys, or at startup with `--variant B`:
//!   A  Classic Figma: toolbar on top, layer tree left, properties right.
//!   B  Floating (Figma UI3): canvas fills the window, panels float over it, tool dock at the bottom.
//!   C  Single sidebar: one left sidebar with Layers / Properties / Assets tabs, contextual toolbar above the selection.
//!
//! Everything lives in memory. The ocean is drawn by our own wgpu pipeline; land,
//! assets, labels and handles are drawn with egui's painter (also on the GPU).

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use eframe::egui_wgpu::{self, wgpu};
use std::time::Instant;

// ---------------------------------------------------------------- document model

const MAP_SIZE: Vec2 = Vec2::new(2400.0, 1500.0);

#[derive(Clone, Copy, PartialEq)]
enum AssetKind {
    Mountain,
    Tree,
    Town,
}

#[derive(Clone)]
enum Kind {
    Layer { children: Vec<Node>, expanded: bool },
    Land { center: Pos2, radius: f32, seed: u32 },
    Asset { pos: Pos2, scale: f32, kind: AssetKind },
    Label { pos: Pos2, text: String, size: f32 },
}

#[derive(Clone)]
struct Node {
    id: u64,
    name: String,
    visible: bool,
    locked: bool,
    opacity: f32,
    kind: Kind,
}

type Path = Vec<usize>;

struct Doc {
    root: Vec<Node>, // index 0 = top of the stack
    next_id: u64,
}

impl Doc {
    fn node(&self, path: &[usize]) -> Option<&Node> {
        let mut list = &self.root;
        let mut node = None;
        for &i in path {
            let n = list.get(i)?;
            node = Some(n);
            if let Kind::Layer { children, .. } = &n.kind {
                list = children;
            }
        }
        node
    }
    fn node_mut(&mut self, path: &[usize]) -> Option<&mut Node> {
        let (first, rest) = path.split_first()?;
        let mut n = self.root.get_mut(*first)?;
        for &i in rest {
            n = match &mut n.kind {
                Kind::Layer { children, .. } => children.get_mut(i)?,
                _ => return None,
            };
        }
        Some(n)
    }
    fn list_mut(&mut self, parent: &[usize]) -> Option<&mut Vec<Node>> {
        if parent.is_empty() {
            return Some(&mut self.root);
        }
        match &mut self.node_mut(parent)?.kind {
            Kind::Layer { children, .. } => Some(children),
            _ => None,
        }
    }
    fn new_node(&mut self, name: impl Into<String>, kind: Kind) -> Node {
        self.next_id += 1;
        Node { id: self.next_id, name: name.into(), visible: true, locked: false, opacity: 1.0, kind }
    }
    /// Move node at `from` to sit just above `to` in `to`'s parent (or into `to` if it is a layer and `into`).
    fn move_node(&mut self, from: &Path, to: &Path, into: bool) {
        if to.starts_with(from) {
            return; // no dropping a layer into itself
        }
        let (fi, fparent) = from.split_last().unwrap();
        let node = self.list_mut(fparent).unwrap().remove(*fi);
        // removing shifts later siblings in the same parent up by one
        let mut to = to.clone();
        if to.len() > fparent.len() && to[..fparent.len()] == *fparent && to[fparent.len()] > *fi {
            to[fparent.len()] -= 1;
        }
        if into {
            if let Some(list) = self.list_mut(&to) {
                list.insert(0, node);
                return;
            }
        }
        let (ti, tparent) = to.split_last().unwrap();
        let list = self.list_mut(tparent).unwrap();
        list.insert((*ti).min(list.len()), node);
    }
    fn count(&self) -> usize {
        fn c(l: &[Node]) -> usize {
            l.iter().map(|n| 1 + if let Kind::Layer { children, .. } = &n.kind { c(children) } else { 0 }).sum()
        }
        c(&self.root)
    }
}

fn sample_doc() -> Doc {
    let mut d = Doc { root: vec![], next_id: 0 };
    let l1 = d.new_node("Kingdom of Aster", Kind::Label { pos: Pos2::new(900.0, 600.0), text: "Kingdom of Aster".into(), size: 34.0 });
    let l2 = d.new_node("Greywater", Kind::Label { pos: Pos2::new(1500.0, 950.0), text: "Greywater".into(), size: 20.0 });
    let mut mountains = vec![];
    for i in 0..7 {
        let m = d.new_node(format!("Mountain {}", i + 1), Kind::Asset { pos: Pos2::new(700.0 + i as f32 * 55.0, 760.0 + (i % 2) as f32 * 30.0), scale: 1.0, kind: AssetKind::Mountain });
        mountains.push(m);
    }
    let mut trees = vec![];
    for i in 0..12 {
        let t = d.new_node(format!("Tree {}", i + 1), Kind::Asset { pos: Pos2::new(1250.0 + (i % 4) as f32 * 30.0, 650.0 + (i / 4) as f32 * 28.0), scale: 1.0, kind: AssetKind::Tree });
        trees.push(t);
    }
    let town = d.new_node("Greywater (town)", Kind::Asset { pos: Pos2::new(1500.0, 900.0), scale: 1.0, kind: AssetKind::Town });
    let forest_label = d.new_node("Whisperwood", Kind::Label { pos: Pos2::new(1300.0, 610.0), text: "Whisperwood".into(), size: 16.0 });
    let forest = d.new_node("Whisperwood (group)", Kind::Layer { children: { let mut v = vec![forest_label]; v.extend(trees); v }, expanded: false });
    let range = d.new_node("Spine Mountains", Kind::Layer { children: mountains, expanded: false });
    let land1 = d.new_node("Aster mainland", Kind::Land { center: Pos2::new(1000.0, 750.0), radius: 520.0, seed: 3 });
    let land2 = d.new_node("Isle of Mor", Kind::Land { center: Pos2::new(1900.0, 400.0), radius: 200.0, seed: 9 });
    let labels = d.new_node("Labels", Kind::Layer { children: vec![l1, l2], expanded: true });
    let assets = d.new_node("Assets", Kind::Layer { children: vec![town, forest, range], expanded: true });
    let land = d.new_node("Land", Kind::Layer { children: vec![land1, land2], expanded: true });
    d.root = vec![labels, assets, land];
    d
}

// ---------------------------------------------------------------- ocean wgpu callback

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OceanUniforms {
    size: [f32; 2],
    offset: [f32; 2],
    zoom: f32,
    time: f32,
    map_size: [f32; 2],
}

struct OceanResources {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
}

struct OceanCallback(OceanUniforms);

impl egui_wgpu::CallbackTrait for OceanCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _sd: &egui_wgpu::ScreenDescriptor,
        _enc: &mut wgpu::CommandEncoder,
        res: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let r: &OceanResources = res.get().unwrap();
        queue.write_buffer(&r.buffer, 0, bytemuck::bytes_of(&self.0));
        vec![]
    }
    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, res: &egui_wgpu::CallbackResources) {
        let r: &OceanResources = res.get().unwrap();
        pass.set_pipeline(&r.pipeline);
        pass.set_bind_group(0, &r.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn init_ocean(rs: &egui_wgpu::RenderState) {
    let device = &rs.device;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ocean"),
        source: wgpu::ShaderSource::Wgsl(include_str!("ocean.wgsl").into()),
    });
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ocean uniforms"),
        size: std::mem::size_of::<OceanUniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bgl,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ocean"),
        layout: Some(&layout),
        vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), buffers: &[], compilation_options: Default::default() },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(rs.target_format.into())],
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    rs.renderer.write().callback_resources.insert(OceanResources { pipeline, bind_group, buffer });
}

// ---------------------------------------------------------------- app state

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Move,
    Land,
    Mountain,
    Tree,
    Label,
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Layers,
    Properties,
    Assets,
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Move,
    Scale,
}

struct App {
    variant: usize,
    doc: Doc,
    selected: Option<Path>,
    tool: Tool,
    tab: Tab,
    offset: Vec2, // map units at the canvas top-left
    zoom: f32,    // screen px per map unit
    drag: Drag,
    start: Instant,
    frame_ms: f32,
    last_action: String,
}

const VARIANTS: [&str; 3] = ["A (Classic Figma)", "B (Floating panels)", "C (Single sidebar)"];

impl App {
    fn new(cc: &eframe::CreationContext, variant: usize) -> Self {
        if let Some(rs) = cc.wgpu_render_state.as_ref() {
            init_ocean(rs);
        }
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        Self {
            variant,
            doc: sample_doc(),
            selected: None,
            tool: Tool::Move,
            tab: Tab::Layers,
            offset: Vec2::new(100.0, 100.0),
            zoom: 0.5,
            drag: Drag::None,
            start: Instant::now(),
            frame_ms: 0.0,
            last_action: "-".into(),
        }
    }

    fn to_screen(&self, rect: Rect, p: Pos2) -> Pos2 {
        rect.min + (p.to_vec2() - self.offset) * self.zoom
    }
    fn to_map(&self, rect: Rect, p: Pos2) -> Pos2 {
        (self.offset + (p - rect.min) / self.zoom).to_pos2()
    }

    // ------------------------------------------------------------ shared widgets

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        for (t, label) in [(Tool::Move, "↖ Move"), (Tool::Land, "🗺 Land"), (Tool::Mountain, "🗻 Mountain"), (Tool::Tree, "🌲 Tree"), (Tool::Label, "🔤 Label")] {
            if ui.selectable_label(self.tool == t, label).clicked() {
                self.tool = t;
            }
        }
        ui.separator();
        if ui.button("Stress: +10,000 trees").clicked() {
            self.stress();
        }
    }

    fn stress(&mut self) {
        let mut kids = Vec::with_capacity(10_000);
        for i in 0..10_000u32 {
            let h = i.wrapping_mul(2654435761);
            let pos = Pos2::new(600.0 + (h % 900) as f32, 450.0 + ((h / 900) % 600) as f32);
            kids.push(self.doc.new_node(format!("Tree {i}"), Kind::Asset { pos, scale: 0.6, kind: AssetKind::Tree }));
        }
        let n = self.doc.new_node("Stress forest (10k)", Kind::Layer { children: kids, expanded: false });
        self.doc.root.insert(1, n);
        self.selected = None;
        self.last_action = "Added 10,000 trees".into();
    }

    fn layer_tree(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("LAYERS").small().weak());
        egui::ScrollArea::vertical().id_salt("layers").auto_shrink([false, false]).show(ui, |ui| {
            let mut action: Option<(Path, Path, bool)> = None;
            let roots = self.doc.root.len();
            for i in 0..roots {
                self.tree_row(ui, vec![i], 0, &mut action);
            }
            if let Some((from, to, into)) = action {
                self.doc.move_node(&from, &to, into);
                self.selected = None;
                self.last_action = "Reordered via drag-and-drop".into();
            }
        });
    }

    fn tree_row(&mut self, ui: &mut egui::Ui, path: Path, depth: usize, action: &mut Option<(Path, Path, bool)>) {
        let Some(node) = self.doc.node(&path) else { return };
        let (name, is_layer, expanded, n_children, visible, locked) = match &node.kind {
            Kind::Layer { children, expanded } => (node.name.clone(), true, *expanded, children.len(), node.visible, node.locked),
            _ => (node.name.clone(), false, false, 0, node.visible, node.locked),
        };
        let icon = match &node.kind {
            Kind::Layer { .. } => if expanded { "⏷ 📁" } else { "⏵ 📁" },
            Kind::Land { .. } => "  🗺",
            Kind::Asset { kind: AssetKind::Mountain, .. } => "  🗻",
            Kind::Asset { kind: AssetKind::Tree, .. } => "  🌲",
            Kind::Asset { kind: AssetKind::Town, .. } => "  🏠",
            Kind::Label { .. } => "  🔤",
        };
        let selected = self.selected.as_ref() == Some(&path);
        let row_id = egui::Id::new(("row", node.id));
        let (_, dropped) = ui.dnd_drop_zone::<Path, _>(egui::Frame::NONE, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * 14.0);
                let resp = ui.dnd_drag_source(row_id, path.clone(), |ui| {
                    ui.selectable_label(selected, format!("{icon}  {name}"))
                });
                if resp.inner.clicked() {
                    self.selected = Some(path.clone());
                    self.tab = Tab::Properties.max_tab(self.tab);
                }
                if is_layer && resp.inner.double_clicked() {
                    if let Some(Node { kind: Kind::Layer { expanded, .. }, .. }) = self.doc.node_mut(&path) {
                        *expanded = !*expanded;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(if locked { "🔒" } else { "🔓" }).on_hover_text("Lock").clicked() {
                        if let Some(n) = self.doc.node_mut(&path) { n.locked = !n.locked; }
                    }
                    if ui.small_button(if visible { "👁" } else { "–" }).on_hover_text("Visibility").clicked() {
                        if let Some(n) = self.doc.node_mut(&path) { n.visible = !n.visible; }
                    }
                    if is_layer {
                        ui.label(egui::RichText::new(format!("{n_children}")).small().weak());
                    }
                });
            });
        });
        if let Some(from) = dropped {
            // dropping on a layer row puts it inside that layer, otherwise above the row
            *action = Some(((*from).clone(), path.clone(), is_layer));
        }
        if is_layer && expanded {
            if n_children > 200 {
                // virtualised: only the visible rows of a huge layer are laid out
                let row_h = ui.spacing().interact_size.y;
                egui::ScrollArea::vertical().id_salt(("big", path.clone())).max_height(240.0).show_rows(ui, row_h, n_children, |ui, range| {
                    for c in range {
                        let mut p = path.clone();
                        p.push(c);
                        self.tree_row(ui, p, depth + 1, action);
                    }
                });
            } else {
                for c in 0..n_children {
                    let mut p = path.clone();
                    p.push(c);
                    self.tree_row(ui, p, depth + 1, action);
                }
            }
        }
    }

    fn properties(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("PROPERTIES").small().weak());
        let Some(path) = self.selected.clone() else {
            ui.weak("Nothing selected. Click an object on the canvas or a row in the layer tree.");
            ui.separator();
            ui.label(format!("Map: {} × {} units, 1 unit = 1 km", MAP_SIZE.x, MAP_SIZE.y));
            return;
        };
        let Some(n) = self.doc.node_mut(&path) else { return };
        egui::Grid::new("props").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut n.name);
            ui.end_row();
            ui.label("Visible");
            ui.checkbox(&mut n.visible, "");
            ui.end_row();
            ui.label("Locked");
            ui.checkbox(&mut n.locked, "");
            ui.end_row();
            ui.label("Opacity");
            ui.add(egui::Slider::new(&mut n.opacity, 0.0..=1.0));
            ui.end_row();
            match &mut n.kind {
                Kind::Layer { children, .. } => {
                    ui.label("Children");
                    ui.label(children.len().to_string());
                    ui.end_row();
                }
                Kind::Land { center, radius, seed } => {
                    ui.label("X / Y");
                    ui.horizontal(|ui| { ui.add(egui::DragValue::new(&mut center.x)); ui.add(egui::DragValue::new(&mut center.y)); });
                    ui.end_row();
                    ui.label("Size");
                    ui.add(egui::DragValue::new(radius).range(20.0..=2000.0));
                    ui.end_row();
                    ui.label("Shape seed");
                    ui.add(egui::DragValue::new(seed));
                    ui.end_row();
                }
                Kind::Asset { pos, scale, .. } => {
                    ui.label("X / Y");
                    ui.horizontal(|ui| { ui.add(egui::DragValue::new(&mut pos.x)); ui.add(egui::DragValue::new(&mut pos.y)); });
                    ui.end_row();
                    ui.label("Scale");
                    ui.add(egui::DragValue::new(scale).speed(0.02).range(0.1..=10.0));
                    ui.end_row();
                }
                Kind::Label { pos, text, size } => {
                    ui.label("Text");
                    ui.text_edit_singleline(text);
                    ui.end_row();
                    ui.label("X / Y");
                    ui.horizontal(|ui| { ui.add(egui::DragValue::new(&mut pos.x)); ui.add(egui::DragValue::new(&mut pos.y)); });
                    ui.end_row();
                    ui.label("Font size");
                    ui.add(egui::DragValue::new(size).range(6.0..=200.0));
                    ui.end_row();
                }
            }
        });
    }

    fn assets_palette(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("ASSETS").small().weak());
        ui.weak("Pick a brush, then click the canvas.");
        for (t, label) in [(Tool::Mountain, "🗻  Mountain"), (Tool::Tree, "🌲  Tree"), (Tool::Land, "🗺  Landmass"), (Tool::Label, "🔤  Label")] {
            if ui.selectable_label(self.tool == t, label).clicked() {
                self.tool = t;
            }
        }
    }

    fn status(&self) -> String {
        let sel = match &self.selected {
            Some(p) => format!("{} {:?}", self.doc.node(p).map(|n| n.name.as_str()).unwrap_or("?"), p),
            None => "none".into(),
        };
        format!(
            "{}  |  zoom {:.0}%  |  objects {}  |  selected: {}  |  last: {}  |  frame {:.1} ms",
            VARIANTS[self.variant], self.zoom * 100.0, self.doc.count(), sel, self.last_action, self.frame_ms
        )
    }

    // ------------------------------------------------------------ canvas

    fn canvas(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let resp = ui.interact(rect, ui.id().with("canvas"), Sense::click_and_drag());
        let painter = ui.painter_at(rect);

        // our own wgpu pipeline: ocean backdrop
        let ppp = ui.ctx().pixels_per_point();
        painter.add(egui_wgpu::Callback::new_paint_callback(
            rect,
            OceanCallback(OceanUniforms {
                size: [rect.width() * ppp, rect.height() * ppp],
                offset: self.offset.into(),
                zoom: self.zoom * ppp,
                time: self.start.elapsed().as_secs_f32(),
                map_size: MAP_SIZE.into(),
            }),
        ));

        // zoom around the cursor, pan with middle/right drag or space+drag
        if let Some(hover) = resp.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let before = self.to_map(rect, hover);
                self.zoom = (self.zoom * (1.0 + scroll * 0.002)).clamp(0.05, 8.0);
                let after = self.to_map(rect, hover);
                self.offset += before - after;
            }
        }
        let space = ui.input(|i| i.key_down(egui::Key::Space));
        if resp.dragged_by(egui::PointerButton::Middle) || resp.dragged_by(egui::PointerButton::Secondary) || (space && resp.dragged()) {
            self.offset -= resp.drag_delta() / self.zoom;
        }

        // draw the document, bottom of the stack first
        let mut hits: Vec<(Path, Rect)> = vec![];
        for i in (0..self.doc.root.len()).rev() {
            self.draw_node(&painter, rect, &[i], 1.0, &mut hits);
        }

        // selection + handles
        let sel_rect = self.selected.as_ref().and_then(|p| hits.iter().find(|(h, _)| h == p).map(|(_, r)| *r)
            .or_else(|| self.group_bounds(p, &hits)))
            .map(|r| r.expand(6.0)); // handles sit outside the object, so small objects stay draggable
        if let Some(r) = sel_rect {
            painter.rect_stroke(r, 0.0, Stroke::new(1.5, Color32::from_rgb(24, 160, 251)), egui::StrokeKind::Outside);
            for c in [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom(), r.center_top(), r.center_bottom(), r.left_center(), r.right_center()] {
                let h = Rect::from_center_size(c, Vec2::splat(8.0));
                painter.rect_filled(h, 1.0, Color32::WHITE);
                painter.rect_stroke(h, 1.0, Stroke::new(1.0, Color32::from_rgb(24, 160, 251)), egui::StrokeKind::Inside);
            }
        }

        if space || resp.dragged_by(egui::PointerButton::Middle) {
            return;
        }

        // clicks: place with brush tools, select/move/scale with Move
        if resp.drag_started_by(egui::PointerButton::Primary) || resp.clicked() {
            let p = resp.interact_pointer_pos().unwrap_or_default();
            if self.tool == Tool::Move {
                let on_handle = sel_rect.is_some_and(|r| [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()].iter().any(|c| c.distance(p) < 6.0));
                if on_handle {
                    self.drag = Drag::Scale;
                } else {
                    // topmost hit = last drawn
                    self.selected = hits.iter().rev().find(|(_, r)| r.contains(p)).map(|(h, _)| h.clone());
                    self.drag = if self.selected.is_some() { Drag::Move } else { Drag::None };
                    if self.selected.is_some() {
                        self.tab = Tab::Properties.max_tab(self.tab);
                    }
                }
            } else if resp.clicked() || resp.drag_started() {
                self.place(self.to_map(rect, p));
            }
        }
        if resp.dragged_by(egui::PointerButton::Primary) && self.tool == Tool::Move {
            let d = resp.drag_delta() / self.zoom;
            if let (Some(path), Some(r)) = (self.selected.clone(), sel_rect) {
                let locked = self.doc.node(&path).is_some_and(|n| n.locked);
                if !locked {
                    match self.drag {
                        Drag::Move => { translate(self.doc.node_mut(&path).unwrap(), d); self.last_action = "Moved".into(); }
                        Drag::Scale => {
                            let p = resp.interact_pointer_pos().unwrap_or_default();
                            // proportional: ratio of pointer distance from the centre now vs last frame
                            let prev = p - resp.drag_delta();
                            let factor = (p.distance(r.center()) / prev.distance(r.center()).max(1.0)).clamp(0.5, 2.0);
                            scale(self.doc.node_mut(&path).unwrap(), factor);
                            self.last_action = "Scaled".into();
                        }
                        Drag::None => {}
                    }
                }
            }
        }
        if resp.drag_stopped() {
            self.drag = Drag::None;
        }
    }

    fn group_bounds(&self, p: &Path, hits: &[(Path, Rect)]) -> Option<Rect> {
        let mut r: Option<Rect> = None;
        for (h, hr) in hits {
            if h.len() > p.len() && h.starts_with(p) {
                r = Some(r.map_or(*hr, |r| r.union(*hr)));
            }
        }
        r
    }

    fn place(&mut self, at: Pos2) {
        let (target_layer, node) = match self.tool {
            Tool::Land => ("Land", self.doc.new_node("New landmass", Kind::Land { center: at, radius: 150.0, seed: (at.x as u32) ^ (at.y as u32) })),
            Tool::Mountain => ("Assets", self.doc.new_node("Mountain", Kind::Asset { pos: at, scale: 1.0, kind: AssetKind::Mountain })),
            Tool::Tree => ("Assets", self.doc.new_node("Tree", Kind::Asset { pos: at, scale: 1.0, kind: AssetKind::Tree })),
            Tool::Label => ("Labels", self.doc.new_node("New label", Kind::Label { pos: at, text: "New label".into(), size: 18.0 })),
            Tool::Move => return,
        };
        // place into the selected layer if one is selected, otherwise the default layer for this kind
        let target = match &self.selected {
            Some(p) if matches!(self.doc.node(p).map(|n| &n.kind), Some(Kind::Layer { .. })) => p.clone(),
            _ => vec![self.doc.root.iter().position(|n| n.name == target_layer).unwrap_or(0)],
        };
        if let Some(list) = self.doc.list_mut(&target) {
            list.insert(0, node);
            let mut p = target.clone();
            p.push(0);
            self.last_action = format!("Placed into {:?}", target);
            self.selected = Some(p);
        }
    }

    fn draw_node(&self, painter: &egui::Painter, rect: Rect, path: &[usize], parent_alpha: f32, hits: &mut Vec<(Path, Rect)>) {
        let Some(n) = self.doc.node(path) else { return };
        if !n.visible {
            return;
        }
        let a = parent_alpha * n.opacity;
        let col = |c: Color32| c.gamma_multiply(a);
        match &n.kind {
            Kind::Layer { children, .. } => {
                for i in (0..children.len()).rev() {
                    let mut p = path.to_vec();
                    p.push(i);
                    self.draw_node(painter, rect, &p, a, hits);
                }
            }
            Kind::Land { center, radius, seed } => {
                let pts: Vec<Pos2> = (0..96)
                    .map(|i| {
                        let t = i as f32 / 96.0 * std::f32::consts::TAU;
                        let s = *seed as f32;
                        let wobble = 1.0 + 0.18 * (3.0 * t + s).sin() + 0.09 * (7.0 * t + s * 1.7).sin() + 0.05 * (13.0 * t + s * 2.3).sin();
                        self.to_screen(rect, *center + Vec2::angled(t) * *radius * wobble)
                    })
                    .collect();
                // egui only fills convex polygons; a fan from the centre is fine for the prototype
                let c = self.to_screen(rect, *center);
                let mut mesh = egui::Mesh::default();
                mesh.colored_vertex(c, col(Color32::from_rgb(214, 196, 150)));
                for p in &pts {
                    mesh.colored_vertex(*p, col(Color32::from_rgb(222, 205, 160)));
                }
                for i in 0..pts.len() as u32 {
                    mesh.add_triangle(0, 1 + i, 1 + (i + 1) % pts.len() as u32);
                }
                painter.add(mesh);
                let mut closed = pts.clone();
                closed.push(pts[0]);
                painter.line(closed.clone(), Stroke::new(3.0 * self.zoom.max(0.4), col(Color32::from_rgb(70, 55, 40))));
                hits.push((path.to_vec(), Rect::from_center_size(c, Vec2::splat(*radius * 2.0 * self.zoom))));
            }
            Kind::Asset { pos, scale, kind } => {
                let p = self.to_screen(rect, *pos);
                let s = 18.0 * scale * self.zoom.max(0.3);
                if !rect.expand(s).contains(p) {
                    return; // cull off-screen assets
                }
                match kind {
                    AssetKind::Mountain => {
                        let pts = vec![p + Vec2::new(-s, s * 0.6), p + Vec2::new(0.0, -s), p + Vec2::new(s, s * 0.6)];
                        painter.add(egui::Shape::convex_polygon(pts, col(Color32::from_rgb(150, 140, 125)), Stroke::new(1.5, col(Color32::from_rgb(60, 50, 40)))));
                    }
                    AssetKind::Tree => {
                        painter.circle(p, s * 0.5, col(Color32::from_rgb(70, 120, 60)), Stroke::new(1.0, col(Color32::from_rgb(30, 50, 25))));
                    }
                    AssetKind::Town => {
                        painter.rect(Rect::from_center_size(p, Vec2::splat(s)), 2.0, col(Color32::from_rgb(170, 70, 60)), Stroke::new(1.5, col(Color32::BLACK)), egui::StrokeKind::Inside);
                    }
                }
                hits.push((path.to_vec(), Rect::from_center_size(p, Vec2::splat(s * 2.0))));
            }
            Kind::Label { pos, text, size } => {
                let p = self.to_screen(rect, *pos);
                let galley = painter.layout_no_wrap(text.clone(), egui::FontId::proportional((size * self.zoom).max(6.0)), col(Color32::from_rgb(40, 30, 20)));
                let r = Rect::from_center_size(p, galley.size());
                painter.galley(r.min, galley, Color32::BLACK);
                hits.push((path.to_vec(), r));
            }
        }
    }

    // ------------------------------------------------------------ variants

    fn variant_a(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("a_top").show(ui, |ui| ui.horizontal(|ui| self.toolbar(ui)));
        egui::Panel::bottom("a_status").show(ui, |ui| ui.small(self.status()));
        egui::Panel::left("a_layers").resizable(true).default_size(260.0).show(ui, |ui| self.layer_tree(ui));
        egui::Panel::right("a_props").resizable(true).default_size(260.0).show(ui, |ui| self.properties(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let r = ui.max_rect();
            self.canvas(ui, r);
        });
    }

    fn variant_b(&mut self, ui: &mut egui::Ui) {
        let full = ui.max_rect();
        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas(ui, full));
        let ctx = ui.ctx().clone();
        let panel = |id: &str, pos: Pos2| {
            egui::Window::new(id.to_owned()).id(egui::Id::new(id)).title_bar(false).resizable(true).default_pos(pos).default_size([250.0, 420.0])
        };
        panel("b_layers", full.min + Vec2::new(12.0, 12.0)).show(&ctx, |ui| self.layer_tree(ui));
        panel("b_props", Pos2::new(full.max.x - 274.0, full.min.y + 12.0)).show(&ctx, |ui| self.properties(ui));
        egui::Area::new(egui::Id::new("b_dock")).anchor(egui::Align2::CENTER_BOTTOM, [0.0, -64.0]).show(&ctx, |ui| {
            egui::Frame::popup(ui.style()).corner_radius(12.0).show(ui, |ui| ui.horizontal(|ui| self.toolbar(ui)));
        });
        egui::Area::new(egui::Id::new("b_status")).anchor(egui::Align2::LEFT_BOTTOM, [12.0, -12.0]).show(&ctx, |ui| ui.small(self.status()));
    }

    fn variant_c(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("c_status").show(ui, |ui| ui.small(self.status()));
        egui::Panel::left("c_side").resizable(true).default_size(300.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                for (t, l) in [(Tab::Layers, "Layers"), (Tab::Properties, "Properties"), (Tab::Assets, "Assets")] {
                    if ui.selectable_label(self.tab == t, l).clicked() { self.tab = t; }
                }
            });
            ui.separator();
            match self.tab {
                Tab::Layers => self.layer_tree(ui),
                Tab::Properties => self.properties(ui),
                Tab::Assets => self.assets_palette(ui),
            }
        });
        let canvas_rect = egui::CentralPanel::no_frame().show(ui, |ui| {
            let r = ui.max_rect();
            self.canvas(ui, r);
            r
        }).inner;
        // contextual toolbar floating above the selection, or a minimal tool strip when nothing is selected
        let ctx = ui.ctx().clone();
        let anchor = self.selected.as_ref().and_then(|p| self.doc.node(p)).and_then(|n| match &n.kind {
            Kind::Land { center, radius, .. } => Some(*center - Vec2::new(0.0, *radius)),
            Kind::Asset { pos, .. } | Kind::Label { pos, .. } => Some(*pos - Vec2::new(0.0, 30.0)),
            _ => None,
        });
        let at = match anchor {
            Some(p) => self.to_screen(canvas_rect, p) - Vec2::new(90.0, 48.0),
            None => canvas_rect.center_top() + Vec2::new(-150.0, 10.0),
        };
        egui::Area::new(egui::Id::new("c_ctx")).fixed_pos(at).show(&ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| ui.horizontal(|ui| {
                if let Some(p) = self.selected.clone() {
                    if let Some(n) = self.doc.node_mut(&p) {
                        ui.label(egui::RichText::new(&n.name).strong());
                        if ui.small_button(if n.visible { "Hide" } else { "Show" }).clicked() { n.visible = !n.visible; }
                        if ui.small_button(if n.locked { "Unlock" } else { "Lock" }).clicked() { n.locked = !n.locked; }
                        ui.add(egui::Slider::new(&mut n.opacity, 0.0..=1.0).show_value(false));
                    }
                } else {
                    self.toolbar(ui);
                }
            }));
        });
    }

    fn switcher(&mut self, ctx: &egui::Context) {
        if !ctx.egui_wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) { self.variant = (self.variant + 1) % VARIANTS.len(); }
            if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) { self.variant = (self.variant + VARIANTS.len() - 1) % VARIANTS.len(); }
        }
        egui::Area::new(egui::Id::new("prototype_switcher")).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_BOTTOM, [0.0, -16.0]).show(ctx, |ui| {
            egui::Frame::NONE.fill(Color32::from_rgb(255, 214, 0)).corner_radius(20.0).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.visuals_mut().override_text_color = Some(Color32::BLACK);
                    if ui.button("◀").clicked() { self.variant = (self.variant + VARIANTS.len() - 1) % VARIANTS.len(); }
                    ui.label(egui::RichText::new(format!("PROTOTYPE  {}", VARIANTS[self.variant])).strong());
                    if ui.button("▶").clicked() { self.variant = (self.variant + 1) % VARIANTS.len(); }
                });
            });
        });
    }
}

impl Tab {
    /// In variant C, selecting something jumps to Properties unless you're in Assets.
    fn max_tab(self, current: Tab) -> Tab {
        if current == Tab::Assets { current } else { self }
    }
}

fn translate(n: &mut Node, d: Vec2) {
    match &mut n.kind {
        Kind::Layer { children, .. } => children.iter_mut().for_each(|c| translate(c, d)),
        Kind::Land { center, .. } => *center += d,
        Kind::Asset { pos, .. } | Kind::Label { pos, .. } => *pos += d,
    }
}

fn scale(n: &mut Node, f: f32) {
    match &mut n.kind {
        Kind::Layer { children, .. } => children.iter_mut().for_each(|c| scale(c, f)),
        Kind::Land { radius, .. } => *radius *= f,
        Kind::Asset { scale, .. } => *scale *= f,
        Kind::Label { size, .. } => *size *= f,
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let t = Instant::now();
        match self.variant {
            0 => self.variant_a(ui),
            1 => self.variant_b(ui),
            _ => self.variant_c(ui),
        }
        let ctx = ui.ctx().clone();
        self.switcher(&ctx);
        self.frame_ms = self.frame_ms * 0.9 + t.elapsed().as_secs_f32() * 1000.0 * 0.1;
        ctx.request_repaint(); // animated ocean
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let variant = args.iter().position(|a| a == "--variant").and_then(|i| args.get(i + 1)).map(|v| match v.to_ascii_uppercase().as_str() {
        "B" => 1,
        "C" => 2,
        _ => 0,
    }).unwrap_or(0);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1400.0, 860.0]).with_title("PROTOTYPE - Map editor shell"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native("map-editor-shell-prototype", options, Box::new(move |cc| Ok(Box::new(App::new(cc, variant)))))
}
