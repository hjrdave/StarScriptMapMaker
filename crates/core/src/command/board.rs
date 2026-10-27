//! Compiling the Art board Commands.

use crate::background::Background;
use crate::board::ArtBoard;
use crate::geometry::Point;
use crate::id::Id;
use crate::map::Map;
use crate::node::{Membership, Node};
use crate::op::{Op, Property};

use super::CommandError;
use super::check::set;
use super::defaults;
use super::layer;

/// A board of the Map: the Canvas or an Art board.
fn known(map: &Map, id: Id) -> Result<bool, CommandError> {
    if id == map.canvas().id {
        Ok(true)
    } else if map.art_board(id).is_some() {
        Ok(false)
    } else {
        Err(CommandError::NoSuchBoard(id))
    }
}

fn check_size(origin: Point, width: u32, height: u32) -> Result<(), CommandError> {
    if width == 0 || height == 0 {
        return Err(CommandError::InvalidCanvasSize { width, height });
    }
    if !origin.iter().all(|c| c.is_finite()) {
        return Err(CommandError::InvalidCanvasOrigin(origin));
    }
    Ok(())
}

/// Adds an Art board on top of the others, with the Canvas's background. An empty `name`
/// becomes "Art board N".
pub(super) fn create_board(
    map: &Map,
    id: Id,
    name: &str,
    origin: Point,
    width: u32,
    height: u32,
) -> Result<Vec<Op>, CommandError> {
    if id == map.canvas().id || map.art_board(id).is_some() || map.contains(id) {
        return Err(CommandError::IdInUse(id));
    }
    check_size(origin, width, height)?;
    let name = if name.is_empty() {
        format!("Art board {}", map.art_boards().len() + 1)
    } else {
        name.to_owned()
    };
    let board = ArtBoard::new(
        id,
        name,
        origin,
        width,
        height,
        map.canvas().background.clone(),
    );
    Ok(vec![Op::AddBoard {
        index: map.art_boards().len(),
        board,
    }])
}

/// Resizes and moves the Canvas or an Art board. Locks don't matter: no Object is touched.
pub(super) fn resize_board(
    map: &Map,
    id: Id,
    origin: Point,
    width: u32,
    height: u32,
) -> Result<Vec<Op>, CommandError> {
    if known(map, id)? {
        return layer::resize_canvas(origin, width, height);
    }
    check_size(origin, width, height)?;
    Ok(vec![Op::ResizeBoard {
        id,
        origin,
        width,
        height,
    }])
}

/// Replaces the background of the Canvas or an Art board.
pub(super) fn set_board_background(
    map: &Map,
    id: Id,
    background: &Background,
) -> Result<Vec<Op>, CommandError> {
    let primary = known(map, id)?;
    let mut ops = defaults::set_background(background)?;
    if !primary {
        ops = vec![Op::SetBoardBackground {
            id,
            background: background.clone(),
        }];
    }
    Ok(ops)
}

/// Renames the Canvas or an Art board.
pub(super) fn rename_board(map: &Map, id: Id, name: &str) -> Result<Vec<Op>, CommandError> {
    known(map, id)?;
    Ok(vec![Op::RenameBoard {
        id,
        name: name.to_owned(),
    }])
}

/// Deletes an Art board. Its Objects are not deleted: they sit off the boards afterwards.
pub(super) fn delete_board(map: &Map, id: Id) -> Result<Vec<Op>, CommandError> {
    if known(map, id)? {
        return Err(CommandError::CannotDeleteCanvas(id));
    }
    let mut members: Vec<Id> = map
        .ids()
        .filter(|&object| {
            matches!(
                map.node(object),
                Some(Node::Object(o)) if o.membership == Membership::Board(id)
            )
        })
        .collect();
    members.sort();
    let mut ops: Vec<Op> = members
        .into_iter()
        .map(|object| set(object, Property::Membership(Membership::OffCanvas)))
        .collect();
    ops.push(Op::RemoveBoard { id });
    Ok(ops)
}

/// Moves an Art board to `index` among the Art boards, lowest first. The Canvas is always
/// lowest, so it cannot be moved.
pub(super) fn reorder_board(map: &Map, id: Id, index: usize) -> Result<Vec<Op>, CommandError> {
    if known(map, id)? {
        return Err(CommandError::CannotReorderCanvas(id));
    }
    let last = map.art_boards().len() - 1;
    Ok(vec![Op::MoveBoard {
        id,
        index: index.min(last),
    }])
}

/// Makes an Art board the Canvas and the Canvas an Art board. The Canvas itself: no change.
pub(super) fn set_primary_board(map: &Map, id: Id) -> Result<Vec<Op>, CommandError> {
    if known(map, id)? {
        return Ok(Vec::new());
    }
    Ok(vec![Op::SetPrimaryBoard { id }])
}
