//! Where the app's read-only files live once installed (ADR 0008).
//!
//! The packager (`[package.metadata.packager]` in this crate's `Cargo.toml`) copies the
//! repo's `assets/` into each package. This module is the one place that knows where each
//! package puts it. Built-in Asset packs and fonts are read from here, never written.

use std::path::{Path, PathBuf};

/// The name of the installed binary, which the Linux packages use for their resource folder.
const BINARY_NAME: &str = "starscript-map-maker";

/// The folder holding the built-in Asset packs and fonts, or `None` if no install layout and
/// no source checkout is found.
pub fn assets_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe = exe.canonicalize().unwrap_or(exe);
    assets_root_near(exe.parent()?).or_else(|| {
        // Running from a source checkout (`cargo run`, tests): use the repo's `assets/`.
        let repo_assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        repo_assets.is_dir().then_some(repo_assets)
    })
}

/// The packaged assets folder next to an executable in `exe_dir`, if there is one.
pub fn assets_root_near(exe_dir: &Path) -> Option<PathBuf> {
    [
        // Windows (NSIS): resources sit beside the exe in the install folder.
        exe_dir.join("assets"),
        // macOS: `StarScript Map Maker.app/Contents/{MacOS,Resources}`.
        exe_dir.join("../Resources/assets"),
        // Linux (.deb, and the AppImage's bundled /usr): `/usr/{bin,lib/<binary>}`.
        exe_dir.join("../lib").join(BINARY_NAME).join("assets"),
    ]
    .into_iter()
    .find(|dir| dir.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(exe_dir: &str, assets: &str) -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let exe_dir = root.path().join(exe_dir);
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(root.path().join(assets)).unwrap();
        (root, exe_dir)
    }

    #[test]
    fn finds_each_package_layout() {
        for (exe_dir, assets) in [
            ("StarScript Map Maker", "StarScript Map Maker/assets"),
            (
                "App.app/Contents/MacOS",
                "App.app/Contents/Resources/assets",
            ),
            ("usr/bin", "usr/lib/starscript-map-maker/assets"),
        ] {
            let (root, exe_dir) = layout(exe_dir, assets);
            let found = assets_root_near(&exe_dir).expect(assets);
            assert_eq!(
                found.canonicalize().unwrap(),
                root.path().join(assets).canonicalize().unwrap()
            );
        }
    }

    #[test]
    fn finds_nothing_without_assets() {
        let (_root, exe_dir) = layout("usr/bin", "usr/lib/other-app/assets");
        assert_eq!(assets_root_near(&exe_dir), None);
    }

    #[test]
    fn falls_back_to_the_repo_assets() {
        // The test binary lives in `target/`, which has no packaged layout.
        let root = assets_root().unwrap();
        assert!(root.join("README.md").is_file());
    }
}
