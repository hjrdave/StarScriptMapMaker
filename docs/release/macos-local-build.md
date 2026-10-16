# Local macOS build and test checklist

How to build the unsigned macOS app on your own Apple Silicon Mac without tagging a
release, open it, and test it by hand. Intel Macs are not covered.

## Build

```sh
scripts/build-macos.sh
```

It needs the Rust toolchain. It installs the same pinned `cargo-packager` as
`release.yml` if missing, runs `cargo packager --release --formats dmg`, and prints
where the `.dmg` and `.app` landed (under `target/packages`).

## First launch: Gatekeeper

The build is unsigned (ADR 0008), so macOS blocks it the first time.

- **Open Anyway:** open the app (or the one in the DMG, dragged to Applications). macOS
  says it can't verify it; click **Done**. Go to **System Settings → Privacy & Security**,
  scroll to Security and click **Open Anyway** next to StarScript Map Maker. Confirm with
  your password. Once only.
- **Shortcut for a build made on this machine:**

  ```sh
  xattr -dr com.apple.quarantine "target/packages/StarScript Map Maker.app"
  ```

  Use the path the script printed, or the copy in Applications. A locally built app is
  often not quarantined at all; this matters for a copy that came through a download.

## Manual test checklist

- [ ] Launch the app. Open the Credits screen.
- [ ] New Map. Draw land, add Stamps and a Label.
- [ ] Save, quit, reopen the file, and confirm the Map matches.
- [ ] Double-click a `.ssmap` file in Finder. Known gap (`docs/packaging.md`): the app
      launches but opens a blank Map. Note what happens. As a workaround,
      `open -a "StarScript Map Maker" --args file.ssmap` should open it.
- [ ] Export a PNG. Export again with a custom region (Export Region tool). Start an
      export and cancel it.
- [ ] Title bar with tabs (#161): the tab strip is drawn in the title bar, 38 px high, with
      the traffic lights clear at the left (84 px) and no title text.
- [ ] Drag the empty part of the strip: the window moves. Double-click it: the window zooms.
- [ ] Open a few Maps. Tabs are flat, 220 px wide, with a 1 px divider; the active tab has a
      gold line at the bottom. Hover shows a ×; a Map with unsaved changes shows a gold dot
      that the × replaces on hover. Hover a tab: the tooltip is the full path ("Not saved
      yet" for a new Map). Middle-click closes a tab. Home cannot be closed.
- [ ] Open enough Maps that the tabs shrink to 120 px and the strip scrolls with chevrons;
      the active tab scrolls into view.
- [ ] ⌘T (New Map), ⌘W, ⌘1 (Home), ⌘2–⌘9 (⌘9 is the last tab), ⌃Tab and ⌃⇧Tab.
- [ ] Check both the dark and the light theme.
- [ ] Open a large Map. Pan and zoom; note how smooth it feels.
- [ ] Run the frame-time measurement and record the numbers on #92:

  ```sh
  "target/packages/StarScript Map Maker.app/Contents/MacOS/starscript-map-maker" --frame-time
  ```
