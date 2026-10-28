//! The Canvas zoom: the steps, presets and framing maths behind the zoom control and its
//! shortcuts. Pure functions on a `View`, so they are tested without a window.

use ssmap_render::View;

use crate::handles::Bounds;

pub(crate) const MIN_ZOOM: f64 = 0.01;
pub(crate) const MAX_ZOOM: f64 = 64.0;

/// What one press of `−` or `+` multiplies the zoom by.
pub(crate) const STEP: f64 = 1.25;
/// The presets of the zoom menu, as a share of 1:1.
pub(crate) const PRESETS: [f64; 6] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0];
/// Space left around the framed area, in points.
pub(crate) const FRAME_MARGIN: f64 = 32.0;

/// A zoom the user asked for, from the control, the menu or a shortcut.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Zoom {
    In,
    Out,
    /// A share of 1:1 (1.0 is 100%), in points rather than physical pixels.
    Percent(f64),
    Fit,
    Selection,
    /// Frames this area, such as a board or all of them.
    Area(Bounds),
}

/// Sets the zoom to `target` (physical pixels per Canvas unit, clamped), keeping the Canvas
/// point under `pixel` where it is.
fn zoom_about(view: &mut View, pixel: [f64; 2], target: f64) {
    let target = target.clamp(MIN_ZOOM, MAX_ZOOM);
    view.zoom_at(pixel, target / view.zoom);
}

/// The middle of the view, in target pixels.
pub(crate) fn centre(view: &View) -> [f64; 2] {
    [f64::from(view.size[0]) / 2.0, f64::from(view.size[1]) / 2.0]
}

/// Applies a zoom that keeps the view's centre. `bounds` is what Zoom to Selection frames;
/// `canvas` is the Canvas size for Zoom to Fit. Returns false when nothing changed because
/// there was nothing to frame.
pub(crate) fn apply(
    view: &mut View,
    zoom: Zoom,
    pixels_per_point: f64,
    canvas: [f64; 2],
    bounds: Option<Bounds>,
) -> bool {
    let middle = centre(view);
    match zoom {
        Zoom::In => zoom_about(view, middle, view.zoom * STEP),
        Zoom::Out => zoom_about(view, middle, view.zoom / STEP),
        Zoom::Percent(share) => zoom_about(view, middle, share * pixels_per_point),
        Zoom::Fit => {
            *view = View::fit(canvas, view.size, FRAME_MARGIN * pixels_per_point);
        }
        Zoom::Selection => {
            let Some(bounds) = bounds else { return false };
            *view = frame(&bounds, view.size, FRAME_MARGIN * pixels_per_point);
        }
        Zoom::Area(area) => *view = frame(&area, view.size, FRAME_MARGIN * pixels_per_point),
    }
    true
}

/// A view of `size` pixels with `bounds` centred and filling it, `margin` pixels clear.
pub(crate) fn frame(bounds: &Bounds, size: [u32; 2], margin: f64) -> View {
    let extent = [
        (bounds.max[0] - bounds.min[0]).max(1.0),
        (bounds.max[1] - bounds.min[1]).max(1.0),
    ];
    let available = [
        (f64::from(size[0]) - 2.0 * margin).max(1.0),
        (f64::from(size[1]) - 2.0 * margin).max(1.0),
    ];
    let zoom = (available[0] / extent[0])
        .min(available[1] / extent[1])
        .clamp(MIN_ZOOM, MAX_ZOOM);
    let middle = bounds.centre();
    View {
        origin: [
            middle[0] - f64::from(size[0]) / (2.0 * zoom),
            middle[1] - f64::from(size[1]) / (2.0 * zoom),
        ],
        zoom,
        size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS: [f64; 2] = [4096.0, 3072.0];

    fn view() -> View {
        View::fit(CANVAS, [1200, 800], FRAME_MARGIN)
    }

    fn near(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
    }

    #[test]
    fn presets_and_steps_keep_the_centre() {
        let before = view();
        let middle = before.to_canvas(centre(&before));
        for zoom in [
            Zoom::In,
            Zoom::Out,
            Zoom::Percent(0.25),
            Zoom::Percent(1.0),
            Zoom::Percent(8.0),
        ] {
            let mut after = before;
            assert!(apply(&mut after, zoom, 1.0, CANVAS, None));
            assert!(near(after.to_canvas(centre(&after)), middle), "{zoom:?}");
        }
    }

    #[test]
    fn presets_set_the_share_of_one_to_one() {
        let mut v = view();
        apply(&mut v, Zoom::Percent(2.0), 1.5, CANVAS, None);
        assert!((v.zoom - 3.0).abs() < 1e-9, "points, not physical pixels");
        apply(&mut v, Zoom::Percent(1.0), 1.0, CANVAS, None);
        assert!((v.zoom - 1.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_is_clamped() {
        let mut v = view();
        apply(&mut v, Zoom::Percent(1e6), 1.0, CANVAS, None);
        assert_eq!(v.zoom, MAX_ZOOM);
        apply(&mut v, Zoom::Percent(1e-6), 1.0, CANVAS, None);
        assert_eq!(v.zoom, MIN_ZOOM);
        apply(&mut v, Zoom::Out, 1.0, CANVAS, None);
        assert_eq!(v.zoom, MIN_ZOOM);
    }

    #[test]
    fn in_then_out_returns() {
        let before = view();
        let mut v = before;
        apply(&mut v, Zoom::In, 1.0, CANVAS, None);
        assert!(v.zoom > before.zoom);
        apply(&mut v, Zoom::Out, 1.0, CANVAS, None);
        assert!((v.zoom - before.zoom).abs() < 1e-9);
        assert!(near(v.origin, before.origin));
    }

    #[test]
    fn fit_matches_the_first_frame() {
        let first = View::fit(CANVAS, [1200, 800], FRAME_MARGIN * 2.0);
        let mut v = first;
        apply(&mut v, Zoom::Percent(4.0), 2.0, CANVAS, None);
        apply(&mut v, Zoom::Fit, 2.0, CANVAS, None);
        assert_eq!(v.zoom, first.zoom);
        assert_eq!(v.origin, first.origin);
    }

    #[test]
    fn selection_is_framed_and_centred() {
        let bounds = Bounds {
            min: [1000.0, 500.0],
            max: [1400.0, 600.0],
        };
        let mut v = view();
        assert!(apply(&mut v, Zoom::Selection, 1.0, CANVAS, Some(bounds)));
        assert!(near(v.to_canvas(centre(&v)), bounds.centre()));
        // The width is the tight side: 1200 - 2 * 32 pixels across 400 units.
        assert!((v.zoom - 1136.0 / 400.0).abs() < 1e-9);
        let corner = v.to_canvas([FRAME_MARGIN, 0.0]);
        assert!((corner[0] - bounds.min[0]).abs() < 1e-6);
    }

    #[test]
    fn a_point_selection_does_not_zoom_past_the_limit() {
        let bounds = Bounds {
            min: [10.0, 10.0],
            max: [10.0, 10.0],
        };
        assert_eq!(frame(&bounds, [800, 600], 32.0).zoom, MAX_ZOOM);
    }

    #[test]
    fn nothing_to_frame_changes_nothing() {
        let before = view();
        let mut v = before;
        assert!(!apply(&mut v, Zoom::Selection, 1.0, CANVAS, None));
        assert_eq!(v.zoom, before.zoom);
        assert_eq!(v.origin, before.origin);
    }
}
