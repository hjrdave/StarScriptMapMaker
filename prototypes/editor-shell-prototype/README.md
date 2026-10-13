# PROTOTYPE: Figma-style editor shell (throwaway)

Answers the map ticket **Prototype: Figma-style three-panel editor shell**
(https://github.com/hjrdave/StarScriptMapMaker/issues/7). Do not build on this code.

Question: does a Figma-style layout feel right for a map editor, and can egui 0.36 + our own wgpu 30 pipeline produce it?

## Run

```sh
cd prototypes/editor-shell-prototype
cargo run --release              # starts on variant A
cargo run --release -- --variant B
```

Needs a Rust toolchain (edition 2024). On Linux you also need the usual X11/Wayland dev libs (`libxkbcommon-x11`).

## What's in it

Switch variants with the yellow bar at the bottom or the Left/Right arrow keys.

- **A (Classic Figma):** toolbar on top, layer tree left, properties right, canvas centre.
- **B (Floating panels):** canvas fills the window; layers and properties float over it as movable panels; tool dock at the bottom (Figma UI3 style).
- **C (Single sidebar):** one left sidebar with Layers / Properties / Assets tabs; a contextual toolbar floats above whatever is selected.

All three share the same in-memory document (layers that nest, holding landmasses, assets and labels) and canvas:

- Ocean backdrop drawn by **our own wgpu pipeline** (`src/ocean.wgsl`) inside egui's render pass; land, assets, labels and handles drawn with egui's painter.
- Click to select, drag to move, drag a corner handle to scale. Scroll to zoom, middle/right-drag or Space+drag to pan.
- Layer tree: drag a row onto another row to reorder; dropping on a layer row puts it inside that layer. Eye and lock toggles. Double-click a layer to expand.
- Pick Land / Mountain / Tree / Label and click the canvas to place into the selected layer (or the default layer for that kind).
- **Stress: +10,000 trees** adds a 10k-object layer; the layer tree virtualises it. The status bar shows UI build time per frame.

## Known prototype shortcuts

Landmass shapes are generated blobs, not editable paths. Icons are emoji from egui's bundled fonts. No undo, no saving. The selection hit-test is bounding boxes only.
