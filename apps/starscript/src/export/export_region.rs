//! The maths behind the Export Region tool: drawing, moving and resizing the custom export
//! rectangle, kept in Canvas units and free of any window so it can be tested headlessly.
//! The outline itself is an editor overlay and never part of an export.

use ssmap_core::Rect;

use crate::handles::{Bounds, Handle};

/// The smallest side, in Canvas units, a region may have. A smaller drag is dropped.
pub(crate) const MIN_SIDE: f64 = 1.0;

/// What a pointer drag on the region is doing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum RegionDrag {
    /// Drawing a new rectangle from the point where the button went down.
    Draw { anchor: [f64; 2] },
    /// Moving the rectangle as it was when the drag began.
    Move { start: [f64; 2], original: Rect },
    /// Dragging a handle of the rectangle as it was when the drag began.
    Resize {
        handle: Handle,
        start: [f64; 2],
        original: Rect,
    },
}

impl RegionDrag {
    /// The region with the pointer at `pointer`, kept inside the Canvas of `canvas` size.
    /// Every frame works from where the drag began, so nothing drifts.
    pub(crate) fn rect(&self, pointer: [f64; 2], canvas: [f64; 2]) -> Rect {
        match *self {
            RegionDrag::Draw { anchor } => {
                let a = clamp_point(anchor, canvas);
                let b = clamp_point(pointer, canvas);
                Rect {
                    min: [a[0].min(b[0]), a[1].min(b[1])],
                    max: [a[0].max(b[0]), a[1].max(b[1])],
                }
            }
            RegionDrag::Move { start, original } => moved(
                &original,
                [pointer[0] - start[0], pointer[1] - start[1]],
                canvas,
            ),
            RegionDrag::Resize {
                handle,
                start,
                original,
            } => resized(
                &original,
                handle,
                [pointer[0] - start[0], pointer[1] - start[1]],
                canvas,
            ),
        }
    }
}

fn clamp_point(point: [f64; 2], canvas: [f64; 2]) -> [f64; 2] {
    [
        point[0].clamp(0.0, canvas[0]),
        point[1].clamp(0.0, canvas[1]),
    ]
}

/// `rect` moved by `delta`, stopping at the Canvas edges rather than changing size.
pub(crate) fn moved(rect: &Rect, delta: [f64; 2], canvas: [f64; 2]) -> Rect {
    let shift = |axis: usize| {
        let low = -rect.min[axis];
        let high = canvas[axis] - rect.max[axis];
        // A rectangle larger than the Canvas has no room to move.
        if low > high {
            0.0
        } else {
            delta[axis].clamp(low, high)
        }
    };
    let (dx, dy) = (shift(0), shift(1));
    Rect {
        min: [rect.min[0] + dx, rect.min[1] + dy],
        max: [rect.max[0] + dx, rect.max[1] + dy],
    }
}

/// `rect` with `handle` dragged by `delta`, kept inside the Canvas. Dragging past the
/// opposite side flips the rectangle.
pub(crate) fn resized(rect: &Rect, handle: Handle, delta: [f64; 2], canvas: [f64; 2]) -> Rect {
    let bounds = Bounds {
        min: rect.min,
        max: rect.max,
    }
    .resized(handle, delta);
    Rect {
        min: clamp_point(bounds.min, canvas),
        max: clamp_point(bounds.max, canvas),
    }
}

/// Whether the region is big enough to keep.
pub(crate) fn is_usable(rect: &Rect) -> bool {
    rect.width() >= MIN_SIDE && rect.height() >= MIN_SIDE
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS: [f64; 2] = [1000.0, 800.0];
    const RECT: Rect = Rect {
        min: [100.0, 200.0],
        max: [300.0, 400.0],
    };

    #[test]
    fn drawing_normalises_any_direction() {
        let drag = RegionDrag::Draw {
            anchor: [300.0, 400.0],
        };
        assert_eq!(drag.rect([100.0, 200.0], CANVAS), RECT);
    }

    #[test]
    fn drawing_stops_at_the_canvas_edge() {
        let drag = RegionDrag::Draw {
            anchor: [900.0, 700.0],
        };
        let rect = drag.rect([1500.0, 900.0], CANVAS);
        assert_eq!(rect.min, [900.0, 700.0]);
        assert_eq!(rect.max, CANVAS);
        let outside = RegionDrag::Draw {
            anchor: [-50.0, -50.0],
        };
        assert_eq!(outside.rect([10.0, 10.0], CANVAS).min, [0.0, 0.0]);
    }

    #[test]
    fn moving_keeps_the_size_and_stops_at_the_edge() {
        let drag = RegionDrag::Move {
            start: [150.0, 250.0],
            original: RECT,
        };
        let rect = drag.rect([160.0, 240.0], CANVAS);
        assert_eq!((rect.width(), rect.height()), (200.0, 200.0));
        assert_eq!(rect.min, [110.0, 190.0]);
        let far = drag.rect([5000.0, -5000.0], CANVAS);
        assert_eq!(far.max[0], 1000.0);
        assert_eq!(far.min[1], 0.0);
        assert_eq!((far.width(), far.height()), (200.0, 200.0));
    }

    #[test]
    fn resizing_keeps_the_opposite_corner_and_stays_inside() {
        let drag = RegionDrag::Resize {
            handle: Handle::BottomRight,
            start: [300.0, 400.0],
            original: RECT,
        };
        let rect = drag.rect([350.0, 450.0], CANVAS);
        assert_eq!(rect.min, RECT.min);
        assert_eq!(rect.max, [350.0, 450.0]);
        assert_eq!(drag.rect([9999.0, 9999.0], CANVAS).max, CANVAS);
        let edge = RegionDrag::Resize {
            handle: Handle::Left,
            start: [100.0, 300.0],
            original: RECT,
        };
        let rect = edge.rect([-40.0, 999.0], CANVAS);
        assert_eq!((rect.min, rect.max), ([0.0, 200.0], [300.0, 400.0]));
    }

    #[test]
    fn resizing_past_the_opposite_side_flips() {
        let drag = RegionDrag::Resize {
            handle: Handle::Left,
            start: [100.0, 300.0],
            original: RECT,
        };
        let rect = drag.rect([400.0, 300.0], CANVAS);
        assert_eq!((rect.min[0], rect.max[0]), (300.0, 400.0));
    }

    #[test]
    fn tiny_regions_are_not_usable() {
        assert!(is_usable(&RECT));
        let sliver = Rect {
            min: [0.0, 0.0],
            max: [0.5, 100.0],
        };
        assert!(!is_usable(&sliver));
    }
}
