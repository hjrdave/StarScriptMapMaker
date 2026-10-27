//! StarScript Map Maker's editor. The binary in `main.rs` only opens a window around
//! [`MapMakerApp`], so UI tests can drive the same app headlessly.

mod app;
mod canvas;
mod chrome;
mod dialogs;
mod export;
mod packs;
mod panels;
mod workspace;

pub use canvas::{frame_time, overlay};
pub use chrome::{menu, theme};
pub use dialogs::{new_map, paste_dialog};
pub use export::{export_dialog, export_run};
pub use packs::pack_install;
pub use panels::prop_widgets;
pub use workspace::{editor, shell, tools};

use canvas::{handles, zoom};
use chrome::{icons, native_menu, tab, toast};
use dialogs::{dialog, native_dialog, recovery};
use export::export_region;
use workspace::{clipboard, credits, document, home, update};

use canvas::{canvas_controls, canvas_resize, canvas_view, tessellation};
use chrome::{tab_strip, tool_dock, tool_section};
use packs::pack_page;
use panels::{asset_browser, label_styles, layer_tree, properties};
use workspace::{file_actions, recent};

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
