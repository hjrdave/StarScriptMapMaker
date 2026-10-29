//! Editing the outlines of Landmasses and Lakes: regions, boolean operations, node edits.

use std::collections::HashSet;

use crate::color::Color;
use crate::geometry::MultiPolygon;
use crate::id::Id;
use crate::land;
use crate::map::{Map, Subtree};
use crate::node::{Node, ObjectKind};
use crate::op::{Op, Property};
use crate::roughness::Roughness;
use crate::shore::ShoreStyle;

use super::CommandError;
use super::check::{existing, set, unlocked};
use super::object::delete;

pub(super) fn add_region(
    map: &Map,
    id: Id,
    region: &MultiPolygon,
) -> Result<Vec<Op>, CommandError> {
    let [id] = stored_outlines(map, &[id])?[..] else {
        unreachable!("one id in, one id out")
    };
    valid_region(id, region)?;
    let geometry = land::union(outline(map, id), region);
    if let Some(problem) = geometry.problem() {
        return Err(CommandError::InvalidGeometry { id, problem });
    }
    Ok(vec![Op::ReplaceGeometry { id, geometry }])
}

pub(super) fn subtract_region(
    map: &Map,
    ids: &[Id],
    region: &MultiPolygon,
) -> Result<Vec<Op>, CommandError> {
    let ids = stored_outlines(map, ids)?;
    if let Some(&first) = ids.first() {
        valid_region(first, region)?;
    }
    let mut ops = Vec::new();
    let mut emptied = Vec::new();
    for id in ids {
        let current = outline(map, id);
        if !land::intersects(current, region) {
            continue;
        }
        let geometry = land::difference(current, region);
        if geometry.is_empty() {
            emptied.push(id);
        } else {
            ops.push(Op::ReplaceGeometry { id, geometry });
        }
    }
    if !emptied.is_empty() {
        ops.extend(delete(map, &emptied)?);
    }
    Ok(ops)
}

/// The distinct, unlocked Landmasses among `ids`, topmost first.
fn landmasses_topmost_first(
    map: &Map,
    ids: &[Id],
    command: &'static str,
) -> Result<Vec<Id>, CommandError> {
    let ids = outlined(map, ids)?;
    for &id in &ids {
        if !matches!(map.node(id), Some(Node::Object(o)) if matches!(o.kind, ObjectKind::Landmass(_)))
        {
            return Err(CommandError::NotALandmass(id));
        }
    }
    if ids.len() < 2 {
        return Err(CommandError::NeedsTwoLandmasses { command });
    }
    let order = map.draw_order();
    let rank = |id: Id| order.iter().position(|&o| o == id);
    let mut ids = ids;
    ids.sort_by_key(|&id| std::cmp::Reverse(rank(id)));
    Ok(ids)
}

pub(super) fn union_landmasses(map: &Map, ids: &[Id]) -> Result<Vec<Op>, CommandError> {
    let ids = landmasses_topmost_first(map, ids, "Union")?;
    let Some((&keeper, others)) = ids.split_first() else {
        return Err(CommandError::NeedsTwoLandmasses { command: "Union" });
    };
    let mut geometry = outline(map, keeper).clone();
    for &other in others {
        geometry = land::union(&geometry, outline(map, other));
    }
    if let Some(problem) = geometry.problem() {
        return Err(CommandError::InvalidGeometry {
            id: keeper,
            problem,
        });
    }
    let mut ops = vec![Op::ReplaceGeometry {
        id: keeper,
        geometry,
    }];
    let mut clipped: Vec<Id> = map
        .ids()
        .filter(|&id| matches!(map.node(id), Some(Node::Layer(l)) if l.clip.is_some_and(|c| others.contains(&c))))
        .collect();
    clipped.sort();
    for id in clipped {
        unlocked(map, id)?;
        ops.push(set(id, Property::Clip(Some(keeper))));
    }
    ops.extend(others.iter().map(|&id| Op::Remove { id }));
    Ok(ops)
}

pub(super) fn subtract_landmasses(map: &Map, ids: &[Id]) -> Result<Vec<Op>, CommandError> {
    let ids = landmasses_topmost_first(map, ids, "Subtract")?;
    let Some((&cutter, targets)) = ids.split_first() else {
        return Err(CommandError::NeedsTwoLandmasses {
            command: "Subtract",
        });
    };
    let region = outline(map, cutter).clone();
    let mut ops = Vec::new();
    let mut gone = vec![cutter];
    for &id in targets {
        let current = outline(map, id);
        if !land::intersects(current, &region) {
            continue;
        }
        let geometry = land::difference(current, &region);
        if geometry.is_empty() {
            gone.push(id);
        } else {
            ops.push(Op::ReplaceGeometry { id, geometry });
        }
    }
    ops.extend(delete(map, &gone)?);
    Ok(ops)
}

pub(super) fn split_apart(map: &Map, id: Id, new_ids: &[Id]) -> Result<Vec<Op>, CommandError> {
    let [id] = outlined(map, &[id])?[..] else {
        unreachable!("one id in, one id out")
    };
    let Some(Node::Object(object)) = map.node(id) else {
        unreachable!("outlined verified the id is an Object")
    };
    let ObjectKind::Landmass(landmass) = &object.kind else {
        return Err(CommandError::NotALandmass(id));
    };
    let parts = &landmass.geometry.0;
    if parts.len() < 2 {
        return Err(CommandError::NothingToSplit(id));
    }
    if new_ids.len() != parts.len() - 1 {
        return Err(CommandError::SplitIds {
            id,
            parts: parts.len(),
            got: new_ids.len(),
        });
    }
    for (i, &new) in new_ids.iter().enumerate() {
        if map.contains(new) || new == id || new_ids[..i].contains(&new) {
            return Err(CommandError::IdInUse(new));
        }
    }
    let parent = map.parent(id).ok_or(CommandError::NotFound(id))?;
    let position = map
        .children(Some(parent))
        .iter()
        .position(|&c| c == id)
        .ok_or(CommandError::NotFound(id))?;
    let mut ops = vec![Op::ReplaceGeometry {
        id,
        geometry: MultiPolygon(vec![parts[0].clone()]),
    }];
    for (k, (&new, part)) in new_ids.iter().zip(&parts[1..]).enumerate() {
        let mut copy = object.clone();
        if let ObjectKind::Landmass(l) = &mut copy.kind {
            l.geometry = MultiPolygon(vec![part.clone()]);
        }
        ops.push(Op::Insert {
            parent: Some(parent),
            index: position + 1 + k,
            subtree: Subtree {
                id: new,
                node: Node::Object(copy),
                children: Vec::new(),
            },
        });
    }
    Ok(ops)
}

/// Replaces the outline of one Landmass or Lake with the result of a node edit.
pub(super) fn node_edit(
    map: &Map,
    id: Id,
    edit: impl FnOnce(&MultiPolygon) -> Result<MultiPolygon, land::NodeEditError>,
) -> Result<Vec<Op>, CommandError> {
    let [id] = stored_outlines(map, &[id])?[..] else {
        unreachable!("one id in, one id out")
    };
    let geometry = edit(outline(map, id)).map_err(|e| CommandError::NodeEdit {
        id,
        problem: e.to_string(),
    })?;
    if let Some(problem) = geometry.problem() {
        return Err(CommandError::InvalidGeometry { id, problem });
    }
    Ok(vec![Op::ReplaceGeometry { id, geometry }])
}

/// Like [`outlined`], but refuses Rivers, whose outlines follow their points: for edits to
/// the stored outline of a Landmass or Lake.
fn stored_outlines(map: &Map, ids: &[Id]) -> Result<Vec<Id>, CommandError> {
    let ids = outlined(map, ids)?;
    match ids
        .iter()
        .find(|&&id| super::river::river(map, id).is_some())
    {
        Some(&river) => Err(CommandError::RiverOutline(river)),
        None => Ok(ids),
    }
}

/// Checks that every id is an unlocked Landmass, Lake or River, and drops repeats.
pub(super) fn outlined(map: &Map, ids: &[Id]) -> Result<Vec<Id>, CommandError> {
    let mut seen = HashSet::new();
    let mut unique = Vec::with_capacity(ids.len());
    for &id in ids {
        match existing(map, id)? {
            Node::Object(object) if object.kind.geometry().is_some() => {}
            _ => return Err(CommandError::NoOutline(id)),
        }
        unlocked(map, id)?;
        if seen.insert(id) {
            unique.push(id);
        }
    }
    Ok(unique)
}

/// The outline of an id `outlined` accepted.
#[allow(
    clippy::expect_used,
    reason = "`outlined` verified the Object has an outline"
)]
pub(super) fn outline(map: &Map, id: Id) -> &MultiPolygon {
    match map.node(id) {
        Some(Node::Object(object)) => object
            .kind
            .geometry()
            .expect("outlined verified the Object has an outline"),
        _ => unreachable!("outlined verified the id is an Object"),
    }
}

/// A region to add or subtract must itself be a valid outline. Errors name `id`, the Object
/// being edited.
fn valid_region(id: Id, region: &MultiPolygon) -> Result<(), CommandError> {
    match region.problem() {
        Some(problem) => Err(CommandError::InvalidGeometry {
            id,
            problem: format!("the region: {problem}"),
        }),
        None => Ok(()),
    }
}

pub(super) fn valid_roughness(roughness: &Roughness) -> Result<(), CommandError> {
    let Roughness { amount, scale, .. } = *roughness;
    if amount.is_finite() && amount >= 0.0 && scale.is_finite() && scale > 0.0 {
        Ok(())
    } else {
        Err(CommandError::InvalidRoughness { amount, scale })
    }
}

/// Sets the fill colour of Landmasses and Lakes.
pub(super) fn set_fill(map: &Map, ids: &[Id], fill: Color) -> Result<Vec<Op>, CommandError> {
    Ok(outlined(map, ids)?
        .into_iter()
        .map(|id| set(id, Property::Fill(fill)))
        .collect())
}

/// Sets the Roughness of Landmasses and Lakes, or with `None` returns them to the Map-wide
/// default.
pub(super) fn set_roughness(
    map: &Map,
    ids: &[Id],
    roughness: &Option<Roughness>,
) -> Result<Vec<Op>, CommandError> {
    if let Some(roughness) = roughness {
        valid_roughness(roughness)?;
    }
    Ok(stored_outlines(map, ids)?
        .into_iter()
        .map(|id| set(id, Property::Roughness(roughness.clone())))
        .collect())
}

/// Sets the Shore style of Landmasses and Lakes, or with `None` returns them to the Map-wide
/// default for their kind.
pub(super) fn set_shore(
    map: &Map,
    ids: &[Id],
    shore: &Option<ShoreStyle>,
) -> Result<Vec<Op>, CommandError> {
    if let Some(problem) = shore.as_ref().and_then(ShoreStyle::problem) {
        return Err(CommandError::InvalidShore(problem));
    }
    Ok(outlined(map, ids)?
        .into_iter()
        .map(|id| set(id, Property::Shore(shore.clone())))
        .collect())
}
