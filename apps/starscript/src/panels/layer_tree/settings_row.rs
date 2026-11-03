//! The settings row above the list: Opacity, Hide, Lock and Pin of what is selected (or of the
//! active Layer when nothing is). Opacity and Hide are for Layers; Lock and Pin also take
//! Objects. A row that holds a mix shows "Mixed" or the on state of all of them.

use ssmap_core::{Command, Id, Node};

use super::*;
use crate::icons::Icon;
use crate::prop_widgets::{self, Handle, ScrubSpec};

/// What the row acts on: the selection, else the active Layer.
fn targets(editor: &Editor) -> Vec<Id> {
    let map = editor.map();
    let picked: Vec<Id> = editor
        .selection()
        .iter()
        .copied()
        .filter(|&id| map.contains(id))
        .collect();
    if picked.is_empty() {
        editor
            .active_layer()
            .filter(|&id| map.contains(id))
            .into_iter()
            .collect()
    } else {
        picked
    }
}

impl LayerTree {
    /// Draws the row and applies what the user did to it.
    pub(super) fn settings_row(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let palette = theme::palette(ui.ctx());
        let ids = targets(editor);
        let map = editor.map();
        let layers: Vec<Id> = ids
            .iter()
            .copied()
            .filter(|&id| map.node(id).is_some_and(Node::is_layer))
            .collect();
        // A lock from a Layer above cannot be changed on the node; it is left out.
        let writable: Vec<Id> = ids
            .iter()
            .copied()
            .filter(|&id| map.locked_ancestor(id).is_none())
            .collect();
        let opacities: Vec<f32> = layers
            .iter()
            .filter_map(|&id| match map.node(id) {
                Some(Node::Layer(layer)) => Some(layer.opacity),
                _ => None,
            })
            .collect();
        let hidden = !layers.is_empty() && layers.iter().all(|&id| !visible(map, id));
        let locked = !writable.is_empty() && writable.iter().all(|&id| node_locked(map, id));
        let pinned = !writable.is_empty() && writable.iter().all(|&id| pin_on(map, id));
        let any_locked = writable.iter().any(|&id| node_locked(map, id));

        let mut percent = opacities.first().map_or(100.0, |&o| f64::from(o) * 100.0);
        let mixed = opacities
            .iter()
            .any(|&o| (o - opacities[0]).abs() > f32::EPSILON);
        let mut opacity_edit = Edit::default();
        let (mut hide_clicked, mut lock_clicked, mut pin_clicked) = (false, false, false);
        ui.add_space(theme::SPACING[2]);
        ui.horizontal(|ui| {
            ui.add_space(theme::SPACING[3]);
            ui.spacing_mut().item_spacing.x = theme::SPACING[1];
            ui.add_enabled_ui(!layers.is_empty(), |ui| {
                ui.allocate_ui(egui::vec2(96.0, metrics::FIELD[0]), |ui| {
                    opacity_edit = prop_widgets::scrub_field(
                        ui,
                        &mut percent,
                        &ScrubSpec::new("Opacity")
                            .handle(Handle::Text("O"))
                            .unit("%")
                            .range(0.0, 100.0)
                            .mixed(mixed),
                    );
                });
                hide_clicked = prop_widgets::icon_toggle(
                    ui,
                    if hidden { Icon::Hidden } else { Icon::Visible },
                    if hidden { "Show" } else { "Hide" },
                    hidden,
                );
            });
            ui.add_enabled_ui(!writable.is_empty(), |ui| {
                lock_clicked = prop_widgets::icon_toggle(
                    ui,
                    if locked { Icon::LockFilled } else { Icon::Lock },
                    if locked { "Unlock" } else { "Lock" },
                    locked,
                );
            });
            ui.add_enabled_ui(!writable.is_empty() && !any_locked, |ui| {
                pin_clicked = prop_widgets::icon_toggle(
                    ui,
                    if pinned { Icon::PinFilled } else { Icon::Pin },
                    if pinned {
                        "Unpin from Canvas"
                    } else {
                        "Pin to Canvas"
                    },
                    pinned,
                );
            });
        });
        ui.add_space(theme::SPACING[2]);
        ui.painter().hline(
            ui.max_rect().x_range(),
            ui.cursor().top(),
            egui::Stroke::new(1.0, palette.border_subtle),
        );

        // Opacity is a drag: live updates, one Undo step.
        let opacity = (percent / 100.0).clamp(0.0, 1.0) as f32;
        self.live
            .apply_all(editor, opacity_edit, "Change Opacity", || {
                layers
                    .iter()
                    .map(|&id| Command::SetOpacity { id, opacity })
                    .collect()
            });
        if hide_clicked {
            apply_toggle(editor, &ids, Toggle::Hide);
        }
        if lock_clicked {
            apply_toggle(editor, &ids, Toggle::Lock);
        }
        if pin_clicked {
            apply_toggle(editor, &ids, Toggle::Pin);
        }
    }
}

/// A flag the settings row and the right-click menu flip on several rows at once.
#[derive(Clone, Copy)]
pub(super) enum Toggle {
    Hide,
    Lock,
    Pin,
}

/// Which of some rows a [`Toggle`] reaches, and whether they all have it on already.
pub(super) struct ToggleState {
    /// The rows it changes: Hide takes Layers only; Lock and Pin skip a node locked from above.
    pub targets: Vec<Id>,
    /// Every target has it on (and there is at least one): the action turns it off.
    pub on: bool,
    /// Pin cannot change while any target is locked, as in the settings row.
    pub enabled: bool,
}

impl Toggle {
    /// The Undo label: "Hide", or "Show" when `on`; and so on.
    pub(super) fn verb(self, on: bool) -> &'static str {
        match (self, on) {
            (Self::Hide, false) => "Hide",
            (Self::Hide, true) => "Show",
            (Self::Lock, false) => "Lock",
            (Self::Lock, true) => "Unlock",
            (Self::Pin, false) => "Pin to Canvas",
            (Self::Pin, true) => "Unpin from Canvas",
        }
    }

    /// A menu label: the verb, naming the count when there are several ("Hide 6 items",
    /// "Pin 6 items to Canvas").
    pub(super) fn menu_label(self, state: &ToggleState) -> String {
        let n = state.targets.len();
        if n < 2 {
            return self.verb(state.on).to_owned();
        }
        match (self, state.on) {
            (Self::Hide, false) => format!("Hide {n} items"),
            (Self::Hide, true) => format!("Show {n} items"),
            (Self::Lock, false) => format!("Lock {n} items"),
            (Self::Lock, true) => format!("Unlock {n} items"),
            (Self::Pin, false) => format!("Pin {n} items to Canvas"),
            (Self::Pin, true) => format!("Unpin {n} items from Canvas"),
        }
    }
}

/// What `toggle` would do to `ids`.
pub(super) fn toggle_state(map: &ssmap_core::Map, ids: &[Id], toggle: Toggle) -> ToggleState {
    let targets: Vec<Id> = match toggle {
        Toggle::Hide => ids
            .iter()
            .copied()
            .filter(|&id| map.node(id).is_some_and(Node::is_layer))
            .collect(),
        Toggle::Lock | Toggle::Pin => ids
            .iter()
            .copied()
            .filter(|&id| map.locked_ancestor(id).is_none())
            .collect(),
    };
    let has = |id: Id| match toggle {
        Toggle::Hide => !visible(map, id),
        Toggle::Lock => node_locked(map, id),
        Toggle::Pin => pin_on(map, id),
    };
    let on = !targets.is_empty() && targets.iter().all(|&id| has(id));
    let enabled = !targets.is_empty()
        && (!matches!(toggle, Toggle::Pin) || !targets.iter().any(|&id| node_locked(map, id)));
    ToggleState {
        targets,
        on,
        enabled,
    }
}

/// Flips `toggle` on `ids` as one Undo step: off if they all have it on, else on.
pub(super) fn apply_toggle(editor: &mut Editor, ids: &[Id], toggle: Toggle) {
    let state = toggle_state(editor.map(), ids, toggle);
    if !state.enabled {
        return;
    }
    let on = !state.on;
    run_all(
        editor,
        toggle.verb(state.on),
        &state.targets,
        |id| match toggle {
            Toggle::Hide => Command::SetVisible { id, visible: !on },
            Toggle::Lock => Command::SetLocked { id, locked: on },
            Toggle::Pin => Command::SetMoveLocked { id, locked: on },
        },
    );
}

/// Whether the Layer itself is shown (its own eye, not one above it).
fn visible(map: &ssmap_core::Map, id: Id) -> bool {
    matches!(map.node(id), Some(Node::Layer(layer)) if layer.visible)
}

fn node_locked(map: &ssmap_core::Map, id: Id) -> bool {
    map.node(id).is_some_and(Node::locked)
}

/// A node's own pin (not one inherited from a Layer above).
fn pin_on(map: &ssmap_core::Map, id: Id) -> bool {
    match map.node(id) {
        Some(Node::Layer(layer)) => layer.pinned,
        Some(Node::Object(object)) => object.move_locked,
        None => false,
    }
}

/// Runs `command` for each of `ids` as one Undo step named `label`.
fn run_all(editor: &mut Editor, label: &str, ids: &[Id], command: impl Fn(Id) -> Command) {
    if ids.is_empty() {
        return;
    }
    let _ = editor.in_gesture(label, |gesture| {
        for &id in ids {
            if !gesture.run(command(id)) {
                return Err(());
            }
        }
        Ok(())
    });
}
