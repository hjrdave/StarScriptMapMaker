//! The Shore preset row (issue 651): one button per preset, each a thumbnail of that preset on
//! a sample coast. Thumbnails are rendered by `ssmap_render` on first use and kept as textures,
//! one for each kind, preset and set of Map colours. Without a GPU (UI tests) the buttons show
//! the preset names alone.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use eframe::egui;
use eframe::egui_wgpu::RenderState;
use ssmap_core::{ShoreKind, ShoreStyle};
use ssmap_render::{
    GpuCapabilities, SHORE_THUMBNAIL_SIZE, ShoreSample, ShoreThumbnailer, Suitability,
};

use crate::theme;

/// Where the thumbnails are kept in egui's memory.
fn store_id() -> egui::Id {
    egui::Id::new("shore preset thumbnails")
}

/// The rendered thumbnails, and what renders more.
struct ShoreThumbnails {
    render_state: RenderState,
    /// Made on the first thumbnail asked for.
    thumbnailer: Option<ShoreThumbnailer>,
    /// By [`key`]. `None` where rendering failed, so it is not tried again every frame.
    made: HashMap<String, Option<egui::TextureHandle>>,
}

type Shared = Arc<Mutex<ShoreThumbnails>>;

/// Lets the preset row render thumbnails on `render_state`'s device.
pub(crate) fn install(ctx: &egui::Context, render_state: &RenderState) {
    let shared: Shared = Arc::new(Mutex::new(ShoreThumbnails {
        render_state: render_state.clone(),
        thumbnailer: None,
        made: HashMap::new(),
    }));
    ctx.data_mut(|d| d.insert_temp(store_id(), shared));
}

/// What a thumbnail depends on: the kind, the preset and the Map's colours.
fn key(kind: ShoreKind, name: &str, sample: &ShoreSample) -> String {
    format!("{kind:?}/{name}/{sample:?}")
}

/// The thumbnail of preset `name` for `kind` on a Map with `sample`'s colours, rendered now if
/// it is the first time. `None` without a GPU or when it could not be rendered.
fn thumbnail(
    ctx: &egui::Context,
    kind: ShoreKind,
    name: &str,
    style: &ShoreStyle,
    sample: &ShoreSample,
) -> Option<egui::TextureHandle> {
    let shared: Shared = ctx.data(|d| d.get_temp(store_id()))?;
    let mut store = shared.lock().unwrap_or_else(PoisonError::into_inner);
    let key = key(kind, name, sample);
    if let Some(made) = store.made.get(&key) {
        return made.clone();
    }
    let made = render(&mut store, ctx, kind, style, sample, &key);
    store.made.insert(key, made.clone());
    made
}

fn render(
    store: &mut ShoreThumbnails,
    ctx: &egui::Context,
    kind: ShoreKind,
    style: &ShoreStyle,
    sample: &ShoreSample,
    key: &str,
) -> Option<egui::TextureHandle> {
    let ShoreThumbnails {
        render_state,
        thumbnailer,
        ..
    } = store;
    let thumbnailer = match thumbnailer {
        Some(made) => made,
        None => {
            let capabilities = GpuCapabilities::query(&render_state.adapter, &render_state.device);
            if matches!(capabilities.suitability(), Suitability::BelowFloor(_)) {
                return None;
            }
            thumbnailer.insert(ShoreThumbnailer::new(&render_state.device, capabilities))
        }
    };
    let image = thumbnailer
        .render(
            &render_state.device,
            &render_state.queue,
            kind,
            style,
            sample,
        )
        .ok()?;
    let size = [image.width as usize, image.height as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, &image.pixels);
    Some(ctx.load_texture(key, pixels, egui::TextureOptions::LINEAR))
}

/// The preset every member matches, if they all match the same one.
pub(super) fn shared_preset(kind: ShoreKind, members: &[ShoreStyle]) -> Option<&'static str> {
    let mut names = members.iter().map(|s| s.preset_name(kind));
    names
        .next()
        .flatten()
        .filter(|first| names.all(|n| n == Some(*first)))
}

/// The preset the members were last given, if they all were given the same one.
fn last_picked(members: &[ShoreStyle]) -> Option<&str> {
    let first = members.first()?.preset.as_deref()?;
    members
        .iter()
        .all(|s| s.preset.as_deref() == Some(first))
        .then_some(first)
}

/// The "Preset" caption, then a row of thumbnail buttons, one per preset for `kind`. The one
/// every member matches is ringed in gold. With none, the caption says "Custom", and which
/// preset they were last given when that is known. Returns the preset clicked.
pub(super) fn preset_row(
    ui: &mut egui::Ui,
    kind: ShoreKind,
    members: &[ShoreStyle],
    sample: &ShoreSample,
) -> Option<&'static str> {
    let palette = theme::palette(ui.ctx());
    let current = shared_preset(kind, members);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Preset")
                .size(theme::CAPTION)
                .color(palette.text_secondary),
        );
        if current.is_none() {
            let custom = match last_picked(members) {
                Some(name) => format!("Custom, from {name}"),
                None => "Custom".to_owned(),
            };
            ui.label(
                egui::RichText::new(custom)
                    .size(theme::CAPTION)
                    .color(palette.gold_text),
            );
        }
    });
    let presets = ShoreStyle::presets(kind);
    // Three to a row, the width of the panel, whatever the number of presets.
    let gap = theme::SPACING[1];
    let width = ((ui.available_width() - 2.0 * gap) / 3.0).max(24.0);
    let [w, h] = SHORE_THUMBNAIL_SIZE.map(|n| n as f32);
    let size = egui::vec2(width, width * h / w);
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
        for (name, style) in presets {
            let texture = thumbnail(ui.ctx(), kind, name, &style, sample);
            if preset_button(ui, name, texture.as_ref(), size, current == Some(name)) {
                picked = Some(name);
            }
        }
    });
    picked
}

/// One preset: its thumbnail with its name under it, ringed in gold when `on`.
fn preset_button(
    ui: &mut egui::Ui,
    name: &str,
    texture: Option<&egui::TextureHandle>,
    size: egui::Vec2,
    on: bool,
) -> bool {
    let palette = theme::palette(ui.ctx());
    let caption = theme::CAPTION + 6.0;
    let (rect, response) =
        ui.allocate_exact_size(size + egui::vec2(0.0, caption), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, ui.is_enabled(), on, name)
    });
    let image = egui::Rect::from_min_size(rect.min, size);
    let radius = theme::RADIUS_FIELD;
    match texture {
        Some(texture) => {
            egui::Image::new((texture.id(), size))
                .corner_radius(radius)
                .paint_at(ui, image);
        }
        None => {
            ui.painter().rect_filled(image, radius, palette.bg_field);
        }
    }
    let ring = if on {
        egui::Stroke::new(2.0, palette.gold)
    } else if response.hovered() {
        egui::Stroke::new(1.0, palette.border_strong)
    } else {
        egui::Stroke::new(1.0, palette.border_subtle)
    };
    ui.painter()
        .rect_stroke(image, radius, ring, egui::StrokeKind::Inside);
    let text = if on {
        palette.gold_text
    } else {
        palette.text_secondary
    };
    ui.painter().text(
        egui::pos2(image.center().x, image.bottom() + caption / 2.0),
        egui::Align2::CENTER_CENTER,
        name,
        egui::FontId::proportional(theme::CAPTION),
        text,
    );
    response.on_hover_text(name).clicked()
}
