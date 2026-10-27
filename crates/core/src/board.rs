//! Art boards: the rectangles of a Map besides the Canvas, which is the primary board.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::background::Background;
use crate::extra::Extra;
use crate::geometry::{Point, Rect};
use crate::id::Id;

/// The id the Canvas has until another board is made the primary one, and the id files saved
/// before Art boards existed give theirs.
pub const CANVAS_BOARD_ID: Id = Id::from_uuid(uuid::Uuid::from_u128(
    0x0000_0000_0000_0000_0000_0000_0000_0201,
));

/// The name the Canvas has until it is renamed.
pub const CANVAS_BOARD_NAME: &str = "Canvas";

/// A drawing area of a Map that is not the Canvas: Objects can belong to it and are clipped at
/// its edge, and Export leaves it out. The real-world scale is the Map's, not the board's.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ArtBoard {
    /// Names the board in Ops and in the membership of Objects.
    pub id: Id,
    /// The name shown on the board and in lists. Need not be unique.
    pub name: String,
    /// Width in Canvas units.
    pub width: u32,
    /// Height in Canvas units.
    pub height: u32,
    /// The workspace position of the top-left corner.
    pub origin: Point,
    /// The water colour and texture behind the board's Objects.
    pub background: Background,
    /// Fields from a newer file format that this version keeps without reading.
    #[serde(flatten)]
    pub extra: Extra,
}

impl ArtBoard {
    /// A board `width` by `height` units with its top-left corner at `origin`.
    pub fn new(
        id: Id,
        name: impl Into<String>,
        origin: Point,
        width: u32,
        height: u32,
        background: Background,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            width,
            height,
            origin,
            background,
            extra: Extra::new(),
        }
    }

    /// The board's rectangle in workspace coordinates.
    pub fn rect(&self) -> Rect {
        Rect {
            min: self.origin,
            max: [
                self.origin[0] + f64::from(self.width),
                self.origin[1] + f64::from(self.height),
            ],
        }
    }
}
