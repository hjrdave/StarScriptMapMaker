//! The UI icon set (docs/design/ui-handoff/HANDOFF.md, Icons): one SVG per icon on a 16 pt
//! grid in `apps/starscript/icons/`, rasterised with resvg into a 1x atlas (16 px cells, 1 px
//! strokes) and a 2x atlas (32 px cells, 1.5 pt = 3 px strokes). Glyphs are white on
//! transparent and tinted when drawn.
//!
//! The atlases are built at startup, on first use of each scale, rather than in a `build.rs`:
//! about 40 tiny paths rasterise in well under a millisecond, and a build script would need
//! resvg as a host build-dependency for no gain.
//!
//! Where a 1.5 pt stroke blurs at 1x, put a hand-hinted `name@1x.svg` next to the icon and
//! list it in `TABLE`; it replaces the 1x raster. Without one the 1x raster is the same
//! SVG with its 1.5 stroke thinned to 1 px.

use std::sync::Arc;

use eframe::egui;
use ssmap_render::raster::Art;

/// Side of an icon's grid in points; the default size an icon is drawn at.
pub const GRID: f32 = 16.0;

macro_rules! icons {
    ($($variant:ident, $label:literal, $file:literal;)*) => {
        /// Every icon in the set. Names follow the `icons` array in `Foundations.html`; the
        /// last few are icons the other mockups draw inline.
        #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
        pub enum Icon { $($variant,)* }

        impl Icon {
            /// Every icon, in atlas order.
            pub const ALL: &'static [Icon] = &[$(Icon::$variant,)*];

            /// The name the mockups give the icon.
            pub fn name(self) -> &'static str {
                match self { $(Icon::$variant => $label,)* }
            }

            fn svg(self) -> &'static str {
                match self {
                    $(Icon::$variant => include_str!(concat!("../../icons/", $file, ".svg")),)*
                }
            }
        }
    };
}

icons! {
    Select, "Select", "select";
    Pan, "Pan", "pan";
    LandBrush, "Land Brush", "land-brush";
    WaterBrush, "Water Brush", "water-brush";
    Lasso, "Lasso", "lasso";
    Label, "Label", "label";
    Stamp, "Stamp", "stamp";
    StampEraser, "Stamp Eraser", "stamp-eraser";
    ExportRegion, "Export Region", "export-region";
    Landmass, "Landmass", "landmass";
    Lake, "Lake", "lake";
    LabelKind, "Label kind", "label-kind";
    LayerGroup, "Layer / Group", "layer-group";
    History, "History", "history";
    Visible, "Visible", "visible";
    Hidden, "Hidden", "hidden";
    Lock, "Lock", "lock";
    Clip, "Clip", "clip";
    DepthSort, "Depth-sort", "depth-sort";
    Inherited, "Inherited", "inherited";
    Reset, "Reset", "reset";
    ReRoll, "Re-roll", "re-roll";
    Attribution, "Attribution", "attribution";
    Nodes, "Points", "nodes";
    Union, "Union", "union";
    Subtract, "Subtract", "subtract";
    SplitApart, "Split apart", "split-apart";
    Search, "Search", "search";
    Add, "Add", "add";
    Undo, "Undo", "undo";
    Export, "Export", "export";
    Warning, "Warning", "warning";
    More, "More", "more";
    Home, "Home", "home";
    Close, "Close", "close";
    Check, "Check", "check";
    ChevronRight, "Chevron right", "chevron-right";
    ChevronDown, "Chevron down", "chevron-down";
    CollapseLeft, "Collapse left", "collapse-left";
    CollapseRight, "Collapse right", "collapse-right";
    CollapseDouble, "Collapse double", "collapse-double";
    Layer, "Layer", "layer";
    Unlock, "Unlock", "unlock";
    Shape, "Shape", "shape";
    ActiveLayer, "Active layer", "active-layer";
    LockFilled, "Lock filled", "lock-filled";
    LockDashed, "Lock dashed", "lock-dashed";
    Link, "Link", "link";
}

/// Hand-hinted 1x replacements, by icon. None yet: the thinned 1 px stroke is crisp enough
/// until someone checks the icons in a real 1x window.
const ONE_X_OVERRIDES: &[(Icon, &str)] = &[];

impl Icon {
    fn one_x_override(self) -> Option<&'static str> {
        ONE_X_OVERRIDES
            .iter()
            .find(|(icon, _)| *icon == self)
            .map(|(_, svg)| *svg)
    }
}

/// Which atlas to draw from.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Scale {
    /// 16 px cells.
    X1,
    /// 32 px cells.
    X2,
}

impl Scale {
    /// 1x only at 1 pixel per point (or less); 1.5 and up use 2x as the design asks, and so
    /// do the scales between, such as 125%, downsampling the 2x atlas linearly. The 1x atlas
    /// is never scaled up.
    pub fn for_pixels_per_point(pixels_per_point: f32) -> Self {
        if pixels_per_point > 1.0 + 1e-3 {
            Scale::X2
        } else {
            Scale::X1
        }
    }

    /// Side of one atlas cell in pixels.
    pub fn cell(self) -> u32 {
        match self {
            Scale::X1 => 16,
            Scale::X2 => 32,
        }
    }
}

const COLUMNS: u32 = 8;

/// All icons rasterised at one scale: premultiplied RGBA8, white glyphs on transparent.
pub struct Atlas {
    scale: Scale,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Atlas {
    pub fn build(scale: Scale) -> Self {
        let cell = scale.cell();
        let rows = (Icon::ALL.len() as u32).div_ceil(COLUMNS);
        let (width, height) = (COLUMNS * cell, rows * cell);
        let mut pixels = vec![0; (width * height * 4) as usize];
        for (index, &icon) in Icon::ALL.iter().enumerate() {
            let raster = rasterise(icon, scale);
            let (x0, y0) = (
                (index as u32 % COLUMNS) * cell,
                (index as u32 / COLUMNS) * cell,
            );
            for row in 0..cell {
                let from = (row * cell * 4) as usize;
                let to = (((y0 + row) * width + x0) * 4) as usize;
                let len = (cell * 4) as usize;
                pixels[to..to + len].copy_from_slice(&raster.pixels[from..from + len]);
            }
        }
        Self {
            scale,
            width,
            height,
            pixels,
        }
    }

    /// The icon's cell as `[x, y]` of its top-left pixel; the cell is `scale.cell()` square.
    pub fn cell_origin(&self, icon: Icon) -> [u32; 2] {
        let index = Icon::ALL
            .iter()
            .position(|&i| i == icon)
            .expect("every icon is in ALL") as u32;
        let cell = self.scale.cell();
        [(index % COLUMNS) * cell, (index / COLUMNS) * cell]
    }

    /// The icon's cell as premultiplied RGBA8 rows.
    #[cfg(test)]
    pub fn cell_pixels(&self, icon: Icon) -> Vec<u8> {
        let cell = self.scale.cell();
        let [x0, y0] = self.cell_origin(icon);
        let mut out = Vec::with_capacity((cell * cell * 4) as usize);
        for row in 0..cell {
            let start = (((y0 + row) * self.width + x0) * 4) as usize;
            out.extend_from_slice(&self.pixels[start..start + (cell * 4) as usize]);
        }
        out
    }

    /// The icon's cell in texture coordinates.
    pub fn uv(&self, icon: Icon) -> egui::Rect {
        let [x, y] = self.cell_origin(icon);
        let cell = self.scale.cell();
        let (w, h) = (self.width as f32, self.height as f32);
        egui::Rect::from_min_max(
            egui::pos2(x as f32 / w, y as f32 / h),
            egui::pos2((x + cell) as f32 / w, (y + cell) as f32 / h),
        )
    }

    fn color_image(&self) -> egui::ColorImage {
        egui::ColorImage::from_rgba_premultiplied(
            [self.width as usize, self.height as usize],
            &self.pixels,
        )
    }
}

fn rasterise(icon: Icon, scale: Scale) -> ssmap_render::raster::Raster {
    let svg = match scale {
        Scale::X1 => match icon.one_x_override() {
            Some(hinted) => hinted.to_owned(),
            // 1.5 pt strokes become 1 px; the dot icons' heavier strokes stay as drawn.
            None => icon
                .svg()
                .replace("stroke-width=\"1.5\"", "stroke-width=\"1\""),
        },
        Scale::X2 => icon.svg().to_owned(),
    };
    Art::from_svg(svg.as_bytes())
        .expect("embedded icons are valid SVG")
        .rasterise(scale.cell())
}

/// The atlas textures, uploaded once per egui context and scale.
#[derive(Clone)]
struct Texture(Arc<(egui::TextureHandle, Atlas)>);

fn texture(ctx: &egui::Context, scale: Scale) -> Texture {
    let id = egui::Id::new(("ssmap-icon-atlas", scale));
    if let Some(texture) = ctx.data(|d| d.get_temp::<Texture>(id)) {
        return texture;
    }
    let atlas = Atlas::build(scale);
    let handle = ctx.load_texture(
        format!("icon-atlas-{scale:?}"),
        atlas.color_image(),
        egui::TextureOptions::LINEAR,
    );
    let texture = Texture(Arc::new((handle, atlas)));
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    texture
}

/// Centres a square of `size` points in `container`, rounded to whole physical pixels so the
/// glyph never straddles a pixel boundary.
pub fn snapped_rect(container: egui::Rect, size: f32, pixels_per_point: f32) -> egui::Rect {
    let pixels = (size * pixels_per_point).round().max(1.0);
    let min = container.center().to_vec2() * pixels_per_point - egui::Vec2::splat(pixels / 2.0);
    let min = egui::pos2(min.x.round(), min.y.round()) / pixels_per_point;
    egui::Rect::from_min_size(min, egui::Vec2::splat(pixels / pixels_per_point))
}

/// Paints `icon` centred in `container` at `size` points (use [`GRID`] for the native size),
/// tinted with `tint`.
pub fn paint(ui: &egui::Ui, container: egui::Rect, icon: Icon, size: f32, tint: egui::Color32) {
    paint_flipped(ui, container, icon, size, tint, false);
}

/// Like [`paint`], optionally mirrored left to right (a left chevron from the right one).
pub fn paint_flipped(
    ui: &egui::Ui,
    container: egui::Rect,
    icon: Icon,
    size: f32,
    tint: egui::Color32,
    flip: bool,
) {
    let ppp = ui.ctx().pixels_per_point();
    let texture = texture(ui.ctx(), Scale::for_pixels_per_point(ppp));
    let (handle, atlas) = &*texture.0;
    let mut uv = atlas.uv(icon);
    if flip {
        std::mem::swap(&mut uv.min.x, &mut uv.max.x);
    }
    ui.painter()
        .image(handle.id(), snapped_rect(container, size, ppp), uv, tint);
}

/// A square button showing only `icon`. `label` is required: it is the accessible name (the
/// mockups' `aria-label`, such as "Group selection (⌘G)") and the hover text.
///
/// The glyph is tinted from the current egui visuals until the Phase 2 theme (#157) supplies
/// palette colours.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, label: &str) -> egui::Response {
    icon_button_with(ui, icon, label, 24.0, GRID, None)
}

/// [`icon_button`] with the glyph mirrored left to right when `flip` is set.
pub fn icon_button_flipped(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    flip: bool,
) -> egui::Response {
    icon_button_inner(ui, icon, label, 24.0, GRID, None, flip)
}

/// [`icon_button`] with its button `side` and glyph `size` in points, and optionally a `tint`
/// in place of the visuals' text colour.
pub fn icon_button_with(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    side: f32,
    size: f32,
    tint: Option<egui::Color32>,
) -> egui::Response {
    icon_button_inner(ui, icon, label, side, size, tint, false)
}

fn icon_button_inner(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    side: f32,
    size: f32,
    tint: Option<egui::Color32>,
    flip: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(side), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        if response.hovered() || response.has_focus() {
            ui.painter()
                .rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
        }
        paint_flipped(
            ui,
            rect,
            icon,
            size,
            tint.unwrap_or(visuals.fg_stroke.color),
            flip,
        );
    }
    response.on_hover_text(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_is_chosen_by_pixels_per_point() {
        assert_eq!(Scale::for_pixels_per_point(1.0), Scale::X1);
        assert_eq!(Scale::for_pixels_per_point(1.25), Scale::X2);
        assert_eq!(Scale::for_pixels_per_point(1.5), Scale::X2);
        assert_eq!(Scale::for_pixels_per_point(2.0), Scale::X2);
        assert_eq!(Scale::X1.cell(), 16);
        assert_eq!(Scale::X2.cell(), 32);
    }

    #[test]
    fn every_icon_has_a_non_empty_cell_at_both_scales() {
        for scale in [Scale::X1, Scale::X2] {
            let atlas = Atlas::build(scale);
            for &icon in Icon::ALL {
                let cell = atlas.cell_pixels(icon);
                assert_eq!(cell.len(), (scale.cell() * scale.cell() * 4) as usize);
                assert!(
                    cell.chunks_exact(4).any(|px| px[3] > 0),
                    "{} is blank at {scale:?}",
                    icon.name()
                );
            }
        }
    }

    #[test]
    fn glyphs_are_white_on_transparent() {
        let atlas = Atlas::build(Scale::X2);
        for px in atlas.cell_pixels(Icon::Add).chunks_exact(4) {
            // Premultiplied white: every colour channel equals alpha.
            assert!(px[0] == px[3] && px[1] == px[3] && px[2] == px[3]);
        }
    }

    #[test]
    fn strokes_are_one_pixel_at_1x_and_three_at_2x() {
        // Add is a plus centred on x = 8: a vertical bar from y 3 to 13.
        let row = |scale: Scale| {
            let atlas = Atlas::build(scale);
            let cell = scale.cell() as usize;
            let pixels = atlas.cell_pixels(Icon::Add);
            let y = cell / 4; // above the horizontal bar, inside the vertical one
            (0..cell)
                .map(|x| u32::from(pixels[(y * cell + x) * 4 + 3]))
                .sum::<u32>()
        };
        assert!(row(Scale::X1).abs_diff(255) <= 3);
        assert!(row(Scale::X2).abs_diff(3 * 255) <= 3);
    }

    #[test]
    fn a_hinted_1x_svg_replaces_the_thinned_raster() {
        // The override table is empty today, so only the lookup is checked.
        assert!(Icon::ALL.iter().all(|i| i.one_x_override().is_none()));
    }

    #[test]
    fn icon_names_are_unique() {
        let mut names: Vec<_> = Icon::ALL.iter().map(|i| i.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Icon::ALL.len());
    }

    #[test]
    fn the_stamp_and_land_brush_icons_are_distinct() {
        let atlas = Atlas::build(Scale::X2);
        assert_ne!(
            atlas.cell_pixels(Icon::Stamp),
            atlas.cell_pixels(Icon::LandBrush)
        );
        assert_ne!(Icon::Stamp.svg(), Icon::LandBrush.svg());
    }

    #[test]
    fn rects_snap_to_whole_physical_pixels() {
        let container = egui::Rect::from_min_size(egui::pos2(10.3, 20.7), egui::vec2(24.0, 24.0));
        for ppp in [1.0, 1.25, 1.5, 2.0] {
            let rect = snapped_rect(container, GRID, ppp);
            for v in [rect.min.x, rect.min.y, rect.width(), rect.height()] {
                let physical = v * ppp;
                assert!((physical - physical.round()).abs() < 1e-3, "{v} at {ppp}");
            }
        }
        // 16 pt at 125% is 20 physical pixels.
        assert_eq!(snapped_rect(container, GRID, 1.25).width(), 16.0);
    }

    #[test]
    fn icon_button_exposes_its_label() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        let mut harness = Harness::new_ui(|ui| {
            icon_button(ui, Icon::Add, "New Layer");
        });
        harness.run();
        harness.get_by_label("New Layer");
    }
}
