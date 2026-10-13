# Fantasy Map Maker

A native, cross-platform desktop application for drawing fantasy maps. Its defining idea is Figma-style layers: everything on a map stays editable, nothing is baked into pixels.

## Language

**Map**:
One document the user creates and edits, made of a Canvas and a tree of Layers.
_Avoid_: Project, file, document

**Canvas**:
The bounded drawing area of a Map, with a fixed size and a real-world scale chosen when the Map is created.
_Avoid_: Board, artboard, page

**Object**:
Any single thing placed on a Map, such as a coastline, a placed asset or a label.
_Avoid_: Element, item, shape, node

**Layer**:
A container in the Map's ordered tree that holds Objects and other Layers, and has a name, visibility, lock and opacity.
_Avoid_: Folder, frame

**Group**:
A Layer the user creates to move, hide or clip several Objects together. Layers nest, so a Group is just a nested Layer.
_Avoid_: Folder, collection

**Stacking order**:
The single top-to-bottom order of the Map's tree that decides what draws above what.
_Avoid_: Z-order, depth

**Clip**:
A relationship that restricts a Layer to the shape of a Landmass, so its contents show only where there is land and not where a Lake covers it.
_Avoid_: Mask

### Land and water

**Landmass**:
An Object that is a filled region of land, made of one or more parts, each with an outer boundary and optional holes that show ocean.
_Avoid_: Island, continent, land shape

**Coastline**:
The boundary of a Landmass. It is not an Object of its own; it is what shore styling and Roughness act on.
_Avoid_: Shore path, outline

**Lake**:
An Object that is a closed region of fresh water drawn above land, separate from any Landmass.
_Avoid_: Pond, hole

**Roughness**:
Generated fractal detail added to a Coastline from an amount, a scale and a seed, so a smooth stored outline renders as a jagged coast.
_Avoid_: Noise, jitter, fractalisation

**Shore style**:
The set of effects drawn along a Coastline: outline, ripple lines and inner shading. A Map has a default, and each Landmass or Lake may override it.
_Avoid_: Coast effect, border style
