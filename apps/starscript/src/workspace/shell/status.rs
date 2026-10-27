//! The text of the status bar and the rails: what is selected and where the pointer is.

use ssmap_core::{Canvas, DistanceUnit, Id, Map, Node, ObjectKind};

use crate::layer_tree::node_name;

/// The Selection's names from its Layer down, for the rail. Empty when nothing is selected;
/// several Objects show the count.
pub fn selection_path(map: &Map, selection: &[Id]) -> Vec<String> {
    match selection {
        [] => Vec::new(),
        [id] => {
            let mut path = vec![node_name(map, *id)];
            let mut node = *id;
            while let Some(parent) = map.parent(node) {
                path.push(node_name(map, parent));
                node = parent;
            }
            path.reverse();
            path
        }
        many => vec![format!("{} selected", many.len())],
    }
}

fn kind_noun(map: &Map, id: Id, plural: bool) -> &'static str {
    match (map.node(id), plural) {
        (Some(Node::Layer(_)), false) => "Layer",
        (Some(Node::Layer(_)), true) => "Layers",
        (Some(Node::Object(object)), plural) => match (&object.kind, plural) {
            (ObjectKind::Landmass(_), false) => "Landmass",
            (ObjectKind::Landmass(_), true) => "Landmasses",
            (ObjectKind::Lake(_), false) => "Lake",
            (ObjectKind::Lake(_), true) => "Lakes",
            (ObjectKind::Stamp(_), false) => "Stamp",
            (ObjectKind::Stamp(_), true) => "Stamps",
            (ObjectKind::Label(_), false) => "Label",
            (ObjectKind::Label(_), true) => "Labels",
        },
        (None, false) => "Object",
        (None, true) => "Objects",
    }
}

/// "2 Landmasses", "Stamp · Pine", or "Active Layer: Roads" when nothing is selected.
pub fn selection_summary(map: &Map, selection: &[Id], active_layer: Option<Id>) -> String {
    match selection {
        [] => active_layer
            .map(|layer| format!("Active Layer: {}", node_name(map, layer)))
            .unwrap_or_default(),
        [id] => {
            let noun = kind_noun(map, *id, false);
            let name = match map.node(*id) {
                Some(Node::Object(object)) => match &object.kind {
                    ObjectKind::Stamp(stamp) => Some(title_case(&stamp.asset.asset)),
                    ObjectKind::Label(_) | ObjectKind::Landmass(_) | ObjectKind::Lake(_) => {
                        Some(node_name(map, *id)).filter(|name| name != noun)
                    }
                },
                Some(Node::Layer(layer)) => Some(layer.name.clone()),
                None => None,
            };
            match name {
                Some(name) => format!("{noun} · {name}"),
                None => noun.to_owned(),
            }
        }
        [first, rest @ ..] => {
            let noun = kind_noun(map, *first, true);
            if rest.iter().all(|id| kind_noun(map, *id, true) == noun) {
                format!("{} {noun}", selection.len())
            } else {
                format!("{} Objects", selection.len())
            }
        }
    }
}

fn title_case(id: &str) -> String {
    let mut chars = id.replace(['-', '_'], " ").chars().collect::<Vec<_>>();
    if let Some(first) = chars.first_mut() {
        *first = first.to_ascii_uppercase();
    }
    chars.into_iter().collect()
}

/// A whole number with thousands separators.
pub(super) fn grouped(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    if value < 0 { format!("-{out}") } else { out }
}

/// A Canvas point in pixels and in the Map's real-world units:
/// "X 3,118 Y 1,886 px · 779 × 471 mi". Without `real_world` only the pixels are given.
pub fn cursor_text(canvas: &Canvas, point: [f64; 2], real_world: bool) -> String {
    let unit = match canvas.scale.unit {
        DistanceUnit::Miles => "mi",
        DistanceUnit::Kilometres => "km",
    };
    let per_unit = canvas.scale.distance_per_unit;
    let whole = |value: f64| grouped(value.floor() as i64);
    if !real_world {
        return format!("X {} Y {} px", whole(point[0]), whole(point[1]));
    }
    format!(
        "X {} Y {} px · {} × {} {unit}",
        whole(point[0]),
        whole(point[1]),
        whole(point[0] * per_unit),
        whole(point[1] * per_unit),
    )
}
