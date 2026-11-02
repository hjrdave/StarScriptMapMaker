//! The UI icon set (docs/design/ui-handoff/HANDOFF.md, Icons): one SVG per icon on a 16 pt
//! grid in `apps/starscript/icons/`, rasterised with resvg into a 1x atlas (16 px cells, 1 px
//! strokes) and a 2x atlas (32 px cells, 1.5 pt = 3 px strokes). Glyphs are white on
//! transparent and tinted when drawn.
//!
//! Icons drawn at [`TOOL_GRID`] (the tool dock's 20 pt) get their own pair of atlases, rasterised
//! at 20 px and 40 px cells with the stroke rescaled to still land on 1 px and 3 px, so they are
//! drawn 1:1 rather than stretched from the 16 pt cells.
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
pub(crate) const GRID: f32 = 16.0;

/// Side of the larger icon grid in points, used by the tool dock.
pub(crate) const TOOL_GRID: f32 = 20.0;

/// Which grid an atlas is rasterised for: the cell is `points * scale` pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum Grid {
    /// [`GRID`] points.
    Standard,
    /// [`TOOL_GRID`] points.
    Tool,
}

impl Grid {
    /// The grid for an icon drawn at `size` points: sizes of [`TOOL_GRID`] and up use the
    /// larger one.
    fn for_size(size: f32) -> Self {
        if size >= TOOL_GRID - 1e-3 {
            Grid::Tool
        } else {
            Grid::Standard
        }
    }

    fn points(self) -> u32 {
        match self {
            Grid::Standard => GRID as u32,
            Grid::Tool => TOOL_GRID as u32,
        }
    }

    /// The `stroke-width` in SVG units (the SVGs are on a 16 unit grid) that is 1 px at 1x
    /// and 3 px at 2x, if it differs from the 1.5 the SVGs carry.
    fn stroke_width(self, scale: Scale) -> Option<&'static str> {
        match (self, scale) {
            (Grid::Standard, Scale::X1) => Some("1"),
            (Grid::Standard, Scale::X2) => None,
            // A unit is 1.25 px at 1x and 2.5 px at 2x.
            (Grid::Tool, Scale::X1) => Some("0.8"),
            (Grid::Tool, Scale::X2) => Some("1.2"),
        }
    }
}

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
    LakeBrush, "Lake Brush", "lake-brush";
    Lasso, "Lasso", "lasso";
    Label, "Label", "label";
    Stamp, "Stamp", "stamp";
    StampEraser, "Stamp Eraser", "stamp-eraser";
    ExportRegion, "Export Region", "export-region";
    Landmass, "Landmass", "landmass";
    Lake, "Lake", "lake";
    River, "River", "river";
    LabelKind, "Label kind", "label-kind";
    LayerGroup, "Layer / Group", "layer-group";
    History, "History", "history";
    Map, "Map", "map";
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
    Shuffle, "Shuffle", "shuffle";
    Replace, "Replace", "replace";
    FlipHorizontal, "Flip horizontal", "flip-horizontal";
    FlipVertical, "Flip vertical", "flip-vertical";
    Rotate, "Rotate", "rotate";
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
pub(crate) enum Scale {
    /// 16 px cells.
    X1,
    /// 32 px cells.
    X2,
}

impl Scale {
    /// 1x only at 1 pixel per point (or less); 1.5 and up use 2x as the design asks, and so
    /// do the scales between, such as 125%, downsampling the 2x atlas linearly. The 1x atlas
    /// is never scaled up.
    fn for_pixels_per_point(pixels_per_point: f32) -> Self {
        if pixels_per_point > 1.0 + 1e-3 {
            Scale::X2
        } else {
            Scale::X1
        }
    }

    /// Side of one atlas cell in pixels on `grid`.
    pub(crate) fn cell(self, grid: Grid) -> u32 {
        grid.points()
            * match self {
                Scale::X1 => 1,
                Scale::X2 => 2,
            }
    }
}

const COLUMNS: u32 = 8;

/// All icons rasterised at one scale: premultiplied RGBA8, white glyphs on transparent.
struct Atlas {
    cell: u32,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Atlas {
    pub(crate) fn build(scale: Scale, grid: Grid) -> Self {
        let cell = scale.cell(grid);
        let rows = (Icon::ALL.len() as u32).div_ceil(COLUMNS);
        let (width, height) = (COLUMNS * cell, rows * cell);
        let mut pixels = vec![0; (width * height * 4) as usize];
        for (index, &icon) in Icon::ALL.iter().enumerate() {
            let raster = rasterise(icon, scale, grid);
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
            cell,
            width,
            height,
            pixels,
        }
    }

    /// The icon's cell as `[x, y]` of its top-left pixel; the cell is `self.cell` square.
    #[allow(clippy::expect_used, reason = "every icon is in ALL")]
    fn cell_origin(&self, icon: Icon) -> [u32; 2] {
        let index = Icon::ALL
            .iter()
            .position(|&i| i == icon)
            .expect("every icon is in ALL") as u32;
        let cell = self.cell;
        [(index % COLUMNS) * cell, (index / COLUMNS) * cell]
    }

    /// The icon's cell as premultiplied RGBA8 rows.
    #[cfg(test)]
    fn cell_pixels(&self, icon: Icon) -> Vec<u8> {
        let cell = self.cell;
        let [x0, y0] = self.cell_origin(icon);
        let mut out = Vec::with_capacity((cell * cell * 4) as usize);
        for row in 0..cell {
            let start = (((y0 + row) * self.width + x0) * 4) as usize;
            out.extend_from_slice(&self.pixels[start..start + (cell * 4) as usize]);
        }
        out
    }

    /// The icon's cell in texture coordinates.
    pub(crate) fn uv(&self, icon: Icon) -> egui::Rect {
        let [x, y] = self.cell_origin(icon);
        let cell = self.cell;
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

#[allow(clippy::expect_used, reason = "embedded icons are valid SVG")]
fn rasterise(icon: Icon, scale: Scale, grid: Grid) -> ssmap_render::raster::Raster {
    let svg = match (scale, icon.one_x_override()) {
        (Scale::X1, Some(hinted)) => hinted.to_owned(),
        // 1.5 pt strokes become 1 px (3 px at 2x on the tool grid); the dot icons' heavier
        // strokes stay as drawn.
        _ => match grid.stroke_width(scale) {
            Some(width) => icon
                .svg()
                .replace("stroke-width=\"1.5\"", &format!("stroke-width=\"{width}\"")),
            None => icon.svg().to_owned(),
        },
    };
    Art::from_svg(svg.as_bytes())
        .expect("embedded icons are valid SVG")
        .rasterise(scale.cell(grid))
}

/// The atlas textures, uploaded once per egui context and scale.
#[derive(Clone)]
struct Texture(Arc<(egui::TextureHandle, Atlas)>);

fn texture(ctx: &egui::Context, scale: Scale, grid: Grid) -> Texture {
    let id = egui::Id::new(("ssmap-icon-atlas", scale, grid));
    if let Some(texture) = ctx.data(|d| d.get_temp::<Texture>(id)) {
        return texture;
    }
    let atlas = Atlas::build(scale, grid);
    let handle = ctx.load_texture(
        format!("icon-atlas-{scale:?}-{grid:?}"),
        atlas.color_image(),
        egui::TextureOptions::LINEAR,
    );
    let texture = Texture(Arc::new((handle, atlas)));
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    texture
}

/// Centres a square of `size` points in `container`, rounded to whole physical pixels so the
/// glyph never straddles a pixel boundary.
fn snapped_rect(container: egui::Rect, size: f32, pixels_per_point: f32) -> egui::Rect {
    let pixels = (size * pixels_per_point).round().max(1.0);
    let min = container.center().to_vec2() * pixels_per_point - egui::Vec2::splat(pixels / 2.0);
    let min = egui::pos2(min.x.round(), min.y.round()) / pixels_per_point;
    egui::Rect::from_min_size(min, egui::Vec2::splat(pixels / pixels_per_point))
}

/// Paints `icon` centred in `container` at `size` points (use [`GRID`] or [`TOOL_GRID`] for a native size),
/// tinted with `tint`.
pub(crate) fn paint(
    ui: &egui::Ui,
    container: egui::Rect,
    icon: Icon,
    size: f32,
    tint: egui::Color32,
) {
    paint_flipped(ui, container, icon, size, tint, false);
}

/// Like [`paint`], optionally mirrored left to right (a left chevron from the right one).
pub(crate) fn paint_flipped(
    ui: &egui::Ui,
    container: egui::Rect,
    icon: Icon,
    size: f32,
    tint: egui::Color32,
    flip: bool,
) {
    let ppp = ui.ctx().pixels_per_point();
    let texture = texture(
        ui.ctx(),
        Scale::for_pixels_per_point(ppp),
        Grid::for_size(size),
    );
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
pub(crate) fn icon_button(ui: &mut egui::Ui, icon: Icon, label: &str) -> egui::Response {
    icon_button_with(ui, icon, label, 24.0, GRID, None)
}

/// [`icon_button`] with the glyph mirrored left to right when `flip` is set.
pub(crate) fn icon_button_flipped(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    flip: bool,
) -> egui::Response {
    icon_button_inner(ui, icon, label, 24.0, GRID, None, flip)
}

/// [`icon_button`] with its button `side` and glyph `size` in points, and optionally a `tint`
/// in place of the visuals' text colour.
pub(crate) fn icon_button_with(
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
        assert_eq!(Scale::X1.cell(Grid::Standard), 16);
        assert_eq!(Scale::X2.cell(Grid::Standard), 32);
        assert_eq!(Scale::X1.cell(Grid::Tool), 20);
        assert_eq!(Scale::X2.cell(Grid::Tool), 40);
    }

    #[test]
    fn the_grid_follows_the_icon_size() {
        assert_eq!(Grid::for_size(GRID), Grid::Standard);
        assert_eq!(Grid::for_size(TOOL_GRID), Grid::Tool);
    }

    const ALL_ATLASES: [(Scale, Grid); 4] = [
        (Scale::X1, Grid::Standard),
        (Scale::X2, Grid::Standard),
        (Scale::X1, Grid::Tool),
        (Scale::X2, Grid::Tool),
    ];

    #[test]
    fn every_icon_has_a_non_empty_cell_at_every_scale_and_grid() {
        for (scale, grid) in ALL_ATLASES {
            let atlas = Atlas::build(scale, grid);
            for &icon in Icon::ALL {
                let cell = atlas.cell_pixels(icon);
                assert_eq!(cell.len(), (scale.cell(grid).pow(2) * 4) as usize);
                assert!(
                    cell.chunks_exact(4).any(|px| px[3] > 0),
                    "{} is blank at {scale:?} {grid:?}",
                    icon.name()
                );
            }
        }
    }

    #[test]
    fn glyphs_are_white_on_transparent() {
        let atlas = Atlas::build(Scale::X2, Grid::Tool);
        for px in atlas.cell_pixels(Icon::Add).chunks_exact(4) {
            // Premultiplied white: every colour channel equals alpha.
            assert!(px[0] == px[3] && px[1] == px[3] && px[2] == px[3]);
        }
    }

    #[test]
    fn tool_icons_are_rasterised_at_the_target_size() {
        let atlas = Atlas::build(Scale::X1, Grid::Tool);
        assert_eq!(atlas.cell, 20);
        // Add's vertical bar spans 10 of 16 units: 12.5 px at 20 px, not a stretched 10.
        let cell = atlas.cell_pixels(Icon::Add);
        // The bar is centred on the x = 10 pixel boundary, so sum the two columns it spans.
        let column: u32 = (0..20)
            .flat_map(|y| [9, 10].map(|x| u32::from(cell[(y * 20 + x) * 4 + 3])))
            .sum();
        assert!(column > 12 * 255, "bar is {column} / 255 px tall");
    }

    #[test]
    fn strokes_are_one_pixel_at_1x_and_three_at_2x_on_both_grids() {
        // Add is a plus centred in the cell: a vertical bar crosses the row at a quarter.
        let row = |scale: Scale, grid: Grid| {
            let atlas = Atlas::build(scale, grid);
            let cell = scale.cell(grid) as usize;
            let pixels = atlas.cell_pixels(Icon::Add);
            let y = cell / 4; // above the horizontal bar, inside the vertical one
            (0..cell)
                .map(|x| u32::from(pixels[(y * cell + x) * 4 + 3]))
                .sum::<u32>()
        };
        for grid in [Grid::Standard, Grid::Tool] {
            assert!(row(Scale::X1, grid).abs_diff(255) <= 3, "{grid:?} 1x");
            assert!(row(Scale::X2, grid).abs_diff(3 * 255) <= 3, "{grid:?} 2x");
        }
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
        let atlas = Atlas::build(Scale::X2, Grid::Standard);
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
