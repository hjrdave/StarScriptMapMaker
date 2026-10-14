# Packaging

How the installable packages are built and laid out (ADR 0008). The config is
`[package.metadata.packager]` in `crates/app/Cargo.toml`. Files it uses are in
`crates/app/packaging/`.

## Building

```sh
cargo install cargo-packager --version 0.11.8 --locked
cd crates/app
cargo packager --release --formats deb,appimage   # Linux
cargo packager --release --formats nsis           # Windows
cargo packager --release --formats dmg            # macOS (arm64)
```

Packages land in `target/packages/`. Each OS builds only its own formats. The nightly
workflow (`.github/workflows/nightly.yml`) builds all of them, installs each one, opens
a Map file with it, and uploads the packages with SHA-256 checksums.

Every package is unsigned (amended ADR 0008). The macOS binary carries only the linker's
ad-hoc signature.

## Install layout

The repo's `assets/` (the built-in Asset packs and fonts) is copied read-only into each
package. `ssmap_app::install::assets_root` is the only code that knows where it goes:

| Package | Binary | Assets |
| --- | --- | --- |
| Windows (NSIS, per-user) | `%LOCALAPPDATA%\StarScript Map Maker\starscript-map-maker.exe` | `…\StarScript Map Maker\assets\` |
| macOS (DMG) | `StarScript Map Maker.app/Contents/MacOS/starscript-map-maker` | `…/Contents/Resources/assets/` |
| Linux (.deb, AppImage) | `/usr/bin/starscript-map-maker` | `/usr/lib/starscript-map-maker/assets/` |

From a source checkout (`cargo run`, tests) it falls back to the repo's `assets/`.

`LICENSE` sits beside `assets/` in every package. It is a plain file, not a license the
installer or DMG asks the user to accept.

User packs and settings do not belong here. They go in the platform data and config
folders.

## `.ssmap` file association

| OS | How it's registered | How the file reaches the app |
| --- | --- | --- |
| Windows | NSIS writes `HKCU\Software\Classes\.ssmap` → ProgID `StarScriptMapMaker.Map`, whose open command is `starscript-map-maker.exe "%1"` | command-line argument |
| Linux | The `.desktop` file has `MimeType=application/x-starscript-map;` and `Exec=… %F`. `packaging/linux/starscript-map-maker.xml` defines the MIME type (installed to `/usr/share/mime/packages/`) | command-line argument |
| macOS | `packaging/macos/Info.plist` declares the `io.github.hjrdave.starscript-map` UTI for `.ssmap` and claims it as its editor (`CFBundleDocumentTypes`) | an open-documents Apple Event, **not handled yet** (below) |

`main.rs` passes the first non-flag argument to `MapMakerApp::open_path` after startup.

An AppImage registers nothing by itself. Integration tools such as AppImageLauncher or Gear
Lever pick up its `.desktop` file and MIME XML.

### Known gap: opening from Finder on macOS

Finder launches the app without the file as an argument. It sends the file through
`application:openURLs:` on the `NSApplicationDelegate`. winit 0.30, used by eframe 0.36,
does not expose that event. Its docs say to register your own delegate with `objc2`,
which needs `unsafe`, and the workspace forbids `unsafe`. Until that is decided, macOS
registers the type and launches the app on a double-click, but the app opens a blank
Map. `open -a "StarScript Map Maker" --args file.ssmap` and running the binary with a
path both work.
