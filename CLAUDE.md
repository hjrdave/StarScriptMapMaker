# CLAUDE.md

See `CONTEXT.md` for the domain language.

## App crate layout (`apps/starscript/src/`)

Modules are grouped by feature. `lib.rs` declares the groups and re-exports each module at the
crate root (`pub use chrome::theme;`), so paths like `ssmap_app::theme` and `crate::theme` stay
the same wherever the file lives. New modules go in a group's `mod.rs` and get a re-export in
`lib.rs`.

- `workspace/`: document model and app state: `document`, `editor`, `shell`, `home`, `tools`,
  `clipboard`, `credits`, `recent`, `file_actions`, `update`
- `canvas/`: the map view: `canvas_view`, `canvas_controls`, `art_board`, `overlay`, `handles`,
  `zoom`, `tessellation`, `frame_time` (frame budget harness)
- `panels/`: side panels: `layer_tree`, `properties`, `prop_widgets`, `asset_browser`,
  `label_styles`
- `chrome/`: window chrome: `menu`, `native_menu`, `tab`, `tab_strip`, `tool_dock`,
  `tool_section`, `toast`, `theme`, `icons`
- `dialogs/`: `dialog`, `native_dialog`, `paste_dialog`, `new_map`, `recovery`
- `export/`: `export_dialog`, `export_region`, `export_run`
- `packs/`: `pack_install`, `pack_page`
