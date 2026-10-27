//! What the Home cards do to a Map's file on disk: show it in the system file manager, and
//! move it to the Trash.

use std::path::Path;
use std::process::Command;

use ssmap_core::asset::TrashError;

/// The menu item that shows a file in the system file manager, named as the platform does.
pub(crate) const REVEAL_LABEL: &str = if cfg!(target_os = "macos") {
    "Show in Finder"
} else if cfg!(target_os = "windows") {
    "Show in File Explorer"
} else {
    "Show in File Manager"
};

/// What the platform calls the place deleted files go.
pub(crate) const TRASH_NAME: &str = if cfg!(target_os = "windows") {
    "Recycle Bin"
} else {
    "Trash"
};

/// Opens the folder holding `path` with the file selected where the platform can (Finder,
/// File Explorer); elsewhere the folder alone is opened.
pub(crate) fn reveal(path: &Path) -> Result<(), String> {
    let mut command = reveal_command(path);
    command
        .spawn()
        .map(drop)
        .map_err(|err| format!("couldn’t open the file manager: {err}"))
}

fn reveal_command(path: &Path) -> Command {
    if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg("-R").arg(path);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = Command::new("explorer");
        // `/select,<path>` must be one argument.
        command.arg(format!("/select,{}", path.display()));
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(path.parent().unwrap_or(path));
        command
    }
}

/// Moves the file at `path` to the system Trash, so it can be put back from there.
pub(crate) fn move_to_trash(path: &Path) -> Result<(), TrashError> {
    trash::delete(path).map_err(|err| err.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_opens_the_folder_holding_the_file() {
        if cfg!(any(target_os = "macos", target_os = "windows")) {
            return;
        }
        let command = reveal_command(Path::new("/maps/Isle.ssmap"));
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["/maps"]);
    }
}
