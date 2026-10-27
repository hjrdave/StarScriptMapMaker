//! What a Map lacks for pasted branches, and how pasting closes the gaps.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::{
    AssetRef, Canvas, DistanceUnit, FontFace, Id, Label, LabelStyle, Layer, Map, Node, Object,
    ObjectKind, Scale, Stamp, Subtree,
};

fn map() -> Map {
    Map::new(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)))
}

fn object(kind: ObjectKind) -> Subtree {
    Subtree {
        id: Id::new(),
        node: Node::Object(Object::new(kind)),
        children: Vec::new(),
    }
}

/// A Layer holding `objects`, as a copied branch.
fn layer_of(objects: Vec<Subtree>) -> Subtree {
    Subtree {
        id: Id::new(),
        node: Node::Layer(Layer::new("Copied")),
        children: objects,
    }
}

fn stamp(asset: &str) -> Subtree {
    let asset: AssetRef = asset.parse().unwrap();
    object(ObjectKind::Stamp(Stamp::new(asset, [0.0, 0.0])))
}

fn label(style: Id, family: Option<&str>) -> Subtree {
    let label = Label {
        style,
        size: Some(40.0),
        font: family.map(|family| FontFace {
            family: family.into(),
            ..FontFace::default()
        }),
        ..Label::default()
    };
    object(ObjectKind::Label(Box::new(label)))
}

fn labels(trees: &[Subtree]) -> Vec<&Label> {
    let mut found = Vec::new();
    for tree in trees {
        if let Node::Object(Object {
            kind: ObjectKind::Label(label),
            ..
        }) = &tree.node
        {
            found.push(&**label);
        }
        found.extend(labels(&tree.children));
    }
    found
}

fn bundled(family: &str) -> bool {
    family == "Cinzel" || family == "Noto Serif"
}

#[test]
fn nothing_is_missing_when_the_map_has_it_all() {
    let trees = vec![layer_of(vec![
        stamp("pack/tree"),
        label(LabelStyle::DEFAULT_ID, Some("Cinzel")),
        label(LabelStyle::DEFAULT_ID, None),
    ])];
    let gaps = map().paste_gaps(&trees, |_| true, bundled);
    assert!(gaps.is_empty());
}

#[test]
fn an_asset_neither_installed_nor_embedded_is_missing_once() {
    let trees = vec![layer_of(vec![
        stamp("pack/tree"),
        stamp("pack/tree"),
        stamp("pack/rock"),
    ])];
    let gaps = map().paste_gaps(&trees, |asset| asset.to_string() == "pack/rock", bundled);
    assert_eq!(gaps.assets, vec!["pack/tree".parse::<AssetRef>().unwrap()]);
    assert!(gaps.fonts.is_empty());
    assert_eq!(gaps.label_styles, 0);
}

#[test]
fn a_font_the_map_cannot_draw_is_missing() {
    let trees = vec![layer_of(vec![
        label(LabelStyle::DEFAULT_ID, Some("Papyrus")),
        label(LabelStyle::DEFAULT_ID, Some("Papyrus")),
    ])];
    let gaps = map().paste_gaps(&trees, |_| true, bundled);
    assert_eq!(gaps.fonts, vec!["Papyrus".to_owned()]);

    let mut trees = trees;
    map().fit_for_paste(&mut trees, bundled, "Noto Serif");
    for label in labels(&trees) {
        assert_eq!(label.font.as_ref().unwrap().family, "Noto Serif");
        // Only the family changes.
        assert_eq!(label.size, Some(40.0));
    }
}

#[test]
fn an_unknown_label_style_falls_back_to_the_default_and_keeps_overrides() {
    let trees = vec![layer_of(vec![label(Id::new(), None)])];
    let gaps = map().paste_gaps(&trees, |_| true, bundled);
    assert_eq!(gaps.label_styles, 1);

    let mut trees = trees;
    map().fit_for_paste(&mut trees, bundled, "Noto Serif");
    let label = labels(&trees)[0];
    assert_eq!(label.style, LabelStyle::DEFAULT_ID);
    assert_eq!(label.size, Some(40.0));
    // The style itself is not copied.
    assert_eq!(map().label_styles().iter().count(), 1);
}

#[test]
fn fitting_leaves_what_the_map_has_alone() {
    let mut trees = vec![layer_of(vec![
        stamp("pack/tree"),
        label(LabelStyle::DEFAULT_ID, Some("Cinzel")),
    ])];
    let before = trees.clone();
    map().fit_for_paste(&mut trees, bundled, "Noto Serif");
    assert_eq!(trees, before);
}
