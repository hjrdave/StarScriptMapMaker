//! The maths behind the Canvas tool's resize: dragging a handle of the Canvas, with Shift
//! keeping the aspect ratio and Alt resizing from the centre. Kept in Canvas units and free
//! of any window so it can be tested headlessly.

use crate::handles::Handle;

/// The smallest side, in Canvas units, the Canvas may be dragged to.
const MIN_SIDE: f64 = 1.0;

/// A handle drag on the Canvas, remembering the Canvas as it was when the drag began so that
/// every frame works from the start and nothing drifts.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct CanvasDrag {
    pub handle: Handle,
    pub start: [f64; 2],
    pub origin: [f64; 2],
    pub size: [u32; 2],
}

impl CanvasDrag {
    /// The Canvas's origin and size with the pointer at `pointer`.
    pub(crate) fn resized(
        &self,
        pointer: [f64; 2],
        shift: bool,
        alt: bool,
    ) -> ([f64; 2], u32, u32) {
        resized(
            self.origin,
            self.size,
            self.handle,
            [pointer[0] - self.start[0], pointer[1] - self.start[1]],
            shift,
            alt,
        )
    }
}

/// The Canvas after dragging `handle` by `delta`. The side opposite the handle stays put, or
/// with `alt` the centre does. With `shift` the aspect ratio is kept: a corner follows the
/// larger change, and an edge scales the other axis about the middle.
pub(crate) fn resized(
    origin: [f64; 2],
    size: [u32; 2],
    handle: Handle,
    delta: [f64; 2],
    shift: bool,
    alt: bool,
) -> ([f64; 2], u32, u32) {
    let old = [f64::from(size[0]), f64::from(size[1])];
    let anchor = handle.anchor();
    // Which way each axis grows when the pointer moves positive: -1 left/top, 1 right/bottom,
    // 0 for the axis a middle handle leaves alone.
    let sign = anchor.map(|a| match a {
        0.0 => -1.0,
        1.0 => 1.0,
        _ => 0.0,
    });
    let grow = if alt { 2.0 } else { 1.0 };
    let mut new = [0, 1].map(|axis| (old[axis] + sign[axis] * delta[axis] * grow).max(MIN_SIDE));
    if shift {
        let ratio = |axis: usize| new[axis] / old[axis];
        let scale = match (sign[0] != 0.0, sign[1] != 0.0) {
            (true, true) => {
                if (ratio(0) - 1.0).abs() >= (ratio(1) - 1.0).abs() {
                    ratio(0)
                } else {
                    ratio(1)
                }
            }
            (true, false) => ratio(0),
            _ => ratio(1),
        };
        new = [0, 1].map(|axis| (old[axis] * scale).max(MIN_SIDE));
    }
    let width = new[0].round().max(MIN_SIDE) as u32;
    let height = new[1].round().max(MIN_SIDE) as u32;
    let new = [f64::from(width), f64::from(height)];
    let origin = [0, 1].map(|axis| {
        if alt || sign[axis] == 0.0 {
            // Centred: about the Canvas's centre.
            origin[axis] + (old[axis] - new[axis]) / 2.0
        } else if sign[axis] < 0.0 {
            // The far side stays put.
            origin[axis] + old[axis] - new[axis]
        } else {
            origin[axis]
        }
    });
    (origin, width, height)
}

/// The text of the cursor HUD while the Canvas is being resized.
pub(crate) fn hud_text(width: u32, height: u32) -> String {
    format!("Canvas · {width} × {height}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: [f64; 2] = [100.0, 50.0];
    const SIZE: [u32; 2] = [400, 200];

    #[test]
    fn a_corner_keeps_the_opposite_corner() {
        let (origin, w, h) = resized(ORIGIN, SIZE, Handle::TopLeft, [-50.0, -20.0], false, false);
        assert_eq!((origin, w, h), ([50.0, 30.0], 450, 220));
        let (origin, w, h) = resized(
            ORIGIN,
            SIZE,
            Handle::BottomRight,
            [30.0, 10.0],
            false,
            false,
        );
        assert_eq!((origin, w, h), (ORIGIN, 430, 210));
    }

    #[test]
    fn an_edge_changes_one_axis() {
        let (origin, w, h) = resized(ORIGIN, SIZE, Handle::Right, [100.0, 77.0], false, false);
        assert_eq!((origin, w, h), (ORIGIN, 500, 200));
        let (origin, w, h) = resized(ORIGIN, SIZE, Handle::Top, [9.0, -40.0], false, false);
        assert_eq!((origin, w, h), ([100.0, 10.0], 400, 240));
    }

    #[test]
    fn shift_keeps_the_aspect_ratio() {
        let (_, w, h) = resized(
            ORIGIN,
            SIZE,
            Handle::BottomRight,
            [200.0, 10.0],
            true,
            false,
        );
        assert_eq!((w, h), (600, 300));
        let (origin, w, h) = resized(ORIGIN, SIZE, Handle::Right, [200.0, 0.0], true, false);
        assert_eq!((w, h), (600, 300));
        assert_eq!(origin, [100.0, 0.0]);
    }

    #[test]
    fn alt_resizes_from_the_centre() {
        let (origin, w, h) = resized(ORIGIN, SIZE, Handle::BottomRight, [50.0, 20.0], false, true);
        assert_eq!((origin, w, h), ([50.0, 30.0], 500, 240));
        let (origin, w, h) = resized(ORIGIN, SIZE, Handle::TopLeft, [-50.0, -20.0], false, true);
        assert_eq!((origin, w, h), ([50.0, 30.0], 500, 240));
    }

    #[test]
    fn dragging_past_the_far_side_stops_at_the_smallest_canvas() {
        let (origin, w, h) = resized(ORIGIN, SIZE, Handle::Right, [-900.0, 0.0], false, false);
        assert_eq!((origin, w, h), (ORIGIN, 1, 200));
    }

    #[test]
    fn the_hud_names_the_canvas_and_its_size() {
        assert_eq!(hud_text(4096, 2560), "Canvas · 4096 × 2560");
    }
}
