//! StarScript Map Maker's editor. The binary in `main.rs` only opens a window around
//! [`MapMakerApp`], so UI tests can drive the same app headlessly.

mod app;
mod art_board;
mod asset_browser;
mod canvas_controls;
mod canvas_view;
pub mod clipboard;
pub mod credits;
pub mod dialog;
pub mod document;
pub mod editor;
pub mod export_dialog;
pub mod export_region;
pub mod export_run;
mod file_actions;
pub mod frame_time;
pub mod handles;
pub mod home;
pub mod icons;
mod label_styles;
mod layer_tree;
pub mod menu;
pub mod native_dialog;
pub mod native_menu;
pub mod new_map;
pub mod overlay;
pub mod pack_install;
mod pack_page;
pub mod paste_dialog;
pub mod prop_widgets;
mod properties;
mod recent;
pub mod recovery;
pub mod shell;
pub mod tab;
mod tab_strip;
mod tessellation;
pub mod theme;
pub mod toast;
mod tool_dock;
mod tool_section;
pub mod tools;
pub mod update;
pub mod zoom;

use eframe::egui;
use std::ffi::OsString;
use std::path::PathBuf;

pub use app::MapMakerApp;

/// The Map file to open at launch, from the command-line arguments after the program name.
///
/// Windows (the installer's file association) and Linux (the `.desktop` file's `%F`) pass a
/// double-clicked file this way. Flags are skipped, including the `-psn_…` process serial
/// number older macOS versions add.
pub fn launch_file(args: impl IntoIterator<Item = OsString>) -> Option<PathBuf> {
    let path = PathBuf::from(
        args.into_iter()
            .find(|arg| !arg.to_string_lossy().starts_with('-'))?,
    );
    // Absolute, so Recent Files still finds it from another working directory.
    Some(std::path::absolute(&path).unwrap_or(path))
}

/// On macOS the tabs are drawn in the title bar, behind the window's own buttons, which they
/// leave room for. Elsewhere the native title bar stays and the strip sits below it.
const CUSTOM_CHROME: bool = cfg!(target_os = "macos");
const CHROME_INSET: f32 = if CUSTOM_CHROME {
    theme::metrics::TRAFFIC_LIGHTS_INSET
} else {
    0.0
};

/// The window options for the title bar: on macOS the content runs under a transparent title
/// bar with no title text, so the tab strip is drawn there.
pub fn window_chrome(viewport: egui::ViewportBuilder) -> egui::ViewportBuilder {
    if CUSTOM_CHROME {
        viewport
            .with_fullsize_content_view(true)
            .with_title_shown(false)
            .with_titlebar_shown(false)
    } else {
        viewport
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn launch_file_is_the_first_non_flag_argument() {
        assert_eq!(launch_file(args(&[])), None);
        assert_eq!(launch_file(args(&["-psn_0_12345"])), None);
        let file = launch_file(args(&["-psn_0_12345", "maps/Isle.ssmap", "other.ssmap"]));
        let file = file.unwrap();
        assert!(file.is_absolute());
        assert!(file.ends_with("maps/Isle.ssmap"));
    }
}
