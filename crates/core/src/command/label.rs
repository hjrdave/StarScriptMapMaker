//! Label and Label style Commands.

use crate::id::Id;
use crate::label::{Label, LabelStyle, LabelStyles};
use crate::map::{Map, MapDefaults, Subtree};
use crate::node::{Node, Object, ObjectKind};
use crate::op::{Op, Property};

use super::CommandError;
use super::check::{existing, layer, set, unlocked};

pub(super) fn create_label(
    map: &Map,
    id: Id,
    parent: Id,
    index: Option<usize>,
    label: &Label,
) -> Result<Vec<Op>, CommandError> {
    if map.contains(id) {
        return Err(CommandError::IdInUse(id));
    }
    layer(map, parent)?;
    unlocked(map, parent)?;
    check_label(map, label)?;
    Ok(vec![Op::Insert {
        parent: Some(parent),
        index: index.unwrap_or(map.children(Some(parent)).len()),
        subtree: Subtree {
            id,
            node: Node::Object(Object::new(ObjectKind::Label(Box::new(label.clone())))),
            children: Vec::new(),
        },
    }])
}

pub(super) fn set_label(map: &Map, id: Id, label: &Label) -> Result<Vec<Op>, CommandError> {
    match existing(map, id)? {
        Node::Object(Object {
            kind: ObjectKind::Label(_),
            ..
        }) => {}
        _ => return Err(CommandError::NotALabel(id)),
    }
    unlocked(map, id)?;
    check_label(map, label)?;
    Ok(vec![set(id, Property::Label(Box::new(label.clone())))])
}

pub(super) fn create_label_style(map: &Map, style: &LabelStyle) -> Result<Vec<Op>, CommandError> {
    valid_label_style(style)?;
    let styles = map.label_styles();
    if styles.get(style.id).is_some() {
        return Err(CommandError::LabelStyleIdInUse(style.id));
    }
    style_name_free(map, style.id, &style.name)?;
    let mut list = styles.to_vec();
    list.push(style.clone());
    Ok(vec![set_label_styles(map, list)])
}

pub(super) fn rename_label_style(map: &Map, id: Id, name: &str) -> Result<Vec<Op>, CommandError> {
    let mut list = map.label_styles().to_vec();
    let style = list
        .iter_mut()
        .find(|style| style.id == id)
        .ok_or(CommandError::NoLabelStyle(id))?;
    style.name = name.to_owned();
    valid_label_style(style)?;
    style_name_free(map, id, name)?;
    Ok(vec![set_label_styles(map, list)])
}

pub(super) fn set_label_style(map: &Map, style: &LabelStyle) -> Result<Vec<Op>, CommandError> {
    valid_label_style(style)?;
    let mut list = map.label_styles().to_vec();
    let slot = list
        .iter_mut()
        .find(|slot| slot.id == style.id)
        .ok_or(CommandError::NoLabelStyle(style.id))?;
    let name = std::mem::take(&mut slot.name);
    *slot = LabelStyle {
        name,
        ..style.clone()
    };
    Ok(vec![set_label_styles(map, list)])
}

pub(super) fn delete_label_style(map: &Map, id: Id) -> Result<Vec<Op>, CommandError> {
    if id == LabelStyle::DEFAULT_ID {
        return Err(CommandError::DefaultLabelStyle);
    }
    let style = map
        .label_styles()
        .get(id)
        .ok_or(CommandError::NoLabelStyle(id))?;
    let mut list = map.label_styles().to_vec();
    list.retain(|style| style.id != id);
    // The Labels are rewritten even if locked: nothing about how they look changes.
    let mut users = map.labels_using(id);
    users.sort();
    let mut ops = vec![set_label_styles(map, list)];
    for user in users {
        let Some(Node::Object(Object {
            kind: ObjectKind::Label(label),
            ..
        })) = map.node(user)
        else {
            continue;
        };
        let mut label = label.clone();
        label.bake(style);
        label.style = LabelStyle::DEFAULT_ID;
        ops.push(set(user, Property::Label(label)));
    }
    Ok(ops)
}

/// Checks a Label's values are drawable.
fn check_label(map: &Map, label: &Label) -> Result<(), CommandError> {
    if let Some(opacity) = label.opacity
        && !(0.0..=1.0).contains(&opacity)
    {
        return Err(CommandError::InvalidOpacity(opacity));
    }
    if let Some(problem) = label.problem() {
        return Err(CommandError::InvalidLabel(problem));
    }
    if map.label_styles().get(label.style).is_none() {
        return Err(CommandError::NoLabelStyle(label.style));
    }
    Ok(())
}

fn valid_label_style(style: &LabelStyle) -> Result<(), CommandError> {
    match style.problem() {
        Some(problem) => Err(CommandError::InvalidLabelStyle(problem)),
        None => Ok(()),
    }
}

/// Fails if a style other than `id` already has `name`.
fn style_name_free(map: &Map, id: Id, name: &str) -> Result<(), CommandError> {
    let name = name.trim();
    if map
        .label_styles()
        .iter()
        .any(|style| style.id != id && style.name.trim() == name)
    {
        return Err(CommandError::LabelStyleNameInUse(name.to_owned()));
    }
    Ok(())
}

/// An Op that replaces the Map's Label styles and leaves the other defaults alone.
fn set_label_styles(map: &Map, styles: Vec<LabelStyle>) -> Op {
    Op::SetDefaults {
        defaults: MapDefaults {
            label_styles: LabelStyles::new(styles),
            ..map.defaults().clone()
        },
    }
}
