use super::*;

impl Editor {
    /// What a land brush stroke would be called in the undo history. The land brush only
    /// makes or changes Landmasses, so it is always land.
    pub fn brush_label(&self, erase: bool) -> &'static str {
        if erase { "Erase Land" } else { "Paint Land" }
    }

    /// Finishes the land brush stroke whose gesture `id` opened: runs the one Command the brush
    /// rules choose ([`land::brush_command`]) and commits it as the gesture's Undo step. A new
    /// Landmass becomes the Selection, so the next stroke merges into it. A stroke that changes
    /// nothing, or is refused, leaves no Undo step.
    pub fn finish_brush_stroke(&mut self, id: GestureId, stroke: BrushStroke) {
        let new_id = Id::new();
        let group = self.placement_parent(Placing::Land);
        let planned = land::brush_command(
            &self.map,
            self.caches.roughness(),
            &self.selection,
            group,
            stroke,
            new_id,
        );
        self.commit_stroke(id, planned, new_id);
    }

    /// Runs a brush stroke's planned Command as the gesture `id`'s Undo step, and selects the
    /// Object it creates, if any, so the next stroke extends it. Nothing planned, or a refused
    /// Command, leaves no Undo step.
    fn commit_stroke(&mut self, id: GestureId, planned: Option<Command>, new_id: Id) {
        let Some(command) = planned else {
            self.cancel_gesture(id);
            return;
        };
        let creates = matches!(
            command,
            Command::CreateLandmass { .. } | Command::CreateLake { .. }
        );
        if self.gesture(&id, command) {
            if creates {
                self.select(vec![new_id]);
            }
            self.end_gesture(id);
        } else {
            self.cancel_gesture(id);
        }
    }

    /// The area, with its rough shoreline, that finishing `stroke` now would add or cut away,
    /// for the live preview. It plans the same Command a release would run
    /// ([`land::brush_command`], or [`land::water_command`]), so the two can't disagree. `None`
    /// when the stroke changes nothing.
    pub fn brush_preview(&mut self, stroke: BrushStroke, water: bool) -> Option<MultiPolygon> {
        let new_id = Id::new();
        let group = self.placement_parent(Placing::Land);
        let planned = if water {
            land::water_command(
                &self.map,
                self.caches.roughness(),
                &self.selection,
                group,
                stroke,
                new_id,
            )
        } else {
            land::brush_command(
                &self.map,
                self.caches.roughness(),
                &self.selection,
                group,
                stroke,
                new_id,
            )
        }?;
        land::preview_area(&self.map, self.caches.roughness(), &planned)
    }

    /// What a paint stroke started now would be called in the undo history.
    pub fn paint_label(erase: bool) -> &'static str {
        if erase { "Erase Paint" } else { "Paint" }
    }

    /// Finishes the paint stroke whose gesture `id` opened: one `AddPaintStroke` Command with the
    /// brush's settings, committed as the gesture's Undo step. A stroke the Command refuses
    /// leaves no Undo step, and the status bar says why. An eraser that meets no paint changes
    /// nothing, so it leaves no Undo step either (issue #710).
    pub fn finish_paint_stroke(&mut self, id: GestureId, points: Vec<Point>, erase: bool) {
        let settings = *self.paint_settings(erase);
        let stroke = PaintStroke {
            id: Id::new(),
            points,
            color: self.paint_color(),
            size: settings.size,
            softness: settings.softness as f32,
            opacity: settings.opacity as f32,
            erase,
        };
        if erase && !ssmap_core::erases_any(self.map.paint_strokes(), &stroke) {
            self.cancel_gesture(id);
            return;
        }
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

    /// Finishes the water brush stroke whose gesture `id` opened, as [`Self::finish_brush_stroke`]
    /// does for land: one Command ([`land::water_command`]) as the gesture's Undo step. With no
    /// Group active, a new Lake goes at the top level. A new Lake becomes the Selection, so the
    /// next stroke extends it.
    pub fn finish_water_stroke(&mut self, id: GestureId, stroke: BrushStroke) {
        let new_id = Id::new();
        let group = self.placement_parent(Placing::Land);
        let planned = land::water_command(
            &self.map,
            self.caches.roughness(),
            &self.selection,
            group,
            stroke,
            new_id,
        );
        self.commit_stroke(id, planned, new_id);
    }
}
