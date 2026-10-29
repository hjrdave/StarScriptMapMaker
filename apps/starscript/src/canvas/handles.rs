//! Figma-style selection bounds and transform handles: drag inside to move, a handle to
//! scale, and the knob above to rotate. The Select tool resizes Landmasses and Lakes from
//! the handles, and scales Labels.

use eframe::egui;
use ssmap_core::{Halo, Id, Label, LabelStyles, Node, Rect, Stamp};
use ssmap_render::View;

use crate::editor::Editor;

/// An axis-aligned box in Canvas units.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Bounds {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

/// The eight resize handles, by position on the box.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Handle {
    pub const ALL: [Handle; 8] = [
        Handle::TopLeft,
        Handle::Top,
        Handle::TopRight,
        Handle::Right,
        Handle::BottomRight,
        Handle::Bottom,
        Handle::BottomLeft,
        Handle::Left,
    ];

    /// The pointer shown over the handle.
    pub fn cursor(self) -> egui::CursorIcon {
        match self {
            Handle::TopLeft | Handle::BottomRight => egui::CursorIcon::ResizeNwSe,
            Handle::TopRight | Handle::BottomLeft => egui::CursorIcon::ResizeNeSw,
            Handle::Top | Handle::Bottom => egui::CursorIcon::ResizeVertical,
            Handle::Left | Handle::Right => egui::CursorIcon::ResizeHorizontal,
        }
    }

    /// Where on the box the handle sits, from (0, 0) top-left to (1, 1) bottom-right.
    pub fn anchor(self) -> [f64; 2] {
        match self {
            Handle::TopLeft => [0.0, 0.0],
            Handle::Top => [0.5, 0.0],
            Handle::TopRight => [1.0, 0.0],
            Handle::Right => [1.0, 0.5],
            Handle::BottomRight => [1.0, 1.0],
            Handle::Bottom => [0.5, 1.0],
            Handle::BottomLeft => [0.0, 1.0],
            Handle::Left => [0.0, 0.5],
        }
    }
}

/// What a pointer can grab on the selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Grip {
    Resize(Handle),
    /// The knob above the top edge.
    Rotate,
    /// The knob below the bottom edge of a single Label, which bends its baseline.
    Arch,
}

impl Grip {
    /// The pointer shown over the grip.
    pub(crate) fn cursor(self) -> egui::CursorIcon {
        match self {
            Grip::Resize(handle) => handle.cursor(),
            Grip::Rotate => egui::CursorIcon::Grab,
            Grip::Arch => egui::CursorIcon::ResizeVertical,
        }
    }
}

/// The knobs a selection offers besides the eight resize handles.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) struct Knobs {
    /// The rotate knob above the box.
    pub(crate) rotate: bool,
    /// The arch knob below the box, holding the arch (-1 to 1) it shows.
    pub(crate) arch: Option<f64>,
}

/// Half the size of a handle square, in points.
const HANDLE_RADIUS: f32 = 4.0;
/// Below this many points across, the inside of the box moves rather than resizes.
const SMALL_BOX: f32 = 6.0 * HANDLE_RADIUS;
/// How far the rotate knob sits above the top edge, in points.
const ROTATE_OFFSET: f32 = 20.0;
/// How far the arch knob sits below the bottom edge, in points.
const ARCH_OFFSET: f32 = 20.0;

impl Bounds {
    /// Combines the outlines of every selected Object that has one, and the text blocks
    /// of selected Labels.
    pub fn of_selection(editor: &mut Editor) -> Option<Self> {
        let ids = editor.selection().to_vec();
        Self::of_ids(editor, &ids)
    }

    /// The same for the Objects `ids` name.
    pub fn of_ids(editor: &mut Editor, ids: &[Id]) -> Option<Self> {
        let mut points: Vec<[f64; 2]> = Vec::new();
        for &id in ids {
            if let Some((label, layout)) = editor.label_layout(id) {
                points.extend(layout.corners(&label));
                continue;
            }
            if let Some(bounds) = editor.stamp_bounds(id) {
                points.extend([bounds.min, bounds.max]);
                continue;
            }
            let Some(Node::Object(object)) = editor.map().node(id) else {
                continue;
            };
            let Some(geometry) = object.kind.geometry() else {
                continue;
            };
            for polygon in &geometry.0 {
                points.extend(&polygon.exterior);
            }
        }
        Self::around(&points)
    }

    /// The smallest box holding every point.
    pub fn around(points: &[[f64; 2]]) -> Option<Self> {
        let (&first, rest) = points.split_first()?;
        Some(rest.iter().fold(
            Bounds {
                min: first,
                max: first,
            },
            |b, &[x, y]| Bounds {
                min: [b.min[0].min(x), b.min[1].min(y)],
                max: [b.max[0].max(x), b.max[1].max(y)],
            },
        ))
    }

    pub fn centre(&self) -> [f64; 2] {
        self.point([0.5, 0.5])
    }

    pub fn contains(&self, point: [f64; 2]) -> bool {
        (self.min[0]..=self.max[0]).contains(&point[0])
            && (self.min[1]..=self.max[1]).contains(&point[1])
    }

    /// The uniform scale and the fixed point for dragging `handle` by `delta`: corners
    /// scale by the mean of both axes, edges by their own axis, about the opposite side.
    pub fn scaling(&self, handle: Handle, delta: [f64; 2]) -> (f64, [f64; 2]) {
        let [ax, ay] = handle.anchor();
        let resized = self.resized(handle, delta);
        let ratio = |axis: usize| {
            let before = self.max[axis] - self.min[axis];
            let after = resized.max[axis] - resized.min[axis];
            if before > 0.0 { after / before } else { 1.0 }
        };
        let scale = match (ax == 0.5, ay == 0.5) {
            (true, _) => ratio(1),
            (_, true) => ratio(0),
            _ => (ratio(0) * ratio(1)).sqrt(),
        };
        (scale.max(MIN_SCALE), self.point([1.0 - ax, 1.0 - ay]))
    }

    pub fn rect(&self) -> Rect {
        Rect {
            min: self.min,
            max: self.max,
        }
    }

    fn point(&self, anchor: [f64; 2]) -> [f64; 2] {
        [
            self.min[0] + (self.max[0] - self.min[0]) * anchor[0],
            self.min[1] + (self.max[1] - self.min[1]) * anchor[1],
        ]
    }

    /// The box after dragging `handle` by `delta` Canvas units, keeping the opposite side or
    /// corner fixed. Dragging past the opposite side flips the box rather than inverting it.
    pub fn resized(&self, handle: Handle, delta: [f64; 2]) -> Bounds {
        let [ax, ay] = handle.anchor();
        let mut min = self.min;
        let mut max = self.max;
        if ax == 0.0 {
            min[0] += delta[0];
        } else if ax == 1.0 {
            max[0] += delta[0];
        }
        if ay == 0.0 {
            min[1] += delta[1];
        } else if ay == 1.0 {
            max[1] += delta[1];
        }
        Bounds {
            min: [min[0].min(max[0]), min[1].min(max[1])],
            max: [min[0].max(max[0]), min[1].max(max[1])],
        }
    }
}

/// The smallest scale a drag can apply, so Labels never shrink to nothing.
const MIN_SCALE: f64 = 0.01;

/// `label` moved by `delta` Canvas units.
pub(crate) fn moved(label: &Label, delta: [f64; 2]) -> Label {
    Label {
        position: [label.position[0] + delta[0], label.position[1] + delta[1]],
        ..label.clone()
    }
}

/// `label` scaled by `scale` about the Canvas point `fixed`. Its size and halo scale with it;
/// spacing and line height are relative to the size already.
pub(crate) fn scaled(label: &Label, styles: &LabelStyles, scale: f64, fixed: [f64; 2]) -> Label {
    let resolved = label.resolve(styles);
    let mut out = label.clone();
    out.position = [
        fixed[0] + (label.position[0] - fixed[0]) * scale,
        fixed[1] + (label.position[1] - fixed[1]) * scale,
    ];
    out.size = Some(resolved.size * scale);
    out.halo = Some(Halo {
        width: resolved.halo.width * scale,
        ..resolved.halo
    });
    out
}

/// `label` rotated clockwise by `degrees` about the Canvas point `centre`.
pub(crate) fn rotated(label: &Label, degrees: f64, centre: [f64; 2]) -> Label {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let [dx, dy] = [label.position[0] - centre[0], label.position[1] - centre[1]];
    let mut out = label.clone();
    out.position = [
        centre[0] + dx * cos - dy * sin,
        centre[1] + dx * sin + dy * cos,
    ];
    out.rotation = (label.rotation + degrees).rem_euclid(360.0);
    out
}

/// `stamp` moved by `delta` Canvas units.
pub(crate) fn moved_stamp(stamp: &Stamp, delta: [f64; 2]) -> Stamp {
    Stamp {
        position: [stamp.position[0] + delta[0], stamp.position[1] + delta[1]],
        ..stamp.clone()
    }
}

/// `stamp` scaled by `scale` about the Canvas point `fixed`.
pub(crate) fn scaled_stamp(stamp: &Stamp, scale: f64, fixed: [f64; 2]) -> Stamp {
    Stamp {
        position: [
            fixed[0] + (stamp.position[0] - fixed[0]) * scale,
            fixed[1] + (stamp.position[1] - fixed[1]) * scale,
        ],
        scale: stamp.scale * scale,
        ..stamp.clone()
    }
}

/// `stamp` rotated clockwise by `degrees` about the Canvas point `centre`.
pub(crate) fn rotated_stamp(stamp: &Stamp, degrees: f64, centre: [f64; 2]) -> Stamp {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let [dx, dy] = [stamp.position[0] - centre[0], stamp.position[1] - centre[1]];
    Stamp {
        position: [
            centre[0] + dx * cos - dy * sin,
            centre[1] + dx * sin + dy * cos,
        ],
        rotation: (stamp.rotation + degrees).rem_euclid(360.0),
        ..stamp.clone()
    }
}

/// Converts a Canvas point to a screen position inside the Canvas panel at `origin`.
pub(crate) fn to_screen(view: &View, origin: egui::Pos2, ppp: f64, point: [f64; 2]) -> egui::Pos2 {
    let x = (point[0] - view.origin[0]) * view.zoom / ppp;
    let y = (point[1] - view.origin[1]) * view.zoom / ppp;
    origin + egui::vec2(x as f32, y as f32)
}

/// Where the rotate knob is drawn.
fn rotate_knob(bounds: &Bounds, view: &View, origin: egui::Pos2, ppp: f64) -> egui::Pos2 {
    to_screen(view, origin, ppp, bounds.point([0.5, 0.0])) - egui::vec2(0.0, ROTATE_OFFSET)
}

/// Where the arch knob is drawn.
fn arch_knob(bounds: &Bounds, view: &View, origin: egui::Pos2, ppp: f64) -> egui::Pos2 {
    to_screen(view, origin, ppp, bounds.point([0.5, 1.0])) + egui::vec2(0.0, ARCH_OFFSET)
}

/// The handle or knob under `pointer`, if any. Each knob is there only when `knobs` has it.
pub(crate) fn hit(
    bounds: &Bounds,
    view: &View,
    origin: egui::Pos2,
    ppp: f64,
    pointer: egui::Pos2,
    knobs: Knobs,
) -> Option<Grip> {
    let near = |centre: egui::Pos2| (pointer - centre).abs().max_elem() <= HANDLE_RADIUS + 2.0;
    if knobs.rotate && near(rotate_knob(bounds, view, origin, ppp)) {
        return Some(Grip::Rotate);
    }
    if knobs.arch.is_some() && near(arch_knob(bounds, view, origin, ppp)) {
        return Some(Grip::Arch);
    }
    // On a box too small to grab inside between its handles, inside means move.
    let rect = egui::Rect::from_two_pos(
        to_screen(view, origin, ppp, bounds.min),
        to_screen(view, origin, ppp, bounds.max),
    );
    if rect.contains(pointer) && rect.width().min(rect.height()) < SMALL_BOX {
        return None;
    }
    Handle::ALL
        .into_iter()
        .find(|&handle| near(to_screen(view, origin, ppp, bounds.point(handle.anchor()))))
        .map(Grip::Resize)
}

/// Draws just the box, for an outline that is shown but not being edited.
pub(crate) fn paint_outline(
    painter: &egui::Painter,
    bounds: &Bounds,
    view: &View,
    origin: egui::Pos2,
    ppp: f64,
    accent: egui::Color32,
) {
    let rect = egui::Rect::from_two_pos(
        to_screen(view, origin, ppp, bounds.min),
        to_screen(view, origin, ppp, bounds.max),
    );
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, accent),
        egui::StrokeKind::Outside,
    );
}

/// The little curve drawn on the arch knob: flat at 0, bowing up for a positive arch.
fn arch_glyph(centre: egui::Pos2, arch: f64) -> Vec<egui::Pos2> {
    let half = HANDLE_RADIUS;
    let bow = (arch.clamp(-1.0, 1.0) as f32) * half;
    (0..=8)
        .map(|i| {
            let t = i as f32 / 8.0 * 2.0 - 1.0;
            egui::pos2(centre.x + t * half, centre.y - bow * (1.0 - t * t))
        })
        .collect()
}

/// The arch a drag of the arch knob gives a Label. The knob moves along the Label's own up
/// direction, and half the text block's width is one whole arch, so the baseline follows
/// the pointer: `delta` is the drag in Canvas units, `rotation` the Label's, in degrees,
/// and `width` its text block's.
pub(crate) fn arched_by(start: f64, delta: [f64; 2], rotation: f64, width: f64) -> f64 {
    let (sin, cos) = rotation.to_radians().sin_cos();
    // Canvas y points down, so a Label's up is (sin, -cos) after turning clockwise.
    let up = delta[0] * sin - delta[1] * cos;
    (start + up / (width * 0.5).max(1.0)).clamp(-1.0, 1.0)
}

/// Draws the box and its handles in `accent`, with the knobs `knobs` has.
pub(crate) fn paint(
    painter: &egui::Painter,
    bounds: &Bounds,
    view: &View,
    origin: egui::Pos2,
    ppp: f64,
    knobs: Knobs,
    accent: egui::Color32,
) {
    let rect = egui::Rect::from_two_pos(
        to_screen(view, origin, ppp, bounds.min),
        to_screen(view, origin, ppp, bounds.max),
    );
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.5, accent),
        egui::StrokeKind::Outside,
    );
    if let Some(arch) = knobs.arch {
        let knob = arch_knob(bounds, view, origin, ppp);
        painter.line_segment(
            [egui::pos2(knob.x, rect.max.y), knob],
            egui::Stroke::new(1.0, accent),
        );
        let chip = egui::Rect::from_center_size(knob, egui::Vec2::splat(2.0 * HANDLE_RADIUS + 6.0));
        painter.rect_filled(chip, 3.0, egui::Color32::WHITE);
        painter.rect_stroke(
            chip,
            3.0,
            egui::Stroke::new(1.0, accent),
            egui::StrokeKind::Inside,
        );
        painter.add(egui::Shape::line(
            arch_glyph(knob, arch),
            egui::Stroke::new(1.5, accent),
        ));
    }
    if knobs.rotate {
        let knob = rotate_knob(bounds, view, origin, ppp);
        painter.line_segment(
            [knob, egui::pos2(knob.x, rect.min.y)],
            egui::Stroke::new(1.0, accent),
        );
        painter.circle(
            knob,
            HANDLE_RADIUS,
            egui::Color32::WHITE,
            egui::Stroke::new(1.0, accent),
        );
    }
    for handle in Handle::ALL {
        let centre = to_screen(view, origin, ppp, bounds.point(handle.anchor()));
        let square = egui::Rect::from_center_size(centre, egui::Vec2::splat(2.0 * HANDLE_RADIUS));
        painter.rect_filled(square, 1.0, egui::Color32::WHITE);
        painter.rect_stroke(
            square,
            1.0,
            egui::Stroke::new(1.0, accent),
            egui::StrokeKind::Inside,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Knobs {
        fn rotate(rotate: bool) -> Self {
            Self { rotate, arch: None }
        }
    }

    const BOX: Bounds = Bounds {
        min: [10.0, 20.0],
        max: [110.0, 70.0],
    };

    #[test]
    fn resizing_keeps_the_opposite_corner() {
        let resized = BOX.resized(Handle::TopLeft, [5.0, -10.0]);
        assert_eq!(resized.min, [15.0, 10.0]);
        assert_eq!(resized.max, BOX.max);
        let edge = BOX.resized(Handle::Right, [20.0, 99.0]);
        assert_eq!(edge.max, [130.0, 70.0]);
        assert_eq!(edge.min, BOX.min);
    }

    #[test]
    fn dragging_past_the_opposite_side_flips() {
        let flipped = BOX.resized(Handle::Left, [150.0, 0.0]);
        assert_eq!(flipped.min, [110.0, 20.0]);
        assert_eq!(flipped.max, [160.0, 70.0]);
    }

    #[test]
    fn handles_are_hit_near_their_centres() {
        let view = View {
            origin: [0.0, 0.0],
            zoom: 2.0,
            size: [400, 300],
        };
        let origin = egui::pos2(100.0, 50.0);
        // BottomRight at Canvas (110, 70) is 2x zoom at 1 ppp: (220, 140) past the origin.
        let at = origin + egui::vec2(221.0, 139.0);
        assert_eq!(
            hit(&BOX, &view, origin, 1.0, at, Knobs::rotate(true)),
            Some(Grip::Resize(Handle::BottomRight))
        );
        assert_eq!(
            hit(&BOX, &view, origin, 1.0, origin, Knobs::rotate(true)),
            None
        );
        // Top centre at Canvas (60, 20) is (120, 40), and the knob 20 points above it.
        let knob = origin + egui::vec2(120.0, 20.0);
        assert_eq!(
            hit(&BOX, &view, origin, 1.0, knob, Knobs::rotate(true)),
            Some(Grip::Rotate)
        );
        assert_eq!(
            hit(&BOX, &view, origin, 1.0, knob, Knobs::rotate(false)),
            None,
            "no knob"
        );

        // A box a few points tall is moved from inside, not scaled from its edges.
        let thin = Bounds {
            min: [10.0, 20.0],
            max: [110.0, 25.0],
        };
        let top_centre = origin + egui::vec2(120.0, 42.0);
        assert_eq!(
            hit(&thin, &view, origin, 1.0, top_centre, Knobs::rotate(true)),
            None
        );
        let outside = origin + egui::vec2(221.0, 52.0);
        assert_eq!(
            hit(&thin, &view, origin, 1.0, outside, Knobs::rotate(true)),
            Some(Grip::Resize(Handle::BottomRight))
        );
    }

    #[test]
    fn the_arch_knob_is_hit_below_the_box_only_when_offered() {
        let view = View {
            origin: [0.0, 0.0],
            zoom: 2.0,
            size: [400, 300],
        };
        let origin = egui::pos2(100.0, 50.0);
        // The bottom centre at Canvas (60, 70) is (120, 140); the knob 20 points below.
        let at = origin + egui::vec2(120.0, 160.0);
        let archable = Knobs {
            rotate: true,
            arch: Some(0.0),
        };
        assert_eq!(
            hit(&BOX, &view, origin, 1.0, at, archable),
            Some(Grip::Arch)
        );
        assert_eq!(hit(&BOX, &view, origin, 1.0, at, Knobs::rotate(true)), None);
    }

    #[test]
    fn dragging_the_arch_knob_up_arches_and_down_sags() {
        // Half of a 100-unit block is one whole arch.
        assert_eq!(arched_by(0.0, [0.0, -25.0], 0.0, 100.0), 0.5);
        assert_eq!(arched_by(0.0, [0.0, 25.0], 0.0, 100.0), -0.5);
        assert_eq!(arched_by(0.8, [0.0, -100.0], 0.0, 100.0), 1.0);
        // A Label turned a quarter clockwise has its up along +x.
        assert!((arched_by(0.0, [25.0, 0.0], 90.0, 100.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn corners_scale_uniformly_about_the_opposite_corner() {
        // Doubling the width and height from the bottom-right keeps the top-left.
        let (scale, fixed) = BOX.scaling(Handle::BottomRight, [100.0, 50.0]);
        assert_eq!((scale, fixed), (2.0, BOX.min));
        // An edge scales by its own axis.
        let (scale, fixed) = BOX.scaling(Handle::Top, [7.0, -50.0]);
        assert_eq!((scale, fixed), (2.0, [60.0, 70.0]));
        // Never to nothing.
        assert_eq!(BOX.scaling(Handle::Right, [-100.0, 0.0]).0, MIN_SCALE);
    }

    #[test]
    fn label_transforms_move_scale_and_rotate() {
        let label = Label {
            position: [10.0, 0.0],
            size: Some(20.0),
            rotation: 350.0,
            ..Label::default()
        };
        assert_eq!(moved(&label, [1.0, 2.0]).position, [11.0, 2.0]);
        let big = scaled(&label, &LabelStyles::default(), 2.0, [0.0, 0.0]);
        assert_eq!((big.position, big.size), ([20.0, 0.0], Some(40.0)));
        // A Label that follows its style gets an override of the scaled size.
        let styled = scaled(&Label::default(), &LabelStyles::default(), 0.5, [0.0, 0.0]);
        assert_eq!(styled.size, Some(24.0));
        let turned = rotated(&label, 90.0, [0.0, 0.0]);
        assert!((turned.position[0]).abs() < 1e-9 && (turned.position[1] - 10.0).abs() < 1e-9);
        assert!((turned.rotation - 80.0).abs() < 1e-9);
    }
}
