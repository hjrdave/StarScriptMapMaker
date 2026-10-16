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

## Native menu bar check (#199)

On macOS the menus are the native menu bar, built with `muda` from the same menu model as the
egui bar (`apps/starscript/src/menu.rs`, `native_menu.rs`). CI cannot build or run that code, so
nobody has seen it yet. Check it by hand:

- [ ] There is no egui menu bar under the tab strip. The menu bar at the top of the screen
      shows StarScript Map Maker, File, Edit, View, Map, Layer, Object, Window and Help.
- [ ] Open every menu. Each lists the items of the egui bar (File: New Map… to Re-export;
      Edit: Undo to Deselect; View: Zoom, panels, Home, Tab 2–9; Window: one entry per open
      Map; Help: Credits, update checks), with the shortcut shown on the right.
- [ ] The StarScript Map Maker menu has About StarScript Map Maker (opens Credits),
      Check for Updates…, Services, Hide, Hide Others, Show All and Quit. ⌘H hides the app.
      ⌘Q with unsaved changes asks what to do with them, as closing the window does.
- [ ] On Home, only New Map…, New Tab, Open…, Open Recent, Home, Credits and the update
      items are enabled. With a Map open the rest enable. Layer ▸ Delete, Edit ▸ Delete,
      Deselect and View ▸ Zoom to Selection are disabled until something is selected;
      Object ▸ Union and Subtract need two selected Objects.
- [ ] Draw something: Edit shows "Undo <name>" (for example "Undo Add Layer"), and the title
      changes after each further change. Undo it: "Redo <name>" appears. Switch to another tab:
      the titles follow that tab. Open Recent lists the recent Maps.
- [ ] Each ⌘ shortcut fires **once**, not twice (watch for a double Undo, two New Map
      dialogs, or the panels toggling back): ⌘N, ⌘T, ⌘O, ⌘W, ⌘S, ⇧⌘S, ⌘E, ⌘Z, ⇧⌘Z, ⌘A, ⇧⌘A,
      ⌘+, ⌘−, ⌥⌘1, ⌥⌘2, ⇧⌘N, ⌘1, ⌘2–⌘9. ⌘+ should also work as ⌘= on a US keyboard.
- [ ] Keys without ⌘ still work and show no label in the menu: Backspace, Enter, Esc, ⇥ (hide
      panels), ⇧1, ⇧2 and ⇧0.
- [ ] Text fields keep their keys: click into a Label's text box, a Properties field and the
      Home search box. ⌘Z, ⌘A, ⌘C and ⌘V edit the text, Backspace and Esc act on the text, and
      the Map is not changed.
- [ ] Open a dialog (Export, New Map): menu items and shortcuts do nothing under it.
- [ ] Tick marks: Help ▸ Check for updates at startup and the active Map under Window show a
      tick, and choosing one leaves the tick matching its state.

## Manual test checklist

- [ ] Launch the app. Open the Credits screen.
- [ ] New Map. Draw land, add Stamps and a Label.
- [ ] Save, quit, reopen the file, and confirm the Map matches.
- [ ] Double-click a `.ssmap` file in Finder. Known gap (`docs/packaging.md`): the app
      launches but opens a blank Map. Note what happens. As a workaround,
      `open -a "StarScript Map Maker" --args file.ssmap` should open it.
- [ ] Export a PNG. Export again with a custom region (Export Region tool). Start an
      export and cancel it.
- [ ] Title bar with tabs (#161): the tab strip is drawn in the title bar, 30 px high, with
      the traffic lights clear at the left (84 px) and no title text.
- [ ] Drag the empty part of the strip: the window moves. Double-click it: the window zooms.
- [ ] Open a few Maps. Tabs are flat, 220 px wide, with a 1 px divider; the active tab has a
      gold line at the bottom. Hover shows a ×; a Map with unsaved changes shows a gold dot
      that the × replaces on hover. Hover a tab: the tooltip is the full path ("Not saved
      yet" for a new Map). Middle-click closes a tab. Home cannot be closed.
- [ ] Open enough Maps that the tabs shrink to 120 px and the strip scrolls with chevrons;
      the active tab scrolls into view.
- [ ] ⌘T (New Map), ⌘W, ⌘1 (Home), ⌘2–⌘9 (⌘9 is the last tab), ⌃Tab and ⌃⇧Tab.
- [ ] Home tab (#162): launching with no file shows Home and no Map tab; launching with a
      file opens it in a tab. The left rail is 232 px with All maps, Recent, Open in tabs
      (counts), Asset packs, Credits & licences (opens the Credits window) and the version.
      The grid has 4 columns, the dashed New Map card first (⌘N), then one card per Map with
      its preview, name, "Edited … ago", Canvas size and an "Open in tab" chip when open.
      Search, the Last edited / Name sort and Open… work. Delete a listed file: its card
      shows as missing with "Remove from list". With ten or more Maps the first frame is not
      held up while previews load. On a first launch (reset the stored choice) the update
      banner shows at the bottom of Home; Not now and Check automatically both dismiss it.
- [ ] Check both the dark and the light theme.
- [ ] Open a large Map. Pan and zoom; note how smooth it feels.
- [ ] Run the frame-time measurement and record the numbers on #92:

  ```sh
  "target/packages/StarScript Map Maker.app/Contents/MacOS/starscript-map-maker" --frame-time
  ```
