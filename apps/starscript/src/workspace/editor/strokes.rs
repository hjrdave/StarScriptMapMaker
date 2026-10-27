use super::*;

impl Editor {
    /// What a land brush stroke started now would be called in the undo history: it paints or
    /// erases a selected Lake, and otherwise land.
    pub fn brush_label(&self, erase: bool) -> &'static str {
        let lake = self.selection.iter().any(|&id| {
            matches!(self.map.node(id), Some(Node::Object(o)) if matches!(o.kind, ObjectKind::Lake(_)))
        });
        match (erase, lake) {
            (false, false) => "Paint Land",
            (false, true) => "Paint Lake",
            (true, false) => "Erase Land",
            (true, true) => "Erase Lake",
        }
    }

    /// Finishes the land brush stroke whose gesture `id` opened: runs the one Command the brush
    /// rules choose ([`land::brush_command`]) and commits it as the gesture's Undo step. A new
    /// Landmass becomes the Selection, so the next stroke merges into it. A stroke that changes
    /// nothing, or is refused, leaves no Undo step.
    pub fn finish_brush_stroke(&mut self, id: GestureId, stroke: BrushStroke) {
        let new_id = Id::new();
        let layer = self.target_layer(Placing::Land);
        let planned = land::brush_command(
            &self.map,
            self.caches.roughness(),
            &self.selection,
            layer,
            stroke,
            new_id,
        );
        match planned {
            Ok(Some(command)) => {
                let creates = matches!(command, Command::CreateLandmass { .. });
                if self.gesture(&id, command) {
                    if creates {
                        self.select(vec![new_id]);
                    }
                    self.end_gesture(id);
                } else {
                    self.cancel_gesture(id);
                }
            }
            Ok(None) => {
                self.cancel_gesture(id);
            }
            Err(err) => {
                self.cancel_gesture(id);
                let text = err.to_string();
                let mut chars = text.chars();
                let first = chars.next().map(|c| c.to_uppercase().to_string());
                self.error(first.unwrap_or_default() + chars.as_str());
            }
        }
    }

    /// The area, with its rough shoreline, that finishing `stroke` now would add or cut away,
    /// for the live preview. It plans the same Command a release would run
    /// ([`land::brush_command`], or [`land::water_command`]), so the two can't disagree. `None`
    /// when the stroke changes nothing.
    pub fn brush_preview(&mut self, stroke: BrushStroke, water: bool) -> Option<MultiPolygon> {
        let new_id = Id::new();
        let (add, region) = (!stroke.erase, stroke.region.clone());
        let planned = if water {
            let layer = self.water_layer();
            land::water_command(
                &self.map,
                self.caches.roughness(),
                &self.selection,
                layer,
                stroke,
                new_id,
            )
        } else {
            let layer = self.target_layer(Placing::Land);
            land::brush_command(
                &self.map,
                self.caches.roughness(),
                &self.selection,
                layer,
                stroke,
                new_id,
            )
        };
        match planned {
            Ok(Some(command)) => land::preview_area(&self.map, self.caches.roughness(), &command),
            // A first water stroke has no Layer yet; it will create one and a Lake in it.
            Err(_) if add => land::preview_area(
                &self.map,
                self.caches.roughness(),
                &Command::CreateLake {
                    id: new_id,
                    parent: new_id,
                    index: None,
                    geometry: region,
                    fill: None,
                },
            ),
            _ => None,
        }
    }

    /// What a water brush stroke would be called in the undo history.
    pub fn water_label(erase: bool) -> &'static str {
        if erase { "Erase Water" } else { "Paint Water" }
    }

    /// The top-level Layer the water brush paints into: the Map's "Water" Layer, if it has one.
    pub(super) fn water_layer(&self) -> Option<Id> {
        self.map.children(None).iter().copied().find(|&id| {
            matches!(self.map.node(id), Some(Node::Layer(layer)) if layer.name == WATER_LAYER)
        })
    }

    /// Finishes the water brush stroke whose gesture `id` opened, as [`Self::finish_brush_stroke`]
    /// does for land: one Command ([`land::water_command`]) as the gesture's Undo step. The
    /// first stroke that needs it also creates the "Water" Layer, in the same Undo step, so
    /// water has its own Layer to rename, lock, hide and reorder. A new Lake becomes the
    /// Selection, so the next stroke extends it.
    pub fn finish_water_stroke(&mut self, id: GestureId, stroke: BrushStroke) {
        let new_id = Id::new();
        let existing = self.water_layer();
        let mut planned = land::water_command(
            &self.map,
            self.caches.roughness(),
            &self.selection,
            existing,
            stroke.clone(),
            new_id,
        );
        if planned == Err(land::BrushError::NoLayer) {
            let layer = Id::new();
            let created = self.gesture(
                &id,
                Command::CreateLayer {
                    id: layer,
                    parent: None,
                    index: None,
                    name: WATER_LAYER.to_owned(),
                },
            );
            if !created {
                self.cancel_gesture(id);
                return;
            }
            planned = land::water_command(
                &self.map,
                self.caches.roughness(),
                &self.selection,
                Some(layer),
                stroke,
                new_id,
            );
        }
        match planned {
            Ok(Some(command)) => {
                let creates = matches!(command, Command::CreateLake { .. });
                if self.gesture(&id, command) {
                    if creates {
                        self.select(vec![new_id]);
                    }
                    self.end_gesture(id);
                } else {
                    self.cancel_gesture(id);
                }
            }
            Ok(None) => {
                self.cancel_gesture(id);
            }
            Err(err) => {
                self.cancel_gesture(id);
                self.error(err.to_string());
            }
        }
    }
}
