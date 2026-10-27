//! The Commands that create, change and delete boards: the Canvas and the Art boards.

use crate::background::Background;
use crate::geometry::Point;
use crate::id::Id;
use crate::map::{Board, Canvas, Map};
use crate::node::{Membership, Node};
use crate::op::{Op, Property};

use super::CommandError;
use super::check::set;

fn existing_board(map: &Map, id: Id) -> Result<&Board, CommandError> {
    map.board(id).ok_or(CommandError::NoSuchBoard(id))
}

fn valid_rect(origin: Point, width: u32, height: u32) -> Result<(), CommandError> {
    if width == 0 || height == 0 {
        return Err(CommandError::InvalidCanvasSize { width, height });
    }
    if !origin.iter().all(|c| c.is_finite()) {
        return Err(CommandError::InvalidCanvasOrigin(origin));
    }
    Ok(())
}

/// The ids of the Objects whose membership is `membership`, in id order so Ops are repeatable.
fn members(map: &Map, membership: Membership) -> Vec<Id> {
    let mut ids: Vec<Id> = map
        .ids()
        .filter(|&id| matches!(map.node(id), Some(Node::Object(o)) if o.membership == membership))
        .collect();
    ids.sort();
    ids
}

/// Creates an Art board with the default background and the Map-wide scale, after the others
/// unless `index` says otherwise.
pub(super) fn create_board(
    map: &Map,
    id: Id,
    name: Option<&str>,
    origin: Point,
    width: u32,
    height: u32,
    index: Option<usize>,
) -> Result<Vec<Op>, CommandError> {
    if map.board(id).is_some() {
        return Err(CommandError::BoardIdInUse(id));
    }
    valid_rect(origin, width, height)?;
    let mut canvas = Canvas::new(width, height, map.canvas().scale.clone());
    canvas.origin = origin;
    canvas.background = Background::default();
    Ok(vec![Op::InsertBoard {
        index: index.unwrap_or(map.boards().len()).min(map.boards().len()),
        board: Board {
            id,
            name: name.map(str::to_owned),
            canvas,
        },
    }])
}

/// Moves and resizes a board. No Object is touched, so Locks don't matter.
pub(super) fn resize_board(
    map: &Map,
    id: Id,
    origin: Point,
    width: u32,
    height: u32,
) -> Result<Vec<Op>, CommandError> {
    existing_board(map, id)?;
    valid_rect(origin, width, height)?;
    Ok(vec![Op::SetBoardRect {
        id,
        origin,
        width,
        height,
    }])
}

/// Renames a board, or with `None` returns it to its default name.
pub(super) fn rename_board(map: &Map, id: Id, name: Option<&str>) -> Result<Vec<Op>, CommandError> {
    existing_board(map, id)?;
    Ok(vec![Op::SetBoardName {
        id,
        name: name.map(str::to_owned),
    }])
}

/// Deletes an Art board. Its Objects go off-board instead of being deleted. The primary
/// board is refused.
pub(super) fn delete_board(map: &Map, id: Id) -> Result<Vec<Op>, CommandError> {
    existing_board(map, id)?;
    if id == map.primary_board() {
        return Err(CommandError::DeletePrimaryBoard(id));
    }
    let mut ops: Vec<Op> = members(map, Membership::Board(id))
        .into_iter()
        .map(|object| set(object, Property::Membership(Membership::OffCanvas)))
        .collect();
    ops.push(Op::RemoveBoard { id });
    Ok(ops)
}

/// Moves a board to `index` among the boards, counted after it leaves its old place.
pub(super) fn reorder_board(map: &Map, id: Id, index: usize) -> Result<Vec<Op>, CommandError> {
    existing_board(map, id)?;
    Ok(vec![Op::MoveBoard {
        id,
        index: index.min(map.boards().len() - 1),
    }])
}

/// Makes `id` the Canvas and the old Canvas an Art board. Each board keeps its Objects.
pub(super) fn set_primary_board(map: &Map, id: Id) -> Result<Vec<Op>, CommandError> {
    existing_board(map, id)?;
    let old = map.primary_board();
    if id == old {
        return Ok(Vec::new());
    }
    let mut ops: Vec<Op> = members(map, Membership::Canvas)
        .into_iter()
        .map(|object| set(object, Property::Membership(Membership::Board(old))))
        .collect();
    ops.extend(
        members(map, Membership::Board(id))
            .into_iter()
            .map(|object| set(object, Property::Membership(Membership::Canvas))),
    );
    ops.push(Op::SetPrimary { id });
    Ok(ops)
}
