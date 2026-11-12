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
        let existing = self.water_layer();
        let mut planned = land::brush_command(
            &self.map,
            self.caches.roughness(),
            &self.selection,
            existing,
            stroke.clone(),
            new_id,
        );
        if planned == Err(land::BrushError::NoLayer) {
            let Some(layer) = self.create_stroke_layer(&id) else {
                self.cancel_gesture(id);
                return;
            };
            planned = land::brush_command(
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
            let layer = self.water_layer();
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

    /// What a paint stroke started now would be called in the undo history.
    pub fn paint_label(erase: bool) -> &'static str {
        if erase { "Erase Paint" } else { "Paint" }
    }

    /// Finishes the paint stroke whose gesture `id` opened: one `AddPaintStroke` Command with the
    /// brush's settings, committed as the gesture's Undo step. A stroke the Command refuses
    /// leaves no Undo step, and the status bar says why.
    pub fn finish_paint_stroke(&mut self, id: GestureId, points: Vec<Point>, erase: bool) {
        let brush = self.paint;
        let stroke = PaintStroke {
            id: Id::new(),
            points,
            color: brush.color,
            size: brush.size,
            softness: brush.softness as f32,
            opacity: brush.opacity as f32,
            erase,
        };
        if self.gesture(&id, Command::AddPaintStroke { stroke }) {
            self.end_gesture(id);
        } else {
            self.cancel_gesture(id);
        }
    }

    /// What a water brush stroke would be called in the undo history.
    pub fn water_label(erase: bool) -> &'static str {
        if erase { "Erase Water" } else { "Paint Water" }
    }

    /// Creates a plain Layer inside the gesture `id` for the first stroke on a Map with none,
    /// and makes it the active Layer.
    fn create_stroke_layer(&mut self, id: &GestureId) -> Option<Id> {
        let layer = Id::new();
        let command = Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: self.map.next_name("Layer"),
        };
        let created = self.gesture(id, command);
        if created {
            self.active_layer = Some(layer);
        }
        created.then_some(layer)
    }

    /// The Layer the brushes paint into: where new Objects go, if the Map has a Layer.
    pub(super) fn water_layer(&self) -> Option<Id> {
        self.placement_layer_or_new(Placing::Land).ok()
    }

    /// Finishes the water brush stroke whose gesture `id` opened, as [`Self::finish_brush_stroke`]
    /// does for land: one Command ([`land::water_command`]) as the gesture's Undo step. The
    /// first stroke on a Map with no Layer also creates a plain Layer, in the same Undo step.
    /// A new Lake becomes the
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
            let Some(layer) = self.create_stroke_layer(&id) else {
                self.cancel_gesture(id);
                return;
            };
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
