//! Turns a Map into what the Canvas draws, keeping derived data (Label layouts) between
//! edits so only what changed is rebuilt.

use std::collections::HashMap;
use std::sync::Arc;

use ssmap_core::{Id, Label, Map, Node, ObjectKind};

use crate::canvas::Content;
use crate::text::{Fonts, LabelLayout, LayoutKey};
use crate::vector::VectorGroup;

/// The Map's drawn Objects and the derived data behind them. The editor keeps one per open
/// Map; export builds its own.
pub struct MapScene {
    fonts: Fonts,
    layouts: HashMap<Id, (LayoutKey, Arc<LabelLayout>)>,
    /// The content last built, and the Map revision it was built from.
    built: Option<(u64, Content)>,
}

impl MapScene {
    pub fn new(fonts: Fonts) -> Self {
        Self {
            fonts,
            layouts: HashMap::new(),
            built: None,
        }
    }

    /// Forgets everything derived, as when another Map opens.
    pub fn clear(&mut self) {
        self.layouts.clear();
        self.built = None;
    }

    /// What the Canvas draws for `map`, rebuilt only when the Map has changed since.
    pub fn content(&mut self, map: &Map) -> Content {
        if let Some((revision, content)) = &self.built
            && *revision == map.revision()
        {
            return content.clone();
        }
        let mut groups = Vec::new();
        let mut seen = Vec::new();
        for &layer in map.children(None) {
            self.collect(map, layer, 1.0, &mut groups, &mut seen);
        }
        self.layouts.retain(|id, _| seen.contains(id));
        let content = Content::new(groups);
        self.built = Some((map.revision(), content.clone()));
        content
    }

    /// The shaped text of a Label, shaping it only if its text or typography changed.
    pub fn label_layout(&mut self, id: Id, label: &Label) -> Arc<LabelLayout> {
        let key = LayoutKey::of(label);
        if let Some((cached, layout)) = self.layouts.get(&id)
            && *cached == key
        {
            return layout.clone();
        }
        let layout = Arc::new(LabelLayout::shape(&mut self.fonts, label));
        self.layouts.insert(id, (key, layout.clone()));
        layout
    }

    /// Adds what `id` and everything inside it draw, in Stacking order. Hidden Layers draw
    /// nothing, and Layer opacity fades what they hold.
    fn collect(
        &mut self,
        map: &Map,
        id: Id,
        opacity: f32,
        groups: &mut Vec<VectorGroup>,
        seen: &mut Vec<Id>,
    ) {
        match map.node(id) {
            Some(Node::Layer(layer)) => {
                if !layer.visible {
                    return;
                }
                let opacity = opacity * layer.opacity;
                for &child in map.children(Some(id)) {
                    self.collect(map, child, opacity, groups, seen);
                }
            }
            Some(Node::Object(object)) => {
                if let ObjectKind::Label(label) = &object.kind {
                    seen.push(id);
                    let layout = self.label_layout(id, label);
                    groups.push(layout.group(label, opacity));
                }
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::{Canvas, Command, DistanceUnit, Scale};

    fn map_with_label() -> (Map, Id, Id) {
        let mut map = Map::new(Canvas::new(800, 600, Scale::new(1.0, DistanceUnit::Miles)));
        let layer = Id::new();
        let label = Id::new();
        map.execute(&Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Labels".into(),
        })
        .unwrap();
        map.execute(&Command::CreateLabel {
            id: label,
            parent: layer,
            index: None,
            label: Box::new(Label {
                text: "Thule".into(),
                ..Label::default()
            }),
        })
        .unwrap();
        (map, layer, label)
    }

    #[test]
    fn content_is_rebuilt_only_after_an_edit() {
        let (mut map, layer, _) = map_with_label();
        let mut scene = MapScene::new(Fonts::bundled());
        let first = scene.content(&map);
        assert_eq!(first.groups().len(), 1);
        assert_eq!(scene.content(&map), first, "same build");
        map.execute(&Command::SetOpacity {
            id: layer,
            opacity: 0.5,
        })
        .unwrap();
        let faded = scene.content(&map);
        assert_ne!(faded, first);
        assert_eq!(faded.groups()[0].shapes[0].color.a, 128);
    }

    #[test]
    fn moving_a_label_reuses_its_layout() {
        let (mut map, _, id) = map_with_label();
        let mut scene = MapScene::new(Fonts::bundled());
        let Some(Node::Object(object)) = map.node(id) else {
            panic!()
        };
        let ObjectKind::Label(label) = &object.kind else {
            panic!()
        };
        let mut label = (**label).clone();
        let before = scene.label_layout(id, &label);
        label.position = [400.0, 300.0];
        label.rotation = 30.0;
        map.execute(&Command::SetLabel {
            id,
            label: Box::new(label.clone()),
        })
        .unwrap();
        assert!(Arc::ptr_eq(&before, &scene.label_layout(id, &label)));
        label.size *= 2.0;
        assert!(!Arc::ptr_eq(&before, &scene.label_layout(id, &label)));
    }

    #[test]
    fn hidden_layers_draw_nothing() {
        let (mut map, layer, _) = map_with_label();
        map.execute(&Command::SetVisible {
            id: layer,
            visible: false,
        })
        .unwrap();
        let mut scene = MapScene::new(Fonts::bundled());
        assert!(scene.content(&map).groups().is_empty());
    }
}
