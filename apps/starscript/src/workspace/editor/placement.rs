use super::*;

impl Editor {
    /// The Objects directly on the active Layer.
    pub fn active_layer_objects(&self) -> Vec<Id> {
        let Some(layer) = self.active_layer.filter(|&id| self.map.contains(id)) else {
            return Vec::new();
        };
        self.map
            .children(Some(layer))
            .iter()
            .copied()
            .filter(|&id| matches!(self.map.node(id), Some(Node::Object(_))))
            .collect()
    }

    /// The Label with id `id`, if it is one.
    pub fn label(&self, id: Id) -> Option<&Label> {
        match self.map.node(id) {
            Some(Node::Object(object)) => match &object.kind {
                ObjectKind::Label(label) => Some(label),
                _ => None,
            },
            _ => None,
        }
    }

    /// A Label with its style applied, and its shaped text.
    pub fn label_layout(&mut self, id: Id) -> Option<(ResolvedLabel, Arc<LabelLayout>)> {
        let label = self.map.resolved_label(id)?;
        let layout = self.scene.label_layout(id, &label);
        Some((label, layout))
    }

    /// The topmost Object under a Canvas point that can be selected there: visible, and
    /// neither locked nor in a locked Layer.
    pub fn pick(&mut self, point: [f64; 2]) -> Option<Id> {
        // The index finds the topmost Landmass or Lake whose roughened fill is under the
        // point; Labels are weighed against it in draw order, so what shows on top is
        // what's picked.
        let outline = self.pick_outline(point);
        let stamp = self.pick_stamp(point);
        for id in self.map.draw_order().into_iter().rev() {
            if Some(id) == outline || Some(id) == stamp {
                return Some(id);
            }
            if !self.map.is_locked(id)
                && let Some((label, layout)) = self.label_layout(id)
                && layout.contains(&label, point)
            {
                return Some(id);
            }
        }
        None
    }

    /// Opens an "Add Label" gesture and places a Label at `position` in the active Layer,
    /// creating a Layer if the Map has none. The gesture stays open for typing the text.
    pub fn place_label(&mut self, position: [f64; 2]) -> Option<(Id, GestureId)> {
        self.end_open_gesture();
        let gesture = self.begin_gesture("Add Label");
        let Some(parent) = self.placement_layer(&gesture, Placing::Label) else {
            self.cancel_gesture(gesture);
            return None;
        };
        let id = Id::new();
        let label = Label {
            text: "Label".into(),
            position,
            ..Label::default()
        };
        if !self.gesture(
            &gesture,
            Command::CreateLabel {
                id,
                parent,
                index: None,
                label: Box::new(label),
            },
        ) {
            self.cancel_gesture(gesture);
            return None;
        }
        self.select(vec![id]);
        Some((id, gesture))
    }

    /// The credit lines for the attribution-required Assets the Map uses; empty when none, or
    /// when no packs are loaded.
    pub fn credit_lines(&self) -> Vec<String> {
        let Some(sprites) = self.caches.sprites() else {
            return Vec::new();
        };
        let assets = sprites.library().attribution_required_assets(&self.map);
        credit_lines(&assets)
    }

    /// Adds the credit lines to the Map as one left-aligned Label near the bottom-left corner
    /// of the Canvas, as one Undo step ("Add Credits Label"), and selects it. Returns `None`
    /// when the Map has nothing to credit.
    pub fn add_credits_label(&mut self) -> Option<Id> {
        let lines = self.credit_lines();
        if lines.is_empty() {
            return None;
        }
        let (width, height) = (
            f64::from(self.map.canvas().width),
            f64::from(self.map.canvas().height),
        );
        let size = (width.min(height) / 80.0).max(8.0);
        let line_height = 1.3;
        let margin = size * 2.0;
        let block = size * line_height * lines.len() as f64;
        let label = Label {
            text: lines.join("\n"),
            position: [margin, (height - margin - block / 2.0).max(block / 2.0)],
            align: Align::Left,
            size: Some(size),
            line_height: Some(line_height),
            ..Label::default()
        };
        let id = Id::new();
        self.end_open_gesture();
        self.in_gesture("Add Credits Label", |g| {
            let parent = g.placement_layer(Placing::Label).ok_or(())?;
            let created = g.run(Command::CreateLabel {
                id,
                parent,
                index: None,
                label: Box::new(label),
            });
            if !created {
                return Err(());
            }
            g.editor.select(vec![id]);
            Ok(())
        })?
        .ok()?;
        Some(id)
    }

    /// Places the selected Asset (or a random member of its Variant group) as a Stamp at
    /// `position` in the active Layer, as one Undo step, and selects it. Does nothing when no
    /// Asset is selected.
    pub fn place_stamp(&mut self, position: [f64; 2]) -> Option<Id> {
        let asset = self.placement_asset()?;
        let id = Id::new();
        self.end_open_gesture();
        self.in_gesture("Add Stamp", |g| {
            let parent = g.placement_layer(Placing::Stamp).ok_or(())?;
            let created = g.run(Command::CreateStamp {
                id,
                parent,
                index: None,
                stamp: Box::new(Stamp::new(asset, position)),
            });
            if !created {
                return Err(());
            }
            g.editor.select(vec![id]);
            Ok(())
        })?
        .ok()?;
        Some(id)
    }

    /// Finishes a scatter brush stroke: scatters Stamps of the selected Asset along `points` in
    /// the active Layer as one Undo step ([`scatter::scatter_command`]). Does nothing, with a
    /// message, when no Asset is chosen, and leaves no Undo step when nothing could be placed.
    pub fn scatter_stamps(&mut self, points: &[Point]) -> bool {
        let Some(selected) = self.selected_asset.clone() else {
            self.error("Choose an Asset in the Asset browser first");
            return false;
        };
        let variants = if self.scatter.random_variant {
            self.variant_group(&selected)
        } else {
            Vec::new()
        };
        let terrain = Terrain::of(&self.map, self.caches.roughness());
        self.end_open_gesture();
        let brush = self.scatter.clone();
        let radius = self.brush_size / 2.0;
        let placed = self.in_gesture("Scatter Stamps", |g| {
            let parent = g.placement_layer(Placing::Stamp).ok_or(())?;
            let base = Stamp::new(selected, [0.0, 0.0]);
            let command = scatter::scatter_command(
                &brush,
                ScatterStroke {
                    points,
                    radius,
                    parent,
                    base: &base,
                    variants: &variants,
                },
                &terrain,
                random_seed(),
                Id::new,
            );
            let command = command.ok_or(())?;
            if g.run(command) { Ok(()) } else { Err(()) }
        });
        matches!(placed, Some(Ok(())))
    }

    /// Finishes a Stamp eraser stroke: removes the Stamps in the active Layer whose Anchor is
    /// inside the brush, as one Undo step. Other Layers are never touched.
    pub fn erase_stamps(&mut self, points: &[Point]) -> bool {
        let Some(layer) = self.active_layer.filter(|&id| self.map.contains(id)) else {
            self.info("Select a Layer to erase Stamps from");
            return false;
        };
        match scatter::erase_command(&self.map, layer, points, self.brush_size / 2.0) {
            Some(command) => self.run(command).is_ok(),
            None => false,
        }
    }

    /// Whether the Selection holds a Stamp, so Shuffle variant has something to work on.
    pub fn can_shuffle(&self) -> bool {
        self.selection.iter().any(|&id| self.stamp(id).is_some())
    }

    /// Shuffle variant: each selected Stamp takes a random Asset from its own Variant group,
    /// all as one Undo step.
    pub fn shuffle_variants(&mut self) -> bool {
        let Some(library) = self.caches.sprites().map(SpriteStore::library) else {
            return false;
        };
        let commands = scatter::shuffle_commands(
            &self.map,
            &self.selection,
            |asset| group_of(&library, asset),
            random_seed(),
        );
        if commands.is_empty() {
            self.info("No selected Stamp has another variant to take");
            return false;
        }
        self.end_open_gesture();
        let shuffled = self.in_gesture("Shuffle Variant", |g| {
            for command in commands {
                if !g.run(command) {
                    return Err(());
                }
            }
            Ok(())
        });
        matches!(shuffled, Some(Ok(())))
    }

    /// The Asset's name and what to say under it: "Variant 2 of 3" when it belongs to a
    /// Variant group of two or more, else its pack's name. Its reference when the pack is
    /// missing.
    pub fn asset_card(&self, asset: &AssetRef) -> (String, String) {
        let library = self.caches.sprites().map(SpriteStore::library);
        let Some((pack, found)) = library.as_ref().and_then(|library| library.asset(asset)) else {
            return (asset.to_string(), "Pack missing".to_owned());
        };
        let group = self.variant_group(asset);
        let subtitle = match group.iter().position(|member| member == asset) {
            Some(index) if group.len() > 1 => format!("Variant {} of {}", index + 1, group.len()),
            _ => pack.name.clone(),
        };
        (found.name.clone(), subtitle)
    }

    /// The Assets in the Variant group `asset` belongs to; empty when it has none.
    pub(super) fn variant_group(&self, asset: &AssetRef) -> Vec<AssetRef> {
        self.caches
            .sprites()
            .map(|sprites| group_of(&sprites.library(), asset))
            .unwrap_or_default()
    }

    /// The concrete Asset the next Stamp gets: the selected one, or a random sibling from its
    /// Variant group.
    pub(super) fn placement_asset(&self) -> Option<AssetRef> {
        let selected = self.selected_asset.as_ref()?;
        let library = self.caches.sprites()?.library();
        let (pack, asset) = library.asset(selected)?;
        let Some(group) = &asset.variant_group else {
            return Some(selected.clone());
        };
        let members: Vec<&str> = pack
            .variant_group(group)
            .map(|member| member.id.as_str())
            .collect();
        let pick = members[random_below(members.len())];
        AssetRef::new(pack.id.clone(), pick).ok()
    }

    /// The template's Layer for what is being placed, if the Map still has it.
    pub(super) fn default_layer(&self, placing: Placing) -> Option<Id> {
        let layers = &self.default_layers;
        let layer = match placing {
            Placing::Land => layers.land,
            Placing::Stamp => layers.assets,
            Placing::Label => layers.labels,
        };
        layer.filter(|&id| matches!(self.map.node(id), Some(Node::Layer(_))))
    }

    /// The template's Layer for `placing` if no Layer is active, or if the active one is the
    /// template's Layer for something else (placing a Stamp makes its Layer active, and the next
    /// Label should still go to the Labels Layer); else the active Layer.
    pub(super) fn target_layer(&self, placing: Placing) -> Option<Id> {
        let active = self.active_layer.filter(|&id| self.map.contains(id));
        let layers = &self.default_layers;
        let taken_by_other = |id: Id| {
            let others = match placing {
                Placing::Land => [layers.assets, layers.labels],
                Placing::Stamp => [layers.land, layers.labels],
                Placing::Label => [layers.land, layers.assets],
            };
            others.contains(&Some(id))
        };
        match active {
            Some(id) if !taken_by_other(id) => Some(id),
            _ => self.default_layer(placing).or(active),
        }
    }

    /// The Layer new Objects go into: see [`Self::target_layer`], else the top Layer, creating
    /// a Layer if the Map has none. Runs inside the open gesture.
    pub(super) fn placement_layer(&mut self, gesture: &GestureId, placing: Placing) -> Option<Id> {
        match self.placement_layer_or_new(placing) {
            Ok(layer) => Some(layer),
            Err(create) => {
                let (id, command) = *create;
                self.gesture(gesture, command).then_some(id)
            }
        }
    }

    /// The Layer a new Object goes into, or, when the Map has none, the Id and Command of a
    /// Layer to create first.
    pub(super) fn placement_layer_or_new(
        &self,
        placing: Placing,
    ) -> Result<Id, Box<(Id, Command)>> {
        if let Some(layer) = self.target_layer(placing) {
            return Ok(layer);
        }
        if let Some(&top) = self.map.children(None).last() {
            return Ok(top);
        }
        let id = Id::new();
        let name = format!("Layer {}", self.layer_count() + 1);
        let command = Command::CreateLayer {
            id,
            parent: None,
            index: None,
            name,
        };
        Err(Box::new((id, command)))
    }

    pub(super) fn layer_count(&self) -> usize {
        self.map
            .ids()
            .filter(|&id| self.map.node(id).is_some_and(Node::is_layer))
            .count()
    }
}
