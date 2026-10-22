//! Where packs live on disk. This is the one place that knows the install layout, so
//! packaging and the app agree on it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The manifest file inside every pack folder.
pub const PACK_MANIFEST: &str = "pack.toml";

/// Overrides [`assets_dir`] when set and non-empty.
pub(crate) const ASSETS_DIR_ENV: &str = "SSMAP_ASSETS_DIR";

/// The application name the per-user data folder is derived from.
const APP_NAME: &str = "StarScript Map Maker";

/// The read-only folder of built-in packs and bundled fonts.
///
/// The first of these that exists, else the last:
///
/// 1. [`ASSETS_DIR_ENV`].
/// 2. `assets/` beside the executable (the Windows installer and portable builds).
/// 3. `../Resources/assets/` from the executable (a macOS app bundle).
/// 4. `../lib/starscript-map-maker/assets/` from the executable (Linux `.deb` and AppImage).
/// 5. The repository's `assets/`, for development builds.
pub fn assets_dir() -> PathBuf {
    let exe = std::env::current_exe().ok();
    resolve_assets_dir(
        std::env::var_os(ASSETS_DIR_ENV),
        exe.as_deref(),
        Path::is_dir,
    )
}

/// The folder holding one folder per built-in pack.
pub(crate) fn built_in_packs_dir() -> PathBuf {
    assets_dir().join("packs")
}

/// The per-user packs folder, such as `~/.local/share/starscriptmapmaker/packs` on Linux.
/// Unsupported in v1, but packs there load. `None` when the platform reports no home folder.
pub(crate) fn user_packs_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", APP_NAME).map(|dirs| dirs.data_dir().join("packs"))
}

fn resolve_assets_dir(
    env: Option<OsString>,
    exe: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    if let Some(dir) = env.filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let beside_exe = exe.and_then(Path::parent).into_iter().flat_map(|bin| {
        [
            bin.join("assets"),
            bin.join("../Resources/assets"),
            bin.join("../lib/starscript-map-maker/assets"),
        ]
    });
    beside_exe
        .chain(std::iter::once(development.clone()))
        .find(|dir| exists(dir))
        .unwrap_or(development)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_wins() {
        let dir = resolve_assets_dir(Some("/opt/packs".into()), None, |_| true);
        assert_eq!(dir, PathBuf::from("/opt/packs"));
    }

    #[test]
    fn an_empty_environment_value_is_ignored() {
        let exe = Path::new("/app/bin/ssmap");
        let dir = resolve_assets_dir(Some("".into()), Some(exe), |_| true);
        assert_eq!(dir, PathBuf::from("/app/bin/assets"));
    }

    #[test]
    fn a_mac_bundle_uses_its_resources() {
        let exe = Path::new("/Applications/S.app/Contents/MacOS/s");
        let dir = resolve_assets_dir(None, Some(exe), |dir| dir.ends_with("Resources/assets"));
        assert_eq!(
            dir,
            PathBuf::from("/Applications/S.app/Contents/MacOS/../Resources/assets")
        );
    }

    #[test]
    fn development_builds_fall_back_to_the_repo() {
        let exe = Path::new("/repo/target/debug/ssmap");
        let dir = resolve_assets_dir(None, Some(exe), |_| false);
        assert!(dir.ends_with("../../assets"), "{}", dir.display());
        assert!(dir.join("README.md").is_file(), "{}", dir.display());
    }
}
