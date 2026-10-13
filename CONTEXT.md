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
A relationship that restricts a Layer to the shape of a landmass, so its contents show only where there is land.
_Avoid_: Mask
