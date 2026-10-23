//! Helpers the tools' tests share.

use eframe::egui;
use ssmap_core::{Canvas, Command, DistanceUnit, Id, Map, Scale};
use ssmap_render::View;

use super::brush::land_brush;
use super::{CanvasFrame, ToolId, ToolState};
use crate::editor::Editor;

pub(super) fn editor() -> Editor {
    Editor::new(Map::new(Canvas::new(
        500,
        500,
        Scale::new(1.0, DistanceUnit::Miles),
    )))
}

/// An Editor whose Map has a Layer for the land brush to paint into.
pub(super) fn land_editor() -> Editor {
    let mut editor = editor();
    let layer = Id::new();
    editor
        .run(Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Land".into(),
        })
        .unwrap();
    // Selecting a Layer makes it the active one; the Selection can then be emptied.
    editor.select(vec![layer]);
    editor.select(Vec::new());
    editor
}

/// One pixel per Canvas unit, and the widget at the screen origin.
pub(super) fn view() -> View {
    View::fit([500.0, 500.0], [500, 500], 0.0)
}

pub(super) fn at(x: f32, y: f32) -> egui::Pos2 {
    egui::pos2(x, y)
}

pub(super) fn input() -> CanvasFrame {
    CanvasFrame {
        pixels_per_point: 1.0,
        ..CanvasFrame::default()
    }
}

pub(super) fn press(x: f32, y: f32) -> CanvasFrame {
    CanvasFrame {
        drag_started: true,
        press_origin: Some(at(x, y)),
        pointer_pos: Some(at(x, y)),
        ..input()
    }
}

pub(super) fn drag_to(x: f32, y: f32) -> CanvasFrame {
    CanvasFrame {
        dragging: true,
        pointer_pos: Some(at(x, y)),
        ..input()
    }
}

pub(super) fn release(x: f32, y: f32) -> CanvasFrame {
    CanvasFrame {
        drag_stopped: true,
        pointer_pos: Some(at(x, y)),
        ..input()
    }
}

pub(super) fn click_at(x: f32, y: f32) -> CanvasFrame {
    CanvasFrame {
        clicked: true,
        press_origin: Some(at(x, y)),
        pointer_pos: Some(at(x, y)),
        ..input()
    }
}

pub(super) fn click(x: f32, y: f32) -> CanvasFrame {
    CanvasFrame {
        clicked: true,
        pointer_pos: Some(at(x, y)),
        ..input()
    }
}

pub(super) fn hovering() -> CanvasFrame {
    CanvasFrame {
        hovered: true,
        hover_pos: Some(at(10.0, 10.0)),
        ..input()
    }
}

/// A land brush stroke from (100, 100) to (200, 150), up to and including the release.
pub(super) fn paint_stroke(editor: &mut Editor, view: &View, state: &mut ToolState, water: bool) {
    land_brush(editor, view, state, &press(100.0, 100.0), water);
    land_brush(editor, view, state, &drag_to(150.0, 120.0), water);
    land_brush(editor, view, state, &drag_to(200.0, 150.0), water);
    land_brush(editor, view, state, &release(200.0, 150.0), water);
}

/// Land painted over (100, 100) to (200, 150), the Select tool active and nothing selected.
pub(super) fn with_land() -> (Editor, View, ToolState) {
    let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
    paint_stroke(&mut editor, &view, &mut state, false);
    editor.select(Vec::new());
    editor.active_tool = ToolId::Select;
    (editor, view, state)
}
