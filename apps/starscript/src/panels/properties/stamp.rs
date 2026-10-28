//! The Stamp section (design B in `docs/design/ui-handoff/mockups/StampProps.html`): the
//! breadcrumb with the lock and the Asset card, then Transform and Appearance. It shows one
//! Stamp or several; a field where the selected Stamps differ reads "Mixed", and an edit sets
//! every one of them, as one Undo step.

use eframe::egui;
use ssmap_core::{AssetRef, Color, Command, Id, Landmass, Map, Node, ObjectKind, Stamp};

use super::PropertiesPanel;
use crate::asset_browser::{ThumbArt, load_art};
use crate::editor::Editor;
use crate::icons::{self, Icon};
use crate::layer_tree::node_name;
use crate::prop_widgets::{self, Edit, Handle, Provenance, ScrubSpec, SectionHeader};
use crate::theme;

/// The colour a Stamp's line art takes when Tint is first chosen.
const DEFAULT_TINT: Color = Color::rgb(0x7A, 0x2E, 0x2E);

/// Side of the Asset card's thumbnail.
const THUMB: f32 = 40.0;

/// A selected Stamp: its id and its settings.
pub(super) struct Member {
    id: Id,
    stamp: Stamp,
}

/// The selected Stamps, or `None` unless `ids` is not empty and every one is a Stamp.
pub(super) fn stamps(map: &Map, ids: &[Id]) -> Option<Vec<Member>> {
    if ids.is_empty() {
        return None;
    }
    ids.iter()
        .map(|&id| match map.node(id) {
            Some(Node::Object(object)) => match &object.kind {
                ObjectKind::Stamp(stamp) => Some(Member {
                    id,
                    stamp: stamp.clone(),
                }),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// How a Stamp is coloured: the options of the Colour control.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Colouring {
    /// The art as drawn.
    Original,
    /// The line art multiplied by a colour.
    Tint,
    /// The Silhouette filled with a colour of its own.
    Fill,
}

impl Colouring {
    const ALL: [Colouring; 3] = [Colouring::Original, Colouring::Tint, Colouring::Fill];
    const LABELS: [&'static str; 3] = ["Original", "Tint", "Fill"];

    fn of(stamp: &Stamp) -> Self {
        if stamp.tint.is_some() {
            Colouring::Tint
        } else if stamp.fill.is_some() {
            Colouring::Fill
        } else {
            Colouring::Original
        }
    }

    /// Gives `stamp` this colouring, keeping a colour it already has for it.
    fn apply(self, stamp: &mut Stamp) {
        match self {
            Colouring::Original => {
                stamp.tint = None;
                stamp.fill = None;
            }
            Colouring::Tint => {
                stamp.tint = Some(stamp.tint.unwrap_or(DEFAULT_TINT));
                stamp.fill = None;
            }
            Colouring::Fill => {
                stamp.fill = Some(stamp.fill.unwrap_or(Landmass::DEFAULT_FILL));
                stamp.tint = None;
            }
        }
    }
}

/// Mirrors the art top to bottom about the Anchor: a left-right flip turned half a turn.
fn flip_vertical(stamp: &mut Stamp) {
    stamp.flip = !stamp.flip;
    let turned = (stamp.rotation + 180.0).rem_euclid(360.0);
    stamp.rotation = if turned > 180.0 {
        turned - 360.0
    } else {
        turned
    };
}

/// Whether the setting `field` reads differs between the selected Stamps.
fn mixed(members: &[Member], field: impl Fn(&Stamp) -> f64) -> bool {
    members
        .split_first()
        .is_some_and(|(first, rest)| rest.iter().any(|m| field(&m.stamp) != field(&first.stamp)))
}

impl PropertiesPanel {
    /// The Stamp section for `members`, which is never empty.
    pub(super) fn stamp_properties(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        members: &[Member],
    ) {
        let Some(first) = members.first() else {
            return;
        };
        let ids: Vec<Id> = members.iter().map(|m| m.id).collect();
        let (title, noun) = match members {
            [single] => (node_name(editor.map(), single.id), "Stamp"),
            _ => (format!("{} Stamps", members.len()), "Stamps"),
        };
        self.object_header(ui, editor, &ids, &title, noun);
        if let [single] = members {
            self.asset_card(ui, editor, &single.stamp.asset);
        }
        ui.add_space(theme::SPACING[2]);
        let first = first.stamp.clone();
        self.stamp_transform(ui, editor, members, &first);
        self.stamp_appearance(ui, editor, members, &first);
    }

    /// The Asset's thumbnail, name and variant, with Shuffle variant and Replace Asset.
    fn asset_card(&mut self, ui: &mut egui::Ui, editor: &mut Editor, asset: &AssetRef) {
        let palette = theme::palette(ui.ctx());
        let (name, subtitle) = editor.asset_card(asset);
        if self
            .stamp_thumb
            .as_ref()
            .is_none_or(|(shown, _)| shown != asset)
        {
            let art = editor
                .sprites()
                .and_then(|sprites| load_art(ui.ctx(), sprites, asset));
            self.stamp_thumb = Some((asset.clone(), art));
        }
        let art = self.stamp_thumb.as_ref().and_then(|(_, art)| art.clone());
        let mut shuffle = false;
        egui::Frame::NONE
            .outer_margin(egui::Margin::symmetric(theme::SPACING[3] as i8, 0))
            .inner_margin(egui::Margin::same(6))
            .fill(palette.bg_raised)
            .stroke(egui::Stroke::new(1.0, palette.border_subtle))
            .corner_radius(theme::RADIUS_BUTTON)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    paint_thumb(ui, art);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(
                            egui::RichText::new(name)
                                .size(theme::BODY)
                                .strong()
                                .color(palette.text_primary),
                        );
                        ui.label(
                            egui::RichText::new(subtitle)
                                .size(theme::CAPTION)
                                .color(palette.text_tertiary),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::icon_button(ui, Icon::Replace, "Replace Asset").clicked() {
                            self.open_assets = true;
                        }
                        shuffle = ui
                            .add_enabled_ui(editor.can_shuffle(), |ui| {
                                icons::icon_button(ui, Icon::Shuffle, "Shuffle variant")
                            })
                            .inner
                            .clicked();
                    });
                });
            });
        if shuffle {
            editor.shuffle_variants();
        }
    }

    /// X, Y, Scale and Rotation in a 2 × 2 grid, then Flip and the board.
    fn stamp_transform(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        members: &[Member],
        first: &Stamp,
    ) {
        let header = SectionHeader {
            kind: "transform",
            title: "Transform",
            badge: None,
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            let spec = |label, handle| {
                ScrubSpec::new(label)
                    .handle(Handle::Text(handle))
                    .step(0.5)
                    .decimals(1)
            };
            let (mut x, mut y) = (first.position[0], first.position[1]);
            let (x_edit, y_edit) = prop_widgets::pair(
                ui,
                |ui| {
                    let spec = spec("X", "X").mixed(mixed(members, |s| s.position[0]));
                    prop_widgets::scrub_field(ui, &mut x, &spec)
                },
                |ui| {
                    let spec = spec("Y", "Y").mixed(mixed(members, |s| s.position[1]));
                    prop_widgets::scrub_field(ui, &mut y, &spec)
                },
            );
            self.stamps_apply(editor, members, x_edit, |s| s.position[0] = x);
            self.stamps_apply(editor, members, y_edit, |s| s.position[1] = y);

            let mut percent = first.scale * 100.0;
            let mut rotation = first.rotation;
            let (scale_edit, rotation_edit) = prop_widgets::pair(
                ui,
                |ui| {
                    let spec = ScrubSpec::new("Scale")
                        .handle(Handle::Text("S"))
                        .unit("%")
                        .range(1.0, 10_000.0)
                        .mixed(mixed(members, |s| s.scale));
                    prop_widgets::scrub_field(ui, &mut percent, &spec)
                },
                |ui| {
                    let spec = ScrubSpec::new("Rotation")
                        .handle(Handle::Icon(Icon::Rotate))
                        .unit("°")
                        .step(0.5)
                        .decimals(1)
                        .mixed(mixed(members, |s| s.rotation));
                    prop_widgets::scrub_field(ui, &mut rotation, &spec)
                },
            );
            self.stamps_apply(editor, members, scale_edit, |s| s.scale = percent / 100.0);
            self.stamps_apply(editor, members, rotation_edit, |s| s.rotation = rotation);

            let flipped = members.iter().all(|m| m.stamp.flip);
            let (mut horizontal, mut vertical) = (false, false);
            prop_widgets::field_row(ui, "Flip", Provenance::Plain, |ui| {
                horizontal =
                    prop_widgets::icon_toggle(ui, Icon::FlipHorizontal, "Flip horizontal", flipped);
                vertical = prop_widgets::icon_action(ui, Icon::FlipVertical, "Flip vertical");
            });
            if horizontal {
                self.stamps_apply(editor, members, Edit::instant(), |s| s.flip = !flipped);
            } else if vertical {
                self.stamps_apply(editor, members, Edit::instant(), flip_vertical);
            }

            self.membership_row(ui, editor);
        });
    }

    /// Colour (Original, Tint or Fill, with the colour of the one chosen) and Opacity.
    fn stamp_appearance(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        members: &[Member],
        first: &Stamp,
    ) {
        let header = SectionHeader {
            kind: "appearance",
            title: "Appearance",
            badge: None,
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            let colouring = Colouring::of(first);
            let same = members.iter().all(|m| Colouring::of(&m.stamp) == colouring);
            // Nothing is selected while the Stamps differ.
            let mut chosen = if same {
                Colouring::ALL
                    .iter()
                    .position(|&c| c == colouring)
                    .unwrap_or(0)
            } else {
                usize::MAX
            };
            let mut changed = false;
            prop_widgets::field_row(ui, "Colour", Provenance::Plain, |ui| {
                changed = prop_widgets::segmented(ui, &Colouring::LABELS, &mut chosen);
            });
            if changed && let Some(&to) = Colouring::ALL.get(chosen) {
                self.stamps_apply(editor, members, Edit::instant(), |s| to.apply(s));
            } else if same {
                if let Some(tint) = first.tint {
                    let mut colour = Some(tint);
                    let response =
                        prop_widgets::swatch_unlabelled(ui, "Tint colour", &mut colour, false);
                    let tint = colour.unwrap_or(tint);
                    self.stamps_apply(editor, members, response.edit, |s| s.tint = Some(tint));
                }
                if let Some(fill) = first.fill {
                    let mut colour = Some(fill);
                    let response =
                        prop_widgets::swatch_unlabelled(ui, "Fill colour", &mut colour, false);
                    let fill = colour.unwrap_or(fill);
                    self.stamps_apply(editor, members, response.edit, |s| s.fill = Some(fill));
                }
            }

            let mut percent = f64::from(first.opacity) * 100.0;
            let spec = ScrubSpec::new("Opacity")
                .unit("%")
                .range(0.0, 100.0)
                .mixed(mixed(members, |s| f64::from(s.opacity)));
            let edit = prop_widgets::scrub_field(ui, &mut percent, &spec);
            let opacity = (percent / 100.0).clamp(0.0, 1.0) as f32;
            self.stamps_apply(editor, members, edit, |s| s.opacity = opacity);
        });
    }

    /// Applies an edit to every selected Stamp, each keeping its other settings. A scrub is
    /// one Undo step, a click one step.
    fn stamps_apply(
        &mut self,
        editor: &mut Editor,
        members: &[Member],
        edit: Edit,
        apply: impl Fn(&mut Stamp),
    ) {
        self.live.apply_all(editor, edit, "Edit Stamp", || {
            members
                .iter()
                .map(|m| {
                    let mut stamp = m.stamp.clone();
                    apply(&mut stamp);
                    Command::SetStamp {
                        id: m.id,
                        stamp: Box::new(stamp),
                    }
                })
                .collect()
        });
    }
}

/// The Asset's art on its tile, fitted inside.
fn paint_thumb(ui: &mut egui::Ui, art: Option<ThumbArt>) {
    let palette = theme::palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(THUMB), egui::Sense::hover());
    let fill = match &art {
        Some(art) if art.light => palette.thumb_bg_light_art,
        _ => palette.thumb_bg,
    };
    ui.painter().rect_filled(rect, theme::RADIUS_FIELD, fill);
    let Some(ThumbArt { texture, .. }) = art else {
        return;
    };
    let inner = rect.shrink(4.0);
    let size = texture.size_vec2();
    let scale = (inner.width() / size.x).min(inner.height() / size.y);
    let fitted = egui::Rect::from_center_size(inner.center(), size * scale);
    ui.painter().image(
        texture.id(),
        fitted,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colouring_follows_the_tint_then_the_fill() {
        let mut stamp = Stamp::default();
        assert_eq!(Colouring::of(&stamp), Colouring::Original);
        stamp.fill = Some(Landmass::DEFAULT_FILL);
        assert_eq!(Colouring::of(&stamp), Colouring::Fill);
        stamp.tint = Some(DEFAULT_TINT);
        assert_eq!(Colouring::of(&stamp), Colouring::Tint);
    }

    #[test]
    fn choosing_a_colouring_clears_the_other_colour_and_keeps_its_own() {
        let red = Color::rgb(200, 0, 0);
        let mut stamp = Stamp {
            tint: Some(red),
            ..Stamp::default()
        };
        Colouring::Tint.apply(&mut stamp);
        assert_eq!(stamp.tint, Some(red));
        Colouring::Fill.apply(&mut stamp);
        assert_eq!(
            (stamp.tint, stamp.fill),
            (None, Some(Landmass::DEFAULT_FILL))
        );
        Colouring::Original.apply(&mut stamp);
        assert_eq!((stamp.tint, stamp.fill), (None, None));
        Colouring::Tint.apply(&mut stamp);
        assert_eq!(stamp.tint, Some(DEFAULT_TINT));
    }

    #[test]
    fn a_vertical_flip_is_a_mirror_turned_half_a_turn_and_undoes_itself() {
        let mut stamp = Stamp {
            rotation: 30.0,
            ..Stamp::default()
        };
        flip_vertical(&mut stamp);
        assert!(stamp.flip);
        assert_eq!(stamp.rotation, -150.0);
        flip_vertical(&mut stamp);
        assert!(!stamp.flip);
        assert_eq!(stamp.rotation, 30.0);
    }
}
