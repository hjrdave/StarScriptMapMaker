//! The modal dialogs: Esc cancels each, New Map shows its coverage line, and Export disables
//! sizes over the cap (no window needed).

use std::path::Path;
use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
mod common;

use ssmap_app::MapMakerApp;
use ssmap_app::export_dialog::ExportDialog;
use ssmap_app::export_run::{self, Progress};
use ssmap_app::new_map::NewMapDialog;
use ssmap_core::file::{MapFile, recovery_path};
use ssmap_core::{Canvas, DistanceUnit, ExportSettings, Map, Scale, SizeMode};

fn canvas() -> Canvas {
    Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles))
}

fn app(recovery_dir: &Path) -> Harness<'static, MapMakerApp> {
    let recovery_dir = recovery_dir.to_path_buf();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(move |cc| MapMakerApp::with_recovery_dir(cc, Some(recovery_dir.clone())));
    harness.run();
    // The app starts on Home with no Map; these tests count tabs from one open Map.
    harness.state_mut().new_map(canvas());
    harness.run();
    harness
}

/// A harness drawing one dialog with the app's theme, which the dialogs' text styles need.
fn dialog_harness<'a, S>(
    mut app: impl FnMut(&mut egui::Ui, &mut S) + 'a,
    state: S,
) -> Harness<'a, S> {
    // The harness draws a frame when built, before the theme can be installed from outside:
    // install it in that frame and draw the dialog from the next.
    let mut installed = false;
    Harness::new_ui_state(
        move |ui, state| {
            if installed {
                app(ui, state);
            } else {
                ssmap_app::theme::install(ui.ctx());
                installed = true;
            }
        },
        state,
    )
}

fn new_layer(harness: &mut Harness<'_, MapMakerApp>) {
    common::new_layer(harness);
}

#[test]
fn esc_cancels_the_unsaved_changes_question() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("first.ssmap");
    MapFile::create(&file, &Map::new(canvas()), None).unwrap();
    let mut harness = app(dir.path());
    harness.state_mut().open_path(&file);
    harness.run();
    new_layer(&mut harness);

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::W);
    harness.run();
    harness.get_by_label("Save changes to “first.ssmap”?");
    harness.get_by_label("Don’t Save");

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(harness.query_by_label("Don’t Save").is_none());
    assert_eq!(harness.state().tabs().len(), 2, "the tab stays open");
    assert!(harness.state().tabs()[1].has_unsaved_changes());
}

#[test]
fn esc_closes_the_recovery_question_and_keeps_the_journal() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("coast.ssmap");
    MapFile::create(&file, &Map::new(canvas()), None).unwrap();
    let mut harness = app(dir.path());
    assert!(harness.state_mut().open_path(&file));
    new_layer(&mut harness);
    let started = Instant::now();
    while harness.state().has_unjournaled_changes() {
        harness.step();
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(harness);

    let mut harness = app(dir.path());
    assert!(harness.state_mut().open_path(&file));
    harness.run();
    harness.get_by_label("Recover Changes");
    harness.get_by_label("Discard");

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(harness.query_by_label("Recover Changes").is_none());
    assert!(recovery_path(&file).exists(), "Esc doesn't discard");
}

#[test]
fn esc_cancels_the_new_map_dialog() {
    let mut harness = app(tempfile::tempdir().unwrap().path());
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::N);
    harness.run();
    harness.get_by_label("Create");

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(harness.query_by_label("Create").is_none());
    assert_eq!(harness.state().tabs().len(), 1);
}

#[test]
fn new_map_shows_the_real_world_coverage() {
    let mut dialog = NewMapDialog::default();
    dialog.open = true;
    let mut harness = dialog_harness(
        |ui, dialog: &mut NewMapDialog| {
            dialog.show(ui.ctx());
        },
        dialog,
    );
    harness.run();
    harness.get_by_label("1,024 × 768 mi");
}

fn template_dialog() -> Harness<'static, NewMapDialog> {
    let mut dialog = NewMapDialog::default();
    dialog.open = true;
    dialog_harness(
        |ui, dialog: &mut NewMapDialog| {
            dialog.show(ui.ctx());
        },
        dialog,
    )
}

#[test]
fn new_map_offers_every_template_with_the_starter_selected() {
    let mut harness = template_dialog();
    harness.run();
    assert!(harness.state().templates().len() >= 6);
    assert_eq!(harness.state().templates()[0].id(), "starter");
    let starter = harness.state().templates()[0].name().to_owned();
    assert_eq!(harness.state().selected_name(), Some(starter.as_str()));
    for template in harness.state().templates() {
        harness.get_by_label(template.name());
    }
    harness.get_by_label(&format!("Preview: {starter}"));
}

#[test]
fn picking_a_template_card_updates_the_preview_and_the_map() {
    let mut harness = template_dialog();
    harness.run();
    let names: Vec<String> = harness
        .state()
        .templates()
        .iter()
        .map(|t| t.name().to_owned())
        .collect();
    for name in names {
        harness.get_by_label(&name).click();
        harness.run();
        harness.get_by_label(&format!("Preview: {name}"));
        assert_eq!(harness.state().selected_name(), Some(name.as_str()));
        let template = harness.state().chosen_template().unwrap();
        assert_eq!(template.name(), name);
        let (map, _, problem) = ssmap_app::new_map::start_map_from(Ok(template.clone()), canvas());
        assert_eq!(problem, None);
        assert_eq!(map.canvas.background, *template.background());
    }
}

#[test]
fn creating_from_each_template_opens_a_map_with_its_background() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(dir.path());
    let (templates, problems) = ssmap_core::MapTemplate::load_all();
    assert!(problems.is_empty());
    for template in templates {
        let before = harness.state().tabs().len();
        harness
            .state_mut()
            .new_map_from(Ok(template.clone()), canvas());
        harness.run();
        assert_eq!(harness.state().tabs().len(), before + 1);
        let tab = harness.state().tabs().last().unwrap();
        assert_eq!(tab.editor().map.canvas.background, *template.background());
    }
}

#[test]
fn esc_cancels_the_export_dialog() {
    let mut dialog = ExportDialog::default();
    dialog.open_with(&ExportSettings::default(), None);
    let mut harness = dialog_harness(
        |ui, dialog: &mut ExportDialog| {
            dialog.show(ui.ctx(), &canvas());
        },
        dialog,
    );
    harness.run();
    harness.get_by_label("Export");

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(!harness.state().open);
    assert!(harness.query_by_label("Export").is_none());
}

#[test]
fn draw_region_on_canvas_hands_the_settings_to_the_region_tool() {
    use ssmap_app::export_dialog::DialogAction;
    use ssmap_core::{ExportRegion, Rect};

    let settings = ExportSettings {
        region: ExportRegion::Custom {
            rect: Rect {
                min: [10.0, 10.0],
                max: [60.0, 60.0],
            },
        },
        ..ExportSettings::default()
    };
    let mut dialog = ExportDialog::default();
    dialog.open_with(&settings, None);
    let mut harness = dialog_harness(
        |ui, (dialog, action): &mut (ExportDialog, Option<DialogAction>)| {
            if let Some(chosen) = dialog.show(ui.ctx(), &canvas()) {
                *action = Some(chosen);
            }
        },
        (dialog, None),
    );
    harness.run();
    harness.get_by_label("Draw region on Canvas").click();
    harness.run();
    assert_eq!(harness.state().1, Some(DialogAction::DrawRegion(settings)));
}

#[test]
fn export_disables_scale_options_over_the_cap() {
    let wide = Canvas::new(16384, 12288, Scale::new(1.0, DistanceUnit::Miles));
    let mut dialog = ExportDialog::default();
    dialog.open_with(&ExportSettings::default(), None);
    let mut harness = dialog_harness(
        move |ui, dialog: &mut ExportDialog| {
            dialog.show(ui.ctx(), &wide);
        },
        dialog,
    );
    harness.run();
    // 2× is 32,768 px, the cap; 4× and 8× are over it.
    for over in ["4×", "8×"] {
        harness.get_by_label(over).click();
        harness.run();
        assert_eq!(
            harness.state().draft().size,
            SizeMode::Scale { factor: 1.0 }
        );
    }
    harness.get_by_label("2×").click();
    harness.run();
    assert_eq!(
        harness.state().draft().size,
        SizeMode::Scale { factor: 2.0 }
    );
}

type PasteAnswer = Option<ssmap_app::paste_dialog::Choice>;

fn paste_prompt(gaps: ssmap_core::PasteGaps) -> Harness<'static, PasteAnswer> {
    dialog_harness(
        move |ui, answer: &mut PasteAnswer| {
            if answer.is_none() {
                *answer = ssmap_app::paste_dialog::show(ui.ctx(), &gaps);
            }
        },
        None,
    )
}

#[test]
fn the_paste_popup_lists_each_kind_of_missing_item() {
    let mut harness = paste_prompt(ssmap_core::PasteGaps {
        assets: vec!["pack/tree".parse().unwrap()],
        fonts: vec!["Papyrus".into()],
        label_styles: 2,
    });
    harness.run();
    harness.get_by_label("• Assets become placeholders until installed: pack/tree");
    harness.get_by_label("• Fonts are swapped for Noto Serif: Papyrus");
    harness.get_by_label("• 2 labels will follow this Map’s default style");
}

#[test]
fn the_paste_popup_only_lists_what_is_missing() {
    let mut harness = paste_prompt(ssmap_core::PasteGaps {
        label_styles: 1,
        ..Default::default()
    });
    harness.run();
    harness.get_by_label("• 1 label will follow this Map’s default style");
    assert!(harness.query_by_label_contains("Assets become").is_none());
    assert!(harness.query_by_label_contains("Fonts are").is_none());
}

#[test]
fn the_paste_popup_pastes_on_confirm_and_not_on_cancel() {
    let gaps = ssmap_core::PasteGaps {
        label_styles: 1,
        ..Default::default()
    };
    let mut harness = paste_prompt(gaps.clone());
    harness.run();
    harness.get_by_label("Paste anyway").click();
    harness.run();
    assert_eq!(
        *harness.state(),
        Some(ssmap_app::paste_dialog::Choice::Paste)
    );

    let mut harness = paste_prompt(gaps.clone());
    harness.run();
    harness.get_by_label("Cancel").click();
    harness.run();
    assert_eq!(
        *harness.state(),
        Some(ssmap_app::paste_dialog::Choice::Cancel)
    );

    let mut harness = paste_prompt(gaps);
    harness.run();
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert_eq!(
        *harness.state(),
        Some(ssmap_app::paste_dialog::Choice::Cancel)
    );
}

#[test]
fn esc_cancels_the_export_progress_dialog() {
    let mut harness = dialog_harness(
        |ui, cancelled: &mut bool| {
            if export_run::show_progress(
                ui.ctx(),
                Path::new("/maps/world.png"),
                Progress::Rendering { done: 3, total: 12 },
                Some(Duration::from_secs(20)),
            ) {
                *cancelled = true;
            }
        },
        false,
    );
    harness.run();
    harness.get_by_label("3 of 12 tiles");
    harness.get_by_label("20 s left");
    assert!(!harness.state());

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(*harness.state());
}

#[test]
fn export_progress_shows_the_waiting_state() {
    let mut harness = dialog_harness(
        |ui, _: &mut ()| {
            export_run::show_progress(
                ui.ctx(),
                Path::new("/maps/world.png"),
                Progress::Waiting,
                None,
            );
        },
        (),
    );
    // The spinner repaints for ever, so `run` would never settle.
    harness.step();
    harness.get_by_label("Waiting for rebuild");
}
