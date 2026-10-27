//! The Label styles window: a list of the Map's named styles, each previewed in its own
//! type, and an editor for the selected one. Every change is a Command, so it is undoable;
//! editing a style restyles all of its Labels in one Undo step.
//!
//! A style has no Align or Rotation (those belong to each Label and are set in its
//! Properties), so the editor has today's fields and no more.

use std::collections::HashMap;

use eframe::egui;
use egui::{Color32, Sense, Vec2};
use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};
use ssmap_core::{Color, Command, Id, LabelStyle, ResolvedLabel};
use ssmap_fonts::LABEL_FAMILIES;
use ssmap_render::{Fonts, LabelLayout};

use crate::editor::Editor;
use crate::prop_widgets::{
    Edit, LiveEdit, Provenance, ScrubSpec, checkbox_row, number_field, slider, swatch,
};
use crate::theme;

/// How tall the preview of a style's name is drawn, in points.
const PREVIEW_EM: f32 = 18.0;
const ROW_HEIGHT: f32 = 40.0;
const LIST_WIDTH: f32 = 220.0;

#[derive(Default)]
pub(crate) struct LabelStylesPanel {
    pub open: bool,
    /// The style listed as being edited.
    selected: Option<Id>,
    /// The rename box: the style it belongs to and the text typed so far.
    rename: Option<(Id, String)>,
    live: LiveEdit,
    /// Loaded on first use: parsing the bundled fonts is not free.
    fonts: Option<Fonts>,
    /// Each style's shaped name, kept while the style is unchanged.
    previews: HashMap<Id, Preview>,
}

/// A style's name shaped and tessellated in the style, in the Label's own frame.
struct Preview {
    /// The style it was built from; the preview is rebuilt when the style differs.
    style: LabelStyle,
    fill: VertexBuffers<[f32; 2], u32>,
    halo: VertexBuffers<[f32; 2], u32>,
    /// `[min, max]` of the text block.
    bounds: [[f64; 2]; 2],
}

impl Preview {
    fn build(fonts: &mut Fonts, style: &LabelStyle) -> Self {
        let resolved = ResolvedLabel {
            text: style.name.clone(),
            font: style.font.clone(),
            size: style.size,
            letter_spacing: style.letter_spacing,
            line_height: style.line_height,
            arch: style.arch,
            all_caps: style.all_caps,
            fill: style.fill,
            opacity: style.opacity,
            halo: style.halo.clone(),
            ..ResolvedLabel::default()
        };
        let layout = LabelLayout::shape(fonts, &resolved);
        let tolerance = (style.size as f32 * 0.005).max(0.01);
        let fill = tessellate_fill(layout.outlines(), tolerance);
        let halo = if style.halo.width > 0.0 {
            tessellate_halo(layout.outlines(), style.halo.width as f32, tolerance)
        } else {
            VertexBuffers::new()
        };
        Self {
            style: style.clone(),
            fill,
            halo,
            bounds: layout.bounds(),
        }
    }

    /// Draws the name at `left`, centred on `centre_y`, no wider than `max_width`.
    fn paint(&self, painter: &egui::Painter, left: f32, centre_y: f32, max_width: f32) {
        let [min, max] = self.bounds;
        let width = (max[0] - min[0]) as f32;
        let mut scale = PREVIEW_EM / self.style.size.max(0.1) as f32;
        if width * scale > max_width && width > 0.0 {
            scale = max_width / width;
        }
        let origin = egui::pos2(left, centre_y);
        let faded = |c: Color| {
            let alpha = (f32::from(c.a) * self.style.opacity.clamp(0.0, 1.0)).round() as u8;
            Color32::from_rgba_unmultiplied(c.r, c.g, c.b, alpha)
        };
        for (buffers, colour) in [
            (&self.halo, faded(self.style.halo.color)),
            (&self.fill, faded(self.style.fill)),
        ] {
            if buffers.indices.is_empty() {
                continue;
            }
            let mut mesh = egui::Mesh::default();
            mesh.indices.clone_from(&buffers.indices);
            mesh.vertices = buffers
                .vertices
                .iter()
                .map(|[x, y]| egui::epaint::Vertex {
                    pos: origin + Vec2::new(x * scale, y * scale),
                    uv: egui::epaint::WHITE_UV,
                    color: colour,
                })
                .collect();
            painter.add(egui::Shape::mesh(mesh));
        }
    }
}

fn tessellate_fill(path: &Path, tolerance: f32) -> VertexBuffers<[f32; 2], u32> {
    let mut buffers = VertexBuffers::new();
    // Variable fonts overlap their contours, which even-odd would punch holes in.
    let options = FillOptions::tolerance(tolerance).with_fill_rule(FillRule::NonZero);
    // A glyph that fails to tessellate is left out of a preview, never an error.
    let _ = FillTessellator::new().tessellate_path(
        path,
        &options,
        &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| v.position().to_array()),
    );
    buffers
}

fn tessellate_halo(path: &Path, width: f32, tolerance: f32) -> VertexBuffers<[f32; 2], u32> {
    let mut buffers = VertexBuffers::new();
    let options = StrokeOptions::tolerance(tolerance).with_line_width(2.0 * width);
    let _ = StrokeTessellator::new().tessellate_path(
        path,
        &options,
        &mut BuffersBuilder::new(&mut buffers, |v: StrokeVertex| v.position().to_array()),
    );
    buffers
}

/// A name for a copy that no style has yet: "Name copy", then "Name copy 2", and so on.
pub(crate) fn free_name(taken: &[&str], base: &str) -> String {
    let first = format!("{base} copy");
    if !taken.contains(&first.as_str()) {
        return first;
    }
    let mut n = 2;
    loop {
        let name = format!("{first} {n}");
        if !taken.contains(&name.as_str()) {
            return name;
        }
        n += 1;
    }
}

/// The notice after deleting a style that `count` Labels used.
pub(crate) fn deleted_notice(name: &str, count: usize, default_name: &str) -> String {
    let labels = if count == 1 {
        "1 Label".to_owned()
    } else {
        format!("{count} Labels")
    };
    format!("Deleted “{name}”: {labels} moved to “{default_name}”, looking the same")
}

impl LabelStylesPanel {
    pub(crate) fn show(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        let mut open = self.open;
        egui::Window::new("Label styles")
            .open(&mut open)
            .default_width(620.0)
            .vscroll(true)
            .show(ctx, |ui| self.contents(ui, editor));
        self.open = open;
    }

    /// The style's preview, shaped again only if the style changed since.
    fn preview(&mut self, style: &LabelStyle) -> &Preview {
        let fonts = self.fonts.get_or_insert_with(Fonts::bundled);
        let entry = self
            .previews
            .entry(style.id)
            .or_insert_with(|| Preview::build(fonts, style));
        if entry.style != *style {
            *entry = Preview::build(fonts, style);
        }
        entry
    }

    fn contents(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let styles: Vec<LabelStyle> = editor.map().label_styles().iter().cloned().collect();
        self.previews
            .retain(|id, _| styles.iter().any(|style| style.id == *id));
        let selected = match self.selected {
            Some(id) if styles.iter().any(|style| style.id == id) => id,
            _ => LabelStyle::DEFAULT_ID,
        };
        self.selected = Some(selected);
        // The default style is always there, so this finds one.
        let Some(current) = styles.iter().find(|style| style.id == selected).cloned() else {
            return;
        };
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(LIST_WIDTH);
                self.list(ui, editor, &styles, selected);
            });
            ui.separator();
            ui.vertical(|ui| {
                self.editor_pane(ui, editor, &styles, &current);
            });
        });
    }

    fn list(&mut self, ui: &mut egui::Ui, editor: &Editor, styles: &[LabelStyle], selected: Id) {
        let palette = theme::palette(ui.ctx());
        for style in styles {
            let used = editor.map().labels_using(style.id).len();
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(LIST_WIDTH, ROW_HEIGHT), Sense::click());
            let is_selected = selected == style.id;
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    true,
                    is_selected,
                    &style.name,
                )
            });
            if is_selected || response.hovered() {
                let fill = if is_selected {
                    palette.gold_subtle
                } else {
                    palette.hover_fill
                };
                ui.painter().rect_filled(rect, theme::RADIUS_FIELD, fill);
            }
            if is_selected {
                ui.painter().rect_stroke(
                    rect,
                    theme::RADIUS_FIELD,
                    egui::Stroke::new(1.0, palette.gold_ring),
                    egui::StrokeKind::Inside,
                );
            }
            let count_rect = ui.painter().text(
                egui::pos2(rect.right() - 8.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                format!("{used} in use"),
                egui::FontId::proportional(theme::CAPTION),
                palette.text_secondary,
            );
            let painter = ui.painter().with_clip_rect(rect);
            let max_width = (count_rect.left() - rect.left() - 20.0).max(20.0);
            self.preview(style)
                .paint(&painter, rect.left() + 8.0, rect.center().y, max_width);
            if response.clicked() {
                self.selected = Some(style.id);
                self.rename = None;
            }
        }
    }

    /// The text of the rename box for `current`, starting from its name when another style was
    /// being renamed before.
    fn rename_text(&mut self, current: &LabelStyle) -> &mut String {
        if self.rename.as_ref().is_none_or(|(id, _)| *id != current.id) {
            self.rename = Some((current.id, current.name.clone()));
        }
        &mut self
            .rename
            .get_or_insert_with(|| (current.id, current.name.clone()))
            .1
    }

    fn editor_pane(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        styles: &[LabelStyle],
        current: &LabelStyle,
    ) {
        ui.set_min_width(300.0);
        ui.horizontal(|ui| {
            if ui
                .button("New")
                .on_hover_text("Copy the selected style")
                .clicked()
            {
                let names: Vec<&str> = styles.iter().map(|s| s.name.as_str()).collect();
                let copy = LabelStyle {
                    id: Id::new(),
                    name: free_name(&names, &current.name),
                    ..current.clone()
                };
                let id = copy.id;
                if editor
                    .run(Command::CreateLabelStyle { style: copy })
                    .is_ok()
                {
                    self.selected = Some(id);
                    self.rename = None;
                }
            }
            let deletable = current.id != LabelStyle::DEFAULT_ID;
            if ui
                .add_enabled(deletable, egui::Button::new("Delete"))
                .on_disabled_hover_text("The default style can't be deleted")
                .clicked()
            {
                let count = editor.map().labels_using(current.id).len();
                if editor
                    .run(Command::DeleteLabelStyle { id: current.id })
                    .is_ok()
                {
                    let default = editor.map().label_styles().default_style().name.clone();
                    editor.info(deleted_notice(&current.name, count, &default));
                }
            }
        });
        ui.add_space(6.0);

        let name = self.rename_text(current);
        let name_field = ui
            .horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(name)
            })
            .inner;
        if name_field.lost_focus() && *name != current.name {
            if editor
                .run(Command::RenameLabelStyle {
                    id: current.id,
                    name: name.clone(),
                })
                .is_err()
            {
                *name = current.name.clone();
            }
        } else if !name_field.has_focus() && *name != current.name {
            *name = current.name.clone();
        }
        ui.add_space(6.0);

        let mut next = current.clone();
        let mut edit = Edit::default();
        let mut merge = |e: Edit| {
            edit.began |= e.began;
            edit.changed |= e.changed;
            edit.finished |= e.finished;
        };

        ui.horizontal(|ui| {
            ui.label("Font");
            let mut picked = false;
            egui::ComboBox::from_id_salt("style font")
                .selected_text(&next.font.family)
                .show_ui(ui, |ui| {
                    for family in LABEL_FAMILIES {
                        picked |= ui
                            .selectable_value(&mut next.font.family, (*family).to_owned(), *family)
                            .clicked();
                    }
                });
            if picked {
                merge(instant());
            }
        });
        let mut weight = f64::from(next.font.weight);
        let spec = ScrubSpec::new("Weight").range(100.0, 900.0).step(100.0);
        merge(number_field(ui, &mut weight, &spec));
        next.font.weight = weight.round() as u16;
        if checkbox_row(ui, "Italic", &mut next.font.italic) {
            merge(instant());
        }
        let spec = ScrubSpec::new("Size")
            .range(0.1, 10_000.0)
            .slider_max(500.0)
            .logarithmic()
            .step(0.5)
            .decimals(1);
        merge(slider(ui, &mut next.size, &spec));
        let spec = ScrubSpec::new("Letter spacing")
            .unit(" em")
            .range(-1.0, 5.0)
            .slider_max(1.0)
            .default_value(0.0)
            .step(0.005)
            .decimals(3);
        merge(slider(ui, &mut next.letter_spacing, &spec));
        let spec = ScrubSpec::new("Line height")
            .unit("×")
            .range(0.1, 10.0)
            .slider_max(3.0)
            .step(0.01)
            .decimals(2);
        merge(slider(ui, &mut next.line_height, &spec));
        let mut percent = next.arch * 100.0;
        let spec = ScrubSpec::new("Arch")
            .unit("%")
            .range(-100.0, 100.0)
            .default_value(0.0)
            .decimals(0);
        merge(slider(ui, &mut percent, &spec));
        next.arch = (percent / 100.0).clamp(-1.0, 1.0);
        if checkbox_row(ui, "All caps", &mut next.all_caps) {
            merge(instant());
        }

        let mut fill = Some(next.fill);
        merge(swatch(ui, "Fill", &mut fill, false, Provenance::Plain).edit);
        next.fill = fill.unwrap_or(next.fill);

        let mut percent = f64::from(next.opacity) * 100.0;
        let spec = ScrubSpec::new("Opacity").unit("%").range(0.0, 100.0);
        merge(slider(ui, &mut percent, &spec));
        next.opacity = (percent / 100.0).clamp(0.0, 1.0) as f32;

        let spec = ScrubSpec::new("Halo")
            .range(0.0, 1_000.0)
            .slider_max(100.0)
            .default_value(0.0)
            .step(0.1)
            .decimals(1);
        merge(slider(ui, &mut next.halo.width, &spec));
        let mut halo = Some(next.halo.color);
        merge(swatch(ui, "Halo colour", &mut halo, false, Provenance::Plain).edit);
        next.halo.color = halo.unwrap_or(next.halo.color);

        let changed = next != *current;
        if changed || edit.finished || edit.began {
            let edit = Edit { changed, ..edit };
            self.live.apply(editor, edit, "Edit Label Style", || {
                Command::SetLabelStyle { style: next }
            });
        }
    }
}

/// A one-shot edit: a click on a checkbox or a pick from a list.
fn instant() -> Edit {
    Edit {
        began: true,
        changed: true,
        finished: true,
        reset: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable};
    use ssmap_core::{Canvas, DistanceUnit, Label, Map, Scale};

    #[test]
    fn copies_get_a_free_name() {
        assert_eq!(free_name(&["Town"], "Town"), "Town copy");
        assert_eq!(free_name(&["Town", "Town copy"], "Town"), "Town copy 2");
        assert_eq!(
            free_name(&["Town", "Town copy", "Town copy 2"], "Town"),
            "Town copy 3"
        );
    }

    #[test]
    fn the_delete_notice_names_the_count() {
        assert!(deleted_notice("Town", 1, "Default").contains("1 Label moved"));
        assert!(deleted_notice("Town", 1000, "Default").contains("1000 Labels moved"));
    }

    struct Fixture {
        editor: Editor,
        panel: LabelStylesPanel,
    }

    /// A Map with a style "Realm" and one Label in it, and the window open on that style.
    fn fixture() -> (Fixture, Id, Id) {
        let mut editor = Editor::new(Map::new(Canvas::new(
            100,
            100,
            Scale::new(1.0, DistanceUnit::Miles),
        )));
        let layer = editor.new_layer().expect("a Layer");
        let style = LabelStyle::new(Id::new(), "Realm");
        let style_id = style.id;
        assert!(editor.run(Command::CreateLabelStyle { style }).is_ok());
        let label_id = Id::new();
        assert!(
            editor
                .run(Command::CreateLabel {
                    id: label_id,
                    parent: layer,
                    index: None,
                    label: Box::new(Label {
                        text: "Valdren".into(),
                        style: style_id,
                        ..Label::default()
                    }),
                })
                .is_ok()
        );
        let panel = LabelStylesPanel {
            open: true,
            selected: Some(style_id),
            ..LabelStylesPanel::default()
        };
        (Fixture { editor, panel }, style_id, label_id)
    }

    fn harness(fixture: Fixture) -> Harness<'static, Fixture> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(760.0, 560.0))
            .build_ui_state(
                |ui, fixture: &mut Fixture| {
                    let ctx = ui.ctx().clone();
                    theme::install(&ctx);
                    fixture.panel.show(&ctx, &mut fixture.editor);
                },
                fixture,
            );
        harness.run();
        harness
    }

    #[test]
    fn renaming_a_style_in_the_window_updates_the_label_using_it() {
        let (fixture, style_id, label_id) = fixture();
        let mut harness = harness(fixture);

        // The Name box is the first text field in the window; the scrub fields follow it.
        fn name_box<'a>(harness: &'a Harness<'_, Fixture>) -> egui_kittest::Node<'a> {
            harness
                .get_all_by_role(Role::TextInput)
                .next()
                .expect("the Name box")
        }
        name_box(&harness).focus();
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        name_box(&harness).type_text("Kingdom");
        harness.key_press(egui::Key::Enter);
        harness.run();

        let editor = &harness.state().editor;
        assert_eq!(
            editor.map().label_styles().get(style_id).unwrap().name,
            "Kingdom"
        );
        assert_eq!(editor.label(label_id).unwrap().style, style_id);
        assert_eq!(editor.map().labels_using(style_id), vec![label_id]);
        harness.get_by_label("Kingdom");
    }

    #[test]
    fn the_default_style_cannot_be_deleted_and_new_copies_the_selection() {
        let (mut fixture, style_id, _) = fixture();
        fixture.panel.selected = Some(LabelStyle::DEFAULT_ID);
        let mut harness = harness(fixture);
        assert!(
            harness
                .get_by_label("Delete")
                .accesskit_node()
                .is_disabled()
        );

        harness.get_by_label("New").click();
        harness.run();
        let styles = &harness.state().editor.map();
        assert_eq!(styles.label_styles().len(), 3);
        assert!(styles.label_styles().get(style_id).is_some());
    }
}
