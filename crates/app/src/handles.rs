//! Figma-style selection bounds and transform handles. Object tickets hook their transforms
//! onto these; for now the Selection's outline bounds are drawn with handles.

use eframe::egui;
use ssmap_core::{Id, Map, Node};
use ssmap_render::View;

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

    /// Where on the box the handle sits, from (0, 0) top-left to (1, 1) bottom-right.
    fn anchor(self) -> [f64; 2] {
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

/// Half the size of a handle square, in points.
const HANDLE_RADIUS: f32 = 4.0;

impl Bounds {
    /// Combines the outlines of every selected Object that has one.
    pub fn of_selection(map: &Map, selection: &[Id]) -> Option<Self> {
        let mut bounds: Option<Bounds> = None;
        for &id in selection {
            let Some(Node::Object(object)) = map.node(id) else {
                continue;
            };
            let Some(geometry) = object.kind.geometry() else {
                continue;
            };
            for polygon in &geometry.0 {
                for &[x, y] in &polygon.exterior {
                    bounds = Some(match bounds {
                        None => Bounds {
                            min: [x, y],
                            max: [x, y],
                        },
                        Some(b) => Bounds {
                            min: [b.min[0].min(x), b.min[1].min(y)],
                            max: [b.max[0].max(x), b.max[1].max(y)],
                        },
                    });
                }
            }
        }
        bounds
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

/// Converts a Canvas point to a screen position inside the Canvas panel at `origin`.
fn to_screen(view: &View, origin: egui::Pos2, ppp: f64, point: [f64; 2]) -> egui::Pos2 {
    let x = (point[0] - view.origin[0]) * view.zoom / ppp;
    let y = (point[1] - view.origin[1]) * view.zoom / ppp;
    origin + egui::vec2(x as f32, y as f32)
}

/// The handle under `pointer`, if any.
pub fn hit(
    bounds: &Bounds,
    view: &View,
    origin: egui::Pos2,
    ppp: f64,
    pointer: egui::Pos2,
) -> Option<Handle> {
    Handle::ALL.into_iter().find(|&handle| {
        let centre = to_screen(view, origin, ppp, bounds.point(handle.anchor()));
        (pointer - centre).abs().max_elem() <= HANDLE_RADIUS + 2.0
    })
}

pub fn paint(painter: &egui::Painter, bounds: &Bounds, view: &View, origin: egui::Pos2, ppp: f64) {
    let accent = egui::Color32::from_rgb(24, 160, 251);
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
        assert_eq!(hit(&BOX, &view, origin, 1.0, at), Some(Handle::BottomRight));
        assert_eq!(hit(&BOX, &view, origin, 1.0, origin), None);
    }
}
