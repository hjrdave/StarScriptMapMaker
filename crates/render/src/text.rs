//! Label shaping and glyph outlines (ADR 0006). This is CPU work only, so it is tested without
//! a GPU, and the editor uses the same layout for picking and handles.
//!
//! Text is shaped with cosmic-text against the bundled fonts only, never system fonts.
//! Characters a Label's family lacks fall back to Noto Serif, and to nothing else, so a Map
//! looks the same on every machine. Glyphs are drawn as vector outlines, not bitmaps, so a
//! Label stays sharp at any zoom and in export.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use cosmic_text::skrifa::instance::Size;
use cosmic_text::skrifa::outline::{DrawSettings, OutlinePen};
use cosmic_text::skrifa::prelude::*;
use cosmic_text::{
    Attrs, Buffer, CacheKeyFlags, Fallback, Family, FontSystem, Metrics, Shaping, Style, Weight,
    Wrap, fontdb,
};
use lyon::math::{Angle, Box2D, Point, Transform, point};
use lyon::path::Path;
use lyon::path::builder::NoAttributes;
use lyon::path::path::BuilderImpl;
use ssmap_core::{Align, ContentHash, Embedded, FontFace, ResolvedLabel};
use ssmap_fonts::{FALLBACK_FAMILY, FONTS};

use crate::color::ColorExt as _;
use crate::vector::{FillRule, Paint, VectorGroup, VectorShape};

/// How far a fake italic slants: the horizontal shift per unit of height, about 14°.
const FAKE_ITALIC_SLANT: f32 = 0.25;

/// The bundled fonts, loaded once and shared by every Label.
pub struct Fonts {
    system: FontSystem,
}

impl std::fmt::Debug for Fonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fonts")
            .field("faces", &self.system.db().len())
            .finish()
    }
}

/// Falls back to Noto Serif only: never to another display family, and never to system fonts.
struct BundledFallback {
    /// The display families as the fonts spell them, which can differ in case from
    /// [`LABEL_FAMILIES`](ssmap_fonts::LABEL_FAMILIES) ("IM FELL Double Pica").
    display: &'static [&'static str],
}

impl Fallback for BundledFallback {
    fn common_fallback(&self) -> &[&'static str] {
        &[FALLBACK_FAMILY]
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        self.display
    }

    fn script_fallback(&self, _script: unicode_script::Script, _locale: &str) -> &[&'static str] {
        &[]
    }
}

impl Fonts {
    /// Loads every bundled font. This parses each font file, so do it once.
    pub fn bundled() -> Self {
        Self::from_sources(
            FONTS
                .iter()
                .map(|font| fontdb::Source::Binary(Arc::new(font.data))),
        )
    }

    /// The fonts a Map renders with: every font file it embeds, and the bundled files it
    /// doesn't. An embedded file wins over the bundled file of the same name (ADR 0005).
    pub fn with_embedded(embedded: &Embedded) -> Self {
        let bundled = FONTS
            .iter()
            .filter(|font| !embedded.fonts.contains_key(font.file))
            .map(|font| fontdb::Source::Binary(Arc::new(font.data)));
        let own = embedded
            .fonts
            .values()
            .filter_map(|font| embedded.blob(&font.data))
            .map(|blob| fontdb::Source::Binary(Arc::new(blob.to_vec())));
        Self::from_sources(bundled.chain(own))
    }

    /// The embedded font files that differ from the bundled ones, by file name. Equal keys
    /// give the same [`Fonts`], so a Map that embeds only unchanged bundled fonts (the usual
    /// case) can share [`Fonts::bundled`].
    pub fn embedded_overrides(embedded: &Embedded) -> Vec<(String, ContentHash)> {
        static BUNDLED: OnceLock<HashMap<&'static str, ContentHash>> = OnceLock::new();
        let bundled = BUNDLED.get_or_init(|| {
            FONTS
                .iter()
                .map(|font| (font.file, ContentHash::of(font.data)))
                .collect()
        });
        embedded
            .fonts
            .iter()
            .filter(|(file, font)| bundled.get(file.as_str()) != Some(&font.data))
            .map(|(file, font)| (file.clone(), font.data))
            .collect()
    }

    fn from_sources(sources: impl Iterator<Item = fontdb::Source>) -> Self {
        let mut db = fontdb::Database::new();
        for source in sources {
            db.load_font_source(source);
        }
        db.set_serif_family(FALLBACK_FAMILY);
        db.set_sans_serif_family(FALLBACK_FAMILY);
        static DISPLAY: OnceLock<Vec<&'static str>> = OnceLock::new();
        let display = DISPLAY.get_or_init(|| {
            let mut names: Vec<&'static str> = Vec::new();
            for face in db.faces() {
                let name = &face.families[0].0;
                if name != FALLBACK_FAMILY && !names.contains(&name.as_str()) {
                    names.push(String::leak(name.clone()));
                }
            }
            names
        });
        // A fixed locale, so shaping never depends on the machine.
        let system = FontSystem::new_with_locale_and_db_and_fallback(
            "en-US".into(),
            db,
            BundledFallback { display },
        );
        Self { system }
    }

    /// The family as the fonts spell it, matched ignoring case; an unknown family is the
    /// fallback.
    fn family_name(&self, family: &str) -> String {
        self.system
            .db()
            .faces()
            .flat_map(|face| face.families.iter())
            .find(|(name, _)| name.eq_ignore_ascii_case(family))
            .map_or_else(|| FALLBACK_FAMILY.to_owned(), |(name, _)| name.clone())
    }

    /// The weight to shape `font` at: the requested one where the family has it (or a
    /// variable range covering it), otherwise the nearest face's, picked as CSS does.
    /// cosmic-text only uses a family's own glyphs at a weight it has.
    fn weight(&self, family: &str, font: &FontFace) -> u16 {
        let db = self.system.db();
        let query = fontdb::Query {
            families: &[Family::Name(family)],
            weight: Weight(font.weight),
            stretch: fontdb::Stretch::Normal,
            style: style(font),
        };
        let Some(id) = db.query(&query) else {
            return font.weight;
        };
        let covers = db
            .with_face_data(id, |data, index| {
                let font_ref = cosmic_text::skrifa::FontRef::from_index(data, index).ok()?;
                let axis = font_ref.axes().get_by_tag(Tag::new(b"wght"))?;
                let weight = f32::from(font.weight);
                Some(weight >= axis.min_value() && weight <= axis.max_value())
            })
            .flatten()
            .unwrap_or(false);
        match db.face(id) {
            Some(face) if !covers => face.weight.0,
            _ => font.weight,
        }
    }
}

fn style(font: &FontFace) -> Style {
    if font.italic {
        Style::Italic
    } else {
        Style::Normal
    }
}

/// A Label's text shaped into glyph outlines, in the Label's own frame: x runs along the
/// text and y down, with the origin at the Label's position, before rotation.
///
/// Shaping depends only on the text and typography, so moving, rotating or recolouring a
/// Label reuses its layout ([`LayoutKey`]).
#[derive(Clone, Debug)]
pub struct LabelLayout {
    outlines: Path,
    /// The text block: the line boxes of every line, unioned.
    bounds: Box2D,
    missing_glyphs: usize,
}

/// The Label fields shaping depends on. Equal keys give equal layouts.
#[derive(Clone, PartialEq, Debug)]
pub struct LayoutKey {
    text: String,
    family: String,
    weight: u16,
    italic: bool,
    size: f64,
    letter_spacing: f64,
    line_height: f64,
    arch: f64,
    align: Align,
}

impl LayoutKey {
    pub fn of(label: &ResolvedLabel) -> Self {
        Self {
            text: label.display_text(),
            family: label.font.family.clone(),
            weight: label.font.weight,
            italic: label.font.italic,
            size: label.size,
            letter_spacing: label.letter_spacing,
            line_height: label.line_height,
            arch: label.arch,
            align: label.align,
        }
    }
}

impl LabelLayout {
    /// Shapes `label`'s text in its font, with letter spacing, line height and alignment.
    pub fn shape(fonts: &mut Fonts, label: &ResolvedLabel) -> Self {
        let family = fonts.family_name(&label.font.family);
        let weight = fonts.weight(&family, &label.font);
        let system = &mut fonts.system;
        let size = label.size as f32;
        let spacing = label.letter_spacing as f32;
        let mut buffer = Buffer::new(system, Metrics::new(size, size * label.line_height as f32));
        buffer.set_wrap(Wrap::None);
        buffer.set_size(None, None);
        let attrs = Attrs::new()
            .family(Family::Name(&family))
            .weight(Weight(weight))
            .style(style(&label.font))
            .letter_spacing(spacing);
        buffer.set_text(&label.display_text(), &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(system, false);

        let mut builder = Path::builder();
        let mut bounds: Option<Box2D> = None;
        let mut missing_glyphs = 0;
        let mut block_top = f32::INFINITY;
        let mut block_bottom = f32::NEG_INFINITY;
        let runs: Vec<_> = buffer.layout_runs().collect();
        for run in &runs {
            block_top = block_top.min(run.line_top);
            block_bottom = block_bottom.max(run.line_top + run.line_height);
        }
        // Lines centre vertically on the position.
        let dy = -(block_top + block_bottom) / 2.0;
        for run in &runs {
            // Letter spacing follows every character, but the last shouldn't push the line
            // off centre.
            let width = if run.glyphs.is_empty() {
                0.0
            } else {
                (run.line_w - spacing * size).max(0.0)
            };
            let dx = match label.align {
                Align::Left => 0.0,
                Align::Centre => -width / 2.0,
                Align::Right => -width,
            };
            // The arc's radius: the line's width spans `arch` half circles. Its sign is the
            // side of the line the circle's centre is on (below for an arch up, y down).
            let radius = (label.arch != 0.0 && width > 0.0)
                .then(|| f64::from(width) / (label.arch * std::f64::consts::PI))
                .map(|r| r as f32);
            let centre = dx + width / 2.0;
            let baseline = run.line_y + dy;
            let (top, bottom) = (run.line_top + dy, run.line_top + run.line_height + dy);
            if radius.is_none() {
                let line = Box2D::new(point(dx, top), point(dx + width, bottom));
                bounds = Some(bounds.map_or(line, |b| b.union(&line)));
            }
            for glyph in run.glyphs {
                // Where the glyph's middle falls on the straight line, and the turn that
                // takes it onto the arc.
                let mid = glyph.x + dx + glyph.w / 2.0;
                let (pivot, sin, cos) = match radius {
                    Some(r) => {
                        let angle = (mid - centre) / r;
                        let (sin, cos) = angle.sin_cos();
                        (point(centre + r * sin, baseline + r - r * cos), sin, cos)
                    }
                    None => (point(mid, baseline), 0.0, 1.0),
                };
                if radius.is_some() {
                    let half = glyph.w / 2.0;
                    for (x, y) in [(-half, top), (half, top), (half, bottom), (-half, bottom)] {
                        let (x, y) = (x, y - baseline);
                        let corner =
                            point(pivot.x + x * cos - y * sin, pivot.y + x * sin + y * cos);
                        // Not `union`: it ignores boxes with no area.
                        bounds = Some(bounds.map_or(Box2D::new(corner, corner), |b| {
                            Box2D::new(b.min.min(corner), b.max.max(corner))
                        }));
                    }
                }
                if glyph.glyph_id == 0 {
                    // Never draw the missing-glyph box.
                    missing_glyphs += 1;
                    continue;
                }
                let origin = point(
                    glyph.x + glyph.font_size * glyph.x_offset + dx,
                    run.line_y + glyph.y - glyph.font_size * glyph.y_offset + dy,
                );
                let slant = if glyph.cache_key_flags.contains(CacheKeyFlags::FAKE_ITALIC) {
                    FAKE_ITALIC_SLANT
                } else {
                    0.0
                };
                let placement = Placement {
                    pivot,
                    offset: point(origin.x - mid, origin.y - baseline),
                    sin,
                    cos,
                };
                draw_glyph(system, glyph, placement, slant, &mut builder);
            }
        }
        Self {
            outlines: builder.build(),
            bounds: bounds.unwrap_or_else(Box2D::zero),
            missing_glyphs,
        }
    }

    /// Characters no bundled font could draw. They are left out, never drawn as boxes.
    pub fn missing_glyphs(&self) -> usize {
        self.missing_glyphs
    }

    /// The glyph outlines in the Label's own frame.
    pub fn outlines(&self) -> &Path {
        &self.outlines
    }

    /// The text block in the Label's own frame, as `[min, max]`.
    pub fn bounds(&self) -> [[f64; 2]; 2] {
        [
            [f64::from(self.bounds.min.x), f64::from(self.bounds.min.y)],
            [f64::from(self.bounds.max.x), f64::from(self.bounds.max.y)],
        ]
    }

    /// The corners of the text block on the Canvas, clockwise from the top-left, grown by
    /// the halo.
    pub fn corners(&self, label: &ResolvedLabel) -> [[f64; 2]; 4] {
        let grow = label.halo.width;
        let [min, max] = self.bounds();
        let (min, max) = (
            [min[0] - grow, min[1] - grow],
            [max[0] + grow, max[1] + grow],
        );
        [
            [min[0], min[1]],
            [max[0], min[1]],
            [max[0], max[1]],
            [min[0], max[1]],
        ]
        .map(|corner| to_canvas(label, corner))
    }

    /// Whether a Canvas point falls on the text block or its halo.
    pub fn contains(&self, label: &ResolvedLabel, canvas_point: [f64; 2]) -> bool {
        let [x, y] = to_local(label, canvas_point);
        let grow = label.halo.width;
        let [min, max] = self.bounds();
        x >= min[0] - grow && x <= max[0] + grow && y >= min[1] - grow && y <= max[1] + grow
    }

    /// What to draw for `label`: its fill on top of its halo, faded by `opacity` (such as
    /// the Layers above it) as well as the Label's own.
    pub fn group(&self, label: &ResolvedLabel, opacity: f32) -> VectorGroup {
        let opacity = opacity * label.opacity;
        let path = Arc::new(self.outlines.clone().transformed(&transform(label)));
        let mut shapes = vec![VectorShape {
            path: path.clone(),
            color: label.fill.faded(opacity),
            // Variable fonts overlap their contours, which even-odd would punch holes in.
            paint: Paint::Fill(FillRule::NonZero),
        }];
        if label.halo.width > 0.0 {
            // The stroke is centred on the outline, and the fill hides its inner half.
            shapes.push(VectorShape {
                path,
                color: label.halo.color.faded(opacity),
                paint: Paint::Stroke {
                    width: (2.0 * label.halo.width) as f32,
                },
            });
        }
        VectorGroup {
            shapes,
            effect: false,
            object: None,
            placement: crate::vector::Placement::Canvas,
        }
    }
}

/// The Label's frame to Canvas: rotate clockwise about the position, then move there.
fn transform(label: &ResolvedLabel) -> Transform {
    Transform::rotation(Angle::degrees(label.rotation as f32))
        .then_translate([label.position[0] as f32, label.position[1] as f32].into())
}

/// A point in the Label's own frame on the Canvas, in `f64` for picking and handles.
pub fn to_canvas(label: &ResolvedLabel, local: [f64; 2]) -> [f64; 2] {
    let (sin, cos) = label.rotation.to_radians().sin_cos();
    [
        label.position[0] + local[0] * cos - local[1] * sin,
        label.position[1] + local[0] * sin + local[1] * cos,
    ]
}

/// A Canvas point in the Label's own frame.
pub fn to_local(label: &ResolvedLabel, canvas: [f64; 2]) -> [f64; 2] {
    let (sin, cos) = label.rotation.to_radians().sin_cos();
    let [dx, dy] = [canvas[0] - label.position[0], canvas[1] - label.position[1]];
    [dx * cos + dy * sin, -dx * sin + dy * cos]
}

/// Where a glyph goes: its origin is `offset` from `pivot` (the middle of its advance on the
/// baseline), and the whole glyph is turned clockwise about `pivot` by the angle `sin`/`cos`.
#[derive(Clone, Copy)]
struct Placement {
    pivot: Point,
    offset: Point,
    sin: f32,
    cos: f32,
}

/// Adds one glyph's outline, placed as `placement` says.
fn draw_glyph(
    system: &mut FontSystem,
    glyph: &cosmic_text::LayoutGlyph,
    placement: Placement,
    slant: f32,
    builder: &mut NoAttributes<BuilderImpl>,
) {
    let Some(font) = system.get_font(glyph.font_id, glyph.font_weight) else {
        return;
    };
    let Some(index) = system.db().face(glyph.font_id).map(|face| face.index) else {
        return;
    };
    let Ok(font_ref) = cosmic_text::skrifa::FontRef::from_index(font.data(), index) else {
        return;
    };
    let Some(outline) = font_ref
        .outline_glyphs()
        .get(GlyphId::new(u32::from(glyph.glyph_id)))
    else {
        return;
    };
    // The same weight the shaper used, for variable fonts.
    let location = font_ref
        .axes()
        .location([(Tag::new(b"wght"), f32::from(glyph.font_weight.0))]);
    let mut pen = Pen {
        builder,
        placement,
        slant,
        open: false,
    };
    let settings = DrawSettings::unhinted(Size::new(glyph.font_size), &location);
    if outline.draw(settings, &mut pen).is_ok() && pen.open {
        pen.builder.end(true);
    }
}

/// Turns font outlines (y up, from the glyph origin) into Label-frame paths (y down).
struct Pen<'a> {
    builder: &'a mut NoAttributes<BuilderImpl>,
    placement: Placement,
    slant: f32,
    open: bool,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> Point {
        let Placement {
            pivot,
            offset,
            sin,
            cos,
        } = self.placement;
        let (x, y) = (offset.x + x + y * self.slant, offset.y - y);
        point(pivot.x + x * cos - y * sin, pivot.y + x * sin + y * cos)
    }
}

impl OutlinePen for Pen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.open {
            self.builder.end(true);
        }
        self.builder.begin(self.at(x, y));
        self.open = true;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(self.at(x, y));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.builder
            .quadratic_bezier_to(self.at(cx0, cy0), self.at(x, y));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.builder
            .cubic_bezier_to(self.at(cx0, cy0), self.at(cx1, cy1), self.at(x, y));
    }

    fn close(&mut self) {
        if self.open {
            self.builder.end(true);
            self.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::FontFace;
    use ssmap_fonts::LABEL_FAMILIES;

    fn label(text: &str, family: &str) -> ResolvedLabel {
        ResolvedLabel {
            text: text.into(),
            font: FontFace {
                family: family.into(),
                ..FontFace::default()
            },
            size: 100.0,
            ..ResolvedLabel::default()
        }
    }

    fn width(layout: &LabelLayout) -> f64 {
        let [min, max] = layout.bounds();
        max[0] - min[0]
    }

    /// Which family drew each glyph of `label`, as the fonts spell it.
    fn families(fonts: &mut Fonts, label: &ResolvedLabel) -> Vec<String> {
        let family = fonts.family_name(&label.font.family);
        let weight = fonts.weight(&family, &label.font);
        let mut buffer = Buffer::new(&mut fonts.system, Metrics::new(100.0, 120.0));
        let attrs = Attrs::new()
            .family(Family::Name(&family))
            .weight(Weight(weight));
        buffer.set_text(&label.text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut fonts.system, false);
        let ids: Vec<_> = buffer
            .layout_runs()
            .flat_map(|run| run.glyphs.iter().map(|g| g.font_id))
            .collect();
        ids.into_iter()
            .map(|id| fonts.system.db().face(id).unwrap().families[0].0.clone())
            .collect()
    }

    #[test]
    fn every_bundled_family_draws_ascii() {
        let mut fonts = Fonts::bundled();
        for family in LABEL_FAMILIES {
            let layout = LabelLayout::shape(&mut fonts, &label("Hamlet", family));
            assert_eq!(layout.missing_glyphs(), 0, "{family}");
            assert!(width(&layout) > 100.0, "{family} drew something");
            assert!(
                families(&mut fonts, &label("Hamlet", family))
                    .iter()
                    .all(|used| used.eq_ignore_ascii_case(family)),
                "{family} drew its own glyphs"
            );
        }
    }

    #[test]
    fn missing_characters_fall_back_to_noto_serif_without_boxes() {
        let mut fonts = Fonts::bundled();
        for family in LABEL_FAMILIES {
            let label = label("Ærendál", family);
            let layout = LabelLayout::shape(&mut fonts, &label);
            assert_eq!(layout.missing_glyphs(), 0, "{family}");
            for used in families(&mut fonts, &label) {
                assert!(
                    used.eq_ignore_ascii_case(family) || used == FALLBACK_FAMILY,
                    "{family} fell back to {used}"
                );
            }
        }
        // Characters only Noto Serif has come from it.
        let greek = label("Ωμέγα", "Uncial Antiqua");
        assert!(
            families(&mut fonts, &greek)
                .iter()
                .any(|f| f == FALLBACK_FAMILY)
        );
        assert_eq!(LabelLayout::shape(&mut fonts, &greek).missing_glyphs(), 0);
    }

    #[test]
    fn weights_a_family_lacks_use_its_nearest_face() {
        let mut fonts = Fonts::bundled();
        // MedievalSharp has only a 500 face; Labels default to 400.
        let mut text = label("Hamlet", "MedievalSharp");
        assert!(
            families(&mut fonts, &text)
                .iter()
                .all(|f| f == "MedievalSharp")
        );
        text.font.weight = 900;
        assert!(
            families(&mut fonts, &text)
                .iter()
                .all(|f| f == "MedievalSharp")
        );
        // Almendra has a bold face, and Cinzel a variable weight axis.
        text.font.family = "Almendra".into();
        text.font.weight = 700;
        assert_eq!(fonts.weight("Almendra", &text.font), 700);
        text.font.weight = 600;
        assert_eq!(fonts.weight("Almendra", &text.font), 700);
        text.font.family = "Cinzel".into();
        assert_eq!(fonts.weight("Cinzel", &text.font), 600);
    }

    #[test]
    fn unknown_families_use_the_fallback() {
        let mut fonts = Fonts::bundled();
        let label = label("Thule", "Comic Sans");
        assert_eq!(LabelLayout::shape(&mut fonts, &label).missing_glyphs(), 0);
        assert!(
            families(&mut fonts, &label)
                .iter()
                .all(|f| f == FALLBACK_FAMILY)
        );
    }

    #[test]
    fn characters_no_font_has_are_counted_and_not_drawn() {
        let mut fonts = Fonts::bundled();
        let layout = LabelLayout::shape(&mut fonts, &label("A\u{e000}B", "Cinzel"));
        assert_eq!(layout.missing_glyphs(), 1);
    }

    #[test]
    fn lines_stack_by_line_height_and_centre_on_the_position() {
        let mut fonts = Fonts::bundled();
        let mut two = label("North\nSea", "Cinzel");
        two.line_height = 1.5;
        let [min, max] = LabelLayout::shape(&mut fonts, &two).bounds();
        assert!((max[1] - min[1] - 300.0).abs() < 1e-3, "two lines of 150");
        assert!((min[1] + max[1]).abs() < 1e-3, "centred vertically");
        assert!((min[0] + max[0]).abs() < 1e-3, "centred horizontally");
    }

    #[test]
    fn alignment_puts_the_position_at_the_left_or_right_edge() {
        let mut fonts = Fonts::bundled();
        let mut text = label("Westmarch\nby the sea", "Almendra");
        text.align = Align::Left;
        let left = LabelLayout::shape(&mut fonts, &text).bounds();
        text.align = Align::Right;
        let right = LabelLayout::shape(&mut fonts, &text).bounds();
        assert!(left[0][0].abs() < 1e-3);
        assert!(right[1][0].abs() < 1e-3);
        assert!(
            (left[1][0] + right[0][0]).abs() < 1e-3,
            "same width either way"
        );
    }

    #[test]
    fn arch_bends_lines_up_or_down_and_zero_leaves_them_straight() {
        let mut fonts = Fonts::bundled();
        let mut text = label("Sea of Storms", "Cinzel");
        let flat = LabelLayout::shape(&mut fonts, &text);
        text.arch = 0.5;
        let up = LabelLayout::shape(&mut fonts, &text);
        text.arch = -0.5;
        let down = LabelLayout::shape(&mut fonts, &text);
        let height = |l: &LabelLayout| l.bounds()[1][1] - l.bounds()[0][1];
        assert!(
            height(&up) > height(&flat) + 10.0,
            "bending adds height {} {}",
            height(&up),
            height(&flat)
        );
        // The line box isn't symmetric about the baseline, so the two aren't exact mirrors.
        assert!(
            (height(&up) - height(&down)).abs() < height(&up) * 0.1,
            "alike either way {} {}",
            height(&up),
            height(&down)
        );
        // An arch up lowers the ends below the middle; a sag raises them.
        let [flat_min, flat_max] = flat.bounds();
        assert!(up.bounds()[1][1] > flat_max[1] + 10.0);
        assert!(down.bounds()[0][1] < flat_min[1] - 10.0);
    }

    #[test]
    fn letter_spacing_widens_by_the_gaps_between_characters() {
        let mut fonts = Fonts::bundled();
        let mut text = label("ABCDE", "Cinzel");
        let tight = width(&LabelLayout::shape(&mut fonts, &text));
        text.letter_spacing = 0.5;
        let spaced = width(&LabelLayout::shape(&mut fonts, &text));
        // Four gaps of half an em at 100 units.
        assert!((spaced - tight - 200.0).abs() < 0.5, "{tight} -> {spaced}");
    }

    #[test]
    fn all_caps_draws_capitals_but_keeps_the_text() {
        let mut fonts = Fonts::bundled();
        let mut text = label("Ærendál", "IM Fell Double Pica");
        let lower = width(&LabelLayout::shape(&mut fonts, &text));
        text.all_caps = true;
        assert_eq!(text.display_text(), "ÆRENDÁL");
        assert_eq!(text.text, "Ærendál");
        let caps = width(&LabelLayout::shape(&mut fonts, &text));
        assert!(caps > lower);
        assert_ne!(
            LayoutKey::of(&text),
            LayoutKey::of(&ResolvedLabel {
                all_caps: false,
                ..text
            })
        );
    }

    #[test]
    fn picking_follows_rotation() {
        let mut fonts = Fonts::bundled();
        let mut text = label("Long Road", "Cinzel");
        text.position = [1000.0, 500.0];
        let layout = LabelLayout::shape(&mut fonts, &text);
        let half = width(&layout) / 2.0;
        let end = [1000.0 + half * 0.9, 500.0];
        assert!(layout.contains(&text, end));
        text.rotation = 90.0;
        assert!(!layout.contains(&text, end));
        // Clockwise: the end of the text now points down.
        assert!(layout.contains(&text, [1000.0, 500.0 + half * 0.9]));
        let corners = layout.corners(&text);
        assert!(corners[1][1] > corners[0][1], "top-right is below top-left");
    }

    #[test]
    fn local_and_canvas_frames_invert() {
        let text = ResolvedLabel {
            position: [12.0, -7.0],
            rotation: 33.0,
            ..label("x", "Cinzel")
        };
        let back = to_local(&text, to_canvas(&text, [5.0, 9.0]));
        assert!((back[0] - 5.0).abs() < 1e-9 && (back[1] - 9.0).abs() < 1e-9);
    }

    #[test]
    fn the_halo_is_a_stroke_under_the_fill() {
        let mut fonts = Fonts::bundled();
        let mut text = label("Isle", "Cinzel");
        let layout = LabelLayout::shape(&mut fonts, &text);
        assert_eq!(layout.group(&text, 1.0).shapes.len(), 1);
        text.halo.width = 4.0;
        let group = layout.group(&text, 0.5);
        assert_eq!(group.shapes[0].paint, Paint::Fill(FillRule::NonZero));
        assert_eq!(group.shapes[1].paint, Paint::Stroke { width: 8.0 });
        assert_eq!(group.shapes[0].color.a, 128);
    }
}
