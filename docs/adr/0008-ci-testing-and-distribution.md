# CI on GitHub Actions with software-GPU golden tests; cargo-packager releases

Status: accepted

> **Amendment (macOS-first):** the app is still intended to be cross-platform, but for now development, CI, nightly and release builds cover **macOS arm64 only**. Linux and Windows checks, packages and GPU measurements are paused, not dropped. The sections below describe the full three-platform plan to restore later.

**Targets:** v1 builds for Linux x86_64, Windows x86_64 and macOS arm64 only. Intel Macs and Windows ARM64 wait until users ask.

**GPU testing:** hosted CI runners have no real GPU, so rendering is checked with **golden-image tests on software adapters**:
- Linux on Mesa lavapipe (Vulkan) is the canonical reference.
- Windows runs on WARP (DX12).
- macOS runs everything except golden comparisons, plus a render smoke test when a Metal device is present.

Comparisons allow a per-pixel tolerance and a cap on the share of pixels that differ. Golden PNGs live in plain git next to the tests. We chose this over self-hosted GPU runners (cost, upkeep) and over manual-only checks (they wouldn't catch the tile seams ADR 0007 requires us to test).

## Test layers

- **`core`:** unit tests; proptest that every Op's inverse restores the Map (ADR 0004); JSON Command-script tests with insta snapshots of the read view; determinism tests showing booleans and Roughness are identical on all three OSes.
- **`render`:** golden images, including export tile seams and Silhouette fills.
- **`app`:** a few egui_kittest smoke tests only (launch, open a Map, run a tool). Logic belongs in `core`.

## Gates

- **Every PR:** `cargo fmt --check`, `clippy -D warnings`, tests on all three OSes, golden tests on Linux and Windows, `cargo deny`, and a `cargo tree` check that `core` never depends on egui or wgpu.
- **Nightly:** criterion benchmarks for save, open and command apply at 100k Stamps on Linux. They fail only above 2× the ADR 0005 targets and otherwise post trend numbers. Nightly also runs full release builds.

## Distribution

- **Packaging** uses one cargo-packager config: an NSIS installer for Windows (per-user, no admin), a DMG for macOS, and an AppImage plus .deb for Linux.
- **File association and install layout:**
  - `.ssmap` is registered on each OS: NSIS registry entries, an `Info.plist` document type, and a `.desktop` file with MIME XML.
  - Built-in Asset packs and fonts install read-only inside the app.
  - User packs and settings live in the platform data and config directories (`directories` crate).
- **No code signing** (amended after acceptance, by the author's decision): no Apple Developer ID, notarisation or Windows certificate. The macOS arm64 build carries only the linker's automatic ad-hoc signature, which Apple Silicon needs to run it at all. Every release publishes SHA-256 checksums, and the download page and release notes explain the first-launch warnings:
  - macOS Gatekeeper blocks the app; users allow it in System Settings → Privacy & Security → "Open Anyway".
  - Windows SmartScreen shows "Windows protected your PC"; users choose "More info" → "Run anyway".
  - Linux needs nothing extra.
- **Releases:** semver. A pushed `vX.Y.Z` tag runs a workflow that builds, checksums and uploads to GitHub Releases on this public repo. GitHub Releases also hosts a small JSON update feed.
- **Updates:** the app checks the feed only if the user agreed on first launch (the setting can be turned off) and shows a download link. There is no self-updater.
